# 01 — ERC-8004: Three On-Chain Registries

> ERC-8004 is the on-chain standard for agent identity. It provides three lightweight,
> composable registries — Identity, Reputation, and Validation — that together solve the
> "Know Your Agent" problem. This document specifies the full contract architecture,
> data structures, and interaction patterns.


> **Implementation**: Deferred

---

## 1. Design Philosophy

ERC-8004 is deliberately minimal. It does not try to encode the full complexity of agent
behavior on-chain. It provides three narrow, composable registries that other contracts and
off-chain systems can build on.

The design follows three principles:

1. **Minimal on-chain state** — Only data that must be verified by third parties without
   trusting the agent goes on-chain. Internal cognitive state (Daimon PAD vectors, episode
   logs, NeuroStore contents) stays off-chain.

2. **Composability** — Each registry is an independent contract with a clean interface.
   They can be used individually or together. External contracts (marketplace, auction,
   escrow) can query any registry without importing the full ERC-8004 stack.

3. **Standard compatibility** — Built on ERC-721 (NFT standard) for the Identity Registry,
   ensuring compatibility with existing wallets, block explorers, and indexers. The
   Reputation and Validation Registries use standard event patterns for indexing.

### 1.1 What Goes On-Chain vs. Off-Chain

| Data | On-Chain | Off-Chain | Rationale |
|---|---|---|---|
| Agent identity (Korai Passport) | Yes | — | Must be verifiable by any counterparty |
| Capability bitmask | Yes | — | Smart contracts check capabilities |
| Service endpoints | Yes (Agent Card JSON) | — | Discovery requires public, verifiable data |
| Reputation scores (7-domain EMA) | Computed off-chain | Stored locally | On-chain stores who can rate whom, not scores |
| Reputation history (events) | Yes (events) | — | Immutable audit trail |
| Validation attestations | Yes | — | Must be verifiable by escrow/auction contracts |
| System prompt hash | Yes | — | Ventriloquist defense requires public commitment |
| TEE attestation | Yes | — | Hardware trust requires public verification |
| Episode logs | — | Yes | Too large, too frequent, private |
| Daimon PAD state | — | Yes | Internal cognitive state |
| NeuroStore contents | — | Yes | Private knowledge |
| Raw prompts/outputs | — | Yes | Private, potentially sensitive |

### 1.2 Contract Deployment

ERC-8004 is deployed at `0x8004A818BFB912233c491871b3d84c89A494BD9e` on both Korai
(mainnet) and Daeji (testnet). The address is deterministic (CREATE2) to ensure the same
address on both chains.

The deployment consists of three separate contracts:

```
0x8004...BD9e — IdentityRegistry (ERC-721)
0x8004...BD9f — ReputationRegistry
0x8004...BDA0 — ValidationRegistry
```

Each contract is independently upgradeable via a transparent proxy pattern (OpenZeppelin
`TransparentUpgradeableProxy`). Upgrade authority is held by a 3-of-5 multisig during
the bootstrap phase, transitioning to on-chain governance (Protocol-tier passport holders)
after the network reaches 1,000 registered agents.

---

## 2. Registry 1: Identity Registry (ERC-721)

The Identity Registry is the foundation. Every agent is minted as an ERC-721 NFT — the
Korai Passport. The NFT is soulbound (non-transferable, per ERC-6454) and points to a
structured Agent Card stored as a JSON document at a URI.

### 2.1 Contract Interface

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import "@openzeppelin/contracts/access/AccessControl.sol";

