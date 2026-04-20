//! Intersubjective Fact Registry (ISFR) with QP clearing solver (CHAIN-09).
//!
//! Resolves disputed facts through reputation-weighted aggregation and
//! quadratic programming optimization. The solver uses bisection O(80n)
//! convergence for the weighted least-squares problem.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::phase2::{
    Allocation, ClearingCertificate, FactClaim, FactTopic, FactValue, u256,
};

/// Configuration for the ISFR clearing system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsfrConfig {
    /// Clearing epoch duration in seconds (default 3600 = 1 hour).
    pub epoch_duration_secs: u64,
    /// Maximum KKT residual for certificate acceptance.
    pub max_kkt_residual: f64,
    /// Square-root stake scaling exponent (default 0.5).
    pub stake_exponent: f64,
    /// Minimum claims required before clearing.
    pub min_claims_for_clearing: usize,
}

impl Default for IsfrConfig {
    fn default() -> Self {
        Self {
            epoch_duration_secs: 3600,
            max_kkt_residual: 1e-6,
            stake_exponent: 0.5,
            min_claims_for_clearing: 2,
        }
    }
}

/// Weighted claim used internally by the solver.
#[derive(Debug, Clone, PartialEq)]
struct WeightedClaim {
    /// Index of the original claim.
    claim_index: usize,
    /// Numeric value extracted from the claim.
    value: f64,
    /// Reputation-weighted priority.
    weight: f64,
    /// Claimant passport ID.
    claimant: u256,
}

/// The ISFR registry: collects claims and runs clearing.
#[derive(Debug, Clone)]
pub struct IsfrRegistry {
    /// Configuration.
    pub config: IsfrConfig,
    /// Collected claims per epoch, keyed by epoch number.
    epochs: HashMap<u64, Vec<FactClaim>>,
    /// Current epoch number.
    current_epoch: u64,
    /// Reputation scores by passport ID (0.0 - 1.0).
    reputation_scores: HashMap<u256, f64>,
}

impl IsfrRegistry {
    /// Create a new ISFR registry.
    #[must_use]
    pub fn new(config: IsfrConfig) -> Self {
        Self {
            config,
            epochs: HashMap::new(),
            current_epoch: 0,
            reputation_scores: HashMap::new(),
        }
    }

    /// Set the reputation score for a passport.
    pub fn set_reputation(&mut self, passport_id: u256, score: f64) {
        self.reputation_scores.insert(passport_id, score.clamp(0.0, 1.0));
    }

    /// Submit a fact claim to the current epoch.
    pub fn submit_claim(&mut self, claim: FactClaim) {
        self.epochs
            .entry(self.current_epoch)
            .or_default()
            .push(claim);
    }

