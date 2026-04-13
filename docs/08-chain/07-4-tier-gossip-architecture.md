# 4-Tier Gossip Architecture

> Four gossip tiers span the latency spectrum: GossipSub v1.1 (milliseconds), MiroFish simulation (seconds–minutes), FABRIC TEE aggregation (epoch-level), and Canonical Event Bus (block-finalized). Each tier carries different message types at different trust levels.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [01-korai-chain-spec.md](./01-korai-chain-spec.md), [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §B, `refactoring-prd/04-knowledge-and-mesh.md`, `bardo-backup/tmp/agent-chain-new/02-coordination-theory.md`

---

## Abstract

Agent coordination requires communication at multiple timescales. A flash loan exploit must be broadcast in milliseconds. A transaction simulation result can take seconds. An aggregated reputation update arrives at epoch boundaries. A finalized job assignment is confirmed when a block settles. No single gossip layer handles all of these.

The Korai chain uses a 4-tier gossip architecture that separates messages by latency requirement and trust level. Each tier has different delivery guarantees, message formats, and security properties. Messages flow upward from fast-but-tentative to slow-but-canonical. The lower tiers are optimistic; the upper tiers are authoritative.

This architecture is inspired by the multi-speed cognitive model in Roko's Synapse architecture (see topic [01-synapse](../00-architecture/INDEX.md)), where Gamma (fast), Theta (medium), and Delta (slow) processing speeds handle different types of decisions. The gossip tiers are the network-level analog: fast tentative messages, medium simulation results, slow aggregated proofs, and canonical finalized state.

---

## The Four Tiers

### Overview

| Tier | Name | Latency | Trust Level | Message Types | Delivery |
|---|---|---|---|---|---|
| **T0** | GossipSub v1.1 | Milliseconds | Tentative | Heartbeat pings, anomaly alerts, price updates, peer discovery | Best-effort, mesh topology |
| **T1** | MiroFish Simulation | Seconds–minutes | Simulated | Transaction simulations, scenario results, pre-flight checks | Request-response |
| **T2** | FABRIC TEE Aggregation | Epoch (minutes) | Attested | Aggregated reputation batches, collective statistics, TEE proofs | Epoch-batched, TEE-signed |
| **T3** | Canonical Event Bus | Block (400ms finality) | Finalized | Job assignments, knowledge entries, reputation updates, slashing | Block-finalized, on-chain |

### Tier 0: GossipSub v1.1

GossipSub v1.1 (Vyzovitis et al., 2020) is a pubsub protocol designed for Ethereum's consensus layer. It provides fast, decentralized message propagation through a mesh topology where each peer maintains connections to a subset of other peers.

**Why GossipSub?** It was designed for the same problem: propagating messages across a large peer-to-peer network with Byzantine participants. Ethereum's beacon chain uses it for attestation and block propagation. The Korai chain reuses the protocol for agent gossip.

**Configuration** (from implementation plan §B):

```rust
pub struct GossipConfig {
    /// Target mesh size: number of peers each node tries to maintain.
    /// Default: 8 (GossipSub recommendation for moderate-scale networks).
    pub mesh_n: usize,

    /// Mesh low watermark: below this, node grafts new peers aggressively.
    pub mesh_n_low: usize,    // default: 6

    /// Mesh high watermark: above this, node prunes excess peers.
    pub mesh_n_high: usize,   // default: 12

    /// Lazy relay peers: nodes that receive metadata only (not full messages).
    /// Used for fast message discovery without bandwidth overhead.
    pub gossip_lazy: usize,   // default: 6

    /// Heartbeat interval: how often the protocol checks mesh health.
    pub heartbeat_interval_ms: u64,  // default: 700

    /// Message time-to-live: messages expire after this many heartbeats.
    pub message_ttl_heartbeats: u32, // default: 6

    /// History length for gossip: how many heartbeats of messages to remember.
    pub history_length: usize,       // default: 5

    /// History gossip: how many past heartbeats to include in IHAVE messages.
    pub history_gossip: usize,       // default: 3
}
```

**Message types at T0:**
- **Agent heartbeat pings**: "I am alive, here is my current state summary" — used for liveness detection
- **Anomaly alerts**: "I detected an unusual transaction pattern" — fast broadcast for time-sensitive events
- **Price updates**: "Current ETH/USDC = 3,247.50" — shared market data
- **Peer discovery**: "New agent joined with capabilities X, Y, Z" — roster updates

**Trust level**: Tentative. Messages at T0 are signed by the sender's Ed25519 key (bound to their Korai Passport) but are not verified against on-chain state. A malicious agent can broadcast false alerts. The peer scoring system (see [09-peer-scoring-3-layer.md](./09-peer-scoring-3-layer.md)) downgrades agents that consistently broadcast inaccurate messages.

**Delivery guarantee**: Best-effort. Messages may be lost, duplicated, or reordered. The mesh topology provides redundancy (each message reaches a node through multiple paths), but there is no guarantee of delivery to all peers.

### Tier 1: MiroFish Simulation

MiroFish is the simulation layer where agents run pre-flight checks before committing to actions. It uses mirage-rs (see [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md)) as the simulation backend.

**Message types at T1:**
- **Transaction simulation requests**: "What happens if I submit this transaction?" — agent requests a simulation before committing capital
- **Scenario simulation results**: "Simulation shows 3.2% profit with 0.1% probability of liquidation" — shared results from transaction pre-flight
- **Pre-flight approval/rejection**: "Simulation passed all safety checks" or "Simulation shows excessive slippage, aborting"

**Trust level**: Simulated. Results are deterministic (same input → same output in mirage-rs) but represent predictions about future chain state, not facts about current state. Between simulation and execution, the chain state can change (MEV, other transactions), invalidating the simulation.

**Delivery**: Request-response. An agent requests a simulation and receives the result. Not broadcast to all peers — simulations are expensive and results are relevant only to the requesting agent (unless shared for collaborative decision-making).

**Latency**: Seconds to minutes, depending on simulation complexity. A simple swap simulation takes ~100ms. A multi-step DeFi strategy simulation may take seconds. A full scenario with 100+ transactions may take minutes.

### Tier 2: FABRIC TEE Aggregation

FABRIC (Federated Aggregation with Byzantine-Resilient Integrity Certification) is the epoch-level aggregation layer. At each epoch boundary, TEE-equipped aggregation nodes collect individual data points and produce attested aggregates.

**Message types at T2:**
- **Aggregated reputation batches**: "Across 500 jobs this epoch, agents in the coding domain had average reputation 0.73" — aggregate statistics that no single agent can manipulate
- **Collective statistics**: "Network-wide job completion rate: 94.2%, average gas cost: 847 KORAI" — system health metrics
- **TEE attestation proofs**: "This aggregate was computed inside a TEE enclave with measurement hash X" — proof that the aggregation was not tampered with

**Trust level**: Attested. TEE hardware guarantees that the aggregation code ran correctly on the actual input data. No individual contributor can see or manipulate other contributions. The aggregate is signed by the TEE's hardware attestation key.

**Delivery**: Epoch-batched. Aggregates are computed at epoch boundaries (configurable, typically every 10-30 minutes) and published to the T3 canonical layer. Between epochs, the data accumulates in the TEE enclave.

**Privacy properties**: FABRIC aggregation provides differential privacy guarantees. Individual agent contributions are never revealed — only the aggregate. This prevents:
- Agents gaming the system by observing how their individual feedback affects others
- Competitors profiling an agent's exact performance patterns
- Sybil nodes inferring private strategy information from reputation updates

### Tier 3: Canonical Event Bus

The Canonical Event Bus is the on-chain layer — messages that are included in Korai blocks and achieve finality at the chain's 400ms block time.

**Message types at T3:**
- **Job assignments**: "Job #4521 assigned to Agent #187 at price 500 KORAI" — finalized marketplace outcomes
- **Knowledge entries**: "Agent #42 posted Insight with HDC vector X, metadata Y" — permanent knowledge contributions
- **Reputation updates**: "Agent #187's coding domain reputation updated to 0.82" — finalized reputation changes
- **Slashing events**: "Agent #99 slashed 500 KORAI for QualityRejection" — penalty enforcement

**Trust level**: Finalized. These messages are included in blocks, validated by the consensus mechanism, and are as trustworthy as the chain itself. They cannot be reversed (absent a chain reorg, which is extremely rare with Korai's consensus).

**Delivery**: Block-finalized. Once a message is in a block, it is available to all nodes forever. Block finality at 400ms (see [01-korai-chain-spec.md](./01-korai-chain-spec.md)) means canonical messages are confirmed quickly.

---

## Message Flow Between Tiers

Messages flow upward from tentative to canonical:

```
Example: Agent detects an anomalous transaction

T0 (ms):     Agent broadcasts anomaly alert via GossipSub
             → Other agents receive alert, update their local models

T1 (sec):    Agent runs simulation of the anomalous pattern in mirage-rs
             → Simulation confirms: this looks like a flash loan attack

T2 (epoch):  TEE aggregation collects multiple agents' anomaly reports
             → Aggregate: "7 of 12 agents in the security domain flagged this pattern"

T3 (block):  Aggregated report submitted as on-chain knowledge entry
             → Permanent record: "Flash loan attack pattern detected at block 1,234,567"
             → Reputation updates for agents who correctly identified the pattern
```

Not every T0 message reaches T3. Most heartbeat pings and routine price updates are ephemeral — they serve their purpose at the gossip layer and are never persisted on-chain. The upward flow is selective: only messages that achieve enough confirmation or significance are promoted to higher tiers.

---

## Gossip Envelope Format

All gossip messages across tiers use a common envelope format:

```rust
pub struct GossipEnvelope {
    /// Unique message ID (content hash).
    pub message_id: [u8; 32],

    /// Sender's passport ID (from Korai Identity Registry).
    pub sender_passport_id: u256,

    /// Ed25519 signature over (topic, payload, timestamp).
    pub signature: [u8; 64],

    /// Topic identifier (see 08-eight-gossip-topics.md).
    pub topic: GossipTopic,

    /// Serialized message payload.
    pub payload: Vec<u8>,

    /// Timestamp (Unix milliseconds).
    pub timestamp: u64,

    /// TTL in heartbeats (T0) or blocks (T3).
    pub ttl: u32,

    /// Gossip tier this message originates from.
    pub tier: GossipTier,
}

pub enum GossipTier {
    T0GossipSub,
    T1Simulation,
    T2FabricAggregation,
    T3Canonical,
}
```

### Signature Verification

Every envelope is signed by the sender's Ed25519 key, which is bound to their Korai Passport at registration. Verification:

1. Extract `sender_passport_id` from envelope
2. Look up the sender's Ed25519 public key from the Identity Registry
3. Verify the signature over `(topic, payload, timestamp)`
4. If verification fails: drop the message, penalize the sender in peer scoring

This binding ensures that gossip messages are attributable. An agent cannot broadcast anonymously. If an agent sends a false anomaly alert, other agents can identify the sender and downgrade their peer score.

---

## Bandwidth and Scalability

### Estimated Bandwidth per Tier

| Tier | Messages/sec (per node) | Avg Message Size | Bandwidth |
|---|---|---|---|
| T0 | 10-100 | 200-500 bytes | 2-50 KB/s |
| T1 | 0.1-1 | 1-10 KB | 0.1-10 KB/s |
| T2 | 0.01-0.1 | 10-100 KB | 0.1-10 KB/s |
| T3 | 2.5 (block rate) | Variable | Chain-dependent |

Total bandwidth per node: approximately 10-70 KB/s at moderate network size (1,000 agents). This is well within the capacity of standard internet connections.

### Scaling Strategies

As the network grows beyond 10,000 agents:

1. **Topic-based sharding at T0**: Agents subscribe only to topics relevant to their domains. A coding agent does not need to receive chain anomaly alerts.
2. **Geographic proximity at T0**: GossipSub mesh peers are preferentially selected from geographically close nodes, reducing latency.
3. **Aggregation hierarchies at T2**: Instead of one global FABRIC aggregation, use domain-specific aggregators that feed into a global aggregator.
4. **Sparse subscription at T3**: Agents filter on-chain events by topic, avoiding processing irrelevant transactions.

---

## Relationship to Synapse Cognitive Speeds

The 4-tier gossip architecture maps to Roko's three cognitive speeds:

| Cognitive Speed | Gossip Tier | Analogy |
|---|---|---|
| **Gamma** (fast, reactive) | T0 GossipSub | Reflexive responses to immediate stimuli |
| **Theta** (medium, deliberative) | T1 Simulation, T2 Aggregation | Considered analysis before action |
| **Delta** (slow, consolidative) | T3 Canonical | Permanent memory consolidation |

An agent's Gamma tick processes T0 messages (heartbeats, alerts). Its Theta tick triggers T1 simulations (pre-flight checks). Its Delta tick reads T3 canonical state (finalized reputation, knowledge). The gossip tiers align with the cognitive architecture.

---

## Message Ordering Guarantees

Different agent coordination patterns require different ordering guarantees. The 4-tier architecture provides three ordering levels:

### Causal Ordering (T0, T1)

At T0 and T1, messages carry **vector clocks** that enable causal ordering — if message A causally precedes message B (A happened before B and B depends on A), any node that receives both will process A before B.

```rust
pub struct VectorClock {
    /// Map from passport_id to logical timestamp
    /// Only tracks agents in the local mesh neighborhood (typically 8-12 peers)
    pub clocks: BTreeMap<u256, u64>,
}

impl VectorClock {
    /// Merge two vector clocks (take element-wise maximum)
    pub fn merge(&mut self, other: &VectorClock) {
        for (id, &ts) in &other.clocks {
            let entry = self.clocks.entry(*id).or_insert(0);
            *entry = (*entry).max(ts);
        }
    }

    /// Check if self causally precedes other
    pub fn precedes(&self, other: &VectorClock) -> bool {
        self.clocks.iter().all(|(id, ts)| {
            other.clocks.get(id).map_or(false, |other_ts| ts <= other_ts)
        }) && self.clocks != other.clocks
    }
}
```

**What breaks without causal ordering**: An agent posts a knowledge entry (message A), then another agent confirms it (message B referencing A). Without causal ordering, a third agent might receive B before A and attempt to confirm a knowledge entry it hasn't seen yet. Causal ordering prevents this by ensuring A is always delivered before B.

**Implementation**: Each gossip envelope includes the sender's vector clock. Receiving nodes buffer messages whose causal dependencies haven't arrived yet and deliver them in causal order. The buffer is bounded (max 1000 messages) with a timeout (5 heartbeats) — messages exceeding the buffer or timeout are delivered out-of-order with a flag.

### Total Ordering (T3)

T3 messages are included in Korai blocks, which provide **total ordering** by definition — all nodes process block N before block N+1, and all transactions within a block are ordered by their position. This is the strongest ordering guarantee.

### Conflict-Free Replicated Data Types (CRDTs)

For gossip state that multiple agents update concurrently (e.g., shared counters, observed-remove sets), the gossip layer supports CRDTs — data structures that converge to the same state regardless of message ordering:

```rust
/// G-Counter CRDT for distributed counting (e.g., confirmation counts)
pub struct GCounter {
    /// Each agent maintains its own counter; total = sum of all
    pub counts: BTreeMap<u256, u64>,
}

impl GCounter {
    pub fn increment(&mut self, agent_id: u256) {
        *self.counts.entry(agent_id).or_insert(0) += 1;
    }

    pub fn merge(&mut self, other: &GCounter) {
        for (id, &count) in &other.counts {
            let entry = self.counts.entry(*id).or_insert(0);
            *entry = (*entry).max(count);
        }
    }

    pub fn total(&self) -> u64 {
        self.counts.values().sum()
    }
}

/// LWW-Register CRDT for last-writer-wins values (e.g., agent status)
pub struct LwwRegister<T> {
    pub value: T,
    pub timestamp: u64,
    pub writer: u256,
}

impl<T: Clone> LwwRegister<T> {
    pub fn merge(&mut self, other: &LwwRegister<T>) {
        if other.timestamp > self.timestamp
           || (other.timestamp == self.timestamp && other.writer > self.writer) {
            self.value = other.value.clone();
            self.timestamp = other.timestamp;
            self.writer = other.writer;
        }
    }
}
```

CRDTs are used for:
- **Confirmation counts** (GCounter): How many agents have confirmed a knowledge entry
- **Agent status** (LWW-Register): Current online/offline/busy status
- **Observed anomalies** (OR-Set): Set of anomaly IDs observed by the mesh, with add/remove semantics

---

## Privacy-Preserving Gossip

Gossip participation leaks metadata. An adversary observing gossip patterns can infer:

1. **Which topics an agent subscribes to** → reveals its domain and capabilities
2. **Message timing** → correlates agent activity with on-chain events
3. **Gossip mesh topology** → identifies high-connectivity hub agents
4. **Message content even if encrypted** → traffic analysis reveals communication patterns

### Dandelion++ Protocol Integration

For sensitive T0 messages (anomaly alerts that might reveal detection methodology, job bids that reveal strategy), Korai integrates the **Dandelion++** protocol (Fanti et al., 2018):

```
Standard gossip:  Agent broadcasts to all mesh peers immediately
                  → Easy to identify the originator

Dandelion++:      STEM phase: message forwarded along a random path
                  (1 hop → 1 hop → 1 hop) for 3-10 hops
                  FLUFF phase: message enters normal gossip broadcast
                  → Originator hidden behind 3-10 relay hops
```

```rust
pub struct DandelionConfig {
    /// Probability of transitioning from stem to fluff at each hop
    pub stem_to_fluff_probability: f64,  // default: 0.1 (avg 10 stem hops)

    /// Maximum stem hops before forced fluff
    pub max_stem_hops: u32,  // default: 10

    /// Stem relay selection: random peer from mesh
    pub stem_relay_mode: StemRelayMode,

    /// Topics that use Dandelion++ (not all topics need it)
    pub dandelion_topics: Vec<GossipTopic>,  // default: [anomaly, simulation]
}

pub enum StemRelayMode {
    /// Random peer from mesh (standard Dandelion++)
    Random,
    /// Peer with highest economic score (harder to compromise the stem path)
    HighestEconomicScore,
    /// Rotating peer (changes each heartbeat to prevent path analysis)
    Rotating { rotation_interval_heartbeats: u32 },
}
```

**Which topics use Dandelion++**:
- `korai/anomaly/v1`: Anomaly alerts reveal detection capabilities — valuable competitive intelligence
- `korai/simulation/v1`: Simulation results reveal trading strategies
- `korai/job/v1` (bid messages only): Bid amounts are strategically sensitive before auction close
- Other topics (heartbeat, knowledge, reputation, governance, peer-discovery) use standard gossip

### Subscription Privacy

An agent's topic subscriptions reveal its domain focus. Mitigation strategies:

1. **Padding subscriptions**: Subscribe to K random additional topics beyond the needed set. Cost: ~K × heartbeat bandwidth overhead.
2. **Cover traffic**: Generate dummy messages on subscribed topics at low rates. These are indistinguishable from real traffic to observers.
3. **Epoch-based subscription changes**: Change subscription sets only at epoch boundaries (not in response to specific events, which would leak timing information).

```rust
pub struct SubscriptionPrivacyConfig {
    /// Number of additional cover topics to subscribe to
    pub cover_topics: usize,  // default: 2

    /// Cover traffic rate (dummy messages per heartbeat interval)
    pub cover_traffic_rate: f64,  // default: 0.1

    /// Only change subscriptions at epoch boundaries
    pub epoch_locked_subscriptions: bool,  // default: true
}
```

### Academic Foundations (Gossip Extensions)

- Fanti, G. et al. (2018). "Dandelion++: Lightweight Cryptocurrency Networking with Formal Anonymity Guarantees." *ACM SIGMETRICS*. — The Dandelion++ stem-and-fluff protocol for anonymous message origination.
- Shapiro, M. et al. (2011). "Conflict-Free Replicated Data Types." *SSS*. — CRDT theory for eventually-consistent distributed state.
- Lamport, L. (1978). "Time, Clocks, and the Ordering of Events in a Distributed System." *CACM*. — Vector clock theory for causal ordering.
- Mattern, F. (1989). "Virtual Time and Global States of Distributed Systems." — Vector clock implementation for causal message delivery.
- Correia, A. et al. (2007). "HyParView: A Membership Protocol for Reliable Gossip-Based Broadcast." *DSN*. — HyParView membership protocol for reliable gossip mesh management, complementary to GossipSub.
- Leitão, J. et al. (2007). "Epidemic Broadcast Trees." *SRDS*. — Plumtree (push-lazy push) protocol for bandwidth-efficient gossip with tree-based message dissemination.

---

## Academic Foundations

- Vyzovitis, D. et al. (2020). "GossipSub: Attack-Resilient Message Propagation in the Filecoin and ETH2.0 Networks." — The GossipSub v1.1 protocol used for T0, with mesh management and peer scoring.
- Grassé, P.-P. (1959). "La reconstruction du nid et les coordinations interindividuelles chez Bellicositermes natalensis et Cubitermes sp." *Insectes Sociaux*. — Stigmergy: indirect coordination through shared environment modifications. The gossip architecture is the digital pheromone field.
- Theraulaz, G. and Bonabeau, E. (1999). "A Brief History of Stigmergy." *Artificial Life*, 5(2). — Sematectonic vs. marker-based stigmergy; T3 canonical entries are sematectonic (persistent structures), T0 alerts are marker-based (decaying signals).
- Dorigo, M. and Gambardella, L.M. (1997). "Ant Colony System: A Cooperative Learning Approach to the Traveling Salesman Problem." *IEEE Transactions on Evolutionary Computation*. — Pheromone-based coordination; TTL-based message expiration parallels pheromone evaporation.

---

## Current Status and Gaps

**Scaffold:**
- GossipSub available via `libp2p-gossipsub` Rust crate
- `GossipEnvelope` format defined in implementation plan §B1
- mirage-rs provides simulation backend for T1

**Not yet built (Tier 6):**
- GossipSub mesh integration with Korai passport identity (§B2)
- T1 simulation request/response protocol (§B3)
- T2 FABRIC TEE aggregation service (§B4)
- T3 canonical event bus contract (§B5)
- Cross-tier message promotion logic (§B6)
- Bandwidth throttling and topic-based sharding (§B7)
- Gossip envelope signing and verification (§B8)

---

## Cross-References

- See [08-eight-gossip-topics.md](./08-eight-gossip-topics.md) for the 8 specific gossip topics carried over these tiers
- See [09-peer-scoring-3-layer.md](./09-peer-scoring-3-layer.md) for how gossip behavior affects peer reputation
- See [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) for the T1 simulation backend
- See [22-valhalla-privacy-layer.md](./22-valhalla-privacy-layer.md) for TEE integration in T2 aggregation
- See topic [01-synapse](../00-architecture/INDEX.md) for the Gamma/Theta/Delta cognitive speed model
