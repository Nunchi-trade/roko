# Snapshot and Recovery

> Depth for [03-GRAPH.md](../../unified/03-GRAPH.md). How Graph execution state is persisted to Store and recovered after crashes, using dual-source recovery (snapshots + event log replay).

---

## Problem

A Graph execution (Flow) may run for hours or days across dozens of plans. Process death -- OOM kill, machine restart, network drop -- must not lose progress. The system needs:

1. **Point-in-time capture** -- fast serialization of the full executor state
2. **Append-only audit trail** -- hash-chained event log that can reconstruct state from first principles
3. **Merged recovery** -- combine both sources for maximum fidelity

These are Store protocol operations: `put` (persist snapshot), `get` (load snapshot), `query` (replay event log from sequence N).

---

## Executor Snapshot

The snapshot is a point-in-time capture of every Flow's mutable state, serialized as JSON to `.roko/state/executor.json`.

### Structure

```rust
/// Point-in-time capture of all Flow states.
struct ExecutorSnapshot {
    /// Per-Flow mutable state, keyed by plan_id.
    plan_states: HashMap<String, PlanState>,
    /// Execution queue order (plan_ids in priority order).
    queue_order: Vec<String>,
    /// Unix millisecond timestamp of capture.
    timestamp_ms: u64,
}
```

Each `PlanState` captures:

| Field | What It Records |
|---|---|
| `current_phase` | Queued, Enriching, Implementing, Gating, Verifying, Reviewing, DocRevision, Merging, Complete, Failed, Skipped |
| `assigned_agents` | Active agent identifiers |
| `gate_results` | Accumulated Verify protocol verdicts |
| `iteration` | Retry count (how many times the Flow has cycled) |
| `files_changed` | Files modified by agents (for merge conflict detection) |
| `merge_attempts` | Number of merge attempts |
| `paused` | Whether the Flow is suspended |
| `priority` | Scheduling priority |

### Atomic Writes

The runtime writes snapshots atomically to prevent corruption from partial writes:

```
1. Write to .roko/state/executor.json.tmp
2. fsync the temp file (flush to disk)
3. Rename .tmp --> .roko/state/executor.json
```

`rename(2)` is atomic on POSIX: either the old snapshot or the new one is visible, never a partial write. If the process crashes during step 1 or 2, the temp file is left behind but the original snapshot is intact.

### Auto-Save Frequency

The `PlanRunner` auto-saves every `AUTOSAVE_INTERVAL` (5) actions. This bounds data loss to at most 5 actions per crash. A typical 100-action run produces ~20 snapshots, each overwriting the previous.

### Legacy Compatibility

`from_json()` handles legacy snapshot formats that used a flat `tasks` array instead of `plan_states`:

```rust
fn from_json(json: &str) -> Result<ExecutorSnapshot, Error> {
    // Detect legacy "tasks" key
    if value.get("tasks").is_some() && value.get("plan_states").is_none() {
        return from_legacy_json(json);  // group by plan_id, infer phases
    }
    serde_json::from_str(json)
}
```

---

## Event Log

The event log is an append-only, hash-chained sequence of orchestration Pulses. Every significant transition is recorded as an `EventEntry`.

### Structure

```rust
/// A single entry in the hash-chained event log.
struct EventEntry {
    /// Monotonically increasing sequence number (0-based).
    sequence_number: u64,
    /// Unix millisecond timestamp.
    timestamp_ms: i64,
    /// Event classification.
    event_kind: EventKind,
    /// Structured JSON payload (event-specific).
    payload: serde_json::Value,
    /// BLAKE3 content hash (includes previous entry's hash).
    content_hash: [u8; 32],
}
```

### Event Kinds

