# Task 082: Streaming-First LlmBackend Redesign

```toml
id = 82
title = "Redesign LlmBackend trait to be streaming-first: StreamEvent enum, stream_turn() primary, TurnConfig, measured TTFT"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "high"
blocked_by = [45]
touches = [
    "crates/roko-agent/src/provider/mod.rs",
    "crates/roko-agent/src/provider/claude_cli.rs",
    "crates/roko-agent/src/provider/openai_compat.rs",
    "crates/roko-agent/src/provider/anthropic_api.rs",
    "crates/roko-agent/src/tool_loop/mod.rs",
    "crates/roko-agent/src/openai_compat_backend.rs",
    "crates/roko-agent/src/provider/claude_cli/stream.rs",
    "crates/roko-agent/src/streaming.rs",
    "crates/roko-agent/src/translate/mod.rs",
    "crates/roko-core/src/chat_types.rs",
]
exclusive_files = [
    "crates/roko-agent/src/provider/mod.rs",
]
estimated_minutes = 480
```

## Context

Today `LlmBackend::send_turn()` in `crates/roko-agent/src/tool_loop/mod.rs` is the primary
method — it blocks until the full response arrives. Streaming exists via
`send_turn_streaming()` with an `UnboundedSender<StreamChunk>` (the subject of task 45), but
streaming is an afterthought layered onto the blocking primary.

The consequences are catalogued in the audit:
- **S22.1**: `OpenAiCompatLlmBackend::send_turn()` blocks HTTP with zero feedback. No heartbeat,
  no spinner, no partial output. Users see nothing for minutes.
- **S22.2**: `ClaudeCliAgent` uses `emit_stream_summary()` which accumulates `text_bytes` but
  never prints them live. Text appears only in the `"result"` summary after full completion.
- **S25.1**: TTFT is unmeasurable. `ResponseMetadata.provider_latency_ms` exists but is never
  populated because `send_turn` gets the full response at once.

This redesign makes `stream_turn()` the PRIMARY method. Every provider implements it.
`send_turn()` becomes a convenience wrapper that collects the stream. TTFT is measured
naturally from the time of the first `StreamEvent::TextDelta` event.

This is Phase 2.1 from `tmp/redesign-plan.md`.

**Scope boundary**: This task redesigns the `LlmBackend` trait and its implementations. It
does NOT wire progress events to the CLI, TUI, or SSE — that is task 083. It DOES emit
`StreamEvent` values into the `tokio::sync::mpsc::Sender` added by task 45 (bounded channel).

## Background

Read these files before starting:

1. `crates/roko-agent/src/tool_loop/mod.rs` — the current `LlmBackend` trait (around line 80),
   `LlmError` (line 138), and `send_turn_streaming()` with `UnboundedSender<StreamChunk>`.
   After task 45 runs, `StreamChunk` is bounded; understand both the before and after state.
2. `crates/roko-agent/src/openai_compat_backend.rs` — `OpenAiCompatLlmBackend::send_turn()`
   at line 401. This is the primary target for S22.1. It does a single blocking HTTP request
   and returns the full `BackendResponse`. No streaming at all.
3. `crates/roko-agent/src/provider/claude_cli/stream.rs` — Claude CLI streaming parser.
   `emit_stream_summary()` accumulates `text_bytes` but never emits them incrementally.
4. `crates/roko-agent/src/provider/mod.rs` — `ProviderAdapter` trait and `adapter_for_kind`.
   The provider layer sits above the `LlmBackend` layer; understand the relationship.
5. `tmp/redesign-plan.md` Phase 2.1 — the full specification for this change, including
   `StreamEvent`, `TurnConfig`, and the `stream_turn()` signature.
6. `tmp/infrastructure-audit.md` sections S22.1, S22.2, S25.1 — the problems being fixed.

## What to Change

### 1. Define `StreamEvent` and `TurnConfig` in `tool_loop/mod.rs`

