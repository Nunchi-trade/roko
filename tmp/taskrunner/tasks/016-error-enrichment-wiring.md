# Task 016: Wire Error Enrichment into Gate Failure Retry Path

```toml
id = 16
title = "Wire error enrichment into gate failure retry prompts in Runner v2"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/gate_dispatch.rs",
]
exclusive_files = []
estimated_minutes = 90
```

## Context

When a gate fails and the runner retries, the retry prompt should include analysis of WHY
the gate failed — not just "gate failed, try again." Error enrichment code exists somewhere
in the codebase but isn't wired into the v2 runner's gate failure path.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` — DCA-3: Wire error_enrichment

## Background

Read these files first:
1. `crates/roko-learn/src/error_enrichment.rs`
   - Contains async `enrich_error_digest(raw_error, agent: &dyn Agent, task_context) -> String`.
   - It requires an already-available cheap agent. Do not spawn a second recursive agent from the retry path just to call it.
2. `crates/roko-gate/src/compile_errors.rs`
   - `classify_gate_failure(gate, output) -> GateFailureClassification`
   - `structured_gate_failure(...)`
   - `render_failure_classification(...)`
3. `crates/roko-gate/src/lib.rs`
   - Re-exports `classify_gate_failure`, `structured_gate_failure`, and `render_failure_classification`.
4. `crates/roko-cli/src/runner/gate_dispatch.rs`
   - Already imports `roko_gate::classify_gate_failure`.
   - Private `classify_failure_kind(...)` maps gate output into `RunnerFailureKind`.
5. `crates/roko-cli/src/runner/event_loop.rs`
   - Gate failure retry branch currently builds `replan_context` containing "Gate error output" and "What you did last time".
   - `dispatch_action(...)` later appends `ctx.state.take_replan_context(plan_id, &task_id)` to the next agent prompt.
6. `crates/roko-cli/src/runner/state.rs`
   - `set_replan_context(...)` and `take_replan_context(...)` store the retry prompt context.

The preferred v2 implementation is deterministic and synchronous: use the public `roko_gate` classification/rendering helpers in the Runner v2 retry path. Use `roko_learn::error_enrichment::enrich_error_digest(...)` only if a cheap agent is already available in that call path without recursive dispatch.

## What to Change

1. Add a small pure helper in `crates/roko-cli/src/runner/event_loop.rs` or `gate_dispatch.rs`, for example:
   - `build_gate_retry_context(completion: &GateCompletion, state: &RunState, attempt_num: u32) -> String`
2. In that helper:
   - Combine failed verdict `summary` and `error_digest` values from `completion.verdicts`; fall back to `completion.output`.
   - Choose a gate label from the first failed verdict's `gate_name`; if none is available, use `completion.kind` rendered as `gate`, `plan_verify`, or `merge`.
   - Call `roko_gate::classify_gate_failure(gate_label, &combined_output)`.
   - Render the classification with `roko_gate::render_failure_classification(...)`.
   - Preserve the existing raw excerpts: truncate gate output to about 3000 chars and previous agent output to about 2000 chars.
3. Replace the existing inline `format!(...)` retry-context block in the gate failure branch with the helper output.
4. The retry context must include these sections, in this order:
   - `## IMPORTANT: Your previous attempt failed`
   - attempt number
   - `### Error analysis` with the rendered classification JSON or stable human summary
   - `### Gate error output`
   - `### What you did last time`
   - existing strategy hint for repeated failures
5. Keep the existing storage and wiring: `state.set_replan_context(...)` in the failure branch, then `dispatch_action(...)` consumes it through `take_replan_context(...)` before spawning the retry agent.
6. Add focused tests for the pure helper:
   - Compile-like output such as `error[E0308]: mismatched types` produces an "Error analysis" section and keeps the raw gate output.
   - Test-like output mentioning a failing test name preserves that test name in the retry context.
   - Long gate/agent output is truncated and does not make the prompt unbounded.

## What NOT to Do

- Don't import from `orchestrate.rs` (it's behind a feature gate).
- Don't change the gate pipeline itself.
- Don't launch an additional LLM call from the retry path unless the runtime already provides a cheap enrichment agent without recursive dispatch.
- Don't replace the existing retry context; enrich it while preserving raw gate output and previous agent output.
- Don't put this only in legacy workflow/orchestrator code. The wire target is Runner v2 `event_loop.rs`.

## Wire Target

```bash
# Trigger a gate failure and check the retry prompt
# (This requires a plan with a task that produces failing code)
cargo run -p roko-cli -- plan run plans/ 2>&1 | grep -B2 -A10 "retry\|enriched\|error analysis"
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test -p roko-gate compile_errors`
- [ ] `cargo test -p roko-cli gate_retry`
- [ ] `cargo test --workspace`
- [ ] Gate failure retries include error analysis context
- [ ] `rg -n 'classify_gate_failure|render_failure_classification|build_gate_retry_context|Error analysis' crates/roko-cli/src/runner crates/roko-gate/src --glob '*.rs'`
- [ ] Status Log documents whether the implementation used deterministic `roko_gate` classification or an already-available cheap enrichment agent

## Status Log

| Time | Agent | Action |
|------|-------|--------|
