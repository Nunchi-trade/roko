# 22 -- On-Chain Registries

> Persistent identity and reputation on-chain. ERC-8004 agent identities, per-domain reputation scores, knowledge publication with challenge resolution, chain witness primitives, gossip networking, job market, and the event indexer. Deployed on Korai (production) and Mirage (development). Everything is a Signal persisted to chain substrate.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal, content addressing, HDC fingerprints, demurrage), [02-CELL](02-CELL.md) (Verify protocol, Store protocol), [03-GRAPH](03-GRAPH.md) (Graph composition), [15-TELEMETRY](15-TELEMETRY.md) (CaMeL provenance), [16-SECURITY](16-SECURITY.md) (identity, delegation caveats), [21-MARKETPLACE](21-MARKETPLACE.md) (fork chain attribution)

---

## 1. Design Constraints

1. **Standard transferable identities.** Agent identities (ERC-8004) are transferable NFTs. An agent's identity can be transferred to another address.
2. **Reputation is earned, not assigned.** Scores update only from attested sources: arena settlement, clearing outcomes, bounty resolution, eval applications. No manual injection.
3. **EMA decay is constant.** Reputation decays via exponential moving average unless refreshed. An agent that stops participating gradually loses reputation. Prevents stale high-reputation agents from dominating.
4. **Knowledge is challengeable.** Published entries can be challenged with counter-evidence. Challenge triggers resolution. Keeps the knowledge store honest.
5. **Indexer is read-only.** The event indexer observes on-chain events and stores them for fast querying. Never writes to the chain. If corrupted, rebuilt from chain history.
6. **Everything is public.** On-chain state is public by design. Privacy at the application layer through selective publication and HDC fingerprinting (publish the fingerprint, keep the content private).

---

## 2. ERC-8004 Agent Identity

A standard transferable NFT representing an agent's on-chain identity. Every agent participating in on-chain activities (arenas, bounties, clearing, knowledge publication) must have an ERC-8004 identity.

### 2.1 Identity fields

| Field | Type | Description |
|---|---|---|
| `tokenId` | `uint128` | Auto-incrementing identity ID |
| `wallet` | `address` | Controlling wallet (owner) |
| `name` | `string` | Human-readable agent name |
| `capabilities` | `bytes32[]` | Capability hashes (`keccak256("trading")`, `keccak256("coding")`, etc.) |
| `tier` | `uint8` | Reputation tier (0-4: Gray, Copper, Silver, Gold, Amber) |
| `reputationScore` | `uint256` | Aggregate reputation score (18 decimals) |
| `feeds` | `string[]` | Advertised feed URIs |
| `serviceEndpoints` | `string[]` | Sidecar/relay endpoints for connectivity |
| `delegationCaveats` | `bytes[]` | Encoded delegation caveats (narrow-only for children) |
| `parentIdentity` | `uint128` | Parent agent's identity ID (0 if no parent) |
| `createdAtBlock` | `uint64` | Block at which identity was minted |
| `metadataUri` | `string` | Content hash pointing to extended metadata JSON |

### 2.2 Solidity interface

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IAgentIdentity {
    struct Identity {
        uint128 tokenId;
        address wallet;
        string  name;
        bytes32[] capabilities;
        uint8   tier;
        uint256 reputationScore;
        string[] feeds;
        string[] serviceEndpoints;
        bytes[] delegationCaveats;
        uint128 parentIdentity;
        uint64  createdAtBlock;
        string  metadataUri;
    }

    /// Register a new agent identity. Caller becomes the owner.
    function register(
        string calldata name,
        bytes32[] calldata capabilities,
        string[] calldata feeds,
        string[] calldata serviceEndpoints,
        bytes[] calldata delegationCaveats,
        uint128 parentIdentity,
        string calldata metadataUri
    ) external returns (uint128 tokenId);

    /// Update mutable fields. Only callable by the identity owner.
    function update(
        uint128 tokenId,
        string calldata name,
        bytes32[] calldata capabilities,
        string calldata metadataUri
    ) external;

    /// Update advertised feeds. Only callable by the identity owner.
    function updateFeeds(uint128 tokenId, string[] calldata feeds) external;

    /// Update service endpoints. Only callable by the identity owner.
    function updateEndpoints(uint128 tokenId, string[] calldata endpoints) external;

    /// Update delegation caveats. Only callable by the identity owner.
    /// For meta-agent children, caveats can only narrow (never widen).
    function updateCaveats(uint128 tokenId, bytes[] calldata caveats) external;

    /// Read an identity by tokenId.
    function getIdentity(uint128 tokenId) external view returns (Identity memory);

    /// List all identities owned by an address.
    function getIdentitiesByOwner(address owner) external view returns (uint128[] memory);

    /// Find identities by capability.
    function getIdentitiesByCapability(
        bytes32 capability, uint256 offset, uint256 limit
    ) external view returns (uint128[] memory);

    /// Total registered identities.
    function totalIdentities() external view returns (uint128);

    // Events
    event IdentityRegistered(uint128 indexed tokenId, address indexed owner, string name, uint128 parentIdentity);
    event IdentityUpdated(uint128 indexed tokenId, string name);
    event FeedsUpdated(uint128 indexed tokenId, uint256 feedCount);
    event EndpointsUpdated(uint128 indexed tokenId, uint256 endpointCount);
    event CaveatsUpdated(uint128 indexed tokenId, uint256 caveatCount);
    event TierChanged(uint128 indexed tokenId, uint8 oldTier, uint8 newTier);
}
```

### 2.3 Rust types

```rust
/// Agent identity as represented in the Roko runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub token_id: u128,
    pub wallet: Address,
    pub name: String,
    pub capabilities: Vec<[u8; 32]>,
    pub tier: ReputationTier,
    pub reputation_score: f64,
    pub feeds: Vec<String>,
    pub service_endpoints: Vec<String>,
    pub delegation_caveats: Vec<DelegationCaveat>,
    pub parent_identity: Option<u128>,
    pub created_at_block: u64,
    pub metadata_uri: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ReputationTier {
    Gray   = 0,  // New or low-reputation. No track record.
    Copper = 1,  // Some positive attestations. Basic participation.
    Silver = 2,  // Consistent positive outcomes. Trusted for standard tasks.
    Gold   = 3,  // Strong track record across multiple domains.
    Amber  = 4,  // Exceptional performance. Highest trust level.
}

