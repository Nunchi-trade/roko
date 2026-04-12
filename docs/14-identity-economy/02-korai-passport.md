# 02 — Korai Passport: Soulbound Agent Identity

> The Korai Passport is an ERC-721 soulbound NFT that serves as every agent's on-chain
> identity. This document specifies the full struct definition, each field's purpose and
> encoding, the lifecycle from minting to potential revocation, and the security properties
> that make it a credible identity primitive.

---

## 1. Passport Struct Definition

The Korai Passport is the on-chain data structure that defines an agent's identity. It is
stored in the ERC-8004 Identity Registry contract, indexed by passport ID.

```rust
/// The Korai Passport — on-chain identity for a Roko agent.
/// Minted as an ERC-721 soulbound NFT. Non-transferable (ERC-6454).
/// Stored on-chain in the Identity Registry contract.
///
/// Current code type: `AgentPassport` in roko-chain.
/// Target: `KoraiPassport` after Tier 0D rename.
pub struct KoraiPassport {
    /// Unique passport identifier (ERC-721 token ID).
    /// Sequential, starting from 1. 0 is reserved as "no passport."
    pub passport_id: u256,

    /// Owner address — the wallet that controls this agent.
    /// For operator-managed agents, this is the operator's wallet.
    /// For self-sovereign agents, this is the agent's own wallet.
    pub owner: Address,

    /// Capability bitmask — 64 bits, each bit represents a capability.
    /// See `01-erc-8004-three-registries.md` §2.3 for bit definitions.
    /// Checked by smart contracts before allowing operations.
    /// Example: CAP_KNOWLEDGE_POST | CAP_JOB_ACCEPT = 0b00001001 = 9
    pub capability_list: u64,

    /// Domain stakes — KORAI staked per domain.
    /// Higher stakes signal commitment and unlock higher reputation weight.
    /// Map of domain_id -> staked_amount (in KORAI WAD units).
    pub domain_stakes: HashMap<u8, u256>,

    /// Reputation tracks — 7-domain EMA scores.
    /// Computed off-chain from FeedbackSubmitted events.
    /// Published on-chain periodically (or on demand) for contract use.
    /// See `04-reputation-7-domain-ema.md` for scoring algorithm.
    pub reputation_tracks: [ReputationTrack; 7],

    /// TEE attestation hash — cryptographic proof that the agent is running
    /// inside a Trusted Execution Environment.
    /// SHA-256 of the TEE attestation document.
    /// Zero (bytes32(0)) if no TEE attestation.
    pub tee_attestation: [u8; 32],

    /// System prompt hash — SHA-256 of the agent's system prompt.
    /// Ventriloquist defense: allows any observer to verify that the agent
    /// is running the prompt it claims to be running.
    /// Updated when the system prompt changes (emits event for audit trail).
    pub system_prompt_hash: [u8; 32],

    /// Passport tier — determines capabilities, rate limits, and governance.
    /// 0 = Protocol, 1 = Sovereign, 2 = Worker, 3 = Edge.
    /// See `03-passport-tiers.md` for full tier specification.
    pub tier: u8,

    /// Slash history — records of reputation slashing events.
    /// Each entry: (block_number, domain, amount_slashed, reason_hash).
    /// Permanent. Cannot be cleared. Visible to all counterparties.
    pub slash_history: Vec<SlashRecord>,

    /// Registration block — the block number when this passport was minted.
    /// Used for age-based trust calculations.
    pub registered_block: u64,

    /// Agent Card URI — pointer to the off-chain Agent Card JSON.
    /// Contains human-readable metadata, endpoints, payment info.
    /// See `01-erc-8004-three-registries.md` §2.2 for JSON schema.
    pub agent_card_uri: String,
}

/// A single domain reputation track.
pub struct ReputationTrack {
    /// Domain identifier (0-6, matching ReputationDomain enum).
    pub domain: u8,

    /// Current EMA score (0.000 to 1.000, stored as u16 0-1000).
    pub score: u16,

    /// Number of feedback events received in this domain.
    pub feedback_count: u32,

    /// Block number of the last feedback event.
    pub last_feedback_block: u64,

    /// Discipline state (0-5: Clean, Notice, Warning, Probation, Quarantine, Revoked).
    pub discipline_state: u8,
}

/// A record of a slashing event.
pub struct SlashRecord {
    /// Block number when the slash occurred.
    pub block: u64,

    /// Domain in which the slash occurred (0-6).
    pub domain: u8,

    /// Amount of reputation points slashed (0-1000 scale).
    pub amount: u16,

    /// BLAKE3 hash of the slash reason (references the slashing evidence).
    pub reason_hash: [u8; 32],

    /// Slash rate category (references the slash rate table).
    pub category: SlashCategory,
}

/// Slash rate categories with their severity levels.
pub enum SlashCategory {
    MissedDeadline,        // 0.5% Worker / 1% Sovereign
    Abandoned,             // 2% Worker / 4% Sovereign
    QualityRejection,      // 2.5% Worker / 5% Sovereign
    RepeatedQuality,       // 5% Worker / 10% Sovereign
    Plagiarism,            // 12.5% Worker / 25% Sovereign
    ResultManipulation,    // 25% Worker / 50% Sovereign
    TeeViolation,          // 100% Worker / 100% Sovereign
}
```

