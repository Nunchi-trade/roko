//! Pointer GC policy — configurable eviction for memory pointers.
//!
//! The pointer store can grow unboundedly if tool results are never
//! evicted. This module provides [`PointerGcPolicy`], a configurable
//! eviction policy that supports:
//!
//! - **Age-based eviction**: pointers older than `max_age_turns` are
//!   unconditionally evicted.
//! - **Size-based eviction**: when total stored bytes exceed
//!   `max_total_bytes`, the least-recently-accessed pointers are evicted
//!   until the total fits within budget.
//!
//! The GC is **passive** — it does not run on a timer. The caller
//! invokes [`PointerGcPolicy::select_evictions`] at convenient points
//! (e.g. between turns) and deletes the returned pointer IDs.

use serde::{Deserialize, Serialize};

// ─── PointerMeta ────────────────────────────────────────────────────────────

/// Metadata about a single pointer, sufficient for GC decisions.
///
/// This is a lightweight summary — it does not contain the pointer's
/// content. The GC policy operates purely on metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerMeta {
    /// Pointer ID (matches the ID in the pointer store).
    pub id: String,
    /// Size of the stored content in bytes.
    pub size_bytes: u64,
    /// Turn number when the pointer was created.
    pub created_at_turn: u32,
    /// Turn number when the pointer was last accessed (expanded).
    pub last_accessed_turn: u32,
}

// ─── PointerGcPolicy ────────────────────────────────────────────────────────

/// Configurable eviction policy for memory pointers.
///
/// Combines age-based and size-based eviction strategies. Both can be
/// active simultaneously: age-based eviction is applied first, then
/// size-based LRU kicks in if the remaining pointers still exceed
/// `max_total_bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerGcPolicy {
    /// Evict pointers older than this many turns. A value of 0 disables
    /// age-based eviction.
    pub max_age_turns: u32,
    /// LRU eviction when total stored bytes exceed this threshold. A
    /// value of `u64::MAX` effectively disables size-based eviction.
    pub max_total_bytes: u64,
}

impl PointerGcPolicy {
    /// Default policy: evict after 10 turns, cap at 10 `MiB`.
    pub const DEFAULT_MAX_AGE_TURNS: u32 = 10;
    /// Default total bytes budget: 10 `MiB`.
    pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024;

    /// Construct a policy with the given parameters.
    #[must_use]
    pub const fn new(max_age_turns: u32, max_total_bytes: u64) -> Self {
        Self {
            max_age_turns,
            max_total_bytes,
        }
    }

    /// Construct a policy with default parameters.
    #[must_use]
    pub const fn default_policy() -> Self {
        Self {
            max_age_turns: Self::DEFAULT_MAX_AGE_TURNS,
            max_total_bytes: Self::DEFAULT_MAX_TOTAL_BYTES,
        }
    }

    /// Check whether a single pointer should be evicted based on age.
    ///
    /// `current_turn` is the current conversation turn number. A pointer
    /// is stale when `current_turn - created_at_turn > max_age_turns`.
    ///
    /// Returns `false` if age-based eviction is disabled
    /// (`max_age_turns == 0`).
    #[must_use]
    pub const fn should_evict(&self, pointer: &PointerMeta, current_turn: u32) -> bool {
        if self.max_age_turns == 0 {
            return false;
        }
        current_turn.saturating_sub(pointer.created_at_turn) > self.max_age_turns
    }

    /// Select pointer IDs to evict from the given set.
    ///
    /// Strategy:
    /// 1. Mark all pointers exceeding `max_age_turns` for eviction.
    /// 2. If the remaining pointers' total size exceeds `budget`
    ///    (`max_total_bytes`), evict the least-recently-accessed pointers
    ///    (by `last_accessed_turn`, ascending) until the total fits.
    ///
    /// Returns the IDs of pointers to evict.
    #[must_use]
    pub fn select_evictions(
        &self,
        pointers: &[PointerMeta],
        current_turn: u32,
    ) -> Vec<String> {
        let mut evict_ids: Vec<String> = Vec::new();

        // Phase 1: age-based eviction.
        let mut survivors: Vec<&PointerMeta> = Vec::new();
        for p in pointers {
            if self.should_evict(p, current_turn) {
                evict_ids.push(p.id.clone());
            } else {
                survivors.push(p);
            }
        }

        // Phase 2: size-based LRU eviction.
        let total: u64 = survivors.iter().map(|p| p.size_bytes).sum();
        if total > self.max_total_bytes {
            // Sort survivors by last_accessed_turn ascending (LRU first),
            // then by size descending (evict larger pointers first on tie).
            let mut sorted: Vec<&PointerMeta> = survivors;
            sorted.sort_by(|a, b| {
                a.last_accessed_turn
                    .cmp(&b.last_accessed_turn)
                    .then_with(|| b.size_bytes.cmp(&a.size_bytes))
            });

            let mut remaining = total;
            for p in &sorted {
                if remaining <= self.max_total_bytes {
                    break;
                }
                evict_ids.push(p.id.clone());
                remaining = remaining.saturating_sub(p.size_bytes);
            }
        }

        evict_ids
    }
}

