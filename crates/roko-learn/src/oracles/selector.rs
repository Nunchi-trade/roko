//! OracleSelector — Thompson Sampling based oracle routing.
//!
//! When multiple oracles can answer a given query domain, the selector
//! uses Thompson Sampling (Beta-Bernoulli bandit) to pick the oracle
//! with the best expected accuracy. After each prediction is resolved,
//! `record_outcome()` updates the posterior for the chosen oracle.
//!
//! This implements **TA-12** from the technical analysis gap checklist:
//! "OracleSelector uses Thompson Sampling to select from multiple oracles."

use std::collections::HashMap;

use rand::Rng;
use roko_core::OracleDomain;

/// Identifier for a registered oracle.
pub type OracleId = String;

/// Thompson arm for a single oracle, tracking its accuracy posterior.
#[derive(Debug, Clone)]
pub struct OracleArm {
    /// Human-readable oracle identifier.
    pub oracle_id: OracleId,
    /// Domain this oracle serves.
    pub domain: OracleDomain,
    /// Beta distribution alpha (success count + prior).
    pub alpha: f64,
    /// Beta distribution beta (failure count + prior).
    pub beta: f64,
    /// Total observations.
    pub observations: u64,
    /// Discount factor for non-stationarity (default 0.99).
    pub discount: f64,
}

impl OracleArm {
    /// Create a new arm with Beta(1, 1) prior (uniform).
    pub fn new(oracle_id: impl Into<String>, domain: OracleDomain) -> Self {
        Self {
            oracle_id: oracle_id.into(),
            domain,
            alpha: 1.0,
            beta: 1.0,
            observations: 0,
            discount: 0.99,
        }
    }

    /// Sample from the Beta posterior.
    pub fn sample(&self) -> f64 {
        let mut rng = rand::thread_rng();
        self.sample_with_rng(&mut rng)
    }

    /// Sample with an explicit RNG (for testing).
    pub fn sample_with_rng<R: Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        sample_beta(self.alpha, self.beta, rng).clamp(0.0, 1.0)
    }

    /// Update the posterior after an observation.
    ///
    /// `accuracy` is in [0, 1]; values >= 0.5 count as success.
    pub fn update(&mut self, accuracy: f64) {
        // Apply discount for non-stationarity.
        self.alpha = 1.0 + self.discount * (self.alpha - 1.0);
        self.beta = 1.0 + self.discount * (self.beta - 1.0);

        if accuracy >= 0.5 {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
        self.observations += 1;
    }

    /// Mean of the Beta posterior (expected accuracy).
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }
}

/// Oracle selector using Thompson Sampling to route queries to the
/// most accurate oracle for a given domain.
#[derive(Debug, Clone, Default)]
pub struct OracleSelector {
    /// Arms keyed by (domain, oracle_id).
    arms: HashMap<(String, OracleId), OracleArm>,
}

impl OracleSelector {
    /// Create an empty selector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an oracle for a domain.
    pub fn register(&mut self, oracle_id: impl Into<String>, domain: OracleDomain) {
        let id = oracle_id.into();
        let key = (domain.to_string(), id.clone());
        self.arms
            .entry(key)
            .or_insert_with(|| OracleArm::new(id, domain));
    }

