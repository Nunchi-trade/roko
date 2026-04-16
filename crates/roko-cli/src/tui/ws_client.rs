//! Lightweight websocket clients for the TUI Agents tab.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, Receiver, Sender, error::TryRecvError};
use tokio::task::JoinHandle;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderValue, header},
    },
};

const CHANNEL_CAPACITY: usize = 128;
const INITIAL_BACKOFF_SECS: u64 = 1;
const MAX_BACKOFF_SECS: u64 = 30;

/// One parsed chunk emitted to the foreground UI.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    /// The websocket handshake completed and the stream is live.
    Connected,
    /// Plain content delta from the agent.
    Text(String),
    /// Reasoning delta emitted by the backend.
    Reasoning(String),
    /// Tool-call delta emitted by the backend.
    ToolCall(Value),
    /// Usage payload emitted by the backend.
    Usage(Value),
    /// Stream-local error message.
    Error(String),
    /// Terminal frame with an optional session identifier.
    Done { session: Option<String> },
    /// The websocket disconnected and the client will reconnect.
    Disconnected,
}

/// Background websocket consumer for one agent's live stream tail.
pub struct AgentStreamClient {
    rx: Receiver<StreamChunk>,
    task: JoinHandle<()>,
}

impl AgentStreamClient {
    /// Connect to the global `roko-serve` event bus and filter for one agent.
    #[must_use]
    pub fn connect(
        agent_id: impl Into<String>,
        serve_base_url: &str,
        auth_token: Option<String>,
    ) -> Self {
        let agent_id = agent_id.into();
        let endpoint = event_bus_endpoint(serve_base_url);
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let task = tokio::spawn(run_event_bus(agent_id, endpoint, auth_token, tx));
        Self { rx, task }
    }

    /// Connect directly to a websocket endpoint.
    #[must_use]
    pub fn connect_direct(endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into();
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let task = tokio::spawn(run_direct(endpoint, tx));
        Self { rx, task }
    }

    /// Poll one ready chunk without blocking the UI thread.
    pub fn try_recv(&mut self) -> Result<StreamChunk, TryRecvError> {
        self.rx.try_recv()
    }
}

impl Drop for AgentStreamClient {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_direct(endpoint: String, tx: Sender<StreamChunk>) {
    run_loop(
        tx,
        move || {
            let endpoint = endpoint.clone();
            async move {
                let (socket, _) = connect_async(&endpoint).await.map_err(|error| {
                    tracing::debug!(%error, %endpoint, "agent stream connect failed");
                })?;
                Ok(socket)
            }
        },
        |text| parse_sidecar_frame(text),
    )
    .await;
}

async fn run_event_bus(
    agent_id: String,
    endpoint: String,
    auth_token: Option<String>,
    tx: Sender<StreamChunk>,
) {
    run_loop(
        tx,
        move || {
            let endpoint = endpoint.clone();
            let auth_token = auth_token.clone();
            async move {
                let request = build_request(&endpoint, auth_token.as_deref()).ok_or_else(|| {
                    tracing::debug!(%endpoint, "failed to build event bus websocket request");
                })?;
                let (mut socket, _) = connect_async(request).await.map_err(|error| {
                    tracing::debug!(%error, %endpoint, "event bus connect failed");
                })?;
                let subscribe = json!({
                    "subscribe": ["agent_output"]
                })
                .to_string();
                socket
                    .send(Message::Text(subscribe.into()))
                    .await
                    .map_err(|error| {
                        tracing::debug!(%error, %endpoint, "failed to subscribe to agent output");
                    })?;
                Ok(socket)
            }
        },
        move |text| parse_event_bus_frame(text, &agent_id),
    )
    .await;
}

async fn run_loop<F, Fut, P, S>(tx: Sender<StreamChunk>, connect: F, parse: P)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<tokio_tungstenite::WebSocketStream<S>, ()>>,
    P: Fn(&str) -> Vec<StreamChunk>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut backoff_secs = INITIAL_BACKOFF_SECS;