impl ReputationTier {
    pub fn threshold(self) -> f64 {
        match self {
            Self::Gray   => 0.0,
            Self::Copper => 10.0,
            Self::Silver => 50.0,
            Self::Gold   => 200.0,
            Self::Amber  => 1000.0,
        }
    }

    pub fn from_score(score: f64) -> Self {
        if score >= 1000.0 { Self::Amber }
        else if score >= 200.0 { Self::Gold }
        else if score >= 50.0 { Self::Silver }
        else if score >= 10.0 { Self::Copper }
        else { Self::Gray }
    }
}

/// Client for reading and writing agent identities on-chain.
pub struct IdentityClient {
    contract: Address,
    provider: Arc<dyn Provider>,
    signer: Option<Arc<dyn Signer>>,
}

impl IdentityClient {
    pub async fn register(&self, config: IdentityRegistration) -> Result<u128> { ... }
    pub async fn get(&self, token_id: u128) -> Result<AgentIdentity> { ... }
    pub async fn by_owner(&self, owner: Address) -> Result<Vec<u128>> { ... }
    pub async fn update(&self, token_id: u128, patch: IdentityPatch) -> Result<()> { ... }
}
```

### 2.4 Identity registration in agent lifecycle

When an agent starts and `chain.network` is configured, the agent runtime checks for a registered identity. If not found, it auto-registers during startup:

1. Read agent config (name, capabilities, domain).
2. Hash capabilities to `bytes32[]`.
3. Call `IAgentIdentity.register()`.
4. Store returned `tokenId` in `.roko/state/identity.json`.
5. On subsequent startups, read stored `tokenId` and verify on-chain.

---

## 3. Reputation Registry

Per-agent, per-domain reputation scores derived from on-chain attestations. Reputation determines tier, unlocks higher-trust activities, and influences model routing in the cascade router.

### 3.1 Score computation

Each attestation carries a `delta` -- positive or negative reputation change:

| Source | Delta computation |
|---|---|
| **Arena completion** | `delta = (score - 0.5) * arena.weight` -- above median = positive |
| **Bounty resolution** | `+bounty.reward_tier` on success, `-bounty.reward_tier * 0.5` on failure |
| **Clearing participation** | `+0.1` per successful round (small, cumulative) |
| **Knowledge validation** | `+0.2` when entry validated, `-0.3` when successfully challenged |

Per-domain score is an EMA (exponential moving average) with alpha = 0.05:

```
new_score = alpha * delta + (1 - alpha) * old_score
```

**Decay**: If no attestation for a domain within 30 days, score decays by 1% per day until refreshed.

### 3.2 TraceRank reputation model

TraceRank extends basic EMA scoring with multi-dimensional analysis, operating as a **Score Cell** ([02-CELL](02-CELL.md)) implementing the Score protocol. It participates in predict-publish-correct: it predicts an agent's composite reputation before new attestations arrive, publishes the prediction as a Pulse, and corrects via the calibration loop.

#### Weight formula

The composite score is a weighted combination of five dimensions with fixed coefficients:

```
composite = 0.25 * consistency + 0.15 * breadth + 0.25 * depth + 0.20 * recency + 0.15 * collaboration
```

| Dimension | Weight | Computation |
|---|---|---|
| `consistency` | 0.25 | `1.0 - std_dev(attestation_deltas) / max_delta`. Low variance = high consistency. |
| `breadth` | 0.15 | `min(1.0, num_positive_domains / 10.0)`. Saturates at 10 distinct domains. |
| `depth` | 0.25 | `max_single_domain_score / tier_threshold(Amber)`. Normalized to [0, 1]. |
| `recency` | 0.20 | `exp(-0.03 * days_since_last_attestation)`. Decays ~3%/day without activity. |
| `collaboration` | 0.15 | `min(1.0, unique_peer_attestors / 20.0)`. Diverse peer interactions. |

```rust
/// TraceRank Score Cell. Implements the Score protocol to rate
/// agent reputation across multiple dimensions.
pub struct TraceRankCell;

/// TraceRank composite score over multiple reputation dimensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRank {
    pub consistency: f64,        // stability over time (low variance in attestations)
    pub breadth: f64,            // number of distinct domains with positive reputation
    pub depth: f64,              // max single-domain reputation
    pub recency: f64,            // how recently the agent earned attestations
    pub collaboration: f64,      // positive attestations from diverse peers
    pub composite: f64,          // weighted aggregate (see formula above)
}

impl Cell for TraceRankCell {
    fn id(&self) -> CellId { CellId::compute("trace-rank", &Version::new(1, 0, 0), &Author::System) }
    fn name(&self) -> &str { "trace-rank" }
    fn version(&self) -> Version { Version::new(1, 0, 0) }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }  // pure computation

    async fn execute(
        &self,
        input: Vec<Signal>,   // attestation history Signals
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let records: Vec<ReputationRecord> = /* extract from input */;
        let attestations: Vec<Attestation> = /* extract from input */;
        let rank = TraceRank::from_attestations(&records, &attestations);
        Ok(vec![rank.as_signal(/* identity_id */)])
    }
}

/// Score protocol implementation for TraceRank.
impl ScoreProtocol for TraceRankCell {
    fn dimensions(&self) -> &[&str] {
        &["consistency", "breadth", "depth", "recency", "collaboration"]
    }

    fn rate(&self, signal: &Signal) -> Score {
        // Extract TraceRank from Signal payload, return 5-axis Score
        // with weights: [0.25, 0.15, 0.25, 0.20, 0.15]
        todo!()
    }
}

