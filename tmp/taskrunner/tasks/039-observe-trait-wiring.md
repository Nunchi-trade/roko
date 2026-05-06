# Task 039: Redesign Observe Trait + Wire StoreObserver into `roko status`

```toml
id = 39
title = "Redesign Observe trait to async + implement StoreObserver + wire into roko status"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-core/src/traits.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-core/src/store_observer.rs",
    "crates/roko-cli/src/status.rs",
    "crates/roko-cli/src/commands/util.rs",
    "crates/roko-cli/src/main.rs",
]
exclusive_files = []
estimated_minutes = 150
```

## Context

The Observe trait already exists in `roko-core/src/traits.rs` but it's a stub:

```rust
pub trait Observe: crate::cell::Cell {
    fn observe(&self) -> Vec<Engram>;
}
```

This has three problems:
1. It's synchronous — observation often requires I/O (store queries, health checks)
2. It takes no context parameter — no way to pass runtime state
3. It has zero implementations and zero callers

This task redesigns Observe to the v2 spec, implements `StoreObserver` as the first concrete
implementation, and wires it into `roko status` so it has a real caller.

Checklist items: P1-9, P1-10.

## Background

Read these files before starting:

1. `crates/roko-core/src/traits.rs` — current Observe stub (lines 397-403)
2. `crates/roko-cli/src/status.rs` — current status implementation
3. `crates/roko-cli/src/main.rs` — find the `status` subcommand handler
4. `tmp/v2-refactoring/06-NEW-PROTOCOLS.md` — the v2 spec for Observe

Also understand how `roko status` currently gets its data:
```bash
grep -rn 'SessionStatus\|signal_count\|episode_count' crates/roko-cli/src/status.rs --include='*.rs'
grep -rn 'cmd_status\|StatusCmd\|run_status' crates/roko-cli/src/main.rs --include='*.rs' | head -20
```

Current source notes that supersede the illustrative snippets below:

- The active `roko status` call chain is `crates/roko-cli/src/main.rs`
  `Command::Status` -> `crates/roko-cli/src/commands/status.rs` ->
  `crates/roko-cli/src/commands/util.rs::cmd_status`. Do not wire only
  `crates/roko-cli/src/status.rs`; that helper is not the active command path.
- `cmd_status` already opens `FileSubstrate::open(workdir.join(".roko")).await`,
  creates `Context::now()`, queries `Query::all()`, and prints/serializes
  `signal_count` from `all.len()`. The observer must replace that count source
  in this function while preserving the existing output fields.
- `Engram::builder` currently requires a `Kind` argument. There is no
  `Kind::Event`, no `.author(...)`, and no zero-argument builder. Use existing
  APIs such as `Engram::builder(Kind::Metric)`, `Body::Json(...)`, and
  `Provenance::agent("store-observer")`, or the corresponding `Signal` alias if
  Task 037 has landed.
- `roko-core` should not depend on `roko-std` for the StoreObserver unit test.
  Use a small local test `Store` implementation in `store_observer.rs`.

## What to Change

### 1. Redesign Observe trait in `crates/roko-core/src/traits.rs`

Replace the existing Observe trait with:

```rust
/// Passive data collection from external or internal state.
///
/// Unlike Connect (which manages bidirectional I/O), Observe is read-only:
/// it reads state and emits observation Signals without side effects.
///
/// Implementations: StoreObserver (storage stats), AgentObserver (agent health),
/// SystemObserver (resource usage).
#[async_trait]
pub trait Observe: Cell {
    /// Collect observations from the environment. Returns zero or more observation signals.
    async fn observe(&self, ctx: &Context) -> Result<Vec<Engram>>;

    /// Human-readable name for this observer.
    fn observer_name(&self) -> &str { self.cell_name() }

    /// Topic filter for the observations this observer produces.
    /// Used by the Bus to route observation pulses.
    fn observation_topic(&self) -> Option<&str> { None }
}
```

**Note**: Uses `Engram` (not `Signal`) since this task is independent of task 037. If task 037
lands first, use `Signal`. If not, use `Engram` and task 038 will rename it.

### 2. Implement StoreObserver

Create a new file `crates/roko-core/src/store_observer.rs`:

```rust
use crate::{Cell, CellVersion, Context, Engram, Kind, Body, Provenance, traits::{Observe, Store}};
use std::sync::Arc;
use async_trait::async_trait;

/// Observes a Store and returns statistics as observation Signals.
pub struct StoreObserver {
    store: Arc<dyn Store>,
}

impl StoreObserver {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }
}

impl Cell for StoreObserver {
    fn cell_id(&self) -> &str { "store-observer" }
    fn cell_name(&self) -> &str { "Store Observer" }
    fn protocols(&self) -> &[&str] { &["Observe"] }
}

#[async_trait]
impl Observe for StoreObserver {
    async fn observe(&self, _ctx: &Context) -> crate::error::Result<Vec<Engram>> {
        let count = self.store.len().await?;
        let store_name = self.store.name();

        // Build an observation engram with store statistics
        let body = serde_json::json!({
            "observer": "store-observer",
            "topic": "observe.store.stats",
            "store": store_name,
            "signal_count": count,
        });

        let engram = Engram::builder(Kind::Metric)
            .body(Body::Json(body))
            .provenance(Provenance::agent("store-observer"))
            .build();

        Ok(vec![engram])
    }

    fn observation_topic(&self) -> Option<&str> {
        Some("observe.store.stats")
    }
}
```

