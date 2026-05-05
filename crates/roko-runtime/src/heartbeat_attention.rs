//! STATUS: NOT WIRED -- built but no non-test runtime caller.
//!
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
            Self::Safety => 1.0 + pad.arousal.abs() * 0.5,
            Self::Exploration | Self::ResearchArtifacts => 1.0 + (1.0 - pad.dominance) * 0.3,
            Self::IterationMemory => 1.0 + (-pad.pleasure).max(0.0) * 0.4,
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

// ---------------------------------------------------------------------------
// BEAT-07: ContextBidder trait and subsystem implementations
// ---------------------------------------------------------------------------

/// Context state provided to bidders for candidate generation.
///
/// Each subsystem inspects its relevant fields to produce context candidates
/// for the VCG auction.
#[derive(Debug, Clone, Default)]
pub struct BidderContext {
    /// Current PAD affect vector.
    pub pad: PadVector,
    /// Current environmental regime.
    pub regime: u8,
    /// Whether a gate failure occurred in the recent tick.
    pub recent_gate_failure: bool,
    /// Number of consecutive failures on the current task.
    pub consecutive_failures: u32,
    /// Number of retry attempts on the current task.
    pub retry_count: u32,
    /// Deadline pressure in `[0.0, 1.0]` (higher = more urgent).
    pub deadline_pressure: f64,
    /// Task safety relevance in `[0.0, 1.0]`.
    pub safety_relevance: f64,
    /// Task description or summary.
    pub task_summary: String,
    /// Available knowledge entry count from neuro.
    pub knowledge_entry_count: usize,
    /// Available playbook rule count.
    pub playbook_rule_count: usize,
    /// Available research artifact count.
    pub research_artifact_count: usize,
    /// Current prediction accuracy.
    pub prediction_accuracy: f32,
    /// Number of code symbols available.
    pub code_symbol_count: usize,
    // ── INT-17: Technical analysis signals ──────────────────────────────
    /// Build pass rate from recent build history (0.0-1.0).
    /// Fed from `CodingOracle` observations.
    pub build_pass_rate: f64,
    /// Test pass rate from recent test history (0.0-1.0).
    /// Fed from `CodingOracle` observations.
    pub test_pass_rate: f64,
    /// Complexity trend: positive means increasing complexity (higher risk).
    /// Derived from `CodingOracle` complexity observations.
    pub complexity_trend: f64,
    /// Whether a build/test regression was detected (quality degradation).
    pub regression_detected: bool,
}

impl BidderContext {
    /// Compute the urgency multiplier from context signals.
    ///
    /// Urgency is clamped to `[0.5, 2.0]` based on deadline pressure,
    /// retry count, and safety relevance.
    #[must_use]
    pub fn urgency(&self) -> f64 {
        let base = 1.0 + self.deadline_pressure * 0.5;
        let retry_boost = (self.retry_count as f64 * 0.1).min(0.5);
        let safety_boost = self.safety_relevance * 0.3;
        (base + retry_boost + safety_boost).clamp(0.5, 2.0)
    }
}

/// A subsystem that generates context candidates for the VCG attention auction.
///
/// Each of the 8 bidding subsystems implements this trait. On every T1/T2 tick
/// during the COMPOSE step, the `ContextGovernor` invokes `generate_candidates`
/// on all registered bidders and feeds the results into `run_attention_auction`.
pub trait ContextBidder: Send + Sync {
    /// Generate context candidates for this subsystem.
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate>;

    /// The subsystem identifier for this bidder.
    fn subsystem_id(&self) -> SubsystemId;
}

/// Neuro knowledge bidder: bids for durable knowledge entries.
#[derive(Debug, Clone, Copy, Default)]
pub struct NeuroBidder;

impl ContextBidder for NeuroBidder {
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate> {
        if ctx.knowledge_entry_count == 0 {
            return Vec::new();
        }
        let expected_value = 0.6 + (ctx.knowledge_entry_count as f64 * 0.02).min(0.3);
        vec![ContextCandidate::new(
            SubsystemId::Neuro,
            ContextCategory::Knowledge,
            512.min(ctx.knowledge_entry_count * 64),
            expected_value,
            ctx.urgency(),
            "neuro knowledge entries",
        )]
    }

    fn subsystem_id(&self) -> SubsystemId {
        SubsystemId::Neuro
    }
}

/// Daimon affect bidder: bids for affect and motivational state context.
#[derive(Debug, Clone, Copy, Default)]
pub struct DaimonBidder;

impl ContextBidder for DaimonBidder {
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate> {
        let pad_magnitude =
            (ctx.pad.pleasure.powi(2) + ctx.pad.arousal.powi(2) + ctx.pad.dominance.powi(2)).sqrt();
        // Only bid when affect state is noteworthy.
        if pad_magnitude < 0.1 {
            return Vec::new();
        }
        vec![ContextCandidate::new(
            SubsystemId::Daimon,
            ContextCategory::Affect,
            128,
            pad_magnitude.min(1.0),
            ctx.urgency(),
            "daimon affect state",
        )]
    }

    fn subsystem_id(&self) -> SubsystemId {
        SubsystemId::Daimon
    }
}

/// Iteration memory bidder: bids for past failure context.
#[derive(Debug, Clone, Copy, Default)]
pub struct IterationMemoryBidder;

impl ContextBidder for IterationMemoryBidder {
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate> {
        if ctx.consecutive_failures == 0 && !ctx.recent_gate_failure {
            return Vec::new();
        }
        let expected_value = (ctx.consecutive_failures as f64 * 0.2 + 0.4).min(1.0);
        vec![ContextCandidate::new(
            SubsystemId::IterationMemory,
            ContextCategory::IterationMemory,
            256.min(ctx.consecutive_failures as usize * 128 + 128),
            expected_value,
            ctx.urgency(),
            "iteration memory (past failures)",
        )]
    }

    fn subsystem_id(&self) -> SubsystemId {
        SubsystemId::IterationMemory
    }
}

/// Code intelligence bidder: bids for symbol graph context.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodeIntelligenceBidder;

