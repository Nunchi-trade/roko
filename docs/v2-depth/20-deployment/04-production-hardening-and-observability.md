# Production Hardening and Observability

> Production hardening is a composition of Verify + React + Lens Cells that make the
> runtime resilient, observable, and upgradeable. Adaptive timeouts, exponential backoff,
> concurrency control, graceful shutdown, zero-downtime upgrades, and full observability
> are not separate systems -- they are Cell compositions operating on the same Bus and
> Store fabric as everything else.

---

## Hardening as Cell Composition

Each hardening concern maps to a specific Cell protocol pattern:

| Concern | Cell Pattern | Protocol |
|---|---|---|
| Adaptive timeouts | Loop (predict -> observe -> correct) | Verify + React |
| Exponential backoff | React (state machine) | React |
| Concurrency control | Verify (pre-condition check) | Verify |
| Graceful shutdown | Graph termination protocol | React + Store |
| Zero-downtime upgrade | Pipeline (drain -> checkpoint -> replace) | Verify + Connect |
| Health checks | Lens (read-only observation) | Observe |
| Structured logs | Bus Pulses (ephemeral observation) | Observe |
| Prometheus metrics | Lens Cell output | Observe |
| OpenTelemetry traces | Lineage-annotated Signals | Observe + Store |

These compose naturally. A single request might flow through: Verify (concurrency permit)
-> React (timeout from adaptive estimate) -> Cell execution -> React (backoff on failure)
-> Observe (trace span close) -> Verify (health check contribution).

---

## Adaptive Timeouts as Loop

Static timeouts are brittle: too short causes false failures on slow providers, too long
wastes time on genuine failures. Adaptive timeouts use the Loop pattern to predict
appropriate timeouts from observed latency.

```rust
/// Adaptive timeout: Loop pattern (predict -> observe -> correct).
/// Maintains per-provider latency history, predicts timeout as p95 * multiplier.
pub struct AdaptiveTimeout {
    /// Sliding window of observed latencies per provider.
    observations: HashMap<String, VecDeque<Duration>>,
    /// Maximum observations to retain per provider.
    window_size: usize,
    /// Multiplier applied to p95 (default: 2.0).
    multiplier: f64,
    /// Hard bounds: [min_timeout, max_timeout].
    min_timeout: Duration,
    max_timeout: Duration,
    /// EMA error for calibration tracking.
    ema_error: HashMap<String, f64>,
}

impl AdaptiveTimeout {
    /// Predict: estimate timeout for next request to this provider.
    pub fn predict(&self, provider: &str) -> Duration {
        let obs = self.observations.get(provider);
        match obs {
            None => Duration::from_secs(30), // No data: conservative default
            Some(samples) if samples.is_empty() => Duration::from_secs(30),
            Some(samples) => {
                let mut sorted: Vec<_> = samples.iter().copied().collect();
                sorted.sort();
                let p95_idx = (sorted.len() as f64 * 0.95) as usize;
                let p95 = sorted[p95_idx.min(sorted.len() - 1)];
                let predicted = Duration::from_secs_f64(
                    (p95.as_secs_f64() * self.multiplier)
                        .clamp(self.min_timeout.as_secs_f64(), self.max_timeout.as_secs_f64())
                );
                predicted
            }
        }
    }

    /// Observe: record actual latency for this request.
    pub fn observe(&mut self, provider: &str, actual: Duration) {
        let window = self.observations
            .entry(provider.to_string())
            .or_insert_with(|| VecDeque::with_capacity(self.window_size));
        if window.len() >= self.window_size {
            window.pop_front();
        }
        window.push_back(actual);
    }

    /// Correct: update EMA error for calibration monitoring.
    pub fn correct(&mut self, provider: &str, predicted: Duration, actual: Duration) {
        let error = (predicted.as_secs_f64() - actual.as_secs_f64()).abs()
            / actual.as_secs_f64().max(0.001);
        let ema = self.ema_error.entry(provider.to_string()).or_insert(0.0);
        *ema = 0.1 * error + 0.9 * *ema; // EMA with alpha=0.1
    }
}
```

The timeout Loop publishes its predictions as Pulses on the Bus. The calibration Lens Cell
observes prediction error and surfaces it in the dashboard.

Configuration:

