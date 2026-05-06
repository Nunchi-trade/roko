# For AI Agents: How to Use This Task System

## The #1 Rule: Wire It or Don't Build It

This codebase has ~15K LOC of dead code — sophisticated implementations that were never
connected to any runtime path. **Your job is NOT to add more.** Every line you write must be
exercised by a CLI command or integration test. If you can't demonstrate your code running
via `cargo run -p roko-cli -- <something>`, you are not done.

## The Quality Bar

### What "done" means (ALL of these, not some):

1. **Compiles**: `cargo build --workspace` passes
2. **Tests pass**: `cargo test --workspace` passes
3. **Clean lint**: `cargo clippy --workspace --no-deps -- -D warnings` passes
4. **Formatted**: `cargo +nightly fmt --all --check` passes
5. **Wired**: Your code is called from a non-test, non-cfg(test) runtime path
6. **Exercisable**: The task's Wire Target command produces the expected behavior
7. **Verified by grep**: `grep -rn 'YourNewFunction' crates/ --include='*.rs' | grep -v target/ | grep -v test` shows at least one callsite outside the defining module
8. **No regressions**: Existing functionality still works after your changes

### What makes a task FAIL audit (any one of these):

- Code exists but has zero callers outside tests
- Wire target command doesn't work or was never tested
- Changes introduced new `#[allow(...)]` suppressions without justification
- Created a new abstraction that duplicates an existing one
- Fixed a symptom but left the root cause intact
- Changed behavior without updating relevant tests
- Left `TODO`, `FIXME`, or `unimplemented!()` in the path

## Before You Write Any Code

### 1. Search first — NEVER reimplement what exists

```bash
grep -rn 'FunctionName\|StructName' crates/ --include='*.rs' | grep -v target/
```

This codebase has duplicate implementations from parallel development. If you find existing
code that does what you need, USE IT. If it's close but not quite right, extend it.
Do NOT create a parallel version.

### 2. Trace the runtime path

Before changing anything, trace the call chain from CLI entry point to where your code will
run. Write it down:

```
CLI main.rs → commands/plan.rs → runner/event_loop.rs → YOUR_CODE
```

If you can't trace this chain, you don't understand the wiring yet. Read more code.

### 3. Read the "What NOT to Do" section

Every task has constraints. Violating them is an automatic audit failure. The constraints
exist because previous agents made exactly those mistakes.

### 4. Check if the dead code you're wiring is well-designed

Not all existing code should be wired as-is. If the code has architectural problems
(wrong abstraction level, incompatible types, missing data), report this in your Status Log
and propose what needs to change. Don't blindly wire broken code.

## Anti-Patterns That Will Fail Audit

### 1. "I'll wire it later"
If you can't wire it now, the task isn't ready. Report the blocker.

### 2. "Tests pass so it works"
Unit test coverage ≠ runtime execution. The Wire Target command must work.

### 3. "I exported it from lib.rs"
Public exports are not wiring targets. The code must be CALLED, not just callable.

### 4. Partial fixes
If the task says "fix all X references" and you fix 3 out of 30, that's not done.
You must grep to verify completeness.

### 5. Adding a 2nd/3rd way to do the same thing
If there's already a function that loads config, don't write another one.
Extend or replace the existing one. Check these common duplicates:
- Config loading: `load_config_unified()`, `load_layered()`, `load_config_validated()`
- Path construction: `Workspace` (roko-core) for workspace-bound public/runtime paths;
  `RokoLayout` (roko-fs) remains live for roko-fs internals, layout migration/versioning,
  and existing callsites until task 004 completes its phased migration.
- Output sinking: `RunOutputSink` trait
- Provider resolution: `effective_providers()`

### 6. Band-aid fixes
If a task says to fix error handling in one file but the same pattern exists in 10 files,
fix all of them or document the remaining ones in your Status Log with grep output.

### 7. Scope creep
If you find another bug while working, add it to your Status Log. Don't fix it inline
unless it's blocking your task. Someone else will get a task for it.

## Architectural Decisions (Already Made)

These decisions have been made. Follow them, don't re-decide:

1. **Workspace struct (roko-core) is the canonical public boundary** for workspace-bound
   path construction. `RokoLayout` (roko-fs) is not deleted yet; treat it as a lower-level
   filesystem/layout catalog used by roko-fs internals and documented legacy callsites until
   task 004 finishes the migration. Use `Workspace` for all new `.roko/` runtime paths.

2. **`.roko/learn/` is the canonical directory** for learning state (episodes, playbooks, etc.).
   `.roko/memory/` is legacy migration surface. New runtime writes should use `.roko/learn/`;
   reads from `.roko/memory/` need an explicit migration/fallback note.

3. **Gates are TOML-configurable** via `[gates]` in roko.toml. Built-in gates become
   shell commands. Users can add custom gates.

4. **Demo app gets redesigned** per SCENARIO-REDESIGN.md (14 scenarios → 5 with custom panels + SSE).
   Don't patch old scenarios — they're being replaced.

5. **Config loading consolidates to `roko-core::config::loader`**. Don't add new loader
   functions elsewhere.

6. **IndexMap for ordered config fields** (providers, models). HashMap stays for lookup-only maps.

## If You're an Agent Assigned a Task

1. Your task file is in `tmp/taskrunner/tasks/{ID}-{name}.md`
2. Read the ENTIRE task file before starting
3. Read ALL files listed in "Background" section
4. Work ONLY in your assigned worktree
5. Make ONLY the changes in "What to Change" — nothing else
6. Run ALL verification commands before reporting done
7. Update the Status Log with what you did, what you found, and any remaining issues

## If You're Orchestrating (Spawning Other Agents)

### Finding work
```bash
cd /Users/will/dev/nunchi/roko/roko/tmp/taskrunner
./scripts/next.sh --count 20    # Find up to 20 claimable tasks
```

### Spawning agents
```bash
./scripts/spawn.sh 003 "agent-name"
# This outputs the full prompt to give the agent
```

### After agents complete
```bash
./scripts/status.sh                # Check status
./scripts/merge.sh 003             # Merge completed worktrees
./scripts/gate.sh wave-1           # Run wave gate after all merges
./scripts/audit.sh wave-1          # Spawn audit agents if gate passes
```

### Parallel execution flow
```
1. ./scripts/next.sh --count 20
2. For each task: ./scripts/spawn.sh {id} {agent}
3. Agents work in parallel (isolated worktrees)
4. As agents finish: ./scripts/merge.sh {id}
5. After all in wave: ./scripts/gate.sh {wave}
6. If gate passes: ./scripts/audit.sh {wave}
7. After audit: back to step 1 with next wave
```

## Status Flow

```
pending → claimed → implemented → tested → wired → verified → done
```

A task is NOT done until an audit agent confirms wiring via the Wire Target command.
