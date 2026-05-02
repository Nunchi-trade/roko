# Plan: dry-run-flag

**PRD**: `.roko/prd/published/dry-run-flag.md`
**Goal**: Add `--dry-run` to `roko run` that resolves config/model/gates/phases and prints a structured preview without dispatching to any LLM or running any gate.

## File map

| File | Change |
|------|--------|
| `crates/roko-cli/src/main.rs` | Add `dry_run: bool` field to `Command::Run`; thread into `cmd_run` call |
| `crates/roko-runtime/src/workflow_engine.rs` | Add `dry_run: bool` to `WorkflowRunConfig`; fix all struct literal sites in tests; add early-return guard in `run_with_cancel` |
| `crates/roko-cli/src/run.rs` | Add `DryRunPreview`, `DryRunGate`, `workflow_config_phases()`, `build_dry_run_preview()`, `print_dry_run_preview()` |
| `crates/roko-cli/src/commands/util.rs` | Add `dry_run` param to `cmd_run`; add early-exit V2 dry-run branch |
| `crates/roko-serve/src/routes/shared_runs.rs` | Add `dry_run: false` to `WorkflowRunConfig { .. }` literal |
| `crates/roko-acp/src/runner.rs` | Add `dry_run: false` to `WorkflowRunConfig { .. }` literal |

## Dependency graph

```
T1 ──────────────────────────────┐
T2 ───────────────────────┬──────┤
T3 ────────────────────── ┼ ─────┤→ T4 ─→ T6
                          └──────┘
                          T5 ─────────────→ T6
```

T1, T2, T3 can run in parallel. T4 and T5 depend on T2. T6 depends on T3+T4+T5.

## Exit-code contract

| Condition | Exit |
|-----------|------|
| Resolution OK | 0 |
| Fatal config error (bad model, missing key, bad TOML) | 2 |
| `--dry-run` MUST NOT | exit 1 |

## DryRunPreview phases by template

| Template | Phases |
|----------|--------|
| express | `["implement", "gate", "commit"]` |
| standard | `["implement", "gate", "review", "commit"]` |
| full | `["strategy", "implement", "gate", "review", "commit"]` |
