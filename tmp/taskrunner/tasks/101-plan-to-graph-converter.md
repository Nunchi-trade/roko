# Task 101: Plan-to-Graph Converter + Engine Integration Tests

```toml
id = 101
title = "Implement tasks.toml → Graph converter, --engine graph flag for roko plan run, and integration tests comparing Engine vs Runner v2"
track = "graph-engine"
wave = "wave-5"
priority = "high"
blocked_by = [66, 67, 68, 71]
touches = [
    "crates/roko-graph/src/convert.rs",
    "crates/roko-graph/src/lib.rs",
    "crates/roko-cli/src/runner/mod.rs",
    "crates/roko-cli/src/main.rs",
    "crates/roko-graph/tests/plan_conversion.rs",
]
exclusive_files = ["crates/roko-graph/src/convert.rs"]
estimated_minutes = 360
```

## Context

Phase 4 of v2 refactoring begins here. The Graph Engine (tasks 66-71) is now built and
working for hand-authored graph TOML files. This task bridges the existing plan execution
system to the Engine by implementing a converter from the `tasks.toml` plan format into a
`Graph` struct, and then exercising it through integration tests that compare Engine output
against Runner v2 output on the same five real plans from the repository.

Checklist items covered: **P4-1** (implement plan TOML → Graph converter), **P4-2** (add
`--engine graph` flag to `roko plan run`), and **P4-3** (run 5 existing plans through Engine
path in integration tests).

This is a **redesign task, not a band-aid**. The converter must produce graphs that are
structurally equivalent to what Runner v2 executes — same DAG topology, same task ordering,
same dependency resolution — so that when the Engine becomes the default in task 102, nothing
changes for users.

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/CHECKLIST.md` — items P4-1 through P4-3 define the acceptance criteria
2. `tmp/v2-refactoring/07-GRAPH-ENGINE.md` — the "How Runner v2 Maps to Graphs" section
   (compose → act → verify → persist pattern) is the conceptual model for what the converter
   must produce
3. `crates/roko-cli/src/task_parser.rs` or its extracted successor in `roko-orchestrator`
   — `TasksFile`, `TaskDef`, `TaskMeta`, and the full field set. This is the input to the
   converter. Focus on: `id`, `title`, `role`, `depends_on`, `depends_on_plan`, `files`,
   `verify` steps, `tier`, `model_hint`, `domain`, `timeout_secs`, `max_retries`, and
   `sequence`. If these types still live in `roko-cli`, resolve the crate-boundary blocker
   described below before implementing `roko_graph::convert`.
4. `crates/roko-cli/src/runner/plan_loader.rs` or its extracted successor in
   `roko-orchestrator` — `Plan`, `load_plan()`, `load_plans()`. The converter receives a
   loaded `Plan` and produces a `Graph`.
5. `crates/roko-cli/src/runner/task_dag.rs` — how Runner v2 resolves readiness and
   cross-plan dependencies today. The converter must mirror its in-plan dependency behavior
   for `graph.entry`, `graph.exits`, and edges; it must warn and skip `depends_on_plan`.
6. `crates/roko-graph/src/types.rs` — `Graph`, `Node`, `Edge`, `CellRef`, `NodeKind`,
   `ExecutionClass`, `GraphPolicy`. This is the output type.
7. `crates/roko-graph/src/engine.rs` — `Engine::start()` and `Engine::await_flow()`. These
   are what the new `--engine graph` path calls.
8. `crates/roko-cli/src/main.rs` — the `PlanCmd::Run` variant and its arguments. You will
   add an `--engine` flag here. Follow the existing flag pattern exactly.
9. `crates/roko-cli/src/commands/plan.rs` — where `cmd_plan()` dispatches `PlanCmd::Run`.
   The engine selection logic goes here.
10. `plans/` (workspace root) — find 5 real plans to use in integration tests. Run
    `ls plans/` to see what is available.

## Implementation Detail

### Current source facts to verify first

- In this checkout, `crates/roko-graph/` is not present yet. This task is blocked until
  tasks 66-71 have landed the crate, Engine, default registry, `Node.config`, AgentCell,
  and ComposeCell. If those files are still missing, do not recreate them in this task.
- The live Runner v2 plan types currently live in `roko-cli`, not `roko-orchestrator`:
  - `crates/roko-cli/src/runner/plan_loader.rs` defines `Plan`, `load_plan()`,
    `load_plans()`, and `scaffold_missing_crates()`.
  - `crates/roko-cli/src/task_parser.rs` defines `TasksFile`, `TaskDef`, `TaskMeta`,
    `VerifyStep`, `TaskContext`, and schema validation.
  - `crates/roko-cli/src/runner/task_dag.rs` defines `TaskDag::ready_tasks()` and
    `TaskDef::is_ready_with_plan_deps()` usage.
- `roko-cli` will depend on `roko-graph` for the new engine path, so `roko-graph` must
  not depend on `roko-cli`. If prior tasks have not moved the plan parser/loader types
  into `roko-orchestrator`, this task has a crate-boundary blocker: first extract the
  pure plan data types/loaders into `roko-orchestrator` in a separate prep change or with
  an explicitly widened touch set. Do not solve this by `#[path]` includes, duplicated
  `TaskDef` structs, or a `roko-graph -> roko-cli` dependency.
