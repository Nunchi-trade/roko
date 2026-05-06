//! Static registry of well-known built-in models.
//!
//! Provides [`BUILTIN_MODELS`] — a compile-time table of popular models across
//! Anthropic, OpenAI, Gemini, and Perplexity — and a [`builtin_model`] lookup
//! function that resolves by exact slug or common alias.

use crate::agent::ProviderKind;

/// A single entry in the built-in model registry.
#[derive(Debug, Clone)]
pub struct BuiltinModel {
    /// Wire slug sent to the provider API (e.g. `"claude-opus-4-6"`).
    pub slug: &'static str,
    /// Provider protocol family.
    pub provider_kind: ProviderKind,
    /// Context window in tokens.
    pub context_window: u64,
    /// Maximum output tokens.
    pub max_output: u64,
    /// Whether the model supports tool/function calling.
    pub supports_tools: bool,
    /// Whether the model supports image/vision inputs.
    pub supports_vision: bool,
    /// Whether the model supports thinking/reasoning output.
    pub supports_thinking: bool,
    /// Whether the model uses `max_completion_tokens` instead of `max_tokens`.
    pub use_max_completion_tokens: bool,
    /// Environment variable name holding the API key.
    pub api_key_env: &'static str,
}

/// All well-known built-in models.
pub static BUILTIN_MODELS: &[BuiltinModel] = &[
    // ── Anthropic ────────────────────────────────────────────────────────
    BuiltinModel {
        slug: "claude-opus-4-6",
        provider_kind: ProviderKind::AnthropicApi,
        context_window: 200_000,
        max_output: 32_000,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: true,
        use_max_completion_tokens: false,
        api_key_env: "ANTHROPIC_API_KEY",
    },
    BuiltinModel {
        slug: "claude-sonnet-4-6",
        provider_kind: ProviderKind::AnthropicApi,
        context_window: 200_000,
        max_output: 16_384,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: true,
        use_max_completion_tokens: false,
        api_key_env: "ANTHROPIC_API_KEY",
    },
    BuiltinModel {
        slug: "claude-haiku-4-5",
        provider_kind: ProviderKind::AnthropicApi,
        context_window: 200_000,
        max_output: 8_192,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: false,
        use_max_completion_tokens: false,
        api_key_env: "ANTHROPIC_API_KEY",
    },
    // ── OpenAI ───────────────────────────────────────────────────────────
    BuiltinModel {
        slug: "gpt-5.5",
        provider_kind: ProviderKind::OpenAiCompat,
        context_window: 200_000,
        max_output: 32_768,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: true,
        use_max_completion_tokens: true,
        api_key_env: "OPENAI_API_KEY",
    },
    BuiltinModel {
        slug: "gpt-5.4-mini",
        provider_kind: ProviderKind::OpenAiCompat,
        context_window: 200_000,
        max_output: 16_384,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: false,
        use_max_completion_tokens: true,
        api_key_env: "OPENAI_API_KEY",
    },
    BuiltinModel {
        slug: "o3",
        provider_kind: ProviderKind::OpenAiCompat,
        context_window: 200_000,
        max_output: 100_000,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: true,
        use_max_completion_tokens: true,
        api_key_env: "OPENAI_API_KEY",
    },
    BuiltinModel {
        slug: "o4-mini",
        provider_kind: ProviderKind::OpenAiCompat,
        context_window: 200_000,
        max_output: 100_000,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: true,
        use_max_completion_tokens: true,
        api_key_env: "OPENAI_API_KEY",
    },
    BuiltinModel {
        slug: "gpt-4o",
        provider_kind: ProviderKind::OpenAiCompat,
        context_window: 128_000,
        max_output: 16_384,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: false,
        use_max_completion_tokens: true,
        api_key_env: "OPENAI_API_KEY",
    },
    BuiltinModel {
        slug: "codex-mini",
        provider_kind: ProviderKind::OpenAiCompat,
        context_window: 200_000,
        max_output: 16_384,
        supports_tools: true,
        supports_vision: false,
        supports_thinking: true,
        use_max_completion_tokens: true,
        api_key_env: "OPENAI_API_KEY",
    },
    // ── Gemini ───────────────────────────────────────────────────────────
    BuiltinModel {
        slug: "gemini-2.5-pro",
        provider_kind: ProviderKind::GeminiApi,
        context_window: 1_048_576,
        max_output: 65_536,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: true,
        use_max_completion_tokens: false,
        api_key_env: "GEMINI_API_KEY",
    },
    BuiltinModel {
        slug: "gemini-2.5-flash",
        provider_kind: ProviderKind::GeminiApi,
        context_window: 1_048_576,
        max_output: 65_536,
        supports_tools: true,
        supports_vision: true,
        supports_thinking: true,
        use_max_completion_tokens: false,
        api_key_env: "GEMINI_API_KEY",
    },
    // ── Perplexity ───────────────────────────────────────────────────────
    BuiltinModel {
        slug: "sonar-pro",
        provider_kind: ProviderKind::PerplexityApi,
        context_window: 200_000,
        max_output: 8_000,
        supports_tools: false,
        supports_vision: false,
        supports_thinking: false,
        use_max_completion_tokens: false,
        api_key_env: "PERPLEXITY_API_KEY",
    },
    BuiltinModel {
        slug: "sonar",
        provider_kind: ProviderKind::PerplexityApi,
        context_window: 128_000,
        max_output: 8_000,
        supports_tools: false,
        supports_vision: false,
        supports_thinking: false,
        use_max_completion_tokens: false,
        api_key_env: "PERPLEXITY_API_KEY",
    },
];

