# Task 067: Graph Engine — Sequential Execution, CLI Command, Default Cells, Example Graphs

```toml
id = 67
title = "Implement Graph Engine with sequential execution, `roko graph run` CLI command, default cell registry, and example graphs"
track = "graph-engine"
wave = "wave-3"
priority = "high"
blocked_by = [66, "P1-4", "P1-5"]
touches = [
    "crates/roko-graph/Cargo.toml",
    "crates/roko-graph/src/lib.rs",
    "crates/roko-graph/src/engine.rs",
    "crates/roko-graph/tests/engine.rs",
    "crates/roko-cli/Cargo.toml",
    "crates/roko-cli/src/commands/graph.rs",
    "crates/roko-cli/src/main.rs",
    "examples/graphs/linear-gates.toml",
    "examples/graphs/score-compose.toml",
    "examples/graphs/single-gate.toml",
]
exclusive_files = [
    "crates/roko-graph/src/engine.rs",
    "crates/roko-graph/tests/engine.rs",
    "crates/roko-cli/src/commands/graph.rs",
    "examples/graphs/linear-gates.toml",
    "examples/graphs/score-compose.toml",
    "examples/graphs/single-gate.toml",
]
estimated_minutes = 360
```

## Context

This task builds the Graph Engine and wires it to a CLI command. It covers checklist items
P2-5 through P2-8. The Engine is the simplest possible interpreter: sequential execution of
nodes in topological order, no parallelism, no branching, no snapshots. The CLI command
`roko graph run <file.toml>` is the wire target — the Engine is not done until you can run
a graph from the command line and see output.

This task depends on task 066 (roko-graph types) and Phase 1 items P1-4/P1-5 (gate Cell
implementations with execute()). The Engine calls `Cell::execute()` on each node, passing
signals along edges.

The design doc says ~500 LOC for the simplest Engine. Stay close to that. Do not over-engineer.

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/07-GRAPH-ENGINE.md` — Engine struct, execute_graph algorithm, FlowHandle,
   FlowStatus, CLI wiring plan, and the `build_default_registry()` example
2. `crates/roko-graph/src/` — Types from task 066 (Graph, Node, Edge, CellRegistry, topo sort)
3. `crates/roko-core/src/cell.rs` — Cell trait with execute() (from P1-2)
4. `crates/roko-gate/src/lib.rs` — Gate implementations. After P1-4/P1-5, these have execute().
   These become default registered cells.
5. `crates/roko-cli/src/main.rs` — CLI command dispatch. You will add `graph run` here.
6. `crates/roko-cli/src/commands/` — Existing command modules. Follow the same pattern.

## What to Change

### 1. Add Engine to roko-graph (`engine.rs`)

Add `tokio` as a dependency to `roko-graph/Cargo.toml` (needed for async execution).

Implement:

```rust
pub struct Engine {
    pub registry: Arc<CellRegistry>,
    pub bus: Arc<dyn BusErased>, // or the exact bus-erased type from task 035's CellContext
    pub store: Arc<dyn Store>,
    cancel: CancellationToken,
}
```

Methods:

- `new(registry, bus, store) -> Self`
- `async start(&self, graph: &Graph, input: Vec<Signal>) -> Result<FlowId>` — spawns execution
- `async await_flow(&self, flow_id: &FlowId) -> Result<Vec<Signal>>` — waits for completion
- `async cancel(&self, flow_id: &FlowId) -> Result<()>`
- `fn status(&self, flow_id: &FlowId) -> Option<FlowStatus>`

The `start` method spawns `execute_graph` on a tokio task. For this version, `execute_graph`
is the sequential algorithm from the design doc:

1. Topological sort the graph nodes
2. Entry nodes receive the input signals
3. For each node in topo order:
   a. Collect signals from all incoming edges (from predecessor outputs)
   b. Resolve the Cell from the registry via `CellRef::Named`
   c. Call `cell.execute(inputs, &ctx)`
   d. Store the output signals keyed by node ID
4. Collect and return signals from exit nodes

**Keep it simple**: No fan-out/fan-in logic (task 068). No conditional edge evaluation (task 068).
No sub-graph recursion (task 069). No snapshots (task 069). If a node's `CellRef` is `SubGraph`,
return an error saying sub-graphs are not yet supported.

Add `FlowId`, `FlowHandle`, `FlowStatus` types (from design doc).

### 2. Implement `build_default_registry()`

Create a function in `engine.rs` that returns a `CellRegistry`
populated with existing Cell implementations:

- `"compile-gate"` — CompileGate from roko-gate
- `"test-gate"` — TestGate from roko-gate
- `"clippy-gate"` — ClippyGate from roko-gate
- `"diff-gate"` — DiffGate from roko-gate (if it has execute())
- `"noop"` — A trivial cell that passes signals through unchanged

At minimum, register the gates that have `execute()` implementations from P1-4/P1-5.
If a gate does not yet have `execute()`, skip it and document which ones are missing in your
Status Log.

### 3. Add `roko graph run` CLI command

Create `crates/roko-cli/src/commands/graph.rs` with:

```rust
pub async fn cmd_graph_run(path: &Path) -> Result<()> {
    let graph = roko_graph::load_graph(path)?;
    let registry = Arc::new(build_default_registry()?);
    // Use in-memory bus and store for now
    let bus = /* in-memory PulseBus or equivalent */;
    let store = /* in-memory store or FileSubstrate */;
    let engine = Engine::new(registry, bus, store);
    let flow_id = engine.start(&graph, vec![]).await?;
    let output = engine.await_flow(&flow_id).await?;
    println!("Graph '{}' completed: {} output signals", graph.name, output.len());
    for signal in &output {
        println!("  - {}: {:?}", signal.kind.as_str(), signal.body);
    }
    Ok(())
}
```

Wire this into `main.rs` under the `graph` subcommand with a `run` sub-subcommand.
Follow the existing clap pattern in the CLI.

### 4. Write 3 example graph TOML files

Create `examples/graphs/` directory with:

**`single-gate.toml`** — One node, one gate. Simplest possible graph.
```toml
[graph]
name = "single-gate"
version = "0.1.0"