- `roko-orchestrator/src/dag.rs::UnifiedTaskDag` is a cross-plan DAG over
  `roko_core::task::Task`, not the Runner v2 `TaskDef` format. Use it only as reference
  for deterministic Kahn-sort behavior; do not force Runner v2 task TOML through that
  unrelated type.

### CLI runtime call chain

Current `roko plan run` flow is:

```text
main.rs::dispatch_subcommand
  -> Command::Plan { cmd }
  -> commands/plan.rs::cmd_plan()
  -> PlanCmd::Run arm
  -> validate_before_run()
  -> cmd_plan_dry_run() when --dry-run is set
  -> workspace lock / --fresh state archival / config bootstrap / provider preflight
  -> runner::plan_loader::load_plans()
  -> runner::plan_loader::scaffold_missing_crates()
  -> build runner::RunConfig
  -> runner::event_loop::run(plans, &run_config, &state_hub, cancel)
```

Add engine selection inside the `PlanCmd::Run` arm after validation and `--dry-run`
handling. The Graph branch should share workdir resolution, config bootstrap, fresh-state
handling, provider/gate preflight, and Ctrl-C cancellation setup where those semantics still
apply. Unsupported Runner-only flags must be explicit:

- `--resume-plan`: print `Note: --resume-plan is not yet supported by the Graph Engine; snapshots will be ignored.`
- `--approval`: either reuse the same StateHub/TUI setup if graph events reach it, or return
  a clear unsupported error. Do not silently ignore it.
- `--max-retries`: pass through in node config for future TaskExecutor behavior, but do not
  implement retry/replan here.

### Converter mechanics

- Build a local `HashSet<String>` of all task IDs before creating edges. For each
  `depends_on` entry:
  - if the dependency exists in the same plan, emit one graph edge;
  - if it does not exist, return an error with plan id, task id, and missing dependency.
    Do not let a dangling edge reach `Graph::validate()`.
- Preserve authored task order by sorting/iterating on `TaskDef.sequence` when available.
  If the extracted type lacks `sequence`, preserve the vector order from `TasksFile.tasks`.
- `graph.entry` is every task whose in-plan `depends_on` list is empty after filtering to
  known task IDs. `graph.exits` is every task ID that never appears as an edge `from`.
- Store the full serialized `TaskDef` in `Node.config`, plus small execution metadata needed
  by the cell: `plan_id`, `plan_dir`, and `max_retries_override` if the CLI flag was set.
- `depends_on_plan` is a multi-plan orchestration concern. Emit one `tracing::warn!` per
  skipped dependency and do not add an edge.
- Call `graph.validate()` before returning. The converter is synchronous and must not start
  the Engine or dispatch agents.

### Test plan details

Use real plans that exist in this repository:

- `plans/P06-process-management`
- `plans/P07-autofix-retry`
- `plans/W01-wire-system-prompts`
- `plans/dry-run-flag`
- `plans/live-demo-phase1`

