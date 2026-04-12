# The 16 T0 Probes: Zero-LLM Cognitive Perception

> 16 deterministic probes that run on every gamma tick with zero LLM cost — the foundation of the 80% T0 suppression rate that makes high-frequency agent cognition economically viable.

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md), [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md)
**Key sources**: `refactoring-prd/09-innovations.md` §I, legacy `bardo-backup/prd/01-golem/02-heartbeat.md` §S3

---

## Abstract

The 16 T0 probes are the agent's "peripheral vision" — lightweight, deterministic checks that run on every gamma tick (~5-15 seconds) with **zero LLM cost**. Each probe is a pure function: `fn probe(state: &EngineState) -> f32`. No LLM inference, no network calls (for domain-agnostic probes), no I/O beyond what the domain requires for state reads. The probes compute a prediction error scalar (0.0-1.0) that drives the T0/T1/T2 gating decision.

The probe architecture implements FrugalGPT's (Chen et al. 2023, arXiv:2305.05176) core insight: you don't need a powerful model for every query — you need intelligent routing that uses cheap checks to determine when the expensive model is necessary. The 16 probes are those cheap checks. They determine, with high precision, whether the current observation is surprising enough to warrant LLM deliberation.

8 probes are domain-specific to the blockchain domain, 6 are domain-specific to the coding domain, and 2 are universal (applicable to any domain). The probe registry is extensible — any domain plugin can add probes by implementing the `Probe` trait.

This document specifies all 16 probes in detail, explains the prediction error aggregation formula, and describes the probe registry extensibility mechanism.

---

## The Probe Trait

Every probe implements a single trait:

```rust
/// A zero-cost cognitive probe.
///
/// Probes are pure functions that evaluate a single dimension of
/// the agent's environment. They run on every gamma tick and
/// produce a scalar signal that contributes to the aggregate
/// prediction error.
///
/// Probes MUST be:
/// - Deterministic (same input → same output)
/// - Fast (< 10ms execution time)
/// - Side-effect-free (no writes, no LLM calls)
/// - Domain probes may perform lightweight reads (RPC calls, file stats)
///   but must not mutate state
pub trait Probe: Send + Sync {
    /// Evaluate this probe against the current engine state.
    /// Returns a scalar in [0.0, 1.0] where:
    /// - 0.0 = completely expected, nothing to report
    /// - 1.0 = maximum anomaly, definitely needs attention
    fn evaluate(&self, state: &EngineState) -> f32;

    /// The weight of this probe in the aggregate prediction error.
    /// Higher weight = more influence on the gating decision.
    fn weight(&self) -> f32;

    /// Human-readable name for logging and debugging.
    fn name(&self) -> &str;

    /// Which domain this probe belongs to.
    fn domain(&self) -> ProbeDomain;
}

pub enum ProbeDomain {
    Chain,
    Coding,
    Research,
    Universal,
    Custom(String),
}
```

The probe registry is a simple `Vec<Box<dyn Probe>>`:

```rust
/// The probe registry: an ordered list of probes that run on every gamma tick.
///
/// Extensible: domain plugins add probes at initialization.
/// No registration conflicts because probes are independent —
/// each contributes its own signal to the aggregate.
pub struct ProbeRegistry {
    probes: Vec<Box<dyn Probe>>,
}

impl ProbeRegistry {
    /// Run all probes against the current state.
    /// Returns individual results and the aggregate prediction error.
    pub fn evaluate_all(&self, state: &EngineState) -> ProbeResults {
        let results: Vec<ProbeResult> = self.probes
            .iter()
            .map(|p| ProbeResult {
                name: p.name().to_string(),
                value: p.evaluate(state),
                weight: p.weight(),
                domain: p.domain(),
                is_anomalous: p.evaluate(state) > 0.5, // configurable threshold
            })
            .collect();

        let aggregate = results.iter()
            .map(|r| r.value * r.weight)
            .sum::<f32>()
            .min(1.0);

        ProbeResults { results, aggregate }
    }

    /// Register a new probe.
    pub fn register(&mut self, probe: Box<dyn Probe>) {
        self.probes.push(probe);
    }
}
```

---

## The 16 Default Probes

### Blockchain Domain Probes (8)

These probes are registered by `roko-chain` at startup. They require lightweight RPC calls to read on-chain state (~10ms each for cached endpoints).

