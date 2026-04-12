# Coding Oracles — TA Equivalents for Software Engineering

> Every financial TA primitive has a structural equivalent in software engineering. Build time trends are price trends. Test failure probability is risk assessment. Dependency vulnerability scoring is portfolio risk. The mathematics is identical; the vocabulary changes.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [01-oracle-trait](./01-oracle-trait.md) for Oracle trait, [02-chain-oracles](./02-chain-oracles.md) for chain-domain comparison
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §4, `refactoring-prd/09-innovations.md` §I (coding probes)

---

## CodingOracle — Implementation overview

The `CodingOracle` implements the universal Oracle trait for software engineering prediction. It wraps coding-specific indicators — build time, test failure probability, complexity drift, dependency risk, performance regression — into the same predict/evaluate interface used by chain oracles:

```rust
pub struct CodingOracle {
    /// Workspace analysis engine (uses roko-index for code intelligence).
    workspace: Arc<WorkspaceAnalyzer>,

    /// Historical build/test/complexity data cache.
    metrics_cache: Arc<CodingMetricsCache>,

    /// Dependency vulnerability scanner integration.
    vuln_scanner: Arc<VulnerabilityScanner>,

    /// Prediction persistence and tracking.
    prediction_store: Arc<PredictionStore>,

    /// Bias correction from collective calibration.
    corrector: Arc<ResidualCorrector>,

    /// Per-(model, category) accuracy tracking.
    calibration: Arc<CalibrationTracker>,
}

#[async_trait]
impl Oracle for CodingOracle {
    async fn predict(
        &self,
        query: &OracleQuery,
        ctx: &Context,
    ) -> Result<Prediction> {
        let coding_payload = query.payload.as_coding()?;

        match coding_payload.metric {
            CodingMetric::BuildTime => self.predict_build_time(coding_payload, ctx).await,
            CodingMetric::TestPassRate => self.predict_test_pass_rate(coding_payload, ctx).await,
            CodingMetric::ComplexityDelta => self.predict_complexity(coding_payload, ctx).await,
            CodingMetric::DependencyRisk => self.predict_dep_risk(coding_payload, ctx).await,
            CodingMetric::PerfRegression => self.predict_perf_regression(coding_payload, ctx).await,
            CodingMetric::CoverageImpact => self.predict_coverage(coding_payload, ctx).await,
        }
    }

    async fn evaluate(
        &self,
        prediction: &Prediction,
        outcome: &Engram,
    ) -> Result<PredictionAccuracy> {
        // Coding outcomes come from external verifiers:
        // compilers, test suites, benchmarks, coverage tools.
        let actual = self.extract_coding_outcome(outcome)?;
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

Coding oracles use three classes of external verifiers — all produce deterministic, reproducible outcomes:

| Verifier | What it produces | Prediction it resolves |
|---|---|---|
| **Compiler** (rustc, gcc, tsc) | Success/failure + error count + compile time | Build time, compilation success |
| **Test suite** (cargo test, pytest, jest) | Pass/fail per test, total pass rate | Test failure probability |
| **Linter/analyzer** (clippy, eslint, mypy) | Warning/error counts, complexity metrics | Complexity drift |
| **Benchmark** (criterion, hyperfine) | Throughput, latency distributions | Performance regression |
| **Coverage tool** (tarpaulin, llvm-cov) | Line/branch coverage percentage | Coverage impact |
| **Vulnerability scanner** (cargo audit, npm audit) | CVE counts, severity scores | Dependency risk |

These verifiers are the coding domain's equivalent of blockchain finality — they provide ground truth that the oracle's predictions are measured against.

---

## The structural analogy table

Each chain TA primitive has a coding equivalent. The math is the same; the domain vocabulary differs:

| Chain TA Primitive | Coding Equivalent | Shared Math |
|---|---|---|
| **Price prediction** (MA, Bollinger, regression) | **Build time prediction** (SMA of compile times, trend regression) | Time series forecasting |
| **Volatility estimation** (Garman-Klass, realized vol) | **Build time variance** (variance in compile times across runs) | Variance estimation |
| **RSI** (overbought/oversold momentum) | **Test pass rate momentum** (improving vs. degrading test suites) | Bounded oscillator |
| **MACD** (trend change detection) | **Complexity trend change** (accelerating vs. decelerating complexity growth) | Moving average crossover |
| **Gas price forecasting** (base fee + priority fee) | **CI pipeline time forecasting** (fixed overhead + variable test time) | Two-component prediction |
| **Liquidity depth analysis** | **Test coverage depth analysis** (which code paths are tested) | Distribution analysis |
| **MEV detection** (adversarial execution risk) | **Supply chain attack detection** (malicious dependencies) | Adversarial threat analysis |
| **TVL trends** (total value locked trajectory) | **Dependency count trends** (growing dependency graph) | Growth rate analysis |
| **Funding rate** (long/short sentiment) | **Error rate direction** (improving vs. degrading code health) | Directional bias indicator |
| **Liquidation proximity** | **Breakage proximity** (how close code is to failing) | Threshold distance metric |

---

## Coding-specific prediction targets

### Build time prediction

```rust
/// Predict compilation time for a given change set.
///
/// Uses historical compile time data + change scope analysis.
/// Features: number of files changed, number of crates affected,
/// dependency depth, incremental vs. full rebuild.
///
/// Analogous to price prediction: both are time series with
/// trend, seasonality, and external shocks.
pub struct BuildTimePredictor {
    /// Historical compile time observations.
    history: Vec<BuildTimeObservation>,

