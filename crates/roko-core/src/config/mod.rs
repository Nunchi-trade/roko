//! Roko runtime configuration.
//!
//! # Modules
//!
//! - [`schema`] -- The unified `RokoConfig` type with hierarchical sections.
//! - [`compat`] -- Reader for legacy Mori `config.toml` format.
//! - [`presets`] -- Named presets (minimal / balanced / thorough).

use std::path::Path;

use thiserror::Error;

pub mod agent;
pub mod budget;
pub mod chain;
pub mod compat;
pub mod gates;
pub mod hot_reload;
pub mod learning;
pub mod loader;
pub mod presets;
pub mod project;
pub mod provenance;
pub mod provider;
pub mod routing;
pub mod schema;
pub mod serve;
pub mod subscriptions;
pub mod tools;
pub mod tui_cfg;
pub mod validation;

// Re-exports for ergonomic use.
pub use crate::temperament::Temperament;
pub use compat::from_mori_toml;
pub use presets::Preset;
pub use provenance::{
    ConfigDiagnostic, ConfigProvenance, ConfigSource, ResolvedRuntimeConfig, ValidatedConfig,
};
pub use provider::{
    BackendModelSlug, ConfigIdentityError, ModelAlias, ModelCapabilities, ModelCost,
    ModelDefinition, ModelMetadataSource, ProviderAuth, ProviderCapabilities, ProviderDefinition,
    ProviderId, ProviderTransport, DEFAULT_TTFT_TIMEOUT_MS,
};
pub use validation::{
    DangerousPermissionOverride, DangerousPermissionOverrideError, StrictConfigSource,
    StrictConfigValidationError, validate_strict_config_toml,
};
// All section structs are re-exported from schema (which re-exports from submodules).
pub use schema::{
    AgentBudget, AgentConfig, AgentDefinition, AgentMode, AgentThresholds, ApiKeyEntry,
    BudgetConfig, CURRENT_SCHEMA_VERSION, ChainConfig, CompileFailRepeatConfig, ConductorConfig,
    ContextWindowPressureConfig, CoreRunnerConfig, CostOverrunConfig, DataLlmConfig, DeployConfig,
    GatesConfig, GeminiConfig, GhostTurnConfig, GithubWebhookConfig, IterationLoopConfig,
    LearningConfig, ModelProfile, PerplexityConfig, PipelineBandConfig, PipelineConfig,
    PipelineReviewerMode, PrdConfig, ProjectConfig, ProviderConfig, ProviderRouting, RelayConfig,
    ReviewLoopConfig, RewardWeights, RokoConfig, RoleOverride, RoutingAlgorithm, RoutingConfig,
    RoutingOverrides, RoutingRewardWeightsConfig, SafetySetting, SchedulerConfig,
    SchedulerCronConfig, ServeAuthConfig, ServeConfig, ServeDeployConfig, ServeDeployWebhookConfig,
    ServerConfig, SpecDriftConfig, StuckPatternConfig, SubscriptionConfig,
    SubscriptionFilterConfig, SubscriptionTrigger, TestFailureBudgetConfig, TimeOverrunConfig,
    ToolProfileConfig, ToolsConfig, TuiConfig, WatcherConfig, WatcherPathConfig, WatcherThresholds,
    WebhooksConfig,
};

/// Error returned when loading a `roko.toml` file from disk.
#[derive(Debug, Error)]
pub enum LoadConfigError {
    /// Reading the config file failed.
    #[error("read {path}: {source}")]
    Read {
        /// Config file path.
        path: std::path::PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Parsing the config file failed.
    #[error("parse {path}: {source}")]
    Parse {
        /// Config file path.
        path: std::path::PathBuf,
        /// Underlying parse error.
        source: toml::de::Error,
    },
    /// Strict validation rejected a safety-sensitive setting.
    #[error("validate {path}: {source}")]
    Validation {
        /// Config file path.
        path: std::path::PathBuf,
        /// Underlying validation error.
        source: StrictConfigValidationError,
    },
}

/// Load the workspace configuration from `workdir/roko.toml`.
///
/// Uses local trust semantics: the project's own `roko.toml` is treated
/// as explicitly controlled by the user, so safety-sensitive settings
/// like `dangerously_skip_permissions = true` are permitted.
///
/// For validating configs that may be inherited from untrusted sources
/// (ancestor directories, team repos), use [`load_config_strict`].
///
/// Missing files fall back to [`ValidatedConfig::default`] (wrapping
/// [`RokoConfig::default`]) so callers can start in an uninitialized
/// workspace.
///
/// After parsing, two secret-resolution passes run automatically:
///   1. `${VAR}` interpolation — expands environment variable references in
///      provider config strings.
///   2. `*_file` resolution — reads secrets from file paths in `extra_headers`
///      whose keys end with `_file`.
///
/// Soft-warning checks (e.g. outdated `config_version`) are surfaced via
/// [`ValidatedConfig::diagnostics`] and do not fail the load.
pub fn load_config(workdir: &Path) -> Result<ValidatedConfig, LoadConfigError> {
    load_config_impl(workdir, ConfigTrust::Local)
}

/// Load the workspace configuration with strict safety validation.
///
/// Identical to [`load_config`] except that safety-sensitive settings
/// (e.g. `dangerously_skip_permissions = true`) are rejected. Use this
/// when loading configs from potentially untrusted sources like ancestor
/// directory inheritance or team-shared repos.
pub fn load_config_strict(workdir: &Path) -> Result<ValidatedConfig, LoadConfigError> {
    load_config_impl(workdir, ConfigTrust::Shared)
}

/// Trust level for workspace config loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigTrust {
    /// Config may be inherited — reject safety-sensitive overrides.
    Shared,
    /// Config is locally owned — permit all settings.
    Local,
}

