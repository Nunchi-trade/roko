//! PELT (Pruned Exact Linear Time) offline change point detection.
//!
//! Detects structural breaks in a time series of observations (e.g., gate
//! pass rates, latency, cost metrics). Unlike online detectors (CUSUM, EWMA),
//! PELT processes a full batch of data and returns all change points.
//!
//! ## Algorithm
//!
//! PELT minimizes: `sum_i cost(y[cp_i..cp_{i+1}]) + beta * num_changepoints`
//!
//! where `beta` is the penalty per change point (controls sensitivity).
//! Uses dynamic programming with pruning for O(n) average complexity.
//!
//! ## Cost Functions
//!
//! - **L2 (mean shift)**: detects changes in mean level
//! - **L1 (median shift)**: robust to outliers
//! - **Normal**: detects changes in mean and/or variance
//!
//! ## Usage
//!
//! ```rust
//! use roko_gate::pelt::{PeltDetector, CostFunction};
//!
//! let data = vec![1.0, 1.1, 0.9, 1.0, 5.0, 5.1, 4.9, 5.0];
//! let detector = PeltDetector::new(CostFunction::L2, 3.0);
//! let change_points = detector.detect(&data);
//! // change_points should contain index ~4 (where values jump from ~1 to ~5)
//! ```

use serde::{Deserialize, Serialize};

/// Cost function for measuring segment homogeneity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostFunction {
    /// L2 (squared error from segment mean). Detects mean shifts.
    L2,
    /// L1 (absolute error from segment median). Robust to outliers.
    L1,
    /// Normal log-likelihood. Detects changes in mean and/or variance.
    Normal,
}

impl CostFunction {
    /// Compute the cost of a segment `data[start..end]`.
    pub fn cost(&self, data: &[f64], start: usize, end: usize) -> f64 {
        if end <= start {
            return 0.0;
        }
        let segment = &data[start..end];
        let n = segment.len() as f64;

        match self {
            Self::L2 => {
                let mean = segment.iter().sum::<f64>() / n;
                segment.iter().map(|&x| (x - mean).powi(2)).sum()
            }
            Self::L1 => {
                let mut sorted = segment.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let median = sorted[sorted.len() / 2];
                segment.iter().map(|&x| (x - median).abs()).sum()
            }
            Self::Normal => {
                if n < 2.0 {
                    return 0.0;
                }
                let mean = segment.iter().sum::<f64>() / n;
                let variance = segment.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / n;
                if variance <= 1e-12 {
                    return 0.0;
                }
                // Negative log-likelihood (up to constants).
                n * variance.ln()
            }
        }
    }
}

/// A detected change point with metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangePoint {
    /// Index in the data where the change occurs.
    pub index: usize,
    /// Cost improvement (how much the total cost decreased by splitting here).
    pub improvement: f64,
}

/// PELT change point detector.
///
/// Uses dynamic programming with pruning for O(n) average-case detection
/// of all change points in a batch of observations.
#[derive(Debug, Clone)]
pub struct PeltDetector {
    /// Cost function to use.
    pub cost_fn: CostFunction,
    /// Penalty per change point. Higher = fewer detected changes.
    /// A good default is `2 * ln(n)` (BIC-like) or a fixed value like 3.0.
    pub penalty: f64,
    /// Minimum segment length between change points.
    pub min_segment_len: usize,
}

impl PeltDetector {
    /// Create a new PELT detector.
    pub fn new(cost_fn: CostFunction, penalty: f64) -> Self {
        Self {
            cost_fn,
            penalty: penalty.max(0.0),
            min_segment_len: 2,
        }
    }

    /// Create with BIC-like penalty scaled to data length.
    pub fn with_bic_penalty(cost_fn: CostFunction, n: usize) -> Self {
        let penalty = 2.0 * (n as f64).ln().max(1.0);
        Self::new(cost_fn, penalty)
    }

    /// Set minimum segment length.
    #[must_use]
    pub fn with_min_segment(mut self, min_len: usize) -> Self {
        self.min_segment_len = min_len.max(1);
        self
    }

    /// Detect all change points in the data.
    ///
    /// Returns change points sorted by index. Empty if no significant
    /// changes detected (or data too short).
    pub fn detect(&self, data: &[f64]) -> Vec<ChangePoint> {
        let n = data.len();
        if n < self.min_segment_len * 2 {
            return Vec::new();
        }

        // Dynamic programming: f[t] = min cost of segmenting data[0..t].
        let mut f = vec![f64::MAX; n + 1];
        f[0] = -self.penalty; // So first segment gets penalty "free".

        // Backtrack pointers: which index was the best predecessor.
        let mut prev = vec![0usize; n + 1];

        // Candidate set for pruning (PELT acceleration).
        let mut candidates: Vec<usize> = vec![0];

        for t in self.min_segment_len..=n {
            let mut best_cost = f64::MAX;
            let mut best_prev = 0;

            for &s in &candidates {
                if t - s < self.min_segment_len {
                    continue;
                }
                let segment_cost = self.cost_fn.cost(data, s, t);
                let total = f[s] + segment_cost + self.penalty;

                if total < best_cost {
                    best_cost = total;
                    best_prev = s;
                }
            }

            f[t] = best_cost;
            prev[t] = best_prev;

            // PELT pruning: remove candidates that can never be optimal.
            candidates.retain(|&s| f[s] + self.cost_fn.cost(data, s, t) + self.penalty <= f[t]);
            candidates.push(t);
        }

        // Backtrack to find change points.
        let mut change_points = Vec::new();
        let mut t = n;
        while t > 0 {
            let s = prev[t];
            if s > 0 {
                // The cost improvement of splitting at s vs. not splitting.
                let unsplit_cost = self.cost_fn.cost(data, 0, t);
                let split_cost = self.cost_fn.cost(data, 0, s) + self.cost_fn.cost(data, s, t);
                let improvement = unsplit_cost - split_cost;

                change_points.push(ChangePoint {
                    index: s,
                    improvement: improvement.max(0.0),
                });
            }
            t = s;
        }

        change_points.sort_by_key(|cp| cp.index);
        change_points
    }