Write one shared helper in `crates/roko-graph/tests/plan_conversion.rs` that loads those
paths from the workspace root and skips missing paths. The five required tests should loop
over the helper's loaded plans rather than each hard-coding a different plan. For edge
assertions, compute expected edges directly from `plan.tasks.tasks[*].depends_on` filtered
to in-plan task IDs and compare as sorted `(from, to)` pairs.

## What to Change

### 1. Implement the converter in `crates/roko-graph/src/convert.rs`

Create a new file. This is the only exclusive file for this task — other changes are additive.

```rust
//! Convert a Runner v2 `Plan` (tasks.toml) into a `Graph` for Engine execution.
//!
//! Mapping:
//! - Each `TaskDef` becomes a Node with `CellRef::Named("task-executor")`
//! - `depends_on` relationships become Edges
//! - Tasks with no incoming edges are `graph.entry`
//! - Tasks with no outgoing edges are `graph.exits`
//! - `TaskMeta.max_parallel` → `GraphPolicy.max_parallelism`
//! - `TaskDef.timeout_secs` → per-node timeout (stored in node config)
//! - `TaskDef.tier` maps to `ExecutionClass::Activity` (all tasks are non-deterministic)

use roko_core::Signal;
use crate::{
    Edge, ExecutionClass, Graph, GraphId, GraphPolicy, Mapping, Node, NodeId, NodeKind, CellRef,
    FailureStrategy,
};

/// Convert a loaded Runner v2 plan into a Graph ready for Engine execution.
///
/// The resulting graph uses `CellRef::Named("task-executor")` for every node.
/// The `task-executor` cell must be registered in the Engine's CellRegistry
/// (see `build_default_registry()` in `crates/roko-graph/src/engine.rs`).
pub fn plan_to_graph(plan: &roko_orchestrator::plan::Plan) -> anyhow::Result<Graph> {
    // ...
}
```

**Mapping rules** (implement each one):

- `plan.id` → `Graph.id` and `Graph.name`
- `plan.tasks.meta.max_parallel` → `GraphPolicy.max_parallelism`
- Each `TaskDef` → one `Node`:
  - `node.id` = `task.id`
  - `node.cell_ref` = `CellRef::Named("task-executor")`
  - `node.kind` = `NodeKind::Cell`
  - `node.execution_class` = `ExecutionClass::Activity` (all LLM-dispatched tasks are non-deterministic)
  - `node.config` = a `serde_json::Value` capturing the full `TaskDef` fields needed by
    `TaskExecutorCell` (title, role, description, files, verify steps, model_hint, timeout_secs,
    max_retries, depends_on, domain). Store the whole `TaskDef` serialized as JSON.
- `task.depends_on` → `Edge { from: dep_id, to: task_id, condition: None, mapping: Some(Mapping::Identity) }`
- `depends_on_plan` edges: skip for now — log a warning and omit. Document this in Status Log.
- Entry nodes: tasks whose `depends_on` is empty after resolving the graph's task set
- Exit nodes: tasks that no other task depends on
- `GraphPolicy.failure_strategy` = `FailureStrategy::ContinueOnFailure` (mirrors Runner v2
  behavior — gate failures trigger replan/retry, not graph abort)
- `GraphPolicy.max_budget` = `None` (not tracked in tasks.toml)
- `GraphPolicy.deadline` = `None`

**Cycle detection**: call `graph.validate()` after construction. If validation fails, return
the error — it means the tasks.toml has a cycle, which is also a bug in the plan.

**Cross-plan dependencies** (`depends_on_plan`): these reference tasks in OTHER plans, which
are outside the scope of a single Graph. For now: emit a `tracing::warn!` for each
cross-plan dependency and skip the edge. Do NOT fail — Runner v2 handles these at a higher
level. Add a note in the Status Log that this is a known gap.

### 2. Add a `TaskExecutorCell` stub to `crates/roko-graph/src/cells/`

The converter emits `CellRef::Named("task-executor")` for every node. The Engine needs a
cell registered under that name. In this task, implement a **stub** `TaskExecutorCell` that
wraps the existing runner dispatch path:

