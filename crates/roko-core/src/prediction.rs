//! Predictive-foraging primitives: oracle contracts, calibration-aware scoring,
//! and policy hooks.

use crate::{
    Budget, ContentHash, Context, Engram, Kind, Policy, Provenance, Score, Scorer, error::Result,
};
use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

/// Universal prediction interface for any structured domain.
///
/// Oracles are a cross-cutting surface over the six Synapse traits: they read
/// current context and prior engrams, return falsifiable predictions, and later
/// evaluate those predictions against externally observed outcomes.
#[async_trait]
pub trait Oracle: Send + Sync {
    /// Make a prediction about future state.
    async fn predict(&self, query: &OracleQuery, ctx: &Context) -> Result<Prediction>;

    /// Evaluate a past prediction against an observed outcome engram.
    async fn evaluate(
        &self,
        prediction: &Prediction,
        outcome: &Engram,
    ) -> Result<PredictionAccuracy>;
}

/// A request for an oracle prediction.
///
/// The top-level shape is domain-agnostic while [`QueryPayload`] carries the
/// domain-specific target and metric.
#[derive(Clone, Debug, PartialEq)]
pub struct OracleQuery {
    /// Unique identifier for this query.
    pub id: ContentHash,
    /// Prediction domain.
    pub domain: OracleDomain,
    /// Domain-specific question being asked.
    pub payload: QueryPayload,
    /// Future span covered by the prediction.
    pub horizon: Duration,
    /// Minimum useful confidence in `[0.0, 1.0]`.
    pub min_confidence: f64,
    /// Categorization tags used by calibration and routing.
    pub tags: BTreeMap<String, String>,
    /// Unix milliseconds when the query was created.
    pub created_at_ms: i64,
}

impl OracleQuery {
    /// Create a new prediction query with a generated content hash.
    pub fn new(
        domain: OracleDomain,
        payload: QueryPayload,
        horizon: Duration,
        min_confidence: f64,
    ) -> Self {
        let created_at_ms = chrono::Utc::now().timestamp_millis();
        let id = ContentHash::of(
            format!(
                "{domain:?}|{payload:?}|{}|{created_at_ms}",
                horizon.as_millis()
            )
            .as_bytes(),
        );
        Self {
            id,
            domain,
            payload,
            horizon,
            min_confidence: min_confidence.clamp(0.0, 1.0),
            tags: BTreeMap::new(),
            created_at_ms,
        }
    }

    /// Add a calibration/routing tag to this query.
    #[must_use]
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Return the category tag used by calibration, if present.
    #[must_use]
    pub fn category(&self) -> Option<&str> {
        self.tags
            .get("task_category")
            .or_else(|| self.tags.get("category"))
            .map(String::as_str)
    }
}

/// Domain classification for oracle predictions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum OracleDomain {
    /// On-chain TA: price, gas, liquidity, MEV, protocol health.
    Chain,
    /// Software engineering: build time, tests, complexity, dependency risk.
    Coding,
    /// Research and information analysis.
    Research,
    /// Operations: deployment success, infrastructure health, latency.
    Operations,
    /// User-defined domain with a stable identifier.
    Custom(String),
}

impl std::fmt::Display for OracleDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chain => f.write_str("chain"),
            Self::Coding => f.write_str("coding"),
            Self::Research => f.write_str("research"),
            Self::Operations => f.write_str("operations"),
            Self::Custom(domain) => f.write_str(domain),
        }
    }
}

/// Domain-specific prediction target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum QueryPayload {
    /// Chain-domain prediction target.
    Chain(ChainQueryPayload),
    /// Coding-domain prediction target.
    Coding(CodingQueryPayload),
    /// Research-domain prediction target.
    Research(ResearchQueryPayload),
    /// Operations-domain prediction target.
    Operations(OperationsQueryPayload),
    /// Arbitrary JSON payload for custom domains.
    Custom(serde_json::Value),
}

impl QueryPayload {
    /// Return the chain payload when this is a chain query.
    #[must_use]
    pub const fn as_chain(&self) -> Option<&ChainQueryPayload> {
        match self {
            Self::Chain(payload) => Some(payload),
            _ => None,
        }
    }

    /// Return the coding payload when this is a coding query.
    #[must_use]
    pub const fn as_coding(&self) -> Option<&CodingQueryPayload> {
        match self {
            Self::Coding(payload) => Some(payload),
            _ => None,
        }
    }

    /// Return the research payload when this is a research query.
    #[must_use]
    pub const fn as_research(&self) -> Option<&ResearchQueryPayload> {
        match self {
            Self::Research(payload) => Some(payload),
            _ => None,
        }
    }
}

/// Chain-specific prediction target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainQueryPayload {
    /// Asset, pool, protocol, or wallet being predicted.
    pub target: ChainTarget,
    /// Chain metric to predict.
    pub metric: ChainMetric,
    /// Optional conditional clauses for scenario predictions.
    pub conditions: Vec<ChainCondition>,
}

/// Chain prediction target descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChainTarget {
    /// Asset symbol or address.
    Asset(String),
    /// Protocol identifier.
    Protocol(String),
    /// Liquidity pool identifier.
    Pool(String),
    /// Wallet or account identifier.
    Wallet(String),
}

/// Chain metric predicted by a chain oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChainMetric {
    /// Asset or pool price.
    Price,
    /// Gas or fee-market level.
    Gas,
    /// Realized or implied volatility.
    Volatility,
    /// Available execution depth.
    LiquidityDepth,
    /// MEV opportunity or adversarial pressure.
    MevOpportunity,
    /// Protocol health signal.
    ProtocolHealth,
    /// Perpetual funding rate.
    FundingRate,
    /// Yield spread or term-structure slope.
    YieldSpread,
}

/// Conditional clause for a chain prediction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainCondition {
    /// Field or signal to compare.
    pub field: String,
    /// Comparison operator such as `>`, `<`, or `=`.
    pub operator: String,
    /// Threshold value for the condition.
    pub value: PredictedValue,
}

/// Coding-specific prediction target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CodingQueryPayload {
    /// Code scope being predicted.
    pub scope: CodingScope,
    /// Software-engineering metric to predict.
    pub metric: CodingMetric,
    /// Optional change set that triggered this prediction.
    pub change_context: Option<ChangeContext>,
}

