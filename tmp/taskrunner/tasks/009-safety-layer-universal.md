# Task 009: Wire SafetyLayer Check Calls into All Dispatch Backends

```toml
id = 9
title = "Add SafetyLayer.check() invocations to backends that bypass ToolDispatcher"
track = "runner-hardening"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-agent/src/dispatcher/mod.rs",
    "crates/roko-agent/src/provider/mod.rs",
    "crates/roko-agent/src/exec.rs",
    "crates/roko-agent/src/cursor_agent.rs",
    "crates/roko-agent/src/gemini/native.rs",
    "crates/roko-agent/src/gemini/adapter.rs",
    "crates/roko-agent/src/tool_loop/backends/gemini_native.rs",
    "crates/roko-agent/src/provider/cursor_acp.rs",
]
exclusive_files = []
estimated_minutes = 90
```

## Context

SafetyLayer uses a thread-local scoping pattern (`ACTIVE_SAFETY_LAYER`, `with_safety_layer()`,
`current_safety_layer()`), not struct fields. `ToolDispatcher` calls safety checks, but some
backends (`ExecAgent`, `GeminiBackend`, `CursorBackend`) have their own dispatch paths that
bypass `ToolDispatcher` entirely and never invoke safety checks.

Current code inspection shows some direct backends may already have safety checks. Treat this task
as a coverage audit plus targeted gap fill; do not duplicate checks that are already present.

**Making SafetyLayer non-optional in structs won't fix this** — the issue is that backends
don't CALL the check methods, not that they don't HOLD a reference.

