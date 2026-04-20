//! Theta reflective loop consumer (BEAT-01).
//!

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::return_self_not_must_use,
    clippy::unnecessary_literal_bound,
    clippy::unused_self,
    clippy::derive_partial_eq_without_eq
)]

//!
//! Runs a five-phase reflection cycle at configurable intervals (default 75s,
//! range 30-120s). Theta always costs T1-T2 and fires every N gamma ticks or
//! on episode completion.
//!
//! Five phases:
//! 1. Summarize recent gamma work
//! 2. Update Daimon affect (ALMA layers)
//! 3. Check prediction calibration drift
//! 4. Re-evaluate plan progress vs DAG schedule
//! 5. Trigger interventions if stuck

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::heartbeat::{
    CognitiveSignal, CorticalState, DecisionCycleRecord, GammaSummary, MetaCognitionResult,
    MetaIssue, PadVector, RetryTracker, meta_cognize, summarize_gamma_history,
};

/// Configuration for the theta reflective loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThetaConfig {
    /// Base interval in seconds (default 75, range 30-120).
    pub interval_secs: u64,
    /// Maximum recent gamma records to retain for summarization.
    pub gamma_buffer_size: usize,
    /// Consecutive failure count that triggers stuck detection.
    pub stuck_threshold: u32,
    /// Approach-change count that triggers thrashing detection.
    pub thrash_threshold: u32,
    /// Window of recent attempts examined for thrashing.
    pub thrash_window: usize,
    /// Accuracy drift threshold that triggers recalibration.
    pub calibration_drift_threshold: f32,
}

impl Default for ThetaConfig {
    fn default() -> Self {
        Self {
            interval_secs: 75,
            gamma_buffer_size: 50,
            stuck_threshold: 3,
            thrash_threshold: 3,
            thrash_window: 6,
            calibration_drift_threshold: 0.15,
        }
    }
}

/// Outcome of a single theta tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThetaOutcome {
    /// UTC timestamp of this theta tick.
    pub timestamp: DateTime<Utc>,
    /// Summary of recent gamma work (phase 1).
    pub gamma_summary: GammaSummary,
    /// Affect update applied (phase 2).
    pub affect_update: AffectUpdate,
    /// Calibration check result (phase 3).
    pub calibration: CalibrationResult,
    /// Plan progress assessment (phase 4).
    pub plan_progress: PlanProgress,
    /// Meta-cognition result and interventions (phase 5).
    pub meta_cognition: MetaCognitionResult,
    /// Cognitive signals emitted.
    pub signals_emitted: Vec<CognitiveSignal>,
}

/// Result of phase 2: affect update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffectUpdate {
    /// PAD before update.
    pub pad_before: PadVector,
    /// PAD after update.
    pub pad_after: PadVector,
    /// Whether a significant affect shift occurred.
    pub significant_shift: bool,
}

/// Result of phase 3: calibration check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationResult {
    /// Current aggregate prediction accuracy.
    pub current_accuracy: f32,
    /// Accuracy trend over recent ticks.
    pub accuracy_trend: f32,
    /// Whether drift exceeds the configured threshold.
    pub drift_detected: bool,
    /// Categories with significant drift.
    pub drifting_categories: Vec<String>,
}

/// Result of phase 4: plan progress assessment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanProgress {
    /// Current task identifier, if any.
    pub current_task: Option<String>,
    /// Fraction of plan completed in `[0.0, 1.0]`.
    pub completion_fraction: f32,
    /// Whether the plan appears stalled.
    pub stalled: bool,
    /// Number of consecutive failures on the current task.
    pub consecutive_failures: u32,
}

/// Context provided to the theta consumer on each tick.
pub struct ThetaContext<'a> {
    /// Cortical state for reading/writing affect and prediction signals.
    pub cortical: &'a CorticalState,
    /// Current plan task identifier, if any.
    pub current_task_id: Option<String>,
    /// Total tasks in the current plan.
    pub total_tasks: usize,
    /// Completed tasks in the current plan.
    pub completed_tasks: usize,
}

/// The theta reflective loop consumer.
pub struct ThetaConsumer {
    config: ThetaConfig,
    last_tick_ms: i64,
    gamma_buffer: VecDeque<DecisionCycleRecord>,
    retry_tracker: RetryTracker,
}

impl ThetaConsumer {
    /// Create a new theta consumer with the given configuration.
    pub fn new(config: ThetaConfig) -> Self {
        let gamma_buffer_size = config.gamma_buffer_size;
        Self {
            config,
            last_tick_ms: 0,
            gamma_buffer: VecDeque::with_capacity(gamma_buffer_size),
            retry_tracker: RetryTracker::new(10),
        }
    }

    /// Access the configuration.
    pub const fn config(&self) -> &ThetaConfig {
        &self.config
    }

    /// Millisecond timestamp of the last theta tick.
    pub const fn last_tick_ms(&self) -> i64 {
        self.last_tick_ms
    }

