# Parallel Executor

> Depth for [03-GRAPH.md](../../unified/03-GRAPH.md). The pure state machine that interprets plan Graphs as Hot Flows, ticking through actions and events without performing I/O.

---

## What This Document Covers

The `ParallelExecutor` is the engine that runs plan Graphs. It is a **pure state machine**: it holds plan states, examines them on each tick, emits action requests (`ExecutorAction`), and consumes event results (`ExecutorEvent`). It never spawns a process, never touches the filesystem, never makes a network call.

This purity is the defining architectural decision. All side effects happen in the runtime harness (`PlanRunner`), which dispatches the executor's action requests to real subsystems and feeds results back as events. The executor is a Hot Flow that stays resident and re-fires per tick.

---

## The Executor as a Hot Flow

In unified vocabulary, the `ParallelExecutor` is a **Hot Flow** -- a Graph specialization that stays resident in memory and re-fires on a periodic tick. Each tick:

1. Examines all plan Cells (each plan is a Cell within the executor Graph)
2. Computes which plans need action based on their current phase
3. Emits `Vec<ExecutorAction>` -- Signals requesting side effects
4. Receives `ExecutorEvent` -- Pulses from the runtime reporting outcomes
5. Transitions plan phases based on events

```rust
// Pseudocode: the executor as a Hot Flow
struct ParallelExecutor {
    plan_states: HashMap<String, PlanState>,   // per-Cell mutable state
    queue_order: Vec<String>,                  // scheduling priority
    config: ExecutorConfig,                    // resource limits
}

impl ParallelExecutor {
    /// Hot Flow tick: examine all Cells, emit action Signals.
    fn tick(&self) -> Vec<ExecutorAction> { /* ... */ }

    /// Event ingress: apply a Pulse, transition Cell state.
    fn apply_event(&mut self, plan_id: &str, event: &ExecutorEvent)
        -> Result<PlanPhase, TransitionError> { /* ... */ }
}
```

### Configuration

The executor's resource budget constrains scheduling:

```rust
struct ExecutorConfig {
    max_concurrent_plans: usize,    // default: 4 -- plan-level parallelism
    max_concurrent_tasks: usize,    // default: 8 -- task-level parallelism
    max_auto_fix_iterations: u32,   // default: 5 -- gate retry bound
    max_merge_attempts: u32,        // default: 3 -- merge retry bound
    task_timeout_secs: u64,         // default: 600 (10 min)
    budget_usd: Option<f64>,        // total cost ceiling
    auto_replan: bool,              // replan on consecutive gate failures
}
```

These limits prevent resource exhaustion. `max_concurrent_plans` bounds worktree count. `max_concurrent_tasks` bounds total agent processes. `budget_usd` enables cost-constrained execution.

---

## The Tick Loop

Each `tick()` call iterates over all plans in queue order:

```
for plan in queue_order:
    if plan.is_terminal() or plan.paused: skip
    action = PlanStateMachine::next_action(plan.state)
    if action.is_some() and within_concurrency_limits():
        emit action
```

### Phase-to-Action Mapping

Each plan phase maps to exactly one action type. The executor does not decide *how* to perform the action -- only *what* to request.

| Plan Phase | Action emitted | Agent role (if any) |
|---|---|---|
| `Queued` | `DispatchPlan` | -- |
| `Enriching` | `SpawnAgent` | Strategist |
| `Implementing` | `SpawnAgent` | Implementer |
| `Gating` | `RunGate` | -- |
| `AutoFixing` | `SpawnAgent` | AutoFixer |
| `Verifying` | `RunVerify` | -- |
| `RegeneratingVerify` | `SpawnAgent` | AutoFixer |
| `Reviewing` | `SpawnAgent` | Auditor |
| `DocRevision` | `SpawnAgent` | Scribe |
| `Merging` | `MergeBranch` | -- |
| `Complete` / `Failed` / `Skipped` | None | (terminal) |

### Concurrency Management

Two levels of concurrency control:

**Plan-level**: `max_concurrent_plans` limits how many plans can be in a non-queued, non-terminal phase simultaneously. Excess plans stay `Queued` until a slot opens.

**Task-level**: `max_concurrent_tasks` limits total agent processes across all plans. The runtime harness tracks running agents via a `JoinSet` and respects this limit.

**Priority scheduling**: Plans dispatch in queue order (initialized by `rank_plans()`: priority descending, then num ascending). The queue is dynamically reorderable via `Reorder` actions.

---

## Per-Plan State: PlanState as a Cell

Each plan's mutable state is a Cell with typed I/O:

```rust
struct PlanState {
    plan_id: String,
    current_phase: PlanPhase,     // type-state: determines available transitions
    assigned_agents: Vec<String>, // active agent IDs
    gate_results: Vec<GateResult>,// accumulated Verify verdicts
    iteration: u32,               // retry counter (starts at 1)
    started_at_ms: u64,           // wall-clock start
    files_changed: Vec<String>,   // modified file tracking
    merge_attempts: u32,          // merge retry counter
    last_error: Option<String>,   // most recent failure reason
    paused: bool,                 // scheduling pause flag
    priority: u32,                // scheduling weight
}
```

