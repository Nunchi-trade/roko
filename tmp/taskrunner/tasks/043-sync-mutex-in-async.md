# Task 043: Replace Sync Mutex with Tokio Mutex in Async Contexts

```toml
id = 43
title = "Replace parking_lot::Mutex with tokio::sync::Mutex where held across .await"
track = "infrastructure"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-serve/src/state.rs",
    "crates/roko-learn/src/runtime_feedback.rs",
]
exclusive_files = [
    "crates/roko-serve/src/state.rs",
    "crates/roko-learn/src/runtime_feedback.rs",
]
estimated_minutes = 90
```

## Context

Sync `parking_lot::Mutex` blocks the OS thread when contended. When held from async
handlers (which run on Tokio worker threads), this starves other tasks on the same
worker. The audit (S15.1) identified two subsystems with this pattern.

## Background

Read these files before starting:
- `crates/roko-serve/src/state.rs` — `affect_engine: Mutex<DaimonState>` (line ~360)
- `crates/roko-learn/src/runtime_feedback.rs` — `affect_engine`, `pattern_miner`,
  `experiment_store`, `local_rewards`, `section_effectiveness` (lines ~1250-1265)

Also grep for all callers:
```bash
grep -rn 'affect_engine\.lock\|pattern_miner\.lock\|experiment_store\.lock\|local_rewards\.lock\|section_effectiveness\.lock' crates/ --include='*.rs' | grep -v target/
```

Current source reality to verify before editing:

- `crates/roko-serve/src/state.rs` currently imports
  `tokio::sync::{Mutex, OnceCell, RwLock}` and defines
  `AppState.affect_engine: Mutex<DaimonState>`.
- Current serve call sites in `crates/roko-serve/src/dispatch.rs` and
  `crates/roko-serve/src/dreams.rs` use `.lock().await` for `affect_engine`.
  If this remains true at implementation time, the serve-side audit finding is
  already fixed and no serve source edit is needed.
- `crates/roko-learn/src/runtime_feedback.rs` still has several
  `parking_lot::Mutex` fields, but the observed current uses are short
  synchronous critical sections with no guard held across `.await`.
- The purpose of this task is not "convert every parking_lot mutex"; it is
  "remove sync mutex guards from async wait points".

## What to Change

1. **`roko-serve/src/state.rs`**: First verify whether `affect_engine` is already
   `tokio::sync::Mutex<DaimonState>`. If it is, and all async handler call sites
   already use `.lock().await`, make no serve change. If the sync mutex has
   returned, change it to `tokio::sync::Mutex<DaimonState>` and update all callers
   to `.lock().await`.

2. **`roko-learn/src/runtime_feedback.rs`**: For each of these fields:
   - `affect_engine: parking_lot::Mutex<DaimonState>`
   - `pattern_miner: parking_lot::Mutex<PatternMiner>`
   - `experiment_store: parking_lot::Mutex<ExperimentStore>`
   - `local_rewards: parking_lot::Mutex<HashMap<String, LocalRewardFunction>>`
   - `section_effectiveness: parking_lot::Mutex<SectionEffectivenessRegistry>`

   Determine whether the lock is held across `.await` points. If yes, migrate to
   `tokio::sync::Mutex`. If the critical section is purely synchronous (no `.await`),
   `parking_lot::Mutex` is correct and should stay.

3. Update accessor methods that return `&parking_lot::Mutex<T>` to return
   `&tokio::sync::Mutex<T>` for migrated fields.

## Mechanical Inspection Plan

1. Inspect these files before editing:
   - `crates/roko-serve/src/state.rs`
   - `crates/roko-serve/src/dispatch.rs`
   - `crates/roko-serve/src/dreams.rs`
   - `crates/roko-learn/src/runtime_feedback.rs`
2. For each lock, identify the lexical scope of the guard. A nearby `.await` in
   the same function is not enough; the guard itself must remain live across the
   await to justify migration.
3. If migrating a field:
   - change the field type;
   - update constructor initialization;
   - update every accessor returning the mutex type;
   - update every call site to `.lock().await`;
   - drop or scope the guard before unrelated I/O.
4. If no guard crosses `.await`, leave source unchanged and record the task as
   verified/no-op in the implementation notes.

## Current-State Decision Table

- `roko-serve::state::AppState.affect_engine`: expected no-op if it remains a
  Tokio mutex and serve handlers use `.lock().await`.
- `runtime_feedback::affect_engine`: synchronous `DaimonState` query/update; keep
  `parking_lot::Mutex` unless a guard crosses `.await`.
- `runtime_feedback::pattern_miner`: `ingest_episode` is synchronous; keep unless
  a guard crosses `.await`.
- `runtime_feedback::experiment_store`: current save/sync work is synchronous;
  do not migrate solely because the enclosing function is async.
- `runtime_feedback::local_rewards`: short map updates/saves; keep unless a guard
  crosses `.await`.
- `runtime_feedback::section_effectiveness`: short registry update/save; keep
  unless a guard crosses `.await`.

## Verification Greps

Use these as navigation aids before and after source edits:

```bash
rg -n 'affect_engine: Mutex<DaimonState>|state\.affect_engine\.lock\(\)\.await|affect_engine\.lock\(\)\.await' crates/roko-serve/src
rg -n 'parking_lot::Mutex|\.lock\(\)' crates/roko-learn/src/runtime_feedback.rs
rg -n 'let mut .*lock\(\)|let .*lock\(\)|\.await' crates/roko-learn/src/runtime_feedback.rs
```

Manually inspect scopes; do not infer a violation from grep output alone.

## What NOT to Do

- Don't convert every sync Mutex to async — only those held across `.await` points.
  Sync Mutex is faster when the critical section is short and synchronous.
- Don't change `parking_lot::RwLock` — the audit is about `Mutex` only.
- Don't touch `CacheCell` — already fixed per audit S15.2.
- Don't modify PlaybookStore locks — already redesigned per audit S15.3.
- Don't convert `parking_lot::Mutex` fields that are used only in short
  synchronous critical sections.
- Don't introduce nested runtimes or `block_on` to work around async locking.
- Don't replace an existing `tokio::sync::Mutex` with another wrapper.

## Wire Target

```bash
# Compile check (this is a type-level fix — existing tests exercise the paths)
cargo build --workspace
cargo test -p roko-serve --lib
cargo test -p roko-learn --lib
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] No `parking_lot::Mutex` held across `.await` in modified files
- [ ] Grep confirms no remaining sync Mutex in async handlers:
      `grep -rn 'parking_lot::Mutex' crates/roko-serve/src/state.rs crates/roko-learn/src/runtime_feedback.rs`
      — only fields that have purely synchronous critical sections remain
- [ ] `cargo check -p roko-serve`
- [ ] `cargo check -p roko-learn`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