impl ContextBidder for CodeIntelligenceBidder {
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate> {
        if ctx.code_symbol_count == 0 {
            return Vec::new();
        }
        let expected_value = 0.5 + (ctx.code_symbol_count as f64 * 0.01).min(0.4);
        vec![ContextCandidate::new(
            SubsystemId::CodeIntelligence,
            ContextCategory::CodeIntelligence,
            512.min(ctx.code_symbol_count * 32),
            expected_value,
            ctx.urgency(),
            "code intelligence symbols",
        )]
    }

    fn subsystem_id(&self) -> SubsystemId {
        SubsystemId::CodeIntelligence
    }
}

/// Playbook rules bidder: bids for learned heuristic context.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlaybookRulesBidder;

impl ContextBidder for PlaybookRulesBidder {
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate> {
        if ctx.playbook_rule_count == 0 {
            return Vec::new();
        }
        let expected_value = 0.5 + (ctx.playbook_rule_count as f64 * 0.05).min(0.4);
        vec![ContextCandidate::new(
            SubsystemId::PlaybookRules,
            ContextCategory::PlaybookRules,
            256.min(ctx.playbook_rule_count * 64),
            expected_value,
            ctx.urgency(),
            "playbook rules",
        )]
    }

    fn subsystem_id(&self) -> SubsystemId {
        SubsystemId::PlaybookRules
    }
}

/// Research artifacts bidder: bids for analysis and literature context.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResearchArtifactsBidder;

impl ContextBidder for ResearchArtifactsBidder {
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate> {
        if ctx.research_artifact_count == 0 {
            return Vec::new();
        }
        let expected_value = 0.4 + (ctx.research_artifact_count as f64 * 0.03).min(0.4);
        vec![ContextCandidate::new(
            SubsystemId::ResearchArtifacts,
            ContextCategory::ResearchArtifacts,
            384.min(ctx.research_artifact_count * 96),
            expected_value,
            ctx.urgency(),
            "research artifacts",
        )]
    }

    fn subsystem_id(&self) -> SubsystemId {
        SubsystemId::ResearchArtifacts
    }
}

/// Task context bidder: bids for PRD/plan/task description context.
#[derive(Debug, Clone, Copy, Default)]
pub struct TaskContextBidder;

impl ContextBidder for TaskContextBidder {
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate> {
        if ctx.task_summary.is_empty() {
            return Vec::new();
        }
        // Task context is always high-value.
        vec![ContextCandidate::new(
            SubsystemId::TaskContext,
            ContextCategory::TaskContext,
            512,
            0.9,
            ctx.urgency(),
            "task context (PRD/plan)",
        )]
    }

    fn subsystem_id(&self) -> SubsystemId {
        SubsystemId::TaskContext
    }
}

/// Oracle predictions bidder: bids for calibration data and technical
/// analysis context (INT-17).
///
/// In addition to the base prediction accuracy signal, this bidder now
/// considers technical analysis signals from the `CodingOracle`:
/// - Build/test pass rate regression boosts bid value
/// - Rising complexity trend increases the context budget
/// - Detected regression triggers an additional high-priority candidate
#[derive(Debug, Clone, Copy, Default)]
pub struct OraclePredictionsBidder;

impl ContextBidder for OraclePredictionsBidder {
    fn generate_candidates(&self, ctx: &BidderContext) -> Vec<ContextCandidate> {
        let mut candidates = Vec::new();

        // Base calibration candidate: bid when prediction accuracy is low.
        if ctx.prediction_accuracy <= 0.9 {
            let expected_value = (1.0 - ctx.prediction_accuracy as f64).clamp(0.2, 0.8);
            candidates.push(ContextCandidate::new(
                SubsystemId::OraclePredictions,
                ContextCategory::OraclePredictions,
                192,
                expected_value,
                ctx.urgency(),
                "oracle prediction calibration",
            ));
        }

        // INT-17: Technical analysis signals from CodingOracle.
        // Build/test regression: when pass rates drop below 0.8, inject
        // build quality context to help the agent avoid known failure patterns.
        let quality_risk = 1.0 - (ctx.build_pass_rate.min(ctx.test_pass_rate));
        if quality_risk > 0.2 || ctx.regression_detected {
            let regression_value = if ctx.regression_detected {
                0.9 // High priority for detected regressions
            } else {
                quality_risk.clamp(0.3, 0.7)
            };
            // Scale token budget by complexity trend: rising complexity
            // needs more context for the agent to navigate safely.
            let budget = 128 + (ctx.complexity_trend.max(0.0) * 64.0).min(128.0) as usize;
            candidates.push(ContextCandidate::new(
                SubsystemId::OraclePredictions,
                ContextCategory::OraclePredictions,
                budget,
                regression_value,
                ctx.urgency(),
                "technical analysis: build/test quality signals",
            ));
        }

        candidates
    }

    fn subsystem_id(&self) -> SubsystemId {
        SubsystemId::OraclePredictions
    }
}

/// Create the default set of all 8 context bidders.
#[must_use]
pub fn default_bidders() -> Vec<Box<dyn ContextBidder>> {
    vec![
        Box::new(NeuroBidder),
        Box::new(DaimonBidder),
        Box::new(IterationMemoryBidder),
        Box::new(CodeIntelligenceBidder),
        Box::new(PlaybookRulesBidder),
        Box::new(ResearchArtifactsBidder),
        Box::new(TaskContextBidder),
        Box::new(OraclePredictionsBidder),
    ]
}

impl ContextGovernor {
    /// Run the full VCG auction pipeline: collect candidates from all bidders,
    /// adjust budget for task complexity, and return allocations.
    pub fn assemble(
        &self,
        bidders: &[Box<dyn ContextBidder>],
        bidder_ctx: &BidderContext,
        tier: InferenceTier,
        complexity: f64,
        attention_budget: &mut AttentionBudget,
        tick_id: u64,
    ) -> AuctionRound {
        let budget_tokens = self.adjusted_budget(tier, complexity);
        let pad = bidder_ctx.pad;

        // Gather candidates from all bidders.
        let candidates: Vec<ContextCandidate> = bidders
            .iter()
            .flat_map(|bidder| bidder.generate_candidates(bidder_ctx))
            .collect();

        run_attention_auction_with_budget(
            &candidates,
            budget_tokens,
            &pad,
            attention_budget,
            tick_id,
            tier,
        )
    }
}

