# Task 084: Concurrency Anti-Pattern Sweep

```toml
id = 84
title = "Fix three concurrency anti-patterns: PlaybookStore nested mutex I/O, TUI thread spawn in async, CancelToken 50ms polling fallback"
track = "infrastructure"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-learn/src/playbook.rs",
    "crates/roko-cli/src/tui/app.rs",
    "crates/roko-cli/src/tui/verdicts.rs",
    "crates/roko-core/src/tool/handler.rs",
    "crates/roko-agent/src/dispatcher/cancel.rs",
]
exclusive_files = [
    "crates/roko-core/src/tool/handler.rs",
    "crates/roko-agent/src/dispatcher/cancel.rs",
]
estimated_minutes = 180
```

## Context

Three concurrency anti-patterns from the infrastructure audit (S15.3, S15.7, S15.8) that
each impose different kinds of latency or correctness risk. None is immediately catastrophic,
but all are fixable in a single sweep because they share the same root cause: wrong primitive
for the job.

**S15.3 — PlaybookStore: `AsyncMutex` held across I/O awaits**
`save_or_merge` in `crates/roko-learn/src/playbook.rs:725-748` acquires an `AsyncMutex`
guard (`_merge_guard`) via `self.id_lock("__playbook_merge__/global").lock().await`, then
calls `self.load()` and `self.save()` — both of which do disk I/O under `.await` — while
holding that guard. This is correct for serialization, but the pattern is fragile: the
`id_lock` helper acquires a `parking_lot::Mutex` on `id_locks` (sync) just to look up or
insert the `Arc<AsyncMutex<()>>`, immediately dropping it, then hands back the Arc. Any
code path that forgets to drop the guard before sleeping will starve all other merge
callers. The lock map also grows unboundedly — one entry per unique `id` string forever.

**S15.7 — TUI tick spawns raw OS threads at TUI refresh rate**
`VerdictsAggregator::tick()` in `crates/roko-cli/src/tui/verdicts.rs:137-143` detects
that it is inside a Tokio runtime via `Handle::try_current()`, then calls
`std::thread::spawn(...)` to run `block_on()` on a current-thread Runtime. This creates
a fresh OS thread for every TUI refresh cycle (~16–33ms), bypassing Tokio's blocking
thread pool (`spawn_blocking`) and accumulating threads faster than the OS scheduler
reclaims them. High-frequency polling of the substrate under high gate volume will cause
thread churn visible in `htop`.

Same pattern in `VerdictsAggregator::open()` at lines 111-114: also spawns a raw thread
when inside a runtime. Both sites need the same fix.

**S15.8 — `CancelToken` trait default is a 50ms polling loop**
`crates/roko-core/src/tool/handler.rs:94-104` — the default implementation of
`CancelToken::cancelled()` polls `is_cancelled()` every 50ms. Any type that implements
`CancelToken` by providing only `is_cancelled()` (not overriding `cancelled()`) will
silently use this polling fallback. This creates up to 50ms of cancellation latency for
every foreign `CancelToken` impl in the codebase.

`AtomicCancel` already overrides `cancelled()` correctly with `tokio::sync::Notify`. The
problem is the trait default itself: new implementors cannot know they must override it, and
the polling loop is invisible in the trait documentation.

## Background

Read these files before writing any code:

1. `crates/roko-learn/src/playbook.rs` — `PlaybookStore` struct (lines 651-676),
   `id_lock()` helper (lines 669-676), `save_or_merge()` (lines 725-748), and
   `record_outcome()` (lines 922-942). Understand which methods hold the lock and for
   how long.

2. `crates/roko-cli/src/tui/verdicts.rs` — `VerdictsAggregator::open()` (lines 98-126)
   and `tick()` (lines 132-168). Both use `std::thread::spawn` when a Tokio runtime is
   active. Read the full `VerdictsAggregator` struct — it owns a `Runtime` field used for
   the non-async context path.

3. `crates/roko-core/src/tool/handler.rs` — `CancelToken` trait (lines 83-105),
   `NeverCancel` impl (lines 107-121), and `AtomicCancel` impl (lines 156-179). The
   `AtomicCancel::cancelled()` override at lines 162-178 is the correct pattern. The
   trait default at lines 94-104 is what needs to change.