/// Scope of a coding oracle prediction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CodingScope {
    /// A single file path.
    File(String),
    /// A module path.
    Module(String),
    /// A crate/package name.
    Crate(String),
    /// The whole workspace.
    Workspace,
}

/// Coding metric predicted by a coding oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CodingMetric {
    /// Build or compile duration.
    BuildTime,
    /// Probability or rate of passing tests.
    TestPassRate,
    /// Complexity delta for a scope.
    ComplexityDelta,
    /// Dependency or supply-chain risk.
    DependencyRisk,
    /// Performance regression likelihood or magnitude.
    PerfRegression,
    /// Test coverage impact.
    CoverageImpact,
}

/// Change context used by coding predictions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeContext {
    /// Files touched by the change set.
    pub files_changed: Vec<String>,
    /// Human-readable diff summary.
    pub diff_summary: String,
    /// Optional commit or patch identifier.
    pub change_id: Option<String>,
}

/// Research-specific prediction target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchQueryPayload {
    /// Source being evaluated.
    pub source: SourceReference,
    /// Research metric to predict.
    pub metric: ResearchMetric,
    /// Claim or topic being assessed.
    pub claim_context: Option<String>,
}

/// Reference to a research or information source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceReference {
    /// Human-readable title.
    pub title: String,
    /// Optional URL.
    pub url: Option<String>,
    /// Optional DOI or stable publication identifier.
    pub doi: Option<String>,
    /// Author names when known.
    pub authors: Vec<String>,
}

/// Research metric predicted by a research oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ResearchMetric {
    /// Source reliability.
    Reliability,
    /// Topic or evidence completeness.
    Completeness,
    /// Risk that sources contradict each other.
    ContradictionRisk,
    /// Probability that a finding replicates.
    ReplicationProbability,
    /// Citation or attention momentum.
    CitationMomentum,
}

/// Operations-specific prediction target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationsQueryPayload {
    /// Service, deployment, or subsystem identifier.
    pub target: String,
    /// Operations metric to predict.
    pub metric: OperationsMetric,
    /// Optional operational conditions for scenario predictions.
    pub conditions: BTreeMap<String, String>,
}

/// Operations metric predicted by an operations oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum OperationsMetric {
    /// Deployment success probability.
    DeploymentSuccess,
    /// Infrastructure health.
    InfrastructureHealth,
    /// Latency or duration.
    Latency,
    /// Error rate.
    ErrorRate,
    /// Cost or budget consumption.
    Cost,
}

/// A prediction about future state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    /// Content-addressed prediction identifier.
    pub id: ContentHash,
    /// Query answered by this prediction.
    pub query_id: ContentHash,
    /// Optional domain hint for pending-domain queries before resolution.
    pub domain: Option<OracleDomain>,
    /// Domain-polymorphic predicted value.
    pub value: PredictedValue,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Optional prediction interval or credible interval.
    pub interval: Option<PredictionInterval>,
    /// Unix milliseconds when the prediction was made.
    pub created_at_ms: i64,
    /// Unix milliseconds by which the prediction should be resolved.
    pub resolve_by_ms: i64,
    /// Model/oracle provenance for calibration.
    pub provenance: PredictionProvenance,
    /// Engram lineage that informed the prediction.
    pub lineage: Vec<ContentHash>,
    /// Resolution state; absent until the prediction is resolved.
    pub outcome: Option<PredictionOutcome>,
}

impl Prediction {
    /// Create a prediction with a generated content hash.
    pub fn new(
        query_id: ContentHash,
        value: PredictedValue,
        confidence: f64,
        resolve_by_ms: i64,
        provenance: PredictionProvenance,
    ) -> Self {
        let created_at_ms = chrono::Utc::now().timestamp_millis();
        let id = ContentHash::of(
            format!("{query_id}|{value:?}|{confidence}|{resolve_by_ms}").as_bytes(),
        );
        Self {
            id,
            query_id,
            domain: None,
            value,
            confidence: confidence.clamp(0.0, 1.0),
            interval: None,
            created_at_ms,
            resolve_by_ms,
            provenance,
            lineage: Vec::new(),
            outcome: None,
        }
    }

    /// Attach a prediction interval.
    #[must_use]
    pub const fn with_interval(mut self, interval: PredictionInterval) -> Self {
        self.interval = Some(interval);
        self
    }

    /// Attach the prediction domain for pending-domain indexing.
    #[must_use]
    pub fn with_domain(mut self, domain: OracleDomain) -> Self {
        self.domain = Some(domain);
        self
    }

    /// Attach lineage engrams used to produce this prediction.
    #[must_use]
    pub fn with_lineage(mut self, lineage: impl IntoIterator<Item = ContentHash>) -> Self {
        self.lineage = lineage.into_iter().collect();
        self
    }

    /// Whether this prediction should now be resolved.
    #[must_use]
    pub const fn is_due(&self, now_ms: i64) -> bool {
        self.outcome.is_none() && self.resolve_by_ms <= now_ms
    }
}

/// Model and oracle provenance for a prediction.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredictionProvenance {
    /// Model or deterministic engine identifier.
    pub model_id: String,
    /// Oracle implementation identifier.
    pub oracle_id: String,
    /// Optional agent identifier that requested or produced the prediction.
    pub agent_id: Option<String>,
}

impl PredictionProvenance {
    /// Construct prediction provenance.
    pub fn new(model_id: impl Into<String>, oracle_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            oracle_id: oracle_id.into(),
            agent_id: None,
        }
    }

    /// Attach an agent identifier.
    #[must_use]
    pub fn with_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }
}

/// The value being predicted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PredictedValue {
    /// Numeric value such as price, duration, or count.
    Numeric(f64),
    /// Probability in `[0.0, 1.0]`.
    Probability(f64),
    /// Ordered category with a numeric rank.
    Ordinal {
        /// Category label.
        label: String,
        /// Ordinal rank.
        rank: u32,
    },
    /// Boolean outcome.
    Binary(bool),
    /// Multiple named predictions.
    Compound(BTreeMap<String, PredictedValue>),
}

