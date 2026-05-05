# Task 092: Write-Ahead Log for Learning State

```toml
id = 92
title = "Add WalWriter to roko-learn and wire WAL appends into CascadeRouter, ExperimentStore, and gate threshold updates"
track = "infrastructure"
wave = "wave-3"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-learn/src/wal.rs",
    "crates/roko-learn/src/cascade_router.rs",
    "crates/roko-learn/src/model_experiment.rs",
    "crates/roko-learn/src/runtime_feedback.rs",
    "crates/roko-learn/src/lib.rs",
    "crates/roko-core/src/defaults.rs",
]
exclusive_files = [
    "crates/roko-learn/src/wal.rs",
    "crates/roko-learn/src/runtime_feedback.rs",
]
estimated_minutes = 300
```

## Context

S16.7 + S19.1: Cascade router observations, experiment outcomes, and gate thresholds
accumulate in memory and are only saved on clean shutdown or task completion. A crash
mid-run loses the entire session's learning — LinUCB weights, confidence stats, experiment
trial counts, and gate EMA values all revert to the last checkpoint.

The three affected systems and their current save points:

- **`CascadeRouter`** (`crates/roko-learn/src/cascade_router.rs:1654`) — `save()` writes a
  full JSON snapshot to `.roko/learn/cascade-router.json`. The primary trigger is
  `LearningRuntime::save_cascade_router()` in `runtime_feedback.rs`, called after each
  conductor intervention and at the end of each task. A crash mid-task loses all observations
  accumulated since the last per-task checkpoint.

- **`ModelExperimentStore`** (`crates/roko-learn/src/model_experiment.rs:235`) — `save()` is
  called from `runtime_feedback.rs` only when a trial concludes. In-progress trial outcomes
  accumulated during a run are lost entirely on crash.

- **Gate thresholds** (`crates/roko-gate/src/adaptive_threshold.rs`) — `AdaptiveThresholds::save()`
  is called from `orchestrate.rs` at the end of each section (task completion). A crash
  between task start and completion loses all rung EMA updates recorded during that task.

This task introduces a synchronous WAL (Write-Ahead Log) in JSONL format. Each learning
event is appended to `.roko/learn/wal.jsonl` with `sync_data()` immediately after the
in-memory state is updated. On the next startup, `LearningRuntime` replays the WAL to
reconstruct the in-memory state, then saves snapshots and truncates the WAL.

This is a redesign, not a band-aid. The WAL is the durable source of truth between
snapshots. Existing periodic snapshot saves are supplementary checkpoints — they continue
to run but are no longer the sole durability mechanism.

## Background

Read these files before writing any code:

1. **`crates/roko-learn/src/cascade_router.rs`** — `CascadeRouter::save()` (line ~1654,
   synchronous, writes JSON with atomic rename), `record_observation()` (line ~994,
   the main entry point for routing feedback), and `observe_internal()` (line ~1220,
   updates `confidence_stats` mutex + LinUCB). The WAL must capture enough data from
   `record_observation()` to replay the update.

2. **`crates/roko-learn/src/cascade/persistence.rs`** — `CascadeSnapshot`, `PersistedModelStats`,
   `LinUCBSnapshot`. The WAL does NOT store the full snapshot — it stores the per-observation
   inputs sufficient to replay the update (`model_slug`, reward, success, context features).

3. **`crates/roko-learn/src/model_experiment.rs`** — `ModelExperiment`, `ModelVariantStats`,
   `ModelExperimentStore`. Find `record_outcome()` and `save()`. Experiment trial outcomes
   are the WAL entries.

4. **`crates/roko-learn/src/runtime_feedback.rs`** — `LearningRuntime` struct, `LearningPaths`
   (has `gate_thresholds_json`, `cascade_router_json` fields at lines ~162-162),
   `save_cascade_router()` (line ~1990), `record_completed_run()`. The `LearningRuntime`
   is the integration point for WAL construction and replay.

