//! Provider and model profile configuration sections.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::agent::ProviderKind;

use super::agent::{default_context_window, default_tool_format, default_true};

// ---- [providers.*] -------------------------------------------------------

/// Provider registry entry for `[providers.<name>]`.
///
/// A provider describes where requests go and how the runtime talks to that
/// endpoint. Use it to capture auth, transport, and provider-specific limits
/// without hardcoding them into Rust.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Protocol family used to talk to the provider.
    pub kind: ProviderKind,
    /// Base URL for HTTP providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Environment variable name holding the API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Command to spawn for CLI providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments passed to the CLI command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Hard request or subprocess timeout in milliseconds.
    #[serde(
        default = "default_provider_timeout_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_ms: Option<u64>,
    /// Time-to-first-token timeout in milliseconds.
    #[serde(
        default = "default_provider_ttft_timeout_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub ttft_timeout_ms: Option<u64>,
    /// TCP connection timeout in milliseconds.
    #[serde(
        default = "default_provider_connect_timeout_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub connect_timeout_ms: Option<u64>,
    /// Extra headers to inject on outbound requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<HashMap<String, String>>,
    /// Maximum concurrent requests allowed for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
}

pub(crate) const fn default_provider_timeout_ms() -> Option<u64> {
    Some(120_000)
}

pub(crate) const fn default_provider_ttft_timeout_ms() -> Option<u64> {
    Some(15_000)
}

pub(crate) const fn default_provider_connect_timeout_ms() -> Option<u64> {
    Some(5_000)
}

impl ProviderConfig {
    /// Resolve the API key from the environment variable named in `api_key_env`.
    #[must_use]
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key_env
            .as_ref()
            .and_then(|env_name| std::env::var(env_name).ok())
    }
}

// ---- [models.*] ----------------------------------------------------------

/// OpenRouter-specific routing overrides for a model profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderRouting {
    /// OpenRouter sort mode (`price`, `throughput`, `latency`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    /// Explicit provider order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
    /// Whether OpenRouter may fall back to alternate providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,
    /// Maximum cost per token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price: Option<f64>,
    /// Required provider parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<Vec<String>>,
}

/// Model registry entry for `[models.<name>]`.
///
/// A model binds a logical model name to a provider entry and the concrete
/// API slug that gets sent on the wire.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Key into the `[providers.*]` table.
    pub provider: String,
    /// Model ID sent to the API.
    pub slug: String,
    /// Context window in tokens.
    #[serde(default = "default_context_window")]
    pub context_window: u64,
    /// Maximum output tokens, if the provider/model sets one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    /// Whether the model supports tool calls.
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    /// Whether the model supports thinking/reasoning output.
    #[serde(default)]
    pub supports_thinking: bool,
    /// Whether the model supports vision inputs.
    #[serde(default)]
    pub supports_vision: bool,
    /// Whether the model supports web search.
    #[serde(default)]
    pub supports_web_search: bool,
    /// Whether the model supports MCP tools.
    #[serde(default)]
    pub supports_mcp_tools: bool,
    /// Whether the model supports partial continuation.
    #[serde(default)]
    pub supports_partial: bool,
    /// Whether the model supports Google Search grounding.
    #[serde(default)]
    pub supports_grounding: bool,
    /// Whether the model supports built-in code execution.
    #[serde(default)]
    pub supports_code_execution: bool,
    /// Whether the model supports provider-side context caching.
    #[serde(default)]
    pub supports_caching: bool,
    /// OpenRouter-specific routing overrides for this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_routing: Option<ProviderRouting>,
    /// Wire format used for tools.
    #[serde(default = "default_tool_format")]
    pub tool_format: String,
    /// Input token cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_input_per_m: Option<f64>,
    /// Output token cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_output_per_m: Option<f64>,
    /// Input token cost per million tokens for the high-context pricing tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_input_per_m_high: Option<f64>,
    /// Output token cost per million tokens for the high-context pricing tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_output_per_m_high: Option<f64>,
    /// Cache read cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cache_read_per_m: Option<f64>,
    /// Cache write cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cache_write_per_m: Option<f64>,
    /// Provider-specific reasoning depth label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    /// Maximum number of tools before behavior degrades.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tools: Option<u32>,
    /// Tokenizer ratio vs OpenAI `o200k_base`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_ratio: Option<f64>,
    /// Whether the model supports web-grounded search (Perplexity Sonar).
    #[serde(default)]
    pub supports_search: bool,
    /// Whether the model returns citations in responses (Perplexity Sonar).
    #[serde(default)]
    pub supports_citations: bool,
    /// Whether the model supports the async job API (Perplexity deep research).
    #[serde(default)]
    pub supports_async: bool,
    /// Whether this is an embedding model rather than a chat model.
    #[serde(default)]
    pub is_embedding_model: bool,
    /// Search context size hint: "low", "medium", or "high" (Perplexity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    /// Per-request fee in USD on top of token costs (Perplexity pricing model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_request: Option<f64>,
}

