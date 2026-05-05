# Task 074: Claude CLI Provider Completeness — Usage, FinishReason, ToolFormat

```toml
id = 74
title = "Fix Claude CLI provider: extract usage from StreamJson, surface finish_reason, correct tool_format in roko.toml and railway config"
track = "runner-hardening"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-agent/src/translate/mod.rs",
    "crates/roko-agent/src/provider/claude_cli.rs",
    "roko.toml",
    "docker/railway.roko.toml",
]
exclusive_files = ["crates/roko-agent/src/translate/mod.rs"]
estimated_minutes = 180
```

## Context

The Claude CLI provider (`kind = "claude_cli"`) is the ONLY provider used in production.
Every claude model in `roko.toml` (`anthropic`, `claude_cli` providers: haiku, sonnet,
claude-opus, claude-sonnet) routes through it. Three concrete gaps corrupt downstream
data for all Claude CLI sessions:

**GAP-I-33 — Zero tokens (all cost and budget accounting is blind)**

`BackendResponse::extract_usage()` in `crates/roko-agent/src/translate/mod.rs` lines 314-319:

```rust
pub fn extract_usage(&self) -> Usage {
    match self {
        Self::Json(v) => openai::parse_usage(v),
        Self::StreamJson(_) | Self::Text(_) => Usage::default(),
    }
}
```

All token accounting, cost estimation, budget tracking, and efficiency metrics show zeros for
every Claude CLI call. The Claude CLI protocol already carries per-session cumulative usage in
the `result` event (`ClaudeResultEvent.usage: Option<ClaudeUsage>`), and per-turn usage in
`assistant` events (`ClaudeMessage.usage: Option<ClaudeUsage>`). The `ClaudeUsage` struct in
`crates/roko-agent/src/provider/claude_cli/stream.rs` has four fields:
`input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`.

**GAP-I-34 — finish_reason always None, token-budget exhaustion invisible**

`BackendResponse::extract_finish_reason_raw()` in `crates/roko-agent/src/translate/mod.rs`
line 233:

```rust
Self::StreamJson(_) => None,
```

The tool loop's output-limit detection in `tool_loop/mod.rs` checks `finish_reason == "length"` to
detect budget exhaustion. This check never fires for Claude CLI because this function always
returns `None`. The Claude CLI `result` event carries `is_error: bool`, and the presence of
`tool` events vs their absence signals whether the session used tools.

**GAP-I-36 — Wrong tool_format for all claude models in roko.toml and railway config**

All models in `roko.toml` with `provider = "anthropic"` or `provider = "claude_cli"` have
`tool_format = "openai_json"`. The actual format used by `ClaudeCliAgent` is `"anthropic_blocks"`.
The test helper `claude_model()` in `crates/roko-agent/src/provider/claude_cli.rs` confirms
this with `tool_format: "anthropic_blocks".to_string()`. This mismatch is currently latent
(Claude CLI handles its own tool format internally) but will cause `translator_for_profile` to
pick the wrong translator the moment anyone switches to `anthropic_api` kind. Same error
exists in `docker/railway.roko.toml`.

## Background

Read these files before starting:

1. `crates/roko-agent/src/translate/mod.rs` — `BackendResponse::extract_usage()` (line ~314),
   `extract_finish_reason_raw()` (line ~225), and the `StreamJson` variant throughout
2. `crates/roko-agent/src/provider/claude_cli/stream.rs` — `ClaudeResultEvent`,
   `ClaudeAssistantEvent`, `ClaudeMessage`, `ClaudeUsage`, `parse_stream_line()`. Note
   `ClaudeResultEvent` has `usage: Option<ClaudeUsage>` and `is_error: bool`. The `result`
   event is what carries the cumulative session-level usage.
3. `crates/roko-agent/src/provider/claude_cli.rs` — `claude_model()` test helper confirms
   `tool_format: "anthropic_blocks"` is the correct value
