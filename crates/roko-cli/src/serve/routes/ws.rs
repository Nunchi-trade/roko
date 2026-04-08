//! WebSocket endpoint for real-time event streaming.
//!
//! Clients connect at `/ws` and receive `ServerEvent` payloads as JSON text
//! frames. On connection, the server replays recent events from the ring
//! buffer, then streams live events via the broadcast channel.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::stream::StreamExt;
use futures::SinkExt;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::serve::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/ws", get(ws_upgrade))
}

/// `GET /ws` — upgrade to a WebSocket connection.
async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(state, socket))
}

/// Client control message (optional filtering).
#[derive(Deserialize)]
struct ClientMsg {
    #[serde(default)]
    subscribe: Vec<String>,
}

/// Main WebSocket handler — replay then stream.
async fn handle_ws(state: Arc<AppState>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut filter: Vec<String> = Vec::new();

    // Replay recent events.
    let backlog = state.event_bus.replay_from(0);
    for envelope in &backlog {
        let Ok(payload) = serde_json::to_string(&envelope.payload) else {
            continue;
        };
        if sink.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }

    // Subscribe to live events.
    let mut rx = state.event_bus.subscribe();

    loop {
        tokio::select! {
            // Incoming client messages (filter subscriptions).
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(cmd) = serde_json::from_str::<ClientMsg>(&text) {
                            filter = cmd.subscribe;
                            debug!(?filter, "ws client updated subscription filter");
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        debug!("ws client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!("ws recv error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
            // Outgoing events.
            event = rx.recv() => {
                match event {
                    Ok(envelope) => {
                        if !filter.is_empty() && !matches_filter(&envelope.payload, &filter) {
                            continue;
                        }
                        match serde_json::to_string(&envelope.payload) {
                            Ok(json) => {
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                warn!("ws serialize error: {e}");
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(n, "ws client lagged, skipped events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        debug!("event bus closed, shutting down ws");
                        break;
                    }
                }
            }
        }
    }

    let _ = sink.close().await;
}

/// Check whether an event matches the client's subscription filter.
///
/// Filter strings are matched against the event's serde `type` tag.
/// An empty filter accepts all events.
fn matches_filter(
    event: &crate::serve::events::ServerEvent,
    filter: &[String],
) -> bool {
    // Serialize to extract the "type" field cheaply.
    let Ok(val) = serde_json::to_value(event) else {
        return true;
    };
    let event_type = val
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("");

    filter.iter().any(|f| event_type.contains(f.as_str()))
}
