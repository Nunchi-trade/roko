# Nunchi Chain Specification

> A dedicated EVM chain with simplex consensus for agent knowledge coordination: 50ms block time, agents as first-class citizens, ERC-8004 native identity, HDC native precompile.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [00-vision-and-framing.md](./00-vision-and-framing.md)
**Key sources**: `refactoring-prd/04-knowledge-and-mesh.md`, `bardo-backup/prd/14-chain/00-architecture.md`, `bardo-backup/tmp/agent-chain/02-chain-architecture.md`, `roko/tmp/implementation-plans/12b-chain-layer.md`

---

## Abstract

Nunchi is a custom EVM chain with simplex consensus, purpose-built for agent coordination. Unlike general-purpose L1/L2 chains, Nunchi treats agents as first-class citizens with native ERC-8004 identity, reputation systems, and economic mechanisms designed for autonomous non-human actors. The chain features a native HDC (Hyperdimensional Computing) precompile that enables 10,240-bit vector similarity search at approximately 400 gas — making collective knowledge queries economically viable as on-chain operations.

Nunchi exists because existing EVM chains lack three critical capabilities: (1) native HDC vector operations at acceptable gas costs, (2) native ERC-8004 identity implemented to its full spec, and (3) economic mechanisms (demurrage tokens, quality-weighted knowledge markets) designed for machine participants rather than human traders. The chain's 50ms block time enables sub-second coordination cycles that match the Gamma frequency (~5-15s) of the universal cognitive loop.

This document specifies the Nunchi mainnet architecture, block structure, state model, and RPC methods. The Nunchi Testnet testnet mirrors this specification with a separate token (NUNCHI_TEST) for development and testing.

---

## Chain Parameters

| Parameter | Nunchi Mainnet | Nunchi Testnet Testnet |
|---|---|---|
| **Chain name** | Nunchi | Nunchi Testnet |
| **Token** | NUNCHI | NUNCHI_TEST |
| **Block time** | 50ms target | 50ms target |
| **Consensus** | Simplex consensus | Simplex consensus (single sequencer for dev) |
| **EVM version** | Shanghai + Nunchi extensions | Shanghai + Nunchi extensions |
| **Native precompiles** | HDC similarity search (0xA01), Agent Registry (0xA02) | Same |
| **Block gas limit** | TBD (capacity planning needed at 10K+ agents) | 30M (Ethereum default) |
| **Chain ID** | TBD (to be registered) | TBD (testnet chain ID) |

### Block Structure

Nunchi blocks follow the standard Ethereum block structure with extensions for agent coordination. Each block header includes the standard fields (number, hash, parent hash, timestamp, state root, receipts root, logs bloom) plus Nunchi-specific metadata.

The Nunchi Testnet chain specification describes a more advanced 5-phase block structure (Oracle → Accrual → Liquidation → Trading → Settlement), inspired by SpecPool-EVM architecture with Kauri consensus. This represents the full production design; initial deployment uses a simpler sequential block model.

### State Model

The Nunchi state model extends the standard EVM account model with agent-specific state:

1. **Standard EVM accounts** — EOAs and contracts, identical to Ethereum
2. **Agent identity state** — Native ERC-8004 identity storing agent capabilities, reputation, and stake (see [06-erc-8004-registries.md](./06-erc-8004-registries.md))
3. **Knowledge entries** — HDC-encoded Engram summaries stored in the HDC index contract, queryable via the native precompile
4. **Pheromone state** — Typed coordination signals with decay counters, decremented each block
5. **Job market state** — Active BountySpecs, escrowed funds, job lifecycle states
6. **Reputation state** — Per-agent, per-domain EMA scores with decay timers

---

## RPC Methods

Nunchi extends the standard Ethereum JSON-RPC with custom methods for agent coordination. Standard methods (`eth_blockNumber`, `eth_getBlockByHash`, `eth_call`, `eth_sendRawTransaction`, etc.) work identically to Ethereum.

### Custom RPC Methods