impl PredictedValue {
    /// Return a numeric scalar when this value has one.
    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Numeric(value) | Self::Probability(value) => Some(*value),
            Self::Ordinal { rank, .. } => Some(*rank as f64),
            Self::Binary(value) => Some(if *value { 1.0 } else { 0.0 }),
            Self::Compound(_) => None,
        }
    }
}

/// Prediction interval bounding expected outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PredictionInterval {
    /// Lower bound.
    pub lower: f64,
    /// Upper bound.
    pub upper: f64,
    /// Target coverage probability in `[0.0, 1.0]`.
    pub coverage: f64,
}

impl PredictionInterval {
    /// Construct a normalized prediction interval.
    pub fn new(lower: f64, upper: f64, coverage: f64) -> Self {
        let (lower, upper) = if lower <= upper {
            (lower, upper)
        } else {
            (upper, lower)
        };
        Self {
            lower,
            upper,
            coverage: coverage.clamp(0.0, 1.0),
        }
    }

    /// Whether a scalar outcome falls inside this interval.
    #[must_use]
    pub fn contains(&self, value: f64) -> bool {
        value >= self.lower && value <= self.upper
    }
}

/// Accuracy feedback for a resolved prediction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PredictionAccuracy {
    /// Prediction being evaluated.
    pub prediction_id: ContentHash,
    /// Evidence engram for the observed outcome.
    pub outcome_id: ContentHash,
    /// Scalar accuracy in `[0.0, 1.0]`.
    pub accuracy: f64,
    /// Signed residual: predicted value minus actual value.
    pub residual: f64,
    /// Whether the prediction interval contained the actual value.
    pub interval_hit: Option<bool>,
    /// Time between prediction and resolution.
    pub resolution_lag_ms: i64,
    /// Prediction domain.
    pub domain: OracleDomain,
    /// Calibration category.
    pub category: String,
}

impl PredictionAccuracy {
    /// Construct prediction accuracy feedback.
    pub fn new(
        prediction_id: ContentHash,
        outcome_id: ContentHash,
        accuracy: f64,
        residual: f64,
        domain: OracleDomain,
        category: impl Into<String>,
    ) -> Self {
        Self {
            prediction_id,
            outcome_id,
            accuracy: accuracy.clamp(0.0, 1.0),
            residual,
            interval_hit: None,
            resolution_lag_ms: 0,
            domain,
            category: category.into(),
        }
    }

    /// Attach interval-hit information.
    #[must_use]
    pub const fn with_interval_hit(mut self, interval_hit: Option<bool>) -> Self {
        self.interval_hit = interval_hit;
        self
    }

    /// Attach the resolution lag.
    #[must_use]
    pub const fn with_resolution_lag_ms(mut self, lag_ms: i64) -> Self {
        self.resolution_lag_ms = lag_ms;
        self
    }
}

// ─── CRPS: Continuous Ranked Probability Score ──────────────────────

/// Continuous Ranked Probability Score (CRPS) — a proper scoring rule for
/// distribution forecasts.
///
/// CRPS generalizes MAE (Mean Absolute Error) to full probability distributions.
/// It measures how well a predictive distribution matches an observation:
/// - CRPS = 0 means the forecast CDF is a step function at the observation (perfect).
/// - CRPS increases as the forecast distribution diverges from reality.
///
/// For a forecast CDF F and observation y:
///   CRPS(F, y) = integral[-inf, +inf] (F(x) - I(x >= y))^2 dx
///
/// ## Usage
///
/// ```rust
/// use roko_core::prediction::crps;
///
/// // Gaussian forecast: mean=10.0, std=2.0, actual observation=11.0
/// let score = crps::gaussian(10.0, 2.0, 11.0);
/// assert!(score > 0.0);
/// assert!(score < 3.0); // Reasonable for a 0.5-sigma deviation
/// ```
pub mod crps {
    /// CRPS for a Gaussian (normal) forecast distribution.
    ///
    /// Closed-form: `CRPS(N(mu, sigma), y) = sigma * [z*(2*Phi(z) - 1) + 2*phi(z) - 1/sqrt(pi)]`
    /// where `z = (y - mu) / sigma`, Phi is the standard normal CDF, phi is the PDF.
    ///
    /// # Arguments
    /// - `mean`: Forecast distribution mean.
    /// - `std_dev`: Forecast distribution standard deviation (must be > 0).
    /// - `observation`: The actual observed value.
    ///
    /// # Returns
    /// The CRPS score (lower is better, 0 = perfect).
    pub fn gaussian(mean: f64, std_dev: f64, observation: f64) -> f64 {
        if std_dev <= 0.0 {
            // Degenerate case: point forecast → CRPS = MAE.
            return (observation - mean).abs();
        }

        let z = (observation - mean) / std_dev;
        let phi_z = standard_normal_pdf(z);
        let big_phi_z = standard_normal_cdf(z);

        // Closed-form CRPS for Gaussian.
        // 1/sqrt(pi) = 0.5641895835477563
        std_dev * (z * (2.0 * big_phi_z - 1.0) + 2.0 * phi_z - std::f64::consts::PI.sqrt().recip())
    }

    /// CRPS for an empirical forecast distribution given as sorted samples.
    ///
    /// Uses the empirical CDF representation:
    ///   CRPS = (1/n) * sum_i |x_i - y| - (1/(2*n^2)) * sum_i sum_j |x_i - x_j|
    ///
    /// # Arguments
    /// - `samples`: Forecast samples (will be used as-is; caller should sort for efficiency).
    /// - `observation`: The actual observed value.
    ///
    /// # Returns
    /// The CRPS score (lower is better, 0 = perfect).
    pub fn empirical(samples: &[f64], observation: f64) -> f64 {
        if samples.is_empty() {
            return f64::NAN;
        }
        let n = samples.len() as f64;

        // Term 1: mean absolute difference from observation.
        let term1: f64 = samples
            .iter()
            .map(|&x| (x - observation).abs())
            .sum::<f64>()
            / n;

        // Term 2: mean pairwise absolute difference (spread penalty).
        let mut term2 = 0.0;
        for i in 0..samples.len() {
            for j in 0..samples.len() {
                term2 += (samples[i] - samples[j]).abs();
            }
        }
        term2 /= 2.0 * n * n;

        term1 - term2
    }

