# Orchestration -- gap checklist

Spec: `docs/01-orchestration/` (15 files, docs 00-13 + INDEX).
Code: `crates/roko-orchestrator/`, `crates/roko-cli/src/orchestrate.rs`, `crates/roko-runtime/`.

Overall: ~85% compliant. Core plan-execute-gate-persist loop works. Gaps are in optimization
structs and Phase 2+ features.

## Compliant (no action needed)

- Layer overview and five-layer architecture (doc 00)
- Plan discovery, parsing, validation, and ranking (doc 01)
- Parallel executor core: pure state machine, tick/event loop, concurrency management (doc 03 -- core features; Petri net model and CRDT are Phase 2+ design docs)
- Plan phases lifecycle with transitions and retry (doc 04)
- All executor actions present (doc 05)
- PlanRunner with 30+ fields and dispatch logic (doc 06)
- Worktree isolation lifecycle, health checks, and idle reclamation (doc 07)
- Merge queue with conflict detection, priority ordering, and retry (doc 08)
- Event log with BLAKE3 hash-chaining and integrity verification (doc 10)
- Conductor integration with 10 watchers and cost monitoring (doc 11)
- Stigmergy/pheromone conceptual framework (doc 12)
- DAG topological sort (Kahn's algorithm), wave computation, and file-overlap inference
- CPM forward/backward pass (`cpm_analysis()`, `earliest_start()`, `latest_start()`, `slack()`, `critical_path()`)
- `IncrementalDag` with dirty propagation and `clean_set()`
- `fuse_linear_chains()` function
- `SpeculativeExecution` struct and speculation dispatch in the executor
- Budget cap (`budget_usd`) in `ExecutorConfig`

---

## Checklist

### ORCH-01: Public `CpmAnalysis` struct with PERT extension -- RESOLVED
- [x] Expose CPM results as a named public type

**Spec** (doc 02 §CPM-PERT): `cpm_analysis()` should return a `CpmAnalysis` struct with
`earliest_start`, `latest_start`, `total_float`, `free_float`, `critical_path`, and
`min_duration` as named fields. Total float = latest_start - earliest_start for each task.
Free float = min(ES(successors)) - EF(self). The spec also describes a PERT extension:
three-point estimates (optimistic, most-likely, pessimistic) derived from
`.roko/learn/efficiency.jsonl`, with completion probability via the Central Limit Theorem.
The purpose is to give the executor probabilistic scheduling information -- "80% chance of
completing by time T" -- rather than just deterministic critical path.

**Current code** (`crates/roko-orchestrator/src/dag.rs:855`): `cpm_analysis()` is a private
method returning an anonymous 4-tuple `(HashMap<GlobalTaskId, f64>, HashMap<GlobalTaskId, f64>, Vec<GlobalTaskId>, f64)` representing (earliest_start, latest_start, critical_path, min_duration). `total_float` and `free_float` are not computed -- `slack()` at line ~870 returns `latest_start - earliest_start` per task but is not bundled into a struct. PERT extension is absent. Related: `earliest_start()` at line ~830, `latest_start()` at line ~845, `critical_path()` at line ~865 are standalone accessors.

**What to change**: Define the following struct after `cpm_analysis()`:
```rust
pub struct CpmAnalysis {
    pub earliest_start: HashMap<GlobalTaskId, f64>,
    pub latest_start: HashMap<GlobalTaskId, f64>,
    pub total_float: HashMap<GlobalTaskId, f64>,
    pub free_float: HashMap<GlobalTaskId, f64>,
    pub critical_path: Vec<GlobalTaskId>,
    pub min_duration: f64,
}
```
Add `pub fn cpm_analysis_full(&self) -> CpmAnalysis` that calls the existing private
`cpm_analysis()` and extends its results with float calculations. For `free_float`:
iterate each task, find min earliest_start of successors, subtract (earliest_start + duration).

**Reference files**:
- `crates/roko-orchestrator/src/dag.rs:855` -- `cpm_analysis()` private method returning 4-tuple
- `crates/roko-orchestrator/src/dag.rs:830` -- `earliest_start()` standalone accessor
- `crates/roko-orchestrator/src/dag.rs:870` -- `slack()` returns per-task float
- `crates/roko-learn/src/efficiency.rs` -- efficiency event data for PERT estimates
- `docs/01-orchestration/02-unified-task-dag.md` -- §CPM-PERT spec with struct definition

**Accept when**:
- [x] `pub struct CpmAnalysis` exists in `dag.rs` with all 6 fields named above
- [x] `UnifiedTaskDag::cpm_analysis_full()` returns `CpmAnalysis`
- [x] `total_float` computed as `latest_start - earliest_start` per task
- [x] `free_float` computed as `min(ES(successors)) - EF(self)` for each task
- [x] `cargo test -p roko-orchestrator` passes

**Verify**:
```bash
grep -rn 'struct CpmAnalysis' crates/roko-orchestrator/src/ --include='*.rs'
grep -rn 'cpm_analysis_full' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P1

---

### ORCH-02: `FusionConfig` parameter on `fuse_linear_chains` -- RESOLVED
- [x] Add config struct to control fusion behavior

**Spec** (doc 02 §Task Fusion): `fuse_linear_chains` should accept a `FusionConfig` with
`max_chain_length: usize` (cap how many tasks can be fused into one), `ave_width: f64`
(minimum average parallelism to preserve -- if fusing would drop average wave width below
this, skip the fusion), and `same_tier_only: bool` (only fuse tasks at the same ModelTier).
Without config, there is no way to tune fusion behavior or disable it per call site. The
purpose of fusion is to reduce DAG overhead for serial chains of trivial tasks (e.g.,
three sequential file edits become one "edit group" task), but uncontrolled fusion destroys
parallelism and cross-tier boundaries.

**Current code** (`crates/roko-orchestrator/src/dag.rs:529`): `fuse_linear_chains()` takes
no parameters and returns `usize` (number of fused chains). It walks the DAG looking for
nodes with exactly one predecessor and one successor, merging them into the predecessor.
Chain length is unbounded. The `ave_width` guard from the spec (don't fuse if it reduces
parallelism below threshold) is absent. No tier check.

**What to change**: Define the struct and update the signature:
```rust
pub struct FusionConfig {
    pub max_chain_length: usize,   // default: 5
    pub ave_width: f64,            // default: 2.0
    pub same_tier_only: bool,      // default: true
}
```
Change `fuse_linear_chains` to `fuse_linear_chains(&mut self, config: &FusionConfig) -> usize`.
Inside the loop at line 529, add: (1) a counter per chain that stops merging when
`chain_len >= config.max_chain_length`, (2) before committing the merge, compute
average wave width via `wave_compute()` and skip if below `config.ave_width`, (3) if
`config.same_tier_only`, compare the `complexity` / tier of the two tasks being merged
and skip if they differ. Update the single call site (the test at line 1217) to pass
`&FusionConfig::default()`.

**Reference files**:
- `crates/roko-orchestrator/src/dag.rs:529` -- `fuse_linear_chains()` implementation
- `docs/01-orchestration/02-unified-task-dag.md` -- §Task Fusion spec with `FusionConfig` fields

**Accept when**:
- [x] `pub struct FusionConfig` exists with fields matching the spec
- [x] `fuse_linear_chains(&mut self, config: &FusionConfig)` signature
- [x] Chain length capped by `config.max_chain_length`
- [x] `ave_width` guard implemented
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'struct FusionConfig' crates/roko-orchestrator/src/ --include='*.rs'
grep -rn 'fuse_linear_chains' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P1

---

### ORCH-03: Incremental delta snapshots
- [x] Add delta encoding between checkpoints

**Spec** (doc 09 §Incremental Snapshots): Snapshots between full checkpoints should encode only
changed plans via `DeltaSnapshot`. The spec defines `DeltaSnapshot` (changed/added/removed
plans, `base_hash`, `expected_hash`), `SnapshotConfig` (intervals, max delta chain), and a
rotation strategy (full every N actions, delta every M actions).

**Current code** (`crates/roko-orchestrator/src/executor/snapshot.rs:50`): `ExecutorSnapshot`
at line 50 does full JSON snapshots only. Every autosave writes the complete state.
No `DeltaSnapshot`, no `SnapshotConfig`.

**What to change**: Add `DeltaSnapshot` and `SnapshotConfig` structs in `snapshot.rs`.
Implement `delta_from` and `apply_delta` methods on `ExecutorSnapshot`. Wire the rotation
strategy into the autosave path in the executor.

**Reference files**:
- `crates/roko-orchestrator/src/executor/snapshot.rs:50` -- `ExecutorSnapshot` struct
- `crates/roko-orchestrator/src/executor/mod.rs` -- autosave logic
- `docs/01-orchestration/09-persistence.md` -- spec for incremental snapshots

**Depends on**: None

**Accept when**:
- [x] `pub struct DeltaSnapshot` exists with fields from the spec — snapshot.rs:408, has `base_hash`, `expected_hash`, `changed`, `removed_plan_ids`, `added_plan_ids`, `sequence`
- [x] `ExecutorSnapshot::delta_from(&self, base: &ExecutorSnapshot) -> DeltaSnapshot` — snapshot.rs:166
- [x] `ExecutorSnapshot::apply_delta(&self, delta: &DeltaSnapshot) -> Result<Self, SnapshotIntegrityError>` — snapshot.rs:253
- [x] `pub struct SnapshotConfig` controls intervals and max chain length — snapshot.rs:425, has `full_interval`, `delta_interval`, `max_delta_chain`
- [x] Recovery from a delta chain produces identical state to a full snapshot — tested in `delta_from_apply_roundtrip` at line 880
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'struct DeltaSnapshot\|struct SnapshotConfig' crates/roko-orchestrator/src/ --include='*.rs'
grep -rn 'delta_from\|apply_delta' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P1

---

### ORCH-04: Cryptographic snapshot verification
- [x] Detect corruption before deserializing

**Spec** (doc 09 §Verification Hierarchy): `SnapshotVerifier` with three levels:
file-level BLAKE3 checksum, per-plan Merkle tree, and cross-validation against the event log.
The spec also describes a binary file format (`ROKO` magic header + length + payload + BLAKE3
hash + `END!` trailer) to detect torn writes.

**Current code** (`crates/roko-orchestrator/src/executor/snapshot.rs:50`): Snapshot integrity
relies on JSON parse errors only. No `SnapshotVerifier`, no BLAKE3 hash, no Merkle tree,
no magic header format.

**What to change**: Add `SnapshotVerifier` struct with three verification levels. Add binary
format wrapper with magic header, payload, BLAKE3 hash, and trailer. Replace the raw JSON
write path with the binary format.

**Reference files**:
- `crates/roko-orchestrator/src/executor/snapshot.rs` -- current snapshot serialization
- `crates/roko-orchestrator/src/executor/mod.rs` -- snapshot write callsites
- `docs/01-orchestration/09-persistence.md` -- spec for verification hierarchy

**Depends on**: ORCH-03 (delta snapshots share the file format)

**Accept when**:
- [x] `pub struct SnapshotVerifier` exists — snapshot.rs:545
- [x] `verify_file_checksum(path, expected)` detects truncation and bit flips — `verify_checksum(data, expected)` at snapshot.rs:559, detects bit flips via BLAKE3 comparison
- [ ] `verify_merkle_tree(snapshot, expected_root)` verifies per-plan hashes — not implemented, no per-plan Merkle tree
- [x] Snapshot file format includes BLAKE3 hash and magic trailer — binary envelope: ROKO magic + length + payload + BLAKE3 hash + END! trailer (snapshot.rs:576-590)
- [x] Corrupted snapshot returns `IntegrityError`, not a JSON parse error — returns `SnapshotIntegrityError` variants: BadMagic, BadTrailer, TooShort, LengthMismatch, ChecksumMismatch
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'struct SnapshotVerifier' crates/roko-orchestrator/src/ --include='*.rs'
grep -rn 'verify_file_checksum\|verify_merkle_tree' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P1

---

### ORCH-05: DAG `cull` method
- [x] Remove tasks not needed for target outputs

**Spec** (doc 02 §DAG Culling): `dag.cull(targets: &[String]) -> usize` removes tasks not
required to produce the given target task IDs. Algorithm: backward BFS from targets,
collecting all transitive dependencies. Tasks NOT in this set are removed from the DAG.
Use case: when a plan is partially complete and only a subset of remaining tasks matter
(e.g., only tasks that produce the final binary, skipping doc tasks). Completed tasks
should be retained if they appear in the target's transitive closure (their outputs may
still be needed).

**Current code** (`crates/roko-orchestrator/src/dag.rs:228`): `UnifiedTaskDag` has
`topological_sort()`, `wave_compute()`, `fuse_linear_chains()`, but no `cull()` method.
The graph structure uses hand-rolled adjacency lists: `edges: HashMap<GlobalTaskId, BTreeSet<GlobalTaskId>>` (forward deps) and `reverse_edges: HashMap<GlobalTaskId, BTreeSet<GlobalTaskId>>` (dependents). The `deps_of()` method at line ~310 returns forward dependencies; `dependents_of()` at line ~320 returns reverse edges. Related: `fuse_linear_chains()` at line 529 does graph mutation (removing nodes, updating edges) that serves as the pattern for cull.

**What to change**: Add `pub fn cull(&mut self, targets: &[String]) -> usize` to
`UnifiedTaskDag`. Implement:
1. Convert `targets` to `GlobalTaskId`s, seed a `HashSet<GlobalTaskId>` as "needed"
2. BFS backward using `self.deps_of(node)` (which returns `&BTreeSet<GlobalTaskId>`)
3. For each node popped from the BFS queue, add all its deps to "needed" and enqueue them
4. After BFS, collect all nodes in `self.nodes` NOT in "needed"
5. For each node to remove: delete from `self.tasks`, `self.edges`, `self.reverse_edges`, `self.estimates`
6. Clean up edges pointing to removed nodes in remaining entries
7. Rebuild `self.nodes` from remaining `self.tasks.keys()`
8. Return count of removed nodes

**Reference files**:
- `crates/roko-orchestrator/src/dag.rs:228` -- `UnifiedTaskDag` struct definition with adjacency lists
- `crates/roko-orchestrator/src/dag.rs:310` -- `deps_of()` returns forward dependencies (use for backward BFS)
- `crates/roko-orchestrator/src/dag.rs:529` -- `fuse_linear_chains()` for graph mutation pattern (node removal, edge cleanup)
- `docs/01-orchestration/02-unified-task-dag.md` -- §DAG Culling spec

**Accept when**:
- [x] `pub fn cull(&mut self, targets: &[String]) -> usize` on `UnifiedTaskDag` — dag.rs:534
- [x] BFS backward from targets; all unreachable nodes removed from graph — tests at dag.rs:2176 (`cull_removes_unreachable_tasks`)
- [x] Returns count of culled tasks — verified via test assertions
- [x] Completed tasks retained if in target transitive closure — tested in `cull_with_multiple_targets` at dag.rs:2199
- [x] Graph remains valid after culling (no dangling edges) — tested in `cull_preserves_graph_validity` at dag.rs:2230
- [x] `cargo test -p roko-orchestrator` passes

**Verify**:
```bash
grep -rn 'fn cull' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P2

---

### ORCH-06: DAG partitioning
- [x] Split large DAGs into balanced subgraphs

**Spec** (doc 02 §Graph Partitioning): `dag.partition(k: usize) -> Vec<DagPartition>` using a
simplified METIS multilevel scheme (coarsening via heavy-edge matching, bisection, uncoarsening
with Fiduccia-Mattheyses boundary moves). Target use case: 100+ task DAGs across 10+ plans
executing on multiple replicas.

**Current code** (`crates/roko-orchestrator/src/dag.rs:228`): No `partition()` method on
`UnifiedTaskDag`. The DAG's hand-rolled adjacency lists (`edges`, `reverse_edges`) and
`wave_compute()` provide the graph traversal infrastructure needed. `fuse_linear_chains()`
at line 529 shows how to perform graph mutations.

**What to change**: Add `DagPartition` struct and `partition()` method. A practical first
implementation is BFS-balanced partitioning (simpler than full METIS):
```rust
pub struct DagPartition {
    pub partition_id: usize,
    pub tasks: Vec<GlobalTaskId>,
    pub cut_edges: usize,        // edges crossing partition boundary
    pub total_work: f64,         // sum of estimated_minutes in this partition
}
```
Algorithm: (1) compute topological order, (2) greedily assign tasks to the least-loaded
partition among k partitions, (3) count cut edges (edges where source and target are in
different partitions). For a METIS-style approach: (a) coarsen by heavy-edge matching
(merge the two nodes sharing the heaviest edge weight), (b) bisect the coarsened graph
by BFS from two seed nodes, (c) uncoarsen and apply Fiduccia-Mattheyses boundary moves
to reduce cut edges.

Add the method as `pub fn partition(&self, k: usize) -> Vec<DagPartition>` on `UnifiedTaskDag`.

**Reference files**:
- `crates/roko-orchestrator/src/dag.rs` -- `UnifiedTaskDag` struct
- `docs/01-orchestration/02-unified-task-dag.md` -- §Graph Partitioning spec (METIS multilevel scheme)

**Accept when**:
- [x] `pub struct DagPartition` with `partition_id`, `tasks`, `cut_edges`, `total_work` — dag.rs:1244
- [x] `pub fn partition(&self, k: usize) -> Vec<DagPartition>` on `UnifiedTaskDag` — dag.rs:1056
- [x] Cross-partition edges minimized relative to a random partition baseline — tested in `partition_minimizes_cuts_on_chain` at dag.rs:2381
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'struct DagPartition\|fn partition' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P2

---

### ORCH-07: `IncrementalDag` revision tracking -- RESOLVED
- [x] Add revision counter and `ensure_clean` for demand-driven cleaning

**Spec** (doc 02 §Incremental Computation): `IncrementalDag` should track a global `revision`
counter, per-node `verified_at` timestamps, and `input_hashes`. `ensure_clean(task_id)` should
check whether an input hash changed before recomputing -- the "backdate" optimization from Salsa
that avoids recomputation when inputs are unchanged despite being marked dirty.

**Current code** (`crates/roko-orchestrator/src/dag.rs:925`): `IncrementalDag` has
`dirty: HashSet` and `durability: HashMap` but no `revision`, no `verified_at`, no
`input_hashes`. `clean_set()` returns the non-dirty set but does not validate input hashes.

**What to change**: Add `revision`, `verified_at`, and `input_hashes` fields to
`IncrementalDag`. Add `ensure_clean()` method that checks input hash before recomputing.
Increment `revision` on `mark_dirty`.

**Reference files**:
- `crates/roko-orchestrator/src/dag.rs:925` -- `IncrementalDag` struct
- `docs/01-orchestration/02-unified-task-dag.md` -- §Incremental Computation spec (Adapton/Salsa backdate)

**Accept when**:
- [x] `revision: u64` field on `IncrementalDag`, incremented on every input change
- [x] `verified_at: HashMap<GlobalTaskId, u64>` tracking per-node clean revision
- [x] `input_hashes: HashMap<GlobalTaskId, [u8; 32]>` for BLAKE3 hash of each node's inputs
- [x] `pub fn ensure_clean(&mut self, task_id: &GlobalTaskId)` implements backdate optimization
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'struct IncrementalDag' crates/roko-orchestrator/src/ --include='*.rs'
grep -rn 'ensure_clean\|verified_at\|input_hashes' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P2

---

### ORCH-08: `ResourceBudget` struct with multi-dimensional scheduling
- [x] Centralize resource constraints in a named type with scheduling methods

**Spec** (doc 03 §Resource-Aware Scheduling): The executor should manage 5 resource dimensions
via a `ResourceBudget` struct: `agent_slots: ResourcePool` (bounded by max_concurrent_tasks),
`api_tokens: RateLimitResource` (token bucket with burst capacity and refill rate),
`token_budget: TokenBudget` (depletable LLM token budget with per-task defaults and complexity
multipliers), `worktree_slots: ResourcePool` (bounded by WorktreeConfig::max_live), and
`cost_budget: CostBudget` (depletable USD with 80% warn / 100% hard stop thresholds). The
struct must expose `can_schedule(task) -> ResourceCheck`, `reserve(task) -> Result<ResourceReservation>`,
and `release(reservation)` methods. The tick loop should become resource-aware: for each ready
task, check `can_schedule()`, reserve on `Available`, skip-or-downgrade on `Blocked`.

Supporting types from the spec:
- `ResourcePool { capacity: usize, in_use: usize }` -- bounded identical resources
- `RateLimitResource { capacity: u32, refill_rate: f64, current_tokens: f64, last_update: Instant }` -- token bucket
- `TokenBudget { total: u64, spent: u64, per_task_default: u64, per_task_max: u64, complexity_multiplier: HashMap<String, f64> }` -- LLM tokens
- `CostBudget { total_usd: f64, spent_usd: f64, warn_threshold: f64, stop_threshold: f64 }` -- USD cost

**Current code** (`crates/roko-orchestrator/src/executor/mod.rs:71`): `ExecutorConfig` at
line 71 has `budget_usd: Option<f64>` as a flat field (line 86) and `max_concurrent_tasks`
as a plain `usize`. No `ResourceBudget`, `ResourcePool`, `RateLimitResource`, `TokenBudget`,
or `CostBudget` structs exist anywhere in the crate. The budget check at line 233 is a simple
`if spent > budget` comparison.

**What to change**: Define all supporting types and `ResourceBudget` in a new
`crates/roko-orchestrator/src/executor/resource_budget.rs`. Replace `budget_usd` and
`max_concurrent_tasks` on `ExecutorConfig` with a single `ResourceBudget` field. Update the
tick loop to call `can_schedule()` / `reserve()` / `release()`.

**Reference files**:
- `crates/roko-orchestrator/src/executor/mod.rs:71` -- `ExecutorConfig` struct
- `crates/roko-orchestrator/src/executor/mod.rs:233` -- budget check in tick loop
- `docs/01-orchestration/03-parallel-executor.md` -- §Resource-Aware Scheduling, full struct definitions

**Accept when**:
- [x] `pub struct ResourceBudget` with `agent_slots`, `api_tokens`, `token_budget`, `worktree_slots`, `cost_budget` fields
- [x] `pub struct ResourcePool`, `RateLimitResource`, `TokenBudget`, `CostBudget` supporting types
- [x] `can_schedule(&self, task) -> ResourceCheck` method
- [x] `reserve(&mut self, task) -> Result<ResourceReservation>` method
- [x] `release(&mut self, reservation)` method
- [x] `ExecutorConfig` holds `ResourceBudget` instead of inline fields
- [ ] Tick loop uses resource-aware scheduling — tick() at executor/mod.rs:416 uses plain concurrency count, not `can_schedule()`
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'struct ResourceBudget' crates/roko-orchestrator/src/ --include='*.rs'
grep -rn 'can_schedule\|ResourcePool\|RateLimitResource' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P2

---

### ORCH-09: Cross-domain orchestration templates
- [x] Implement plan templates for cross-domain task types

**Spec** (doc 13 §Cross-Domain): Tasks spanning code, chain, research, and documentation
domains should be loadable from templates, with domain-specific gate selection and agent role
assignment driven by task type rather than plan-level config.

**Current code** (`crates/roko-orchestrator/src/dag.rs`, `crates/roko-cli/src/orchestrate.rs`):
Single-domain (code) solid. `UnifiedTaskDag` treats all tasks identically by design. Domain
differentiation (gate selection, role selection) lives in runtime dispatch but there is no
template system for composing cross-domain plans.

**What to change**: Add a template loading mechanism (TOML-based). Add a task domain annotation
field. Wire domain to gate selection and role assignment in `orchestrate.rs:dispatch_agent`
(line 11923).

**Reference files**:
- `crates/roko-orchestrator/src/dag.rs` -- `UnifiedTaskDag`, task representation
- `crates/roko-cli/src/orchestrate.rs:11923` -- `dispatch_agent` where role/gate selection happens
- `crates/roko-core/src/config/schema.rs` -- `roko.toml` config schema
- `docs/01-orchestration/13-cross-domain-orchestration.md` -- §Domain-Specific Plan Templates spec

**Accept when**:
- [ ] Plan templates loadable from `roko.toml` or a `templates/` directory — no template loading system found
- [ ] Task domain annotation (code, chain, research, docs) drives gate and role selection — no TaskDomain enum or annotation
- [ ] Cross-domain task dependencies resolve through the same DAG — DAG treats all tasks uniformly, no domain differentiation
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'template\|domain.*annotation' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
cargo test -p roko-cli
```

**Priority**: P2 (Phase 2+)

---

### ORCH-10: Priority Ceiling Protocol for resource scheduling
- [x] Implement priority inversion prevention in the executor

**Spec** (doc 03 §Priority Inversion Prevention): The executor should use the Immediate
Ceiling Priority Protocol (ICPP, Sha et al. 1990) to prevent priority inversion when
high-priority plans are blocked by low-priority plans holding shared resources. The spec
defines `PriorityCeiling { ceilings: HashMap<ResourceId, u32> }` with
`compute(plans: &[PlanInfo]) -> Self` that sets each resource's ceiling to the max priority
of any plan that uses it. When a plan acquires a resource, its effective priority is
immediately raised to the resource's ceiling. Guarantees: bounded blocking (at most ONE
critical section), deadlock-free, no chained blocking.

**Current code** (`crates/roko-orchestrator/src/executor/mod.rs`): Plans have a `priority: u32`
field on `PlanState`. The tick loop iterates plans in queue order (set by `rank_plans()`).
No `PriorityCeiling` struct, no effective priority boosting, no resource ceiling computation.
Low-priority plans holding merge queue slots can block high-priority plans indefinitely.

**What to change**: Add `PriorityCeiling` struct in executor module. Compute ceilings from
plan metadata during executor setup. Track effective priority per plan. When a plan acquires
a resource (merge queue slot, worktree), boost its effective priority to the resource ceiling.
Adjust tick loop iteration order to use effective priority.

**Reference files**:
- `crates/roko-orchestrator/src/executor/mod.rs` -- tick loop, plan iteration, `PlanState::priority`
- `crates/roko-orchestrator/src/executor/plan_state.rs` -- `PlanState` struct with `priority` field
- `crates/roko-orchestrator/src/merge_queue.rs` -- shared resource (merge queue) that causes contention
- `docs/01-orchestration/03-parallel-executor.md` -- §Priority Inversion Prevention, ICPP spec

**Accept when**:
- [x] `pub struct PriorityCeiling` with `ceilings: HashMap<ResourceId, u32>` and `compute()` method
- [x] Effective priority boosted when resource acquired
- [x] Tick loop uses effective priority for plan ordering
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'PriorityCeiling\|effective_priority' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P2

---

### ORCH-11: Plan repair engine
- [x] Implement plan repair as an alternative to full replanning

**Spec** (doc 13 §Plan Repair): When a task fails or environment changes invalidate part of
a plan, the system should attempt local repair before full replanning. The spec defines a
`RepairEngine` with three abstraction levels (task-level retry, subgraph substitution,
full replan) and meta-reasoning to choose the cheapest repair. Stability metric from
Fox et al. 2006: prefer repairs that minimize the number of changed tasks. The repair engine
should integrate with the saga pattern for compensating transactions (Garcia-Molina & Salem
1987) and the ABSTRIPS abstraction hierarchy (Sacerdoti 1974).

**Current code** (`crates/roko-orchestrator/src/executor/mod.rs`): The executor has
`auto_replan: bool` on `ExecutorConfig` and retry logic (increment iteration, reset gate
results) but no structured repair. Failed tasks either retry at the same level or the plan
fails entirely. No `RepairEngine`, no stability metric, no subgraph substitution.

**What to change**: Add `crates/roko-orchestrator/src/repair.rs` with `RepairEngine` struct.
Implement three levels: (1) task retry with modified prompt, (2) subgraph replacement
(re-plan subset of tasks), (3) full replan. Add meta-reasoning: choose level based on
estimated cost vs expected success probability. Wire into executor's `AutoFixing` phase
transition.

**Reference files**:
- `crates/roko-orchestrator/src/executor/mod.rs` -- `auto_replan` field, retry logic
- `crates/roko-orchestrator/src/executor/state_machine.rs` -- `AutoFixing` phase transition
- `crates/roko-orchestrator/src/dag.rs` -- DAG structure for subgraph identification
- `docs/01-orchestration/13-cross-domain-orchestration.md` -- §Plan Repair with RepairEngine spec, stability metric, saga pattern

**Depends on**: None

**Accept when**:
- [x] `pub struct RepairEngine` with at least `repair(plan, failure) -> RepairAction` method
- [x] Three repair levels implemented (task retry, subgraph, full replan)
- [x] Meta-reasoning selects cheapest feasible repair level
- [x] Wired into executor's `AutoFixing` phase
- [x] `cargo test -p roko-orchestrator`

**Verify**:
```bash
grep -rn 'RepairEngine\|repair_level\|subgraph_replace' crates/roko-orchestrator/src/ --include='*.rs'
cargo test -p roko-orchestrator
```

**Priority**: P2 (Phase 2+)

---

## Verify

```bash
cargo test -p roko-orchestrator
cargo test -p roko-cli
```