/// Runs the VCG attention auction with carryover-budget modulation.
///
/// This extends [`run_attention_auction`] by applying per-subsystem budget
/// multipliers from [`AttentionBudget`]. Subsystems that have been losing
/// auctions accumulate credit and bid more aggressively; winners are debited.
///
/// The auction records all bids (winners and losers) in an [`AuctionRound`]
/// for diagnostics and persistence.
pub fn run_attention_auction_with_budget(
    candidates: &[ContextCandidate],
    budget_tokens: usize,
    pad: &PadVector,
    attention_budget: &mut AttentionBudget,
    tick_id: u64,
    tier: InferenceTier,
) -> AuctionRound {
    // Compute modulated bids.
    let mut scored: Vec<(usize, f64, f64, f64)> = candidates
        .iter()
        .enumerate()
        .map(|(idx, candidate)| {
            let affect_mult = candidate.affect_multiplier(pad);
            let carryover_mult = attention_budget.bid_multiplier(candidate.subsystem_id);
            let final_bid = candidate.base_bid() * affect_mult * carryover_mult;
            (idx, final_bid, affect_mult, carryover_mult)
        })
        .collect();

    scored.sort_by(|(a_idx, a_bid, _, _), (b_idx, b_bid, _, _)| {
        b_bid
            .total_cmp(a_bid)
            .then_with(|| tiebreak(&candidates[*a_idx], &candidates[*b_idx]))
    });

    // Greedy allocation.
    let mut remaining = budget_tokens;
    let mut winning_set: HashSet<usize> = HashSet::new();
    let mut allocations = Vec::new();

    for &(candidate_idx, bid, _, _) in &scored {
        if remaining == 0 {
            break;
        }

        let candidate = &candidates[candidate_idx];
        let tokens_allocated = candidate.token_count.min(remaining);
        if tokens_allocated == 0 {
            continue;
        }

        winning_set.insert(candidate_idx);
        remaining = remaining.saturating_sub(tokens_allocated);

        // VCG payment: the next excluded bid.
        let payment = if remaining == 0 {
            scored
                .iter()
                .find(|(idx, _, _, _)| !winning_set.contains(idx))
                .map_or(0.0, |(_, next_bid, _, _)| *next_bid)
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

    // Update carryover budget.
    attention_budget.apply_auction_results(&allocations, candidates);

    // Build bid results for the full round record.
    let bids: Vec<BidResult> = scored
        .iter()
        .map(|&(idx, _final_bid, affect_mult, carryover_mult)| {
            let candidate = &candidates[idx];
            let alloc = allocations.iter().find(|a| a.candidate_idx == idx);
            BidResult::new(
                candidate.subsystem_id,
                candidate.category,
                candidate.expected_value,
                candidate.urgency,
                affect_mult,
                carryover_mult,
                alloc.map_or(0.0, |a| a.payment),
                candidate.token_count,
                alloc.map_or(0, |a| a.tokens_allocated),
                alloc.is_some(),
                candidate.content_summary.clone(),
            )
        })
        .collect();

    AuctionRound::new(tick_id, chrono::Utc::now(), tier, budget_tokens, *pad, bids)
}

// ─── POMDP heartbeat decision ──────────────────────────────────────────

/// Observation signal consumed by the heartbeat POMDP.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatObservation {
    /// Aggregate prediction error from probes.
    pub prediction_error: f32,
    /// Number of anomalous probes.
    pub anomaly_count: u32,
    /// World-model drift.
    pub drift: f32,
    /// Current environmental regime.
    pub regime: crate::heartbeat::Regime,
    /// Whether any gate failure occurred in the last tick.
    pub gate_failure: bool,
}

/// POMDP action space for heartbeat governance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatAction {
    /// Continue at the current tick rate.
    Maintain,
    /// Speed up gamma ticks (higher attention).
    Accelerate,
    /// Slow down gamma ticks (lower attention).
    Decelerate,
    /// Trigger a theta reflective tick early.
    TriggerTheta,
    /// Enter delta consolidation.
    TriggerDelta,
}

/// Belief state for the heartbeat POMDP.
///
/// Uses a simplified 5-state model: [calm, normal, volatile, crisis, recovery]
/// with observation likelihoods derived from probe outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatBelief {
    /// Probability distribution over [calm, normal, volatile, crisis, recovery].
    pub state_probs: [f64; 5],
    /// Number of updates applied.
    pub updates: u64,
}

impl Default for HeartbeatBelief {
    fn default() -> Self {
        Self {
            state_probs: [0.2; 5],
            updates: 0,
        }
    }
}

impl HeartbeatBelief {
    /// Update belief state from a new observation using Bayesian filtering.
    pub fn update(&mut self, obs: &HeartbeatObservation) {
        let likelihoods = observation_likelihoods(obs);
        for (prob, likelihood) in self.state_probs.iter_mut().zip(likelihoods.iter()) {
            *prob *= likelihood;
        }
        self.normalize();
        self.updates += 1;
    }

