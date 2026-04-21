//! Multi-gate coordination via Hotelling's T-squared statistic.
//!
//! When multiple gates shift together (e.g., compile AND lint pass rates both
//! drop), this signals a systemic problem rather than a gate-specific issue.
//! Hotelling's T-squared is the multivariate extension of the t-test that
//! detects joint anomalies across the gate vector.
//!
//! Formula: T² = n × (x̄ - μ)ᵀ × S⁻¹ × (x̄ - μ)
//!
//! where x̄ is the current gate pass rate vector, μ is the historical mean,
//! S is the covariance matrix, and n is the sample size.

use serde::{Deserialize, Serialize};

/// Result of a joint anomaly check.
#[derive(Debug, Clone, PartialEq)]
pub struct JointAnomalyResult {
    /// The computed T-squared statistic.
    pub t_squared: f64,
    /// The threshold used for comparison.
    pub threshold: f64,
    /// Whether the statistic exceeds the threshold.
    pub is_anomalous: bool,
}

/// Multi-gate joint anomaly detector using Hotelling's T-squared statistic.
///
/// Tracks pass rate vectors across pipeline runs and detects when multiple
/// gates shift simultaneously, indicating systemic rather than gate-specific
/// issues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotellingDetector {
    /// Number of gate types tracked (dimensionality of the observation vector).
    dimension: usize,
    /// Running mean per gate.
    mean: Vec<f64>,
    /// Covariance matrix (flattened p×p, row-major).
    covariance: Vec<f64>,
    /// Total observations ingested.
    observations: usize,
    /// Chi-squared critical value threshold for the configured alpha.
    threshold: f64,
    /// Running sum of (x_i - mean)(x_j - mean) products for online covariance.
    /// Uses Welford's online algorithm extended to covariance.
    m2: Vec<f64>,
}

impl HotellingDetector {
    /// Create a new detector for `dimension` gates at significance level `alpha`.
    ///
    /// The `alpha` parameter controls the false alarm rate. Common values:
    /// - 0.05 for 95% confidence
    /// - 0.01 for 99% confidence
    ///
    /// The critical value is approximated from the chi-squared distribution
    /// with `dimension` degrees of freedom using the Wilson-Hilferty
    /// approximation.
    #[must_use]
    pub fn new(dimension: usize, alpha: f64) -> Self {
        let dim = dimension.max(1);
        let threshold = chi_squared_critical(dim, alpha);
        Self {
            dimension: dim,
            mean: vec![0.0; dim],
            covariance: identity_matrix(dim),
            observations: 0,
            threshold,
            m2: vec![0.0; dim * dim],
        }
    }

    /// Update the detector with a new observation vector of gate pass rates.
    ///
    /// Each element should be 0.0 (fail) or 1.0 (pass), though intermediate
    /// values (e.g., partial scores) are also accepted.
    ///
    /// Uses Welford's online algorithm extended to multivariate data for
    /// numerically stable incremental mean and covariance updates.
    pub fn update(&mut self, gate_pass_rates: &[f64]) {
        if gate_pass_rates.len() != self.dimension {
            return;
        }

        self.observations += 1;
        let n = self.observations as f64;

        // Welford's: compute delta from old mean.
        let delta: Vec<f64> = gate_pass_rates
            .iter()
            .zip(self.mean.iter())
            .map(|(x, m)| x - m)
            .collect();

        // Update mean.
        for (m, d) in self.mean.iter_mut().zip(delta.iter()) {
            *m += d / n;
        }

        // Compute delta from new mean.
        let delta2: Vec<f64> = gate_pass_rates
            .iter()
            .zip(self.mean.iter())
            .map(|(x, m)| x - m)
            .collect();

        // Update M2 matrix: M2[i][j] += delta[i] * delta2[j].
        for i in 0..self.dimension {
            for j in 0..self.dimension {
                self.m2[i * self.dimension + j] += delta[i] * delta2[j];
            }
        }

        // Recompute covariance from M2 (unbiased: divide by n-1).
        if self.observations >= 2 {
            let denom = n - 1.0;
            for i in 0..self.dimension {
                for j in 0..self.dimension {
                    self.covariance[i * self.dimension + j] =
                        self.m2[i * self.dimension + j] / denom;
                }
            }
        }
    }

