# Task 059: Streaming CLI Output — Real-Time Agent Output via RunOutputSink + SSE Client

```toml
id = 59
title = "Wire streaming CLI output: RunOutputSink for real-time agent lines + SSE client for remote plan progress"
track = "cli-redesign"
wave = "wave-2"
priority = "high"
blocked_by = [6, 56]
touches = [
    "crates/roko-cli/src/runner/output_sink.rs",
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/agent_events.rs",
    "crates/roko-cli/src/commands/util.rs",
    "crates/roko-cli/src/run.rs",
    "crates/roko-cli/Cargo.toml",
]
exclusive_files = []
estimated_minutes = 300
```

## Context

Currently, agent output during plan execution is either:
- **Visible** — via `if stream_to_stderr { eprintln!(...) }` blocks scattered across
  `agent_events.rs` and `event_loop.rs` (being refactored in task 006)
- **Silent** — no output until the entire plan completes

After task 006 wires `RunOutputSink`, the trait-based output path exists. This task builds
on that foundation to provide two streaming capabilities:

1. **Local streaming**: When running `roko do` or `roko plan run`, agent output (code being
   written, tool calls, reasoning) streams to the terminal in real-time as it happens. This
   uses `RunOutputSink` (from task 006) with a new `FormattedStderrSink` that applies
   structured formatting (prefixes, colors, progress indicators).

2. **Remote streaming (SSE client)**: When `roko serve` is running a plan, a CLI client can
   connect to `/api/events` and stream progress. This enables `roko show --follow` for live
   monitoring of remote runs.

The infrastructure exists on the server side: `TuiBridge` emits `DashboardEvent` variants
into `StateHub`, and `StateHub` feeds the SSE endpoint at `/api/events`. What is missing is
the CLI consumer side.

Sources:
- `tmp/solutions/demo-running/CURRENT-STATE.md` — RunOutputSink dead code section, SSE streaming section
- Task 006 — RunOutputSink trait wiring (prerequisite)
- `crates/roko-serve/src/routes/sse.rs` — server-side SSE endpoint

## Background

Read these files:
1. `crates/roko-cli/src/runner/output_sink.rs` — `RunOutputSink` trait, `StderrSink`, `NoopSink`
2. `crates/roko-cli/src/runner/agent_events.rs` — `handle_agent_event()`, `AgentStreamBuffer`
3. `crates/roko-cli/src/runner/event_loop.rs` — TuiBridge usage, the main event loop
4. `crates/roko-serve/src/routes/sse.rs` — SSE endpoint implementation
5. `crates/roko-cli/src/state_hub.rs` — `StateHub` and `DashboardEvent` types
6. `crates/roko-cli/src/runner/tui_bridge.rs` — how events flow from runner to StateHub

Understand the event flow:
```bash
# What events does the TuiBridge emit?
grep -n 'DashboardEvent\|runner_event\|task_started\|task_completed\|agent_output' crates/roko-cli/src/runner/tui_bridge.rs | head -20

# What does the SSE endpoint serve?
grep -n 'DashboardEvent\|event_stream\|sse' crates/roko-serve/src/routes/sse.rs | head -20
```

## What to Change

### 1. Create `FormattedStderrSink` implementing `RunOutputSink`

This replaces the basic `StderrSink` from task 006 with structured formatting.

The sink must format output with context:

```
[plan-id/task-id] ▸ Agent starting: "Add rate limiting middleware"
[plan-id/task-id] │ Writing crates/roko-serve/src/middleware/rate_limit.rs
[plan-id/task-id] │ use std::sync::Arc;
[plan-id/task-id] │ use tokio::sync::Semaphore;
[plan-id/task-id] │ ...
[plan-id/task-id] ▸ Running gate: compile
[plan-id/task-id] ✓ Gate passed: compile (2.3s)
[plan-id/task-id] ▸ Running gate: test
[plan-id/task-id] ✗ Gate failed: test — 2 test failures
```

Features:
- Plan/task prefix on every line for multi-plan disambiguation
- Color: green for pass, red for fail, yellow for in-progress, dim for agent output
- Progress indicator: task X/Y at the top level
- Respects `NO_COLOR` and `--color never`
- Truncation: agent code output longer than 50 lines shows first 10, last 10, "... (30 lines omitted)"

### 2. Wire `FormattedStderrSink` as the default for `roko do` and `roko plan run`

In `run.rs` / the current `cmd_do` implementation in `commands/util.rs`, construct
`FormattedStderrSink` and pass it through the
`RunConfig` into the event loop. When `--quiet` is set, use `NoopSink`. When `--json` is
set, use a future `JsonSink` (stub for now — just use `NoopSink` and document the gap).

