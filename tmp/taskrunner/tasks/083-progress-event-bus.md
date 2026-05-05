# Task 083: RuntimeEvent Progress Projection

```toml
id = 83
title = "Project progress through RuntimeEvent/EventConsumer; do not create ProgressEvent bus"
track = "wiring"
wave = "wave-2"
priority = "high"
blocked_by = [82]
touches = [
    "crates/roko-core/src/runtime_event.rs",
    "crates/roko-core/src/foundation.rs",
    "crates/roko-agent/src/runtime_events.rs",
    "crates/roko-agent/src/tool_loop/mod.rs",
    "crates/roko-agent/src/tool_loop/agent_wrapper.rs",
    "crates/roko-cli/src/dispatch_v2.rs",
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-runtime/src/http_event_sink.rs",
    "crates/roko-serve/src/adapters.rs",
    "crates/roko-serve/src/routes/event_ingest.rs",
    "crates/roko-serve/src/routes/sse.rs",
    "crates/roko-acp/src/event_forward.rs",
]
exclusive_files = []
estimated_minutes = 360
```

## Context

Today there is no unified progress feedback in roko. The audit catalogues this in multiple
sections:
- **S22.1**: `OpenAiCompatLlmBackend` blocks silently for minutes during LLM calls.
- **S22.5**: `ToolLoop::run_inner()` has an `on_turn` callback slot but it is never wired for
  `prd draft new` or `prd plan` invocations.
- **S25.3**: "Progress event bus" — no unified progress for CLI/TUI/demo/bench.

After task 082 makes streaming the primary backend path, `StreamEvent` values flow from the
provider. But `StreamEvent` is internal to `roko-agent` and carries low-level provider data.
Higher-level consumers (CLI stderr, TUI dashboard, demo app SSE, bench metrics) need a richer
vocabulary: task started, gate began, agent finished, tool called.

The original task proposed a new `ProgressEvent`/`ProgressSink` bus. That design is superseded:
the implementation must project progress onto the existing `RuntimeEvent` vocabulary and
`EventConsumer` delivery path so CLI/TUI/SSE/bench observers share one canonical event surface.
It also fixes S22.5 by wiring the existing ToolLoop/runner progress callbacks into that path.

This is Phase 4.1 from `tmp/redesign-plan.md`.

## Current Branch Status - 2026-05-05

Status: **needs rescope before implementation**.

The architecture batch chose `RuntimeEvent` + `EventConsumer` as the canonical event path.
Do not introduce a parallel `ProgressEvent` bus unless the task is explicitly redesigned to
be a projection over `RuntimeEvent`.

Relevant current implementation:
- `POST /api/events/ingest` and `/api/events/ingest/batch` exist in
  `crates/roko-serve/src/routes/event_ingest.rs`.
- `roko-runtime::HttpEventSink` exists and posts `RuntimeEvent` batches to the ingest route.
- The v2 runner auto-creates the HTTP sink from `ROKO_SERVE_URL`.
- ACP forwarding maps `CognitiveEvent` to `RuntimeEvent` through
  `crates/roko-acp/src/event_forward.rs`.

Remaining cleanup belongs in follow-up tasks:
- Add route-level ingest tests, HttpEventSink batching/auth tests, and ACP forwarding E2E tests.
- Finish `RuntimeEvent` variant coverage and SseAdapter match arms instead of adding a new
  event enum.
- Resolve the StateHub/EventBus crate-boundary cleanup tracked by task 104.

## Rescoped Spec: RuntimeEvent Progress Projection

This section supersedes the legacy "What to Change" instructions below. Do not create
`crates/roko-core/src/progress.rs`, `ProgressEvent`, or `ProgressSink`; those would create a
parallel event bus. The canonical bus is now:

`roko_core::runtime_event::RuntimeEvent` -> `roko_core::foundation::EventConsumer` ->
`roko_runtime::{event_bus,HttpEventSink,JsonlLogger}` -> `roko_serve::{SseAdapter,StateHub}`.

