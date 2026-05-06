# Task 063: IDE/ACP MCP Status Notification + Discovery Timeout Config

```toml
id = 63
title = "Emit MCP status as structured session/update notification and add per-server discovery timeout"
track = "ide-acp"
wave = "wave-1"
priority = "high"
blocked_by = [19]
touches = [
    "crates/roko-acp/src/bridge_events.rs",
    "crates/roko-acp/src/types.rs",
]
exclusive_files = []
estimated_minutes = 40
```

## Context

After task 019 (MCP error accumulation), `setup_session_mcp_tools` returns
`(SessionMcpRuntime, Vec<McpServerStatus>)` but the statuses are not yet surfaced to the
IDE client. This task:

1. **W2-C**: Adds a `CognitiveEvent::McpStatus` variant, emits it after MCP setup, maps it to
   a `SessionUpdate::McpStatusUpdate` that reaches the IDE as a structured `session/update`
   notification.
2. **W2-D**: Adds `discovery_timeout_ms: Option<u64>` to `McpServerConfig` so each MCP server
   can have its own timeout instead of the hardcoded 5s default.

These two changes are grouped because they both modify the same two files and W2-D depends
on W2-B (which is task 019).

Sources:
- `tmp/solutions/ide/CHECKLIST.md` — Group 4: items 4.1-4.6
- `tmp/solutions/ide/batches/W2-C-mcp-notification.md` — notification changes
- `tmp/solutions/ide/batches/W2-D-mcp-config-options.md` — timeout config changes
- `tmp/solutions/ide/04-mcp-configuration.md` — original issue analysis

## Background

Read these files before starting:
- `crates/roko-acp/src/bridge_events.rs` — `CognitiveEvent` enum (lines 207-236),
  `run_openai_compat_mcp_tool_loop` (line ~1809), `map_event_to_update` (lines 3315-3354),
  `setup_session_mcp_tools` (after task 019 returns tuple)
- `crates/roko-acp/src/types.rs` — `SessionUpdate` enum (lines 401-480),
  `McpServerConfig` (lines 255-263), `McpServerStatus` + `McpInitStatus` (added by task 019)
- `crates/roko-core/src/defaults.rs` — `DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS = 5` (line 241)

The batch files have EXACT FIND/REPLACE blocks:
- `tmp/solutions/ide/batches/W2-C-mcp-notification.md`
- `tmp/solutions/ide/batches/W2-D-mcp-config-options.md`

## What to Change

### Part A: MCP Status Notification (W2-C)

#### A1. Add McpStatus variant to CognitiveEvent enum (bridge_events.rs)

After the `MaxTokens` variant (lines 234-236), add:
```rust
/// MCP server discovery results.
McpStatus(Vec<crate::types::McpServerStatus>),
```

#### A2. Emit McpStatus event after MCP setup (bridge_events.rs)

In `run_openai_compat_mcp_tool_loop`, after the `setup_session_mcp_tools` call returns
`(mcp_state, mcp_statuses)`, emit the structured notification BEFORE the "No MCP tools"
text fallback:

```rust
// Always emit structured MCP status notification (for IDE clients).
if !mcp_statuses.is_empty() {
    send_cognitive_event(
        &event_sender,
        CognitiveEvent::McpStatus(mcp_statuses),
    )
    .await;
}
```

Keep the existing "No MCP tools" text fallback for backward compatibility.

#### A3. Add McpStatusUpdate variant to SessionUpdate enum (types.rs)

Before the closing brace of `SessionUpdate` (lines 472-480), add:
```rust
/// MCP server discovery results.
McpStatusUpdate {
    /// Per-server status after initialization attempt.
    servers: Vec<McpServerStatus>,
},
```

#### A4. Handle McpStatus in map_event_to_update (bridge_events.rs)

Add a match arm before the terminal events (lines 3347-3353):
```rust
CognitiveEvent::McpStatus(servers) => SessionUpdate::McpStatusUpdate { servers },
```