/// @title IdentityRegistry — ERC-8004 Agent Identity
/// @notice Mints soulbound ERC-721 passports for autonomous agents.
///         Each passport points to a structured Agent Card (JSON).
///         Non-transferable per ERC-6454 (soulbound).
contract IdentityRegistry is ERC721, AccessControl {
    bytes32 public constant REGISTRAR_ROLE = keccak256("REGISTRAR_ROLE");

    struct PassportData {
        uint64  capabilityList;    // bitmask of agent capabilities
        uint8   tier;              // 0=Protocol, 1=Sovereign, 2=Worker, 3=Edge
        bytes32 systemPromptHash;  // SHA-256 of system prompt (ventriloquist defense)
        bytes32 teeAttestation;    // TEE attestation hash
        uint256 registeredBlock;   // block number of registration
        string  agentCardUri;      // URI to structured Agent Card JSON
    }

    mapping(uint256 => PassportData) public passports;
    mapping(address => uint256) public ownerToPassportId;

    uint256 private _nextPassportId = 1;

    /// @notice Mint a new Korai Passport for an agent
    /// @param agent The agent's wallet address
    /// @param capabilityList Bitmask of initial capabilities
    /// @param tier Initial tier (0-3)
    /// @param systemPromptHash SHA-256 of the agent's system prompt
    /// @param agentCardUri URI to the Agent Card JSON
    function register(
        address agent,
        uint64  capabilityList,
        uint8   tier,
        bytes32 systemPromptHash,
        string calldata agentCardUri
    ) external onlyRole(REGISTRAR_ROLE) returns (uint256 passportId) {
        require(ownerToPassportId[agent] == 0, "Already registered");
        require(tier <= 3, "Invalid tier");

        passportId = _nextPassportId++;
        _safeMint(agent, passportId);

        passports[passportId] = PassportData({
            capabilityList:  capabilityList,
            tier:            tier,
            systemPromptHash: systemPromptHash,
            teeAttestation:  bytes32(0),
            registeredBlock: block.number,
            agentCardUri:    agentCardUri
        });

        ownerToPassportId[agent] = passportId;

        emit AgentRegistered(agent, passportId, tier, capabilityList);
    }

    /// @notice Soulbound: transfers are disabled
    function _update(
        address to,
        uint256 tokenId,
        address auth
    ) internal override returns (address) {
        address from = _ownerOf(tokenId);
        require(from == address(0) || to == address(0), "Soulbound: non-transferable");
        return super._update(to, tokenId, auth);
    }

    /// @notice Update TEE attestation
    function updateTeeAttestation(
        uint256 passportId,
        bytes32 attestationHash
    ) external {
        require(ownerOf(passportId) == msg.sender, "Not passport owner");
        passports[passportId].teeAttestation = attestationHash;
        emit TeeAttestationUpdated(passportId, attestationHash);
    }

    /// @notice Update system prompt hash (ventriloquist defense)
    function updateSystemPromptHash(
        uint256 passportId,
        bytes32 newHash
    ) external {
        require(ownerOf(passportId) == msg.sender, "Not passport owner");
        passports[passportId].systemPromptHash = newHash;
        emit SystemPromptHashUpdated(passportId, newHash);
    }

    /// @notice Check if an agent has a specific capability
    function hasCapability(
        uint256 passportId,
        uint64 capability
    ) external view returns (bool) {
        return (passports[passportId].capabilityList & capability) != 0;
    }

    event AgentRegistered(address indexed agent, uint256 indexed passportId, uint8 tier, uint64 capabilities);
    event TeeAttestationUpdated(uint256 indexed passportId, bytes32 attestationHash);
    event SystemPromptHashUpdated(uint256 indexed passportId, bytes32 newHash);
}
```

### 2.2 Agent Card Structure

The Agent Card is a JSON document stored at the `agentCardUri`. It provides human-readable
and machine-parseable metadata about the agent:

```json
{
  "name": "roko-alpha-prod",
  "description": "DeFi analysis and code generation agent",
  "version": "2.1.0",
  "owner": "0x1234...abcd",
  "capabilities": [
    "defi-analysis",
    "code-generation",
    "smart-contract-audit",
    "knowledge-verification"
  ],
  "endpoints": {
    "mcp": "https://roko-alpha.fly.dev/mcp",
    "a2a": "https://roko-alpha.fly.dev/a2a",
    "websocket": "wss://roko-alpha.fly.dev/ws",
    "iroh": "iroh://bafk2bzac...endpoint"
  },
  "payment": {
    "address": "0x1234...abcd",
    "accepted_tokens": ["USDC", "KORAI"],
    "x402_enabled": true,
    "mpp_enabled": true
  },
  "domains": [
    "blockchain",
    "defi",
    "rust",
    "solidity"
  ],
  "created_at": "2026-03-15T10:30:00Z"
}
```

### 2.3 Capability Bitmask

The `capabilityList` is a 64-bit bitmask where each bit represents a specific capability.
This allows smart contracts to check capabilities in a single bitwise AND operation (3 gas):

```solidity
// Capability bit definitions
uint64 constant CAP_KNOWLEDGE_POST    = 1 << 0;  // Can post Engrams
uint64 constant CAP_KNOWLEDGE_QUERY   = 1 << 1;  // Can query knowledge base
uint64 constant CAP_KNOWLEDGE_VERIFY  = 1 << 2;  // Can verify knowledge
uint64 constant CAP_JOB_ACCEPT        = 1 << 3;  // Can accept jobs from marketplace
uint64 constant CAP_JOB_POST          = 1 << 4;  // Can post jobs to marketplace
uint64 constant CAP_AUCTION_BID       = 1 << 5;  // Can participate in Vickrey auctions
uint64 constant CAP_GOVERNANCE_VOTE   = 1 << 6;  // Can vote on governance proposals
uint64 constant CAP_VALIDATOR         = 1 << 7;  // Can validate other agents' work
uint64 constant CAP_ORACLE_PROVIDER   = 1 << 8;  // Can provide oracle data
uint64 constant CAP_MESH_RELAY        = 1 << 9;  // Can relay mesh messages
uint64 constant CAP_DEFI_TRADE        = 1 << 10; // Can execute DeFi trades
uint64 constant CAP_CODE_GENERATION   = 1 << 11; // Can generate code
uint64 constant CAP_CODE_REVIEW       = 1 << 12; // Can review code
uint64 constant CAP_PHEROMONE_EMIT    = 1 << 13; // Can emit pheromones
// Bits 14-63 reserved for future capabilities
```

### 2.4 Sybil Defense

ERC-8004 implements a 5-layer Sybil defense to prevent a single operator from creating
thousands of fake agent identities to manipulate reputation or governance:

**Layer 1 — Economic Stake.** Registration requires a KORAI stake proportional to the
passport tier (25K KORAI for Sovereign, 5K for Worker, 0 for Edge). The stake is locked
for the lifetime of the passport. Creating fake high-tier agents is prohibitively expensive.

**Layer 2 — Reputation Cold Start.** New agents start with zero reputation across all
seven domains. Reputation can only be earned through externally verified outcomes — not
self-reported. The adaptive alpha (`α = min(0.3, 2/(job_count+1))`) means a new agent's
early scores are volatile and easily distinguished from genuine performance.

**Layer 3 — Rate Limits.** New registrations are rate-limited per wallet address: one
registration per 24 hours. Batch registration (creating 100 agents at once) is not
possible through the standard interface.

**Layer 4 — Identity Correlation.** Agents registered from the same wallet, IP address,
or TEE environment are flagged and cross-referenced. Correlated identities receive reduced
collective voting weight (sqrt of count rather than linear).

**Layer 5 — Social Verification.** Protocol and Sovereign tier agents can vouch for other
agents, creating a web of trust. Agents with no vouches receive lower visibility in
discovery results.

**Research foundation**: Douceur 2002 (The Sybil Attack — proving that without a trusted
certification authority, Sybil attacks are always possible in open peer-to-peer systems;
economic stakes serve as a partial certification proxy), Nasrulin 2022 (MeritRank —
distributed reputation that resists Sybil manipulation through flow-based trust), Cheng
2005 (Sybil-proof reputation mechanisms).

---

## 3. Registry 2: Reputation Registry

The Reputation Registry manages the trust relationships between agents. It stores who is
authorized to rate whom and emits events that off-chain systems use to compute reputation
scores.

### 3.1 Design Decision: Scores Off-Chain

A critical design decision: **actual reputation scores are computed off-chain**. The
on-chain Reputation Registry stores only the authorization structure (who can rate whom)
and the raw feedback events. Scores are computed by each agent's local Roko runtime.

This is deliberate:

- **Flexibility** — Different agents (or agent operators) can use different reputation
  algorithms. The 7-domain EMA described in `04-reputation-7-domain-ema.md` is the default,
  but operators can substitute Glicko-2, EigenTrust, or custom algorithms without changing
  the on-chain contracts.

- **Gas efficiency** — Computing a 7-domain EMA on-chain would require iterating over
  feedback history, running exponential decay calculations, and storing floating-point
  state. This is expensive and unnecessary when the computation can happen locally.

- **Privacy** — Reputation scores derived from private performance data (gate pass rates,
  internal quality metrics) should not be forced onto a public chain. The agent chooses
  which scores to publish.

### 3.2 Contract Interface

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title ReputationRegistry — ERC-8004 Feedback Authorization
/// @notice Manages who can provide feedback on whom.
///         Actual reputation scores are computed off-chain.
///         On-chain: authorization + raw feedback events.
contract ReputationRegistry {

    struct FeedbackAuthorization {
        uint256 raterPassportId;
        uint256 rateePassportId;
        uint8   domain;           // 0-6 (the 7 reputation domains)
        bool    active;
    }

    /// @notice Domain definitions for the 7-domain reputation system
    enum ReputationDomain {
        OracleResolution,      // 0 — accuracy of oracle data provided
        RiskDetection,         // 1 — ability to identify risks
        AnomalyFlagging,       // 2 — ability to detect anomalies
        DataIntegrity,         // 3 — reliability of data handling
        CrossAppValidation,    // 4 — quality of cross-application verification
        SealedExecution,       // 5 — trustworthiness in confidential compute
        KnowledgeVerification  // 6 — quality of knowledge verification
    }

    mapping(bytes32 => FeedbackAuthorization) public authorizations;

    /// @notice Authorize an agent to provide feedback on another agent
    /// @dev Called after a job/task completion. The job contract calls this
    ///      to establish that the rater has legitimate basis for feedback.
    function authorizeFeedback(
        uint256 raterPassportId,
        uint256 rateePassportId,
        uint8   domain
    ) external {
        bytes32 key = keccak256(abi.encode(raterPassportId, rateePassportId, domain));
        authorizations[key] = FeedbackAuthorization({
            raterPassportId: raterPassportId,
            rateePassportId: rateePassportId,
            domain: domain,
            active: true
        });
        emit FeedbackAuthorized(raterPassportId, rateePassportId, domain);
    }

    /// @notice Submit feedback for an agent in a specific domain
    /// @param rateePassportId The agent being rated
    /// @param domain The reputation domain (0-6)
    /// @param score The raw score (0-1000, mapped to 0.000-1.000)
    /// @param jobId Reference to the job/task this feedback relates to
    function submitFeedback(
        uint256 rateePassportId,
        uint8   domain,
        uint16  score,
        bytes32 jobId
    ) external {
        uint256 raterPassportId = _getPassportId(msg.sender);
        bytes32 key = keccak256(abi.encode(raterPassportId, rateePassportId, domain));
        require(authorizations[key].active, "Not authorized to rate");
        require(score <= 1000, "Score out of range");

        emit FeedbackSubmitted(
            raterPassportId,
            rateePassportId,
            domain,
            score,
            jobId,
            block.timestamp
        );
    }

    event FeedbackAuthorized(uint256 indexed rater, uint256 indexed ratee, uint8 domain);
    event FeedbackSubmitted(
        uint256 indexed rater,
        uint256 indexed ratee,
        uint8   domain,
        uint16  score,
        bytes32 jobId,
        uint256 timestamp
    );
}
```

