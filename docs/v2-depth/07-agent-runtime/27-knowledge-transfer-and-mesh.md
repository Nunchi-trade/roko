# 27. Knowledge Transfer and Mesh

> Knowledge sharing via Agent Mesh as a Connect Cell with Bus federation. Version-vector sync for delta tracking. Bloom filter discovery for fast neighbor lookup. Four-tier gossip architecture. Demurrage on shared knowledge. Genomic bottleneck for portable export.

See [01-SIGNAL.md](../../unified/01-SIGNAL.md) for Signal and demurrage, [11-CONNECTIVITY.md](../../unified/11-CONNECTIVITY.md) for Connect protocol, [10-GROUPS.md](../../unified/10-GROUPS.md) for Group coordination.

---

## 1. The Mesh as a Connect Cell

The Agent Mesh is not a special subsystem. It is a Cell implementing the Connect protocol with lifecycle management (health checks, reconnection, backpressure). The Mesh provides the transport; knowledge sharing is implemented as Store operations over that transport.

```rust
/// MeshConnector: Connect Cell for Agent-to-Agent knowledge transfer.
///
/// Implements: Connect protocol (connect, query, execute, disconnect)
/// Transport: WebSocket to Mesh relay (wss://mesh.roko.dev/v1/ws)
/// Partition tolerance: AP design (available + partition-tolerant, eventually consistent)
///
/// Crate: `crates/roko-runtime/src/mesh.rs`
pub struct MeshConnector {
    relay_url: String,
    collective_id: String,
    version_vector: VersionVector,
    bloom_filter: BloomFilter,
    connection: Option<WebSocketConnection>,
    config: MeshConfig,
}

impl Connect for MeshConnector {
    async fn connect(&mut self) -> Result<()> {
        self.connection = Some(
            WebSocketConnection::new(&self.relay_url)
                .with_auth(self.agent_attestation())
                .with_reconnect(ExponentialBackoff::default())
                .connect()
                .await?
        );
        Ok(())
    }

    async fn query(&self, request: &Signal) -> Result<Vec<Signal>> {
        // Query peer knowledge via relay
        let query = KnowledgeQuery::from_signal(request);
        self.connection.as_ref()
            .ok_or(ConnectError::NotConnected)?
            .send(MeshMessage::Query(query))
            .await?;
        // Receive response (async)
        let response = self.connection.as_ref().unwrap().recv().await?;
        Ok(response.signals)
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(conn) = self.connection.take() {
            conn.send(MeshMessage::Deregister).await?;
            conn.close().await?;
        }
        Ok(())
    }
}
```

---

## 2. Version-Vector Sync

The Mesh uses version-vector-based delta sync (Lamport 1978, Fidge 1988). Each Agent maintains a vector tracking the highest sequence number received from each peer:

```rust
/// Version vector: tracks sync state per peer.
/// Maps agent_id -> highest_sequence_number_received.
///
/// When Agent A wants to sync with Agent B:
///   1. A sends its version vector to B
///   2. B computes delta: signals where B.seq > A.vector[B.id]
///   3. B sends only the delta signals to A
///   4. A updates its vector: A.vector[B.id] = B.current_seq
pub type VersionVector = HashMap<AgentId, u64>;

/// Delta sync message.
pub struct SyncDelta {
    pub source: AgentId,
    pub signals: Vec<SharedSignal>,
    pub version_vector: VersionVector,
    pub timestamp: u64,
}

/// A Signal packaged for mesh sharing.
pub struct SharedSignal {
    pub signal: Signal,
    pub seq: u64,                    // monotonically increasing per source
    pub shared_by: AgentId,
    pub shared_at: u64,
    pub attestation: Option<Ed25519Signature>,
}
```

### 2.1 Sync Protocol

```
Agent A                          Relay                          Agent B
   |                               |                               |
   |--- SyncRequest(my_vv) ------->|                               |
   |                               |--- SyncRequest(A.vv) -------->|
   |                               |                               |
   |                               |<-- SyncDelta(new_signals) ----|
   |<-- SyncDelta(new_signals) ----|                               |
   |                               |                               |
   |--- ACK(updated_vv) ---------->|                               |
   |                               |                               |
```

