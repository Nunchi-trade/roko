# Task 029: Delete roko-calc Skeleton Crate

```toml
id = 29
title = "Remove the empty roko-calc skeleton crate and its subcrate"
track = "cleanup"
wave = "wave-1"
priority = "low"
blocked_by = []
touches = [
    "crates/roko-calc/",
]
exclusive_files = ["crates/roko-calc/"]
estimated_minutes = 15
```

## Context

`crates/roko-calc/` is a skeleton crate for a CLI calculator. It has a Cargo.toml
referencing a subcrate `roko-calc-engine` but is not listed in the workspace `members`
array and is not used by any other crate. It's dead weight in the repo.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- QW-5
- `tmp/v2-refactoring/03-QUICK-WINS.md` -- QW-5
- `tmp/v2-refactoring/01-CURRENT-STATE.md` -- "roko-calc: skeleton with no lib.rs"

## Background

Read these files first:
1. `crates/roko-calc/Cargo.toml` -- the skeleton crate definition
2. `Cargo.toml` (workspace root) -- verify roko-calc is NOT in `members`

Current repo snapshot to confirm before deleting:
- `Cargo.toml` workspace members currently do not include `crates/roko-calc`.
- `crates/roko-calc/` currently contains only `Cargo.toml`; the referenced `src/main.rs`
  and `engine` path crate are absent.
- A repo search for `roko-calc`, `roko_calc`, or `roko-calc-engine` should only find the
  skeleton crate itself and task/refactor docs.

## What to Change

1. **Verify roko-calc is NOT in workspace members**:
   ```bash
   rg -n 'roko-calc' Cargo.toml
   ```
   Expected: no matches. If it IS listed, remove it from the `members` array.

2. **Verify no crate depends on roko-calc**:
   ```bash
   rg -n 'roko-calc|roko_calc|roko-calc-engine' Cargo.toml Cargo.lock crates/ \
     --glob '!crates/roko-calc/**' --glob '!target/**'
   ```
   Expected: no matches.

3. **Delete the directory and nothing else**:
   ```bash
   rm -rf crates/roko-calc/
   ```

4. **Verify the workspace still builds and tests without the skeleton**:
   ```bash
   cargo build --workspace
   cargo test --workspace
   ```

## What NOT to Do

- Don't delete any other crate.
- Don't modify the workspace Cargo.toml unless roko-calc is listed there (it shouldn't be).
- Don't try to "fix" the skeleton by adding `src/main.rs`, an `engine` crate, or workspace
  membership. The task is deletion.
- Don't clean unrelated stale dependencies from `Cargo.lock`; `roko-calc` is not expected to
  have contributed lockfile entries because it is not a workspace member.

## Wire Target

```bash
# Deletion -- verify the workspace still builds:
cargo build --workspace
cargo test --workspace
```

## Verification

- [ ] `crates/roko-calc/` directory no longer exists
- [ ] `rg -n 'roko-calc|roko_calc|roko-calc-engine' Cargo.toml Cargo.lock crates/ --glob '!target/**'` -- returns nothing
- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
