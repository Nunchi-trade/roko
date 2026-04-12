# Unified Task DAG

> **Module**: `roko-orchestrator/src/dag.rs`
> **Key type**: `UnifiedTaskDag`
> **Tests**: 20 tests covering empty DAG, linear chain, fan-out/fan-in, cycles,
> cross-plan deps, file overlap, max wave width, critical path


> **Implementation**: Shipping

---

## Overview

The `UnifiedTaskDag` is the cross-plan scheduling backbone of the Roko
Orchestrator. It takes task definitions from multiple plans, resolves intra-plan
and cross-plan dependencies, infers file-conflict edges, rejects cycles, and
produces:

1. A topological ordering of all tasks
2. Execution waves (groups of tasks that can run in parallel)
3. Critical path estimation for time budgeting

This is the data structure that makes multi-plan parallel execution safe — it
ensures no two tasks that modify the same files run simultaneously, and that
dependency ordering is respected across plan boundaries.

---

## Concepts

### GlobalTaskId

Every task in the DAG is identified by a `GlobalTaskId` in the format
`"plan_id:task_id"`. This composite key ensures uniqueness across plans:

```
plan-A:task-1
plan-A:task-2
plan-B:task-1     ← different task, different plan
```

### Nodes and edges

- **Node**: A `GlobalTaskId` representing a single task
- **Forward edge**: `A → B` means "B depends on A" (A must complete before B starts)
- **Reverse edge**: `B → A` means "A is a dependency of B" (used for fan-in detection)

Edges come from three sources:

1. **Intra-plan task dependencies**: Declared in `tasks.toml` via `depends_on`
2. **Cross-plan dependencies**: Declared in `tasks.toml` via `depends_on_plan`
   or in plan frontmatter via `depends_on`
3. **File-conflict inference**: If two tasks from different plans modify the same
   files, a synthetic dependency edge is added to prevent concurrent execution

### Synthetic `__whole__` nodes

When a task declares a cross-plan dependency on an entire plan (e.g.,
`depends_on_plan = ["01-core"]`), the DAG creates a synthetic node
`"01-core:__whole__"` that depends on every task in plan `01-core`. The
dependent task then depends on this synthetic node, ensuring it waits for the
entire upstream plan to complete.

---

## Construction: `UnifiedTaskDag::build()`

The `build()` method takes a slice of `(plan_id, Vec<TaskDef>)` pairs and a
`DagConfig`, and constructs the complete DAG:

### Step 1: Collect nodes

Every task from every plan becomes a node. Tasks are keyed by their
`GlobalTaskId`.

### Step 2: Resolve intra-plan dependencies

For each task's `depends_on` list, a forward edge is created from the
dependency to the dependent task. Both must belong to the same plan.

### Step 3: Resolve cross-plan dependencies

For each task's `depends_on_plan` list:

1. If the referenced plan has tasks, create a synthetic `__whole__` node
2. Add forward edges from every task in the referenced plan to the `__whole__`
   node
3. Add a forward edge from the `__whole__` node to the dependent task

### Step 4: File-overlap inference

This is where the DAG prevents concurrent modification conflicts. For every
pair of tasks from different plans:

- If `task_a.files` ∩ `task_b.files` ≠ ∅, add a dependency edge between them

The direction is deterministic: the task from the lexicographically earlier plan
depends on the task from the later plan (or vice versa, consistently). This
prevents deadlocks while ensuring mutual exclusion.

### Step 5: Reverse edges

For every forward edge `A → B`, a reverse edge `B → A` is recorded. Reverse
edges are used for fan-in detection and wave computation.

### Step 6: Cycle detection

After all edges are added, the DAG runs Kahn's algorithm for topological sort.
If the sort does not visit all nodes, a cycle exists and the build fails with
`DagError::CycleDetected`.

### Configuration

```rust
pub struct DagConfig {
    /// Maximum number of tasks in a single execution wave.
    pub max_wave_width: usize,
}
```

`max_wave_width` controls parallelism. If a wave has more ready tasks than
`max_wave_width`, overflow tasks are pushed to the next wave. This prevents
resource exhaustion when many tasks become ready simultaneously.

---

## Topological Sort

The `topological_sort()` method implements Kahn's algorithm:

1. Compute in-degrees for all nodes
2. Initialize a queue with all zero-in-degree nodes
3. While the queue is non-empty:
   a. Extract the lexicographically smallest node (deterministic tie-breaking)
   b. For each successor, decrement in-degree
   c. If a successor's in-degree reaches zero, add it to the queue
4. If the sorted list has fewer nodes than the graph, a cycle exists

The lexicographic tie-breaking ensures deterministic ordering across runs. Given
the same inputs, the DAG always produces the same topological order.

---

## Wave Computation

The `waves()` method computes execution waves — groups of tasks that can safely
run in parallel:

1. Start with all zero-in-degree nodes as Wave 0
2. For each wave:
   a. If the wave exceeds `max_wave_width`, split into chunks
   b. Mark all tasks in the wave as "executed"
   c. Find newly unblocked tasks (in-degree reaches zero) → next wave
3. Continue until all tasks are assigned to waves

Each `ExecutionWave` contains:

```rust
pub struct ExecutionWave {
    /// Zero-based wave index.
    pub index: usize,
    /// Task IDs in this wave (can run in parallel).
    pub tasks: Vec<String>,
}
```

### Example: 4-plan scheduling

Consider four plans with file overlaps:

```
Plan A: tasks A1 → A2 → A3 (files: crates/roko-core/)
Plan B: tasks B1 → B2      (files: crates/roko-core/, crates/roko-agent/)
Plan C: tasks C1            (files: crates/roko-gate/)
Plan D: tasks D1 → D2      (files: crates/roko-agent/)
```

File-conflict edges:
- A ↔ B (both touch `roko-core`)
- B ↔ D (both touch `roko-agent`)

With `max_wave_width = 4`, the waves might be:

```
Wave 0: [A1, C1]       ← A and C have no conflicts, B blocked by A overlap
Wave 1: [A2, B1]       ← B1 can start after A1 completes (file overlap resolved)
Wave 2: [A3, B2, D1]   ← D1 can start after B1 completes (file overlap resolved)
Wave 3: [D2]           ← D2 depends on D1
```

---

## Critical Path Estimation

The `stats()` method computes DAG statistics including critical path length:

```rust
pub struct DagStats {
    /// Total number of nodes (tasks + synthetic).
    pub total_nodes: usize,
    /// Total number of edges.
    pub total_edges: usize,
    /// Length of the longest path through the DAG.
    pub critical_path_length: usize,
    /// Number of execution waves.
    pub wave_count: usize,
}
```

Critical path is computed via dynamic programming on the topological order:

1. Initialize all path lengths to 1
2. For each node in topological order, update successor path lengths:
   `path[successor] = max(path[successor], path[node] + 1)`
3. The critical path length is `max(path[*])`

This gives the minimum number of sequential steps needed to complete all tasks,
regardless of parallelism. It's useful for:

- Estimating minimum wall-clock time
- Identifying bottleneck task chains
- Setting expectations for budget and timeline

---

## Connection to the Original Mori Design

The `UnifiedTaskDag` in Roko corresponds to the `UnifiedTaskDag` described in
the original Mori orchestrator documentation
(`bardo-backup/prd/25-mori/mori-unified-dag.md`). Key concepts carried forward:

- **GlobalTaskId format**: `"plan:task"` composite keys
- **File-conflict detection**: The `next_runnable()` algorithm that checks file
  overlaps between ready tasks
- **Wave scheduling**: BFS-layered parallel groups
- **Critical path estimation**: DP on topological order

Concepts from Mori that are reframed in Roko:

- **Task routing classification** (complexity, category, quality, speed,
  reasoning, context_weight): Now handled by the `CascadeRouter` in
  `roko-learn`, not the DAG itself. The DAG is purely structural.
- **Model selection matrix**: Moved to `roko-learn/src/model_router.rs`
- **Token budget management**: Moved to the runtime harness (`PlanRunner`)

The Roko DAG is intentionally simpler than the Mori version — it handles
structure and ordering only. Routing, model selection, and budget management
are separated into their own subsystems following the Synapse Architecture's
principle of composable, single-responsibility components.

---

## HEFT Scheduling Context

The DAG's wave computation relates to the HEFT (Heterogeneous Earliest Finish
Time) scheduling algorithm used in multi-agent orchestration
(`refactoring-prd/05-agent-types.md`, §7):

> The agent pool (per collective) uses HEFT-like scheduling to dispatch tasks:
> estimate finish time per task considering (a) agent capability, (b) task
> complexity, (c) current load. The result is the Heterogeneous Earliest Finish
> Time heuristic.

In the current implementation, the DAG computes wave structure (what can run in
parallel), while the `PlanRunner` handles the HEFT-like dispatch decisions
(which agent gets which task, considering model routing, crate familiarity,
and affect state). This separation keeps the DAG pure and the scheduling
logic in the runtime where it has access to dynamic state.

---

## Error Types

```rust
pub enum DagError {
    /// A cycle was detected in the dependency graph.
    CycleDetected,
    /// A referenced dependency does not exist.
    MissingDependency { task_id: String, dependency: String },
    /// A plan referenced in cross-plan deps was not found.
    MissingPlan { plan_id: String },
}
```

Cycle detection is critical for safety — a cyclic dependency graph would cause
the executor to deadlock, with tasks waiting for each other indefinitely.

---

## References

- The wave scheduling approach draws on topological sort algorithms (Kahn 1962)
  and list scheduling heuristics from the task scheduling literature.
- Topcuoglu, H., Hariri, S. & Wu, M.-Y. (2002). Performance-effective and
  low-complexity task scheduling for heterogeneous computing. *IEEE Trans.
  Parallel and Distributed Systems*, 13(3), 260–274. (HEFT algorithm)
- The file-conflict inference mechanism is analogous to lock-based concurrency
  control in database systems, where conflicting transactions are serialized
  to prevent anomalies.
- Grassé, P.-P. (1959). La reconstruction du nid et les coordinations
  interindividuelles. *Insectes Sociaux*, 6(1), 41–80. (Stigmergic
  coordination — agents coordinate through shared artifacts, here the codebase
  files they modify.)
