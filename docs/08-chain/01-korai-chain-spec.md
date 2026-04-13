# Korai Chain Specification

> A dedicated EVM chain for agent knowledge coordination: 400ms block time, agents as first-class citizens, HDC native precompile.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [00-vision-and-framing.md](./00-vision-and-framing.md)
**Key sources**: `refactoring-prd/04-knowledge-and-mesh.md`, `bardo-backup/prd/14-chain/00-architecture.md`, `bardo-backup/tmp/agent-chain/02-chain-architecture.md`, `roko/tmp/implementation-plans/12b-chain-layer.md`

---

## Abstract

Korai is a custom EVM chain purpose-built for agent coordination. Unlike general-purpose L1/L2 chains, Korai treats agents as first-class citizens with dedicated identity registries, reputation systems, and economic mechanisms designed for autonomous non-human actors. The chain features a native HDC (Hyperdimensional Computing) precompile that enables 10,240-bit vector similarity search at approximately 400 gas — making collective knowledge queries economically viable as on-chain operations.

Korai exists because existing EVM chains lack three critical capabilities: (1) native HDC vector operations at acceptable gas costs, (2) agent-specific identity standards (ERC-8004 Korai Passport), and (3) economic mechanisms (demurrage tokens, quality-weighted knowledge markets) designed for machine participants rather than human traders. The chain's 400ms block time enables sub-second coordination cycles that match the Gamma frequency (~5-15s) of the universal cognitive loop.

This document specifies the Korai mainnet architecture, block structure, state model, and RPC methods. The Daeji testnet mirrors this specification with a separate token (DAEJI) for development and testing.

---

## Chain Parameters

| Parameter | Korai Mainnet | Daeji Testnet |
|---|---|---|
| **Chain name** | Korai | Daeji |
| **Token** | KORAI | DAEJI |
| **Block time** | 400ms target | 400ms target |
| **Consensus** | Validator set (details TBD — Tier 6 design) | Single sequencer (development mode) |
| **EVM version** | Shanghai + Korai extensions | Shanghai + Korai extensions |
| **Native precompiles** | HDC similarity search (0xA01), Agent Registry (0xA02) | Same |
| **Block gas limit** | TBD (capacity planning needed at 10K+ agents) | 30M (Ethereum default) |
| **Chain ID** | TBD (to be registered) | TBD (testnet chain ID) |

### Block Structure

Korai blocks follow the standard Ethereum block structure with extensions for agent coordination. Each block header includes the standard fields (number, hash, parent hash, timestamp, state root, receipts root, logs bloom) plus Korai-specific metadata.

The Daeji chain specification describes a more advanced 5-phase block structure (Oracle → Accrual → Liquidation → Trading → Settlement), inspired by SpecPool-EVM architecture with Kauri consensus. This represents the full production design; initial deployment uses a simpler sequential block model.

### State Model

The Korai state model extends the standard EVM account model with agent-specific state:

1. **Standard EVM accounts** — EOAs and contracts, identical to Ethereum
2. **Agent Passport state** — ERC-721 soulbound NFTs storing agent identity, capabilities, reputation, and stake (see [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md))
3. **Knowledge entries** — HDC-encoded Engram summaries stored in the HDC index contract, queryable via the native precompile
4. **Pheromone state** — Typed coordination signals with decay counters, decremented each block
5. **Job market state** — Active BountySpecs, escrowed funds, job lifecycle states
6. **Reputation state** — Per-agent, per-domain EMA scores with decay timers

---

## RPC Methods

Korai extends the standard Ethereum JSON-RPC with custom methods for agent coordination. Standard methods (`eth_blockNumber`, `eth_getBlockByHash`, `eth_call`, `eth_sendRawTransaction`, etc.) work identically to Ethereum.

### Custom RPC Methods

