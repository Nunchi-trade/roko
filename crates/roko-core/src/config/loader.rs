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
//! ## Hierarchical `ROKO__SECTION__FIELD` overrides
//!
//! In addition to the named variables above, hierarchical overrides using
//! `ROKO__SECTION__FIELD` syntax are supported. The prefix `ROKO__` is stripped,
//! and `__` separators are converted to `.` in the config path. The value is then
//! applied to the serialized TOML representation via structured serde roundtrip.
//!
//! Examples:
//! - `ROKO__AGENT__DEFAULT_MODEL=gpt-4` -> `agent.default_model = "gpt-4"`
//! - `ROKO__CONDUCTOR__MAX_AGENTS=16` -> `conductor.max_agents = 16`
//! - `ROKO__GATES__SKIP_TESTS=true` -> `gates.skip_tests = true`

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
    /// Apply hierarchical `ROKO__SECTION__FIELD` env overrides.
    pub apply_hierarchical_env: bool,
    /// Apply strict safety validation (reject `dangerously_skip_permissions`).
    pub strict_validation: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            merge_global: true,
            apply_env_overrides: true,
            apply_hierarchical_env: true,
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
            apply_hierarchical_env: false,
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

/// Load config with full provenance tracking (for CLI `load_resolved_config` compatibility).
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
    if opts.apply_hierarchical_env {
        apply_hierarchical_env_overrides(&mut migrated);
    }
    migrated.interpolate_env_vars();
    migrated.resolve_file_secrets();

    // Post-merge provider reference validation (same as load_from_resolved_path).
    if !migrated.models.is_empty() {
        validate_provider_references(&migrated, &path)?;
    }

    let diagnostics = collect_diagnostics(&migrated);

    let mut provenance = match &path {
        Some(p) => vec![ConfigProvenance::file(p.clone(), "roko.toml")],
        None => vec![ConfigProvenance::default(
            "roko.toml",
            "missing file; using built-in defaults",
        )],
    };

    // Record which hierarchical env overrides were applied.
    if opts.apply_hierarchical_env {
        let env_paths = collect_hierarchical_env_paths();
        for path_key in &env_paths {
            provenance.push(ConfigProvenance::env(
                path_key.clone(),
                format!(
                    "ROKO__{} env override",
                    path_key.to_ascii_uppercase().replace('.', "__")
                ),
            ));
        }
    }

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
    if opts.apply_hierarchical_env {
        apply_hierarchical_env_overrides(&mut config);
    }
    config.interpolate_env_vars();
    config.resolve_file_secrets();

    // Post-merge provider reference validation.
    // When strict_validation is enabled in config, dangling model->provider
    // references become hard errors instead of warnings.
    if !config.models.is_empty() {
        validate_provider_references(&config, path)?;
    }

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

/// Validate that all model profiles reference providers that exist in the
/// merged config. In strict mode, missing references become hard errors.
/// In lenient mode (default), they are logged as warnings only — the
/// `collect_diagnostics()` call handles the lenient warning path.
fn validate_provider_references(
    config: &RokoConfig,
    path: &Option<PathBuf>,
) -> Result<(), LoadConfigError> {
    if !config.validation.strict_validation {
        return Ok(());
    }

    for (model_key, model_profile) in &config.models {
        if !config.providers.contains_key(&model_profile.provider) {
            let msg = format!(
                "model '{}' references provider '{}' which does not exist in the merged config. \
                 Check roko.toml [models.{}] and ensure [providers.{}] is defined.",
                model_key, model_profile.provider, model_key, model_profile.provider
            );
            return Err(LoadConfigError::ProviderReference {
                path: path.clone().unwrap_or_default(),
                model_key: model_key.clone(),
                provider_key: model_profile.provider.clone(),
                message: msg,
            });
        }
    }
    Ok(())
}

// ─── Hierarchical env override support ───────────────────────────────────

