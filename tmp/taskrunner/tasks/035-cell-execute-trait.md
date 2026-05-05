# Task 035: Add execute() to Cell Trait — CellContext, TypeSchema, Default Impl

```toml
id = 35
title = "Add execute() default method to Cell trait with CellContext and TypeSchema"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-core/src/cell.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-core/Cargo.toml",
]
exclusive_files = ["crates/roko-core/src/cell.rs"]
estimated_minutes = 120
```

## Context

The Cell trait is the universal computation unit. Every protocol trait (Store, Score, Verify,
Route, Compose, React) requires Cell as a supertrait. But Cell currently only provides identity
and metadata — it has no execution method. V2 needs Cell to be executable so the future
Graph engine can call `cell.execute(input, ctx)` on any Cell without knowing its protocol.

This task adds three things to roko-core:
1. **CellContext** — the runtime context passed to execute() (bus, store, cancel token)
2. **TypeSchema** — input/output schema for edge validation between cells
3. **execute()** — async default method on Cell that returns an error (override to implement)

This is the foundation for all subsequent Phase 1 and Phase 2 work.

Checklist items: P1-1, P1-2, P1-3.

## Background

Read these files before starting:

1. `crates/roko-core/src/cell.rs` — current Cell trait (6 methods, all metadata)
2. `crates/roko-core/src/context.rs` — existing Context struct (CellContext is different)
3. `crates/roko-core/src/traits.rs` — the six protocol traits that require Cell as supertrait
4. `crates/roko-core/src/lib.rs` — what's currently exported
5. `crates/roko-core/Cargo.toml` — check dependencies (need async_trait, tokio_util for CancellationToken)
6. `tmp/v2-refactoring/04-CELL-EXECUTE.md` — the design spec for this change

## What to Change

### 1. Add CellContext struct to `crates/roko-core/src/cell.rs`

```rust
use std::sync::Arc;
use crate::traits::{Bus, Store};
use tokio_util::sync::CancellationToken;

/// Runtime context passed to Cell::execute(). Provides access to
/// shared infrastructure without cells needing to manage their own.
pub struct CellContext {
    /// Pub/sub transport for ephemeral Pulses.
    pub bus: Arc<dyn Bus<Receiver = tokio::sync::broadcast::Receiver<crate::Pulse>>>,
    /// Durable storage for Signals.
    pub store: Arc<dyn Store>,
    /// Cancellation token for cooperative shutdown.
    pub cancel: CancellationToken,
    /// Trace context for observability.
    pub trace_id: Option<String>,
    /// Run identifier (if executing within a Graph/Flow).
    pub run_id: Option<String>,
    /// Remaining budget for this execution (USD).
    pub budget_remaining: Option<f64>,
}
```

**Important**: The Bus trait has an associated type `Receiver`. CellContext needs to erase that
type. The cleanest approach: use `Arc<dyn BusErased>` (which already exists in roko-core as
`bus_backends::BusErased`) or define CellContext with a generic. Check `bus_backends.rs` to see
what's already available. Use the existing `BusErased` wrapper if it fits; otherwise use a
boxed trait object. Do NOT create a duplicate bus abstraction.

Add a constructor:
```rust
impl CellContext {
    pub fn new(bus: Arc<dyn BusErased>, store: Arc<dyn Store>, cancel: CancellationToken) -> Self {
        Self { bus, store, cancel, trace_id: None, run_id: None, budget_remaining: None }
    }
}
```

### 2. Add TypeSchema enum to `crates/roko-core/src/cell.rs`

```rust
use serde::{Serialize, Deserialize};
use crate::Kind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeSchema {
    /// Accepts any input.
    Any,
    /// Accepts signals of a specific Kind.
    OfKind(Kind),
    /// Accepts signals matching a JSON schema string.
    JsonSchema(String),
}

impl TypeSchema {
    /// Check if an output of type `self` is compatible as input to a cell expecting `target`.
    pub fn is_compatible_with(&self, target: &TypeSchema) -> bool {
        match (self, target) {
            (_, TypeSchema::Any) => true,
            (TypeSchema::Any, _) => true,
            (TypeSchema::OfKind(a), TypeSchema::OfKind(b)) => a == b,
            _ => false,
        }
    }
}
```

### 3. Add execute() and schema methods to Cell trait