    /// CRPS for a uniform distribution on [a, b].
    ///
    /// Closed-form for U(a, b):
    ///   - If y < a: CRPS = (a - y) + (b - a)/3
    ///   - If y > b: CRPS = (y - b) + (b - a)/3
    ///   - If a <= y <= b: CRPS = (b - a) * [(z^2 + (1-z)^2 - 1) / 3 + z*(2*z - 1)]
    ///     where z = (y - a) / (b - a)... simplified below.
    pub fn uniform(lower: f64, upper: f64, observation: f64) -> f64 {
        if upper <= lower {
            return (observation - lower).abs();
        }

        let width = upper - lower;

        if observation < lower {
            (lower - observation) + width / 3.0
        } else if observation > upper {
            (observation - upper) + width / 3.0
        } else {
            // Inside the interval.
            let z = (observation - lower) / width;
            width * (z * z - z + 1.0 / 3.0)
        }
    }

    /// Standard normal PDF: phi(z) = exp(-z^2/2) / sqrt(2*pi).
    fn standard_normal_pdf(z: f64) -> f64 {
        (-0.5 * z * z).exp() / (2.0 * std::f64::consts::PI).sqrt()
    }

    /// Standard normal CDF using the error function approximation.
    ///
    /// Abramowitz & Stegun approximation 7.1.26 (max error < 1.5e-7).
    fn standard_normal_cdf(z: f64) -> f64 {
        0.5 * (1.0 + erf(z / std::f64::consts::SQRT_2))
    }

    /// Error function approximation (Horner form).
    fn erf(x: f64) -> f64 {
        let sign = x.signum();
        let x = x.abs();

        // Abramowitz & Stegun coefficients.
        let t = 1.0 / (1.0 + 0.3275911 * x);
        let poly = t
            * (0.254829592
                + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));

        sign * (1.0 - poly * (-x * x).exp())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn gaussian_crps_perfect_prediction() {
            // Point prediction at the observation → CRPS approaches 0.
            let score = gaussian(10.0, 0.001, 10.0);
            assert!(
                score < 0.001,
                "perfect Gaussian prediction should have CRPS near 0, got {score}"
            );
        }

        #[test]
        fn gaussian_crps_increases_with_error() {
            let close = gaussian(10.0, 1.0, 10.5);
            let far = gaussian(10.0, 1.0, 15.0);
            assert!(
                far > close,
                "CRPS should increase with distance: close={close}, far={far}"
            );
        }

        #[test]
        fn gaussian_crps_increases_with_uncertainty() {
            // Same observation, wider distribution = worse CRPS.
            let narrow = gaussian(10.0, 0.5, 10.0);
            let wide = gaussian(10.0, 5.0, 10.0);
            assert!(
                wide > narrow,
                "wider distribution should have worse CRPS: narrow={narrow}, wide={wide}"
            );
        }

        #[test]
        fn empirical_crps_perfect_sample() {
            // All samples at the observation.
            let samples = vec![5.0, 5.0, 5.0, 5.0];
            let score = empirical(&samples, 5.0);
            assert!(
                score.abs() < 1e-10,
                "perfect empirical forecast should have CRPS=0, got {score}"
            );
        }

        #[test]
        fn empirical_crps_spread_penalty() {
            // Samples spread around observation.
            let concentrated = vec![9.9, 10.0, 10.0, 10.1];
            let spread = vec![5.0, 8.0, 12.0, 15.0];
            let s1 = empirical(&concentrated, 10.0);
            let s2 = empirical(&spread, 10.0);
            assert!(
                s2 > s1,
                "spread forecast should score worse: concentrated={s1}, spread={s2}"
            );
        }

        #[test]
        fn uniform_crps_observation_at_center() {
            let score = uniform(0.0, 10.0, 5.0);
            // For U(0,10) with y=5 (center): CRPS = 10 * (0.25 - 0.5 + 1/3) = 10 * 0.0833 = 0.833
            assert!(
                (score - 0.8333).abs() < 0.01,
                "uniform center CRPS should be ~0.833, got {score}"
            );
        }

        #[test]
        fn uniform_crps_observation_outside() {
            let score = uniform(0.0, 10.0, 15.0);
            // Outside: (15-10) + 10/3 = 5 + 3.33 = 8.33
            assert!(
                (score - 8.333).abs() < 0.01,
                "uniform outside CRPS should be ~8.33, got {score}"
            );
        }

        #[test]
        fn degenerate_gaussian_equals_mae() {
            let score = gaussian(10.0, 0.0, 13.0);
            assert!(
                (score - 3.0).abs() < 1e-10,
                "degenerate Gaussian CRPS should equal MAE=3.0, got {score}"
            );
        }
    }
}

/// Resolution state of a prediction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PredictionOutcome {
    /// Actual value observed.
    pub actual: PredictedValue,
    /// Evidence engram that produced this observation.
    pub evidence_id: ContentHash,
    /// Unix milliseconds when the outcome was observed.
    pub resolved_at_ms: i64,
    /// Accuracy assessment.
    pub accuracy: PredictionAccuracy,
}

/// Aggregate accuracy statistics for a domain/category slice.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AccuracyStats {
    /// Number of resolved predictions.
    pub count: u64,
    /// Mean scalar accuracy.
    pub mean_accuracy: f64,
    /// Mean signed residual.
    pub mean_residual: f64,
    /// Fraction of interval predictions that contained the outcome.
    pub interval_coverage: f64,
}

/// In-memory prediction lifecycle manager.
///
/// This Phase 2+ surface mirrors the documented `PredictionStore` contract
/// without committing to a persistence backend. Off-chain and Korai-backed
/// stores can preserve the same public lifecycle later.
#[derive(Debug, Default)]
pub struct PredictionStore {
    pending: RwLock<HashMap<ContentHash, Prediction>>,
    resolved: RwLock<HashMap<ContentHash, PredictionOutcome>>,
}

impl PredictionStore {
    /// Create an empty prediction store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a prediction for future resolution.
    pub async fn register(&self, prediction: Prediction) -> Result<()> {
        self.pending.write().insert(prediction.id, prediction);
        Ok(())
    }