**Important**: Verify the builder snippet against the current tree before coding.
Read `crates/roko-core/src/engram.rs` and use the real builder shape:
`Engram::builder(Kind::Metric)` or `Engram::builder(Kind::Custom(...))`,
`.body(Body::Json(...))`, `.provenance(Provenance::agent(...))`, then `.build()`.
The emitted JSON body must include at least:

```json
{
  "observer": "store-observer",
  "store": "<store name>",
  "signal_count": 0
}
```

Add a stable topic marker such as a tag or body field with `observe.store.stats` so the
status command and later trigger tests can identify the observation without parsing display
text.

### 3. Wire StoreObserver into `roko status`

In `crates/roko-cli/src/commands/util.rs::cmd_status`, the active status command currently
populates `signal_count` from `all.len()`. Replace that count source with an `Observe` call:

```rust
use roko_core::traits::Observe;

/// If a store is available, use StoreObserver to get the signal count.
pub async fn observe_store_stats(store: &Arc<dyn Store>) -> Option<usize> {
    let observer = StoreObserver::new(Arc::clone(store));
    let ctx = Context::now();
    match observer.observe(&ctx).await {
        Ok(observations) => {
            // Extract signal_count from the observation body
            observations.first().and_then(|obs| {
                // Parse the JSON body for "signal_count"
                // ...
            })
        }
        Err(_) => None,
    }
}
```

Then use the extracted count in both branches of `cmd_status`: the JSON `SessionStatus` branch
and the human output branch. Keep the rest of status output unchanged.

**Trace the call path first**:
```bash
grep -rn 'signal_count' crates/roko-cli/src/status.rs
grep -rn 'SessionStatus' crates/roko-cli/src/main.rs | head -10
```

Find where `signal_count` is currently set in `commands/util.rs` and wire the observer there.

### 4. Add unit test for StoreObserver

```rust
#[tokio::test]
async fn store_observer_returns_count() {
    let store = /* in-memory store with 5 signals */;
    let observer = StoreObserver::new(Arc::new(store));
    let ctx = Context::now();
    let obs = observer.observe(&ctx).await.unwrap();
    assert_eq!(obs.len(), 1);
    // verify the body contains signal_count: 5
}
```

### 5. Export StoreObserver from roko-core

Add to `lib.rs`:
```rust
pub mod store_observer;
pub use store_observer::StoreObserver;
```

## Mechanical Implementation Plan

1. Change `Observe` to `async fn observe(&self, ctx: &Context) -> Result<Vec<Engram>>`
   or `Result<Vec<Signal>>` if Task 037 has made `Signal` canonical.
2. Add `StoreObserver { store: Arc<dyn Store> }` in `store_observer.rs`.
3. Implement `Cell` for `StoreObserver` with a stable id/name and
   `protocols() == &["Observe"]`.
4. Implement `Observe` by calling `self.store.len().await?` and `self.store.name()`.
5. Export `StoreObserver` from `roko-core/src/lib.rs`.
6. In `commands/util.rs::cmd_status`, wrap the already-opened `FileSubstrate` in `Arc`,
   construct `StoreObserver`, call `Observe::observe(&observer, &ctx).await?`, extract
   `signal_count`, and feed that value into existing JSON and human output.
7. Add a focused core unit test and update/add one CLI status test that would fail if the
   observer call path were removed.

Expected runtime path after wiring:

`roko status` -> `main.rs` `Command::Status` -> `commands::status::cmd_status` ->
`commands::util::cmd_status` -> `StoreObserver::new(Arc<dyn Store>)` ->
`Observe::observe(...).await` -> `Store::len().await`.

## What NOT to Do

- Do NOT implement Observe on every possible type. Just StoreObserver for now.
- Do NOT create an ObserverRegistry. The Graph engine (Phase 2) will manage that.
- Do NOT change how `roko status` works for episode_count, cfactor, or other fields.
  Only wire the store signal_count through the Observe trait.
- Do NOT add Observe to the universal loop (loop_tick.rs). Observe is called on-demand,
  not per-tick.
- Do NOT break the existing `roko status` output format. The same information should appear;
  it's just sourced through the Observe trait now.
- Do NOT only edit `crates/roko-cli/src/status.rs`; the active CLI command is in
  `crates/roko-cli/src/commands/util.rs`.
- Do NOT add blocking wrappers around async store calls.
- Do NOT use nonexistent APIs such as `Engram::builder()` without a `Kind`,
  `Kind::Event`, `.author(...)`, or `RokoError::Other`.

## Wire Target

```bash
cargo run -p roko-cli -- status
# Should show signal count sourced through StoreObserver::observe()
# Output should include "signals: N" (or equivalent) in the status report
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- status` — shows signal count
- [ ] `cargo test -p roko-core -- store_observer` — observer test passes
- [ ] `grep -rn 'StoreObserver\|Observe::observe\|\.observe(' crates/roko-cli/src/commands crates/roko-cli/src/main.rs --include='*.rs' | grep -v target/ | grep -v test` — shows at least one callsite in the active status path
- [ ] `grep -rn 'fn observe' crates/roko-core/src/traits.rs` — trait method is async
- [ ] Existing `roko status` output is unchanged (same fields, same format)

## Status Log

| Time | Agent | Action |
|------|-------|--------|
