//! Conductor decision type and intervention classification.
//!
//! The conductor is Roko's meta-watcher: it observes the stream of
//! orchestration signals and emits a [`ConductorDecision`] that tells the
//! event loop what to do. Per §11.2 of the parity checklist, the
//! conductor has **exactly three outcomes**: `Continue`, `Restart`, `Fail`.
//! There are no "nudges" — Mori's tiered-intervention model was simplified
//! to reduce false-positive agent interrupts.
//!
//! The actual decision function lives in `roko-conductor` (once created).
//! These types live here because they're pure data, used by both the
//! conductor and its consumers.

use crate::phase::FailureKind;
use serde::{Deserialize, Serialize};

/// The three possible outcomes the conductor emits per tick.
///
/// Simplified from Mori's `InterventionTier { Nudge, Restart, Abort }` —
/// this is §11.2 of the parity checklist: **no nudges**. Either the work
/// continues, we tear down and restart, or we fail terminally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConductorDecision {
    /// Work is healthy; let the event loop proceed.
    Continue,
    /// Kill current agents and restart from the phase boundary.
    Restart {
        /// Which watcher tripped (for observability).
        watcher: String,
        /// Human-readable reason (logged + surfaced to user).
        reason: String,
    },
    /// Terminal failure; do not retry.
    Fail {
        /// Which watcher tripped.
        watcher: String,
        /// Structured failure classification.
        reason: FailureKind,
    },
}

impl ConductorDecision {
    /// Shorthand for a continue decision.
    #[must_use]
    pub const fn cont() -> Self {
        Self::Continue
    }

    /// Shorthand for a restart decision.
    #[must_use]
    pub fn restart(watcher: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Restart {
            watcher: watcher.into(),
            reason: reason.into(),
        }
    }

    /// Shorthand for a fail decision.
    #[must_use]
    pub fn fail(watcher: impl Into<String>, reason: FailureKind) -> Self {
        Self::Fail {
            watcher: watcher.into(),
            reason,
        }
    }

    /// True if this decision tears down current work.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }

    /// True if this decision keeps the current work running.
    #[must_use]
    pub const fn is_continue(&self) -> bool {
        matches!(self, Self::Continue)
    }

    /// A one-word label for the TUI.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Restart { .. } => "restart",
            Self::Fail { .. } => "fail",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_is_not_terminal() {
        let d = ConductorDecision::cont();
        assert!(d.is_continue());
        assert!(!d.is_terminal());
        assert_eq!(d.label(), "continue");
    }

    #[test]
    fn restart_carries_watcher_and_reason() {
        let d = ConductorDecision::restart("ghost-turn", "60s no progress");
        assert!(!d.is_terminal());
        assert!(!d.is_continue());
        assert_eq!(d.label(), "restart");
        match d {
            ConductorDecision::Restart { watcher, reason } => {
                assert_eq!(watcher, "ghost-turn");
                assert_eq!(reason, "60s no progress");
            }
            _ => panic!("expected Restart"),
        }
    }

    #[test]
    fn fail_is_terminal() {
        let d = ConductorDecision::fail("circuit-breaker", FailureKind::Deadlock);
        assert!(d.is_terminal());
        assert!(!d.is_continue());
        assert_eq!(d.label(), "fail");
    }

    #[test]
    fn serde_roundtrip_continue() {
        let d = ConductorDecision::Continue;
        let json = serde_json::to_string(&d).unwrap();
        let decoded: ConductorDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, d);
    }

    #[test]
    fn serde_roundtrip_restart() {
        let d = ConductorDecision::restart("w", "r");
        let json = serde_json::to_string(&d).unwrap();
        let decoded: ConductorDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, d);
    }

    #[test]
    fn serde_roundtrip_fail() {
        let d = ConductorDecision::fail("w", FailureKind::SpawnFailures);
        let json = serde_json::to_string(&d).unwrap();
        let decoded: ConductorDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, d);
    }
}
