# Vision and Framing: Blockchain as Domain Plugin

> Korai chain is ONE domain plugin for agent coordination — not the default framing, not the center of the architecture. The core cognitive system is domain-agnostic.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md), [06-neuro](../06-neuro/INDEX.md)
**Key sources**: `refactoring-prd/04-knowledge-and-mesh.md`, `refactoring-prd/07-implementation-priorities.md`, `bardo-backup/tmp/agent-chain/01-overview.md`

---

## Abstract

Roko is a cognitive agent operating system. Its kernel — the Synapse Architecture with Engrams, six composable traits (Substrate, Scorer, Gate, Router, Composer, Policy), five architectural layers (Runtime, Framework, Scaffold, Harness, Orchestration), and three cognitive cross-cuts (Neuro, Daimon, Dreams) — operates identically whether the agent writes code, monitors infrastructure, conducts research, or interacts with blockchains.

The Korai chain is a **domain plugin** that extends this kernel with chain-specific capabilities: on-chain identity, token economics, decentralized job markets, reputation systems, and collective knowledge coordination. It is one instance of the pattern `domain_specific_trait_implementations + domain_specific_configuration = domain_agent`. Coding agents have their own domain plugin (CompileGate, TestGate, SymbolScorer). Chain agents have theirs (TxSimGate, WalletGate, ChainSubstrate). Neither is more fundamental than the other.

This framing matters because the most powerful agents will span multiple domains simultaneously. A single agent can write Solidity contracts (coding domain), simulate their deployment on mirage-rs (chain domain), monitor on-chain performance (chain domain), and research competing protocols (research domain). The Synapse Architecture makes this composition natural — each domain contributes its Gate, Scorer, and Substrate implementations, and the universal cognitive loop orchestrates them all.

This document establishes the vision for how blockchain capabilities fit into the Roko architecture, what goes on-chain versus off-chain, and why a dedicated agent coordination chain (Korai) exists at all.

---

## The Problem: Siloed Agent Knowledge

Every AI agent learns valuable operational knowledge from real tasks. A coding agent discovers that "Rust trait objects cannot be Send + Clone simultaneously." A chain agent discovers that "high gas spikes on Ethereum correlate with MEV bot activity in the next 3 blocks." A research agent discovers that "contradictory sources on topic X indicate an emerging paradigm shift."

This knowledge is **siloed, ephemeral, and inaccessible** to other agents. Each agent's learning dies when the agent process terminates. Even within a single operator's set of agents, knowledge sharing is ad-hoc — there is no structured mechanism for one agent's hard-won insight to benefit another.

The cost of this isolation is enormous. Every new agent starts from zero. Every agent independently discovers the same patterns, makes the same mistakes, and pays the same inference costs to re-learn what hundreds of predecessor agents already knew.

The Korai chain exists to solve this problem: a shared, self-curating knowledge ledger that grows smarter with every participating agent. But it is one solution among several in the Roko architecture, not the only one.

---

## Three-Level Knowledge Architecture

Agents access knowledge at three levels, each with different properties:

```
┌──────────────────────────────────────────────────┐
│          Korai Chain (Global Public)              │
│  On-chain HDC vectors, KORAI tokenomics,         │
│  collective knowledge, reputation, ERC-8004       │
├──────────────────────────────────────────────────┤
│          Agent Mesh (Peer/Private)                │
│  WebSocket / Iroh P2P connections,                │
│  permissioned subnets, company collectives        │
├──────────────────────────────────────────────────┤
│          Local Neuro Store (Private)              │
│  Per-agent knowledge, JSONL + HDC indexing,       │
│  tiered with half-life decay                      │
└──────────────────────────────────────────────────┘
```

### Level 1: Local Neuro Store (Private)

Every agent has a local NeuroStore — a structured knowledge base with six knowledge types (Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge), four decay tiers (Transient, Working, Consolidated, Persistent), and HDC encoding for similarity search. This is the agent's private memory, managed by the `roko-neuro` crate. It exists whether or not a chain is involved.

