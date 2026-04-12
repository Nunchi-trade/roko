# The Oracle Trait — Universal Prediction Interface

> Every domain-specific prediction system in Roko implements a single trait. This document specifies the full Rust signature, supporting types, and integration points.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [00-vision-ta-generalized](./00-vision-ta-generalized.md) for motivation, [00-architecture](../00-architecture/INDEX.md) for Synapse traits and Engram
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §4, `refactoring-prd/01-synapse-architecture.md`

---

## The Oracle trait

The `Oracle` trait is the single interface through which all prediction capabilities are expressed. It is async, object-safe (`Send + Sync`), and designed for composition with the six Synapse traits:

```rust
/// Universal prediction interface for any domain.
///
/// Chain oracles predict prices, gas, liquidity depth, MEV opportunities.
/// Coding oracles predict build times, test failures, complexity drift.
/// Research oracles predict source reliability, contradiction density.
/// Custom domains implement the same trait with domain-specific queries.
pub trait Oracle: Send + Sync {
    /// Make a prediction about future state.
    ///
    /// The query encodes WHAT to predict. The context encodes the agent's
    /// current cognitive state (PAD vector, active knowledge, recent history).
    /// The returned Prediction includes confidence bounds and a time horizon
    /// by which the prediction should be resolved.
    async fn predict(
        &self,
        query: &OracleQuery,
        ctx: &Context,
    ) -> Result<Prediction>;

    /// Evaluate a past prediction against the actual outcome.
    ///
    /// The outcome is an Engram produced by external verification —
    /// a compiler result, a blockchain state, a test suite output, a
    /// replication study. The returned PredictionAccuracy drives feedback
    /// into Router, Daimon, Neuro, and Gate subsystems.
    async fn evaluate(
        &self,
        prediction: &Prediction,
        outcome: &Engram,
    ) -> Result<PredictionAccuracy>;
}
```

Both methods are async because predictions may require I/O — querying a blockchain node, running a compilation probe, or fetching citation data. The trait is `Send + Sync` to allow concurrent prediction evaluation across multiple oracle instances.

### Design rationale

The Oracle trait deliberately does **not** include:

- **`subscribe()`** — Real-time streams are handled by `Substrate.query()` with watch semantics. Oracles predict; Substrates observe.
- **`calibrate()`** — Calibration is a `Policy` concern. The `CalibrationTracker` (see [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md)) wraps any Oracle and adds calibration as a separate layer, following the Synapse Architecture's composition principle.
- **`batch_predict()`** — Batch semantics are provided by the caller iterating over queries. Oracle implementations may internally batch for efficiency (e.g., a chain oracle batching RPC calls), but this is an implementation detail, not a trait concern.

This follows Ousterhout's "deep module" principle (Ousterhout, 2018, *A Philosophy of Software Design*) — the interface is narrow (2 methods), but the implementation depth is substantial.

---

## OracleQuery — What to predict

The `OracleQuery` struct encodes the prediction request. It is domain-agnostic at the top level, with domain-specific payloads carried in the `domain` field:

```rust
/// A request for a prediction.
///
/// The query specifies WHAT to predict (domain-specific payload),
/// at what confidence level, over what time horizon.
pub struct OracleQuery {
    /// Unique identifier for this query (content-addressed, BLAKE3).
    pub id: ContentHash,

    /// What domain this prediction belongs to.
    pub domain: OracleDomain,

    /// The specific question being asked, as a domain-specific payload.
    /// Chain: "What will ETH price be in 5 blocks?"
    /// Coding: "Will this change break the test suite?"
    /// Research: "Is this source reliable for this claim?"
    pub payload: QueryPayload,

    /// How far into the future the prediction should cover.
    pub horizon: Duration,

    /// Minimum acceptable confidence for the prediction to be useful.
    /// Below this threshold, the oracle should return `Err(LowConfidence)`.
    pub min_confidence: f64,

    /// Tags for categorization (used by CalibrationTracker to track
    /// per-category accuracy).
    pub tags: BTreeMap<String, String>,

    /// Timestamp of query creation.
    pub created_at_ms: i64,
}
```

### OracleDomain — Domain classification

