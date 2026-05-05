//! Strict config validation helpers for safety-sensitive settings.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml::Value;

use super::provenance::ConfigSource;

/// Explicit source mode for strict config validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StrictConfigSource {
    Shared { path: Option<PathBuf> },
    LocalOverride { path: PathBuf },
    TestFixture { path: PathBuf },
}

impl StrictConfigSource {
    #[must_use]
    pub fn shared(path: impl Into<Option<PathBuf>>) -> Self {
        Self::Shared { path: path.into() }
    }

    #[must_use]
    pub fn local_override(path: impl Into<PathBuf>) -> Self {
        Self::LocalOverride { path: path.into() }
    }

    #[must_use]
    pub fn test_fixture(path: impl Into<PathBuf>) -> Self {
        Self::TestFixture { path: path.into() }
    }

    fn allows_dangerous_permission_skip(&self) -> bool {
        matches!(self, Self::LocalOverride { .. } | Self::TestFixture { .. })
    }

    fn path(&self) -> Option<&Path> {
        match self {
            Self::Shared { path } => path.as_deref(),
            Self::LocalOverride { path } | Self::TestFixture { path } => Some(path.as_path()),
        }
    }
}

/// Strict config validation error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StrictConfigValidationError {
    #[error("parse config toml: {0}")]
    ParseToml(String),
    #[error("runner.dangerously_skip_permissions=true is not valid in shared config{path_suffix}")]
    DangerousRootPermission { path_suffix: String },
}

/// Validate safety-sensitive raw TOML settings without changing legacy load behavior.
///
/// This checks only explicitly configured values. An absent
/// `runner.dangerously_skip_permissions` key is accepted here so strict validation
/// can be introduced without changing deserialization defaults in this packet.
pub fn validate_strict_config_toml(
    toml_text: &str,
    source: &StrictConfigSource,
) -> Result<(), StrictConfigValidationError> {
    let value = toml_text
        .parse::<Value>()
        .map_err(|err| StrictConfigValidationError::ParseToml(err.to_string()))?;

    let dangerous_skip = value
        .get("runner")
        .and_then(|runner| runner.get("dangerously_skip_permissions"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if dangerous_skip && !source.allows_dangerous_permission_skip() {
        let path_suffix = source
            .path()
            .map(|path| format!(" ({})", path.display()))
            .unwrap_or_default();
        return Err(StrictConfigValidationError::DangerousRootPermission { path_suffix });
    }

    Ok(())
}

/// Local-only permission bypass metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DangerousPermissionOverride {
    pub enabled: bool,
    pub scope: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    pub ack_env: String,
    pub source: ConfigSource,
}

impl DangerousPermissionOverride {
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            scope: String::new(),
            reason: String::new(),
            expires_at: None,
            ack_env: String::new(),
            source: ConfigSource::Default,
        }
    }

    pub fn validate_at(&self, now: DateTime<Utc>) -> Result<(), DangerousPermissionOverrideError> {
        if !self.enabled {
            return Ok(());
        }

        if self.reason.trim().is_empty() {
            return Err(DangerousPermissionOverrideError::MissingReason);
        }
        if self.scope.trim().is_empty() {
            return Err(DangerousPermissionOverrideError::MissingScope);
        }
        let expires_at = self
            .expires_at
            .ok_or(DangerousPermissionOverrideError::MissingExpiry)?;
        if expires_at <= now {
            return Err(DangerousPermissionOverrideError::Expired);
        }
        if self.source != ConfigSource::LocalOverride {
            return Err(DangerousPermissionOverrideError::NonLocalSource);
        }
        if self.ack_env.trim().is_empty() {
            return Err(DangerousPermissionOverrideError::MissingAcknowledgementEnv);
        }

        Ok(())
    }
}

