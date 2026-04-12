//! Prediction records for routing calibration and residual tracking.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
            predicted_success_prob,
            predicted_cost_usd,
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
}

/// Aggregated calibration data for model/category pairs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalibrationTracker {
    /// Success residuals keyed by `(model_slug, task_category)`.
    residuals: HashMap<(String, String), Vec<f64>>,
}

impl CalibrationTracker {
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
            .push(residual);
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

    /// Average residual for a model/category pair.
    #[must_use]
    pub fn mean_bias(&self, model: &str, category: &str) -> f64 {
        let Some(residuals) = self
            .residuals
            .get(&(model.to_string(), category.to_string()))
        else {
            return 0.0;
        };

        if residuals.is_empty() {
            0.0
        } else {
            residuals.iter().sum::<f64>() / residuals.len() as f64
        }
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

    /// Apply the learned bias correction to a raw success prediction.
    #[must_use]
    pub fn adjust_prediction(&self, model: &str, category: &str, raw_pred: f64) -> f64 {
        (raw_pred - self.mean_bias(model, category)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{CalibrationTracker, PredictionRecord};

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
}
