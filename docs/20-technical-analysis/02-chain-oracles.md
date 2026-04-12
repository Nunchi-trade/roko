# Chain Oracles — On-Chain Technical Analysis Primitives

> The chain domain is where TA originated. Chain oracles implement the universal Oracle trait with blockchain-specific state variables, verification mechanisms, and adversarial threat models.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [01-oracle-trait](./01-oracle-trait.md) for the Oracle trait, [00-vision-ta-generalized](./00-vision-ta-generalized.md) for generalization rationale
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §4, `bardo-backup/prd/23-ta/00-witness-as-technical-analyst.md`, `bardo-backup/prd/23-ta/07-defi-native-technical-analysis.md`

---

## ChainOracle — Implementation overview

The `ChainOracle` is the first and most mature Oracle implementation. It wraps traditional financial TA primitives (moving averages, RSI, Bollinger bands) alongside DeFi-native indicators (concentrated liquidity shape analysis, funding rates, yield term structures) into the universal Oracle trait interface:

```rust
pub struct ChainOracle {
    /// Connection to chain data (via roko-chain ChainClient).
    client: Arc<dyn ChainClient>,

    /// Historical price/volume/liquidity data cache.
    market_data: Arc<MarketDataCache>,

    /// DeFi-native indicator engine.
    defi_indicators: Arc<DeFiIndicatorEngine>,

    /// Prediction persistence and tracking.
    prediction_store: Arc<PredictionStore>,

    /// Bias correction from collective calibration.
    corrector: Arc<ResidualCorrector>,

    /// Per-(model, category) accuracy tracking.
    calibration: Arc<CalibrationTracker>,
}

#[async_trait]
impl Oracle for ChainOracle {
    async fn predict(
        &self,
        query: &OracleQuery,
        ctx: &Context,
    ) -> Result<Prediction> {
        let chain_payload = query.payload.as_chain()?;

        match chain_payload.metric {
            ChainMetric::Price => self.predict_price(chain_payload, ctx).await,
            ChainMetric::Gas => self.predict_gas(chain_payload, ctx).await,
            ChainMetric::Volatility => self.predict_volatility(chain_payload, ctx).await,
            ChainMetric::LiquidityDepth => self.predict_liquidity(chain_payload, ctx).await,
            ChainMetric::MevOpportunity => self.predict_mev(chain_payload, ctx).await,
            ChainMetric::ProtocolHealth => self.predict_protocol_health(chain_payload, ctx).await,
            ChainMetric::FundingRate => self.predict_funding(chain_payload, ctx).await,
            ChainMetric::YieldSpread => self.predict_yield(chain_payload, ctx).await,
        }
    }

    async fn evaluate(
        &self,
        prediction: &Prediction,
        outcome: &Engram,
    ) -> Result<PredictionAccuracy> {
        // Chain outcomes are deterministic — the blockchain state IS the ground truth.
        // Extract actual value from the outcome Engram (block data, DEX state, etc.)
        let actual = self.extract_chain_outcome(outcome)?;
        let accuracy = self.compute_accuracy(prediction, &actual);

        // Feed back to ResidualCorrector
        self.corrector.update(
            &prediction.provenance.model_id,
            &accuracy.category,
            accuracy.residual,
        );

        Ok(accuracy)
    }
}
```

### Verification mechanism: blockchain finality

Chain oracles have the strongest verification mechanism of any domain — the blockchain itself provides deterministic, tamper-proof ground truth. When a prediction about ETH price is made, the actual price at the predicted block height is an indisputable fact. This makes chain oracles ideal for calibrating the entire prediction system, since the feedback signal has zero noise.

---

## Traditional TA primitives

These are the financial TA indicators adapted for on-chain data. Each operates as a sub-predictor within the `ChainOracle`:

### Price prediction

