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

---

## DAG Optimization Passes

Beyond basic topological sort and wave computation, the DAG supports several
optimization passes inspired by dataflow system optimizers (Dask, Flink, Spark)
and classical project scheduling algorithms (CPM, PERT).

### Critical Path Analysis (CPM/PERT)

The current `stats()` method computes a simple critical path length. A more
sophisticated analysis uses the **Critical Path Method** with the
forward/backward pass algorithm:

```rust
/// Extended critical path analysis with earliest/latest start times.
pub struct CpmAnalysis {
    /// For each task: earliest it can start.
    pub earliest_start: HashMap<String, Duration>,
    /// For each task: latest it can start without delaying the project.
    pub latest_start: HashMap<String, Duration>,
    /// For each task: total float (slack).
    /// Float = latest_start - earliest_start.
    /// Tasks with zero float are on the critical path.
    pub total_float: HashMap<String, Duration>,
    /// For each task: free float (slack before delaying successors).
    /// Free_float = min(ES(successors)) - EF(self).
    pub free_float: HashMap<String, Duration>,
    /// The critical path (ordered list of zero-float tasks).
    pub critical_path: Vec<String>,
    /// Minimum project duration.
    pub min_duration: Duration,
}

impl UnifiedTaskDag {
    /// CPM forward and backward pass. O(V + E) — two passes over the DAG.
    ///
    /// Forward pass computes earliest start/finish:
    ///   ES(v) = max(EF(u) for all predecessors u)  // 0 for root
    ///   EF(v) = ES(v) + duration(v)
    ///
    /// Backward pass computes latest start/finish:
    ///   LF(v) = min(LS(u) for all successors u)    // EF for sink
    ///   LS(v) = LF(v) - duration(v)
    ///
    /// Float: TF(v) = LS(v) - ES(v).
    /// Critical path: all tasks where TF = 0.
    pub fn cpm_analysis(
        &self,
        durations: &HashMap<String, Duration>,
    ) -> CpmAnalysis { /* ... */ }
}
```

For agent tasks where durations are uncertain, the **PERT** extension uses a
Beta distribution with three estimates:

```
Expected_time = (optimistic + 4 × most_likely + pessimistic) / 6
Std_dev       = (pessimistic - optimistic) / 6
Variance      = ((pessimistic - optimistic) / 6)²
```

The project completion probability is estimated by summing along the critical
path and applying the Central Limit Theorem: `P(finish < T) = Φ((T - E) / σ)`.

For Roko, PERT estimates can be derived from historical efficiency events in
`.roko/learn/efficiency.jsonl` — past task durations grouped by complexity tier
provide the three-point estimates.

### Task Fusion

Inspired by Dask's `fuse()` pass and Flink's operator chaining, task fusion
merges sequential single-dependency tasks into a single compound task to reduce
scheduling overhead:

```rust
/// Configuration for the task fusion optimization pass.
pub struct FusionConfig {
    /// Maximum chain length to fuse (avoids mega-tasks).
    /// Default: 5. Range: 2..=20.
    pub max_chain_length: usize,
    /// Average width threshold — don't fuse if it reduces parallelism
    /// below this. Mirrors Dask's ave_width parameter.
    /// Default: 1.0.
    /// Derived max_height = 1.5 + ave_width × ln(ave_width + 1).
    pub ave_width: f64,
    /// Only fuse tasks of the same complexity tier.
    /// Default: true.
    pub same_tier_only: bool,
}

/// A fused task group that executes as a single agent dispatch.
pub struct FusedTaskGroup {
    /// The synthetic task ID for the fused group.
    pub fused_id: String,
    /// Original task IDs in execution order.
    pub tasks: Vec<String>,
    /// Combined estimated duration.
    pub combined_duration: Duration,
    /// Files touched by all tasks in the group.
    pub combined_files: HashSet<String>,
}

impl UnifiedTaskDag {
    /// Identify linear chains (single-input, single-output) and fuse them.
    ///
    /// Algorithm:
    /// 1. Find all tasks with exactly one successor and whose successor
    ///    has exactly one predecessor (linear chain candidates).
    /// 2. Walk forward to build chains up to max_chain_length.
    /// 3. Check that fusion doesn't reduce parallelism below ave_width.
    /// 4. Replace chain nodes with a single FusedTaskGroup node.
    ///
    /// Complexity: O(V + E).
    pub fn fuse_linear_chains(
        &mut self,
        config: &FusionConfig,
    ) -> Vec<FusedTaskGroup> { /* ... */ }
}
```

