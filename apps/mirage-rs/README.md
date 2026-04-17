# mirage-rs

**Standalone EVM fork simulator** | MIT/Apache-2.0 | minimal internal deps

## Feature surface

| Feature        | Status  | Purpose                                                                                                  |
|----------------|---------|----------------------------------------------------------------------------------------------------------|
| `binary`       | default | Enable the `mirage-rs` binary entrypoint (`src/main.rs`).                                                |
| `library`      | opt-in  | Library-only usage (no binary).                                                                          |
| `sim-gas`      | opt-in  | Use revm's gas accounting path.                                                                          |
| `chain`        | opt-in  | HDC index, `InsightEntry` knowledge layer, stigmergy pheromones. Adds `chain_*` JSON-RPC methods.        |
| `roko`         | opt-in  | Bridge to `roko-core` traits: `SimulationGate`, `HdcSubstrate`, `ChainSubstrate`. Implies `chain`.       |

Default build = pure EVM fork simulator. The `chain` / `roko` extensions are opt-in so consumers who only want an Anvil-style simulator pay nothing for them.

## Add as a dependency

Paths below are relative to the `roko/` workspace root. From outside the workspace, use a git dependency or an absolute path.

```toml
[dependencies]
# Pure EVM fork simulator, no chain extensions.
mirage-rs = { path = "apps/mirage-rs", default-features = false, features = ["library"] }

# With the agent-chain knowledge layer (HDC + insight entries + pheromones).
mirage-rs = { path = "apps/mirage-rs", default-features = false, features = ["library", "chain"] }

# Full roko integration (Gate + Substrate impls usable in a roko golem).
mirage-rs = { path = "apps/mirage-rs", default-features = false, features = ["library", "roko"] }
```

## Quickstart

```bash
# 1. Pure EVM simulator (like Anvil, but forking a live chain)
cargo run -p mirage-rs -- --rpc-url https://ethereum-rpc.publicnode.com

# 2. EVM + chain extensions (HDC semantic memory + stigmergy pheromones)
cargo run -p mirage-rs --features chain -- \
  --rpc-url https://ethereum-rpc.publicnode.com \
  --enable-hdc --enable-knowledge --enable-stigmergy

# 3. Isolated mode (no upstream, all accounts start with 1 ETH — fast CI tests)
cargo run -p mirage-rs
```