Key state predicates:
- `is_terminal()` -- true for `Complete`, `Failed`, `Skipped`
- `all_gates_passed()` -- all `GateResult` entries are passing
- `has_gate_failure()` -- any `GateResult` failed
- `reset_for_retry()` -- clears gate results, increments iteration, clears error

### GateResult as Verify Verdict

```rust
struct GateResult {
    gate_name: String,   // "compile", "test", "clippy"
    rung: u32,           // position in the Verify Pipeline
    passed: bool,        // Verify verdict
    summary: String,     // human-readable output
    duration_ms: u64,    // wall-clock gate duration
}
```

Gate results accumulate on the PlanState as the plan progresses through the Verify Pipeline (rung 0: compile, rung 1: test, rung 2: clippy). Any failure triggers the auto-fix Loop.

---

## The Action/Event Vocabulary

### ExecutorAction -- Outbound Signals

Actions are *requests*, not effects. They are data structures that describe what the runtime should do:

| Action | Payload | Runtime effect |
|---|---|---|
| `DispatchPlan` | `plan_id` | Create worktree, parse tasks, initialize tracker |
| `SpawnAgent` | `plan_id, role, task` | Build config, launch agent process |
| `RunGate` | `plan_id, rung` | Execute compile/test/clippy in worktree |
| `RunVerify` | `plan_id` | Run task-level verification commands |
| `MergeBranch` | `plan_id` | Enqueue in merge queue, execute git merge |
| `FailPlan` | `plan_id, reason` | Transition to `Failed` |
| `CompletePlan` | `plan_id` | Transition to `Complete`, cleanup |
| `PausePlan` | `plan_id` | Set `paused = true` |
| `ResumePlan` | `plan_id` | Set `paused = false` |
| `Reorder` | `plan_id, position` | Move plan in execution queue |

All variants implement `Serialize + Deserialize` for snapshot persistence and event logging.

### ExecutorEvent -- Inbound Pulses

Events are results fed back from the runtime:

| Event | Meaning | Typical transition |
|---|---|---|
| `Start` | Plan dispatched | Queued -> Enriching |
| `EnrichmentDone` | Strategist finished | Enriching -> Implementing |
| `ImplementationDone` | All tasks in iteration done | Implementing -> Gating |
| `GatePassed` | All gates passed | Gating -> Verifying |
| `GateFailed` | Any gate failed | Gating -> AutoFixing (or Failed) |
| `AutoFixDone` | Fix agent finished | AutoFixing -> Gating |
| `VerifyPassed` | Verification passed | Verifying -> Reviewing |
| `VerifyFailed` | Verification failed | Verifying -> RegeneratingVerify |
| `ReviewApproved` | Auditor approved | Reviewing -> DocRevision |
| `ReviewRejected` | Auditor rejected | Reviewing -> Implementing |
| `DocRevisionDone` | Scribe finished | DocRevision -> Merging |
| `MergeSucceeded` | Git merge passed | Merging -> Complete |
| `MergeFailed` | Git merge conflict | Merging -> Failed |
| `Skip` | Operator skip | Any -> Skipped |
| `Fatal(reason)` | Unrecoverable error | Any -> Failed |

---

## Resource-Aware Scheduling

Beyond simple concurrency counts, the executor tracks multiple resource dimensions:

```rust
struct ResourceBudget {
    agent_slots: ResourcePool,       // bounded by max_concurrent_tasks
    api_tokens: RateLimitResource,   // token bucket (burst: 50, refill: 4/s)
    token_budget: TokenBudget,       // LLM token ceiling
    worktree_slots: ResourcePool,    // bounded by WorktreeConfig::max_live
    cost_budget: CostBudget,         // USD ceiling with 80% warning
}
```

The tick loop becomes resource-aware:

```
for each ready task in priority order:
    check = resources.can_schedule(task)
    if Available: reserve resources, emit SpawnAgent
    if Blocked(resource, wait_estimate):
        if wait_estimate < task.deadline_slack: skip (retry next tick)
        else: consider model downgrade or task decomposition
```

### Token Budget by Complexity

```
Mechanical:     0.3x base (3,000 tokens)
Fast:           0.5x base (5,000 tokens)
Standard:       1.0x base (10,000 tokens)
Focused:        2.0x base (20,000 tokens)
Architectural:  4.0x base (40,000 tokens)
```

### Priority Inversion Prevention

The executor uses the Immediate Ceiling Priority Protocol (ICPP) for shared resources (Sha, Rajkumar & Lehoczky 1990):

- Each resource has a **ceiling** = highest priority of any plan that uses it
- When a plan acquires a resource, its effective priority is raised to the ceiling
- This prevents medium-priority plans from preempting low-priority plans that hold resources needed by high-priority plans

Guarantees: bounded blocking (at most one critical section), deadlock-free, no chained blocking.

---

## Formal Properties

### Petri Net Model

The executor can be modeled as a Workflow Net (WF-net) for formal verification (van der Aalst 1997):