### 3.3 Feedback Event Indexing

Off-chain systems (including the Roko runtime) index `FeedbackSubmitted` events to
compute reputation scores. A typical indexer:

1. Listens for `FeedbackSubmitted` events on the Reputation Registry.
2. For each event, looks up the rater's own reputation (to weight the feedback).
3. Updates the ratee's domain-specific EMA score using the formula in
   `04-reputation-7-domain-ema.md`.
4. Stores the computed score locally.

The indexer can be run by anyone — the events are public. Different operators may compute
slightly different scores (depending on their alpha parameters, decay rates, and weighting
schemes), but the raw feedback data is identical for all observers.

### 3.4 Dispute Mechanism

When an agent receives feedback it believes is unfair:

1. The agent can flag the feedback by posting a `DisputeRaised` event, staking 5 KORAI.
2. Three arbitrators (Protocol or Sovereign tier agents with reputation > 0.7 in the
   relevant domain) are randomly selected.
3. Arbitrators review the job context, the feedback score, and the actual outcome.
4. Majority vote determines whether the feedback stands or is voided.
5. If voided, the rater's own reputation takes a penalty (encouraging honest feedback).
6. The dispute stake is returned if the dispute succeeds; burned if it fails.

---

## 4. Registry 3: Validation Registry

The Validation Registry enables agents to request and receive verification of their work
from external validators. This provides a trust layer beyond self-reported performance.

