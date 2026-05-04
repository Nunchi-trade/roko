//! Perplexity Sonar provider adapter.
//!
//! Routes model creation to the appropriate agent type based on model capabilities:
//! - Embedding models → [`PerplexityEmbedAgent`]
//! - Async/deep-research models → [`PerplexityDeepResearchAgent`]
//! - Standard chat models → [`PerplexityChatAgent`]

use crate::agent::{Agent, AgentResult, derived_output};
use crate::dispatcher::HandlerResolver;
use crate::perplexity::PerplexityDeepResearchAgent;
use crate::perplexity::chat::PerplexityChatAgent;
use crate::perplexity::embed;
use crate::perplexity::tool_loop::{PerplexityToolLoopAgent, PerplexityToolLoopBackend};
use crate::perplexity::types::SearchOptions;
use crate::provider::openai_compat::tool_registry_for_options;
use crate::provider::{
    AgentCreationError, AgentOptions, PERPLEXITY_SEARCH_OPTIONS_ARG_PREFIX, ProviderAdapter,
    ProviderError, build_tool_dispatcher, tool_loop_max_iterations,
};
use crate::tool_loop::ToolLoop;
use crate::translate::{OpenAiTranslator, Translator};
use async_trait::async_trait;
use roko_core::agent::ProviderKind;
#[cfg(test)]
use roko_core::config::DEFAULT_TTFT_TIMEOUT_MS;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;
use roko_core::{Body, Context, Engram, Kind, Provenance};
use serde_json::Value;
use std::sync::Arc;

/// Default Perplexity API base URL.
const DEFAULT_BASE_URL: &str = "https://api.perplexity.ai";

// ─── PerplexityEmbedAgentAdapter ─────────────────────────────────────────────

/// `Agent` wrapper around the real Perplexity `/v1/embeddings` client.
///
/// Extracts text from the input `Engram`, calls `embed::PerplexityEmbedAgent`
/// for real embeddings, and returns the float vectors as a JSON body.
pub struct PerplexityEmbedAgentAdapter {
    inner: embed::PerplexityEmbedAgent,
    name: String,
}

impl PerplexityEmbedAgentAdapter {
    #[must_use]
    pub fn new(api_key: String, base_url: String, model_slug: String) -> Self {
        let name = format!("perplexity-embed:{model_slug}");
        // The real embed client expects the base_url to include /v1 for the
        // embeddings path construction (`{base_url}/embeddings`).
        let embed_base_url = if base_url.ends_with("/v1") || base_url.ends_with("/v1/") {
            base_url
        } else {
            format!("{}/v1", base_url.trim_end_matches('/'))
        };
        let inner = embed::PerplexityEmbedAgent::new(api_key, embed_base_url, model_slug);
        Self { inner, name }
    }
}