| Kind | When | Payload |
|---|---|---|
| `PlanStarted` | Flow begins | `{ plan_id }` |
| `TaskAssigned` | Cell dispatched to agent | `{ plan_id, task_id, files }` |
| `AgentSpawned` | Agent process launched | `{ plan_id, files }` |
| `GateResult` | Verify protocol produces Verdict | `{ plan_id, gate, passed, summary }` |
| `PhaseTransition` | Flow changes phase | `{ plan_id, phase }` |
| `MergeAttempted` | Merge Pipeline entered | `{ plan_id, branch }` |
| `PlanCompleted` | Flow succeeded | `{ plan_id }` |
| `PlanFailed` | Flow failed terminally | `{ plan_id, reason, iteration }` |
| `ErrorOccurred` | Runtime error | `{ plan_id, error }` |
| `InterventionFired` | Conductor intervened | `{ plan_id, intervention }` |
| `EnrichmentValidated` | Enrichment data validated | `{ plan_id }` |

All payloads include `plan_id` for per-Flow filtering and recovery.

### Hash Chain

Each entry's BLAKE3 hash includes the previous entry's hash, creating a tamper-evident chain:

```rust
fn compute_hash(
    seq: u64, ts_ms: i64,
    kind: &EventKind, payload: &Value,
    prev_hash: &[u8; 32],
) -> [u8; 32] {
    let mut buf = Vec::new();
    buf.extend(b"eventv1|");           // version prefix
    buf.extend(seq.to_be_bytes());      // sequence number
    buf.extend(ts_ms.to_be_bytes());    // timestamp
    buf.extend(prev_hash);              // chain link
    push_length_prefixed(&mut buf, kind_str.as_bytes());
    push_length_prefixed(&mut buf, &canonical_json(payload));
    blake3::hash(&buf).into()
}
```

The genesis entry uses `[0u8; 32]` as its previous hash. Length-prefixed fields prevent field-body collisions (where concatenating different fields could produce the same byte sequence).

### Tamper Detection

The hash chain detects:

| Attack | How Detected |
|---|---|
| Payload modification | Recomputed hash does not match stored hash |
| Entry deletion | All subsequent entries' hashes become invalid (broken chain link) |
| Entry reordering | Hash encodes sequence number; wrong position breaks hash |
| Entry insertion | Changes chain link for all subsequent entries |
| Hash modification | Subsequent entries encoded the original hash as their `prev_hash` |

The only undetectable modification is end-truncation (removing the last N entries). The snapshot's `last_sequence` number reveals this discrepancy.

### Operations

| Operation | What It Does |
|---|---|
| `append(kind, payload)` | Acquires lock, computes hash from tip, pushes entry, updates tip |
| `replay()` | Returns all entries in insertion order (full reconstruction) |
| `replay_from(seq)` | Returns entries from sequence N onward (incremental recovery) |
| `verify_integrity()` | Recomputes every hash; returns `IntegrityError` at first break |
| `entries_by_kind(kind)` | Filters entries by classification |
| `snapshot() / restore()` | Serializes/restores the full log including tip hash |

---

## Recovery Engine

The `RecoveryEngine` is stateless -- it provides methods but holds no state.

### Recovery from Snapshot

```rust
fn recover_from_snapshot(json: &str) -> Result<RecoveredState, RecoveryError>
```

Deserializes the snapshot, converts each `PlanState` to a `PlanPhaseInfo` (the minimal subset needed to resume), and preserves queue order.

### Recovery from Event Log

```rust
fn recover_from_event_log(events: &[EventEntry]) -> Result<RecoveredState, RecoveryError>
```

Replays events in sequence, building per-Flow state:

| Event | Effect on State |
|---|---|
| `PlanStarted` | Create plan entry, set phase to Enriching |
| `PhaseTransition` | Update phase from payload |
| `TaskAssigned` / `AgentSpawned` | Add files to `files_changed` |
| `GateResult` | Update `last_gate_result` |
| `PlanCompleted` | Set phase to Complete |
| `PlanFailed` | Set phase to Failed with reason |

Validates monotonic sequence numbers -- non-monotonic sequences indicate log corruption.

### Merged Recovery

When both sources are available, the engine merges them:

```
event_log_state  +  snapshot_state  -->  merged_state
```

