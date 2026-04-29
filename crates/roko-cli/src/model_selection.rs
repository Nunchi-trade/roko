use std::collections::HashMap;
use std::fmt;

use roko_core::agent::{ProviderKind, resolve_model};
use roko_core::config::schema::{ProviderConfig, RokoConfig};
use roko_learn::cascade_router::CascadeRouter;
use thiserror::Error;

use crate::config_helpers::find_role_override;

/// Provenance for the selected model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    /// Explicit `--model` CLI override.
    CliOverride,
    /// Model hint from the task definition.
    TaskModel,
    /// Model override from the role configuration.
    RoleConfig,
    /// Model selected by the cascade router.
    CascadeRouter,
    /// Workspace/project default model.
    ProjectDefault,
    /// Built-in fallback model.
    BuiltInDefault,
}

impl SelectionSource {
    /// Stable human-readable label used in errors and reasons.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::CliOverride => "cli override",
            Self::TaskModel => "task model",
            Self::RoleConfig => "role config",
            Self::CascadeRouter => "cascade router",
            Self::ProjectDefault => "project default",
            Self::BuiltInDefault => "built-in default",
        }
    }
}

impl fmt::Display for SelectionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Fully resolved model/provider selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveModelSelection {
    /// The originally requested model string, if one was selected.
    pub requested_model: Option<String>,
    /// The final model key used for resolution.
    pub effective_model_key: String,
    /// Provider registry key.
    pub provider_key: String,
    /// Provider family label.
    pub provider_kind: String,
    /// Concrete backend slug sent to the provider.
    pub backend_slug: String,
    /// Which precedence step produced this selection.
    pub source: SelectionSource,
    /// Human-readable explanation of why this selection won.
    pub reason: String,
}

/// Errors returned by [`resolve_effective_model`].
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum Error {
    /// The caller provided an empty model string for a required input.
    #[error("{source} received an empty model value")]
    EmptyModel { source: SelectionSource },
    /// The selected model points at a provider key that is not configured.
    #[error("{source} selected model '{model}', but provider '{provider_key}' is not configured")]
    MissingProvider {
        /// Which precedence step selected the model.
        source: SelectionSource,
        /// Model that won precedence.
        model: String,
        /// Provider key referenced by the selected model.
        provider_key: String,
    },
    /// The selected model could not be backed by any configured provider.
    #[error("{source} selected unknown model '{model}', and no configured provider matches kind '{provider_kind}'")]
    UnknownModel {
        /// Which precedence step selected the model.
        source: SelectionSource,
        /// Model that won precedence.
        model: String,
        /// Provider kind inferred from the selected model.
        provider_kind: String,
    },
}

#[derive(Debug, Clone)]
struct ModelCandidate {
    source: SelectionSource,
    model: String,
}

/// Resolve the effective model/provider pair using the shared precedence chain.
pub fn resolve_effective_model(
    cli_model: Option<String>,
    task_hint: Option<String>,
    role: Option<String>,
    cascade_router: Option<&CascadeRouter>,
    config: &RokoConfig,
) -> Result<EffectiveModelSelection, Error> {
    let candidate = select_candidate(cli_model, task_hint, role, cascade_router, config)?;
    let source = candidate.source;
    let requested_model = candidate.model;
    let resolved = resolve_model(config, &requested_model);
    let providers = config.effective_providers();
    let (provider_key, provider) = select_provider(source, &requested_model, &resolved, &providers)?;

    let effective_model_key = resolved.model_key;
    let backend_slug = resolved.slug;
    let provider_kind = provider.kind.label().to_string();
    let reason = build_reason(
        source,
        &requested_model,
        &effective_model_key,
        &provider_key,
        &provider_kind,
        &backend_slug,
    );

    Ok(EffectiveModelSelection {
        requested_model: Some(requested_model),
        effective_model_key,
        provider_key,
        provider_kind,
        backend_slug,
        source,
        reason,
    })
}