Add these types near the existing `LlmError` definition (around line 136):

```rust
use std::time::Instant;
use futures::stream::BoxStream;

/// A single event emitted by a streaming LLM backend during one turn.
///
/// Events arrive in order. The sequence for a normal turn is:
/// TextDelta* → ToolCallStart? → ToolCallDelta* → ToolCallEnd? → Usage → Done
/// A turn may have multiple interleaved ToolCall* sequences for parallel tool calls.
#[derive(Debug, Clone)]
pub struct StreamEvent {
    pub kind: StreamEventKind,
    /// Monotonic timestamp — used to measure TTFT from first TextDelta.
    pub timestamp: Instant,
}

#[derive(Debug, Clone)]
pub enum StreamEventKind {
    /// First text token received. TTFT is measured at this event.
    /// Always emitted even if the text delta is empty, so callers can
    /// measure TTFT independently of text content.
    TextDelta(String),

    /// A tool call is starting. Emitted before ToolCallDelta events.
    ToolCallStart { id: String, name: String },

    /// Partial JSON arguments for an in-progress tool call.
    ToolCallDelta { id: String, json_fragment: String },

    /// A tool call is complete with fully assembled arguments.
    ToolCallEnd { id: String, name: String, args: serde_json::Value },

    /// Final usage statistics for this turn.
    Usage(crate::usage::Usage),

    /// The turn is complete. No more events will follow.
    Done { finish_reason: String },
}

/// Per-turn configuration that replaces scattered parameters.
///
/// Previously these were passed as individual arguments or threaded through
/// session state. `TurnConfig` consolidates them into one struct so backends
/// can be called with a consistent interface.
#[derive(Debug, Clone)]
pub struct TurnConfig {
    /// Maximum output tokens. Taken from model profile or DEFAULT_MAX_TOKENS.
    pub max_tokens: u32,
    /// Optional sampling temperature. None = provider default.
    pub temperature: Option<f32>,
    /// Timeout for the first token to arrive. After this, the backend
    /// should return LlmError::Timeout rather than waiting indefinitely.
    pub ttft_timeout: std::time::Duration,
    /// Total request timeout including all streaming. The backend must
    /// complete the stream within this duration or return LlmError::Timeout.
    pub request_timeout: std::time::Duration,
    /// Stop sequences. Provider-specific; pass through verbatim.
    pub stop_sequences: Vec<String>,
}

impl Default for TurnConfig {
    fn default() -> Self {
        use roko_core::defaults::{DEFAULT_MAX_TOKENS, DEFAULT_TTFT_TIMEOUT_MS, DEFAULT_REQUEST_TIMEOUT_MS};
        Self {
            max_tokens: DEFAULT_MAX_TOKENS,
            temperature: None,
            ttft_timeout: std::time::Duration::from_millis(DEFAULT_TTFT_TIMEOUT_MS),
            request_timeout: std::time::Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
            stop_sequences: vec![],
        }
    }
}
```

### 2. Redesign the `LlmBackend` trait

Replace the existing trait definition:

```rust
// BEFORE (roughly):
#[async_trait::async_trait]
pub trait LlmBackend: Send + Sync {
    async fn send_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        session: &ToolContext,
    ) -> Result<BackendResponse, LlmError>;

    async fn send_turn_streaming(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        session: &ToolContext,
        event_tx: tokio::sync::mpsc::Sender<StreamChunk>,  // from task 45
    ) -> Result<BackendResponse, LlmError> { ... }
}
```

