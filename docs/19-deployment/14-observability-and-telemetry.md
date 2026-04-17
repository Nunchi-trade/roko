# Observability and Telemetry

> Deployment-level observability is part of the product, not an afterthought. Roko runs across
> laptop-local, single-server, container, clustered, and edge shapes, but every shape still has
> to expose the same operator story: structured logs, scrapeable metrics, distributed traces,
> typed event surfaces, replayable episodes, and attributed cost. The distinctive claim is not
> just that the runtime emits telemetry, but that the telemetry is aligned with Roko's two
> mediums (`Engram` and `Pulse`) moving through two fabrics (`Substrate` and `Bus`). For the
> canonical vocabulary, see [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).
> See also `../../tmp/refinements/33-observability-telemetry.md`.

> **Implementation**: Baseline exists; advanced exporters are target-state

> **Implementation status**: The current observability baseline is narrower than this chapter originally implied. Shipping today: JSONL episode logs, efficiency events, the existing `StateHub`/dashboard path, and tracing-based structured logs. Not shipped as deployment defaults: a stable Prometheus `/metrics` endpoint, shipped alert rules, or an OTLP exporter. Treat Prometheus and OpenTelemetry sections below as **target-state** operator surfaces unless a concrete subdocument says otherwise.

---

## Operator Framing

Deployment observability answers one operator question: "what did the system know, what did it
do, what did it cost, and how do I replay that decision?" In Roko, that question spans both
mediums and both fabrics:

- `Engram` persistence on `Substrate` gives durable, queryable audit and replay inputs.
- `Pulse` movement on the `Bus` gives live progress, gate outcomes, route choices, and stream
  state while a run is still unfolding.
- `StateHub` projections turn raw Bus activity plus durable Substrate state into typed views for
  dashboards, CLIs, TUIs, and remote consumers.

That framing matters operationally. Generic observability tells an operator whether a process is
alive. Roko-specific observability tells an operator whether the system is getting smarter,
forgetting appropriately, spending within budget, and drifting away from calibrated behavior.

---

## Current Baseline

The current codebase already has a usable observability baseline:

- JSONL episode logs for durable run history
- efficiency events in `.roko/learn/efficiency.jsonl`
- the existing `StateHub` plus `DashboardSnapshot` path used by TUI, SSE, WebSocket, and REST status views
- tracing-based structured logs

The rest of this chapter describes how that baseline could grow into a fuller deployment observability surface.

---

## Telemetry Surfaces by Deployment Shape

Target-state, every deployment shape should expose the same telemetry contract, even though the default sinks may differ:

| Shape | Default operator posture | Target telemetry surfaces |
|---|---|---|
| laptop-local | human-driven inspection and postmortem | human or JSON logs, local metrics endpoint, optional OTLP traces, local replay |
| single-server | one long-lived service with local state | JSON logs, `/metrics`, OTLP exporter, StateHub projections, alert rules |
| container | image-first, orchestrator-managed | stdout/stderr logs, `/metrics`, readiness/liveness probes, external trace sink |
| clustered | multi-node service behind ingress | centralized logs, shared metrics scraping, distributed traces, shared StateHub and replay retention |
| edge | constrained footprint, selective export | compact logs, minimal metrics, sampled traces, deferred replay via graduated Engrams |

The deployment rule is consistency: operators should not learn a different observability model
for each shape. The same labels, topic names, projections, and replay semantics should work
everywhere, even if the retention or exporter defaults are smaller at the edge.

---

## Logs

Structured logs are the lowest-friction deployment surface because every operator already has a
path to `stderr`, `docker logs`, `journalctl`, or a log shipper. Roko should default to one JSON
object per line and allow a human-readable mode for interactive sessions.

### Required Log Fields

Every structured log line should carry enough context to join it to traces, Pulses, or replay:

- `ts`, `level`, and `target`
- `trace_id` and span identifiers when inside an operator boundary
- stable runtime identifiers such as `plan_id`, `task_id`, `agent_id`, `principal_id`, and
  tenant or cohort identifiers where cardinality is safe
- `topic` for Bus publishes and `engram_hash` plus `engram_kind` for Substrate writes
- `gate` and `passed` for verification outcomes
- `usd` or token counts for cost-bearing actions

Large bodies should not be dumped raw into logs. Deployment-friendly logging replaces large
payloads with a compact `{hash, len}` summary so aggregation and grep stay usable.

### Logging Modes

| Mode | Deployment use |
|---|---|
| JSON default | containers, systemd, Fly.io, clustered ingestion pipelines |
| human format | laptop-local sessions and incident triage |
| debug | postmortems, local reproduction, high-detail operator forensics |

`--debug` should raise visibility by logging Bus publishes, Substrate puts, and discarded span
events, but production profiles should still default to bounded-volume structured output.