```rust
/// The domain a prediction belongs to.
///
/// Used by CalibrationTracker to maintain per-(model, domain) accuracy
/// statistics, and by the Router to select appropriate oracle implementations.
#[non_exhaustive]
pub enum OracleDomain {
    /// On-chain TA: price, gas, liquidity, MEV, protocol health.
    Chain,

    /// Software engineering: build time, test failure, complexity, dependency risk.
    Coding,

    /// Research and information analysis: source reliability, completeness, contradiction.
    Research,

    /// Operations: deployment success, infrastructure health, latency prediction.
    Operations,

    /// User-defined domain with a string identifier.
    Custom(String),
}
```

The `#[non_exhaustive]` attribute ensures new domains can be added without breaking existing code. The `Custom(String)` variant allows users to define domains not anticipated by the framework.

### QueryPayload — Domain-specific prediction targets

```rust
/// The specific prediction target.
///
/// Each variant carries domain-specific fields that the corresponding
/// Oracle implementation knows how to interpret.
pub enum QueryPayload {
    /// Chain domain predictions.
    Chain(ChainQueryPayload),

    /// Coding domain predictions.
    Coding(CodingQueryPayload),

    /// Research domain predictions.
    Research(ResearchQueryPayload),

    /// Operations domain predictions.
    Operations(OperationsQueryPayload),

    /// Arbitrary JSON payload for custom domains.
    Custom(serde_json::Value),
}

/// Chain-specific prediction targets.
pub struct ChainQueryPayload {
    /// The asset or protocol to predict about.
    pub target: ChainTarget,

    /// The metric to predict (price, gas, tvl, liquidity_depth, mev_opportunity).
    pub metric: ChainMetric,

    /// Optional: specific conditions to check (e.g., "if ETH > $3000").
    pub conditions: Vec<ChainCondition>,
}

/// Coding-specific prediction targets.
pub struct CodingQueryPayload {
    /// The scope of the prediction (file, module, crate, workspace).
    pub scope: CodingScope,

    /// The metric to predict (build_time, test_pass_rate, complexity_delta,
    /// dependency_risk, perf_regression).
    pub metric: CodingMetric,

    /// The change set that triggers this prediction (if applicable).
    pub change_context: Option<ChangeContext>,
}

/// Research-specific prediction targets.
pub struct ResearchQueryPayload {
    /// The source being evaluated.
    pub source: SourceReference,

    /// The metric to predict (reliability, completeness, contradiction_risk,
    /// replication_probability).
    pub metric: ResearchMetric,

    /// The claim or topic being assessed.
    pub claim_context: Option<String>,
}
```

---

## Prediction — The output

The `Prediction` struct is what an Oracle returns. It is designed to be stored as an Engram (via `Substrate.put()`) and later resolved against actual outcomes:

```rust
/// A prediction about future state.
///
/// Predictions are stored as Engrams with `kind: Kind::Prediction`.
/// They are resolved when the time horizon elapses or when external
/// verification produces an outcome.
pub struct Prediction {
    /// Content-addressed ID (BLAKE3 of query_id + predicted_value + confidence + horizon).
    pub id: ContentHash,

    /// The query this prediction answers.
    pub query_id: ContentHash,

    /// The predicted value. Domain-specific interpretation.
    ///
    /// Chain: PredictedValue::Numeric(3245.50)  // ETH price
    /// Coding: PredictedValue::Probability(0.85)  // test pass rate
    /// Research: PredictedValue::Ordinal(Reliability::High)  // source reliability
    pub value: PredictedValue,

    /// Confidence in this prediction, [0.0, 1.0].
    ///
    /// Maps to the Engram Score.confidence axis.
    /// Fed into the VCG auction as a bid weight for prediction context.
    pub confidence: f64,

    /// Prediction interval — the range within which the actual value
    /// is expected to fall with the stated confidence.
    ///
    /// For Probability predictions, this is a credible interval.
    /// For Numeric predictions, this is a prediction interval.
    pub interval: Option<PredictionInterval>,

    /// When this prediction was made.
    pub created_at_ms: i64,

    /// When this prediction should be resolved.
    /// After this time, the PredictionStore marks it for evaluation.
    pub resolve_by_ms: i64,

    /// The model and oracle that produced this prediction.
    /// Used by CalibrationTracker for per-model accuracy tracking.
    pub provenance: PredictionProvenance,

    /// Lineage — which Engrams informed this prediction.
    /// Enables causal replay: "why did the oracle predict X?"
    pub lineage: Vec<ContentHash>,

    /// Resolution state. None until resolved.
    pub outcome: Option<PredictionOutcome>,
}
```