    /// Exponential moving average of recent compile times.
    ema: ExponentialMovingAverage,

    /// Per-crate compile time model.
    crate_models: HashMap<String, CrateCompileModel>,
}

pub struct BuildTimeObservation {
    pub timestamp_ms: i64,
    pub files_changed: usize,
    pub crates_affected: Vec<String>,
    pub incremental: bool,
    pub compile_time_ms: u64,
    pub success: bool,
}

impl BuildTimePredictor {
    /// Predict compile time given a change context.
    pub fn predict(&self, change: &ChangeContext) -> (f64, f64) {
        // Base: EMA of recent compile times
        let base = self.ema.current();

        // Adjust for change scope
        let scope_factor = self.scope_adjustment(change);

        // Adjust for affected crates (some crates are slower to compile)
        let crate_factor = self.crate_adjustment(&change.affected_crates);

        // Adjust for incremental vs. full
        let incr_factor = if change.incremental { 1.0 } else { 3.5 };

        let predicted = base * scope_factor * crate_factor * incr_factor;
        let confidence = self.confidence_from_history(change);

        (predicted, confidence)
    }
}
```

### Test failure probability

```rust
/// Predict which tests will fail given a change set.
///
/// Uses file-to-test mapping (from roko-index) + historical
/// failure rates per test. Tests that cover changed code are
/// more likely to fail. Tests with high historical flakiness
/// are discounted.
///
/// Analogous to risk assessment in finance: both estimate
/// the probability of an adverse event given current conditions.
pub struct TestFailurePredictor {
    /// File → test mapping (from symbol graph).
    file_test_map: Arc<FileTestMap>,

    /// Per-test historical failure rate.
    test_histories: HashMap<String, TestHistory>,

    /// Flakiness estimator: tests that fail randomly
    /// are weighted lower in the prediction.
    flakiness: HashMap<String, f64>,
}

pub struct TestHistory {
    /// Total runs.
    pub total_runs: u64,
    /// Failed runs.
    pub failures: u64,
    /// Recent failure rate (EMA with α=0.1).
    pub recent_rate: ExponentialMovingAverage,
    /// Last N results (ring buffer).
    pub recent_results: VecDeque<bool>,
}

