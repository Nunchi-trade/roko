# Cryptographic Audit Trail: Merkle Hash-Chain and Engram Lineage

> **Layer**: L0 Runtime (persistence), L3 Harness (verification), Cross-cut (Safety & Provenance)
>
> **Crate**: `roko-core` (Engram lineage), `roko-fs` (FileSubstrate persistence)
>
> **Synapse traits**: `Substrate` (persist and query Engrams), `Gate` (verify chain integrity)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md)


> **Implementation**: Partially Wired

---

## The Problem

An autonomous agent managing tasks, code, or capital needs forensic-grade evidence of exactly what happened. When something goes wrong — a build fails, a test regresses, a trade loses money, an unexpected action occurs — the operator needs to reconstruct the precise sequence of decisions, tool calls, and outcomes that led to the failure.

Standard log files (JSONL, text) can be tampered with after the fact. Entries can be deleted, modified, or reordered without detection. This is unacceptable for agents operating in regulated environments or managing valuable assets.

---

## The Engram Lineage DAG

Every piece of information in Roko is an **Engram** — a content-addressed, scored, decaying, lineage-tracked unit of cognition. The audit trail is built into the Engram's structure via two fields:

### Content Addressing

Every Engram has an `id` field that is a BLAKE3 content hash of its canonical fields:

```rust
pub struct Engram {
    pub id: ContentHash,              // BLAKE3(kind + body + author + tags)
    pub kind: Kind,                   // semantic type
    pub body: Body,                   // payload (text, JSON, binary)
    pub tags: BTreeMap<String, String>, // ordered metadata (included in hash)
    pub created_at_ms: i64,           // Unix milliseconds
    pub decay: Decay,                 // None | HalfLife | Ttl | Ebbinghaus
    pub score: Score,                 // 7-axis appraisal
    pub lineage: Vec<ContentHash>,    // parent Engrams (audit DAG)
    pub provenance: Provenance,       // author, model fingerprint, taint chain
    pub attestation: Option<Attestation>, // cryptographic proof of origin
}
```

Content addressing means:
- The same content always produces the same hash (deterministic)
- Any modification to the content changes the hash (tamper-evident)
- The hash serves as a unique, unforgeable identifier

### Lineage (Audit DAG)

The `lineage: Vec<ContentHash>` field links every Engram to its parents — the Engrams that caused it to exist. This creates a directed acyclic graph (DAG) where any Engram can be traced back through its causal chain to the original inputs.

For example, when the `ToolDispatcher` executes a tool call:

1. The tool call request becomes an Engram with `Kind::ToolInvocation`
2. The tool result becomes an Engram with `lineage = [tool_call_engram.id]`
3. The gate verdict becomes an Engram with `lineage = [tool_result_engram.id]`
4. The final persisted output becomes an Engram with `lineage = [gate_verdict_engram.id]`

Walking the lineage backwards from any Engram reconstructs the complete decision chain.

### Provenance

The `Provenance` struct records who created the Engram and how:

```rust
pub struct Provenance {
    pub author: String,           // "tool_dispatcher", "claude-sonnet-4-20250514", etc.
    pub model_fingerprint: Option<String>,  // Model identifier if LLM-generated
    pub taint_chain: Vec<String>, // Sequence of processing steps
}
```

### Attestation

The optional `Attestation` field provides cryptographic proof of origin:

```rust
pub struct Attestation {
    pub signature: Vec<u8>,       // Cryptographic signature
    pub signer: String,           // Identity of the signer
    pub algorithm: String,        // Signing algorithm (e.g., "ed25519")
    pub timestamp: i64,           // When the attestation was created
}
```

---

## The SHA-256 Merkle Hash-Chain (Legacy Design)

The legacy specification describes a linear hash-chain audit trail where each entry contains a SHA-256 hash of the previous entry. This design is preserved in the Roko architecture as a specialized Substrate implementation:

### AuditChain Structure

