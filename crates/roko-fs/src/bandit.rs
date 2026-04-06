//! Bandit arm persistence — append-only JSONL store under `.roko/bandit/`.
//!
//! Each bandit key maps to a file `.roko/bandit/{key}.jsonl`. Every call
//! to [`BanditStore::save_arms`] appends one [`ArmSnapshot`] line — the
//! full arm table at that instant. On reload, [`BanditStore::load_arms`]
//! returns the most recent snapshot's entries (or an empty vec if no data
//! exists yet).
//!
//! The append-only format is crash-safe: a partial last line is silently
//! skipped on replay, and the store never reads its own writes within the
//! same call.

use std::io;
use std::path::{Path, PathBuf};

use roko_core::tool::bandit::ArmEntry;
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ─── ArmSnapshot ────────────────────────────────────────────────────────────

/// One snapshot line persisted to the JSONL file.
///
/// Contains the full arm table at one point in time, plus a timestamp so
/// the most recent snapshot can be identified on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmSnapshot {
    /// The bandit key this snapshot belongs to (echoed for grep-ability).
    pub key: String,
    /// The full arm table at this instant.
    pub arms: Vec<ArmEntry>,
    /// Milliseconds since the Unix epoch when this snapshot was taken.
    pub timestamp_ms: i64,
}

// ─── BanditStore ────────────────────────────────────────────────────────────

/// Manages the `.roko/bandit/` directory for arm persistence.
///
/// Each distinct bandit key maps to its own `.jsonl` file. All writes are
/// appends; reads replay the file and return the last valid snapshot.
#[derive(Debug, Clone)]
pub struct BanditStore {
    /// Root directory (`.roko/bandit/`).
    root: PathBuf,
}