```toml
[hardening.timeouts]
window_size = 100       # Observations per provider
multiplier = 2.0        # p95 * 2.0
min_timeout_sec = 5     # Never timeout faster than 5s
max_timeout_sec = 300   # Never wait longer than 5 minutes
```

---

## Exponential Backoff as React Cell State Machine

When a request fails with a retryable error, the backoff React Cell determines the retry
delay. It is a state machine with transitions driven by outcomes.

```rust
/// Backoff state machine: React Cell that computes retry delays.
///
/// States: Ready -> Backing Off -> Retry -> (success: Ready) | (failure: Backing Off)
///
/// Algorithm: full jitter over exponential base.
///   sleep = random(0, min(cap, base * 2^attempt))
pub struct BackoffReact {
    base: Duration,        // Starting delay (default: 1s)
    cap: Duration,         // Maximum delay (default: 60s)
    max_attempts: u32,     // Give up after N attempts (default: 5)
    current_attempt: u32,
    jitter_rng: StdRng,
}

impl BackoffReact {
    /// Compute next retry delay with full jitter.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.current_attempt >= self.max_attempts {
            return None; // Exhausted: give up
        }

        let exp_delay = self.base.as_secs_f64() * 2.0_f64.powi(self.current_attempt as i32);
        let capped = exp_delay.min(self.cap.as_secs_f64());
        let jittered = self.jitter_rng.gen_range(0.0..capped);
        self.current_attempt += 1;

        Some(Duration::from_secs_f64(jittered))
    }

    /// Reset on success.
    pub fn reset(&mut self) {
        self.current_attempt = 0;
    }
}

/// Retry decision: what to do after a failure.
#[derive(Debug, Clone)]
pub enum RetryAction {
    /// Retry with computed delay.
    Retry { delay: Duration, attempt: u32 },
    /// Fail over to a different provider.
    Failover { provider: String },
    /// Give up immediately (non-retryable error).
    Abort { reason: String },
}

/// Classify errors into retry actions.
pub fn classify_error(err: &ProviderError) -> RetryAction {
    match err {
        // Retryable: server errors, timeouts, rate limits
        ProviderError::Timeout => RetryAction::Retry { /* computed */ },
        ProviderError::ServerError(5xx) => RetryAction::Retry { /* computed */ },
        ProviderError::RateLimit { retry_after } => RetryAction::Retry { delay: *retry_after, attempt: 0 },
        // Failover: provider down
        ProviderError::Unavailable => RetryAction::Failover { /* next provider */ },
        // Non-retryable: client errors
        ProviderError::AuthFailed => RetryAction::Abort { reason: "Invalid API key".into() },
        ProviderError::InvalidRequest(_) => RetryAction::Abort { reason: "Malformed request".into() },
    }
}
```

Configuration:

```toml
[hardening.backoff]
base_sec = 1.0
cap_sec = 60.0
max_attempts = 5
# Full jitter: delay = random(0, min(cap, base * 2^attempt))
```

---

## Concurrency Control via Verify Pre-Conditions

Provider rate limits and system resource constraints are enforced through Verify Cells that
check pre-conditions before allowing execution to proceed.

```rust
/// Concurrency semaphore: Verify Cell that gates execution.
/// Must acquire a permit before proceeding. Blocks if pool is exhausted.
pub struct ConcurrencyVerify {
    /// Per-provider semaphores (respects provider rate limits).
    provider_semaphores: HashMap<String, Arc<Semaphore>>,
    /// Global agent limit.
    global_semaphore: Arc<Semaphore>,
}

impl ConcurrencyVerify {
    /// Verify pre-condition: is there capacity for this request?
    pub async fn verify_pre(&self, provider: &str) -> Result<OwnedSemaphorePermit> {
        // Check global limit first
        let global = self.global_semaphore.clone().acquire_owned().await?;

        // Then provider-specific limit
        let provider_sem = self.provider_semaphores
            .get(provider)
            .ok_or_else(|| anyhow!("Unknown provider: {provider}"))?;
        let provider_permit = provider_sem.clone().acquire_owned().await?;

        Ok(CombinedPermit { global, provider: provider_permit })
    }
}
```

Default concurrency limits per deployment shape:

| Shape | Global Agents | Per-Provider | Rationale |
|---|---|---|---|
| Laptop | 4 | 2 | Conservative, interactive use |
| Server | 8 | 4 | Moderate, shared machine |
| Container | 8 | 4 | Tuned per container instance |
| Clustered | 16/node | 8/node | Horizontal scale |
| Edge | 1 | 1 | Minimal, request-scoped |

