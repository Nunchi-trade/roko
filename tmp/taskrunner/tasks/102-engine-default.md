# Task 102: Engine as Default + Feature-Gate Runner v2

```toml
id = 102
title = "Make Graph Engine the default for roko plan run and feature-gate Runner v2 behind legacy-runner-v2 Cargo feature"
track = "graph-engine"
wave = "wave-5"
priority = "high"
blocked_by = [101]
touches = [
    "crates/roko-cli/src/runner/mod.rs",
    "crates/roko-cli/src/main.rs",
    "crates/roko-cli/Cargo.toml",
]
exclusive_files = []
estimated_minutes = 240
```

## Context

Task 101 adds `--engine graph` as an opt-in. This task flips the default: the Graph Engine
becomes the execution path for `roko plan run`, and Runner v2 is feature-gated behind
`legacy-runner-v2`. This mirrors exactly how `orchestrate.rs` was feature-gated before
Runner v2 replaced it.

Checklist items covered: **P4-4** (make Engine the default for `roko plan run`) and **P4-5**
(feature-gate Runner v2 behind `legacy-runner-v2` Cargo feature).

This is a **migration-complete** task. After it lands, users get the Graph Engine by default.
Runner v2 remains available via `--engine runner-v2` or by compiling with `legacy-runner-v2`.
All existing `plan run` tests must pass through the Engine path — if any test fails, the
Engine is not ready to be the default, and the fix belongs in task 101.

**Before starting this task**, confirm that all integration tests from task 101 pass and that
the Engine path handles every real plan in the `plans/` directory without error.

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/CHECKLIST.md` — items P4-4 and P4-5, including the done definition:
   "Runner v2 is feature-gated, Engine is default"
2. `crates/roko-cli/src/main.rs` — how `orchestrate.rs` was previously feature-gated.
   Search for `#[cfg(feature = "legacy-orchestrate")]` to find the pattern. Follow it exactly
   for Runner v2.
3. `crates/roko-cli/src/runner/mod.rs` — Runner v2 entry point. This goes behind the feature
   gate. Do NOT delete it — it must remain compilable when the feature is enabled.
4. `crates/roko-cli/src/main.rs` — `PlanCmd::Run` variant and `PlanEngine` enum from task 101.
   The default value switches from `RunnerV2` to `Graph` here.
5. `crates/roko-cli/Cargo.toml` — where features are declared. Add `legacy-runner-v2` here.
6. `crates/roko-graph/tests/plan_conversion.rs` — integration tests from task 101. These
   must all pass through the Engine path before this task is considered done.

## Implementation Detail

### Current source facts to verify first

- `crates/roko-cli/Cargo.toml` currently has `default = []`,
  `legacy-orchestrate = ["legacy-direct-dispatch"]`, and `legacy-direct-dispatch = []`.
  Use this exact feature style for `legacy-runner-v2`.
- The previous feature-gate pattern is in `crates/roko-cli/src/lib.rs`:
  `#[cfg(feature = "legacy-orchestrate")] pub mod orchestrate;` and the matching gated
  `pub use orchestrate::{...};`.
- `runner` is currently declared unconditionally in `crates/roko-cli/src/lib.rs` as
  `pub mod runner;`.
- `PlanCmd::Run` currently has no `engine` field; the construction site in
  `main.rs::dispatch_subcommand` for `Command::Resume` also manually builds
  `PlanCmd::Run` and must be updated with `engine: PlanEngine::Graph`.
- Runner v2 is not isolated to `roko plan run`. Current non-test call sites include:
  `commands/do_cmd.rs`, `prd.rs`, `serve_runtime.rs`, `worker/cloud.rs`,
  `dispatch/mod.rs`, and `projection/*`. A wholesale `#[cfg(feature = "legacy-runner-v2")]`
  on `pub mod runner` will not compile unless those shared call sites have already been
  moved to the Graph Engine or to non-legacy modules.

### Required scope decision

Before editing, run:

```bash
rg -n 'roko_cli::runner|crate::runner|runner::|use .*runner|pub mod runner' crates/roko-cli/src --glob '*.rs'
```

Then choose one of these paths and record it in the Status Log:

1. **Preferred for this task if non-plan call sites still exist**: gate only the `roko plan run
   --engine runner-v2` CLI variant and branch. Leave `pub mod runner` compiled because other
   commands still depend on shared runner modules. This satisfies "Engine is default for
   `roko plan run`" without changing `roko do`, `roko serve`, PRD execution, or worker code.
2. **Only if prior work removed all non-plan runner use**: gate `pub mod runner` and every
   remaining runner import behind `legacy-runner-v2`.

Do not attempt path 2 within the current touch list if the grep above still shows non-plan
call sites. That would require a separate extraction/migration task for shared modules such
as `runner::projection`, `runner::types`, `runner::agent_stream`, and `runner::plan_loader`.

### Exact plan-run call chain

The default switch happens in:

