# Task 036: Implement execute() on Gate Cells — CompileGate, TestGate, ClippyGate, DiffGate

```toml
id = 36
title = "Implement Cell::execute() on 4 gates: CompileGate, TestGate, ClippyGate, DiffGate"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "high"
blocked_by = [35]
touches = [
    "crates/roko-gate/src/compile.rs",
    "crates/roko-gate/src/test_gate.rs",
    "crates/roko-gate/src/clippy_gate.rs",
    "crates/roko-gate/src/diff_gate.rs",
]
exclusive_files = [
    "crates/roko-gate/src/compile.rs",
    "crates/roko-gate/src/test_gate.rs",
    "crates/roko-gate/src/clippy_gate.rs",
    "crates/roko-gate/src/diff_gate.rs",
]
estimated_minutes = 150
```

## Context

Task 035 added `Cell::execute()` with a default error impl. Now we need to prove the pattern
works by implementing `execute()` on real cells. Gates are the best first target because:
1. They already implement both `Cell` and `Verify`
2. Their `verify()` method takes an Engram and returns a Verdict
3. The execute() wrapper converts: `Vec<Engram> in -> verify() -> Verdict -> Vec<Engram> out`

This task implements execute() on 4 gates: CompileGate, TestGate, ClippyGate, DiffGate. These
are the 4 most-used gates in the system.

Checklist items: P1-4, P1-5.

## Background

Read these files before starting:

1. `crates/roko-gate/src/compile.rs` — CompileGate struct and Verify impl
2. `crates/roko-gate/src/test_gate.rs` — TestGate struct and Verify impl
3. `crates/roko-gate/src/clippy_gate.rs` — ClippyGate struct and Verify impl
4. `crates/roko-gate/src/diff_gate.rs` — DiffGate struct and Verify impl
5. `crates/roko-core/src/cell.rs` — Cell trait with execute() (after task 035)
6. `crates/roko-core/src/verdict.rs` — Verdict struct (the output of verify())
7. `crates/roko-core/src/engram.rs` — Engram struct, especially any factory methods

Also check:
```bash
grep -rn 'impl Cell for' crates/roko-gate/ --include='*.rs' | grep -v target/
```
to see which gates already have Cell impls vs which need them added.

## What to Change

### 1. For each of the 4 gates, implement Cell::execute()

The pattern is the same for all 4:

```rust
#[async_trait::async_trait]
impl Cell for CompileGate {
    fn cell_id(&self) -> &str { "compile-gate" }
    fn cell_name(&self) -> &str { "Compile Gate" }
    fn protocols(&self) -> &[&str] { &["Verify"] }

    fn input_schema(&self) -> Option<&TypeSchema> {
        // Gates accept any engram (they examine the workspace, not the input)
        None
    }

    fn output_schema(&self) -> Option<&TypeSchema> {
        // Gates output verdict-kind signals
        None // Or TypeSchema::OfKind(Kind::GateVerdict) if Kind has that variant
    }

    async fn execute(
        &self,
        input: Vec<Engram>,
        ctx: &CellContext,
    ) -> Result<Vec<Engram>> {
        // Use the first input signal (or a synthetic empty one if input is empty)
        let engram = input.first()
            .cloned()
            .unwrap_or_else(|| Engram::default()); // or equivalent

        // Build a Context from CellContext for the verify() call
        let verify_ctx = Context::now();

        // Call the existing verify() method
        let verdict = self.verify(&engram, &verify_ctx).await;

        // Convert Verdict to an output Engram
        let output = verdict_to_engram(&verdict, self.cell_name());
        Ok(vec![output])
    }
}
```

### 2. Create a `verdict_to_engram` helper

Either in `roko-gate/src/lib.rs` or a new `roko-gate/src/cell_bridge.rs`:

```rust
/// Convert a gate Verdict into an Engram for Cell::execute() output.
fn verdict_to_engram(verdict: &Verdict, gate_name: &str) -> Engram {
    // Use EngramBuilder to construct an engram with:
    // - kind: Kind::GateVerdict (or appropriate kind)
    // - body: JSON serialization of the Verdict
    // - author: gate_name
    // Check what Kind variants exist first!
}
```

