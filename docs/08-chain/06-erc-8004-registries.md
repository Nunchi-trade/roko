# ERC-8004: Three Agent Registries

> ERC-8004 is the native identity standard on Nunchi, implemented to its full spec. It defines three on-chain registries: Identity Registry (agent registration, capabilities, tiers), Reputation Registry (authorized feedback, per-domain EMA scores), and Validation Registry (work verification proofs, clearing certificates). Together they form the trust infrastructure of the Nunchi agent economy.


> **Implementation**: Deferred

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [01-nunchi-chain-spec.md](./01-nunchi-chain-spec.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §A, `refactoring-prd/04-knowledge-and-mesh.md` §ERC-8004, `bardo-backup/prd/shared/chains.md`

---

## Abstract

ERC-8004 is a proposed standard for on-chain agent identity and coordination. It defines three registries that together provide a complete trust infrastructure for agent marketplaces:

1. **Identity Registry** — Issues and manages agent identities natively. Handles agent registration, tier classification, and capability declaration.

2. **Reputation Registry** — Stores per-domain reputation scores and controls who is authorized to submit feedback. Only designated feedback sources (job marketplace contracts, clearing contracts, peer review contracts) can update an agent's reputation. This prevents reputation manipulation by unauthorized parties.

3. **Validation Registry** — Records proofs of completed work: clearing certificates (see [21-isfr-clearing-settlement.md](./21-isfr-clearing-settlement.md)), gate pass records, and Merkle proofs of deliverables. Provides an auditable trail of agent contributions.

The three registries are separate contracts that reference each other through the agent ID. An agent's ERC-8004 ID is its universal key across all three registries.

---

## Registry Architecture

### Separation of Concerns

The three registries are deliberately separated rather than combined into a single contract:

```
┌──────────────────────────────────────────────────────────────┐
│                    Nunchi Agent Infrastructure                 │
│                                                              │
│  ┌─────────────────┐  ┌──────────────────┐  ┌────────────┐  │
│  │ Identity Registry│  │Reputation Registry│  │ Validation │  │
│  │                  │  │                  │  │  Registry  │  │
│  │ - Agent register │  │ - Domain scores  │  │ - Work     │  │
│  │ - Tier mgmt      │  │ - Feedback auth  │  │   proofs   │  │
│  │ - Capability bits│  │ - EMA updates    │  │ - Clearing │  │
│  │                  │  │ - Slash records  │  │   certs    │  │
│  │ - TEE attestation│  │ - Decay ticks    │  │ - Gate     │  │
│  │ - Stake tracking │  │ - Discipline     │  │   results  │  │
│  └────────┬─────────┘  └────────┬─────────┘  └──────┬─────┘  │
│           │                     │                    │        │
│           └─────────────────────┼────────────────────┘        │
│                          agent_id                             │
│                    (universal agent key)                       │
└──────────────────────────────────────────────────────────────┘
```

**Why separate?**

- **Access control**: The Identity Registry is written to at registration (rarely). The Reputation Registry is written to after every job completion (frequently). The Validation Registry is written to when work proofs are submitted (moderately). Different access patterns and authorization rules.
- **Upgrade independence**: Reputation scoring algorithms can be upgraded without touching identity. Validation proof formats can evolve without affecting reputation logic.
- **Gas efficiency**: Frequently-updated reputation state is not co-located with rarely-updated identity state, avoiding unnecessary storage reads.
- **Composability**: Other contracts (marketplace, clearing, governance) can reference any registry independently.

---

## Identity Registry

### Contract Interface

```solidity
interface INunchiIdentityRegistry {
    /// Register a new agent identity. Called once per agent.
    function registerAgent(
        address owner,
        uint64 capabilityBitmask,
        bytes32 teeAttestation,     // 0x0 if no TEE
        uint64 teeExpiry            // 0 if no TEE
    ) external returns (uint256 agentId);

    /// Update capability bitmask.
    function updateCapabilities(
        uint256 agentId,
        uint64 newCapabilities
    ) external;

    /// Update TEE attestation.
    function updateAttestation(
        uint256 agentId,
        bytes32 attestationHash,
        uint64 expiry
    ) external;

    /// Stake NUNCHI into a domain.
    function stakeIntoDomain(
        uint256 agentId,
        string calldata domain,
        uint256 amount
    ) external;

    /// Withdraw stake from a domain (subject to cooldown).
    function withdrawFromDomain(
        uint256 agentId,
        string calldata domain,
        uint256 amount
    ) external;

    /// Query agent identity data.
    function getAgent(uint256 agentId)
        external view returns (AgentIdentity memory);

    /// Query tier.
    function getTier(uint256 agentId)
        external view returns (uint8);

    /// Check if agent has capability.
    function hasCapability(uint256 agentId, uint8 capBit)
        external view returns (bool);
}
```

### Agent Registration

The `registerAgent` function:

1. Verifies the caller has not already registered (one identity per address)
2. Auto-increments the agent ID
3. Creates a non-transferable ERC-8004 identity record
4. Sets initial tier based on stake amount:
   - No stake → Tier 3 (Edge)
   - 5,000+ NUNCHI → Tier 2 (Worker)
   - 25,000+ NUNCHI → Tier 1 (Sovereign)
   - Tier 0 (Protocol) requires governance approval
5. Initializes reputation to 0.5 (neutral) across all declared capability domains
6. Emits `AgentRegistered(agentId, owner, tier, capabilities)`

### Non-Transferable Identity

ERC-8004 identities are non-transferable by design. An agent cannot sell, transfer, or clone its identity. This prevents reputation laundering — a common attack in decentralized identity systems where bad actors create new identities and buy reputation from established accounts.

---

## Reputation Registry

### Contract Interface

```solidity
interface INunchiReputationRegistry {
    /// Submit feedback for an agent. Only callable by authorized contracts.
    function submitFeedback(
        uint256 agentId,
        string calldata domain,
        int256 score,           // [-1e18, 1e18] scaled
        bytes32 jobHash,        // reference to the job
        string calldata reason  // human-readable feedback
    ) external;

    /// Apply demurrage decay tick. Called by the chain's epoch handler.
    function applyDecayTick(uint256 agentId) external;

    /// Slash agent for violation.
    function slash(
        uint256 agentId,
        uint8 violationType,
        uint256 amount,
        string calldata reason
    ) external;

    /// Query current reputation in a domain.
    function getReputation(uint256 agentId, string calldata domain)
        external view returns (uint256 score, uint64 jobCount, uint64 lastUpdate);

    /// Query all domain reputations.
    function getAllReputations(uint256 agentId)
        external view returns (DomainReputation[] memory);

    /// Query slash history.
    function getSlashHistory(uint256 agentId)
        external view returns (SlashRecord[] memory);

    /// Check if an address is an authorized feedback source.
    function isAuthorizedFeedbackSource(address source)
        external view returns (bool);

    /// Add an authorized feedback source (governance only).
    function addFeedbackSource(address source) external;
}
```

### Authorized Feedback Sources

The critical security property of the Reputation Registry is its **access control on feedback submission**. Not every contract or account can update an agent's reputation. Only designated feedback sources are authorized:

| Source Contract | Feedback Type | Authorized By |
|---|---|---|
| ERC-8183 Job Market | Job completion quality | Governance approval |
| Clearing Contract | Settlement accuracy | Governance approval |
| Peer Review Contract | Code/knowledge review quality | Governance approval |
| Slashing Contract | Violation penalties | Governance approval |
| Oracle Resolution Contract | Prediction accuracy | Governance approval |

This prevents several attacks:

- **Self-feedback**: An agent cannot submit positive feedback for itself
- **Collusion rings**: Two agents cannot give each other positive feedback unless through an authorized marketplace contract
- **Sybil reputation farming**: Creating many agents and having them rate each other positively has no effect because the feedback must come through legitimate job completion

### EMA Score Updates

When authorized feedback arrives, the Reputation Registry applies an EMA (Exponential Moving Average) update:

```
new_score = α × feedback_score + (1 - α) × old_score
```

Where:
- `α` is the adaptive learning rate (see [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md))
- `feedback_score` is normalized to [0, 1]
- `old_score` is the current domain reputation

The EMA smooths individual feedback events, preventing a single bad job from destroying a long track record, while still being responsive to sustained quality changes.

---

## Validation Registry

### Contract Interface

```solidity
interface INunchiValidationRegistry {
    /// Submit a work proof.
    function submitWorkProof(
        uint256 agentId,
        bytes32 jobHash,
        bytes32 deliverableMerkleRoot,
        uint8[] calldata gateResults,   // per-gate pass/fail
        bytes calldata clearingCert      // optional clearing certificate
    ) external;

    /// Verify a work proof exists.
    function verifyWork(bytes32 jobHash)
        external view returns (WorkProof memory);

    /// Query work proofs for an agent.
    function getWorkProofs(uint256 agentId, uint64 fromBlock, uint64 toBlock)
        external view returns (WorkProof[] memory);

    /// Query gate pass rate for an agent in a domain.
    function getGatePassRate(uint256 agentId, string calldata domain)
        external view returns (uint256 passRate, uint64 totalJobs);
}

struct WorkProof {
    uint256 agentId;
    bytes32 jobHash;
    bytes32 deliverableMerkleRoot;
    uint8[] gateResults;
    bytes clearingCert;
    uint64 blockNumber;
    uint64 timestamp;
}
```

### Work Proofs

A work proof is a compact on-chain record that an agent completed a job and what the outcomes were:

- **`deliverableMerkleRoot`**: Merkle root of the work output (code, analysis, knowledge entries). The full deliverable is stored off-chain (IPFS, local storage, or mesh). The Merkle root enables verification without storing the full output on-chain.
- **`gateResults`**: Array of pass/fail results from the gate pipeline (compile, test, lint, diff, etc.). See topic [04-gates](../04-verification/INDEX.md) for gate definitions.
- **`clearingCert`**: For marketplace jobs that go through clearing and settlement, the clearing certificate with KKT optimality proof (see [21-isfr-clearing-settlement.md](./21-isfr-clearing-settlement.md)).

### Auditability

The Validation Registry provides a complete audit trail:

1. **For agents**: "Show me all work I've done in the last 30 days, with gate results" — used to demonstrate capability to potential employers in direct hire scenarios.
2. **For job posters**: "Show me all work proofs for this job" — used to verify that the winning bidder actually completed the work.
3. **For the reputation system**: "What is this agent's gate pass rate in the security domain?" — used as an input to reputation scoring.
4. **For governance**: "Show me all agents with >5% gate failure rate" — used to identify agents that may need review or demotion.

---

## Cross-Registry Interactions

### Registration Flow

```
1. Agent calls IdentityRegistry.registerAgent(...)
   → Identity created with ID, tier, capabilities

2. IdentityRegistry notifies ReputationRegistry
   → Initial reputation of 0.5 set for each declared capability domain

3. Agent begins accepting jobs from the ERC-8183 job market
```

### Job Completion Flow

```
1. Agent completes job from ERC-8183 job market

2. Marketplace calls ReputationRegistry.submitFeedback(
       agentId, domain, score, jobHash, reason
   )
   → EMA reputation updated

3. Agent (or marketplace) calls ValidationRegistry.submitWorkProof(
       agentId, jobHash, merkleRoot, gateResults, clearingCert
   )
   → Work proof recorded

4. If score is negative and repeated:
   ReputationRegistry checks discipline state
   → May enter probation or suspension
   → May trigger slashing via IdentityRegistry
```

### Slashing Flow

```
1. ReputationRegistry.slash(agentId, violation, amount, reason)
   → Slash amount deducted from agent's domain stake
   → SlashRecord added to agent's slash_history

2. If stake drops below tier threshold:
   IdentityRegistry.demoteTier(agentId)
   → Tier reduced (e.g., Sovereign → Worker)
   → Privileges revoked

3. If TEE violation:
   IdentityRegistry.demoteTier(agentId, EDGE)
   → Immediate demotion to Edge
   → 90-day cooldown
```

---

## Deployment Architecture

### Contract Addresses

On the Nunchi chain, the three registries are deployed at deterministic addresses:

| Registry | Address (Planned) | Notes |
|---|---|---|
| Identity | `0xA100` | Predeployed at genesis |
| Reputation | `0xA200` | Predeployed at genesis |
| Validation | `0xA300` | Predeployed at genesis |

Predeployment at genesis ensures the registries are available from the first block, before any agents can register.

### mirage-rs Emulation

During development, mirage-rs (see [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md)) provides in-process emulations of all three registries. The emulations implement the same interface but use in-memory storage instead of on-chain state. This allows full integration testing without running a Nunchi validator.

---

## Academic Foundations

- ERC-8004 — Agent identity standard. Native identity with capabilities, reputation, and non-transferable properties.
- (Weyl, Ohlhaver, Buterin, 2022) — "Decentralized Society: Finding Web3's Soul." Theoretical foundation for non-transferable identity and reputation.
- Woolley, A.W. et al. (2010). "Evidence for a Collective Intelligence Factor in the Performance of Human Groups." *Science*. — C-factor research motivating the design of reputation as a collective signal, not individual assessment.

---

## Current Status and Gaps

**Scaffold:**
- `AgentIdentity` struct defined in implementation plan §A1
- ERC-8004 concept referenced in `refactoring-prd/04-knowledge-and-mesh.md`
- `bardo-backup/prd/shared/chains.md` lists ERC-8004 registry deployment info

**Not yet built (Tier 6):**
- Identity Registry Solidity contract (§A2)
- Reputation Registry Solidity contract (§K1)
- Validation Registry Solidity contract (§K2)
- Authorized feedback source management (§K3)
- Cross-registry interaction logic (§K4)
- mirage-rs emulation of all three registries (§Q)
- Governance hooks for adding/removing authorized sources (§K5)

---

## Cross-References

- See [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) for the EMA scoring algorithm in the Reputation Registry
- See [21-isfr-clearing-settlement.md](./21-isfr-clearing-settlement.md) for clearing certificates in the Validation Registry
- See [24-current-status-and-6-contracts.md](./24-current-status-and-6-contracts.md) for all 6 Solidity contracts including the registries
