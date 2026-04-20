//! Intersubjective Fact Registry (ISFR) — collective rate discovery (CHAIN-09).
//!
//! ISFR functions as a collective price/rate discovery mechanism analogous to
//! SOFR/LIBOR for the agent economy. Agents submit rate observations for
//! hierarchical market IDs, and the system computes a robust aggregate using
//! **weighted median** (not weighted mean) with 3-sigma outlier exclusion.
//!
//! ## Spec alignment
//!
//! Based on `docs/14-identity-economy/13-isfr-clearing-settlement.md`:
//! - Agents submit `IsfrSubmission` with `market_id`, `rate`, `components`, `confidence`.
//! - Aggregation uses two-level weighted median with outlier exclusion.
//! - Rates update every 8 hours (configurable).
//! - Output is `IsfrAggregate` with `median_rate`, `std_deviation`, `excluded_count`.
//!
//! Also retains the QP clearing solver for backwards compatibility with existing
//! `ClearingCertificate` consumers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::phase2::{Allocation, ClearingCertificate, FactClaim, FactValue, u256};

// ─── Configuration ──────────────────────────────────────────────────

/// Configuration for the ISFR system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsfrConfig {
    /// Epoch duration in seconds (default 28800 = 8 hours per spec).
    pub epoch_duration_secs: u64,
    /// Maximum KKT residual for certificate acceptance (legacy solver).
    pub max_kkt_residual: f64,
    /// Minimum submissions required before aggregation.
    pub min_submissions_for_clearing: usize,
    /// Minimum submitter reputation for eligibility (spec: 0.5).
    pub min_reputation: f64,
    /// Maximum absolute rate value (spec: 0.1 for bounded rates, relaxed for prices).
    pub max_rate_bound: Option<f64>,
    /// Sigma multiplier for outlier exclusion (spec: 3.0).
    pub outlier_sigma: f64,
}

impl Default for IsfrConfig {
    fn default() -> Self {
        Self {
            epoch_duration_secs: 28_800, // 8 hours = 3 epochs/day
            max_kkt_residual: 1e-6,
            min_submissions_for_clearing: 2,
            min_reputation: 0.5,
            max_rate_bound: Some(0.1),
            outlier_sigma: 3.0,
        }
    }
}

// ─── Market IDs ─────────────────────────────────────────────────────

/// Hierarchical market identifier for ISFR rate submissions.
///
/// Standard market IDs follow hierarchical naming:
/// - `knowledge/defi`, `knowledge/security`
/// - `compute/inference`, `compute/indexing`
/// - `services/code-review`, `services/audit`
///
/// Custom market IDs can be registered by Sovereign-tier agents via governance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MarketId(pub String);

impl MarketId {
    /// Create a new market ID from a hierarchical path.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Top-level category (e.g., "knowledge", "compute", "services").
    pub fn category(&self) -> &str {
        self.0.split('/').next().unwrap_or(&self.0)
    }

    /// Sub-category (e.g., "defi", "inference", "code-review").
    pub fn subcategory(&self) -> Option<&str> {
        self.0.split('/').nth(1)
    }

    /// Standard market IDs defined by the protocol.
    pub fn standard_markets() -> &'static [&'static str] {
        &[
            "knowledge/defi",
            "knowledge/security",
            "knowledge/research",
            "compute/inference",
            "compute/indexing",
            "services/code-review",
            "services/audit",
            "services/orchestration",
        ]
    }
}

// ─── Submission and Aggregate Types ─────────────────────────────────

/// Discipline state check for submission eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitterStatus {
    /// Eligible to submit.
    Eligible,
    /// Rejected: reputation too low.
    InsufficientReputation,
    /// Rejected: in quarantine or revoked state.
    Quarantined,
    /// Rejected: rate out of bounds.
    RateOutOfBounds,
    /// Rejected: components don't sum to rate.
    ComponentMismatch,
}