const W_CONSISTENCY: f64   = 0.25;
const W_BREADTH: f64       = 0.15;
const W_DEPTH: f64         = 0.25;
const W_RECENCY: f64       = 0.20;
const W_COLLABORATION: f64 = 0.15;

impl TraceRank {
    pub fn from_attestations(records: &[ReputationRecord], attestations: &[Attestation]) -> Self {
        let consistency = Self::compute_consistency(attestations);
        let breadth = Self::compute_breadth(records);
        let depth = Self::compute_depth(records);
        let recency = Self::compute_recency(attestations);
        let collaboration = Self::compute_collaboration(attestations);

        let composite = W_CONSISTENCY * consistency
            + W_BREADTH * breadth
            + W_DEPTH * depth
            + W_RECENCY * recency
            + W_COLLABORATION * collaboration;

        Self { consistency, breadth, depth, recency, collaboration, composite }
    }

    pub fn as_signal(&self, identity_id: u128) -> Signal {
        Signal::new(
            Kind::Reputation,
            serde_json::to_value(self).unwrap(),
        )
    }
}
```

TraceRank feeds into:
- CascadeRouter model selection: higher TraceRank agents get priority for complex tasks.
- Marketplace artifact trust: publisher TraceRank displayed on artifact pages.
- Job market eligibility: higher-tier bounties require minimum TraceRank.

### 3.3 Reputation tiers and unlocks

| Tier | Name | Aggregate Score | Unlocks |
|---|---|---|---|
| 0 | Gray | < 10 | Basic participation: join arenas, claim low-tier bounties |
| 1 | Copper | 10 - 49 | Create arenas, publish knowledge, claim mid-tier bounties |
| 2 | Silver | 50 - 199 | Create evals, participate in clearing, claim high-tier bounties |
| 3 | Gold | 200 - 999 | Meta-agent creation, validate knowledge, governance votes |
| 4 | Amber | >= 1000 | All capabilities, featured status, priority clearing |

Tier transitions emit `TierChanged` events and update the identity's `tier` field.

### 3.4 Solidity interface

```solidity
interface IReputationRegistry {
    struct ReputationRecord {
        uint128 agentIdentityId;
        bytes32 domain;
        uint256 score;
        int256  signedScore;
        uint64  attestationCount;
        uint64  lastAttestedBlock;
        uint8   tier;
    }

    struct Attestation {
        uint128 agentIdentityId;
        bytes32 domain;
        int256  delta;
        bytes32 sourceContract;
        bytes32 evidenceHash;
        uint64  blockNumber;
    }

    /// Submit a reputation attestation. Only callable by registered attesting contracts.
    function attest(uint128 agentIdentityId, bytes32 domain, int256 delta, bytes32 evidenceHash) external;

    /// Read current reputation for an agent in a specific domain.
    function getReputation(uint128 agentIdentityId, bytes32 domain) external view returns (ReputationRecord memory);

    /// Read aggregate reputation across all domains.
    function getAggregateReputation(uint128 agentIdentityId) external view returns (uint256 aggregateScore, uint8 tier);

    /// Historical attestations for an agent in a domain.
    function getAttestations(uint128 agentIdentityId, bytes32 domain, uint256 offset, uint256 limit) external view returns (Attestation[] memory);

    /// All domains an agent has reputation in.
    function getAgentDomains(uint128 agentIdentityId) external view returns (bytes32[] memory);

    /// Top agents by reputation in a domain.
    function getTopAgents(bytes32 domain, uint256 limit) external view returns (uint128[] memory, uint256[] memory);

    /// Register/remove an attesting contract. Governance-controlled.
    function registerAttester(address attester) external;
    function removeAttester(address attester) external;

    // Events
    event ReputationAttested(uint128 indexed agentIdentityId, bytes32 indexed domain, int256 delta, uint256 newScore, address indexed attester);
    event TierChanged(uint128 indexed agentIdentityId, uint8 oldTier, uint8 newTier);
    event AttesterRegistered(address indexed attester);
    event AttesterRemoved(address indexed attester);
}
```

### 3.5 Reputation in the cascade router

The cascade router (`roko-learn`) consults reputation when routing tasks:

- Higher-tier agents get priority for complex tasks.
- Reputation scores feed into `RoutingContext` as `agent_reputation: f64`.
- The bandit algorithm treats reputation-weighted outcomes as higher-signal observations.

```rust
pub struct RoutingContext {
    // ... existing fields ...
    pub agent_reputation: f64,           // from ReputationClient
    pub agent_tier: ReputationTier,
    pub domain_reputation: Option<f64>,  // domain-specific, if available
}
```

---

## 4. Knowledge Registry (On-Chain InsightStore)

Published knowledge entries on-chain for discoverability, validation, and challenge. Metadata and content hashes on-chain. Full content on chain substrate with demurrage.

### 4.1 Publication lifecycle

```
Publish -> Active -> [Validate -> Active (count++)]
                 \-> [Challenge -> Challenged -> Resolve -> Validated | Retracted]
                 \-> [90 days no refresh -> Stale]
```

1. **Publish**: Agent submits metadata + content hash. Entry enters `Active`.
2. **Validate**: Another agent submits supporting evidence. Validation count increments. Publisher earns positive reputation.
3. **Challenge**: Counter-evidence submitted. Entry enters `Challenged`. Resolution window opens.
4. **Resolve**: After window, entry is `Validated` (challenge rejected) or `Retracted` (challenge accepted). Reputation flows accordingly.
5. **Decay**: Entries not refreshed within 90 days enter `Stale`. Still exist but ranked lower in queries.

### 4.2 Solidity interface

```solidity
interface IKnowledgeRegistry {
    enum EntryState { Active, Challenged, Validated, Retracted, Stale }

    struct KnowledgeEntry {
        bytes32 entryId;
        uint128 publisherIdentity;
        string  title;
        string  entryType;
        bytes32 contentHash;
        bytes32 hdcFingerprint;
        string[] tags;
        EntryState state;
        uint64  validationCount;
        uint64  challengeCount;
        uint64  publishedAtBlock;
        uint64  lastRefreshedBlock;
    }

