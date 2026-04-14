//! Anomaly detection for runaway agent loops, cost spikes, and quality drift.
//!
//! This module keeps lightweight session-local state so the dispatch pipeline
//! can detect repeated prompt loops, sudden cost spikes, and sustained quality
//! degradation without depending on heavier planner or conductor machinery.

use std::collections::VecDeque;

const PROMPT_LOOP_WINDOW: usize = 20;
const PROMPT_LOOP_THRESHOLD: usize = 5;
const COST_SPIKE_Z_THRESHOLD: f64 = 3.0;
const QUALITY_WINDOW: usize = 50;
const QUALITY_RECENT_WINDOW: usize = 5;
const QUALITY_EARLIER_WINDOW: usize = 10;
const QUALITY_MIN_SAMPLES: usize = QUALITY_RECENT_WINDOW + QUALITY_EARLIER_WINDOW;
const DEFAULT_EWMA_ALPHA: f64 = 0.2;

/// Session-local detector for runaway loops and cost spikes.
#[derive(Debug, Clone)]
pub struct AnomalyDetector {
    prompt_hash_window: VecDeque<u64>,
    cost_ewma: EwmaState,
    quality_history: VecDeque<f64>,
    session_cost_usd: f64,
    session_start_ms: i64,
}

impl AnomalyDetector {
    /// Create a detector for a new session.
    #[must_use]
    pub fn new(session_start_ms: i64) -> Self {
        Self {
            prompt_hash_window: VecDeque::with_capacity(PROMPT_LOOP_WINDOW),
            cost_ewma: EwmaState::default(),
            quality_history: VecDeque::with_capacity(QUALITY_WINDOW),
            session_cost_usd: 0.0,
            session_start_ms,
        }
    }

    /// Return the session start timestamp in Unix milliseconds.
    #[must_use]
    pub const fn session_start_ms(&self) -> i64 {
        self.session_start_ms
    }

    /// Check a prompt hash for repeated-loop behavior.
    ///
    /// Returns [`Anomaly::PromptLoop`] when the same hash appears at least
    /// five times in the last twenty prompts.
    #[must_use]
    pub fn check_prompt(&mut self, prompt_hash: u64) -> Option<Anomaly> {
        self.prompt_hash_window.push_back(prompt_hash);
        if self.prompt_hash_window.len() > PROMPT_LOOP_WINDOW {
            self.prompt_hash_window.pop_front();
        }

        let repeated_count = self
            .prompt_hash_window
            .iter()
            .filter(|&&hash| hash == prompt_hash)
            .count();

        if repeated_count >= PROMPT_LOOP_THRESHOLD {
            Some(Anomaly::PromptLoop { repeated_count })
        } else {
            None
        }
    }

    /// Check a cost observation for a spike relative to the EWMA baseline.
    ///
    /// The new value is compared against the current EWMA state before the
    /// state is updated, which keeps sudden spikes visible instead of folding
    /// them into the baseline immediately.
    #[must_use]
    pub fn check_cost(&mut self, cost_usd: f64) -> Option<Anomaly> {
        let z_score = self.cost_ewma.z_score(cost_usd);
        self.cost_ewma.update(cost_usd);
        self.session_cost_usd += cost_usd;

        if z_score > COST_SPIKE_Z_THRESHOLD {
            Some(Anomaly::CostSpike { z_score })
        } else {
            None
        }
    }

    /// Check a rolling quality score for sustained degradation.
    ///
    /// The detector compares the most recent five scores against the ten
    /// scores immediately before them. If the average drops by more than 0.15
    /// and the recent average is below 0.5, it flags degradation.
    #[must_use]
    pub fn check_quality(&mut self, score: f64) -> Option<Anomaly> {
        self.quality_history.push_back(score);
        if self.quality_history.len() > QUALITY_WINDOW {
            self.quality_history.pop_front();
        }

        if self.quality_history.len() < QUALITY_MIN_SAMPLES {
            return None;
        }

        let recent_avg = average_recent(&self.quality_history, QUALITY_RECENT_WINDOW);
        let earlier_avg = average_earlier(
            &self.quality_history,
            QUALITY_RECENT_WINDOW,
            QUALITY_EARLIER_WINDOW,
        );
        let avg_drop = earlier_avg - recent_avg;

        if recent_avg < earlier_avg - 0.15 && recent_avg < 0.5 {
            Some(Anomaly::QualityDegradation { avg_drop })
        } else {
            None
        }
    }

    /// Check whether the accumulated session cost has exhausted a budget.
    #[must_use]
    pub fn check_budget(&self, limit_usd: f64) -> Option<Anomaly> {
        if self.session_cost_usd >= limit_usd {
            Some(Anomaly::BudgetExhausted {
                used: self.session_cost_usd,
                limit: limit_usd,
            })
        } else {
            None
        }
    }
}

fn average_recent(history: &VecDeque<f64>, count: usize) -> f64 {
    let sum: f64 = history.iter().rev().take(count).copied().sum();
    sum / count as f64
}