**When fusion helps**: Chains of small, sequential tasks where scheduling
overhead (agent spawn, prompt assembly, model routing) dominates actual work.

**When to avoid**: If tasks in the chain require different agent roles, touch
different codebases, or have different complexity tiers.

### Speculative Execution

Inspired by Spark's speculative execution for straggler mitigation, the DAG can
support **backup task** scheduling when a task exceeds expected duration:

```rust
/// Configuration for speculative execution of straggler tasks.
pub struct SpeculationConfig {
    /// Enable speculative execution. Default: false.
    pub enabled: bool,
    /// Fraction of tasks that must complete before speculation starts.
    /// Default: 0.75 (75%). Range: 0.5..=1.0.
    pub quantile: f64,
    /// Multiplier over median duration to be considered a straggler.
    /// Default: 1.5 (50% slower than median). Range: 1.1..=5.0.
    pub multiplier: f64,
    /// Minimum elapsed time before a task is eligible.
    /// Default: 120s. Range: 30s..=600s.
    pub min_runtime: Duration,
    /// Maximum concurrent speculative tasks.
    /// Default: 2. Range: 1..=4.
    pub max_speculative: usize,
}

impl UnifiedTaskDag {
    /// Identify straggler tasks eligible for speculative re-execution.
    ///
    /// Algorithm (adapted from Spark's TaskSetManager):
    /// 1. Wait until quantile fraction of wave tasks complete.
    /// 2. Compute median duration of completed tasks.
    /// 3. For each running task:
    ///    if runtime > multiplier × median AND runtime > min_runtime
    ///    AND only one copy running AND max_speculative not exceeded:
    ///    → mark as speculative, schedule backup on DIFFERENT worktree.
    /// 4. First copy to complete wins; other is cancelled via CancelToken.
    pub fn find_speculative_candidates(
        &self,
        completed_durations: &HashMap<String, Duration>,
        running_since: &HashMap<String, Instant>,
        config: &SpeculationConfig,
    ) -> Vec<String> { /* ... */ }
}
```

Speculative execution trades budget for latency — most valuable for tasks on
the critical path where a straggler delays the entire plan.

### DAG Culling

Adapted from Dask's `cull()` pass, DAG culling removes tasks not required to
produce the target outputs. This is useful when a plan is partially completed
and only a subset of remaining tasks matter:

```rust
impl UnifiedTaskDag {
    /// Remove tasks not required to produce the given target task IDs.
    /// Returns the number of tasks culled.
    ///
    /// Algorithm: BFS backward from targets, collecting reachable nodes.
    /// Remove everything else. O(V + E).
    pub fn cull(&mut self, targets: &[String]) -> usize { /* ... */ }
}
```

### Graph Partitioning (METIS-Inspired)

For very large DAGs (100+ tasks across 10+ plans), partitioning groups tasks
into balanced clusters that minimize cross-cluster dependencies:

```rust
/// Partition the DAG into k balanced groups.
///
/// Algorithm (simplified METIS multilevel scheme):
/// Phase 1 — Coarsening: Repeatedly contract graph via Heavy-Edge
///   Matching (preferentially match nodes sharing files). O(E)/level.
/// Phase 2 — Initial Partition: Bisect coarsened graph via
///   Kernighan-Lin. O(V² log V) on the tiny coarsened graph.
/// Phase 3 — Uncoarsening: Project back, refine with
///   Fiduccia-Mattheyses boundary moves. O(E)/level.
///
/// Overall complexity: O(V + E).
pub fn partition(&self, k: usize) -> Vec<DagPartition> { /* ... */ }

pub struct DagPartition {
    pub partition_id: usize,
    pub tasks: Vec<String>,
    /// Cross-partition dependencies requiring coordination.
    pub cut_edges: usize,
    /// Estimated total work in this partition.
    pub total_work: Duration,
}
```

