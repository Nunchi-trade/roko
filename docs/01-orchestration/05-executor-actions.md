# Executor Actions

> **Module**: `roko-orchestrator/src/executor/action.rs`
> **Key type**: `ExecutorAction`
> **Consumed by**: `PlanRunner` in `roko-cli/src/orchestrate.rs`

---

## Overview

`ExecutorAction` is the vocabulary of side-effects that the pure state machine
can request. Each call to `ParallelExecutor::tick()` returns a
`Vec<ExecutorAction>`. The runtime harness is responsible for dispatching each
action to the appropriate subsystem and feeding results back as events.

Actions are *requests*, not effects. The executor never performs I/O itself.
This separation is what makes the orchestrator testable, serializable, and
crash-recoverable.

---

## Action Variants

### DispatchPlan

```rust
DispatchPlan { plan_id: String }
```

**Trigger**: Plan is `Queued` and within the concurrent plan limit.

**Runtime effect**: The runtime:
1. Creates a git worktree for the plan via `WorktreeManager::create_for_plan()`
2. Parses `tasks.toml` in the plan directory
3. Initializes a `TaskTracker` for per-task progress
4. Transitions the plan to `Enriching` via `apply_event(Start)`

**Success event**: `ExecutorEvent::Start`

---

### SpawnAgent

```rust
SpawnAgent {
    plan_id: String,
    role: AgentRole,
    task: String,
}
```

**Trigger**: Plan is in a phase that requires an agent (Enriching, Implementing,
AutoFixing, RegeneratingVerify, Reviewing, DocRevision).

**Runtime effect**: The runtime:
1. Builds an `AgentRunConfig` with role-specific parameters:
   - System prompt from `RoleSystemPromptSpec` (6-layer prompt builder)
   - Model from `CascadeRouter` (LinUCB + anomaly detection)
   - Tool permissions per role
   - MCP config passthrough
   - Environment variables (plan ID, task ID, worktree path)
2. Launches the agent via `ClaudeCliAgent` or `ExecAgent`
3. Records the agent process in `ProcessSupervisor`
4. Logs an efficiency event on completion

**Success event**: Depends on the role:
- Strategist → `ExecutorEvent::EnrichmentDone`
- Implementer → `ExecutorEvent::ImplementationDone` (when all tasks done)
- AutoFixer → `ExecutorEvent::AutoFixDone`
- AutoFixer (regen-verify) → `ExecutorEvent::VerifyRegenDone`
- Auditor → `ExecutorEvent::ReviewApproved` or `ReviewRejected`
- Scribe → `ExecutorEvent::DocRevisionDone`

### Agent roles

| Role | Phase | Purpose |
|------|-------|---------|
| `Strategist` | Enriching | Enriches the plan with context, validates structure |
| `Implementer` | Implementing | Executes a single task from the plan |
| `AutoFixer` | AutoFixing | Fixes compilation/test failures from gate results |
| `AutoFixer` | RegeneratingVerify | Regenerates verification code |
| `Auditor` | Reviewing | Reviews the implementation for correctness |
| `Scribe` | DocRevision | Updates documentation to reflect changes |

Each role receives a different system prompt, tool set, and model tier.
The `RoleSystemPromptSpec` builds 6-layer prompts with:

1. Core identity and capabilities
2. Role-specific instructions
3. Plan context (PRD, task description)
4. Learned context (skills, playbooks, knowledge)
5. Feedback context (gate failures, review feedback)
6. Operating constraints (budget, timeout, tool restrictions)

---

### RunGate

```rust
RunGate { plan_id: String, rung: u32 }
```

**Trigger**: Plan is in `Gating` phase.

**Runtime effect**: The runtime executes the gate at the specified rung in the
plan's worktree:

| Rung | Gate | What it checks |
|------|------|----------------|
| 0 | `CompileGate` | `cargo build --workspace` passes |
| 1 | `TestGate` | `cargo test --workspace` passes |
| 2 | `ClippyGate` | `cargo clippy --workspace --no-deps -- -D warnings` passes |

Gate results are recorded as `GateResult` on the plan's `PlanState` and
logged as `EventKind::GateResult` in the event log.

**Success event**: `ExecutorEvent::GatePassed` (all rungs pass) or
`ExecutorEvent::GateFailed` (any rung fails)

### Adaptive gate thresholds

The runtime uses `AdaptiveThresholds` to track per-rung pass rates via
exponential moving average (EMA). This data feeds into the learning subsystem
for retry budget decisions — if a gate consistently fails for a particular
kind of task, the system can adjust model routing or task decomposition.

---

### RunVerify

```rust
RunVerify { plan_id: String }
```

