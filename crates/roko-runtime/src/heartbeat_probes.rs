//! Zero-LLM heartbeat probe registry and default probe set.
//!
//! Probes are deterministic, side-effect-light checks that feed prediction
//! error and tier selection without invoking a model.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_const_for_fn,
    clippy::return_self_not_must_use,
    clippy::unnecessary_literal_bound
)]

use std::collections::HashMap;

/// Identifier for a tracked asset in the chain probe set.
pub type AssetId = String;

/// A zero-cost cognitive probe.
///
/// Heartbeat probes are deterministic, side-effect-free checks that evaluate a
/// single dimension of the current engine state and contribute to the aggregate
/// prediction error.
///
/// This is distinct from [`roko_core::obs::health::Probe`], which is a
/// readiness/liveness health check returning pass/fail. `HeartbeatProbe`
/// evaluates a continuous signal (0.0..=1.0) for prediction-error tracking and
/// tier selection.
pub trait HeartbeatProbe: Send + Sync {
    /// Evaluate this probe against the current engine state.
    fn evaluate(&self, state: &EngineState) -> f32;

    /// The relative contribution of this probe to the aggregate prediction error.
    fn weight(&self) -> f32;

    /// Human-readable probe identifier.
    fn name(&self) -> &str;

    /// The probe's domain.
    fn domain(&self) -> ProbeDomain;
}

/// Domain classification for a probe.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProbeDomain {
    /// Blockchain and trading signals.
    Chain,
    /// Coding and repository signals.
    Coding,
    /// Research and knowledge-work signals.
    Research,
    /// Domain-independent signals.
    Universal,
    /// Custom domain provided by a plugin.
    Custom(String),
}

/// A tracked asset used by the chain probes.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackedAsset {
    /// Asset identifier.
    pub id: AssetId,
    /// Current observed price.
    pub current_price: f32,
    /// Price recorded on the previous tick.
    pub last_tick_price: f32,
}

impl TrackedAsset {
    /// Create a tracked asset snapshot.
    pub fn new(id: impl Into<AssetId>, current_price: f32, last_tick_price: f32) -> Self {
        Self {
            id: id.into(),
            current_price,
            last_tick_price,
        }
    }
}

/// A position snapshot used by the position-health probe.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    /// Health factor where 1.0 is safe and 0.0 is liquidatable.
    pub health_factor: f32,
}

impl Position {
    /// Create a position snapshot.
    pub const fn new(health_factor: f32) -> Self {
        Self { health_factor }
    }
}

/// The current MACD snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacdSnapshot {
    /// Current MACD value.
    pub value: f32,
    /// Current divergence value.
    pub divergence: f32,
    /// Baseline divergence used for comparison.
    pub baseline_divergence: f32,
    /// Whether the MACD just crossed on this tick.
    pub just_crossed: bool,
}

impl MacdSnapshot {
    /// Create a MACD snapshot.
    pub const fn new(
        value: f32,
        divergence: f32,
        baseline_divergence: f32,
        just_crossed: bool,
    ) -> Self {
        Self {
            value,
            divergence,
            baseline_divergence,
            just_crossed,
        }
    }
}

/// Build status used by the build-health probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildResult {
    /// The last build succeeded cleanly.
    Success,
    /// The last build succeeded with warnings.
    Warning(u32),
    /// The last build failed.
    Failure,
    /// No recent build information is available.
    Unknown,
}

/// Engine metrics consumed by the heartbeat probes.
///
/// This is a lightweight metric holder rather than a full world model. It
/// keeps the probe module dependency-light and lets callers populate only the
/// signals they actually track.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineState {
    tracked_assets: Vec<TrackedAsset>,
    tvl_delta_percent: f32,
    positions: Vec<Position>,
    gas_price_gwei: f32,
    gas_ema_gwei: f32,
    korai_balance: f32,
    daily_burn_rate: f32,
    rsi_14: f32,
    macd: MacdSnapshot,
    any_circuit_breaker_active: bool,
    last_build_result: BuildResult,
    test_pass_count_delta: i32,
    complexity_delta_percent: f32,
    new_vulnerability_count: u32,
    coverage_delta_percent: f32,
    gate_failure_rate_last_n: f32,
    predicted_state_vector: Vec<f32>,
    actual_state_vector: Vec<f32>,
    lineage_dag_issues: u32,
    custom_metrics: HashMap<String, f32>,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            tracked_assets: Vec::new(),
            tvl_delta_percent: 0.0,
            positions: Vec::new(),
            gas_price_gwei: 0.0,
            gas_ema_gwei: 0.0,
            korai_balance: 0.0,
            daily_burn_rate: 0.0,
            rsi_14: 50.0,
            macd: MacdSnapshot::new(0.0, 0.0, 0.0, false),
            any_circuit_breaker_active: false,
            last_build_result: BuildResult::Unknown,
            test_pass_count_delta: 0,
            complexity_delta_percent: 0.0,
            new_vulnerability_count: 0,
            coverage_delta_percent: 0.0,
            gate_failure_rate_last_n: 0.0,
            predicted_state_vector: Vec::new(),
            actual_state_vector: Vec::new(),
            lineage_dag_issues: 0,
            custom_metrics: HashMap::new(),
        }
    }
}

