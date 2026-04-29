# Oracle as Score Cell

> Depth for [01-oracle-trait.md](../../docs/20-technical-analysis/01-oracle-trait.md), [02-chain-oracles.md](../../docs/20-technical-analysis/02-chain-oracles.md), [03-coding-oracles.md](../../docs/20-technical-analysis/03-coding-oracles.md), [04-research-oracles.md](../../docs/20-technical-analysis/04-research-oracles.md), [13-predictive-foraging-and-active-inference.md](../../docs/20-technical-analysis/13-predictive-foraging-and-active-inference.md). Reframes the Oracle prediction machinery as Score Cells that rate Signals by prediction accuracy, using the predict-publish-correct pattern as the universal learning loop.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, Bus, Store, demurrage, lineage), [02-CELL](../../unified/02-CELL.md) (Cell, Score protocol, predict-publish-correct, CalibrationTracker concept), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Loop pattern), [07-LEARNING](../../unified/07-LEARNING.md) (CascadeRouter, adaptive thresholds, EMA)

---

## 1. The Core Insight

An Oracle is not a separate subsystem. It is a **Score Cell** that rates Signals by their predicted accuracy against future reality. The two methods of the Oracle trait (`predict` and `evaluate`) map directly onto the Score protocol's `rate()` and the predict-publish-correct pattern:

| Oracle concept | Unified concept | What it does |
|---|---|---|
| `Oracle.predict(query, ctx)` | Score Cell publishes a Prediction Pulse | Claims a specific future outcome |
| `Oracle.evaluate(prediction, outcome)` | CalibrationPolicy React Cell joins by lineage | Computes error between claim and reality |
| `ResidualCorrector` | Functor (endofunctor on Score Cell output) | Adjusts future predictions by EMA bias |
| `CalibrationTracker` | Store Cell accumulating accuracy per-(model, category) | The Score protocol's calibration mechanism |
| `PredictionStore` | Store protocol specialization | Lifecycle management: register, track, resolve |

The key realization: every Cell in the system already does predict-publish-correct (this is a universal property from spec doc 02). The Oracle simply makes this explicit for domain-specific prediction targets -- price, build time, source reliability -- and adds the machinery (ResidualCorrector, CalibrationTracker, conformal prediction) that makes the predictions calibrated and trustworthy.

---

## 2. The Predict-Publish-Correct Cycle

The Oracle's lifecycle is a three-phase Loop that maps perfectly onto the universal pattern:

```
Phase 1: PREDICT
  Oracle (Score Cell) receives a query Signal.
  Produces a PredictionClaim Signal with Kind::Prediction.
  Publishes the claim as a Pulse on topic "oracle.predictions.{domain}".
  The claim is stored in PredictionStore (Store protocol) BEFORE action.

Phase 2: REALITY
  Time passes. The predicted event occurs (or doesn't).
  An external process produces an Outcome Signal -- a compiler result,
  a blockchain state, a test suite output, a replication study.
  The outcome is published as a Pulse on "oracle.outcomes.{domain}".

Phase 3: CORRECT
  A CalibrationPolicy (React Cell) watches both topics.
  When an outcome Pulse arrives, it joins it to the original prediction
  by lineage (same query_id).
  Computes PredictionAccuracy: scalar accuracy, signed residual,
  interval coverage hit.
  Feeds the residual into ResidualCorrector (Functor).
  Feeds the accuracy into CalibrationTracker (Store Cell).
  Feeds the reward into CascadeRouter (Route Cell).
```

This is not a bespoke prediction subsystem. It is the standard predict-publish-correct pattern (Friston 2006) with domain-specific Signals flowing through standard Bus and Store.

