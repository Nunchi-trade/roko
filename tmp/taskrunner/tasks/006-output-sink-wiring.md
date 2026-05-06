# Task 006: Wire RunOutputSink into Entire Event Loop (Both Files)

```toml
id = 6
title = "Wire RunOutputSink trait into event_loop.rs AND agent_events.rs — replace ALL inline output"
track = "runner-hardening"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-cli/src/runner/output_sink.rs",
    "crates/roko-cli/src/runner/agent_events.rs",
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/inline_output.rs",
    "crates/roko-cli/src/runner/mod.rs",
    "crates/roko-cli/src/runner/types.rs",
    "crates/roko-cli/src/commands/plan.rs",
    "crates/roko-cli/src/serve_runtime.rs",
]
exclusive_files = ["crates/roko-cli/src/runner/output_sink.rs", "crates/roko-cli/src/runner/agent_events.rs"]
estimated_minutes = 120
```

## Context

`RunOutputSink` trait exists with `StderrSink` + `NoopSink`, but current runtime code still has
ZERO callers. The older audit described direct `if stream_to_stderr { eprintln!(...) }` blocks; the
current branch moved most runner progress behind `RunnerInlineTerminal`. That is still a parallel
output path because `event_loop.rs` constructs `RunnerInlineTerminal::new(config.stream_to_stderr)`
and passes it into `handle_agent_event()`.

The task must cover BOTH runtime files or the abstraction is half-applied:
- `agent_events.rs` — `handle_agent_event()` receives `&mut RunnerInlineTerminal` plus
  `&mut AgentStreamBuffer` and calls `inline.agent_*`, `inline.tool_*`, and `inline.token_usage()`.
- `event_loop.rs` — owns the inline renderer and calls `inline.warm_cache_*`,
  `inline.gate_completed()`, `inline.diff_block()`, `inline.gate_retry()`, `inline.task_done()`,
  and `inline.task_failed()`.
- `types.rs`, `commands/plan.rs`, and `serve_runtime.rs` still carry or construct the
  `stream_to_stderr` bool.

**Important**: `agent_events.rs` also has `AgentStreamBuffer` (flush-last-N-lines logic) that
doesn't map cleanly to `RunOutputSink.agent_line()`. The trait may need a buffered text mode
or `AgentStreamBuffer` should be moved into `StderrSink` as internal state.

Sources:
- `tmp/v2-refactoring/10-DEAD-CODE-AUDIT.md` — RunOutputSink (WIRE NOW)
- Audit finding: 17+ output sites across both files. Treat the audit line numbers as historical;
  the current branch facts below are authoritative.

## Background

Read these files:
1. `crates/roko-cli/src/runner/output_sink.rs` — the trait and implementations
2. `crates/roko-cli/src/runner/agent_events.rs` — `handle_agent_event()`,
   `RunnerInlineTerminal` calls, and `AgentStreamBuffer`
3. `crates/roko-cli/src/runner/event_loop.rs` — `RunnerInlineTerminal::new()`, `inline.*`
   calls, and the call into `handle_agent_event()`
4. `crates/roko-cli/src/runner/types.rs`, `crates/roko-cli/src/commands/plan.rs`, and
   `crates/roko-cli/src/serve_runtime.rs` — `RunConfig.stream_to_stderr` construction
5. Find ALL output sites:
   ```bash
   rg 'RunnerInlineTerminal|inline\.|stream_to_stderr|RunOutputSink|StderrSink|NoopSink|eprintln!' \
     crates/roko-cli/src/runner crates/roko-cli/src/commands/plan.rs crates/roko-cli/src/serve_runtime.rs
   ```

Current branch facts to verify before editing:
- `agent_events.rs` no longer has direct `if stream_to_stderr { eprintln!(...) }` blocks. It now
  receives `&mut RunnerInlineTerminal` and calls methods such as `agent_started()`,
  `agent_text()`, `tool_call_started()`, `tool_output()`, `token_usage()`,
  `agent_turn_completed()`, and `agent_error()`.
- `event_loop.rs` still constructs `RunnerInlineTerminal::new(config.stream_to_stderr)`, logs
  the `stream_to_stderr` flag, and calls inline methods for warm-cache, gate, diff, retry, task
  done, and task failed output.
- `RunConfig.stream_to_stderr` still exists in `runner/types.rs` and is set in
  `commands/plan.rs` and `serve_runtime.rs`.
- `RunOutputSink`, `StderrSink`, and `NoopSink` still have no runtime callers. Grep:
  `rg 'RunOutputSink|StderrSink|NoopSink' crates/roko-cli/src/runner -g '*.rs'`.
- `RunnerInlineTerminal` is an output renderer, not a sink boundary. The implementation may keep
  it as an internal helper, but callers should talk to `RunOutputSink`.

## What to Change