    /// Select the best heartbeat action by minimizing expected free energy.
    #[must_use]
    pub fn select_action(&self) -> HeartbeatAction {
        let actions = [
            HeartbeatAction::Maintain,
            HeartbeatAction::Accelerate,
            HeartbeatAction::Decelerate,
            HeartbeatAction::TriggerTheta,
            HeartbeatAction::TriggerDelta,
        ];

        actions
            .into_iter()
            .min_by(|a, b| {
                self.expected_free_energy(*a)
                    .partial_cmp(&self.expected_free_energy(*b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(HeartbeatAction::Maintain)
    }

    /// Expected free energy for an action given the current belief.
    fn expected_free_energy(&self, action: HeartbeatAction) -> f64 {
        let [calm, normal, volatile, crisis, recovery] = self.state_probs;

        // Risk: probability of being in a bad state without adequate response.
        let risk = match action {
            HeartbeatAction::Maintain => crisis * 0.8 + volatile * 0.3,
            HeartbeatAction::Accelerate => crisis * 0.2 + volatile * 0.1,
            HeartbeatAction::Decelerate => crisis * 1.0 + volatile * 0.6 + normal * 0.1,
            HeartbeatAction::TriggerTheta => crisis * 0.4 + volatile * 0.2,
            HeartbeatAction::TriggerDelta => crisis * 0.5 + volatile * 0.4,
        };

        // Cost: resource expenditure.
        let cost = match action {
            HeartbeatAction::Maintain => 0.1,
            HeartbeatAction::Accelerate => 0.3,
            HeartbeatAction::Decelerate => 0.05,
            HeartbeatAction::TriggerTheta => 0.2,
            HeartbeatAction::TriggerDelta => 0.15,
        };

        // Efficiency: unnecessary action in calm/recovery states is waste.
        let waste = match action {
            HeartbeatAction::Accelerate => (calm + recovery) * 0.4,
            HeartbeatAction::TriggerTheta => calm * 0.3,
            HeartbeatAction::TriggerDelta => (calm + normal) * 0.2,
            _ => 0.0,
        };

        risk + cost + waste
    }

    fn normalize(&mut self) {
        let total: f64 = self.state_probs.iter().sum();
        if total <= 0.0 || !total.is_finite() {
            self.state_probs = [0.2; 5];
            return;
        }
        for prob in &mut self.state_probs {
            *prob = (*prob / total).clamp(0.0, 1.0);
        }
    }
}

// ─── BEAT-08: Factorized discrete POMDP for tier selection ──────────────

/// Task phase in the factorized POMDP (6 values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPhase {
    /// Gathering requirements and planning approach.
    Planning,
    /// Writing code or producing artifacts.
    Implementing,
    /// Running tests and checks.
    Testing,
    /// Reviewing output quality.
    Reviewing,
    /// Diagnosing and fixing failures.
    Debugging,
    /// Finalizing and publishing.
    Deploying,
}

impl TaskPhase {
    /// All variants for iteration.
    pub const ALL: [Self; 6] = [
        Self::Planning,
        Self::Implementing,
        Self::Testing,
        Self::Reviewing,
        Self::Debugging,
        Self::Deploying,
    ];

    /// Convert to index for matrix addressing.
    pub const fn index(self) -> usize {
        match self {
            Self::Planning => 0,
            Self::Implementing => 1,
            Self::Testing => 2,
            Self::Reviewing => 3,
            Self::Debugging => 4,
            Self::Deploying => 5,
        }
    }

    /// Reconstruct from index.
    pub const fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Planning,
            1 => Self::Implementing,
            2 => Self::Testing,
            3 => Self::Reviewing,
            4 => Self::Debugging,
            _ => Self::Deploying,
        }
    }
}

/// Context quality in the factorized POMDP (5 values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextQuality {
    /// Insufficient context for the task.
    Poor,
    /// Marginal context coverage.
    Fair,
    /// Adequate context for standard execution.
    Good,
    /// Rich context enabling nuanced handling.
    Excellent,
    /// Comprehensive context with full coverage.
    Perfect,
}

impl ContextQuality {
    /// All variants for iteration.
    pub const ALL: [Self; 5] = [
        Self::Poor,
        Self::Fair,
        Self::Good,
        Self::Excellent,
        Self::Perfect,
    ];

    /// Convert to index.
    pub const fn index(self) -> usize {
        match self {
            Self::Poor => 0,
            Self::Fair => 1,
            Self::Good => 2,
            Self::Excellent => 3,
            Self::Perfect => 4,
        }
    }

    /// Reconstruct from index.
    pub const fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Poor,
            1 => Self::Fair,
            2 => Self::Good,
            3 => Self::Excellent,
            _ => Self::Perfect,
        }
    }
}

/// Uncertainty level in the factorized POMDP (3 values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Uncertainty {
    /// Situation is well-understood.
    Low,
    /// Some unknowns remain.
    Medium,
    /// Significant unknowns requiring exploration.
    High,
}

impl Uncertainty {
    /// All variants for iteration.
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    /// Convert to index.
    pub const fn index(self) -> usize {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
        }
    }

    /// Reconstruct from index.
    pub const fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Low,
            1 => Self::Medium,
            _ => Self::High,
        }
    }
}

/// Total state count: 6 x 5 x 3 = 90.
pub const POMDP_STATE_COUNT: usize = 6 * 5 * 3;

/// Number of task phases.
pub const PHASE_COUNT: usize = 6;
/// Number of context quality levels.
pub const QUALITY_COUNT: usize = 5;
/// Number of uncertainty levels.
pub const UNCERTAINTY_COUNT: usize = 3;
/// Number of tiers (T0, T1, T2).
pub const TIER_COUNT: usize = 3;

/// Encode a factorized state triple to a flat index.
#[must_use]
pub const fn encode_state(phase: usize, quality: usize, uncertainty: usize) -> usize {
    phase * QUALITY_COUNT * UNCERTAINTY_COUNT + quality * UNCERTAINTY_COUNT + uncertainty
}

/// Decode a flat index to a factorized state triple.
#[must_use]
pub const fn decode_pomdp_state(idx: usize) -> (usize, usize, usize) {
    let phase = idx / (QUALITY_COUNT * UNCERTAINTY_COUNT);
    let remainder = idx % (QUALITY_COUNT * UNCERTAINTY_COUNT);
    let quality = remainder / UNCERTAINTY_COUNT;
    let uncertainty = remainder % UNCERTAINTY_COUNT;
    (phase, quality, uncertainty)
}

/// POMDP matrices for tier selection.
///
/// A: likelihood — P(observation | state) — how probe results map to states
/// B: transition — P(state' | state, action) — how tier selection changes state
/// C: preferences — desired observations (low error, low cost)
/// D: prior — initial belief distribution
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PomdpMatrices {
    /// Likelihood matrix: `A[state][obs_bin]` — P(obs_bin | state).
    /// Obs bins: [low_error, mid_error, high_error].
    pub a: Vec<[f64; 3]>,
    /// Transition matrices per tier: `B[tier][from_state][to_state]`.
    /// Simplified to self-transition with quality improvement for higher tiers.
    pub b_quality_shift: [[f64; TIER_COUNT]; QUALITY_COUNT],
    /// Preferred observation distribution (lower error is better).
    pub c: [f64; 3],
    /// Prior belief distribution.
    pub d: Vec<f64>,
}

