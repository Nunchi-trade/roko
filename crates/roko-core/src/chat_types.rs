//! Canonical chat message types shared across prompt assembly, tool loops,
//! and provider adapters.

use serde::{Deserialize, Serialize};

/// Canonical chat message format shared across prompt assembly, tool loops,
/// and provider adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain-text content.
    Text(String),
    /// Multimodal content blocks.
    Blocks(Vec<ContentBlock>),
}

/// One multimodal content block in a user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    /// Image source, typically a `data:` URL or remote HTTPS URL.
    pub url: String,
}

/// Tool call payload attached to an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallMessage {
    /// Provider-issued tool call identifier.
    pub id: String,
    /// Tool call kind, typically `"function"`.
    pub r#type: String,
    /// Function call details.
    pub function: ToolCallFunction,
}

/// Function name and JSON-stringified arguments for a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    /// Tool function name.
    pub name: String,
    /// JSON-stringified arguments object.
    pub arguments: String,
}
