# Task 003: Wire TimeoutConfig — Replace Hardcoded Durations

```toml
id = 3
title = "Wire TimeoutConfig fields to replace hardcoded Duration::from_secs() calls"
track = "config-foundation"
wave = "wave-0"
priority = "high"
blocked_by = [1]
touches = [
    "crates/roko-core/src/config/timeouts.rs",
    "crates/roko-core/src/config/schema.rs",
    "crates/roko-cli/src/commands/plan.rs",
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/gate_dispatch.rs",
    "crates/roko-cli/src/runner/types.rs",
    "crates/roko-cli/src/serve_runtime.rs",
    "crates/roko-agent/src/dispatcher/mod.rs",
]
exclusive_files = ["crates/roko-core/src/config/timeouts.rs"]
estimated_minutes = 90
```

## Context

`TimeoutConfig` exists in the schema with 9 second fields and Duration accessors. It deserializes
correctly from `[timeouts]` in roko.toml.

Current branch note: some wiring already exists in `RunConfig::from_roko_config()` and
`runner/event_loop.rs` helper functions. Treat this task as "finish and verify runtime timeout
wiring"; do not assume the older audit statement "zero code reads config.timeouts" is still true.

Sources:
- `tmp/v2-refactoring/10-DEAD-CODE-AUDIT.md` — TimeoutConfig (WIRE NOW)
- `tmp/solutions/demo-running/CURRENT-STATE.md` — Hardcoded timeouts in 4+ places
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` — W15-C: TimeoutConfig dead code

## Background

Read these files first:
1. `crates/roko-core/src/config/timeouts.rs` — the TimeoutConfig struct
2. `crates/roko-core/src/config/schema.rs` — where TimeoutConfig is embedded in RokoConfig
3. Find hardcoded timeouts: `grep -rn 'Duration::from_secs' crates/ --include='*.rs' | grep -v target/ | grep -v test`

Source-doc correction: older task text cited `tmp/solutions/demo-running/BATCH-GAPS.md`, but that
file is now `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md`.

Current branch facts to verify before editing:
- `TimeoutConfig` fields are `agent_dispatch_secs`, `gate_compile_secs`, `gate_test_secs`,
  `gate_clippy_secs`, `llm_call_secs`, `http_request_secs`, `workspace_lock_secs`,
  `health_check_secs`, and `plan_total_secs`. There is no `plan_overall` field.
- `crates/roko-cli/src/runner/task_runner.rs` does not exist on this branch. The runner config
  type is in `crates/roko-cli/src/runner/types.rs`; gate execution is in
  `crates/roko-cli/src/runner/gate_dispatch.rs`.
- `RunConfig::from_roko_config()` already copies `roko_config.timeouts.agent_dispatch()` and
  `roko_config.timeouts.plan_total()` into legacy scalar fields for compatibility.
- `event_loop.rs` already has helper functions `agent_dispatch_timeout()`, `plan_total_timeout()`,
  `llm_call_timeout()`, and `gate_timeout()` that read `config.roko_config.as_deref().map(...)`.
- Remaining `Duration::from_secs()` hits are not automatically bugs. Examples that should remain:
  polling intervals, shutdown grace periods, retry backoff constants, tests, and domain-specific
  time windows (C-Factor, heartbeats, retention, rate limits).

## What to Change

1. **Trace the runtime call chain**:
   - `roko plan run` -> `commands/plan.rs::cmd_plan()` -> load `roko_config` ->
     construct `RunConfig` -> `runner::event_loop::run()`.
   - `serve_runtime.rs::build_run_config()` constructs the same `RunConfig` for serve/job paths.
   - `event_loop.rs` passes timeout seconds into `gate_dispatch::{spawn_gate, spawn_plan_verify}`.
2. **Make all runner operational timeouts derive from `TimeoutConfig`**:
   - Agent dispatch wall-clock: `agent_dispatch_timeout(config)` -> `cfg.timeouts.agent_dispatch()`.
   - Whole plan wall-clock: `plan_total_timeout(config)` -> `cfg.timeouts.plan_total()`.
   - LLM/model-call timeout: `llm_call_timeout(config)` -> `cfg.timeouts.llm_call()`.
   - Gate rung timeout: `gate_timeout(config, rung)` maps rung 0 -> `gate_compile()`, rung 1 ->
     `gate_clippy()`, rung >= 2 -> `gate_test()`.
   - HTTP health/probe paths that already have a loaded `RokoConfig` should use
     `cfg.timeouts.health_check()` or `cfg.timeouts.http_request()` instead of inline constants.
3. **Keep compatibility scalars but make them derived**:
   - `RunConfig.timeout_secs` and `RunConfig.plan_timeout_secs` may stay for older tests and
     fallback config paths, but any `RunConfig` built from `RokoConfig` must populate them from
     `roko_config.timeouts`.
   - Do not add a second timeout config struct in CLI.
4. **Audit `Duration::from_secs()` hits with intent**:
   - Replace hardcoded values only when they are operational runner/gate/dispatch timeouts that
     match a `TimeoutConfig` field.
   - Leave tests, retry delays, UI flush intervals, shutdown grace, rate limit windows, retention,
     domain math, and scaffolding examples unless they are explicitly part of runner/gate timeout
     behavior.
5. **Tests to add/update**:
   - Keep/update `runner::types::tests::run_config_uses_timeout_config_from_roko_toml`.
   - Add focused tests for `event_loop` timeout helper selection if helpers are made test-visible,
     especially rung 0/1/2 mapping.
   - Add a CLI/runner setup test proving `[timeouts] plan_total_secs` and
     `agent_dispatch_secs` reach `RunConfig` from `commands/plan.rs` construction.

## What NOT to Do

- Don't change the TimeoutConfig struct itself (it's already correct).
- Don't add new timeout fields.
- Don't change timeout VALUES — only the source (hardcoded → config).
- Don't try to eliminate every `Duration::from_secs()` in the repo. Many are not configurable
  operational timeouts and are outside this task.
- Don't edit `crates/roko-agent/src/dispatcher/mod.rs` test-only `Duration::from_secs(5)` calls
  just to satisfy grep.
- Don't reintroduce task-specific timeout literals in `commands/plan.rs` or `serve_runtime.rs`;
  derive them from `RokoConfig` once at `RunConfig` construction.

## Wire Target

Use the current runner path and the existing `ROKO_CONFIG` override:
```bash
tmpdir=$(mktemp -d)
cat > "$tmpdir/roko.toml" <<'TOML'
config_version = 2
[timeouts]
agent_dispatch_secs = 1
plan_total_secs = 2
gate_compile_secs = 1
gate_clippy_secs = 1
gate_test_secs = 1
TOML
cargo test -p roko-cli run_config_uses_timeout_config_from_roko_toml -- --nocapture
ROKO_CONFIG="$tmpdir/roko.toml" cargo run -p roko-cli -- plan run plans/ --dry-run
```

Expected observable behavior: run setup/logging reports timeout values derived from `[timeouts]`
when a real plan is run; dry-run remains useful as a non-dispatch smoke check.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo test -p roko-cli run_config_uses_timeout_config_from_roko_toml`
- [ ] `grep -rn 'Duration::from_secs' crates/roko-cli/src/runner crates/roko-agent/src/dispatcher --include='*.rs' | grep -v target/ | grep -v test` — remaining hits are documented non-config timeouts
- [ ] `grep -rn 'config.timeouts\|\.timeouts\.' crates/ --include='*.rs' | grep -v target/` — shows multiple callers
- [ ] `test ! -e crates/roko-cli/src/runner/task_runner.rs` — the implementation plan must use
  `runner/types.rs`, `runner/event_loop.rs`, and `runner/gate_dispatch.rs` instead

## Status Log

| Time | Agent | Action |
|------|-------|--------|
