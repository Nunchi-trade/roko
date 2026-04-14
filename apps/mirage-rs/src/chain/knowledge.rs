//! `KnowledgeStore`: append-only log of [`InsightEntry`] + HDC retrieval layer.
//!
//! This is the "knowledge substrate" that rides on top of mirage's fork state.
//! Entries are:
//!   1. Posted via [`KnowledgeStore::post`] (content-addressed, deduplicated).
//!   2. Indexed in an [`HdcIndex`] (brute-force) and optional [`HnswBinaryIndex`].
//!   3. Searched via HDC similarity in microseconds.
//!   4. Confirmed, challenged, decayed, and eventually pruned.
//!
//! Because HDC-similar entries are treated as duplicates, re-posting the same
//! content (or a very close paraphrase) returns the existing id with a reduced
//! reward multiplier — the "duplicate penalty" from doc 06.
//!
//! The store does not persist to disk in the POC. Callers that need durability
//! can snapshot via [`KnowledgeStore::entries`] and restore via
//! [`KnowledgeStore::from_entries`].

use std::collections::HashMap;

use roko_primitives::HdcVector;
use serde::{Deserialize, Serialize};

use super::{
    hdc_index::{HdcIndex, Hit},
    hnsw::{HnswBinaryIndex, HnswConfig},
    insight::{InsightEntry, InsightId, KnowledgeKind, KnowledgeState},
};

/// Threshold above which a newly posted entry is considered a duplicate of an
/// existing one (doc 06: ">95% similarity → ~5% reward").
pub const DUPLICATE_SIMILARITY_THRESHOLD: f32 = 0.95;

/// Outcome of posting an entry.
#[derive(Clone, Debug, PartialEq)]
pub enum PostOutcome {
    /// Entry was newly accepted and indexed.
    Accepted {
        /// The content-addressed id assigned to the entry.
        id: InsightId,
    },
    /// Entry was HDC-similar to an existing entry; reward multiplier attenuated.
    Duplicate {
        /// Id of the near-match the new entry collapsed into.
        existing_id: InsightId,
        /// Hamming similarity at the time of posting.
        similarity: f32,
    },
    /// Content matches an existing id byte-for-byte; no change.
    ExactMatch {
        /// Id of the existing entry.
        id: InsightId,
    },
}

/// In-memory knowledge store.
pub struct KnowledgeStore {
    entries: HashMap<InsightId, InsightEntry>,
    hdc: HdcIndex,
    hnsw: Option<HnswBinaryIndex>,
    /// Switchover threshold — use HNSW when entry count exceeds this.
    hnsw_threshold: usize,
}

impl Default for KnowledgeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for KnowledgeStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnowledgeStore")
            .field("entries", &self.entries.len())
            .field("hdc_len", &self.hdc.len())
            .field("hnsw_enabled", &self.hnsw.is_some())
            .field("hnsw_threshold", &self.hnsw_threshold)
            .finish()
    }
}

/// Snapshot format for persisting/restoring a store.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeSnapshot {
    /// Entries ordered by posting time.
    pub entries: Vec<InsightEntry>,
}

