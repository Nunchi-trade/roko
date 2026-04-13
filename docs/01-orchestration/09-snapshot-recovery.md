# Snapshot & Crash Recovery

> **Modules**: `roko-orchestrator/src/executor/snapshot.rs`,
> `roko-orchestrator/src/executor/recovery.rs`
> **Key types**: `ExecutorSnapshot`, `RecoveryEngine`, `RecoveredState`
> **Persistence path**: `.roko/state/executor.json`
> **CLI flag**: `--resume .roko/state/executor.json`


> **Implementation**: Shipping

---

## Overview

The Roko Orchestrator is designed to survive crashes. Long-running
multi-plan orchestration sessions (hours or days) must not lose progress
when a process dies, a machine restarts, or a network connection drops.

The recovery system provides two complementary mechanisms:

1. **Executor snapshots** — periodic point-in-time captures of the full
   executor state, serialized to `.roko/state/executor.json`
2. **Event log replay** — reconstruction of state from the append-only,
   hash-chained event log

These mechanisms can be used independently or merged for maximum fidelity.

---

## Executor Snapshots

### Structure

```rust
pub struct ExecutorSnapshot {
    /// Per-plan mutable state, keyed by plan_id.
    pub plan_states: HashMap<String, PlanState>,
    /// Queue order: plan_ids in execution priority order.
    pub queue_order: Vec<String>,
    /// Unix millisecond timestamp when the snapshot was taken.
    pub timestamp_ms: u64,
}
```

A snapshot captures everything the executor needs to resume:

- The phase of every plan (Queued, Implementing, Gating, etc.)
- Gate results accumulated so far
- Files modified by agents
- Iteration counts
- Pause states
- Priority assignments
- The execution queue order

### Serialization

Snapshots serialize to JSON via `to_json()` and deserialize via `from_json()`.
The JSON format is human-readable for debugging:

```json
{
  "plan_states": {
    "01-workspace": {
      "plan_id": "01-workspace",
      "current_phase": { "kind": "implementing" },
      "assigned_agents": ["impl-t1"],
      "gate_results": [],
      "iteration": 2,
      "started_at_ms": 1712345678000,
      "files_changed": ["crates/roko-core/src/lib.rs"],
      "merge_attempts": 0,
      "last_error": null,
      "paused": false,
      "priority": 0
    }
  },
  "queue_order": ["01-workspace", "02-core"],
  "timestamp_ms": 1712345690000
}
```

### Atomic writes

The runtime writes snapshots atomically to prevent corruption:

```
1. Write to .roko/state/executor.json.tmp
2. fsync the temp file
3. Rename .roko/state/executor.json.tmp → .roko/state/executor.json
```

If the process crashes during step 1 or 2, the temp file is left behind but
the original snapshot is intact. If it crashes during step 3, the rename is
atomic on POSIX systems — either the old snapshot or the new one is visible,
never a partial write.

### Auto-save frequency

The `PlanRunner` auto-saves every `AUTOSAVE_INTERVAL` (5) actions. This means:

- At most 5 actions of work can be lost in a crash
- A typical orchestration run with 100 actions produces ~20 snapshots
- Each snapshot overwrites the previous one (only the latest is kept on disk)

### Legacy compatibility

`ExecutorSnapshot::from_json()` handles legacy snapshot formats:

```rust
pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
    // Check for legacy "tasks" key
    if value.get("tasks").is_some() && value.get("plan_states").is_none() {
        return Self::from_legacy_json(json);
    }
    // Try current format
    serde_json::from_str(json)
}
```

The legacy format used a flat `tasks` array instead of the current
`plan_states` HashMap. The compat loader converts legacy task entries to
`PlanState` objects by grouping by plan ID and inferring phases from task
statuses.

---

## Event Log Replay

The event log (see `10-event-log.md`) provides an alternate recovery path.
Because every significant orchestration event is recorded in the log, the
entire executor state can be reconstructed by replaying events from the
beginning.

### RecoveryEngine

```rust
pub struct RecoveryEngine {
    _private: (),
}
```

The `RecoveryEngine` is stateless — it provides methods for recovery but
holds no state itself.

### Recovery from snapshot

```rust
pub fn recover_from_snapshot(
    &self,
    snapshot_json: &str,
) -> Result<RecoveredState, RecoveryError>
```

Deserializes the snapshot JSON, converts each `PlanState` to a
`PlanPhaseInfo`, and preserves the queue order. The last gate result summary
is extracted for diagnostic purposes.