Sync interval is configurable (default: 300s). The interval adapts with behavioral state:
- Struggling: 150s (share more, seek help)
- Exploring: 200s (share discoveries)
- Focused: 600s (minimize distraction)
- Resting: 900s (minimal sync)

---

## 3. Bloom Filter Discovery

Before exchanging full Signal content, Agents exchange Bloom filters to discover which Signals exist across the collective. This prevents redundant transfers:

```rust
/// Bloom filter for Signal hash discovery.
///
/// Parameters chosen for ~1% false positive rate at 50,000 entries:
///   m = 479,252 bits (~58.5 KB)
///   k = 7 hash functions
///
/// False positive means we skip a Signal we do not have (acceptable:
/// we will discover it on the next sync cycle).
pub struct SignalBloomFilter {
    bits: BitVec,
    hash_count: u32,
    entry_count: u64,
}

impl SignalBloomFilter {
    /// Create from the local Store's Signal hashes.
    pub fn from_store(store: &Store) -> Self {
        let mut bloom = Self::new(50_000, 0.01);
        for hash in store.all_hashes() {
            bloom.insert(&hash);
        }
        bloom
    }

    /// Check which of the peer's signals we do NOT have.
    pub fn missing_from(&self, peer_hashes: &[ContentHash]) -> Vec<ContentHash> {
        peer_hashes.iter()
            .filter(|h| !self.probably_contains(h))
            .cloned()
            .collect()
    }
}
```

### 3.1 Discovery Protocol

```
1. Agent A publishes its Bloom filter to relay (every sync_interval)
2. Agent B receives A's Bloom filter
3. B checks: which of MY signals are NOT in A's filter?
4. B sends only those signals in the next SyncDelta
```

This reduces bandwidth by ~90% compared to full sync (most signals are already shared).

---

## 4. Four-Tier Gossip Architecture

Knowledge propagates through four tiers with increasing latency and scope:

| Tier | Protocol | Latency | Scope | Content |
|---|---|---|---|---|
| **1. GossipSub** | Pub/sub over WebSocket | Milliseconds | Immediate collective | Warnings, urgent insights (Pulses) |
| **2. Delta Sync** | Version-vector exchange | Seconds-minutes | Extended collective | Validated Signals, cross-checked findings |
| **3. Aggregation** | Periodic batch | Per epoch (5 min) | Cross-collective | Anonymized statistics, aggregate patterns |
| **4. Canonical** | Store replication | Per block / per hour | All agents | Consensus knowledge, verified facts |

### 4.1 Tier 1: GossipSub (Real-Time)

Urgent Signals broadcast immediately via the relay's pub/sub layer. Warning-type and Threat-pheromone signals use this tier:

```rust
/// Tier 1: Immediate gossip for urgent signals.
pub async fn gossip_urgent(&self, signal: &Signal) {
    if signal.kind == Kind::Warning || signal.urgency > 0.8 {
        self.connection.as_ref().unwrap()
            .publish("collective.urgent", signal)
            .await;
    }
}
```

### 4.2 Tier 2: Delta Sync (Standard)

The version-vector sync described above. Standard knowledge sharing.

### 4.3 Tier 3: Aggregation (Cross-Collective)

Aggregated statistics shared across collective boundaries. Individual Signals are never shared cross-collective -- only aggregate patterns:

```rust
/// Tier 3: Cross-collective aggregate sharing.
pub struct CollectiveAggregate {
    pub collective_id: String,
    pub period: TimeRange,
    pub metrics: AggregateMetrics,
}

pub struct AggregateMetrics {
    pub avg_gate_pass_rate: f64,
    pub dominant_regime: Regime,
    pub top_anomaly_patterns: Vec<PatternHash>,
    pub knowledge_growth_rate: f64,
}
```

### 4.4 Tier 4: Canonical (Consensus)