---

## Incremental Computation: Re-Executing Only Changed Subgraphs

When a plan is modified during execution (tasks added, removed, or
re-prioritized), the entire DAG need not be rebuilt. Drawing on Adapton
(Hammer et al., PLDI 2014) and Salsa (used in rust-analyzer), the DAG supports
selective invalidation and reconstruction.

### The Dirty/Clean Propagation Model

```rust
/// Tracks which DAG nodes need recomputation after a change.
pub struct IncrementalDag {
    dag: UnifiedTaskDag,
    /// Dirty nodes whose scheduling metadata is stale.
    dirty: HashSet<String>,
    /// Per-node input hash (dependencies + files). Detects whether a
    /// "dirty" node actually changed or can be cleaned without recomputation.
    input_hashes: HashMap<String, [u8; 32]>,
    /// Global revision counter — increments on every input change.
    revision: u64,
    /// Per-node "verified at" revision. If verified_at == revision,
    /// the node's metadata is current.
    verified_at: HashMap<String, u64>,
}

impl IncrementalDag {
    /// Phase 1: Dirtying. Mark the changed task and all transitively
    /// reachable downstream tasks as dirty.
    /// Stops at already-dirty nodes (amortized cost).
    /// Complexity: O(affected edges), NOT O(total graph).
    pub fn mark_dirty(&mut self, task_id: &str) { /* ... */ }

    /// Phase 2: Cleaning (demand-driven). When scheduling metadata is
    /// requested for a node:
    /// 1. If verified_at == revision → return cached (memoization hit).
    /// 2. If dirty, recompute input hash and compare to stored hash.
    ///    - Unchanged → clean without recomputation (Salsa's "backdate").
    ///    - Changed → recompute wave assignment, critical path, etc.
    /// 3. Update verified_at to current revision.
    pub fn ensure_clean(&mut self, task_id: &str) { /* ... */ }

    /// Durability optimization (from Salsa): Skip verification for nodes
    /// whose inputs are all at a higher durability level than the change.
    pub fn set_durability(&mut self, task_id: &str, level: Durability) { /* ... */ }
}

pub enum Durability {
    /// Task metadata (estimated duration, priority). Changes frequently.
    Low,
    /// File lists, crate associations. Changes occasionally.
    Medium,
    /// Structural dependencies (depends_on). Changes rarely.
    High,
}
```

### Build Systems à la Carte Classification

Following Mokhov, Mitchell & Peyton Jones (ICFP 2018), Roko's DAG sits at a
specific point in the build system design space:

| Dimension | Roko's Choice | Alternative |
|-----------|--------------|-------------|
| **Scheduler** | Topological (static deps known at plan parse time) | Suspending (deps discovered at runtime) |
| **Rebuilder** | Verifying traces (compare input hashes) | Dirty bit (conservative) or Constructive traces (cloud cache) |
| **Dependency type** | Applicative (static — all deps in `tasks.toml`) | Monadic (dynamic — deps discovered during execution) |

This maps Roko closest to **Ninja** (topological + verifying traces). If
dynamic DAG modification is added, the scheduler shifts to **restarting** or
**suspending**, moving closer to **Shake** or **Bazel**.

---

## Dynamic DAG Modification

Plans can grow or shrink during execution based on gate feedback, agent
discoveries, or operator intervention. Inspired by Flyte's `@dynamic`
workflows, Prefect's code-as-workflow model, and Airflow's Dynamic Task Mapping.

### DAG Mutation Operations

