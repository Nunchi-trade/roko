//! Nelson-Siegel yield curve model for DeFi oracle rate term structure.
//!
//! The Nelson-Siegel model parametrizes the yield curve with 4 parameters:
//! - `beta0`: long-term level (asymptotic rate as maturity → ∞)
//! - `beta1`: short-term slope (difference between short and long rates)
//! - `beta2`: medium-term hump (curvature of the yield curve)
//! - `tau`: decay factor (controls where the hump peaks)
//!
//! ## Formula
//!
//! ```text
//! y(t) = beta0 + beta1 * (1 - exp(-t/tau)) / (t/tau)
//!              + beta2 * ((1 - exp(-t/tau)) / (t/tau) - exp(-t/tau))
//! ```
//!
//! Where `t` is the time to maturity (in the same units as `tau`).
//!
//! ## Usage
//!
//! ```rust
//! use roko_chain::nelson_siegel::NelsonSiegel;
//!
//! // Typical upward-sloping curve
//! let curve = NelsonSiegel::new(0.05, -0.02, 0.01, 1.5);
//! let rate_1y = curve.rate(1.0);   // ~3.5%
//! let rate_10y = curve.rate(10.0); // ~4.8%
//! ```

use serde::{Deserialize, Serialize};

/// Nelson-Siegel parametric yield curve model.
///
/// Models the term structure of interest rates with 4 interpretable parameters.
/// Used for PT/YT (principal token / yield token) pricing in the DeFi oracle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NelsonSiegel {
    /// Long-term level (asymptotic rate as maturity → ∞).
    pub beta0: f64,
    /// Short-term slope (short rate = beta0 + beta1).
    pub beta1: f64,
    /// Medium-term hump (curvature).
    pub beta2: f64,
    /// Decay factor controlling hump location (in years or matching maturity units).
    pub tau: f64,
}

impl NelsonSiegel {
    /// Create a new Nelson-Siegel curve.
    pub fn new(beta0: f64, beta1: f64, beta2: f64, tau: f64) -> Self {
        Self {
            beta0,
            beta1,
            beta2,
            tau: tau.max(0.001), // Prevent division by zero
        }
    }

    /// Compute the yield (rate) at a given maturity.
    ///
    /// `maturity` should be in the same time units as `tau` (typically years).
    /// Returns the annualized yield at that maturity point.
    pub fn rate(&self, maturity: f64) -> f64 {
        if maturity <= 0.0 {
            // Instantaneous rate = beta0 + beta1
            return self.beta0 + self.beta1;
        }

        let x = maturity / self.tau;
        let exp_neg_x = (-x).exp();
        let loading1 = (1.0 - exp_neg_x) / x;
        let loading2 = loading1 - exp_neg_x;

        self.beta0 + self.beta1 * loading1 + self.beta2 * loading2
    }

    /// Compute the forward rate at a given maturity.
    ///
    /// The instantaneous forward rate f(t) = d(t*y(t))/dt.
    pub fn forward_rate(&self, maturity: f64) -> f64 {
        if maturity <= 0.0 {
            return self.beta0 + self.beta1;
        }

        let x = maturity / self.tau;
        let exp_neg_x = (-x).exp();

        self.beta0 + self.beta1 * exp_neg_x + self.beta2 * x * exp_neg_x
    }

    /// Compute rates for a vector of maturities.
    pub fn rate_curve(&self, maturities: &[f64]) -> Vec<f64> {
        maturities.iter().map(|&t| self.rate(t)).collect()
    }

    /// The short rate (instantaneous, maturity → 0).
    pub fn short_rate(&self) -> f64 {
        self.beta0 + self.beta1
    }

    /// The long rate (asymptotic, maturity → ∞).
    pub fn long_rate(&self) -> f64 {
        self.beta0
    }

    /// Spread between long and short rate.
    pub fn term_spread(&self) -> f64 {
        self.long_rate() - self.short_rate()
    }

    /// Maturity at which the hump peaks (approximate).
    pub fn hump_maturity(&self) -> f64 {
        self.tau
    }

