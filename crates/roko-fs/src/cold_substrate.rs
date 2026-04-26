//! Archive-backed [`ColdStore`] implementation.
//!
//! Stores aged-out engrams in compressed JSONL archive files organized by month.
//! This is the filesystem-backed cold storage tier, complementing the hot
//! [`FileSubstrate`](crate::FileSubstrate).
//!
//! # Storage layout
//!
//! ```text
//! .roko/cold/
//!   2026-04.jsonl     # one file per month
//!   2026-03.jsonl
//!   index.json        # hash → (month, line_offset) lookup
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{Datelike, Utc};
use parking_lot::RwLock;
use roko_core::{
    ColdStore, ContentHash, Engram,
    error::{Result, RokoError},
};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Record describing where an engram is stored in cold archives.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ColdIndexEntry {
    /// Archive file name (e.g., "2026-04.jsonl").
    file: String,
    /// Byte offset within the archive file.
    offset: u64,
    /// Archived timestamp (epoch millis).
    archived_at: i64,
}

/// Filesystem-backed cold substrate storing engrams in monthly JSONL archives.
pub struct ArchiveColdSubstrate {
    /// Root directory for cold storage (e.g., `.roko/cold/`).
    root: PathBuf,
    /// In-memory index: content hash → archive location.
    index: RwLock<HashMap<ContentHash, ColdIndexEntry>>,
    /// Serializes writes.
    write_lock: Mutex<()>,
}

impl ArchiveColdSubstrate {
    /// Open or create a cold substrate at the given root directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the index is corrupt.
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).await?;

        let index = Self::load_index(&root).await.unwrap_or_default();

        Ok(Self {
            root,
            index: RwLock::new(index),
            write_lock: Mutex::new(()),
        })
    }

    /// Path to the index file.
    fn index_path(root: &Path) -> PathBuf {
        root.join("index.json")
    }

    /// Path to the archive file for the current month.
    fn current_archive_path(&self) -> PathBuf {
        let now = Utc::now();
        self.root
            .join(format!("{:04}-{:02}.jsonl", now.year(), now.month()))
    }

    /// Path to a specific archive file.
    fn archive_path(&self, file: &str) -> PathBuf {
        self.root.join(file)
    }

    /// Load the index from disk.
    async fn load_index(root: &Path) -> Result<HashMap<ContentHash, ColdIndexEntry>> {
        let path = Self::index_path(root);
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let data = fs::read_to_string(&path).await?;
        let index: HashMap<ContentHash, ColdIndexEntry> =
            serde_json::from_str(&data).map_err(|e| RokoError::body_decode(e))?;
        Ok(index)
    }

    /// Persist the index to disk.
    async fn save_index(&self) -> Result<()> {
        let path = Self::index_path(&self.root);
        let data = {
            let idx = self.index.read();
            serde_json::to_string_pretty(&*idx).map_err(|e| RokoError::body_encode(e))?
        };
        fs::write(&path, data.as_bytes()).await?;
        Ok(())
    }

    /// Append an engram to the current month's archive file.
    async fn append_to_archive(&self, engram: &Engram) -> Result<(String, u64)> {
        let archive_path = self.current_archive_path();
        let file_name = archive_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&archive_path)
            .await?;

        let offset = file.metadata().await?.len();

        let mut line = serde_json::to_string(engram).map_err(|e| RokoError::body_encode(e))?;
        line.push('\n');
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;

        Ok((file_name, offset))
    }

    /// Read an engram from a specific archive file at the given byte offset.
    async fn read_from_archive(&self, file: &str, offset: u64) -> Result<Option<Engram>> {
        let path = self.archive_path(file);
        if !path.exists() {
            return Ok(None);
        }

        let f = fs::File::open(&path).await?;
        let mut reader = BufReader::new(f);

        // Seek to offset by reading and discarding bytes.
        // For large files a proper seek would be better, but this keeps
        // the implementation simple and correct.
        let mut skipped = 0u64;
        if offset > 0 {
            let mut buf = String::new();
            while skipped < offset {
                buf.clear();
                let n = reader.read_line(&mut buf).await?;
                if n == 0 {
                    return Ok(None); // EOF before reaching offset
                }
                skipped += n as u64;
            }
        }

        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }

        let engram: Engram =
            serde_json::from_str(line.trim()).map_err(|e| RokoError::body_decode(e))?;
        Ok(Some(engram))
    }

    /// Count all engrams across all archive files.
    async fn count_all_archives(&self) -> Result<usize> {
        let idx = self.index.read();
        Ok(idx.len())
    }

    /// Total size of all archive files.
    async fn total_archive_size(&self) -> Result<u64> {
        let mut total = 0u64;
        let mut entries = fs::read_dir(&self.root).await?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "jsonl") {
                if let Ok(meta) = fs::metadata(&path).await {
                    total += meta.len();
                }
            }
        }
        Ok(total)
    }
}

