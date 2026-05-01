//! Compatibility resolver that projects existing model selection into a shared dispatch plan.
//!
//! This module intentionally does not change execution. It wraps the current
//! config/model lookup behavior and records the validation gaps that still need
//! dedicated migration packets.

use std::collections::HashMap;

use roko_core::agent::{ProviderKind, ResolvedModel, resolve_model};
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::{
    DispatchAttempt, DispatchAttemptKind, DispatchAuthStatus, DispatchError, DispatchPlan,
    DispatchRequest, DispatchRequirement, FallbackPolicy, TransportAuth, TransportPlan,
};
use thiserror::Error;

/// Produces shared [`DispatchPlan`] values from existing config selection behavior.
#[derive(Debug, Clone)]
pub struct DispatchResolver {
    config: RokoConfig,
}

impl DispatchResolver {
    /// Create a resolver over a config snapshot.
    #[must_use]
    pub const fn new(config: RokoConfig) -> Self {
        Self { config }
    }

    /// Resolve a shared dispatch plan from the current selection/config path.
    ///
    /// This is a compatibility wrapper. It does not validate auth or
    /// capabilities yet; those states are surfaced explicitly as
    /// `Unvalidated` diagnostics in the returned plan.
    pub fn resolve_existing(
        &self,
        request: DispatchRequest,
    ) -> Result<DispatchPlan, DispatchResolverError> {
        let selected_model = self.select_model(&request)?;
        let resolved = resolve_model(&self.config, &selected_model);
        let providers = self.config.effective_providers();
        let (provider_id, provider_config) =
            select_provider(&selected_model, &resolved, &providers)?;
        let model_profile = resolved
            .profile
            .clone()
            .unwrap_or_else(|| inferred_model_profile(&provider_id, &resolved));
        let transport = transport_for(provider_config);
        let diagnostics =
            diagnostics_for(&request.requirements, request.provider_override.as_ref());
        let fallback_candidates = fallback_candidates(&self.config, &resolved.model_key);
        let fallback_policy =
            fallback_policy(request.fallback_policy.clone(), &fallback_candidates);
        let auth_status = DispatchAuthStatus::Unvalidated {
            reason: "auth validation is not implemented in resolve_existing".to_string(),
        };

        Ok(DispatchPlan {
            requested_model: requested_model(&request),
            requested_provider: request.provider_override.clone(),
            effective_model_key: resolved.model_key.clone(),
            model_slug: resolved.slug.clone(),
            provider_id: provider_id.clone(),
            provider_kind: provider_config.kind,
            provider_config: provider_config.clone(),
            model_profile,
            transport: transport.clone(),
            auth_status,
            requirements: request.requirements,
            fallback_policy,
            fallback_candidates,
            attempts: vec![DispatchAttempt {
                kind: DispatchAttemptKind::Primary,
                model_key: resolved.model_key,
                model_slug: resolved.slug,
                provider_id,
                provider_kind: provider_config.kind,
                transport,
            }],
            diagnostics,
        })
    }

    fn select_model(&self, request: &DispatchRequest) -> Result<String, DispatchResolverError> {
        if let Some(model) = normalized(request.model_override.as_deref()) {
            return Ok(model);
        }

        if let Some(provider) = normalized(request.provider_override.as_deref()) {
            return find_model_for_provider(&self.config, &provider).ok_or_else(|| {
                DispatchResolverError::Dispatch(DispatchError::ConfigInvalid {
                    detail: format!(
                        "provider override '{provider}' did not match any configured model"
                    ),
                })
            });
        }

        if let Some(model) = normalized(Some(request.model_call.model.as_str())) {
            return Ok(model);
        }

        if let Some(model) = normalized(Some(self.config.agent.default_model.as_str())) {
            return Ok(model);
        }

        normalized(Some(RokoConfig::default().agent.default_model.as_str())).ok_or(
            DispatchResolverError::EmptyModel {
                selection_source: "built-in default",
            },
        )
    }
}

/// Error returned while building a compatibility dispatch plan.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum DispatchResolverError {
    /// A selection source produced only whitespace.
    #[error("{selection_source} produced an empty model")]
    EmptyModel { selection_source: &'static str },
    /// Shared typed dispatch error.
    #[error("{0:?}")]
    Dispatch(DispatchError),
}