1. **Choose the buffering design before changing callsites**:
   - Preferred current-branch design: move `AgentStreamBuffer` behavior into `StderrSink` as
     internal `Mutex<String>` state and expose trait methods like `agent_text_delta()` and
     `flush_agent_text(max_lines, max_chars)`. This works with the existing `&self` trait methods
     and `Arc<dyn RunOutputSink + Send + Sync>`.
   - Acceptable alternative: change trait methods to `&mut self` and keep a local buffer beside
     the sink, but then update every callsite consistently. Do not mix both patterns.
   - Document the choice in the Status Log.
2. **Make `RunConfig` carry the sink, not a bool**:
   - In `runner/types.rs`, replace `stream_to_stderr: bool` with
     `output_sink: Arc<dyn RunOutputSink>`.
   - `RunConfig::default()` and serve/job paths use `Arc::new(NoopSink)`.
   - `commands/plan.rs` uses `Arc::new(StderrSink::new())` when
     `!approval && !cli.quiet && !cli.json`; otherwise `NoopSink`.
   - Update `Debug` for `RunConfig` to print only the sink kind/name, not the trait object.
3. **Make `RunOutputSink` cover every current inline output site**:
   - Keep existing semantic methods (`task_started`, `agent_line`, `tool_call`, `tool_output`,
     `token_usage`, `gate_result`, `task_completed`, `task_failed`, `plan_summary`).
   - Add methods for currently unrepresented event-loop output if needed:
     `warm_cache_started`, `warm_cache_completed`, `gate_retry`, `diff_block`, and any task
     progress method currently only on `RunnerInlineTerminal`.
   - `StderrSink` may delegate to `RunnerInlineTerminal` internally to preserve current rich
     formatting; `NoopSink` must ignore all events.
4. **Thread the sink through runtime code**:
   - In `event_loop.rs`, remove local `RunnerInlineTerminal::new(config.stream_to_stderr)`.
     Use `let sink = Arc::clone(&config.output_sink)` and pass `sink.as_ref()` to helpers.
   - In `agent_events.rs`, change `handle_agent_event()` to accept `&dyn RunOutputSink` instead of
     `&mut RunnerInlineTerminal` and replace inline calls with sink methods.
   - In `event_loop.rs`, replace all `inline.*` calls with equivalent `sink.*` calls.
   - Remove `AgentStreamBuffer` from `event_loop.rs` if it moved into `StderrSink`; otherwise keep
     it only as sink-owned/internal buffering, not as a separate output API.
5. **Remove the old flag completely**:
   - Delete `stream_to_stderr` from `RunConfig`, constructors, debug output, logs, and command
     construction.
   - Delete `RunnerInlineTerminal::new(stream_to_stderr: bool)` or make it private to
     `StderrSink`; no runtime caller should pass a bool.
6. **Tests to add/update**:
   - Unit test `handle_agent_event()` with a recording test sink: `Started`, `MessageDelta`,
     `ToolCall`, `TokenUsage`, `TurnCompleted`, and `Error` should call expected sink methods.
   - Unit test `NoopSink` can be used in `RunConfig::default()` without panics.
   - Update any compile failures from `RunConfig` construction in tests and serve runtime.

## What NOT to Do

- Don't skip `event_loop.rs` — the trait must cover ALL output sites.
- Don't leave the `stream_to_stderr` bool alive (it's replaced by sink selection).
- Don't add new output formats (JSON sink, etc.) — just wire the existing ones.
- Don't leave `RunnerInlineTerminal` as a parallel public output path. It can be an implementation
  detail of `StderrSink`, but `event_loop.rs` and `agent_events.rs` should depend on
  `RunOutputSink`.
- Don't leave buffering split between `AgentStreamBuffer` and `StderrSink`; choose one owner.
- Don't remove TUI/state updates from `agent_events.rs`. `TuiBridge` updates are not output-sink
  rendering and should remain.

## Wire Target

```bash
# Run a plan — output should come via StderrSink
cargo run -p roko-cli -- plan run plans/ 2>&1 | head -20

# Verify stream_to_stderr is completely gone:
grep -rn 'stream_to_stderr' crates/roko-cli/src/runner/ --include='*.rs'
# Should return nothing
```

Expected observable behavior:
- `plan run` with normal CLI output still shows agent/task/gate progress on stderr.
- `--quiet`, `--json`, approval TUI, and serve/job paths use `NoopSink` and do not emit inline
  stderr progress.
- `rg 'stream_to_stderr' crates/roko-cli/src -g '*.rs'` returns nothing.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'stream_to_stderr' crates/roko-cli/src/ --include='*.rs'` — nothing
- [ ] `grep -rn 'RunOutputSink\|StderrSink' crates/roko-cli/src/runner/ --include='*.rs'` — shows callers in BOTH files
- [ ] `rg 'RunnerInlineTerminal::new|inline\\.' crates/roko-cli/src/runner/event_loop.rs crates/roko-cli/src/runner/agent_events.rs` — no runtime output calls remain outside the sink
- [ ] `cargo run -p roko-cli -- plan run plans/ 2>&1 | head -20` shows progress through `StderrSink`
- [ ] Status Log documents AgentStreamBuffer decision

## Status Log

| Time | Agent | Action |
|------|-------|--------|