    loop {
        if tx.is_closed() {
            return;
        }

        match connect().await {
            Ok(mut socket) => {
                backoff_secs = INITIAL_BACKOFF_SECS;
                if send_chunk(&tx, StreamChunk::Connected).await.is_err() {
                    return;
                }
                pump_socket(&mut socket, &tx, &parse).await;
            }
            Err(()) => {
                if send_chunk(&tx, StreamChunk::Disconnected).await.is_err() {
                    return;
                }
            }
        }

        if tx.is_closed() {
            return;
        }

        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs.saturating_mul(2)).min(MAX_BACKOFF_SECS);
    }
}

async fn pump_socket<S, P>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    tx: &Sender<StreamChunk>,
    parse: &P,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    P: Fn(&str) -> Vec<StreamChunk>,
{
    loop {
        let message = match socket.next().await {
            Some(Ok(message)) => message,
            Some(Err(error)) => {
                tracing::debug!(%error, "agent stream receive failed");
                let _ = send_chunk(tx, StreamChunk::Disconnected).await;
                return;
            }
            None => {
                let _ = send_chunk(tx, StreamChunk::Disconnected).await;
                return;
            }
        };

        match message {
            Message::Text(text) => {
                if emit_chunks(&text, tx, parse).await {
                    return;
                }
            }
            Message::Binary(bytes) => {
                if let Ok(text) = String::from_utf8(bytes.to_vec())
                    && emit_chunks(&text, tx, parse).await
                {
                    return;
                }
            }
            Message::Ping(payload) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    let _ = send_chunk(tx, StreamChunk::Disconnected).await;
                    return;
                }
            }
            Message::Close(_) => {
                let _ = send_chunk(tx, StreamChunk::Disconnected).await;
                return;
            }
            Message::Pong(_) => {}
            _ => {}
        }
    }
}

async fn emit_chunks<P>(text: &str, tx: &Sender<StreamChunk>, parse: &P) -> bool
where
    P: Fn(&str) -> Vec<StreamChunk>,
{
    for chunk in parse(text) {
        let terminal = matches!(chunk, StreamChunk::Done { .. } | StreamChunk::Disconnected);
        if send_chunk(tx, chunk).await.is_err() {
            return true;
        }
        if terminal {
            return true;
        }
    }

    false
}

async fn send_chunk(tx: &Sender<StreamChunk>, chunk: StreamChunk) -> Result<(), ()> {
    tx.send(chunk).await.map_err(|_| ())
}

fn build_request(
    endpoint: &str,
    auth_token: Option<&str>,
) -> Option<tokio_tungstenite::tungstenite::http::Request<()>> {
    let mut request = endpoint.to_string().into_client_request().ok()?;
    if let Some(token) = auth_token.filter(|token| !token.trim().is_empty()) {
        let header_value = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, header_value);
    }
    Some(request)
}

fn event_bus_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}/ws")
    } else if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}/ws")
    } else if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        format!("{trimmed}/ws")
    } else {
        format!("ws://{trimmed}/ws")
    }
}

fn parse_event_bus_frame(text: &str, agent_id: &str) -> Vec<StreamChunk> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };

    if value.get("type").and_then(Value::as_str) != Some("agent_output") {
        return Vec::new();
    }
    if value.get("agent_id").and_then(Value::as_str) != Some(agent_id) {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    if let Some(content) = value.get("content").and_then(json_value_to_string)
        && !content.is_empty()
    {
        chunks.push(StreamChunk::Text(content));
    }
    if value.get("done").and_then(Value::as_bool).unwrap_or(false) {
        chunks.push(StreamChunk::Done { session: None });
    }
    chunks
}