    /// Return predictions whose resolution horizon has elapsed.
    pub async fn pending_resolutions(&self, now_ms: i64) -> Vec<Prediction> {
        self.pending
            .read()
            .values()
            .filter(|prediction| prediction.is_due(now_ms))
            .cloned()
            .collect()
    }

    /// Resolve a prediction with an observed outcome using the supplied oracle.
    pub async fn resolve(
        &self,
        prediction_id: &ContentHash,
        outcome: &Engram,
        oracle: &dyn Oracle,
    ) -> Result<PredictionAccuracy> {
        let mut prediction = self
            .pending
            .write()
            .remove(prediction_id)
            .ok_or(crate::RokoError::NotFound(*prediction_id))?;
        let accuracy = oracle.evaluate(&prediction, outcome).await?;
        let resolved_at_ms = chrono::Utc::now().timestamp_millis();
        let actual = prediction.value.clone();
        let prediction_outcome = PredictionOutcome {
            actual,
            evidence_id: outcome.id,
            resolved_at_ms,
            accuracy: accuracy.clone(),
        };
        prediction.outcome = Some(prediction_outcome.clone());
        self.resolved
            .write()
            .insert(*prediction_id, prediction_outcome);
        Ok(accuracy)
    }

    /// Get aggregate accuracy statistics for a domain/category pair.
    pub async fn accuracy_stats(&self, domain: &OracleDomain, category: &str) -> AccuracyStats {
        let mut stats = AccuracyStats::default();
        let mut interval_count = 0_u64;
        for outcome in self.resolved.read().values() {
            if &outcome.accuracy.domain != domain || outcome.accuracy.category != category {
                continue;
            }
            stats.count = stats.count.saturating_add(1);
            stats.mean_accuracy += outcome.accuracy.accuracy;
            stats.mean_residual += outcome.accuracy.residual;
            if let Some(hit) = outcome.accuracy.interval_hit {
                interval_count = interval_count.saturating_add(1);
                if hit {
                    stats.interval_coverage += 1.0;
                }
            }
        }

        if stats.count > 0 {
            let count = stats.count as f64;
            stats.mean_accuracy /= count;
            stats.mean_residual /= count;
        }
        if interval_count > 0 {
            stats.interval_coverage /= interval_count as f64;
        }
        stats
    }

    /// Get unresolved predictions for a domain.
    pub async fn pending_for_domain(&self, domain: &OracleDomain) -> Vec<Prediction> {
        self.pending
            .read()
            .values()
            .filter(|prediction| prediction_domain(prediction) == Some(domain))
            .cloned()
            .collect()
    }

    /// Resolve a prediction and feed accuracy back into calibration and
    /// residual correction (TA-15 feedback loop wiring).
    ///
    /// This combines three feedback loops:
    /// 1. After resolution: update Oracle accuracy metrics via `resolve()`
    /// 2. Feed accuracy into `CalibrationTracker` for confidence adjustment
    /// 3. Feed residual into `ResidualCorrector` for bias correction
    pub async fn resolve_with_feedback(
        &self,
        prediction_id: &ContentHash,
        outcome: &Engram,
        oracle: &dyn Oracle,
        calibration: &CalibrationTracker,
        corrector: &ResidualCorrector,
    ) -> Result<PredictionAccuracy> {
        let accuracy = self.resolve(prediction_id, outcome, oracle).await?;

        // Feed accuracy into calibration tracker
        calibration.update_accuracy(&accuracy);

        // Feed residual into corrector for bias estimation
        corrector.update(
            &accuracy.domain.to_string(),
            &accuracy.category,
            accuracy.residual,
        );

        Ok(accuracy)
    }

    /// Attempt to resolve predictions that match a gate verdict outcome.
    ///
    /// Scans pending predictions for any whose domain matches the verdict's
    /// domain and whose horizon has elapsed, then resolves them. Returns
    /// all accuracy results.
    pub async fn resolve_from_gate_verdict(
        &self,
        verdict_engram: &Engram,
        oracle: &dyn Oracle,
        calibration: &CalibrationTracker,
        corrector: &ResidualCorrector,
    ) -> Vec<PredictionAccuracy> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let due = self.pending_resolutions(now_ms).await;
        let mut results = Vec::new();

        for prediction in &due {
            match self
                .resolve_with_feedback(
                    &prediction.id,
                    verdict_engram,
                    oracle,
                    calibration,
                    corrector,
                )
                .await
            {
                Ok(accuracy) => results.push(accuracy),
                Err(_) => continue,
            }
        }

        results
    }
}

fn prediction_domain(prediction: &Prediction) -> Option<&OracleDomain> {
    prediction.domain.as_ref().or_else(|| {
        prediction
            .outcome
            .as_ref()
            .map(|outcome| &outcome.accuracy.domain)
    })
}

/// Exponential moving average used by prediction calibration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExponentialMovingAverage {
    alpha: f64,
    current: f64,
    initialized: bool,
}

impl ExponentialMovingAverage {
    /// Construct an EMA with a smoothing factor in `[0.0, 1.0]`.
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.0, 1.0),
            current: 0.0,
            initialized: false,
        }
    }

    /// Update the EMA with a new sample.
    pub fn update(&mut self, sample: f64) {
        self.current = if self.initialized {
            self.alpha
                .mul_add(sample, (1.0 - self.alpha) * self.current)
        } else {
            self.initialized = true;
            sample
        };
    }

    /// Current EMA value.
    #[must_use]
    pub const fn current(&self) -> f64 {
        self.current
    }
}

/// Corrects oracle predictions by subtracting estimated systematic bias.
#[derive(Debug)]
pub struct ResidualCorrector {
    biases: RwLock<HashMap<(String, String), ExponentialMovingAverage>>,
    alpha: f64,
}

impl Default for ResidualCorrector {
    fn default() -> Self {
        Self::new(0.1)
    }
}

impl ResidualCorrector {
    /// Create a residual corrector with the given EMA smoothing factor.
    #[must_use]
    pub fn new(alpha: f64) -> Self {
        Self {
            biases: RwLock::new(HashMap::new()),
            alpha: alpha.clamp(0.0, 1.0),
        }
    }

    /// Correct a raw scalar prediction.
    #[must_use]
    pub fn correct(&self, model: &str, category: &str, raw_value: f64) -> f64 {
        raw_value - self.bias(model, category)
    }

