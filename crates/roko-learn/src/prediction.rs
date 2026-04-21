//! Prediction records for routing calibration and residual tracking.

use chrono::Utc;
use roko_core::{PredictionCalibrationSource, PredictionCalibrationSummary};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::routing_log::{RoutingDecisionLog, RoutingDecisionLogStore};

/// A single bin in a reliability diagram.
///
/// For calibration visualization: x-axis = absolute residual magnitude,
/// y-axis = mean residual. Bins near zero with near-zero mean residual
/// indicate good calibration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReliabilityBin {
    /// Center of the residual magnitude bin (e.g. 0.05, 0.15, ...).
    pub bin_center: f64,
    /// Mean residual in this bin (signed, shows direction of miscalibration).
    pub mean_residual: f64,
    /// Number of observations in this bin.
    pub count: usize,
}

/// Prediction captured before task execution and resolved after completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionRecord {
    /// Unique identifier for the routed task.
    pub task_id: String,
    /// Canonical model slug selected for the task.
    pub model_slug: String,
    /// Broad task category used by the router.
    pub task_category: String,
    /// Complexity label recorded alongside the prediction.
    pub complexity: String,

    /// Predicted probability that the task will succeed.
    pub predicted_success_prob: f64,
    /// Predicted task cost in USD.
    pub predicted_cost_usd: f64,
    /// Predicted task duration in milliseconds.
    pub predicted_duration_ms: u64,

    /// Actual task outcome once the task has completed.
    pub actual_success: Option<bool>,
    /// Actual task cost in USD once known.
    pub actual_cost_usd: Option<f64>,
    /// Actual task duration in milliseconds once known.
    pub actual_duration_ms: Option<u64>,

    /// Success residual: predicted success minus actual success (0.0 or 1.0).
    pub residual_success: Option<f64>,
    /// Cost residual: predicted cost minus actual cost.
    pub residual_cost: Option<f64>,
    /// Duration residual: predicted duration minus actual duration.
    pub residual_duration: Option<f64>,

    /// RFC 3339 timestamp for when the prediction was registered.
    pub timestamp: String,
}

impl PredictionRecord {
    /// Register a prediction before task execution starts.
    #[must_use]
    pub fn register(
        task_id: impl Into<String>,
        model_slug: impl Into<String>,
        task_category: impl Into<String>,
        complexity: impl Into<String>,
        predicted_success_prob: f64,
        predicted_cost_usd: f64,
        predicted_duration_ms: u64,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            model_slug: model_slug.into(),
            task_category: task_category.into(),
            complexity: complexity.into(),
            predicted_success_prob: predicted_success_prob.clamp(0.0, 1.0),
            predicted_cost_usd: predicted_cost_usd.max(0.0),
            predicted_duration_ms,
            actual_success: None,
            actual_cost_usd: None,
            actual_duration_ms: None,
            residual_success: None,
            residual_cost: None,
            residual_duration: None,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    /// Resolve a prediction after task completion and compute residuals.
    pub fn resolve(&mut self, actual_success: bool, actual_cost_usd: f64, actual_duration_ms: u64) {
        self.actual_success = Some(actual_success);
        self.actual_cost_usd = Some(actual_cost_usd);
        self.actual_duration_ms = Some(actual_duration_ms);
        self.residual_success =
            Some(self.predicted_success_prob - if actual_success { 1.0 } else { 0.0 });
        self.residual_cost = Some(self.predicted_cost_usd - actual_cost_usd);
        self.residual_duration =
            Some(self.predicted_duration_ms as f64 - actual_duration_ms as f64);
    }

    /// Convert a completed routing log record into a calibrated prediction record.
    ///
    /// The selected candidate score is normalized against the candidate set via
    /// softmax so the tracker always sees a probability-like quantity even when
    /// the underlying routing stage uses raw UCB or confidence scores.
    #[must_use]
    pub fn from_routing_log(record: &RoutingDecisionLog) -> Option<Self> {
        let actual_success = record.outcome_success?;
        let actual_cost_usd = record.outcome_cost_usd.unwrap_or(0.0);
        let actual_duration_ms = record.outcome_latency_ms.unwrap_or(0);
        let predicted_success_prob =
            selected_probability(record).unwrap_or_else(|| fallback_stage_probability(record));

        let mut prediction = Self::register(
            record.task_id.clone(),
            record.selected_model.clone(),
            if record.task_category.is_empty() {
                "unknown".to_string()
            } else {
                record.task_category.clone()
            },
            record.task_complexity.clone(),
            predicted_success_prob,
            actual_cost_usd,
            actual_duration_ms,
        );
        prediction.resolve(actual_success, actual_cost_usd, actual_duration_ms);
        prediction.timestamp = record.timestamp.clone();
        Some(prediction)
    }
}

/// Aggregated calibration data for model/category pairs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalibrationTracker {
    /// Success residuals keyed by `(model_slug, task_category)`.
    residuals: HashMap<(String, String), Vec<f64>>,
}