### Part B: Per-Server Discovery Timeout (W2-D)

#### B1. Add discovery_timeout_ms to McpServerConfig (types.rs)

In the `McpServerConfig` struct (lines 255-263), add:
```rust
/// Per-server discovery timeout in milliseconds.
/// If absent, uses the system default (5000ms = `DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS`).
/// Applied independently to `initialize()` and `list_tools()` calls.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub discovery_timeout_ms: Option<u64>,
```

#### B2. Use per-server timeout in setup_session_mcp_tools (bridge_events.rs)

Move the `discovery_timeout` binding from outside the per-server loop to inside it, using the
per-server value when present:
```rust
// BEFORE (outside loop):
// let discovery_timeout = Duration::from_secs(DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS);
// for server in mcp_servers {

// AFTER (inside loop):
for server in mcp_servers {
    let discovery_timeout = server
        .discovery_timeout_ms
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS));
```

## What NOT to Do

- Do NOT modify `setup_session_mcp_tools` return type — that was done in task 019.
- Do NOT add a `session/mcp_status` query method (defer to follow-up).
- Do NOT modify the text fallback behavior ("No MCP tools were discovered...").
- Do NOT modify `SessionMcpRuntime` struct.
- Do NOT change `DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS` in defaults.rs (5s is fine as default).
- Do NOT add `eager_mcp_discovery` to SessionNewParams (defer to follow-up).

## Wire Target

