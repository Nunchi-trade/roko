# Observability as a Lens Pipeline

> Depth for [14-observability-and-telemetry.md](../../docs/19-deployment/14-observability-and-telemetry.md). Redesigns production observability as a Pipeline of Lens Cells -- read-only projections over Bus and Store. Structured logs are Bus Pulses. Metrics are numeric Lens outputs. Traces are lineage-annotated Signals. Dashboards are named Lens compositions.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, lineage, content addressing), [02-CELL](../../unified/02-CELL.md) (Observe protocol, Lens specialization), [03-GRAPH](../../unified/03-GRAPH.md) (Pipeline pattern), [04-BUS-AND-STORE](../../unified/04-BUS-AND-STORE.md) (Bus topics, Store queries)

---

## 1. The Core Insight: Observability Is the Observe Protocol

Observability is not a separate system bolted onto the runtime. It is the **Observe protocol** applied systematically. Every telemetry surface -- logs, metrics, traces, dashboards, cost reports, replay -- is a Lens Cell (a Cell conforming to the Observe protocol) that reads Signals and Pulses without mutating them.

This means:

- Adding a new metric = adding a new Lens Cell to a Graph.
- Adding a new dashboard panel = composing existing Lens Cells into a named projection.
- Telemetry never interferes with the runtime because Lens Cells are read-only by contract.
- Every telemetry surface speaks the same language (Signals in, observation Signals out).

---

## 2. The Telemetry Pipeline

Observability is a **Pipeline pattern** -- a linear chain of Lens Cells where each stage transforms the raw Bus/Store data into progressively more useful operator views.

```
Bus (Pulses)  ──┐
                 ├──▶ [Collector Lens] ──▶ [Transform Lens] ──▶ [Export Lens] ──▶ sink
Store (Signals) ─┘
```

Three stages:

| Stage | Cell | Responsibility |
|---|---|---|
| **Collect** | `CollectorLens` | Subscribe to Bus topics and/or query Store. Filter by relevance. |
| **Transform** | `TransformLens` | Parse, aggregate, sample, or reshape. Numeric projection for metrics. Span assembly for traces. |
| **Export** | `ExportLens` | Serialize to the target format and deliver to the sink (stdout, `/metrics`, OTLP collector, StateHub). |

Each stage is a Cell. The Pipeline is a Graph. Adding a new telemetry surface means adding a new Pipeline instance with appropriate Lens Cells -- no changes to the runtime itself.

---

## 3. Structured Logs as Bus Pulses

Structured logs are not a separate I/O channel. They are **Pulses published to `telemetry.log.*` topics** on the Bus, consumed by a LogExportLens that serializes them to stdout.

### Log Pulse Schema

```rust
/// A log entry is a Pulse on the Bus with topic "telemetry.log.{level}".
///
/// The LogExportLens subscribes to telemetry.log.* and writes JSON to stdout.
/// In human mode, it formats the same Pulse differently.
pub struct LogPulse {
    pub ts: DateTime<Utc>,
    pub level: Level,           // trace, debug, info, warn, error
    pub target: String,         // module path (e.g., "roko_agent::dispatcher")
    pub message: String,
    // Correlation fields -- join to traces, episodes, tasks
    pub trace_id: Option<TraceId>,
    pub span_id: Option<SpanId>,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    // Domain fields
    pub topic: Option<String>,        // if this log relates to a Bus publish
    pub signal_hash: Option<String>,  // if this log relates to a Store write
    pub gate: Option<String>,         // if this log relates to a gate verdict
    pub passed: Option<bool>,
    pub usd: Option<f64>,             // if this log relates to a cost event
}
```

### Bus Topics

| Topic | When | Volume |
|---|---|---|
| `telemetry.log.error` | Errors, panics, unrecoverable failures | Low |
| `telemetry.log.warn` | Degraded state, near-threshold conditions | Low-medium |
| `telemetry.log.info` | Operational events: task start/end, gate verdict, route decision | Medium |
| `telemetry.log.debug` | Detailed internal state (Bus publishes, Store puts) | High (off by default) |
| `telemetry.log.trace` | Per-field-level detail | Very high (off by default) |

