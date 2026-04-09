//! Context pack cache — memoized composed prompts keyed by task fingerprint.
//!
//! Building the "context pack" that wraps every agent spawn (workspace map,
//! prd2 extract, plan content, playbook hits, and research prepass) is
//! expensive. When two spawns share the same inputs, recomposition wastes
//! 500ms–2s per attempt. [`ContextPackCache`] memoizes the composed pack in
//! memory with an LRU eviction policy and writes a JSON snapshot to disk so
//! restarts warm quickly.
//!
//! # Design
//!
//! The cache is keyed by a **fingerprint string** supplied by the caller.
//! Callers are expected to derive that fingerprint deterministically from the
//! inputs that influenced composition (scope files, tags, mtimes, playbook
//! ids, etc.) — see `roko-compose` for the canonical recipe.
//!
//! Each [`ContextPack`] carries the composed markdown, the list of
//! [`ContentHash`] signals that were folded into the prompt, a token count,
//! and access stats. Per the user-supplied design the cache tracks
//! `created_at_ms`, `access_count`, and `last_access_ms` so downstream tools
//! (dashboards, dream-consolidation) can reason about warm vs cold entries.
//!
//! The API is `async` and uses `tokio::fs` for persistence. All in-memory
//! state lives behind a single [`parking_lot::Mutex`] so that `get`/`put` are
//! cheap and cannot deadlock.
//!
//! # Example
//!
//! ```no_run
//! use roko_learn::context_pack_cache::{ContextPack, ContextPackCache};
//! use std::path::PathBuf;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let cache = ContextPackCache::new(8, PathBuf::from(".roko/memory/context-packs.json"));
//! let pack = ContextPack::new(
//!     "plan-42::implementer".into(),
//!     "# Context\n...".into(),
//!     vec![],
//!     128,
//! );
//! cache.put("plan-42::implementer".into(), pack);
//! let hit = cache.get("plan-42::implementer");
//! assert!(hit.is_some());
//! cache.persist().await?;
//! # Ok(()) }
//! ```

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use chrono::Utc;
use parking_lot::Mutex;
use roko_core::{ContentHash, Result, RokoError};
use serde::{Deserialize, Serialize};

/// One cached, composed prompt pack plus its access metadata.
///
/// Returned from [`ContextPackCache::get`]; written by
/// [`ContextPackCache::put`]. The struct is `Serialize`/`Deserialize` so the
/// whole cache can be snapshotted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPack {
    /// Caller-supplied fingerprint. Two packs with identical fingerprints are
    /// assumed interchangeable.
    pub fingerprint: String,
    /// The assembled prompt text.
    pub composed_text: String,
    /// Content hashes of the signals folded into `composed_text`. Preserved
    /// so later stages (audit, provenance) can trace what went into a spawn.
    pub composed_signals: Vec<ContentHash>,
    /// Approximate token count of `composed_text`. Caller-supplied because
    /// tokenization is model-specific.
    pub token_count: usize,
    /// Unix-millisecond timestamp at which this pack was first stored.
    pub created_at_ms: i64,
    /// How many times this pack has been returned from [`ContextPackCache::get`].
    pub access_count: u64,
    /// Unix-millisecond timestamp of the last `get` or `put` that touched
    /// this entry.
    pub last_access_ms: i64,
}

impl ContextPack {
    /// Build a fresh pack with `access_count = 0` and timestamps set to now.
    ///
    /// Most callers will construct packs this way, then hand ownership to
    /// [`ContextPackCache::put`].
    #[must_use]
    pub fn new(
        fingerprint: String,
        composed_text: String,
        composed_signals: Vec<ContentHash>,
        token_count: usize,
    ) -> Self {
        let now = Utc::now().timestamp_millis();
        Self {
            fingerprint,
            composed_text,
            composed_signals,
            token_count,
            created_at_ms: now,
            access_count: 0,
            last_access_ms: now,
        }
    }
}

/// Snapshot of cache statistics returned by [`ContextPackCache::stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Number of entries currently held in memory.
    pub entries: usize,
    /// Maximum number of entries before LRU eviction kicks in.
    pub capacity: usize,
    /// Number of [`ContextPackCache::get`] calls that returned `Some`.
    pub hits: u64,
    /// Number of [`ContextPackCache::get`] calls that returned `None`.
    pub misses: u64,
    /// Number of entries dropped via LRU eviction.
    pub evictions: u64,
}

/// Serializable on-disk representation of the cache.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DiskSnapshot {
    capacity: usize,
    packs: Vec<ContextPack>,
    order: Vec<String>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

