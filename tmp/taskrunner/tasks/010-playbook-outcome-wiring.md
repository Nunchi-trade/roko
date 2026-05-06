# Task 010: Wire Playbook Outcome Recording via Prompt Diagnostics

```toml
id = 10
title = "Wire playbook outcome recording using prompt_diagnostics.playbook_ids on task completion"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/state.rs",
    "crates/roko-learn/src/playbook.rs",
]
exclusive_files = []
estimated_minutes = 60
```

## Context

`PlaybookStore::record_outcome(id, success)` exists and works — it updates per-playbook
success_count/failure_count and last_used_ms. But the event loop never calls it.

The key connection: during dispatch, `prompt_diagnostics.playbook_ids` records which playbooks
influenced the prompt assembly. On task completion, those playbook IDs should get outcome
feedback. This closes the learning loop: playbooks that lead to success get higher scores.

Sources:
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` — W10-E: Playbook outcome recording never wired
- Audit finding: must use prompt_diagnostics.playbook_ids, not arbitrary IDs

## Background

Read these files:
1. `crates/roko-learn/src/playbook.rs` — find `record_outcome()` method signature
2. `crates/roko-cli/src/runner/event_loop.rs` — find task completion handler
3. Find playbook ID connection:
   ```bash
   grep -rn 'playbook_ids\|prompt_diagnostics' crates/roko-cli/src/runner/ --include='*.rs' | grep -v target/
   ```

## What to Change

1. **On task completion** (success or failure), extract `playbook_ids` from the dispatch's
   `prompt_diagnostics` (stored in the task's dispatch context or result).
2. **For each playbook ID**, call `playbook_store.record_outcome(id, success)`.
3. **Handle errors gracefully** — playbook recording failure logs a warning, doesn't fail the run.
4. **If `playbook_ids` is not available** in the v2 runner's task result, trace back to where
   it's populated during dispatch and ensure it's propagated to the completion handler.

## What NOT to Do

- Don't change the PlaybookStore API.
- Don't call `record_outcome` with arbitrary task IDs — use the actual playbook IDs from diagnostics.
- Don't block on playbook recording (async, fire-and-forget with error logging).
- Don't record on every retry attempt; record once for the terminal task outcome to avoid double-counting.

## Implementation Notes

Current dispatch/completion call chain:
`event_loop.rs` `ExecutorAction::ExecuteTask` →
`dispatcher.plan(task_def, &dispatch_ctx)` →
`dispatch_plan.prompt.diagnostics` →
`RunnerEvent::prompt_assembled(...)` →
agent run →
`ExecutorAction::RunGate` →
`gate_dispatch::spawn_gate()` →
gate completion branch in `event_loop.rs`.

Files/functions to read before editing:
- `crates/roko-learn/src/playbook.rs`: `PlaybookStore::record_outcome(id, success) -> io::Result<bool>`.
- `crates/roko-cli/src/dispatch/prompt_builder.rs`: `PromptDiagnostics.playbook_ids` and
  `PromptAssembler::enforce_budget()` where ids are copied from included playbook sections.
- `crates/roko-cli/src/runner/types.rs`: `PromptAssemblyDiagnostics` and
  `RunnerEvent::prompt_assembled()`.
- `crates/roko-cli/src/runner/state.rs`: current per-task state fields and `reset_for_task()`;
  this is the right place to store playbook IDs until task completion.
- `crates/roko-cli/src/runner/event_loop.rs`: dispatch block around
  `let prompt_diagnostics = dispatch_plan.prompt.diagnostics.clone();`, success completion branch,
  terminal gate-failure branch, and spawn-failure branch.
- `crates/roko-cli/src/runner/event_loop.rs`: `seed_playbooks_if_empty()` shows the canonical
  playbook directory via `config.layout.playbooks_dir()`.

Mechanical steps:
1. Add a small state holder in `RunState`, for example
   `task_playbook_ids: HashMap<String, Vec<String>>` keyed by `"{plan_id}:{task_id}"`, with helpers:
   - `record_task_playbook_ids(plan_id, task_id, ids)`
   - `take_task_playbook_ids(plan_id, task_id) -> Vec<String>`
   Deduplicate IDs while preserving first-seen order.
2. In the dispatch block, immediately after cloning `prompt_diagnostics`, store
   `prompt_diagnostics.playbook_ids.clone()` in `RunState`. Do this before moving diagnostics into
   `RunnerEvent::prompt_assembled(...)`.
3. Add a helper in `event_loop.rs`, e.g. `spawn_record_playbook_outcomes(config, ids, success,
   plan_id, task_id)`, that:
   - returns early for an empty ID list;
   - constructs `PlaybookStore::new(config.layout.playbooks_dir())`;
   - `tokio::spawn`s an async task;
   - calls `record_outcome(&id, success).await` for each ID;
   - logs `warn!` on `Err` and `debug!` or `info!` when an ID is missing (`Ok(false)`), without
     failing the runner.
4. Call the helper exactly once when a task reaches a terminal outcome:
   - success path: after `completion.passed` and before or after `state.mark_task_completed(...)`;
   - failure path: only in the non-retryable or retries-exhausted branch, after `state.task_failed()`;
   - do not record during retryable `GateFailed` transitions.
5. Avoid penalizing prompt playbooks for infrastructure failures before the agent had a chance to
   run, such as spawn failures or model-resolution failures, unless the failure is classified as a
   task/gate failure.
6. Keep `prompt.assembled` event payload unchanged; the new state storage is an internal bridge from
   prompt diagnostics to completion.

Tests to add/update:
- `RunState` unit test: record/take playbook IDs deduplicates and clears the key.
- `event_loop.rs` helper test with a temp playbook store: two IDs, one existing and one missing;
  existing playbook count increments, missing ID is ignored.
- Runner integration test if feasible: mocked plan run with an injected playbook section records
  success once after the final gate pass.
- Negative test: a retryable gate failure followed by success increments success once and does not
  increment failure.

## Wire Target

```bash
ROKO_DISPATCHER=mock-self-host-fixture \
ROKO_MOCK_STATE_PATH=/tmp/roko-playbook-mock-state.txt \
cargo run -p roko-cli -- plan run plans/
cargo run -p roko-cli -- learn all 2>&1 | grep -i 'playbook\|success_count\|failure_count'
```

**Expected behavior**: for playbook IDs included in `prompt.assembled.playbook_ids`, terminal task
success increments `success_count`; terminal task failure increments `failure_count`; retry attempts
do not double-count.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `grep -rn 'record_outcome' crates/roko-cli/ --include='*.rs' | grep -v target/` — shows caller in event_loop
- [ ] `grep -rn 'playbook_ids' crates/roko-cli/src/runner/ --include='*.rs' | grep -v target/` — shows storage in runner state plus prompt event emission
- [ ] Playbook success/failure counts increment once after terminal plan task outcomes

## Status Log

| Time | Agent | Action |
|------|-------|--------|
