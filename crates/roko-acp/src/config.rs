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

    /// Load the workspace `RokoConfig`.
    ///
    /// If an explicit `--config` path is set, loads from that path (lenient,
    /// with global merge). Otherwise delegates to the unified loader which
    /// handles `ROKO_CONFIG` env var, ancestor walk, global merge, env
    /// overrides, and secret resolution.
    pub fn load_roko_config(&self) -> roko_core::config::schema::RokoConfig {
        // If explicit config path is set, use it
        if let Some(ref path) = self.config_path
            && let Ok(config) = roko_core::config::load_config_from_path_lenient(path)
        {
            let mut config = config;
            roko_core::config::loader::merge_global_into(&mut config);
            return config;
        }
        // Otherwise use unified loader (handles ROKO_CONFIG, ancestor walk, global merge)
        roko_core::config::loader::load_config_unified(&self.workdir).unwrap_or_default()
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