/// A single ISFR rate submission from an agent.
///
/// Matches spec's `IsfrSubmission` schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsfrSubmission {
    /// Hierarchical market ID (e.g., "knowledge/defi").
    pub market_id: MarketId,
    /// Observed rate, bounded by config (spec default: [-0.1, 0.1]).
    pub rate: f64,
    /// Rate components that should sum to `rate` (within floating point tolerance).
    pub components: Vec<f64>,
    /// Submitter confidence in [0.0, 1.0].
    pub confidence: f64,
    /// Submitter passport ID.
    pub submitter_passport_id: u256,
    /// Block number at submission time.
    pub submitted_at_block: u64,
}

impl IsfrSubmission {
    /// Validate that components sum to rate within tolerance.
    pub fn components_valid(&self) -> bool {
        if self.components.is_empty() {
            return true; // Components are optional
        }
        let sum: f64 = self.components.iter().sum();
        (sum - self.rate).abs() < 1e-9
    }
}

/// Aggregated ISFR rate for a market after clearing.
///
/// Matches spec's `IsfrAggregate` output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsfrAggregate {
    /// The computed weighted median rate.
    pub median_rate: f64,
    /// Number of eligible submissions included.
    pub submission_count: usize,
    /// Standard deviation of included submissions.
    pub std_deviation: f64,
    /// Number of submissions excluded as outliers.
    pub excluded_count: usize,
    /// Market this aggregate applies to.
    pub market_id: MarketId,
    /// Epoch number this aggregate was computed for.
    pub epoch: u64,
    /// Block at which clearing was performed.
    pub clearing_block: u64,
}

// ─── ISFR Registry ──────────────────────────────────────────────────

/// The ISFR registry: collects submissions and produces aggregates via weighted median.
#[derive(Debug, Clone)]
pub struct IsfrRegistry {
    /// Configuration.
    pub config: IsfrConfig,
    /// Submissions per (epoch, market_id).
    submissions: HashMap<(u64, MarketId), Vec<IsfrSubmission>>,
    /// Current epoch number.
    current_epoch: u64,
    /// Reputation scores by passport ID (0.0 - 1.0).
    reputation_scores: HashMap<u256, f64>,
    /// Quarantined/revoked passport IDs.
    quarantined: Vec<u256>,
    /// Registered custom market IDs (beyond standard set).
    custom_markets: Vec<MarketId>,
}

impl IsfrRegistry {
    /// Create a new ISFR registry.
    #[must_use]
    pub fn new(config: IsfrConfig) -> Self {
        Self {
            config,
            submissions: HashMap::new(),
            current_epoch: 0,
            reputation_scores: HashMap::new(),
            quarantined: Vec::new(),
            custom_markets: Vec::new(),
        }
    }

    /// Set the reputation score for a passport.
    pub fn set_reputation(&mut self, passport_id: u256, score: f64) {
        self.reputation_scores
            .insert(passport_id, score.clamp(0.0, 1.0));
    }

    /// Mark a passport as quarantined (ineligible for submissions).
    pub fn quarantine(&mut self, passport_id: u256) {
        if !self.quarantined.contains(&passport_id) {
            self.quarantined.push(passport_id);
        }
    }

    /// Register a custom market ID.
    pub fn register_market(&mut self, market_id: MarketId) {
        if !self.custom_markets.contains(&market_id) {
            self.custom_markets.push(market_id);
        }
    }

    /// Check if a submitter is eligible.
    pub fn check_eligibility(&self, submission: &IsfrSubmission) -> SubmitterStatus {
        // Check quarantine.
        if self.quarantined.contains(&submission.submitter_passport_id) {
            return SubmitterStatus::Quarantined;
        }

        // Check minimum reputation.
        let rep = self
            .reputation_scores
            .get(&submission.submitter_passport_id)
            .copied()
            .unwrap_or(0.0);
        if rep < self.config.min_reputation {
            return SubmitterStatus::InsufficientReputation;
        }

        // Check rate bounds.
        if let Some(bound) = self.config.max_rate_bound {
            if submission.rate.abs() > bound {
                return SubmitterStatus::RateOutOfBounds;
            }
        }

        // Check component sum.
        if !submission.components_valid() {
            return SubmitterStatus::ComponentMismatch;
        }

        SubmitterStatus::Eligible
    }

