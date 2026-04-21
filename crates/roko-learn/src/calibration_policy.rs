//! CalibrationPolicy — closes the predict-publish-correct loop (LEARN-09).
//!
//! The policy listens for prediction events, matches them to outcomes, and
//! feeds residuals into the `CalibrationTracker`. When accumulated bias
//! exceeds a configurable threshold, it publishes a correction event that
//! downstream consumers (routers, composers) can react to.
//!
//! This is part of the Bus-backed cybernetic loops architecture: learners
//! subscribe to events rather than being called directly from the orchestrator.

use std::collections::HashMap;

use crate::events::AgentEvent;
use crate::prediction::CalibrationTracker;

/// A calibration correction generated when systematic bias is detected.
#[derive(Debug, Clone)]
pub struct CalibrationCorrection {
    /// The model that needs correction.
    pub model: String,
    /// The task category with bias.
    pub category: String,
    /// The mean bias detected (positive = overconfident).
    pub mean_bias: f64,
    /// Suggested correction to apply to future predictions.
    pub correction: f64,
    /// Number of observations backing this correction.
    pub sample_count: usize,
}

/// Policy that processes event streams to maintain calibration state.
///
/// The policy accumulates prediction residuals from completed turns and
/// triggers corrections when systematic bias is detected. It operates
/// as a stateful event processor in the Bus-backed architecture.
#[derive(Debug, Clone)]
pub struct CalibrationPolicy {
    /// Underlying tracker that accumulates residuals.
    tracker: CalibrationTracker,
    /// Bias threshold that triggers a correction (default: 0.15).
    bias_threshold: f64,
    /// Minimum sample count before corrections are emitted (default: 10).
    min_samples: usize,
    /// Pending predictions waiting for outcomes, keyed by task_id.
    pending: HashMap<String, PendingPrediction>,
    /// Corrections emitted since last reset.
    corrections: Vec<CalibrationCorrection>,
}

#[derive(Debug, Clone)]
struct PendingPrediction {
    model: String,
    category: String,
    predicted_success_prob: f64,
}

impl CalibrationPolicy {
    /// Create a new policy with default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tracker: CalibrationTracker::default(),
            bias_threshold: 0.15,
            min_samples: 10,
            pending: HashMap::new(),
            corrections: Vec::new(),
        }
    }

    /// Set the bias threshold that triggers corrections.
    #[must_use]
    pub fn with_bias_threshold(mut self, threshold: f64) -> Self {
        self.bias_threshold = threshold.clamp(0.01, 0.5);
        self
    }

    /// Set the minimum sample count before corrections are emitted.
    #[must_use]
    pub fn with_min_samples(mut self, min: usize) -> Self {
        self.min_samples = min.max(1);
        self
    }

    /// Process an agent event and update calibration state.
    ///
    /// Returns a correction if one is triggered by this event.
    pub fn process_event(&mut self, event: &AgentEvent) -> Option<CalibrationCorrection> {
        match event {
            AgentEvent::TurnStarted { task_id, model, .. } => {
                // Register a pending prediction. Use a default predicted
                // probability since the actual routing confidence is not
                // carried on TurnStarted. The calibration tracker only
                // needs the residual (predicted - actual).
                self.pending.insert(
                    task_id.clone(),
                    PendingPrediction {
                        model: model.clone(),
                        category: String::new(),     // filled on completion
                        predicted_success_prob: 0.7, // prior
                    },
                );
                None
            }
            AgentEvent::ModelSelected { model, score, .. } => {
                // If we have a pending prediction for this model, update
                // the predicted probability with the actual routing score.
                for pending in self.pending.values_mut() {
                    if pending.model == *model {
                        pending.predicted_success_prob = score.clamp(0.0, 1.0);
                    }
                }
                None
            }
            AgentEvent::TurnCompleted { gate_passed, .. } => {
                // Match the most recent pending prediction.
                // In practice the orchestrator processes one task at a time,
                // so we drain all pending entries on completion.
                let actual = if gate_passed.unwrap_or(false) {
                    1.0
                } else {
                    0.0
                };

                let entries: Vec<_> = self.pending.drain().collect();
                let mut correction = None;

                for (_task_id, pending) in entries {
                    let residual = pending.predicted_success_prob - actual;
                    let category = if pending.category.is_empty() {
                        "unknown"
                    } else {
                        &pending.category
                    };

                    self.tracker
                        .record_residual(&pending.model, category, residual);

                    // Check if bias exceeds threshold.
                    let samples = self.tracker.sample_count(&pending.model, category);
                    if samples >= self.min_samples {
                        let bias = self.tracker.mean_bias(&pending.model, category);
                        if bias.abs() > self.bias_threshold {
                            let c = CalibrationCorrection {
                                model: pending.model,
                                category: category.to_string(),
                                mean_bias: bias,
                                correction: -bias,
                                sample_count: samples,
                            };
                            self.corrections.push(c.clone());
                            correction = Some(c);
                        }
                    }
                }

                correction
            }
            _ => None,
        }
    }

    /// Drain all accumulated corrections.
    pub fn drain_corrections(&mut self) -> Vec<CalibrationCorrection> {
        std::mem::take(&mut self.corrections)
    }

    /// Return a reference to the underlying tracker.
    #[must_use]
    pub fn tracker(&self) -> &CalibrationTracker {
        &self.tracker
    }
}

