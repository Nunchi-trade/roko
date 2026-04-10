//! Stuck detection: heuristics that analyze agent activity history for stuck patterns.
//!
//! The stuck detector is a pure function: given a slice of [`ActivityEntry`]
//! records, it evaluates a set of heuristics and returns a [`StuckSignal`]
//! when a stuck condition is detected. It holds no mutable state.
//!
//! # Usage
//!
//! ```rust
//! use roko_conductor::stuck_detection::{StuckDetector, StuckKind, ActivityEntry};
//!
//! let detector = StuckDetector::default();
//! let entries = vec![
//!     ActivityEntry::new(1000, "abc123", 0, None, 1),
//!     ActivityEntry::new(2000, "abc123", 0, None, 2),
//!     ActivityEntry::new(3000, "abc123", 0, None, 3),
//!     ActivityEntry::new(4000, "abc123", 0, None, 4),
//!     ActivityEntry::new(5000, "abc123", 0, None, 5),
//! ];
//! let signal = detector.check_stuck(&entries);
//! assert!(signal.is_some());
//! assert_eq!(signal.unwrap().kind, StuckKind::OutputLoop);
//! ```

use roko_core::{Body, Kind, OperatingFrequency, Signal};
use serde::{Deserialize, Serialize};

// ---- StuckKind --------------------------------------------------------------

/// Classification of stuck condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StuckKind {
    /// Same output repeated consecutively.
    OutputLoop,
    /// No file changes for an extended period.
    NoProgress,
    /// Same gate failure repeated.
    GateLoop,
    /// Same compile error repeated.
    CompileLoop,
    /// Agent producing no output (empty entries).
    EmptyOutput,
    /// Excessive retries on the same task.
    ExcessiveRetries,
}

// ---- StuckSignal ------------------------------------------------------------

/// A detected stuck condition with metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StuckSignal {
    /// Classification of the stuck condition.
    pub kind: StuckKind,
    /// Confidence of the detection (0.0 to 1.0).
    pub confidence: f64,
    /// Duration of the stuck condition in milliseconds (if applicable).
    pub duration_ms: Option<i64>,
    /// Human-readable description of the stuck condition.
    pub description: String,
}

// ---- ActivityEntry ----------------------------------------------------------

/// A single record of agent activity for stuck-detection analysis.
///
/// Callers build these from the signal stream or from task state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    /// Unix milliseconds when this activity occurred.
    pub timestamp_ms: i64,
    /// Hash of the agent's output (for loop detection).
    pub output_hash: String,
    /// Number of files changed in this activity.
    pub files_changed: u32,
    /// Gate result string, if a gate ran (e.g. "pass", "fail:compile", "fail:test").
    pub gate_result: Option<String>,
    /// Iteration number (monotonically increasing).
    pub iteration: u32,
}

impl ActivityEntry {
    /// Create a new activity entry.
    #[must_use]
    pub fn new(
        timestamp_ms: i64,
        output_hash: impl Into<String>,
        files_changed: u32,
        gate_result: Option<String>,
        iteration: u32,
    ) -> Self {
        Self {
            timestamp_ms,
            output_hash: output_hash.into(),
            files_changed,
            gate_result,
            iteration,
        }
    }
}

// ---- StuckDetector config ---------------------------------------------------

/// Configurable thresholds for stuck detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StuckThresholds {
    /// Number of consecutive identical outputs before firing `OutputLoop`.
    pub output_loop_count: usize,
    /// Duration in milliseconds with no file changes before firing `NoProgress`.
    pub no_progress_ms: i64,
    /// Number of consecutive identical gate failures before firing `GateLoop`.
    pub gate_loop_count: usize,
    /// Number of consecutive identical compile errors before firing `CompileLoop`.
    pub compile_loop_count: usize,
    /// Number of consecutive empty outputs before firing `EmptyOutput`.
    pub empty_output_count: usize,
    /// Total iteration count before firing `ExcessiveRetries`.
    pub excessive_retry_count: u32,
}

