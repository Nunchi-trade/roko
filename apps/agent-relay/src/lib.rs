#![deny(unsafe_code)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
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

pub mod bus;
pub mod chain_watcher;
pub mod protocol;
pub mod state;

pub use bus::{TopicBus, TopicBusConfig};

use protocol::{
    AgentInboundFrame, RelayEvent, RelayMessageRequest, RelayOutboundFrame, TopicEnvelope,
};
use state::{AwaitMessageError, BeginMessageError, RegisteredAgent, RelayState};

pub fn app(state: Arc<RelayState>) -> Router {
    Router::new()
        .route("/relay/health", get(health))
        .route("/relay/agents", get(list_agents))
        .route("/relay/agents/ws", get(agent_ws))
        .route("/relay/cards/{id}", get(get_card))
        .route("/relay/messages", post(forward_message))
        .route("/relay/events/ws", get(events_ws))
        .route("/relay/workspaces", get(list_workspaces))
        .route("/relay/workspaces/register", post(register_workspace))
        .route(
            "/relay/workspaces/{id}/heartbeat",
            post(workspace_heartbeat),
        )
        .route(
            "/relay/workspaces/{id}",
            axum::routing::delete(unregister_workspace),
        )
        // Feed metadata endpoints (A5)
        .route("/relay/topics", get(list_topics))
        .route("/relay/topics/{topic}/messages", get(topic_messages))
        .route("/relay/topics/{topic}/subscribers", get(topic_subscribers))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn list_agents(State(state): State<Arc<RelayState>>) -> Json<Vec<protocol::ConnectedAgent>> {
    Json(state.list_agents())
}

async fn list_workspaces(
    State(state): State<Arc<RelayState>>,
) -> Json<Vec<protocol::ConnectedWorkspace>> {
    Json(state.list_workspaces())
}

async fn register_workspace(
    State(state): State<Arc<RelayState>>,
    Json(hello): Json<protocol::WorkspaceHello>,
) -> impl IntoResponse {
    state.register_workspace(hello);
    StatusCode::OK
}

async fn workspace_heartbeat(
    State(state): State<Arc<RelayState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agents_count = body
        .get("agents_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let agents_count = u32::try_from(agents_count).unwrap_or(u32::MAX);
    state.workspace_heartbeat(&id, agents_count);
    StatusCode::OK
}

async fn unregister_workspace(
    State(state): State<Arc<RelayState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    state.unregister_workspace(&id);
    StatusCode::OK
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

async fn agent_ws(State(state): State<Arc<RelayState>>, ws: WebSocketUpgrade) -> impl IntoResponse {
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
        AwaitMessageError::Agent(error) => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": error })))
        }
    }
}

async fn handle_agent_socket(state: Arc<RelayState>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let Some(first_frame) = next_text_frame(&mut stream).await else {
        return;
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
                if !handle_agent_frame(&state, &agent_id, &outbound_tx, text.as_str()) {
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

    state.bus.unsubscribe_all(&agent_id);
    state.unregister_agent(&agent_id, session_id);
    writer.abort();
}

fn handle_agent_frame(
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
        Ok(AgentInboundFrame::Subscribe { topic }) => {
            tracing::debug!(%agent_id, %topic, "subscribe");
            let replay = state.bus.subscribe(agent_id, &topic);
            for envelope in replay {
                let frame = RelayOutboundFrame::TopicMessage {
                    topic: envelope.topic,
                    msg_type: envelope.msg_type,
                    payload: envelope.payload,
                    publisher_id: envelope.publisher_id,
                    seq: envelope.seq,
                };
                if outbound_tx.send(frame).is_err() {
                    tracing::warn!(%agent_id, "failed to send replay — agent disconnected");
                    break;
                }
            }
            let _ = outbound_tx.send(RelayOutboundFrame::Ack {
                event: format!("subscribed:{topic}"),
            });
            true
        }
        Ok(AgentInboundFrame::Unsubscribe { topic }) => {
            tracing::debug!(%agent_id, %topic, "unsubscribe");
            state.bus.unsubscribe(agent_id, &topic);
            let _ = outbound_tx.send(RelayOutboundFrame::Ack {
                event: format!("unsubscribed:{topic}"),
            });
            true
        }
        Ok(AgentInboundFrame::Publish {
            topic,
            msg_type,
            payload,
        }) => {
            tracing::debug!(%agent_id, %topic, %msg_type, "publish");
            let envelope = TopicEnvelope::new(&topic, &msg_type, payload).with_publisher(agent_id);
            let (seq, subscribers) = state.bus.publish(envelope.clone());
            for sub_id in &subscribers {
                if sub_id == agent_id {
                    continue;
                }
                let frame = RelayOutboundFrame::TopicMessage {
                    topic: envelope.topic.clone(),
                    msg_type: envelope.msg_type.clone(),
                    payload: envelope.payload.clone(),
                    publisher_id: envelope.publisher_id.clone(),
                    seq,
                };
                state.send_to_agent(sub_id, frame);
            }
            let _ = outbound_tx.send(RelayOutboundFrame::Ack {
                event: format!("published:{topic}:{seq}"),
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

async fn next_text_frame(stream: &mut futures::stream::SplitStream<WebSocket>) -> Option<String> {
    loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => return Some(text.to_string()),
            Some(Ok(Message::Close(_)) | Err(_)) | None => return None,
            Some(Ok(_)) => {}
        }
    }
}

// ── Feed metadata endpoints (A5) ────────────────────────────────────────────

/// Query parameters for the `GET /relay/topics/:topic/messages` endpoint.
#[derive(Debug, serde::Deserialize)]
struct TopicMessagesQuery {
    /// Maximum number of messages to return (default 50, max 200).
    limit: Option<usize>,
}

/// `GET /relay/topics` — list all active topics with subscriber counts.
async fn list_topics(State(state): State<Arc<RelayState>>) -> Json<Value> {
    let mut stats = state.bus.topic_stats();
    stats.sort_by(|a, b| a.0.cmp(&b.0));
    let topics: Vec<Value> = stats
        .iter()
        .map(|(topic, count)| {
            json!({
                "topic": topic,
                "subscribers": count,
            })
        })
        .collect();
    Json(json!({ "topics": topics }))
}

/// `GET /relay/topics/:topic/messages` — get recent messages from the ring buffer.
async fn topic_messages(
    State(state): State<Arc<RelayState>>,
    Path(topic): Path<String>,
    Query(params): Query<TopicMessagesQuery>,
) -> Json<Value> {
    let limit = params.limit.unwrap_or(50).min(200);
    let messages: Vec<Value> = state
        .bus
        .peek_ring(&topic)
        .into_iter()
        .rev()
        .take(limit)
        .map(|env| {
            json!({
                "seq": env.seq,
                "topic": env.topic,
                "msg_type": env.msg_type,
                "payload": env.payload,
                "publisher_id": env.publisher_id,
                "timestamp_ms": env.timestamp_ms,
            })
        })
        .collect();
    Json(json!({ "topic": topic, "messages": messages }))
}

/// `GET /relay/topics/:topic/subscribers` — subscriber count for a topic.
async fn topic_subscribers(
    State(state): State<Arc<RelayState>>,
    Path(topic): Path<String>,
) -> Json<Value> {
    let count = state.bus.subscriber_count(&topic);
    Json(json!({
        "topic": topic,
        "subscriber_count": count,
    }))
}