/// Internal state protected by a single mutex.
#[derive(Debug)]
struct Inner {
    packs: HashMap<String, ContextPack>,
    /// Access order from oldest (front) to newest (back). Used for LRU.
    order: VecDeque<String>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl Inner {
    fn new() -> Self {
        Self {
            packs: HashMap::new(),
            order: VecDeque::new(),
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    fn touch(&mut self, fingerprint: &str) {
        if let Some(pos) = self.order.iter().position(|s| s == fingerprint) {
            // `remove` on VecDeque returns Option but we just found the index.
            if let Some(key) = self.order.remove(pos) {
                self.order.push_back(key);
            }
        }
    }
}

/// A capacity-bounded, persistable cache of composed context packs.
///
/// Cheap to clone — the underlying storage is shared through an `Arc`-less
/// design where every method takes `&self` and locks briefly. Cloning gives
/// you a distinct cache; typically you hold one per process.
#[derive(Debug)]
pub struct ContextPackCache {
    capacity: usize,
    path: PathBuf,
    inner: Mutex<Inner>,
}

impl ContextPackCache {
    /// Create an empty cache bounded by `capacity` entries. `path` is where
    /// [`Self::persist`] writes and [`Self::load`] reads; the file is not
    /// touched during construction.
    ///
    /// A `capacity` of 0 disables the in-memory tier: every `put` is a no-op
    /// and every `get` returns `None`. This matches the behaviour documented
    /// in the compose spec.
    #[must_use]
    pub fn new(capacity: usize, path: impl Into<PathBuf>) -> Self {
        Self {
            capacity,
            path: path.into(),
            inner: Mutex::new(Inner::new()),
        }
    }

    /// Configured LRU capacity.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Path that [`Self::persist`] writes to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Current number of in-memory entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().packs.len()
    }

    /// Whether the in-memory tier is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().packs.is_empty()
    }

    /// Snapshot of current cache statistics.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock();
        CacheStats {
            entries: inner.packs.len(),
            capacity: self.capacity,
            hits: inner.hits,
            misses: inner.misses,
            evictions: inner.evictions,
        }
    }

    /// Insert `pack` under `fingerprint`, evicting the LRU entry if we would
    /// exceed capacity. If an entry already exists under this key it is
    /// replaced and its access stats are reset to match `pack`.
    ///
    /// When `capacity == 0` this is a no-op.
    pub fn put(&self, fingerprint: String, mut pack: ContextPack) {
        if self.capacity == 0 {
            return;
        }
        let now = Utc::now().timestamp_millis();
        pack.fingerprint.clone_from(&fingerprint);
        pack.last_access_ms = now;

        let mut inner = self.inner.lock();
        if inner.packs.contains_key(&fingerprint) {
            inner.packs.insert(fingerprint.clone(), pack);
            inner.touch(&fingerprint);
            return;
        }
        while inner.packs.len() >= self.capacity {
            // Evict LRU (front of the order deque).
            if let Some(victim) = inner.order.pop_front() {
                inner.packs.remove(&victim);
                inner.evictions += 1;
            } else {
                break;
            }
        }
        inner.packs.insert(fingerprint.clone(), pack);
        inner.order.push_back(fingerprint);
    }

    /// Look up a pack by fingerprint. On hit the entry's `access_count` is
    /// incremented, `last_access_ms` advanced, and the entry moved to the
    /// MRU position. The returned value is a *clone* with the updated stats.
    pub fn get(&self, fingerprint: &str) -> Option<ContextPack> {
        let mut inner = self.inner.lock();
        let snapshot = {
            let Some(pack) = inner.packs.get_mut(fingerprint) else {
                inner.misses = inner.misses.saturating_add(1);
                return None;
            };
            pack.access_count = pack.access_count.saturating_add(1);
            pack.last_access_ms = Utc::now().timestamp_millis();
            pack.clone()
        };
        inner.hits = inner.hits.saturating_add(1);
        inner.touch(fingerprint);
        drop(inner);
        Some(snapshot)
    }

    /// Evict the least-recently-used entry, if any. Returns the fingerprint
    /// of the dropped entry. Usually callers rely on automatic eviction
    /// during `put`; this is exposed for tests and for aggressive memory
    /// reclamation.
    pub fn evict_lru(&self) -> Option<String> {
        let mut inner = self.inner.lock();
        let victim = inner.order.pop_front()?;
        inner.packs.remove(&victim);
        inner.evictions = inner.evictions.saturating_add(1);
        drop(inner);
        Some(victim)
    }