Configuration:

```toml
[hardening.concurrency]
max_total_agents = 8

[hardening.concurrency.providers]
anthropic = 4
openai = 4
openrouter = 2
```

---

## Graceful Shutdown: Graph Termination Protocol

Shutdown is not "kill the process." It is a structured protocol that drains work, persists
state, and closes connections cleanly. This enables zero-downtime upgrades and crash
recovery.

```rust
/// Graph termination protocol: 4 phases.
///
/// Phase 1: STOP ACCEPTING
///   - Set accepting = false
///   - Health check /readyz returns 503
///   - No new requests enter the Graph
///
/// Phase 2: DRAIN
///   - Wait for in-flight requests to complete
///   - Bounded by drain_timeout (default: 30s)
///   - Bus subscribers get disconnect notification
///
/// Phase 3: SERIALIZE STATE
///   - Flush executor snapshot to Store
///   - Persist subscription states
///   - Write efficiency events
///   - Close Bus (remaining Pulses lost -- they're ephemeral)
///
/// Phase 4: CLOSE CONNECTIONS AND EXIT
///   - Close IPC socket (remove socket file)
///   - Close HTTP listener
///   - Close WebSocket connections (send close frame)
///   - Exit with code 0 (clean exit = don't restart)
pub struct ShutdownProtocol {
    drain_timeout: Duration,
    state: ShutdownState,
}

#[derive(Debug, Clone, PartialEq)]
enum ShutdownState {
    Running,
    StopAccepting,
    Draining { deadline: Instant, remaining: usize },
    Serializing,
    Exiting,
}

impl ShutdownProtocol {
    pub async fn execute(&mut self, runtime: &mut RuntimeState) {
        // Phase 1
        self.state = ShutdownState::StopAccepting;
        runtime.accepting = false;

        // Phase 2
        let deadline = Instant::now() + self.drain_timeout;
        self.state = ShutdownState::Draining {
            deadline,
            remaining: runtime.active_tasks(),
        };

        tokio::select! {
            _ = runtime.drain_all_tasks() => {
                // All tasks completed cleanly
            }
            _ = tokio::time::sleep_until(deadline) => {
                // Timeout: force-kill remaining
                runtime.force_shutdown_agents().await;
            }
        }

        // Phase 3
        self.state = ShutdownState::Serializing;
        runtime.save_executor_snapshot().await;
        runtime.flush_efficiency_events().await;
        runtime.save_subscription_state().await;

        // Phase 4
        self.state = ShutdownState::Exiting;
        runtime.close_ipc_socket().await;
        runtime.close_http_listener().await;
        runtime.close_websockets().await;
    }
}
```

For real-time subscribers (WebSocket, SSE), the protocol ensures:
- `/readyz` fails BEFORE `/healthz` fails, so orchestrators stop sending new connections
- Existing connections receive a graceful close frame with retry hint
- Cursor retention outlives short restarts so clients can resume

---

## Zero-Downtime Upgrades via Rolling Restart

For server and container shapes, upgrades proceed without losing in-flight work:

```
Old Instance:                    New Instance:
  Running                          Starting
    |                                |
  /readyz = false                    |
    |                                |
  Draining (30s)                   /readyz = true (new traffic here)
    |                                |
  Serialized state                   |
    |                                |
  Exit(0)                          Load state, resume
```

The key properties:
1. State is serialized to Store before exit (executor snapshot)
2. New instance loads from Store and resumes from last checkpoint
3. Overlap period: both instances alive, but only new accepts traffic
4. No lost work: in-flight tasks complete on old instance before it exits

For clustered deployments: rolling replacement across N nodes, one at a time, behind a load
balancer that respects readiness probes.

---

## Observability: Three Lens Families

Observability in Roko uses the Lens Cell pattern: read-only observation of the Bus and
Store without mutation. Three families cover the standard operator surfaces.

### 1. Structured Logs as Bus Pulses

Every significant operation publishes a structured log Pulse on the Bus. The default Lens
Cell formats these to stderr as JSON or human-readable text.

