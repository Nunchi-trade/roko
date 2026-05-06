# Task 042: Phase 1 Integration Test — Exercise All New Abstractions Together

```toml
id = 42
title = "Phase 1 integration test: Cell execute + Signal rename + Observe/Connect/Trigger protocols"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "medium"
blocked_by = [35, 36, 37, 39, 40, 41]
touches = [
    "crates/roko-core/tests/phase1_integration.rs",
    "crates/roko-cli/tests/phase1_protocols.rs",
    "crates/roko-cli/src/doctor.rs",
]
exclusive_files = [
    "crates/roko-core/tests/phase1_integration.rs",
    "crates/roko-cli/tests/phase1_protocols.rs",
    "crates/roko-cli/src/doctor.rs",
]
estimated_minutes = 120
```

## Context

Tasks 035-041 each add a piece of Phase 1: Cell execute, Signal rename, Observe, Connect,
and Trigger. This task creates integration tests that exercise them together, proving the
pieces compose correctly and the system remains coherent after all the changes.

This is NOT a "test-only" task with no runtime impact. The integration tests serve as the
reference for how Phase 2 (Graph engine) will use these abstractions. They prove:

1. A Cell can be created, given a CellContext, and executed
2. Signals flow through the system (renamed from Engram)
3. An observer can be queried for system state
4. A connection can be health-checked
5. A trigger can arm, check pulses, and fire

This task also wires a `roko doctor` check that verifies the v2 abstractions are functional
(Cell::execute works, protocols are registered, etc.).

Checklist: Final verification of P1-1 through P1-14.

## Background

Read the implementations from previous tasks:

1. `crates/roko-core/src/cell.rs` — Cell with execute(), CellContext, TypeSchema (task 035)
2. `crates/roko-core/src/engram.rs` — Signal struct (formerly Engram) (task 037)
3. `crates/roko-core/src/traits.rs` — redesigned Observe, Connect, Trigger (tasks 039-041)
4. `crates/roko-core/src/store_observer.rs` — StoreObserver (task 039)
5. `crates/roko-core/src/bus_trigger.rs` — BusTrigger (task 041)
6. `crates/roko-gate/src/compile.rs` — CompileGate with execute() (task 036)

Also read:
7. `crates/roko-std/src/memory.rs` — MemorySubstrate (in-memory Store for tests)
8. `crates/roko-core/src/bus_backends.rs` — BroadcastBus, MemoryBus (for test CellContext)

Dependency and source notes:

- This task must run after Tasks 035-041 are merged. If `CellContext`,
  `TypeSchema`, `Cell::execute`, `Signal`, `StoreObserver`, `ProviderConnection`,
  or `BusTrigger` are missing, stop and report the missing dependency instead of
  reimplementing it here.
- In the current baseline, `roko-core` does not depend on `roko-std` for tests.
  Do not add a new dev-dependency only for `MemorySubstrate`; use a tiny local
  test `Store` implementation unless the dependency already exists after prior
  tasks.
- Existing CLI tests use `assert_cmd::Command::cargo_bin("roko")`,
  `tempfile::tempdir`, and bootstrap patterns in
  `crates/roko-cli/tests/doctor.rs` and `crates/roko-cli/tests/e2e.rs`.
- The active doctor path is `main.rs` `Command::Doctor` ->
  `commands::util::cmd_doctor` -> `roko_cli::doctor::run_doctor`.
- `Engram::builder` currently requires a `Kind`. If Task 037 made `Signal`
  canonical, use `Signal::builder(Kind::...)`; otherwise use the public alias
  that exists after Task 037.

## What to Change

### 1. Create `crates/roko-core/tests/phase1_integration.rs`

This test file exercises all new roko-core abstractions together:

```rust
//! Phase 1 integration test — verifies all v2 core abstractions work together.

use roko_core::*;
use roko_core::traits::*;
use std::sync::Arc;

/// Test: Create a custom Cell, execute it, and verify output signals.
#[tokio::test]
async fn cell_execute_produces_signals() {
    // Create a simple Cell that transforms input signals
    struct DoubleCell;
    impl Cell for DoubleCell {
        fn cell_id(&self) -> &str { "double" }
        fn cell_name(&self) -> &str { "Double Cell" }
        fn protocols(&self) -> &[&str] { &[] }

        // Override execute to double the input signals
        async fn execute(&self, input: Vec<Signal>, _ctx: &CellContext) -> Result<Vec<Signal>> {
            let mut output = input.clone();
            output.extend(input);
            Ok(output)
        }
    }

    let ctx = /* build CellContext with MemoryBus + local test Store + CancellationToken */;
    let cell = DoubleCell;
    let input = vec![Signal::builder(Kind::Metric).build()];

    let output = cell.execute(input, &ctx).await.unwrap();
    assert_eq!(output.len(), 2);
}

/// Test: Default Cell::execute() returns an error.
#[tokio::test]
async fn default_execute_returns_error() {
    struct StubCell;
    impl Cell for StubCell {
        fn cell_id(&self) -> &str { "stub" }
        fn cell_name(&self) -> &str { "Stub" }
    }

    let ctx = /* build CellContext */;
    let result = StubCell.execute(vec![], &ctx).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not implemented"));
}

/// Test: TypeSchema compatibility checks.
#[test]
fn type_schema_compatibility() {
    assert!(TypeSchema::Any.is_compatible_with(&TypeSchema::Any));
    assert!(TypeSchema::OfKind(Kind::Metric).is_compatible_with(&TypeSchema::Any));
    assert!(TypeSchema::Any.is_compatible_with(&TypeSchema::OfKind(Kind::Metric)));
    assert!(TypeSchema::OfKind(Kind::Metric).is_compatible_with(&TypeSchema::OfKind(Kind::Metric)));
    // Different kinds are incompatible
    // assert!(!TypeSchema::OfKind(Kind::Metric).is_compatible_with(&TypeSchema::OfKind(Kind::Task)));
    // ^ Adjust based on actual Kind variants
}

/// Test: StoreObserver observes store statistics.
#[tokio::test]
async fn store_observer_reports_signal_count() {
    let store = /* in-memory store */;
    // Put some signals in the store
    // ...

    let observer = StoreObserver::new(Arc::new(store));
    let ctx = Context::now();
    let observations = observer.observe(&ctx).await.unwrap();

    assert_eq!(observations.len(), 1);
    // Verify the observation contains the correct count
}

/// Test: BusTrigger arm/check/disarm lifecycle.
#[tokio::test]
async fn trigger_lifecycle() {
    let trigger = BusTrigger::new("test", TopicFilter::Exact("test.event".into()));
    let ctx = Context::now();

    // Arm
    let binding = trigger.arm(&ctx).await.unwrap();
    assert!(binding.description.contains("test"));

    // Check with matching pulse -> fires
    let pulse = /* create a pulse with topic "test.event" */;
    let fired = trigger.check(&[pulse], &ctx).await.unwrap();
    assert!(fired.is_some());

    // Disarm
    trigger.disarm(&ctx).await.unwrap();

    // Check after disarm -> does not fire
    let fired = trigger.check(&[pulse], &ctx).await.unwrap();
    assert!(fired.is_none());
}

/// Test: Signal is the canonical type name (Engram is deprecated alias).
#[test]
fn signal_is_canonical() {
    let signal = Signal::builder(Kind::Metric)
        .build();
    assert!(!signal.cell_id.is_empty() || true); // Just verify it compiles as Signal
}

/// Test: Full pipeline — observe state, trigger on observation, execute cell.
#[tokio::test]
async fn observe_trigger_execute_pipeline() {
    // 1. Observe store state
    let store = /* in-memory store with 3 signals */;
    let observer = StoreObserver::new(Arc::clone(&store));
    let ctx = Context::now();
    let observations = observer.observe(&ctx).await.unwrap();
    assert!(!observations.is_empty());

    // 2. Create a trigger that fires on observation events
    let trigger = BusTrigger::new(
        "on-observation",
        TopicFilter::Prefix("observe.".into()),
    );
    trigger.arm(&ctx).await.unwrap();

    // 3. Simulate: convert observation to a pulse and check trigger
    // (In the real system, the Bus would deliver the pulse)
    let pulse = /* convert observation signal to pulse */;
    let fired = trigger.check(&[pulse], &ctx).await.unwrap();
    assert!(fired.is_some());

    // 4. Execute a cell with the trigger output
    struct LogCell;
    impl Cell for LogCell {
        fn cell_id(&self) -> &str { "log" }
        fn cell_name(&self) -> &str { "Log Cell" }

        async fn execute(&self, input: Vec<Signal>, _ctx: &CellContext) -> Result<Vec<Signal>> {
            // Just pass through — proves the pipeline connects
            Ok(input)
        }
    }

    let cell_ctx = /* build CellContext */;
    let trigger_output = fired.unwrap();
    let result = LogCell.execute(trigger_output, &cell_ctx).await.unwrap();
    assert!(!result.is_empty());
}
```

### 2. Create `crates/roko-cli/tests/phase1_protocols.rs`

This test verifies the CLI-level wiring:

```rust
//! Phase 1 protocol wiring tests — verifies CLI commands use the new protocol traits.

/// Verify `roko status` uses StoreObserver (not just file reading).
#[test]
fn status_command_uses_observe_trait() {
    // Check that status.rs imports StoreObserver
    // This is a compile-time / grep verification
    // The actual functional test is in the roko-core integration test
}

/// Verify `roko config providers health` uses Connect::health().
#[test]
fn provider_health_uses_connect_trait() {
    // Similar compile-time verification
}
```

**Note**: If functional CLI tests are feasible (e.g., running roko as a subprocess), prefer
those. But CLI integration tests are often flaky. Adjust based on what the existing test
patterns look like:
```bash
ls crates/roko-cli/tests/
```

