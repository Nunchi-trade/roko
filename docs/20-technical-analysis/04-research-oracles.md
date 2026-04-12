# Research Oracles — Prediction for Information Analysis

> Research oracles predict source reliability, information completeness, and contradiction risk. The same TA framework that tracks price trends tracks citation momentum. The same adversarial detection that identifies MEV identifies p-hacking.

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [01-oracle-trait](./01-oracle-trait.md) for Oracle trait, [02-chain-oracles](./02-chain-oracles.md) and [03-coding-oracles](./03-coding-oracles.md) for domain comparisons
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §4

---

## ResearchOracle — Implementation overview

The `ResearchOracle` implements the universal Oracle trait for research and information analysis tasks. It evaluates sources, detects contradictions, estimates completeness, and predicts replication probability:

```rust
pub struct ResearchOracle {
    /// Source evaluation engine.
    evaluator: Arc<SourceEvaluator>,

    /// Citation graph analyzer.
    citation_graph: Arc<CitationGraphAnalyzer>,

    /// Contradiction detection engine.
    contradiction_detector: Arc<ContradictionDetector>,

    /// Prediction persistence and tracking.
    prediction_store: Arc<PredictionStore>,

    /// Bias correction from collective calibration.
    corrector: Arc<ResidualCorrector>,

    /// Per-(model, category) accuracy tracking.
    calibration: Arc<CalibrationTracker>,
}

#[async_trait]
impl Oracle for ResearchOracle {
    async fn predict(
        &self,
        query: &OracleQuery,
        ctx: &Context,
    ) -> Result<Prediction> {
        let research_payload = query.payload.as_research()?;

        match research_payload.metric {
            ResearchMetric::Reliability => self.predict_reliability(research_payload, ctx).await,
            ResearchMetric::Completeness => self.predict_completeness(research_payload, ctx).await,
            ResearchMetric::ContradictionRisk => self.predict_contradiction(research_payload, ctx).await,
            ResearchMetric::ReplicationProbability => self.predict_replication(research_payload, ctx).await,
            ResearchMetric::CitationMomentum => self.predict_citation_momentum(research_payload, ctx).await,
        }
    }

    async fn evaluate(
        &self,
        prediction: &Prediction,
        outcome: &Engram,
    ) -> Result<PredictionAccuracy> {
        // Research outcomes are softer than chain/coding outcomes.
        // Verification comes from: cross-validation with other sources,
        // replication studies, meta-analyses, expert review.
        let actual = self.extract_research_outcome(outcome)?;
        let accuracy = self.compute_accuracy(prediction, &actual);

        self.corrector.update(
            &prediction.provenance.model_id,
            &accuracy.category,
            accuracy.residual,
        );

        Ok(accuracy)
    }
}
```

### Verification mechanisms

Research verification is inherently weaker than chain or coding verification. There is no compiler to produce a deterministic pass/fail. Instead, research oracles use probabilistic verification:

| Verification method | Strength | Latency | What it resolves |
|---|---|---|---|
| **Cross-source agreement** | Moderate | Immediate | If 5 independent sources agree, reliability is likely high |
| **Citation analysis** | Moderate | Immediate | High-citation papers are more likely reliable (imperfect signal) |
| **Replication study** | Strong | Months/years | Direct test of whether findings reproduce |
| **Meta-analysis** | Strong | Months/years | Statistical aggregation of multiple studies |
| **Expert review** | Moderate | Days/weeks | Human expert assessment of claims |
| **Logical consistency** | Moderate | Immediate | Internal contradictions indicate unreliability |

Because verification is softer, research oracle predictions carry wider confidence intervals than chain or coding predictions. The CalibrationTracker learns these domain-specific accuracy profiles automatically.

---

## The structural analogy table

| Chain TA | Coding TA | Research TA | Shared Math |
|---|---|---|---|
| Price prediction | Build time prediction | **Citation count prediction** | Time series forecasting |
| Volatility estimation | Build time variance | **Citation velocity variance** | Variance estimation |
| RSI (momentum) | Test pass rate momentum | **Field maturity oscillator** | Bounded oscillator |
| MACD (trend change) | Complexity trend change | **Paradigm shift detection** | Moving average crossover |
| Liquidity depth | Test coverage depth | **Information completeness depth** | Distribution analysis |
| MEV detection | Supply chain attacks | **p-hacking detection** | Adversarial threat analysis |
| TVL trends | Dependency count trends | **Publication volume trends** | Growth rate analysis |
| Funding rate | Error rate direction | **Contradiction density direction** | Directional bias |
| Liquidation proximity | Breakage proximity | **Replication crisis proximity** | Threshold distance |

