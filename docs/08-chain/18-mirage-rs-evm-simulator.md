# mirage-rs: In-Process EVM Simulator

> mirage-rs is a full EVM simulator that runs in-process alongside the agent. It emulates the Nunchi chain during development, provides transaction simulation for pre-flight checks, and enables deterministic testing of chain interactions. Built on revm, it supports fork mode (clone mainnet state), scenario replay, and Nunchi-specific chain extensions (HDC precompile, agent registry, reputation registry).


> **Implementation**: Shipping

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [01-nunchi-chain-spec.md](./01-nunchi-chain-spec.md), [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md)
**Key sources**: `roko/apps/mirage-rs/src/lib.rs`, `roko/tmp/implementation-plans/12b-chain-layer.md` §Q

---

## Abstract

mirage-rs is the development and simulation layer for Nunchi chain interactions. It provides a full EVM environment that runs in the same process as the Roko agent, eliminating network latency for chain queries and enabling deterministic testing. During development, mirage-rs acts as a Nunchi chain proxy — the agent interacts with it exactly as it would with the real Nunchi chain, but all state is local and ephemeral.

mirage-rs is built on revm (Rust EVM by Dragan Rakita), the same EVM implementation used by Foundry, Reth, and major L2 chains. It passes all official Ethereum test suites and provides identical execution semantics to production EVM nodes.

The crate lives at `apps/mirage-rs/` in the Roko workspace.

---

## Module Architecture

```
apps/mirage-rs/src/
├── lib.rs              # Core types, MirageError, TransactionRequest
├── cow/                # Copy-on-write state management
├── events/             # Event emission and subscription
├── fork/               # Fork mode: clone mainnet state at a block
├── integration/        # Integration test harness
├── provider/           # JSON-RPC provider implementation
├── rate_limit/         # RPC rate limiting for fork mode
├── replay/             # Transaction and scenario replay
├── resources/          # Resource management (state snapshots, cleanup)
├── rpc/                # JSON-RPC server (HTTP/WS)
├── scenario/           # Scenario definition and execution
│
│   Chain Extensions (feature-gated: "chain-extensions"):
├── chain/              # Nunchi chain state emulation
├── chain_rpc/          # Nunchi-specific RPC methods
├── http_api/           # REST API for management
└── roko_bridge/        # Bridge between roko-chain traits and mirage
```

### Core Types

```rust
/// Transaction request for mirage-rs execution.
pub struct TransactionRequest {
    pub from: Address,
    pub to: Option<Address>,
    pub value: U256,
    pub data: Bytes,
    pub gas_limit: Option<u64>,
    pub nonce: Option<u64>,
}

/// Error types with JSON-RPC error codes.
pub enum MirageError {
    /// Standard JSON-RPC errors.
    InvalidParams { message: String },          // -32602
    InternalError { message: String },          // -32603
    ExecutionReverted { message: String, data: Option<Bytes> }, // 3

    /// Mirage-specific errors.
    ForkError { message: String },
    SnapshotNotFound { id: String },
    ChainExtensionError { message: String },
}
```

---

## Operating Modes

### 1. Local Mode (Default)

A fresh EVM state with no external dependencies. Used for unit testing and isolated development.

```rust
let mirage = MirageInstance::new_local(MirageConfig {
    chain_id: 31337,            // Local chain ID
    block_time_ms: 400,         // Nunchi block time
    initial_balance: U256::from(1_000_000) * U256::from(10).pow(18), // 1M ETH
    enable_chain_extensions: true,
});
```

### 2. Fork Mode

Clone the state of a live chain at a specific block number. Used for testing against real protocol state (Uniswap pools, Aave markets, etc.).

```rust
let mirage = MirageInstance::new_fork(ForkConfig {
    rpc_url: "https://eth-mainnet.g.alchemy.com/v2/KEY",
    fork_block: Some(19_500_000),  // Fork at this block
    chain_id: 1,
    rate_limit_rps: 50,            // Don't hammer the RPC
    cache_dir: Some("~/.mirage/cache/"),
});
```

Fork mode uses copy-on-write state management: reads go to the remote RPC if the state is not cached locally, and writes go to local storage. This means the first access to a storage slot incurs RPC latency, but subsequent accesses are instant.

### 3. Scenario Mode

Execute a predefined sequence of transactions and state assertions. Used for integration testing and regression testing.

