# Task 052: Migrate Remaining Critical Writes to Atomic Pattern

```toml
id = 52
title = "Replace std::fs::write with atomic_write in PRD, learn, and config paths"
track = "infrastructure"
wave = "wave-1"
priority = "low"
blocked_by = []
touches = [
    "crates/roko-cli/src/prd.rs",
    "crates/roko-learn/src/cascade_router.rs",
    "crates/roko-learn/src/costs_log.rs",
    "crates/roko-learn/src/episode_logger.rs",
]
exclusive_files = []
estimated_minutes = 60
```

## Context

The audit (S9.5, S12.8) identified that while the runner persistence layer uses
atomic writes (write-to-tmp-then-rename), several other subsystems still use raw
`std::fs::write`. A crash mid-write leaves a corrupted file.

`roko_core::io::atomic_write` and `roko_fs::atomic_write_bytes` both exist and are
already used in the runner. This task extends that pattern to remaining critical paths.

## Background

Grep for non-atomic writes in production code:
```bash
rg -n "std::fs::write|tokio::fs::write|fs::write\(" crates/roko-cli/src/prd.rs crates/roko-learn/src --glob '*.rs'
```

Existing atomic write utilities:
- `roko_core::io::atomic_write(path, data)` — write to a unique sibling temp
  path, then rename
- `roko_core::io::atomic_write_str(path, data)` — string variant
- `roko_fs::atomic_write_json(path, value)` — JSON serialize + atomic write
- `roko_fs::atomic_write_bytes(path, data)` — raw bytes

Current grep facts to preserve:
- `crates/roko-core/src/io.rs` already provides `atomic_write`,
  `atomic_write_async`, `atomic_write_str`, `atomic_write_str_async`, and
  `read_optional`. Its temp files are sibling paths named
  `<path>.tmp.<pid>.<counter>`.
- `crates/roko-cli/src/prd.rs` production overwrites still include
  `regenerate_old_format_plan` rollback writes around lines 330-374,
  `cmd_promote` writing the promoted PRD around line 761, plan generation writes
  around lines 1204/1215/1227, and `update_prd_plans_generated` around line 1770.
- `crates/roko-learn/src/cascade_router.rs::CascadeRouter::save` already writes
  to a sibling temp file and renames it. It is atomic in behavior, but can be
  simplified to the shared helper if this task touches it.
- Current `tokio::fs::write` matches in `crates/roko-learn/src/costs_log.rs` and
  `crates/roko-learn/src/episode_logger.rs` are test setup writes, not production
  persistence. `EpisodeLogger::compact` already writes a temp file, flushes,
  syncs, and renames.

## What to Change

### 1. PRD writes (`prd.rs`)

These production-path writes should use atomic writes:
- Plan tasks.toml: `std::fs::write(plan_dir.join("tasks.toml"), ...)` (~line 1203)
- Plan plan.md: `std::fs::write(plan_dir.join("plan.md"), ...)` (~lines 1214, 1226)
- PRD update: `std::fs::write(prd_path, updated)` (~line 1769)

Mechanical changes:
1. Add `use roko_core::io::atomic_write_str;` near the existing imports.
2. In `generate_plan_from_prd_with_outcome`, assign explicit paths before
   writing so error messages name the destination:
   `let tasks_path = plan_dir.join("tasks.toml");`
   `let plan_md_path = plan_dir.join("plan.md");`
3. Replace each critical overwrite with `atomic_write_str(&path, contents)` and
   keep the existing `.with_context(|| format!("write ..."))` context.
4. In `update_prd_plans_generated`, replace `std::fs::write(prd_path, updated)?`
   with `atomic_write_str(prd_path, &updated)?`.

`cmd_promote` currently writes the published PRD then removes the draft. If Task
046 has already landed an atomic promote path, leave it alone. If Task 046 has
not landed and this raw `std::fs::write(&dst, &content)?` still exists, do not
change promote in this task unless you also add a note to the Status Log that
you intentionally widened scope to remove the same crash window.

**Do NOT change**:
- Idea append: `std::fs::write(&ideas, ...)` (~line 451) — this is creating a new
  file, not overwriting critical state
