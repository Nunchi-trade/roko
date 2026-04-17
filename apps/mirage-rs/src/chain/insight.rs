//! Knowledge entries committed to the chain substrate.
//!
//! This module defines the on-chain knowledge layer described in
//! `tmp/agent-chain/05-knowledge-layer.md`. Each entry has a content-addressed id,
//! a 10,240-bit HDC hypervector for semantic retrieval, a typed body, and a state
//! machine that tracks its lifecycle from creation through confirmation, decay, and
//! potential pruning.
//!
//! The chain uses plain ETH as the settlement unit — token names from the design
//! docs (GNOS, reward schedules) are intentionally elided. Reward/stake parameters
//! are expressed as raw integers (wei) that callers can interpret.

use std::collections::HashSet;

use roko_primitives::HdcVector;
use serde::{Deserialize, Serialize};

/// Content-addressed identifier for an [`InsightEntry`].
///
/// Computed as a FNV-1a64 hash of (author ‖ content ‖ kind_tag). Deterministic
/// across nodes given identical inputs. 128 bits of raw space are emulated by
/// combining two FNV rounds; collisions are astronomically unlikely at realistic
/// entry counts (<1 in 10^10 at 10M entries).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InsightId(pub [u8; 16]);

impl InsightId {
    /// Computes the content-addressed id from the author, content bytes, and knowledge kind.
    #[must_use]
    pub fn derive(author: &[u8], content: &[u8], kind: KnowledgeKind) -> Self {
        let tag = kind.tag_byte();
        let mut lo: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in author.iter().chain(content).chain(std::iter::once(&tag)) {
            lo ^= u64::from(*byte);
            lo = lo.wrapping_mul(0x0100_0000_01b3);
        }
        let mut hi: u64 = lo ^ 0xA5A5_A5A5_5A5A_5A5A;
        for byte in content.iter().chain(author).rev() {
            hi ^= u64::from(*byte);
            hi = hi.wrapping_mul(0x0100_0000_01b3);
        }
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&lo.to_le_bytes());
        out[8..].copy_from_slice(&hi.to_le_bytes());
        Self(out)
    }

    /// Returns a hex representation of the id (32 lowercase characters, no prefix).
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(32);
        for byte in self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }
}

impl std::fmt::Display for InsightId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "insight:{}", self.to_hex())
    }
}

/// Typed body of a knowledge entry — six variants as described in doc 05.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    /// A factual observation derived from task execution (what IS).
    Insight,
    /// A learned behavioural strategy for a specific context (what to DO).
    Heuristic,
    /// Knowledge about what NOT to do (failure modes, dead ends).
    Warning,
    /// An observed cause-and-effect relationship (mechanism).
    CausalLink,
    /// A reusable partial plan / sequence of steps.
    StrategyFragment,
    /// Explicitly wrong information that was once believed correct.
    AntiKnowledge,
}

impl KnowledgeKind {
    /// Returns a stable single-byte tag for hashing.
    #[must_use]
    pub const fn tag_byte(self) -> u8 {
        match self {
            Self::Insight => 0x01,
            Self::Heuristic => 0x02,
            Self::Warning => 0x03,
            Self::CausalLink => 0x04,
            Self::StrategyFragment => 0x05,
            Self::AntiKnowledge => 0x06,
        }
    }

    /// Default half-life in seconds, from the doc 05 table (400ms blocks).
    #[must_use]
    pub const fn default_half_life_seconds(self) -> u64 {
        // 216,000 blocks per day × block_secs = seconds.
        match self {
            Self::Warning => 180,                  // 3 minutes
            Self::Insight => 7 * 86_400,           // 7 days
            Self::Heuristic => 15 * 86_400,        // 15 days
            Self::CausalLink => 15 * 86_400,       // 15 days
            Self::StrategyFragment => 15 * 86_400, // 15 days
            Self::AntiKnowledge => 15 * 86_400,    // 15 days
        }
    }

    /// Base reward in wei (plain ETH; GNOS tokenomics intentionally elided).
    ///
    /// Kept as a simple scalar for the POC so callers can build richer reward
    /// formulas on top. Values chosen to preserve the relative magnitudes from
    /// doc 06 (`Warning` > `Insight` > others), scaled to micro-ether.
    #[must_use]
    pub const fn base_reward_wei(self) -> u128 {
        match self {
            Self::Warning => 75_000_000_000_000,        // 0.000075 ETH
            Self::Insight => 50_000_000_000_000,        // 0.00005  ETH
            Self::CausalLink => 65_000_000_000_000,     // 0.000065 ETH
            Self::AntiKnowledge => 100_000_000_000_000, // 0.0001 ETH
            Self::Heuristic | Self::StrategyFragment => 60_000_000_000_000,
        }
    }
}

