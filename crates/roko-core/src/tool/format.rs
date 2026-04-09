//! Per-model tool-call formats and profiles (§36.j, parity items 36.71–75).
//!
//! Research finding (see `tmp/roko-progress/tool-reserach.md` §1):
//! **Tool-call format preference is model-specific**. Using the same format
//! across backends leaves 5–30 percentage points of accuracy on the table
//! (ToolHop ACL 2025; WildToolBench 2025; Qwen3-coder documented format
//! switch above 5 tools).
//!
//! This module provides:
//! - [`ToolFormat`] — every format family a Roko translator might emit
//! - [`ToolFormatProfile`] — per-model metadata (preferred format, fallback
//!   chain, streaming quirks, tool-count degradation thresholds)
//! - [`profile_for_model`] — static lookup table covering Claude 4.x,
//!   GPT-5.x, Gemma 4, Qwen 3 / 3.5 / coder, Llama 4, Llama 3.2, Mistral
//!   7B / Small+, Phi, and an unknown-default fallback.
//!
//! Profiles are **priors** for the [`crate::tool::FormatBandit`] — they
//! seed initial arm rewards. The bandit then refines selection online
//! based on real success/latency/cost from execution traces.

#![allow(clippy::doc_lazy_continuation)] // wrapped-line continuations read as list items

use serde::{Deserialize, Serialize};

// ─── ToolFormat ───────────────────────────────────────────────────────────

/// A tool-call wire format emitted / parsed by a backend translator.
///
/// New formats can be added via [`Self::Custom`] without modifying this
/// crate. Canonical variants mirror the format families observed across
/// the current LLM landscape (as of early 2026).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolFormat {
    /// OpenAI `tools: [{type:"function",function:{name,parameters,...}}]`
    /// + `tool_calls[]` response. Used by GPT-4/5 and any OpenAI-compatible
    /// gateway (vLLM, TGI, most self-hosted servers).
    OpenAiJson,
    /// Anthropic `tool_use` content blocks within streaming events.
    /// Used by Claude 3.x / 4.x / 4.6.
    AnthropicBlocks,
    /// Hermes-style `<tool_call>{...}</tool_call>` JSON embedded in text.
    /// Qwen 3 base, NousResearch Hermes models.
    HermesJson,
    /// Gemma 4 special tokens:
    /// `<|tool_call|>call:fn{k:v}<|/tool_call|>`.
    Gemma4Tokens,
    /// Mistral control tokens: `[AVAILABLE_TOOLS]`, `[TOOL_CALLS]`,
    /// `[TOOL_RESULTS]`. Requires 9-digit tool-call IDs.
    MistralTokens,
    /// Llama 4 Pythonic function-call syntax: `[get_weather(location="SF")]`.
    Pythonic,
    /// Qwen 3.5 / Qwen3-coder XML: `<function=name><parameter=key>v</parameter></function>`.
    QwenXml,
    /// Plain ReAct text: `Thought:` / `Action:` / `Action Input:` /
    /// `Observation:` / `Final Answer:` loop, no special tokens.
    ReActText,
    /// Unconstrained JSON mode — the model emits a JSON object with a
    /// fixed schema, no native tool-calling envelope. Schema provides
    /// the structural ground truth via constrained decoding.
    JsonMode,
    /// Custom format identified by a dotted string, for extensions.
    Custom(String),
}

impl ToolFormat {
    /// Stable short identifier for logs / TUI / config.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::OpenAiJson => "openai_json",
            Self::AnthropicBlocks => "anthropic_blocks",
            Self::HermesJson => "hermes_json",
            Self::Gemma4Tokens => "gemma4_tokens",
            Self::MistralTokens => "mistral_tokens",
            Self::Pythonic => "pythonic",
            Self::QwenXml => "qwen_xml",
            Self::ReActText => "react_text",
            Self::JsonMode => "json_mode",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Whether this format relies on model-side native tool-calling support
    /// (as opposed to prompt-level ReAct-style emulation).
    #[must_use]
    pub const fn is_native(&self) -> bool {
        !matches!(self, Self::ReActText | Self::JsonMode | Self::Custom(_))
    }

