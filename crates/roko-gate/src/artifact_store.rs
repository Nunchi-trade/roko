//! Hash-addressed immutable artifact store.
//!
//! [`ArtifactStore`] holds gate artifacts (build logs, test output, diff
//! snapshots) keyed by their BLAKE3 content hash. This gives us:
//!
//! - **Deduplication**: identical artifacts share storage
//! - **Addressability**: any subsystem can refer to an artifact by hash
//! - **Immutability**: once stored, content never changes
//!
//! The store can run fully in-memory or be opened against a disk root. The
//! disk-backed mode uses a sharded content-addressed layout and eagerly loads
//! existing artifacts on startup so retrieval semantics stay identical.

use roko_core::ContentHash;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A hash-addressed artifact store.
///
/// Artifacts are stored as raw byte vectors, keyed by [`ContentHash`]. When a
/// root directory is configured, newly stored artifacts are also persisted to a
/// sharded on-disk content-addressed store and reloaded when reopened.
#[derive(Clone, Debug, Default)]
pub struct ArtifactStore {
    inner: HashMap<ContentHash, Vec<u8>>,
    root: Option<PathBuf>,
}

impl ArtifactStore {
    /// Create an empty in-memory artifact store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
            root: None,
        }
    }

    /// Open a disk-backed artifact store rooted at `root`.
    ///
    /// Existing artifacts are eagerly loaded so borrowed retrieval semantics
    /// remain the same as the in-memory variant.
    ///
    /// # Errors
    ///
    /// Returns any filesystem error while creating or scanning the store root,
    /// or [`io::ErrorKind::InvalidData`] if a persisted artifact's contents do
    /// not match its hash-derived path.
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        let inner = load_artifacts(&root)?;
        Ok(Self {
            inner,
            root: Some(root),
        })
    }

    /// Returns the configured disk root, if this store is persistent.
    #[must_use]
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Store `content` and return its content hash.
    ///
    /// If the content already exists (same hash), this is a no-op and
    /// returns the existing hash. The store never holds duplicate data.
    ///
    /// # Errors
    ///
    /// Returns any filesystem error while persisting to a disk-backed store.
    pub fn store(&mut self, content: &[u8]) -> io::Result<ContentHash> {
        let hash = ContentHash::of(content);
        if self.inner.contains_key(&hash) {
            return Ok(hash);
        }

        if let Some(root) = &self.root {
            persist_artifact(root, &hash, content)?;
        }

        self.inner.insert(hash, content.to_vec());
        Ok(hash)
    }

    /// Retrieve the artifact with the given hash, or `None` if absent.
    #[must_use]
    pub fn retrieve(&self, hash: &ContentHash) -> Option<&[u8]> {
        self.inner.get(hash).map(Vec::as_slice)
    }

    /// Returns `true` if an artifact with the given hash is stored.
    #[must_use]
    pub fn exists(&self, hash: &ContentHash) -> bool {
        self.inner.contains_key(hash)
    }

    /// Number of distinct artifacts currently stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the store contains no artifacts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

fn load_artifacts(root: &Path) -> io::Result<HashMap<ContentHash, Vec<u8>>> {
    let mut inner = HashMap::new();

    for shard in fs::read_dir(root)? {
        let shard = shard?;
        if !shard.file_type()?.is_dir() {
            continue;
        }

        let shard_name = shard.file_name();
        let shard_name = shard_name.to_string_lossy();
        if shard_name.len() != 2 || !is_hex_component(&shard_name) {
            continue;
        }

        for artifact in fs::read_dir(shard.path())? {
            let artifact = artifact?;
            if !artifact.file_type()?.is_file() {
                continue;
            }

            let file_name = artifact.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name.len() != 62 || !is_hex_component(&file_name) {
                continue;
            }

            let full_hex = format!("{shard_name}{file_name}");
            let hash = ContentHash::from_hex(&full_hex).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid artifact hash path {full_hex}"),
                )
            })?;
            let bytes = fs::read(artifact.path())?;
            let computed = ContentHash::of(&bytes);
            if computed != hash {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("artifact {full_hex} content hash mismatch"),
                ));
            }
            inner.insert(hash, bytes);
        }
    }

    Ok(inner)
}

fn persist_artifact(root: &Path, hash: &ContentHash, content: &[u8]) -> io::Result<()> {
    let path = artifact_path(root, hash);
    if path.exists() {
        return Ok(());
    }

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("artifact path {} has no parent", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let tmp = temporary_artifact_path(parent, hash);
    {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(content)?;
        file.sync_all()?;
    }

    match fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp);
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn artifact_path(root: &Path, hash: &ContentHash) -> PathBuf {
    let hex = hash.to_hex();
    let (shard, file) = hex.split_at(2);
    root.join(shard).join(file)
}

fn temporary_artifact_path(parent: &Path, hash: &ContentHash) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(
        ".{}.tmp-{}-{now}",
        hash.to_hex(),
        std::process::id()
    ))
}

