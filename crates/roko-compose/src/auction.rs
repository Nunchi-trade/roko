//! Auction-learning and diagnostics helpers for prompt-budget allocation.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::AttentionBidder;

/// Canonical subsystem identifier for prompt-budget bidding.
pub type SubsystemId = AttentionBidder;

/// Historical bid pairs for one subsystem pair.
pub type BidHistoryPairs = Vec<(f64, f64)>;

/// One bidder-pair history entry used for correlation analysis.
pub type BidHistoryEntry = (SubsystemId, SubsystemId, BidHistoryPairs);

/// One section's allocation data inside an auction or fairness pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SectionAllocation {
    /// Human-readable section name.
    pub name: String,
    /// Bid or welfare value assigned to the section.
    pub value: f64,
    /// Token cost for including the section.
    pub tokens: usize,
}

/// A bidder that learns per-section value from past inclusion outcomes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LearningBidder {
    /// Subsystem this bidder represents.
    pub subsystem_id: SubsystemId,
    /// Beta-posterior parameters keyed by section name.
    pub section_betas: HashMap<String, (f64, f64)>,
    /// Cost-effectiveness observations keyed by section name.
    #[serde(default)]
    pub section_costs: HashMap<String, SectionCostStats>,
    /// Prior value used before any observations are recorded.
    pub prior_bid: f64,
}

/// Accumulated cost statistics for one section within a [`LearningBidder`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SectionCostStats {
    /// Total attributed cost across included turns.
    pub total_cost_usd: f64,
    /// Total estimated prompt tokens across included turns.
    pub total_tokens: usize,
    /// Included-turn observations carrying cost data.
    pub observation_count: u32,
    /// Number of included observations whose downstream gate passed.
    pub passes: u32,
}

impl LearningBidder {
    /// Create a learning bidder with a prior bid weight.
    #[must_use]
    pub fn new(subsystem_id: SubsystemId, prior_bid: f64) -> Self {
        Self {
            subsystem_id,
            section_betas: HashMap::new(),
            section_costs: HashMap::new(),
            prior_bid,
        }
    }

    /// Compute the current bid for a section.
    ///
    /// This uses a dependency-free Thompson-style approximation: posterior mean
    /// plus a deterministic exploration offset derived from posterior variance.
    #[must_use]
    pub fn bid(&self, section_name: &str, relevance: f64) -> f64 {
        let (alpha, beta) = self
            .section_betas
            .get(section_name)
            .copied()
            .unwrap_or((1.0, 1.0));
        let sampled_track_record = thompson_like_sample(section_name, alpha, beta);
        sampled_track_record * relevance.max(0.0) * self.prior_bid.max(0.0)
    }

    /// Compute a bid that incorporates historical cost-effectiveness.
    #[must_use]
    pub fn bid_with_cost(&self, section_name: &str, relevance: f64) -> f64 {
        self.bid(section_name, relevance) * self.cost_effectiveness_factor(section_name)
    }

    /// Update the posterior after observing one task outcome.
    pub fn update(&mut self, section_name: &str, was_included: bool, gate_passed: bool) {
        if !was_included {
            return;
        }

        let entry = self
            .section_betas
            .entry(section_name.to_string())
            .or_insert((1.0, 1.0));
        if gate_passed {
            entry.0 += 1.0;
        } else {
            entry.1 += 1.0;
        }
    }

    /// Update posterior and attributed-cost statistics after one observed turn.
    pub fn update_with_cost(
        &mut self,
        section_name: &str,
        was_included: bool,
        gate_passed: bool,
        attributed_cost_usd: f64,
        estimated_tokens: usize,
    ) {
        self.update(section_name, was_included, gate_passed);
        if !was_included || estimated_tokens == 0 {
            return;
        }

        let entry = self
            .section_costs
            .entry(section_name.to_string())
            .or_default();
        entry.total_cost_usd += attributed_cost_usd.max(0.0);
        entry.total_tokens = entry.total_tokens.saturating_add(estimated_tokens);
        entry.observation_count = entry.observation_count.saturating_add(1);
        if gate_passed {
            entry.passes = entry.passes.saturating_add(1);
        }
    }

