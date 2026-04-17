//! Hash-addressed immutable artifact store.
//!
//! [`ArtifactStore`] holds gate artifacts (build logs, test output, diff
//! snapshots) keyed by their BLAKE3 content hash. This gives us:
//!
//! - **Deduplication**: identical artifacts share storage
//! - **Addressability**: any subsystem can refer to an artifact by hash
//! - **Immutability**: once stored, content never changes
//!
//! The current implementation is in-memory (`HashMap`). A future version
//! may back this with disk or a content-addressed filesystem.

use roko_core::ContentHash;
use std::collections::HashMap;

/// An in-memory, hash-addressed artifact store.
///
/// Artifacts are stored as raw byte vectors, keyed by [`ContentHash`].
/// The store is append-only: there is no `delete` or `update` operation.
#[derive(Clone, Debug, Default)]
pub struct ArtifactStore {
    inner: HashMap<ContentHash, Vec<u8>>,
}

impl ArtifactStore {
    /// Create an empty artifact store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Store `content` and return its content hash.
    ///
    /// If the content already exists (same hash), this is a no-op and
    /// returns the existing hash. The store never holds duplicate data.
    pub fn store(&mut self, content: &[u8]) -> ContentHash {
        let hash = ContentHash::of(content);
        self.inner.entry(hash).or_insert_with(|| content.to_vec());
        hash
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

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        let hash = store.store(content);
        let retrieved = store
            .retrieve(&hash)
            .expect("invariant: stored artifact should be retrievable by its hash");
        assert_eq!(retrieved, content);
    }

    #[test]
    fn artifact_store_exists_and_missing() {
        let mut store = ArtifactStore::new();
        let hash = store.store(b"data");
        assert!(store.exists(&hash));

        let missing = ContentHash::of(b"not stored");
        assert!(!store.exists(&missing));
        assert!(store.retrieve(&missing).is_none());
    }

    #[test]
    fn artifact_store_deduplicates() {
        let mut store = ArtifactStore::new();
        let h1 = store.store(b"same content");
        let h2 = store.store(b"same content");
        assert_eq!(h1, h2);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn artifact_store_distinct_content_different_hashes() {
        let mut store = ArtifactStore::new();
        let h1 = store.store(b"alpha");
        let h2 = store.store(b"beta");
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
        let hash = store.store(b"");
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
        let hash = store.store(&big);
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
        let h1 = store.store(b"deterministic");
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
            hashes.push(store.store(content.as_bytes()));
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
}
