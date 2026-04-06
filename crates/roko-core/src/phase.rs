//! Plan-lifecycle state machine: phases, failure kinds, legal transitions.
//!
//! This is the executor's view of a plan's lifecycle — finer-grained than
//! the TOML-serialized [`PlanStatus`](crate::task::PlanStatus). A plan
//! flows through phases like Implementing → Gating → Verifying → Reviewing
//! → Merging → Complete, with back-edges to `AutoFixing` / `RegeneratingVerify`
//! / Failed.
//!
//! The [`valid_transitions`] table enforces the contract from
//! §12.2 of the parity checklist: only the listed transitions are legal.
//!
//! Mirrors `apps/mori/src/orchestrator/executor.rs` `PlanPhase` + `FailureKind`.

use serde::{Deserialize, Serialize};

// ─── FailureKind (structured failure classification) ──────────────────────

/// Structured classification of why a plan failed.
///
/// Matches Mori's `FailureKind` verbatim so `.mori/state/*.json` files
/// persisted by Mori are readable by Roko.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum FailureKind {
    /// Auto-fix attempts exhausted (express mode).
    AutoFixExhausted,
    /// Every task in the plan failed.
    AllTasksFailed,
    /// A task exhausted its retry budget; plan may be partially implemented.
    TaskRetriesExhausted,
    /// Plan setup failed (missing files, worktree issues).
    SetupFailed,
    /// Exceeded max iteration/revision cycles.
    MaxIterations,
    /// Too many consecutive spawn failures.
    SpawnFailures,
    /// Merge queue deadlock detected.
    Deadlock,
    /// Worktree missing or inaccessible.
    WorktreeMissing,
    /// Agent wrote no code after all retry attempts.
    VacuousImplementation,
    /// Verify-chain script is fundamentally broken.
    VerifyScriptBroken,
    /// Transient or unclassified failure (may be retried).
    Other(String),
}

impl FailureKind {
    /// True for failures that explicitly require human intervention.
    ///
    /// Matches Mori: only `Other("manual repair ...")` demands humans.
    #[must_use]
    pub fn requires_manual_repair(&self) -> bool {
        matches!(self, Self::Other(s) if s.contains("manual repair"))
    }

    /// Failures not requiring manual repair are recoverable.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        !self.requires_manual_repair()
    }

    /// Whether this kind should auto-retry when the operator resumes.
    #[must_use]
    pub fn auto_retry_on_resume(&self) -> bool {
        !self.requires_manual_repair()
    }

    /// Per-kind cooldown before auto-retry, in seconds.
    ///
    /// Figures from Mori's `FailureKind::retry_cooldown_secs`.
    #[must_use]
    pub fn retry_cooldown_secs(&self) -> u64 {
        match self {
            Self::Other(s) if s.contains("startup reconciliation") => 30,
            Self::SpawnFailures | Self::SetupFailed | Self::WorktreeMissing | Self::Deadlock => 120,
            Self::AutoFixExhausted | Self::TaskRetriesExhausted | Self::AllTasksFailed => 300,
            Self::VacuousImplementation | Self::VerifyScriptBroken | Self::MaxIterations => 600,
            Self::Other(_) => 180,
        }
    }
}

impl std::fmt::Display for FailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AutoFixExhausted => write!(f, "auto-fix attempts exhausted"),
            Self::AllTasksFailed => write!(f, "all tasks failed"),
            Self::TaskRetriesExhausted => write!(f, "task retries exhausted"),
            Self::SetupFailed => write!(f, "setup failed"),
            Self::MaxIterations => write!(f, "exceeded max iterations"),
            Self::SpawnFailures => write!(f, "spawn failures"),
            Self::Deadlock => write!(f, "deadlock"),
            Self::WorktreeMissing => write!(f, "worktree missing"),
            Self::VacuousImplementation => {
                write!(f, "vacuous implementation (no code written after retries)")
            }
            Self::VerifyScriptBroken => {
                write!(f, "verify script broken (impossible conditions after regen)")
            }
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

// ─── PlanPhase (executor state machine) ───────────────────────────────────

/// Discriminant for a [`PlanPhase`] without the [`FailureKind`] payload.
///
/// Used to index transition tables and persist phase labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PhaseKind {
    /// Plan has been enqueued but not started.
    Queued,
    /// Enrichment pipeline is producing brief/tests/invariants.
    Enriching,
    /// Implementer agents editing files.
    Implementing,
    /// Compile/test gates running.
    Gating,
    /// Custom verify-chain scripts running.
    Verifying,
    /// Reviewer agents inspecting diff.
    Reviewing,
    /// Scribe finalizing docs.
    DocRevision,
    /// `AutoFixer` patching errors after gate failure.
    AutoFixing,
    /// Regenerating verify-chain script after convergence detection.
    RegeneratingVerify,
    /// All reviews passed, waiting for merge slot.
    Merging,
    /// Fully merged.
    Complete,
    /// Gates/reviews completed; awaiting manual merge decision.
    Done,
    /// Terminally failed.
    Failed,
    /// Operator-requested skip.
    Skipped,
}

