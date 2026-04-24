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

use roko_core::{Body, Engram, Kind, OperatingFrequency};
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
    /// 3+ consecutive REVISE verdicts.
    ReviewLoop,
    /// 6+ iterations cycling through the same phases.
    IterationLoop,
    /// No output for 180 seconds.
    SilenceTimeout,
    /// 3+ consecutive compile failures (broader than `CompileLoop` which checks identical errors).
    CompileFailThreshold,
    /// Single task blocking for 300+ seconds.
    TaskStall,
    /// Prompt exceeds 80% of context window.
    ContextPressure,
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
    /// Free-form activity description (e.g. "compile", "revise", "test").
    #[serde(default)]
    pub activity: String,
    /// Phase label for cycle detection (e.g. "plan", "code", "review").
    #[serde(default)]
    pub phase: String,
    /// Task identifier, for per-task stall detection.
    #[serde(default)]
    pub task_id: String,
    /// Number of prompt tokens used in this turn (for context-pressure detection).
    #[serde(default)]
    pub tokens_used: Option<u64>,
    /// Total context window size for the model (for context-pressure detection).
    #[serde(default)]
    pub context_window: Option<u64>,
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
            activity: String::new(),
            phase: String::new(),
            task_id: String::new(),
            tokens_used: None,
            context_window: None,
        }
    }

    /// Set the activity description.
    #[must_use]
    pub fn with_activity(mut self, activity: impl Into<String>) -> Self {
        self.activity = activity.into();
        self
    }

    /// Set the phase label.
    #[must_use]
    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = phase.into();
        self
    }

    /// Set the task identifier.
    #[must_use]
    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = task_id.into();
        self
    }

    /// Set token usage and context window for context-pressure detection.
    #[must_use]
    pub fn with_token_usage(mut self, tokens_used: u64, context_window: u64) -> Self {
        self.tokens_used = Some(tokens_used);
        self.context_window = Some(context_window);
        self
    }
}

// ---- StuckDetector config ---------------------------------------------------

/// Configurable thresholds for stuck detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Consecutive REVISE verdicts before firing `ReviewLoop`.
    pub review_loop_threshold: usize,
    /// Iterations cycling through same phases before firing `IterationLoop`.
    pub iteration_loop_threshold: usize,
    /// Milliseconds of silence before firing `SilenceTimeout`.
    pub silence_timeout_ms: u64,
    /// Consecutive compile failures before firing `CompileFailThreshold`.
    pub compile_fail_threshold: usize,
    /// Milliseconds a single task can block before firing `TaskStall`.
    pub task_stall_ms: u64,
    /// Fraction of context window usage that triggers `ContextPressure` (0.0 to 1.0).
    pub context_pressure_pct: f64,
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
            review_loop_threshold: 3,
            iteration_loop_threshold: 6,
            silence_timeout_ms: 180_000, // 3 minutes
            compile_fail_threshold: 3,
            task_stall_ms: 300_000, // 5 minutes
            context_pressure_pct: 0.80,
        }
    }
}

impl StuckThresholds {
    /// Create thresholds using the docs-facing names for the built-in heuristics.
    ///
    /// New thresholds added after the original six use their defaults. Use the
    /// struct literal or builder-style setters to customise them.
    #[must_use]
    pub const fn new(
        output_loop: usize,
        no_progress_ms: i64,
        gate_loop: usize,
        compile_loop: usize,
        empty_output: usize,
        excessive_retry: u32,
    ) -> Self {
        Self {
            output_loop_count: output_loop,
            no_progress_ms,
            gate_loop_count: gate_loop,
            compile_loop_count: compile_loop,
            empty_output_count: empty_output,
            excessive_retry_count: excessive_retry,
            review_loop_threshold: 3,
            iteration_loop_threshold: 6,
            silence_timeout_ms: 180_000,
            compile_fail_threshold: 3,
            task_stall_ms: 300_000,
            context_pressure_pct: 0.80,
        }
    }

    /// Docs-compatible alias for the output-loop threshold.
    #[must_use]
    pub const fn output_loop(&self) -> usize {
        self.output_loop_count
    }

    /// Docs-compatible alias for the no-progress threshold.
    #[must_use]
    pub const fn no_progress(&self) -> i64 {
        self.no_progress_ms
    }

    /// Docs-compatible alias for the gate-loop threshold.
    #[must_use]
    pub const fn gate_loop(&self) -> usize {
        self.gate_loop_count
    }