```text
main.rs::dispatch_subcommand
  -> Command::Plan { cmd }
  -> commands/plan.rs::cmd_plan()
  -> PlanCmd::Run { engine, ... }
  -> match engine
     -> PlanEngine::Graph => cmd_plan_run_engine(...)
     -> PlanEngine::RunnerV2 => existing runner setup/event_loop path (feature-gated)
```

Keep `--dry-run` behavior engine-independent: it should still parse and display plans without
starting either engine. Keep validation before the engine branch.

### CLI behavior to lock down

- Without `legacy-runner-v2`, `roko plan run --help` must list only `graph` as a possible
  `--engine` value.
- Without `legacy-runner-v2`, `roko plan run plans/ --engine runner-v2` must fail during clap
  parsing with an invalid value error.
- With `legacy-runner-v2`, help must list both `graph` and `runner-v2`; default remains `graph`.
- `PlanEngine` should derive `PartialEq, Eq` so parser tests can assert exact variants.

## What to Change

### 1. Declare the `legacy-runner-v2` feature in `crates/roko-cli/Cargo.toml`

```toml
[features]
default = []
legacy-runner-v2 = []
# ... existing features if any ...
```

The feature has no dependencies — it is purely a compilation gate for the runner v2 code
paths and the `PlanEngine::RunnerV2` CLI variant.

### 2. Feature-gate the Runner v2 dispatch path

Do this only after applying the "Required scope decision" above. In the current checkout,
the conservative implementation is to gate the Runner v2 **plan-run dispatch branch** and
`PlanEngine::RunnerV2` variant, while leaving `pub mod runner` compiled for non-plan callers.
If the grep proves no non-plan callers remain, use the module gate below.

In `crates/roko-cli/src/runner/mod.rs`, add a crate-level attribute at the top:

```rust
#![cfg(feature = "legacy-runner-v2")]
```

**Alternative if the module is re-exported**: wrap the `pub mod runner;` declaration in
`crates/roko-cli/src/lib.rs` (or wherever it is declared) with:

```rust
#[cfg(feature = "legacy-runner-v2")]
pub mod runner;
```

Check how the module is declared before choosing the approach. Prefer wrapping the `mod`
declaration so the entire module is elided when the feature is absent.

**Critical**: The `runner` module is used in many places (plan_loader, event_loop, etc.).
Every use site must also be conditionally compiled. Use `grep -rn 'use.*runner\|runner::'
crates/roko-cli/src/ --include='*.rs' | grep -v target/` to find all call sites.

For each call site, wrap with `#[cfg(feature = "legacy-runner-v2")]` or move the code to
the Engine path. The goal: `cargo build -p roko-cli` without the feature must compile clean.

If the call site belongs to `roko do`, `roko serve`, PRD execution, cloud worker execution,
or projection rendering, do not gate it in this task unless an Engine replacement already
exists and is wired. Those are outside the stated behavior change for task 102.

### 3. Change the default engine to `Graph` in `main.rs`

In the `PlanEngine` enum (added by task 101), change the default:

```rust
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum PlanEngine {
    /// Graph Engine (default).
    #[default]
    #[value(name = "graph")]
    Graph,
    /// Runner v2 (legacy; requires legacy-runner-v2 feature).
    #[cfg(feature = "legacy-runner-v2")]
    #[value(name = "runner-v2")]
    RunnerV2,
}
```

The `PlanEngine::RunnerV2` variant is only available when the feature is enabled.

Update the `--engine` flag's `default_value` and `help` text:

```rust
/// Engine to use for plan execution.
#[arg(long, default_value = "graph", value_enum)]
engine: PlanEngine,
```

### 4. Update `cmd_plan()` dispatch in `commands/plan.rs`

Wrap the `PlanEngine::RunnerV2` branch with the feature gate:

```rust
PlanCmd::Run { engine, plans_dir, .. } => {
    match engine {
        PlanEngine::Graph => {
            cmd_plan_run_engine(/* args */).await?
        }
        #[cfg(feature = "legacy-runner-v2")]
        PlanEngine::RunnerV2 => {
            // existing runner::run() call
        }
    }
}
```

If `PlanEngine::RunnerV2` is not available as a variant (feature absent), this match is
exhaustive on `PlanEngine::Graph` only.

### 5. Update CLI tests in `main.rs`

The existing tests that parse `--engine runner-v2` must be gated:

```rust
#[cfg(feature = "legacy-runner-v2")]
#[test]
fn cli_parses_plan_engine_runner_v2() {
    let cli = Cli::try_parse_from(["roko", "plan", "run", "plans", "--engine", "runner-v2"])
        .unwrap();
    // assert engine == RunnerV2
}
```

Add a new test that confirms `graph` is the default:

```rust
#[test]
fn cli_plan_run_defaults_to_graph_engine() {
    let cli = Cli::try_parse_from(["roko", "plan", "run", "plans"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Plan {
            cmd: PlanCmd::Run {
                engine: PlanEngine::Graph,
                ..
            }
        })
    ));
}
```