```rust
let scenario = Scenario::load("scenarios/flash_loan_attack.json")?;
let mirage = MirageInstance::new_scenario(scenario);
let results = mirage.execute_scenario().await?;
assert!(results.all_assertions_passed());
```

---

## Nunchi Chain Extensions

When the `chain-extensions` feature flag is enabled, mirage-rs emulates Nunchi-specific functionality:

### Emulated Registries

| Registry | Address | Emulation |
|---|---|---|
| Identity Registry | `0xA100` | In-memory ERC-8004 identity storage |
| Reputation Registry | `0xA200` | In-memory EMA scoring, feedback submission |
| Validation Registry | `0xA300` | In-memory work proof storage |

### Emulated Precompiles

| Precompile | Address | Emulation |
|---|---|---|
| HDC similarity | `0xA01` | Local `roko-primitives` HDC operations |
| HDC topk | `0xA01` | In-memory index with HNSW search |
| HDC bind | `0xA01` | Local XOR operation |
| HDC bundle | `0xA01` | Local majority vote |

The HDC precompile emulation uses the same code as `roko-primitives`, ensuring that similarity scores computed in mirage-rs match what the real Nunchi precompile would produce.

### Nunchi RPC Methods

mirage-rs implements the custom Nunchi RPC namespace:

```
nunchi_registerAgent(...)      → agent_id
nunchi_getAgent(id)            → AgentIdentity
nunchi_submitKnowledge(...)    → entry_hash
nunchi_queryKnowledge(...)     → Vec<(similarity, entry)>
nunchi_submitFeedback(...)     → tx_hash
nunchi_getReputation(id, dom)  → ReputationScore
```

These mirror the planned Nunchi chain RPC methods, allowing agent code to be developed and tested against the same API that will be used in production.

---

## Integration with roko-chain

The `roko_bridge` module provides implementations of `ChainClient` and `ChainWallet` backed by mirage-rs:

```rust
/// ChainClient implementation backed by mirage-rs.
pub struct MirageChainClient {
    mirage: Arc<MirageInstance>,
}

impl ChainClient for MirageChainClient {
    async fn block_number(&self) -> ChainResult<BlockNumber> {
        Ok(self.mirage.current_block_number())
    }

    async fn get_logs(...) -> ChainResult<Vec<LogEntry>> {
        Ok(self.mirage.query_logs(from, to, addresses, topics))
    }

    async fn eth_call(&self, request: &TxRequest, block: Option<BlockNumber>)
        -> ChainResult<CallResult>
    {
        let result = self.mirage.simulate(request, block)?;
        Ok(CallResult {
            output: result.output,
            gas_used: result.gas_used,
            success: result.success,
        })
    }
    // ... other methods
}
```

Agent code uses `ChainClient` and `ChainWallet` trait objects, so switching from mirage-rs to a live chain requires only changing the implementation — no agent code changes.

---

## State Snapshots

mirage-rs supports state snapshots for checkpoint/restore during testing:

```rust
// Take a snapshot
let snapshot_id = mirage.snapshot().await?;

// Execute some transactions...
mirage.execute(tx1).await?;
mirage.execute(tx2).await?;

// Restore to the snapshot (undo tx1 and tx2)
mirage.revert(snapshot_id).await?;
```

Snapshots are used by the gate pipeline to test transactions without permanently modifying state. The `TxSimGate` takes a snapshot, simulates the transaction, checks the results, and reverts — the actual state is never modified by simulation.

---

## Test Coverage

mirage-rs includes 141 tests covering:

- Basic EVM execution (transfers, contract deployment, function calls)
- Fork mode (mainnet state cloning, storage reads)
- Scenario replay (multi-transaction sequences with assertions)
- Copy-on-write state management
- Rate limiting for fork mode RPC calls
- Nunchi chain extensions (registry emulation, HDC precompile)
- Error handling (reverts, out-of-gas, invalid transactions)

---

## Simulation Fidelity Guarantees

mirage-rs uses revm (the same EVM used by Reth, Foundry/Anvil, and major L2 chains), providing bytecode-level execution equivalence with production EVM. However, simulation inherently diverges from mainnet in several dimensions. This section documents what mirage-rs CAN and CANNOT faithfully simulate.

### What mirage-rs Faithfully Simulates