```rust
/// The Oracle expressed as a Score Cell.
///
/// Conforms to: Score protocol (rate Signals by predicted accuracy).
/// Participates in: predict-publish-correct Loop.
///
/// Input Signals: OracleQuery (what to predict, domain, horizon, min_confidence).
/// Output Signals: PredictionClaim (the prediction as a stored, falsifiable Signal).
///
/// Location: `crates/roko-learn/src/oracle/`
pub struct OracleScoreCell {
    /// Domain this oracle specializes in.
    domain: OracleDomain,

    /// The ResidualCorrector Functor applied to raw predictions.
    corrector: Arc<ResidualCorrector>,

    /// CalibrationTracker: per-(model, category) accuracy statistics.
    calibration: Arc<CalibrationTracker>,

    /// The underlying prediction logic (domain-specific).
    predictor: Box<dyn Predictor>,
}

/// The Score protocol implementation for Oracle.
impl ScoreProtocol for OracleScoreCell {
    /// Rate a Signal by producing a PredictionClaim.
    ///
    /// The "score" IS the prediction: a claim about what will happen,
    /// stored as a Signal with confidence, interval, and provenance.
    async fn rate(&self, signal: &Signal, ctx: &CellContext) -> Score {
        let query = OracleQuery::from_signal(signal)?;

        // 1. Raw prediction from domain-specific logic
        let mut prediction = self.predictor.predict(&query, ctx).await?;

        // 2. Apply ResidualCorrector Functor (bias elimination, ~50ns)
        self.corrector.correct(&mut prediction);

        // 3. Score axes from prediction confidence and calibration history
        let model_id = &prediction.provenance.model_id;
        let category = query.category();
        let cal_stats = self.calibration.get_stats(model_id, category);

        Score {
            confidence: prediction.confidence * cal_stats.recent_accuracy,
            novelty: prediction.epistemic_value(),
            utility: prediction.pragmatic_value(),
            reputation: cal_stats.reliability_score(),
            coherence: cal_stats.interval_coverage.current(),
        }
    }
}
```

---

## 3. Domain Oracle Specializations

Three domain oracles are Score Cell specializations with domain-specific Criteria (what they predict) and Evidence (how they verify). Each shares the same ResidualCorrector and CalibrationTracker infrastructure -- the differentiation is in what they observe and what counts as ground truth.

### Chain Oracle (Score Cell for on-chain prediction)

```rust
/// Chain Oracle: predicts price, gas, liquidity depth, MEV opportunity.
///
/// Criteria: ChainMetric (price, gas, tvl, liquidity_depth, mev_opportunity)
/// Evidence: blockchain state at resolution time (block data, DEX state)
/// Ground truth source: on-chain observation (RPC queries at horizon expiry)
///
/// Domain-specific indicators:
///   MA (moving average), RSI, Bollinger bands, MACD,
///   concentrated liquidity depth, funding rates, order flow imbalance.
///
/// Location: `crates/roko-learn/src/oracle/chain.rs`
pub struct ChainOracleCell {
    /// Moving average ensemble (7, 25, 99 periods).
    ma_ensemble: MovingAverageEnsemble,
    /// RSI calculator with oversold/overbought thresholds.
    rsi: RelativeStrengthIndex,
    /// Bollinger band width for volatility estimation.
    bollinger: BollingerBands,
    /// MACD for momentum detection.
    macd: Macd,
    /// Concentrated liquidity depth scanner.
    liquidity_scanner: LiquidityDepthScanner,
    /// Funding rate analyzer for perpetuals.
    funding_rates: FundingRateAnalyzer,
}
```

### Coding Oracle (Score Cell for software engineering prediction)

```rust
/// Coding Oracle: predicts build time, test failure rate, complexity drift,
/// dependency risk, performance regression.
///
/// Criteria: CodingMetric (build_time, test_pass_rate, complexity_delta,
///           dependency_risk, perf_regression)
/// Evidence: compiler output, test suite results, clippy diagnostics
/// Ground truth source: CI/CD pipeline output after code change
///
/// Location: `crates/roko-learn/src/oracle/coding.rs`
pub struct CodingOracleCell {
    /// Historical build times per scope (file, module, crate, workspace).
    build_history: PerScopeSeries,
    /// Test failure correlation: which files tend to break which tests.
    test_correlation: TestCorrelationMatrix,
    /// Complexity trend tracking (cyclomatic, cognitive, Halstead).
    complexity_tracker: ComplexityTracker,
    /// Dependency risk model (freshness, advisory CVEs, breaking changes).
    dep_risk: DependencyRiskModel,
}
```