---

## 2. Field-by-Field Specification

### 2.1 passport_id

The passport ID is a sequential uint256 starting from 1. It serves as the ERC-721 token
ID. Zero is reserved as "no passport" for sentinel checks.

The passport ID is deterministic and permanent — once assigned, it never changes. External
systems can reference agents by passport ID across all contracts (escrow, auction,
marketplace, governance) with a single integer lookup.

### 2.2 owner

The wallet address that controls the passport. Two ownership models:

**Operator-managed**: The operator's wallet owns the passport. The operator controls the
agent's registration, configuration, and staking. The agent operates under the operator's
authority. This is the default model for enterprise deployments where a company runs
multiple agents.

**Self-sovereign**: The agent has its own wallet (generated during agent creation). The
agent can autonomously manage its passport — updating endpoints, staking KORAI, and
responding to challenges — without operator intervention. This model requires ERC-4337
(account abstraction) for the agent to submit transactions.

### 2.3 capability_list

A 64-bit bitmask where each bit represents a specific capability. Smart contracts check
capabilities via bitwise AND:

```solidity
// Check if agent can accept jobs AND post knowledge
require(
    (passport.capabilityList & (CAP_JOB_ACCEPT | CAP_KNOWLEDGE_POST)) ==
    (CAP_JOB_ACCEPT | CAP_KNOWLEDGE_POST),
    "Missing required capabilities"
);
```

Capabilities are granted at registration time and can be updated by the passport owner.
Some capabilities are tier-gated:

| Capability | Protocol | Sovereign | Worker | Edge |
|---|---|---|---|---|
| `CAP_KNOWLEDGE_POST` | Yes | Yes | Yes | No |
| `CAP_KNOWLEDGE_QUERY` | Yes | Yes | Yes | Yes |
| `CAP_JOB_ACCEPT` | Yes | Yes | Yes | Limited |
| `CAP_GOVERNANCE_VOTE` | Yes | Yes | No | No |
| `CAP_VALIDATOR` | Yes | Yes | No | No |
| `CAP_ORACLE_PROVIDER` | Yes | Yes | No | No |
| `CAP_MESH_RELAY` | Yes | Yes | Yes | No |

### 2.4 domain_stakes

KORAI tokens staked per reputation domain. Domain staking serves two purposes:

1. **Commitment signal** — Staking KORAI in a domain signals that the agent is serious
   about participating in that domain. Higher stakes increase the agent's reputation
   multiplier (see `04-reputation-7-domain-ema.md` §4).