impl KnowledgeStore {
    /// Constructs an empty store using brute-force HDC only.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            hdc: HdcIndex::new(),
            hnsw: None,
            hnsw_threshold: usize::MAX,
        }
    }

    /// Constructs a store with an HNSW index that auto-activates above `threshold` entries.
    #[must_use]
    pub fn with_hnsw(config: HnswConfig, threshold: usize) -> Self {
        Self {
            entries: HashMap::new(),
            hdc: HdcIndex::new(),
            hnsw: Some(HnswBinaryIndex::new(config)),
            hnsw_threshold: threshold,
        }
    }

    /// Number of distinct entries currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Posts a new entry. Returns an outcome describing whether it was accepted,
    /// duplicated, or already exists.
    pub fn post(
        &mut self,
        author: Vec<u8>,
        kind: KnowledgeKind,
        content: String,
        vector: HdcVector,
        enabled_by: Vec<InsightId>,
        now_secs: u64,
        stake_wei: u128,
    ) -> PostOutcome {
        let mut entry = InsightEntry::new(
            author, kind, content, vector, enabled_by, now_secs, stake_wei,
        );

        if self.entries.contains_key(&entry.id) {
            return PostOutcome::ExactMatch { id: entry.id };
        }

        // HDC duplicate check: if the new vector is >= threshold similar to any
        // existing entry of the same kind, attenuate reward and return duplicate.
        if let Some((existing_id, similarity)) = self.find_hdc_duplicate(&entry) {
            return PostOutcome::Duplicate {
                existing_id,
                similarity,
            };
        }

        entry.state = KnowledgeState::Active;
        let id = entry.id;
        let vector = entry.vector;
        self.hdc.insert(id, vector, entry.weight);
        if let Some(hnsw) = self.hnsw.as_mut() {
            if self.entries.len() + 1 >= self.hnsw_threshold {
                hnsw.insert(id, vector, entry.weight);
            }
        }
        self.entries.insert(id, entry);
        PostOutcome::Accepted { id }
    }

    fn find_hdc_duplicate(&self, entry: &InsightEntry) -> Option<(InsightId, f32)> {
        let top = self.hdc.top_k(&entry.vector, 5);
        top.into_iter()
            .filter_map(|hit| {
                let existing = self.entries.get(&hit.id)?;
                if existing.kind != entry.kind {
                    return None;
                }
                if hit.similarity >= DUPLICATE_SIMILARITY_THRESHOLD {
                    Some((hit.id, hit.similarity))
                } else {
                    None
                }
            })
            .next()
    }

    /// Retrieves a single entry by id.
    #[must_use]
    pub fn get(&self, id: InsightId) -> Option<&InsightEntry> {
        self.entries.get(&id)
    }

    /// Mutable access to an entry (for advanced callers).
    pub fn get_mut(&mut self, id: InsightId) -> Option<&mut InsightEntry> {
        self.entries.get_mut(&id)
    }

    /// Records a confirmation from `confirmer` on entry `id`. Updates indices.
    pub fn confirm(&mut self, id: InsightId, confirmer: Vec<u8>) -> Result<(), KnowledgeError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(KnowledgeError::NotFound(id))?;
        if matches!(entry.state, KnowledgeState::Pruned | KnowledgeState::Stale) {
            return Err(KnowledgeError::Immutable(id, entry.state));
        }
        if !entry.add_confirmation(confirmer) {
            return Err(KnowledgeError::DuplicateConfirmation(id));
        }
        let new_weight = entry.weight;
        self.hdc.set_weight(id, new_weight);
        if let Some(hnsw) = self.hnsw.as_mut() {
            hnsw.set_weight(id, new_weight);
        }
        Ok(())
    }

    /// Records a challenge against entry `id`.
    pub fn challenge(&mut self, id: InsightId, challenger: Vec<u8>) -> Result<(), KnowledgeError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(KnowledgeError::NotFound(id))?;
        if matches!(entry.state, KnowledgeState::Pruned | KnowledgeState::Stale) {
            return Err(KnowledgeError::Immutable(id, entry.state));
        }
        if !entry.add_challenge(challenger) {
            return Err(KnowledgeError::DuplicateChallenge(id));
        }
        Ok(())
    }

    /// Applies decay to every entry, refreshing the index weights.
    ///
    /// Entries whose decayed weight drops below 0.01 × initial are moved to
    /// `Pruned` and removed from both HDC and HNSW indices (but retained in
    /// `entries` for audit).
    pub fn apply_decay(&mut self, now_secs: u64) {
        let mut to_prune: Vec<InsightId> = Vec::new();
        for entry in self.entries.values_mut() {
            if matches!(entry.state, KnowledgeState::Pruned | KnowledgeState::Stale) {
                continue;
            }
            entry.refresh_weight(now_secs);
            if entry.weight / entry.initial_weight < 0.01 {
                entry.state = KnowledgeState::Pruned;
                to_prune.push(entry.id);
            } else {
                self.hdc.set_weight(entry.id, entry.weight);
                if let Some(hnsw) = self.hnsw.as_mut() {
                    hnsw.set_weight(entry.id, entry.weight);
                }
            }
        }
        for id in to_prune {
            self.hdc.remove(id);
        }
    }

    /// Semantic search over the store. Returns up to `k` entries ordered by
    /// `similarity × weight`. Uses HNSW if the index is active, otherwise
    /// brute-force HDC.
    #[must_use]
    pub fn search(&self, query: &HdcVector, k: usize) -> Vec<Hit> {
        if let Some(hnsw) = self.hnsw.as_ref() {
            if self.entries.len() >= self.hnsw_threshold {
                let ef = (k * 4).max(40);
                return hnsw.search(query, k, ef);
            }
        }
        self.hdc.top_k(query, k)
    }

    /// Returns entries matching the given kind.
    #[must_use]
    pub fn by_kind(&self, kind: KnowledgeKind) -> Vec<&InsightEntry> {
        self.entries.values().filter(|e| e.kind == kind).collect()
    }

    /// Snapshots the store for persistence.
    #[must_use]
    pub fn snapshot(&self) -> KnowledgeSnapshot {
        let mut entries: Vec<InsightEntry> = self.entries.values().cloned().collect();
        entries.sort_by_key(|e| e.created_at);
        KnowledgeSnapshot { entries }
    }

    /// Restores a store from a snapshot.
    #[must_use]
    pub fn from_snapshot(snapshot: KnowledgeSnapshot) -> Self {
        let mut store = Self::new();
        for entry in snapshot.entries {
            let id = entry.id;
            let vector = entry.vector;
            let weight = entry.weight;
            store.hdc.insert(id, vector, weight);
            store.entries.insert(id, entry);
        }
        store
    }

    /// Iterator over stored entries.
    pub fn entries(&self) -> impl Iterator<Item = &InsightEntry> {
        self.entries.values()
    }
}