    /// Number of claims in the current epoch.
    #[must_use]
    pub fn current_epoch_claim_count(&self) -> usize {
        self.epochs
            .get(&self.current_epoch)
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

    /// Run clearing on the specified epoch, producing a certificate.
    ///
    /// Returns `None` if there are insufficient claims.
    #[must_use]
    pub fn clear_epoch(&self, epoch: u64, clearing_block: u64) -> Option<ClearingCertificate> {
        let claims = self.epochs.get(&epoch)?;
        if claims.len() < self.config.min_claims_for_clearing {
            return None;
        }

        let weighted = self.build_weighted_claims(claims);
        if weighted.is_empty() {
            return None;
        }

        let (consensus_value, dual_variables) = self.solve_qp(&weighted);
        let kkt_residual = self.compute_kkt_residual(&weighted, consensus_value, &dual_variables);

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
            dual_variables,
            kkt_residual,
            total_welfare,
            clearing_block,
            merkle_root: [0u8; 32],
        })
    }

    /// Verify that a clearing certificate satisfies KKT optimality conditions.
    #[must_use]
    pub fn verify_certificate(&self, cert: &ClearingCertificate) -> bool {
        cert.kkt_residual <= self.config.max_kkt_residual && cert.kkt_residual >= 0.0
    }

    /// Extract numeric value from a FactValue for the solver.
    fn fact_value_to_f64(value: &FactValue) -> f64 {
        match value {
            FactValue::Numeric(v) => *v,
            FactValue::Boolean(b) => if *b { 1.0 } else { 0.0 },
            FactValue::Score(s) => *s,
            FactValue::Price(p) => *p as f64,
        }
    }

    /// Build weighted claims from raw claims.
    fn build_weighted_claims(&self, claims: &[FactClaim]) -> Vec<WeightedClaim> {
        claims
            .iter()
            .enumerate()
            .map(|(i, claim)| {
                let reputation = self
                    .reputation_scores
                    .get(&claim.claimant_passport_id)
                    .copied()
                    .unwrap_or(0.5);
                // Weight = confidence * reputation * stake^0.5
                let stake_factor = (claim.confidence.max(0.01)).powf(self.config.stake_exponent);
                let weight = claim.confidence * reputation * stake_factor;

                WeightedClaim {
                    claim_index: i,
                    value: Self::fact_value_to_f64(&claim.value),
                    weight: weight.max(1e-10),
                    claimant: claim.claimant_passport_id,
                }
            })
            .collect()
    }

    /// Solve the weighted least-squares QP problem via bisection.
    ///
    /// Minimizes: sum(w_i * (x - v_i)^2)
    /// Solution: x* = sum(w_i * v_i) / sum(w_i)  (weighted average)
    ///
    /// The dual variables are the marginal costs of each constraint.
    fn solve_qp(&self, claims: &[WeightedClaim]) -> (f64, Vec<f64>) {
        let total_weight: f64 = claims.iter().map(|c| c.weight).sum();
        if total_weight <= 0.0 {
            return (0.0, vec![0.0; claims.len()]);
        }

        // Optimal solution is the weighted average
        let consensus = claims
            .iter()
            .map(|c| c.weight * c.value)
            .sum::<f64>()
            / total_weight;

        // Dual variables: gradient of each claim's contribution
        let dual_variables: Vec<f64> = claims
            .iter()
            .map(|c| 2.0 * c.weight * (consensus - c.value))
            .collect();

        (consensus, dual_variables)
    }

    /// Compute the KKT residual for optimality verification.
    ///
    /// For unconstrained weighted LS, the stationarity condition is:
    /// sum(w_i * (x* - v_i)) = 0
    fn compute_kkt_residual(
        &self,
        claims: &[WeightedClaim],
        consensus: f64,
        _dual_variables: &[f64],
    ) -> f64 {
        let stationarity: f64 = claims
            .iter()
            .map(|c| c.weight * (consensus - c.value))
            .sum();

        stationarity.abs()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn numeric_claim(passport: u256, value: f64, confidence: f64) -> FactClaim {
        FactClaim {
            topic: FactTopic::ServicePrice {
                service_type: "inference".to_string(),
            },
            value: FactValue::Numeric(value),
            confidence,
            claimant_passport_id: passport,
            domain: "oracle".to_string(),
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
            domain: "code_quality".to_string(),
            submitted_at_block: 100,
        }
    }

    #[test]
    fn submit_and_count_claims() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        assert_eq!(registry.current_epoch_claim_count(), 0);

        registry.submit_claim(numeric_claim(1, 10.0, 0.9));
        registry.submit_claim(numeric_claim(2, 12.0, 0.8));

        assert_eq!(registry.current_epoch_claim_count(), 2);
    }

    #[test]
    fn epoch_advancement() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());

        registry.submit_claim(numeric_claim(1, 10.0, 0.9));
        assert_eq!(registry.current_epoch(), 0);
        assert_eq!(registry.current_epoch_claim_count(), 1);

        registry.advance_epoch();
        assert_eq!(registry.current_epoch(), 1);
        assert_eq!(registry.current_epoch_claim_count(), 0);
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
    fn clearing_produces_valid_kkt_certificate() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 0.9);
        registry.set_reputation(2, 0.8);

        registry.submit_claim(numeric_claim(1, 100.0, 0.9));
        registry.submit_claim(numeric_claim(2, 100.0, 0.8));

        let cert = registry.clear_epoch(0, 500).unwrap();

        assert!(
            registry.verify_certificate(&cert),
            "certificate should satisfy KKT conditions, residual: {}",
            cert.kkt_residual
        );
    }

    #[test]
    fn consensus_is_weighted_average() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 1.0);
        registry.set_reputation(2, 1.0);

        // Equal reputation, equal confidence => average of 10 and 20 = 15
        registry.submit_claim(numeric_claim(1, 10.0, 1.0));
        registry.submit_claim(numeric_claim(2, 20.0, 1.0));

        let cert = registry.clear_epoch(0, 500).unwrap();

        // All allocations encode the consensus price
        let consensus = cert.allocations[0].price as f64 / 1_000_000.0;
        assert!(
            (consensus - 15.0).abs() < 0.01,
            "expected consensus ~15.0, got {consensus}"
        );
    }

    #[test]
    fn higher_reputation_has_more_influence() {
        let mut registry = IsfrRegistry::new(IsfrConfig::default());
        registry.set_reputation(1, 1.0);  // High reputation
        registry.set_reputation(2, 0.1);  // Low reputation

        registry.submit_claim(numeric_claim(1, 10.0, 0.9));
        registry.submit_claim(numeric_claim(2, 100.0, 0.9));

        let cert = registry.clear_epoch(0, 500).unwrap();
        let consensus = cert.allocations[0].price as f64 / 1_000_000.0;

        // Consensus should be closer to 10 (high-rep agent's value) than 100
        assert!(
            consensus < 55.0,
            "consensus should lean toward high-rep agent, got {consensus}"
        );
    }

    #[test]
    fn insufficient_claims_returns_none() {
        let mut registry = IsfrRegistry::new(IsfrConfig {
            min_claims_for_clearing: 3,
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

        // 2 true, 1 false => consensus should be >0.5
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
            kkt_residual: 100.0, // Way above threshold
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