### 3. Create `SseStreamClient` for remote progress

Add a simple SSE client that connects to `http://{host}:{port}/api/events` and prints
structured events to stderr using the same formatting as `FormattedStderrSink`.

Implementation:
- Use `reqwest` with streaming response (already a dependency)
- Parse SSE `data:` lines into `DashboardEvent` structs
- Feed parsed events through the same formatting logic as local output
- Handle reconnection on disconnect (exponential backoff, 3 retries)
- Cancel on Ctrl+C

### 4. Add `--follow` flag to `roko show`

In `roko show` (task 058), add a `--follow` flag:

```rust
/// Stream live events from a running roko serve instance.
#[arg(long, short = 'f')]
follow: bool,

/// URL of the roko serve instance for --follow (default: http://localhost:6677).
#[arg(long, default_value = "http://localhost:6677")]
serve_url: String,
```

When `--follow` is active:
- Connect to `{serve_url}/api/events` via SSE
- Print events as they arrive using `FormattedStderrSink`
- Keep running until Ctrl+C or the run completes

### 5. Add `reqwest-eventsource` or equivalent dependency

If `reqwest` alone does not support SSE parsing, add a lightweight SSE parsing crate.
Prefer `eventsource-stream` (small, maintained) or implement SSE parsing inline (the
protocol is trivial: lines starting with `data:`, separated by blank lines).

Do NOT add a heavy dependency. SSE parsing is ~30 lines of code.

## What NOT to Do

- Don't build a full TUI. This is text output to stderr, not a ratatui application.
- Don't buffer all output and print at the end. The entire point is real-time streaming.
- Don't modify the server-side SSE endpoint format. Consume what it already produces.
- Don't add WebSocket support. SSE is sufficient for one-directional progress streaming.
- Don't add `--follow` to every command. Only `roko show --follow` for now.
- Don't implement `JsonSink` beyond a stub. It is a future task.

## Wire Target

```bash
# Local streaming — agent output appears in real-time:
cargo run -p roko-cli -- do "Add a hello world test to roko-core" 2>&1 | head -30
# Should show structured output with [plan/task] prefixes as the agent runs

# Plan run streaming:
cargo run -p roko-cli -- plan run plans/ 2>&1 | head -30
# Should show formatted progress output

# Quiet mode — no output:
cargo run -p roko-cli -- do "test" --quiet 2>/dev/null
echo $?
# Should exit 0/1 with no stderr output

# SSE client (requires roko serve running):
# Terminal 1: cargo run -p roko-cli -- serve
# Terminal 2: cargo run -p roko-cli -- show --follow
# Should print live events from the server
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- do "test" --dry-run 2>&1` — shows formatted output
- [ ] `grep -rn 'FormattedStderrSink' crates/roko-cli/src/ --include='*.rs' | grep -v test` — has non-test callers
- [ ] `grep -rn 'SseStreamClient\|eventsource' crates/roko-cli/src/ --include='*.rs'` — SSE client exists
- [ ] Agent output during `plan run` shows structured prefixes (not raw eprintln)
- [ ] `--quiet` flag suppresses all stderr output
- [ ] Color output respects `NO_COLOR` env var

## Implementation Notes for Later Agent

Current branch facts to verify before editing:
- Task 006 is a hard prerequisite. On the inspected branch, `RunOutputSink` exists in
  `crates/roko-cli/src/runner/output_sink.rs`, but `event_loop.rs` and `agent_events.rs`
  still route output through `RunnerInlineTerminal` and `RunConfig.stream_to_stderr`.
  If task 006 has not removed that bool and threaded a sink through both files, do not
  add a parallel streaming path here.
- `roko plan run` call chain:
  `crates/roko-cli/src/main.rs` `Command::Plan` -> `commands/plan.rs` `PlanCmd::Run`
  -> constructs `runner::RunConfig` -> `runner/event_loop.rs::run` -> receives
  `AgentEvent`s -> `runner/agent_events.rs::handle_agent_event` -> `TuiBridge` ->
  `StateHub` -> `roko-serve` `/api/events`.
- `roko do` call chain is different:
  `main.rs` `Command::Do` -> `commands/util.rs::cmd_do` ->
  `roko_cli::run::run_workflow_engine_report_with_hub` ->
  `run.rs::run_workflow_engine_with_services` -> `WorkflowEngine`. Passing a sink through
  the runner event loop alone will not affect `roko do`; stream `WorkflowEngine`
  `RuntimeEvent`s through the same formatter or explicitly document any remaining gap.