**Crate-boundary correction**: `TaskExecutorCell` may live in `roko-graph` only if it can be
implemented using crates that `roko-graph` can legally depend on (`roko-agent`,
`roko-compose`, `roko-orchestrator`, `roko-core`, etc.). It must not import
`roko_cli::runner` or any `roko-cli` type. If the only available dispatch path is still
inside `roko-cli`, register a dry-run `task-executor` stub in `roko-graph` for tests and
put the real CLI adapter in the `roko plan run --engine graph` branch, or stop and report
the crate-boundary blocker. Do not create a `roko-graph -> roko-cli -> roko-graph` cycle.

```rust
/// Stub cell that delegates to the existing Runner v2 task dispatch.
///
/// In Phase 4 task 102, this will be replaced with a full implementation.
/// For now, it is a thin wrapper that allows integration tests to run.
pub struct TaskExecutorCell {
    /// The full task definition, deserialized from node.config.
    // Internally: deserialize TaskDef from ctx.node_config on each execute() call.
}
```

`TaskExecutorCell::execute()` should:
1. Deserialize the `TaskDef` from `ctx.node_config` (where the converter stored it).
2. Call the existing Runner v2 agent dispatch path for the task.
3. Return the agent output as a `Vec<Signal>` containing a signal with `kind = "task-output"`.

For integration tests, if no LLM provider is configured, `TaskExecutorCell::execute()` should
return a synthetic "dry-run" signal rather than failing. Add a `dry_run: bool` field to
`TaskExecutorCell` for this purpose.

Register `TaskExecutorCell` in `build_default_registry()` under the name `"task-executor"`.

### 3. Add `--engine` flag to `PlanCmd::Run` in `crates/roko-cli/src/main.rs`

Add an `engine` argument to the `Run` variant in `PlanCmd`:

```rust
/// Engine to use for plan execution.
#[arg(long, default_value = "runner-v2", value_enum)]
engine: PlanEngine,
```

Add the enum:

```rust
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum PlanEngine {
    #[default]
    #[value(name = "runner-v2")]
    RunnerV2,
    #[value(name = "graph")]
    Graph,
}
```

This flag must appear in the help text as:
```
  --engine <ENGINE>    Execution engine [default: runner-v2] [possible values: runner-v2, graph]
```

### 4. Wire engine selection in `crates/roko-cli/src/commands/plan.rs`

In `cmd_plan()`, when handling `PlanCmd::Run`, branch on the `engine` field:

```rust
PlanCmd::Run { plans_dir, engine, resume_plan, .. } => {
    match engine {
        PlanEngine::RunnerV2 => {
            // existing runner::run() call — unchanged
        }
        PlanEngine::Graph => {
            cmd_plan_run_engine(plans_dir, /* other flags */).await?
        }
    }
}
```

Implement `cmd_plan_run_engine()`:

```rust
async fn cmd_plan_run_engine(plans_dir: &Path, /* other flags */) -> Result<i32> {
    let plans = roko_orchestrator::plan::load_plans(plans_dir)?;
    let registry = Arc::new(build_default_registry()?);
    let bus = /* in-memory bus */;
    let store = /* in-memory or FileSubstrate */;
    let engine = Engine::new(registry, bus, store);

    for plan in &plans {
        println!("Running plan '{}' via Graph Engine...", plan.id);
        let graph = roko_graph::convert::plan_to_graph(plan)?;
        let flow_id = engine.start(&graph, vec![]).await?;
        let output = engine.await_flow(&flow_id).await?;
        println!(
            "Plan '{}' completed: {} output signals",
            plan.id,
            output.len()
        );
    }
    Ok(0)
}
```

### 5. Write integration tests in `crates/roko-graph/tests/plan_conversion.rs`

Create a test file with at least 5 tests, one per real plan from the repository. Use
`dry_run = true` on `TaskExecutorCell` so tests don't require a configured LLM.

