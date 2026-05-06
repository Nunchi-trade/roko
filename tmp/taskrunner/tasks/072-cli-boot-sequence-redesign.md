# Task 072: CLI Boot Sequence Redesign

```toml
id = 72
title = "Redesign CLI boot sequence: accurate banner provider label, session-resolved /provider slash command"
track = "terminal-chat"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-cli/src/chat_inline.rs",
    "crates/roko-cli/src/inline/terminal.rs",
    "crates/roko-cli/src/chat.rs",
]
exclusive_files = ["crates/roko-cli/src/inline/terminal.rs"]
estimated_minutes = 180
```

## Context

This task redesigns the CLI boot sequence to close four Phase 0 gaps from `tmp/redesign-plan.md`.
**Read the code before writing anything.** Several gaps mentioned in the redesign plan are already
implemented. The remaining work is more subtle and is described precisely below.

Sources:
- `tmp/redesign-plan.md` lines 938-1155 — Phase 0 full description
- `tmp/infrastructure-audit.md` sections 1-3 — dev workflow and workspace lifecycle audit

## Background

Read these files before writing any code:

1. `crates/roko-cli/src/inline/terminal.rs` — `RawModeGuard` (lines 25-52) and `InlineTerminal`
   (lines 56-242). Already has:
   - `RawModeGuard` with `Drop` impl that calls `disable_raw_mode()` (lines 48-52)
   - Panic hook installed in `InlineTerminal::new()` (lines 85-91): restores raw mode + shows cursor
   - `_raw_guard: RawModeGuard` held in `InlineTerminal` (line 63): RAII guard for the lifetime
   **Phase 0.1 is fully implemented. Do not touch this.**

2. `crates/roko-cli/src/chat_inline.rs` — Two entry points:
   - `run_chat_inline()` (line ~1167): HTTP/sidecar mode, used by `roko agent chat`
   - `run_unified_inline()` (line ~1552): main entry for `roko` with no args
   Both entry points already handle `Ctrl+C` in `Phase::Error` (lines 1323-1328 and 1696-1700):
   `KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL)` → `Phase::Done`.
   **Phase 0.2 is fully implemented. Do not touch Ctrl+C handling.**

3. `crates/roko-cli/src/unified.rs` — `cmd_unified_chat()`. Calls `detect_auth_from_config(&workdir)`
   (line 38) then checks `AuthMethod::NeedsSetup` (line 39): prints setup instructions and returns
   `Ok(1)`. **Phase 0.3 startup provider validation is already implemented.**

4. `crates/roko-cli/src/auth_detect.rs` — `detect_auth_from_config()` (line 65): loads `roko.toml`
   and checks each configured provider's credentials in order. Falls back to env-var detection.
   This is config-aware and correct. **Phase 0.6 config-aware auth detection is already wired.**

5. `crates/roko-cli/src/chat_inline.rs` line 1586 — **The actual gap**:
   ```rust
   &format!("v{version}  {}  {}", symbols::SEP, auth.label()),
   ```
   The banner uses `auth.label()` — the value from `detect_auth_from_config()`. But
   `run_unified_inline()` then calls `build_unified_inline_agent_session()` (line 1563) which
   resolves the model via `resolve_effective_model()` and stores it in a `ChatAgentSession`.
   The session's `model_selection: EffectiveModelSelection` is the ground truth for what dispatch
   will actually use. The banner should show `model_selection.display_line()` or a compact form
   of it, not the upstream `auth.label()` value which may differ when `roko.toml` specifies a
   `default_backend` or specific model.

6. `crates/roko-cli/src/chat_inline.rs` lines 2881-2891 — The `/provider` and `/auth` slash
   commands for `DispatchMode::Session` already read `agent_session.model_selection.provider_key`
   and `provider_kind`. But `current_model_name()` (line ~927) returns `"session"` for
   `DispatchMode::Session`. The banner at line 1586 is the primary gap.

7. `crates/roko-cli/src/model_selection.rs` — `EffectiveModelSelection` struct (line 55):
   - `effective_model_key: String` — model slug, e.g. `"claude-sonnet-4-6"`
   - `provider_key: String` — provider registry key, e.g. `"claude_cli"`
   - `provider_kind: String` — provider family label, e.g. `"claude-cli"`
   - `display_line() -> String` — canonical rendering: `"model: X via Y (source: Z)"`

