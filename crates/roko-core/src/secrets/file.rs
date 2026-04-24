//! File-backed [`SecretStore`] (§43.3).
//!
//! Reads and writes a TOML document such as:
//!
//! ```toml
//! [llm]
//! anthropic = "sk-..."
//! openai    = "sk-..."
//!
//! [rpc]
//! alchemy   = "..."
//! ```
//!
//! On unix, the backing file must have `0600` permissions. When [`FileStore::open`]
//! creates a missing file, it is created with `0600`. If the file exists with
//! looser permissions, open fails.

use super::{Namespace, SecretStore};
use crate::error::{Result, RokoError};
use parking_lot::RwLock;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Reads secrets from a TOML file. Path must have `0600` perms on unix
/// (enforced on open and re-enforced on every write).
pub struct FileStore {
    path: PathBuf,
    cache: RwLock<toml::Value>,
}

impl std::fmt::Debug for FileStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl FileStore {
    /// Open a file-backed store. Creates an empty TOML file (with `0600` perms
    /// on unix) if the path does not exist; otherwise verifies permissions
    /// and loads existing contents into an in-memory cache.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.exists() {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    fs::create_dir_all(parent)?;
                }
            }
            // Create empty TOML file with 0600 perms on unix.
            write_atomic_restricted(&path, "")?;
        }
        verify_permissions(&path)?;
        let text = fs::read_to_string(&path)?;
        let value: toml::Value = if text.trim().is_empty() {
            toml::Value::Table(toml::value::Table::new())
        } else {
            text.parse::<toml::Value>().map_err(|e| {
                RokoError::invalid(format!("file {:?} has invalid TOML: {e}", path.display()))
            })?
        };
        Ok(Self {
            path,
            cache: RwLock::new(value),
        })
    }

    /// Re-read the underlying file into the in-memory cache.
    pub fn refresh(&self) -> Result<()> {
        verify_permissions(&self.path)?;
        let text = fs::read_to_string(&self.path)?;
        let value: toml::Value = if text.trim().is_empty() {
            toml::Value::Table(toml::value::Table::new())
        } else {
            text.parse::<toml::Value>().map_err(|e| {
                RokoError::invalid(format!(
                    "file {:?} has invalid TOML: {e}",
                    self.path.display()
                ))
            })?
        };
        *self.cache.write() = value;
        Ok(())
    }

    /// Absolute path of the backing file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Delete a secret by namespace. Returns `true` if the key existed.
    pub fn delete(&self, ns: &Namespace) -> Result<bool> {
        let mut cache = self.cache.write();
        let existed = cache
            .as_table_mut()
            .and_then(|root| root.get_mut(&ns.category))
            .and_then(toml::Value::as_table_mut)
            .is_some_and(|cat| cat.remove(&ns.provider).is_some());
        if existed {
            let snapshot = cache.clone();
            drop(cache);
            self.persist(&snapshot)?;
        }
        Ok(existed)
    }

    /// List all `(category, provider)` pairs stored in the file.
    pub fn list_entries(&self) -> Vec<(String, String)> {
        let cache = self.cache.read();
        let mut entries = Vec::new();
        if let Some(root) = cache.as_table() {
            for (category, val) in root {
                if let Some(providers) = val.as_table() {
                    for provider in providers.keys() {
                        entries.push((category.clone(), provider.clone()));
                    }
                }
            }
        }
        entries
    }

    fn persist(&self, value: &toml::Value) -> Result<()> {
        let text = toml::to_string_pretty(value)
            .map_err(|e| RokoError::invalid(format!("serialize secrets toml: {e}")))?;
        write_atomic_restricted(&self.path, &text)
    }
}

impl SecretStore for FileStore {
    fn get(&self, ns: &Namespace) -> Result<Option<String>> {
        let cache = self.cache.read();
        let result = cache
            .as_table()
            .and_then(|t| t.get(&ns.category))
            .and_then(toml::Value::as_table)
            .and_then(|t| t.get(&ns.provider))
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    RokoError::invalid(format!(
                        "secret at {}.{} is not a string",
                        ns.category, ns.provider
                    ))
                })
            });
        drop(cache);
        match result {
            None => Ok(None),
            Some(Ok(s)) => Ok(Some(s)),
            Some(Err(e)) => Err(e),
        }
    }

    fn set(&self, ns: &Namespace, value: String) -> Result<()> {
        let mut cache = self.cache.write();
        // Ensure root is a table.
        if !cache.is_table() {
            *cache = toml::Value::Table(toml::value::Table::new());
        }
        let Some(root) = cache.as_table_mut() else {
            return Err(RokoError::invalid(
                "cache root is not a TOML table (invariant violated)",
            ));
        };
        let category_entry = root
            .entry(ns.category.clone())
            .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
        if !category_entry.is_table() {
            *category_entry = toml::Value::Table(toml::value::Table::new());
        }
        let Some(category_table) = category_entry.as_table_mut() else {
            return Err(RokoError::invalid(
                "category entry is not a TOML table (invariant violated)",
            ));
        };
        category_table.insert(ns.provider.clone(), toml::Value::String(value));
        let snapshot = cache.clone();
        drop(cache);
        self.persist(&snapshot)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "file"
    }
}

