# Parallel Executor

> **Module**: `roko-orchestrator/src/executor/mod.rs`
> **Key type**: `ParallelExecutor`
> **Sub-modules**: `action.rs`, `plan_state.rs`, `state_machine.rs`,
> `snapshot.rs`, `recovery.rs`, `reorder.rs`


> **Implementation**: Shipping

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

---

## Resource-Aware Scheduling

The executor manages multiple resource dimensions beyond simple concurrency
counts. Each resource type imposes constraints on task scheduling.

### Resource Model

```rust
/// Resources tracked by the executor for scheduling decisions.
pub struct ResourceBudget {
    /// Agent process slots (bounded by max_concurrent_tasks).
    pub agent_slots: ResourcePool,
    /// API rate limit tokens (replenishable).
    pub api_tokens: RateLimitResource,
    /// Token budget for LLM calls (depletable).
    pub token_budget: TokenBudget,
    /// Git worktree slots (bounded by WorktreeConfig::max_live).
    pub worktree_slots: ResourcePool,
    /// USD cost budget (depletable).
    pub cost_budget: CostBudget,
}

/// A bounded pool of identical resources (e.g., agent slots).
pub struct ResourcePool {
    pub capacity: usize,
    pub in_use: usize,
}

/// A replenishable rate-limited resource (e.g., API calls).
/// Implements the token bucket algorithm.
pub struct RateLimitResource {
    /// Burst capacity.
    pub capacity: u32,           // default: 50
    /// Refill rate (tokens per second).
    pub refill_rate: f64,        // default: 4.0 (240 RPM)
    /// Current available tokens.
    pub current_tokens: f64,
    /// Last refill timestamp.
    pub last_update: Instant,
}

/// Token budget for LLM dispatches.
pub struct TokenBudget {
    /// Total budget for the plan run.
    pub total: u64,              // default: 1_000_000
    /// Tokens spent so far.
    pub spent: u64,
    /// Per-task default allocation.
    pub per_task_default: u64,   // default: 10_000
    /// Per-task maximum (hard cap).
    pub per_task_max: u64,       // default: 50_000
    /// Multipliers by complexity tier.
    /// Mechanical: 0.3, Fast: 0.5, Standard: 1.0,
    /// Focused: 2.0, Architectural: 4.0.
    pub complexity_multiplier: HashMap<String, f64>,
}

/// USD cost budget with early warning.
pub struct CostBudget {
    /// Total budget in USD.
    pub total_usd: f64,         // from ExecutorConfig::budget_usd
    /// Spent so far.
    pub spent_usd: f64,
    /// Warning threshold (fraction). Default: 0.8 (80%).
    pub warn_threshold: f64,
    /// Hard stop threshold. Default: 1.0 (100%).
    pub stop_threshold: f64,
}

impl ResourceBudget {
    /// Check if a task can be scheduled given current resources.
    /// Returns the resource that blocks (if any) and estimated wait time.
    pub fn can_schedule(&self, task: &TaskDef) -> ResourceCheck { /* ... */ }

    /// Reserve resources for a task dispatch.
    pub fn reserve(&mut self, task: &TaskDef) -> Result<ResourceReservation, ResourceError> {
        /* ... */
    }

    /// Release resources when a task completes or fails.
    pub fn release(&mut self, reservation: ResourceReservation) { /* ... */ }
}
```

### Scheduling Algorithm with Resources

The tick loop becomes resource-aware:

```
for each ready task in priority order:
    check = resources.can_schedule(task)
    match check:
        Available → reserve resources, emit SpawnAgent
        Blocked(resource, wait_estimate) →
            if wait_estimate < task.deadline_slack:
                skip (will retry next tick)
            else:
                consider model downgrade or task decomposition
```

This extends the simple concurrency-count model to handle the multi-dimensional
resource constraints of an LLM-based agent system.

---

## Priority Inversion Prevention

Priority inversion occurs when a high-priority plan is blocked because a
low-priority plan holds a shared resource (e.g., a merge queue slot, a crate
lock, an API rate limit token). A medium-priority plan then preempts the
low-priority one, causing the high-priority plan to wait for both.

This is the same bug that caused the Mars Pathfinder resets in 1997 — a
high-priority bus management task was blocked by a low-priority meteorological
task holding a mutex, while a medium-priority communications task preempted both
(Sha, Rajkumar & Lehoczky, IEEE TC 1990).

### Priority Ceiling Protocol

The executor uses the **Immediate Ceiling Priority Protocol** (ICPP) for shared
resources:

```rust
/// Each shared resource has a priority ceiling = the highest priority
/// of any plan that may use it.
pub struct PriorityCeiling {
    /// Resource → ceiling priority.
    ceilings: HashMap<ResourceId, u32>,
}

impl PriorityCeiling {
    /// Compute ceiling from plan metadata.
    /// ceiling(R) = max(priority(P) for all plans P that declare use of R).
    pub fn compute(plans: &[PlanInfo]) -> Self { /* ... */ }
}

/// When a plan acquires a resource, its effective priority is immediately
/// raised to the resource's ceiling. This prevents preemption by
/// medium-priority plans.
///
/// Guarantees (Sha et al. 1990):
/// 1. Bounded blocking: a plan is blocked for at most ONE critical section
///    of a lower-priority plan. Strictly better than basic priority inheritance.
/// 2. Deadlock-free: prevents deadlock from nested resource acquisition.
/// 3. No chained blocking: at most one blocking event per plan invocation.
///
/// Worst-case blocking bound:
///   B_i = max over all lower-priority plans L_j and resources R_k:
///         duration of L_j's critical section for R_k,
///         where ceiling(R_k) >= priority(plan_i)
```