| Executor concept | Petri net element |
|---|---|
| Task | Transition (fires when preconditions met) |
| Task preconditions | Input places (must have tokens) |
| Task completion | Output places (tokens deposited) |
| Dependencies | Places connecting transitions |
| Agent slot | Place with bounded tokens |
| Plan start | Initial marking (token in source) |
| Plan completion | Token in sink place |

**Soundness** (van der Aalst): A WF-net is sound iff (1) every reachable state can reach the final state (no deadlocks), (2) the final state has no orphaned tokens, and (3) every transition can fire in some reachable state (no dead code).

**Structural invariants** derived from the incidence matrix:

```
agents_in_use + agents_idle = MAX_CONCURRENT_TASKS
for each plan P: tasks_pending(P) + tasks_running(P) + tasks_complete(P) = total_tasks(P)
```

These hold for ALL reachable markings, providing global guarantees without state enumeration.

---

## Design Rationale: Why a Pure State Machine?

1. **Testability**: All orchestration logic tested without mocking I/O. Construct plans, fire events, assert phase transitions.
2. **Crash recovery**: Executor serializes to JSON at any point; restored exactly via `from_snapshot()`.
3. **Composability**: Embeddable in different runtimes -- CLI harness, HTTP server, test framework. The runtime provides effects; the executor provides logic.
4. **Auditability**: Every transition driven by an explicit event, logged with hash-chaining for tamper detection.

**Why not actors?** An actor system distributes state across mailboxes, making snapshots harder and crash recovery more complex. The centralized state machine is single-threaded -- `tick()` and `apply_event()` run from one async task -- but this is acceptable because the executor's work is lightweight (phase transitions, queue management). All heavy work runs in the runtime's `JoinSet`.

---

## Reality Check: Implementation vs. Spec

From the mori-diffs reality check:

**Budget check is post-hoc.** The per-plan budget check runs at `SpawnAgent` dispatch time, but cost accumulates only after the agent completes a turn. An expensive turn can overshoot. The spec calls for pre- and post-dispatch checks, plus per-turn re-evaluation.

**No gate timeout.** `gate_dispatch::spawn_gate` spawns a `tokio::spawn` with no timeout. A hanging `cargo test` blocks the plan indefinitely. The spec calls for a 300-second default with synthetic failure verdicts on timeout.

**No concurrency limiting for cargo.** Multiple plans can spawn `cargo test` / `cargo clippy` simultaneously, competing for build artifacts and filesystem locks. The spec calls for a `GateSemaphore` (default: 2 concurrent cargo processes).

**No failure classification.** Gate failures are binary (passed/failed). The event loop treats all failures identically. The spec calls for `GateFailureClassification` (Permanent vs Transient) to decide retry strategy.

**No retry backoff.** Retries are immediate. The spec calls for exponential backoff: 5s, 10s, 20s, 40s (capped at 45s) with +/-20% jitter.

**No plan-level timeout.** `RunConfig::timeout_secs` exists but is only per-task. A plan can loop through retries indefinitely. The spec calls for a 1-hour wall-clock hard limit.

**What IS wired:** The `ParallelExecutor` state machine, `PlanStateMachine` transitions, `PlanState` tracking, action/event vocabulary, snapshot/restore, and the tick loop in the runner v2 event loop. The pure-state-machine architecture is solid; the gaps are in the runtime harness's safety guardrails.

---

## What This Enables

1. **Deterministic replay**: Given the same event sequence, the executor always produces the same state. Enables crash recovery via event-log replay.
2. **Resource-bounded execution**: Multi-dimensional resource tracking prevents budget overruns, API rate limit violations, and worktree exhaustion.
3. **Formal verification**: The Petri net model enables static plan verification before execution begins.
4. **Runtime swappability**: The executor is independent of its runtime. The same state machine can be driven by a CLI, an HTTP API, or a test harness.

## Feedback Loops

- **Loop: GateAdaptation** -- per-rung pass rates (via `AdaptiveThresholds` EMA) feed back into the executor's retry budget. If a gate consistently fails for a task type, the system escalates model routing before retrying.
- **Loop: CostLearning** -- efficiency events from completed agents feed back into `CostBudget` projections. Over time, the system's per-task cost estimates improve, enabling tighter budget enforcement.
- **Lens: ExecutorLens** -- observes plan phase distribution, queue depth, concurrency utilization, action throughput. Powers the TUI dashboard and HTTP status endpoints.

## Open Questions

1. **Multi-task parallelism within a plan.** The DAG computes waves allowing intra-plan parallelism, but the runner currently dispatches one task at a time. Requires per-plan agent handle maps instead of a single global `agent_handle`.
2. **Partial plan success.** If tasks A, B, C pass but task D fails, should the plan report partial completion? Currently it's all-or-nothing.
3. **Preemption.** Can a high-priority plan preempt a running agent from a low-priority plan? Currently agents run to completion once spawned.
4. **Work stealing.** Rayon-style work-stealing (Blumofe & Leiserson 1999) could improve utilization when some plans complete faster than others. Is the complexity warranted given current plan counts (typically < 20)?
