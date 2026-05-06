# Task 037: Rename Engram to Signal in roko-core — Flip the Canonical Direction

```toml
id = 37
title = "Rename Engram -> Signal in roko-core: struct rename + deprecated alias + internal migration"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-core/src/engram.rs",
    "crates/roko-core/src/signal.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-core/src/traits.rs",
    "crates/roko-core/src/datum.rs",
    "crates/roko-core/src/pulse.rs",
    "crates/roko-core/src/kind.rs",
    "crates/roko-core/src/body.rs",
    "crates/roko-core/src/loop_tick.rs",
    "crates/roko-core/src/attestation.rs",
    "crates/roko-core/src/catalyst.rs",
    "crates/roko-core/src/cfactor.rs",
    "crates/roko-core/src/forensic.rs",
    "crates/roko-core/src/immune.rs",
    "crates/roko-core/src/prediction.rs",
    "crates/roko-core/src/affect.rs",
    "crates/roko-core/src/chat_types.rs",
    "crates/roko-core/src/error/mod.rs",
    "crates/roko-core/src/tool/handler.rs",
    "crates/roko-core/src/config/subscriptions.rs",
    "crates/roko-core/src/config/serve.rs",
]
exclusive_files = [
    "crates/roko-core/src/engram.rs",
    "crates/roko-core/src/signal.rs",
]
estimated_minutes = 180
```

## Context

The v2 spec establishes `Signal` as the canonical name for the universal datum. Currently
`Engram` is the struct name and `Signal` is a type alias (`pub type Signal = Engram`). This
task flips that: `Signal` becomes the real struct, `Engram` becomes a deprecated alias.

The rename is scoped to roko-core only. External crates continue to compile via the deprecated
`Engram` alias. Task 038 handles propagating the rename to downstream crates.

Current state:
- `engram.rs`: defines `pub struct Engram` (~60 references within roko-core)
- `signal.rs`: has `pub use crate::engram::{Engram as Signal, EngramBuilder as SignalBuilder}`
- ~222 uses of `Engram` within roko-core/src
- ~50 uses of `Signal` already (via the alias)

Checklist items: P1-6, P1-7.

## Background

Read these files before starting:

1. `crates/roko-core/src/engram.rs` — the struct definition, all methods, EngramBuilder
2. `crates/roko-core/src/signal.rs` — current alias module
3. `crates/roko-core/src/lib.rs` — exports for both Engram and Signal
4. `crates/roko-core/src/traits.rs` — all 6 protocol traits reference Engram in signatures
5. `tmp/v2-refactoring/05-SIGNAL-RENAME.md` — the design spec

## What to Change

### Step 1: Rename the struct in engram.rs

