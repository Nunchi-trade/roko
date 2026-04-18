//! Heartbeat attention auction primitives.
//!
//! This module provides a self-contained VCG-style attention auction surface
//! for heartbeat-driven context allocation. It deliberately avoids depending on
//! higher-level domain crates so it can stay inside `roko-runtime`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_const_for_fn,
    clippy::suboptimal_flops,
    clippy::too_many_arguments,
    clippy::unused_self
)]

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::heartbeat::{InferenceTier, PadVector};

/// Logical subsystem identity used by the auction and carryover budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsystemId {
    /// Neuro knowledge and memory candidates.
    Neuro,
    /// Daimon affect and motivational candidates.
    Daimon,
    /// Iteration-memory candidates.
    IterationMemory,
    /// Code-intelligence candidates.
    CodeIntelligence,
    /// Learned playbook rule candidates.
    PlaybookRules,
    /// Research artifact candidates.
    ResearchArtifacts,
    /// Task-context candidates.
    TaskContext,
    /// Oracle prediction candidates.
    OraclePredictions,
}

/// Context category used for bid modulation and deterministic tie-breaking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCategory {
    /// Task-defining context such as goals, requirements, or plans.
    TaskContext,
    /// Safety warnings, risk assessments, and critical invariants.
    Safety,
    /// Historical failures, retries, and fix-up context.
    IterationMemory,
    /// Exploration-oriented or uncertainty-reducing content.
    Exploration,
    /// General knowledge and retrieved facts.
    Knowledge,
    /// Affective or motivational state.
    Affect,
    /// Code symbols, dependencies, and implementation details.
    CodeIntelligence,
    /// Playbook rules and learned heuristics.
    PlaybookRules,
    /// Pre-computed research and literature synthesis.
    ResearchArtifacts,
    /// Prediction and calibration context.
    OraclePredictions,
    /// Catch-all for content that does not fit the main buckets.
    Other,
}

impl ContextCategory {
    /// Returns the category priority used as the last tie-breaker.
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            Self::TaskContext => 100,
            Self::Safety => 90,
            Self::IterationMemory => 80,
            Self::Exploration => 70,
            Self::Knowledge => 60,
            Self::Affect => 50,
            Self::CodeIntelligence => 40,
            Self::PlaybookRules => 30,
            Self::ResearchArtifacts => 20,
            Self::OraclePredictions => 10,
            Self::Other => 0,
        }
    }

    fn affect_multiplier(self, pad: &PadVector) -> f64 {
        match self {
            Self::Safety => 1.0 + f64::from(pad.arousal).abs() * 0.5,
            Self::Exploration | Self::ResearchArtifacts => {
                1.0 + (1.0 - f64::from(pad.dominance)) * 0.3
            }
            Self::IterationMemory => 1.0 + f64::from(-pad.pleasure).max(0.0) * 0.4,
            _ => 1.0,
        }
    }
}

/// A single context candidate submitted to the auction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextCandidate {
    /// The subsystem that produced the candidate.
    pub subsystem_id: SubsystemId,
    /// The category used for affect modulation and tie-breaking.
    pub category: ContextCategory,
    /// Requested token count.
    pub token_count: usize,
    /// Estimated marginal value before modulation.
    pub expected_value: f64,
    /// Urgency multiplier.
    pub urgency: f64,
    /// Brief human-readable summary of the content.
    pub content_summary: String,
}

impl ContextCandidate {
    /// Creates a new context candidate.
    #[must_use]
    pub fn new(
        subsystem_id: SubsystemId,
        category: ContextCategory,
        token_count: usize,
        expected_value: f64,
        urgency: f64,
        content_summary: impl Into<String>,
    ) -> Self {
        Self {
            subsystem_id,
            category,
            token_count,
            expected_value,
            urgency,
            content_summary: content_summary.into(),
        }
    }

    /// Computes the unmodulated bid.
    #[must_use]
    pub fn base_bid(&self) -> f64 {
        self.expected_value * self.urgency
    }

    /// Computes the affect multiplier for the candidate's category.
    #[must_use]
    pub fn affect_multiplier(&self, pad: &PadVector) -> f64 {
        self.category.affect_multiplier(pad)
    }