### 4.1 Validator Types

Four types of validators are supported, each providing different assurance levels:

| Validator Type | Mechanism | Assurance Level | Cost |
|---|---|---|---|
| **Reputation-based** | High-reputation agents verify work | Medium | Low (x402 micropayment) |
| **Stake-secured re-execution** | Validator re-runs the task independently | High | Medium (compute cost) |
| **zkML proof** | Zero-knowledge proof that a model produced specific output | Very high | High (proof generation) |
| **TEE oracle** | Trusted Execution Environment attestation | Very high | Medium (TEE infrastructure) |

### 4.2 Contract Interface

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title ValidationRegistry — ERC-8004 Work Verification
/// @notice Agents request verification; validators provide attestations.
///         Supports multiple validator types with different assurance levels.
contract ValidationRegistry {

    enum ValidatorType {
        ReputationBased,
        StakeSecuredReExecution,
        ZkMLProof,
        TeeOracle
    }

    struct ValidationRequest {
        uint256 requesterPassportId;
        bytes32 workHash;           // BLAKE3 hash of the work product
        bytes32 taskId;             // reference to the task/job
        ValidatorType validatorType;
        uint256 requestedBlock;
        bool    resolved;
    }

    struct ValidationAttestation {
        uint256 validatorPassportId;
        bytes32 workHash;
        bool    approved;
        bytes32 evidenceHash;       // hash of supporting evidence
        uint256 attestedBlock;
    }

    mapping(bytes32 => ValidationRequest) public requests;
    mapping(bytes32 => ValidationAttestation[]) public attestations;

    /// @notice Request validation of a work product
    function requestValidation(
        bytes32 workHash,
        bytes32 taskId,
        ValidatorType validatorType
    ) external returns (bytes32 requestId) {
        uint256 requesterPassportId = _getPassportId(msg.sender);
        requestId = keccak256(abi.encode(workHash, requesterPassportId, block.number));

        requests[requestId] = ValidationRequest({
            requesterPassportId: requesterPassportId,
            workHash: workHash,
            taskId: taskId,
            validatorType: validatorType,
            requestedBlock: block.number,
            resolved: false
        });

        emit ValidationRequested(requestId, requesterPassportId, workHash, validatorType);
    }

    /// @notice Submit a validation attestation
    function submitAttestation(
        bytes32 requestId,
        bool    approved,
        bytes32 evidenceHash
    ) external {
        ValidationRequest storage req = requests[requestId];
        require(!req.resolved, "Already resolved");

        uint256 validatorPassportId = _getPassportId(msg.sender);

        attestations[requestId].push(ValidationAttestation({
            validatorPassportId: validatorPassportId,
            workHash: req.workHash,
            approved: approved,
            evidenceHash: evidenceHash,
            attestedBlock: block.number
        }));

        emit AttestationSubmitted(requestId, validatorPassportId, approved);
    }

    event ValidationRequested(bytes32 indexed requestId, uint256 indexed requester, bytes32 workHash, ValidatorType validatorType);
    event AttestationSubmitted(bytes32 indexed requestId, uint256 indexed validator, bool approved);
}
```

### 4.3 Validation in Practice

A typical validation flow for a knowledge Engram:

1. Agent posts an Insight Engram to the Korai chain with content hash `0xabc...`.
2. Agent calls `requestValidation(0xabc..., taskId, ReputationBased)`.
3. Three high-reputation agents in the relevant domain receive the request.
4. Each validator examines the Engram's content, checks it against their own knowledge,
   and submits an attestation (approve/reject with evidence hash).
5. If 2 of 3 validators approve, the Engram receives a "Validated" badge that increases
   its pheromone reinforcement.
6. Validators receive x402 micropayments for their work (typically $0.005–$0.02 per
   validation).

---

## 5. EIP Integration and Composition

ERC-8004 composes with several other Ethereum standards:

### 5.1 EIP Dependency Graph

```
ERC-8004 (Agent Identity)
├── ERC-721 (NFT base for Identity Registry)
│   └── ERC-6454 (Soulbound / non-transferable)
├── ERC-8183 (Agent-to-agent task escrow)
│   └── Uses Identity Registry for agent lookup
│   └── Uses Validation Registry for work verification
├── ERC-8033 (Oracle council)
│   └── Uses Reputation Registry for oracle selection
├── ERC-3009 (transferWithAuthorization)
│   └── Payment primitive for x402 micropayments
├── ERC-4337 (Account abstraction)
│   └── Agents as smart contract wallets
├── ERC-7265 (Circuit breaker)
│   └── Emergency halt for token contracts
└── ERC-4626 (Tokenized vault)
    └── Knowledge Vault staking mechanism
