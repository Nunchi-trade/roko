# Task 080: Production Code Unwrap Elimination

```toml
id = 80
title = "Replace .unwrap() and .expect() in production code paths with proper error handling"
track = "infrastructure"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-serve/src/lib.rs",
    "crates/roko-agent/src/provider/openai_compat.rs",
    "crates/roko-agent/src/provider/mod.rs",
    "crates/roko-agent/src/codex_agent.rs",
    "crates/roko-agent/src/provider/anthropic_api/tool_loop.rs",
    "crates/roko-agent/src/safety/hooks.rs",
    "crates/roko-chain/src/marketplace.rs",
    "crates/roko-cli/src/prd.rs",
]
exclusive_files = []
estimated_minutes = 120
```

## Context

Source: infrastructure-audit.md §S6.4 — "Unwrap in Non-Test Code".

`.unwrap()` and `.expect()` in production code paths are process-kill panics waiting to happen.
Each one is a single bad runtime state away from taking down the entire `roko serve` process or
corrupting an in-flight agent run with no error message, no recovery, and no log context.

The rule is simple: **`.unwrap()` and `.expect()` are only acceptable in:**

1. `#[cfg(test)]` or `mod tests { ... }` blocks
2. Provably-infallible operations at compile time — e.g. `Regex::new("literal").expect("literal regex compiles")` where the string is a visible compile-time constant, or `[u8; 8].try_into().expect("...")` converting a fixed-length slice

Everything else must use `?`, `.ok_or(...)`, `.unwrap_or_default()`, or an explicit error
branch with a `tracing::warn!` or `tracing::error!` call.

The audit identified three primary file clusters with confirmed production panics:

- **`crates/roko-serve/src/lib.rs`** — `self.state.as_ref().expect("state just set")` at line
  311, the chain-watcher `/dev/null` fallback around line 394, and
  `create_backend("manual", ...).expect("manual backend cannot fail")` around line 1865. The
  state check is a programming-invariant panic in server startup; the others are defensive
  fallback paths that can fail under unusual runtime conditions.
- **`crates/roko-agent/src/provider/openai_compat.rs`** — Three `.expect()` calls at lines
  260–269 in the `block_on` helper function: `tokio::runtime::Builder::new_current_thread().build().expect("create MCP discovery runtime")` and `.join().expect("join MCP discovery thread")`. Runtime construction can fail under resource exhaustion. Thread join panics if the spawned thread panicked.
- **`crates/roko-agent/src/provider/mod.rs`** — `semaphore.acquire_owned().await.expect("semaphore closed")` at line 456. The semaphore close path is uncommon but reachable if the concurrency pool is torn down while a request is in flight.
  Code inspection also found the shared HTTP client builder expect around line 115 and
  Perplexity search-options serialization expect around line 550.
- **`crates/roko-chain/src/marketplace.rs`** — `self.jobs.get_mut(job_id).unwrap()` at lines 469 and 514, inside `settle_job` and `expire_job`. These occur after earlier error-checked lookups but the HashMap contract does not guarantee the entry is still present between calls. Should use `.ok_or(MarketplaceError::NotFound)?`.
- **`crates/roko-cli/src/prd.rs`** — `best.unwrap().1` at line 1896 inside a fuzzy-field matcher. Safe by local logic (`best.is_none()` is checked), but non-idiomatic and fragile to refactor. Replace with `best.map(|(_, d)| d).unwrap_or(usize::MAX)` or restructure the condition.

## Background

Read these files before writing any code:

1. `crates/roko-serve/src/lib.rs` — Lines 305–315 (`self.state.as_ref().expect(...)`),
   the chain watcher log fallback around line 394 (`/dev/null`).expect, and
   `create_backend("manual", ...).expect(...)` around line 1865. Understand which enclosing
   functions return `anyhow::Result` and which must use a non-panicking fallback.
2. `crates/roko-agent/src/provider/openai_compat.rs` — Lines 250–275, the `block_on` helper.
   Understand why it exists (bridges sync callers to async MCP discovery). The fix must preserve
   the bridge behaviour while surfacing runtime construction failures as `AgentCreationError` instead
   of panics.
