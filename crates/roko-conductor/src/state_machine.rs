//! Plan-lifecycle state machine extensions for the conductor.
//!
//! The canonical [`PlanPhase`] and [`PhaseKind`] types live in `roko-core`.
//! This module adds conductor-specific extensions: phase timeouts by
//! complexity band and structured phase-transition records.

use roko_core::{PhaseKind, TaskComplexityBand};
use serde::{Deserialize, Serialize};

// ─── Phase timeout constants (seconds) ──────────────────────────────────

/// Implementing timeout for complex tasks (seconds).
pub const TIMEOUT_IMPLEMENTING_COMPLEX: u64 = 600;
/// Implementing timeout for standard tasks (seconds).
pub const TIMEOUT_IMPLEMENTING_STANDARD: u64 = 300;
/// Implementing timeout for simple tasks (seconds).
pub const TIMEOUT_IMPLEMENTING_SIMPLE: u64 = 120;
/// Gating timeout (seconds).
pub const TIMEOUT_GATING: u64 = 300;
/// Reviewing timeout (seconds).
pub const TIMEOUT_REVIEWING: u64 = 300;
/// Merging timeout (seconds).
pub const TIMEOUT_MERGING: u64 = 60;
/// Enriching timeout (seconds).
pub const TIMEOUT_ENRICHING: u64 = 120;
/// Verifying timeout (seconds).
pub const TIMEOUT_VERIFYING: u64 = 300;
/// Auto-fixing timeout (seconds).
pub const TIMEOUT_AUTO_FIXING: u64 = 300;
/// Doc-revision timeout (seconds).
pub const TIMEOUT_DOC_REVISION: u64 = 120;

/// Returns the phase timeout in seconds for the given phase and complexity.
///
/// Terminal and non-timed phases return `None`.
#[must_use]
pub const fn phase_timeout(phase: PhaseKind, complexity: TaskComplexityBand) -> Option<u64> {
    match phase {
        PhaseKind::Implementing => Some(match complexity {
            TaskComplexityBand::Complex => TIMEOUT_IMPLEMENTING_COMPLEX,
            TaskComplexityBand::Fast => TIMEOUT_IMPLEMENTING_SIMPLE,
            // Standard and any future bands get the standard timeout.
            _ => TIMEOUT_IMPLEMENTING_STANDARD,
        }),
        PhaseKind::Gating => Some(TIMEOUT_GATING),
        PhaseKind::Verifying => Some(TIMEOUT_VERIFYING),
        PhaseKind::Reviewing => Some(TIMEOUT_REVIEWING),
        PhaseKind::Merging => Some(TIMEOUT_MERGING),
        PhaseKind::Enriching => Some(TIMEOUT_ENRICHING),
        PhaseKind::AutoFixing => Some(TIMEOUT_AUTO_FIXING),
        PhaseKind::DocRevision => Some(TIMEOUT_DOC_REVISION),
        // Queued, Complete, Failed, Skipped, Done, RegeneratingVerify — no timeout.
        _ => None,
    }
}

/// A recorded phase transition with timing metadata.
///
/// Used for conductor observability: each transition is logged so watchers
/// can reason about phase durations and detect stuck states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseTransition {
    /// The plan identifier this transition applies to.
    pub plan_id: String,
    /// Phase before the transition.
    pub from: PhaseKind,
    /// Phase after the transition.
    pub to: PhaseKind,
    /// Unix milliseconds when the transition occurred.
    pub at_ms: i64,
    /// Optional reason (e.g. which watcher triggered the transition).
    pub reason: Option<String>,
}

impl PhaseTransition {
    /// Create a new phase transition record.
    #[must_use]
    pub fn new(plan_id: impl Into<String>, from: PhaseKind, to: PhaseKind, at_ms: i64) -> Self {
        Self {
            plan_id: plan_id.into(),
            from,
            to,
            at_ms,
            reason: None,
        }
    }

    /// Attach a reason to this transition.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Duration in milliseconds since this transition (relative to `now_ms`).
    #[must_use]
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        (now_ms - self.at_ms).max(0)
    }

    /// Duration in seconds since this transition (relative to `now_ms`).
    #[must_use]
    pub fn elapsed_secs(&self, now_ms: i64) -> u64 {
        #[allow(clippy::cast_sign_loss)]
        {
            self.elapsed_ms(now_ms).max(0) as u64 / 1000
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementing_timeout_varies_by_complexity() {
        assert_eq!(
            phase_timeout(PhaseKind::Implementing, TaskComplexityBand::Complex),
            Some(600)
        );
        assert_eq!(
            phase_timeout(PhaseKind::Implementing, TaskComplexityBand::Standard),
            Some(300)
        );
        assert_eq!(
            phase_timeout(PhaseKind::Implementing, TaskComplexityBand::Fast),
            Some(120)
        );
    }

    #[test]
    fn non_implementing_phases_ignore_complexity() {
        // Gating timeout is the same regardless of complexity.
        assert_eq!(
            phase_timeout(PhaseKind::Gating, TaskComplexityBand::Complex),
            Some(300)
        );
        assert_eq!(
            phase_timeout(PhaseKind::Gating, TaskComplexityBand::Fast),
            Some(300)
        );
    }

    #[test]
    fn terminal_phases_have_no_timeout() {
        assert_eq!(
            phase_timeout(PhaseKind::Complete, TaskComplexityBand::Standard),
            None
        );
        assert_eq!(
            phase_timeout(PhaseKind::Failed, TaskComplexityBand::Standard),
            None
        );
        assert_eq!(
            phase_timeout(PhaseKind::Skipped, TaskComplexityBand::Standard),
            None
        );
    }

    #[test]
    fn queued_has_no_timeout() {
        assert_eq!(
            phase_timeout(PhaseKind::Queued, TaskComplexityBand::Standard),
            None
        );
    }

    #[test]
    fn all_active_phases_have_timeouts() {
        let active = [
            PhaseKind::Enriching,
            PhaseKind::Implementing,
            PhaseKind::Gating,
            PhaseKind::Verifying,
            PhaseKind::Reviewing,
            PhaseKind::AutoFixing,
            PhaseKind::Merging,
            PhaseKind::DocRevision,
        ];
        for p in active {
            assert!(
                phase_timeout(p, TaskComplexityBand::Standard).is_some(),
                "{p:?} should have a timeout"
            );
        }
    }

    #[test]
    fn phase_transition_elapsed() {
        let t = PhaseTransition::new("plan-1", PhaseKind::Implementing, PhaseKind::Gating, 1000);
        assert_eq!(t.elapsed_ms(2500), 1500);
        assert_eq!(t.elapsed_secs(2500), 1);
    }

    #[test]
    fn phase_transition_elapsed_clamps_negative() {
        let t = PhaseTransition::new("plan-1", PhaseKind::Queued, PhaseKind::Enriching, 5000);
        // now_ms before at_ms — should clamp to 0
        assert_eq!(t.elapsed_ms(3000), 0);
        assert_eq!(t.elapsed_secs(3000), 0);
    }

    #[test]
    fn phase_transition_with_reason() {
        let t = PhaseTransition::new("plan-2", PhaseKind::Gating, PhaseKind::AutoFixing, 1000)
            .with_reason("compile failure");
        assert_eq!(t.reason.as_deref(), Some("compile failure"));
    }

    #[test]
    fn phase_transition_serde_roundtrip() {
        let t = PhaseTransition::new("plan-3", PhaseKind::Implementing, PhaseKind::Failed, 42_000)
            .with_reason("stuck");
        let json = serde_json::to_string(&t).expect("serialize");
        let decoded: PhaseTransition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, decoded);
    }
}