```rust
/// Moving average family: SMA, EMA, WMA, DEMA, TEMA.
pub struct MovingAveragePredictor {
    /// Window sizes (e.g., [7, 25, 99] for short/medium/long term).
    windows: Vec<usize>,
    /// Type of moving average.
    ma_type: MovingAverageType,
}

/// Bollinger Bands: mean ± k*σ for dynamic support/resistance.
/// Standard: 20-period SMA with k=2 (captures 95% of price action).
pub struct BollingerBandPredictor {
    period: usize,
    num_std_dev: f64,
}

/// Relative Strength Index (Wilder, 1978).
/// RSI > 70 → overbought → predict mean reversion.
/// RSI < 30 → oversold → predict recovery.
/// In crypto, thresholds are often shifted (80/20) due to stronger trends.
pub struct RsiPredictor {
    period: usize,
    overbought_threshold: f64,
    oversold_threshold: f64,
}

/// MACD (Moving Average Convergence/Divergence).
/// Signal line crossovers predict trend changes.
/// Histogram divergence from price predicts momentum shifts.
pub struct MacdPredictor {
    fast_period: usize,   // typically 12
    slow_period: usize,   // typically 26
    signal_period: usize, // typically 9
}
```

### Volatility estimation

```rust
/// Realized volatility from historical price data.
/// Uses Garman-Klass estimator (more efficient than close-to-close)
/// when OHLC data is available.
pub struct VolatilityPredictor {
    /// Lookback window for volatility estimation.
    window: usize,
    /// Estimator type.
    estimator: VolatilityEstimator,
}

pub enum VolatilityEstimator {
    /// Close-to-close: σ = std(ln(P_t/P_{t-1}))
    CloseToClose,
    /// Garman-Klass (1980): uses OHLC for 5-8x efficiency gain.
    GarmanKlass,
    /// Parkinson (1980): uses high-low range.
    Parkinson,
    /// Yang-Zhang (2000): combines overnight and trading hour volatility.
    YangZhang,
}
```

### Gas price forecasting

```rust
/// Gas prediction using block-level fee data (EIP-1559 base fee dynamics).
///
/// The base fee follows a deterministic formula:
///   base_fee_next = base_fee * (1 + 0.125 * (gas_used - gas_target) / gas_target)
///
/// But the PRIORITY fee is market-driven and requires prediction.
/// We use exponential smoothing + day-of-week/hour-of-day seasonality.
pub struct GasPredictor {
    /// Base fee model (deterministic from block data).
    base_fee_model: BaseFeeModel,
    /// Priority fee model (statistical, requires prediction).
    priority_fee_model: PriorityFeeModel,
    /// Seasonal adjustment factors.
    seasonality: SeasonalityModel,
}
```

Gas prediction is a T0 probe — it runs at Gamma frequency with no LLM cost. The base fee is deterministic from EIP-1559 mechanics; only the priority fee requires statistical prediction.

---

## DeFi-native indicators

These indicators have no traditional finance equivalent. They arise from the unique mechanics of decentralized protocols:

### Concentrated liquidity shape analysis (Uniswap v3+)

```rust
/// Analyzes the distribution of liquidity across price ticks.
/// Concentrated liquidity creates a "liquidity landscape" that reveals
/// market maker expectations about future price ranges.
pub struct ConcentratedLiquidityAnalyzer {
    /// Pool address and chain.
    pool: PoolAddress,

    /// Indicators computed from tick-level data.
    indicators: ConcentratedLiquidityIndicators,
}

pub struct ConcentratedLiquidityIndicators {
    /// Tick asymmetry: ratio of liquidity above vs. below current price.
    /// High asymmetry → market expects directional move.
    /// Computed as: sum(liquidity_above) / sum(liquidity_below).
    pub tick_asymmetry: f64,

    /// Migration velocity: rate at which LPs are repositioning their ranges.
    /// High velocity → market makers expect imminent price action.
    /// Computed as: Δ(center_of_mass) / Δt.
    pub migration_velocity: f64,

    /// Density gaps: contiguous tick ranges with zero liquidity.
    /// Gaps indicate "air pockets" where price can move rapidly.
    /// Each gap is a (lower_tick, upper_tick) range.
    pub density_gaps: Vec<(i32, i32)>,

    /// Herfindahl-Hirschman Index of liquidity concentration.
    /// Low HHI → diffuse liquidity (high resilience).
    /// High HHI → concentrated liquidity (fragile, LP-dependent).
    pub hhi: f64,

    /// JIT (Just-In-Time) liquidity fraction: percentage of liquidity
    /// added and removed within the same block.
    /// High JIT → sophisticated MEV activity, higher execution risk.
    pub jit_fraction: f64,
}
```