8. `crates/roko-cli/src/chat_session.rs` — `ChatAgentSession` (line ~372):
   - `pub model: String` — mutable model string for slash commands
   - `pub model_selection: EffectiveModelSelection` — resolved identity from boot

## What to Change

### 1. Surface the session-resolved model in the banner

In `run_unified_inline()` (line ~1569 onwards in `chat_inline.rs`), `build_unified_inline_agent_session()`
is called before `InlineTerminal::new()`. The returned `agent_session` has `model_selection` with the
full resolution context. Replace the `auth.label()` call in the banner (line 1586) with a compact label
derived from the session's resolved model selection.

Add a free function `session_banner_label(selection: &EffectiveModelSelection) -> String` near the
banner-building code:

```rust
/// Format the resolved model selection for display in the startup banner.
///
/// Returns a compact label like `"claude-sonnet-4-6 (claude-cli)"` that reflects
/// what dispatch will actually use, not what auth detection reported upstream.
fn session_banner_label(selection: &EffectiveModelSelection) -> String {
    format!("{} ({})", selection.effective_model_key, selection.provider_kind)
}
```

Replace the banner line:
```rust
// Before (line ~1586):
&format!("v{version}  {}  {}", symbols::SEP, auth.label()),

// After:
&format!("v{version}  {}  {}", symbols::SEP, session_banner_label(&agent_session.model_selection)),
```

The `auth` parameter is still needed by `run_unified_inline()` for the `DispatchMode::Direct`
fallback path (if it exists). Only the banner line changes.

### 2. Fix `current_model_name()` for `DispatchMode::Session`

`current_model_name()` (line ~927) returns `"session"` for `DispatchMode::Session`. This is used
in the status bar and other display contexts. Update it to read from `agent_session`:

```rust
DispatchMode::Session => session
    .agent_session
    .as_ref()
    .map(|s| s.model.clone())
    .unwrap_or_else(|| "session".to_string()),
```

The `session.agent_session` field is `Option<ChatAgentSession>`. The `model` field holds the
mutable current model slug (updated by `/model` switches). This is the correct value to display.

### 3. Add startup debug log for model resolution

In `build_unified_inline_agent_session()` (line ~1516 in `chat_inline.rs`), after model resolution
succeeds and `agent_session` is built, add a `tracing::debug!()` call:

```rust
tracing::debug!(
    model = %agent_session.model_selection.effective_model_key,
    provider = %agent_session.model_selection.provider_key,
    source = %agent_session.model_selection.source,
    "resolved effective model for chat session"
);
```

This is not user-visible but makes `RUST_LOG=roko_cli=debug cargo run -p roko-cli -- chat`
diagnosable when the banner shows an unexpected provider.

### 4. Verify Ctrl+C in ALL phases matches the spec

Audit every `Phase::*` arm in both event loops in `chat_inline.rs` that has `_ => {}` and
confirm it either:
- Explicitly matches `KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL)` and
  transitions to `Phase::Done`, OR
- Delegates to a handler function that does so

If any phase is missing Ctrl+C coverage, add it. Document what you found in the Status Log.
(Based on reading the code, `Phase::Input` handles Ctrl+C via `handle_input_key`, and
`Phase::Thinking`, `Phase::Streaming`, `Phase::Error` all have explicit Ctrl+C → `Phase::Done`.
Audit confirms this; add only if something is missing.)

## What NOT to Do

- Do NOT touch `RawModeGuard` or the panic hook in `terminal.rs` — already correctly implemented.
- Do NOT change Ctrl+C handling in any `Phase` unless the audit in step 4 finds a missing case.
- Do NOT change `detect_auth_from_config` — already config-aware and correct.
- Do NOT change the `auth` parameter threading from `cmd_unified_chat()`.
- Do NOT reimport or duplicate auth detection logic — use what is in `auth_detect.rs`.
- Do NOT add new crates or dependencies.
- Do NOT change `run_chat_inline()` (the HTTP/sidecar entry for `roko agent chat`) — it does not
  use `ChatAgentSession` and is out of scope.
- Do NOT add a `RokoBootstrap` struct — `unified.rs` already has `RokoBootstrap::new()` from
  `bootstrap.rs`. Use what exists.

## Wire Target