/// Common aliases mapped to their canonical slug.
static ALIASES: &[(&str, &str)] = &[
    // Anthropic short names
    ("opus", "claude-opus-4-6"),
    ("claude-opus", "claude-opus-4-6"),
    ("sonnet", "claude-sonnet-4-6"),
    ("claude-sonnet", "claude-sonnet-4-6"),
    ("haiku", "claude-haiku-4-5"),
    ("claude-haiku", "claude-haiku-4-5"),
    // OpenAI short names
    ("gpt5", "gpt-5.5"),
    ("gpt-5", "gpt-5.5"),
    ("4o", "gpt-4o"),
    ("codex", "codex-mini"),
    // Gemini short names
    ("gemini-pro", "gemini-2.5-pro"),
    ("gemini-flash", "gemini-2.5-flash"),
    ("flash", "gemini-2.5-flash"),
];

/// Look up a built-in model by exact slug or common alias.
///
/// Returns `None` if the slug is not recognized.
#[must_use]
pub fn builtin_model(slug: &str) -> Option<&'static BuiltinModel> {
    // Exact match first.
    if let Some(m) = BUILTIN_MODELS.iter().find(|m| m.slug == slug) {
        return Some(m);
    }
    // Alias resolution.
    let canonical = ALIASES.iter().find(|(alias, _)| *alias == slug)?.1;
    BUILTIN_MODELS.iter().find(|m| m.slug == canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_slug_lookup() {
        let m = builtin_model("claude-opus-4-6").expect("opus by slug");
        assert_eq!(m.slug, "claude-opus-4-6");
        assert_eq!(m.provider_kind, ProviderKind::AnthropicApi);
        assert!(m.supports_thinking);
    }

    #[test]
    fn alias_lookup() {
        let m = builtin_model("opus").expect("opus alias");
        assert_eq!(m.slug, "claude-opus-4-6");

        let m = builtin_model("sonnet").expect("sonnet alias");
        assert_eq!(m.slug, "claude-sonnet-4-6");

        let m = builtin_model("haiku").expect("haiku alias");
        assert_eq!(m.slug, "claude-haiku-4-5");

        let m = builtin_model("flash").expect("flash alias");
        assert_eq!(m.slug, "gemini-2.5-flash");
    }

    #[test]
    fn unknown_returns_none() {
        assert!(builtin_model("nonexistent-model").is_none());
    }

    #[test]
    fn all_models_present() {
        assert_eq!(BUILTIN_MODELS.len(), 14);
    }

    #[test]
    fn openai_models_use_max_completion_tokens() {
        for m in BUILTIN_MODELS {
            if m.provider_kind == ProviderKind::OpenAiCompat {
                assert!(
                    m.use_max_completion_tokens,
                    "{} should use max_completion_tokens",
                    m.slug
                );
            }
        }
    }
}