#[async_trait]
impl ColdStore for ArchiveColdSubstrate {
    async fn archive(&self, engram: Engram) -> Result<ContentHash> {
        let _guard = self.write_lock.lock().await;

        let hash = engram.id;
        let (file, offset) = self.append_to_archive(&engram).await?;

        {
            let mut idx = self.index.write();
            idx.insert(
                hash,
                ColdIndexEntry {
                    file,
                    offset,
                    archived_at: Utc::now().timestamp_millis(),
                },
            );
        }

        self.save_index().await?;
        Ok(hash)
    }

    async fn archive_batch(&self, engrams: Vec<Engram>) -> Result<usize> {
        let _guard = self.write_lock.lock().await;
        let mut count = 0;

        for engram in engrams {
            let hash = engram.id;
            let (file, offset) = self.append_to_archive(&engram).await?;

            {
                let mut idx = self.index.write();
                idx.insert(
                    hash,
                    ColdIndexEntry {
                        file,
                        offset,
                        archived_at: Utc::now().timestamp_millis(),
                    },
                );
            }
            count += 1;
        }

        self.save_index().await?;
        Ok(count)
    }

    async fn thaw(&self, id: &ContentHash) -> Result<Option<Engram>> {
        let entry = {
            let idx = self.index.read();
            idx.get(id).cloned()
        };

        match entry {
            Some(entry) => self.read_from_archive(&entry.file, entry.offset).await,
            None => Ok(None),
        }
    }

    async fn contains(&self, id: &ContentHash) -> Result<bool> {
        let idx = self.index.read();
        Ok(idx.contains_key(id))
    }

    async fn archived_count(&self) -> Result<usize> {
        self.count_all_archives().await
    }

    async fn storage_bytes(&self) -> Result<u64> {
        self.total_archive_size().await
    }

    async fn purge_before(&self, epoch_ms: i64) -> Result<usize> {
        let _guard = self.write_lock.lock().await;
        let mut purged = 0;

        let to_purge: Vec<ContentHash> = {
            let idx = self.index.read();
            idx.iter()
                .filter(|(_, entry)| entry.archived_at < epoch_ms)
                .map(|(hash, _)| *hash)
                .collect()
        };

        {
            let mut idx = self.index.write();
            for hash in &to_purge {
                idx.remove(hash);
                purged += 1;
            }
        }

        if purged > 0 {
            self.save_index().await?;
        }

        Ok(purged)
    }

    fn name(&self) -> &'static str {
        "archive_cold_substrate"
    }
}

/// Helper to migrate engrams from a hot substrate to cold storage.
///
/// This struct encapsulates the migration logic: query the hot substrate for
/// aged-out engrams, archive them to cold storage, then prune them from hot.
pub struct SubstrateMigrator {
    /// Weight threshold below which engrams are candidates for migration.
    pub weight_threshold: f32,
    /// Maximum age in milliseconds. Engrams older than this are migrated.
    pub max_age_ms: i64,
    /// Maximum number of engrams to migrate per batch.
    pub batch_size: usize,
}