    /// Computes the final bid after affect modulation.
    #[must_use]
    pub fn final_bid(&self, pad: &PadVector) -> f64 {
        self.base_bid() * self.affect_multiplier(pad)
    }
}

/// A token allocation produced by the auction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextAllocation {
    /// Index of the winning candidate in the original candidate slice.
    pub candidate_idx: usize,
    /// Tokens allocated to the candidate.
    pub tokens_allocated: usize,
    /// Final bid value used for ordering.
    pub bid: f64,
    /// VCG-style payment for the winner.
    pub payment: f64,
}

impl ContextAllocation {
    /// Creates a new allocation record.
    #[must_use]
    pub const fn new(
        candidate_idx: usize,
        tokens_allocated: usize,
        bid: f64,
        payment: f64,
    ) -> Self {
        Self {
            candidate_idx,
            tokens_allocated,
            bid,
            payment,
        }
    }
}

/// A single bid record, including winners and losers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BidResult {
    /// Which subsystem submitted this bid.
    pub subsystem_id: SubsystemId,
    /// The context category of the candidate.
    pub category: ContextCategory,
    /// Raw expected value before modulation.
    pub expected_value: f64,
    /// Urgency multiplier applied.
    pub urgency: f64,
    /// Affect multiplier applied.
    pub affect_multiplier: f64,
    /// Carryover budget multiplier applied.
    pub carryover_multiplier: f64,
    /// Final bid after all multipliers.
    pub final_bid: f64,
    /// VCG payment. Zero for losers.
    pub payment: f64,
    /// Requested tokens.
    pub tokens_requested: usize,
    /// Allocated tokens.
    pub tokens_allocated: usize,
    /// Whether the bid won.
    pub won: bool,
    /// Human-readable description of the content.
    pub content_summary: String,
}

impl BidResult {
    /// Creates a new bid result record.
    #[must_use]
    pub fn new(
        subsystem_id: SubsystemId,
        category: ContextCategory,
        expected_value: f64,
        urgency: f64,
        affect_multiplier: f64,
        carryover_multiplier: f64,
        payment: f64,
        tokens_requested: usize,
        tokens_allocated: usize,
        won: bool,
        content_summary: impl Into<String>,
    ) -> Self {
        let final_bid = expected_value * urgency * affect_multiplier * carryover_multiplier;
        Self {
            subsystem_id,
            category,
            expected_value,
            urgency,
            affect_multiplier,
            carryover_multiplier,
            final_bid,
            payment,
            tokens_requested,
            tokens_allocated,
            won,
            content_summary: content_summary.into(),
        }
    }
}

/// A complete auction round for persistence and diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuctionRound {
    /// Tick identifier.
    pub tick_id: u64,
    /// Timestamp of the auction.
    pub timestamp: DateTime<Utc>,
    /// Tier that triggered the round.
    pub tier: InferenceTier,
    /// Total token budget.
    pub budget_tokens: usize,
    /// PAD vector at auction time.
    pub pad: PadVector,
    /// All bids submitted, including losers.
    pub bids: Vec<BidResult>,
    /// Number of winners.
    pub winners: usize,
    /// Total allocated tokens.
    pub tokens_used: usize,
    /// Tokens left after allocation.
    pub tokens_remaining: usize,
    /// Total candidates seen.
    pub total_candidates: usize,
}

impl AuctionRound {
    /// Creates a new auction round snapshot.
    #[must_use]
    pub fn new(
        tick_id: u64,
        timestamp: DateTime<Utc>,
        tier: InferenceTier,
        budget_tokens: usize,
        pad: PadVector,
        bids: Vec<BidResult>,
    ) -> Self {
        let winners = bids.iter().filter(|bid| bid.won).count();
        let tokens_used = bids.iter().map(|bid| bid.tokens_allocated).sum();
        let tokens_remaining = budget_tokens.saturating_sub(tokens_used);
        let total_candidates = bids.len();

        Self {
            tick_id,
            timestamp,
            tier,
            budget_tokens,
            pad,
            bids,
            winners,
            tokens_used,
            tokens_remaining,
            total_candidates,
        }
    }
}

/// Per-subsystem attention budget tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttentionBudget {
    balances: HashMap<SubsystemId, f64>,
    max_debt: f64,
    decay: f64,
    loser_credit: f64,
}