These indicators are unique to DeFi and have no TradFi equivalent. They provide structural information about execution costs that traditional order book analysis cannot capture.

### Lending market indicators

```rust
/// Indicators derived from lending protocol state (Aave, Compound, etc.).
pub struct LendingIndicators {
    /// Utilization rate: borrowed / total_supplied.
    /// Above optimal (typically 80%) → interest rates spike nonlinearly.
    pub utilization_rate: f64,

    /// Liquidation proximity: distribution of borrower health factors.
    /// Concentration near 1.0 → cascade risk.
    pub liquidation_proximity: LiquidationDistribution,

    /// Supply/borrow rate spread: lender yield vs. borrower cost.
    /// Narrowing spread → protocol stress.
    pub rate_spread: f64,

    /// Flash loan volume trend: elevated flash loans often precede
    /// governance attacks or liquidation cascades.
    pub flash_loan_trend: f64,
}
```

### Perpetual funding rates

```rust
/// Funding rate indicators for perpetual futures (dYdX, GMX, etc.).
pub struct FundingRateIndicators {
    /// Current funding rate (annualized).
    /// Positive → longs pay shorts → market is long-biased.
    /// Negative → shorts pay longs → market is short-biased.
    pub current_rate: f64,

    /// Funding rate vs. 30-day moving average.
    /// Extreme deviation → mean reversion likely.
    pub deviation_from_mean: f64,

    /// Open interest trend: rising OI + positive funding → leveraged long squeeze risk.
    pub open_interest_trend: f64,

    /// Basis: spot price - perpetual price.
    /// Persistent negative basis → market structure stress.
    pub basis: f64,
}
```

### Yield term structure

```rust
/// Yield curves across DeFi lending protocols.
/// An inverted yield curve (short rates > long rates) signals stress,
/// analogous to inverted yield curves in TradFi bond markets.
pub struct YieldTermStructure {
    /// Rates at standard maturities.
    pub rates: BTreeMap<Duration, f64>,

    /// Slope: long_rate - short_rate.
    /// Positive slope → normal (compensation for duration risk).
    /// Negative slope → inverted (stress signal).
    pub slope: f64,

    /// Curvature: 2 * medium_rate - short_rate - long_rate.
    /// High curvature → convexity opportunity.
    pub curvature: f64,

    /// Rate of slope change: Δslope / Δt.
    /// Rapid flattening → potential regime change.
    pub slope_velocity: f64,
}
```

### On-chain options indicators

```rust
/// Indicators from on-chain options protocols (Lyra, Hegic, etc.).
pub struct OnChainOptionsIndicators {
    /// Implied volatility surface: IV across strikes and expirations.
    pub iv_surface: VolatilitySurface,

    /// Put/call ratio: elevated put buying → hedging demand → bearish signal.
    pub put_call_ratio: f64,

    /// Skew: difference between OTM put IV and OTM call IV.
    /// High skew → market pricing in downside risk.
    pub skew: f64,

    /// Term structure of IV: short-dated vs. long-dated implied vol.
    /// Inverted term structure → imminent event expected.
    pub iv_term_structure_slope: f64,
}
```

---

## MEV opportunity detection

Maximal Extractable Value (MEV) is an adversarial dynamic unique to blockchains. The chain oracle detects MEV exposure as a risk factor:

```rust
/// MEV analysis for execution risk assessment.
pub struct MevAnalyzer {
    /// Sandwich attack risk: probability of being sandwiched on a given pool.
    /// Estimated from historical mempool + block builder data.
    pub sandwich_risk: f64,

    /// Backrun opportunity: value available from transaction ordering.
    pub backrun_value: f64,

    /// Block builder concentration: if one builder dominates,
    /// MEV extraction is more predictable.
    pub builder_hhi: f64,

    /// Private transaction fraction: percentage of transactions
    /// submitted through private mempools (Flashbots, etc.).
    /// High fraction → less public mempool data for MEV prediction.
    pub private_tx_fraction: f64,
}
```

MEV detection is the chain oracle's adversarial threat model — analogous to the coding oracle's supply chain attack detection or the research oracle's p-hacking detection. The Oracle trait's generalization maps these domain-specific adversarial dynamics to a common pattern: "detect when the environment is actively working against you."

