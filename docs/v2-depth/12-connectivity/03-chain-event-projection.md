# Chain Event Projection

> Depth for [11-CONNECTIVITY.md](../../v2/11-CONNECTIVITY.md). How the relay's chain watcher subscribes to EVM contract events, decodes them into typed Pulses, and publishes them on Bus topics for agent consumption.

**Depends on**: [01-SIGNAL](../../v2/01-SIGNAL.md) (Pulse), [11-CONNECTIVITY](../../v2/11-CONNECTIVITY.md) (relay, Connect protocol, finality oracle), [22-REGISTRIES](../../v2/22-REGISTRIES.md) (ERC-8004, ERC-8183)

---

## 1. Chain Watcher Overview

The chain watcher is a background task that bridges on-chain state into the Bus. It subscribes to a chain RPC endpoint, watches for new blocks and specific contract events, decodes each event into a typed payload, and publishes it as a Pulse on the Bus. Agents never connect directly to the chain RPC -- they subscribe to `chain.{chain_id}` topics and receive decoded events without running their own watchers.

The watcher runs alongside other chain subsystems (ISFR keeper, witness engine) as a long-lived tokio task. On startup it seeds its position by querying `eth_blockNumber`, backfills the most recent 20 blocks for continuity, then enters its poll loop.

```
Chain RPC                 BlockWatcher                  Bus
   |                          |                          |
   |--- eth_blockNumber ----->|                          |
   |<-- block 18294021 -------|                          |
   |                          |--- backfill 18294001..   |
   |--- eth_getBlockByNumber->|    ..18294021 ---------->|
   |<-- block + txs + logs ---|                          |
   |                          |--- publish chain:block ->|
   |                          |--- publish chain:tx ---->|
   |                          |--- publish chain:event ->|
   |                          |                          |
   |     [poll every 2s]      |                          |
   |--- eth_blockNumber ----->|                          |
   |<-- block 18294022 -------|                          |
   |                          |--- process_block ------->|
```

Each published Pulse carries the decoded event payload as JSON. Downstream Cells filter by `msg_type` to consume only the events they care about.

---

## 2. Event Sources

Two ingestion modes. The current implementation uses polling; the target architecture uses native RPC subscriptions.

**Polling (current).** Every `poll_interval` (default 2,000ms), the watcher calls `eth_blockNumber`. If the returned number exceeds the last-seen block, it fetches each new block with full transactions via `eth_getBlockByNumber`, iterates every transaction, fetches receipts for gas and logs, decodes known event signatures, and publishes the results. Polling works with any JSON-RPC provider, including Anvil forks and mirage-rs devnets.

**Subscription (target).** Uses `eth_subscribe("logs", {address, topics})` over a WebSocket transport. Each matching log arrives as a push notification, eliminating poll latency. When subscription mode is active, polling serves as a fallback: if the WebSocket disconnects, the watcher reverts to polling until the subscription can be re-established.

```rust
/// The poll loop core. Runs until the CancellationToken fires.
/// On each tick: query head block, process any new blocks since last_block.
pub async fn run(self, publish: PublishFn, cancel: CancellationToken) {
    let mut last_block: u64 = 0;
    // Seed: retry up to 30 times until the chain is reachable.
    // Once seeded, backfill the most recent BACKFILL_COUNT blocks.
    loop {
        let current = match self.provider.get_block_number().await {
            Ok(n) => n,
            Err(_) => { /* backoff and retry */ continue; }
        };
        if current <= last_block { continue; }
        for num in (last_block + 1)..=current {
            self.process_block(num, &publish).await;
        }
        last_block = current;
        tokio::time::sleep(self.poll_interval).await;
    }
}
```

---

## 3. Contract Events Watched

The chain watcher decodes events from three on-chain contract families. Each event maps to a Bus topic and a `msg_type` discriminant that downstream Cells use for filtering.

| Contract | Event | Topic | msg_type |
|---|---|---|---|
| AgentRegistry | `AgentRegistered(uint128,address,string)` | `chain.{id}` | `agent_registered` |
| AgentRegistry | `HeartbeatUpdated(uint128,uint64)` | `chain.{id}` | `heartbeat_updated` |
| MultiAgentMarket | `JobPosted(uint256,address,uint256,uint64,bytes32,bytes32[])` | `chain.{id}` | `job_posted` |
| MultiAgentMarket | `JobAwarded(uint256,uint128)` | `chain.{id}` | `job_awarded` |
| MultiAgentMarket | `JobSubmitted(uint256,bytes32)` | `chain.{id}` | `job_submitted` |
| MultiAgentMarket | `JobResolved(uint256,uint8)` | `chain.{id}` | `job_resolved` |
| ISFROracle | `RateSubmitted(uint256,uint256,uint256,address)` | `chain.{id}` | `rate_submitted` |
| ISFROracle | `RangeClosed(uint256,uint256,uint256)` | `chain.{id}` | `range_closed` |

All events share the same topic namespace (`chain.{chain_id}`). Agents subscribe to the topic and filter on `msg_type` to select relevant events.

