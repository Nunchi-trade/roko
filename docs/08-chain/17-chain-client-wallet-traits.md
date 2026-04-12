# ChainClient and ChainWallet Traits

> Two async traits define the interface between Roko agents and any EVM-compatible chain: `ChainClient` for reading chain state (blocks, logs, storage, eth_call) and `ChainWallet` for writing (signing, submitting, and waiting for transactions). Implementations exist for live RPC nodes, mirage-rs simulation, and mock testing.

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [01-korai-chain-spec.md](./01-korai-chain-spec.md)
**Key sources**: `roko/crates/roko-chain/src/client.rs`, `roko/crates/roko-chain/src/wallet.rs`, `roko/crates/roko-chain/src/types.rs`

---

## Abstract

The `roko-chain` crate defines two async traits that provide a clean abstraction over EVM-compatible blockchain interactions. `ChainClient` handles read operations (querying blocks, logs, storage, and making simulated calls). `ChainWallet` handles write operations (signing transactions, submitting them, and waiting for receipts). Both traits are object-safe and `Send + Sync`, enabling use in multi-threaded async runtimes.

The trait design follows Roko's composability principle: the same agent code works against a live Ethereum node, a forked mirage-rs simulation, or a mock implementation for testing. The chain is a domain plugin, and these traits are the plugin interface.

---

## ChainClient Trait

The `ChainClient` trait provides read-only access to chain state:

```rust
#[async_trait::async_trait]
pub trait ChainClient: Send + Sync {
    /// Current block number.
    async fn block_number(&self) -> ChainResult<BlockNumber>;

    /// Get block header by number.
    async fn get_block_header(&self, number: BlockNumber) -> ChainResult<ChainHeader>;

    /// Get transaction receipt.
    async fn get_receipt(&self, tx_hash: &TxHash) -> ChainResult<Option<Receipt>>;

    /// Get logs matching a filter.
    async fn get_logs(
        &self,
        from_block: BlockNumber,
        to_block: BlockNumber,
        addresses: &[Address],
        topics: &[Vec<B256>],
    ) -> ChainResult<Vec<LogEntry>>;

    /// Read raw storage at a contract address and slot.
    async fn get_storage_at(
        &self,
        address: Address,
        slot: U256,
        block: Option<BlockNumber>,
    ) -> ChainResult<U256>;

    /// Simulate a call without submitting a transaction.
    async fn eth_call(
        &self,
        request: &TxRequest,
        block: Option<BlockNumber>,
    ) -> ChainResult<CallResult>;

    /// Chain ID.
    async fn chain_id(&self) -> ChainResult<u64>;

    /// Human-readable name for this client (e.g., "ethereum-mainnet", "mirage-fork").
    fn name(&self) -> &str;
}
```

### Key Design Decisions

1. **Block number as `u64`**: Ethereum block numbers fit comfortably in u64. Using a native type avoids the overhead of U256 for the most common parameter.

2. **`get_logs` with topic arrays**: Follows the Ethereum JSON-RPC `eth_getLogs` specification. Topics are arrays of arrays, supporting OR logic within a position and AND logic across positions.

3. **`eth_call` with `TxRequest`**: Simulates a transaction without submitting it. Essential for pre-flight checks: "what would happen if I submitted this transaction?"

4. **`Option<BlockNumber>` for historical queries**: When `None`, queries run against the latest block. When `Some(n)`, queries run against historical state at block n (requires archive node).

---

## ChainWallet Trait

The `ChainWallet` trait provides write access — the ability to sign and submit transactions:

```rust
#[async_trait::async_trait]
pub trait ChainWallet: Send + Sync {
    /// The wallet's address.
    fn address(&self) -> Address;

    /// Current balance in native token (ETH, KORAI, etc.).
    async fn balance(&self) -> ChainResult<U256>;

    /// Current nonce.
    async fn nonce(&self) -> ChainResult<u64>;

    /// Sign and submit a transaction. Returns the transaction hash.
    async fn sign_and_submit(&self, request: TxRequest) -> ChainResult<TxHash>;

    /// Wait for a transaction to be included in a block.
    /// Returns the receipt.
    async fn wait_for_receipt(
        &self,
        tx_hash: &TxHash,
        timeout_ms: u64,
    ) -> ChainResult<Receipt>;

    /// Human-readable name for this wallet.
    fn name(&self) -> &str;
}
```

### Three Custody Modes

The `ChainWallet` trait can be implemented with three custody modes, each with different security properties:

| Mode | Implementation | Key Storage | Use Case |
|---|---|---|---|
| **Delegation (Enclave)** | TEE-backed wallet | Keys in secure hardware (SGX/TDX enclave) | Production with high-value operations |
| **Embedded (ERC-4337)** | Account abstraction wallet | Smart contract account with session keys | Production with flexible permissions |
| **Local Key (Dev)** | Direct private key | In-memory or file-based | Development and testing with mirage-rs |

```rust
/// Local key wallet for development and testing.
pub struct LocalKeyWallet {
    private_key: SigningKey,
    address: Address,
    provider: Arc<dyn ChainClient>,
}

/// ERC-4337 account abstraction wallet.
pub struct ERC4337Wallet {
    account_address: Address,
    session_key: SigningKey,
    entrypoint: Address,
    bundler_url: String,
    provider: Arc<dyn ChainClient>,
}
```