    /// Ingest a gamma decision cycle record for later summarization.
    pub fn ingest_gamma_record(&mut self, record: DecisionCycleRecord) {
        if self.gamma_buffer.len() >= self.config.gamma_buffer_size {
            self.gamma_buffer.pop_front();
        }
        // Track retries for meta-cognition
        if let Some(outcome) = &record.outcome {
            let task_id = record.agent_id.clone();
            let approach = record.gating_reason.clone();
            if !outcome.passed {
                self.retry_tracker.record_attempt(&task_id, &approach);
            }
        }
        self.gamma_buffer.push_back(record);
    }

    /// Run the five-phase theta cycle.
    pub fn tick(&mut self, ctx: &ThetaContext<'_>) -> ThetaOutcome {
        self.last_tick_ms = Utc::now().timestamp_millis();

        // Phase 1: Summarize recent gamma work
        let records: Vec<_> = self.gamma_buffer.iter().cloned().collect();
        let gamma_summary = summarize_gamma_history(&records);

        // Phase 2: Update affect based on recent performance
        let affect_update = self.update_affect(ctx.cortical, &gamma_summary);

        // Phase 3: Check prediction calibration
        let calibration = self.check_calibration(ctx.cortical);

        // Phase 4: Re-evaluate plan progress
        let plan_progress = self.evaluate_plan_progress(ctx);

        // Phase 5: Meta-cognition and interventions
        let meta_cognition = meta_cognize(
            &self.retry_tracker,
            ctx.cortical,
            self.config.stuck_threshold,
            self.config.thrash_threshold,
            self.config.thrash_window,
        );

        let signals_emitted = meta_cognition.signals();

        // Clear gamma buffer after summarization
        self.gamma_buffer.clear();

        ThetaOutcome {
            timestamp: Utc::now(),
            gamma_summary,
            affect_update,
            calibration,
            plan_progress,
            meta_cognition,
            signals_emitted,
        }
    }

    /// Phase 2: Update Daimon affect via ALMA three-layer model.
    ///
    /// The reactive layer responds immediately to outcome valence,
    /// the learned layer adjusts based on accumulated patterns,
    /// and the stable layer barely moves.
    fn update_affect(&self, cortical: &CorticalState, summary: &GammaSummary) -> AffectUpdate {
        let pad_before = cortical.pad();

        // Reactive layer: respond to success rate
        let pleasure_delta = (f64::from(summary.success_rate) - 0.5) * 0.2;
        // Arousal tracks anomaly density
        let anomaly_density =
            summary.recurring_anomalies.len() as f64 / f64::from(summary.tick_count.max(1));
        let arousal_delta = (anomaly_density - 0.1) * 0.3;
        // Dominance tracks cost efficiency
        let cost_per_tick = if summary.tick_count > 0 {
            summary.total_cost / f64::from(summary.tick_count)
        } else {
            0.0
        };
        let dominance_delta: f64 = if cost_per_tick < 0.01 { 0.05 } else { -0.05 };

        let pad_after = PadVector {
            pleasure: (pad_before.pleasure + pleasure_delta).clamp(-1.0, 1.0),
            arousal: (pad_before.arousal + arousal_delta).clamp(-1.0, 1.0),
            dominance: (pad_before.dominance + dominance_delta).clamp(-1.0, 1.0),
        };

        cortical.set_pad(pad_after);

        let significant_shift = (pad_after.pleasure - pad_before.pleasure).abs() > 0.1
            || (pad_after.arousal - pad_before.arousal).abs() > 0.1
            || (pad_after.dominance - pad_before.dominance).abs() > 0.1;

        AffectUpdate {
            pad_before,
            pad_after,
            significant_shift,
        }
    }

    /// Phase 3: Check prediction calibration for drift.
    fn check_calibration(&self, cortical: &CorticalState) -> CalibrationResult {
        let snapshot = cortical.snapshot();
        let current_accuracy = snapshot.aggregate_accuracy;

        // Use accuracy trend from cortical state
        let accuracy_trend = match snapshot.accuracy_trend {
            t if t > 0 => 0.1,
            t if t < 0 => -0.1,
            _ => 0.0,
        };

        // Check per-category drift
        let mut drifting_categories = Vec::new();
        for (i, &accuracy) in snapshot.category_accuracies.iter().enumerate() {
            if (accuracy - current_accuracy).abs() > self.config.calibration_drift_threshold {
                drifting_categories.push(format!("category_{i}"));
            }
        }

        let drift_detected = accuracy_trend < -0.05 || !drifting_categories.is_empty();

        CalibrationResult {
            current_accuracy,
            accuracy_trend,
            drift_detected,
            drifting_categories,
        }
    }

    /// Phase 4: Evaluate plan progress against the DAG schedule.
    fn evaluate_plan_progress(&self, ctx: &ThetaContext<'_>) -> PlanProgress {
        let completion_fraction = if ctx.total_tasks > 0 {
            ctx.completed_tasks as f32 / ctx.total_tasks as f32
        } else {
            0.0
        };

        let consecutive_failures = self.retry_tracker.retry_count;
        let stalled = consecutive_failures >= self.config.stuck_threshold;

        PlanProgress {
            current_task: ctx.current_task_id.clone(),
            completion_fraction,
            stalled,
            consecutive_failures,
        }
    }