/// Lifecycle state of an [`InsightEntry`] — see doc 05 §4.
///
/// Transitions are driven by agent activity (confirmations, challenges) and by
/// passive decay. `KnowledgeStore::apply_decay` walks the table to move entries
/// between `Active`, `Decaying`, and `Stale`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeState {
    /// Just posted, not yet indexed for retrieval.
    Created,
    /// Indexed and searchable; weight >= 0.25 × initial_weight.
    Active,
    /// Passed its first confirmation threshold (>=3 confirmations).
    Confirmed,
    /// Weight has decayed below 0.25 × initial_weight but entry is still searchable.
    Decaying,
    /// An open challenge is contesting this entry; reads still allowed.
    Challenged,
    /// Challenge resolved against the entry OR weight dropped below 0.01 × initial; removed from index.
    Pruned,
    /// Entry has aged out (>=5 × half_life elapsed) and is no longer competing for retrieval.
    Stale,
}

impl KnowledgeState {
    /// Returns whether transitioning to `next` is valid from `self`.
    ///
    /// See doc 05 §4 for the full state machine. Terminal states (`Pruned`, `Stale`)
    /// are sinks — no outbound transitions. Any non-terminal state may age into
    /// `Stale` once the 5×half-life threshold passes.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        match (self, next) {
            (Self::Created, Self::Active | Self::Stale) => true,
            (Self::Active, Self::Confirmed | Self::Decaying | Self::Challenged | Self::Stale) => {
                true
            }
            (Self::Confirmed, Self::Active | Self::Decaying | Self::Challenged | Self::Stale) => {
                true
            }
            (
                Self::Decaying,
                Self::Active | Self::Confirmed | Self::Stale | Self::Challenged | Self::Pruned,
            ) => true,
            (
                Self::Challenged,
                Self::Active | Self::Confirmed | Self::Pruned | Self::Decaying | Self::Stale,
            ) => true,
            (Self::Pruned | Self::Stale, _) => false,
            _ => false,
        }
    }
}

/// An on-chain knowledge entry.
///
/// This is the unit of shared intelligence between golems. Each entry carries its
/// HDC vector inline so downstream indices can rebuild without an external embedding
/// step. The chain stores roughly 1.3 KB per entry (vector 1.25 KB + metadata).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InsightEntry {
    /// Content-addressed identifier.
    pub id: InsightId,
    /// Author address (generic bytes — caller picks encoding, e.g. 20-byte EVM address).
    pub author: Vec<u8>,
    /// Knowledge type.
    pub kind: KnowledgeKind,
    /// Human-readable text body.
    pub content: String,
    /// 10,240-bit HDC hypervector projected from the content.
    pub vector: HdcVector,
    /// Entries this one depends on (their content hashes). Enables knowledge graphs.
    pub enabled_by: Vec<InsightId>,
    /// Current lifecycle state.
    pub state: KnowledgeState,
    /// Unix timestamp (seconds) of creation.
    pub created_at: u64,
    /// Half-life in seconds.
    pub half_life_seconds: u64,
    /// Initial weight at post time (typically 1.0).
    pub initial_weight: f32,
    /// Current weight; updated by `KnowledgeStore::apply_decay`.
    pub weight: f32,
    /// Confirmations from distinct agents.
    pub confirmations: Vec<Vec<u8>>,
    /// Open challenges (by challenger address).
    pub challenges: Vec<Vec<u8>>,
    /// Stake committed by the author (wei).
    pub stake_wei: u128,
}

impl InsightEntry {
    /// Constructs a new entry with defaults derived from `kind`.
    ///
    /// `now_secs` is the unix timestamp to record as creation time. The caller
    /// supplies the HDC vector separately (see `chain::projection`).
    #[must_use]
    pub fn new(
        author: Vec<u8>,
        kind: KnowledgeKind,
        content: String,
        vector: HdcVector,
        enabled_by: Vec<InsightId>,
        now_secs: u64,
        stake_wei: u128,
    ) -> Self {
        let id = InsightId::derive(&author, content.as_bytes(), kind);
        Self {
            id,
            author,
            kind,
            content,
            vector,
            enabled_by,
            state: KnowledgeState::Created,
            created_at: now_secs,
            half_life_seconds: kind.default_half_life_seconds(),
            initial_weight: 1.0,
            weight: 1.0,
            confirmations: Vec::new(),
            challenges: Vec::new(),
            stake_wei,
        }
    }

