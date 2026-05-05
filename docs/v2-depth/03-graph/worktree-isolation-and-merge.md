# Worktree Isolation and Merge Queue

> Depth for [03-GRAPH.md](../../unified/03-GRAPH.md). How parallel agent execution is isolated via git worktrees and serialized back into a shared branch via a merge queue.

---

## Problem

When a Graph dispatches multiple Cells (agents) to execute in parallel, they share a single filesystem. Agent A editing `src/lib.rs` while Agent B compiles against the same file produces corruption. Naive serialization (one agent at a time) wastes parallelism. The system needs isolation during execution and controlled integration afterward.

Two complementary patterns solve this:

1. **Space pattern** -- Each agent gets its own git worktree (isolated filesystem + branch). This is a physical manifestation of the Space specialization: a Bus+Store partition with an isolation boundary.
2. **Pipeline pattern** -- Completed work passes through a merge queue Pipeline: enqueue, conflict-check, merge, regression-verify. Each step can reject, transforming the merge back into a retry.

---

## Space: Worktree Isolation

### Model

A worktree is a Space Cell that wraps a git working directory. Each active Flow (plan execution) gets one. The Space owns:

- A **Store partition**: the filesystem rooted at `.roko/worktrees/<plan_id>/`
- A **Bus partition**: commits on branch `roko/plan/<plan_id>`
- An **isolation boundary**: agents inside the Space cannot observe or corrupt sibling Spaces

```rust
/// A worktree Space -- isolation boundary for one Flow.
struct WorktreeSpace {
    /// Identity, derived from plan_id.
    id: String,
    /// Filesystem path to the isolated working directory.
    path: PathBuf,
    /// Git branch scoped to this Space.
    branch: String,          // e.g., "roko/plan/01-workspace-scaffold"
    /// Creation timestamp (milliseconds since epoch).
    created_at_ms: u64,
    /// Last activity timestamp, used for idle reclamation.
    last_active_ms: u64,
}
```

### Lifecycle

The `WorktreeManager` Cell manages Space creation and reclamation:

```
ensure_for_plan(plan_id)
  |
  v
[Space exists?] --yes--> return existing handle
  |no
  v
[budget check: live < max_live?]
  |no                            |yes
  v                              v
reclaim_idle()                 git branch roko/plan/<id> <base>
  |                            git worktree add <path> <branch>
  v                              |
[still over?]                    v
  |yes           return WorktreeSpace
  v
BudgetExceeded error
```

Key operations:

| Operation | What It Does |
|---|---|
| `ensure_for_plan(id)` | Idempotent: create-or-return. Handles resume scenarios where the Space already exists from a prior run. |
| `check_health(id)` | Returns `Ok`, `Missing`, `StaleLock`, or `Detached`. Stale locks are leftover `*.lock` files from crashed git operations. |
| `reclaim_idle()` | Removes Spaces inactive longer than `idle_ttl` (default 30 min). Prevents disk exhaustion. |
| `clear_stale_locks()` | Removes `*.lock` files left by crashed `git merge` or `git rebase` operations. Without this, the Space becomes permanently wedged. |
| `prune()` | Runs `git worktree prune` to clean metadata pointing at deleted directories. |

### Budget Enforcement

The `max_live` parameter (default: 8, from `config.conductor.max_agents`) caps concurrent Spaces. This is a hard resource limit -- each worktree consumes disk (full working copy) and inode budget. When a `create()` would exceed the limit, the manager first attempts idle reclamation. If still over budget, it returns `BudgetExceeded`.

### Thread Safety

