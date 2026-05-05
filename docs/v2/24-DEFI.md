# 24 -- DeFi Infrastructure

> The economic substrate. ISFR oracle, yield perpetuals, cooperative clearing, multi-chain data aggregation, and agent DeFi capabilities. All DeFi types are domain-specific Cell specializations implementing standard protocols. Every trade flows through the DeFiRiskEngine (a Verify-protocol Cell) before execution.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse for trade events), [02-CELL](02-CELL.md) (Cell trait, Verify protocol for risk checks), [03-GRAPH](03-GRAPH.md) (Graph composition for trading pipelines), [05-AGENT](05-AGENT.md) (Agent runtime, daimon affect engine), [07-LEARNING](07-LEARNING.md) (continuous P&L reward, cascade router updates), [13-TRIGGERS](13-TRIGGERS.md) (heartbeat clock for tick processing), [22-REGISTRIES](22-REGISTRIES.md) (on-chain contract anchoring)

---

## 1. Design Constraints

1. **Safety first, speed second.** Every DeFi operation flows through the DeFiRiskEngine (a Verify-protocol Cell) before execution. No agent can bypass position limits, drawdown caps, or MEV protection.
2. **Continuous reward, not binary.** DeFi outcomes produce P&L -- a continuous Signal. The learning pipeline ([doc-07](07-LEARNING.md)) replaces binary gate-pass reward with risk-adjusted return.
3. **Venue-agnostic execution.** Agents interact with DeFi protocols through a VenueAdapter trait (a Cell specialization). Adding a new protocol means implementing one Cell, not rewriting agent logic.
4. **Affect modulation is real.** Position sizing passes through the daimon affect engine. Losses are weighted 2.25x per prospect theory (Tversky & Kahneman 1992). This prevents agents from doubling down after drawdowns.
5. **Multi-chain by default.** ISFR components come from Ethereum, Base, and Arbitrum. The system aggregates cross-chain data into a single rate.
6. **Simulation before execution.** Trades run through mirage-rs fork simulation before hitting live chains. The `TxSimulator` trait abstracts this.

---

## 2. Kernel Mapping

All DeFi types are domain-specific Cell specializations. No new kernel primitives are introduced.

| DeFi Concept | Kernel Primitive | Protocol | Notes |
|---|---|---|---|
| ISFR Oracle | Cell | Score + Verify | Produces rated Signals (ISFR snapshots) with ground truth from chain state |
| Yield Perpetual | Signal (Position kind) | -- | Position state tracked as Signals in Store, anchored on-chain |
| ClearingHouse | Cell | Compose | VCG welfare-maximizing settlement = Compose-protocol clearing |
| VenueAdapter | Cell | Act | Protocol-normalized execution: swap, add/remove liquidity, quote |
| DeFiRiskEngine | Cell | Verify | Pre-trade risk check producing RiskVerdict Signals |
| TradingReflect | Cell | Score | FIFO P&L attribution producing continuous reward Signals |
| ChainDataSource | Cell | Substrate | Data ingestion from EVM chains into Store |
| ChainDataAggregator | Graph | Compose | Composes multiple ChainDataSource Cells into unified state |
| Affect-modulated sizing | Cell | Route | Daimon state modulates position size via prospect_value |
| Price feed ingestion | Hot Flow | -- | Gamma-tick (1-5s) Flow bound to heartbeat clock |

---

## 3. ISFR Oracle

The Internet Secured Funding Rate is a benchmark rate computed from DeFi lending markets. It answers the question: what is the risk-free rate of return available on-chain right now?

ISFR is the reference rate for yield perpetuals, agent compensation models, and cost-of-capital calculations across the system.

### 3.1 Rate Computation

ISFR aggregates weighted lending rates from major DeFi protocols. Each protocol supplies a "component" -- a lending rate for a specific stablecoin market. Components are weighted by TVL (total value locked) in each market.

The aggregation uses a **dual-median** approach: for each validator, compute the TVL-weighted median of submitted components; then take the median across all validator submissions. This resists outlier manipulation -- a single corrupted rate source cannot skew the benchmark.

```
ISFR = median_across_validators(
    for each validator v:
        tvl_weighted_median(v.components)
)
```

### 3.2 Components

| Component | Source Chain | Market | Weight Basis |
|---|---|---|---|
| Aave USDC | Ethereum | aUSDC supply rate | TVL in Aave USDC pool |
| Aave USDT | Ethereum | aUSDT supply rate | TVL in Aave USDT pool |
| Compound USDC | Ethereum | cUSDC supply rate | TVL in Compound USDC pool |
| Morpho USDC | Ethereum | Morpho optimizer rate | TVL in Morpho USDC vault |
| Aave USDC (Base) | Base | aUSDC supply rate | TVL in Aave Base USDC |
| Aave USDC (Arb) | Arbitrum | aUSDC supply rate | TVL in Aave Arbitrum USDC |

