# Task 069: Graph Engine — Sub-Graph Execution and Flow Snapshots/Resume

```toml
id = 69
title = "Add sub-graph recursive execution and flow snapshot/resume to Graph Engine"
track = "graph-engine"
wave = "wave-3"
priority = "medium"
blocked_by = [68]
touches = [
    "crates/roko-graph/Cargo.toml",
    "crates/roko-graph/src/lib.rs",
    "crates/roko-graph/src/engine.rs",
    "crates/roko-graph/src/types.rs",
    "crates/roko-graph/src/snapshot.rs",
    "crates/roko-graph/src/loader.rs",
    "crates/roko-graph/tests/snapshot_subgraph.rs",
    "crates/roko-cli/src/commands/graph.rs",
    "examples/graphs/nested-subgraph.toml",
]
exclusive_files = [
    "crates/roko-graph/src/snapshot.rs",
    "crates/roko-graph/tests/snapshot_subgraph.rs",
    "examples/graphs/nested-subgraph.toml",
]
estimated_minutes = 360
```

## Context

This task adds two Engine features from Phase 2C: sub-graph support (P2-11) and flow
snapshots with resume (P2-12). These are independent features but naturally grouped because
both modify the Engine's execution loop and state management.

Sub-graphs enable fractal composition — a Graph node can contain another Graph. The Engine
recursively executes the inner graph, mapping its inputs/outputs to the outer graph's signal
flow. This is the foundation for composable workflow libraries.

Flow snapshots enable crash recovery. After each node completes, the Engine writes a snapshot
of completed node outputs. On resume, completed nodes are skipped and execution continues from
where it left off. This follows the same pattern as Runner v2's `--resume` flag.

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/07-GRAPH-ENGINE.md` — Sub-graph execution (CellRef::SubGraph recursive
   call), FlowSnapshot concept
2. `crates/roko-graph/src/engine.rs` — Current Engine with sequential + parallel execution
   from tasks 066-068
3. `crates/roko-graph/src/types.rs` — CellRef::SubGraph(Box<Graph>) already defined in types
4. `crates/roko-cli/src/runner/persist.rs` — Runner v2's snapshot persistence pattern.
   Follow the same approach (JSONL or JSON file in `.roko/state/`)
5. `crates/roko-cli/src/runner/resume.rs` — Runner v2's resume logic. Same pattern:
   load snapshot, determine which nodes are done, skip them
6. `crates/roko-core/src/workspace.rs` — Workspace struct for `.roko/` path construction

## What to Change

### 1. Implement sub-graph execution in `engine.rs`

In the `execute_graph` method, handle `CellRef::SubGraph`:

```rust
CellRef::SubGraph(sub_graph) => {
    // Recursive call: execute the sub-graph with this node's inputs
    let output = self.execute_graph(
        (**sub_graph).clone(),
        inputs.clone(),
        ctx.clone(),
    ).await?;
    node_outputs.insert(node_id.clone(), output);
    continue;
}
```

This replaces the error that task 067 returns for sub-graphs.

Add validation in the TOML loader or `Graph::validate()`:
- Detect infinite sub-graph recursion (a graph containing itself). Use a visited set of
  graph IDs during validation.
- Sub-graphs must have matching entry/exit schemas with their parent node's edges (if schemas
  are defined). If schemas are `None`, skip validation.

### 2. Support sub-graph definition in TOML

Extend the TOML loader to handle inline sub-graphs:

```toml
[[graph.nodes]]
id = "validation"
kind = "SubGraph"

[graph.nodes.sub_graph]
name = "validation-pipeline"
version = "0.1.0"

[[graph.nodes.sub_graph.nodes]]
id = "compile"
cell = "compile-gate"
kind = "Cell"

[[graph.nodes.sub_graph.nodes]]
id = "test"
cell = "test-gate"
kind = "Cell"

