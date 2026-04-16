//! Messaging and stream routes.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use futures::StreamExt;
use roko_agent::{
    streaming::StreamChunk,
    tool_loop::LlmError,
    translate::{
        BackendResponse, FinishReason, RenderedTools, SessionState, normalize_finish_reason,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
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
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state.metrics().record_message();
    state.dispatcher().ok_or_else(missing_dispatcher)?;
    let backend = state.llm_backend().ok_or_else(missing_backend)?;
    let response = backend
        .send_turn(
            &request_messages(&request.prompt),
            &empty_tools(),
            &SessionState::default(),
        )
        .await
        .map_err(|error| dispatch_failed(&error))?;

    Ok(Json(json!({
        "response": response.extract_text(),
        "reasoning": response.extract_reasoning(),
        "usage": response.extract_usage(),
        "session": session_json(&backend.extract_session(&response)),
        "finish_reason": finish_reason_json(response_finish_reason(&response)),
        "engram_id": format!("engram-{}", Uuid::new_v4()),
        "context": request.context,
    })))
}

async fn stream(State(state): State<Arc<AgentState>>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_stream(socket, state))
}

async fn handle_stream(mut socket: WebSocket, state: Arc<AgentState>) {
    while let Some(message) = socket.next().await {
        match message {
            Ok(Message::Text(text)) => {
                state.metrics().record_message();
                if stream_prompt(&mut socket, &state, text.as_str())
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

fn missing_dispatcher() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "agent has no configured dispatcher"
        })),
    )
}

fn missing_backend() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "agent has no configured llm backend"
        })),
    )
}

fn dispatch_failed(error: &LlmError) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({
            "error": format!("dispatch failed: {error}")
        })),
    )
}

fn request_messages(prompt: &str) -> Vec<Value> {
    vec![json!({
        "role": "user",
        "content": prompt,
    })]
}

#[allow(clippy::missing_const_for_fn)]
fn empty_tools() -> RenderedTools {
    RenderedTools::JsonArray(json!([]))
}

fn session_json(session: &SessionState) -> Value {
    json!({
        "session_id": session.session_id,
        "thread_id": session.thread_id,
        "conversation_id": session.conversation_id,
    })
}

fn finish_reason_json(finish_reason: Option<FinishReason>) -> Value {
    finish_reason.map_or(Value::Null, |reason| {
        Value::String(match reason {
            FinishReason::Stop => "stop".to_string(),
            FinishReason::Length => "length".to_string(),
            FinishReason::ToolCalls => "tool_calls".to_string(),
            FinishReason::ContentFilter => "content_filter".to_string(),
            FinishReason::Error(reason) => reason,
        })
    })
}

fn response_finish_reason(response: &BackendResponse) -> Option<FinishReason> {
    match response {
        BackendResponse::Json(value) => value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .pointer("/candidates/0/finishReason")
                    .and_then(Value::as_str)
            })
            .map(normalize_finish_reason),
        BackendResponse::StreamJson(_) | BackendResponse::Text(_) => None,
    }
}

async fn send_socket_payload(socket: &mut WebSocket, payload: Value) -> Result<(), ()> {
    socket
        .send(Message::Text(payload.to_string().into()))
        .await
        .map_err(|_| ())
}

