//! Plan state machine — drives a plan through [`PlanPhase`] transitions.
//!
//! The [`PlanStateMachine`] is the pure-logic core of the executor. Given a
//! [`PlanState`] and an [`ExecutorEvent`], it computes the next phase (or
//! rejects illegal transitions) and suggests the next
//! [`ExecutorAction`](super::action::ExecutorAction) based on the new state.

use roko_core::{valid_transitions, AgentRole, FailureKind, PhaseKind, PlanPhase};

use super::action::ExecutorAction;
use super::plan_state::PlanState;

/// Maximum auto-fix iterations before declaring failure.
const MAX_AUTO_FIX_ITERATIONS: u32 = 5;

/// Maximum merge attempts before declaring failure.
const MAX_MERGE_ATTEMPTS: u32 = 3;

// ─── TransitionError ────────────────────────────────────────────────────

/// Error returned when a phase transition is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    /// The phase the plan was in.
    pub from: PhaseKind,
    /// The phase the caller tried to move to.
    pub to: PhaseKind,
    /// Human-readable explanation.
    pub reason: String,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot transition {:?} -> {:?}: {}",
            self.from, self.to, self.reason
        )
    }
}

impl std::error::Error for TransitionError {}

// ─── ExecutorEvent ──────────────────────────────────────────────────────

/// Events fed into the state machine to drive transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorEvent {
    /// Plan has been dispatched — start enrichment.
    Start,
    /// Enrichment completed successfully.
    EnrichmentDone,
    /// Implementation completed (all tasks done).
    ImplementationDone,
    /// A gate passed.
    GatePassed,
    /// A gate failed.
    GateFailed,
    /// Auto-fix completed — retry gating.
    AutoFixDone,
    /// Verification (verify-chain) passed.
    VerifyPassed,
    /// Verification (verify-chain) failed — needs regeneration.
    VerifyFailed,
    /// Verify regeneration completed — retry verification.
    VerifyRegenDone,
    /// Review approved — proceed to doc revision.
    ReviewApproved,
    /// Review requested rework — back to implementing.
    ReviewRejected,
    /// Doc revision completed.
    DocRevisionDone,
    /// Merge succeeded.
    MergeSucceeded,
    /// Merge failed.
    MergeFailed,
    /// Done phase: operator triggers merge.
    OperatorMerge,
    /// Operator requested skip.
    Skip,
    /// Unrecoverable failure with reason.
    Fatal(String),
}

// ─── PlanStateMachine ───────────────────────────────────────────────────

/// Pure-logic state machine that drives plan phase transitions.
///
/// Stateless: all mutable state lives in [`PlanState`]. The machine
/// only reads the plan state and returns transition results.
#[derive(Debug, Default)]
pub struct PlanStateMachine;