The `LocalKeyWallet` is used with mirage-rs during development. It stores the private key in memory — never suitable for production. The `ERC4337Wallet` uses account abstraction, where the "wallet" is a smart contract that the agent controls via session keys. Session keys can have limited permissions (e.g., "can only interact with these 5 contracts, spending at most 100 KORAI per day").

---

## Supporting Types

### Types from `roko-chain/src/types.rs`

```rust
/// Block number type.
pub type BlockNumber = u64;

/// Transaction hash type.
pub type TxHash = B256;

/// Block header information.
pub struct ChainHeader {
    pub number: BlockNumber,
    pub hash: B256,
    pub parent_hash: B256,
    pub timestamp: u64,
    pub base_fee_per_gas: Option<u64>,
}

/// Result of an eth_call simulation.
pub struct CallResult {
    pub output: Bytes,
    pub gas_used: u64,
    pub success: bool,
}

/// Transaction request.
pub struct TxRequest {
    pub to: Option<Address>,
    pub value: U256,
    pub data: Bytes,
    pub gas_limit: Option<u64>,
    pub max_fee_per_gas: Option<u64>,
    pub max_priority_fee_per_gas: Option<u64>,
}

/// Transaction receipt.
pub struct Receipt {
    pub tx_hash: TxHash,
    pub block_number: BlockNumber,
    pub status: bool,  // true = success, false = revert
    pub gas_used: u64,
    pub logs: Vec<LogEntry>,
}

/// Log entry from a transaction receipt.
pub struct LogEntry {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Bytes,
    pub block_number: BlockNumber,
    pub tx_hash: TxHash,
    pub log_index: u64,
}
```

### Error Types

```rust
pub enum ChainError {
    /// RPC communication failure.
    Rpc(String),
    /// Request timed out.
    Timeout,
    /// Node is offline or unreachable.
    Offline,
    /// Wallet has insufficient funds for the transaction.
    InsufficientFunds,
    /// Nonce gap detected (transaction ordering issue).
    NonceGap,
    /// Invalid address format.
    InvalidAddress,
    /// Operation not supported by this client/wallet implementation.
    Unsupported(String),
}

pub type ChainResult<T> = Result<T, ChainError>;
```

---

## Implementations

### Live RPC Client

Connects to a real Ethereum node via JSON-RPC:

```rust
pub struct RpcChainClient {
    provider: Arc<dyn Provider>,
    chain_id: u64,
    name: String,
}
```

Uses the `alloy` crate for Ethereum RPC communication. Supports both HTTP and WebSocket transports.

### mirage-rs Client

Connects to the in-process EVM simulator:

```rust
pub struct MirageChainClient {
    mirage: Arc<MirageInstance>,
    chain_id: u64,
}
```

The mirage client implements `ChainClient` by delegating to mirage-rs's internal EVM state. Reads are instantaneous (no network latency). See [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) for the simulator.

### Mock Client

For unit testing:

```rust
pub struct MockChainClient {
    block_number: AtomicU64,
    headers: HashMap<BlockNumber, ChainHeader>,
    logs: Vec<LogEntry>,
    storage: HashMap<(Address, U256), U256>,
}
```

The mock client allows tests to set up specific chain states and verify that agent code handles them correctly without any external dependencies.

---

## Gate Integration

The `roko-chain` crate also provides two domain-specific gates:

### TxSimGate

Pre-flight transaction simulation. Before submitting a transaction, simulate it via `eth_call` and verify:
- Transaction does not revert
- Gas usage is within expected bounds
- State changes match expectations
- No unexpected token transfers

### WalletGate

Post-transaction verification. After a transaction is submitted, verify:
- Transaction was included in a block
- Receipt status is success
- State changes match the simulation
- No unexpected side effects

Both gates implement the `Gate` trait from `roko-core` (see topic [04-gates](../04-gates/INDEX.md)), making them composable with other gates in the gate pipeline.

---

## Current Status and Gaps

**Built:**
- `ChainClient` trait with all methods (`crates/roko-chain/src/client.rs`)
- `ChainWallet` trait with all methods (`crates/roko-chain/src/wallet.rs`)
- All supporting types (`crates/roko-chain/src/types.rs`)
- `TxSimGate` and `WalletGate` stubs (`crates/roko-chain/src/lib.rs`)
- Mock implementations for testing

**Not yet built (Tier 6):**
- Live RPC client implementation with alloy (§R1)
- mirage-rs client implementation (§R2)
- ERC-4337 wallet implementation (§R3)
- TEE enclave wallet implementation (§R4)
- Gate implementations with full verification logic (§R5)

---

## Cross-references

- See [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) for the mirage-rs implementation of `ChainClient`
- See [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md) for how `ChainClient` is used in block ingestion
- See [19-chain-agent-heartbeat.md](./19-chain-agent-heartbeat.md) for how `ChainWallet` is used in the EXECUTE step
- See topic [04-gates](../04-gates/INDEX.md) for the gate pipeline that includes `TxSimGate` and `WalletGate`