```rust
use crate::Engram;
use crate::error::Result;

#[async_trait::async_trait]
pub trait Cell: Send + Sync + 'static {
    // --- existing methods unchanged ---
    fn cell_id(&self) -> &str;
    fn cell_name(&self) -> &str;
    fn cell_version(&self) -> CellVersion { (0, 1, 0) }
    fn protocols(&self) -> &[&str] { &[] }
    fn estimated_cost(&self) -> Option<f64> { None }
    fn estimated_duration(&self) -> Option<Duration> { None }

    // --- new in v2 ---
    fn input_schema(&self) -> Option<&TypeSchema> { None }
    fn output_schema(&self) -> Option<&TypeSchema> { None }

    /// Execute this cell. Default returns an error — override in implementations.
    async fn execute(
        &self,
        input: Vec<Engram>,
        ctx: &CellContext,
    ) -> Result<Vec<Engram>> {
        let _ = (input, ctx);
        Err(crate::error::RokoError::Invalid(
            format!("{}: execute() not implemented", self.cell_name())
        ))
    }
}
```

**Critical**: Adding `#[async_trait]` to Cell changes it from a regular trait to an async
trait. This will require every existing `impl Cell for X` to also use `#[async_trait]`. Check
how many Cell implementations exist:
```bash
grep -rn 'impl Cell for' crates/ --include='*.rs' | grep -v target/
```
If there are many, consider making execute() a separate trait (ExecutableCell: Cell) to avoid
churn. But the design spec says it goes on Cell — follow the spec unless implementation proves
it infeasible, then document why in the Status Log.

### 4. Export from lib.rs

Add `CellContext` and `TypeSchema` to the `pub use cell::*` line (which already re-exports
everything from cell.rs). Verify these names don't collide with existing exports.

### 5. Add integration test

Create a test in `crates/roko-core/tests/` (or add to an existing test file) that:
1. Creates a struct implementing Cell with a custom execute()
2. Creates a CellContext (using MemoryBus or BroadcastBus + an in-memory store)
3. Calls execute() and verifies the output
4. Calls execute() on the DEFAULT impl and verifies it returns an error

## What NOT to Do

- Do NOT implement execute() on any existing types (gates, scorers, etc.). That's task 036.
- Do NOT add fields to CellContext beyond what's listed above. Start minimal.
- Do NOT build a CellRegistry. That comes in Phase 2.
- Do NOT change the existing Context struct. CellContext is separate (Cell-execution-specific;
  Context is the legacy scoring/routing context).
- Do NOT add runtime wiring. This task is trait-level only. Callers come in Phase 2.

## Wire Target

This task is a trait addition — it has no CLI wire target yet (that comes with Phase 2's
`roko graph run`). The wire target is the integration test:

```bash
cargo test -p roko-core -- cell_execute
# Should show: test for custom execute() passing, default execute() returning error
```

## Verification

- [ ] `cargo build --workspace` — all existing Cell impls still compile
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo test -p roko-core -- cell_execute` — new integration test passes
- [ ] `grep -rn 'CellContext' crates/roko-core/src/ --include='*.rs' | grep -v target/` — struct exists and is exported
- [ ] `grep -rn 'TypeSchema' crates/roko-core/src/ --include='*.rs' | grep -v target/` — enum exists and is exported
- [ ] `grep -rn 'fn execute' crates/roko-core/src/cell.rs` — method exists on Cell trait
- [ ] No existing `impl Cell for` blocks broke (check every crate that implements Cell)

## Implementation Detail

### Current API facts

- `crates/roko-core/src/cell.rs` currently contains only `CellId`,
  `CellVersion`, and the metadata-only `Cell` trait.
- `Bus` is not object-safe because it has an associated `Receiver` type.
  `crates/roko-core/src/bus_backends.rs` already defines `BusErased`; use
  `Arc<dyn BusErased>` in `CellContext`.
- `Store` is object-safe via `#[async_trait]` and can be stored as
  `Arc<dyn Store>`.
- `RokoError::Other` does not exist. The default `execute()` error should use
  `RokoError::Invalid(format!("{}: execute() not implemented", self.cell_name()))`.
- The protocol traits in `traits.rs` currently do not have `Cell` as a Rust
  supertrait even though comments describe that architecture. Do not add
  `: Cell` to `Store`, `Score`, `Verify`, `Route`, `Compose`, or `React` in
  this task; that is a broader compatibility change.
- Existing `impl roko_core::Cell for ...` blocks are in:
  `roko-learn/src/cascade_router.rs`, `roko-fs/src/file_substrate.rs`,
  `roko-std/src/memory.rs`, `roko-std/src/noop.rs`, and many files under
  `roko-gate/src/`. Because `execute()` has a default implementation, these
  impls should not need code changes unless the compiler requires an
  `#[async_trait]` impl annotation.

### Mechanical steps

1. In `crates/roko-core/Cargo.toml`, add:
   ```toml
   tokio-util = { workspace = true }
   ```
   `tokio` already has the `sync` feature and `async-trait` already exists.