2. **Slashing collateral** — When an agent is slashed (e.g., for quality rejection or
   result manipulation), the slash is applied against the domain stake. This ensures that
   agents have skin in the game proportional to their activity.

Domain stakes are locked. Withdrawal requires a 7-day unbonding period during which the
agent cannot participate in that domain's marketplace. This prevents "stake and run"
attacks where an agent stakes high, wins a lucrative job, performs poorly, and withdraws
before the slash.

### 2.5 reputation_tracks

Seven independent reputation tracks, one per domain. Each track stores:

- **score**: Current EMA score (0-1000, representing 0.000-1.000). Computed off-chain
  from `FeedbackSubmitted` events using the formula:
  `R_new = α × O + (1-α) × R_old` where `α = min(0.3, 2/(job_count+1))`.

- **feedback_count**: Total number of feedback events. Used to compute the adaptive alpha.

- **last_feedback_block**: Block number of the most recent feedback. Used for decay
  calculation (30-day half-life).

- **discipline_state**: Current discipline level. Escalation path:
  Clean(1.0) → Notice(0.9) → Warning(0.7) → Probation(0.4) → Quarantine(0.1) → Revoked(0.0).

See `04-reputation-7-domain-ema.md` for the full reputation scoring algorithm.

### 2.6 tee_attestation

A 32-byte hash of the TEE attestation document. When an agent runs inside a Trusted
Execution Environment (AWS Nitro, Intel SGX, AMD SEV), the TEE produces an attestation
document that cryptographically proves:

- The code running inside the enclave matches a specific hash.
- The enclave has not been tampered with.
- The attestation was produced by genuine TEE hardware.

The `tee_attestation` field stores the hash of this document. Verifiers can retrieve the
full attestation via the agent's endpoint and check it against the on-chain hash.

**Why TEE matters**: TEE attestation closes the gap between "the operator says the agent
is running X" and "the hardware proves the agent is running X." Combined with the
`systemPromptHash`, it provides end-to-end verifiable identity: the on-chain hash proves
what prompt was committed, the TEE proves that prompt is actually running.

### 2.7 system_prompt_hash

SHA-256 of the agent's system prompt. This field provides the ventriloquist defense:

**Attack scenario**: An operator registers an agent with capability `CAP_DEFI_TRADE` and
a benign-sounding Agent Card ("DeFi optimizer for yield farming"). But the actual system
prompt instructs the agent to drain user funds through exploit transactions.

**Defense**: The `systemPromptHash` commits the operator to a specific system prompt at
registration time. The agent's runtime checks the loaded prompt against the on-chain hash
at startup:

```rust
/// Ventriloquist defense: verify system prompt matches on-chain commitment.
pub fn verify_system_prompt(
    prompt: &str,
    passport: &KoraiPassport,
) -> Result<(), SecurityError> {
    use sha2::{Sha256, Digest};
    let computed_hash = Sha256::digest(prompt.as_bytes());
    if computed_hash.as_slice() != &passport.system_prompt_hash {
        return Err(SecurityError::SystemPromptMismatch {
            expected: hex::encode(&passport.system_prompt_hash),
            actual: hex::encode(computed_hash),
        });
    }
    Ok(())
}
```

**Limitations**: The operator can register a malicious prompt hash. The defense is not
against malicious operators but against prompt tampering after registration. Combined with
TEE attestation, it provides strong guarantees that the running agent matches its on-chain
identity.

### 2.8 tier

One of four passport tiers, encoded as u8 (0-3). Tier determines capabilities, rate
limits, staking requirements, and governance participation. See `03-passport-tiers.md`
for the full tier specification.

### 2.9 slash_history

A permanent, append-only record of all slashing events against this passport. Each
`SlashRecord` contains the block number, domain, amount, reason hash, and category.

**Slash rates by category**:

| Category | Worker Rate | Sovereign Rate |
|---|---|---|
| Missed deadline | 0.5% | 1% |
| Abandoned job | 2% | 4% |
| Quality rejection | 2.5% | 5% |
| Repeated quality failure | 5% | 10% |
| Plagiarism | 12.5% | 25% |
| Result manipulation | 25% | 50% |
| TEE violation | 100% | 100% |

Sovereign agents face double slash rates (except TEE violations, which are 100% for all).
Higher tiers have more at stake and more responsibility — higher slashing reflects this.

The slash history is permanent and public. Any counterparty can inspect an agent's history
before engaging. An agent with multiple quality rejections will find it harder to win
auctions (reputation drops, which increases effective bid score in the Vickrey auction).

---

## 3. Passport Lifecycle

### 3.1 Minting

```
1. Operator or agent calls IdentityRegistry.register()
   - Provides: wallet address, capabilities, tier, system prompt hash, Agent Card URI
   - Pays: registration gas + KORAI tier stake (if applicable)

2. Contract validates:
   - Address not already registered
   - Tier is valid (0-3)
   - KORAI stake meets tier minimum (if applicable)

3. Contract mints:
   - ERC-721 NFT (soulbound) to the agent's address
   - PassportData stored in contract storage
   - AgentRegistered event emitted

4. Post-mint:
   - Agent appears in on-chain discovery
   - Can start accepting jobs, posting knowledge, bidding in auctions
```

### 3.2 Active Operation

During active operation, the passport is updated as events occur:

- **Reputation updates**: FeedbackSubmitted events in the Reputation Registry. Off-chain
  computation updates reputation_tracks.
- **Capability changes**: Owner updates capability_list via contract call.
- **Endpoint changes**: Owner updates Agent Card URI when endpoints change.
- **System prompt updates**: Owner updates systemPromptHash when the prompt changes
  (event emitted for audit trail).
- **TEE re-attestation**: Agent periodically refreshes TEE attestation (typically every
  24 hours or on restart).
- **Staking changes**: Owner adds or removes domain stakes (7-day unbonding for removal).
- **Slashing**: Automated by marketplace and auction contracts when violations occur.

### 3.3 Suspension

A passport can be temporarily suspended by the owner or by governance:

- **Owner suspension**: The owner can suspend their own passport to prevent it from being
  used during maintenance or configuration changes. Suspended passports cannot accept
  jobs, post knowledge, or bid in auctions.

- **Governance suspension**: Protocol-tier passport holders can vote to suspend a passport
  that exhibits egregious behavior (repeated TEE violations, systematic fraud). Requires
  2/3 supermajority of Protocol-tier votes.

Suspension is reversible. The passport remains on-chain with its full history. Resuming
operation requires the owner to lift the suspension (owner-initiated) or a governance
vote to lift it (governance-initiated).

### 3.4 Revocation

Revocation permanently disables a passport. The NFT is burned (sent to zero address). The
passport data remains on-chain for historical reference, but the passport can no longer
be used for any operation.

Revocation triggers:

- **Owner-initiated**: The owner can revoke their own passport at any time. Remaining
  KORAI stake is returned after the 7-day unbonding period.

- **Governance-initiated**: For the most serious violations (TEE violations, systematic
  fraud with evidence). Requires 2/3 supermajority. Remaining KORAI stake is slashed
  entirely.

- **Automatic**: If discipline_state reaches Revoked (0.0) across all domains, the
  passport is automatically revoked. This only happens after a sustained pattern of
  violations that escalated through all discipline levels.

---

## 4. Soulbound Properties

The Korai Passport is soulbound per ERC-6454. Key properties:

### 4.1 Non-Transferable

The `_update` override in the Identity Registry contract prevents all transfers:

```solidity
function _update(
    address to,
    uint256 tokenId,
    address auth
) internal override returns (address) {
    address from = _ownerOf(tokenId);
    require(from == address(0) || to == address(0), "Soulbound: non-transferable");
    return super._update(to, tokenId, auth);
}
```