```rust
// AFTER:
#[async_trait::async_trait]
pub trait LlmBackend: Send + Sync {
    /// PRIMARY: Return a stream of events for one LLM turn.
    ///
    /// The stream MUST emit at minimum one TextDelta (even if empty) before Done.
    /// The FIRST TextDelta event is used to measure TTFT — emit it as soon as
    /// the first bytes arrive from the provider, before accumulating a complete
    /// message.
    ///
    /// The stream MUST emit Done as the final event. After Done, the stream ends.
    ///
    /// Backends should respect `config.ttft_timeout` and `config.request_timeout`.
    async fn stream_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        config: &TurnConfig,
    ) -> Result<BoxStream<'static, Result<StreamEvent, LlmError>>, LlmError>;

    /// Convenience wrapper: collect stream into a BackendResponse.
    ///
    /// Default implementation drives stream_turn() and reassembles the events.
    /// Backends should NOT override this — override stream_turn() instead.
    async fn send_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        config: &TurnConfig,
    ) -> Result<BackendResponse, LlmError> {
        let request_start = std::time::Instant::now();
        let stream = self.stream_turn(messages, tools, config).await?;
        collect_stream_to_response(stream, request_start).await
    }
}

/// Collect a StreamEvent stream into a BackendResponse and capture TTFT.
async fn collect_stream_to_response(
    mut stream: BoxStream<'static, Result<StreamEvent, LlmError>>,
    request_start: std::time::Instant,
) -> Result<BackendResponse, LlmError> {
    use futures::StreamExt;
    let mut text = String::new();
    let mut tool_calls = vec![];
    let mut usage = crate::usage::Usage::default();
    let mut finish_reason = "stop".to_string();
    let mut ttft_ms: Option<u64> = None;
    let mut in_progress_calls: std::collections::HashMap<String, (String, String)> = Default::default();

    while let Some(event) = stream.next().await {
        let event = event?;
        match event.kind {
            StreamEventKind::TextDelta(delta) => {
                if ttft_ms.is_none() {
                    ttft_ms = Some(request_start.elapsed().as_millis() as u64);
                }
                text.push_str(&delta);
            }
            StreamEventKind::ToolCallStart { id, name } => {
                in_progress_calls.insert(id, (name, String::new()));
            }
            StreamEventKind::ToolCallDelta { id, json_fragment } => {
                if let Some((_, args)) = in_progress_calls.get_mut(&id) {
                    args.push_str(&json_fragment);
                }
            }
            StreamEventKind::ToolCallEnd { id, name, args } => {
                in_progress_calls.remove(&id);
                tool_calls.push(ToolCall { id, name, args });
            }
            StreamEventKind::Usage(u) => usage = u,
            StreamEventKind::Done { finish_reason: fr } => {
                finish_reason = fr;
            }
        }
    }

    // Current tree note: `BackendResponse` is an enum, not this struct.
    // If the trait still returns `BackendResponse`, wrap the collected output in
    // the existing wire variant and keep TTFT in the canonical metadata path.
    // If the implementation migrates the collected return type to `ChatResponse`,
    // set `response.metadata.provider_ttft_ms = ttft_ms` there.
    let mut response = serde_json::json!({
        "message": { "content": text },
        "done_reason": finish_reason,
        "tool_calls": tool_calls,
        "usage": usage,
        "metadata": {},
    });
    response["metadata"]["provider_ttft_ms"] = serde_json::json!(ttft_ms);
    Ok(BackendResponse::Json(response))
}
```

**Critical**: `BoxStream<'static, ...>` requires the stream be `'static`. Use
`futures::stream::unfold` or `async_stream::stream!` to construct the stream. Do not
return a stream that borrows `&self` — box and pin the generator.

### 3. Implement `stream_turn` for `OpenAiCompatLlmBackend`

This is the S22.1 fix. Current `send_turn` at line 401 does a blocking HTTP call.

The new implementation uses `reqwest`'s streaming response (`Response::bytes_stream()`).
Parse SSE/JSONL chunks as they arrive and emit `StreamEvent` values:

```rust
#[async_trait::async_trait]
impl LlmBackend for OpenAiCompatLlmBackend {
    async fn stream_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        config: &TurnConfig,
    ) -> Result<BoxStream<'static, Result<StreamEvent, LlmError>>, LlmError> {
        // Build request body with stream: true
        let body = self.build_request_body(messages, tools, config, /*stream=*/true)?;
        let response = self.post_streaming(&body, config.request_timeout).await?;

        // Return a stream that parses SSE chunks
        let stream = response_to_stream(response);
        Ok(Box::pin(stream))
    }
}

fn response_to_stream(
    response: reqwest::Response,
) -> impl futures::Stream<Item = Result<StreamEvent, LlmError>> + 'static + Send {
    async_stream::try_stream! {
        use futures::StreamExt;
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut first_token = true;

        while let Some(chunk) = byte_stream.next().await {
            let bytes = chunk.map_err(|e| LlmError::Network(e.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // Parse SSE lines from buffer
            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer.drain(..=line_end);

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" { break; }
                    if let Ok(delta) = serde_json::from_str::<serde_json::Value>(data) {
                        for event in parse_openai_delta(&delta, &mut first_token) {
                            yield event;
                        }
                    }
                }
            }
        }

        yield StreamEvent {
            kind: StreamEventKind::Done { finish_reason: "stop".into() },
            timestamp: std::time::Instant::now(),
        };
    }
}
```

The key function `parse_openai_delta` emits `TextDelta` for content chunks and
`ToolCallStart/Delta/End` for tool call chunks. Consult the existing
`translate/openai.rs` for the delta parsing logic — do NOT duplicate it, call into it.

### 4. Implement `stream_turn` for Claude CLI

This fixes S22.2. Current `emit_stream_summary()` in `provider/claude_cli/stream.rs`
accumulates `text_bytes` without emitting them live.

The Claude CLI emits JSONL on stdout. When `content_block_delta` events arrive, emit
`StreamEvent::TextDelta` immediately:

```rust
// In the Claude CLI streaming reader:
match event_type.as_str() {
    "content_block_delta" => {
        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
            // EMIT immediately instead of accumulating
            if let Err(_) = event_tx.send(StreamEvent {
                kind: StreamEventKind::TextDelta(text.to_string()),
                timestamp: std::time::Instant::now(),
            }).await {
                break; // consumer dropped
            }
        }
    }
    // ... other event types
}
```

The Claude CLI backend's `stream_turn` should spawn the Claude CLI process, read its stdout
line by line, and yield `StreamEvent` values as they arrive. This is structurally similar to
the existing `ClaudeCliAgent::run_with_streaming` — reuse that subprocess logic, change only
the output format.

### 5. Update `ToolLoop::run_inner` to use `send_turn` with `TurnConfig`

The `ToolLoop` calls `self.backend.send_turn(...)` in the iteration loop. Update the call
to pass `TurnConfig`:

```rust
let config = TurnConfig {
    max_tokens: self.context.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
    temperature: self.context.temperature,
    ttft_timeout: Duration::from_millis(DEFAULT_TTFT_TIMEOUT_MS),
    request_timeout: Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
    stop_sequences: vec![],
};
let response = self.backend.send_turn(&messages, &tools, &config).await
    .map_err(|e| StopReason::BackendError(e.to_string()))?;
```

The `ToolContext` struct may need `max_tokens` and `temperature` fields added if they don't
exist. Check `ToolContext` definition in `tool_loop/mod.rs` first.

### 6. Remove or adapt the old `send_turn_streaming` signature

After task 45 bounded the channels, `send_turn_streaming(event_tx: Sender<StreamChunk>)`
still exists. This redesign supersedes that approach. Options:
- Remove `send_turn_streaming` from the trait and update callers to use `stream_turn`.
- Keep `send_turn_streaming` as a deprecated adapter that wraps `stream_turn` for backward
  compatibility during transition.

Prefer removal — fewer surfaces to maintain. If there are callers that depend on
`send_turn_streaming`, add a module-level note `// TODO(082): migrate to stream_turn`.

## Current Tree Notes and Mechanical Plan

The current code already has streaming infrastructure that the earlier prose does not mention.
Do not create a parallel parser or response accumulator.