/// Convert a `ROKO__SECTION__FIELD` env var key to a dotted config path.
///
/// Strips the `ROKO__` prefix, lowercases, and replaces `__` with `.`.
/// Returns `None` if the key does not start with `ROKO__` or is empty after
/// stripping the prefix.
fn hierarchical_env_to_path(key: &str) -> Option<String> {
    let suffix = key.strip_prefix("ROKO__")?;
    if suffix.is_empty() {
        return None;
    }
    Some(suffix.to_ascii_lowercase().replace("__", "."))
}

/// Collect all `ROKO__*` env vars that represent hierarchical config paths.
///
/// Returns the list of dotted config paths that were found in the environment.
fn collect_hierarchical_env_paths() -> Vec<String> {
    collect_hierarchical_env_paths_from(std::env::vars())
}

/// Collect hierarchical env paths from an iterator (testable).
fn collect_hierarchical_env_paths_from<I>(vars: I) -> Vec<String>
where
    I: IntoIterator<Item = (String, String)>,
{
    vars.into_iter()
        .filter_map(|(key, _)| hierarchical_env_to_path(&key))
        .collect()
}

/// Apply hierarchical `ROKO__SECTION__FIELD` env overrides to a config.
///
/// This works by serializing the config to TOML, applying the overrides to the
/// TOML value tree, then deserializing back. This approach uses serde's
/// structured handling rather than ad-hoc string edits.
fn apply_hierarchical_env_overrides(config: &mut RokoConfig) {
    apply_hierarchical_env_overrides_from(config, std::env::vars());
}

/// Apply hierarchical env overrides from a given set of vars (testable).
pub(crate) fn apply_hierarchical_env_overrides_from<I>(config: &mut RokoConfig, vars: I)
where
    I: IntoIterator<Item = (String, String)>,
{
    // Collect all ROKO__* vars and convert to dotted paths.
    let overrides: Vec<(String, String)> = vars
        .into_iter()
        .filter_map(|(key, value)| hierarchical_env_to_path(&key).map(|path| (path, value)))
        .collect();

    if overrides.is_empty() {
        return;
    }

    // Serialize current config to a TOML value tree.
    let mut toml_value = match toml::Value::try_from(config.clone()) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize config for hierarchical env overrides");
            return;
        }
    };

    // Apply each override to the TOML tree.
    for (path, value) in &overrides {
        set_toml_value_at_path(&mut toml_value, path, value);
    }

    // Deserialize back into RokoConfig.
    match toml_value.try_into::<RokoConfig>() {
        Ok(updated) => *config = updated,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "failed to deserialize config after hierarchical env overrides; \
                 overrides will be partially applied"
            );
        }
    }
}

/// Set a value in a TOML value tree at a dotted path.
///
/// Creates intermediate tables as needed. The value is parsed as a TOML
/// literal (bool, integer, float) or stored as a string.
fn set_toml_value_at_path(root: &mut toml::Value, path: &str, raw_value: &str) {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        return;
    }

    // Navigate to the parent table, creating intermediate tables if needed.
    let mut current = root;
    for segment in &segments[..segments.len() - 1] {
        let table = match current.as_table_mut() {
            Some(t) => t,
            None => return, // Path doesn't resolve to a table; skip this override.
        };
        current = table
            .entry(*segment)
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    }

    // Set the leaf value.
    let leaf_key = segments[segments.len() - 1];
    if let Some(table) = current.as_table_mut() {
        let parsed_value = parse_env_value_to_toml(raw_value);
        table.insert(leaf_key.to_string(), parsed_value);
    }
}

