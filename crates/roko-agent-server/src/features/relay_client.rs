//! Dedicated outbound relay bridge for presence, card hosting, and messaging.

use std::sync::Arc;

use agent_relay::protocol::{AgentHello, AgentInboundFrame, RelayOutboundFrame};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

use crate::registration::AgentCard;
use crate::state::{AgentState, DispatchError};

type RelaySocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Callback interface for receiving topic messages from the relay.
#[async_trait]
pub trait TopicHandler: Send + Sync + 'static {
    /// Called when a message arrives on a subscribed topic.
    async fn on_topic_message(
        &self,
        topic: &str,
        msg_type: &str,
        payload: serde_json::Value,
        publisher_id: Option<&str>,
        seq: u64,
    );
}

/// Handle returned from [`connect`] that allows sending pub/sub frames.
///
/// The handle is cheap to clone — each clone shares the same underlying
/// sender channel.  If the background relay task stops (e.g. the relay
/// disconnects), subsequent sends return an error but will never panic.
#[derive(Clone)]
pub struct RelayHandle {
    outbound_tx: mpsc::UnboundedSender<AgentInboundFrame>,
}

impl RelayHandle {
    /// Subscribe to a topic on the relay.
    ///
    /// # Errors
    ///
    /// Returns an error when the relay connection has been closed.
    pub fn subscribe(&self, topic: impl Into<String>) -> Result<()> {
        self.outbound_tx
            .send(AgentInboundFrame::Subscribe {
                topic: topic.into(),
            })
            .map_err(|_| anyhow!("relay connection closed"))
    }

    /// Unsubscribe from a previously subscribed topic.
    ///
    /// # Errors
    ///
    /// Returns an error when the relay connection has been closed.
    pub fn unsubscribe(&self, topic: impl Into<String>) -> Result<()> {
        self.outbound_tx
            .send(AgentInboundFrame::Unsubscribe {
                topic: topic.into(),
            })
            .map_err(|_| anyhow!("relay connection closed"))
    }

    /// Publish a message to a topic.  The relay fans the message out to all
    /// current subscribers of that topic.
    ///
    /// # Errors
    ///
    /// Returns an error when the relay connection has been closed.
    pub fn publish(
        &self,
        topic: impl Into<String>,
        msg_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.outbound_tx
            .send(AgentInboundFrame::Publish {
                topic: topic.into(),
                msg_type: msg_type.into(),
                payload,
            })
            .map_err(|_| anyhow!("relay connection closed"))
    }
}

/// Configuration for connecting an agent runtime to `agent-relay`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayClientConfig {
    /// Relay base URL or `/relay`-scoped base URL.
    pub base_url: String,
}

impl RelayClientConfig {
    /// Build relay-client configuration from a base URL.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    /// Build the externally visible card URI for `agent_id`.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured base URL does not use an HTTP(S)
    /// or WS(S) scheme.
    pub fn card_uri(&self, agent_id: &str) -> Result<String> {
        let http_base = http_base_url(&self.base_url)?;
        Ok(format!("{}/cards/{agent_id}", relay_base_path(&http_base)))
    }

    fn websocket_url(&self) -> Result<String> {
        let ws_base = websocket_base_url(&self.base_url)?;
        Ok(format!("{}/agents/ws", relay_base_path(&ws_base)))
    }
}

/// Connect an agent runtime to the relay and keep servicing relay messages.
///
/// Returns a [`RelayHandle`] that can be used to subscribe to topics,
/// unsubscribe, and publish messages after the connection is established.
/// Pass `topic_handler` to receive incoming [`RelayOutboundFrame::TopicMessage`]
/// frames; pass `None` if pub/sub delivery is not required.
///
/// # Errors
///
/// Returns an error if the websocket connection or hello handshake fails.
pub async fn connect(
    config: RelayClientConfig,
    state: Arc<AgentState>,
    card: AgentCard,
    topic_handler: Option<Arc<dyn TopicHandler>>,
) -> Result<RelayHandle> {
    let websocket_url = config.websocket_url()?;
    let card_uri = config.card_uri(state.agent_id())?;
    let relay_card = serde_json::to_value(&card)?;
    let (mut socket, _) = connect_async(websocket_url.as_str()).await?;
    send_frame(
        &mut socket,
        AgentInboundFrame::Hello(AgentHello {
            agent_id: state.agent_id().to_string(),
            name: Some(card.name.clone()),
            capabilities: card.capabilities.clone(),
            rest_endpoint: public_rest_endpoint(&card),
            card: None,
            card_uri: Some(card_uri),
            metadata: json!({
                "transport": "relay",
                "version": card.version,
            }),
        }),
    )
    .await?;
    await_hello_ack(&mut socket).await?;
    send_frame(
        &mut socket,
        AgentInboundFrame::Card {
            card: relay_card,
            card_uri: Some(config.card_uri(state.agent_id())?),
        },
    )
    .await?;

    // Channel used by `RelayHandle` to enqueue outbound pub/sub frames.
    let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<AgentInboundFrame>();

    tokio::spawn(async move {
        if let Err(error) = run(socket, state, topic_handler, outbound_rx).await {
            tracing::warn!(%error, "relay client stopped");
        }
    });

    Ok(RelayHandle { outbound_tx })
}