Current facts to verify before editing:
- `crates/roko-agent/src/streaming.rs` defines `StreamChunk`, `UnifiedStreamEvent`,
  `StreamAccumulator`, `OpenAiSseParser`, `ClaudeCliParser`, and `parse_sse_line()`.
- `crates/roko-agent/src/openai_compat_backend.rs` already implements
  `send_turn_streaming()` with `stream: true`, first-body-chunk TTFT timeout, and
  incremental `StreamChunk` forwarding.
- `crates/roko-agent/src/provider/claude_cli/stream.rs` parses Claude stream-json into
  `AgentRuntimeEvent::MessageDelta` as soon as text appears.
- `BackendResponse` is an enum in `crates/roko-agent/src/translate/mod.rs`; canonical
  response metadata lives in `roko_core::chat_types::ResponseMetadata` and currently has
  `provider_latency_ms`, but no TTFT field.
- The active tool-loop call chain is
  `ToolLoopAgent::run_streaming()` -> `ToolLoop::run_streaming()` ->
  `ToolLoop::run_inner()` -> `LlmBackend::send_turn_streaming()` when an event sender is
  provided. The non-streaming path calls `send_turn_with_retry()` -> `LlmBackend::send_turn()`.
- The current `LlmBackend` signature uses `messages: &[serde_json::Value]`,
  `tools: &RenderedTools`, and `session: &SessionState`. Any new `stream_turn()` signature must
  preserve those data flows or explicitly put `SessionState` alongside `TurnConfig`; do not copy
  older examples that use undefined `Message`/`ToolDef` types in this module.

Implementation order:
1. Add `StreamEvent`, `StreamEventKind`, `TurnConfig`, and `collect_stream_to_response()` next
   to the `LlmBackend` trait in `tool_loop/mod.rs`. Reuse `StreamChunk` parsing helpers by
   converting `StreamChunk` -> `StreamEventKind`; do not duplicate SSE parsing.
2. Update `LlmBackend` so `stream_turn()` is primary and `send_turn()` is the default collector.
   Keep a temporary `send_turn_streaming()` adapter only if existing CLI dispatch callsites still
   need `StreamChunk`; the adapter should call `stream_turn()` and translate events back to
   `StreamChunk`.
3. Update every in-crate `impl LlmBackend` found with
   `rg -n "impl LlmBackend" crates/roko-agent/src -g '*.rs'`. At minimum this includes
   `openai_compat_backend.rs`, `cursor_agent.rs`, and test backends in `tool_loop/mod.rs`.
4. In `openai_compat_backend.rs`, build the request with `stream: true` in `stream_turn()` and
   feed bytes through `parse_sse_line()`/`StreamAccumulator`. Emit `TextDelta` immediately for
   content chunks and a final `Done` event. Preserve existing TTFT timeout behavior.
5. In Claude CLI code, map `AgentRuntimeEvent::MessageDelta` to `StreamEventKind::TextDelta`.
   Prefer adapting `parse_stream_line()` output rather than parsing Claude JSON a second time.
6. Add `provider_ttft_ms: Option<u64>` to `roko_core::chat_types::ResponseMetadata` unless a
   newer field already exists. Populate it in the collector from first `TextDelta`; leave
   `provider_latency_ms` for total provider/request latency.
7. Update `ToolLoop::run_inner()` to build `TurnConfig` from existing context/model defaults.
   If `ToolContext` does not contain max tokens/temperature, use model profile/default constants
   and do not add unrelated context fields.
8. Run `rg -n "send_turn_streaming|mpsc::UnboundedSender<StreamChunk>" crates/roko-agent/src`
   and either remove old callsites or annotate temporary adapters with the migration reason.

Observable behavior expected after implementation:
- `run_streaming()` receives text deltas before the full turn completes.
- `send_turn()` still returns the same final `BackendResponse` content/tool calls as before.
- OpenAI-compatible requests on the ToolLoop path contain `"stream": true`.
- TTFT timeout still fails before request timeout when the provider sends no first body chunk.
- `ResponseMetadata.provider_ttft_ms` is set for collected streamed responses.

