# Task 075: Provider Translator Parity — URL Fix, Tool Sanitization, Assistant Message, Anthropic API Status

```toml
id = 75
title = "Fix provider/translator gaps: Gemini doubled URL, Ollama tool name sanitization, render_assistant_message documentation, Anthropic API status"
track = "runner-hardening"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-agent/src/gemini/adapter.rs",
    "crates/roko-agent/src/gemini/compat.rs",
    "crates/roko-agent/src/translate/ollama.rs",
    "crates/roko-agent/src/provider/anthropic_api/tool_loop.rs",
    "crates/roko-agent/src/provider/anthropic_api.rs",
]
exclusive_files = [
    "crates/roko-agent/src/translate/ollama.rs",
    "crates/roko-agent/src/provider/anthropic_api/tool_loop.rs",
]
estimated_minutes = 240
```

## Context

Four gaps across the provider/translator layer cause silent failures or dead code that blocks
future provider switching. None of these are blocked by task 074, but they compound its value:
once token usage is visible (074), Gemini and Ollama need to be correct to use that data.

**GAP-I-31 — Gemini OpenAI-compat URL is doubled in production**

`roko.toml` configures the gemini provider:
```toml
[providers.gemini]
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
```

Two separate path-construction helpers then append the same suffix to this URL:

In `crates/roko-agent/src/gemini/compat.rs` line 14-17:
```rust
fn compat_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{trimmed}/v1beta/openai")
}
```
Result when given the roko.toml value: `.../v1beta/openai/v1beta/openai`.

In `crates/roko-agent/src/gemini/adapter.rs` line 31-33:
```rust
fn gemini_tool_loop_base_url(base_url: &str) -> String {
    format!("{}/v1beta/openai/v1", base_url.trim_end_matches('/'))
}
```
Result when given the roko.toml value: `.../v1beta/openai/v1beta/openai/v1`.

The `DEFAULT_BASE_URL` constant in `adapter.rs` line 29 is `"https://generativelanguage.googleapis.com"`
(bare domain, no path) — this works correctly. The bug manifests only when `base_url` is
provided in `roko.toml` (the production path). Currently latent because all gemini models use
`kind = "openai_compat"` not `kind = "gemini_api"`, so `GeminiAdapter` is never invoked. But
this must be fixed before enabling native Gemini grounding or code-execution features.

**GAP-I-38 — Ollama tool names with dots silently break**

`crates/roko-agent/src/translate/openai.rs` lines 161-181 sanitizes dotted names:
```rust
fn sanitize_tool_name(name: &str) -> String {
    if name.contains('.') { name.replace('.', "__DOT__") } else { name.to_string() }
}
fn unsanitize_tool_name(name: &str) -> String {
    if name.contains("__DOT__") { name.replace("__DOT__", ".") } else { name.to_string() }
}
```
OpenAI-compat protocol requires tool names matching `^[a-zA-Z0-9_-]+$`. Ollama uses the same
wire format but `crates/roko-agent/src/translate/ollama.rs` `render_tools()` lines 61-74 sends
`t.name` verbatim. If any MCP tool has a dotted name (e.g. `chain.balance`, `github.create_issue`),
Ollama rejects or misroutes it. The `parse_calls()` counterpart in the same file does not
unsanitize names either, so the round-trip is broken both ways.

**GAP-I-39 — render_assistant_message status needs verification and documentation**

The audit (S23.12) claims `render_assistant_message` is missing for Anthropic API and Ollama.
Before treating this as a gap to fix, verify current state:

- `OllamaTranslator` in `crates/roko-agent/src/translate/ollama.rs` — check line ~119
- `AnthropicTranslator` (private struct) in `crates/roko-agent/src/provider/anthropic_api/tool_loop.rs` — check line ~196
- `ClaudeTranslator` in `crates/roko-agent/src/translate/claude.rs` — check if it implements or inherits default

If these already implement `render_assistant_message`, the audit is stale. In that case, add
doc comments to `ClaudeTranslator` and `ReActTranslator` explaining why `None` is correct for
each (they have intentional reasons). If implementations are missing where they should exist,
add them.

**GAP-I-35 — Anthropic API adapter is untested dead code**