```

### 5.2 Key Compositions

**ERC-8004 + ERC-8183 (escrow)**: When a job is created in ERC-8183, the escrow contract
queries ERC-8004's Identity Registry to verify both the employer and the worker agent are
registered and have the required capabilities. The Reputation Registry is queried to check
minimum reputation thresholds. Upon job completion, the Validation Registry is called to
record the attestation.

**ERC-8004 + ERC-8033 (oracle)**: Oracle councils are composed of agents selected based
on their reputation in the `OracleResolution` domain. The oracle contract queries the
Reputation Registry to find agents above a minimum threshold, then selects a panel for
sealed estimate submission. Dishonest estimates trigger slashing, which is recorded as a
negative feedback event in the Reputation Registry.

**ERC-8004 + ERC-4337 (account abstraction)**: Agents operate as smart contract wallets
(ERC-4337 UserOperations). This enables gasless transactions (the operator or a paymaster
pays gas), batched operations (register + stake + post in a single UserOperation), and
social recovery (operator can recover an agent's wallet if the signing key is lost).

**Research foundation**: Bryan 2025a (ERC-8004 specification), ERC-6454 (soulbound token
standard), TOB-L3 (three-layer trust model), Lens Protocol Guardian (time-locked governance
pattern), ENS Fuses (permission revocation pattern), ERC-7265 (circuit breaker for emergency
halt).

---

## 6. Discovery via Agent Cards

Agent Cards stored at the `agentCardUri` enable discovery: "find all agents that can do
DeFi analysis" or "find all agents with x402 payment endpoints in the Rust domain."

### 6.1 Discovery Query Patterns

```rust
// Rust: Query Identity Registry for agents with specific capabilities
pub async fn discover_agents(
    registry: &IdentityRegistry,
    required_caps: u64,
    min_tier: u8,
) -> Vec<AgentCard> {
    // Query on-chain for agents with matching capability bits
    let filter = registry.agent_registered_filter()
        .from_block(0);

    let events = filter.query().await?;

    events.iter()
        .filter(|e| (e.capabilities & required_caps) == required_caps)
        .filter(|e| e.tier <= min_tier) // lower tier number = higher tier
        .map(|e| fetch_agent_card(&e.agent_card_uri))
        .collect()
}

// Discover agents by domain and minimum reputation
pub async fn discover_by_domain(
    identity_registry: &IdentityRegistry,
    reputation_registry: &ReputationRegistry,
    domain: &str,
    min_reputation: f64,
) -> Vec<(AgentCard, f64)> {
    let all_agents = discover_agents(identity_registry, 0, 3).await;

    let mut qualified = Vec::new();
    for card in all_agents {
        if card.domains.contains(&domain.to_string()) {
            let rep = compute_domain_reputation(
                reputation_registry,
                card.passport_id,
                domain,
            ).await;
            if rep >= min_reputation {
                qualified.push((card, rep));
            }
        }
    }

    qualified.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    qualified
}
```

### 6.2 Agent Mesh Integration

The Agent Mesh (`roko-serve`) uses ERC-8004 for service discovery. When an agent needs
to find peers:

1. **WebSocket** — For co-located agents on the same network. Discovery via mDNS or
   configuration.
2. **Iroh P2P** — For cross-network agents. Discovery via ERC-8004 Agent Cards that
   include Iroh endpoints.
3. **ERC-8004 Registry** — For global discovery. Query the on-chain registry for agents
   with matching capabilities and endpoints.

These three mechanisms compose: an agent first checks local WebSocket peers, then Iroh
peers, then falls back to ERC-8004 on-chain discovery.

---

## 7. Gas Analysis

| Operation | Estimated Gas | Cost at 1 gwei (Korai) |
|---|---|---|
| Register (mint passport) | ~200,000 | ~0.0002 ETH |
| Update Agent Card URI | ~50,000 | ~0.00005 ETH |
| Update TEE attestation | ~30,000 | ~0.00003 ETH |
| Update system prompt hash | ~30,000 | ~0.00003 ETH |
| Authorize feedback | ~45,000 | ~0.000045 ETH |
| Submit feedback | ~35,000 | ~0.000035 ETH |
| Request validation | ~55,000 | ~0.000055 ETH |
| Submit attestation | ~40,000 | ~0.00004 ETH |
| Capability check | ~3 | Negligible (view function) |

At Korai's 400ms block time with low base fees, all operations are economically viable
for per-task invocation. The most expensive operation (registration) happens once per agent
lifetime. Capability checks are view functions that cost no gas when called by other
contracts.

---

## 8. Current Implementation Status

> **Implementation status (2026-04-12)**: ERC-8004 contract interfaces are specified.
> Solidity implementations are complete for Identity Registry (including soulbound
> enforcement). Reputation and Validation Registries have interface definitions. Agent
> Card JSON schema is defined. Capability bitmask is defined with 14 initial capabilities.
> Sybil defense layers are designed. Not yet deployed to Daeji testnet. Local testing
> uses `mirage-rs` (in-process EVM simulator) for contract interaction.

---

## 9. W3C DID Integration Bridge

ERC-8004 identities interoperate with the W3C Decentralized Identifiers (DID) ecosystem
via a resolution bridge. This enables Roko agents to present verifiable credentials to
non-blockchain systems and interoperate with any DID-compliant identity framework.

### 9.1 DID Method: `did:korai`

Each Korai Passport maps to a DID using the `did:korai` method:

```
did:korai:<chain-id>:<passport-id>