### Recovery from event log

```rust
pub fn recover_from_event_log(
    &self,
    events: &[EventEntry],
) -> Result<RecoveredState, RecoveryError>
```

Replays events in sequence, building up per-plan state:

| Event Kind | Effect on PlanPhaseInfo |
|-----------|------------------------|
| `PlanStarted` | Create plan entry, set phase to `Enriching` |
| `PhaseTransition` | Update phase from payload |
| `TaskAssigned` / `AgentSpawned` | Add files to `files_changed` |
| `GateResult` | Update `last_gate_result` |
| `PlanCompleted` | Set phase to `Complete` |
| `PlanFailed` | Set phase to `Failed` with reason |
| `MergeAttempted` | Set phase to `Merging` (if not terminal) |

Iteration numbers are tracked from event payloads — the highest iteration
seen for a plan is preserved.

### Validation

Event log recovery validates monotonic sequence numbers:

```rust
for window in events.windows(2) {
    if window[1].sequence_number <= window[0].sequence_number {
        return Err(RecoveryError::InvalidEventSequence(...));
    }
}
```

Non-monotonic sequences indicate log corruption or tampering.

---

## Merged Recovery

When both a snapshot and event log are available, the `RecoveryEngine` merges
them:

```rust
pub fn merge_recovery(
    snapshot: Option<RecoveredState>,
    event_log: Option<RecoveredState>,
) -> RecoveredState
```

### Merge rules

1. **Event log wins on conflict**: If both sources have state for the same plan,
   the event log version is used. The event log is append-only and may contain
   events recorded after the snapshot.

2. **Disjoint plans are combined**: Plans that appear only in the snapshot or
   only in the event log are both included.

3. **Queue order**: Event log's queue order is preferred if non-empty; otherwise
   the snapshot's order is used.

4. **Sequence numbers**: The higher sequence number is taken.

This merge strategy ensures that:

- A snapshot that's 5 minutes stale gets updated by recent events
- Plans that started after the last snapshot are still recovered
- No data is lost from either source

---

## Recovery Validation

After recovery, `validate_recovery()` checks for inconsistencies:

```rust
pub fn validate_recovery(state: &RecoveredState) -> Vec<RecoveryWarning>
```

### Checks performed

| Check | Severity | Meaning |
|-------|----------|---------|
| Plan in `queue_order` but not in `plan_states` | Critical | Orphan queue entry — plan state is missing |
| Plan in `plan_states` but not in `queue_order` | Warning | Orphan plan state — may be unscheduled |
| `iteration == 0` | Warning | Invalid iteration (should be ≥ 1) |
| Complete plan with no `files_changed` | Info | Plan completed without modifying files (suspicious) |
| Duplicate entries in `queue_order` | Critical | Queue corruption |

### Warning severity

```rust
pub enum WarningSeverity {
    Info,      // operator should know, but safe to proceed
    Warning,   // state may be stale, proceed with caution
    Critical,  // recovered state is likely incorrect, manual inspection needed
}
```

An empty warnings list means the recovered state is consistent. Critical
warnings should halt automatic execution and require operator review.

---

## RecoveredState

```rust
pub struct RecoveredState {
    /// Per-plan phase information, keyed by plan_id.
    pub plan_states: HashMap<String, PlanPhaseInfo>,
    /// Queue order.
    pub queue_order: Vec<String>,
    /// Highest event sequence number processed.
    pub last_sequence: u64,
    /// Timestamp of recovery.
    pub recovery_timestamp_ms: u64,
}
```

### PlanPhaseInfo

```rust
pub struct PlanPhaseInfo {
    pub plan_id: String,
    pub phase: PlanPhase,
    pub iteration: u32,
    pub last_gate_result: Option<String>,
    pub files_changed: Vec<String>,
}
```

This is a subset of `PlanState` — just enough to reconstruct the executor's
understanding of where each plan is in its lifecycle.

---

## Error Types

```rust
pub enum RecoveryError {
    /// Snapshot JSON is corrupt or unparseable.
    CorruptedSnapshot(String),
    /// Event sequence numbers are not monotonically increasing.
    InvalidEventSequence(String),
    /// A plan referenced in events has no state.
    MissingPlanState(String),
}
```

All recovery errors include descriptive messages for diagnosis.

---

## CLI Usage

### Resuming from snapshot