impl AttentionBudget {
    /// Creates a zeroed budget for the supplied subsystems.
    #[must_use]
    pub fn new(subsystems: &[SubsystemId]) -> Self {
        Self {
            balances: subsystems
                .iter()
                .copied()
                .map(|subsystem| (subsystem, 0.0))
                .collect(),
            max_debt: -5.0,
            decay: 0.95,
            loser_credit: 0.1,
        }
    }

    /// Returns the current balance for a subsystem.
    #[must_use]
    pub fn balance(&self, subsystem: SubsystemId) -> f64 {
        self.balances.get(&subsystem).copied().unwrap_or(0.0)
    }

    /// Applies auction results to the carryover budget.
    pub fn apply_auction_results(
        &mut self,
        results: &[ContextAllocation],
        candidates: &[ContextCandidate],
    ) {
        for alloc in results {
            let subsystem = candidates[alloc.candidate_idx].subsystem_id;
            let entry = self.balances.entry(subsystem).or_insert(0.0);
            *entry = (*entry - alloc.payment).max(self.max_debt);
        }

        let winner_subsystems: HashSet<SubsystemId> = results
            .iter()
            .map(|alloc| candidates[alloc.candidate_idx].subsystem_id)
            .collect();

        for (subsystem, balance) in &mut self.balances {
            if !winner_subsystems.contains(subsystem) {
                *balance += self.loser_credit;
            }
            *balance *= self.decay;
        }
    }

    /// Returns the multiplier used to bias the next round's bids.
    #[must_use]
    pub fn bid_multiplier(&self, subsystem: SubsystemId) -> f64 {
        let balance = self.balance(subsystem).max(self.max_debt);
        if balance >= 0.0 {
            1.0 + balance * 0.1
        } else {
            (1.0 + balance * 0.2).max(0.1)
        }
    }
}

/// Runs the VCG-style attention auction and returns the winning allocations.
///
/// The implementation is intentionally simple: it sorts candidates by final
/// bid, applies a greedy fill, and charges each winner the next excluded bid
/// when the budget is exhausted.
#[must_use]
pub fn run_attention_auction(
    candidates: &[ContextCandidate],
    budget_tokens: usize,
    pad: &PadVector,
) -> Vec<ContextAllocation> {
    let mut scored: Vec<(usize, f64)> = candidates
        .iter()
        .enumerate()
        .map(|(idx, candidate)| (idx, candidate.final_bid(pad)))
        .collect();

    if scored.is_empty() || budget_tokens == 0 {
        return Vec::new();
    }

    let all_non_positive = scored.iter().all(|(_, bid)| *bid <= 0.0);
    if all_non_positive {
        return fallback_allocate_proportionally(candidates, budget_tokens, &scored);
    }

    scored.sort_by(|(a_idx, a_bid), (b_idx, b_bid)| {
        b_bid
            .total_cmp(a_bid)
            .then_with(|| tiebreak(&candidates[*a_idx], &candidates[*b_idx]))
    });

    let mut remaining = budget_tokens;
    let mut winning_positions = Vec::new();

    for (position, (candidate_idx, _bid)) in scored.iter().enumerate() {
        if remaining == 0 {
            break;
        }

        let candidate = &candidates[*candidate_idx];
        let tokens_allocated = candidate.token_count.min(remaining);
        if tokens_allocated == 0 {
            continue;
        }

        winning_positions.push((position, *candidate_idx, tokens_allocated));
        remaining = remaining.saturating_sub(tokens_allocated);
    }

    let budget_exhausted = remaining == 0 && winning_positions.len() < scored.len();
    let mut allocations = Vec::with_capacity(winning_positions.len());

    for (position, candidate_idx, tokens_allocated) in winning_positions {
        let bid = scored[position].1;
        let payment = if budget_exhausted {
            scored
                .get(position + 1)
                .map_or(0.0, |(_, next_bid)| *next_bid)
        } else {
            0.0
        };

        allocations.push(ContextAllocation::new(
            candidate_idx,
            tokens_allocated,
            bid,
            payment,
        ));
    }

    allocations
}

