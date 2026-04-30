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
    CompileFailRepeatConfig, ConductorConfig, ContextWindowPressureConfig, CoreRunnerConfig,
    CostOverrunConfig, DataLlmConfig, DemurrageConfig, DeployConfig, EnergyConfig, GatesConfig,
    GeminiConfig, GhostTurnConfig, GithubWebhookConfig, GoalsConfig, ImmuneConfig,
    IterationLoopConfig, LearningConfig, ModelProfile, OneirographyConfig, PerplexityConfig,
    PipelineBandConfig, PipelineConfig, PipelineReviewerMode, PrdConfig, ProjectConfig,
    ProviderConfig, ProviderRouting, RelayConfig, ReviewLoopConfig, RewardWeights, RokoConfig,
    RoleOverride, RoutingAlgorithm, RoutingConfig, RoutingOverrides, RoutingRewardWeightsConfig,
    SafetySetting, SchedulerConfig, SchedulerCronConfig, ServeAuthConfig, ServeConfig,
    ServeDeployConfig, ServeDeployWebhookConfig, ServerConfig, SpecDriftConfig, StuckPatternConfig,
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
    let path = workdir.join("roko.toml");
    if !path.exists() {
        return Ok(RokoConfig::default());
    }

    let text = std::fs::read_to_string(&path).map_err(|source| LoadConfigError::Read {
        path: path.clone(),
        source,
    })?;
    let mut config: RokoConfig =
        toml::from_str(&text).map_err(|source| LoadConfigError::Parse {
            path: path.clone(),
            source,
        })?;

    // Secret resolution passes.
    config.interpolate_env_vars();
    config.resolve_file_secrets();

    Ok(config)
}
