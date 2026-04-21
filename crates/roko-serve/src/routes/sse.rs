//! SSE endpoint for real-time dashboard event streaming.
//!
//! Clients connect at `/api/events` and receive `DashboardEvent` payloads as
//! SSE `data:` frames. Each event carries a monotonic `id:` for reconnection.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures::stream::{self, Stream, StreamExt};
use tokio::sync::broadcast;
use tracing::warn;

use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/events", get(sse_handler))
        .route("/sse", get(sse_handler))
}

/// `GET /api/events` and `GET /api/sse` — SSE stream of dashboard events.
async fn sse_handler(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_event_id = headers
        .get("Last-Event-ID")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    let replay = state
        .state_hub
        .replay_from(last_event_id)
        .into_iter()
        .map(|envelope| {
            let data = serde_json::to_string(&envelope.payload).unwrap_or_default();
            Ok(Event::default().data(data).id(envelope.seq.to_string()))
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

    Sse::new(stream::iter(replay).chain(live)).keep_alive(KeepAlive::default())
}
