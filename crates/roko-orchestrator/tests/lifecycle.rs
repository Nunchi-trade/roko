//! Full lifecycle integration test (I.2.8).
//!
//! Verifies that a plan transitions through the complete happy-path
//! lifecycle via the `ParallelExecutor`, and that the audit chain
//! records an entry for each transition.

#![allow(clippy::unwrap_used)]

use roko_core::{PhaseKind, PlanPhase};
use roko_orchestrator::safety::audit_chain::AuditChain;
use roko_orchestrator::{
    ExecutorAction, ExecutorConfig, ExecutorEvent, ParallelExecutor, PlanState,
};

/// Full lifecycle: Queued -> Enriching -> Implementing -> Gating ->
/// Verifying -> Reviewing -> DocRevision -> Merging -> Complete.
///
/// Uses `apply_event()` at each step, checks the final state is
/// `Complete`, and verifies the audit chain has one entry per
/// transition.
#[test]
fn full_lifecycle_with_audit_chain() {
    let chain = AuditChain::new();
    let mut ex = ParallelExecutor::new(ExecutorConfig::default())
        .with_audit_chain(chain.clone());

    ex.add_plan(PlanState::new("lifecycle-1"));

    // Tick should suggest DispatchPlan for the queued plan.
    let actions = ex.tick();
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        &actions[0],
        ExecutorAction::DispatchPlan { plan_id } if plan_id == "lifecycle-1"
    ));

    // 1. Queued -> Enriching
    let phase = ex.apply_event("lifecycle-1", &ExecutorEvent::Start).unwrap();
    assert_eq!(phase.kind(), PhaseKind::Enriching);

    // 2. Enriching -> Implementing
    let phase = ex
        .apply_event("lifecycle-1", &ExecutorEvent::EnrichmentDone)
        .unwrap();
    assert_eq!(phase.kind(), PhaseKind::Implementing);

    // 3. Implementing -> Gating
    let phase = ex
        .apply_event("lifecycle-1", &ExecutorEvent::ImplementationDone)
        .unwrap();
    assert_eq!(phase.kind(), PhaseKind::Gating);

    // 4. Gating -> Verifying
    let phase = ex
        .apply_event("lifecycle-1", &ExecutorEvent::GatePassed)
        .unwrap();
    assert_eq!(phase.kind(), PhaseKind::Verifying);

    // 5. Verifying -> Reviewing
    let phase = ex
        .apply_event("lifecycle-1", &ExecutorEvent::VerifyPassed)
        .unwrap();
    assert_eq!(phase.kind(), PhaseKind::Reviewing);

    // 6. Reviewing -> DocRevision
    let phase = ex
        .apply_event("lifecycle-1", &ExecutorEvent::ReviewApproved)
        .unwrap();
    assert_eq!(phase.kind(), PhaseKind::DocRevision);

    // 7. DocRevision -> Merging
    let phase = ex
        .apply_event("lifecycle-1", &ExecutorEvent::DocRevisionDone)
        .unwrap();
    assert_eq!(phase.kind(), PhaseKind::Merging);

    // 8. Merging -> Complete
    let phase = ex
        .apply_event("lifecycle-1", &ExecutorEvent::MergeSucceeded)
        .unwrap();
    assert_eq!(phase, PlanPhase::Complete);

    // Final state is Complete.
    let state = ex.plan_state("lifecycle-1").unwrap();
    assert!(state.is_terminal());
    assert_eq!(state.current_phase, PlanPhase::Complete);

    // Audit chain has exactly 8 entries (one per transition).
    let audit = ex.audit_chain().unwrap();
    assert_eq!(audit.len(), 8, "expected 8 audit entries, got {}", audit.len());
    assert!(audit.verify(), "audit chain must be valid");

    // All entries should reference the plan_id as the resource.
    let entries = audit.iter();
    for entry in &entries {
        assert_eq!(entry.resource, "lifecycle-1");
        assert_eq!(entry.actor, "executor");
        assert!(entry.kind.starts_with("phase."));
    }

    // Completed plans list contains our plan.
    assert!(ex.completed_plans().contains(&"lifecycle-1".to_string()));
}

/// Verify executor works correctly without an audit chain (default behaviour).
#[test]
fn lifecycle_without_audit_chain() {
    let mut ex = ParallelExecutor::new(ExecutorConfig::default());
    ex.add_plan(PlanState::new("no-audit"));

    ex.apply_event("no-audit", &ExecutorEvent::Start).unwrap();
    ex.apply_event("no-audit", &ExecutorEvent::EnrichmentDone)
        .unwrap();
    ex.apply_event("no-audit", &ExecutorEvent::ImplementationDone)
        .unwrap();
    ex.apply_event("no-audit", &ExecutorEvent::GatePassed)
        .unwrap();
    ex.apply_event("no-audit", &ExecutorEvent::VerifyPassed)
        .unwrap();
    ex.apply_event("no-audit", &ExecutorEvent::ReviewApproved)
        .unwrap();
    ex.apply_event("no-audit", &ExecutorEvent::DocRevisionDone)
        .unwrap();
    ex.apply_event("no-audit", &ExecutorEvent::MergeSucceeded)
        .unwrap();

    assert!(ex.plan_state("no-audit").unwrap().is_terminal());
    assert!(ex.audit_chain().is_none());
}

/// Multiple plans go through the lifecycle in parallel, audit chain
/// tracks all of them.
#[test]
fn multi_plan_lifecycle_with_audit_chain() {
    let chain = AuditChain::new();
    let mut ex = ParallelExecutor::new(ExecutorConfig::default())
        .with_audit_chain(chain.clone());

    ex.add_plan(PlanState::new("plan-a"));
    ex.add_plan(PlanState::new("plan-b"));

    // Advance plan-a fully.
    for event in &[
        ExecutorEvent::Start,
        ExecutorEvent::EnrichmentDone,
        ExecutorEvent::ImplementationDone,
        ExecutorEvent::GatePassed,
        ExecutorEvent::VerifyPassed,
        ExecutorEvent::ReviewApproved,
        ExecutorEvent::DocRevisionDone,
        ExecutorEvent::MergeSucceeded,
    ] {
        ex.apply_event("plan-a", event).unwrap();
    }

    // Advance plan-b partially (to Gating).
    for event in &[
        ExecutorEvent::Start,
        ExecutorEvent::EnrichmentDone,
        ExecutorEvent::ImplementationDone,
    ] {
        ex.apply_event("plan-b", event).unwrap();
    }

    assert_eq!(
        ex.plan_state("plan-a").unwrap().current_phase,
        PlanPhase::Complete
    );
    assert_eq!(
        ex.plan_state("plan-b").unwrap().current_phase.kind(),
        PhaseKind::Gating
    );

    // 8 from plan-a + 3 from plan-b = 11.
    assert_eq!(chain.len(), 11);
    assert!(chain.verify());
}