fn select_candidate(
    cli_model: Option<String>,
    task_hint: Option<String>,
    role: Option<String>,
    cascade_router: Option<&CascadeRouter>,
    config: &RokoConfig,
) -> Result<ModelCandidate, Error> {
    if let Some(model) = required_model(cli_model, SelectionSource::CliOverride)? {
        return Ok(ModelCandidate {
            source: SelectionSource::CliOverride,
            model,
        });
    }

    if let Some(model) = required_model(task_hint, SelectionSource::TaskModel)? {
        return Ok(ModelCandidate {
            source: SelectionSource::TaskModel,
            model,
        });
    }

    if let Some(role_label) = normalized_label(role) {
        if let Some(override_cfg) = find_role_override(config, &role_label) {
            if let Some(model) = override_cfg
                .model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_owned)
            {
                return Ok(ModelCandidate {
                    source: SelectionSource::RoleConfig,
                    model,
                });
            }
        }
    }

    if let Some(router) = cascade_router {
        // This selector has no richer feature context, so we ask the cascade
        // for its deterministic raw-context choice.
        let model = router.select(Vec::new()).model.slug;
        let model = model.trim();
        if model.is_empty() {
            return Err(Error::EmptyModel {
                source: SelectionSource::CascadeRouter,
            });
        }
        return Ok(ModelCandidate {
            source: SelectionSource::CascadeRouter,
            model: model.to_string(),
        });
    }

    let default_model = config.agent.default_model.trim();
    if !default_model.is_empty() {
        return Ok(ModelCandidate {
            source: SelectionSource::ProjectDefault,
            model: default_model.to_string(),
        });
    }

    Ok(ModelCandidate {
        source: SelectionSource::BuiltInDefault,
        model: builtin_default_model(),
    })
}

fn required_model(
    input: Option<String>,
    source: SelectionSource,
) -> Result<Option<String>, Error> {
    match input {
        Some(model) => {
            let model = model.trim();
            if model.is_empty() {
                Err(Error::EmptyModel { source })
            } else {
                Ok(Some(model.to_string()))
            }
        }
        None => Ok(None),
    }
}

fn normalized_label(input: Option<String>) -> Option<String> {
    input
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
}

fn builtin_default_model() -> String {
    RokoConfig::default().agent.default_model
}

fn select_provider<'a>(
    source: SelectionSource,
    model: &str,
    resolved: &roko_core::agent::ResolvedModel,
    providers: &'a HashMap<String, ProviderConfig>,
) -> Result<(String, &'a ProviderConfig), Error> {
    if let Some(profile) = resolved.profile.as_ref() {
        let provider_key = profile.provider.trim();
        if provider_key.is_empty() {
            return Err(Error::MissingProvider {
                source,
                model: model.to_string(),
                provider_key: profile.provider.clone(),
            });
        }

        let provider = providers.get(provider_key).ok_or_else(|| Error::MissingProvider {
            source,
            model: model.to_string(),
            provider_key: provider_key.to_string(),
        })?;

        return Ok((provider_key.to_string(), provider));
    }

    let Some((provider_key, provider)) = provider_for_kind(providers, resolved.provider_kind)
    else {
        return Err(Error::UnknownModel {
            source,
            model: model.to_string(),
            provider_kind: resolved.provider_kind.label().to_string(),
        });
    };

    Ok((provider_key, provider))
}

fn provider_for_kind<'a>(
    providers: &'a HashMap<String, ProviderConfig>,
    kind: ProviderKind,
) -> Option<(String, &'a ProviderConfig)> {
    let exact_key = kind.label();
    if let Some(provider) = providers.get(exact_key) {
        if provider.kind == kind {
            return Some((exact_key.to_string(), provider));
        }
    }

    let mut matches = providers
        .iter()
        .filter_map(|(key, provider)| (provider.kind == kind).then_some((key.as_str(), provider)))
        .collect::<Vec<_>>();
    matches.sort_unstable_by(|a, b| a.0.cmp(b.0));
    matches.first().map(|&(key, provider)| (key.to_string(), provider))
}