---

## 4. Event Decoding

Event decoding converts raw EVM log data into structured JSON payloads. The watcher matches events by their `topic0` (the keccak256 of the event signature), extracts indexed topics, ABI-decodes the data segment, and publishes a `ContractEventInfo` Pulse.

```rust
/// A decoded contract event ready for Bus publication.
pub struct ContractEventInfo {
    /// Block number where this event was emitted.
    pub block_number: u64,
    /// Transaction hash containing the event.
    pub tx_hash: String,
    /// Log index within the transaction receipt.
    pub log_index: u32,
    /// Emitting contract address.
    pub contract: String,
    /// Decoded event name (e.g. "RateSubmitted", "JobPosted").
    pub event_name: String,
    /// Decoded event parameters as JSON.
    pub decoded: serde_json::Value,
}
```

### 4.1 Example: RateSubmitted decode

When the ISFROracle contract emits `RateSubmitted`, the watcher extracts `epochId` from `topic1` and ABI-decodes `compositeBps` and `confidenceBps` from the data segment.

```rust
/// Decode a RateSubmitted event from ISFROracle.
fn decode_rate_submitted(topics: &[B256], data: &[u8]) -> serde_json::Value {
    let epoch_id = topics.get(1)
        .map(|t| format!("{t:#x}"))
        .unwrap_or_default();
    let composite_bps = if data.len() >= 32 {
        U256::from_be_slice(&data[0..32]).to_string()
    } else { "0".to_string() };
    let confidence_bps = if data.len() >= 64 {
        U256::from_be_slice(&data[32..64]).to_string()
    } else { "0".to_string() };
    serde_json::json!({
        "epochId": epoch_id,
        "compositeBps": composite_bps,
        "confidenceBps": confidence_bps,
    })
}
```

### 4.2 Unknown events

Events whose `topic0` does not match any known signature are still published with the truncated `topic0` as the event name and raw hex-encoded data payload. The Bus carries all on-chain activity, not only recognized events.

---

## 5. Chain Event Trait

The `ChainEventSource` trait abstracts over event ingestion strategy. Two implementations cover the two deployment topologies.

```rust
/// Abstract source of chain events.
///
/// Implementations produce a stream of ChainEvents. The chain watcher
/// consumes events from this stream and publishes them as Pulses on the Bus.
#[async_trait]
pub trait ChainEventSource: Send + Sync + 'static {
    /// Subscribe to chain events. Returns a receiver that yields events
    /// as they are observed. The stream ends when the source shuts down.
    fn subscribe(&self) -> Receiver<ChainEvent>;
}

/// A single chain event produced by a ChainEventSource.
pub enum ChainEvent {
    /// A new block was finalized or observed.
    NewBlock(BlockInfo),
    /// A transaction was included in a block.
    Transaction(TxInfo),
    /// A decoded contract event.
    ContractEvent(ContractEventInfo),
    /// A chain reorganization was detected.
    Reorg(ChainReorgInfo),
}
```

**AlloyChainWatcher.** The standalone implementation. Connects to a JSON-RPC endpoint via alloy's `DynProvider`. Uses polling or subscription depending on transport capabilities. Suitable for relay processes connecting to external chain RPCs (Alchemy, Infura, local Anvil, mirage-rs devnets).

**LedgerChainWatcher.** The validator-embedded implementation. Reads from an internal finalized block channel exposed by the validator's consensus engine. Zero network overhead, zero latency beyond block finalization time. Used when the relay runs inside the validator process itself.

The trait boundary means the Bus publication layer is identical regardless of event source. A relay operator switches between implementations by configuration, with no change to downstream Cells.

---

## 6. Finality Tagging

Every chain event Pulse carries a `FinalityTag` (defined in [11-CONNECTIVITY](../../v2/11-CONNECTIVITY.md) section 16). The chain watcher assigns finality level based on confirmation depth: `confirmations = head_block - event_block`.

```rust
/// Finality classification for a chain event.
pub enum FinalityLevel {
    /// Sufficient confirmations. Reorg probability < 10^-6.
    Final,
    /// Moderate confirmation depth. Reorg probability < 10^-3.
    QuasiFinalized,
    /// Recent block or mempool. Reorg probability is non-trivial.
    Reversible,
}
```

| Chain | Final | QuasiFinalized | Reversible |
|---|---|---|---|
| Ethereum | 64 blocks (~13 min) | 12 blocks (~2.5 min) | < 12 blocks |
| Base / Arbitrum / OP | L1 finality + proof posted | Sequencer confirmed | Sequencer pending |
| Mirage (devnet) | 1 block (instant finality) | 1 block | n/a |

Agents specify their required finality level when subscribing. A Verify Cell validating a deposit filters for `FinalityLevel::Final`. A monitoring Cell tracking gas prices accepts `FinalityLevel::Reversible`. Events below the required level are buffered and re-emitted once sufficient confirmations accumulate.

---

## 7. Reorg Handling

Chain reorganizations invalidate previously published events. The watcher tracks `parent_hash` for each processed block. If a new block's `parent_hash` does not match the stored hash for `block_number - 1`, a reorg has occurred.