### LogExportLens

```rust
/// Lens Cell: subscribes to telemetry.log.* and writes to the configured sink.
///
/// Modes:
///   json  -- one JSON object per line (default for containers)
///   human -- colored, human-readable (default for interactive)
///   debug -- extended fields visible (enabled by --debug flag)
pub struct LogExportLens {
    mode: LogMode,
    min_level: Level,
    sink: Box<dyn Write + Send>,  // stdout, file, or network
}
```

The key property: log output is a **projection**, not a side effect. The runtime publishes Pulses to the Bus regardless of whether any LogExportLens is subscribed. Logs exist because an observer watches the Bus -- identical to how a metric exists because a MetricLens aggregates Bus data.

---

## 4. Metrics as Numeric Lens Projections

Prometheus-compatible metrics are the output of **MetricLens Cells** -- Cells that subscribe to Bus topics, maintain rolling aggregates, and expose them on the `/metrics` endpoint.

### Architecture

```
Bus: telemetry.gate.verdict  ──▶ [GateMetricLens] ──▶ roko_gate_verdicts_total{rung, passed}
Bus: telemetry.cost.event    ──▶ [CostMetricLens] ──▶ roko_cost_usd_total{model, role}
Bus: telemetry.bus.publish   ──▶ [BusMetricLens]  ──▶ roko_bus_pulses_total{topic}
Timer: per request           ──▶ [HttpMetricLens] ──▶ roko_http_request_duration_seconds{path}
```

Each MetricLens maintains an in-memory counter/gauge/histogram. The `/metrics` endpoint reads from all registered MetricLens Cells and serializes Prometheus exposition format.

### Metric Catalog

#### Runtime metrics (generic process health)

| Metric | Type | Labels |
|---|---|---|
| `roko_http_requests_total` | counter | `method`, `path`, `status` |
| `roko_http_request_duration_seconds` | histogram | `method`, `path` |
| `roko_process_cpu_seconds_total` | counter | -- |
| `roko_process_memory_bytes` | gauge | `kind` (rss, heap) |
| `roko_tokio_tasks_active` | gauge | -- |

#### Cognitive metrics (Roko-specific)

| Metric | Type | Labels | What it tells operators |
|---|---|---|---|
| `roko_gate_verdicts_total` | counter | `rung`, `passed` | Gate throughput and pass rate |
| `roko_gate_pipeline_duration_seconds` | histogram | `plan_id` | End-to-end verification latency |
| `roko_cost_usd_total` | counter | `model`, `role`, `plan_id` | Cumulative spend by dimension |
| `roko_cost_budget_remaining_usd` | gauge | `scope` (session, plan, task) | Spend headroom |
| `roko_bus_pulses_total` | counter | `topic` | Bus throughput |
| `roko_store_signals_total` | gauge | `kind` | Store cardinality |
| `roko_c_factor` | gauge | `cohort` | Collective intelligence metric |
| `roko_cascade_route_total` | counter | `tier`, `outcome` | Model routing decisions |
| `roko_demurrage_balance_p95` | histogram | `kind` | Memory self-trimming health |
| `roko_safety_escalations_total` | counter | `reason` | Safety friction rate |

### The /metrics Endpoint

```
GET /metrics HTTP/1.1

# HELP roko_gate_verdicts_total Gate verdicts by rung and outcome
# TYPE roko_gate_verdicts_total counter
roko_gate_verdicts_total{rung="compile",passed="true"} 847
roko_gate_verdicts_total{rung="compile",passed="false"} 23
roko_gate_verdicts_total{rung="test",passed="true"} 712
roko_gate_verdicts_total{rung="test",passed="false"} 158

# HELP roko_cost_usd_total Cumulative LLM spend in USD
# TYPE roko_cost_usd_total counter
roko_cost_usd_total{model="claude-sonnet-4-6",role="implementer"} 14.37
roko_cost_usd_total{model="claude-haiku-4-5",role="reviewer"} 1.82
```

---

## 5. Traces as Lineage-Annotated Signals