In `crates/roko-core/src/engram.rs`:
- Rename `pub struct Engram` to `pub struct Signal`
- Rename `pub struct EngramBuilder` to `pub struct SignalBuilder`
- Rename `pub struct HdcFingerprint` stays the same (it's not being renamed)
- Update all references within the file: method names like `from_pulse_synthetic`, `content_hash`, etc. stay the same (they're methods on the struct, not named after it)
- Update doc comments that say "Engram" to say "Signal"
- Keep the file name as `engram.rs` for now (renaming the file would break module paths; can be done later)

### Step 2: Flip the alias direction in signal.rs

Replace the contents of `crates/roko-core/src/signal.rs` with:

```rust
//! Signal — the universal datum of the Roko system.
//!
//! Re-exports from engram.rs. The Engram -> Signal rename is complete within
//! roko-core; the deprecated Engram alias ensures downstream crates compile.

pub use crate::engram::{Signal, SignalBuilder, HdcFingerprint};
```

### Step 3: Add deprecated Engram alias

In `crates/roko-core/src/engram.rs` (or in `lib.rs`), add:

```rust
/// Deprecated alias for [`Signal`]. Use `Signal` in new code.
#[deprecated(since = "0.2.0", note = "Use Signal instead of Engram")]
pub type Engram = Signal;

/// Deprecated alias for [`SignalBuilder`]. Use `SignalBuilder` in new code.
#[deprecated(since = "0.2.0", note = "Use SignalBuilder instead of EngramBuilder")]
pub type EngramBuilder = SignalBuilder;
```

### Step 4: Update all roko-core internal references

Replace `Engram` with `Signal` in every file within `crates/roko-core/src/`. Run:
```bash
grep -rn 'Engram' crates/roko-core/src/ --include='*.rs' | grep -v target/ | grep -v 'type Engram' | grep -v deprecated
```
and update each reference. There are ~222 occurrences across ~21 files within roko-core.

Key files with many references:
- `traits.rs` (~28 occurrences) — Store::put takes Engram, Score::score takes &Engram, etc.
- `datum.rs` (~21 occurrences) — Datum::Engram variant
- `loop_tick.rs` (~18 occurrences)
- `prediction.rs` (~17 occurrences)
- `attestation.rs` (~16 occurrences)
- `pulse.rs` (~14 occurrences)

**Important for traits.rs**: The `Datum` enum has a variant `Datum::Engram(...)`. Rename it
to `Datum::Signal(...)` but add a deprecated alias pattern or update all match sites. Since
this is roko-core-internal, update all match sites.

**Important for lib.rs exports**: The lib.rs currently has:
```rust
pub use engram::{Engram, EngramBuilder, HdcFingerprint};
```
Change to:
```rust
pub use engram::{Signal, SignalBuilder, HdcFingerprint};
// Deprecated re-exports
#[allow(deprecated)]
pub use engram::{Engram, EngramBuilder};
```

### Step 5: Suppress deprecation warnings within roko-core

Since roko-core itself defines the deprecated alias, add `#[allow(deprecated)]` on the
alias definitions only. Do NOT suppress deprecation warnings elsewhere — they should fire
in downstream crates to signal the migration.

### Step 6: Update roko-core tests and benches

```bash
grep -rn 'Engram' crates/roko-core/tests/ --include='*.rs' | grep -v target/
grep -rn 'Engram' crates/roko-core/benches/ --include='*.rs' | grep -v target/
```
Update these to use `Signal` and `SignalBuilder`.

## What NOT to Do

- Do NOT rename files (engram.rs stays as engram.rs). Module renames are separate cleanup.
- Do NOT update crates outside roko-core. That's task 038.
- Do NOT remove the Engram alias. It must exist as deprecated for backwards compat.
- Do NOT rename `HdcFingerprint` — it's not part of this rename.
- Do NOT rename the `Datum::Engram` variant to something other than `Datum::Signal` (keep
  the naming consistent).
- Do NOT change serialization format. The struct's serde field names stay the same so
  existing JSONL files remain compatible.

## Wire Target

This is a rename — no new functionality. The wire target is that the entire workspace still
compiles and tests pass:

```bash
cargo build --workspace
cargo test -p roko-core
# Downstream crates compile via deprecated alias
cargo build -p roko-gate
cargo build -p roko-agent
```

## Verification

- [ ] `cargo build --workspace` — compiles (deprecated warnings expected in non-core crates)
- [ ] `cargo test --workspace` — all tests pass
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean (deprecated warnings are
  allowed in downstream crates that haven't migrated yet)
- [ ] `grep -rn 'pub struct Signal' crates/roko-core/src/engram.rs` — Signal is the real struct
- [ ] `grep -rn 'type Engram = Signal' crates/roko-core/src/` — deprecated alias exists
- [ ] `grep -rn 'Engram' crates/roko-core/src/ --include='*.rs' | grep -v 'type Engram' | grep -v deprecated | grep -v target/ | grep -v '#\[allow'` — returns 0 lines (all internal uses migrated)
- [ ] Existing `.roko/signals.jsonl` files still parse correctly (serde compat)
- [ ] `cargo test -p roko-core` — core tests pass with Signal, not Engram

## Implementation Detail

### Source doc clarification

`tmp/v2-refactoring/05-SIGNAL-RENAME.md` mentions adding a `balance` field as
part of QW-3. That is not part of this task. Do not add `balance` here unless
the separate QW-3 task has already landed and the field is already present.

### Current reference set

Current roko-core files containing `Engram`, `EngramBuilder`, or
`Datum::Engram`:

```text
crates/roko-core/src/engram.rs
crates/roko-core/src/signal.rs
crates/roko-core/src/lib.rs
crates/roko-core/src/traits.rs
crates/roko-core/src/datum.rs
crates/roko-core/src/pulse.rs
crates/roko-core/src/body.rs
crates/roko-core/src/kind.rs
crates/roko-core/src/loop_tick.rs
crates/roko-core/src/attestation.rs
crates/roko-core/src/catalyst.rs
crates/roko-core/src/cfactor.rs
crates/roko-core/src/forensic.rs
crates/roko-core/src/immune.rs
crates/roko-core/src/prediction.rs
crates/roko-core/src/affect.rs
crates/roko-core/src/chat_types.rs
crates/roko-core/src/error/mod.rs
crates/roko-core/src/tool/handler.rs
crates/roko-core/src/config/subscriptions.rs
crates/roko-core/src/config/serve.rs
crates/roko-core/tests/property_tests.rs
crates/roko-core/benches/engram_bench.rs
```

The TOML `touches` list covers the source files but not the test/bench files.
If touch enforcement is strict, expand task metadata before implementation or
leave a status-log blocker before editing tests/benches.

### Mechanical steps

1. In `crates/roko-core/src/engram.rs`, rename the concrete types:
   - `pub struct Engram` -> `pub struct Signal`
   - `impl Engram` -> `impl Signal`
   - `pub struct EngramBuilder` -> `pub struct SignalBuilder`
   - `impl EngramBuilder` -> `impl SignalBuilder`
   - `EngramBuilder::new(...)` -> `SignalBuilder::new(...)`
   - `build(self) -> Engram` -> `build(self) -> Signal`

   Keep method names such as `builder`, `derive`, `derive_verdict`,
   `from_pulse_synthetic`, `from_pulses`, `bind`, and `bundle`.

2. Add deprecated aliases in `engram.rs` after the concrete type definitions:
   ```rust
   /// Deprecated alias for [`Signal`]. Use `Signal` in new code.
   #[deprecated(since = "0.2.0", note = "Use Signal instead of Engram")]
   pub type Engram = Signal;

   /// Deprecated alias for [`SignalBuilder`]. Use `SignalBuilder` in new code.
   #[deprecated(since = "0.2.0", note = "Use SignalBuilder instead of EngramBuilder")]
   pub type EngramBuilder = SignalBuilder;
   ```
   Do not put `#[allow(deprecated)]` around normal roko-core uses; roko-core
   internals should use `Signal`.

3. Replace `crates/roko-core/src/signal.rs` with a canonical re-export:
   ```rust
   //! Signal — the universal datum of the Roko system.
   //!
   //! Re-exports the canonical type from `engram.rs`. The old `Engram`
   //! spelling remains available through deprecated aliases for downstream
   //! compatibility.

   pub use crate::engram::{HdcFingerprint, Signal, SignalBuilder};
   ```

4. In `crates/roko-core/src/lib.rs`:
   - Update crate docs from "Engram" to "Signal" where they describe the
     universal datum.
   - Change the primary export to:
     ```rust
     pub use engram::{HdcFingerprint, Signal, SignalBuilder};
     #[allow(deprecated)]
     pub use engram::{Engram, EngramBuilder};
     ```
   - Keep `pub use signal::{Signal, SignalBuilder};` only if it does not create
     duplicate-name export errors. Prefer one primary export path if the
     compiler complains.

5. In `crates/roko-core/src/datum.rs`, rename the enum variant:
   - `Datum::Engram(&'a Engram)` -> `Datum::Signal(&'a Signal)`
   - `is_engram()` -> `is_signal()` if all callsites are updated in this task.
     If preserving method compatibility is desired, keep `is_engram()` as a
     deprecated forwarding method and add `is_signal()`.
   - `impl From<&Engram> for Datum` becomes `impl From<&Signal> for Datum`.

   There is no Rust enum-variant alias for `Datum::Engram`. Before removing
   it, confirm `rg -n "Datum::Engram" crates/ --glob '*.rs'` only returns
   roko-core files that this task is changing.

6. In `crates/roko-core/src/traits.rs`, update public trait signatures to
   `Signal`:
   - `Store::put`, `Store::get`, `Store::query`
   - `ColdStore::archive`, `archive_batch`, `thaw`, `contains`
   - `Score::score` and the parameter/return types inside `score_engram`
   - `Verify::verify`
   - `Route::select` and the parameter/return types inside `select_engram`
   - `Compose::compose`, `compose_datums`
   - `React::decide`
   - `Observe::observe`

   Keep public helper method names like `score_engram` and `select_engram` in
   this task unless the compiler forces a rename; changing method names would
   expand the downstream breakage beyond the type rename.

7. Mechanically update the remaining roko-core source files to import/use
   `Signal` and `SignalBuilder`. Keep variable names like `signal` where they
   already exist. Do not rename filenames or module names.

8. Update `crates/roko-core/tests/property_tests.rs` and
   `crates/roko-core/benches/engram_bench.rs` to use `Signal`/`SignalBuilder`
   and `Datum::Signal`.

9. After the mechanical rename, run:
   ```bash
   rg -n "\bEngram\b|\bEngramBuilder\b|Datum::Engram" \
     crates/roko-core/src crates/roko-core/tests crates/roko-core/benches \
     --glob '*.rs'
   ```
   The only expected source hits are the deprecated alias definitions,
   deprecated compatibility wrappers if intentionally kept, and comments
   explaining backward compatibility.

### Serialization compatibility check

Renaming the Rust type does not change the serialized field names because
`Engram`/`Signal` is a struct name, not a tagged enum variant. Keep all fields
and serde attributes in `engram.rs` unchanged. Add or preserve a round-trip test
that deserializes legacy JSON into `Signal`:

```rust
let json = serde_json::to_string(&Signal::builder(Kind::Task).build()).unwrap();
let parsed: Signal = serde_json::from_str(&json).unwrap();
```

### Verification details

Use these targeted commands before the full workspace gate:

```bash
cargo test -p roko-core
cargo bench -p roko-core --bench engram_bench --no-run
rg -n "pub struct Signal|pub struct SignalBuilder" crates/roko-core/src/engram.rs
rg -n "type Engram = Signal|type EngramBuilder = SignalBuilder" crates/roko-core/src/engram.rs
rg -n "Datum::Engram|\bEngram\b|\bEngramBuilder\b" crates/roko-core/src crates/roko-core/tests crates/roko-core/benches --glob '*.rs'
```

The final `rg` should only show intentional compatibility references.

### Anti-patterns

- Do not rename `engram.rs`, `engram_bench.rs`, or the `engram` module in this
  task.
- Do not add the QW-3 `balance` field.
- Do not update downstream crates here; task 038 owns that propagation.
- Do not remove deprecated `Engram`/`EngramBuilder` aliases.
- Do not silence deprecation warnings broadly with crate-level `allow`
  attributes.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
