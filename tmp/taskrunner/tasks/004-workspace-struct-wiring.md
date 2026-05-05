# Task 004: Define Workspace/RokoLayout Boundary and Migrate Path Construction

```toml
id = 4
title = "Define Workspace/RokoLayout boundary, migrate workspace path construction in phases"
track = "config-foundation"
wave = "wave-0"
priority = "critical"
blocked_by = []
touches = [
    "crates/roko-core/src/workspace.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-fs/src/layout.rs",
    "crates/roko-fs/src/archive.rs",
    "crates/roko-serve/src/routes/workspaces.rs",
    "crates/roko-learn/src/runtime_feedback.rs",
    "crates/roko-learn/src/episode_logger.rs",
    "crates/roko-cli/src/dispatch/prompt_builder.rs",
    "crates/roko-core/src/dashboard_snapshot.rs",
]
exclusive_files = ["crates/roko-core/src/workspace.rs", "crates/roko-fs/src/layout.rs"]
estimated_minutes = 180
```

## Context

Two competing path abstractions exist:
- **`Workspace`** (roko-core/src/workspace.rs) — public project-root boundary with typed
  `.roko/` accessors. It now has live callers in `roko-cli`.
- **`RokoLayout`** (roko-fs/src/layout.rs) — older, broader path catalog with many existing
  callsites. It remains live, especially inside filesystem/layout code.

**Current decision: `Workspace` (roko-core) is canonical for workspace-bound public/runtime
paths. `RokoLayout` remains a lower-level roko-fs layout catalog during migration.**

The canonical directory for learning state is `.roko/learn/`, not `.roko/memory/`.
`.roko/memory/` may remain only as explicit legacy/migration fallback surface.

This task also subsumes the old task 005 (path unification), but the original "replace every
`RokoLayout` and every raw `.join(".roko")` in one pass" acceptance criteria are not realistic
for the current branch. Treat this as a phased migration.

## Current Branch Status - 2026-05-05

Status: **partial / needs rescope before more implementation**.

Implemented on `wp-arch2`:
- `crates/roko-core/src/workspace.rs` documents `Workspace` as the public workspace path boundary.
- `Workspace` has live callers in `crates/roko-cli/src/commands/util.rs` and
  `crates/roko-cli/src/main.rs`.
- `crates/roko-fs/src/layout.rs` no longer claims to be the public canonical boundary.

Known gaps:
- `RokoLayout` is not deprecated in code and still has many runtime callsites.
- `Workspace` still lacks several accessors needed to replace `RokoLayout` completely.
- Raw `.join(".roko")` usage remains widespread and should be migrated by subsystem.
- `.roko/memory` remains in live code for legacy/migration behavior; do not remove it without
  a deliberate migration path.