impl CalibrationTracker {
    /// Whether no calibration observations are recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.residuals.values().all(Vec::is_empty)
    }

    /// Record a success residual for a model/category pair.
    pub fn record_residual(
        &mut self,
        model: impl Into<String>,
        category: impl Into<String>,
        residual: f64,
    ) {
        self.residuals
            .entry((model.into(), category.into()))
            .or_default()
            .push(residual.clamp(-1.0, 1.0));
    }

    /// Ingest the resolved success residual from a prediction record.
    pub fn record_prediction(&mut self, prediction: &PredictionRecord) {
        if let Some(residual) = prediction.residual_success {
            self.record_residual(
                prediction.model_slug.clone(),
                prediction.task_category.clone(),
                residual,
            );
        }
    }

    /// Ingest a completed routing decision directly.
    pub fn record_routing_decision(&mut self, record: &RoutingDecisionLog) {
        if let Some(prediction) = PredictionRecord::from_routing_log(record) {
            self.record_prediction(&prediction);
        }
    }

    /// Load calibration state by replaying a routing decision log.
    ///
    /// # Errors
    ///
    /// Returns an error only when the routing log itself cannot be read.
    pub async fn load_from_routing_log(path: impl AsRef<Path>) -> Result<Self, io::Error> {
        let store = RoutingDecisionLogStore::at(path.as_ref()).without_fsync();
        let records = store.read_all().await?;
        Ok(Self::from_routing_logs(&records))
    }

    /// Build calibration state by replaying routing decision records.
    #[must_use]
    pub fn from_routing_logs(records: &[RoutingDecisionLog]) -> Self {
        let mut tracker = Self::default();
        for record in records {
            tracker.record_routing_decision(record);
        }
        tracker
    }

    /// Average residual for a model/category pair.
    #[must_use]
    pub fn mean_bias(&self, model: &str, category: &str) -> f64 {
        let Some(residuals) = self
            .residuals
            .get(&(model.to_string(), category.to_string()))
        else {
            return 0.0;
        };

        mean(residuals)
    }

    /// Number of observations for a model/category pair.
    #[must_use]
    pub fn sample_count(&self, model: &str, category: &str) -> usize {
        self.residuals
            .get(&(model.to_string(), category.to_string()))
            .map_or(0, Vec::len)
    }

    /// Fraction of residuals that fall within the requested error band.
    #[must_use]
    pub fn coverage_rate(&self, model: &str, category: &str, confidence: f64) -> f64 {
        let Some(residuals) = self
            .residuals
            .get(&(model.to_string(), category.to_string()))
        else {
            return 0.0;
        };

        if residuals.is_empty() {
            return 0.0;
        }

        let band = confidence.abs();
        let covered = residuals
            .iter()
            .filter(|residual| residual.abs() <= band)
            .count();
        covered as f64 / residuals.len() as f64
    }

    /// Approximate recent accuracy from the absolute residual.
    #[must_use]
    pub fn recent_accuracy(&self, model: &str, category: &str) -> f64 {
        let Some(residuals) = self
            .residuals
            .get(&(model.to_string(), category.to_string()))
        else {
            return 0.5;
        };
        if residuals.is_empty() {
            return 0.5;
        }

        let window = tail_window(residuals);
        let accuracy_sum = window
            .iter()
            .map(|residual| 1.0 - residual.abs().min(1.0))
            .sum::<f64>();
        accuracy_sum / window.len() as f64
    }

    /// Short-horizon change in accuracy. Negative means degradation.
    #[must_use]
    pub fn accuracy_trend(&self, model: &str, category: &str) -> f64 {
        let Some(residuals) = self
            .residuals
            .get(&(model.to_string(), category.to_string()))
        else {
            return 0.0;
        };
        if residuals.len() < 4 {
            return 0.0;
        }

        let split = residuals.len() / 2;
        let early = &residuals[..split];
        let late = &residuals[split..];
        mean_accuracy(late) - mean_accuracy(early)
    }

    /// Apply the learned bias correction to a raw success prediction.
    #[must_use]
    pub fn adjust_prediction(&self, model: &str, category: &str, raw_pred: f64) -> f64 {
        (raw_pred - self.mean_bias(model, category)).clamp(0.0, 1.0)
    }

    /// Brier score: mean squared error of probabilistic predictions.
    ///
    /// Lower is better. Perfect calibration = 0.0, random = 0.25.
    /// Residuals are (predicted - actual), so Brier = mean(residual^2).
    #[must_use]
    pub fn brier_score(&self, model: &str, category: &str) -> Option<f64> {
        let residuals = self
            .residuals
            .get(&(model.to_string(), category.to_string()))?;
        if residuals.is_empty() {
            return None;
        }
        Some(residuals.iter().map(|r| r * r).sum::<f64>() / residuals.len() as f64)
    }

    /// Bin residuals into 10 equally-spaced buckets for reliability diagrams.
    ///
    /// Each bucket covers a 0.2-wide band of absolute residual magnitude
    /// (since residuals are already in [-1, 1]). Returns tuples of
    /// `(bin_center, mean_residual, count)` for non-empty bins.
    ///
    /// For a reliability diagram: x-axis = predicted, y-axis = observed.
    /// Diagonal means perfectly calibrated.
    #[must_use]
    pub fn reliability_bins(&self, model: &str, category: &str) -> Vec<ReliabilityBin> {
        let Some(residuals) = self
            .residuals
            .get(&(model.to_string(), category.to_string()))
        else {
            return Vec::new();
        };
        if residuals.is_empty() {
            return Vec::new();
        }

        // We work with absolute residuals binned into 10 buckets of width 0.1
        // across [0.0, 1.0].
        let num_bins = 10;
        let bin_width = 1.0 / num_bins as f64;
        let mut sums = vec![0.0_f64; num_bins];
        let mut counts = vec![0_usize; num_bins];

        for &r in residuals {
            let abs_r = r.abs().min(0.9999);
            let idx = (abs_r / bin_width) as usize;
            let idx = idx.min(num_bins - 1);
            sums[idx] += r;
            counts[idx] += 1;
        }

        let mut bins = Vec::new();
        for i in 0..num_bins {
            if counts[i] > 0 {
                bins.push(ReliabilityBin {
                    bin_center: (i as f64 + 0.5) * bin_width,
                    mean_residual: sums[i] / counts[i] as f64,
                    count: counts[i],
                });
            }
        }
        bins
    }

    /// Arithmetic corrector: returns the mean bias for a model/category pair.
    ///
    /// Subtract this value from raw predictions to correct systematic
    /// over- or under-confidence. ~50ns per correction (pure arithmetic).
    #[must_use]
    pub fn arithmetic_corrector(&self, model: &str, category: &str) -> f64 {
        self.mean_bias(model, category)
    }

    /// Build a calibration summary for the predictive scorer/policy layer.
    #[must_use]
    pub fn summary(&self, model: &str, category: &str) -> PredictionCalibrationSummary {
        let sample_count = self.sample_count(model, category);
        if sample_count == 0 {
            return PredictionCalibrationSummary::cold_start();
        }

        let recent_accuracy = self.recent_accuracy(model, category);
        PredictionCalibrationSummary {
            recent_accuracy,
            coverage: self.coverage_rate(model, category, 1.0 - recent_accuracy),
            mean_bias: self.mean_bias(model, category),
            accuracy_trend: self.accuracy_trend(model, category),
            sample_count,
            confidence: (sample_count as f64 / 200.0).min(1.0),
        }
    }
}