fn average_earlier(history: &VecDeque<f64>, recent_count: usize, count: usize) -> f64 {
    let sum: f64 = history
        .iter()
        .rev()
        .skip(recent_count)
        .take(count)
        .copied()
        .sum();
    sum / count as f64
}

/// Exponentially weighted moving average state used for cost spike detection.
#[derive(Debug, Clone)]
pub struct EwmaState {
    /// Current EWMA mean.
    pub mean: f64,
    /// Current EWMA variance estimate.
    pub variance: f64,
    alpha: f64,
}

impl EwmaState {
    /// Create a new EWMA state with a custom smoothing factor.
    #[must_use]
    pub fn new(alpha: f64) -> Self {
        Self {
            mean: 0.0,
            variance: 0.0,
            alpha: sanitize_alpha(alpha),
        }
    }

    /// Update the EWMA state with a new observation.
    pub fn update(&mut self, value: f64) {
        let diff = value - self.mean;
        self.mean += self.alpha * diff;
        self.variance = (1.0 - self.alpha) * (self.variance + self.alpha * diff * diff);
    }

    /// Compute the z-score of a value relative to the current EWMA state.
    #[must_use]
    pub fn z_score(&self, value: f64) -> f64 {
        let stddev = self.variance.sqrt();
        if stddev < 1e-10 {
            0.0
        } else {
            (value - self.mean) / stddev
        }
    }
}

impl Default for EwmaState {
    fn default() -> Self {
        Self::new(DEFAULT_EWMA_ALPHA)
    }
}

fn sanitize_alpha(alpha: f64) -> f64 {
    if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        DEFAULT_EWMA_ALPHA
    }
}

/// An anomaly detected in the current session.
#[derive(Debug, Clone, PartialEq)]
pub enum Anomaly {
    /// The same prompt hash appeared repeatedly in the sliding window.
    PromptLoop {
        /// Number of repeated occurrences seen in the window.
        repeated_count: usize,
    },
    /// The current cost is more than three standard deviations above baseline.
    CostSpike {
        /// Cost z-score against the EWMA baseline.
        z_score: f64,
    },
    /// Quality dropped materially across the recent rolling window.
    QualityDegradation {
        /// Average drop between the earlier and recent windows.
        avg_drop: f64,
    },
    /// The running session cost has exhausted the configured budget.
    BudgetExhausted {
        /// Total cost used so far.
        used: f64,
        /// Budget limit that was exceeded.
        limit: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::{Anomaly, AnomalyDetector};

    #[test]
    fn anomaly_detector_prompt_loop_detection_triggers_at_five_identical_hashes() {
        let mut detector = AnomalyDetector::new(1_700_000_000_000);

        for _ in 0..4 {
            assert!(detector.check_prompt(42).is_none());
        }

        let anomaly = detector
            .check_prompt(42)
            .expect("expected prompt loop at five repeats");

        assert_eq!(anomaly, Anomaly::PromptLoop { repeated_count: 5 });
    }

    #[test]
    fn anomaly_detector_cost_spike_detection_triggers_on_large_z_score() {
        let mut detector = AnomalyDetector::new(1_700_000_000_000);

        for cost in [1.0, 1.2, 0.9, 1.1, 1.05, 0.95, 1.15, 1.0] {
            assert!(detector.check_cost(cost).is_none());
        }

        let anomaly = detector
            .check_cost(10.0)
            .expect("expected cost spike anomaly");

        match anomaly {
            Anomaly::CostSpike { z_score } => {
                assert!(z_score > 3.0, "expected z-score > 3.0, got {z_score}");
            }
            other => panic!("expected cost spike, got {other:?}"),
        }
    }

    #[test]
    fn anomaly_detector_quality_degradation_triggers_after_sustained_drop() {
        let mut detector = AnomalyDetector::new(1_700_000_000_000);

        for _ in 0..10 {
            assert!(detector.check_quality(0.82).is_none());
        }

        let mut fired = None;
        for score in [0.34, 0.32, 0.31, 0.29, 0.30] {
            fired = detector.check_quality(score);
        }

        match fired.expect("expected quality degradation") {
            Anomaly::QualityDegradation { avg_drop } => {
                assert!(avg_drop > 0.15);
            }
            other => panic!("expected quality degradation, got {other:?}"),
        }
    }

    #[test]
    fn anomaly_detector_budget_exhaustion_triggers_when_cost_exceeds_limit() {
        let mut detector = AnomalyDetector::new(1_700_000_000_000);

        assert!(detector.check_cost(3.0).is_none());
        assert!(detector.check_budget(5.0).is_none());

        assert!(detector.check_cost(2.5).is_none());

        match detector
            .check_budget(5.0)
            .expect("expected budget exhaustion")
        {
            Anomaly::BudgetExhausted { used, limit } => {
                assert!(used >= limit);
                assert_eq!(limit, 5.0);
            }
            other => panic!("expected budget exhaustion, got {other:?}"),
        }
    }
}