#[allow(clippy::too_many_lines)]
async fn stream_prompt(
    socket: &mut WebSocket,
    state: &Arc<AgentState>,
    prompt: &str,
) -> Result<(), ()> {
    if state.dispatcher().is_none() {
        return send_socket_payload(
            socket,
            json!({
                "error": "agent has no configured dispatcher",
                "done": true,
            }),
        )
        .await;
    }
    let Some(backend) = state.llm_backend().cloned() else {
        return send_socket_payload(
            socket,
            json!({
                "error": "agent has no configured llm backend",
                "done": true,
            }),
        )
        .await;
    };

    let messages = request_messages(prompt);
    let tools = empty_tools();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let stream_backend = Arc::clone(&backend);
    let stream_task = tokio::spawn(async move {
        stream_backend
            .send_turn_streaming(&messages, &tools, &SessionState::default(), event_tx)
            .await
    });
    let mut streamed_finish_reason = None;

    while let Some(chunk) = event_rx.recv().await {
        let payload = match chunk {
            StreamChunk::ReasoningDelta(reasoning) => json!({
                "reasoning": reasoning,
                "done": false,
            }),
            StreamChunk::ContentDelta(content) => json!({
                "chunk": content,
                "done": false,
            }),
            StreamChunk::ToolCallDelta {
                index,
                id_delta,
                name_delta,
                arguments_delta,
            } => json!({
                "tool_call": {
                    "index": index,
                    "id_delta": id_delta,
                    "name_delta": name_delta,
                    "arguments_delta": arguments_delta,
                },
                "done": false,
            }),
            StreamChunk::Usage(usage) => json!({
                "usage": usage,
                "done": false,
            }),
            StreamChunk::Done(finish_reason) => {
                streamed_finish_reason = Some(finish_reason);
                continue;
            }
            StreamChunk::Error(error) => json!({
                "error": error,
                "done": false,
            }),
        };

        send_socket_payload(socket, payload).await?;
    }

    let response = stream_task.await.map_err(|error| {
        tracing::warn!("message stream task failed: {error}");
    });
    match response {
        Ok(Ok(response)) => {
            send_socket_payload(
                socket,
                json!({
                    "done": true,
                    "session": session_json(&backend.extract_session(&response)),
                    "usage": response.extract_usage(),
                    "finish_reason": finish_reason_json(
                        streamed_finish_reason.or_else(|| response_finish_reason(&response))
                    ),
                }),
            )
            .await
        }
        Ok(Err(error)) => {
            send_socket_payload(
                socket,
                json!({
                    "error": format!("dispatch failed: {error}"),
                    "done": true,
                }),
            )
            .await
        }
        Err(()) => {
            send_socket_payload(
                socket,
                json!({
                    "error": "dispatch failed: stream task join failed",
                    "done": true,
                }),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use roko_agent::dispatcher::{HandlerResolver, ToolDispatcher};
    use roko_agent::tool_loop::LlmBackend;
    use roko_agent::translate::BackendResponse;
    use roko_core::tool::{ToolHandler, ToolRegistry, VecToolRegistry};

    enum FixedBackendResponse {
        Ok(BackendResponse),
        Err(&'static str),
    }

    struct FixedBackend {
        response: FixedBackendResponse,
    }

    #[async_trait]
    impl LlmBackend for FixedBackend {
        async fn send_turn(
            &self,
            _messages: &[Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            match &self.response {
                FixedBackendResponse::Ok(response) => Ok(response.clone()),
                FixedBackendResponse::Err(error) => Err(LlmError::Backend((*error).to_string())),
            }
        }
    }

    fn test_dispatcher() -> Arc<ToolDispatcher> {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::new());
        let resolver: Arc<dyn HandlerResolver> =
            Arc::new(|_name: &str| -> Option<Arc<dyn ToolHandler>> { None });
        Arc::new(ToolDispatcher::new(registry, resolver))
    }

    fn test_state(with_dispatcher: bool, backend: Option<Arc<dyn LlmBackend>>) -> Arc<AgentState> {
        let state = AgentState::new(
            "agent-1".to_string(),
            None,
            "0.1.0".to_string(),
            vec!["messaging".to_string()],
            None,
            backend,
            None,
        );
        let state = if with_dispatcher {
            state.with_dispatcher(test_dispatcher())
        } else {
            state
        };
        Arc::new(state)
    }

    #[tokio::test]
    async fn message_dispatches_to_backend() {
        let state = test_state(
            true,
            Some(Arc::new(FixedBackend {
                response: FixedBackendResponse::Ok(BackendResponse::Json(json!({
                    "choices": [{
                        "message": { "content": "mock response" },
                        "finish_reason": "stop",
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 4,
                    },
                    "session_id": "sess-1",
                }))),
            })),
        );

        let result = message(
            State(state),
            Json(MessageRequest {
                prompt: "hello".to_string(),
                context: MessageContext::default(),
            }),
        )
        .await
        .expect("dispatch ok");

        assert_eq!(result.0["response"], json!("mock response"));
        assert_eq!(result.0["finish_reason"], json!("stop"));
    }

    #[tokio::test]
    async fn message_returns_service_unavailable_without_dispatcher() {
        let state = test_state(
            false,
            Some(Arc::new(FixedBackend {
                response: FixedBackendResponse::Ok(BackendResponse::Json(json!({}))),
            })),
        );

        let error = message(
            State(state),
            Json(MessageRequest {
                prompt: "hello".to_string(),
                context: MessageContext::default(),
            }),
        )
        .await
        .expect_err("missing dispatcher");

        assert_eq!(error.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            error.1.0["error"],
            json!("agent has no configured dispatcher")
        );
    }

    #[tokio::test]
    async fn message_returns_bad_gateway_on_backend_error() {
        let state = test_state(
            true,
            Some(Arc::new(FixedBackend {
                response: FixedBackendResponse::Err("boom"),
            })),
        );

        let error = message(
            State(state),
            Json(MessageRequest {
                prompt: "hello".to_string(),
                context: MessageContext::default(),
            }),
        )
        .await
        .expect_err("backend error");

        assert_eq!(error.0, StatusCode::BAD_GATEWAY);
        assert_eq!(
            error.1.0["error"],
            json!("dispatch failed: backend error: boom")
        );
    }
}
