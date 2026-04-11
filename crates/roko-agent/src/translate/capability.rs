//! Model capability detector (§36.37).
//!
//! Wraps [`roko_core::tool::format::profile_for_model`] to produce a
//! [`ModelCapabilities`] snapshot, and maps a model slug to the
//! appropriate concrete [`Translator`].
//!
//! The mapping rule is:
//!
//! | `tool_format` (from profile)      | Translator            |
//! |-----------------------------------|-----------------------|
//! | `AnthropicBlocks`                 | [`ClaudeTranslator`]  |
//! | `OpenAiJson`                      | [`OllamaTranslator`]  |
//! | `ReActText`                       | [`ReActTranslator`]   |
//! | anything else, OR `supports_tools: false` | [`ReActTranslator`] (safe fallback) |
//!
//! `OpenAiJson` routes through [`OllamaTranslator`] deliberately — both
//! Ollama and generic `OpenAI`-compatible servers share the same
//! `tools: [...]` / `tool_calls[]` wire shape, so one translator covers
//! both until a dedicated `OpenAI` translator ships.

use std::sync::Arc;

use roko_core::config::schema::ModelProfile;
use roko_core::tool::{ToolFormat, format::profile_for_model};

use super::{ClaudeTranslator, OllamaTranslator, ReActTranslator, Translator};

/// Snapshot of a model's tool-calling capabilities.
///
/// Derived from [`roko_core::tool::format::profile_for_model`]. Used by
/// [`translator_for`] to pick the right translator for a model slug and
/// by the multi-turn loop to cap tool counts before degrading.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct ModelCapabilities {
    /// Whether the model supports any native tool-calling format.
    pub supports_tools: bool,
    /// Whether it's safe to emit multiple tool calls in one turn.
    pub supports_parallel_tool_calls: bool,
    /// The model's preferred wire format.
    pub tool_format: ToolFormat,
    /// Tool-count ceiling before the model starts misbehaving.
    pub max_tools_before_degrade: u8,
    /// Whether the model supports thinking / reasoning content.
    pub supports_thinking: bool,
    /// Whether the model supports vision inputs.
    pub supports_vision: bool,
    /// Whether the model supports built-in web search.
    pub supports_web_search: bool,
    /// Whether the model supports native MCP tools.
    pub supports_mcp_tools: bool,
    /// Whether the model supports partial continuation.
    pub supports_partial: bool,
    /// Whether the model supports streaming tool events.
    pub supports_tool_streaming: bool,
}

/// Return the capability snapshot for a model slug.
///
/// Thin wrapper around [`profile_for_model`] that projects the per-model
/// profile into a smaller, translator-facing shape.
#[must_use]
pub fn capabilities_for(slug: &str) -> ModelCapabilities {
    if slug.starts_with("glm-5") || slug == "glm-5.1" {
        return ModelCapabilities {
            supports_tools: true,
            supports_parallel_tool_calls: true,
            tool_format: ToolFormat::OpenAiJson,
            max_tools_before_degrade: 128,
            supports_thinking: true,
            supports_vision: false,
            supports_web_search: true,
            supports_mcp_tools: true,
            supports_partial: false,
            supports_tool_streaming: true,
        };
    }

    if slug.starts_with("kimi-k2") {
        return ModelCapabilities {
            supports_tools: true,
            supports_parallel_tool_calls: true,
            tool_format: ToolFormat::OpenAiJson,
            max_tools_before_degrade: 128,
            supports_thinking: true,
            supports_vision: slug.contains("k2.5") || slug.contains("k2-5"),
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: true,
            supports_tool_streaming: false,
        };
    }

    let profile = profile_for_model(slug);
    ModelCapabilities {
        supports_tools: profile.supports_tools,
        supports_parallel_tool_calls: profile.parallel_safe,
        tool_format: profile.preferred.clone(),
        max_tools_before_degrade: profile.max_tools_before_degrade,
        supports_thinking: false,
        supports_vision: false,
        supports_web_search: false,
        supports_mcp_tools: false,
        supports_partial: false,
        supports_tool_streaming: false,
    }
}

