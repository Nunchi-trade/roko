# Event Log

> **Module**: `roko-orchestrator/src/event_log.rs`
> **Key types**: `EventLog`, `EventEntry`, `EventKind`, `EventLogSnapshot`
> **Tests**: 12 tests covering append, hash chain, integrity, replay, snapshot,
> concurrent appends, tamper detection


> **Implementation**: Shipping

---

## Overview

The `EventLog` is an append-only, hash-chained sequence of orchestration
events. Every significant action — plan started, agent spawned, gate result,
merge attempted, plan completed — is recorded as an `EventEntry`. Entries are
linked by BLAKE3 content hashes: each entry's hash includes the previous
entry's hash, creating a tamper-evident chain.

The event log serves three purposes:

1. **Crash recovery**: Events can be replayed to reconstruct executor state
   (see `09-snapshot-recovery.md`)
2. **Audit trail**: The hash chain makes any modification, deletion, or
   reordering of events detectable
3. **Observability**: The log provides a complete timeline of orchestration
   activity for debugging and analysis

---

## EventKind

```rust
pub enum EventKind {
    PlanStarted,         // A plan began execution
    TaskAssigned,        // A task was assigned to an agent
    AgentSpawned,        // An agent process was launched
    GateResult,          // A gate produced a verdict
    MergeAttempted,      // A merge was attempted
    PlanCompleted,       // A plan completed successfully
    PlanFailed,          // A plan failed terminally
    ErrorOccurred,       // An error occurred
    InterventionFired,   // A conductor intervention was triggered
    PhaseTransition,     // A plan changed phases
    EnrichmentValidated, // Enrichment data was validated
}
```

Each kind has a string display form for logging:

```
plan.started, task.assigned, agent.spawned, gate.result,
merge.attempted, plan.completed, plan.failed, error.occurred,
intervention.fired, phase.transition, enrichment.validated
```

---

## EventEntry

```rust
pub struct EventEntry {
    /// Monotonically increasing sequence number (0-based).
    pub sequence_number: u64,
    /// Unix millisecond timestamp.
    pub timestamp_ms: i64,
    /// Classification of the event.
    pub event_kind: EventKind,
    /// Structured payload (event-specific JSON).
    pub payload: serde_json::Value,
    /// BLAKE3 content hash (includes previous entry's hash).
    pub content_hash: [u8; 32],
}
```

### Payload conventions

Payloads are structured JSON with event-specific fields:

```json
// PlanStarted
{ "plan_id": "01-workspace" }

// PhaseTransition
{ "plan_id": "01-workspace", "phase": { "kind": "implementing" } }

// GateResult
{ "plan_id": "01-workspace", "gate": "compile", "passed": true, "summary": "ok" }

// AgentSpawned
{ "plan_id": "01-workspace", "files": ["src/main.rs", "src/lib.rs"] }

// PlanFailed
{ "plan_id": "01-workspace", "reason": "compilation errors", "iteration": 3 }

// PlanCompleted
{ "plan_id": "01-workspace" }
```

All payloads include `plan_id` to enable per-plan event filtering and recovery.

---

## Hash Chain

The hash chain is the event log's tamper-detection mechanism. Each entry's
content hash is computed from:

1. A version prefix (`"eventv1|"`)
2. The sequence number (big-endian u64)
3. The timestamp (big-endian i64)
4. The **previous entry's content hash** (32 bytes)
5. The event kind (length-prefixed string)
6. The payload (length-prefixed canonical JSON)

```rust
fn compute_hash(
    seq: u64,
    ts_ms: i64,
    kind: &EventKind,
    payload: &serde_json::Value,
    prev_hash: &[u8; 32],
) -> [u8; 32] {
    let mut buf = Vec::new();
    buf.extend(b"eventv1|");
    buf.extend(seq.to_be_bytes());
    buf.extend(ts_ms.to_be_bytes());
    buf.extend(prev_hash);
    push_lp(&mut buf, kind_str.as_bytes());
    push_lp(&mut buf, &payload_bytes);
    ContentHash::of(&buf).0
}
```

The BLAKE3 hash function provides cryptographic security — finding a collision
(modifying an entry without changing its hash) is computationally infeasible.

### Chain initialization

The first entry uses `ZERO_HASH` (`[0u8; 32]`) as its previous hash. This is
the genesis of the chain.

### Length-prefixed fields

Fields are length-prefixed (`push_lp`) to prevent field-body collisions. Without
length prefixes, concatenating fields could produce the same byte sequence from
different inputs.

---

## Operations

### append()

```rust
pub fn append(&self, event_kind: EventKind, payload: Value) -> EventEntry
```

Appends a new event to the log:

1. Acquires the mutex lock
2. Computes the sequence number (= current length)
3. Gets the current timestamp
4. Computes the content hash using the current tip hash
5. Creates the `EventEntry`
6. Updates the tip hash
7. Pushes the entry

Returns the fully constructed entry (with hash) for immediate use.

### replay()

```rust
pub fn replay(&self) -> Vec<EventEntry>
```

Returns all events in insertion order. Used for full state reconstruction.

### replay_from()

```rust
pub fn replay_from(&self, seq: u64) -> Vec<EventEntry>
```

