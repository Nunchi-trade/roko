//! Unified config loader for all Roko binaries (CLI, serve, ACP, agent-server).
//!
//! Before this module, 12+ separate `load_roko_config` functions existed across
//! the codebase, each with different behavior around global config merging,
//! `ROKO_CONFIG` env var, env overrides, and validation. This module provides
//! a **single entry point** that all callsites should use.
//!
//! # Precedence (highest wins)
//!
//! 1. Named env var overrides (see list below)
//! 2. `ROKO_CONFIG` env var -> load that file instead of ancestor walk
//! 3. Project `roko.toml` (found via ancestor walk from workdir)
//! 4. Global `~/.roko/config.toml` (providers/models/agent defaults merged)
//! 5. Built-in defaults ([`RokoConfig::default()`])
//!
//! # Supported environment variable overrides
//!
//! | Variable | Config field |
//! |---|---|
//! | `ROKO_MODEL` | `agent.default_model` |
//! | `ROKO_BACKEND` | `agent.default_backend` |
//! | `ROKO_EFFORT` | `agent.default_effort` |
//! | `ROKO_CONTEXT_LIMIT_K` | `agent.context_limit_k` |
//! | `ROKO_MAX_AGENTS` | `conductor.max_agents` |
//! | `ROKO_BUDGET_USD` | `budget.max_plan_usd` |
//! | `ROKO_PARALLEL` | `conductor.parallel_enabled` |
//! | `ROKO_EXPRESS` | `conductor.express_mode` |
//! | `ROKO_SKIP_TESTS` | `gates.skip_tests` |
//! | `ROKO_CLIPPY` | `gates.clippy_enabled` |
//! | `ROKO_PROVIDER` | synthesized model profile provider |
//! | `ROKO_MODEL_SLUG` | synthesized model profile slug |
//!
//! **Note**: Hierarchical `ROKO__SECTION__FIELD` overrides are not currently
//! implemented. Only the named variables listed above are supported.

use std::path::{Path, PathBuf};

use super::LoadConfigError;
use super::provenance::{ConfigDiagnostic, ConfigProvenance, ValidatedConfig};
use super::schema::RokoConfig;

// ─── Load options ───────────────────────────────────────────────────────

/// Controls how the unified loader behaves.
#[derive(Clone, Debug)]
pub struct LoadOptions {
    /// Merge providers/models from `~/.roko/config.toml`.
    pub merge_global: bool,
    /// Apply named env var overrides (ROKO_MODEL, ROKO_BACKEND, etc.).
    pub apply_env_overrides: bool,
    /// Apply strict safety validation (reject `dangerously_skip_permissions`).
    pub strict_validation: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            merge_global: true,
            apply_env_overrides: true,
            strict_validation: false,
        }
    }
}

impl LoadOptions {
    /// Options for ACP / Zed integration: lenient, with global merge.
    ///
    /// Currently identical to `Default`, but kept as a named constructor so
    /// ACP-specific divergences (e.g. workspace-scoped overrides) can be
    /// added without touching every callsite.
    #[must_use]
    pub fn acp() -> Self {
        Self::default()
    }

    /// Options for strict / inherited config loading.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            merge_global: false,
            apply_env_overrides: false,
            strict_validation: true,
        }
    }
}

// ─── Public API ─────────────────────────────────────────────────────────

/// Load config with all defaults: global merge + env overrides, no strict validation.
///
/// This is the function that all `load_roko_config()` callsites should migrate to.
pub fn load_config_unified(workdir: &Path) -> Result<RokoConfig, LoadConfigError> {
    load_config_with_options(workdir, &LoadOptions::default())
}

/// Load config with custom options.
pub fn load_config_with_options(
    workdir: &Path,
    opts: &LoadOptions,
) -> Result<RokoConfig, LoadConfigError> {
    let path = find_config_path(workdir);
    load_from_resolved_path(&path, opts)
}

