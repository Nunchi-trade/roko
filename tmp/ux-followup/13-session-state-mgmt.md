# Session & State Management — Resume / Schema / Process Lifecycle

> **New file** added 2026-04-16 during post-PR-13 audit. Items here describe
> resume-path, schema-versioning, and process-supervision gaps that don't fit
> cleanly into either "partially-wired subsystems" (file 05) or
> "hygiene & coverage" (file 09). They are the *resilience* surface — what
> happens when roko crashes, restarts, or upgrades.
>
> **Re-audit 2026-04-20**: 3 items closed (79, 80, 82). 1 item still open (81).

## Summary

Four items spanning the executor snapshot, the process supervisor, and
plan-discovery consistency at resume time. All P1 — none block PR #13 but each
is a latent footgun the moment a real user runs roko across two roko releases
or an unexpected SIGKILL.

## Items

### 79. [DONE] Executor snapshot has no schema-version field

**Resolved in**: `ExecutorSnapshot` now carries a `schema_version` field, asserted at
orchestrate.rs line ~734 (`debug_assert_eq!(snapshot.schema_version, CURRENT_SCHEMA_VERSION)`).
`crates/roko-cli/src/snapshot_migrate.rs` implements the migration framework with per-version
upgrade functions, handling v0 through v2. Cross-ref item 60d also DONE.

**Status**: DONE.

---

### 80. [DONE] ProcessSupervisor zombie / signal escalation incomplete

**Resolved in**: `crates/roko-runtime/src/process.rs` now has full SIGTERM-then-SIGKILL
escalation in `shutdown()` (line ~326-344): sends `Signal::SIGTERM` (line ~339), waits
for graceful exit (line ~341), then escalates to `force_kill()` (line ~344).
`impl Drop for ProcessSupervisor` is implemented (line ~708-729): logs a warning, takes
all live children, and calls `force_kill_sync()` on each. The `CancellationToken` is
plumbed through the supervisor (cancel field at line ~80, `cancel_token()` accessor at
line ~348). Cross-ref items 60e and 18.

**Status**: DONE.

---

### 81. No migration strategy for `.roko/state/executor.json` between roko releases

**Evidence**: Even after item 79 lands a `schema_version` field, there is no
`SnapshotMigrate::upgrade` mechanism to walk v1 → v2 → … on load. A new roko
release that bumps the version will simply refuse to resume an old snapshot.

**Current state**: Schema versioning is necessary but not sufficient — without
migration, every upgrade bricks in-flight runs.

**Gap**: New `crates/roko-cli/src/snapshot_migrate.rs` with one fn per version
delta, dispatched on the snapshot's `schema_version` at load.

**Fix scope**: 1–2 days for the framework + first migration. Cross-ref T32 in
`11-execution-plan.md`.

**Priority**: P1.

---

### 82. [DONE] Resume doesn't validate plan-discovery consistency

**Resolved in**: `crates/roko-cli/src/snapshot_reconcile.rs` implements
`reconcile_snapshot_vs_plans()` which walks the snapshot's plan IDs and asserts each exists
in the discovered plan set. Returns `SnapshotReconcileError::PlanIdsMissing` with clear
error messaging (line ~63: "resume snapshot references plans [X] but plans/ at Y has [Z].
Rename or prune the snapshot before resuming."). Called from main.rs line ~3983 during resume.
Tests at `crates/roko-cli/tests/snapshot.rs` verify the validation.

**Status**: DONE.