[[graph.nodes]]
id = "compile"
cell = "compile-gate"
kind = "Cell"

[graph.policy]
failure_strategy = "FailFast"
```

**`linear-gates.toml`** — Three gates in sequence: compile -> clippy -> test.

**`score-compose.toml`** — A 2-3 node pipeline using whichever cells are available from the
default registry. Pick cells that demonstrate signal flow between nodes.

### 5. Add integration test

Write at least one integration test in `crates/roko-graph/tests/` that:
1. Loads a graph from TOML
2. Creates an Engine with a registry containing a test cell (e.g., NoopCell)
3. Runs the graph
4. Asserts the output signals are correct

## What NOT to Do

- Do NOT implement parallel execution (fan-out/fan-in). Sequential only. Task 068 adds parallelism.
- Do NOT implement conditional edge evaluation. All edges are always-true for now. Task 068.
- Do NOT implement sub-graph recursion. Return an error for `CellRef::SubGraph`. Task 069.
- Do NOT implement snapshot/resume. Task 069.
- Do NOT implement budget/deadline enforcement. Task 070.
- Do NOT add AgentCell or ComposeCell. Task 071.
- Do NOT build a graph validator beyond what task 066 provides.
- Do NOT use `roko-fs::RokoLayout` — use `roko-core::Workspace` if you need paths.

## Wire Target

```bash
cargo run -p roko-cli -- graph run examples/graphs/single-gate.toml
# Expected: Loads graph, runs compile gate, prints output signals