```rust
/// Structured log entry: published as a Pulse on topic "log.*"
#[derive(Debug, Serialize)]
pub struct LogPulse {
    pub ts: DateTime<Utc>,
    pub level: Level,
    pub target: String,
    pub message: String,
    /// Correlation fields
    pub trace_id: Option<String>,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    /// Domain-specific fields
    pub topic: Option<String>,      // Bus topic if this is about a Bus operation
    pub signal_hash: Option<String>, // Store hash if about a Store operation
    pub gate: Option<String>,       // Gate name if about verification
    pub usd: Option<f64>,           // Cost if about an LLM call
}
```

Output modes per shape:

| Shape | Default Format | Sink |
|---|---|---|
| Laptop | Human-readable (colored) | stderr |
| Server | JSON (one object per line) | journald or file |
| Container | JSON | stdout (for log aggregators) |
| Clustered | JSON with node ID | centralized log system |
| Edge | Compact binary | local buffer, deferred export |

Configuration:

```toml
[observe.logs]
format = "json"          # json | human | compact
level = "info"           # trace | debug | info | warn | error
# Large payloads are replaced with {hash, len} summaries
max_body_log_bytes = 256
```

### 2. Prometheus Metrics as Lens Cell Output

Metrics Lens Cells maintain counters, gauges, and histograms. They observe Bus traffic and
Store state, exposing the results at `/metrics` in Prometheus exposition format.

```rust
/// Metrics Lens: observes Bus/Store and exposes Prometheus metrics.
pub struct MetricsLens {
    /// Process metrics
    http_requests_total: CounterVec,
    http_request_duration: HistogramVec,
    /// Cognitive metrics
    c_factor: Gauge,
    gate_verdicts_total: CounterVec,
    gate_pipeline_duration: Histogram,
    bus_pulses_total: CounterVec,
    bus_ring_occupancy: Gauge,
    /// Economic metrics
    cost_usd_total: CounterVec,
    cost_budget_remaining: GaugeVec,
    /// Storage metrics
    store_query_latency: HistogramVec,
    store_signal_count: Gauge,
    demurrage_balance_p95: Histogram,
    /// Safety metrics
    safety_escalations_total: Counter,
    /// Hardening metrics
    timeout_predictions_total: CounterVec,
    backoff_retries_total: CounterVec,
    concurrency_permits_available: GaugeVec,
}
```

Key Roko-specific metrics (not just generic process health):

| Metric | Type | Operator Insight |
|---|---|---|
| `roko_c_factor` | gauge | Is collective intelligence improving? |
| `roko_gate_pass_rate` | gauge | Are agents producing passing work? |
| `roko_bus_pulses_per_second` | gauge | Is the system active? |
| `roko_cost_usd_total` | counter | How much are we spending? |
| `roko_cost_budget_remaining_usd` | gauge | When will we hit the budget? |
| `roko_store_demurrage_balance_p95` | histogram | Is memory self-trimming? |
| `roko_timeout_ema_error` | gauge | Are timeout predictions accurate? |
| `roko_heuristic_calibration_brier` | histogram | Are heuristics staying calibrated? |

### 3. OpenTelemetry Traces as Lineage-Annotated Signals

Every operator boundary (sense, assess, compose, act, verify, persist, react) emits a trace
span. Spans carry Signal lineage, enabling correlation between trace data and the durable
audit DAG in Store.

```rust
/// Trace span attributes for Roko operations.
/// These map 1:1 to the 7-step cognitive loop.
pub fn instrument_cognitive_loop(task: &Task) -> Span {
    let span = tracing::info_span!(
        "cognitive_loop",
        plan_id = %task.plan_id,
        task_id = %task.task_id,
        agent_id = %task.agent_id,
        rung = task.rung,
        // Lineage: links this span to the Signal DAG
        parent_signal_hash = %task.input_signal_hash,
    );
    span
}

/// Sub-spans for each cognitive step.
pub async fn execute_with_tracing(task: &Task) -> Result<Signal> {
    let _sense = tracing::info_span!("op.sense").entered();
    let input = sense(task).await?;
    drop(_sense);

    let _assess = tracing::info_span!("op.assess").entered();
    let route = assess(&input).await?;
    drop(_assess);

    let _compose = tracing::info_span!(
        "op.compose",
        budget_tokens = route.token_budget,
    ).entered();
    let prompt = compose(&input, &route).await?;
    drop(_compose);

    let _act = tracing::info_span!(
        "op.act",
        model = %route.model,
        provider = %route.provider,
    ).entered();
    let output = act(&prompt, &route).await?;
    drop(_act);

    let _verify = tracing::info_span!("op.verify").entered();
    let verdict = verify(&output).await?;
    drop(_verify);

    let _persist = tracing::info_span!(
        "op.persist",
        signal_hash = %output.id,
    ).entered();
    persist(&output).await?;
    drop(_persist);

    Ok(output)
}
```

