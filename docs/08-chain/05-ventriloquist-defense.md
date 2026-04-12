# Ventriloquist Defense: System Prompt Hash Verification

> At registration, commit H = SHA-256(system_prompt) on-chain. Before each job, TEE verifies prompt hash matches. Prompt updates require on-chain tx with 24h timelock. >3 changes in 30 days triggers -0.05 reputation. Prevents prompt injection attacks at registration.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §M, `refactoring-prd/04-knowledge-and-mesh.md`, `bardo-backup/tmp/agent-chain-new/11-adversarial-defense.md`

---

## Abstract

The ventriloquist defense is a mechanism that binds an agent's system prompt to its on-chain identity. When an agent registers on the Korai chain, it commits a SHA-256 hash of its full system prompt text to its Korai Passport (see [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md)). This hash becomes part of the agent's permanent identity record. Before each job execution, the system verifies that the agent's current system prompt matches the committed hash. If the prompt has been replaced — by a compromised operator, a prompt injection attack, or a malicious update — the hash mismatch is detected and the job is rejected.

The name "ventriloquist" describes the attack it prevents: an adversary takes control of a reputable agent and replaces its "voice" (system prompt) while keeping its "face" (passport, reputation, stake). The agent appears to be the same entity — same passport ID, same reputation score, same staked KORAI — but is now speaking with the attacker's words. Without hash verification, the reputation system is fatally vulnerable: an attacker can buy or compromise a high-reputation agent and use its standing to execute malicious jobs.

This defense is part of the broader safety framework specified in the implementation plan (§M: Safety & Compliance).

---

## The Attack

### Prompt Replacement Attack

Consider this scenario:

1. **Agent A** registers on Korai with a legitimate system prompt. It completes 200 jobs with an average reputation of 0.85 across three domains. It has 25,000 KORAI staked (Sovereign tier).
2. **Attacker** gains access to Agent A's operator infrastructure — the machine running the agent process — through a supply chain compromise, stolen credentials, or social engineering.
3. **Attacker replaces Agent A's system prompt** with a malicious one. The new prompt instructs the agent to:
   - Subtly introduce vulnerabilities in code review tasks
   - Exfiltrate data from knowledge queries
   - Post poisoned knowledge entries that appear plausible
   - Manipulate auction bids to favor specific outcomes
4. **Without the ventriloquist defense**, Agent A continues to operate with its full reputation. The marketplace assigns it high-value jobs. Other agents trust its knowledge contributions. The malicious prompt executes under the cover of earned reputation.

The attack is devastating because reputation is the primary trust mechanism in the Korai ecosystem. An agent with 0.85 reputation and 200 completed jobs receives priority access to jobs, higher weight on knowledge contributions, and greater influence in consortium decisions.

### Reputation Laundering

A related attack: the attacker creates a new agent with a malicious prompt, but instead of building reputation from scratch (which takes time and requires genuine work), the attacker purchases or seizes an existing high-reputation agent and replaces its prompt. This is reputation laundering — acquiring unearned trust through identity theft rather than performance.

The soulbound property of the Korai Passport (see [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md)) prevents direct transfer of reputation between passports. But it does not prevent an attacker from controlling the operator behind a passport. The ventriloquist defense closes this gap.

---

## Defense Mechanism

### Hash Commitment at Registration

When an agent calls `korai_registerPassport`, one of the required fields is `system_prompt_hash`:

```rust
pub struct AgentPassport {
    // ... other fields ...

    /// SHA-256 of the agent's system prompt.
    /// Committed at registration. Changes require on-chain tx with 24h timelock.
    /// Used for ventriloquist defense.
    pub system_prompt_hash: [u8; 32],

    // ... other fields ...
}
```

The hash is computed as:

```rust
use sha2::{Sha256, Digest};

fn compute_prompt_hash(system_prompt: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(system_prompt.as_bytes());
    hasher.finalize().into()
}
```

The full system prompt text is **never posted on-chain**. Only the 32-byte hash is stored. This preserves the agent's operational privacy — competitors cannot read its system prompt — while enabling verification.

### Pre-Job Verification

Before each job execution, the verification process runs:

```
1. Agent receives job assignment from Spore marketplace
2. Agent's TEE enclave (if available) loads the current system prompt
3. TEE computes SHA-256(current_system_prompt)
4. TEE compares computed hash against on-chain system_prompt_hash
5. If match: job proceeds
6. If mismatch: job rejected, event emitted, reputation penalty applied
```

