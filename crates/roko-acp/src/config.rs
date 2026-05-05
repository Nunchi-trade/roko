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
    /// Optional shared/global configuration file to merge with the workspace/editor config.
    pub global_config_path: Option<PathBuf>,
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
            global_config_path: None,
            log_file: log_file.into(),
        }
    }

    /// Attach an explicit global config file.
    #[must_use]
    pub fn with_global_config(mut self, global_config_path: Option<PathBuf>) -> Self {
        self.global_config_path = global_config_path;
        self
    }

    /// Returns the configured log file path.
    pub fn log_file(&self) -> &Path {
        &self.log_file
    }

    /// Return config files relevant to this ACP process, in effective load order.
    ///
    /// Each entry uses a prefix indicating where the config came from:
    /// - `"global:"` — explicit `--global-config` path or implicit `~/.roko/config.toml`
    /// - `"project:"` — workspace `roko.toml` or explicit `--config` path
    /// - `"env:"` — `ROKO_CONFIG` environment variable
    /// - `"default:"` — implicit `~/.roko/config.toml` merged by the core loader
    ///
    /// Only paths that exist on disk are included. Ordering:
    /// 1. Explicit global config (`--global-config`)
    /// 2. Project config (workspace `roko.toml` or explicit `--config`)
    /// 3. `ROKO_CONFIG` env var
    /// 4. Implicit canonical global config (`~/.roko/config.toml`) when no
    ///    explicit `--global-config` was provided and the core loader merged it
    #[must_use]
    pub fn config_sources(&self) -> Vec<String> {
        let mut sources = Vec::new();

        // 1. Explicit --global-config (highest-priority global layer).
        if let Some(path) = self.global_config_path.as_ref() {
            if path.is_file() {
                sources.push(format!("global:{}", display_path(path)));
            }
        }

        // 2. Project config: explicit --config or workspace roko.toml.
        match self.config_path.as_ref() {
            Some(path) => {
                if path.is_file() {
                    sources.push(format!("project:{}", display_path(path)));
                }
            }
            None => {
                let workspace_toml = self.workdir.join("roko.toml");
                if workspace_toml.is_file() {
                    sources.push(format!("project:{}", display_path(&workspace_toml)));
                }
            }
        }

        // 3. ROKO_CONFIG env var.
        if let Ok(path) = std::env::var("ROKO_CONFIG")
            && !path.trim().is_empty()
        {
            let env_path = std::path::Path::new(path.trim());
            if env_path.is_file() {
                sources.push(format!("env:{}", display_path(env_path)));
            }
        }

        // 4. Implicit canonical global config (~/.roko/config.toml) when the
        //    core loader would merge it (no explicit --global-config provided).
        if self.global_config_path.is_none() {
            let implicit = roko_core::config::loader::global_config_path();
            if implicit.is_file() {
                sources.push(format!("default:{}", display_path(&implicit)));
            }
        }

        sources
    }

    /// Load the workspace `RokoConfig` and return a warning string if the config
    /// file exists but failed to parse or contains unknown fields.
    ///
    /// Returns `(RokoConfig, None)` when no config file exists (normal, use defaults).
    /// Returns `(RokoConfig, Some(reason))` when a file exists but loading failed.
    /// Returns `(RokoConfig, None)` on success.
    pub fn load_roko_config_with_warning(
        &self,
    ) -> (roko_core::config::schema::RokoConfig, Option<String>) {
        let opts = roko_core::config::loader::LoadOptions::acp();

        // Check project config (explicit or implicit workspace roko.toml).
        let project_warning = match self.config_path.as_deref() {
            Some(path) => {
                // Explicit --config path: warn if it exists but fails to load.
                if path.is_file() {
                    let mut check_opts = opts.clone();
                    check_opts.merge_global = false;
                    match roko_core::config::loader::load_config_file(path, &check_opts) {
                        Ok(_) => None,
                        Err(e) => {
                            Some(format!("roko.toml parse error ({}): {e:#}", path.display()))
                        }
                    }
                } else {
                    None
                }
            }
            None => {
                // Implicit workspace file: warn only if file exists and fails.
                let candidate = self.workdir.join("roko.toml");
                if candidate.is_file() {
                    let mut check_opts = opts.clone();
                    check_opts.merge_global = false;
                    match roko_core::config::loader::load_config_file(&candidate, &check_opts) {
                        Ok(_) => None,
                        Err(e) => Some(format!(
                            "roko.toml parse error ({}): {e:#}",
                            candidate.display()
                        )),
                    }
                } else {
                    None
                }
            }
        };

        // Check global config (explicit or implicit ~/.roko/config.toml).
        let global_warning = if let Some(global_path) = self.global_config_path.as_deref() {
            if global_path.is_file() {
                let mut check_opts = opts.clone();
                check_opts.merge_global = false;
                match roko_core::config::loader::load_config_file(global_path, &check_opts) {
                    Ok(_) => None,
                    Err(e) => Some(format!(
                        "global config parse error ({}): {e:#}",
                        global_path.display()
                    )),
                }
            } else {
                None
            }
        } else {
            let implicit = roko_core::config::loader::global_config_path();
            if implicit.is_file() {
                let mut check_opts = opts;
                check_opts.merge_global = false;
                match roko_core::config::loader::load_config_file(&implicit, &check_opts) {
                    Ok(_) => None,
                    Err(e) => Some(format!(
                        "global config parse error ({}): {e:#}",
                        implicit.display()
                    )),
                }
            } else {
                None
            }
        };

        // Combine warnings from project and global config.
        let warning = match (project_warning, global_warning) {
            (Some(pw), Some(gw)) => Some(format!("{pw}; {gw}")),
            (Some(w), None) | (None, Some(w)) => Some(w),
            (None, None) => None,
        };

        (self.load_roko_config(), warning)
    }

    /// Load the workspace `RokoConfig`.
    ///
    /// If an explicit `--config` path is set, loads from that path (lenient,
    /// with global merge). Otherwise delegates to the unified loader which
    /// handles `ROKO_CONFIG` env var, ancestor walk, global merge, env
    /// overrides, and secret resolution.
    pub fn load_roko_config(&self) -> roko_core::config::schema::RokoConfig {
        let opts = roko_core::config::loader::LoadOptions::acp();
        let mut local_opts = opts.clone();
        // The explicit ACP global config is handled below so callers can pass
        // nonstandard locations such as ~/.nunchi/roko/roko.toml.
        local_opts.merge_global = self.global_config_path.is_none();
        let mut cfg = match self.config_path.as_deref() {
            Some(path) => roko_core::config::loader::load_config_file(path, &local_opts),
            None => roko_core::config::loader::load_config_with_options(&self.workdir, &local_opts),
        }
        .unwrap_or_default();
        if let Some(global_path) = self.global_config_path.as_deref() {
            let mut global_opts = opts;
            global_opts.merge_global = false;
            if let Ok(global_cfg) =
                roko_core::config::loader::load_config_file(global_path, &global_opts)
            {
                merge_inherited_config(&mut cfg, global_cfg);
            }
        }
        cfg
    }
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn merge_inherited_config(
    config: &mut roko_core::config::schema::RokoConfig,
    global: roko_core::config::schema::RokoConfig,
) {
    let local_default_model_before_merge = config.agent.default_model.clone();
    let local_default_model_declared_before_merge = config
        .models
        .contains_key(local_default_model_before_merge.trim());
    let roko_core::config::schema::RokoConfig {
        providers,
        models,
        agent,
        ..
    } = global;
    for (name, provider) in providers {
        config.providers.entry(name).or_insert(provider);
    }
    for (name, model) in models {
        config.models.entry(name).or_insert(model);
    }
    let roko_core::config::AgentConfig {
        default_model,
        default_backend,
        default_effort,
        ..
    } = agent;
    if should_inherit_default_model(config, local_default_model_declared_before_merge)
        && !default_model.is_empty()
    {
        config.agent.default_model = default_model;
    }
    if config.agent.default_backend.is_empty() && !default_backend.is_empty() {
        config.agent.default_backend = default_backend;
    }
    if config.agent.default_effort.is_empty() && !default_effort.is_empty() {
        config.agent.default_effort = default_effort;
    }
}