### PredictedValue — Domain-polymorphic values

```rust
/// The value being predicted.
///
/// Supports numeric (prices, times), probability (pass rates, risk scores),
/// categorical (reliability levels), and compound (multiple related values).
pub enum PredictedValue {
    /// A numeric value (price, time, count).
    Numeric(f64),

    /// A probability [0.0, 1.0].
    Probability(f64),

    /// An ordinal category with associated numeric rank.
    Ordinal { label: String, rank: u32 },

    /// A boolean prediction (will it happen or not).
    Binary(bool),

    /// Multiple related predictions (e.g., price + volume + volatility).
    Compound(BTreeMap<String, PredictedValue>),
}
```

### PredictionInterval — Uncertainty quantification

```rust
/// Prediction interval bounding the expected outcome range.
///
/// The Oracle should produce intervals that are well-calibrated:
/// a 90% interval should contain the actual value 90% of the time.
/// CalibrationTracker measures this and adjusts via residual correction.
pub struct PredictionInterval {
    /// Lower bound of the prediction interval.
    pub lower: f64,

    /// Upper bound of the prediction interval.
    pub upper: f64,

    /// The coverage probability this interval targets (e.g., 0.90 for 90%).
    pub coverage: f64,
}
```

---

## PredictionAccuracy — The feedback signal

When a prediction resolves, the Oracle's `evaluate()` method returns a `PredictionAccuracy` that drives feedback into every Synapse subsystem:

```rust
/// The accuracy of a resolved prediction.
///
/// This is the primary feedback signal for the entire predictive foraging
/// loop. It feeds into:
/// - Router: accurate oracles get higher routing weight
/// - Daimon: prediction errors update Dominance (confidence)
/// - Neuro: prediction patterns become knowledge entries
/// - Gate: calibrate adaptive thresholds via EMA
/// - CalibrationTracker: update per-(model, category) bias estimates
pub struct PredictionAccuracy {
    /// The prediction being evaluated.
    pub prediction_id: ContentHash,

    /// The actual outcome Engram.
    pub outcome_id: ContentHash,

    /// Scalar accuracy [0.0, 1.0].
    /// 1.0 = perfect prediction. 0.0 = maximally wrong.
    pub accuracy: f64,

    /// Signed residual: predicted_value - actual_value.
    /// Positive = overestimated. Negative = underestimated.
    /// Used by ResidualCorrector for bias correction.
    pub residual: f64,

    /// Whether the prediction interval contained the actual value.
    /// Used by CalibrationTracker to measure interval calibration.
    pub interval_hit: Option<bool>,

    /// Time between prediction and resolution.
    /// Used to evaluate prediction quality at different horizons.
    pub resolution_lag_ms: i64,

    /// The domain and category for per-category tracking.
    pub domain: OracleDomain,
    pub category: String,
}
```

### PredictionOutcome — Resolution state

```rust
/// The resolution of a prediction.
pub struct PredictionOutcome {
    /// The actual value observed.
    pub actual: PredictedValue,

    /// The Engram that constitutes the evidence (compiler output, block data, etc.).
    pub evidence_id: ContentHash,

    /// When the outcome was observed.
    pub resolved_at_ms: i64,

    /// The accuracy assessment.
    pub accuracy: PredictionAccuracy,
}
```

---

## Integration with the Synapse traits

The Oracle trait is not a seventh Synapse trait — it is a **cognitive cross-cut** that integrates with all six traits through well-defined injection points:

### Substrate integration

Predictions and outcomes are persisted as Engrams:

```rust
// Store a new prediction
let prediction = oracle.predict(&query, &ctx).await?;
let engram = Engram::builder()
    .kind(Kind::Prediction)
    .body(Body::Json(serde_json::to_value(&prediction)?))
    .tag("domain", prediction.provenance.domain.as_str())
    .tag("horizon_ms", prediction.resolve_by_ms.to_string())
    .score(Score {
        confidence: prediction.confidence,
        novelty: 0.5,  // predictions start at baseline novelty
        utility: 0.0,  // utility accumulates after resolution
        reputation: prediction.provenance.model_reputation,
        ..Default::default()
    })
    .lineage(prediction.lineage.clone())
    .build();
substrate.put(engram).await?;
```

### Scorer integration — PredictiveScorer