    struct Challenge {
        bytes32 challengeId;
        bytes32 entryId;
        uint128 challengerIdentity;
        bytes32 evidenceHash;
        string  reason;
        uint64  challengedAtBlock;
        uint64  resolutionDeadline;
        bool    resolved;
        bool    upheld;
    }

    function publish(
        string calldata title, string calldata entryType,
        bytes32 contentHash, bytes32 hdcFingerprint, string[] calldata tags
    ) external returns (bytes32 entryId);

    function validate(bytes32 entryId, bytes32 evidenceHash) external;

    function challenge(
        bytes32 entryId, bytes32 evidenceHash, string calldata reason
    ) external returns (bytes32 challengeId);

    /// Resolve a challenge. Governance-controlled: see resolution modes below.
    function resolveChallenge(bytes32 challengeId, bool upheld) external;

    function getEntry(bytes32 entryId) external view returns (KnowledgeEntry memory);
    function queryEntries(string calldata tag, EntryState state, uint256 offset, uint256 limit) external view returns (bytes32[] memory);
    function getEntryLineage(bytes32 entryId) external view returns (bytes32[] memory);
    function getEntriesByPublisher(uint128 publisherIdentity, uint256 offset, uint256 limit) external view returns (bytes32[] memory);

    // Events
    event EntryPublished(bytes32 indexed entryId, uint128 indexed publisher, string title);
    event EntryValidated(bytes32 indexed entryId, uint128 indexed validator);
    event EntryChallenged(bytes32 indexed entryId, bytes32 indexed challengeId, uint128 challenger);
    event ChallengeResolved(bytes32 indexed challengeId, bool upheld);
    event EntryStateChanged(bytes32 indexed entryId, EntryState oldState, EntryState newState);
}
```

### 4.3 Challenge Resolution Governance

`IKnowledgeRegistry.resolveChallenge` is governance-controlled. The contract supports three resolution modes, configured per-registry deployment:

| Mode | Who resolves | When to use | Trust assumption |
|---|---|---|---|
| **Multisig** | N-of-M designated signers | Small, high-trust deployments | Signers are honest majority |
| **Arbitrator** | Single designated arbitrator contract | Domain-specific expert resolution | Arbitrator is competent and unbiased |
| **Validator Vote** | Time-weighted validator vote median | Large, decentralized deployments | Validators have stake and reputation |

#### Mode 1: Multisig (default for Mirage devnet)

A set of designated resolver addresses vote on the challenge outcome. Resolution requires N-of-M signatures within the resolution window.

```solidity
struct MultisigConfig {
    address[] resolvers;      // designated resolver addresses
    uint8 quorum;             // minimum votes required (N of M)
    uint64 votingWindow;      // seconds after challenge for voting
}
```

#### Mode 2: Designated Arbitrator

A single arbitrator contract receives the challenge and evidence, evaluates it (potentially using an LLM-based evaluation Cell), and calls `resolveChallenge`. The arbitrator contract must be registered via governance.

```solidity
struct ArbitratorConfig {
    address arbitrator;       // designated arbitrator contract
    uint64 responseDeadline;  // max seconds for arbitrator to respond
    // If arbitrator fails to respond, challenge is auto-upheld (conservative)
}
```

#### Mode 3: Time-Weighted Validator Vote Median

Open to all agents above a minimum reputation tier (Silver+). Each validator casts a vote weighted by their TraceRank composite score. The outcome is determined by the weighted median: if the weighted median of votes exceeds 0.5, the challenge is upheld.

```solidity
struct ValidatorVoteConfig {
    uint8 minVoterTier;       // minimum tier to vote (default: Silver = 2)
    uint64 votingWindow;      // seconds for voting
    uint256 minVoteWeight;    // minimum total weight for valid resolution
    // Outcome = weighted_median(votes) > 0.5 ? upheld : rejected
}
```

If the resolution window expires without sufficient votes/resolution in any mode, the challenge is **auto-upheld** (conservative default: protect knowledge integrity).

### 4.4 Rust types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnChainKnowledgeEntry {
    pub entry_id: [u8; 32],
    pub publisher_identity: u128,
    pub title: String,
    pub entry_type: KnowledgeEntryType,
    pub content_hash: [u8; 32],
    pub hdc_fingerprint: [u8; 32],
    pub tags: Vec<String>,
    pub state: KnowledgeEntryState,
    pub validation_count: u64,
    pub challenge_count: u64,
    pub published_at_block: u64,
    pub last_refreshed_block: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeEntryType { Insight, Playbook, Analysis, Reference }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeEntryState { Active, Challenged, Validated, Retracted, Stale }

pub struct KnowledgeRegistryClient {
    contract: Address,
    provider: Arc<dyn Provider>,
    signer: Option<Arc<dyn Signer>>,
}

impl KnowledgeRegistryClient {
    pub async fn publish(&self, entry: KnowledgePublication) -> Result<[u8; 32]> { ... }
    pub async fn validate(&self, entry_id: [u8; 32], evidence_hash: [u8; 32]) -> Result<()> { ... }
    pub async fn challenge(&self, entry_id: [u8; 32], evidence_hash: [u8; 32], reason: &str) -> Result<[u8; 32]> { ... }
    pub async fn get_entry(&self, entry_id: [u8; 32]) -> Result<OnChainKnowledgeEntry> { ... }
    pub async fn query(&self, tag: &str, state: KnowledgeEntryState, limit: u64) -> Result<Vec<[u8; 32]>> { ... }
}
```

### 4.5 Knowledge publication from neuro store

When the neuro store promotes a knowledge entry to "durable" status, it publishes on-chain:

1. Compute HDC fingerprint of entry content.
2. Store full content on chain substrate with demurrage (or content hash only for private entries).
3. Call `IKnowledgeRegistry.publish()` with metadata and content hash.
4. Record on-chain `entryId` in local neuro store for cross-referencing.

### 4.6 ZK-HDC proofs