| Method | Parameters | Returns | Description |
|---|---|---|---|
| `nunchi_registerAgent` | `(AgentIdentity)` | `(agentId: uint256)` | Register a new agent on-chain via native ERC-8004. |
| `nunchi_getAgent` | `(agentId: uint256)` | `(AgentIdentity)` | Retrieve an agent's full identity including capabilities, reputation, tier, and stake. |
| `nunchi_queryAgentsByCapability` | `(capabilityBitmask: u64)` | `(Vec<agentId>)` | Find all agents with matching capabilities. |
| `nunchi_getReputation` | `(agentId: uint256, domain: string)` | `(ReputationScore)` | Retrieve per-domain reputation for an agent. |
| `nunchi_submitKnowledge` | `(KnowledgeEntry)` | `(entryHash: bytes32)` | Post an HDC-encoded knowledge entry to the chain. |
| `nunchi_queryKnowledge` | `(queryVector: bytes, topK: u32)` | `(Vec<KnowledgeResult>)` | HDC similarity search via the native precompile. |
| `nunchi_postJob` | `(BountySpec)` | `(jobId: uint256)` | Post a job to the ERC-8183 market with escrowed budget. |
| `nunchi_getJobStatus` | `(jobId: uint256)` | `(JobStatus)` | Query the lifecycle state of a job. |
| `nunchi_submitBid` | `(SparrowBid)` | `(bidId: uint256)` | Submit a bid on an open job. |
| `nunchi_agentHeartbeat` | `(agentId: uint256, status: bytes)` | `()` | Publish agent liveness heartbeat. |
| `nunchi_getIsfrRate` | `(marketId: string)` | `(IsfrAggregate)` | Query the latest ISFR collective rate for a market. |

### mirage-rs RPC Compatibility

During development, mirage-rs implements all custom `nunchi_*` methods as local in-process operations. The existing `mirage_*` namespace methods continue to work for EVM-level operations (snapshots, time manipulation, account impersonation). When transitioning to the real Nunchi chain, agents switch their RPC endpoint — no code changes are needed because the API surface is identical.

---

## Chain Intelligence Pipeline

The chain intelligence architecture describes how agents perceive on-chain activity. Originally specified across five crates in the legacy architecture (now renamed: witness, triage, protocol-state, chain-scope, stream-api), this pipeline maps to the Roko Synapse Architecture as follows:

```
[Nunchi Node / mirage-rs]
    | WS subscription (newHeads, logs)
    v
[ChainWitness] — Substrate.query() equivalent for on-chain data
    -- Block arrives → Binary Fuse filter pre-screening (O(1), ~10ns)
    ---- miss → update gas metrics, skip
    ---- hit  → fetch full block + receipts
    v
[Triage Pipeline] — Scorer.score() for chain events
    -- Stage 1: Rule-based fast filters (known MEV, value thresholds)
    -- Stage 2: Statistical anomaly (MIDAS-R, DDSketch)
    -- Stage 3: Contextual enrichment (protocol state, ABI, history)
    -- Stage 4: HDC fingerprint + Bayesian surprise scoring
    v (fan-out by curiosity score)
    |-- score > 0.8  → TriageAlert → Router escalates to T2
    |-- 0.5-0.8      → ChainEvent → standard processing
    |-- 0.2-0.5      → silent protocol state update
    +-- < 0.2        → discard (audit log only)
```

This pipeline implements the **cybernetic feedback loop** described by Friston et al. (2010): the agent maintains a generative model of its chain environment, acts to reduce uncertainty, and updates its model based on outcomes. The prediction error at the Gamma gate is the variational free energy that drives the cycle.

### Event-Driven Perception

Block ingestion runs continuously in its own async task — it is not clock-gated. Rather than processing on a fixed Gamma timer, perception is block-arrival-triggered:

```rust
select! {
    block = chain_receiver.recv() => { run_triage(block); }
    _ = theta_timer.tick() => { run_cognition(); }
    _ = delta_timer.tick() => { run_consolidation(); }
}
```

This eliminates latency on fast chains (Nunchi at 50ms blocks would batch 2-3 blocks per 5s Gamma tick otherwise). The Gamma oscillator remains as a heartbeat health check but does not gate chain perception.

---

## Deployment Modes

| Mode | Description | Use Case |
|---|---|---|
| **Full** | All chain intelligence components, embedded in agent runtime | Default for chain agents |
| **Light** | Witness + triage only, no protocol state cache | Resource-constrained agents |
| **Archive** | Full + extended retention + historical queries | Agents needing deep historical analysis |