    /// Docs-compatible alias for the compile-loop threshold.
    #[must_use]
    pub const fn compile_loop(&self) -> usize {
        self.compile_loop_count
    }

    /// Docs-compatible alias for the empty-output threshold.
    #[must_use]
    pub const fn empty_output(&self) -> usize {
        self.empty_output_count
    }

    /// Docs-compatible alias for the excessive-retry threshold.
    #[must_use]
    pub const fn excessive_retry(&self) -> u32 {
        self.excessive_retry_count
    }

    /// Docs-compatible alias for the review-loop threshold.
    #[must_use]
    pub const fn review_loop(&self) -> usize {
        self.review_loop_threshold
    }

    /// Docs-compatible alias for the iteration-loop threshold.
    #[must_use]
    pub const fn iteration_loop(&self) -> usize {
        self.iteration_loop_threshold
    }

    /// Docs-compatible alias for the silence-timeout threshold (ms).
    #[must_use]
    pub const fn silence_timeout(&self) -> u64 {
        self.silence_timeout_ms
    }

    /// Docs-compatible alias for the compile-fail threshold.
    #[must_use]
    pub const fn compile_fail(&self) -> usize {
        self.compile_fail_threshold
    }

    /// Docs-compatible alias for the task-stall threshold (ms).
    #[must_use]
    pub const fn task_stall(&self) -> u64 {
        self.task_stall_ms
    }