Returns events starting from a given sequence number (inclusive). Used for
incremental recovery — if the snapshot is at sequence 42, replay from 42
onward to catch up.

### verify_integrity()

```rust
pub fn verify_integrity(&self) -> Result<(), IntegrityError>
```

Recomputes every entry's hash from scratch and compares it to the stored hash.
Returns `Ok(())` if the chain is intact, or an `IntegrityError` at the first
broken link.

Verification also checks that the tip hash matches the last entry's hash.

```rust
pub struct IntegrityError {
    pub at_sequence: u64,
    pub reason: String,
}
```

### entries_by_kind()

```rust
pub fn entries_by_kind(&self, kind: &EventKind) -> Vec<EventEntry>
```

Filters events by kind. Useful for extracting all gate results, all errors,
or all phase transitions.

### snapshot() / restore()

```rust
pub fn snapshot(&self) -> EventLogSnapshot
pub fn restore(snapshot: EventLogSnapshot) -> Self
```

Serializes the log for crash recovery. The snapshot includes all entries and
the tip hash. A restored log can continue appending — new entries chain from
the restored tip.

---

## Thread Safety

The `EventLog` uses `Arc<Mutex<LogInner>>` for thread-safe access:

```rust
struct LogInner {
    entries: Vec<EventEntry>,
    tip: [u8; 32],
}
```

Multiple async tasks can append events concurrently. The mutex serializes
appends to maintain the hash chain invariant — each append must know the
previous tip hash.

### Concurrent append test

The test suite includes a concurrent append test that spawns 4 threads, each
appending 25 events (100 total). After all threads complete, the hash chain
verifies successfully. This demonstrates the safety of the `Arc<Mutex<>>`
approach under contention.

---

## Tamper Detection

The hash chain detects several types of tampering:

### Payload modification

If an event's payload is altered (e.g., changing a gate result from `passed: false`
to `passed: true`), the recomputed hash won't match the stored hash.

### Entry deletion

If an entry is removed, all subsequent entries' hashes become invalid because
they depend on the deleted entry's hash.

### Entry reordering

If entries are swapped, their hashes become invalid because each hash
encodes the sequence number.

### Insertion

If an entry is inserted between existing entries, all subsequent hashes
break because the chain link (previous hash) changes.

### Hash direct modification

Even if someone modifies both the payload and the stored hash, subsequent
entries will fail verification because they encoded the original hash as
their `prev_hash`.

The only undetectable modification is truncating the log at the end — removing
the last N entries is not detectable by the chain itself. However, the
snapshot's `last_sequence` number would reveal the discrepancy.

---

## Integration with the Orchestrator

The `PlanRunner` appends events at every significant point:

| When | Event Kind | Payload |
|------|-----------|---------|
| Plan dispatched | `PlanStarted` | plan_id |
| Agent spawned | `AgentSpawned` | plan_id, role, task, files |
| Task assigned | `TaskAssigned` | plan_id, task_id, files |
| Gate result received | `GateResult` | plan_id, gate, passed, summary |
| Phase transition | `PhaseTransition` | plan_id, phase |
| Merge attempted | `MergeAttempted` | plan_id, branch |
| Plan completed | `PlanCompleted` | plan_id |
| Plan failed | `PlanFailed` | plan_id, reason |
| Error occurred | `ErrorOccurred` | plan_id, error |
| Conductor intervention | `InterventionFired` | plan_id, intervention |
| Enrichment validated | `EnrichmentValidated` | plan_id |

The event log is also snapshotted alongside the executor snapshot for
crash recovery.

---

## Relationship to Forensic AI

The hash-chained event log is the orchestrator's implementation of the
Forensic AI innovation described in `refactoring-prd/09-innovations.md`:

> Content-addressed causal replay — every Engram and every decision carries a
> content hash, forming a Merkle DAG from raw observation to final action.
> Regulators, auditors, or the agent itself can replay the exact causal chain
> that led to any outcome.

The event log provides exactly this capability at the orchestration layer:
every plan transition, every agent dispatch, every gate result is recorded
with a hash chain that enables exact causal replay.

This is critical for:

- **Debugging**: Understanding why a plan failed by replaying its event
  sequence
- **Cost attribution**: Tracing which decisions led to which costs
- **Compliance**: Providing an auditable record of automated actions
- **Learning**: Analyzing event patterns to improve future orchestration

---

## References

- BLAKE3: O'Connor, J. et al. (2020). BLAKE3: One function, fast everywhere.
  *blake3.io*. (The hash function used for content addressing)
- Hash-chaining for tamper detection follows the Bitcoin blockchain's linked
  hash chain pattern (Nakamoto, S. (2008). Bitcoin: A Peer-to-Peer Electronic
  Cash System), adapted for local audit trails rather than distributed
  consensus.
- Event sourcing: Fowler, M. (2005). Event Sourcing. The event log is a
  textbook implementation of event sourcing, where state is derived from
  a sequence of events rather than stored directly.
- Causal replay for AI systems: the Forensic AI concept from
  `refactoring-prd/09-innovations.md`, which extends content-addressed
  logging to the full agent decision chain.
