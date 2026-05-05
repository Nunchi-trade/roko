# Plan Phases and Actions

> Depth for [03-GRAPH.md](../../unified/03-GRAPH.md). The lifecycle phases of a plan expressed as a type-state machine, and the action vocabulary that connects the pure executor to the effectful runtime.

---

## What This Document Covers

Every plan in the orchestrator progresses through a defined sequence of phases: enrichment, implementation, quality gating, verification, review, documentation, and merge. This document covers:

1. **The phase lifecycle** as a type-state machine (Cell transitions)
2. **The action vocabulary** -- the side-effects the executor can request
3. **The runtime harness** -- how actions become real effects
4. **Bounded retry loops** -- how the system prevents infinite cycling

The separation between "what should happen" (executor) and "how it happens" (runtime) is the central design principle.

---

## Phase Lifecycle as a Type-State Machine

Each plan is a Cell that transitions through phases. The phase determines what the Cell can do next, what events it accepts, and what actions it emits. In Rust type-state terms, each phase is a distinct state with its own valid transition set.

### Active Phases

| Phase | What happens | Agent role | Verify protocol? |
|---|---|---|---|
| `Queued` | Plan waits in the execution queue | -- | -- |
| `Enriching` | Strategist gathers context, validates plan structure | Strategist | -- |
| `Implementing` | Implementer agents execute tasks from the DAG | Implementer | -- |
| `Gating` | Quality gates (compile, test, clippy) run as a Verify Pipeline | -- | Yes |
| `AutoFixing` | AutoFixer agent addresses gate failures | AutoFixer | -- |
| `Verifying` | Task-level verification commands run | -- | Yes |
| `RegeneratingVerify` | AutoFixer regenerates failed verification code | AutoFixer | -- |
| `Reviewing` | Auditor agent reviews implementation for correctness | Auditor | Implicit |
| `DocRevision` | Scribe agent updates documentation | Scribe | -- |
| `Merging` | Plan's worktree branch is merged via the merge queue | -- | -- |
| `Done` | All phases complete; awaiting operator merge trigger | -- | -- |

### Terminal Phases

| Phase | Meaning |
|---|---|
| `Complete` | Plan merged successfully. Permanent. |
| `Failed { reason: FailureKind }` | Plan failed terminally. Includes failure classification. |
| `Skipped` | Plan was skipped by the operator. |

Terminal phases emit no actions. Once reached, a plan stays there permanently within a single execution run.

### Failure Classification

```rust
enum FailureKind {
    AutoFixExhausted,  // 5 gate-fix cycles without passing
    Deadlock,          // 3 merge attempts without success
    Other(String),     // arbitrary failure reason (budget, timeout, crash)
}
```

`FailureKind` is serialized into executor snapshots and event logs, enabling post-mortem analysis.

---

## The Transition Graph

The phase lifecycle forms a directed graph with two retry Loops and multiple terminal exits:

```
Queued
  | Start
  v
Enriching
  | EnrichmentDone
  v
Implementing  <---------+  <-------- ReviewRejected
  | ImplementationDone   |
  v                      |
Gating  <-------+        |
  |              |        |
  +--GatePassed  |        |
  |  |           |        |
  |  v           |        |
  |  Verifying   |        |
  |    |         |        |
  |    +--Passed |        |
  |    |  |      |        |
  |    |  v      |        |
  |    |  Reviewing       |
  |    |    |             |
  |    |    +--Approved   |
  |    |    |  |          |
  |    |    |  v          |
  |    |    |  DocRevision|
  |    |    |    |        |
  |    |    |    | Done   |
  |    |    |    v        |
  |    |    |  Merging    |
  |    |    |    |    |   |
  |    |    |    OK  Fail |
  |    |    |    |    |   |
  |    |    |    v    v   |
  |    |    | Complete Failed
  |    |    |             |
  |    |    +--Rejected---+
  |    |
  |    +--Failed
  |       |
  |       v
  |    RegeneratingVerify
  |       | RegenDone
  |       +---> (back to Verifying)
  |
  +--GateFailed (iteration < 5)
  |  |
  |  v
  |  AutoFixing
  |    | AutoFixDone
  |    +---> (back to Gating)
  |
  +--GateFailed (iteration >= 5)
     |
     v
     Failed { AutoFixExhausted }
```

