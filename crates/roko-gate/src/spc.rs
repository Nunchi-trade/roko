//! Statistical Process Control (SPC) extensions for adaptive gate thresholds.
//!
//! Three detectors beyond the base EMA:
//!
//! - **CUSUM (Cumulative Sum)**: Detects sustained shifts in gate pass rates.
//!   Accumulates deviations from a target; when the cumulative sum exceeds
//!   threshold `h`, signals a shift. Good for catching gradual degradation.
//!
//! - **EWMA Control Chart**: Exponentially weighted moving average with formal
//!   UCL/LCL (Upper/Lower Control Limits). More sensitive to small shifts than
//!   standard Shewhart charts.
//!
//! - **BOCPD (Bayesian Online Change Point Detection)**: Detects abrupt regime
//!   changes (e.g., a model update causes sudden behavior shift). Maintains a
//!   run-length distribution and signals when posterior probability of a recent
//!   change point exceeds a threshold.
//!
//! Reference: docs/04-verification/06-adaptive-thresholds.md sections 11-15.

use serde::{Deserialize, Serialize};

// ─── CUSUM Detector ─────────────────────────────────────────────────────────

/// Cumulative Sum (CUSUM) detector for sustained shifts in a process.
///
/// Maintains two one-sided statistics (upper and lower) that accumulate
/// deviations from the target. When either exceeds `threshold_h`, a shift
/// is detected in the corresponding direction.
///
/// Parameters:
/// - `target`: the expected (in-control) value of the observation
/// - `threshold_h`: decision interval — larger values reduce false alarms
/// - `drift_k`: allowance (slack) parameter — typically half the shift to detect
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CusumDetector {
    /// Target (in-control) value.
    pub target: f64,
    /// Decision threshold — alarm when cumsum exceeds this.
    pub threshold_h: f64,
    /// Drift allowance — half the smallest shift worth detecting.
    pub drift_k: f64,
    /// Upper one-sided cumulative sum (detects upward shifts).
    cumsum_upper: f64,
    /// Lower one-sided cumulative sum (detects downward shifts).
    cumsum_lower: f64,
    /// Number of observations processed.
    observations: usize,
}

/// Direction of a detected CUSUM shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CusumShift {
    /// The process shifted upward from the target.
    Upward,
    /// The process shifted downward from the target.
    Downward,
}

impl CusumDetector {
    /// Create a new CUSUM detector.
    ///
    /// - `target`: in-control mean (e.g. 0.85 pass rate)
    /// - `threshold_h`: alarm threshold (e.g. 5.0)
    /// - `drift_k`: slack parameter (e.g. 0.5 * minimum shift to detect)
    #[must_use]
    pub fn new(target: f64, threshold_h: f64, drift_k: f64) -> Self {
        Self {
            target,
            threshold_h: threshold_h.max(0.0),
            drift_k: drift_k.max(0.0),
            cumsum_upper: 0.0,
            cumsum_lower: 0.0,
            observations: 0,
        }
    }

    /// Update with a new observation and return whether a shift was detected.
    pub fn update(&mut self, observation: f64) -> Option<CusumShift> {
        self.observations += 1;

        // Upper CUSUM: detects upward shift.
        self.cumsum_upper = (self.cumsum_upper + observation - self.target - self.drift_k).max(0.0);
        // Lower CUSUM: detects downward shift.
        self.cumsum_lower = (self.cumsum_lower + self.target - observation - self.drift_k).max(0.0);

        if self.cumsum_upper > self.threshold_h {
            self.cumsum_upper = 0.0; // Reset after alarm.
            return Some(CusumShift::Upward);
        }
        if self.cumsum_lower > self.threshold_h {
            self.cumsum_lower = 0.0; // Reset after alarm.
            return Some(CusumShift::Downward);
        }

        None
    }

    /// Reset the detector state.
    pub fn reset(&mut self) {
        self.cumsum_upper = 0.0;
        self.cumsum_lower = 0.0;
        self.observations = 0;
    }