fn normalized(input: Option<&str>) -> Option<String> {
    input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn requested_model(request: &DispatchRequest) -> Option<String> {
    normalized(request.model_override.as_deref())
        .or_else(|| normalized(Some(request.model_call.model.as_str())))
}

fn find_model_for_provider(config: &RokoConfig, provider: &str) -> Option<String> {
    let mut matches = config
        .effective_models()
        .into_iter()
        .filter(|(_, profile)| profile.provider.eq_ignore_ascii_case(provider))
        .map(|(key, _)| key)
        .collect::<Vec<_>>();
    matches.sort();
    matches.into_iter().next()
}

fn select_provider<'a>(
    selected_model: &str,
    resolved: &ResolvedModel,
    providers: &'a HashMap<String, ProviderConfig>,
) -> Result<(String, &'a ProviderConfig), DispatchResolverError> {
    if let Some(profile) = resolved.profile.as_ref() {
        let provider_id = normalized(Some(profile.provider.as_str())).ok_or_else(|| {
            DispatchResolverError::Dispatch(DispatchError::ConfigInvalid {
                detail: format!("model '{selected_model}' has an empty provider id"),
            })
        })?;
        let provider = providers.get(&provider_id).ok_or_else(|| {
            DispatchResolverError::Dispatch(DispatchError::ConfigInvalid {
                detail: format!(
                    "model '{selected_model}' references missing provider '{provider_id}'"
                ),
            })
        })?;
        return Ok((provider_id, provider));
    }

    let mut matches = providers
        .iter()
        .filter(|(_, provider)| provider.kind == resolved.provider_kind)
        .collect::<Vec<_>>();
    matches.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));

    matches
        .into_iter()
        .next()
        .map(|(provider_id, provider)| (provider_id.clone(), provider))
        .ok_or_else(|| {
            DispatchResolverError::Dispatch(DispatchError::UnsupportedProvider {
                provider_id: resolved.provider_kind.label().to_string(),
                provider_kind: resolved.provider_kind,
            })
        })
}

fn inferred_model_profile(provider_id: &str, resolved: &ResolvedModel) -> ModelProfile {
    ModelProfile {
        provider: provider_id.to_string(),
        slug: resolved.slug.clone(),
        ..Default::default()
    }
}

fn transport_for(provider: &ProviderConfig) -> TransportPlan {
    match provider.kind {
        ProviderKind::ClaudeCli => TransportPlan::Cli {
            command: provider
                .command
                .clone()
                .unwrap_or_else(|| "claude".to_string()),
            args: provider.args.clone().unwrap_or_default(),
            protocol: "stream_json".to_string(),
        },
        ProviderKind::CursorAcp => provider.command.clone().map_or_else(
            || TransportPlan::Unsupported {
                reason: "cursor_acp provider has no configured command".to_string(),
            },
            |command| TransportPlan::Acp {
                command_or_endpoint: command,
                protocol: "acp_json_rpc".to_string(),
            },
        ),
        ProviderKind::AnthropicApi
        | ProviderKind::OpenAiCompat
        | ProviderKind::PerplexityApi
        | ProviderKind::GeminiApi
        | ProviderKind::CerebrasApi => provider.base_url.clone().map_or_else(
            || TransportPlan::Unsupported {
                reason: format!(
                    "{} provider has no configured base_url",
                    provider.kind.label()
                ),
            },
            |base_url| TransportPlan::Http {
                base_url,
                auth: provider.api_key_env.clone().map_or_else(
                    || TransportAuth::Unknown {
                        reason: "provider api key source is not configured".to_string(),
                    },
                    |name| TransportAuth::EnvVar { name },
                ),
                protocol: http_protocol(provider.kind).to_string(),
            },
        ),
    }
}

fn http_protocol(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::AnthropicApi => "anthropic_messages",
        ProviderKind::GeminiApi => "gemini_generate_content",
        ProviderKind::OpenAiCompat | ProviderKind::PerplexityApi | ProviderKind::CerebrasApi => {
            "chat_completions"
        }
        ProviderKind::ClaudeCli | ProviderKind::CursorAcp => "unsupported_http",
    }
}

fn diagnostics_for(
    requirements: &[DispatchRequirement],
    provider_override: Option<&String>,
) -> Vec<String> {
    let mut diagnostics = vec![
        "auth validation is unvalidated in resolve_existing".to_string(),
        "capability validation is unvalidated in resolve_existing".to_string(),
    ];

    if !requirements.is_empty() {
        diagnostics.push(format!(
            "requirements are recorded but not validated: {:?}",
            requirements
        ));
    }
    if provider_override.is_some() {
        diagnostics.push("provider override used existing model selection behavior".to_string());
    }

    diagnostics
}

fn fallback_candidates(config: &RokoConfig, primary_model_key: &str) -> Vec<String> {
    let mut fallbacks = Vec::new();
    if let Some(fallback) = normalized(config.agent.fallback_model.as_deref()) {
        push_unique_fallback(config, &mut fallbacks, &fallback);
    }

    let mut tier_models = config.agent.tier_models.values().collect::<Vec<_>>();
    tier_models.sort();
    for tier_model in tier_models {
        push_unique_fallback(config, &mut fallbacks, tier_model);
    }

    fallbacks.retain(|fallback| fallback != primary_model_key);
    fallbacks.retain(|m| config.provider_available_for_model_key(m));
    fallbacks
}

fn push_unique_fallback(config: &RokoConfig, fallbacks: &mut Vec<String>, model_key: &str) {
    let slug = resolve_model(config, model_key).slug;
    if normalized(Some(&slug)).is_some() && !fallbacks.contains(&slug) {
        fallbacks.push(slug);
    }
}

fn fallback_policy(
    requested_policy: FallbackPolicy,
    fallback_candidates: &[String],
) -> FallbackPolicy {
    match requested_policy {
        FallbackPolicy::Disabled if !fallback_candidates.is_empty() => {
            FallbackPolicy::ConfigOrdered {
                models: fallback_candidates.to_vec(),
            }
        }
        policy => policy,
    }
}

