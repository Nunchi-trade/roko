# Task 078: Learning Loop Completeness

```toml
id = 78
title = "Ensure every agent dispatch surface records cascade observations and provider health"
track = "wiring"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-cli/src/agent_exec.rs",
    "crates/roko-cli/src/commands/prd.rs",
    "crates/roko-cli/src/prd.rs",
    "crates/roko-learn/src/runtime_feedback.rs",
]
exclusive_files = []
estimated_minutes = 180
```

## Context

Combines GAP-M-7 and GAP-M-8. Two gaps remain in the learning feedback loop after Batches 8–9:

1. **GAP-M-7**: `roko prd draft/plan/consolidate` commands dispatch agents via
   `run_agent_capture_silent` (in `crates/roko-cli/src/agent_exec.rs`) or helper paths that do
   not pass an `AgentExecEpisode`. Some PRD branches now manually call
   `commands::util::persist_capture_episode`, but that duplicate helper is not the canonical
   `agent_exec::persist_capture_episode` path and `prd plan` still has no complete episode
   persistence. As a result, PRD learning observations are incomplete and provider-health
   persistence can diverge from direct agent-exec paths.

2. **GAP-M-8**: The in-memory `ProviderHealthTracker` (inside `LearningRuntime`) transitions
   from `Unhealthy` to `Probing` only when the runtime observes a success. When the user runs
   `--model <provider>` directly via a PRD or research command and it succeeds, the call goes
   through `agent_exec.rs` → `record_persisted_provider_health` which updates the persisted
   JSON health file — but NOT the in-memory `ProviderHealthTracker` held by any live
   `LearningRuntime`. So the circuit breaker stays `Open` in memory until process restart, even
   though the manual success proved the provider is healthy.

The existing `persist_capture_episode` function in `agent_exec.rs` DOES record cascade
observations when called with an episode (see the test at line 434+ that asserts
`cascade-router.json` is written). The fix for GAP-M-7 is to wire episode persistence into
every PRD dispatch path. The fix for GAP-M-8 is to ensure the persisted health file is the
ground truth that `LearningRuntime` loads on construction rather than maintaining a diverged
in-memory state.

## Background

Read these files before writing any code:

1. `crates/roko-cli/src/agent_exec.rs` — `run_agent_capture_impl` and
   `persist_capture_episode`. Note how `AgentExecEpisode` is optional and how the cascade
   router is updated only when an episode is provided. Also note the existing test at line 382+
   that verifies `cascade-router.json` is updated when an episode is given.
2. `crates/roko-cli/src/commands/prd.rs` — `cmd_prd`. The PRD subcommand handlers call
   `run_agent_capture_silent` without providing an `AgentExecEpisode`. Line 296 shows the
   import. Current code manually calls `crate::commands::util::persist_capture_episode` after
   `draft new`, `draft edit`, and `consolidate`, but that helper does not resolve configured
   model slugs the same way as `agent_exec::persist_capture_episode` and does not update
   persisted provider health. `PrdCmd::Plan` delegates into `roko_cli::prd::generate_plan...`,
   where learning is still missing.
3. `crates/roko-learn/src/model_call_feedback.rs` — `ModelCallFeedbackRecorder` and
   `record_provider_health_at`. The recorder exists and works — it just isn't called from PRD
   paths.
4. `crates/roko-learn/src/runtime_feedback.rs` — `LearningRuntime`. Understands the split
   between in-memory `ProviderHealthTracker` (runtime) and the serialized
   `ProviderHealthRegistry` (disk). The circuit breaker lives in `ProviderHealthTracker`; only
   `record_completed_run` updates it.
5. `crates/roko-cli/src/learning_helpers.rs` — `record_persisted_provider_health`. This writes
   to disk but does not affect any in-memory `ProviderHealthTracker`.
6. `crates/roko-cli/src/runner/event_loop.rs` — How the runner event loop records learning for
   comparison. Uses `LearningRuntime` via `record_completed_run` which updates both in-memory
   state and persists to disk.

## What to Change

### 1. Wire `AgentExecEpisode` into every PRD dispatch call (GAP-M-7)

