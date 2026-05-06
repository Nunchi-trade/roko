# Task 027: Add `balance` Field to Engram for Demurrage Support

```toml
id = 27
title = "Add balance: f64 field to Engram struct for v2 demurrage tracking"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-core/src/engram.rs",
]
exclusive_files = ["crates/roko-core/src/engram.rs"]
estimated_minutes = 45
```

## Context

The v2 architecture requires every Signal (Engram) to carry a `balance: f64` field for
Gesellian demurrage -- value that decays over time unless actively refreshed. The
`Demurrage` trait already exists in `roko-core/src/demurrage.rs`, and `KnowledgeEntry`
in roko-neuro already has `balance`. But the core `Engram` struct is missing it.

Without a balance field on Engram, the DemurrageConsumer (roko-runtime) and Store::prune()
cannot enforce decay at the signal level.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- QW-3
- `tmp/v2-refactoring/03-QUICK-WINS.md` -- QW-3: Add balance field
- `crates/roko-core/src/demurrage.rs` -- the Demurrage trait

## Background

Read these files first:
1. `crates/roko-core/src/engram.rs` -- the Engram struct (lines 62-95), EngramBuilder, content_hash()
2. `crates/roko-core/src/demurrage.rs` -- the Demurrage trait with balance/tick/replenish
3. `crates/roko-core/src/lib.rs` -- Engram re-exports
4. `crates/roko-core/src/signal.rs` -- `Signal` is a type alias to `Engram`, so it inherits fields
5. `crates/roko-core/src/traits.rs` -- `Store::put/get/query/prune` traffic carries `Engram`
6. `crates/roko-neuro/src/lib.rs` -- `KnowledgeEntry` already has `#[serde(default = "default_balance")] pub balance: f64`

Current-state details:

- `Engram` has `decay: Decay` and `score: Score`, but no `balance`.
- `content_hash()` currently hashes `kind`, `body`, provenance author/taint, lineage, and tags.
  It intentionally excludes mutable metadata: score, decay, timestamp, fingerprint, attestation,
  and emotional tag. `balance` must join that excluded set.
- Most callsites construct engrams via `Engram::builder(...)`, so adding a builder default is the
  compatibility path. The only direct struct literal in core is `EngramBuilder::build`.

## What to Change

1. **Add `balance: f64` field to `Engram`** in `crates/roko-core/src/engram.rs`:
   ```rust
   /// Demurrage balance in [0.0, 1.0]. Decays over time; refreshed on access.
   #[serde(default = "default_balance")]
   pub balance: f64,
   ```
   Add the default function:
   ```rust
   fn default_balance() -> f64 { 1.0 }
   ```
   Place the field near `decay`/`score`; update the struct docs to mention that identity excludes
   balance. Keep the field public.

2. **Exclude `balance` from `content_hash()`** -- balance is mutable metadata, like `score`.
   Find where `ContentHash` is computed and confirm `balance` is NOT included. (Score is
   already excluded, so follow the same pattern.)

3. **Add `touch()` method** to Engram:
   ```rust
   /// Reset balance to 1.0 (demurrage refresh on access).
   pub fn touch(&mut self) {
       self.balance = 1.0;
   }
   ```
   Put it near `weight_at`/`age_ms` or other metadata helpers.

4. **Update `EngramBuilder`** to set `balance: 1.0` by default and optionally accept a
   custom value via `.balance(val)`.
   Mechanical changes:
   - add `balance: f64` to `EngramBuilder`
   - set `balance: default_balance()` in `EngramBuilder::new`
   - add a public builder method:
     ```rust
     #[must_use]
     pub fn balance(mut self, balance: f64) -> Self {
         self.balance = balance;
         self
     }
     ```
   - set `balance: self.balance` in the `Engram { ... }` literal in `build`

5. **Add tests**:
   - Serde backwards compatibility: deserialize an Engram JSON without `balance` field,
     confirm it defaults to 1.0
   - `touch()` resets balance
   - `balance` is excluded from content hash
   Add these in `crates/roko-core/src/engram.rs` next to existing content-hash and serde tests:
   - `builder_defaults` asserts `s.balance == 1.0`
   - `builder_balance_sets_custom_value`
   - `content_hash_ignores_balance` creates same identity fields with `.balance(1.0)` and `.balance(0.25)`
   - `serde_defaults_missing_balance_to_one` serializes an engram to `serde_json::Value`, removes
     `"balance"`, deserializes, and asserts `1.0`
   - `touch_resets_balance_to_one`

## What NOT to Do

- Don't implement `Demurrage` trait on Engram yet -- that's a separate task (DCA-1 wiring).
- Don't change any Store implementations -- they'll pick up the new field via serde automatically.
- Don't modify the `Signal` type alias -- it points to Engram and inherits the field.
- Don't include `balance` in `content_hash()` or `id` computation.
- Don't change `weight_at()` to multiply by balance in this task. That behavior belongs with the
  future demurrage consumer/prune wiring.
- Don't update `roko-neuro::KnowledgeEntry`; it already has a balance model.
- Don't hand-edit generated scaffold strings in `crates/roko-cli/src/scaffold.rs` unless the
  compiler proves they are compiled Rust. They are template text, not active `Engram` literals.

## Wire Target

```bash
# balance is a serde field -- it's automatically wired into Store::put/get.
# Verify with:
cargo test -p roko-core -- engram
cargo test -p roko-core -- property_tests
```

Expected observable behavior:

- New `Engram::builder(...).build()` values have `balance == 1.0`.
- Existing JSON/JSONL records without a `balance` field still deserialize successfully with
  `balance == 1.0`.
- Changing only `balance` and calling `touch()` does not change `Engram::id` or `content_hash()`.
- `roko_core::Signal` users can access the field because `Signal` aliases `Engram`.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `grep -rn 'pub balance' crates/roko-core/src/engram.rs --include='*.rs'` -- shows the field
- [ ] `grep -rn 'fn touch' crates/roko-core/src/engram.rs --include='*.rs'` -- shows the method
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