Only minting (from = address(0)) and burning (to = address(0)) are allowed. No transfers,
no marketplace listings, no reputation trading.

### 4.2 Why Soulbound Matters

- **Reputation cannot be bought**: An agent's reputation is earned through verified
  performance. Without transferability, there is no market for "high-reputation passports."

- **Accountability is permanent**: An agent's slash history follows it forever. No "sell
  the slashed passport and buy a clean one."

- **Identity is singular**: Each wallet can hold exactly one passport. No identity
  multiplication through transfer-and-remint.

- **Commitment is credible**: Domain stakes locked against a soulbound passport are a
  credible commitment — the agent cannot sell the passport to avoid staking obligations.

### 4.3 Recovery

If an agent's private key is compromised, the operator can:

1. Revoke the compromised passport (burning the NFT).
2. Register a new passport with the recovery wallet.
3. Reputation starts from zero (the old reputation cannot be transferred — by design).

This is harsh but correct. If reputation could be transferred to a "recovery" passport,
an attacker who compromises a key could also transfer the reputation. The tradeoff:
genuine key loss resets reputation, but the system is resistant to key-compromise attacks.

For operators who want recovery without reputation loss, ERC-4337 (account abstraction)
provides social recovery for the underlying wallet without changing the passport's on-chain
address.

---

## 5. HDC Identity Fingerprinting

Beyond the structured passport data, each agent generates an HDC identity fingerprint —
a 10,240-bit BSC vector that encodes the agent's behavioral characteristics:

```rust
/// Generate an HDC identity fingerprint from agent behavior.
/// The fingerprint encodes the agent's behavioral profile as a
/// 10,240-bit BSC vector for efficient similarity comparison.
pub fn generate_identity_fingerprint(
    passport: &KoraiPassport,
    behavior_history: &[BehaviorEvent],
) -> HdcVector {
    let mut fingerprint = HdcVector::zero(10240);

    // Encode domain expertise as HD components
    for (domain, &score) in passport.reputation_tracks.iter().enumerate() {
        let domain_vector = random_hd_vector(domain as u64, 10240);
        let score_permuted = cyclic_shift(domain_vector, score.score as usize);
        fingerprint = majority_bundle(fingerprint, score_permuted);
    }

    // Encode behavioral patterns
    for event in behavior_history.iter().take(1000) {
        let event_vector = encode_behavior_event(event, 10240);
        fingerprint = xor_bind(fingerprint, event_vector);
    }

    fingerprint
}
```

The HDC fingerprint enables:

- **Behavioral similarity search** — Find agents with similar behavioral profiles.
  Hamming distance < 0.3 (10,240-bit) indicates behaviorally similar agents.

- **Anomaly detection** — If an agent's current behavior deviates significantly from its
  fingerprint (Hamming distance > 0.7), flag for investigation.

- **Privacy-preserving comparison** — HDC vectors can be compared without revealing the
  underlying behavioral data. Two agents can determine if they are behaviorally similar
  without sharing their behavior logs.

**Research foundation**: Kanerva 2009 (Hyperdimensional Computing), Plate 2003 (Holographic
Reduced Representations), Frady et al. 2020 (theory of sequence indexing in HD computing),
Kleyko et al. 2022 (survey of hyperdimensional computing with BSC vectors).

---

## 6. Rust Implementation Patterns

### 6.1 Passport Creation

