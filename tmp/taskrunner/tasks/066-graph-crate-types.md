# Task 066: Create roko-graph Crate — Types, TOML Loader, Topo Sort, CellRegistry

```toml
id = 66
title = "Create roko-graph crate with Graph/Node/Edge types, TOML loader, CellRegistry, and topological sort"
track = "graph-engine"
wave = "wave-3"
priority = "high"
blocked_by = ["P1-1", "P1-2", "P1-3"]
touches = [
    "crates/roko-graph/Cargo.toml",
    "crates/roko-graph/src/lib.rs",
    "crates/roko-graph/src/types.rs",
    "crates/roko-graph/src/loader.rs",
    "crates/roko-graph/src/registry.rs",
    "crates/roko-graph/src/topo.rs",
    "Cargo.toml",
]
exclusive_files = [
    "crates/roko-graph/Cargo.toml",
    "crates/roko-graph/src/lib.rs",
    "crates/roko-graph/src/types.rs",
    "crates/roko-graph/src/loader.rs",
    "crates/roko-graph/src/registry.rs",
    "crates/roko-graph/src/topo.rs",
]
estimated_minutes = 300
```

## Context

This task creates the `roko-graph` crate from scratch. It covers checklist items P2-1 through
P2-4: the foundational types (Graph, Node, Edge, GraphPolicy), the TOML-to-Graph loader,
the CellRegistry for resolving cell names to implementations, and topological sort for
determining execution order.

This crate is **pure data + algorithms** — no async, no runtime, no IO. The Engine (task 067)
consumes these types. This task is deliberately separated from the Engine so that the types
can be reviewed and tested independently.

**Critical constraint**: This task must NOT be considered done until `roko graph run` works
(task 067). These two tasks are designed to be worked in sequence — 066 builds the library,
067 wires it to CLI + Engine. If you are assigned 066 without 067, your verification is
limited to unit tests.

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/07-GRAPH-ENGINE.md` — Full design spec with struct definitions, TOML
   format, and algorithm pseudocode. This is the source of truth for types.
2. `tmp/v2-refactoring/04-CELL-EXECUTE.md` — Cell trait with execute(), CellContext, TypeSchema.
   The Graph types reference Cell and TypeSchema.
3. `crates/roko-core/src/cell.rs` — Current Cell trait. CellRegistry stores `Arc<dyn Cell>`.
4. `crates/roko-core/src/lib.rs` — What roko-core exports. roko-graph will depend on roko-core.
5. `crates/roko-orchestrator/src/dag.rs` — Existing DAG implementation. Check for reusable topo
   sort logic. Do NOT duplicate it; if it's suitable, depend on it or extract the algorithm.

## What to Change

### 1. Create the crate scaffold

Create `crates/roko-graph/` with `Cargo.toml`. Add it to the workspace `Cargo.toml` members list.

Dependencies: `roko-core` (for Cell, Signal/Engram, TypeSchema), `serde`, `serde_json`, `toml`,
`anyhow`, `thiserror`. No async runtime — no `tokio` here (that goes in the Engine, task 067).

### 2. Define core types in `types.rs`

Implement the structs from the design doc (`07-GRAPH-ENGINE.md`):

- `Graph` — id, name, version, nodes, edges, entry/exits, input/output schema, policy
- `Node` — id, cell_ref, kind, execution_class
- `CellRef` — Named(String) or SubGraph(Box<Graph>)
- `NodeKind` — Cell, SubGraph, Branch, FanOut, FanIn, Noop
- `ExecutionClass` — Workflow (deterministic) or Activity (non-deterministic)
- `Edge` — from, to, condition, mapping
- `Mapping` — Identity, FilterKind, Project
- `GraphPolicy` — max_budget, deadline, max_parallelism, failure_strategy
- `FailureStrategy` — FailFast, ContinueOnFailure, Retry(usize)
- `GraphId`, `NodeId` — type aliases for String

All types must derive `Debug, Clone, Serialize, Deserialize`. Add sensible `Default` impls
where appropriate (e.g., `GraphPolicy` defaults to FailFast, no budget/deadline).

Add helper methods on `Graph`:
- `node(&self, id: &str) -> Option<&Node>` — look up node by ID
- `incoming_edges(&self, node_id: &str) -> Vec<&Edge>` — edges targeting a node
- `outgoing_edges(&self, node_id: &str) -> Vec<&Edge>` — edges leaving a node
- `validate(&self) -> Result<()>` — check for: duplicate node IDs, edges referencing
  nonexistent nodes, entry/exit nodes exist in the node list

### 3. Implement TOML loader in `loader.rs`

`pub fn load_graph(path: &Path) -> Result<Graph>` — reads a TOML file and deserializes into
a `Graph`. After deserialization, call `graph.validate()` to catch structural errors early.

The TOML format matches the design doc:

```toml
[graph]
name = "example"
version = "0.1.0"