/// Project a full model profile into translator-facing capabilities.
#[must_use]
pub fn capabilities_from_profile(profile: &ModelProfile) -> ModelCapabilities {
    let tool_profile = profile_for_model(&profile.slug);
    let tool_format = tool_format_from_str(&profile.tool_format);
    let max_tools_before_degrade = profile
        .max_tools
        .and_then(|value| u8::try_from(value).ok())
        .unwrap_or(tool_profile.max_tools_before_degrade);
    ModelCapabilities {
        supports_tools: profile.supports_tools,
        supports_parallel_tool_calls: tool_profile.parallel_safe,
        tool_format,
        max_tools_before_degrade,
        supports_thinking: profile.supports_thinking,
        supports_vision: profile.supports_vision,
        supports_web_search: profile.supports_web_search,
        supports_mcp_tools: profile.supports_mcp_tools,
        supports_partial: profile.supports_partial,
        supports_tool_streaming: false,
    }
}

fn tool_format_from_str(tool_format: &str) -> ToolFormat {
    match tool_format.trim() {
        "openai_json" => ToolFormat::OpenAiJson,
        "anthropic_blocks" => ToolFormat::AnthropicBlocks,
        "hermes_json" => ToolFormat::HermesJson,
        "gemma4_tokens" => ToolFormat::Gemma4Tokens,
        "mistral_tokens" => ToolFormat::MistralTokens,
        "pythonic" => ToolFormat::Pythonic,
        "qwen_xml" => ToolFormat::QwenXml,
        "react_text" => ToolFormat::ReActText,
        "json_mode" => ToolFormat::JsonMode,
        other => ToolFormat::Custom(other.to_string()),
    }
}

/// Pick the translator best suited to a model slug.
///
/// Falls back to [`ReActTranslator`] whenever either (a) the model
/// doesn't support native tool-calling, or (b) Roko doesn't yet have a
/// dedicated translator for the model's preferred format.
#[must_use]
pub fn translator_for(slug: &str) -> Arc<dyn Translator> {
    let caps = capabilities_for(slug);
    if !caps.supports_tools {
        return Arc::new(ReActTranslator);
    }
    match caps.tool_format {
        ToolFormat::AnthropicBlocks => Arc::new(ClaudeTranslator),
        ToolFormat::OpenAiJson => Arc::new(OllamaTranslator),
        // `ReActText` plus every format without a dedicated translator yet
        // (HermesJson, Gemma4Tokens, MistralTokens, Pythonic, QwenXml,
        // JsonMode, Custom) all fall through to the ReAct fallback.
        _ => Arc::new(ReActTranslator),
    }
}