    /// Fit a Nelson-Siegel curve to observed (maturity, rate) pairs using least squares.
    ///
    /// Uses a simple grid search over tau followed by linear regression for betas.
    /// This is not a production-grade optimizer but is sufficient for the oracle use case.
    ///
    /// Returns `None` if fewer than 4 observations are provided.
    pub fn fit(observations: &[(f64, f64)]) -> Option<Self> {
        if observations.len() < 4 {
            return None;
        }

        let mut best_sse = f64::MAX;
        let mut best = Self::new(0.0, 0.0, 0.0, 1.0);

        // Grid search over tau in [0.1, 30.0] with 100 steps.
        for i in 1..=100 {
            let tau = 0.1 + (i as f64 / 100.0) * 29.9;

            // For fixed tau, compute factor loadings and solve via OLS.
            let n = observations.len() as f64;
            let mut sum_y = 0.0;
            let mut sum_l1 = 0.0;
            let mut sum_l2 = 0.0;
            let mut sum_l1_l1 = 0.0;
            let mut sum_l2_l2 = 0.0;
            let mut sum_l1_l2 = 0.0;
            let mut sum_y_l1 = 0.0;
            let mut sum_y_l2 = 0.0;
            let mut sum_l1_y = 0.0;

            for &(t, y) in observations {
                let x = t / tau;
                let exp_neg_x = (-x).exp();
                let l1 = if x > 0.001 {
                    (1.0 - exp_neg_x) / x
                } else {
                    1.0
                };
                let l2 = l1 - exp_neg_x;

                sum_y += y;
                sum_l1 += l1;
                sum_l2 += l2;
                sum_l1_l1 += l1 * l1;
                sum_l2_l2 += l2 * l2;
                sum_l1_l2 += l1 * l2;
                sum_y_l1 += y * l1;
                sum_y_l2 += y * l2;
                sum_l1_y += l1 * y;
            }

            // Simple estimation: beta0 ≈ mean(y - beta1*l1 - beta2*l2)
            // For simplicity, use mean regression.
            let mean_y = sum_y / n;
            let mean_l1 = sum_l1 / n;
            let mean_l2 = sum_l2 / n;

            // Approximate beta1 and beta2 from centered moments.
            let denom = (sum_l1_l1 - n * mean_l1 * mean_l1) * (sum_l2_l2 - n * mean_l2 * mean_l2)
                - (sum_l1_l2 - n * mean_l1 * mean_l2).powi(2);

            if denom.abs() < 1e-12 {
                continue;
            }

            let beta1 = ((sum_y_l1 - n * mean_y * mean_l1) * (sum_l2_l2 - n * mean_l2 * mean_l2)
                - (sum_y_l2 - n * mean_y * mean_l2) * (sum_l1_l2 - n * mean_l1 * mean_l2))
                / denom;

            let beta2 = ((sum_y_l2 - n * mean_y * mean_l2) * (sum_l1_l1 - n * mean_l1 * mean_l1)
                - (sum_l1_y - n * mean_l1 * mean_y) * (sum_l1_l2 - n * mean_l1 * mean_l2))
                / denom;

            let beta0 = mean_y - beta1 * mean_l1 - beta2 * mean_l2;

            let candidate = Self::new(beta0, beta1, beta2, tau);

            // Compute SSE.
            let sse: f64 = observations
                .iter()
                .map(|&(t, y)| (candidate.rate(t) - y).powi(2))
                .sum();

            if sse < best_sse {
                best_sse = sse;
                best = candidate;
            }
        }

        Some(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_rate_equals_beta0_plus_beta1() {
        let curve = NelsonSiegel::new(0.05, -0.02, 0.01, 1.5);
        assert!((curve.short_rate() - 0.03).abs() < 1e-10);
    }

    #[test]
    fn long_rate_equals_beta0() {
        let curve = NelsonSiegel::new(0.05, -0.02, 0.01, 1.5);
        assert!((curve.long_rate() - 0.05).abs() < 1e-10);
    }

    #[test]
    fn upward_sloping_curve() {
        // beta1 < 0 → short rate < long rate → upward sloping
        let curve = NelsonSiegel::new(0.05, -0.03, 0.01, 1.5);
        let short = curve.rate(0.25);
        let long = curve.rate(10.0);
        assert!(long > short, "upward sloping: short={short}, long={long}");
    }

    #[test]
    fn rate_at_zero_maturity() {
        let curve = NelsonSiegel::new(0.05, -0.02, 0.01, 1.5);
        let rate = curve.rate(0.0);
        assert!((rate - curve.short_rate()).abs() < 1e-10);
    }

    #[test]
    fn rate_converges_to_beta0_at_long_maturity() {
        let curve = NelsonSiegel::new(0.05, -0.02, 0.01, 1.5);
        let rate = curve.rate(100.0);
        assert!(
            (rate - 0.05).abs() < 0.001,
            "should converge to beta0=0.05, got {rate}"
        );
    }

    #[test]
    fn flat_curve_when_beta1_and_beta2_are_zero() {
        let curve = NelsonSiegel::new(0.04, 0.0, 0.0, 1.0);
        assert!((curve.rate(1.0) - 0.04).abs() < 1e-10);
        assert!((curve.rate(10.0) - 0.04).abs() < 1e-10);
    }

    #[test]
    fn rate_curve_computes_vector() {
        let curve = NelsonSiegel::new(0.05, -0.02, 0.01, 1.5);
        let maturities = vec![0.5, 1.0, 2.0, 5.0, 10.0];
        let rates = curve.rate_curve(&maturities);
        assert_eq!(rates.len(), 5);
        // Should be monotonically increasing for this upward-sloping curve.
        for w in rates.windows(2) {
            assert!(
                w[1] >= w[0],
                "should be non-decreasing: {} >= {}",
                w[1],
                w[0]
            );
        }
    }

    #[test]
    fn fit_recovers_parameters_approximately() {
        let true_curve = NelsonSiegel::new(0.05, -0.03, 0.02, 2.0);
        let maturities = vec![0.25, 0.5, 1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 20.0, 30.0];
        let observations: Vec<(f64, f64)> = maturities
            .iter()
            .map(|&t| (t, true_curve.rate(t)))
            .collect();

        let fitted = NelsonSiegel::fit(&observations).unwrap();

        // Check rates match within 10bp at key maturities.
        for &t in &[1.0, 5.0, 10.0] {
            let true_rate = true_curve.rate(t);
            let fitted_rate = fitted.rate(t);
            assert!(
                (true_rate - fitted_rate).abs() < 0.001,
                "at t={t}: true={true_rate:.4}, fitted={fitted_rate:.4}"
            );
        }
    }

    #[test]
    fn fit_returns_none_for_too_few_points() {
        let obs = vec![(1.0, 0.03), (5.0, 0.04)];
        assert!(NelsonSiegel::fit(&obs).is_none());
    }

    #[test]
    fn term_spread() {
        let curve = NelsonSiegel::new(0.05, -0.03, 0.0, 1.0);
        assert!((curve.term_spread() - 0.03).abs() < 1e-10);
    }
}