4. `roko.toml` — grep for `provider = "anthropic"` to find all affected model entries.
   The haiku, claude-opus, claude-sonnet, and sonnet models all have `tool_format = "openai_json"`.
5. `docker/railway.roko.toml` — contains duplicate model entries with the same issue

Key observation: `parse_stream_line` in `stream.rs` already emits `AgentRuntimeEvent::TokenUsage`
from both `assistant` and `result` events correctly. That path (the runtime event flow) works.
The gap is in `BackendResponse::extract_usage()`, which operates directly on the raw JSON event
list stored in `StreamJson(Vec<serde_json::Value>)` and never looks for a `result` event.

The `Usage` struct in `roko-core/src/chat_types.rs` has these fields:
`input_tokens: u32`, `output_tokens: u32`, `cache_read_tokens: u32`, `cache_create_tokens: u32`,
`cost_usd: f32`, `wall_ms: u64`. Note: the cache-write field is `cache_create_tokens` (not
`cache_write_tokens`). Match these names exactly in the fix.

## What to Change

### 1. Fix `extract_usage()` for `StreamJson` (translate/mod.rs)

The `StreamJson(Vec<serde_json::Value>)` variant holds raw JSON events. Prefer the final
`result` event's cumulative usage; fall back to the last `assistant` event's usage if no
result event exists (partial/interrupted streams):

```rust
Self::StreamJson(events) => {
    // Prefer the final `result` event — it carries cumulative session usage.
    for ev in events.iter().rev() {
        if ev.get("type").and_then(|t| t.as_str()) == Some("result") {
            if let Some(usage) = ev.get("usage") {
                return Usage {
                    input_tokens: usage
                        .get("input_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    output_tokens: usage
                        .get("output_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    cache_read_tokens: usage
                        .get("cache_read_input_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    cache_create_tokens: usage
                        .get("cache_creation_input_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    ..Default::default()
                };
            }
            break; // result event present but no usage block — stop here
        }
    }
    // Fall back to the last assistant event's usage (partial stream).
    for ev in events.iter().rev() {
        if ev.get("type").and_then(|t| t.as_str()) == Some("assistant") {
            if let Some(msg) = ev.get("message")
                && let Some(usage) = msg.get("usage")
            {
                return Usage {
                    input_tokens: usage
                        .get("input_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    output_tokens: usage
                        .get("output_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    cache_read_tokens: usage
                        .get("cache_read_input_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    cache_create_tokens: usage
                        .get("cache_creation_input_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    ..Default::default()
                };
            }
        }
    }
    Usage::default()
}
```

**Important**: Verify the `Usage` field names against `roko-core/src/chat_types.rs` before
coding. The codebase uses `cache_create_tokens` not `cache_write_tokens`.

### 2. Fix `extract_finish_reason_raw()` for `StreamJson` (translate/mod.rs)

Scan events in reverse for the `result` event and derive a finish reason:

```rust
Self::StreamJson(events) => {
    for ev in events.iter().rev() {
        if ev.get("type").and_then(|t| t.as_str()) == Some("result") {
            // is_error: true maps to a terminal error condition.
            if ev.get("is_error").and_then(serde_json::Value::as_bool) == Some(true) {
                return Some("error".to_string());
            }
            // If any `tool` events exist in the stream, the session used tools.
            let has_tool = events
                .iter()
                .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("tool"));
            return Some(if has_tool { "tool_use" } else { "end_turn" }.to_string());
        }
    }
    // Also look for `stop_reason` in assistant events if Claude CLI exposes it.
    for ev in events.iter().rev() {
        if ev.get("type").and_then(|t| t.as_str()) == Some("assistant") {
            if let Some(stop_reason) = ev
                .pointer("/message/stop_reason")
                .and_then(serde_json::Value::as_str)
            {
                return Some(stop_reason.to_string());
            }
        }
    }
    None
}
```

**Note**: Check whether the actual Claude CLI `--output-format stream-json` protocol includes
`stop_reason` in assistant events. If it does not, remove that block. Do not add code paths
for fields that do not exist in the protocol.

