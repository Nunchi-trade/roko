# Plan Discovery and DAG Construction

> Depth for [03-GRAPH.md](../../unified/03-GRAPH.md). How plans are discovered on disk, parsed into structured metadata, and assembled into a cross-plan executable Graph where each task is a Cell.

---

## What This Document Covers

Before any Cell can execute, the system must answer three questions: *what work exists*, *what depends on what*, and *what conflicts with what*. This document covers the pipeline from files-on-disk to a validated, topologically sorted Graph of Cells ready for the execution engine.

The pipeline has three stages:

```
Filesystem scan  -->  Plan parsing + ranking  -->  DAG construction + validation
  (discovery)         (frontmatter)                (UnifiedTaskDag)
```

Each stage produces data that the next stage consumes. No stage performs I/O beyond reading files. The entire pipeline is deterministic: given the same directory contents, it always produces the same Graph.

---

## Stage 1: Plan Discovery

### The Problem

Plans live on disk as directories containing markdown and TOML. The system must find them, parse their metadata, handle legacy formats, resolve conflicts, and produce a ranked list. This is the ingress point where human-authored documents become machine-readable Cells.

### Directory Layouts

Two layouts are supported. The structured layout takes precedence:

```
plans/
  01-workspace-scaffold/     # Structured layout (preferred)
    plan.md                  # Description + YAML frontmatter
    tasks.toml               # Task definitions (Cell specifications)
    CONTEXT.md               # Optional context (skipped by discovery)
  02-core-traits/
    plan.md
    tasks.toml
```

```
plans/
  01-workspace-scaffold.md   # Legacy flat layout (fallback)
  02-core-traits.md
```

When both `plans/03-foo/plan.md` and `plans/03-foo.md` exist, the directory layout wins. The flat file is silently skipped.

### Frontmatter as Cell Configuration

Each plan's YAML frontmatter is the configuration surface for the plan-as-Cell. In unified vocabulary, this is the `params` field of a `NodeKind::Cell` within the plan Graph.

Key fields and their Graph-level meanings:

| Frontmatter field | Graph meaning | Type |
|---|---|---|
| `plan` | `NodeId` -- stable identity for cross-references | `Option<String>` |
| `depends_on` | Forward edges to other plan Cells | `Vec<String>` |
| `parallel_with` | Advisory: explicitly safe for concurrent execution | `Vec<String>` |
| `crates_touched` | File-conflict inference input (synthetic edges) | `Vec<String>` |
| `priority` | Scheduling weight (higher = earlier in queue) | `Option<u32>` |
| `parallel_safe` | Whether tasks can run concurrently with other plans | `bool` (default `true`) |
| `estimated_tasks` | Advisory parallelism hint | `Option<usize>` |
| `estimated_minutes` | Advisory duration (feeds PERT estimation) | `Option<u32>` |

The frontmatter parser is BOM-tolerant (strips `U+FEFF`), handles CRLF, and uses `serde_yaml_ng`. Malformed YAML fails loudly with `DiscoveryError::BadFrontmatter` -- the system refuses to silently drop a plan.

### Validation

Validation is intentionally lax -- only load-bearing invariants trigger errors:

1. Plan ID must not be empty string (omitted is fine)
2. Estimated minutes must be > 0 if set
3. Estimated parallel width must be > 0 if set

This allows incremental plan authoring: start with prose, add metadata as the plan matures.

### Ranking

After discovery, plans are sorted by `rank_plans()`:

1. **Primary**: Priority descending (higher runs first)
2. **Secondary**: Numeric prefix ascending, lexicographic (lower numbers first)

The ranking determines initial queue order in the executor. The queue can be dynamically reordered during execution.

### Expressed as a Graph

Discovery itself is a Pipeline of three Cells:

```rust
// Pseudocode: discovery as a Pipeline Graph
Graph {
    nodes: [
        Node { id: "scan",     kind: Cell { ref: "FsScanCell" } },
        Node { id: "parse",    kind: Cell { ref: "FrontmatterParseCell" } },
        Node { id: "validate", kind: Cell { ref: "ValidationCell" } },
        Node { id: "rank",     kind: Cell { ref: "RankCell" } },
    ],
    edges: [
        Edge { from: "scan",     to: "parse" },
        Edge { from: "parse",    to: "validate" },
        Edge { from: "validate", to: "rank" },
    ],
}
// Input Signal: PathBuf (plans directory)
// Output Signal: Vec<PlanInfo> (ranked plan list)
```

Each Cell in this Pipeline can reject its input (malformed YAML, invalid fields), which short-circuits the Pipeline -- a Verify protocol applied inline.

---

## Stage 2: Task Definition and the TOML Schema

Each plan directory contains `tasks.toml`, which defines the Cells that make up the plan's internal Graph:

```toml
[[task]]
id = "t1"
description = "Implement the routing layer"
depends_on = ["t0"]
depends_on_plan = ["01-core"]
files = ["crates/roko-agent/src/router.rs"]
role = "implementer"
complexity = "standard"
verify = "cargo test -p roko-agent -- routing"
```