impl EngineState {
    /// Create an empty engine state snapshot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Tracked assets for the price-delta probe.
    pub fn tracked_assets(&self) -> &[TrackedAsset] {
        &self.tracked_assets
    }

    /// Replace the tracked assets snapshot.
    pub fn with_tracked_assets(mut self, tracked_assets: Vec<TrackedAsset>) -> Self {
        self.tracked_assets = tracked_assets;
        self
    }

    /// TVL delta percentage.
    pub const fn tvl_delta_percent(&self) -> f32 {
        self.tvl_delta_percent
    }

    /// Set the TVL delta percentage.
    pub fn with_tvl_delta_percent(mut self, tvl_delta_percent: f32) -> Self {
        self.tvl_delta_percent = tvl_delta_percent;
        self
    }

    /// Active positions.
    pub fn positions(&self) -> &[Position] {
        &self.positions
    }

    /// Replace the position snapshot.
    pub fn with_positions(mut self, positions: Vec<Position>) -> Self {
        self.positions = positions;
        self
    }

    /// Current gas price in gwei.
    pub const fn gas_price_gwei(&self) -> f32 {
        self.gas_price_gwei
    }

    /// Set the current gas price in gwei.
    pub fn with_gas_price_gwei(mut self, gas_price_gwei: f32) -> Self {
        self.gas_price_gwei = gas_price_gwei;
        self
    }

    /// Exponential moving average of gas price in gwei.
    pub const fn gas_ema_gwei(&self) -> f32 {
        self.gas_ema_gwei
    }

    /// Set the gas price EMA in gwei.
    pub fn with_gas_ema_gwei(mut self, gas_ema_gwei: f32) -> Self {
        self.gas_ema_gwei = gas_ema_gwei;
        self
    }

    /// Current KORAI balance.
    pub const fn korai_balance(&self) -> f32 {
        self.korai_balance
    }

    /// Set the KORAI balance.
    pub fn with_korai_balance(mut self, korai_balance: f32) -> Self {
        self.korai_balance = korai_balance;
        self
    }

    /// Daily burn rate for the credit balance probe.
    pub const fn daily_burn_rate(&self) -> f32 {
        self.daily_burn_rate
    }

    /// Set the daily burn rate.
    pub fn with_daily_burn_rate(mut self, daily_burn_rate: f32) -> Self {
        self.daily_burn_rate = daily_burn_rate;
        self
    }

    /// 14-period RSI.
    pub const fn rsi_14(&self) -> f32 {
        self.rsi_14
    }

    /// Set the 14-period RSI.
    pub fn with_rsi_14(mut self, rsi_14: f32) -> Self {
        self.rsi_14 = rsi_14;
        self
    }

    /// MACD snapshot.
    pub const fn macd(&self) -> MacdSnapshot {
        self.macd
    }

    /// Set the MACD snapshot.
    pub fn with_macd(mut self, macd: MacdSnapshot) -> Self {
        self.macd = macd;
        self
    }

    /// Whether any circuit breaker is currently active.
    pub const fn any_circuit_breaker_active(&self) -> bool {
        self.any_circuit_breaker_active
    }

    /// Set the circuit breaker state.
    pub fn with_any_circuit_breaker_active(mut self, active: bool) -> Self {
        self.any_circuit_breaker_active = active;
        self
    }

    /// Last build result.
    pub const fn last_build_result(&self) -> BuildResult {
        self.last_build_result
    }

    /// Set the last build result.
    pub fn with_last_build_result(mut self, result: BuildResult) -> Self {
        self.last_build_result = result;
        self
    }

    /// Change in passing test count since the last run.
    pub const fn test_pass_count_delta(&self) -> i32 {
        self.test_pass_count_delta
    }

