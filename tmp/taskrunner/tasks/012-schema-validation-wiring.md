# Task 012: Wire validate_against_schema() into Plan Loading

```toml
id = 12
title = "Wire validate_against_schema() so plan TOML is validated before execution"
track = "runner-hardening"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/task_parser.rs",
    "crates/roko-cli/src/runner/plan_loader.rs",
    "crates/roko-cli/src/plan_validate.rs",
]
exclusive_files = ["crates/roko-cli/src/runner/plan_loader.rs"]
estimated_minutes = 30
```

## Context

`validate_against_schema()` exists on TasksFile in `task_parser.rs:830` but is never called
from `plan_loader.rs`, `validate.rs`, or any runtime path. Schema violations are silently
ignored, causing confusing runtime failures.

Current code inspection shows `plan_loader.rs` and `plan_validate.rs` may already call this method.
Treat this task as "verify both runtime paths and fill missing pieces", not as a mandate to add
duplicate validation.

Sources:
- `tmp/solutions/demo-running/CURRENT-STATE.md` — validate_against_schema dead code

## Background

Read these files first:
1. `crates/roko-cli/src/task_parser.rs` — find `validate_against_schema()` method
2. `crates/roko-cli/src/runner/plan_loader.rs` — where plans are loaded

## What to Change

1. **Call `validate_against_schema()` in `plan_loader.rs`** after parsing the TOML.
2. **If validation fails**, return a clear error with the specific validation failures.
3. **Also wire it into `roko plan validate`** command if not already there.

## What NOT to Do

- Don't change the validation logic itself.
- Don't add new schema rules.
- Don't make validation optional (it should always run).
- Don't add a second validator in parallel with `TasksFile::validate_against_schema()`.

## Implementation Notes

Current code inspection shows this wiring may already be present. Verify before editing.

Runtime call chains that must both surface schema errors:
- `roko plan run <dir>`:
  `crates/roko-cli/src/commands/plan.rs` `PlanCmd::Run` →
  `validate_before_run()` →
  `plan_validate::validate_plans_dir_with_workdir()` →
  `plan_validate.rs::validate_tasks_file()` →
  `TasksFile::parse_str()` →
  `TasksFile::validate_against_schema()`;
  then the execution path calls `runner::plan_loader::load_plans()` →
  `load_plan()` →
  `TasksFile::parse()` →
  `TasksFile::validate_against_schema()`.
- `roko plan validate <dir>`:
  `PlanCmd::Validate` →
  `cmd_plan_validate()` →
  `plan_validate::validate_plans_dir_with_workdir()` →
  `validate_tasks_file()` →
  `TasksFile::validate_against_schema()`.

Files/functions to read before editing:
- `crates/roko-cli/src/task_parser.rs`: `TasksFile::parse()`, `TasksFile::parse_str()`,
  `TasksFile::validate_against_schema()`.
- `crates/roko-cli/src/runner/plan_loader.rs`: `load_plan()`, `load_plans()`, and the
  `load_plan_rejects_schema_issues` test.
- `crates/roko-cli/src/plan_validate.rs`: `validate_tasks_file()` and rule IDs around runtime parse
  and schema diagnostics (`PLAN_034`, `PLAN_035`).
- `crates/roko-cli/src/commands/plan.rs`: `validate_before_run()` and `cmd_plan_validate()`.

Mechanical steps:
1. If `plan_loader::load_plan()` does not call `validate_against_schema()` immediately after
   parsing `tasks.toml`, add that call and `bail!` with one bullet per issue:
   `schema validation failed for <path>:\n  - <issue>`.
2. If `plan_validate.rs::validate_tasks_file()` does not call `validate_against_schema()` after
   `TasksFile::parse_str()`, add diagnostics with severity `Error`, rule ID `PLAN_035`, the plan ID,
   and message `schema validation failed: <issue>`.
3. Keep TOML parse errors distinct from schema errors (`PLAN_034` for runtime parse failure,
   `PLAN_035` for schema failure).
4. Do not validate by re-reading the file in `commands/plan.rs`; keep validation centralized in
   `plan_loader.rs` and `plan_validate.rs`.

Tests to add/update:
- `plan_loader.rs`: invalid-but-parseable `tasks.toml` fails `load_plan()` with
  `schema validation failed`, `missing 'verify'`, and `missing 'files'`.
- `plan_validate.rs`: invalid-but-parseable `tasks.toml` yields `PLAN_035` errors and exit code `1`.
- `commands/plan.rs` integration/unit coverage if present: `validate_before_run()` blocks
  `plan run` before agents spawn.

## Wire Target

```bash
# Create an invalid tasks.toml and try to load it
rm -rf /tmp/bad-plan
mkdir -p /tmp/bad-plan
cat > /tmp/bad-plan/tasks.toml <<'TOML'
[meta]
plan = "bad-plan"

[[task]]
id = "T1"
title = ""
role = "implementer"
TOML
cargo run -p roko-cli -- plan validate /tmp/bad-plan/
# Should show validation errors
```

**Expected behavior**: command exits non-zero and reports `PLAN_035` / `schema validation failed`
with missing title plus missing implementer `verify` and `files`.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `grep -rn 'validate_against_schema' crates/roko-cli/ --include='*.rs' | grep -v target/` — shows callers in plan_loader and/or validate
- [ ] `cargo run -p roko-cli -- plan validate /tmp/bad-plan/` exits non-zero for the wire target above
- [ ] `cargo run -p roko-cli -- plan run /tmp/bad-plan/` exits before spawning agents and prints schema validation errors

## Status Log

| Time | Agent | Action |
|------|-------|--------|