OpenTelemetry traces map naturally to Roko's **lineage system**. A trace is a tree of Signals where each Signal's `parent_hashes` field points to its causal parent. Span metadata (start time, end time, attributes) is carried in the Signal's metadata.

### Mapping

| OTel Concept | Roko Concept |
|---|---|
| Trace | A Signal lineage tree rooted at a request entry point |
| Span | A Signal with `Kind::Span` and start/end timestamps |
| Span attributes | Signal metadata fields |
| trace_id | The root Signal's content hash |
| parent_span_id | `parent_hashes[0]` |
| Baggage | Metadata propagated through lineage |

### Span Signals per Operator Step

The universal loop emits one Span Signal per step:

| Step | Span name | Key attributes |
|---|---|---|
| Sense | `op.sense` | `source_topics`, `store_queries` |
| Assess | `op.assess` | `route_candidates`, `selected_tier` |
| Compose | `op.compose` | `budget_tokens`, `sections_count`, `retrieved_signals` |
| Act | `op.act` | `model`, `tokens_in`, `tokens_out`, `tool_calls` |
| Verify | `op.verify` | `rung`, `passed`, `duration_ms` |
| Persist | `op.persist` | `signal_hash`, `store_id` |
| Broadcast | `op.broadcast` | `topic`, `pulse_seq` |
| React | `op.react` | `policy_triggered`, `action` |

### TraceLens Cell

```rust
/// Lens Cell: collects Span Signals from Store lineage and exports via OTLP.
///
/// The TraceLens does NOT instrument the runtime. It reads Span Signals that the
/// runtime already produces (because Span is just another Signal Kind) and batches
/// them for OTLP export.
pub struct TraceLens {
    exporter: OtlpExporter,
    sample_rate: f64,          // baseline sample rate (0.0-1.0)
    force_on_error: bool,      // always export error spans (true by default)
}
```

### Sampling Strategy

| Condition | Sample rate | Rationale |
|---|---|---|
| Normal operation | 1% - 10% (configurable) | Bounded telemetry volume |
| Error spans | 100% | Always export failures for diagnosis |
| Cost > threshold | 100% | Expensive operations need visibility |
| Safety escalation | 100% | Security events are always captured |

---

## 6. StateHub as Named Lens Compositions

StateHub projections are **compositions of multiple Lens Cells** into named views that dashboards, TUI, CLI, and SSE consumers subscribe to. A projection is itself a Graph (of Lens Cells).

### Architecture

```
[GateMetricLens] ──┐
[CostMetricLens] ──┼──▶ [ProjectionComposeLens("plan_health")] ──▶ SSE / TUI / WebSocket
[TaskStatusLens] ──┘
```

### Named Projections

| Projection name | Composed from | Consumer |
|---|---|---|
| `plan_health` | gate verdicts + task status + cost | TUI Plan tab, SSE |
| `agent_status` | agent lifecycle events + model calls | TUI Agent tab |
| `cost_meter` | cost events per model/role/plan | TUI Efficiency page, REST |
| `gate_pipeline` | current rung + pass/fail counts | TUI Dashboard tab |
| `bus_stats` | pulse throughput per topic | Metrics, debug view |
| `safety_events` | auth denials + escalations | Alert pipeline, audit |
| `learning_state` | router weights + experiment outcomes | Learn CLI, dashboard |

### Live Subscription

Consumers subscribe to named projections rather than raw Bus topics:

```rust
/// Subscribe to a named projection via SSE or WebSocket.
///
/// The projection Graph runs independently. When it emits a new observation
/// Signal, all subscribers receive it. This decouples the telemetry compute
/// from the number of consumers.
GET /api/v1/projections/plan_health/subscribe
Accept: text/event-stream
```

---

## 7. Cost Visibility as a Lens Pipeline

Cost is a first-class telemetry concern. The cost pipeline reads efficiency events from `.roko/learn/efficiency.jsonl` and Bus cost Pulses, then projects them into multiple views.

### Pipeline