#[async_trait]
impl Agent for PerplexityEmbedAgentAdapter {
    async fn run(&self, input: &Engram, _ctx: &Context) -> AgentResult {
        let text = input.body.as_text().unwrap_or_default();
        match self.inner.embed(&[text]).await {
            Ok(vectors) => {
                let body_json = serde_json::to_string(&vectors).unwrap_or_default();
                let output = derived_output(input, Kind::AgentOutput, Body::text(&body_json))
                    .provenance(Provenance::agent(&self.name))
                    .tag("agent", &self.name)
                    .tag(
                        "embedding_dims",
                        vectors.first().map_or(0, |v| v.len()).to_string(),
                    )
                    .build();
                AgentResult::ok(output)
            }
            Err(err) => {
                let output = derived_output(
                    input,
                    Kind::AgentOutput,
                    Body::text(format!("embedding error: {err}")),
                )
                .provenance(Provenance::agent(&self.name))
                .tag("agent", &self.name)
                .tag("error", err.to_string())
                .build();
                AgentResult::fail(output)
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_id(&self) -> &'static str {
        "perplexity"
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

fn agent_name(options: &AgentOptions, default: &str) -> String {
    if options.name.is_empty() {
        default.to_string()
    } else {
        options.name.clone()
    }
}

fn merge_search_options(target: &mut SearchOptions, source: &SearchOptions) {
    if source.search_domain_filter.is_some() {
        target.search_domain_filter = source.search_domain_filter.clone();
    }
    if source.search_recency_filter.is_some() {
        target.search_recency_filter = source.search_recency_filter.clone();
    }
    if source.search_after_date_filter.is_some() {
        target.search_after_date_filter = source.search_after_date_filter.clone();
    }
    if source.search_before_date_filter.is_some() {
        target.search_before_date_filter = source.search_before_date_filter.clone();
    }
    if source.last_updated_after_filter.is_some() {
        target.last_updated_after_filter = source.last_updated_after_filter.clone();
    }
    if source.last_updated_before_filter.is_some() {
        target.last_updated_before_filter = source.last_updated_before_filter.clone();
    }
    if source.search_context_size.is_some() {
        target.search_context_size = source.search_context_size.clone();
    }
    if source.search_mode.is_some() {
        target.search_mode = source.search_mode.clone();
    }
    if source.return_images.is_some() {
        target.return_images = source.return_images;
    }
    if source.return_related_questions.is_some() {
        target.return_related_questions = source.return_related_questions;
    }
    if source.user_location.is_some() {
        target.user_location = source.user_location.clone();
    }
}

fn perplexity_search_options(model: &ModelProfile, options: &AgentOptions) -> SearchOptions {
    let mut search_options = SearchOptions {
        search_context_size: model.search_context_size.clone(),
        ..Default::default()
    };
    for extra_arg in &options.extra_args {
        if let Some(payload) = extra_arg.strip_prefix(PERPLEXITY_SEARCH_OPTIONS_ARG_PREFIX) {
            if let Ok(extra) = serde_json::from_str::<SearchOptions>(payload) {
                merge_search_options(&mut search_options, &extra);
            }
        }
    }
    search_options
}

fn perplexity_tool_loop_agent(
    api_key: String,
    base_url: String,
    model: &ModelProfile,
    options: &AgentOptions,
) -> Result<Box<dyn Agent>, AgentCreationError> {
    let (registry, tools) = tool_registry_for_options(model, options)?;
    let resolver: Arc<dyn HandlerResolver> =
        Arc::new(|name: &str| roko_std::tool::handlers::handler_for(name));
    let dispatcher = build_tool_dispatcher(registry, resolver);
    let translator: Arc<dyn Translator> = Arc::new(OpenAiTranslator);
    let timeout_ms = options.timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS);
    let backend = Arc::new(PerplexityToolLoopBackend::new(
        api_key,
        base_url,
        model.slug.clone(),
        perplexity_search_options(model, options),
        timeout_ms,
    ));

    let tool_loop = ToolLoop::new(translator, dispatcher, backend.clone())
        .with_max_iterations(tool_loop_max_iterations())
        .with_context_token_limit(usize::try_from(model.context_window).unwrap_or(usize::MAX))
        .with_model_profile(model.clone());

    let name = agent_name(options, &format!("perplexity-tool-loop:{}", model.slug));
    let mut agent = PerplexityToolLoopAgent::new(tool_loop, backend, model.slug.clone())
        .with_tools(tools)
        .with_name(name);
    if let Some(prompt) = &options.system_prompt {
        agent = agent.with_system_prompt(prompt.clone());
    }

    Ok(Box::new(agent))
}

// ─── PerplexityAdapter ───────────────────────────────────────────────────────

/// Provider adapter for the Perplexity Sonar API.
pub struct PerplexityAdapter;

impl ProviderAdapter for PerplexityAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PerplexityApi
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
                    .unwrap_or_else(|| "PERPLEXITY_API_KEY".into()),
            )
        })?;

        let base_url = provider
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        if model.is_embedding_model {
            return Ok(Box::new(PerplexityEmbedAgentAdapter::new(
                api_key,
                base_url,
                model.slug.clone(),
            )));
        }

        // Deep-research / async models follow a dedicated provider flow and do not
        // participate in the shared tool-loop dispatcher.
        if model.supports_async {
            let name = agent_name(options, &format!("perplexity-deep:{}", model.slug));
            let timeout_ms = options.timeout_ms.unwrap_or(30_000);
            return Ok(Box::new(
                PerplexityDeepResearchAgent::new(api_key, base_url, model.slug.clone(), name)
                    .with_request_timeout_ms(timeout_ms),
            ));
        }

        if model.supports_tools {
            return perplexity_tool_loop_agent(api_key, base_url, model, options);
        }

        let name = agent_name(options, &format!("perplexity:{}", model.slug));
        let timeout = options.timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS);

        Ok(Box::new(
            PerplexityChatAgent::new(api_key, base_url, model.slug.clone(), name, timeout)
                .with_search_options(perplexity_search_options(model, options)),
        ))
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        match status {
            429 => ProviderError::RateLimit {
                retry_after_ms: body
                    .pointer("/retry_after")
                    .and_then(|v| v.as_u64())
                    .map(|s| s * 1000),
            },
            401 | 403 => ProviderError::AuthFailure,
            404 => ProviderError::ModelNotFound,
            408 | 504 => ProviderError::Timeout,
            500..=599 => ProviderError::ServerError(status),
            _ => ProviderError::Other(format!("HTTP {status}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::config::schema::ProviderConfig;

    fn base_model() -> ModelProfile {
        ModelProfile {
            provider: "perplexity".to_string(),
            slug: "sonar".to_string(),
            context_window: 127_072,
            max_output: Some(8_192),
            supports_tools: false,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: true,
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
            supports_search: true,
            supports_citations: true,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: Some("medium".to_string()),
            cost_per_request: None,
            use_max_completion_tokens: false,
            tier: None,
        }
    }

    fn perplexity_provider() -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::PerplexityApi,
            base_url: None,
            api_key_env: Some("PATH".to_string()), // PATH is always set
            command: None,
            args: None,
            timeout_ms: None,
            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
            connect_timeout_ms: Some(5_000),
            extra_headers: None,
            max_concurrent: None,
        }
    }

    fn named_options(name: &str) -> AgentOptions {
        AgentOptions {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn perplexity_adapter_kind() {
        assert_eq!(PerplexityAdapter.kind(), ProviderKind::PerplexityApi);
    }

    #[test]
    fn perplexity_adapter_creates_chat_agent_with_options_name() {
        let model = base_model();
        let agent = PerplexityAdapter
            .create_agent(&perplexity_provider(), &model, &named_options("my-agent"))
            .expect("create chat agent");
        assert_eq!(agent.name(), "my-agent");
    }

    #[test]
    fn perplexity_adapter_creates_chat_agent_default_name() {
        let model = base_model();
        let agent = PerplexityAdapter
            .create_agent(&perplexity_provider(), &model, &named_options(""))
            .expect("create chat agent");
        assert_eq!(agent.name(), "perplexity:sonar");
    }

    #[test]
    fn perplexity_adapter_routes_embedding_model_to_embed_agent() {
        let model = ModelProfile {
            is_embedding_model: true,
            slug: "sonar-embeddings".to_string(),
            ..base_model()
        };
        let agent = PerplexityAdapter
            .create_agent(&perplexity_provider(), &model, &named_options(""))
            .expect("create embed agent");
        assert_eq!(agent.name(), "perplexity-embed:sonar-embeddings");
    }

    #[test]
    fn perplexity_adapter_routes_async_model_to_deep_research_agent() {
        let model = ModelProfile {
            supports_async: true,
            slug: "sonar-deep-research".to_string(),
            ..base_model()
        };
        let agent = PerplexityAdapter
            .create_agent(&perplexity_provider(), &model, &named_options(""))
            .expect("create deep research agent");
        assert_eq!(agent.name(), "perplexity-deep:sonar-deep-research");
    }

    #[test]
    fn perplexity_adapter_routes_tool_capable_model_to_tool_loop_agent() {
        let model = ModelProfile {
            supports_tools: true,
            slug: "sonar-tools".to_string(),
            ..base_model()
        };
        let agent = PerplexityAdapter
            .create_agent(&perplexity_provider(), &model, &named_options(""))
            .expect("create tool-loop agent");
        assert_eq!(agent.name(), "perplexity-tool-loop:sonar-tools");
    }

    #[test]
    fn perplexity_search_options_merge_model_defaults_and_agent_overrides() {
        let model = base_model();
        let options = AgentOptions::default().with_perplexity_search_options(SearchOptions {
            search_domain_filter: Some(vec!["arxiv.org".to_string()]),
            search_recency_filter: Some("week".to_string()),
            search_context_size: Some("high".to_string()),
            search_mode: Some("academic".to_string()),
            return_images: Some(true),
            ..Default::default()
        });

        let merged = perplexity_search_options(&model, &options);
        assert_eq!(merged.search_context_size, Some("high".to_string()));
        assert_eq!(
            merged.search_domain_filter,
            Some(vec!["arxiv.org".to_string()])
        );
        assert_eq!(merged.search_recency_filter, Some("week".to_string()));
        assert_eq!(merged.search_mode, Some("academic".to_string()));
        assert_eq!(merged.return_images, Some(true));
    }

    #[test]
    fn perplexity_search_options_use_model_default_when_not_overridden() {
        let model = base_model();
        let options = AgentOptions::default().with_perplexity_search_options(SearchOptions {
            search_domain_filter: Some(vec!["arxiv.org".to_string()]),
            ..Default::default()
        });

        let merged = perplexity_search_options(&model, &options);
        assert_eq!(merged.search_context_size, Some("medium".to_string()));
        assert_eq!(
            merged.search_domain_filter,
            Some(vec!["arxiv.org".to_string()])
        );
        assert_eq!(merged.search_recency_filter, None);
        assert_eq!(merged.search_mode, None);
    }

    #[test]
    fn perplexity_adapter_embedding_takes_priority_over_async() {
        // A model that is both embedding AND async → embed agent wins
        let model = ModelProfile {
            is_embedding_model: true,
            supports_async: true,
            slug: "sonar-embed-async".to_string(),
            ..base_model()
        };
        let agent = PerplexityAdapter
            .create_agent(&perplexity_provider(), &model, &named_options(""))
            .expect("create agent");
        assert_eq!(agent.name(), "perplexity-embed:sonar-embed-async");
    }

    #[test]
    fn perplexity_adapter_missing_api_key_returns_error() {
        let model = base_model();
        let provider = ProviderConfig {
            api_key_env: Some("PERPLEXITY_TEST_KEY_NONEXISTENT_ZZZZ".to_string()),
            ..perplexity_provider()
        };
        let result = PerplexityAdapter.create_agent(&provider, &model, &named_options(""));
        assert!(matches!(result, Err(AgentCreationError::MissingApiKey(_))));
    }

    #[test]
    fn perplexity_adapter_missing_api_key_env_returns_error() {
        let model = base_model();
        let provider = ProviderConfig {
            api_key_env: None,
            ..perplexity_provider()
        };
        let result = PerplexityAdapter.create_agent(&provider, &model, &named_options(""));
        let Err(AgentCreationError::MissingApiKey(env_name)) = result else {
            panic!("expected MissingApiKey error");
        };
        assert_eq!(env_name, "PERPLEXITY_API_KEY");
    }

    #[test]
    fn perplexity_adapter_classify_rate_limit_with_retry_after() {
        let err = PerplexityAdapter.classify_error(429, &serde_json::json!({ "retry_after": 10 }));
        match err {
            ProviderError::RateLimit {
                retry_after_ms: Some(ms),
            } => assert_eq!(ms, 10_000),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn perplexity_adapter_classify_rate_limit_no_retry_after() {
        let err = PerplexityAdapter.classify_error(429, &serde_json::Value::Null);
        assert!(matches!(
            err,
            ProviderError::RateLimit {
                retry_after_ms: None
            }
        ));
    }

    #[test]
    fn perplexity_adapter_classify_auth() {
        assert!(matches!(
            PerplexityAdapter.classify_error(401, &serde_json::Value::Null),
            ProviderError::AuthFailure
        ));
        assert!(matches!(
            PerplexityAdapter.classify_error(403, &serde_json::Value::Null),
            ProviderError::AuthFailure
        ));
    }

    #[test]
    fn perplexity_adapter_classify_not_found() {
        assert!(matches!(
            PerplexityAdapter.classify_error(404, &serde_json::Value::Null),
            ProviderError::ModelNotFound
        ));
    }

    #[test]
    fn perplexity_adapter_classify_timeout() {
        assert!(matches!(
            PerplexityAdapter.classify_error(408, &serde_json::Value::Null),
            ProviderError::Timeout
        ));
        assert!(matches!(
            PerplexityAdapter.classify_error(504, &serde_json::Value::Null),
            ProviderError::Timeout
        ));
    }

    #[test]
    fn perplexity_adapter_classify_server_error() {
        assert!(matches!(
            PerplexityAdapter.classify_error(500, &serde_json::Value::Null),
            ProviderError::ServerError(500)
        ));
        assert!(matches!(
            PerplexityAdapter.classify_error(503, &serde_json::Value::Null),
            ProviderError::ServerError(503)
        ));
    }

    #[test]
    fn perplexity_adapter_classify_other() {
        let err = PerplexityAdapter.classify_error(422, &serde_json::Value::Null);
        assert!(matches!(err, ProviderError::Other(msg) if msg == "HTTP 422"));
    }
}
