//! Per-plan mutable state tracked by the executor.
//!
//! Each plan gets a [`PlanState`] when it enters the executor via
//! [`add_plan`](super::ParallelExecutor::add_plan). The state machine
//! ([`super::state_machine`]) reads and updates `PlanState` as the plan
//! progresses through phases.

use roko_core::{PlanPhase, TestCount, Verdict};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Mutable per-plan state held by the executor.
///
/// Contains everything the executor needs to make scheduling decisions for
/// one plan: current phase, assigned agents, gate verdicts, iteration
/// count, and error tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanState {
    /// Stable plan identifier (matches [`PlanInfo::base`](crate::PlanInfo)).
    pub plan_id: String,
    /// Current executor phase.
    #[serde(default = "PlanState::default_current_phase")]
    pub current_phase: PlanPhase,
    /// Agent instance keys currently assigned to this plan.
    pub assigned_agents: Vec<String>,
    /// Accumulated gate verdicts for the current iteration.
    pub gate_results: Vec<GateResult>,
    /// Current iteration (starts at 1, bumps on retry).
    pub iteration: u32,
    /// Unix millisecond timestamp when execution started.
    pub started_at_ms: u64,
    /// Files modified by agents so far (for conflict detection).
    pub files_changed: Vec<String>,
    /// Number of merge attempts so far.
    pub merge_attempts: u32,
    /// Last error message, if any.
    pub last_error: Option<String>,
    /// Whether the plan is paused.
    pub paused: bool,
    /// Priority (higher runs first, default 0).
    pub priority: u32,
}

/// A gate verdict recorded against a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    /// Which gate ran (e.g. `"compile"`, `"test"`, `"clippy"`).
    pub gate_name: String,
    /// The rung index within the gate ladder.
    pub rung: u32,
    /// Whether the gate passed.
    pub passed: bool,
    /// Human-readable summary of the result.
    pub summary: String,
    /// Wall-clock milliseconds.
    pub duration_ms: u64,
    /// Structured test counts when this result came from a test gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_count: Option<TestCount>,
}

impl GateResult {
    /// Construct from a [`Verdict`].
    #[must_use]
    pub fn from_verdict(verdict: &Verdict, rung: u32) -> Self {
        Self {
            gate_name: verdict.gate.clone(),
            rung,
            passed: verdict.passed,
            summary: verdict.reason.clone(),
            duration_ms: verdict.duration_ms,
            test_count: verdict.test_count,
        }
    }

    /// True when a failed test result is mostly passing.
    ///
    /// Mostly passing means more than 20 total tests, at least one failed test,
    /// and a pass rate greater than 90%. This is a signal for targeted retry
    /// rather than broad replanning.
    #[must_use]
    pub fn is_mostly_passing(results: &[Self]) -> bool {
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut ignored = 0u32;
        let mut saw_failed_test_gate = false;

        for result in results {
            let Some(count) = result.test_count else {
                continue;
            };
            if !result.passed {
                saw_failed_test_gate = true;
            }
            passed = passed.saturating_add(count.passed);
            failed = failed.saturating_add(count.failed);
            ignored = ignored.saturating_add(count.ignored);
        }

        let total = passed.saturating_add(failed).saturating_add(ignored);
        saw_failed_test_gate
            && total > 20
            && failed > 0
            && f64::from(passed) / f64::from(total) > 0.9
    }
}

impl PlanState {
    const fn default_current_phase() -> PlanPhase {
        PlanPhase::Queued
    }

    /// Create a new plan state starting at `Queued`.
    #[must_use]
    pub fn new(plan_id: impl Into<String>) -> Self {
        Self {
            plan_id: plan_id.into(),
            current_phase: Self::default_current_phase(),
            assigned_agents: Vec::new(),
            gate_results: Vec::new(),
            iteration: 1,
            started_at_ms: 0,
            files_changed: Vec::new(),
            merge_attempts: 0,
            last_error: None,
            paused: false,
            priority: 0,
        }
    }

    /// Create a plan state with a given priority.
    #[must_use]
    pub const fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Whether this plan is in a terminal phase.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.current_phase.is_terminal()
    }

    /// Whether all gate results collected so far have passed.
    #[must_use]
    pub fn all_gates_passed(&self) -> bool {
        !self.gate_results.is_empty() && self.gate_results.iter().all(|g| g.passed)
    }

    /// Whether any gate result has failed.
    #[must_use]
    pub fn has_gate_failure(&self) -> bool {
        self.gate_results.iter().any(|g| !g.passed)
    }

    /// Clear gate results for a new iteration.
    pub fn reset_for_retry(&mut self) {
        self.gate_results.clear();
        self.iteration += 1;
        self.last_error = None;
    }

    /// Reset the plan to the queued state after a structural re-plan.
    ///
    /// This preserves historical file-change context but clears execution
    /// bookkeeping so the plan can be re-dispatched cleanly.
    pub fn restart_for_replan(&mut self) {
        self.reset_for_retry();
        self.current_phase = PlanPhase::Queued;
        self.assigned_agents.clear();
        self.merge_attempts = 0;
        self.paused = false;
        self.started_at_ms = current_timestamp_ms();
    }
}