Current code paths to read before editing:
- `crates/roko-core/src/runtime_event.rs` — canonical event enum, `run_id()`, `kind()`, serde
  contract tests. Add any progress vocabulary here as additive variants.
- `crates/roko-core/src/foundation.rs` — `EventConsumer` trait. Reuse it; do not add a second
  sink trait.
- `crates/roko-agent/src/runtime_events.rs` — provider-neutral streaming events from agents.
- `crates/roko-agent/src/tool_loop/mod.rs` — `on_turn` callback and streaming call branch in
  `run_inner()`.
- `crates/roko-cli/src/dispatch_v2.rs` — converts `StreamChunk` to `AgentRuntimeEvent` while
  driving `Agent::run_streaming()`.
- `crates/roko-cli/src/runner/event_loop.rs` — `handle_runner_event()` forwards
  `RunnerEvent` to `RuntimeEvent` through `HttpEventSink`.
- `crates/roko-runtime/src/http_event_sink.rs` — non-blocking bounded HTTP event batching.
- `crates/roko-serve/src/routes/event_ingest.rs` — `POST /api/events/ingest[/batch]`.
- `crates/roko-serve/src/adapters.rs` — `SseAdapter::to_sse_event()` must match every
  progress-relevant `RuntimeEvent` variant.
- `crates/roko-acp/src/event_forward.rs` — ACP `CognitiveEvent` -> `RuntimeEvent` mapping.

Mechanical implementation steps:
1. Add only missing `RuntimeEvent` variants needed by task 082 streaming output. Prefer
   additive variants such as `InferenceFirstToken { run_id, request_id, model, agent_id, ttft_ms }`
   if TTFT needs first-class routing. Use existing `AgentOutput`, `AgentTrace`, `GateStarted`,
   `GatePassed`, and `GateFailed` where they already fit.
2. Update `RuntimeEvent::run_id()`, `RuntimeEvent::kind()`, `Display` tests, and
   `roko-runtime` round-trip contract tests for every added variant.
3. Add an optional `Arc<dyn EventConsumer>` or equivalent non-blocking consumer hook to the
   ToolLoop/ToolLoopAgent path. Default must be `None`/no-op so existing `create_agent_for_model`
   callsites remain source-compatible.
4. In `ToolLoop::run_inner()`, when consuming the streaming-first events from task 082, emit:
   `InferenceStarted` before the backend stream, `InferenceFirstToken` on the first text delta
   if that variant is added, `AgentOutput` for visible text deltas, `AgentTrace` after each
   turn, `AgentCompleted` or `AgentFailed` at termination, and gate events around gate dispatch
   only in the runner/gate dispatch path.
5. In CLI paths, do not add ad-hoc `eprintln!` scattered through providers. Either render from
   `AgentRuntimeEvent` where that already exists (`chat_session::render_stream_event`) or consume
   `RuntimeEvent` at the command boundary. Text deltas may print inline; lifecycle events should
   be one-line stderr status messages.
6. In serve, update `SseAdapter::to_sse_event()` and any StateHub bridge/projection match arms so
   the new RuntimeEvent variants reach `/api/workflow/events` and, where relevant, dashboard SSE.
7. In ACP, update `event_forward.rs` only for mappings that cannot already be represented by
   `AgentOutput`, `GateStarted`, `GatePassed`, `GateFailed`, or `FeedbackRecorded`.
8. Add tests before manual CLI smoke tests: `runtime_event` serde round-trip, `SseAdapter`
   conversion for each added variant, `HttpEventSink` batching/auth or route-level ingest tests,
   and a ToolLoop fake backend test that records emitted `RuntimeEvent`s.

Expected observable behavior:
- `ROKO_SERVE_URL=http://127.0.0.1:<port>` causes `roko-cli` runner events to POST to
  `/api/events/ingest/batch` without blocking the runner.
- SSE clients on `GET /api/workflow/events` receive the same canonical RuntimeEvent vocabulary
  that JSONL logging receives.
- CLI users see streaming text/progress from the command boundary, not provider-internal
  print statements.
