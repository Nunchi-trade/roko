# Cost Normalization

> **Crate:** `roko-learn` · **Modules:** `costs_db.rs`, `costs_log.rs`
> **Persistence:** `.roko/learn/costs.jsonl`
> **Implementation plan:** `modelrouting/09-cost-normalization.md` (tasks 2H.01–2H.10)
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [06-task-metrics-and-baselines](06-task-metrics-and-baselines.md), [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)


> **Implementation**: Shipping

---

## Purpose

Cost normalization provides a consistent framework for comparing model costs across providers, pricing tiers, and token types. Raw cost data from providers is heterogeneous: some charge per input token, some per output token, some per request; cache hits reduce costs differently; reasoning tokens may have distinct pricing. The cost normalization layer transforms this into a single comparable metric — blended cost per million tokens — that the cascade router and budget guardrails can use for routing decisions.

---

## CostRecord Schema

```rust
pub struct CostRecord {
    /// Timestamp of the cost observation.
    pub timestamp: DateTime<Utc>,
    /// Model slug (e.g. "claude-sonnet-4-20250514").
    pub model: String,
    /// Provider identifier (e.g. "anthropic", "openrouter").
    pub provider: String,
    /// Agent role.
    pub role: String,
    /// Plan identifier.
    pub plan_id: String,
    /// Task identifier.
    pub task_id: String,
    /// Complexity band ("fast", "standard", "complex").
    pub complexity_band: String,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Total cost in USD.
    pub cost_usd: f64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the task succeeded.
    pub success: bool,
    /// Session identifier for grouping.
    pub session_id: String,
}
```

### CostSummary

Aggregate view over a collection of `CostRecord`s:

```rust
pub struct CostSummary {
    /// Total records summarized.
    pub total_records: usize,
    /// Total cost across all records.
    pub total_cost_usd: f64,
    /// Average cost per record.
    pub avg_cost_usd: f64,
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Average cost per successful task.
    pub avg_cost_per_success: f64,
    /// Number of successful records.
    pub success_count: usize,
}
```

---

## Blended Cost Formula

The blended cost per million tokens uses a 3:1 input-to-output weighting ratio, following the Artificial Analysis methodology:

```
blended_cost_per_m = (3 × input_price_per_m + 1 × output_price_per_m) / 4
```

### Why 3:1?

The 3:1 ratio reflects the typical token mix in agent workloads: agents read more than they write. In Roko's measured workloads, the median input-to-output ratio is approximately 3:1 (for every output token generated, the agent consumes ~3 input tokens in context, prior conversation, and tool results). Using this ratio makes the blended cost metric correspond to actual expenditure patterns.

### Token-Type Normalization

Not all input tokens cost the same:

| Token Type | Typical Pricing | Normalization |
|------------|----------------|---------------|
| Fresh input tokens | Full input price | 1.0× |
| Cache read tokens | 10-90% discount | Weighted by actual cache price |
| Cache write tokens | Usually same as input | 1.0× |
| Reasoning/thinking tokens | Usually same as output | Counted as output |
| System prompt tokens | Full input price | 1.0× (but often cached) |

The `AgentEfficiencyEvent` captures both `cost_usd` (actual cost after cache discounts) and `cost_usd_without_cache` (hypothetical full-price cost), enabling analysis of cache savings.

---

## CostTable Design

The CostTable (from implementation plan 2H.01–2H.04) structures per-model pricing:

```rust
pub struct ModelPricing {
    /// Model slug.
    pub model: String,
    /// Provider.
    pub provider: String,
    /// Input price per million tokens.
    pub input_price_per_m: f64,
    /// Output price per million tokens.
    pub output_price_per_m: f64,
    /// Cache read price per million tokens (if different).
    pub cache_read_price_per_m: Option<f64>,
    /// Blended cost per million tokens (3:1 ratio).
    pub blended_cost_per_m: f64,
}
```

The CostTable is loaded from configuration and updated periodically as providers change pricing. It provides the cost signal that the cascade router uses in its cost penalty computation.

---

## Budget Guardrails

The budget guardrail system (from implementation plan 2H.05–2H.10) enforces multi-level spending limits:

```rust
pub struct BudgetGuardrail {
    /// Per-task cost limit in USD.
    pub per_task_limit: f64,
    /// Per-session cost limit in USD.
    pub per_session_limit: f64,
    /// Per-day cost limit in USD.
    pub per_day_limit: f64,
}

pub enum BudgetAction {
    /// Continue with current model.
    Continue,
    /// Downgrade to a cheaper model (triggered at 80% of limit).
    Downgrade,
    /// Block the request (triggered at 95% of limit).
    Block,
    /// Hard stop — no further requests (triggered at 100% of limit).
    HardStop,
}
```

### Escalation Thresholds

| Level | % of Limit | Action | Rationale |
|-------|------------|--------|-----------|
| Normal | < 80% | Continue | Full freedom |
| Warn | 80% | Downgrade | Route to cheaper model |
| Block | 95% | Block | Reject new requests |
| Hard stop | 100% | HardStop | Terminate session |

The downgrade action at 80% is a soft intervention: instead of failing the task, the router automatically selects a cheaper model. This preserves task completion while controlling costs. The 95% block prevents any new requests from starting, and the 100% hard stop terminates the session entirely.

### Multi-Level Enforcement

Budget limits are enforced at three levels simultaneously:

1. **Per-task** — prevents a single task from consuming disproportionate resources. A task that exceeds its budget is downgraded or blocked before reaching the session limit.
2. **Per-session** — prevents a session (typically one plan execution) from exceeding its allocation. This catches scenarios where many cheap tasks collectively exceed budget.
3. **Per-day** — absolute daily spending cap. This protects against runaway automation that might execute many plans in a day.

```
Incoming request
    │
    ▼
Check per-task budget
    │ if task_cost ≥ 80% of per_task_limit → Downgrade
    │ if task_cost ≥ 95% of per_task_limit → Block
    │
    ▼
Check per-session budget
    │ if session_cost ≥ 80% of per_session_limit → Downgrade
    │ if session_cost ≥ 95% of per_session_limit → Block
    │
    ▼
Check per-day budget
    │ if day_cost ≥ 80% of per_day_limit → Downgrade
    │ if day_cost ≥ 100% of per_day_limit → HardStop
    │
    ▼
Route to selected model (or cheaper alternative if Downgrade)
```

---

## CostsLog: Append-Only Persistence

The `CostsLog` provides durable, file-backed persistence for `CostRecord` values:

```rust
pub struct CostsLog {
    path: PathBuf,
    fsync: bool,
}
```

Key operations:
- `CostsLog::at(path)` — construct a log at `path` with fsync enabled.
- `CostsLog::open_creating(path)` — create parent directories and construct a log.
- `CostsLog::append(record)` — append one `CostRecord` as a JSON line with optional fsync.
- `CostsLog::append_all(records)` — batch append with a single open/close cycle.
- `CostsLog::read_all(path)` — read all records, tolerant of corrupt lines.

The `without_fsync()` builder method disables per-append fsync for test environments where crash safety is not needed.

### Relationship to CostsDb

`CostsDb` is the in-memory cost database used for real-time queries. `CostsLog` is its durable companion: each completed call is appended to the log, and the log is replayed on startup to reconstruct the in-memory database. This separation keeps the hot path fast (in-memory lookups) while maintaining durability (append-only JSONL).

---

## Cost-to-Routing Feedback Loop

Cost data feeds back into routing through two paths:

1. **Direct cost penalty in cascade router** — the confidence stage subtracts a cost penalty from each model's score, biasing toward cheaper models when pass rates are similar.
2. **Budget guardrail enforcement** — when budget limits approach, the guardrail system forces the router to select cheaper models or block requests entirely.

This creates cybernetic feedback loop 6 (Cost→Routing) from [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md): higher costs trigger routing changes that reduce costs, which relaxes the budget pressure, which may allow routing back to better (more expensive) models.

---

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — The cascade router uses cost data for its cost penalty computation.
- **[06-task-metrics-and-baselines](06-task-metrics-and-baselines.md)** — Task metrics include cost fields that come from cost normalization.
- **[09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)** — Provider health affects cost indirectly: unhealthy providers cause retries that increase total cost.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Loop 6 (Cost→Routing) describes the cost-to-routing feedback path.
- **[10-pareto-frontier-pruning](10-pareto-frontier-pruning.md)** — Pareto pruning uses cost_per_success as one of its optimization axes.