    /// Docs-compatible alias for the context-pressure threshold (0.0-1.0).
    #[must_use]
    pub const fn context_pressure(&self) -> f64 {
        self.context_pressure_pct
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
        if let Some(s) = self.check_review_loop(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_iteration_loop(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_silence_timeout(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_compile_fail_threshold(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_task_stall(history) {
            signals.push(s);
        }
        if let Some(s) = self.check_context_pressure(history) {
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
            stuck_kinds: stuck_signals.iter().map(|signal| signal.kind).collect(),
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

    // ---- Extended heuristics ----

    /// Detect consecutive REVISE verdicts (review loop).
    ///
    /// Counts trailing entries whose `activity` field contains "revise" or
    /// "rejected" (case-insensitive).
    pub fn check_review_loop(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        let threshold = self.thresholds.review_loop_threshold;
        if history.len() < threshold {
            return None;
        }

        let consecutive = count_consecutive_from_end(history, |e| {
            let lower = e.activity.to_ascii_lowercase();
            lower.contains("revise") || lower.contains("rejected")
        });

        if consecutive >= threshold {
            Some(StuckSignal {
                kind: StuckKind::ReviewLoop,
                confidence: confidence_from_count(consecutive, threshold),
                duration_ms: None,
                description: format!("{consecutive} consecutive REVISE verdicts"),
            })
        } else {
            None
        }
    }

    /// Detect phase cycling (iteration loop).
    ///
    /// Looks at the `phase` field to find the shortest repeating subsequence
    /// in the trailing entries. Fires when the same phase sequence repeats
    /// `iteration_loop_threshold` or more times.
    pub fn check_iteration_loop(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        let threshold = self.thresholds.iteration_loop_threshold;
        if history.len() < threshold {
            return None;
        }

        // Collect non-empty phases from tail.
        let phases: Vec<&str> = history
            .iter()
            .rev()
            .filter(|e| !e.phase.is_empty())
            .map(|e| e.phase.as_str())
            .collect();

        if phases.len() < threshold {
            return None;
        }

        // Try cycle lengths 1..=phases.len()/2 and see if the pattern repeats.
        for cycle_len in 1..=(phases.len() / 2) {
            let pattern = &phases[..cycle_len];
            let repeats = phases
                .chunks(cycle_len)
                .take_while(|chunk| *chunk == pattern)
                .count();

            if repeats >= threshold {
                return Some(StuckSignal {
                    kind: StuckKind::IterationLoop,
                    confidence: confidence_from_count(repeats, threshold),
                    duration_ms: None,
                    description: format!(
                        "{repeats} repetitions of phase cycle {:?}",
                        pattern.iter().rev().copied().collect::<Vec<_>>()
                    ),
                });
            }
        }

        None
    }

    /// Detect silence (no output for an extended period).
    ///
    /// Looks at the gap between the last entry's timestamp and the
    /// second-to-last entry (or, if only one entry exists, returns `None`).
    #[allow(clippy::cast_precision_loss)]
    pub fn check_silence_timeout(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        if history.len() < 2 {
            return None;
        }

        let last = history.last()?;
        let prev = &history[history.len() - 2];
        let gap = last.timestamp_ms.saturating_sub(prev.timestamp_ms);

        if gap < 0 {
            return None;
        }

        #[allow(clippy::cast_sign_loss)]
        let gap_u64 = gap as u64;

        if gap_u64 >= self.thresholds.silence_timeout_ms {
            Some(StuckSignal {
                kind: StuckKind::SilenceTimeout,
                confidence: (gap_u64 as f64 / self.thresholds.silence_timeout_ms as f64).min(1.0),
                duration_ms: Some(gap),
                description: format!("no output for {:.0}s", gap as f64 / 1000.0),
            })
        } else {
            None
        }
    }

    /// Detect consecutive compile failures regardless of whether the error is
    /// identical (broader than `check_compile_loop`).
    ///
    /// Counts trailing entries whose `activity` contains "compile" and whose
    /// `gate_result` starts with "fail", or whose gate result alone indicates
    /// a compile failure.
    pub fn check_compile_fail_threshold(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        let threshold = self.thresholds.compile_fail_threshold;
        if history.len() < threshold {
            return None;
        }

        let consecutive = count_consecutive_from_end(history, |e| {
            let activity_is_compile = e.activity.to_ascii_lowercase().contains("compile");
            let gate_is_compile_fail = e
                .gate_result
                .as_deref()
                .is_some_and(|r| r.starts_with("fail:compile") || r.starts_with("fail:build"));
            // Either the activity says compile and gate says fail, or the gate
            // alone indicates a compile failure.
            (activity_is_compile
                && e.gate_result
                    .as_deref()
                    .is_some_and(|r| r.starts_with("fail")))
                || gate_is_compile_fail
        });

        if consecutive >= threshold {
            Some(StuckSignal {
                kind: StuckKind::CompileFailThreshold,
                confidence: confidence_from_count(consecutive, threshold),
                duration_ms: None,
                description: format!("{consecutive} consecutive compile failures"),
            })
        } else {
            None
        }
    }

    /// Detect a single task blocking for too long.
    ///
    /// Groups entries by `task_id` and checks if the most recent task has been
    /// running longer than `task_stall_ms`.
    #[allow(clippy::cast_precision_loss)]
    pub fn check_task_stall(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        if history.len() < 2 {
            return None;
        }

        let last = history.last()?;
        if last.task_id.is_empty() {
            return None;
        }

        // Find the first entry with the same task_id.
        let first_for_task = history.iter().find(|e| e.task_id == last.task_id)?;

        let elapsed = last
            .timestamp_ms
            .saturating_sub(first_for_task.timestamp_ms);

        #[allow(clippy::cast_sign_loss)]
        let elapsed_u64 = elapsed as u64;

        if elapsed_u64 >= self.thresholds.task_stall_ms {
            Some(StuckSignal {
                kind: StuckKind::TaskStall,
                confidence: (elapsed_u64 as f64 / self.thresholds.task_stall_ms as f64).min(1.0),
                duration_ms: Some(elapsed),
                description: format!(
                    "task {} stalled for {:.0}s",
                    truncate_str(&last.task_id, 16),
                    elapsed as f64 / 1000.0
                ),
            })
        } else {
            None
        }
    }

    /// Detect context-window pressure.
    ///
    /// Fires when any entry's `tokens_used / context_window` exceeds
    /// `context_pressure_pct`.
    #[allow(clippy::cast_precision_loss)]
    pub fn check_context_pressure(&self, history: &[ActivityEntry]) -> Option<StuckSignal> {
        // Check the most recent entry with token data.
        let entry = history
            .iter()
            .rev()
            .find(|e| e.tokens_used.is_some() && e.context_window.is_some())?;

        let used = entry.tokens_used? as f64;
        let window = entry.context_window? as f64;

        if window <= 0.0 {
            return None;
        }

        let ratio = used / window;

        if ratio >= self.thresholds.context_pressure_pct {
            Some(StuckSignal {
                kind: StuckKind::ContextPressure,
                confidence: ratio.min(1.0),
                duration_ms: None,
                description: format!(
                    "context pressure at {:.0}% ({}/{} tokens)",
                    ratio * 100.0,
                    entry.tokens_used.unwrap_or(0),
                    entry.context_window.unwrap_or(0)
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

    /// Docs-compatible alias for [`Self::label`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.label()
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
    /// The categories of stuck patterns seen in this assessment.
    pub stuck_kinds: Vec<StuckKind>,
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
    ///
    /// # Panics
    ///
    /// Panics if the assessment cannot be serialized into JSON for the signal
    /// body payload.
    #[must_use]
    pub fn to_signal(&self) -> Option<Engram> {
        match self.action {
            MetaCognitionAction::Continue => None,
            _ => Some(
                Engram::builder(Kind::Custom("roko.meta_cognition".into()))
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

    /// Docs-compatible alias for [`Self::to_signal`].
    ///
    /// # Panics
    ///
    /// Panics if the assessment cannot be serialized into JSON for the signal
    /// body payload.
    #[must_use]
    pub fn to_engram(&self) -> Option<Engram> {
        self.to_signal().map(|_| {
            Engram::builder(Kind::Custom("conductor.meta_cognition".into()))
                .body(
                    Body::from_json(self)
                        .expect("meta-cognition assessment should serialize to JSON"),
                )
                .tag("frequency", "theta")
                .tag("action", self.action.as_str())
                .tag("reason", self.reason.as_str())
                .build()
        })
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
    ///
    /// # Panics
    ///
    /// Panics if `threshold` is zero.
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

// ---- CooldownFilter ---------------------------------------------------------

/// Debounce filter that prevents the same [`StuckKind`] from firing too
/// frequently.
///
/// Tracks the timestamp (in milliseconds) of the last intervention for each
/// kind and suppresses re-fires within a configurable cooldown window
/// (default: 120 000 ms / 2 minutes).
#[derive(Debug, Clone)]
pub struct CooldownFilter {
    /// Minimum interval between fires for the same kind, in milliseconds.
    cooldown_ms: u64,
    /// Last fire timestamp per kind.
    last_fire: std::collections::HashMap<StuckKind, i64>,
}

impl Default for CooldownFilter {
    fn default() -> Self {
        Self {
            cooldown_ms: 120_000,
            last_fire: std::collections::HashMap::new(),
        }
    }
}

impl CooldownFilter {
    /// Create a filter with the default 120-second cooldown.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a filter with a custom cooldown period.
    #[must_use]
    pub fn with_cooldown_ms(cooldown_ms: u64) -> Self {
        Self {
            cooldown_ms,
            last_fire: std::collections::HashMap::new(),
        }
    }

    /// Returns `true` if the given kind is allowed to fire at `now_ms`.
    ///
    /// A kind is allowed if it has never fired, or if the last fire was
    /// at least `cooldown_ms` ago.
    #[must_use]
    pub fn should_fire(&self, kind: &StuckKind, now_ms: i64) -> bool {
        match self.last_fire.get(kind) {
            None => true,
            Some(&last) => {
                let elapsed = now_ms.saturating_sub(last);
                #[allow(clippy::cast_sign_loss)]
                let elapsed_u64 = elapsed as u64;
                elapsed_u64 >= self.cooldown_ms
            }
        }
    }

    /// Record that the given kind fired at `now_ms`.
    pub fn record_fire(&mut self, kind: StuckKind, now_ms: i64) {
        self.last_fire.insert(kind, now_ms);
    }

    /// Convenience: check and record in one call. Returns `true` if the fire
    /// was allowed (and records it); `false` if suppressed.
    pub fn try_fire(&mut self, kind: StuckKind, now_ms: i64) -> bool {
        if self.should_fire(&kind, now_ms) {
            self.record_fire(kind, now_ms);
            true
        } else {
            false
        }
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
        Some(
            StuckKind::GateLoop
                | StuckKind::CompileLoop
                | StuckKind::ExcessiveRetries
                | StuckKind::CompileFailThreshold
                | StuckKind::ReviewLoop
        )
    ) || repeated_gate_failure_count >= thresholds.gate_loop_count
    {
        return MetaCognitionAction::Escalate;
    }

    if iterations_without_progress >= 3
        || repeated_output_count >= thresholds.output_loop_count
        || matches!(
            primary_signal.map(|signal| signal.kind),
            Some(
                StuckKind::OutputLoop
                    | StuckKind::EmptyOutput
                    | StuckKind::NoProgress
                    | StuckKind::IterationLoop
                    | StuckKind::SilenceTimeout
                    | StuckKind::TaskStall
                    | StuckKind::ContextPressure
            )
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
            ..StuckThresholds::default()
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

    #[test]
    fn docs_facing_aliases_work() {
        let thresholds = StuckThresholds::new(4, 300_000, 3, 3, 3, 6);
        assert_eq!(thresholds.output_loop(), 4);
        assert_eq!(thresholds.no_progress(), 300_000);
        assert_eq!(thresholds.gate_loop(), 3);
        assert_eq!(thresholds.compile_loop(), 3);
        assert_eq!(thresholds.empty_output(), 3);
        assert_eq!(thresholds.excessive_retry(), 6);
        assert_eq!(MetaCognitionAction::Escalate.as_str(), "escalate");

        let mut history = make_history(&["same", "same", "same", "same"], 1000, 1000);
        for entry in &mut history {
            entry.gate_result = Some("fail:test:assertion".into());
        }

        let assessment = hook().assess(&history);
        assert!(!assessment.stuck_kinds.is_empty());
        let docs_signal = assessment.to_engram().expect("docs-facing signal");
        let runtime_signal = assessment.to_signal().expect("runtime signal");
        assert_eq!(docs_signal.tag("action"), runtime_signal.tag("action"));
        assert_eq!(
            docs_signal.kind,
            Kind::Custom("conductor.meta_cognition".into())
        );
    }

    // ---- ReviewLoop ----

    #[test]
    fn review_loop_detected() {
        let history: Vec<ActivityEntry> = (0..3)
            .map(|i| {
                ActivityEntry::new(i64::from(i) * 1000, format!("h{i}"), 0, None, 1)
                    .with_activity("revise")
            })
            .collect();
        let signal = detector().check_review_loop(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().unwrap().kind, StuckKind::ReviewLoop);
    }

    #[test]
    fn review_loop_case_insensitive() {
        let history: Vec<ActivityEntry> = vec![
            ActivityEntry::new(1000, "h1", 0, None, 1).with_activity("REVISE"),
            ActivityEntry::new(2000, "h2", 0, None, 2).with_activity("Rejected"),
            ActivityEntry::new(3000, "h3", 0, None, 3).with_activity("revise"),
        ];
        let signal = detector().check_review_loop(&history);
        assert!(signal.is_some());
    }

    #[test]
    fn review_loop_below_threshold() {
        let history: Vec<ActivityEntry> = (0..2)
            .map(|i| {
                ActivityEntry::new(i64::from(i) * 1000, format!("h{i}"), 0, None, 1)
                    .with_activity("revise")
            })
            .collect();
        let signal = detector().check_review_loop(&history);
        assert!(signal.is_none());
    }

    // ---- IterationLoop ----

    #[test]
    fn iteration_loop_detected() {
        // Phase sequence: [plan, code, review] repeated 6+ times (reversed because
        // we iterate in reverse).
        let mut history = Vec::new();
        let phases = ["review", "code", "plan"];
        for i in 0..(6 * 3) {
            let phase = phases[i % 3];
            history.push(
                ActivityEntry::new(i as i64 * 1000, format!("h{i}"), 0, None, 1).with_phase(phase),
            );
        }
        let signal = detector().check_iteration_loop(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().unwrap().kind, StuckKind::IterationLoop);
    }

    #[test]
    fn iteration_loop_below_threshold() {
        let mut history = Vec::new();
        let phases = ["plan", "code", "review"];
        for i in 0..(2 * 3) {
            let phase = phases[i % 3];
            history.push(
                ActivityEntry::new(i as i64 * 1000, format!("h{i}"), 0, None, 1).with_phase(phase),
            );
        }
        let signal = detector().check_iteration_loop(&history);
        assert!(signal.is_none());
    }

    // ---- SilenceTimeout ----

    #[test]
    fn silence_timeout_detected() {
        let history = vec![
            ActivityEntry::new(0, "a", 0, None, 1),
            ActivityEntry::new(200_000, "b", 0, None, 2), // 200s gap > 180s threshold
        ];
        let signal = detector().check_silence_timeout(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().unwrap().kind, StuckKind::SilenceTimeout);
        assert_eq!(signal.as_ref().unwrap().duration_ms, Some(200_000));
    }

    #[test]
    fn silence_timeout_below_threshold() {
        let history = vec![
            ActivityEntry::new(0, "a", 0, None, 1),
            ActivityEntry::new(60_000, "b", 0, None, 2), // 60s < 180s
        ];
        let signal = detector().check_silence_timeout(&history);
        assert!(signal.is_none());
    }

    #[test]
    fn silence_timeout_single_entry() {
        let history = vec![ActivityEntry::new(0, "a", 0, None, 1)];
        let signal = detector().check_silence_timeout(&history);
        assert!(signal.is_none());
    }

    // ---- CompileFailThreshold ----

    #[test]
    fn compile_fail_threshold_detected() {
        let history: Vec<ActivityEntry> = (0..3)
            .map(|i| {
                ActivityEntry::new(
                    i64::from(i) * 1000,
                    format!("h{i}"),
                    0,
                    Some(format!("fail:compile:E{i}")), // different errors each time
                    1,
                )
                .with_activity("compile")
            })
            .collect();
        let signal = detector().check_compile_fail_threshold(&history);
        assert!(signal.is_some());
        assert_eq!(
            signal.as_ref().unwrap().kind,
            StuckKind::CompileFailThreshold
        );
    }

    #[test]
    fn compile_fail_threshold_gate_only() {
        // Gate result alone (without activity field) should also trigger.
        let history: Vec<ActivityEntry> = (0..3)
            .map(|i| {
                ActivityEntry::new(
                    i64::from(i) * 1000,
                    format!("h{i}"),
                    0,
                    Some(format!("fail:compile:E{i}")),
                    1,
                )
            })
            .collect();
        let signal = detector().check_compile_fail_threshold(&history);
        assert!(signal.is_some());
    }

    #[test]
    fn compile_fail_threshold_below() {
        let history: Vec<ActivityEntry> = (0..2)
            .map(|i| {
                ActivityEntry::new(
                    i64::from(i) * 1000,
                    format!("h{i}"),
                    0,
                    Some(format!("fail:compile:E{i}")),
                    1,
                )
                .with_activity("compile")
            })
            .collect();
        let signal = detector().check_compile_fail_threshold(&history);
        assert!(signal.is_none());
    }

    // ---- TaskStall ----

    #[test]
    fn task_stall_detected() {
        let history = vec![
            ActivityEntry::new(0, "a", 0, None, 1).with_task_id("task-1"),
            ActivityEntry::new(100_000, "b", 0, None, 2).with_task_id("task-1"),
            ActivityEntry::new(350_000, "c", 0, None, 3).with_task_id("task-1"), // 350s > 300s
        ];
        let signal = detector().check_task_stall(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().unwrap().kind, StuckKind::TaskStall);
    }

    #[test]
    fn task_stall_below_threshold() {
        let history = vec![
            ActivityEntry::new(0, "a", 0, None, 1).with_task_id("task-1"),
            ActivityEntry::new(200_000, "b", 0, None, 2).with_task_id("task-1"),
        ];
        let signal = detector().check_task_stall(&history);
        assert!(signal.is_none());
    }

    #[test]
    fn task_stall_no_task_id() {
        let history = vec![
            ActivityEntry::new(0, "a", 0, None, 1),
            ActivityEntry::new(400_000, "b", 0, None, 2),
        ];
        let signal = detector().check_task_stall(&history);
        assert!(signal.is_none());
    }

    // ---- ContextPressure ----

    #[test]
    fn context_pressure_detected() {
        let history =
            vec![ActivityEntry::new(1000, "a", 1, None, 1).with_token_usage(85_000, 100_000)];
        let signal = detector().check_context_pressure(&history);
        assert!(signal.is_some());
        assert_eq!(signal.as_ref().unwrap().kind, StuckKind::ContextPressure);
        assert!(signal.as_ref().unwrap().confidence >= 0.85);
    }

    #[test]
    fn context_pressure_below_threshold() {
        let history =
            vec![ActivityEntry::new(1000, "a", 1, None, 1).with_token_usage(50_000, 100_000)];
        let signal = detector().check_context_pressure(&history);
        assert!(signal.is_none());
    }

    #[test]
    fn context_pressure_no_token_data() {
        let history = vec![ActivityEntry::new(1000, "a", 1, None, 1)];
        let signal = detector().check_context_pressure(&history);
        assert!(signal.is_none());
    }

    // ---- CooldownFilter ----

    #[test]
    fn cooldown_filter_allows_first_fire() {
        let filter = CooldownFilter::new();
        assert!(filter.should_fire(&StuckKind::OutputLoop, 1000));
    }

    #[test]
    fn cooldown_filter_suppresses_rapid_refire() {
        let mut filter = CooldownFilter::new();
        filter.record_fire(StuckKind::OutputLoop, 1000);
        // 10s later -- still within 120s cooldown.
        assert!(!filter.should_fire(&StuckKind::OutputLoop, 11_000));
    }

    #[test]
    fn cooldown_filter_allows_after_cooldown() {
        let mut filter = CooldownFilter::new();
        filter.record_fire(StuckKind::OutputLoop, 1000);
        // 121s later -- past 120s cooldown.
        assert!(filter.should_fire(&StuckKind::OutputLoop, 122_000));
    }

    #[test]
    fn cooldown_filter_independent_per_kind() {
        let mut filter = CooldownFilter::new();
        filter.record_fire(StuckKind::OutputLoop, 1000);
        // Different kind should still be allowed.
        assert!(filter.should_fire(&StuckKind::GateLoop, 2000));
    }

    #[test]
    fn cooldown_filter_try_fire() {
        let mut filter = CooldownFilter::with_cooldown_ms(5000);
        assert!(filter.try_fire(StuckKind::ReviewLoop, 1000));
        assert!(!filter.try_fire(StuckKind::ReviewLoop, 3000)); // 2s < 5s
        assert!(filter.try_fire(StuckKind::ReviewLoop, 7000)); // 6s > 5s
    }

    // ---- check_all includes new detectors ----

    #[test]
    fn check_all_includes_review_loop() {
        let history: Vec<ActivityEntry> = (0..4)
            .map(|i| {
                ActivityEntry::new(i64::from(i) * 1000, format!("h{i}"), 0, None, 1)
                    .with_activity("revise")
            })
            .collect();
        let signals = detector().check_all(&history);
        let kinds: Vec<StuckKind> = signals.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&StuckKind::ReviewLoop));
    }

    #[test]
    fn check_all_includes_context_pressure() {
        let history = vec![
            ActivityEntry::new(1000, "a", 1, None, 1).with_token_usage(90_000, 100_000),
            ActivityEntry::new(2000, "b", 1, None, 2).with_token_usage(95_000, 100_000),
        ];
        let signals = detector().check_all(&history);
        let kinds: Vec<StuckKind> = signals.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&StuckKind::ContextPressure));
    }

    // ---- Serde for new ActivityEntry fields ----

    #[test]
    fn activity_entry_serde_with_new_fields() {
        let entry = ActivityEntry::new(1000, "abc", 2, Some("pass".into()), 1)
            .with_activity("compile")
            .with_phase("code")
            .with_task_id("task-42")
            .with_token_usage(50_000, 100_000);
        let json = serde_json::to_string(&entry).expect("serialize");
        let decoded: ActivityEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.activity, "compile");
        assert_eq!(decoded.phase, "code");
        assert_eq!(decoded.task_id, "task-42");
        assert_eq!(decoded.tokens_used, Some(50_000));
        assert_eq!(decoded.context_window, Some(100_000));
    }

    #[test]
    fn activity_entry_serde_backward_compat() {
        // Old JSON without new fields should deserialize with defaults.
        let json = r#"{"timestamp_ms":1000,"output_hash":"abc","files_changed":2,"gate_result":"pass","iteration":1}"#;
        let decoded: ActivityEntry = serde_json::from_str(json).expect("deserialize");
        assert_eq!(decoded.activity, "");
        assert_eq!(decoded.phase, "");
        assert_eq!(decoded.task_id, "");
        assert!(decoded.tokens_used.is_none());
        assert!(decoded.context_window.is_none());
    }

    // ---- New StuckKind serde ----

    #[test]
    fn new_stuck_kind_serde_roundtrip() {
        let kinds = [
            StuckKind::ReviewLoop,
            StuckKind::IterationLoop,
            StuckKind::SilenceTimeout,
            StuckKind::CompileFailThreshold,
            StuckKind::TaskStall,
            StuckKind::ContextPressure,
        ];
        for kind in &kinds {
            let signal = StuckSignal {
                kind: *kind,
                confidence: 0.9,
                duration_ms: None,
                description: "test".into(),
            };
            let json = serde_json::to_string(&signal).expect("serialize");
            let decoded: StuckSignal = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(decoded.kind, *kind);
        }
    }
}
