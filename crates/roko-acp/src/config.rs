//! ACP server configuration.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Runtime configuration for the ACP stdio server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfig {
    /// Path to the file that receives ACP server logs.
    pub log_file: PathBuf,
    /// Stable ACP agent name exposed during initialization.
    pub agent_name: String,
    /// Human-readable ACP agent title.
    pub agent_title: String,
    /// Agent version string reported to clients.
    pub agent_version: String,
}

impl AcpConfig {
    /// Creates a configuration using the provided log file path.
    pub fn new(log_file: impl Into<PathBuf>) -> Self {
        Self {
            log_file: log_file.into(),
            agent_name: "roko".to_owned(),
            agent_title: "Roko ACP".to_owned(),
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    /// Returns the configured log file path.
    pub fn log_file(&self) -> &Path {
        &self.log_file
    }
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self::new(".roko/roko-acp.log")
    }
}
