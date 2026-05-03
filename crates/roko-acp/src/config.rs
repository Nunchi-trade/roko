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
        let opts = roko_core::config::loader::LoadOptions::acp();
        match self.config_path.as_deref() {
            Some(path) => roko_core::config::loader::load_config_file(path, &opts),
            None => roko_core::config::loader::load_config_with_options(&self.workdir, &opts),
        }
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_roko_config_uses_explicit_config_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("roko.toml"),
            r#"
config_version = 2
schema_version = 2

[providers.parent-provider]
kind = "openai_compat"
base_url = "https://parent.example/v1"
"#,
        )
        .expect("write parent roko.toml");
        let explicit_path = dir.path().join("editor-config.toml");
        std::fs::write(
            &explicit_path,
            r#"
config_version = 2
schema_version = 2

[providers.explicit-provider]
kind = "openai_compat"
base_url = "https://explicit.example/v1"
"#,
        )
        .expect("write explicit config");

        let acp_config = AcpConfig::new(
            dir.path(),
            "default",
            Some(explicit_path),
            dir.path().join("acp.log"),
        );
        let config = acp_config.load_roko_config();

        assert!(config.providers.contains_key("explicit-provider"));
        assert!(!config.providers.contains_key("parent-provider"));
    }
}
