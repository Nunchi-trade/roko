# Protocol Standards

> Blockchain protocol standards, agent identity specifications, and interoperability protocols relevant to Roko's on-chain subsystems.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md` §12

---

## Abstract

Roko agents can operate on-chain via the Korai chain — a dedicated EVM for agent coordination with 400ms blocks and an HDC precompile. This section collects the ERC standards, protocol specifications, and interoperability frameworks that ground the on-chain subsystems: agent identity (ERC-8004), token standards (ERC-20, ERC-721), account abstraction (ERC-4337), micropayments (x402), and cross-chain coordination.

---

## Agent Identity

- Bryan, K. (2024). ERC-8004: Agent Identity. EIPs.
  *Grounds: Korai Passport — agent identity standard. ERC-721 soulbound with capabilityList bitmask, domainStakes, reputationTracks, teeAttestation, systemPromptHash (ventriloquist defense), tier classification, and slashHistory.*

- Bryan, K. (2024). ERC-8001: Agent Coordination Framework. EIPs. Status: Final.
  *Grounds: Agent coordination — framework for on-chain agent coordination. Provides the protocol-level primitives for multi-agent interaction.*

- Parikh, R. & Ross, J.M. (2025). ERC-8033: Agent Council Oracles. EIPs. Status: Draft.
  *Grounds: Council oracles — on-chain oracle mechanism for multi-agent council decisions.*

- Crapis, D. et al. (2026). ERC-8183: Agentic Commerce Protocol. EIPs. Status: Draft.
  *Grounds: Agent commerce — protocol for agent-to-agent economic transactions.*

---

## Token Standards

- Ethereum Foundation. ERC-20: Token Standard. EIPs.
  *Grounds: KORAI token — standard fungible token interface. KORAI (mainnet) and DAEJI (testnet) implement ERC-20 with 1% annual demurrage.*

- Ethereum Foundation. ERC-721: Non-Fungible Token Standard. EIPs.
  *Grounds: Korai Passport — ERC-721 soulbound token for agent identity. Each agent has a unique non-transferable NFT that encodes its capabilities and reputation.*

- Ethereum Foundation (2021). ERC-4337: Account Abstraction Using Alt Mempool. EIPs.
  *Grounds: Account abstraction — enables agents to operate with smart contract wallets that support custom validation logic, gas sponsorship, and batched transactions.*

- Ethereum Foundation (2024). ERC-7683: Cross Chain Intents. EIPs.
  *Grounds: Cross-chain intents — standardized format for expressing cross-chain transaction intents. Enables agents to operate across multiple chains.*

---

## Micropayments

- Cloudflare/Linux Foundation (2025). x402: HTTP 402 Payment Required Protocol for Machine-to-Machine Micropayments.
  *Grounds: x402 innovation — self-funding agents via per-API-call billing at < $0.001 per transaction. Sub-second USDC settlement on Base. Enables the self-funding economic cycle: agent earns from knowledge → spends on compute → produces value → earns more.*

---

## Attestation and Provenance

- Ethereum Attestation Service (2024-2026). EAS Documentation. attest.sh.
  *Grounds: On-chain attestation — attestation service for Engram verification. Provides the infrastructure for the Attestation field on Engrams.*

- Uniswap Labs (2022). Permit2: Signature-Based Token Approvals.
  *Grounds: Token permissions — signature-based token approval system. Enables gasless token approvals for agent-to-agent transactions.*

---

## Agent Protocols

- Google (2025). Agent-to-Agent (A2A) Protocol Specification.
  *Grounds: A2A protocol — standardized agent-to-agent communication. Cross-referenced in [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md).*

- Anthropic (2024). Model Context Protocol (MCP) Specification.
  *Grounds: MCP — tool interaction protocol. Cross-referenced in [04-coordination-and-multi-agent.md](./04-coordination-and-multi-agent.md).*

- Virtuals Protocol (2025). Agent Commerce Protocol: Agent-to-Agent Economic Coordination.
  *Grounds: Agent commerce — agent-to-agent economic coordination protocol for autonomous value exchange.*

---

## Cryptographic Primitives

- Merkle, R.C. (1987). A Digital Signature Based on a Conventional Encryption Function. In _CRYPTO '87_, LNCS 293, 369-378.
  *Grounds: Merkle trees — foundational data structure for content-addressed verification. Roko's BLAKE3 content hashing on Engrams is a descendant of Merkle's content-addressing approach.*

- Goldwasser, S., Micali, S., & Rackoff, C. (1985). The Knowledge Complexity of Interactive Proof Systems. In _STOC '85_, 291-304.
  *Grounds: Zero-knowledge proofs — foundational work on ZK proofs. Grounds the Valhalla privacy layer's ZK range proofs for privacy-preserving knowledge verification.*

- Ben-Sasson, E. et al. (2018). Scalable, Transparent, and Post-Quantum Secure Computational Integrity. Cryptology ePrint Archive, 2018/046.
  *Grounds: STARKs — scalable transparent proofs without trusted setup. Post-quantum secure. Potential future proving system for agent computation verification.*

- Benet, J. (2014). IPFS — Content Addressed, Versioned, P2P File System. arXiv:1407.3561.
  *Grounds: Content addressing — content-addressed P2P storage. Roko's BLAKE3 content-addressed Engrams follow the same content-addressing principle.*

- Szabo, N. (1997). Formalizing and Securing Relationships on Public Networks. _First Monday_, 2(9).
  *Grounds: Smart contracts — foundational concept of self-enforcing digital agreements. The Policy trait enforces agent behavior constraints as computational contracts.*

---

## Cross-references

- See [08-security-and-provenance.md](./08-security-and-provenance.md) for security architecture
- See [19-regulatory-compliance.md](./19-regulatory-compliance.md) for compliance frameworks
- See [21-mechanism-design.md](./21-mechanism-design.md) for token economics