#### Probe 1: Price Delta

```rust
/// Detects significant price changes since the last tick.
///
/// A large price delta indicates either an opportunity (potential trade)
/// or a threat (position health deterioration). The threshold is
/// calibrated per-asset based on historical volatility.
pub struct PriceDeltaProbe {
    /// Per-asset volatility-normalized thresholds
    thresholds: HashMap<AssetId, f32>,
}

impl Probe for PriceDeltaProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let max_delta = state.tracked_assets()
            .iter()
            .map(|asset| {
                let delta = (asset.current_price - asset.last_tick_price).abs()
                    / asset.last_tick_price;
                let threshold = self.thresholds.get(&asset.id)
                    .copied()
                    .unwrap_or(0.02);  // 2% default
                (delta / threshold).min(1.0)
            })
            .fold(0.0f32, f32::max);
        max_delta
    }

    fn weight(&self) -> f32 { 0.15 }
    fn name(&self) -> &str { "price_delta" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Chain }
}
```

#### Probe 2: TVL Delta

Detects changes in total value locked across tracked protocols. A sudden TVL drop may indicate a bank run, exploit, or loss of confidence.

```rust
pub struct TvlDeltaProbe;

impl Probe for TvlDeltaProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let delta = state.tvl_delta_percent().abs();
        (delta / 0.05).min(1.0)  // 5% TVL change = maximum signal
    }

    fn weight(&self) -> f32 { 0.10 }
    fn name(&self) -> &str { "tvl_delta" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Chain }
}
```

#### Probe 3: Position Health

Monitors collateral ratios and liquidation distance for active positions. This is the most safety-critical chain probe — a position approaching liquidation demands immediate attention.

```rust
pub struct PositionHealthProbe;

impl Probe for PositionHealthProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        state.positions()
            .iter()
            .map(|pos| {
                let health = pos.health_factor();  // 1.0 = safe, 0.0 = liquidation
                if health < 1.2 {
                    1.0  // Critical: near liquidation
                } else if health < 1.5 {
                    0.6  // Warning: declining health
                } else if health < 2.0 {
                    0.2  // Moderate: worth monitoring
                } else {
                    0.0  // Healthy
                }
            })
            .fold(0.0f32, f32::max)
    }

    fn weight(&self) -> f32 { 0.20 }  // Highest-weight chain probe
    fn name(&self) -> &str { "position_health" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Chain }
}
```

#### Probe 4: Gas Spike

Detects sudden gas price increases. High gas costs may make pending transactions uneconomical.

```rust
pub struct GasSpikeProbe;

impl Probe for GasSpikeProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let current = state.gas_price_gwei();
        let baseline = state.gas_ema_gwei();  // EMA over last 100 ticks
        let ratio = current / baseline.max(1.0);
        ((ratio - 1.0) / 2.0).clamp(0.0, 1.0)  // 3× baseline = maximum signal
    }

    fn weight(&self) -> f32 { 0.05 }
    fn name(&self) -> &str { "gas_spike" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Chain }
}
```

#### Probe 5: Credit Balance

Monitors remaining KORAI balance. Low balance constrains the agent's ability to act.

```rust
pub struct CreditBalanceProbe;

impl Probe for CreditBalanceProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let balance = state.korai_balance();
        let daily_burn = state.daily_burn_rate();
        let days_remaining = balance / daily_burn.max(0.01);
        if days_remaining < 1.0 {
            1.0  // Critical: less than 1 day
        } else if days_remaining < 7.0 {
            0.5  // Warning: less than a week
        } else {
            0.0  // Healthy
        }
    }

    fn weight(&self) -> f32 { 0.05 }
    fn name(&self) -> &str { "credit_balance" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Chain }
}
```

#### Probe 6: RSI (Relative Strength Index)

14-period RSI. Extreme values (>70 overbought, <30 oversold) indicate potential reversals.

```rust
pub struct RsiProbe;

impl Probe for RsiProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let rsi = state.rsi_14();
        if rsi > 80.0 || rsi < 20.0 {
            0.8  // Extreme
        } else if rsi > 70.0 || rsi < 30.0 {
            0.4  // Notable
        } else {
            0.0  // Normal range
        }
    }

    fn weight(&self) -> f32 { 0.05 }
    fn name(&self) -> &str { "rsi" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Chain }
}
```