Sources:
- `tmp/v2-refactoring/10-DEAD-CODE-AUDIT.md` — Workspace struct (WIRE NOW)
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` — W10-C: .roko/memory path inconsistency
- Audit finding: 30+ `.roko/memory` references across 9+ files

## Background

Read these files first:
1. `crates/roko-core/src/workspace.rs` — Workspace struct (the winner)
2. `crates/roko-fs/src/layout.rs` — RokoLayout struct (being replaced)
3. Find ALL path references:
   ```bash
   rg '\.roko/memory|\.roko/learn|memory_dir|learn_dir' crates/ -g '*.rs'
   rg 'RokoLayout' crates/ -g '*.rs'
   rg '\.join\(".roko' crates/ -g '*.rs'
   ```

Current branch facts to verify before editing:
- `Workspace` currently has accessors for `.roko/`, `state`, `plans`, `runtime`, legacy
  `memory`, `runs`, `config`, `cache`, `learn`, root `episodes.jsonl`, `signals.jsonl`,
  `roko.log`, root `roko.toml`, `prd`, `research`, `state/executor.json`,
  `learn/gate-thresholds.json`, `learn/cascade-router.json`, and `learn/efficiency.jsonl`.
- `Workspace` is missing accessors that current runtime code still gets from `RokoLayout` or raw
  joins: `events_jsonl_path`, `run_state_path`, `task_trackers_path`, `playbooks_dir`,
  `skills_path`/`skills_dir`, `archive_dir`, `mcp_config_path`, `runner_stderr_log`,
  `jobs_dir`, `extensions_dir`, `neuro_dir`, `custody_log`, and `worktrees_dir`.
- `RokoLayout` is still broadly used. Treat `roko-fs/src/layout.rs`, `roko-fs/src/archive.rs`,
  and `roko-fs/src/gc.rs` as documented filesystem-internal exceptions unless this task
  explicitly migrates a callsite.
- `crates/roko-learn/src/runtime_feedback.rs::project_episode_paths()` currently says canonical
  first but returns `.roko/memory/episodes.jsonl` before root and learn paths. That is a concrete
  bug to fix in this task.
- `crates/roko-serve/src/routes/workspaces.rs::get_workspace_state()` still reads only
  `.roko/memory/episodes.jsonl`; it must read canonical/root/learn first and memory only as a
  legacy fallback.

## What to Change

1. **Expand `Workspace` accessors only for paths this task actually migrates**:
   - Add accessors before changing callsites. Likely first-pass additions:
     `events_jsonl_path() -> .roko/events.jsonl`,
     `run_state_path() -> .roko/state/run-state.json`,
     `task_trackers_path() -> .roko/state/task-trackers.json`,
     `playbooks_dir() -> .roko/learn/playbooks`,
     `archive_dir() -> .roko/learn/archive`,
     `mcp_config_path() -> .roko/mcp.json`,
     `runner_stderr_log() -> .roko/runner-stderr.log`.
   - Keep `memory_dir()` but document it as legacy read/migration surface only.
2. **Add the migration warning in one place**:
   - In `Workspace::open()` (and therefore `open_or_create()`), if `.roko/memory` exists and
     `.roko/learn` does not, emit a `tracing::warn!` telling the user that `.roko/memory` is
     legacy and `.roko/learn` is canonical. Do not move files automatically.
3. **Fix episode path ordering and readers**:
   - In `runtime_feedback.rs::project_episode_paths()`, return canonical/root paths before legacy:
     `.roko/episodes.jsonl`, `.roko/learn/episodes.jsonl`, then `.roko/memory/episodes.jsonl`.
   - In `prompt_builder.rs::episode_paths()`, keep the existing order root -> learn -> memory,
     but build paths through `Workspace` accessors and keep the memory fallback comment explicit.
   - In `dashboard_snapshot.rs::load_from_workdir()` and `bootstrap_episodes()`, use
     `Workspace` accessors for state/learn/root episode paths; keep memory fallback only in
     `bootstrap_episodes()`.
   - In `roko-serve/src/routes/workspaces.rs::get_workspace_state()`, read episodes from root
     `Workspace::episodes_path()`, then `.roko/learn/episodes.jsonl`, then
     `Workspace::memory_dir().join("episodes.jsonl")` as legacy fallback.
4. **Migrate high-risk runtime `RokoLayout` usage in touched files**:
   - `routes/workspaces.rs`: replace create-state layout construction with `Workspace::create()`
     or `Workspace::open_or_create()`; remove `use roko_fs::layout::RokoLayout` from this file if
     no longer needed.
   - `dashboard_snapshot.rs`: derive `Workspace` from the resolved root and use its accessors for
     state, learn, engrams/signals, and episode paths.
   - `runtime_feedback.rs`: use `Workspace` for project-root path construction, but leave
     `LearningPaths::under(workspace.learn_dir())` as the runtime storage root.
   - `prompt_builder.rs`: use `Workspace` for playbooks and episode candidates.
   - `roko-fs/src/archive.rs`: either keep as a documented roko-fs internal exception, or migrate
     the archive root to a `Workspace` accessor if no roko-fs-only coupling remains. Do not do both.
5. **Document exceptions in the Status Log**:
   - Paste the final grep summaries for `RokoLayout`, `.roko/memory`, and raw `.join(".roko")`.
   - For each remaining runtime hit, write one phrase explaining why it remains (test, legacy
     fallback, roko-fs internal, or later subsystem migration).

## What NOT to Do

- Don't change the actual data format of files inside the directories.
- Don't move files on disk automatically (just warn). Users should control that.
- Don't claim full migration unless grep output is included and documented exceptions are listed.
- Don't create a THIRD path abstraction.
- Don't delete or globally deprecate `RokoLayout` while `roko-fs`, serve dispatch, runner
  persistence, and tests still use it.
- Don't leave `.roko/memory` as the first path tried by any production reader.
- Don't convert broad test fixtures unless they are in files touched for runtime behavior; test-only
  `.roko/memory` setup is acceptable when it verifies legacy fallback.

## Wire Target

```bash
# Phase check:
rg '\.roko/memory' crates/ -g '*.rs'
# Every remaining hit must be a documented legacy/migration fallback.

rg 'RokoLayout' crates/ -g '*.rs'
# Remaining hits must be roko-fs internals or documented migration exceptions.

rg '\.join\(".roko' crates/roko-cli/src crates/roko-serve/src -g '*.rs'
# High-traffic runtime paths should trend down as Workspace accessors are added.
```

Expected observable behavior:
- A workspace containing `.roko/episodes.jsonl` is read before legacy
  `.roko/memory/episodes.jsonl` by serve workspace state, dashboard snapshot, runtime feedback,
  and prompt episode context.
- Opening a workspace with `.roko/memory/` but no `.roko/learn/` logs a warning and does not move
  files.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo test -p roko-core workspace`
- [ ] `cargo test -p roko-learn project_episode_paths`
- [ ] `cargo test -p roko-cli learn_paths`
- [ ] Remaining `RokoLayout` callsites are either roko-fs internals or documented exceptions
- [ ] Remaining `.roko/memory` hits are explicit migration/fallback paths
- [ ] No new raw `.join(".roko")` call was added in touched runtime code
- [ ] `rg 'use roko_core::.*Workspace' crates/ -g '*.rs'` shows imports in migrated callers

## Status Log

| Time | Agent | Action |
|------|-------|--------|
| 2026-05-05 | wp-arch2 audit | Rescoped task. Current branch uses `Workspace` as public boundary but has not completed the `RokoLayout`/raw path migration. Do not mark done until remaining callsites are reduced or documented by subsystem. |