impl Default for PomdpMatrices {
    fn default() -> Self {
        // A: states with higher quality/lower uncertainty -> lower error observations.
        let mut a = vec![[0.0; 3]; POMDP_STATE_COUNT];
        for idx in 0..POMDP_STATE_COUNT {
            let (_phase, quality, uncertainty) = decode_pomdp_state(idx);
            // Quality 0..4 maps to error expectation, uncertainty 0..2 modulates.
            let quality_score = quality as f64 / 4.0; // 0.0 = poor, 1.0 = perfect
            let uncertainty_penalty = uncertainty as f64 * 0.15;
            let p_low = (quality_score - uncertainty_penalty).clamp(0.1, 0.8);
            let p_high = (1.0 - quality_score + uncertainty_penalty * 0.5).clamp(0.1, 0.6);
            let p_mid = (1.0 - p_low - p_high).max(0.1);
            let total = p_low + p_mid + p_high;
            a[idx] = [p_low / total, p_mid / total, p_high / total];
        }

        // B: quality shift probability per tier — higher tiers are more likely to improve quality.
        let b_quality_shift = [
            // quality=Poor: P(improve by tier)
            [0.1, 0.3, 0.6], // T0 rarely improves, T2 often improves
            // quality=Fair
            [0.15, 0.35, 0.55],
            // quality=Good
            [0.2, 0.4, 0.5],
            // quality=Excellent
            [0.3, 0.45, 0.45],
            // quality=Perfect (already best, less room to improve)
            [0.5, 0.5, 0.5],
        ];

        // C: preference for low-error observations.
        let c = [0.8, 0.15, 0.05]; // strongly prefer low error

        // D: uniform prior.
        let d = vec![1.0 / POMDP_STATE_COUNT as f64; POMDP_STATE_COUNT];

        Self {
            a,
            b_quality_shift,
            c,
            d,
        }
    }
}

/// Factorized POMDP belief state over 90 states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorizedBelief {
    /// Probability distribution over the 90 states.
    pub probabilities: Vec<f64>,
    /// Number of updates applied.
    pub updates: u64,
}

impl Default for FactorizedBelief {
    fn default() -> Self {
        Self {
            probabilities: vec![1.0 / POMDP_STATE_COUNT as f64; POMDP_STATE_COUNT],
            updates: 0,
        }
    }
}

impl FactorizedBelief {
    /// Create a belief from a prior.
    #[must_use]
    pub fn from_prior(prior: &[f64]) -> Self {
        let mut b = Self {
            probabilities: prior.to_vec(),
            updates: 0,
        };
        b.normalize();
        b
    }

    /// Update belief from a prediction error observation.
    ///
    /// The observation is binned into [low, mid, high] error and used to
    /// weight the likelihood of each state via the A matrix.
    pub fn update_from_observation(&mut self, prediction_error: f32, matrices: &PomdpMatrices) {
        let obs_bin = if prediction_error < 0.2 {
            0 // low error
        } else if prediction_error < 0.5 {
            1 // mid error
        } else {
            2 // high error
        };

        for (idx, prob) in self.probabilities.iter_mut().enumerate() {
            *prob *= matrices.a[idx][obs_bin];
        }
        self.normalize();
        self.updates += 1;
    }

    /// Select the tier that minimizes expected free energy.
    ///
    /// EFE(tier) = pragmatic_value + epistemic_value
    ///   pragmatic: KL divergence between predicted and preferred observations
    ///   epistemic: expected information gain (uncertainty reduction)
    #[must_use]
    pub fn select_tier(&self, matrices: &PomdpMatrices) -> InferenceTier {
        let tiers = [InferenceTier::T0, InferenceTier::T1, InferenceTier::T2];
        tiers
            .into_iter()
            .min_by(|a, b| {
                self.expected_free_energy(*a, matrices)
                    .partial_cmp(&self.expected_free_energy(*b, matrices))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(InferenceTier::T1)
    }

    /// Compute the expected free energy for a given tier.
    fn expected_free_energy(&self, tier: InferenceTier, matrices: &PomdpMatrices) -> f64 {
        let tier_idx = match tier {
            InferenceTier::T0 => 0,
            InferenceTier::T1 => 1,
            InferenceTier::T2 => 2,
        };

        let mut pragmatic = 0.0;
        let mut epistemic = 0.0;

        // For each state, weight by belief probability.
        for (idx, &prob) in self.probabilities.iter().enumerate() {
            if prob < 1e-12 {
                continue;
            }
            let (_phase, quality, _uncertainty) = decode_pomdp_state(idx);

            // Predicted observation distribution given this tier acts on this state.
            let improvement_prob = matrices.b_quality_shift[quality][tier_idx];
            // After tier action, predict observations.
            let pred_obs = [
                matrices.a[idx][0] * improvement_prob
                    + matrices.a[idx][0] * (1.0 - improvement_prob),
                matrices.a[idx][1],
                matrices.a[idx][2] * (1.0 - improvement_prob * 0.3),
            ];

            // Pragmatic value: KL(preferred || predicted).
            for (obs_bin, &c_pref) in matrices.c.iter().enumerate() {
                if c_pref > 1e-12 && pred_obs[obs_bin] > 1e-12 {
                    pragmatic += prob * c_pref * (c_pref / pred_obs[obs_bin]).ln();
                }
            }

            // Epistemic value: expected entropy of posterior (higher tiers reduce uncertainty).
            let entropy_reduction = match tier {
                InferenceTier::T0 => 0.0,
                InferenceTier::T1 => 0.1,
                InferenceTier::T2 => 0.3,
            };
            epistemic -= prob * entropy_reduction;
        }

        // Cost penalty for higher tiers.
        let cost = match tier {
            InferenceTier::T0 => 0.0,
            InferenceTier::T1 => 0.05,
            InferenceTier::T2 => 0.2,
        };

        pragmatic + epistemic + cost
    }

    /// Normalize the belief distribution.
    fn normalize(&mut self) {
        let total: f64 = self.probabilities.iter().sum();
        if total <= 0.0 || !total.is_finite() {
            self.probabilities.fill(1.0 / POMDP_STATE_COUNT as f64);
            return;
        }
        for p in &mut self.probabilities {
            *p = (*p / total).clamp(0.0, 1.0);
        }
    }

    /// Get the most likely task phase.
    #[must_use]
    pub fn most_likely_phase(&self) -> TaskPhase {
        let mut phase_probs = [0.0_f64; PHASE_COUNT];
        for (idx, &prob) in self.probabilities.iter().enumerate() {
            let (phase, _, _) = decode_pomdp_state(idx);
            phase_probs[phase] += prob;
        }
        let best = phase_probs
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(i, _)| i)
            .unwrap_or(0);
        TaskPhase::from_index(best)
    }

    /// Get the most likely context quality.
    #[must_use]
    pub fn most_likely_quality(&self) -> ContextQuality {
        let mut quality_probs = [0.0_f64; QUALITY_COUNT];
        for (idx, &prob) in self.probabilities.iter().enumerate() {
            let (_, quality, _) = decode_pomdp_state(idx);
            quality_probs[quality] += prob;
        }
        let best = quality_probs
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(i, _)| i)
            .unwrap_or(0);
        ContextQuality::from_index(best)
    }