/// The executor's phase for a plan, with a failure payload when `Failed`.
///
/// Mirrors `apps/mori/src/orchestrator/executor.rs::PlanPhase` but splits
/// the discriminant out as [`PhaseKind`] so transition tables can live
/// as pure data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PlanPhase {
    /// Plan has been enqueued but not started.
    Queued,
    /// Enrichment pipeline is producing brief/tests/invariants.
    Enriching,
    /// Implementer agents editing files.
    Implementing,
    /// Compile/test gates running.
    Gating,
    /// Custom verify-chain scripts running.
    Verifying,
    /// Reviewer agents inspecting diff.
    Reviewing,
    /// Scribe finalizing docs.
    DocRevision,
    /// `AutoFixer` patching errors after gate failure.
    AutoFixing,
    /// Regenerating verify-chain script after convergence detection.
    RegeneratingVerify,
    /// All reviews passed, waiting for merge slot.
    Merging,
    /// Fully merged.
    Complete,
    /// Gates/reviews completed; awaiting manual merge decision.
    Done,
    /// Terminally failed.
    Failed {
        /// The structured failure classification.
        reason: FailureKind,
    },
    /// Operator-requested skip.
    Skipped,
}

impl PlanPhase {
    /// Discriminant-only view (drops the `FailureKind` payload).
    #[must_use]
    pub const fn kind(&self) -> PhaseKind {
        match self {
            Self::Queued => PhaseKind::Queued,
            Self::Enriching => PhaseKind::Enriching,
            Self::Implementing => PhaseKind::Implementing,
            Self::Gating => PhaseKind::Gating,
            Self::Verifying => PhaseKind::Verifying,
            Self::Reviewing => PhaseKind::Reviewing,
            Self::DocRevision => PhaseKind::DocRevision,
            Self::AutoFixing => PhaseKind::AutoFixing,
            Self::RegeneratingVerify => PhaseKind::RegeneratingVerify,
            Self::Merging => PhaseKind::Merging,
            Self::Complete => PhaseKind::Complete,
            Self::Done => PhaseKind::Done,
            Self::Failed { .. } => PhaseKind::Failed,
            Self::Skipped => PhaseKind::Skipped,
        }
    }

    /// Terminal phases cannot transition further.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self.kind(), PhaseKind::Complete | PhaseKind::Failed | PhaseKind::Skipped)
    }

    /// True when the phase can legally transition to `next`.
    ///
    /// Enforces the contract in §12.2: only the listed transitions are
    /// legal. This is the canonical table — all other arcs are bugs.
    #[must_use]
    pub fn can_transition_to(&self, next: PhaseKind) -> bool {
        valid_transitions(self.kind()).contains(&next)
    }
}

/// The legal successors for a given phase (§12.2 transition table).
///
/// This is a pure, data-driven table. Any phase not listed here has no
/// valid successors (it's either terminal or a bug).
#[must_use]
#[allow(clippy::match_same_arms)] // arms intentionally spelled out for doc clarity
pub const fn valid_transitions(from: PhaseKind) -> &'static [PhaseKind] {
    use PhaseKind::{
        AutoFixing, Complete, DocRevision, Done, Enriching, Failed, Gating, Implementing, Merging,
        Queued, RegeneratingVerify, Reviewing, Skipped, Verifying,
    };
    match from {
        Queued => &[Enriching, Skipped, Failed],
        Enriching => &[Implementing, Failed, Skipped],
        Implementing => &[Gating, Failed, Skipped],
        Gating => &[Verifying, AutoFixing, Failed, Skipped],
        AutoFixing => &[Gating, Failed, Skipped],
        Verifying => &[Reviewing, RegeneratingVerify, Failed, Skipped],
        RegeneratingVerify => &[Verifying, Failed, Skipped],
        Reviewing => &[DocRevision, Implementing, Failed, Skipped],
        DocRevision => &[Merging, Done, Failed, Skipped],
        Merging => &[Complete, Failed, Skipped],
        Done => &[Merging, Skipped, Failed],
        // Terminal — no transitions.
        Complete | Failed | Skipped => &[],
    }
}

