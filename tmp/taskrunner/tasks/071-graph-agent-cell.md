# Task 071: Agent as Cell — AgentCell, ComposeCell, and Task-Execution Graph

```toml
id = 71
title = "Implement AgentCell and ComposeCell wrappers, write end-to-end task-execution graph"
track = "graph-engine"
wave = "wave-3"
priority = "high"
blocked_by = [67, 68]
touches = [
    "crates/roko-graph/src/cells/mod.rs",
    "crates/roko-graph/src/cells/agent.rs",
    "crates/roko-graph/src/cells/compose.rs",
    "crates/roko-graph/src/engine.rs",
    "crates/roko-graph/src/lib.rs",
    "crates/roko-cli/src/commands/graph.rs",
    "examples/graphs/task-execution.toml",
]
exclusive_files = [
    "crates/roko-graph/src/cells/mod.rs",
    "crates/roko-graph/src/cells/agent.rs",
    "crates/roko-graph/src/cells/compose.rs",
    "examples/graphs/task-execution.toml",
]
estimated_minutes = 360
```

## Context

This task is the capstone of Phase 2: making agents and prompt composition available as Graph
cells (P2-15 through P2-17). Once AgentCell and ComposeCell exist, a graph can express the full
task-execution loop: compose a prompt -> dispatch to an LLM agent -> verify with gates ->
persist results. This is the proof that the Graph Engine can eventually replace Runner v2.

AgentCell wraps the existing agent dispatch system (roko-agent). ComposeCell wraps the
SystemPromptBuilder (roko-compose). Neither creates new LLM or prompt logic — they are thin
adapters that make existing subsystems callable as Cells.

The task-execution graph (`task-execution.toml`) is the end-to-end demonstration: it takes
a coding task description as input, composes a system prompt, dispatches to Claude, runs the
gate pipeline, and outputs the result. This is the same workflow that Runner v2 does
procedurally, expressed declaratively as a graph.

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/07-GRAPH-ENGINE.md` — AgentCell and ComposeCell design, the
   task-execution graph TOML example (compose -> act -> verify -> persist)
2. `crates/roko-agent/src/dispatcher/mod.rs` — Agent dispatch. This is what AgentCell wraps.
   Understand the DispatchRequest/DispatchResponse types and how dispatch_to() works.
3. `crates/roko-compose/src/system_prompt_builder.rs` — SystemPromptBuilder. This is what
   ComposeCell wraps. Understand how build() is called and what it returns.
4. `crates/roko-compose/src/templates/` — Role templates (implementer, reviewer, etc.).
   ComposeCell needs to accept a role parameter.
5. `crates/roko-core/src/cell.rs` — Cell trait with execute(). AgentCell and ComposeCell
   implement this trait.
6. `crates/roko-graph/src/engine.rs` — Engine from tasks 066-068. The new cells must work
   within the Engine's execution loop.
7. `crates/roko-cli/src/commands/graph.rs` — CLI command. You will register the new cells
   in `build_default_registry()`.
8. `crates/roko-agent/src/task_runner.rs` — How tasks are currently dispatched. Understand
   the full flow so AgentCell can replicate it within the graph.

## What to Change

### 1. Create `crates/roko-graph/src/cells/` module

Create a `cells` module directory with `mod.rs`, `agent.rs`, and `compose.rs`.

### 2. Implement AgentCell in `cells/agent.rs`

```rust
pub struct AgentCell {
    dispatcher: Arc<dyn Dispatcher>,
    model_hint: Option<String>,
    backend: Option<String>,
}
```

`AgentCell` implements the `Cell` trait:

- `cell_id()` / `cell_name()` — `"agent-cell"` / `"claude-agent"` (or parameterized)
- `execution_class` is always `Activity` (non-deterministic LLM call)
- `estimated_cost()` — return a rough estimate based on model (e.g., $0.01 for a typical
  Claude call). This enables budget tracking.
- `execute(input, ctx)`:
  1. Extract the prompt/task description from input signals. Look for a signal with
     `kind = "prompt"` or `kind = "task"`, falling back to the first signal's body.
  2. Build a `DispatchRequest` from the prompt and AgentCell's config (model, backend).
  3. Call `dispatcher.dispatch_to(&request)` (or the appropriate dispatch method).
  4. Convert the `DispatchResponse` into output signals:
     - Signal with `kind = "agent-response"`, body = response text
     - If the agent produced tool calls or structured output, include those as separate signals

**Configuration from graph TOML**: AgentCell needs to accept parameters from the node
definition. Add a `config` field to the Node struct (or use node metadata):

```toml
[[graph.nodes]]
id = "act"
cell = "claude-agent"
kind = "Cell"
execution_class = "Activity"