```rust
use sha2::{Sha256, Digest};

/// The append-only audit chain. One per agent lifetime.
pub struct AuditChain {
    writer: std::io::BufWriter<std::fs::File>,
    current_seq: u64,
    last_hash: [u8; 32],
}

/// A single entry in the audit chain. Every field participates in
/// the hash computation. Tampering with any field invalidates the
/// chain from that point forward.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    /// Hash of the previous entry (chain link).
    pub prev_hash: [u8; 32],
    pub timestamp: u64,
    pub tick: u64,
    pub event: AuditEvent,
    /// Hash of THIS entry (computed over all fields above).
    pub hash: [u8; 32],
}
```

### Audit Event Types

Eleven event types cover the state transitions an operator might want to inspect:

```rust
pub enum AuditEvent {
    /// A tool was called. params_hash and result_hash are SHA-256 of
    /// the serialized parameters and result (not the raw data, to avoid
    /// logging sensitive values).
    ToolCall { tool: String, params_hash: [u8; 32], result_hash: [u8; 32] },
    /// A gate verdict was rendered.
    GateVerdict { gate: String, passed: bool, score: f64 },
    /// A capability permit was created.
    PermitCreated { permit_id: String, action: String, value_limit: String },
    /// A capability permit was consumed.
    PermitConsumed { permit_id: String },
    /// An inference call was made.
    InferenceCall { model: String, tokens_in: u32, tokens_out: u32, cost: f64 },
    /// The Neuro knowledge store was mutated.
    NeuroMutation { mutation_type: String, entry_id: String },
    /// An operator intervention was received.
    InterventionReceived { source: String, severity: String },
    /// A phase transition occurred in the execution pipeline.
    PhaseTransition { from: String, to: String },
    /// A taint violation was blocked.
    TaintViolationBlocked { label: String, sink: String },
    /// A safety policy check result.
    SafetyCheck { policy: String, passed: bool, reason: String },
    /// An Engram was persisted to the Substrate.
    EngramPersisted { engram_id: String, kind: String },
}
```

### Chain Operations

```rust
impl AuditChain {
    /// Append an entry to the chain. Returns the new entry's hash.
    pub fn append(&mut self, tick: u64, event: AuditEvent) -> Result<[u8; 32]> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        let mut hasher = Sha256::new();
        hasher.update(&self.current_seq.to_le_bytes());
        hasher.update(&self.last_hash);
        hasher.update(&timestamp.to_le_bytes());
        hasher.update(&tick.to_le_bytes());
        hasher.update(serde_json::to_vec(&event)?);
        let hash: [u8; 32] = hasher.finalize().into();

        let entry = AuditEntry {
            seq: self.current_seq,
            prev_hash: self.last_hash,
            timestamp,
            tick,
            event,
            hash,
        };

        bincode::serialize_into(&mut self.writer, &entry)?;
        self.writer.flush()?;

        self.last_hash = hash;
        self.current_seq += 1;
        Ok(hash)
    }

    /// Verify the integrity of the entire chain.
    /// Returns false if any entry has been tampered with.
    pub fn verify(entries: &[AuditEntry]) -> bool {
        for window in entries.windows(2) {
            if window[1].prev_hash != window[0].hash {
                return false;
            }
            let mut hasher = Sha256::new();
            hasher.update(&window[1].seq.to_le_bytes());
            hasher.update(&window[1].prev_hash);
            hasher.update(&window[1].timestamp.to_le_bytes());
            hasher.update(&window[1].tick.to_le_bytes());
            hasher.update(serde_json::to_vec(&window[1].event).unwrap());
            let expected: [u8; 32] = hasher.finalize().into();
            if window[1].hash != expected {
                return false;
            }
        }
        true
    }
}
```

### On-Chain Anchoring (Chain-Domain Agents)

For agents operating in the chain domain (via `roko-chain`), the audit chain root hash can be periodically anchored on-chain:

```rust
/// Anchor the current chain root hash on-chain.
/// Cost: ~$0.001 per anchor on L2. Recommended: every 1,000 ticks or daily.
/// Creates an on-chain commitment that the audit log existed at this
/// block — if the operator disputes what happened, the chain state
/// proves the log's integrity.
pub async fn anchor_onchain(
    &self,
    provider: &impl alloy::providers::Provider,
) -> Result<String> {
    let tx_hash = anchor_hash_onchain(provider, self.last_hash).await?;
    Ok(tx_hash)
}
```

