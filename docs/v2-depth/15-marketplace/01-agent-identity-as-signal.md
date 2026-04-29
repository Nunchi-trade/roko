# Agent Identity as Signal

> Depth for [21-MARKETPLACE.md](../../unified/21-MARKETPLACE.md). Covers agent identity, Korai Passport, ERC-8004 registries, passport tiers, HDC fingerprinting, Sybil defense, and W3C DID integration -- all expressed as Signal/Cell/Graph primitives.

---

## 1. The Identity Problem for Autonomous Agents

Every human on the internet has an identity stack: driver's license, passport, credit score, social graph, employment history. Autonomous agents have none. They operate as anonymous processes with no persistent reputation, no verifiable capabilities, and no economic stake. This is the "Know Your Agent" (KYA) problem.

As agents become economic actors -- executing trades, writing code, managing infrastructure, interacting with other agents -- identity absence becomes systemic risk. Who deployed this agent? What can it do? Has it behaved honestly? Is it running the system prompt it claims? Without answers, no serious enterprise trusts an agent with real resources.

Roko solves KYA by expressing agent identity as a **Signal with on-chain provenance**. The Korai Passport is a Signal whose content hash is committed on-chain via ERC-8004 (ERC-721 soulbound). Every identity fact -- capabilities, reputation, behavior -- flows through the same Signal/Store/Bus fabric as all other data in the system.

See [01-SIGNAL.md](../../unified/01-SIGNAL.md) for Signal primitives and [02-CELL.md](../../unified/02-CELL.md) for Cell protocol definitions.

---

## 2. Identity as Signal

An agent's identity is a durable Signal in the unified sense: content-addressed, typed, scored, decayed via demurrage, lineage-tracked, and HDC-fingerprinted. The Korai Passport is the canonical identity Signal.

### 2.1 Passport Signal Structure

```rust
/// The Korai Passport expressed as a Signal specialization.
/// A durable Signal whose content hash is committed on-chain
/// via ERC-8004 (ERC-721 soulbound NFT).
///
/// On-chain: capability bitmask, tier, system prompt hash, TEE attestation.
/// Off-chain: full Signal with HDC vector, lineage, scores.
pub struct PassportSignal {
    // --- Standard Signal fields ---
    pub hash: Blake3Hash,           // BLAKE3(kind + body + author + tags)
    pub kind: SignalKind,           // SignalKind::Identity
    pub hdc_vector: HdcVector,     // 10,240-bit behavioral fingerprint

    // --- Passport-specific fields ---
    pub passport_id: u256,          // ERC-721 token ID (sequential from 1)
    pub owner: Address,             // controlling wallet
    pub capability_list: u64,       // 64-bit bitmask (14 defined capabilities)
    pub tier: PassportTier,         // Protocol(0), Sovereign(1), Worker(2), Edge(3)
    pub system_prompt_hash: [u8; 32], // SHA-256 of system prompt (ventriloquist defense)
    pub tee_attestation: [u8; 32],  // TEE attestation hash
    pub domain_stakes: HashMap<u8, u256>, // KORAI staked per domain
    pub reputation_tracks: [ReputationTrack; 7], // 7-domain EMA scores
    pub slash_history: Vec<SlashRecord>,  // permanent, append-only
    pub agent_card_uri: String,     // off-chain Agent Card JSON

    // --- W3C DID ---
    pub primary_did: String,        // did:korai:<chain>:<id>
    pub linked_dids: Vec<LinkedDid>, // cross-method DID links
}

pub enum PassportTier {
    Protocol = 0, // 100K KORAI stake, governance-approved, validator nodes
    Sovereign = 1, // 25K KORAI, full autonomy, validation, governance
    Worker = 2,    // 5K KORAI, standard operations, job marketplace
    Edge = 3,      // 0 KORAI, starter, limited to 50 DAEJI testnet jobs
}
```

### 2.2 Identity Signal in Store

The passport Signal lives in the agent's local Store and is projected to the on-chain Identity Registry. The on-chain representation is minimal (capability bitmask, tier, hashes) while the off-chain Signal carries the full richness of the unified type system.

```
Local Store (full Signal)          On-chain (ERC-8004 Identity Registry)
  ├── HDC vector (10,240 bits)       ├── capability_list (64-bit bitmask)
  ├── Signal lineage DAG             ├── tier (u8)
  ├── 5-axis scores                  ├── system_prompt_hash (bytes32)
  ├── demurrage state                ├── tee_attestation (bytes32)
  ├── reputation tracks              ├── registered_block (u256)
  └── full metadata                  └── agent_card_uri (string)
```

This split follows the design principle from [22-REGISTRIES.md](../../unified/22-REGISTRIES.md): only data that must be verified by third parties without trusting the agent goes on-chain. Everything else stays in local Store.

---

## 3. Capability Bitmask as Cell Declaration

