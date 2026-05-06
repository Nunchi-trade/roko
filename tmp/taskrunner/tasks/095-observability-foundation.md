# Task 095: Observability Foundation (Prometheus + Tracing)

```toml
id = 95
title = "Add GET /metrics Prometheus endpoint and optional OTLP tracing export to roko-serve"
track = "infrastructure"
wave = "wave-3"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-serve/src/routes/metrics.rs",
    "crates/roko-serve/src/state.rs",
    "crates/roko-serve/Cargo.toml",
    "crates/roko-core/src/config/schema.rs",
]
exclusive_files = ["crates/roko-serve/src/routes/metrics.rs"]
estimated_minutes = 300
```

## Context

This task builds the observability infrastructure that all other metrics collection builds on
(task 096 wires actual measurements into it). Without this foundation, there is nowhere to
emit observations to and nothing to scrape.

Two problems from S24.2 and S24.3:

**S24.2 — No Prometheus scrape endpoint**: `roko-serve` has ~85 routes but no `GET /metrics`.
`MetricRegistry` in `roko-core` already renders Prometheus text format via
`render_prometheus()`, but the registry counters are never populated and there is no HTTP
route that exposes them. `GET /api/metrics/prometheus` exists in `routes/status/metrics.rs`
but it produces output via a hand-rolled `prom!` macro over `state_hub` snapshots, not from
`MetricRegistry`. The two outputs are disconnected.

**S24.3 — No distributed request tracing**: No `trace_id`, no `span_id`, no W3C
`traceparent`. Cannot trace a request from `roko serve` through agent dispatch and back.
`tracing-opentelemetry` is not a dependency. No OTLP export.

The redesign: this task creates a proper `GET /metrics` route (the standard Prometheus scrape
path), wires `MetricRegistry` to populate it, adds a `MetricsRegistry` accessor to
`AppState`, and optionally adds OTLP tracing export behind a `[serve.tracing]` config block.

Checklist items: S24.2, S24.3, Phase 5.4.

## Background

Read these files before writing any code:

1. `crates/roko-core/src/obs/metrics.rs` — `MetricRegistry` API: `register`, `get_counter`,
   `get_gauge`, `get_histogram`, `render_prometheus` (line 432), `register_standard_metrics`
   (line 599). Read the existing standard metric names. Understand `LabelSet` and how labeled
   counters and histograms work. Note: the registry is zero-external-dep — do not change this.
2. `crates/roko-core/src/obs/histograms.rs` — `Histogram` API: `observe`, `snapshot`.
   Understand the configurable bucket boundaries passed at construction time.
3. `crates/roko-serve/src/state.rs` — `AppState` struct. `pub metrics: Arc<MetricRegistry>`
   already exists (line 356). It is initialized as `Arc::new(MetricRegistry::new())` (line 610).
   `register_standard_metrics` is NOT called anywhere — that is the primary gap.
4. `crates/roko-serve/src/routes/status/metrics.rs` — existing `prometheus_metrics` handler
   (line 140). It uses the `prom!` macro over `state_hub` snapshots. Understand exactly what
   stats it produces so you can preserve them while appending the `MetricRegistry` output.
5. `crates/roko-serve/src/routes/mod.rs` — how routes are registered. The outer router (not
   under `/api`) is where `GET /metrics` should be added.
6. `crates/roko-serve/src/routes/status/mod.rs` — existing route registrations for
   `/api/metrics/*` paths. Do not remove these.
7. `crates/roko-core/src/config/schema.rs` — `ServeConfig` struct (around line 109). The new
   `[serve.tracing]` config block goes here.

## What to Change

### 1. Register standard metrics and additional metric names at `AppState` construction

In `crates/roko-serve/src/state.rs`, immediately after `Arc::new(MetricRegistry::new())`,
call `register_standard_metrics` and register the additional named metrics that task 096 will
emit into:

```rust
let metrics = Arc::new(MetricRegistry::new());
roko_core::obs::metrics::register_standard_metrics(&metrics);

// Additional per-call metrics — values emitted by task 096
// Register now so they appear in GET /metrics output even before first observation.
for (name, kind, help) in [
    ("roko_requests_total",              MetricKind::Counter,   "Total HTTP requests by route and method"),
    ("roko_request_duration_seconds",    MetricKind::Histogram, "HTTP request latency in seconds"),
    ("roko_active_agents",               MetricKind::Gauge,     "Number of currently active agents"),
    ("roko_gate_verdicts_total",         MetricKind::Counter,   "Gate rung verdict counts by rung and verdict"),
    ("roko_llm_calls_total",             MetricKind::Counter,   "LLM calls by provider and model"),
    ("roko_llm_ttft_seconds",            MetricKind::Histogram, "Time to first token in seconds by provider and model"),
    ("roko_llm_errors_total",            MetricKind::Counter,   "LLM errors by provider, model, and error_type"),
    ("roko_llm_cost_usd_total",          MetricKind::Counter,   "Cumulative LLM cost in USD by provider and model"),
    ("roko_tool_calls_total",            MetricKind::Counter,   "Tool dispatch call counts by tool and status"),
    ("roko_context_utilization",         MetricKind::Gauge,     "Context window fraction used by model"),
] {
    metrics.register(name, kind, help);
}
```

Match the actual `MetricRegistry::register` signature — it may differ from the pseudocode
above. If `register` takes `(name: &str, kind: MetricKind, help: &str)`, use that. If it
takes a builder, use the builder. Do NOT invent a new API.

Document the `metrics` field in `AppState` to indicate that task 096 wires the emission sites:

```rust
/// Prometheus-compatible metric registry. Populated by task 096 emission call sites
/// in provider dispatch, gate pipeline, and tool dispatch. Call
/// `state.metrics.get_counter(name, labels)` from any handler or middleware.
pub metrics: Arc<MetricRegistry>,
```

### 2. Create `crates/roko-serve/src/routes/metrics.rs`

This is a new file — it must not conflict with `routes/status/metrics.rs` (a different
module). Add:

```rust
//! Top-level GET /metrics handler — standard Prometheus scrape endpoint.
//!
//! Combines output from the `MetricRegistry` (labelled counters and histograms,
//! populated by provider dispatch and gate pipeline) with the existing state-hub
//! hand-rolled stats from `routes/status/metrics::prometheus_metrics`.
//!
//! Prometheus expects this endpoint at the root `/metrics` path without an `/api` prefix.

use std::sync::Arc;
use axum::{extract::State, http::header::CONTENT_TYPE, response::Response};
use http::StatusCode;

use crate::state::AppState;

/// Prometheus text exposition format content type (OpenMetrics compatible).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Handler for `GET /metrics`.
///
/// Returns Prometheus text format output combining:
/// 1. `MetricRegistry` output (labelled per-provider/model/gate counters and histograms)
/// 2. State-hub aggregate stats (uptime, active agent count, etc.)
pub async fn metrics_handler(
    State(state): State<Arc<AppState>>,
) -> Response {
    let mut output = String::with_capacity(4096);

    // Part 1: MetricRegistry labelled output
    output.push_str(&state.metrics.render_prometheus());

    // Part 2: State-hub aggregate stats (delegated to existing handler logic)
    // Re-use the state_hub snapshot to avoid duplicating that logic here.
    // If a shared helper exists in routes/status/metrics.rs, call it. If not, inline a
    // minimal version:
    if let Some(snapshot) = state.state_hub.try_current_snapshot() {
        let uptime_secs = snapshot.uptime_seconds.unwrap_or(0);
        output.push_str(&format!(
            "# HELP roko_uptime_seconds Server uptime in seconds\n\
             # TYPE roko_uptime_seconds gauge\n\
             roko_uptime_seconds {uptime_secs}\n"
        ));
        // Add any other state_hub fields not already in MetricRegistry output.
        // Do NOT duplicate metric names that are also in MetricRegistry.
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)
        .body(axum::body::Body::from(output))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::empty())
                .unwrap()
        })
}
```

Read the existing `state_hub` snapshot API before writing the Part 2 block. If
`state.state_hub.try_current_snapshot()` does not exist, use whatever method is available.
If the state-hub stats are already exposed completely by `render_prometheus()`, Part 2 can
be omitted — do not duplicate metric names.

### 3. Register `GET /metrics` at the top-level router in `routes/mod.rs`

In `build_router` (or equivalent function in `crates/roko-serve/src/routes/mod.rs`), add the
bare `/metrics` route to the outer (non-`/api`) router:

```rust
use crate::routes::metrics::metrics_handler;

// In the outer Router before the /api nest:
.route("/metrics", get(metrics_handler))
```

Do NOT remove `GET /api/metrics/prometheus`. Both must exist:
- `GET /metrics` — standard Prometheus scrape path (this task)
- `GET /api/metrics/prometheus` — existing path (keep for backward compat)

