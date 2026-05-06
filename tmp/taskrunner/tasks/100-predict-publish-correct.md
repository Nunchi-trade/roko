# Task 100: Predict-Publish-Correct for CascadeRouter

```toml
id = 100
title = "Wire predict-publish-correct calibration loop into CascadeRouter"
track = "wiring"
wave = "wave-4"
priority = "high"
blocked_by = [31, 99]
touches = [
    "crates/roko-learn/src/cascade_router.rs",
    "crates/roko-learn/src/calibration_policy.rs",
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/types.rs",
    "crates/roko-cli/src/runtime_feedback/routing.rs",
]
exclusive_files = []
estimated_minutes = 240
```

## Context

This is the P3-8 wiring task: close the predict-publish-correct loop that
turns the CascadeRouter from a static heuristic into a self-calibrating
system.

The subsystem exists but is disconnected:

- `CascadeRouter::route()` already selects a model with a confidence score
- `CalibrationPolicy` already accumulates prediction residuals and triggers
  corrections when systematic bias is detected
- `AgentEvent::ModelSelected` exists in `roko-learn`, but it is taskless in
  this checkout and needs task-scoped `CalibrationPolicy` helper methods for
  runner v2

The gap: nothing calls `CalibrationPolicy::process_event()` from the runner,
and `CalibrationPolicy` corrections are never fed back into `CascadeRouter`.
This task connects those two ends.

After this task, running 3+ agent tasks will show evolving router confidence
values in `.roko/learn/cascade-router.json`.

Checklist item: P3-8.

## Background

Read these files before starting:

1. `crates/roko-learn/src/calibration_policy.rs` — the full
   `CalibrationPolicy` implementation. The public API is:
   - `CalibrationPolicy::new()` — create policy
   - `policy.process_event(&AgentEvent)` → `Option<CalibrationCorrection>`
   - `policy.drain_corrections()` → `Vec<CalibrationCorrection>`
   - `CalibrationCorrection.model`, `.mean_bias`, `.correction`
2. `crates/roko-learn/src/events.rs` — `AgentEvent` enum. The relevant
   variants are `TurnStarted`, `ModelSelected`, and `TurnCompleted`. The
   `AgentEvent` in runner types (`crates/roko-cli/src/runner/types.rs`) is
   a different type from the learning `AgentEvent` — check both.
3. `crates/roko-learn/src/cascade_router.rs` — the `CascadeRouter` struct.
   Key methods:
   - `route()` / `route_logged()` — returns `CascadeModel` + logs the
     selected slug. The selection confidence lives in `CascadeModel` fields.
   - `record_confidence_outcome(model_slug, success)` — updates the
     confidence-stage statistics per model.
   - `record_confidence_outcome(model_slug, success)` — updates confidence-stage stats.
   - `feedback_from_prediction(model_slug, predicted_success, actual_success)` — computes
     a residual and feeds the contextual bandit path.
   - `snapshot_json()` / `save(path)` — persist router state.
4. `crates/roko-cli/src/runner/event_loop.rs` — the core event loop.
   `agent_rx` (line ~353) receives `AgentEvent` variants. The cascade
   router is stored in `config.cascade_router: Option<Arc<CascadeRouter>>`.
5. `crates/roko-cli/src/runner/types.rs` — `RunConfig` and the runner's
   `AgentEvent` alias. This is `roko_agent::AgentRuntimeEvent`; gate results
   arrive separately through `GateCompletion`.
6. `tmp/v2-refactoring/09-GRADUATION.md` — the predict-publish-correct
   section for context.

## Current Checkout Corrections

These notes are authoritative for this checkout and override stale examples below:

- The runner's `AgentEvent` is `roko_agent::AgentRuntimeEvent` re-exported from
  `runner/types.rs`; it is not `roko_learn::events::AgentEvent` and it has no
  `gate_passed` field. Do not infer gate outcome from runner agent events.