fn load_config_impl(
    workdir: &Path,
    trust: ConfigTrust,
) -> Result<ValidatedConfig, LoadConfigError> {
    let path = workdir.join("roko.toml");
    if !path.exists() {
        let mut validated = ValidatedConfig::from_config(RokoConfig::default());
        validated.provenance.push(ConfigProvenance::default(
            "roko.toml",
            "missing file; using built-in defaults",
        ));
        return Ok(validated);
    }

    let text = std::fs::read_to_string(&path).map_err(|source| LoadConfigError::Read {
        path: path.clone(),
        source,
    })?;

    // Only apply strict safety checks when the config might be inherited
    // from an untrusted source. Local configs (serve, daemon) skip this.
    if trust == ConfigTrust::Shared {
        let strict_source = StrictConfigSource::shared(Some(path.clone()));
        validate_strict_config_toml(&text, &strict_source).map_err(|source| {
            LoadConfigError::Validation {
                path: path.clone(),
                source,
            }
        })?;
    }

    let raw: RokoConfig = toml::from_str(&text).map_err(|source| LoadConfigError::Parse {
        path: path.clone(),
        source,
    })?;

    let mut migrated = raw.clone();
    migrated.interpolate_env_vars();
    migrated.resolve_file_secrets();

    let mut diagnostics = Vec::new();
    if raw.config_version < schema::CURRENT_CONFIG_VERSION {
        diagnostics.push(ConfigDiagnostic {
            key: "config_version".to_string(),
            message: format!(
                "config_version={} is older than current {}; consider running a migration",
                raw.config_version,
                schema::CURRENT_CONFIG_VERSION,
            ),
        });
    }

    let provenance = vec![ConfigProvenance::file(path.clone(), "roko.toml")];

    Ok(ValidatedConfig {
        raw,
        migrated,
        diagnostics,
        provenance,
    })
}

/// Load configuration from an explicit file path.
///
/// Unlike [`load_config`] which constructs a path from a workdir,
/// this loads directly from the given path. Missing files return an error
/// (not a default) since the caller explicitly requested this path.
pub fn load_config_from_path(path: &Path) -> Result<RokoConfig, LoadConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    let strict_source = StrictConfigSource::shared(Some(path.to_path_buf()));
    validate_strict_config_toml(&text, &strict_source).map_err(|source| {
        LoadConfigError::Validation {
            path: path.to_path_buf(),
            source,
        }
    })?;

    let mut config: RokoConfig =
        toml::from_str(&text).map_err(|source| LoadConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;

    config.interpolate_env_vars();
    config.resolve_file_secrets();
    Ok(config)
}

/// Load configuration from an explicit file path without strict safety validation.
///
/// This skips the `dangerously_skip_permissions` check, making it suitable for
/// contexts (like ACP) that don't enforce permission semantics and just need
/// provider/model configuration.
pub fn load_config_from_path_lenient(path: &Path) -> Result<RokoConfig, LoadConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    let mut config: RokoConfig =
        toml::from_str(&text).map_err(|source| LoadConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;

    config.interpolate_env_vars();
    config.resolve_file_secrets();
    Ok(config)
}

#[cfg(test)]
mod load_config_tests {
    use super::*;

    #[test]
    fn strict_rejects_dangerously_skip_permissions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let toml_text = "[runner]\ndangerously_skip_permissions = true\n";
        std::fs::write(dir.path().join("roko.toml"), toml_text).expect("write roko.toml");

