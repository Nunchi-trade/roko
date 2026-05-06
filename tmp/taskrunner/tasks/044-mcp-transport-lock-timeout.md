# Task 044: Add Timeout to MCP StdioTransport Locks

```toml
id = 44
title = "Add per-call timeout to MCP StdioTransport stdin/stdout locks"
track = "infrastructure"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-agent/src/mcp/client.rs",
    "crates/roko-core/src/defaults.rs",
]
exclusive_files = [
    "crates/roko-agent/src/mcp/client.rs",
    "crates/roko-core/src/defaults.rs",
]
estimated_minutes = 60
```

## Context

`StdioTransport` in `crates/roko-agent/src/mcp/client.rs` holds `tokio::sync::Mutex`
locks on `stdin` and `stdout` across child-process I/O awaits (lines ~157-230). If the
MCP server child process hangs (e.g., deadlock, infinite loop), every concurrent
`roundtrip()` call blocks indefinitely waiting for the lock. The audit (S15.4)
identified this as a starvation risk.

## Background

Read:
- `crates/roko-agent/src/mcp/client.rs` — `StdioTransport` at lines 156-160,
  `spawn_with_env()` at lines 176-205, `Transport::roundtrip()` at lines 210-245,
  and the in-file MCP tests at lines 369-591.
- `crates/roko-core/src/defaults.rs` — timeout defaults live near the top;
  MCP discovery already has `DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS` at line 241,
  but there is no stdio write/read timeout default yet.
- Runtime callsites: `crates/roko-agent/src/mcp/bridge.rs:49-72`,
  `crates/roko-acp/src/bridge_events.rs:2038-2055`,
  `crates/roko-cli/src/worker/cloud.rs:395-401`, and legacy
  `crates/roko-cli/src/orchestrate.rs:4113-4185`.

The current flow:
1. Lock stdin mutex
2. Write JSON-RPC request to child stdin
3. Flush
4. Drop stdin lock
5. Lock stdout mutex
6. Read one line from child stdout (blocks if child hangs)
7. Drop stdout lock

Step 6 can block forever. All other callers queue behind the stdout lock.
Lock acquisition itself must be inside the timeout future; timing out only
after the lock is acquired still lets queued callers wait unboundedly behind a
hung call.

## What to Change

1. **Add a per-call timeout** to `roundtrip()`:
   - Add `DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS = 5` and
     `DEFAULT_MCP_RESPONSE_TIMEOUT_SECS = 30` to `roko_core::defaults`.
   - Wrap the stdout lock acquisition plus `read_line` in
     `tokio::time::timeout(Duration::from_secs(DEFAULT_MCP_RESPONSE_TIMEOUT_SECS), async { ... })`.
   - On timeout, return `McpError::Transport("MCP server response timed out after 30s")`.

2. **Add timeout to stdin write** as well:
   - Wrap stdin lock acquisition, `write_all`, and `flush` in one
     `tokio::time::timeout(Duration::from_secs(DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS), async { ... })`.
   - On timeout, return `McpError::Transport("MCP server stdin write timed out after 5s")`.
   - Prevents blocking if the child's stdin buffer is full.

3. **Drop locks before error returns** — ensure the lock guard is dropped before
   returning an error so subsequent calls aren't permanently blocked.

## Mechanical Implementation Notes

Use this shape so cancellation drops the mutex guard:

```rust
let write_result = tokio::time::timeout(MCP_STDIN_WRITE_TIMEOUT, async {
    let mut stdin = self.stdin.lock().await;
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;
    Ok::<(), std::io::Error>(())
}).await;
```

Apply the same pattern to stdout with a mutable `response_line` captured by the
async block. Map `Err(_)` from `timeout` to `McpError::Transport`, and map the
inner `std::io::Error` to the existing `"write to stdin"`, `"flush stdin"`, and
`"read from stdout"` transport messages. Do not hold a `MutexGuard` across a
`return Err(...)` branch outside the timeout block.

## Tests to Add

In `crates/roko-agent/src/mcp/client.rs`:
- Add a `#[tokio::test(start_paused = true)]` that spawns a child process which
  accepts stdin but never writes stdout, calls `roundtrip()`, advances Tokio time
  past `DEFAULT_MCP_RESPONSE_TIMEOUT_SECS`, and asserts the error is
  `McpError::Transport` containing `"response timed out after 30s"`.
- Add a cheap defaults test in `crates/roko-core/src/defaults.rs` asserting
  `DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS < DEFAULT_MCP_RESPONSE_TIMEOUT_SECS`.

Use existing Tokio `test-util` support in `roko-agent`; do not make the test
sleep for real wall-clock seconds.

## Expected Runtime Behavior

Runtime chain:

```text
MCP config/ACP/cloud worker/orchestrate
  -> StdioTransport::spawn[_with_env]()
  -> McpClient::initialize/list_tools/call_tool()
  -> McpClient::call()
  -> Transport::roundtrip()
  -> timed stdin write/flush and timed stdout read
```

A hung MCP process now fails one request after at most 30 seconds of response
wait, releases the stdout mutex, and allows later calls to make their own
attempt instead of waiting forever.

## What NOT to Do

- Don't redesign the transport architecture (e.g., no connection pooling, no
  multiplexing). This is a targeted fix.
- Don't add retry logic to `roundtrip()` — callers handle retries.
- Don't change the JSON-RPC protocol or message format.
- Don't wrap only `read_line()` after `self.stdout.lock().await`; include lock
  acquisition in the timeout future.
- Don't add real-time 30 second tests.

## Wire Target

```bash
cargo build --workspace
cargo test -p roko-agent -- mcp
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `roundtrip()` has explicit timeout on stdin write and stdout read
- [ ] Timeout returns `McpError::Transport` (not a panic)
- [ ] `grep -rn 'DEFAULT_MCP_RESPONSE_TIMEOUT_SECS\|DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS' crates/ --include='*.rs' | grep -v target/`
      shows definitions in `roko-core` and use from `roko-agent`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