| Method | Parameters | Returns | Description |
|---|---|---|---|
| `korai_registerPassport` | `(AgentPassport)` | `(passportId: uint256)` | Register a new agent on-chain. Mints a soulbound ERC-721 Korai Passport. |
| `korai_getPassport` | `(passportId: uint256)` | `(AgentPassport)` | Retrieve an agent's full passport including capabilities, reputation, tier, and stake. |
| `korai_queryAgentsByCapability` | `(capabilityBitmask: u64)` | `(Vec<passportId>)` | Find all agents with matching capabilities. |
| `korai_getReputation` | `(passportId: uint256, domain: string)` | `(ReputationScore)` | Retrieve per-domain reputation for an agent. |
| `korai_submitKnowledge` | `(KnowledgeEntry)` | `(entryHash: bytes32)` | Post an HDC-encoded knowledge entry to the chain. |
| `korai_queryKnowledge` | `(queryVector: bytes, topK: u32)` | `(Vec<KnowledgeResult>)` | HDC similarity search via the native precompile. |
| `korai_postJob` | `(BountySpec)` | `(jobId: uint256)` | Post a job to the Spore market with escrowed budget. |
| `korai_getJobStatus` | `(jobId: uint256)` | `(JobStatus)` | Query the lifecycle state of a job. |
| `korai_submitBid` | `(SparrowBid)` | `(bidId: uint256)` | Submit a bid on an open job. |
| `korai_agentHeartbeat` | `(passportId: uint256, status: bytes)` | `()` | Publish agent liveness heartbeat. |
| `korai_getIsfrRate` | `(marketId: string)` | `(IsfrAggregate)` | Query the latest ISFR collective rate for a market. |

### mirage-rs RPC Compatibility

During development, mirage-rs implements all custom `korai_*` methods as local in-process operations. The existing `mirage_*` namespace methods continue to work for EVM-level operations (snapshots, time manipulation, account impersonation). When transitioning to the real Korai chain, agents switch their RPC endpoint — no code changes are needed because the API surface is identical.

---

## Chain Intelligence Pipeline

The chain intelligence architecture describes how agents perceive on-chain activity. Originally specified across five crates in the legacy architecture (now renamed: witness, triage, protocol-state, chain-scope, stream-api), this pipeline maps to the Roko Synapse Architecture as follows:

```
[Korai Node / mirage-rs]
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

This eliminates latency on fast chains (Korai at 400ms blocks would batch 2-3 blocks per 5s Gamma tick otherwise). The Gamma oscillator remains as a heartbeat health check but does not gate chain perception.

---

## Deployment Modes

| Mode | Description | Use Case |
|---|---|---|
| **Full** | All chain intelligence components, embedded in agent runtime | Default for chain agents |
| **Light** | Witness + triage only, no protocol state cache | Resource-constrained agents |
| **Archive** | Full + extended retention + historical queries | Agents needing deep historical analysis |

Memory overhead per chain instance: MIDAS-R ~128KB, DDSketch ~2KB, Count-Min Sketch ~32-64KB. For 10 chain instances: ~1.6MB total — negligible.

---

## L2/L3 Deployment Architecture

### Preferred Deployment: Arbitrum Orbit L3

Rather than running as a sovereign L1, Korai's preferred deployment model is as an **Arbitrum Orbit L3 chain** settling to Arbitrum One (L2), which itself settles to Ethereum L1. This provides three layers of security inheritance:

```
┌─────────────────────────┐
│     Ethereum L1          │  Security: ~$60B staked ETH
│     (Settlement)         │  Finality: ~15 min (6 blocks)
├─────────────────────────┤
│     Arbitrum One (L2)    │  Security: Nitro fraud proofs + BoLD
│     (DA + Settlement)    │  Finality: 7 days (optimistic), minutes (ZK)
├─────────────────────────┤
│     Korai Chain (L3)     │  Custom: HDC precompile, agent registries
│     (Execution)          │  Block time: 400ms, KORAI gas token
└─────────────────────────┘
```

#### Orbit Configuration for Korai

```rust
/// Orbit chain configuration for Korai
pub struct KoraiOrbitConfig {
    /// Chain name
    pub chain_name: String,  // "Korai"