### 3. Fix tool_format in roko.toml

Change `tool_format = "openai_json"` to `tool_format = "anthropic_blocks"` for every model
entry that uses `provider = "anthropic"` or `provider = "claude_cli"`. Search the file:

```bash
grep -n 'provider = "anthropic"\|provider = "claude_cli"' roko.toml
```

Each matching model section should have `tool_format = "anthropic_blocks"`. Affected models
in the current file are approximately: `haiku`, `claude-opus`, `claude-sonnet`, `sonnet`,
and any `claude_cli`-only model entries.

Do NOT change `tool_format` for gemini, ollama, openai, kimi, cerebras, zai, zhipu, or other
non-claude models.

### 4. Fix tool_format in docker/railway.roko.toml

Apply the same fix to `docker/railway.roko.toml`. Find the claude model entries there and
change their `tool_format` to `"anthropic_blocks"`.

### 5. Add tests to translate/mod.rs

Add to the `#[cfg(test)]` block in `translate/mod.rs`:

- `stream_json_extract_usage_from_result_event` — `StreamJson` with a `result` event that
  has a `usage` block with all four token fields set; assert each field is extracted correctly
- `stream_json_extract_usage_from_result_event_missing_usage` — `StreamJson` with a `result`
  event but no `usage` key; assert `Usage::default()` is returned
- `stream_json_extract_usage_no_result_falls_back_to_assistant` — `StreamJson` with an
  `assistant` event carrying usage but no `result` event; assert usage is extracted from
  the assistant event
- `stream_json_extract_usage_returns_default_when_no_events` — empty `StreamJson`; assert
  `Usage::default()` is returned
- `stream_json_extract_finish_reason_end_turn` — `StreamJson` with a `result` event where
  `is_error` is false and there are no `tool` events; assert `Some("end_turn")`
- `stream_json_extract_finish_reason_tool_use` — `StreamJson` with a `result` event and a
  `tool` event; assert `Some("tool_use")`
- `stream_json_extract_finish_reason_error` — `StreamJson` with a `result` event where
  `is_error` is true; assert `Some("error")`
- `stream_json_extract_finish_reason_none_when_no_result` — `StreamJson` with only
  `assistant` events and no `result` event; assert `None`

## What NOT to Do

- Do NOT modify `parse_stream_line` in `stream.rs` — it already correctly emits
  `AgentRuntimeEvent::TokenUsage`. The gap is upstream in `BackendResponse::extract_usage()`.
- Do NOT change the `Usage` struct definition or its field names. Verify field names first.
- Do NOT change `extract_text()`, `extract_reasoning()`, `extract_tool_outputs()`, or
  `extract_session_id()` — only `extract_usage()` and `extract_finish_reason_raw()`.
- Do NOT add fields to `ClaudeResultEvent` or `ClaudeUsage` — they already have what is needed.
- Do NOT change `tool_format` for non-claude models. The fix is surgical: only `provider =
  "anthropic"` and `provider = "claude_cli"` entries.
- Do NOT add a new `anthropic_api` provider entry to roko.toml — that is task 075's decision.

## Wire Target

```bash
# After fix: token counts should appear in efficiency log entries for claude runs
cargo run -p roko-cli -- run "summarize this project" 2>&1 | grep -i "token\|usage\|cost"

# Verify roko.toml has correct tool_format for claude models:
grep -A20 'provider = "anthropic"' roko.toml | grep tool_format
# Should show: tool_format = "anthropic_blocks"

# Verify railway config:
grep -A20 'provider = "anthropic"' docker/railway.roko.toml | grep tool_format
# Should show: tool_format = "anthropic_blocks"
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo test -p roko-agent -- stream_json_extract_usage` — all new usage tests pass
- [ ] `cargo test -p roko-agent -- stream_json_extract_finish_reason` — all new finish reason tests pass
- [ ] `cargo test -p roko-agent -- backend_response_extract_usage_from_openai_json` — existing test unchanged
- [ ] `grep 'provider = "anthropic"' roko.toml | wc -l` — count claude models
- [ ] `grep 'tool_format = "anthropic_blocks"' roko.toml | wc -l` — same count as above
- [ ] Same check passes for `docker/railway.roko.toml`
- [ ] No `tool_format = "openai_json"` entries remain under claude model sections

