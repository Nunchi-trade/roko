# Gossip and Privacy

> Depth for [22-REGISTRIES.md](../../unified/22-REGISTRIES.md). How peer-to-peer communication emerges as Bus federation across network boundaries, and how privacy emerges as Verify Cells that accept ZK proof Signals instead of plaintext.

---

## 1. The Design Error This Corrects

Early gossip architecture invented bespoke concepts: a `GossipSub` layer, a `FABRIC TEE aggregation` service, a `Canonical Event Bus` (confusingly named but NOT the unified Bus), a `DandelionConfig` struct, a `VectorClock` type, a `GCounter` CRDT, and eight named topic channels with their own schemas. The privacy layer similarly invented its own taxonomy (four tiers P0-P3) with ad-hoc integration points.

The result: gossip could not compose with the rest of the system. An agent that wanted to subscribe to both local Bus topics (like `verify.completed`) and network gossip topics (like `korai/knowledge/v1`) needed two completely different subscription mechanisms. Privacy was bolted on per-feature rather than being a uniform property of the Verify protocol.

This depth doc redesigns gossip as **Bus federation** -- the same Bus primitive extended across network boundaries -- and privacy as **Verify Cells with privacy-preserving evidence types** -- the same Verify protocol accepting ZK proofs as evidence instead of plaintext.

---

## 2. Gossip IS Bus Federation

The unified Bus is Roko's ephemeral pub/sub transport fabric. Pulses flow on topics; Cells subscribe and publish. Within a single agent process, Bus is in-memory. Between agents on the same machine, Bus is IPC. Between agents across the network, Bus is the gossip protocol.

This is not an analogy. It is a literal identity. The gossip layer IS the inter-process, inter-machine implementation of Bus transport.

### The Federation Protocol

A federated Bus extends the publish/subscribe contract across process and network boundaries:

```rust
/// BusFederation: a Connector Cell that bridges local Bus to network Bus.
///
/// Implements Connect protocol (peer mesh lifecycle) and React protocol
/// (forward Pulses between local and network domains).
///
/// This IS the gossip layer -- expressed as a standard Cell.
pub struct BusFederation {
    /// Cell identity.
    id: CellId,

    /// Local Bus handle (in-process pub/sub).
    local_bus: BusHandle,

    /// Peer mesh: the set of remote BusFederation nodes this instance
    /// maintains bidirectional connections with.
    /// Managed via HyParView membership protocol.
    mesh: PeerMesh,

    /// Topic subscriptions: which topics this node replicates.
    /// Not all topics are replicated -- only those with subscribers.
    subscriptions: TopicSet,

    /// Dissemination strategy per topic.
    /// PlumTree (push-lazy-push) for bandwidth efficiency.
    dissemination: DisseminationConfig,

    /// Identity: this node's Korai Passport (required for gossip participation).
    passport: PassportSignal,

    /// Privacy overlay: per-topic Dandelion++ stem routing.
    privacy: PrivacyOverlay,
}
```

### Topic Mapping: 8 Gossip Topics as 8 Bus Partitions

The eight gossip "topics" from the source architecture are simply Bus topic partitions with a network scope:

| Gossip Topic | Bus Topic Pattern | Scope | Graduation? |
|---|---|---|---|
| `korai/knowledge/v1` | `knowledge.*` | Federated | Yes: entry -> Store |
| `korai/reputation/v1` | `reputation.*` | Federated | Yes: score -> Store |
| `korai/job/v1` | `jobs.*` | Federated (filtered) | Yes: assignment -> Store |
| `korai/heartbeat/v1` | `heartbeat.*` | Federated | No (ephemeral only) |
| `korai/anomaly/v1` | `challenge.*` | Federated | Conditional |
| `korai/simulation/v1` | `pheromone.*` | Request-response | No |
| `korai/governance/v1` | `governance.*` | Federated | Yes: vote -> Store |
| `korai/peer-discovery/v1` | `identity.*` | Federated | Yes: registration -> Store |

The key insight: **some Pulses graduate to Signals; most do not**. A heartbeat Pulse never graduates -- it serves its purpose (liveness detection) and expires from the ring buffer. A knowledge announcement Pulse graduates to a durable Signal in Store when it achieves sufficient confirmation. This IS the unified Graduation primitive applied to network communication.

### Four Tiers as Latency Bands

The source architecture's four gossip tiers (T0 GossipSub, T1 Simulation, T2 FABRIC, T3 Canonical) are not four separate systems. They are four latency bands of the same federated Bus:

```rust
/// Tier is a delivery guarantee level, not a separate transport.
/// All tiers flow through the same BusFederation infrastructure.
#[derive(Clone, Copy)]
pub enum DeliveryTier {
    /// Best-effort, millisecond delivery. Pulses may be lost.
    /// Implementation: GossipSub mesh (push to all mesh peers).
    BestEffort,

    /// Request-response, seconds. Guaranteed delivery to target.
    /// Implementation: direct peer message (not broadcast).
    Addressed,

    /// Epoch-batched, minutes. TEE-attested aggregate.
    /// Implementation: accumulate -> TEE compute -> publish aggregate.
    Batched,

    /// Block-finalized. Total ordering. Permanent.
    /// Implementation: submit as chain transaction.
    Canonical,
}
```

A Pulse published at `DeliveryTier::BestEffort` propagates through the mesh immediately. The same conceptual message, when it achieves sufficient confirmation at the mesh layer, graduates to a Signal and is submitted at `DeliveryTier::Canonical` (an on-chain transaction). The tiers represent a trust ladder, not separate infrastructure.

### Membership: HyParView as Connect Protocol

The gossip mesh uses HyParView (Correia et al., 2007) for membership management. In unified terms, HyParView is the `connect()` / `disconnect()` lifecycle of the BusFederation Connector:

```rust
#[async_trait]
impl Connect for BusFederation {
    async fn connect(&mut self, config: &ConnectConfig) -> Result<()> {
        // HyParView JOIN:
        // 1. Contact a known seed node (bootstrap peer)
        // 2. Seed forwards JOIN to its active view
        // 3. Receiving nodes add joiner to passive view, probabilistically
        //    promote to active view
        // 4. Maintain active_view_size = 5, passive_view_size = 30
        self.mesh.join(config.seed_peers()).await?;
        Ok(())
    }

    async fn health(&self) -> HealthStatus {
        // Active view health: do we have enough mesh peers?
        let active = self.mesh.active_view_size();
        if active >= 4 { HealthStatus::Healthy }
        else if active >= 2 { HealthStatus::Degraded }
        else { HealthStatus::Unhealthy }
    }

    async fn disconnect(&mut self) -> Result<()> {
        // HyParView LEAVE: notify active view peers
        self.mesh.leave().await?;
        Ok(())
    }
}
```

### Dissemination: PlumTree as Publish Strategy

PlumTree (Leitao et al., 2007) provides bandwidth-efficient broadcast by constructing an overlay tree from the mesh. In unified terms, PlumTree is the publish strategy of the federated Bus:

- **Eager push** (tree edges): full Pulse payload forwarded immediately to tree children.
- **Lazy push** (non-tree edges): only Pulse ID gossiped; receiver requests full payload if not received via tree path.

This is invisible to Cells publishing Pulses. A Cell calls `bus.publish(topic, pulse)` and the federation layer handles dissemination -- whether local (in-memory dispatch), tree (eager push to tree children), or mesh (lazy push to non-tree peers).

---

## 3. The Korai Passport as Identity Signal

Every participant in the federated Bus must hold a Korai Passport -- an ERC-721 soulbound NFT that serves as the agent's on-chain identity. In unified terms, the passport is a **Signal of Kind::Identity** stored on-chain:

```rust
/// The Korai Passport expressed as a Signal.
///
/// This is the on-chain attestation that a Bus participant is a registered
/// agent with declared capabilities, staked resources, and verifiable identity.
pub struct PassportSignal {
    /// Standard Signal envelope.
    signal: Signal,

    /// Passport-specific fields (stored on-chain via Agent Registry contract).
    passport_id: u256,
    owner: Address,

    /// Capability bitmask (10 declared capabilities).
    capabilities: u64,

    /// Per-domain KORAI stakes.
    domain_stakes: BTreeMap<String, U256>,

    /// Per-domain reputation tracks.
    reputation: BTreeMap<String, ReputationScore>,

    /// System prompt hash (ventriloquist defense -- see section 4).
    prompt_hash: [u8; 32],

    /// TEE attestation (optional hardware-backed identity proof).
    tee_attestation: Option<TeeAttestation>,

    /// Tier classification (Protocol / Sovereign / Worker / Edge).
    tier: PassportTier,
}

/// Four tiers with economic boundaries.
pub enum PassportTier {
    /// Governance-approved. Operates protocol surfaces.
    Protocol,
    /// 25,000 KORAI staked. Direct hire eligible.
    Sovereign,
    /// 5,000 KORAI staked. Standard marketplace access.
    Worker,
    /// No stake required. Rate-limited access.
    Edge,
}
```