    /// Apply scalar correction to a prediction when its value is numeric.
    pub fn correct_prediction(&self, prediction: &mut Prediction, category: &str) {
        let bias = self.bias(&prediction.provenance.model_id, category);
        if let PredictedValue::Numeric(value) | PredictedValue::Probability(value) =
            &mut prediction.value
        {
            *value -= bias;
        }
    }

    /// Update the bias estimate for a model/category pair.
    pub fn update(&self, model: &str, category: &str, residual: f64) {
        self.biases
            .write()
            .entry((model.to_string(), category.to_string()))
            .or_insert_with(|| ExponentialMovingAverage::new(self.alpha))
            .update(residual);
    }

    /// Current estimated bias for a model/category pair.
    #[must_use]
    pub fn bias(&self, model: &str, category: &str) -> f64 {
        self.biases
            .read()
            .get(&(model.to_string(), category.to_string()))
            .map_or(0.0, ExponentialMovingAverage::current)
    }
}

/// Per-model/category calibration tracker.
#[derive(Debug)]
pub struct CalibrationTracker {
    stats: RwLock<HashMap<(String, String), CalibrationStats>>,
    alpha: f64,
}

impl Default for CalibrationTracker {
    fn default() -> Self {
        Self::new(0.1)
    }
}

impl CalibrationTracker {
    /// Create a calibration tracker with the given EMA smoothing factor.
    #[must_use]
    pub fn new(alpha: f64) -> Self {
        Self {
            stats: RwLock::new(HashMap::new()),
            alpha: alpha.clamp(0.0, 1.0),
        }
    }

    /// Update calibration for a model/category pair.
    pub fn update(&self, model: &str, category: &str, accuracy: &PredictionAccuracy) {
        self.stats
            .write()
            .entry((model.to_string(), category.to_string()))
            .or_insert_with(|| CalibrationStats::new(self.alpha))
            .update(accuracy);
    }

    /// Update calibration using the domain as the model key.
    pub fn update_accuracy(&self, accuracy: &PredictionAccuracy) {
        self.update(&accuracy.domain.to_string(), &accuracy.category, accuracy);
    }

    /// Return a copy of calibration stats for a model/category pair.
    #[must_use]
    pub fn stats(&self, model: &str, category: &str) -> Option<CalibrationStats> {
        self.stats
            .read()
            .get(&(model.to_string(), category.to_string()))
            .copied()
    }

    /// Confidence adjusted by learned mean absolute error.
    #[must_use]
    pub fn calibrated_confidence(&self, model: &str, category: &str) -> f64 {
        self.stats(model, category)
            .map_or(0.5, |stats| 1.0 - stats.mean_absolute_error.current())
            .clamp(0.0, 1.0)
    }

    /// Recent accuracy trend for a model/category pair.
    #[must_use]
    pub fn accuracy_trend(&self, model: &str, category: &str) -> f64 {
        self.stats(model, category).map_or(0.0, |stats| stats.trend)
    }

    /// Build a core calibration summary for scorer/policy integration.
    #[must_use]
    pub fn summary(&self, model: &str, category: &str) -> PredictionCalibrationSummary {
        let Some(stats) = self.stats(model, category) else {
            return PredictionCalibrationSummary::cold_start();
        };
        let recent_accuracy = (1.0 - stats.mean_absolute_error.current()).clamp(0.0, 1.0);
        PredictionCalibrationSummary {
            recent_accuracy,
            coverage: stats.interval_coverage.current().clamp(0.0, 1.0),
            mean_bias: stats.mean_residual.current(),
            accuracy_trend: stats.trend,
            sample_count: stats.count as usize,
            confidence: (stats.count as f64 / 200.0).min(1.0),
        }
    }
}

impl PredictionCalibrationSource for CalibrationTracker {
    fn summary(&self, model: &str, task_category: &str) -> PredictionCalibrationSummary {
        CalibrationTracker::summary(self, model, task_category)
    }
}

/// Aggregated calibration state for one model/category pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalibrationStats {
    /// EMA of signed residuals.
    pub mean_residual: ExponentialMovingAverage,
    /// EMA of absolute residuals.
    pub mean_absolute_error: ExponentialMovingAverage,
    /// EMA of interval hits.
    pub interval_coverage: ExponentialMovingAverage,
    /// Number of resolved predictions.
    pub count: u64,
    /// Recent scalar accuracy trend.
    pub trend: f64,
    last_accuracy: Option<f64>,
}

impl CalibrationStats {
    /// Create empty calibration stats.
    #[must_use]
    pub fn new(alpha: f64) -> Self {
        Self {
            mean_residual: ExponentialMovingAverage::new(alpha),
            mean_absolute_error: ExponentialMovingAverage::new(alpha),
            interval_coverage: ExponentialMovingAverage::new(alpha),
            count: 0,
            trend: 0.0,
            last_accuracy: None,
        }
    }

    /// Update stats from a resolved prediction accuracy record.
    pub fn update(&mut self, accuracy: &PredictionAccuracy) {
        self.count = self.count.saturating_add(1);
        self.mean_residual.update(accuracy.residual);
        self.mean_absolute_error.update(accuracy.residual.abs());
        if let Some(hit) = accuracy.interval_hit {
            self.interval_coverage.update(f64::from(hit));
        }
        if let Some(last) = self.last_accuracy {
            self.trend = accuracy.accuracy - last;
        }
        self.last_accuracy = Some(accuracy.accuracy);
    }
}

/// Calibration summary for one `(model, task_category)` pair.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PredictionCalibrationSummary {
    /// Recent empirical accuracy in `[0, 1]`.
    pub recent_accuracy: f64,
    /// Coverage / interval hit rate in `[0, 1]`.
    pub coverage: f64,
    /// Signed mean residual (`predicted - actual`).
    pub mean_bias: f64,
    /// Short-horizon accuracy trend. Negative means degradation.
    pub accuracy_trend: f64,
    /// Number of observations behind the estimate.
    pub sample_count: usize,
    /// Confidence in the estimate, typically `min(sample_count / 200, 1)`.
    pub confidence: f64,
}