- `roko_learn::events::AgentEvent::ModelSelected` has no `task_id`, and
  `TurnCompleted` has no `task_id`. `CalibrationPolicy::process_event()` currently drains
  all pending predictions on completion, which is unsafe for runner v2 parallel tasks.
  Add task-scoped methods to `CalibrationPolicy` and use those from the runner.
- `CascadeRouter::current_stage()`, `explain_route()`, `record_confidence_outcome()`,
  `feedback_from_prediction(model_slug, predicted_success, actual_success)`,
  `snapshot_json()`, and `save(path)` already exist. `record_outcome()` is deprecated.
- `CascadeModel` does not carry a confidence field. To get a prediction score, call
  `router.explain_route(&routing_context, None)` and find the selected candidate score,
  then clamp it to `0.0..=1.0`; fall back to `0.5`.
- Normal task outcome learning is already handled by `RoutingObservationSink` through the
  feedback facade. The calibration path must not blindly record the same outcome a second
  time. Only apply extra router updates when `CalibrationPolicy` emits a correction, and
  log that it is a synthetic calibration adjustment.
- `roko-learn/src/event_subscriber.rs` has an internal `CalibrationPolicy`, but that
  subscriber is not the runner v2 path. Do not count it as this task's wiring.
- `snapshot_json()` returns a `String`; there is no `snapshot_json_result()`.

## Recovery Worker 19 Checkout Notes

Use these details when wiring the loop:

- `RunnerDispatchPlan` in `crates/roko-cli/src/dispatch/mod.rs` currently carries only
  `model`, `forced`, and `prompt`; it does not preserve `ModelChoiceSource`, and this task's
  touch list does not include `dispatch/mod.rs`. In `event_loop.rs`, infer "came from
  cascade" using the existing predicate: cascade router present, routing context present,
  `!dispatch_plan.forced`, no `task_def.model_hint`, and no `cli_model_override`.
- `RunState.routing_context` is set immediately after `DispatchContext` is built in
  `event_loop.rs` before `dispatcher.plan(...)`. Use that context for
  `router.explain_route(routing_ctx, None)`. The score type is
  `CascadeCandidateScore { slug, score, selected, ... }`; find by `selected` first, or by
  `slug == requested_model`, and clamp to `0.0..=1.0`. If knowledge or Daimon later changes
  `requested_model` away from `explanation.selected_slug`, skip calibration for that attempt
  because the final choice was no longer a pure router prediction.
- Use one helper for the key, for example
  `fn calibration_attempt_key(plan_id: &str, task_id: &str, attempt: u32) -> String`.
  The attempt value used for registration is `attempt_num`; the gate branch already has
  `completion_attempt`. Keep these aligned in a unit test because an off-by-one silently
  leaves predictions pending.
- The terminal gate branch has two relevant `RunnerEvent::task_attempt_completed(...)` emit
  sites in `event_loop.rs`: the pass path near the final gate success branch and the exhausted
  or not-retryable failure path. Resolve calibration immediately after both emits. Do not
  resolve on every intermediate failed rung that will be retried.
- If `resolve_prediction()` pushes the returned correction into `self.corrections`, the
  immediate apply path must also remove/mark it applied or the run-end `drain_corrections()`
  fallback will double count it. Simpler acceptable implementation: task-scoped
  `resolve_prediction()` returns the correction but does not enqueue it; keep
  `process_event()` legacy behavior responsible for `drain_corrections()`.
- `save_snapshot()` near the `cascade_router_json` serialization does not have a `TuiBridge`
  argument. If adding a run-end flush, put it in a helper called from a scope that has `tui`,
  or persist the router without a TUI update there. Do not thread `TuiBridge` through snapshot
  code just for calibration.
- Normal learning already flows through `FeedbackEvent::TaskCompleted ->
  RoutingObservationSink`, which records success via `observe_multi_objective()` and failure
  via `record_confidence_outcome()`. Calibration should add only synthetic correction
  observations after threshold breach; do not call both `feedback_from_prediction()` and the
  existing feedback sink for the same raw outcome.