The passport is required for gossip participation because every Pulse on the federated Bus is signed by the sender's Ed25519 key, which is bound to their passport at registration. Unsigned or unattributable Pulses are dropped. This makes the federated Bus Sybil-resistant: creating a new identity requires economic commitment (stake), and reputation cannot be transferred between passports (soulbound property).

### Gossip Envelope as Signed Pulse

Every Pulse crossing a network boundary is wrapped in a signature envelope:

```rust
/// A Pulse crossing the network boundary becomes a SignedPulse.
/// This is the gossip "envelope" -- a Pulse with attribution.
pub struct SignedPulse {
    /// The original Pulse (topic, payload, sequence number).
    pulse: Pulse,

    /// Sender's passport ID (looked up in Identity Registry).
    sender: u256,

    /// Ed25519 signature over (topic, payload, timestamp).
    signature: [u8; 64],

    /// Delivery tier (determines propagation strategy).
    tier: DeliveryTier,

    /// TTL: heartbeats remaining before expiry.
    ttl: u32,
}
```

---

## 4. Ventriloquist Defense as Verify Cell

The ventriloquist defense (SHA-256 commitment of system prompt on-chain) is a **Verify Cell** that checks Signal provenance against on-chain attestation. It prevents identity spoofing: an attacker who gains control of an agent's infrastructure cannot replace the system prompt without detection, because the prompt hash is committed on-chain.

In unified terms:

```rust
/// VentriloquistVerify: a Verify Cell that checks whether an agent's
/// current system prompt matches its on-chain commitment.
///
/// This sits in the pre-execution Pipeline for any job assignment.
/// If the prompt hash does not match, the Verdict is Reject.
pub struct VentriloquistVerify {
    id: CellId,

    /// ChainConnector for reading the Identity Registry.
    chain: Arc<ChainConnector>,
}

#[async_trait]
impl Verify for VentriloquistVerify {
    type Evidence = PromptHashEvidence;

    async fn verify_pre(&self, signal: &Signal) -> Verdict {
        // 1. Extract the agent's passport_id from the Signal's provenance
        let passport_id = signal.provenance().agent_id();

        // 2. Read the committed prompt hash from on-chain Identity Registry
        let committed_hash = self.chain.query(
            "getPassport", passport_id
        ).await?.prompt_hash;

        // 3. Compute SHA-256 of the agent's current system prompt
        let current_hash = sha256(signal.context().system_prompt());

        // 4. Compare
        if current_hash == committed_hash {
            Verdict::Pass {
                confidence: 1.0,
                evidence: PromptHashEvidence::Match { hash: current_hash },
            }
        } else {
            Verdict::Reject {
                reason: "System prompt hash mismatch (ventriloquist attack detected)",
                evidence: PromptHashEvidence::Mismatch {
                    expected: committed_hash,
                    actual: current_hash,
                },
            }
        }
    }
}
```

### Prompt Update as Timelocked Store Write

Legitimate prompt updates (agents evolve over time) require a timelocked on-chain transaction. In unified terms, this is a Store write with a temporal constraint:

```toml
# Prompt update constraint (expressed as Graph edge metadata)
[edges.prompt_update]
from = "agent.prompt_change"
to = "chain.identity_registry.update_prompt_hash"
constraints = { timelock_hours = 24, rate_limit = "3_per_30_days" }
penalty = { reputation = -0.05, per_excess_change = true }
```

The 24-hour timelock creates a detection window. If an agent's prompt hash change is pending, monitoring Cells can flag it for review. If more than 3 changes occur in 30 days, an automatic reputation penalty is applied -- this is the Verify protocol enforcing update hygiene.

---

## 5. Privacy as Verify with ZK Evidence

The Valhalla privacy layer provides four tiers of privacy (P0-P3). In unified terms, these are not four separate systems but four **evidence types** that the Verify protocol can accept:

| Privacy Tier | Unified Expression | Evidence Type |
|---|---|---|
| P0 (Public) | Standard Verify with plaintext evidence | `Evidence::Plaintext(data)` |
| P1 (Access-Gated) | Verify with encrypted evidence + key proof | `Evidence::Encrypted(ciphertext, key_proof)` |
| P2 (Confidential) | Verify with TEE attestation evidence | `Evidence::TeeAttested(attestation_report)` |
| P3 (Full Sealed) | Verify with ZK proof evidence | `Evidence::ZkProof(proof, public_inputs)` |

