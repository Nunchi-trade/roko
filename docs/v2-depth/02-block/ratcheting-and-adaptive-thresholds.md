# Ratcheting and Adaptive Thresholds

> Depth for [02-CELL.md](../../unified/02-CELL.md). Quality ratcheting as a monotonic constraint on Verify verdicts. Adaptive thresholds as a calibration Loop: EMA per rung, adjusted by gate pass/fail rates, with SPC extensions for regime detection.

---

## 1. The GateRatchet: A One-Way Valve

The GateRatchet prevents verification regression. Once a plan passes rung N, it should never regress to rung N-1. This solves **convergence thrashing** -- a specific failure mode in multi-attempt agent loops.

### 1.1 The Thrashing Problem

```
Attempt 1: Compile PASS, Lint FAIL
  Agent receives: "warning: unused variable"
  Agent fixes lint issue

Attempt 2: Compile FAIL (lint fix introduced a type error)
  Agent receives: "error[E0308]: mismatched types"
  Agent fixes type error

Attempt 3: Compile PASS, Lint FAIL (type fix reintroduced lint issue)
  Agent receives: "warning: unused variable"
  ... (infinite oscillation)
```

Each attempt passes one rung and fails the next. The agent is modifying code but not making progress. The net verification state oscillates.

### 1.2 Data Structure

```rust
pub struct GateRatchet {
    passes: HashMap<String, u8>,  // plan_id -> highest rung passed
}
```

A map from plan identifier to the highest rung number (u8) that plan has passed. Seven rungs fit in 3 bits. The structure is minimal and correct by construction.

### 1.3 The Monotonic Property

The stored value for any plan ID can only increase or stay the same. It never decreases.

```rust
pub fn record_pass(&mut self, plan_id: impl Into<String>, rung: u8) {
    let entry = self.passes.entry(plan_id.into()).or_insert(0);
    if rung > *entry {
        *entry = rung;
    }
}

pub fn can_regress(&self, plan_id: &str, rung: u8) -> bool {
    match self.passes.get(plan_id) {
        None => true,                     // unknown plan: no constraint
        Some(&highest) => rung >= highest, // OK if same or higher
    }
}
```

`record_pass` only advances the watermark. `can_regress` returns `false` when accepting the proposed rung would be a regression (plan already passed a strictly higher rung).

### 1.4 Ratchet + Escalation = Monotonically Advancing Frontier

The ratchet and escalation serve complementary purposes:

| Mechanism | Direction | Purpose |
|---|---|---|
| Escalation | Forward (adds rungs) | Failed -> try harder |
| Ratchet | Backward (blocks regression) | Passed -> do not lose progress |

Together they create an advancing frontier:

```
Attempt 1: Trivial -> [Compile]
  Compile PASS -> ratchet records rung 0
  -> escalate to Simple

Attempt 2: Simple -> [Compile, Lint]
  Compile must still pass (ratchet enforces rung 0)
  Lint PASS -> ratchet records rung 1
  -> escalate to Standard

Attempt 3: Standard -> [Compile, Lint, Test, Symbol]
  Compile must still pass (ratchet)
  Lint must still pass (ratchet)
  Test PASS -> ratchet records rung 2
  -> task verified
```

### 1.5 Per-Plan Isolation

Each plan has its own ratchet entry. Plan A's progress has no effect on Plan B. The `HashMap<String, u8>` keying on plan ID provides this isolation naturally.

### 1.6 Ratchet as a Simple Process Reward

The ratchet is a simple form of process reward (see [process-reward-and-artifacts.md](process-reward-and-artifacts.md)): it tracks intermediate verification progress, not just final outcomes. A plan at Rung 3 has demonstrated more progress than one at Rung 1. This data feeds:

- **Promise score**: How likely is this plan to eventually pass all rungs, given it has passed rung N?
- **Progress score**: Is the plan advancing (higher rungs on successive attempts) or stalling?

### 1.7 Edge Cases

