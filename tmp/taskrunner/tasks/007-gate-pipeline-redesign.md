# Task 007: Redesign Gate Pipeline — TOML-Configurable Shell Commands

```toml
id = 7
title = "Redesign gate pipeline: TOML-configurable shell commands replacing hardcoded Rust dispatch"
track = "runner-hardening"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-core/src/config/gates.rs",
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/gate_dispatch.rs",
]
exclusive_files = ["crates/roko-core/src/config/gates.rs", "crates/roko-cli/src/runner/gate_dispatch.rs"]
estimated_minutes = 180
```

## Context

The current gate pipeline has an architectural mismatch:
- `GateRungConfig` in `gates.rs` defines gates as **shell commands** (`command: String`)
- `run_rung()` in `roko-gate` dispatches to **Rust-native** gate implementations
- `event_loop.rs` hardcodes rung numbers (0=compile, 1=clippy, 2=test) with magic-number skip logic

**Decision: Gates become TOML-configurable shell commands.** Built-in gates (compile, test, clippy)
become their shell command equivalents. Users can add custom gates.

This is a redesign, not a wiring task. The existing `GateRungConfig` shell command approach wins.

Sources:
- `tmp/v2-refactoring/10-DEAD-CODE-AUDIT.md` — GateRungConfig (WIRE NOW)
- Audit finding: `event_loop.rs` has hardcoded rung logic, `gate_dispatch.rs` passes through to `run_rung()`

## Background

Read these files:
1. `crates/roko-core/src/config/gates.rs` — GateRungConfig with `command`, `timeout_secs`, `required`, `parallel_with`
2. `crates/roko-cli/src/runner/event_loop.rs` — find the `RunGate` handler, rung sequence logic
3. `crates/roko-cli/src/runner/gate_dispatch.rs` — current dispatch logic
4. `crates/roko-gate/src/lib.rs` — Rust-native gate implementations (these become shell commands)

## What to Change

1. **Wire `effective_rungs()` into the gate pipeline startup** in `event_loop.rs`.
   When gate is triggered, iterate over `effective_rungs()` instead of hardcoded rung numbers.
2. **Replace `run_rung()` dispatch with shell command execution**:
   - Each `GateRungConfig` has a `command: String` field
   - Execute it via `tokio::process::Command`
   - Parse exit code: 0 = pass, non-zero = fail
   - Capture stdout/stderr for gate report
3. **Convert built-in gates to shell commands** in the Default impl of `GateRungConfig`:
   ```toml
   [[gates.rungs]]
   name = "compile"
   command = "cargo build --workspace"
   timeout_secs = 300
   required = true

   [[gates.rungs]]
   name = "lint"
   command = "cargo clippy --workspace --no-deps -- -D warnings"
   timeout_secs = 120
   required = true

   [[gates.rungs]]
   name = "test"
   command = "cargo test --workspace"
   timeout_secs = 600
   required = true
   ```
4. **Support `parallel_with`** from GateRungConfig — allow gates to run concurrently.
5. **Remove hardcoded rung numbers** from `event_loop.rs` (no more `rung == 1`, `rung <= 2`).
6. **Keep `run_rung()` as a fallback** behind a feature flag if needed for backwards compat,
   but the default path must use shell commands.

## What NOT to Do

- Don't delete roko-gate crate (other code may still reference it).
- Don't change the gate RESULT format (pass/fail + report).
- Don't add complex gate scripting (commands are simple shell strings).
- Don't preserve a hidden Rust-native default path. The default user-visible path must be shell
  commands from `GatesConfig::effective_rungs()`.

## Implementation Notes

Current runtime call chain:
`roko-cli/src/commands/plan.rs` `PlanCmd::Run` →
`runner::plan_loader::load_plans()` →
`runner::event_loop::run()` →
`handle_executor_action()` →
`ExecutorAction::RunGate` →
`gate_dispatch::spawn_gate()` →
`roko_gate::rung_dispatch::GatePipelineBuilder`.

Files/functions to read before editing:
- `crates/roko-core/src/config/gates.rs`: `GateRungConfig`,
  `GatesConfig::effective_rungs()`, `GatesConfig::has_custom_rungs()`.
- `crates/roko-cli/src/runner/types.rs`: `RunConfig::from_roko_config()` still derives
  `max_gate_rung` from `skip_tests` / `clippy_enabled`; do not add new consumers of it.
- `crates/roko-cli/src/runner/event_loop.rs`: `gate_timeout()`, `gates_config_for_run()`,
  `record_skipped_gate_rung()`, the gate-completion branch, and `ExecutorAction::RunGate`.