Memory overhead per chain instance: MIDAS-R ~128KB, DDSketch ~2KB, Count-Min Sketch ~32-64KB. For 10 chain instances: ~1.6MB total — negligible.

---

## Deployment Architecture

Nunchi is a **sovereign EVM L1** — a fork of reth/revm with Simplex consensus and custom native precompiles registered at genesis. There is no settlement dependency on Arbitrum, Ethereum, or any parent chain.

### Execution Layer

Nunchi forks the reth execution client (Rust Ethereum) and the revm interpreter. Custom precompiles (HDC similarity search at 0xA01, Agent Registry at 0xA02) are registered in the genesis configuration and execute as native Rust code — no WASM layer, no Stylus, no op-geth modifications required.

```rust
/// Nunchi genesis precompile registration
pub struct NunchiPrecompiles {
    /// HDC similarity search precompile
    /// Registered at 0xA01; implements hdc_similarity, hdc_topk, hdc_bind, hdc_bundle
    pub hdc: HdcPrecompile,

    /// Agent registry precompile
    /// Registered at 0xA02; implements ERC-8004 identity operations natively
    pub agent_registry: AgentRegistryPrecompile,
}
```

Native precompiles execute at the lowest possible cost: no VM context switches, no serialization overhead, direct access to SIMD POPCNT instructions. The top-K similarity search over 100K index entries costs approximately 400 gas.

### Consensus: Simplex

Nunchi uses **Simplex consensus** (Chan & Pass, IACR 2023/463) — a BFT protocol designed for high-throughput permissioned validator sets. Simplex achieves safety and liveness under standard BFT assumptions (fewer than 1/3 Byzantine validators) with a single-phase commit path that enables consistent sub-100ms block times.

The 50ms block time target is achieved by co-locating validators in the same data center region. This is the same operational model used by Hyperliquid for their L1: tight geographic co-location eliminates network round-trip latency as the bottleneck, making the consensus protocol's single-phase path reliably fast.

### Consensus Roadmap

Nunchi's validator set evolves through three phases:

**Phase 1 — Co-located Tokyo validators (Launch)**:
A small set of Nunchi-operated validators co-located in Tokyo data centers produce blocks at 50ms intervals. Co-location eliminates propagation latency, making Simplex's single-phase commit path reliable. This is not a centralized sequencer — multiple validators participate in consensus — but the set is intentionally small and geographically concentrated for performance.

**Phase 2 — Expanding validator set (12-24 months)**:
Admit additional validators beyond Tokyo, accepting slightly higher latency (100-200ms) in exchange for broader geographic distribution and reduced operational concentration. Simplex's BFT safety guarantees hold as long as fewer than 1/3 of validators are Byzantine, regardless of their locations.

**Phase 3 — Fully decentralized (24+ months)**:
Open validator admission to high-tier ERC-8004 identity holders (Protocol and Sovereign tiers) who stake NUNCHI as collateral. Block times stabilize around network propagation latency rather than physical co-location. Full decentralization with the same Simplex consensus protocol — no consensus layer change required.

### Alternative: Cosmos SDK Appchain with IBC

If the Nunchi community decides that IBC-native interoperability with the Cosmos ecosystem outweighs the EVM composability advantages of a sovereign EVM L1, a Cosmos SDK appchain is the secondary option:

**Advantages over sovereign EVM L1**:
- IBC native: instant interop with 200+ Cosmos chains without external bridge infrastructure
- Custom modules in Go with native CometBFT deterministic finality (~6s, tunable)
- Cosmos Hub Interchain Security available for bootstrapping validator security

**Disadvantages**:
- HDC precompile would be a native Go Cosmos SDK keeper module rather than a native reth precompile
- Less DeFi composability with the Ethereum ecosystem (no ERC-20 bridging without IBC-Solidity)
- Smaller auditing and tooling ecosystem than EVM

The sovereign EVM L1 approach is preferred for Nunchi's initial deployment because native reth precompiles provide the lowest possible HDC gas costs and the EVM ecosystem provides the broadest DeFi composability. The Cosmos path remains viable if agent coordination requirements shift toward Cosmos-native chains.

---

## Storage Budget (Per Chain)