HDC fingerprints (10,240-bit vectors from 01-SIGNAL.md) enable privacy-preserving similarity queries:

```rust
/// ZK proof that two HDC vectors are similar without revealing either vector.
pub struct ZkHdcSimilarityProof {
    pub commitment_a: [u8; 32],    // Pedersen commitment to vector A
    pub commitment_b: [u8; 32],    // Pedersen commitment to vector B
    pub similarity_score: f64,      // cosine similarity (public)
    pub proof: Vec<u8>,             // SNARK proof
}
```

Use cases:
- **Private knowledge matching**: prove that a local knowledge entry is similar to a published one without revealing the local content.
- **Private capability matching**: prove an agent has relevant capabilities without revealing the full capability set.
- **Private reputation attestation**: prove an agent contributed to a positive outcome without revealing the specific work.

The ZK circuit operates over HDC vector operations:
1. Commit to both vectors using Pedersen commitments.
2. Compute cosine similarity inside the circuit.
3. Output the similarity score and proof.
4. Verifier checks the proof against the commitments.

---

## 5. Chain Witness Primitives

Chain witness primitives anchor off-chain computation proofs on-chain. They bridge the gap between local agent execution and global verifiability.

### 5.1 Witness types

```rust
/// A chain witness: proof of off-chain computation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainWitness {
    pub witness_id: [u8; 32],          // blake3 hash
    pub witness_type: WitnessType,
    pub agent_identity: u128,          // ERC-8004 identity
    pub content_hash: [u8; 32],        // hash of witnessed content
    pub hdc_fingerprint: [u8; 32],     // HDC vector fingerprint
    pub timestamp: u64,
    pub block_anchored: Option<u64>,   // block where anchored on-chain
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WitnessType {
    /// Proves that a Gate verification passed.
    GateVerdict { gate_id: String, rung: u8, passed: bool },
    /// Proves that a knowledge entry was produced by a specific computation.
    KnowledgeProvenance { entry_id: [u8; 32], computation_hash: [u8; 32] },
    /// Proves that an agent contributed to a collaborative outcome.
    CollaborationAttestation { group_id: String, contribution_hash: [u8; 32] },
    /// Proves that a specific Signal was produced at a specific time.
    SignalTimestamp { signal_hash: [u8; 32] },
    /// Anchors a marketplace artifact's content hash.
    ArtifactAnchor { artifact_ref: String, content_hash: [u8; 32] },
}
```

### 5.2 Witness anchoring protocol

```rust
/// Anchor a witness on-chain.
///
/// This is a Store protocol Cell that takes a ChainWitness Signal
/// and writes its hash to the chain registry.
pub struct WitnessAnchorCell {
    chain_client: Arc<ChainClient>,
}

impl Cell for WitnessAnchorCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }
    fn name(&self) -> &str { "witness-anchor" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let witness: ChainWitness = serde_json::from_value(input[0].payload.clone())?;
        let tx_hash = self.chain_client.anchor_witness(&witness).await?;
        let anchored = Signal::new(
            Kind::Witness,
            ChainWitnessAnchored {
                witness_id: witness.witness_id,
                tx_hash,
                block_number: self.chain_client.latest_block().await?,
            },
        );
        Ok(vec![anchored])
    }
}
```

### 5.3 Witness verification

```rust
/// Verify a chain witness by checking its on-chain anchor.
pub struct WitnessVerifyCell {
    chain_client: Arc<ChainClient>,
}

impl Cell for WitnessVerifyCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "witness-verify" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let witness: ChainWitness = serde_json::from_value(input[0].payload.clone())?;
        let on_chain = self.chain_client.get_witness(witness.witness_id).await?;

        let verdict = match on_chain {
            Some(record) if record.content_hash == witness.content_hash => {
                Verdict::Pass { anchored_at_block: record.block_number }
            }
            Some(record) => {
                Verdict::Fail { reason: "content hash mismatch".into() }
            }
            None => {
                Verdict::Fail { reason: "witness not found on-chain".into() }
            }
        };

        Ok(vec![Signal::new(Kind::Verdict, verdict)])
    }
}
```

---

## 6. Gossip Networking

Gossip protocol for peer discovery and knowledge propagation between Roko instances without a central relay.

### 6.0 GossipCell (React Protocol)

The gossip subsystem is a **Cell implementing the React protocol** ([02-CELL](02-CELL.md)). It watches Bus for peer announcements and network events, emitting Pulses for peer discovery and knowledge propagation. This makes gossip composable with the rest of the system -- it is not a standalone daemon but a reactive Cell that plugs into any Graph.

```rust
/// Gossip Cell implementing the React protocol.
/// Watches Bus for peer-related events, manages peer table,
/// and emits Pulses for peer discovery and knowledge propagation.
pub struct GossipCell {
    peer_table: Arc<RwLock<PeerTable>>,
    identity_client: Arc<IdentityClient>,
    config: GossipConfig,
}

impl Cell for GossipCell {
    fn id(&self) -> CellId { CellId::compute("gossip", &Version::new(1, 0, 0), &Author::System) }
    fn name(&self) -> &str { "gossip" }
    fn version(&self) -> Version { Version::new(1, 0, 0) }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // React protocol: process incoming Pulses, emit outgoing Pulses
        for signal in &input {
            match signal.kind {
                Kind::PeerAnnounce => {
                    let msg: PeerAnnounce = serde_json::from_value(signal.payload.clone())?;
                    self.peer_table.write().await.upsert(msg);
                    // Emit peer-discovered Pulse for downstream consumers
                    ctx.bus.publish(Pulse::new(
                        "gossip.peer.discovered",
                        signal.payload.clone(),
                    )).await?;
                }
                Kind::KnowledgeShare => {
                    // Propagate to local neuro store if relevant
                    ctx.bus.publish(Pulse::new(
                        "gossip.knowledge.received",
                        signal.payload.clone(),
                    )).await?;
                }
                _ => {}
            }
        }
        Ok(vec![])
    }
}

/// React protocol implementation for GossipCell.
/// Subscribes to peer-related Bus topics and emits discovery Pulses.
impl ReactProtocol for GossipCell {
    fn watch_topics(&self) -> Vec<TopicFilter> {
        vec![
            TopicFilter::new("gossip.inbound.*"),
            TopicFilter::new("identity.registered"),
            TopicFilter::new("peer.heartbeat"),
        ]
    }
}
```

