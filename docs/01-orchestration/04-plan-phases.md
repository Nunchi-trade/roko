# Plan Phase Lifecycle

> **Module**: `roko-orchestrator/src/executor/state_machine.rs`
> **Key type**: `PlanStateMachine`
> **Phase type**: `roko_core::PlanPhase`

---

## Overview

Every plan in the Roko Orchestrator progresses through a defined sequence of
phases. The `PlanStateMachine` is the pure-logic core that governs these
transitions: given a `PlanState` and an `ExecutorEvent`, it computes the next
`PlanPhase` or rejects the transition as illegal.

The phase lifecycle encodes the entire plan execution workflow: enrichment,
implementation, quality gating, verification, code review, documentation, and
merge. It includes retry loops for gate failures and review rejections, with
bounded iteration counts to prevent infinite loops.

---

## Phase Definitions

### Active phases

| Phase | Description | Agent role |
|-------|-------------|------------|
| `Queued` | Plan is waiting in the execution queue | — |
| `Enriching` | Strategist agent is enriching the plan with context | Strategist |
| `Implementing` | Implementer agents are executing tasks | Implementer |
| `Gating` | Quality gates (compile, test, clippy) are running | — |
| `AutoFixing` | AutoFixer agent is fixing gate failures | AutoFixer |
| `Verifying` | Task-level verification commands are running | — |
| `RegeneratingVerify` | AutoFixer is regenerating failed verification code | AutoFixer |
| `Reviewing` | Auditor agent is reviewing the implementation | Auditor |
| `DocRevision` | Scribe agent is updating documentation | Scribe |
| `Merging` | Plan's worktree branch is being merged | — |
| `Done` | Plan has completed all phases, awaiting operator merge | — |

### Terminal phases

| Phase | Description |
|-------|-------------|
| `Complete` | Plan merged successfully |
| `Failed { reason }` | Plan failed terminally (with `FailureKind`) |
| `Skipped` | Plan was skipped by the operator |

Terminal phases emit no actions. Once a plan reaches a terminal phase, it stays
there permanently (within a single execution run).

---

## Phase Transition Diagram

```
                                         ┌──────────────────┐
                                         │      Queued       │
                                         └────────┬─────────┘
                                                  │ Start
                                                  ▼
                                         ┌──────────────────┐
                                         │    Enriching      │
                                         └────────┬─────────┘
                                                  │ EnrichmentDone
                                                  ▼
                                ┌───────────────────────────────────┐
                        ┌──────►│          Implementing              │◄──────┐
                        │       └────────────┬──────────────────────┘       │
                        │                    │ ImplementationDone           │
                        │                    ▼                              │
                        │       ┌──────────────────┐                        │
                        │       │     Gating        │◄────────┐             │
                        │       └───┬──────────┬───┘          │             │
                        │           │          │               │             │
                        │    GatePassed   GateFailed           │             │
                        │           │          │               │             │
                        │           │          ▼               │             │
                        │           │  ┌──────────────┐        │             │
                        │           │  │  AutoFixing   │       │             │
                        │           │  └──────┬───────┘        │             │
                        │           │         │ AutoFixDone    │             │
                        │           │         └────────────────┘             │
                        │           ▼                                        │
                        │  ┌──────────────────┐                              │
                        │  │    Verifying      │◄─────────┐                  │
                        │  └───┬──────────┬───┘           │                  │
                        │      │          │                │                  │
                        │ VerifyPassed  VerifyFailed       │                  │
                        │      │          │                │                  │
                        │      │          ▼                │                  │
                        │      │  ┌──────────────────┐     │                  │
                        │      │  │ RegeneratingVerify│     │                  │
                        │      │  └──────┬───────────┘     │                  │
                        │      │         │ VerifyRegenDone │                  │
                        │      │         └─────────────────┘                  │
                        │      ▼                                              │
                        │  ┌──────────────────┐                              │
                        │  │    Reviewing      │──── ReviewRejected ─────────┘
                        │  └────────┬─────────┘
                        │           │ ReviewApproved
                        │           ▼
                        │  ┌──────────────────┐
                        │  │   DocRevision     │
                        │  └────────┬─────────┘
                        │           │ DocRevisionDone
                        │           ▼
                        │  ┌──────────────────┐
                        │  │     Merging       │
                        │  └───┬──────────┬───┘
                        │      │          │
                        │ MergeSucceeded  MergeFailed
                        │      │          │
                        │      ▼          ▼
                        │  ┌────────┐  ┌────────┐
                        │  │Complete│  │ Failed  │
                        │  └────────┘  └────────┘
                        │
                        │  (Skip from any non-terminal → Skipped)
                        │  (Fatal from any non-terminal → Failed)
                        └──────────────────────────────────
```

---

## Transition Rules

### The `transition()` method

```rust
pub fn transition(
    plan_state: &PlanState,
    event: &ExecutorEvent,
) -> Result<PlanPhase, TransitionError>
```

This method:

1. Reads the plan's current phase
2. Matches (current_phase, event) to compute the next phase
3. Validates the transition against `roko_core::valid_transitions()` — a
   canonical transition table that defines all legal phase-to-phase moves
4. Returns the new phase or a `TransitionError`

### Legal transitions

