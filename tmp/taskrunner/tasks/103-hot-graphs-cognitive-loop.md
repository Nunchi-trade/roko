# Task 103: Hot Graphs + Cognitive Loop

```toml
id = 103
title = "Add Hot Graph support to Engine (tick-driven, resident) and define agent cognitive loop as declarative TOML Graph"
track = "graph-engine"
wave = "wave-5"
priority = "high"
blocked_by = [102]
touches = [
    "crates/roko-graph/src/engine.rs",
    "crates/roko-graph/src/hot.rs",
    "crates/roko-cli/src/commands/agent.rs",
    "examples/graphs/cognitive-loop.toml",
]
exclusive_files = ["crates/roko-graph/src/hot.rs"]
estimated_minutes = 480
```

## Context

The Graph Engine currently runs a Graph to completion and exits. Hot Graphs are persistent,
tick-driven Graph instances that loop until explicitly stopped — the execution analog of a
long-running process. `roko agent start` currently spawns an agent using a procedural loop
in `run_agent_start()`. This task replaces that procedural loop with a Hot Graph executing
a declarative cognitive loop defined in TOML.

Checklist items covered: **P4-6** (Hot Graph support — tick-driven, resident, persist state
across ticks) and **P4-7** (define cognitive loop as a declarative TOML Graph: Sense →
Assess → Compose → Act → Verify → Persist → React).

This is a **redesign task, not a band-aid**. The cognitive loop is currently scattered across
`run_agent_start()`, `run_agent_serve()`, and various agent state management functions. After
this task, the loop is a TOML file. The procedural code becomes thin glue that loads the
TOML and hands it to the Engine.

The cognitive loop TOML must be checkable into the repository at `examples/graphs/cognitive-loop.toml`
and loadable by `roko agent start` via the standard `roko_graph::load_graph()` function.

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/CHECKLIST.md` — items P4-6 and P4-7, done definitions: "Hot Graphs
   persist state across ticks" and "Agent cognitive loop is declarative TOML, not procedural code"
2. `tmp/v2-refactoring/07-GRAPH-ENGINE.md` — the "Step 5: Incrementally add features" section
   on Hot Graphs. The spec says: "Hot Graphs persist state across ticks." Read the entire doc
   to understand how Hot Graphs differ from one-shot Graph execution.
3. `crates/roko-cli/src/agent_serve.rs` — the current `run_agent_start()` function (around
   line 957). This is the procedural loop you are replacing. Read the full function and the
   agent lifecycle: manifest loading, entry registration, port binding, and the serve loop.
4. `crates/roko-graph/src/engine.rs` — the existing Engine. Hot Graphs add a new execution
   mode alongside the existing `start()`/`await_flow()` pair. Do NOT change `start()` —
   add a new `start_hot()` method.
5. `crates/roko-graph/src/types.rs` — `Graph`, `Node`, `GraphPolicy`. Hot Graphs need a new
   `GraphPolicy.hot: bool` or a separate `HotGraphPolicy` struct. Choose the simplest option
   that captures tick interval and max tick count.
6. `crates/roko-graph/src/cells/agent.rs` — `AgentCell` from task 071. The cognitive loop
   uses this cell for the "Act" step (LLM dispatch). Do NOT reimplement it.
7. `crates/roko-graph/src/cells/compose.rs` — `ComposeCell` from task 071. Used for the
   "Compose" step (system prompt assembly). Do NOT reimplement it.
8. `crates/roko-cli/src/commands/agent.rs` — `cmd_agent()`. The `AgentCmd::Start` branch
   calls `run_agent_start()`. After this task, it calls `run_agent_start_hot_graph()` instead.
9. `examples/graphs/` — existing example graphs from tasks 067-071. Follow their TOML format.
   The cognitive loop TOML follows the same format, with 7 nodes.

## Implementation Detail

### Current source facts to verify first

- In this checkout, `crates/roko-graph/` and `examples/graphs/` are not present yet. This
  task is blocked until tasks 66-71 and task 102 have landed. Do not recreate the graph
  crate from this task.
- The touch list is incomplete for the described implementation. Hot policy parsing and
  stubs require changes to `crates/roko-graph/src/types.rs`, `loader.rs`, `lib.rs`, likely
  `cells/mod.rs`, `cells/stubs.rs`, `Cargo.toml`, and tests under `crates/roko-graph/tests/`.
  If those files are not already included by prior task metadata, record the required touch
  expansion before implementing.
- `roko agent start` does not currently run an inline cognitive loop. The call chain is:

```text
main.rs::dispatch_subcommand
  -> Command::Agent { cmd }
  -> commands/agent.rs::cmd_agent()
  -> agent_serve::run(cmd)
  -> AgentCmd::Start branch
  -> run_agent_start(name, bind, workdir)
  -> spawns detached `roko agent serve --agent-id <name> --bind <bind>`