fn tiebreak(a: &ContextCandidate, b: &ContextCandidate) -> Ordering {
    let a_efficiency = a.expected_value / a.token_count.max(1) as f64;
    let b_efficiency = b.expected_value / b.token_count.max(1) as f64;

    b_efficiency
        .total_cmp(&a_efficiency)
        .then_with(|| b.category.priority().cmp(&a.category.priority()))
}

fn fallback_allocate_proportionally(
    candidates: &[ContextCandidate],
    budget_tokens: usize,
    scored: &[(usize, f64)],
) -> Vec<ContextAllocation> {
    if candidates.is_empty() || budget_tokens == 0 {
        return Vec::new();
    }

    let total_requested: usize = candidates
        .iter()
        .map(|candidate| candidate.token_count)
        .sum();
    if total_requested == 0 {
        return Vec::new();
    }

    let mut remaining = budget_tokens;
    let mut allocations = Vec::new();

    for (position, (candidate_idx, _)) in scored.iter().enumerate() {
        let candidate = &candidates[*candidate_idx];
        if candidate.token_count == 0 {
            continue;
        }

        let proportional = budget_tokens.saturating_mul(candidate.token_count) / total_requested;
        let tokens_allocated = proportional.min(candidate.token_count).min(remaining);
        if tokens_allocated == 0 {
            continue;
        }

        allocations.push(ContextAllocation::new(
            *candidate_idx,
            tokens_allocated,
            scored[position].1,
            0.0,
        ));
        remaining = remaining.saturating_sub(tokens_allocated);
    }

    if remaining > 0 {
        for (position, (candidate_idx, _)) in scored.iter().enumerate() {
            if remaining == 0 {
                break;
            }

            let current = allocations
                .iter_mut()
                .find(|allocation| allocation.candidate_idx == *candidate_idx);
            if let Some(allocation) = current {
                let candidate = &candidates[*candidate_idx];
                let headroom = candidate
                    .token_count
                    .saturating_sub(allocation.tokens_allocated);
                let extra = headroom.min(remaining);
                allocation.tokens_allocated += extra;
                remaining -= extra;
                allocation.bid = scored[position].1;
            }
        }
    }

    allocations
}

/// Budget helper for the heartbeat context governor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextGovernor {
    /// Per-tier base budgets.
    pub tier_budgets: HashMap<InferenceTier, usize>,
}

impl ContextGovernor {
    /// Creates a governor from explicit tier budgets.
    #[must_use]
    pub fn new(tier_budgets: HashMap<InferenceTier, usize>) -> Self {
        Self { tier_budgets }
    }

    /// Returns the default heartbeat budgets.
    #[must_use]
    pub fn default_budgets() -> HashMap<InferenceTier, usize> {
        HashMap::from([
            (InferenceTier::T0, 0),
            (InferenceTier::T1, 4_000),
            (InferenceTier::T2, 32_000),
        ])
    }

    /// Returns the budget for a tier.
    #[must_use]
    pub fn budget_for_tier(&self, tier: InferenceTier) -> usize {
        self.tier_budgets.get(&tier).copied().unwrap_or(0)
    }

    /// Returns the budget for a tier after scaling by task complexity.
    ///
    /// Complexity is clamped to `[0.0, 1.0]`, then mapped to a scale factor
    /// in `[0.5, 1.5]`.
    #[must_use]
    pub fn adjusted_budget(&self, tier: InferenceTier, complexity: f64) -> usize {
        self.adjusted_budget_for_complexity(tier, complexity)
    }

    /// Returns the budget for a tier after scaling by task complexity.
    ///
    /// Complexity is clamped to `[0.0, 1.0]`, then mapped to a scale factor
    /// in `[0.5, 1.5]`.
    #[must_use]
    pub fn adjusted_budget_for_complexity(&self, tier: InferenceTier, complexity: f64) -> usize {
        let base = self.budget_for_tier(tier);
        let scale = 0.5 + complexity.clamp(0.0, 1.0);
        let adjusted = (base as f64 * scale) as usize;
        adjusted.min(self.model_limit(tier))
    }

    fn model_limit(&self, tier: InferenceTier) -> usize {
        match tier {
            InferenceTier::T0 => 0,
            InferenceTier::T1 => 8_000,
            InferenceTier::T2 => 128_000,
        }
    }
}

impl Default for ContextGovernor {
    fn default() -> Self {
        Self::new(Self::default_budgets())
    }
}