---

## Research-specific prediction targets

### Source reliability estimation

```rust
/// Estimate the reliability of a source for a specific claim.
///
/// Uses multiple signals: publication venue, citation count,
/// author track record, methodology quality, internal consistency,
/// and cross-source agreement.
///
/// Analogous to credit rating in finance: both aggregate multiple
/// risk factors into a single reliability score.
pub struct SourceReliabilityEstimator {
    /// Venue quality scores (preprint, peer-reviewed, top-tier journal).
    venue_scores: HashMap<String, f64>,

    /// Author track record (historical reliability of predictions
    /// based on this author's work).
    author_scores: HashMap<String, AuthorReliability>,

    /// Cross-source agreement scores.
    agreement_cache: Arc<AgreementCache>,
}

pub struct SourceReliability {
    /// Overall reliability score [0.0, 1.0].
    pub score: f64,

    /// Decomposition for explainability.
    pub factors: ReliabilityFactors,

    /// Confidence in this reliability assessment.
    pub confidence: f64,
}

pub struct ReliabilityFactors {
    /// Venue quality (top-tier journal = high, preprint = lower).
    pub venue_quality: f64,

    /// Citation momentum (increasing citations = positive signal).
    pub citation_momentum: f64,

    /// Author track record (based on historical accuracy).
    pub author_reliability: f64,

    /// Methodology quality (sample size, statistical rigor, preregistration).
    pub methodology_quality: f64,

    /// Internal consistency (no contradictions within the source).
    pub internal_consistency: f64,

    /// Cross-source agreement (other sources confirm these claims).
    pub cross_source_agreement: f64,
}
```

### Information completeness assessment

```rust
/// Assess whether the agent has enough information about a topic
/// to make reliable decisions.
///
/// Completeness is measured against a topic model: which subtopics
/// have been covered, which are missing, and how critical each is.
///
/// Analogous to portfolio coverage analysis in finance: are all
/// risk factors accounted for?
pub struct CompletenessAssessor {
    /// Topic model: expected subtopics for a given research area.
    topic_models: HashMap<String, TopicModel>,

    /// Current coverage state.
    coverage: HashMap<String, TopicCoverage>,
}

pub struct TopicCoverage {
    /// Fraction of expected subtopics covered [0.0, 1.0].
    pub completeness: f64,

    /// List of covered subtopics with confidence per subtopic.
    pub covered: Vec<(String, f64)>,

    /// List of missing subtopics with criticality score.
    pub missing: Vec<(String, f64)>,

    /// Shannon entropy of the coverage distribution.
    /// Low entropy → concentrated coverage (some subtopics deep, others absent).
    /// High entropy → even coverage (breadth without depth).
    pub coverage_entropy: f64,
}

impl CompletenessAssessor {
    /// Predict whether additional research will meaningfully improve
    /// the agent's understanding.
    ///
    /// Uses Charnov's marginal value theorem (1976): stop foraging
    /// when the marginal information gain drops below the cost.
    pub fn should_continue_research(&self, topic: &str, cost_per_query: f64) -> bool {
        let coverage = self.coverage.get(topic);
        let marginal_gain = self.estimated_marginal_gain(coverage);
        marginal_gain > cost_per_query
    }
}
```

The stopping rule uses Charnov's marginal value theorem (Charnov, 1976, *Theoretical Population Biology*) — the same optimal foraging framework used by the predictive foraging system (see [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md)). An agent stops researching when the expected information gain per additional query drops below the cost of that query.

### Contradiction detection across sources