    /// Set the test count delta.
    pub fn with_test_pass_count_delta(mut self, delta: i32) -> Self {
        self.test_pass_count_delta = delta;
        self
    }

    /// Complexity change percentage.
    pub const fn complexity_delta_percent(&self) -> f32 {
        self.complexity_delta_percent
    }

    /// Set the complexity delta percentage.
    pub fn with_complexity_delta_percent(mut self, complexity_delta_percent: f32) -> Self {
        self.complexity_delta_percent = complexity_delta_percent;
        self
    }

    /// Number of newly observed vulnerabilities.
    pub const fn new_vulnerability_count(&self) -> u32 {
        self.new_vulnerability_count
    }

    /// Set the vulnerability count.
    pub fn with_new_vulnerability_count(mut self, new_vulnerability_count: u32) -> Self {
        self.new_vulnerability_count = new_vulnerability_count;
        self
    }

    /// Coverage change percentage.
    pub const fn coverage_delta_percent(&self) -> f32 {
        self.coverage_delta_percent
    }

    /// Set the coverage delta percentage.
    pub fn with_coverage_delta_percent(mut self, coverage_delta_percent: f32) -> Self {
        self.coverage_delta_percent = coverage_delta_percent;
        self
    }

    /// Failure rate over the last N tasks.
    pub const fn gate_failure_rate_last_n(&self, _n: usize) -> f32 {
        self.gate_failure_rate_last_n
    }

    /// Set the gate failure rate snapshot.
    pub fn with_gate_failure_rate_last_n(mut self, gate_failure_rate_last_n: f32) -> Self {
        self.gate_failure_rate_last_n = gate_failure_rate_last_n;
        self
    }

    /// Predicted state vector.
    pub fn predicted_state_vector(&self) -> &[f32] {
        &self.predicted_state_vector
    }

    /// Set the predicted state vector.
    pub fn with_predicted_state_vector(mut self, predicted_state_vector: Vec<f32>) -> Self {
        self.predicted_state_vector = predicted_state_vector;
        self
    }

    /// Actual observed state vector.
    pub fn actual_state_vector(&self) -> &[f32] {
        &self.actual_state_vector
    }

    /// Set the actual state vector.
    pub fn with_actual_state_vector(mut self, actual_state_vector: Vec<f32>) -> Self {
        self.actual_state_vector = actual_state_vector;
        self
    }

    /// Number of lineage DAG issues.
    pub const fn lineage_dag_issues(&self) -> u32 {
        self.lineage_dag_issues
    }

    /// Set the lineage DAG issue count.
    pub fn with_lineage_dag_issues(mut self, lineage_dag_issues: u32) -> Self {
        self.lineage_dag_issues = lineage_dag_issues;
        self
    }

    /// Read a custom metric by key.
    pub fn custom_metric(&self, key: &str) -> Option<f32> {
        self.custom_metrics.get(key).copied()
    }

    /// Set a custom metric.
    pub fn with_custom_metric(mut self, key: impl Into<String>, value: f32) -> Self {
        self.custom_metrics.insert(key.into(), value);
        self
    }
}

/// A single probe measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeResult {
    /// Probe identifier.
    pub probe_id: String,
    /// Raw probe output in the range `[0.0, 1.0]`.
    pub value: f32,
    /// Rolling mean used by z-score anomaly detection.
    pub rolling_mean: f32,
    /// Rolling standard deviation used by z-score anomaly detection.
    pub rolling_stddev: f32,
    /// Z-score threshold for anomaly detection.
    pub z_threshold: f32,
    /// Probe weight in the aggregate prediction error.
    pub weight: f32,
    /// Probe domain.
    pub domain: ProbeDomain,
}

impl ProbeResult {
    /// Create a probe result snapshot.
    pub fn new(
        probe_id: impl Into<String>,
        value: f32,
        rolling_mean: f32,
        rolling_stddev: f32,
        z_threshold: f32,
        weight: f32,
        domain: ProbeDomain,
    ) -> Self {
        Self {
            probe_id: probe_id.into(),
            value,
            rolling_mean,
            rolling_stddev,
            z_threshold,
            weight,
            domain,
        }
    }

    /// Whether this result is anomalous under the rolling z-score rule.
    pub fn is_anomalous(&self) -> bool {
        if self.rolling_stddev < f32::EPSILON {
            return false;
        }
        let z = (self.value - self.rolling_mean).abs() / self.rolling_stddev;
        z > self.z_threshold
    }
}

