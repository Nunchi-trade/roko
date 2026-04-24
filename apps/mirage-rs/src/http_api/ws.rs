//! WebSocket streaming endpoint — live pheromone and insight events.
//!
//! Connects to the internal `PheromoneBus` and `InsightBus` via broadcast sinks
//! and forwards serialized JSON events to the WebSocket client. Supports
//! optional filtering by event type via query parameters.
//!
//! # Wire format
//!
//! Each WebSocket text frame contains a JSON object with a `"channel"` field
//! indicating the event source:
//!
//! ```json
//! {"channel": "pheromone", "data": {"id": 1, "kind": "threat", ...}}
//! {"channel": "insight", "data": {"type": "posted", "id": "abc123", ...}}
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use super::ApiState;
use crate::chain_rpc::{insight_event_to_json, pheromone_event_to_json};
use crate::roko_bridge::{BackpressurePolicy, BroadcastSink, InsightEvent, PheromoneEvent};

/// Interval between server-initiated pings.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// If no pong is received within this duration, the connection is considered dead.
const PONG_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct WsParams {
    /// Subscribe to pheromone events (default true).
    #[serde(default = "default_true")]
    pub pheromones: bool,
    /// Subscribe to insight events (default true).
    #[serde(default = "default_true")]
    pub insights: bool,
    /// Subscribe to agent events (default false).
    #[serde(default)]
    pub agents: bool,
    /// Subscribe to prediction events (default false).
    #[serde(default)]
    pub predictions: bool,
    /// Optional agent ID filter for agent events.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Optional session ID filter for prediction events.
    #[serde(default)]
    pub session_id: Option<String>,
}

fn default_true() -> bool {
    true
}

/// WebSocket upgrade handler for `/api/ws`.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ApiState>,
    Query(params): Query<WsParams>,
) -> impl IntoResponse {
    tracing::info!(
        pheromones = params.pheromones,
        insights = params.insights,
        agents = params.agents,
        predictions = params.predictions,
        "WS upgrade request on /api/ws"
    );
    if !state.ws_registry.try_connect() {
        tracing::warn!("WS upgrade rejected: connection limit reached");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let registry = Arc::clone(&state.ws_registry);
    tracing::info!("WS connection accepted on /api/ws");
    ws.on_upgrade(move |socket| async move {
        handle_ws(socket, state, params).await;
        registry.disconnect();
        tracing::info!("WS connection closed on /api/ws");
    })
    .into_response()
}

async fn handle_ws(mut socket: WebSocket, state: ApiState, params: WsParams) {
    let Some(subs) = state.subs.clone() else {
        let _ = socket
            .send(Message::Text(
                serde_json::json!({"error": "streaming not available (no subscription buses)"})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    };

    // Set up broadcast sinks for each event type the client wants.
    let (mut pher_rx, mut pher_sub_id) = (None, None);
    if params.pheromones {
        let (sink, rx) = BroadcastSink::<PheromoneEvent>::new(256);
        let id = subs.register_pheromone_sink(Arc::new(sink), BackpressurePolicy::DropOldest);
        pher_rx = Some(rx);
        pher_sub_id = Some(id);
    }

    let (mut insi_rx, mut insi_sub_id) = (None, None);
    if params.insights {
        let (sink, rx) = BroadcastSink::<InsightEvent>::new(256);
        let id = subs.register_insight_sink(Arc::new(sink), BackpressurePolicy::DropOldest);
        insi_rx = Some(rx);
        insi_sub_id = Some(id);
    }

    // Subscribe to agent events if requested.
    let mut agent_rx = if params.agents {
        Some(state.chain.read().agent_bus.subscribe())
    } else {
        None
    };
    let agent_id_filter = params.agent_id.clone();
    let mut prediction_rx = if params.predictions {
        Some(state.chain.read().prediction_bus.subscribe())
    } else {
        None
    };
    let session_id_filter = params.session_id.clone();

    // Send initial confirmation.
    let _ = socket
        .send(Message::Text(
            serde_json::json!({
                "type": "connected",
                "pheromones": params.pheromones,
                "insights": params.insights,
                "agents": params.agents,
                "predictions": params.predictions,
            })
            .to_string()
            .into(),
        ))
        .await;

    // Heartbeat state.
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await; // consume the immediate first tick
    let mut last_pong = Instant::now();

    // Forward events from the buses to the WebSocket.
    loop {
        tokio::select! {
            // Pheromone events
            event = async {
                match pher_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    Ok(ev) => {
                        let payload = serde_json::json!({
                            "channel": "pheromone",
                            "data": pheromone_event_to_json(&ev),
                        });
                        if socket.send(Message::Text(payload.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let _ = socket.send(Message::Text(
                            serde_json::json!({"type": "lagged", "channel": "pheromone", "missed": n})
                                .to_string().into()
                        )).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }

            // Insight events
            event = async {
                match insi_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    Ok(ev) => {
                        let payload = serde_json::json!({
                            "channel": "insight",
                            "data": insight_event_to_json(&ev),
                        });
                        if socket.send(Message::Text(payload.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let _ = socket.send(Message::Text(
                            serde_json::json!({"type": "lagged", "channel": "insight", "missed": n})
                                .to_string().into()
                        )).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }

            // Agent events
            event = async {
                match agent_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    Ok(ev) => {
                        let event_agent_id = match &ev {
                            crate::chain::AgentEvent::Trace { agent_id, .. }
                            | crate::chain::AgentEvent::Heartbeat { agent_id, .. }
                            | crate::chain::AgentEvent::Stats { agent_id, .. }
                            | crate::chain::AgentEvent::Registered { agent_id, .. } => agent_id,
                        };
                        if let Some(ref wanted) = agent_id_filter {
                            if wanted != event_agent_id {
                                continue;
                            }
                        }
                        let payload = serde_json::json!({
                            "channel": "agent",
                            "data": ev,
                        });
                        if socket.send(Message::Text(payload.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let _ = socket.send(Message::Text(
                            serde_json::json!({"type": "lagged", "channel": "agent", "missed": n})
                                .to_string().into()
                        )).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }

            // Prediction events
            event = async {
                match prediction_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    Ok(ev) => {
                        let event_session_id = match &ev {
                            crate::chain::PredictionEvent::SessionCreated { session_id, .. }
                            | crate::chain::PredictionEvent::ClaimSubmitted { session_id, .. }
                            | crate::chain::PredictionEvent::SessionRegistered { session_id, .. }
                            | crate::chain::PredictionEvent::SessionResolved { session_id, .. } => session_id,
                        };
                        if let Some(ref wanted) = session_id_filter {
                            if wanted != event_session_id {
                                continue;
                            }
                        }
                        let payload = serde_json::json!({
                            "channel": "prediction",
                            "data": ev,
                        });
                        if socket.send(Message::Text(payload.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let _ = socket.send(Message::Text(
                            serde_json::json!({"type": "lagged", "channel": "prediction", "missed": n})
                                .to_string().into()
                        )).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }

            // Server-initiated ping / dead-connection check
            _ = ping_interval.tick() => {
                if last_pong.elapsed() > PONG_TIMEOUT {
                    tracing::debug!("WebSocket client failed pong timeout, closing");
                    break;
                }
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }

            // Client message or disconnect
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_pong = Instant::now();
                    }
                    _ => {} // ignore other messages
                }
            }
        }
    }

    // Cleanup: unsubscribe from buses.
    if let Some(id) = pher_sub_id {
        subs.unsubscribe(&id);
    }
    if let Some(id) = insi_sub_id {
        subs.unsubscribe(&id);
    }
}