- No `ProgressEvent` or `ProgressSink` symbols exist after the change.

## What NOT to Do

- Do not create `crates/roko-core/src/progress.rs`.
- Do not introduce `ProgressEvent`, `ProgressSink`, `StderrSink`, or `BroadcastSink`.
- Do not bypass `RuntimeEvent`, `EventConsumer`, `HttpEventSink`, `JsonlLogger`, or the
  existing SSE adapter when wiring progress.
- Do not print provider/tool-loop progress directly from provider internals; surface it at the
  CLI or consumer boundary.
- Do not treat the legacy sections below as implementation instructions. They are retained only
  to explain why this task was rescoped.

## Wire Target

```bash
ROKO_SERVE_URL=http://127.0.0.1:6677 cargo run -p roko-cli -- run "say hello"
```

Expected observable behavior: with `roko serve` running, the CLI emits canonical
`RuntimeEvent` records through `HttpEventSink`, `/api/workflow/events` receives those events,
and no provider-internal print path is required for progress output. Without a serve process,
the command must still run without blocking or panicking on event delivery failure.

## Verification

```bash
rg -n "ProgressEvent|ProgressSink|progress.rs" crates
# Expected in crates/: no matches.

cargo test -p roko-core runtime_event
cargo test -p roko-runtime http_event_sink jsonl_logger
cargo test -p roko-serve event_ingest sse
cargo test -p roko-agent tool_loop -- --nocapture

ROKO_SERVE_URL=http://127.0.0.1:3000 cargo run -p roko-cli -- run "say hello"
# Expected: command still runs; with a local serve process, runtime events appear on SSE.
```

## Legacy Background (Superseded)

The background below describes the original `ProgressEvent` design and is retained only to explain
why this task was rescoped. Do not use it as implementation guidance; use the current-tree file
list and RuntimeEvent plan above.

Read these files before starting:

1. `crates/roko-core/src/` — scan for existing progress/event types. There is no
   `progress.rs` yet; it must be created. Check `lib.rs` for existing pub re-exports.
2. `crates/roko-agent/src/tool_loop/mod.rs` — the `ToolLoop::run_inner()` loop. Find
   the `on_turn` callback slot and understand its current type signature.
3. `crates/roko-agent/src/tool_loop/agent_wrapper.rs` — `ToolLoopAgent::run()` and how
   it calls `ToolLoop`. This is the connection point to wire progress events out.
4. `crates/roko-cli/src/runner/event_loop.rs` — the CLI-side event consumer. Understand
   how it currently handles output and where to add progress printing.
5. `crates/roko-serve/src/routes/sse.rs` — existing SSE infrastructure. `DashboardEvent`
   is broadcast via `state.state_hub`. `ProgressEvent` should be convertible to
   `DashboardEvent` and forwarded through the same channel.
6. `crates/roko-serve/src/state.rs` — `AppState` and `StateHub`. The `state_hub` is the
   shared event bus; understand how to publish to it.
7. `tmp/redesign-plan.md` Phase 4.1 — the full `ProgressEvent` enum definition.
8. `tmp/infrastructure-audit.md` sections S22.5, S25.3 — the problems being fixed.

## Legacy What to Change (Superseded)

The section below is retained for historical context from the original task. Do not implement it
unless the architecture decision is reversed by a new task. The active implementation contract is
the `RuntimeEvent` rescope above.

### 1. Create `crates/roko-core/src/progress.rs`