/// The outcome of running a probe registry against an engine state.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeResults {
    /// Per-probe outputs.
    pub results: Vec<ProbeResult>,
    /// Aggregate weighted prediction error capped at 1.0.
    pub aggregate: f32,
}

impl ProbeResults {
    /// Count the number of anomalous probe results.
    pub fn anomaly_count(&self) -> usize {
        self.results
            .iter()
            .filter(|result| result.is_anomalous())
            .count()
    }
}

/// The heartbeat probe registry: an ordered list of probes evaluated on each gamma tick.
#[derive(Default)]
pub struct HeartbeatProbeRegistry {
    probes: Vec<Box<dyn HeartbeatProbe>>,
}

impl HeartbeatProbeRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new probe.
    pub fn register(&mut self, probe: Box<dyn HeartbeatProbe>) {
        self.probes.push(probe);
    }

    /// Evaluate every registered probe and return the individual results and
    /// the aggregate prediction error.
    pub fn evaluate_all(&self, state: &EngineState) -> ProbeResults {
        let results: Vec<ProbeResult> = self
            .probes
            .iter()
            .map(|probe| {
                let value = probe.evaluate(state);
                ProbeResult::new(
                    probe.name(),
                    value,
                    value,
                    0.0,
                    2.0,
                    probe.weight(),
                    probe.domain(),
                )
            })
            .collect();

        let aggregate = results
            .iter()
            .map(|result| result.value * result.weight)
            .sum::<f32>()
            .min(1.0);

        ProbeResults { results, aggregate }
    }

    /// Number of registered probes.
    pub fn len(&self) -> usize {
        self.probes.len()
    }

    /// Whether the registry contains no probes.
    pub fn is_empty(&self) -> bool {
        self.probes.is_empty()
    }

    /// Evaluate only probes belonging to the given domain.
    pub fn evaluate_domain(&self, domain: &ProbeDomain, state: &EngineState) -> Vec<ProbeResult> {
        self.probes
            .iter()
            .filter(|probe| &probe.domain() == domain)
            .map(|probe| {
                let value = probe.evaluate(state);
                ProbeResult::new(
                    probe.name(),
                    value,
                    value,
                    0.0,
                    2.0,
                    probe.weight(),
                    probe.domain(),
                )
            })
            .collect()
    }

    /// Return the set of distinct domains represented by registered probes.
    pub fn domains(&self) -> Vec<ProbeDomain> {
        let mut seen = Vec::new();
        for probe in &self.probes {
            let domain = probe.domain();
            if !seen.contains(&domain) {
                seen.push(domain);
            }
        }
        seen
    }

    /// Alias for `len()` — number of registered probes.
    pub fn probe_count(&self) -> usize {
        self.probes.len()
    }

    /// Create a registry pre-populated with all 16 default probes.
    ///
    /// Registers 8 chain probes, 6 coding probes, and 2 universal probes.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        // Chain probes (8)
        registry.register(Box::new(PriceDeltaProbe::new()));
        registry.register(Box::new(TvlDeltaProbe::new()));
        registry.register(Box::new(PositionHealthProbe::new()));
        registry.register(Box::new(GasSpikeProbe::new()));
        registry.register(Box::new(CreditBalanceProbe::new()));
        registry.register(Box::new(RsiProbe::new()));
        registry.register(Box::new(MacdProbe::new()));
        registry.register(Box::new(CircuitBreakerProbe::new()));
        // Coding probes (6)
        registry.register(Box::new(BuildHealthProbe::new()));
        registry.register(Box::new(TestRegressionProbe::new()));
        registry.register(Box::new(ComplexityDriftProbe::new()));
        registry.register(Box::new(DependencyRiskProbe::new()));
        registry.register(Box::new(CoverageDeltaProbe::new()));
        registry.register(Box::new(ErrorRateProbe::new()));
        // Universal probes (2)
        registry.register(Box::new(WorldModelDriftProbe::new()));
        registry.register(Box::new(CausalConsistencyProbe::new()));
        registry
    }
}

/// Detects significant price changes since the last tick.
#[derive(Debug, Clone, Default)]
pub struct PriceDeltaProbe {
    /// Per-asset volatility-normalized thresholds.
    pub thresholds: HashMap<AssetId, f32>,
}

impl PriceDeltaProbe {
    /// Create a new price delta probe.
    pub fn new() -> Self {
        Self::default()
    }
}