impl PredictionCalibrationSource for CalibrationTracker {
    fn summary(&self, model: &str, task_category: &str) -> PredictionCalibrationSummary {
        self.summary(model, task_category)
    }
}

// ─── Residual Corrector (P0-33) ─────────────────────────────────────

/// Circular buffer with O(1) streaming statistics for residual tracking.
///
/// Maintains the last `capacity` residuals with running sum and sum-of-squares
/// for instant mean, variance, and coverage computation.
#[derive(Debug, Clone)]
pub struct ResidualBuffer {
    /// Ring buffer of residual values.
    values: Vec<f64>,
    /// Next write position (wraps around).
    write_idx: usize,
    /// Number of values inserted (may exceed capacity; used for stats).
    count: usize,
    /// Running sum for O(1) mean.
    sum: f64,
    /// Running sum of squares for O(1) variance.
    sum_sq: f64,
    /// Buffer capacity (default 200).
    capacity: usize,
}

impl ResidualBuffer {
    /// Create a new buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            write_idx: 0,
            count: 0,
            sum: 0.0,
            sum_sq: 0.0,
            capacity: capacity.max(1),
        }
    }

    /// Push a new residual value. O(1) amortized.
    pub fn push(&mut self, value: f64) {
        if self.values.len() < self.capacity {
            // Buffer not yet full — just append.
            self.values.push(value);
        } else {
            // Buffer full — overwrite oldest and adjust running sums.
            let old = self.values[self.write_idx];
            self.sum -= old;
            self.sum_sq -= old * old;
            self.values[self.write_idx] = value;
        }
        self.sum += value;
        self.sum_sq += value * value;
        self.write_idx = (self.write_idx + 1) % self.capacity;
        self.count += 1;
    }

    /// Number of values currently in the buffer (up to capacity).
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Mean residual. O(1).
    pub fn mean(&self) -> f64 {
        if self.values.is_empty() {
            0.0
        } else {
            self.sum / self.values.len() as f64
        }
    }

    /// Variance of residuals. O(1).
    pub fn variance(&self) -> f64 {
        let n = self.values.len();
        if n < 2 {
            return 0.0;
        }
        let mean = self.mean();
        (self.sum_sq / n as f64 - mean * mean).max(0.0)
    }

    /// Standard deviation of residuals.
    pub fn std_dev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Fraction of residuals within the given half-width of zero.
    ///
    /// Coverage = count(|residual| <= half_width) / total.
    pub fn coverage_rate(&self, half_width: f64) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        let within = self.values.iter().filter(|r| r.abs() <= half_width).count();
        within as f64 / self.values.len() as f64
    }

    /// Total number of values ever inserted (including overwritten).
    pub fn total_count(&self) -> usize {
        self.count
    }
}