See [Chain extensions](#chain-extensions-feature--chain) and [Roko bridge](#roko-bridge-feature--roko) below for deep dives on the opt-in subsystems.

---

A local Ethereum node for development and testing, like [Anvil](https://getfoundry.sh/reference/anvil/) — but connected to live chains. mirage-rs forks mainnet state lazily over RPC, keeps watched contracts in sync block-by-block, and gives you the full `eth_*` / `evm_*` / `anvil_*` manipulation API you already know. No full node sync. Instant startup.

Where Anvil forks at a pinned block and stays there, mirage-rs optionally follows the chain forward, selectively replaying transactions that touch your contracts so the local view stays current as the market moves.

```bash
# Drop-in replacement for Anvil — fork mainnet on port 8545
mirage-rs --rpc-url https://ethereum-rpc.publicnode.com

# With live following over WebSocket
mirage-rs \
  --rpc-url https://ethereum-rpc.publicnode.com \
  --ws-url wss://ethereum-rpc.publicnode.com

# Isolated mode (no upstream, all accounts start with 1 ETH)
mirage-rs
```

Point any Ethers/Viem/Alloy client at `http://127.0.0.1:8545` and it works the same as Anvil. Your Hardhat tests, Foundry scripts, and custom tooling need zero changes.

## Why not just use Anvil?

Anvil forks at a block and freezes. That's fine for unit tests, but it can't answer questions like: *what happens to my Uniswap position after my transaction, as the next 10 blocks of real market activity play out?*

mirage-rs can, because it:

- **Follows the chain.** A targeted follower subscribes to `newHeads` via WebSocket, filters each block for transactions that touch watched contracts, and replays only those locally. For a typical portfolio of 3-10 DeFi positions, that's ~5-15 transactions per block instead of ~150.
- **Classifies contracts automatically.** When a transaction writes 3+ storage slots on a new address, the diff classifier promotes it to the watch list. Simple token transfers (1-2 slots) get slot-level overrides without full tracking. This propagation is recursive — composability chains across protocols are captured automatically.
- **Branches with copy-on-write.** Scenarios fork from a shared baseline using CoW overlays (~12.8 KB per branch vs ~3.2 MB for a full clone), so you can run parallel what-if simulations cheaply.

For pure unit testing against static state, Anvil is great. For anything that touches live DeFi positions, mirage-rs fills the gap.

## How it works

mirage-rs sits between your application and a real Ethereum RPC endpoint. It maintains a three-layer state model:

```
 Reads flow top-down; first hit wins.

 ┌─────────────────────────────────┐
 │  1. DirtyStore (local writes)   │  ← eth_sendTransaction, setBalance, scenarios
 ├─────────────────────────────────┤
 │  2. ReadCache (LRU + TTL)       │  ← <1µs hot reads, 12s default TTL
 ├─────────────────────────────────┤
 │  3. UpstreamRpc (lazy fetch)    │  ← token-bucket rate limiter, retries w/ backoff
 └────────────────┬────────────────┘
                  │
          Live Ethereum node
```

On first access, account balances, nonces, storage slots, and bytecode are fetched from upstream and cached. Writes go into the dirty overlay and never touch upstream. You get a mutable view of mainnet state without syncing anything.

When a WebSocket URL is configured, the **targeted follower** subscribes to new blocks and replays only the transactions that matter to your watched contracts. Everything else is ignored.

## Installation

From source (requires Rust 1.85+):

```bash
# Install the pure EVM fork simulator binary
cargo install --path roko/apps/mirage-rs

# Install with chain extensions enabled
cargo install --path roko/apps/mirage-rs --features chain

# Install with the full roko bridge (Gate/Substrate trait impls)
cargo install --path roko/apps/mirage-rs --features roko
```

Or run directly from the workspace:

```bash
# Pure EVM fork simulator
cargo run -p mirage-rs -- --rpc-url https://ethereum-rpc.publicnode.com

# With chain extensions
cargo run -p mirage-rs --features chain -- \
  --rpc-url https://ethereum-rpc.publicnode.com \
  --enable-hdc --enable-knowledge --enable-stigmergy
```

## Usage

### Forking mainnet

```bash
# Fork at latest block
mirage-rs --rpc-url https://ethereum-rpc.publicnode.com

# Specify chain ID and port
mirage-rs --rpc-url https://ethereum-rpc.publicnode.com --chain-id 1 --port 8545

# With live block following
mirage-rs \
  --rpc-url https://ethereum-rpc.publicnode.com \
  --ws-url wss://ethereum-rpc.publicnode.com
```

### Forking other chains

```bash
# Arbitrum
mirage-rs --rpc-url https://arbitrum-one-rpc.publicnode.com --chain-id 42161

# Base
mirage-rs --rpc-url https://base-rpc.publicnode.com --chain-id 8453

# Polygon
mirage-rs --rpc-url https://polygon-bor-rpc.publicnode.com --chain-id 137
```

### Using with Foundry

mirage-rs is a drop-in backend for `forge script` and `forge test`:

```bash
# Run a Forge script against the live fork
forge script script/Deploy.s.sol --rpc-url http://127.0.0.1:8545 --broadcast

# Run tests
forge test --fork-url http://127.0.0.1:8545
```

### Using with Hardhat

```js
// hardhat.config.js
module.exports = {
  networks: {
    mirage: {
      url: "http://127.0.0.1:8545",
    },
  },
};
```

```bash
npx hardhat test --network mirage
```

### Using with Viem

```ts
import { createPublicClient, createWalletClient, http } from "viem";
import { mainnet } from "viem/chains";

const transport = http("http://127.0.0.1:8545");

const publicClient = createPublicClient({ chain: mainnet, transport });
const walletClient = createWalletClient({ chain: mainnet, transport });

// Works exactly like Anvil
const blockNumber = await publicClient.getBlockNumber();
```

### Using with Ethers.js

```js
const { ethers } = require("ethers");
const provider = new ethers.JsonRpcProvider("http://127.0.0.1:8545");
const blockNumber = await provider.getBlockNumber();
```

## CLI Reference

```
mirage-rs [OPTIONS]
```

### Server options

| Flag | Default | Description |
|------|---------|-------------|
| `--host` | `127.0.0.1` | Bind address |
| `--port` | `8545` | Bind port |
| `--chain-id` | `1` | Effective chain ID |
| `--watchdog-timeout` | (none) | Shut down after N seconds of inactivity |

### Fork options

| Flag | Default | Description |
|------|---------|-------------|
| `--rpc-url` | (none) | Upstream HTTP JSON-RPC URL. Omit for isolated mode |
| `--ws-url` | (none) | Upstream WebSocket URL. Enables targeted block following |
| `--upstream-rps` | `100` | Upstream requests per second budget |
| `--upstream-burst` | `200` | Upstream burst capacity |
| `--cache-size` | `10000` | Read cache entry capacity |
| `--cache-ttl-secs` | `12` | Read cache TTL in seconds (~1 block) |

### Validation options

| Flag | Default | Description |
|------|---------|-------------|
| `--strict-nonce` | `false` | Reject transactions with incorrect nonces |
| `--strict-balance` | `false` | Reject transactions that overdraw sender balance |
| `--verify-signatures` | `false` | Require valid ECDSA signatures on raw transactions |

By default, validation is relaxed (same as Anvil's default behavior) so you can send transactions from any address without signing.

### Chain extension flags (require `--features chain`)

| Flag | Default | Description |
|------|---------|-------------|
| `--enable-hdc` | `false` | Enable the HDC index (`chain_postInsight`, `chain_searchInsights`) |
| `--enable-knowledge` | `false` | Enable the knowledge state machine (confirmations, challenges, decay) |
| `--enable-stigmergy` | `false` | Enable pheromone deposits/queries (`chain_depositPheromone`, `chain_queryPheromones`) |
| `--chain-hnsw-threshold` | `100000` | Entry count above which the HDC index auto-switches from brute-force to HNSW |

Subsystems default to off even when compiled in — you must set the flags explicitly when you want them active.

### Persistence options

| Flag | Default | Description |
|------|---------|-------------|
| `--state-dir` | `.roko/state/` | Directory for snapshot files |
| `--snapshot-interval-secs` | `30` | Seconds between periodic snapshots |
| `--no-persist` | `false` | Disable disk persistence entirely |

See [State persistence](#state-persistence) below for a full walkthrough.

### Resource profiles

| Flag | Default | Description |
|------|---------|-------------|
| `--profile` | `standard` | Resource profile: `micro`, `standard`, `power` |

Profiles control memory ceilings and capacity limits:

| Profile | Memory ceiling | Watched contracts | Cache entries | Bytecode cache |
|---------|---------------|-------------------|---------------|----------------|
| `micro` | 256 MB | 32 | 5,000 | 1,000 |
| `standard` | 512 MB | 64 | 10,000 | 2,000 |
| `power` | 2 GB | 256 | 50,000 | 10,000 |

The process checks available system memory at startup and exits with code 2 if the selected profile doesn't fit (128 MB headroom margin required). At runtime, memory pressure is monitored and the fork responds in tiers:

| Pressure level | Threshold | Response |
|----------------|-----------|----------|
| Warning | 50% of ceiling | Evict LRU cache entries |
| Throttle | 70% | Demote auto-classified contracts to slot-only reads |
| Emergency | 90% | Demote to proxy mode (disable replay) |

## Supported RPC Methods

### Standard Ethereum methods

The same `eth_*` namespace you use with Anvil and any other node:

| Method | Description |
|--------|-------------|
| `eth_chainId` | Returns the chain ID |
| `eth_blockNumber` | Returns the current block number |
| `eth_gasPrice` | Returns the current gas price |
| `eth_maxPriorityFeePerGas` | Returns the current priority fee |
| `eth_feeHistory` | Returns fee history for a range of blocks |
| `eth_getBalance` | Returns the balance of an address |
| `eth_getTransactionCount` | Returns the nonce of an address |
| `eth_getStorageAt` | Returns the value of a storage slot |
| `eth_getCode` | Returns the bytecode at an address |
| `eth_call` | Executes a call without creating a transaction |
| `eth_estimateGas` | Estimates gas for a transaction |
| `eth_sendTransaction` | Sends a transaction (auto-signed, like Anvil) |
| `eth_sendRawTransaction` | Sends a signed raw transaction |
| `eth_getTransactionReceipt` | Returns the receipt of a transaction |
| `eth_getTransactionByHash` | Returns transaction details by hash |
| `eth_getBlockByNumber` | Returns a block by number |
| `eth_getBlockByHash` | Returns a block by hash |
| `eth_getLogs` | Returns logs matching a filter |
| `web3_clientVersion` | Returns the client version string |
| `net_version` | Returns the network ID |

### EVM manipulation methods

Anvil/Hardhat-compatible state manipulation. If your test suite uses these with Anvil, it works the same here:

| Method | Description |
|--------|-------------|
| `evm_snapshot` | Capture current state, returns a snapshot ID |
| `evm_revert` | Roll back to a snapshot |
| `evm_increaseTime` | Advance the block timestamp by N seconds |
| `evm_setNextBlockTimestamp` | Set a specific next-block timestamp |

### State override methods

Available under the `anvil_*`, `hardhat_*`, and `mirage_*` namespaces (all three work):

| Method | Description |
|--------|-------------|
| `setBalance(address, value)` | Override an account's ETH balance |
| `setStorageAt(address, slot, value)` | Write a single storage slot |
| `setCode(address, bytecode)` | Deploy bytecode at an address |
| `setNonce(address, nonce)` | Override an account's nonce |

```bash
# Set balance using cast (works with any namespace prefix)
cast rpc anvil_setBalance 0xf39F...2266 0xDE0B6B3A7640000 --rpc-url http://127.0.0.1:8545
```

### Mirage-specific methods

These extend the Anvil API with live-chain capabilities:

| Method | Description |
|--------|-------------|
| `mirage_mintERC20(token, to, amount)` | Mint ERC-20 tokens by detecting and writing the balance storage slot |
| `mirage_watchContract(address)` | Add a contract to the targeted follower's watch list |
| `mirage_unwatchContract(address)` | Remove a contract from the watch list |
| `mirage_getWatchList()` | Return all watched contracts with metadata |
| `mirage_prefetchAccount(address)` | Warm the cache for an account |
| `mirage_prefetchSlots(address, slots[])` | Warm specific storage slots |
| `mirage_getDirtySlots(address)` | Return locally modified storage slots for an address |
| `mirage_status()` | Readiness status, chain ID, block number, watch list size |
| `mirage_getResourceUsage()` | Memory, cache stats, pressure score, upstream counters |
| `mirage_setResourceLimits(...)` | Dynamically adjust resource caps at runtime |
| `mirage_getPosition(request)` | Read a DeFi position snapshot |
| `mirage_subscribeEvents(filter)` | Open a WebSocket event stream with address/topic filters |
| `mirage_shutdown()` | Graceful process shutdown |

### Scenario methods

Run branching what-if simulations against live state:

| Method | Description |
|--------|-------------|
| `mirage_beginScenarioSet(baseline)` | Create a scenario set from a baseline state |
| `mirage_defineScenario(setId, scenario)` | Add a scenario with transactions and assertions |
| `mirage_runScenarioSet(setId, mode)` | Execute in `sequential` or `parallel` mode |
| `mirage_getScenarioResults(jobId)` | Poll for results |
| `mirage_compareScenarios(setId)` | Diff outcomes across scenarios in a set |

## Targeted Following

This is the core feature that separates mirage-rs from static forks.

When you provide a `--ws-url`, mirage-rs subscribes to `newHeads` and for each new block:

1. Fetches the full block from upstream
2. Filters for transactions touching any watched address
3. Replays only those transactions through the local fork's EVM
4. Runs the diff classifier on the resulting state changes
5. Auto-promotes new contracts that cross the slot threshold (3+ storage writes)

For a typical DeFi portfolio (3-10 positions), this means replaying ~5-15 transactions per block instead of the full ~150. Blocks process in <100ms at steady state.

### Watch list management

Contracts enter the watch list three ways:

1. **Manual** — call `mirage_watchContract(address)` or define `track.addresses` in a scenario fixture
2. **Auto-classification** — the diff classifier sees 3+ storage slots written on a new address and promotes it
3. **Contagion** — a replayed transaction writes to a new contract that exceeds the slot threshold, recursively extending the watch list

```bash
# Manually watch the Uniswap V3 Router
cast rpc mirage_watchContract 0xE592427A0AEce92De3Edee1F18E0157C05861564 \
  --rpc-url http://127.0.0.1:8545

# Check the watch list
cast rpc mirage_getWatchList --rpc-url http://127.0.0.1:8545
```

## Scenarios

Scenarios let you define branching what-if simulations. Each scenario is a named sequence of transactions that execute against a shared baseline snapshot. In parallel mode, each branch gets an isolated copy-on-write overlay, so branches can't observe each other's mutations.

### TOML fixtures

```toml
# tests/scenarios/eth_crash.toml
[scenario]
name = "eth_crash"
description = "Directional WETH->USDC selloff with repeated router pressure"

[[transactions]]
from = "0x1000000000000000000000000000000000000001"
to = "0x10000000000000000000000000000000000000a0"
value = "0x0"
gas = 320000
data = "0x414bf389..."

[assertions]
watch_list_contains = ["0x10000000000000000000000000000000000000a0"]

[assertions.token_balance_gte]
token = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
address = "0x1000000000000000000000000000000000000001"
amount = "0x1"

[track]
addresses = ["0xE592427A0AEce92De3Edee1F18E0157C05861564"]
```

### Included scenarios

| File | Description |
|------|-------------|
| `uniswap_v3_entry.toml` | Position-manager mint + liquidity increase on a watched pool |
| `eth_crash.toml` | Directional WETH→USDC selloff with 20+ router transactions |
| `aave_liquidation.toml` | Oracle shock, account deterioration, and liquidation flow |
| `new_pool.toml` | Deploy token, initialize pool, seed liquidity, route first swap |
| `volume_spike.toml` | High-frequency volume burst across multiple pairs |

### Programmatic usage

```rust
use mirage_rs::{MirageClient, Scenario, RunMode, ScenarioAssertions};

let set_id = client.mirage_begin_scenario_set("latest").await?;

client.mirage_define_scenario(&set_id, &Scenario {
    id: "bull-case".into(),
    name: "large buy".into(),
    transactions: vec![buy_tx],
    track_addresses: vec![pool, router],
    max_gas: Some(500_000),
    timeout: Duration::from_secs(5),
    assertions: ScenarioAssertions::default(),
}).await?;

client.mirage_define_scenario(&set_id, &Scenario {
    id: "bear-case".into(),
    name: "large sell".into(),
    transactions: vec![sell_tx],
    track_addresses: vec![pool, router],
    max_gas: Some(500_000),
    timeout: Duration::from_secs(5),
    assertions: ScenarioAssertions::default(),
}).await?;

// Run both branches in parallel with isolated CoW overlays
let job_id = client.mirage_run_scenario_set(&set_id, RunMode::Parallel).await?;
let results = client.mirage_get_scenario_results(&job_id).await?;
```

## Library Usage

mirage-rs ships as both a binary and a library crate.

```toml
[dependencies]
mirage-rs = { path = "apps/mirage-rs", default-features = false, features = ["library"] }
```

### Spawning a test instance

```rust
use mirage_rs::{MirageClient, spawn_mirage_test_instance, TransactionRequest};
use std::time::Duration;
use alloy_primitives::U256;

let mut instance = spawn_mirage_test_instance(None, Some(18_545)).await?;
let client = MirageClient::new(instance.config()).await?;
client.wait_ready(Duration::from_secs(10)).await?;

// Use it like Anvil
let tx_hash = client.eth_send_transaction(TransactionRequest {
    from: Some(sender),
    to: Some(receiver),
    gas: Some(21_000),
    value: Some(U256::from(1_000_000)),
    ..Default::default()
}).await?;

let snap = client.evm_snapshot().await?;
// ... speculative work ...
client.evm_revert(snap).await?;

instance.shutdown().await?;
```

## Chain extensions (feature = `chain`)

The `chain` feature turns mirage into an agent-coordination substrate on top of the EVM fork: HDC-indexed knowledge entries, six typed knowledge kinds with decay lifecycles, and time-decaying stigmergy pheromones.

### Enabling subsystems at runtime

Compile with `--features chain` and toggle subsystems via CLI flags:

```bash
cargo run -p mirage-rs --features chain -- \
  --host 127.0.0.1 --port 8545 \
  --enable-hdc \
  --enable-knowledge \
  --enable-stigmergy \
  --chain-hnsw-threshold 100000
```

Each toggle is independent — you can enable only the HDC index without the knowledge state machine, or only stigmergy without HDC, etc. Methods targeting a disabled subsystem return JSON-RPC error `-32600` ("subsystem disabled").

### Chain JSON-RPC methods

| Method | Purpose |
|--------|---------|
| `chain_postInsight` | Commit a new `InsightEntry` (content-addressed, HDC-indexed). Returns `{outcome, id}` where outcome is `accepted` / `duplicate` / `exact_match`. |
| `chain_searchInsights` | HDC semantic top-K search. Project free text via `{query, k}` or provide a raw 1280-byte `{queryVector, k}`. |
| `chain_getInsight` | Fetch one entry by id. |
| `chain_confirmInsight` | Record a confirmation (boosts weight, extends effective half-life). |
| `chain_challengeInsight` | Open a challenge against an entry (moves it to `Challenged` state). |
| `chain_applyDecay` | Run the decay sweep: refresh weights, prune entries that drop below 1% of initial. |
| `chain_depositPheromone` | Deposit a `threat` / `opportunity` / `wisdom` signal with exponential decay. |
| `chain_queryPheromones` | HDC semantic top-K over the pheromone field, decay applied at query time. |
| `chain_stats` | Return `{insights, pheromones, toggles}`. |

### Example: post-then-search flow

```bash
# Post an insight
cast rpc chain_postInsight '{
  "author": "agent:alice",
  "kind": "warning",
  "content": "never call selfdestruct in a proxy implementation contract",
  "enabledBy": [],
  "stakeWei": 2000000000000000
}' --rpc-url http://127.0.0.1:8545

# → {"outcome": "accepted", "id": "a1b2c3d4..."}

# Confirm from another agent
cast rpc chain_confirmInsight '{
  "id": "insight:a1b2c3d4...",
  "confirmer": "agent:bob"
}' --rpc-url http://127.0.0.1:8545

# Semantic search — finds the above even though the query uses different words
cast rpc chain_searchInsights '{
  "query": "proxy contract destruction safety",
  "k": 5
}' --rpc-url http://127.0.0.1:8545

# Apply decay (normally run on a schedule)
cast rpc chain_applyDecay '{}' --rpc-url http://127.0.0.1:8545
```

### Six knowledge kinds

| Kind | Default τ (half-life) | Use case |
|------|----------------------|----------|
| `insight` | 7 days | Factual observation ("what IS") |
| `heuristic` | 15 days | Learned strategy ("what to DO") |
| `warning` | 3 minutes | Urgent "don't do X" (short τ so conditions can change) |
| `causal_link` | 15 days | Observed cause↔effect |
| `strategy_fragment` | 15 days | Reusable partial plan |
| `anti_knowledge` | 15 days | Explicit corrections to prior incorrect beliefs |

Weights decay as `w(t) = w₀ × 2^(-age/τ)`. Each confirmation extends the effective τ via `τ_eff = τ × (1 + √confirms × 0.5)`, so well-validated entries persist while unreinforced noise fades.

### Three pheromone kinds

| Kind | τ | Use case |
|------|---|----------|
| `threat` | 2h | Short-lived danger signals (rug pulls, active exploits) |
| `opportunity` | 4h | Time-sensitive openings (arb windows, liquidations) |
| `wisdom` | 24h | Durable tactical knowledge |

## HTTP REST API

When chain extensions are enabled (`--enable-hdc --enable-knowledge --enable-stigmergy`), mirage-rs exposes an HTTP REST API on the same port as the JSON-RPC server (default 8545) under the `/api` prefix. This API powers dashboards, monitoring UIs, and any tooling that prefers standard REST over JSON-RPC.

CORS is permissive -- any origin can call these endpoints. All timestamps are Unix seconds. Pagination follows an offset/limit pattern.

### Architecture

```
                          +-----------+
                          |  Client   |
                          | (curl/UI) |
                          +-----+-----+
                                |
                    HTTP :8545  |  WS :8545
                                v
                    +-----------+-----------+
                    |    axum Router        |
                    |  /api/* REST routes   |
                    |  /api/ws  WebSocket   |
                    |  /*       JSON-RPC    |
                    +-----------+-----------+
                                |
                    +-----------+-----------+
                    |    Handler Layer      |
                    | pheromone / knowledge |
                    | agent / topology / ws |
                    +-----------+-----------+
                                |
                    +-----------+-----------+
                    |    ChainContext       |
                    | (parking_lot RwLock)  |
                    +-----------+-----------+
                       /        |        \
              +-------+   +----+----+   +--------+
              |Pheromone|  |Knowledge|  |Agent    |
              |Field    |  |Store    |  |Registry |
              +---------+  +---------+  +---------+
```

All REST handlers read from or write to `ChainContext` via a shared `Arc<RwLock>`. The WebSocket endpoint subscribes to internal broadcast buses (`PheromoneBus`, `InsightBus`) and forwards events as JSON frames.

### Quick start

```bash
# One-command quickstart: builds, starts, seeds, opens dashboard
./apps/mirage-rs/static/quickstart.sh

# Or manually:

# 1. Start mirage-rs with all chain extensions enabled
MIRAGE_DASHBOARD_DIR=apps/mirage-rs/static \
  cargo run -p mirage-rs --features chain,roko --bin mirage-rs -- \
    --enable-hdc --enable-knowledge --enable-stigmergy

# 2. Seed with demo data (50 insights, 20 pheromones, 3 agents)
cargo run -p mirage-rs --features chain,roko --example seed_chain_fixtures

# 3. Open the dashboard
open http://127.0.0.1:8545/dashboard/

# 4. Check health
curl http://127.0.0.1:8545/api/health | jq

# 5. View aggregated stats
curl http://127.0.0.1:8545/api/stats | jq

# 6. List active pheromones sorted by intensity
curl 'http://127.0.0.1:8545/api/pheromones?sort=intensity&order=desc&limit=10' | jq

# 7. Search knowledge entries semantically
curl 'http://127.0.0.1:8545/api/knowledge/search?q=uniswap+pool+revert&k=5' | jq

# 8. List registered agents
curl http://127.0.0.1:8545/api/agents | jq

# 9. Get the agent interaction graph
curl http://127.0.0.1:8545/api/agents/topology | jq

# 10. Stream live events over WebSocket (requires `roko` feature)
websocat ws://127.0.0.1:8545/api/ws
```

### Dashboard

The interactive dashboard is served at `/dashboard/` when the `MIRAGE_DASHBOARD_DIR` environment variable or a `static/` directory is present. It provides:

- **Real-time block stream** — live mainnet block data from the mirage fork
- **Pheromone particle field** — animated canvas with decaying threat/opportunity/wisdom signals
- **Knowledge graph** — force-directed HDC similarity graph with click-to-inspect detail
- **Agent topology** — confirm/challenge network visualization
- **Agent registry** — registered agents with stats, traces, and liveness
- **Pheromone heatmap** — time-bucketed deposit activity timeline
- **Leaderboard** — agents ranked by contribution count
- **Semantic search** — free-text HDC query with ranked results
- **Manual controls** — post insights, deposit pheromones, register agents, trigger decay
- **WebSocket live updates** — real-time event stream from pheromone and insight buses
- **Performance metrics** — RPC latency, search timing, cache stats, block saturation

The dashboard is split into ES modules (`static/js/`) for maintainability:

| Module | Lines | Purpose |
|--------|-------|---------|
| `state.js` | ~50 | Shared mutable state object |
| `api.js` | ~110 | REST/RPC client, logging, toasts |
| `polling.js` | ~470 | All poll functions (block, chain, entries, edges, agents, heatmap, topology, kinds) |
| `charts.js` | ~300 | Sparklines, heatmap, growth timeline, hero stats, metric tick |
| `pheromones.js` | ~170 | Particle system canvas |
| `graph.js` | ~300 | Force-directed knowledge graph + detail panel |
| `topology.js` | ~130 | Agent topology graph |
| `ws.js` | ~130 | WebSocket connection + event handling |
| `main.js` | ~260 | Init, connect, animation loop, DOM event wiring |

### Feature flags and API availability

| Feature | CLI flags required | Endpoints enabled |
|---------|-------------------|-------------------|
| `chain` | `--enable-hdc --enable-knowledge --enable-stigmergy` | All `/api/*` REST endpoints |
| `roko` | (implies `chain`) | Adds `/api/ws` WebSocket streaming |
| `binary` | (default) | Enables the `mirage-rs` binary entrypoint |

Individual chain toggles control subsystem availability at runtime:

| Toggle | Controls | HTTP 503 if disabled |
|--------|----------|---------------------|
| `--enable-stigmergy` | Pheromone field endpoints | `POST /api/pheromones`, `POST /api/pheromones/query` |
| `--enable-knowledge` | Knowledge store endpoints | All `/api/knowledge/*` endpoints |
| `--enable-hdc` | HDC vector projections | Semantic search, pheromone query |

### Endpoint summary

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/health` | Health check with subsystem status |
| `GET` | `/api/stats` | Aggregated counts and intensities |
| `GET` | `/api/pheromones` | List pheromones (paginated, filterable) |
| `POST` | `/api/pheromones` | Deposit a new pheromone |
| `GET` | `/api/pheromones/summary` | Per-kind intensity statistics |
| `POST` | `/api/pheromones/query` | Semantic pheromone search via HDC |
| `GET` | `/api/pheromones/heatmap` | Time-bucketed intensity heatmap |
| `GET` | `/api/pheromones/{id}/projection` | Decay projection for one pheromone |
| `GET` | `/api/knowledge/entries` | List insight entries (paginated, filterable) |
| `POST` | `/api/knowledge/entries` | Post a new insight entry |
| `POST` | `/api/knowledge/entries/{id}/confirm` | Confirm an insight |
| `POST` | `/api/knowledge/entries/{id}/challenge` | Challenge an insight |
| `POST` | `/api/knowledge/decay` | Trigger a decay sweep |
| `GET` | `/api/knowledge/edges` | HDC similarity edges between entries |
| `GET` | `/api/knowledge/search` | Semantic search over entries |
| `GET` | `/api/knowledge/kinds` | List available knowledge kinds |
| `GET` | `/api/agents` | List registered agents |
| `POST` | `/api/agents` | Register a new agent |
| `GET` | `/api/agents/topology` | Force-directed agent interaction graph |
| `GET` | `/api/agents/{id}/trace` | Agent activity trace |
| `GET` | `/api/agents/{id}/heartbeat` | Latest heartbeat for an agent |
| `POST` | `/api/agents/{id}/heartbeat` | Send a heartbeat |
| `GET` | `/api/agents/{id}/stats` | Aggregated stats for an agent |
| `GET` | `/api/ws` | WebSocket streaming (requires `roko` feature) |

---

### Health and stats

#### `GET /api/health`

Health check with subsystem status and counts.

```bash
curl http://127.0.0.1:8545/api/health | jq
```

Response:

```json
{
  "status": "ok",
  "uptime_secs": 3600,
  "chain": {
    "toggles": {
      "hdc": true,
      "knowledge": true,
      "stigmergy": true
    },
    "counts": {
      "insights": 128,
      "pheromones": 42,
      "agents": 5
    }
  }
}
```

#### `GET /api/stats`

Combined statistics for dashboard overview panels. One request, one response, no round trips.

```bash
curl http://127.0.0.1:8545/api/stats | jq
```

Response:

```json
{
  "insights": {
    "total": 128,
    "active": 94,
    "confirmed": 67,
    "challenged": 3,
    "decaying": 24
  },
  "pheromones": {
    "total": 42,
    "threat": 12,
    "opportunity": 18,
    "wisdom": 12,
    "total_intensity": 28.6
  },
  "toggles": {
    "hdc": true,
    "knowledge": true,
    "stigmergy": true
  },
  "timestamp": 1712400300
}
```

---

### Pheromone field

#### `GET /api/pheromones`

List active pheromones with decay projections.

```bash
curl 'http://127.0.0.1:8545/api/pheromones?sort=intensity&order=desc&limit=10' | jq
```

| Query param | Default | Description |
|-------------|---------|-------------|
| `offset` | `0` | Pagination offset |
| `limit` | `100` | Max results per page (capped at 500) |
| `kind` | (all) | Filter by kind: `threat`, `opportunity`, or `wisdom` |
| `min_intensity` | (none) | Floor threshold (float) |
| `sort` | `intensity` | Sort field: `intensity`, `deposited_at`, or `confirmations` |
| `order` | `desc` | Sort direction: `desc` or `asc` |

Response:

```json
{
  "pheromones": [
    {
      "id": "ph:abc123",
      "kind": "threat",
      "intensity": 0.82,
      "base_intensity": 1.0,
      "deposited_at": 1712400000,
      "confirmations": 3,
      "half_life_seconds": 7200,
      "effective_half_life_seconds": 9327,
      "bucket": "threat",
      "decay_projection": {
        "in_1h": 0.74,
        "in_4h": 0.53,
        "in_24h": 0.08
      }
    }
  ],
  "total": 42,
  "offset": 0,
  "limit": 100,
  "timestamp": 1712400300
}
```

Decay projections show predicted intensity at +1h, +4h, and +24h -- useful for animated UI visualizations that show signals fading over time.

#### `POST /api/pheromones`

Deposit a new pheromone into the field. The `content` text is projected into a 10,240-bit HDC vector for semantic search.

```bash
curl -X POST http://127.0.0.1:8545/api/pheromones \
  -H 'Content-Type: application/json' \
  -d '{
    "kind": "threat",
    "content": "flash loan attack draining AAVE v3 WETH pool",
    "intensity": 1.0,
    "half_life_secs": 7200
  }' | jq
```

Request body:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `kind` | string | yes | `"threat"`, `"opportunity"`, or `"wisdom"` |
| `content` | string | yes | Text content (projected to HDC vector) |
| `intensity` | float | yes | Initial intensity (0.0 - any positive) |
| `half_life_secs` | integer | no | Custom half-life in seconds (defaults to kind default: threat=7200, opportunity=14400, wisdom=43200) |

Response:

```json
{
  "id": 7,
  "kind": "threat",
  "intensity": 1.0,
  "deposited_at": 1712400000
}
```

Returns HTTP 503 if stigmergy subsystem is disabled.

#### `GET /api/pheromones/summary`

Aggregated statistics per pheromone kind.

```bash
curl http://127.0.0.1:8545/api/pheromones/summary | jq
```

Response:

```json
{
  "by_kind": {
    "threat": {
      "count": 12,
      "total_intensity": 8.4,
      "avg_intensity": 0.7,
      "min_intensity": 0.1,
      "max_intensity": 1.0
    },
    "opportunity": { "..." : "..." },
    "wisdom": { "..." : "..." }
  },
  "total_count": 42,
  "total_intensity": 28.6,
  "timestamp": 1712400300
}
```

#### `POST /api/pheromones/query`

Top-K similarity search using HDC vectors. Finds pheromones semantically related to a free-text query.

```bash
curl -X POST http://127.0.0.1:8545/api/pheromones/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "flash loan attack on lending protocol", "k": 10}' | jq
```

Request body:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `query` | string | no | Free-text query (projected to HDC internally) |
| `k` | integer | no | Max results (default 10, capped at 100) |

Response:

```json
{
  "results": [
    {
      "id": "ph:abc123",
      "kind": "threat",
      "similarity": 0.87,
      "intensity": 0.82,
      "score": 0.71,
      "deposited_at": 1712400000,
      "confirmations": 3
    }
  ],
  "timestamp": 1712400300
}
```

HDC similarity ranges from 0.0 to 1.0 (Hamming similarity of 10,240-bit vectors). The `score` field is `similarity * intensity`.

#### `GET /api/pheromones/heatmap`

Time-bucketed deposit activity for heatmap and timeline visualizations.

```bash
curl 'http://127.0.0.1:8545/api/pheromones/heatmap?bucket_seconds=3600&since=1712361600' | jq
```

| Query param | Default | Description |
|-------------|---------|-------------|
| `bucket_seconds` | `3600` | Bucket width in seconds (minimum 60) |
| `since` | 24h ago | Unix timestamp for the start of the window |

Response:

```json
{
  "buckets": [
    {
      "timestamp": 1712361600,
      "threat": 3,
      "opportunity": 7,
      "wisdom": 1,
      "total_intensity": 8.2
    }
  ],
  "bucket_seconds": 3600,
  "timestamp": 1712400300
}
```

#### `GET /api/pheromones/{id}/projection`

Decay projection curve for a single pheromone, returning intensity at evenly spaced time points into the future. Useful for rendering per-pheromone decay sparklines.

```bash
curl 'http://127.0.0.1:8545/api/pheromones/7/projection?duration_secs=3600&points=60' | jq
```

| Query param | Default | Description |
|-------------|---------|-------------|
| `duration_secs` | `3600` | How far into the future to project |
| `points` | `60` | Number of evenly spaced data points |

Response:

```json
{
  "id": 7,
  "kind": "threat",
  "base_intensity": 1.0,
  "half_life_secs": 7200,
  "effective_half_life_secs": 7200,
  "points": [
    { "offset_secs": 0, "intensity": 0.95 },
    { "offset_secs": 60, "intensity": 0.94 },
    { "offset_secs": 120, "intensity": 0.93 }
  ]
}
```

Returns HTTP 404 if the pheromone ID does not exist.

---

### Knowledge graph

#### `GET /api/knowledge/entries`

List insight entries with full lifecycle metadata.

```bash
curl 'http://127.0.0.1:8545/api/knowledge/entries?kind=warning&sort=weight&limit=10' | jq
```

| Query param | Default | Description |
|-------------|---------|-------------|
| `offset` | `0` | Pagination offset |
| `limit` | `100` | Max results per page (capped at 500) |
| `kind` | (all) | Filter: `insight`, `heuristic`, `warning`, `causal_link`, `strategy_fragment`, or `anti_knowledge` |
| `state` | (all) | Filter: `created`, `active`, `confirmed`, `decaying`, `challenged`, `pruned`, or `stale` |
| `min_weight` | (none) | Minimum weight threshold (float) |
| `sort` | `weight` | Sort field: `weight`, `created_at`, or `confirmations` |
| `order` | `desc` | Sort direction: `desc` or `asc` |

Response:

```json
{
  "entries": [
    {
      "id": "insight:d4e5f6",
      "kind": "warning",
      "weight": 0.95,
      "initial_weight": 1.0,
      "state": "confirmed",
      "confirmations": 5,
      "challenges": 0,
      "created_at": 1712390000,
      "content": "never call selfdestruct in a proxy implementation contract",
      "author": "agent:alice",
      "enabled_by": [],
      "half_life_seconds": 180,
      "effective_half_life_seconds": 381,
      "stake_wei": "2000000000000000"
    }
  ],
  "total": 128,
  "offset": 0,
  "limit": 100,
  "timestamp": 1712400300
}
```

`stake_wei` is serialized as a string to avoid JSON number precision loss for u128 values.

#### `POST /api/knowledge/entries`

Post a new insight entry into the knowledge store. The `content` text is projected into a 10,240-bit HDC vector for semantic search and similarity edges.

```bash
curl -X POST http://127.0.0.1:8545/api/knowledge/entries \
  -H 'Content-Type: application/json' \
  -d '{
    "kind": "warning",
    "content": "never call selfdestruct in a proxy implementation contract",
    "author": "agent:alice",
    "enabled_by": [],
    "stake_wei": 2000000000000000
  }' | jq
```

Request body:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `kind` | string | yes | `"insight"`, `"heuristic"`, `"warning"`, `"causal_link"`, `"strategy_fragment"`, or `"anti_knowledge"` |
| `content` | string | yes | Text content (projected to HDC vector) |
| `author` | string | no | Author identifier (e.g. `"agent:alice"`) |
| `enabled_by` | string[] | no | Hex IDs of entries this insight depends on |
| `stake_wei` | integer | no | Stake amount in wei (default 0) |

Response:

```json
{
  "id": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
  "kind": "warning",
  "state": "created",
  "weight": 1.0,
  "created_at": 1712400000
}
```

Returns HTTP 503 if the knowledge subsystem is disabled.

#### `POST /api/knowledge/entries/{id}/confirm`

Confirm an existing insight entry. Confirmations extend the entry's effective half-life via `tau_eff = tau * (1 + sqrt(confirms) * 0.5)`.

```bash
curl -X POST http://127.0.0.1:8545/api/knowledge/entries/a1b2c3d4e5f6.../confirm \
  -H 'Content-Type: application/json' \
  -d '{"confirmer": "agent:bob", "stake_wei": 1000000000000000}' | jq
```

Request body:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `confirmer` | string | yes | Agent ID of the confirmer |
| `stake_wei` | integer | no | Additional stake (default 0) |

Response:

```json
{
  "id": "a1b2c3d4...",
  "confirmations": 6,
  "new_state": "confirmed",
  "effective_half_life_seconds": 420,
  "timestamp": 1712400100
}
```

Returns HTTP 404 if the entry does not exist, HTTP 409 if this confirmer already confirmed this entry.

#### `POST /api/knowledge/entries/{id}/challenge`

Challenge an existing insight entry. Challenges reduce the entry's weight and may transition it to the `challenged` state.

```bash
curl -X POST http://127.0.0.1:8545/api/knowledge/entries/a1b2c3d4e5f6.../challenge \
  -H 'Content-Type: application/json' \
  -d '{"challenger": "agent:carol", "stake_wei": 1000000000000000}' | jq
```

Request body:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `challenger` | string | yes | Agent ID of the challenger |
| `stake_wei` | integer | no | Stake amount (default 0) |

Response:

```json
{
  "id": "a1b2c3d4...",
  "challenges": 1,
  "new_state": "challenged",
  "timestamp": 1712400200
}
```

Returns HTTP 404 if the entry does not exist, HTTP 409 if this challenger already challenged this entry.

#### `POST /api/knowledge/decay`

Trigger a manual decay sweep on the knowledge store. Entries below their minimum weight threshold are pruned.

```bash
curl -X POST http://127.0.0.1:8545/api/knowledge/decay \
  -H 'Content-Type: application/json' \
  -d '{}' | jq
```

Request body:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `now_secs` | integer | no | Override current time (useful for testing; defaults to wall clock) |

Response:

```json
{
  "pruned": 3,
  "remaining": 125,
  "timestamp": 1712400300
}
```

#### `GET /api/knowledge/edges`

Dependency and HDC-similarity edges, designed for force-directed graph layouts (d3.js/force-graph).

```bash
curl 'http://127.0.0.1:8545/api/knowledge/edges?similarity_threshold=0.6&max_hdc_edges_per_node=3' | jq
```

Two types of edges appear in the response:

- **`enabled_by`** -- explicit dependency edges from the knowledge graph (entry A was enabled by entry B).
- **`hdc`** -- implicit similarity edges computed from HDC vector proximity above the configured threshold.

| Query param | Default | Description |
|-------------|---------|-------------|
| `similarity_threshold` | `0.5` | Minimum HDC similarity for implicit edges |
| `max_hdc_edges_per_node` | `5` | Cap on HDC edges per node (prevents hairball graphs) |
| `include_enabled_by` | `true` | Include explicit dependency edges |
| `include_hdc` | `true` | Include HDC similarity edges |

Response:

```json
{
  "edges": [
    { "from": "insight:aaa", "to": "insight:bbb", "type": "enabled_by" },
    { "from": "insight:aaa", "to": "insight:ccc", "similarity": 0.73, "type": "hdc" }
  ],
  "node_count": 128,
  "timestamp": 1712400300
}
```

#### `GET /api/knowledge/search`

Semantic search over knowledge entries via HDC projection.

```bash
curl 'http://127.0.0.1:8545/api/knowledge/search?q=proxy+destruction+safety&k=5' | jq
```

| Query param | Default | Description |
|-------------|---------|-------------|
| `q` | (required) | Free-text search query |
| `k` | `10` | Max results (capped at 100) |
| `kind` | (all) | Optional kind filter |

Response:

```json
{
  "results": [
    {
      "id": "insight:d4e5f6",
      "kind": "warning",
      "similarity": 0.91,
      "weight": 0.95,
      "score": 0.86,
      "content": "never call selfdestruct in a proxy implementation contract",
      "state": "confirmed",
      "author": "agent:alice",
      "created_at": 1712390000,
      "confirmations": 5
    }
  ],
  "query": "proxy destruction safety",
  "timestamp": 1712400300
}
```

Returns HTTP 503 if HDC or knowledge subsystem is disabled.

#### `GET /api/knowledge/kinds`

Enumerate knowledge and pheromone kind metadata, including current entry counts and default half-lives.

```bash
curl http://127.0.0.1:8545/api/knowledge/kinds | jq
```

Response:

```json
{
  "knowledge_kinds": [
    {
      "name": "insight",
      "default_half_life_seconds": 604800,
      "base_reward_wei": 1000000000000000,
      "count": 42
    }
  ],
  "pheromone_kinds": [
    {
      "name": "threat",
      "default_half_life_seconds": 7200,
      "count": 12
    }
  ],
  "timestamp": 1712400300
}
```

---

### Agent registry

#### `GET /api/agents`

List all registered agents with summary stats.

```bash
curl http://127.0.0.1:8545/api/agents | jq
```

Response:

```json
{
  "agents": [
    {
      "id": "agent:alice",
      "role": "researcher",
      "registered_at": 1712390000,
      "last_heartbeat_block": 19500000,
      "last_heartbeat_ts": 1712399000,
      "stats": {
        "confirmations_given": 18,
        "challenges_given": 2,
        "warnings_posted": 5,
        "total_tokens": 150000,
        "total_cost_usd": 4.50,
        "tasks_completed": 12
      }
    }
  ],
  "total": 5,
  "timestamp": 1712400300
}
```

#### `POST /api/agents`

Register a new agent in the registry.

```bash
curl -X POST http://127.0.0.1:8545/api/agents \
  -H 'Content-Type: application/json' \
  -d '{"id": "agent:alice", "pubkey": "0xabc123", "role": "researcher"}' | jq
```

Request body:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Unique agent identifier |
| `pubkey` | string | no | Public key or address (stored as bytes) |
| `role` | string | yes | Agent role (e.g. `"researcher"`, `"validator"`, `"executor"`) |

Response (success):

```json
{
  "registered": true,
  "agent_id": "agent:alice",
  "role": "researcher",
  "registered_at": 1712400000
}
```

Returns HTTP 400 if `id` or `role` is empty, HTTP 409 if the agent is already registered.

#### `GET /api/agents/topology`

Agent interaction graph derived from confirmation and challenge activity in the knowledge store. Returns nodes and weighted edges suitable for d3.js force-directed layouts.

```bash
curl http://127.0.0.1:8545/api/agents/topology | jq
```

Response:

```json
{
  "nodes": [
    {
      "id": "agent:alice",
      "address": "0x616c696365",
      "insights_posted": 24,
      "confirmations_given": 18,
      "challenges_given": 2,
      "total_weight": 19.4
    }
  ],
  "edges": [
    {
      "from": "agent:alice",
      "to": "agent:bob",
      "weight": 12,
      "type": "confirmed"
    },
    {
      "from": "agent:carol",
      "to": "agent:alice",
      "weight": 1,
      "type": "challenged"
    }
  ],
  "timestamp": 1712400300
}
```

#### `GET /api/agents/{id}/trace`

Activity trace for a single agent: recent confirmations, challenges, and insight posts.

```bash
curl 'http://127.0.0.1:8545/api/agents/agent:alice/trace?limit=20&offset=0' | jq
```

| Query param | Default | Description |
|-------------|---------|-------------|
| `limit` | `10` | Max trace entries to return |
| `offset` | `0` | Pagination offset |

Response:

```json
{
  "agent_id": "agent:alice",
  "trace": [
    {
      "type": "confirmation",
      "entry_id": "a1b2c3d4...",
      "timestamp": 1712399500
    },
    {
      "type": "post",
      "entry_id": "d4e5f6a7...",
      "kind": "warning",
      "timestamp": 1712399000
    }
  ],
  "total": 42,
  "offset": 0,
  "limit": 20
}
```

Returns HTTP 404 if the agent is not found.

#### `GET /api/agents/{id}/heartbeat`

Latest heartbeat data for a specific agent.

```bash
curl http://127.0.0.1:8545/api/agents/agent:alice/heartbeat | jq
```

Response:

```json
{
  "agent_id": "agent:alice",
  "last_heartbeat_block": 19500000,
  "last_heartbeat_ts": 1712399000,
  "alive": true
}
```

Returns a JSON object with `"error": "agent not found"` if the agent is not registered.

#### `POST /api/agents/{id}/heartbeat`

Send a heartbeat from an agent. Optionally report token usage and cost.

```bash
curl -X POST http://127.0.0.1:8545/api/agents/agent:alice/heartbeat \
  -H 'Content-Type: application/json' \
  -d '{"tokens_used": 5000, "cost_usd": 0.15, "tasks_completed": 1}' | jq
```

Request body:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tokens_used` | integer | no | Tokens consumed since last heartbeat (default 0) |
| `cost_usd` | float | no | Cost in USD since last heartbeat (default 0.0) |
| `tasks_completed` | integer | no | Tasks completed since last heartbeat (default 0) |

Response:

```json
{
  "ok": true,
  "agent_id": "agent:alice",
  "timestamp": 1712400100
}
```

Returns HTTP 404 if the agent is not registered.

#### `GET /api/agents/{id}/stats`

Aggregated lifetime statistics for an agent.

```bash
curl http://127.0.0.1:8545/api/agents/agent:alice/stats | jq
```

Response:

```json
{
  "agent_id": "agent:alice",
  "confirmations_given": 18,
  "challenges_given": 2,
  "warnings_posted": 5,
  "total_tokens": 150000,
  "total_cost_usd": 4.50,
  "tasks_completed": 12
}
```

Returns a default zeroed stats object if the agent is not found.

---

### WebSocket streaming (requires `roko` feature)

#### `WS /api/ws`

Live event stream from the pheromone and insight buses. Connect via any WebSocket client.

```bash
# Subscribe to both channels
websocat ws://127.0.0.1:8545/api/ws

# Pheromones only
websocat 'ws://127.0.0.1:8545/api/ws?insights=false'

# Insights only
websocat 'ws://127.0.0.1:8545/api/ws?pheromones=false'
```

| Query param | Default | Description |
|-------------|---------|-------------|
| `pheromones` | `true` | Subscribe to pheromone deposits |
| `insights` | `true` | Subscribe to insight lifecycle events |

#### Wire format

Each WebSocket text frame contains a JSON object with a `"channel"` field indicating the event source.

**Connection acknowledgment** (sent immediately on connect):

```json
{"type": "connected", "pheromones": true, "insights": true}
```

**Pheromone event** (emitted on deposit):

```json
{
  "channel": "pheromone",
  "data": {
    "id": "ph:abc123",
    "kind": "threat",
    "intensity": 1.0,
    "depositedAt": 1712400000
  }
}
```

**Insight event** (emitted on lifecycle transitions):

```json
{
  "channel": "insight",
  "data": {
    "type": "posted",
    "id": "insight:d4e5f6",
    "kind": "warning",
    "author": "agent:alice"
  }
}
```

Insight event types: `posted`, `stateTransition`, `confirmed`, `challenged`, `decayed`.

**Lag warning** (client fell behind the broadcast buffer):

```json
{"type": "lagged", "channel": "pheromone", "missed": 12}
```

The `lagged` frame means the client fell behind the broadcast buffer. Missed events are dropped -- reconnect or increase consumption speed.

## Roko bridge (feature = `roko`)

The `roko` feature adds trait implementations that let `mirage-rs` slot directly into a roko agent's universal loop (query substrate → score → route/compose → gate → write back → policy).

Three adapters are exposed in `mirage_rs::roko_bridge`:

| Type | Trait impl | Role |
|------|-----------|------|
| `SimulationGate` | `roko_core::Gate` | Runs a planned transaction through the mirage fork; `Verdict.passed` = would succeed on mainnet |
| `HdcSubstrate` | `roko_core::Substrate` | Raw HDC semantic index (no lifecycle) — good for throwaway context pools |
| `ChainSubstrate` | `roko_core::Substrate` | Full `InsightEntry` lifecycle, content-hashed, with confirm/challenge/decay |

### Using `SimulationGate` in a roko loop

```rust
use mirage_rs::{
    fork::{ForkState, HybridDB, MirageFork},
    provider::UpstreamRpc,
    resources::{MirageMode, Profile, ResourceModel},
    roko_bridge::SimulationGate,
};
use roko_core::{Body, Context, Kind, Provenance, Signal, traits::Gate};
use std::{num::NonZeroUsize, sync::Arc, time::Duration};

// Build a mirage fork
let upstream = Arc::new(UpstreamRpc::new_for_url(
    "https://ethereum-rpc.publicnode.com",
)?);
let db = HybridDB::new(upstream, 1024, Duration::from_secs(12), NonZeroUsize::MIN, 1);
let fork = ForkState::new(db, 0, 1);
let mirage = MirageFork::new(
    fork,
    ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
    MirageMode::Live,
);

// Wire a SimulationGate
let gate = SimulationGate::new(mirage);

// Golem presents a planned tx as a Signal
let planned = Signal::builder(Kind::Transaction)
    .body(Body::Json(serde_json::json!({
        "from": "0x1111111111111111111111111111111111111111",
        "to":   "0x2222222222222222222222222222222222222222",
        "gas":  "0x5208",
        "value": "0x0",
        "data": "0x"
    })))
    .provenance(Provenance::agent("golem:alice"))
    .build();

// Verify before committing to mainnet
let verdict = gate.verify(&planned, &Context::now()).await;
if verdict.passed {
    // Safe to submit to real chain.
} else {
    // Signal came back with reason + detail (gas_used, output, revert reason).
    eprintln!("tx would revert: {}", verdict.reason);
}
```

### Using `ChainSubstrate` as semantic memory

```rust
use mirage_rs::roko_bridge::ChainSubstrate;
use roko_core::{Body, Context, Kind, Provenance, Query, Score, Signal, traits::Substrate};

let memory = ChainSubstrate::new("golem-shared-memory");

// Store an insight (automatically HDC-projected and lifecycle-tracked)
let insight = Signal::builder(Kind::Insight)
    .body(Body::text(
        "uniswap v3 STF revert means insufficient allowance",
    ))
    .provenance(Provenance::agent("golem:alice"))
    .score(Score::new(0.9, 0.5, 1.0, 1.0))
    .build();
let hash = memory.put(insight).await?;

// Other agents confirm it
memory.confirm(hash, b"golem:bob".to_vec());
memory.confirm(hash, b"golem:carol".to_vec());

// Retrieve via semantic similarity — uses `text_query` tag for HDC projection
let q = Query::all()
    .with_tag("text_query", "uniswap swap failed with STF error")
    .limit(5);
let hits = memory.query(&q, &Context::now()).await?;
for signal in hits {
    println!("- {}", signal.body.as_text()?);
}

// Periodically apply decay (typically driven by a policy loop)
memory.apply_decay(std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)?
    .as_secs());
```

### End-to-end example

See [`tests/roko_e2e.rs`](tests/roko_e2e.rs) for a full golem loop:

1. Golem A plans a transaction
2. `SimulationGate` verifies it would succeed
3. Golem A posts an `InsightEntry` documenting the learned behaviour
4. Another agent confirms
5. Golem B retrieves it via HDC semantic search before acting
6. Decay runs, lifecycle states advance

```bash
cargo test -p mirage-rs --features roko --test roko_e2e
```

## Architecture

```
                        +-----------------+
                        |  Your app /     |
                        |  Foundry / test |
                        +--------+--------+
                                 |
                            JSON-RPC
                                 |
                        +--------v--------+
                        |   RPC Server    |
                        |  (axum + json-  |
                        |   rpsee)        |
                        +--------+--------+
                                 |
                        +--------v--------+
                        |   MirageFork    |
                        |  (Arc<RwLock>)  |
                        +--------+--------+
                                 |
              +------------------+------------------+
              |                  |                  |
     +--------v------+  +-------v-------+  +-------v--------+
     |  DirtyStore   |  |  ReadCache    |  |  UpstreamRpc   |
     | (local writes)|  | (LRU + TTL)   |  | (rate-limited) |
     +---------------+  +---------------+  +-------+--------+
                                                   |
                                           +-------v--------+
                                           | Ethereum node  |
                                           | (HTTP + WS)    |
                                           +-------+--------+
                                                   |
                                          +--------v--------+
                                          | Targeted        |
                                          | Follower        |
                                          | (newHeads sub,  |
                                          |  selective      |
                                          |  replay)        |
                                          +-----------------+
```

### Core types

**State layer:**

- `MirageFork` — thread-safe handle (`Arc<RwLock<MirageState>>`) shared across the RPC server, follower, and scenario runner.
- `ForkState` — mutable fork state: `HybridDB`, block number, chain ID, timestamp, watch list, snapshot stack, dirty tracking.
- `HybridDB` — three-layer database: `DirtyStore` (local writes) → `ReadCache` (LRU with TTL) → `UpstreamRpc` (lazy fetches).
- `DirtyAccount` / `DirtyStore` — tracks accounts and storage slots modified since the last snapshot or baseline.

**Copy-on-write:**

- `CowState` — shared baseline + per-branch overlay. Branches read from the overlay first, then fall through to the shared baseline. Writes go only to the overlay.
- `MultiVersionStore` — per-slot multi-version storage for the Block-STM test harness.
- `BytecodeCache` — LRU keyed by code hash. Bytecode is immutable, so no CoW needed.

**Upstream:**

- `UpstreamRpc` — wraps `reqwest::blocking::Client` with a token-bucket rate limiter, retries with exponential backoff, and a mock mode for offline testing.
- `ReadCache` — LRU cache with per-entry TTL. Tracks hit/miss counts and supports targeted eviction under memory pressure.

**Replay and speculative execution:**

- `TargetedFollower` — subscribes to `newHeads` via WebSocket, replays only transactions touching watched contracts.
- `SpeculativeExecutor` — runs transactions against a CoW branch without mutating base state. Returns execution result + full `StateDiff` + read set for invalidation tracking.
- `TxReplay` — fetches a historical transaction by hash from upstream and re-executes it locally.
- `DiffClassifier` — inspects state diffs and classifies contracts as `Protocol` (complex, should be watched), `SlotOnly` (simple override), or `ReadOnly`.

**Scenarios:**

- `ScenarioRunner` — orchestrates scenario sets with `Sequential` (revert between runs) or `Parallel` (independent CoW branches) execution modes.
- `Scenario` — named transaction sequence with tracked addresses, gas budget, timeout, and assertions.

**Integration:**

- `MirageClient` / `MirageConfig` — async HTTP client wrapping all RPC methods with retry and timeout.
- `MirageTestInstance` — spawned child process with config access and clean shutdown.
- `EventFilter` / `MirageEvent` — WebSocket event subscription with address/topic filters, carrying provenance (`LocalTx` or `FollowerReplay`).

## Error Handling

All library errors are `MirageError`. Each variant maps to a JSON-RPC error code:

| Variant | Code | When |
|---------|------|------|
| `InvalidParams` | -32602 | Malformed RPC parameters |
| `Unsupported` | -32603 | Operation not supported in current mode |
| `InvalidFrom` | -32010 | Invalid sender address |
| `SnapshotNotFound` | -32001 | Snapshot ID doesn't exist or was consumed |
| `SlotDetectionFailed` | -32020 | ERC-20 balance slot detection failed |
| `WatchListFull` | -32030 | Watch list at capacity for the current profile |
| `UnknownProtocolType` | -32040 | Position helper doesn't recognize the protocol |
| `SetNotFound` | -32050 | Scenario set doesn't exist |
| `JobNotFound` | -32054 | Scenario job doesn't exist |
| `JobNotComplete` | -32055 | Scenario job still running |
| `Upstream` | -32099 | Upstream RPC failure |
| `Timeout` | -32603 | Operation exceeded its time budget |
| `BindFailed` | -32603 | Could not bind the server port |

## Testing

```bash
# Default build — pure EVM fork simulator (126 unit tests)
cargo test -p mirage-rs --lib

# With chain extensions (182 unit tests: default + 56 chain)
cargo test -p mirage-rs --features chain --lib

# With the roko bridge (200 unit tests: chain + 18 bridge)
cargo test -p mirage-rs --features roko --lib

# End-to-end roko golem loop (4 tests: gate → post → retrieve → decay)
cargo test -p mirage-rs --features roko --test roko_e2e

# Process-spawning integration tests (spawns real mirage binaries on ports)
cargo test -p mirage-rs --test integration
```

## Cargo Features

| Feature | Default | Description |
|---------|---------|-------------|
| `binary` | yes | Includes the CLI entrypoint (required to build `src/main.rs`) |
| `library` | no | Library-only builds (no binary dependencies) |
| `sim-gas` | no | Gas simulation instrumentation |
| `chain` | no | HDC index + `InsightEntry` knowledge layer + stigmergy pheromones + `chain_*` JSON-RPC methods |
| `roko` | no | Implements `roko_core::{Gate, Substrate}` over mirage (implies `chain`) |

Feature flags compose additively:

```bash
cargo build -p mirage-rs                                    # binary (default)
cargo build -p mirage-rs --no-default-features -F library   # library-only
cargo build -p mirage-rs -F chain                           # EVM + knowledge layer
cargo build -p mirage-rs -F roko                            # EVM + knowledge + roko traits
```

## Startup artifacts

On startup, mirage writes two files:

- `/tmp/mirage-{port}.pid` — process ID
- `/tmp/mirage-{port}-status.json` — `{"status":"ready","port":N}`

Both are cleaned up on shutdown. Use the status file for CI health checks or orchestrator readiness probes.

## State persistence

mirage-rs can persist all in-memory state to a single JSON snapshot file so that data survives process restarts. This is especially useful on platforms like Railway where deploys wipe ephemeral filesystem state.

### What is persisted

| Persisted | Skipped (rebuilt on startup) |
|---|---|
| Dirty accounts (balances, nonces, code, storage) | Read cache (LRU+TTL, re-fetched from upstream) |
| Local transactions, receipts, blocks | Event bus channels (reconnected) |
| Impersonated accounts | Speculative executor cache |
| Watch list and unwatch list | EVM revert snapshots |
| Deployed contract bytecode | Upstream RPC handle |
| Fork metadata (block number, chain ID, timestamp) | |
| Knowledge store entries (insights) | HDC/HNSW indices (recomputed from entries) |
| Pheromone field (stigmergic signals) | |
| Agent registry (identities, traces, stats) | |
| Task store (all tasks + state) | |
| Prediction store (sessions + claims) | |

### How it works

1. **Periodic snapshots**: A background task captures state every `--snapshot-interval-secs` seconds (default 30). Serialization happens outside any lock, so RPC serving is never blocked.
2. **Atomic writes**: Each snapshot is written to a `.tmp` file, then renamed to the final path. If the process crashes mid-write, the previous valid snapshot is preserved.
3. **Shutdown snapshot**: On graceful shutdown (SIGTERM or Ctrl+C), a final snapshot is written before the process exits.
4. **Startup restore**: On startup, mirage looks for a snapshot in `--state-dir`. If found, it restores all state and sets the block number so the targeted follower catches up from where it left off.

The snapshot file is plain JSON (~5-50 KB for typical usage, larger with many deployed contracts), readable with `cat` or `jq` for debugging.

### Quick example

```bash
# Start mirage with persistence (default: .roko/state/, 30s interval)
mirage-rs --rpc-url https://ethereum-rpc.publicnode.com \
  --enable-hdc --enable-knowledge --enable-stigmergy

# Seed some data (agents, insights, deploy contracts, etc.)
# ...

# Kill and restart — all state is preserved
kill $(cat /tmp/mirage-8545.pid)
mirage-rs --rpc-url https://ethereum-rpc.publicnode.com \
  --enable-hdc --enable-knowledge --enable-stigmergy
# Logs will show: "restored snapshot from .roko/state"
```

### Customizing persistence

```bash
# Custom state directory and faster snapshots
mirage-rs --state-dir /data/mirage --snapshot-interval-secs 10

# Disable persistence entirely (useful for tests)
mirage-rs --no-persist

# Inspect a snapshot file
jq '.fork.local_block_number, .chain.knowledge.entries | length' \
  .roko/state/mirage-snapshot.json
```

### Deploying on Railway

Railway wipes the filesystem on each deploy, but you can persist state using a Railway volume:

1. **Create a volume** in the Railway dashboard, mounted at `/workspace/.roko` (or any path).
2. **Set the state dir** to point inside the volume:

```bash
# In your Railway service start command or Dockerfile CMD:
mirage-rs --host 0.0.0.0 --port 8545 \
  --enable-hdc --enable-knowledge --enable-stigmergy \
  --state-dir /workspace/.roko/state \
  --snapshot-interval-secs 15
```

3. On deploy, mirage picks up the previous snapshot from the volume and resumes from where it left off. Seeded agents, insights, pheromones, tasks, deployed contracts, and account state all survive.

Railway gives 10 seconds of SIGTERM grace on shutdown — the final snapshot is written within that window. With a 15-second interval, worst-case data loss is 15 seconds of state (and that state is typically recoverable since block data can be replayed from upstream).

### Docker Compose

The `docker/docker-compose.yml` already mounts a named volume. Add persistence flags to the mirage service:

```yaml
services:
  mirage:
    # ...existing config...
    volumes:
      - mirage-state:/workspace/.roko
    command:
      - "--host"
      - "0.0.0.0"
      - "--port"
      - "8545"
      - "--enable-hdc"
      - "--enable-knowledge"
      - "--enable-stigmergy"
      - "--state-dir"
      - "/workspace/.roko/state"
      - "--snapshot-interval-secs"
      - "15"

volumes:
  mirage-state:
```

## Anvil compatibility at a glance

| Capability | Anvil | mirage-rs |
|------------|-------|-----------|
| Fork from RPC | Yes (pinned block) | Yes (latest, follows forward) |
| `eth_*` methods | Full | Common DeFi subset |
| `evm_snapshot` / `evm_revert` | Yes | Yes |
| `anvil_setBalance` / `setStorageAt` / etc. | Yes | Yes (also `hardhat_*` and `mirage_*` prefixes) |
| `evm_increaseTime` / `evm_setNextBlockTimestamp` | Yes | Yes |
| Auto-mine | Yes | Yes |
| Impersonate accounts | Yes | Yes (relaxed signing by default) |
| Live block following | No | Yes (targeted follower via WebSocket) |
| Contract auto-classification | No | Yes (diff classifier + contagion) |
| Copy-on-write scenario branching | No | Yes |
| ERC-20 balance slot detection + mint | No | Yes (`mirage_mintERC20`) |
| Memory pressure management | No | Yes (tiered eviction/demotion) |
| Resource profiles | No | Yes (`micro` / `standard` / `power`) |
| State persistence across restarts | No (`--dump-state` file only) | Yes (periodic atomic JSON snapshots) |