        let err =
            load_config_strict(dir.path()).expect_err("must reject dangerous shared override");
        assert!(
            matches!(err, LoadConfigError::Validation { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn local_permits_dangerously_skip_permissions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let toml_text = "[runner]\ndangerously_skip_permissions = true\n";
        std::fs::write(dir.path().join("roko.toml"), toml_text).expect("write roko.toml");

        let validated = load_config(dir.path()).expect("local trust must permit dangerous flag");
        assert!(validated.config().runner.dangerously_skip_permissions);
    }

    #[test]
    fn missing_roko_toml_returns_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let validated = load_config(dir.path()).expect("default load ok");
        assert_eq!(validated.config(), &RokoConfig::default());
        // Default path should still emit no soft warnings.
        assert!(validated.diagnostics().is_empty());
    }

    #[test]
    fn clean_config_has_empty_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let toml_text = format!(
            "config_version = {}\nschema_version = {}\n",
            schema::CURRENT_CONFIG_VERSION,
            schema::CURRENT_SCHEMA_VERSION,
        );
        std::fs::write(dir.path().join("roko.toml"), toml_text).expect("write roko.toml");

        let validated = load_config(dir.path()).expect("clean load ok");
        assert!(
            validated.diagnostics().is_empty(),
            "unexpected diagnostics: {:?}",
            validated.diagnostics()
        );
        assert!(!validated.provenance().is_empty(), "provenance missing");
    }

    #[test]
    fn outdated_config_version_produces_soft_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Pin config_version to 1 to trip the soft-warning check.
        let toml_text = "config_version = 1\n";
        std::fs::write(dir.path().join("roko.toml"), toml_text).expect("write roko.toml");

        let validated = load_config(dir.path()).expect("older config still loads");
        let diagnostics = validated.diagnostics();
        assert_eq!(diagnostics.len(), 1, "got {diagnostics:?}");
        assert_eq!(diagnostics[0].key, "config_version");
    }

    #[test]
    fn into_config_returns_inner_roko_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = load_config(dir.path())
            .expect("default load ok")
            .into_config();
        assert_eq!(cfg, RokoConfig::default());
    }

    #[test]
    fn toml_serialize_roundtrip_default_config() {
        let original = RokoConfig::default();
        let serialized =
            toml::to_string_pretty(&original).expect("serialize RokoConfig to TOML");
        let deserialized: RokoConfig =
            toml::from_str(&serialized).expect("deserialize RokoConfig from TOML");
        assert_eq!(original, deserialized, "roundtrip mismatch");
    }

    #[test]
    fn toml_serialize_roundtrip_with_providers() {
        let mut config = RokoConfig::default();
        config.providers.insert(
            "test-provider".to_string(),
            schema::ProviderConfig {
                kind: crate::agent::ProviderKind::OpenAiCompat,
                base_url: Some("https://api.example.com/v1".to_string()),
                api_key_env: Some("TEST_API_KEY".to_string()),
                command: None,
                args: None,
                timeout_ms: Some(120_000),
                ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
                connect_timeout_ms: Some(5_000),
                extra_headers: None,
                max_concurrent: Some(8),
            },
        );
        config.models.insert(
            "test-model".to_string(),
            schema::ModelProfile {
                provider: "test-provider".to_string(),
                slug: "test-model-v1".to_string(),
                context_window: 128_000,
                supports_tools: true,
                tool_format: "openai_json".to_string(),
                ..Default::default()
            },
        );
        config.runner.dangerously_skip_permissions = true;
        let serialized =
            toml::to_string_pretty(&config).expect("serialize config with providers");
        let deserialized: RokoConfig =
            toml::from_str(&serialized).expect("deserialize config with providers");
        assert_eq!(config, deserialized, "roundtrip mismatch with providers");
    }

    #[test]
    fn project_roko_toml_loads_successfully() {
        // Verify the actual project roko.toml loads through the unified loader.
        let project_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("find workspace root");
        let roko_toml = project_root.join("roko.toml");
        if roko_toml.exists() {
            let validated = load_config(project_root)
                .expect("project roko.toml must load through unified loader");
            let config = validated.config();
            assert!(
                !config.providers.is_empty(),
                "project config should have providers"
            );
            // Verify it serializes back to valid TOML.
            let serialized =
                toml::to_string_pretty(config).expect("serialize project config");
            let _: RokoConfig =
                toml::from_str(&serialized).expect("re-parse serialized project config");
        }
    }
}