    /// Gas token: KORAI (not ETH)
    pub gas_token: Address,  // KORAI ERC-20 address on parent chain

    /// Block time target
    pub block_time_ms: u64,  // 400

    /// Data availability mode
    pub da_mode: DaMode,  // AnyTrust for lower cost, Rollup for max security

    /// Stylus enabled for HDC precompiles
    pub stylus_enabled: bool,  // true

    /// Custom precompile addresses
    pub precompiles: Vec<PrecompileConfig>,

    /// Sequencer mode
    pub sequencer: SequencerMode,
}

pub enum DaMode {
    /// Full rollup: all data on Arbitrum One (L2) → Ethereum L1
    /// Maximum security, higher cost
    Rollup,
    /// AnyTrust: data availability committee (DAC) + L1 fallback
    /// Lower cost, trust the DAC (N-of-M honest assumption)
    AnyTrust { committee_size: usize, threshold: usize },
    /// Celestia: post data to Celestia DA layer via Blobstream
    /// Modular DA, separate security assumption
    Celestia { namespace: [u8; 32] },
}

pub enum SequencerMode {
    /// Single sequencer (development, early production)
    Centralized { sequencer_address: Address },
    /// Shared sequencer (Espresso, Astria) for cross-chain atomic inclusion
    Shared { sequencer_url: String },
    /// Decentralized sequencer set (full decentralization)
    Decentralized { validator_set: Vec<Address>, consensus: ConsensusType },
}
```

#### Why Orbit Over OP Stack?

| Property | Arbitrum Orbit | OP Stack |
|---|---|---|
| **Stylus (WASM contracts)** | Native support | Not available |
| **Custom gas token** | Supported | Supported |
| **Fraud proofs** | WASM-based (Nitro), permissionless (BoLD) | MIPS-based (Cannon), permissionless |
| **ZK path** | In development | OP Succinct (Succinct Labs) |
| **Custom precompiles** | Via Stylus (Rust → WASM) | Requires op-geth fork |
| **DA flexibility** | Rollup, AnyTrust, Celestia | Rollup, Celestia, EigenDA |

The decisive factor is **Stylus**: Korai's HDC precompile requires high-performance bitwise operations that are 10-100x cheaper in WASM than in EVM bytecode. Stylus provides this natively on Orbit chains. The OP Stack would require forking op-geth to add custom precompiles — a higher maintenance burden.

### Consensus Evolution Roadmap

Korai's consensus evolves through three phases:

**Phase 1 — Centralized Sequencer (Launch)**:
A single Korai-operated sequencer produces blocks. This is the standard model for new Orbit chains and all major L2s at launch. The sequencer cannot steal funds (fraud proofs protect users) but can censor transactions and extract MEV.

**Phase 2 — Shared Sequencer (12-18 months post-launch)**:
Integrate with a shared sequencing layer (Espresso Sequencer or Astria). Shared sequencing provides:
- Cross-chain atomic inclusion (agents on Korai and Base can have transactions included atomically)
- Censorship resistance (no single party controls ordering)
- MEV redistribution (MEV extracted from Korai flows back to validators/stakers)

**Phase 3 — Decentralized Sequencer Set (24+ months)**:
Korai-specific validator set using CometBFT-style consensus adapted for the Orbit framework. Validators are high-tier Korai Passport holders (Protocol and Sovereign) who stake KORAI as collateral. This provides full decentralization while maintaining 400ms block times.

### EigenLayer AVS Integration

Korai can optionally leverage EigenLayer's Actively Validated Services (AVS) framework for specific validation tasks that benefit from Ethereum's restaked security:

```rust
/// EigenLayer AVS configuration for Korai validation tasks
pub struct KoraiAvsConfig {
    /// AVS tasks that Korai delegates to EigenLayer operators
    pub tasks: Vec<AvsTask>,