`AnthropicApiAdapter` in `crates/roko-agent/src/provider/anthropic_api.rs` implements the full
Anthropic Messages API tool loop. Zero model entries in `roko.toml` use `kind = "anthropic_api"`.
The adapter and its tool-loop code (`anthropic_api/tool_loop.rs`) are never invoked in production.
This must be either explicitly marked as experimental with activation instructions, or wired to
at least one model entry to make it production-reachable.

## Background

Read these files before starting:

1. `crates/roko-agent/src/gemini/adapter.rs` — `gemini_tool_loop_base_url()` (lines 31-33),
   `DEFAULT_BASE_URL` (line 29), `gemini_tool_loop_agent()` (lines 35-106). Note that this
   adapter is only invoked when a provider has `kind = "gemini_api"` — currently zero models.
2. `crates/roko-agent/src/gemini/compat.rs` — `compat_base_url()` (lines 14-17),
   `GeminiCompatAgent::new()` to see how `base_url` flows through to the HTTP backend.
3. `crates/roko-agent/src/translate/ollama.rs` — `render_tools()` (lines 60-75) uses
   `t.name` directly; `parse_calls()` (lines 77-117) uses the raw name. Both need sanitization.
4. `crates/roko-agent/src/translate/openai.rs` — `sanitize_tool_name()` and
   `unsanitize_tool_name()` (lines 161-181). Check if these functions are `pub` or `pub(crate)`
   or private. If private, duplicate the logic in ollama.rs (do not make them pub just for this).
5. `crates/roko-agent/src/translate/mod.rs` — `Translator` trait, `render_assistant_message`
   default impl (lines 99-103). Check which translators implement it.
6. `crates/roko-agent/src/translate/claude.rs` — check for `render_assistant_message` impl.
7. `crates/roko-agent/src/translate/react.rs` — check for `render_assistant_message` impl.
8. `crates/roko-agent/src/provider/anthropic_api.rs` — read the full adapter file.
9. `crates/roko-agent/src/provider/anthropic_api/tool_loop.rs` — `AnthropicTranslator` struct,
   check if it implements `render_assistant_message` (it does, around line 196).
10. `roko.toml` — `[providers.gemini]` base_url value; grep for `kind = "anthropic_api"` entries.

Important: the `openai.rs` sanitization functions are likely private (`fn`, not `pub fn`).
Do NOT make them public just to reuse them from ollama.rs. Duplicate the two small functions
with a comment pointing to the canonical source in openai.rs.

## What to Change

### 1. Fix Gemini URL doubling — idempotent suffix normalization

The cleanest fix strips the known suffix from the incoming URL before appending, making both
functions idempotent whether or not the suffix is already present.

In `crates/roko-agent/src/gemini/compat.rs`, change `compat_base_url()`:
```rust
fn compat_base_url(base_url: &str) -> String {
    // Strip the path suffix if it was already included in base_url (e.g. via roko.toml)
    // so this function is idempotent regardless of how base_url is configured.
    let trimmed = base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1beta/openai")
        .trim_end_matches('/');
    format!("{trimmed}/v1beta/openai")
}
```

In `crates/roko-agent/src/gemini/adapter.rs`, change `gemini_tool_loop_base_url()`:
```rust
fn gemini_tool_loop_base_url(base_url: &str) -> String {
    // Strip the path suffix if already present in base_url (idempotent).
    let trimmed = base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1beta/openai")
        .trim_end_matches('/');
    format!("{trimmed}/v1beta/openai/v1")
}
```

Add tests in each file verifying idempotency:

```rust
#[test]
fn compat_base_url_idempotent_with_suffix() {
    let with_suffix = "https://generativelanguage.googleapis.com/v1beta/openai";
    let bare = "https://generativelanguage.googleapis.com";
    assert_eq!(compat_base_url(with_suffix), compat_base_url(bare));
    assert_eq!(compat_base_url(with_suffix), "https://generativelanguage.googleapis.com/v1beta/openai");
}

#[test]
fn gemini_tool_loop_base_url_idempotent_with_suffix() {
    let with_suffix = "https://generativelanguage.googleapis.com/v1beta/openai";
    let bare = "https://generativelanguage.googleapis.com";
    assert_eq!(gemini_tool_loop_base_url(with_suffix), gemini_tool_loop_base_url(bare));
    assert_eq!(gemini_tool_loop_base_url(with_suffix), "https://generativelanguage.googleapis.com/v1beta/openai/v1");
}
```