```bash
roko plan run plans/ --resume .roko/state/executor.json
```

The `--resume` flag loads the specified snapshot, restores executor state,
and continues from where the previous run left off. Plans that were
`Complete` or `Failed` are not re-executed. Plans that were `Implementing` or
`Gating` resume from their last recorded phase.

### Manual inspection

The snapshot file is plain JSON and can be inspected with `jq`:

```bash
# List all plan phases
jq '.plan_states | to_entries[] | {plan: .key, phase: .value.current_phase}' \
  .roko/state/executor.json

# Find failed plans
jq '.plan_states | to_entries[] | select(.value.current_phase.kind == "failed")' \
  .roko/state/executor.json
```

---

## Test Coverage

### Snapshot tests

- Empty snapshot roundtrips
- Snapshot with plans roundtrips (phases, iterations, files, gates)
- Queue order is preserved
- Partial plan state uses defaults
- Legacy task-based snapshot falls back to compat loader
- Terminal plan detection

### Recovery tests

- Basic snapshot recovery
- Corrupted snapshot detection
- Gate results preserved through recovery
- Basic event log recovery
- Invalid sequence detection
- Plan failure and iteration tracking through events
- Multi-plan event log recovery
- Event log tracks files from agent/task events
- Merge: event log takes precedence over snapshot
- Merge: combines disjoint plans from both sources
- Validation: queue without state (critical)
- Validation: orphan plans (warning)
- Validation: consistent state (no warnings)
- Validation: duplicate queue entries (critical)
- End-to-end recovery pipeline

---

---

## Incremental Snapshots: Delta Encoding Between Checkpoints

Full snapshots grow with the number of plans and tasks. For long-running
orchestration sessions (100+ plans), incremental snapshots reduce I/O and
storage by encoding only what changed since the last checkpoint.

### Delta Snapshot Architecture

```rust
/// An incremental snapshot that encodes only changes since a base snapshot.
pub struct DeltaSnapshot {
    /// Sequence number of the base (full) snapshot this delta applies to.
    pub base_sequence: u64,
    /// Sequence number of this delta.
    pub delta_sequence: u64,
    /// Plans whose state changed since the base.
    pub changed_plans: HashMap<String, PlanState>,
    /// Plans removed since the base.
    pub removed_plans: Vec<String>,
    /// New plans added since the base.
    pub added_plans: HashMap<String, PlanState>,
    /// Queue order (only if it changed).
    pub queue_order: Option<Vec<String>>,
    /// BLAKE3 hash of the base snapshot (for verification).
    pub base_hash: [u8; 32],
    /// BLAKE3 hash of the reconstructed full state after applying this delta.
    pub expected_hash: [u8; 32],
    /// Timestamp.
    pub timestamp_ms: u64,
}

/// Configuration for incremental snapshot behavior.
pub struct SnapshotConfig {
    /// How many actions between full snapshots.
    /// Default: 50. Range: 10..=500.
    pub full_snapshot_interval: usize,
    /// How many actions between delta snapshots.
    /// Default: 5 (same as current AUTOSAVE_INTERVAL).
    pub delta_snapshot_interval: usize,
    /// Maximum number of deltas before forcing a full snapshot.
    /// Default: 10. Prevents long delta chains.
    pub max_delta_chain: usize,
    /// Whether to verify hash after applying deltas.
    /// Default: true. Costs ~1ms for typical state sizes.
    pub verify_on_apply: bool,
}

impl ExecutorSnapshot {
    /// Compute a delta from a base snapshot to the current state.
    ///
    /// Algorithm:
    /// 1. For each plan in current state:
    ///    - If not in base → added_plans
    ///    - If in base but different (compare BLAKE3 hash of serialized
    ///      PlanState) → changed_plans
    /// 2. For each plan in base but not in current → removed_plans
    /// 3. Compare queue_order; include only if changed.
    ///
    /// Complexity: O(P) where P = number of plans.
    pub fn delta_from(&self, base: &ExecutorSnapshot) -> DeltaSnapshot { /* ... */ }

    /// Apply a delta to produce a new full snapshot.
    pub fn apply_delta(&self, delta: &DeltaSnapshot) -> Result<Self, RecoveryError> {
        // 1. Start with base state
        // 2. Apply changed_plans (overwrite)
        // 3. Apply added_plans (insert)
        // 4. Apply removed_plans (delete)
        // 5. Apply queue_order if present
        // 6. Verify expected_hash matches (if configured)
        /* ... */
    }
}
```