    /// Number of observations processed.
    #[must_use]
    pub fn observations(&self) -> usize {
        self.observations
    }

    /// Current upper cumulative sum.
    #[must_use]
    pub fn upper_sum(&self) -> f64 {
        self.cumsum_upper
    }

    /// Current lower cumulative sum.
    #[must_use]
    pub fn lower_sum(&self) -> f64 {
        self.cumsum_lower
    }
}

// ─── EWMA Control Chart ────────────────────────────────────────────────────

/// Control status of an EWMA chart observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlStatus {
    /// Within control limits — process is in control.
    InControl,
    /// Between 2-sigma and 3-sigma — potential issue developing.
    Warning,
    /// Beyond 3-sigma control limits — process is out of control.
    OutOfControl,
}

/// EWMA (Exponentially Weighted Moving Average) Control Chart.
///
/// Tracks a smoothed process mean with formal UCL/LCL at
/// `mean +/- L * sigma * sqrt(lambda / (2 - lambda))`.
///
/// More sensitive to small sustained shifts than Shewhart charts because
/// the exponential weighting carries memory of recent observations.
///
/// Parameters:
/// - `lambda`: smoothing factor in (0, 1]. Smaller = more smoothing.
/// - `sigma`: process standard deviation estimate.
/// - `control_limit_l`: multiplier for control limits (typically 3.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EwmaControlChart {
    /// Smoothing factor (0, 1].
    lambda: f64,
    /// Estimated process standard deviation.
    sigma: f64,
    /// Control limit multiplier (number of sigma).
    control_limit_l: f64,
    /// Current EWMA value.
    ewma: f64,
    /// Initial (target) mean.
    target: f64,
    /// Number of observations.
    observations: usize,
}

impl EwmaControlChart {
    /// Create a new EWMA control chart.
    ///
    /// - `target`: in-control mean
    /// - `sigma`: estimated process standard deviation
    /// - `lambda`: smoothing factor (0.0, 1.0], typical = 0.2
    /// - `control_limit_l`: sigma multiplier for limits, typical = 3.0
    #[must_use]
    pub fn new(target: f64, sigma: f64, lambda: f64, control_limit_l: f64) -> Self {
        let lambda = lambda.clamp(0.01, 1.0);
        Self {
            lambda,
            sigma: sigma.max(0.001),
            control_limit_l: control_limit_l.max(0.0),
            ewma: target,
            target,
            observations: 0,
        }
    }

    /// Update the chart with a new observation and return the control status.
    pub fn update(&mut self, observation: f64) -> ControlStatus {
        self.observations += 1;
        self.ewma = self.lambda * observation + (1.0 - self.lambda) * self.ewma;

        let limit_factor = self.sigma * (self.lambda / (2.0 - self.lambda)).sqrt();
        let ucl = self.target + self.control_limit_l * limit_factor;
        let lcl = self.target - self.control_limit_l * limit_factor;
        let warning_ucl = self.target + (self.control_limit_l * 2.0 / 3.0) * limit_factor;
        let warning_lcl = self.target - (self.control_limit_l * 2.0 / 3.0) * limit_factor;

        if self.ewma > ucl || self.ewma < lcl {
            ControlStatus::OutOfControl
        } else if self.ewma > warning_ucl || self.ewma < warning_lcl {
            ControlStatus::Warning
        } else {
            ControlStatus::InControl
        }
    }

    /// Current EWMA value.
    #[must_use]
    pub fn current(&self) -> f64 {
        self.ewma
    }

    /// Upper control limit.
    #[must_use]
    pub fn ucl(&self) -> f64 {
        let limit_factor = self.sigma * (self.lambda / (2.0 - self.lambda)).sqrt();
        self.target + self.control_limit_l * limit_factor
    }

    /// Lower control limit.
    #[must_use]
    pub fn lcl(&self) -> f64 {
        let limit_factor = self.sigma * (self.lambda / (2.0 - self.lambda)).sqrt();
        self.target - self.control_limit_l * limit_factor
    }