```rust
/// Verify that a real tasks.toml plan converts to a structurally valid Graph.
#[test]
fn convert_plan_preserves_task_count() {
    let plan_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap()  // workspace root
        .join("plans/<plan-id>");
    if !plan_dir.exists() {
        // Skip if this plan doesn't exist in the test environment.
        return;
    }
    let plan = roko_orchestrator::plan::load_plan(&plan_dir).unwrap();
    let graph = roko_graph::convert::plan_to_graph(&plan).unwrap();
    assert_eq!(graph.nodes.len(), plan.tasks.tasks.len(),
        "every task must become exactly one node");
}
```

Write five such tests covering five different plan directories. Name the functions:
- `convert_plan_preserves_task_count` (structural: node count == task count)
- `convert_plan_preserves_dependency_edges` (edges match depends_on relationships)
- `convert_plan_entry_nodes_have_no_incoming_edges` (entry detection correct)
- `convert_plan_exit_nodes_have_no_outgoing_edges` (exit detection correct)
- `convert_plan_validates_cleanly` (graph.validate() returns Ok for all 5 real plans)

For each test, try multiple plan directories and pick the first one that exists.

Also add an async integration test that runs a plan through the Engine in dry-run mode:

```rust
#[tokio::test]
async fn engine_can_run_converted_plan_dry_run() {
    // Load a small real plan, convert it, run via Engine with dry_run TaskExecutorCell.
    // Assert: flow completes without error, output signals count >= 0.
}
```

## What NOT to Do

- Do NOT change how `PlanEngine::RunnerV2` dispatches — the existing runner path is
  completely untouched. The `--engine graph` flag is an additive opt-in.
- Do NOT implement retry/replan logic in `TaskExecutorCell` for this task. That logic lives
  in Runner v2 and will be ported to Engine in task 102 after the default switches.
- Do NOT implement cross-plan dependency resolution (depends_on_plan). Warn and skip.
- Do NOT implement `FlowSnapshot`/resume for the Engine path yet. That is task 069 and is
  a prerequisite for the Graph Engine being production-ready.
- Do NOT remove or modify any existing `PlanCmd::Run` fields — only add the `engine` field.
- Do NOT make the integration tests require a running LLM. All tests must pass in CI with
  `dry_run = true` on `TaskExecutorCell`.
- Do NOT implement the converter as a recursive call into the Engine execution loop — it is
  a pure data transformation from `Plan` → `Graph`. No async, no side effects.

## Wire Target

```bash
# Verify the flag exists
cargo run -p roko-cli -- plan run --help | grep -A1 'engine'
# Expected: --engine <ENGINE>    Execution engine [default: runner-v2]

# Run a small plan via the new engine path (dry-run if no LLM configured)
cargo run -p roko-cli -- plan run plans/ --engine graph
# Expected: "Running plan '...' via Graph Engine..." then "Plan '...' completed: N output signals"

# Confirm runner-v2 path unchanged
cargo run -p roko-cli -- plan run plans/ --engine runner-v2
# Expected: same behavior as before this task
```

## Verification

- [ ] `cargo build --workspace` — compiles clean
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo test -p roko-graph -- plan_conversion` — all 5 conversion tests pass
- [ ] `cargo test -p roko-graph -- engine_can_run_converted_plan_dry_run` — async test passes
- [ ] `cargo run -p roko-cli -- plan run --help` — shows `--engine` flag with `runner-v2` default
- [ ] `cargo run -p roko-cli -- plan run plans/ --engine graph` — runs without panic
- [ ] `cargo run -p roko-cli -- plan run plans/ --engine runner-v2` — unchanged behavior
- [ ] `grep -rn 'plan_to_graph' crates/roko-graph/src/ --include='*.rs' | grep -v target/` — function exists
- [ ] `grep -rn 'task-executor' crates/roko-graph/ --include='*.rs' | grep -v target/` — cell registered
- [ ] `grep -rn 'PlanEngine' crates/roko-cli/src/ --include='*.rs' | grep -v target/` — enum wired in CLI
- [ ] Converted graph for any real plan passes `graph.validate()` without error
- [ ] `depends_on` relationships preserved as edges: for a task with `depends_on = ["T1"]`,
  the graph has an edge `from = "T1", to = "<that task>"`
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in `convert.rs`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
