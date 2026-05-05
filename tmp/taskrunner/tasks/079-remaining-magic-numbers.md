# Task 079: Remaining Magic Number Centralization — Active Workflow Iteration Literals

```toml
id = 79
title = "Centralize remaining active workflow iteration literals in runner, agent, and serve paths"
track = "cleanup"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-core/src/defaults.rs",
    "crates/roko-cli/src/runner/",
    "crates/roko-agent/src/",
    "crates/roko-serve/src/",
]
exclusive_files = []
estimated_minutes = 120
```

## Context

Batches 39-47 (infrastructure-audit §6.1) cleared request-timeout, retry-policy, relay
circuit-breaker, runner plan timeout/DAG backoff, active provider tool-loop iteration
defaults, and vision-loop defaults. The audit status line (§6.1, last updated 2026-05-04)
records: "Active workflow iteration literals outside these paths remain open."

The remaining work targets numeric literals that control agent execution loop counts,
retry counts, and workflow step limits inside the **active runner stack** — the code
paths that run when a user executes `roko plan run`. The legacy orchestrate path
(`crates/roko-cli/src/orchestrate.rs`) is gated behind `legacy-orchestrate` and is
explicitly out of scope per the 2026-05-04 course correction note.

This task is a targeted audit-and-centralize pass, not a redesign. Every numeric
literal in active runner, agent dispatch, and serve route code that controls iteration
behavior must either already reference `roko_core::defaults` or must be migrated to it.
Literals that are incidental (e.g. array indices, fixed-size buffers, test assertions
against output lengths) are out of scope.

## Background

Read these files before starting:

1. `crates/roko-core/src/defaults.rs` — the complete file. Know what constants already
   exist so you do not duplicate them. The existing constants cover timeouts, retry
   policies, relay, runner plan, tool-loop iterations, and vision-loop. The gap is in
   "active workflow" iteration controls: things like max auto-fix attempts per agent
   step, max merge retries, max concurrent task concurrency limits, and any other
   integer that governs "how many times does the runner loop before giving up."
2. `crates/roko-cli/src/runner/mod.rs` — entry point for the active runner.
3. `crates/roko-cli/src/runner/event_loop.rs` — main execution loop. Search for
   integer literals next to `for`, `while`, `retry`, `attempt`, `iteration`, `limit`.
4. `crates/roko-cli/src/runner/merge.rs` — merge queue logic. Look for retry counts.
5. `crates/roko-cli/src/runner/task_dag.rs` — DAG execution. Concurrency limits.
6. `crates/roko-cli/src/runner/gate_dispatch.rs` — gate invocation. Retry counts.
7. `crates/roko-agent/src/dispatcher/mod.rs` — agent dispatch loop. Look for iteration
   caps that are not already routed through `tool_loop_max_iterations()`.
8. `crates/roko-serve/src/routes/` — route handlers. Look for numeric loop limits,
   poll counts, and retry caps that are not already in defaults.
9. `tmp/infrastructure-audit.md` §6.1 — read the full section and the batch update
   comments to understand exactly which literals were already cleared and which remain.

## What to Change

### 1. Audit pass — identify remaining literals

Run the following to find candidates:

```bash
# Iteration and retry literals in active runner
grep -n '[0-9]\+' crates/roko-cli/src/runner/*.rs | grep -E '(max_|limit|retry|attempt|iter|loop|cap|count)' | grep -v target/ | grep -v '//.*#' | grep -v 'test'

# Iteration literals in agent dispatch (excluding tool_loop which is already covered)
grep -n '[0-9]\+' crates/roko-agent/src/dispatcher/*.rs | grep -E '(max_|limit|retry|attempt|iter|loop|cap)' | grep -v target/

# Serve route loop controls
grep -n '[0-9]\+' crates/roko-serve/src/routes/*.rs | grep -E '(max_|limit|retry|attempt|iter|loop|poll)' | grep -v target/
```

For each match, classify it as:
- **In scope**: controls how many times a loop runs or how many retries are attempted
  in active execution code. Must be moved to `defaults.rs`.
- **Out of scope**: array size, test assertion, port number, HTTP status code, buffer
  size not related to iteration behavior.

Document your classification in the Status Log before writing any code.

Code inspection on 2026-05-05 found these likely in-scope candidates. Re-run the greps before
editing in case another task already moved one:

| File | Literal(s) | Classification |
|------|------------|----------------|
| `crates/roko-cli/src/runner/agent_stream.rs` | `max_turns: 50` | In scope: agent turn limit |
| `crates/roko-cli/src/runner/event_loop.rs` | local `DEFAULT_AGENT_TURN_LIMIT: u32 = 50` | In scope: move to `roko_core::defaults` and import it |
| `crates/roko-cli/src/runner/event_loop.rs` | `max_concurrent_plans: 4` | In scope: active executor concurrency default |
| `crates/roko-cli/src/runner/event_loop.rs` | `SnapshotWriter::new(4)` | In scope only if treating runner persistence queue capacity as active runtime limit; otherwise document out of scope |
| `crates/roko-cli/src/runner/event_loop.rs` | `state.iteration_for(...) >= 3` | In scope: retry strategy pivot after repeated failures |
| `crates/roko-cli/src/runner/persist.rs` | `MIN_RETRIES = 1`, `MAX_RETRIES = 5`, cold-start return `3`, `stats.total_count < 5` | In scope: adaptive gate retry bounds |
| `crates/roko-cli/src/runner/state.rs` | `attempt.min(5)`, `unwrap_or(32)`, `.min(45)` | In scope: retry cooldown backoff cap/fallback/max |
| `crates/roko-cli/src/runner/types.rs` | `max_concurrent_tasks.unwrap_or(4)`, `max_retries: 2`, default `max_concurrent_tasks: 4`, `gate_concurrency: 4` | In scope: active runner default concurrency/retry limits; preserve current values |
| `crates/roko-serve/src/routes/deployments.rs` | `interval_ms = 5_000`, `max_interval_ms = 60_000` | In scope if serve polling loops are included; add poll interval constants |

Likely out-of-scope candidates from the same grep:

- Channel/backlog sizes (`mpsc::channel(256)`, gate buffer clamp `7`, `32`, `256`,
  feedback backlog `32`) unless the implementation explicitly chooses to centralize runtime
  resource limits in this task.
- Query/page limits (`aggregator.rs` trace `1000`, `heartbeats.rs` default `50`) because they
  do not control workflow retries or agent iterations.
- Test-only literals and demo request payloads (`max_turns: 2`/`3` inside route tests/templates).
- Heuristic scoring constants in routing/daimon observations (`max_loc.unwrap_or(50)`,
  `failure_pressure` denominator `5.0`, PAD thresholds) unless the literal directly caps a loop.

### 2. Add missing constants to `crates/roko-core/src/defaults.rs`

For each in-scope literal, add a named constant. Naming convention (existing pattern):

```
DEFAULT_<SUBSYSTEM>_<WHAT>_<UNIT>
```

Illustrative examples of the style to use (these are not authoritative values for the current
codebase; use the inspected-candidate table below for actual names/values):

```rust
/// Maximum auto-fix iterations in the runner gate-dispatch loop.
///
/// When a gate fails and the runner has an auto-fix model configured, it retries
/// up to this many times before marking the task as failed.
pub const DEFAULT_RUNNER_AUTO_FIX_MAX_ATTEMPTS: u32 = 5;

/// Maximum merge-queue retry attempts before a merge is abandoned.
pub const DEFAULT_RUNNER_MERGE_MAX_RETRIES: u32 = 5;

/// Maximum concurrent tasks the runner DAG allows to be in-flight at once.
///
/// Separate from provider-level concurrency (`DEFAULT_PROVIDER_MAX_CONCURRENT`):
/// this is the plan-level task concurrency cap, not the per-provider HTTP cap.
pub const DEFAULT_RUNNER_MAX_CONCURRENT_TASKS: usize = 4;

/// Maximum poll iterations for agent-stream completion before timeout.
pub const DEFAULT_AGENT_STREAM_MAX_POLL: u32 = 300;
```

Only add constants for literals you actually found in the audit. Do not pre-emptively
add constants for values that do not exist as literals in the codebase.

Note: `DEFAULT_MAX_AUTO_FIX_ITERATIONS` and `DEFAULT_MAX_MERGE_RETRIES` already exist
in `defaults.rs` (lines 85 and 82). Check whether the literals in the runner code
already use these constants before adding duplicates.

Suggested names for the inspected candidates, preserving current values:

```rust
pub const DEFAULT_AGENT_TURN_LIMIT: u32 = 50;
pub const DEFAULT_RUNNER_MAX_CONCURRENT_PLANS: usize = 4;
pub const DEFAULT_RUNNER_MAX_CONCURRENT_TASKS: usize = 4;
pub const DEFAULT_RUNNER_GATE_CONCURRENCY: usize = DEFAULT_RUNNER_MAX_CONCURRENT_TASKS;
pub const DEFAULT_RUNNER_CONFIG_MAX_RETRIES: u32 = 2;
pub const DEFAULT_RUNNER_RETRY_STRATEGY_PIVOT_ATTEMPT: u32 = 3;
pub const DEFAULT_GATE_RETRY_MIN: u32 = 1;
pub const DEFAULT_GATE_RETRY_MAX: u32 = 5;
pub const DEFAULT_GATE_RETRY_COLD_START: u32 = 3;
pub const DEFAULT_GATE_RETRY_MIN_OBSERVATIONS: u64 = 5;
pub const DEFAULT_RUNNER_RETRY_BACKOFF_SHIFT_CAP: u32 = 5;
pub const DEFAULT_RUNNER_RETRY_BACKOFF_MULTIPLIER_FALLBACK: u64 = 32;
pub const DEFAULT_RUNNER_RETRY_BACKOFF_MAX_SECS: u64 = 45;
pub const DEFAULT_DEPLOYMENT_STATUS_POLL_INITIAL_MS: u64 = 5_000;
pub const DEFAULT_DEPLOYMENT_STATUS_POLL_MAX_MS: u64 = 60_000;
```

