//! Canonical provider-agnostic chat request types plus shared response re-exports.

use std::collections::HashMap;

use roko_core::{ChatMessage, Engram, MessageContent, ToolDef};

pub use roko_core::chat_types::{ChatResponse, FinishReason, ResponseMetadata, SessionState};

/// Canonical request shape shared by provider adapters.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub model_slug: String,
    pub tools: Vec<ToolDef>,
    pub tool_choice: ToolChoice,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop: Option<Vec<String>>,
    pub stream: bool,
    pub options: RequestOptions,
}

impl ChatRequest {
    /// Build a canonical chat request from the orchestrator's signal format.
    #[must_use]
    pub fn from_signal(
        signal: &Engram,
        model_slug: &str,
        system_prompt: Option<&str>,
        tools: Vec<ToolDef>,
        options: RequestOptions,
    ) -> Self {
        let mut messages = Vec::new();
        if let Some(system_prompt) = system_prompt {
            messages.push(ChatMessage::System {
                content: system_prompt.to_string(),
            });
        }

        messages.push(ChatMessage::User {
            content: MessageContent::Text(signal.body.as_text().unwrap_or_default().to_string()),
        });

        Self {
            messages,
            model_slug: model_slug.to_string(),
            tools,
            tool_choice: ToolChoice::Auto,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop: None,
            stream: false,
            options,
        }
    }
}

/// Provider-agnostic options plus adapter-specific passthrough fields.
#[derive(Debug, Clone, Default)]
pub struct RequestOptions {
    pub enable_thinking: Option<bool>,
    pub preserve_thinking: Option<bool>,
    pub enable_tool_streaming: Option<bool>,
    pub cache_key: Option<String>,
    pub response_format: Option<ResponseFormat>,
    pub extra: HashMap<String, serde_json::Value>,
}

/// Canonical policy for how the model may call tools.
#[derive(Debug, Clone)]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Specific { name: String },
}

/// Canonical response formatting hint for adapters that support it.
#[derive(Debug, Clone)]
pub enum ResponseFormat {
    Text,
    JsonObject,
}
