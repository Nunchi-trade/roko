# Task 045: Bound LLM Streaming Channels

```toml
id = 45
title = "Replace UnboundedSender<StreamChunk> with bounded channels in LLM streaming"
track = "infrastructure"
wave = "wave-2"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-agent/src/agent.rs",
    "crates/roko-agent/src/tool_loop/mod.rs",
    "crates/roko-agent/src/tool_loop/agent_wrapper.rs",
    "crates/roko-agent/src/openai_compat_backend.rs",
    "crates/roko-agent/src/cursor_agent.rs",
    "crates/roko-agent/src/testutil.rs",
    "crates/roko-agent/tests/cursor_streaming.rs",
    "crates/roko-agent/tests/codex_conformance.rs",
    "crates/roko-cli/src/dispatch_v2.rs",
    "crates/roko-agent-server/src/state.rs",
    "crates/roko-agent-server/src/features/messaging.rs",
]
exclusive_files = [
    "crates/roko-agent/src/agent.rs",
    "crates/roko-agent/src/tool_loop/mod.rs",
    "crates/roko-agent/src/tool_loop/agent_wrapper.rs",
    "crates/roko-agent/src/openai_compat_backend.rs",
    "crates/roko-agent/src/cursor_agent.rs",
    "crates/roko-cli/src/dispatch_v2.rs",
    "crates/roko-agent-server/src/state.rs",
    "crates/roko-agent-server/src/features/messaging.rs",
]
estimated_minutes = 180
```

## Context

The `LlmBackend::send_turn_streaming` trait and its callers use
`mpsc::UnboundedSender<StreamChunk>` for LLM streaming. The audit (S15.5) noted this
as "not fixed (trait-constrained)" because changing it requires modifying 10+ backend
implementations. While LLM response rate provides natural backpressure, a misbehaving
or extremely fast provider could fill memory.

## Background

Read:
- `crates/roko-agent/src/agent.rs` — `Agent::run_streaming` default implementation
- `crates/roko-agent/src/tool_loop/mod.rs` — tool loop streaming
- `crates/roko-agent/src/openai_compat_backend.rs` — `send_turn_streaming` impl
- `crates/roko-agent/src/cursor_agent.rs` — second `LlmBackend::send_turn_streaming`
  impl and `push_stream_line` helper
- `crates/roko-cli/src/dispatch_v2.rs` — runtime `Agent::run_streaming` caller
  currently creates `mpsc::unbounded_channel::<StreamChunk>()` at line 829
- `crates/roko-agent-server/src/state.rs` and
  `crates/roko-agent-server/src/features/messaging.rs` — public
  `DispatchLike` streaming remains unbounded for now; bridge it internally to
  the bounded `LlmBackend` sender without changing the public trait
- Existing tests that create streaming channels:
  `crates/roko-agent/src/testutil.rs:463`,
  `crates/roko-agent/src/openai_compat_backend.rs:1231,1387`,
  `crates/roko-agent/src/tool_loop/mod.rs:1586`,
  `crates/roko-agent/tests/cursor_streaming.rs:453`, and
  `crates/roko-agent/tests/codex_conformance.rs:431`

Grep for the scope:
```bash
grep -rn 'UnboundedSender<StreamChunk>\|unbounded_channel' \
  crates/roko-agent/src crates/roko-agent/tests crates/roko-cli/src/dispatch_v2.rs crates/roko-agent-server/src \
  --include='*.rs' | grep -v target/
```

Runtime chain for the main streaming path:

```text
roko-cli dispatch_v2::run_agent_streaming()
  -> created.agent.run_streaming(...)
  -> ToolLoopAgent::run_streaming()
  -> ToolLoop::run_streaming()/run_messages_streaming()
  -> ToolLoop::run_inner()
  -> LlmBackend::send_turn_streaming()
  -> OpenAiCompatLlmBackend or CursorAgent streaming parser
```

## What to Change

1. **Change streaming trait signatures inside `roko-agent`**:
   - `Agent::run_streaming` in `agent.rs`: change
     `mpsc::UnboundedSender<StreamChunk>` to `mpsc::Sender<StreamChunk>`.
   - `LlmBackend::send_turn_streaming` in `tool_loop/mod.rs`: change
     `event_tx: mpsc::UnboundedSender<StreamChunk>` to
     `event_tx: mpsc::Sender<StreamChunk>` (bounded).
   - Update `ToolLoop::run_streaming`, `ToolLoop::run_messages_streaming`,
     `ToolLoop::run_inner`, and `ToolLoopAgent::run_streaming` to carry
     `mpsc::Sender<StreamChunk>`.