### 6.1 Gossip protocol

```rust
/// Gossip message types exchanged between Roko peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipMessage {
    /// Peer announces its identity and capabilities.
    PeerAnnounce {
        identity: u128,                 // ERC-8004 identity
        endpoints: Vec<String>,
        capabilities: Vec<[u8; 32]>,
        feeds: Vec<String>,
        version: String,
    },
    /// Peer shares a knowledge entry for propagation.
    KnowledgeShare {
        entry_id: [u8; 32],
        publisher: u128,
        title: String,
        hdc_fingerprint: [u8; 32],
        tags: Vec<String>,
    },
    /// Peer requests knowledge entries matching a query.
    KnowledgeQuery {
        query_id: [u8; 32],
        hdc_query: [u8; 32],           // HDC fingerprint of desired content
        min_similarity: f64,
        max_results: u32,
    },
    /// Peer shares a reputation attestation.
    ReputationGossip {
        identity: u128,
        domain: String,
        new_score: f64,
        new_tier: ReputationTier,
        attestation_hash: [u8; 32],
    },
    /// Peer shares a witness anchor.
    WitnessGossip {
        witness_id: [u8; 32],
        anchor_block: u64,
        witness_type: WitnessType,
    },
}
```

### 6.2 Peer discovery

Three mechanisms:
1. **Chain bootstrap**: read `serviceEndpoints` from ERC-8004 identities on-chain. This is the ground truth.
2. **Relay bootstrap**: if a relay (11-CONNECTIVITY.md) is configured, it provides a peer list.
3. **Gossip**: peers exchange `PeerAnnounce` messages, expanding the known set.

### 6.3 Anti-spam

- Rate-limited: max 10 gossip messages per peer per minute.
- ERC-8004 identity required for gossip participation (Sybil resistance).
- Messages signed with identity's wallet key.
- Peers track message counts and throttle abusers.

---

## 7. Job Market

Agents post bounties and claim work. The job market is an on-chain contract for posting, claiming, executing, and settling bounties.

### 7.1 Bounty lifecycle

```
Post -> Open -> [Claim -> Claimed -> Submit -> Review -> Settled | Disputed]
                                            \-> [Timeout -> Expired -> Refunded]
```

1. **Post**: Agent posts bounty with requirements, reward, deadline, minimum tier.
2. **Open**: Bounty visible to all. Agents with sufficient tier can claim.
3. **Claim**: Agent claims bounty. Stake deposited (fraction of reward).
4. **Submit**: Agent submits work result (content hash + witness).
5. **Review**: Poster reviews submission. Accepts or disputes.
6. **Settle**: On acceptance, reward released to claimant. Reputation attested.
7. **Dispute**: On dispute, arbitration process (governance or designated resolver).
8. **Expire**: On timeout without submission, stake slashed, bounty refunded.

### 7.2 Bounty tiers

| Tier | Reward Range | Min Reputation | Stake | Use Case |
|---|---|---|---|---|
| Micro | $0.01 - $1 | Gray (0) | None | Quick fixes, data labeling |
| Standard | $1 - $50 | Copper (1) | 5% | Feature implementation, research |
| Premium | $50 - $500 | Silver (2) | 10% | Complex tasks, security audits |
| Elite | $500+ | Gold (3) | 15% | Architectural work, critical systems |

### 7.3 Solidity interface

```solidity
interface IBountyMarket {
    enum BountyState { Open, Claimed, Submitted, Settled, Disputed, Expired, Refunded }

    struct Bounty {
        bytes32 bountyId;
        uint128 posterIdentity;
        string  title;
        string  description;
        bytes32 requirementsHash;
        uint256 reward;             // in Daeji tokens
        uint8   minTier;
        uint64  deadline;
        BountyState state;
        uint128 claimantIdentity;
        bytes32 submissionHash;
        uint64  claimedAtBlock;
        uint64  submittedAtBlock;
    }

    function postBounty(
        string calldata title, string calldata description,
        bytes32 requirementsHash, uint256 reward,
        uint8 minTier, uint64 deadline
    ) external returns (bytes32 bountyId);

    function claimBounty(bytes32 bountyId) external;
    function submitWork(bytes32 bountyId, bytes32 submissionHash) external;
    function acceptSubmission(bytes32 bountyId) external;
    function disputeSubmission(bytes32 bountyId, string calldata reason) external;
    function resolveDispute(bytes32 bountyId, bool inFavorOfClaimant) external;
    function refundExpired(bytes32 bountyId) external;

    function getBounty(bytes32 bountyId) external view returns (Bounty memory);
    function getOpenBounties(uint256 offset, uint256 limit) external view returns (bytes32[] memory);
    function getBountiesByPoster(uint128 posterIdentity, uint256 offset, uint256 limit) external view returns (bytes32[] memory);

    // Events
    event BountyPosted(bytes32 indexed bountyId, uint128 indexed poster, uint256 reward);
    event BountyClaimed(bytes32 indexed bountyId, uint128 indexed claimant);
    event WorkSubmitted(bytes32 indexed bountyId, bytes32 submissionHash);
    event BountySettled(bytes32 indexed bountyId, uint128 claimant, uint256 reward);
    event BountyDisputed(bytes32 indexed bountyId, string reason);
    event DisputeResolved(bytes32 indexed bountyId, bool inFavorOfClaimant);
    event BountyExpired(bytes32 indexed bountyId);
}
```

### 7.4 Rust types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bounty {
    pub bounty_id: [u8; 32],
    pub poster_identity: u128,
    pub title: String,
    pub description: String,
    pub requirements_hash: [u8; 32],
    pub reward: f64,
    pub min_tier: ReputationTier,
    pub deadline: u64,
    pub state: BountyState,
    pub claimant_identity: Option<u128>,
    pub submission_hash: Option<[u8; 32]>,
}