### Transition Table

The `PlanStateMachine::transition()` method is the canonical reference. It matches `(current_phase, event)` -> `next_phase`:

| From | Event | To | Condition |
|---|---|---|---|
| Queued | Start | Enriching | Within concurrent plan limit |
| Queued | Skip | Skipped | -- |
| Enriching | EnrichmentDone | Implementing | -- |
| Implementing | ImplementationDone | Gating | All tasks in current iteration done |
| Gating | GatePassed | Verifying | All gates passed |
| Gating | GateFailed | AutoFixing | `iteration < max_auto_fix_iterations` (5) |
| Gating | GateFailed | Failed(AutoFixExhausted) | `iteration >= 5` |
| AutoFixing | AutoFixDone | Gating | Increments iteration counter |
| Verifying | VerifyPassed | Reviewing | -- |
| Verifying | VerifyFailed | RegeneratingVerify | -- |
| RegeneratingVerify | VerifyRegenDone | Verifying | -- |
| Reviewing | ReviewApproved | DocRevision | -- |
| Reviewing | ReviewRejected | Implementing | Resets for new implementation |
| DocRevision | DocRevisionDone | Merging | -- |
| Done | OperatorMerge | Merging | Operator triggers merge |
| Merging | MergeSucceeded | Complete | -- |
| Merging | MergeFailed | Failed | `merge_attempts >= max_merge_attempts` (3) |
| Any non-terminal | Skip | Skipped | Operator override |
| Any non-terminal | Fatal(reason) | Failed(Other(reason)) | Crash or unrecoverable error |

### Illegal Transitions

The state machine rejects any transition not in the table above. If the runtime attempts an illegal operation (e.g., gate-pass from Queued), it gets a `TransitionError`:

```rust
struct TransitionError {
    from: PhaseKind,   // phase the plan was in
    to: PhaseKind,     // phase the caller tried to reach
    reason: String,    // human-readable explanation
}
```

This indicates a bug in the runtime harness, not a normal failure mode.

---

## Bounded Retry Loops

Two retry Loops have explicit bounds to prevent infinite cycling:

### Auto-Fix Loop

```
Gating -> AutoFixing -> Gating -> AutoFixing -> ... (max 5 iterations)
```

When a gate fails, the AutoFixer agent attempts to fix the issues. On each cycle, `iteration` is incremented. After 5 failed gate cycles, the plan transitions to `Failed { reason: AutoFixExhausted }`.

The bound is a compile-time constant, not configurable -- it represents a hard safety limit. If 5 attempts with different fix strategies cannot pass the gates, the task needs human intervention.

### Merge Retry

```
Merging -> MergeFailed -> ... (max 3 attempts)
```

After 3 failed merges, the plan transitions to `Failed { reason: Deadlock }`. Merge failures typically indicate persistent git conflicts that require manual resolution.

### Why These Specific Bounds?

5 auto-fix iterations is empirically derived from mori experience: most gate failures resolve within 2-3 attempts. Beyond 5, the agent is likely making the same mistake repeatedly. 3 merge attempts follows the distributed systems convention for retry limits (Nygard 2007, *Release It!*).

---

## The Action Vocabulary

`ExecutorAction` is the vocabulary of side-effects the executor can request. Each action is a Signal emitted by the executor and consumed by the runtime.

### DispatchPlan

```rust
DispatchPlan { plan_id: String }
```

Begins plan execution. The runtime creates a git worktree, parses `tasks.toml`, initializes a `TaskTracker`, and fires `ExecutorEvent::Start`.

### SpawnAgent

```rust
SpawnAgent { plan_id: String, role: AgentRole, task: String }
```

Launches an agent process. The runtime builds an `AgentRunConfig` with:

1. **Model selection** via `CascadeRouter` (LinUCB + anomaly detection). Considers: task complexity, agent role, iteration count, prior gate failures, crate familiarity, Daimon affect confidence.
2. **System prompt** via `RoleSystemPromptSpec` (6-layer builder): core identity, role instructions, plan context, learned context, feedback context, operating constraints.
3. **Tool permissions** per role (Implementer gets file tools; Auditor gets read-only tools).
4. **MCP config** passthrough from `roko.toml`.
5. **Environment variables** (plan ID, task ID, worktree path).

