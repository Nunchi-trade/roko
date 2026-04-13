# 06 — Adaptive Gate Thresholds

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-gate` (`crates/roko-gate/src/adaptive_threshold.rs`)
> **Status**: Implemented (215 lines), persists to `.roko/learn/gate-thresholds.json`


> **Implementation**: Shipping

---

## 1. Overview

Adaptive thresholds tune verification behavior based on historical pass rates. They use
exponential moving averages (EMA) per gate rung to track how often each rung passes, and
from that, derive two advisory signals:

1. **Retry budget**: How many retries should a rung get? High pass rate → fewer retries.
   Low pass rate → more retries.
2. **Skip advisory**: Should a rung be skipped? If it has passed 20+ times consecutively,
   it's probably always passing and could be skipped to save time.

Both signals are advisory — the orchestrator may override them. But they provide a
data-driven default that adapts to the project's actual verification characteristics.

> **Citation**: crates/roko-gate/src/adaptive_threshold.rs — Full implementation.

---

## 2. Per-Rung Statistics

```rust
pub struct RungStats {
    pub ema_pass_rate: f64,       // Exponential moving average of pass rate [0.0, 1.0]
    pub total_observations: u64,  // Total gate runs for this rung
    pub consecutive_passes: u32,  // Consecutive passes (reset on any failure)
}
```

Each rung gets its own `RungStats`. A fresh rung starts with:
- `ema_pass_rate = 0.5` (neutral prior — no assumption about pass/fail tendency)
- `total_observations = 0`
- `consecutive_passes = 0`

---

## 3. The EMA Algorithm

### 3.1 Update Rule

```rust
pub fn update(&mut self, rung: u32, passed: bool) {
    let stats = self.rungs.entry(rung).or_default();
    let value = if passed { 1.0 } else { 0.0 };

    if stats.total_observations == 0 {
        stats.ema_pass_rate = value;  // First observation sets the rate directly
    } else {
        stats.ema_pass_rate = EMA_ALPHA.mul_add(value, (1.0 - EMA_ALPHA) * stats.ema_pass_rate);
    }

    stats.total_observations += 1;

    if passed {
        stats.consecutive_passes += 1;
    } else {
        stats.consecutive_passes = 0;
    }
}
```

### 3.2 Why EMA?

An exponential moving average with α = 0.1 means:
- Recent observations weigh more than old ones
- The effective memory is ~1/α ≈ 10 observations
- Gradual changes in pass rate are tracked smoothly

This is important because gate pass rates change over time:
- A new project with many issues has low pass rates initially
- As issues are fixed, pass rates climb
- A major refactor temporarily drops pass rates before they recover

A simple average (total passes / total observations) would be slow to respond to these
shifts. The EMA adapts within ~10 observations.

### 3.3 The α Parameter

`EMA_ALPHA = 0.1` is the decay constant. Higher α means more responsive (recent data
dominates), lower α means more stable (historical data dominates).

| α | Effective window | Behavior |
|---|---|---|
| 0.01 | ~100 observations | Very stable, slow to adapt |
| 0.1 | ~10 observations | Balanced (current default) |
| 0.3 | ~3 observations | Responsive, potentially noisy |

The choice of 0.1 balances responsiveness with stability. A gate that fails once
shouldn't immediately triple the retry budget, but a gate that fails 5 times in a row
should.

> **Citation**: bardo-backup/prd/16-testing/07-fast-feedback-loops.md — Fast feedback
> loops using EMA-based calibration.

---

## 4. Retry Budget Suggestion

```rust
pub fn suggested_max_retries(&self, rung: u32) -> u32 {
    let Some(stats) = self.rungs.get(&rung) else {
        return 3; // Default for unknown rungs
    };

    if stats.total_observations < 5 {
        return 3; // Not enough data
    }

    // Map pass rate to retries: high pass → low retries, low pass → high retries
    let retries = stats.ema_pass_rate.mul_add(-range, max).round() as u32;
    retries.clamp(MIN_RETRIES, MAX_RETRIES)
}
```

The mapping is linear:
- Pass rate 1.0 → 1 retry (it almost always passes; one attempt is enough)
- Pass rate 0.5 → 3 retries (coin flip; give it a few tries)
- Pass rate 0.0 → 5 retries (it almost never passes; maximize attempts)

### Constants

| Constant | Value | Purpose |
|---|---|---|
| `MIN_RETRIES` | 1 | Floor: always try at least once |
| `MAX_RETRIES` | 5 | Ceiling: don't waste resources endlessly |

### Cold Start

For unknown rungs or rungs with fewer than 5 observations, the default is 3 retries.
This avoids extreme behavior early in a project's lifecycle.

---

## 5. Skip Advisory

```rust
pub fn should_skip_rung(&self, rung: u32) -> bool {
    self.rungs
        .get(&rung)
        .is_some_and(|s| s.consecutive_passes >= SKIP_STREAK_THRESHOLD)
}
```

If a rung has passed `SKIP_STREAK_THRESHOLD` (20) consecutive times, the system
suggests it can be skipped. This advisory is not enforced — the orchestrator decides
whether to honor it.

### Why 20?

Twenty consecutive passes means the gate hasn't failed in at least 20 task executions.
For a gate like Compile, this suggests the project's code generation is reliable enough
that compile failures are rare. Running the gate still costs time (seconds to minutes),
and if the system has high confidence the gate will pass, skipping it saves that time.

### Why Advisory Only?

Even a gate with 100 consecutive passes can fail unexpectedly (new dependency breaks,
CI environment changes, agent produces unusually complex code). Making the skip advisory
rather than mandatory means:
- The orchestrator can skip the gate 90% of the time for speed
- Every Nth run (e.g., every 5th), it still runs the gate to check
- A failure resets the consecutive pass counter, re-enabling the gate

This "mostly skip, periodically verify" pattern is common in testing infrastructure
(see: test quarantine systems, flaky test skip-and-retry).

> **Citation**: bardo-backup/prd/16-testing/09-evaluation-map.md — 14 feedback loops
> across 5 speed tiers, including machine-speed confidence calibration.

---

## 6. Persistence

```rust
pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
    let json = serde_json::to_string_pretty(self)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_or_new(path: &Path) -> Self {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
```

### Atomic Write

The `save()` method uses the atomic write pattern: write to a temporary file, then
rename. This ensures the file is never in a half-written state. If the process crashes
between `write` and `rename`, the old file remains intact.

### Graceful Degradation

`load_or_new()` returns a fresh `AdaptiveThresholds` if the file is missing or corrupt.
This means the system always starts correctly — it just loses its historical data on
corruption.

### Storage Location

The thresholds persist to `.roko/learn/gate-thresholds.json`. This is the learning
subsystem's data directory, alongside:
- `.roko/learn/cascade-router.json` (model routing state)
- `.roko/learn/experiments.json` (prompt experiment state)
- `.roko/learn/efficiency.jsonl` (per-turn efficiency events)

> **Citation**: CLAUDE.md — "Adaptive gate thresholds: EMA per rung in
> `.roko/learn/gate-thresholds.json`."

---

## 7. Interaction with Other Components

### 7.1 Rung Selector

The rung selector (see [02-6-rung-selector.md](./02-6-rung-selector.md)) determines
which rungs to run based on static complexity. The adaptive thresholds refine this:

```
Static selection: complexity → rungs [0, 1, 2, 3]
Adaptive refinement:
  Rung 0: 25 consecutive passes → skip advisory
  Rung 1: 3 consecutive passes → run normally
  Rung 2: 12 consecutive passes → run normally
  Rung 3: not tracked yet → run with default retries
Final: rungs [1, 2, 3] with rung 0 skipped
```

### 7.2 Retry Logic

The orchestrator's retry loop consults `suggested_max_retries()`:

```rust
let max_retries = thresholds.suggested_max_retries(current_rung);
for attempt in 0..max_retries {
    let verdict = pipeline.verify(signal, ctx).await;
    if verdict.passed { break; }
    // ... escalate, adjust prompt, retry
}
```

### 7.3 Gate Pipeline Feedback

After each pipeline execution, the orchestrator updates the thresholds:

```rust
for (rung, verdict) in rung_verdicts {
    thresholds.update(rung as u32, verdict.passed);
}
thresholds.save(&thresholds_path)?;
```

This closes the feedback loop: gate outcomes → EMA update → retry budget adjustment →
different gate behavior on next execution.

---

## 8. Reporting

```rust
pub fn rung_stats(&self, rung: u32) -> Option<&RungStats> {
    self.rungs.get(&rung)
}

pub fn all_rungs(&self) -> impl Iterator<Item = (&u32, &RungStats)> {
    self.rungs.iter()
}
```

These methods enable the dashboard and status commands to display per-rung health:

```
Gate Thresholds:
  Rung 0 (Compile):  98.2% pass rate, 142 observations, 31 consecutive passes [SKIP ADVISORY]
  Rung 1 (Lint):     87.5% pass rate, 130 observations, 8 consecutive passes
  Rung 2 (Test):     72.1% pass rate, 118 observations, 3 consecutive passes
  Rung 3 (Symbol):   95.0% pass rate, 45 observations, 15 consecutive passes
```

---

## 9. Relationship to the GVU Framework

The adaptive thresholds are a practical implementation of the GVU framework's guidance
on verification investment. The framework proves that stronger verifiers yield better
self-improvement. The thresholds operationalize this by:

1. **Allocating more retries** to gates with low pass rates (investing more in
   verification where it's most needed).
2. **Reducing retries** for gates with high pass rates (not wasting resources on
   verification that's already reliable).
3. **Skipping gates** that are essentially always passing (redirecting verification
   budget to where it matters).

This is adaptive resource allocation for verification — a concrete instance of the GVU
insight that verification quality matters more than generation quality.

> **Citation**: Song et al. (ICLR 2025) — GVU framework, verification-first investment
> strategy.

---

## 10. Testing

| Test | Property |
|---|---|
| `new_rung_starts_neutral` | Unknown rung → default 3 retries, no skip |
| `high_pass_rate_reduces_retries` | ~100% pass rate → 1 retry |
| `low_pass_rate_increases_retries` | ~0% pass rate → 5 retries |
| `consecutive_passes_trigger_skip` | 20 consecutive → skip advisory |
| `failure_resets_skip_streak` | One failure → no skip advisory |
| `round_trip_persistence` | Save/load preserves state |

> **Citation**: crates/roko-gate/src/adaptive_threshold.rs — Tests section.

---

## 11. Statistical Process Control (SPC) Extensions

The current EMA provides a smoothed estimate of pass rates. Statistical Process Control
adds formal anomaly detection — distinguishing true process changes from random
fluctuation. Three complementary SPC methods detect different kinds of shifts.

> **Citation**: "Improved adaptive CUSUM control chart for industrial process monitoring"
> (Nature Scientific Reports, 2025).

### 11.1 CUSUM (Cumulative Sum) for Sustained Shifts

CUSUM detects small, sustained changes in gate pass rates that the EMA might smooth
over. A gate whose pass rate drifts from 90% to 80% over 20 runs may not trigger an
alert in EMA but will accumulate signal in CUSUM.

```rust
/// CUSUM detector for sustained shifts in gate pass rates.
///
/// Tracks cumulative departures from target in both directions.
/// Signals when accumulated drift exceeds the decision interval.
pub struct CusumDetector {
    /// Reference value (slack parameter). Typically 0.5 * delta where
    /// delta is the shift size to detect in standard deviation units.
    /// Default: 0.25 (detects ~0.5σ shifts in pass rate).
    pub k: f64,
    /// Decision interval. Higher = fewer false alarms, slower detection.
    /// Default: 4.0 (ARL₀ ≈ 168 observations before false alarm).
    pub h: f64,
    /// Target pass rate (process mean under normal operation).
    /// Updated from historical data or set from EMA baseline.
    pub mu_0: f64,
    /// Process standard deviation estimate.
    /// For binary pass/fail: σ = sqrt(p * (1-p)).
    pub sigma: f64,
    /// Upper CUSUM accumulator (detects upward shift — improving).
    pub c_plus: f64,
    /// Lower CUSUM accumulator (detects downward shift — degrading).
    pub c_minus: f64,
    /// Whether a shift has been detected.
    pub shift_detected: bool,
    /// Direction of detected shift.
    pub shift_direction: Option<ShiftDirection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftDirection {
    /// Pass rate is improving (higher than target).
    Improving,
    /// Pass rate is degrading (lower than target).
    Degrading,
}

impl CusumDetector {
    /// Update with a new observation.
    ///
    /// value: 1.0 for pass, 0.0 for fail.
    pub fn update(&mut self, value: f64) {
        let z = (value - self.mu_0) / self.sigma; // standardize

        // Detect upward shift (improving)
        self.c_plus = (self.c_plus + z - self.k).max(0.0);
        // Detect downward shift (degrading)
        self.c_minus = (self.c_minus - z - self.k).max(0.0);

        if self.c_plus > self.h {
            self.shift_detected = true;
            self.shift_direction = Some(ShiftDirection::Improving);
            self.c_plus = self.h / 2.0; // Fast Initial Response (FIR) reset
        } else if self.c_minus > self.h {
            self.shift_detected = true;
            self.shift_direction = Some(ShiftDirection::Degrading);
            self.c_minus = self.h / 2.0; // FIR reset
        } else {
            self.shift_detected = false;
            self.shift_direction = None;
        }
    }
}
```

**Parameters**:

| Parameter | Default | Range | Effect |
|---|---|---|---|
| `k` (reference value) | 0.25 | 0.1–1.0 | Lower = more sensitive to small shifts |
| `h` (decision interval) | 4.0 | 2.0–8.0 | Lower = faster detection, more false alarms |
| FIR reset | `h/2` | — | Halves accumulator on signal, enabling re-detection |

### 11.2 EWMA Control Chart

Extends the existing EMA with formal control limits. The current implementation tracks
`ema_pass_rate` but has no formal bounds for when that rate is "out of control."

```rust
/// EWMA control chart with time-varying control limits.
///
/// Adds formal upper/lower control limits (UCL/LCL) to the existing
/// EMA pass rate tracking. When the EMA crosses a limit, the gate
/// is flagged as out-of-control.
pub struct EwmaControlChart {
    /// Smoothing factor (same as EMA_ALPHA). Range: [0.05, 0.25].
    /// Lower = more memory, detects smaller shifts.
    pub lambda: f64,      // default: 0.10
    /// Control limit width in sigma units. Range: [2.5, 3.5].
    /// Wider = fewer false alarms.
    pub l_factor: f64,    // default: 2.814
    /// Target mean (established from historical data).
    pub mu_0: f64,
    /// Process standard deviation.
    pub sigma: f64,
    /// Current EWMA value (= ema_pass_rate from RungStats).
    pub z: f64,
    /// Number of observations (for time-varying limit computation).
    pub n: u64,
}

impl EwmaControlChart {
    /// Compute the current upper and lower control limits.
    ///
    /// Time-varying: limits are wider early (few observations) and
    /// converge to steady-state as n grows.
    pub fn control_limits(&self) -> (f64, f64) {
        let asymptotic_var = self.lambda / (2.0 - self.lambda);
        let time_factor = 1.0 - (1.0 - self.lambda).powi(2 * self.n as i32);
        let sigma_z = self.sigma * (asymptotic_var * time_factor).sqrt();

        let ucl = self.mu_0 + self.l_factor * sigma_z;
        let lcl = self.mu_0 - self.l_factor * sigma_z;
        (lcl.max(0.0), ucl.min(1.0)) // clamp to [0, 1] for pass rates
    }

    /// Check if the current EWMA is within control limits.
    pub fn is_in_control(&self) -> bool {
        let (lcl, ucl) = self.control_limits();
        self.z >= lcl && self.z <= ucl
    }
}
```

**ARL (Average Run Length) Tuning**:

| λ | L | ARL₀ (in-control) | ARL₁ (1σ shift) | Best for |
|---|---|---|---|---|
| 0.05 | 2.625 | ~500 | ~26 | Small persistent drifts |
| 0.10 | 2.814 | ~500 | ~31 | **Balanced (default)** |
| 0.20 | 2.962 | ~500 | ~41 | Larger sudden shifts |

ARL₀ ~ 500 means one false alarm per ~500 observations. ARL₁ ~ 31 means a true
1σ shift is detected in ~31 observations on average.

### 11.3 BOCPD (Bayesian Online Change Point Detection)

When EMA and CUSUM detect a shift, BOCPD provides a probabilistic answer to "did the
gate's fundamental behavior change?" This is critical after major refactors, dependency
updates, or model switches, where the baseline itself shifts.

> **Citation**: Adams & MacKay, "Bayesian Online Changepoint Detection" (arXiv:0710.3742,
> 2007).

```rust
/// Bayesian Online Change Point Detection.
///
/// Maintains a posterior distribution over run lengths (time since last
/// change point). When P(run_length = 0) spikes, a change point has
/// occurred and the gate baseline should be recalibrated.
pub struct BocpdDetector {
    /// Prior probability of a change point at each step.
    /// Lower = fewer expected change points (more stable process).
    /// Default: 1/200 (expect one change point per 200 observations).
    pub hazard_rate: f64,
    /// Maximum run length to track (truncation for O(R_max) per step).
    pub max_run_length: usize,  // default: 300
    /// Run-length posterior probabilities.
    pub run_length_probs: Vec<f64>,
    /// Sufficient statistics for the underlying model (Normal-Gamma
    /// conjugate for Gaussian observations).
    pub sufficient_stats: Vec<NormalGammaStats>,
    /// Threshold for declaring a change point.
    /// When P(r=0) > threshold, a change point is declared.
    pub changepoint_threshold: f64,  // default: 0.5
}

#[derive(Debug, Clone)]
pub struct NormalGammaStats {
    pub mu: f64,     // posterior mean
    pub kappa: f64,  // pseudo-observations for mean
    pub alpha: f64,  // shape for variance
    pub beta: f64,   // rate for variance
}

impl BocpdDetector {
    /// Update with a new observation and return whether a change point was detected.
    pub fn update(&mut self, value: f64) -> bool {
        // 1. Compute predictive probability for each run length
        let predictive: Vec<f64> = self.sufficient_stats.iter()
            .map(|s| s.predictive_probability(value))
            .collect();

        // 2. Growth probabilities (no change point)
        let growth: Vec<f64> = self.run_length_probs.iter()
            .zip(predictive.iter())
            .map(|(p, pi)| p * pi * (1.0 - self.hazard_rate))
            .collect();

        // 3. Change-point probability (run length resets to 0)
        let changepoint_mass: f64 = self.run_length_probs.iter()
            .zip(predictive.iter())
            .map(|(p, pi)| p * pi * self.hazard_rate)
            .sum();

        // 4. Build new posterior
        let mut new_probs = vec![changepoint_mass];
        new_probs.extend(growth.iter().take(self.max_run_length - 1));

        // 5. Normalize
        let total: f64 = new_probs.iter().sum();
        if total > 0.0 {
            for p in &mut new_probs {
                *p /= total;
            }
        }

        // 6. Update sufficient statistics for each run length
        // (extend by one entry for r=0, update existing entries)
        self.update_sufficient_stats(value);

        self.run_length_probs = new_probs;

        // 7. Detect change point
        self.run_length_probs[0] > self.changepoint_threshold
    }
}
```

**Parameters**:

| Parameter | Default | Range | Effect |
|---|---|---|---|
| `hazard_rate` | 1/200 | 1/50 – 1/1000 | Prior on change frequency |
| `max_run_length` | 300 | 50–1000 | Truncation depth (memory vs accuracy) |
| `changepoint_threshold` | 0.5 | 0.3–0.8 | Sensitivity to regime changes |

**When to recalibrate**: When BOCPD detects a change point, the system should:
1. Reset the CUSUM accumulators to zero
2. Update the EWMA target mean (μ₀) to the post-changepoint EMA
3. Log a regime-change event to `.roko/learn/efficiency.jsonl`
4. Optionally notify the dashboard

---

## 12. Multi-Gate Threshold Coordination

When one gate's behavior changes, should other gates adjust? The answer is yes —
gates are not independent. A compile time increase often precedes test flakiness.
A coverage drop in lint gates correlates with more test failures.

### 12.1 The Coordination Problem

Consider: the test gate's pass rate drops from 90% to 60%. The adaptive threshold
increases test retries from 2 to 4. But the *reason* tests are failing is that the
compile gate is letting through code with subtle type errors that the linter would
catch. The correct response isn't more test retries — it's tighter lint enforcement.

Independent threshold adjustment misses these cross-gate correlations.

### 12.2 Hotelling's T² for Multi-Gate Anomaly Detection

```rust
/// Multi-gate anomaly detector using Hotelling's T-squared statistic.
///
/// Monitors the joint distribution of gate metrics (pass rates, durations,
/// scores) across all rungs simultaneously. Detects correlated anomalies
/// that per-gate monitors miss.
pub struct MultiGateDetector {
    /// Number of metrics being tracked (one per gate).
    pub p: usize,
    /// Historical mean vector (one entry per gate's pass rate).
    pub mu: Vec<f64>,
    /// Inverse covariance matrix (p × p).
    /// Captures inter-gate correlations.
    pub sigma_inv: Vec<Vec<f64>>,
    /// Chi-squared critical value for the chosen alpha.
    /// Default alpha=0.01, p=7 gates → threshold ≈ 18.48.
    pub threshold: f64,
    /// Minimum observations before monitoring begins.
    pub warmup_period: u64,   // default: 30
    /// Current observation count.
    pub observation_count: u64,
}

impl MultiGateDetector {
    /// Feed a new observation vector (one pass rate per gate) and check
    /// for multi-gate anomaly.
    pub fn observe(&mut self, x: &[f64]) -> Option<MultiGateAnomaly> {
        self.observation_count += 1;
        if self.observation_count < self.warmup_period {
            self.update_statistics(x);
            return None;
        }

        // Compute T² = (x - μ)ᵀ Σ⁻¹ (x - μ)
        let diff: Vec<f64> = x.iter().zip(self.mu.iter())
            .map(|(xi, mi)| xi - mi)
            .collect();
        let t_squared = self.quadratic_form(&diff, &self.sigma_inv);

        if t_squared > self.threshold {
            // Identify which gate(s) are contributing most to the anomaly
            let contributions = self.per_gate_contributions(&diff);
            Some(MultiGateAnomaly {
                t_squared,
                threshold: self.threshold,
                primary_gates: contributions,
            })
        } else {
            self.update_statistics(x);
            None
        }
    }
}

pub struct MultiGateAnomaly {
    /// The T² statistic value.
    pub t_squared: f64,
    /// The threshold that was exceeded.
    pub threshold: f64,
    /// Gates ranked by their contribution to the anomaly,
    /// with attribution scores.
    pub primary_gates: Vec<(usize, f64)>,
}
```

### 12.3 Coordination Policies

When a multi-gate anomaly is detected, a coordination policy determines the response:

```rust
pub enum CoordinationPolicy {
    /// Independent: each gate adjusts thresholds independently.
    /// This is the current behavior and the default.
    Independent,

    /// Sympathetic: when a downstream gate degrades, upstream gates tighten.
    /// Example: test failures → tighten compile/lint gates.
    Sympathetic {
        /// How much to tighten upstream gates when downstream fails.
        /// 0.0 = no tightening, 1.0 = maximum tightening.
        tightening_factor: f64, // default: 0.3
    },

    /// Compensatory: when one gate relaxes (high pass rate), neighboring
    /// gates tighten to maintain overall verification strength.
    /// Conserves total verification investment.
    Compensatory {
        /// Target aggregate verification score across all gates.
        target_aggregate: f64, // default: 0.85
    },

    /// Diagnostic: on anomaly, run additional diagnostic gates that are
    /// normally skipped, to identify root cause.
    Diagnostic {
        /// Additional gates to activate on anomaly.
        diagnostic_gates: Vec<Box<dyn Gate>>,
    },
}
```

### 12.4 Sympathetic Tightening Example

```
Gate pass rates at T=100:
  Compile: 98%  →  retry budget: 1
  Lint:    90%  →  retry budget: 2
  Test:    85%  →  retry budget: 2

Gate pass rates at T=120 (test degrades):
  Compile: 97%  →  retry budget: 1
  Lint:    88%  →  retry budget: 2
  Test:    60%  →  retry budget: 4

Multi-gate anomaly detected (T² = 22.1 > 18.48 threshold).
Primary contributor: Test gate.

Sympathetic response (tightening_factor=0.3):
  Compile: tighten → enable --all-targets flag, add 1 retry
  Lint:    tighten → enable -D warnings (deny all warnings)
  Test:    increase retries as normal

Rationale: if tests are failing more, tighter upstream gates catch problems
earlier and cheaper, reducing the load on the expensive test gate.
```

---

## 13. Domain-Specific Threshold Profiles

Different agent roles (code writer, test writer, documentation, infra) have different
gate characteristics. A compile gate that passes 99% of the time for a test-writer
agent might pass only 80% of the time for a complex refactoring agent.

### 13.1 Profile Structure

```rust
/// Pre-configured threshold profile for a specific agent role or domain.
pub struct ThresholdProfile {
    /// Profile identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Initial pass rate priors per rung.
    /// Used instead of the neutral 0.5 prior for cold-start.
    pub initial_priors: HashMap<u32, f64>,
    /// Per-rung EMA alpha overrides.
    /// Some domains need faster adaptation (higher α).
    pub alpha_overrides: HashMap<u32, f64>,
    /// Per-rung retry budget overrides [min, max].
    pub retry_bounds: HashMap<u32, (u32, u32)>,
    /// Per-rung skip streak threshold overrides.
    pub skip_thresholds: HashMap<u32, u32>,
    /// CUSUM parameters per rung.
    pub cusum_params: HashMap<u32, CusumParams>,
}

#[derive(Debug, Clone)]
pub struct CusumParams {
    pub k: f64,
    pub h: f64,
}
```

### 13.2 Built-In Profiles

```rust
/// Profiles for common agent roles.
pub fn profile_for_role(role: &str) -> ThresholdProfile {
    match role {
        "code-writer" => ThresholdProfile {
            name: "Code Writer".into(),
            initial_priors: hashmap! {
                0 => 0.85, // compile: most generated code compiles
                1 => 0.75, // lint: often has warnings
                2 => 0.60, // test: tests frequently break
                3 => 0.90, // symbol: symbol checks are reliable
            },
            alpha_overrides: hashmap! {
                2 => 0.15, // test gate: adapt faster (more volatile)
            },
            retry_bounds: hashmap! {
                2 => (2, 6), // tests: more retries allowed
            },
            ..Default::default()
        },
        "test-writer" => ThresholdProfile {
            name: "Test Writer".into(),
            initial_priors: hashmap! {
                0 => 0.95, // compile: test code almost always compiles
                1 => 0.90, // lint: test code is cleaner
                2 => 0.50, // test: new tests often need iteration
            },
            retry_bounds: hashmap! {
                2 => (3, 8), // tests: expect more iteration
            },
            ..Default::default()
        },
        "refactoring" => ThresholdProfile {
            name: "Refactoring".into(),
            initial_priors: hashmap! {
                0 => 0.70, // compile: refactors break things
                1 => 0.65, // lint: type changes cascade
                2 => 0.55, // test: existing tests may break
                3 => 0.80, // symbol: symbols shift during refactors
            },
            alpha_overrides: hashmap! {
                0 => 0.20, // compile: adapt very fast during refactors
                1 => 0.20,
            },
            cusum_params: hashmap! {
                0 => CusumParams { k: 0.15, h: 3.0 }, // more sensitive
            },
            ..Default::default()
        },
        _ => ThresholdProfile::default(),
    }
}
```

### 13.3 Profile Selection

The orchestrator selects a profile based on the task description and plan metadata:

```
Task: "Refactor auth module to use trait objects"
  → Keywords: "refactor" → profile: "refactoring"
  → Initial compile prior: 0.70 (instead of neutral 0.50)
  → CUSUM sensitivity increased

Task: "Add unit tests for the parser"
  → Keywords: "test" → profile: "test-writer"
  → Initial test prior: 0.50 (expects iteration)
  → More retries for test gate
```

---

## 14. Change-Point Detection Integration

CUSUM, EWMA control charts, and BOCPD work together in a hierarchy:

```
Per gate observation (pass/fail)
    │
    ├── EMA update (existing) ─── smoothed pass rate
    │
    ├── CUSUM update ─── sustained shift detection
    │    └── shift detected? → adjust retry budget more aggressively
    │
    ├── EWMA control chart ─── formal anomaly detection
    │    └── out of control? → flag gate in dashboard, notify conductor
    │
    └── BOCPD update ─── regime change detection
         └── change point? → recalibrate baselines for all detectors
```

### 14.1 Offline Batch Analysis with PELT

For retrospective analysis (e.g., "when did our test reliability degrade?"), the
PELT algorithm finds optimal change points in historical gate data:

> **Citation**: Killick et al., "Optimal Detection of Changepoints with a Linear
> Computational Cost" (arXiv:1101.1438, 2012).

```rust
/// Offline change-point detection using PELT (Pruned Exact Linear Time).
///
/// Finds all points where the gate's statistical properties changed.
/// Used for retrospective analysis, not online monitoring.
pub struct PeltDetector {
    /// Cost function for a segment of observations.
    /// Default: negative log-likelihood for Gaussian data.
    pub cost: CostFunction,
    /// Penalty term controlling number of change points.
    /// BIC: p * ln(n), where p = parameters, n = observations.
    /// Higher penalty = fewer change points detected.
    pub penalty: f64,
    /// Minimum segment length between change points.
    pub min_segment: usize,  // default: 5
}

pub enum CostFunction {
    /// Gaussian negative log-likelihood (for continuous scores).
    Gaussian,
    /// Bernoulli negative log-likelihood (for binary pass/fail).
    Bernoulli,
}

impl PeltDetector {
    /// Find all change points in a historical sequence.
    ///
    /// Returns indices where the process changed.
    /// Complexity: O(n) expected with pruning.
    pub fn detect(&self, data: &[f64]) -> Vec<usize> {
        let n = data.len();
        let mut f = vec![0.0_f64; n + 1];
        f[0] = -self.penalty;
        let mut cp = vec![Vec::new(); n + 1];
        let mut candidates: Vec<usize> = vec![0];

        for t in 1..=n {
            let mut best_cost = f64::MAX;
            let mut best_tau = 0;

            for &tau in &candidates {
                if t - tau < self.min_segment { continue; }
                let segment_cost = self.cost.compute(&data[tau..t]);
                let total = f[tau] + segment_cost + self.penalty;
                if total < best_cost {
                    best_cost = total;
                    best_tau = tau;
                }
            }

            f[t] = best_cost;
            cp[t] = cp[best_tau].clone();
            cp[t].push(best_tau);

            // Pruning: remove candidates that can never be optimal
            candidates.retain(|&tau| {
                f[tau] + self.cost.compute(&data[tau..t]) <= f[t]
            });
            candidates.push(t);
        }

        cp[n].clone()
    }
}
```

### 14.2 Retrospective Report

```
PELT analysis of Test gate pass rates (last 500 observations):

Change points detected at observations: [47, 183, 312]

Segment 1 (obs 0-47):   mean pass rate 0.92 ± 0.04  [stable, healthy]
Segment 2 (obs 48-183):  mean pass rate 0.71 ± 0.08  [regression, likely caused by auth refactor]
Segment 3 (obs 184-312): mean pass rate 0.88 ± 0.05  [recovery after fix batch]
Segment 4 (obs 313-500): mean pass rate 0.82 ± 0.06  [current regime, moderate]

Recommendation: current regime is below segment 1 baseline. Consider targeted
testing improvements for the code patterns introduced since observation 312.
```

---

## 15. Enhanced RungStats Structure

The SPC extensions require additional per-rung state:

```rust
/// Extended per-rung statistics with SPC monitoring.
pub struct RungStatsExtended {
    // --- Existing fields ---
    pub ema_pass_rate: f64,
    pub total_observations: u64,
    pub consecutive_passes: u32,

    // --- CUSUM ---
    pub cusum: CusumDetector,

    // --- EWMA control chart ---
    pub ewma_chart: EwmaControlChart,

    // --- BOCPD ---
    pub bocpd: BocpdDetector,

    // --- Metadata ---
    /// Timestamp of last observation (for decay calculations).
    pub last_observation_ms: u64,
    /// Number of regime changes detected (lifetime).
    pub regime_changes: u32,
    /// Current regime start observation index.
    pub current_regime_start: u64,
}

impl RungStatsExtended {
    /// Update all detectors with a new observation.
    pub fn observe(&mut self, passed: bool, timestamp_ms: u64) {
        let value = if passed { 1.0 } else { 0.0 };

        // Existing EMA update
        self.update_ema(value);
        self.update_consecutive(passed);

        // SPC updates
        self.cusum.update(value);
        self.ewma_chart.update(value);
        let changepoint = self.bocpd.update(value);

        // On regime change, recalibrate everything
        if changepoint {
            self.regime_changes += 1;
            self.current_regime_start = self.total_observations;
            self.cusum.reset();
            self.ewma_chart.recalibrate(self.ema_pass_rate);
        }

        self.last_observation_ms = timestamp_ms;
        self.total_observations += 1;
    }

    /// Comprehensive health assessment for this rung.
    pub fn health(&self) -> RungHealth {
        RungHealth {
            pass_rate: self.ema_pass_rate,
            in_control: self.ewma_chart.is_in_control(),
            shift_detected: self.cusum.shift_detected,
            shift_direction: self.cusum.shift_direction,
            recent_changepoint: self.bocpd.run_length_probs[0]
                > self.bocpd.changepoint_threshold * 0.5,
            suggested_retries: self.suggested_max_retries(),
            should_skip: self.should_skip(),
        }
    }
}

pub struct RungHealth {
    pub pass_rate: f64,
    pub in_control: bool,
    pub shift_detected: bool,
    pub shift_direction: Option<ShiftDirection>,
    pub recent_changepoint: bool,
    pub suggested_retries: u32,
    pub should_skip: bool,
}
```

---

## 16. Test Criteria for SPC Extensions

| Test | Property |
|---|---|
| `cusum_detects_sustained_drop` | 20 observations at 90%, then 20 at 70% → shift detected |
| `cusum_ignores_noise` | Random fluctuations around 80% → no shift signal |
| `cusum_fir_reset` | After detection, accumulator resets to h/2, can re-detect |
| `ewma_control_limits_converge` | UCL/LCL stabilize after ~50 observations |
| `ewma_flags_out_of_control` | Pass rate 90% → suddenly 50% → flagged within 10 obs |
| `bocpd_detects_regime_change` | 100 obs at 90%, then 100 at 60% → change point at ~100 |
| `bocpd_no_false_alarm` | Stable 85% pass rate for 500 obs → no change point |
| `pelt_finds_known_changepoints` | Synthetic data with breaks at [50, 150] → detected ±3 |
| `multi_gate_detects_correlated_drop` | Two gates degrade together → T² exceeds threshold |
| `sympathetic_tightening_triggers` | Downstream degradation → upstream retry budget decreases |
| `profile_cold_start` | New gate with profile prior starts at profile rate, not 0.5 |
| `regime_change_recalibrates` | BOCPD changepoint → CUSUM reset + EWMA target update |