/// Check whether transitioning from `from` to `to` is a valid monotonic
/// progression according to the [`valid_transitions`] table.
///
/// Returns `true` when `to` appears in `valid_transitions(from.kind())`.
/// This is a convenience wrapper for use by progress-enforcement code
/// outside `roko-core` (e.g. `roko-orchestrator::progress`).
#[must_use]
pub fn is_monotonic_progression(from: &PlanPhase, to: &PlanPhase) -> bool {
    valid_transitions(from.kind()).contains(&to.kind())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_kind_manual_repair_detection() {
        assert!(FailureKind::Other("needs manual repair".into()).requires_manual_repair());
        assert!(!FailureKind::Other("startup reconciliation".into()).requires_manual_repair());
        assert!(!FailureKind::AutoFixExhausted.requires_manual_repair());
    }

    #[test]
    fn failure_kind_recoverability() {
        assert!(FailureKind::AutoFixExhausted.is_recoverable());
        assert!(!FailureKind::Other("needs manual repair".into()).is_recoverable());
    }

    #[test]
    fn failure_kind_cooldowns_are_positive() {
        use FailureKind::{
            AllTasksFailed, AutoFixExhausted, Deadlock, MaxIterations, Other, SetupFailed,
            SpawnFailures, TaskRetriesExhausted, VacuousImplementation, VerifyScriptBroken,
            WorktreeMissing,
        };
        let all = [
            AutoFixExhausted,
            AllTasksFailed,
            TaskRetriesExhausted,
            SetupFailed,
            MaxIterations,
            SpawnFailures,
            Deadlock,
            WorktreeMissing,
            VacuousImplementation,
            VerifyScriptBroken,
            Other("generic".into()),
        ];
        for k in &all {
            assert!(k.retry_cooldown_secs() > 0, "{k:?}");
        }
    }

    #[test]
    fn terminal_phases_have_no_successors() {
        assert!(PlanPhase::Complete.is_terminal());
        assert!(PlanPhase::Skipped.is_terminal());
        assert!(PlanPhase::Failed { reason: FailureKind::Deadlock }.is_terminal());
        assert!(valid_transitions(PhaseKind::Complete).is_empty());
        assert!(valid_transitions(PhaseKind::Failed).is_empty());
        assert!(valid_transitions(PhaseKind::Skipped).is_empty());
    }

    #[test]
    fn happy_path_transitions_are_legal() {
        // Queued → Enriching → Implementing → Gating → Verifying
        // → Reviewing → DocRevision → Merging → Complete
        assert!(PlanPhase::Queued.can_transition_to(PhaseKind::Enriching));
        assert!(PlanPhase::Enriching.can_transition_to(PhaseKind::Implementing));
        assert!(PlanPhase::Implementing.can_transition_to(PhaseKind::Gating));
        assert!(PlanPhase::Gating.can_transition_to(PhaseKind::Verifying));
        assert!(PlanPhase::Verifying.can_transition_to(PhaseKind::Reviewing));
        assert!(PlanPhase::Reviewing.can_transition_to(PhaseKind::DocRevision));
        assert!(PlanPhase::DocRevision.can_transition_to(PhaseKind::Merging));
        assert!(PlanPhase::Merging.can_transition_to(PhaseKind::Complete));
    }

    #[test]
    fn auto_fix_loop_is_legal() {
        // Gating → AutoFixing → Gating (retry)
        assert!(PlanPhase::Gating.can_transition_to(PhaseKind::AutoFixing));
        assert!(PlanPhase::AutoFixing.can_transition_to(PhaseKind::Gating));
    }

    #[test]
    fn review_can_send_back_to_implementing() {
        // Reviewer demands rework → back to Implementing
        assert!(PlanPhase::Reviewing.can_transition_to(PhaseKind::Implementing));
    }

    #[test]
    fn illegal_transitions_rejected() {
        // Cannot jump from Queued straight to Complete
        assert!(!PlanPhase::Queued.can_transition_to(PhaseKind::Complete));
        // Cannot reverse from Complete to anything
        assert!(!PlanPhase::Complete.can_transition_to(PhaseKind::Implementing));
        // Cannot rewind from Gating to Implementing (only via Reviewing)
        assert!(!PlanPhase::Gating.can_transition_to(PhaseKind::Implementing));
    }

    #[test]
    fn kind_discriminant_roundtrip() {
        let p = PlanPhase::Failed { reason: FailureKind::Deadlock };
        assert_eq!(p.kind(), PhaseKind::Failed);
        assert_eq!(PlanPhase::Queued.kind(), PhaseKind::Queued);
        assert_eq!(PlanPhase::Merging.kind(), PhaseKind::Merging);
    }

    #[test]
    fn failed_phase_serializes_reason() {
        let p = PlanPhase::Failed { reason: FailureKind::AutoFixExhausted };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("failed"));
        assert!(json.contains("AutoFixExhausted"));
        let decoded: PlanPhase = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn display_formats_failure_kinds() {
        assert_eq!(FailureKind::Deadlock.to_string(), "deadlock");
        assert_eq!(FailureKind::AutoFixExhausted.to_string(), "auto-fix attempts exhausted");
        assert_eq!(FailureKind::Other("custom".into()).to_string(), "custom");
    }
}
