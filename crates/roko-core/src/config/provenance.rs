//! Typed provenance for config resolution.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::provider::{ModelAlias, ModelDefinition, ProviderDefinition, ProviderId};
use super::schema::RokoConfig;

/// Source category for a resolved config value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    File,
    Migration,
    Default,
    Env,
    LocalOverride,
    CliOverride,
}

/// Machine-readable trace for where a config value came from and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProvenance {
    pub source: ConfigSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ConfigProvenance {
    #[must_use]
    pub fn file(path: impl Into<PathBuf>, key: impl Into<String>) -> Self {
        Self {
            source: ConfigSource::File,
            path: Some(path.into()),
            key: key.into(),
            reason: None,
        }
    }

    #[must_use]
    pub fn default(key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            source: ConfigSource::Default,
            path: None,
            key: key.into(),
            reason: Some(reason.into()),
        }
    }

    #[must_use]
    pub fn migration(key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            source: ConfigSource::Migration,
            path: None,
            key: key.into(),
            reason: Some(reason.into()),
        }
    }

    #[must_use]
    pub fn env(key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            source: ConfigSource::Env,
            path: None,
            key: key.into(),
            reason: Some(reason.into()),
        }
    }

    #[must_use]
    pub fn local_override(
        path: impl Into<PathBuf>,
        key: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            source: ConfigSource::LocalOverride,
            path: Some(path.into()),
            key: key.into(),
            reason: Some(reason.into()),
        }
    }

    #[must_use]
    pub fn cli_override(key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            source: ConfigSource::CliOverride,
            path: None,
            key: key.into(),
            reason: Some(reason.into()),
        }
    }
}

/// Config diagnostic captured during migration or validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDiagnostic {
    pub key: String,
    pub message: String,
}

/// Parsed config after migration and validation, with provenance retained.
///
/// `raw` holds the config as deserialized before any migration step; `migrated`
/// is the authoritative post-migration value that callers consume via
/// [`ValidatedConfig::config`] / [`ValidatedConfig::into_config`]. Today the
/// two are identical because `load_config` does not perform schema migration;
/// future migration passes should populate `migrated` separately and leave
/// `raw` untouched for provenance/audit tooling.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidatedConfig {
    pub raw: RokoConfig,
    pub migrated: RokoConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ConfigDiagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<ConfigProvenance>,
}

impl ValidatedConfig {
    /// Wrap a fully-populated `RokoConfig` with empty diagnostics/provenance.
    ///
    /// Intended for callers (tests, synthesized configs) that construct a
    /// config in-process and don't need a provenance trace.
    #[must_use]
    pub fn from_config(config: RokoConfig) -> Self {
        Self {
            raw: config.clone(),
            migrated: config,
            diagnostics: Vec::new(),
            provenance: Vec::new(),
        }
    }

    /// Access the authoritative (post-migration) config.
    #[must_use]
    pub fn config(&self) -> &RokoConfig {
        &self.migrated
    }

    /// Consume the wrapper and return the post-migration `RokoConfig`.
    ///
    /// Prefer this over field access at call sites that don't need
    /// provenance or diagnostics.
    #[must_use]
    pub fn into_config(self) -> RokoConfig {
        self.migrated
    }

    /// Machine-readable provenance entries for each config key that was
    /// resolved from a non-default source (file, env, CLI override, etc.).
    #[must_use]
    pub fn provenance(&self) -> &[ConfigProvenance] {
        &self.provenance
    }

    /// Soft-warning diagnostics surfaced by the loader.
    ///
    /// Hard-rejection failures are returned as [`LoadConfigError`] and never
    /// appear here. Callers that want to display warnings to the user (CLI,
    /// TUI, dashboard) should iterate this slice.
    ///
    /// [`LoadConfigError`]: super::LoadConfigError
    #[must_use]
    pub fn diagnostics(&self) -> &[ConfigDiagnostic] {
        &self.diagnostics
    }
}

/// Runtime-ready config identities after resolution.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolvedRuntimeConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub providers: HashMap<ProviderId, ProviderDefinition>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub models: HashMap<ModelAlias, ModelDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<ConfigProvenance>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_provenance_constructs_all_sources() {
        let entries = [
            ConfigProvenance::file("roko.toml", "providers.anthropic.kind"),
            ConfigProvenance::default("agent.default_model", "built-in fallback"),
            ConfigProvenance::migration("agent.default_model", "migrated from agent.model"),
            ConfigProvenance::env(
                "providers.anthropic.api_key_env",
                "ANTHROPIC_API_KEY present",
            ),
            ConfigProvenance::local_override(
                ".roko/local-overrides.toml",
                "runner.dangerously_skip_permissions",
                "developer local override",
            ),
            ConfigProvenance::cli_override("agent.default_model", "--model"),
        ];

        assert_eq!(entries[0].source, ConfigSource::File);
        assert_eq!(
            entries[0].path.as_deref(),
            Some(std::path::Path::new("roko.toml"))
        );
        assert_eq!(entries[1].source, ConfigSource::Default);
        assert_eq!(entries[2].source, ConfigSource::Migration);
        assert_eq!(entries[3].source, ConfigSource::Env);
        assert_eq!(entries[4].source, ConfigSource::LocalOverride);
        assert_eq!(entries[5].source, ConfigSource::CliOverride);
        assert!(entries.iter().all(|entry| !entry.key.is_empty()));
    }

    #[test]
    fn config_provenance_validated_and_resolved_config_are_constructible() {
        let raw = RokoConfig::default();
        let migrated = raw.clone();
        let provenance = vec![ConfigProvenance::default(
            "config_version",
            "default config version",
        )];

        let validated = ValidatedConfig {
            raw,
            migrated,
            diagnostics: vec![ConfigDiagnostic {
                key: "config_version".to_string(),
                message: "already current".to_string(),
            }],
            provenance: provenance.clone(),
        };
        let resolved = ResolvedRuntimeConfig {
            providers: HashMap::new(),
            models: HashMap::new(),
            provenance,
        };

        assert_eq!(validated.provenance.len(), 1);
        assert_eq!(resolved.provenance.len(), 1);
    }
}