### 4. Add optional OTLP tracing export config to `ServeConfig`

In `crates/roko-core/src/config/schema.rs`, add a `tracing` sub-config to `ServeConfig`:

```rust
/// Optional distributed tracing export configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TracingConfig {
    /// OTLP gRPC endpoint for trace export (e.g. "http://localhost:4317").
    /// When absent, tracing export is disabled.
    pub otlp_endpoint: Option<String>,
    /// Service name reported in OTLP spans. Defaults to "roko-serve".
    #[serde(default = "default_service_name")]
    pub service_name: String,
    /// Sample rate 0.0–1.0. Default 1.0 (trace everything).
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
}

fn default_service_name() -> String { "roko-serve".to_string() }
fn default_sample_rate() -> f64 { 1.0 }
```

Add the field to `ServeConfig`:

```rust
pub struct ServeConfig {
    // ... existing fields ...
    /// Optional OTLP tracing export. Disabled when not configured.
    #[serde(default)]
    pub tracing: TracingConfig,
}
```

### 5. Initialize OTLP tracing export at server startup (conditional)

In `crates/roko-serve/src/lib.rs` (or wherever `serve` starts), after loading config, if
`serve_config.tracing.otlp_endpoint` is `Some`:

```rust
#[cfg(feature = "otlp")]
if let Some(endpoint) = &config.serve.tracing.otlp_endpoint {
    init_otlp_tracing(endpoint, &config.serve.tracing.service_name)?;
}
```

Gate this behind a `otlp` Cargo feature so the OTLP dependencies are opt-in:

In `crates/roko-serve/Cargo.toml`:
```toml
[features]
default = []
otlp = [
    "opentelemetry",
    "opentelemetry_sdk",
    "opentelemetry-otlp",
    "tracing-opentelemetry",
]

[dependencies]
# ... existing ...

[dependencies.opentelemetry]
version = "0.27"
optional = true

[dependencies.opentelemetry_sdk]
version = "0.27"
features = ["rt-tokio"]
optional = true

[dependencies.opentelemetry-otlp]
version = "0.27"
features = ["tonic"]
optional = true

[dependencies.tracing-opentelemetry]
version = "0.28"
optional = true
```

Check the latest compatible version of these crates in `crates.io` and use semver-compatible
versions that do not conflict with the workspace's existing `tracing` and `tracing-subscriber`
versions (currently `0.1` and `0.3`). The `opentelemetry` ecosystem has breaking changes
between minor versions — pin to a specific minor version.

The `init_otlp_tracing` function:

```rust
#[cfg(feature = "otlp")]
fn init_otlp_tracing(endpoint: &str, service_name: &str) -> anyhow::Result<()> {
    use opentelemetry_otlp::WithExportConfig;
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .with_trace_config(
            opentelemetry_sdk::trace::Config::default()
                .with_resource(opentelemetry_sdk::Resource::new(vec![
                    opentelemetry::KeyValue::new("service.name", service_name.to_string()),
                ])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)?;

    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
    // The tracing subscriber is already initialized elsewhere in the serve startup.
    // Use `tracing_subscriber::registry().with(telemetry)` only if no subscriber is set yet.
    // Otherwise, access the global subscriber and add the layer.
    // Check how tracing-subscriber is currently initialized in roko-serve before writing this.
    tracing::info!(endpoint, "OTLP tracing export enabled");
    Ok(())
}
```

If the tracing subscriber initialization is complex (layered), document the interaction in a
comment and leave the OTLP layer registration for a follow-up — the key deliverable here is
the config schema and the feature flag, not a fully functional OTLP export.

### 6. Add a `MetricsSink` helper to `AppState` for use by task 096 emission sites

Add a convenience method to `AppState` (in `state.rs`) that returns a cloneable reference
to the registry for passing into dispatch code that does not hold `Arc<AppState>`:

```rust
impl AppState {
    /// Return a cloneable `Arc<MetricRegistry>` for passing into dispatch helpers
    /// that do not hold a full `AppState` reference.
    pub fn metrics_sink(&self) -> Arc<MetricRegistry> {
        Arc::clone(&self.metrics)
    }
}
```

## What NOT to Do

- Do NOT add the `prometheus` crate or `metrics` crate as a dependency. `MetricRegistry` in
  `roko-core` is the authoritative Prometheus renderer and must remain zero-external-dep.
- Do NOT remove `GET /api/metrics/prometheus` — keep it and add the new `GET /metrics`
  alongside it.
