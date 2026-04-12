# 04 — The Artifact Store

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-gate` (`crates/roko-gate/src/artifact_store.rs`)
> **Status**: Implemented (172 lines)

---

## 1. Overview

The `ArtifactStore` is a content-addressed, append-only store for gate artifacts. Every
artifact is identified by its BLAKE3 hash. The store deduplicates automatically: storing
the same content twice returns the same hash without writing a second copy.

Content addressing is a cornerstone of the verification architecture. It enables:
- **Immutable artifacts**: Once stored, an artifact's content is fixed. The hash is its
  identity. There is no "update" operation.
- **Deduplication**: Identical outputs from different gate runs share storage.
- **Reproducibility**: Given a hash, you can always retrieve the exact artifact.
- **Forensic replay**: Any verdict can be traced to its exact inputs and outputs.

> **Citation**: crates/roko-gate/src/artifact_store.rs — Full implementation.

---

## 2. Structure

```rust
pub type ContentHash = [u8; 32];

pub struct ArtifactStore {
    items: HashMap<ContentHash, Vec<u8>>,
}
```

The store is an in-memory `HashMap` from 32-byte BLAKE3 hashes to byte vectors. This
is intentionally simple — no filesystem, no database, no network. The current
implementation is an in-process store suitable for a single plan execution.

### Why BLAKE3

BLAKE3 is chosen over SHA-256 for three reasons:
1. **Speed**: BLAKE3 is 5–15x faster than SHA-256 on modern hardware, critical when
   hashing megabytes of test output.
2. **Streaming**: BLAKE3 supports incremental hashing without buffering the full input.
3. **Keyed mode**: BLAKE3 supports keyed hashing, enabling future per-session namespacing
   without a separate HMAC construction.

---

## 3. Operations

### 3.1 Store

```rust
pub fn store(&mut self, data: &[u8]) -> ContentHash {
    let hash = blake3::hash(data).into();
    self.items.entry(hash).or_insert_with(|| data.to_vec());
    hash
}
```

Computes the BLAKE3 hash of the input data. If the hash is not already in the store,
inserts the data. Returns the hash in both cases. This is the only write operation.

### 3.2 Retrieve

```rust
pub fn get(&self, hash: &ContentHash) -> Option<&[u8]> {
    self.items.get(hash).map(Vec::as_slice)
}
```

Returns the artifact bytes for a given hash, or `None` if the hash is not in the store.

### 3.3 Contains

```rust
pub fn contains(&self, hash: &ContentHash) -> bool {
    self.items.contains_key(hash)
}
```

Check existence without retrieving the data.

### 3.4 Count

```rust
pub fn len(&self) -> usize {
    self.items.len()
}
```

Number of unique artifacts stored.

---

## 4. Immutability and Append-Only Semantics

The store has no `delete`, `update`, or `clear` operations in its public API. Once an
artifact is stored, it exists for the lifetime of the store. This is a deliberate design
constraint:

- **No accidental loss**: A gate artifact that was used to produce a verdict cannot
  disappear.
- **Audit trail**: The chain from verdict → artifact hash → artifact content is always
  intact.
- **Concurrency safety**: Append-only structures have simpler concurrency properties
  than mutable ones.

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> "Immutable verification artifacts" as a key architectural decision.

---

## 5. Deduplication

When an agent retries a task and produces identical output, the artifact store does not
allocate new memory. The BLAKE3 hash matches the existing entry, and the `or_insert_with`
short-circuits. This matters because:

- Gate outputs can be large (megabytes of test runner output)
- Retries are common (3–5 attempts per task is typical)
- Many retries produce identical or near-identical output on the portions that haven't
  changed

Deduplication is automatic and zero-cost at the application level.

---

## 6. Relationship to Gate Verdicts

The artifact store sits alongside the gate pipeline, not inside it. The current
integration pattern is:

```
Gate produces verdict with detail (full output)
    ↓
