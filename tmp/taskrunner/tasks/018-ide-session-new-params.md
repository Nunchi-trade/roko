# Task 018: IDE/ACP SessionNewParams Extension

```toml
id = 18
title = "Add model/provider/effort to SessionNewParams and wire into session creation"
track = "ide-acp"
wave = "wave-1"
priority = "high"
blocked_by = [2]
touches = [
    "crates/roko-acp/src/types.rs",
    "crates/roko-acp/src/session.rs",
]
exclusive_files = [
    "crates/roko-acp/src/types.rs",
    "crates/roko-acp/src/session.rs",
]
estimated_minutes = 30
```

## Context

`session/new` drops the model/provider/effort parameters (BUG#02) and IDE-selected model
state is not reliably reflected in the created ACP session (BUG#15).
SessionNewParams needs model/provider/effort fields, and session creation must use them.

Sources:
- `tmp/solutions/ide/CHECKLIST.md` — Agent 2D: SessionNewParams extension
- `tmp/solutions/ide/batches/W1-A-session-new-params.md` — detailed FIND/REPLACE

## Background

Read:
- `crates/roko-cli/src/main.rs` — ACP command wiring. The real entrypoint is
  `roko acp`, not `agent serve`.
- `crates/roko-acp/src/handler.rs` — `session/new` handling and slash-command
  notification.
- `crates/roko-acp/src/types.rs` — `SessionNewParams` and `SessionNewResult`.
- `crates/roko-acp/src/session.rs` — `SessionManager::create_session`,
  `AcpSession::new_with_config`, config-option construction, and tests.

The IDE solution docs have EXACT line numbers: `tmp/solutions/ide/batches/W1-A-session-new-params.md`

Current branch note: parts of this task may already be present. Verify behavior and add missing
tests/hardening instead of duplicating override logic.

## What to Change

1. **Add/keep `model`, `provider`, `effort` fields on `SessionNewParams`** in
   `types.rs`. Each field must be `Option<String>` and use
   `#[serde(default, skip_serializing_if = "Option::is_none")]`.
2. **Add/keep `warnings: Vec<String>` on `SessionNewResult`** with
   `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.
3. **Apply overrides when constructing `AcpSession`**, before the initial
   `configOptions` are returned. Preferred location is a small helper called from
   `AcpSession::new_with_config`; do not scatter the logic between
   `SessionManager::create_session` and result construction.
4. **Rebuild config options after overrides** so the `session/new` result reflects
   the selected model/provider/effort, not the default profile values.
5. **Return warnings for invalid overrides** rather than failing `session/new`.
   Invalid input should leave the profile/default value in place.

## Runtime Call Chain

1. `cargo run -p roko-cli -- acp ...` enters `Command::Acp` in
   `crates/roko-cli/src/main.rs`.
2. `main.rs` builds `roko_acp::AcpConfig` and calls `roko_acp::run_acp_server`.
3. `crates/roko-acp/src/handler.rs` handles JSON-RPC method `session/new`.
4. The handler deserializes `SessionNewParams`, then calls
   `SessionManager::create_session`.
5. `SessionManager::create_session` constructs `AcpSession` via
   `AcpSession::new_with_config`.
6. `AcpSession::new_with_config` must apply model/provider/effort overrides before
   `build_config_options` and before `new_result()`.
7. The handler returns `SessionNewResult` and then sends the available slash commands
   notification.

## Mechanical Implementation Notes

- Trim string overrides and ignore empty strings with a warning.
- Apply `model` first. If valid, set the session model and the model's provider from
  the active config/profile.
- Apply `provider` second. Provider is allowed to override the provider implied by the
  model. If the selected model is not available for the requested provider, choose that
  provider's first configured model when available; otherwise keep/clear the prior model
  and add a warning.
- When selecting a fallback model for a provider, use the same deterministic model order
  used by `configOptions`; do not rely on raw `HashMap` iteration.
- Apply `effort` last. Accept only `low`, `medium`, `high`, and `max` unless the
  existing config schema already exposes a stricter enum/helper. Invalid effort adds a
  warning and keeps the previous value.
- Keep warning text deterministic enough for tests to assert key substrings such as the
  invalid model/provider/effort value.
- Keep serde compatibility: older IDE clients that omit the fields must still create a
  session successfully.

## Tests to Add or Update

- In `crates/roko-acp/src/session.rs` tests, add focused unit tests for:
  - valid `model` override updates the returned `configOptions` current value;
  - valid `provider` override takes precedence over the model-implied provider;
  - invalid `model` leaves defaults in place and returns a warning;
  - invalid `provider` leaves defaults in place and returns a warning;
  - invalid `effort` leaves defaults in place and returns a warning.
- If the existing ACP shell harness is used, update/add a case near
  `tmp/solutions/ide/tests/test-models.sh` rather than introducing a second ad-hoc
  protocol driver.

## What NOT to Do

- Don't change the ACP protocol format.
- Don't touch MCP handling (that's task 019+).
- Don't implement the override only in the JSON response; the live `AcpSession` state
  must use the same values.
- Don't reintroduce `agent serve`; the ACP command is `roko acp`.
- Don't fail `session/new` for unknown model/provider/effort values.

## Wire Target

```bash
# Test that session/new respects ACP params.
TMP_CONFIG="$(mktemp)"
cat >"$TMP_CONFIG" <<'EOF'
config_version = 2
schema_version = 2

[project]
name = "acp-session-new-test"

[serve]
port = 6699

[agent]
command = "cat"
model = "test-fast"

[providers.openai]
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.anthropic]
kind = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[models.test-fast]
provider = "openai"
slug = "gpt-4o-mini"
supports_tools = true
context_window = 128000
max_output = 16000

[models.test-deep]
provider = "anthropic"
slug = "claude-sonnet-4-20250514"
supports_tools = true
context_window = 200000
max_output = 16000
EOF

printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"model":"test-deep","provider":"anthropic","effort":"high"}}' |
  cargo run -p roko-cli -- acp --quiet --no-serve --config "$TMP_CONFIG"
```

Expected observable behavior: the `session/new` result includes no warnings for the
valid request, and its `configOptions` current/default values reflect `test-deep`,
`anthropic`, and `high`. Re-run with an invalid model and expect a non-empty
`result.warnings` array while the session still succeeds.

## Verification

- [ ] `cargo test -p roko-acp session_new -- --nocapture`
- [ ] `cargo test -p roko-acp config_options -- --nocapture`
- [ ] `cargo build -p roko-cli -p roko-acp`
- [ ] Session created with model param uses that model, not default
- [ ] Invalid params produce warnings and do not fail session creation

## Status Log

| Time | Agent | Action |
|------|-------|--------|