```rust
//! Progress event vocabulary for roko-wide observability.
//!
//! `ProgressEvent` is the single language for reporting what roko is doing.
//! Every component emits `ProgressEvent`s through a `ProgressSink`. Consumers
//! (CLI stderr, TUI, SSE, bench) register their own sinks and receive all events.
//!
//! Design rules:
//! - Events are cheap to clone (no large payloads — use summaries and counts).
//! - Events are additive — adding new variants is not a breaking change.
//! - No business logic in this module. Events describe facts, not decisions.

use std::time::Duration;
use serde::{Deserialize, Serialize};

/// All progress events emitted during roko operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    // ─── Agent lifecycle ─────────────────────────────────────────────────

    /// An agent is being created (before the LLM call starts).
    AgentCreating {
        model: String,
        provider: String,
        task_id: Option<String>,
    },

    /// The LLM stream started and the first token arrived.
    /// `ttft_ms` is the time-to-first-token in milliseconds.
    AgentStreamStarted {
        model: String,
        ttft_ms: u64,
    },

    /// Incremental text from the LLM stream.
    /// Emitted for every `StreamEvent::TextDelta` from task 082.
    AgentTextDelta {
        text: String,
    },

    /// The LLM called a tool. Emitted before the tool executes.
    AgentToolCallStarted {
        tool: String,
        iteration: usize,
        max_iterations: usize,
    },

    /// A tool call finished executing.
    AgentToolCallComplete {
        tool: String,
        duration_ms: u64,
        /// Whether the tool succeeded (did not return an error result).
        success: bool,
    },

    /// One turn of the tool loop finished (LLM responded + tools called).
    AgentTurnComplete {
        iteration: usize,
        /// Approximate token count for this turn. May be 0 for Claude CLI
        /// until token parsing is fixed (see S23.6).
        tokens_used: u32,
    },

    /// The agent (tool loop) finished all turns successfully.
    AgentComplete {
        total_tokens: u32,
        duration_ms: u64,
    },

    /// The agent failed with an error.
    AgentFailed {
        error: String,
        /// Number of iterations completed before failure.
        iterations_completed: usize,
    },

    // ─── Task pipeline ───────────────────────────────────────────────────

    /// A plan task started executing.
    TaskStarted {
        task_id: String,
        task_title: String,
        role: String,
    },

    /// A plan task's gate pipeline started.
    TaskGateStarted {
        task_id: String,
        rung: String,
    },

    /// A single gate rung completed.
    TaskGateComplete {
        task_id: String,
        rung: String,
        passed: bool,
        duration_ms: u64,
    },

    /// A plan task finished (gate pipeline complete).
    TaskComplete {
        task_id: String,
        passed: bool,
        duration_ms: u64,
    },

    // ─── Pipeline-level events ────────────────────────────────────────────

    /// The overall plan pipeline entered a new phase.
    PipelinePhase {
        phase: String,
        /// "started", "complete", "failed"
        status: String,
    },
}

/// A sink that receives progress events.
///
/// Implementations include:
/// - `StderrSink` — prints human-readable lines to stderr
/// - `BroadcastSink` — forwards to a `tokio::sync::broadcast` channel (SSE)
/// - `NoopSink` — discards all events (for tests)
/// - `MultiSink` — fans out to multiple sinks
pub trait ProgressSink: Send + Sync + 'static {
    /// Called for each progress event. Must not block.
    /// Implementations should use internal channels and return immediately.
    fn send(&self, event: ProgressEvent);
}

/// A `ProgressSink` that discards all events.
pub struct NoopSink;

impl ProgressSink for NoopSink {
    fn send(&self, _event: ProgressEvent) {}
}

/// A `ProgressSink` that prints human-readable progress to stderr.
pub struct StderrSink {
    /// Optional prefix, e.g. "[agent]" or "[gate]".
    pub prefix: String,
}

impl ProgressSink for StderrSink {
    fn send(&self, event: ProgressEvent) {
        let line = format_progress_event(&self.prefix, &event);
        if let Some(line) = line {
            eprintln!("{line}");
        }
    }
}

/// Formats a `ProgressEvent` into a human-readable string for CLI output.
/// Returns `None` for events that should not be printed to stderr
/// (e.g., `AgentTextDelta` is printed inline, not as a log line).
pub fn format_progress_event(prefix: &str, event: &ProgressEvent) -> Option<String> {
    let p = if prefix.is_empty() { String::new() } else { format!("[{prefix}] ") };
    match event {
        ProgressEvent::AgentCreating { model, provider, .. } =>
            Some(format!("{p}creating agent: {model} via {provider}")),
        ProgressEvent::AgentStreamStarted { model, ttft_ms } =>
            Some(format!("{p}{model}: first token in {ttft_ms}ms")),
        ProgressEvent::AgentTextDelta { .. } => None, // printed inline
        ProgressEvent::AgentToolCallStarted { tool, iteration, max_iterations } =>
            Some(format!("{p}turn {iteration}/{max_iterations}: calling {tool}...")),
        ProgressEvent::AgentToolCallComplete { tool, duration_ms, success } => {
            let status = if *success { "ok" } else { "err" };
            Some(format!("{p}{tool}: {status} ({duration_ms}ms)"))
        }
        ProgressEvent::AgentTurnComplete { iteration, tokens_used } =>
            Some(format!("{p}turn {iteration} complete ({tokens_used} tokens)")),
        ProgressEvent::AgentComplete { total_tokens, duration_ms } =>
            Some(format!("{p}agent done: {total_tokens} tokens in {duration_ms}ms")),
        ProgressEvent::AgentFailed { error, .. } =>
            Some(format!("{p}agent failed: {error}")),
        ProgressEvent::TaskStarted { task_id, task_title, .. } =>
            Some(format!("{p}task {task_id}: {task_title}")),
        ProgressEvent::TaskGateStarted { rung, .. } =>
            Some(format!("{p}gate: {rung}...")),
        ProgressEvent::TaskGateComplete { rung, passed, duration_ms, .. } => {
            let result = if *passed { "PASS" } else { "FAIL" };
            Some(format!("{p}gate {rung}: {result} ({duration_ms}ms)"))
        }
        ProgressEvent::TaskComplete { task_id, passed, .. } => {
            let result = if *passed { "PASS" } else { "FAIL" };
            Some(format!("{p}task {task_id}: {result}"))
        }
        ProgressEvent::PipelinePhase { phase, status } =>
            Some(format!("{p}phase: {phase} [{status}]")),
    }
}

/// A `ProgressSink` that fans out to multiple inner sinks.
pub struct MultiSink {
    pub sinks: Vec<Box<dyn ProgressSink>>,
}

impl ProgressSink for MultiSink {
    fn send(&self, event: ProgressEvent) {
        for sink in &self.sinks {
            sink.send(event.clone());
        }
    }
}
```

