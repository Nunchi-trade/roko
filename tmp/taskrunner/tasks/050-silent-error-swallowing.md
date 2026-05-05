# Task 050: Fix Silent Error Swallowing in Serve Routes and Provider Adapters

```toml
id = 50
title = "Add warn!/error! logging to silent if-let-ok error swallowing"
track = "infrastructure"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-serve/src/routes/",
    "crates/roko-agent/src/provider/",
]
exclusive_files = []
estimated_minutes = 90
```

## Context

The audit (S6.2) identified a pervasive pattern where fallible operations are silently
ignored:
```rust
if let Ok(x) = fallible_op() { use(x) }
// failure silently ignored — no log, no error propagation
```

This pattern hides real failures behind silent no-ops, making debugging extremely
difficult. Users see "nothing happened" with no indication of why.

## Background

Use `rg` so the output is fast enough to review repeatedly:
```bash
rg -n "if let Ok\(|let _ =|\.ok\(\);" crates/roko-serve/src/routes crates/roko-agent/src/provider --glob '*.rs'
```

The current grep is intentionally noisy. Ignore matches in `#[cfg(test)]` test
modules and normal optional environment lookups where absence is expected, but
classify every production match before changing it.

High-priority production candidates observed during enrichment:

| File | Function/area | Required handling |
| --- | --- | --- |
| `crates/roko-agent/src/provider/openai_compat.rs` | `inject_provider_routing` | Serialization failure should at least `warn!` with provider/model context; only return `Result` if the call chain can propagate without broad churn. |
| `crates/roko-agent/src/provider/anthropic_api/tool_loop.rs` | cache marker injection | Preserve request execution, but log a warning when cache marker injection fails so prompt-cache behavior is debuggable. |
| `crates/roko-agent/src/provider/claude_cli/stream.rs` | stream JSON fallback parsing | This is a deliberate fallback path; keep protocol behavior and add a short comment or debug log only if the failure is otherwise silent. |
| `crates/roko-serve/src/routes/swe_bench.rs` | background result persistence and list endpoints | Missing directories may mean empty results. Directory read, JSON parse, serialization, and write failures should `warn!` with run id/path. |
| `crates/roko-serve/src/routes/bench.rs` | run updates, index updates, cost summaries | Log save/update/read failures with run id and path. Do not turn best-effort summary generation into a route failure unless the handler already treats it as required state. |
| `crates/roko-serve/src/routes/config.rs` | config propagation to ephemeral workspaces | Main config update may stay successful, but each failed workspace config write needs a `warn!` with workspace id/path. |
| `crates/roko-serve/src/routes/team.rs` | invitation cleanup persistence | Log persistence failure with workspace/email context. |
| `crates/roko-serve/src/routes/deployments.rs` | deployment persistence | Log or propagate create-dir/serialize/write/rename failures; deployment state is user-visible. |
| `crates/roko-serve/src/routes/plans.rs` | paused snapshot, chat mutation write, review append | Snapshot persistence should fail the pause request before reporting success. Chat/review side effects can stay best-effort but must log write failures. |
| `crates/roko-serve/src/routes/gateway.rs` | bandit/router state fallback | Replace silent fallback with `debug!` for missing state and `warn!` for corrupt/unreadable persisted state if the error type allows distinguishing them. |
| `crates/roko-serve/src/routes/workspaces.rs` | cleanup after partial create/delete | Cleanup failures can remain best-effort, but must warn with workspace path. |
| `crates/roko-serve/src/routes/vision_loop.rs` | child kill on cancellation | Warn when killing the child process fails. |
| `crates/roko-serve/src/routes/agents.rs` and `aggregator.rs` | JSON/text fallback parsing | Preserve raw-text/domain-name fallback behavior; add comments or low-level logs rather than converting valid fallback traffic into errors. |

## What to Change

For each instance of the pattern, apply one of three fixes:

### Fix 1: Propagate with `?` (preferred when caller can handle)
```rust
// BEFORE:
if let Ok(data) = fs::read_to_string(&path) { process(data); }

// AFTER:
let data = fs::read_to_string(&path)?;
process(data);
```