```bash
# Run unified inline chat — check the banner shows the actual resolved model,
# e.g. "claude-sonnet-4-6 (claude-cli)" not just "claude CLI"
cargo run -p roko-cli -- chat

# Verify the /provider slash command in the REPL shows the same model label
# as the banner (not "session")
# Type /provider then Enter in the REPL

# Verify the tracing debug log when RUST_LOG=debug
RUST_LOG=roko_cli=debug cargo run -p roko-cli -- chat 2>&1 | head -20
# Expected: "resolved effective model for chat session" log line before banner appears

# Verify current_model_name shows the session model in the status bar
# (visible in the bottom status bar during input phase)
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] Banner in `run_unified_inline()` shows `session_banner_label()` output, not `auth.label()`
- [ ] `grep -n 'auth\.label()' crates/roko-cli/src/chat_inline.rs` — does not appear in the
  banner line (~1586). Only appears in `DispatchMode::Direct` arms (expected).
- [ ] `current_model_name()` for `DispatchMode::Session` returns `agent_session.model`, not `"session"`
- [ ] `RUST_LOG=roko_cli=debug cargo run -p roko-cli -- chat` shows debug log line with
  model key, provider key, and source before the banner
- [ ] `/provider` slash command output is consistent with the banner label (both reflect the
  session-resolved model+provider)
- [ ] `run_chat_inline()` (HTTP mode) is unchanged — no regression
- [ ] All `Phase::*` arms have Ctrl+C coverage (audit documented in Status Log)
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in changed files

## Implementation Detail

### Current Code Facts to Account For

- `crates/roko-cli/src/inline/terminal.rs` already has `RawModeGuard`, disables raw mode in `Drop`, and installs a panic hook in `InlineTerminal::new`. Audit it, but do not duplicate this guard unless a real uncovered path is found.
- `crates/roko-cli/src/unified.rs` already runs `detect_auth_from_config(&workdir)` before launching inline chat and exits with setup guidance on `NeedsSetup`.
- `crates/roko-cli/src/chat_inline.rs::active_model_name` already returns `agent_session.model` when an agent session exists. The remaining display gap is the unified startup banner still using `auth.label()` and the `/provider` command using provider-only text for session mode.

### Mechanical Implementation Steps

1. In `chat_inline.rs`, add one small helper near the existing model/banner helpers, for example `session_banner_label(selection: &EffectiveModelSelection) -> String`, that formats the resolved model key plus provider kind from the loaded model selection.
2. In `build_unified_inline_agent_session`, after `resolve_effective_model(...)` succeeds and before returning `ChatAgentSession`, emit a `tracing::debug!` record with the effective model key, provider key, provider kind, and source if available.
3. In `run_unified_inline`, keep the existing order: build `agent_session` before `InlineTerminal::new(auth)?`. Replace the banner's `auth.label()` text with the resolved session label from the agent session/model selection so the first visible line reflects the actual configured model.
4. Update the `/provider` command path in `handle_slash_command` for `DispatchMode::Session` to use the same resolved model/provider label or a shared helper. Direct mode can continue using the auth-derived label.
5. Add focused unit tests in the existing `chat_inline.rs` test module using `make_session`/`make_agent_session`: one for the banner/helper formatting, one proving `active_model_name` returns `agent_session.model` for session dispatch, and one for the `/provider` formatting helper if you extract it.
6. Re-audit Ctrl+C paths after the display changes: `handle_input_key` covers search/palette/normal input, and the main loop covers thinking/streaming/error phases. Only change these paths if a manual reproduction shows terminal state is still leaked.

### Expected Observable Behavior

- With a project `roko.toml`, `roko` shows the resolved model/provider in the banner, not just `Claude CLI`, `OpenAI`, or `session`.
- `/provider` in unified session mode reports the same resolved model/provider identity as the banner.
- If model resolution fails, the user sees the pre-terminal setup/error path and the terminal is not left in raw mode.

### Additional Verification Commands

- `cargo test -p roko-cli chat_inline`
- `cargo test -p roko-cli auth_detect`
- `cargo run -p roko-cli -- --help`
- Manual terminal check: run `cargo run -p roko-cli`, start a streaming request with a mock or low-risk provider, press Ctrl+C, then confirm `stty -a` is sane in the same terminal.

### Additional What NOT To Do

- Do not move terminal initialization earlier than agent/model resolution.
- Do not replace the existing auth detection with provider-specific shell probes in the UI path.
- Do not add broad integration tests that require a real TTY or live provider credentials for CI.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
