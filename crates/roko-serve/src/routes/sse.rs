//! SSE endpoint for real-time dashboard event streaming.
//!
//! Clients connect at `/api/events` and receive `DashboardEvent` payloads as
//! SSE `data:` frames. Each event carries a monotonic `id:` for reconnection.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures::stream::{self, StreamExt};
use tokio::sync::broadcast;
use tracing::warn;

use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/events", get(sse_handler))
        .route("/sse", get(sse_handler))
}

/// Response headers that instruct HTTP/2 proxies (Railway, Nginx, Cloudflare)
/// to disable buffering so SSE frames reach the client immediately.
pub(crate) fn sse_response_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("X-Accel-Buffering", HeaderValue::from_static("no"));
    headers.insert(
        "Cache-Control",
        HeaderValue::from_static("no-cache, no-store, no-transform, must-revalidate"),
    );
    headers.insert("Connection", HeaderValue::from_static("keep-alive"));
    headers
}

/// `GET /api/events` and `GET /api/sse` — SSE stream of dashboard events.
async fn sse_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let last_event_id = headers
        .get("Last-Event-ID")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    // Cap the replay to 256 events to prevent a reconnecting client from
    // materializing the entire ring buffer into memory at once.
    let replay = state
        .state_hub
        .replay_from(last_event_id)
        .into_iter()
        .take(256)
        .map(|envelope| {
            let data = serde_json::to_string(&envelope.payload).unwrap_or_default();
            Ok::<_, Infallible>(Event::default().data(data).id(envelope.seq.to_string()))
        });

    let live = stream::unfold(state.state_hub.subscribe_events(), |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    let data = serde_json::to_string(&envelope.payload).unwrap_or_default();
                    let event = Event::default().data(data).id(envelope.seq.to_string());
                    return Some((Ok(event), rx));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(n, "SSE client lagged, skipped events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    // Use a shorter keep-alive interval than the default 15s to survive
    // aggressive proxy timeouts (Railway 30s, Nginx 60s). The "keepalive"
    // text triggers a proper SSE comment event in clients that ignore
    // empty comments.
    let sse = Sse::new(stream::iter(replay).chain(live)).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(8))
            .text("keepalive"),
    );

    (sse_response_headers(), sse)
}