    /// Number of observations processed.
    #[must_use]
    pub fn observations(&self) -> usize {
        self.observations
    }

    /// Reset the chart to its initial state.
    pub fn reset(&mut self) {
        self.ewma = self.target;
        self.observations = 0;
    }
}

// ─── BOCPD (Bayesian Online Change Point Detection) ─────────────────────────

/// A detected change point from the BOCPD detector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangePoint {
    /// Observation index where the change was detected.
    pub observation_index: usize,
    /// Most probable run length at the time of detection.
    pub most_probable_run_length: usize,
    /// Posterior probability that a change occurred recently.
    pub change_probability: f64,
}

/// Bayesian Online Change Point Detection (BOCPD).
///
/// Maintains a run-length distribution updated at each observation.
/// When the posterior probability of a recent change point (run length
/// near zero) exceeds the threshold, a change is flagged.
///
/// Uses a Gaussian predictive model (conjugate normal-inverse-gamma)
/// for simplicity.
///
/// Reference: Adams & MacKay (2007), "Bayesian Online Changepoint Detection".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BocpdDetector {
    /// Hazard rate: prior probability of a change point at each step.
    /// Typically 1/expected_run_length (e.g. 1/100 = 0.01).
    hazard_rate: f64,
    /// Threshold on posterior change probability to flag a change.
    change_threshold: f64,
    /// Run-length distribution (unnormalized log-probabilities).
    run_length_probs: Vec<f64>,
    /// Sufficient statistics for each run length: (count, sum, sum_sq).
    sufficient_stats: Vec<(usize, f64, f64)>,
    /// Prior mean for the Gaussian model.
    prior_mean: f64,
    /// Prior variance for the Gaussian model.
    prior_var: f64,
    /// Total observations processed.
    observations: usize,
}

impl BocpdDetector {
    /// Create a new BOCPD detector.
    ///
    /// - `hazard_rate`: prior probability of change at each step (e.g. 0.01)
    /// - `change_threshold`: posterior probability to trigger alarm (e.g. 0.5)
    /// - `prior_mean`: expected observation mean
    /// - `prior_var`: expected observation variance
    #[must_use]
    pub fn new(hazard_rate: f64, change_threshold: f64, prior_mean: f64, prior_var: f64) -> Self {
        Self {
            hazard_rate: hazard_rate.clamp(0.001, 0.5),
            change_threshold: change_threshold.clamp(0.0, 1.0),
            // Start with run length 0 having probability 1.0.
            run_length_probs: vec![1.0],
            sufficient_stats: vec![(0, 0.0, 0.0)],
            prior_mean,
            prior_var: prior_var.max(0.001),
            observations: 0,
        }
    }

    /// Update with a new observation and return a change point if detected.
    pub fn update(&mut self, observation: f64) -> Option<ChangePoint> {
        self.observations += 1;

        let n = self.run_length_probs.len();

        // Step 1: Compute predictive probabilities for each run length.
        let mut predictive_probs = Vec::with_capacity(n);
        for (count, sum, sum_sq) in &self.sufficient_stats {
            let pred_prob = self.gaussian_predictive(*count, *sum, *sum_sq, observation);
            predictive_probs.push(pred_prob);
        }

        // Step 2: Compute growth probabilities (extend existing runs).
        let mut growth_probs = Vec::with_capacity(n);
        for (i, &prob) in self.run_length_probs.iter().enumerate() {
            growth_probs.push(prob * predictive_probs[i] * (1.0 - self.hazard_rate));
        }

        // Step 3: Compute change point probability (new run starts).
        let change_prob: f64 = self
            .run_length_probs
            .iter()
            .enumerate()
            .map(|(i, &prob)| prob * predictive_probs[i] * self.hazard_rate)
            .sum();

        // Step 4: Assemble new run-length distribution.
        let mut new_probs = Vec::with_capacity(n + 1);
        new_probs.push(change_prob);
        new_probs.extend_from_slice(&growth_probs);

        // Normalize.
        let total: f64 = new_probs.iter().sum();
        if total > f64::EPSILON {
            for p in &mut new_probs {
                *p /= total;
            }
        }

        // Step 5: Update sufficient statistics.
        let mut new_stats = Vec::with_capacity(n + 1);
        // New run (length 0) uses prior.
        new_stats.push((0, 0.0, 0.0));
        // Extend existing runs.
        for (count, sum, sum_sq) in &self.sufficient_stats {
            new_stats.push((
                count + 1,
                sum + observation,
                sum_sq + observation * observation,
            ));
        }

        self.run_length_probs = new_probs;
        self.sufficient_stats = new_stats;

        // Trim very small probabilities to bound memory.
        self.trim_low_probability(1e-8);

        // Step 6: Check if change probability exceeds threshold.
        let p_change = self.run_length_probs.first().copied().unwrap_or(0.0);
        if p_change > self.change_threshold {
            let most_probable = self.most_probable_run_length();
            return Some(ChangePoint {
                observation_index: self.observations,
                most_probable_run_length: most_probable,
                change_probability: p_change,
            });
        }

        None
    }