- The current show command is `Command::Show { live, workdir, subject }` in `main.rs`
  and `crates/roko-cli/src/commands/show.rs`, not `commands/show_cmd.rs`. Adding
  `--follow` requires touching those files; the task metadata touch list is incomplete
  unless task 058 lands with a different filename.
- `StateHub`/`StateHubSender` live in `roko-core/src/state_hub.rs` and are re-exported
  through `roko_cli::state_hub`; `DashboardEvent` lives in
  `roko-core/src/dashboard_snapshot.rs` with `#[serde(tag = "type",
  rename_all = "snake_case")]`.
- `crates/roko-serve/src/routes/sse.rs` emits SSE frames with only `id:` and `data:`;
  `data:` is serialized `DashboardEvent` JSON. There is no SSE event name to switch on.

Mechanical implementation order:
1. After task 006, extend the existing sink contract instead of reintroducing
   `stream_to_stderr`. Either rename/replace `StderrSink` with `FormattedStderrSink`
   or make `FormattedStderrSink` a wrapper used anywhere `StderrSink` was selected.
2. Keep `RunnerInlineTerminal` private as a formatter/helper if useful, but ensure
   `event_loop.rs` and `agent_events.rs` call only `RunOutputSink` methods for terminal
   output. `TuiBridge` calls stay separate because they feed the state hub, not stderr.
3. Centralize formatting in one helper that can render both local sink events and
   remote `DashboardEvent`s. Do not implement one formatter in `output_sink.rs` and a
   second incompatible formatter in the SSE client.
4. Respect color via the existing CLI color decision (`Cli.color`) when constructing
   the sink. If the sink is constructed outside `main.rs`, pass a plain `color_enabled:
   bool`; do not parse process args inside the sink. Also honor `NO_COLOR`.
5. Implement agent-output truncation at the formatter boundary: for blocks longer than
   50 lines, show first 10 lines, an omission line, and last 10 lines. Do not truncate
   the persisted `RunState.agent_output`.
6. Add an SSE parser/client in `roko-cli` (for example `runner/sse_stream.rs` or
   `commands/show.rs` local helpers). Use existing `reqwest`; if `Response::bytes_stream`
   is used, add the minimal `stream` feature to the workspace `reqwest` dependency.
   Parse `data:` lines separated by blank lines and ignore keep-alive comments.
7. Add `--follow` and `--serve-url` to the existing `Command::Show` variant and branch
   in `commands/show.rs::cmd_show` before loading local `.roko` state. `--live` should
   continue to open the TUI/dashboard; `--follow` should print remote SSE events to
   stderr and keep running until Ctrl+C or disconnect retry exhaustion.
8. For `roko do`, either pass the active serve `StateHub` into
   `run_workflow_engine_report_with_hub` or add a `WorkflowEventConsumer` that feeds the
   same formatted sink. Current `cmd_do` passes `external_hub: None`, so `roko serve`
   cannot observe it through `/api/events` without additional wiring.

Tests to add or update:
- Unit-test the formatter with a fake event sequence: task start, agent output,
  gate pass, gate fail, task completed, task failed, and long agent output truncation.
- Unit-test color disabled behavior by setting `NO_COLOR=1` or constructing the sink
  with `color_enabled=false`; assert no ANSI escapes.
- Unit-test SSE parsing for single-line `data:`, multi-line `data:`, blank-line
  dispatch, comments/keepalive, invalid JSON, and replay `id:` preservation if tracked.
- Add a non-network test that maps representative `DashboardEvent` variants to
  formatted lines. Avoid requiring a live `roko serve` in unit tests.
- Update plan-run tests/build fixtures for the post-task-006 `RunConfig` shape.

Additional verification commands:
```bash
rg 'stream_to_stderr|RunnerInlineTerminal::new|inline\.' \
  crates/roko-cli/src/runner/event_loop.rs crates/roko-cli/src/runner/agent_events.rs
# Expected after task 006 + this task: no runtime output calls outside sink internals.

rg 'FormattedStderrSink|RunOutputSink' crates/roko-cli/src -g '*.rs'
# Expected: constructor callsites in CLI command paths plus calls from event loop/agent event handling.

cargo run -p roko-cli -- show --follow --serve-url http://localhost:6677
# Expected with `roko serve` running: printed DashboardEvent progress from /api/events.
```

What not to do:
- Do not make `roko do` appear "done" by only formatting the final
  `WorkflowRunReport`; the requirement is live progress while the run is active.
- Do not add WebSocket support or change the server SSE schema.
- Do not make `--dry-run` the only wire target for streaming; dry-run may only exercise
  preview output and can miss the live agent/gate path.
- Do not add a second `show` command module or rename the existing `commands/show.rs`
  unless task 058 explicitly did so.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