**Rung 0 ratchet**: If a plan passes Rung 0, the ratchet records `highest = 0`. `can_regress("plan", 0)` returns `true` (0 >= 0). There is no rung below 0, so a plan that passed Compile can fail Compile on a subsequent attempt without the ratchet blocking it. The regression check fires only when attempting a rung strictly below the recorded highest.

**Full pipeline pass**: When a plan passes all 7 rungs, `highest_pass` = 6. Any subsequent failure at any rung is a regression (0-5 are all below 6).

### 1.8 Future: Persistent Ratchet

The current ratchet is in-memory. For resumable executions, it should persist to `.roko/state/gate-ratchet.json` and reload on `--resume`, alongside the existing executor snapshot.

---

## 2. Adaptive Thresholds: A Calibration Loop

Adaptive thresholds tune verification behavior based on historical pass rates. They form a Loop (see [03-GRAPH.md](../../unified/03-GRAPH.md) S4): predict-publish-correct applied to gate behavior.

### 2.1 Per-Rung Statistics

```rust
pub struct RungStats {
    pub ema_pass_rate: f64,       // exponential moving average [0.0, 1.0]
    pub total_observations: u64,  // total gate runs for this rung
    pub consecutive_passes: u32,  // consecutive passes (reset on any failure)
}
```

Fresh rungs start at `ema_pass_rate = 0.5` (neutral prior), `total_observations = 0`, `consecutive_passes = 0`.

### 2.2 The EMA Update Rule

```rust
const EMA_ALPHA: f64 = 0.1;

pub fn update(&mut self, rung: u32, passed: bool) {
    let stats = self.rungs.entry(rung).or_default();
    let value = if passed { 1.0 } else { 0.0 };

    if stats.total_observations == 0 {
        stats.ema_pass_rate = value;  // first observation sets rate directly
    } else {
        stats.ema_pass_rate = EMA_ALPHA * value + (1.0 - EMA_ALPHA) * stats.ema_pass_rate;
    }
    stats.total_observations += 1;

    if passed { stats.consecutive_passes += 1; }
    else { stats.consecutive_passes = 0; }
}
```

**Why EMA with alpha=0.1**: Effective memory ~10 observations. Balances responsiveness with stability. A gate that fails once should not immediately triple the retry budget, but a gate that fails 5 times in a row should.

| alpha | Effective window | Behavior |
|---|---|---|
| 0.01 | ~100 observations | Very stable, slow to adapt |
| **0.10** | **~10 observations** | **Balanced (default)** |
| 0.30 | ~3 observations | Responsive, potentially noisy |

### 2.3 Two Advisory Signals

The adaptive thresholds produce two advisory signals (advisory, not enforced -- the orchestrator decides):

**Retry budget**:

```rust
pub fn suggested_max_retries(&self, rung: u32) -> u32 {
    let stats = self.rungs.get(&rung);
    if stats.is_none() || stats.unwrap().total_observations < 5 {
        return 3; // cold start default
    }
    // Linear map: high pass rate -> low retries, low pass rate -> high retries
    // Pass rate 1.0 -> 1 retry, 0.5 -> 3, 0.0 -> 5
    let retries = stats.ema_pass_rate.mul_add(-4.0, 5.0).round() as u32;
    retries.clamp(1, 5)  // MIN_RETRIES=1, MAX_RETRIES=5
}
```

**Skip advisory**:

```rust
const SKIP_STREAK_THRESHOLD: u32 = 20;

pub fn should_skip_rung(&self, rung: u32) -> bool {
    self.rungs.get(&rung)
        .is_some_and(|s| s.consecutive_passes >= SKIP_STREAK_THRESHOLD)
}
```

Twenty consecutive passes means the gate has not failed in at least 20 executions. The advisory is not enforced: the orchestrator can skip 90% of the time for speed, run every 5th execution to verify, and reset the counter on any failure.

### 2.4 The Calibration Loop

The adaptive threshold system forms a Loop:

```
Gate execution (observation)
    |
    v
EMA update (predict: current pass rate)
    |
    v
Retry budget / skip advisory (publish: recommendation)
    |
    v
Orchestrator acts on recommendation
    |
    v
Next gate execution (correct: actual outcome updates EMA)
    |
    +--> back to top
```

