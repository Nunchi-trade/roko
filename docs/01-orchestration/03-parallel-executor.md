# Parallel Executor

> **Module**: `roko-orchestrator/src/executor/mod.rs`
> **Key type**: `ParallelExecutor`
> **Sub-modules**: `action.rs`, `plan_state.rs`, `state_machine.rs`,
> `snapshot.rs`, `recovery.rs`, `reorder.rs`

---

## Overview

The `ParallelExecutor` is the pure state machine at the heart of the Roko
Orchestrator. It manages the lifecycle of multiple plans simultaneously,
tracking each plan's phase, gate results, assigned agents, and queue position.
It never performs I/O — it emits `ExecutorAction` requests and consumes
`ExecutorEvent` results.

This purity is the defining architectural characteristic of the executor. All
side effects — spawning agent processes, running compilers, merging git
branches — happen in the runtime harness (`PlanRunner`). The executor only
decides what should happen next.

---

## Architecture

```
                    ┌─────────────────────────┐
                    │   ParallelExecutor       │
                    │                          │
  ExecutorEvent ──► │  plan_states: HashMap    │ ──► Vec<ExecutorAction>
                    │  queue_order: Vec        │
                    │  config: ExecutorConfig  │
                    │                          │
                    └─────────────────────────┘
```

The executor maintains:

- **`plan_states: HashMap<String, PlanState>`** — per-plan mutable state
- **`queue_order: Vec<String>`** — plan IDs in execution priority order
- **`config: ExecutorConfig`** — concurrency limits, timeouts, budget

Two methods drive the loop:

- **`tick() -> Vec<ExecutorAction>`** — examine all plans, compute next actions
- **`apply_event(plan_id, event) -> Result<PlanPhase>`** — apply an event,
  transition plan phase

---

## ExecutorConfig

```rust
pub struct ExecutorConfig {
    /// Maximum plans executing concurrently.
    pub max_concurrent_plans: usize,    // default: 4
    /// Maximum tasks executing concurrently across all plans.
    pub max_concurrent_tasks: usize,    // default: 8
    /// Maximum auto-fix iterations before declaring failure.
    pub max_auto_fix_iterations: u32,   // default: 5
    /// Maximum merge attempts before declaring deadlock.
    pub max_merge_attempts: u32,        // default: 3
    /// Task timeout in seconds.
    pub task_timeout_secs: u64,         // default: 600 (10 min)
    /// Total USD budget for the run.
    pub budget_usd: Option<f64>,
    /// Whether to auto-replan on consecutive gate failures.
    pub auto_replan: bool,              // default: false
}
```

These limits prevent resource exhaustion. The `max_concurrent_plans` limit
ensures the system doesn't spawn too many worktrees or agent processes. The
`max_concurrent_tasks` limit bounds total parallelism across all plans. The
`budget_usd` limit enables cost-constrained execution.

---

## The Tick Loop

The `tick()` method is called by the runtime harness in a loop. Each call:

1. Iterates over all plans in queue order
2. For each non-terminal, non-paused plan:
   - Calls `PlanStateMachine::next_action(plan_state)` to get the next action
   - If the plan is `Queued` and within the concurrent plan limit, emits
     `DispatchPlan`
   - If the plan is `Implementing`, emits `SpawnAgent` for the next ready task
   - If the plan is `Gating`, emits `RunGate` for the next gate rung
   - If the plan is `Merging`, emits `MergeBranch`
3. Returns all collected actions

The runtime dispatches each action asynchronously, then feeds results back via
`apply_event()`.

### Phase-to-action mapping

| Phase | Action | Role |
|-------|--------|------|
| `Queued` | `DispatchPlan` | — |
| `Enriching` | `SpawnAgent` | Strategist |
| `Implementing` | `SpawnAgent` | Implementer |
| `Gating` | `RunGate` | — |
| `AutoFixing` | `SpawnAgent` | AutoFixer |
| `Verifying` | `RunVerify` | — |
| `RegeneratingVerify` | `SpawnAgent` | AutoFixer |
| `Reviewing` | `SpawnAgent` | Auditor |
| `DocRevision` | `SpawnAgent` | Scribe |
| `Merging` | `MergeBranch` | — |
| `Complete` / `Failed` / `Skipped` | None | (terminal) |

---

## Per-Plan State

Each plan's mutable state is tracked by `PlanState`:

```rust
pub struct PlanState {
    pub plan_id: String,
    pub current_phase: PlanPhase,
    pub assigned_agents: Vec<String>,
    pub gate_results: Vec<GateResult>,
    pub iteration: u32,           // starts at 1, bumps on retry
    pub started_at_ms: u64,
    pub files_changed: Vec<String>,
    pub merge_attempts: u32,
    pub last_error: Option<String>,
    pub paused: bool,
    pub priority: u32,
}
```

Key methods:

- **`is_terminal()`** — `true` for `Complete`, `Failed`, `Skipped`
- **`all_gates_passed()`** — `true` when all gate results are passing
- **`has_gate_failure()`** — `true` when any gate result failed
- **`reset_for_retry()`** — clears gate results, increments iteration, clears
  last error

### GateResult

```rust
pub struct GateResult {
    pub gate_name: String,     // "compile", "test", "clippy"
    pub rung: u32,             // position in the gate ladder
    pub passed: bool,
    pub summary: String,
    pub duration_ms: u64,
}
```

Gate results accumulate on the `PlanState` as the plan progresses through the
gate ladder. If any gate fails, the plan enters `AutoFixing`. If all gates pass,
the plan advances to `Verifying`.

---

## Concurrency Management

The executor enforces two concurrency limits:

### Plan-level concurrency

`max_concurrent_plans` limits how many plans can be in a non-queued,
non-terminal phase simultaneously. Plans exceeding this limit stay `Queued`
until a slot opens.

### Task-level concurrency

`max_concurrent_tasks` limits total agent processes across all plans. The
runtime harness (`PlanRunner`) uses a `JoinSet` to track running agents and
respects this limit when deciding whether to dispatch additional `SpawnAgent`
actions.

### Priority scheduling

Plans are dispatched in queue order. The queue is initialized by
`rank_plans()` (priority descending, then num ascending) and can be
dynamically reordered via `Reorder` actions.

Within a plan, tasks are dispatched in dependency order. The `TaskTracker`
in the runtime harness tracks which tasks are completed, failed, or skipped,
and computes ready tasks based on dependency satisfaction.

---

## Plan Lifecycle Methods

### Adding plans

```rust
executor.add_plan(plan_id, PlanState::new(plan_id).with_priority(priority));
```

Plans are added in the order returned by `discover_plans()`. Each gets a
`PlanState` starting at `Queued`.

### Pausing and resuming

```rust
executor.pause_plan(plan_id)?;   // sets paused = true
executor.resume_plan(plan_id)?;  // sets paused = false
```

Paused plans do not emit actions from `tick()`. Their state is preserved.

### Snapshots

```rust
let snapshot: ExecutorSnapshot = executor.snapshot();
let restored = ParallelExecutor::from_snapshot(snapshot);
```

Snapshots capture the full mutable state for crash recovery. See
`09-snapshot-recovery.md` for details.

---

## Design Rationale

### Why a pure state machine?

1. **Testability**: All orchestration logic can be tested without mocking I/O.
   The executor's tests construct plans, fire events, and assert phase
   transitions — no filesystem, no processes, no network.

2. **Crash recovery**: The executor can be serialized to JSON at any point and
   restored exactly. The event log provides an alternate recovery path via
   replay.

3. **Composability**: The executor can be embedded in different runtimes — the
   CLI harness (`PlanRunner`), a future HTTP server, or a testing framework.
   The runtime provides the effects; the executor provides the logic.

4. **Auditability**: Every state transition is driven by an explicit
   `ExecutorEvent`. The event log records these with hash-chaining for
   tamper detection.

### Why not an actor system?

An actor system (e.g., Actix) would distribute state across actor mailboxes,
making snapshots harder to take and crashes harder to recover from. The
centralized state machine is easier to reason about, serialize, and test.

The trade-off is that the executor is single-threaded — `tick()` and
`apply_event()` are called from one async task. This is acceptable because the
executor's work is lightweight (phase transitions, queue management). All
heavy work (agent processes, compilation, git operations) happens in the
runtime's `JoinSet`.

---

## References

- The pure state machine approach draws on the Event Sourcing pattern
  (Fowler 2005) where state transitions are driven by explicit events that
  can be replayed.
- The executor's tick-based loop is similar to game engine update loops and
  the CoALA (Cognitive Architectures for Language Agents) 9-step cognitive
  cycle (Sumers et al. 2023) — both use a regular polling mechanism to drive
  state forward.
- Concurrency limits follow the bounded-concurrency pattern from operating
  systems scheduling (semaphore-based admission control).