impl Default for StuckThresholds {
    fn default() -> Self {
        Self {
            output_loop_count: 4,
            no_progress_ms: 300_000, // 5 minutes
            gate_loop_count: 3,
            compile_loop_count: 3,
            empty_output_count: 3,
            excessive_retry_count: 6,
        }
    }
}

// ---- StuckDetector ----------------------------------------------------------

/// The stuck detector: analyzes a sequence of [`ActivityEntry`] records
/// for stuck patterns using configurable heuristics.
#[derive(Debug, Clone)]
pub struct StuckDetector {
    thresholds: StuckThresholds,
}

impl Default for StuckDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl StuckDetector {
    /// Create a detector with default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            thresholds: StuckThresholds::default(),
        }
    }

    /// Create a detector with custom thresholds.
    #[must_use]
    pub const fn with_thresholds(thresholds: StuckThresholds) -> Self {
        Self { thresholds }
    }

    /// Access the current thresholds.
    #[must_use]
    pub const fn thresholds(&self) -> &StuckThresholds {
        &self.thresholds
    }

    /// Check the activity history for stuck conditions.
    ///
    /// Returns the most severe stuck signal found, or `None` if the
    /// agent appears healthy.
    ///
    /// The checks are evaluated in priority order; the first match wins.
    #[must_use]
    pub fn check_stuck(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        if history.is_empty() {
            return None;
        }

        // Check in priority order: most severe first.
        if let Some(s) = self.check_excessive_retries(history) {
            return Some(s);
        }
        if let Some(s) = self.check_output_loop(history) {
            return Some(s);
        }
        if let Some(s) = self.check_gate_loop(history) {
            return Some(s);
        }
        if let Some(s) = self.check_compile_loop(history) {
            return Some(s);
        }
        if let Some(s) = self.check_empty_output(history) {
            return Some(s);
        }
        if let Some(s) = self.check_no_progress(history) {
            return Some(s);
        }

        None
    }

    /// Check all heuristics and return every stuck signal found.
    #[must_use]
    pub fn check_all(&self, history: &[ActivityEntry]) -> Vec<StuckSignal> {
        if history.is_empty() {
            return Vec::new();
        }

        let mut signals = Vec::new();
        if let Some(s) = self.check_excessive_retries(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_output_loop(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_gate_loop(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_compile_loop(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_empty_output(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_no_progress(history) {
            signals.push(s);
        }
        signals
    }

    /// Run the theta-frequency meta-cognition hook over recent activity.
    ///
    /// This wraps the stuck detector with a higher-level self-check that asks:
    /// "Am I stuck? Am I thrashing? Should I escalate?"
    #[must_use]
    pub fn meta_cognition(&self, history: &[ActivityEntry]) -> MetaCognitionAssessment {
        let stuck_signals = self.check_all(history);
        let primary_signal = self.check_stuck(history);
        let iterations_without_progress = trailing_no_progress_iterations(history);
        let repeated_output_count = trailing_output_repetition_count(history);
        let repeated_gate_failure_count = trailing_gate_failure_count(history);

        let action = classify_meta_cognition_action(
            primary_signal.as_ref(),
            iterations_without_progress,
            repeated_output_count,
            repeated_gate_failure_count,
            &self.thresholds,
        );

        let reason = meta_cognition_reason(
            action,
            primary_signal.as_ref(),
            iterations_without_progress,
            repeated_output_count,
            repeated_gate_failure_count,
        );

        MetaCognitionAssessment {
            frequency: OperatingFrequency::Theta,
            action,
            reason,
            primary_signal,
            stuck_signals,
            iterations_without_progress,
            repeated_output_count,
            repeated_gate_failure_count,
        }
    }

    // ---- Individual heuristics ----

    /// Detect consecutive identical output hashes.
    fn check_output_loop(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        let threshold = self.thresholds.output_loop_count;
        if history.len() < threshold {
            return None;
        }

        // Count consecutive identical hashes from the end.
        let last_hash = &history.last()?.output_hash;
        if last_hash.is_empty() {
            return None;
        }
        let consecutive = count_consecutive_from_end(history, |e| e.output_hash == *last_hash);

        if consecutive >= threshold {
            let duration_ms = if history.len() >= 2 {
                let first_idx = history.len() - consecutive;
                Some(history.last()?.timestamp_ms - history[first_idx].timestamp_ms)
            } else {
                None
            };

            Some(StuckSignal {
                kind: StuckKind::OutputLoop,
                confidence: confidence_from_count(consecutive, threshold),
                duration_ms,
                description: format!(
                    "{consecutive} consecutive identical outputs (hash: {})",
                    truncate_str(last_hash, 16)
                ),
            })
        } else {
            None
        }
    }

    /// Detect no file changes over a time period.
    #[allow(clippy::cast_precision_loss)]
    fn check_no_progress(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        if history.len() < 2 {
            return None;
        }

        // Find the last entry with file changes.
        let last_with_changes = history.iter().rev().find(|e| e.files_changed > 0);

        let first_ts = match last_with_changes {
            Some(entry) => entry.timestamp_ms,
            None => history.first()?.timestamp_ms,
        };

        let last_ts = history.last()?.timestamp_ms;
        let elapsed = last_ts - first_ts;

        if elapsed >= self.thresholds.no_progress_ms {
            Some(StuckSignal {
                kind: StuckKind::NoProgress,
                confidence: (elapsed as f64 / self.thresholds.no_progress_ms as f64).min(1.0),
                duration_ms: Some(elapsed),
                description: format!("no file changes for {:.0}s", elapsed as f64 / 1000.0),
            })
        } else {
            None
        }
    }

    /// Detect repeated identical gate failures.
    fn check_gate_loop(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        let threshold = self.thresholds.gate_loop_count;

        // Only look at entries with gate results.
        let gate_entries: Vec<&ActivityEntry> = history
            .iter()
            .rev()
            .filter(|e| e.gate_result.is_some())
            .collect();

        if gate_entries.len() < threshold {
            return None;
        }

        let last_result = gate_entries.first()?.gate_result.as_ref()?;
        // Only fire for failures.
        if !last_result.starts_with("fail") {
            return None;
        }

        let consecutive = gate_entries
            .iter()
            .take_while(|e| e.gate_result.as_deref() == Some(last_result))
            .count();

        if consecutive >= threshold {
            Some(StuckSignal {
                kind: StuckKind::GateLoop,
                confidence: confidence_from_count(consecutive, threshold),
                duration_ms: None,
                description: format!(
                    "{consecutive} consecutive identical gate failures: {last_result}"
                ),
            })
        } else {
            None
        }
    }

    /// Detect repeated identical compile errors (gate result starting with "fail:compile").
    fn check_compile_loop(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        let threshold = self.thresholds.compile_loop_count;

        let compile_failures: Vec<&ActivityEntry> = history
            .iter()
            .rev()
            .filter(|e| {
                e.gate_result
                    .as_deref()
                    .is_some_and(|r| r.starts_with("fail:compile"))
            })
            .collect();

        if compile_failures.len() < threshold {
            return None;
        }

        let last_result = compile_failures.first()?.gate_result.as_ref()?;
        let consecutive = compile_failures
            .iter()
            .take_while(|e| e.gate_result.as_deref() == Some(last_result))
            .count();

        if consecutive >= threshold {
            Some(StuckSignal {
                kind: StuckKind::CompileLoop,
                confidence: confidence_from_count(consecutive, threshold),
                duration_ms: None,
                description: format!("{consecutive} consecutive identical compile failures"),
            })
        } else {
            None
        }
    }

    /// Detect consecutive empty outputs (agent not producing work).
    fn check_empty_output(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        let threshold = self.thresholds.empty_output_count;
        if history.len() < threshold {
            return None;
        }

        let consecutive = count_consecutive_from_end(history, |e| {
            e.output_hash.is_empty() && e.files_changed == 0
        });

        if consecutive >= threshold {
            let duration_ms = if history.len() >= 2 {
                let first_idx = history.len() - consecutive;
                Some(history.last()?.timestamp_ms - history[first_idx].timestamp_ms)
            } else {
                None
            };

            Some(StuckSignal {
                kind: StuckKind::EmptyOutput,
                confidence: confidence_from_count(consecutive, threshold),
                duration_ms,
                description: format!("{consecutive} consecutive empty outputs"),
            })
        } else {
            None
        }
    }

    /// Detect excessive retries (too many iterations).
    fn check_excessive_retries(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        let max_iter = history.iter().map(|e| e.iteration).max()?;
        if max_iter >= self.thresholds.excessive_retry_count {
            let duration_ms = if history.len() >= 2 {
                Some(history.last()?.timestamp_ms - history.first()?.timestamp_ms)
            } else {
                None
            };

            Some(StuckSignal {
                kind: StuckKind::ExcessiveRetries,
                confidence: (f64::from(max_iter)
                    / f64::from(self.thresholds.excessive_retry_count))
                .min(1.0),
                duration_ms,
                description: format!(
                    "{max_iter} iterations (threshold: {})",
                    self.thresholds.excessive_retry_count
                ),
            })
        } else {
            None
        }
    }
}

/// Meta-cognition decision: keep going, adjust strategy, or escalate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaCognitionAction {
    /// No stuck pattern detected.
    Continue,
    /// Agent should change tactics before retrying.
    AdjustStrategy,
    /// Agent should escalate to a stronger model or broader context.
    Escalate,
}

impl MetaCognitionAction {
    /// Stable label for logging and signal tags.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::AdjustStrategy => "adjust_strategy",
            Self::Escalate => "escalate",
        }
    }
}

/// Theta-frequency assessment of the agent's current cognitive state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaCognitionAssessment {
    /// This hook always runs at theta frequency.
    pub frequency: OperatingFrequency,
    /// The recommended response.
    pub action: MetaCognitionAction,
    /// Human-readable explanation of the recommendation.
    pub reason: String,
    /// The primary stuck signal detected, if any.
    pub primary_signal: Option<StuckSignal>,
    /// All stuck signals detected in this pass.
    pub stuck_signals: Vec<StuckSignal>,
    /// Consecutive iterations with no file changes.
    pub iterations_without_progress: usize,
    /// Consecutive identical output hashes at the tail of the history.
    pub repeated_output_count: usize,
    /// Consecutive identical gate failures at the tail of the history.
    pub repeated_gate_failure_count: usize,
}