- Do NOT emit observations in this task — task 096 handles that. This task only registers
  metric names, creates the HTTP route, and wires the renderer.
- Do NOT make OTLP a hard dependency. It must be behind the `otlp` feature flag and compile
  cleanly without it.
- Do NOT change `MetricRegistry::render_prometheus` or any other `roko-core` API. This task
  is a consumer, not a redesign of the metric primitives.
- Do NOT touch the existing `routes/status/metrics.rs` handler — only add the new top-level
  `routes/metrics.rs` and wire it to the router.

## Wire Target

```bash
# Start the server
cargo run -p roko-cli -- serve &

# Standard Prometheus scrape path (new)
curl -s http://localhost:6677/metrics | head -20
# Expected: Prometheus text format with HELP and TYPE lines for roko_llm_calls_total,
# roko_gate_verdicts_total, roko_active_agents, etc. Values will be 0 until task 096 wires
# the emission sites.

# Existing path must still work
curl -s http://localhost:6677/api/metrics/prometheus | head -5
# Expected: non-empty Prometheus text (existing hand-rolled stats)

# Config: optional OTLP block compiles and parses correctly
cat >> roko.toml <<'EOF'
[serve.tracing]
otlp_endpoint = "http://localhost:4317"
service_name = "roko-serve-dev"
sample_rate = 0.1
EOF
cargo run -p roko-cli -- serve 2>&1 | grep -i otlp
# Expected: "OTLP tracing export enabled" or similar (if OTLP feature enabled)
# Or no error if feature not enabled (config parsed but ignored)
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo build --workspace --features roko-serve/otlp` (if OTLP feature added)
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `crates/roko-serve/src/routes/metrics.rs` is a new file containing `metrics_handler`
- [ ] `GET /metrics` is registered in `routes/mod.rs` outer router
- [ ] `register_standard_metrics` called in `state.rs` `AppState` construction
- [ ] `roko_llm_calls_total` metric name registered (appears in `GET /metrics` output even at value 0)
- [ ] `roko_active_agents` gauge registered
- [ ] `roko_gate_verdicts_total` counter registered
- [ ] `curl http://localhost:6677/metrics` returns 200 with `Content-Type: text/plain; version=0.0.4`
- [ ] `curl http://localhost:6677/api/metrics/prometheus` still returns 200 (not broken)
- [ ] `TracingConfig` struct present in `roko-core/src/config/schema.rs`
- [ ] `ServeConfig` has `tracing: TracingConfig` field
- [ ] `[serve.tracing]` block in `roko.toml` parses without error
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file touched by this task

## Implementation Ground Truth (Worker 18 Enrichment)

Current code details to use instead of the pseudocode above:

- `MetricRegistry` has no generic `register(name, kind, help)` method. Use `register_counter(name, help, LabelSet::new())`, `register_gauge(...)`, and `register_histogram(name, help, LabelSet::new(), buckets)`.
- `register_standard_metrics(&MetricRegistry)` already exists in `roko-core/src/obs/metrics.rs` and registers seven schema-backed metric families. It is not called in `AppState` today.
- `AppState.metrics` already exists in `crates/roko-serve/src/state.rs`; initialization is currently `metrics: Arc::new(MetricRegistry::new())` inside `new_with_daimon_strategy_and_state_hub()`.
- Existing Prometheus text output is `routes/status/metrics.rs::prometheus_metrics(State(state))`, mounted as `/api/metrics/prometheus` by `routes/status/mod.rs`. That handler uses `state.state_hub.current_snapshot()`, `state.supervisor.count().await`, active plans, and episode file reads.
- Top-level router assembly is in `routes/mod.rs::build_router()`. The non-API router currently has `/health`, `/ready`, public webhooks/share routes, terminal routes, `.nest("/api", api)`, WebSockets, relay proxy, and SPA fallback. Add `/metrics` before the fallback and outside `/api`.
- `ServeConfig` is in `crates/roko-core/src/config/serve.rs`, re-exported through `config/mod.rs` and used by `schema.rs::RokoConfig`. The task's `touches` list naming `schema.rs` is stale for the actual config struct.
- `roko-serve` has no `src/main.rs`; startup is `crates/roko-serve/src/lib.rs::ServerBuilder::start_background()`. The CLI initializes global tracing elsewhere before calling into serve, so adding an OTLP layer after a subscriber is already installed may not work without a broader tracing bootstrap refactor.

## Mechanical Implementation Steps (Worker 18 Enrichment)

