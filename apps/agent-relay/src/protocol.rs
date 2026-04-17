use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum message timeout the relay will accept from clients.
pub const MAX_MESSAGE_TIMEOUT_MS: u64 = 60_000;

/// Default message timeout for forwarding requests.
pub const DEFAULT_MESSAGE_TIMEOUT_MS: u64 = 15_000;

/// Initial agent hello frame sent over the relay websocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHello {
    pub agent_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub rest_endpoint: Option<String>,
    #[serde(default)]
    pub card: Option<Value>,
    #[serde(default)]
    pub card_uri: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// Directory entry for a connected agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectedAgent {
    pub agent_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub rest_endpoint: Option<String>,
    #[serde(default)]
    pub card_uri: Option<String>,
    pub connected_at_ms: u64,
    pub relay_backed: bool,
}

/// HTTP request to forward a message to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayMessageRequest {
    pub agent_id: String,
    pub message: Value,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl RelayMessageRequest {
    #[must_use]
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
            .unwrap_or(DEFAULT_MESSAGE_TIMEOUT_MS)
            .clamp(1, MAX_MESSAGE_TIMEOUT_MS)
    }
}

/// HTTP response returned after a forwarded message completes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelayMessageResponse {
    pub message_id: String,
    pub agent_id: String,
    pub response: Value,
}

/// Frames the relay receives from agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentInboundFrame {
    Hello(AgentHello),
    Card {
        card: Value,
        #[serde(default)]
        card_uri: Option<String>,
    },
    Response {
        message_id: String,
        response: Value,
    },
    Error {
        #[serde(default)]
        message_id: Option<String>,
        error: String,
    },
    Ping,
}

/// Frames the relay sends to agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayOutboundFrame {
    Ack {
        event: String,
    },
    Message {
        message_id: String,
        message: Value,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        error: String,
    },
    Pong,
}

/// Optional dashboard event stream payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayEvent {
    AgentConnected { agent: ConnectedAgent },
    AgentDisconnected { agent_id: String },
    CardUpdated { agent_id: String, card_uri: String },
    MessageDelivered { agent_id: String, message_id: String },
    MessageResponded { agent_id: String, message_id: String },
    AgentError {
        agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        error: String,
    },
}