```

- The long-running sidecar is `crates/roko-cli/src/agent_serve.rs::AgentServeRuntimeConfig::run()`,
  which builds an `AgentServer` and awaits `server.serve().await`. If the hot graph is meant
  to be resident for a started agent, wire it into the spawned `agent serve` process, not by
  making `agent start` block forever. `agent start` should continue to return after spawning
  and registering the sidecar.

### Hot Graph mechanics

- Add `HotPolicy` beside `GraphPolicy` in `types.rs` and parse `[graph.policy.hot]` through
  the existing TOML loader. `GraphPolicy.hot = None` must preserve one-shot graph behavior.
- `Engine` must be cloneable without cloning or losing active flow state. Prefer storing
  registry, bus, store, flow table, and cancellation root in `Arc` fields and deriving or
  implementing `Clone` over those handles.
- Refactor the private one-shot execution function into a `pub(crate)` method such as
  `execute_graph_once(&self, graph: &Graph, input: Vec<Signal>) -> Result<Vec<Signal>>`.
  `start()`/`await_flow()` must continue to use the same one-shot path as before.
- `start_hot(graph)` returns immediately with `HotGraphHandle` and spawns the tick loop.
  The handle needs an observable completion path for tests; add either
  `async fn wait(self) -> Result<()>` or expose the internal join handle behind a method.
  Without this, `hot_graph_respects_max_ticks` can race.
- Persisted tick state can start as exit signals stored under a sentinel node id such as
  `"tick-output"`. Do not claim per-node state persistence unless `execute_graph_once`
  actually returns per-node outputs.
- When `tick_interval_ms > 0`, use `tokio::select!` on sleep vs cancellation so `cancel()`
  stops promptly instead of waiting for a long interval.

### Cognitive loop wiring

- Keep `run_agent_start()` as the process supervisor that spawns the sidecar and writes
  `.roko/agents.json`. Do not replace it with a foreground hot graph loop.
- Add the hot graph to the sidecar path:
  - load/create the manifest in `run_agent_start()` exactly as today;
  - spawn `roko agent serve` as today, optionally adding a flag/env var if needed to enable
    the hot graph in the sidecar;
  - in `AgentServeRuntimeConfig::run()`, after successful server build/startup wiring, start
    the cognitive-loop Hot Graph on the same cancellation lifecycle as the sidecar.
- If integrating with `AgentServer::serve()` makes the hot graph lifecycle awkward, add a
  side task that starts before `server.serve().await` and is cancelled after serve returns.
  Do not let a hot graph task outlive the agent sidecar process.
- Loading order for the graph path:
  1. manifest-configured cognitive loop path if such a manifest field already exists;
  2. workspace-relative `examples/graphs/cognitive-loop.toml`;
  3. repository-root `examples/graphs/cognitive-loop.toml`;
  4. fallback to existing procedural `AgentServer::serve()` with a warning.

### Test plan details

- `crates/roko-graph/tests/hot_graph.rs`: use a trivial graph with a registered `noop`
  or passthrough cell, `tick_interval_ms = 1`, and `max_ticks = 3`. Await completion through
  the handle's wait method, then assert `tick_count() == 3`.
- Cancellation test: start a graph with no `max_ticks`, call `handle.cancel()`, and wrap
  `handle.wait()` in `tokio::time::timeout(Duration::from_millis(250), ...)`.
- TOML parse test: load `examples/graphs/cognitive-loop.toml`, assert node IDs exactly
  `["sense", "assess", "compose", "act", "verify", "persist", "react"]`, assert
  `graph.policy.hot.is_some()`, then call `graph.validate()`.
- CLI smoke: `roko agent start --name test-agent` should still return after starting the
  sidecar. Verify the sidecar logs contain hot graph startup/tick messages, then stop it
  with `roko agent stop --name test-agent`.

## What to Change

### 1. Add `HotPolicy` to Graph types in `crates/roko-graph/src/types.rs`

A Hot Graph has a `HotPolicy` that controls its tick behavior. Add this to `GraphPolicy`:

```rust
/// Controls tick-driven (Hot Graph) execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HotPolicy {
    /// How long to wait between ticks (ms). 0 = run as fast as possible.
    pub tick_interval_ms: u64,
    /// Stop after this many ticks. None = run until cancelled.
    pub max_ticks: Option<u64>,
    /// If true, persist cell output state between ticks so cells can
    /// resume from their previous output.
    pub persist_tick_state: bool,
}
```

Add `hot: Option<HotPolicy>` to `GraphPolicy`. When `hot` is `None`, the Graph executes
once (existing behavior). When `hot` is `Some(_)`, the Engine runs it in tick mode.

Update the TOML loader to parse `[graph.policy.hot]` sections.

### 2. Implement `HotGraphHandle` and `start_hot()` in `crates/roko-graph/src/hot.rs`

Create a new file `hot.rs` (this is the exclusive file for this task):

```rust
//! Hot Graph execution — tick-driven, resident Graph instances.
//!
//! A Hot Graph is a Graph with `policy.hot` set. The Engine runs it in a
//! loop, executing all nodes once per tick, persisting outputs between ticks
//! so each tick starts from the previous tick's state.
//!
//! Hot Graphs run until:
//!   1. `HotPolicy.max_ticks` is reached, OR
//!   2. The `HotGraphHandle.cancel()` token is triggered, OR
//!   3. A node returns an unrecoverable error (non-retriable failure).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use roko_core::Signal;
use tokio_util::sync::CancellationToken;