5. **`crates/roko-cli/src/runner/persist.rs`** — `append_jsonl()` and `atomic_write()`.
   These are the durability patterns in the executor snapshot subsystem. The WAL follows
   the same approach: JSONL append for incremental events, atomic rename for snapshots.

6. **`crates/roko-core/src/defaults.rs`** — add `DEFAULT_LEARN_WAL_MAX_ENTRIES` here.
   Check existing constants for naming convention before adding.

7. **`crates/roko-learn/src/lib.rs`** — the module list. Add `pub mod wal;` here.

Verify that `crates/roko-gate` does NOT import from `roko-learn`:
```bash
grep -rn 'roko.learn\|roko_learn' crates/roko-gate/Cargo.toml crates/roko-gate/src/ --include='*.rs'
```
This confirms the gate threshold WAL write must be routed through `orchestrate.rs` or
`LearningRuntime`, not through `AdaptiveThresholds` itself.

## What to Change

### 1. Create `crates/roko-learn/src/wal.rs`

This is a new file. Do not add it to an existing file.

```rust
//! Synchronous Write-Ahead Log for learning state durability (S16.7, S19.1).
//!
//! WAL lives at `.roko/learn/wal.jsonl`. Each line is a JSON-serialized
//! [`WalEntry`]. Entries are appended with `sync_data()` for durability.
//! After a successful snapshot save, the WAL is truncated to zero.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A single durable learning event.
///
/// Each variant carries exactly the fields needed to replay the in-memory
/// update — not the full snapshot. This keeps WAL entries small (~100–400
/// bytes each) while preserving full replay fidelity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WalEntry {
    /// A cascade router observation: confidence stats + LinUCB arm update.
    CascadeObservation {
        model_slug: String,
        /// Context feature vector passed to LinUCB at routing time.
        context_features: Vec<f64>,
        /// Index into the router's model slug list.
        model_idx: usize,
        /// Scalar reward computed from gate outcome.
        reward: f64,
        /// Whether the task gated successfully.
        success: bool,
        ts_ms: i64,
    },
    /// A model experiment trial outcome.
    ExperimentOutcome {
        experiment_id: String,
        variant_id: String,
        success: bool,
        /// Optional scalar metric (cost, latency) for the trial.
        metric: Option<f64>,
        ts_ms: i64,
    },
    /// A gate rung adaptive threshold EMA update.
    GateThresholdUpdate {
        /// Gate rung index (0-based).
        rung: u32,
        /// Whether the rung passed.
        passed: bool,
        ts_ms: i64,
    },
}

/// Append-only WAL writer backed by a plain `File`.
///
/// Writes are synchronous and call `sync_data()` after each append to
/// guarantee durability. Do NOT wrap in `BufWriter` — buffering would
/// defeat the crash-safety guarantee.
pub struct WalWriter {
    path: PathBuf,
    file: File,
    entry_count: usize,
}

impl WalWriter {
    /// Open the WAL at `path`, creating it if absent.
    ///
    /// Returns both the writer and the existing entry count (so the
    /// caller can decide whether to replay before proceeding).
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Count existing entries before opening in append mode.
        let entry_count = if path.exists() {
            BufReader::new(File::open(path)?).lines().count()
        } else {
            0
        };
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            entry_count,
        })
    }

    /// Append `entry` to the WAL, flushing to the OS page cache and
    /// syncing the data to durable storage before returning.
    pub fn append(&mut self, entry: &WalEntry) -> io::Result<()> {
        let mut line = serde_json::to_string(entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data()?;
        self.entry_count += 1;
        Ok(())
    }

    /// Number of entries written since the WAL was last truncated.
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// Truncate the WAL to zero after a successful snapshot save.
    ///
    /// Reopens the file with `O_TRUNC` so that subsequent appends start
    /// from an empty file. `entry_count` resets to zero.
    pub fn truncate(&mut self) -> io::Result<()> {
        // Close the append-mode handle, open truncate, then reopen append.
        let _ = std::mem::replace(
            &mut self.file,
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)?,
        );
        self.file = OpenOptions::new().append(true).open(&self.path)?;
        self.entry_count = 0;
        Ok(())
    }
}

/// Read and deserialize all entries from `path`.
///
/// Malformed lines are logged as warnings and skipped rather than
/// returning an error, so that a partially-written tail entry (from a
/// crash during `write_all`) does not block replay.
///
/// Returns an empty `Vec` if the WAL file does not exist.
pub fn replay_wal(path: &Path) -> io::Result<Vec<WalEntry>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let mut entries = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<WalEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!(line = i, error = %e, "[wal] skipping malformed entry");
            }
        }
    }
    Ok(entries)
}
```