| From | Event | To | Notes |
|------|-------|----|-------|
| Queued | Start | Enriching | Plan begins execution |
| Queued | Skip | Skipped | Operator skip |
| Enriching | EnrichmentDone | Implementing | Context enrichment complete |
| Implementing | ImplementationDone | Gating | All tasks in current iteration done |
| Gating | GatePassed | Verifying | All gates passed |
| Gating | GateFailed (iteration < 5) | AutoFixing | Gate failed, retry available |
| Gating | GateFailed (iteration ≥ 5) | Failed(AutoFixExhausted) | Max auto-fix iterations reached |
| AutoFixing | AutoFixDone | Gating | Fix applied, re-run gates |
| Verifying | VerifyPassed | Reviewing | Verification passed |
| Verifying | VerifyFailed | RegeneratingVerify | Verification failed |
| RegeneratingVerify | VerifyRegenDone | Verifying | Regeneration complete, re-verify |
| Reviewing | ReviewApproved | DocRevision | Auditor approved |
| Reviewing | ReviewRejected | Implementing | Auditor rejected, reimpl |
| DocRevision | DocRevisionDone | Merging | Docs updated, merge |
| Merging | MergeSucceeded | Complete | Success |
| Merging | MergeFailed (attempts < 3) | Failed | Merge conflict |
| Merging | MergeFailed (attempts ≥ 3) | Failed(Deadlock) | Deadlock |
| Done | OperatorMerge | Merging | Operator triggers merge |
| Any non-terminal | Skip | Skipped | Operator skip |
| Any non-terminal | Fatal(reason) | Failed(Other(reason)) | Crash |

### Bounded retry loops

Two retry loops have explicit bounds:

1. **Auto-fix loop**: `Gating → AutoFixing → Gating`. Maximum
   `MAX_AUTO_FIX_ITERATIONS` (5) iterations. After 5 failed gate cycles, the
   plan transitions to `Failed { reason: AutoFixExhausted }`.

2. **Merge retry**: `Merging → Failed`. Maximum `MAX_MERGE_ATTEMPTS` (3)
   attempts. After 3 failed merges, the plan transitions to
   `Failed { reason: Deadlock }`.

These bounds prevent infinite loops. The values are compile-time constants,
not configurable — they represent hard safety limits.

---

## Failure Types

```rust
pub enum FailureKind {
    AutoFixExhausted,           // 5 gate-fix cycles without passing
    Deadlock,                   // 3 merge attempts without success
    Other(String),              // arbitrary failure reason
}
```

`FailureKind` is part of the `PlanPhase::Failed { reason }` variant. It is
serialized into executor snapshots and event logs, enabling post-mortem
analysis of why plans failed.

---

## The `next_action()` Method

```rust
pub fn next_action(plan_state: &PlanState) -> Option<ExecutorAction>
```

Given a plan's current state, `next_action()` suggests what the runtime should
do next. Returns `None` if:

- The plan is paused (`plan_state.paused == true`)
- The plan is in a terminal phase (`Complete`, `Failed`, `Skipped`)
- The plan is in a phase that waits for external input (no proactive action)

The action suggestions correspond to the phase-to-action mapping defined in
`03-parallel-executor.md`. The runtime harness uses this to determine what
to dispatch.

---

## TransitionError

```rust
pub struct TransitionError {
    pub from: PhaseKind,    // the phase the plan was in
    pub to: PhaseKind,      // the phase the caller tried to reach
    pub reason: String,     // human-readable explanation
}
```

Transition errors are informational, not recoverable. If the state machine
rejects a transition, it means the runtime attempted an illegal operation
(e.g., trying to pass a gate when the plan is still `Queued`). This indicates
a bug in the runtime harness, not a normal failure mode.

---

## Mapping from the Original Mori Pipeline

The Mori orchestrator (`bardo-backup/prd/25-mori/`) defined pipeline phases:

```
Preflight → Strategist → Implementer → Gates → Review → Verdict
```

These map to the Roko phase lifecycle as follows
(`refactoring-prd/08-translation-guide.md`):

| Mori Phase | Roko Phase | Notes |
|-----------|-----------|-------|
| Preflight | Enriching | Context gathering and plan validation |
| Strategist | Enriching | Merged into enrichment |
| Implementer | Implementing | Agent task execution |
| Gates | Gating + Verifying | Split into gate ladder and verification |
| Review | Reviewing | Auditor review |
| Verdict | DocRevision + Merging | Split into doc update and merge |

The Roko lifecycle is more granular: it separates verification from gating,
adds an explicit documentation revision phase, and includes the auto-fix and
verify-regeneration retry loops that Mori handled informally.

---

## Test Coverage

The state machine has comprehensive tests:

- **Happy path full lifecycle**: Queued → Enriching → Implementing → Gating →
  Verifying → Reviewing → DocRevision → Merging → Complete
- **Auto-fix loop**: Gate failure enters AutoFixing; AutoFix returns to Gating
- **Max auto-fix iterations**: Exhaustion leads to Failed
- **Verify regeneration loop**: Verify failure enters RegeneratingVerify
- **Review rejection**: Returns to Implementing
- **Skip from any phase**: Any non-terminal phase can be skipped
- **Fatal from any phase**: Any non-terminal phase can transition to Failed
- **Illegal transitions**: Cannot start from Implementing, cannot gate-pass
  from Queued, cannot transition from Complete
- **Merge failure**: Both with retries and with deadlock detection
- **Done to Merging**: Operator merge trigger
- **next_action correctness**: Each phase maps to the correct action
- **Paused plans**: Return None from next_action
- **Terminal plans**: Return None from next_action

---

## References

- Finite state machines as a modeling tool for workflow systems: van der Aalst,
  W. M. P. (1998). The application of Petri nets to workflow management.
  *Journal of Circuits, Systems and Computers*, 8(1), 21–66.
- The retry loop pattern with bounded iterations follows the "circuit breaker"
  pattern from distributed systems (Nygard, M. T. (2007). *Release It!*.
  Pragmatic Bookshelf).
- Phase transitions correspond to the "state machine replication" pattern used
  in consensus protocols, where deterministic state machines process events
  identically across replicas.