- Runtime defaults on `CalibrationPolicy` are `bias_threshold = 0.15` and `min_samples = 10`.
  Unit tests can lower `min_samples`; live runs may need more than three tasks before a
  correction log appears, even though normal `confidence_stats` should change after each
  task outcome.

## Mechanical Implementation Plan

1. In `CalibrationPolicy`, add task-scoped APIs:
   `register_prediction(task_id, model, category, predicted_success_prob)` and
   `resolve_prediction(task_id, actual_success) -> Option<CalibrationCorrection>`. These
   should update the same tracker/correction logic as `process_event()` but remove only the
   matching pending entry.
2. Keep `process_event()` for existing tests/subscribers, but implement its
   `TurnStarted`/`ModelSelected`/`TurnCompleted` behavior in terms of the new helpers where
   possible. The legacy `TurnCompleted` path may still drain all pending entries because the
   event type lacks task identity.
3. Add `calibration_policy: Option<Arc<parking_lot::Mutex<CalibrationPolicy>>>` to
   `RunConfig`. Initialize it in `RunConfig::from_roko_config()` and every direct
   `RunConfig { ... }` literal found by `rg -n "RunConfig \\{" crates/roko-cli/src`.
4. At dispatch time in `event_loop.rs`, after `dispatcher.plan(...)` and after
   `requested_model` is final, register a prediction only when the model came from the
   cascade path and was not later changed by knowledge or Daimon: cascade router exists,
   routing context exists, no `force_backend`, no task model hint, no CLI override, and
   `requested_model == explanation.selected_slug`. Use key `"{plan_id}:{task_id}:{attempt_num}"`, category from
   `routing_context.task_category.label()`, and score from `explain_route()`.
5. At terminal gate completion, immediately after emitting `RunnerEvent::task_attempt_completed`
   in both pass and exhausted/fail branches, resolve the same key with `actual_success =
   completion.passed`.
6. When `resolve_prediction()` returns a correction, use `record_confidence_outcome()` with
   synthetic success set to `correction.correction > 0.0` (underconfident) and synthetic
   failure otherwise (overconfident). Do not call `feedback_from_prediction()` if
   `RoutingObservationSink` is already enabled for the same task outcome.
7. Persist the cascade router after applying corrections by calling `router.save()` with
   `config.layout.cascade_router_path()`. Also update the TUI with
   `tui.cascade_router_updated(&router.snapshot_json())`.
8. Add startup logging to include `has_calibration_policy = config.calibration_policy.is_some()`.
9. Add unit tests in `calibration_policy.rs` for task-scoped prediction/outcome matching with
   two concurrent task ids. Add a runner helper unit test if a pure helper is introduced for
   attempt-key construction or score extraction.

### What "predict-publish-correct" means here

```
1. Router selects model → publishes prediction (model + confidence)
2. Agent completes + gate runs → outcome known (pass/fail)
3. CalibrationPolicy joins prediction + outcome → computes bias
4. If bias > threshold → CalibrationCorrection emitted
5. Router applies correction → confidence statistics updated
```

Steps 1 and 2 already happen. This task wires steps 3-5.

## What to Change

### 1. Add `calibration_policy` field to `RunConfig` in `crates/roko-cli/src/runner/types.rs`

Find the `RunConfig` struct. Add:

```rust
use roko_learn::calibration_policy::CalibrationPolicy;
use parking_lot::Mutex;
use std::sync::Arc;

// Inside RunConfig:
/// Calibration policy for closing the predict-publish-correct loop.
/// Shared between the event loop and the calibration flush task.
pub calibration_policy: Option<Arc<Mutex<CalibrationPolicy>>>,
```

In `RunConfig::default()` or wherever it is constructed, set:

```rust
calibration_policy: Some(Arc::new(Mutex::new(CalibrationPolicy::new()))),
```

**Check whether `RunConfig` is constructed in `plan.rs` or `event_loop.rs`
before editing.** Use `grep -n 'RunConfig {' crates/roko-cli/src/` to find
all construction sites.

### 2. Record predictions when the router selects a model