Wire the new module: add `pub mod wal;` to `crates/roko-learn/src/lib.rs`.

### 2. Add `DEFAULT_LEARN_WAL_MAX_ENTRIES` to `crates/roko-core/src/defaults.rs`

```rust
/// Maximum WAL entries before an automatic compaction (snapshot + truncate).
/// Each entry is ~100–400 bytes; at 10_000 entries the WAL is at most ~4 MB.
pub const DEFAULT_LEARN_WAL_MAX_ENTRIES: usize = 10_000;
```

### 3. Wire WAL into `LearningRuntime`

In `crates/roko-learn/src/runtime_feedback.rs`:

**a) Add WAL field and path to `LearningPaths`**:
```rust
// In LearningPaths:
pub wal_jsonl: PathBuf,

// In LearningPaths::new() / from_root():
wal_jsonl: root.join("wal.jsonl"),
```

**b) Add WAL writer field to `LearningRuntime`**:
```rust
// Use parking_lot::Mutex (sync) — WAL writes are never held across .await.
wal: Option<parking_lot::Mutex<WalWriter>>,
```

**c) In `LearningRuntime` constructor / `open()`**:
1. Before any in-memory state initialization, call `replay_wal(&paths.wal_jsonl)`.
2. For each replayed entry, apply it to in-memory state using internal
   no-WAL-write variants (see step d below).
3. After replay, call `save_cascade_router()`, `experiment_store.save()`, and truncate
   the WAL — this promotes replayed state to durable snapshots immediately.
4. Open the `WalWriter`: `WalWriter::open(&paths.wal_jsonl)`.

**d) Internal replay methods**:
Add `_replay` variants to `CascadeRouter` and `ModelExperimentStore` that apply an update
without writing a WAL entry (to avoid re-logging during replay):

```rust
// In CascadeRouter:
/// Apply a WAL-replayed observation. Does NOT write a WAL entry.
pub fn replay_observation(&self, slug: &str, reward: f64, success: bool,
                           context: &[f64], model_idx: usize) {
    // Call observe_internal directly, bypassing any WAL hook.
}

// In ModelExperimentStore:
/// Apply a WAL-replayed trial outcome. Does NOT write a WAL entry.
pub fn replay_outcome(&mut self, experiment_id: &str, variant_id: &str,
                       success: bool, metric: Option<f64>) { ... }
```

**e) Add `wal_append()` helper to `LearningRuntime`**:
```rust
fn wal_append(&self, entry: WalEntry) {
    let Some(ref wal) = self.wal else { return };
    let mut w = wal.lock();
    if let Err(e) = w.append(&entry) {
        tracing::warn!(error = %e, "[wal] append failed — learning not durable this entry");
        return;
    }
    if w.entry_count() >= roko_core::defaults::DEFAULT_LEARN_WAL_MAX_ENTRIES {
        self.compact_wal_locked(&mut w);
    }
}

fn compact_wal_locked(&self, wal: &mut WalWriter) {
    // Save snapshots first, then truncate.
    let _ = self.save_cascade_router();
    if let Err(e) = self.experiment_store.save(&self.paths.experiments_json) {
        tracing::warn!(error = %e, "[wal] experiment snapshot failed during compaction");
    }
    if let Err(e) = wal.truncate() {
        tracing::warn!(error = %e, "[wal] truncate failed during compaction");
    }
}
```