/// Validation failure for a local dangerous permission override.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DangerousPermissionOverrideError {
    #[error("dangerous permission override requires a non-empty reason")]
    MissingReason,
    #[error("dangerous permission override requires a non-empty scope")]
    MissingScope,
    #[error("dangerous permission override requires an expiry")]
    MissingExpiry,
    #[error("dangerous permission override has expired")]
    Expired,
    #[error("dangerous permission override source must be local_override")]
    NonLocalSource,
    #[error("dangerous permission override requires an acknowledgement env name")]
    MissingAcknowledgementEnv,
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn valid_override(now: DateTime<Utc>) -> DangerousPermissionOverride {
        DangerousPermissionOverride {
            enabled: true,
            scope: "command:plan".to_string(),
            reason: "local smoke test".to_string(),
            expires_at: Some(now + Duration::hours(1)),
            ack_env: "ROKO_ACK_DANGEROUS_PERMISSIONS".to_string(),
            source: ConfigSource::LocalOverride,
        }
    }

    #[test]
    fn dangerously_skip_permissions_root_shared_config_fails() {
        let err = validate_strict_config_toml(
            "[runner]\ndangerously_skip_permissions = true\n",
            &StrictConfigSource::shared(Some(PathBuf::from("roko.toml"))),
        )
        .expect_err("shared config must reject dangerous skip");

        assert!(matches!(
            err,
            StrictConfigValidationError::DangerousRootPermission { .. }
        ));
    }

    #[test]
    fn dangerously_skip_permissions_false_or_absent_passes() {
        validate_strict_config_toml(
            "[runner]\ndangerously_skip_permissions = false\n",
            &StrictConfigSource::shared(Some(PathBuf::from("roko.toml"))),
        )
        .expect("false is allowed");

        validate_strict_config_toml(
            "[runner]\nplan_timeout_secs = 30\n",
            &StrictConfigSource::shared(None),
        )
        .expect("absent is allowed");
    }

    #[test]
    fn dangerously_skip_permissions_test_or_local_source_passes() {
        validate_strict_config_toml(
            "[runner]\ndangerously_skip_permissions = true\n",
            &StrictConfigSource::local_override(".roko/local-overrides.toml"),
        )
        .expect("local override source is explicit");

        validate_strict_config_toml(
            "[runner]\ndangerously_skip_permissions = true\n",
            &StrictConfigSource::test_fixture("tests/fixtures/dangerous.roko.toml"),
        )
        .expect("test fixture source is explicit");
    }

    #[test]
    fn dangerous_permission_override_accepts_valid_local_override() {
        let now = Utc::now();

        valid_override(now)
            .validate_at(now)
            .expect("valid local override should pass");
    }

    #[test]
    fn dangerous_permission_override_requires_reason() {
        let now = Utc::now();
        let mut override_policy = valid_override(now);
        override_policy.reason.clear();

        assert_eq!(
            override_policy.validate_at(now),
            Err(DangerousPermissionOverrideError::MissingReason)
        );
    }

    #[test]
    fn dangerous_permission_override_requires_scope() {
        let now = Utc::now();
        let mut override_policy = valid_override(now);
        override_policy.scope.clear();

        assert_eq!(
            override_policy.validate_at(now),
            Err(DangerousPermissionOverrideError::MissingScope)
        );
    }

    #[test]
    fn dangerous_permission_override_requires_future_expiry() {
        let now = Utc::now();
        let mut override_policy = valid_override(now);
        override_policy.expires_at = None;
        assert_eq!(
            override_policy.validate_at(now),
            Err(DangerousPermissionOverrideError::MissingExpiry)
        );

        override_policy.expires_at = Some(now - Duration::seconds(1));
        assert_eq!(
            override_policy.validate_at(now),
            Err(DangerousPermissionOverrideError::Expired)
        );
    }

    #[test]
    fn dangerous_permission_override_requires_local_source() {
        let now = Utc::now();
        let mut override_policy = valid_override(now);
        override_policy.source = ConfigSource::File;

        assert_eq!(
            override_policy.validate_at(now),
            Err(DangerousPermissionOverrideError::NonLocalSource)
        );
    }

    #[test]
    fn dangerous_permission_override_requires_ack_env() {
        let now = Utc::now();
        let mut override_policy = valid_override(now);
        override_policy.ack_env.clear();

        assert_eq!(
            override_policy.validate_at(now),
            Err(DangerousPermissionOverrideError::MissingAcknowledgementEnv)
        );
    }
}