    /// Submit a rate observation. Returns the eligibility status.
    pub fn submit(&mut self, submission: IsfrSubmission) -> SubmitterStatus {
        let status = self.check_eligibility(&submission);
        if status != SubmitterStatus::Eligible {
            return status;
        }

        let key = (self.current_epoch, submission.market_id.clone());
        self.submissions.entry(key).or_default().push(submission);
        SubmitterStatus::Eligible
    }

    /// Number of submissions in the current epoch for a market.
    #[must_use]
    pub fn submission_count(&self, market_id: &MarketId) -> usize {
        self.submissions
            .get(&(self.current_epoch, market_id.clone()))
            .map_or(0, Vec::len)
    }

    /// Advance to the next epoch.
    pub fn advance_epoch(&mut self) {
        self.current_epoch += 1;
    }

    /// Current epoch number.
    #[must_use]
    pub fn current_epoch(&self) -> u64 {
        self.current_epoch
    }

    /// Compute the ISFR aggregate for a market in the given epoch.
    ///
    /// Uses two-level aggregation:
    /// 1. Compute initial weighted median.
    /// 2. Exclude submissions > 3-sigma from the initial median.
    /// 3. Recompute weighted median on filtered set.
    #[must_use]
    pub fn aggregate(
        &self,
        market_id: &MarketId,
        epoch: u64,
        clearing_block: u64,
    ) -> Option<IsfrAggregate> {
        let submissions = self.submissions.get(&(epoch, market_id.clone()))?;
        if submissions.len() < self.config.min_submissions_for_clearing {
            return None;
        }

        // Build weighted entries: weight = confidence * reputation_multiplier.
        let weighted: Vec<(f64, f64)> = submissions
            .iter()
            .filter_map(|s| {
                let rep = self
                    .reputation_scores
                    .get(&s.submitter_passport_id)
                    .copied()
                    .unwrap_or(0.5);
                let weight = s.confidence * rep;
                if weight > 0.0 {
                    Some((s.rate, weight))
                } else {
                    None
                }
            })
            .collect();

        if weighted.is_empty() {
            return None;
        }

        // Step 1: Compute initial weighted median.
        let initial_median = weighted_median(&weighted);

        // Step 2: Compute standard deviation and exclude 3-sigma outliers.
        let std_dev = weighted_std_deviation(&weighted, initial_median);
        let sigma_bound = self.config.outlier_sigma * std_dev;

        let total_before = weighted.len();
        let filtered: Vec<(f64, f64)> = weighted
            .iter()
            .filter(|(rate, _)| (rate - initial_median).abs() <= sigma_bound)
            .copied()
            .collect();

        let excluded_count = total_before - filtered.len();

        // Step 3: Recompute weighted median on filtered set.
        let final_entries = if filtered.is_empty() {
            &weighted
        } else {
            &filtered
        };
        let median_rate = weighted_median(final_entries);
        let final_std = weighted_std_deviation(final_entries, median_rate);

        Some(IsfrAggregate {
            median_rate,
            submission_count: final_entries.len(),
            std_deviation: final_std,
            excluded_count,
            market_id: market_id.clone(),
            epoch,
            clearing_block,
        })
    }

    // ─── Legacy compatibility (FactClaim-based API) ─────────────────

    /// Submit a legacy FactClaim to the current epoch.
    ///
    /// Converts to the new market-based system internally.
    pub fn submit_claim(&mut self, claim: FactClaim) {
        let rate = fact_value_to_f64(&claim.value);
        let submission = IsfrSubmission {
            market_id: MarketId::new(&claim.domain),
            rate,
            components: Vec::new(),
            confidence: claim.confidence,
            submitter_passport_id: claim.claimant_passport_id,
            submitted_at_block: claim.submitted_at_block,
        };
        // Skip eligibility for legacy claims (backwards compat).
        let key = (self.current_epoch, submission.market_id.clone());
        self.submissions.entry(key).or_default().push(submission);
    }