    /// Get the most likely uncertainty level.
    #[must_use]
    pub fn most_likely_uncertainty(&self) -> Uncertainty {
        let mut uncertainty_probs = [0.0_f64; UNCERTAINTY_COUNT];
        for (idx, &prob) in self.probabilities.iter().enumerate() {
            let (_, _, uncertainty) = decode_pomdp_state(idx);
            uncertainty_probs[uncertainty] += prob;
        }
        let best = uncertainty_probs
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(i, _)| i)
            .unwrap_or(0);
        Uncertainty::from_index(best)
    }
}

/// Compute observation likelihoods for [calm, normal, volatile, crisis, recovery].
fn observation_likelihoods(obs: &HeartbeatObservation) -> [f64; 5] {
    let error = obs.prediction_error as f64;
    let drift = obs.drift as f64;
    let anomalies = obs.anomaly_count as f64;
    let gate_fail = if obs.gate_failure { 1.0 } else { 0.0 };

    // Higher error/drift/anomalies -> more likely volatile/crisis.
    let calm_likelihood = ((1.0 - error) * (1.0 - drift) * (1.0 - gate_fail)).clamp(0.01, 1.0)
        * (1.0 / (1.0 + anomalies));
    let normal_likelihood = (0.7 - error * 0.3).clamp(0.01, 1.0);
    let volatile_likelihood = (error * 0.5 + drift * 0.3 + anomalies * 0.1).clamp(0.01, 1.0);
    let crisis_likelihood =
        (error * 0.7 + drift * 0.5 + gate_fail * 0.5 + anomalies * 0.15).clamp(0.01, 1.0);
    let recovery_likelihood = match obs.regime {
        crate::heartbeat::Regime::Calm => 0.3,
        crate::heartbeat::Regime::Normal => 0.2,
        crate::heartbeat::Regime::Volatile => 0.1,
        crate::heartbeat::Regime::Crisis => 0.05,
    };

    [
        calm_likelihood,
        normal_likelihood,
        volatile_likelihood,
        crisis_likelihood,
        recovery_likelihood,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neutral_pad() -> PadVector {
        PadVector::neutral()
    }

    #[test]
    fn empty_candidates_produce_empty_allocations() {
        let allocations = run_attention_auction(&[], 1000, &neutral_pad());
        assert!(allocations.is_empty());
    }

    #[test]
    fn single_candidate_gets_full_allocation() {
        let candidates = vec![ContextCandidate::new(
            SubsystemId::Neuro,
            ContextCategory::Knowledge,
            500,
            0.8,
            1.0,
            "neuro memory",
        )];
        let allocations = run_attention_auction(&candidates, 1000, &neutral_pad());
        assert_eq!(allocations.len(), 1);
        assert_eq!(allocations[0].tokens_allocated, 500);
        assert_eq!(allocations[0].payment, 0.0);
    }

    #[test]
    fn vcg_payment_equals_excluded_bid() {
        let candidates = vec![
            ContextCandidate::new(
                SubsystemId::Neuro,
                ContextCategory::Knowledge,
                800,
                0.9,
                1.0,
                "neuro",
            ),
            ContextCandidate::new(
                SubsystemId::Daimon,
                ContextCategory::Affect,
                800,
                0.5,
                1.0,
                "daimon",
            ),
        ];
        let allocations = run_attention_auction(&candidates, 800, &neutral_pad());
        assert_eq!(allocations.len(), 1);
        // Winner pays the loser's bid.
        assert!((allocations[0].payment - 0.5).abs() < 1e-9);
    }

    #[test]
    fn carryover_budget_modulates_bids() {
        let subsystems = vec![SubsystemId::Neuro, SubsystemId::Daimon];
        let mut budget = AttentionBudget::new(&subsystems);

        let candidates = vec![
            ContextCandidate::new(
                SubsystemId::Neuro,
                ContextCategory::Knowledge,
                500,
                0.8,
                1.0,
                "neuro",
            ),
            ContextCandidate::new(
                SubsystemId::Daimon,
                ContextCategory::Affect,
                500,
                0.7,
                1.0,
                "daimon",
            ),
        ];

        let round = run_attention_auction_with_budget(
            &candidates,
            1000,
            &neutral_pad(),
            &mut budget,
            1,
            InferenceTier::T1,
        );

        assert_eq!(round.total_candidates, 2);
        assert!(round.winners > 0);
        assert!(round.tokens_used > 0);
    }

    #[test]
    fn attention_budget_tracks_balances() {
        let subsystems = vec![SubsystemId::Neuro, SubsystemId::Daimon];
        let mut budget = AttentionBudget::new(&subsystems);

        assert_eq!(budget.balance(SubsystemId::Neuro), 0.0);
        assert!((budget.bid_multiplier(SubsystemId::Neuro) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn heartbeat_belief_update_shifts_distribution() {
        let mut belief = HeartbeatBelief::default();
        let before = belief.state_probs;

        let obs = HeartbeatObservation {
            prediction_error: 0.8,
            anomaly_count: 3,
            drift: 0.6,
            regime: crate::heartbeat::Regime::Volatile,
            gate_failure: true,
        };
        belief.update(&obs);

        assert_ne!(belief.state_probs, before);
        // Crisis/volatile probability should increase.
        assert!(belief.state_probs[3] > belief.state_probs[0]); // crisis > calm
        assert!((belief.state_probs.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn heartbeat_pomdp_accelerates_in_crisis() {
        let mut belief = HeartbeatBelief::default();
        // Push toward crisis.
        for _ in 0..5 {
            belief.update(&HeartbeatObservation {
                prediction_error: 0.9,
                anomaly_count: 5,
                drift: 0.8,
                regime: crate::heartbeat::Regime::Crisis,
                gate_failure: true,
            });
        }

        let action = belief.select_action();
        assert_eq!(action, HeartbeatAction::Accelerate);
    }

    #[test]
    fn heartbeat_pomdp_decelerates_in_calm() {
        let mut belief = HeartbeatBelief::default();
        // Push toward calm.
        for _ in 0..5 {
            belief.update(&HeartbeatObservation {
                prediction_error: 0.05,
                anomaly_count: 0,
                drift: 0.02,
                regime: crate::heartbeat::Regime::Calm,
                gate_failure: false,
            });
        }

        let action = belief.select_action();
        assert!(
            action == HeartbeatAction::Decelerate || action == HeartbeatAction::Maintain,
            "Expected Decelerate or Maintain in calm, got {action:?}"
        );
    }

    #[test]
    fn context_governor_default_budgets() {
        let governor = ContextGovernor::default();
        assert_eq!(governor.budget_for_tier(InferenceTier::T0), 0);
        assert_eq!(governor.budget_for_tier(InferenceTier::T1), 4_000);
        assert_eq!(governor.budget_for_tier(InferenceTier::T2), 32_000);
    }

    #[test]
    fn context_governor_complexity_scaling() {
        let governor = ContextGovernor::default();
        let low = governor.adjusted_budget(InferenceTier::T1, 0.0);
        let high = governor.adjusted_budget(InferenceTier::T1, 1.0);
        assert!(high > low);
    }

    #[test]
    fn auction_round_snapshot_consistency() {
        let bids = vec![BidResult::new(
            SubsystemId::Neuro,
            ContextCategory::Knowledge,
            0.8,
            1.0,
            1.0,
            1.0,
            0.0,
            500,
            500,
            true,
            "test",
        )];
        let round = AuctionRound::new(
            1,
            chrono::Utc::now(),
            InferenceTier::T1,
            1000,
            PadVector::neutral(),
            bids,
        );
        assert_eq!(round.winners, 1);
        assert_eq!(round.tokens_used, 500);
        assert_eq!(round.tokens_remaining, 500);
    }

    // ----- BEAT-07: ContextBidder trait and subsystem implementations -----

    #[test]
    fn default_bidders_returns_eight_subsystems() {
        let bidders = default_bidders();
        assert_eq!(bidders.len(), 8);
        let ids: Vec<SubsystemId> = bidders.iter().map(|b| b.subsystem_id()).collect();
        assert!(ids.contains(&SubsystemId::Neuro));
        assert!(ids.contains(&SubsystemId::Daimon));
        assert!(ids.contains(&SubsystemId::IterationMemory));
        assert!(ids.contains(&SubsystemId::CodeIntelligence));
        assert!(ids.contains(&SubsystemId::PlaybookRules));
        assert!(ids.contains(&SubsystemId::ResearchArtifacts));
        assert!(ids.contains(&SubsystemId::TaskContext));
        assert!(ids.contains(&SubsystemId::OraclePredictions));
    }

    #[test]
    fn neuro_bidder_generates_candidates_when_entries_exist() {
        let bidder = NeuroBidder;
        let ctx = BidderContext {
            knowledge_entry_count: 5,
            ..Default::default()
        };
        let candidates = bidder.generate_candidates(&ctx);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].subsystem_id, SubsystemId::Neuro);
        assert!(candidates[0].expected_value > 0.0);
    }

    #[test]
    fn neuro_bidder_empty_when_no_entries() {
        let bidder = NeuroBidder;
        let ctx = BidderContext::default();
        let candidates = bidder.generate_candidates(&ctx);
        assert!(candidates.is_empty());
    }

    #[test]
    fn iteration_memory_bidder_activates_on_failures() {
        let bidder = IterationMemoryBidder;
        let ctx = BidderContext {
            consecutive_failures: 3,
            ..Default::default()
        };
        let candidates = bidder.generate_candidates(&ctx);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].subsystem_id, SubsystemId::IterationMemory);
    }

    #[test]
    fn iteration_memory_bidder_silent_when_no_failures() {
        let bidder = IterationMemoryBidder;
        let ctx = BidderContext::default();
        let candidates = bidder.generate_candidates(&ctx);
        assert!(candidates.is_empty());
    }

    #[test]
    fn task_context_bidder_requires_task_summary() {
        let bidder = TaskContextBidder;
        let no_task = BidderContext::default();
        assert!(bidder.generate_candidates(&no_task).is_empty());

        let with_task = BidderContext {
            task_summary: "implement feature X".into(),
            ..Default::default()
        };
        let candidates = bidder.generate_candidates(&with_task);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].subsystem_id, SubsystemId::TaskContext);
    }

    #[test]
    fn oracle_bidder_bids_when_accuracy_low() {
        let bidder = OraclePredictionsBidder;
        // Low accuracy + healthy build/test rates => only calibration candidate.
        let low_acc = BidderContext {
            prediction_accuracy: 0.5,
            build_pass_rate: 1.0,
            test_pass_rate: 1.0,
            ..Default::default()
        };
        let candidates = bidder.generate_candidates(&low_acc);
        assert_eq!(candidates.len(), 1);

        // High accuracy + healthy rates => no candidates.
        let high_acc = BidderContext {
            prediction_accuracy: 0.95,
            build_pass_rate: 1.0,
            test_pass_rate: 1.0,
            ..Default::default()
        };
        assert!(bidder.generate_candidates(&high_acc).is_empty());
    }

    #[test]
    fn oracle_bidder_adds_tech_analysis_on_regression() {
        let bidder = OraclePredictionsBidder;
        // High accuracy but regression detected => tech analysis candidate.
        let regression = BidderContext {
            prediction_accuracy: 0.95,
            build_pass_rate: 0.6,
            test_pass_rate: 0.9,
            regression_detected: true,
            ..Default::default()
        };
        let candidates = bidder.generate_candidates(&regression);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].content_summary.contains("technical analysis"));
    }

    #[test]
    fn oracle_bidder_both_calibration_and_tech_analysis() {
        let bidder = OraclePredictionsBidder;
        // Low accuracy AND low build rate => both candidates.
        let both = BidderContext {
            prediction_accuracy: 0.5,
            build_pass_rate: 0.5,
            test_pass_rate: 0.9,
            ..Default::default()
        };
        let candidates = bidder.generate_candidates(&both);
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn bidder_context_urgency_clamps_correctly() {
        let low = BidderContext::default();
        assert!(low.urgency() >= 0.5 && low.urgency() <= 2.0);

        let high = BidderContext {
            deadline_pressure: 1.0,
            retry_count: 10,
            safety_relevance: 1.0,
            ..Default::default()
        };
        assert!((high.urgency() - 2.0).abs() < 1e-10);
    }

    #[test]
    fn governor_assemble_runs_full_pipeline() {
        let governor = ContextGovernor::default();
        let bidders = default_bidders();
        let ctx = BidderContext {
            knowledge_entry_count: 5,
            task_summary: "test task".into(),
            prediction_accuracy: 0.6,
            ..Default::default()
        };
        let subsystems: Vec<SubsystemId> = bidders.iter().map(|b| b.subsystem_id()).collect();
        let budget = &mut AttentionBudget::new(&subsystems);

        let round = governor.assemble(&bidders, &ctx, InferenceTier::T1, 0.5, budget, 1);

        assert!(round.total_candidates > 0);
        assert!(round.winners > 0);
        assert!(round.budget_tokens > 0);
    }

    // ----- BEAT-08: Factorized POMDP state space -----

    #[test]
    fn pomdp_state_count_is_90() {
        assert_eq!(POMDP_STATE_COUNT, 90);
        assert_eq!(PHASE_COUNT * QUALITY_COUNT * UNCERTAINTY_COUNT, 90);
    }

    #[test]
    fn encode_decode_roundtrip() {
        for phase in 0..PHASE_COUNT {
            for quality in 0..QUALITY_COUNT {
                for uncertainty in 0..UNCERTAINTY_COUNT {
                    let idx = encode_state(phase, quality, uncertainty);
                    assert!(idx < POMDP_STATE_COUNT);
                    let (p, q, u) = decode_pomdp_state(idx);
                    assert_eq!((p, q, u), (phase, quality, uncertainty));
                }
            }
        }
    }

    #[test]
    fn factorized_belief_initializes_uniform() {
        let belief = FactorizedBelief::default();
        assert_eq!(belief.probabilities.len(), POMDP_STATE_COUNT);
        let sum: f64 = belief.probabilities.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-9,
            "belief should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn factorized_belief_update_shifts_distribution() {
        let mut belief = FactorizedBelief::default();
        let matrices = PomdpMatrices::default();

        let before = belief.probabilities.clone();
        belief.update_from_observation(0.8, &matrices); // high error
        assert_ne!(belief.probabilities, before);

        let sum: f64 = belief.probabilities.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-9,
            "belief should remain normalized, got {sum}"
        );
    }

    #[test]
    fn factorized_belief_high_error_shifts_toward_poor_quality() {
        let mut belief = FactorizedBelief::default();
        let matrices = PomdpMatrices::default();

        // Push with many high-error observations.
        for _ in 0..10 {
            belief.update_from_observation(0.9, &matrices);
        }

        // Poor/Fair quality states should have more mass than Perfect.
        let mut poor_mass = 0.0;
        let mut perfect_mass = 0.0;
        for (idx, &prob) in belief.probabilities.iter().enumerate() {
            let (_, quality, _) = decode_pomdp_state(idx);
            if quality == 0 {
                poor_mass += prob;
            }
            if quality == 4 {
                perfect_mass += prob;
            }
        }
        assert!(
            poor_mass > perfect_mass,
            "poor quality should dominate after high errors: poor={poor_mass} > perfect={perfect_mass}"
        );
    }

    #[test]
    fn factorized_belief_selects_higher_tier_with_high_error() {
        let mut belief = FactorizedBelief::default();
        let matrices = PomdpMatrices::default();

        for _ in 0..10 {
            belief.update_from_observation(0.9, &matrices);
        }

        let tier = belief.select_tier(&matrices);
        // With consistently high error, should escalate to T1 or T2.
        assert!(
            tier == InferenceTier::T1 || tier == InferenceTier::T2,
            "expected T1 or T2 for high error, got {tier:?}"
        );
    }

    #[test]
    fn factorized_belief_prefers_lower_tier_with_low_error() {
        let mut belief_low = FactorizedBelief::default();
        let mut belief_high = FactorizedBelief::default();
        let matrices = PomdpMatrices::default();

        for _ in 0..10 {
            belief_low.update_from_observation(0.05, &matrices);
            belief_high.update_from_observation(0.9, &matrices);
        }

        let tier_low = belief_low.select_tier(&matrices);
        let tier_high = belief_high.select_tier(&matrices);
        // Low error should select a tier no higher than high error.
        assert!(
            u8::from(tier_low) <= u8::from(tier_high),
            "low error tier {tier_low:?} should be <= high error tier {tier_high:?}"
        );
    }

    #[test]
    fn factorized_belief_marginal_queries_work() {
        let belief = FactorizedBelief::default();
        // With uniform prior, most likely should be deterministic.
        let _phase = belief.most_likely_phase();
        let _quality = belief.most_likely_quality();
        let _uncertainty = belief.most_likely_uncertainty();
        // Just check they don't panic.
    }

    #[test]
    fn pomdp_matrices_a_rows_sum_to_one() {
        let matrices = PomdpMatrices::default();
        for (idx, row) in matrices.a.iter().enumerate() {
            let sum: f64 = row.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-6,
                "A matrix row {idx} should sum to 1.0, got {sum}"
            );
        }
    }

    #[test]
    fn pomdp_matrices_c_sums_to_one() {
        let matrices = PomdpMatrices::default();
        let sum: f64 = matrices.c.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "C preferences should sum to 1.0, got {sum}"
        );
    }
}