    /// Drop every entry in memory. Counters (`hits`, `misses`, `evictions`)
    /// are left intact so observers can see lifetime totals.
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.packs.clear();
        inner.order.clear();
    }

    /// Serialize the current cache to `self.path` as JSON. Creates parent
    /// directories if needed. The write is performed atomically via a
    /// tempfile + rename so a crash mid-write cannot corrupt a previous
    /// snapshot. Intended for best-effort warm-up snapshots — callers
    /// should not rely on this for strict durability guarantees.
    pub async fn persist(&self) -> Result<()> {
        let snapshot = {
            let inner = self.inner.lock();
            DiskSnapshot {
                capacity: self.capacity,
                packs: inner
                    .order
                    .iter()
                    .filter_map(|fp| inner.packs.get(fp).cloned())
                    .collect(),
                order: inner.order.iter().cloned().collect(),
                hits: inner.hits,
                misses: inner.misses,
                evictions: inner.evictions,
            }
        };
        let encoded = serde_json::to_vec_pretty(&snapshot).map_err(RokoError::from)?;
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(RokoError::from)?;
            }
        }
        // Atomic write: stage in a sibling tempfile then rename into place.
        let tmp = self.path.file_name().map_or_else(
            || self.path.with_extension("tmp"),
            |name| {
                let mut stem = name.to_os_string();
                stem.push(".tmp");
                self.path.with_file_name(stem)
            },
        );
        tokio::fs::write(&tmp, &encoded)
            .await
            .map_err(RokoError::from)?;
        tokio::fs::rename(&tmp, &self.path)
            .await
            .map_err(RokoError::from)?;
        Ok(())
    }

    /// Load a cache from a JSON snapshot on disk. The returned cache uses
    /// the capacity stored in the snapshot (falling back to `fallback_capacity`
    /// if the snapshot records `0`).
    ///
    /// A missing file yields an empty cache — this is the "cold start" case
    /// and is not an error. Malformed JSON is surfaced as
    /// [`RokoError::Json`].
    pub async fn load(path: impl Into<PathBuf>, fallback_capacity: usize) -> Result<Self> {
        let path = path.into();
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::new(fallback_capacity, path));
            }
            Err(e) => return Err(RokoError::from(e)),
        };
        let snapshot: DiskSnapshot = serde_json::from_slice(&bytes).map_err(RokoError::from)?;
        let capacity = if snapshot.capacity == 0 {
            fallback_capacity
        } else {
            snapshot.capacity
        };
        let cache = Self::new(capacity, path);
        {
            let mut inner = cache.inner.lock();
            inner.hits = snapshot.hits;
            inner.misses = snapshot.misses;
            inner.evictions = snapshot.evictions;
            // Preserve insertion order from the snapshot; honour capacity.
            let keep = snapshot.order.len().min(capacity);
            let skip = snapshot.order.len().saturating_sub(keep);
            let kept_keys: Vec<String> = snapshot.order.iter().skip(skip).cloned().collect();
            let by_fp: HashMap<String, ContextPack> = snapshot
                .packs
                .into_iter()
                .map(|p| (p.fingerprint.clone(), p))
                .collect();
            for fp in kept_keys {
                if let Some(pack) = by_fp.get(&fp).cloned() {
                    inner.packs.insert(fp.clone(), pack);
                    inner.order.push_back(fp);
                }
            }
        }
        Ok(cache)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_pack(fp: &str, text: &str, tokens: usize) -> ContextPack {
        ContextPack::new(
            fp.to_string(),
            text.to_string(),
            vec![ContentHash::of(text.as_bytes())],
            tokens,
        )
    }

    fn tmp_path(dir: &TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    #[tokio::test]
    async fn put_then_get_returns_clone() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(4, tmp_path(&dir, "c.json"));
        let pack = sample_pack("fp-1", "hello world", 3);
        cache.put("fp-1".to_string(), pack);

        let got = cache.get("fp-1").expect("hit");
        assert_eq!(got.composed_text, "hello world");
        assert_eq!(got.fingerprint, "fp-1");
        assert_eq!(got.access_count, 1);
        assert_eq!(got.composed_signals.len(), 1);
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn get_on_empty_is_miss() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(2, tmp_path(&dir, "c.json"));
        assert!(cache.get("nope").is_none());
        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
    }

    #[tokio::test]
    async fn access_stats_update_on_repeat_get() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(4, tmp_path(&dir, "c.json"));
        cache.put("k".to_string(), sample_pack("k", "body", 1));

        let first = cache.get("k").expect("hit");
        // sleep 2ms to guarantee a monotonic timestamp advance on fast machines
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let second = cache.get("k").expect("hit");
        assert_eq!(first.access_count, 1);
        assert_eq!(second.access_count, 2);
        assert!(second.last_access_ms >= first.last_access_ms);
    }

    #[tokio::test]
    async fn lru_eviction_drops_oldest() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(2, tmp_path(&dir, "c.json"));
        cache.put("a".into(), sample_pack("a", "a", 1));
        cache.put("b".into(), sample_pack("b", "b", 1));
        // Touch "a" so "b" becomes the LRU.
        let _ = cache.get("a");
        cache.put("c".into(), sample_pack("c", "c", 1));

        assert_eq!(cache.len(), 2);
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_some());
        assert_eq!(cache.stats().evictions, 1);
    }

    #[tokio::test]
    async fn capacity_is_enforced_on_rapid_inserts() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(3, tmp_path(&dir, "c.json"));
        for i in 0..10u32 {
            let k = format!("k{i}");
            cache.put(k.clone(), sample_pack(&k, &k, 1));
            assert!(cache.len() <= 3);
        }
        assert_eq!(cache.len(), 3);
        // Oldest keys evicted.
        assert!(cache.get("k0").is_none());
        assert!(cache.get("k9").is_some());
    }

    #[tokio::test]
    async fn zero_capacity_disables_memory() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(0, tmp_path(&dir, "c.json"));
        cache.put("a".into(), sample_pack("a", "a", 1));
        assert_eq!(cache.len(), 0);
        assert!(cache.get("a").is_none());
    }

    #[tokio::test]
    async fn persist_and_load_round_trip() {
        let dir = TempDir::new().expect("tempdir");
        let path = tmp_path(&dir, "nested/dir/pack.json");
        let cache = ContextPackCache::new(4, path.clone());
        cache.put("a".into(), sample_pack("a", "alpha", 2));
        cache.put("b".into(), sample_pack("b", "beta", 3));
        let _ = cache.get("a");
        cache.persist().await.expect("persist");

        let loaded = ContextPackCache::load(&path, 4).await.expect("load");
        assert_eq!(loaded.capacity(), 4);
        assert_eq!(loaded.len(), 2);
        let a = loaded.get("a").expect("hit a");
        assert_eq!(a.composed_text, "alpha");
        assert_eq!(a.access_count, 1 + 1); // prior 1 + this get
    }

    #[tokio::test]
    async fn load_missing_file_returns_empty_cache() {
        let dir = TempDir::new().expect("tempdir");
        let path = tmp_path(&dir, "absent.json");
        let cache = ContextPackCache::load(&path, 5).await.expect("load");
        assert_eq!(cache.capacity(), 5);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.path(), path);
    }

    #[tokio::test]
    async fn load_rejects_malformed_json() {
        let dir = TempDir::new().expect("tempdir");
        let path = tmp_path(&dir, "bad.json");
        tokio::fs::write(&path, b"{not json").await.expect("write");
        let err = ContextPackCache::load(&path, 4).await.unwrap_err();
        assert!(matches!(err, RokoError::Json(_)));
    }

    #[tokio::test]
    async fn clear_drops_entries_but_keeps_counters() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(3, tmp_path(&dir, "c.json"));
        cache.put("a".into(), sample_pack("a", "a", 1));
        let _ = cache.get("a");
        let _ = cache.get("missing");
        cache.clear();
        assert_eq!(cache.len(), 0);
        let s = cache.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.entries, 0);
    }

    #[tokio::test]
    async fn evict_lru_returns_oldest_fingerprint() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(4, tmp_path(&dir, "c.json"));
        cache.put("a".into(), sample_pack("a", "a", 1));
        cache.put("b".into(), sample_pack("b", "b", 1));
        cache.put("c".into(), sample_pack("c", "c", 1));
        let _ = cache.get("a"); // bump a to MRU
        let victim = cache.evict_lru().expect("someone to evict");
        assert_eq!(victim, "b");
        assert_eq!(cache.len(), 2);
        assert!(cache.get("b").is_none());
    }

    #[tokio::test]
    async fn put_replaces_existing_entry_without_growing() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(2, tmp_path(&dir, "c.json"));
        cache.put("k".into(), sample_pack("k", "v1", 1));
        cache.put("k".into(), sample_pack("k", "v2", 2));
        assert_eq!(cache.len(), 1);
        let got = cache.get("k").expect("hit");
        assert_eq!(got.composed_text, "v2");
        assert_eq!(got.token_count, 2);
    }

    #[tokio::test]
    async fn stats_reports_hits_misses_and_capacity() {
        let dir = TempDir::new().expect("tempdir");
        let cache = ContextPackCache::new(8, tmp_path(&dir, "c.json"));
        cache.put("a".into(), sample_pack("a", "a", 1));
        let _ = cache.get("a");
        let _ = cache.get("a");
        let _ = cache.get("zzz");
        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.capacity, 8);
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.evictions, 0);
    }

    #[tokio::test]
    async fn persist_creates_parent_directories() {
        let dir = TempDir::new().expect("tempdir");
        let nested = tmp_path(&dir, "a/b/c/pack.json");
        let cache = ContextPackCache::new(2, nested.clone());
        cache.put("x".into(), sample_pack("x", "x", 1));
        cache.persist().await.expect("persist");
        assert!(nested.exists());
    }
}