### Research Oracle (Score Cell for information quality prediction)

```rust
/// Research Oracle: predicts source reliability, contradiction density,
/// replication probability, completeness.
///
/// Criteria: ResearchMetric (reliability, completeness, contradiction_risk,
///           replication_probability)
/// Evidence: independent verification, citation analysis, replication studies
/// Ground truth source: expert review, replication attempts, citation graphs
///
/// Location: `crates/roko-learn/src/oracle/research.rs`
pub struct ResearchOracleCell {
    /// Source reputation tracker (per-author, per-venue, per-institution).
    source_reputation: SourceReputationTracker,
    /// Contradiction detector (claims that conflict across sources).
    contradiction_scanner: ContradictionScanner,
    /// Citation graph analyzer (h-index, PageRank, recency weighting).
    citation_analyzer: CitationGraphAnalyzer,
}
```

---

## 4. ResidualCorrector as Functor

The ResidualCorrector is a **Functor** -- a cross-cut that enriches Score Cell output without changing the Graph topology. It is an endofunctor `F: Signal -> Signal` that adjusts predicted values by subtracting estimated systematic bias.

```rust
/// ResidualCorrector: Functor that eliminates systematic prediction bias.
///
/// Applied between raw prediction and published PredictionClaim.
/// Cost: ~50ns per correction (HashMap lookup + EMA subtraction).
///
/// Mathematical model:
///   raw_prediction = true_value + systematic_bias + noise
///   corrected = raw_prediction - EMA(historical_residuals)
///   => corrected ≈ true_value + noise  (bias eliminated)
///
/// The EMA smoothing factor (alpha = 0.1 by default) controls how
/// quickly the corrector adapts to changing bias. Lower alpha = more
/// stable but slower to adapt. Higher alpha = faster adaptation but
/// more noise.
///
/// Location: `crates/roko-learn/src/calibration/residual.rs`
pub struct ResidualCorrector {
    /// Per-(model, category) bias estimates.
    biases: DashMap<(String, String), ExponentialMovingAverage>,
    /// EMA smoothing factor.
    alpha: f64,
}

impl Functor for ResidualCorrector {
    type Input = Signal;  // Raw prediction Signal
    type Output = Signal; // Bias-corrected prediction Signal

    fn map(&self, signal: Signal) -> Signal {
        let prediction = signal.as_prediction();
        let model = &prediction.provenance.model_id;
        let category = prediction.category();
        let key = (model.clone(), category.clone());

        if let Some(bias_ema) = self.biases.get(&key) {
            let corrected_value = prediction.value.numeric() - bias_ema.current();
            signal.with_value(PredictedValue::Numeric(corrected_value))
        } else {
            signal // No correction data yet -- pass through unchanged
        }
    }
}
```

The Functor pattern means ResidualCorrector composes with any Score Cell without modifying the Cell itself. You can stack multiple Functors (bias correction, interval recalibration, conformal wrapping) on any Oracle without changing its implementation.

---

## 5. CalibrationTracker as Store Cell

The CalibrationTracker is a **Store Cell** that aggregates prediction accuracy per-(model, category) pair. It is the concrete implementation of the Score protocol's calibration mechanism:

```rust
/// CalibrationTracker: Store Cell that maintains prediction accuracy history.
///
/// For each (model, category) pair, tracks:
///   - Mean residual (bias direction and magnitude)
///   - Mean absolute error (accuracy regardless of direction)
///   - Interval coverage (are prediction intervals well-calibrated?)
///   - Accuracy trend (improving or degrading?)
///   - Brier score decomposition (REL, RES, UNC)
///
/// Provides the data that drives:
///   - ResidualCorrector bias estimates
///   - CascadeRouter model selection (accurate models get more traffic)
///   - Adaptive gate thresholds (tighter when predictions are reliable)
///   - VCG auction weights (confident predictions bid higher)
///
/// Location: `crates/roko-learn/src/calibration/tracker.rs`
pub struct CalibrationTracker {
    stats: DashMap<(String, String), CalibrationStats>,
}

pub struct CalibrationStats {
    /// Bias estimate (signed residual EMA).
    pub mean_residual: ExponentialMovingAverage,
    /// Accuracy estimate (absolute residual EMA).
    pub mean_absolute_error: ExponentialMovingAverage,
    /// Interval calibration (fraction of outcomes within intervals).
    pub interval_coverage: ExponentialMovingAverage,
    /// Resolved prediction count.
    pub count: u64,
    /// Accuracy trend (slope of recent accuracy window).
    pub trend: f64,
    /// Brier decomposition (Murphy 1973).
    pub brier: BrierDecomposition,
}

pub struct BrierDecomposition {
    pub reliability: f64,  // Lower = better calibrated
    pub resolution: f64,   // Higher = better discrimination
    pub uncertainty: f64,  // Base rate (irreducible)
}
```

### How CalibrationTracker serves the Route protocol

The CascadeRouter (a Route Cell) uses CalibrationTracker data to select models. When a new query arrives, the router checks each candidate model's accuracy for the query's category:

```rust
/// CascadeRouter consults CalibrationTracker for model selection.
///
/// The Route protocol's EFE formula:
///   EFE(model) = pragmatic_value(model, category)
///              + epistemic_value(model, category)
///              - cost(model)
///
/// where pragmatic_value IS the calibration accuracy:
///   pragmatic_value = calibration.get_accuracy(model, category)
///
/// This means accurate models naturally get more routing weight.
/// New models start with prior accuracy 0.5 and must earn traffic
/// through demonstrated calibration.
fn route_to_model(
    &self,
    query: &OracleQuery,
    candidates: &[ModelId],
    calibration: &CalibrationTracker,
) -> ModelId {
    candidates.iter()
        .max_by_key(|model| {
            let stats = calibration.get_stats(model, &query.category());
            let efe = stats.recent_accuracy        // pragmatic
                + stats.uncertainty_reduction()    // epistemic
                - self.cost_per_token(model);     // cost
            OrderedFloat(efe)
        })
        .unwrap()
        .clone()
}
```

---

## 6. Conformal Prediction as Verify Cell

Conformal prediction (Vovk et al. 2005) provides distribution-free coverage guarantees. In the unified model, it is a **Verify Cell** that wraps any Oracle's output and adds mathematical guarantees:

```rust
/// ConformalVerifyCell: Verify protocol Cell that provides coverage guarantees.
///
/// Property: P(actual in prediction_set) >= 1 - alpha
/// for ANY underlying distribution. Only requires exchangeability.
///
/// This is the Verify protocol's mathematical guarantee mechanism:
/// rather than checking "is the prediction correct?" (binary),
/// it checks "does the prediction set CONTAIN the actual value?"
/// (coverage guarantee).
///
/// The prediction set width adapts automatically: when the Oracle
/// is well-calibrated, sets are tight. When calibration degrades,
/// sets widen to maintain coverage.
///
/// Location: `crates/roko-learn/src/calibration/conformal.rs`
pub struct ConformalVerifyCell {
    /// Nonconformity scores from calibration set.
    calibration_scores: Vec<f64>,
    /// Target miscoverage rate (e.g., 0.10 for 90% coverage).
    alpha: f64,
    /// Maximum calibration set size (sliding window).
    max_calibration_size: usize,
}

impl VerifyProtocol for ConformalVerifyCell {
    async fn verify(&self, signal: &Signal, ctx: &CellContext) -> Verdict {
        let prediction = signal.as_prediction();
        let n = self.calibration_scores.len();

        // Quantile threshold: ceil((1 - alpha)(n + 1)) / n
        let quantile_idx = ((1.0 - self.alpha) * (n + 1) as f64).ceil() as usize;
        let threshold = self.calibration_scores
            .get(quantile_idx.min(n - 1))
            .copied()
            .unwrap_or(f64::MAX);

        // Prediction set: [point - threshold, point + threshold]
        let point = prediction.value.numeric();
        let interval = PredictionInterval {
            lower: point - threshold,
            upper: point + threshold,
            coverage: 1.0 - self.alpha,
        };

        Verdict {
            pass: true, // Conformal always produces a valid set
            reward: interval_tightness(&interval), // Tighter = better
            evidence: Evidence::ConformalSet { interval, n_calibration: n },
            message: format!(
                "Coverage {:.0}% guaranteed (n={}, threshold={:.4})",
                (1.0 - self.alpha) * 100.0, n, threshold
            ),
        }
    }
}
```