`WorktreeManager` uses `Arc<parking_lot::Mutex<HashMap<String, WorktreeSpace>>>`. The `parking_lot` mutex is non-poisoning: a panic in one task does not permanently lock the manager. Git operations are further serialized by filesystem lock files (git's own concurrency mechanism).

---

## Pipeline: Merge Queue

When a Flow's gates pass, its worktree branch must merge into the shared batch branch. The merge queue is a Pipeline of four steps: **enqueue, select, merge, verify**. Each step can reject, sending the request back to the queue.

### Architecture

```rust
/// The merge queue serializes branch integration.
/// Arc<Mutex<Inner>> -- thread-safe, non-poisoning.
struct MergeQueue {
    /// Pending merge requests, ordered by priority.
    pending: Vec<MergeRequest>,
    /// Currently in-progress merges, keyed by plan_id.
    /// Files from these requests are locked.
    merging: HashMap<String, MergeRequest>,
    /// Files currently locked by in-progress merges.
    /// O(1) lookup for conflict detection.
    locked_files: HashSet<String>,
    /// Completed results (for audit).
    completed: Vec<MergeResult>,
}

struct MergeRequest {
    plan_id: String,
    branch_name: String,
    /// Files modified by this plan. The conflict-detection key.
    files_changed: Vec<String>,
    /// Higher priority merges first. From PlanState.priority.
    priority: u32,
    /// Retry count. After MAX_RETRIES (5), the merge fails permanently.
    retry_count: u32,
}
```

### Conflict Detection

The critical safety mechanism. Two merge requests conflict if they share any file in `files_changed`:

```
for each pending request R:
    if R.files_changed intersects locked_files:
        R is blocked -- skip
    else:
        return R as next_mergeable
```

Complexity: O(P x F) where P = pending requests, F = avg files per request. In practice P < 10 and F < 100, so this is negligible.

**File-level granularity** maximizes parallelism. Two plans touching the same crate but different files merge in parallel. Only plans modifying the exact same files are serialized.

### The Four-Step Pipeline

```
1. ENQUEUE
   Flow gates pass --> MergeRequest enters pending queue
   Ordered by priority (higher first), then FIFO within priority class.

2. SELECT (next_mergeable)
   Find highest-priority pending request whose files
   do not intersect locked_files.
   Returns None if all pending requests conflict with in-progress merges.

3. MERGE (mark_merging --> git merge --no-ff)
   Move request from pending to merging.
   Add files to locked_files.
   Execute: git checkout <batch_branch> && git merge --no-ff <plan_branch>

4. VERIFY (post-merge regression)
   Run cargo check --workspace on the merged result.
   On success: mark_complete, release file locks.
   On failure: mark_failed, increment retry_count, re-enqueue or fail permanently.
```

### Retry Logic

Failed merges re-enter the queue with reduced priority (positioned after peers at the same level). This implements positional backoff: the failed request waits while other merges complete, which may resolve the conflict.

Common scenario: two plans both modify `Cargo.lock`. Plan A merges first. Plan B's retry rebases onto the updated batch branch, resolving the lock file conflict automatically.

After `MAX_RETRIES` (5), the plan transitions to `Failed { reason: Deadlock }`. This prevents infinite retry loops.

### Dual-Level Conflict Prevention

The system prevents file conflicts at two stages:

| Stage | Mechanism | Where |
|---|---|---|
| **Execution** | `UnifiedTaskDag` infers file-overlap edges, serializing conflicting tasks | During implementation |
| **Integration** | `MergeQueue` locks files, serializing conflicting merges | During merge |

Both use file overlap as the conflict signal but operate at different pipeline stages. The DAG prevents concurrent modification; the merge queue prevents concurrent integration.

---

## MergeCoordinator: Wiring Queue to Event Loop

The `MergeQueue` module exists in `roko-orchestrator` but (as of the mori-diffs audit) is **not wired into the runtime event loop**. The current `MergeBranch` handler auto-advances:

```rust
// CURRENT: auto-advance, no queue
ExecutorAction::MergeBranch { plan_id } => {
    let _ = executor.apply_event(plan_id, &ExecutorEvent::MergeSucceeded);
}
```

The ideal wiring introduces a `MergeCoordinator` Cell that:

1. Receives `MergeBranch` actions from the event loop
2. Delegates to the `MergeQueue` for conflict-safe ordering
3. Runs post-merge regression gates as background tasks
4. Sends `MergeCompletion` Pulses back to the event loop via an `mpsc` channel

```rust
/// Thin orchestration layer that wires the MergeQueue into the event loop.
struct MergeCoordinator {
    queue: MergeQueue,
    /// Channel back to event loop for merge results.
    completion_tx: mpsc::Sender<MergeCompletion>,
    workdir: PathBuf,
    batch_branch: String,
}

struct MergeCompletion {
    plan_id: String,
    passed: bool,
    files_merged: Vec<String>,
    regression_output: String,
}
```

The coordinator drains non-conflicting merges in parallel via `tokio::spawn`, each running `git merge --no-ff` followed by `cargo check`. Results flow back to the event loop through the channel.

**Graceful degradation**: When `max_concurrent_tasks == 1` and only one plan is running, the coordinator falls through to the existing auto-advance path. No regression in single-plan mode.

---

## Warm Agent Pool

A complementary optimization. During gate execution (5-30s of compile/test/clippy), the system sits idle instead of pre-spawning the next agent. Over a 20-task plan this wastes 40-160 seconds.

The `WarmPool` pre-spawns agents during gate windows:

```rust
struct WarmPool {
    /// Pre-spawned agents awaiting promotion.
    pool: Mutex<VecDeque<WarmAgent>>,
    /// Max warm agents (default: 1, each ~200MB RSS).
    capacity: usize,
    /// Max age before eviction (default: 120s).
    max_age: Duration,
}

struct WarmAgent {
    handle: AgentHandle,
    config: AgentSpawnConfig,
    spawned_at: Instant,
}
```

**Lifecycle**: On gate dispatch, peek at the next DAG-ready task and pre-spawn its agent (Strategy A: speculative identity). On gate pass, promote the warm agent. On gate fail, evict (wrong context). On cancellation, evict all.

**Hit rate**: ~80% on sequential task execution. Cold-start penalty saved: 2-8s per task boundary.

---

## What This Enables

1. **True parallel plan execution** -- Multiple Flows execute simultaneously in isolated Spaces, merging through a conflict-aware queue. No filesystem corruption, no merge conflicts.
2. **Crash-safe integration** -- The merge queue tracks state; interrupted merges are retried on resume.
3. **Incremental merge validation** -- Post-merge regression gates catch cross-plan regressions that individual plan gates miss.
4. **Resource-bounded parallelism** -- The `max_live` budget prevents disk exhaustion; the `WarmPool` capacity cap prevents memory exhaustion.

---

## Feedback Loops

1. **Merge retry loop**: Failed merges re-enter the queue at lower priority. Other merges complete, updating the batch branch. Retry rebases onto the updated branch, often resolving the conflict. This is a Loop pattern with convergence.

2. **Idle reclamation loop**: The `WatcherRunner` (Conductor) monitors Space activity. Spaces idle beyond `idle_ttl` are reclaimed. This is a Lens (Observe protocol) triggering a React (reclaim).

3. **Warm pool prediction loop**: The next-task predictor's accuracy is observable. If gate failures frequently invalidate pre-spawned agents (high eviction rate), the system could learn to delay pre-spawning until later in the gate pipeline.

---

## Open Questions

1. **Merge queue not wired**: The `MergeQueue` exists (627 lines, 20+ tests) but is disconnected from the runtime event loop. The `MergeCoordinator` described above is the design -- it needs implementation and wiring into `event_loop.rs`.

2. **Warm pool not implemented**: No `WarmPool`, `warm_pool`, or pre-spawn mechanism exists anywhere in the codebase. The design requires a `spawn_agent_warm()` variant in `agent_stream.rs`. Claude CLI does not natively support "start idle, inject prompt later," so Strategy A (speculative identity) is preferred.

3. **Semantic merge strategies**: The merge queue uses git's textual merge. For structured files (TOML, Cargo.lock, JSON), semantic merge strategies (described in the cross-domain orchestration doc) would reduce false conflicts. These are not implemented.

4. **Deadlock detection**: The mori-diffs doc proposes cycle detection in the merge queue's conflict graph. The current `MergeQueue` handles deadlocks via priority demotion, but explicit cycle detection (O(n^2) on queue length, n < 20) would provide better diagnostics.

5. **Distributed merge coordination**: The current design is single-process. For multi-orchestrator scenarios (future CRDT executor state), the merge queue would need distributed locking or lease-based coordination.