    /// Number of gamma records currently buffered.
    pub fn gamma_buffer_len(&self) -> usize {
        self.gamma_buffer.len()
    }

    /// Whether interventions are warranted based on the most recent meta-cognition.
    pub fn has_stuck_issues(result: &MetaCognitionResult) -> bool {
        result
            .issues
            .iter()
            .any(|issue| matches!(issue, MetaIssue::Stuck { .. }))
    }
}

impl Default for ThetaConsumer {
    fn default() -> Self {
        Self::new(ThetaConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heartbeat::*;

    fn make_record(tick: u64, passed: bool) -> DecisionCycleRecord {
        DecisionCycleRecord {
            tick,
            timestamp: Utc::now(),
            agent_id: "test-agent".into(),
            observation: Observation {
                summary: "test".into(),
                signals: Default::default(),
            },
            regime: Regime::Normal,
            probe_results: vec![],
            anomalies: vec![],
            prediction_error: 0.1,
            deliberation_threshold: 0.2,
            tier: InferenceTier::T0,
            gating_reason: "low_error".into(),
            context_bundle_summary: ContextSummary {
                tokens_allocated: 0,
                sections: vec![],
            },
            retrieved_entries: vec![],
            active_interventions: vec![],
            deliberation: None,
            actions: vec![],
            outcome: Some(OutcomeRecord {
                passed,
                summary: "test outcome".into(),
                score: Some(if passed { 1.0 } else { 0.0 }),
            }),
            episodes_written: vec![],
            neuro_mutations: vec![],
            pad_before: PadVector::neutral(),
            pad_after: PadVector::neutral(),
            somatic_markers_fired: vec![],
            primary_emotion: PlutchikLabel::Joy,
            inference_cost: 0.001,
            domain_cost: 0.0,
            total_cost: 0.001,
        }
    }

    #[test]
    fn theta_consumer_ingests_and_summarizes() {
        let mut consumer = ThetaConsumer::default();
        for i in 0..5 {
            consumer.ingest_gamma_record(make_record(i, true));
        }
        assert_eq!(consumer.gamma_buffer_len(), 5);

        let cortical = CorticalState::default();
        let ctx = ThetaContext {
            cortical: &cortical,
            current_task_id: Some("task-1".into()),
            total_tasks: 10,
            completed_tasks: 3,
        };
        let outcome = consumer.tick(&ctx);
        assert_eq!(outcome.gamma_summary.tick_count, 5);
        assert!((outcome.gamma_summary.success_rate - 1.0).abs() < f32::EPSILON);
        assert_eq!(consumer.gamma_buffer_len(), 0);
    }

    #[test]
    fn theta_detects_stuck_state() {
        let mut consumer = ThetaConsumer::new(ThetaConfig {
            stuck_threshold: 2,
            ..ThetaConfig::default()
        });
        // Ingest multiple failures for the same agent
        for i in 0..4 {
            consumer.ingest_gamma_record(make_record(i, false));
        }

        let cortical = CorticalState::default();
        let ctx = ThetaContext {
            cortical: &cortical,
            current_task_id: Some("task-1".into()),
            total_tasks: 5,
            completed_tasks: 0,
        };
        let outcome = consumer.tick(&ctx);
        assert!(outcome.plan_progress.stalled || !outcome.meta_cognition.issues.is_empty());
    }

    #[test]
    fn theta_updates_affect() {
        let mut consumer = ThetaConsumer::default();
        // All successes should increase pleasure
        for i in 0..5 {
            consumer.ingest_gamma_record(make_record(i, true));
        }

        let cortical = CorticalState::default();
        let pad_before = cortical.pad();

        let ctx = ThetaContext {
            cortical: &cortical,
            current_task_id: None,
            total_tasks: 0,
            completed_tasks: 0,
        };
        let outcome = consumer.tick(&ctx);

        // Success rate = 1.0, so pleasure should increase
        assert!(outcome.affect_update.pad_after.pleasure >= pad_before.pleasure);
    }

    #[test]
    fn theta_plan_progress_tracks_completion() {
        let mut consumer = ThetaConsumer::default();
        consumer.ingest_gamma_record(make_record(0, true));

        let cortical = CorticalState::default();
        let ctx = ThetaContext {
            cortical: &cortical,
            current_task_id: Some("task-5".into()),
            total_tasks: 10,
            completed_tasks: 7,
        };
        let outcome = consumer.tick(&ctx);
        assert!((outcome.plan_progress.completion_fraction - 0.7).abs() < f32::EPSILON);
        assert!(!outcome.plan_progress.stalled);
    }

    #[test]
    fn theta_gamma_buffer_respects_capacity() {
        let mut consumer = ThetaConsumer::new(ThetaConfig {
            gamma_buffer_size: 3,
            ..ThetaConfig::default()
        });
        for i in 0..5 {
            consumer.ingest_gamma_record(make_record(i, true));
        }
        assert_eq!(consumer.gamma_buffer_len(), 3);
    }
}