impl TestFailurePredictor {
    /// Predict aggregate test pass rate for a change set.
    pub fn predict_pass_rate(&self, change: &ChangeContext) -> (f64, f64) {
        let affected_tests = self.file_test_map.tests_for_files(&change.files);

        if affected_tests.is_empty() {
            return (1.0, 0.9);  // no affected tests → high pass rate, high confidence
        }

        let mut expected_failures = 0.0;
        let total = affected_tests.len() as f64;

        for test in &affected_tests {
            if let Some(history) = self.test_histories.get(test) {
                let base_rate = history.recent_rate.current();
                let flakiness = self.flakiness.get(test).copied().unwrap_or(0.0);

                // Adjusted failure probability:
                // Higher if the test covers changed code AND has historical failures.
                // Discounted by flakiness (flaky tests are less informative).
                let adj_rate = base_rate * (1.0 - flakiness);
                expected_failures += adj_rate;
            } else {
                // Unknown test: conservative assumption (10% failure rate).
                expected_failures += 0.1;
            }
        }

        let predicted_pass_rate = 1.0 - (expected_failures / total);
        let confidence = self.confidence_from_sample_size(total as u64);

        (predicted_pass_rate, confidence)
    }
}
```

### Complexity drift detection

```rust
/// Track cyclomatic complexity trends at module/crate/workspace level.
///
/// Uses moving averages and trend regression to detect when
/// complexity is accelerating (danger) or decelerating (healthy).
///
/// Analogous to MACD in finance: both detect changes in the
/// rate of change of a metric.
pub struct ComplexityDriftDetector {
    /// Per-module complexity history.
    module_histories: HashMap<String, Vec<ComplexityObservation>>,

    /// Short-term EMA (5 commits).
    short_ema: ExponentialMovingAverage,

    /// Long-term EMA (25 commits).
    long_ema: ExponentialMovingAverage,
}

pub struct ComplexityObservation {
    pub commit_hash: String,
    pub timestamp_ms: i64,
    pub cyclomatic_complexity: f64,
    pub cognitive_complexity: f64,
    pub lines_of_code: usize,
    pub function_count: usize,
}

impl ComplexityDriftDetector {
    /// Detect complexity trend direction and acceleration.
    ///
    /// Returns (trend_direction, acceleration, confidence):
    /// - trend_direction > 0: complexity increasing
    /// - acceleration > 0: complexity growth accelerating (red flag)
    pub fn detect(&self) -> ComplexityTrend {
        let short = self.short_ema.current();
        let long = self.long_ema.current();

        // MACD equivalent: difference between short and long EMA
        let macd = short - long;

        // Signal line: EMA of MACD
        let signal = self.macd_signal_ema.current();

        ComplexityTrend {
            direction: macd.signum(),
            magnitude: macd.abs(),
            acceleration: macd - signal,  // histogram equivalent
            confidence: self.confidence_from_history(),
        }
    }
}
```

### Dependency risk scoring

```rust
/// Score the risk of dependency updates and additions.
///
/// Combines multiple signals: known CVEs, maintenance activity,
/// dependency depth, license compatibility, download trends.
///
/// Analogous to portfolio risk in finance: both aggregate
/// multiple risk factors into a single score with decomposition.
pub struct DependencyRiskScorer {
    /// Known vulnerability database.
    vuln_db: Arc<VulnerabilityDatabase>,

    /// Package registry metadata.
    registry: Arc<PackageRegistry>,

    /// Per-dependency risk history.
    histories: HashMap<String, DependencyRiskHistory>,
}

pub struct DependencyRisk {
    /// Overall risk score [0.0, 1.0].
    pub score: f64,

    /// Risk decomposition (for explainability).
    pub factors: DependencyRiskFactors,

    /// Confidence in the risk assessment.
    pub confidence: f64,
}

pub struct DependencyRiskFactors {
    /// Known CVE risk (number × severity weighting).
    pub cve_risk: f64,