Additional components can be registered through governance. The minimum component count for a valid ISFR update is 3.

### 3.3 Update Cadence

ISFR updates every 8 hours under normal conditions. An immediate update triggers when the computed rate deviates from the current on-chain rate by more than 50 basis points. Validators can also force an update through a quorum vote.

### 3.4 Solidity Interface

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IISFROracle {
    struct Component {
        bytes32 sourceId;       // Protocol identifier (e.g., keccak256("aave-usdc-eth"))
        uint256 rate;           // Rate in basis points (1 bps = 0.01%)
        uint256 tvl;            // TVL in USD (18 decimals)
        uint64  chain;          // Source chain ID
        uint64  timestamp;      // When the rate was observed
    }

    struct RateSnapshot {
        uint256 rate;           // ISFR in basis points
        uint64  timestamp;      // When this snapshot was computed
        uint64  blockNumber;    // Block at which it was recorded
        uint8   componentCount; // Number of components used
        bytes32 merkleRoot;     // Merkle root of component data
    }

    /// Submit a batch of rate components for aggregation.
    /// Only callable by registered validators.
    function submitComponents(Component[] calldata components) external;

    /// Trigger aggregation. Reverts if quorum not met or cadence not elapsed.
    function aggregate() external;

    /// Current ISFR rate snapshot.
    function getCurrentRate() external view returns (RateSnapshot memory);

    /// Historical rate at a specific block.
    function getRateAt(uint64 blockNumber) external view returns (RateSnapshot memory);

    /// All components used in the most recent aggregation.
    function getComponents() external view returns (Component[] memory);

    /// Whether an update is due (cadence elapsed or deviation exceeded).
    function isUpdateDue() external view returns (bool);

    // Events
    event ComponentSubmitted(address indexed validator, bytes32 sourceId, uint256 rate);
    event RateAggregated(uint256 rate, uint64 timestamp, uint8 componentCount);
    event DeviationTriggered(uint256 oldRate, uint256 newRate, uint256 deviationBps);
}
```

### 3.5 Rust Types

```rust
/// A single lending rate observation from a DeFi protocol.
/// Implemented as a Cell producing Score-protocol Signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsfrComponent {
    /// Protocol identifier (e.g., "aave-usdc-eth").
    pub source_id: String,
    /// Annualized lending rate as a decimal (e.g., 0.0435 for 4.35%).
    pub rate: f64,
    /// Total value locked in USD.
    pub tvl_usd: f64,
    /// Source chain ID.
    pub chain_id: u64,
    /// Observation timestamp (Unix seconds).
    pub observed_at: u64,
}

/// Aggregated ISFR snapshot. Stored as a Signal in the Store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsfrSnapshot {
    /// ISFR as a decimal (e.g., 0.0412 for 4.12%).
    pub rate: f64,
    /// Components used in this aggregation.
    pub components: Vec<IsfrComponent>,
    /// Block number at which this snapshot was recorded.
    pub block_number: u64,
    /// Snapshot timestamp (Unix seconds).
    pub timestamp: u64,
}
```

---

## 4. Yield Perpetuals

Perpetual contracts that settle against the ISFR. A yield perpetual lets a user take a long or short position on the direction of on-chain lending rates. Long = betting rates go up. Short = betting rates go down.

Yield perpetuals are the primary tradable instrument in the system. Agents and humans can open, close, and manage positions. Clearing happens cooperatively at regular intervals.

### 4.1 Position Lifecycle

1. **Open**: Agent or user submits an `openPosition` call with side (long/short), size, and collateral.
2. **Mark**: Between clearing rounds, positions accrue unrealized P&L based on the current ISFR vs. their entry rate.
3. **Settle**: During a clearing round, funding payments flow between longs and shorts based on rate movement.
4. **Close**: Agent or user closes the position, realizing P&L and reclaiming remaining collateral.

### 4.2 Margin and Liquidation

- **Initial margin**: 10% of notional.
- **Maintenance margin**: 5% of notional.
- When a position's margin ratio falls below maintenance, it becomes liquidatable.
- Liquidation is permissionless -- any address can liquidate an undercollateralized position and receive a **2% bonus** from the liquidated margin.

### 4.3 Cooperative Clearing

Clearing uses a VCG (Vickrey-Clarke-Groves) welfare-maximizing auction -- the same Compose-protocol mechanism used for context assembly and bounty matching. Each clearing round:

1. Collect all pending settlement obligations.
2. Compute the welfare-maximizing allocation (who pays whom, how much).
3. Execute settlement atomically.
4. Distribute any surplus proportionally.

VCG ensures truthful reporting of obligations. The clearing contract runs every **30 minutes** or every **150 blocks**, whichever comes first.

### 4.4 Solidity Interface

```solidity
interface IClearingHouse {
    struct Position {
        uint128 id;
        address owner;
        bool    isLong;
        uint256 size;           // Notional in USDC (18 decimals)
        uint256 entryRate;      // ISFR at entry (basis points)
        uint256 collateral;     // Posted collateral in USDC (18 decimals)
        uint64  openedAtBlock;
        uint64  lastSettledBlock;
    }