/// Load config from one explicit file path.
///
/// This bypasses `ROKO_CONFIG` and ancestor discovery but still applies the
/// processing requested by [`LoadOptions`]: optional global merge, `ROKO__*`
/// env overrides, interpolation, file secrets, and strict validation.
pub fn load_config_file(path: &Path, opts: &LoadOptions) -> Result<RokoConfig, LoadConfigError> {
    load_from_resolved_path(&Some(path.to_path_buf()), opts)
}

/// Load config with full provenance tracking (for CLI `load_layered` compatibility).
///
/// Returns a [`ValidatedConfig`] with diagnostics and provenance info.
pub fn load_config_validated(workdir: &Path) -> Result<ValidatedConfig, LoadConfigError> {
    load_config_validated_with_options(workdir, &LoadOptions::default())
}

/// Load config with provenance tracking and custom options.
///
/// Returns a [`ValidatedConfig`] where `raw` is the parsed-only config
/// (before env overrides and secret interpolation) and `migrated` is the
/// fully resolved config.
pub fn load_config_validated_with_options(
    workdir: &Path,
    opts: &LoadOptions,
) -> Result<ValidatedConfig, LoadConfigError> {
    let path = find_config_path(workdir);

    // Parse + validate (no env overrides or secret resolution yet).
    let raw = parse_from_resolved_path(&path, opts)?;

    // Apply runtime mutations (global merge, env overrides, secrets).
    let mut migrated = raw.clone();
    if opts.merge_global {
        merge_global_into(&mut migrated);
    }
    if opts.apply_env_overrides {
        migrated.apply_process_env();
    }
    migrated.interpolate_env_vars();
    migrated.resolve_file_secrets();

    let diagnostics = collect_diagnostics(&migrated);

    let provenance = match &path {
        Some(p) => vec![ConfigProvenance::file(p.clone(), "roko.toml")],
        None => vec![ConfigProvenance::default(
            "roko.toml",
            "missing file; using built-in defaults",
        )],
    };

    Ok(ValidatedConfig {
        raw,
        migrated,
        diagnostics,
        provenance,
    })
}

/// Parse config from an already-resolved path (read + validate + parse only).
///
/// Does NOT apply global merge, env overrides, or secret interpolation.
/// Use this when you need the raw parsed config before mutations.
fn parse_from_resolved_path(
    path: &Option<PathBuf>,
    opts: &LoadOptions,
) -> Result<RokoConfig, LoadConfigError> {
    // 1. Read the raw text once (returns default if no file).
    let raw_text = match path {
        Some(p) => Some(
            std::fs::read_to_string(p).map_err(|source| LoadConfigError::Read {
                path: p.clone(),
                source,
            })?,
        ),
        None => None,
    };

    // 2. Optionally apply strict validation on the raw text.
    if opts.strict_validation {
        if let (Some(p), Some(text)) = (path, &raw_text) {
            let strict_source = super::validation::StrictConfigSource::shared(Some(p.clone()));
            super::validation::validate_strict_config_toml(text, &strict_source).map_err(
                |source| LoadConfigError::Validation {
                    path: p.clone(),
                    source,
                },
            )?;
        }
    }

    // 3. Parse (or use defaults if no file).
    match (&path, raw_text) {
        (Some(p), Some(text)) => toml::from_str(&text).map_err(|source| LoadConfigError::Parse {
            path: p.clone(),
            source,
        }),
        _ => Ok(RokoConfig::default()),
    }
}

