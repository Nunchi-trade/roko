//! Gemini provider adapter.
//!
//! This task wires Gemini into the provider abstraction and selects the
//! appropriate concrete agent shape based on model capabilities.
//! The embedding agent remains a thin wrapper for now.

use super::native::GeminiNativeAgent;
use crate::agent::{Agent, AgentResult};
use crate::codex_agent::{CodexAgent, DEFAULT_MAX_TOKENS};
use crate::provider::{AgentCreationError, AgentOptions, ProviderAdapter, ProviderError};
use async_trait::async_trait;
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use roko_core::{Context, Signal};
use serde_json::Value;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

fn compat_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{trimmed}/v1beta/openai")
}

fn resolved_timeout_ms(options: &AgentOptions) -> u64 {
    options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)
}

fn resolved_max_tokens(model: &ModelProfile) -> u32 {
    model
        .max_output
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(DEFAULT_MAX_TOKENS)
}

fn resolved_name(options: &AgentOptions, default_name: String) -> String {
    if options.name.is_empty() {
        default_name
    } else {
        options.name.clone()
    }
}

/// Embedding-only Gemini agent.
///
/// Stub implementation for adapter routing tests. A dedicated embeddings API
/// implementation is added in the follow-on Gemini embedding task.
pub struct GeminiEmbedAgent {
    inner: CodexAgent,
}

impl GeminiEmbedAgent {
    /// Construct a Gemini embedding agent stub.
    #[must_use]
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_slug: impl Into<String>,
    ) -> Self {
        let model_slug = model_slug.into();
        let name = format!("gemini-embed:{model_slug}");
        let inner = CodexAgent::new(api_key, &model_slug)
            .with_base_url(compat_base_url(&base_url.into()))
            .with_name(name);
        Self { inner }
    }

    /// Override the display name used for logs and tests.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.inner = self.inner.with_name(name);
        self
    }
}

#[async_trait]
impl Agent for GeminiEmbedAgent {
    async fn run(&self, input: &Signal, ctx: &Context) -> AgentResult {
        self.inner.run(input, ctx).await
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }
}

/// Gemini OpenAI-compatible agent.
///
/// Used for models that do not require native Gemini-only features.
pub struct GeminiCompatAgent {
    inner: CodexAgent,
}

impl GeminiCompatAgent {
    /// Construct a Gemini OpenAI-compatible agent.
    #[must_use]
    pub fn new(
        api_key: String,
        base_url: String,
        model: ModelProfile,
        options: &AgentOptions,
    ) -> Self {
        let name = resolved_name(options, format!("gemini-compat:{}", model.slug));
        let inner = CodexAgent::new(api_key, &model.slug)
            .with_base_url(compat_base_url(&base_url))
            .with_timeout_ms(resolved_timeout_ms(options))
            .with_max_tokens(resolved_max_tokens(&model))
            .with_name(name);
        Self { inner }
    }
}

#[async_trait]
impl Agent for GeminiCompatAgent {
    async fn run(&self, input: &Signal, ctx: &Context) -> AgentResult {
        self.inner.run(input, ctx).await
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }
}

/// Provider adapter for Gemini.
pub struct GeminiAdapter;