    /// Detect change points and return segment means.
    ///
    /// Returns `(change_points, segment_means)` where `segment_means[i]`
    /// is the mean of data between `change_points[i-1]` and `change_points[i]`.
    pub fn detect_with_segments(&self, data: &[f64]) -> (Vec<ChangePoint>, Vec<f64>) {
        let cps = self.detect(data);
        let mut means = Vec::new();
        let mut boundaries: Vec<usize> = vec![0];
        for cp in &cps {
            boundaries.push(cp.index);
        }
        boundaries.push(data.len());

        for w in boundaries.windows(2) {
            let segment = &data[w[0]..w[1]];
            if segment.is_empty() {
                means.push(0.0);
            } else {
                means.push(segment.iter().sum::<f64>() / segment.len() as f64);
            }
        }

        (cps, means)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_single_mean_shift() {
        let mut data = vec![1.0; 50];
        data.extend(vec![5.0; 50]);

        let detector = PeltDetector::new(CostFunction::L2, 5.0);
        let cps = detector.detect(&data);

        assert!(
            !cps.is_empty(),
            "should detect change point in mean shift data"
        );
        // Change point should be near index 50.
        let cp_idx = cps[0].index;
        assert!(
            (45..=55).contains(&cp_idx),
            "change point should be near 50, got {cp_idx}"
        );
    }

    #[test]
    fn no_change_in_constant_data() {
        let data = vec![3.0; 100];
        let detector = PeltDetector::new(CostFunction::L2, 5.0);
        let cps = detector.detect(&data);
        assert!(
            cps.is_empty(),
            "constant data should have no change points, got {} points",
            cps.len()
        );
    }

    #[test]
    fn detects_multiple_shifts() {
        let mut data = vec![1.0; 30];
        data.extend(vec![5.0; 30]);
        data.extend(vec![2.0; 30]);

        let detector = PeltDetector::new(CostFunction::L2, 5.0);
        let cps = detector.detect(&data);

        assert!(
            cps.len() >= 2,
            "should detect at least 2 change points, got {}",
            cps.len()
        );
    }

    #[test]
    fn l1_cost_is_lower_than_l2_for_outliers() {
        // L1 (median-based) should give lower cost than L2 (mean-based)
        // for segments with outliers, since median is outlier-robust.
        let mut data = vec![1.0; 20];
        data[10] = 100.0; // Single outlier

        let l1_cost = CostFunction::L1.cost(&data, 0, data.len());
        let l2_cost = CostFunction::L2.cost(&data, 0, data.len());

        // L2 is much more affected by the outlier than L1.
        assert!(
            l1_cost < l2_cost,
            "L1 should be more robust: L1={l1_cost}, L2={l2_cost}"
        );
    }

    #[test]
    fn normal_cost_detects_variance_change() {
        let mut data = Vec::new();
        // Low variance segment.
        for i in 0..50 {
            data.push(5.0 + (i % 3) as f64 * 0.1);
        }
        // High variance segment.
        for i in 0..50 {
            data.push(5.0 + (i % 5) as f64 * 2.0);
        }

        let detector = PeltDetector::new(CostFunction::Normal, 10.0);
        let cps = detector.detect(&data);

        assert!(!cps.is_empty(), "Normal cost should detect variance change");
    }

    #[test]
    fn bic_penalty_scales_with_data_length() {
        let short = PeltDetector::with_bic_penalty(CostFunction::L2, 10);
        let long = PeltDetector::with_bic_penalty(CostFunction::L2, 1000);
        assert!(
            long.penalty > short.penalty,
            "BIC penalty should increase with data length"
        );
    }

    #[test]
    fn detect_with_segments_returns_means() {
        let mut data = vec![2.0; 30];
        data.extend(vec![8.0; 30]);

        let detector = PeltDetector::new(CostFunction::L2, 5.0);
        let (cps, means) = detector.detect_with_segments(&data);

        assert!(!cps.is_empty());
        assert_eq!(means.len(), cps.len() + 1);
        // First segment mean should be ~2, second ~8.
        assert!((means[0] - 2.0).abs() < 1.0);
        assert!((means[means.len() - 1] - 8.0).abs() < 1.0);
    }

    #[test]
    fn empty_data_returns_empty() {
        let detector = PeltDetector::new(CostFunction::L2, 5.0);
        assert!(detector.detect(&[]).is_empty());
    }

    #[test]
    fn min_segment_enforced() {
        let data = vec![1.0, 5.0]; // Only 2 points
        let detector = PeltDetector::new(CostFunction::L2, 1.0).with_min_segment(3);
        assert!(
            detector.detect(&data).is_empty(),
            "2 points with min_segment=3 should find nothing"
        );
    }
}