/// Internal: load config from an already-resolved path with full processing.
///
/// All public functions resolve the path once via [`find_config_path`] then
/// delegate here, avoiding double discovery and double file reads.
fn load_from_resolved_path(
    path: &Option<PathBuf>,
    opts: &LoadOptions,
) -> Result<RokoConfig, LoadConfigError> {
    let mut config = parse_from_resolved_path(path, opts)?;

    // Apply runtime mutations.
    if opts.merge_global {
        merge_global_into(&mut config);
    }
    if opts.apply_env_overrides {
        config.apply_process_env();
    }
    config.interpolate_env_vars();
    config.resolve_file_secrets();

    // Emit diagnostics as warnings so callers don't need to opt into
    // load_config_validated() to see slug duplicates and orphaned models.
    for diag in collect_diagnostics(&config) {
        if diag.key.starts_with('_') {
            // Skip the env-override meta-note; it's noise on the hot path.
            continue;
        }
        tracing::warn!(
            config_key = %diag.key,
            "config warning: {}",
            diag.message
        );
    }

    Ok(config)
}

/// Collect semantic diagnostics from a fully-loaded config.
///
/// Checks: outdated config version, orphaned model→provider references,
/// duplicate model slugs.
fn collect_diagnostics(config: &RokoConfig) -> Vec<ConfigDiagnostic> {
    let mut diagnostics = Vec::new();

    if config.config_version < super::schema::CURRENT_CONFIG_VERSION {
        diagnostics.push(ConfigDiagnostic {
            key: "config_version".to_string(),
            message: format!(
                "config_version={} is older than current {}; consider running a migration",
                config.config_version,
                super::schema::CURRENT_CONFIG_VERSION,
            ),
        });
    }

    // Validate models reference existing providers.
    for (key, profile) in &config.models {
        if !config.providers.contains_key(&profile.provider) {
            diagnostics.push(ConfigDiagnostic {
                key: format!("models.{key}.provider"),
                message: format!(
                    "model '{}' references provider '{}' which is not configured",
                    key, profile.provider
                ),
            });
        }

        if let Some(max_output) = profile.max_output
            && max_output < 1_000
        {
            diagnostics.push(ConfigDiagnostic {
                key: format!("models.{key}.max_output"),
                message: format!(
                    "model '{}' sets max_output={} which is unusually low for IDE usage",
                    key, max_output
                ),
            });
        }
    }

    // Check for duplicate slugs.
    let mut slug_to_keys: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for (key, profile) in &config.models {
        slug_to_keys
            .entry(profile.slug.as_str())
            .or_default()
            .push(key.as_str());
    }
    for (slug, keys) in &slug_to_keys {
        if keys.len() > 1 {
            diagnostics.push(ConfigDiagnostic {
                key: format!("models.*.slug={slug}"),
                message: format!(
                    "duplicate model slug '{}' defined by keys: {}",
                    slug,
                    keys.join(", ")
                ),
            });
        }
    }

    // If env overrides were applied, add a note so users understand that
    // some diagnostics may refer to env-injected values.
    let env_vars_present = std::env::var("ROKO_MODEL").is_ok()
        || std::env::var("ROKO_BACKEND").is_ok()
        || std::env::var("ROKO_PROVIDER").is_ok()
        || std::env::var("ROKO_CONFIG").is_ok();

    if env_vars_present && !diagnostics.is_empty() {
        diagnostics.push(ConfigDiagnostic {
            key: "_env_override_note".to_string(),
            message: "one or more ROKO_* env vars are set; some diagnostics above \
                      may reflect env-injected values rather than roko.toml contents"
                .to_string(),
        });
    }

    diagnostics
}

/// Serialize the effective (fully-resolved) config as TOML.
///
/// Useful for workspace creation (write resolved config, not blind copy)
/// and debugging (`roko config show --effective`).
pub fn serialize_effective(config: &RokoConfig) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(config)
}

// ─── Path discovery ─────────────────────────────────────────────────────

