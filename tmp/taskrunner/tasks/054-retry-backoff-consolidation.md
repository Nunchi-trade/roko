# Task 054: Consolidate Retry Logic to Use Shared RetryPolicy

```toml
id = 54
title = "Replace ad-hoc retry loops with RetryPolicy from roko-core"
track = "infrastructure"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-core/src/error/mod.rs",
    "crates/roko-core/src/error/retry.rs",
    "crates/roko-agent/src/retry.rs",
    "crates/roko-agent/src/provider/",
    "crates/roko-agent/src/model_call_service.rs",
    "crates/roko-agent/src/mcp/client.rs",
]
exclusive_files = []
estimated_minutes = 90
```

## Context

The audit (S6.5) identified immediate retry loops without backoff:
```rust
for _ in 0..3 {
    if let Ok(r) = try_request().await { return Ok(r); }
}
```

`roko_core::error::ErrorKind::retry_policy()` already returns retry parameters (max
attempts, backoff). The constants are centralized in `roko_core::defaults` (batch 43).
But callers still implement their own retry loops instead of using a shared executor.

## Background

Read:
- `crates/roko-core/src/error/mod.rs` — `retry_policy()` method
- `crates/roko-core/src/error/retry.rs` — existing shared `RetryPolicy`
- `crates/roko-core/src/defaults.rs` — retry default constants
- `crates/roko-agent/src/retry.rs` — duplicate local agent retry policy to consolidate
- `crates/roko-agent/src/model_call_service.rs` — provider fallback loop used by runtime calls

Grep for ad-hoc retry patterns:
```bash
grep -rn 'for _ in 0\.\.' crates/roko-agent/src/ --include='*.rs' | grep -v target/ | grep -v test
grep -rn 'retry\|retries\|attempt' crates/roko-agent/src/provider/ --include='*.rs' | grep -v target/ | grep -v test | head -20
grep -rn 'for _ in 0\.\.' crates/roko-cli/src/ --include='*.rs' | grep -v target/ | grep -v test | head -20
```

## Current Code Reality - 2026-05-05

- The shared policy already exists at `crates/roko-core/src/error/retry.rs` with
  `RetryPolicy::{new,max_attempts,should_retry,delay_for}`. Do not create a second
  `crates/roko-core/src/retry.rs`.
- `ErrorKind::retry_policy()` and `RokoError::retry_policy()` already return
  `Option<retry::RetryPolicy>`. `None` means permanent/non-retryable.
- `crates/roko-agent/src/retry.rs` is a separate duplicate policy using `ProviderError`
  and random jitter. Consolidate this toward `roko_core::error::retry::RetryPolicy`
  instead of extending the duplicate.
- The runtime provider call chain is
  `roko-cli`/`WorkflowEngine` -> `ModelCallService` -> `ProviderCallCell::execute()`
  -> `ProviderAgent::run()`. `ProviderCallCell::execute()` currently iterates primary
  plus `fallback_models` but does not sleep/back off between retryable provider failures.
- Current grep shows no MCP retry loop in `crates/roko-agent/src/mcp/client.rs`. Inspect
  it for drift, but do not invent MCP retries if there is no existing retry behavior to
  consolidate.

## What to Change

### 1. Add `RetryPolicy::execute` helper to roko-core

If not already present, add an async retry executor:

```rust
// crates/roko-core/src/error/retry.rs
impl RetryPolicy {
    /// Execute `f` with exponential backoff + optional deterministic jitter.
    pub async fn execute<F, Fut, T, E>(&self, mut f: F) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let mut attempt = 0;
        loop {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) if !self.should_retry(attempt) => return Err(e),
                Err(e) => {
                    let delay = self.delay_for(attempt);
                    tracing::warn!(
                        attempt,
                        max = self.max_attempts(),
                        "retryable error: {e}, backing off {delay:?}"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }
}
```

Adapt the snippet to the existing fields and methods in `error/retry.rs`; it is a shape,
not a copy/paste replacement. Use `self.should_retry(attempt)` and
`self.delay_for(attempt)` so the existing tests keep defining the semantics. Attempts are
currently zero-based in `should_retry`, so add tests before changing any indexing.

### 2. Keep `ErrorKind::retry_policy()` as the source of truth

Do not add a tuple-returning conversion; the current API already returns
`Option<RetryPolicy>`. Callers that have a `RokoError`/`ErrorKind` should use that API.
Callers that only have `ProviderError` need a small mapping helper:

- retryable: rate limit, timeout, server/transient/unknown transient provider errors
- permanent: auth, content policy, context overflow, model not found/configuration
- provider `retry_after_ms`/equivalent, when available, overrides the computed delay

### 3. Replace ad-hoc retry loops

Find and replace immediate retry loops in:
- `crates/roko-agent/src/model_call_service.rs` — provider call execution and fallback
  handling
- `crates/roko-agent/src/provider/` — only if an actual provider retry loop remains after
  the model-call service change
- `crates/roko-agent/src/mcp/client.rs` — only if inspection finds an existing MCP retry loop

Replace with:
```rust
let policy = error_kind.retry_policy().expect("retryable");
policy.execute(|| async { provider.send(request).await }).await?
```

For `ProviderCallCell::execute()`, preserve the existing primary/fallback model behavior:
retry a retryable failure according to policy, then move to the next fallback only after
the policy is exhausted or the error is better handled by fallback. Do not turn fallback
selection itself into "retry without backoff".

### 4. Add focused tests

- In `roko-core`, add async tests for `RetryPolicy::execute`: succeeds after N retryable
  failures, stops after `max_attempts`, and does not sleep after the final attempt. Use
  `tokio::time::pause/advance` where possible instead of real sleeps.
- In `roko-agent`, add or update a provider/model-call test proving retryable provider
  failures are attempted more than once with policy-controlled delay, and permanent errors
  are attempted once.
- If the local `crates/roko-agent/src/retry.rs` remains as a compatibility wrapper, test
  that it delegates to core semantics and does not define divergent constants.

## What NOT to Do

- Don't add retry to paths that currently don't retry (e.g., file I/O).
- Don't add a `rand` dependency if not already present — use a simpler jitter
  (e.g., `SystemTime::now().as_nanos() % range`).
- Don't change the retry defaults — they are already centralized.
- Don't make `RetryPolicy` the only way to retry — callers that need custom
  retry behavior can still implement their own.
- Don't sleep while holding locks or while owning a mutable borrow that prevents fallback
  selection after retry exhaustion.
- Don't retry non-idempotent tool execution; this task is about provider/MCP request retries.

## Wire Target

```bash
cargo build --workspace
cargo test -p roko-core -- retry
cargo test -p roko-agent --lib
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `RetryPolicy::execute` exists and has unit tests
- [ ] Runtime provider calls in `ModelCallService` use `RetryPolicy`
- [ ] Duplicate agent retry policy is removed or reduced to a thin compatibility wrapper
- [ ] Backoff is exponential (not immediate retry)

## Status Log

| Time | Agent | Action |
|------|-------|--------|
