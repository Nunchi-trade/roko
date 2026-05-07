use std::fmt;
use std::path::Path;

use indexmap::IndexMap;
use roko_core::agent::resolve_model;
use roko_core::config::schema::{ProviderConfig, RokoConfig};
use roko_learn::cascade_router::CascadeRouter;
use thiserror::Error;

use crate::config_helpers::find_role_override;

/// Provenance for the selected model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    /// Explicit `--model` CLI override.
    CliOverride,
    /// `--provider` CLI override resolved to a model.
    ProviderOverride,
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
            Self::ProviderOverride => "provider override",
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

impl EffectiveModelSelection {
    /// Return the canonical one-line rendering for stderr / user-facing logs.
    #[must_use]
    pub fn display_line(&self) -> String {
        format!(
            "model: {} via {} (source: {})",
            self.effective_model_key, self.provider_key, self.source
        )
    }

    /// Print the canonical selection line to stderr.
    pub fn print_stderr(&self) {
        eprintln!("{}", self.display_line());
    }

    /// Serialize the selection to a JSON value for embedding in log records.
    #[must_use]
    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "effective_model_key": &self.effective_model_key,
            "provider_key": &self.provider_key,
            "provider_kind": &self.provider_kind,
            "backend_slug": &self.backend_slug,
            "source": self.source.to_string(),
            "reason": &self.reason,
            "requested_model": &self.requested_model,
        })
    }
}

/// Errors returned by [`resolve_effective_model`].
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum Error {
    /// The caller provided an empty model string for a required input.
    #[error("{selection_source} received an empty model value")]
    EmptyModel { selection_source: SelectionSource },
    /// The selected model points at a provider key that is not configured.
    #[error(
        "{selection_source} selected model '{model}', but provider '{provider_key}' is not configured"
    )]
    MissingProvider {
        /// Which precedence step selected the model.
        selection_source: SelectionSource,
        /// Model that won precedence.
        model: String,
        /// Provider key referenced by the selected model.
        provider_key: String,
    },
    /// The selected model could not be backed by any configured provider.
    #[error("{}", format_unknown_model_error(selection_source, model, provider_kind, suggestions))]
    UnknownModel {
        /// Which precedence step selected the model.
        selection_source: SelectionSource,
        /// Model that won precedence.
        model: String,
        /// Provider kind inferred from the selected model.
        provider_kind: String,
        /// Suggested similar model names.
        suggestions: Vec<String>,
    },
}

fn format_unknown_model_error(
    source: &SelectionSource,
    model: &str,
    provider_kind: &str,
    suggestions: &[String],
) -> String {
    let mut msg = format!(
        "{source} selected unknown model '{model}' (inferred kind '{provider_kind}')"
    );
    if suggestions.is_empty() {
        msg.push_str("; add an explicit [models.*] entry for this model");
    } else {
        msg.push_str(". Did you mean one of: ");
        msg.push_str(&suggestions.join(", "));
        msg.push('?');
    }
    msg
}

/// Jaro-Winkler similarity between two strings (0.0 – 1.0).
fn jaro_winkler(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let a_len = a.len();
    let b_len = b.len();

    if a_len == 0 && b_len == 0 {
        return 1.0;
    }
    if a_len == 0 || b_len == 0 {
        return 0.0;
    }

    let match_distance = (a_len.max(b_len) / 2).saturating_sub(1);
    let mut a_matched = vec![false; a_len];
    let mut b_matched = vec![false; b_len];
    let mut matches = 0usize;
    let mut transpositions = 0usize;

    for i in 0..a_len {
        let start = i.saturating_sub(match_distance);
        let end = (i + match_distance + 1).min(b_len);
        for j in start..end {
            if b_matched[j] || a[i] != b[j] {
                continue;
            }
            a_matched[i] = true;
            b_matched[j] = true;
            matches += 1;
            break;
        }
    }

    if matches == 0 {
        return 0.0;
    }

    let mut k = 0usize;
    for i in 0..a_len {
        if !a_matched[i] {
            continue;
        }
        while !b_matched[k] {
            k += 1;
        }
        if a[i] != b[k] {
            transpositions += 1;
        }
        k += 1;
    }

    let m = matches as f64;
    let jaro = (m / a_len as f64 + m / b_len as f64 + (m - transpositions as f64 / 2.0) / m) / 3.0;

    // Winkler boost for common prefix (up to 4 chars).
    let prefix_len = a.iter().zip(b.iter()).take(4).take_while(|(x, y)| x == y).count();
    jaro + prefix_len as f64 * 0.1 * (1.0 - jaro)
}