When a reorg is detected:

1. The watcher publishes a `ChainReorg` Pulse on `chain.{chain_id}` with the orphaned block range.
2. All Signals derived from orphaned blocks are tagged `reorg_invalidated: true` in the Store.
3. Cells subscribed to `chain.{chain_id}` receive the invalidation notice.
4. The watcher reprocesses blocks from the fork point on the new canonical chain.

```rust
/// Reorg information published to the Bus.
pub struct ChainReorgInfo {
    pub chain_id: u64,
    pub old_head: String,
    pub new_head: String,
    pub orphaned_range: Range<u64>,
    pub depth: u64,
}
```

Reorgs deeper than a configurable threshold (default: 10 blocks) trigger a `SafetyViolation` Signal in addition to the standard reorg Pulse, alerting operator-facing Lens Cells.

---

## 8. Backoff, Recovery, and Ring Buffers

### 8.1 Exponential backoff

The chain watcher handles RPC failures with exponential backoff: `min(2^min(failures, 5) * 2000ms, 60_000ms) + jitter`. Jitter is derived from `subsec_nanos` (no rand dependency). The delay ranges from 2s (first failure) to 60s (cap at 5+ failures). On recovery, the failure counter resets to zero. If the watcher never seeds (30 consecutive startup failures), it enters a late-seed mode where the first successful poll establishes the baseline.

```rust
/// Compute backoff duration for consecutive failures.
pub fn compute_backoff(consecutive_failures: u32) -> Duration {
    let exp = consecutive_failures.min(5);
    let base_ms: u64 = 2u64.pow(exp) * 2000;
    let capped_ms = base_ms.min(60_000);
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos() % 1000))
        .unwrap_or(0);
    Duration::from_millis(capped_ms + jitter_ms)
}
```

### 8.2 Chain state ring buffers

The watcher maintains in-memory ring buffers for recent chain data: 64 blocks, 128 transactions, 128 events. These buffers serve REST endpoints (via `roko serve`) and dashboard panels without requiring a database or re-fetching from the chain. When the buffer is full, the oldest entry is evicted. A `watcher_running` flag lets health endpoints distinguish "no chain data yet" from "chain watcher is not running."

---

## 9. Configuration

```toml
[chain]
enabled = true
profile = "mirage"
rpc_url = "http://127.0.0.1:8545"
chain_id = 31337
agent_registry = "0x5FbDB2315678afecb367f032d93F642f64180aa3"
bounty_market = "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"

[relay.chain_watcher]
rpc_url = "wss://rpc.daeji.network"
chain_id = 31337
contracts = [
  { name = "agent-registry", address = "0x5FbDB2315678afecb367f032d93F642f64180aa3" },
  { name = "bounty-market", address = "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512" },
  { name = "isfr-oracle", address = "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0" },
]
poll_interval_ms = 2000
```

The ISFR keeper runs alongside the chain watcher as a separate background task. Its configuration lives in `[isfr]` and controls rate source polling independently of block watching.

```toml
[isfr]
enabled = true
poll_interval_secs = 10
epoch_duration_secs = 60
min_submissions = 2

[[isfr.sources]]
name = "aave-v3-usdc"
kind = "aave_v3"
weight = 0.30
class = "lending"
rpc_url = "http://127.0.0.1:8545"
pool_address = "0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"
```

---

## 10. Crate Mapping

| Crate | File | Responsibility |
|---|---|---|
| `roko-chain` | `src/block_watcher.rs` | `BlockWatcher` poll loop, block/tx/event processing, `ChainState` ring buffers, backoff |
| `roko-chain` | `src/isfr_keeper.rs` | `ISFRKeeper` orchestrator, rate polling, composite aggregation, relay publication |
| `roko-chain` | `src/isfr_sources/mod.rs` | `ISFRSource` trait, `CompositeRate`, `SourceReading`, weighted median |
| `roko-chain` | `src/isfr_sources/aave_v3.rs` | Aave V3 lending rate source (alloy-backed) |
| `roko-chain` | `src/isfr_sources/compound_v3.rs` | Compound V3 lending rate source (alloy-backed) |
| `roko-chain` | `src/isfr_sources/ethena.rs` | Ethena sUSDe structured yield source (alloy-backed) |
| `roko-chain` | `src/isfr_sources/lido.rs` | Lido stETH staking rate source (alloy-backed) |
| `roko-chain` | `src/isfr_sources/mock.rs` | `MockSource` (dev/test) and `OfflineSource` (unreachable RPC fallback) |
| `roko-chain` | `src/observer.rs` | `BlockObserver` with address filtering for targeted event watching |
| `roko-core` | `src/config/chain.rs` | `ChainConfig`, `RelayConfig`, `ISFRSection` TOML deserialization |
| `roko-serve` | `src/state.rs` | `AppState` holds `Arc<ChainState>` for REST endpoint access |
| `roko-serve` | `src/lib.rs` | Spawns `BlockWatcher::run()` as background task on server startup |