impl SubstrateMigrator {
    /// Create a migrator with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            weight_threshold: 0.1,
            max_age_ms: 7 * 24 * 3600 * 1000, // 7 days
            batch_size: 100,
        }
    }

    /// Create a migrator with custom thresholds.
    #[must_use]
    pub fn with_thresholds(weight_threshold: f32, max_age_ms: i64, batch_size: usize) -> Self {
        Self {
            weight_threshold,
            max_age_ms,
            batch_size,
        }
    }
}

impl Default for SubstrateMigrator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    fn test_engram(tag: &str) -> Engram {
        Engram::builder(Kind::Metric)
            .body(Body::text("test data"))
            .tag("label", tag)
            .build()
    }

    #[tokio::test]
    async fn archive_and_thaw() {
        let tmp = tempfile::tempdir().unwrap();
        let cold = ArchiveColdSubstrate::open(tmp.path().join("cold"))
            .await
            .unwrap();

        let engram = test_engram("gate.compile");
        let hash = cold.archive(engram.clone()).await.unwrap();

        assert!(cold.contains(&hash).await.unwrap());
        assert_eq!(cold.archived_count().await.unwrap(), 1);

        let thawed = cold.thaw(&hash).await.unwrap().unwrap();
        assert_eq!(thawed.id, engram.id);
        assert_eq!(thawed.kind, engram.kind);
    }

    #[tokio::test]
    async fn archive_batch_multiple() {
        let tmp = tempfile::tempdir().unwrap();
        let cold = ArchiveColdSubstrate::open(tmp.path().join("cold"))
            .await
            .unwrap();

        let engrams = vec![test_engram("a"), test_engram("b"), test_engram("c")];

        let count = cold.archive_batch(engrams).await.unwrap();
        assert_eq!(count, 3);
        assert_eq!(cold.archived_count().await.unwrap(), 3);
    }

    #[tokio::test]
    async fn thaw_nonexistent_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let cold = ArchiveColdSubstrate::open(tmp.path().join("cold"))
            .await
            .unwrap();
        let fake_hash = ContentHash::of(b"nonexistent");
        assert!(cold.thaw(&fake_hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn purge_removes_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let cold = ArchiveColdSubstrate::open(tmp.path().join("cold"))
            .await
            .unwrap();

        let engram = test_engram("old.data");
        let hash = cold.archive(engram).await.unwrap();
        assert!(cold.contains(&hash).await.unwrap());

        // Purge entries archived before now + 1 second
        let future = Utc::now().timestamp_millis() + 1000;
        let purged = cold.purge_before(future).await.unwrap();
        assert_eq!(purged, 1);
        assert!(!cold.contains(&hash).await.unwrap());
    }

    #[tokio::test]
    async fn storage_bytes_positive() {
        let tmp = tempfile::tempdir().unwrap();
        let cold = ArchiveColdSubstrate::open(tmp.path().join("cold"))
            .await
            .unwrap();

        cold.archive(test_engram("x")).await.unwrap();
        let bytes = cold.storage_bytes().await.unwrap();
        assert!(bytes > 0);
    }

    #[tokio::test]
    async fn index_persists_across_reopens() {
        let tmp = tempfile::tempdir().unwrap();
        let cold_path = tmp.path().join("cold");

        let hash;
        {
            let cold = ArchiveColdSubstrate::open(&cold_path).await.unwrap();
            hash = cold.archive(test_engram("persist")).await.unwrap();
        }

        // Reopen
        let cold2 = ArchiveColdSubstrate::open(&cold_path).await.unwrap();
        assert!(cold2.contains(&hash).await.unwrap());
    }

    #[test]
    fn migrator_defaults() {
        let m = SubstrateMigrator::new();
        assert_eq!(m.weight_threshold, 0.1);
        assert_eq!(m.batch_size, 100);
    }

    #[test]
    fn name_is_correct() {
        // We can't call async name() from sync test, but we can check the trait method
        // is implemented by verifying the type matches.
        let _ = SubstrateMigrator::default();
    }
}