3. `crates/roko-agent/src/provider/mod.rs` — Lines 450–460, the semaphore acquire path. The
   `ProviderSemaphores` type and how `acquire_owned` is called. Also inspect the shared HTTP
   client `LazyLock` around line 115 and `with_perplexity_search_options()` around line 550.
4. `crates/roko-chain/src/marketplace.rs` — Lines 460–480 (`settle_job`) and 508–520
   (`expire_job`). The `MarketplaceError` enum is already defined in the same file; use it.
5. `crates/roko-cli/src/prd.rs` — Lines 1888–1905, the fuzzy-field matcher. The surrounding
   `find_similar_field` function and how `best` is constructed and consumed.

Search commands:
```bash
# Confirm current production unwrap/expect sites (exclude test modules)
grep -n '\.unwrap()\|\.expect(' crates/roko-serve/src/lib.rs | head -20
grep -n '\.unwrap()\|\.expect(' crates/roko-agent/src/provider/openai_compat.rs | head -20
grep -n '\.unwrap()\|\.expect(' crates/roko-agent/src/provider/mod.rs | head -20
grep -n '\.unwrap()\|\.expect(' crates/roko-agent/src/codex_agent.rs crates/roko-agent/src/provider/anthropic_api/tool_loop.rs | head -20
grep -n '\.unwrap()\|\.expect(' crates/roko-agent/src/safety/hooks.rs | head -20
grep -n '\.unwrap()\|\.expect(' crates/roko-chain/src/marketplace.rs | head -10
grep -n '\.unwrap()\|\.expect(' crates/roko-cli/src/prd.rs | head -10
```

## What to Change

### 1. `crates/roko-serve/src/lib.rs` — state invariant at line 310

The call `self.state.as_ref().expect("state just set")` asserts that `self.state` is `Some`
after being set two lines earlier in the same function. This is a programming invariant, not a
runtime condition. Convert to a proper error return:

```rust
// Before
let state = Arc::clone(self.state.as_ref().expect("state just set"));

// After
let state = Arc::clone(
    self.state
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("server state not initialized before bind"))?,
);
```

The enclosing function already returns `anyhow::Result<_>`, so `?` propagates cleanly.

### 2. `crates/roko-serve/src/lib.rs` — chain-watcher log fallback

The chain watcher spawn path clones a log file and falls back to
`std::fs::File::open("/dev/null").expect("/dev/null")` if `try_clone()` fails. Replace the
fallback with `Stdio::null()` and a warning:

```rust
let stderr_target = match f.try_clone() {
    Ok(f2) => std::process::Stdio::from(f2),
    Err(err) => {
        warn!(error = %err, "failed to clone chain watcher log file; discarding stderr");
        std::process::Stdio::null()
    }
};
```

Keep stdout connected to the original `f`; only stderr falls back.

### 3. `crates/roko-serve/src/lib.rs` — manual backend fallback around line 1865

`create_backend("manual", None, None, None).expect("manual backend cannot fail")` is in a
fallback arm that runs when the configured backend fails to create. If the manual backend
itself fails, the process panics. Convert:

```rust
// Before
Arc::from(
    deploy::create_backend("manual", None, None, None)
        .expect("manual backend cannot fail"),
)

// After
match deploy::create_backend("manual", None, None, None) {
    Ok(backend) => Arc::from(backend),
    Err(e) => {
        tracing::error!(error = %e, "fallback manual backend factory failed; constructing manual backend directly");
        let backend: Arc<dyn deploy::DeployBackend> =
            Arc::new(deploy::manual::ManualBackend::default());
        backend
    }
}
```

There is no `NoopBackend` in the current codebase. `create_deploy_backend()` returns
`Arc<dyn deploy::DeployBackend>`, not `Result`, so use the direct `ManualBackend::default()`
fallback above instead of changing the public function signature.

### 4. `crates/roko-agent/src/provider/openai_compat.rs` — `block_on` tokio runtime

The `block_on` function (lines 250–275) creates a throwaway Tokio runtime to drive MCP tool
discovery from a synchronous call site. It uses `.expect()` on two operations that can fail:
runtime construction (memory/fd exhaustion) and thread join (if the spawned thread panicked).

