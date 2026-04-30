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
pub mod presets;
pub mod project;
pub mod provider;
pub mod routing;
pub mod schema;
pub mod serve;
pub mod subscriptions;
pub mod tools;
pub mod tui_cfg;

// Re-exports for ergonomic use.
pub use crate::temperament::Temperament;
pub use compat::from_mori_toml;
pub use presets::Preset;
// All section structs are re-exported from schema (which re-exports from submodules).
pub use schema::{
    AgentBudget, AgentConfig, AgentDefinition, AgentMode, AgentRoleToggles, AgentThresholds,
    ApiKeyEntry, AttentionConfig, BudgetConfig, CURRENT_SCHEMA_VERSION, ChainConfig,
    CompileFailRepeatConfig, ConductorConfig, ContextWindowPressureConfig, CostOverrunConfig,
    DataLlmConfig, DemurrageConfig, DeployConfig, EnergyConfig, GatesConfig, GeminiConfig,
    GhostTurnConfig, GithubWebhookConfig, GoalsConfig, ImmuneConfig, IterationLoopConfig,
    LearningConfig, ModelProfile, OneirographyConfig, PerplexityConfig, PipelineBandConfig,
    PipelineConfig, PipelineReviewerMode, PrdConfig, ProjectConfig, ProviderConfig,
    ProviderRouting, RelayConfig, ReviewLoopConfig, RewardWeights, RokoConfig, RoleOverride,
    RoutingAlgorithm, RoutingConfig, RoutingOverrides, RoutingRewardWeightsConfig, SafetySetting,
    SchedulerConfig, SchedulerCronConfig, ServeAuthConfig, ServeConfig, ServeDeployConfig,
    ServeDeployWebhookConfig, ServerConfig, SpecDriftConfig, StuckPatternConfig,
    SubscriptionConfig, SubscriptionFilterConfig, SubscriptionTrigger, TemporalConfig,
    TestFailureBudgetConfig, TimeOverrunConfig, ToolProfileConfig, ToolsConfig, TuiConfig,
    WatcherConfig, WatcherPathConfig, WatcherThresholds, WebhooksConfig,
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
}

/// Load the workspace configuration from `workdir/roko.toml`.
///
/// Missing files fall back to `RokoConfig::default()` so callers can start a
/// daemon in an uninitialized workspace.
///
/// After parsing, two secret-resolution passes run automatically:
///   1. `${VAR}` interpolation — expands environment variable references in
///      provider config strings.
///   2. `*_file` resolution — reads secrets from file paths in `extra_headers`
///      whose keys end with `_file`.
pub fn load_config(workdir: &Path) -> Result<RokoConfig, LoadConfigError> {
    let path = std::env::var_os("ROKO_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workdir.join("roko.toml"));
    load_config_file(&path)
}

/// Load configuration from an exact file path.
///
/// Missing files fall back to `RokoConfig::default()`, matching
/// [`load_config`].
pub fn load_config_file(path: &Path) -> Result<RokoConfig, LoadConfigError> {
    if !path.exists() {
        return Ok(RokoConfig::default());
    }

    let text = std::fs::read_to_string(&path).map_err(|source| LoadConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut config: RokoConfig =
        toml::from_str(&text).map_err(|source| LoadConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;

    // Secret resolution passes.
    config.interpolate_env_vars();
    config.resolve_file_secrets();

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_config_file_reads_the_exact_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let default_path = dir.path().join("roko.toml");
        let explicit_path = dir.path().join("local-dev.toml");

        std::fs::write(
            &default_path,
            "schema_version = 2\n[project]\nname = \"default\"\n",
        )
        .expect("write default config");
        std::fs::write(
            &explicit_path,
            "schema_version = 2\n[project]\nname = \"local-dev\"\n",
        )
        .expect("write explicit config");

        let cfg = load_config_file(&explicit_path).expect("load explicit config");

        assert_eq!(cfg.project.name, "local-dev");
    }
}
