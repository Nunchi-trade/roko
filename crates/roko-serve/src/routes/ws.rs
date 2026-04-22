//! WebSocket endpoint for real-time event streaming.
//!
//! Clients connect at `/ws` and receive `ServerEvent` payloads as JSON text
//! frames. On connection, the server replays recent events from the ring
//! buffer, then streams live events via the broadcast channel.

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures::SinkExt;
use futures::stream::StreamExt;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ws", get(ws_upgrade))
        .route("/roko-ws", get(ws_upgrade))
}

/// `GET /ws` — upgrade to a WebSocket connection.
async fn ws_upgrade(State(state): State<Arc<AppState>>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(state, socket))
}

/// Back-pressure mode for a WebSocket subscription channel.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum BackPressureMode {
    /// Deliver every event, dropping only on transport failure.
    #[default]
    AtMostOnce,
    /// Coalesce rapid-fire events of the same type into one delivery.
    Coalesce,
    /// Server must buffer and redeliver after reconnect.
    ResumeRequired,
}

/// Client control message (optional filtering with cursor resume).
///
/// ```json
/// {
///   "subscribe": ["projection:gate_pipeline", "topic:agent.*"],
///   "cursor": 42,
///   "back_pressure": "at_most_once"
/// }
/// ```
#[derive(Deserialize)]
struct ClientMsg {
    #[serde(default)]
    subscribe: Vec<String>,
    /// Resume from this sequence number (replay events with seq >= cursor).
    #[serde(default)]
    cursor: Option<u64>,
    /// Per-connection back-pressure mode.
    #[serde(default)]
    back_pressure: Option<BackPressureMode>,
}

/// Main WebSocket handler — replay then stream.
async fn handle_ws(state: Arc<AppState>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut filter: Vec<String> = Vec::new();
    let mut replay_cursor: u64 = 0;
    let mut _back_pressure = BackPressureMode::AtMostOnce;

    // Wait for the first client message to get the cursor, or replay from 0.
    // We do an initial replay from 0; if the client sends a cursor later we
    // will not re-replay (the cursor is used for the initial catchup only via
    // the subscribe message).
    let backlog = state.event_bus.replay_from(replay_cursor);
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
    let mut last_lag_warn = Instant::now()
        .checked_sub(std::time::Duration::from_secs(10))
        .unwrap_or(Instant::now());
    let mut lag_total: u64 = 0;

    loop {
        tokio::select! {
            // Incoming client messages (filter subscriptions + cursor).
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(cmd) = serde_json::from_str::<ClientMsg>(&text) {
                            filter = cmd.subscribe;
                            if let Some(bp) = cmd.back_pressure {
                                _back_pressure = bp;
                            }
                            // If client provides a cursor, replay missed events.
                            if let Some(cursor) = cmd.cursor {
                                replay_cursor = cursor;
                                let catchup = state.event_bus.replay_from(replay_cursor);
                                for envelope in &catchup {
                                    if !filter.is_empty() && !matches_filter(&envelope.payload, &filter) {
                                        continue;
                                    }
                                    let Ok(json) = serde_json::to_string(&envelope.payload) else {
                                        continue;
                                    };
                                    if sink.send(Message::Text(json.into())).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            debug!(?filter, cursor = ?cmd.cursor, "ws client updated subscription");
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
                        lag_total += n;
                        if last_lag_warn.elapsed() >= std::time::Duration::from_secs(5) {
                            warn!(skipped = lag_total, "ws client lagged");
                            lag_total = 0;
                            last_lag_warn = Instant::now();
                        }
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
/// Filter strings support two forms:
///   1. **Plain type match** — the filter string is matched against the event's
///      serde `type` tag (substring match, backward-compatible).
///   2. **Channel prefix** — `projection:<name>`, `topic:<pattern>`, or
///      `engram-stream:<name>`.  These are matched against the serialized event's
///      type using glob-like semantics where `*` matches any suffix.
///
/// An empty filter accepts all events.
fn matches_filter(event: &crate::events::ServerEvent, filter: &[String]) -> bool {
    // Serialize to extract the "type" field cheaply.
    let Ok(val) = serde_json::to_value(event) else {
        return true;
    };
    let mut event_types = Vec::new();
    if let Some(event_type) = val.get("type").and_then(|t| t.as_str()) {
        event_types.push(event_type.to_string());
    }
    if event_types.iter().any(|t| t == "execution") {
        if let Some(exec_type) = val
            .get("event")
            .and_then(|event| event.get("type"))
            .and_then(|t| t.as_str())
        {
            event_types.push(exec_type.to_string());
        }
    }

    filter.iter().any(|f| {
        // Channel prefix patterns: `projection:gate_pipeline`, `topic:agent.*`
        if let Some(pattern) = f
            .strip_prefix("projection:")
            .or_else(|| f.strip_prefix("topic:"))
            .or_else(|| f.strip_prefix("engram-stream:"))
        {
            return channel_pattern_matches(&event_types, pattern);
        }
        // Legacy: plain substring match against event type tags.
        event_types
            .iter()
            .any(|event_type| event_type.contains(f.as_str()))
    })
}

/// Match a channel pattern against event types. Supports `*` wildcard suffix
/// (e.g., `agent.*` matches `agent.spawned`, `agent.output`).
fn channel_pattern_matches(event_types: &[String], pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return event_types
            .iter()
            .any(|t| t.starts_with(prefix) || t.contains(prefix));
    }
    event_types
        .iter()
        .any(|t| t == pattern || t.contains(pattern))
}