impl MetaCognitionAssessment {
    /// Convert the assessment into a structured signal when action is needed.
    #[must_use]
    pub fn to_signal(&self) -> Option<Signal> {
        match self.action {
            MetaCognitionAction::Continue => None,
            _ => Some(
                Signal::builder(Kind::Custom("roko.meta_cognition".into()))
                    .body(
                        Body::from_json(self)
                            .expect("meta-cognition assessment should serialize to JSON"),
                    )
                    .tag("frequency", "theta")
                    .tag("action", self.action.label())
                    .tag("reason", self.reason.as_str())
                    .build(),
            ),
        }
    }
}

/// A light wrapper around the stuck detector for theta-frequency reflection.
#[derive(Debug, Clone)]
pub struct MetaCognitionHook {
    detector: StuckDetector,
    no_progress_iterations_threshold: usize,
}

impl Default for MetaCognitionHook {
    fn default() -> Self {
        Self {
            detector: StuckDetector::default(),
            no_progress_iterations_threshold: 3,
        }
    }
}

impl MetaCognitionHook {
    /// Create a hook with the default stuck detector and no-progress threshold.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the number of consecutive no-progress iterations required to
    /// trigger a strategy adjustment.
    #[must_use]
    pub fn with_no_progress_iterations_threshold(mut self, threshold: usize) -> Self {
        assert!(threshold > 0, "no-progress threshold must be positive");
        self.no_progress_iterations_threshold = threshold;
        self
    }

