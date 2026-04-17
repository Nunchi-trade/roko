#![deny(unsafe_code)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tower_http::trace::TraceLayer;
use tracing::warn;

pub mod protocol;
pub mod state;

use protocol::{AgentInboundFrame, RelayEvent, RelayMessageRequest, RelayOutboundFrame};
use state::{AwaitMessageError, BeginMessageError, RegisteredAgent, RelayState};

#[must_use]
pub fn app(state: Arc<RelayState>) -> Router {
    Router::new()
        .route("/relay/health", get(health))
        .route("/relay/agents", get(list_agents))
        .route("/relay/agents/ws", get(agent_ws))
        .route("/relay/cards/{id}", get(get_card))
        .route("/relay/messages", post(forward_message))
        .route("/relay/events/ws", get(events_ws))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn list_agents(State(state): State<Arc<RelayState>>) -> Json<Vec<protocol::ConnectedAgent>> {
    Json(state.list_agents())
}

async fn get_card(
    State(state): State<Arc<RelayState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    state.card(&id).map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn forward_message(
    State(state): State<Arc<RelayState>>,
    Json(request): Json<RelayMessageRequest>,
) -> Result<Json<protocol::RelayMessageResponse>, (StatusCode, Json<Value>)> {
    let pending = state.begin_message(request).map_err(begin_message_error)?;
    pending
        .await_response()
        .await
        .map(Json)
        .map_err(await_message_error)
}

async fn agent_ws(
    State(state): State<Arc<RelayState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_agent_socket(state, socket))
}

async fn events_ws(
    State(state): State<Arc<RelayState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_events_socket(state, socket))
}

fn begin_message_error(error: BeginMessageError) -> (StatusCode, Json<Value>) {
    match error {
        BeginMessageError::UnknownAgent => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown agent" })),
        ),
        BeginMessageError::NotConnected => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": "agent connection is not writable" })),
        ),
    }
}

fn await_message_error(error: AwaitMessageError) -> (StatusCode, Json<Value>) {
    match error {
        AwaitMessageError::Timeout => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({ "error": "agent response timed out" })),
        ),
        AwaitMessageError::Agent(error) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": error })),
        ),
    }
}

async fn handle_agent_socket(state: Arc<RelayState>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let first_frame = match next_text_frame(&mut stream).await {
        Some(text) => text,
        None => return,
    };

    let hello = match serde_json::from_str::<AgentInboundFrame>(&first_frame) {
        Ok(AgentInboundFrame::Hello(hello)) => hello,
        Ok(_) => {
            let _ = send_raw_json(
                &mut sink,
                json!({
                    "error": "first frame must be hello"
                }),
            )
            .await;
            let _ = sink.close().await;
            return;
        }
        Err(error) => {
            let _ = send_raw_json(
                &mut sink,
                json!({
                    "error": format!("invalid hello frame: {error}")
                }),
            )
            .await;
            let _ = sink.close().await;
            return;
        }
    };

    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<RelayOutboundFrame>();
    let RegisteredAgent {
        session_id,
        agent_id,
    } = state.register_agent(hello, outbound_tx.clone());

    let writer = tokio::spawn(async move {
        while let Some(frame) = outbound_rx.recv().await {
            let Ok(payload) = serde_json::to_string(&frame) else {
                continue;
            };
            if sink.send(Message::Text(payload.into())).await.is_err() {
                break;
            }
        }
    });

    let _ = outbound_tx.send(RelayOutboundFrame::Ack {
        event: "hello".to_string(),
    });

    while let Some(message) = stream.next().await {
        match message {
            Ok(Message::Text(text)) => {
                if !handle_agent_frame(&state, &agent_id, &outbound_tx, text.as_str()).await {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(error) => {
                warn!(%agent_id, %error, "relay websocket receive failed");
                break;
            }
        }
    }

    state.unregister_agent(&agent_id, session_id);
    writer.abort();
}

async fn handle_agent_frame(
    state: &Arc<RelayState>,
    agent_id: &str,
    outbound_tx: &mpsc::UnboundedSender<RelayOutboundFrame>,
    text: &str,
) -> bool {
    match serde_json::from_str::<AgentInboundFrame>(text) {
        Ok(AgentInboundFrame::Card { card, card_uri }) => {
            state.update_card(agent_id, card, card_uri);
            let _ = outbound_tx.send(RelayOutboundFrame::Ack {
                event: "card".to_string(),
            });
            true
        }
        Ok(AgentInboundFrame::Response {
            message_id,
            response,
        }) => {
            state.resolve_response(&message_id, Ok(response));
            true
        }
        Ok(AgentInboundFrame::Error { message_id, error }) => {
            state.agent_error(agent_id, message_id.clone(), error.clone());
            if message_id.is_none() {
                let _ = outbound_tx.send(RelayOutboundFrame::Error {
                    message_id: None,
                    error,
                });
            }
            true
        }
        Ok(AgentInboundFrame::Ping) => {
            let _ = outbound_tx.send(RelayOutboundFrame::Pong);
            true
        }
        Ok(AgentInboundFrame::Hello(_)) => {
            let _ = outbound_tx.send(RelayOutboundFrame::Error {
                message_id: None,
                error: "agent already registered on this socket".to_string(),
            });
            true
        }
        Err(error) => {
            let _ = outbound_tx.send(RelayOutboundFrame::Error {
                message_id: None,
                error: format!("invalid frame: {error}"),
            });
            true
        }
    }
}

async fn handle_events_socket(state: Arc<RelayState>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut events = state.subscribe_events();

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(error)) => {
                        warn!(%error, "relay events websocket receive failed");
                        break;
                    }
                    _ => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if send_event(&mut sink, &event).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        if send_raw_json(
                            &mut sink,
                            json!({
                                "type": "lagged",
                                "skipped": skipped,
                            }),
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    let _ = sink.close().await;
}

async fn send_event(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    event: &RelayEvent,
) -> Result<(), ()> {
    let payload = serde_json::to_value(event).map_err(|_| ())?;
    send_raw_json(sink, payload).await
}

async fn send_raw_json(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    payload: Value,
) -> Result<(), ()> {
    sink.send(Message::Text(payload.to_string().into()))
        .await
        .map_err(|_| ())
}

async fn next_text_frame(
    stream: &mut futures::stream::SplitStream<WebSocket>,
) -> Option<String> {
    loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => return Some(text.to_string()),
            Some(Ok(Message::Close(_))) | None => return None,
            Some(Ok(_)) => {}
            Some(Err(_)) => return None,
        }
    }
}