Agent role determines behavior:

| Role | Phase | Purpose | Model tier |
|---|---|---|---|
| Strategist | Enriching | Context gathering, plan validation | T1 (fast) |
| Implementer | Implementing | Execute a single task | T1-T2 (complexity-dependent) |
| AutoFixer | AutoFixing | Fix compilation/test failures | T2 (needs reasoning) |
| AutoFixer | RegeneratingVerify | Regenerate verification code | T1-T2 |
| Auditor | Reviewing | Review implementation correctness | T2 (needs judgment) |
| Scribe | DocRevision | Update documentation | T1 (formulaic) |

### RunGate

```rust
RunGate { plan_id: String, rung: u32 }
```

Executes a gate in the Verify Pipeline:

| Rung | Gate Cell | Command | What it checks |
|---|---|---|---|
| 0 | CompileGate | `cargo build --workspace` | Compilation passes |
| 1 | TestGate | `cargo test --workspace` | Tests pass |
| 2 | ClippyGate | `cargo clippy --workspace --no-deps -- -D warnings` | No lint warnings |

Gate results accumulate as `GateResult` entries on the PlanState. The runtime also feeds gate pass rates into `AdaptiveThresholds` (EMA tracking) for the learning Loop.

### RunVerify

```rust
RunVerify { plan_id: String }
```

Runs task-level verification commands declared in `tasks.toml` via the `verify` field. These are custom checks that test the specific behavior a task was supposed to implement.

### MergeBranch

```rust
MergeBranch { plan_id: String }
```

Enqueues the plan in the `MergeQueue`, which serializes merges to prevent concurrent git conflicts. The merge queue checks for file conflicts with other in-flight merges before proceeding.

### FailPlan / CompletePlan

Terminal actions. `FailPlan` transitions to `Failed`, `CompletePlan` transitions to `Complete`. The runtime records final cost and efficiency metrics on completion.

### PausePlan / ResumePlan

Toggle `plan_state.paused`. Paused plans emit no actions from `tick()`. State is preserved for later resumption.

### Reorder

```rust
Reorder { plan_id: String, new_position: usize }
```

Moves a plan in the execution queue. Used for dynamic priority adjustment by the conductor or operator.

---

## The Runtime Harness: PlanRunner

The `PlanRunner` is the effectful half that dispatches actions to real subsystems. It owns 30+ fields spanning every cognitive subsystem:

### Core Loop

```rust
loop {
    let actions = self.executor.tick();
    for action in actions {
        match action {
            DispatchPlan { plan_id } => self.dispatch_plan(&plan_id).await?,
            SpawnAgent { plan_id, role, task } => self.spawn_agent(&plan_id, role, &task).await?,
            RunGate { plan_id, rung } => self.run_gate(&plan_id, rung).await?,
            MergeBranch { plan_id } => self.merge_branch(&plan_id).await?,
            // ...
        }
        self.maybe_autosave().await?;  // every 5 actions
    }
    if self.all_plans_terminal() { break; }
}
```

### Subsystems Integrated

The runtime harness integrates every cognitive cross-cut:

**Neuro (knowledge):** `KnowledgeStore` queried per-task for scoped context. Successful patterns distilled into knowledge entries.

**Daimon (affect):** `DaimonState` modulates dispatch -- arousal influences task prioritization, confidence affects model selection.

**Learning:** `LearningRuntime` records efficiency events, episode logs, model routing feedback. `CrateFamiliarityTracker` tracks per-crate success rates. `ContextAttributionTracker` measures which context types agents actually use.

**Conductor (anomaly detection):** Background `WatcherRunner` tails signal logs every 30 seconds, running 10 watchers (cost overrun, context pressure, silence, ghost turns). Alert Signals persist back for the orchestrator to act on.

### Auto-Save

The executor snapshot saves every 5 actions via atomic write (write-to-temp + rename). At most 5 actions of work can be lost in a crash.

### Reporting

`PlanRunner::run()` returns an `OrchestrationReport`:

```rust
struct OrchestrationReport {
    plans: Vec<PlanRunReport>,    // per-plan results
    total_agent_calls: usize,     // total agent dispatches
    total_gate_runs: usize,       // total gate executions
    fleet_cfactor: Option<FleetCFactor>,  // collective intelligence metric
}
```

