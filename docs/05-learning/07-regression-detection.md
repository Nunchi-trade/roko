# Regression Detection

> **Crate:** `roko-learn` · **Module:** `regression.rs`
> **Wiring:** `LearningRuntime::record_completed_run()` → regression check
> **Cross-references:** [06-task-metrics-and-baselines](06-task-metrics-and-baselines.md), [14-stability-mechanisms](14-stability-mechanisms.md), [15-collective-calibration-31x](15-collective-calibration-31x.md)


> **Implementation**: Shipping

---

## Purpose

The regression detector answers a critical question: "Did this configuration change make things worse?" It compares a fresh batch of `TaskMetric` records against a previously computed `Baseline` and fires alerts when key indicators breach configurable thresholds. This closes the feedback loop between system changes (prompt modifications, model routing updates, playbook rule changes) and their observable impact on task outcomes.

Without regression detection, the learning system could silently degrade: a bandit might converge on a model that worked well last week but performs poorly after a provider update, or a playbook rule might be promoted despite introducing regressions in edge cases. The regression detector surfaces these degradations as structured alerts that trigger investigation or automatic rollback.

---

## Threshold Configuration

```rust
pub struct RegressionThresholds {
    /// Maximum allowed drop in first-attempt pass rate (default: 0.15 = 15%).
    pub pass_rate_drop: f64,
    /// Maximum allowed increase in average cost (default: 0.20 = 20%).
    pub cost_increase: f64,
    /// Maximum allowed increase in average duration (default: 0.30 = 30%).
    pub duration_increase: f64,
    /// Maximum allowed increase in average iterations (default: 0.25 = 25%).
    pub iterations_increase: f64,
    /// Minimum records needed before detection fires (default: 5).
    pub min_records: usize,
}
```

### Default Thresholds

| Metric | Threshold | Severity | Rationale |
|--------|-----------|----------|-----------|
| Pass rate drop | > 15% | **Alert** | Direct impact on task completion |
| Cost increase | > 20% | **Alert** | Budget impact |
| Duration increase | > 30% | Warning | May be acceptable for higher quality |
| Iterations increase | > 25% | Warning | More iterations may reflect harder tasks |

The asymmetry between Alert and Warning severities reflects priority: pass rate and cost regressions are immediate blockers that demand investigation, while duration and iteration increases may be acceptable tradeoffs (e.g., a harder task mix this week).

---

## Detection Algorithm

The `detect_regressions()` function compares current metrics against a baseline:

```
Baseline (from historical TaskMetric records)
    │
    ▼
Current batch (recent N task metrics)
    │
    ▼
For each (role, complexity_band) slice:
    │
    ├── Pass rate regression:
    │     change = (baseline.pass_rate - current.pass_rate) / baseline.pass_rate
    │     if change > pass_rate_drop → Alert
    │     if change < -pass_rate_drop → Improvement
    │
    ├── Cost regression:
    │     change = (current.avg_cost - baseline.avg_cost) / baseline.avg_cost
    │     if change > cost_increase → Alert
    │     if change < -cost_increase → Improvement
    │
    ├── Duration regression:
    │     change = (current.avg_duration - baseline.avg_duration) / baseline.avg_duration
    │     if change > duration_increase → Warning
    │     if change < -duration_increase → Improvement
    │
    └── Iterations regression:
          change = (current.avg_iterations - baseline.avg_iterations) / baseline.avg_iterations
          if change > iterations_increase → Warning
          if change < -iterations_increase → Improvement
```

### Per-Slice Analysis

Regressions are detected per `(role, complexity_band)` slice, not just in aggregate. This prevents a scenario where a severe regression in "Implementer/complex" tasks is masked by improvements in "Reviewer/standard" tasks. Each slice that has enough records (≥ `min_records`) is analyzed independently.

---

## Alert Schema

```rust
pub enum AlertSeverity {
    Alert,        // Key metric breached (pass rate, cost)
    Warning,      // Secondary metric breached (duration, iterations)
    Improvement,  // Metric improved relative to baseline
}

pub struct RegressionAlert {
    /// Which metric regressed (e.g. "pass_rate", "cost").
    pub metric_name: String,
    /// Severity.
    pub severity: AlertSeverity,
    /// Baseline value.
    pub baseline_value: f64,
    /// Current (observed) value.
    pub current_value: f64,
    /// Fractional change (positive = worsened).
    pub change_fraction: f64,
    /// The threshold that was breached.
    pub threshold: f64,
    /// Human-readable description.
    pub description: String,
    /// Optional (role, complexity) slice. None = overall.
    pub slice: Option<(String, String)>,
}
```

### RegressionReport

```rust
pub struct RegressionReport {
    /// All alerts (breaches and improvements).
    pub alerts: Vec<RegressionAlert>,
    /// Whether any alert has Alert severity.
    pub has_regressions: bool,
    /// Whether the current data set has enough records.
    pub sufficient_data: bool,
    /// Number of current records analyzed.
    pub current_records: usize,
    /// Number of baseline records.
    pub baseline_records: usize,
}
```

The report provides convenience methods:
- `regressions()` — filter to Alert-severity items only.
- `warnings()` — filter to Warning-severity items only.

---

## LearningRuntime Integration

The regression detector runs as part of `LearningRuntime::record_completed_run()` when a `RegressionConfig` is configured:

```rust
pub struct RegressionConfig {
    pub thresholds: RegressionThresholds,
    /// Number of latest metrics used as the "current" sample.
    pub current_window: usize,  // default: 20
}
```

The runtime:
1. Reads all `TaskMetric` records from `.roko/learn/task-metrics.jsonl`.
2. Splits into baseline (all records except the latest `current_window`) and current (latest `current_window` records).
3. Computes baselines for both sets.
4. Calls `detect_regressions(baseline, current, thresholds)`.
5. If `report.has_regressions`, logs the alerts and optionally triggers corrective actions.

### Current Window

The `current_window` parameter (default: 20) determines how many recent metrics are treated as "current" for comparison against the baseline. This value balances:
- **Too small** (< 10): noisy, a single outlier can trigger false alerts.
- **Too large** (> 50): sluggish, a real regression takes many tasks to surface.
- **20** provides a reasonable tradeoff: enough data for statistical stability, but responsive enough to catch regressions within a single plan execution.

---

## C-Factor Regression

In addition to per-metric regression detection, the C-Factor module provides its own regression check over the composite C-Factor score:

```rust
pub struct CFactorRegression {
    pub current_snapshot_at: DateTime<Utc>,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub sample_count: usize,
    // ... delta analysis fields
}
```

C-Factor regression detects systemic decline: when the composite score drops against a trailing history window, it indicates that the system as a whole is performing worse, even if individual metrics haven't breached their thresholds. This catches subtle multi-dimensional regressions where pass rate drops slightly, cost rises slightly, and speed decreases slightly — none individually alarming, but collectively significant.

See [15-collective-calibration-31x](15-collective-calibration-31x.md) for the C-Factor computation.

---

## Adaptive Gate Thresholds

Regression detection interacts with the adaptive gate threshold system. Gate thresholds (pass/fail criteria for compile, test, lint, diff gates) are adjusted via EMA (Exponential Moving Average) based on historical pass rates:

```
Gate threshold = EMA(pass_rates, alpha=0.1)
```

When the regression detector fires a pass_rate Alert, it signals that thresholds may need recalibration. The adaptive threshold system can then tighten thresholds (require higher quality) or loosen them (accept the current performance level) based on operational priorities.

See [04-verification](../04-verification/INDEX.md) for the gate pipeline and adaptive threshold mechanism.

---

## Practical Example

Consider a system running 200 tasks over 3 days. After task 150, a prompt template change was deployed.

### Baseline (tasks 1-130)

```
Overall:
    pass_rate: 0.72
    avg_cost: $0.83
    avg_duration_ms: 45,000
    avg_iterations: 1.4

Slice (Implementer, standard):
    pass_rate: 0.75
    avg_cost: $0.78
    avg_duration_ms: 42,000
    avg_iterations: 1.3
```

### Current window (tasks 151-170, after template change)

```
Overall:
    pass_rate: 0.55     ← dropped
    avg_cost: $1.12     ← increased
    avg_duration_ms: 52,000
    avg_iterations: 1.9

Slice (Implementer, standard):
    pass_rate: 0.50     ← dropped significantly
    avg_cost: $1.05     ← increased
    avg_duration_ms: 48,000
    avg_iterations: 2.0
```

### Regression Report

```
ALERT: pass_rate regression in (Implementer, standard)
    Baseline: 0.75, Current: 0.50
    Change: -33.3% (threshold: 15%)
    "Pass rate dropped from 75% to 50% for Implementer/standard tasks"

ALERT: cost regression in (Implementer, standard)
    Baseline: $0.78, Current: $1.05
    Change: +34.6% (threshold: 20%)
    "Average cost increased from $0.78 to $1.05 for Implementer/standard tasks"

WARNING: iterations regression in (Implementer, standard)
    Baseline: 1.3, Current: 2.0
    Change: +53.8% (threshold: 25%)
    "Average iterations increased from 1.3 to 2.0 for Implementer/standard tasks"
```

### Corrective Action

The regression report identifies that the prompt template change degraded Implementer/standard tasks. The operator can:
1. **Revert**: Roll back the template change.
2. **Investigate**: Examine which specific tasks failed and why.
3. **Adjust**: Modify the template to address the failure pattern while preserving improvements in other slices.

Without regression detection, this degradation would be invisible — the system would continue spending more money for worse results.

---

## False Positive Management

Regression detection can produce false positives when:
- The task mix shifts (harder tasks → lower pass rate, not a regression).
- A single expensive task dominates the cost average.
- The current window is too small to be statistically stable.

Mitigation strategies:
1. **min_records threshold**: Don't fire alerts with fewer than 5 records per slice.
2. **Per-slice analysis**: Detect whether the regression is slice-specific or systemic.
3. **Improvement tracking**: Report improvements alongside regressions to provide context.
4. **Config hash correlation**: If a specific config change correlates with the regression, flag it as the likely cause.

---

## Relationship to Other Documents

- **[06-task-metrics-and-baselines](06-task-metrics-and-baselines.md)** — Task metrics and baselines are the input to regression detection.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Regression detection is itself a stability mechanism, providing negative feedback when performance degrades.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — C-Factor regression provides a composite view of system health.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Loop 4 (Failure→Replanning) uses regression alerts to trigger plan revision.
- **[04-cascade-router](04-cascade-router.md)** — Routing changes can cause regressions detected here.