2. In `crates/roko-core/src/cell.rs`, add imports:
   ```rust
   use std::sync::Arc;
   use async_trait::async_trait;
   use serde::{Deserialize, Serialize};
   use tokio_util::sync::CancellationToken;
   use crate::bus_backends::BusErased;
   use crate::{Engram, Kind, Store};
   use crate::error::{Result, RokoError};
   ```
   If task 037 has already landed in the worktree, use `crate::Signal` in the
   new API instead of `crate::Engram`. Otherwise `Engram` is acceptable and the
   deprecated alias migration will handle it.

3. Add `CellContext` before the trait:
   ```rust
   pub struct CellContext {
       pub bus: Arc<dyn BusErased>,
       pub store: Arc<dyn Store>,
       pub cancel: CancellationToken,
       pub trace_id: Option<String>,
       pub run_id: Option<String>,
       pub budget_remaining: Option<f64>,
   }

   impl CellContext {
       #[must_use]
       pub fn new(
           bus: Arc<dyn BusErased>,
           store: Arc<dyn Store>,
           cancel: CancellationToken,
       ) -> Self {
           Self {
               bus,
               store,
               cancel,
               trace_id: None,
               run_id: None,
               budget_remaining: None,
           }
       }
   }
   ```
   Do not derive `Debug` unless you first confirm `dyn Store` and
   `dyn BusErased` support it.

4. Add `TypeSchema` before the trait:
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   pub enum TypeSchema {
       Any,
       OfKind(Kind),
       JsonSchema(String),
   }

   impl TypeSchema {
       #[must_use]
       pub fn is_compatible_with(&self, target: &TypeSchema) -> bool {
           match (self, target) {
               (_, TypeSchema::Any) => true,
               (TypeSchema::Any, _) => true,
               (TypeSchema::OfKind(a), TypeSchema::OfKind(b)) => a == b,
               _ => false,
           }
       }
   }
   ```

5. Add `#[async_trait]` above `pub trait Cell`.

6. Add default schema methods to `Cell` after `estimated_duration()`:
   ```rust
   fn input_schema(&self) -> Option<&TypeSchema> { None }
   fn output_schema(&self) -> Option<&TypeSchema> { None }
   ```

7. Add the async default `execute()` method to `Cell`:
   ```rust
   async fn execute(
       &self,
       input: Vec<Engram>,
       ctx: &CellContext,
   ) -> Result<Vec<Engram>> {
       let _ = (input, ctx);
       Err(RokoError::Invalid(format!(
           "{}: execute() not implemented",
           self.cell_name()
       )))
   }
   ```

8. `crates/roko-core/src/lib.rs` already does `pub use cell::*;`; no explicit
   export line is needed. Verify `CellContext` and `TypeSchema` are visible via
   `roko_core::{CellContext, TypeSchema}`.

### Tests to add

Add `crates/roko-core/tests/cell_execute.rs` (the task metadata may need to be
expanded if the runner enforces `touches` strictly):

- Define a local `TestStore` implementing `Store` with no-op async methods.
- Define `EchoCell` implementing `Cell` and overriding `execute()` to return
  its input. Put `#[async_trait::async_trait]` on this impl because it
  overrides an async trait method.
- Define `DefaultOnlyCell` implementing `Cell` without overriding `execute()`.
- Build a context with:
  ```rust
  let bus: Arc<dyn BusErased> = Arc::new(MemoryBus::new(16));
  let store: Arc<dyn Store> = Arc::new(TestStore::default());
  let ctx = CellContext::new(bus, store, CancellationToken::new());
  ```
- Assert `EchoCell.execute(vec![signal], &ctx).await` returns the signal.
- Assert `DefaultOnlyCell.execute(Vec::new(), &ctx).await` returns
  `RokoError::Invalid` containing `execute() not implemented`.
- Add `TypeSchema` compatibility assertions for `Any`, matching `OfKind`, and
  mismatched `OfKind`.

### Verification details

Run these targeted checks before workspace-wide checks:

```bash
cargo test -p roko-core --test cell_execute
rg -n "pub struct CellContext|pub enum TypeSchema|async fn execute" crates/roko-core/src/cell.rs
rg -n "tokio-util" crates/roko-core/Cargo.toml
```

Then run the task's full verification list.

### Anti-patterns

- Do not introduce a second `ExecutableCell` trait; the task is specifically
  to add the default execution method to `Cell`.
- Do not use `Arc<dyn Bus<...>>`; that reopens the associated-type object
  safety problem already solved by `BusErased`.
- Do not add `Cell` supertrait bounds to protocol traits in `traits.rs`.
- Do not edit gate implementations in this task; task 036 handles overrides.
- Do not add `#[allow(deprecated)]` or other lint suppressions for this change.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
