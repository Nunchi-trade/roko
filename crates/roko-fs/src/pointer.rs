//! Pointer store — disk-backed storage for large tool-result payloads.
//!
//! Stores pointer content under `.roko/runs/{run}/pointers/{id}`.
//! Content below [`PointerStore::max_inline_bytes`] is considered
//! "inline" and callers may choose to skip storage; content above that
//! threshold gets persisted to disk and referenced by pointer ID.
//!
//! All operations are synchronous (`std::fs`) because pointer reads
//! happen on the tool-loop hot path and should not yield. The store
//! is safe to use from async contexts via `spawn_blocking` if needed.

use std::io;
use std::path::{Path, PathBuf};

// ─── PointerStore ───────────────────────────────────────────────────────────

/// Manages pointer content on disk under a per-run directory.
///
/// Layout: `{root}/runs/{run_id}/pointers/{pointer_id}`
///
/// The store is stateless — it operates purely on the filesystem and
/// can be reconstructed from just the root path. Thread-safety is
/// delegated to the filesystem (one-writer semantics assumed).
#[derive(Debug, Clone)]
pub struct PointerStore {
    /// Root directory (typically `.roko/`).
    root: PathBuf,
    /// Payloads at or below this size are considered inline and need
    /// not be stored. Default: 4096 bytes.
    max_inline_bytes: usize,
}

impl PointerStore {
    /// Default inline threshold: 4 `KiB`.
    pub const DEFAULT_MAX_INLINE: usize = 4096;