/// Find the config file to load. Checks, in order:
///
/// 1. `ROKO_CONFIG` env var (explicit path override)
/// 2. Ancestor walk from `workdir` (find nearest `roko.toml`)
///
/// Returns `None` if no config file is found (defaults will be used).
fn find_config_path(workdir: &Path) -> Option<PathBuf> {
    // 1. ROKO_CONFIG env var takes precedence.
    if let Ok(env_path) = std::env::var("ROKO_CONFIG") {
        let p = PathBuf::from(&env_path);
        if p.is_file() {
            return Some(p);
        }
        tracing::warn!(
            path = %env_path,
            "ROKO_CONFIG env var set but file not found; falling back to discovery"
        );
    }

    // 2. Ancestor walk from workdir (also checks workdir itself).
    discover_project_config(workdir)
}

/// Walk up from `start` looking for `roko.toml`. Returns the first hit.
#[must_use]
pub fn discover_project_config(start: &Path) -> Option<PathBuf> {
    let mut cur = start
        .canonicalize()
        .ok()
        .unwrap_or_else(|| start.to_path_buf());
    loop {
        let candidate = cur.join("roko.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Canonical global config path: `~/.roko/config.toml`, with legacy
/// `$XDG_CONFIG_HOME/roko/config.toml` fallback.
#[must_use]
pub fn global_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let canonical = PathBuf::from(&home).join(".roko").join("config.toml");

    if canonical.exists() {
        return canonical;
    }

    // Legacy: $XDG_CONFIG_HOME/roko/config.toml or ~/.config/roko/config.toml
    let legacy = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            PathBuf::from(xdg).join("roko").join("config.toml")
        } else {
            PathBuf::from(&home)
                .join(".config")
                .join("roko")
                .join("config.toml")
        }
    } else {
        PathBuf::from(&home)
            .join(".config")
            .join("roko")
            .join("config.toml")
    };

    if legacy.exists() {
        return legacy;
    }

    // Neither exists — return canonical for new installs.
    canonical
}

// ─── Internal helpers ───────────────────────────────────────────────────

