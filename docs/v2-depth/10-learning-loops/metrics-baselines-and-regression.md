# Metrics, Baselines, and Regression Detection

> Depth for [07-LEARNING.md](../../unified/07-LEARNING.md). Task metrics as Score Cells rating agent performance along quality/cost/speed axes, regression detection as a Verify Cell comparing current performance against historical baselines, and cost normalization across providers -- expressed as a Lens (observation) feeding a Loop (threshold adjustment).

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse), [02-CELL](../../unified/02-CELL.md) (Score protocol, Verify protocol, Observe/Lens), [04-EXECUTION](../../unified/04-EXECUTION.md) (Loop specialization), [07-LEARNING](../../unified/07-LEARNING.md) (L1 Parameter Tuning, predict-publish-correct)

**Source docs**: `docs/05-learning/06-task-metrics-and-baselines.md`, `docs/05-learning/07-regression-detection.md`, `docs/05-learning/08-cost-normalization.md`

---

## 1. Metrics as Score Cells

Every gate execution produces one immutable `TaskMetric` Signal. The metrics pipeline is a **Score protocol Cell** -- it rates agent performance along quality, cost, and speed axes, producing numerical scores that feed baselines, regression detection, and dashboard Lenses.

### TaskMetric Signal Schema

```rust
/// A TaskMetric is a Signal produced by the Score protocol.
/// One per gate execution, immutable, append-only to JSONL Store.
///
/// See [02-CELL.md](../../unified/02-CELL.md) for the Score protocol:
/// "rate along 5 dimensions."
pub struct TaskMetric {
    pub task_id: String,
    pub plan_id: String,
    pub role: String,
    pub complexity_band: String,   // "fast", "standard", "complex"
    pub model: String,
    pub backend: String,
    pub gate: String,
    pub gate_passed: bool,
    pub iteration: u32,
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub config_hash: ConfigHash,   // for A/B comparison
    pub timestamp: DateTime<Utc>,
}
```

### Filtering

The `MetricFilter` provides declarative AND-combined filtering over the metric stream. All non-empty filter fields must match:

```rust
pub struct MetricFilter {
    pub roles: HashSet<String>,
    pub complexity_bands: HashSet<String>,
    pub gates: HashSet<String>,
    pub models: HashSet<String>,
    pub gate_passed: Option<bool>,
    pub iteration_range: Option<(u32, u32)>,
    pub min_cost_usd: Option<f64>,
    pub max_cost_usd: Option<f64>,
    pub config_hashes: HashSet<String>,
}
```

---

## 2. Baselines as Lens Cells

Baselines are **Lens** projections (see [02-CELL.md](../../unified/02-CELL.md)) -- read-only observations that group TaskMetric Signals by `(role, complexity_band)` and compute statistical profiles. The Lens does not mutate; it projects the Store's contents into a summary view.

### SliceBaseline

```rust
/// Lens Cell: per-slice statistical profile.
///
/// Groups TaskMetric Signals by (role, complexity_band)
/// and computes descriptive statistics.
pub struct SliceBaseline {
    pub role: String,
    pub complexity_band: String,
    pub pass_rate: f64,
    pub avg_cost: f64,
    pub avg_duration_ms: f64,
    pub avg_iterations: f64,
    pub avg_input_tokens: f64,
    pub avg_output_tokens: f64,
    pub avg_cache_hit_rate: f64,
    pub n_records: usize,
}
```

### Computation Pipeline

```
TaskMetric Store (append-only JSONL)
    |
    v
Lens Cell: compute_baseline()
    |
    +-- Group by (role, complexity_band)
    |
    +-- ("Implementer", "standard") -> 156 records
    |       pass_rate: 0.72, avg_cost: $0.83
    |       avg_duration_ms: 45,000, avg_iterations: 1.4
    |
    +-- ("Implementer", "complex") -> 48 records
    |       pass_rate: 0.58, avg_cost: $1.52
    |
    +-- ("Reviewer", "standard") -> 89 records
            pass_rate: 0.91, avg_cost: $0.35
```

