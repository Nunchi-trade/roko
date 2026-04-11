//! Roko runtime configuration.
//!
//! # Modules
//!
//! - [`schema`] -- The unified `RokoConfig` type with hierarchical sections.
//! - [`compat`] -- Reader for legacy Mori `config.toml` format.
//! - [`presets`] -- Named presets (minimal / balanced / thorough).

use std::path::Path;

use thiserror::Error;

pub mod compat;
pub mod presets;
pub mod schema;

// Re-exports for ergonomic use.
pub use compat::from_mori_toml;
pub use presets::Preset;
pub use schema::{
    AgentConfig, AgentRoleToggles, BudgetConfig, CURRENT_SCHEMA_VERSION, ConductorConfig,
    GatesConfig, LearningConfig, PrdConfig, ProjectConfig, RokoConfig, RoleOverride,
    RoutingAlgorithm, RoutingConfig, SchedulerConfig, SchedulerCronConfig, ServeAuthConfig,
    ServeConfig, ServeDeployConfig, ServeDeployWebhookConfig, ServerConfig, TuiConfig,
    WatcherConfig, WatcherPathConfig,
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
pub fn load_config(workdir: &Path) -> Result<RokoConfig, LoadConfigError> {
    let path = workdir.join("roko.toml");
    if !path.exists() {
        return Ok(RokoConfig::default());
    }

    let text = std::fs::read_to_string(&path).map_err(|source| LoadConfigError::Read {
        path: path.clone(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| LoadConfigError::Parse {
        path: path.clone(),
        source,
    })
}