The `PredictiveScorer` uses oracle accuracy history to weight Engram relevance:

```rust
/// Scores Engrams based on how well the oracle that produced them
/// has been performing recently.
pub struct PredictiveScorer {
    calibration: Arc<CalibrationTracker>,
}

impl Scorer for PredictiveScorer {
    fn score(&self, engram: &Engram) -> Score {
        let model = engram.provenance.model_id();
        let category = engram.tag("task_category").unwrap_or("unknown");

        // Oracle accuracy history modulates confidence
        let calibration = self.calibration.get_accuracy(model, category);
        let mut score = engram.score.clone();
        score.confidence *= calibration.recent_accuracy;
        score
    }
}
```

### Router integration

Prediction accuracy feeds into `Router.feedback()`, updating bandit arms for model selection:

```rust
// After prediction resolution
let accuracy = oracle.evaluate(&prediction, &outcome).await?;
router.feedback(
    &prediction.provenance.model_id,
    accuracy.accuracy,  // reward signal
)?;
```

This is how the CascadeRouter (with LinUCB + Thompson Sampling, see `roko-learn`) learns which models are best at predicting in which domains — the same bandit mechanism that routes LLM inference also routes oracle predictions.

### Gate integration

Prediction residuals calibrate adaptive gate thresholds:

```rust
// Residual correction updates gate thresholds
let residual = accuracy.residual;
gate_thresholds.update_ema(
    accuracy.category.as_str(),
    residual.abs(),
    alpha: 0.1,  // EMA smoothing factor
);
```

This creates a direct feedback loop: if an oracle consistently overestimates test pass rates, the gate threshold for that category tightens automatically.

### Composer integration — EFE bidding

Oracle predictions participate in the VCG attention auction (Vickrey 1961, Clarke 1971, Groves 1973) through Expected Free Energy (EFE) decomposition:

```rust
// Prediction context bids for attention budget
let efe = pragmatic_value + epistemic_value - ambiguity;
let bid = efe * urgency * affect_weight;
composer.bid("oracle_predictions", bid, prediction_context);
```

High-uncertainty predictions bid more aggressively because resolving them has high epistemic value (Friston, 2010, *Nature Reviews Neuroscience*).

### Policy integration — PredictionPolicy

The `PredictionPolicy` observes prediction streams and emits new Engrams based on patterns:

```rust
/// Watches prediction accuracy streams and generates meta-predictions,
/// warnings, and routing recommendations.
pub struct PredictionPolicy {
    tracker: Arc<CalibrationTracker>,
    neuro: Arc<dyn Substrate>,
}

impl Policy for PredictionPolicy {
    fn decide(&self, engrams: &[Engram]) -> Vec<Engram> {
        let mut outputs = Vec::new();

        // Detect systematic bias
        let bias = self.tracker.mean_residual("coding", "test_prediction");
        if bias.abs() > 0.15 {
            outputs.push(Engram::warning(
                format!("Systematic prediction bias detected: {:.2} in coding/test_prediction", bias),
            ));
        }

        // Detect accuracy degradation
        let trend = self.tracker.accuracy_trend("chain", "price");
        if trend < -0.05 {  // accuracy dropping
            outputs.push(Engram::insight(
                "Chain price prediction accuracy declining — possible regime change",
            ));
        }

        outputs
    }
}
```

---

## PredictionStore — Persistence layer

The `PredictionStore` manages the lifecycle of predictions from creation to resolution:

```rust
/// Manages prediction lifecycle: register → track → resolve → feedback.
///
/// Built on top of Substrate for persistence.
/// Provides efficient querying by domain, horizon, and resolution status.
pub struct PredictionStore {
    substrate: Arc<dyn Substrate>,
    pending: DashMap<ContentHash, Prediction>,
    resolved: DashMap<ContentHash, PredictionOutcome>,
}

impl PredictionStore {
    /// Register a new prediction for tracking.
    pub async fn register(&self, prediction: Prediction) -> Result<()>;

    /// Get all predictions that should have resolved by now.
    pub async fn pending_resolutions(&self) -> Vec<Prediction>;

    /// Resolve a prediction with an observed outcome.
    pub async fn resolve(
        &self,
        prediction_id: &ContentHash,
        outcome: &Engram,
        oracle: &dyn Oracle,
    ) -> Result<PredictionAccuracy>;

    /// Get accuracy statistics for a given domain and category.
    pub async fn accuracy_stats(
        &self,
        domain: &OracleDomain,
        category: &str,
    ) -> AccuracyStats;

    /// Get all unresolved predictions for a given domain.
    pub async fn pending_for_domain(
        &self,
        domain: &OracleDomain,
    ) -> Vec<Prediction>;
}
```