/// Collect all known model names from config keys, builtin slugs, and aliases,
/// then return the top suggestions sorted by similarity to `input`.
fn suggest_models(input: &str, config: &RokoConfig) -> Vec<String> {
    use roko_core::config::model_registry::{ALIASES, BUILTIN_MODELS};
    use std::collections::BTreeSet;

    let mut candidates = BTreeSet::new();
    for key in config.models.keys() {
        candidates.insert(key.as_str());
    }
    for m in BUILTIN_MODELS {
        candidates.insert(m.slug);
    }
    for &(alias, _) in ALIASES {
        candidates.insert(alias);
    }

    let input_lower = input.to_ascii_lowercase();
    let mut scored: Vec<(&str, f64)> = candidates
        .into_iter()
        .map(|name| {
            let name_lower = name.to_ascii_lowercase();
            let jw = jaro_winkler(&input_lower, &name_lower);
            // Boost if the candidate contains the input or vice versa.
            let contains_bonus = if name_lower.contains(&input_lower)
                || input_lower.contains(&name_lower)
            {
                0.15
            } else {
                0.0
            };
            (name, (jw + contains_bonus).min(1.0))
        })
        .filter(|(_, score)| *score > 0.6)
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(3);
    scored.into_iter().map(|(name, _)| name.to_string()).collect()
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
    cli_provider: Option<String>,
) -> Result<EffectiveModelSelection, Error> {
    let candidate = select_candidate(
        cli_model,
        task_hint,
        role,
        cascade_router,
        config,
        cli_provider,
    )?;
    let source = candidate.source;
    let requested_model = candidate.model;
    let resolved = resolve_model(config, &requested_model);
    let providers = config.effective_providers();
    let (provider_key, provider) =
        select_provider(source, &requested_model, &resolved, &providers, config)?;

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

/// Convenience wrapper used by CLI command handlers.
///
/// Loads `roko.toml` from `workdir`, resolves the effective model with the
/// standard precedence chain, prints the selection to stderr, and returns the
/// `effective_model_key` string.  The `context` string is used only in the
/// error message when resolution fails.
pub fn resolve_effective_model_key(
    workdir: &Path,
    cli_model: Option<String>,
    role: Option<&str>,
    context: &str,
) -> anyhow::Result<String> {
    let config = roko_core::config::loader::load_config_unified(workdir)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let selection = resolve_effective_model(
        cli_model,
        None,
        role.map(str::to_string),
        None,
        &config,
        None,
    )
    .map_err(|err| anyhow::anyhow!("resolve model selection for {context}: {err}"))?;
    selection.print_stderr();
    Ok(selection.effective_model_key)
}

fn select_candidate(
    cli_model: Option<String>,
    task_hint: Option<String>,
    role: Option<String>,
    cascade_router: Option<&CascadeRouter>,
    config: &RokoConfig,
    cli_provider: Option<String>,
) -> Result<ModelCandidate, Error> {
    if let Some(model) = required_model(cli_model, SelectionSource::CliOverride)? {
        return Ok(ModelCandidate {
            source: SelectionSource::CliOverride,
            model,
        });
    }

    if let Some(ref provider) = cli_provider {
        if let Some(model) = find_model_for_provider(config, provider) {
            return Ok(ModelCandidate {
                source: SelectionSource::ProviderOverride,
                model,
            });
        }
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
                selection_source: SelectionSource::CascadeRouter,
            });
        }
        if config.provider_available_for_model_key(model) {
            return Ok(ModelCandidate {
                source: SelectionSource::CascadeRouter,
                model: model.to_string(),
            });
        }
        tracing::warn!(
            model,
            "cascade router selected model whose provider is unavailable; falling through"
        );
    }

    let default_model = config.agent.default_model.trim();
    if !default_model.is_empty() {
        if config.provider_available_for_model_key(default_model) {
            return Ok(ModelCandidate {
                source: SelectionSource::ProjectDefault,
                model: default_model.to_string(),
            });
        }
        tracing::warn!(
            model = default_model,
            "default_model provider is unavailable; searching for first available model"
        );
        // Find the first available model from config.models.
        if let Some(available) = config
            .models
            .keys()
            .find(|k| config.provider_available_for_model_key(k))
        {
            return Ok(ModelCandidate {
                source: SelectionSource::ProjectDefault,
                model: available.clone(),
            });
        }
    }

    Ok(ModelCandidate {
        source: SelectionSource::BuiltInDefault,
        model: builtin_default_model(),
    })
}