The Verify Cell does not care HOW the evidence was produced -- it only checks whether the evidence satisfies the verification criterion. This makes privacy a property of the evidence, not a property of the infrastructure.

### ZK Proofs as Privacy-Preserving Verify Evidence

```rust
/// ZkVerify: a Verify Cell that accepts zero-knowledge proofs
/// as evidence instead of requiring plaintext data.
///
/// Example: an agent proves its reputation is above 0.7 without
/// revealing the exact score.
pub struct ZkVerify {
    id: CellId,

    /// The verification key for this proof type.
    /// Compiled once from the ZK circuit; used to verify proofs.
    verifying_key: VerifyingKey,
}

#[async_trait]
impl Verify for ZkVerify {
    type Evidence = ZkEvidence;

    async fn verify_pre(&self, signal: &Signal) -> Verdict {
        let evidence = signal.evidence::<ZkEvidence>()?;

        // The proof asserts a statement about private data without
        // revealing the data itself.
        let valid = verify_proof(
            &self.verifying_key,
            &evidence.proof,
            &evidence.public_inputs,
        );

        if valid {
            Verdict::Pass {
                confidence: 1.0, // ZK proofs are binary: valid or invalid
                evidence: evidence.clone(),
            }
        } else {
            Verdict::Reject {
                reason: "ZK proof verification failed",
                evidence: ZkEvidence::invalid(),
            }
        }
    }
}

/// Evidence types for privacy-preserving verification.
pub enum ZkEvidence {
    /// Range proof: value is within [min, max] without revealing exact value.
    /// Use case: "My bid is between 100 and 1000 KORAI."
    RangeProof {
        proof: Vec<u8>,
        public_inputs: RangePublicInputs,
    },

    /// Threshold proof: value exceeds a minimum without revealing exact value.
    /// Use case: "My reputation in security domain is above 0.7."
    ThresholdProof {
        proof: Vec<u8>,
        public_inputs: ThresholdPublicInputs,
    },

    /// HDC similarity proof: two vectors have similarity above threshold
    /// without revealing either vector.
    /// Use case: "My knowledge entry is relevant to this query."
    SimilarityProof {
        proof: Vec<u8>,
        public_inputs: SimilarityPublicInputs,
    },
}
```

### HDC Similarity Without Content Revelation

The most powerful application of ZK in the Korai ecosystem: proving that a knowledge entry is relevant (HDC similarity above threshold) without revealing the content. This enables private knowledge markets:

```rust
/// PP-HDC (Privacy-Preserving HDC): prove similarity without revealing vectors.
///
/// The prover has: private_vector (their knowledge entry's HDC fingerprint)
/// The verifier has: query_vector (the query's HDC fingerprint), threshold
///
/// The proof asserts: hamming_similarity(private_vector, query_vector) >= threshold
/// Without revealing: private_vector
pub struct HdcSimilarityCircuit {
    /// The private witness: the agent's knowledge HDC vector.
    private_vector: [u64; 160], // 10,240-bit BSC vector

    /// Public input: the query vector (known to verifier).
    query_vector: [u64; 160],

    /// Public input: minimum similarity threshold.
    threshold: f64,
}
```

This is expressed entirely within the existing Verify protocol. A Score Cell asks "is this knowledge entry relevant?" and receives a `Verdict::Pass` with ZK evidence, without ever seeing the knowledge content. The Store Cell that holds the knowledge entry remains private; only the relevance proof is shared.

---

## 6. Ordering Guarantees as Bus Properties

The source architecture describes three ordering levels (causal via vector clocks, total via block inclusion, convergent via CRDTs). In unified terms, these are properties of the Bus transport layer, not separate mechanisms:

```rust
/// Bus delivery semantics (per-topic configurable).
pub enum BusOrdering {
    /// Unordered: Pulses delivered as they arrive.
    /// Cheapest. Used for heartbeats.
    Unordered,

    /// Causal: if Pulse A causally precedes Pulse B, A delivered before B.
    /// Implemented via vector clocks piggybacked on Pulses.
    /// Used for knowledge confirmations, job lifecycle.
    Causal,

    /// Total: all subscribers see Pulses in the same global order.
    /// Only available at DeliveryTier::Canonical (on-chain).
    /// Used for reputation updates, slashing, governance.
    Total,
}

/// CRDT state types for convergent replication.
/// These ride on the Bus as special Pulses that merge rather than replace.
pub enum CrdtPulse {
    /// Grow-only counter (e.g., confirmation count for knowledge entries).
    GCounter { agent_id: u256, increment: u64 },

    /// Last-writer-wins register (e.g., agent status).
    LwwRegister { value: Vec<u8>, timestamp: u64, writer: u256 },

    /// Observed-remove set (e.g., set of confirmed anomalies).
    OrSet { add: Option<Vec<u8>>, remove: Option<Vec<u8>>, tag: u64 },
}
```