use crate::{FlowId, FlowStatus, Graph, NodeId};
use crate::engine::Engine;

/// A running Hot Graph instance.
pub struct HotGraphHandle {
    pub flow_id: FlowId,
    /// Cancel token — call `.cancel()` to stop the tick loop.
    pub cancel: CancellationToken,
    /// Monotonic tick counter.
    pub tick: Arc<std::sync::atomic::AtomicU64>,
    /// Most recent outputs per node (persisted across ticks when `persist_tick_state = true`).
    pub tick_state: Arc<DashMap<NodeId, Vec<Signal>>>,
}

impl HotGraphHandle {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn tick_count(&self) -> u64 {
        self.tick.load(std::sync::atomic::Ordering::Relaxed)
    }
}
```

Add `start_hot()` to `Engine` in `engine.rs`:

```rust
impl Engine {
    /// Start a Hot Graph. Returns a handle immediately; execution continues
    /// on a background task until the handle is cancelled or max_ticks reached.
    pub async fn start_hot(&self, graph: Graph) -> Result<HotGraphHandle> {
        let hot_policy = graph.policy.hot.clone()
            .ok_or_else(|| anyhow::anyhow!("Graph '{}' has no hot policy", graph.name))?;

        let cancel = self.cancel.child_token();
        let tick = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tick_state: Arc<DashMap<NodeId, Vec<Signal>>> = Arc::new(DashMap::new());
        let flow_id = uuid::Uuid::new_v4().to_string();

        let engine = self.clone(); // Engine must be Clone (add derive or manual impl)
        let graph_clone = graph.clone();
        let cancel_clone = cancel.clone();
        let tick_clone = tick.clone();
        let state_clone = tick_state.clone();

        tokio::spawn(async move {
            let mut current_tick = 0u64;
            loop {
                // Check cancellation and max_ticks before each tick.
                if cancel_clone.is_cancelled() {
                    break;
                }
                if let Some(max) = hot_policy.max_ticks {
                    if current_tick >= max {
                        break;
                    }
                }

                // Build initial signals from previous tick state.
                let initial: Vec<Signal> = if hot_policy.persist_tick_state {
                    state_clone.iter()
                        .flat_map(|e| e.value().clone())
                        .collect()
                } else {
                    vec![]
                };

                // Run one tick of the graph.
                match engine.execute_graph_once(&graph_clone, initial).await {
                    Ok(outputs) => {
                        // Persist tick state.
                        if hot_policy.persist_tick_state {
                            // Re-populate state_clone with per-node outputs.
                            // execute_graph_once must return per-node outputs for this.
                            // If it only returns exit signals, use those as next tick's input.
                            // Design note: refactor execute_graph_once to return both
                            // node-level and exit-level outputs if persist_tick_state is needed.
                            // For now: store exit signals under "tick-output" key.
                            state_clone.insert("tick-output".to_string(), outputs);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Hot Graph tick {} failed: {e}", current_tick);
                        // Continue on failure unless the graph policy says FailFast.
                        if matches!(graph_clone.policy.failure_strategy,
                                    crate::FailureStrategy::FailFast) {
                            break;
                        }
                    }
                }

                current_tick += 1;
                tick_clone.store(current_tick, std::sync::atomic::Ordering::Relaxed);

                // Wait for next tick.
                if hot_policy.tick_interval_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(hot_policy.tick_interval_ms)).await;
                }

                // Check cancellation after sleep.
                if cancel_clone.is_cancelled() {
                    break;
                }
            }
        });