impl BanditStore {
    /// Construct a store rooted at `dir` (typically `.roko/bandit/`).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { root: dir.into() }
    }

    /// The path for a given key's JSONL file.
    ///
    /// Keys are sanitized: any character that is not alphanumeric, dash,
    /// or underscore is replaced with `_` to keep filenames portable.
    #[must_use]
    pub fn path_for(&self, key: &str) -> PathBuf {
        let safe: String = key
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        self.root.join(format!("{safe}.jsonl"))
    }

    /// Root directory of this store.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Append the current arm table as a snapshot line.
    ///
    /// Creates the directory and file if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if directory creation or file write fails.
    pub async fn save_arms(&self, key: &str, arms: &[ArmEntry]) -> io::Result<()> {
        fs::create_dir_all(&self.root).await?;
        let path = self.path_for(key);
        let snapshot = ArmSnapshot {
            key: key.to_owned(),
            arms: arms.to_vec(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        };
        let mut line = serde_json::to_string(&snapshot)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    /// Load the most recent snapshot's arm entries for `key`.
    ///
    /// Returns an empty vector if the file does not exist or contains no
    /// valid snapshots. Partial / corrupt lines are silently skipped.
    ///
    /// # Errors
    ///
    /// Returns an I/O error only on unexpected read failures (not
    /// file-not-found).
    pub async fn load_arms(&self, key: &str) -> io::Result<Vec<ArmEntry>> {
        let path = self.path_for(key);
        let file = match fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut latest: Option<ArmSnapshot> = None;
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(snap) = serde_json::from_str::<ArmSnapshot>(&line) {
                latest = Some(snap);
            }
            // Corrupt / partial lines are silently skipped.
        }
        Ok(latest.map_or_else(Vec::new, |s| s.arms))
    }

    /// Load all snapshots for `key` in chronological order.
    ///
    /// Useful for diagnostics and replay. Corrupt lines are skipped.
    ///
    /// # Errors
    ///
    /// Returns an I/O error only on unexpected read failures.
    pub async fn load_all_snapshots(&self, key: &str) -> io::Result<Vec<ArmSnapshot>> {
        let path = self.path_for(key);
        let file = match fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut snapshots = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(snap) = serde_json::from_str::<ArmSnapshot>(&line) {
                snapshots.push(snap);
            }
        }
        Ok(snapshots)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::format::ToolFormat;
    use tempfile::TempDir;

    fn sample_arms() -> Vec<ArmEntry> {
        vec![
            ArmEntry {
                format: ToolFormat::HermesJson,
                pulls: 10,
                cumulative_reward: 8.5,
                last_pulled_ms: 1_700_000_000_000,
                consecutive_failures: 0,
            },
            ArmEntry {
                format: ToolFormat::ReActText,
                pulls: 3,
                cumulative_reward: 1.2,
                last_pulled_ms: 1_700_000_001_000,
                consecutive_failures: 1,
            },
        ]
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = BanditStore::new(tmp.path().join("bandit"));

        let arms = sample_arms();
        store.save_arms("test_key", &arms).await.unwrap();
        let loaded = store.load_arms("test_key").await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].format, ToolFormat::HermesJson);
        assert_eq!(loaded[0].pulls, 10);
        assert_eq!(loaded[1].format, ToolFormat::ReActText);
    }

    #[tokio::test]
    async fn load_missing_key_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let store = BanditStore::new(tmp.path().join("bandit"));
        let loaded = store.load_arms("nonexistent").await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn append_only_keeps_history() {
        let tmp = TempDir::new().unwrap();
        let store = BanditStore::new(tmp.path().join("bandit"));

        let arms_v1 = vec![ArmEntry::new(ToolFormat::HermesJson)];
        store.save_arms("k", &arms_v1).await.unwrap();

        let arms_v2 = sample_arms();
        store.save_arms("k", &arms_v2).await.unwrap();

        // load_arms returns the latest snapshot.
        let loaded = store.load_arms("k").await.unwrap();
        assert_eq!(loaded.len(), 2);

        // load_all_snapshots returns both.
        let all = store.load_all_snapshots("k").await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].arms.len(), 1);
        assert_eq!(all[1].arms.len(), 2);
    }

    #[tokio::test]
    async fn corrupt_lines_are_skipped() {
        let tmp = TempDir::new().unwrap();
        let store = BanditStore::new(tmp.path().join("bandit"));

        // Write a valid snapshot, then a corrupt line, then another valid one.
        let arms = sample_arms();
        store.save_arms("c", &arms).await.unwrap();

        let path = store.path_for("c");
        let mut file = OpenOptions::new().append(true).open(&path).await.unwrap();
        file.write_all(b"THIS IS NOT JSON\n").await.unwrap();
        file.flush().await.unwrap();

        let arms2 = vec![ArmEntry::new(ToolFormat::JsonMode)];
        store.save_arms("c", &arms2).await.unwrap();

        // Should get the last valid snapshot (arms2).
        let loaded = store.load_arms("c").await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].format, ToolFormat::JsonMode);

        // All snapshots should be 2 (corrupt line skipped).
        let all = store.load_all_snapshots("c").await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn path_for_sanitizes_key() {
        let store = BanditStore::new("/tmp/bandit");
        let path = store.path_for("model/role:bucket");
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(filename, "model_role_bucket.jsonl");
        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));
    }

    #[tokio::test]
    async fn snapshot_serde_roundtrip() {
        let snap = ArmSnapshot {
            key: "test".to_owned(),
            arms: sample_arms(),
            timestamp_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let decoded: ArmSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, snap);
    }

    #[tokio::test]
    async fn multiple_keys_use_separate_files() {
        let tmp = TempDir::new().unwrap();
        let store = BanditStore::new(tmp.path().join("bandit"));

        let arms_a = vec![ArmEntry::new(ToolFormat::HermesJson)];
        let arms_b = sample_arms();
        store.save_arms("key_a", &arms_a).await.unwrap();
        store.save_arms("key_b", &arms_b).await.unwrap();

        let loaded_a = store.load_arms("key_a").await.unwrap();
        let loaded_b = store.load_arms("key_b").await.unwrap();
        assert_eq!(loaded_a.len(), 1);
        assert_eq!(loaded_b.len(), 2);
    }
}
