use chrono::Utc;
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayMessageResponse {
    pub message_id: String,
    pub agent_id: String,
    pub response: Value,
}

/// Descriptor for a data feed an agent can provide.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeedDescriptor {
    pub feed_id: String,
    pub topic: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub rate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
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
    /// Subscribe to a topic. Relay will forward matching TopicEnvelopes.
    Subscribe {
        topic: String,
    },
    /// Unsubscribe from a previously subscribed topic.
    Unsubscribe {
        topic: String,
    },
    /// Publish a message to a topic. Relay fans out to all subscribers.
    Publish {
        topic: String,
        msg_type: String,
        payload: serde_json::Value,
    },
    /// Register a data feed this agent provides.
    RegisterFeed {
        feed: FeedDescriptor,
    },
    /// Unregister a previously registered feed.
    UnregisterFeed {
        feed_id: String,
    },
}

/// Frames the relay sends to agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    /// A message published to a topic this agent is subscribed to.
    TopicMessage {
        topic: String,
        msg_type: String,
        payload: serde_json::Value,
        publisher_id: Option<String>,
        seq: u64,
    },
}

/// Internal representation of a published topic message.
/// Used within the relay bus; serialized as TopicMessage when sent to agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicEnvelope {
    pub topic: String,
    pub msg_type: String,
    pub payload: serde_json::Value,
    pub publisher_id: Option<String>,
    pub seq: u64,
    pub timestamp_ms: i64,
}

impl TopicEnvelope {
    pub fn new(
        topic: impl Into<String>,
        msg_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            topic: topic.into(),
            msg_type: msg_type.into(),
            payload,
            publisher_id: None,
            seq: 0,
            timestamp_ms: Utc::now().timestamp_millis(),
        }
    }

    pub fn with_publisher(mut self, id: impl Into<String>) -> Self {
        self.publisher_id = Some(id.into());
        self
    }

    pub fn with_seq(mut self, seq: u64) -> Self {
        self.seq = seq;
        self
    }
}

/// Workspace hello frame sent by roko-serve on startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHello {
    pub workspace_id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Public URL of the roko instance (e.g. `https://my-roko.up.railway.app`).
    pub url: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub owner_wallet: Option<String>,
    #[serde(default)]
    pub agents_count: u32,
}

/// Directory entry for a connected workspace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectedWorkspace {
    pub workspace_id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub url: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub owner_wallet: Option<String>,
    pub agents_count: u32,
    pub connected_at_ms: u64,
    pub last_heartbeat_ms: u64,
}

/// Optional dashboard event stream payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayEvent {
    AgentConnected {
        agent: ConnectedAgent,
    },
    AgentDisconnected {
        agent_id: String,
    },
    CardUpdated {
        agent_id: String,
        card_uri: String,
    },
    MessageDelivered {
        agent_id: String,
        message_id: String,
    },
    MessageResponded {
        agent_id: String,
        message_id: String,
    },
    AgentError {
        agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        error: String,
    },
    WorkspaceConnected {
        workspace: ConnectedWorkspace,
    },
    WorkspaceDisconnected {
        workspace_id: String,
    },
    WorkspaceHeartbeat {
        workspace_id: String,
        agents_count: u32,
    },
    FeedRegistered {
        agent_id: String,
        feed: FeedDescriptor,
    },
    FeedUnregistered {
        agent_id: String,
        feed_id: String,
    },
}