Trace export configuration:

```toml
[observe.traces]
enabled = true
exporter = "otlp"                           # otlp | jaeger | none
endpoint = "http://collector:4317"          # OTLP gRPC endpoint
sample_rate = 0.1                           # 10% baseline sampling
error_sample_rate = 1.0                     # 100% on errors
service_name = "roko"
```

---

## StateHub Projections: Named Lens Compositions

StateHub projections are named Lens compositions that provide typed, queryable views of
system state. They combine Bus observation with Store queries into coherent snapshots for
dashboards and remote consumers.

```rust
/// StateHub: registry of named projections.
/// Each projection is a Lens Cell composition that maintains a live view.
pub struct StateHub {
    projections: HashMap<String, Box<dyn Projection>>,
}

/// Named projections available to all surfaces (TUI, web, CLI, remote).
pub trait Projection: Send + Sync {
    /// Current state snapshot (for HTTP GET).
    fn snapshot(&self) -> serde_json::Value;
    /// Subscribe to incremental updates (for SSE/WebSocket).
    fn subscribe(&self) -> broadcast::Receiver<ProjectionDelta>;
}

/// Example projections:
///
/// "active_tasks" -> currently running tasks with agent assignments
/// "gate_pipeline" -> current rung status, pass/fail counts
/// "cost_meter" -> spend by session, role, model
/// "bus_stats" -> pulses per second by topic
/// "safety_events" -> recent auth denials and escalations
/// "calibration_curves" -> heuristic drift trends
```

Remote consumers (web dashboard, Slack bot, another Roko instance) subscribe to projections
over SSE or WebSocket:

```
GET /projections/active_tasks/stream
  -> SSE stream with cursor-based resume
  -> Initial state snapshot, then incremental deltas
  -> Reconnect with ?cursor=0x42 to resume from last position
```

---

## Monitoring Dashboard Architecture

The observability stack composes into a layered dashboard:

```
Layer 4: Alerts (PagerDuty, Slack)
    ^
Layer 3: Dashboards (Grafana, custom web UI)
    ^
Layer 2: Storage (Prometheus, Loki, Tempo)
    ^
Layer 1: Export (metrics endpoint, log shipper, OTLP exporter)
    ^
Layer 0: Lens Cells (observe Bus + Store, emit telemetry)
```

Default alert rules (shipped as Prometheus alerting rules):

| Alert | Condition | Severity |
|---|---|---|
| `RokoCFactorFalling` | c_factor < 0.8 for 30m | warning |
| `RokoGateStalled` | gate verdicts = 0 for 15m during active run | critical |
| `RokoCostSpike` | cost rate > 3x 1h average | warning |
| `RokoSafetyEscalation` | escalations > 5 in 10m | critical |
| `RokoBusRingSaturated` | ring occupancy > 90% | warning |
| `RokoCalibrationDrift` | Brier score > 0.3 for any heuristic | warning |
| `RokoStoreBlout` | signal count growth > 10%/hour sustained | warning |

---

## Health Check Patterns

Health probes are Verify Cells that run on each check interval:

```rust
/// Health check: composition of Verify Cells.
/// /healthz = liveness (is the process fundamentally alive?)
/// /readyz = readiness (should traffic be sent here?)
pub struct HealthCheck {
    checks: Vec<Box<dyn HealthVerify>>,
}

/// Readiness fails BEFORE liveness during shutdown.
/// This ensures the orchestrator stops new traffic before killing the process.
pub async fn readyz(state: &RuntimeState) -> StatusCode {
    if !state.accepting {
        return StatusCode::SERVICE_UNAVAILABLE; // Draining: don't send traffic
    }
    if state.bus.ring_occupancy() > 0.95 {
        return StatusCode::SERVICE_UNAVAILABLE; // Backpressure: too full
    }
    StatusCode::OK
}

pub async fn healthz(state: &RuntimeState) -> StatusCode {
    if state.last_tick_age() > Duration::from_secs(120) {
        return StatusCode::INTERNAL_SERVER_ERROR; // Main loop stalled
    }
    StatusCode::OK
}
```