- `crates/roko-cli/src/runner/gate_dispatch.rs`: `spawn_gate()`, `gate_signal()`,
  `run_verify_steps()`, `render_output()`, `classify_failure_kind()`.
- Read-only reference: `crates/roko-gate/src/shell.rs` (`ShellGate`) and
  `crates/roko-gate/src/rung_dispatch.rs`; do not depend on its canonical Rust rung mapping for
  the new default path.

Mechanical steps:
1. In `GatesConfig::effective_rungs()`, make built-in defaults actual shell-command
   `GateRungConfig`s and respect `clippy_enabled` / `skip_tests`.
   - `compile`: `cargo build --workspace`, timeout `300`, required.
   - `lint`: `cargo clippy --workspace --no-deps -- -D warnings`, timeout `120`, required,
     omitted when `clippy_enabled = false`.
   - `test`: `cargo test --workspace`, timeout `600`, required, omitted when `skip_tests = true`.
   - Custom `[[gates.rungs]]` must still replace defaults exactly.
2. In `gate_dispatch.rs`, add a helper that runs `Vec<GateRungConfig>` as shell gates using
   `ShellGate::new("sh", vec!["-c", command])` on Unix and `cmd /C` on Windows. Keep using
   `gate_signal()` so `GatePayload::working_dir`, `ROKO_GATE_*`, and `CARGO_TARGET_DIR` behavior
   remains available.
3. Implement `parallel_with` as a simple undirected grouping relation between rung names. Execute
   groups in declaration order; run rungs within one group concurrently and aggregate all verdicts.
4. Preserve `required = false`: failed optional rungs should become passing verdicts whose detail
   names the original failure. Required rung failures must fail the gate completion.
5. Append task-level `verify` step verdicts after configured gate-rung verdicts. Do not fold
   per-task `verify` commands into `[[gates.rungs]]`.
6. In `event_loop.rs`, treat `ExecutorAction::RunGate` as "run the configured gate pipeline once".
   Remove magic-number advancement such as `completion.rung < config.max_gate_rung`; the spawned
   pipeline already contains all configured rungs.
7. Replace `gate_timeout(config, rung)` magic-number matching with a helper that does not branch on
   literal rung numbers. For the configured pipeline, prefer configured rung timeouts; keep timeout
   config only as a fallback for plan verify and merge paths that still pass sentinel rungs.
8. Preserve `GateCompletion`, `GateVerdictSummary`, TUI events, runner events, daimon recording, and
   retry behavior. The audit surface should see the same `gate.completed` event shape, with verdict
   names matching configured rung names.

Tests to add/update:
- `roko-core` unit tests in `gates.rs`: default command list, `skip_tests`, `clippy_enabled`, and
  custom replacement preserving `parallel_with`.
- `roko-cli` unit tests in `gate_dispatch.rs`: echo pass with captured output, required `false`
  failure, optional `false` pass-with-detail, and a `parallel_with` pair producing both verdicts.
- Update event-loop tests that assumed multiple numeric rung advancements.

## Wire Target

```bash
tmpdir="$(mktemp -d)"
cat > "$tmpdir/roko.toml" <<'TOML'
schema_version = 2
[project]
name = "gate-wire-smoke"
[agent]
default_model = "mock-model"
command = "cat"
[[gates.rungs]]
name = "custom-check"
command = "echo custom gate passed"
timeout_secs = 30
required = true
TOML
mkdir -p "$tmpdir/plans/demo"
touch "$tmpdir/README.md"
cat > "$tmpdir/plans/demo/tasks.toml" <<'TOML'
[meta]
plan = "demo"
[[task]]
id = "T1"
title = "Mock implementation"
role = "implementer"
files = ["README.md"]
verify = [{ phase = "smoke", command = "echo task verify" }]
TOML
ROKO_DISPATCHER=mock-self-host-fixture \
ROKO_MOCK_STATE_PATH="$tmpdir/mock-state.txt" \
cargo run -p roko-cli -- --workdir "$tmpdir" plan run "$tmpdir/plans"
```

**Expected behavior**: the run reaches `gate.completed`; output or logs include
`custom-check` and `custom gate passed`; no numeric compile/clippy/test rungs run unless they were
declared in `[[gates.rungs]]`.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'rung == \|rung <= \|match rung' crates/roko-cli/src/runner/ --include='*.rs'` — no default gate selection by numeric rung
- [ ] `grep -rn 'effective_rungs' crates/roko-cli/ --include='*.rs' | grep -v target/` — shows a non-test caller
- [ ] Default gate config produces compile, lint, test shell commands in order
- [ ] Custom gates from roko.toml are executed
- [ ] Optional custom gate failures do not fail the run; required custom gate failures do fail

## Status Log

| Time | Agent | Action |
|------|-------|--------|