    /// Minimum observed outcomes across sections known to this bidder.
    #[must_use]
    pub fn observation_count(&self) -> u32 {
        let beta_min = self
            .section_betas
            .values()
            .map(|(alpha, beta)| ((*alpha + *beta - 2.0).max(0.0)) as u32)
            .min();
        let cost_min = self
            .section_costs
            .values()
            .map(|stats| stats.observation_count)
            .min();

        match (beta_min, cost_min) {
            (Some(beta), Some(cost)) => beta.min(cost),
            (Some(beta), None) => beta,
            (None, Some(cost)) => cost,
            (None, None) => 0,
        }
    }

    fn cost_effectiveness_factor(&self, section_name: &str) -> f64 {
        let Some(stats) = self.section_costs.get(section_name) else {
            return 1.0;
        };
        if stats.observation_count < 3 || stats.total_tokens == 0 {
            return 1.0;
        }

        let pass_rate = stats.passes as f64 / stats.observation_count as f64;
        let cost_per_1k_tokens = stats.total_cost_usd.max(0.0) / stats.total_tokens as f64 * 1000.0;
        let cost_efficiency = 1.0 / (1.0 + cost_per_1k_tokens);
        let quality = (0.7 * pass_rate + 0.3 * cost_efficiency).clamp(0.0, 1.0);

        (0.5 + quality * 1.5).clamp(0.5, 2.0)
    }
}

/// Correlation-oriented diagnostics for one completed auction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuctionDiagnostics {
    /// Total welfare captured by winning sections.
    pub total_welfare: f64,
    /// Sum of diagnostic VCG-style payments.
    pub total_payments: f64,
    /// Estimated welfare loss relative to a stronger allocation baseline.
    pub welfare_loss: f64,
    /// Whether the allocation is Pareto optimal under the simple swap check.
    pub pareto_optimal: bool,
    /// Highest payment sections and their payment values.
    pub highest_payment_sections: Vec<(String, f64)>,
    /// Excluded sections displaced by budget pressure.
    pub displaced_sections: Vec<(String, f64)>,
    /// Fraction of the available token budget that was consumed.
    pub budget_utilization: f64,
}

/// Configurable fairness policy for auction-based budget allocation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FairnessConfig {
    /// Alpha parameter from the alpha-fairness family.
    pub alpha: f64,
    /// Minimum token floor reserved for safety-sensitive context.
    pub safety_floor_tokens: usize,
}

impl Default for FairnessConfig {
    fn default() -> Self {
        Self {
            alpha: 0.0,
            safety_floor_tokens: 200,
        }
    }
}

/// Detect strongly correlated bidder pairs that may indicate structural coupling.
#[must_use]
pub fn detect_bid_correlation(
    bid_history: &[BidHistoryEntry],
    threshold: f64,
) -> Vec<(SubsystemId, SubsystemId, f64)> {
    bid_history
        .iter()
        .filter_map(|(left, right, pairs)| {
            let correlation = pearson_correlation(pairs)?;
            (correlation > threshold).then_some((*left, *right, correlation))
        })
        .collect()
}

/// Check the simple Pareto-optimality condition described in the docs.
#[must_use]
pub fn is_pareto_optimal(
    included: &[SectionAllocation],
    excluded: &[SectionAllocation],
    budget_remaining: usize,
) -> bool {
    for excluded_section in excluded {
        if excluded_section.tokens <= budget_remaining {
            return false;
        }
        for included_section in included {
            if included_section.value < excluded_section.value
                && included_section.tokens >= excluded_section.tokens
            {
                return false;
            }
        }
    }
    true
}

fn thompson_like_sample(section_name: &str, alpha: f64, beta: f64) -> f64 {
    let total = (alpha + beta).max(f64::EPSILON);
    let mean = alpha / total;
    let variance = (alpha * beta) / (total.powi(2) * (total + 1.0)).max(f64::EPSILON);
    let spread = variance.sqrt();
    let centered_unit = hash_to_unit(section_name) * 2.0 - 1.0;
    (mean + centered_unit * spread).clamp(0.0, 1.0)
}

fn hash_to_unit(text: &str) -> f64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish() as f64 / u64::MAX as f64
}

