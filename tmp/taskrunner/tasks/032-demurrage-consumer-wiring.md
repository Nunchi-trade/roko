# Task 032: Wire DemurrageConsumer to Periodic Store Pruning in `roko serve`

```toml
id = 32
title = "Wire DemurrageConsumer into roko serve as a periodic tokio interval task"
track = "wiring"
wave = "wave-2"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-runtime/src/demurrage_consumer.rs",
    "crates/roko-cli/src/serve_runtime.rs",
    "crates/roko-serve/src/lib.rs",
    "crates/roko-serve/src/state.rs",
]
exclusive_files = ["crates/roko-serve/src/lib.rs"]
estimated_minutes = 120
```

## Context

`DemurrageConsumer` in roko-runtime is a fully implemented periodic decay engine that
reduces confidence on knowledge entries over time. It has zero callers outside its own
tests. The natural runtime host is `roko serve`, which already runs a long-lived tokio
runtime with interval tasks.

The KnowledgeStore (roko-neuro) has `apply_demurrage()` which does the actual decay. The
DemurrageConsumer wraps this with configurable intervals and domain-specific multipliers.

Currently the DemurrageConsumer is the only caller-free module in roko-runtime that has a
clear runtime home.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- DCA-1
- `tmp/v2-refactoring/01-CURRENT-STATE.md` -- DemurrageConsumer listed as floating

## Background

Read these files first:
1. `crates/roko-runtime/src/demurrage_consumer.rs` -- DemurrageConsumer, DemurrageConsumerConfig, tick()
2. `crates/roko-neuro/src/knowledge_store.rs` -- KnowledgeStore::apply_demurrage() (around line 1173)
3. `crates/roko-serve/src/lib.rs` -- ServerBuilder::start_background() and existing interval task helpers
4. `crates/roko-serve/src/state.rs` -- AppState used by the serve routes
5. `crates/roko-cli/src/serve_runtime.rs` -- read only to confirm it is not the long-lived
   `roko serve` background-task host

Important current-state corrections:
- The long-lived `roko serve` background tasks are spawned in
  `crates/roko-serve/src/lib.rs` (`ServerBuilder::start_background`), not in
  `crates/roko-cli/src/serve_runtime.rs`.
- `roko-serve` already depends on both `roko-runtime` and `roko-neuro`; do not add new
  crate dependencies for this wiring.
- `DemurrageConsumer::tick(&[DemurrageEntry])` produces schedule/report semantics for
  runtime entries. The real persisted knowledge-store mutation is currently
  `KnowledgeStore::apply_demurrage()`. Do not pretend `tick()` alone writes the store.
- `KnowledgeStore::apply_demurrage()` currently computes elapsed time from each entry's
  creation time. Repeated frequent calls may tax the same elapsed window more than once.
  If the implementation cannot make demurrage incremental or cadence-safe, report that
  blocker instead of wiring an over-taxing loop.

## What to Change

1. **Add a demurrage background task** to `ServerBuilder::start_background()` in
   `crates/roko-serve/src/lib.rs`, next to `start_state_snapshot_saver`,
   `start_workspace_gc`, and `start_cold_archival_timer`. Prefer a helper shaped like:
   ```rust
   fn start_demurrage_timer(state: Arc<AppState>) -> tokio::task::JoinHandle<()>
   ```
   The task should:
   - Creates a `DemurrageConsumer` with default config
   - Constructs `KnowledgeStore::for_workdir(&state.workdir)` inside the task and logs any
     initialization error without blocking server startup
   - Uses `tokio::select!` on `state.cancel.cancelled()` and an interval tick, matching the
     existing background task style
   - Calls a small helper for one demurrage pass so tests can exercise it without sleeping

2. **Use the existing store mutation path deliberately**:
   - Use `DemurrageConsumer` as the interval/validation gate.
   - When the consumer says a demurrage pass is due, call `KnowledgeStore::apply_demurrage()`
     to mutate persisted knowledge balances.
   - Do not manually read and rewrite store entries from `roko-serve`; `KnowledgeStore`'s
     rewrite helper is private and the store owns its file format.
   - If you must fix repeated-tax semantics, make that fix in `roko-neuro` with focused
     tests and document the expanded scope.

3. **Choose a cadence that matches `DemurrageConsumerConfig`**. Default
   `validation_interval = 250`; if the wall-clock interval is 40 seconds, demurrage is due
   about every 2.8 hours. If you choose a 5-minute wall-clock interval, the first default
   demurrage pass is about 20.8 hours later. Tests should use
   `DemurrageConsumerConfig { validation_interval: 1, ..Default::default() }`.

4. **Log demurrage events** at debug level:
   ```rust
   tracing::debug!(taxed = report.taxed_count, "demurrage pass completed");
   ```

## What NOT to Do

- Don't modify DemurrageConsumer's internals -- it's well-designed and tested.
- Don't add demurrage to the plan runner (that's a separate concern).
- Don't block the serve startup on demurrage initialization.
- Don't add new dependencies to roko-serve; it already has the needed crate deps.
- Don't add this to `RokoCliRuntime` and assume `roko serve` is covered. The actual server
  background-task host is `roko-serve/src/lib.rs`.
- Don't call `KnowledgeStore::apply_demurrage()` on every short interval unless the
  repeated-tax issue has been addressed.

## Wire Target

```bash
# Start the server and observe demurrage logs:
RUST_LOG=roko=debug cargo run -p roko-cli -- serve 2>&1 | grep -i demurrage
# With default validation_interval, do not expect a persisted decay after only a few
# short wall-clock ticks unless tests/config use validation_interval = 1.
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo test -p roko-runtime demurrage`
- [ ] `cargo test -p roko-neuro store_apply_demurrage`
- [ ] `cargo test -p roko-serve demurrage`
- [ ] `rg -n 'DemurrageConsumer' crates/ --glob '*.rs' --glob '!target/**' | grep -v 'roko-runtime/src/demurrage_consumer.rs'` -- shows at least one non-test callsite in `roko-serve/src/lib.rs`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
