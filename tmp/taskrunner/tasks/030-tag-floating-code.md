# Task 030: Tag All Floating Code with STATUS Comments

```toml
id = 30
title = "Add STATUS: NOT WIRED comments to all floating modules identified in dead code audit"
track = "cleanup"
wave = "wave-1"
priority = "low"
blocked_by = []
touches = [
    "crates/roko-runtime/src/theta_consumer.rs",
    "crates/roko-runtime/src/delta_consumer.rs",
    "crates/roko-runtime/src/demurrage_consumer.rs",
    "crates/roko-runtime/src/energy.rs",
    "crates/roko-runtime/src/heartbeat_attention.rs",
    "crates/roko-runtime/src/heartbeat_probes.rs",
    "crates/roko-runtime/src/task_scheduler.rs",
    "crates/roko-learn/src/active_inference.rs",
    "crates/roko-learn/src/bayesian_confidence.rs",
    "crates/roko-learn/src/calibration_policy.rs",
    "crates/roko-learn/src/error_enrichment.rs",
    "crates/roko-learn/src/event_subscriber.rs",
    "crates/roko-learn/src/oracles/mod.rs",
    "crates/roko-learn/src/quality_judge.rs",
    "crates/roko-learn/src/verdict_scorer.rs",
]
exclusive_files = []
estimated_minutes = 90
```

## Context

The dead code audit (`01-CURRENT-STATE.md`) identified floating modules in roko-runtime,
roko-learn, and the language parser crates. That inventory is stale in this checkout, so
this task must start with a fresh callsite audit and only tag modules that are still not
reachable from a non-test runtime/CLI path. Each tagged module needs a visible status
header so developers immediately see the module's wiring state.

Note: `roko-runtime/src/run_ledger.rs` is excluded because it has its own wiring task (015).

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- QW-7
- `tmp/v2-refactoring/03-QUICK-WINS.md` -- QW-7
- `tmp/v2-refactoring/01-CURRENT-STATE.md` -- floating code inventory

## Background

Read these files first:
1. `tmp/v2-refactoring/01-CURRENT-STATE.md` -- the floating code inventory (lines 59-103)
2. Any one of the listed modules to see their current doc comment style

Current audit notes to preserve:
- `crates/roko-learn/src/oracles.rs` does not exist; the module root is
  `crates/roko-learn/src/oracles/mod.rs`.
- `crates/roko-lang-rust`, `crates/roko-lang-typescript`, and `crates/roko-lang-go` are
  wired through `roko-index/src/workspace.rs` (`static_providers`, `provider_for_path`) and
  the `roko index` CLI path in `crates/roko-cli/src/commands/util.rs`. Do not tag them as
  `NOT WIRED` unless a new audit proves that index path has been removed.
- These roko-learn modules have observed non-test call paths and should not be tagged from
  the stale inventory: `baseline`, `jsonl_rotation`, `local_reward`, `pareto`,
  `post_gate_reflection`, and `section_outcome`.
- `event_subscriber::run_learning_subscriber` currently has no non-test caller. Its
  internal callees, including `calibration_policy` and `verdict_scorer`, should use the
  "called internally by floating code" wording if they remain unwired.
- A public `pub mod` export or `pub use` is not a runtime caller. Treat it as visibility
  only; find an actual CLI/runtime path before deciding a module is wired.

## What to Change

1. **Re-audit each candidate immediately before editing**. Use `rg` to find non-test
   callers, excluding the module's own file, tests, docs, and generated output. Examples:
   ```bash
   rg -n 'DemurrageConsumer|run_learning_subscriber|CalibrationPolicy|VerdictScorer' \
     crates --glob '*.rs' --glob '!target/**'
   rg -n 'select_tier_with_active_inference|ActiveInference' crates --glob '*.rs'
   ```
   If task 031, 032, or 033 has already wired one of these modules, skip that module.

2. **For each module that remains floating**, add a status line as the FIRST line of the
   module-level doc comment (`//!`). If the module already has a `//!` doc comment, prepend
   the status line. If it has none, add one.

   Format:
   ```rust
   //! STATUS: NOT WIRED -- built but no non-test runtime caller.
   //!
   //! [existing doc comment continues...]
   ```

3. **Some modules have internal callers** (e.g., `calibration_policy` is called from
   `event_subscriber`, which is itself floating). The status should read:
   ```rust
   //! STATUS: NOT WIRED -- called internally by floating code but no runtime entrypoint.
   ```

4. **Verify the exact target set**, not the stale `>= 22` count. In this checkout the
   expected count is 15 if no related wiring tasks have landed first: 7 runtime modules and
   8 roko-learn modules. Recalculate if your pre-audit skips any newly wired module.
   ```bash
   rg -n '^//! STATUS: NOT WIRED' crates/roko-runtime/src crates/roko-learn/src \
     --glob '*.rs' --glob '!target/**'
   ```

## What NOT to Do

- Don't modify any code logic -- only add doc comments.
- Don't mark modules as "NOT WIRED" if they actually have runtime callers (check with
  grep first).
- Don't change the `run_ledger.rs` module -- it has its own task (015).
- Don't tag the language parser crates from the stale audit unless `roko-index` no longer
  uses them.
- Don't tag wired roko-learn modules listed in the Background notes above.

## Wire Target

```bash
# Documentation only -- verify by grep:
rg -n '^//! STATUS: NOT WIRED' crates/roko-runtime/src crates/roko-learn/src \
  --glob '*.rs' --glob '!target/**'
# Current expected count: 15, minus any modules wired by completed prerequisite work.
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `rg -n '^//! STATUS: NOT WIRED' crates/roko-runtime/src crates/roko-learn/src --glob '*.rs' --glob '!target/**'` -- returns only modules still proven floating
- [ ] `rg -n '^//! STATUS: NOT WIRED' crates/roko-lang-* crates/roko-learn/src/{baseline,jsonl_rotation,local_reward,pareto,post_gate_reflection,section_outcome}.rs` -- returns no matches
- [ ] No code logic was changed (only `//!` comments added)
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