Change the signature from `fn block_on<F>(...) -> F::Output` to
`fn block_on<F>(...) -> Result<F::Output, AgentCreationError>` to match this file's
agent-construction error convention. Do not introduce `anyhow` here.

```rust
fn block_on<F>(future: F) -> Result<F::Output, AgentCreationError>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| AgentCreationError::MissingConfig(format!("MCP discovery runtime: {e}")))?
                .block_on(future)
        })
        .join()
        .map_err(|_| AgentCreationError::MissingConfig("MCP discovery thread panicked".into()))?
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AgentCreationError::MissingConfig(format!("MCP discovery runtime: {e}")))?
            .block_on(future)
    }
}
```

`discover_mcp_tools()` itself returns a `Result`, so the caller needs to flatten both layers:

```rust
let mcp_tools = block_on(async move { discover_mcp_tools(&mcp_config).await })?
    .map_err(|err| {
        AgentCreationError::MissingConfig(format!(
            "mcp tool discovery from {} failed: {err}",
            mcp_config_path.display()
        ))
    })?;
```

### 5. `crates/roko-agent/src/provider/mod.rs` — shared HTTP client and search options

`shared_http_client()` uses a `LazyLock` whose builder currently has
`.expect("failed to build shared HTTP client")`. Replace that with a non-panicking fallback:

```rust
match reqwest::Client::builder()
    // existing builder options unchanged
    .build()
{
    Ok(client) => client,
    Err(err) => {
        tracing::error!(error = %err, "failed to build shared HTTP client; using reqwest default client");
        reqwest::Client::new()
    }
}
```

`AgentOptions::with_perplexity_search_options()` currently expects JSON serialization to be
infallible. Keep the method returning `Self`, but log and skip the extra arg on error:

```rust
match serde_json::to_string(&search_options) {
    Ok(encoded) => self.extra_args.push(format!("{PERPLEXITY_SEARCH_OPTIONS_ARG_PREFIX}{encoded}")),
    Err(err) => tracing::warn!(error = %err, "failed to encode Perplexity search options"),
}
```

### 6. `crates/roko-agent/src/provider/mod.rs` — semaphore acquire

`semaphore.acquire_owned().await.expect("semaphore closed")` at line 456. The `acquire_owned`
method returns `Result<OwnedSemaphorePermit, AcquireError>`. The error only occurs when the
semaphore is closed, which should not happen under normal operation but is reachable during
shutdown. Convert `ProviderSemaphores::acquire` to return
`Result<OwnedSemaphorePermit, ProviderError>` and add a concrete provider error variant:

```rust
pub enum ProviderError {
    // existing variants...
    Concurrency(String),
}

semaphore
    .acquire_owned()
    .await
    .map_err(|_| ProviderError::Concurrency("semaphore closed during acquire".into()))
```

Update `Display` and `should_retry()` for `ProviderError::Concurrency`; classify it as
`RetryAction::TryFallback`.

Then update all current callers:

- `crates/roko-agent/src/codex_agent.rs`: map acquire failure to the existing
  `self.fail(input, "...", started)` path before making the HTTP request.
- `crates/roko-agent/src/provider/anthropic_api/tool_loop.rs`: map acquire failure to
  `LlmError::Backend(format!("provider concurrency acquire failed: {err}"))`.

### 7. `crates/roko-chain/src/marketplace.rs` — HashMap get_mut after prior check

Lines 469 and 514 call `self.jobs.get_mut(job_id).unwrap()` inside `settle_job` and
`expire_job` after earlier `ok_or(MarketplaceError::NotFound)?` guards on the same key. The
HashMap entry could theoretically disappear between the read guard and the mutable access if
the data structure were shared, but even in single-threaded code the pattern is fragile and
clippy flags it. Convert both to:

```rust
// Before
let job = self.jobs.get_mut(job_id).unwrap();

// After
let job = self
    .jobs
    .get_mut(job_id)
    .ok_or(MarketplaceError::NotFound)?;
```

The enclosing functions already return `Result<_, MarketplaceError>`, so `?` propagates without
any signature change.