1. Initialize metric families in `state.rs`.
   - Replace the inline `Arc::new(MetricRegistry::new())` with:
     ```rust
     let metrics = Arc::new(MetricRegistry::new());
     roko_core::obs::metrics::register_standard_metrics(&metrics);
     register_observability_foundation_metrics(&metrics);
     ```
   - Add a private helper in `state.rs` that registers the task-096 metric names. Use zero-label registrations so `/metrics` shows them before first observation.
   - Use histogram buckets from `roko_core::obs::histograms` (for example `LLM_LATENCY_BUCKETS.to_vec()`) for duration/TTFT histograms. Do not invent bucket constants in `roko-serve`.
   - Add `pub fn metrics_sink(&self) -> Arc<MetricRegistry>` to `impl AppState`.

2. Create top-level metrics route.
   - Add `mod metrics;` in `routes/mod.rs`.
   - Create `crates/roko-serve/src/routes/metrics.rs` with `metrics_handler(State<Arc<AppState>>) -> Response`.
   - Start with `state.metrics.render_prometheus()`.
   - If adding state-hub aggregates, avoid duplicate names. `roko_active_agents` is planned for `MetricRegistry`; existing status output uses `roko_agents_active`. Keep both names only if intentional and documented.
   - Prefer not to call `routes/status/metrics::prometheus_metrics()` from the new module unless you first make a shared helper. Its module is currently `pub(super)` under `status`, and the task explicitly says not to edit `routes/status/metrics.rs`.

3. Register the route.
   - In `routes/mod.rs`, add `.route("/metrics", get(metrics::metrics_handler))` on the outer router near `/health` and `/ready`, before `.nest("/api", api)` and before `.fallback(...)`.
   - Do not wrap it in API-key middleware. Prometheus scrape auth can be added separately if needed; this task defines a standard public scrape path like `/health`.

4. Add tracing config in the correct config module.
   - Add `TracingConfig` to `crates/roko-core/src/config/serve.rs`.
   - Add `pub tracing: TracingConfig` to `ServeConfig`, default disabled with `otlp_endpoint: None`.
   - Re-export `TracingConfig` from `crates/roko-core/src/config/mod.rs` if callers need the type.
   - Add config parse/default tests in `serve.rs`, not `schema.rs`.

5. Add optional OTLP feature conservatively.
   - Add an `otlp` feature in `crates/roko-serve/Cargo.toml` with optional `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, and `tracing-opentelemetry` deps. If the root workspace does not already define those versions, add pinned compatible versions there.
   - In `ServerBuilder::start_background()`, after `let roko_config = state.load_roko_config();` and before building routes, call a small `init_otlp_tracing_if_configured(&roko_config.serve.tracing)` behind `#[cfg(feature = "otlp")]`.
   - If a global subscriber is already installed, log a clear warning and skip layer installation rather than panicking. A no-op-with-warning is acceptable for this foundation task as long as config parsing and `--features roko-serve/otlp` compile.

## Tests and Verification Details (Worker 18 Enrichment)

- Add a route test in `routes/mod.rs` or `routes/metrics.rs` that builds an `AppState`, calls `/metrics`, asserts `200`, `Content-Type` contains `text/plain`, and body contains `# HELP roko_llm_calls_total`, `# TYPE roko_gate_verdicts_total counter`, and `roko_active_agents`.
- Add a regression test that `/api/metrics/prometheus` still returns 200 through the existing status route.
- Add a `state.rs` unit test for `metrics_sink()` returning the same registry and for standard/task-096 metric families being present after `AppState::new(...)`.
- Add `ServeConfig` TOML parse tests for missing `[serve.tracing]` (disabled), configured endpoint/service/sample rate, and sample-rate default.
- Verification commands should include:
  ```bash
  cargo build -p roko-serve
  cargo build -p roko-serve --features otlp
  cargo test -p roko-serve metrics
  cargo test -p roko-core serve_tracing
  ```

## Scope Notes (Worker 18 Enrichment)

The current `touches` list is incomplete for the stated implementation. Required files include `crates/roko-serve/src/routes/mod.rs`, `crates/roko-serve/src/lib.rs`, `crates/roko-core/src/config/serve.rs`, and likely `crates/roko-core/src/config/mod.rs` in addition to the listed files. If OTLP dependencies are not already in the workspace, root `Cargo.toml` must also be in scope. Do not edit `routes/status/metrics.rs` unless the task owner explicitly allows extracting a shared helper.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
