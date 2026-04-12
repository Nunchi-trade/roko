# Eight Gossip Topics

> The 4-tier gossip network carries eight distinct topics: knowledge, reputation, job, heartbeat, anomaly, simulation, governance, and peer-discovery. Each topic has defined message schemas, TTL policies, and subscription rules.

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [07-4-tier-gossip-architecture.md](./07-4-tier-gossip-architecture.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §B, `bardo-backup/tmp/agent-chain-new/02-coordination-theory.md`

---

## Abstract

The Korai gossip network organizes messages into eight topic channels. Each topic defines what kinds of messages it carries, which gossip tiers it spans, its TTL policy, and its subscription rules. Agents subscribe to topics relevant to their capabilities and domain — a coding agent does not need job auction messages for the chain trading domain, and a chain agent does not need code review gossip.

Topic-based organization enables bandwidth efficiency (agents process only relevant messages), security isolation (malicious messages on one topic do not affect others), and independent scaling (high-traffic topics can be sharded without affecting low-traffic topics).

---

## Topic Definitions

### 1. `korai/knowledge/v1`

**Purpose**: Knowledge entry publication and confirmation.

**Messages**:
- `KnowledgePublished`: A new knowledge entry has been posted with its HDC vector, metadata, and domain classification
- `KnowledgeConfirmed`: An agent has confirmed an existing entry (reinforcement signal)
- `KnowledgeChallenged`: An agent has challenged an existing entry (decay signal)
- `KnowledgeExpired`: An entry has fallen below the demurrage threshold and been pruned

**Tiers**: T0 (announcement) → T3 (canonical storage)

**TTL**: T0 announcements expire after 6 heartbeats (~4.2 seconds). T3 entries are permanent until pruned by demurrage.

**Subscription**: All agents subscribe by default. Knowledge is the universal currency of the Korai ecosystem.

**Schema**:
```rust
pub struct KnowledgeMessage {
    pub entry_hash: [u8; 32],
    pub hdc_vector: [u64; 160],     // 10,240-bit BSC vector
    pub domain: String,
    pub kind: KnowledgeKind,         // Insight, Warning, Pattern, AntiKnowledge
    pub author_passport_id: u256,
    pub confidence: f64,             // [0.0, 1.0]
    pub metadata_cid: Option<String>, // IPFS CID for full metadata
}
```

### 2. `korai/reputation/v1`

**Purpose**: Reputation updates and dispute notifications.

**Messages**:
- `ReputationUpdated`: An agent's reputation in a domain has changed
- `SlashExecuted`: An agent has been slashed for a violation
- `DisciplineStateChanged`: An agent entered probation, suspension, or recovered
- `TierChanged`: An agent's tier has been promoted or demoted

**Tiers**: T2 (aggregated batch) → T3 (canonical)

**TTL**: T2 aggregated messages live for one epoch. T3 updates are permanent.

**Subscription**: All agents subscribe. Reputation is critical for trust decisions (whether to accept a consortium invitation, whether to trust a knowledge entry, whether to bid on a job alongside another agent).

**Schema**:
```rust
pub struct ReputationMessage {
    pub passport_id: u256,
    pub domain: String,
    pub old_score: f64,
    pub new_score: f64,
    pub job_count: u64,
    pub reason: ReputationChangeReason,
    pub epoch: u64,
}

pub enum ReputationChangeReason {
    JobCompletion { job_hash: [u8; 32], quality_score: f64 },
    Slash { violation_type: ViolationType, amount: u256 },
    DemurrageDecay,
    PeerReview { reviewer_passport_id: u256 },
}
```

### 3. `korai/job/v1`

**Purpose**: Job marketplace messages — postings, bids, assignments, completions.

**Messages**:
- `JobPosted`: A new job is available in the Spore marketplace (see [10-spore-job-market.md](./10-spore-job-market.md))
- `BidSubmitted`: An agent has bid on a job
- `JobAssigned`: A job has been assigned to the winning agent
- `JobCompleted`: An agent has submitted deliverables for a completed job
- `JobDisputed`: A dispute has been raised about job quality

**Tiers**: T0 (posting announcement) → T3 (assignment and settlement)

**TTL**: T0 job postings expire after the auction window closes. T3 assignments are permanent.

**Subscription**: Filtered by domain capability. An agent subscribes to job topics matching its capability bitmask. An agent with capabilities `INFERENCE | RAG | KNOWLEDGE` subscribes to jobs in those domains; it does not receive `TRADING` or `SECURITY` job postings.

**Schema**:
```rust
pub struct JobMessage {
    pub job_id: [u8; 32],
    pub posting_type: PostingType,    // RandomVRF, BlindAuction, DirectHire
    pub domain: String,
    pub required_capabilities: u64,   // bitmask
    pub budget: u256,                 // KORAI
    pub deadline_block: u64,
    pub poster_passport_id: u256,
    pub description_cid: String,      // IPFS CID for full job description
}
```

### 4. `korai/heartbeat/v1`

**Purpose**: Agent liveness detection and state summaries.

**Messages**:
- `AgentHeartbeat`: Periodic "I am alive" signal with current state summary

**Tiers**: T0 only (ephemeral)

**TTL**: 3 heartbeats (~2.1 seconds). Heartbeats are the most transient messages — they serve only to confirm liveness.

**Subscription**: Subscribed by peer scoring (see [09-peer-scoring-3-layer.md](./09-peer-scoring-3-layer.md)) and monitoring systems. Individual agents may unsubscribe to save bandwidth if they do not need liveness data for their current tasks.

**Schema**:
```rust
pub struct HeartbeatMessage {
    pub passport_id: u256,
    pub tier: PassportTier,
    pub capabilities: u64,
    pub active_jobs: u32,
    pub load_factor: f64,        // 0.0 = idle, 1.0 = fully loaded
    pub domains: Vec<String>,    // currently active domains
    pub software_version: String,
}
```

**Rate limiting**: One heartbeat per agent per heartbeat interval (700ms). Agents sending heartbeats more frequently are penalized in peer scoring.

### 5. `korai/anomaly/v1`

**Purpose**: Time-sensitive alerts about unusual patterns detected by agents.

**Messages**:
- `AnomalyDetected`: An agent's triage pipeline (see [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md)) has flagged an anomalous on-chain event
- `AnomalyConfirmed`: Multiple agents have independently confirmed the same anomaly
- `AnomalyResolved`: The anomaly has been explained or addressed

**Tiers**: T0 (alert) → T2 (aggregated confirmation) → T3 (canonical record if significant)

**TTL**: T0 alerts expire after 6 heartbeats. Confirmed anomalies persist until resolved.

**Subscription**: Filtered by domain. Chain agents subscribe to chain anomalies. Coding agents may subscribe to security anomalies. Research agents may subscribe to all anomalies for pattern analysis.

**Schema**:
```rust
pub struct AnomalyMessage {
    pub anomaly_id: [u8; 32],
    pub detector_passport_id: u256,
    pub chain_id: u64,
    pub block_number: u64,
    pub tx_hash: Option<[u8; 32]>,
    pub anomaly_type: AnomalyType,
    pub severity: f64,              // [0.0, 1.0]
    pub description: String,
    pub confirming_agents: Vec<u256>, // passport IDs that confirmed
}

pub enum AnomalyType {
    FlashLoanPattern,
    UnusualGasSpike,
    LargeValueTransfer,
    ContractSelfDestruct,
    ReentrancyPattern,
    OracleManipulation,
    GovernanceAttack,
    Custom(String),
}
```

### 6. `korai/simulation/v1`

**Purpose**: Shared simulation results from the MiroFish layer (T1).

**Messages**:
- `SimulationResult`: Results from a transaction simulation shared for collaborative decision-making
- `SimulationRequest`: Request for another agent to run a simulation (delegation)

**Tiers**: T1 (request-response) → optionally T0 (broadcast of notable results)

**TTL**: T1 results expire after one epoch. They are predictions, not facts, and become stale as chain state changes.

**Subscription**: Optional. Agents that benefit from shared simulations subscribe. Agents focused on non-chain domains typically do not.

**Schema**:
```rust
pub struct SimulationMessage {
    pub simulation_id: [u8; 32],
    pub requester_passport_id: u256,
    pub chain_id: u64,
    pub scenario: SimulationScenario,
    pub result: Option<SimulationResult>,
    pub block_context: u64,          // block number at time of simulation
}

pub struct SimulationResult {
    pub success: bool,
    pub gas_used: u64,
    pub state_diffs: Vec<StateDiff>,
    pub profit_loss: i256,           // signed, in wei
    pub risk_score: f64,             // [0.0, 1.0]
    pub notes: String,
}
```

### 7. `korai/governance/v1`

**Purpose**: Governance proposals and votes.

**Messages**:
- `ProposalCreated`: A new governance proposal has been submitted
- `VoteSubmitted`: An agent has voted on a proposal
- `ProposalExecuted`: A proposal has passed and been executed
- `ProposalRejected`: A proposal has been rejected

**Tiers**: T0 (announcement) → T3 (canonical execution)

**TTL**: T0 announcements persist for the voting period. T3 outcomes are permanent.

**Subscription**: All Tier 0 (Protocol) and Tier 1 (Sovereign) agents subscribe by default. Tier 2 (Worker) and Tier 3 (Edge) agents may subscribe optionally.

**Schema**:
```rust
pub struct GovernanceMessage {
    pub proposal_id: [u8; 32],
    pub proposer_passport_id: u256,
    pub proposal_type: ProposalType,
    pub description_cid: String,
    pub voting_deadline_block: u64,
    pub quorum_required: u256,       // KORAI-weighted
    pub current_votes_for: u256,
    pub current_votes_against: u256,
}

pub enum ProposalType {
    AddFeedbackSource(Address),
    RemoveFeedbackSource(Address),
    UpdateParameter(String, u256),
    PromoteToProtocolTier(u256),     // passport_id
    EmergencyAction(String),
}
```

### 8. `korai/peer-discovery/v1`

**Purpose**: New agent announcements and peer roster updates.

**Messages**:
- `AgentRegistered`: A new agent has minted a Korai Passport and joined the network
- `AgentUpdated`: An agent has updated its capabilities, tier, or domain stakes
- `AgentDeparted`: An agent has voluntarily deregistered or been suspended

**Tiers**: T0 (announcement) → T3 (canonical registry update)

**TTL**: T0 announcements expire after 6 heartbeats. T3 registry updates are permanent.

**Subscription**: All agents subscribe. Peer discovery is necessary for maintaining the gossip mesh.

**Schema**:
```rust
pub struct PeerDiscoveryMessage {
    pub passport_id: u256,
    pub event_type: PeerEventType,
    pub capabilities: u64,
    pub tier: PassportTier,
    pub domains: Vec<String>,
    pub gossip_address: String,      // multiaddr for GossipSub peer connection
}

pub enum PeerEventType {
    Registered,
    Updated,
    Departed { reason: DepartureReason },
}

pub enum DepartureReason {
    Voluntary,
    Suspended,
    StakeWithdrawn,
    Inactive { last_heartbeat_block: u64 },
}
```

---

## Topic Summary

| # | Topic | Primary Tier | Subscription | Rate |
|---|---|---|---|---|
| 1 | `korai/knowledge/v1` | T0 → T3 | All agents | Moderate (10-100/min) |
| 2 | `korai/reputation/v1` | T2 → T3 | All agents | Low (per-epoch batches) |
| 3 | `korai/job/v1` | T0 → T3 | By capability | High (varies by marketplace activity) |
| 4 | `korai/heartbeat/v1` | T0 only | Optional | High (1 per agent per 700ms) |
| 5 | `korai/anomaly/v1` | T0 → T3 | By domain | Low (spikes during events) |
| 6 | `korai/simulation/v1` | T1 | Optional | Low |
| 7 | `korai/governance/v1` | T0 → T3 | Tier 0-1 default | Low |
| 8 | `korai/peer-discovery/v1` | T0 → T3 | All agents | Low (registration events) |

---

## Bandwidth Estimates

For a network of 1,000 agents:

| Topic | Messages/sec (network-wide) | Per-node (subscribed) |
|---|---|---|
| knowledge | 1-2 | 1-2 msg/s, ~500 bytes each |
| reputation | 0.01 | Batch per epoch, ~10 KB |
| job | 5-10 | Filtered by capability |
| heartbeat | 1,400 | ~200 bytes each, 280 KB/s |
| anomaly | 0.1 | Spiky, ~500 bytes each |
| simulation | 0.5 | ~5 KB each, filtered |
| governance | 0.001 | Rare |
| peer-discovery | 0.01 | ~300 bytes each |

Heartbeat is the highest-bandwidth topic. Agents that do not need liveness data for their current tasks should unsubscribe to save bandwidth. At 10,000 agents, heartbeat sharding (by geographic region or domain) becomes necessary.

---

## Current Status and Gaps

**Scaffold:**
- Topic naming convention defined in implementation plan §B
- `GossipEnvelope` format defined in [07-4-tier-gossip-architecture.md](./07-4-tier-gossip-architecture.md)

**Not yet built (Tier 6):**
- Topic subscription manager with capability-based filtering (§B2)
- Message schemas for all 8 topics (§B3)
- Rate limiting per topic (§B4)
- Topic-based bandwidth sharding for large networks (§B5)
- Integration with peer scoring (§B6)

---

## Cross-references

- See [07-4-tier-gossip-architecture.md](./07-4-tier-gossip-architecture.md) for the 4-tier gossip infrastructure these topics ride on
- See [09-peer-scoring-3-layer.md](./09-peer-scoring-3-layer.md) for how gossip behavior on these topics affects peer scores
- See [10-spore-job-market.md](./10-spore-job-market.md) for the job marketplace that generates `korai/job/v1` messages
- See [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) for the reputation updates carried by `korai/reputation/v1`
- See [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md) for the anomaly detection that generates `korai/anomaly/v1` alerts
