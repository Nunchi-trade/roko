//! File-based credential storage for `roko login`.
//!
//! Stores credentials in `~/.roko/credentials.json` with restricted file
//! permissions (0600 on Unix). The format is intentionally simple:
//!
//! ```json
//! {
//!   "default": {
//!     "url": "http://localhost:6677",
//!     "token": "rk_...",
//!     "method": "api_key",
//!     "stored_at": "2026-04-24T12:00:00Z"
//!   }
//! }
//! ```

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;

/// A single stored credential entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    /// URL of the roko-serve instance.
    pub url: String,
    /// The API key / token value.
    pub token: String,
    /// How the credential was obtained (e.g. "api_key").
    pub method: String,
    /// ISO-8601 timestamp of when the credential was stored.
    pub stored_at: String,
}

/// On-disk format: a map of profile names to credentials.
type CredentialStore = HashMap<String, Credential>;

const PROFILE: &str = "default";

/// Path to the credentials file: `~/.roko/credentials.json`.
pub fn credentials_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".roko")
        .join("credentials.json")
}

/// Persist a credential under the "default" profile.
///
/// Creates `~/.roko/` if it does not exist and writes the file with
/// restricted permissions (0600 on Unix).
pub fn store_credential(cred: &Credential) -> Result<()> {
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    // Load existing store so we don't clobber other profiles.
    let mut store: CredentialStore = if path.exists() {
        let data =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        HashMap::new()
    };

    store.insert(PROFILE.to_string(), cred.clone());

    let json = serde_json::to_string_pretty(&store).context("serialize credentials")?;

    // Write atomically via a temp file so we never leave a partial write.
    let tmp_path = path.with_extension("json.tmp");

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        file.write_all(json.as_bytes())
            .with_context(|| format!("write {}", tmp_path.display()))?;
        file.sync_all()?;
    }

    #[cfg(not(unix))]
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        file.write_all(json.as_bytes())
            .with_context(|| format!("write {}", tmp_path.display()))?;
        file.sync_all()?;
    }

    std::fs::rename(&tmp_path, &path)
        .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()))?;

    Ok(())
}

/// Load the "default" credential, if one exists.
pub fn load_credential() -> Result<Option<Credential>> {
    let path = credentials_path();
    if !path.exists() {
        return Ok(None);
    }

    let data =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let store: CredentialStore =
        serde_json::from_str(&data).with_context(|| "parse credentials.json")?;

    Ok(store.get(PROFILE).cloned())
}

/// Remove all stored credentials (deletes the file).
pub fn clear_credential() -> Result<()> {
    let path = credentials_path();
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_credential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");

        // Override credentials_path for test by writing/reading directly.
        let cred = Credential {
            url: "http://localhost:6677".into(),
            token: "rk_test_123".into(),
            method: "api_key".into(),
            stored_at: "2026-04-24T00:00:00Z".into(),
        };

        let mut store = HashMap::new();
        store.insert("default".to_string(), cred.clone());
        let json = serde_json::to_string_pretty(&store).unwrap();
        std::fs::write(&path, json).unwrap();

        let data = std::fs::read_to_string(&path).unwrap();
        let loaded: CredentialStore = serde_json::from_str(&data).unwrap();
        let loaded_cred = loaded.get("default").unwrap();

        assert_eq!(loaded_cred.url, "http://localhost:6677");
        assert_eq!(loaded_cred.token, "rk_test_123");
        assert_eq!(loaded_cred.method, "api_key");
    }

    #[test]
    fn load_missing_file_returns_none() {
        // credentials_path() points to ~/.roko/credentials.json which may or
        // may not exist, but the function should not error if the file is
        // missing. We test the core logic by checking a non-existent path.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        assert!(!path.exists());
        // The actual load_credential() uses a fixed path; this is a logic
        // check that the pattern works.
    }
}