cargo run -p roko-cli -- graph run examples/graphs/linear-gates.toml
# Expected: Runs compile -> clippy -> test in sequence, prints results
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/single-gate.toml` — runs without error
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/linear-gates.toml` — runs 3 gates in sequence
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/score-compose.toml` — runs without error
- [ ] `roko graph run` with a nonexistent file produces a clear error message
- [ ] `roko graph run` with invalid TOML produces a clear parse error
- [ ] Integration test in `crates/roko-graph/tests/` passes
- [ ] `grep -rn 'graph run\|cmd_graph_run' crates/roko-cli/src/ --include='*.rs' | grep -v target/` — shows CLI wiring
- [ ] `grep -rn 'build_default_registry' crates/ --include='*.rs' | grep -v target/` — shows callers
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file

## Implementation Detail

### Current source facts to account for

- The CLI command enum and dispatch live in `crates/roko-cli/src/main.rs`, not in the library
  crate. Add a top-level `Command::Graph { cmd: GraphCmd }` variant, define `GraphCmd` near
  `PlanCmd`, and add a `Command::Graph` arm in `dispatch_subcommand()`.
- `crates/roko-cli/src/commands/mod.rs` must export `pub mod graph;`; otherwise
  `commands::graph::...` will not compile.
- `crates/roko-cli/Cargo.toml` must add `roko-graph = { path = "../roko-graph" }`; the original
  touch list missed this.
- `crates/roko-graph/src/lib.rs` must export the new engine API:
  `pub mod engine;` plus `pub use engine::{build_default_registry, Engine, FlowId, FlowStatus};`.
- `roko_core::Bus` has an associated `Receiver` type and is not object-safe. Task 035 should land
  `CellContext` using `Arc<dyn BusErased>`. Match the landed `CellContext` constructor exactly;
  do not write `Arc<dyn Bus>` in `Engine` if it does not compile.
- The existing gate constructors are `CompileGate::cargo()`, `TestGate::cargo()`,
  `ClippyGate::cargo()`, and `DiffGate::new()`.

### Mechanical implementation steps

1. Update `crates/roko-graph/Cargo.toml` for engine/runtime support:
   - Add `tokio` with `rt`, `macros`, `sync`, and `time` features or use the workspace `full`
     feature if consistent with adjacent crates.
   - Add `tokio-util` for `CancellationToken`.
   - Add `async-trait` if a local `NoopCell` overrides async `Cell::execute()`.
   - Add `uuid` for `FlowId::new()` generation.
   - Add `tracing` for engine diagnostics.
   - Add `roko-gate` and `roko-std` only for `build_default_registry()` and test/default cells.

2. Implement `engine.rs` with these public shapes:
   - `pub type FlowId = String;`
   - `pub enum FlowStatus { Running, Completed { output_count: usize }, Failed { error: String },
     Cancelled }`
   - `pub struct Engine { registry: Arc<CellRegistry>, ... }` with internal flow handles and
     statuses protected by `Mutex`/`DashMap`. Do not hold a lock while awaiting a join handle.
   - `Engine::new(registry, bus, store)` should accept the same bus/store trait object types that
     `CellContext::new()` accepts after task 035.
   - `start()` creates a fresh UUID flow id, a child `CancellationToken`, a `CellContext`, stores
     `FlowStatus::Running`, and spawns the execution future.
   - `await_flow()` removes the join handle, awaits it, updates status to `Completed` or `Failed`,
     and returns the output signals or error.
   - `cancel()` cancels the stored token and sets status to `Cancelled`.

3. Keep `execute_graph` sequential in this task:
   - Call `graph.validate()` and `topological_sort(&graph.nodes, &graph.edges)` before execution.
   - Insert the initial input for every `graph.entry` node. The loader from task 066 should infer
     entries/exits when omitted.
   - For each node in topo order, collect predecessor outputs from incoming edges. Ignore
     `condition` and treat `mapping` as identity in this task.
   - Execute entry nodes even when their input vector is empty. Skip non-entry nodes whose
     collected input is empty.
   - Resolve only `CellRef::Named(name)`. Return a clear error for `CellRef::SubGraph(_)`:
     `"sub-graphs are not supported until task 069"`.
   - Collect final output in `graph.exits` order.

4. Implement `build_default_registry()` in `engine.rs`:
   - Register `"compile-gate"`, `"test-gate"`, `"clippy-gate"`, and `"diff-gate"` only if their
     `execute()` overrides exist after task 036. If any still have the default
     "execute() not implemented" behavior, do not register them; document it in Status Log.
   - Add a local `NoopCell` in `engine.rs` and register it as `"noop"`. It should pass input
     signals through unchanged and return an empty vector for empty input.
   - Do not use `roko_std::NoOpGate` as the graph noop unless task 036 or a later task has given it
     an `execute()` override; today it is only a `Verify` no-op.

5. Implement CLI wiring:
   - Create `crates/roko-cli/src/commands/graph.rs`.
   - Prefer `pub(crate) async fn cmd_graph(cli: &Cli, cmd: GraphCmd) -> Result<i32>` matching
     other command modules, then dispatch `GraphCmd::Run { file }`.
   - The CLI call chain should be:
     `main.rs::dispatch_subcommand -> commands::graph::cmd_graph ->
     roko_graph::load_graph -> roko_graph::build_default_registry -> Engine::start ->
     Engine::await_flow`.
   - Use `roko_core::MemoryBus::new(4096)` or the landed task-035 bus-erased helper, and
     `roko_std::MemorySubstrate::new()` for the initial in-memory runtime.
   - Print the graph name, flow id, final status, and output signal count. For each output, print
     `signal.kind.as_str()` and a compact body/tag summary; do not rely on a nonexistent
     `signal.topic` field because `Signal` is currently an alias for `Engram`.

6. Example graphs:
   - Use `cell = "noop"` for at least one example path so the command has a deterministic success
     path independent of the local workspace compiling.
   - Gate examples may produce `GateVerdict` signals with `passed = false` when run in an arbitrary
     workspace; that is still an engine success as long as `Cell::execute()` returns `Ok`.

### Tests to add

- `crates/roko-graph/tests/engine.rs`:
  - Register an `EchoCell`/`AppendCell` test cell and run a two-node linear graph.
  - Assert node execution order by recording calls in a shared vector.
  - Assert unknown cell names return an error containing the missing name.
  - Assert `CellRef::SubGraph` returns the task-069 unsupported error.
- `crates/roko-cli/src/main.rs` parser tests:
  - Add a parse test for `roko graph run examples/graphs/single-gate.toml`.
- Manual CLI verification remains required because parser tests are not runtime wiring.

### Anti-patterns specific to this task

- Do not refactor Runner v2 or make `roko run` use the graph engine.
- Do not implement conditions, mappings, parallel levels, subgraphs, snapshots, budgets, or
  deadlines here. Leave fields present but inert.
- Do not create a second command parser outside clap.
- Do not swallow execution errors. If a cell is missing or `execute()` returns Err, the CLI must
  exit non-zero with context.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