```rust
/// Operations that modify the DAG during execution.
pub enum DagMutation {
    /// Add a new task to an existing plan. Must not create a cycle.
    AddTask { plan_id: String, task: TaskDef },
    /// Remove a pending task. Dependencies re-linked: predecessors
    /// connect directly to successors.
    RemoveTask { task_id: String },
    /// Split a task into subtasks. Original replaced; deps transferred.
    SplitTask { task_id: String, subtasks: Vec<TaskDef> },
    /// Add a dependency edge. Must not create a cycle.
    AddDependency { from: String, to: String },
    /// Update estimated duration or file list. Triggers incremental
    /// recomputation of waves and critical path.
    UpdateTaskMetadata {
        task_id: String,
        estimated_duration: Option<Duration>,
        files: Option<Vec<String>>,
    },
}

/// Result of applying a mutation.
pub struct MutationResult {
    pub success: bool,
    /// Tasks whose scheduling metadata was invalidated.
    pub invalidated: Vec<String>,
    /// Recomputed wave assignments (only for invalidated tasks).
    pub recomputed_waves: Vec<ExecutionWave>,
    /// Updated critical path (if it changed).
    pub new_critical_path: Option<Vec<String>>,
}

impl UnifiedTaskDag {
    /// Apply a mutation during execution.
    ///
    /// Consistency rules:
    /// 1. Completed tasks are immutable — their nodes and edges cannot change.
    /// 2. Running tasks cannot be removed (only cancelled then mutated).
    /// 3. Cycle check on every AddTask/AddDependency (local DFS from new
    ///    edge target to source). Worst case O(V + E), typically O(subgraph).
    /// 4. File-conflict edges are recomputed for affected task pairs.
    /// 5. Incremental recomputation via dirty/clean model.
    pub fn apply_mutation(
        &mut self,
        mutation: DagMutation,
    ) -> Result<MutationResult, DagError> { /* ... */ }
}
```

### Triggers for Dynamic Modification

| Trigger | Mutation | Source |
|---------|----------|--------|
| Gate failure reveals missing subtask | `AddTask` | AutoFixer agent |
| Task proves too large for single agent | `SplitTask` | Conductor complexity detection |
| Research reveals new dependency | `AddDependency` | Strategist agent |
| Task already done (discovered during review) | `RemoveTask` | Auditor |
| Operator adds urgent task | `AddTask` with high priority | Manual intervention |
| Plan repair from PDDL solver | Batch of `AddTask` + `RemoveTask` | Plan repair engine |

### Consistency Invariants

Following patterns from Flyte (append-only expansion), Temporal (deterministic
replay), and Prefect (state transition rules):

1. **Completed tasks are immutable** — preserves event log and snapshot integrity
2. **Running tasks are observable but not mutable** — can only be cancelled then mutated
3. **Pending tasks are fully mutable** — add, remove, split, reparent
4. **File-conflict edges recomputed** — after any file list change
5. **Cycle detection mandatory** — on every AddTask/AddDependency

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
  coordination — agents coordinate through shared artifacts.)
- Hammer, M. A. et al. (2014). Adapton: Composable, demand-driven incremental
  computation. *PLDI 2014*. (Demand-driven dirtying/cleaning for incremental
  DAG recomputation.)
- Mokhov, A., Mitchell, N. & Peyton Jones, S. (2018). Build systems à la carte.
  *ICFP 2018*. (Taxonomy: schedulers × rebuilders × dependency types.)
- Salsa. Incremental computation for rust-analyzer. Red-green validation,
  durability levels, backdate optimization. *github.com/salsa-rs/salsa*
- Karypis, G. & Kumar, V. (1998). A fast and high quality multilevel scheme
  for partitioning irregular graphs. *SIAM J. Sci. Comput.*, 20(1), 359–392.
  (METIS graph partitioning.)
- Dean, J. & Ghemawat, S. (2008). MapReduce: Simplified data processing on
  large clusters. *Comm. ACM*, 51(1), 107–113. (Speculative execution.)
- Rocklin, M. (2015). Dask: Parallel computation with blocked algorithms and
  task scheduling. *SciPy 2015*. (Task graph optimization: cull, inline, fuse.)