    /// Compute the T-squared statistic for a given observation vector.
    ///
    /// Returns `f64::INFINITY` if the covariance matrix is singular (not
    /// enough observations to estimate covariance).
    #[must_use]
    pub fn t_squared(&self, current: &[f64]) -> f64 {
        if current.len() != self.dimension || self.observations < self.dimension + 1 {
            return 0.0;
        }

        // Deviation from mean: d = current - mean.
        let d: Vec<f64> = current
            .iter()
            .zip(self.mean.iter())
            .map(|(x, m)| x - m)
            .collect();

        // Invert covariance matrix.
        let Some(inv) = invert_matrix(&self.covariance, self.dimension) else {
            return f64::INFINITY;
        };

        // T² = n × dᵀ × S⁻¹ × d.
        let n = self.observations as f64;
        let mut result = 0.0;
        for i in 0..self.dimension {
            let mut inner = 0.0;
            for j in 0..self.dimension {
                inner += inv[i * self.dimension + j] * d[j];
            }
            result += d[i] * inner;
        }
        result * n
    }

    /// Check whether the current observation is anomalous.
    #[must_use]
    pub fn is_anomalous(&self, current: &[f64]) -> bool {
        self.t_squared(current) > self.threshold
    }

    /// Check the current observation and return a detailed result.
    #[must_use]
    pub fn check(&self, current: &[f64]) -> JointAnomalyResult {
        let t_sq = self.t_squared(current);
        JointAnomalyResult {
            t_squared: t_sq,
            threshold: self.threshold,
            is_anomalous: t_sq > self.threshold,
        }
    }

    /// Return the current mean vector.
    #[must_use]
    pub fn mean(&self) -> &[f64] {
        &self.mean
    }

    /// Return the number of observations processed.
    #[must_use]
    pub fn observations(&self) -> usize {
        self.observations
    }

    /// Return the configured threshold.
    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold
    }
}

/// Chi-squared critical value approximation using Wilson-Hilferty.
///
/// For k degrees of freedom and significance level alpha:
/// χ²_α,k ≈ k × (1 - 2/(9k) + z_α × sqrt(2/(9k)))³
///
/// where z_α is the standard normal quantile.
fn chi_squared_critical(k: usize, alpha: f64) -> f64 {
    let k = k as f64;
    // Standard normal quantile approximation (Beasley-Springer-Moro).
    let z = normal_quantile(1.0 - alpha);
    let term = 2.0 / (9.0 * k);
    let base = 1.0 - term + z * term.sqrt();
    (k * base * base * base).max(0.0)
}

/// Approximate standard normal quantile via rational approximation.
/// Abramowitz and Stegun formula 26.2.23.
fn normal_quantile(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    if (p - 0.5).abs() < f64::EPSILON {
        return 0.0;
    }

    let sign = if p < 0.5 { -1.0 } else { 1.0 };
    let p = if p < 0.5 { p } else { 1.0 - p };

    let t = (-2.0 * p.ln()).sqrt();
    let c0 = 2.515517;
    let c1 = 0.802853;
    let c2 = 0.010328;
    let d1 = 1.432788;
    let d2 = 0.189269;
    let d3 = 0.001308;

    sign * (t - (c0 + c1 * t + c2 * t * t) / (1.0 + d1 * t + d2 * t * t + d3 * t * t * t))
}

/// Create a p×p identity matrix (flattened row-major).
fn identity_matrix(p: usize) -> Vec<f64> {
    let mut m = vec![0.0; p * p];
    for i in 0..p {
        m[i * p + i] = 1.0;
    }
    m
}