/// Parse a raw env var value into an appropriate TOML value type.
///
/// Attempts to parse as: bool (word-form only), integer, float; falls back to string.
/// Note: "0" and "1" are treated as integers, NOT booleans, to avoid breaking
/// numeric config fields. Only explicit words like "true"/"false"/"yes"/"no"
/// are treated as booleans.
fn parse_env_value_to_toml(raw: &str) -> toml::Value {
    // Try bool (word-form only; "0"/"1" are integers).
    match raw.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => return toml::Value::Boolean(true),
        "false" | "no" | "off" => return toml::Value::Boolean(false),
        _ => {}
    }

    // Try integer.
    if let Ok(n) = raw.parse::<i64>() {
        return toml::Value::Integer(n);
    }

    // Try float (only if it contains a dot to avoid int->float coercion).
    if raw.contains('.') {
        if let Ok(f) = raw.parse::<f64>() {
            return toml::Value::Float(f);
        }
    }

    // Fall back to string.
    toml::Value::String(raw.to_string())
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
#[allow(unsafe_code)]
mod tests {
    use super::*;

    #[test]
    fn load_without_merge_returns_default_when_no_config() {
        let dir = tempfile::tempdir().unwrap();
        let opts = LoadOptions {
            merge_global: false,
            apply_env_overrides: false,
            apply_hierarchical_env: false,
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
            apply_hierarchical_env: false,
            strict_validation: false,
        };
        let config = load_config_with_options(dir.path(), &opts).unwrap();
        // Without global merge, only project-level providers exist.
        // (No assertion on specific providers since global config varies per machine.)
        assert_eq!(config.config_version, 2);
    }

    #[test]
    fn hierarchical_env_to_path_parses_correctly() {
        assert_eq!(
            super::hierarchical_env_to_path("ROKO__AGENT__DEFAULT_MODEL"),
            Some("agent.default_model".to_string())
        );
        assert_eq!(
            super::hierarchical_env_to_path("ROKO__CONDUCTOR__MAX_AGENTS"),
            Some("conductor.max_agents".to_string())
        );
        assert_eq!(super::hierarchical_env_to_path("ROKO__"), None);
        assert_eq!(super::hierarchical_env_to_path("OTHER_VAR"), None);
        assert_eq!(super::hierarchical_env_to_path("ROKO_MODEL"), None);
    }

    #[test]
    fn parse_env_value_to_toml_types() {
        use super::parse_env_value_to_toml;
        assert_eq!(parse_env_value_to_toml("true"), toml::Value::Boolean(true));
        assert_eq!(
            parse_env_value_to_toml("false"),
            toml::Value::Boolean(false)
        );
        assert_eq!(parse_env_value_to_toml("yes"), toml::Value::Boolean(true));
        assert_eq!(parse_env_value_to_toml("no"), toml::Value::Boolean(false));
        // "0" and "1" are integers, not booleans (avoids breaking numeric fields).
        assert_eq!(parse_env_value_to_toml("0"), toml::Value::Integer(0));
        assert_eq!(parse_env_value_to_toml("1"), toml::Value::Integer(1));
        assert_eq!(parse_env_value_to_toml("42"), toml::Value::Integer(42));
        assert_eq!(parse_env_value_to_toml("3.14"), toml::Value::Float(3.14));
        assert_eq!(
            parse_env_value_to_toml("hello"),
            toml::Value::String("hello".to_string())
        );
    }

    #[test]
    fn hierarchical_env_overrides_apply_to_config() {
        let mut config = RokoConfig::default();
        let vars = vec![
            (
                "ROKO__AGENT__DEFAULT_MODEL".to_string(),
                "test-model".to_string(),
            ),
            ("ROKO__CONDUCTOR__MAX_AGENTS".to_string(), "16".to_string()),
            ("ROKO__GATES__SKIP_TESTS".to_string(), "true".to_string()),
        ];

        super::apply_hierarchical_env_overrides_from(&mut config, vars);

        assert_eq!(config.agent.default_model, "test-model");
        assert_eq!(config.conductor.max_agents, 16);
        assert!(config.gates.skip_tests);
    }