#### Probe 7: MACD (Moving Average Convergence/Divergence)

Detects momentum shifts via MACD crossovers and divergences.

```rust
pub struct MacdProbe;

impl Probe for MacdProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let macd = state.macd();
        if macd.just_crossed() {
            0.7  // Crossover: significant momentum shift
        } else if macd.divergence().abs() > macd.baseline_divergence() * 2.0 {
            0.4  // Strong divergence
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 { 0.05 }
    fn name(&self) -> &str { "macd" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Chain }
}
```

#### Probe 8: Circuit Breaker

Detects exchange halts, protocol pauses, or emergency shutdowns.

```rust
pub struct CircuitBreakerProbe;

impl Probe for CircuitBreakerProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        if state.any_circuit_breaker_active() {
            1.0  // Immediate escalation
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 { 0.10 }
    fn name(&self) -> &str { "circuit_breaker" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Chain }
}
```

### Coding Domain Probes (6)

These probes are registered by the coding domain plugin. They read local filesystem state and build system outputs.

#### Probe 9: Build Health

Monitors the last compilation result and trend. A compilation failure is always significant.

```rust
pub struct BuildHealthProbe;

impl Probe for BuildHealthProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        match state.last_build_result() {
            BuildResult::Success => 0.0,
            BuildResult::Warning(count) => (count as f32 * 0.1).min(0.5),
            BuildResult::Failure => 0.8,
            BuildResult::Unknown => 0.3,  // No recent build data
        }
    }

    fn weight(&self) -> f32 { 0.20 }
    fn name(&self) -> &str { "build_health" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Coding }
}
```

#### Probe 10: Test Regression

Detects changes in test count since the last run. A decrease in passing tests indicates regression.

```rust
pub struct TestRegressionProbe;

impl Probe for TestRegressionProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let delta = state.test_pass_count_delta();
        if delta < 0 {
            ((-delta) as f32 * 0.2).min(1.0)  // Each failing test: 0.2
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 { 0.20 }
    fn name(&self) -> &str { "test_regression" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Coding }
}
```

#### Probe 11: Complexity Drift

Monitors cyclomatic complexity moving average. Increasing complexity may indicate code quality degradation.

```rust
pub struct ComplexityDriftProbe;

impl Probe for ComplexityDriftProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let drift = state.complexity_delta_percent();
        (drift / 10.0).clamp(0.0, 1.0)  // 10% complexity increase = maximum
    }

    fn weight(&self) -> f32 { 0.05 }
    fn name(&self) -> &str { "complexity_drift" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Coding }
}
```

#### Probe 12: Dependency Risk

Monitors vulnerability scan results for dependency changes.

```rust
pub struct DependencyRiskProbe;

impl Probe for DependencyRiskProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let new_vulns = state.new_vulnerability_count();
        match new_vulns {
            0 => 0.0,
            1..=2 => 0.4,
            3..=5 => 0.7,
            _ => 1.0,
        }
    }

    fn weight(&self) -> f32 { 0.10 }
    fn name(&self) -> &str { "dependency_risk" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Coding }
}
```

#### Probe 13: Coverage Delta

Monitors test coverage changes. A significant drop in coverage may indicate untested new code.

```rust
pub struct CoverageDeltaProbe;

impl Probe for CoverageDeltaProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let delta = state.coverage_delta_percent();
        if delta < -2.0 {
            ((-delta) / 10.0).min(1.0)  // 10% coverage drop = maximum
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 { 0.05 }
    fn name(&self) -> &str { "coverage_delta" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Coding }
}
```

#### Probe 14: Error Rate

Monitors gate failure trend over the last N tasks.

```rust
pub struct ErrorRateProbe;

impl Probe for ErrorRateProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let failure_rate = state.gate_failure_rate_last_n(10);
        if failure_rate > 0.5 {
            0.8  // More than half failing
        } else if failure_rate > 0.3 {
            0.4
        } else {
            0.0
        }
    }

    fn weight(&self) -> f32 { 0.10 }
    fn name(&self) -> &str { "error_rate" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Coding }
}
```

### Universal Probes (2)

These probes apply to all domains.

#### Probe 15: World Model Drift

Measures divergence between the agent's predicted state and the actual observed state. This is the core active inference signal (Friston 2010).