### TaskDef as Cell specification

Each `TaskDef` becomes a Node in the Graph. The mapping:

| TaskDef field | Graph/Cell concept |
|---|---|
| `id` | `NodeId` |
| `description` | `Node.label` + Cell prompt Signal |
| `depends_on` | Forward edges (intra-plan) |
| `depends_on_plan` | Forward edges (cross-plan, via synthetic `__whole__` nodes) |
| `files` | File-conflict inference input |
| `role` | Cell specialization (Implementer, Strategist, Auditor, etc.) |
| `complexity` | Route protocol input (T0/T1/T2 model selection) |
| `verify` | Verify protocol: post-execution check command |

---

## Stage 3: DAG Construction

The `UnifiedTaskDag` takes `Vec<(plan_id, Vec<TaskDef>)>` and builds the cross-plan execution Graph.

### Construction Algorithm

Six steps, each adding structure:

**Step 1 -- Collect nodes.** Every TaskDef from every plan becomes a Node, keyed by `GlobalTaskId` in format `"plan_id:task_id"`. This composite key ensures uniqueness across plans.

**Step 2 -- Resolve intra-plan edges.** For each task's `depends_on` list, create a forward edge from dependency to dependent. Both must share the same plan.

**Step 3 -- Resolve cross-plan edges.** For `depends_on_plan` references:
1. Create a synthetic `__whole__` node for the referenced plan
2. Add edges from every task in that plan to the `__whole__` node
3. Add an edge from `__whole__` to the dependent task

This ensures a task waits for an entire upstream plan to complete.

**Step 4 -- File-conflict inference.** For every pair of tasks from *different* plans: if their `files` sets intersect, add a synthetic dependency edge. Direction is deterministic (lexicographic plan ordering) to prevent deadlocks. This is analogous to lock-based concurrency control in databases -- conflicting transactions are serialized.

**Step 5 -- Reverse edge index.** For every forward edge A -> B, record a reverse edge B -> A. Used for fan-in detection and wave computation.

**Step 6 -- Cycle detection via Kahn's algorithm.** Topological sort with deterministic tie-breaking (lexicographic smallest node). If the sorted list is shorter than the node count, a cycle exists and the build fails.

```rust
// Pseudocode: Kahn's algorithm with deterministic ordering
fn topological_sort(nodes: &[Node], edges: &[Edge]) -> Result<Vec<NodeId>> {
    let mut in_degree: HashMap<NodeId, usize> = compute_in_degrees();
    let mut queue: BTreeSet<NodeId> = nodes_with_zero_in_degree();  // sorted!
    let mut order = Vec::new();

    while let Some(node) = queue.pop_first() {  // deterministic: smallest first
        order.push(node);
        for successor in adjacency[node] {
            in_degree[successor] -= 1;
            if in_degree[successor] == 0 {
                queue.insert(successor);
            }
        }
    }

    if order.len() != nodes.len() {
        return Err(DagError::CycleDetected {
            tasks: nodes_not_in_order()
        });
    }
    Ok(order)
}
```

### Wave Computation

Waves are groups of Cells that can execute in parallel -- the Graph's natural parallelism layers:

1. Start with all zero-in-degree nodes as Wave 0
2. If wave exceeds `max_wave_width` (from `DagConfig`), split into chunks
3. Mark wave tasks as "executed," find newly unblocked nodes -> next wave
4. Repeat until all nodes assigned

Example with file-conflict edges:

```
Plan A: A1 -> A2 -> A3  (touches: roko-core/)
Plan B: B1 -> B2         (touches: roko-core/, roko-agent/)
Plan C: C1               (touches: roko-gate/)
Plan D: D1 -> D2         (touches: roko-agent/)

File-conflict edges: A <-> B (roko-core), B <-> D (roko-agent)

Wave 0: [A1, C1]         -- no conflicts between A and C
Wave 1: [A2, B1]         -- B1 starts after A1 (file overlap resolved)
Wave 2: [A3, B2, D1]     -- D1 starts after B1 (file overlap resolved)
Wave 3: [D2]             -- depends on D1
```

### Critical Path Estimation

The longest path through the DAG determines minimum wall-clock time regardless of parallelism. Computed via dynamic programming on the topological order:

```
path_length[node] = 1  (initial)
for node in topological_order:
    for successor in adjacency[node]:
        path_length[successor] = max(path_length[successor], path_length[node] + 1)
critical_path = max(path_length[*])
```

For uncertain agent task durations, PERT (Program Evaluation and Review Technique) uses a Beta distribution:

```
Expected = (optimistic + 4 * most_likely + pessimistic) / 6
StdDev   = (pessimistic - optimistic) / 6
```

The three-point estimates can be derived from historical efficiency events in `.roko/learn/efficiency.jsonl`.

### Advanced Optimization Passes

The DAG supports several optimization passes drawn from dataflow systems:

**Task Fusion** (Dask's `fuse()`): Merge sequential single-dependency Cells into a compound Cell to reduce scheduling overhead. Only fuse same-complexity-tier tasks; never fuse if it reduces parallelism below `ave_width`.

**DAG Culling** (Dask's `cull()`): Remove Cells not required to produce target outputs. BFS backward from targets, keep only reachable nodes. O(V + E).

**Speculative Execution** (Spark): When a Cell on the critical path exceeds 1.5x the median duration of completed Cells in its wave, schedule a backup copy on a different worktree. First to complete wins; other is cancelled.

**Graph Partitioning** (METIS): For large DAGs (100+ tasks), partition into balanced clusters minimizing cross-cluster edges. Uses Heavy-Edge Matching -> Kernighan-Lin bisection -> Fiduccia-Mattheyses refinement.

### Incremental Recomputation

When the DAG changes during execution (task added, removed, or re-prioritized), only affected nodes need recomputation. Drawing on Adapton (Hammer et al., PLDI 2014) and Salsa (rust-analyzer):

1. **Dirty phase**: Mark changed node + all transitively downstream nodes as dirty. O(affected edges).
2. **Clean phase** (demand-driven): When scheduling metadata is requested, recompute input hash. If unchanged, clean without recomputation (Salsa's "backdate"). If changed, recompute wave assignment and critical path.
3. **Durability levels**: Structural dependencies (High) skip verification when only metadata (Low) changes.

### Dynamic DAG Mutation

Plans can grow during execution via five mutation operations:

| Mutation | Trigger | Safety check |
|---|---|---|
| `AddTask` | Gate failure reveals missing subtask | Cycle check (local DFS) |
| `RemoveTask` | Task already done (discovered during review) | Only pending tasks |
| `SplitTask` | Task too large for single agent | Deps transferred to subtasks |
| `AddDependency` | Research reveals new dependency | Cycle check |
| `UpdateTaskMetadata` | Duration/file list change | Triggers incremental recomputation |

Invariants: completed tasks are immutable, running tasks can only be cancelled then mutated, cycle detection is mandatory on every structural change.

---

## Reality Check: Implementation vs. Spec

The mori-diffs document (`02-PLAN-EXECUTION.md`) reveals several gaps between the implementation and this spec-level design:

**Sentinel-based resolution (not DAG).** The current runner v2 event loop uses sentinel task names (`"next"`, `"fix"`, `"regen-verify"`) instead of proper DAG resolution. It walks all tasks and picks the first whose `is_ready()` returns true -- a linear scan sorted by string ID, not topological order. No cycle detection, no parallelism within a plan, no cross-plan dependency enforcement.

**No file-conflict inference at runtime.** `UnifiedTaskDag` exists in `roko-orchestrator` with file-overlap detection, but the runner v2 event loop does not use it. Tasks are dispatched one-at-a-time.

**No wave computation at dispatch time.** Waves are computed by `UnifiedTaskDag::waves()` but not consulted by the event loop's tick cycle.

**Advanced passes are aspirational.** Task fusion, speculative execution, graph partitioning, and incremental recomputation are designed but not implemented.

**What IS wired:** Plan discovery (both layouts), frontmatter parsing, TOML task parsing, plan ranking, and basic topological sort in `roko-orchestrator/src/dag.rs`. The foundation exists; the gap is in the runner's use of it.

---

## What This Enables

1. **Multi-plan parallel execution**: File-conflict edges prevent concurrent modification of the same code, while unrelated plans run in parallel.
2. **Deterministic scheduling**: Same inputs always produce the same topological order, wave assignment, and critical path.
3. **Incremental plan evolution**: Plans can grow or shrink during execution without rebuilding the entire Graph.
4. **Cost estimation**: Critical path + PERT gives probabilistic completion time, enabling budget decisions before execution starts.

## Feedback Loops

- **Lens: DiscoveryLens** -- observes how many plans are discovered, how many have frontmatter, how many have validation errors. Feeds into workspace health reporting.
- **Loop: EfficientDAG** -- historical task durations (from efficiency events) feed back into PERT estimates, improving critical path accuracy over time. The DAG learns which task types take longer.
- **Loop: ConflictLearning** -- file-conflict edges that consistently prevent parallelism signal that those plans should be restructured or merged.

## Open Questions

1. **Dynamic vs. static dependencies.** The current design uses static dependencies declared in `tasks.toml`. Should agents be able to declare new dependencies at runtime (shifting the scheduler from "topological" to "suspending" in the Build Systems a la Carte taxonomy)?
2. **Cross-plan task parallelism.** The runner currently dispatches one task at a time globally. The DAG computes waves that allow parallelism, but the event loop doesn't consume them. Bridging this gap requires per-plan agent handles instead of a single global `agent_handle`.
3. **Conflict granularity.** File-conflict inference operates at the crate-directory level (`crates_touched`). Finer granularity (per-file) would allow more parallelism but at higher discovery cost. Is the trade-off worth it?
4. **Plan identity stability.** Plans are identified by their directory base name (`01-workspace-scaffold`). Renaming a directory breaks all cross-plan references. Should plan identity be content-addressed (like `GraphId`) instead of path-based?