```bash
# Send session/new with nonexistent MCP binary, look for mcp_status_update notification
echo '{"jsonrpc":"2.0","method":"session/new","id":1,"params":{"mcpServers":[{"name":"bad","transport":{"type":"stdio","command":"/nonexistent","args":[]}}]}}' \
  | timeout 10 cargo run -p roko-cli -- acp --quiet --no-serve 2>/dev/null \
  | grep -c 'mcp_status_update'
# EXPECTED: "1" (one notification with the failure status)

# Test per-server timeout override
echo '{"jsonrpc":"2.0","method":"session/new","id":1,"params":{"mcpServers":[{"name":"test","transport":{"type":"stdio","command":"/nonexistent","args":[]},"discoveryTimeoutMs":500}]}}' \
  | timeout 5 cargo run -p roko-cli -- acp --quiet --no-serve 2>/dev/null \
  | head -5
# EXPECTED: response within ~1s, not 5s
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] Failed MCP server produces `mcp_status_update` session/update notification
- [ ] Per-server `discoveryTimeoutMs` is respected (short timeout = faster failure)
- [ ] Servers without `discoveryTimeoutMs` use the 5s default
- [ ] Successful MCP servers also appear in the status with `"ready"` status

## Implementation Notes for Later Agent

Current branch facts to verify before editing:
- `McpServerStatus`, `McpInitStatus`, `McpServerConfig.discovery_timeout_ms`,
  `CognitiveEvent::McpStatus`, and `SessionUpdate::McpStatusUpdate` may already exist.
  If they do, align the existing code/tests instead of adding duplicate variants.
- The inspected branch uses struct variants and wire field `statuses`:
  `CognitiveEvent::McpStatus { statuses: Vec<McpServerStatus> }` and
  `SessionUpdate::McpStatusUpdate { statuses: Vec<McpServerStatus> }`. Do not add a
  parallel tuple variant or a second `servers` field unless the product contract is
  explicitly changed.
- MCP discovery is lazy. `session/new` stores `mcpServers`; actual discovery happens on
  the first `session/prompt` through:
  `handler.rs` -> `bridge_events.rs::handle_session_prompt` ->
  `dispatch_with_model_call_service` -> `maybe_run_openai_compat_mcp_tool_loop` ->
  `run_openai_compat_mcp_tool_loop` -> `setup_session_mcp_tools`.
  A wire target that sends only `session/new` does not exercise this task.
- `setup_session_mcp_tools` should return `(SessionMcpRuntime, Vec<McpServerStatus>)`.
  Keep status accumulation for unsupported transport, spawn failure, initialize failure,
  initialize timeout, tools/list failure, tools/list timeout, and ready success.

Mechanical steps:
1. In `types.rs`, add/verify `discovery_timeout_ms: Option<u64>` on
   `McpServerConfig` with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
   Because the struct uses `rename_all = "camelCase"`, the wire key is
   `discoveryTimeoutMs`.
2. In `bridge_events.rs::setup_session_mcp_tools`, compute `discovery_timeout` inside
   the per-server loop from `server.discovery_timeout_ms.map(Duration::from_millis)`
   or `Duration::from_secs(DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS)`. Use the same duration
   independently for both `client.initialize()` and `client.list_tools()`.
3. In `run_openai_compat_mcp_tool_loop`, emit a structured MCP status event immediately
   after setup returns and before the `"No MCP tools were discovered..."` token fallback.
   Do not return early before emitting statuses.
4. In `stream_events_to_editor`, non-terminal `CognitiveEvent`s are converted through
   `map_event_to_update`; ensure `McpStatus` takes that path and reaches
   `send_session_update`, producing a JSON-RPC `session/update` notification.
5. If `crates/roko-acp/src/event_forward.rs` maps `CognitiveEvent` to `RuntimeEvent`,
   preserve/update its `McpStatus` arm so runtime event forwarding still compiles.

Tests to add or update:
- `types.rs`: serde round-trip for `McpServerConfig` with `discoveryTimeoutMs`, and
  omitted field -> `None`.
- `types.rs` or `bridge_events.rs`: serialize `SessionUpdate::McpStatusUpdate` and
  assert `{"sessionUpdate":"mcp_status_update","statuses":[...]}`.
- `bridge_events.rs`: unit-test `map_event_to_update(CognitiveEvent::McpStatus { ... })`
  maps to `SessionUpdate::McpStatusUpdate`.
- `bridge_events.rs`: add an async test using a deliberately slow stdio command
  (`python3 -c 'import time; time.sleep(10)'`) with `discovery_timeout_ms = Some(200)`
  and assert the status is `InitializeTimeout` in under 2 seconds. A nonexistent binary
  tests spawn failure, not timeout behavior.

Corrected wire target:
```bash
# Discovery is triggered by session/prompt, not session/new. This script keeps
# one ACP process alive, creates a session with a bad MCP server, then prompts it.
python3 - <<'PY'
import json, subprocess, sys
p = subprocess.Popen(
    ["cargo", "run", "-p", "roko-cli", "--", "acp", "--quiet", "--no-serve"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
)
new = {"jsonrpc":"2.0","method":"session/new","id":1,"params":{
    "mcpServers":[{"name":"bad","transport":{"type":"stdio","command":"/nonexistent","args":[]},"discoveryTimeoutMs":500}]
}}
p.stdin.write(json.dumps(new) + "\n"); p.stdin.flush()
sid = json.loads(p.stdout.readline())["result"]["sessionId"]
prompt = {"jsonrpc":"2.0","method":"session/prompt","id":2,"params":{
    "sessionId": sid, "prompt":[{"type":"text","text":"hi"}]
}}
p.stdin.write(json.dumps(prompt) + "\n"); p.stdin.flush()
for _ in range(20):
    line = p.stdout.readline()
    if not line:
        break
    if "mcp_status_update" in line:
        print(line.strip())
        p.kill()
        sys.exit(0)
p.kill()
sys.exit("missing mcp_status_update")
PY
```

What not to do:
- Do not add eager discovery to `session/new`; that is explicitly deferred.
- Do not add a `session/mcp_status` method here.
- Do not treat `/nonexistent` as proof that `discoveryTimeoutMs` works; it bypasses the
  timeout path by failing at process spawn.
- Do not remove the text fallback for clients that do not understand the structured
  update.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