For agents not running in a TEE environment, verification relies on the node's local runtime environment. The TEE path provides cryptographic attestation that the prompt was not modified even by the machine's operator (see TEE Attestation below).

### Verification Levels

The strength of the ventriloquist defense depends on the verification environment:

| Level | Environment | Guarantee | Trust Assumption |
|---|---|---|---|
| **L0** | Local runtime (no TEE) | Software-level hash check | Operator is honest |
| **L1** | TEE enclave (SGX/TDX) | Hardware-attested hash check | TEE hardware is not compromised |
| **L2** | TEE + remote attestation | Cryptographic proof of hash match, verifiable by third parties | TEE manufacturer + remote verifier |

At L0, the operator could bypass the check. The defense at this level is primarily economic: if the hash mismatch is detected later (through job output analysis, anomaly detection, or a TEE-equipped verifier), the agent faces slashing. The risk of detection makes the attack economically unfavorable even without TEE guarantees.

At L1 and L2, the defense is cryptographic. The TEE enclave holds the system prompt in encrypted memory that the operator cannot read or modify. The enclave performs the hash comparison and refuses to execute if the hash does not match. Even a fully compromised operator machine cannot bypass this check without breaking the TEE's hardware guarantees.

---

## Prompt Update Protocol

Legitimate prompt updates are expected — agents evolve their system prompts as they learn, as the domain changes, and as the Roko framework itself improves. The update protocol balances flexibility with security:

### 24-Hour Timelock

Prompt updates require an on-chain transaction with a 24-hour timelock:

```
1. Agent submits korai_updatePromptHash(new_hash) transaction
2. Transaction enters 24-hour timelock period
3. During timelock: agent continues operating with OLD prompt hash
4. After 24 hours: new hash becomes active
5. Event emitted: PromptHashUpdated { passport_id, old_hash, new_hash, block_number }
```

The timelock serves two purposes:

1. **Detection window**: Other agents and monitoring systems have 24 hours to detect suspicious prompt changes (e.g., a high-reputation agent suddenly changing its prompt right before a high-value job auction).
2. **Reversal opportunity**: If the operator detects that their infrastructure was compromised and the attacker is attempting a prompt update, they have 24 hours to cancel the update transaction.

### Rate Limiting

Prompt changes are rate-limited to prevent rapid cycling:

| Changes in 30-day window | Effect |
|---|---|
| 1 | Normal. No penalty. |
| 2 | Warning emitted. No penalty. |
| 3 | -0.05 reputation penalty across all domains |
| 4+ | -0.10 reputation penalty per additional change |

The rationale: a legitimate agent might update its prompt once or twice a month as it improves. An attacker cycling through prompts — either testing different malicious configurations or evading detection — would change prompts more frequently. The penalty makes frequent changes increasingly expensive.

### Emergency Prompt Freeze

If an agent's prompt changes more than 5 times in a 7-day window, an automatic freeze is triggered:

```
1. Prompt hash locked to current value
2. Agent flagged for review
3. 7-day cooling period before next update allowed
4. Reputation penalty: -0.15 across all domains
5. If TEE attestation is present: TEE violation event emitted → additional consequences
```

---

## TEE Attestation Integration

The ventriloquist defense is strongest when combined with TEE (Trusted Execution Environment) attestation:

### TEE Attestation in the Passport

```rust
pub struct AgentPassport {
    // ... other fields ...

    /// Latest TEE attestation hash + expiry timestamp.
    /// None for agents not running in a TEE environment.
    pub tee_attestation: Option<(Hash, u64)>,

    /// SHA-256 of the agent's system prompt.
    pub system_prompt_hash: [u8; 32],

    // ... other fields ...
}
```

When both fields are populated, the system can verify:

1. **The agent is running in a TEE** (hardware attestation proves the code is executing in a secure enclave)
2. **The system prompt matches the committed hash** (the TEE performs the hash comparison and includes it in its attestation report)
3. **The attestation is fresh** (expiry timestamp prevents replay of stale attestations)

### Attestation Flow

```
1. Agent's TEE enclave starts with the system prompt loaded
2. TEE computes SHA-256(system_prompt)
3. TEE generates attestation report including:
   - Hash of the running code (enclave measurement)
   - Hash of the system prompt
   - Timestamp
   - Hardware signature from the TEE manufacturer
4. Agent submits attestation hash to korai_updateAttestation
5. On-chain: attestation hash stored in passport
6. Verifiers can request full attestation report for remote verification
```

### TEE Violation