async fn run(
    mut socket: RelaySocket,
    state: Arc<AgentState>,
    topic_handler: Option<Arc<dyn TopicHandler>>,
    mut outbound_rx: mpsc::UnboundedReceiver<AgentInboundFrame>,
) -> Result<()> {
    loop {
        tokio::select! {
            // Incoming frames from the relay.
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let outbound: RelayOutboundFrame = serde_json::from_str(text.as_str())?;
                        handle_outbound_frame(&mut socket, &state, topic_handler.as_ref(), outbound).await?;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        socket.send(Message::Pong(payload)).await?;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error.into()),
                }
            }
            // Outbound pub/sub frames queued via RelayHandle.
            frame = outbound_rx.recv() => {
                match frame {
                    Some(frame) => {
                        let json = serde_json::to_string(&frame)?;
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    // All RelayHandle senders dropped — no more outbound frames possible.
                    None => break,
                }
            }
        }
    }
    Ok(())
}

async fn handle_outbound_frame(
    socket: &mut RelaySocket,
    state: &Arc<AgentState>,
    topic_handler: Option<&Arc<dyn TopicHandler>>,
    frame: RelayOutboundFrame,
) -> Result<()> {
    match frame {
        RelayOutboundFrame::Message {
            message_id,
            message,
        } => {
            let response = match dispatch_relay_message(state, message).await {
                Ok(response) => AgentInboundFrame::Response {
                    message_id,
                    response,
                },
                Err(error) => AgentInboundFrame::Error {
                    message_id: Some(message_id),
                    error,
                },
            };
            send_frame(socket, response).await?;
        }
        RelayOutboundFrame::TopicMessage {
            topic,
            msg_type,
            payload,
            publisher_id,
            seq,
        } => {
            tracing::debug!(%topic, %msg_type, seq, "received topic message");
            if let Some(handler) = topic_handler {
                handler
                    .on_topic_message(&topic, &msg_type, payload, publisher_id.as_deref(), seq)
                    .await;
            }
        }
        RelayOutboundFrame::Error { error, .. } => {
            tracing::warn!(%error, "relay reported outbound error");
        }
        RelayOutboundFrame::Pong | RelayOutboundFrame::Ack { .. } => {}
    }
    Ok(())
}

async fn dispatch_relay_message(state: &AgentState, message: Value) -> Result<Value, String> {
    let prompt = extract_prompt(&message)
        .ok_or_else(|| "relay message did not contain a string prompt".to_string())?;
    let response = state
        .dispatch_prompt(&prompt)
        .await
        .map_err(dispatch_error)?;
    Ok(json!({
        "response": response.content,
        "reasoning": response.reasoning,
        "usage": response.usage,
        "finish_reason": format_finish_reason(response.finish_reason),
        "session": {
            "session_id": response.session.session_id,
            "thread_id": response.session.thread_id,
            "conversation_id": response.session.conversation_id,
        }
    }))
}

fn dispatch_error(error: DispatchError) -> String {
    match error {
        DispatchError::NotConfigured => "agent has no configured dispatcher".to_string(),
        DispatchError::DispatchFailed(reason) => format!("dispatch failed: {reason}"),
    }
}

fn extract_prompt(message: &Value) -> Option<String> {
    match message {
        Value::String(prompt) => Some(prompt.clone()),
        Value::Object(_) => message
            .get("prompt")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| {
                message
                    .pointer("/messages/0/content")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            }),
        _ => None,
    }
}

fn format_finish_reason(finish_reason: roko_agent::chat_types::FinishReason) -> String {
    match finish_reason {
        roko_agent::chat_types::FinishReason::Stop => "stop".to_string(),
        roko_agent::chat_types::FinishReason::Length => "length".to_string(),
        roko_agent::chat_types::FinishReason::ToolCalls => "tool_calls".to_string(),
        roko_agent::chat_types::FinishReason::ContentFilter => "content_filter".to_string(),
        roko_agent::chat_types::FinishReason::Error(reason) => reason,
    }
}

fn public_rest_endpoint(card: &AgentCard) -> Option<String> {
    let endpoint = card.endpoints.rest.as_ref()?;
    is_public_endpoint(endpoint).then(|| endpoint.clone())
}

fn is_public_endpoint(endpoint: &str) -> bool {
    let normalized = endpoint.to_ascii_lowercase();
    ![
        "http://127.0.0.1",
        "https://127.0.0.1",
        "http://localhost",
        "https://localhost",
        "http://0.0.0.0",
        "https://0.0.0.0",
        "http://[::1]",
        "https://[::1]",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
}

fn relay_base_path(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/relay") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/relay")
    }
}