**f) Add WAL appends at the three event sites**:

After `record_observation()` / `observe_internal()` updates in-memory state:
```rust
self.wal_append(WalEntry::CascadeObservation {
    model_slug: slug.to_string(),
    context_features: context.to_vec(),
    model_idx,
    reward,
    success,
    ts_ms: chrono::Utc::now().timestamp_millis(),
});
```

After `record_outcome()` updates experiment in-memory state:
```rust
self.wal_append(WalEntry::ExperimentOutcome {
    experiment_id: experiment_id.to_string(),
    variant_id: variant_id.to_string(),
    success,
    metric,
    ts_ms: chrono::Utc::now().timestamp_millis(),
});
```

For gate threshold updates — these originate in `orchestrate.rs` which calls
`adaptive_thresholds.update(rung, passed)`. Add a wrapper in `LearningRuntime`
(or wherever `AdaptiveThresholds` is held in the runtime) that calls update then
`wal_append`. Do NOT add WAL writes inside `roko-gate` — that crate must not depend
on `roko-learn`.

**g) Truncate WAL after successful snapshot saves**:
```rust
// In save_cascade_router(), after successful router.save():
if let Some(ref wal) = self.wal {
    let mut w = wal.lock();
    if let Err(e) = w.truncate() {
        tracing::warn!(error = %e, "[wal] truncate after cascade-router save failed");
    }
}
```

Same after `experiment_store.save()` succeeds.

### 4. Update `crates/roko-learn/src/lib.rs`

Add to the module list:
```rust
/// Write-Ahead Log for crash-safe learning state persistence.
pub mod wal;
```

## What NOT to Do

- Do NOT replace the existing periodic `save_cascade_router()` call — the WAL supplements
  it. The periodic save remains the primary durability path; the WAL fills the gap between
  periodic saves.
- Do NOT WAL-write the full `CascadeSnapshot` (with all confidence stats) on every
  observation — it would be 10–100 KB per entry. Only write the per-observation inputs
  needed to replay the update.
- Do NOT add WAL writes inside `crates/roko-gate/src/adaptive_threshold.rs` —
  `roko-gate` must not depend on `roko-learn`. Route gate threshold WAL writes through
  `orchestrate.rs` or `LearningRuntime`.
- Do NOT make WAL writes async — WAL writes are synchronous and must complete in
  microseconds. Using a sync `File` with `sync_data()` is correct and required.
- Do NOT use `BufWriter` — buffering defeats the crash-safety guarantee.
- Do NOT change existing snapshot file paths (`.roko/learn/cascade-router.json`, etc.) —
  the WAL is additive, not a replacement.
- Do NOT add the WAL to `roko-gate` or `roko-core` — it belongs entirely in `roko-learn`.
- Do NOT WAL-replay during normal `record_observation()` calls (only during startup).
  The `_replay` internal methods must NOT call `wal_append()`.

## Wire Target

```bash
# Build
cargo build --workspace

# Run a plan, kill mid-way, inspect WAL, resume
cargo run -p roko-cli -- plan run plans/ &
PID=$!
sleep 20
kill -9 $PID

# WAL should have entries
cat .roko/learn/wal.jsonl | wc -l   # expect > 0

# Resume — WAL should be replayed and truncated
cargo run -p roko-cli -- plan run plans/ --resume .roko/state/executor.json

# After clean completion, WAL should be empty
cat .roko/learn/wal.jsonl | wc -l   # expect 0

# Verify WAL kinds are what we expect
cat .roko/learn/wal.jsonl | python3 -c "
import sys, json
for line in sys.stdin:
    e = json.loads(line)
    print(e['kind'])
" | sort | uniq -c
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'WalEntry\|WalWriter\|replay_wal' crates/roko-gate/ --include='*.rs'` — returns nothing
- [ ] `grep -rn 'pub mod wal' crates/roko-learn/src/lib.rs` — present
- [ ] WAL file created at `.roko/learn/wal.jsonl` when `roko plan run` starts
- [ ] WAL file has zero entries after a clean run completes (truncated after snapshots)
- [ ] Unit test: `replay_wal` correctly reconstructs cascade router observation count
      from a WAL with 10 `CascadeObservation` entries