    /// Borrow the underlying stuck detector.
    #[must_use]
    pub const fn detector(&self) -> &StuckDetector {
        &self.detector
    }

    /// The hook always runs at theta frequency.
    #[must_use]
    pub const fn frequency(&self) -> OperatingFrequency {
        OperatingFrequency::Theta
    }

    /// Reflect on recent activity and return a structured assessment.
    #[must_use]
    pub fn assess(&self, history: &[ActivityEntry]) -> MetaCognitionAssessment {
        self.detector.meta_cognition(history)
    }
}

fn classify_meta_cognition_action(
    primary_signal: Option<&StuckSignal>,
    iterations_without_progress: usize,
    repeated_output_count: usize,
    repeated_gate_failure_count: usize,
    thresholds: &StuckThresholds,
) -> MetaCognitionAction {
    if matches!(
        primary_signal.map(|signal| signal.kind),
        Some(StuckKind::GateLoop | StuckKind::CompileLoop | StuckKind::ExcessiveRetries)
    ) || repeated_gate_failure_count >= thresholds.gate_loop_count
    {
        return MetaCognitionAction::Escalate;
    }

    if iterations_without_progress >= 3
        || repeated_output_count >= thresholds.output_loop_count
        || matches!(
            primary_signal.map(|signal| signal.kind),
            Some(StuckKind::OutputLoop)
        )
        || matches!(
            primary_signal.map(|signal| signal.kind),
            Some(StuckKind::EmptyOutput)
        )
        || matches!(
            primary_signal.map(|signal| signal.kind),
            Some(StuckKind::NoProgress)
        )
    {
        return MetaCognitionAction::AdjustStrategy;
    }

    MetaCognitionAction::Continue
}