fn build_reason(
    source: SelectionSource,
    requested_model: &str,
    effective_model_key: &str,
    provider_key: &str,
    provider_kind: &str,
    backend_slug: &str,
) -> String {
    format!(
        "{source} selected `{requested_model}` as `{effective_model_key}` -> `{backend_slug}` via provider `{provider_key}` ({provider_kind})"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use roko_core::config::schema::{ModelProfile, RoleOverride, RokoConfig};
    use roko_learn::cascade_router::CascadeRouter;

    fn role_model(model: &str) -> RoleOverride {
        RoleOverride {
            model: Some(model.to_string()),
            ..Default::default()
        }
    }

    fn explicit_profile(provider: &str, slug: &str) -> ModelProfile {
        ModelProfile {
            provider: provider.to_string(),
            slug: slug.to_string(),
            ..Default::default()
        }
    }

    fn cascade_router(model: &str) -> CascadeRouter {
        CascadeRouter::new(vec![model.to_string()])
    }

    #[test]
    fn cli_override_wins_over_everything() {
        let mut config = RokoConfig::default();
        config.agent.default_model = "claude-opus-4-6".to_string();
        config.agent.roles.insert("implementer".to_string(), role_model("claude-haiku-4-5"));
        let router = cascade_router("claude-sonnet-4-6");

        let selection = resolve_effective_model(
            Some("claude-haiku-4-5".to_string()),
            Some("claude-sonnet-4-6".to_string()),
            Some("implementer".to_string()),
            Some(&router),
            &config,
        )
        .expect("selection");

        assert_eq!(selection.source, SelectionSource::CliOverride);
        assert_eq!(selection.requested_model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(selection.effective_model_key, "claude-haiku-4-5");
        assert_eq!(selection.backend_slug, "claude-haiku-4-5");
        assert_eq!(selection.provider_key, "claude_cli");
        assert_eq!(selection.provider_kind, "claude_cli");
        assert!(selection.reason.contains("cli override"));
    }

    #[test]
    fn task_hint_wins_when_no_cli_override() {
        let mut config = RokoConfig::default();
        config.agent.roles.insert("implementer".to_string(), role_model("claude-opus-4-6"));
        let router = cascade_router("claude-sonnet-4-6");

        let selection = resolve_effective_model(
            None,
            Some("claude-haiku-4-5".to_string()),
            Some("implementer".to_string()),
            Some(&router),
            &config,
        )
        .expect("selection");

        assert_eq!(selection.source, SelectionSource::TaskModel);
        assert_eq!(selection.requested_model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(selection.effective_model_key, "claude-haiku-4-5");
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("task model"));
    }

    #[test]
    fn role_default_used_as_fallback() {
        let mut config = RokoConfig::default();
        config.agent.roles.insert("architect".to_string(), role_model("claude-opus-4-6"));

        let selection = resolve_effective_model(
            None,
            None,
            Some("architect".to_string()),
            None,
            &config,
        )
        .expect("selection");

        assert_eq!(selection.source, SelectionSource::RoleConfig);
        assert_eq!(selection.requested_model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(selection.effective_model_key, "claude-opus-4-6");
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("role config"));
    }

    #[test]
    fn cascade_router_is_consulted_when_no_explicit_selection_exists() {
        let config = RokoConfig::default();
        let router = cascade_router("claude-haiku-4-5");

        let selection = resolve_effective_model(None, None, None, Some(&router), &config)
            .expect("selection");

        assert_eq!(selection.source, SelectionSource::CascadeRouter);
        assert_eq!(selection.requested_model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(selection.effective_model_key, "claude-haiku-4-5");
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("cascade router"));
    }

    #[test]
    fn config_default_is_used_when_cascade_is_absent() {
        let mut config = RokoConfig::default();
        config.agent.default_model = "claude-opus-4-6".to_string();

        let selection = resolve_effective_model(None, None, None, None, &config).expect("selection");

        assert_eq!(selection.source, SelectionSource::ProjectDefault);
        assert_eq!(selection.requested_model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(selection.effective_model_key, "claude-opus-4-6");
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("project default"));
    }

    #[test]
    fn builtin_fallback_is_used_when_config_has_no_default() {
        let mut config = RokoConfig::default();
        config.agent.default_model.clear();
        let builtin_default = RokoConfig::default().agent.default_model;

        let selection = resolve_effective_model(None, None, None, None, &config).expect("selection");

        assert_eq!(selection.source, SelectionSource::BuiltInDefault);
        assert_eq!(selection.requested_model.as_deref(), Some(builtin_default.as_str()));
        assert_eq!(selection.effective_model_key, builtin_default);
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("built-in default"));
    }

    #[test]
    fn cli_override_with_unavailable_provider_returns_error() {
        let mut config = RokoConfig::default();
        config
            .models
            .insert("custom".to_string(), explicit_profile("openai", "gpt-4o"));

        let err = resolve_effective_model(Some("custom".to_string()), None, None, None, &config)
            .expect_err("selection should fail");

        assert!(err.to_string().contains("provider 'openai'"));
    }

    #[test]
    fn unknown_model_slug_returns_error() {
        let config = RokoConfig::default();

        let err = resolve_effective_model(
            Some("definitely-not-a-model".to_string()),
            None,
            None,
            None,
            &config,
        )
        .expect_err("selection should fail");

        assert!(err.to_string().contains("no configured provider matches kind"));
    }
}