    /// Returns the age of the entry in seconds at the given time.
    #[must_use]
    pub const fn age_seconds(&self, now_secs: u64) -> u64 {
        now_secs.saturating_sub(self.created_at)
    }

    /// Exponential-decay weight: `initial × 2^(-age / half_life)`.
    ///
    /// Matches doc 05 §4.2: `weight(t) = initial × 2^(-age / half_life)`.
    /// Confirmations extend the effective half-life through [`Self::effective_half_life_seconds`].
    #[must_use]
    pub fn decayed_weight(&self, now_secs: u64) -> f32 {
        let tau = self.effective_half_life_seconds() as f32;
        if tau <= 0.0 {
            return 0.0;
        }
        let age = self.age_seconds(now_secs) as f32;
        let decay = (-age / tau * std::f32::consts::LN_2).exp();
        self.initial_weight * decay
    }

    /// Effective half-life in seconds, extended by confirmations.
    ///
    /// Doc 03 §2.2 formula: `tau_eff = tau_base × (1 + sqrt(confirmations) × 2 × 0.5)`
    /// simplified — each confirmation contributes 0.5× of base tau to the extension,
    /// scaled by sqrt to avoid unbounded growth from spam.
    #[must_use]
    pub fn effective_half_life_seconds(&self) -> u64 {
        let base = self.half_life_seconds as f32;
        let confirms = self.confirmations.len() as f32;
        let extension = confirms.sqrt() * 0.5 * base;
        (base + extension).max(0.0) as u64
    }

    /// Records a confirmation from `confirmer`. Returns true if accepted (first time).
    pub fn add_confirmation(&mut self, confirmer: Vec<u8>) -> bool {
        if self.confirmations.iter().any(|a| a == &confirmer) {
            return false;
        }
        self.confirmations.push(confirmer);
        // Boost weight modestly per confirmation (caps via sqrt damping on tau, not here).
        self.weight = (self.weight + 0.10).min(2.0 * self.initial_weight);
        if self.state == KnowledgeState::Active && self.confirmations.len() >= 3 {
            self.state = KnowledgeState::Confirmed;
        }
        true
    }

    /// Records a challenge. Returns true if accepted (not already challenged by this agent).
    pub fn add_challenge(&mut self, challenger: Vec<u8>) -> bool {
        if self.challenges.iter().any(|a| a == &challenger) {
            return false;
        }
        self.challenges.push(challenger);
        if !matches!(
            self.state,
            KnowledgeState::Challenged | KnowledgeState::Pruned
        ) {
            self.state = KnowledgeState::Challenged;
        }
        true
    }

    /// Updates the cached weight field based on elapsed time and current state.
    pub fn refresh_weight(&mut self, now_secs: u64) {
        let w = self.decayed_weight(now_secs);
        self.weight = w;
        // Only non-terminal states may be re-labelled.
        let q = w / self.initial_weight;
        let age = self.age_seconds(now_secs);
        let new_state = if age >= 5 * self.half_life_seconds {
            KnowledgeState::Stale
        } else if self.state == KnowledgeState::Challenged {
            KnowledgeState::Challenged
        } else if q < 0.25 {
            KnowledgeState::Decaying
        } else if self.confirmations.len() >= 3 {
            KnowledgeState::Confirmed
        } else if self.state == KnowledgeState::Created {
            KnowledgeState::Active
        } else {
            self.state
        };
        if self.state.can_transition_to(new_state) || self.state == new_state {
            self.state = new_state;
        }
    }