```rust
pub struct WorldModelDriftProbe;

impl Probe for WorldModelDriftProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let predicted = state.predicted_state_vector();
        let actual = state.actual_state_vector();
        let drift = cosine_distance(&predicted, &actual);
        drift.clamp(0.0, 1.0)
    }

    fn weight(&self) -> f32 { 0.15 }
    fn name(&self) -> &str { "world_model_drift" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Universal }
}
```

#### Probe 16: Causal Consistency

Checks the integrity of the lineage DAG. Detects missing parents, hash mismatches, or orphaned Engrams that could indicate corruption or tampering.

```rust
pub struct CausalConsistencyProbe;

impl Probe for CausalConsistencyProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        let issues = state.lineage_dag_issues();
        match issues {
            0 => 0.0,
            1..=2 => 0.3,
            _ => 0.8,
        }
    }

    fn weight(&self) -> f32 { 0.10 }
    fn name(&self) -> &str { "causal_consistency" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Universal }
}
```

---

## Prediction Error Aggregation

The 16 probe results are aggregated into a single prediction error scalar:

```
prediction_error = Σ(probe_value × probe_weight)    capped at 1.0
```

The error thresholds for tier routing:

```
error < 0.2  → T0 (suppress, no LLM)     ~80% of ticks
error < 0.6  → T1 (fast model, shallow)   ~15% of ticks
error ≥ 0.6  → T2 (full model, deep)      ~5% of ticks
```

The thresholds (0.2 and 0.6) are the base values. They are adjusted by the adaptive threshold computation (see [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md)) which modulates based on affect state, resource constraints, and strategy confidence.

---

## Extensibility: Adding Custom Probes

New domains add probes by implementing the `Probe` trait and registering with the `ProbeRegistry`:

```rust
// Example: a medical domain probe
pub struct PatientVitalsProbe;

impl Probe for PatientVitalsProbe {
    fn evaluate(&self, state: &EngineState) -> f32 {
        // Check vital sign deviations from baselines
        let deviation = state.custom_metric("patient_vitals_deviation");
        deviation.clamp(0.0, 1.0)
    }

    fn weight(&self) -> f32 { 0.25 }
    fn name(&self) -> &str { "patient_vitals" }
    fn domain(&self) -> ProbeDomain { ProbeDomain::Custom("medical".into()) }
}

// Registration at domain plugin initialization
registry.register(Box::new(PatientVitalsProbe));
```

The probe set is composable: users combine whatever probe set matches their domain. A chain agent registers the 8 chain probes + 2 universal probes. A coding agent registers the 6 coding probes + 2 universal probes. A medical agent registers custom medical probes + 2 universal probes. A multi-domain agent registers probes from all relevant domains.

---

## Academic Foundations

- **Chen et al. 2023** — FrugalGPT (arXiv:2305.05176, published 2024 TMLR). Cascade architectures with intelligent routing achieve up to 98% cost reduction while matching top-model quality.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Prediction error as the fundamental signal driving attention and action.
- **Kahneman 2011** — "Thinking, Fast and Slow" (Farrar, Straus and Giroux). System 1 (probes) handles the majority; System 2 (LLM) handles exceptions.
- **Sims 2003** — "Implications of rational inattention" (Journal of Monetary Economics 50(3)). Agents optimally allocate attention based on information value relative to cost.

---

## Current Status and Gaps

**What exists:**
- `InferenceTier` enum and `TierRouter` in `bardo-primitives/src/tier.rs`.
- The conceptual probe design exists in the PRD documents.
- `CascadeRouter` implements three-stage model routing based on observation counts.

**What is missing:**
- `Probe` trait definition.
- `ProbeRegistry` struct and `evaluate_all()` method.
- Concrete implementations of all 16 probes.
- Integration of probe results into the orchestration loop for tier gating.
- Domain plugin registration mechanism for custom probes.
- Probe-level telemetry (which probes fire, how often, latency).

---

## Cross-References

- See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for how probe results drive tier selection
- See [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md) for how probes run in the gamma loop's PERCEIVE step
- See [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md) for the theoretical framework behind probe-driven allocation
- See topic [08-chain](../08-chain/INDEX.md) for chain-specific probe details
- See topic [15-code-intelligence](../15-code-intelligence/INDEX.md) for coding probe integration