For Agents on the same chain (e.g., Korai), consensus knowledge is written to on-chain storage and replicated via block finality. For non-chain Agents, Tier 4 uses relay-mediated consensus.

---

## 5. Receiving Knowledge: The Adoption Pipeline

Received Signals go through a Pipeline Graph before entering the local Store:

```toml
[graph]
id = "mesh_adoption_pipeline"
kind = "pipeline"

[[cells]]
id = "rate_limit"
protocol = "Verify"
description = "Max 100 signals/hour from mesh"

[[cells]]
id = "verify_attestation"
protocol = "Verify"
description = "Check Ed25519 signature on SharedSignal"

[[cells]]
id = "reputation_check"
protocol = "Score"
description = "Score sender reputation (TraceRank)"

[[cells]]
id = "confidence_discount"
protocol = "Score"
description = "Multiply confidence by 0.7 (mesh discount)"

[[cells]]
id = "duplicate_check"
protocol = "Verify"
description = "Check against local Store hashes (Bloom filter)"

[[cells]]
id = "quarantine"
protocol = "Store"
description = "Place in quarantine tier, await validation"

[[cells]]
id = "provenance_stamp"
protocol = "Store"
description = "Add mesh provenance entry to Signal lineage"
```

### 5.1 Confidence Discount

All received Signals have their confidence multiplied by `received_confidence_discount` (default: 0.7). This is the mesh equivalent of generational decay:

```
Original confidence: 0.85
After mesh transfer:  0.85 * 0.7 = 0.595
After validation:     0.595 + 0.1 = 0.695 (if gate passes)
```

### 5.2 Behavioral State Modulation

The Daimon modulates receiving behavior:

| State | Threshold Modifier | Rationale |
|---|---|---|
| Struggling | -0.15 (receive more) | Need help, accept more input |
| Exploring | -0.20 (receive most) | Active discovery |
| Focused | +0.15 (receive less) | Deep work, minimize distraction |
| Coasting | +0.10 (selective) | Low urgency |

---

## 6. Stigmergic Coordination

The Mesh supports indirect coordination via typed pheromone Signals:

```rust
/// Pheromone types for stigmergic coordination.
/// Agents deposit pheromones that modify the shared environment.
/// Other agents respond to accumulated patterns.
pub enum PheromoneKind {
    /// Immediate danger. Fast decay (1 hour half-life).
    Threat { domain: String, intensity: f32 },
    /// Discovered opportunity. Moderate decay (6 hour half-life).
    Opportunity { domain: String, details: String },
    /// Validated long-term knowledge. Slow decay (7 day half-life).
    Wisdom { topic: String, confidence: f32 },
    /// Unusual pattern requiring investigation. Variable decay.
    Anomaly { signature: Vec<f32>, novelty: f32 },
}
```

Agents reading the shared pheromone field respond to accumulated patterns:
- High concentration of Threat pheromones -> increased caution (affect modulation).
- Opportunity cluster -> exploration bias.
- Wisdom convergence -> confidence boost for matching local knowledge.

This implements Grassi's termite coordination (1959): no direct message passing, only environment modification.

---

## 7. Knowledge Demurrage on Shared Signals

Shared Signals carry demurrage just like local Signals. But mesh Signals have an additional "freshness cost" -- the older a shared Signal, the more expensive it is to maintain in the receiving Store:

```rust
/// Demurrage for mesh-received Signals.
///
/// Base demurrage (from Signal type) * freshness_penalty
/// Freshness penalty = 1.0 + (age_hours / 168.0)  // grows 1.0 per week
///
/// This creates economic pressure to use shared knowledge promptly
/// or let it decay. Matches Gesell's velocity principle.
pub fn mesh_demurrage(signal: &Signal, hours_since_received: f64) -> f64 {
    let base = signal.base_demurrage_rate();
    let freshness_penalty = 1.0 + (hours_since_received / 168.0);
    base * freshness_penalty
}
```

---

## 8. Portable Export via Genomic Bottleneck

