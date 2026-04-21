//! Adaptive threshold learning for conductor watchers (COND-03).
//!
//! After each intervention, the conductor records whether the intervention
//! improved the outcome. This feedback adjusts watcher thresholds via EMA,
//! making the conductor more precise over time.
//!
//! Follows the EMA pattern from `roko-gate/src/adaptive_threshold.rs`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::Path;

/// EMA smoothing factor. 0.1 means recent observations weigh more heavily.
const DEFAULT_ALPHA: f64 = 0.1;

/// Default restart threshold severity level.
const DEFAULT_RESTART_THRESHOLD: f64 = 0.7;

/// Default fail threshold severity level.
const DEFAULT_FAIL_THRESHOLD: f64 = 0.9;

/// Minimum observations before adaptive thresholds override defaults.
const WARMUP_OBSERVATIONS: u64 = 10;

/// Maximum history entries kept per watcher.
const MAX_HISTORY: usize = 100;

/// Per-watcher adaptive threshold state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveThreshold {
    /// Current EMA of the optimal intervention boundary.
    pub ema: f64,
    /// Total observations for this watcher.
    pub observations: u64,
    /// Interventions that were effective (task succeeded after).
    pub effective_count: u64,
    /// Interventions that were ineffective (task still failed after).
    pub ineffective_count: u64,
}

impl AdaptiveThreshold {
    /// Create a new threshold starting at the given default.
    fn new(default: f64) -> Self {
        Self {
            ema: default,
            observations: 0,
            effective_count: 0,
            ineffective_count: 0,
        }
    }

    /// Update the threshold based on intervention outcome.
    ///
    /// If the intervention was effective, lower the threshold slightly
    /// (intervene earlier next time). If ineffective, raise it (intervene
    /// later to avoid wasting resources).
    fn update(&mut self, alpha: f64, effective: bool) {
        self.observations += 1;
        if effective {
            self.effective_count += 1;
        } else {
            self.ineffective_count += 1;
        }

        // Target: lower threshold if effective (intervene earlier),
        // raise if ineffective (intervene later).
        let target = if effective {
            (self.ema - 0.05).max(0.1)
        } else {
            (self.ema + 0.05).min(1.0)
        };

        if self.observations == 1 {
            self.ema = target;
        } else {
            self.ema = alpha.mul_add(target, (1.0 - alpha) * self.ema);
        }
    }

    /// Whether enough observations have accumulated for this threshold to
    /// be used instead of the static default.
    fn is_warmed_up(&self) -> bool {
        self.observations >= WARMUP_OBSERVATIONS
    }
}

/// Outcome of a conductor intervention, recorded for learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterventionOutcome {
    /// Name of the watcher that triggered the intervention.
    pub watcher_name: String,
    /// Severity at which the watcher fired.
    pub severity_at_fire: f64,
    /// The decision that was taken (label: "restart", "fail", etc.).
    pub decision_label: String,
    /// Whether the task improved after the intervention.
    pub task_improved: bool,
    /// Unix milliseconds when the outcome was recorded.
    pub recorded_at_ms: i64,
}

/// Learns optimal watcher thresholds from intervention outcomes.
///
/// Persists to `.roko/learn/conductor-thresholds.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdLearner {
    /// Per-watcher adaptive thresholds.
    pub watcher_thresholds: HashMap<String, AdaptiveThreshold>,
    /// Recent intervention outcomes (ring buffer).
    pub intervention_history: VecDeque<InterventionOutcome>,
    /// EMA smoothing factor.
    pub alpha: f64,
    /// Default restart threshold (used before warmup).
    pub default_restart: f64,
    /// Default fail threshold (used before warmup).
    pub default_fail: f64,
}

impl Default for ThresholdLearner {
    fn default() -> Self {
        Self::new()
    }
}