impl Default for ResidualBuffer {
    fn default() -> Self {
        Self::new(200)
    }
}

/// A corrected prediction with adjusted center and interval width.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectedPrediction {
    /// Bias-corrected center value.
    pub center: f64,
    /// Calibrated interval half-width.
    pub half_width: f64,
    /// Original uncorrected center.
    pub original_center: f64,
    /// Original uncorrected half-width.
    pub original_half_width: f64,
    /// Difficulty weight applied to this prediction.
    pub difficulty_weight: f64,
}

/// Residual corrector for predictive foraging.
///
/// Maintains per-key circular buffers and applies two corrections:
/// 1. **Bias correction**: `center -= mean_residual` (removes systematic over/under-prediction)
/// 2. **Interval calibration**: adjusts width toward target coverage (default 85%)
///
/// Keys are `(category, context, metric)` triples, allowing different correction
/// parameters for different types of predictions.
///
/// Per spec (agent-chain-new/08-predictive-foraging.md lines 186-267).
#[derive(Debug, Clone)]
pub struct ResidualCorrector {
    /// Per-key residual buffers.
    buffers: HashMap<(String, String, String), ResidualBuffer>,
    /// Target coverage rate for interval calibration.
    pub target_coverage: f64,
    /// Buffer capacity per key.
    pub buffer_capacity: usize,
    /// Width adjustment rate (how fast intervals expand/contract).
    pub width_adjust_rate: f64,
    /// Coverage tolerance band (hysteresis to prevent oscillation).
    pub coverage_tolerance: f64,
}