The per-slice granularity is critical: a severe regression in "Implementer/complex" tasks must not be masked by improvements in "Reviewer/standard" tasks.

### Four Headline Metrics

From the legacy mori design, four metrics drive self-improvement:

| Metric | Definition | Target |
|---|---|---|
| First-attempt pass rate | % tasks passing gates on first try | > 60% |
| Iterations per plan | Average iterations to complete a plan | < 2.0 |
| Cost per plan | Total USD per plan | Decreasing trend |
| Prompt tokens per spawn | Input tokens for initial agent prompt | < 50K |

These are computed from the TaskMetric stream and surfaced via a Headlines Lens Cell:

```rust
pub struct Headlines {
    pub total_tasks: usize,
    pub passed_tasks: usize,
    pub pass_rate: f64,
    pub total_cost_usd: f64,
    pub avg_cost_per_task: f64,
    pub avg_iterations: f64,
    pub avg_duration_ms: f64,
}
```

---

## 3. Regression Detection as a Verify Cell

The regression detector is a **Verify protocol Cell** that compares a fresh batch of TaskMetric Signals against a historical Baseline and emits Verdict Signals when indicators breach thresholds. This closes the feedback loop: system changes -> metric impact -> alert or rollback.

### Threshold Configuration

```rust
/// Verify Cell: regression detection thresholds.
///
/// See [02-CELL.md](../../unified/02-CELL.md) for the Verify protocol:
/// "check -> Verdict. Conjunctive hard + Pareto soft."
pub struct RegressionThresholds {
    pub pass_rate_drop: f64,      // max allowed drop (default: 0.15 = 15%)
    pub cost_increase: f64,       // max allowed increase (default: 0.20 = 20%)
    pub duration_increase: f64,   // max allowed increase (default: 0.30 = 30%)
    pub iterations_increase: f64, // max allowed increase (default: 0.25 = 25%)
    pub min_records: usize,       // minimum records before firing (default: 5)
}
```

The asymmetry between Alert and Warning severity:

| Metric | Threshold | Severity | Rationale |
|---|---|---|---|
| Pass rate drop | > 15% | **Alert** | Direct impact on task completion |
| Cost increase | > 20% | **Alert** | Budget impact |
| Duration increase | > 30% | Warning | May be acceptable tradeoff |
| Iterations increase | > 25% | Warning | May reflect harder task mix |

### Detection Algorithm

```
For each (role, complexity_band) slice with >= min_records:
    |
    +-- pass_rate:
    |     change = (baseline - current) / baseline
    |     if change > pass_rate_drop -> Alert
    |     if change < -pass_rate_drop -> Improvement
    |
    +-- cost:
    |     change = (current - baseline) / baseline
    |     if change > cost_increase -> Alert
    |
    +-- duration:
    |     change = (current - baseline) / baseline
    |     if change > duration_increase -> Warning
    |
    +-- iterations:
          change = (current - baseline) / baseline
          if change > iterations_increase -> Warning
```

### Regression Verdict Signal

```rust
/// The Verify Cell's output: a Verdict Signal per detected regression.
pub struct RegressionAlert {
    pub metric_name: String,
    pub severity: AlertSeverity,    // Alert, Warning, Improvement
    pub baseline_value: f64,
    pub current_value: f64,
    pub change_fraction: f64,
    pub threshold: f64,
    pub description: String,
    pub slice: Option<(String, String)>,
}
```

### Current Window

The `current_window` parameter (default: 20 records) determines how many recent metrics form the "current" batch:

- Too small (< 10): noisy, single outliers trigger false alerts.
- Too large (> 50): sluggish, real regressions take many tasks to surface.
- 20 provides stability while catching regressions within a single plan execution.

### C-Factor Regression

Beyond per-metric regression, the C-Factor provides a composite regression check. When the c-factor score drops against a trailing history window, it indicates systemic decline -- multiple metrics degrading slightly, none individually alarming, but collectively significant. See [c-factor-as-lens.md](c-factor-as-lens.md) for the C-Factor Lens.

---

## 4. The Lens-to-Loop Pipeline