    /// Select the best oracle for a domain using Thompson Sampling.
    ///
    /// Returns `None` if no oracles are registered for the domain.
    pub fn select(&self, domain: &OracleDomain) -> Option<&OracleArm> {
        let domain_str = domain.to_string();
        let candidates: Vec<&OracleArm> = self
            .arms
            .iter()
            .filter(|((d, _), _)| d == &domain_str)
            .map(|(_, arm)| arm)
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Thompson Sampling: sample from each arm's posterior, pick highest.
        candidates.into_iter().max_by(|a, b| {
            a.sample()
                .partial_cmp(&b.sample())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Select deterministically using mean posterior (for testing/logging).
    pub fn select_by_mean(&self, domain: &OracleDomain) -> Option<&OracleArm> {
        let domain_str = domain.to_string();
        self.arms
            .iter()
            .filter(|((d, _), _)| d == &domain_str)
            .map(|(_, arm)| arm)
            .max_by(|a, b| {
                a.mean()
                    .partial_cmp(&b.mean())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Record an accuracy outcome for an oracle.
    ///
    /// `accuracy` should be in [0, 1].
    pub fn record_outcome(&mut self, oracle_id: &str, domain: &OracleDomain, accuracy: f64) {
        let key = (domain.to_string(), oracle_id.to_string());
        if let Some(arm) = self.arms.get_mut(&key) {
            arm.update(accuracy);
        }
    }

    /// Return all arms for a domain, sorted by mean accuracy descending.
    pub fn arms_for_domain(&self, domain: &OracleDomain) -> Vec<&OracleArm> {
        let domain_str = domain.to_string();
        let mut arms: Vec<&OracleArm> = self
            .arms
            .iter()
            .filter(|((d, _), _)| d == &domain_str)
            .map(|(_, arm)| arm)
            .collect();
        arms.sort_by(|a, b| {
            b.mean()
                .partial_cmp(&a.mean())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        arms
    }

    /// Total number of registered oracle arms.
    pub fn arm_count(&self) -> usize {
        self.arms.len()
    }

    /// Select a batch of oracles for a domain using MVT foraging.
    ///
    /// Orders oracles by their posterior mean and stops adding more when the
    /// marginal expected accuracy gain from the next oracle drops below the
    /// average gain rate (Charnov's Marginal Value Theorem).
    ///
    /// This implements **TA-12** (remaining item): `MultiPatchForager::
    /// should_stop_searching()` limits oracle query batching.
    ///
    /// `sufficiency` is the estimated context sufficiency in [0, 1] — when
    /// sufficiency is already high, fewer oracles are queried.
    /// `sufficiency_threshold` controls when sufficiency alone stops search.
    ///
    /// Returns oracle IDs in priority order.
    pub fn select_batch(
        &self,
        domain: &OracleDomain,
        sufficiency: f64,
        sufficiency_threshold: f64,
    ) -> Vec<&OracleArm> {
        let arms = self.arms_for_domain(domain);
        if arms.is_empty() {
            return Vec::new();
        }

        // Compute the overall mean accuracy as the environment rate.
        let total_mean: f64 = arms.iter().map(|a| a.mean()).sum::<f64>();
        let env_rate = total_mean / arms.len() as f64;

        let mut batch = Vec::new();
        let mut cumulative_gain = 0.0;

        for (i, arm) in arms.iter().enumerate() {
            let marginal_gain = arm.mean();
            cumulative_gain += marginal_gain;
            let avg_gain = cumulative_gain / (i + 1) as f64;

            // MVT stopping: when the marginal gain from the next oracle
            // drops below the average gain rate, stop.
            let mvt_ratio = if avg_gain > f64::EPSILON {
                marginal_gain / avg_gain
            } else {
                1.0
            };

            // Use the Charnov MVT stopping criterion.
            if i > 0 && (mvt_ratio <= 1.0 || sufficiency >= sufficiency_threshold) {
                break;
            }

            batch.push(*arm);

            // Also stop if the marginal gain is below the environment rate
            // (diminishing returns).
            if marginal_gain < env_rate * 0.8 {
                break;
            }
        }

        // Always include at least one oracle.
        if batch.is_empty() {
            if let Some(best) = arms.into_iter().next() {
                batch.push(best);
            }
        }

        batch
    }
}

/// Sample from Beta(alpha, beta) using Gamma variates.
fn sample_beta<R: Rng + ?Sized>(alpha: f64, beta: f64, rng: &mut R) -> f64 {
    let x = sample_gamma(alpha.max(f64::MIN_POSITIVE), rng);
    let y = sample_gamma(beta.max(f64::MIN_POSITIVE), rng);
    let total = x + y;
    if total < f64::EPSILON { 0.5 } else { x / total }
}

/// Sample from Gamma(shape, 1) using Marsaglia-Tsang method.
fn sample_gamma<R: Rng + ?Sized>(shape: f64, rng: &mut R) -> f64 {
    if shape < 1.0 {
        // Boost: Gamma(a) = Gamma(a+1) * U^(1/a)
        let u: f64 = rng.gen_range(f64::MIN_POSITIVE..1.0);
        return sample_gamma(shape + 1.0, rng) * u.powf(1.0 / shape);
    }
    let d = shape - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();
    loop {
        let x: f64 = {
            // Standard normal via Box-Muller.
            let u1: f64 = rng.gen_range(f64::MIN_POSITIVE..1.0);
            let u2: f64 = rng.gen_range(0.0_f64..1.0);
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        };
        let v = (1.0 + c * x).powi(3);
        if v <= 0.0 {
            continue;
        }
        let u: f64 = rng.gen_range(f64::MIN_POSITIVE..1.0);
        if u.ln() < 0.5 * x * x + d - d * v + d * v.ln() {
            return d * v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_select() {
        let mut selector = OracleSelector::new();
        selector.register("chain_oracle", OracleDomain::Chain);
        selector.register("chain_oracle_v2", OracleDomain::Chain);
        selector.register("coding_oracle", OracleDomain::Coding);

        assert_eq!(selector.arm_count(), 3);

        // Should return an oracle for Chain domain.
        let selected = selector.select(&OracleDomain::Chain);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().domain, OracleDomain::Chain);

        // No Research oracles registered.
        assert!(selector.select(&OracleDomain::Research).is_none());
    }

    #[test]
    fn record_outcome_updates_posterior() {
        let mut selector = OracleSelector::new();
        selector.register("oracle_a", OracleDomain::Coding);
        selector.register("oracle_b", OracleDomain::Coding);

        // Feed many successes to oracle_a.
        for _ in 0..20 {
            selector.record_outcome("oracle_a", &OracleDomain::Coding, 0.9);
        }
        // Feed many failures to oracle_b.
        for _ in 0..20 {
            selector.record_outcome("oracle_b", &OracleDomain::Coding, 0.2);
        }

        // Deterministic selection by mean should prefer oracle_a.
        let best = selector.select_by_mean(&OracleDomain::Coding).unwrap();
        assert_eq!(best.oracle_id, "oracle_a");
        assert!(best.mean() > 0.5);
    }

    #[test]
    fn arms_for_domain_sorted_by_mean() {
        let mut selector = OracleSelector::new();
        selector.register("weak", OracleDomain::Chain);
        selector.register("strong", OracleDomain::Chain);

        for _ in 0..10 {
            selector.record_outcome("strong", &OracleDomain::Chain, 0.95);
            selector.record_outcome("weak", &OracleDomain::Chain, 0.3);
        }

        let arms = selector.arms_for_domain(&OracleDomain::Chain);
        assert_eq!(arms.len(), 2);
        assert_eq!(arms[0].oracle_id, "strong");
        assert!(arms[0].mean() > arms[1].mean());
    }

    #[test]
    fn discount_fades_old_evidence() {
        let mut arm = OracleArm::new("test", OracleDomain::Coding);
        arm.discount = 0.9; // Aggressive discount.

        // Build up strong success prior.
        for _ in 0..50 {
            arm.update(0.9);
        }
        let high_mean = arm.mean();

        // Now feed failures.
        for _ in 0..50 {
            arm.update(0.1);
        }
        let low_mean = arm.mean();

        assert!(
            low_mean < high_mean,
            "mean should drop after failures: {low_mean} vs {high_mean}"
        );
    }

    #[test]
    fn oracle_arm_mean_starts_at_half() {
        let arm = OracleArm::new("fresh", OracleDomain::Research);
        assert!((arm.mean() - 0.5).abs() < 0.01);
    }

    // ─── MVT batch selection tests (TA-12 wiring) ─────────────────

    #[test]
    fn select_batch_returns_at_least_one() {
        let mut selector = OracleSelector::new();
        selector.register("only_one", OracleDomain::Coding);
        let batch = selector.select_batch(&OracleDomain::Coding, 0.0, 0.85);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].oracle_id, "only_one");
    }

    #[test]
    fn select_batch_empty_for_unknown_domain() {
        let selector = OracleSelector::new();
        let batch = selector.select_batch(&OracleDomain::Chain, 0.0, 0.85);
        assert!(batch.is_empty());
    }

    #[test]
    fn select_batch_stops_on_high_sufficiency() {
        let mut selector = OracleSelector::new();
        selector.register("oracle_a", OracleDomain::Coding);
        selector.register("oracle_b", OracleDomain::Coding);
        selector.register("oracle_c", OracleDomain::Coding);

        // High sufficiency should stop after the first oracle.
        let batch = selector.select_batch(&OracleDomain::Coding, 0.95, 0.85);
        assert_eq!(batch.len(), 1, "should stop early with high sufficiency");
    }

    #[test]
    fn select_batch_prefers_high_accuracy_oracles() {
        let mut selector = OracleSelector::new();
        selector.register("strong", OracleDomain::Coding);
        selector.register("weak", OracleDomain::Coding);

        for _ in 0..20 {
            selector.record_outcome("strong", &OracleDomain::Coding, 0.95);
            selector.record_outcome("weak", &OracleDomain::Coding, 0.2);
        }

        let batch = selector.select_batch(&OracleDomain::Coding, 0.0, 0.85);
        assert!(!batch.is_empty());
        // The first oracle in the batch should be the strongest.
        assert_eq!(batch[0].oracle_id, "strong");
    }
}