fn meta_cognition_reason(
    action: MetaCognitionAction,
    primary_signal: Option<&StuckSignal>,
    iterations_without_progress: usize,
    repeated_output_count: usize,
    repeated_gate_failure_count: usize,
) -> String {
    match action {
        MetaCognitionAction::Continue => {
            "no stuck pattern detected; continue current strategy".to_string()
        }
        MetaCognitionAction::AdjustStrategy => {
            if repeated_output_count > 0 {
                format!(
                    "{repeated_output_count} consecutive identical outputs and {iterations_without_progress} iterations without progress; adjust strategy"
                )
            } else if iterations_without_progress > 0 {
                format!(
                    "{iterations_without_progress} iterations without progress; adjust strategy"
                )
            } else if let Some(signal) = primary_signal {
                format!("{}; adjust strategy", signal.description)
            } else {
                "progress has stalled; adjust strategy".to_string()
            }
        }
        MetaCognitionAction::Escalate => {
            if repeated_gate_failure_count > 0 {
                format!(
                    "{repeated_gate_failure_count} repeated gate failures; escalate to a stronger model or broader context"
                )
            } else if let Some(signal) = primary_signal {
                format!(
                    "{}; escalate to a stronger model or broader context",
                    signal.description
                )
            } else {
                "gate failure pattern detected; escalate to a stronger model or broader context"
                    .to_string()
            }
        }
    }
}

// ---- Helpers ----------------------------------------------------------------

/// Count consecutive entries from the end matching a predicate.
fn count_consecutive_from_end<F>(history: &[ActivityEntry], pred: F) -> usize
where
    F: Fn(&ActivityEntry) -> bool,
{
    history.iter().rev().take_while(|e| pred(e)).count()
}

/// Count how many trailing iterations made no file changes.
fn trailing_no_progress_iterations(history: &[ActivityEntry]) -> usize {
    history
        .iter()
        .rev()
        .take_while(|entry| entry.files_changed == 0)
        .count()
}

