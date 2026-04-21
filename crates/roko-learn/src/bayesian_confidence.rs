//! Bayesian confidence updating (AS-07).
//!
//! Provides a lightweight Bayesian updater that maintains a confidence estimate
//! (a beta distribution parameterized by alpha/beta) and updates it as new
//! evidence arrives. This is used by the learning subsystem to track confidence
//! in hypotheses, model quality estimates, and gate reliability.
//!
//! The updater uses the conjugate Beta-Binomial model:
//! - Prior: Beta(alpha, beta)
//! - Observation: success (1) or failure (0)
//! - Posterior: Beta(alpha + successes, beta + failures)
//!
//! The mean confidence is `alpha / (alpha + beta)`.

use serde::{Deserialize, Serialize};

/// Bayesian confidence updater using a Beta distribution.
///
/// Maintains a running posterior over a Bernoulli success probability.
/// The confidence (expected probability of success) is the mean of the
/// Beta distribution: `alpha / (alpha + beta)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BayesianConfidenceUpdater {
    /// Beta distribution alpha parameter (pseudo-count of successes + prior).
    pub alpha: f64,
    /// Beta distribution beta parameter (pseudo-count of failures + prior).
    pub beta: f64,
    /// Total observations incorporated.
    pub observations: u64,
    /// Optional label for what this updater tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Default for BayesianConfidenceUpdater {
    fn default() -> Self {
        Self::uniform()
    }
}

impl BayesianConfidenceUpdater {
    /// Construct with a uniform prior: Beta(1, 1).
    /// Initial confidence = 0.5.
    #[must_use]
    pub fn uniform() -> Self {
        Self {
            alpha: 1.0,
            beta: 1.0,
            observations: 0,
            label: None,
        }
    }

    /// Construct with a custom prior.
    ///
    /// `alpha` and `beta` must be positive. Values are clamped to `[0.01, ...]`.
    #[must_use]
    pub fn with_prior(alpha: f64, beta: f64) -> Self {
        Self {
            alpha: alpha.max(0.01),
            beta: beta.max(0.01),
            observations: 0,
            label: None,
        }
    }

    /// Construct with an informative prior that encodes a prior belief.
    ///
    /// `prior_confidence` is the expected probability (0.0..1.0).
    /// `strength` controls how many pseudo-observations the prior is worth.
    /// Higher strength = prior is harder to override with new data.
    #[must_use]
    pub fn with_informative_prior(prior_confidence: f64, strength: f64) -> Self {
        let p = prior_confidence.clamp(0.01, 0.99);
        let s = strength.max(0.1);
        Self {
            alpha: p * s,
            beta: (1.0 - p) * s,
            observations: 0,
            label: None,
        }
    }