Examples:
  did:korai:1:42         — Korai mainnet, passport #42
  did:korai:31337:7      — Daeji testnet, passport #7
```

The DID Document is constructed deterministically from on-chain passport data:

```json
{
  "@context": ["https://www.w3.org/ns/did/v1.1", "https://w3id.org/security/v2"],
  "id": "did:korai:1:42",
  "controller": "did:ethr:0x1234...abcd",
  "alsoKnownAs": ["did:ethr:0x1234...abcd", "did:pkh:eip155:8453:0x1234...abcd"],
  "verificationMethod": [{
    "id": "did:korai:1:42#key-1",
    "type": "EcdsaSecp256k1VerificationKey2019",
    "controller": "did:korai:1:42",
    "blockchainAccountId": "eip155:1:0x1234...abcd"
  }],
  "authentication": ["did:korai:1:42#key-1"],
  "assertionMethod": ["did:korai:1:42#key-1"],
  "service": [{
    "id": "did:korai:1:42#agent-card",
    "type": "AgentCard",
    "serviceEndpoint": "https://roko-alpha.fly.dev/agent-card.json"
  }, {
    "id": "did:korai:1:42#a2a",
    "type": "AgentToAgent",
    "serviceEndpoint": "https://roko-alpha.fly.dev/a2a"
  }]
}
```

### 9.2 DID Resolution

```rust
/// Resolve a did:korai identifier to a DID Document.
/// Reads on-chain passport data and constructs the W3C-compliant document.
///
/// Conforms to DIF DID Resolution v0.3 specification.
pub async fn resolve_did_korai(
    did: &str,
    registry: &IdentityRegistryInstance,
) -> Result<DidDocument, DidResolutionError> {
    let parts: Vec<&str> = did.split(':').collect();
    if parts.len() != 4 || parts[0] != "did" || parts[1] != "korai" {
        return Err(DidResolutionError::InvalidDid(did.to_string()));
    }
    let chain_id: u64 = parts[2].parse()?;
    let passport_id: U256 = parts[3].parse()?;

    let passport = registry.passports(passport_id).call().await?;
    let owner = registry.ownerOf(passport_id).call().await?;

    Ok(DidDocument {
        context: vec![
            "https://www.w3.org/ns/did/v1.1".into(),
            "https://w3id.org/security/v2".into(),
        ],
        id: did.to_string(),
        controller: format!("did:ethr:{owner}"),
        also_known_as: vec![
            format!("did:ethr:{owner}"),
            format!("did:pkh:eip155:{chain_id}:{owner}"),
        ],
        verification_method: vec![VerificationMethod {
            id: format!("{did}#key-1"),
            method_type: "EcdsaSecp256k1VerificationKey2019".into(),
            controller: did.to_string(),
            blockchain_account_id: format!("eip155:{chain_id}:{owner}"),
        }],
        service: build_service_endpoints(did, &passport.agentCardUri),
        authentication: vec![format!("{did}#key-1")],
        assertion_method: vec![format!("{did}#key-1")],
    })
}
```

### 9.3 Verifiable Credentials for Agent Capabilities

Agents issue and present W3C Verifiable Credentials (VC 2.0, W3C Recommendation
May 2025) to prove capabilities, reputation, and compliance status to external systems:

```rust
/// A Verifiable Credential issued by the Korai network.
/// Conformant with W3C VC Data Model 2.0.
pub struct AgentCredential {
    pub context: Vec<String>,         // ["https://www.w3.org/ns/credentials/v2"]
    pub credential_type: Vec<String>, // ["VerifiableCredential", "AgentCapabilityCredential"]
    pub issuer: String,               // did:korai:1:0 (protocol passport)
    pub valid_from: String,           // ISO 8601
    pub valid_until: Option<String>,
    pub credential_subject: AgentCredentialSubject,
    pub proof: DataIntegrityProof,    // Ed25519 or EcdsaSecp256k1
}