#[cfg(test)]
mod dispatch_resolver_tests {
    use super::*;
    use roko_core::config::schema::{ModelProfile, RokoConfig};
    use roko_core::{DispatchCaller, ModelCallRequest};

    fn request(model: &str) -> DispatchRequest {
        DispatchRequest {
            caller: DispatchCaller::CliOneShot,
            model_call: ModelCallRequest {
                model: model.to_string(),
                ..Default::default()
            },
            ..default_request()
        }
    }

    fn default_request() -> DispatchRequest {
        DispatchRequest {
            caller: DispatchCaller::CliOneShot,
            workdir: None,
            role: None,
            model_call: ModelCallRequest::default(),
            model_override: None,
            provider_override: None,
            requirements: Vec::new(),
            cache_policy: Default::default(),
            budget: None,
            fallback_policy: FallbackPolicy::Disabled,
        }
    }

    fn provider(kind: ProviderKind) -> ProviderConfig {
        match kind {
            ProviderKind::ClaudeCli => ProviderConfig {
                kind,
                base_url: None,
                api_key_env: None,
                command: Some("claude".to_string()),
                args: None,
                timeout_ms: Some(120_000),
                ttft_timeout_ms: Some(15_000),
                connect_timeout_ms: Some(5_000),
                extra_headers: None,
                max_concurrent: None,
            },
            _ => ProviderConfig {
                kind,
                base_url: Some("https://example.test/v1".to_string()),
                api_key_env: Some("EXAMPLE_API_KEY".to_string()),
                command: None,
                args: None,
                timeout_ms: Some(120_000),
                ttft_timeout_ms: Some(15_000),
                connect_timeout_ms: Some(5_000),
                extra_headers: None,
                max_concurrent: None,
            },
        }
    }

    fn model(provider: &str, slug: &str) -> ModelProfile {
        ModelProfile {
            provider: provider.to_string(),
            slug: slug.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn dispatch_resolver_cli_override_produces_plan() {
        let mut config = RokoConfig::default();
        config.providers.insert(
            "anthropic".to_string(),
            provider(ProviderKind::AnthropicApi),
        );
        config.models.insert(
            "sonnet".to_string(),
            model("anthropic", "claude-sonnet-4-6"),
        );

        let mut request = request("");
        request.model_override = Some("sonnet".to_string());

        let plan = DispatchResolver::new(config)
            .resolve_existing(request)
            .expect("dispatch plan");

        assert_eq!(plan.requested_model.as_deref(), Some("sonnet"));
        assert_eq!(plan.effective_model_key, "sonnet");
        assert_eq!(plan.provider_id, "anthropic");
        assert_eq!(plan.provider_kind, ProviderKind::AnthropicApi);
        assert!(matches!(
            plan.auth_status,
            DispatchAuthStatus::Unvalidated { .. }
        ));
        assert!(matches!(plan.transport, TransportPlan::Http { .. }));
    }

    #[test]
    fn dispatch_resolver_project_default_produces_plan() {
        let mut config = RokoConfig::default();
        config.agent.default_model = "fast".to_string();
        config
            .providers
            .insert("openai".to_string(), provider(ProviderKind::OpenAiCompat));
        config
            .models
            .insert("fast".to_string(), model("openai", "gpt-5-mini"));

        let plan = DispatchResolver::new(config)
            .resolve_existing(default_request())
            .expect("dispatch plan");

        assert_eq!(plan.requested_model, None);
        assert_eq!(plan.effective_model_key, "fast");
        assert_eq!(plan.model_slug, "gpt-5-mini");
        assert_eq!(plan.provider_id, "openai");
    }

    #[test]
    fn dispatch_resolver_missing_provider_returns_typed_error() {
        let mut config = RokoConfig::default();
        config
            .models
            .insert("broken".to_string(), model("missing", "missing-model"));

        let err = DispatchResolver::new(config)
            .resolve_existing(request("broken"))
            .expect_err("missing provider should fail");

        assert!(matches!(
            err,
            DispatchResolverError::Dispatch(DispatchError::ConfigInvalid { ref detail })
                if detail.contains("missing provider")
        ));
    }

    #[test]
    fn dispatch_resolver_records_unsupported_capability_placeholder() {
        let mut config = RokoConfig::default();
        config
            .providers
            .insert("local".to_string(), provider(ProviderKind::ClaudeCli));
        config
            .models
            .insert("haiku".to_string(), model("local", "claude-haiku-4-5"));

        let mut request = request("haiku");
        request.requirements = vec![
            DispatchRequirement::Streaming,
            DispatchRequirement::McpTools,
        ];

        let plan = DispatchResolver::new(config)
            .resolve_existing(request)
            .expect("dispatch plan");

        assert_eq!(
            plan.requirements,
            vec![
                DispatchRequirement::Streaming,
                DispatchRequirement::McpTools,
            ]
        );
        assert!(
            plan.diagnostics
                .iter()
                .any(|message| message.contains("capability validation is unvalidated"))
        );
    }
}