fn parse_sidecar_frame(text: &str) -> Vec<StreamChunk> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return vec![StreamChunk::Text(text.to_string())];
    };

    let mut chunks = Vec::new();

    if let Some(reasoning) = value.get("reasoning").and_then(json_value_to_string) {
        chunks.push(StreamChunk::Reasoning(reasoning));
    }
    if let Some(tool_call) = value.get("tool_call") {
        chunks.push(StreamChunk::ToolCall(tool_call.clone()));
    }
    if let Some(usage) = value.get("usage") {
        chunks.push(StreamChunk::Usage(usage.clone()));
    }
    if let Some(error) = value.get("error").and_then(json_value_to_string) {
        chunks.push(StreamChunk::Error(error));
    }
    if let Some(chunk) = value.get("chunk").and_then(json_value_to_string) {
        chunks.push(StreamChunk::Text(chunk));
    }
    if value.get("done").and_then(Value::as_bool).unwrap_or(false) {
        let session = value
            .get("session")
            .and_then(Value::as_object)
            .and_then(|session| session.get("session_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        chunks.push(StreamChunk::Done { session });
    }

    if chunks.is_empty() {
        chunks.push(StreamChunk::Text(text.to_string()));
    }
    chunks
}

fn json_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        _ => serde_json::to_string(value).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::{
        Router,
        extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
    };
    use tokio::net::TcpListener;
    use tokio::time::{Duration, timeout};
    use tokio_tungstenite::accept_async;

    async fn spawn_mock_server() -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let handle = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept");
            let mut socket = accept_async(stream).await.expect("ws handshake");

            socket
                .send(Message::Text(
                    serde_json::json!({"chunk":"hello","done":false})
                        .to_string()
                        .into(),
                ))
                .await
                .expect("send text");
            socket
                .send(Message::Text(
                    serde_json::json!({"done":true}).to_string().into(),
                ))
                .await
                .expect("send done");
        });

        (format!("ws://{addr}/stream"), handle)
    }

    #[tokio::test]
    async fn receives_text_then_done_in_order() {
        let (endpoint, server) = spawn_mock_server().await;
        let mut client = AgentStreamClient::connect_direct(endpoint);

        let mut observed = Vec::new();
        let deadline = Duration::from_secs(5);
        let result = timeout(deadline, async {
            while observed.len() < 3 {
                match client.try_recv() {
                    Ok(chunk) => observed.push(chunk),
                    Err(TryRecvError::Empty) => tokio::task::yield_now().await,
                    Err(TryRecvError::Disconnected) => panic!("channel disconnected early"),
                }
            }
        })
        .await;

        result.expect("timed out");
        assert_eq!(
            observed,
            vec![
                StreamChunk::Connected,
                StreamChunk::Text("hello".to_string()),
                StreamChunk::Done { session: None }
            ]
        );

        server.abort();
    }

    async fn mock_event_bus(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket: WebSocket| async move {
            let _ = socket.next().await;
            let frames = [
                json!({
                    "type": "agent_output",
                    "agent_id": "agent-1",
                    "content": "visible",
                    "done": false,
                }),
                json!({
                    "type": "agent_output",
                    "agent_id": "agent-2",
                    "content": "hidden",
                    "done": false,
                }),
                json!({
                    "type": "agent_output",
                    "agent_id": "agent-1",
                    "content": "",
                    "done": true,
                }),
            ];
            for frame in frames {
                socket
                    .send(AxumMessage::Text(frame.to_string().into()))
                    .await
                    .expect("send frame");
            }
        })
    }

    #[tokio::test]
    async fn filters_event_bus_frames_to_the_target_agent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let app = Router::new().route("/ws", get(mock_event_bus));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        let mut client = AgentStreamClient::connect("agent-1", &format!("http://{addr}"), None);
        let mut observed = Vec::new();
        let result = timeout(Duration::from_secs(5), async {
            while observed.len() < 3 {
                match client.try_recv() {
                    Ok(chunk) => observed.push(chunk),
                    Err(TryRecvError::Empty) => tokio::task::yield_now().await,
                    Err(TryRecvError::Disconnected) => panic!("channel disconnected early"),
                }
            }
        })
        .await;

        result.expect("timed out");
        assert_eq!(
            observed,
            vec![
                StreamChunk::Connected,
                StreamChunk::Text("visible".to_string()),
                StreamChunk::Done { session: None }
            ]
        );

        server.abort();
    }
}