## Implementation Detail

### Current Code Facts to Account For

- `crates/roko-agent/src/translate/mod.rs::BackendResponse::extract_usage` currently returns `Usage::default()` for `StreamJson`.
- `extract_finish_reason_raw` currently returns `None` for `StreamJson`.
- Claude CLI streaming already parses assistant/result usage into `AgentRuntimeEvent::TokenUsage` in `crates/roko-agent/src/provider/claude_cli/stream.rs`; this task is about summary extraction from stored raw stream events.
- The current `roko.toml` has Claude CLI provider entries under `providers.anthropic` and `providers.claude_cli`, while model sections such as `haiku`, `claude-opus`, `sonnet`, `claude-sonnet`, and `opus` still use `tool_format = "openai_json"`.

### Mechanical Implementation Steps

1. In `translate/mod.rs`, add private helper functions for `StreamJson` extraction rather than changing the streaming parser. Parse the raw `serde_json::Value` events already stored in `BackendResponse::StreamJson`.
2. Usage extraction priority: use the last `result` event that contains a `usage` object; map `input_tokens`, `output_tokens`, `cache_read_input_tokens`, and `cache_creation_input_tokens` into `Usage`. If a `result` event exists with no usage, return default usage rather than falling through to assistant events. If there is no `result`, fall back to the last assistant/message usage object.
3. Convert JSON integer fields with clamping or checked conversion to the `Usage` field type. Do not use unchecked `as` casts from unbounded JSON numbers.
4. Finish reason extraction: use the last `result` event. If `is_error == true`, return `error`. If the stream contains any Claude tool-use shape, return `tool_use`. Otherwise return `end_turn`. If no result event exists, return `None`.
5. Tool-use detection must cover both assistant content blocks with `type = "tool_use"` and stream/event shapes such as `content_block_start` whose nested `content_block.type` is `tool_use`. Keep detection local to `StreamJson` finish extraction.
6. Update `roko.toml` only for model sections whose `provider` resolves to a provider with `kind = "claude_cli"`. Use a TOML-aware script or careful section-local edits; do not globally replace every `tool_format = "openai_json"`.
7. For `docker/railway.roko.toml`, first check whether the file exists in the implementation worktree. If it is intentionally deleted or moved, report that scope issue rather than recreating it blindly. If present and using Anthropic Messages API instead of Claude CLI, `anthropic_blocks` is still the correct tool format, but the reason is Anthropic API compatibility rather than Claude CLI.

### Tests to Add or Update

- In the existing `translate/mod.rs` tests, add `StreamJson` usage tests for: result event with all token fields, result event without usage returning default, no-result assistant fallback, empty stream default, and large token values clamped safely.
- Add finish reason tests for normal result -> `end_turn`, result plus tool-use block -> `tool_use`, error result -> `error`, and no result -> `None`.
- Keep existing OpenAI JSON usage tests unchanged; this task must not regress non-stream summary extraction.

### Additional Verification Commands

- `cargo test -p roko-agent -- stream_json`
- `cargo test -p roko-agent -- backend_response_extract_usage_from_openai_json`
- `cargo test -p roko-agent -- provider::claude_cli`
- Config audit: parse `roko.toml`, find providers with `kind = "claude_cli"`, then assert every model that references one of those providers has `tool_format = "anthropic_blocks"`.

### Additional What NOT To Do

- Do not modify `parse_stream_line` to solve summary extraction unless a failing test proves raw events are missing required data.
- Do not map token exhaustion to `length` unless the Claude CLI raw result actually exposes a stop/max-token reason in current fixtures.
- Do not change tool formats for Gemini, Ollama, OpenAI, OpenRouter, or other non-Claude models while fixing Claude CLI sections.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