/// Verify the file has `0600` perms on unix. No-op on non-unix.
fn verify_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)?.permissions().mode() & 0o777;
        if mode != 0o600 {
            return Err(RokoError::invalid(format!(
                "file {:?} has unsafe perms {mode:o}; expected 0600",
                path.display()
            )));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Write `text` to `path` atomically (tempfile + rename) with `0600` perms on unix.
fn write_atomic_restricted(path: &Path, text: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let file_name = path
        .file_name()
        .ok_or_else(|| RokoError::invalid(format!("path {} has no file name", path.display())))?;
    let tmp_name = format!(".{}.tmp", file_name.to_string_lossy());
    let tmp_path = parent.join(tmp_name);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn file_store_creates_file_on_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        assert!(!path.exists());
        let _store = FileStore::open(&path).unwrap();
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn file_store_get_missing_returns_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let store = FileStore::open(&path).unwrap();
        let ns = Namespace::new("llm", "anthropic");
        assert_eq!(store.get(&ns).unwrap(), None);
    }

    #[test]
    fn file_store_set_then_get_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let store = FileStore::open(&path).unwrap();
        let ns = Namespace::new("llm", "anthropic");
        store.set(&ns, "sk-ant-xyz".into()).unwrap();
        assert_eq!(store.get(&ns).unwrap(), Some("sk-ant-xyz".into()));
    }

    #[test]
    fn file_store_set_persists_to_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        {
            let store = FileStore::open(&path).unwrap();
            store
                .set(&Namespace::new("rpc", "alchemy"), "alch-key".into())
                .unwrap();
        }
        // Reopen and verify.
        let store2 = FileStore::open(&path).unwrap();
        assert_eq!(
            store2.get(&Namespace::new("rpc", "alchemy")).unwrap(),
            Some("alch-key".into())
        );
    }

    #[test]
    #[cfg(unix)]
    fn file_store_rejects_world_readable_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        fs::write(&path, "").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&path, perms).unwrap();
        let err = FileStore::open(&path).unwrap_err();
        assert!(matches!(err, RokoError::Invalid(_)));
        assert!(format!("{err}").contains("unsafe perms"));
    }

    #[test]
    fn file_store_refresh_reloads_from_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let store = FileStore::open(&path).unwrap();
        let ns = Namespace::new("webhook", "github");
        assert_eq!(store.get(&ns).unwrap(), None);

        // Externally modify the file (simulating another process).
        let external = "[webhook]\ngithub = \"external-value\"\n";
        write_atomic_restricted(&path, external).unwrap();

        // Cache is stale until refresh.
        assert_eq!(store.get(&ns).unwrap(), None);
        store.refresh().unwrap();
        assert_eq!(store.get(&ns).unwrap(), Some("external-value".into()));
    }

    #[test]
    fn file_store_overwrites_existing_value() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let store = FileStore::open(&path).unwrap();
        let ns = Namespace::new("llm", "openai");
        store.set(&ns, "first".into()).unwrap();
        store.set(&ns, "second".into()).unwrap();
        assert_eq!(store.get(&ns).unwrap(), Some("second".into()));
    }

    #[test]
    fn file_store_name_is_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let store = FileStore::open(&path).unwrap();
        assert_eq!(store.name(), "file");
    }

    #[test]
    fn file_store_write_preserves_0600_perms() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let store = FileStore::open(&path).unwrap();
        store
            .set(&Namespace::new("llm", "cohere"), "cohere-key".into())
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn file_store_rejects_invalid_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        write_atomic_restricted(&path, "this is [not valid toml").unwrap();
        let err = FileStore::open(&path).unwrap_err();
        assert!(matches!(err, RokoError::Invalid(_)));
    }
}