In `crates/roko-cli/src/runner/event_loop.rs`, use the dispatch block around
`dispatcher.plan(...)` and `requested_model` (currently near the model-selected
TUI/log call). Register directly with `CalibrationPolicy`; do not emit the
taskless learning `AgentEvent::ModelSelected` from the runner:

```rust
if let (Some(cal), Some(router), Some(routing_ctx)) = (
    &ctx.config.calibration_policy,
    &ctx.config.cascade_router,
    &ctx.state.routing_context,
) {
    let came_from_router = !dispatch_plan.forced
        && task_def.model_hint.is_none()
        && ctx.config.cli_model_override.is_none();
    if came_from_router {
        let explanation = router.explain_route(routing_ctx, None);
        if explanation.selected_slug == requested_model {
            let score = explanation
                .candidates
                .iter()
                .find(|candidate| candidate.selected || candidate.slug == requested_model)
                .map(|candidate| candidate.score.clamp(0.0, 1.0))
                .unwrap_or(0.5);
            cal.lock().register_prediction(
                calibration_attempt_key(plan_id, &task_id, attempt_num),
                requested_model.clone(),
                routing_ctx.task_category.label().to_string(),
                score,
            );
        }
    }
}
```

### 3. Record outcomes after gate completion

In `event_loop.rs`, use the `GateCompletion` branch. The pass path emits
`RunnerEvent::task_attempt_completed(... TaskAttemptOutcome::Passed ...)`
near the final gate pass branch; the fail/exhausted path emits the same
event with `Failed` or `Exhausted`. Resolve calibration immediately after
those emissions:

```rust
if let (Some(cal), Some(router)) = (&config.calibration_policy, &config.cascade_router) {
    let key = calibration_attempt_key(
        &completion.plan_id,
        &completion.task_id,
        completion_attempt.attempt,
    );
    if let Some(correction) = cal.lock().resolve_prediction(key, completion.passed) {
        let synthetic_success = correction.correction > 0.0;
        if router.record_confidence_outcome(&correction.model, synthetic_success) {
            tracing::info!(
                model = %correction.model,
                mean_bias = correction.mean_bias,
                correction = correction.correction,
                synthetic_success,
                "CalibrationPolicy correction applied to CascadeRouter"
            );
            if let Err(err) = router.save(&config.layout.cascade_router_path()) {
                tracing::warn!(error = %err, "failed to persist router after calibration");
            }
            tui.cascade_router_updated(&router.snapshot_json());
        }
    }
}
```

### 4. Flush calibration corrections at run end

In the snapshot-writing / run-end path (near line 2299 in `event_loop.rs`
where `cascade_router_json` is serialized), drain any corrections that were
emitted but not applied due to an early shutdown path. Use the existing
`save()` and `snapshot_json()` methods:

```rust
// After serializing cascade router snapshot:
if let (Some(cal), Some(router)) = (&config.calibration_policy, &config.cascade_router) {
    let corrections = cal.lock().drain_corrections();
    for correction in &corrections {
        let synthetic_success = correction.correction > 0.0;
        tracing::debug!(
            model = %correction.model,
            samples = correction.sample_count,
            bias = correction.mean_bias,
            synthetic_success,
            "Calibration correction at run end"
        );
        router.record_confidence_outcome(&correction.model, synthetic_success);
    }
    if !corrections.is_empty() {
        if let Err(e) = router.save(&config.layout.cascade_router_path()) {
            tracing::warn!(error = %e, "failed to persist router after calibration");
        } else {
            tui.cascade_router_updated(&router.snapshot_json());
        }
    }
}
```

### 5. Add a log line confirming the loop is active

In the run startup sequence (where `has_cascade_router` is logged around
line 474), also log calibration:

```rust
tracing::info!(
    has_cascade_router = config.cascade_router.is_some(),
    has_calibration_policy = config.calibration_policy.is_some(),
    "predict-publish-correct calibration loop active"
);
```

### 6. Add integration tests to `calibration_policy.rs`

