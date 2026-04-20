//! Dispatch-level dedup cache for idempotent agent dispatch (DEPLOY-09).
//!
//! Prevents duplicate agent dispatches that can occur when:
//! - The same task is retried after a transient failure
//! - A race condition submits two dispatch requests for the same work
//! - A watchdog re-dispatches a task before the first attempt completes
//!
//! The cache is keyed on a content hash of the dispatch parameters (system
//! prompt hash + user message hash + tool set hash). Entries expire after
//! a configurable TTL.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

/// Default TTL for dedup entries (10 minutes).
pub const DEFAULT_DEDUP_TTL: Duration = Duration::from_secs(600);

/// Maximum dedup entries before eviction.
const MAX_DEDUP_ENTRIES: usize = 512;

/// A key identifying a unique dispatch request.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DedupKey {
    /// Hash of the dispatch parameters.
    content_hash: u64,
}

/// Status of a dedup entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DedupStatus {
    /// This is a new request; proceed with dispatch.
    New,
    /// This request is a duplicate of an in-flight dispatch.
    InFlight {
        /// When the original dispatch started.
        started_at: Instant,
    },
    /// This request already completed (result is cached).
    Completed {
        /// Whether the original dispatch succeeded.
        success: bool,
        /// When the original dispatch completed.
        completed_at: Instant,
    },
}

/// Internal entry tracking dispatch state.
#[derive(Clone, Debug)]
struct DedupEntry {
    /// When this entry was created.
    created_at: Instant,
    /// Whether the dispatch is still in flight.
    in_flight: bool,
    /// Whether the dispatch succeeded (set on completion).
    success: Option<bool>,
    /// When the dispatch completed (set on completion).
    completed_at: Option<Instant>,
}

/// Cache that deduplicates agent dispatch requests.
///
/// Before dispatching, call [`check`] to see if a request with the same
/// parameters is already in flight or recently completed. After dispatch
/// completes, call [`complete`] to record the result.
pub struct DedupCache {
    entries: HashMap<DedupKey, DedupEntry>,
    ttl: Duration,
    /// Number of duplicate requests suppressed.
    dedup_count: u64,
    /// Number of unique dispatches.
    dispatch_count: u64,
}

impl DedupCache {
    /// Create a new dedup cache with the default TTL.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            ttl: DEFAULT_DEDUP_TTL,
            dedup_count: 0,
            dispatch_count: 0,
        }
    }

    /// Create a cache with a custom TTL.
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
            dedup_count: 0,
            dispatch_count: 0,
        }
    }

    /// Check whether a dispatch request should proceed.
    ///
    /// Returns [`DedupStatus::New`] for new requests (and marks them as
    /// in-flight). Returns [`DedupStatus::InFlight`] or
    /// [`DedupStatus::Completed`] for duplicates.
    pub fn check(&mut self, key: &DedupKey) -> DedupStatus {
        self.evict_expired();

        if let Some(entry) = self.entries.get(key) {
            if entry.in_flight {
                self.dedup_count += 1;
                return DedupStatus::InFlight {
                    started_at: entry.created_at,
                };
            }
            if let Some(success) = entry.success {
                self.dedup_count += 1;
                return DedupStatus::Completed {
                    success,
                    completed_at: entry.completed_at.unwrap_or(entry.created_at),
                };
            }
        }

        // New request: mark as in-flight.
        if self.entries.len() >= MAX_DEDUP_ENTRIES {
            self.evict_oldest();
        }

        let now = Instant::now();
        self.entries.insert(
            key.clone(),
            DedupEntry {
                created_at: now,
                in_flight: true,
                success: None,
                completed_at: None,
            },
        );
        self.dispatch_count += 1;

        DedupStatus::New
    }

    /// Record dispatch completion.
    ///
    /// Marks the entry as no longer in-flight and records the success/failure.
    pub fn complete(&mut self, key: &DedupKey, success: bool) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.in_flight = false;
            entry.success = Some(success);
            entry.completed_at = Some(Instant::now());
        }
    }

    /// Remove an entry (e.g., if the dispatch was cancelled).
    pub fn remove(&mut self, key: &DedupKey) {
        self.entries.remove(key);
    }

    /// Number of active entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total duplicate requests suppressed.
    #[must_use]
    pub fn dedup_count(&self) -> u64 {
        self.dedup_count
    }

    /// Total unique dispatches.
    #[must_use]
    pub fn dispatch_count(&self) -> u64 {
        self.dispatch_count
    }

    /// Dedup rate as a fraction in `[0.0, 1.0]`.
    #[must_use]
    pub fn dedup_rate(&self) -> f64 {
        let total = self.dedup_count + self.dispatch_count;
        if total == 0 {
            0.0
        } else {
            self.dedup_count as f64 / total as f64
        }
    }

    /// Evict expired entries.
    fn evict_expired(&mut self) {
        let now = Instant::now();
        self.entries
            .retain(|_, entry| now.duration_since(entry.created_at) <= self.ttl);
    }

    /// Evict the oldest entry.
    fn evict_oldest(&mut self) {
        let oldest_key = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.created_at)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest_key {
            self.entries.remove(&key);
        }
    }
}

