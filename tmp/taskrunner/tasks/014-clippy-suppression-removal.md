# Task 014: Remove Blanket Clippy Suppression in main.rs

```toml
id = 14
title = "Remove blanket clippy allow-all and fix underlying lint warnings"
track = "cleanup"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/main.rs",
]
exclusive_files = ["crates/roko-cli/src/main.rs"]
estimated_minutes = 120
```

## Context

`crates/roko-cli/src/main.rs` lines 10-20 have `#![cfg_attr(clippy, allow(clippy::all, ...))]`
that suppresses ALL clippy warnings for the entire crate. This hides real issues.

W8-A was never implemented — all 6 checklist items unchecked.

Sources:
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` — W8-A: Zero implementation
- `tmp/solutions/demo-running/archive/batches-executed/W8-A-clippy-blanket.md` — original suppression-removal target and accepted remaining crate-level allows

## Background

Read the top of `crates/roko-cli/src/main.rs` to see the suppression attributes.

Current code snapshot:
- Keep `#![allow(clippy::too_many_lines)]`.
- Add/keep a standalone `#![allow(missing_docs)]` if removing the blanket attribute exposes missing-docs failures; the W8-A source explicitly allowed this crate-level exception for the CLI binary.
- Remove the blanket `#![cfg_attr(clippy, allow(clippy::all, clippy::pedantic, clippy::nursery, clippy::restriction, missing_docs))]`.
- After removal, the implementation loop is driven by `cargo clippy -p roko-cli --no-deps -- -D warnings`.

## What to Change

1. **Remove the blanket `#![cfg_attr(clippy, allow(...))]` attributes**.
2. **Run `cargo clippy -p roko-cli --no-deps -- -D warnings`** to see what warnings appear.
3. **Fix warnings in `crates/roko-cli/src/main.rs` only** unless the task metadata is explicitly expanded. This task owns only that file.
4. **If some warnings are intentional**, add targeted `#[allow(clippy::specific_lint)]` on
   individual functions/items with a short reason comment.
5. Run `cargo fmt --check` after code changes. If formatting is needed, run `cargo fmt` and verify the diff only contains mechanical formatting in files touched by this task.

Mechanical order:
1. Edit only the crate attributes at the top of `main.rs`.
2. Run clippy.
3. Fix one warning class at a time and rerun clippy after each small batch.
4. For warnings that are intentionally not worth changing, use the narrowest possible item-level allow, for example `#[allow(clippy::large_enum_variant)]`, with a reason.
5. Confirm no blanket `clippy::all`, `clippy::pedantic`, `clippy::nursery`, or `clippy::restriction` allow remains in `main.rs`.

## What NOT to Do

- Don't suppress warnings you can fix.
- Don't add new blanket suppressions.
- Don't change program behavior — only lint fixes.
- Don't reformat code (leave that to `cargo fmt`).
- Don't rename CLI flags, clap variants, subcommands, environment variables, or output strings as a lint fix.
- Don't use module-wide or crate-wide pedantic/nursery/restriction allows.
- If clippy reports warnings outside `crates/roko-cli/src/main.rs`, do not edit those files under this task; record the blocker in the Status Log and request a touch-list expansion.

## Wire Target

```bash
cargo clippy -p roko-cli --no-deps -- -D warnings
# Should pass clean (no warnings)
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy -p roko-cli --no-deps -- -D warnings` — passes clean
- [ ] `rg -n 'cfg_attr\\(\\s*clippy|clippy::all|clippy::pedantic|clippy::nursery|clippy::restriction' crates/roko-cli/src/main.rs` returns no blanket suppression

## Status Log

| Time | Agent | Action |
|------|-------|--------|
