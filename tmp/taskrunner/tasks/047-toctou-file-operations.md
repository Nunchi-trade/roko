# Task 047: Fix TOCTOU Race Conditions in File Operations

```toml
id = 47
title = "Replace check-then-act file patterns with direct-read-handle-error"
track = "infrastructure"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/plan_loader.rs",
    "crates/roko-cli/src/runner/extension_loader.rs",
]
exclusive_files = [
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/plan_loader.rs",
    "crates/roko-cli/src/runner/extension_loader.rs",
]
estimated_minutes = 60
```

## Context

The audit (S6.3) identified a check-then-act anti-pattern in file operations:
```rust
if path.exists() {
    fs::read_to_string(&path)?  // TOCTOU: path may be deleted between check and read
}
```

This appears in the runner's plan loader, event loop, and extension loader. While the
race window is small, it causes spurious panics/errors when concurrent processes or
agents modify files during a plan run.

## Background

Read:
- `crates/roko-cli/src/runner/plan_loader.rs` — `exists()` checks before reading
  tasks.toml, plan.md, crate scaffolds
- `crates/roko-cli/src/runner/event_loop.rs` — `exists()` checks before reading
  orchestrator snapshots, episode logs, playbook dirs
- `crates/roko-cli/src/runner/extension_loader.rs` — `exists()` check before reading
  extensions dir
- `crates/roko-core/src/io.rs` — use existing `read_optional()` instead of
  open-coding optional reads where it fits
- `crates/roko-cli/src/task_parser.rs:680-683` — `TasksFile::parse()` already
  reads the file; if you need to distinguish `NotFound`, read the file in
  `plan_loader.rs` and call `TasksFile::parse_str()`

Grep for the pattern:
```bash
grep -n '\.exists()' crates/roko-cli/src/runner/plan_loader.rs crates/roko-cli/src/runner/event_loop.rs crates/roko-cli/src/runner/extension_loader.rs
```

Current live `event_loop.rs` sites are at approximately 2387, 2392, 2527,
3202, 3731, and 3921; the older audit line numbers in this task are stale.
Do not change the `Cargo.toml` existence check around 3202 because it gates
whether built-in Cargo gates should run, not a read-after-exists pattern.

## What to Change

Replace `if path.exists() { read(path) }` with direct read + `NotFound` handling:

```rust
// BEFORE:
if tasks_path.exists() {
    let content = fs::read_to_string(&tasks_path)?;
    // process
}

// AFTER:
match fs::read_to_string(&tasks_path) {
    Ok(content) => { /* process */ },
    Err(e) if e.kind() == io::ErrorKind::NotFound => { /* skip or default */ },
    Err(e) => return Err(e.into()),
}
```

Apply this pattern to these locations:

1. **`plan_loader.rs`**:
   - Lines 32-43: replace `if !tasks_path.exists()` plus `TasksFile::parse()`
     with a direct `std::fs::read_to_string(&tasks_path)` match. On
     `NotFound`, return `bail!("No tasks.toml found in {}", dir.display())`.
     On success, call `TasksFile::parse_str(&content)`.
   - Lines 77-79: replace `if dir.join("tasks.toml").exists()` with a helper
     that attempts to read/parse `dir/tasks.toml`; return a single plan on
     `Ok(Some(plan))`, continue to subdir scan on `Ok(None)`.
   - Lines 89-90: keep `path.is_dir()`, but remove
     `path.join("tasks.toml").exists()`. Call the same direct load helper and
     skip only when the helper reports `NotFound`.
   - Lines 143-144: replace PRD excerpt `path.exists()` + read with
     `roko_core::io::read_optional(path)`. Continue on `Ok(None)` and preserve
     current best-effort behavior for unreadable PRDs by logging at debug/warn
     and trying the next candidate.
   - Lines 334-344: replace `if !ws_cargo_path.exists() { write minimal }`
     followed by read with one direct read. On `NotFound`, write the minimal
     manifest and use that string as `ws_content`; on other errors, propagate.