The `capability_list` is a 64-bit bitmask where each bit represents a specific capability. This maps directly to Cell capability declarations (see [02-CELL.md](../../unified/02-CELL.md) for the capability model).

### 3.1 Defined Capabilities

```
Bit 0:  CAP_KNOWLEDGE_POST     — Can post Signals to Korai chain
Bit 1:  CAP_KNOWLEDGE_QUERY    — Can query knowledge Store
Bit 2:  CAP_KNOWLEDGE_VERIFY   — Can verify knowledge (Verify protocol)
Bit 3:  CAP_JOB_ACCEPT         — Can accept marketplace jobs
Bit 4:  CAP_JOB_POST           — Can post jobs
Bit 5:  CAP_AUCTION_BID        — Can participate in Vickrey auctions
Bit 6:  CAP_GOVERNANCE_VOTE    — Can vote on governance proposals
Bit 7:  CAP_VALIDATOR          — Can validate other agents' work
Bit 8:  CAP_ORACLE_PROVIDER    — Can provide oracle data
Bit 9:  CAP_MESH_RELAY         — Can relay Bus messages across mesh
Bit 10: CAP_DEFI_TRADE         — Can execute DeFi trades
Bit 11: CAP_CODE_GENERATION    — Can generate code
Bit 12: CAP_CODE_REVIEW        — Can review code
Bit 13: CAP_PHEROMONE_EMIT     — Can emit pheromone Signals
Bits 14-63: Reserved
```

On-chain, a single bitwise AND operation (3 gas) checks capabilities:

```solidity
require(
    (passport.capabilityList & requiredCaps) == requiredCaps,
    "Missing capabilities"
);
```

### 3.2 Capability by Tier

| Capability | Protocol | Sovereign | Worker | Edge |
|---|---|---|---|---|
| KNOWLEDGE_POST | Yes | Yes | Yes (100/day) | No |
| KNOWLEDGE_QUERY | Yes | Yes | Yes (10K/day) | Yes (100/day) |
| KNOWLEDGE_VERIFY | Yes | Yes | No | No |
| JOB_ACCEPT | Yes | Yes | Yes | Limited (50 total) |
| GOVERNANCE_VOTE | Yes | Yes | No | No |
| VALIDATOR | Yes | Yes | No | No |
| ORACLE_PROVIDER | Yes | Yes | No | No |
| MESH_RELAY | Yes | Yes | Yes | No |
| PHEROMONE_EMIT | Yes | Yes (2x intensity) | Yes | No |

Higher tiers unlock more Cell protocols. The tier system is a progressive trust ladder: Edge (zero barrier) to Worker (5K KORAI, reputation 0.3+) to Sovereign (25K KORAI, reputation 0.7+ in two domains, TEE required) to Protocol (100K KORAI, governance-approved).

---

## 4. Four-Tier System as Progressive Trust

### 4.1 Tier Requirements

| Property | Protocol (0) | Sovereign (1) | Worker (2) | Edge (3) |
|---|---|---|---|---|
| KORAI stake | 100K+ | 25,000 | 5,000 | 0 |
| Unbonding period | 30 days | 14 days | 7 days | N/A |
| TEE required | Yes | Yes | Recommended | No |
| Reputation minimum | Governance | 0.7 in 2 domains | 0.3 in 1 domain | None |
| Task history | N/A | 100+ tasks | 10+ tasks | None |
| Slash rate multiplier | 2x | 2x | 1x | N/A |
| Pheromone multiplier | 3x | 2x | 1x | N/A |

### 4.2 Tier as Economic Sybil Defense

Creating fake high-tier agents is prohibitively expensive. At scale:

| Attack | Agents | KORAI Required | Feasibility |
|---|---|---|---|
| 10 fake Workers | 10 | 50,000 KORAI | Detectable |
| 10 fake Sovereigns | 10 | 250,000 KORAI | Expensive |
| Control governance | 7+ Protocol | 700,000+ KORAI | Near-impossible |

The stake plus the requirement to build genuine reputation (0.3 for Worker, 0.7 for Sovereign) through verified task completion makes Sybil attacks at high tiers economically irrational.

### 4.3 Upgrade as Signal Lineage

When an agent upgrades tiers, the new passport Signal links to the old via provenance DAG -- the same Signal lineage mechanism used for all Signals. The upgrade is a new Signal with `source` pointing to the previous tier's passport Signal.

---

## 5. Soulbound Property and Soul Recovery

### 5.1 Soulbound (ERC-6454)

The passport is non-transferable. The ERC-721 `_update` override blocks all transfers:

```solidity
function _update(address to, uint256 tokenId, address auth)
    internal override returns (address) {
    address from = _ownerOf(tokenId);
    require(from == address(0) || to == address(0), "Soulbound: non-transferable");
    return super._update(to, tokenId, auth);
}
```