    /// Attach a label to this updater.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Current confidence estimate (posterior mean).
    /// Returns `alpha / (alpha + beta)`.
    #[must_use]
    pub fn confidence(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Variance of the posterior distribution.
    /// Lower variance = more confident in the estimate.
    #[must_use]
    pub fn variance(&self) -> f64 {
        let total = self.alpha + self.beta;
        (self.alpha * self.beta) / (total * total * (total + 1.0))
    }

    /// Standard deviation of the posterior.
    #[must_use]
    pub fn std_dev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Credible interval at the given level (e.g., 0.95 for 95% CI).
    ///
    /// Uses a normal approximation for simplicity.
    /// Returns `(lower, upper)`.
    #[must_use]
    pub fn credible_interval(&self, level: f64) -> (f64, f64) {
        let z = match level {
            l if l >= 0.99 => 2.576,
            l if l >= 0.95 => 1.96,
            l if l >= 0.90 => 1.645,
            l if l >= 0.80 => 1.282,
            _ => 1.0,
        };
        let mean = self.confidence();
        let sd = self.std_dev();
        (
            (mean - z * sd).clamp(0.0, 1.0),
            (mean + z * sd).clamp(0.0, 1.0),
        )
    }

    /// Update with a single binary observation.
    ///
    /// `success = true` increments alpha, `success = false` increments beta.
    pub fn observe(&mut self, success: bool) {
        if success {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
        self.observations += 1;
    }

    /// Update with a batch of observations.
    pub fn observe_batch(&mut self, successes: u64, failures: u64) {
        self.alpha += successes as f64;
        self.beta += failures as f64;
        self.observations += successes + failures;
    }

    /// Update with a weighted (soft) observation.
    ///
    /// `weight` in `[0.0, 1.0]` is distributed between alpha and beta.
    /// Weight of 1.0 = full success, 0.0 = full failure.
    pub fn observe_weighted(&mut self, weight: f64) {
        let w = weight.clamp(0.0, 1.0);
        self.alpha += w;
        self.beta += 1.0 - w;
        self.observations += 1;
    }

    /// Merge another updater into this one (combine evidence).
    ///
    /// Subtracts the shared prior to avoid double-counting.
    pub fn merge(&mut self, other: &BayesianConfidenceUpdater) {
        // Subtract the default prior of Beta(1,1) from the other to avoid
        // double-counting, then add the other's evidence.
        self.alpha += (other.alpha - 1.0).max(0.0);
        self.beta += (other.beta - 1.0).max(0.0);
        self.observations += other.observations;
    }

    /// Reset to the uniform prior, discarding all observations.
    pub fn reset(&mut self) {
        self.alpha = 1.0;
        self.beta = 1.0;
        self.observations = 0;
    }

    /// Effective sample size (total pseudo-observations).
    #[must_use]
    pub fn effective_sample_size(&self) -> f64 {
        self.alpha + self.beta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_prior_starts_at_half() {
        let u = BayesianConfidenceUpdater::uniform();
        assert!((u.confidence() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn successes_increase_confidence() {
        let mut u = BayesianConfidenceUpdater::uniform();
        let initial = u.confidence();
        u.observe(true);
        assert!(u.confidence() > initial);
    }

    #[test]
    fn failures_decrease_confidence() {
        let mut u = BayesianConfidenceUpdater::uniform();
        let initial = u.confidence();
        u.observe(false);
        assert!(u.confidence() < initial);
    }

    #[test]
    fn batch_update() {
        let mut u = BayesianConfidenceUpdater::uniform();
        u.observe_batch(8, 2);
        // Alpha = 9, Beta = 3 -> confidence = 9/12 = 0.75.
        assert!((u.confidence() - 0.75).abs() < 1e-10);
        assert_eq!(u.observations, 10);
    }

    #[test]
    fn weighted_observation() {
        let mut u = BayesianConfidenceUpdater::uniform();
        u.observe_weighted(0.7);
        // Alpha = 1.7, Beta = 1.3 -> confidence = 1.7/3.0.
        assert!((u.confidence() - 1.7 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn variance_decreases_with_more_data() {
        let mut u = BayesianConfidenceUpdater::uniform();
        let initial_var = u.variance();
        u.observe_batch(10, 10);
        assert!(u.variance() < initial_var);
    }

    #[test]
    fn credible_interval_narrows_with_data() {
        let u_prior = BayesianConfidenceUpdater::uniform();
        let (lo1, hi1) = u_prior.credible_interval(0.95);
        let width1 = hi1 - lo1;

        let mut u_data = BayesianConfidenceUpdater::uniform();
        u_data.observe_batch(50, 50);
        let (lo2, hi2) = u_data.credible_interval(0.95);
        let width2 = hi2 - lo2;

        assert!(width2 < width1, "CI should narrow with more data");
    }

    #[test]
    fn informative_prior() {
        let u = BayesianConfidenceUpdater::with_informative_prior(0.8, 10.0);
        assert!((u.confidence() - 0.8).abs() < 1e-10);
        assert!((u.alpha - 8.0).abs() < 1e-10);
        assert!((u.beta - 2.0).abs() < 1e-10);
    }

    #[test]
    fn merge_combines_evidence() {
        let mut a = BayesianConfidenceUpdater::uniform();
        a.observe_batch(5, 0);
        let mut b = BayesianConfidenceUpdater::uniform();
        b.observe_batch(3, 2);
        a.merge(&b);
        // a: alpha = 6 + 3 = 9, beta = 1 + 2 = 3 -> confidence = 9/12 = 0.75.
        assert!((a.confidence() - 0.75).abs() < 1e-10);
    }

    #[test]
    fn reset_returns_to_uniform() {
        let mut u = BayesianConfidenceUpdater::uniform();
        u.observe_batch(100, 0);
        u.reset();
        assert!((u.confidence() - 0.5).abs() < 1e-10);
        assert_eq!(u.observations, 0);
    }

    #[test]
    fn label_preserved() {
        let u = BayesianConfidenceUpdater::uniform().with_label("gate_reliability");
        assert_eq!(u.label.as_deref(), Some("gate_reliability"));
    }
}