### Snapshot Rotation Strategy

Following PostgreSQL's WAL segment management and EventStoreDB's snapshot
intervals (default: every 250 events in Axon Framework):

```
Timeline:  ──────────────────────────────────────────►
           F     D  D  D  D  D  F     D  D  D  D  F
           │                    │                  │
           full                 full               full
           snapshot             snapshot           snapshot

F = full snapshot (every full_snapshot_interval actions)
D = delta snapshot (every delta_snapshot_interval actions)
```

On recovery:
1. Load the most recent full snapshot
2. Apply deltas in sequence (at most `max_delta_chain`)
3. Replay event log entries after the last delta's sequence number

### Storage Savings

For a typical orchestration run with 20 plans, each `PlanState` serializes to
~500 bytes. A full snapshot is ~10KB. A delta that changes 2 plans is ~1KB —
a 90% reduction. For 100-plan runs, the savings are proportionally larger.

---

## Snapshot Verification: Detecting Corruption

The current system relies on JSON parse errors to detect corruption. A more
robust approach uses cryptographic verification at multiple levels.

### Verification Hierarchy

```rust
/// Multi-level snapshot integrity verification.
pub struct SnapshotVerifier;

impl SnapshotVerifier {
    /// Level 1: File-level BLAKE3 checksum.
    /// Detects: truncation, bit flips, partial writes.
    /// Cost: ~1ms per 10KB snapshot (BLAKE3: 8.4 GB/s single-thread).
    pub fn verify_file_checksum(
        path: &Path,
        expected: &[u8; 32],
    ) -> Result<(), IntegrityError> { /* ... */ }

    /// Level 2: Per-plan hash tree (Merkle verification).
    /// Detects: individual plan state corruption without full re-parse.
    /// Structure:
    ///   root_hash = H(queue_hash || plans_hash)
    ///   plans_hash = H(plan_0_hash || plan_1_hash || ... || plan_n_hash)
    ///   plan_i_hash = H(serialize(plan_states[i]))
    ///
    /// Verification complexity: O(log P) to check a single plan,
    /// O(P) for full verification.
    pub fn verify_merkle_tree(
        snapshot: &ExecutorSnapshot,
        expected_root: &[u8; 32],
    ) -> Result<(), IntegrityError> { /* ... */ }

    /// Level 3: Cross-validation with event log.
    /// Detects: snapshot/log divergence (snapshot was tampered or
    /// log was truncated).
    /// Algorithm:
    /// 1. Reconstruct state from event log.
    /// 2. Compare each plan's phase with snapshot.
    /// 3. Report discrepancies.
    pub fn cross_validate(
        snapshot: &ExecutorSnapshot,
        event_log: &[EventEntry],
    ) -> Vec<CrossValidationWarning> { /* ... */ }
}

pub enum IntegrityError {
    /// File-level checksum mismatch.
    ChecksumMismatch { expected: [u8; 32], actual: [u8; 32] },
    /// Merkle proof verification failed for a specific plan.
    MerkleProofFailed { plan_id: String },
    /// Snapshot state diverges from event log reconstruction.
    LogDivergence { plan_id: String, snapshot_phase: String, log_phase: String },
    /// File is truncated (size < minimum valid snapshot).
    Truncated { expected_min: usize, actual: usize },
}
```

### Torn Write Detection

Even with atomic rename, torn writes can occur if the filesystem doesn't
guarantee rename atomicity (some networked filesystems). Additional protection:

```rust
/// Snapshot file format with torn-write detection.
///
/// Layout:
///   [4 bytes] magic: 0x524F4B4F ("ROKO")
///   [4 bytes] version: 1
///   [4 bytes] payload_length (little-endian)
///   [N bytes] JSON payload
///   [32 bytes] BLAKE3 hash of payload
///   [4 bytes] magic trailer: 0x454E4421 ("END!")
///
/// Verification:
/// 1. Check magic header and trailer are present → detects truncation.
/// 2. Check payload_length matches actual payload size → detects partial write.
/// 3. Verify BLAKE3 hash → detects bit flips.
/// 4. Parse JSON → detects structural corruption.
pub struct SnapshotFileFormat;
```

This is modeled on PostgreSQL's page checksum approach (each 8KB page has a
checksum in its header, verified on every read from disk) and SQLite's WAL
frame checksums.

---