impl HeartbeatProbe for PriceDeltaProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        state
            .tracked_assets()
            .iter()
            .map(|asset| {
                let baseline = asset.last_tick_price.abs().max(1.0);
                let delta = (asset.current_price - asset.last_tick_price).abs() / baseline;
                let threshold = self.thresholds.get(&asset.id).copied().unwrap_or(0.02);
                (delta / threshold).min(1.0)
            })
            .fold(0.0, f32::max)
    }

    fn weight(&self) -> f32 {
        0.15
    }

    fn name(&self) -> &str {
        "price_delta"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Chain
    }
}

/// Detects changes in total value locked across tracked protocols.
#[derive(Debug, Clone, Copy, Default)]
pub struct TvlDeltaProbe;

impl TvlDeltaProbe {
    /// Create a new TVL delta probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for TvlDeltaProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        (state.tvl_delta_percent().abs() / 0.05).min(1.0)
    }

    fn weight(&self) -> f32 {
        0.10
    }

    fn name(&self) -> &str {
        "tvl_delta"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Chain
    }
}

/// Monitors collateral ratios and liquidation distance for active positions.
#[derive(Debug, Clone, Copy, Default)]
pub struct PositionHealthProbe;

impl PositionHealthProbe {
    /// Create a new position-health probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for PositionHealthProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        state
            .positions()
            .iter()
            .map(|position| {
                let health = position.health_factor;
                if health < 1.2 {
                    1.0
                } else if health < 1.5 {
                    0.6
                } else if health < 2.0 {
                    0.2
                } else {
                    0.0
                }
            })
            .fold(0.0, f32::max)
    }

    fn weight(&self) -> f32 {
        0.20
    }

    fn name(&self) -> &str {
        "position_health"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Chain
    }
}

/// Detects sudden gas price increases.
#[derive(Debug, Clone, Copy, Default)]
pub struct GasSpikeProbe;

impl GasSpikeProbe {
    /// Create a new gas-spike probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for GasSpikeProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let baseline = state.gas_ema_gwei().max(1.0);
        let ratio = state.gas_price_gwei() / baseline;
        ((ratio - 1.0) / 2.0).clamp(0.0, 1.0)
    }

    fn weight(&self) -> f32 {
        0.05
    }

    fn name(&self) -> &str {
        "gas_spike"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Chain
    }
}

/// Monitors the remaining KORAI balance.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreditBalanceProbe;

impl CreditBalanceProbe {
    /// Create a new credit-balance probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for CreditBalanceProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let balance = state.korai_balance();
        let daily_burn = state.daily_burn_rate().max(0.01);
        let days_remaining = balance / daily_burn;
        if days_remaining < 1.0 {
            1.0
        } else if days_remaining < 7.0 {
            0.5
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 {
        0.05
    }

    fn name(&self) -> &str {
        "credit_balance"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Chain
    }
}

/// Relative strength index probe.
#[derive(Debug, Clone, Copy, Default)]
pub struct RsiProbe;

impl RsiProbe {
    /// Create a new RSI probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for RsiProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let rsi = state.rsi_14();
        if !(20.0..=80.0).contains(&rsi) {
            0.8
        } else if !(30.0..=70.0).contains(&rsi) {
            0.4
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 {
        0.05
    }

    fn name(&self) -> &str {
        "rsi"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Chain
    }
}

/// Detects momentum shifts via MACD crossovers and divergences.
#[derive(Debug, Clone, Copy, Default)]
pub struct MacdProbe;

impl MacdProbe {
    /// Create a new MACD probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for MacdProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let macd = state.macd();
        if macd.just_crossed {
            0.7
        } else if macd.divergence.abs() > macd.baseline_divergence * 2.0 {
            0.4
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 {
        0.05
    }

    fn name(&self) -> &str {
        "macd"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Chain
    }
}

/// Detects exchange halts, protocol pauses, or emergency shutdowns.
#[derive(Debug, Clone, Copy, Default)]
pub struct CircuitBreakerProbe;

impl CircuitBreakerProbe {
    /// Create a new circuit-breaker probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for CircuitBreakerProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        if state.any_circuit_breaker_active() {
            1.0
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 {
        0.10
    }

    fn name(&self) -> &str {
        "circuit_breaker"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Chain
    }
}

/// Monitors the last compilation result and trend.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuildHealthProbe;

impl BuildHealthProbe {
    /// Create a new build-health probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for BuildHealthProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        match state.last_build_result() {
            BuildResult::Success => 0.0,
            BuildResult::Warning(count) => (count as f32 * 0.1).min(0.5),
            BuildResult::Failure => 0.8,
            BuildResult::Unknown => 0.3,
        }
    }