impl PlanStateMachine {
    /// Attempt to transition a plan given an event.
    ///
    /// Returns the new [`PlanPhase`] on success, or a [`TransitionError`]
    /// if the transition is not legal according to the
    /// [`valid_transitions`] table.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] when the requested transition is not in
    /// the legal transition table for the plan's current phase.
    #[allow(clippy::match_same_arms)] // Each phase's transitions listed separately for clarity
    pub fn transition(
        plan_state: &PlanState,
        event: &ExecutorEvent,
    ) -> Result<PlanPhase, TransitionError> {
        let current = &plan_state.current_phase;
        let current_kind = current.kind();

        let next = match (current_kind, event) {
            // ── Queued ──
            (PhaseKind::Queued, ExecutorEvent::Start) => PlanPhase::Enriching,
            (PhaseKind::Queued, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── Enriching ──
            (PhaseKind::Enriching, ExecutorEvent::EnrichmentDone) => PlanPhase::Implementing,
            (PhaseKind::Enriching, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── Implementing ──
            (PhaseKind::Implementing, ExecutorEvent::ImplementationDone) => PlanPhase::Gating,
            (PhaseKind::Implementing, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── Gating ──
            (PhaseKind::Gating, ExecutorEvent::GatePassed) => PlanPhase::Verifying,
            (PhaseKind::Gating, ExecutorEvent::GateFailed) => {
                if plan_state.iteration >= MAX_AUTO_FIX_ITERATIONS {
                    PlanPhase::Failed {
                        reason: FailureKind::AutoFixExhausted,
                    }
                } else {
                    PlanPhase::AutoFixing
                }
            }
            (PhaseKind::Gating, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── AutoFixing ──
            (PhaseKind::AutoFixing, ExecutorEvent::AutoFixDone) => PlanPhase::Gating,
            (PhaseKind::AutoFixing, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── Verifying ──
            (PhaseKind::Verifying, ExecutorEvent::VerifyPassed) => PlanPhase::Reviewing,
            (PhaseKind::Verifying, ExecutorEvent::VerifyFailed) => PlanPhase::RegeneratingVerify,
            (PhaseKind::Verifying, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── RegeneratingVerify ──
            (PhaseKind::RegeneratingVerify, ExecutorEvent::VerifyRegenDone) => PlanPhase::Verifying,
            (PhaseKind::RegeneratingVerify, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── Reviewing ──
            (PhaseKind::Reviewing, ExecutorEvent::ReviewApproved) => PlanPhase::DocRevision,
            (PhaseKind::Reviewing, ExecutorEvent::ReviewRejected) => PlanPhase::Implementing,
            (PhaseKind::Reviewing, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── DocRevision ──
            (PhaseKind::DocRevision, ExecutorEvent::DocRevisionDone) => PlanPhase::Merging,
            (PhaseKind::DocRevision, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── Merging ──
            (PhaseKind::Merging, ExecutorEvent::MergeSucceeded) => PlanPhase::Complete,
            (PhaseKind::Merging, ExecutorEvent::MergeFailed) => {
                if plan_state.merge_attempts >= MAX_MERGE_ATTEMPTS {
                    PlanPhase::Failed {
                        reason: FailureKind::Deadlock,
                    }
                } else {
                    PlanPhase::Failed {
                        reason: FailureKind::Other("merge conflict — retry".into()),
                    }
                }
            }
            (PhaseKind::Merging, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── Done ──
            (PhaseKind::Done, ExecutorEvent::OperatorMerge) => PlanPhase::Merging,
            (PhaseKind::Done, ExecutorEvent::Skip) => PlanPhase::Skipped,

            // ── Fatal from any non-terminal phase ──
            (kind, ExecutorEvent::Fatal(reason)) => {
                let target = PhaseKind::Failed;
                if valid_transitions(kind).contains(&target) {
                    PlanPhase::Failed {
                        reason: FailureKind::Other(reason.clone()),
                    }
                } else {
                    return Err(TransitionError {
                        from: kind,
                        to: target,
                        reason: format!("cannot fail from {kind:?}"),
                    });
                }
            }

            // ── Anything else ──
            (from, evt) => {
                let to = event_target_kind(evt);
                return Err(TransitionError {
                    from,
                    to,
                    reason: format!("no transition for event {evt:?} in phase {from:?}"),
                });
            }
        };

        // Validate against the canonical transition table.
        let next_kind = next.kind();
        if !valid_transitions(current_kind).contains(&next_kind) {
            return Err(TransitionError {
                from: current_kind,
                to: next_kind,
                reason: format!(
                    "transition {current_kind:?} -> {next_kind:?} not in valid_transitions table",
                ),
            });
        }

        Ok(next)
    }

    /// Suggest the next action for a plan based on its current state.
    ///
    /// Returns `None` if the plan is paused, terminal, or waiting for an
    /// external event (no proactive action needed).
    #[must_use]
    pub fn next_action(plan_state: &PlanState) -> Option<ExecutorAction> {
        if plan_state.paused || plan_state.is_terminal() {
            return None;
        }

        match plan_state.current_phase.kind() {
            PhaseKind::Queued => Some(ExecutorAction::DispatchPlan {
                plan_id: plan_state.plan_id.clone(),
            }),
            PhaseKind::Implementing => Some(ExecutorAction::SpawnAgent {
                plan_id: plan_state.plan_id.clone(),
                role: AgentRole::Implementer,
                task: "next".into(),
            }),
            PhaseKind::Gating => Some(ExecutorAction::RunGate {
                plan_id: plan_state.plan_id.clone(),
                rung: plan_state
                    .gate_results
                    .len()
                    .try_into()
                    .unwrap_or(u32::MAX),
            }),
            PhaseKind::AutoFixing => Some(ExecutorAction::SpawnAgent {
                plan_id: plan_state.plan_id.clone(),
                role: AgentRole::AutoFixer,
                task: "fix".into(),
            }),
            PhaseKind::Verifying => Some(ExecutorAction::RunGate {
                plan_id: plan_state.plan_id.clone(),
                rung: 0,
            }),
            PhaseKind::Reviewing => Some(ExecutorAction::SpawnAgent {
                plan_id: plan_state.plan_id.clone(),
                role: AgentRole::Auditor,
                task: "review".into(),
            }),
            PhaseKind::DocRevision => Some(ExecutorAction::SpawnAgent {
                plan_id: plan_state.plan_id.clone(),
                role: AgentRole::Scribe,
                task: "docs".into(),
            }),
            PhaseKind::Merging => Some(ExecutorAction::MergeBranch {
                plan_id: plan_state.plan_id.clone(),
            }),
            // Enriching, RegeneratingVerify, Done, terminal — waiting for external input
            _ => None,
        }
    }
}

/// Map an event to its likely target phase kind (for error messages).
const fn event_target_kind(event: &ExecutorEvent) -> PhaseKind {
    match event {
        ExecutorEvent::Start => PhaseKind::Enriching,
        ExecutorEvent::EnrichmentDone | ExecutorEvent::ReviewRejected => PhaseKind::Implementing,
        ExecutorEvent::ImplementationDone | ExecutorEvent::AutoFixDone => PhaseKind::Gating,
        ExecutorEvent::GatePassed | ExecutorEvent::VerifyRegenDone => PhaseKind::Verifying,
        ExecutorEvent::GateFailed => PhaseKind::AutoFixing,
        ExecutorEvent::VerifyPassed => PhaseKind::Reviewing,
        ExecutorEvent::VerifyFailed => PhaseKind::RegeneratingVerify,
        ExecutorEvent::ReviewApproved => PhaseKind::DocRevision,
        ExecutorEvent::DocRevisionDone | ExecutorEvent::OperatorMerge => PhaseKind::Merging,
        ExecutorEvent::MergeSucceeded => PhaseKind::Complete,
        ExecutorEvent::MergeFailed | ExecutorEvent::Fatal(_) => PhaseKind::Failed,
        ExecutorEvent::Skip => PhaseKind::Skipped,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn queued() -> PlanState {
        PlanState::new("test-plan")
    }

    fn at_phase(phase: PlanPhase) -> PlanState {
        let mut ps = PlanState::new("test-plan");
        ps.current_phase = phase;
        ps
    }

    // ── Happy path ──

    #[test]
    fn happy_path_full_lifecycle() {
        let mut ps = queued();

        // Queued -> Enriching
        ps.current_phase = PlanStateMachine::transition(&ps, &ExecutorEvent::Start).unwrap();
        assert_eq!(ps.current_phase.kind(), PhaseKind::Enriching);

        // Enriching -> Implementing
        ps.current_phase =
            PlanStateMachine::transition(&ps, &ExecutorEvent::EnrichmentDone).unwrap();
        assert_eq!(ps.current_phase.kind(), PhaseKind::Implementing);

        // Implementing -> Gating
        ps.current_phase =
            PlanStateMachine::transition(&ps, &ExecutorEvent::ImplementationDone).unwrap();
        assert_eq!(ps.current_phase.kind(), PhaseKind::Gating);

        // Gating -> Verifying
        ps.current_phase =
            PlanStateMachine::transition(&ps, &ExecutorEvent::GatePassed).unwrap();
        assert_eq!(ps.current_phase.kind(), PhaseKind::Verifying);

        // Verifying -> Reviewing
        ps.current_phase =
            PlanStateMachine::transition(&ps, &ExecutorEvent::VerifyPassed).unwrap();
        assert_eq!(ps.current_phase.kind(), PhaseKind::Reviewing);

        // Reviewing -> DocRevision
        ps.current_phase =
            PlanStateMachine::transition(&ps, &ExecutorEvent::ReviewApproved).unwrap();
        assert_eq!(ps.current_phase.kind(), PhaseKind::DocRevision);

        // DocRevision -> Merging
        ps.current_phase =
            PlanStateMachine::transition(&ps, &ExecutorEvent::DocRevisionDone).unwrap();
        assert_eq!(ps.current_phase.kind(), PhaseKind::Merging);

        // Merging -> Complete
        ps.current_phase =
            PlanStateMachine::transition(&ps, &ExecutorEvent::MergeSucceeded).unwrap();
        assert_eq!(ps.current_phase.kind(), PhaseKind::Complete);
        assert!(ps.is_terminal());
    }

    // ── Auto-fix loop ──

    #[test]
    fn gate_failure_enters_auto_fix() {
        let ps = at_phase(PlanPhase::Gating);
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::GateFailed).unwrap();
        assert_eq!(next.kind(), PhaseKind::AutoFixing);
    }

    #[test]
    fn auto_fix_returns_to_gating() {
        let ps = at_phase(PlanPhase::AutoFixing);
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::AutoFixDone).unwrap();
        assert_eq!(next.kind(), PhaseKind::Gating);
    }

    #[test]
    fn max_auto_fix_iterations_leads_to_failure() {
        let mut ps = at_phase(PlanPhase::Gating);
        ps.iteration = MAX_AUTO_FIX_ITERATIONS;
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::GateFailed).unwrap();
        assert_eq!(next.kind(), PhaseKind::Failed);
    }

    // ── Verify regeneration loop ──

    #[test]
    fn verify_failure_enters_regen() {
        let ps = at_phase(PlanPhase::Verifying);
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::VerifyFailed).unwrap();
        assert_eq!(next.kind(), PhaseKind::RegeneratingVerify);
    }

    #[test]
    fn verify_regen_returns_to_verifying() {
        let ps = at_phase(PlanPhase::RegeneratingVerify);
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::VerifyRegenDone).unwrap();
        assert_eq!(next.kind(), PhaseKind::Verifying);
    }

    // ── Review rejection ──

    #[test]
    fn review_rejection_returns_to_implementing() {
        let ps = at_phase(PlanPhase::Reviewing);
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::ReviewRejected).unwrap();
        assert_eq!(next.kind(), PhaseKind::Implementing);
    }

    // ── Skip from any non-terminal ──

    #[test]
    fn skip_from_queued() {
        let ps = queued();
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::Skip).unwrap();
        assert_eq!(next.kind(), PhaseKind::Skipped);
    }

    #[test]
    fn skip_from_implementing() {
        let ps = at_phase(PlanPhase::Implementing);
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::Skip).unwrap();
        assert_eq!(next.kind(), PhaseKind::Skipped);
    }

    // ── Fatal ──

    #[test]
    fn fatal_from_implementing() {
        let ps = at_phase(PlanPhase::Implementing);
        let next =
            PlanStateMachine::transition(&ps, &ExecutorEvent::Fatal("crash".into())).unwrap();
        assert_eq!(next.kind(), PhaseKind::Failed);
    }

    #[test]
    fn fatal_from_gating() {
        let ps = at_phase(PlanPhase::Gating);
        let next =
            PlanStateMachine::transition(&ps, &ExecutorEvent::Fatal("oom".into())).unwrap();
        assert_eq!(next.kind(), PhaseKind::Failed);
    }

    // ── Illegal transitions ──

    #[test]
    fn cannot_start_from_implementing() {
        let ps = at_phase(PlanPhase::Implementing);
        let result = PlanStateMachine::transition(&ps, &ExecutorEvent::Start);
        assert!(result.is_err());
    }

    #[test]
    fn cannot_gate_pass_from_queued() {
        let ps = queued();
        let result = PlanStateMachine::transition(&ps, &ExecutorEvent::GatePassed);
        assert!(result.is_err());
    }

    #[test]
    fn cannot_transition_from_complete() {
        let ps = at_phase(PlanPhase::Complete);
        let result = PlanStateMachine::transition(&ps, &ExecutorEvent::Start);
        assert!(result.is_err());
    }

    // ── Merge failure ──

    #[test]
    fn merge_failure_with_attempts_left() {
        let mut ps = at_phase(PlanPhase::Merging);
        ps.merge_attempts = 1;
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::MergeFailed).unwrap();
        assert_eq!(next.kind(), PhaseKind::Failed);
    }

    #[test]
    fn merge_failure_exhausted_is_deadlock() {
        let mut ps = at_phase(PlanPhase::Merging);
        ps.merge_attempts = MAX_MERGE_ATTEMPTS;
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::MergeFailed).unwrap();
        assert_eq!(next.kind(), PhaseKind::Failed);
        if let PlanPhase::Failed { reason } = &next {
            assert_eq!(*reason, FailureKind::Deadlock);
        } else {
            panic!("expected Failed");
        }
    }

    // ── Done -> Merging ──

    #[test]
    fn done_to_merging_on_operator_merge() {
        let ps = at_phase(PlanPhase::Done);
        let next = PlanStateMachine::transition(&ps, &ExecutorEvent::OperatorMerge).unwrap();
        assert_eq!(next.kind(), PhaseKind::Merging);
    }

    // ── next_action ──

    #[test]
    fn next_action_queued_dispatches() {
        let ps = queued();
        let action = PlanStateMachine::next_action(&ps);
        assert!(matches!(action, Some(ExecutorAction::DispatchPlan { .. })));
    }

    #[test]
    fn next_action_implementing_spawns_implementer() {
        let ps = at_phase(PlanPhase::Implementing);
        let action = PlanStateMachine::next_action(&ps);
        match action {
            Some(ExecutorAction::SpawnAgent { role, .. }) => {
                assert_eq!(role, AgentRole::Implementer);
            }
            other => panic!("expected SpawnAgent, got {other:?}"),
        }
    }

    #[test]
    fn next_action_gating_runs_gate() {
        let ps = at_phase(PlanPhase::Gating);
        let action = PlanStateMachine::next_action(&ps);
        assert!(matches!(action, Some(ExecutorAction::RunGate { .. })));
    }

    #[test]
    fn next_action_merging_merges() {
        let ps = at_phase(PlanPhase::Merging);
        let action = PlanStateMachine::next_action(&ps);
        assert!(matches!(action, Some(ExecutorAction::MergeBranch { .. })));
    }

    #[test]
    fn next_action_paused_is_none() {
        let mut ps = at_phase(PlanPhase::Implementing);
        ps.paused = true;
        assert!(PlanStateMachine::next_action(&ps).is_none());
    }

    #[test]
    fn next_action_terminal_is_none() {
        let ps = at_phase(PlanPhase::Complete);
        assert!(PlanStateMachine::next_action(&ps).is_none());
    }

    #[test]
    fn next_action_auto_fixing_spawns_auto_fixer() {
        let ps = at_phase(PlanPhase::AutoFixing);
        let action = PlanStateMachine::next_action(&ps);
        match action {
            Some(ExecutorAction::SpawnAgent { role, .. }) => {
                assert_eq!(role, AgentRole::AutoFixer);
            }
            other => panic!("expected SpawnAgent(AutoFixer), got {other:?}"),
        }
    }

    #[test]
    fn next_action_reviewing_spawns_auditor() {
        let ps = at_phase(PlanPhase::Reviewing);
        let action = PlanStateMachine::next_action(&ps);
        match action {
            Some(ExecutorAction::SpawnAgent { role, .. }) => {
                assert_eq!(role, AgentRole::Auditor);
            }
            other => panic!("expected SpawnAgent(Auditor), got {other:?}"),
        }
    }

    #[test]
    fn next_action_doc_revision_spawns_scribe() {
        let ps = at_phase(PlanPhase::DocRevision);
        let action = PlanStateMachine::next_action(&ps);
        match action {
            Some(ExecutorAction::SpawnAgent { role, .. }) => {
                assert_eq!(role, AgentRole::Scribe);
            }
            other => panic!("expected SpawnAgent(Scribe), got {other:?}"),
        }
    }

    #[test]
    fn transition_error_display() {
        let err = TransitionError {
            from: PhaseKind::Queued,
            to: PhaseKind::Complete,
            reason: "illegal jump".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("Queued"));
        assert!(msg.contains("Complete"));
        assert!(msg.contains("illegal jump"));
    }

    #[test]
    fn doc_revision_can_go_to_done() {
        // DocRevision -> Done is in the valid_transitions table
        assert!(PlanPhase::DocRevision.can_transition_to(PhaseKind::Done));
    }
}