Tests to add or update:
- `collect_stream_to_response()` unit test: text deltas concatenate, tool call deltas assemble,
  usage is preserved, and TTFT is non-`None`.
- OpenAI-compatible streaming test with delayed chunks: first delta arrives before final result
  and request body includes `"stream":true`.
- Claude parser adapter test: a text stream-json line becomes `StreamEventKind::TextDelta`.
- ToolLoop regression: existing `run_streaming()` test still emits chunks and final result matches
  non-streaming collection.

## What NOT to Do

- Do NOT wire `StreamEvent` values to the CLI stderr, TUI, or SSE routes. That is task 083.
  This task only defines and emits the events; task 083 consumes them.
- Do NOT implement `stream_turn` for providers other than OpenAI-compat and Claude CLI in
  this task. Gemini, Perplexity, Anthropic API, Cerebras adapters can return a stub:
  ```rust
  async fn stream_turn(&self, ...) -> Result<BoxStream<...>, LlmError> {
      // TODO(082): implement native streaming for this provider.
      // For now, move the backend's old `send_turn` body into a private helper
      // and emit one synthetic text/final event stream from that response.
      let response = self.send_turn_non_streaming_compat(messages, tools, config).await?;
      let events = response_to_event_stream(response);
      Ok(Box::pin(events))
  }
  ```
  This is acceptable because `send_turn` already works and the default impl handles it.
- Do NOT change the `Agent` trait or `ClaudeCliAgent::run`. Those use a different abstraction
  layer. Only the `LlmBackend` trait (used by `ToolLoopAgent`) is in scope.
- Do NOT add `provider_ttft_ms` to `BackendResponse`; it is currently an enum, not a metadata
  struct. Add/populate the field on `roko_core::chat_types::ResponseMetadata` unless a newer
  metadata location already exists.
- Do NOT duplicate the OpenAI delta parsing logic from `translate/openai.rs`. Call into the
  existing `streaming.rs` / translator helpers.

## Wire Target

The wire target is the existing `ToolLoopAgent` path. After this task, running any agent
via `create_agent_for_model` with an OpenAI-compat provider should stream text to the
stream consumer rather than blocking silently.

```bash
# Smoke test: run a short agent turn and verify it completes without hanging
RUST_LOG=roko_agent=debug cargo run -p roko-cli -- run "say hello" 2>&1 | head -20
# Should show streaming debug output, not a silent block.
```

Unit test targeting the new trait:
```bash
cargo test -p roko-agent -- stream_turn
# Should show tests for StreamEvent collection and TTFT measurement.
```

## Verification

- [ ] `cargo build --workspace` — all crates compile
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `grep -n 'fn stream_turn' crates/roko-agent/src/tool_loop/mod.rs` — method exists on trait
- [ ] `grep -n 'fn stream_turn' crates/roko-agent/src/openai_compat_backend.rs` — implemented
- [ ] `grep -n 'fn stream_turn' crates/roko-agent/src/provider/claude_cli/stream.rs` — implemented
- [ ] `grep -n 'struct TurnConfig' crates/roko-agent/src/tool_loop/mod.rs` — TurnConfig defined
- [ ] `grep -n 'struct StreamEvent\|enum StreamEventKind' crates/roko-agent/src/tool_loop/mod.rs`
  — both types defined
- [ ] `grep -n 'provider_ttft_ms' crates/roko-core/src/chat_types.rs crates/roko-agent/src/`
  finds the metadata field plus at least one assignment site
- [ ] Unit test for `collect_stream_to_response` with a mock stream passes
- [ ] The `ToolLoop` still calls into `LlmBackend::send_turn` (via the default impl) without
  panicking in the existing integration test suite

## Status Log

| Time | Agent | Action |
|------|-------|--------|