```rust
/// Detect contradictions between sources on the same topic.
///
/// Contradictions are the research domain's equivalent of arbitrage
/// in finance: two prices for the same asset indicate market
/// inefficiency. Two contradictory claims about the same phenomenon
/// indicate at least one source is wrong.
pub struct ContradictionDetector {
    /// Claim extraction engine.
    claim_extractor: Arc<ClaimExtractor>,

    /// Semantic similarity engine (HDC-based).
    similarity: Arc<HdcSimilarity>,

    /// Known contradictions with resolution status.
    known_contradictions: Vec<Contradiction>,
}

pub struct Contradiction {
    /// The two claims that contradict each other.
    pub claim_a: ContentHash,
    pub claim_b: ContentHash,

    /// Semantic similarity of the claims (high similarity + different conclusions = contradiction).
    pub claim_similarity: f64,

    /// How confident we are that these truly contradict.
    pub confidence: f64,

    /// Resolution status: which claim is more likely correct?
    pub resolution: Option<ContradictionResolution>,
}

pub struct ContradictionResolution {
    /// Which claim is favored after analysis.
    pub favored: ContentHash,

    /// Why (cross-source agreement, recency, methodology quality).
    pub reason: String,

    /// Confidence in the resolution.
    pub confidence: f64,
}
```

HDC encoding (Kleyko et al., 2022, *ACM Computing Surveys*) enables nanosecond contradiction detection: encode each claim as a 10,240-bit vector, compute Hamming similarity between claim pairs, and flag pairs with high semantic similarity but opposite conclusions. This runs at Gamma frequency without LLM cost.

### Replication probability estimation

```rust
/// Estimate the probability that a study's findings would replicate.
///
/// Based on the Open Science Collaboration's (2015) replication crisis
/// research: only 36% of psychology studies replicated. Signals that
/// predict replication failure include small sample size, novel claims
/// without preregistration, p-values near 0.05, and single-author studies.
///
/// Analogous to default probability estimation in credit risk.
pub struct ReplicationEstimator {
    /// Features that predict replication.
    feature_weights: ReplicationFeatures,

    /// Historical replication outcomes (training data).
    history: Vec<ReplicationOutcome>,
}

pub struct ReplicationFeatures {
    /// Sample size relative to effect size (power analysis).
    pub statistical_power: f64,

    /// Whether the study was preregistered.
    pub preregistered: bool,

    /// p-value proximity to 0.05 (p = 0.049 is suspicious).
    pub p_value_proximity: f64,

    /// Number of dependent variables tested (multiple comparisons risk).
    pub n_comparisons: usize,

    /// Effect size magnitude (implausibly large effects are suspicious).
    pub effect_size: f64,

    /// Field replication rate (psychology ≈ 36%, economics ≈ 61%).
    pub field_base_rate: f64,
}
```

### Citation momentum analysis

```rust
/// Track citation trends over time.
///
/// Analogous to price momentum in finance: papers with accelerating
/// citations are gaining influence. Papers with decelerating citations
/// may be superseded or found incorrect.
pub struct CitationMomentumAnalyzer {
    /// Citation time series per paper.
    citation_series: HashMap<String, Vec<(i64, u64)>>,

    /// Short-term citation EMA (6 months).
    short_ema: ExponentialMovingAverage,

    /// Long-term citation EMA (3 years).
    long_ema: ExponentialMovingAverage,
}

impl CitationMomentumAnalyzer {
    /// Compute citation MACD for a paper.
    ///
    /// Positive MACD → accelerating citations → growing influence.
    /// Negative MACD → decelerating citations → declining relevance.
    /// MACD crossover → paradigm shift signal.
    pub fn compute_macd(&self, paper_id: &str) -> Option<CitationMacd> {
        let series = self.citation_series.get(paper_id)?;
        let short = self.short_ema.compute(series);
        let long = self.long_ema.compute(series);

        Some(CitationMacd {
            value: short - long,
            signal: self.signal_ema.compute_from(short - long),
            histogram: (short - long) - self.signal_ema.current(),
        })
    }
}
```

---

## Adversarial dynamics: p-hacking detection

The research domain's adversarial threat model centers on publication bias, p-hacking, and selective reporting — researchers who game their methodology to produce publishable results:

```rust
/// Detect potential p-hacking in research sources.
///
/// Analogous to MEV detection in chain oracles: both identify
/// when participants are gaming the system.
///
/// Signals of p-hacking (Simmons et al., 2011):
/// - p-values clustered just below 0.05
/// - Effect sizes that don't decrease with larger samples
/// - Multiple unreported comparisons
/// - Post-hoc hypothesis refinement
pub struct PHackingDetector {
    /// p-value distribution analysis.
    p_value_analyzer: PValueAnalyzer,

    /// Effect size consistency checker.
    effect_checker: EffectSizeChecker,

    /// Known p-hacking patterns.
    patterns: Vec<PHackingPattern>,
}

pub struct PHackingAssessment {
    /// Overall p-hacking risk [0.0, 1.0].
    pub risk: f64,

    /// Specific red flags detected.
    pub red_flags: Vec<PHackingRedFlag>,

    /// Confidence in this assessment.
    pub confidence: f64,
}

pub enum PHackingRedFlag {
    /// p-value clustering below 0.05.
    PValueClustering { count: usize, expected_by_chance: f64 },

    /// Effect size inconsistent with sample size.
    EffectSizeAnomaly { reported: f64, expected_range: (f64, f64) },

    /// Multiple comparisons without correction.
    MultipleComparisons { reported_tests: usize, likely_tests: usize },

    /// Selective reporting (outcomes mentioned in methods but not results).
    SelectiveReporting { missing_outcomes: Vec<String> },
}
```

---

## Research oracle as VCG auction bidder

Research predictions participate in the VCG attention auction when the agent is composing context for a research task:

```rust
// Research oracle predictions bid for context inclusion.
// High-contradiction areas bid aggressively (high epistemic value).
// High-confidence areas bid modestly (low information gain).

let contradiction_bid = contradiction_detector.risk_score(topic)
    * urgency
    * affect_weight;

let completeness_bid = (1.0 - completeness_assessor.score(topic))
    * urgency
    * 0.8;  // completeness context is valuable but less urgent

let reliability_bid = reliability_estimator.uncertainty(source)
    * urgency
    * affect_weight;

composer.bid("contradiction_context", contradiction_bid, contradiction_engrams);
composer.bid("completeness_gaps", completeness_bid, gap_engrams);
composer.bid("reliability_warnings", reliability_bid, warning_engrams);
```

---

## Collective research calibration

On the Korai mesh, research oracles share their source reliability assessments. When 100 agents have all evaluated the same source, the collective reliability estimate converges faster than any individual agent's assessment. This is the research domain's version of collective calibration:

```
Agent A rates Source X at reliability 0.7
Agent B rates Source X at reliability 0.8  (used it successfully)
Agent C rates Source X at reliability 0.5  (found a contradiction)
...
Collective estimate: weighted average by agent reputation = 0.68
New agent importing this: starts at 0.68, not at 0.5 (prior)
```

This directly implements the 31.6× faster calibration heuristic from `refactoring-prd/09-innovations.md` §VI, adapted for the research domain where "verification" is probabilistic rather than deterministic.

---

## Academic foundations

- Open Science Collaboration. (2015). "Estimating the reproducibility of psychological science." *Science*, 349(6251), aac4716. — Replication crisis data (36% replication rate).
- Simmons, J. P., Nelson, L. D., & Simonsohn, U. (2011). "False-Positive Psychology." *Psychological Science*, 22(11), 1359-1366. — p-hacking mechanisms and detection.
- Ioannidis, J. P. A. (2005). "Why Most Published Research Findings Are False." *PLoS Medicine*, 2(8), e124. — Base rate of false positives in research.
- Charnov, E. L. (1976). "Optimal foraging: the marginal value theorem." *Theoretical Population Biology*, 9, 129-136. — Stopping rule for information foraging.
- Pirolli, P., & Card, S. (1999). "Information foraging." *Psychological Review*, 106(4), 643-675. — Optimal foraging applied to information retrieval.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). — HDC for contradiction detection via similarity.

---

## Cross-references

- See [01-oracle-trait.md](./01-oracle-trait.md) for the Oracle trait interface
- See [02-chain-oracles.md](./02-chain-oracles.md) for chain domain comparison
- See [03-coding-oracles.md](./03-coding-oracles.md) for coding domain comparison
- See [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) for HDC-based contradiction detection details
- See [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for the Charnov stopping rule
