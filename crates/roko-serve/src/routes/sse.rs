//! SSE endpoint for real-time dashboard event streaming.
//!
//! Clients connect at `/api/events` and receive `DashboardEvent` payloads as
//! SSE `data:` frames. Each event carries a monotonic `id:` for reconnection.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures::stream::{self, Stream};
use tokio::sync::broadcast;
use tracing::warn;

use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/events", get(sse_handler))
}

/// `GET /api/events` — SSE stream of dashboard events.
async fn sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.state_hub.subscribe_events();

    let stream = stream::unfold(rx, |mut rx| async move {
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

    Sse::new(stream).keep_alive(KeepAlive::default())
}