/// Merge providers, models, and agent defaults from the global config.
///
/// Project entries take precedence: global entries are only inserted if the
/// key doesn't already exist in the project config.
pub fn merge_global_into(config: &mut RokoConfig) {
    let global_path = global_config_path();
    if !global_path.exists() {
        return;
    }

    let text = match std::fs::read_to_string(&global_path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                path = %global_path.display(),
                error = %e,
                "failed to read global config"
            );
            return;
        }
    };

    let global = match RokoConfig::from_toml(&text) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!(
                path = %global_path.display(),
                error = %e,
                "failed to parse global config"
            );
            return;
        }
    };

    // -- Providers and models (always merge, project wins) --
    for (name, provider) in global.providers {
        config.providers.entry(name).or_insert(provider);
    }
    for (name, model) in global.models {
        config.models.entry(name).or_insert(model);
    }

    // -- Agent defaults (fill gaps only) --
    if config.agent.default_model.is_empty() && !global.agent.default_model.is_empty() {
        tracing::debug!(model = %global.agent.default_model, "merged global agent.default_model");
        config.agent.default_model = global.agent.default_model;
    }
    if config.agent.default_backend.is_empty() && !global.agent.default_backend.is_empty() {
        tracing::debug!(backend = %global.agent.default_backend, "merged global agent.default_backend");
        config.agent.default_backend = global.agent.default_backend;
    }
    if config.agent.default_effort.is_empty() && !global.agent.default_effort.is_empty() {
        tracing::debug!(effort = %global.agent.default_effort, "merged global agent.default_effort");
        config.agent.default_effort = global.agent.default_effort.clone();
    }

    // -- Budget defaults (fill when project uses default values) --
    // BudgetConfig::default().max_plan_usd = 25.0
    let default_max_plan_usd: f32 = 25.0;
    if (config.budget.max_plan_usd - default_max_plan_usd).abs() < f32::EPSILON
        && (global.budget.max_plan_usd - default_max_plan_usd).abs() > f32::EPSILON
    {
        tracing::debug!(
            max_plan_usd = global.budget.max_plan_usd,
            "merged global budget.max_plan_usd"
        );
        config.budget.max_plan_usd = global.budget.max_plan_usd;
    }

    // -- Conductor defaults (fill when project uses defaults) --
    // ConductorConfig default max_agents = 8
    let default_max_agents: usize = 8;
    if config.conductor.max_agents == default_max_agents
        && global.conductor.max_agents != default_max_agents
    {
        tracing::debug!(
            max_agents = global.conductor.max_agents,
            "merged global conductor.max_agents"
        );
        config.conductor.max_agents = global.conductor.max_agents;
    }

    // Post-merge: validate model->provider references now that both layers are present.
    for (model_key, profile) in &config.models {
        if !config.providers.contains_key(&profile.provider) {
            tracing::warn!(
                model = %model_key,
                provider = %profile.provider,
                "model references provider '{}' which is missing after global+project merge",
                profile.provider,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_without_merge_returns_default_when_no_config() {
        let dir = tempfile::tempdir().unwrap();
        let opts = LoadOptions {
            merge_global: false,
            apply_env_overrides: false,
            strict_validation: false,
        };
        let config = load_config_with_options(dir.path(), &opts).unwrap();
        assert_eq!(config, RokoConfig::default());
    }

    #[test]
    fn load_unified_reads_roko_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("roko.toml"),
            r#"
config_version = 2
schema_version = 2

[providers.test-prov]
kind = "openai_compat"
base_url = "https://example.com/v1"
"#,
        )
        .unwrap();

        let config = load_config_unified(dir.path()).unwrap();
        assert!(config.providers.contains_key("test-prov"));
    }

    #[test]
    fn load_validated_detects_orphaned_models() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("roko.toml"),
            r#"
config_version = 2
schema_version = 2

[models.orphan]
provider = "nonexistent"
slug = "orphan-v1"
context_window = 4096
"#,
        )
        .unwrap();

        let validated = load_config_validated(dir.path()).unwrap();
        let has_orphan_warning = validated
            .diagnostics()
            .iter()
            .any(|d| d.key.contains("orphan") && d.message.contains("nonexistent"));
        assert!(has_orphan_warning, "should warn about orphaned model");
    }

    #[test]
    fn load_validated_detects_duplicate_slugs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("roko.toml"),
            r#"
config_version = 2
schema_version = 2

[providers.prov]
kind = "openai_compat"
base_url = "https://example.com/v1"

[models.model-a]
provider = "prov"
slug = "same-slug"
context_window = 4096

[models.model-b]
provider = "prov"
slug = "same-slug"
context_window = 4096
"#,
        )
        .unwrap();

        let validated = load_config_validated(dir.path()).unwrap();
        let has_dup_warning = validated
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("duplicate model slug"));
        assert!(has_dup_warning, "should warn about duplicate slug");
    }

    #[test]
    fn serialize_effective_roundtrips() {
        let config = RokoConfig::default();
        let toml_str = serialize_effective(&config).unwrap();
        let reparsed: RokoConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(config, reparsed);
    }

    #[test]
    fn discover_project_config_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.path().join("roko.toml"), "config_version = 2\n").unwrap();

        let found = discover_project_config(&nested);
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("roko.toml"));
    }

    #[test]
    fn load_strict_rejects_dangerous_permissions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("roko.toml"),
            "[runner]\ndangerously_skip_permissions = true\n",
        )
        .unwrap();

        let opts = LoadOptions::strict();
        let result = load_config_with_options(dir.path(), &opts);
        assert!(result.is_err());
    }

    #[test]
    fn load_without_global_merge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("roko.toml"), "config_version = 2\n").unwrap();

        let opts = LoadOptions {
            merge_global: false,
            apply_env_overrides: false,
            strict_validation: false,
        };
        let config = load_config_with_options(dir.path(), &opts).unwrap();
        // Without global merge, only project-level providers exist.
        // (No assertion on specific providers since global config varies per machine.)
        assert_eq!(config.config_version, 2);
    }
}