fn pearson_correlation(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.len() < 2 {
        return None;
    }

    let len = pairs.len() as f64;
    let sum_left = pairs.iter().map(|(left, _)| left).sum::<f64>();
    let sum_right = pairs.iter().map(|(_, right)| right).sum::<f64>();
    let mean_left = sum_left / len;
    let mean_right = sum_right / len;

    let mut numerator = 0.0;
    let mut left_sq = 0.0;
    let mut right_sq = 0.0;
    for (left, right) in pairs {
        let left_centered = left - mean_left;
        let right_centered = right - mean_right;
        numerator += left_centered * right_centered;
        left_sq += left_centered.powi(2);
        right_sq += right_centered.powi(2);
    }

    let denominator = (left_sq * right_sq).sqrt();
    (denominator > 0.0).then_some((numerator / denominator).clamp(-1.0, 1.0))
}

// ---------------------------------------------------------------------------
// DAIM-09: VCG auction with affect modulation.
// ---------------------------------------------------------------------------

/// PAD-derived modulation parameters for VCG-style allocation.
///
/// Per the spec (doc 10), the Daimon modulates bids through:
/// - `urgency_multiplier`: arousal increases urgency (high arousal => more context)
/// - `affect_weight`: pleasure biases toward positive/negative-valence entries
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AffectModulation {
    /// Arousal-derived urgency multiplier (default 1.0, range [0.5, 2.0]).
    pub urgency_multiplier: f64,
    /// Pleasure-derived valence bias (range [-1.0, 1.0]).
    pub affect_weight: f64,
}

impl Default for AffectModulation {
    fn default() -> Self {
        Self {
            urgency_multiplier: 1.0,
            affect_weight: 0.0,
        }
    }
}

impl AffectModulation {
    /// Derive modulation parameters from a PAD state.
    ///
    /// - Arousal maps to urgency: `1.0 + arousal * 0.5` clamped to `[0.5, 2.0]`
    /// - Pleasure maps to affect weight directly.
    #[must_use]
    pub fn from_pad(pleasure: f64, arousal: f64) -> Self {
        Self {
            urgency_multiplier: (1.0 + arousal * 0.5).clamp(0.5, 2.0),
            affect_weight: pleasure.clamp(-1.0, 1.0),
        }
    }

    /// Adjust a base bid using the affect modulation formula.
    ///
    /// `adjusted_bid = base_bid * urgency_multiplier * (1 + affect_weight * valence)`
    ///
    /// where `valence` is the entry's emotional valence in `[-1.0, 1.0]`.
    #[must_use]
    pub fn adjust_bid(&self, base_bid: f64, entry_valence: f64) -> f64 {
        let valence = entry_valence.clamp(-1.0, 1.0);
        base_bid * self.urgency_multiplier * (1.0 + self.affect_weight * valence)
    }
}

/// One subsystem's bid in the VCG auction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VcgBid {
    /// Subsystem that placed the bid.
    pub bidder: SubsystemId,
    /// Section name.
    pub section_name: String,
    /// Token cost for this section.
    pub tokens: usize,
    /// Raw bid value before affect modulation.
    pub raw_bid: f64,
    /// Affect-adjusted bid value.
    pub adjusted_bid: f64,
    /// Emotional valence of the content (if applicable).
    pub valence: f64,
}

/// Result of a VCG-style allocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VcgAllocation {
    /// Sections that won the auction, sorted by adjusted bid descending.
    pub winners: Vec<VcgBid>,
    /// Sections excluded due to budget constraints.
    pub excluded: Vec<VcgBid>,
    /// VCG-style payments for each winner (second-price clearing).
    pub payments: Vec<(String, f64)>,
    /// Total tokens allocated.
    pub total_tokens_used: usize,
    /// Budget utilization fraction.
    pub budget_utilization: f64,
    /// Diagnostics.
    pub diagnostics: AuctionDiagnostics,
}