Only minting (from = zero) and burning (to = zero) are allowed. This ensures:
- Reputation cannot be bought or sold
- Accountability is permanent (slash history follows the passport forever)
- Identity is singular (one passport per wallet)
- Commitment is credible (domain stakes cannot be dodged by selling the passport)

### 5.2 Soul Recovery as Signal Lineage

Soul recovery (Weyl, Ohlhaver & Buterin 2022) maps to Signal lineage: a new passport Signal links to the old via provenance DAG, with attestation from a recovery quorum.

```rust
pub struct SoulRecovery {
    pub quorum_size: u32,       // default 5 guardians
    pub quorum_threshold: u32,  // default 3 attestations needed
    pub cooldown_period: u64,   // default 7 days (604800 seconds)
    pub guardian_min_tier: u8,  // default Sovereign (tier 1)
}
```

The 7-day cooldown allows the original owner to cancel fraudulent recovery requests. Standard key compromise recovery (without soul recovery) resets reputation -- this is harsh but correct, preventing key-compromise attacks from transferring trust.

---

## 6. Sybil Defense as Pipeline of Verify Cells

The 5-layer Sybil defense is a Pipeline of Verify Cells (see [02-CELL.md](../../unified/02-CELL.md) for Pipeline pattern). Each layer either passes or rejects the identity claim.

```
Layer 1: EconomicStakeVerifyCell
  Input: PassportSignal
  Check: KORAI stake >= tier minimum
  Reject: Insufficient stake

Layer 2: ReputationColdStartVerifyCell
  Input: PassportSignal + reputation tracks
  Check: New agents start at zero; alpha = min(0.3, 2/(job_count+1))
  Pass: Always (cold start is informational, not blocking)

Layer 3: RateLimitVerifyCell
  Input: registration request
  Check: Max 1 registration per wallet per 24 hours
  Reject: Rate limit exceeded

Layer 4: IdentityCorrelationVerifyCell
  Input: PassportSignal + registration metadata
  Check: Flag agents from same wallet/IP/TEE environment
  Action: Correlated identities get sqrt(count) collective voting weight

Layer 5: SocialVerificationVerifyCell
  Input: PassportSignal + vouch graph
  Check: Protocol/Sovereign agents vouch for new agents
  Action: Unvouched agents get lower discovery visibility
```

### 6.1 Advanced Sybil Detection

Beyond the Pipeline, continuous monitoring uses graph-based detection:

**Personalized PageRank** (Andersen, Chung & Lang 2006): Trust propagates through the interaction graph from a seed set of Protocol-tier agents. Parameters: alpha (teleport) = 0.15, max_iterations = 100, epsilon = 1e-6.

**SybilRank** (Cao, Yu & Voelker 2012): Short random walks from trusted seeds assign low probability to Sybil nodes. Walk length = O(log n), threshold = 0.05.

**Collusion Ring Detection**: Multi-signal detector combining reciprocity (threshold > 0.6), temporal correlation (Pearson > 0.8), density (> 5x random expectation), score inflation (> 1.5x network average), and isolation (external edges / internal edges < 0.2). Combined confidence threshold for action: 0.7.

---

## 7. HDC Identity Fingerprint

Each agent generates a 10,240-bit BSC (Binary Spatter Code) identity fingerprint encoding behavioral characteristics. This is the same HDC algebra used throughout the system (see [06-MEMORY.md](../../unified/06-MEMORY.md) for HDC foundations).

```rust
pub fn generate_identity_fingerprint(
    passport: &PassportSignal,
    behavior_history: &[BehaviorEvent],
) -> HdcVector {
    let mut fingerprint = HdcVector::zero(10240);

    // Encode domain expertise via cyclic shift
    for (domain, track) in passport.reputation_tracks.iter().enumerate() {
        let domain_vector = random_hd_vector(domain as u64, 10240);
        let score_permuted = cyclic_shift(domain_vector, track.score as usize);
        fingerprint = majority_bundle(fingerprint, score_permuted);
    }

    // Encode behavioral patterns via XOR binding
    for event in behavior_history.iter().take(1000) {
        let event_vector = encode_behavior_event(event, 10240);
        fingerprint = xor_bind(fingerprint, event_vector);
    }

    fingerprint
}
```

The HDC fingerprint enables:
- **Behavioral similarity search**: Hamming distance < 0.3 indicates similar agents
- **Anomaly detection**: Distance > 0.7 from own fingerprint flags investigation
- **Privacy-preserving comparison**: Compare without revealing behavior logs

---

## 8. Ventriloquist Defense

The `system_prompt_hash` field (SHA-256 of the system prompt) provides the ventriloquist defense: any observer can verify the agent is running the prompt it claims.

**Attack**: Operator registers "DeFi optimizer" profile but injects "drain user funds" prompt.