| Component | Per Day | 7-Day Default | 30-Day |
|---|---|---|---|
| Triage filtered events | 2-8 MB | 14-56 MB | 60-240 MB |
| Protocol state (snapshots) | 0.5 MB | 3.5 MB | 15 MB |
| Protocol state (deltas) | 0.2 MB | 1.4 MB | 6 MB |
| Seen block bitmap (Roaring) | ~2 KB | ~14 KB | ~60 KB |
| HDC codebook + bundles | ~200 KB | ~200 KB | ~200 KB |
| MIDAS-R sketch | ~100 KB | ~100 KB | ~100 KB |
| DDSketch distributions | ~50 KB | ~50 KB | ~50 KB |
| **Total per chain** | **~3-9 MB/day** | **~20-62 MB** | **~90-270 MB** |

Assumptions: ~100 protocols, ~1,000 watched addresses, ~1,728,000 blocks/day at Nunchi's 50ms block time, ~10% filter hit rate. If storage cap is reached, triage retention halves automatically.

---

## Non-EVM Chain Adaptation

The chain intelligence layer is EVM-specific by default, but the `ChainAdapter` trait abstracts chain-specific operations for potential future support of non-EVM chains:

```rust
#[async_trait]
pub trait ChainAdapter: Send + Sync {
    /// Fetch a block and its events, chain-specific.
    async fn fetch_block(&self, block_id: BlockId) -> Result<NormalizedBlock>;

    /// Extract filter keys from a block header for pre-screening.
    /// EVM: address + topics. Solana: program ID + account addresses.
    fn extract_filter_keys(&self, header: &BlockHeader) -> Vec<u64>;

    /// Decode events from raw transaction data.
    /// EVM: ABI decoding. Solana: Anchor IDL. Cosmos: protobuf.
    fn decode_events(&self, raw: &RawTxData) -> Vec<DecodedEvent>;

    /// Read protocol state from chain.
    /// EVM: eth_call. Solana: getAccountInfo. Cosmos: ABCI query.
    async fn read_state(&self, address: &ChainAddress) -> Result<RawState>;
}
```

The Binary Fuse filter works across all chains because it operates on u64 hashes of filter keys, not the keys themselves.

---

## Academic Foundations

- Bloom, B.H. (1970). "Space/time trade-offs in hash coding with allowable errors." *Communications of the ACM*, 13(7). — The probabilistic data structure underlying Ethereum's logsBloom and informing the Binary Fuse filter.
- Lemire, D. et al. (2022). "Binary Fuse Filters: Fast and Smaller Than Xor Filters." *Journal of Experimental Algorithmics*. — The O(1) pre-screening filter at 8.7 bits/entry used by ChainWitness.
- (Friston et al., Nature Reviews Neuroscience 11(2), 2010) — Free-energy principle grounding the cybernetic feedback loop.
- (Sims, Journal of Monetary Economics 50(3), 2003) — Rational inattention: agents with finite processing capacity optimally allocate attention proportional to stakes.
- (Vitter, ACM TOMS 11(1), 1985) — Reservoir sampling for statistically representative block sampling on passive chains.
- Wood, G. (2014). Ethereum Yellow Paper. Section 4.3. — logsBloom definition informing filter design.
- Bhat et al. (2023). "MIDAS-R: Streaming multi-aspect anomaly detection." — Real-time anomaly detection in the triage pipeline.

---

## Current Status and Gaps

**Built:**
- `roko-chain` crate with `ChainClient` and `ChainWallet` traits, `TxSimGate`, `WalletGate`, mock implementations (52 tests)
- `mirage-rs` with fork state, JSON-RPC, chain extensions, scenario engine (141 tests)

**Not yet built (Tier 6, deferred):**
- Nunchi genesis configuration
- Nunchi Testnet testnet deployment
- HDC precompile implementation
- Custom `nunchi_*` RPC methods on real chain (mirage-rs stubs exist)
- Chain intelligence pipeline (ChainWitness, triage, protocol state, chain scope)
- 5-phase block structure from Nunchi Testnet specification

---

## Cross-References

- See [00-vision-and-framing.md](./00-vision-and-framing.md) for why a dedicated chain exists
- See [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) for HDC precompile details
- See [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md) for the event watching pipeline
- See [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md) for the triage and anomaly detection pipeline
- See [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) for the development chain proxy
- See topic [00-architecture](../00-architecture/INDEX.md) for the core Synapse Architecture