For offline transfer (no mesh), the genomic bottleneck produces a compressed Store snapshot:

```rust
/// Export knowledge as portable snapshot using genomic bottleneck.
///
/// At most max_signals are selected. The compression acts as
/// a regularizer: domain-specific overfitting is stripped,
/// generalizable knowledge is preserved.
pub fn export_bottleneck(
    store: &Store,
    max_signals: usize,  // default: 2048
) -> BottleneckExport {
    let mut selected = Vec::with_capacity(max_signals);

    // 25% safety reserve: all Warnings + Persistent-tier
    let safety: Vec<_> = store.query_kind(Kind::Warning)
        .chain(store.query_tier(Tier::Persistent))
        .take(max_signals / 4)
        .collect();
    selected.extend(safety);

    // 50% diversity sample: top per-type, HDC-diverse
    let remaining = max_signals - selected.len();
    let diverse = hdc_diverse_sample(
        store,
        remaining / 2,
        &selected, // avoid duplicates
    );
    selected.extend(diverse);

    // 25% quality fill: highest-scored regardless of type
    let remaining = max_signals - selected.len();
    let quality = store.top_by_score(remaining, &selected);
    selected.extend(quality);

    BottleneckExport {
        signals: selected,
        manifest: ExportManifest {
            source_agent: store.agent_id(),
            total_in_store: store.count(),
            exported_count: selected.len(),
            compression_ratio: store.count() as f64 / selected.len() as f64,
            timestamp: now(),
        },
    }
}
```

---

## What This Enables

- **Live knowledge sharing**: Running Agents exchange validated insights in real-time without stopping.
- **Partition tolerance**: AP design means the mesh works even with network partitions -- eventual consistency recovers.
- **Bandwidth efficiency**: Bloom filters + version vectors reduce sync traffic by ~90%.
- **Trust without face-to-face**: Attestation + reputation + confidence discount ensure received knowledge is treated with appropriate skepticism.
- **Emergent coordination**: Stigmergic pheromones enable collective intelligence without explicit messaging.
- **Portable snapshots**: Genomic bottleneck produces small, transferable knowledge packages.

## Feedback Loops

1. **Share -> peer validates -> gate passes -> peer boosts confidence -> peer shares back improved version** (cross-agent Loop): Knowledge improves through collective validation.
2. **Pheromone deposit -> peers respond -> outcomes change -> new pheromones reflect updated state** (stigmergic Loop): The shared environment self-corrects.
3. **Confidence discount -> validation required -> gate outcomes -> confidence update -> eventually full trust** (trust Loop): Earned trust through demonstrated utility.
4. **C-factor measurement -> sharing parameter adjustment -> better collective performance -> higher c-factor** (meta-Loop): The collective tunes its own sharing behavior.

## Open Questions

1. Should the confidence discount be sender-specific (trusted peers get higher discount)?
2. How should the Bloom filter handle Store growth beyond 50,000 Signals (multiple filters? adaptive sizing)?
3. Should pheromone intensity be weighted by sender reputation?
4. What is the correct gossip protocol for permissioned subnets (restrict Tier 3/4 to subnet members)?

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Define `MeshConnector` Connect Cell | `crates/roko-runtime/src/mesh.rs` | Not started |
| Implement version-vector delta sync | `crates/roko-runtime/src/mesh/sync.rs` | Not started |
| Implement Bloom filter discovery | `crates/roko-runtime/src/mesh/bloom.rs` | Not started |
| Implement adoption Pipeline Graph | `crates/roko-runtime/src/mesh/adoption.rs` | Not started |
| Implement pheromone types and decay | `crates/roko-core/src/pheromone.rs` | Not started |
| Implement genomic bottleneck export | `crates/roko-neuro/src/export.rs` | Not started |
| Wire MeshConnector into Agent lifecycle | `crates/roko-cli/src/orchestrate.rs` | Partial (mesh config exists) |
| Mesh-specific demurrage calculation | `crates/roko-neuro/src/demurrage.rs` | Not started |