/// Count how many trailing outputs repeated the same hash.
fn trailing_output_repetition_count(history: &[ActivityEntry]) -> usize {
    let Some(last_hash) = history.last().map(|entry| entry.output_hash.as_str()) else {
        return 0;
    };
    if last_hash.is_empty() {
        return 0;
    }
    count_consecutive_from_end(history, |entry| entry.output_hash == last_hash)
}

/// Count how many trailing gate failures repeated the same failure string.
fn trailing_gate_failure_count(history: &[ActivityEntry]) -> usize {
    let Some(last_result) = history
        .iter()
        .rev()
        .find_map(|entry| entry.gate_result.as_deref())
    else {
        return 0;
    };

    if !last_result.starts_with("fail") {
        return 0;
    }

    history
        .iter()
        .rev()
        .filter_map(|entry| entry.gate_result.as_deref())
        .take_while(|result| *result == last_result)
        .count()
}

/// Compute confidence based on how far past the threshold we are.
#[allow(clippy::cast_precision_loss)]
fn confidence_from_count(count: usize, threshold: usize) -> f64 {
    if threshold == 0 {
        return 1.0;
    }
    let ratio = count as f64 / threshold as f64;
    // Scale: at threshold = 0.7, at 2x threshold = 0.9, at 3x = 1.0
    0.2f64.mul_add(ratio, 0.5).min(1.0)
}

/// Truncate a string for display.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let mut t = s[..max].to_owned();
        t.push_str("...");
        t
    }
}