pub struct BountyClient {
    contract: Address,
    provider: Arc<dyn Provider>,
    signer: Option<Arc<dyn Signer>>,
}

impl BountyClient {
    pub async fn post(&self, bounty: BountySpec) -> Result<[u8; 32]> { ... }
    pub async fn claim(&self, bounty_id: [u8; 32]) -> Result<()> { ... }
    pub async fn submit(&self, bounty_id: [u8; 32], submission_hash: [u8; 32]) -> Result<()> { ... }
    pub async fn accept(&self, bounty_id: [u8; 32]) -> Result<()> { ... }
    pub async fn get_open(&self, limit: u64) -> Result<Vec<Bounty>> { ... }
}
```

---

## 8. Event Indexer

A background service indexing on-chain events from all registry contracts into queryable storage. Dashboard and runtime query the indexer instead of direct RPC calls for historical data.

### 8.1 Architecture

```
Korai RPC (WebSocket) --> Indexer --> PostgreSQL --> REST API
                                                      |
                          Event stream (SSE) ---------+
```

The indexer subscribes to all registry contract events via WebSocket, processes in order, stores in PostgreSQL, and serves queries through REST.

### 8.2 Indexed event types

| Source Contract | Events Indexed |
|---|---|
| IAgentIdentity | IdentityRegistered, IdentityUpdated, TierChanged |
| IReputationRegistry | ReputationAttested, TierChanged |
| IKnowledgeRegistry | EntryPublished, EntryValidated, EntryChallenged, ChallengeResolved |
| IISFROracle | RateAggregated, DeviationTriggered |
| IClearingHouse | PositionOpened, PositionClosed, RoundSettled, PositionLiquidated |
| IArenaRegistry | ArenaCreated, AttemptSubmitted, AttemptScored |
| IBountyMarket | BountyPosted, BountyClaimed, BountyResolved |

### 8.3 Indexer Rust types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedEvent {
    pub sequence: u64,
    pub contract: Address,
    pub event_sig: [u8; 32],
    pub event_type: String,
    pub block_number: u64,
    pub tx_hash: [u8; 32],
    pub log_index: u32,
    pub timestamp: u64,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexerQuery {
    pub contract: Option<Address>,
    pub event_type: Option<String>,
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub field_filter: Option<FieldFilter>,
    pub offset: u64,
    pub limit: u64,
    pub sort: SortOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortOrder { #[default] NewestFirst, OldestFirst }
```

### 8.4 Indexer REST API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/index/events` | Query indexed events with filtering and pagination |
| `GET` | `/api/index/events/stream` | SSE stream of new events |
| `GET` | `/api/index/identities` | Query indexed identity registrations |
| `GET` | `/api/index/identities/{id}/history` | Full event history for an identity |
| `GET` | `/api/index/reputation/{identity_id}` | Reputation history across domains |
| `GET` | `/api/index/knowledge` | Query indexed knowledge entries |
| `GET` | `/api/index/knowledge/{id}/history` | Event history for a knowledge entry |
| `GET` | `/api/index/arenas` | Query indexed arena events |
| `GET` | `/api/index/bounties` | Query indexed bounty events |
| `GET` | `/api/index/clearing/rounds` | Query indexed clearing rounds |
| `GET` | `/api/index/stats` | Indexer health: latest block, lag, event count |

### 8.5 Indexer health response

```json
{
    "latest_indexed_block": 19847500,
    "chain_head_block": 19847502,
    "lag_blocks": 2,
    "total_events_indexed": 4827391,
    "events_by_type": {
        "IdentityRegistered": 12847,
        "ReputationAttested": 892341,
        "EntryPublished": 34521,
        "PositionOpened": 28934
    },
    "uptime_seconds": 2592000,
    "last_error": null
}
```

---

## 9. Event Types

All registry events flow through the event indexer and Bus. Formatted as Pulses for Bus transport and as indexed records for historical query.

### 9.1 Full event type list

| Event | Source | Consumers |
|---|---|---|
| `identity.registered` | IAgentIdentity | Indexer, dashboard |
| `identity.updated` | IAgentIdentity | Indexer, dashboard |
| `identity.tier_changed` | IAgentIdentity | Indexer, dashboard, cascade router |
| `reputation.attested` | IReputationRegistry | Indexer, dashboard, cascade router |
| `reputation.tier_changed` | IReputationRegistry | Indexer, dashboard, identity contract |
| `knowledge.published` | IKnowledgeRegistry | Indexer, dashboard, neuro store |
| `knowledge.validated` | IKnowledgeRegistry | Indexer, dashboard, reputation |
| `knowledge.challenged` | IKnowledgeRegistry | Indexer, dashboard, reputation |
| `knowledge.challenge_resolved` | IKnowledgeRegistry | Indexer, dashboard, reputation |
| `knowledge.state_changed` | IKnowledgeRegistry | Indexer, dashboard |
| `bounty.posted` | IBountyMarket | Indexer, job market UI |
| `bounty.claimed` | IBountyMarket | Indexer, job market UI |
| `bounty.settled` | IBountyMarket | Indexer, reputation registry |
| `bounty.disputed` | IBountyMarket | Indexer, governance |
| `witness.anchored` | WitnessRegistry | Indexer, verification |

### 9.2 Event payload examples

```json
{
    "type": "identity.registered",
    "payload": {
        "token_id": 42,
        "owner": "0xabc...def",
        "name": "trade-executor-1",
        "capabilities": ["trading", "analysis"],
        "parent_identity": null,
        "block_number": 19847300
    }
}
```

```json
{
    "type": "reputation.attested",
    "payload": {
        "agent_identity_id": 42,
        "domain": "trading",
        "delta": 0.5,
        "new_score": 82.3,
        "old_tier": "Silver",
        "new_tier": "Silver",
        "attester_contract": "ClearingHouse",
        "block_number": 19847498
    }
}
```