    struct ClearingRound {
        uint128 roundId;
        uint256 clearingRate;   // ISFR snapshot used for this round
        uint256 totalLongSize;
        uint256 totalShortSize;
        uint256 fundingPaid;    // Total funding transferred this round
        uint64  settledAtBlock;
    }

    /// Open a new yield perpetual position.
    /// Requires prior ERC-20 approval for collateral transfer.
    function openPosition(
        bool isLong,
        uint256 size,
        uint256 collateral
    ) external returns (uint128 positionId);

    /// Close an existing position. Sends realized P&L + remaining collateral to owner.
    function closePosition(uint128 positionId) external;

    /// Add collateral to an existing position.
    function addCollateral(uint128 positionId, uint256 amount) external;

    /// Remove excess collateral (must remain above initial margin after removal).
    function removeCollateral(uint128 positionId, uint256 amount) external;

    /// Execute a clearing round. Permissionless -- anyone can trigger if the
    /// cadence condition is met.
    function settle() external returns (uint128 roundId);

    /// Liquidate an undercollateralized position. Caller receives 2% bonus.
    function liquidate(uint128 positionId) external;

    /// Query functions.
    function getPosition(uint128 positionId) external view returns (Position memory);
    function getPositionsByOwner(address owner) external view returns (Position[] memory);
    function getLatestRound() external view returns (ClearingRound memory);
    function isLiquidatable(uint128 positionId) external view returns (bool);
    function nextSettlementBlock() external view returns (uint64);

    // Events
    event PositionOpened(uint128 indexed positionId, address indexed owner, bool isLong, uint256 size);
    event PositionClosed(uint128 indexed positionId, uint256 realizedPnl, bool profitable);
    event RoundSettled(uint128 indexed roundId, uint256 clearingRate, uint256 fundingPaid);
    event PositionLiquidated(uint128 indexed positionId, address indexed liquidator, uint256 bonus);
    event CollateralAdded(uint128 indexed positionId, uint256 amount);
    event CollateralRemoved(uint128 indexed positionId, uint256 amount);
}
```

### 4.5 Rust Types

```rust
/// A yield perpetual position tracked in the agent runtime.
/// Stored as a Signal (Position kind) in the Store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YieldPerpPosition {
    pub id: u128,
    pub owner: Address,
    pub is_long: bool,
    /// Notional size in USD.
    pub size_usd: f64,
    /// ISFR at entry as a decimal.
    pub entry_rate: f64,
    /// Current collateral in USD.
    pub collateral_usd: f64,
    /// Block at which the position was opened.
    pub opened_at_block: u64,
    /// Unrealized P&L based on current ISFR.
    pub unrealized_pnl_usd: f64,
    /// Current margin ratio (collateral / notional).
    pub margin_ratio: f64,
}

/// Clearing round summary from the chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearingRoundSummary {
    pub round_id: u128,
    pub clearing_rate: f64,
    pub total_long_size_usd: f64,
    pub total_short_size_usd: f64,
    pub funding_paid_usd: f64,
    pub settled_at_block: u64,
}
```

---

## 5. Multi-Chain Data Architecture

ISFR components come from protocols deployed across multiple EVM chains. Agents need cross-chain data to compute accurate rates, monitor positions across chains, and (eventually) execute cross-chain strategies.

### 5.1 Architecture

```
Ethereum RPC ──────┐
                    |
Base RPC ───────────┼──> ChainDataAggregator (Graph) ──> Store (CorticalState)
                    |         |
Arbitrum RPC ───────┘         |
                              ├──> ISFR Oracle Cell (components)
                              └──> Agent context (multi-chain state)
```

Each chain connection uses a WebSocket subscription for real-time events and an HTTP fallback for historical queries. Each chain is a ChainDataSource Cell (Substrate-protocol specialization).

### 5.2 ChainDataSource Trait

```rust
/// Abstraction over a single chain's data source.
/// Implemented as a Cell conforming to the Substrate protocol.
#[async_trait]
pub trait ChainDataSource: Send + Sync {
    /// Chain identifier.
    fn chain_id(&self) -> u64;

    /// Human-readable chain name.
    fn chain_name(&self) -> &str;

    /// Current block number.
    async fn current_block(&self) -> Result<u64>;

    /// Subscribe to new block headers.
    async fn subscribe_blocks(&self) -> Result<BlockStream>;

    /// Read a lending rate from a specific protocol on this chain.
    async fn get_lending_rate(
        &self,
        protocol: &str,
        market: &str,
    ) -> Result<LendingRateReading>;