Baselines and regression detection form a two-stage pipeline: a **Lens** (observation) feeding a **Loop** (threshold adjustment). This is the L1 Parameter Tuning Loop from [07-LEARNING.md](../../unified/07-LEARNING.md) applied to verification thresholds.

```
Lens Stage (observation, read-only):
    TaskMetric Store -> Baseline Lens -> SliceBaseline Signals
                                             |
                                             v
Loop Stage (feedback, read-write):
    SliceBaseline + Current Batch -> Regression Verify Cell -> Verdict Signals
                                                                    |
                                                                    v
                                                          Adaptive Threshold Update
                                                                    |
                                                     FEEDBACK: adjusted thresholds
                                                     affect next Verify evaluation
```

### Adaptive Gate Threshold Update

Gate thresholds (compile, test, lint, diff pass/fail criteria) are adjusted via EMA:

```
new_threshold = alpha * observed_pass_rate + (1 - alpha) * old_threshold
```

Where `alpha = 0.1` (slow learning rate to avoid oscillation). This is predict-publish-correct: the current threshold predicts the pass/fail boundary, the observed pass rate is the outcome, and the EMA update is the correction.

Thresholds persist to `.roko/learn/gate-thresholds.json` and are loaded on startup. The mori-diffs reality doc notes that this loading was not wired in the runner v2 event loop (gap L5) -- thresholds would reset on restart.

---

## 5. Cost Normalization

Cost normalization is a Score Cell that transforms heterogeneous provider pricing into a single comparable metric: blended cost per million tokens.

### Blended Cost Formula

```
blended_cost_per_m = (3 * input_price_per_m + 1 * output_price_per_m) / 4
```

The 3:1 ratio reflects measured agent workloads: agents read ~3x more than they write. Using this ratio makes the blended cost correspond to actual expenditure.

### Token-Type Normalization

| Token Type | Normalization |
|---|---|
| Fresh input tokens | 1.0x (full input price) |
| Cache read tokens | Weighted by actual cache discount (10-90% off) |
| Cache write tokens | 1.0x (same as input) |
| Reasoning tokens | Counted as output |

The `AgentEfficiencyEvent` captures both actual cost (after cache discounts) and hypothetical full-price cost, enabling cache savings analysis.

### Budget Guardrails

Multi-level spending limits enforced as a Verify Cell pipeline:

```rust
/// Verify Cell: budget enforcement at three levels.
///
/// Conjunctive hard constraints: all three must pass.
pub struct BudgetGuardrail {
    pub per_task_limit: f64,
    pub per_session_limit: f64,
    pub per_day_limit: f64,
}

pub enum BudgetAction {
    Continue,                    // < 80% of limit
    Downgrade,                   // >= 80% -> route to cheaper model
    Block,                       // >= 95% -> reject new requests
    HardStop,                    // >= 100% -> terminate session
}
```

The escalation creates a cybernetic feedback loop: budget pressure -> cheaper model routing -> lower cost -> reduced pressure. This is Loop 6 (Cost -> Routing) from the source docs.

---

## 6. Efficiency Events

The `AgentEfficiencyEvent` is a detailed Score Cell output with 20+ fields for per-turn cost and quality instrumentation:

```rust
/// Score Cell: per-turn efficiency measurement.
///
/// Includes prompt-level attribution, tool utilization,
/// and cache analysis.
pub struct AgentEfficiencyEvent {
    // Identity
    pub agent_id: String,
    pub role: String,
    pub model: String,

    // Token accounting
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,

    // Cost accounting
    pub cost_usd: f64,
    pub cost_usd_without_cache: f64,

    // Prompt composition attribution
    pub prompt_sections: Vec<PromptSectionMeta>,

    // Tool utilization
    pub tools_available: u32,
    pub tools_used: u32,
    pub tool_calls: Vec<ToolCallMeta>,

    // Timing
    pub wall_time_ms: u64,
    pub time_to_first_token_ms: u64,
    pub was_warm_start: bool,
}
```

### Prompt Efficiency Grading