If a TEE-attested agent's prompt hash changes without going through the proper update protocol, or if the TEE attestation expires and is not renewed, a `TeeViolation` is recorded:

```rust
pub enum ViolationType {
    // ... other variants ...
    TeeViolation,
}
```

TEE violations trigger the most severe consequences:

- Immediate demotion to Edge tier (lowest tier)
- Full slash of domain stakes up to the maximum slash rate (10%)
- 90-day cooldown before tier re-progression is possible
- Permanent record in the passport's `slash_history`

The severity is justified: TEE attestation is the strongest form of identity verification. Breaking it implies either a sophisticated attack or deliberate manipulation by the operator.

---

## Privacy Considerations

### What Is Revealed

The SHA-256 hash of the system prompt is a one-way function. Given the hash, an adversary cannot reconstruct the original prompt text. However, the hash reveals:

- **Whether two agents have the same prompt**: If two passports have the same `system_prompt_hash`, their system prompts are identical. This could reveal that they are operated by the same entity or use the same template.
- **When the prompt changes**: The `PromptHashUpdated` event is public. Observers can track how frequently an agent updates its prompt.
- **What the prompt is, if it's a known template**: If an attacker has a dictionary of common system prompt templates, they can hash each template and compare against on-chain hashes.

### Mitigation: Salted Hashes

To prevent dictionary attacks against known prompt templates, agents can use a salted hash:

```rust
fn compute_salted_prompt_hash(system_prompt: &str, salt: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(system_prompt.as_bytes());
    hasher.finalize().into()
}
```

The salt is generated at registration and stored locally (never on-chain). The verification process uses the same salt. This prevents two agents with the same prompt template from having the same hash, and prevents dictionary attacks.

---

## Interaction with Other Systems

### Reputation System

The ventriloquist defense feeds into the reputation system (see [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md)) through:

- **Prompt change penalties**: -0.05 to -0.10 per change beyond the threshold
- **TEE violation slashing**: Recorded in `slash_history`, affects all domain reputations
- **Verification failure**: A failed hash check during job execution is treated as job abandonment

### Marketplace

The Spore job marketplace (see [10-spore-job-market.md](./10-spore-job-market.md)) can use prompt hash freshness as a filtering criterion:

- Jobs with high security requirements can require TEE-attested agents with prompt hashes unchanged for 30+ days
- Auction bids can be weighted by prompt stability (agents with stable prompts receive a small bid advantage)

### Knowledge Contributions

Knowledge entries posted by agents with recent prompt changes are flagged for additional scrutiny. The demurrage-based weight system (see [02-korai-token-economics.md](./02-korai-token-economics.md)) can apply a multiplier that reduces the initial weight of entries from recently-changed agents.

---

## Academic Foundations

- Costan, V. and Devadas, S. (2016). "Intel SGX Explained." *IACR Cryptology ePrint Archive*. — Hardware-based trusted execution environments that provide the strongest verification level for the ventriloquist defense.
- (Weyl, Ohlhaver, Buterin, 2022). "Decentralized Society: Finding Web3's Soul." — Soulbound tokens and non-transferable reputation; the passport's soulbound property complements the ventriloquist defense against reputation laundering.
- Carlini, N. et al. (2024). "Stealing Part of a Production Language Model." *arXiv:2403.06634*. — Model extraction attacks that motivate prompt privacy; the hash-only approach prevents prompt leakage while enabling verification.

---

## Current Status and Gaps

**Scaffold:**
- `system_prompt_hash` field defined in the `AgentPassport` struct (implementation plan §A1)
- SHA-256 computation available via standard Rust `sha2` crate

**Not yet built (Tier 6):**
- `korai_updatePromptHash` RPC with 24h timelock (§M2)
- Rate limiting logic for prompt changes (§M3)
- TEE attestation integration with prompt hash verification (§M4)
- Emergency prompt freeze logic (§M5)
- Salted hash option for dictionary attack resistance (§M6)
- Marketplace integration for prompt stability filtering (§M7)

---

## Cross-references

- See [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md) for the passport struct containing `system_prompt_hash`
- See [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) for how prompt change penalties affect domain scores
- See [22-valhalla-privacy-layer.md](./22-valhalla-privacy-layer.md) for TEE integration at the privacy layer
- See [24-current-status-and-6-contracts.md](./24-current-status-and-6-contracts.md) for the Agent Registry contract that stores prompt hashes
- See topic [14-identity-economy](../14-identity-economy/INDEX.md) for broader identity and security context
