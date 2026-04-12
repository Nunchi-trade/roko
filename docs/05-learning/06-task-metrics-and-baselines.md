# Task Metrics and Baselines

> **Crate:** `roko-learn` · **Modules:** `task_metric.rs`, `baseline.rs`, `efficiency.rs`
> **Persistence:** `.roko/learn/task-metrics.jsonl`, `.roko/learn/efficiency.jsonl`
> **Wiring:** `LearningRuntime::record_completed_run()` → metrics pipeline
> **Cross-references:** [00-episode-logger](00-episode-logger.md), [07-regression-detection](07-regression-detection.md), [15-collective-calibration-31x](15-collective-calibration-31x.md)

---

## Purpose

The task metrics and baselines subsystem provides the quantitative foundation for all performance evaluation in Roko. Every gate execution produces one immutable `TaskMetric` record. These records accumulate in an append-only JSONL file, and the baseline computation groups them by `(role, complexity_band)` to produce per-slice statistical profiles. The regression detector then compares fresh batches against these baselines to identify performance degradation.

The efficiency module extends per-turn instrumentation with prompt-level attribution, tool utilization tracking, and A-D letter grading for prompt assembly quality.

---

## TaskMetric Schema

The canonical `TaskMetric` struct lives in `roko-core::metric` and is re-exported by `roko-learn::task_metric`:

```rust
pub struct TaskMetric {
    /// Task identifier.
    pub task_id: String,
    /// Plan identifier.
    pub plan_id: String,
    /// Agent role (e.g. "Implementer").
    pub role: String,
    /// Complexity band ("fast", "standard", "complex").
    pub complexity_band: String,
    /// Model slug used.
    pub model: String,
    /// Backend provider.
    pub backend: String,
    /// Gate name.
    pub gate: String,
    /// Whether the gate passed.
    pub gate_passed: bool,
    /// Zero-based iteration index.
    pub iteration: u32,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Configuration hash for A/B comparison.
    pub config_hash: ConfigHash,
    /// Timestamp.
    pub timestamp: DateTime<Utc>,
}
```

### MetricFilter

The `MetricFilter` provides declarative filtering over the metric stream:

```rust
pub struct MetricFilter {
    pub roles: HashSet<String>,
    pub complexity_bands: HashSet<String>,
    pub plan_ids: HashSet<String>,
    pub gates: HashSet<String>,
    pub models: HashSet<String>,
    pub backends: HashSet<String>,
    pub gate_passed: Option<bool>,
    pub iteration_range: Option<(u32, u32)>,
    pub min_cost_usd: Option<f64>,
    pub max_cost_usd: Option<f64>,
    pub config_hashes: HashSet<String>,
}
```

All predicates are AND-combined: a record must match every non-empty filter field.

### MetricsWriter and MetricsReader

- `MetricsWriter` — thread-safe, append-only JSONL writer that batches records in memory and flushes to an `AsyncWrite` sink. Uses `parking_lot::Mutex` for serialization.
- `MetricsReader` — parse JSONL lines from bytes, tolerant of corrupted lines (same resilience pattern as the episode logger).

---

## Baseline Computation

The `baseline` module computes per-slice statistical profiles from accumulated `TaskMetric` records.

### SliceBaseline

```rust
pub struct SliceBaseline {
    /// Role for this slice.
    pub role: String,
    /// Complexity band for this slice.
    pub complexity_band: String,
    /// Gate pass rate (0.0 – 1.0).
    pub pass_rate: f64,
    /// Average cost in USD per task.
    pub avg_cost: f64,
    /// Average duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Average number of iterations to pass.
    pub avg_iterations: f64,
    /// Average input tokens per turn.
    pub avg_input_tokens: f64,
    /// Average output tokens per turn.
    pub avg_output_tokens: f64,
    /// Average cache hit rate.
    pub avg_cache_hit_rate: f64,
    /// Number of records in this slice.
    pub n_records: usize,
}
```

### Baseline

```rust
pub struct Baseline {
    /// Per-(role, complexity) statistical profiles.
    pub slices: Vec<SliceBaseline>,
    /// Overall aggregate across all slices.
    pub overall_pass_rate: f64,
    pub overall_avg_cost: f64,
    pub overall_avg_duration_ms: f64,
    pub overall_avg_iterations: f64,
    pub overall_n_records: usize,
}
```

### Computation

`compute_baseline()` groups `TaskMetric` records by `(role, complexity_band)` and computes descriptive statistics for each group:

```
TaskMetric records
    │
    ▼
Group by (role, complexity_band)
    │
    ├── ("Implementer", "standard") → 156 records
    │       pass_rate: 0.72
    │       avg_cost: $0.83
    │       avg_duration_ms: 45,000
    │       avg_iterations: 1.4
    │
    ├── ("Implementer", "complex") → 48 records
    │       pass_rate: 0.58
    │       avg_cost: $1.52
    │       avg_duration_ms: 120,000
    │       avg_iterations: 2.1
    │
    └── ("Reviewer", "standard") → 89 records
            pass_rate: 0.91
            avg_cost: $0.35
            avg_duration_ms: 12,000
            avg_iterations: 1.1
```