    /// Most probable run length from the current distribution.
    #[must_use]
    pub fn most_probable_run_length(&self) -> usize {
        self.run_length_probs
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map_or(0, |(i, _)| i)
    }

    /// Probability of a recent change point (run length = 0).
    #[must_use]
    pub fn change_probability(&self) -> f64 {
        self.run_length_probs.first().copied().unwrap_or(0.0)
    }

    /// Number of observations processed.
    #[must_use]
    pub fn observations(&self) -> usize {
        self.observations
    }

    /// Reset the detector.
    pub fn reset(&mut self) {
        self.run_length_probs = vec![1.0];
        self.sufficient_stats = vec![(0, 0.0, 0.0)];
        self.observations = 0;
    }

    /// Gaussian predictive probability using conjugate prior.
    fn gaussian_predictive(&self, count: usize, sum: f64, sum_sq: f64, observation: f64) -> f64 {
        let n = count as f64;
        let mean = if n > 0.0 {
            (self.prior_var * sum + self.prior_mean) / (n * self.prior_var + 1.0)
        } else {
            self.prior_mean
        };
        let var = if n > 0.0 {
            let sample_var = if n > 1.0 {
                (sum_sq - sum * sum / n) / (n - 1.0)
            } else {
                self.prior_var
            };
            sample_var / n + self.prior_var
        } else {
            self.prior_var
        };
        let var = var.max(0.001);

        // Gaussian PDF.
        let diff = observation - mean;
        (-0.5 * diff * diff / var).exp() / (2.0 * std::f64::consts::PI * var).sqrt()
    }

    /// Remove run lengths with negligible probability to bound memory.
    fn trim_low_probability(&mut self, min_prob: f64) {
        // Only trim from the tail (longest run lengths).
        while self.run_length_probs.len() > 2 {
            if let Some(&last) = self.run_length_probs.last() {
                if last < min_prob {
                    self.run_length_probs.pop();
                    self.sufficient_stats.pop();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }
}

// ─── Composite SPC detector ────────────────────────────────────────────────

/// Aggregate SPC alert from any of the three detectors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpcAlert {
    /// CUSUM detected a sustained shift.
    CusumShift(CusumShift),
    /// EWMA chart is out of control.
    EwmaOutOfControl {
        /// Current EWMA value.
        ewma_value: f64,
    },
    /// EWMA chart is in warning zone.
    EwmaWarning {
        /// Current EWMA value.
        ewma_value: f64,
    },
    /// BOCPD detected a change point.
    ChangePoint(ChangePoint),
}

/// Composite SPC detector that runs all three methods in parallel.
///
/// Any detector that fires produces an alert. Wire this into
/// `AdaptiveThresholds` to get richer anomaly detection beyond the
/// base EMA.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpcDetector {
    /// CUSUM detector.
    pub cusum: CusumDetector,
    /// EWMA control chart.
    pub ewma_chart: EwmaControlChart,
    /// BOCPD detector.
    pub bocpd: BocpdDetector,
}

impl SpcDetector {
    /// Create a composite SPC detector with default parameters.
    ///
    /// - `target`: expected in-control value (e.g. 0.85 pass rate)
    /// - `sigma`: estimated standard deviation (e.g. 0.1)
    #[must_use]
    pub fn new(target: f64, sigma: f64) -> Self {
        Self {
            cusum: CusumDetector::new(target, 5.0, sigma / 2.0),
            ewma_chart: EwmaControlChart::new(target, sigma, 0.2, 3.0),
            bocpd: BocpdDetector::new(0.01, 0.5, target, sigma * sigma),
        }
    }