4. `crates/roko-agent/src/dispatcher/cancel.rs` — `wait_cancelled()` (lines 21-23) and
   its test suite. `wait_cancelled` now simply delegates to `token.cancelled()` — the
   implementation is already correct. Any remaining polling risk lives entirely in the
   trait default, not here.

5. Check whether `VerdictsAggregator` is called from sync OR async contexts outside the
   TUI:
   ```bash
   grep -rn 'VerdictsAggregator' crates/ --include='*.rs' | grep -v target/ | grep -v verdicts.rs
   ```
   This determines whether the `Runtime` field inside the struct is still needed after
   the fix, or whether the entire current-thread Runtime can be removed.

## What to Change

### Fix 1: PlaybookStore — bounded lock map + documented ordering

The current design is sound (single global mutex serializes merges) but the lock map grows
unboundedly. Replace the unbounded `HashMap<String, Arc<AsyncMutex<()>>>` with a single
dedicated merge `AsyncMutex` that is a named field — no map needed for the global merge
lock:

```rust
// Before (in PlaybookStore struct):
id_locks: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,

// After:
/// Serializes all save_or_merge calls. Held across disk I/O — intentional.
/// Per-id record_outcome locks remain in id_locks; this lock covers only merge.
merge_lock: Arc<AsyncMutex<()>>,
id_locks: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
```

In `new()`:
```rust
Self {
    root: path.into(),
    tmp_counter: Arc::new(Mutex::new(0)),
    merge_lock: Arc::new(AsyncMutex::new(())),
    id_locks: Arc::new(Mutex::new(HashMap::new())),
}
```

In `save_or_merge()`:
```rust
pub async fn save_or_merge(&self, playbook: &Playbook) -> io::Result<()> {
    validate_playbook_id(&playbook.id)?;
    // Named field — no map lookup, no parking_lot lock during merge.
    let _merge_guard = self.merge_lock.lock().await;
    // ... rest unchanged
}
```

Add a doc comment above `save_or_merge` documenting the lock ordering:
```rust
/// Lock ordering: `merge_lock` is always acquired BEFORE any `id_locks` entry.
/// `record_outcome` acquires only an `id_locks` entry, never `merge_lock`.
/// These two lock domains are disjoint — no deadlock is possible.
```

The `id_lock()` method used by `record_outcome` is unchanged — it still uses the
per-id `AsyncMutex` map for fine-grained record_outcome serialization.

The `id_locks` map still grows unboundedly for per-id record-outcome locks. This is a
separate concern (Phase 2: use a slab or LRU). Do NOT address it here.

### Fix 2: VerdictsAggregator — remove raw thread spawns

In `crates/roko-cli/src/tui/verdicts.rs`:

Current-tree refinement: because the TUI app is already async, prefer making
`VerdictsAggregator::open()` and `tick()` async and awaiting `FileSubstrate` directly. Use the
`spawn_blocking` sketches below only if a real sync callsite remains after grepping all callers.

**In `open()`** — replace the `std::thread::spawn` with `tokio::task::spawn_blocking`:

```rust
// Before:
let (runtime, substrate) = if tokio::runtime::Handle::try_current().is_ok() {
    std::thread::spawn(load)
        .join()
        .map_err(|_| anyhow::anyhow!("verdict loader thread panicked"))??
} else {
    load()?
};

// After:
let (runtime, substrate) = if tokio::runtime::Handle::try_current().is_ok() {
    tokio::task::spawn_blocking(load)
        .await
        .map_err(|e| anyhow::anyhow!("verdict loader task panicked: {e}"))??
} else {
    load()?
};
```

Note: `open()` must be made `async` to use `.await` here. Check all callers of
`VerdictsAggregator::open()` and update their signatures accordingly. If `open()` is
called from a sync context (e.g., `fn main()`), wrap in `tokio::runtime::Handle::current()
.block_on(...)` at the call site. Read the callers first before deciding.

**In `tick()`** — same substitution:

```rust
// Before:
let mut verdicts = if tokio::runtime::Handle::try_current().is_ok() {
    let rt_handle = self.runtime.handle().clone();
    std::thread::spawn(move || rt_handle.block_on(substrate.query(&query, &ctx)))
        .join()
        .map_err(|_| anyhow::anyhow!("verdict tick thread panicked"))?
        .context("query verdict substrate")?
} else {
    self.runtime
        .block_on(substrate.query(&query, &ctx))
        .context("query verdict substrate")?
};

// After (tick() must also be async):
let mut verdicts = if tokio::runtime::Handle::try_current().is_ok() {
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tick runtime")
            .block_on(substrate.query(&query, &ctx))
    })
    .await
    .map_err(|e| anyhow::anyhow!("verdict tick task panicked: {e}"))??
} else {
    self.runtime
        .block_on(substrate.query(&query, &ctx))
        .context("query verdict substrate")?
};
```