### Fix 2: Log at warn/error level (when failure is recoverable)
```rust
// BEFORE:
if let Ok(data) = fs::read_to_string(&path) { process(data); }

// AFTER:
match fs::read_to_string(&path) {
    Ok(data) => process(data),
    Err(e) => tracing::warn!("failed to read {}: {e:#}", path.display()),
}
```

### Fix 3: Explicit comment (when intentional)
```rust
// BEFORE:
let _ = cleanup_temp_file(&path);

// AFTER:
// Intentionally ignoring: temp file cleanup is best-effort; failure is harmless
let _ = cleanup_temp_file(&path);
```

Rules:
- Route handlers should propagate errors to return proper HTTP status codes (500, 404)
- Provider adapters should log at `warn!` level and return an error only when the
  failed operation changes required provider behavior. If the caller cannot
  reasonably recover and the operation is optional, log and continue.
- Cleanup/shutdown code can use `let _ =` with a comment
- Every `if let Ok(...)` without an else branch gets reviewed

Mechanical pass order:
1. Run the `rg` command above and make a short classification list in the task's
   Status Log while implementing: `propagate`, `warn-and-continue`,
   `debug-or-comment-intentional`, or `test-only`.
2. Start with user-visible persistence paths in `routes/plans.rs`,
   `routes/deployments.rs`, `routes/bench.rs`, `routes/swe_bench.rs`,
   `routes/config.rs`, and `routes/workspaces.rs`.
3. Then handle provider-adapter matches in `openai_compat.rs`,
   `anthropic_api/tool_loop.rs`, and `claude_cli/stream.rs`.
4. Re-run `rg` and document any remaining production matches inline with a
   comment or adjacent log message explaining why the failure is intentionally
   non-fatal.

Logging shape:
- Prefer structured tracing fields: `tracing::warn!(path = %path.display(), error = %err, "...")`.
- Use `debug!` for expected parse fallbacks such as websocket raw text or domain
  hostnames.
- Use `warn!` for persistence, state recovery, or filesystem failures where the
  user may otherwise see stale or missing state.

## What NOT to Do

- Don't change error types or introduce new error enums.
- Don't change the `if let Err(e) = ...` pattern where errors ARE logged.
- Don't touch test code.
- Don't change the `let _ = event_bus.emit(...)` pattern (event emission is
  intentionally fire-and-forget).
- Don't require an "intentionally ignoring" comment on `let _ = writeln!(...)`
  calls that write into an in-memory `String` prompt/buffer.
- Don't log absence of optional environment variables such as provider API keys
  or `ROKO_MOCK_STATE_PATH` unless the surrounding code already treats the value
  as required.
- Don't convert fallback parsers into hard failures when current protocol
  behavior accepts either structured JSON or raw text.
- Don't add noisy logs inside hot per-token stream loops unless the log is gated
  to failure paths and uses `debug!`.

## Tests to Add or Update

Add focused tests only where behavior can be observed without brittle log
assertions:
- Route persistence paths changed from silent success to route failure should get
  a test that induces a write/read error and asserts the HTTP status/body.
- List endpoints that skip malformed result files should get a test proving one
  corrupt file does not break the entire list response.
- Provider fallback changes should keep existing protocol behavior. If a JSON
  parse fallback is touched, add a unit test that raw text still succeeds.
- Do not add tests that only assert a tracing log was emitted unless a local
  tracing capture helper already exists in the crate.

## Wire Target

```bash
# No specific wire target — this is a code quality fix
rg -n "if let Ok\(|let _ =|\.ok\(\);" crates/roko-serve/src/routes crates/roko-agent/src/provider --glob '*.rs'
cargo build --workspace
cargo test -p roko-serve --lib
cargo test -p roko-agent --lib
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `rg -n "if let Ok\(|let _ =|\.ok\(\);" crates/roko-serve/src/routes crates/roko-agent/src/provider --glob '*.rs'`
      has no unclassified production matches
- [ ] Remaining production `let _ =` matches are limited to fire-and-forget
      events, best-effort cleanup with a comment, or in-memory string writes
- [ ] Persistence/state failures either propagate to the route response or emit
      `tracing::warn!` with enough context to identify the affected file/entity

## Status Log

| Time | Agent | Action |
|------|-------|--------|