fn is_hex_component(component: &str) -> bool {
    component.bytes().all(|byte| byte.is_ascii_hexdigit())
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn artifact_store_new_is_empty() {
        let store = ArtifactStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn artifact_store_store_and_retrieve() {
        let mut store = ArtifactStore::new();
        let content = b"hello world";
        let hash = store.store(content).expect("store artifact");
        let retrieved = store
            .retrieve(&hash)
            .expect("invariant: stored artifact should be retrievable by its hash");
        assert_eq!(retrieved, content);
    }

    #[test]
    fn artifact_store_exists_and_missing() {
        let mut store = ArtifactStore::new();
        let hash = store.store(b"data").expect("store artifact");
        assert!(store.exists(&hash));

        let missing = ContentHash::of(b"not stored");
        assert!(!store.exists(&missing));
        assert!(store.retrieve(&missing).is_none());
    }

    #[test]
    fn artifact_store_deduplicates() {
        let mut store = ArtifactStore::new();
        let h1 = store.store(b"same content").expect("store first artifact");
        let h2 = store
            .store(b"same content")
            .expect("store duplicate artifact");
        assert_eq!(h1, h2);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn artifact_store_distinct_content_different_hashes() {
        let mut store = ArtifactStore::new();
        let h1 = store.store(b"alpha").expect("store alpha");
        let h2 = store.store(b"beta").expect("store beta");
        assert_ne!(h1, h2);
        assert_eq!(store.len(), 2);
        assert_eq!(
            store
                .retrieve(&h1)
                .expect("invariant: first stored artifact should exist"),
            b"alpha"
        );
        assert_eq!(
            store
                .retrieve(&h2)
                .expect("invariant: second stored artifact should exist"),
            b"beta"
        );
    }

    #[test]
    fn artifact_store_empty_content() {
        let mut store = ArtifactStore::new();
        let hash = store.store(b"").expect("store empty artifact");
        assert!(store.exists(&hash));
        assert_eq!(
            store
                .retrieve(&hash)
                .expect("invariant: empty artifact should be retrievable"),
            b""
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn artifact_store_large_content() {
        let mut store = ArtifactStore::new();
        let big = vec![0xAB_u8; 1_000_000];
        let hash = store.store(&big).expect("store large artifact");
        let retrieved = store
            .retrieve(&hash)
            .expect("invariant: large stored artifact should be retrievable");
        assert_eq!(retrieved.len(), 1_000_000);
        assert!(retrieved.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn artifact_store_default_is_empty() {
        let store = ArtifactStore::default();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn artifact_store_hash_deterministic() {
        let mut store = ArtifactStore::new();
        let h1 = store
            .store(b"deterministic")
            .expect("store deterministic artifact");
        // Compute hash independently
        let expected = ContentHash::of(b"deterministic");
        assert_eq!(h1, expected);
    }

    #[test]
    fn artifact_store_multiple_inserts_and_lookups() {
        let mut store = ArtifactStore::new();
        let mut hashes = Vec::new();
        for i in 0..50 {
            let content = format!("artifact-{i}");
            hashes.push(
                store
                    .store(content.as_bytes())
                    .expect("store artifact during sweep"),
            );
        }
        assert_eq!(store.len(), 50);
        for (i, hash) in hashes.iter().enumerate() {
            let expected = format!("artifact-{i}");
            assert_eq!(
                store
                    .retrieve(hash)
                    .expect("invariant: stored artifact should exist during lookup sweep"),
                expected.as_bytes()
            );
        }
    }

    #[test]
    fn artifact_store_open_empty_root_is_empty() {
        let dir = tempdir().expect("create temp dir");
        let store = ArtifactStore::open(dir.path()).expect("open persistent store");
        assert!(store.is_empty());
        assert_eq!(store.root(), Some(dir.path()));
    }

    #[test]
    fn artifact_store_persists_across_reopen() {
        let dir = tempdir().expect("create temp dir");
        let hash = {
            let mut store = ArtifactStore::open(dir.path()).expect("open persistent store");
            store
                .store(b"persist me")
                .expect("store persisted artifact")
        };

        let reopened = ArtifactStore::open(dir.path()).expect("reopen persistent store");
        assert_eq!(reopened.len(), 1);
        assert!(reopened.exists(&hash));
        assert_eq!(
            reopened
                .retrieve(&hash)
                .expect("persisted artifact should be available after reopen"),
            b"persist me"
        );
    }

    #[test]
    fn artifact_store_disk_mode_deduplicates_existing_content() {
        let dir = tempdir().expect("create temp dir");

        let original = {
            let mut store = ArtifactStore::open(dir.path()).expect("open persistent store");
            store
                .store(b"same content")
                .expect("store initial artifact")
        };

        let mut reopened = ArtifactStore::open(dir.path()).expect("reopen persistent store");
        let duplicate = reopened
            .store(b"same content")
            .expect("store duplicate artifact");

        assert_eq!(original, duplicate);
        assert_eq!(reopened.len(), 1);

        let shard_count = fs::read_dir(dir.path())
            .expect("read root")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .count();
        assert_eq!(shard_count, 1);
    }

    #[test]
    fn artifact_store_open_rejects_corrupt_artifacts() {
        let dir = tempdir().expect("create temp dir");
        let hash = {
            let mut store = ArtifactStore::open(dir.path()).expect("open persistent store");
            store
                .store(b"trusted bytes")
                .expect("store persisted artifact")
        };

        let path = artifact_path(dir.path(), &hash);
        fs::write(&path, b"tampered bytes").expect("tamper artifact file");

        let err = ArtifactStore::open(dir.path()).expect_err("corruption should be detected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