/// Invert a p×p matrix via Gauss-Jordan elimination.
///
/// Returns `None` if the matrix is singular.
fn invert_matrix(matrix: &[f64], p: usize) -> Option<Vec<f64>> {
    let mut aug = vec![0.0; p * 2 * p];

    // Build augmented matrix [A | I].
    for i in 0..p {
        for j in 0..p {
            aug[i * (2 * p) + j] = matrix[i * p + j];
        }
        aug[i * (2 * p) + p + i] = 1.0;
    }

    // Forward elimination with partial pivoting.
    for col in 0..p {
        // Find pivot.
        let mut max_row = col;
        let mut max_val = aug[col * (2 * p) + col].abs();
        for row in (col + 1)..p {
            let val = aug[row * (2 * p) + col].abs();
            if val > max_val {
                max_val = val;
                max_row = row;
            }
        }

        if max_val < 1e-12 {
            return None; // Singular.
        }

        // Swap rows.
        if max_row != col {
            for j in 0..(2 * p) {
                let a = col * (2 * p) + j;
                let b = max_row * (2 * p) + j;
                aug.swap(a, b);
            }
        }

        // Scale pivot row.
        let pivot = aug[col * (2 * p) + col];
        for j in 0..(2 * p) {
            aug[col * (2 * p) + j] /= pivot;
        }

        // Eliminate column.
        for row in 0..p {
            if row == col {
                continue;
            }
            let factor = aug[row * (2 * p) + col];
            for j in 0..(2 * p) {
                aug[row * (2 * p) + j] -= factor * aug[col * (2 * p) + j];
            }
        }
    }

    // Extract inverse from augmented matrix.
    let mut inv = vec![0.0; p * p];
    for i in 0..p {
        for j in 0..p {
            inv[i * p + j] = aug[i * (2 * p) + p + j];
        }
    }

    Some(inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_detector_has_correct_dimension() {
        let det = HotellingDetector::new(3, 0.05);
        assert_eq!(det.dimension, 3);
        assert_eq!(det.observations(), 0);
        assert!(det.threshold() > 0.0);
    }

    #[test]
    fn update_accumulates_observations() {
        let mut det = HotellingDetector::new(2, 0.05);
        det.update(&[1.0, 0.0]);
        det.update(&[0.0, 1.0]);
        det.update(&[1.0, 1.0]);
        assert_eq!(det.observations(), 3);
    }

    #[test]
    fn normal_baseline_is_not_anomalous() {
        let mut det = HotellingDetector::new(2, 0.05);
        // Train on a baseline with independent variation in each dimension
        // to produce a non-singular covariance matrix.
        for _ in 0..30 {
            det.update(&[0.80, 0.90]);
        }
        for _ in 0..30 {
            det.update(&[0.85, 0.90]);
        }
        for _ in 0..30 {
            det.update(&[0.80, 0.85]);
        }
        for _ in 0..30 {
            det.update(&[0.85, 0.85]);
        }
        // The mean is [0.825, 0.875]; a point very close should not be anomalous.
        assert!(!det.is_anomalous(&[0.825, 0.875]));
    }

    #[test]
    fn joint_shift_triggers_anomaly() {
        let mut det = HotellingDetector::new(2, 0.05);
        // Establish stable baseline around (0.9, 0.9).
        for _ in 0..100 {
            det.update(&[0.9, 0.9]);
        }
        // Inject small variance.
        for _ in 0..50 {
            det.update(&[0.88, 0.92]);
        }
        for _ in 0..50 {
            det.update(&[0.92, 0.88]);
        }
        // A large simultaneous drop should trigger.
        let result = det.check(&[0.2, 0.1]);
        assert!(result.is_anomalous);
        assert!(result.t_squared > result.threshold);
    }

    #[test]
    fn check_returns_detailed_result() {
        let mut det = HotellingDetector::new(2, 0.05);
        // Use four observation clusters with independent variation in each
        // dimension to ensure a non-singular covariance matrix.
        for _ in 0..10 {
            det.update(&[0.90, 0.85]);
        }
        for _ in 0..10 {
            det.update(&[0.88, 0.85]);
        }
        for _ in 0..10 {
            det.update(&[0.90, 0.87]);
        }
        for _ in 0..10 {
            det.update(&[0.88, 0.87]);
        }
        // Mean is [0.89, 0.86]; check exactly at the mean.
        let result = det.check(&[0.89, 0.86]);
        assert!(!result.is_anomalous);
        assert!(result.t_squared >= 0.0);
    }

    #[test]
    fn insufficient_observations_return_zero() {
        let det = HotellingDetector::new(3, 0.05);
        assert_eq!(det.t_squared(&[0.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn identity_matrix_is_correct() {
        let m = identity_matrix(3);
        assert_eq!(m.len(), 9);
        assert_eq!(m[0], 1.0);
        assert_eq!(m[4], 1.0);
        assert_eq!(m[8], 1.0);
        assert_eq!(m[1], 0.0);
    }

    #[test]
    fn invert_identity_yields_identity() {
        let m = identity_matrix(3);
        let inv = invert_matrix(&m, 3).unwrap();
        for i in 0..3 {
            for j in 0..3 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (inv[i * 3 + j] - expected).abs() < 1e-10,
                    "inv[{i}][{j}] = {}, expected {expected}",
                    inv[i * 3 + j]
                );
            }
        }
    }

    #[test]
    fn chi_squared_critical_reasonable_values() {
        // For k=2, alpha=0.05, the critical value should be approximately 5.99.
        let crit = chi_squared_critical(2, 0.05);
        assert!(crit > 4.0 && crit < 8.0, "chi2 critical = {crit}");
    }
}