---

## Efficiency Events

The `AgentEfficiencyEvent` provides per-turn cost and quality instrumentation with 20+ fields:

```rust
pub struct AgentEfficiencyEvent {
    // ── Identity ──────────────────────────────
    pub agent_id: String,
    pub role: String,
    pub backend: String,
    pub model: String,
    pub plan_id: String,
    pub task_id: String,

    // ── Token accounting ──────────────────────
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,

    // ── Cost accounting ───────────────────────
    pub cost_usd: f64,
    pub cost_usd_without_cache: f64,

    // ── Prompt composition ────────────────────
    pub prompt_sections: Vec<PromptSectionMeta>,
    pub total_prompt_tokens: u64,
    pub system_prompt_tokens: u64,

    // ── Tool utilization ──────────────────────
    pub tools_available: u32,
    pub tools_used: u32,
    pub tool_calls: Vec<ToolCallMeta>,

    // ── Timing ────────────────────────────────
    pub wall_time_ms: u64,
    pub time_to_first_token_ms: u64,
    pub was_warm_start: bool,
}
```

### PromptSectionMeta

Attributes token budget consumption to individual prompt sections:

```rust
pub struct PromptSectionMeta {
    /// Section name (e.g. "prd2", "workspace_map", "playbook_hits").
    pub name: String,
    /// Tokens consumed in the final prompt.
    pub tokens: u64,
    /// Composer-assigned priority (0 = highest).
    pub priority: u8,
    /// Whether this section was truncated due to budget pressure.
    pub was_truncated: bool,
    /// Whether this section was dropped entirely.
    pub was_dropped: bool,
}
```

### ToolCallMeta

Per-tool-call instrumentation:

```rust
pub struct ToolCallMeta {
    /// Tool name (e.g. "Read", "Write", "Bash").
    pub tool_name: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Tokens in the tool result.
    pub result_tokens: u64,
    /// Whether the call succeeded.
    pub succeeded: bool,
}
```

---

## Prompt Efficiency Grading

The efficiency module provides A-D letter grading for prompt assembly:

```rust
pub enum Grade {
    A,  // ≥ 0.8 — excellent efficiency
    B,  // ≥ 0.6 — good efficiency
    C,  // ≥ 0.4 — fair efficiency
    D,  // < 0.4 — poor efficiency
}
```

The `PromptEfficiencyScore` evaluates:

1. **Section utilization** — what fraction of included sections contributed to the successful outcome?
2. **Token efficiency** — output tokens / input tokens (higher = more productive per token spent).
3. **Cache hit rate** — cache_read_tokens / input_tokens (higher = better cache utilization).
4. **Tool utilization** — tools_used / tools_available (higher = more tools leveraged).

These four components are weighted and combined into a composite score:

```
efficiency = 0.30 × section_utilization
           + 0.30 × token_efficiency
           + 0.20 × cache_hit_rate
           + 0.20 × tool_utilization
```

### Role Cost Profiles

The `RoleCostProfile` aggregates cost data per agent role:

```rust
pub struct RoleCostProfile {
    pub role: String,
    pub total_cost_usd: f64,
    pub avg_cost_per_turn: f64,
    pub avg_cost_per_success: f64,
    pub total_turns: u64,
    pub total_successes: u64,
    pub avg_input_tokens: f64,
    pub avg_output_tokens: f64,
    pub avg_cache_hit_rate: f64,
}
```

These profiles answer operational questions: "Which role is most expensive?" "Does the warm pool save money?" "Which prompt sections drove the cost?"

---

## Four Key Metrics

From the legacy design (mori-agents/07-self-improvement.md), four metrics drive self-improvement:

| Metric | Definition | Baseline Target |
|--------|-----------|-----------------|
| **First-attempt pass rate** | % of tasks passing gates on first try | > 60% |
| **Iterations per plan** | Average iterations to complete a plan | < 2.0 |
| **Cost per plan** | Total USD spent per plan | Decreasing trend |
| **Prompt tokens per spawn** | Input tokens for the initial agent prompt | < 50K |

These metrics are computed from the `TaskMetric` stream and surfaced via the `compute_headlines()` function:

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

## Persistence

| Artifact | Format | Path |
|----------|--------|------|
| Task metrics | JSONL | `.roko/learn/task-metrics.jsonl` |
| Efficiency events | JSONL | `.roko/learn/efficiency.jsonl` |

Both files are append-only. The `MetricsWriter` batches records in memory and flushes periodically for efficiency.

---

## Relationship to Other Documents

- **[00-episode-logger](00-episode-logger.md)** — Episodes produce the raw data; task metrics are the per-gate derivative.
- **[07-regression-detection](07-regression-detection.md)** — Baselines are the reference point for regression detection.
- **[04-cascade-router](04-cascade-router.md)** — Routing decisions affect metrics; metrics affect future routing decisions.
- **[08-cost-normalization](08-cost-normalization.md)** — Cost fields in metrics use normalized costs.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — Metrics feed into C-Factor components (gate_pass_rate, cost_efficiency, speed).

See also: [04-verification](../04-verification/INDEX.md) for the gate pipeline that produces individual gate outcomes.