impl Default for DedupCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a dedup key from dispatch parameters.
///
/// Hashes the system prompt, user message, and model identifier to produce
/// a content-addressed key. Two requests with identical parameters will
/// produce the same key.
#[must_use]
pub fn dedup_key(system_prompt: &str, user_message: &str, model: &str) -> DedupKey {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    system_prompt.hash(&mut hasher);
    user_message.hash(&mut hasher);
    model.hash(&mut hasher);
    DedupKey {
        content_hash: hasher.finish(),
    }
}

/// Build a dedup key from a task ID and agent name.
///
/// Simpler variant for plan execution where task+agent is the natural key.
#[must_use]
pub fn dedup_key_task(task_id: &str, agent_name: &str) -> DedupKey {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "task".hash(&mut hasher);
    task_id.hash(&mut hasher);
    agent_name.hash(&mut hasher);
    DedupKey {
        content_hash: hasher.finish(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_request_returns_new() {
        let mut cache = DedupCache::new();
        let key = dedup_key("system", "hello world", "claude-3.5");

        let status = cache.check(&key);
        assert!(matches!(status, DedupStatus::New));
        assert_eq!(cache.dispatch_count(), 1);
    }

    #[test]
    fn duplicate_in_flight_detected() {
        let mut cache = DedupCache::new();
        let key = dedup_key("system", "hello world", "claude-3.5");

        cache.check(&key); // First: New
        let status = cache.check(&key); // Second: InFlight

        assert!(matches!(status, DedupStatus::InFlight { .. }));
        assert_eq!(cache.dedup_count(), 1);
        assert_eq!(cache.dispatch_count(), 1);
    }

    #[test]
    fn completed_request_detected() {
        let mut cache = DedupCache::new();
        let key = dedup_key("system", "hello world", "claude-3.5");

        cache.check(&key);
        cache.complete(&key, true);

        let status = cache.check(&key);
        assert!(matches!(status, DedupStatus::Completed { success: true, .. }));
        assert_eq!(cache.dedup_count(), 1);
    }

    #[test]
    fn different_params_different_keys() {
        let mut cache = DedupCache::new();

        let key1 = dedup_key("system", "hello", "claude-3.5");
        let key2 = dedup_key("system", "world", "claude-3.5");

        cache.check(&key1);
        let status = cache.check(&key2);

        assert!(matches!(status, DedupStatus::New));
        assert_eq!(cache.dispatch_count(), 2);
    }

    #[test]
    fn remove_allows_retry() {
        let mut cache = DedupCache::new();
        let key = dedup_key("system", "hello", "claude-3.5");

        cache.check(&key);
        cache.remove(&key);

        let status = cache.check(&key);
        assert!(matches!(status, DedupStatus::New));
        assert_eq!(cache.dispatch_count(), 2);
    }

    #[test]
    fn ttl_expiration_clears_entries() {
        let mut cache = DedupCache::with_ttl(Duration::from_millis(1));
        let key = dedup_key("system", "hello", "claude-3.5");

        cache.check(&key);

        std::thread::sleep(Duration::from_millis(5));

        let status = cache.check(&key);
        assert!(matches!(status, DedupStatus::New));
    }

    #[test]
    fn task_key_works() {
        let mut cache = DedupCache::new();
        let key = dedup_key_task("task-42", "claude-agent-1");

        let status = cache.check(&key);
        assert!(matches!(status, DedupStatus::New));

        let status = cache.check(&key);
        assert!(matches!(status, DedupStatus::InFlight { .. }));
    }

    #[test]
    fn dedup_rate_calculation() {
        let mut cache = DedupCache::new();
        let key = dedup_key("system", "hello", "claude-3.5");

        cache.check(&key); // dispatch_count = 1
        cache.check(&key); // dedup_count = 1

        assert!((cache.dedup_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn failed_dispatch_still_deduped() {
        let mut cache = DedupCache::new();
        let key = dedup_key("system", "hello", "claude-3.5");

        cache.check(&key);
        cache.complete(&key, false);

        let status = cache.check(&key);
        assert!(matches!(
            status,
            DedupStatus::Completed {
                success: false,
                ..
            }
        ));
    }
}