2. **Update all implementations** to use `event_tx.send().await` instead of
   `event_tx.send()` (bounded send is async). Where the sender is used in a
   synchronous context, first try to make the helper async. Use
   `event_tx.try_send()` only in non-async boundary adapters where awaiting is
   impossible, and log when a chunk is dropped.

3. **Update all callsites** that create the channel from `mpsc::unbounded_channel()`
   to `mpsc::channel(roko_core::defaults::DEFAULT_CHANNEL_BUFFER)`. The
   constant already exists at `crates/roko-core/src/defaults.rs:235`; do not add
   another buffer constant unless you can justify why streaming needs a distinct
   size.

4. **Handle backpressure**: If `send().await` blocks because the consumer is slow,
   that is correct behavior (backpressure). Do NOT drop chunks silently — the
   consumer must keep up or the stream will slow down naturally.

5. **Bridge agent-server without changing its public trait**:
   - Keep `DispatchLike::dispatch_streaming(..., mpsc::UnboundedSender<StreamChunk>)`
     unchanged in `roko-agent-server/src/state.rs`.
   - In `BackendMessageDispatcher::dispatch_streaming`, create an internal
     bounded channel, pass its `Sender` to `LlmBackend::send_turn_streaming`,
     and forward received chunks to the existing unbounded public sender. Await
     the forwarder after the backend finishes so the final chunks drain.
   - In `features/messaging.rs`, leave the websocket-facing unbounded channel
     unless you are doing the separate `DispatchLike` refactor.

## Mechanical Implementation Notes

`OpenAiCompatLlmBackend::push_stream_line` and `CursorAgent::push_stream_line`
currently take `&mpsc::UnboundedSender<StreamChunk>` and synchronously call
`send()`. Make these helpers `async fn ... -> Result<(), mpsc::error::SendError<StreamChunk>>`
or inline the send at the async callsite so parsed provider chunks use
`event_tx.send(chunk).await`.

The current error paths use `map_err` closures that call `event_tx.send(...)`.
Those closures cannot `.await`. Rewrite them as explicit `match` blocks:

```rust
let response = match req.body(body_bytes).send().await {
    Ok(response) => response,
    Err(err) => {
        let message = format!("request failed: {err}");
        let _ = event_tx.send(StreamChunk::Error(message.clone())).await;
        return Err(LlmError::Network(message));
    }
};
```

Apply the same pattern to non-success response-body reads, TTFT timeout error
emission, and `response.chunk().await` errors.

## Tests to Add or Update

- Update all streaming tests and test utilities listed in Background to create
  bounded channels with `mpsc::channel(roko_core::defaults::DEFAULT_CHANNEL_BUFFER)`.
- Add/adjust one `ToolLoop` test to prove a bounded channel still receives the
  final `Done` chunk for a normal streaming turn.
- Add/adjust one provider streaming test (`openai_compat_backend.rs` or
  `cursor_streaming.rs`) to drain the receiver while the streaming future runs;
  do not let a bounded buffer fill in the test before polling the receiver.

## Expected Observable Behavior

Streaming output order and content must not change. Under a slow consumer, the
provider streaming loop naturally slows down once the bounded channel fills
instead of allocating unbounded memory.

## What NOT to Do

- Don't change the `StreamChunk` type or its fields.
- Don't change the `DispatchLike` trait signature in agent-server — bridge it
  internally as described above.
- Don't add buffering on the consumer side — the channel IS the buffer.
- Don't convert to `broadcast` channels — these are point-to-point streams.
- Don't keep `try_send()` in provider stream parsers; it can silently reorder or
  drop meaningful chunks under backpressure.

## Wire Target

```bash
cargo build --workspace
cargo test -p roko-agent --lib
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'UnboundedSender<StreamChunk>' crates/roko-agent/src/` returns zero matches
- [ ] `grep -rn 'unbounded_channel' crates/roko-agent/src/` returns only test code
- [ ] `grep -rn 'mpsc::channel(roko_core::defaults::DEFAULT_CHANNEL_BUFFER)' crates/roko-agent/src crates/roko-cli/src/dispatch_v2.rs crates/roko-agent-server/src --include='*.rs'`
      shows the new bounded stream channels/adapters

## Status Log

| Time | Agent | Action |
|------|-------|--------|