    /// Returns the set of unique confirmers (deduplication helper).
    #[must_use]
    pub fn confirmer_set(&self) -> HashSet<&[u8]> {
        self.confirmations.iter().map(Vec::as_slice).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_entry(kind: KnowledgeKind, content: &str) -> InsightEntry {
        InsightEntry::new(
            b"author:alice".to_vec(),
            kind,
            content.to_owned(),
            HdcVector::from_seed(content.as_bytes()),
            Vec::new(),
            1_700_000_000,
            2_000_000_000_000_000,
        )
    }

    #[test]
    fn insight_id_is_deterministic() {
        let a = InsightId::derive(
            b"alice",
            b"the router reverts with STF",
            KnowledgeKind::Insight,
        );
        let b = InsightId::derive(
            b"alice",
            b"the router reverts with STF",
            KnowledgeKind::Insight,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn insight_id_depends_on_author_content_and_kind() {
        let base = InsightId::derive(b"alice", b"hello", KnowledgeKind::Insight);
        assert_ne!(
            base,
            InsightId::derive(b"bob", b"hello", KnowledgeKind::Insight)
        );
        assert_ne!(
            base,
            InsightId::derive(b"alice", b"world", KnowledgeKind::Insight)
        );
        assert_ne!(
            base,
            InsightId::derive(b"alice", b"hello", KnowledgeKind::Warning)
        );
    }

    #[test]
    fn half_lives_match_doc_table() {
        assert_eq!(KnowledgeKind::Warning.default_half_life_seconds(), 180);
        assert_eq!(
            KnowledgeKind::Insight.default_half_life_seconds(),
            7 * 86_400
        );
        assert_eq!(
            KnowledgeKind::CausalLink.default_half_life_seconds(),
            15 * 86_400
        );
    }

    #[test]
    fn decayed_weight_halves_at_half_life() {
        let mut entry = mk_entry(KnowledgeKind::Warning, "never selfdestruct in proxy impl");
        let tau = entry.half_life_seconds;
        entry.initial_weight = 1.0;
        let at_half_life = entry.decayed_weight(entry.created_at + tau);
        assert!(
            (at_half_life - 0.5).abs() < 0.01,
            "expected ~0.5 at one half-life, got {at_half_life}"
        );
        let at_two_half_lives = entry.decayed_weight(entry.created_at + 2 * tau);
        assert!((at_two_half_lives - 0.25).abs() < 0.01);
    }

    #[test]
    fn confirmations_extend_effective_half_life() {
        let mut entry = mk_entry(KnowledgeKind::Warning, "check gas on arbitrum");
        let base_tau = entry.effective_half_life_seconds();
        entry.add_confirmation(b"bob".to_vec());
        entry.add_confirmation(b"carol".to_vec());
        entry.add_confirmation(b"dave".to_vec());
        entry.add_confirmation(b"eve".to_vec());
        let extended = entry.effective_half_life_seconds();
        assert!(
            extended > base_tau,
            "4 confirms should extend tau: base={base_tau} ext={extended}"
        );
    }

    #[test]
    fn add_confirmation_dedups() {
        let mut entry = mk_entry(KnowledgeKind::Insight, "eth_call batches");
        assert!(entry.add_confirmation(b"bob".to_vec()));
        assert!(!entry.add_confirmation(b"bob".to_vec()));
        assert_eq!(entry.confirmations.len(), 1);
    }

    #[test]
    fn three_confirmations_move_to_confirmed_via_refresh() {
        let mut entry = mk_entry(KnowledgeKind::Insight, "prefer static calls");
        entry.state = KnowledgeState::Active;
        for who in [&b"b"[..], b"c", b"d"] {
            entry.add_confirmation(who.to_vec());
        }
        entry.refresh_weight(entry.created_at + 60);
        assert_eq!(entry.state, KnowledgeState::Confirmed);
    }

    #[test]
    fn state_machine_rejects_transitions_from_terminal() {
        assert!(!KnowledgeState::Pruned.can_transition_to(KnowledgeState::Active));
        assert!(!KnowledgeState::Stale.can_transition_to(KnowledgeState::Active));
    }

    #[test]
    fn refresh_marks_stale_after_five_half_lives() {
        let mut entry = mk_entry(KnowledgeKind::Warning, "temporary oracle drift");
        entry.state = KnowledgeState::Active;
        let later = entry.created_at + 5 * entry.half_life_seconds + 1;
        entry.refresh_weight(later);
        assert_eq!(entry.state, KnowledgeState::Stale);
    }

    #[test]
    fn challenge_sets_challenged_state() {
        let mut entry = mk_entry(
            KnowledgeKind::AntiKnowledge,
            "WRONG: erc20 needs approve(0)",
        );
        entry.state = KnowledgeState::Active;
        assert!(entry.add_challenge(b"challenger".to_vec()));
        assert_eq!(entry.state, KnowledgeState::Challenged);
        assert!(!entry.add_challenge(b"challenger".to_vec()));
    }
}