[[graph.nodes.sub_graph.edges]]
from = "compile"
to = "test"
```

Also support sub-graph-by-reference (load from a separate file):

```toml
[[graph.nodes]]
id = "validation"
kind = "SubGraph"
graph_file = "validation-pipeline.toml"
```

For `graph_file`, resolve the path relative to the parent graph's file location.

### 3. Implement FlowSnapshot in `snapshot.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSnapshot {
    pub flow_id: FlowId,
    pub graph_id: GraphId,
    pub graph_name: String,
    pub started_at: String,   // ISO 8601
    pub updated_at: String,
    pub completed_nodes: HashMap<NodeId, NodeSnapshot>,
    pub status: FlowStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSnapshot {
    pub node_id: NodeId,
    pub completed_at: String,
    pub output_signals: Vec<Signal>,
    pub duration_ms: u64,
}
```

Methods:
- `FlowSnapshot::new(flow_id, graph) -> Self`
- `record_node(&mut self, node_id, outputs, duration)` — mark a node as complete
- `is_complete(&self, node_id: &str) -> bool`
- `save(&self, path: &Path) -> Result<()>` — write to JSON file
- `load(path: &Path) -> Result<Self>` — read from JSON file

### 4. Wire snapshots into Engine execution

Modify `execute_graph` to:

1. Accept an optional `FlowSnapshot` parameter (for resume)
2. Before executing each node, check `snapshot.is_complete(node_id)`:
   - If complete: load outputs from snapshot, skip execution
   - If not complete: execute normally
3. After each node completes, call `snapshot.record_node()` and `snapshot.save()`
4. Save snapshots to `.roko/state/flows/{flow_id}.json` using `Workspace` for path construction

### 5. Add `--resume` flag to `roko graph run`

Extend the CLI command:

```bash
roko graph run <file.toml>                          # Fresh run
roko graph run <file.toml> --resume <snapshot.json>  # Resume from snapshot
```

When `--resume` is provided:
1. Load the FlowSnapshot from the given path
2. Validate that the snapshot's `graph_id` matches the loaded graph
3. Pass the snapshot to `Engine::start()` (or a new `Engine::resume()` method)
4. The Engine skips completed nodes and continues from where it left off

### 6. Write example graph and tests

**`examples/graphs/nested-subgraph.toml`** — A graph with an inline sub-graph node:
- Entry node -> sub-graph (compile + test) -> exit node

Tests:
- Unit test: `FlowSnapshot` save/load round-trips correctly
- Unit test: `is_complete()` returns correct values
- Integration test: run a graph, verify snapshot file is created
- Integration test: run a graph, kill it (or mock failure), resume from snapshot, verify
  completed nodes are skipped
- Integration test: run a graph with a sub-graph node, verify recursive execution
- Unit test: `validate()` detects self-referencing sub-graph

## What NOT to Do

- Do NOT implement Hot Graphs (tick-driven, persistent). That is Phase 4.
- Do NOT implement budget/deadline enforcement in this task. That is task 070.
- Do NOT build a graph visual editor or graph-to-dot converter.
- Do NOT use `RokoLayout` for path construction — use `Workspace`.
- Do NOT make snapshots async or use a database. Simple JSON files, same as Runner v2.
- Do NOT implement snapshot compaction or garbage collection. One file per flow is fine.
- Do NOT add sub-graph support for `graph_file` references across network/HTTP — local paths only.

## Wire Target

```bash
# Sub-graph execution
cargo run -p roko-cli -- graph run examples/graphs/nested-subgraph.toml
# Expected: Sub-graph nodes execute recursively, output signals printed

# Snapshot creation (automatic)
cargo run -p roko-cli -- graph run examples/graphs/linear-gates.toml
ls .roko/state/flows/
# Expected: A snapshot JSON file exists for the completed flow

# Resume from snapshot
cargo run -p roko-cli -- graph run examples/graphs/linear-gates.toml --resume .roko/state/flows/<flow-id>.json
# Expected: Completed nodes are skipped, output matches fresh run
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/nested-subgraph.toml` — sub-graph executes
- [ ] Snapshot JSON file appears in `.roko/state/flows/` after a graph run
- [ ] Resume from snapshot skips completed nodes (visible in output or logs)
- [ ] Existing example graphs from tasks 067-068 still work unchanged
- [ ] `grep -rn 'FlowSnapshot' crates/roko-graph/ --include='*.rs' | grep -v target/` — used in engine
- [ ] `grep -rn 'SubGraph' crates/roko-graph/src/engine.rs` — sub-graph handled (not erroring)
- [ ] Unit tests for snapshot save/load pass
- [ ] Integration test for sub-graph execution passes
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file

## Implementation Detail

### Current source facts to account for

- `crates/roko-core/src/workspace.rs::Workspace` is the canonical public path boundary. Use
  `Workspace::open_or_create(workdir)` in the CLI and `workspace.state_dir().join("flows")` for
  graph snapshots. Do not use `roko_fs::RokoLayout`.
- Runner v2's `atomic_write()` lives in `crates/roko-cli/src/runner/persist.rs`, which is not a
  dependency target for `roko-graph`. Reuse the write-to-temp-then-rename pattern in
  `snapshot.rs`; do not make `roko-graph` depend on `roko-cli`.
- Direct recursive `async fn execute_graph(...)` will not compile once it calls itself for
  subgraphs. Refactor the internal method to return a boxed future using only std types, for
  example `Pin<Box<dyn Future<Output = Result<Vec<Signal>>> + Send + '_>>`; do not add an
  `async-recursion` dependency.
- `crates/roko-graph/Cargo.toml` needs `chrono` from the workspace for RFC3339 timestamps unless a
  prior task already added an equivalent timestamp helper.

### Mechanical implementation steps

1. `snapshot.rs`:
   - Define `FlowSnapshot` and `NodeSnapshot` exactly as in this task, deriving
     `Debug, Clone, Serialize, Deserialize`.
   - `FlowSnapshot::new(flow_id, graph)` should set `graph_id`, `graph_name`, `started_at`, and
     `updated_at` using `chrono::Utc::now().to_rfc3339()`, initialize an empty
     `completed_nodes`, and set status to `FlowStatus::Running`.
   - `record_node()` updates `completed_nodes[node_id]`, stores a clone of output signals, updates
     `updated_at`, and records duration in milliseconds.
   - `save()` creates parent directories and atomically writes pretty JSON via a `.tmp` sibling and
     `std::fs::rename`.
   - `load()` returns a contextual parse error that includes the snapshot path.

2. Engine snapshot wiring:
   - Keep `Engine::new(...)` source-compatible with task 067.
   - Add `Engine::with_snapshot_dir(mut self, dir: impl Into<PathBuf>) -> Self` or an equivalent
     builder so the CLI can enable persistence without making every test write files.
   - Add `Engine::resume(&self, graph: &Graph, input: Vec<Signal>, snapshot: FlowSnapshot)
     -> Result<FlowId>` or a `start_with_snapshot(...)` variant. Fresh `start()` should still work.
   - Fresh CLI runs should create a `FlowSnapshot` automatically when `snapshot_dir` is configured
     and save it to `.roko/state/flows/{flow_id}.json`.
   - Resume runs should reuse `snapshot.flow_id` as the flow id and save back to the same snapshot
     path unless the CLI passes a different path explicitly.
   - Before executing a node, if `snapshot.is_complete(node_id)` is true, insert the stored
     `output_signals` into `node_outputs`, print/log that the node was skipped, and do not call the
     cell.
   - After a node completes normally, record and save the snapshot before moving to downstream
     nodes. For parallel levels from task 068, record joined node outputs after the level completes
     in deterministic node-id order.
   - Snapshot keys are top-level node IDs in this task. For a subgraph node, record the subgraph
     node once after the recursive call returns; do not persist inner subgraph nodes into the parent
     snapshot because node IDs can collide across graph boundaries.

3. Subgraph execution:
   - Replace the task-067 unsupported error for `CellRef::SubGraph(sub_graph)`.
   - Execute the subgraph recursively with the current node's active inputs and the same registry,
     store, bus, cancellation token, and budget/deadline context if task 070 has already landed.
   - Insert the recursive output under the outer subgraph node ID.
   - Preserve task-068 edge conditions and mappings before and after the subgraph node.

4. Loader support for subgraphs:
   - Extend the raw TOML node struct with `sub_graph: Option<RawGraph>` and
     `graph_file: Option<PathBuf>`.
   - Inline subgraphs use `[graph.nodes.sub_graph]` and nested `[[graph.nodes.sub_graph.nodes]]`
     exactly as shown in this task.
   - `graph_file` paths resolve relative to the parent graph file's directory. Canonicalize paths
     when possible and keep a stack/set of files being loaded so `a.toml -> b.toml -> a.toml`
     returns a clear recursion error.
   - A node with `kind = "SubGraph"` must specify exactly one of `sub_graph` or `graph_file`.
     A node with `cell = "..."` must not also specify subgraph fields.

5. Validation:
   - Add an internal `Graph::validate_with_ancestors(&mut Vec<GraphId>)` helper and call it from
     `validate()`.
   - Reject a subgraph whose `graph.id` already appears in the ancestor stack.
   - Keep schema compatibility conservative: if both parent edge schema and subgraph schema exist,
     use `TypeSchema::is_compatible_with`; if either side is `None`, skip that check.
   - Do not attempt network, HTTP, or package-registry resolution for graph references.

6. CLI:
   - Extend `GraphCmd::Run` with `#[arg(long)] resume: Option<PathBuf>` and optionally
     `#[arg(long)] workdir: Option<PathBuf>` if task 067 did not already add it.
   - Call chain for fresh run:
     `cmd_graph -> Workspace::open_or_create -> load_graph -> Engine::new(...).with_snapshot_dir(...) -> start -> await_flow`.
   - Call chain for resume:
     `cmd_graph -> load_graph -> FlowSnapshot::load -> graph_id check -> Engine::resume -> await_flow`.
   - If `snapshot.graph_id != graph.id`, return an error that names both IDs and the snapshot path.
   - Print the snapshot path after fresh runs and print skipped-node lines during resume so the
     wire target can observe that resume happened.

### Tests to add

- `snapshot.rs` unit tests for `new`, `record_node`, `is_complete`, and save/load round-trip using
  `tempfile`.
- Loader unit tests for inline subgraph and `graph_file` relative resolution.
- Validation unit test for duplicate ancestor graph IDs.
- `crates/roko-graph/tests/snapshot_subgraph.rs`:
  - Run a graph containing an inline subgraph and assert recursive output is returned.
  - Create a snapshot with the first node complete, resume, and assert that node's test cell was
    not executed again.
  - Run with snapshot persistence enabled and assert `.roko/state/flows/{flow_id}.json` exists.

### Anti-patterns specific to this task

- Do not make snapshot persistence best-effort silent. Save/load errors must surface with path
  context.
- Do not store snapshots under `.roko/memory` or `.roko/runs`; the required location is
  `.roko/state/flows/`.
- Do not add a database or async file writer for graph snapshots.
- Do not recursively record inner subgraph node IDs into the parent snapshot without path
  qualification.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