Sources:
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` — W15-B: SafetyLayer required pattern
- Audit finding: thread-local scoping means the fix is adding check() invocations, not struct changes

## Background

Read these files:
1. `crates/roko-agent/src/dispatcher/mod.rs` — how ToolDispatcher calls safety checks
2. `crates/roko-agent/src/provider/mod.rs` — thread-local scoping (`ACTIVE_SAFETY_LAYER`)
3. Trace each backend's dispatch path:
   ```bash
   grep -rn 'ExecAgent\|GeminiBackend\|CursorBackend' crates/roko-agent/ --include='*.rs' | grep -v target/
   ```
4. Find where tool calls are executed WITHOUT safety checks:
   ```bash
   grep -rn 'execute_tool\|run_tool\|tool_call' crates/roko-agent/ --include='*.rs' | grep -v target/ | grep -v safety
   ```

## What to Change

1. **Trace each backend's tool execution path** — identify exactly where tool calls happen
   without safety checks.
2. **Add `safety_layer.check(tool_call)` invocations** at each tool execution point in:
   - `ExecAgent` (subprocess dispatch)
   - `GeminiBackend` (direct API tool calls)
   - `CursorBackend` (direct tool calls)
   - Any other backend that bypasses ToolDispatcher
3. **Use `current_safety_layer()`** from the thread-local to get the active layer, or
   thread a safety reference through the backend's dispatch method.
4. **If a backend has no tool execution path** (i.e., it only does text completion, no tool use),
   document this in the Status Log — not all backends need safety checks.

## What NOT to Do

- Don't change SafetyLayer's API.
- Don't change the thread-local scoping pattern.
- Don't just make `Option<SafetyLayer>` into `SafetyLayer` on structs — that's cosmetic, not functional.
- Don't add safety checks to the text completion path (only tool execution needs it).
- Don't double-check calls that already go through `ToolDispatcher`; that can consume rate limits twice.

## Implementation Notes

Current runtime construction path:
`roko-cli` dispatch resolves a model →
`roko-agent::provider::create_agent_for_model()` →
`with_safety_layer(Some(SafetyLayer::from_config(config)), || adapter.create_agent(...))` →
backend-specific agent construction. Tool-loop backends receive a `ToolDispatcher` from
`provider::build_tool_dispatcher()`, while direct backends need their own safety calls.

Files/functions to read before editing:
- `crates/roko-agent/src/dispatcher/mod.rs`: `ToolDispatcher::dispatch()` already runs
  `self.safety.check_pre_execution(&call, ctx)` and scrubs/recovery-checks results later.
- `crates/roko-agent/src/provider/mod.rs`: `ACTIVE_SAFETY_LAYER`, `with_safety_layer()`,
  `with_scoped_safety_layer()`, `build_tool_dispatcher()`, `current_safety_layer()`,
  `create_agent_for_model()`.
- `crates/roko-agent/src/safety/mod.rs`: `SafetyLayer::check_pre_execution()`,
  `check_exec_command()`, `scrub_text()`, `check_recovery()`.
- `crates/roko-agent/src/exec.rs`: `ExecAgent::run()` raw subprocess path.
- `crates/roko-agent/src/cursor_agent.rs`: `CursorAgent::check_tool_calls_for_safety()`,
  `LlmBackend::send_turn()`, `send_turn_streaming()`.
- `crates/roko-agent/src/gemini/native.rs`: `GeminiNativeAgent::new()`, `parse_response()`,
  `Agent::run()`.
- `crates/roko-agent/src/tool_loop/backends/gemini_native.rs`: `GeminiNativeBackend::send_turn()`.
- `crates/roko-agent/src/gemini/adapter.rs` and `provider/cursor_acp.rs`: ensure adapters pass
  `current_safety_layer().unwrap_or_else(SafetyLayer::with_defaults)` into direct backends.

Mechanical coverage rules:
1. Tool-loop paths that call `ToolDispatcher::dispatch()` or `dispatch_batch()` are already covered.
   Document them in the Status Log; do not add another `SafetyLayer` check around them.
2. `ExecAgent` has no model tool-call loop. Its safety requirement is subprocess launch safety:
   `self.safety.check_exec_command(&self.program, &self.args)` must run before `Command::new()`.
3. Direct LLM backends that expose tool calls without `ToolDispatcher` must parse tool calls into
   `roko_core::tool::ToolCall` and call `self.safety.check_pre_execution(&call, &tool_ctx)` before
   returning the response to the caller.
4. Use `ToolContext::testing(current_dir)` only as the minimal existing context when the backend has
   no richer worktree context. If richer worktree context is already available, use it instead.
5. For direct text outputs, keep result scrubbing/recovery checks (`scrub_text()` and
   `check_recovery()`) where already present, but do not treat ordinary text completion as a tool
   execution.
6. If `current_safety_layer()` is used, call it during synchronous construction as the provider code
   does today. Do not rely on thread-local safety surviving across `.await`.

Expected backend classification:
- Covered by `ToolDispatcher`: shared `tool_loop` backends, OpenAI-compatible tool loop,
  Anthropic tool loop, Perplexity tool loop.
- Direct checks required: `ExecAgent` (`check_exec_command()`), `CursorAgent` tool-call responses,
  `GeminiNativeAgent` grounding/code-execution path, `GeminiNativeBackend` shared tool-loop backend
  adapter before it hands parsed calls back.
- Text-only/no-tool paths: Gemini embed/compat, basic Claude CLI, Ollama non-tool response paths
  unless they are explicitly wrapped by a tool loop.

Tests to add/update:
- `exec.rs`: dangerous shell wrapper command is blocked before spawn and returns a failed
  `AgentResult`.
- `cursor_agent.rs`: non-streaming and streaming fixture with `bash {"command":"rm -rf /"}` returns
  an `LlmError` containing `blocked by safety layer`.
- `gemini/native.rs` and `tool_loop/backends/gemini_native.rs`: function-call fixture with dangerous
  bash command is blocked.
- Provider construction tests: scoped safety layer is propagated into Cursor/Gemini direct agents.

## Wire Target

```bash
# Verify via grep that all tool execution paths have safety checks:
grep -rn 'execute_tool\|run_tool' crates/roko-agent/ --include='*.rs' | grep -v target/
# Each result should be near a safety check call
grep -rn 'check_pre_execution\|check_exec_command' crates/roko-agent/src --include='*.rs' | grep -v target/
cargo test -p roko-agent safety_layer -- --nocapture
cargo test -p roko-agent blocks_dangerous_tool_call -- --nocapture
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] Every backend's tool execution path calls a safety check method
- [ ] Status Log documents which backends have tool execution (vs text-only)
- [ ] `rg -n 'ToolDispatcher::new\\(|build_tool_dispatcher\\(' crates/roko-agent/src -g '*.rs'` — new dispatcher construction either uses `build_tool_dispatcher()` or explicitly calls `.with_safety(...)`
- [ ] `rg -n 'check_pre_execution|check_exec_command' crates/roko-agent/src -g '*.rs'` — shows direct-backend safety callsites outside tests

## Status Log

| Time | Agent | Action |
|------|-------|--------|
