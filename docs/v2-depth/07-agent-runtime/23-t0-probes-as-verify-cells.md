# 23. T0 Probes as Verify Cells

> 16 zero-cost Verify Cells that run every gamma tick. Each implements the Verify protocol with O(1) deterministic evaluation -- no LLM needed. Their aggregate output drives the T0/T1/T2 gating decision that makes high-frequency cognition economically viable.

See [02-CELL.md](../../unified/02-CELL.md) for Cell and Verify protocol, [05-AGENT.md](../../unified/05-AGENT.md) for CorticalState.

---

## 1. Probes Are Verify Cells

A T0 probe is a Cell implementing the Verify protocol with one constraint: **evaluation must be O(1) and deterministic.** No LLM call, no network request (for universal probes), no file I/O beyond cached reads. Domain probes (chain, coding) may perform lightweight cached reads (~10ms) but must never block.

The Verify protocol signature is:

```rust
/// Every T0 probe implements this trait.
/// The Verify protocol returns a Verdict -- here simplified to a scalar
/// anomaly score and a weight for aggregation.
///
/// Crate: `crates/roko-core/src/probe.rs`
pub trait Probe: Cell + Send + Sync {
    /// Evaluate against current state. Returns anomaly in [0.0, 1.0].
    /// 0.0 = completely expected. 1.0 = maximum anomaly.
    /// MUST complete in < 10ms. MUST be deterministic. MUST be side-effect-free.
    fn evaluate(&self, state: &EngineState) -> f32;

    /// Weight in aggregate prediction error. Higher = more influence on gating.
    fn weight(&self) -> f32;

    /// Domain this probe belongs to.
    fn domain(&self) -> ProbeDomain;
}
```

Because probes implement the Verify protocol, they compose naturally with the Gate pipeline. A T0 probe is the same type of Cell as a CompileGate or TestGate -- just faster and cheaper.

---

## 2. The 16 Default Probes

### 2.1 Chain Domain (8 probes)

| # | Name | What It Detects | Weight | Threshold |
|---|---|---|---|---|
| 1 | PriceDelta | Significant price movement vs last tick | 0.15 | 2% per-asset volatility-normalized |
| 2 | TvlDelta | Total value locked change across tracked protocols | 0.10 | 5% TVL change = max signal |
| 3 | PositionHealth | Collateral ratio approaching liquidation | 0.20 | health < 1.2 = critical |
| 4 | GasSpike | Gas price surge above EMA baseline | 0.05 | 3x baseline = max signal |
| 5 | CreditBalance | Remaining operational budget (days of runway) | 0.05 | < 1 day = critical |
| 6 | RSI | Relative Strength Index extreme values | 0.05 | >80 or <20 = extreme |
| 7 | MACD | Momentum shift via crossover or divergence | 0.05 | Crossover = 0.7 signal |
| 8 | CircuitBreaker | Exchange halt, protocol pause, emergency shutdown | 0.10 | Binary: active = 1.0 |

### 2.2 Coding Domain (6 probes)

| # | Name | What It Detects | Weight | Threshold |
|---|---|---|---|---|
| 9 | BuildHealth | Last compilation result (success/warning/failure) | 0.20 | Failure = 0.8 |
| 10 | TestRegression | Change in passing test count | 0.20 | Each failing test = 0.2 |
| 11 | ComplexityDrift | Cyclomatic complexity moving average increase | 0.05 | 10% increase = max |
| 12 | DependencyRisk | New vulnerabilities in dependency scan | 0.10 | 3+ vulns = 0.7 |
| 13 | CoverageDelta | Test coverage percentage change | 0.05 | 10% drop = max |
| 14 | ErrorRate | Gate failure trend over last N tasks | 0.10 | >50% failure = 0.8 |

### 2.3 Universal (2 probes)

| # | Name | What It Detects | Weight | Threshold |
|---|---|---|---|---|
| 15 | WorldModelDrift | Divergence between predicted and actual state | 0.15 | Cosine distance in [0, 1] |
| 16 | CausalConsistency | Lineage DAG integrity (missing parents, hash mismatches) | 0.10 | 3+ issues = 0.8 |

---

## 3. Anomaly Detection: Rolling Z-Score

Each probe maintains a rolling window of its own outputs (last 100 values by default). An output is flagged **anomalous** when it exceeds 2 standard deviations from the rolling mean:

```rust
/// Per-probe anomaly detector using rolling z-score.
///
/// Window: last 100 evaluations (configurable).
/// Threshold: z > 2.0 flags as anomalous.
///
/// This is the "peripheral vision" that lets the Agent notice
/// change without LLM deliberation.
pub struct AnomalyDetector {
    window: VecDeque<f32>,
    window_size: usize,    // default: 100
    threshold: f32,        // default: 2.0
    running_sum: f64,
    running_sum_sq: f64,
}

impl AnomalyDetector {
    pub fn push(&mut self, value: f32) -> bool {
        if self.window.len() >= self.window_size {
            let old = self.window.pop_front().unwrap() as f64;
            self.running_sum -= old;
            self.running_sum_sq -= old * old;
        }
        self.window.push_back(value);
        self.running_sum += value as f64;
        self.running_sum_sq += (value as f64) * (value as f64);

        let n = self.window.len() as f64;
        let mean = self.running_sum / n;
        let variance = (self.running_sum_sq / n) - (mean * mean);
        let std_dev = variance.max(0.0).sqrt();

        if std_dev < 1e-6 {
            return value > 0.5; // fallback for constant signal
        }

        let z_score = ((value as f64) - mean) / std_dev;
        z_score.abs() > self.threshold as f64
    }
}
```

The anomaly flag feeds two outputs:
1. **Aggregate prediction error**: weighted sum of probe values (not anomaly flags).
2. **Anomaly count**: number of probes that flagged anomalous (feeds regime detection).

---

## 4. Prediction Error Aggregation

The ProbeRegistry runs all probes and computes the aggregate:

```rust
/// Aggregate prediction error from all probes.
///
/// prediction_error = sum(probe_value * probe_weight), capped at 1.0
///
/// This scalar drives the T0/T1/T2 gating decision.
pub struct ProbeRegistry {
    probes: Vec<(Box<dyn Probe>, AnomalyDetector)>,
}

impl ProbeRegistry {
    pub fn evaluate_all(&mut self, state: &EngineState) -> ProbeResults {
        let mut results = Vec::with_capacity(self.probes.len());
        let mut aggregate: f32 = 0.0;
        let mut anomaly_count: u32 = 0;

        for (probe, detector) in &mut self.probes {
            let value = probe.evaluate(state);
            let weight = probe.weight();
            let is_anomalous = detector.push(value);

            aggregate += value * weight;
            if is_anomalous {
                anomaly_count += 1;
            }

            results.push(ProbeResult {
                name: probe.name().to_string(),
                value,
                weight,
                is_anomalous,
                domain: probe.domain(),
            });
        }

        ProbeResults {
            results,
            aggregate_prediction_error: aggregate.min(1.0),
            anomaly_count,
        }
    }
}
```

### 4.1 Tier Gating Thresholds

```
prediction_error < 0.2  ->  T0 (suppress, no LLM)      ~80% of ticks
prediction_error < 0.6  ->  T1 (Haiku-class, shallow)   ~15% of ticks
prediction_error >= 0.6 ->  T2 (Opus-class, deep)       ~5% of ticks
```

These base thresholds are modulated by the adaptive threshold computation (see [dual-process-and-efe-routing.md](dual-process-and-efe-routing.md)):
- **Affect state**: Low dominance -> lower threshold (escalate sooner).
- **Arousal**: High arousal -> lower threshold (pay more attention).
- **Budget pressure**: Approaching ceiling -> higher threshold (be conservative).
- **Strategy confidence**: High confidence -> higher threshold (coast on heuristics).

---

## 5. How Probes Feed CorticalState

CorticalState is the Agent's 32-signal atomic shared perception surface. Probes write to it every gamma tick:

```rust
/// CorticalState fields written by probes (subset of 32 total fields).
///
/// Lock-free atomic reads: any Cell can read CorticalState at any time
/// without blocking. Probes are the primary writers.
///
/// Crate: `crates/roko-core/src/cortical.rs`
pub struct CorticalState {
    // Written by RegimeDetector (from probe aggregation)
    pub regime: AtomicU8,              // Calm=0, Normal=1, Volatile=2, Crisis=3

    // Written by ProbeRegistry
    pub prediction_error: AtomicF32,   // [0.0, 1.0]
    pub anomaly_count: AtomicU32,      // 0-16
    pub last_tier: AtomicU8,           // T0=0, T1=1, T2=2

    // Written by Daimon (read by probes for threshold modulation)
    pub pleasure: AtomicF32,           // [-1.0, 1.0]
    pub arousal: AtomicF32,            // [-1.0, 1.0]
    pub dominance: AtomicF32,          // [-1.0, 1.0]
    pub behavioral_state: AtomicU8,    // Engaged=0..Resting=5

    // Written by BudgetTracker
    pub budget_fraction: AtomicF32,    // [0.0, 1.0] daily spend / limit
    pub resource_health: AtomicF32,    // [0.0, 1.0]

    // ... remaining 22 fields (accuracy, causal_consistency, etc.)
}
```

