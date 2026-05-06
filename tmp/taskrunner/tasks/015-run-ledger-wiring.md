# Task 015: Wire RunLedger for Per-Task Cost Tracking

```toml
id = 15
title = "Wire RunLedger into Runner v2 for per-task cost tracking"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-runtime/src/run_ledger.rs",
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/state.rs",
    "crates/roko-cli/src/commands/plan.rs",
]
exclusive_files = []
estimated_minutes = 60
```

## Context

`RunLedger` exists in `roko-runtime/src/run_ledger.rs` for per-task cost tracking but is
not called from the Runner v2 event loop. Cost breakdown by task is not available.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` — DCA-2: Wire run_ledger

## Background

Read these files first:
1. `crates/roko-runtime/src/run_ledger.rs` — RunLedger struct and methods
   - There is no `RunLedger::record_cost()` method.
   - Existing write APIs are `record_agent_completed(...)`, `record_agent_failed(...)`, and `record_gate_run(...)`.
   - Token/cost input type is `roko_core::foundation::TokenUsage`.
2. `crates/roko-cli/src/runner/event_loop.rs`
   - `run(...)` drives Runner v2.
   - `dispatch_action(...)` spawns agents and appends retry context to prompts.
   - Gate completion handling calls `state.task_completed()` / `state.task_failed()`.
   - `build_report(...)` constructs the final `RunReport`.
3. `crates/roko-cli/src/runner/agent_events.rs`
   - `handle_agent_event(...)` accumulates `RunState.tokens_in`, `tokens_out`, cache token counts, `cost_usd`, `agent_model`, and `agent_provider`.
4. `crates/roko-cli/src/runner/state.rs`
   - `RunState::start_task(...)` resets per-task counters.
   - `RunState::task_completed()` / `task_failed()` call `roll_into_totals()`.
5. `crates/roko-cli/src/commands/plan.rs`
   - `PlanCmd::Run` calls `roko_cli::runner::event_loop::run(...)`.
   - Human summary and `--json` output are printed from the returned `RunReport`.

## What to Change

1. **Do not call a nonexistent `RunLedger::record_cost()`**. Preserve the existing `RunLedger` API and use `record_agent_completed(...)` / `record_agent_failed(...)` with a `TokenUsage` built from Runner v2 state.
2. Add a small Runner v2 task-cost record, for example `TaskCostReport`, with:
   - `plan_id`, `task_id`, `status`
   - `model`, `provider`
   - `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `total_tokens`
   - `cost_usd`, `agent_calls`
3. Store these records on `RunState` or collect them in `event_loop.rs`; include them in `RunReport` as `task_costs: Vec<TaskCostReport>`.
4. Record the per-task cost immediately before or inside the gate completion paths that currently call `state.task_completed()` / `state.task_failed()`, while `RunState` still contains the current task counters. Use:
   - `state.plan_id` or `completion.plan_id`
   - `state.current_task` or `completion.task_id`
   - `state.agent_model`, `state.agent_provider`
   - `state.tokens_in`, `state.tokens_out`, `state.cache_read_tokens`, `state.cache_write_tokens`, `state.cost_usd`, `state.task_agent_calls`
5. Feed the same data into a `RunLedger` instance for compatibility accounting:
   - Build `TokenUsage { input_tokens, output_tokens, total_tokens, cost_usd }`.
   - Use a role string that preserves task identity, for example `format!("task:{plan_id}/{task_id}")`, unless a better typed field is added without breaking the public API.
   - On failed tasks, call `record_agent_failed(...)` only if no usable token/cost completion exists; otherwise still record the completed agent outcome and mark the task status in `TaskCostReport`.
6. Update `build_report(...)` so the final `RunReport` carries the task-cost records.
7. Update `crates/roko-cli/src/commands/plan.rs`:
   - In human output, print a compact "Task costs:" block after the plan summary when records are non-empty.
   - In `--json` output, include a stable `task_costs` array.

## What NOT to Do

- Don't change the RunLedger API.
- Don't add USD conversion (that's a separate concern).
- Don't change the efficiency event system.
- Don't recompute totals by replaying `.roko/events.jsonl`; use the live `RunState` counters that Runner v2 already maintains.
- Don't print per-agent streaming token events as the final summary. The requested observable is one summary row per task.
- Don't drop cache token counts from the task-level record even if totals remain input/output only.
- Don't let final report totals diverge from existing `RunState::roll_into_totals()` totals.

## Wire Target

```bash
cargo run -p roko-cli -- plan run plans/
# Should print per-task cost summary at completion

cargo run -p roko-cli -- plan run plans/ --json
# JSON should include a task_costs array with plan_id/task_id/token/cost fields
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test -p roko-runtime run_ledger`
- [ ] `cargo test -p roko-cli runner`
- [ ] `cargo test --workspace`
- [ ] `rg -n 'RunLedger|record_agent_completed|task_costs|TaskCost' crates/roko-cli crates/roko-runtime --glob '*.rs'`
- [ ] Human `roko plan run ...` output includes one per-task cost summary row for each attempted task
- [ ] `roko plan run ... --json` includes `task_costs` and existing total cost fields are unchanged

## Status Log

| Time | Agent | Action |
|------|-------|--------|