### 6. Update `roko plan run` help text

The `after_help` string in the `PlanCmd::Run` variant currently shows `runner-v2` examples.
Update the examples to show the Graph Engine as default:

```
Examples:
  roko plan run plans/              Run all plans (Graph Engine, default)
  roko plan run plans/my-plan       Run a specific plan
  roko plan run plans/ --approval   Run with interactive TUI approval
  roko plan run plans/ --dry-run    Preview without executing
  roko plan run plans/ --fresh      Archive old state and start clean
  roko plan run plans/ --resume-plan .roko/state/executor.json   Resume from snapshot
```

Remove `--engine runner-v2` from the after_help examples (it is legacy; discourage use).

### 7. Document the migration in `.roko/GAPS.md`

After completing this task, append to `.roko/GAPS.md`:

```
## Task 102: Engine as Default

- Runner v2 is feature-gated behind `legacy-runner-v2` in roko-cli.
- Cross-plan dependencies (depends_on_plan) are not yet handled by the Engine path.
  Runner v2 handled these at the PlanRunner level; the Engine needs a multi-graph
  orchestrator for this. Tracked as a gap for Phase 4 completion.
- FlowSnapshot/resume is not yet implemented for the Engine path. Users who relied on
  `--resume-plan` will get an error message when using the Engine path. The flag is
  still accepted but does nothing in the Engine path (with a warning).
- Gate failure replan is not yet implemented in TaskExecutorCell. The Engine runs each
  task once; retry/replan logic from Runner v2 has not been ported.
```

## What NOT to Do

- Do NOT delete Runner v2 code. It must remain compilable and correct when `legacy-runner-v2`
  feature is enabled. Someone may need to `cargo build --features legacy-runner-v2` to bisect
  a regression.
- Do NOT change the Engine implementation or the converter from task 101. All fixes to Engine
  behavior belong in task 101's scope; this task is only about switching the default.
- Do NOT silently ignore `--resume-plan` when the Engine is used. Print a clear warning:
  "Note: --resume-plan is not yet supported by the Graph Engine; snapshots will be ignored."
- Do NOT remove the `--engine runner-v2` flag entirely — users with the feature enabled
  need it. Gate it, don't delete it.
- Do NOT change the behavior of `roko agent start`, `roko serve`, or any command other than
  `roko plan run`. This task is scoped to the plan runner default switch only.
- Do NOT add any new functionality to the Engine in this task. If a test fails because the
  Engine cannot handle a specific plan structure, fix the converter in task 101 first, then
  come back here.

## Wire Target

```bash
# Build without legacy feature — confirms Runner v2 is fully gated
cargo build -p roko-cli
# Must compile clean with zero references to runner::run() unless feature-gated.

# Confirm graph is now the default
cargo run -p roko-cli -- plan run --help | head -5
# Expected: shows "graph" as default value for --engine

# Confirm default behavior changed
cargo run -p roko-cli -- plan run plans/
# Expected: "Running plan '...' via Graph Engine..." (not Runner v2 output)

# Confirm runner-v2 still works when compiled with feature
cargo build -p roko-cli --features legacy-runner-v2
cargo run -p roko-cli --features legacy-runner-v2 -- plan run plans/ --engine runner-v2
# Expected: original Runner v2 output
```

## Verification

- [ ] `cargo build -p roko-cli` (without `legacy-runner-v2` feature) — compiles clean
- [ ] `cargo build -p roko-cli --features legacy-runner-v2` — also compiles clean
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo run -p roko-cli -- plan run --help` — `--engine` shows `graph` as default
- [ ] `cargo run -p roko-cli -- plan run plans/` — uses Graph Engine by default (no `--engine` flag needed)
- [ ] `cargo run -p roko-cli --features legacy-runner-v2 -- plan run plans/ --engine runner-v2` — uses Runner v2
- [ ] `cargo test -p roko-cli -- cli_plan_run_defaults_to_graph_engine` — new test passes
- [ ] All integration tests from task 101 (`plan_conversion`) pass without `legacy-runner-v2` feature
- [ ] If only the plan-run branch is gated: `rg -n 'PlanEngine::RunnerV2|value\\(name = "runner-v2"\\)' crates/roko-cli/src --glob '*.rs'` shows every hit guarded by `#[cfg(feature = "legacy-runner-v2")]`
- [ ] If the whole runner module is gated: `rg -n 'roko_cli::runner|crate::runner|runner::|use .*runner' crates/roko-cli/src --glob '*.rs'` shows no unguarded hits outside `#[cfg(feature = "legacy-runner-v2")]`
- [ ] `.roko/GAPS.md` updated with task 102 gap entries
- [ ] The `PlanEngine::RunnerV2` variant is absent from `cargo run -p roko-cli -- plan run --help` output (when built without feature)
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any changed file

## Status Log

| Time | Agent | Action |
|------|-------|--------|
