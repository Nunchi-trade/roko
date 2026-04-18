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
    /// Prior value used before any observations are recorded.
    pub prior_bid: f64,
}

impl LearningBidder {
    /// Create a learning bidder with a prior bid weight.
    #[must_use]
    pub fn new(subsystem_id: SubsystemId, prior_bid: f64) -> Self {
        Self {
            subsystem_id,
            section_betas: HashMap::new(),
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
}