impl ResidualCorrector {
    /// Create a new corrector with default settings (85% target coverage, 200-element buffers).
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            target_coverage: 0.85,
            buffer_capacity: 200,
            width_adjust_rate: 0.05,
            coverage_tolerance: 0.05,
        }
    }

    /// Create with custom target coverage.
    pub fn with_target_coverage(mut self, target: f64) -> Self {
        self.target_coverage = target.clamp(0.5, 0.99);
        self
    }

    /// Create with custom buffer capacity.
    pub fn with_buffer_capacity(mut self, capacity: usize) -> Self {
        self.buffer_capacity = capacity.max(10);
        self
    }

    /// Record a residual observation for the given key.
    ///
    /// `residual = predicted - actual` (positive = overprediction).
    pub fn record(&mut self, category: &str, context: &str, metric: &str, residual: f64) {
        let key = (
            category.to_string(),
            context.to_string(),
            metric.to_string(),
        );
        let buffer = self
            .buffers
            .entry(key)
            .or_insert_with(|| ResidualBuffer::new(self.buffer_capacity));
        buffer.push(residual);
    }

    /// Get the buffer for a specific key, if it exists.
    pub fn buffer(&self, category: &str, context: &str, metric: &str) -> Option<&ResidualBuffer> {
        let key = (
            category.to_string(),
            context.to_string(),
            metric.to_string(),
        );
        self.buffers.get(&key)
    }

    /// Apply bias correction and interval calibration to a prediction.
    ///
    /// Returns a `CorrectedPrediction` with adjusted center and width.
    pub fn correct(
        &self,
        category: &str,
        context: &str,
        metric: &str,
        center: f64,
        half_width: f64,
    ) -> CorrectedPrediction {
        let key = (
            category.to_string(),
            context.to_string(),
            metric.to_string(),
        );

        let Some(buffer) = self.buffers.get(&key) else {
            // No history — return uncorrected.
            return CorrectedPrediction {
                center,
                half_width,
                original_center: center,
                original_half_width: half_width,
                difficulty_weight: 1.0,
            };
        };

        if buffer.len() < 5 {
            // Not enough data for reliable correction.
            return CorrectedPrediction {
                center,
                half_width,
                original_center: center,
                original_half_width: half_width,
                difficulty_weight: 1.0,
            };
        }

        // Step 1: Bias correction — subtract mean residual from center.
        let bias = buffer.mean();
        let corrected_center = center - bias;

        // Step 2: Interval width calibration toward target coverage.
        let coverage = buffer.coverage_rate(half_width);
        let corrected_width = if coverage < self.target_coverage - self.coverage_tolerance {
            // Under-covering: widen interval.
            half_width * (1.0 + self.width_adjust_rate)
        } else if coverage > self.target_coverage + self.coverage_tolerance * 2.0 {
            // Over-covering: narrow interval (slower than widening to be conservative).
            half_width * (1.0 - self.width_adjust_rate)
        } else {
            half_width
        };

        // Step 3: Difficulty weighting.
        // Higher variance = harder to predict = higher weight for this observation.
        let category_variance = buffer.variance();
        let novelty = 1.0 / (buffer.len() as f64).sqrt(); // Fewer samples = more novel
        let tightness = if corrected_width > 0.0 {
            1.0 / corrected_width
        } else {
            1.0
        };
        let difficulty_weight = (category_variance * novelty * tightness).clamp(0.1, 10.0);

        CorrectedPrediction {
            center: corrected_center,
            half_width: corrected_width.max(0.01),
            original_center: center,
            original_half_width: half_width,
            difficulty_weight,
        }
    }

    /// Number of tracked keys.
    pub fn key_count(&self) -> usize {
        self.buffers.len()
    }

    /// Total observations across all buffers.
    pub fn total_observations(&self) -> usize {
        self.buffers.values().map(|b| b.total_count()).sum()
    }

    /// Mean bias for a specific key (0.0 if not tracked).
    pub fn mean_bias(&self, category: &str, context: &str, metric: &str) -> f64 {
        self.buffer(category, context, metric)
            .map_or(0.0, ResidualBuffer::mean)
    }
}

impl Default for ResidualCorrector {
    fn default() -> Self {
        Self::new()
    }
}

fn tail_window(values: &[f64]) -> &[f64] {
    let window = values.len().clamp(1, 16);
    &values[values.len() - window..]
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn mean_accuracy(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.5;
    }
    values
        .iter()
        .map(|residual| 1.0 - residual.abs().min(1.0))
        .sum::<f64>()
        / values.len() as f64
}

fn selected_probability(record: &RoutingDecisionLog) -> Option<f64> {
    let selected = record
        .candidates
        .iter()
        .find(|candidate| candidate.model == record.selected_model)?;
    let max_score = record
        .candidates
        .iter()
        .map(|candidate| candidate.score)
        .reduce(f64::max)
        .unwrap_or(selected.score);
    let denom = record
        .candidates
        .iter()
        .map(|candidate| (candidate.score - max_score).exp())
        .sum::<f64>();
    if denom <= f64::EPSILON {
        None
    } else {
        Some(((selected.score - max_score).exp() / denom).clamp(0.01, 0.99))
    }
}