2. **`event_loop.rs`**:
   - Lines 2387-2406: remove the preflight `exists()` checks from
     `load_executor()`. Try `load_orchestrator_checkpoint(paths)` first; if it
     returns `Ok(None)`, attempt `roko_core::io::read_optional(&paths.executor_json)`.
     If both are `None`, return the existing `ResumeOutcome::Fresh` marker.
   - Lines 2409-2425: when executor JSON is missing, treat it as the fresh
     no-snapshot case, not as a read failure.
   - Lines 2527-2531: in `load_orchestrator_checkpoint()`, replace the
     `exists()` guard with `read_optional(&paths.orchestrator_json)`.
     Return `Ok(None)` for missing and parse only `Some(json)`.
   - Lines 3731-3735: do not call `EpisodeLogger::compact()` on a missing path,
     because it will currently create an empty log. Replace the guard with a
     direct async `tokio::fs::metadata(episodes_path).await` match. Treat
     `NotFound` as a no-op; if the file disappears after metadata but before or
     during compaction, treat that NotFound-shaped compaction error as a no-op
     and preserve existing logging for other compaction errors.
   - Lines 3921-3922: for playbook seeding, replace `pb_dir.exists()` plus
     `tokio::fs::read_dir()` with direct `tokio::fs::read_dir(&pb_dir).await`.
     Treat `NotFound` as "empty, seed starter playbooks"; propagate/log other
     read_dir errors as best-effort and continue to seed.

3. **`extension_loader.rs`**:
   - Lines 127-133: replace `if !dir.exists()` with direct
     `std::fs::read_dir(dir)`. On `NotFound`, log the same debug skip and
     continue. On other errors, warn and continue. Then pass the directory to
     `roko_plugin::manifest::discover_plugins(dir)`.
   - Note: `roko_plugin::manifest::discover_plugins()` has its own internal
     `exists()` checks. Do not modify that crate in this task unless the touch
     list is explicitly expanded; document the residual lower-level pattern in
     the Status Log if left unchanged.

## Mechanical Implementation Notes

Prefer one small helper in `plan_loader.rs`, for example:

```rust
fn try_load_plan(dir: &Path) -> Result<Option<Plan>> { ... }
```

It should return:
- `Ok(Some(plan))` when `tasks.toml` was read and parsed
- `Ok(None)` only for `io::ErrorKind::NotFound`
- `Err(_)` for parse errors and non-NotFound I/O errors

This keeps discovery able to skip non-plan subdirectories without swallowing
corrupt `tasks.toml` files.

For resume loading, keep the existing observable markers:
missing snapshots -> `ResumeOutcome::Fresh`; unreadable snapshot file ->
`ResumeOutcome::ReadFailed`; corrupt JSON -> `ResumeOutcome::Corrupt`.
The task changes how files are opened, not the user-facing resume semantics.

## Tests to Add or Update

- `plan_loader.rs`: add/adjust tests for loading a directory containing one
  valid plan and for scanning subdirectories where one subdir has no
  `tasks.toml`.
- `event_loop.rs`: add a focused unit test around resume loading if existing
  helpers allow it, covering "orchestrator missing, executor present" and
  "both missing means fresh".
- `extension_loader.rs`: add a test that calls `load_extensions()` with no
  `.roko/extensions` or `plugins` directories and asserts it returns `0`
  without error.

## What NOT to Do

- Don't change `is_dir()` checks — those are legitimate for determining path type.
- Don't change `create_dir_all` patterns — those are idempotent.
- Don't change unrelated test code.
- Don't introduce async file I/O — stay with `std::fs` where the crate already uses it.
- Don't turn missing optional files into warnings; preserve existing quiet
  skip/default behavior for optional PRD excerpts, extension dirs, playbook dirs,
  and absent snapshots.

## Wire Target

```bash
cargo build --workspace
cargo test -p roko-cli -- runner
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] No `path.exists()` immediately followed by `read_to_string(path)` in modified files
  (grep: `grep -A2 '\.exists()' <modified_files> | grep 'read_to_string'`)
- [ ] `grep -n '\.exists()' crates/roko-cli/src/runner/plan_loader.rs crates/roko-cli/src/runner/event_loop.rs crates/roko-cli/src/runner/extension_loader.rs`
      shows only path-type/gate checks explicitly allowed above

## Status Log

| Time | Agent | Action |
|------|-------|--------|