If `tick()` becoming `async` creates a cascade of signature changes in the TUI's periodic
refresh loop, that is expected and correct — the TUI already runs inside a Tokio runtime.

**If the `runtime: Runtime` field is no longer needed** after callers are all async-aware,
remove it from the struct to eliminate the embedded current-thread runtime entirely.

### Fix 3: `CancelToken` default — remove polling, require Notify or document the gap

In `crates/roko-core/src/tool/handler.rs`, change the default `cancelled()` implementation
to make the 50ms polling visible and push implementors to use `Notify`:

**Option A (preferred): Make the default panic in debug, poll in release**

```rust
async fn cancelled(&self) {
    // Default polls every 50ms — acceptable only for foreign impls that cannot
    // use Notify. For owned types, override this with a Notify-backed impl.
    // In debug mode, log a warning so that new impls don't silently use polling.
    #[cfg(debug_assertions)]
    tracing::warn!(
        type_name = std::any::type_name::<Self>(),
        "CancelToken: using 50ms polling fallback — consider overriding cancelled() with Notify"
    );
    if self.is_cancelled() { return; }
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if self.is_cancelled() { return; }
    }
}
```

**Option B: Document the polling contract explicitly**

Update the `CancelToken` trait doc to make the polling fallback opt-in via a marker:
```rust
/// The default implementation polls every 50 ms. Override this method with a
/// `tokio::sync::Notify`-backed impl for zero-latency cancellation. See
/// [`AtomicCancel`] for the canonical pattern.
///
/// **Required override for owned types.** The default is a compatibility shim
/// for foreign impls only; any type you fully control MUST override `cancelled()`.
```

Implement Option A AND update the doc comment with Option B's language. The `tracing::warn!`
fires once per unique type name in debug builds, making it easy to audit during `cargo test`.

Check that no tests regress from the new warning. The test in `dispatcher/mod.rs` for
`cancellation_returns_cancelled` uses `AtomicCancel` which overrides `cancelled()` —
no warning fires there.

## Current Tree Notes and Mechanical Plan

Current facts to verify before editing:
- `PlaybookStore` currently has `tmp_counter` and `id_locks`; `save_or_merge()` obtains
  `self.id_lock("__playbook_merge__/global").lock().await` and holds it through `load()` and
  `save()`.
- `VerdictsAggregator` currently stores a `tokio::runtime::Runtime`, and both `open()` and
  `tick()` use `std::thread::spawn` when `Handle::try_current()` detects an active runtime.
- The TUI app is already driven from async `crates/roko-cli/src/tui/app.rs::run()`, but
  `tick_snapshot()`, `reseed_verdicts_aggregator()`, and `refresh_verdicts_from_aggregator()`
  are sync today.
- `CancelToken::cancelled()` still has the 50 ms polling default in
  `crates/roko-core/src/tool/handler.rs`; `AtomicCancel` and `NeverCancel` override it.
- `crates/roko-acp/src/bridge_events.rs::AcpToolCancelToken` is a known foreign wrapper that
  currently relies on the default polling fallback. Do not edit it in this task unless the
  touch list is explicitly expanded.

Ordered implementation steps:
1. In `playbook.rs`, add `merge_lock: Arc<AsyncMutex<()>>` to `PlaybookStore`, initialize it in
   `new()`, update the struct doc to describe both lock domains, and change `save_or_merge()` to
   lock `self.merge_lock` directly. Leave `id_lock()` and `record_outcome()` per-id locking intact.
2. Add or update a `save_or_merge` concurrency test that spawns several same-pattern saves and
   verifies the resulting playbook has merged counters instead of a lost update.
3. In `verdicts.rs`, prefer converting `VerdictsAggregator::open()` and `tick()` to `async` and
   remove the embedded `Runtime` field entirely. Then `FileSubstrate::open(...).await` and
   `substrate.query(...).await` can be awaited directly with no raw thread and no blocking bridge.