- [ ] Unit test: `replay_wal` with a malformed tail line logs a warning and skips it
- [ ] Unit test: `WalWriter::truncate()` resets file to empty and `entry_count` to 0
- [ ] Unit test: `wal_append` triggers compaction when entry count reaches
      `DEFAULT_LEARN_WAL_MAX_ENTRIES`, calling `save_cascade_router()` and `truncate()`
- [ ] `grep -rn 'roko_learn\|roko-learn' crates/roko-gate/Cargo.toml crates/roko-gate/src/'`
      — no new dependency added
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in new or changed files

## Implementation Ground Truth (Worker 18 Enrichment)

The current learning persistence surface has moved since the source docs were written. Use these concrete call sites:

- `LearningPaths` is in `crates/roko-learn/src/runtime_feedback.rs` and is constructed by `LearningPaths::under(root)`. Add `wal_jsonl: PathBuf` there as `root.join("wal.jsonl")`.
- `LearningRuntime::open()` and `LearningRuntime::open_with_models()` both duplicate initialization. WAL replay must happen in both paths or, preferably, after extracting the shared construction logic into a private helper so replay behavior cannot drift.
- `CascadeRouter::record_observation()` currently returns `()` and calls private `observe_internal(&ctx.to_features(), model_idx, reward, success, None, None)`. Add a public `replay_observation(&self, model_slug, context_features, model_idx, reward, success)` that validates `model_idx` still maps to `model_slug` and then calls `observe_internal` without writing another WAL entry.
- The normal `LearningRuntime` cascade update path is `LearningRuntime::record_completed_run()` -> `update_cascade_router()` -> `cascade_router.record_observation(...)`. `record_conductor_intervention()` is a second in-runtime cascade write path. Both need WAL append after the in-memory mutation succeeds.
- There are direct cascade-router mutation paths outside this task's current `touches`: `crates/roko-cli/src/orchestrate.rs` calls `self.learning.cascade_router().observe_multi_objective(...)` and `record_override_outcome(...)` around the task-success learning path. A WAL that only hooks `LearningRuntime::update_cascade_router()` will not cover those direct observations.
- `ModelExperimentStore` is persisted in two places: `LearningRuntime::record_completed_run()` locks `self.experiment_store` and calls `store.record_outcome(...)`, while `crates/roko-cli/src/orchestrate.rs::record_model_experiment_outcome()` loads `ModelExperimentStore` directly, records, and saves. Covering only the runtime path leaves direct plan experiment outcomes non-WAL durable.
- Gate thresholds in the runner currently use `crates/roko-cli/src/runner/persist.rs::GateThresholds`, loaded in `runner/event_loop.rs` and mutated by `update_gate_thresholds(thresholds, &paths.gate_thresholds_json, rung, passed)`. They are not `crates/roko-gate/src/adaptive_threshold.rs` in the active plan-run path.
- `grep -rn 'roko.learn\|roko_learn' crates/roko-gate/Cargo.toml crates/roko-gate/src/ --include='*.rs'` returns no matches. Keep it that way.

## Mechanical Implementation Steps (Worker 18 Enrichment)