The Local Neuro Store implements the `Substrate` Synapse trait (L0 Runtime layer). Knowledge entries are Engrams — content-addressed, scored, decaying, lineage-tracked units of cognition. Each entry has a BLAKE3 content hash, 7-axis score (confidence, novelty, utility, reputation, precision, salience, coherence), and configurable decay (Ebbinghaus half-life × tier multiplier).

### Level 2: Agent Mesh (Peer/Private)

Agents connect to each other via the Agent Mesh — WebSocket for co-located agents, Iroh P2P for cross-network agents. Within a mesh, agents share knowledge selectively through permissioned subnets. A company's agents might share a private mesh where proprietary knowledge circulates without ever touching the public chain.

The Agent Mesh enables what legacy documentation called "clade synchronization" (now collective mesh sync). Members of a collective share high-confidence entries, coordinate task division, and maintain internal reputation scores separate from public scores.

### Level 3: Korai Chain (Global Public)

The Korai chain is a dedicated EVM chain for agent knowledge coordination. Agents opt into publishing validated knowledge to the public chain, where it becomes available to all participating agents. The chain provides:

- **ERC-8004 Agent Identity** — on-chain registration with capabilities, endpoints, and reputation
- **HDC Precompile** — native EVM precompile for 10,240-bit hyperdimensional vector similarity search at ~400 gas
- **KORAI Token Economics** — demurrage token (1% annual decay) that incentivizes knowledge quality
- **Pheromone Contracts** — typed coordination signals with on-chain decay profiles
- **Reputation Registry** — 7-domain exponential moving average reputation system

The key design principle: **agents that never interact with a blockchain still benefit from the full Roko cognitive stack**. The Korai chain amplifies collective intelligence but is not required for individual agent operation. Solo coding agents, event-driven operations agents, and local research agents work perfectly without it.

---

## What Goes On-Chain vs. Off-Chain

This distinction is critical for understanding what the Korai chain is and is not.

### On-Chain (Korai)

| Data Type | Rationale |
|---|---|
| **Knowledge entries** | HDC-encoded insights, heuristics, warnings, causal links, strategy fragments, anti-knowledge. These are the collective's shared intelligence. |
| **Agent identity** | ERC-8004 registration with Agent Card (capabilities, endpoints, reputation). Discovery: "find all agents that can do DeFi analysis." |
| **Reputation signals** | Earned through validated knowledge contributions. Per-domain EMA scores. |
| **Pheromones** | Typed coordination signals (Threat, Opportunity, Wisdom, Alpha, Pattern, Anomaly, Consensus) with on-chain decay. |
| **Job market state** | BountySpec postings, bids, escrow, completion proofs. The Spore/Sparrow marketplace. |
| **ISFR rates** | Intersubjective Fact Registry — collectively discovered reference rates. |
| **Clearing certificates** | QP solver optimality proofs for cross-agent obligation settlement. |

### Off-Chain (Never on Korai)

| Data Type | Rationale |
|---|---|
| **Episode logs** | Too large, too frequent. Agent execution traces stay local. |
| **Raw prompts/outputs** | Private. LLM conversations are not published. |
| **Daimon state** | Internal cognitive state (PAD vector, behavioral state). Private to the agent. |
| **Proprietary strategies** | Unless the user explicitly opts to publish. Competitive advantage stays local. |
| **Full Engram bodies** | Only HDC-encoded summaries go on-chain. Full text stays in the Local Neuro Store. |

---

## Why a Dedicated Chain?

Why not use Ethereum mainnet, or Base, or any existing EVM chain?

### 1. HDC Precompile

The core innovation is a native EVM precompile for 10,240-bit hyperdimensional vector similarity search. This is not possible as a Solidity contract — the gas costs would be prohibitive. As a native precompile on the Korai chain, top-K similarity search over 10,240-bit BSC (Binary Spatter Code) vectors costs approximately 400 gas. This enables agents to query the collective knowledge base directly from smart contracts, with the same encoding used locally by the `roko-primitives` crate.