    /// Minimum restaked ETH required per operator
    pub min_operator_stake: U256,  // default: 32 ETH

    /// Slashing conditions
    pub slashing: SlashingConfig,
}

pub enum AvsTask {
    /// Validate HDC similarity search results
    /// Operators re-compute top-K and verify against submitted results
    HdcSearchValidation {
        challenge_window_blocks: u64,  // 100 blocks (~40s)
        slash_amount_bps: u16,         // 1000 (10% of operator stake)
    },

    /// Validate knowledge entry quality scores
    /// Operators independently score entries and compare
    KnowledgeQualityValidation {
        min_validators: u8,  // 3
        consensus_threshold: f64,  // 0.67 (2/3 agreement)
    },

    /// Validate clearing certificate KKT conditions
    /// Operators verify the QP solution is optimal
    ClearingCertificateValidation {
        verification_timeout_blocks: u64,  // 50 blocks (~20s)
    },
}
```

EigenLayer AVS is particularly relevant for Korai because it allows Korai to borrow Ethereum's >$7B restaked security for critical validation tasks without requiring Korai-native staking to reach the same security level. As of April 2025, EigenLayer supports 39 live AVSs and has rebranded AVS from "Actively Validated Services" to "Autonomous Verifiable Services" — explicitly embracing agentic systems.

### Alternative: Cosmos SDK Appchain with IBC

If the Korai community decides that full sovereignty (own validator set, own consensus, no L1 dependency) outweighs Ethereum security inheritance, a Cosmos SDK appchain is the secondary option:

**Advantages over Orbit**:
- Full sovereignty: no dependency on Arbitrum or Ethereum
- IBC native: instant interop with 200+ Cosmos chains
- Custom modules in Go (vs. Solidity/Rust smart contracts)
- CometBFT deterministic finality (~6s, tunable)

**Disadvantages**:
- Must bootstrap own validator set (or use Interchain Security from Cosmos Hub)
- No Stylus — HDC precompile would be a native Go module (fast, but Go vs. Rust)
- Less DeFi composability with Ethereum ecosystem
- Smaller auditing/tooling ecosystem

```rust
/// Cosmos SDK module interface for HDC operations (Go)
/// Would be implemented as a native Cosmos SDK keeper
pub struct HdcModuleConfig {
    /// Module name in the Cosmos SDK app
    pub module_name: String,  // "korai_hdc"

    /// Maximum vectors in on-chain index
    pub max_index_size: u64,  // 1_000_000

    /// Query gas cost per vector comparison
    pub gas_per_comparison: u64,  // 10 (Cosmos gas, not EVM gas)
}
```

The Orbit L3 approach is preferred for Korai's initial deployment due to Ethereum security inheritance and Stylus support. The Cosmos path remains viable for a future where agent coordination spans beyond the EVM ecosystem.

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

Assumptions: ~100 protocols, ~1,000 watched addresses, ~7,500 blocks/day at Korai's 400ms block time, ~10% filter hit rate. If storage cap is reached, triage retention halves automatically.

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
- Korai genesis configuration
- Daeji testnet deployment
- HDC precompile implementation
- Custom `korai_*` RPC methods on real chain (mirage-rs stubs exist)
- Chain intelligence pipeline (ChainWitness, triage, protocol state, chain scope)
- 5-phase block structure from Daeji specification

---

## Cross-References

- See [00-vision-and-framing.md](./00-vision-and-framing.md) for why a dedicated chain exists
- See [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) for HDC precompile details
- See [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md) for the event watching pipeline
- See [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md) for the triage and anomaly detection pipeline
- See [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) for the development chain proxy
- See topic [00-architecture](../00-architecture/INDEX.md) for the core Synapse Architecture