impl PredictionCalibrationSummary {
    /// Conservative default when no calibration history exists yet.
    #[must_use]
    pub fn cold_start() -> Self {
        Self {
            recent_accuracy: 0.5,
            coverage: 0.5,
            mean_bias: 0.0,
            accuracy_trend: 0.0,
            sample_count: 0,
            confidence: 0.0,
        }
    }

    #[must_use]
    fn coherence(self) -> f32 {
        (1.0 - self.mean_bias.abs().min(1.0)) as f32
    }
}

/// Read-only calibration source used by predictive scoring / policies.
pub trait PredictionCalibrationSource: Send + Sync {
    /// Return the current calibration summary for `model` in `task_category`.
    fn summary(&self, model: &str, task_category: &str) -> PredictionCalibrationSummary;
}

/// Calibration-aware scorer approximating expected free energy for Engrams.
pub struct PredictiveScorer {
    calibration: Arc<dyn PredictionCalibrationSource>,
    pragmatic_weight: f32,
    token_cost_per_1k: f32,
}

impl PredictiveScorer {
    /// Construct a predictive scorer with the default PRD constants.
    #[must_use]
    pub fn new(calibration: Arc<dyn PredictionCalibrationSource>) -> Self {
        Self {
            calibration,
            pragmatic_weight: 1.0,
            token_cost_per_1k: 0.01,
        }
    }

    /// Override the pragmatic-value weight.
    #[must_use]
    pub const fn with_pragmatic_weight(mut self, pragmatic_weight: f32) -> Self {
        self.pragmatic_weight = pragmatic_weight;
        self
    }

    /// Override the per-1k-token cost coefficient.
    #[must_use]
    pub const fn with_token_cost_per_1k(mut self, token_cost_per_1k: f32) -> Self {
        self.token_cost_per_1k = token_cost_per_1k;
        self
    }

    #[must_use]
    fn pragmatic_value(&self, signal: &Engram, ctx: &Context) -> f32 {
        let base = signal.score.utility.max(0.0);
        let goal_overlap = ctx
            .goal
            .as_deref()
            .map(|goal| overlap_ratio(goal, &body_text(signal)))
            .unwrap_or(0.0);
        let task_overlap = ctx
            .attr("roko.task_text")
            .map(|task| overlap_ratio(task, &body_text(signal)))
            .unwrap_or(0.0);
        (base + goal_overlap.max(task_overlap)).max(0.0)
    }

    #[must_use]
    fn epistemic_value(
        &self,
        signal: &Engram,
        summary: PredictionCalibrationSummary,
        body: &str,
    ) -> f32 {
        let uncertainty = (1.0 - summary.confidence).clamp(0.0, 1.0) as f32;
        let low_accuracy = (1.0 - summary.recent_accuracy).clamp(0.0, 1.0) as f32;
        let warningish = keyword_weight(
            body,
            &[
                "warning",
                "risk",
                "uncertain",
                "verify",
                "counterexample",
                "prediction",
                "error",
                "failure",
                "fallback",
            ],
        );
        (signal.score.novelty.max(0.0)
            + warningish * (0.45 + 0.35 * uncertainty)
            + low_accuracy * 0.20)
            .clamp(0.0, 1.0)
    }

    #[must_use]
    fn token_cost_penalty(&self, signal: &Engram) -> f32 {
        let tokens = Budget::estimate_tokens(signal.body.byte_size()) as f32;
        (tokens / 1000.0) * self.token_cost_per_1k
    }
}

impl Scorer for PredictiveScorer {
    fn score(&self, signal: &Engram, ctx: &Context) -> Score {
        let model = signal
            .tag("model_slug")
            .or_else(|| signal.tag("model"))
            .or_else(|| ctx.attr("roko.model_slug"))
            .unwrap_or(signal.provenance.author.as_str());
        let task_category = signal
            .tag("task_category")
            .or_else(|| ctx.attr("roko.task_category"))
            .unwrap_or("unknown");
        let summary = self.calibration.summary(model, task_category);
        let body = body_text(signal);
        let pragmatic = self.pragmatic_value(signal, ctx);
        let epistemic = self.epistemic_value(signal, summary, &body);
        let salience = (pragmatic * self.pragmatic_weight + epistemic
            - self.token_cost_penalty(signal))
        .clamp(0.0, 1.0);
        let calibration_confidence = if summary.sample_count == 0 {
            1.0
        } else {
            summary.recent_accuracy.clamp(0.0, 1.0) as f32
        };

        Score::new_extended(
            (signal.score.confidence * calibration_confidence).clamp(0.0, 1.0),
            signal.score.novelty.max(epistemic).clamp(0.0, 1.0),
            (signal.score.utility + pragmatic).max(0.0),
            signal.score.reputation,
            summary.coverage.clamp(0.0, 1.0) as f32,
            salience,
            signal.score.coherence.max(summary.coherence()),
        )
    }

    fn name(&self) -> &'static str {
        "predictive_scorer"
    }
}

/// Policy that emits calibration warnings / regime-shift insights.
pub struct PredictionPolicy {
    calibration: Arc<dyn PredictionCalibrationSource>,
    min_samples: usize,
    bias_threshold: f64,
    degradation_threshold: f64,
}

impl PredictionPolicy {
    /// Construct a prediction policy with conservative defaults.
    #[must_use]
    pub fn new(calibration: Arc<dyn PredictionCalibrationSource>) -> Self {
        Self {
            calibration,
            min_samples: 8,
            bias_threshold: 0.15,
            degradation_threshold: 0.05,
        }
    }

    /// Require at least this many samples before emitting interventions.
    #[must_use]
    pub const fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.min_samples = min_samples;
        self
    }

    /// Override the systematic-bias alert threshold.
    #[must_use]
    pub const fn with_bias_threshold(mut self, bias_threshold: f64) -> Self {
        self.bias_threshold = bias_threshold;
        self
    }

    /// Override the degradation alert threshold.
    #[must_use]
    pub const fn with_degradation_threshold(mut self, degradation_threshold: f64) -> Self {
        self.degradation_threshold = degradation_threshold;
        self
    }
}