The HDC precompile is a **custom Korai feature**, not a mainnet Ethereum capability. It needs benchmarking and validation on the Korai testnet (Daeji) before production use.

### 2. 400ms Block Time

Korai targets 400ms block times — fast enough for agent coordination but slow enough for meaningful consensus. This is significantly faster than Ethereum's 12-second blocks or even most L2 sequencing. Agent job markets, reputation updates, and pheromone coordination benefit from sub-second finality.

### 3. Agents as First-Class Citizens

On Korai, agents are not "users pretending to be smart contracts." They are first-class citizens with dedicated identity (ERC-8004 Korai Passport), reputation systems designed for non-human actors, and economic mechanisms (demurrage tokens, job markets, clearing) tuned for autonomous agent behavior.

### 4. Purpose-Built Economics

KORAI token economics are designed around knowledge quality incentives, not speculation. The 1% annual demurrage ensures that stale, unvalidated knowledge decays economically just as it decays in the NeuroStore's half-life system. Earning mechanisms reward validated knowledge contributions; spending mechanisms create anti-spam barriers.

---

## Chain Selection Rationale: Why EVM?

The choice of EVM as the execution environment for Korai was deliberate, with four alternative VM architectures evaluated and rejected. This section documents the decision and its trade-offs.

### Alternatives Evaluated

#### Move VM (Aptos/Sui)

Move's linear type system (resources cannot be copied or dropped — enforced at the bytecode verifier level) is theoretically superior for representing agent state. An agent's capabilities, credentials, and memory map naturally to resources that cannot be accidentally duplicated. Sui's object-centric model with DAG-based execution achieves sub-second finality for owned-object transactions without full consensus, and its explicit parallelism model is ideal for non-conflicting agent actions.

**Why rejected**: The Move ecosystem is 1/100th the size of EVM by developer tooling, auditor availability, and DeFi liquidity. Cross-chain bridge support is limited. The decision to build on EVM preserves access to the largest smart contract ecosystem — Foundry, OpenZeppelin, the entire Solidity auditing industry — while Arbitrum Stylus (see below) closes most of Move's safety advantages by enabling Rust contracts on EVM chains.

#### CosmWasm (Cosmos SDK)

Cosmos SDK appchains with CosmWasm offer full sovereignty (own validator set), IBC-native interoperability across 200+ chains, and Rust-based smart contracts with an actor model that eliminates re-entrancy by design. CometBFT consensus provides deterministic ~6s finality.

**Why rejected**: Empirical evidence shows EVM consistently winning even within the Cosmos ecosystem — Sei's CosmWasm usage dropped below 20% after adding an EVM layer; Injective followed similar patterns. The Cosmos security model requires bootstrapping your own validator set (or using Interchain Security from the Cosmos Hub), while EVM L2/L3 rollups inherit Ethereum's $60B+ security budget. IBC interoperability is valuable but secondary to EVM composability for the Korai use case.

#### Solana VM (SVM)

SVM offers the highest throughput of any production L1 (50,000+ TPS, 400ms block time) and the lowest transaction fees. The explicit account ownership model forces data isolation that maps well to multi-agent systems.

**Why rejected**: SVM's account model is extremely unintuitive for EVM developers. Programs store no state; all state resides in separate accounts. The developer experience gap is large, and the network has experienced multiple outages. SVM outside Solana L1 (Eclipse, etc.) is still early-stage.

#### Custom VM (from scratch)

A purpose-built VM optimized for HDC operations and agent coordination could achieve maximum efficiency but would require building an entire compiler toolchain, debugger, and developer ecosystem from scratch.

**Why rejected**: The cold-start problem. Building a VM ecosystem takes years and millions of dollars. EVM provides a ready-made foundation that can be extended with custom precompiles. The 80/20 analysis: EVM handles 80% of the workload well; custom precompiles (via Stylus or native) handle the 20% that needs specialization.

### The Winning Approach: EVM + Stylus Precompiles