        Ok(HotGraphHandle { flow_id, cancel, tick, tick_state })
    }
}
```

**Note on `execute_graph_once`**: refactor the existing `execute_graph()` private method in
`engine.rs` to be accessible as `execute_graph_once()` from `hot.rs`. It should have the
same signature as the existing private method. Add `pub(crate)` visibility.

### 3. Write the cognitive loop TOML at `examples/graphs/cognitive-loop.toml`

The cognitive loop is the 7-step cycle that every agent executes:

```toml
# examples/graphs/cognitive-loop.toml
#
# Roko Agent Cognitive Loop
# ─────────────────────────
# This graph defines the canonical agent execution cycle:
#
#   Sense → Assess → Compose → Act → Verify → Persist → React
#
# It is a Hot Graph: it loops indefinitely until cancelled.
# Each tick = one agent turn.

[graph]
name = "cognitive-loop"
version = "0.1.0"

# ── Nodes ──────────────────────────────────────────────────────────────────

[[graph.nodes]]
id = "sense"
cell = "signal-reader"
kind = "Cell"
execution_class = "Workflow"

[graph.nodes.config]
# Read pending signals from the store (tasks, events, user messages).
sources = ["task-queue", "event-bus"]

[[graph.nodes]]
id = "assess"
cell = "relevance-scorer"
kind = "Cell"
execution_class = "Workflow"

[graph.nodes.config]
# Score sensed signals for relevance. High-relevance signals trigger action.
threshold = 0.5

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
# Backend and model are resolved from roko.toml at runtime.
backend = "auto"

[[graph.nodes]]
id = "verify"
cell = "gate-pipeline"
kind = "Cell"
execution_class = "Workflow"

[graph.nodes.config]
# Run all gates configured for the agent's domain.
rungs = ["compile", "test", "clippy"]

[[graph.nodes]]
id = "persist"
cell = "store-writer"
kind = "Cell"
execution_class = "Workflow"

[graph.nodes.config]
# Write verified outputs to the signal store.
target = "signal-store"

[[graph.nodes]]
id = "react"
cell = "event-publisher"
kind = "Cell"
execution_class = "Workflow"

[graph.nodes.config]
# Publish completion events to the bus so watchers and the TUI update.
topics = ["agent.turn.completed", "agent.state.updated"]

# ── Edges ──────────────────────────────────────────────────────────────────

[[graph.edges]]
from = "sense"
to = "assess"

[[graph.edges]]
from = "assess"
to = "compose"
# Only proceed to compose if sense found signals above the relevance threshold.
condition = "has_relevant_signals"

[[graph.edges]]
from = "compose"
to = "act"

[[graph.edges]]
from = "act"
to = "verify"

[[graph.edges]]
from = "verify"
to = "persist"
# Only persist if verification passed.
condition = "verdict.hard_pass == true"

[[graph.edges]]
from = "verify"
to = "react"
# React to failures too — publish error events.