### 2. Export from `crates/roko-core/src/lib.rs`

```rust
// Add to lib.rs:
pub mod progress;
pub use progress::{
    ProgressEvent, ProgressSink, NoopSink, StderrSink, MultiSink, format_progress_event,
};
```

Check that `progress` does not conflict with any existing export.

### 3. Add `ProgressSink` to `ToolLoop`

In `crates/roko-agent/src/tool_loop/mod.rs`, the `ToolLoop` struct needs a sink field:

```rust
pub struct ToolLoop {
    // ... existing fields ...
    /// Progress sink — receives events for every iteration.
    /// Defaults to NoopSink when not provided.
    pub progress: Arc<dyn roko_core::ProgressSink>,
}
```

In `ToolLoop::run_inner()`, emit events at key points:

```rust
// Before the LLM call:
self.progress.send(ProgressEvent::AgentToolCallStarted {
    tool: "turn".into(),
    iteration,
    max_iterations: self.context.max_iterations,
});

// After receiving the StreamEvent::TextDelta (via send_turn from task 082):
// The default send_turn collects the stream — for text delta printing, callers
// should use stream_turn() directly or register a streaming callback on TurnConfig.
// For now, emit AgentTurnComplete after each successful send_turn:
self.progress.send(ProgressEvent::AgentTurnComplete {
    iteration,
    tokens_used: response.usage.total_tokens(),
});

// After tool dispatch for each tool call:
self.progress.send(ProgressEvent::AgentToolCallComplete {
    tool: call.name.clone(),
    duration_ms: tool_start.elapsed().as_millis() as u64,
    success: !result.is_error,
});
```