[[graph.nodes]]
id = "step-1"
cell = "compile-gate"
kind = "Cell"

[[graph.edges]]
from = "step-1"
to = "step-2"

[graph.policy]
failure_strategy = "FailFast"
```

Handle the serde mapping: `cell` field in TOML maps to `CellRef::Named(cell)`.
The `kind` field defaults to `"Cell"` if omitted.

### 4. Implement CellRegistry in `registry.rs`

```rust
pub struct CellRegistry {
    cells: HashMap<String, Arc<dyn Cell>>,
}
```

Methods:
- `new() -> Self`
- `register(&mut self, name: &str, cell: Arc<dyn Cell>)`
- `get(&self, name: &str) -> Option<Arc<dyn Cell>>`
- `names(&self) -> Vec<&str>` — list registered cell names (for diagnostics)
- `contains(&self, name: &str) -> bool`

### 5. Implement topological sort in `topo.rs`

`pub fn topological_sort(nodes: &[Node], edges: &[Edge]) -> Result<Vec<NodeId>>`

**Before implementing**: check `crates/roko-orchestrator/src/dag.rs` for the existing topo sort.
The current `UnifiedTaskDag::topological_sort()` is tied to private task-DAG fields and
`GlobalTaskId`, so it is not directly reusable by `roko-graph`. Implement a local Kahn sort for
`NodeId`, mirroring the orchestrator's deterministic ordering and cycle diagnostics:
1. Compute in-degree for each node
2. Start with zero-in-degree nodes
3. Process queue, decrementing in-degrees
4. If result length != node count, there is a cycle — return an error with the cycle nodes

### 6. Wire `lib.rs` exports

Export all public types and functions from `lib.rs`. The public API should be:

```rust
pub mod types;
pub mod loader;
pub mod registry;
pub mod topo;

pub use types::*;
pub use loader::load_graph;
pub use registry::CellRegistry;
pub use topo::topological_sort;
```

## What NOT to Do

- Do NOT add `tokio` as a dependency. This crate is synchronous. The Engine (task 067) adds async.
- Do NOT implement the Engine in this task. Types and algorithms only.
- Do NOT implement expression evaluation for `Edge::condition`. Store it as `Option<String>`,
  evaluation comes in task 068.
- Do NOT implement Hot Graphs or tick-driven execution. That is Phase 4.
- Do NOT add a `roko-orchestrator` dependency just to reuse task-DAG internals.
- Do NOT add `Graph` as a `Cell` impl yet. Sub-graph execution is task 069.

## Wire Target

```bash
# This task's types are wired when task 067 completes.
# Standalone verification for this task:
cargo build -p roko-graph
cargo test -p roko-graph
```

The full wire target is `roko graph run <file.toml>` — that requires task 067 (Engine + CLI
command). This task provides the library that 067 consumes.

## Verification

- [ ] `cargo build --workspace` — roko-graph compiles, workspace includes it
- [ ] `cargo test -p roko-graph` — all unit tests pass
- [ ] `cargo clippy -p roko-graph --no-deps -- -D warnings` — clean lint
- [ ] Unit test: parse a valid TOML graph file into a Graph struct
- [ ] Unit test: `validate()` catches duplicate node IDs
- [ ] Unit test: `validate()` catches edges referencing nonexistent nodes
- [ ] Unit test: `topological_sort()` correctly orders a 5-node DAG
- [ ] Unit test: `topological_sort()` detects a cycle and returns an error
- [ ] Unit test: `CellRegistry` register + get round-trips
- [ ] `grep -rn 'roko-graph' Cargo.toml` — workspace members includes roko-graph
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file

## Implementation Detail

### Current source facts to account for

- In this checkout, `crates/roko-core/src/cell.rs` is still metadata-only. This task is blocked
  until task 035 has landed `CellContext`, `TypeSchema`, and object-safe async `Cell::execute()`.
  Import `Cell`, `Signal`, `TypeSchema`, and `Kind` from `roko_core`; do not define duplicate
  graph-local versions.
- `crates/roko-orchestrator/src/dag.rs::UnifiedTaskDag::topological_sort()` is a Kahn sort over
  private `GlobalTaskId` task state. It is not suitable as a direct dependency for `roko-graph`.
  Implement a local graph-node Kahn sort, mirroring its deterministic `BTreeSet` tie-breaking and
  cycle diagnostics. Do not add a `roko-orchestrator` dependency to `roko-graph`.
- The workspace manifest uses explicit `members` and workspace dependencies. Add
  `"crates/roko-graph"` to the root `Cargo.toml` members list near the other core/runtime crates.

### Mechanical implementation notes

1. `crates/roko-graph/Cargo.toml`:
   - Use `edition.workspace`, `rust-version.workspace`, `license.workspace`, `authors.workspace`,
     and `[lints] workspace = true`.
   - Add dependencies with workspace versions where available:
     `roko-core`, `serde` with derive, `serde_json`, `toml`, `anyhow`, `thiserror`.
   - Do not add `tokio`, `tokio-util`, `roko-gate`, or `roko-std` in this task.

2. `types.rs`:
   - Use `pub type GraphId = String;` and `pub type NodeId = String;`.
   - Keep `Graph.version` compatible with the design doc and examples. Either store it as
     `(u32, u32, u32)` with serde helpers that accept `"0.1.0"`, or store it as `String`; if you
     choose the tuple, add tests for string TOML parsing because serde will not deserialize
     `"0.1.0"` into a tuple automatically.
   - Add serde rename attributes so TOML values like `kind = "Cell"` and
     `failure_strategy = "FailFast"` deserialize without custom callers:
     `#[serde(rename_all = "PascalCase")]` on `NodeKind`, `ExecutionClass`, and
     `FailureStrategy`.
   - Set defaults on optional graph fields: empty `nodes`/`edges`, default `policy`, and default
     `entry`/`exits` to empty before loader inference.
   - `GraphPolicy::default()` must be unlimited budget/deadline/parallelism with
     `FailureStrategy::FailFast`.
   - `CellRef::Named(String)` is the normal runtime path. `CellRef::SubGraph` is only a type
     placeholder until task 069; do not execute or validate recursion here.

