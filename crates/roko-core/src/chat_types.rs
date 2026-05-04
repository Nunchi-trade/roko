//! Canonical chat message and response types shared across prompt assembly,
//! tool loops, and provider adapters.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::tool::{ToolCall, ToolDef};
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

    /// Whether the cost is known (non-zero or provider-confirmed zero).
    ///
    /// Returns `false` when `cost_usd` is the uninitialised default (`0.0`)
    /// **and** there were tokens consumed — meaning cost was never computed.
    #[must_use]
    pub fn has_known_cost(&self) -> bool {
        self.cost_usd.abs() > f32::EPSILON || self.total_tokens() == 0
    }

    /// Compute cost from per-million token pricing when the provider did not
    /// report a dollar amount.
    ///
    /// This is a no-op when `cost_usd` is already non-zero.  When
    /// `input_per_m` **and** `output_per_m` are both `None`, the cost stays
    /// at zero so the display layer can distinguish "unknown" from "free".
    pub fn fill_cost_from_pricing(
        &mut self,
        input_per_m: Option<f64>,
        output_per_m: Option<f64>,
        cache_read_per_m: Option<f64>,
    ) {
        if self.cost_usd.abs() > f32::EPSILON {
            return; // already set (e.g. Claude CLI reports cost natively)
        }
        let (Some(inp), Some(out)) = (input_per_m, output_per_m) else {
            return; // no pricing data — leave at 0.0 so display shows "—"
        };
        let cache_r = cache_read_per_m.unwrap_or(inp * 0.1);

        let cost = (self.input_tokens as f64 * inp / 1_000_000.0)
            + (self.output_tokens as f64 * out / 1_000_000.0)
            + (self.cache_read_tokens as f64 * cache_r / 1_000_000.0);

        self.cost_usd = cost as f32;
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
            .tag("model", self.metadata.model_used.as_deref().unwrap_or(""))
            .tag("finish_reason", format!("{:?}", self.finish_reason))
            .build()
    }
}

// ─── Request-side types ──────────────────────────────────────────────────
//
// Canonical request shape shared by provider adapters.  Previously lived in
// `roko-agent/src/chat_types.rs`; moved here so downstream crates
// (roko-compose, roko-learn) can construct or inspect request objects
// without depending on roko-agent.

/// Canonical request shape shared by provider adapters.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Message history for the conversation turn.
    pub messages: Vec<ChatMessage>,
    /// Model slug to dispatch to.
    pub model_slug: String,
    /// Tool definitions available for the turn.
    pub tools: Vec<ToolDef>,
    /// How the model may call tools.
    pub tool_choice: ToolChoice,
    /// Maximum tokens for the completion.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f64>,
    /// Top-p (nucleus) sampling.
    pub top_p: Option<f64>,
    /// Stop sequences.
    pub stop: Option<Vec<String>>,
    /// Whether to stream the response.
    pub stream: bool,
    /// Additional per-request options.
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
    /// Enable extended thinking / chain-of-thought mode.
    pub enable_thinking: Option<bool>,
    /// Preserve thinking tokens in the response.
    pub preserve_thinking: Option<bool>,
    /// Enable streaming of tool call deltas.
    pub enable_tool_streaming: Option<bool>,
    /// Cache key hint for prompt caching.
    pub cache_key: Option<String>,
    /// Response format hint for structured output.
    pub response_format: Option<ResponseFormat>,
    /// Adapter-specific passthrough fields.
    pub extra: HashMap<String, serde_json::Value>,
}

/// Canonical policy for how the model may call tools.
#[derive(Debug, Clone)]
pub enum ToolChoice {
    /// Model decides whether and which tools to call.
    Auto,
    /// Model must not call any tools.
    None,
    /// Model must call at least one tool.
    Required,
    /// Model must call the named tool.
    Specific {
        /// Name of the required tool.
        name: String,
    },
}

/// Canonical response formatting hint for adapters that support it.
#[derive(Debug, Clone)]
pub enum ResponseFormat {
    /// Free-form text.
    Text,
    /// Structured JSON object.
    JsonObject,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_cost_from_pricing_computes_correctly() {
        let mut usage = Usage {
            input_tokens: 10_000,
            output_tokens: 2_000,
            cache_read_tokens: 500,
            cache_create_tokens: 0,
            cost_usd: 0.0,
            wall_ms: 100,
        };

        // gpt-5.4-mini pricing: $0.40/M input, $1.60/M output, $0.10/M cache read
        usage.fill_cost_from_pricing(Some(0.40), Some(1.60), Some(0.10));

        // Expected: (10000 * 0.40 / 1M) + (2000 * 1.60 / 1M) + (500 * 0.10 / 1M)
        //         = 0.004 + 0.0032 + 0.00005 = 0.00725
        let expected: f32 = 0.00725;
        assert!(
            (usage.cost_usd - expected).abs() < 1e-6,
            "expected {expected}, got {}",
            usage.cost_usd,
        );
    }

    #[test]
    fn fill_cost_does_not_overwrite_existing_cost() {
        let mut usage = Usage {
            input_tokens: 10_000,
            output_tokens: 2_000,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            cost_usd: 0.05, // already set (e.g. Claude CLI)
            wall_ms: 100,
        };

        usage.fill_cost_from_pricing(Some(0.40), Some(1.60), None);

        // Should remain at the original value
        assert!(
            (usage.cost_usd - 0.05).abs() < 1e-6,
            "cost should not be overwritten, got {}",
            usage.cost_usd,
        );
    }

    #[test]
    fn fill_cost_noop_when_no_pricing_data() {
        let mut usage = Usage {
            input_tokens: 10_000,
            output_tokens: 2_000,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            cost_usd: 0.0,
            wall_ms: 100,
        };

        // Both None -> no-op, cost stays 0.0 so display shows "—"
        usage.fill_cost_from_pricing(None, None, None);
        assert!(usage.cost_usd.abs() < f32::EPSILON);

        // Only input -> still no-op (need both)
        usage.fill_cost_from_pricing(Some(0.40), None, None);
        assert!(usage.cost_usd.abs() < f32::EPSILON);

        // Only output -> still no-op
        usage.fill_cost_from_pricing(None, Some(1.60), None);
        assert!(usage.cost_usd.abs() < f32::EPSILON);
    }

    #[test]
    fn fill_cost_defaults_cache_read_when_omitted() {
        let mut usage = Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 1_000_000,
            cache_create_tokens: 0,
            cost_usd: 0.0,
            wall_ms: 0,
        };

        // cache_read_per_m = None -> defaults to input_per_m * 0.1 = 0.04
        usage.fill_cost_from_pricing(Some(0.40), Some(1.60), None);

        let expected: f32 = 0.04; // 1M * 0.04 / 1M
        assert!(
            (usage.cost_usd - expected).abs() < 1e-6,
            "expected {expected}, got {}",
            usage.cost_usd,
        );
    }

    #[test]
    fn has_known_cost_true_when_nonzero() {
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.01,
            ..Usage::default()
        };
        assert!(usage.has_known_cost());
    }

    #[test]
    fn has_known_cost_true_when_zero_tokens() {
        // Zero tokens + zero cost = legitimately free (no work done)
        let usage = Usage::default();
        assert!(usage.has_known_cost());
    }

    #[test]
    fn has_known_cost_false_when_tokens_but_zero_cost() {
        // Tokens consumed but cost is 0.0 = cost was never computed
        let usage = Usage {
            input_tokens: 10_000,
            output_tokens: 2_000,
            cost_usd: 0.0,
            ..Usage::default()
        };
        assert!(!usage.has_known_cost());
    }
}