[graph.nodes.config]
model = "claude-sonnet-4-20250514"
backend = "claude-cli"
```

The `config` field is a `serde_json::Value` (or `toml::Value`) that gets passed to the Cell
at construction time via the registry or at execution time via CellContext.

### 3. Implement ComposeCell in `cells/compose.rs`

```rust
pub struct ComposeCell {
    role: String,  // "implementer", "reviewer", etc.
}
```

`ComposeCell` implements the `Cell` trait:

- `cell_id()` / `cell_name()` — `"compose-cell"` / `"system-prompt-builder"`
- `execution_class` is `Workflow` (deterministic prompt assembly)
- `estimated_cost()` — `Some(0.0)` (no LLM cost, just string assembly)
- `execute(input, ctx)`:
  1. Extract task context from input signals (task description, codebase context, etc.)
  2. Build a `RoleSystemPromptSpec` from the role and input context
  3. Call `SystemPromptBuilder::new().build(&spec)` to assemble the 9-layer prompt
  4. Return a signal with `kind = "prompt"`, body = the assembled system prompt

**Configuration from graph TOML**:

```toml
[[graph.nodes]]
id = "compose"
cell = "system-prompt-builder"
kind = "Cell"

[graph.nodes.config]
role = "implementer"
```

### 4. Add node config support to types and loader

Extend `Node` in `types.rs`:

```rust
pub struct Node {
    pub id: NodeId,
    pub cell_ref: CellRef,
    pub kind: NodeKind,
    pub execution_class: ExecutionClass,
    pub config: Option<serde_json::Value>,  // NEW: per-node configuration
}
```

Update the TOML loader to parse `[graph.nodes.config]` sections.

Update CellRegistry to support parameterized cell construction — either:
- `register_factory(name, factory_fn)` where factory takes config and returns a Cell, OR
- Pass `node.config` to `Cell::execute()` via `CellContext`

The simpler approach (pass config via CellContext) is preferred for this task. Add a `config`
field to `CellContext`:

```rust
pub struct CellContext {
    // ... existing fields ...
    pub node_config: Option<serde_json::Value>,
}
```

### 5. Register new cells in `build_default_registry()`

```rust
fn build_default_registry(config: &RokoConfig) -> Result<CellRegistry> {
    let mut reg = CellRegistry::new();
    // ... existing gates ...
    reg.register("claude-agent", Arc::new(AgentCell::new(dispatcher)));
    reg.register("system-prompt-builder", Arc::new(ComposeCell::new("implementer")));
    Ok(reg)
}
```

The `dispatcher` comes from the same initialization path as the CLI's agent dispatch
(see `crates/roko-cli/src/dispatch/mod.rs` or `crates/roko-agent/src/dispatcher/`).

### 6. Write the task-execution graph

Create `examples/graphs/task-execution.toml`:

```toml
[graph]
name = "task-execution"
version = "0.1.0"

[[graph.nodes]]
id = "compose"
cell = "system-prompt-builder"
kind = "Cell"
execution_class = "Workflow"

[graph.nodes.config]
role = "implementer"

[[graph.nodes]]
id = "act"
cell = "claude-agent"
kind = "Cell"
execution_class = "Activity"

[graph.nodes.config]
model = "claude-sonnet-4-20250514"

[[graph.nodes]]
id = "verify"
cell = "compile-gate"
kind = "Cell"
execution_class = "Workflow"

[[graph.edges]]
from = "compose"
to = "act"

[[graph.edges]]
from = "act"
to = "verify"

[graph.policy]
failure_strategy = "FailFast"
max_budget = 0.50
deadline = "300s"
```

This graph demonstrates: compose prompt -> agent generates code -> compile gate verifies.

### 7. Add tests

- Unit test: AgentCell with a mock dispatcher produces correct output signals
- Unit test: ComposeCell produces a signal with kind "prompt" containing assembled prompt text
- Integration test: run task-execution.toml (with mock agent or real agent if configured)
- Unit test: node config is correctly parsed from TOML and available in CellContext
- Test that AgentCell reports correct `execution_class` (Activity) and `estimated_cost`

## What NOT to Do

- Do NOT reimplement agent dispatch logic. AgentCell wraps the existing dispatcher, it does not
  replace it.
- Do NOT reimplement prompt building. ComposeCell wraps SystemPromptBuilder, it does not
  replace it.
- Do NOT add new LLM backends or prompt templates. Use what exists.
- Do NOT implement retry logic in AgentCell — that is handled by Engine's FailureStrategy.
- Do NOT implement a "persist" cell in this task. The graph outputs signals; persistence is
  a concern for Phase 4 when the Engine replaces Runner v2.
- Do NOT make AgentCell work without a configured LLM provider — if no provider is configured,
  AgentCell::execute() should return a clear error, not a mock response.
- Do NOT implement Hot Graphs or agent cognitive loops. That is Phase 4.
- Do NOT break existing graphs that don't use AgentCell/ComposeCell.

## Wire Target

```bash
# ComposeCell only (no LLM needed)
cargo run -p roko-cli -- graph run examples/graphs/task-execution.toml --dry-run
# Expected: Compose step runs, agent step is skipped (dry-run), output shows prompt