impl ThresholdLearner {
    /// Create a new learner with default parameters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            watcher_thresholds: HashMap::new(),
            intervention_history: VecDeque::new(),
            alpha: DEFAULT_ALPHA,
            default_restart: DEFAULT_RESTART_THRESHOLD,
            default_fail: DEFAULT_FAIL_THRESHOLD,
        }
    }

    /// Load from a JSON file, or create new if missing/corrupt.
    pub fn load_or_new(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save to a JSON file (atomic write).
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be serialized or the file
    /// system operations fail.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Record an intervention outcome and update the corresponding threshold.
    pub fn record_outcome(&mut self, outcome: InterventionOutcome) {
        let watcher = outcome.watcher_name.clone();
        let effective = outcome.task_improved;

        // Update the adaptive threshold for this watcher.
        let threshold = self
            .watcher_thresholds
            .entry(watcher)
            .or_insert_with(|| AdaptiveThreshold::new(self.default_restart));
        threshold.update(self.alpha, effective);

        // Add to history ring buffer.
        self.intervention_history.push_back(outcome);
        if self.intervention_history.len() > MAX_HISTORY {
            self.intervention_history.pop_front();
        }
    }

    /// Get the current restart threshold for a watcher.
    ///
    /// Returns the adaptive threshold if warmed up, otherwise the default.
    #[must_use]
    pub fn restart_threshold(&self, watcher: &str) -> f64 {
        self.watcher_thresholds
            .get(watcher)
            .filter(|t| t.is_warmed_up())
            .map_or(self.default_restart, |t| t.ema)
    }

    /// Get the current fail threshold for a watcher.
    ///
    /// The fail threshold is always higher than the restart threshold.
    #[must_use]
    pub fn fail_threshold(&self, watcher: &str) -> f64 {
        let restart = self.restart_threshold(watcher);
        // Fail threshold is restart + a gap, clamped to [restart+0.05, 1.0].
        let gap = self.default_fail - self.default_restart;
        (restart + gap).min(1.0)
    }

    /// Total observations across all watchers.
    #[must_use]
    pub fn total_observations(&self) -> u64 {
        self.watcher_thresholds
            .values()
            .map(|t| t.observations)
            .sum()
    }

    /// Per-watcher effectiveness rate (effective / total).
    #[must_use]
    pub fn effectiveness_rate(&self, watcher: &str) -> Option<f64> {
        self.watcher_thresholds.get(watcher).map(|t| {
            if t.observations == 0 {
                0.0
            } else {
                t.effective_count as f64 / t.observations as f64
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_learner_uses_defaults() {
        let learner = ThresholdLearner::new();
        assert_eq!(
            learner.restart_threshold("unknown"),
            DEFAULT_RESTART_THRESHOLD
        );
        assert_eq!(learner.total_observations(), 0);
    }

    #[test]
    fn effective_interventions_lower_threshold() {
        let mut learner = ThresholdLearner::new();
        let initial = learner.restart_threshold("ghost-turn");

        // Record enough effective interventions to warm up and lower threshold.
        for i in 0..20 {
            learner.record_outcome(InterventionOutcome {
                watcher_name: "ghost-turn".to_string(),
                severity_at_fire: 0.7,
                decision_label: "restart".to_string(),
                task_improved: true,
                recorded_at_ms: i * 1000,
            });
        }

        let after = learner.restart_threshold("ghost-turn");
        assert!(
            after < initial,
            "threshold should decrease after effective interventions: {after} < {initial}"
        );
    }

    #[test]
    fn ineffective_interventions_raise_threshold() {
        let mut learner = ThresholdLearner::new();
        let initial = learner.restart_threshold("cost-overrun");

        for i in 0..20 {
            learner.record_outcome(InterventionOutcome {
                watcher_name: "cost-overrun".to_string(),
                severity_at_fire: 0.7,
                decision_label: "restart".to_string(),
                task_improved: false,
                recorded_at_ms: i * 1000,
            });
        }

        let after = learner.restart_threshold("cost-overrun");
        assert!(
            after > initial,
            "threshold should increase after ineffective interventions: {after} > {initial}"
        );
    }

    #[test]
    fn warmup_period_uses_default() {
        let mut learner = ThresholdLearner::new();

        // Only 5 observations -- below warmup threshold.
        for i in 0..5 {
            learner.record_outcome(InterventionOutcome {
                watcher_name: "test-watcher".to_string(),
                severity_at_fire: 0.7,
                decision_label: "restart".to_string(),
                task_improved: true,
                recorded_at_ms: i * 1000,
            });
        }

        // Should still return default since not warmed up.
        assert_eq!(
            learner.restart_threshold("test-watcher"),
            DEFAULT_RESTART_THRESHOLD
        );
    }

    #[test]
    fn history_ring_buffer_caps_at_max() {
        let mut learner = ThresholdLearner::new();

        for i in 0..(MAX_HISTORY + 50) {
            learner.record_outcome(InterventionOutcome {
                watcher_name: "test".to_string(),
                severity_at_fire: 0.7,
                decision_label: "restart".to_string(),
                task_improved: i % 2 == 0,
                recorded_at_ms: i as i64 * 1000,
            });
        }

        assert_eq!(learner.intervention_history.len(), MAX_HISTORY);
    }

    #[test]
    fn fail_threshold_always_above_restart() {
        let mut learner = ThresholdLearner::new();

        for i in 0..20 {
            learner.record_outcome(InterventionOutcome {
                watcher_name: "test".to_string(),
                severity_at_fire: 0.7,
                decision_label: "restart".to_string(),
                task_improved: true,
                recorded_at_ms: i * 1000,
            });
        }

        let restart = learner.restart_threshold("test");
        let fail = learner.fail_threshold("test");
        assert!(
            fail > restart,
            "fail threshold ({fail}) should be above restart ({restart})"
        );
    }

    #[test]
    fn effectiveness_rate_tracks_correctly() {
        let mut learner = ThresholdLearner::new();

        for i in 0..10 {
            learner.record_outcome(InterventionOutcome {
                watcher_name: "test".to_string(),
                severity_at_fire: 0.7,
                decision_label: "restart".to_string(),
                task_improved: i < 7, // 7 effective, 3 ineffective
                recorded_at_ms: i * 1000,
            });
        }

        let rate = learner.effectiveness_rate("test").unwrap();
        assert!((rate - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn persistence_round_trip() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join("conductor-thresholds.json");

        let mut learner = ThresholdLearner::new();
        for i in 0..15 {
            learner.record_outcome(InterventionOutcome {
                watcher_name: "ghost-turn".to_string(),
                severity_at_fire: 0.7,
                decision_label: "restart".to_string(),
                task_improved: true,
                recorded_at_ms: i * 1000,
            });
        }
        learner.save(&path).expect("save");

        let loaded = ThresholdLearner::load_or_new(&path);
        assert_eq!(loaded.total_observations(), 15);
        assert_eq!(
            loaded.effectiveness_rate("ghost-turn"),
            learner.effectiveness_rate("ghost-turn")
        );
    }

    #[test]
    fn unknown_watcher_has_no_effectiveness() {
        let learner = ThresholdLearner::new();
        assert!(learner.effectiveness_rate("nonexistent").is_none());
    }
}