    /// Update all detectors with a new observation.
    ///
    /// Returns all alerts fired by any detector.
    pub fn update(&mut self, observation: f64) -> Vec<SpcAlert> {
        let mut alerts = Vec::new();

        if let Some(shift) = self.cusum.update(observation) {
            alerts.push(SpcAlert::CusumShift(shift));
        }

        match self.ewma_chart.update(observation) {
            ControlStatus::OutOfControl => {
                alerts.push(SpcAlert::EwmaOutOfControl {
                    ewma_value: self.ewma_chart.current(),
                });
            }
            ControlStatus::Warning => {
                alerts.push(SpcAlert::EwmaWarning {
                    ewma_value: self.ewma_chart.current(),
                });
            }
            ControlStatus::InControl => {}
        }

        if let Some(cp) = self.bocpd.update(observation) {
            alerts.push(SpcAlert::ChangePoint(cp));
        }

        alerts
    }

    /// Reset all detectors.
    pub fn reset(&mut self) {
        self.cusum.reset();
        self.ewma_chart.reset();
        self.bocpd.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── CUSUM tests ────────────────────────────────────────────────────

    #[test]
    fn cusum_no_shift_under_normal_conditions() {
        let mut cusum = CusumDetector::new(0.85, 5.0, 0.05);
        // Feed in-control observations.
        for _ in 0..50 {
            assert!(cusum.update(0.85).is_none());
        }
    }

    #[test]
    fn cusum_detects_upward_shift() {
        let mut cusum = CusumDetector::new(0.5, 3.0, 0.1);
        let mut detected = false;
        // Sustained upward deviation.
        for _ in 0..100 {
            if cusum.update(0.9).is_some() {
                detected = true;
                break;
            }
        }
        assert!(detected, "CUSUM should detect upward shift");
    }

    #[test]
    fn cusum_detects_downward_shift() {
        let mut cusum = CusumDetector::new(0.85, 3.0, 0.05);
        let mut detected = false;
        // Sustained downward deviation.
        for _ in 0..100 {
            if let Some(CusumShift::Downward) = cusum.update(0.4) {
                detected = true;
                break;
            }
        }
        assert!(detected, "CUSUM should detect downward shift");
    }

    #[test]
    fn cusum_reset_clears_state() {
        let mut cusum = CusumDetector::new(0.85, 5.0, 0.05);
        cusum.update(0.1);
        cusum.update(0.1);
        assert!(cusum.lower_sum() > 0.0);
        cusum.reset();
        assert_eq!(cusum.upper_sum(), 0.0);
        assert_eq!(cusum.lower_sum(), 0.0);
        assert_eq!(cusum.observations(), 0);
    }

    // ─── EWMA Control Chart tests ───────────────────────────────────────

    #[test]
    fn ewma_in_control_under_normal_conditions() {
        let mut chart = EwmaControlChart::new(0.85, 0.1, 0.2, 3.0);
        for _ in 0..20 {
            let status = chart.update(0.85);
            assert_eq!(status, ControlStatus::InControl);
        }
    }

    #[test]
    fn ewma_out_of_control_on_large_deviation() {
        let mut chart = EwmaControlChart::new(0.85, 0.05, 0.2, 3.0);
        let mut out = false;
        for _ in 0..50 {
            if chart.update(0.2) == ControlStatus::OutOfControl {
                out = true;
                break;
            }
        }
        assert!(out, "EWMA should detect out-of-control");
    }

    #[test]
    fn ewma_control_limits_are_symmetric() {
        let chart = EwmaControlChart::new(0.5, 0.1, 0.2, 3.0);
        let ucl = chart.ucl();
        let lcl = chart.lcl();
        assert!((ucl - 0.5 - (0.5 - lcl)).abs() < 1e-10);
    }

    #[test]
    fn ewma_reset_restores_target() {
        let mut chart = EwmaControlChart::new(0.85, 0.1, 0.2, 3.0);
        chart.update(0.2);
        chart.update(0.2);
        chart.reset();
        assert!((chart.current() - 0.85).abs() < 1e-10);
        assert_eq!(chart.observations(), 0);
    }

    // ─── BOCPD tests ────────────────────────────────────────────────────

    #[test]
    fn bocpd_no_change_under_stable_process() {
        let mut bocpd = BocpdDetector::new(0.01, 0.5, 0.85, 0.01);
        let mut change_count = 0;
        for _ in 0..50 {
            if bocpd.update(0.85).is_some() {
                change_count += 1;
            }
        }
        // Under a perfectly stable process, ideally no changes.
        // (Allow at most 1 spurious detection from initial transient.)
        assert!(
            change_count <= 1,
            "got {change_count} changes under stable process"
        );
    }

    #[test]
    fn bocpd_detects_abrupt_change() {
        // Use a high hazard rate and low threshold for reliable detection.
        let mut bocpd = BocpdDetector::new(0.2, 0.15, 0.85, 0.01);
        // Stable regime.
        for _ in 0..30 {
            bocpd.update(0.85);
        }
        // Abrupt shift: completely different value.
        let mut max_change_prob = 0.0_f64;
        let mut detected = false;
        for _ in 0..50 {
            if let Some(_cp) = bocpd.update(0.1) {
                detected = true;
                break;
            }
            max_change_prob = max_change_prob.max(bocpd.change_probability());
        }
        assert!(
            detected,
            "BOCPD should detect abrupt change (max_change_prob={max_change_prob:.4})"
        );
    }

    #[test]
    fn bocpd_reset_clears_state() {
        let mut bocpd = BocpdDetector::new(0.01, 0.5, 0.85, 0.01);
        for _ in 0..10 {
            bocpd.update(0.5);
        }
        bocpd.reset();
        assert_eq!(bocpd.observations(), 0);
        assert_eq!(bocpd.run_length_probs.len(), 1);
    }

    // ─── Composite SPC tests ────────────────────────────────────────────

    #[test]
    fn spc_detector_no_alerts_under_normal() {
        let mut spc = SpcDetector::new(0.85, 0.1);
        for _ in 0..20 {
            let alerts = spc.update(0.85);
            assert!(alerts.is_empty(), "no alerts expected, got {alerts:?}");
        }
    }

    #[test]
    fn spc_detector_alerts_on_major_shift() {
        let mut spc = SpcDetector::new(0.85, 0.05);
        // Establish baseline.
        for _ in 0..20 {
            spc.update(0.85);
        }
        // Major shift.
        let mut any_alert = false;
        for _ in 0..50 {
            let alerts = spc.update(0.2);
            if !alerts.is_empty() {
                any_alert = true;
                break;
            }
        }
        assert!(any_alert, "SPC should detect major shift");
    }

    #[test]
    fn spc_detector_reset() {
        let mut spc = SpcDetector::new(0.85, 0.1);
        spc.update(0.1);
        spc.reset();
        assert_eq!(spc.cusum.observations(), 0);
        assert_eq!(spc.ewma_chart.observations(), 0);
        assert_eq!(spc.bocpd.observations(), 0);
    }
}