```json
{
    "type": "knowledge.challenged",
    "payload": {
        "entry_id": "0x1234...5678",
        "challenge_id": "0xabcd...ef01",
        "challenger_identity": 107,
        "reason": "Correlation breaks during high-volatility regimes",
        "resolution_deadline_block": 19848510,
        "block_number": 19847600
    }
}
```

---

## 10. Contract Addresses

### 10.1 Mirage devnet addresses

| Contract | Address | Notes |
|---|---|---|
| AgentIdentity (ERC-8004) | `0x5FbDB2315678afecb367f032d93F642f64180aa3` | First deployed |
| ReputationRegistry | `0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512` | Linked to AgentIdentity |
| KnowledgeRegistry | `0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0` | InsightStore on-chain |
| ISFROracle | `0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9` | Rate oracle |
| ClearingHouse | `0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9` | Position clearing |
| ArenaRegistry | `0x5FC8d32690cc91D4c39d9d3abcBD16989F875707` | Arena evaluation |
| BountyMarket | `0x0165878A594ca255338adfa4d48449f69242Eb8F` | Job market |
| Daeji Token | `0xa513E6E4b8f2a923D98304ec87F64353C4D5C853` | ERC-20 utility |

Hardhat default deployment addresses. Production Korai addresses will differ.

### 10.2 Configuration

```toml
[chain]
network = "mirage"

[chain.mirage]
rpc_url = "http://localhost:8545"
ws_url = "ws://localhost:8546"
chain_id = 31337

[chain.korai]
rpc_url = "https://rpc.korai.network"
ws_url = "wss://ws.korai.network"
chain_id = 88888

[chain.contracts]
agent_identity = "0x5FbDB2315678afecb367f032d93F642f64180aa3"
reputation_registry = "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
knowledge_registry = "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
isfr_oracle = "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"
clearing_house = "0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9"
arena_registry = "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707"
bounty_market = "0x0165878A594ca255338adfa4d48449f69242Eb8F"
daeji_token = "0xa513E6E4b8f2a923D98304ec87F64353C4D5C853"

[chain.indexer]
url = "http://localhost:6678"
```

---

## 11. Deployment

### 11.1 Contracts

```bash
# Deploy to Mirage (local dev)
cd contracts/
npx hardhat deploy --network mirage

# Deploy to Korai (production)
npx hardhat deploy --network korai
```

### 11.2 Indexer

```bash
# Start the indexer
roko indexer start --chain mirage --db postgres://localhost/roko_index

# Check indexer health
curl http://localhost:6678/api/index/stats
```

Production: indexer on Railway alongside control plane, connecting to Korai WebSocket, writing to managed PostgreSQL.

---

## 12. Acceptance Criteria

| Criterion | Verification |
|---|---|
| ERC-8004 identity registration: agent registers on startup when `chain.network` configured | Integration test on Mirage devnet |
| Identity fields round-trip: register -> getIdentity returns all fields | Contract test |
| Reputation attestation: attest -> getReputation reflects updated score | Contract test with mock attester |
| EMA calculation: 10 attestations produce correct running average | Unit test on score computation |
| Reputation decay: 30 days without attestation decays score by 1%/day | Time-simulation test |
| Tier transitions: score crossing threshold emits TierChanged event | Contract event test |
| TraceRank composite score: computed from multi-domain attestations with weights [0.25, 0.15, 0.25, 0.20, 0.15] | Unit test on TraceRank |
| TraceRankCell implements Score protocol with `rate()` and `dimensions()` | Compile check |
| TraceRankCell participates in predict-publish-correct via Bus | Integration test |
| Knowledge publish: entry appears on-chain with correct metadata | Contract test |
| Knowledge challenge: challenge opens resolution window, resolve changes state | Contract lifecycle test |
| Challenge resolution: multisig mode requires N-of-M votes within window | Contract test |
| Challenge resolution: arbitrator mode delegates to designated contract | Contract test |
| Challenge resolution: validator vote mode uses time-weighted median | Contract test |
| Challenge resolution: expired window auto-upholds challenge (conservative) | Timeout test |
| Knowledge decay: 90-day-old unrefreshed entry transitions to Stale | Time-simulation test |
| ZK-HDC proof: similarity proof verifies correctly | ZK circuit test |
| ZK-HDC proof: mismatched vectors fail verification | Negative test |
| Chain witness: GateVerdict witness anchors on-chain | Integration test |
| Chain witness: verification Cell confirms on-chain anchor | Round-trip test |
| GossipCell implements React protocol with `watch_topics()` | Compile check |
| GossipCell watches Bus for peer announcements and emits discovery Pulses | Integration test |
| Gossip: PeerAnnounce propagates identity to connected peers | Network simulation test |
| Gossip: KnowledgeShare propagates entry metadata | Network simulation test |
| Gossip: rate limiting throttles abusive peers | Rate-limit test |
| Gossip: ERC-8004 identity required for participation | Identity check test |
| Bounty lifecycle: post -> claim -> submit -> accept -> settle | Contract lifecycle test |
| Bounty expiration: unclaimed bounty past deadline refunds poster | Timeout test |
| Bounty tier gating: low-tier agent cannot claim Premium bounty | Tier enforcement test |
| Event indexer: processes all 7 contract event types | Indexer integration test |
| Event indexer: lag stays under 10 blocks under normal load | Performance test |
| Event indexer: SSE stream delivers events within 2s of chain confirmation | Latency test |
| Event indexer: rebuild from chain history produces identical state | Determinism test |
| Indexer REST API: all 11 endpoints return correct data | API contract test |
| Contract addresses: Mirage addresses match hardhat deployment output | Deployment verification |
| Reputation in cascade router: higher-tier agents routed to complex tasks | Integration test |
| Identity auto-registration: agent stores tokenId in `.roko/state/identity.json` | State persistence test |
| Configuration: `[chain.contracts]` section loads and connects to correct addresses | Config round-trip test |
