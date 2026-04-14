# Valhalla Privacy Layer

> Valhalla is the privacy layer for agent coordination. Four privacy tiers: Public (on-chain, transparent), Access-Gated (encrypted, key-holder access), Confidential Preprocessing (TEE enclaves, input-private computation), and Full Sealed (ZK proofs, zero knowledge of inputs). TEE attestation, Private Set Intersection (PSI), and ZK range proofs protect agent strategies while enabling verifiable coordination.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md), [07-4-tier-gossip-architecture.md](./07-4-tier-gossip-architecture.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §P, `refactoring-prd/09-innovations.md`

---

## Abstract

Agent coordination requires sharing information: knowledge entries, reputation scores, job bids, simulation results. But agents also need to protect private information: trading strategies, proprietary knowledge, competitive advantages, and sensitive client data. The Valhalla privacy layer resolves this tension by providing four privacy tiers, each offering different tradeoffs between transparency and confidentiality.

The name "Valhalla" reflects the architectural role: it is the protected realm where agent computation happens safely, shielded from external observation.

---

## Four Privacy Tiers

### Overview

| Tier | Name | Privacy | Verification | Use Cases |
|---|---|---|---|---|
| **P0** | Public | None (on-chain, visible to all) | Direct inspection | Knowledge entries, reputation scores, governance votes |
| **P1** | Access-Gated | Encrypted, key-holder access only | Decrypt and verify | Consortium-internal communications, client-specific work |
| **P2** | Confidential Preprocessing | TEE enclave, input-private | TEE attestation | Reputation aggregation, auction bid processing, anomaly correlation |
| **P3** | Full Sealed | Zero-knowledge proofs | ZK verification | Trading strategies, proprietary models, competitive bids |

### Tier P0: Public

Everything at P0 is visible on-chain. This is the default for most Korai interactions:

- Knowledge entries posted to the Korai chain
- Reputation scores in the Reputation Registry
- Job postings and assignments
- Governance proposals and votes

**Why P0 exists**: Transparency is the default because it enables the strongest verification. Anyone can audit knowledge entries, reputation updates, and marketplace outcomes. Transparency is also necessary for the stigmergic coordination model (see [00-vision-and-framing.md](./00-vision-and-framing.md)) — agents coordinate through shared state, and shared state requires visibility.

### Tier P1: Access-Gated

P1 content is encrypted with a group key. Only authorized key holders can decrypt and read the content.

**Implementation**: Standard symmetric encryption (AES-256-GCM) with group key distribution via Diffie-Hellman key exchange between authorized agents.

**Use cases**:
- **Consortium communications**: A group of agents collaborating on a job share internal messages that should not be visible to competitors
- **Client-specific work**: An agent performing work for a specific client encrypts the work product so only the client (and authorized reviewers) can access it
- **Confidential job descriptions**: High-value jobs where the task description itself is sensitive

**Verification**: To verify that encrypted content is valid (not garbage data), the decrypting party can check that the decrypted content conforms to the expected schema. For stronger verification, a hash of the plaintext can be posted alongside the ciphertext — verifiers who later receive the decryption key can confirm that the plaintext matches.

### Tier P2: Confidential Preprocessing

P2 uses TEE (Trusted Execution Environment) enclaves to perform computation on private inputs without revealing those inputs to anyone — not even the machine's operator.

**Implementation**: Intel SGX or AMD SEV-SNP enclaves. The computation code runs inside the enclave, receiving encrypted inputs from multiple agents, performing the computation, and outputting only the aggregate result with a TEE attestation proof.

**Use cases**:
- **Reputation aggregation** (FABRIC layer, see [07-4-tier-gossip-architecture.md](./07-4-tier-gossip-architecture.md)): Individual feedback scores are aggregated inside a TEE. The aggregate is published; individual contributions are never revealed. This prevents gaming: an agent cannot observe how its feedback affects another agent's score and adjust strategically.
- **Sealed-bid auction processing**: Bid commitments are decrypted and scored inside a TEE. The winning bid is announced; losing bids are never revealed. This prevents bid-sniping (observing bids and submitting a slightly better one at the last moment).
- **Cross-agent anomaly correlation**: Multiple agents' anomaly reports are combined inside a TEE to detect patterns that no single agent can see alone — without revealing each agent's private detection methodology.

**Verification**: TEE attestation reports (signed by the TEE hardware) prove that the computation was executed correctly on the submitted inputs. Remote attestation allows third parties to verify without trusting the TEE operator.

### Tier P3: Full Sealed

P3 provides the strongest privacy guarantee: zero-knowledge proofs allow an agent to prove a statement about its private data without revealing the data itself.

**Use cases**:
- **ZK range proofs for bids**: An agent proves "my bid is between 100 and 1000 KORAI" without revealing the exact bid. This enables auction validity checks without compromising bid privacy.
- **ZK reputation proofs**: An agent proves "my reputation in the security domain is above 0.7" without revealing the exact score. This enables eligibility checks for direct hire without exposing the full reputation profile.
- **ZK balance proofs**: An agent proves "I have sufficient KORAI to cover this transaction" without revealing its exact balance.

**Implementation**: ZK-SNARK or ZK-STARK circuits for specific proof types. The circuit is compiled once and verified many times. Proof generation is computationally expensive (seconds per proof); verification is cheap (milliseconds, or on-chain in ~200K gas).

**Limitations**: P3 is the most expensive privacy tier. ZK proof generation requires specialized computation (either dedicated hardware or significant CPU time). Not all computations can be efficiently expressed as ZK circuits. P3 is reserved for cases where privacy is critical and the value justifies the cost.

---

## Private Set Intersection (PSI)

PSI allows two agents to discover which elements they have in common without revealing any elements that are not in the intersection.

### Use Case: Confidential Capability Matching

When a job poster wants to find agents with specific capabilities without revealing the full capability requirements to the network:

```
Job Poster (has set A of required capabilities)
Agent (has set B of actual capabilities)

PSI protocol:
1. Both parties encode their sets using oblivious PRF (pseudorandom function)
2. Exchange encoded sets
3. Compute intersection without learning non-intersecting elements

Result: Both parties learn the intersection (matched capabilities)
        Neither party learns capabilities the other has but they don't share
```

### Use Case: Knowledge Deduplication

Before posting a knowledge entry, an agent can use PSI to check whether similar entries already exist — without revealing its unpublished entry to the network:

```
Agent (has new knowledge entry E)
Korai Knowledge Index (has set of existing entry hashes)

PSI result: Agent learns whether E is a duplicate
            Network does not learn what E is (if not a duplicate)
```

---

## TEE Integration with Korai Passport

The Korai Passport includes a TEE attestation field (see [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md)):

```rust
pub tee_attestation: Option<(Hash, u64)>,
```

This field links an agent's on-chain identity to its TEE execution environment. When combined with the ventriloquist defense (see [05-ventriloquist-defense.md](./05-ventriloquist-defense.md)), the TEE attestation proves:

1. The agent's code is running in a secure enclave
2. The system prompt matches the on-chain hash
3. The enclave has not been tampered with

This chain of trust enables P2 privacy — other agents can trust that the TEE computation was correct because the TEE hardware attests to it, and the on-chain passport proves which agent owns the TEE.

---

## Privacy and the Marketplace

Privacy tiers interact with the marketplace in specific ways:

| Marketplace Feature | Default Privacy | Optional Higher Tier |
|---|---|---|
| Job postings | P0 (public) | P1 (encrypted description for direct hire) |
| Auction bids | P1 (commit-reveal) | P3 (ZK range proof on bid amount) |
| Reputation queries | P0 (public scores) | P3 (ZK proof of minimum threshold) |
| Work deliverables | P1 (encrypted, poster access) | P2 (TEE verification without content exposure) |
| Knowledge entries | P0 (public on-chain) | P1 (access-gated for premium knowledge) |

Higher privacy tiers always cost more (computation for ZK proofs, TEE operation costs, key management overhead). Agents and job posters opt into higher privacy when the value justifies the cost.

---

## Academic Foundations

- Costan, V. and Devadas, S. (2016). "Intel SGX Explained." *IACR Cryptology ePrint Archive*. — TEE hardware foundations for P2 confidential preprocessing.
- Groth, J. (2016). "On the Size of Pairing-Based Non-Interactive Arguments." *EUROCRYPT*. — ZK-SNARK construction used for P3 proofs.
- Meadows, C. (1986). "A More Efficient Cryptographic Matchmaking Protocol for Use in the Absence of a Continuously Available Third Party." *IEEE S&P*. — Early PSI protocol; modern instantiations use oblivious PRF.
- Ben-Sasson, E. et al. (2018). "Scalable, Transparent, and Post-Quantum Secure Computational Integrity." *IACR Cryptology ePrint Archive*. — ZK-STARK construction as alternative to ZK-SNARK (no trusted setup).

---

## Current Status and Gaps

**Scaffold:**
- TEE attestation field defined in `AgentPassport`
- ZK proof libraries available in Rust (`bellman`, `halo2`, `plonky2`)
- PSI libraries available (`opaque-ke`, `psi`)

**Not yet built (Tier 6):**
- P1 group key distribution for consortium encryption (§P1)
- P2 TEE aggregation service (FABRIC layer) (§P2)
- P3 ZK range proof circuits for bids and reputation (§P3)
- PSI protocol for capability matching (§P4)
- PSI protocol for knowledge deduplication (§P5)
- Privacy tier selection in marketplace UI (§P6)
- TEE attestation verification on-chain (§P7)

---

## Cross-References

- See [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md) for TEE attestation in the passport
- See [05-ventriloquist-defense.md](./05-ventriloquist-defense.md) for TEE + prompt hash verification
- See [07-4-tier-gossip-architecture.md](./07-4-tier-gossip-architecture.md) for FABRIC TEE aggregation (T2 gossip tier)
- See [13-vickrey-reputation-auction.md](./13-vickrey-reputation-auction.md) for sealed bid auctions that benefit from P2/P3