---

## 7. Subscription Privacy as React Configuration

An agent's Bus subscriptions reveal its capabilities (if you subscribe to `jobs.trading.*`, you are a trading agent). The source architecture proposes mitigations (cover topics, cover traffic, epoch-locked changes). In unified terms, these are configuration properties of the React protocol on the BusFederation Cell:

```toml
# Privacy configuration for Bus federation (in roko.toml)
[bus.federation.privacy]
# Subscribe to 2 additional random topics as cover traffic
cover_topics = 2

# Generate dummy Pulses on cover topics at this rate (per heartbeat)
cover_traffic_rate = 0.1

# Only change subscriptions at epoch boundaries (not reactively)
epoch_locked_subscriptions = true

# Topics using Dandelion++ stem routing (hide originator)
dandelion_topics = ["challenge.*", "pheromone.*", "jobs.*.bid"]

# Dandelion stem length (average hops before fluff broadcast)
dandelion_stem_probability = 0.1  # ~10 hops average
```

Dandelion++ (Fanti et al., 2018) is the privacy overlay for message origination. In unified terms, it is a **Functor** (cross-cut) on the publish path: before a Pulse enters the mesh broadcast, it traverses a random stem path that hides the originator.

---

## 8. End-to-End Example: Knowledge Publication with Privacy

```
Agent A discovers an insight (private knowledge entry).

1. Agent A computes HDC fingerprint of the insight.

2. Agent A wants to check if similar knowledge exists (without revealing content):
   - Constructs ZK similarity proof: "my entry has similarity < 0.8 to all
     existing entries" (proving non-duplication without revealing content).
   - Submits proof to a ZkVerify Cell.
   - Verdict::Pass -> entry is novel.

3. Agent A publishes knowledge announcement on federated Bus:
   - Topic: knowledge.published
   - Tier: DeliveryTier::BestEffort (fast broadcast)
   - Privacy: Dandelion++ stem routing (hide which agent published)
   - Payload: HDC vector + domain + confidence (NOT full content)

4. Other agents receive the announcement Pulse:
   - Score Cells evaluate relevance to their domains.
   - Interested agents publish confirmation Pulses (GCounter CRDT).
   - Confirmation count converges across mesh.

5. When confirmation count reaches threshold (GCounter.total() >= 3):
   - Graduation: Pulse -> Signal.
   - The knowledge entry is submitted at DeliveryTier::Canonical (on-chain tx).
   - Agent Registry records attribution (passport_id gets reputation credit).

6. Privacy properties maintained throughout:
   - Content is never broadcast (only HDC vector + metadata).
   - Originator is hidden via Dandelion++ (other agents cannot identify A).
   - Relevance is verifiable via ZK proofs (no need to reveal full entry).
   - On-chain record contains only hash + HDC vector (not plaintext).
```

---

## 9. Implementation Surface

| Component | Crate | Status |
|---|---|---|
| Bus (local) | `crates/roko-runtime/` | Wired |
| Bus federation (network) | `crates/roko-chain/` | Tier 6 (not yet built) |
| GossipSub mesh | External: `libp2p-gossipsub` | Available |
| HyParView membership | `crates/roko-chain/` | Tier 6 |
| PlumTree dissemination | `crates/roko-chain/` | Tier 6 |
| Passport Signal | `crates/roko-chain/` | Struct defined, contract not deployed |
| VentriloquistVerify Cell | `crates/roko-chain/` | Tier 6 |
| ZkVerify Cell | `crates/roko-chain/` | Tier 6 |
| HDC similarity ZK circuit | `crates/roko-primitives/` | Tier 6 (HDC ops built, ZK circuit not) |
| Dandelion++ overlay | `crates/roko-chain/` | Tier 6 |
| CRDT Pulse types | `crates/roko-chain/` | Tier 6 |

---

## What This Enables