fn fallback_stage_probability(record: &RoutingDecisionLog) -> f64 {
    match record.routing_stage.as_str() {
        "static" => 0.65,
        "confidence" => 0.70,
        "ucb" => 0.75,
        _ => 0.60,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CalibrationTracker, PredictionRecord, ResidualBuffer, ResidualCorrector,
        fallback_stage_probability, selected_probability,
    };
    use crate::routing_log::{CandidateEntry, RoutingDecisionLog};

    #[test]
    fn prediction_record_register_starts_unresolved() {
        let record = PredictionRecord::register(
            "task-1",
            "glm-5.1",
            "implementation",
            "complex",
            0.82,
            0.25,
            1_500,
        );

        assert_eq!(record.task_id, "task-1");
        assert_eq!(record.model_slug, "glm-5.1");
        assert_eq!(record.task_category, "implementation");
        assert_eq!(record.complexity, "complex");
        assert_eq!(record.predicted_success_prob, 0.82);
        assert_eq!(record.predicted_cost_usd, 0.25);
        assert_eq!(record.predicted_duration_ms, 1_500);
        assert!(record.actual_success.is_none());
        assert!(record.actual_cost_usd.is_none());
        assert!(record.actual_duration_ms.is_none());
        assert!(record.residual_success.is_none());
        assert!(record.residual_cost.is_none());
        assert!(record.residual_duration.is_none());
        assert!(!record.timestamp.is_empty());
    }

    #[test]
    fn prediction_record_resolve_computes_actuals_and_residuals() {
        let mut record = PredictionRecord::register(
            "task-2",
            "glm-5.1",
            "implementation",
            "complex",
            0.82,
            0.25,
            1_500,
        );

        record.resolve(true, 0.31, 1_700);

        assert_eq!(record.actual_success, Some(true));
        assert_eq!(record.actual_cost_usd, Some(0.31));
        assert_eq!(record.actual_duration_ms, Some(1_700));
        assert!((record.residual_success.expect("residual success") + 0.18).abs() < 1e-12);
        assert!((record.residual_cost.expect("residual cost") + 0.06).abs() < 1e-12);
        assert_eq!(record.residual_duration, Some(-200.0));
    }

    #[test]
    fn calibration_tracker_mean_bias_converges_after_many_observations() {
        let mut tracker = CalibrationTracker::default();

        for _ in 0..50 {
            tracker.record_residual("glm-5.1", "implementation", 0.2);
        }

        assert!((tracker.mean_bias("glm-5.1", "implementation") - 0.2).abs() < 1e-12);
        assert_eq!(tracker.coverage_rate("glm-5.1", "implementation", 0.2), 1.0);
    }

    #[test]
    fn calibration_tracker_adjust_prediction_corrects_systematic_overconfidence() {
        let mut tracker = CalibrationTracker::default();

        for _ in 0..50 {
            tracker.record_residual("glm-5.1", "implementation", 0.15);
        }

        assert!((tracker.adjust_prediction("glm-5.1", "implementation", 0.8) - 0.65).abs() < 1e-12);
    }

    #[test]
    fn calibration_tracker_record_prediction_uses_resolved_success_residual() {
        let mut tracker = CalibrationTracker::default();
        let mut record = PredictionRecord::register(
            "task-3",
            "glm-5.1",
            "verification",
            "standard",
            0.25,
            0.10,
            900,
        );

        tracker.record_prediction(&record);
        assert_eq!(tracker.mean_bias("glm-5.1", "verification"), 0.0);

        record.resolve(false, 0.12, 950);
        tracker.record_prediction(&record);

        assert!((tracker.mean_bias("glm-5.1", "verification") - 0.25).abs() < 1e-12);
        assert_eq!(tracker.coverage_rate("glm-5.1", "verification", 0.2), 0.0);
        assert_eq!(tracker.coverage_rate("glm-5.1", "verification", 0.3), 1.0);
    }

    #[test]
    fn routing_log_probability_uses_candidate_scores() {
        let record = RoutingDecisionLog {
            timestamp: "2026-04-14T10:00:00Z".to_string(),
            trace_id: "trace-1".to_string(),
            task_id: "task-1".to_string(),
            requested_model: "claude-sonnet-4-5".to_string(),
            role: "implementer".to_string(),
            task_complexity: "focused".to_string(),
            task_category: "implementation".to_string(),
            selected_provider: "anthropic".to_string(),
            selected_model: "claude-sonnet-4-5".to_string(),
            routing_stage: "ucb".to_string(),
            routing_reason: "highest_ucb_score".to_string(),
            candidates: vec![
                CandidateEntry {
                    model: "claude-sonnet-4-5".to_string(),
                    provider: "anthropic".to_string(),
                    score: 1.4,
                    disqualified: None,
                },
                CandidateEntry {
                    model: "claude-haiku-4-5".to_string(),
                    provider: "anthropic".to_string(),
                    score: 0.2,
                    disqualified: None,
                },
            ],
            outcome_success: Some(true),
            outcome_cost_usd: Some(0.12),
            outcome_latency_ms: Some(1_000),
        };

        let probability = selected_probability(&record).expect("probability");
        assert!(probability > 0.5);
    }

    #[test]
    fn routing_log_fallback_probability_depends_on_stage() {
        let base = RoutingDecisionLog {
            timestamp: "2026-04-14T10:00:00Z".to_string(),
            trace_id: "trace-1".to_string(),
            task_id: "task-1".to_string(),
            requested_model: "claude-sonnet-4-5".to_string(),
            role: "implementer".to_string(),
            task_complexity: "focused".to_string(),
            task_category: "implementation".to_string(),
            selected_provider: "anthropic".to_string(),
            selected_model: "claude-sonnet-4-5".to_string(),
            routing_stage: "static".to_string(),
            routing_reason: "role_default".to_string(),
            candidates: Vec::new(),
            outcome_success: Some(true),
            outcome_cost_usd: Some(0.12),
            outcome_latency_ms: Some(1_000),
        };

        assert_eq!(fallback_stage_probability(&base), 0.65);
    }

    #[test]
    fn tracker_builds_summary_from_routing_logs() {
        let record = RoutingDecisionLog {
            timestamp: "2026-04-14T10:00:00Z".to_string(),
            trace_id: "trace-1".to_string(),
            task_id: "task-1".to_string(),
            requested_model: "claude-sonnet-4-5".to_string(),
            role: "implementer".to_string(),
            task_complexity: "focused".to_string(),
            task_category: "implementation".to_string(),
            selected_provider: "anthropic".to_string(),
            selected_model: "claude-sonnet-4-5".to_string(),
            routing_stage: "confidence".to_string(),
            routing_reason: "highest_confidence_score".to_string(),
            candidates: vec![CandidateEntry {
                model: "claude-sonnet-4-5".to_string(),
                provider: "anthropic".to_string(),
                score: 0.8,
                disqualified: None,
            }],
            outcome_success: Some(true),
            outcome_cost_usd: Some(0.10),
            outcome_latency_ms: Some(900),
        };

        let tracker = CalibrationTracker::from_routing_logs(&[record]);
        let summary = tracker.summary("claude-sonnet-4-5", "implementation");

        assert_eq!(summary.sample_count, 1);
        assert!(summary.recent_accuracy > 0.0);
    }

    #[test]
    fn brier_score_computes_mean_squared_residual() {
        let mut tracker = CalibrationTracker::default();
        // Record residuals of 0.2 and -0.4 => Brier = (0.04 + 0.16) / 2 = 0.10
        tracker.record_residual("model-a", "impl", 0.2);
        tracker.record_residual("model-a", "impl", -0.4);

        let brier = tracker.brier_score("model-a", "impl").unwrap();
        assert!((brier - 0.10).abs() < 1e-10);
    }

    #[test]
    fn brier_score_returns_none_for_unknown_pair() {
        let tracker = CalibrationTracker::default();
        assert!(tracker.brier_score("unknown", "unknown").is_none());
    }

    #[test]
    fn brier_score_perfect_predictions_are_zero() {
        let mut tracker = CalibrationTracker::default();
        for _ in 0..10 {
            tracker.record_residual("model-a", "impl", 0.0);
        }
        let brier = tracker.brier_score("model-a", "impl").unwrap();
        assert!(brier.abs() < 1e-10);
    }

    #[test]
    fn reliability_bins_returns_non_empty_for_data() {
        let mut tracker = CalibrationTracker::default();
        // Small residuals should land in the first bin.
        for _ in 0..10 {
            tracker.record_residual("model-a", "impl", 0.05);
        }
        // Larger residuals go to later bins.
        for _ in 0..5 {
            tracker.record_residual("model-a", "impl", 0.3);
        }

        let bins = tracker.reliability_bins("model-a", "impl");
        assert!(!bins.is_empty());
        // Should have at least 2 bins populated.
        assert!(bins.len() >= 2);
        // All counts should be > 0.
        assert!(bins.iter().all(|b| b.count > 0));
    }

    #[test]
    fn reliability_bins_empty_for_unknown_pair() {
        let tracker = CalibrationTracker::default();
        assert!(tracker.reliability_bins("unknown", "unknown").is_empty());
    }

    #[test]
    fn arithmetic_corrector_equals_mean_bias() {
        let mut tracker = CalibrationTracker::default();
        for _ in 0..50 {
            tracker.record_residual("model-a", "impl", 0.15);
        }
        assert!((tracker.arithmetic_corrector("model-a", "impl") - 0.15).abs() < 1e-10);
    }

    // ─── ResidualBuffer tests ───────────────────────────────────────

    #[test]
    fn residual_buffer_basic_stats() {
        let mut buf = ResidualBuffer::new(100);
        for i in 0..10 {
            buf.push(i as f64);
        }
        assert_eq!(buf.len(), 10);
        assert!((buf.mean() - 4.5).abs() < 1e-10);
        assert!(buf.variance() > 0.0);
    }

    #[test]
    fn residual_buffer_wraps_correctly() {
        let mut buf = ResidualBuffer::new(5);
        for i in 0..10 {
            buf.push(i as f64);
        }
        // Should only contain [5, 6, 7, 8, 9].
        assert_eq!(buf.len(), 5);
        assert!((buf.mean() - 7.0).abs() < 1e-10);
        assert_eq!(buf.total_count(), 10);
    }

    #[test]
    fn residual_buffer_coverage() {
        let mut buf = ResidualBuffer::new(100);
        // 85 values within 0.5, 15 outside.
        for _ in 0..85 {
            buf.push(0.3);
        }
        for _ in 0..15 {
            buf.push(2.0);
        }
        let cov = buf.coverage_rate(0.5);
        assert!(
            (cov - 0.85).abs() < 0.01,
            "coverage should be ~0.85, got {cov}"
        );
    }

    // ─── ResidualCorrector tests ────────────────────────────────────

    #[test]
    fn corrector_removes_systematic_bias() {
        let mut corrector = ResidualCorrector::new();

        // Consistently overpredicting by 0.1.
        for _ in 0..50 {
            corrector.record("impl", "plan-a", "success", 0.1);
        }

        let result = corrector.correct("impl", "plan-a", "success", 0.8, 0.2);
        // Center should be reduced by ~0.1 (the mean bias).
        assert!(
            (result.center - 0.7).abs() < 0.02,
            "bias correction should reduce center: got {}",
            result.center
        );
    }

    #[test]
    fn corrector_widens_undercovering_intervals() {
        let mut corrector = ResidualCorrector::new();

        // Large residuals that exceed the interval → undercovering.
        for _ in 0..50 {
            corrector.record("impl", "plan-a", "cost", 0.5); // Way outside typical interval
        }

        let result = corrector.correct("impl", "plan-a", "cost", 1.0, 0.2);
        assert!(
            result.half_width > 0.2,
            "should widen interval when undercovering: got {}",
            result.half_width
        );
    }

    #[test]
    fn corrector_returns_uncorrected_for_unknown_key() {
        let corrector = ResidualCorrector::new();
        let result = corrector.correct("unknown", "ctx", "metric", 0.5, 0.1);
        assert!((result.center - 0.5).abs() < f64::EPSILON);
        assert!((result.half_width - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn corrector_difficulty_weight_increases_with_variance() {
        let mut low_var = ResidualCorrector::new();
        let mut high_var = ResidualCorrector::new();

        // Low variance residuals.
        for _ in 0..50 {
            low_var.record("a", "b", "c", 0.01);
        }
        // High variance residuals.
        for i in 0..50 {
            high_var.record("a", "b", "c", if i % 2 == 0 { 0.5 } else { -0.5 });
        }

        let low = low_var.correct("a", "b", "c", 0.5, 0.3);
        let high = high_var.correct("a", "b", "c", 0.5, 0.3);

        assert!(
            high.difficulty_weight > low.difficulty_weight,
            "high variance should have higher difficulty: {} vs {}",
            high.difficulty_weight,
            low.difficulty_weight
        );
    }
}
