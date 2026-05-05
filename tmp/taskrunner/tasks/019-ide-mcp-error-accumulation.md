# Task 019: IDE/ACP MCP Error Accumulation

```toml
id = 19
title = "Accumulate MCP server errors as structured status instead of silent failure"
track = "ide-acp"
wave = "wave-1"
priority = "high"
blocked_by = [18]
touches = [
    "crates/roko-acp/src/bridge_events.rs",
    "crates/roko-acp/src/types.rs",
]
exclusive_files = [
    "crates/roko-acp/src/bridge_events.rs",
    "crates/roko-acp/src/types.rs",
]
estimated_minutes = 60
```

## Context

MCP spawn failures are completely silent (BUG#01). When an MCP server fails to start, the
error is swallowed and the tool just doesn't appear. Need structured error accumulation
with typed status per MCP server.

Sources:
- `tmp/solutions/ide/CHECKLIST.md` — Agent 2F + 3G: MCP status types + error accumulation
- `tmp/solutions/ide/batches/W2-A-mcp-status-types.md` and `W2-B-mcp-error-accumulation.md`

## Background

Read:
- `crates/roko-acp/src/types.rs` — `McpServerConfig`, `McpServerStatus`,
  `McpInitStatus`, and `SessionUpdate::McpStatusUpdate`.
- `crates/roko-acp/src/handler.rs` — `session/prompt` handling and outbound
  `session/update` notifications.
- `crates/roko-acp/src/bridge_events.rs` — `run_openai_compat_mcp_tool_loop`,
  `setup_session_mcp_tools`, `CognitiveEvent::McpStatus`, and
  `map_event_to_update`.
- `tmp/solutions/ide/tests/test-mcp.sh` — shell harness expectations.

The IDE solution docs have EXACT change specifications in the batch files.

Current branch note: structured MCP status and notifications may already exist. If so,
this task is to verify the failure paths and add/update tests. Do not remove
`mcp_status_update` notifications as "out of scope"; they are now part of the desired
observable behavior.

## What to Change

1. **Add/keep `McpServerStatus` and `McpInitStatus`** in `types.rs`.
   Required statuses: `ready`, `transport_unsupported`, `spawn_failed`,
   `initialize_failed`, `initialize_timeout`, `tools_list_failed`, and
   `tools_list_timeout`.
2. **Make MCP discovery return status alongside runtime tools.** The current shape should
   be equivalent to `setup_session_mcp_tools(...) -> (SessionMcpRuntime,
   Vec<McpServerStatus>)`.
3. **Record exactly one terminal status per configured MCP server** during discovery:
   unsupported transport, spawn failure, initialize failure, initialize timeout,
   tools/list failure, tools/list timeout, or ready with tool count.
4. **Emit the accumulated statuses after discovery** by sending a cognitive event that
   maps to `SessionUpdate::McpStatusUpdate`.
5. **Preserve successful tool use.** Ready servers must still register their tools and be
   usable by the OpenAI-compat loop.

## Runtime Call Chain

1. IDE sends `session/new` with optional `mcpServers`; this stores config only.
2. IDE sends `session/prompt`.
3. `handler.rs` routes the prompt to the session runtime.
4. `bridge_events.rs::run_openai_compat_mcp_tool_loop` starts prompt execution.
5. The loop calls `setup_session_mcp_tools`.
6. `setup_session_mcp_tools` attempts per-server MCP startup/discovery and accumulates
   `McpServerStatus` values.
7. The loop emits `CognitiveEvent::McpStatus { servers }`.
8. `map_event_to_update` converts that event to `SessionUpdate::McpStatusUpdate`.
9. The handler forwards it to the IDE as a `session/update` notification.

Important: MCP servers are not spawned by `session/new`. Wire tests must send a prompt
after creating the session.

## Mechanical Implementation Notes

- Use `discovery_timeout_ms` from `McpServerConfig` where present; do not replace the
  existing timeout behavior with a global constant.
- For HTTP/SSE transport that is still unsupported by this path, return
  `transport_unsupported` with a useful message.
- For stdio spawn errors, include the server name and a short error message in
  `McpServerStatus.message`; do not require tests to match OS-specific wording exactly.
- For initialize and tools/list timeout branches, classify timeout separately from other
  failures.
- Push ready status only after tools/list succeeds and the tool count is known.
- Keep logs as diagnostics only; the IDE-facing status must be structured JSON.

## What NOT to Do

- Don't change the MCP client itself (just add reporting around it).
- Don't fail `session/new` just because an MCP server is invalid.
- Don't silently drop a configured server without a status record.
- Don't put the failure only in logs or stderr.
- Don't block prompt execution when one MCP server fails and other work can continue.

## Wire Target

```bash
# Configure a nonexistent MCP binary, create a session, capture sessionId, then prompt.
# Expect a session/update notification with sessionUpdate = "mcp_status_update".
TMP_CONFIG="$(mktemp)"
cat >"$TMP_CONFIG" <<'EOF'
config_version = 2
schema_version = 2

[project]
name = "acp-mcp-status-test"

[serve]
port = 6699

[agent]
command = "cat"
model = "test"

[providers.openai]
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[models.test]
provider = "openai"
slug = "gpt-4o-mini"
supports_tools = true
context_window = 128000
max_output = 16000
EOF

# Implement this in the existing ACP shell harness rather than a one-shot pipe:
# 1. acp_start "$TMP_CONFIG"
# 2. acp_send initialize
# 3. acp_send session/new with:
#    {"mcpServers":[{"name":"missing-mcp","command":"definitely-not-a-real-mcp-binary"}]}
# 4. parse result.sessionId from response id 2
# 5. acp_send session/prompt with that sessionId
```

Expected observable behavior: after the prompt starts, the stream contains
`session/update` with `sessionUpdate: "mcp_status_update"` and a server status for
`missing-mcp` whose `status` is `spawn_failed`.

If using the shell harness, capture the `sessionId` from response id `2` before sending
the prompt; do not assert MCP failure during `session/new`.

## Tests to Add or Update

- Add/update unit coverage in `crates/roko-acp/src/bridge_events.rs` for:
  - HTTP transport config produces `transport_unsupported`;
  - nonexistent stdio command produces `spawn_failed`;
  - successful discovery produces `ready` with a nonzero/known tool count when a fake
    MCP server fixture is available;
  - `map_event_to_update(CognitiveEvent::McpStatus { ... })` returns
    `SessionUpdate::McpStatusUpdate`.
- Update `tmp/solutions/ide/tests/test-mcp.sh` or the ACP integration harness if it still
  expects "session creation failed" for nonexistent MCP. The correct behavior is
  successful session creation plus a later structured status notification.

## Verification

- [ ] `cargo test -p roko-acp mcp_status -- --nocapture`
- [ ] `cargo test -p roko-acp setup_session_mcp_tools -- --nocapture`
- [ ] `cargo build -p roko-acp -p roko-cli`
- [ ] MCP failures produce structured status, not silent drops
- [ ] Nonexistent MCP binary does not fail `session/new`
- [ ] `session/prompt` produces an IDE-visible `mcp_status_update`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