This is predict-publish-correct ([02-CELL.md](../../unified/02-CELL.md) S8) applied to verification intensity. The system predicts (via EMA) how likely each rung is to pass, publishes a recommendation (retry budget, skip advisory), and corrects based on the actual outcome.

### 2.5 Persistence

```rust
pub fn save(&self, path: &Path) -> Result<(), io::Error> {
    let json = serde_json::to_string_pretty(self)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &json)?;
    fs::rename(&tmp, path)?;  // atomic write
    Ok(())
}

pub fn load_or_new(path: &Path) -> Self {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()  // graceful degradation on missing/corrupt
}
```

Persists to `.roko/learn/gate-thresholds.json` alongside cascade-router, experiments, and efficiency data. Atomic write (write to tmp, rename) ensures no half-written state. Graceful degradation: missing or corrupt file produces a fresh instance.

### 2.6 Interaction with Rung Selection

The rung selector determines the baseline set of rungs (static, based on complexity). Adaptive thresholds refine it (dynamic, based on history):

```
Static selection:  complexity -> rungs [0, 1, 2, 3]
Adaptive refinement:
  Rung 0: 25 consecutive passes -> skip advisory
  Rung 1: 3 consecutive passes -> run normally
  Rung 2: 12 consecutive passes -> run normally
  Rung 3: not tracked yet -> run with default retries (3)
Final: rungs [1, 2, 3] with rung 0 skipped
```

---

## 3. Statistical Process Control (SPC) Extensions

The EMA provides a smoothed pass rate estimate. SPC adds formal anomaly detection -- distinguishing true process changes from random fluctuation.

### 3.1 CUSUM (Cumulative Sum) for Sustained Shifts

CUSUM detects small, sustained changes that EMA might smooth over. A gate whose pass rate drifts from 90% to 80% over 20 runs accumulates signal in CUSUM.

```rust
pub struct CusumDetector {
    pub k: f64,           // reference value (default: 0.25, detects ~0.5sigma shifts)
    pub h: f64,           // decision interval (default: 4.0, ARL0 ~ 168)
    pub mu_0: f64,        // target pass rate
    pub sigma: f64,       // process std dev (for binary: sqrt(p*(1-p)))
    pub c_plus: f64,      // upper accumulator (improving)
    pub c_minus: f64,     // lower accumulator (degrading)
    pub shift_detected: bool,
    pub shift_direction: Option<ShiftDirection>,
}

impl CusumDetector {
    pub fn update(&mut self, value: f64) {
        let z = (value - self.mu_0) / self.sigma;
        self.c_plus = (self.c_plus + z - self.k).max(0.0);
        self.c_minus = (self.c_minus - z - self.k).max(0.0);

        if self.c_plus > self.h {
            self.shift_detected = true;
            self.shift_direction = Some(ShiftDirection::Improving);
            self.c_plus = self.h / 2.0; // Fast Initial Response reset
        } else if self.c_minus > self.h {
            self.shift_detected = true;
            self.shift_direction = Some(ShiftDirection::Degrading);
            self.c_minus = self.h / 2.0;
        } else {
            self.shift_detected = false;
            self.shift_direction = None;
        }
    }
}
```

### 3.2 EWMA Control Chart

Extends the existing EMA with formal upper/lower control limits (UCL/LCL). When the EMA crosses a limit, the gate is flagged as out-of-control.

```rust
pub struct EwmaControlChart {
    pub lambda: f64,    // smoothing factor (default: 0.10)
    pub l_factor: f64,  // control limit width in sigma (default: 2.814)
    pub mu_0: f64,      // target mean
    pub sigma: f64,     // process std dev
    pub z: f64,         // current EWMA value
    pub n: u64,         // observations
}

impl EwmaControlChart {
    pub fn control_limits(&self) -> (f64, f64) {
        let asymptotic_var = self.lambda / (2.0 - self.lambda);
        let time_factor = 1.0 - (1.0 - self.lambda).powi(2 * self.n as i32);
        let sigma_z = self.sigma * (asymptotic_var * time_factor).sqrt();
        let ucl = self.mu_0 + self.l_factor * sigma_z;
        let lcl = self.mu_0 - self.l_factor * sigma_z;
        (lcl.max(0.0), ucl.min(1.0))
    }

    pub fn is_in_control(&self) -> bool {
        let (lcl, ucl) = self.control_limits();
        self.z >= lcl && self.z <= ucl
    }
}
```