---

## The 8 T0 chain probes

At Gamma frequency (~5-15s), 8 chain-specific probes run with zero LLM cost (FrugalGPT-inspired; Chen et al., 2023, arXiv:2305.05176):

```rust
/// The 8 chain-domain T0 probes.
/// Each is a pure function: fn(state) -> f32.
/// Combined via weighted sum into a prediction error scalar.
pub fn chain_probes() -> Vec<Box<dyn Probe>> {
    vec![
        // 1. Price delta — has the price moved more than expected?
        Box::new(PriceDeltaProbe::new(threshold: 0.02)),

        // 2. TVL delta — has total value locked shifted significantly?
        Box::new(TvlDeltaProbe::new(threshold: 0.05)),

        // 3. Position health — is any position approaching liquidation?
        Box::new(PositionHealthProbe::new(min_health_factor: 1.2)),

        // 4. Gas spike — has base fee jumped more than 2x?
        Box::new(GasSpikeProbe::new(spike_multiplier: 2.0)),

        // 5. Credit balance — is KORAI balance below operating threshold?
        Box::new(CreditBalanceProbe::new(min_balance: 100.0)),

        // 6. RSI — is RSI in extreme territory (>80 or <20)?
        Box::new(RsiProbe::new(period: 14, overbought: 80.0, oversold: 20.0)),

        // 7. MACD — has MACD crossed the signal line?
        Box::new(MacdCrossProbe::new(fast: 12, slow: 26, signal: 9)),

        // 8. Circuit breaker — has any monitored exchange halted trading?
        Box::new(CircuitBreakerProbe::new()),
    ]
}
```

These probes cost microseconds each. When the weighted sum of all 16 probes (8 chain + 6 coding + 2 universal) produces an error scalar below 0.2, the agent suppresses cognitive activity — no LLM call, no cost. This happens ~80% of ticks, making the chain agent dramatically cheaper to run than naive polling-based agents.

---

## ChainOracle integration with the witness crate

The chain oracle integrates with `roko-chain`'s witness infrastructure (formerly the "Witness crate" in legacy documents):

```rust
/// The witness pipeline feeds data to the chain oracle.
///
/// Data flow:
///   ChainClient → raw block/tx data
///   → TriagePipeline → filtered, classified events
///   → MarketDataCache → indexed price/volume/liquidity history
///   → ChainOracle → predictions
///   → PredictionStore → tracked predictions
///   → ResidualCorrector → calibrated predictions
///
/// At each step, data is an Engram flowing through Synapse traits.
pub struct ChainWitnessPipeline {
    client: Arc<dyn ChainClient>,
    triage: TriagePipeline,
    cache: Arc<MarketDataCache>,
    oracle: Arc<ChainOracle>,
    store: Arc<PredictionStore>,
}
```

The triage pipeline uses MIDAS-R (Massively Irregular Data Aggregation using Streaming) for real-time anomaly detection and DDSketch (Masson et al., 2019) for percentile estimation on streaming data. Both are O(1) memory and sub-microsecond per update.

### CorticalState — The shared signal bus

The `CorticalState` (formerly `TaCorticalExtension` in legacy documents) is the shared state that all chain TA subsystems read and write:

```rust
/// Shared state for chain technical analysis.
///
/// All chain TA subsystems read from and write to this state.
/// Atomic operations ensure consistency at Gamma frequency.
/// This is the chain oracle's "working memory."
pub struct CorticalState {
    /// 8 atomic signal values, updated by probes.
    pub signals: [AtomicF64; 8],

    /// Current prediction error scalar (drives T0/T1/T2 routing).
    pub prediction_error: AtomicF64,

    /// Current behavioral state from Daimon.
    pub behavioral_state: AtomicU8,

    /// Timestamp of last update.
    pub last_update_ms: AtomicI64,
}
```

---

## Mirage-rs integration — Simulation-backed predictions

Chain oracles can validate predictions against `mirage-rs`, Roko's in-process EVM simulator (141 tests). Before executing a trade, the oracle can simulate the transaction:

```rust
/// Simulate a trade in mirage-rs to validate oracle predictions.
///
/// This creates a fork of the current chain state, executes the
/// proposed transaction, and compares the simulated outcome against
/// the oracle's prediction. If the simulation contradicts the
/// prediction, the confidence is reduced.
pub async fn validate_with_simulation(
    oracle: &ChainOracle,
    prediction: &Prediction,
    mirage: &MirageSimulator,
) -> ValidationResult {
    let simulated = mirage.simulate_trade(&prediction.trade_params).await?;
    let predicted_value = prediction.value.as_numeric()?;
    let simulated_value = simulated.execution_price;

    let divergence = (predicted_value - simulated_value).abs() / simulated_value;

    if divergence > 0.05 {
        ValidationResult::Divergent {
            predicted: predicted_value,
            simulated: simulated_value,
            divergence,
            recommendation: "Reduce confidence or re-predict with updated state",
        }
    } else {
        ValidationResult::Consistent { divergence }
    }
}
```

Mirage-rs enables the chain oracle's dream cycle to run counterfactual simulations — "what would have happened if gas was 5x higher?" or "what if liquidity was withdrawn from this pool?" — without risking real assets. This is the concrete implementation of Pearl's do-operator (Pearl, 2009, *Causality*) in the chain domain: simulate interventions on the causal model.

---

## On-chain prediction infrastructure

Predictions are published on-chain (Korai) for collective calibration:

```rust
/// On-chain prediction registry on Korai.
///
/// Each prediction is a `PredictionClaim` Engram posted to the
/// Intersubjective Fact Registry (ISFR). When resolved, the
/// resolution is also posted, and the agent's calibration score
/// is updated on-chain.
///
/// This enables:
/// 1. Collective calibration — all agents share prediction outcomes
/// 2. Reputation building — accurate predictors earn higher reputation
/// 3. Knowledge futures — predictions can be staked with KORAI tokens
pub struct OnChainPredictionStore {
    /// ISFR contract address on Korai.
    registry: Address,

    /// Agent's wallet for posting predictions.
    wallet: Arc<dyn ChainWallet>,

    /// Local cache of on-chain predictions.
    cache: Arc<PredictionStore>,
}
```

The on-chain prediction infrastructure connects to the Knowledge Futures Market (see `refactoring-prd/09-innovations.md` §XVI), where agents can stake KORAI tokens on their predictions. Accurate predictors earn rewards; inaccurate ones lose stake. This creates an economic incentive for oracle quality that compounds with the technical calibration loop.

---

## Academic foundations

- Wilder, J. W. (1978). *New Concepts in Technical Trading Systems*. — RSI, ADX, parabolic SAR.
- Bollinger, J. (2001). *Bollinger on Bollinger Bands*. — Dynamic support/resistance via standard deviation bands.
- Garman, M. B., & Klass, M. J. (1980). "On the Estimation of Security Price Volatilities from Historical Data." *Journal of Business*, 53(1), 67-78. — Efficient volatility estimation from OHLC data.
- Pearl, J. (2009). *Causality: Models, Reasoning, and Inference*. 2nd ed. Cambridge University Press. — Structural causal models, do-operator for counterfactual simulation.
- Masson, C., et al. (2019). "DDSketch: A Fast and Fully-Mergeable Quantile Sketch with Relative-Error Guarantees." *PVLDB*, 12(12), 2195-2205. — Streaming percentile estimation.
- Chen, L., et al. (2023). "FrugalGPT." arXiv:2305.05176. — Cascade architectures for T0/T1/T2 probe system.
- Vickrey, W. (1961). "Counterspeculation, Auctions, and Competitive Sealed Tenders." *Journal of Finance*, 16(1), 8-37. — VCG auction for context allocation.

---

## Cross-references

- See [01-oracle-trait.md](./01-oracle-trait.md) for the Oracle trait signature
- See [03-coding-oracles.md](./03-coding-oracles.md) for coding domain equivalents of these indicators
- See [07-spectral-liquidity-manifolds.md](./07-spectral-liquidity-manifolds.md) for Riemannian geometry over liquidity landscapes
- See [11-adversarial-signal-robustness.md](./11-adversarial-signal-robustness.md) for MEV defense via adversarial robustness
- See [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for the full prediction loop