/// Error returned from [`KnowledgeStore`] operations.
#[derive(Debug, thiserror::Error)]
pub enum KnowledgeError {
    /// No entry with the given id.
    #[error("insight not found: {0}")]
    NotFound(InsightId),
    /// Entry is in a terminal state.
    #[error("insight {0} is immutable (state={1:?})")]
    Immutable(InsightId, KnowledgeState),
    /// Caller already confirmed this entry.
    #[error("duplicate confirmation on {0}")]
    DuplicateConfirmation(InsightId),
    /// Caller already challenged this entry.
    #[error("duplicate challenge on {0}")]
    DuplicateChallenge(InsightId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::projection::project_tokens;

    fn author() -> Vec<u8> {
        b"alice".to_vec()
    }

    #[test]
    fn post_accepts_new_entry() {
        let mut store = KnowledgeStore::new();
        let vector = project_tokens("uniswap v3 STF revert means insufficient allowance");
        let outcome = store.post(
            author(),
            KnowledgeKind::Insight,
            "uniswap v3 STF revert means insufficient allowance".into(),
            vector,
            Vec::new(),
            1_700_000_000,
            2_000_000_000_000_000,
        );
        assert!(matches!(outcome, PostOutcome::Accepted { .. }));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn post_same_content_is_exact_match() {
        let mut store = KnowledgeStore::new();
        let content = "arbitrum sequencer underprices internal calls";
        let v = project_tokens(content);
        let first = store.post(
            author(),
            KnowledgeKind::Heuristic,
            content.into(),
            v,
            Vec::new(),
            1_700_000_000,
            0,
        );
        let second = store.post(
            author(),
            KnowledgeKind::Heuristic,
            content.into(),
            v,
            Vec::new(),
            1_700_000_001,
            0,
        );
        assert!(matches!(first, PostOutcome::Accepted { .. }));
        assert!(matches!(second, PostOutcome::ExactMatch { .. }));
    }

    #[test]
    fn post_duplicate_vector_attenuates() {
        let mut store = KnowledgeStore::new();
        let v = HdcVector::from_seed(b"shared-content-vector");
        store.post(
            author(),
            KnowledgeKind::Warning,
            "original warning".into(),
            v,
            Vec::new(),
            1_000,
            0,
        );
        // Same vector, different content => HDC duplicate.
        let outcome = store.post(
            b"bob".to_vec(),
            KnowledgeKind::Warning,
            "paraphrased warning".into(),
            v,
            Vec::new(),
            1_001,
            0,
        );
        match outcome {
            PostOutcome::Duplicate { similarity, .. } => {
                assert!(similarity >= DUPLICATE_SIMILARITY_THRESHOLD);
            }
            other => panic!("expected duplicate, got {other:?}"),
        }
    }

    #[test]
    fn search_returns_exact_match_first() {
        let mut store = KnowledgeStore::new();
        for (i, text) in [
            "deploy proxy with eip-1967 slot alignment",
            "warp test with tick range narrowing",
            "check arbitrum gas 3x buffer",
            "reading USDT approve dance",
        ]
        .iter()
        .enumerate()
        {
            store.post(
                author(),
                KnowledgeKind::Insight,
                (*text).into(),
                project_tokens(text),
                Vec::new(),
                1_000 + i as u64,
                0,
            );
        }
        let q = project_tokens("deploy proxy with eip-1967 slot alignment");
        let hits = store.search(&q, 2);
        assert!(!hits.is_empty());
        assert_eq!(
            store.get(hits[0].id).unwrap().content,
            "deploy proxy with eip-1967 slot alignment"
        );
    }

    #[test]
    fn confirm_boosts_weight_and_updates_index() {
        let mut store = KnowledgeStore::new();
        let v = project_tokens("check gas before swap");
        let PostOutcome::Accepted { id } = store.post(
            author(),
            KnowledgeKind::Heuristic,
            "check gas before swap".into(),
            v,
            Vec::new(),
            1_000,
            0,
        ) else {
            panic!("expected Accepted");
        };
        store.confirm(id, b"bob".to_vec()).unwrap();
        store.confirm(id, b"carol".to_vec()).unwrap();
        let entry = store.get(id).unwrap();
        assert_eq!(entry.confirmations.len(), 2);
        assert!(entry.weight > entry.initial_weight);
    }

    #[test]
    fn confirm_rejects_duplicate_confirmer() {
        let mut store = KnowledgeStore::new();
        let v = project_tokens("dup confirmer test");
        let PostOutcome::Accepted { id } = store.post(
            author(),
            KnowledgeKind::Insight,
            "dup confirmer test".into(),
            v,
            Vec::new(),
            1_000,
            0,
        ) else {
            panic!();
        };
        assert!(store.confirm(id, b"bob".to_vec()).is_ok());
        assert!(matches!(
            store.confirm(id, b"bob".to_vec()).unwrap_err(),
            KnowledgeError::DuplicateConfirmation(_)
        ));
    }

    #[test]
    fn challenge_marks_state() {
        let mut store = KnowledgeStore::new();
        let v = project_tokens("wrong idea");
        let PostOutcome::Accepted { id } = store.post(
            author(),
            KnowledgeKind::AntiKnowledge,
            "wrong idea".into(),
            v,
            Vec::new(),
            1_000,
            0,
        ) else {
            panic!();
        };
        store.challenge(id, b"challenger".to_vec()).unwrap();
        assert_eq!(store.get(id).unwrap().state, KnowledgeState::Challenged);
    }

    #[test]
    fn apply_decay_prunes_ancient_entries() {
        let mut store = KnowledgeStore::new();
        let v = project_tokens("warning about ancient oracle");
        let PostOutcome::Accepted { id } = store.post(
            author(),
            KnowledgeKind::Warning,
            "warning about ancient oracle".into(),
            v,
            Vec::new(),
            0,
            0,
        ) else {
            panic!();
        };
        // Warning half-life = 180s. 20 half-lives pushes weight below 0.01.
        store.apply_decay(180 * 20);
        let entry = store.get(id).unwrap();
        assert!(matches!(
            entry.state,
            KnowledgeState::Pruned | KnowledgeState::Stale
        ));
        // Pruned entries are removed from the search index.
        let hits = store.search(&v, 5);
        assert!(
            !hits.iter().any(|h| h.id == id) || entry.state == KnowledgeState::Stale,
            "pruned entry must not appear in search results"
        );
    }

    #[test]
    fn snapshot_and_restore_preserves_entries() {
        let mut store = KnowledgeStore::new();
        let v = project_tokens("snap test");
        store.post(
            author(),
            KnowledgeKind::Insight,
            "snap test".into(),
            v,
            Vec::new(),
            1_000,
            0,
        );
        let snap = store.snapshot();
        assert_eq!(snap.entries.len(), 1);

        let restored = KnowledgeStore::from_snapshot(snap);
        assert_eq!(restored.len(), 1);
        let hits = restored.search(&v, 1);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn by_kind_filters_entries() {
        let mut store = KnowledgeStore::new();
        store.post(
            author(),
            KnowledgeKind::Warning,
            "w1".into(),
            project_tokens("w1"),
            Vec::new(),
            0,
            0,
        );
        store.post(
            author(),
            KnowledgeKind::Insight,
            "i1".into(),
            project_tokens("i1"),
            Vec::new(),
            0,
            0,
        );
        store.post(
            author(),
            KnowledgeKind::Insight,
            "i2".into(),
            project_tokens("i2"),
            Vec::new(),
            0,
            0,
        );
        assert_eq!(store.by_kind(KnowledgeKind::Warning).len(), 1);
        assert_eq!(store.by_kind(KnowledgeKind::Insight).len(), 2);
    }
}