# Full execution (requires LLM provider configured in roko.toml)
cargo run -p roko-cli -- graph run examples/graphs/task-execution.toml
# Expected: Compose -> Agent -> Verify pipeline runs end-to-end
```

Note: `--dry-run` may need to be added to the graph run command. If not feasible, the wire
target without dry-run requires a configured LLM provider. Document which provider is needed.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/task-execution.toml` — runs (with
  configured provider) or produces clear error about missing provider
- [ ] AgentCell unit test with mock dispatcher passes
- [ ] ComposeCell unit test produces valid prompt signal
- [ ] Node config parsed correctly from TOML
- [ ] Existing example graphs from tasks 067-070 still work unchanged
- [ ] `grep -rn 'AgentCell' crates/ --include='*.rs' | grep -v target/ | grep -v test` — registered in default registry
- [ ] `grep -rn 'ComposeCell' crates/ --include='*.rs' | grep -v target/ | grep -v test` — registered in default registry
- [ ] `grep -rn 'claude-agent\|system-prompt-builder' crates/roko-graph/ --include='*.rs' | grep -v target/` — cell names used
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file

## Implementation Detail

### Prerequisite and API Checks

- This task is downstream of tasks 066-070. In the pre-graph checkout, `crates/roko-graph` is absent and `crates/roko-core/src/cell.rs` still exposes only metadata methods on `Cell`. Do not create a parallel graph API here. Start by confirming the merged graph crate has `Cell::execute`, `CellContext`, a loader, a registry, and an engine entrypoint from the earlier tasks.
- If the landed graph API has no way to attach per-node config to `CellContext`, add that to the graph/core API used by the real engine path. Do not read node config from globals, environment variables, or ad hoc side tables.
- The source touch list is too narrow for a complete implementation. Expect source changes in `crates/roko-graph/src/types.rs`, `loader.rs`, `registry.rs`, `engine.rs`, `crates/roko-graph/Cargo.toml`, and graph tests in addition to `agent.rs`/`compose.rs`. If the runner enforces exact source touch lists, update ownership before implementation.

### Runtime Call Chain to Wire

`crates/roko-cli/src/main.rs` -> `commands::graph::cmd_graph_run` -> graph TOML loader -> default `CellRegistry` construction -> graph engine `start`/`execute_graph` -> per-node `Cell::execute`.

The agent and compose cells must be reachable through that call chain by cell type name in graph TOML. A unit-only constructor or an unregistered cell is not complete.

### Mechanical Implementation Steps

1. Extend the graph node type and TOML loader with optional `config: serde_json::Value` or the equivalent structured map. Preserve existing node fields and reject malformed config with the loader's normal error style.
2. In the engine, clone or derive `CellContext` for each node and attach that node's config before calling `Cell::execute`. Keep graph-wide runtime state, budget, deadlines, and snapshots on the existing context path.
3. Implement `ComposeCell` with the existing compose APIs: `roko_compose::RoleSystemPromptSpec` and `TaskContext`. `SystemPromptBuilder::new` currently requires a role identity and `.build()` takes no spec argument, so do not invent a `SystemPromptBuilder::new().build(&spec)` flow.
4. Parse role config into `roko_core::agent::AgentRole` with an explicit match or serde kebab-case deserialization. Do not assume a `FromStr` implementation.
5. Build `TaskContext` from node config plus input signals: task text, goal, workspace, domain notes, prior gate feedback, and any model/context-window hints. Emit a prompt signal/engram that downstream `agent` nodes can consume directly.
6. Implement `AgentCell` through `roko_agent::provider::create_agent_for_model`, `AgentOptions`, and the `Agent::run(&Engram, &Context)` trait path. Use `ProviderCallCell` in `crates/roko-agent/src/model_call_service.rs` as the closest existing pattern for building prompt engrams, creating an agent, running it, and mapping usage/cost.
7. Resolve the model key from node config first, then the configured default model. Return a graph execution error when no provider/model can be resolved; do not silently fall back to a placeholder.
8. Register both cells in the default graph registry built by task 067. Use stable type names `compose` and `agent`, plus aliases only if the graph registry already has an alias pattern.

### Tests to Add or Update

- Compose unit test: graph node config plus representative input signals produces the expected role/task prompt content and output kind.
- Agent unit test: inject a fake or mock `Agent`/factory if the graph crate has a test seam; otherwise use the existing mock provider environment path supported by `create_agent_for_model`.
- Graph integration test: load a two-node graph (`compose` -> `agent`), run it through the real engine, and assert the agent receives the composed prompt and emits an output signal.
- CLI smoke test if the repo has CLI integration tests: `roko graph run examples/graphs/agent-plan.toml` with a mock provider path.

### Additional What NOT To Do

- Do not route through `crates/roko-agent/src/dispatcher/mod.rs`; that module is for tool dispatch in the current tree, not the provider-backed LLM agent runtime.
- Do not hard-code one provider, one model, or Claude-specific behavior in graph cells.
- Do not make graph nodes read config from process environment as a substitute for TOML node config.
- Do not leave the example graph depending on live credentials for the normal test path.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