The flow: Probes -> CorticalState -> HeartbeatPolicy (reads regime, anomaly_count) -> adjusts tick interval. This creates a feedback loop where the environment's volatility directly controls the Agent's sampling rate.

---

## 6. Prediction Error Gating: T0 -> T1 -> T2 Cascade

The cascade emerges from the economics of cost vs information gain:

```
           Prediction Error
    0.0         0.2         0.6         1.0
     |-----------|-----------|-----------|
     |    T0     |    T1     |    T2     |
     | $0.000    | $0.001    | $0.100    |
     | ~80%      | ~15%      | ~5%       |
     | probes    | Haiku     | Opus      |
     | only      | fast      | deep      |
```

When a T0 probe detects anomaly:
1. Prediction error rises above 0.2.
2. Gamma flow's ASSESS Cell routes to T1.
3. T1 Cell evaluates with a fast model.
4. If T1 result has high residual (error still > 0.6 after T1), escalate to T2.

This **progressive cascade** means most anomalies resolve at T1 ($0.001) and only genuinely surprising situations reach T2 ($0.10). The daily cost model:

| Regime | Daily Ticks | T0 (80%) | T1 (15%) | T2 (5%) | Daily Cost |
|---|---|---|---|---|---|
| Calm | 5,760 | 4,608 | 864 | 288 | ~$0.86 + $28.80 = ~$30 |
| Normal | 8,640 | 6,912 | 1,296 | 432 | ~$1.30 + $43.20 = ~$45 |

With context engineering (caching, prompt cache alignment): divide by ~6x -> $5-8/day.

---

## 7. Extensibility: Custom Probes

Any domain plugin registers probes by implementing the Probe trait:

```rust
// Example: medical domain probe
pub struct PatientVitalsProbe;

impl Probe for PatientVitalsProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        state.custom_metric("patient_vitals_deviation").clamp(0.0, 1.0)
    }
    fn weight(&self) -> f32 { 0.25 }
    fn name(&self) -> &str { "patient_vitals" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Custom("medical") }
}

// Registration at plugin init
registry.register(Box::new(PatientVitalsProbe));
```

The probe set is composable: a multi-domain agent registers probes from all its active domains. Weights are normalized at runtime so the aggregate stays in [0.0, 1.0].

---

## What This Enables

- **80% of ticks cost $0.00**: The majority of gamma ticks are handled by pure-Rust deterministic probes. No LLM, no cost.
- **High-frequency perception**: An Agent can tick every 5 seconds without going bankrupt.
- **Principled escalation**: The T0->T1->T2 cascade is driven by information theory (prediction error), not arbitrary thresholds.
- **Domain extensibility**: New domains add probes; the gating machinery works unchanged.
- **CorticalState as shared surface**: All subsystems can read probe results without querying the ProbeRegistry.

## Feedback Loops

1. **Probes -> prediction_error -> tier selection -> action outcome -> calibration -> threshold adjustment** (Loop): Actions taken at T1/T2 produce outcomes that calibrate future thresholds.
2. **Probe value -> AnomalyDetector -> anomaly_count -> regime -> gamma_interval -> probe frequency** (Loop): More anomalies -> faster ticks -> more probe evaluations -> resolve or confirm the anomaly.
3. **Probe -> CorticalState -> Daimon -> threshold modulation -> probe interpretation** (Loop): Affect state (from Daimon) modulates how aggressively probes trigger escalation.

## Open Questions

1. Should probe weights be learnable (predict-publish-correct on weight allocation)?
2. Should there be a minimum set of universal probes that cannot be disabled (safety invariant)?
3. How should the rolling z-score window size adapt to regime (shorter window in Crisis for faster detection)?
4. Should probes emit their own Pulses for fine-grained observability, or is CorticalState sufficient?

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Define `Probe` trait | `crates/roko-core/src/probe.rs` | Not started |
| Define `ProbeRegistry` + `evaluate_all()` | `crates/roko-core/src/probe.rs` | Not started |
| Implement `AnomalyDetector` (rolling z-score) | `crates/roko-core/src/anomaly.rs` | Not started |
| Implement 6 coding probes | `crates/roko-cli/src/probes/coding.rs` | Not started |
| Implement 2 universal probes | `crates/roko-core/src/probes/universal.rs` | Not started |
| Define `CorticalState` atomic struct | `crates/roko-core/src/cortical.rs` | Not started |
| Wire ProbeRegistry into orchestrate.rs gamma loop | `crates/roko-cli/src/orchestrate.rs` | Not started |
| Define `EngineState` trait for probe state access | `crates/roko-core/src/engine_state.rs` | Not started |