    /// Maintenance risk (time since last commit, bus factor).
    pub maintenance_risk: f64,

    /// Depth risk (how deep in the dependency tree — deeper = harder to fix).
    pub depth_risk: f64,

    /// License risk (compatibility with project license).
    pub license_risk: f64,

    /// Popularity risk (very popular = well-tested; unpopular = less scrutiny).
    pub popularity_risk: f64,
}
```

### Performance regression forecasting

```rust
/// Predict performance impact of code changes.
///
/// Uses historical benchmark data + change scope analysis.
/// Analogous to volatility estimation in finance: both predict
/// the magnitude of future deviations from baseline.
pub struct PerfRegressionPredictor {
    /// Historical benchmark results per test.
    benchmarks: HashMap<String, Vec<BenchmarkResult>>,

    /// File-to-benchmark mapping.
    file_bench_map: Arc<FileBenchMap>,

    /// Baseline performance per benchmark.
    baselines: HashMap<String, BenchmarkBaseline>,
}

pub struct BenchmarkBaseline {
    /// Median throughput or latency.
    pub median: f64,
    /// Interquartile range (robust spread estimate).
    pub iqr: f64,
    /// Number of observations.
    pub n: u64,
}

impl PerfRegressionPredictor {
    /// Predict whether a change will cause a performance regression.
    /// Returns (probability_of_regression, expected_magnitude, confidence).
    pub fn predict(&self, change: &ChangeContext) -> PerfPrediction {
        let affected_benches = self.file_bench_map.benches_for_files(&change.files);

        let mut regression_prob = 0.0;
        let mut expected_magnitude = 0.0;
        let count = affected_benches.len() as f64;

        for bench in &affected_benches {
            if let Some(baseline) = self.baselines.get(bench) {
                // Historical regression rate for this benchmark
                let hist_rate = self.historical_regression_rate(bench);

                // Scale by change size (larger changes → more likely to regress)
                let adj_rate = hist_rate * self.change_size_factor(change);

                regression_prob += adj_rate;
                expected_magnitude += baseline.iqr * adj_rate;
            }
        }

        if count > 0.0 {
            regression_prob /= count;
            expected_magnitude /= count;
        }

        PerfPrediction {
            regression_probability: regression_prob,
            expected_magnitude,
            confidence: self.confidence_from_sample_size(count as u64),
            affected_benchmarks: affected_benches,
        }
    }
}
```

---

## The 6 T0 coding probes

At Gamma frequency, 6 coding-specific probes run with zero LLM cost:

```rust
/// The 6 coding-domain T0 probes.
/// These are the coding equivalents of the 8 chain probes.
pub fn coding_probes() -> Vec<Box<dyn Probe>> {
    vec![
        // 9. Build health — did the last compile succeed?
        //    Is the success rate trending down?
        //    error = 0.0 if last build passed and trend stable
        //    error = 1.0 if last build failed and trend declining
        Box::new(BuildHealthProbe::new()),

        // 10. Test regression — have any tests started failing
        //     since the last run? Delta of passing test count.
        //     error = 0.0 if no change, scales with delta
        Box::new(TestRegressionProbe::new()),

        // 11. Complexity drift — is cyclomatic complexity moving
        //     average accelerating? (MACD-equivalent probe)
        //     error = 0.0 if stable, scales with acceleration
        Box::new(ComplexityDriftProbe::new()),

        // 12. Dependency risk — have any new vulnerabilities
        //     appeared in the dependency tree?
        //     error = 0.0 if clean, scales with CVE severity
        Box::new(DependencyRiskProbe::new()),

        // 13. Coverage delta — has test coverage dropped?
        //     error = 0.0 if stable or increasing
        //     error scales with coverage decrease magnitude
        Box::new(CoverageDeltaProbe::new()),

        // 14. Error rate — is the gate failure trend over the
        //     last N tasks increasing?
        //     error = 0.0 if improving, scales with failure trend
        Box::new(ErrorRateProbe::new()),
    ]
}
```

Combined with the 8 chain probes and 2 universal probes, these form the 16 T0 probes that drive ~80% of cognitive cycles to zero LLM cost. For a pure coding agent (no chain domain), the chain probes are disabled and only the 6 coding + 2 universal probes run.

---

## Tech debt as a feedback loop

The coding oracle detects and quantifies tech debt accumulation, which creates the same kind of feedback loop that chain oracles track in DeFi:

```
Tech debt accumulates
  → Development slows (increasing build times, more test failures)
  → Engineers take more shortcuts (increasing complexity)
  → More tech debt accumulates
  → Eventually: system becomes unmaintainable (analogous to protocol insolvency)