---

## 7. Oracle Composition with VCG

Oracle predictions participate in the Compose protocol's VCG auction. When assembling context for an LLM call, prediction context bids for attention budget based on Expected Free Energy:

```rust
/// Oracle predictions bid into VCG attention auction.
///
/// High-uncertainty predictions bid HIGHER because resolving them
/// has high epistemic value (Friston 2010). This is counter-intuitive
/// but correct: the system allocates more attention to predictions it
/// is uncertain about, because that is where information gain is highest.
///
/// Bid formula:
///   bid = pragmatic_value + epistemic_value - ambiguity
///       = confidence * utility + (1.0 - confidence) * novelty - cost
///
/// This implements the EFE decomposition from active inference:
///   G(pi) = E_Q[log Q(s) - log P(o|s)] + E_Q[log Q(s) - log P(s)]
///         = ambiguity (pragmatic) + complexity (epistemic)
pub fn oracle_bid_for_attention(
    prediction: &PredictionClaim,
    calibration: &CalibrationStats,
) -> AttentionBid {
    let pragmatic = prediction.confidence * prediction.utility_if_correct();
    let epistemic = (1.0 - prediction.confidence) * prediction.novelty_score();
    let cost = prediction.token_cost_estimate();

    AttentionBid {
        source: BidSource::Oracle(prediction.domain.clone()),
        value: pragmatic + epistemic - cost,
        content: prediction.as_context_section(),
        marginal_effect: calibration.section_effect_estimate(),
    }
}
```

---

## 8. The Complete Oracle Loop as a Graph

Bringing it all together, the Oracle system is a Loop pattern (Graph with feedback edge):

```toml
# oracle-loop.toml -- The Oracle as a Graph of Cells

[graph]
name = "oracle-prediction-loop"
pattern = "Loop"
frequency = "Gamma"  # 5-15 seconds

[[cells]]
name = "query-ingress"
protocol = "Trigger"
watches = ["oracle.queries.*"]

[[cells]]
name = "domain-oracle"
protocol = "Score"
# Specialization: ChainOracleCell | CodingOracleCell | ResearchOracleCell
config.domain = "coding"

[[cells]]
name = "residual-corrector"
protocol = "Functor"
# Adjusts raw prediction by EMA bias

[[cells]]
name = "conformal-wrapper"
protocol = "Verify"
config.alpha = 0.10
config.max_calibration_size = 500

[[cells]]
name = "prediction-store"
protocol = "Store"
# Persists PredictionClaim as Signal with Kind::Prediction

[[cells]]
name = "outcome-watcher"
protocol = "React"
watches = ["oracle.outcomes.*"]

[[cells]]
name = "calibration-policy"
protocol = "React"
# Joins prediction + outcome by lineage, computes accuracy

[[cells]]
name = "calibration-tracker"
protocol = "Store"
# Accumulates per-(model, category) accuracy stats

[[cells]]
name = "router-feedback"
protocol = "Route"
# Updates CascadeRouter bandit arms with accuracy signal

# Edges define the Loop
[[edges]]
from = "query-ingress"
to = "domain-oracle"

[[edges]]
from = "domain-oracle"
to = "residual-corrector"

[[edges]]
from = "residual-corrector"
to = "conformal-wrapper"

[[edges]]
from = "conformal-wrapper"
to = "prediction-store"

# Feedback path (the Loop):
[[edges]]
from = "outcome-watcher"
to = "calibration-policy"

[[edges]]
from = "calibration-policy"
to = "calibration-tracker"

[[edges]]
from = "calibration-tracker"
to = "residual-corrector"
label = "bias update"

[[edges]]
from = "calibration-tracker"
to = "router-feedback"
label = "accuracy signal"
```