### 3. Wire a `roko doctor` v2-abstractions check

In the existing `roko doctor` command, add a check that verifies the v2 abstractions are
functional:

```rust
// In the doctor command handler:
println!("v2 abstractions:");
println!("  Cell::execute() ... ok (trait method exists)");
println!("  Signal type ...... ok (canonical name)");
println!("  Observe protocol . ok (StoreObserver registered)");
println!("  Connect protocol . ok (ProviderConnection available)");
println!("  Trigger protocol . ok (BusTrigger available)");
```

This is a lightweight check — just verify the types exist and can be instantiated. It
proves the abstractions compiled and are reachable from the CLI crate.

Make the check concrete:

- Add a `v2_abstractions` `DoctorCheck` in `crates/roko-cli/src/doctor.rs`.
- Keep it deterministic: no provider network calls, daemon calls, or external commands.
- Compile-reference the public types added by dependencies: `CellContext`, `TypeSchema`,
  `Signal`/`Engram`, `StoreObserver`, `ProviderConnection`, `BusTrigger`, and the
  `Observe`, `Connect`, and `Trigger` traits.
- Verify runtime wiring by either calling small exported probe helpers from Tasks 039-041 or by
  tightly scoped source-text checks for the expected command callsites. This check should fail
  if the traits compile but no visible command path calls them.
- Expected human output should include a stable id, for example:
  `[ok] v2_abstractions: phase 1 protocol abstractions are reachable`.

Find the doctor command:
```bash
grep -rn 'doctor\|Doctor' crates/roko-cli/src/main.rs | head -10
```

## Mechanical Core Test Plan

In `crates/roko-core/tests/phase1_integration.rs`:

1. Import public core types from the completed dependency tasks.
2. Define local test doubles only where needed to avoid network and new crate dependencies.
3. Assert metadata/schema for representative cells: stable name/id, expected protocol strings,
   and non-empty schema data if Task 035 exposes it.
4. Assert execute behavior with a trivial `Cell::execute` implementation that transforms one
   input signal into a deterministic output.
5. Assert observe -> trigger composition:
   - call `StoreObserver::observe`;
   - create a `Pulse::new(1, Topic::new("observe.store.stats"), Kind::Metric, Body::Json(...))`;
   - arm `BusTrigger` with `TopicFilter::Prefix("observe.".into())`;
   - call `Trigger::check` and assert one output.
6. Assert connect behavior with a fake/local `Connect` implementation or the deterministic
   no-network branch of `ProviderConnection`.

## Mechanical CLI Test Plan

In `crates/roko-cli/tests/phase1_protocols.rs`:

1. Create a temp workdir using the same layout/bootstrap pattern as existing doctor/e2e tests.
2. Write a minimal `roko.toml` with one provider that has a deliberately missing API key and
   one subscription trigger such as `observe:*` or `observe.store.*`.
3. Run `roko status --workdir <dir> --json` and assert JSON still contains `signal_count`.
4. Run `roko config providers health --workdir <dir>` and assert the live connection section
   appears with deterministic missing-key/unhealthy output.
5. Run `roko config subscriptions list --workdir <dir> --json` and assert the trigger
   binding/filter field appears.
6. Run `roko doctor --workdir <dir>` and assert stdout contains `v2_abstractions`.

## What NOT to Do

- Do NOT test individual protocol implementations in detail. Tasks 035-041 have their own
  unit tests. This task tests composition and integration.
- Do NOT create test infrastructure that duplicates existing test helpers. Use what exists, but
  prefer a local test double over adding a new cross-crate dev-dependency.
- Do NOT add performance tests or benchmarks. This is correctness only.
- Do NOT modify unrelated implementation code. The only source implementation file in scope is
  `crates/roko-cli/src/doctor.rs` for the doctor check.
- Do NOT use network-dependent tests or require real provider credentials.
- Do NOT reimplement missing dependencies from Tasks 035-041 in this task.
- Do NOT assert exact human table spacing when a stable JSON mode is available.

## Wire Target

```bash
# Integration tests
cargo test -p roko-core -- phase1_integration
cargo test -p roko-cli -- phase1_protocols

# Doctor check
cargo run -p roko-cli -- doctor
# Should include "v2 abstractions: ... ok" lines
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo test -p roko-core -- phase1_integration` — all integration tests pass
- [ ] `cargo test -p roko-cli -- phase1_protocols` — CLI protocol tests pass
- [ ] `cargo run -p roko-cli -- doctor` — shows v2 abstraction checks
- [ ] The observe -> trigger -> execute pipeline test proves end-to-end composition
- [ ] No test uses deprecated `Engram` name (all use `Signal`)
- [ ] `grep -rn 'v2_abstractions' crates/roko-cli/src/doctor.rs crates/roko-cli/tests/phase1_protocols.rs` — doctor check and test use the stable id

## Status Log

| Time | Agent | Action |
|------|-------|--------|
