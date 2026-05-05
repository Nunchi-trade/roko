# Agent Instructions

You are implementing a single task in the roko codebase. Read these instructions carefully.

## Project

Roko is a Rust toolkit for building agents (~177K LOC, 38 crates).
Workspace root: `/Users/will/dev/nunchi/roko/roko`

## Your Job

1. Read the task description below carefully
2. Read ALL files listed in "Files to read first" BEFORE making changes
3. Make ONLY the changes described in "What to Change"
4. Do NOT modify files outside the `touches` list
5. Run the verification commands to confirm your work
6. Do NOT add comments, docstrings, or type annotations to code you didn't change
7. Do NOT refactor or "improve" surrounding code

## Critical Rules

### Search before writing
Before creating any new struct, trait, or function:
```bash
grep -rn 'YourNewName' crates/ --include='*.rs' | grep -v target/
```
If it already exists, USE IT. Do not create duplicates.

### Wire, don't just build
Your code must be CALLED from a runtime path (not just tests).
The task's "Wire Target" section tells you exactly what CLI command should exercise your code.
If the wire target doesn't work after your changes, you're not done.

### No scope creep
If you discover something else that needs fixing, add a note in the Status Log section.
Do NOT fix it yourself — that's a separate task.

## Pre-commit checks

Before marking the task as implemented, run:
```bash
cargo build -p <crate>          # Must compile
cargo test -p <crate>           # Tests must pass
cargo clippy -p <crate> --no-deps -- -D warnings  # No warnings
```

## When Done

Run ALL verification commands from the task. If they pass, update your status:
```bash
cd /Users/will/dev/nunchi/roko/roko/tmp/taskrunner
./scripts/complete.sh <TASK_ID> verified "All verification commands pass"
```