    #[test]
    fn hierarchical_env_and_named_env_precedence() {
        // Hierarchical overrides run after named overrides in the loader,
        // so ROKO__AGENT__DEFAULT_MODEL should win over ROKO_MODEL when
        // both are applied. This test exercises the internal function only.
        let mut config = RokoConfig::default();
        config.agent.default_model = "from-named-env".to_string();

        let vars = vec![(
            "ROKO__AGENT__DEFAULT_MODEL".to_string(),
            "from-hierarchical".to_string(),
        )];

        super::apply_hierarchical_env_overrides_from(&mut config, vars);
        assert_eq!(config.agent.default_model, "from-hierarchical");
    }

    #[test]
    fn validated_loader_records_hierarchical_env_provenance() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("roko.toml"), "config_version = 2\n").unwrap();

        // Set a hierarchical env var for this test.
        // Note: this test is isolated so env var pollution is acceptable.
        // SAFETY: test is single-threaded; no other thread reads this env var.
        unsafe { std::env::set_var("ROKO__AGENT__DEFAULT_MODEL", "env-test-model") };
        let opts = LoadOptions {
            merge_global: false,
            apply_env_overrides: false,
            apply_hierarchical_env: true,
            strict_validation: false,
        };
        let validated = load_config_validated_with_options(dir.path(), &opts).unwrap();
        // SAFETY: test is single-threaded; no other thread reads this env var.
        unsafe { std::env::remove_var("ROKO__AGENT__DEFAULT_MODEL") };

        assert_eq!(validated.config().agent.default_model, "env-test-model");
        // Should have provenance entry for the env override.
        let has_env_provenance = validated
            .provenance()
            .iter()
            .any(|p| p.key == "agent.default_model");
        assert!(has_env_provenance, "expected env provenance entry");
    }

    #[test]
    fn strict_validation_rejects_dangling_provider_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let toml_text = r#"
config_version = 2

[validation]
strict_validation = true

[providers.anthropic]
kind = "anthropic_api"
api_key_env = "ANTHROPIC_API_KEY"

[models.fast]
provider = "nonexistent_provider"
slug = "claude-sonnet-4-20250514"
"#;
        std::fs::write(dir.path().join("roko.toml"), toml_text).expect("write roko.toml");

        let result = load_config_with_options(
            dir.path(),
            &LoadOptions {
                merge_global: false,
                apply_env_overrides: false,
                apply_hierarchical_env: false,
                strict_validation: false,
            },
        );
        assert!(
            result.is_err(),
            "strict mode should reject dangling provider ref"
        );
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("nonexistent_provider"),
            "error should mention the missing provider: {err_msg}"
        );
        assert!(
            err_msg.contains("fast"),
            "error should mention the model key: {err_msg}"
        );
    }

    #[test]
    fn lenient_validation_allows_dangling_provider_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let toml_text = r#"
config_version = 2

[providers.anthropic]
kind = "anthropic_api"
api_key_env = "ANTHROPIC_API_KEY"

[models.fast]
provider = "nonexistent_provider"
slug = "claude-sonnet-4-20250514"
"#;
        std::fs::write(dir.path().join("roko.toml"), toml_text).expect("write roko.toml");

        let result = load_config_with_options(
            dir.path(),
            &LoadOptions {
                merge_global: false,
                apply_env_overrides: false,
                apply_hierarchical_env: false,
                strict_validation: false,
            },
        );
        assert!(
            result.is_ok(),
            "lenient mode should allow dangling provider ref: {:?}",
            result.err()
        );
    }

    #[test]
    fn empty_models_skips_provider_validation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let toml_text = r#"
config_version = 2

[validation]
strict_validation = true
"#;
        std::fs::write(dir.path().join("roko.toml"), toml_text).expect("write roko.toml");

        let result = load_config_with_options(
            dir.path(),
            &LoadOptions {
                merge_global: false,
                apply_env_overrides: false,
                apply_hierarchical_env: false,
                strict_validation: false,
            },
        );
        assert!(
            result.is_ok(),
            "empty models should not trigger validation: {:?}",
            result.err()
        );
    }
}