The Korai chain uses EVM as its base execution layer, extended with custom precompiles for HDC operations. Two implementation paths are viable:

**Path A — Arbitrum Orbit + Stylus (preferred)**:
Deploy Korai as an Arbitrum Orbit L3 chain with the Stylus WASM VM enabled. HDC operations are implemented as Stylus contracts in Rust, achieving 10-100x gas reduction over equivalent Solidity. Stylus contracts share state with Solidity contracts and are called via standard ABI — no special integration needed. This path inherits Arbitrum's fraud proof system (BoLD, permissionless as of 2025) and Ethereum's security.

```rust
// Stylus HDC precompile — compiles to WASM, runs at near-native speed
#[external]
fn hdc_similarity(a: Bytes, b: Bytes) -> Result<U256, Vec<u8>> {
    // 160 native 64-bit XOR + POPCNT operations
    // ~5-6 gas via Stylus (vs. ~2,220 gas in Solidity)
    let a_words: &[u64; 160] = bytemuck::cast_ref(&a[..1280]);
    let b_words: &[u64; 160] = bytemuck::cast_ref(&b[..1280]);
    let matching = a_words.iter().zip(b_words.iter())
        .map(|(x, y)| (x ^ y).count_ones())
        .sum::<u32>();
    let similarity = U256::from(10240 - matching) * U256::from(10).pow(U256::from(18))
        / U256::from(10240);
    Ok(similarity)
}
```

**Path B — Native Reth fork (maximum performance)**:
Fork Reth and add HDC as a native precompile (like SHA-256 at 0x02), deployed at genesis address 0xA01. This provides the absolute lowest gas costs (~16 gas for Hamming distance) but requires maintaining a custom execution client fork. Suitable if Korai runs its own validator set.

**Performance comparison**:

| Operation | Solidity | Stylus (Rust/WASM) | Native Precompile |
|---|---|---|---|
| HDC XOR (1280 bytes) | ~120 gas | ~5-6 gas | ~5 gas |
| Hamming distance | ~2,220 gas | ~16-20 gas | ~16 gas |
| Top-K (N=1000, K=20) | Infeasible | ~16,000 gas | ~400 gas* |

*Native precompile with index access; Stylus would need to iterate over on-chain storage.

### Cross-Chain Interoperability Architecture

Korai must interoperate with Ethereum mainnet (where DeFi liquidity lives), other L2s (where agents may operate), and potentially non-EVM chains. The interoperability stack uses a layered approach:

#### Layer 1: Native Bridge (Orbit)

If Korai is deployed as an Orbit L3, it inherits Arbitrum's canonical bridge to Ethereum L1. This bridge is secured by the Nitro fraud proof system with 7-day withdrawal windows (or faster via ZK validity proofs via OP Succinct-style replacements). Token transfers between Korai and Ethereum L1 are trustless — no external validator set required.

#### Layer 2: Hyperlane ISM (Permissionless Cross-Chain)

For cross-chain messaging beyond the native bridge, Korai deploys Hyperlane's `Mailbox` contract. Hyperlane is fully permissionless to deploy — no approval from the Hyperlane team required. The Interchain Security Module (ISM) is configured per-application:

```rust
/// Korai Hyperlane ISM configuration
pub struct KoraiIsmConfig {
    /// Multisig ISM: require 3-of-5 validator signatures for cross-chain messages
    pub multisig_threshold: u8,  // default: 3
    pub multisig_validators: Vec<Address>,  // 5 trusted validators

    /// Aggregation ISM: require BOTH multisig AND optimistic
    pub require_optimistic: bool, // default: true
    pub optimistic_window_blocks: u64, // default: 100 (~40s on Korai)

    /// ZK ISM (future): require ZK light client proof of source chain state
    pub zk_enabled: bool, // default: false (enable when ZK ISMs mature)
}
```

#### Layer 3: IBC (Cosmos Ecosystem)