fn websocket_base_url(base_url: &str) -> Result<String> {
    if let Some(rest) = base_url.strip_prefix("http://") {
        return Ok(format!("ws://{}", rest.trim_end_matches('/')));
    }
    if let Some(rest) = base_url.strip_prefix("https://") {
        return Ok(format!("wss://{}", rest.trim_end_matches('/')));
    }
    if base_url.starts_with("ws://") || base_url.starts_with("wss://") {
        return Ok(base_url.trim_end_matches('/').to_string());
    }
    Err(anyhow!(
        "relay base_url must use http(s) or ws(s): {base_url}"
    ))
}

fn http_base_url(base_url: &str) -> Result<String> {
    if base_url.starts_with("http://") || base_url.starts_with("https://") {
        return Ok(base_url.trim_end_matches('/').to_string());
    }
    if let Some(rest) = base_url.strip_prefix("ws://") {
        return Ok(format!("http://{}", rest.trim_end_matches('/')));
    }
    if let Some(rest) = base_url.strip_prefix("wss://") {
        return Ok(format!("https://{}", rest.trim_end_matches('/')));
    }
    Err(anyhow!(
        "relay base_url must use http(s) or ws(s): {base_url}"
    ))
}

async fn await_hello_ack(socket: &mut RelaySocket) -> Result<()> {
    while let Some(frame) = socket.next().await {
        match frame? {
            Message::Text(text) => {
                let outbound: RelayOutboundFrame = serde_json::from_str(text.as_str())?;
                match outbound {
                    RelayOutboundFrame::Ack { event } if event == "hello" => return Ok(()),
                    RelayOutboundFrame::Error { error, .. } => {
                        return Err(anyhow!("relay hello rejected: {error}"));
                    }
                    RelayOutboundFrame::Pong | RelayOutboundFrame::Ack { .. } => {}
                    RelayOutboundFrame::Message { .. } => {
                        return Err(anyhow!("relay sent a message before hello ack"));
                    }
                    // A TopicMessage before hello ack is unexpected but harmless.
                    RelayOutboundFrame::TopicMessage { .. } => {}
                }
            }
            Message::Ping(payload) => {
                socket.send(Message::Pong(payload)).await?;
            }
            Message::Close(_) => return Err(anyhow!("relay websocket closed before hello ack")),
            _ => {}
        }
    }
    Err(anyhow!("relay websocket ended before hello ack"))
}

async fn send_frame(socket: &mut RelaySocket, frame: AgentInboundFrame) -> Result<()> {
    let payload = serde_json::to_string(&frame)?;
    socket.send(Message::Text(payload.into())).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::registration::AgentCardEndpoints;

    #[test]
    fn card_uri_uses_relay_path() {
        let config = RelayClientConfig::new("https://relay.example.test");
        assert_eq!(
            config.card_uri("agent-1").expect("card uri"),
            "https://relay.example.test/relay/cards/agent-1"
        );
    }

    #[test]
    fn relay_scoped_base_url_is_preserved() {
        let config = RelayClientConfig::new("https://relay.example.test/relay");
        assert_eq!(
            config.websocket_url().expect("ws url"),
            "wss://relay.example.test/relay/agents/ws"
        );
    }

    #[test]
    fn loopback_rest_endpoints_are_not_advertised() {
        let card = AgentCard {
            name: "agent-1".to_string(),
            capabilities: vec!["messaging".to_string()],
            endpoints: AgentCardEndpoints {
                rest: Some("http://127.0.0.1:8080".to_string()),
                websocket: None,
                a2a: None,
                mcp: None,
            },
            domain_tags: vec!["roko".to_string()],
            version: "1.0.0".to_string(),
        };
        assert_eq!(public_rest_endpoint(&card), None);
    }

    #[test]
    fn prompt_is_extracted_from_supported_message_shapes() {
        assert_eq!(
            extract_prompt(&json!({ "prompt": "hello" })),
            Some("hello".to_string())
        );
        assert_eq!(
            extract_prompt(&json!({
                "messages": [{ "content": "hello from transcript" }]
            })),
            Some("hello from transcript".to_string())
        );
        assert_eq!(
            extract_prompt(&Value::String("hello from string".to_string())),
            Some("hello from string".to_string())
        );
    }

    #[test]
    fn relay_handle_subscribe_returns_error_on_closed_channel() {
        // Create a sender whose receiver has been dropped, simulating a dead relay.
        let (tx, rx) = mpsc::unbounded_channel::<AgentInboundFrame>();
        drop(rx);
        let handle = RelayHandle { outbound_tx: tx };
        assert!(handle.subscribe("topic").is_err());
        assert!(handle.unsubscribe("topic").is_err());
        assert!(
            handle
                .publish("topic", "rate_update", serde_json::json!({}))
                .is_err()
        );
    }
}