    /// Read TVL for a specific market.
    async fn get_tvl(&self, protocol: &str, market: &str) -> Result<f64>;

    /// Health check: is this chain connection alive and synced?
    async fn health(&self) -> ChainHealth;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LendingRateReading {
    pub protocol: String,
    pub market: String,
    pub chain_id: u64,
    /// Annualized supply rate as a decimal.
    pub supply_rate: f64,
    /// Annualized borrow rate as a decimal.
    pub borrow_rate: f64,
    pub tvl_usd: f64,
    pub block_number: u64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainHealth {
    /// Connected and synced within 3 blocks.
    Live,
    /// Connected but lagging more than 3 blocks.
    Stale { blocks_behind: u64 },
    /// Connection lost.
    Offline { since: u64 },
}
```

### 5.3 ChainDataAggregator

The aggregator is a Graph that composes multiple ChainDataSource Cells into a unified cross-chain view.

```rust
/// Aggregates data across multiple chains.
/// Implemented as a Graph composing ChainDataSource Cells.
pub struct ChainDataAggregator {
    sources: Vec<Box<dyn ChainDataSource>>,
}

impl ChainDataAggregator {
    /// Collect ISFR components from all connected chains.
    pub async fn collect_isfr_components(&self) -> Result<Vec<IsfrComponent>> { ... }

    /// Health status of all chain connections.
    pub fn chain_health(&self) -> Vec<(u64, ChainHealth)> { ... }
}
```

### 5.4 Bridge Interface (Deferred)

Cross-chain bridging is deferred to Phase 2. The trait is defined for future implementation.

```rust
/// Cross-chain bridge abstraction. Phase 2 -- not implemented.
#[async_trait]
pub trait Bridge: Send + Sync {
    /// Initiate a bridge transfer from source to destination chain.
    async fn initiate_transfer(
        &self,
        source_chain: u64,
        dest_chain: u64,
        token: Address,
        amount: U256,
        recipient: Address,
    ) -> Result<BridgeTransferId>;

    /// Query the status of a pending bridge transfer.
    async fn transfer_status(
        &self,
        id: &BridgeTransferId,
    ) -> Result<BridgeTransferStatus>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeTransferStatus {
    Pending,
    SourceConfirmed { tx_hash: [u8; 32] },
    DestinationConfirmed { tx_hash: [u8; 32] },
    Failed { reason: String },
}
```

---

## 6. Agent DeFi Capabilities

Agents interact with DeFi protocols through a layered stack of Cell specializations: VenueAdapter for protocol interaction, DeFiRiskEngine for safety enforcement, TradingReflect for P&L attribution, and the daimon affect engine for position sizing.

### 6.1 VenueAdapter Trait

The venue adapter normalizes interactions across DEXs, lending protocols, and other DeFi venues. One Cell implementation per protocol. Agents call the trait; they never call protocol-specific ABIs directly.

```rust
/// Normalized interface to a DeFi protocol.
/// Implemented as a Cell conforming to the Act protocol.
#[async_trait]
pub trait VenueAdapter: Send + Sync {
    /// Human-readable venue name (e.g., "Uniswap V3", "Aave V3").
    fn name(&self) -> &str;

    /// Chain this venue operates on.
    fn chain_id(&self) -> u64;

    /// Execute a token swap.
    async fn swap(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
        min_amount_out: U256,
        deadline: u64,
    ) -> Result<SwapReceipt>;

    /// Add liquidity to a pool.
    async fn add_liquidity(
        &self,
        pool: Address,
        amounts: &[U256],
        min_lp_tokens: U256,
    ) -> Result<LiquidityReceipt>;

    /// Remove liquidity from a pool.
    async fn remove_liquidity(
        &self,
        pool: Address,
        lp_tokens: U256,
        min_amounts: &[U256],
    ) -> Result<LiquidityReceipt>;

    /// Get current pool state (reserves, fee tier, tick).
    async fn get_pool_state(&self, pool: Address) -> Result<PoolState>;

    /// Get a price quote without executing.
    async fn get_quote(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
    ) -> Result<Quote>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapReceipt {
    pub tx_hash: [u8; 32],
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: U256,
    pub amount_out: U256,
    pub effective_price: f64,
    pub slippage_bps: f64,
    pub gas_used: u64,
    pub gas_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub amount_out: U256,
    pub effective_price: f64,
    pub price_impact_bps: f64,
    pub route: Vec<Address>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolState {
    pub address: Address,
    pub reserves: Vec<U256>,
    pub fee_bps: u64,
    pub tvl_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityReceipt {
    pub tx_hash: [u8; 32],
    pub pool: Address,
    pub lp_tokens: U256,
    pub amounts: Vec<U256>,
}
```

### 6.2 DeFiRiskEngine

The risk engine is a Verify-protocol Cell that enforces portfolio-level constraints before any trade executes. Every DeFi tool call passes through this engine. If a trade would violate any constraint, the engine rejects it before submission.

```rust
/// Portfolio-level risk enforcement for DeFi operations.
/// Implemented as a Cell conforming to the Verify protocol.
pub struct DeFiRiskEngine {
    /// Maximum position size as a fraction of portfolio (0.0 to 1.0).
    pub max_position_fraction: f64,
    /// Maximum number of concurrent open positions.
    pub max_open_positions: usize,
    /// Maximum portfolio drawdown before halting all trading (0.0 to 1.0).
    pub max_drawdown: f64,
    /// Current portfolio drawdown tracking.
    pub current_drawdown: f64,
    /// Peak portfolio value for drawdown computation.
    pub peak_value_usd: f64,
    /// Current portfolio value.
    pub current_value_usd: f64,
    /// MEV protection: maximum slippage tolerance in basis points.
    pub max_slippage_bps: f64,
    /// MEV protection: use private mempool when available.
    pub use_private_mempool: bool,
    /// Daily loss limit in USD. Trading halts if breached.
    pub daily_loss_limit_usd: f64,
    /// Running daily realized loss.
    pub daily_realized_loss_usd: f64,
}

/// Risk check result. Produced as a verdict Signal by the DeFiRiskEngine Cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskVerdict {
    /// Trade is within all limits.
    Approved,
    /// Trade rejected with specific reason.
    Rejected { reason: String },
    /// Trade approved but with reduced size.
    SizeReduced { original: f64, adjusted: f64, reason: String },
}

impl DeFiRiskEngine {
    /// Check whether a proposed trade passes all risk constraints.
    pub fn check_trade(
        &self,
        proposed_size_usd: f64,
        open_position_count: usize,
        estimated_slippage_bps: f64,
    ) -> RiskVerdict { ... }

    /// Update drawdown tracking after a trade outcome.
    pub fn record_pnl(&mut self, realized_pnl_usd: f64) { ... }

    /// Reset daily loss counter. Called at the start of each UTC day.
    pub fn reset_daily_loss(&mut self) { ... }

    /// Whether trading is currently halted due to drawdown or daily loss.
    pub fn is_halted(&self) -> bool { ... }
}
```

### 6.3 TradingReflect: P&L Attribution

FIFO (first-in, first-out) matching of position entries to exits. When a position closes, TradingReflect traces the P&L back to the decision that opened it: which agent, which model, which indicators, what regime. This continuous reward Signal feeds the cascade router, playbook outcomes, and episode logger.

TradingReflect is a Score-protocol Cell: it takes position-close Signals as input and produces attributed P&L Signals as output.

```rust
/// FIFO matching engine that pairs position entries with exits.
/// Implemented as a Cell conforming to the Score protocol.
pub struct FifoMatcher {
    open_entries: Vec<PositionEntry>,
}

impl FifoMatcher {
    /// Record a new position entry with full decision context.
    pub fn record_entry(&mut self, entry: PositionEntry) { ... }

    /// Match an exit against the oldest open entry for the same asset+side.
    /// Returns the closed position with realized P&L and full attribution chain.
    pub fn record_exit(
        &mut self,
        asset: &str,
        side: TradeSide,
        exit_price: f64,
        exit_size: f64,
        exit_gas_usd: f64,
        slippage_bps: f64,
    ) -> Option<ClosedPosition> { ... }

    pub fn open_positions(&self) -> &[PositionEntry] { ... }
}

/// Closed position with realized P&L and full attribution chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedPosition {
    pub entry: PositionEntry,
    pub exit_price: f64,
    /// Net realized P&L in USD (after gas and slippage).
    pub realized_pnl: f64,
    pub gas_cost_total_usd: f64,
    pub slippage_bps: f64,
    pub hold_duration_secs: f64,
    pub closed_at: DateTime<Utc>,
}
```

### 6.4 Affect-Modulated Sizing

Position sizing passes through the daimon affect engine (a Route-protocol Cell) before execution. The core mechanism is prospect theory (Tversky & Kahneman 1992): losses are weighted 2.25x relative to gains. An agent that just suffered a loss will naturally reduce position size because the daimon's arousal state shifts the risk threshold.

```rust
/// Prospect theory value function for P&L-to-affect mapping.
///
/// Gains: v(x) = x^0.88
/// Losses: v(x) = -2.25 * |x|^0.88
///
/// The 2.25x loss aversion coefficient (lambda) means a $100 loss feels
/// equivalent to a $225 gain in terms of affect impact.
pub fn prospect_value(pnl: f64) -> f64 {
    const LOSS_AVERSION: f64 = 2.25;  // lambda
    const CURVATURE: f64 = 0.88;

    if pnl >= 0.0 {
        pnl.powf(CURVATURE)
    } else {
        -LOSS_AVERSION * pnl.abs().powf(CURVATURE)
    }
}

/// Compute a position size adjustment factor from the current daimon state.
///
/// Returns a multiplier in [0.25, 1.5]:
/// - Low arousal, positive valence: full size (1.0-1.5x)
/// - High arousal, negative valence: reduced size (0.25-0.5x)
/// - Neutral: no adjustment (1.0x)
pub fn affect_size_multiplier(
    pleasure: f64,    // PAD pleasure dimension [-1, 1]
    arousal: f64,     // PAD arousal dimension [-1, 1]
    dominance: f64,   // PAD dominance dimension [-1, 1]
) -> f64 { ... }
```

The affect sizing pipeline:

```
Realized P&L (ClosedPosition.realized_pnl)
    |
    v
prospect_value(pnl) -> affect delta
    |
    v
DaimonState PAD vector update (pleasure, arousal, dominance)
    |
    v
affect_size_multiplier(p, a, d) -> multiplier in [0.25, 1.5]
    |
    v
proposed_size * multiplier -> actual_size
    |
    v
DeFiRiskEngine.check_trade(actual_size, ...) -> RiskVerdict
```

---

## 7. Event Types

All DeFi events flow through the Bus as Pulses ([doc-01](01-SIGNAL.md)) and are indexed by the event indexer.

### 7.1 Event Payloads

```json
{
    "type": "isfr.updated",
    "payload": {
        "rate": 0.0412,
        "previous_rate": 0.0398,
        "change_bps": 14,
        "component_count": 6,
        "block_number": 19847231,
        "timestamp": 1714003200
    }
}
```

```json
{
    "type": "position.opened",
    "payload": {
        "position_id": 4827,
        "owner": "0xabc...def",
        "agent_id": "trade-executor-1",
        "is_long": true,
        "size_usd": 10000.00,
        "entry_rate": 0.0412,
        "collateral_usd": 1200.00,
        "block_number": 19847235
    }
}
```

```json
{
    "type": "position.closed",
    "payload": {
        "position_id": 4827,
        "realized_pnl_usd": 47.23,
        "gas_cost_usd": 1.02,
        "hold_duration_secs": 14400,
        "exit_rate": 0.0426,
        "block_number": 19848431
    }
}
```

```json
{
    "type": "clearing.started",
    "payload": {
        "round_id": 892,
        "total_long_size_usd": 4500000.00,
        "total_short_size_usd": 3200000.00,
        "clearing_rate": 0.0412,
        "block_number": 19847250
    }
}
```

```json
{
    "type": "clearing.settled",
    "payload": {
        "round_id": 892,
        "funding_paid_usd": 12340.00,
        "positions_settled": 247,
        "settlement_block": 19847252,
        "duration_ms": 1200
    }
}
```

```json
{
    "type": "risk.drawdown_warning",
    "payload": {
        "agent_id": "trade-executor-1",
        "current_drawdown": 0.08,
        "max_drawdown": 0.10,
        "peak_value_usd": 100000.00,
        "current_value_usd": 92000.00
    }
}
```

```json
{
    "type": "risk.mev_detected",
    "payload": {
        "tx_hash": "0x123...abc",
        "type": "sandwich",
        "estimated_loss_usd": 12.50,
        "attacker": "0xdead...beef",
        "block_number": 19847240
    }
}
```

### 7.2 Full Event Type Table

| Event | Emitted By | Consumed By |
|---|---|---|
| `isfr.updated` | ISFROracle contract / ChainDataAggregator | Dashboard, yield perp mark-to-market, agent context |
| `isfr.deviation_triggered` | ISFROracle contract | Dashboard (alert), clearing house |
| `position.opened` | ClearingHouse contract | Dashboard, risk engine, TradingReflect |
| `position.closed` | ClearingHouse contract | Dashboard, TradingReflect, learning pipeline |
| `position.liquidated` | ClearingHouse contract | Dashboard (alert), risk engine |
| `clearing.started` | ClearingHouse contract | Dashboard, agent trading logic |
| `clearing.settled` | ClearingHouse contract | Dashboard, TradingReflect |
| `risk.drawdown_warning` | DeFiRiskEngine | Dashboard (alert), agent trading logic |
| `risk.mev_detected` | MEV detection module | Dashboard (alert), risk engine |
| `risk.daily_limit_hit` | DeFiRiskEngine | Dashboard (alert), trading halt |
| `chain.health_changed` | ChainDataAggregator | Dashboard, ISFR oracle |
| `collateral.added` | ClearingHouse contract | Dashboard, risk engine |
| `collateral.removed` | ClearingHouse contract | Dashboard, risk engine |

---

## 8. API Surface

Routes served by `roko-serve` on the control plane. These feed the Treasury section of the dashboard and provide programmatic access for external integrations.

### 8.1 ISFR Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/defi/isfr` | Current ISFR rate with all components |
| `GET` | `/api/defi/isfr/history?window={24h,7d,30d}` | Historical ISFR snapshots |
| `GET` | `/api/defi/isfr/components` | Current component breakdown with TVL weights |
| `GET` | `/api/defi/isfr/curves` | Derived forward rate curve and term structure |

**Response: `GET /api/defi/isfr`**

```json
{
    "rate": 0.0412,
    "rate_bps": 412,
    "change_24h_bps": 14,
    "change_7d_bps": -8,
    "last_update": "2026-04-24T08:00:00Z",
    "last_update_block": 19847231,
    "component_count": 6,
    "components": [
        {
            "source_id": "aave-usdc-eth",
            "rate": 0.0435,
            "tvl_usd": 2400000000,
            "weight": 0.32,
            "chain_id": 1,
            "chain_name": "Ethereum"
        }
    ]
}
```

### 8.2 Position Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/defi/positions` | All positions for the authenticated agent/user |
| `GET` | `/api/defi/positions/{id}` | Single position detail |
| `POST` | `/api/defi/positions` | Open a new position (proxies to ClearingHouse) |
| `DELETE` | `/api/defi/positions/{id}` | Close a position |
| `POST` | `/api/defi/positions/{id}/collateral` | Add or remove collateral |

### 8.3 Clearing Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/defi/clearing/next` | Next clearing round info and countdown |
| `GET` | `/api/defi/clearing/history?limit=20` | Recent clearing rounds |
| `GET` | `/api/defi/clearing/{round_id}` | Specific clearing round detail |

### 8.4 Risk Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/defi/risk` | Current risk state (drawdown, limits, halt status) |
| `GET` | `/api/defi/risk/mev?window=24h` | Recent MEV events |
| `PUT` | `/api/defi/risk/config` | Update risk parameters |

### 8.5 Chain Health Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/defi/chains` | Connected chains with health status |
| `GET` | `/api/defi/chains/{chain_id}/rates` | Lending rates from a specific chain |

---

## 9. Heartbeat Integration

The heartbeat clock ([doc-13](13-TRIGGERS.md)) drives DeFi tick processing. Chain events feed the Store (CorticalState surface). The three tick frequencies map to DeFi activities as Hot Flows bound to the heartbeat clock:

| Frequency | Period | DeFi Activity | Hot Flow |
|---|---|---|---|
| **Gamma** | 1-5s | Price feed ingestion, MEV detection, liquidation monitoring | `defi-gamma-tick` |
| **Theta** | 5-60s | Strategy evaluation, position sizing, trade execution | `defi-theta-tick` |
| **Delta** | 120s | Portfolio rebalancing, ISFR update checks, risk report generation | `defi-delta-tick` |

---

## 10. Learning Integration

TradingReflect events feed into the learning pipeline, which distributes continuous P&L reward to:

- **Cascade router** ([doc-08](08-GATEWAY.md)): Updates arm weights based on model-specific trade outcomes. If model X consistently produces better Sharpe ratios, the router routes more trading tasks to model X.
- **Episode logger**: Records trade-level data in episode `extra` map, including entry/exit prices, hold duration, gas costs, and slippage.
- **Playbook store**: Updates per-playbook win/loss counters and P&L. Playbooks that produce negative P&L decay via demurrage.
- **Indicator accuracy**: Validates indicator predictions against realized outcomes. An indicator that predicted "rate increase" is scored against the actual ISFR movement.

---

## 11. Daimon Integration

The `prospect_value` function (section 6.4) maps realized P&L to affect updates. The resulting PAD vector shift modulates:

- **Position sizing** via `affect_size_multiplier()` -- a Route-protocol Cell that adjusts trade size based on emotional state.
- **Strategy selection** via daimon policy in the cascade router -- high arousal + negative valence biases toward conservative strategies.
- **Risk tolerance** via somatic marker intensity -- strong negative markers from past losses increase the effective maintenance margin.

---

## 12. Configuration

```toml
# roko.toml

[defi]
enabled = true

[defi.isfr]
update_cadence_hours = 8
deviation_trigger_bps = 50
min_components = 3

[defi.clearing]
interval_blocks = 150
interval_minutes = 30
initial_margin_pct = 10
maintenance_margin_pct = 5
liquidation_bonus_pct = 2

[defi.risk]
max_position_fraction = 0.10
max_open_positions = 5
max_drawdown = 0.10
max_slippage_bps = 50
use_private_mempool = true
daily_loss_limit_usd = 500

[defi.chains]
ethereum_rpc = "wss://eth-mainnet.g.alchemy.com/v2/..."
base_rpc = "wss://base-mainnet.g.alchemy.com/v2/..."
arbitrum_rpc = "wss://arb-mainnet.g.alchemy.com/v2/..."

[defi.affect]
loss_aversion = 2.25
curvature = 0.88
min_size_multiplier = 0.25
max_size_multiplier = 1.5
```

---

## 13. Deployment

### 13.1 Control Plane (Railway, Always-On)

The control plane hosts:
- **ChainDataAggregator** (multi-chain WebSocket connections) -- a Graph of ChainDataSource Cells
- **ISFR computation and caching** -- ISFROracle Cell producing snapshot Signals
- **Risk engine state** -- DeFiRiskEngine Cell with persistent drawdown tracking
- **Learning pipeline** -- TradingReflect store, indicator accuracy, regime tracker
- **DeFi API routes** -- served by `roko-serve` on :6677

### 13.2 Trading Agents (Fly Machines, Per-Agent)

Each trading agent runs on an isolated Fly Machine with:
- **Local heartbeat clock** (DeFi preset: gamma/theta/delta) -- drives Hot Flows
- **Local VenueAdapter instance** -- Act-protocol Cell for protocol interaction
- **Portfolio state** synced from control plane
- **P&L events** reported back via `POST /api/agents/{id}/events`

This isolation means a misbehaving trading agent cannot affect other agents or the control plane.

---

## 14. Acceptance Criteria

| Criterion | Verification |
|---|---|
| ISFR dual-median aggregation produces correct rate from 6 components | Unit test: 6 known components -> expected ISFR, verify median-of-medians |
| ISFR deviation trigger fires when rate moves > 50bps | Unit test: inject rate shift > 50bps, verify immediate update |
| ISFR minimum component count enforced (3) | Unit test: submit 2 components, verify aggregation reverts |
| IISFROracle contract: submitComponents, aggregate, getCurrentRate | Integration test: deploy, submit, aggregate, read back |
| Yield perpetual position lifecycle: open -> mark -> settle -> close | Integration test: full position lifecycle with P&L verification |
| Initial margin (10%) and maintenance margin (5%) enforced | Unit test: open position with insufficient collateral -> rejected |
| Liquidation triggers at maintenance margin, 2% bonus to liquidator | Unit test: position below 5% margin -> liquidatable, bonus computed |
| Cooperative clearing executes VCG settlement every 150 blocks / 30 min | Integration test: accumulate positions, trigger settle, verify funding flows |
| IClearingHouse contract: openPosition, closePosition, settle, liquidate | Integration test: deploy, full lifecycle on-chain |
| VenueAdapter: swap, addLiquidity, removeLiquidity, getPoolState, getQuote | Unit test per method per venue implementation |
| DeFiRiskEngine: max_position_fraction, max_drawdown, daily_loss_limit | Unit test: trades exceeding each limit -> RiskVerdict::Rejected |
| DeFiRiskEngine: SizeReduced verdict when trade is partially acceptable | Unit test: large trade -> SizeReduced with adjusted amount |
| DeFiRiskEngine: is_halted() returns true after drawdown breach | Unit test: record losses past max_drawdown, verify halted |
| TradingReflect FIFO matching: entries paired with exits in order | Unit test: 3 entries, 3 exits, verify FIFO pairing and P&L |
| ClosedPosition includes full attribution (agent, model, indicators) | Unit test: PositionEntry with context -> ClosedPosition preserves it |
| prospect_value: gains use x^0.88, losses use -2.25*|x|^0.88 | Unit test: prospect_value(100) = 100^0.88, prospect_value(-100) = -2.25*100^0.88 |
| affect_size_multiplier: output in [0.25, 1.5] | Unit test: sweep PAD space, verify bounds |
| affect_size_multiplier: high arousal + negative valence -> reduced size | Unit test: (pleasure=-0.8, arousal=0.9, dominance=0.0) -> multiplier < 0.5 |
| ChainDataSource: health returns Live/Stale/Offline correctly | Unit test per health state |
| ChainDataAggregator: collects components from all chains | Integration test: 3 mock chains, verify all components collected |
| Multi-chain: Ethereum + Base + Arbitrum data aggregated | Integration test with real RPC endpoints (testnet) |
| Heartbeat gamma/theta/delta ticks drive DeFi Hot Flows | Integration test: bind flows to clock, verify execution at correct frequency |
| Learning: TradingReflect feeds cascade router arm weights | Integration test: close position, verify router weight update |
| Daimon: P&L -> prospect_value -> PAD update -> size adjustment | Integration test: loss event -> reduced position size on next trade |
| DeFi events emitted as Pulses on Bus | Integration test: subscribe to defi topics, verify Pulses received |
| API: all endpoints return correct data | Integration test per endpoint |
| Deployment: control plane on Railway, agents on Fly Machines | Deployment test: verify isolation, verify P&L event flow |
| Configuration: all roko.toml [defi] fields parsed and applied | Unit test: parse config, verify all fields |