**Trigger**: Plan is in `Verifying` phase (all gates passed).

**Runtime effect**: The runtime executes task-level verification commands
declared in `tasks.toml` via the `verify` field. These are custom commands
that test the specific behavior the task was supposed to implement.

**Success event**: `ExecutorEvent::VerifyPassed` or `ExecutorEvent::VerifyFailed`

---

### MergeBranch

```rust
MergeBranch { plan_id: String }
```

**Trigger**: Plan is in `Merging` phase.

**Runtime effect**: The runtime:
1. Enqueues the plan in the `MergeQueue`
2. The merge queue checks for file conflicts with other in-flight merges
3. If no conflicts, merges the plan's worktree branch into the batch branch
4. If conflicts, waits or retries

**Success event**: `ExecutorEvent::MergeSucceeded` or `ExecutorEvent::MergeFailed`

See `08-merge-queue.md` for the full merge serialization protocol.

---

### FailPlan

```rust
FailPlan { plan_id: String, reason: String }
```

**Trigger**: Unrecoverable failure detected by the runtime.

**Runtime effect**: The runtime transitions the plan to
`PlanPhase::Failed { reason: FailureKind::Other(reason) }`.

---

### CompletePlan

```rust
CompletePlan { plan_id: String }
```

**Trigger**: Plan has merged successfully.

**Runtime effect**: The plan transitions to `PlanPhase::Complete`. The runtime:
1. Records a `PlanCompleted` event in the event log
2. Updates the executor snapshot
3. Cleans up the plan's worktree (if configured)
4. Records final cost and efficiency metrics

---

### PausePlan / ResumePlan

```rust
PausePlan { plan_id: String }
ResumePlan { plan_id: String }
```

**Trigger**: Resource contention, operator intervention, or budget constraints.

**Runtime effect**: Sets `plan_state.paused = true` (or `false`). Paused plans
do not emit actions from `tick()`. Their state is preserved — they can be
resumed later without loss of progress.

---

### Reorder

```rust
Reorder { plan_id: String, new_position: usize }
```

**Trigger**: Dynamic priority adjustment by the conductor or operator.

**Runtime effect**: Moves the plan to a new position in the execution queue.
Plans at lower positions execute first (when priority is equal).

---

## Serialization

All `ExecutorAction` variants implement `Serialize + Deserialize`. Actions are
serialized in event logs, executor snapshots, and debug traces. The
serialization format uses tagged enums:

```json
{
  "SpawnAgent": {
    "plan_id": "01-workspace",
    "role": "implementer",
    "task": "t1"
  }
}
```

### Display formatting

`ExecutorAction` implements `Display` for human-readable logging:

```
dispatch(01-workspace)
spawn(01-workspace, implementer, t1)
gate(01-workspace, rung=0)
verify(01-workspace)
merge(01-workspace)
fail(01-workspace: compilation errors)
complete(01-workspace)
reorder(01-workspace -> 3)
pause(01-workspace)
resume(01-workspace)
```

---

## Action Flow

The complete action flow from state machine to side effect:

```
ParallelExecutor::tick()
  │
  ├─► PlanStateMachine::next_action(plan_state)
  │     Returns Option<ExecutorAction>
  │
  └─► Vec<ExecutorAction>  ──►  PlanRunner dispatch loop
                                  │
                                  ├─► SpawnAgent  ──►  ClaudeCliAgent / ExecAgent
                                  ├─► RunGate     ──►  CompileGate / TestGate / ClippyGate
                                  ├─► MergeBranch ──►  MergeQueue → git merge
                                  ├─► DispatchPlan──►  WorktreeManager + TaskTracker
                                  └─► etc.
                                  │
                                  ▼
                            ExecutorEvent  ──►  executor.apply_event()
                                                  │
                                                  └─► PlanStateMachine::transition()
                                                        Returns new PlanPhase
```

This cycle repeats until all plans reach terminal phases.

---

## Test Coverage

The action module has tests covering:

- **Display formatting**: All variants format correctly
- **Serde roundtrip**: All variants serialize and deserialize without loss
- **All variants serialize**: Exhaustive test of every `ExecutorAction` variant

---

## References

- The action/event pattern is a variant of the Command pattern (Gamma et al.
  1994, *Design Patterns*) where actions are reified as data structures.
- The separation of actions from effects follows the functional programming
  principle of "programs as values" — the state machine produces descriptions
  of effects (actions) rather than performing them directly.
- Agent role assignment draws on the multi-agent systems literature, where
  agents are assigned roles based on capabilities and task requirements
  (Wooldridge, M. (2009). *An Introduction to MultiAgent Systems*. Wiley).