    /// Legacy: Run clearing on the specified epoch using the QP solver path.
    ///
    /// Returns `None` if there are insufficient claims.
    #[must_use]
    pub fn clear_epoch(&self, epoch: u64, clearing_block: u64) -> Option<ClearingCertificate> {
        // Collect all submissions across all markets for this epoch.
        let all_submissions: Vec<&IsfrSubmission> = self
            .submissions
            .iter()
            .filter(|((e, _), _)| *e == epoch)
            .flat_map(|(_, subs)| subs.iter())
            .collect();

        if all_submissions.len() < self.config.min_submissions_for_clearing {
            return None;
        }

        // Build weighted claims for legacy solver.
        let weighted: Vec<WeightedClaim> = all_submissions
            .iter()
            .enumerate()
            .map(|(i, sub)| {
                let rep = self
                    .reputation_scores
                    .get(&sub.submitter_passport_id)
                    .copied()
                    .unwrap_or(0.5);
                let weight = sub.confidence * rep;
                WeightedClaim {
                    claim_index: i,
                    value: sub.rate,
                    weight: weight.max(1e-10),
                    claimant: sub.submitter_passport_id,
                }
            })
            .collect();

        if weighted.is_empty() {
            return None;
        }

        // Use weighted median instead of weighted mean.
        let entries: Vec<(f64, f64)> = weighted.iter().map(|w| (w.value, w.weight)).collect();
        let consensus_value = weighted_median(&entries);

        // Compute KKT-like residual (stationarity check using median deviation).
        let kkt_residual = weighted
            .iter()
            .map(|c| c.weight * (consensus_value - c.value))
            .sum::<f64>()
            .abs();

        let allocations = weighted
            .iter()
            .map(|wc| Allocation {
                agent_passport_id: wc.claimant,
                job_id: [0u8; 32],
                price: (consensus_value * 1_000_000.0) as u256,
                quality_score: wc.weight,
            })
            .collect();

        let total_welfare = weighted
            .iter()
            .map(|wc| wc.weight * (1.0 - (wc.value - consensus_value).powi(2)))
            .sum();

        Some(ClearingCertificate {
            allocations,
            dual_variables: vec![0.0; weighted.len()],
            kkt_residual,
            total_welfare,
            clearing_block,
            merkle_root: [0u8; 32],
        })
    }

    /// Verify that a clearing certificate satisfies optimality conditions.
    #[must_use]
    pub fn verify_certificate(&self, cert: &ClearingCertificate) -> bool {
        cert.kkt_residual <= self.config.max_kkt_residual && cert.kkt_residual >= 0.0
    }
}

// ─── Weighted Median Algorithm ──────────────────────────────────────

/// Internal weighted claim for the legacy solver path.
#[derive(Debug, Clone, PartialEq)]
struct WeightedClaim {
    claim_index: usize,
    value: f64,
    weight: f64,
    claimant: u256,
}

/// Compute the weighted median of a set of (value, weight) pairs.
///
/// The weighted median is the value where the cumulative weight from below
/// equals the cumulative weight from above. This is more robust than
/// weighted mean because it resists outlier influence.
fn weighted_median(entries: &[(f64, f64)]) -> f64 {
    if entries.is_empty() {
        return 0.0;
    }
    if entries.len() == 1 {
        return entries[0].0;
    }

    // Sort by value.
    let mut sorted: Vec<(f64, f64)> = entries.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let total_weight: f64 = sorted.iter().map(|(_, w)| w).sum();
    let half_weight = total_weight / 2.0;

    // Walk through sorted values, accumulating weight.
    let mut cumulative = 0.0;
    for (i, &(value, weight)) in sorted.iter().enumerate() {
        cumulative += weight;
        if cumulative >= half_weight {
            // If we're exactly at half, interpolate with next value.
            if (cumulative - half_weight).abs() < 1e-12 && i + 1 < sorted.len() {
                return (value + sorted[i + 1].0) / 2.0;
            }
            return value;
        }
    }

    // Fallback: return last value.
    sorted.last().unwrap().0
}

/// Compute weighted standard deviation around a center value.
fn weighted_std_deviation(entries: &[(f64, f64)], center: f64) -> f64 {
    if entries.len() < 2 {
        return 0.0;
    }

    let total_weight: f64 = entries.iter().map(|(_, w)| w).sum();
    if total_weight <= 0.0 {
        return 0.0;
    }

    let variance: f64 = entries
        .iter()
        .map(|(value, weight)| weight * (value - center).powi(2))
        .sum::<f64>()
        / total_weight;

    variance.sqrt()
}

