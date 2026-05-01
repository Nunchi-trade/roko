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

    /// Load the workspace `RokoConfig`, searching multiple sources in priority order:
    /// 1. Explicit `--config` path
    /// 2. `ROKO_CONFIG` env var
    /// 3. Parent-walk from workdir looking for `roko.toml`
    /// 4. `ROKO_WORKDIR` env var
    /// 5. [`roko_core::config::load_config`] on workdir (defaults + provenance when missing)
    pub fn load_roko_config(&self) -> roko_core::config::schema::RokoConfig {
        // 1. Explicit config path (--config flag)
        if let Some(ref path) = self.config_path {
            return Self::load_from_path(path);
        }

        // 2. ROKO_CONFIG env var
        if let Ok(env_path) = std::env::var("ROKO_CONFIG") {
            return Self::load_from_path(&PathBuf::from(env_path));
        }

        // 3. Walk up from workdir looking for roko.toml
        let mut dir = self.workdir.clone();
        loop {
            let candidate = dir.join("roko.toml");
            if candidate.exists() {
                return Self::load_from_path(&candidate);
            }
            if !dir.pop() {
                break;
            }
        }

        // 4. ROKO_WORKDIR env var
        if let Ok(workdir) = std::env::var("ROKO_WORKDIR") {
            let candidate = PathBuf::from(&workdir).join("roko.toml");
            if candidate.exists() {
                return Self::load_from_path(&candidate);
            }
        }

        // 5. Fall back to workdir-based load (validated defaults when file is absent)
        tracing::warn!("no roko.toml found in search paths; using load_config(workdir) defaults");
        match roko_core::config::load_config(&self.workdir) {
            Ok(validated) => validated.into_config(),
            Err(e) => {
                tracing::warn!(error = %e, "load_config failed; using built-in defaults");
                roko_core::config::schema::RokoConfig::default()
            }
        }
    }

    /// Load and parse a specific `roko.toml` file path, falling back to defaults on error.
    fn load_from_path(path: &Path) -> roko_core::config::schema::RokoConfig {
        match roko_core::config::load_config_from_path(path) {
            Ok(config) => {
                tracing::info!(
                    path = %path.display(),
                    providers = config.providers.len(),
                    "loaded roko.toml configuration"
                );
                config
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to load roko.toml, using defaults"
                );
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
