# Task 034: Wire SectionOutcome Recording into Runner v2 Task Completion

```toml
id = 34
title = "Wire SectionOutcome telemetry into the runner event loop after task completion"
track = "wiring"
wave = "wave-2"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/persist.rs",
    "crates/roko-learn/src/section_outcome.rs",
]
exclusive_files = ["crates/roko-cli/src/runner/persist.rs"]
estimated_minutes = 90
```

## Context

`SectionOutcomeStore` in roko-learn records which prompt sections were present during each
agent invocation and whether the task passed or failed. This telemetry feeds the contextual
bandit for future prompt section selection. The store has full JSONL persistence and
summarization.

The FeedbackService in roko-learn already has `apply_section_outcome()` which records
outcomes. But the Runner v2 event loop never calls it. When a task completes (pass or fail),
the runner should log which prompt sections were used and the outcome status.

The missing link: after each task completion in the runner event loop, emit section outcome
records to the SectionOutcomeStore.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- DCA-6
- `crates/roko-learn/src/section_outcome.rs` -- SectionOutcomeStore, SectionOutcomeRecord

## Background

Read these files first:
1. `crates/roko-learn/src/section_outcome.rs` -- SectionOutcomeStore, SectionOutcomeRecord::from_workspace()
2. `crates/roko-learn/src/feedback_service.rs` -- apply_section_outcome() (lines 392, 423)
3. `crates/roko-cli/src/runner/event_loop.rs` -- task completion handling
4. `crates/roko-cli/src/runner/persist.rs` -- persistence helpers

## What to Change

1. **Identify the task completion point** in `event_loop.rs` where the gate verdict is known
   (the `GateCompletion` handling around line 930+). At this point we know:
   - Which task was executed (plan_id, task_id)
   - Whether the gate passed or failed
   - The prompt sections used (from the dispatch context or prompt cache)

2. **Record section outcomes** after each task completion:
   - Open/create the `SectionOutcomeStore` at `.roko/learn/section-outcomes.jsonl`
   - Build `SectionOutcomeRecord`s from the prompt section IDs used in the dispatch
   - Set status based on gate outcome (Passed/Failed)
   - Append the records via `store.append_many()`

3. **Thread prompt section IDs** through the task execution path. The prompt builder
   already tracks which sections it includes. If section IDs aren't available at the
   completion point, you'll need to:
   - Store the section IDs from prompt assembly in the `RunState` (keyed by plan_id/task_id)
   - Retrieve them when the gate completes

4. **Add persistence helper** in `persist.rs` for the section outcome path:
   ```rust
   pub fn section_outcomes_path(workdir: &Path) -> PathBuf {
       workdir.join(".roko/learn/section-outcomes.jsonl")
   }
   ```

## What NOT to Do

- Don't modify the SectionOutcomeStore or SectionOutcomeRecord -- they're well-designed.
- Don't add section outcome recording to orchestrate.rs (it's legacy).
- Don't block task execution on section outcome persistence -- fire-and-forget or async append.
- Don't add new prompt section tracking -- use whatever section IDs the prompt builder already
  provides. If none are available, use placeholder IDs like "system_prompt", "task_context",
  "knowledge_context" based on what was included.

## Wire Target

```bash
# Run a plan and check for section outcome data:
cargo run -p roko-cli -- plan run plans/
cat .roko/learn/section-outcomes.jsonl | head -5
# Each line should be a SectionOutcomeRecord with section_id, status, action_id
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `grep -rn 'SectionOutcome\|section_outcome' crates/roko-cli/ --include='*.rs' | grep -v target/` -- shows at least one callsite in the runner
- [ ] After a plan run, `.roko/learn/section-outcomes.jsonl` contains records
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`

## Implementation Detail

### Runtime call chain to trace

1. `crates/roko-cli/src/main.rs` parses `roko plan run <dir>`.
2. `crates/roko-cli/src/commands/plan.rs::cmd_plan` loads plans and calls
   `roko_cli::runner::event_loop::run(...)`.
3. `crates/roko-cli/src/runner/event_loop.rs::run` creates `PersistPaths`, the
   `PromptCache`, channels, and the `RunState`, then enters the `tokio::select!`
   loop.
4. `dispatch_action(..., ExecutorAction::SpawnAgent { .. })` assembles prompts
   through `ctx.factory.dispatcher().plan(task_def, &dispatch_ctx)`.
5. `dispatch_plan.prompt.diagnostics` is the only section telemetry available
   in runner v2 today. It has section names only:
   `included_sections`, `dropped_sections`, `estimated_tokens`,
   `knowledge_ids`, and `playbook_ids`.
6. The terminal gate verdict arrives in the `Some(completion) = gate_rx.recv()`
   branch. `completion_attempt` is created immediately after the receive and is
   the stable key for joining dispatch-time prompt sections to completion-time
   outcome.

Do not try to call `SectionOutcomeRecord::from_workspace()` from the runner:
runner v2 currently does not build a `CognitiveWorkspace` or have
`PromptSectionAudit` rows. Use the runner diagnostics fallback described below.

### Mechanical steps

1. In `crates/roko-cli/src/runner/persist.rs`, add one path helper near the other
   path helpers:
   ```rust
   pub fn section_outcomes_path(workdir: &Path) -> PathBuf {
       RokoLayout::for_project(workdir)
           .learn_dir()
           .join("section-outcomes.jsonl")
   }
   ```
   `PersistPaths::from_workdir()` already creates `.roko/learn`; the helper may
   still be used with `SectionOutcomeStore::open_creating()` for safety.