impl Default for PlanState {
    fn default() -> Self {
        Self::new("")
    }
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn new_plan_state_starts_queued() {
        let ps = PlanState::new("plan-1");
        assert_eq!(ps.plan_id, "plan-1");
        assert_eq!(ps.current_phase, PlanPhase::Queued);
        assert_eq!(ps.iteration, 1);
        assert!(!ps.is_terminal());
        assert!(!ps.paused);
    }

    #[test]
    fn with_priority_sets_priority() {
        let ps = PlanState::new("plan-2").with_priority(10);
        assert_eq!(ps.priority, 10);
    }

    #[test]
    fn all_gates_passed_when_empty_is_false() {
        let ps = PlanState::new("p");
        assert!(!ps.all_gates_passed());
    }

    #[test]
    fn all_gates_passed_with_passing_gates() {
        let mut ps = PlanState::new("p");
        ps.gate_results.push(GateResult {
            gate_name: "compile".into(),
            rung: 0,
            passed: true,
            summary: "ok".into(),
            duration_ms: 100,
            test_count: None,
        });
        ps.gate_results.push(GateResult {
            gate_name: "test".into(),
            rung: 1,
            passed: true,
            summary: "ok".into(),
            duration_ms: 200,
            test_count: None,
        });
        assert!(ps.all_gates_passed());
        assert!(!ps.has_gate_failure());
    }

    #[test]
    fn has_gate_failure_detects_failures() {
        let mut ps = PlanState::new("p");
        ps.gate_results.push(GateResult {
            gate_name: "compile".into(),
            rung: 0,
            passed: true,
            summary: "ok".into(),
            duration_ms: 100,
            test_count: None,
        });
        ps.gate_results.push(GateResult {
            gate_name: "test".into(),
            rung: 1,
            passed: false,
            summary: "2 failures".into(),
            duration_ms: 500,
            test_count: None,
        });
        assert!(ps.has_gate_failure());
        assert!(!ps.all_gates_passed());
    }

    #[test]
    fn reset_for_retry_increments_iteration() {
        let mut ps = PlanState::new("p");
        ps.gate_results.push(GateResult {
            gate_name: "test".into(),
            rung: 0,
            passed: false,
            summary: "fail".into(),
            duration_ms: 0,
            test_count: None,
        });
        ps.last_error = Some("bad".into());
        assert_eq!(ps.iteration, 1);

        ps.reset_for_retry();
        assert_eq!(ps.iteration, 2);
        assert!(ps.gate_results.is_empty());
        assert!(ps.last_error.is_none());
    }

    #[test]
    fn gate_result_from_verdict() {
        let v = Verdict::pass("compile");
        let gr = GateResult::from_verdict(&v, 0);
        assert!(gr.passed);
        assert_eq!(gr.gate_name, "compile");
        assert_eq!(gr.rung, 0);
    }

    #[test]
    fn mostly_passing_detects_targeted_test_failure() {
        let mostly = vec![GateResult::from_verdict(
            &Verdict::fail("test", "one failing test").with_test_count(TestCount::new(95, 1, 0)),
            2,
        )];
        assert!(GateResult::is_mostly_passing(&mostly));

        let half = vec![GateResult::from_verdict(
            &Verdict::fail("test", "many failures").with_test_count(TestCount::new(10, 10, 0)),
            2,
        )];
        assert!(!GateResult::is_mostly_passing(&half));

        let passing = vec![GateResult::from_verdict(
            &Verdict::pass("test").with_test_count(TestCount::new(95, 0, 0)),
            2,
        )];
        assert!(!GateResult::is_mostly_passing(&passing));
    }

    #[test]
    fn plan_state_serde_roundtrip() {
        let mut ps = PlanState::new("plan-42");
        ps.current_phase = PlanPhase::Implementing;
        ps.iteration = 3;
        ps.files_changed = vec!["src/lib.rs".into()];
        ps.assigned_agents = vec!["impl-t1".into()];
        ps.gate_results.push(GateResult {
            gate_name: "compile".into(),
            rung: 0,
            passed: true,
            summary: "ok".into(),
            duration_ms: 42,
            test_count: None,
        });
        let json = serde_json::to_string(&ps).unwrap();
        let decoded: PlanState = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.plan_id, ps.plan_id);
        assert_eq!(decoded.iteration, 3);
        assert_eq!(decoded.files_changed.len(), 1);
        assert_eq!(decoded.gate_results.len(), 1);
    }

    #[test]
    fn terminal_detection() {
        let mut ps = PlanState::new("p");
        assert!(!ps.is_terminal());

        ps.current_phase = PlanPhase::Complete;
        assert!(ps.is_terminal());

        ps.current_phase = PlanPhase::Failed {
            reason: roko_core::FailureKind::Deadlock,
        };
        assert!(ps.is_terminal());

        ps.current_phase = PlanPhase::Skipped;
        assert!(ps.is_terminal());
    }
}