impl Default for CalibrationPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_agent::Usage;
    use roko_agent::chat_types::FinishReason;

    fn turn_started(task_id: &str, model: &str) -> AgentEvent {
        AgentEvent::TurnStarted {
            task_id: task_id.to_string(),
            model: model.to_string(),
            provider: "test".to_string(),
            timestamp_ms: 0,
        }
    }

    fn turn_completed(passed: bool) -> AgentEvent {
        AgentEvent::TurnCompleted {
            turn: 1,
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_create_tokens: 0,
                cost_usd: 0.01,
                wall_ms: 100,
            },
            tool_call_count: 1,
            gate_passed: Some(passed),
            finish_reason: FinishReason::Stop,
        }
    }

    #[test]
    fn policy_accumulates_residuals() {
        let mut policy = CalibrationPolicy::new().with_min_samples(3);
        for i in 0..5 {
            let task_id = format!("task-{i}");
            policy.process_event(&turn_started(&task_id, "model-a"));
            policy.process_event(&turn_completed(false));
        }
        assert!(policy.tracker().sample_count("model-a", "unknown") >= 5);
    }

    #[test]
    fn policy_triggers_correction_on_bias() {
        let mut policy = CalibrationPolicy::new()
            .with_min_samples(3)
            .with_bias_threshold(0.1);

        let mut correction_found = false;
        // Predict high (0.7) but always fail -> bias is 0.7.
        for i in 0..5 {
            policy.process_event(&turn_started(&format!("t-{i}"), "model-b"));
            if let Some(c) = policy.process_event(&turn_completed(false)) {
                assert!(c.mean_bias > 0.1);
                correction_found = true;
            }
        }
        assert!(correction_found);
    }

    #[test]
    fn no_correction_when_well_calibrated() {
        let mut policy = CalibrationPolicy::new()
            .with_min_samples(3)
            .with_bias_threshold(0.5);

        for i in 0..10 {
            policy.process_event(&turn_started(&format!("t-{i}"), "model-c"));
            // Alternate pass/fail to keep bias near zero.
            let pass = i % 2 == 0;
            let correction = policy.process_event(&turn_completed(pass));
            if let Some(c) = correction {
                // Should not trigger with alternating results and high threshold.
                assert!(
                    c.mean_bias.abs() <= 0.5,
                    "unexpected correction: bias={}",
                    c.mean_bias
                );
            }
        }
    }
}