    /// Construct a store rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_inline_bytes: Self::DEFAULT_MAX_INLINE,
        }
    }

    /// Construct a store with a custom inline threshold.
    #[must_use]
    pub fn with_max_inline(root: impl Into<PathBuf>, max_inline_bytes: usize) -> Self {
        Self {
            root: root.into(),
            max_inline_bytes,
        }
    }

    /// The inline-threshold in bytes.
    #[must_use]
    pub const fn max_inline_bytes(&self) -> usize {
        self.max_inline_bytes
    }

    /// Root directory of this store.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory for a given run's pointers.
    #[must_use]
    fn pointers_dir(&self, run_id: &str) -> PathBuf {
        self.root.join("runs").join(run_id).join("pointers")
    }

    /// Full path for a specific pointer.
    #[must_use]
    fn pointer_path(&self, run_id: &str, pointer_id: &str) -> PathBuf {
        self.pointers_dir(run_id).join(pointer_id)
    }

    /// Store content for a pointer.
    ///
    /// Creates the directory tree if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if directory creation or file write fails.
    pub fn store(&self, run_id: &str, pointer_id: &str, content: &[u8]) -> io::Result<()> {
        let dir = self.pointers_dir(run_id);
        std::fs::create_dir_all(&dir)?;
        let path = self.pointer_path(run_id, pointer_id);
        std::fs::write(&path, content)
    }

    /// Retrieve the full content of a pointer.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` if the pointer does not exist, or another I/O
    /// error on read failure.
    pub fn retrieve(&self, run_id: &str, pointer_id: &str) -> io::Result<Vec<u8>> {
        let path = self.pointer_path(run_id, pointer_id);
        std::fs::read(&path)
    }

    /// Check whether a pointer exists on disk.
    #[must_use]
    pub fn exists(&self, run_id: &str, pointer_id: &str) -> bool {
        self.pointer_path(run_id, pointer_id).is_file()
    }

    /// List all pointer IDs for a given run.
    ///
    /// Returns an empty vector if the run or pointers directory does not
    /// exist.
    ///
    /// # Errors
    ///
    /// Returns an I/O error only on unexpected read failures (not
    /// directory-not-found).
    pub fn list_pointers(&self, run_id: &str) -> io::Result<Vec<String>> {
        let dir = self.pointers_dir(run_id);
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                let mut ids = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    if entry.file_type()?.is_file() {
                        if let Some(name) = entry.file_name().to_str() {
                            ids.push(name.to_owned());
                        }
                    }
                }
                ids.sort();
                Ok(ids)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Delete a stored pointer.
    ///
    /// Returns `Ok(())` even if the pointer did not exist (idempotent).
    ///
    /// # Errors
    ///
    /// Returns an I/O error only on unexpected removal failures.
    pub fn delete(&self, run_id: &str, pointer_id: &str) -> io::Result<()> {
        let path = self.pointer_path(run_id, pointer_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Returns `true` if `content` is small enough to be inlined (no
    /// pointer storage needed).
    #[must_use]
    pub const fn should_inline(&self, content: &[u8]) -> bool {
        content.len() <= self.max_inline_bytes
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store(tmp: &TempDir) -> PointerStore {
        PointerStore::new(tmp.path())
    }

    #[test]
    fn store_and_retrieve_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        let content = b"hello world, this is a large tool result";
        store.store("run-1", "ptr-abc", content).unwrap();
        let retrieved = store.retrieve("run-1", "ptr-abc").unwrap();
        assert_eq!(retrieved, content);
    }

    #[test]
    fn retrieve_missing_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        let err = store.retrieve("run-1", "no-such-ptr").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn exists_reports_correctly() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        assert!(!store.exists("run-1", "ptr-1"));
        store.store("run-1", "ptr-1", b"data").unwrap();
        assert!(store.exists("run-1", "ptr-1"));
    }

    #[test]
    fn list_pointers_returns_sorted() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        store.store("run-1", "ptr-c", b"c").unwrap();
        store.store("run-1", "ptr-a", b"a").unwrap();
        store.store("run-1", "ptr-b", b"b").unwrap();
        let ids = store.list_pointers("run-1").unwrap();
        assert_eq!(ids, vec!["ptr-a", "ptr-b", "ptr-c"]);
    }

    #[test]
    fn list_pointers_missing_run_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        let ids = store.list_pointers("nonexistent-run").unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn delete_removes_pointer() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        store.store("run-1", "ptr-1", b"data").unwrap();
        assert!(store.exists("run-1", "ptr-1"));
        store.delete("run-1", "ptr-1").unwrap();
        assert!(!store.exists("run-1", "ptr-1"));
    }

    #[test]
    fn delete_idempotent_on_missing() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        // Deleting a non-existent pointer should not error.
        store.delete("run-1", "no-such").unwrap();
    }

    #[test]
    fn should_inline_threshold() {
        let store = PointerStore::with_max_inline("/tmp", 100);
        assert!(store.should_inline(&[0u8; 100]));
        assert!(store.should_inline(&[0u8; 50]));
        assert!(!store.should_inline(&[0u8; 101]));
    }

    #[test]
    fn store_large_content() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        // 50 KiB of data — well above the default 4 KiB threshold.
        let large = vec![0xABu8; 50 * 1024];
        store.store("run-2", "big-ptr", &large).unwrap();
        let retrieved = store.retrieve("run-2", "big-ptr").unwrap();
        assert_eq!(retrieved.len(), 50 * 1024);
        assert_eq!(retrieved, large);
    }

    #[test]
    fn separate_runs_are_isolated() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(&tmp);
        store.store("run-a", "ptr-1", b"alpha").unwrap();
        store.store("run-b", "ptr-1", b"beta").unwrap();

        assert_eq!(store.retrieve("run-a", "ptr-1").unwrap(), b"alpha");
        assert_eq!(store.retrieve("run-b", "ptr-1").unwrap(), b"beta");

        let ids_a = store.list_pointers("run-a").unwrap();
        let ids_b = store.list_pointers("run-b").unwrap();
        assert_eq!(ids_a, vec!["ptr-1"]);
        assert_eq!(ids_b, vec!["ptr-1"]);
    }

    #[test]
    fn default_max_inline_is_4k() {
        let store = PointerStore::new("/tmp");
        assert_eq!(store.max_inline_bytes(), 4096);
    }
}