impl ProviderAdapter for GeminiAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::GeminiApi
    }

    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError> {
        let api_key = provider.resolve_api_key().ok_or_else(|| {
            AgentCreationError::MissingApiKey(
                provider
                    .api_key_env
                    .clone()
                    .unwrap_or_else(|| "GEMINI_API_KEY".into()),
            )
        })?;

        let base_url = provider
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let options = AgentOptions {
            timeout_ms: options.timeout_ms.or(provider.timeout_ms),
            ..options.clone()
        };

        if model.is_embedding_model {
            let agent = GeminiEmbedAgent::new(api_key, base_url, model.slug.clone());
            return Ok(if options.name.is_empty() {
                Box::new(agent)
            } else {
                Box::new(agent.with_name(options.name.clone()))
            });
        }

        let needs_native = model.supports_grounding
            || model.supports_code_execution
            || model.thinking_level.as_deref() == Some("dynamic");

        if needs_native {
            Ok(Box::new(GeminiNativeAgent::new(
                api_key,
                base_url,
                model.clone(),
                &options,
            )))
        } else {
            Ok(Box::new(GeminiCompatAgent::new(
                api_key,
                base_url,
                model.clone(),
                &options,
            )))
        }
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        match status {
            429 => ProviderError::RateLimit {
                retry_after_ms: body
                    .pointer("/error/details/0/retryDelay")
                    .and_then(Value::as_str)
                    .and_then(|s| s.trim_end_matches('s').parse::<f64>().ok())
                    .map(|seconds| (seconds * 1000.0) as u64),
            },
            401 | 403 => ProviderError::AuthFailure,
            404 => ProviderError::ModelNotFound,
            400 => {
                let msg = body
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if msg.contains("exceeds the maximum") || msg.contains("token limit") {
                    ProviderError::ContextOverflow
                } else {
                    ProviderError::Other(format!("Bad request: {msg}"))
                }
            }
            500..=599 => ProviderError::ServerError(status),
            _ => ProviderError::Other(format!("HTTP {status}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gemini_provider() -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::GeminiApi,
            base_url: Some("https://generativelanguage.googleapis.com".to_string()),
            api_key_env: Some("PATH".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(90_000),
            extra_headers: None,
            max_concurrent: None,
        }
    }

    fn base_model() -> ModelProfile {
        ModelProfile {
            provider: "gemini".to_string(),
            slug: "gemini-2.5-flash-lite".to_string(),
            context_window: 1_048_576,
            max_output: Some(65_536),
            supports_tools: true,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: None,
            max_tools: None,
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        }
    }

    fn named_options(name: &str) -> AgentOptions {
        AgentOptions {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn gemini_adapter_kind() {
        assert_eq!(GeminiAdapter.kind(), ProviderKind::GeminiApi);
    }

    #[test]
    fn gemini_adapter_routes_simple_models_to_compat_agent() {
        let agent = GeminiAdapter
            .create_agent(&gemini_provider(), &base_model(), &named_options(""))
            .expect("create compat agent");
        assert_eq!(agent.name(), "gemini-compat:gemini-2.5-flash-lite");
    }

    #[test]
    fn gemini_adapter_routes_grounding_models_to_native_agent() {
        let model = ModelProfile {
            supports_grounding: true,
            slug: "gemini-3-flash-preview".to_string(),
            ..base_model()
        };
        let agent = GeminiAdapter
            .create_agent(&gemini_provider(), &model, &named_options(""))
            .expect("create native agent");
        assert_eq!(agent.name(), "gemini-native:gemini-3-flash-preview");
    }

    #[test]
    fn gemini_adapter_routes_code_execution_models_to_native_agent() {
        let model = ModelProfile {
            supports_code_execution: true,
            slug: "gemini-2.5-pro".to_string(),
            ..base_model()
        };
        let agent = GeminiAdapter
            .create_agent(&gemini_provider(), &model, &named_options(""))
            .expect("create native agent");
        assert_eq!(agent.name(), "gemini-native:gemini-2.5-pro");
    }

    #[test]
    fn gemini_adapter_routes_dynamic_thinking_models_to_native_agent() {
        let model = ModelProfile {
            slug: "gemini-3.1-pro-preview".to_string(),
            thinking_level: Some("dynamic".to_string()),
            ..base_model()
        };
        let agent = GeminiAdapter
            .create_agent(&gemini_provider(), &model, &named_options(""))
            .expect("create native agent");
        assert_eq!(agent.name(), "gemini-native:gemini-3.1-pro-preview");
    }

    #[test]
    fn gemini_adapter_routes_embedding_models_to_embed_agent() {
        let model = ModelProfile {
            is_embedding_model: true,
            slug: "gemini-embedding-2-preview".to_string(),
            ..base_model()
        };
        let agent = GeminiAdapter
            .create_agent(&gemini_provider(), &model, &named_options(""))
            .expect("create embed agent");
        assert_eq!(agent.name(), "gemini-embed:gemini-embedding-2-preview");
    }

    #[test]
    fn gemini_adapter_uses_custom_name_override() {
        let agent = GeminiAdapter
            .create_agent(
                &gemini_provider(),
                &base_model(),
                &named_options("gemini-custom"),
            )
            .expect("create named agent");
        assert_eq!(agent.name(), "gemini-custom");
    }

    #[test]
    fn gemini_adapter_missing_api_key_uses_default_env_name() {
        let provider = ProviderConfig {
            api_key_env: None,
            ..gemini_provider()
        };

        let Err(AgentCreationError::MissingApiKey(env_name)) =
            GeminiAdapter.create_agent(&provider, &base_model(), &named_options(""))
        else {
            panic!("expected MissingApiKey");
        };

        assert_eq!(env_name, "GEMINI_API_KEY");
    }

    #[test]
    fn gemini_adapter_classifies_rate_limit_retry_delay() {
        let err = GeminiAdapter.classify_error(
            429,
            &serde_json::json!({
                "error": {
                    "details": [
                        { "retryDelay": "1.5s" }
                    ]
                }
            }),
        );

        match err {
            ProviderError::RateLimit {
                retry_after_ms: Some(ms),
            } => assert_eq!(ms, 1_500),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn gemini_adapter_classifies_context_overflow() {
        let err = GeminiAdapter.classify_error(
            400,
            &serde_json::json!({
                "error": {
                    "message": "Request exceeds the maximum token limit."
                }
            }),
        );
        assert!(matches!(err, ProviderError::ContextOverflow));
    }

    #[test]
    fn gemini_adapter_classifies_auth_failures() {
        assert!(matches!(
            GeminiAdapter.classify_error(401, &Value::Null),
            ProviderError::AuthFailure
        ));
        assert!(matches!(
            GeminiAdapter.classify_error(403, &Value::Null),
            ProviderError::AuthFailure
        ));
    }

    #[test]
    fn gemini_adapter_classifies_not_found() {
        assert!(matches!(
            GeminiAdapter.classify_error(404, &Value::Null),
            ProviderError::ModelNotFound
        ));
    }

    #[test]
    fn gemini_adapter_classifies_server_errors() {
        assert!(matches!(
            GeminiAdapter.classify_error(503, &Value::Null),
            ProviderError::ServerError(503)
        ));
    }
}