Orchestrator stores detail in ArtifactStore
    ↓
ArtifactStore returns ContentHash
    ↓
ContentHash can be attached to the verdict or signal for later retrieval
```

This separation means gates don't need to know about the artifact store. They produce
verdicts with `detail` strings, and the orchestrator decides what to persist.

---

## 7. Future: Persistent Artifact Store

The current in-memory store is ephemeral — it lives and dies with the process. The
design anticipates a persistent version:

### 7.1 Filesystem Layout

```
.roko/artifacts/
├── ab/
│   ├── ab3f8c1d2e...  (BLAKE3 hash as filename)
│   └── abd9e4f720...
├── cd/
│   └── cd1a2b3c4d...
└── manifest.jsonl      (hash → metadata mapping)
```

Two-character prefix directories prevent any single directory from having too many
entries (a common filesystem performance issue).

### 7.2 Manifest

A JSONL file mapping hashes to metadata:
```json
{"hash": "ab3f8c1d2e...", "gate": "compile:cargo", "plan": "plan-42", "rung": 0, "timestamp": "2026-04-10T12:00:00Z", "size_bytes": 4096}
```

### 7.3 Garbage Collection

With persistence comes the need for GC. Artifacts older than a configurable threshold
(e.g., 30 days) with no references from active plans can be pruned. The JSONL manifest
enables efficient reference counting.

---

## 8. Content Addressing in the Broader Architecture

Content-addressed storage appears in multiple places in Roko:

| Component | What It Hashes | Hash Algorithm |
|---|---|---|
| `ArtifactStore` | Gate output bytes | BLAKE3 |
| `Signal` | Signal content | BLAKE3 |
| `FileSubstrate` | Signal bodies (JSONL) | BLAKE3 |
| Episode logs | Agent turns | N/A (sequential) |

The consistency of BLAKE3 across the system means any artifact or signal can be
cross-referenced by hash. A verdict's detail text, stored in the artifact store, hashes
to the same value whether you compute it from the store or from the verdict's detail
field.

> **Citation**: refactoring-prd/01-synapse-architecture.md — "Content-addressed,
> scored, decaying, lineage-tracked" — Engrams are content-addressed; artifacts follow
> the same principle.

---

## 9. Relationship to Forensic AI

The artifact store is a building block for Forensic AI causal replay (see
[12-forensic-ai-causal-replay.md](./12-forensic-ai-causal-replay.md)). To replay an
agent's verification history:

1. Retrieve the signal (engram) by hash from the Substrate.
2. Retrieve the gate artifacts by hash from the ArtifactStore.
3. Replay the gate pipeline with the original signal and compare verdicts.

Content addressing makes this replay exact: the same hash guarantees the same content,
so the replayed inputs are byte-identical to the originals.

> **Citation**: refactoring-prd/09-innovations.md — Innovation IX: Forensic AI Causal
> Replay, "content-addressed replay of any agent action."

---

## 10. Testing

The artifact store's tests cover:

| Test | What It Verifies |
|---|---|
| `store_and_retrieve` | Basic store/get roundtrip |
| `deduplication` | Same content → same hash, no duplicate storage |
| `missing_hash` | `get()` returns `None` for unstored hashes |
| `contains_check` | `contains()` matches `get().is_some()` |
| `empty_data` | Empty byte slices are valid artifacts |
| `large_data` | Large inputs (megabytes) work correctly |

> **Citation**: crates/roko-gate/src/artifact_store.rs — Tests section.

---

## 11. Summary

The `ArtifactStore` is deliberately minimal: store bytes, get bytes, check existence.
No deletion, no mutation, no networking. This simplicity makes it correct by construction
— there are no race conditions, no consistency issues, and no data loss paths.

The key insight is that **verification artifacts are write-once, read-many**. A gate's
output never changes after the gate runs. By giving each artifact a unique
content-derived identity (BLAKE3 hash), the system can reference artifacts reliably
across time, across retries, and across processes without coordination.