3. `loader.rs`:
   - Deserialize the file as a wrapper struct, not directly as `Graph`:
     `struct GraphDocument { graph: RawGraph }`.
   - Use raw TOML structs with fields that match examples: `RawNode { id, cell, kind,
     execution_class }`, where `cell = "compile-gate"` converts to
     `CellRef::Named("compile-gate".to_string())`.
   - If `kind` is omitted, default to `NodeKind::Cell`. If `execution_class` is omitted, default
     to `ExecutionClass::Workflow`.
   - If `id` is omitted at graph level, derive `Graph.id` from `graph.name`.
   - If `entry` is omitted or empty, infer entries as nodes with no incoming edges. If `exits` is
     omitted or empty, infer exits as nodes with no outgoing edges. This is required because all
     current example TOML snippets omit entry/exit lists.
   - Add `with_context` messages that include the path for read, parse, and validation errors.

4. `Graph::validate()`:
   - Reject duplicate node IDs.
   - Reject edges whose `from` or `to` does not exist.
   - Reject `entry` and `exits` values that do not exist.
   - Reject `policy.max_parallelism == Some(0)` so task 068 can treat `None` as unlimited and
     `Some(n)` as a real positive limit.
   - Keep validation pure and synchronous.

5. `registry.rs`:
   - Store `HashMap<String, Arc<dyn Cell>>`.
   - `get()` returns a cloned `Arc`.
   - `names()` should return a sorted `Vec<&str>` for stable diagnostics and tests.

6. `topo.rs`:
   - Build node ID sets from `nodes`; return an error for duplicate IDs or edges that reference
     missing nodes even if callers forgot `validate()`.
   - Use lexical ordering for ready nodes so test output is deterministic.
   - On cycles, include the stuck node IDs in the error message.

### Test placement

- Put narrow unit tests next to the module being tested (`types.rs`, `loader.rs`, `registry.rs`,
  `topo.rs`). Avoid creating broad workspace tests in this foundational task.
- Include a loader test that writes a temporary TOML file using the documented `[graph]` wrapper
  and omits `entry`/`exits`; assert they are inferred.
- Include a test proving `names()` sorting is deterministic.

### Anti-patterns specific to this task

- Do not make `roko-graph` depend on `roko-cli` or `roko-orchestrator`.
- Do not add a second `TypeSchema`, `Signal`, or `Kind` model to get around missing prerequisites.
- Do not use stringly typed JSON manipulation for TOML loading when serde raw structs can express
  the file shape.
- Do not skip entry/exit inference; otherwise the examples in task 067 will load but produce no
  output.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