In `crates/roko-cli/src/commands/prd.rs` and `crates/roko-cli/src/prd.rs`, ensure every PRD
agent dispatch produces exactly one call to the canonical
`roko_cli::agent_exec::persist_capture_episode` path. Do not double-log by both calling
`run_agent_capture_logged` and manually persisting after artifact validation.

Use these episode identifiers:

- `PrdCmd::Draft { New }` → `task_kind = "prd-draft-new"`, `task_id = format!("prd:draft:{slug}")`
- `PrdCmd::Draft { Edit }` → `task_kind = "prd-draft-edit"`, `task_id = format!("prd:draft:edit:{slug}")`
- `PrdCmd::Plan` → `task_kind = "prd-plan-generate"`, `task_id = format!("prd:plan:{slug}")`
- `PrdCmd::Consolidate` → `task_kind = "prd-consolidate"`, `task_id = "prd:consolidate"`

Mechanical wiring:

1. In `commands/prd.rs`, replace the import with
   `use roko_cli::agent_exec::{AgentExecOpts, persist_capture_episode, run_agent_capture_silent};`
   (and `run_agent_capture_logged` only for paths that genuinely log directly).
2. Keep `run_agent_capture_silent` only if the command must compute artifact success after
   the agent exits (`draft new`, `draft edit`, `consolidate`). In those paths, replace
   `crate::commands::util::persist_capture_episode(...)` with
   `roko_cli::agent_exec::persist_capture_episode(...)` so model-key resolution,
   cascade-router persistence, and provider-health persistence all use the canonical helper.
3. If a path can use process success directly and does not need post-processing, prefer
   `run_agent_capture_logged` with `AgentExecEpisode`.
4. In `crates/roko-cli/src/prd.rs`, `generate_plan_from_prd_with_model()` currently calls
   `run_agent_capture_silent` for the initial plan attempt and retries. Add one canonical
   `persist_capture_episode` call for the overall plan generation outcome:
   - success when `GenerationOutcome::fully_successful()` is true;
   - failure before each early `return Err(...)` caused by nonzero exit, empty output, or
     unparseable TOML after retries;
   - `model` should be the effective model passed to the dispatch
     (`model.or_else(|| resolved.config.agent.model.as_deref())`);
   - `agent_command` should be derived with the same helper used by other direct dispatch
     paths (`command_from_config(workdir_ref).unwrap_or_else(|| "claude".to_string())`).
5. Use a small local helper in `prd.rs` if needed to avoid duplicating the long
   `persist_capture_episode` argument list across success and failure branches.

### 2. Ensure the `ProviderHealthTracker` loads from disk on construction (GAP-M-8)

The core problem is that `LearningRuntime::open_under` constructs a fresh
`ProviderHealthTracker::new()` (all healthy by default) and the serialized
`ProviderHealthRegistry` snapshot is only used for persisting — not for bootstrapping the
in-memory circuit state. When a PRD command records a manual success via
`record_persisted_provider_health`, the on-disk file is updated but the next `LearningRuntime`
instance starts fresh.

The fix: When `LearningRuntime` opens, load `paths.root.join("provider-health.json")` with
`ProviderHealthRegistry::load_or_new()` and pre-seed the in-memory `ProviderHealthTracker`
from `registry.snapshot()`. Specifically:

- For `CircuitState::Open`, call `record_failure(provider_id)` three times on the tracker
  (the default threshold is three failures) so `is_healthy(provider_id)` returns false until
  the tracker recovery window elapses.
- For `CircuitState::Closed` and `CircuitState::HalfOpen`, call `record_success(provider_id)`
  or skip insertion; both are acceptable as long as `is_healthy(provider_id)` returns true.
- Put this logic in a private helper in `runtime_feedback.rs`, e.g.
  `fn provider_health_tracker_from_persisted(root: &Path) -> ProviderHealthTracker`, and call it
  from both `LearningRuntime::open()` and `LearningRuntime::open_with_models()`.

Alternatively (simpler): when `record_persisted_provider_health` is called (i.e., after a
direct CLI agent success), also call `record_success` on any `LearningRuntime` that is in scope
for that process. Since `agent_exec.rs` doesn't hold a `LearningRuntime` at the point where
provider health is recorded, the cleanest fix is to load-and-resave: when recording a manual
success, load the `ProviderHealthRegistry`, call `record_success`, and save. This is already
done — the real gap is that the fresh `LearningRuntime` on the NEXT invocation doesn't read
this file back. Fix the construction path.