// ---- Gemini config -------------------------------------------------------

fn default_thinking_medium() -> String {
    "medium".to_string()
}

/// Gemini-specific model and request settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiConfig {
    /// Default model for standard Gemini chat requests.
    pub default_model: Option<String>,
    /// Default model for Gemini grounding requests.
    pub grounding_model: Option<String>,
    /// Default model for Gemini code execution requests.
    pub code_exec_model: Option<String>,
    /// Default Gemini embedding model.
    pub embed_model: Option<String>,
    /// Prefer the standard-tier free models when available.
    #[serde(default)]
    pub use_free_tier: bool,
    /// Gemini native thinking depth: "minimal", "low", "medium", or "high".
    #[serde(default = "default_thinking_medium")]
    pub thinking_level: String,
    /// Enable provider-side context caching when supported.
    #[serde(default)]
    pub enable_context_caching: bool,
    /// Per-category Gemini safety thresholds.
    #[serde(default)]
    pub safety_settings: Vec<SafetySetting>,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            default_model: None,
            grounding_model: None,
            code_exec_model: None,
            embed_model: None,
            use_free_tier: false,
            thinking_level: default_thinking_medium(),
            enable_context_caching: false,
            safety_settings: Vec::new(),
        }
    }
}

/// Gemini native safety configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetySetting {
    /// Gemini harm category, e.g. `HARM_CATEGORY_HATE_SPEECH`.
    pub category: String,
    /// Gemini blocking threshold, e.g. `BLOCK_NONE`.
    pub threshold: String,
}

// ---- Perplexity config ---------------------------------------------------

fn default_recency() -> String {
    "year".to_string()
}

/// Perplexity-specific search and model settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerplexityConfig {
    /// Default model for search-grounded queries.
    pub default_search_model: Option<String>,
    /// Default model for deep research tasks.
    pub default_research_model: Option<String>,
    /// Default model for reasoning tasks.
    pub default_reasoning_model: Option<String>,
    /// Default model for embeddings.
    pub default_embed_model: Option<String>,
    /// Recency filter applied to web search: "hour"/"day"/"week"/"month"/"year".
    #[serde(default = "default_recency")]
    pub search_recency_filter: String,
    /// Restrict results to academic sources.
    #[serde(default)]
    pub academic_mode: bool,
    /// Global domain allowlist for web search.
    #[serde(default)]
    pub search_domain_filter: Vec<String>,
    /// Include images in search results.
    #[serde(default)]
    pub return_images: bool,
    /// Include related questions in search results.
    #[serde(default = "default_true")]
    pub return_related_questions: bool,
}

impl Default for PerplexityConfig {
    fn default() -> Self {
        Self {
            default_search_model: None,
            default_research_model: None,
            default_reasoning_model: None,
            default_embed_model: None,
            search_recency_filter: default_recency(),
            academic_mode: false,
            search_domain_filter: Vec::new(),
            return_images: false,
            return_related_questions: true,
        }
    }
}