---

## Current Implementation: Signal-Based Audit Trail

The current Roko codebase implements audit trailing via the `Signal` type (to be renamed to `Engram` in Tier 0D) and the `AuditSink` trait:

### AuditSink Trait

```rust
/// A sink for audit signals emitted by the ToolDispatcher.
pub trait AuditSink: Send + Sync {
    fn emit(&self, signal: Signal);
}
```

The `ToolDispatcher` emits audit signals at every phase of tool dispatch:

| Phase | Status | Signal Tags |
|-------|--------|-------------|
| `validation` | `passed` or `failed` | `call_id`, `tool`, `phase=validation` |
| `tool_filter` | `denied` | `call_id`, `tool`, `phase=tool_filter` |
| `permission` | `granted` or `denied` | `call_id`, `tool`, `phase=permission` |
| `safety` | `blocked` | `call_id`, `tool`, `phase=safety` |
| `handler` | `started` or `missing` | `call_id`, `tool`, `phase=handler` |
| `completion` | `succeeded` or `failed` | `call_id`, `tool`, `phase=completion` |

Each signal is a `Signal` (future `Engram`) with:
- `Kind::ToolInvocation`
- `Provenance::trusted("tool_dispatcher")`
- Tags for `call_id`, `tool`, `phase`, and `status`
- Body containing JSON with phase-specific details

### FileSubstrate Persistence

Audit signals are persisted via the `FileSubstrate` in `roko-fs`, which writes to JSONL files (`.roko/signals.jsonl`). The FileSubstrate provides:

- Append-only JSONL writes with fsync
- Query by kind, tags, and time range
- Garbage collection of expired Engrams (respecting decay schedules)
- Layout management for the `.roko/` directory structure

### Episode Logger

The `EpisodeLogger` in `roko-learn` records higher-level audit events (agent turns, gate verdicts, usage metrics) to `.roko/episodes.jsonl`:

```rust
pub struct Episode {
    pub task_id: String,
    pub agent_role: String,
    pub turns: Vec<Turn>,
    pub gate_verdicts: Vec<GateVerdict>,
    pub usage: Usage,
    pub started_at: i64,
    pub finished_at: i64,
}
```

---

## Relationship to Witness DAG

The linear hash-chain described above is extended by the Witness DAG (see [12-witness-dag.md](12-witness-dag.md)), which uses BLAKE3 content-addressed vertices with five vertex types: Observation, Decision, Action, Outcome, and Attestation. The Witness DAG provides:

- Full causal DAG structure (not just linear chain)
- ZK proofs for strategy auditing without revealing proprietary details
- SQLite storage with indexed queries
- Cross-agent DAG merging via content-addressed vertex deduplication

---

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| `Signal` (Engram) with `lineage` field | Built | `roko-core/src/signal.rs` |
| `Provenance` struct | Built | `roko-core/src/signal.rs` |
| `AuditSink` trait | Built | `roko-core/src/tool/mod.rs` |
| ToolDispatcher audit signal emission | Built | `roko-agent/src/dispatcher/mod.rs` |
| FileSubstrate JSONL persistence | Built | `roko-fs/src/lib.rs` |
| EpisodeLogger | Built | `roko-learn/src/episode_logger.rs` |
| `Attestation` on Engrams | Design only | Target: Tier 2 |
| SHA-256 linear hash-chain | Design only | Target: Tier 2 |
| On-chain anchoring | Design only | Target: Tier 3 (chain domain) |
| Witness DAG | Design only | Target: Tier 3 |

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Merkle (1979) | Merkle hash trees — cryptographic data integrity verification |
| Nakamoto (2008) | Bitcoin — append-only hash-chain for transaction ordering |
| Benet (2014) | IPFS — content-addressed DAG for distributed storage |
| BLAKE3 Team (2020) | BLAKE3 — fast, parallelizable cryptographic hash function |

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — Overall safety architecture
- [12-witness-dag.md](12-witness-dag.md) — Extended DAG with ZK proofs
- [15-forensic-ai.md](15-forensic-ai.md) — Regulatory compliance via causal replay
