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
    chat_types::{ChatRequest, FinishReason, RequestOptions, ToolChoice},
    streaming::StreamChunk,
    translate::SessionState,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::state::{AgentState, DispatchError, MessageContext};

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
    let dispatcher = state.message_dispatcher().ok_or_else(missing_dispatcher)?;
    let response = dispatcher
        .dispatch(message_request(&request.prompt, false))
        .await
        .map_err(|error| dispatch_failed(&error))?;
    state
        .append_log_line(format!("message prompt={:?} status=ok", request.prompt))
        .await;

    Ok(Json(json!({
        "response": response.content,
        "reasoning": response.reasoning,
        "usage": response.usage,
        "session": session_json(&response.session),
        "finish_reason": finish_reason_json(Some(response.finish_reason)),
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

fn dispatch_failed(error: &DispatchError) -> (StatusCode, Json<Value>) {
    match error {
        DispatchError::NotConfigured => missing_dispatcher(),
        DispatchError::DispatchFailed(_) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": error.to_string(),
            })),
        ),
    }
}

fn message_request(prompt: &str, stream: bool) -> ChatRequest {
    ChatRequest {
        messages: vec![
            serde_json::from_value(json!({
                "role": "user",
                "content": prompt,
            }))
            .unwrap_or_else(|error| panic!("valid message request: {error}")),
        ],
        model_slug: String::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: None,
        stream,
        options: RequestOptions::default(),
    }
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
    let Some(dispatcher) = state.message_dispatcher() else {
        return send_socket_payload(
            socket,
            json!({
                "error": "agent has no configured dispatcher",
                "done": true,
            }),
        )
        .await;
    };

    let request = message_request(prompt, true);
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let stream_task =
        tokio::spawn(async move { dispatcher.dispatch_streaming(request, event_tx).await });

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
            StreamChunk::Done(_) => continue,
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
                    "session": session_json(&response.session),
                    "usage": response.usage,
                    "finish_reason": finish_reason_json(Some(response.finish_reason)),
                }),
            )
            .await
        }
        Ok(Err(error)) => {
            let payload = match error {
                DispatchError::NotConfigured => json!({
                    "error": "agent has no configured dispatcher",
                    "done": true,
                }),
                DispatchError::DispatchFailed(_) => json!({
                    "error": error.to_string(),
                    "done": true,
                }),
            };
            send_socket_payload(socket, payload).await
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
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use futures::SinkExt;
    use roko_agent::chat_types::{ChatResponse, FinishReason};
    use roko_agent::dispatcher::{HandlerResolver, ToolDispatcher};
    use roko_core::tool::{ToolHandler, ToolRegistry, VecToolRegistry};
    use tokio::{net::TcpListener, task::JoinHandle};
    use tokio_tungstenite::{
        MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message as ClientMessage,
    };
    use tower::ServiceExt;

    use crate::state::{DispatchError, DispatchLike};

    #[derive(Clone)]
    struct MockDispatcher {
        response: ChatResponse,
        stream_chunks: Vec<String>,
        error: Option<DispatchError>,
    }

    #[async_trait]
    impl DispatchLike for MockDispatcher {
        async fn dispatch(&self, _request: ChatRequest) -> Result<ChatResponse, DispatchError> {
            match &self.error {
                Some(error) => Err(error.clone()),
                None => Ok(self.response.clone()),
            }
        }

        async fn dispatch_streaming(
            &self,
            _request: ChatRequest,
            event_tx: mpsc::UnboundedSender<StreamChunk>,
        ) -> Result<ChatResponse, DispatchError> {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }

            for chunk in &self.stream_chunks {
                let _ = event_tx.send(StreamChunk::ContentDelta(chunk.clone()));
            }
            let _ = event_tx.send(StreamChunk::Done(FinishReason::Stop));
            Ok(self.response.clone())
        }
    }

    fn tool_dispatcher() -> Arc<ToolDispatcher> {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::new());
        let resolver: Arc<dyn HandlerResolver> =
            Arc::new(|_name: &str| -> Option<Arc<dyn ToolHandler>> { None });
        Arc::new(ToolDispatcher::new(registry, resolver))
    }

    fn chat_response(content: &str) -> ChatResponse {
        ChatResponse {
            content: content.to_string(),
            finish_reason: FinishReason::Stop,
            ..Default::default()
        }
    }

    fn test_state(
        with_tool_dispatcher: bool,
        dispatcher: Option<Arc<dyn DispatchLike>>,
    ) -> Arc<AgentState> {
        let mut state = AgentState::new(
            "agent-1".to_string(),
            None,
            "0.1.0".to_string(),
            vec!["messaging".to_string()],
            None,
            None,
            None,
        );
        if with_tool_dispatcher {
            state = state.with_dispatcher(tool_dispatcher());
        }
        if let Some(dispatcher) = dispatcher {
            state = state.with_message_dispatcher(dispatcher);
        }
        Arc::new(state)
    }

    fn message_request_json(body: Value) -> Request<Body> {
        Request::builder()
            .uri("/message")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&body).expect("json")
    }

    async fn spawn_ws_server(state: Arc<AgentState>) -> (std::net::SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let app = router().with_state(state);
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (addr, handle)
    }

    async fn next_ws_json(
        socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Value {
        loop {
            match socket.next().await {
                Some(Ok(ClientMessage::Text(text))) => {
                    return serde_json::from_str(&text).expect("ws json");
                }
                Some(Ok(_)) => continue,
                Some(Err(error)) => panic!("websocket error: {error}"),
                None => panic!("websocket closed"),
            }
        }
    }

    #[tokio::test]
    async fn message_with_mock_dispatcher_returns_real_content() {
        let state = test_state(
            true,
            Some(Arc::new(MockDispatcher {
                response: chat_response("Hello, test"),
                stream_chunks: Vec::new(),
                error: None,
            })),
        );
        let response = router()
            .with_state(state)
            .oneshot(message_request_json(json!({ "prompt": "ping" })))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["response"], json!("Hello, test"));
        assert_ne!(payload["response"], json!("agent-1: ping"));
    }

    #[tokio::test]
    async fn message_without_dispatcher_returns_503() {
        let response = router()
            .with_state(test_state(false, None))
            .oneshot(message_request_json(json!({ "prompt": "ping" })))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let payload = response_json(response).await;
        assert!(
            payload["error"]
                .as_str()
                .expect("error string")
                .contains("no configured dispatcher")
        );
    }

    #[tokio::test]
    async fn message_dispatch_error_returns_502() {
        let state = test_state(
            true,
            Some(Arc::new(MockDispatcher {
                response: ChatResponse::default(),
                stream_chunks: Vec::new(),
                error: Some(DispatchError::DispatchFailed("boom".to_string())),
            })),
        );
        let response = router()
            .with_state(state)
            .oneshot(message_request_json(json!({ "prompt": "ping" })))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let payload = response_json(response).await;
        assert!(
            payload["error"]
                .as_str()
                .expect("error string")
                .contains("dispatch failed")
        );
    }

    #[tokio::test]
    async fn message_preserves_context() {
        let state = test_state(
            true,
            Some(Arc::new(MockDispatcher {
                response: chat_response("Hello, test"),
                stream_chunks: Vec::new(),
                error: None,
            })),
        );
        let response = router()
            .with_state(state)
            .oneshot(message_request_json(json!({
                "prompt": "ping",
                "context": { "thread": "xyz" }
            })))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["context"]["thread"], json!("xyz"));
    }

    #[tokio::test]
    async fn stream_with_mock_dispatcher_streams_chunks() {
        let state = test_state(
            true,
            Some(Arc::new(MockDispatcher {
                response: chat_response("Hello, world"),
                stream_chunks: vec!["Hello".to_string(), ", ".to_string(), "world".to_string()],
                error: None,
            })),
        );
        let (addr, handle) = spawn_ws_server(state).await;
        let url = format!("ws://{addr}/stream");
        let (mut socket, _) = connect_async(&url).await.expect("connect websocket");

        socket
            .send(ClientMessage::Text("hi".to_string().into()))
            .await
            .expect("send websocket prompt");

        let first = tokio::time::timeout(Duration::from_secs(5), next_ws_json(&mut socket))
            .await
            .expect("first frame");
        let second = tokio::time::timeout(Duration::from_secs(5), next_ws_json(&mut socket))
            .await
            .expect("second frame");
        let third = tokio::time::timeout(Duration::from_secs(5), next_ws_json(&mut socket))
            .await
            .expect("third frame");
        let done = tokio::time::timeout(Duration::from_secs(5), next_ws_json(&mut socket))
            .await
            .expect("done frame");

        assert_eq!(first["chunk"], json!("Hello"));
        assert_eq!(second["chunk"], json!(", "));
        assert_eq!(third["chunk"], json!("world"));
        assert_eq!(done["done"], json!(true));

        socket.close(None).await.expect("close websocket");
        handle.abort();
    }
}