impl Policy for PredictionPolicy {
    fn decide(&self, stream: &[Engram], ctx: &Context) -> Vec<Engram> {
        let mut seen = BTreeSet::new();
        let mut outputs = Vec::new();

        for signal in stream {
            let model = signal
                .tag("model_slug")
                .or_else(|| signal.tag("model"))
                .or_else(|| ctx.attr("roko.model_slug"))
                .unwrap_or(signal.provenance.author.as_str());
            let category = signal
                .tag("task_category")
                .or_else(|| ctx.attr("roko.task_category"))
                .unwrap_or("unknown");
            if !seen.insert((model.to_string(), category.to_string())) {
                continue;
            }

            let summary = self.calibration.summary(model, category);
            if summary.sample_count < self.min_samples {
                continue;
            }

            if summary.mean_bias.abs() >= self.bias_threshold {
                outputs.push(
                    Engram::builder(Kind::Insight)
                        .body(crate::Body::text(format!(
                            "Prediction calibration drift for {model}/{category}: mean bias {:+.2} over {} runs",
                            summary.mean_bias, summary.sample_count
                        )))
                        .provenance(Provenance::trusted("prediction_policy"))
                        .score(Score::new_extended(
                            0.8,
                            0.3,
                            0.4,
                            1.0,
                            summary.coverage.clamp(0.0, 1.0) as f32,
                            0.75,
                            summary.coherence(),
                        ))
                        .tag("model_slug", model)
                        .tag("task_category", category)
                        .tag("policy", "prediction")
                        .tag("alert_kind", "systematic_bias")
                        .build(),
                );
            }

            if summary.accuracy_trend <= -self.degradation_threshold {
                outputs.push(
                    Engram::builder(Kind::Prediction)
                        .body(crate::Body::text(format!(
                            "Prediction accuracy is degrading for {model}/{category}: trend {:+.2}",
                            summary.accuracy_trend
                        )))
                        .provenance(Provenance::trusted("prediction_policy"))
                        .score(Score::new_extended(
                            0.7,
                            0.5,
                            0.3,
                            1.0,
                            summary.coverage.clamp(0.0, 1.0) as f32,
                            0.82,
                            summary.coherence(),
                        ))
                        .tag("model_slug", model)
                        .tag("task_category", category)
                        .tag("policy", "prediction")
                        .tag("alert_kind", "degrading_accuracy")
                        .build(),
                );
            }
        }

        outputs
    }

    fn name(&self) -> &'static str {
        "prediction_policy"
    }
}

fn body_text(signal: &Engram) -> String {
    match &signal.body {
        crate::Body::Empty => String::new(),
        crate::Body::Text(text) => text.clone(),
        crate::Body::Json(value) => value.to_string(),
        crate::Body::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn overlap_ratio(left: &str, right: &str) -> f32 {
    let left_terms = tokenize(left);
    let right_terms = tokenize(right);
    if left_terms.is_empty() || right_terms.is_empty() {
        return 0.0;
    }
    let overlap = left_terms.intersection(&right_terms).count() as f32;
    overlap / left_terms.len().max(right_terms.len()) as f32
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let normalized = token.trim().to_ascii_lowercase();
            (!normalized.is_empty()).then_some(normalized)
        })
        .collect()
}

fn keyword_weight(text: &str, keywords: &[&str]) -> f32 {
    let lower = text.to_ascii_lowercase();
    keywords
        .iter()
        .any(|keyword| lower.contains(keyword))
        .then_some(1.0)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeCalibration {
        summaries: HashMap<(String, String), PredictionCalibrationSummary>,
    }

    impl FakeCalibration {
        fn with_summary(
            mut self,
            model: &str,
            category: &str,
            summary: PredictionCalibrationSummary,
        ) -> Self {
            self.summaries
                .insert((model.to_string(), category.to_string()), summary);
            self
        }
    }

    impl PredictionCalibrationSource for FakeCalibration {
        fn summary(&self, model: &str, task_category: &str) -> PredictionCalibrationSummary {
            self.summaries
                .get(&(model.to_string(), task_category.to_string()))
                .copied()
                .unwrap_or_else(PredictionCalibrationSummary::cold_start)
        }
    }

    #[test]
    fn predictive_scorer_boosts_warning_sections_when_calibration_is_uncertain() {
        let calibration = Arc::new(FakeCalibration::default().with_summary(
            "claude-sonnet-4-5",
            "implementation",
            PredictionCalibrationSummary {
                recent_accuracy: 0.45,
                coverage: 0.60,
                mean_bias: 0.10,
                accuracy_trend: -0.08,
                sample_count: 32,
                confidence: 0.15,
            },
        ));
        let scorer = PredictiveScorer::new(calibration);
        let signal = Engram::builder(Kind::PromptSection)
            .body(crate::Body::text(
                "Warning: verify assumptions and check likely failure modes",
            ))
            .score(Score::new(0.8, 0.2, 0.2, 1.0))
            .tag("model_slug", "claude-sonnet-4-5")
            .tag("task_category", "implementation")
            .build();
        let ctx = Context::at(0).with_goal("fix compiler failure safely");

        let score = scorer.score(&signal, &ctx);

        assert!(score.salience > 0.5);
        assert!(score.novelty >= 0.2);
        assert!(score.precision > 0.5);
    }

    #[test]
    fn prediction_policy_emits_bias_and_degradation_alerts() {
        let calibration = Arc::new(FakeCalibration::default().with_summary(
            "gpt-5",
            "implementation",
            PredictionCalibrationSummary {
                recent_accuracy: 0.55,
                coverage: 0.70,
                mean_bias: 0.22,
                accuracy_trend: -0.07,
                sample_count: 18,
                confidence: 0.8,
            },
        ));
        let policy = PredictionPolicy::new(calibration);
        let stream = vec![
            Engram::builder(Kind::Prediction)
                .body(crate::Body::text("route coding task"))
                .tag("model_slug", "gpt-5")
                .tag("task_category", "implementation")
                .build(),
        ];

        let outputs = policy.decide(&stream, &Context::at(0));

        assert_eq!(outputs.len(), 2);
        assert!(
            outputs
                .iter()
                .any(|engram| engram.tag("alert_kind") == Some("systematic_bias"))
        );
        assert!(
            outputs
                .iter()
                .any(|engram| engram.tag("alert_kind") == Some("degrading_accuracy"))
        );
    }
}