ARL tuning: lambda=0.10, L=2.814 gives ARL0~500 (one false alarm per ~500 observations), ARL1~31 (true 1-sigma shift detected in ~31 observations).

### 3.3 BOCPD (Bayesian Online Change Point Detection)

When EMA and CUSUM detect a shift, BOCPD provides a probabilistic answer to "did the gate's fundamental behavior change?" This is critical after major refactors, dependency updates, or model switches.

BOCPD maintains a posterior distribution over run lengths (time since last change point). When P(run_length=0) spikes, a change point has occurred and the gate baseline should be recalibrated.

Parameters: `hazard_rate` = 1/200 (expect one change point per 200 observations), `max_run_length` = 300, `changepoint_threshold` = 0.5.

On change point detection: reset CUSUM accumulators, update EWMA target mean to post-changepoint EMA, log regime-change event to `.roko/learn/efficiency.jsonl`.

### 3.4 PELT (Offline Batch Analysis)

For retrospective analysis ("when did test reliability degrade?"), the PELT algorithm (Killick et al. 2012) finds optimal change points in historical gate data with O(n) expected complexity via pruning.

```
PELT analysis of Test gate pass rates (last 500 observations):

Change points at: [47, 183, 312]
Segment 1 (0-47):    mean 0.92 +/- 0.04  [stable, healthy]
Segment 2 (48-183):  mean 0.71 +/- 0.08  [regression, likely auth refactor]
Segment 3 (184-312): mean 0.88 +/- 0.05  [recovery after fix batch]
Segment 4 (313-500): mean 0.82 +/- 0.06  [current regime, moderate]
```

### 3.5 The SPC Hierarchy

All detectors work together:

```
Per gate observation (pass/fail)
    |
    +-- EMA update (existing) ---------- smoothed pass rate
    |
    +-- CUSUM update ------------------- sustained shift detection
    |    shift detected? -> adjust retry budget more aggressively
    |
    +-- EWMA control chart ------------- formal anomaly detection
    |    out of control? -> flag gate in dashboard, notify conductor
    |
    +-- BOCPD update ------------------- regime change detection
         change point? -> recalibrate baselines for all detectors
```

---

## 4. Multi-Gate Threshold Coordination

Gates are not independent. A compile time increase often precedes test flakiness. A coverage drop in lint correlates with more test failures. Independent threshold adjustment misses these cross-gate correlations.

### 4.1 Hotelling's T-squared for Multi-Gate Anomaly Detection

Monitors the joint distribution of gate metrics across all rungs simultaneously. Detects correlated anomalies that per-gate monitors miss. Uses a chi-squared critical value (alpha=0.01, p=7 gates -> threshold ~ 18.48).

### 4.2 Coordination Policies

| Policy | Behavior |
|---|---|
| **Independent** (default) | Each gate adjusts independently |
| **Sympathetic** | Downstream degradation tightens upstream gates |
| **Compensatory** | When one gate relaxes, neighbors tighten to maintain aggregate strength |
| **Diagnostic** | On anomaly, activate additional diagnostic gates normally skipped |

**Sympathetic tightening example**: Test pass rate drops from 85% to 60%. Multi-gate anomaly detected. Sympathetic response: compile gate adds `--all-targets`, lint gate enforces `-D warnings`. Rationale: tighter upstream gates catch problems earlier and cheaper, reducing load on the expensive test gate.

### 4.3 Domain-Specific Threshold Profiles

Different agent roles have different gate characteristics. Profiles provide informed priors instead of the neutral 0.5 cold start.

| Profile | Compile prior | Test prior | EMA alpha override | Notes |
|---|---|---|---|---|
| **code-writer** | 0.85 | 0.60 | test: 0.15 | Most generated code compiles; tests frequently break |
| **test-writer** | 0.95 | 0.50 | -- | Test code compiles; new tests need iteration |
| **refactoring** | 0.70 | 0.55 | compile: 0.20, lint: 0.20 | Refactors break things; adapt very fast |