[[graph.edges]]
from = "persist"
to = "react"

# ── Policy ─────────────────────────────────────────────────────────────────

[graph.policy]
failure_strategy = "ContinueOnFailure"
max_parallelism = 1

[graph.policy.hot]
# Run one tick per second (one agent turn per second maximum).
tick_interval_ms = 1000
# No max_ticks — run until the agent is stopped.
persist_tick_state = true
```

**Note on unregistered cells**: `signal-reader`, `relevance-scorer`, `gate-pipeline`,
`store-writer`, `event-publisher` are named in the TOML but may not be registered in the
default registry yet. For this task, register **stub** implementations that pass signals
through (or return empty output) with a `tracing::info!` log. The cognitive loop TOML must
load without error — the stubs ensure it does. Real implementations of each stub cell are
separate tasks.

Add stubs to `crates/roko-graph/src/cells/stubs.rs`:

```rust
/// Stub cell — passes input signals through unchanged.
/// Used as a placeholder until the real implementation is built.
pub struct PassthroughCell { pub name: String }

#[async_trait::async_trait]
impl Cell for PassthroughCell {
    fn cell_id(&self) -> &str { &self.name }
    fn cell_name(&self) -> &str { &self.name }
    async fn execute(&self, input: Vec<Signal>, _ctx: &CellContext) -> Result<Vec<Signal>> {
        tracing::info!("PassthroughCell '{}' — {} input signals (stub)", self.name, input.len());
        Ok(input)
    }
}
```

Register in `build_default_registry()`:

```rust
for name in &["signal-reader", "relevance-scorer", "gate-pipeline", "store-writer", "event-publisher"] {
    reg.register(name, Arc::new(PassthroughCell { name: (*name).to_string() }));
}
```

### 4. Wire `roko agent start` to use the cognitive loop Hot Graph

Preserve the existing process model: `agent start` is a supervisor command that starts a
detached `agent serve` sidecar and returns. The Hot Graph should run inside the sidecar
started by `agent serve`, not in the short-lived `agent start` process.

In `crates/roko-cli/src/agent_serve.rs`, add a new function
`run_agent_start_hot_graph()` (called through the existing `commands/agent.rs -> agent_serve::run`
path) that:

1. Loads the cognitive loop graph from `examples/graphs/cognitive-loop.toml` (or from a
   path configured in the agent manifest — prefer the manifest field if present, fall back to
   the built-in path).
2. Builds the default registry and Engine.
3. Calls `engine.start_hot(graph)` to get a `HotGraphHandle`.
4. Registers the agent in the runtime agent list (same as `run_agent_start()` does today).
5. Blocks on `handle.cancel` token (waits until the agent is stopped via `roko agent stop`
   or Ctrl+C).
6. On cancellation: prints a "agent stopped after N ticks" message and returns.

Wire this into the `AgentCmd::Start` branch:

```rust
AgentCmd::Start { name, bind, workdir } => {
    // Keep the existing manifest-loading, validation, sidecar spawn, and agents.json logic
    // from run_agent_start(). The spawned sidecar is what starts the Hot Graph.
    run_agent_start_hot_graph(&name, &bind, workdir.as_deref()).await?;
    Ok(EXIT_SUCCESS)
}
```

**Important**: Do NOT remove or gate `run_agent_start()`. Keep it as a fallback that is
called when the cognitive loop TOML cannot be loaded (with a warning). This ensures `roko
agent start` never hard-fails due to a missing TOML file.

### 5. Export from `roko-graph/src/lib.rs`

Add the new module:

```rust
pub mod hot;
pub use hot::{HotGraphHandle};
```

Update `GraphPolicy` in the re-exports if needed.

### 6. Write tests

**Unit test** in `crates/roko-graph/tests/hot_graph.rs`:

```rust
/// Hot Graph runs for exactly max_ticks ticks and then stops.
#[tokio::test]
async fn hot_graph_respects_max_ticks() {
    // Build a trivial hot graph with a NoopCell and max_ticks = 3.
    // Start it via engine.start_hot().
    // Wait for it to complete (poll tick_count until >= 3, then assert).
    // Assert: handle.tick_count() == 3 after the graph completes.
}