    /// Whether this format is text-embedded (parsed from raw completion
    /// text) vs structured (carried in dedicated API fields).
    #[must_use]
    pub const fn is_text_embedded(&self) -> bool {
        matches!(
            self,
            Self::HermesJson
                | Self::Gemma4Tokens
                | Self::MistralTokens
                | Self::Pythonic
                | Self::QwenXml
                | Self::ReActText
        )
    }
}

impl std::fmt::Display for ToolFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─── ToolFormatProfile ────────────────────────────────────────────────────

/// Per-model tool-calling metadata.
///
/// Profiles are consumed by:
/// - Per-backend translators (§36.c) to choose a wire format
/// - The dispatcher to decide on streaming / tool-count caps
/// - The [`crate::tool::FormatBandit`] as initial arm priors
///
/// All fields are **priors** — at runtime the bandit and telemetry refine
/// the picture based on empirical outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolFormatProfile {
    /// Format the model was trained/fine-tuned on; the bandit's initial favorite.
    pub preferred: ToolFormat,
    /// Ordered fallback chain — each next entry is tried after
    /// `demotion_after_failures` consecutive failures of the previous.
    pub fallback_chain: Vec<ToolFormat>,
    /// Does the model claim to support native function calling at all?
    pub supports_tools: bool,
    /// Does the model handle ≥2 tool calls in a single turn correctly?
    pub parallel_safe: bool,
    /// Maximum number of tools in the registry before the model degrades
    /// (e.g. Qwen3-coder format-switches above 5). Dispatcher clamps to
    /// `min(this, config.max_tools_per_call)` via progressive discovery.
    pub max_tools_before_degrade: u8,
    /// Does the backend require `stream: false` when tools are present?
    /// (Ollama issues #9632, #12557 — streaming drops tool calls.)
    pub needs_stream_disabled: bool,
    /// Required length of the tool-call ID (Mistral needs exactly 9).
    /// `None` = any length is fine.
    pub tool_call_id_len: Option<u8>,
    /// How many consecutive format failures before demoting to the next
    /// entry in `fallback_chain`. Default 3 (browser-use waterfall).
    pub demotion_after_failures: u8,
}

impl ToolFormatProfile {
    /// Sensible default for an unknown model: ReAct text, tool-count cap 3,
    /// stream-disabled, demote-on-3-failures.
    #[must_use]
    pub fn unknown_default() -> Self {
        Self {
            preferred: ToolFormat::ReActText,
            fallback_chain: vec![ToolFormat::JsonMode],
            supports_tools: false,
            parallel_safe: false,
            max_tools_before_degrade: 3,
            needs_stream_disabled: true,
            tool_call_id_len: None,
            demotion_after_failures: 3,
        }
    }

    /// Is a given `tool_count` within the model's degradation threshold?
    #[must_use]
    pub const fn within_tool_limit(&self, tool_count: u8) -> bool {
        tool_count <= self.max_tools_before_degrade
    }
}

// ─── profile_for_model ────────────────────────────────────────────────────