Check `crates/roko-core/src/kind.rs` for available Kind variants. If there's no
`GateVerdict` kind, use whatever kind existing gate code uses, or use `Kind::Event`.

### 3. Add integration tests per gate

In `crates/roko-gate/tests/` (create a new file `cell_execute.rs` if needed):

```rust
#[tokio::test]
async fn compile_gate_execute_returns_verdict_signal() {
    let gate = CompileGate::new(/* workspace path */);
    let ctx = /* create CellContext with test bus + memory store */;
    let result = gate.execute(vec![], &ctx).await;
    assert!(result.is_ok());
    let signals = result.unwrap();
    assert_eq!(signals.len(), 1);
    // Check the output is a verdict-shaped engram
}
```

**Note**: These tests may need a real workspace with Cargo.toml to compile/test against.
Use a temporary directory with a minimal Rust project, or use the roko workspace itself.
Check how existing gate tests handle this (look at `crates/roko-gate/tests/compile_real_project.rs`).

### 4. Register protocols() on each gate

Each gate's Cell impl should return `&["Verify"]` from `protocols()` so the future
CellRegistry can discover which protocols a cell supports.

## What NOT to Do

- Do NOT implement execute() on ALL gates. Only these 4. Other gates get execute() when
  they're wired into a Graph (Phase 2).
- Do NOT change the existing Verify trait or verify() method signatures.
- Do NOT modify how gates are called from the runner (event_loop.rs). The runner still calls
  verify() directly. execute() is an alternative entry point for Graph-based execution.
- Do NOT build a CellRegistry or Graph engine. That's Phase 2.

## Wire Target

Integration tests that prove execute() works on real gates:

```bash
cargo test -p roko-gate -- cell_execute
# Should show 4 passing tests (one per gate)
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo test -p roko-gate -- cell_execute` — 4 gate execute tests pass
- [ ] `grep -rn 'fn execute' crates/roko-gate/src/ --include='*.rs' | grep -v target/` — shows 4 impls
- [ ] Each gate's execute() calls its existing verify() method (not a reimplementation)
- [ ] Each gate's protocols() returns `&["Verify"]`

## Implementation Detail

### Current code facts

- All four target gates already have `impl roko_core::Cell for ...` blocks:
  - `crates/roko-gate/src/compile.rs::impl roko_core::Cell for CompileGate`
  - `crates/roko-gate/src/test_gate.rs::impl roko_core::Cell for TestGate`
  - `crates/roko-gate/src/clippy_gate.rs::impl roko_core::Cell for ClippyGate`
  - `crates/roko-gate/src/diff_gate.rs::impl roko_core::Cell for DiffGate`
- Do not add a second `impl Cell`; extend the existing impl blocks.
- `Kind::GateVerdict` already exists in `crates/roko-core/src/kind.rs`.
- `Engram::derive_verdict(Body)` already preserves lineage/tags and builds a
  `Kind::GateVerdict` signal. Use it when there is an input signal.
- `Body::from_json(&verdict)` returns `roko_core::Result<Body>` and can be
  used with `?` inside `execute()`.
- If task 037 has already landed in the worktree, use `Signal` instead of
  `Engram` in imports/signatures. Otherwise use the current `Engram` name;
  the alias migration keeps it compiling.

### Mechanical implementation pattern

For each target file:

1. Expand imports from `roko_core` to include the new Cell API and signal
   construction types:
   ```rust
   use roko_core::{
       Body, CellContext, Context, Engram, Kind, Provenance, Result, TypeSchema, Verdict, Verify,
   };
   ```
   Keep `TestCount` in `test_gate.rs`. If a file already imports only a subset,
   merge imports rather than adding a second `use roko_core::...` block.

2. Add `#[async_trait]` directly above the existing
   `impl roko_core::Cell for <Gate>` block, because the impl will now override
   an async trait method.

3. Add schema methods inside the existing `Cell` impl:
   ```rust
   fn input_schema(&self) -> Option<&TypeSchema> {
       None
   }

   fn output_schema(&self) -> Option<&TypeSchema> {
       None
   }
   ```
   Keep schema precision out of this task. Returning
   `Some(&TypeSchema::OfKind(Kind::GateVerdict))` directly would borrow a
   temporary and fail to compile; a future task can add static schemas if it
   needs precise graph validation.