```
Store: efficiency.jsonl  ──┐
                            ├──▶ [CostCollectorLens] ──▶ [CostAggregatorLens] ──▶ [CostExportLens]
Bus: telemetry.cost.event ─┘
```

### Cost Pulse Schema

```rust
/// Published on Bus topic "telemetry.cost.event" after every LLM call.
pub struct CostPulse {
    pub model: String,
    pub role: String,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub usd: f64,
    pub duration_ms: u64,
    pub cache_hit_tokens: u32,
}
```

### Aggregation Dimensions

The CostAggregatorLens maintains rolling windows:

| Dimension | Granularity | Use |
|---|---|---|
| Per session | lifetime of CLI invocation | "How much did this run cost?" |
| Per task | single task within a plan | "Which tasks are expensive?" |
| Per plan | full plan execution | "Was this plan worth it?" |
| Per model | all time | "Which model is cheapest for this task type?" |
| Per role | all time | "Does the reviewer role need an expensive model?" |
| Budget scope | configurable window | "Am I approaching my limit?" |

### Export Surfaces

- `roko_cost_usd_total` -- Prometheus counter (per model, role)
- `roko_cost_budget_remaining_usd` -- Prometheus gauge (per scope)
- `cost_meter` projection -- live view for TUI/SSE
- `roko status --cost` -- CLI summary
- `GET /api/v1/cost/summary` -- REST endpoint

---

## 8. Replay as Store Traversal

Replay is observability applied backwards in time. It reconstructs what an agent saw and why it acted by traversing Signal lineage in Store.

### Replay Algorithm

```
1. Start from a target Signal (e.g., a gate verdict or episode).
2. Walk parent_hashes to reconstruct the causal chain.
3. For each Signal in the chain, retrieve its Bus context:
   - If the Bus ring still holds the Pulse: read directly.
   - If the ring has wrapped: reconstruct from graduated Signals in Store.
4. Render the sequence as a timeline with decision points annotated.
```

### ReplayLens Cell

```rust
/// Lens Cell: traverses Store lineage to reconstruct an episode's decision chain.
///
/// Input: a Signal hash (the replay target).
/// Output: an ordered sequence of Signals representing the causal history.
pub struct ReplayLens {
    store: Arc<dyn Store>,
    max_depth: usize,       // how far back to traverse (default: 100)
}
```

### Override-Based Replay

Operators can replay with alternate configuration to test sensitivity:

```bash
# Replay episode abc123 with a different routing config
roko replay abc123 --override routing.cost_weight=0.8

# Replay with a different gate threshold
roko replay abc123 --override gates.test.threshold=0.9
```

This re-runs the decision chain through the same Pipeline of Cells but with modified config Signals, showing how the outcome would change. The replay does not mutate Store -- it produces new observation Signals annotated as counterfactual.

---

## 9. Deployment Shape Consistency

Every deployment shape exposes the same Lens Pipeline. The difference is which ExportLens sinks are configured:

| Shape | Log sink | Metric sink | Trace sink | Projection sink |
|---|---|---|---|---|
| Laptop | stderr (human mode) | optional `/metrics` | none (or local Jaeger) | TUI direct |
| Single-server | stderr (JSON) | `/metrics` | OTLP to local collector | SSE + REST |
| Container | stdout (JSON) | `/metrics` | OTLP to external collector | REST only |
| Clustered | stdout (JSON) | `/metrics` (per-node) | OTLP to shared collector | WebSocket + SSE |
| Edge | compact JSON | minimal counters | sampled OTLP | deferred batch |

The Lens Cells themselves are identical. Only the terminal ExportLens differs. An operator switching from laptop to container changes the export configuration, not the observability logic.

---

## 10. Alert Pipeline

Alerts are **React Cells** that subscribe to MetricLens outputs and fire when thresholds are crossed:

```rust
/// React Cell: fires when a metric crosses a threshold.
///
/// Subscribes to the MetricLens output. When the value crosses the configured
/// threshold for the configured duration, publishes an alert Pulse.
pub struct AlertReactCell {
    metric: String,
    condition: AlertCondition,  // gt, lt, rate_of_change
    threshold: f64,
    for_duration: Duration,
    severity: AlertSeverity,
}
```