/// Short, stable name of the translator Roko picks for a model slug.
///
/// Returns one of `"claude"`, `"ollama"`, or `"react"`. Useful for logs,
/// TUI telemetry, and tests where the concrete type is hidden behind a
/// `dyn Translator` object.
///
/// This matches [`translator_for`] exactly.
#[must_use]
pub fn translator_name_for(slug: &str) -> &'static str {
    let caps = capabilities_for(slug);
    if !caps.supports_tools {
        return "react";
    }
    match caps.tool_format {
        ToolFormat::AnthropicBlocks => "claude",
        ToolFormat::OpenAiJson => "ollama",
        // ReActText + every format without a dedicated translator
        // (HermesJson, Gemma4Tokens, MistralTokens, Pythonic, QwenXml,
        // JsonMode, Custom) falls through to the ReAct fallback.
        _ => "react",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_for_claude_returns_anthropic_blocks() {
        let caps = capabilities_for("claude-opus-4-6");
        assert_eq!(caps.tool_format, ToolFormat::AnthropicBlocks);
        assert!(caps.supports_tools);
        assert!(caps.supports_parallel_tool_calls);

        let caps = capabilities_for("claude-sonnet-4-5");
        assert_eq!(caps.tool_format, ToolFormat::AnthropicBlocks);
        assert!(caps.supports_tools);
    }

    #[test]
    fn capabilities_for_openai_returns_openai_json() {
        let caps = capabilities_for("gpt-5");
        assert_eq!(caps.tool_format, ToolFormat::OpenAiJson);
        assert!(caps.supports_tools);
        assert!(caps.supports_parallel_tool_calls);

        let caps = capabilities_for("gpt-4.1");
        assert_eq!(caps.tool_format, ToolFormat::OpenAiJson);
        assert!(caps.supports_tools);
    }

    #[test]
    fn glm_capabilities_for_glm_51_returns_expected_profile() {
        let caps = capabilities_for("glm-5.1");
        assert!(caps.supports_tools);
        assert!(caps.supports_parallel_tool_calls);
        assert_eq!(caps.tool_format, ToolFormat::OpenAiJson);
        assert_eq!(caps.max_tools_before_degrade, 128);
        assert!(caps.supports_thinking);
        assert!(caps.supports_web_search);
        assert!(caps.supports_mcp_tools);
        assert!(caps.supports_tool_streaming);
    }

    #[test]
    fn kimi_capabilities_for_kimi_k25_returns_expected_profile() {
        let caps = capabilities_for("kimi-k2.5");
        assert!(caps.supports_tools);
        assert!(caps.supports_parallel_tool_calls);
        assert_eq!(caps.tool_format, ToolFormat::OpenAiJson);
        assert_eq!(caps.max_tools_before_degrade, 128);
        assert!(caps.supports_thinking);
        assert!(caps.supports_vision);
        assert!(caps.supports_partial);
        assert!(!caps.supports_web_search);
        assert!(!caps.supports_mcp_tools);
        assert!(!caps.supports_tool_streaming);
    }

    #[test]
    fn capabilities_for_unknown_falls_back_to_react() {
        // profile_for_model returns unknown_default() for unmatched slugs:
        // preferred = ReActText, supports_tools = false.
        let caps = capabilities_for("random-model-123");
        assert_eq!(caps.tool_format, ToolFormat::ReActText);
        assert!(!caps.supports_tools);
        assert!(!caps.supports_parallel_tool_calls);
        assert_eq!(caps.max_tools_before_degrade, 3);
    }

    #[test]
    fn capabilities_fields_match_profile() {
        for slug in [
            "claude-opus-4-6",
            "claude-sonnet-4-5",
            "gpt-5",
            "gpt-4.1",
            "random-model-123",
            "qwen3-32b",
            "mistral-7b-instruct",
        ] {
            let profile = profile_for_model(slug);
            let caps = capabilities_for(slug);
            assert_eq!(
                caps.supports_tools, profile.supports_tools,
                "supports_tools mismatch for {slug}"
            );
            assert_eq!(
                caps.supports_parallel_tool_calls, profile.parallel_safe,
                "parallel_safe mismatch for {slug}"
            );
            assert_eq!(
                caps.tool_format, profile.preferred,
                "tool_format mismatch for {slug}"
            );
        }
    }

    #[test]
    fn capabilities_from_profile_maps_all_fields() {
        let profile = ModelProfile {
            provider: "zai".to_string(),
            slug: "gpt-5".to_string(),
            context_window: 200_000,
            max_output: Some(131_072),
            supports_tools: true,
            supports_thinking: true,
            supports_vision: true,
            supports_web_search: true,
            supports_mcp_tools: true,
            supports_partial: true,
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: Some(1.40),
            cost_output_per_m: Some(4.40),
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: None,
            max_tools: Some(32),
            tokenizer_ratio: Some(1.0),
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        };

        let caps = capabilities_from_profile(&profile);
        assert!(caps.supports_tools);
        assert!(caps.supports_parallel_tool_calls);
        assert_eq!(caps.tool_format, ToolFormat::OpenAiJson);
        assert_eq!(caps.max_tools_before_degrade, 32);
        assert!(caps.supports_thinking);
        assert!(caps.supports_vision);
        assert!(caps.supports_web_search);
        assert!(caps.supports_mcp_tools);
        assert!(caps.supports_partial);
        assert!(!caps.supports_tool_streaming);
    }

    #[test]
    fn capabilities_max_tools_propagates() {
        // Claude profile: 32. GPT profile: 64. Qwen3: 5. Unknown: 3.
        assert_eq!(
            capabilities_for("claude-opus-4-6").max_tools_before_degrade,
            profile_for_model("claude-opus-4-6").max_tools_before_degrade
        );
        assert_eq!(
            capabilities_for("gpt-5").max_tools_before_degrade,
            profile_for_model("gpt-5").max_tools_before_degrade
        );
        assert_eq!(
            capabilities_for("qwen3-32b").max_tools_before_degrade,
            profile_for_model("qwen3-32b").max_tools_before_degrade
        );
        assert_eq!(
            capabilities_for("random-model-123").max_tools_before_degrade,
            profile_for_model("random-model-123").max_tools_before_degrade
        );
    }

    #[test]
    fn translator_for_claude_slug_uses_claude_translator() {
        // Trait objects can't be downcast, so we discriminate via
        // `.format()` — each translator advertises its own wire format.
        let t = translator_for("claude-opus-4-6");
        assert_eq!(t.format(), ToolFormat::AnthropicBlocks);

        let t = translator_for("claude-sonnet-4-5");
        assert_eq!(t.format(), ToolFormat::AnthropicBlocks);
    }

    #[test]
    fn translator_for_openai_slug_uses_ollama_translator() {
        let t = translator_for("gpt-5");
        assert_eq!(t.format(), ToolFormat::OpenAiJson);

        let t = translator_for("gpt-4.1");
        assert_eq!(t.format(), ToolFormat::OpenAiJson);
    }

    #[test]
    fn translator_for_unsupported_format_uses_react() {
        // A model whose profile says supports_tools: false → ReAct.
        // profile_for_model("llama3.2-3b") is one such case.
        let caps = capabilities_for("llama3.2-3b");
        assert!(
            !caps.supports_tools,
            "llama3.2-3b should not claim tool support"
        );
        let t = translator_for("llama3.2-3b");
        assert_eq!(t.format(), ToolFormat::ReActText);

        // Unknown model also falls through to ReAct.
        let t = translator_for("random-model-123");
        assert_eq!(t.format(), ToolFormat::ReActText);
    }

    #[test]
    fn translator_for_native_format_without_dedicated_translator_uses_react() {
        // Qwen3's preferred format is HermesJson — Roko doesn't have a
        // dedicated translator for it yet, so we fall through to ReAct.
        let caps = capabilities_for("qwen3-32b");
        assert_eq!(caps.tool_format, ToolFormat::HermesJson);
        assert!(caps.supports_tools);
        let t = translator_for("qwen3-32b");
        assert_eq!(t.format(), ToolFormat::ReActText);
    }

    #[test]
    fn translator_name_for_known_slugs() {
        assert_eq!(translator_name_for("claude-opus-4-6"), "claude");
        assert_eq!(translator_name_for("claude-sonnet-4-5"), "claude");
        assert_eq!(translator_name_for("gpt-5"), "ollama");
        assert_eq!(translator_name_for("gpt-4.1"), "ollama");
        assert_eq!(translator_name_for("random-model-123"), "react");
        assert_eq!(translator_name_for("llama3.2-3b"), "react");
        // HermesJson has no dedicated translator → react
        assert_eq!(translator_name_for("qwen3-32b"), "react");
    }

    #[test]
    fn translator_is_send_and_sync() {
        // Compile-time: `Arc<dyn Translator>` must already be Send+Sync
        // because the trait bounds include `Send + Sync`. This test
        // pins the invariant so refactors that loosen the bounds fail
        // at compile time.
        let _: Arc<dyn Translator + Send + Sync> = translator_for("claude-opus-4-6");
        let _: Arc<dyn Translator + Send + Sync> = translator_for("gpt-5");
        let _: Arc<dyn Translator + Send + Sync> = translator_for("random-model-123");
    }
}
