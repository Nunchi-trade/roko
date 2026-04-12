# Snapshot & Crash Recovery

> **Modules**: `roko-orchestrator/src/executor/snapshot.rs`,
> `roko-orchestrator/src/executor/recovery.rs`
> **Key types**: `ExecutorSnapshot`, `RecoveryEngine`, `RecoveredState`
> **Persistence path**: `.roko/state/executor.json`
> **CLI flag**: `--resume .roko/state/executor.json`

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

## References

- The snapshot + event-log dual recovery is a variation of the "snapshotting +
  write-ahead log" pattern from database systems (Mohan, C. et al. (1992).
  ARIES: A transaction recovery method supporting fine-granularity locking and
  partial rollbacks using write-ahead logging. *ACM TODS*, 17(1), 94–162).
- Event sourcing: Fowler, M. (2005). Event Sourcing.
  *martinfowler.com/eaaDev/EventSourcing.html*.
- Atomic file writes via rename: POSIX guarantees that `rename(2)` is atomic
  within a single filesystem, preventing partial-write corruption.