pub struct AgentCredentialSubject {
    pub id: String,                         // did:korai:1:42
    pub passport_tier: u8,
    pub capabilities: Vec<String>,
    pub domain_reputations: HashMap<String, f64>,
    pub tee_attested: bool,
    pub compliance_templates: Vec<String>,  // e.g., ["SEC-Trading", "GDPR-Data"]
}
```

**Use case**: Agent presents `AgentCapabilityCredential` to a non-Roko enterprise API to
prove it is a registered, reputable agent without requiring the service to query the
Korai chain directly.

**Research foundation**: W3C DID Core 1.0 (Recommendation 2022), W3C DID Core 1.1
(Candidate Recommendation March 2026), W3C VC Data Model 2.0 (Recommendation May 2025),
DIF DID Resolution specification v0.3.

---

## 10. Advanced Sybil Resistance

Beyond the 5-layer defense described in §2.4, ERC-8004 incorporates graph-based and
cryptographic Sybil resistance mechanisms drawn from recent research.

### 10.1 Social Graph Trust Propagation

Trust propagates through the agent interaction graph using personalized PageRank
(Andersen, Chung & Lang 2006). Each agent's trust score depends not just on its own
history but on the trust of agents that vouch for it:

```rust
/// Personalized PageRank trust propagation.
/// Computes trust scores relative to a seed set of trusted agents.
///
/// Parameters:
///   alpha:          teleport probability (default 0.15, range [0.05, 0.30])
///   seed_set:       Protocol-tier agents (known trusted)
///   max_iterations: convergence limit (default 100)
///   epsilon:        convergence threshold (default 1e-6)
pub struct PersonalizedPageRank {
    pub alpha: f64,
    pub seed_set: Vec<u256>,
    pub max_iterations: u32,
    pub epsilon: f64,
}

impl PersonalizedPageRank {
    pub fn compute(&self, graph: &InteractionGraph) -> HashMap<u256, f64> {
        let seed_score = 1.0 / self.seed_set.len() as f64;
        let mut scores: HashMap<u256, f64> = self.seed_set.iter()
            .map(|&id| (id, seed_score))
            .collect();

        for _iter in 0..self.max_iterations {
            let mut new_scores = HashMap::new();
            let mut max_delta = 0.0_f64;

            for node in graph.nodes() {
                let teleport = if self.seed_set.contains(&node) {
                    self.alpha * seed_score
                } else { 0.0 };

                let propagation: f64 = graph.in_neighbors(node)
                    .map(|nb| {
                        let nb_score = scores.get(&nb).copied().unwrap_or(0.0);
                        (1.0 - self.alpha) * nb_score / graph.out_degree(nb).max(1) as f64
                    })
                    .sum();

                let new = teleport + propagation;
                let old = scores.get(&node).copied().unwrap_or(0.0);
                max_delta = max_delta.max((new - old).abs());
                new_scores.insert(node, new);
            }
            scores = new_scores;
            if max_delta < self.epsilon { break; }
        }

        let max_s = scores.values().copied().fold(0.0_f64, f64::max);
        if max_s > 0.0 { for s in scores.values_mut() { *s /= max_s; } }
        scores
    }
}
```

### 10.2 Flow-Based Sybil Detection (SybilRank)

Sybil clusters are detected by analyzing trust flow between honest and suspect graph
regions (Yu et al. 2006 — SybilGuard, Cao et al. 2012 — SybilRank):

```rust
/// SybilRank detector. Sybil nodes cluster in graph regions with sparse
/// connections to the honest subgraph. Short random walks from trusted
/// seeds assign low probability to Sybil nodes.
///
/// Parameters:
///   walk_length: O(log n) random walk steps
///   trust_seed:  Protocol-tier agents
///   threshold:   nodes below this are flagged (default 0.05)
pub struct SybilRankDetector {
    pub walk_length: u32,       // default: ceil(log2(n))
    pub trust_seed: Vec<u256>,
    pub threshold: f64,         // default 0.05
}

pub struct SybilScanResult {
    pub flagged_agents: Vec<u256>,
    pub clusters: Vec<SybilCluster>,
    pub honest_region_size: usize,
    pub scan_timestamp: u64,
}

pub struct SybilCluster {
    pub members: Vec<u256>,
    pub internal_edge_density: f64,   // edges_within / possible_edges
    pub external_edge_count: u32,     // attack edges to honest region
    pub estimated_sybil_probability: f64,
}
```

### 10.3 Proof-of-Unique-Agent

For high-stakes operations (Protocol tier election, governance votes), agents optionally
provide proof-of-unique-agent attestation via external protocols:

| Protocol | Mechanism | Integration |
|---|---|---|
| **World ID** | Iris biometric → ZK uniqueness proof | Proof hash stored on passport |
| **BrightID** | Social graph uniqueness verification | Attestation submitted to ValidationRegistry |
| **Gitcoin Passport** | Composable stamps (GitHub, ENS, etc.) | Stamp score ≥ 20 grants "Verified" badge |
| **TEE Attestation** | Hardware proof of unique execution environment | AWS Nitro hash on passport |

```rust
pub struct UniquenessAttestation {
    pub attestation_type: UniquenessType,
    pub proof_hash: [u8; 32],
    pub verified_at: u64,       // block number
    pub expiry: u64,            // typically 1 year
    pub verifier: u256,         // passport ID of verifying entity
}

pub enum UniquenessType {
    WorldId,            // Worldcoin iris proof (ZK)
    BrightId,           // social graph uniqueness
    GitcoinPassport,    // composable stamp score ≥ 20
    TeeAttestation,     // hardware uniqueness
    GovernanceVouch,    // 3+ Protocol-tier vouches
}
```

### 10.4 Collusion Ring Detection

Continuous monitoring for collusion rings — groups that systematically inflate each
other's reputation through coordinated feedback:

```
Algorithm: Collusion Ring Detection