/// Extract numeric value from a FactValue.
fn fact_value_to_f64(value: &FactValue) -> f64 {
    match value {
        FactValue::Numeric(v) => *v,
        FactValue::Boolean(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        FactValue::Score(s) => *s,
        FactValue::Price(p) => *p as f64,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::phase2::FactTopic;

    // ─── New API tests ──────────────────────────────────────────────

    #[test]
    fn submit_and_aggregate_basic() {
        let mut registry = IsfrRegistry::new(IsfrConfig {
            min_reputation: 0.5,
            max_rate_bound: Some(0.1),
            ..Default::default()
        });
        registry.set_reputation(1, 0.9);
        registry.set_reputation(2, 0.8);
        registry.set_reputation(3, 0.7);

        let market = MarketId::new("knowledge/defi");

        assert_eq!(
            registry.submit(IsfrSubmission {
                market_id: market.clone(),
                rate: 0.05,
                components: vec![0.03, 0.02],
                confidence: 0.9,
                submitter_passport_id: 1,
                submitted_at_block: 100,
            }),
            SubmitterStatus::Eligible
        );

        assert_eq!(
            registry.submit(IsfrSubmission {
                market_id: market.clone(),
                rate: 0.06,
                components: vec![0.04, 0.02],
                confidence: 0.85,
                submitter_passport_id: 2,
                submitted_at_block: 100,
            }),
            SubmitterStatus::Eligible
        );

        assert_eq!(
            registry.submit(IsfrSubmission {
                market_id: market.clone(),
                rate: 0.055,
                components: vec![0.035, 0.02],
                confidence: 0.8,
                submitter_passport_id: 3,
                submitted_at_block: 100,
            }),
            SubmitterStatus::Eligible
        );

        let agg = registry.aggregate(&market, 0, 500).unwrap();
        assert_eq!(agg.submission_count, 3);
        assert_eq!(agg.excluded_count, 0);
        // Median should be around 0.055 (middle value).
        assert!(
            (agg.median_rate - 0.055).abs() < 0.02,
            "median_rate = {}",
            agg.median_rate
        );
    }

    #[test]
    fn rejects_low_reputation() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 0.3); // Below min 0.5

        let status = registry.submit(IsfrSubmission {
            market_id: MarketId::new("compute/inference"),
            rate: 0.05,
            components: vec![],
            confidence: 0.9,
            submitter_passport_id: 1,
            submitted_at_block: 100,
        });

        assert_eq!(status, SubmitterStatus::InsufficientReputation);
    }

    #[test]
    fn rejects_quarantined_submitter() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 0.9);
        registry.quarantine(1);

        let status = registry.submit(IsfrSubmission {
            market_id: MarketId::new("knowledge/security"),
            rate: 0.05,
            components: vec![],
            confidence: 0.9,
            submitter_passport_id: 1,
            submitted_at_block: 100,
        });

        assert_eq!(status, SubmitterStatus::Quarantined);
    }

    #[test]
    fn rejects_rate_out_of_bounds() {
        let mut registry = IsfrRegistry::new(IsfrConfig {
            max_rate_bound: Some(0.1),
            ..Default::default()
        });
        registry.set_reputation(1, 0.9);

        let status = registry.submit(IsfrSubmission {
            market_id: MarketId::new("knowledge/defi"),
            rate: 0.15, // Exceeds 0.1 bound
            components: vec![],
            confidence: 0.9,
            submitter_passport_id: 1,
            submitted_at_block: 100,
        });

        assert_eq!(status, SubmitterStatus::RateOutOfBounds);
    }

    #[test]
    fn rejects_component_mismatch() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 0.9);

        let status = registry.submit(IsfrSubmission {
            market_id: MarketId::new("knowledge/defi"),
            rate: 0.05,
            components: vec![0.01, 0.01], // Sum = 0.02 != 0.05
            confidence: 0.9,
            submitter_passport_id: 1,
            submitted_at_block: 100,
        });

        assert_eq!(status, SubmitterStatus::ComponentMismatch);
    }

    #[test]
    fn outlier_exclusion_works() {
        let mut registry = IsfrRegistry::new(IsfrConfig {
            min_reputation: 0.0,
            max_rate_bound: None, // No bound for this test
            outlier_sigma: 2.0,   // Tighter threshold for test
            ..Default::default()
        });

        let market = MarketId::new("test/outlier");

        // 4 submissions clustered around 0.05, 1 extreme outlier at 0.90.
        for id in 1..=4 {
            registry.set_reputation(id, 0.8);
            let key = (0, market.clone());
            registry
                .submissions
                .entry(key)
                .or_default()
                .push(IsfrSubmission {
                    market_id: market.clone(),
                    rate: 0.05 + (id as f64 - 2.5) * 0.005,
                    components: vec![],
                    confidence: 0.9,
                    submitter_passport_id: id,
                    submitted_at_block: 100,
                });
        }

        // Add outlier.
        registry.set_reputation(5, 0.8);
        let key = (0, market.clone());
        registry
            .submissions
            .entry(key)
            .or_default()
            .push(IsfrSubmission {
                market_id: market.clone(),
                rate: 0.90,
                components: vec![],
                confidence: 0.9,
                submitter_passport_id: 5,
                submitted_at_block: 100,
            });

        let agg = registry.aggregate(&market, 0, 500).unwrap();
        assert!(
            agg.excluded_count >= 1,
            "outlier should be excluded, got {}",
            agg.excluded_count
        );
        assert!(
            agg.median_rate < 0.1,
            "median should not be pulled by outlier, got {}",
            agg.median_rate
        );
    }

    #[test]
    fn weighted_median_gives_middle_value_for_equal_weights() {
        let entries = vec![(1.0, 1.0), (2.0, 1.0), (3.0, 1.0), (4.0, 1.0), (5.0, 1.0)];
        let median = weighted_median(&entries);
        assert!((median - 3.0).abs() < 0.01, "expected 3.0, got {median}");
    }

    #[test]
    fn weighted_median_favors_heavier_weight() {
        // Value 10 has weight 9, value 20 has weight 1.
        let entries = vec![(10.0, 9.0), (20.0, 1.0)];
        let median = weighted_median(&entries);
        assert!(
            (median - 10.0).abs() < 0.01,
            "expected 10.0 (heavy weight), got {median}"
        );
    }

    #[test]
    fn market_id_hierarchy() {
        let market = MarketId::new("knowledge/defi");
        assert_eq!(market.category(), "knowledge");
        assert_eq!(market.subcategory(), Some("defi"));

        let top_level = MarketId::new("compute");
        assert_eq!(top_level.category(), "compute");
        assert_eq!(top_level.subcategory(), None);
    }

    // ─── Legacy API tests ───────────────────────────────────────────

    fn numeric_claim(passport: u256, value: f64, confidence: f64) -> FactClaim {
        FactClaim {
            topic: FactTopic::ServicePrice {
                service_type: "inference".to_string(),
            },
            value: FactValue::Numeric(value),
            confidence,
            claimant_passport_id: passport,
            domain: "chain".to_string(),
            submitted_at_block: 100,
        }
    }

    fn boolean_claim(passport: u256, value: bool, confidence: f64) -> FactClaim {
        FactClaim {
            topic: FactTopic::QualityAssessment {
                job_hash: [0u8; 32],
            },
            value: FactValue::Boolean(value),
            confidence,
            claimant_passport_id: passport,
            domain: "coding".to_string(),
            submitted_at_block: 100,
        }
    }

    #[test]
    fn submit_and_count_claims() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        let market = MarketId::new("chain");
        assert_eq!(registry.submission_count(&market), 0);

        registry.submit_claim(numeric_claim(1, 10.0, 0.9));
        registry.submit_claim(numeric_claim(2, 12.0, 0.8));

        assert_eq!(registry.submission_count(&market), 2);
    }

    #[test]
    fn epoch_advancement() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        let market = MarketId::new("chain");

        registry.submit_claim(numeric_claim(1, 10.0, 0.9));
        assert_eq!(registry.current_epoch(), 0);
        assert_eq!(registry.submission_count(&market), 1);

        registry.advance_epoch();
        assert_eq!(registry.current_epoch(), 1);
        assert_eq!(registry.submission_count(&market), 0);
    }

    #[test]
    fn clear_epoch_produces_certificate() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 0.9);
        registry.set_reputation(2, 0.8);
        registry.set_reputation(3, 0.7);

        registry.submit_claim(numeric_claim(1, 10.0, 0.9));
        registry.submit_claim(numeric_claim(2, 11.0, 0.8));
        registry.submit_claim(numeric_claim(3, 10.5, 0.85));

        let cert = registry.clear_epoch(0, 500).unwrap();
        assert_eq!(cert.allocations.len(), 3);
        assert_eq!(cert.clearing_block, 500);
        assert!(cert.total_welfare > 0.0);
    }

    #[test]
    fn consensus_uses_weighted_median() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 1.0);
        registry.set_reputation(2, 1.0);
        registry.set_reputation(3, 1.0);

        // Three equal-weight submissions: 10, 15, 20. Median should be 15.
        registry.submit_claim(numeric_claim(1, 10.0, 1.0));
        registry.submit_claim(numeric_claim(2, 15.0, 1.0));
        registry.submit_claim(numeric_claim(3, 20.0, 1.0));

        let cert = registry.clear_epoch(0, 500).unwrap();
        let consensus = cert.allocations[0].price as f64 / 1_000_000.0;
        assert!(
            (consensus - 15.0).abs() < 0.01,
            "expected weighted median ~15.0, got {consensus}"
        );
    }

    #[test]
    fn higher_reputation_has_more_influence() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 1.0); // High reputation
        registry.set_reputation(2, 0.1); // Low reputation

        registry.submit_claim(numeric_claim(1, 10.0, 0.9));
        registry.submit_claim(numeric_claim(2, 100.0, 0.9));

        let cert = registry.clear_epoch(0, 500).unwrap();
        let consensus = cert.allocations[0].price as f64 / 1_000_000.0;

        // Consensus should be 10.0 (high-rep agent dominates the weighted median)
        assert!(
            consensus < 55.0,
            "consensus should lean toward high-rep agent, got {consensus}"
        );
    }

    #[test]
    fn insufficient_claims_returns_none() {
        let mut registry = IsfrRegistry::new(IsfrConfig {
            min_submissions_for_clearing: 3,
            ..Default::default()
        });

        registry.submit_claim(numeric_claim(1, 10.0, 0.9));
        registry.submit_claim(numeric_claim(2, 12.0, 0.8));

        assert!(registry.clear_epoch(0, 500).is_none());
    }

    #[test]
    fn empty_epoch_returns_none() {
        let registry = IsfrRegistry::new(IsfrConfig::default());
        assert!(registry.clear_epoch(0, 500).is_none());
    }

    #[test]
    fn boolean_claims_clear_correctly() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 0.9);
        registry.set_reputation(2, 0.8);
        registry.set_reputation(3, 0.7);

        registry.submit_claim(boolean_claim(1, true, 0.9));
        registry.submit_claim(boolean_claim(2, true, 0.85));
        registry.submit_claim(boolean_claim(3, false, 0.7));

        let cert = registry.clear_epoch(0, 500).unwrap();
        let consensus = cert.allocations[0].price as f64 / 1_000_000.0;

        assert!(
            consensus > 0.5,
            "boolean majority-true should produce consensus > 0.5, got {consensus}"
        );
    }

    #[test]
    fn verify_rejects_bad_certificate() {
        let registry = IsfrRegistry::new(IsfrConfig {
            max_kkt_residual: 1e-6,
            ..Default::default()
        });

        let bad_cert = ClearingCertificate {
            kkt_residual: 100.0,
            ..Default::default()
        };

        assert!(!registry.verify_certificate(&bad_cert));
    }

    #[test]
    fn verify_accepts_good_certificate() {
        let registry = IsfrRegistry::new(IsfrConfig::default());

        let good_cert = ClearingCertificate {
            kkt_residual: 1e-10,
            total_welfare: 5.0,
            ..Default::default()
        };

        assert!(registry.verify_certificate(&good_cert));
    }
}