/// Look up the static tool-format profile for a model slug.
///
/// Matching is prefix-based on normalized slug (e.g. `claude-sonnet-4-5`
/// matches the `claude-` family). Unknown slugs get
/// [`ToolFormatProfile::unknown_default`].
///
/// To override profile fields per model, merge a user config
/// `[models.<slug>.tool_format]` over the result of this function
/// (see §36.74).
#[must_use]
#[allow(clippy::too_many_lines)] // deliberate: this is a static family→profile table
pub fn profile_for_model(slug: &str) -> ToolFormatProfile {
    let slug = slug.trim();

    // Claude 4.x / 4.5 / 4.6 → Anthropic blocks
    if slug.starts_with("claude-")
        || slug.starts_with("opus-")
        || slug.starts_with("sonnet-")
        || slug.starts_with("haiku-")
    {
        return ToolFormatProfile {
            preferred: ToolFormat::AnthropicBlocks,
            fallback_chain: vec![
                ToolFormat::OpenAiJson,
                ToolFormat::JsonMode,
                ToolFormat::ReActText,
            ],
            supports_tools: true,
            parallel_safe: true,
            max_tools_before_degrade: 32,
            needs_stream_disabled: false,
            tool_call_id_len: None,
            demotion_after_failures: 3,
        };
    }

    // GPT-4 / GPT-5 / o1 / o3 → OpenAI JSON
    if slug.starts_with("gpt-")
        || slug.starts_with("o1")
        || slug.starts_with("o3")
        || slug.starts_with("o4")
    {
        return ToolFormatProfile {
            preferred: ToolFormat::OpenAiJson,
            fallback_chain: vec![ToolFormat::JsonMode, ToolFormat::ReActText],
            supports_tools: true,
            parallel_safe: true,
            max_tools_before_degrade: 64,
            needs_stream_disabled: false,
            tool_call_id_len: None,
            demotion_after_failures: 3,
        };
    }

    // Gemma 4 → special-token format
    if slug.starts_with("gemma-4") || slug.starts_with("gemma4") || slug.contains("gemma-4") {
        return ToolFormatProfile {
            preferred: ToolFormat::Gemma4Tokens,
            fallback_chain: vec![ToolFormat::JsonMode, ToolFormat::ReActText],
            supports_tools: true,
            parallel_safe: true,
            max_tools_before_degrade: 8,
            needs_stream_disabled: true,
            tool_call_id_len: None,
            demotion_after_failures: 3,
        };
    }

    // Qwen 3.5 / Qwen3-coder → XML
    if slug.starts_with("qwen3.5")
        || slug.starts_with("qwen-3.5")
        || slug.contains("qwen3-coder")
        || slug.contains("qwen-3-coder")
    {
        return ToolFormatProfile {
            preferred: ToolFormat::QwenXml,
            fallback_chain: vec![ToolFormat::HermesJson, ToolFormat::ReActText],
            supports_tools: true,
            parallel_safe: true,
            max_tools_before_degrade: 5,
            needs_stream_disabled: true,
            tool_call_id_len: None,
            demotion_after_failures: 3,
        };
    }

    // Qwen 3 (not 3.5) → Hermes JSON
    if slug.starts_with("qwen3") || slug.starts_with("qwen-3") || slug.starts_with("qwen2") {
        return ToolFormatProfile {
            preferred: ToolFormat::HermesJson,
            fallback_chain: vec![ToolFormat::JsonMode, ToolFormat::ReActText],
            supports_tools: true,
            parallel_safe: true,
            max_tools_before_degrade: 5,
            needs_stream_disabled: true,
            tool_call_id_len: None,
            demotion_after_failures: 3,
        };
    }

    // Llama 4 → Pythonic
    if slug.starts_with("llama4")
        || slug.starts_with("llama-4")
        || slug.contains("maverick")
        || slug.contains("scout")
    {
        return ToolFormatProfile {
            preferred: ToolFormat::Pythonic,
            fallback_chain: vec![ToolFormat::ReActText],
            supports_tools: true,
            parallel_safe: true,
            max_tools_before_degrade: 16,
            needs_stream_disabled: true,
            tool_call_id_len: None,
            demotion_after_failures: 3,
        };
    }

    // Llama 3.x / small llamas → unreliable, default to ReAct
    if slug.starts_with("llama3") || slug.starts_with("llama-3") || slug.starts_with("llama") {
        return ToolFormatProfile {
            preferred: ToolFormat::ReActText,
            fallback_chain: vec![ToolFormat::JsonMode],
            supports_tools: false,
            parallel_safe: false,
            max_tools_before_degrade: 3,
            needs_stream_disabled: true,
            tool_call_id_len: None,
            demotion_after_failures: 2,
        };
    }

    // Mistral 7B (unreliable at parallel)
    if slug.contains("mistral-7b") || slug.contains("mistral7b") {
        return ToolFormatProfile {
            preferred: ToolFormat::MistralTokens,
            fallback_chain: vec![ToolFormat::JsonMode, ToolFormat::ReActText],
            supports_tools: true,
            parallel_safe: false,
            max_tools_before_degrade: 5,
            needs_stream_disabled: true,
            tool_call_id_len: Some(9),
            demotion_after_failures: 2,
        };
    }

    // Other Mistral (Small / Medium / Large)
    if slug.starts_with("mistral") || slug.starts_with("mixtral") {
        return ToolFormatProfile {
            preferred: ToolFormat::MistralTokens,
            fallback_chain: vec![ToolFormat::OpenAiJson, ToolFormat::ReActText],
            supports_tools: true,
            parallel_safe: true,
            max_tools_before_degrade: 16,
            needs_stream_disabled: true,
            tool_call_id_len: Some(9),
            demotion_after_failures: 3,
        };
    }

    // Phi (small Microsoft models) → ReAct
    if slug.starts_with("phi-") || slug.starts_with("phi4") {
        return ToolFormatProfile {
            preferred: ToolFormat::ReActText,
            fallback_chain: vec![ToolFormat::JsonMode],
            supports_tools: false,
            parallel_safe: false,
            max_tools_before_degrade: 3,
            needs_stream_disabled: true,
            tool_call_id_len: None,
            demotion_after_failures: 2,
        };
    }

    // `ollama/…` prefix (local gateway) — treat as unknown but disable stream
    if slug.starts_with("ollama/") {
        return profile_for_model(slug.trim_start_matches("ollama/"));
    }

    ToolFormatProfile::unknown_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_as_str_stable() {
        assert_eq!(ToolFormat::OpenAiJson.as_str(), "openai_json");
        assert_eq!(ToolFormat::HermesJson.as_str(), "hermes_json");
        assert_eq!(ToolFormat::Gemma4Tokens.as_str(), "gemma4_tokens");
        assert_eq!(ToolFormat::ReActText.as_str(), "react_text");
        assert_eq!(ToolFormat::Custom("xyz".into()).as_str(), "xyz");
    }

    #[test]
    fn format_is_native_reflects_model_support() {
        assert!(ToolFormat::OpenAiJson.is_native());
        assert!(ToolFormat::AnthropicBlocks.is_native());
        assert!(ToolFormat::HermesJson.is_native());
        assert!(!ToolFormat::ReActText.is_native());
        assert!(!ToolFormat::JsonMode.is_native());
        assert!(!ToolFormat::Custom("x".into()).is_native());
    }

    #[test]
    fn format_is_text_embedded_identifies_prompt_parsed() {
        // Structured (carried in API fields)
        assert!(!ToolFormat::OpenAiJson.is_text_embedded());
        assert!(!ToolFormat::AnthropicBlocks.is_text_embedded());
        // Text-embedded (parsed from completion text)
        assert!(ToolFormat::HermesJson.is_text_embedded());
        assert!(ToolFormat::Gemma4Tokens.is_text_embedded());
        assert!(ToolFormat::MistralTokens.is_text_embedded());
        assert!(ToolFormat::Pythonic.is_text_embedded());
        assert!(ToolFormat::QwenXml.is_text_embedded());
        assert!(ToolFormat::ReActText.is_text_embedded());
    }

    #[test]
    fn format_serde_roundtrip() {
        for f in [
            ToolFormat::OpenAiJson,
            ToolFormat::AnthropicBlocks,
            ToolFormat::HermesJson,
            ToolFormat::Gemma4Tokens,
            ToolFormat::MistralTokens,
            ToolFormat::Pythonic,
            ToolFormat::QwenXml,
            ToolFormat::ReActText,
            ToolFormat::JsonMode,
            ToolFormat::Custom("vendor.x".into()),
        ] {
            let json = serde_json::to_string(&f).unwrap();
            let decoded: ToolFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, f);
        }
    }

    #[test]
    fn unknown_default_is_safe() {
        let p = ToolFormatProfile::unknown_default();
        assert_eq!(p.preferred, ToolFormat::ReActText);
        assert!(!p.supports_tools);
        assert!(!p.parallel_safe);
        assert_eq!(p.max_tools_before_degrade, 3);
        assert!(p.needs_stream_disabled);
    }

    #[test]
    fn profile_for_claude_model() {
        let p = profile_for_model("claude-sonnet-4-5");
        assert_eq!(p.preferred, ToolFormat::AnthropicBlocks);
        assert!(p.supports_tools);
        assert!(p.parallel_safe);
        assert_eq!(p.max_tools_before_degrade, 32);
        assert!(!p.needs_stream_disabled);
    }

    #[test]
    fn profile_for_gpt5_model() {
        let p = profile_for_model("gpt-5-high");
        assert_eq!(p.preferred, ToolFormat::OpenAiJson);
        assert!(p.supports_tools);
    }

    #[test]
    fn profile_for_gemma4_model() {
        let p = profile_for_model("gemma-4-27b");
        assert_eq!(p.preferred, ToolFormat::Gemma4Tokens);
        assert!(p.needs_stream_disabled);
        assert_eq!(p.max_tools_before_degrade, 8);
    }

    #[test]
    fn profile_for_qwen3_vs_qwen35_differ() {
        let p3 = profile_for_model("qwen3-32b");
        assert_eq!(p3.preferred, ToolFormat::HermesJson);

        let p35 = profile_for_model("qwen3.5-coder-32b");
        assert_eq!(p35.preferred, ToolFormat::QwenXml);

        // Both have 5-tool threshold
        assert_eq!(p3.max_tools_before_degrade, 5);
        assert_eq!(p35.max_tools_before_degrade, 5);
    }

    #[test]
    fn profile_for_llama4_pythonic() {
        let p = profile_for_model("llama4-maverick");
        assert_eq!(p.preferred, ToolFormat::Pythonic);
    }

    #[test]
    fn profile_for_small_llama_falls_back_to_react() {
        let p = profile_for_model("llama3.2-3b");
        assert_eq!(p.preferred, ToolFormat::ReActText);
        assert!(!p.supports_tools);
        assert_eq!(p.max_tools_before_degrade, 3);
    }

    #[test]
    fn profile_for_mistral_7b_is_serial_only() {
        let p = profile_for_model("mistral-7b-instruct");
        assert_eq!(p.preferred, ToolFormat::MistralTokens);
        assert!(!p.parallel_safe);
        assert_eq!(p.tool_call_id_len, Some(9));
    }

    #[test]
    fn profile_for_mistral_small_is_parallel_safe() {
        let p = profile_for_model("mistral-small-latest");
        assert!(p.parallel_safe);
        assert_eq!(p.tool_call_id_len, Some(9));
    }

    #[test]
    fn profile_for_unknown_is_safe_default() {
        let p = profile_for_model("some-unreleased-model-42b");
        assert_eq!(p.preferred, ToolFormat::ReActText);
        assert!(!p.supports_tools);
    }

    #[test]
    fn ollama_prefix_strips_and_looks_up() {
        let p = profile_for_model("ollama/qwen3-32b");
        assert_eq!(p.preferred, ToolFormat::HermesJson);
    }

    #[test]
    fn within_tool_limit_matches_threshold() {
        let p = profile_for_model("qwen3-32b");
        assert!(p.within_tool_limit(5));
        assert!(!p.within_tool_limit(6));
    }

    #[test]
    fn profile_serde_roundtrip() {
        let p = profile_for_model("claude-sonnet-4-5");
        let json = serde_json::to_string(&p).unwrap();
        let decoded: ToolFormatProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, p);
    }
}