impl Default for PointerGcPolicy {
    fn default() -> Self {
        Self::default_policy()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, size: u64, created: u32, accessed: u32) -> PointerMeta {
        PointerMeta {
            id: id.to_owned(),
            size_bytes: size,
            created_at_turn: created,
            last_accessed_turn: accessed,
        }
    }

    #[test]
    fn should_evict_old_pointer() {
        let policy = PointerGcPolicy::new(5, u64::MAX);
        let p = meta("old", 100, 1, 2);
        // current_turn=7, age=6 > max_age_turns=5 => evict
        assert!(policy.should_evict(&p, 7));
    }

    #[test]
    fn should_not_evict_young_pointer() {
        let policy = PointerGcPolicy::new(5, u64::MAX);
        let p = meta("young", 100, 5, 5);
        // current_turn=7, age=2 <= max_age_turns=5 => keep
        assert!(!policy.should_evict(&p, 7));
    }

    #[test]
    fn should_evict_disabled_when_max_age_zero() {
        let policy = PointerGcPolicy::new(0, u64::MAX);
        let p = meta("any", 100, 0, 0);
        assert!(!policy.should_evict(&p, 1000));
    }

    #[test]
    fn select_evictions_age_only() {
        let policy = PointerGcPolicy::new(3, u64::MAX);
        let pointers = vec![
            meta("old-1", 100, 1, 1),
            meta("old-2", 200, 2, 2),
            meta("recent", 300, 8, 9),
        ];
        let evicted = policy.select_evictions(&pointers, 10);
        assert!(evicted.contains(&"old-1".to_owned()));
        assert!(evicted.contains(&"old-2".to_owned()));
        assert!(!evicted.contains(&"recent".to_owned()));
    }

    #[test]
    fn select_evictions_size_lru() {
        // No age eviction, but total exceeds budget.
        let policy = PointerGcPolicy::new(0, 500);
        let pointers = vec![
            meta("lru-1", 200, 1, 1),   // least recently accessed
            meta("lru-2", 200, 2, 3),   // mid
            meta("recent", 200, 3, 5),  // most recently accessed
        ];
        // Total = 600, budget = 500. Need to evict 100+ bytes.
        // LRU order: lru-1 (accessed=1), lru-2 (accessed=3), recent (accessed=5).
        // Evict lru-1 (200 bytes), remaining = 400 <= 500. Done.
        let evicted = policy.select_evictions(&pointers, 10);
        assert_eq!(evicted, vec!["lru-1"]);
    }

    #[test]
    fn select_evictions_combined_age_and_size() {
        let policy = PointerGcPolicy::new(5, 300);
        let pointers = vec![
            meta("ancient", 100, 1, 1),  // age=9 > 5 => evicted by age
            meta("p1", 200, 6, 6),       // within age, accessed=6
            meta("p2", 200, 7, 8),       // within age, accessed=8
        ];
        // After age eviction: survivors = [p1, p2], total = 400 > 300.
        // LRU: p1 (accessed=6). Evict p1 => remaining 200 <= 300.
        let evicted = policy.select_evictions(&pointers, 10);
        assert!(evicted.contains(&"ancient".to_owned()));
        assert!(evicted.contains(&"p1".to_owned()));
        assert!(!evicted.contains(&"p2".to_owned()));
    }

    #[test]
    fn select_evictions_nothing_to_evict() {
        let policy = PointerGcPolicy::new(10, 10_000);
        let pointers = vec![
            meta("a", 100, 5, 5),
            meta("b", 200, 6, 7),
        ];
        let evicted = policy.select_evictions(&pointers, 8);
        assert!(evicted.is_empty());
    }

    #[test]
    fn select_evictions_empty_input() {
        let policy = PointerGcPolicy::default();
        let evicted = policy.select_evictions(&[], 100);
        assert!(evicted.is_empty());
    }

    #[test]
    fn default_policy_values() {
        let policy = PointerGcPolicy::default();
        assert_eq!(policy.max_age_turns, 10);
        assert_eq!(policy.max_total_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn pointer_meta_serde_roundtrip() {
        let m = meta("test-ptr", 4096, 5, 8);
        let json = serde_json::to_string(&m).unwrap();
        let decoded: PointerMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, m);
    }

    #[test]
    fn policy_serde_roundtrip() {
        let policy = PointerGcPolicy::new(7, 5_000_000);
        let json = serde_json::to_string(&policy).unwrap();
        let decoded: PointerGcPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, policy);
    }

    #[test]
    fn size_eviction_prefers_larger_on_tie() {
        // When two pointers have the same last_accessed_turn, evict the larger one first.
        let policy = PointerGcPolicy::new(0, 300);
        let pointers = vec![
            meta("small", 100, 1, 3),
            meta("big", 300, 2, 3),    // same access turn but bigger
            meta("recent", 100, 3, 5),
        ];
        // Total = 500, budget = 300. LRU sort by (accessed, -size):
        // big (3, 300), small (3, 100), recent (5, 100).
        // Evict big (300) => remaining 200 <= 300.
        let evicted = policy.select_evictions(&pointers, 10);
        assert_eq!(evicted, vec!["big"]);
    }
}
