# Task 031: Wire CalibrationPolicy to CascadeRouter for Model Routing Feedback

```toml
id = 31
title = "Wire CalibrationPolicy corrections into CascadeRouter confidence updates"
track = "wiring"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-learn/src/event_subscriber.rs",
    "crates/roko-learn/src/calibration_policy.rs",
    "crates/roko-learn/src/cascade_router.rs",
]
exclusive_files = ["crates/roko-learn/src/event_subscriber.rs"]
estimated_minutes = 120
```

## Context

`CalibrationPolicy` in roko-learn processes agent turn events to detect systematic bias
in model routing predictions. When it finds a model is consistently over/under-confident,
it emits a `CalibrationCorrection`. But that correction is currently logged and discarded --
it never feeds back into the `CascadeRouter`'s confidence estimates.

The event_subscriber already instantiates a `CalibrationPolicy` and processes events through
it. When a correction is triggered, it logs an info message but does not update the router.
The missing link is: correction.model + correction.correction -> router confidence adjustment.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- QW-8
- `tmp/v2-refactoring/03-QUICK-WINS.md` -- QW-8

## Background

Read these files first:
1. `crates/roko-learn/src/calibration_policy.rs` -- CalibrationPolicy, CalibrationCorrection, process_event()
2. `crates/roko-learn/src/event_subscriber.rs` -- run_learning_subscriber(), lines 64-103 where CalibrationPolicy is used
3. `crates/roko-learn/src/cascade_router.rs` -- CascadeRouter, especially record_confidence_outcome()

Current wiring reality check:
- `run_learning_subscriber(...)` currently appears only in its definition and tests. Before
  changing code, run:
  ```bash
  rg -n 'run_learning_subscriber\(' crates --glob '*.rs'
  ```
  If there is still no non-test caller, wiring only `event_subscriber.rs` and
  `cascade_router.rs` will not affect runtime behavior. Either expand the implementation
  scope to start the subscriber and feed it turn events from the runner, or use an existing
  live feedback path. Do not mark this task complete with a dead subscriber-only call.
- The CLI/runtime path for plan execution is
  `crates/roko-cli/src/main.rs` -> `commands/plan.rs` -> `runner/event_loop.rs`, where
  `RunConfig.cascade_router` is handed to `SharedAgentFactory` and persisted during
  shutdown to `.roko/learn/cascade-router.json`.
- `CascadeRouter` currently stores confidence as `ModelStats { trials, successes }` in
  `confidence_stats`; there is no standalone numeric confidence/bias field to clamp.

## What to Change

1. **First make or verify a live event path**:
   - If `run_learning_subscriber` has a non-test caller, continue with the direct subscriber
     wiring below.
   - If it does not, stop and expand the task scope before implementation. Acceptable
     approaches are to start the subscriber from the runner and send `TurnStarted`,
     `ModelSelected`, and `TurnCompleted` events into its channel, or to route the
     correction through an already live learning feedback component. The finished task must
     have a non-test call chain from `roko plan run` to the calibration correction.

2. **In `event_subscriber.rs`**, when `calibration_policy.process_event()` returns a
   `CalibrationCorrection`, apply it to the CascadeRouter:

   The `run_learning_subscriber` function already has access to `router: Arc<CascadeRouter>`.
   After the existing `tracing::info!` log (around line 97-103), add a call that adjusts the
   router's confidence for the corrected model.

   Look at `CascadeRouter`'s public API for a method to adjust model confidence or bias.
   If `record_confidence_outcome` is too coarse (it takes a boolean), check if there's a
   method that accepts a numeric adjustment, or add a minimal one:
   ```rust
   pub fn apply_calibration_correction(&self, model: &str, correction: f64)
   ```

3. **If CascadeRouter needs a new method**, add `apply_calibration_correction()` against
   the current `ModelStats` representation. Because there is no scalar confidence field,
   implement the correction as bounded synthetic outcomes or add an explicit bias field and
   update router save/load tests. The lower-risk mechanical option is:
   - positive `correction` means the model was under-confident; add one or more synthetic
     successes.
   - negative `correction` means the model was over-confident; add one or more synthetic
     failures.
   - bound the weight, for example `1..=10` synthetic observations from
     `correction.abs() * 10.0`, so a single correction cannot dominate all history.
   - return `bool` to report whether the model entry was found/updated, matching
     `record_confidence_outcome` style.

4. **Add focused tests** in `event_subscriber.rs` and/or `cascade_router.rs` that verify:
   - Multiple failing turns for one model -> correction triggered
   - Correction is applied to router
   - A negative correction lowers that model's `confidence_snapshot()` pass rate or upper
     bound, and a positive correction raises it
   - The runtime call chain has a non-test caller if the subscriber is the chosen path

## What NOT to Do

- Don't restructure the CalibrationPolicy -- it works correctly.
- Don't change how events are dispatched in the event subscriber.
- Don't add Bus/Pulse integration -- this is direct function calls, not event-driven wiring.
  The Bus-backed version is a Phase 3 task.
- Don't persist calibration state to disk yet -- the CascadeRouter already persists its own state.
- Don't implement a clamp against a nonexistent router confidence scalar. Use the existing
  `ModelStats` data model or update persistence deliberately.
- Don't count a unit test or a public `pub mod` export as wiring. There must be a non-test
  runtime/CLI call path.

## Wire Target

```bash
# First prove the subscriber or chosen feedback path is live:
rg -n 'run_learning_subscriber\(|apply_calibration_correction' crates --glob '*.rs'

# After a plan run with enough completed turns, check calibration logs and router state:
cargo run -p roko-cli -- plan run plans/ 2>&1 | grep -i calibration
cat .roko/learn/cascade-router.json | python3 -m json.tool
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `rg -n 'run_learning_subscriber\(' crates --glob '*.rs'` -- shows a non-test caller, or the chosen alternative live feedback path is documented in the diff
- [ ] `rg -n 'apply_calibration_correction|CalibrationCorrection' crates/roko-learn crates/roko-cli --glob '*.rs' --glob '!target/**'` -- shows at least one non-test runtime callsite
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