---

## What This Enables

1. **Universal prediction infrastructure**: Any Cell can become a calibrated predictor by wrapping it in the Oracle Loop. The infrastructure (ResidualCorrector, CalibrationTracker, ConformalVerifyCell) is domain-agnostic.
2. **Automatic model routing**: CascadeRouter learns which models are best at which prediction tasks through the same bandit mechanism that routes LLM inference.
3. **Self-calibrating agents**: New agents import collective calibration data from Store, starting with pre-learned biases for every model-category pair the collective has encountered.
4. **Cost-free learning**: The ResidualCorrector's ~50ns per correction means 1,000 corrections per day costs effectively nothing. Learning is pure arithmetic, not LLM calls.
5. **Mathematical guarantees**: Conformal prediction provides distribution-free coverage guarantees on any Oracle output without distributional assumptions.

---

## Feedback Loops

| Loop | Participants | Signal | Timescale |
|---|---|---|---|
| **Bias correction** | ResidualCorrector <-> CalibrationPolicy | Signed residual | Per-prediction (~50ns update) |
| **Accuracy routing** | CalibrationTracker -> CascadeRouter | Recent accuracy | Per-prediction |
| **Gate threshold** | CalibrationTracker -> Adaptive gate | Mean absolute error | Per-prediction (EMA) |
| **VCG bidding** | CalibrationStats -> Compose auction | Section effect estimate | Per-LLM-call |
| **Collective calibration** | Local CalibrationTracker <-> On-chain Store | Shared accuracy stats | Delta frequency (consolidation) |

---

## Open Questions

1. **Cold start**: How should a new domain oracle bootstrap calibration before sufficient prediction-outcome pairs exist? Current answer: start with prior accuracy 0.5 and wide conformal intervals, tightening as data accumulates. Is this sufficient for safety-critical domains?

2. **Non-stationary environments**: The EMA-based ResidualCorrector adapts to slowly-changing bias, but regime changes (market crashes, major refactors) can cause sudden bias shifts. Should the corrector detect regime changes and reset, or should it use a heavier-tailed distribution?

3. **Cross-domain calibration transfer**: If the chain oracle is well-calibrated, does that evidence transfer to the coding oracle for the same model? The CalibrationTracker tracks per-domain, but model-level calibration quality might be partially shared.

4. **Adversarial prediction**: If agents share calibration data on-chain, an adversary could pollute the collective calibration. The trust mechanism (reputation-weighted contributions) mitigates this, but what is the attack surface?

---

## Implementation Tasks

- [ ] Extract `OracleScoreCell` from existing Oracle trait code (`crates/roko-learn/src/`) to express as Cell with Score protocol conformance
- [ ] Wire `ResidualCorrector` as explicit Functor in the orchestrate.rs dispatch path (currently called inline)
- [ ] Add `CalibrationTracker` persistence to `.roko/learn/calibration.json` with per-(model, category) stats
- [ ] Implement `ConformalVerifyCell` with sliding-window calibration set and quantile computation
- [ ] Wire Oracle accuracy into CascadeRouter's `feedback()` method in `crates/roko-learn/src/cascade_router.rs`
- [ ] Add oracle-loop.toml graph definition for declarative oracle instantiation
- [ ] Create domain-specific oracle Cells (chain, coding, research) as Score Cell specializations in `crates/roko-learn/src/oracle/`
- [ ] Wire CalibrationTracker data into VCG attention auction bids in `crates/roko-compose/`