/// Helper to build a history where each entry has a specific hash pattern.
/// Useful in tests.
#[cfg(test)]
fn make_history(hashes: &[&str], base_ts: i64, interval_ms: i64) -> Vec<ActivityEntry> {
    hashes
        .iter()
        .enumerate()
        .map(|(i, h)| {
            ActivityEntry::new(
                base_ts + (i as i64) * interval_ms,
                *h,
                0,
                None,
                (i + 1) as u32,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detector() -> StuckDetector {
        StuckDetector::default()
    }

    fn hook() -> MetaCognitionHook {
        MetaCognitionHook::default()
    }

    // ---- Output loop ----

    #[test]
    fn output_loop_detected() {
        let history = make_history(&["abc", "abc", "abc", "abc"], 1000, 1000);
        let signal = detector().check_stuck(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().expect("signal").kind, StuckKind::OutputLoop);
    }

    #[test]
    fn output_loop_below_threshold() {
        let history = make_history(&["abc", "abc", "abc"], 1000, 1000);
        // Default threshold is 4; only 3 repeats shouldn't fire output loop.
        // However, ExcessiveRetries threshold is 6, so let's make sure iterations are low.
        let mut history = history;
        for e in &mut history {
            e.iteration = 1;
        }
        let signal = detector().check_stuck(&history);
        // Should not match output loop (3 < 4).
        let is_output_loop = signal
            .as_ref()
            .is_some_and(|s| s.kind == StuckKind::OutputLoop);
        assert!(!is_output_loop);
    }

    #[test]
    fn output_loop_mixed_hashes_no_fire() {
        let history = make_history(&["abc", "def", "ghi", "jkl"], 1000, 1000);
        let signal = detector().check_stuck(&history);
        // No stuck conditions (iterations are 1-4, below 6).
        assert!(signal.is_none(), "unexpected: {signal:?}");
    }

    // ---- No progress ----

    #[test]
    fn no_progress_detected() {
        // 6 minutes with no file changes.
        let mut history = make_history(&["a", "b", "c"], 0, 180_000);
        for e in &mut history {
            e.files_changed = 0;
            e.iteration = 1;
        }
        let signal = detector().check_stuck(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().expect("signal").kind, StuckKind::NoProgress);
    }

    #[test]
    fn no_progress_with_recent_changes() {
        let mut history = make_history(&["a", "b"], 0, 60_000);
        // Recent file changes -> no stuck.
        history.last_mut().expect("last").files_changed = 3;
        for e in &mut history {
            e.iteration = 1;
        }
        let signal = detector().check_stuck(&history);
        assert!(signal.is_none(), "unexpected: {signal:?}");
    }

    // ---- Gate loop ----

    #[test]
    fn gate_loop_detected() {
        let mut history = make_history(&["a", "b", "c"], 1000, 1000);
        for e in &mut history {
            e.gate_result = Some("fail:test:assertion".into());
            e.iteration = 1;
        }
        let signal = detector().check_stuck(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().expect("signal").kind, StuckKind::GateLoop);
    }

    #[test]
    fn gate_loop_passing_gates_no_fire() {
        let mut history = make_history(&["a", "b", "c"], 1000, 1000);
        for e in &mut history {
            e.gate_result = Some("pass".into());
            e.iteration = 1;
        }
        let signal = detector().check_stuck(&history);
        assert!(signal.is_none(), "unexpected: {signal:?}");
    }

    // ---- Compile loop ----

    #[test]
    fn compile_loop_detected() {
        let mut history = make_history(&["a", "b", "c"], 1000, 1000);
        for e in &mut history {
            e.gate_result = Some("fail:compile:E0308".into());
            e.iteration = 1;
        }
        let signal = detector().check_stuck(&history);
        assert!(signal.is_some());
        let kind = signal.as_ref().expect("signal").kind;
        // Both GateLoop and CompileLoop might match; GateLoop has higher priority
        // but CompileLoop is more specific. Either is acceptable.
        assert!(
            kind == StuckKind::GateLoop || kind == StuckKind::CompileLoop,
            "expected gate or compile loop, got {kind:?}"
        );
    }

    // ---- Empty output ----

    #[test]
    fn empty_output_detected() {
        let history = make_history(&["", "", ""], 1000, 1000);
        let mut history = history;
        for e in &mut history {
            e.iteration = 1;
        }
        let signal = detector().check_stuck(&history);
        assert!(signal.is_some());
        assert_eq!(
            signal.as_ref().expect("signal").kind,
            StuckKind::EmptyOutput
        );
    }

    // ---- Excessive retries ----

    #[test]
    fn excessive_retries_detected() {
        let mut entries = Vec::new();
        for i in 0..7 {
            entries.push(ActivityEntry::new(
                i64::from(i) * 1000,
                format!("hash{i}"),
                1,
                None,
                i + 1,
            ));
        }
        let signal = detector().check_stuck(&entries);
        assert!(signal.is_some());
        assert_eq!(
            signal.as_ref().expect("signal").kind,
            StuckKind::ExcessiveRetries
        );
    }

    #[test]
    fn below_retry_threshold_no_fire() {
        let mut entries = Vec::new();
        for i in 0..5 {
            entries.push(ActivityEntry::new(
                i64::from(i) * 1000,
                format!("hash{i}"),
                1,
                None,
                i + 1,
            ));
        }
        let signal = detector().check_stuck(&entries);
        assert!(signal.is_none(), "unexpected: {signal:?}");
    }

    // ---- Empty history ----

    #[test]
    fn empty_history_returns_none() {
        let signal = detector().check_stuck(&[]);
        assert!(signal.is_none());
    }

    // ---- Custom thresholds ----

    #[test]
    fn custom_thresholds() {
        let thresholds = StuckThresholds {
            output_loop_count: 2,
            no_progress_ms: 60_000,
            gate_loop_count: 2,
            compile_loop_count: 2,
            empty_output_count: 2,
            excessive_retry_count: 3,
        };
        let det = StuckDetector::with_thresholds(thresholds);
        // Two identical outputs should fire with threshold=2.
        let history = make_history(&["same", "same"], 1000, 1000);
        let signal = det.check_stuck(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().expect("signal").kind, StuckKind::OutputLoop);
    }

    // ---- check_all ----

    #[test]
    fn check_all_returns_multiple() {
        // Build a history with both empty outputs and excessive retries.
        let mut entries = Vec::new();
        for i in 0..7 {
            entries.push(ActivityEntry::new(i64::from(i) * 1000, "", 0, None, i + 1));
        }
        let signals = detector().check_all(&entries);
        assert!(signals.len() >= 2, "expected >=2, got {}", signals.len());
        let kinds: Vec<StuckKind> = signals.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&StuckKind::ExcessiveRetries));
        assert!(kinds.contains(&StuckKind::EmptyOutput));
    }

    // ---- Confidence ----

    #[test]
    fn confidence_increases_past_threshold() {
        let thresholds = StuckThresholds {
            output_loop_count: 2,
            excessive_retry_count: 100,
            ..StuckThresholds::default()
        };
        let det = StuckDetector::with_thresholds(thresholds);

        let h2 = make_history(&["same", "same"], 1000, 1000);
        let h4 = make_history(&["same", "same", "same", "same"], 1000, 1000);

        let s2 = det.check_stuck(&h2).expect("should fire at 2");
        let s4 = det.check_stuck(&h4).expect("should fire at 4");
        assert!(s4.confidence >= s2.confidence);
    }

    // ---- Serde ----

    #[test]
    fn stuck_signal_serde_roundtrip() {
        let signal = StuckSignal {
            kind: StuckKind::OutputLoop,
            confidence: 0.85,
            duration_ms: Some(5000),
            description: "test".into(),
        };
        let json = serde_json::to_string(&signal).expect("serialize");
        let decoded: StuckSignal = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.kind, StuckKind::OutputLoop);
    }

    #[test]
    fn activity_entry_serde_roundtrip() {
        let entry = ActivityEntry::new(1000, "abc", 2, Some("pass".into()), 1);
        let json = serde_json::to_string(&entry).expect("serialize");
        let decoded: ActivityEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, entry);
    }

    // ---- Duration tracking ----

    #[test]
    fn output_loop_reports_duration() {
        let history = make_history(&["abc", "abc", "abc", "abc"], 1000, 5000);
        let signal = detector().check_stuck(&history);
        assert!(signal.is_some());
        let s = signal.expect("signal");
        assert_eq!(s.kind, StuckKind::OutputLoop);
        assert!(s.duration_ms.is_some());
        assert_eq!(s.duration_ms, Some(15000));
    }

    #[test]
    fn meta_cognition_is_theta_frequency() {
        assert_eq!(hook().frequency(), OperatingFrequency::Theta);
    }

    #[test]
    fn meta_cognition_adjusts_for_repeated_outputs() {
        let history = vec![
            ActivityEntry::new(1000, "same", 0, Some("pass".into()), 1),
            ActivityEntry::new(2000, "same", 0, Some("pass".into()), 2),
            ActivityEntry::new(3000, "same", 0, Some("pass".into()), 3),
            ActivityEntry::new(4000, "same", 0, Some("pass".into()), 4),
        ];

        let assessment = hook().assess(&history);
        assert_eq!(assessment.action, MetaCognitionAction::AdjustStrategy);
        assert_eq!(assessment.repeated_output_count, 4);
        assert!(assessment.to_signal().is_some());
    }

    #[test]
    fn meta_cognition_escalates_for_gate_failure_patterns() {
        let mut history = make_history(&["a", "b", "c"], 1000, 1000);
        for entry in &mut history {
            entry.gate_result = Some("fail:test:assertion".into());
        }

        let assessment = hook().assess(&history);
        assert_eq!(assessment.action, MetaCognitionAction::Escalate);
        assert_eq!(assessment.repeated_gate_failure_count, 3);
        let signal = assessment.to_signal().expect("signal");
        assert_eq!(signal.tag("frequency"), Some("theta"));
        assert_eq!(signal.tag("action"), Some("escalate"));
    }

    #[test]
    fn meta_cognition_continues_when_healthy() {
        let history = vec![
            ActivityEntry::new(1000, "a", 1, Some("pass".into()), 1),
            ActivityEntry::new(2000, "b", 1, Some("pass".into()), 2),
        ];

        let assessment = hook().assess(&history);
        assert_eq!(assessment.action, MetaCognitionAction::Continue);
        assert!(assessment.to_signal().is_none());
    }
}