### Default Alert Rules

| Alert | Condition | Severity |
|---|---|---|
| Gate stall | `roko_gate_verdicts_total` rate < 1/min for 10min | warning |
| Cost spike | `roko_cost_usd_total` rate > 3x rolling average | warning |
| Budget exhaustion | `roko_cost_budget_remaining_usd` < 10% | critical |
| Safety escalation surge | `roko_safety_escalations_total` rate > 5/hour | critical |
| C-factor decline | `roko_c_factor` decreasing for 1 hour | warning |
| Bus saturation | `roko_bus_pulses_total` exceeds ring capacity threshold | warning |

---

## What This Enables

1. **Telemetry as composition**: Adding a new metric, log field, or dashboard panel is adding a Cell to a Graph -- not modifying the runtime.
2. **Zero-overhead when unobserved**: Lens Cells are read-only. If no MetricLens subscribes to gate events, no metric computation occurs.
3. **Uniform replay**: Because all telemetry flows through the same Bus/Store system, replay reconstructs the complete picture -- not just logs, not just traces, but the full causal chain.
4. **Cost as first-class telemetry**: Cost visibility is not an afterthought; it is a Lens Pipeline with the same status as metrics or traces.
5. **Shape-portable observability**: Same Lens Cells, different export sinks. Operators learn one model regardless of deployment target.

## Feedback Loops

- **L1 (per-turn)**: MetricLens outputs feed into adaptive gate thresholds. If `roko_gate_verdicts_total{passed=false}` spikes, the threshold EMA adjusts.
- **L2 (per-session)**: CostMetricLens outputs feed into CascadeRouter. High-cost models that do not improve gate pass rate get deprioritized.
- **L3 (cross-session)**: ReplayLens enables postmortem analysis. Operators identify failure patterns, which become playbook rules injected into future prompts.
- **Alert loop**: AlertReactCell fires a Pulse. The Pulse is itself observable (logged, metriced). This prevents silent alert failures.

## Open Questions

1. **Cardinality explosion**: Metrics with high-cardinality labels (per-task, per-signal-hash) can overwhelm Prometheus. Should MetricLens Cells enforce cardinality limits, or is that an ExportLens concern?
2. **Lens ordering in Pipeline**: When multiple Lens Cells compose into a projection, does ordering matter? Currently assumed commutative, but some aggregations (rate-then-threshold vs threshold-then-rate) are not.
3. **Hot-path overhead**: Even read-only Lens Cells have CPU cost. Should the runtime elide Lens Cell execution entirely when no consumer is subscribed? (Zero-subscriber optimization.)
4. **Cross-node trace assembly**: In clustered deployments, Span Signals live in different Stores. How does ReplayLens traverse cross-node lineage? Requires a federated Store query or a shared trace Store.
5. **Retention policy per Lens**: Should each MetricLens declare its own retention (e.g., "keep 7 days of histogram data"), or is retention always a global Store policy?

## Implementation Tasks

| Task | Crate | Effort | Priority |
|---|---|---|---|
| Define `LogPulse` struct and Bus topic convention | `roko-core` | S | High |
| Implement `LogExportLens` (JSON + human modes) | `roko-cli` | M | High |
| Implement `MetricLens` trait and 5 runtime metrics | `roko-serve` | M | High |
| Add `/metrics` endpoint to `roko-serve` | `roko-serve` | S | High |
| Implement `CostCollectorLens` reading efficiency.jsonl | `roko-learn` | M | Medium |
| Implement `TraceLens` with OTLP batch export | `roko-serve` | L | Medium |
| Define 6 named projections as Graph configs | `roko-serve` | M | Medium |
| Implement `ReplayLens` for lineage traversal | `roko-cli` | M | Medium |
| Implement `AlertReactCell` with 6 default rules | `roko-serve` | M | Low |
| Add cost summary to `roko status --cost` | `roko-cli` | S | High |
| Wire structured log Pulses into existing `tracing` integration | `roko-cli` | M | High |