/// Allocate context window tokens using a greedy VCG-style mechanism.
///
/// Bids are sorted by `adjusted_bid / tokens` (value density). Sections
/// are included greedily until the budget is exhausted. VCG payments are
/// computed as the externality each winner imposes on others: the payment
/// for winner `i` is the total welfare of others *without* `i` minus the
/// total welfare of others *with* `i`.
#[must_use]
pub fn vcg_allocate(
    bids: Vec<VcgBid>,
    total_budget: usize,
    modulation: &AffectModulation,
) -> VcgAllocation {
    if bids.is_empty() || total_budget == 0 {
        return VcgAllocation {
            winners: Vec::new(),
            excluded: bids,
            payments: Vec::new(),
            total_tokens_used: 0,
            budget_utilization: 0.0,
            diagnostics: AuctionDiagnostics {
                total_welfare: 0.0,
                total_payments: 0.0,
                welfare_loss: 0.0,
                pareto_optimal: true,
                highest_payment_sections: Vec::new(),
                displaced_sections: Vec::new(),
                budget_utilization: 0.0,
            },
        };
    }

    let _ = modulation; // Already applied to adjusted_bid

    // Sort by value density (adjusted_bid / tokens), descending.
    let mut sorted = bids;
    sorted.sort_by(|a, b| {
        let density_a = if a.tokens > 0 {
            a.adjusted_bid / a.tokens as f64
        } else {
            f64::INFINITY
        };
        let density_b = if b.tokens > 0 {
            b.adjusted_bid / b.tokens as f64
        } else {
            f64::INFINITY
        };
        density_b
            .partial_cmp(&density_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Greedy allocation.
    let mut remaining = total_budget;
    let mut winners = Vec::new();
    let mut excluded = Vec::new();
    for bid in sorted {
        if bid.tokens <= remaining {
            remaining -= bid.tokens;
            winners.push(bid);
        } else {
            excluded.push(bid);
        }
    }

    let total_used = total_budget - remaining;
    let total_welfare: f64 = winners.iter().map(|b| b.adjusted_bid).sum();

    // Compute VCG payments: for each winner, payment = externality imposed.
    // Simplified: payment = highest excluded bid that would have fit.
    let mut payments = Vec::new();
    for winner in &winners {
        let payment = excluded
            .iter()
            .filter(|e| e.tokens <= winner.tokens)
            .map(|e| e.adjusted_bid)
            .fold(0.0_f64, f64::max);
        payments.push((winner.section_name.clone(), payment));
    }

    let total_payments: f64 = payments.iter().map(|(_, p)| *p).sum();
    let budget_utilization = if total_budget > 0 {
        total_used as f64 / total_budget as f64
    } else {
        0.0
    };

    let included_allocs: Vec<_> = winners
        .iter()
        .map(|b| SectionAllocation {
            name: b.section_name.clone(),
            value: b.adjusted_bid,
            tokens: b.tokens,
        })
        .collect();
    let excluded_allocs: Vec<_> = excluded
        .iter()
        .map(|b| SectionAllocation {
            name: b.section_name.clone(),
            value: b.adjusted_bid,
            tokens: b.tokens,
        })
        .collect();
    let pareto = is_pareto_optimal(&included_allocs, &excluded_allocs, remaining);
    let welfare_loss: f64 = excluded.iter().map(|e| e.adjusted_bid).sum();

    let highest_payment_sections: Vec<_> =
        payments.iter().filter(|(_, p)| *p > 0.0).cloned().collect();
    let displaced_sections: Vec<_> = excluded
        .iter()
        .map(|b| (b.section_name.clone(), b.adjusted_bid))
        .collect();

    VcgAllocation {
        winners,
        excluded,
        payments,
        total_tokens_used: total_used,
        budget_utilization,
        diagnostics: AuctionDiagnostics {
            total_welfare,
            total_payments,
            welfare_loss,
            pareto_optimal: pareto,
            highest_payment_sections,
            displaced_sections,
            budget_utilization,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_bidder_updates_posterior() {
        let mut bidder = LearningBidder::new(SubsystemId::TaskContext, 1.0);
        let before = bidder.bid("task_context", 0.8);
        bidder.update("task_context", true, true);
        let after = bidder.bid("task_context", 0.8);

        assert!(after >= before);
    }

    #[test]
    fn learning_bidder_cost_factor_is_neutral_without_data() {
        let bidder = LearningBidder::new(SubsystemId::TaskContext, 1.0);

        assert_eq!(
            bidder.bid_with_cost("task_context", 0.8),
            bidder.bid("task_context", 0.8)
        );
    }

    #[test]
    fn learning_bidder_rewards_cheap_effective_sections() {
        let mut bidder = LearningBidder::new(SubsystemId::TaskContext, 1.0);
        for _ in 0..5 {
            bidder.update_with_cost("task_context", true, true, 0.0001, 400);
        }

        assert!(bidder.bid_with_cost("task_context", 0.8) > bidder.bid("task_context", 0.8));
        assert_eq!(bidder.observation_count(), 5);
    }

    #[test]
    fn learning_bidder_penalizes_expensive_ineffective_sections() {
        let mut bidder = LearningBidder::new(SubsystemId::TaskContext, 1.0);
        for _ in 0..5 {
            bidder.update_with_cost("task_context", true, false, 2.0, 100);
        }

        assert!(bidder.bid_with_cost("task_context", 0.8) < bidder.bid("task_context", 0.8));
    }

    #[test]
    fn detects_high_correlation() {
        let correlated = detect_bid_correlation(
            &[(
                SubsystemId::Neuro,
                SubsystemId::Research,
                vec![(0.1, 0.11), (0.2, 0.19), (0.3, 0.31), (0.4, 0.39)],
            )],
            0.85,
        );

        assert_eq!(correlated.len(), 1);
    }

    #[test]
    fn pareto_check_rejects_improvable_allocation() {
        let included = vec![SectionAllocation {
            name: "low".into(),
            value: 0.2,
            tokens: 300,
        }];
        let excluded = vec![SectionAllocation {
            name: "high".into(),
            value: 0.9,
            tokens: 200,
        }];

        assert!(!is_pareto_optimal(&included, &excluded, 0));
    }

    // -------------------------------------------------------------------
    // DAIM-09: VCG auction with affect modulation tests
    // -------------------------------------------------------------------

    #[test]
    fn affect_modulation_from_pad() {
        let modulation = AffectModulation::from_pad(0.5, 0.8);
        assert!(
            modulation.urgency_multiplier > 1.0,
            "high arousal => high urgency"
        );
        assert!(
            modulation.affect_weight > 0.0,
            "positive pleasure => positive affect weight"
        );
    }

    #[test]
    fn affect_modulation_adjusts_bid() {
        let modulation = AffectModulation::from_pad(0.6, 0.0);
        let base = 1.0;
        let positive_entry = modulation.adjust_bid(base, 0.8);
        let negative_entry = modulation.adjust_bid(base, -0.8);
        assert!(
            positive_entry > negative_entry,
            "positive pleasure should prefer positive valence"
        );
    }

    #[test]
    fn vcg_allocate_basic() {
        let bids = vec![
            VcgBid {
                bidder: SubsystemId::Neuro,
                section_name: "knowledge".into(),
                tokens: 500,
                raw_bid: 0.8,
                adjusted_bid: 0.8,
                valence: 0.0,
            },
            VcgBid {
                bidder: SubsystemId::TaskContext,
                section_name: "task".into(),
                tokens: 300,
                raw_bid: 0.6,
                adjusted_bid: 0.6,
                valence: 0.0,
            },
            VcgBid {
                bidder: SubsystemId::Research,
                section_name: "research".into(),
                tokens: 400,
                raw_bid: 0.4,
                adjusted_bid: 0.4,
                valence: 0.0,
            },
        ];

        let allocation = vcg_allocate(bids, 800, &AffectModulation::default());
        assert_eq!(
            allocation.winners.len(),
            2,
            "should fit 2 of 3 bids in 800 tokens"
        );
        assert_eq!(allocation.excluded.len(), 1);
        assert!(allocation.total_tokens_used <= 800);
        assert!(allocation.budget_utilization > 0.0);
    }

    #[test]
    fn vcg_allocate_empty_budget() {
        let bids = vec![VcgBid {
            bidder: SubsystemId::Neuro,
            section_name: "knowledge".into(),
            tokens: 500,
            raw_bid: 0.8,
            adjusted_bid: 0.8,
            valence: 0.0,
        }];

        let allocation = vcg_allocate(bids, 0, &AffectModulation::default());
        assert!(allocation.winners.is_empty());
    }

    #[test]
    fn vcg_allocate_value_density_wins() {
        let bids = vec![
            VcgBid {
                bidder: SubsystemId::Neuro,
                section_name: "expensive".into(),
                tokens: 900,
                raw_bid: 0.5,
                adjusted_bid: 0.5,
                valence: 0.0,
            },
            VcgBid {
                bidder: SubsystemId::TaskContext,
                section_name: "cheap_high_value".into(),
                tokens: 100,
                raw_bid: 0.9,
                adjusted_bid: 0.9,
                valence: 0.0,
            },
        ];

        let allocation = vcg_allocate(bids, 1000, &AffectModulation::default());
        // cheap_high_value has much better value density (0.9/100 = 0.009 vs 0.5/900 = 0.0006)
        // so it should be picked first.
        assert_eq!(allocation.winners[0].section_name, "cheap_high_value");
    }
}