/// Hot Graph is cancelled when handle.cancel() is called.
#[tokio::test]
async fn hot_graph_cancels_cleanly() {
    // Build a hot graph with no max_ticks (runs forever).
    // Start it, then immediately call handle.cancel().
    // Assert: no panic, flow exits within 100ms.
}
```

**TOML parse test**:

```rust
#[test]
fn cognitive_loop_toml_parses_and_validates() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap()
        .join("examples/graphs/cognitive-loop.toml");
    let graph = roko_graph::load_graph(&path).unwrap();
    assert_eq!(graph.nodes.len(), 7);
    assert!(graph.policy.hot.is_some());
    graph.validate().unwrap();
}
```

## What NOT to Do

- Do NOT implement real `signal-reader`, `relevance-scorer`, `gate-pipeline`, `store-writer`,
  or `event-publisher` cells in this task. Stubs are correct here; real implementations are
  separate tasks that come after Phase 4.
- Do NOT implement condition evaluation for edges (the `condition = "..."` field). Leave it
  as an uninterpreted string — the Engine already stores conditions as `Option<String>` and
  currently ignores them (always-true). Hot Graphs use the same behavior.
- Do NOT change the Engine's existing `start()`/`await_flow()` path. Add `start_hot()` as a
  new method. One-shot graphs and Hot Graphs coexist.
- Do NOT make `roko agent start` fail if the cognitive loop TOML is missing. Fall back to
  the existing procedural loop with a warning.
- Do NOT implement agent-to-agent communication or multi-agent hot graph coordination. This
  task is scoped to a single agent running a single cognitive loop.
- Do NOT add persistence of `HotGraphHandle` to disk. The agent runtime list already tracks
  running agents; the Hot Graph handle lives only in memory.
- Do NOT implement the `FlowSnapshot` resume path for Hot Graphs. If a Hot Graph is killed,
  it restarts from the beginning. Resume is a separate task.
- Do NOT rename or restructure the 7-step cognitive loop nodes. The names (sense, assess,
  compose, act, verify, persist, react) match the canonical roko architecture vocabulary.

## Wire Target

```bash
# Confirm cognitive loop TOML parses correctly
cargo test -p roko-graph -- cognitive_loop_toml_parses_and_validates
# Expected: passes, 7 nodes, hot policy present

# Hot graph unit tests
cargo test -p roko-graph -- hot_graph
# Expected: respects_max_ticks and cancels_cleanly both pass

# Start an agent and confirm it uses the cognitive loop
cargo run -p roko-cli -- agent create --name test-agent --domain coding
cargo run -p roko-cli -- agent start --name test-agent
# Expected: "Starting agent 'test-agent' with cognitive loop (hot graph)"
#           then ticks begin logging: "PassthroughCell 'sense' — 0 input signals (stub)"
#           Agent runs until Ctrl+C
```

## Verification

- [ ] `cargo build --workspace` — compiles clean
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo test -p roko-graph -- cognitive_loop_toml_parses_and_validates` — passes
- [ ] `cargo test -p roko-graph -- hot_graph_respects_max_ticks` — passes
- [ ] `cargo test -p roko-graph -- hot_graph_cancels_cleanly` — passes
- [ ] `cargo run -p roko-cli -- agent start --name test-agent` — uses Hot Graph path (with stub cells)
- [ ] `examples/graphs/cognitive-loop.toml` exists and has exactly 7 nodes in the right order
- [ ] `grep -rn 'HotPolicy\|start_hot\|HotGraphHandle' crates/roko-graph/src/ --include='*.rs' | grep -v target/` — all three exist
- [ ] `grep -rn 'cognitive-loop\|cognitive_loop' crates/roko-cli/ --include='*.rs' | grep -v target/` — wired in agent start
- [ ] `graph.policy.hot` field is parsed from TOML in the cognitive loop file
- [ ] `cargo run -p roko-cli -- agent stop --name test-agent` (in another terminal) stops the Hot Graph cleanly
- [ ] Stub cells registered for all 7 nodes — no "Cell not found" errors when the loop ticks
- [ ] `.roko/GAPS.md` updated with known gaps: stub cells, condition evaluation, resume
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in `hot.rs` or `cognitive-loop.toml`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