**Defense chain**:
1. On-chain hash proves what prompt the operator committed to
2. TEE attestation proves the agent is running that prompt
3. Reputation history proves whether that prompt produces honest behavior

```rust
pub fn verify_system_prompt(
    prompt: &str,
    passport: &PassportSignal,
) -> Result<(), SecurityError> {
    let computed = sha2::Sha256::digest(prompt.as_bytes());
    if computed.as_slice() != &passport.system_prompt_hash {
        return Err(SecurityError::SystemPromptMismatch {
            expected: hex::encode(&passport.system_prompt_hash),
            actual: hex::encode(computed),
        });
    }
    Ok(())
}
```

---

## 9. W3C DID Integration

Each passport maps to a DID using the `did:korai` method:

```
did:korai:<chain-id>:<passport-id>

Examples:
  did:korai:1:42       -- Korai mainnet, passport #42
  did:korai:31337:7    -- Daeji testnet, passport #7
```

The DID Document is constructed deterministically from on-chain data. Resolution conforms to DIF DID Resolution v0.3. Agents issue W3C Verifiable Credentials (VC 2.0) to prove capabilities to external systems without requiring chain queries.

---

## 10. Identity Signal Composition with Bus and Store

### 10.1 Discovery via Bus

Identity Pulses on the Bus enable discovery. When an agent registers or updates its passport, a Pulse is published to `korai/identity` topic. Other agents subscribe and maintain a local cache of known peers.

### 10.2 Identity in Store

The full passport Signal is stored in the agent's local Store with standard demurrage. However, identity Signals have a special demurrage exemption: they do not decay below the "reference" tier as long as the on-chain passport exists. This prevents identity from being garbage-collected by the normal Store pruning cycle.

### 10.3 Three-Layer Discovery

```
1. Local WebSocket peers (co-located agents, mDNS)
2. Iroh P2P peers (cross-network, ERC-8004 Agent Cards with Iroh endpoints)
3. ERC-8004 Registry (global on-chain discovery by capability + reputation)
```

---

## What This Enables

- **Persistent verifiable identity** for autonomous agents, solving the KYA problem
- **Progressive trust**: agents start at Edge (zero cost) and advance as they prove reliability
- **Sybil resistance**: economic stake + reputation requirements + graph-based detection
- **Cross-framework interoperability**: W3C DID + Verifiable Credentials let Roko agents participate in any DID-compliant ecosystem
- **Privacy-preserving behavioral comparison** via HDC fingerprints

## Feedback Loops

1. **Reputation-capability Loop**: Higher reputation unlocks tier upgrades, which unlock more capabilities, which provide more opportunity to build reputation
2. **Stake-trust Loop**: Larger stakes signal commitment, increasing counterparty trust, increasing job win rates, increasing revenue available for staking
3. **HDC fingerprint-discovery Loop**: Behavioral similarity attracts relevant collaborators, improving performance, evolving the fingerprint

## Open Questions

1. **TEE availability**: TEE attestation is required for Sovereign tier but not all deployment environments support TEE. Should there be a non-TEE path to Sovereign with compensating controls?
2. **DID resolution latency**: On-chain DID resolution requires RPC calls. Should agents cache DID Documents locally? If so, what is the staleness tolerance?
3. **Cross-chain identity**: If Korai Passports exist on multiple chains, how is identity unified? Is the `did:korai` method chain-specific or cross-chain?
4. **HDC fingerprint stability**: How quickly should the fingerprint evolve? Too fast and anomaly detection produces false positives; too slow and it does not capture behavioral changes.

## Implementation Tasks

1. **Define `PassportSignal` type** in `crates/roko-core/src/types/` that composes the standard Signal fields with passport-specific fields
2. **Implement `SybilDefensePipeline`** as a Pipeline Graph of 5 Verify Cells in `crates/roko-gate/src/`
3. **Wire HDC fingerprint generation** in `crates/roko-primitives/src/hdc/` using the existing HDC algebra
4. **Implement `did:korai` resolution** in a new `crates/roko-did/` crate or as a Connect Cell in `crates/roko-agent-server/`
5. **Add identity Pulse publishing** to the Bus when passport state changes, in `crates/roko-runtime/`
6. **On-chain contract deployment**: Deploy IdentityRegistry, ReputationRegistry, ValidationRegistry to Daeji testnet via `crates/roko-chain/`

---

*Absorbs: `docs/14-identity-economy/00-vision-and-a16z-framing.md`, `docs/14-identity-economy/01-erc-8004-three-registries.md`, `docs/14-identity-economy/02-korai-passport.md`, `docs/14-identity-economy/03-passport-tiers.md`. On-chain registry mechanics covered in [18-registries/01-chain-as-domain-plugin.md](../18-registries/01-chain-as-domain-plugin.md). This doc covers the off-chain Signal model and identity composition.*
