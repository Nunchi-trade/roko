# Task 094: Atomic Group State Persistence

```toml
id = 94
title = "Redesign orchestration state persistence as a single checksummed StateSnapshot"
track = "infrastructure"
wave = "wave-3"
priority = "high"
blocked_by = [52]
touches = [
    "crates/roko-cli/src/runner/event_loop.rs",
    "crates/roko-cli/src/runner/persist.rs",
    "crates/roko-runtime/src/state_snapshot.rs",
]
exclusive_files = ["crates/roko-runtime/src/state_snapshot.rs"]
estimated_minutes = 180
```

## Context

This is a redesign of orchestration state persistence, not a bandaid. The current runner
writes state across three separate files sequentially:

1. `executor.json` — executor + task DAG snapshot
2. `run-state.json` — runner-owned counters (tokens, cost, task counts)
3. `orchestrator.json` — orchestrator snapshot with merge queue

Each file is written atomically (via tmp-rename), but the three writes are not grouped.
`gate_thresholds.json` is written on a separate code path entirely (`update_gate_thresholds`
in `event_loop.rs`). A crash between any two writes leaves the state group in an inconsistent
condition: the executor shows tasks as completed while the counters have not advanced, or the
gate thresholds reflect a rung outcome that has not been committed to the executor snapshot.

The redesign: introduce a `StateSnapshot` struct in `roko-runtime` that bundles all four
pieces of state into a single JSON document with a `sha256` checksum field. The runner
serializes one `StateSnapshot` and writes it with a single `atomic_write` call. On resume,
the checksum is validated before any state is loaded. Three separate disk paths collapse into
one.

Checklist items: S13.5, Phase 4.3.

## Background

Read these files before writing any code:

1. `crates/roko-cli/src/runner/persist.rs` — `PersistPaths` (lines 25–68), `atomic_write`
   (line 238), `save_executor_snapshot` (line 278), `save_orchestrator_snapshot` (line 285),
   `save_run_state` (line 301), `load_run_state` (line 310), `RunStateSnapshot` (line 82),
   `GateThresholds` and `load_gate_thresholds` (line 233). Understand what each file currently
   contains and which fields belong to which struct.
2. `crates/roko-cli/src/runner/event_loop.rs` — `save_snapshot` function (line 2253): how the
   three-file payload is built and enqueued to the async `SnapshotWriter`. Also read
   `update_gate_thresholds` (around line 3493): this is the separate code path that writes
   `gate_thresholds.json` outside the snapshot group. Read `restore_state_from_resume_snapshot`
   (line 2323) to understand what fields must be present after loading.
3. `crates/roko-cli/src/runner/snapshot_writer.rs` — `SnapshotPayload` struct (current three
   fields), `write_all_files`, the background writer thread. The redesign replaces
   `SnapshotPayload` with a single serialized blob.
4. `crates/roko-runtime/src/lib.rs` — what is currently exported from `roko-runtime`. The new
   `state_snapshot.rs` module goes here.
5. `crates/roko-cli/src/runner/resume.rs` — the resume path that calls `load_run_state` and
   `verify_checkpoint`. Must be updated to load `StateSnapshot` and validate its checksum
   instead.

## What to Change

### 1. Create `StateSnapshot` in `crates/roko-runtime/src/state_snapshot.rs`

This is a new file. Create it:

```rust
//! Single-file, checksummed orchestration state snapshot.
//!
//! All four mutable state groups (executor, orchestrator, run counters, gate thresholds)
//! are serialized into this struct and written atomically in one `atomic_write` call.
//! The `checksum` field is a SHA-256 hex digest of the JSON-serialized inner payloads
//! (computed before they are embedded, not over the outer document).

use serde::{Deserialize, Serialize};

/// Bump this constant whenever the shape of `StateSnapshot` changes in an incompatible way.
/// Resume code must reject snapshots with a different version.
pub const STATE_SNAPSHOT_VERSION: u32 = 1;

/// All runtime state groups bundled for a single atomic write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// Schema version — compared against `STATE_SNAPSHOT_VERSION` on load.
    pub version: u32,
    /// Wall-clock timestamp of this snapshot (milliseconds since Unix epoch).
    pub timestamp_ms: u64,
    /// Executor snapshot JSON (opaque to roko-runtime; owned by roko-orchestrator).
    pub executor_json: String,
    /// Orchestrator snapshot JSON (opaque; includes merge queue).
    pub orchestrator_json: String,
    /// Run-state counters JSON.
    pub run_state_json: String,
    /// Gate threshold EMA state JSON.
    pub gate_thresholds_json: String,
    /// SHA-256 hex digest of the concatenation
    /// `executor_json || orchestrator_json || run_state_json || gate_thresholds_json`
    /// computed at save time. Validated at load time before any field is consumed.
    pub checksum: String,
}

impl StateSnapshot {
    /// Construct and checksum a new snapshot from its constituent serialized pieces.
    pub fn new(
        timestamp_ms: u64,
        executor_json: String,
        orchestrator_json: String,
        run_state_json: String,
        gate_thresholds_json: String,
    ) -> Self {
        let checksum = compute_checksum(
            &executor_json,
            &orchestrator_json,
            &run_state_json,
            &gate_thresholds_json,
        );
        Self {
            version: STATE_SNAPSHOT_VERSION,
            timestamp_ms,
            executor_json,
            orchestrator_json,
            run_state_json,
            gate_thresholds_json,
            checksum,
        }
    }

    /// Validate the embedded checksum. Returns `Err` with a descriptive message on mismatch.
    pub fn verify(&self) -> Result<(), String> {
        if self.version != STATE_SNAPSHOT_VERSION {
            return Err(format!(
                "state snapshot version mismatch: file has {}, code expects {}",
                self.version, STATE_SNAPSHOT_VERSION
            ));
        }
        let expected = compute_checksum(
            &self.executor_json,
            &self.orchestrator_json,
            &self.run_state_json,
            &self.gate_thresholds_json,
        );
        if expected != self.checksum {
            return Err(format!(
                "state snapshot checksum mismatch: stored {}, computed {expected}",
                self.checksum
            ));
        }
        Ok(())
    }
}

fn compute_checksum(
    executor: &str,
    orchestrator: &str,
    run_state: &str,
    gate_thresholds: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(executor.as_bytes());
    hasher.update(orchestrator.as_bytes());
    hasher.update(run_state.as_bytes());
    hasher.update(gate_thresholds.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

Check whether `sha2` is already a workspace dependency (`grep -rn 'sha2' Cargo.toml
crates/*/Cargo.toml`). If not, add it to the workspace `Cargo.toml` and to
`crates/roko-runtime/Cargo.toml`. If `sha2` is not available, use the `hex`-encoded output of
`std::collections::hash_map::DefaultHasher` as a fallback only — but prefer `sha2` for
collision resistance. If `ring` is already a dep, use `ring::digest::SHA256` instead of `sha2`.

Export `StateSnapshot` and `STATE_SNAPSHOT_VERSION` from `crates/roko-runtime/src/lib.rs`.

### 2. Add `StateSnapshot` save/load helpers to `crates/roko-cli/src/runner/persist.rs`

Add a `state_snapshot_json` path to `PersistPaths` that resolves to
`.roko/state/state-snapshot.json`. Also add two helpers:

```rust
/// Serialize and atomically write a [`StateSnapshot`] to disk.
pub fn save_state_snapshot(paths: &PersistPaths, snapshot: &StateSnapshot) -> Result<()> {
    let json = serde_json::to_vec_pretty(snapshot)?;
    atomic_write(&paths.state_snapshot_json, &json)
}

/// Load a [`StateSnapshot`] from disk and validate its checksum.
/// Returns `None` if the file does not exist.
/// Returns `Err` if the file exists but is corrupt or the checksum fails.
pub fn load_state_snapshot(paths: &PersistPaths) -> Result<Option<StateSnapshot>> {
    let path = &paths.state_snapshot_json;
    if !path.exists() {
        return Ok(None);
    }
    let json = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let snapshot: StateSnapshot = serde_json::from_str(&json)
        .with_context(|| format!("parsing {}", path.display()))?;
    snapshot.verify().map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(Some(snapshot))
}
```

### 3. Replace `save_snapshot` in `event_loop.rs` with a single-file write

Rewrite `save_snapshot` to build a `StateSnapshot` instead of a `SnapshotPayload`:

```rust
fn save_snapshot(
    config: &RunConfig,
    executor: &ParallelExecutor,
    paths: &PersistPaths,
    state: &mut RunState,
    merge_queue: &MergeQueue,
    gate_thresholds: &GateThresholds,
    writer: &SnapshotWriter,
) {
    let timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;

    // Serialize all four pieces
    let executor_json = match serde_json::to_string_pretty(&executor.snapshot(timestamp_ms)) { ... };
    let orchestrator_json = match OrchestratorSnapshot::new(...).to_json() { ... };
    let run_state_json = match serde_json::to_string_pretty(&build_run_state_snapshot(...)) { ... };
    let gate_thresholds_json = match serde_json::to_string_pretty(gate_thresholds) { ... };

    let snapshot = StateSnapshot::new(
        timestamp_ms,
        executor_json,
        orchestrator_json,
        run_state_json,
        gate_thresholds_json,
    );

    let snapshot_json = match serde_json::to_vec_pretty(&snapshot) { ... };

    writer.write(SnapshotPayload {
        snapshot_json,
        snapshot_path: paths.state_snapshot_json.clone(),
    });
}
```

Add `gate_thresholds: &GateThresholds` to the function signature and update all call sites.

### 4. Simplify `SnapshotPayload` and `SnapshotWriter`

`SnapshotPayload` shrinks to two fields (the single JSON blob and its path). Update
`write_all_files` in `snapshot_writer.rs` to write only one file:

```rust
fn write_all_files(payload: &SnapshotPayload) -> anyhow::Result<()> {
    atomic_write(&payload.snapshot_path, &payload.snapshot_json)?;
    Ok(())
}
```

Remove the `write_checkpoint` call — the checksum is now embedded in the JSON itself.

### 5. Update resume to load `StateSnapshot` and validate checksum

In `event_loop.rs` and `resume.rs`, replace calls to `load_run_state` + `verify_checkpoint`
with `load_state_snapshot`. After loading, call `snapshot.verify()` (already called inside
`load_state_snapshot`, but log the outcome explicitly):

```rust
match persist::load_state_snapshot(&paths)? {
    Some(snapshot) => {
        tracing::info!(
            timestamp_ms = snapshot.timestamp_ms,
            "loaded state snapshot — checksum valid"
        );
        // Deserialize each inner piece and restore state
        let run_state: RunStateSnapshot = serde_json::from_str(&snapshot.run_state_json)?;
        let gate_thresholds: GateThresholds = serde_json::from_str(&snapshot.gate_thresholds_json)?;
        // ... restore executor from snapshot.executor_json ...
    }
    None => {
        tracing::info!("no state snapshot found — starting fresh");
    }
}
```

### 6. Remove the separate `update_gate_thresholds` disk write

`update_gate_thresholds` in `event_loop.rs` currently calls `GateThresholds::save` (a direct
disk write outside the snapshot group). Remove that call. Gate thresholds are now persisted
only through `save_snapshot`. Update `update_gate_thresholds` to mutate in-memory state only
and remove any path parameter.

### 7. Keep the old individual files for one release as legacy fallback

On resume, if `state_snapshot_json` does not exist but `run_state_json` does, log a warning
and fall back to loading from the old three-file format. This prevents breaking existing
sessions after the upgrade. Remove the fallback in the next task wave.

## What NOT to Do

- Do NOT write a new file for every state group. The entire point is one file, one write.
- Do NOT keep the three-file `SnapshotPayload` alongside the new `StateSnapshot` — replace it.
- Do NOT compute the checksum outside `StateSnapshot::new`. The checksum must always be
  computed from the same four fields in the same order.
- Do NOT block the event loop on the serialization — keep the same pattern of serializing on
  the event loop thread and dispatching the bytes to the background writer thread.
- Do NOT add `StateSnapshot` to `roko-cli`. It belongs in `roko-runtime` so any future
  consumer (agent-server, serve) can deserialize a snapshot without taking a `roko-cli` dep.
- Do NOT remove backward-compatible resume support without adding the legacy fallback (step 7).

## Wire Target

```bash
# Run a plan and kill it mid-execution
cargo run -p roko-cli -- plan run plans/ &
sleep 10 && kill %1

# Verify a single unified snapshot file was written
ls -la .roko/state/
# Expected: state-snapshot.json present; executor.json / run-state.json also present
#           (old files still written via legacy path — confirm state-snapshot.json is primary)

cat .roko/state/state-snapshot.json | python3 -m json.tool | grep -E '"version"|"checksum"'
# Expected: "version": 1, "checksum": "<64-char hex>"

# Resume must succeed and log "checksum valid"
cargo run -p roko-cli -- plan run plans/ --resume .roko/state/run-state.json 2>&1 | grep checksum
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `StateSnapshot` struct exists in `crates/roko-runtime/src/state_snapshot.rs`
- [ ] `StateSnapshot::verify()` returns `Err` when checksum is corrupted (unit test)
- [ ] `save_state_snapshot` and `load_state_snapshot` exist in `persist.rs`
- [ ] `save_snapshot` in `event_loop.rs` passes `gate_thresholds` and builds `StateSnapshot`
- [ ] `update_gate_thresholds` no longer calls `GateThresholds::save` or takes a path param
- [ ] `SnapshotPayload` in `snapshot_writer.rs` has a single blob field
- [ ] `write_all_files` calls `atomic_write` exactly once
- [ ] Resume path calls `load_state_snapshot` and logs checksum validation result
- [ ] Legacy fallback: if `state-snapshot.json` absent but `run-state.json` present, resume still works
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file touched by this task

## Implementation Ground Truth (Worker 18 Enrichment)

Active persistence today:

- `PersistPaths::from_workdir()` in `runner/persist.rs` derives `.roko/state/executor.json`, `.roko/state/orchestrator.json`, `.roko/state/run-state.json`, and `.roko/learn/gate-thresholds.json` from `RokoLayout`. `RokoLayout` has no `state_snapshot_path()` helper; add `state_snapshot_json: state.join("state-snapshot.json")` directly in `PersistPaths` unless task scope is expanded to `roko-fs`.
- `save_snapshot()` in `runner/event_loop.rs` serializes executor, orchestrator, and `RunStateSnapshot`, then enqueues a three-file `SnapshotPayload`.
- `runner/snapshot_writer.rs::write_all_files()` writes the three files and then writes `checkpoint.txt` with FNV hashes. This file must change for the one-file design, but it is not currently listed in `touches`.
- Resume validation starts in `runner/event_loop.rs` by calling `persist::load_run_state()` to seed fingerprints, then `resume::prepare_resume_with_force()` calls `load_run_state()` again. It also verifies `checkpoint.txt`. Both paths must move to `load_state_snapshot()` with legacy fallback.
- `update_gate_thresholds()` in `runner/event_loop.rs` currently mutates and immediately saves `.roko/learn/gate-thresholds.json`. This is the direct out-of-group write this task removes.
- `sha2` is already a workspace dependency in root `Cargo.toml`; `roko-runtime` does not currently depend on it. Add `sha2 = { workspace = true }` to `crates/roko-runtime/Cargo.toml`, not a new workspace version.

## Mechanical Implementation Steps (Worker 18 Enrichment)

1. Add runtime snapshot type.
   - Create `crates/roko-runtime/src/state_snapshot.rs` with `StateSnapshot`, `STATE_SNAPSHOT_VERSION`, `StateSnapshot::new()`, and `verify()`.
   - Export it from `roko-runtime/src/lib.rs`.
   - Unit test checksum success, checksum mismatch, and version mismatch in `roko-runtime`.

2. Add single-file persistence helpers.
   - In `runner/persist.rs`, add `state_snapshot_json` to `PersistPaths`.
   - Add `save_state_snapshot()` and `load_state_snapshot()` exactly as one-file helpers.
   - Keep `load_run_state()` and old path fields for legacy fallback only; new writes should not call `save_run_state()`, `save_executor_snapshot()`, or `save_orchestrator_snapshot()`.

3. Collapse `SnapshotPayload`.
   - In `runner/snapshot_writer.rs`, replace the three JSON/path pairs with `snapshot_json: Vec<u8>` and `snapshot_path: PathBuf`.
   - Replace `write_all_files()` with one `atomic_write(&payload.snapshot_path, &payload.snapshot_json)`.
   - Remove `write_checkpoint()` calls from the writer. Leave the helper in `persist.rs` only if legacy tests still need it, but new resume must not depend on `checkpoint.txt`.

4. Rewrite `save_snapshot()`.
   - Add `gate_thresholds: &GateThresholds` to the function signature and every call site. Current call sites include the main select loop around lines 655, 723, 1150, 1209, 1239, 1266, helper functions around 1449/1560, completion paths around 3399/3457/3472, and finalization around 3778.
   - Serialize the executor snapshot, orchestrator snapshot, `RunStateSnapshot`, and `gate_thresholds` to strings.
   - Build `StateSnapshot::new(timestamp_ms, executor_json, orchestrator_json, run_state_json, gate_thresholds_json)`.
   - Serialize that outer snapshot to bytes and enqueue one payload to `paths.state_snapshot_json`.

5. Move resume to `StateSnapshot`.
   - In `event_loop.rs`, replace the initial `persist::load_run_state()` with `persist::load_state_snapshot()`. If present, parse `snapshot.run_state_json` into `RunStateSnapshot` for fingerprints and cascade-router JSON.
   - In `resume.rs`, replace direct `load_run_state()` use with a helper that first reads `StateSnapshot`, verifies checksum, deserializes `run_state_json`, and falls back to old `run-state.json` only when `state-snapshot.json` is absent.
   - Remove checkpoint verification from the new path. If the legacy fallback loads old files, it may still verify `checkpoint.txt` and warn.
   - Update `load_executor()` or its caller so resume can restore executor/orchestrator from `snapshot.executor_json`/`snapshot.orchestrator_json` when the unified snapshot exists, instead of requiring `executor.json`.

6. Remove the out-of-group gate-threshold write.
   - Change `update_gate_thresholds(thresholds, path, rung, passed)` to `update_gate_thresholds(thresholds, rung, passed)`.
   - Delete the `thresholds.save(path)` call and its warning block.
   - Ensure every place that calls `update_gate_thresholds()` is followed by a `save_snapshot(..., &gate_thresholds, ...)` before the event loop can exit normally. The existing completion path already saves snapshots; verify the order.

## Verification Corrections (Worker 18 Enrichment)

The current Wire Target text says old files should still be written. That conflicts with this task's single-file design. For a clean verification, remove `.roko/state/` before running, then assert:

```bash
test -f .roko/state/state-snapshot.json
test ! -f .roko/state/run-state.json
test ! -f .roko/state/orchestrator.json
test ! -f .roko/state/executor.json
```

Legacy fallback should be tested separately by creating only the old files from a fixture and confirming resume still works. Do not use a fresh new-format run to prove legacy fallback.

## Scope Notes (Worker 18 Enrichment)

The current `touches` list is incomplete. A working implementation must edit `crates/roko-cli/src/runner/snapshot_writer.rs`, `crates/roko-cli/src/runner/resume.rs`, `crates/roko-runtime/src/lib.rs`, and `crates/roko-runtime/Cargo.toml` in addition to the listed files. If the implementation chooses to add a `RokoLayout::state_snapshot_path()` helper, it must also include `crates/roko-fs/src/layout.rs`; otherwise keep the new path local to `PersistPaths`.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