### 3. Add a `DispatchAuditTest` (regression guard)

In `crates/roko-cli/src/agent_exec.rs` tests, add a test named
`dispatch_surfaces_provide_episodes`. The old "zero `run_agent_capture_silent` calls" check is
too blunt because PRD artifact validation may need silent capture first. Instead assert that
every PRD source file with silent dispatch also references the canonical persistence helper:

```rust
#[test]
fn dispatch_surfaces_provide_episodes() {
    let commands_prd = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands/prd.rs")
    ).unwrap();
    assert!(
        !commands_prd.contains("crate::commands::util::persist_capture_episode"),
        "PRD commands must use roko_cli::agent_exec::persist_capture_episode"
    );
    assert!(
        commands_prd.matches("persist_capture_episode").count()
            >= commands_prd.matches("run_agent_capture_silent").count(),
        "every silent PRD dispatch needs canonical episode persistence"
    );

    let prd_rs = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/prd.rs")
    ).unwrap();
    assert!(
        prd_rs.contains("prd-plan-generate")
            && prd_rs.contains("persist_capture_episode"),
        "generate_plan_from_prd_with_model must persist a prd-plan-generate episode"
    );
}
```

This test will fail immediately if someone adds a new PRD dispatch path without wiring learning.

Also add a `roko-learn` unit test in `runtime_feedback.rs`:

1. Create a temp `learn` root.
2. Use `ProviderHealthRegistry::load_or_new(&learn_root.join("provider-health.json"))`.
3. Record three failures for provider `"zai"` and save the registry.
4. Open `LearningRuntime::open_under(&learn_root)`.
5. Assert `!runtime.provider_health().is_healthy("zai")`.
6. Record a manual success in the registry, save, reopen the runtime, and assert
   `runtime.provider_health().is_healthy("zai")`.

## What NOT to Do

- Do NOT change `run_agent_capture_silent` itself. The function is correct for callers that
  genuinely don't need learning (e.g., internal scaffolding checks). Only the PRD commands need
  canonical persistence.
- Do NOT add a new learning path separate from `persist_capture_episode`. The existing path
  already works and has test coverage. Wire into it, don't duplicate it.
- Do NOT change the provider health circuit breaker thresholds or cooldown durations.
- Do NOT touch `crates/roko-cli/src/runner/event_loop.rs` — the runner event loop already
  calls `record_completed_run` correctly.
- Do NOT add new struct types or traits. This task is pure wiring.

## Wire Target

```bash
# Start a PRD draft (dispatches an agent)
cargo run -p roko-cli -- prd draft new "Test learning wire"

# Verify cascade-router.json was updated
cat .roko/learn/cascade-router.json | python3 -m json.tool | grep trials
# Expected: "trials" > 0 for at least one model

# Verify provider-health.json was updated
cat .roko/learn/provider-health.json | python3 -m json.tool | grep total_requests
# Expected: "total_requests" > 0 for at least one provider
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `dispatch_surfaces_provide_episodes` test passes
- [ ] `cargo run -p roko-cli -- prd draft new "test"` → `.roko/learn/cascade-router.json` updated
- [ ] `cargo run -p roko-cli -- prd plan <slug>` → `.roko/learn/cascade-router.json` updated
- [ ] `grep -n 'crate::commands::util::persist_capture_episode' crates/roko-cli/src/commands/prd.rs` → empty output
- [ ] `grep -n 'persist_capture_episode' crates/roko-cli/src/commands/prd.rs crates/roko-cli/src/prd.rs` → lists every PRD agent dispatch surface
- [ ] Any remaining `run_agent_capture_silent` call in PRD paths is followed by exactly one canonical `agent_exec::persist_capture_episode` call after artifact success/failure is known
- [ ] New `LearningRuntime` provider-health bootstrap test proves persisted Open providers reload as unhealthy and persisted successes reload as healthy
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file touched by this task

## Status Log

| Time | Agent | Action |
|------|-------|--------|