---

## Metrics

Prometheus-compatible metrics are a target-state deployment surface for trendlines, SLOs, dashboards, and
alerts. Roko needs both generic process metrics and Roko-specific metrics, but this chapter should not be read as claiming a shipped `/metrics` endpoint today.

### Generic Runtime Metrics

If and when a stable Prometheus surface is exposed, it should include the table-stakes runtime signals:

| Metric | Type | Purpose |
|---|---|---|
| `roko.http.requests_total` | counter | control-plane request volume |
| `roko.http.request_duration_seconds` | histogram | request latency |
| `roko.process.cpu_seconds_total` | counter | CPU consumption |
| `roko.process.memory_bytes` | gauge | RSS and memory growth |
| `roko.tokio.tasks_active` | gauge | async runtime pressure |
| `roko.tokio.blocking_tasks` | gauge | blocking-pool contention |

### Roko-Specific Metrics

The deployment chapter needs the metrics that would justify running Roko instead of a generic agent
wrapper. The most important families below are target-state examples, not a claim that every metric already exists today:

| Metric | Type | Why operators care |
|---|---|---|
| `roko.c_factor` | gauge | whether a cohort is improving collective intelligence |
| `roko.turn_taking_entropy` | gauge | whether collaboration is dominated by too few participants |
| `roko.demurrage.balance_p95` | histogram | whether durable memory is bloating instead of self-trimming |
| `roko.substrate.query_similar_latency_ms` | histogram | HDC-backed similar-query performance |
| `roko.heuristic.calibration_brier` | histogram | whether heuristics are staying calibrated |
| `roko.prediction.ema_error` | gauge | per-operator drift in predictive accuracy |
| `roko.gate.verdicts_total` | counter | gate outcomes over time |
| `roko.gate.pipeline_duration_ms` | histogram | end-to-end verification latency |
| `roko.bus.pulses_total` | counter | Pulse throughput by topic |
| `roko.bus.ring_occupancy` | gauge | whether replayable transport retention is near saturation |
| `roko.cost.usd_total` | counter | cumulative spend by model and role |
| `roko.cost.budget_remaining_usd` | gauge | how much spend headroom remains |
| `roko.safety.escalations_total` | counter | escalation rate and operational friction |

The deployment implication is straightforward: dashboards should prioritize these metrics, not
bury them underneath CPU and memory charts. A healthy Roko node is one whose cognitive and
economic signals are legible, not merely one whose process is up.

---

## Traces

OpenTelemetry traces are the target-state export surface for operator latency and call structure across the seven-step loop.
Every operator boundary should emit spans, with the trace id flowing through Bus and Substrate
interactions:

- `op.sense` with child spans such as `substrate.query` and `bus.receive`
- `op.assess` and route-selection children
- `op.compose` including similar-query retrieval on `Substrate`
- `op.act` including model calls, tool calls, and streamed Bus publishes
- `op.verify` with gate-pipeline children
- `op.persist` for durable writes
- `op.broadcast` for Bus publication
- `op.react` for policy reactions

Span attributes should include `operator_id`, `principal_id`, `content_hash`, `pulse_seq`,
topic, and deployment-shape identifiers. OTLP is the intended export contract if tracing exporters are added later; Jaeger, Zipkin,
Tempo, and other OTLP-compatible collectors can sit downstream.

For clustered deployments, traces are the primary tool for answering cross-node questions such
as which node handled a route decision, where latency accumulated, and which tool invocation
caused a gate stall.

---

## Events and StateHub

Logs, metrics, and traces are generic observability surfaces. The current Roko-specific baseline is the existing `StateHub` and `DashboardSnapshot` path. A richer named-projection catalog remains the target evolution because many operators want live, queryable telemetry rather than scrape-based aggregation.

Target-state deployment projections include:

| Projection | Operator use |
|---|---|
| `cohort_health` | live `c_factor`, roster, and delivery health |
| `gate_pipeline` | current rung status plus pass/fail counts |
| `bus_stats` | pulses per second by topic and subscriber pressure |
| `substrate_stats` | Engram counts, warm/cold balance, and demurrage histograms |
| `cost_meter` | spend by session, role, and model |
| `safety_events` | recent authz denials, confirmations, and escalations |
| `replication_ledger` | claim status tracking for research-to-runtime workflows |
| `calibration_curves` | heuristic and operator drift trends |

Deployment surfaces such as the web UI, TUI, CLI watch mode, and remote consumers should read
from these projections instead of hand-assembling their own telemetry joins. That keeps the
observability surface typed, filterable, and consistent with the Bus/Substrate model.

---

## Replay and Time-Travel

Replay is observability, not a separate debugging toy. The deployment promise is that an
operator can reconstruct what an agent saw and why it acted.