The orchestrator selects a profile based on task keywords and plan metadata.

---

## 5. The Ratchet-Threshold-SPC Composition

The three mechanisms compose into a layered verification control system:

```
Layer 1 (Ratchet): Monotonic constraint
  "You passed Rung 2. You cannot regress below Rung 2."
  Simple. Binary. HashMap<String, u8>.

Layer 2 (Adaptive thresholds): Resource allocation
  "Rung 2 has 85% pass rate. Give it 2 retries."
  "Rung 0 has 25 consecutive passes. Consider skipping."
  EMA-based. Responds in ~10 observations.

Layer 3 (SPC): Regime detection
  "Test gate pass rate shifted from 90% to 70% at observation 183."
  "Compile and test gates degrading simultaneously -- correlated anomaly."
  CUSUM + EWMA chart + BOCPD. Detects sustained process changes.
```

Each layer operates at a different timescale and provides different information. The ratchet is per-attempt. Adaptive thresholds are per-~10-observations. SPC is per-regime (tens to hundreds of observations).

---

## What This Enables

1. **No thrashing**: The ratchet eliminates oscillation between verification states. Progress is monotonic.

2. **Adaptive resource allocation**: Retry budgets scale automatically with gate reliability. Gates that always pass get fewer retries. Gates that often fail get more. No manual tuning.

3. **Early skip optimization**: Gates with 20+ consecutive passes can be skipped, saving seconds-to-minutes per task execution.

4. **Regime awareness**: SPC detects when gate behavior fundamentally changes (refactors, dependency updates, model switches) and triggers recalibration rather than accumulating stale statistics.

5. **Cross-gate coordination**: Sympathetic tightening catches upstream causes of downstream failures. The system tightens lint when tests degrade, catching problems earlier and cheaper.

---

## Feedback Loops

- **Ratchet -> Escalation**: When the ratchet blocks regression at rung N, the system knows the current attempt is failing at a previously-passed level. This triggers stronger escalation than a first-time failure.
- **Adaptive thresholds -> Retry loop**: `suggested_max_retries()` directly controls how many attempts the orchestrator gives each rung. High pass rate -> fewer retries -> faster throughput.
- **SPC -> Baseline recalibration**: BOCPD change point -> reset CUSUM accumulators + update EWMA target. Prevents stale baselines from producing false alarms or masking true shifts.
- **Multi-gate anomaly -> Sympathetic tightening**: Downstream degradation -> upstream gates tighten -> problems caught earlier -> downstream stabilizes.
- **Threshold data -> Dashboard**: Per-rung health (pass rate, in-control status, shift detection, skip advisory) rendered in the TUI and HTTP API.

---

## Open Questions

1. **Ratchet strictness**: The current ratchet is absolute -- a plan that passed Rung 2 can never record a Rung 1 pass as its watermark. Should there be a "soft ratchet" that allows regression after N consecutive failures at the higher rung? This would handle the case where a plan's Rung 2 pass was itself incorrect (e.g., flaky test) and the plan genuinely cannot pass Rung 2.

2. **SPC parameter tuning per project**: The default CUSUM k=0.25, h=4.0 works for a generic gate, but projects with high natural variability (many language targets, frequent dependency updates) may need different parameters. Should profiles include SPC parameters? The refactoring profile already does (k=0.15, h=3.0).

3. **Persistent ratchet + adaptive thresholds synchronization**: Both the ratchet and thresholds should persist, but they need to be consistent. If the ratchet says "plan X passed rung 3" and the thresholds say "rung 3 pass rate is 0%", something is wrong. Should they share a persistence lifecycle (save/load together)?

4. **BOCPD computational cost**: The O(R_max) per-step cost of BOCPD with max_run_length=300 is manageable for per-gate updates (hundreds per session). But with 7 gates and hundreds of tasks, the total BOCPD state across all rungs could be significant. Should BOCPD be run only on the most important rungs (Compile, Test)?
