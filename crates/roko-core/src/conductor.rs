//! Conductor decision type, cognitive signals, and intervention classification.
//!
//! The conductor is Roko's meta-watcher: it observes the stream of
//! orchestration signals and emits a [`ConductorDecision`] that tells the
//! event loop what to do. The decision carries both a primary action
//! (Continue/Restart/Fail) and optional [`CognitiveSignal`]s — modulatory
//! hints that can be active simultaneously without changing the primary action.
//!
//! The actual decision function lives in `roko-conductor` (once created).
//! These types live here because they're pure data, used by both the
//! conductor and its consumers.

use crate::phase::FailureKind;
use serde::{Deserialize, Serialize};

// ─── CognitiveSignal ────────────────────────────────────────────────────

/// Rich cognitive signal — modulations richer than binary decisions.
///
/// Unlike [`ConductorDecision`] variants (which are final: restart/continue/fail),
/// cognitive signals are modulatory hints that can be active simultaneously.
/// For example, `Escalate` + `InjectContext` can both be emitted in a single
/// evaluation tick without changing the primary Continue/Restart/Fail action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CognitiveSignal {
    /// Temporarily suspend execution, preserve state.
    Pause,
    /// Resume from paused state.
    Resume,
    /// Reorder task queue based on new information.
    Reprioritize {
        /// Why the queue should be reordered.
        reason: String,
    },
    /// Add context without restarting.
    InjectContext {
        /// The context to inject into the next turn.
        context: String,
    },
    /// Promote to stronger model tier or request human review.
    Escalate {
        /// Target tier to escalate to (0 = fastest, higher = stronger).
        to_tier: u8,
    },
    /// Reduce pressure: extend deadlines, lower expectations.
    Cooldown {
        /// Multiplicative factor to extend budgets by (e.g. 1.5 = 50% more).
        factor: f64,
    },
    /// Switch to exploratory mode.
    Explore {
        /// Multiplier on the exploration budget (e.g. 2.0 = double).
        budget_multiplier: f64,
    },
    /// Graceful termination with state persistence.
    Shutdown {
        /// Why shutdown was requested.
        reason: String,
    },
}

impl CognitiveSignal {
    /// One-word label for logging and dashboards.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Reprioritize { .. } => "reprioritize",
            Self::InjectContext { .. } => "inject_context",
            Self::Escalate { .. } => "escalate",
            Self::Cooldown { .. } => "cooldown",
            Self::Explore { .. } => "explore",
            Self::Shutdown { .. } => "shutdown",
        }
    }
}

// ─── ConductorDecision ──────────────────────────────────────────────────

/// The possible outcomes the conductor emits per tick.
///
/// Each decision carries a primary action (continue, restart, or fail) plus
/// zero or more [`CognitiveSignal`]s that modulate behavior without changing
/// the primary action.
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

    /// Pair this decision with cognitive signals to form a full evaluation.
    #[must_use]
    pub fn with_signals(self, signals: Vec<CognitiveSignal>) -> ConductorEvaluation {
        ConductorEvaluation {
            decision: self,
            signals,
        }
    }
}

/// A full conductor evaluation: the primary decision plus any cognitive signals.
///
/// `ConductorDecision` is the primary action (continue/restart/fail), while
/// `signals` carry modulatory hints that can be active simultaneously.
/// For example, a `Continue` decision may come with an `Escalate` signal
/// suggesting a model-tier bump without requiring a restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConductorEvaluation {
    /// The primary action: continue, restart, or fail.
    pub decision: ConductorDecision,
    /// Zero or more cognitive signals emitted alongside the decision.
    pub signals: Vec<CognitiveSignal>,
}

impl ConductorEvaluation {
    /// Create an evaluation with no signals.
    #[must_use]
    pub fn from_decision(decision: ConductorDecision) -> Self {
        Self {
            decision,
            signals: Vec::new(),
        }
    }

    /// Convenience: continue with no signals.
    #[must_use]
    pub fn cont() -> Self {
        Self::from_decision(ConductorDecision::Continue)
    }

    /// True if the primary decision is terminal.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.decision.is_terminal()
    }

    /// True if the primary decision is continue.
    #[must_use]
    pub const fn is_continue(&self) -> bool {
        self.decision.is_continue()
    }

    /// Whether any signal of the given label is present.
    #[must_use]
    pub fn has_signal(&self, label: &str) -> bool {
        self.signals.iter().any(|s| s.label() == label)
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

    #[test]
    fn cognitive_signal_labels() {
        assert_eq!(CognitiveSignal::Pause.label(), "pause");
        assert_eq!(CognitiveSignal::Resume.label(), "resume");
        assert_eq!(CognitiveSignal::Escalate { to_tier: 2 }.label(), "escalate");
        assert_eq!(
            CognitiveSignal::Shutdown {
                reason: "done".into()
            }
            .label(),
            "shutdown"
        );
    }

    #[test]
    fn cognitive_signal_serde_roundtrip() {
        let signals = vec![
            CognitiveSignal::Pause,
            CognitiveSignal::Escalate { to_tier: 3 },
            CognitiveSignal::InjectContext {
                context: "extra info".into(),
            },
            CognitiveSignal::Cooldown { factor: 1.5 },
            CognitiveSignal::Explore {
                budget_multiplier: 2.0,
            },
        ];
        for s in &signals {
            let json = serde_json::to_string(s).unwrap();
            let decoded: CognitiveSignal = serde_json::from_str(&json).unwrap();
            assert_eq!(&decoded, s);
        }
    }

    #[test]
    fn evaluation_with_signals() {
        let eval = ConductorDecision::cont().with_signals(vec![
            CognitiveSignal::Escalate { to_tier: 2 },
            CognitiveSignal::Cooldown { factor: 1.5 },
        ]);
        assert!(eval.is_continue());
        assert!(!eval.is_terminal());
        assert!(eval.has_signal("escalate"));
        assert!(eval.has_signal("cooldown"));
        assert!(!eval.has_signal("pause"));
        assert_eq!(eval.signals.len(), 2);
    }

    #[test]
    fn evaluation_from_decision() {
        let eval = ConductorEvaluation::from_decision(ConductorDecision::restart("w", "r"));
        assert!(!eval.is_continue());
        assert!(eval.signals.is_empty());
    }

    #[test]
    fn evaluation_serde_roundtrip() {
        let eval = ConductorDecision::cont().with_signals(vec![CognitiveSignal::Pause]);
        let json = serde_json::to_string(&eval).unwrap();
        let decoded: ConductorEvaluation = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, eval);
    }
}
