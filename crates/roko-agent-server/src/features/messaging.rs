//! Messaging and stream routes.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
    routing::{get, post},
};
use futures::StreamExt;
use serde::Deserialize;
use uuid::Uuid;

use crate::state::{AgentState, MessageContext};

/// Messaging routes.
pub fn router() -> Router<Arc<AgentState>> {
    Router::new()
        .route("/message", post(message))
        .route("/stream", get(stream))
}

#[derive(Debug, Deserialize)]
struct MessageRequest {
    prompt: String,
    #[serde(default)]
    context: MessageContext,
}

async fn message(
    State(state): State<Arc<AgentState>>,
    Json(request): Json<MessageRequest>,
) -> Json<serde_json::Value> {
    state.metrics().record_message();
    let response = if request.prompt.trim().is_empty() {
        String::new()
    } else {
        format!("{}: {}", state.agent_id(), request.prompt.trim())
    };

    Json(serde_json::json!({
        "response": response,
        "engram_id": format!("engram-{}", Uuid::new_v4()),
        "context": request.context,
    }))
}

async fn stream(
    State(state): State<Arc<AgentState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_stream(socket, state))
}

async fn handle_stream(mut socket: WebSocket, state: Arc<AgentState>) {
    while let Some(message) = socket.next().await {
        match message {
            Ok(Message::Text(text)) => {
                state.metrics().record_message();
                let payload = serde_json::json!({
                    "agent_id": state.agent_id(),
                    "chunk": format!("{}: {}", state.agent_id(), text),
                    "done": true,
                });
                if socket
                    .send(Message::Text(payload.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }
}