- Scaffold creation: new file, not overwrite
- Regeneration restore paths: `std::fs::write(&tasks_path, &existing)` — these are
  rollback writes that restore the original content on error. They should stay as
  `std::fs::write` because the original content IS the safe state.

### 2. Learn subsystem writes

Check each write in `crates/roko-learn/src/`:
```bash
rg -n "std::fs::write|tokio::fs::write|fs::write\(" crates/roko-learn/src --glob '*.rs'
```

Many learn subsystem files already use `roko_fs::atomic_write_json`. Migrate any
remaining raw `fs::write` calls that write to `.roko/learn/` files.

Focus on:
- `cascade_router.rs` — `CascadeRouter::save` is currently hand-rolled atomic
  write (`path.with_extension("json.tmp")`, write, rename). Prefer changing it
  to `roko_core::io::atomic_write_str(path, &json)` so it uses the same unique
  temp path and cleanup behavior as the rest of the repo. Keep the existing
  serialization-empty guard.
- `costs_log.rs` — current raw `tokio::fs::write` grep match is in tests. Do not
  change production append-mode JSONL writes.
- `episode_logger.rs` — current raw `tokio::fs::write` grep matches are tests.
  Do not change line-by-line append. If touching compaction, preserve flush,
  `sync_all`, and rename semantics; no migration is required for this task.

Do not roam outside the declared touch list. If the learn grep finds raw writes
in other files such as `feedback_service.rs`, record them as out of scope in the
Status Log instead of editing them.

### 3. Import pattern

Use `roko_core::io::atomic_write` (or `atomic_write_str` for string data) consistently:

```rust
use roko_core::io::atomic_write;

// Instead of:
std::fs::write(&path, &content)?;

// Use:
atomic_write(&path, content.as_bytes())?;
```

## What NOT to Do

- Don't change append-mode writes (JSONL files that append lines).
- Don't change test code.
- Don't change rollback/restore writes that are writing known-good content.
- Don't introduce a new atomic write function — use the existing ones.
- Don't change the runner persist.rs — it already uses atomic writes.
- Don't replace `OpenOptions::append` JSONL logging with whole-file read/modify
  rewrites.
- Don't use a fixed temp filename like `file.json.tmp` for new code; the shared
  helper already avoids concurrent-writer collisions.
- Don't remove error context around writes while swapping to atomic helpers.

## Tests to Add or Update

Add focused regression coverage only where the migrated function has existing
unit-test coverage nearby:
- In `roko-cli` PRD tests, add/extend a test for `update_prd_plans_generated`
  or plan-generation materialization that asserts the target file is updated and
  no `.tmp.` sibling remains after success.
- In `roko-learn` cascade router tests, add a save test that writes an existing
  router snapshot, calls `CascadeRouter::save`, reloads it, and asserts no
  `.tmp.` sibling remains in the directory.
- Do not add tests that mock mid-rename crashes; the helper's own tests in
  `roko-core/src/io.rs` already cover temp cleanup/success behavior.

## Wire Target

```bash
cargo build --workspace
cargo test -p roko-cli -- prd
cargo test -p roko-learn --lib
rg -n "std::fs::write" crates/roko-cli/src/prd.rs
rg -n "std::fs::write|tokio::fs::write|fs::write\(" crates/roko-learn/src --glob '*.rs'
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `rg -n "std::fs::write" crates/roko-cli/src/prd.rs` shows no plan-generation
      or `update_prd_plans_generated` overwrites. Remaining matches are limited
      to idea/scaffold creation, rollback/restore writes, tests, and (if Task
      046 has not landed) the promote path owned by Task 046.
- [ ] PRD plan writes use `atomic_write`
- [ ] `CascadeRouter::save` uses the shared atomic helper or is explicitly left
      as an already-atomic temp-write/rename path in the Status Log
- [ ] Learn subsystem critical state writes use `atomic_write`,
      `atomic_write_str`, or `atomic_write_json`; test-only raw writes are left
      unchanged

## Status Log

| Time | Agent | Action |
|------|-------|--------|