fn required_model(input: Option<String>, source: SelectionSource) -> Result<Option<String>, Error> {
    match input {
        Some(model) => {
            let model = model.trim();
            if model.is_empty() {
                Err(Error::EmptyModel {
                    selection_source: source,
                })
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

/// Find the first model configured for the given provider name.
///
/// Returns the alphabetically first matching model key to ensure deterministic
/// selection when multiple models share the same provider.
fn find_model_for_provider(config: &RokoConfig, provider: &str) -> Option<String> {
    let mut matches: Vec<&str> = config
        .models
        .iter()
        .filter(|(_, profile)| profile.provider.eq_ignore_ascii_case(provider))
        .map(|(key, _)| key.as_str())
        .collect();
    matches.sort_unstable();
    matches.first().map(|k| (*k).to_owned())
}

fn builtin_default_model() -> String {
    RokoConfig::default().agent.default_model
}

fn select_provider<'a>(
    source: SelectionSource,
    model: &str,
    resolved: &roko_core::agent::ResolvedModel,
    providers: &'a IndexMap<String, ProviderConfig>,
    config: &RokoConfig,
) -> Result<(String, &'a ProviderConfig), Error> {
    if let Some(profile) = resolved.profile.as_ref() {
        let provider_key = profile.provider.trim();
        if provider_key.is_empty() {
            return Err(Error::MissingProvider {
                selection_source: source,
                model: model.to_string(),
                provider_key: profile.provider.clone(),
            });
        }

        let provider = providers
            .get(provider_key)
            .ok_or_else(|| Error::MissingProvider {
                selection_source: source,
                model: model.to_string(),
                provider_key: provider_key.to_string(),
            })?;

        return Ok((provider_key.to_string(), provider));
    }

    Err(Error::UnknownModel {
        selection_source: source,
        suggestions: suggest_models(model, config),
        model: model.to_string(),
        provider_kind: resolved.provider_kind.label().to_string(),
    })
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

    use roko_core::agent::ProviderKind;
    use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig, RoleOverride};
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

    fn claude_provider() -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::ClaudeCli,
            base_url: None,
            api_key_env: None,
            command: Some("claude".to_string()),
            args: None,
            timeout_ms: None,
            ttft_timeout_ms: None,
            connect_timeout_ms: None,
            extra_headers: None,
            max_concurrent: None,
        }
    }

    fn config_with_claude_models() -> RokoConfig {
        let mut config = RokoConfig::default();
        config.providers.clear();
        config.models.clear();
        config
            .providers
            .insert("claude_cli".to_string(), claude_provider());
        for slug in ["claude-haiku-4-5", "claude-sonnet-4-6", "claude-opus-4-6"] {
            config
                .models
                .insert(slug.to_string(), explicit_profile("claude_cli", slug));
        }
        config
    }

    fn cascade_router(model: &str) -> CascadeRouter {
        CascadeRouter::new(vec![model.to_string()])
    }

    #[test]
    fn cli_override_wins_over_everything() {
        let mut config = config_with_claude_models();
        config.agent.default_model = "claude-opus-4-6".to_string();
        config
            .agent
            .roles
            .insert("implementer".to_string(), role_model("claude-haiku-4-5"));
        let router = cascade_router("claude-sonnet-4-6");

        let selection = resolve_effective_model(
            Some("claude-haiku-4-5".to_string()),
            Some("claude-sonnet-4-6".to_string()),
            Some("implementer".to_string()),
            Some(&router),
            &config,
            None,
        )
        .expect("selection");

        assert_eq!(selection.source, SelectionSource::CliOverride);
        assert_eq!(
            selection.requested_model.as_deref(),
            Some("claude-haiku-4-5")
        );
        assert_eq!(selection.effective_model_key, "claude-haiku-4-5");
        assert_eq!(selection.backend_slug, "claude-haiku-4-5");
        assert_eq!(selection.provider_key, "claude_cli");
        assert_eq!(selection.provider_kind, "claude_cli");
        assert!(selection.reason.contains("cli override"));
    }

    #[test]
    fn task_hint_wins_when_no_cli_override() {
        let mut config = config_with_claude_models();
        config
            .agent
            .roles
            .insert("implementer".to_string(), role_model("claude-opus-4-6"));
        let router = cascade_router("claude-sonnet-4-6");

        let selection = resolve_effective_model(
            None,
            Some("claude-haiku-4-5".to_string()),
            Some("implementer".to_string()),
            Some(&router),
            &config,
            None,
        )
        .expect("selection");

        assert_eq!(selection.source, SelectionSource::TaskModel);
        assert_eq!(
            selection.requested_model.as_deref(),
            Some("claude-haiku-4-5")
        );
        assert_eq!(selection.effective_model_key, "claude-haiku-4-5");
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("task model"));
    }

    #[test]
    fn role_default_used_as_fallback() {
        let mut config = config_with_claude_models();
        config
            .agent
            .roles
            .insert("architect".to_string(), role_model("claude-opus-4-6"));

        let selection = resolve_effective_model(
            None,
            None,
            Some("architect".to_string()),
            None,
            &config,
            None,
        )
        .expect("selection");

        assert_eq!(selection.source, SelectionSource::RoleConfig);
        assert_eq!(
            selection.requested_model.as_deref(),
            Some("claude-opus-4-6")
        );
        assert_eq!(selection.effective_model_key, "claude-opus-4-6");
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("role config"));
    }

    #[test]
    fn cascade_router_is_consulted_when_no_explicit_selection_exists() {
        let config = config_with_claude_models();
        // Use claude-sonnet-4-6 because cold-start static routing for the
        // Standard tier selects from ["glm-5.1", "claude-sonnet-4-6", ...] and
        // only returns a slug present in the router's model_slugs.
        let router = cascade_router("claude-sonnet-4-6");

        let selection = resolve_effective_model(None, None, None, Some(&router), &config, None)
            .expect("selection");

        assert_eq!(selection.source, SelectionSource::CascadeRouter);
        assert_eq!(
            selection.requested_model.as_deref(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(selection.effective_model_key, "claude-sonnet-4-6");
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("cascade router"));
    }

    #[test]
    fn config_default_is_used_when_cascade_is_absent() {
        let mut config = config_with_claude_models();
        config.agent.default_model = "claude-opus-4-6".to_string();

        let selection =
            resolve_effective_model(None, None, None, None, &config, None).expect("selection");

        assert_eq!(selection.source, SelectionSource::ProjectDefault);
        assert_eq!(
            selection.requested_model.as_deref(),
            Some("claude-opus-4-6")
        );
        assert_eq!(selection.effective_model_key, "claude-opus-4-6");
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("project default"));
    }

    #[test]
    fn builtin_fallback_is_used_when_config_has_no_default() {
        let mut config = config_with_claude_models();
        config.agent.default_model.clear();
        let builtin_default = RokoConfig::default().agent.default_model;

        let selection =
            resolve_effective_model(None, None, None, None, &config, None).expect("selection");

        assert_eq!(selection.source, SelectionSource::BuiltInDefault);
        assert_eq!(
            selection.requested_model.as_deref(),
            Some(builtin_default.as_str())
        );
        assert_eq!(selection.effective_model_key, builtin_default);
        assert_eq!(selection.provider_key, "claude_cli");
        assert!(selection.reason.contains("built-in default"));
    }

    #[test]
    fn display_line_and_json_are_canonical() {
        let mut config = config_with_claude_models();
        config.agent.default_model = "claude-opus-4-6".to_string();

        let selection =
            resolve_effective_model(None, None, None, None, &config, None).expect("selection");

        assert_eq!(
            selection.display_line(),
            "model: claude-opus-4-6 via claude_cli (source: project default)"
        );

        let json = selection.as_json();
        assert_eq!(json["effective_model_key"], "claude-opus-4-6");
        assert_eq!(json["provider_key"], "claude_cli");
        assert_eq!(json["source"], "project default");
        assert_eq!(json["requested_model"], "claude-opus-4-6");
    }

    #[test]
    fn cli_override_with_unavailable_provider_returns_error() {
        let mut config = RokoConfig::default();
        // Use a provider name that won't be auto-synthesized from env vars.
        config.models.insert(
            "custom".to_string(),
            explicit_profile("custom-provider", "gpt-4o"),
        );

        let err =
            resolve_effective_model(Some("custom".to_string()), None, None, None, &config, None)
                .expect_err("selection should fail");

        assert!(err.to_string().contains("provider 'custom-provider'"));
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
            None,
        )
        .expect_err("selection should fail");

        assert!(
            err.to_string().contains("add an explicit [models.*]")
                || err.to_string().contains("Did you mean")
        );
    }

    #[test]
    fn unknown_model_slug_does_not_route_by_provider_kind() {
        let mut config = RokoConfig::default();
        config.models.clear();
        config.providers.clear();
        config.providers.insert(
            "openai_compat".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: None,
                api_key_env: Some("OPENAI_API_KEY".to_string()),
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        );

        let err = resolve_effective_model(
            Some("gpt-new-unconfigured".to_string()),
            None,
            None,
            None,
            &config,
            None,
        )
        .expect_err("selection should fail");

        assert!(matches!(
            &err,
            Error::UnknownModel {
                provider_kind,
                suggestions: _,
                ..
            } if provider_kind == "openai_compat"
        ));
        assert!(
            err.to_string().contains("explicit [models.*]")
                || err.to_string().contains("Did you mean")
        );
    }

    // ── Task 064: IDE/ACP fallback scenario tests ────────────────────────

    /// Fallback scenario 1: `--provider` CLI override selects a model by provider
    /// key, bypassing task hint and role config.
    #[test]
    fn provider_override_selects_model_for_named_provider() {
        let mut config = config_with_claude_models();
        config.agent.default_model = "claude-opus-4-6".to_string();
        config
            .agent
            .roles
            .insert("implementer".to_string(), role_model("claude-opus-4-6"));

        let selection = resolve_effective_model(
            None,
            Some("claude-opus-4-6".to_string()),
            Some("implementer".to_string()),
            None,
            &config,
            Some("claude_cli".to_string()),
        )
        .expect("selection should succeed");

        assert_eq!(selection.source, SelectionSource::ProviderOverride);
        assert!(
            selection.reason.contains("provider override"),
            "reason should mention provider override"
        );
    }

    /// Fallback scenario 2: `--provider` with unknown provider name falls through
    /// to the next precedence level (task hint).
    #[test]
    fn provider_override_unknown_provider_falls_through_to_task_hint() {
        let config = config_with_claude_models();

        let selection = resolve_effective_model(
            None,
            Some("claude-sonnet-4-6".to_string()),
            None,
            None,
            &config,
            Some("nonexistent_provider".to_string()),
        )
        .expect("selection should succeed via task hint fallback");

        assert_eq!(selection.source, SelectionSource::TaskModel);
    }

    /// Fallback scenario 3: empty `--model ""` produces an EmptyModel error
    /// (the precedence chain should NOT silently fall through).
    #[test]
    fn empty_cli_model_string_is_an_error() {
        let config = config_with_claude_models();

        let err = resolve_effective_model(Some("".to_string()), None, None, None, &config, None)
            .expect_err("empty model should error");

        assert!(matches!(
            err,
            Error::EmptyModel {
                selection_source: SelectionSource::CliOverride
            }
        ));
    }

    /// Fallback scenario 4: whitespace-only `--model` is treated as empty.
    #[test]
    fn whitespace_only_cli_model_is_an_error() {
        let config = config_with_claude_models();

        let err = resolve_effective_model(Some("   ".to_string()), None, None, None, &config, None)
            .expect_err("whitespace-only model should error");

        assert!(matches!(
            err,
            Error::EmptyModel {
                selection_source: SelectionSource::CliOverride
            }
        ));
    }

    /// Fallback scenario 5: role config with empty model falls through to
    /// cascade router or project default (does not error).
    #[test]
    fn role_with_empty_model_falls_through() {
        let mut config = config_with_claude_models();
        config.agent.default_model = "claude-sonnet-4-6".to_string();
        config
            .agent
            .roles
            .insert("architect".to_string(), role_model(""));

        let selection = resolve_effective_model(
            None,
            None,
            Some("architect".to_string()),
            None,
            &config,
            None,
        )
        .expect("should fall through to project default");

        assert_eq!(selection.source, SelectionSource::ProjectDefault);
    }

    /// Fallback scenario 6: provider override with multiple matching models
    /// picks alphabetically first to ensure deterministic selection.
    #[test]
    fn provider_override_picks_alphabetically_first_model() {
        let mut config = RokoConfig::default();
        config.providers.clear();
        config.models.clear();
        config
            .providers
            .insert("shared_provider".to_string(), claude_provider());
        config.models.insert(
            "z-model".to_string(),
            explicit_profile("shared_provider", "z-slug"),
        );
        config.models.insert(
            "a-model".to_string(),
            explicit_profile("shared_provider", "a-slug"),
        );
        config.models.insert(
            "m-model".to_string(),
            explicit_profile("shared_provider", "m-slug"),
        );

        let selection = resolve_effective_model(
            None,
            None,
            None,
            None,
            &config,
            Some("shared_provider".to_string()),
        )
        .expect("should pick alphabetically first model");

        assert_eq!(selection.source, SelectionSource::ProviderOverride);
        // "a-model" is alphabetically first among the three.
        assert_eq!(
            selection.requested_model.as_deref(),
            Some("a-model"),
            "should select alphabetically first model for the provider"
        );
    }
}