    fn weight(&self) -> f32 {
        0.20
    }

    fn name(&self) -> &str {
        "build_health"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Coding
    }
}

/// Detects changes in test count since the last run.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestRegressionProbe;

impl TestRegressionProbe {
    /// Create a new test-regression probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for TestRegressionProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let delta = state.test_pass_count_delta();
        if delta < 0 {
            ((-delta) as f32 * 0.2).min(1.0)
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 {
        0.20
    }

    fn name(&self) -> &str {
        "test_regression"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Coding
    }
}

/// Monitors cyclomatic complexity moving average.
#[derive(Debug, Clone, Copy, Default)]
pub struct ComplexityDriftProbe;

impl ComplexityDriftProbe {
    /// Create a new complexity-drift probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for ComplexityDriftProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        (state.complexity_delta_percent() / 10.0).clamp(0.0, 1.0)
    }

    fn weight(&self) -> f32 {
        0.05
    }

    fn name(&self) -> &str {
        "complexity_drift"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Coding
    }
}

/// Monitors vulnerability scan results for dependency changes.
#[derive(Debug, Clone, Copy, Default)]
pub struct DependencyRiskProbe;

impl DependencyRiskProbe {
    /// Create a new dependency-risk probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for DependencyRiskProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        match state.new_vulnerability_count() {
            0 => 0.0,
            1..=2 => 0.4,
            3..=5 => 0.7,
            _ => 1.0,
        }
    }

    fn weight(&self) -> f32 {
        0.10
    }

    fn name(&self) -> &str {
        "dependency_risk"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Coding
    }
}

/// Monitors test coverage changes.
#[derive(Debug, Clone, Copy, Default)]
pub struct CoverageDeltaProbe;

impl CoverageDeltaProbe {
    /// Create a new coverage-delta probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for CoverageDeltaProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let delta = state.coverage_delta_percent();
        if delta < -2.0 {
            ((-delta) / 10.0).min(1.0)
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 {
        0.05
    }

    fn name(&self) -> &str {
        "coverage_delta"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Coding
    }
}

/// Monitors gate failure trend over the last N tasks.
#[derive(Debug, Clone, Copy, Default)]
pub struct ErrorRateProbe;

impl ErrorRateProbe {
    /// Create a new error-rate probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for ErrorRateProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let failure_rate = state.gate_failure_rate_last_n(10);
        if failure_rate > 0.5 {
            0.8
        } else if failure_rate > 0.3 {
            0.4
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 {
        0.10
    }

    fn name(&self) -> &str {
        "error_rate"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Coding
    }
}

/// Measures divergence between the predicted and observed state vectors.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorldModelDriftProbe;

impl WorldModelDriftProbe {
    /// Create a new world-model drift probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for WorldModelDriftProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        cosine_distance(state.predicted_state_vector(), state.actual_state_vector()).clamp(0.0, 1.0)
    }

    fn weight(&self) -> f32 {
        0.15
    }

    fn name(&self) -> &str {
        "world_model_drift"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Universal
    }
}

/// Checks the integrity of the lineage DAG.
#[derive(Debug, Clone, Copy, Default)]
pub struct CausalConsistencyProbe;

impl CausalConsistencyProbe {
    /// Create a new causal-consistency probe.
    pub const fn new() -> Self {
        Self
    }
}

impl HeartbeatProbe for CausalConsistencyProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        match state.lineage_dag_issues() {
            0 => 0.0,
            1..=2 => 0.3,
            _ => 0.8,
        }
    }

    fn weight(&self) -> f32 {
        0.10
    }

    fn name(&self) -> &str {
        "causal_consistency"
    }

    fn domain(&self) -> ProbeDomain {
        ProbeDomain::Universal
    }
}

fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 1.0;
    }

    let len = left.len().min(right.len());
    let (mut dot, mut left_norm, mut right_norm) = (0.0_f32, 0.0_f32, 0.0_f32);
    for idx in 0..len {
        let l = left[idx];
        let r = right[idx];
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }

    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        return 1.0;
    }

    let cosine_similarity = dot / (left_norm.sqrt() * right_norm.sqrt());
    (1.0 - cosine_similarity).clamp(0.0, 2.0)
}
