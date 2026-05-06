# Task 096: Observability Metrics Wiring

```toml
id = 96
title = "Wire real TTFT, gate verdicts, cost, error rates, and context utilization into MetricRegistry"
track = "infrastructure"
wave = "wave-3"
priority = "high"
blocked_by = [95, 82]
touches = [
    "crates/roko-core/src/chat_types.rs",
    "crates/roko-core/src/obs/schema.rs",
    "crates/roko-serve/src/routes/status/metrics.rs",
    "crates/roko-serve/src/routes/bench.rs",
    "crates/roko-serve/src/routes/plans.rs",
    "crates/roko-serve/src/runtime.rs",
    "crates/roko-serve/src/state.rs",
    "crates/roko-agent/src/openai_compat_backend.rs",
    "crates/roko-agent/src/model_call_service.rs",
    "crates/roko-agent/src/task_runner.rs",
    "crates/roko-agent/src/gateway_events.rs",
    "crates/roko-orchestrator/src/service_factory.rs",
    "crates/roko-cli/src/runner/gate_dispatch.rs",
    "crates/roko-cli/src/runner/types.rs",
    "crates/roko-cli/src/serve_runtime.rs",
    "crates/roko-learn/src/runtime_feedback.rs",
    "crates/roko-learn/src/regression.rs",
]
exclusive_files = []
estimated_minutes = 360
```

## Context

Task 095 registers metric names and exposes Prometheus output. In this checkout the route is
`GET /api/metrics/prometheus`. This task wires the actual
measurements so the counters and histograms have real values.

The following gaps from S24.1 and S24.4–24.10 are all addressed here:

- **S24.1** — `ResponseMetadata.provider_latency_ms` exists but is never populated. TTFT is
  derived from internal signal timestamps (includes queueing) rather than actual HTTP-layer
  timing. Real TTFT must come from the streaming backend: time-to-first-chunk measured at the
  HTTP client boundary.
- **S24.4** — Bench `gate_verdicts: Vec::new()` and `retries_used: 0` hardcoded. Gate verdict
  data from the pipeline must flow into both `roko_gate_verdicts_total` and the bench response.
- **S24.5** — Bench cost uses hardcoded per-token rates by model name substring. Must use
  `CostTable` from `roko-agent`.
- **S24.6** — No error rate tracking by category. `GatewayEvent.error` is a freeform string.
  Need `roko_llm_errors_total{model,provider,error_type}` where `error_type` is an enum
  variant name (RateLimit, Timeout, ModelError, NetworkError, etc.).
- **S24.7** — HTTP API cannot answer "what did plan X cost?" — gateway events have `caller`
  but not plan/task hierarchy.
- **S24.8** — No throughput metrics. `LatencyStats` tracks tokens/sec per model but no
  aggregate tokens/second.
- **S24.9** — Context window utilization not recorded per call.
- **S24.10** — Bench regression detection not wired (`regression.rs` exists but is never
  called from bench).

All of these are emission call sites, not infrastructure. Task 095 provides the registry;
this task populates it.

Checklist items: S24.1, S24.4–10, Phase 2.4.

## Background

Read these files before writing any code:

1. `crates/roko-agent/src/provider/mod.rs` (or wherever the streaming backend trait lives
   after task 082's streaming-first redesign). Find where the first HTTP chunk arrives and
   where request completion is signaled. This is where TTFT and total duration are measured.
   If task 082 is not yet merged, look at `crates/roko-agent/src/openai_compat_backend.rs`
   lines 460–520: the TTFT timeout fires at `DEFAULT_TTFT_TIMEOUT_MS = 15_000`. The real TTFT
   is the elapsed time from HTTP request send to the first non-error chunk received.
2. `crates/roko-agent/src/model_call_service.rs` — `ModelCallService::dispatch` or
   `send_turn`. This is the central serve-path dispatch point. It already holds a
   `cost_table: CostTable`. The provider call result includes `Usage` with token counts.
   Add `metrics: Option<Arc<MetricRegistry>>` field here and emit on each call.
3. `crates/roko-agent/src/task_runner.rs` — `CostTable::calculate` at line 478 computes
   `iteration_cost_usd`. This is the emission site for `roko_llm_cost_usd_total`. The
   `model_slug` and `Usage` are both available here.
4. `crates/roko-serve/src/routes/bench.rs` — lines 294–296: `gate_verdicts: Vec::new()` and
   `retries_used: 0` hardcoded. Lines 700–713: hardcoded per-1K-token rate lookup by model
   substring. Both must be replaced with real data.
5. `crates/roko-serve/src/routes/plans.rs` — existing plan query routes. The new
   `GET /api/plans/{id}/costs` endpoint goes here.
6. `crates/roko-learn/src/runtime_feedback.rs` — `RuntimeFeedbackWrite` (line 775) and
   `RuntimeFeedbackSnapshot` (line 828). The per-task cost aggregation data lives in
   `AgentEfficiencyEvent` and related structs. Read how efficiency events are structured to
   build the per-plan cost breakdown.
7. `crates/roko-core/src/obs/metrics.rs` — `MetricRegistry::register_counter`,
   `register_histogram`, `register_gauge`, and `LabelSet`. Register methods return handles
   you call `.inc()`, `.inc_by(n)`, `.set(n)`, or `.observe(v)` on. `get_*` methods only
   look up existing handles and must not be used as create-or-get helpers.
8. `crates/roko-learn/src/regression.rs` — what the regression detector expects (bench run
   comparison type). Also read `routes/bench.rs` to understand what `BenchRun` contains and
   where historical runs are stored on disk.

## Current Checkout Corrections

These notes are authoritative for this checkout and override stale examples below:

- The Prometheus route is `GET /api/metrics/prometheus` in
  `crates/roko-serve/src/routes/status/metrics.rs`, not `GET /metrics` and not
  `crates/roko-serve/src/routes/metrics.rs`. `prometheus_metrics()` currently hand-renders
  uptime/task/gate lines and never appends `state.metrics.render_prometheus()`. Append the
  registry output there so dynamic `MetricRegistry` families become visible.
- `MetricRegistry` does not create metrics from `get_counter("name", &[...])`. Build labels
  with `LabelSet::from_pairs(&[("provider", provider), ...])`, then call `register_counter`,
  `register_gauge`, or `register_histogram`. `get_*` only returns an existing handle.
- `Gauge::set()` takes `i64`, not `f64`. Store scaled integers for fractional values:
  context utilization in basis points or percent, and throughput as integer tokens/sec.
  Document the scale in the metric help string.
- The canonical schema in `crates/roko-core/src/obs/schema.rs` currently lacks
  `roko_llm_calls_total`, `roko_llm_errors_total`, `roko_llm_ttft_seconds`,
  `roko_llm_request_duration_seconds`, `roko_context_utilization`, and
  `roko_token_throughput_per_second`. Add descriptors/constants there or register the
  families explicitly at serve startup before emitting. Do not assume task 095 added them.
- `ResponseMetadata` in `crates/roko-core/src/chat_types.rs` has
  `provider_latency_ms` but no `provider_ttft_ms`. Either add `provider_ttft_ms:
  Option<u64>` and populate it, or explicitly store TTFT only in metrics; do not write code
  that references a nonexistent field.
- The OpenAI-compatible streaming path is
  `OpenAiCompatLlmBackend::send_turn_streaming()` in
  `crates/roko-agent/src/openai_compat_backend.rs`. The existing TTFT timeout starts at the
  first `response.chunk()` after HTTP headers. Real S24.1 TTFT must start before
  `.send().await` and stop at the first parsed non-error content/tool/reasoning stream chunk.
- `ModelCallService::call()` in `crates/roko-agent/src/model_call_service.rs` is the
  serve-path gateway. Success, cache-hit, provider-call error, and convergence-error branches
  all write gateway events; emit call/error/token/cost/context/throughput metrics in those
  same branches so cache hits and failures are counted.
- `ModelCallService` is built in `crates/roko-orchestrator/src/service_factory.rs`, then
  installed into `AppState` in `crates/roko-serve/src/state.rs`. Add a
  `with_metrics(Arc<MetricRegistry>)` builder or extend `ServiceConfig` so serve can pass
  `state.metrics` without introducing any `roko-serve` dependency into `roko-agent`.
- Bench execution uses `CliRuntime::run_once_with_config()` and receives
  `RunResult { usage, gate_results }` from `crates/roko-serve/src/runtime.rs`.
  `RokoCliRuntime::run_once_with_config()` in `crates/roko-cli/src/serve_runtime.rs`
  currently returns `gate_results: Vec::new()`. Wire this from `crate::run::RunReport`
  (`gate_verdicts`) instead of fabricating bench gate data in `bench.rs`.
- `AgentEfficiencyEvent` fields are concrete values (`plan_id: String`,
  `task_id: String`, `cost_usd: f64`), not `Option`s. The plan cost endpoint should skip
  events whose `plan_id != requested_id`, sum `cost_usd` by `task_id`, and return an empty
  task list with total `0.0` when the file is missing.

## Recovery Worker 19 Checkout Notes

Use these concrete call chains and APIs when implementing:

- Serve model-call chain:
  `roko-cli serve` -> `roko-serve::state::AppState::new*` ->
  `roko-orchestrator::service_factory::ServiceFactory::build` ->
  `roko_agent::model_call_service::ModelCallService::call` ->
  `ProviderCallCell::execute` -> provider backend such as
  `OpenAiCompatLlmBackend::send_turn_streaming`.
- `AppState` currently creates `metrics: Arc::new(MetricRegistry::new())` inside the final
  struct literal. Create `let metrics = Arc::new(MetricRegistry::new());` before
  `ServiceFactory::build(...)`, pass a clone through `ServiceConfig` or a builder into
  `ModelCallService`, and store the same clone in `AppState.metrics`.
- `MetricRegistry` is create-or-get through `register_counter`, `register_gauge`, and
  `register_histogram`; `Counter::inc_by` takes `u64`, `Gauge::set` takes `i64`, and
  histogram registration needs explicit bucket boundaries. Use
  `roko_core::obs::histograms::LLM_LATENCY_BUCKETS.to_vec()` for TTFT/request duration
  histograms. Use constants in `obs/schema.rs` for metric and label names, including a new
  `LABEL_ERROR_TYPE`.
- Real TTFT in `openai_compat_backend.rs` starts before `.send().await`, but should stop only
  when the stream parser has accepted the first non-error content/reasoning/tool delta. Do
  not count HTTP headers, empty keepalive bytes, comments, metadata-only chunks, or error
  chunks as the first token. Observe total request duration on every return path: success,
  HTTP status error, send error, read error, parse error, and TTFT timeout.
- `ResponseMetadata.provider_latency_ms` exists; `provider_ttft_ms` does not. If adding it,
  update `crates/roko-core/src/chat_types.rs` and populate it only on real streaming
  responses. Cache hits should not invent TTFT.
- `ModelCallService::call()` has distinct cache-hit, provider-error, convergence-error, and
  success branches that already write gateway events. Emit call/error/token/cost/context and
  throughput metrics in those same branches so every return is counted exactly once. Cache
  hits count as `status="cache_hit"` with token/cost values from the cached response if
  available.
- Context utilization should use `self.config.effective_models()` to find a matching model
  key or slug and `ModelProfile.context_window`. If no context window is configured, skip the
  gauge or use a clearly documented fallback; do not hardcode a context size silently.
- Bench chain:
  `POST /api/bench/run` -> `routes/bench.rs::start_bench_run` ->
  `execute_bench_run` -> `state.runtime.run_once_with_config(...)` ->
  `RokoCliRuntime::run_once_with_config`. The current prompt bench path returns
  `gate_results: Vec::new()` from `serve_runtime.rs`; only populate bench
  `gate_verdicts` when `RunResult.gate_results` contains real gate results. Do not fabricate
  gate data in `routes/bench.rs`.
- `crates/roko-serve/src/bench.rs::estimate_cost_usd()` is outside this task's touch list.
  Replace bench cost at call sites in `routes/bench.rs` with a local helper that uses
  `roko_agent::task_runner::CostTable::from_config_with_defaults()` plus the effective
  config, or expand the touch list before editing `bench.rs`.
- `RunResult.usage` in `crates/roko-serve/src/runtime.rs` is not `roko_agent::Usage`.
  Convert its token fields into a temporary `roko_agent::Usage` with cache tokens and
  wall/cost fields set to zero before calling `CostTable::calculate()`.
- The plan-cost endpoint can use
  `state.workdir.join(".roko/learn/efficiency.jsonl")` or the existing projection helper
  pattern from `routes/status/metrics.rs` (`RuntimeProjectionSet::load(&state)`) if it gives
  bounded access to `AgentEfficiencyEvent`. Missing efficiency data is a successful empty
  response, not a 404.
- `routes/plans.rs` handlers return `Result<..., ApiError>` and the router currently uses
  Axum's brace syntax (`/plans/{id}`), not `:id`. Add
  `.route("/plans/{id}/costs", get(plan_costs))` and keep validation/error style consistent
  with the neighboring plan handlers.
- `detect_regressions()` has signature
  `detect_regressions(&Baseline, &[TaskMetric], &RegressionThresholds)`. Build a historical
  baseline with `roko_learn::baseline::compute_baseline(previous_records,
  thresholds.min_records)`, then compare current bench `TaskMetric` records. `TaskMetric` is
  `roko_core::metric::TaskMetric`; populate every required field, including
  `ConfigHash`, role/backend/model/complexity/gate, token/cost fields, and `cache_hit_rate`.
- Runner gate verdict metrics should be emitted where `GateCompletion` is processed, not from
  `gate_dispatch.rs` alone. In `event_loop.rs`, each `completion.verdicts` entry has a real
  `gate_name` and `passed` value; use labels `{gate=gate_name, verdict="pass"|"fail"}`.
  If `RunConfig` gets a metrics field, initialize it in every `RunConfig { ... }` literal
  found by `rg -n "RunConfig \\{" crates/roko-cli/src`.
- Add focused tests instead of relying only on workspace tests: schema constants include the
  new metric families, Prometheus output appends a registered registry metric, error
  categorization maps timeout/rate-limit examples, plan costs aggregate two tasks and missing
  files return total `0.0`, bench maps real `RunResult.gate_results`, and regression detection
  calls the actual baseline API.

## Mechanical Implementation Plan

1. Add the missing metric descriptors/constants in `obs/schema.rs`, including labels:
   `provider`, `model`, `status`, `error_type`, `direction`, and `gate`/`verdict` where
   applicable. Keep existing metric names unchanged.
2. Make `prometheus_metrics()` append `state.metrics.render_prometheus()` after its existing
   hand-rendered process metrics. Preserve `GET /api/metrics` JSON behavior.
3. Thread `Arc<MetricRegistry>` from serve state through `ServiceFactory` into
   `ModelCallService` and, if needed, into the OpenAI-compatible backend constructor.
4. In `OpenAiCompatLlmBackend::send_turn_streaming()`, capture request start before
   `.send().await`, detect the first parsed non-error stream chunk exactly once, observe
   `roko_llm_ttft_seconds`, and observe total request duration on success and error.
5. In `ModelCallService::call()`, emit:
   `roko_llm_calls_total{provider,model,status}`, `roko_llm_errors_total{provider,model,error_type}`,
   `roko_llm_tokens_total{provider,model,direction}`, `roko_llm_cost_usd_total{provider,model}`
   as microdollar counters, `roko_context_utilization{provider,model}` as basis points, and
   `roko_token_throughput_per_second{provider,model}` as integer tokens/sec.
6. Extend `GatewayEvent` only if the plan-cost endpoint needs durable plan/task hierarchy.
   Prefer writing `caller` as a stable `plan_id/task_id` value from request context if the
   existing request already carries it; otherwise document the missing upstream request field.
7. Wire `RunConfig` or gate completion handling so gate completions increment
   `roko_gate_verdicts_total{gate,verdict}`. Use the existing verdict names in
   `GateVerdictSummary`; do not invent rung-only labels if gate names are available.
8. In bench, map `RunResult.gate_results` into `BenchTaskResult.gate_verdicts` and populate
   `retries_used` from the runtime when available. If retries are not exposed, add a runtime
   field rather than leaving a hardcoded zero.
9. Replace bench hardcoded cost estimation with `CostTable::from_config_with_defaults()` from
   `roko-agent/src/task_runner.rs` using the effective `RokoConfig` models.
10. Add `GET /api/plans/{id}/costs` in `routes/plans.rs`, reading at most the last bounded
    chunk of `.roko/learn/efficiency.jsonl`, parsing `AgentEfficiencyEvent`, and returning
    deterministic task ordering.
11. After a bench run completes, convert current and previous run summaries into
    `TaskMetric` records if `detect_regressions()` needs `TaskMetric`, then log or surface
    `RegressionReport` alerts. Do not add a fake `RegressionDetector` type.

## What to Change

### 1. Real TTFT measurement in the streaming backend

In the provider streaming path (after task 082, this is in the `LlmBackend::stream_turn`
implementation; before task 082, it is in `OpenAiCompatBackend`):

```rust
let request_start = std::time::Instant::now();

// ... send HTTP request ...

// When the first streaming chunk arrives (non-error):
let ttft_secs = request_start.elapsed().as_secs_f64();

// Populate ResponseMetadata after adding the field in chat_types.rs:
metadata.provider_ttft_ms = Some((ttft_secs * 1000.0) as u64);

// Also emit to MetricRegistry if a registry handle is available:
if let Some(registry) = &self.metrics {
    let labels = LabelSet::from_pairs(&[
        ("provider", provider_id.as_str()),
        ("model", model_slug.as_str()),
    ]);
    registry
        .register_histogram("roko_llm_ttft_seconds", "HTTP time to first token", labels, ttft_buckets())
        .observe(ttft_secs);
}
```

Add `metrics: Option<Arc<MetricRegistry>>` to `OpenAiCompatBackend` (or the streaming backend
type) defaulting to `None`. Wire it from `ModelCallService` when constructing the backend in
the serve path.

Also measure total request duration: start timer before the HTTP send, record elapsed when
the stream closes or errors, emit to `roko_llm_request_duration_seconds`.

### 2. Emit `roko_llm_calls_total` and `roko_llm_errors_total` from `ModelCallService`

In `crates/roko-agent/src/model_call_service.rs`, add `metrics: Option<Arc<MetricRegistry>>`
field (default `None`). In the dispatch result handler:

```rust
// On success:
if let Some(ref registry) = self.metrics {
    let labels = LabelSet::from_pairs(&[
        ("provider", provider_id.as_str()),
        ("model", model.as_str()),
        ("status", "success"),
    ]);
    registry.register_counter("roko_llm_calls_total", "LLM calls by status", labels).inc();

    // Context utilization
    let utilization_bps = ((usage.input_tokens as f64
        / model_profile.context_window.unwrap_or(200_000) as f64) * 10_000.0) as i64;
    registry
        .register_gauge(
            "roko_context_utilization",
            "Context window utilization in basis points",
            LabelSet::from_pairs(&[("provider", provider_id.as_str()), ("model", model.as_str())]),
        )
        .set(utilization_bps);
}

// On error (categorize the error kind):
if let Some(ref registry) = self.metrics {
    let error_type = categorize_error(&err); // "rate_limit" | "timeout" | "model_error" | "network_error" | "unknown"
    let labels = LabelSet::from_pairs(&[
        ("provider", provider_id.as_str()),
        ("model", model.as_str()),
        ("error_type", error_type),
    ]);
    registry.register_counter("roko_llm_errors_total", "LLM errors by type", labels).inc();
    registry
        .register_counter(
            "roko_llm_calls_total",
            "LLM calls by status",
            LabelSet::from_pairs(&[
                ("provider", provider_id.as_str()),
                ("model", model.as_str()),
                ("status", "error"),
            ]),
        )
        .inc();
}
```

Add a private `fn categorize_error(err: &RokoError) -> &'static str` that matches on the
error variant and returns a lowercase snake_case string. Match the known error types:
`RateLimit` / `Throttled` → `"rate_limit"`, `TimedOut` / timeout strings → `"timeout"`,
`ModelError` / `InternalServerError` → `"model_error"`, network errors → `"network_error"`,
everything else → `"unknown"`.

### 3. Emit cost from `TaskRunner`

In `crates/roko-agent/src/task_runner.rs`, at the `CostTable::calculate` call site
(line 478), add metric emission:

```rust
let cost_usd = self.cost_table.calculate(&self.model_slug, &result.usage);

if let Some(ref registry) = self.metrics {
    let labels = LabelSet::from_pairs(&[
        ("provider", self.provider_id.as_str()),
        ("model", self.model_slug.as_str()),
    ]);
    // Use integer microdollars to avoid float accumulation issues
    let cost_micro = (cost_usd * 1_000_000.0) as u64;
    registry
        .register_counter("roko_llm_cost_usd_total", "Cumulative LLM spend in microdollars", labels)
        .inc_by(cost_micro);
}
```

If `Counter::inc_by` takes `u64` only, this works. If it takes `f64`, use that directly.
Check the actual `Counter` API before writing this.

Add `metrics: Option<Arc<MetricRegistry>>` and `provider_id: String` fields to `TaskRunner`.
Wire from `ModelCallService`.

### 4. Emit `roko_gate_verdicts_total` from the runner gate dispatch

In `crates/roko-cli/src/runner/gate_dispatch.rs` (or wherever gate completion events are
processed in the event loop), emit after each rung verdict:

```rust
if let Some(ref registry) = config.metrics {
    let rung_str = rung.to_string();
    let verdict_str = if passed { "pass" } else { "fail" };
    let labels = LabelSet::from_pairs(&[("gate", rung_str.as_str()), ("verdict", verdict_str)]);
    registry
        .register_counter("roko_gate_verdicts_total", "Verify verdicts by gate and verdict", labels)
        .inc();
}
```

Pass `Option<Arc<MetricRegistry>>` through `RunConfig` (which already holds the cascade
router and other cross-cutting infrastructure). Do NOT add a `roko-serve` dependency to
`roko-cli` — pass the registry as a plain `Arc<MetricRegistry>` from `roko-core`.

### 5. Fix bench gate verdicts and cost

In `crates/roko-serve/src/routes/bench.rs`:

**Gate verdicts (S24.4)**: Replace `gate_verdicts: Vec::new()` with real data. The bench run
calls the gate pipeline and receives `GateResult` or similar. Thread those results into the
response struct. Read the bench handler to find where `GateCompletion` or gate pass/fail data
is available (it likely comes back through a channel or `JoinHandle`). If wiring is complex,
at minimum record `retries_used` from the task retry counter.

**Cost via CostTable (S24.5)**: Replace the hardcoded rate table (lines 700–713) with:
```rust
use roko_agent::task_runner::CostTable;
// `roko_config` is already available in the bench handler
let cost_table = CostTable::from_config_with_defaults(&roko_config.models);
let cost_usd = cost_table.calculate(&model_slug, &usage);
```

Check whether `CostTable::from_config` exists. If not, use `CostTable::default()` and
`cost_table.insert(slug, input_rate, output_rate)` from the model profile's pricing fields.
The model profile pricing is in `ModelProfile` in `roko-core/src/config/schema.rs`.

### 6. Add `GET /api/plans/{id}/costs` endpoint

In `crates/roko-serve/src/routes/plans.rs`, add a new handler:

```rust
pub async fn plan_costs(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(plan_id): axum::extract::Path<String>,
) -> Result<Json<PlanCostBreakdown>, StatusCode> {
    // Read efficiency events from .roko/learn/efficiency.jsonl filtered by plan_id.
    // The `AgentEfficiencyEvent` struct has a `plan_id` or `caller` field — check.
    // Group by task_id, sum cost_usd per task.
    let events = tokio::fs::read_to_string(
        state.workdir.join(".roko/learn/efficiency.jsonl")
    ).await.unwrap_or_default();

    let mut task_costs: std::collections::HashMap<String, f64> = Default::default();
    let mut total = 0.0f64;

    for line in events.lines() {
        if let Ok(event) = serde_json::from_str::<AgentEfficiencyEvent>(line) {
            if event.plan_id == plan_id {
                *task_costs.entry(event.task_id.clone()).or_default() += event.cost_usd;
                total += event.cost_usd;
            }
        }
    }

    Ok(Json(PlanCostBreakdown {
        plan_id,
        total_cost_usd: total,
        tasks: task_costs.into_iter()
            .map(|(task_id, cost_usd)| TaskCostEntry { task_id, cost_usd })
            .collect(),
    }))
}

#[derive(Serialize)]
pub struct PlanCostBreakdown {
    pub plan_id: String,
    pub total_cost_usd: f64,
    pub tasks: Vec<TaskCostEntry>,
}

#[derive(Serialize)]
pub struct TaskCostEntry {
    pub task_id: String,
    pub cost_usd: f64,
}
```

Read `AgentEfficiencyEvent` in `roko-learn` first — it may use different field names than
`plan_id`/`task_id`/`cost_usd`. Match the actual struct. If it does not have a `plan_id`
field, use the `caller` field (which should be the role/task identifier).

Register the route in the plans router:
```rust
.route("/plans/{id}/costs", get(plan_costs))
```

### 7. Wire bench regression detection

In `crates/roko-serve/src/routes/bench.rs`, after a bench run completes and is persisted,
call the existing `roko_learn::regression::detect_regressions()` API. It expects
`TaskMetric` slices plus `RegressionThresholds`, so convert bench task results into
`TaskMetric` records and compare the current run against previous runs from the same suite.
Log every `RegressionReport::regressions()` entry with suite id, metric name, baseline,
current value, and change fraction. Do not invent a `RegressionDetector` type.

### 8. Throughput metric: tokens/sec rolling window

In `ModelCallService` (or wherever `Usage` is available after a model call), emit:

```rust
if let Some(ref registry) = self.metrics {
    let throughput = usage.output_tokens as f64 / duration_secs;
    // Use a rolling gauge — update the gauge to the current call's throughput.
    // A more accurate implementation would maintain a EWMA, but the gauge is good enough
    // for alerting on sudden drops.
    registry
        .register_gauge(
            "roko_token_throughput_per_second",
            "Output token throughput for the latest call",
            LabelSet::from_pairs(&[("provider", provider_id.as_str()), ("model", model.as_str())]),
        )
        .set(throughput.round() as i64);
}
```

Register `roko_token_throughput_per_second` in task 095's startup block if not already there,
or add it to the registration in `state.rs` if this task merges first.

## What NOT to Do

- Do NOT add the `prometheus` crate. Use `MetricRegistry` from `roko-core`.
- Do NOT add a `roko-serve` dependency to `roko-agent` or `roko-cli`. Pass
  `Option<Arc<MetricRegistry>>` as a constructor parameter — `MetricRegistry` lives in
  `roko-core` which all crates already depend on.
- Do NOT try to retrofit every provider backend individually. Wire only
  `OpenAiCompatBackend` (which covers most calls) and `ModelCallService` (which wraps all
  serve-path calls).
- Do NOT block the `plan_costs` handler on a full scan of `efficiency.jsonl` without
  pagination or a size guard. Add a `head_limit` (e.g., last 50 000 lines) to prevent OOM.
- Do NOT remove or rename any existing metric routes or bench response fields.
- Do NOT change gate threshold logic, cascade router, or episode logger.
- Do NOT attempt to wire OTLP span emission in this task — that is task 095's OTLP section.
- Do NOT leave examples that call `get_counter()`/`get_gauge()` with raw label slices in
  production code. Use `LabelSet` plus `register_*`.
- Do NOT publish `roko_context_utilization` as a floating value with the current gauge type.

## Wire Target

```bash
# Start the server
cargo run -p roko-cli -- serve &

# Dispatch a model call through the API
curl -X POST http://localhost:6677/api/run \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Say hello in one word"}'

# Verify metrics incremented
curl -s http://localhost:6677/api/metrics/prometheus | grep roko_llm_calls_total
# Expected: roko_llm_calls_total{provider="...",model="...",status="success"} 1

curl -s http://localhost:6677/api/metrics/prometheus | grep roko_llm_ttft_seconds
# Expected: roko_llm_ttft_seconds_bucket / roko_llm_ttft_seconds_sum lines with non-zero values

# Run a plan with a gate pipeline
cargo run -p roko-cli -- plan run plans/ &
sleep 30 && kill %1
curl -s http://localhost:6677/api/metrics/prometheus | grep roko_gate_verdicts_total
# Expected: roko_gate_verdicts_total{rung="1",verdict="pass"} N (some positive N)

# Per-plan cost endpoint
curl -s http://localhost:6677/api/plans/my-plan-id/costs
# Expected: JSON with total_cost_usd and tasks array
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `ResponseMetadata.provider_ttft_ms` is populated by the streaming backend
- [ ] `OpenAiCompatBackend` (or streaming backend) has `metrics: Option<Arc<MetricRegistry>>` field
- [ ] `ModelCallService` has `metrics: Option<Arc<MetricRegistry>>` field
- [ ] After a model call: `roko_llm_calls_total` > 0 in `GET /api/metrics/prometheus` output
- [ ] After a model call: `roko_llm_ttft_seconds` histogram has non-zero sum
- [ ] After a model call: `roko_llm_cost_usd_total` > 0 in `GET /api/metrics/prometheus` output
- [ ] `roko_context_utilization` gauge has a value after a model call
- [ ] Gate verdicts from the runner populate `roko_gate_verdicts_total`
- [ ] `bench.rs` no longer has `gate_verdicts: Vec::new()` hardcoded (uses real gate data)
- [ ] `bench.rs` cost calculation uses `CostTable` instead of hardcoded rate table
- [ ] `GET /api/plans/{id}/costs` returns 200 with `total_cost_usd` and `tasks` array
- [ ] Bench regression detection called after bench run completes
- [ ] `fn categorize_error` exists in `model_call_service.rs` and returns non-`"unknown"` for
      rate-limit and timeout errors
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any new file

## Status Log

| Time | Agent | Action |
|------|-------|--------|