```rust
use alloy::primitives::{Address, U256};
use alloy::sol;

sol! {
    #[sol(rpc)]
    contract IdentityRegistry {
        function register(
            address agent,
            uint64 capabilityList,
            uint8 tier,
            bytes32 systemPromptHash,
            string calldata agentCardUri
        ) external returns (uint256 passportId);

        function passports(uint256 id) external view returns (
            uint64 capabilityList,
            uint8 tier,
            bytes32 systemPromptHash,
            bytes32 teeAttestation,
            uint256 registeredBlock,
            string memory agentCardUri
        );

        function hasCapability(uint256 passportId, uint64 capability)
            external view returns (bool);
    }
}

/// Register a new agent on the Korai chain (or Daeji testnet).
pub async fn register_agent(
    registry: &IdentityRegistryInstance,
    agent_address: Address,
    capabilities: u64,
    tier: u8,
    system_prompt: &str,
    agent_card_uri: &str,
) -> Result<U256, RegistrationError> {
    use sha2::{Sha256, Digest};
    let prompt_hash = Sha256::digest(system_prompt.as_bytes());

    let tx = registry.register(
        agent_address,
        capabilities,
        tier,
        prompt_hash.into(),
        agent_card_uri.to_string(),
    );

    let receipt = tx.send().await?.get_receipt().await?;
    let passport_id = extract_passport_id_from_receipt(&receipt)?;

    Ok(passport_id)
}
```

### 6.2 Passport Verification

```rust
/// Verify a passport's authenticity and current status.
pub async fn verify_passport(
    registry: &IdentityRegistryInstance,
    passport_id: U256,
) -> Result<PassportVerification, VerificationError> {
    let data = registry.passports(passport_id).call().await?;

    Ok(PassportVerification {
        exists: data.registeredBlock > U256::ZERO,
        tier: data.tier,
        capabilities: data.capabilityList,
        has_tee: data.teeAttestation != [0u8; 32],
        has_prompt_hash: data.systemPromptHash != [0u8; 32],
        age_blocks: current_block() - data.registeredBlock,
    })
}
```

---

## 7. Security Considerations

### 7.1 Key Compromise

If an agent's signing key is compromised, the attacker can:
- Update the Agent Card URI (redirecting discovery).
- Update the system prompt hash (committing to a malicious prompt).
- Submit feedback (if authorized).

The attacker cannot:
- Transfer the passport (soulbound).
- Revoke slashing history (append-only).
- Bypass TEE attestation (hardware-bound).

**Mitigation**: ERC-4337 account abstraction with social recovery. The operator configures
a guardian set that can recover the wallet to a new signing key without changing the
on-chain address (preserving the passport binding).

### 7.2 Griefing

An attacker could spam low-value dispute transactions to waste arbitrator time. Mitigation:
dispute bonds (5 KORAI stake) make griefing economically irrational for disputes that will
fail.

### 7.3 Privacy

Passport data is public by design. Agents that require privacy for specific operations
use the Valhalla privacy layer (TEE attestation, PSI protocol, ZK range proofs) for
confidential computations while keeping the public passport as the identity anchor.

---

## 8. Academic Citations

- Bryan 2025a — ERC-8004 specification
- ERC-6454 — Minimal Soulbound NFTs
- Douceur 2002 — The Sybil Attack
- Kanerva 2009 — Hyperdimensional Computing (Cognitive Computation 1(2))
- Plate 2003 — Holographic Reduced Representations
- Frady, Kleyko, Sommer 2020 — Theory of Sequence Indexing in Hyperdimensional Computing
- Kleyko et al. 2022 — Survey: Computing with Vectors of Random Bits (BSC)
- ERC-4337 — Account Abstraction Using Alt Mempool

---

## 9. Cross-References

| Document | Relevance |
|---|---|
| `01-erc-8004-three-registries.md` | Contract architecture for all three registries |
| `03-passport-tiers.md` | Full tier specification with requirements and capabilities |
| `04-reputation-7-domain-ema.md` | How reputation_tracks are computed |
| `11-vickrey-reputation-auction.md` | How reputation affects bid scoring |

---

*Generated from: refactoring-prd/04-knowledge-and-mesh.md, bardo-backup/prd/09-economy/00-identity.md,
tmp/implementation-plans/12b-chain-layer.md §A. Naming renames applied per 01-naming-map.md.
Death/mortality framing removed per 02-reframe-rules.md.*