If Korai needs to interoperate with Cosmos SDK chains (e.g., for cross-ecosystem agent coordination), IBC-Solidity implementations exist for EVM chains. The IBC light client model — each chain maintains a light client of the counterparty and verifies Merkle proofs — provides trust-minimized cross-chain messaging without relying on external validator sets.

#### Layer 4: Intent-Based Bridge (Fast Path)

For time-sensitive cross-chain operations (agent needs to move KORAI to Base to pay for an MCP service), an intent-based bridge model (Across Protocol pattern) provides near-instant transfers:

1. Agent signals intent: "Move 500 KORAI from Korai to Base"
2. Solver on Base immediately delivers 500 KORAI-equivalent from their own capital
3. Solver claims reimbursement from Korai chain after the canonical bridge settles
4. Agent experiences sub-second transfer; solver bears the settlement delay risk

```rust
/// Cross-chain intent for agent transfers
pub struct CrossChainIntent {
    /// Source chain (Korai)
    pub source_chain_id: u64,
    /// Destination chain
    pub dest_chain_id: u64,
    /// Token to transfer
    pub token: Address,
    /// Amount in smallest unit
    pub amount: U256,
    /// Maximum fee the agent will pay (basis points)
    pub max_fee_bps: u16,  // default: 50 (0.5%)
    /// Deadline block on source chain
    pub deadline_block: u64,
    /// Agent's passport ID for identity verification
    pub passport_id: u256,
}
```

### Academic Foundations (Chain Selection)

- Azar, Y., Broder, A.Z., Karlin, A.R., and Upfal, E. (1999). "Balanced Allocations." *SIAM Journal on Computing*. — Theoretical foundations for load-balanced dispatch applicable to cross-chain routing.
- Buterin, V. (2021). "Endgame." *vitalik.ca*. — The modular blockchain thesis: separate execution, DA, and settlement. Korai as an Orbit L3 follows this architecture.
- Zamyatin, A. et al. (2021). "SoK: Communication Across Distributed Ledgers." *Financial Cryptography*. — Taxonomy of cross-chain protocols (relay chains, hash time-locks, notary schemes). Korai's layered approach combines native bridge (relay) with ISM (notary) and intent (hash time-lock variant).

---

## The Domain Plugin Pattern

Chain capabilities are implemented as domain-specific trait implementations, just like any other domain in Roko:

| Synapse Trait | Chain Domain Implementation | Coding Domain Equivalent |
|---|---|---|
| `Substrate` | `ChainSubstrate` — query on-chain Engrams via HDC precompile | `FileSubstrate` — JSONL persistence |
| `Scorer` | `ChainScorer` — 4-factor scoring (price, TVL, gas, health) | `CodeScorer` — complexity, coverage, coupling |
| `Gate` | `TxSimGate` — pre-flight tx simulation via mirage-rs | `CompileGate` — `cargo check` |
| `Gate` | `WalletGate` — position limits, approved assets | `TestGate` — `cargo test` |
| `Gate` | `VerifyChainGate` — post-execution state verification | `ClippyGate` — `cargo clippy` |
| `Router` | `CascadeRouter` — T0/T1/T2 with chain probes | `CascadeRouter` — T0/T1/T2 with code probes |
| `Policy` | `ChainPolicy` — subscribe to on-chain events | `FileWatchPolicy` — watch for file changes |

The cognitive loop, Neuro knowledge tiers, Daimon affect engine, Dreams consolidation, and C-Factor tracking all work automatically with these chain-specific trait implementations. No core changes are required to add chain capabilities — it is pure composition.

---

## Tier 6: Deferred Status

The Korai chain and all associated infrastructure (Daeji testnet, KORAI token, HDC precompile, on-chain contracts) are classified as **Tier 6 in the implementation priorities** — intentionally deferred and blocked by Tier 5 (Agent Mesh) completion.

The rationale is straightforward: solo agents and event-driven agents do NOT need the chain layer. The critical path for Roko's first release focuses on Tiers 1-5:

- **Tier 1**: Multi-provider model routing (in progress)
- **Tier 2**: Cognitive integration (Neuro, Daimon, Dreams — the differentiator)
- **Tier 3**: Agent platform (serve, events, MCP, daemon)
- **Tier 4**: Interfaces (TUI, web portal)
- **Tier 5**: Agent Mesh (P2P connections, knowledge backup/restore)

Only after Tier 5 delivers a working Agent Mesh does the Korai chain become relevant. The full chain-layer implementation plan is documented in `roko/tmp/implementation-plans/12b-chain-layer.md` (76 items across 11 sections).

---

## Capacity Planning

At Korai's target parameters:

- **400ms blocks** = 2.5 blocks/second
- **1,000 agents** posting 1 knowledge entry/day = 1,000 entries/day — easily manageable
- **10,000 agents** with higher posting frequency → needs capacity planning for block space and HDC index size
- **100,000 agents** → requires sharding or hierarchical indexing strategies

The HDC precompile's ~400 gas for top-K=20 similarity search keeps per-query costs low, but the total index size grows with the number of active knowledge entries. Three-tier search (Bloom filter fast reject → approximate coarse → exact top-K) bounds query latency even as the index grows.

---

## Academic Foundations

- [Grassé 1959] — Coined "stigmergy" (indirect coordination through shared environment modification). Foundation of the pheromone system.
- (Sumers et al., arXiv:2309.02427, 2023) — CoALA cognitive architecture. The 9-step cognitive loop that all agents run.
- (Ostrom 1990) — Governing the Commons. Foundation for understanding collective resource management and free-rider prevention.
- (Woolley et al., Science 330(6004), 2010) — Collective intelligence measurement. Basis for C-Factor metric.
- (Kanerva 2009, Cognitive Computation 1(2)) — Hyperdimensional computing. Foundation for HDC encoding used on-chain.
- (Friston et al., Nature Reviews Neuroscience 11(2), 2010) — Free-energy principle. Grounds the cybernetic feedback loop between chain perception and agent action.
- (Sims, Journal of Monetary Economics 50(3), 2003) — Rational inattention. Agents with finite processing capacity optimally allocate attention proportional to stakes.
- Reed's Law — The value of a network with groups scales as 2^N. Agent collectives that share knowledge create exponentially more value than isolated agents.

---

## Current Status and Gaps

**What exists today:**
- `roko-chain` crate (L1 Framework): `ChainClient` trait (read-only chain access), `ChainWallet` trait (sign/submit), `TxSimGate`, `WalletGate`, `MockChainClient`, `MockChainWallet`. 52 tests passing.
- `mirage-rs` app: In-process EVM simulator with fork mode, scenario engine, chain extensions (HDC index, knowledge, pheromone, agent coordination), HTTP API, JSON-RPC server. 141 tests passing.

**What does not exist yet:**
- Korai chain genesis and Daeji testnet deployment
- KORAI/DAEJI token contracts
- HDC precompile implementation
- ERC-8004 agent identity contracts
- Agent Mesh (WebSocket/Iroh P2P connections)
- On-chain job market (Spore/Sparrow)
- Reputation system (7-domain EMA)
- ISFR collective price discovery
- Clearing/settlement infrastructure
- Valhalla privacy layer (TEE, PSI, ZK proofs)

See `roko/tmp/implementation-plans/12b-chain-layer.md` for the full 76-item implementation plan.

---

## Cross-references

- See [01-korai-chain-spec.md](./01-korai-chain-spec.md) for Korai mainnet technical specification
- See [02-korai-token-economics.md](./02-korai-token-economics.md) for KORAI/DAEJI tokenomics
- See [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) for HDC vector precompile details
- See [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) for the development chain proxy
- See topic [00-architecture](../00-architecture/INDEX.md) for the core Synapse Architecture
- See topic [06-neuro](../06-neuro/INDEX.md) for knowledge types and HDC encoding (shared with on-chain)
- See topic [13-coordination](../13-coordination/INDEX.md) for stigmergy and mesh coordination
- See topic [14-identity-economy](../14-identity-economy/INDEX.md) for identity, reputation, and x402
