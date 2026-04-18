//! Canonical chat message and response types shared across prompt assembly,
//! tool loops, and provider adapters.

use serde::{Deserialize, Serialize};

use crate::tool::ToolCall;
use crate::{Body, Engram, Kind};

/// Canonical chat message format shared across prompt assembly, tool loops,
/// and provider adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum ChatMessage {
    /// System instruction applied to the turn.
    #[serde(rename = "system")]
    System {
        /// System prompt text.
        content: String,
    },

    /// User input, either plain text or multimodal blocks.
    #[serde(rename = "user")]
    User {
        /// User message content.
        content: MessageContent,
    },

    /// Assistant output, optionally carrying reasoning and tool calls.
    #[serde(rename = "assistant")]
    Assistant {
        /// Visible assistant text content.
        content: Option<String>,
        /// Provider-specific hidden reasoning text.
        reasoning_content: Option<String>,
        /// Function-style tool calls emitted by the assistant.
        tool_calls: Option<Vec<ToolCallMessage>>,
        /// Whether this assistant message is a partial continuation.
        #[serde(default)]
        partial: bool,
    },

    /// Tool result message keyed to a prior tool call.
    #[serde(rename = "tool")]
    Tool {
        /// Provider-issued tool call identifier.
        tool_call_id: String,
        /// Serialized tool result content.
        content: String,
    },
}

/// User content may be plain text or multimodal blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain-text content.
    Text(String),
    /// Multimodal content blocks.
    Blocks(Vec<ContentBlock>),
}

/// One multimodal content block in a user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// Plain-text block.
    #[serde(rename = "text")]
    Text {
        /// Text payload for this block.
        text: String,
    },
    /// Image reference block.
    #[serde(rename = "image_url")]
    ImageUrl {
        /// Image payload for this block.
        image_url: ImageUrl,
    },
}

/// Image URL payload for multimodal messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageUrl {
    /// Image source, typically a `data:` URL or remote HTTPS URL.
    pub url: String,
}

/// Tool call payload attached to an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallMessage {
    /// Provider-issued tool call identifier.
    pub id: String,
    /// Tool call kind, typically `"function"`.
    pub r#type: String,
    /// Function call details.
    pub function: ToolCallFunction,
}

/// Function name and JSON-stringified arguments for a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallFunction {
    /// Tool function name.
    pub name: String,
    /// JSON-stringified arguments object.
    pub arguments: String,
}

/// Usage metrics from a single agent or chat invocation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Input (prompt) tokens consumed.
    pub input_tokens: u32,
    /// Output (completion) tokens produced.
    pub output_tokens: u32,
    /// Cache-read tokens (from prompt caching, if supported).
    pub cache_read_tokens: u32,
    /// Cache-creation tokens (wrote to prompt cache).
    pub cache_create_tokens: u32,
    /// Estimated cost in USD.
    pub cost_usd: f32,
    /// Wall-clock duration in milliseconds.
    pub wall_ms: u64,
}

impl Usage {
    /// An empty usage record.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            cost_usd: 0.0,
            wall_ms: 0,
        }
    }

    /// Total tokens consumed (input + output + cache-create; excludes cache reads).
    #[must_use]
    pub const fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens + self.cache_create_tokens
    }

    /// Add another usage record into this one (for aggregating multi-turn runs).
    pub fn add(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_create_tokens += other.cache_create_tokens;
        self.cost_usd += other.cost_usd;
        self.wall_ms += other.wall_ms;
    }
}

/// Provider/session continuity state carried across turns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub conversation_id: Option<String>,
}

/// Provider-specific metadata normalized onto the shared response surface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResponseMetadata {
    pub response_id: Option<String>,
    pub model_used: Option<String>,
    pub cached_tokens: Option<u64>,
    pub content_filter: Option<serde_json::Value>,
    pub web_search: Option<serde_json::Value>,
    pub extra: Option<serde_json::Value>,
    pub provider_latency_ms: Option<u64>,
    pub raw_finish_reason: Option<String>,
}

/// Canonical finish reasons across provider families.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FinishReason {
    #[default]
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error(String),
}

/// Canonical provider-agnostic chat response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish_reason: FinishReason,
    pub metadata: ResponseMetadata,
    pub raw_assistant_message: Option<ChatMessage>,
    pub session: SessionState,
}

impl ChatResponse {
    /// Convert the response into a canonical assistant message for history.
    #[must_use]
    pub fn as_assistant_message(&self) -> ChatMessage {
        ChatMessage::Assistant {
            content: if self.content.is_empty() {
                None
            } else {
                Some(self.content.clone())
            },
            reasoning_content: self.reasoning.clone(),
            tool_calls: if self.tool_calls.is_empty() {
                None
            } else {
                Some(
                    self.tool_calls
                        .iter()
                        .map(|tc| ToolCallMessage {
                            id: tc.id.clone(),
                            r#type: "function".to_string(),
                            function: ToolCallFunction {
                                name: tc.name.clone(),
                                arguments: tc.arguments.to_string(),
                            },
                        })
                        .collect(),
                )
            },
            partial: false,
        }
    }

    /// Convert the canonical chat response back into an `AgentOutput` engram.
    #[must_use]
    pub fn to_signal(&self) -> Engram {
        Engram::builder(Kind::AgentOutput)
            .body(Body::text(&self.content))
            .tag(
                "model",
                self.metadata.model_used.as_deref().unwrap_or("unknown"),
            )
            .tag("finish_reason", format!("{:?}", self.finish_reason))
            .build()
    }
}