### 8. `crates/roko-cli/src/prd.rs` — `best.unwrap().1` in fuzzy matcher

Line 1896:
```rust
if best.is_none() || dist < best.unwrap().1 {
    best = Some((known_field, dist));
}
```

`best.unwrap()` is safe here because the `is_none()` short-circuits, but the pattern confuses
both humans and static analysis. Replace with idiomatic Rust:

```rust
if best.map_or(true, |(_, best_dist)| dist < best_dist) {
    best = Some((known_field, dist));
}
```

This is equivalent, removes the `unwrap`, and is more readable.

## What NOT to Do

- Do NOT replace regex-literal `.expect()` calls with `?` — those are compile-time infallible
  and the `.expect()` is the correct annotation. The rule only applies to runtime operations.
- Do NOT replace `TaintedString::as_str()`'s
  `.expect("tainted string stores valid UTF-8")` in `safety/hooks.rs`; the only public
  constructor accepts `String` and stores its bytes, so this is a type invariant. Leave a
  Status Log note that it was inspected and intentionally kept.
- Do NOT introduce new `Box<dyn Error>` to avoid specifying error types. Pick the concrete error
  type that the enclosing function already uses, or add a variant to it.
- Do NOT add a `// SAFETY:` comment claiming an unwrap is safe without removing the unwrap.
  Comments don't prevent panics.
- Do NOT add error logging in addition to `?` propagation — choose one: propagate with `?`, or
  log and use a fallback value. Doing both creates duplicate log lines.
- Do NOT change function signatures in public crate APIs unless required to propagate the error.
  Use `unwrap_or_else(|e| { warn!(...); fallback })` at API boundaries where callers don't
  expect a `Result`.
- Do NOT touch test modules. Tests may legitimately use `.unwrap()` for brevity.

## Wire Target

```bash
# After the fix, these patterns should not appear in production code paths
grep -rn '\.unwrap()\|\.expect(' \
  crates/roko-serve/src/lib.rs \
  crates/roko-agent/src/provider/openai_compat.rs \
  crates/roko-agent/src/provider/mod.rs \
  crates/roko-agent/src/codex_agent.rs \
  crates/roko-agent/src/provider/anthropic_api/tool_loop.rs \
  crates/roko-chain/src/marketplace.rs \
  crates/roko-cli/src/prd.rs \
  | grep -v '#\[cfg(test)\]\|mod tests\|Regex::new\|try_into\|tainted string stores valid UTF-8'
# Expected: zero results for the targeted locations

# Full workspace build — no regressions
cargo build --workspace

# All tests still pass
cargo test --workspace
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -n '\.unwrap()' crates/roko-serve/src/lib.rs | awk -F: '$2 < 1878'` — zero results before the test module
- [ ] `grep -n '\.expect(' crates/roko-serve/src/lib.rs | awk -F: '$2 < 1878'` — zero results before the test module
- [ ] `grep -n '\.expect(' crates/roko-agent/src/provider/openai_compat.rs | awk -F: '$2 < 447'` — zero results (before test module at line 447)
- [ ] `grep -n '\.expect(' crates/roko-agent/src/provider/mod.rs | awk -F: '$2 < 641'` — zero results outside comments/tests
- [ ] `grep -n '\.expect(' crates/roko-agent/src/codex_agent.rs crates/roko-agent/src/provider/anthropic_api/tool_loop.rs | grep -v test` — no new runtime expects from semaphore handling
- [ ] `grep -n '\.expect(' crates/roko-agent/src/safety/hooks.rs` — only the inspected `TaintedString::as_str()` UTF-8 invariant and test code remain
- [ ] `head -820 crates/roko-chain/src/marketplace.rs | grep '\.unwrap()'` — zero results (before test module at line 821)
- [ ] `head -2536 crates/roko-cli/src/prd.rs | grep '\.unwrap()'` — zero results (before test module at line 2537)
- [ ] All targeted runtime panics have been replaced with `?`, `ok_or`, explicit `match`, or logged fallback behavior
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` introduced in any touched file

## Status Log

| Time | Agent | Action |
|------|-------|--------|