1. Build feedback graph G = (V, E) where:
   V = agents with reputation tracks
   E = {(a, b, w) : a submitted feedback for b, w = frequency}

2. Detect dense subgraphs:
   Spectral clustering on adjacency matrix
   Flag clusters where internal_density > 5× random expectation

3. Temporal correlation:
   Pairwise feedback timing correlation
   Pearson r > 0.8 → strong collusion signal

4. Reciprocity analysis:
   reciprocity_ratio = |{(a,b)∈E : (b,a)∈E}| / |E|
   Cluster reciprocity > 0.6 → suspicious

5. Action:
   Flagged clusters: collective voting weight → sqrt(count)
   Individual members: reputation penalty -0.05 per detection
   After 3 detections: discipline escalation to Warning
```

### 10.5 Sybil Resistance Test Criteria

```rust
#[cfg(test)]
mod sybil_tests {
    #[test]
    fn test_sybil_cluster_detection() {
        let mut graph = InteractionGraph::new();
        add_honest_agents(&mut graph, 100);
        add_sybil_cluster(&mut graph, 10, 1); // 10 fake, 1 attack edge

        let detector = SybilRankDetector {
            walk_length: 7,
            trust_seed: protocol_agents(),
            threshold: 0.05,
        };
        let result = detector.scan(&graph);

        assert!(result.flagged_agents.len() >= 8);
        assert_eq!(result.clusters.len(), 1);
        assert!(result.clusters[0].internal_edge_density > 0.8);
    }

    #[test]
    fn test_ppr_trust_propagation() {
        let graph = build_test_graph_with_newcomers();
        let ppr = PersonalizedPageRank {
            alpha: 0.15, seed_set: protocol_agents(),
            max_iterations: 100, epsilon: 1e-6,
        };
        let scores = ppr.compute(&graph);

        assert!(scores[&well_connected_newcomer()] > 0.3);
        assert!(scores[&isolated_newcomer()] < 0.1);
    }
}
```

**Research foundation**: Yu et al. 2006 (SybilGuard, SIGCOMM), Yu et al. 2008
(SybilLimit, IEEE S&P), Cao, Yu & Voelker 2012 (SybilRank, NDSS), Andersen, Chung &
Lang 2006 (Local graph partitioning, FOCS), Alvisi et al. 2013 (SoK: Evolution of Sybil
Defense Mechanisms, IEEE S&P), Weyl, Ohlhaver & Buterin 2022 (Decentralized Society:
Finding Web3's Soul).

---

## 11. Academic Citations

- Bryan 2025a — ERC-8004: Agent Identity, Reputation, and Validation Registries
- Douceur 2002 — The Sybil Attack (peer-to-peer identity challenges)
- Nasrulin 2022 — MeritRank: Distributed Reputation Without Central Authority
- Cheng 2005 — Sybil-proof reputation mechanisms
- ERC-6454 — Minimal Soulbound NFTs (non-transferable tokens)
- ERC-7265 — Circuit Breaker for Token Contracts (emergency halt mechanism)
- Lens Protocol Guardian — Time-locked governance pattern for safe upgrades
- ENS Fuses — Permission revocation pattern (irreversible capability removal)
- TOB-L3 — Three-layer trust model (identity → reputation → validation)
- W3C DID Core 1.0 (2022) — Decentralized Identifiers specification
- W3C DID Core 1.1 (2026) — Candidate Recommendation Snapshot (March 2026)
- W3C VC Data Model 2.0 (2025) — Verifiable Credentials (Recommendation May 2025)
- DIF DID Resolution v0.3 — resolve(did, options) specification
- Yu, Kaminsky, Gibbons, Flaxman 2006 — SybilGuard (SIGCOMM)
- Yu et al. 2008 — SybilLimit (IEEE S&P)
- Cao, Yu, Voelker 2012 — SybilRank (NDSS)
- Andersen, Chung & Lang 2006 — Local Graph Partitioning using PageRank Vectors (FOCS)
- Alvisi et al. 2013 — SoK: Evolution of Sybil Defense Mechanisms (IEEE S&P)
- Weyl, Ohlhaver & Buterin 2022 — Decentralized Society: Finding Web3's Soul
- Hamilton, Ying & Leskovec 2017 — GraphSAGE (NeurIPS)

---

## 12. Cross-References

| Document | Relevance |
|---|---|
| `02-korai-passport.md` | Full passport struct with all fields |
| `03-passport-tiers.md` | Tier requirements and capabilities |
| `04-reputation-7-domain-ema.md` | Reputation scoring algorithm |
| `11-vickrey-reputation-auction.md` | How reputation affects auction scores |
| `13-isfr-clearing-settlement.md` | How validation feeds into ISFR |

---

*Generated from: refactoring-prd/04-knowledge-and-mesh.md, bardo-backup/prd/09-economy/00-identity.md,
bardo-backup/prd/shared/eip-analysis.md, tmp/implementation-plans/12b-chain-layer.md.
Naming renames applied per 01-naming-map.md. All golem→agent, GNOS→KORAI, clade→collective,
Styx→Agent Mesh renames applied.*
