//! Canonical provider-agnostic chat request types for adapter input.

use std::collections::HashMap;

use roko_core::{ChatMessage, ToolDef};

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
