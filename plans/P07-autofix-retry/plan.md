# P07 — Auto-fix Retry in `dispatch_agent`

## Goal

When per-task verify steps fail inside `dispatch_agent`, instead of immediately
returning an `Err`, attempt up to **2 auto-fix cycles**:

1. Extract the combined stdout+stderr from the failed verify command.
2. Build a focused "fix prompt" that includes the original task context plus the
   error output.
3. Spawn a second agent call (model selection: haiku for compile/structural
   errors, sonnet for test failures).
4. Re-run the full verify pipeline. If it passes, return success. After 2
   failed attempts, return the original error.

## Scope

| File | Change |
|------|--------|
| `crates/roko-cli/src/task_parser.rs` | Add `TaskDef::build_fix_prompt(error, phase)` method |
| `crates/roko-cli/src/orchestrate.rs` | Replace inline verify loop with `run_verify_with_autofix` helper |

## Architecture

```
dispatch_agent()
  └─ run agent (existing)
  └─ run_verify_with_autofix(td, exec_dir, original_prompt, model)
       ├─ attempt 0: run all verify steps
       │     if pass → Ok(())
       │     if fail → collect error text
       │              select fix_model (haiku/sonnet)
       │              build fix_prompt via TaskDef::build_fix_prompt
       │              spawn agent with fix_prompt
       ├─ attempt 1: re-run verify steps
       │     if pass → Ok(())
       │     if fail → collect error text, repeat
       └─ attempt 2: return Err (exhausted)
```

## Constraints

- **Max 2 fix attempts** (constant `MAX_AUTOFIX_ATTEMPTS = 2`).
- Model selection rule:
  - phase == "compile" OR phase == "structural" → `claude-haiku-4-5`
  - phase == "test" OR anything else → `claude-sonnet-4-6`
- The fix prompt must include: original task title, failing command, captured
  output (truncated to 4 000 chars), and the original verification list.
- Do NOT modify the outer retry/escalation loop (lines 475–540 of
  `orchestrate.rs`). Only the verify pipeline block (lines 817–852) changes.

## Task Dependency Graph

```
T1 (add build_fix_prompt to task_parser.rs)
  └─ T2 (extract run_verify_with_autofix helper — orchestrate.rs)
       └─ T3 (wire autofix loop into dispatch_agent — orchestrate.rs)
            └─ T4 (unit tests for build_fix_prompt — task_parser.rs)
                 └─ T5 (integration smoke-test assertion — orchestrate.rs tests)
```