### 2. Fix Ollama tool name sanitization (translate/ollama.rs)

The sanitization functions in `openai.rs` are private. Duplicate them locally in `ollama.rs`
with a comment pointing to the canonical definition:

```rust
// Tool name sanitization — Ollama uses the same OpenAI-compatible wire format, which
// requires names matching ^[a-zA-Z0-9_-]+$. Dotted names (e.g. MCP server tools like
// `chain.balance`) are encoded as `chain__DOT__balance` and reversed on parse.
// Canonical definition lives in translate/openai.rs.
fn sanitize_tool_name(name: &str) -> String {
    if name.contains('.') {
        name.replace('.', "__DOT__")
    } else {
        name.to_string()
    }
}

fn unsanitize_tool_name(name: &str) -> String {
    if name.contains("__DOT__") {
        name.replace("__DOT__", ".")
    } else {
        name.to_string()
    }
}
```

In `render_tools()`, change the `"name"` field from `t.name` to `sanitize_tool_name(&t.name)`.

In `parse_calls()`, after extracting the raw function name, apply `unsanitize_tool_name`:
```rust
let name = call
    .pointer("/function/name")
    .and_then(|v| v.as_str())
    .ok_or_else(|| TranslatorError::Malformed("missing function.name".into()))?;
let name = unsanitize_tool_name(name);
```

Add tests in the `#[cfg(test)]` block in `translate/ollama.rs`:
- `render_tools_sanitizes_dotted_name` — `ToolDef` with name `"chain.balance"` renders as
  `"chain__DOT__balance"` in the tools array
- `render_tools_plain_name_unchanged` — `ToolDef` with name `"read_file"` is not modified
- `parse_calls_unsanitizes_dotted_name` — response JSON with function name
  `"chain__DOT__balance"` parses to `ToolCall` with `name == "chain.balance"`
- `parse_calls_plain_name_unchanged` — response with `"read_file"` parses unchanged

### 3. Verify and document render_assistant_message (translate/claude.rs and translate/react.rs)

Before writing any code, read the current state of each translator:

**If `OllamaTranslator` already implements `render_assistant_message`**: no code change needed.
Add a comment in the `impl Translator for OllamaTranslator` block noting it is intentional
and what shape it returns.

**If `AnthropicTranslator` (in `anthropic_api/tool_loop.rs`) already implements
`render_assistant_message`**: no code change needed there either.

For `ClaudeTranslator` in `translate/claude.rs` — Claude CLI manages its own tool loop
internally. The outer `ToolLoop` never calls `render_assistant_message` for backends that
return `RenderedResults::HandledByBackend`. Add an explicit override that returns `None` with a
doc comment rather than silently inheriting the default:

```rust
/// Claude CLI manages its own multi-turn loop. The ToolLoop never calls this
/// for `HandledByBackend` backends — returning `None` here is intentional and correct.
fn render_assistant_message(&self, _response: &BackendResponse) -> Option<serde_json::Value> {
    None
}
```

For `ReActTranslator` in `translate/react.rs` — ReAct reconstructs full conversation context
from text prompts on each turn. There is no structured assistant message to inject. Add the
same pattern: explicit `None` override with a doc comment explaining why.

If implementations ARE missing and the audit is current (OllamaTranslator or
AnthropicTranslator returning `None` from the default): implement them following the pattern
in `translate/openai.rs` `render_assistant_message`. For Ollama, the assistant message is
`json.get("message").cloned()`. For the Anthropic API translator, it is
`json!({ "role": "assistant", "content": content_array })`.

**Document your findings in the Status Log**: state whether these were already implemented
(audit stale) or were missing (audit current) and what you changed.

### 4. Anthropic API adapter — wire or mark

Read `crates/roko-agent/src/provider/anthropic_api.rs` and
`crates/roko-agent/src/provider/anthropic_api/tool_loop.rs` fully.

If the adapter has existing passing tests (check with `cargo test -p roko-agent -- anthropic`):
choose **Option A — mark as experimental** by adding a `//!` doc comment at the top of
`anthropic_api.rs`:

```rust
//! Adapter for the Anthropic Messages API (direct HTTP, not Claude CLI subprocess).
//!
//! # Status: experimental — implemented and tested, not wired to production models
//!
//! This adapter implements the full Anthropic Messages API tool loop via HTTP
//! requests to `https://api.anthropic.com/v1/messages`. It is not used in production
//! because all claude models in `roko.toml` route through `claude_cli` (subprocess).
//!
//! ## To activate
//!
//! Add a provider entry to `roko.toml`:
//! ```toml
//! [providers.anthropic_api_direct]
//! kind = "anthropic_api"
//! base_url = "https://api.anthropic.com/v1"
//! api_key_env = "ANTHROPIC_API_KEY"
//! timeout_ms = 120000
//! ttft_timeout_ms = 15000
//! connect_timeout_ms = 5000
//! ```
//! Then add a model entry pointing to it (with `tool_format = "anthropic_blocks"`).
//! Run `cargo test -p roko-agent -- anthropic` to verify the adapter before enabling.
```

If the adapter's tests are failing or the adapter is incomplete: choose **Option B — wire to
a test model** by adding both the provider entry above and a `[models.claude-sonnet-api]`
entry in `roko.toml`, then fix whatever is broken until `cargo test -p roko-agent -- anthropic`
passes.

Document the choice and rationale in the Status Log.

## What NOT to Do

- Do NOT change the `Translator` trait's default `render_assistant_message` — the `None`
  default is intentional for backends that do not support it.
- Do NOT change `GeminiTranslator::render_assistant_message` — it already implements this.
- Do NOT change `OpenAiTranslator::render_assistant_message` — already correct.
- Do NOT make `sanitize_tool_name` / `unsanitize_tool_name` public in openai.rs. Duplicate
  the two-line functions in ollama.rs with a comment pointing to the source.
- Do NOT apply tool name sanitization to `ClaudeTranslator` — Claude CLI accepts dotted
  names natively and does not use the OpenAI wire format.
- Do NOT change roko.toml `[providers.gemini]` base_url to remove the path suffix — the
  idempotent code fix (stripping the suffix before appending) is preferred because it makes
  the code robust to both URL forms without requiring config changes.
- Do NOT touch the `Usage` struct or `extract_usage()` in translate/mod.rs — that is task 074.

## Wire Target

```bash
# Verify Gemini URL construction is idempotent:
cargo test -p roko-agent -- compat_base_url_idempotent
cargo test -p roko-agent -- gemini_tool_loop_base_url_idempotent

# Verify Ollama sanitization round-trip:
cargo test -p roko-agent -- render_tools_sanitizes_dotted
cargo test -p roko-agent -- parse_calls_unsanitizes_dotted

# Verify Anthropic adapter status (existing tests should still pass):
cargo test -p roko-agent -- anthropic

# Verify Claude and ReAct translators compile with explicit None override:
cargo test -p roko-agent -- claude
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo test -p roko-agent -- compat_base_url` — idempotency test passes
- [ ] `cargo test -p roko-agent -- gemini_tool_loop_base_url` — idempotency test passes
- [ ] `cargo test -p roko-agent -- ollama` — new sanitization tests pass, existing tests unaffected
- [ ] `cargo test -p roko-agent -- anthropic` — existing adapter tests pass
- [ ] `gemini_tool_loop_base_url("...googleapis.com/v1beta/openai")` equals `gemini_tool_loop_base_url("...googleapis.com")` — confirmed by test
- [ ] `compat_base_url("...googleapis.com/v1beta/openai")` equals `compat_base_url("...googleapis.com")` — confirmed by test
- [ ] `ClaudeTranslator` has explicit `fn render_assistant_message` returning `None` with doc comment
- [ ] `ReActTranslator` has explicit `fn render_assistant_message` returning `None` with doc comment
- [ ] `anthropic_api.rs` has `//!` doc comment stating experimental status and activation instructions
- [ ] Status Log documents: (a) whether render_assistant_message was already implemented for Ollama/Anthropic (audit stale or current), (b) which Anthropic API option was chosen and why

## Implementation Detail

### Current Code Facts to Account For

- Gemini has two URL helpers: `crates/roko-agent/src/gemini/compat.rs::compat_base_url` and `crates/roko-agent/src/gemini/adapter.rs::gemini_tool_loop_base_url`. Both currently append OpenAI-compat suffixes unconditionally.
- `roko.toml` currently configures `providers.gemini` as `kind = "openai_compat"` with `base_url` already ending in `/v1beta/openai`; current model routing does not exercise `GeminiAdapter` unless a provider is changed to `gemini_api`.
- `OllamaTranslator` already implements `render_assistant_message`, but it does not sanitize dotted tool names in `render_tools` or unsanitize them in `parse_calls`.
- `AnthropicTranslator` in `provider/anthropic_api/tool_loop.rs` already implements assistant message rendering and has adapter tests. Prefer marking Anthropic API experimental in docs/config comments over rewiring production config unless those tests are failing.