The fleet c-factor (Woolley et al. 2010) measures how much better the multi-agent system performs compared to the sum of individual agents.

---

## Mapping from Mori

The Roko phase lifecycle refines the original Mori pipeline:

| Mori phase | Roko phase(s) | What changed |
|---|---|---|
| Preflight | Enriching | Context gathering and plan validation |
| Strategist | Enriching | Merged into enrichment (same agent) |
| Implementer | Implementing | Agent task execution (unchanged) |
| Gates | Gating + Verifying | Split: gates verify compilation, verify checks task-specific behavior |
| Review | Reviewing | Auditor review (unchanged) |
| Verdict | DocRevision + Merging | Split: explicit doc update phase before merge |

The Roko lifecycle is more granular: it separates verification from gating, adds an explicit documentation revision phase, and formalizes the auto-fix and verify-regeneration retry Loops that Mori handled informally.

---

## Reality Check: Implementation vs. Spec

**MergeBranch auto-advances.** The current event loop auto-succeeds `MergeBranch` with no actual git merge:

```rust
ExecutorAction::MergeBranch { plan_id } => {
    info!(plan_id = %plan_id, "auto-advancing merge");
    let _ = ctx.executor.apply_event(plan_id, &ExecutorEvent::MergeSucceeded);
}
```

Multiple plans merging simultaneously would risk git conflicts. No serialization or conflict detection.

**RunVerify auto-passes.** The `RunVerify` action is implemented but the verification commands from `tasks.toml` are not actually executed. It transitions directly to `VerifyPassed`.

**No failure classification.** Gate failures are binary. The `GateFailureClassification` / `FailureClass` / `classify_gate_failure` types exist in `roko-gate` but are never consulted at dispatch time. All failures get the same treatment.

**No retry backoff.** When a gate fails, the executor transitions immediately to `AutoFixing` and re-runs gates as fast as the tick interval (100ms). No exponential backoff, no jitter, no cooldown.

**What IS wired:** The phase state machine, all transitions, bounded retry loops (5 auto-fix, 3 merge), the action/event vocabulary, `SpawnAgent` with full model selection and prompt assembly, `RunGate` with the 3-rung Verify Pipeline, auto-save, and reporting.

---

## What This Enables

1. **Auditable execution**: Every phase transition is an explicit event. The hash-chained event log provides tamper-evident audit trail.
2. **Bounded failure recovery**: Retry loops with hard bounds prevent infinite cycling. The system always terminates.
3. **Cognitive agent pipeline**: Each plan phase uses the right agent role with the right model tier. The system doesn't use an expensive model for documentation updates.
4. **Operator override**: `Skip`, `Pause`, `Resume`, and `Reorder` provide fine-grained human control without breaking the state machine.

## Feedback Loops

- **Loop: ModelEscalation** -- gate failures cause iteration count to increase, which the `CascadeRouter` reads to escalate model tier. Repeated failure -> better model -> higher cost per attempt but higher chance of success.
- **Loop: GateFailureReplan** -- if `learning_config.replan_on_gate_failure` is enabled and failures accumulate past a threshold, the task list is regenerated from the PRD. Completed tasks are preserved.
- **Lens: PhaseDistribution** -- observes how long plans spend in each phase. Identifies bottlenecks (e.g., plans stuck in `Gating` suggest gate infrastructure problems; plans stuck in `Reviewing` suggest the Auditor is too strict).

## Open Questions

1. **Review calibration.** The Auditor can reject implementations, sending them back to Implementing. But what prevents an overly strict Auditor from creating an infinite rejection loop? Currently bounded only by the global plan timeout (if implemented) or operator Skip.
2. **Phase skip granularity.** `Skip` sends a plan directly to `Skipped`. Should there be a way to skip individual phases (e.g., skip verification but keep review)?
3. **Cross-plan phase coordination.** Two plans modifying the same crate should ideally coordinate their `Gating` phases to avoid cargo lock contention. The gate semaphore addresses this at the resource level, but phase-level coordination could be more efficient.
4. **Re-enrichment.** If a plan's context changes significantly during execution (e.g., another plan merges changes to the same crate), should the plan re-enter `Enriching`? Currently there is no mechanism for phase regression beyond the defined retry loops.