A-D letter grades for prompt assembly quality, computed from four components:

```
efficiency = 0.30 * section_utilization   // fraction of sections that contributed
           + 0.30 * token_efficiency       // output_tokens / input_tokens
           + 0.20 * cache_hit_rate         // cache_read / input_tokens
           + 0.20 * tool_utilization       // tools_used / tools_available

A >= 0.8, B >= 0.6, C >= 0.4, D < 0.4
```

This feeds the Compose protocol's section effect tracking from [07-LEARNING.md](../../unified/07-LEARNING.md): which prompt sections correlate with gate success? Sections with low utilization across many episodes are candidates for removal or budget reduction.

---

## 7. False Positive Management

Regression detection can produce false positives when the task mix shifts (harder tasks appear as lower pass rate). Mitigation strategies, each mapping to a unified primitive:

| Strategy | Unified Mapping |
|---|---|
| Minimum record threshold (>= 5 per slice) | Verify Cell pre-condition |
| Per-slice analysis | Score protocol: rate each slice independently |
| Improvement tracking | Publish improvements alongside regressions (full Verdict) |
| Config hash correlation | Lineage tracking: which config change correlates with regression |

---

## 8. Mori-Diffs Reality

The mori-diffs document identifies these specific gaps in the metrics pipeline:

| Gap | Impact |
|---|---|
| Efficiency events per-task not per-turn (L4) | Prompt section attribution impossible at turn granularity |
| Adaptive gate thresholds not loaded from disk (L5) | Thresholds reset on restart, learning is lost |
| Episodes missing provider field | Cost normalization cannot distinguish providers |

The `LearningCollector` design addresses these by collecting per-turn snapshots and flushing enriched data atomically on gate completion.

---

## What This Enables

1. **Per-slice performance visibility**: operators see pass rate, cost, speed broken down by (role, complexity), not just aggregate numbers.
2. **Automatic regression alerts**: system changes that degrade performance are caught within ~20 tasks, before they accumulate significant damage.
3. **Cost-aware routing feedback**: budget pressure automatically adjusts routing toward cheaper models, creating a self-regulating cost control loop.
4. **Prompt assembly optimization**: efficiency grading identifies which prompt sections contribute to success and which waste tokens.
5. **Adaptive gate thresholds**: L1 EMA tuning ensures gate pass/fail boundaries reflect the system's actual capabilities, not static defaults.

## Feedback Loops

- **Lens -> Loop**: baselines (Lens observation) feed regression detection (Verify Cell), which triggers threshold adjustment (L1 Loop).
- **Cost -> Routing**: budget guardrails force cheaper model selection, reducing cost, relaxing pressure. Cybernetic loop 6.
- **Regression -> Rollback**: Alert-severity regressions trigger parameter rollback (L1 safety), reverting the last adjustment and halving the learning rate.
- **Efficiency -> Compose**: prompt section grading feeds the Compose protocol's budget allocation, reducing tokens for low-impact sections.
- **Predict-publish-correct on thresholds**: current threshold predicts pass/fail boundary, observed rate is outcome, EMA update is correction.

## Open Questions

1. **Window size adaptation**: should the current_window (default: 20) adapt to task throughput? High-throughput deployments could use smaller windows for faster detection.
2. **Multi-dimensional regression**: the current detector checks each metric independently. Should it detect multi-dimensional regressions where no single metric breaches its threshold but the combined deviation is significant? The C-Factor regression provides a partial answer.
3. **Baseline decay**: as the system improves, old metrics in the baseline drag down expectations. Should the baseline use a sliding window rather than all-time history?
4. **Cost normalization staleness**: provider pricing changes without notice. How frequently should the CostTable be refreshed? Should the system detect price changes from observed cost/token ratios?
5. **Relationship to autocatalytic-compounding.md**: regression detection is the negative feedback that prevents compounding from inverting. If the anti-metrics (warm tier growth, unconfirmed heuristic count, lineage depth) from [autocatalytic-compounding.md](autocatalytic-compounding.md) overlap with regression detection, should they be unified into a single Verify Cell pipeline?
