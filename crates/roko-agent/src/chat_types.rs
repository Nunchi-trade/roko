//! Canonical provider-agnostic chat request types for adapter input.

use std::collections::HashMap;

use roko_core::tool::ToolCall;
use roko_core::{
    Body, ChatMessage, Engram, Kind, MessageContent, ToolCallFunction, ToolCallMessage, ToolDef,
};

use crate::usage::Usage;

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

/// Canonical response from any provider, after adapter parsing.
#[derive(Debug, Clone, Default)]
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

/// Provider/session continuity state carried across turns.
#[derive(Debug, Clone, Default)]
pub struct SessionState {
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ResponseMetadata {
    pub response_id: Option<String>,
    pub model_used: Option<String>,
    pub cached_tokens: Option<u64>,
    pub content_filter: Option<serde_json::Value>,
    pub web_search: Option<serde_json::Value>,
    pub provider_latency_ms: Option<u64>,
    pub raw_finish_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FinishReason {
    #[default]
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error(String),
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

    /// Convert the canonical chat response back into the orchestrator's signal format.
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