Replay consumes an episode's Engrams and the relevant Pulse history:

- if the Bus ring still holds the Pulses, replay reads them directly
- if the ring has wrapped, replay reconstructs the episode from graduated Engrams and durable
  projections
- override-based replay allows operators to test sensitivity to configuration changes such as
  demurrage rates, gate thresholds, or routing policies without mutating live state

That makes postmortems materially better. The question is no longer "do we have enough logs to
guess what happened?" but "can we replay the exact sequence with alternate assumptions and see
how the decision boundary moves?"

---

## Cost Visibility

Cost is a first-class deployment concern because Roko claims model-routing and budget awareness
as part of its operational value. Raw token counters are not enough; operators need attributed
spend:

- per session, visible during active work
- per task after a plan run
- per role across historical windows
- per model so routing quality can be compared against spend
- per budget scope so burn rate and remaining headroom are visible

The deployment surface should therefore expose both metrics and higher-level projections. The
`cost_meter` projection is the live view; `roko.cost.tokens_total`, `roko.cost.usd_total`, and
`roko.cost.budget_remaining_usd` provide durable timeseries backing for alerts and dashboards.

---

## Alerting and Readiness

Roko should ship default alert rules for the failures that matter operationally:

- falling `roko.c_factor`
- stalled gate verdict throughput
- rising safety escalations
- substrate bloat and demurrage imbalance
- calibration drift spikes
- Bus ring saturation
- abnormal cost spikes
- plugin or sandbox violation surges

Readiness must include the observability plane itself. If logs are backing up, trace exporters
are dropping spans beyond threshold, or projection latency has crossed a deployment limit,
`/readyz` should fail before operators silently lose visibility during a rollout.

This is stricter than generic web-service health, but it is justified: a deployment that is
processing work while black-holing observability is operating below Roko's claimed standard.

---

## Retention and Sampling

Not every telemetry surface retains forever, and the retention model must follow the medium:

| Surface | Default retention posture |
|---|---|
| metrics | retained by the downstream monitoring stack |
| logs | retained by the configured log pipeline |
| traces | sampled, with error paths forced to full capture |
| Bus Pulses | ring-buffer retention with bounded capacity |
| Engrams | durable retention governed by demurrage and storage policy |
| Custody records | long-lived retention for compliance and audit |

Operational defaults should be conservative:

- traces sampled at a low baseline rate, with 100% capture on errors
- Bus ring capacity sized to make recent replay practical without unbounded memory growth
- edge deployments allowed to graduate important Pulses into Engrams earlier when local rings are
  small
- cost, safety, and custody data retained longer than ephemeral transport noise

Sampling decisions are part of deployment configuration, not hidden implementation detail.

---

## Integration with Existing Stacks

Roko should plug into established operator stacks rather than requiring a bespoke monitoring
island:

| Existing stack | Roko surface | Deployment integration |
|---|---|---|
| Prometheus + Grafana | planned `/metrics` endpoint and alert rules | scrape, alert, and dashboard timeseries once that surface exists |
| Loki, Elastic, Datadog Logs | structured stdout/stderr JSON | standard container or system log shipping |
| Jaeger, Zipkin, Tempo, Honeycomb | planned OTLP traces | point the runtime at the collector endpoint once exporter support exists |
| Sentry or Bugsnag | crash output plus plugin hook | exception and crash reporting |
| Slack or PagerDuty | Alertmanager receivers | route alert notifications to operators |
| custom dashboards and clients | `StateHub` projections plus realtime surface | subscribe to typed telemetry directly |

The deployment recommendation is pragmatic: use generic tooling for generic surfaces, and use
StateHub when Roko-specific semantics matter.

---

## Deployment Guidance

An operator-ready deployment should satisfy the following:

1. Logs are structured by default and safe to aggregate.
2. add `/metrics` anywhere a long-lived service runs once the Prometheus surface is implemented.
3. add OTLP export for remote or clustered shapes once exporter support exists.
4. expose the current `StateHub` telemetry baseline to first-party surfaces, then split it into finer projections only when needed.
5. Replay retention is sized to support postmortem windows.
6. Cost, safety, gate, Bus, and calibration dashboards exist before production cutover.
7. Alerting is wired before unattended operation begins.

The outcome is not merely "a monitored process." The outcome is a deployment where every gate,
every heuristic, every cost-bearing action, and every important decision is observable through a
surface appropriate to the operator using it.

---

## Cross-References

- Glossary and naming contract:
  [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md)
- Production hardening:
  [12-production-hardening.md](12-production-hardening.md)
- Remote service and realtime exposure:
  [11-remote-orchestrator.md](11-remote-orchestrator.md)
- Canonical refinement source:
  `../../tmp/refinements/33-observability-telemetry.md`
