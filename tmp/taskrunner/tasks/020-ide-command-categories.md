# Task 020: IDE/ACP Command Categories and bare_mode Filtering

```toml
id = 20
title = "Add category field to SlashCommand and filter by bare_mode"
track = "ide-acp"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-acp/src/types.rs",
    "crates/roko-acp/src/session.rs",
    "crates/roko-acp/src/handler.rs",
]
exclusive_files = []
estimated_minutes = 60
```

## Context

`bare_mode` shows all 47 commands instead of filtering (BUG#05). Need to add categories to
commands and filter based on bare_mode.

Sources:
- `tmp/solutions/ide/CHECKLIST.md` — Agent 3H: Command categories
- `tmp/solutions/ide/batches/W3-A-command-categories.md`

## Background

Read:
1. `crates/roko-acp/src/types.rs` — SlashCommand struct
2. `crates/roko-acp/src/session.rs` — `build_slash_commands()` function
3. `crates/roko-acp/src/handler.rs` — where commands are sent to IDE

Current branch note: `SlashCommand.category` and `build_slash_commands(bare_mode)` may
already exist. Verify the actual bare-mode set against the source batch; a broad
category allow-list that still exposes implementation/workflow commands does not satisfy
this task.

## What to Change

1. **Add/keep `category` on `SlashCommand`** in `types.rs`. Use
   `#[serde(default, skip_serializing_if = "Option::is_none")]` if the field remains
   optional for wire compatibility.
2. **Every command returned by `build_slash_commands(false)` must have a category.**
   Use a local helper constructor in `session.rs` to avoid hand-written field drift.
3. **Accept/keep `bare_mode: bool` in `build_slash_commands`.**
4. **Filter bare mode to the intended IDE-safe set only.** The source batch expects core
   and research commands, not the full implementation/workflow surface.
5. **Pass `config.bare_mode` through handler call sites** that send
   `available_commands_update` after `session/new`.

## Expected Category Model

Use the source batch naming unless the implementation already has an equivalent mapping:

- `core`: `status`, `doctor`, `config`, `help`
- `research`: `research`, `search`, `enhance-prd`, `analyze`
- hidden in bare mode: workspace/build/implementation commands, agent management,
  knowledge/learning commands, workflow/history commands, and spec/planning commands

The observable bare-mode command list must be exactly:

```text
status
doctor
config
help
research
search
enhance-prd
analyze
```

If keeping the current internal category strings, implement an explicit bare-mode
command whitelist with those eight names and keep the serialized category values
consistent. Do not allow a broad category such as `workflow`, `implementation`, or
`verification` in bare mode just because it sounds useful.

## Runtime Call Chain

1. `roko acp --config <config>` loads `AcpConfig` from CLI/config.
2. `handler.rs` handles `session/new`.
3. After creating the session, the handler calls
   `send_slash_commands_notification(..., bare_mode)`.
4. `send_slash_commands_notification` calls `build_slash_commands(bare_mode)`.
5. The IDE receives `session/update` with `sessionUpdate:
   "available_commands_update"`.

## Tests to Add or Update

- In `crates/roko-acp/src/session.rs`, add focused unit tests:
  - `slash_commands_have_category_for_every_command`;
  - `bare_mode_returns_exact_core_and_research_set`;
  - `full_mode_keeps_workspace_or_implementation_commands`;
  - `bare_mode_command_categories_are_serialized` if serde coverage is not already
    present.
- If adding protocol-level coverage, start ACP with `[agent] bare_mode = true`, send
  `session/new`, and inspect the `available_commands_update` notification.

## What NOT to Do

- Don't change command implementations.
- Don't add new commands.
- Don't change the JSON-RPC protocol.
- Don't hide commands in full mode.
- Don't leave any command with `category: None`.
- Don't filter only on the client side; the ACP notification itself must be filtered.

## Wire Target

```bash
# Start ACP in bare_mode and verify the available_commands_update notification contains
# only the eight commands listed above.
TMP_CONFIG="$(mktemp)"
cat >"$TMP_CONFIG" <<'EOF'
[agent]
bare_mode = true
EOF

printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}' |
  cargo run -p roko-cli -- acp --quiet --no-serve --config "$TMP_CONFIG"
```

Expected observable behavior: the `available_commands_update` notification contains
exactly `status`, `doctor`, `config`, `help`, `research`, `search`, `enhance-prd`, and
`analyze`, and each command object includes a category. Running without bare mode still
returns the full command set.

## Verification

- [ ] `cargo test -p roko-acp slash_commands -- --nocapture`
- [ ] `cargo build -p roko-acp -p roko-cli`
- [ ] bare_mode shows filtered command list
- [ ] full mode still shows the complete command list

## Status Log

| Time | Agent | Action |
|------|-------|--------|
