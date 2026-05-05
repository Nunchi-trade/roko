# Task 033: Wire PostGateReflection into Runner v2 Gate Failure Path

```toml
id = 33
title = "Wire PostGateReflection insights into runner gate failure retry prompts"
track = "wiring"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-learn/src/post_gate_reflection.rs",
]
exclusive_files = ["crates/roko-cli/src/runner/event_loop.rs"]
estimated_minutes = 90
```

## Context

`PostGateReflectionStore` in roko-learn captures structured reflections from gate outcomes:
what failed, why, and what lesson can be drawn. It's already called from
`LearningRuntime::complete_gate_run()` in `runtime_feedback.rs` (the learning subsystem),
where it accumulates reflection records and playbook candidates.

However, the Runner v2 gate failure path in `event_loop.rs` (lines 991-1015) builds its
retry context with raw gate output and agent output. It does NOT consult the reflection
store for historical failure patterns on similar tasks. This means the agent retries blind,
without the accumulated lessons from past failures.

The wiring gap: when building the retry prompt context after a gate failure, load the
reflection store, query for matching reflections (same gate, similar task), and include
any relevant lessons in the retry context.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- DCA-5
- `crates/roko-learn/src/post_gate_reflection.rs` -- PostGateReflectionStore
- `crates/roko-learn/src/runtime_feedback.rs` -- LearningRuntime where reflections are recorded

## Background

Read these files first:
1. `crates/roko-cli/src/runner/event_loop.rs` -- lines 960-1040 (gate failure retry path)
2. `crates/roko-learn/src/post_gate_reflection.rs` -- PostGateReflectionStore, PostGateReflectionRecord
3. `crates/roko-cli/src/runner/state.rs` -- RunState::set_replan_context() (lines 613-622)
4. `crates/roko-learn/src/runtime_feedback.rs` -- `LearningPaths::post_gate_reflections_json`
   and the existing reflection recording path

Current call chain and storage details:
- Regular gate retry context is built in `crates/roko-cli/src/runner/event_loop.rs` after a
  failed gate with retry allowed. It currently includes gate output, previous agent output,
  and a strategy hint, then calls `state.set_replan_context(...)`.
- Plan-verify completion has a separate failure path; do not expand into that path unless
  the task scope is explicitly widened.
- Reflection records are persisted by learning feedback at
  `.roko/learn/post-gate-reflections.json`. In runner code, derive that path from
  `config.layout.learn_dir().join("post-gate-reflections.json")`; do not hard-code a
  relative `.roko` path.
- Use `PostGateReflectionStore::load(path)`, which is already tolerant of missing or
  malformed files.

## What to Change

1. **In the gate failure retry path** (`event_loop.rs`, around line 993-1014), after
   building the raw `replan_context` string, query the reflection store for relevant past
   reflections. Add small pure helpers so this is testable without running the full event
   loop:
   ```rust
   fn post_gate_reflection_path(config: &RunConfig) -> PathBuf
   fn lessons_from_post_gate_reflections(path: &Path, gate_name: &str, task_id: &str) -> Vec<String>
   ```
   The helper should:
   - Load `PostGateReflectionStore` from `config.layout.learn_dir().join("post-gate-reflections.json")`
   - Filter records matching the current gate name (`completion.gate_name` or similar)
   - Keep only failed outcomes (`ReflectionGateOutcome::Failed`)
   - Keep only records with `confidence > 0.3` and non-empty `proposed_lesson`
   - Prefer the same task id if the record has comparable task metadata; otherwise fall
     back to same-gate matches because there is no semantic similarity helper here
   - Sort by `created_at` descending, deduplicate lesson strings, and take at most 3
   - Append their `proposed_lesson` text to the `replan_context`

2. **Format the reflection lessons** clearly in the retry context:
   ```
   ### Lessons from past failures on this gate
   - [lesson 1]
   - [lesson 2]
   ```
   Append this after the existing strategy hint and before `state.set_replan_context(...)`.

3. **Handle missing/empty store gracefully** -- if the file doesn't exist or has no
   matching reflections, just skip this section. The store's `load()` method already
   returns `Default` on missing files.

4. **Add a test** that verifies reflection context is included in the retry prompt when
   reflections exist. The test should cover:
   - matching failed records are included
   - passed outcomes, mismatched gate names, low-confidence records, and empty lessons are
     excluded
   - only the 3 most recent lessons are returned
   - missing store path returns an empty list

5. **Log observable wiring** at debug level when lessons are added:
   ```rust
   tracing::debug!(gate = %gate_name, lessons = lessons.len(), "post-gate reflection lessons added to retry prompt");
   ```

## What NOT to Do

- Don't modify PostGateReflectionStore or its recording logic -- that's already wired
  through LearningRuntime.
- Don't add reflection recording to the runner -- it's already done in runtime_feedback.rs.
- Don't block the retry on reflection loading -- load synchronously (it's a small JSON file).
- Don't change the retry policy logic (backoff, attempt counting, etc.).
- Don't include successful/passed reflections in a failure retry prompt.
- Don't panic or fail the retry if the reflection JSON is absent or malformed.
- Don't use broad free-text similarity. Use same-gate filtering plus deterministic ordering
  unless a real similarity helper already exists.

## Wire Target

```bash
# Run a plan that will fail a gate, then retry:
RUST_LOG=roko_cli::runner::event_loop=debug cargo run -p roko-cli -- plan run plans/ 2>&1 | grep -i 'post-gate reflection'
# After the second failure, the retry prompt should include lessons from the first failure.
# Check the reflection store:
cat .roko/learn/post-gate-reflections.json | python3 -m json.tool
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo test -p roko-cli post_gate_reflection`
- [ ] `rg -n 'PostGateReflectionStore::load|post-gate-reflections.json|post-gate reflection lessons' crates/roko-cli/src/runner/event_loop.rs` -- shows the runner read path
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
