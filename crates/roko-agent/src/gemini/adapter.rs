//! Gemini provider adapter.
//!
//! This task wires Gemini into the provider abstraction and selects the
//! appropriate concrete agent shape based on model capabilities.

use super::compat::GeminiCompatAgent;
use super::embed::GeminiEmbedAgent;
use super::native::GeminiNativeAgent;
use crate::agent::Agent;
use crate::dispatcher::HandlerResolver;
use crate::http::ReqwestPoster;
use crate::provider::openai_compat::{max_tokens_for_model, tool_registry_for_options};
use crate::provider::{
    AgentCreationError, AgentOptions, ProviderAdapter, ProviderError, build_tool_dispatcher,
};
use crate::tool_loop::backends::GeminiNativeBackend;
use crate::tool_loop::{OpenAiCompatBackend, ToolLoop, ToolLoopAgent};
use crate::translate::{GeminiTranslator, OpenAiTranslator, Translator};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::Value;
use std::sync::Arc;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";

fn gemini_tool_loop_base_url(base_url: &str) -> String {
    format!("{}/v1beta/openai/v1", base_url.trim_end_matches('/'))
}

fn gemini_tool_loop_agent(
    api_key: String,
    provider: &ProviderConfig,
    model: &ModelProfile,
    options: &AgentOptions,
) -> Result<Box<dyn Agent>, AgentCreationError> {
    let (registry, tools) = tool_registry_for_options(options)?;
    let resolver: Arc<dyn HandlerResolver> =
        Arc::new(|name: &str| roko_std::tool::handlers::handler_for(name));
    let dispatcher = build_tool_dispatcher(registry, resolver);
    let translator: Arc<dyn Translator> = Arc::new(OpenAiTranslator);
    let timeout_ms = options.timeout_ms.unwrap_or(120_000);
    let mut extra_body_params = serde_json::Map::new();
    if let Some(cached_content) = options.cached_content.as_deref() {
        extra_body_params.insert(
            "cached_content".to_string(),
            Value::String(cached_content.to_string()),
        );
    }

    let backend = OpenAiCompatBackend::new(api_key, model.slug.clone())
        .with_provider_id(model.provider.clone())
        .with_base_url(gemini_tool_loop_base_url(
            provider.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL),
        ))
        .with_timeout_ms(timeout_ms)
        .with_max_tokens(max_tokens_for_model(model))
        .with_extra_headers(provider.extra_headers.clone().unwrap_or_default())
        .with_extra_body_params(extra_body_params)
        .with_poster(Box::new(ReqwestPoster::new()));

    let tool_loop = ToolLoop::new(translator, dispatcher, Arc::new(backend))
        .with_max_iterations(50)
        .with_context_token_limit(usize::try_from(model.context_window).unwrap_or(usize::MAX))
        .with_model_profile(model.clone());

    let name = if options.name.is_empty() {
        format!("gemini-compat:{}", model.slug)
    } else {
        options.name.clone()
    };

    let mut agent = ToolLoopAgent::new(tool_loop)
        .with_tools(tools)
        .with_name(name);
    if let Some(prompt) = &options.system_prompt {
        agent = agent.with_system_prompt(prompt.clone());
    }

    Ok(Box::new(agent))
}

fn gemini_native_tool_loop_agent(
    api_key: String,
    provider: &ProviderConfig,
    model: &ModelProfile,
    options: &AgentOptions,
) -> Result<Box<dyn Agent>, AgentCreationError> {
    let (registry, tools) = tool_registry_for_options(options)?;
    let resolver: Arc<dyn HandlerResolver> =
        Arc::new(|name: &str| roko_std::tool::handlers::handler_for(name));
    let dispatcher = build_tool_dispatcher(registry, resolver);
    let translator: Arc<dyn Translator> = Arc::new(GeminiTranslator);
    let backend = GeminiNativeBackend::new(
        api_key,
        provider.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL),
        model.clone(),
        options,
    );

    let tool_loop = ToolLoop::new(translator, dispatcher, Arc::new(backend))
        .with_max_iterations(50)
        .with_context_token_limit(usize::try_from(model.context_window).unwrap_or(usize::MAX))
        .with_model_profile(model.clone());

    let name = if options.name.is_empty() {
        format!("gemini-native:{}", model.slug)
    } else {
        options.name.clone()
    };

    let mut agent = ToolLoopAgent::new(tool_loop)
        .with_tools(tools)
        .with_name(name);
    if let Some(prompt) = &options.system_prompt {
        agent = agent.with_system_prompt(prompt.clone());
    }

    Ok(Box::new(agent))
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
            let mut agent = GeminiEmbedAgent::new(api_key, base_url, model.slug.clone());
            if let Some(timeout_ms) = options.timeout_ms {
                agent = agent.with_timeout_ms(timeout_ms);
            }
            if !options.name.is_empty() {
                agent = agent.with_name(options.name.clone());
            }
            return Ok(Box::new(agent));
        }

        let needs_native = model.supports_grounding || model.supports_code_execution;

        if needs_native {
            Ok(Box::new(GeminiNativeAgent::new(
                api_key,
                base_url,
                model.clone(),
                &options,
            )))
        } else if model.supports_tools && model.tool_format == "gemini_native" {
            gemini_native_tool_loop_agent(api_key, provider, model, &options)
        } else if model.supports_tools {
            gemini_tool_loop_agent(api_key, provider, model, &options)
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
            ttft_timeout_ms: Some(15_000),
            connect_timeout_ms: Some(5_000),
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
            tool_format: "gemini_native".to_string(),
            ..base_model()
        };
        let agent = GeminiAdapter
            .create_agent(&gemini_provider(), &model, &named_options(""))
            .expect("create native tool-loop agent");
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