Health check semantics across deployment targets:

| Target | Readiness Probe | Liveness Probe |
|---|---|---|
| Docker Compose | `GET /readyz` (30s interval) | `GET /healthz` (30s interval) |
| Fly.io | `http_service.checks` (30s interval) | Machine auto-stop on persistent failure |
| systemd | `sd_notify::Watchdog` (60s window) | Process exit triggers restart |
| Kubernetes | `readinessProbe` | `livenessProbe` |

---

## What This Enables

1. **Self-healing runtime**: Adaptive timeouts + backoff + concurrency control mean the
   system adapts to provider behavior without manual tuning.

2. **Safe upgrades**: Graceful shutdown + state serialization means upgrades never lose
   in-flight work. Resume from last checkpoint on restart.

3. **Deep observability**: Not just "is the process alive?" but "is the agent getting
   smarter, spending within budget, and staying calibrated?"

4. **Unified monitoring**: Same metrics, logs, and traces across all five deployment shapes.
   Operators learn one observability model.

5. **Alert-driven operations**: Default alert rules catch cognitive degradation (c-factor
   drop, calibration drift) before it manifests as task failures.

---

## Feedback Loops

- **Timeout calibration**: The adaptive timeout Loop tracks its own prediction error via EMA.
  If error drifts above threshold, the multiplier auto-adjusts (increase multiplier = more
  conservative timeouts).

- **Backoff effectiveness**: Each retry outcome (success/failure) feeds back into the
  backoff state machine. Persistent failures trigger failover rather than infinite retries.

- **Concurrency pressure**: When the semaphore is consistently saturated (>90% utilization
  over 10 minutes), the system publishes a capacity Pulse suggesting configuration increase.

- **Observability self-monitoring**: If the metrics endpoint takes >1s to scrape, or if
  trace export is dropping spans, readiness degrades. The observability plane monitors itself.

---

## Open Questions

1. **Metrics cardinality**: Per-tenant labels on metrics risk cardinality explosion in
   multi-tenant deployments. Current approach: tenant labels only on counters (not
   histograms), with cardinality cap at 100 unique tenants per metric.

2. **Trace sampling strategy**: Should high-cost requests (Opus model calls) always be
   traced, regardless of sample rate? Leaning yes -- cost-bearing actions justify the
   tracing overhead.

3. **Log volume in debug mode**: Debug logging can produce >100MB/hour during active plan
   execution. Should there be an automatic log level reduction when disk pressure is high?

4. **Cross-node trace correlation in clustered mode**: How do traces correlate when a
   request hits multiple nodes? Standard approach: propagate trace context via HTTP headers
   (W3C Trace Context). But Bus Pulses also need trace context in their metadata.

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| AdaptiveTimeout struct + Loop | `crates/roko-agent/src/timeout.rs` | Implemented |
| BackoffReact state machine | `crates/roko-agent/src/backoff.rs` | Implemented |
| RetryAction enum + error classification | `crates/roko-agent/src/retry.rs` | Implemented |
| Per-provider concurrency semaphores | `crates/roko-agent/src/concurrency.rs` | Scaffolded |
| Graceful shutdown protocol | `crates/roko-runtime/src/supervisor.rs` | Partial |
| ShutdownProtocol 4-phase implementation | `crates/roko-cli/src/shutdown.rs` | Not started |
| Health check /readyz and /healthz | `crates/roko-serve/src/routes/health.rs` | Partial |
| Prometheus MetricsLens | `crates/roko-serve/src/observe/metrics.rs` | Not started |
| Structured log Lens (JSON/human) | `crates/roko-cli/src/observe/logs.rs` | Partial (tracing) |
| OpenTelemetry trace export | `crates/roko-cli/src/observe/traces.rs` | Not started |
| StateHub projection registry | `crates/roko-serve/src/state.rs` | Partial |
| Alert rule definitions (Prometheus) | `deploy/monitoring/alerts.yml` | Not started |
| Grafana dashboard JSON | `deploy/monitoring/dashboards/` | Not started |
| Zero-downtime upgrade test | `tests/integration/upgrade_test.rs` | Not started |