If you choose to centralize snapshot writer capacity/failure-streak thresholds, use names like
`DEFAULT_RUNNER_SNAPSHOT_QUEUE_CAPACITY` and `DEFAULT_RUNNER_SNAPSHOT_DEGRADED_AFTER_FAILURES`,
but document that choice in the Status Log because they are persistence resource limits rather
than workflow iteration limits.

### 3. Replace literals with constants

For each in-scope literal, replace the raw number with the named constant. Import
`roko_core::defaults::DEFAULT_*` at the top of the file.

The replacement must be semantically exact: if the literal is `5` and you introduce
`DEFAULT_RUNNER_AUTO_FIX_MAX_ATTEMPTS = 5`, the behavior is unchanged. Do not change
the value of any constant from its current hardcoded literal unless you have an
explicit justification.

### 4. Add ordering/sanity tests to `defaults.rs`

Add assertions to the existing `retry_backoff_ordering` test (or a new adjacent test)
for any constants whose values must satisfy invariants. Example:

```rust
#[test]
fn runner_limits_are_sane() {
    assert!(DEFAULT_RUNNER_MAX_CONCURRENT_TASKS >= 1);
    assert!(DEFAULT_RUNNER_AUTO_FIX_MAX_ATTEMPTS >= 1);
    assert!(DEFAULT_RUNNER_MERGE_MAX_RETRIES >= 1);
}
```

Only add assertions that capture real constraints (e.g. "must be positive", "A < B").

## What NOT to Do

- Do NOT touch `crates/roko-cli/src/orchestrate.rs`. It is behind `legacy-orchestrate`
  feature flag and is explicitly out of scope per the 2026-05-04 course correction.
- Do NOT consolidate literals that are already named constants imported from `defaults`.
  Verify with `grep -n 'DEFAULT_' <file>` before assuming a value is a raw literal.
- Do NOT collapse semantically different values just because they are numerically equal. For
  example, runner max-concurrent-tasks `4`, snapshot queue capacity `4`, and max-concurrent-plans
  `4` need separate names if all are centralized.
- Do NOT change any literal values — only name them. Behavioral changes require a
  separate task with a test plan.
- Do NOT add constants for values that appear only in test code. Test assertions for
  exact counts are not magic numbers; they are behavioral contracts.
- Do NOT rename existing constants. The audit batches 39-47 established names that are
  already referenced across crates. Adding aliases causes confusion.
- Do NOT add `roko-gate` or `roko-orchestrator` to the touches list. This task is
  scoped to the active runner stack, agent dispatch, and serve routes.

## Wire Target

No new CLI surface. Verify by inspection and test:

```bash
# Confirm no remaining raw iteration literals in runner paths
grep -rn '\b[0-9]\{1,3\}\b' crates/roko-cli/src/runner/ --include='*.rs' \
  | grep -E '(max_|limit|retry|attempt|iter|loop|cap)' \
  | grep -v 'DEFAULT_\|target/\|//.*#\|#\[test\]'

# All tests pass
cargo test -p roko-cli runner:: -- --nocapture
cargo test -p roko-core defaults -- --nocapture

# No new compiler warnings
cargo clippy -p roko-core -p roko-cli -p roko-agent -p roko-serve --no-deps -- -D warnings
```

## Verification

- [ ] `cargo build --workspace` — clean build
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo test -p roko-core defaults` — new sanity assertions pass
- [ ] `grep -rn 'DEFAULT_' crates/roko-cli/src/runner/ --include='*.rs' | grep -v target/` —
  all iteration limits in runner files now reference `DEFAULT_*` constants
- [ ] `grep -rn 'DEFAULT_' crates/roko-agent/src/dispatcher/ --include='*.rs' | grep -v target/` —
  same for agent dispatcher
- [ ] `diff` of `crates/roko-core/src/defaults.rs` shows only additions, no removals
  or value changes
- [ ] Status Log records the audit classification for every literal examined

## Status Log

| Time | Agent | Action |
|------|-------|--------|