```

The coding oracle breaks this loop by making it visible. When complexity drift acceleration exceeds a threshold, the oracle emits a Warning knowledge entry via the Neuro subsystem:

```rust
// Complexity drift exceeds threshold → emit Warning
if complexity_trend.acceleration > 0.05 {
    neuro.store(KnowledgeEntry {
        kind: KnowledgeType::Warning,
        content: format!(
            "Complexity growth accelerating in module {}: Δ²C = {:.3}. \
             Historical pattern: modules with this acceleration rate \
             reach unmaintainability within {} commits.",
            module, complexity_trend.acceleration,
            self.estimated_commits_to_crisis(complexity_trend),
        ),
        confidence: complexity_trend.confidence,
        tier: KnowledgeTier::Working,
        ..Default::default()
    }).await?;
}
```

---

## Integration with roko-index

The coding oracle relies on `roko-index` for code intelligence — symbol graphs, dependency analysis, file-to-test mappings, and HDC fingerprints of code structure:

```rust
/// roko-index provides the code intelligence layer.
/// The coding oracle queries it for:
/// - File → symbol graph (function signatures, type definitions)
/// - File → test mapping (which tests cover which files)
/// - File → dependency mapping (which crates/modules depend on which)
/// - Module → complexity metrics (cyclomatic, cognitive, LOC)
/// - Workspace → HDC fingerprint (10,240-bit structural hash)
pub struct CodeIntelligenceIntegration {
    index: Arc<RokoIndex>,
}
```

HDC fingerprints from `roko-index` enable structural similarity search across codebases. When the coding oracle detects a pattern (e.g., "high-churn modules with low coverage tend to produce production bugs"), it encodes this as an HDC vector. If the same structural pattern appears in a different crate or even a different project, the Neuro subsystem's cross-domain similarity search detects the resonance.

---

## Academic foundations

- McCabe, T. J. (1976). "A Complexity Measure." *IEEE Transactions on Software Engineering*, SE-2(4), 308-320. — Cyclomatic complexity metric.
- Lehman, M. M. (1980). "Programs, Life Cycles, and Laws of Software Evolution." *Proceedings of the IEEE*, 68(9), 1060-1076. — Software evolution laws (increasing complexity, declining quality).
- Nagappan, N., & Ball, T. (2005). "Use of Relative Code Churn Measures to Predict System Defect Density." *ICSE 2005*. — Code churn as defect predictor.
- Chen, L., et al. (2023). "FrugalGPT." arXiv:2305.05176. — Cascade architecture for T0 probe system.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). — HDC fingerprints for code structure.
- Ousterhout, J. (2018). *A Philosophy of Software Design*. — Complexity management principles.

---

## Cross-references

- See [01-oracle-trait.md](./01-oracle-trait.md) for the Oracle trait these implement
- See [02-chain-oracles.md](./02-chain-oracles.md) for the chain equivalents
- See [04-research-oracles.md](./04-research-oracles.md) for the research equivalents
- See [05-witness-as-ta-generalized.md](./05-witness-as-ta-generalized.md) for the generalized witness pipeline
- See [09-causal-microstructure-discovery.md](./09-causal-microstructure-discovery.md) for causal analysis of code change patterns
