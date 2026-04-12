# mirage-rs: In-Process EVM Simulator

> mirage-rs is a full EVM simulator that runs in-process alongside the agent. It emulates the Korai chain during development, provides transaction simulation for pre-flight checks, and enables deterministic testing of chain interactions. Built on revm, it supports fork mode (clone mainnet state), scenario replay, and Korai-specific chain extensions (HDC precompile, agent registry, reputation registry).


> **Implementation**: Shipping

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [01-korai-chain-spec.md](./01-korai-chain-spec.md), [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md)
**Key sources**: `roko/apps/mirage-rs/src/lib.rs`, `roko/tmp/implementation-plans/12b-chain-layer.md` §Q

---

## Abstract

mirage-rs is the development and simulation layer for Korai chain interactions. It provides a full EVM environment that runs in the same process as the Roko agent, eliminating network latency for chain queries and enabling deterministic testing. During development, mirage-rs acts as a Korai chain proxy — the agent interacts with it exactly as it would with the real Korai chain, but all state is local and ephemeral.

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
├── chain/              # Korai chain state emulation
├── chain_rpc/          # Korai-specific RPC methods
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
    block_time_ms: 400,         // Korai block time
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

## Korai Chain Extensions

When the `chain-extensions` feature flag is enabled, mirage-rs emulates Korai-specific functionality:

### Emulated Registries

| Registry | Address | Emulation |
|---|---|---|
| Identity Registry | `0xA100` | In-memory passport storage, soulbound minting |
| Reputation Registry | `0xA200` | In-memory EMA scoring, feedback submission |
| Validation Registry | `0xA300` | In-memory work proof storage |

### Emulated Precompiles

| Precompile | Address | Emulation |
|---|---|---|
| HDC similarity | `0xA01` | Local `roko-primitives` HDC operations |
| HDC topk | `0xA01` | In-memory index with HNSW search |
| HDC bind | `0xA01` | Local XOR operation |
| HDC bundle | `0xA01` | Local majority vote |

The HDC precompile emulation uses the same code as `roko-primitives`, ensuring that similarity scores computed in mirage-rs match what the real Korai precompile would produce.

### Korai RPC Methods

mirage-rs implements the custom Korai RPC namespace:

```
korai_registerPassport(...)   → passport_id
korai_getPassport(id)         → AgentPassport
korai_submitKnowledge(...)    → entry_hash
korai_queryKnowledge(...)     → Vec<(similarity, entry)>
korai_submitFeedback(...)     → tx_hash
korai_getReputation(id, dom)  → ReputationScore
```

These mirror the planned Korai chain RPC methods, allowing agent code to be developed and tested against the same API that will be used in production.

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
- Korai chain extensions (registry emulation, HDC precompile)
- Error handling (reverts, out-of-gas, invalid transactions)

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
- Korai RPC stubs defined

**Not yet built (Tier 6):**
- HDC precompile emulation using `roko-primitives` (§Q1)
- Registry emulation (Identity, Reputation, Validation) (§Q2)
- Full Korai RPC method implementation (§Q3)
- `roko_bridge` implementations of `ChainClient` and `ChainWallet` (§Q4)
- Integration with `TxSimGate` for pre-flight simulation (§Q5)

---

## Cross-references

- See [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md) for the `ChainClient` and `ChainWallet` traits that mirage-rs implements
- See [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) for the HDC precompile that mirage-rs emulates
- See [06-erc-8004-registries.md](./06-erc-8004-registries.md) for the registries that mirage-rs emulates
- See [01-korai-chain-spec.md](./01-korai-chain-spec.md) for the chain parameters mirage-rs simulates
- See topic [04-gates](../04-verification/INDEX.md) for `TxSimGate` that uses mirage-rs for pre-flight checks