## CRDTs for Distributed Orchestrator State (Future)

For scenarios where multiple orchestrator instances coordinate (e.g., across
machines or in a high-availability setup), CRDTs provide convergence without
coordination.

### CRDT Model for Executor State

```rust
/// CRDT-based executor state that converges across replicas.
///
/// Each field uses an appropriate CRDT type:
/// - Plan phases: Monotonic join-semilattice (phases only advance).
/// - Task sets: OR-Set (add/remove with causal ordering).
/// - Counters: PN-Counter (increment/decrement).
/// - Event log: Append-only sequence (G-Set of events).
pub struct CrdtExecutorState {
    /// Plan states as LWW-Registers (last-writer-wins per plan).
    /// Ties broken by Lamport timestamp + node ID.
    pub plan_states: LwwMap<String, PlanState>,
    /// Completed plans: G-Set (grow-only, irreversible).
    pub completed: GSet<String>,
    /// Iteration counters: PN-Counter per plan.
    pub iterations: PnCounterMap<String>,
    /// Logical clock for ordering.
    pub clock: HybridLogicalClock,
}

/// Hybrid Logical Clock (Kulkarni et al., OPODIS 2014).
/// Combines physical time with logical counter for causal ordering.
/// Bounded drift: HLC stays within clock synchronization error of
/// physical time. Constant space (unlike vector clocks).
pub struct HybridLogicalClock {
    /// Physical component (milliseconds since epoch).
    pub physical: u64,
    /// Logical counter (increments on same-physical-time events).
    pub logical: u32,
    /// Node identifier.
    pub node_id: u64,
}
```

### Plan Phase as a Join-Semilattice

Plan phases form a natural lattice where phases only advance:

```
Queued < Enriching < Implementing < Gating < Verifying < Reviewing
       < DocRevision < Merging < Complete

Failed and Skipped are terminal (absorbing elements).
```

The merge operation is `max(phase_a, phase_b)` — if replica A has a plan at
`Gating` and replica B has it at `Implementing`, the merged state is `Gating`.
This is inherently conflict-free because phase transitions are monotonic.

### Delta-State CRDTs for Efficiency

Rather than shipping full state (CvRDT) or requiring exactly-once delivery
(CmRDT), use **delta-state CRDTs** — transmit only the delta (mutation) and
merge it into the receiver's state:

- Delta message size: O(changed fields) rather than O(total state)
- Network requirement: unreliable channel (same as state-based, no exactly-once needed)
- Used by Riak, Automerge 3, and most modern CRDT systems

### Convergence Reference

Automerge 3 uses compressed columnar storage with RLE encoding, achieving
~500× memory reduction for text-heavy states. For Roko's structured JSON
state, the improvement would be more modest (~10×) but still significant
for large plan sets.

---

## References

- The snapshot + event-log dual recovery is a variation of the "snapshotting +
  write-ahead log" pattern from database systems (Mohan, C. et al. (1992).
  ARIES: A transaction recovery method supporting fine-granularity locking and
  partial rollbacks using write-ahead logging. *ACM TODS*, 17(1), 94–162).
- Event sourcing: Fowler, M. (2005). Event Sourcing.
  *martinfowler.com/eaaDev/EventSourcing.html*.
- Atomic file writes via rename: POSIX guarantees that `rename(2)` is atomic
  within a single filesystem, preventing partial-write corruption.
- Shapiro, M. et al. (2011). Conflict-free replicated data types. *SSS 2011*.
  (CRDTs: state-based, operation-based, delta-state.)
- Kleppmann, M. & Beresford, A. R. (2017). A conflict-free replicated JSON
  datatype. *IEEE TPDS*, 28(10), 2733–2746. (Automerge foundation.)
- Kulkarni, S. S. et al. (2014). Logical physical clocks and consistent
  snapshots in globally distributed databases. *OPODIS 2014*. (Hybrid
  Logical Clocks.)
- Hinto, P. et al. (2024). Loro: Reimagining state synchronization for local-
  first software. *loro.dev*. (Replayable Event Graph CRDTs in Rust.)
- Percival, C. (2003). Naive differences of executable code. *bsdiff*.
  (Delta encoding for binary snapshots.)
- O'Connor, J. et al. (2020). BLAKE3: One function, fast everywhere.
  *blake3.io*. (8.4 GB/s single-thread, 92 GB/s 16-core; Merkle tree
  structure enables incremental verification.)