**Merge rules**:

1. **Event log wins on conflict**: For the same plan, the event log version is used (may contain events after the snapshot).
2. **Disjoint plans are combined**: Plans appearing only in one source are included.
3. **Queue order**: Event log's order preferred if non-empty; otherwise snapshot's.
4. **Sequence numbers**: Higher value taken.

This ensures a 5-minute-stale snapshot gets updated by recent events, and plans started after the last snapshot are still recovered.

### Validation

After recovery, `validate_recovery()` checks consistency:

| Check | Severity | Meaning |
|---|---|---|
| Plan in `queue_order` but not in `plan_states` | Critical | Orphan queue entry |
| Plan in `plan_states` but not in `queue_order` | Warning | Unscheduled plan |
| `iteration == 0` | Warning | Invalid (should be >= 1) |
| Complete plan with no `files_changed` | Info | Suspicious no-op |
| Duplicate entries in `queue_order` | Critical | Queue corruption |

Critical warnings halt automatic execution and require operator review.

### RecoveredState

```rust
struct RecoveredState {
    plan_states: HashMap<String, PlanPhaseInfo>,
    queue_order: Vec<String>,
    last_sequence: u64,
    recovery_timestamp_ms: u64,
}

struct PlanPhaseInfo {
    plan_id: String,
    phase: PlanPhase,
    iteration: u32,
    last_gate_result: Option<String>,
    files_changed: Vec<String>,
}
```

---

## Incremental Snapshots (Design)

Full snapshots grow with the number of plans. For long-running sessions (100+ plans), delta encoding reduces I/O by capturing only what changed since the last full snapshot.

### Delta Structure

```rust
struct DeltaSnapshot {
    base_sequence: u64,
    delta_sequence: u64,
    changed_plans: HashMap<String, PlanState>,
    removed_plans: Vec<String>,
    added_plans: HashMap<String, PlanState>,
    queue_order: Option<Vec<String>>,  // only if changed
    /// BLAKE3 hashes for verification.
    base_hash: [u8; 32],
    expected_hash: [u8; 32],
}
```

**Rotation strategy** (following PostgreSQL WAL segment management):

```
Timeline: ----F----D--D--D--D--D--F----D--D--D--D--F----
          full                    full                full

F = full snapshot (every full_snapshot_interval actions, default 50)
D = delta snapshot (every delta_snapshot_interval actions, default 5)
```

On recovery: load most recent full snapshot, apply deltas in sequence (max `max_delta_chain`), then replay event log entries after the last delta's sequence.

**Storage savings**: For 20 plans, a full snapshot is ~10KB. A delta changing 2 plans is ~1KB -- 90% reduction.

### Verification Hierarchy

```
Level 1: File-level BLAKE3 checksum
    Detects: truncation, bit flips, partial writes
    Cost: ~1ms per 10KB (BLAKE3: 8.4 GB/s single-thread)

Level 2: Per-plan Merkle tree
    root_hash = H(queue_hash || plans_hash)
    plans_hash = H(plan_0_hash || ... || plan_n_hash)
    Detects: individual plan corruption without full re-parse
    Cost: O(log P) for single plan, O(P) for full

Level 3: Cross-validation with event log
    Reconstruct from log, compare each plan's phase
    Detects: snapshot/log divergence (tampering or truncation)
```

### Torn Write Protection

A binary file format with header/trailer magic for environments where `rename(2)` atomicity is not guaranteed (some networked filesystems):

```
[4 bytes] magic: 0x524F4B4F ("ROKO")
[4 bytes] version: 1
[4 bytes] payload_length (little-endian)
[N bytes] JSON payload
[32 bytes] BLAKE3 hash of payload
[4 bytes] magic trailer: 0x454E4421 ("END!")
```

Verification: check header+trailer (detects truncation), check payload length (detects partial write), verify hash (detects bit flips), parse JSON (detects structural corruption).

---

## CRDT Executor State (Future)