1. **Unified subscription model**: Cells subscribe to `knowledge.*` and receive both local Pulses (from in-process Bus) and federated Pulses (from network gossip) through the same API. No dual subscription mechanisms.

2. **Privacy as a dial, not a mode**: Any Signal or Pulse can carry privacy-preserving evidence. A Score Cell can rate knowledge relevance using ZK proofs without ever seeing content. Privacy is not a separate system -- it is an evidence type.

3. **Composable gossip topics**: Because gossip topics ARE Bus topics, all existing Bus machinery (topic filtering, ring buffers, sequence numbers, back-pressure) applies to network communication without modification.

4. **Sybil-resistant coordination**: Passport-based identity (soulbound, staked, reputation-bearing) makes every Bus participant accountable. False heartbeats, spam announcements, and reputation manipulation are economically punished.

5. **Graduated trust**: Pulses start ephemeral and tentative; those that achieve confirmation graduate to durable Signals. The same Graduation primitive handles both in-process event promotion and network-wide consensus.

---

## Feedback Loops

1. **Peer scoring from gossip behavior**: Agents that send false alerts, spam heartbeats, or consistently publish low-quality knowledge Pulses see their peer scores (and thus mesh connectivity) degrade. This feeds back into routing: poorly-connected agents receive fewer job assignments.

2. **Subscription adaptation from relevance**: If an agent subscribes to a topic but never acts on its Pulses (low relevance), the BusFederation Cell can suggest unsubscription to reduce bandwidth. Conversely, frequent engagement with a topic's Pulses suggests the subscription set should expand to related topics.

3. **Privacy cost feedback**: ZK proof generation is expensive. If an agent frequently uses P3 privacy for low-value operations, the cost feedback loop (via efficiency events) suggests downgrading to P1 or P0 for those operations.

4. **Ventriloquist defense calibration**: Rate-limiting thresholds for prompt updates (3 per 30 days) can be adjusted via adaptive gate thresholds based on observed legitimate update patterns across the network.

---

## Open Questions

1. **Gossip partition tolerance**: If the mesh partitions (two disconnected subgraphs), Pulses in one partition never reach the other. What is the reconciliation protocol when partitions heal? CRDTs converge automatically, but non-CRDT Pulses may have expired from ring buffers.

2. **ZK circuit compilation cost**: Compiling a new ZK circuit (e.g., for a novel proof type) takes minutes to hours. Should circuits be pre-compiled and distributed as part of the protocol spec, or should agents compile them on demand?

3. **Dandelion++ latency trade-off**: Stem routing adds 3-10 hops of latency before a Pulse enters broadcast. For time-sensitive anomaly alerts, this delay could be dangerous. Should certain topic priorities bypass Dandelion++?

4. **Passport revocation propagation**: If a passport is revoked (slashed to zero stake, or governance ban), how quickly does the mesh reject Pulses from that passport? Clock skew between mesh peers means some will enforce revocation before others.

5. **Cover traffic distinguishability**: Dummy Pulses for subscription privacy must be statistically indistinguishable from real Pulses. If dummy Pulses are simpler (shorter payload, no causal dependencies), traffic analysis can filter them out. How to generate realistic cover traffic at low cost?

---

## Implementation Tasks

1. **Define `BusFederation` Cell** in `crates/roko-chain/src/gossip/mod.rs` implementing Connect + React protocols.

2. **Implement HyParView membership** as the `connect()` / `disconnect()` lifecycle of BusFederation, using `libp2p-gossipsub` as the underlying transport.

3. **Implement PlumTree dissemination** as the publish strategy, constructing an overlay tree from the active view for bandwidth-efficient broadcast.

4. **Define `SignedPulse` envelope** that wraps Pulses crossing network boundaries with passport-bound Ed25519 signatures.

5. **Implement `VentriloquistVerify` Cell** that checks system prompt hash against on-chain commitment before job execution.

6. **Define `ZkVerify` Cell** with pluggable circuit backends (Groth16, PLONK, STARK) for privacy-preserving evidence verification.

7. **Implement HDC similarity ZK circuit** using `halo2` or `plonky2` that proves Hamming similarity above threshold without revealing the private vector.

8. **Add `DeliveryTier` and `BusOrdering` configuration** to Bus topic definitions in `roko.toml`, so topics can declare their latency and ordering requirements.

9. **Implement Dandelion++ Functor** as a cross-cut on the publish path for sensitive topics.

10. **Implement CRDT Pulse types** (GCounter, LWW-Register, OR-Set) as special Pulse variants that merge rather than replace on the Bus.
