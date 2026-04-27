//! ACP server configuration.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Runtime configuration for the ACP stdio server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfig {
    /// Working directory used to resolve ACP operations.
    pub workdir: PathBuf,
    /// Named configuration profile for ACP sessions.
    pub profile: String,
    /// Optional path to an explicit Roko configuration file.
    pub config_path: Option<PathBuf>,
    /// Path to the file that receives ACP server logs.
    pub log_file: PathBuf,
}

impl AcpConfig {
    /// Creates a configuration using the provided ACP paths and profile.
    pub fn new(
        workdir: impl Into<PathBuf>,
        profile: impl Into<String>,
        config_path: Option<PathBuf>,
        log_file: impl Into<PathBuf>,
    ) -> Self {
        Self {
            workdir: workdir.into(),
            profile: profile.into(),
            config_path,
            log_file: log_file.into(),
        }
    }

    /// Returns the configured log file path.
    pub fn log_file(&self) -> &Path {
        &self.log_file
    }

    /// Load the workspace `RokoConfig` from `workdir/roko.toml`.
    pub fn load_roko_config(&self) -> roko_core::config::schema::RokoConfig {
        match roko_core::config::load_config(&self.workdir) {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load roko.toml, using defaults");
                roko_core::config::schema::RokoConfig::default()
            }
        }
    }
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            workdir: std::env::current_dir().unwrap_or_default(),
            profile: "default".to_owned(),
            config_path: None,
            log_file: PathBuf::from(".roko/acp.log"),
        }
    }
}