| Aspect | Fidelity | Notes |
|---|---|---|
| **EVM bytecode execution** | Exact | revm passes all official Ethereum execution test suites |
| **Storage reads/writes** | Exact | Copy-on-write state from fork or local |
| **Contract-to-contract calls** | Exact | Full call stack with proper gas forwarding |
| **Gas metering (EVM opcodes)** | Exact | revm implements the official gas schedule |
| **Precompile behavior** | Exact (standard) | SHA-256, ECRECOVER, BN256, etc. |
| **Event emission** | Exact | Logs are captured and queryable |
| **Revert handling** | Exact | Revert reasons propagated correctly |
| **EIP-1559 basefee** | Configurable | Can be set to match mainnet or fixed |

### What mirage-rs CANNOT Simulate

| Aspect | Divergence | Impact | Mitigation |
|---|---|---|---|
| **MEV and transaction ordering** | No mempool competition | Simulation assumes isolated execution; mainnet places your tx among competing transactions. Sandwich attacks, frontrunning, and backrunning are invisible in simulation. | Use Flashbots MEV-Share or private mempool for production transactions. Compare simulation vs. actual execution post-facto. |
| **Block builder behavior** | No builder auction | Different validators/builders may reorder transactions. Priority fee ordering, private order flow, and MEV-Boost auctions are not modeled. | For MEV-sensitive operations, use bundle simulation (multiple txs in sequence) rather than single-tx simulation. |
| **Cross-block state changes** | Static snapshot | Oracle prices (Chainlink, TWAP), governance timelocks, and other time-dependent state change between simulation and execution. | Use short validity windows. Re-simulate immediately before execution. |
| **Gas price dynamics** | Fixed basefee | EIP-1559 basefee fluctuates per block. Simulations at a fixed basefee will diverge if blocks fill unexpectedly. | Set `basefee` to 110% of current mainnet basefee as safety margin. |
| **Private mempool (~40-60% of Ethereum block space)** | Invisible | Transactions routed through Flashbots, MEV-Boost, and private builders are invisible to simulation. | Accept that simulation is a lower bound on state competition. |
| **EVM implementation divergences** | Minor | OpDiffer (arXiv:2504.12034, 2025) found 26 bugs across 9 EVM implementations affecting ~7.21% of deployed contracts. revm may produce different results than go-ethereum for edge-case bytecodes. | Run differential tests against go-ethereum for critical contracts. |
| **Nunchi HDC precompile (in Stylus mode)** | Emulated locally | mirage-rs emulates HDC operations using `roko-primitives`; production uses Stylus WASM. Results are numerically identical but gas costs may differ slightly. | Calibrate gas estimates against Stylus benchmarks on testnet. |

### Simulation Confidence Score

mirage-rs computes a **simulation confidence score** for each simulated transaction, indicating how likely the simulation matches mainnet execution:

```rust
pub struct SimulationConfidence {
    /// Overall confidence [0.0, 1.0]
    pub score: f64,

    /// Individual confidence factors
    pub factors: ConfidenceFactors,
}

pub struct ConfidenceFactors {
    /// How recent is the forked state? (seconds since fork block)
    /// Fresh fork = high confidence; stale fork = low
    pub state_freshness: f64,

    /// Does the transaction interact with oracles or time-dependent state?
    /// No oracle interaction = 1.0; oracle-heavy = 0.5
    pub oracle_independence: f64,

    /// Is the transaction sensitive to ordering (e.g., AMM swap)?
    /// Ordering-independent = 1.0; MEV-exposed = 0.3
    pub ordering_independence: f64,

    /// Does the transaction involve cross-contract calls to unverified contracts?
    /// All known contracts = 1.0; unknown contracts = 0.5
    pub contract_verification: f64,

    /// Gas estimation confidence
    /// Simple transfer = 1.0; complex DeFi = 0.7
    pub gas_confidence: f64,
}

impl SimulationConfidence {
    pub fn compute(factors: &ConfidenceFactors) -> f64 {
        factors.state_freshness * 0.25
            + factors.oracle_independence * 0.25
            + factors.ordering_independence * 0.25
            + factors.contract_verification * 0.15
            + factors.gas_confidence * 0.10
    }
}
```

The confidence score is included in the `TxSimGate` output. If confidence falls below 0.5, the gate recommends re-simulation with a fresher fork state before execution.

---

## Simulation-to-Mainnet Migration Testing

### Differential Testing Framework

mirage-rs includes a differential testing framework that compares simulation results against actual mainnet execution:

```rust
/// Differential test: compare simulation vs. mainnet for the same transaction
pub struct DifferentialTest {
    /// Transaction hash on mainnet
    pub mainnet_tx_hash: TxHash,

    /// Fork block: simulate at the block BEFORE the mainnet tx
    pub fork_block: BlockNumber,

    /// Expected vs. actual comparison
    pub comparison: DiffComparison,
}

pub struct DiffComparison {
    /// Gas used: simulation vs. mainnet
    pub gas_diff: i64,
    pub gas_diff_pct: f64,

    /// Return data match
    pub return_data_matches: bool,

    /// State diff match (storage changes)
    pub state_diffs_match: bool,
    pub divergent_slots: Vec<(Address, U256)>,

    /// Event logs match
    pub logs_match: bool,

    /// Success/failure match
    pub status_matches: bool,
}
```

**Usage pattern**: After executing a real transaction on mainnet, replay it through mirage-rs at the pre-execution fork point. Compare results. If they diverge, investigate:
- Gas difference > 5%: likely an ordering/MEV issue
- State diff mismatch: likely a cross-block state change between simulation and execution
- Status mismatch (simulated success, mainnet revert): critical — indicates a simulation blindspot

### Invariant Testing Integration

mirage-rs supports Foundry-style invariant testing against Nunchi contracts:

```rust
/// Run invariant tests against Nunchi registry contracts
pub struct NunchiInvariantTest {
    pub mirage: MirageInstance,

    /// Invariants that must hold after every random transaction sequence
    pub invariants: Vec<NunchiInvariant>,
}

pub enum NunchiInvariant {
    /// Total agent count equals registry length
    AgentCountConsistency,
    /// Sum of all domain stakes <= total NUNCHI supply
    StakeSupplyBound,
    /// No agent has tier < required stake for that tier
    TierStakeConsistency,
    /// All reputation scores in [0.0, 1.0]
    ReputationBounds,
    /// Escrow balance >= sum of all active job budgets
    EscrowSolvency,
    /// Custom invariant (Solidity expression evaluated on-chain)
    Custom { expression: String },
}
```

### Formal Verification Pipeline

For critical Nunchi contracts (NUNCHI token, escrow, reputation registry), mirage-rs integrates with formal verification tools:

1. **Certora Prover**: Write CVL (Certora Verification Language) specifications for Nunchi contracts. Certora proves properties like "NUNCHI total supply equals sum of all balances after demurrage" across all possible execution paths.
2. **Halmos** (a16z): Symbolic execution for Foundry tests. Replace concrete fuzz values with symbolic variables — Halmos explores ALL possible inputs, not random samples.
3. **Kontrol** (Runtime Verification): Formal semantics via KEVM (K framework). Most rigorous option for verifying the EVM bytecode of Nunchi precompile contracts.

### Academic Foundations (Simulation)

- Rakita, D. (2022). revm: Rust Ethereum Virtual Machine. — The EVM implementation underlying mirage-rs.
- Yang, S. et al. (2025). "OpDiffer: Detecting Cross-Implementation Bugs in EVM via LLM-Guided Differential Testing." *arXiv:2504.12034*. — Differential EVM testing methodology; found 26 bugs across 9 implementations.
- Grieco, G. et al. (2020). "Echidna: Effective, Usable, and Fast Fuzzing for Smart Contracts." *ISSTA*. — Property-based testing for smart contracts; foundation for mirage-rs invariant testing.

---

## Current Status and Gaps

**Built:**
- Full EVM simulator with revm backend
- Fork mode with copy-on-write state management
- Scenario replay engine
- JSON-RPC provider
- Rate limiting for fork mode
- 141 passing tests

**Partially built (chain extensions):**
- Chain extension module structure exists
- Nunchi RPC stubs defined

**Not yet built (Tier 6):**
- HDC precompile emulation using `roko-primitives` (§Q1)
- Registry emulation (Identity, Reputation, Validation) (§Q2)
- Full Nunchi RPC method implementation (§Q3)
- `roko_bridge` implementations of `ChainClient` and `ChainWallet` (§Q4)
- Integration with `TxSimGate` for pre-flight simulation (§Q5)

---

## Cross-References

- See [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md) for the `ChainClient` and `ChainWallet` traits that mirage-rs implements
- See [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) for the HDC precompile that mirage-rs emulates
- See [06-erc-8004-registries.md](./06-erc-8004-registries.md) for the registries that mirage-rs emulates
- See [01-nunchi-chain-spec.md](./01-nunchi-chain-spec.md) for the chain parameters mirage-rs simulates
- See topic [04-gates](../04-verification/INDEX.md) for `TxSimGate` that uses mirage-rs for pre-flight checks