For distributed orchestrator scenarios (multiple instances coordinating across machines), CRDTs provide convergence without coordination.

### Plan Phase as Join-Semilattice

Plan phases form a natural lattice where phases only advance:

```
Queued < Enriching < Implementing < Gating < Verifying
       < Reviewing < DocRevision < Merging < Complete

Failed and Skipped are terminal (absorbing elements).
```

Merge = `max(phase_a, phase_b)`. If replica A has `Gating` and replica B has `Implementing`, merged state is `Gating`. Conflict-free because phase transitions are monotonic.

### CRDT Types

| Field | CRDT Type | Property |
|---|---|---|
| Plan states | LWW-Register per plan | Last-writer-wins, ties broken by HLC + node_id |
| Completed plans | G-Set (grow-only) | Irreversible completion |
| Iteration counters | PN-Counter per plan | Increment/decrement |
| Logical clock | Hybrid Logical Clock (Kulkarni 2014) | Bounded drift from physical time, constant space |

Delta-state CRDTs transmit only mutations (O(changed fields)) rather than full state, reducing network overhead.

---

## CLI Usage

### Resume from snapshot

```bash
roko plan run plans/ --resume .roko/state/executor.json
```

Loads the snapshot, restores executor state, continues from where the previous run left off. Complete and Failed plans are not re-executed. Implementing and Gating plans resume from their last recorded phase.

### Manual inspection

```bash
# List all plan phases
jq '.plan_states | to_entries[] | {plan: .key, phase: .value.current_phase}' \
  .roko/state/executor.json

# Find failed plans
jq '.plan_states | to_entries[] | select(.value.current_phase.kind == "failed")' \
  .roko/state/executor.json
```

---

## What This Enables

1. **Crash tolerance** -- Multi-hour orchestration sessions survive process death. At most 5 actions lost per crash (AUTOSAVE_INTERVAL).
2. **Dual-source recovery** -- Snapshot for speed, event log for completeness. Merge gives maximum fidelity.
3. **Tamper-evident audit** -- The hash chain makes any modification to the event log detectable. Critical for compliance and debugging.
4. **Incremental persistence** -- Delta snapshots reduce I/O by 90% for long-running sessions.

---

## Feedback Loops

1. **Snapshot-event log cross-validation**: After recovery, the engine compares both sources. Discrepancies feed back into validation warnings that can trigger operator review. This is a Verify protocol operation on the recovery itself.

2. **Auto-save frequency adaptation**: The fixed interval (5 actions) could be adapted based on crash frequency. More crashes -> more frequent saves. Less crashes -> larger intervals. This would be a Loop pattern with the crash rate as the feedback signal.

3. **Event log compaction**: Over very long runs, the event log grows unbounded. The snapshot "checkpoints" the log, allowing entries before the snapshot to be pruned. This is analogous to PostgreSQL's WAL checkpoint + segment recycling.

---

## Open Questions

1. **Incremental snapshots not implemented**: The delta encoding design exists (including BLAKE3 verification and Merkle trees) but only full snapshots are implemented. The current AUTOSAVE_INTERVAL (5) with full snapshots works for typical plan sizes (< 50 plans) but may become I/O-bound for larger sessions.

2. **Event log persistence path**: The event log is snapshotted alongside the executor snapshot, but the actual persistence mechanism (file format, rotation, compaction) is not specified. Currently it lives in memory during execution.

3. **Recovery validation thresholds**: `validate_recovery()` produces warnings but the thresholds for "too many warnings" -> "halt execution" are not defined. A Critical warning halts, but the boundary between Warning and Critical is currently hardcoded.

4. **CRDT executor state**: The distributed recovery design is fully specified (join-semilattice phases, HLC, delta-state CRDTs) but not implemented. This blocks multi-machine orchestration scenarios.

5. **Binary snapshot format**: The torn-write-protected binary format is designed but not implemented. The current JSON format relies on POSIX `rename(2)` atomicity, which may not hold on all filesystems.