The existing tests in `calibration_policy.rs` test the policy in isolation.
Add task-scoped tests first, then keep the existing event-based tests green:

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn overconfident_router_triggers_task_scoped_correction() {
        let mut policy = CalibrationPolicy::new()
            .with_min_samples(5)
            .with_bias_threshold(0.15);

        let mut saw_correction = false;

        for i in 0..10 {
            let task_id = format!("t-{i}");
            policy.register_prediction(
                task_id.clone(),
                "model-a",
                "implementation",
                0.9,
            );

            if let Some(c) = policy.resolve_prediction(task_id, false) {
                assert!(c.mean_bias > 0.1, "bias should be positive (overconfident)");
                assert_eq!(c.model, "model-a");
                saw_correction = true;
            }
        }

        assert!(
            saw_correction,
            "CalibrationPolicy should have emitted a correction after 10 overconfident failures"
        );
    }

    #[test]
    fn concurrent_predictions_resolve_by_task_id() {
        let mut policy = CalibrationPolicy::new()
            .with_min_samples(1)
            .with_bias_threshold(0.1);

        policy.register_prediction("task-a", "model-a", "implementation", 0.9);
        policy.register_prediction("task-b", "model-b", "implementation", 0.2);

        let correction_a = policy.resolve_prediction("task-a", false);
        assert!(correction_a.is_some());

        let correction_b = policy.resolve_prediction("task-b", true);
        assert!(correction_b.is_some());
        assert_ne!(correction_a.unwrap().model, correction_b.unwrap().model);
    }
}
```

## What NOT to Do

- Do NOT build a separate Pulse/Bus publish layer for prediction events in
  this task. The graduation + Bus infrastructure from tasks 097-099 is the
  right long-term path, but P3-8 explicitly targets the CalibrationPolicy
  → CascadeRouter loop using the existing `AgentEvent` channel. Document in
  the Status Log that the future path is Bus-backed.
- Do NOT add new fields to `CascadeRouter` for calibration state. The router
  already has `record_confidence_outcome()` and `feedback_from_prediction()`;
  use those or a small helper around them.
- Do NOT change `CalibrationPolicy` to be async. The existing sync API is
  correct — wrap it in `Mutex` for concurrent access.
- Do NOT emit taskless learning `AgentEvent::TurnCompleted` from the gate
  path with fabricated `Usage`. Use the task-scoped `CalibrationPolicy`
  methods added in this task.
- Do NOT double count normal task outcomes in the router. The existing
  `RoutingObservationSink` owns normal outcome observations; calibration
  applies only synthetic correction adjustments.
- Do NOT make calibration failures fatal. Gate failures are expected; a
  CalibrationPolicy error is a warning-level event, not a crash.

## Wire Target

```bash
# Run 3+ agent tasks with a cascade router configured:
cargo run -p roko-cli -- plan run plans/ --max-tasks 3

# After the run, check the cascade router state:
cat .roko/learn/cascade-router.json | python3 -m json.tool | grep -A5 '"confidence_stats"'

# The confidence stats should show observations for the models that were used.
# Run again and verify the numbers change (router is learning).
```

To test without running actual agents, use the integration test:

```bash
cargo test -p roko-learn -- integration_tests::overconfident_router_triggers_correction
```

## Verification

- [ ] `cargo build --workspace` — clean
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo test -p roko-learn -- integration_tests` — new integration test passes
- [ ] `grep -n 'calibration_policy' crates/roko-cli/src/runner/types.rs` — field exists on RunConfig
- [ ] `grep -n 'CalibrationPolicy\|calibration_policy' crates/roko-cli/src/runner/event_loop.rs` — wired into loop
- [ ] `grep -n 'predict-publish-correct' crates/roko-cli/src/runner/event_loop.rs` — log line exists
- [ ] After running 3 agent tasks: `cat .roko/learn/cascade-router.json` shows `confidence_stats` entries
- [ ] Run the same plan twice — the second run shows different confidence scores (learning is happening)
- [ ] `tracing` log output during a run includes at least one `"CalibrationPolicy correction applied"` line (after enough data)

## Status Log

| Time | Agent | Action |
|------|-------|--------|