**Note**: `AgentTextDelta` events require streaming to reach the sink. The `send_turn`
default wrapper (from task 082) collects the stream internally. To emit `AgentTextDelta`
live, `ToolLoop` must call `stream_turn()` directly and consume events as they arrive.
Implement this: replace `self.backend.send_turn(...)` with `stream_turn()` in the tool loop
body. Process `TextDelta` events by forwarding them to `self.progress`.

### 4. Wire `ProgressSink` in `ToolLoopAgent`

In `crates/roko-agent/src/tool_loop/agent_wrapper.rs`, `ToolLoopAgent` constructs a
`ToolLoop`. Pass a `ProgressSink` to it:

```rust
impl ToolLoopAgent {
    pub fn new(
        backend: Arc<dyn LlmBackend>,
        dispatcher: Arc<dyn ToolDispatcher>,
        progress: Arc<dyn roko_core::ProgressSink>,
        // ... other args ...
    ) -> Self {
        // ...
    }
}
```

The CLI path (`create_agent_for_model`) should pass `NoopSink` by default. Callers that
want progress feedback upgrade to `StderrSink` or `MultiSink` at the call site.

### 5. Wire `StderrSink` in CLI commands

In `crates/roko-cli/src/runner/event_loop.rs` (and `commands/prd.rs`, `commands/plan.rs`),
pass a `StderrSink` when constructing agents:

```rust
// In prd.rs dispatch_prd_agent() or equivalent:
let sink = Arc::new(roko_core::StderrSink { prefix: "prd".into() });
let agent = create_agent_for_model(&config, model_key, options, sink)?;
```

This fixes S22.5 — the `on_turn` callback that was never wired. Replace the old callback
slot with the new `ProgressSink`.

For `prd draft new` and `prd plan`, the user should now see:
```
[prd] creating agent: claude-opus-4-6 via claude_cli
[prd] claude-opus-4-6: first token in 1823ms
[prd] turn 1/25: calling read_file...
[prd] read_file: ok (12ms)
[prd] turn 1 complete (1847 tokens)
```

### 6. Wire `BroadcastSink` into the SSE route

Add a `BroadcastSink` type in `crates/roko-serve/src/routes/sse.rs`:

```rust
use roko_core::{ProgressEvent, ProgressSink};
use crate::state::DashboardEvent;

/// A ProgressSink that converts ProgressEvents to DashboardEvents and
/// forwards them to the SSE broadcast channel via StateHub.
pub struct BroadcastSink {
    pub hub: Arc<crate::state::StateHub>,
}

impl ProgressSink for BroadcastSink {
    fn send(&self, event: ProgressEvent) {
        let dashboard_event = DashboardEvent::Progress(event);
        self.hub.publish(dashboard_event);
    }
}
```

Then when `roko serve` dispatches an agent (via `POST /api/run` or similar), use a
`MultiSink` that combines `StderrSink` (for server-side logging) and `BroadcastSink`
(for SSE clients):

```rust
let sink = Arc::new(MultiSink {
    sinks: vec![
        Box::new(StderrSink { prefix: "serve".into() }),
        Box::new(BroadcastSink { hub: state.state_hub.clone() }),
    ],
});
```

**Important**: Check `DashboardEvent` in `crates/roko-serve/src/event_bus.rs` or similar
to see if a `Progress` variant exists. If it does not, add:
```rust
// In DashboardEvent enum:
Progress(roko_core::ProgressEvent),
```

### 7. Gate dispatch progress events

In the gate pipeline dispatch (called from `orchestrate.rs` or `runner/gate_dispatch.rs`),
emit `TaskGateStarted` before each rung and `TaskGateComplete` after:

```rust
// Before rung:
sink.send(ProgressEvent::TaskGateStarted {
    task_id: task_id.to_string(),
    rung: rung_name.to_string(),
});

let gate_start = std::time::Instant::now();
let verdict = run_gate_rung(rung, payload).await;
let duration_ms = gate_start.elapsed().as_millis() as u64;

// After rung:
sink.send(ProgressEvent::TaskGateComplete {
    task_id: task_id.to_string(),
    rung: rung_name.to_string(),
    passed: verdict.is_pass(),
    duration_ms,
});
```