1. Add `wal.rs` and module export.
   - Put `WalEntry`, `WalWriter`, and `replay_wal()` exactly in `crates/roko-learn/src/wal.rs`.
   - Add `pub mod wal;` in `crates/roko-learn/src/lib.rs` near the other persistence modules.
   - Use `std::fs::OpenOptions`, newline-delimited JSON, `write_all`, and `sync_data`. Keep malformed-tail replay lossy and warning-only.

2. Add runtime ownership of the writer.
   - Add `wal_jsonl` to `LearningPaths`.
   - Add `wal: Option<parking_lot::Mutex<WalWriter>>` to `LearningRuntime`.
   - During open, create parent dirs, replay existing entries before accepting new records, apply entries through replay-only methods, save snapshots, then truncate/open the writer. Do not append WAL while replaying.

3. Define replay APIs on the state owners.
   - `CascadeRouter::replay_observation(...)` should call `observe_internal` and should no-op with a `tracing::warn!` if the slug/index pair is invalid after config changes.
   - `ModelExperimentStore::replay_outcome(...)` must use the full active outcome shape. The current store method requires `experiment_id`, `variant_id`, `success`, `cost`, `tokens`, and `duration`; the WAL entry in this spec's pseudocode is missing `tokens` and `duration_ms`. Add those fields or the replay cannot reproduce stats exactly.
   - Gate threshold replay cannot live in `roko-gate`. If the active threshold type remains `runner::persist::GateThresholds`, replay support belongs in `runner/persist.rs` or must be scoped out.

4. Append WAL entries immediately after in-memory mutation.
   - Cascade runtime path: in `update_cascade_router()`, compute `context_features = ctx.to_features()` before `record_observation()`, resolve `model_idx`, call the router, then append `WalEntry::CascadeObservation`.
   - Conductor intervention path: `record_conductor_intervention()` uses reward `0.0` and `success = false`; append the same cascade WAL entry after `record_observation()`.
   - Experiment runtime path: after `store.record_outcome(...)` in `record_completed_run()`, append `WalEntry::ExperimentOutcome` with experiment id, variant id, success, cost, tokens, duration, and timestamp.
   - Gate threshold path: append `WalEntry::GateThresholdUpdate` after `GateThresholds::observe()` in the active runner code, but only after the task scope includes the runner files.

5. Compact safely.
   - A compaction may save the cascade router and experiment store, then truncate the WAL. Do not truncate after only one subsystem's snapshot if the WAL contains entries for multiple subsystems; otherwise a cascade save can delete pending experiment/gate entries. Track whether the replay/snapshot set is complete before truncation.
   - Keep existing snapshot saves; the WAL closes the crash window between snapshots.

## Required Tests (Worker 18 Enrichment)

- `wal.rs`: append two entries, assert JSONL kinds, reopen and assert `entry_count`, truncate and assert empty file/count zero.
- `wal.rs`: malformed final line is skipped and valid earlier lines replay.
- `cascade_router.rs`: replaying a `CascadeObservation` increments total observations/confidence exactly once and does not append to WAL.
- `model_experiment.rs`: replaying an `ExperimentOutcome` reproduces trials, successes, cost, token, and duration stats.
- `runtime_feedback.rs`: opening a runtime with a preexisting WAL replays, saves snapshots, and truncates the WAL.
- Crash-style integration: run a small learning operation, assert `.roko/learn/wal.jsonl` has entries before compaction, then reopen and assert snapshots include the entries and WAL is empty.

## Scope Notes (Worker 18 Enrichment)

The current `touches` list is not sufficient for the full stated task. Full durability for direct cascade observations, direct model experiment outcomes, and active runner gate threshold updates requires at least `crates/roko-cli/src/orchestrate.rs`, `crates/roko-cli/src/runner/event_loop.rs`, and/or `crates/roko-cli/src/runner/persist.rs`. Do not claim the task is fully wired unless the implementation scope is expanded or the Status Log explicitly narrows the task to `LearningRuntime`-owned events only.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