The `PredictionStore` has both off-chain (JSONL via `roko-fs`) and on-chain (Korai smart contract) variants. The on-chain variant enables collective calibration — all agents in the mesh share prediction outcomes, achieving up to 31.6× faster calibration for new agents (see [00-vision-ta-generalized.md](./00-vision-ta-generalized.md); collective calibration math in `refactoring-prd/09-innovations.md` §VI with explicit caveats about the independence assumption).

---

## ResidualCorrector — Bias elimination

The `ResidualCorrector` is a lightweight arithmetic layer that adjusts oracle predictions based on historical bias:

```rust
/// Corrects oracle predictions by subtracting the estimated systematic bias.
///
/// Cost: ~50 nanoseconds per correction (pure arithmetic, no LLM).
/// This is the mechanism that makes predictive foraging cost-effective —
/// the correction is free, but the learning is real.
pub struct ResidualCorrector {
    /// Mean bias per (model, category) pair.
    biases: DashMap<(String, String), ExponentialMovingAverage>,
}

impl ResidualCorrector {
    /// Apply bias correction to a raw prediction.
    pub fn correct(&self, prediction: &mut Prediction) {
        let key = (
            prediction.provenance.model_id.clone(),
            prediction.query_id_category(),
        );
        if let Some(bias) = self.biases.get(&key) {
            if let PredictedValue::Numeric(ref mut v) = prediction.value {
                *v -= bias.current();  // subtract estimated bias
            }
        }
    }

    /// Update bias estimate with a new residual.
    pub fn update(&self, model: &str, category: &str, residual: f64) {
        let key = (model.to_string(), category.to_string());
        self.biases
            .entry(key)
            .or_insert_with(|| ExponentialMovingAverage::new(0.1))
            .value_mut()
            .update(residual);
    }
}
```

The cost profile is critical: 50 nanoseconds per correction means 1,000 corrections per day per agent costs effectively nothing. This is what makes the predictive foraging loop viable at Gamma frequency (~5-15s) — corrections happen in microseconds, not milliseconds.

---

## CalibrationTracker — Per-model accuracy tracking

The `CalibrationTracker` aggregates prediction accuracy across (model, task_category) pairs, enabling bias-aware routing:

```rust
/// Tracks prediction calibration per (model, task_category) pair.
///
/// The key insight: different models have different biases on different
/// task categories. GPT-4 might overestimate test pass rates but
/// underestimate build times. Claude might do the reverse. The
/// CalibrationTracker learns these patterns and feeds them to the
/// ResidualCorrector and CascadeRouter.
pub struct CalibrationTracker {
    /// Per-(model, category) accuracy statistics.
    stats: DashMap<(String, String), CalibrationStats>,
}

pub struct CalibrationStats {
    /// Exponential moving average of residuals (bias estimate).
    pub mean_residual: ExponentialMovingAverage,

    /// Exponential moving average of absolute residuals (accuracy estimate).
    pub mean_absolute_error: ExponentialMovingAverage,

    /// Count of resolved predictions in this category.
    pub count: u64,

    /// Recent accuracy trend (positive = improving, negative = degrading).
    pub trend: f64,

    /// Interval calibration: fraction of outcomes within prediction intervals.
    pub interval_coverage: ExponentialMovingAverage,
}
```

On-chain (Korai), the CalibrationTracker is shared across all agents. A new agent importing the collective calibration starts with pre-learned biases for every model-category pair the collective has encountered. This is the concrete mechanism behind the 31.6× faster calibration heuristic (see `refactoring-prd/09-innovations.md` §VI).

---

## Implementing a custom Oracle

Adding prediction capability for a new domain requires implementing the `Oracle` trait:

```rust
/// Example: a deployment oracle that predicts deployment success probability.
pub struct DeploymentOracle {
    history: Arc<PredictionStore>,
    corrector: Arc<ResidualCorrector>,
}

#[async_trait]
impl Oracle for DeploymentOracle {
    async fn predict(
        &self,
        query: &OracleQuery,
        ctx: &Context,
    ) -> Result<Prediction> {
        let payload = query.payload.as_operations()?;

        // Gather features: recent deployment history, change size, time of day
        let features = self.extract_features(payload, ctx).await?;

        // Base prediction from historical success rate
        let mut prediction = Prediction {
            id: ContentHash::compute(&query, &features),
            query_id: query.id,
            value: PredictedValue::Probability(features.base_success_rate),
            confidence: features.sample_confidence,
            interval: Some(PredictionInterval {
                lower: features.base_success_rate - features.std_dev,
                upper: (features.base_success_rate + features.std_dev).min(1.0),
                coverage: 0.68,  // 1-sigma interval
            }),
            created_at_ms: now_ms(),
            resolve_by_ms: now_ms() + payload.expected_duration_ms,
            provenance: PredictionProvenance::local("deployment-oracle"),
            lineage: features.source_engrams,
            outcome: None,
        };

        // Apply bias correction from CalibrationTracker
        self.corrector.correct(&mut prediction);

        Ok(prediction)
    }

    async fn evaluate(
        &self,
        prediction: &Prediction,
        outcome: &Engram,
    ) -> Result<PredictionAccuracy> {
        let actual_success = outcome.tag("deployment_success")
            .map(|v| v == "true")
            .unwrap_or(false);

        let predicted_prob = prediction.value.as_probability()?;
        let actual_value = if actual_success { 1.0 } else { 0.0 };

        let accuracy = PredictionAccuracy {
            prediction_id: prediction.id,
            outcome_id: outcome.id,
            accuracy: 1.0 - (predicted_prob - actual_value).abs(),
            residual: predicted_prob - actual_value,
            interval_hit: prediction.interval.as_ref().map(|i| {
                actual_value >= i.lower && actual_value <= i.upper
            }),
            resolution_lag_ms: outcome.created_at_ms - prediction.created_at_ms,
            domain: OracleDomain::Operations,
            category: "deployment".to_string(),
        };

        // Update ResidualCorrector
        self.corrector.update(
            &prediction.provenance.model_id,
            "deployment",
            accuracy.residual,
        );

        Ok(accuracy)
    }
}
```

This pattern — predict, resolve, correct, repeat — is the same regardless of domain. The Oracle trait's simplicity (2 methods) hides substantial implementation depth in the supporting types (`PredictionStore`, `ResidualCorrector`, `CalibrationTracker`), following Ousterhout's deep module principle.

---

## Academic foundations

- Ousterhout, J. (2018). *A Philosophy of Software Design*. Yaknyam Press. — Deep module design principle motivating the 2-method Oracle trait.
- Vickrey, W. (1961). "Counterspeculation, Auctions, and Competitive Sealed Tenders." *Journal of Finance*, 16(1), 8-37. — VCG auction mechanism used for context allocation.
- Clarke, E. H. (1971). "Multipart Pricing of Public Goods." *Public Choice*, 11(1), 17-33. — VCG mechanism design.
- Groves, T. (1973). "Incentives in Teams." *Econometrica*, 41(4), 617-631. — VCG incentive compatibility.
- Friston, K. (2010). "The free-energy principle: a unified brain theory?" *Nature Reviews Neuroscience*, 11(2), 127-138. — EFE decomposition for context bidding.
- Conant, R. C., & Ashby, W. R. (1970). "Every good regulator of a system must be a model of that system." *International Journal of Systems Science*, 1(2), 89-97. — Good Regulator Theorem.
- Sumers, T. R., et al. (2023). "Cognitive Architectures for Language Agents." arXiv:2309.02427. — CoALA cognitive architecture.
- Chen, L., et al. (2023). "FrugalGPT." arXiv:2305.05176. — Cascade routing for cost reduction.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). — HDC for cross-domain pattern matching.

---

## Cross-references

- See [00-vision-ta-generalized.md](./00-vision-ta-generalized.md) for why TA is generalized across domains
- See [02-chain-oracles.md](./02-chain-oracles.md) for `ChainOracle` implementation
- See [03-coding-oracles.md](./03-coding-oracles.md) for `CodingOracle` implementation
- See [04-research-oracles.md](./04-research-oracles.md) for `ResearchOracle` implementation
- See [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for the full prediction loop with active inference
- See topic [05-learning](../05-learning/INDEX.md) for CascadeRouter bandit integration
