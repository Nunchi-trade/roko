//! Prediction records for routing calibration and residual tracking.

use chrono::Utc;
use roko_core::{PredictionCalibrationSource, PredictionCalibrationSummary};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::routing_log::{RoutingDecisionLog, RoutingDecisionLogStore};

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
        CalibrationTracker, PredictionRecord, fallback_stage_probability, selected_probability,
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
}