4. If a sync callsite truly remains after `rg -n "VerdictsAggregator::open|\\.tick\\("`, keep a
   small `open_blocking()`/`tick_blocking()` wrapper only for that callsite and implement the
   blocking work with `tokio::task::spawn_blocking(...).await` from async callers. Do not keep
   the current per-refresh `std::thread::spawn` path.
5. In `app.rs`, update `run()` to await any methods that now refresh verdicts asynchronously.
   The expected chain is `run()` -> `drain_snapshot_channel().await` (if changed) ->
   `tick_snapshot().await` -> `refresh_verdicts_from_aggregator().await`. Keep the UI draw path
   non-blocking.
6. Convert affected `verdicts.rs` unit tests to `#[tokio::test]` and await `open()`/`tick()`.
7. In `handler.rs`, update the `CancelToken` trait docs to make the polling fallback a
   compatibility shim only. Add a debug-only `tracing::warn!` in the default `cancelled()` before
   the sleep loop. If implementing the "once per type" requirement, use a process-local
   `OnceLock<Mutex<HashSet<&'static str>>>` keyed by `std::any::type_name::<Self>()`; do not log on
   every 50 ms poll iteration.
8. Leave `crates/roko-agent/src/dispatcher/cancel.rs::wait_cancelled()` as a pure delegation to
   `token.cancelled().await`; update only comments/tests if they mention invisible polling.

Observable behavior expected after implementation:
- `rg -n "std::thread::spawn" crates/roko-cli/src/tui/verdicts.rs` returns no matches.
- Running `cargo run -p roko-cli -- dashboard` for 10 seconds does not grow OS thread count every
  refresh tick.
- `wait_cancelled()` with `AtomicCancel` still resolves within 30 ms of cancellation.
- Debug builds surface one warning for any owned/foreign `CancelToken` impl that uses the default
  polling fallback, making the latency explicit.

## What NOT to Do

- Do NOT remove the `id_locks` map — `record_outcome` still needs per-id locks. Only
  the merge lock is promoted to a named struct field.
- Do NOT attempt to bound the `id_locks` map in this task — that is a separate concern
  requiring an LRU cache and is out of scope here.
- Do NOT use `tokio::task::block_in_place` in `verdicts.rs` — `block_in_place` requires
  a multi-thread runtime; the TUI may run on a current-thread runtime in tests.
  `spawn_blocking` is correct and works in both configurations.
- Do NOT remove the `runtime: Runtime` field from `VerdictsAggregator` unless you have
  verified there are no remaining sync call sites. Check with grep first.
- Do NOT change `AtomicCancel::cancelled()` — it already uses `Notify` correctly.
- Do NOT add polling to any `CancelToken` impl that currently overrides `cancelled()`.
- Do NOT change `wait_cancelled()` in `dispatcher/cancel.rs` — it already correctly
  delegates to `token.cancelled()`.

## Wire Target

```bash
# Build — must compile clean
cargo build --workspace

# Verify the TUI tab renders without thread explosion
# (run the TUI and check that thread count doesn't grow over time)
cargo run -p roko-cli -- dashboard

# Verify cancellation still works correctly
cargo test -p roko-agent -- dispatcher::cancel --nocapture
cargo test -p roko-core -- tool::handler --nocapture

# Check no new OS threads spawn per TUI tick (run for 10s, count threads):
cargo run -p roko-cli -- dashboard &
PID=$!
sleep 10
# Linux: cat /proc/$PID/status | grep Threads
# macOS: ps -M $PID | wc -l
kill $PID

# Verify the debug warning fires for any non-Notify CancelToken impls:
RUST_LOG=roko_core=debug cargo test --workspace 2>&1 | grep "polling fallback"
# Only impls that don't override cancelled() should appear here.
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'std::thread::spawn' crates/roko-cli/src/tui/verdicts.rs` — returns nothing
- [ ] `grep -rn 'spawn_blocking' crates/roko-cli/src/tui/verdicts.rs` — shows both call sites
- [ ] `PlaybookStore` struct has a named `merge_lock: Arc<AsyncMutex<()>>` field
- [ ] `save_or_merge` doc comment includes lock-ordering note
- [ ] `CancelToken::cancelled()` default has a `tracing::warn!` in `#[cfg(debug_assertions)]`
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in changed files
- [ ] Unit test: `wait_cancelled` with `AtomicCancel` resolves within 30ms of cancellation
- [ ] Unit test: `save_or_merge` concurrent calls on same ID do not lose updates

## Status Log

| Time | Agent | Action |
|------|-------|--------|
