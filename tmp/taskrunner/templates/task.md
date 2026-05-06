+++
id = "000"
title = ""
track = ""
wave = ""
priority = "p1"
blocked_by = []
touches = []                    # Files this task may modify
exclusive_files = []            # Files ONLY this task may touch (prevents conflicts)
estimated_minutes = 0
+++

# Task {ID}: {TITLE}

## Context

<!-- WHY this task exists. Link to source docs if relevant. -->
<!-- Include enough context that a fresh agent with zero prior knowledge -->
<!-- can understand the problem and implement the solution. -->

**Source**: `tmp/solutions/...`
**Track**: {TRACK}

## Background

<!-- What does the current code look like? Include file paths and key -->
<!-- line numbers. The agent MUST read these files before changing anything. -->

**Files to read first**:
- `crates/.../file.rs` — what this file does, what to look for

## What to Change

<!-- Specific, actionable instructions. No ambiguity. -->
<!-- Include code snippets if the change is non-obvious. -->

1. In `crates/.../file.rs`:
   - Change X to Y
   - Add Z

2. In `crates/.../other.rs`:
   - Wire the new thing

## What NOT to Do

<!-- Explicit anti-patterns. Prevent scope creep and wasted work. -->

- Do NOT modify files outside the `touches` list
- Do NOT add new crates or dependencies unless specified
- Do NOT refactor surrounding code — only change what's described above
- Do NOT add comments, docstrings, or type annotations to unchanged code

## Wire Target

<!-- The CLI command or runtime path that exercises this code. -->
<!-- If you can't fill this in, the task isn't ready. -->

```bash
cargo run -p roko-cli -- <command>
```

**Expected behavior**: Describe what the command should do after the change.

## Verification

<!-- Commands to run to prove the task is complete. -->
<!-- Include both positive tests (it works) and negative tests (it fails correctly). -->

```bash
# 1. Compiles
cargo build -p <crate>

# 2. Tests pass
cargo test -p <crate>

# 3. Wire target works
cargo run -p roko-cli -- <command>

# 4. Code is called from non-test path
grep -rn '<key_function>' crates/ --include='*.rs' | grep -v test | grep -v target/
# Should show at least one callsite outside tests
```

## Status Log

<!-- Agents append their progress here -->
<!-- Format: YYYY-MM-DD HH:MM agent-name: status — notes -->