In practice, the executor tracks effective priorities and adjusts the tick
loop's plan iteration order accordingly. When a low-priority plan holds the
merge queue and a high-priority plan is waiting, the low-priority plan's merge
is prioritized (its effective priority is boosted).

---

## Formal Model: Petri Net Representation

The executor can be modeled as a **Workflow Net** (WF-net), enabling formal
verification of correctness properties (van der Aalst 1998).

### Mapping to Petri Net Elements

| Executor Concept | Petri Net Element | Semantics |
|-----------------|-------------------|-----------|
| Task | Transition | Fires when preconditions met |
| Task preconditions | Input places | Must have tokens for transition to fire |
| Task completion | Output places | Tokens deposited after firing |
| Dependencies | Places connecting transitions | Output→input |
| Agent slot | Place with bounded tokens | Token = available agent |
| API rate limit | Timed place | Token available at `t + cooldown` |
| Plan start | Initial marking (token in source) | Plan is ready |
| Plan completion | Token in sink place | All tasks done |

### Multi-Plan Colored Petri Net

For concurrent multi-plan execution, use **Colored Petri Nets** (CPNs) where
tokens carry identity:

```
Color sets:
  PlanID   = string         // e.g., "01-workspace"
  TaskID   = string         // e.g., "t1"
  AgentID  = string         // e.g., "agent-0"
  Token    = PlanID × TaskID
  Resource = AgentID × PlanID

Place markings:
  ready:    {("01-workspace", "t1"), ("02-core", "t1")}
  running:  {}
  agents:   {("agent-0", _), ("agent-1", _)}  // 2 available
  complete: {}
```

Guard conditions on transitions enforce constraints:
- "Agent can only work on Rust tasks": `guard [agent_type(a) = "rust"]`
- "Plan gets at most 2 concurrent tasks": token count filter

### Soundness Verification

A WF-net is **sound** if and only if (van der Aalst 1997):

1. **Option to complete**: For every reachable marking from the initial state,
   there exists a firing sequence to the final state. (No deadlocks.)
2. **Proper completion**: When the final token arrives, no other tokens remain.
   (No orphaned tasks.)
3. **No dead transitions**: Every transition can fire in at least one reachable
   marking. (No unreachable code.)

**The fundamental theorem**: A WF-net N is sound **iff** the short-circuited
net N' (with an extra transition from sink back to source) is **live and
bounded**.

For free-choice nets (where every arc from a place goes to transitions sharing
the same input places), soundness is decidable in **polynomial time** using
the rank theorem. General WF-nets are EXPSPACE-complete.

### Structural Analysis

**Place invariants** (P-invariants) verify conservation laws without state
explosion. For the executor:

```
Invariant: agents_in_use + agents_idle = MAX_CONCURRENT_TASKS
           (tokens are never created or destroyed)

Invariant: for each plan P:
           tasks_pending(P) + tasks_running(P) + tasks_complete(P)
           = total_tasks(P)
```

These invariants are derived from the incidence matrix `C[p][t]` by solving
`y^T · C = 0` for non-negative y. They hold for ALL reachable markings,
providing global guarantees without enumerating states.

### Practical Application

The Petri net model enables:

1. **Static plan verification** — check that every plan completes without
   deadlock before execution begins
2. **Resource conservation** — verify that agents are never lost or duplicated
3. **Bounded concurrency** — prove that `max_concurrent_tasks` is never exceeded
4. **Deadlock detection** — identify plan dependency cycles that the topological
   sort might miss when combined with resource constraints

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
- Sha, L., Rajkumar, R. & Lehoczky, J. P. (1990). Priority inheritance
  protocols: An approach to real-time synchronization. *IEEE Trans. Computers*,
  39(9), 1175–1185. (Priority ceiling protocol, priority inversion prevention.)
- van der Aalst, W. M. P. (1997). Verification of workflow nets. *Application
  and Theory of Petri Nets 1997*. LNCS 1248. (WF-net soundness.)
- van der Aalst, W. M. P. (1998). The application of Petri nets to workflow
  management. *J. Circuits, Systems and Computers*, 8(1), 21–66.
- Blumofe, R. D. & Leiserson, C. E. (1999). Scheduling multithreaded
  computations by work stealing. *JACM*, 46(5), 720–748. (Work-stealing
  scheduler bounds: E[T_P] = T_1/P + O(T_inf).)
- Chase, D. & Lev, Y. (2005). Dynamic circular work-stealing deque. *SPAA
  2005*. (Lock-free deque used by Rayon/crossbeam-deque.)
- Wei, C. et al. (2025). Agent.xpu: Scheduling concurrent agentic workloads
  on heterogeneous SoCs. *arXiv:2506.24045*. (LLM agent scheduling with
  kernel-level preemption; 3.2× throughput gain.)
- Patel, S. et al. (2024). BudgetMLAgent: Multi-agent cascade for cost-efficient
  LLM task execution. *AIMLSystems 2024*. (94.2% cost reduction via
  three-tier model cascade.)
