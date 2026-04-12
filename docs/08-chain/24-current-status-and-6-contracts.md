# Current Status and 6 Solidity Contracts

> Summary of what is built, what is scaffolded, and what remains for Tier 6. Six planned Solidity contracts: Agent Registry (Identity), Reputation Registry, Marketplace (Spore), Escrow, KORAI Token, and Validation Registry. All are Tier 6 deferred — blocked by Tier 5 completion.

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: All previous sub-docs in this topic
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md`, `refactoring-prd/07-implementation-priorities.md`

---

## Abstract

This document summarizes the current implementation status of the Korai chain layer and catalogs the six planned Solidity contracts that will form the on-chain infrastructure. All Tier 6 (chain layer) work is deferred — it is blocked by completion of Tier 5 (the self-hosting loop: TUI, automatic plan generation, feedback loop). The chain layer is the most complex tier in the implementation plan, with 76 items across 11 sections.

---

## Implementation Status

### What Is Built

| Component | Crate/Location | Status | Notes |
|---|---|---|---|
| `ChainClient` trait | `roko-chain/src/client.rs` | **Built** | Async trait with 8 methods for chain reads |
| `ChainWallet` trait | `roko-chain/src/wallet.rs` | **Built** | Async trait with 5 methods for chain writes |
| Chain types | `roko-chain/src/types.rs` | **Built** | BlockNumber, TxHash, ChainHeader, Receipt, LogEntry, ChainError |
| `TxSimGate` stub | `roko-chain/src/lib.rs` | **Stub** | Interface defined, verification logic not implemented |
| `WalletGate` stub | `roko-chain/src/lib.rs` | **Stub** | Interface defined, verification logic not implemented |
| Mock implementations | `roko-chain/src/` | **Built** | MockChainClient, MockChainWallet for testing |
| mirage-rs core | `apps/mirage-rs/` | **Built** | Full EVM simulator, 141 tests, fork mode, scenario replay |
| mirage-rs chain extensions | `apps/mirage-rs/src/chain/` | **Scaffold** | Module structure exists, implementation incomplete |
| HDC local operations | `bardo-primitives/src/hdc.rs` | **Built** | BIND, BUNDLE, PERMUTE, Hamming similarity with SIMD |
| HDC index (HNSW) | `mirage-rs/src/chain/hnsw.rs` | **Built** | Approximate nearest neighbor index |
| AgentPassport struct | Implementation plan §A1 | **Specified** | Full struct defined, not yet implemented |

### What Is Scaffolded

| Component | Location | What Exists | What Is Missing |
|---|---|---|---|
| Chain intelligence pipeline | Legacy `bardo-backup/prd/14-chain/` | Full specification (9 docs) | All implementation |
| Gossip architecture | Implementation plan §B | Config structs, topic definitions | GossipSub integration, message handling |
| Job marketplace (Spore) | Implementation plan §C | Transaction types, hiring models | All Solidity contracts and runtime logic |
| Reputation system | Implementation plan §K | Score structs, EMA formula | Contract, decay ticks, discipline states |
| ChainWitness | Legacy `bardo-backup/prd/14-chain/01-witness.md` | Full specification | All implementation |
| Triage pipeline | Legacy `bardo-backup/prd/14-chain/02-triage.md` | Full specification (71KB) | All implementation |
| Privacy layer (Valhalla) | Implementation plan §P | Privacy tier definitions | TEE integration, ZK circuits, PSI |
| x402 micropayments | `refactoring-prd/09-innovations.md` §VIII | Protocol description | Client library, server middleware |

### What Is Not Yet Built (Tier 6)

All Tier 6 items are deferred. The implementation plan (12b-chain-layer.md) contains 76 items across 11 sections:

| Section | Items | Key Deliverables |
|---|---|---|
| **A: Identity** | 7 items | Full AgentPassport struct, korai_registerPassport RPC, tier progression, capability bitmask, Ed25519 wallet, local identity |
| **B: Gossip** | 10 items | GossipSub mesh, 8 topics, gossip envelope, peer scoring, FABRIC aggregation |
| **C: Job Market** | 20 items | Spore marketplace, Sparrow dispatch, 3 hiring models, Vickrey auction, escrow |
| **H: ChainWitness** | 17 items | WitnessEngine, Binary Fuse filter, triage pipeline, MIDAS-R, curiosity scoring |
| **K: Reputation** | 11 items | Reputation Registry, EMA scoring, adaptive alpha, decay, discipline states, slashing |
| **L: Payments** | 6 items | x402 client/server, batch settlement, balance verification |
| **M: Safety** | 7 items | Ventriloquist defense, prompt update timelock, TEE attestation, emergency freeze |
| **N: ISFR** | 2 items | Intersubjective Fact Registry, reputation-weighted aggregation |
| **O: Clearing** | 5 items | QP solver, bisection algorithm, KKT certificates, on-chain verification |
| **P: Privacy** | 7 items | Valhalla 4-tier privacy, TEE aggregation, ZK proofs, PSI |
| **Q: mirage-rs** | 5 items | HDC precompile emulation, registry emulation, Korai RPC, roko_bridge |
| **R: Crate Architecture** | 5 items | Live RPC client, ERC-4337 wallet, TEE wallet, gate implementations |

---

## Six Planned Solidity Contracts

### 1. Agent Registry (Identity Registry)

**Address**: `0xA100` (predeployed at genesis)

**Purpose**: Manages Korai Passports — soulbound ERC-721 NFTs that serve as agent identity.

**Key functions**:
- `registerPassport(owner, capabilities, promptHash, teeAttestation)` → passport_id
- `updateCapabilities(passportId, newCapabilities)`
- `updatePromptHash(passportId, newHash)` — 24h timelock
- `stakeIntoDomain(passportId, domain, amount)`
- `withdrawFromDomain(passportId, domain, amount)` — cooldown period
- `getPassport(passportId)` → AgentPassport
- `getTier(passportId)` → tier

**Spec**: See [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md), [06-erc-8004-registries.md](./06-erc-8004-registries.md)

### 2. Reputation Registry

**Address**: `0xA200` (predeployed at genesis)

**Purpose**: Stores per-domain reputation scores and manages feedback authorization.

**Key functions**:
- `submitFeedback(passportId, domain, score, jobHash, reason)` — authorized sources only
- `applyDecayTick(passportId)` — called by epoch handler
- `slash(passportId, violationType, amount, reason)`
- `getReputation(passportId, domain)` → (score, jobCount, lastUpdate)
- `isAuthorizedFeedbackSource(address)` → bool
- `addFeedbackSource(address)` — governance only

**Spec**: See [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md), [06-erc-8004-registries.md](./06-erc-8004-registries.md)

### 3. Marketplace (Spore)

**Address**: Deployed by governance

**Purpose**: Job posting, matching, and lifecycle management.

**Key functions**:
- `postJob(domain, capabilities, budget, deadline, hiringModel)` → jobId
- `submitBid(jobId, amount, commitment)` — for auction hiring
- `revealBid(jobId, amount, salt)` — for sealed bid auctions
- `acceptDirectHire(jobId)` — for direct hire offers
- `submitDeliverables(jobId, merkleRoot, gateResults)`
- `disputeJob(jobId, reason)`
- `resolveDispute(jobId, resolution)`

**Spec**: See [10-spore-job-market.md](./10-spore-job-market.md), [12-three-hiring-models.md](./12-three-hiring-models.md), [13-vickrey-reputation-auction.md](./13-vickrey-reputation-auction.md)

### 4. Escrow

**Address**: Deployed by governance

**Purpose**: Holds job budgets in escrow during execution. Releases to agent on verification, returns to poster on abandonment.

**Key functions**:
- `deposit(jobId, amount)` — called by Marketplace on job posting
- `release(jobId, agentPassportId, amount)` — called by Marketplace on verification
- `refund(jobId, posterAddress)` — called by Marketplace on abandonment
- `disputeHold(jobId)` — freeze funds during dispute resolution
- `disputeRelease(jobId, allocation)` — distribute funds per dispute resolution

**Fee structure**: 2% escrow fee (non-refundable), 3% marketplace fee (deducted from agent payout)

**Spec**: See [10-spore-job-market.md](./10-spore-job-market.md)

### 5. KORAI Token

**Address**: Predeployed at genesis

**Purpose**: The native token of the Korai chain. Implements ERC-20 with demurrage (1% annual decay) and ERC-3009 (transferWithAuthorization for x402 payments).

**Key functions**:
- Standard ERC-20: `transfer`, `approve`, `transferFrom`, `balanceOf`
- ERC-3009: `transferWithAuthorization(from, to, value, validAfter, validBefore, nonce, v, r, s)`
- Demurrage: `applyDemurrage(address)` — called at epoch boundaries
- Minting: `mint(address, amount)` — called by authorized minting sources (job rewards, registration)
- Burning: `burn(amount)` — voluntary burn; also triggered by posting fees and slashing

**Spec**: See [02-korai-token-economics.md](./02-korai-token-economics.md), [20-x402-micropayments.md](./20-x402-micropayments.md)

### 6. Validation Registry

**Address**: `0xA300` (predeployed at genesis)

**Purpose**: Records proofs of completed work. Provides an auditable trail of agent contributions.

**Key functions**:
- `submitWorkProof(passportId, jobHash, merkleRoot, gateResults, clearingCert)`
- `verifyWork(jobHash)` → WorkProof
- `getWorkProofs(passportId, fromBlock, toBlock)` → WorkProof[]
- `getGatePassRate(passportId, domain)` → (passRate, totalJobs)

**Spec**: See [06-erc-8004-registries.md](./06-erc-8004-registries.md)

---

## Contract Interaction Diagram

```
┌─────────────┐     registers     ┌──────────────┐
│   Agent     │────────────────→ │  Agent       │
│   (off-chain)│                  │  Registry    │
│             │←─── passport_id ──│  (0xA100)    │
└──────┬──────┘                  └──────┬───────┘
       │                                │
       │ bids on job                    │ reputation lookup
       │                                │
       ▼                                ▼