## Legacy What NOT to Do (Superseded)

The original anti-pattern list below assumes a `ProgressEvent`/`ProgressSink` design. The active
anti-pattern is stricter: do not create those symbols anywhere in `crates/`; extend/project
`RuntimeEvent` through `EventConsumer` instead.

- Do NOT add `ProgressEvent` to `roko-agent`. It belongs in `roko-core` so every crate can
  use it without depending on `roko-agent`. The sink implementations that live in roko-serve
  or roko-cli import from `roko-core`.
- Do NOT use `tokio::sync::broadcast` directly in `ProgressSink::send()`. The `send` method
  is non-async for a reason — it must not block the caller. Sinks that forward to async
  channels should use `try_send` (for bounded) or maintain an internal `UnboundedSender`
  (if bounded is infeasible). Log a warning and drop the event if the channel is full —
  do NOT block.
- Do NOT include large payloads in `ProgressEvent` variants. The `AgentTextDelta.text`
  field is the largest acceptable payload (a streaming text chunk). Never include full file
  contents, full prompts, or full agent outputs in event variants.
- Do NOT add `ProgressSink` as a required parameter to `create_agent_for_model`. The
  existing API must continue to work with `NoopSink` as a default. Make it optional or
  use a `with_progress()` builder method.
- Do NOT implement TUI-specific event handling in this task. The TUI (ratatui, F1-F7 tabs)
  receives `DashboardEvent` already via the state hub. As long as `DashboardEvent::Progress`
  is added and the `BroadcastSink` is wired, the TUI can handle it in a follow-up task.
- Do NOT remove the existing `on_turn` callback slot from `ToolLoop` until all callers have
  been migrated to `ProgressSink`. Deprecate it first, remove in the next wave.

## Legacy Wire Target (Superseded)

The wire target is the `prd draft new` command. After this task, running:

```bash
cargo run -p roko-cli -- prd draft new "test-prd"
```

should print live progress to stderr without requiring `RUST_LOG`:

```
[prd] creating agent: claude-opus-4-6 via claude_cli
[prd] claude-opus-4-6: first token in 1823ms
[prd] turn 1/25: calling read_file...
[prd] read_file: ok (12ms)
[prd] turn 1 complete (1847 tokens)
...
[prd] agent done: 12340 tokens in 47200ms
```

And for `roko serve`, `GET /api/events` SSE clients should receive `Progress` events
as agents run.

## Legacy Verification (Superseded, Do Not Use)

Do not run this checklist as acceptance criteria. The active verification commands are in
`Rescoped Spec: RuntimeEvent Progress Projection`.

- [ ] `cargo build --workspace` — all crates compile
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `crates/roko-core/src/progress.rs` exists and is exported from `lib.rs`
- [ ] `grep -n 'ProgressEvent\|ProgressSink' crates/roko-core/src/lib.rs` — both exported
- [ ] `grep -n 'ProgressSink\|progress' crates/roko-agent/src/tool_loop/mod.rs` — sink field
  present on ToolLoop
- [ ] `grep -n 'ProgressSink\|progress' crates/roko-agent/src/tool_loop/agent_wrapper.rs`
  — sink passed to ToolLoop
- [ ] `grep -n 'StderrSink' crates/roko-cli/src/runner/event_loop.rs` — wired in CLI
- [ ] `grep -n 'BroadcastSink' crates/roko-serve/src/routes/sse.rs` — wired in SSE
- [ ] `grep -n 'DashboardEvent::Progress' crates/roko-serve/src/` — variant added
- [ ] Running `cargo run -p roko-cli -- prd draft new test-082` shows progress lines on
  stderr during agent execution (manual smoke test)
- [ ] `cargo test -p roko-core -- progress` — unit tests for `format_progress_event` pass
- [ ] `cargo test -p roko-agent -- tool_loop_progress` — integration test that verifies
  `ToolLoop` emits expected events to a `MultiSink`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