### Mechanical Implementation Steps

1. Make both Gemini URL helpers idempotent. Normalize by trimming trailing slashes, then stripping `/v1beta/openai/v1` first if present, then `/v1beta/openai`, before appending the helper-specific suffix. This covers bare host, OpenAI-compat root, and full chat base URL inputs.
2. Add unit tests beside each helper for at least: bare `https://generativelanguage.googleapis.com`, existing `/v1beta/openai`, existing `/v1beta/openai/`, and existing `/v1beta/openai/v1`.
3. In `translate/ollama.rs`, add local `sanitize_tool_name`/`unsanitize_tool_name` helpers matching the OpenAI translator's dot mapping behavior. Keep them private and add a short comment pointing to the OpenAI translator as the canonical matching behavior.
4. Use `sanitize_tool_name(&t.name)` when rendering Ollama tool definitions. In `parse_calls`, unsanitize the raw returned tool name before constructing `ToolCall`.
5. Add Ollama tests for dotted names and plain names in both directions: render dotted -> encoded form, parse encoded -> dotted form, and plain names unchanged.
6. Add explicit `render_assistant_message` overrides returning `None` in `ClaudeTranslator` and `ReActTranslator` with a short comment explaining that these formats intentionally do not replay assistant messages. This makes the default behavior an audited choice.
7. Add a module-level doc comment at the top of `crates/roko-agent/src/provider/anthropic_api.rs` marking the Anthropic Messages API adapter as experimental but implemented/tested. Include the provider kind (`anthropic_api`), expected `tool_format = "anthropic_blocks"`, and base URL guidance.

### Tests to Add or Update

- Gemini: helper idempotency tests in both `gemini/compat.rs` and `gemini/adapter.rs`; update existing request-path tests only if the normalized helper changes expected setup, not the final request path.
- Ollama: add focused tests to the existing `translate::ollama` test module; do not require a live Ollama server.
- Claude/ReAct: add small tests only if there is an existing translator default-behavior test area; otherwise the explicit method bodies are enough.
- Anthropic API: run existing adapter/tool-loop tests first. If they fail due to adapter incompleteness, fix those tests or choose the task's Option B deliberately; do not silently leave the status ambiguous.

### Expected Observable Behavior

- Gemini base URLs no longer duplicate `/v1beta/openai` or `/v1` regardless of whether config gives the bare host or an already-normalized compatibility URL.
- Ollama tool names with dots round-trip through render/parse while preserving the original internal tool id.
- Anthropic API status is explicit to future implementers and config authors.

### Additional Verification Commands

- `cargo test -p roko-agent -- compat_base_url`
- `cargo test -p roko-agent -- gemini_tool_loop_base_url`
- `cargo test -p roko-agent -- ollama`
- `cargo test -p roko-agent -- anthropic`
- `cargo test -p roko-agent -- translate`

### Additional What NOT To Do

- Do not switch existing Gemini config from `openai_compat` to `gemini_api` as part of URL normalization.
- Do not make OpenAI translator sanitize helpers public just to reuse them in Ollama; duplicate the tiny mapping to avoid widening API surface.
- Do not remove Anthropic API code or tests because it is experimental.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
| 2026-05-05 | claude | Implemented all 4 changes. (1) Gemini URL idempotency: both `compat_base_url` and `gemini_tool_loop_base_url` now strip `/v1beta/openai/v1` and `/v1beta/openai` before appending. Added 3 tests each. (2) Ollama tool sanitization: added local `sanitize_tool_name`/`unsanitize_tool_name`, wired into `render_tools` and `parse_calls`. Added 4 tests. (3) render_assistant_message: OllamaTranslator and AnthropicTranslator already implement it (audit GAP-I-39 is STALE). Added explicit `None` overrides with doc comments to ClaudeTranslator and ReActTranslator. (4) Anthropic API: chose Option A (mark experimental). Adapter has full tests passing. Added module-level `//!` doc comment with status and activation instructions. |