fn should_inherit_default_model(
    config: &roko_core::config::schema::RokoConfig,
    local_default_model_declared_before_merge: bool,
) -> bool {
    if config.agent.default_model.trim().is_empty() {
        return true;
    }
    let built_in_default = roko_core::config::AgentConfig::default().default_model;
    config.agent.default_model == built_in_default && !local_default_model_declared_before_merge
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            workdir: std::env::current_dir().unwrap_or_default(),
            profile: "default".to_owned(),
            config_path: None,
            global_config_path: None,
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

    #[test]
    fn explicit_global_config_inherits_registries_without_overriding_ide_settings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let editor_path = dir.path().join("editor-config.toml");
        let global_path = dir.path().join("global-config.toml");
        std::fs::write(
            &editor_path,
            r#"
config_version = 2
schema_version = 2

[agent]
bare_mode = true
"#,
        )
        .expect("write editor config");
        std::fs::write(
            &global_path,
            r#"
config_version = 2
schema_version = 2

[agent]
default_model = "global-model"
default_effort = "high"

[providers.global-provider]
kind = "openai_compat"
base_url = "https://global.example/v1"

[models.global-model]
provider = "global-provider"
slug = "global-slug"
"#,
        )
        .expect("write global config");

        let acp_config = AcpConfig::new(
            dir.path(),
            "default",
            Some(editor_path),
            dir.path().join("acp.log"),
        )
        .with_global_config(Some(global_path));
        let config = acp_config.load_roko_config();

        assert!(config.agent.bare_mode);
        assert_eq!(config.agent.default_model, "global-model");
        assert_eq!(config.agent.default_effort, "medium");
        assert!(config.providers.contains_key("global-provider"));
        assert!(config.models.contains_key("global-model"));
    }

    // ---- config_sources tests ----

    #[test]
    fn config_sources_includes_explicit_global_with_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global_path = dir.path().join("global.toml");
        std::fs::write(&global_path, "config_version = 2\nschema_version = 2\n")
            .expect("write global");

        let acp = AcpConfig::new(dir.path(), "default", None, dir.path().join("acp.log"))
            .with_global_config(Some(global_path.clone()));
        let sources = acp.config_sources();

        assert!(
            sources.iter().any(|s| s.starts_with("global:")),
            "expected a global: prefixed entry, got {sources:?}"
        );
    }

    #[test]
    fn config_sources_includes_workspace_toml_with_project_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("roko.toml"),
            "config_version = 2\nschema_version = 2\n",
        )
        .expect("write roko.toml");

        let acp = AcpConfig::new(dir.path(), "default", None, dir.path().join("acp.log"));
        let sources = acp.config_sources();

        assert!(
            sources.iter().any(|s| s.starts_with("project:")),
            "expected a project: prefixed entry, got {sources:?}"
        );
    }

    #[test]
    fn config_sources_includes_explicit_config_path_with_project_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let explicit = dir.path().join("editor.toml");
        std::fs::write(&explicit, "config_version = 2\nschema_version = 2\n")
            .expect("write editor.toml");

        let acp = AcpConfig::new(
            dir.path(),
            "default",
            Some(explicit),
            dir.path().join("acp.log"),
        );
        let sources = acp.config_sources();

        assert!(
            sources.iter().any(|s| s.starts_with("project:")),
            "expected a project: prefixed entry for explicit --config, got {sources:?}"
        );
    }

    #[test]
    fn config_sources_excludes_missing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No roko.toml, no global config, no ROKO_CONFIG.
        let acp = AcpConfig::new(dir.path(), "default", None, dir.path().join("acp.log"));

        // This may contain a `default:` entry if ~/.roko/config.toml exists
        // on the test machine, but should never contain a project: entry.
        let sources = acp.config_sources();
        assert!(
            !sources.iter().any(|s| s.starts_with("project:")),
            "missing roko.toml should not produce a project: entry, got {sources:?}"
        );
    }

    #[test]
    fn config_sources_ordering_global_before_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global_path = dir.path().join("global.toml");
        std::fs::write(&global_path, "config_version = 2\nschema_version = 2\n")
            .expect("write global");
        std::fs::write(
            dir.path().join("roko.toml"),
            "config_version = 2\nschema_version = 2\n",
        )
        .expect("write roko.toml");

        let acp = AcpConfig::new(dir.path(), "default", None, dir.path().join("acp.log"))
            .with_global_config(Some(global_path));
        let sources = acp.config_sources();

        let global_pos = sources.iter().position(|s| s.starts_with("global:"));
        let project_pos = sources.iter().position(|s| s.starts_with("project:"));

        assert!(
            global_pos.is_some() && project_pos.is_some(),
            "expected both global and project entries, got {sources:?}"
        );
        assert!(
            global_pos.unwrap() < project_pos.unwrap(),
            "global config must come before project config in sources ordering"
        );
    }

    #[test]
    fn config_sources_implicit_global_uses_default_prefix() {
        // When no --global-config is set, the implicit ~/.roko/config.toml
        // should appear with a "default:" prefix (if the file exists on disk).
        let dir = tempfile::tempdir().expect("tempdir");
        let acp = AcpConfig::new(dir.path(), "default", None, dir.path().join("acp.log"));
        let sources = acp.config_sources();

        // The implicit global may or may not exist on the test machine.
        // If it does, it must use the "default:" prefix, never "global:".
        for s in &sources {
            if s.contains(".roko/config.toml") || s.contains(".roko\\config.toml") {
                assert!(
                    s.starts_with("default:"),
                    "implicit global config must use default: prefix, got {s}"
                );
            }
        }
    }
}