┌──────────────┐    feedback    ┌──────────────┐
│  Marketplace │───────────────→│  Reputation  │
│  (Spore)     │                │  Registry    │
│              │                │  (0xA200)    │
└──────┬───────┘                └──────────────┘
       │
       │ escrow deposit/release
       │
       ▼
┌──────────────┐    fee burn    ┌──────────────┐
│  Escrow      │───────────────→│  KORAI       │
│              │                │  Token       │
└──────┬───────┘                └──────────────┘
       │
       │ work proof submission
       │
       ▼
┌──────────────┐
│  Validation  │
│  Registry    │
│  (0xA300)    │
└──────────────┘
```

---

## Tier 6 Dependencies

Tier 6 (chain layer) is blocked by Tier 5 (self-hosting loop):

| Tier 5 Blocker | Why It Blocks Tier 6 |
|---|---|
| Interactive TUI | Need TUI to monitor chain agent behavior during development |
| Automatic plan generation | Chain layer has 76 items; needs automated planning |
| Feedback loop | Failed chain operations need to feed back into plan generator |

Additionally, Tier 6 has internal dependencies:

```
Agent Registry ← (needed by) ← Reputation Registry
Agent Registry ← (needed by) ← Marketplace
KORAI Token   ← (needed by) ← Marketplace, Escrow
Reputation Registry ← (needed by) ← Marketplace (for auction scoring)
Marketplace ← (needed by) ← Escrow
Marketplace ← (needed by) ← Validation Registry
```

The recommended build order:
1. KORAI Token (no dependencies)
2. Agent Registry (depends on KORAI for staking)
3. Reputation Registry (depends on Agent Registry)
4. Validation Registry (depends on Agent Registry)
5. Escrow (depends on KORAI)
6. Marketplace (depends on all of the above)

---

## Cross-references

- See [00-vision-and-framing.md](./00-vision-and-framing.md) for why Tier 6 is deferred
- See [01-korai-chain-spec.md](./01-korai-chain-spec.md) for the chain that hosts these contracts
- See [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) for the development environment that emulates these contracts
- See `refactoring-prd/07-implementation-priorities.md` for the full tier dependency graph