4. Add `execute()` inside each existing `Cell` impl:
   ```rust
   async fn execute(
       &self,
       input: Vec<Engram>,
       _ctx: &CellContext,
   ) -> Result<Vec<Engram>> {
       let fallback = Engram::builder(Kind::Task)
           .body(Body::empty())
           .provenance(Provenance::agent(self.name()))
           .build();
       let signal = input.first().unwrap_or(&fallback);
       let verify_ctx = Context::now();
       let verdict = self.verify(signal, &verify_ctx).await;
       let body = Body::from_json(&verdict)?;
       let output = signal
           .derive_verdict(body)
           .provenance(Provenance::agent(self.name()))
           .tag("gate", verdict.gate.clone())
           .tag("passed", verdict.passed.to_string())
           .build();
       Ok(vec![output])
   }
   ```
   This intentionally delegates to `verify()` and does not duplicate gate
   logic. `_ctx` is currently unused; future Graph work may thread trace/budget
   data through it.

5. Do not add a shared helper module unless the task metadata is expanded to
   include `crates/roko-gate/src/lib.rs` and the new helper file. The current
   owned source touch list is the four gate files, so the inline pattern above
   avoids metadata drift.

### Tests to add

Add `crates/roko-gate/tests/cell_execute.rs` (the task metadata may need to be
expanded if touch enforcement is strict). Use existing fixture patterns from
`crates/roko-gate/tests/rungs.rs`:

- Helper `scaffold_cargo_project(root, lib_rs, extra_files)` for a minimal
  Cargo project.
- Helper `gate_payload_signal(root)` using `GatePayload::in_dir(root)` and
  `Body::from_json(&payload)`.
- Helper `cell_context()` using:
  ```rust
  let bus: Arc<dyn BusErased> = Arc::new(MemoryBus::new(16));
  let store: Arc<dyn Store> = Arc::new(roko_std::MemorySubstrate::new());
  CellContext::new(bus, store, CancellationToken::new())
  ```

Test cases:

- `compile_gate_execute_returns_verdict_signal`: valid Cargo fixture,
  `CompileGate::cargo().with_timeout_ms(60_000)`, one input signal, output
  length 1, output kind `Kind::GateVerdict`, JSON body decodes to `Verdict`
  with `gate == "compile:cargo"` and `passed == true`.
- `test_gate_execute_returns_verdict_signal`: valid Cargo fixture with at
  least one unit test, `TestGate::cargo().with_timeout_ms(60_000)`, same
  assertions with `gate == "test:cargo"`.
- `clippy_gate_execute_returns_verdict_signal`: valid Cargo fixture,
  `ClippyGate::cargo().with_timeout_ms(60_000)`, same assertions with
  `gate == "clippy:cargo"`.
- `diff_gate_execute_returns_verdict_signal`: no Cargo fixture needed; input
  signal body is `DiffPayload::new("+++ b/src/lib.rs\n+pub fn x() -> i32 { 1 }\n")`,
  same assertions with `gate == "diff"`.

Use `roko_core::Cell` in the test imports so `.execute(...)` is in scope.

### Verification details

Run targeted checks first:

```bash
cargo test -p roko-gate --test cell_execute
rg -n "async fn execute" crates/roko-gate/src/{compile.rs,test_gate.rs,clippy_gate.rs,diff_gate.rs}
rg -n "self\\.verify\\(" crates/roko-gate/src/{compile.rs,test_gate.rs,clippy_gate.rs,diff_gate.rs}
```

Then run the task's full verification list.

### Anti-patterns

- Do not reimplement compile/test/clippy/diff logic in `execute()`.
- Do not call shell commands directly from `execute()`; call `self.verify(...)`.
- Do not change runner gate dispatch. Runner v2 still uses `Verify` directly.
- Do not add `execute()` to any gate outside the four named files.
- Do not return raw text output signals; the output body must JSON-decode to a
  `Verdict` and the signal kind must be `Kind::GateVerdict`.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