2. In `crates/roko-cli/src/runner/event_loop.rs`, import the learn types:
   `SectionGateOutcome`, `SectionKind`, `SectionOutcomeRecord`,
   `SectionOutcomeStatus`, `SECTION_OUTCOME_SCHEMA_VERSION`, and
   `SectionOutcomeStore`.

3. Add a small local struct in `event_loop.rs`:
   ```rust
   #[derive(Clone, Debug)]
   struct PromptOutcomeContext {
       attempt: TaskAttemptRef,
       role: String,
       provider: String,
       model: String,
       included_sections: Vec<String>,
       dropped_sections: Vec<String>,
   }
   ```
   Keep it private to the runner. Do not add fields to `RunState`; a local
   `HashMap<TaskAttemptRef, PromptOutcomeContext>` inside `run()` is enough and
   avoids touching `state.rs`.

4. Create `let mut prompt_outcomes = HashMap::<TaskAttemptRef, PromptOutcomeContext>::new();`
   near the other `run()`-scoped mutable state.

5. In the `SpawnAgent` dispatch path, after `dispatch` has been resolved but
   before it is matched/moved, derive a provider label:
   - CLI: `cli_provider.as_ref().map(|p| p.descriptor.provider_id.clone()).unwrap_or_else(|| "cli".to_string())`
   - Bridge: `provider_id.clone()`
   Store a `PromptOutcomeContext` keyed by `attempt_ref.clone()` using
   `prompt_diagnostics.included_sections.clone()` and
   `prompt_diagnostics.dropped_sections.clone()`. Keep the existing
   `RunnerEvent::prompt_assembled(...)` unchanged.

6. Add helper functions in `event_loop.rs`:
   - `fn stable_prompt_section_id(name: &str) -> String` returning
     `prompt:<normalized-name>`.
   - `fn stable_prompt_action_id(name: &str) -> String` returning
     `prompt_section:<normalized-name>`.
   - `fn prompt_context_to_section_records(...) -> Vec<SectionOutcomeRecord>`.
   Normalize with ASCII lowercase and single `-` separators; do not pull in a
   dependency for this.

7. For each included section name, emit a `SectionOutcomeRecord` with:
   - `section_kind: SectionKind::Prompt`
   - `included: true`
   - `estimated_tokens: 0`, `tokens_used: 0`, `token_budget: None`
   - `source_type/source_id/experiment_id: None`
   - `workspace_id: config.workdir.display().to_string()`
   - `invocation_id: format!("{}:{}", state.run_id(), attempt.key())`
   - `task_id: attempt.task_id.clone()`
   - `task_type`: use `task_def.role.as_deref().unwrap_or("task")` if available
     from `task_index`, otherwise `"task"`
   - `role_id`, `provider`, `model` from `PromptOutcomeContext`
   - `gate_outcomes`: map `completion.verdicts` into `SectionGateOutcome`
     with `gate_id = gate_name`, `outcome = "passed"`/`"failed"`, and
     `required = true`
   - `review_verdicts: Vec::new()`

   For each dropped section name, emit the same shape with `included: false`.
   Do not invent per-section token counts from the aggregate prompt estimate.

8. In the gate completion branch, record only `GateCompletionKind::Gate`:
   - If `completion.passed && completion.rung < config.max_gate_rung`, do not
     record yet; the task has not reached a terminal verdict.
   - If `completion.passed` on the final rung, record status
     `SectionOutcomeStatus::Passed`, then remove the prompt context for that
     attempt.
   - If `!completion.passed`, record status `SectionOutcomeStatus::Failed`
     before the retry decision branches. Remove the context so a retry attempt
     gets its own fresh prompt telemetry.
   - Skip `GateCompletionKind::Merge` and `GateCompletionKind::PlanVerify`.

9. Add an async append helper:
   ```rust
   async fn append_section_outcomes(
       path: PathBuf,
       records: Vec<SectionOutcomeRecord>,
   ) {
       if records.is_empty() {
           return;
       }
       match SectionOutcomeStore::open_creating(path).await {
           Ok(store) => {
               if let Err(err) = store.append_many(&records).await {
                   warn!(err = %err, "failed to append section outcome records");
               }
           }
           Err(err) => warn!(err = %err, "failed to open section outcome store"),
       }
   }
   ```
   Await this helper from the event loop so records are durable before the run
   exits, but never return an error or fail the task because telemetry failed.

### Tests to add or update

- Add unit tests in `event_loop.rs` for:
  - `stable_prompt_section_id` normalization.
  - `prompt_context_to_section_records` emits both included and dropped
    sections with the expected status/action ids.
  - Failed gate completion status maps to `SectionOutcomeStatus::Failed`.
- Add a small test in `persist.rs` for `section_outcomes_path(&tmp)` ending in
  `.roko/learn/section-outcomes.jsonl`.

### Observable behavior

After a real `roko plan run <plans-dir>`, each terminal task attempt should add
one JSONL row per prompt section name in `.roko/learn/section-outcomes.jsonl`.
Every row must include `section_id`, `action_id`, `task_id`, `status`,
`provider`, `model`, and `gate_outcomes`.

### Anti-patterns

- Do not wire this through `orchestrate.rs`; runner v2 is
  `runner/event_loop.rs`.
- Do not add `CognitiveWorkspace` construction to this task. The runner lacks
  enough raw-content-free audit metadata; name-based records are the current
  mechanical fallback.
- Do not persist raw prompt text or agent output in section outcome records.
- Do not use `std::fs` from the async event loop.
- Do not let section outcome persistence errors affect gate/task outcome.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
