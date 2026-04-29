# Resilience Algebra and Numerical Stability

> Depth for [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md). Derives a resilience algebra from error classification, models circuit breakers as React-protocol state machines, and specifies the precision decisions that keep the hot path numerically stable.

---

## 1. The Resilience Algebra

Errors are not a list. They form an algebra with four kinds, two operations (retry and escalate), and composition rules that determine how failures propagate through Graph execution.

### 1.1 Four Error Kinds

```rust
/// Error classification for the resilience algebra.
/// Each kind has algebraic retry and escalation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Network timeout, rate limit, temporary API error.
    /// Retry with exponential backoff. Escalate after N retries.
    Transient,

    /// Compile error, invalid config, schema mismatch.
    /// Same input = same failure. Never retry blindly.
    /// Escalate to replan or decompose.
    Deterministic,

    /// Disk full, memory pressure, too many open files.
    /// Retry after resource is freed. Escalate after timeout.
    Resource,

    /// Data corruption, missing critical files, auth revocation.
    /// Never retry. Escalate immediately.
    Catastrophic,
}
```

### 1.2 The Algebra: Operations on Error Kinds

The two operations are `retry` (attempt the same Cell again) and `escalate` (propagate to the containing Graph or human).

```
retry(Transient)      = Transient        -- retry is meaningful
retry(Deterministic)  = Deterministic    -- retry is pointless (same input, same failure)
retry(Resource)       = Resource         -- retry after resource freed
retry(Catastrophic)   = Catastrophic     -- retry is forbidden

escalate(Transient)   = after N retries  -- bounded patience
escalate(Deterministic) = immediately    -- no retry will help
escalate(Resource)    = after timeout    -- wait for resource, then give up
escalate(Catastrophic) = immediately     -- halt and preserve state
```

### 1.3 Composition: How Errors Combine

When a Graph has multiple nodes and multiple failures, the composite error kind is the supremum (worst case) under this partial order:

```
Catastrophic > Deterministic > Resource > Transient

sup(Transient, Resource)       = Resource
sup(Deterministic, Transient)  = Deterministic
sup(Catastrophic, anything)    = Catastrophic
```

This means: if any node in a parallel fan-out fails with a Catastrophic error, the entire fan-out fails as Catastrophic regardless of other nodes' status.

```rust
impl ErrorKind {
    /// Combine two error kinds. Returns the more severe.
    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Catastrophic, _) | (_, Self::Catastrophic) => Self::Catastrophic,
            (Self::Deterministic, _) | (_, Self::Deterministic) => Self::Deterministic,
            (Self::Resource, _) | (_, Self::Resource) => Self::Resource,
            _ => Self::Transient,
        }
    }

    /// Can this error kind be retried?
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Transient | Self::Resource)
    }

    /// Should this error kind escalate immediately?
    pub fn immediate_escalation(&self) -> bool {
        matches!(self, Self::Deterministic | Self::Catastrophic)
    }
}
```

### 1.4 Retry Policy as a Monoid

The retry policy composes monoidal over sequential Cell execution. Each Cell can declare its own retry policy, and the Graph-level policy provides the identity element.

```rust
struct RetryPolicy {
    base_ms: u64,
    max_delay_ms: u64,
    max_retries: u32,
    jitter_ms: u64,
}

impl RetryPolicy {
    /// Identity: the Graph-level default.
    const DEFAULT: Self = Self {
        base_ms: 500,
        max_delay_ms: 30_000,
        max_retries: 3,
        jitter_ms: 200,
    };

    /// Combine: node policy overrides graph policy for non-default fields.
    fn combine(&self, node_override: &RetryPolicy) -> Self {
        Self {
            base_ms: if node_override.base_ms != 0 { node_override.base_ms } else { self.base_ms },
            max_delay_ms: node_override.max_delay_ms.max(self.max_delay_ms),
            max_retries: node_override.max_retries.min(self.max_retries),
            jitter_ms: node_override.jitter_ms,
        }
    }

    /// Compute delay for attempt N.
    fn delay_for(&self, attempt: u32) -> Duration {
        let exp = self.base_ms.saturating_mul(2u64.saturating_pow(attempt));
        let jitter = thread_rng().gen_range(0..=self.jitter_ms);
        let total = exp.saturating_add(jitter).min(self.max_delay_ms);
        Duration::from_millis(total)
    }
}
```

The delay sequence for defaults: 500ms, 1000ms, 2000ms (+ jitter), then escalate.

---

## 2. Circuit Breaker as a React Cell

The circuit breaker is not a standalone utility -- it is a Cell implementing the React protocol. It observes failure Pulses on the Bus and emits state-transition Pulses that other Cells (especially the Route protocol) consume.

### 2.1 State Machine

```
         ┌─────────────────────────────────────┐
         │                                     │
    record_success()                      record_failure()
         │                                     │
         v                                     v
    ┌─────────┐    failure_count >= N    ┌──────────┐
    │ Closed  │ ──────────────────────►  │  Open    │
    │ (allow) │                          │ (reject) │
    └─────────┘                          └──────────┘
         ^                                     │
         │    success                          │  reset_timeout elapsed
         │                                     v
         │                              ┌────────────┐
         └───────────────────────────── │ HalfOpen   │
                                        │ (probe 1)  │
                                        └────────────┘
                                               │
                                          failure
                                               │
                                               v
                                        ┌──────────┐
                                        │  Open    │
                                        └──────────┘
```

### 2.2 The Circuit Breaker as a React Cell

```rust
struct CircuitBreakerCell {
    /// Per-provider circuit state.
    circuits: DashMap<ProviderId, CircuitState>,

    /// Configuration.
    threshold: u32,          // failures before open (default: 5)
    reset_timeout: Duration, // time before half-open (default: 300s)
}

#[derive(Debug, Clone)]
struct CircuitState {
    status: CircuitStatus,
    failure_count: u32,
    last_failure: Option<Instant>,
    consecutive_successes_in_half_open: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitStatus {
    Closed,
    Open,
    HalfOpen,
}

impl Cell for CircuitBreakerCell {
    fn protocols(&self) -> &[ProtocolId] {
        &[REACT_PROTOCOL]
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Input: stream of Verdict Signals + provider health Pulses
        let events = parse_health_events(&input);

        let mut transitions = Vec::new();

        for event in events {
            let provider = &event.provider_id;
            let mut state = self.circuits
                .entry(provider.clone())
                .or_insert_with(CircuitState::closed);

            match event.outcome {
                Outcome::Success => {
                    let old = state.status;
                    state.record_success();
                    if old != state.status {
                        transitions.push(CircuitTransition {
                            provider: provider.clone(),
                            from: old,
                            to: state.status,
                        });
                    }
                }
                Outcome::Failure(kind) if kind.retryable() => {
                    let old = state.status;
                    state.record_failure(self.threshold);
                    if old != state.status {
                        transitions.push(CircuitTransition {
                            provider: provider.clone(),
                            from: old,
                            to: state.status,
                        });
                    }
                }
                Outcome::Failure(ErrorKind::Catastrophic) => {
                    // Catastrophic: open immediately, no threshold
                    state.force_open();
                    transitions.push(CircuitTransition {
                        provider: provider.clone(),
                        from: CircuitStatus::Closed,
                        to: CircuitStatus::Open,
                    });
                }
                _ => {}
            }

            // Check for half-open transition (time-based)
            if state.status == CircuitStatus::Open {
                if let Some(last) = state.last_failure {
                    if last.elapsed() >= self.reset_timeout {
                        state.status = CircuitStatus::HalfOpen;
                        transitions.push(CircuitTransition {
                            provider: provider.clone(),
                            from: CircuitStatus::Open,
                            to: CircuitStatus::HalfOpen,
                        });
                    }
                }
            }
        }

        // Emit transition Pulses for the Route protocol to consume
        let pulses: Vec<Signal> = transitions.iter()
            .map(|t| Signal::pulse(
                Topic::new(format!("circuit.{}.{}", t.provider, t.to.as_str())),
                serde_json::to_value(t).unwrap(),
            ))
            .collect();

        Ok(pulses)
    }
}

impl CircuitState {
    fn closed() -> Self {
        Self {
            status: CircuitStatus::Closed,
            failure_count: 0,
            last_failure: None,
            consecutive_successes_in_half_open: 0,
        }
    }

    fn record_success(&mut self) {
        match self.status {
            CircuitStatus::HalfOpen => {
                // One success in half-open closes the circuit
                self.status = CircuitStatus::Closed;
                self.failure_count = 0;
                self.consecutive_successes_in_half_open = 0;
            }
            CircuitStatus::Closed => {
                self.failure_count = 0;
            }
            CircuitStatus::Open => {
                // Ignored: should not be receiving successes in Open state
            }
        }
    }

    fn record_failure(&mut self, threshold: u32) {
        self.failure_count += 1;
        self.last_failure = Some(Instant::now());

        match self.status {
            CircuitStatus::Closed => {
                if self.failure_count >= threshold {
                    self.status = CircuitStatus::Open;
                }
            }
            CircuitStatus::HalfOpen => {
                // Any failure in half-open reopens
                self.status = CircuitStatus::Open;
                self.consecutive_successes_in_half_open = 0;
            }
            CircuitStatus::Open => {
                // Already open
            }
        }
    }

    fn force_open(&mut self) {
        self.status = CircuitStatus::Open;
        self.last_failure = Some(Instant::now());
    }
}
```

### 2.3 How Route Consumes Circuit State

The Route protocol subscribes to `circuit.*.opened` and `circuit.*.closed` Pulses. When a provider's circuit opens, Route removes it from the candidate set. When it transitions to HalfOpen, Route allows one probe request.

```rust
// Inside RouteProtocol implementation
fn filter_by_circuit_state(
    &self,
    candidates: &[RouteCandidate],
    circuits: &DashMap<ProviderId, CircuitState>,
) -> Vec<RouteCandidate> {
    candidates.iter()
        .filter(|c| {
            match circuits.get(&c.provider_id) {
                Some(state) => match state.status {
                    CircuitStatus::Closed => true,
                    CircuitStatus::HalfOpen => true,  // allow one probe
                    CircuitStatus::Open => false,      // reject
                },
                None => true,  // no circuit state = assume healthy
            }
        })
        .cloned()
        .collect()
}
```

---

## 3. Graceful Degradation as a Typed State Machine

The degradation ladder is not a comment in code -- it is a state machine with Verify-gated transitions. Each level restricts system behavior, and the transition between levels is guarded by explicit conditions.

### 3.1 The Six Degradation Levels

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DegradationLevel {
    /// Full operation. All features active.
    Normal      = 0,

    /// Budget pressure reached warn_threshold.
    /// Route to cheaper models. Disable experiment exploration.
    BudgetWarn  = 1,

    /// Budget reached block_threshold.
    /// Block new tasks. Complete running tasks. Save state.
    BudgetBlock = 2,

    /// One or more provider circuits are open.
    /// Route to alternative providers. Fall back to local models.
    ProviderDegraded = 3,

    /// All providers are degraded or unreachable.
    /// Queue tasks. Retry periodically. Notify user.
    AllProvidersDegraded = 4,

    /// Disk pressure or I/O failure.
    /// Reduce logging. Prune aggressively. Warn user.
    DiskPressure = 5,

    /// Unrecoverable state.
    /// Save state. Print diagnostic. Exit with non-zero code.
    Unrecoverable = 6,
}
```

### 3.2 Transition Guards (Verify-gated)

Each transition has an explicit condition. The DegradationLens (an Observe-protocol Cell) monitors these conditions and publishes transition Pulses.

```rust
struct DegradationLens {
    current: AtomicU8,  // DegradationLevel as u8
}

impl Cell for DegradationLens {
    fn protocols(&self) -> &[ProtocolId] {
        &[OBSERVE_PROTOCOL]
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let health = SystemHealth::from_signals(&input)?;
        let current = DegradationLevel::from_u8(
            self.current.load(Ordering::Relaxed)
        );

        let next = self.evaluate(&health);

        if next != current {
            // Verify the transition is warranted (hysteresis)
            if self.transition_confirmed(current, next, &health) {
                self.current.store(next as u8, Ordering::Relaxed);
                return Ok(vec![Signal::pulse(
                    Topic::new(format!("system.degradation.{}", next.as_str())),
                    DegradationTransition { from: current, to: next, health }.to_value(),
                )]);
            }
        }

        Ok(vec![])
    }
}

impl DegradationLens {
    fn evaluate(&self, health: &SystemHealth) -> DegradationLevel {
        // Ordered from most severe to least severe
        if health.unrecoverable_error {
            return DegradationLevel::Unrecoverable;
        }
        if health.disk_utilization > 0.95 {
            return DegradationLevel::DiskPressure;
        }
        if health.all_providers_down() {
            return DegradationLevel::AllProvidersDegraded;
        }
        if health.any_provider_down() {
            return DegradationLevel::ProviderDegraded;
        }
        if health.budget_utilization > health.block_threshold {
            return DegradationLevel::BudgetBlock;
        }
        if health.budget_utilization > health.warn_threshold {
            return DegradationLevel::BudgetWarn;
        }
        DegradationLevel::Normal
    }

    fn transition_confirmed(
        &self,
        from: DegradationLevel,
        to: DegradationLevel,
        health: &SystemHealth,
    ) -> bool {
        // Degradation (getting worse): require 2 consecutive observations
        if to > from {
            return health.consecutive_degraded_observations >= 2;
        }
        // Recovery (getting better): require 5 consecutive observations
        // Asymmetric: slower to recover than to degrade (hysteresis)
        if to < from {
            return health.consecutive_healthy_observations >= 5;
        }
        false
    }
}
```

### 3.3 Per-Level Behavioral Restrictions

Each level restricts what the Engine does. These restrictions are enforced at the Engine level, not by individual Cells.

```rust
impl Engine {
    fn apply_degradation(&self, level: DegradationLevel) {
        match level {
            DegradationLevel::Normal => {
                // Full operation
            }
            DegradationLevel::BudgetWarn => {
                // Route to cheaper models
                self.route_constraints.set_max_tier(CognitiveTier::T1Deliberate);
                // Disable exploration
                self.learning_config.exploration_rate.store(0.0);
            }
            DegradationLevel::BudgetBlock => {
                // Block new Flows
                self.accept_new_flows.store(false, Ordering::Relaxed);
                // Complete running Flows
                // Save state proactively
                self.snapshot_all_flows().await;
            }
            DegradationLevel::ProviderDegraded => {
                // Route protocol already handles via circuit breaker
                // Additionally: emit user notification
            }
            DegradationLevel::AllProvidersDegraded => {
                // Queue incoming tasks
                self.accept_new_flows.store(false, Ordering::Relaxed);
                // Start periodic probe timer
            }
            DegradationLevel::DiskPressure => {
                // Reduce log verbosity to error-only
                // Trigger aggressive GC on Store
                // Prune Signals below demurrage threshold
            }
            DegradationLevel::Unrecoverable => {
                // Save all state
                self.snapshot_all_flows().await;
                // Flush episodes
                // Exit with diagnostic
                std::process::exit(1);
            }
        }
    }
}
```

---

## 4. Numerical Stability: Where Precision Matters

Not every float decision matters equally. The ones that matter are on the hot path or accumulate over time.

### 4.1 The f32 vs f64 Decision Table

| Domain | Type | Why |
|---|---|---|
| Score axes (7 dimensions) | `f32` | Range [-1.0, 1.0]. 7 significant digits is sufficient for ranking. Stored per-Signal at high volume. |
| PAD vector (3 dimensions) | `f32` | Range [-1.0, 1.0]. Psychometric resolution does not need f64. |
| demurrage balance | `f32` | Range [0.0, 1.0]. The balance is a ratio, not a dollar amount. |
| HDC vectors | `u64` bitfield | Binary. No floating-point at all. 10,240 bits = 160 `u64`s. |
| Cost tracking (USD) | `f64` | Accumulates across the entire session. f32 loses precision past $16,777 (2^24). |
| EMA thresholds | `f64` | Small alpha (0.05) compounds rounding. After 1000 updates, f32 drift is measurable. |
| Bandit arm parameters | `f64` | UCB1 and Thompson sampling convergence depends on parameter precision. |
| Timestamps | `i64` | Millisecond Unix. No floating-point. |
| Token counts | `usize` | Integer. Saturating arithmetic. |
| Metric counters | `u64` | Monotonic. Never round through float. |

### 4.2 The Hot-Path Budget Table

These are the time budgets for the inner loop. Exceeding them delays the cognitive loop tick.

| Operation | Budget | Notes |
|---|---|---|
| `Decay::apply()` | < 10ns | Single `powf` or `exp`. Inline candidate. |
| `Score::effective()` | < 50ns | Weighted sum of 7 `f32`s. |
| HDC Hamming distance | < 1us | 160 `popcnt` on `u64` XOR result. |
| CorticalState read | < 1us | Single atomic load per field. |
| Metric counter increment | < 250ns | No heap allocation. |
| Histogram observation | < 750ns | Fixed bucket family. |
| Trace span start/finish | < 10us | Attribute copy. Exporter excluded. |
| Structured log enqueue | < 50us | JSON serialization may spill to background. |
| Prompt assembly | < 5ms | Token counting dominates. |
| Cascade router select | < 100us | Candidate scoring + bandit. |
| Episode log write | < 1ms | JSONL append. |
| Flow snapshot write | < 10ms | JSON serialize + atomic rename. |

**Total non-LLM overhead per tick**: < 20ms. This leaves the LLM call as the dominant cost, which is the correct budget distribution.

### 4.3 Serialization Precision

When Signals are serialized to JSONL, floating-point values need stable precision to avoid bloating storage with insignificant digits.

```rust
/// Round f32 to N decimal places before serialization.
/// Apply at serialization boundaries, not at every computation step.
fn round_f32(v: f32, decimals: u32) -> f32 {
    let factor = 10_f32.powi(decimals as i32);
    (v * factor).round() / factor
}
```

| Domain | Decimal places | Example | Storage impact |
|---|---|---|---|
| Score axis | 4 | 0.8500 | "0.85" vs "0.8499999..." |
| demurrage ratio | 6 | 0.002500 | "0.0025" vs "0.002499999..." |
| Cost (USD) | 4 | 12.3456 | Consistent with pricing granularity |
| EMA threshold | 6 | 0.654321 | Preserves convergence signal |
| Calibration gauge | 4 | 0.8125 | Display-friendly |

### 4.4 NaN/Inf Defense

The defensive pattern applies at computation boundaries. Clamping at every intermediate step masks bugs; clamping at output boundaries catches them.

```rust
/// Apply at the output boundary of any Cell that produces f32/f64.
/// Log anomalies for debugging. Clamp to the valid range.
trait NumericallyStable {
    fn stabilize(self, name: &str, min: Self, max: Self, default: Self) -> Self;
}

impl NumericallyStable for f32 {
    fn stabilize(self, name: &str, min: f32, max: f32, default: f32) -> f32 {
        if self.is_nan() || self.is_infinite() {
            tracing::warn!(
                value = %self,
                field = name,
                "numerical anomaly, using default"
            );
            return default;
        }
        self.clamp(min, max)
    }
}

impl NumericallyStable for f64 {
    fn stabilize(self, name: &str, min: f64, max: f64, default: f64) -> f64 {
        if self.is_nan() || self.is_infinite() {
            tracing::warn!(
                value = %self,
                field = name,
                "numerical anomaly, using default"
            );
            return default;
        }
        self.clamp(min, max)
    }
}
```

### 4.5 Specific NaN/Inf Sources and Mitigations

| Source | How it arises | Mitigation |
|---|---|---|
| `0.0 / 0.0` | Division by zero in score normalization | Check denominator before division |
| `exp(710.0_f64)` | Overflow in demurrage exponent | Clamp exponent input to 700.0 |
| `powf(0.5, 0.0 / 0.0)` | NaN propagation from `half_life_ms = 0` | Guard: `if half_life_ms == 0 { return 0.0; }` |
| `(-1.0_f32).sqrt()` | Negative sqrt | Never occurs: all inputs are non-negative by construction |
| `Inf - Inf` | Indeterminate from unbounded accumulation | Avoid unbounded accumulation; use saturating arithmetic |

### 4.6 EMA Precision Under Long Runs

EMA with small alpha accumulates rounding error in f32:

```
EMA formula: new = alpha * sample + (1.0 - alpha) * old

After N updates with alpha = 0.05:
  f32 drift from true value: ~1e-4 at N=1000, ~1e-3 at N=10000
  f64 drift from true value: ~1e-12 at N=1000, ~1e-11 at N=10000
```

For adaptive gate thresholds that accumulate over thousands of gate evaluations, f64 is mandatory. The error compounds because each update reads the previous (already-rounded) value.

The concrete risk: an f32 EMA for gate thresholds could drift by 0.001 after 10,000 updates, which is enough to flip borderline pass/fail decisions. For this reason, all EMA computations use f64 and only round to f32 at serialization time.

---

## 5. Error Propagation Through Graphs

Errors propagate through the Graph's node hierarchy. Each layer catches errors from the layer below and decides: retry, escalate, or absorb.

### 5.1 Propagation Hierarchy

```
Cell error
  |
  v
Node failure strategy (retry, decompose, skip, etc.)
  |
  v
Graph failure strategy (fail, compensate, replan)
  |
  v
Flow failure state
  |
  v
Engine: log, snapshot, notify
  |
  v
CLI/API: show to user
```

### 5.2 Absorption Rules

Some errors are absorbed -- logged but not propagated upward. These are non-critical subsystems where failure should not halt the cognitive loop.

```rust
/// Errors from these subsystems are absorbed.
/// The system continues without the failed subsystem.
const ABSORBABLE_SUBSYSTEMS: &[&str] = &[
    "episode_logger",       // Learning is optional
    "metric_emitter",       // Observability is not critical
    "dashboard_renderer",   // Display errors are cosmetic
    "playbook_extractor",   // Skills improve future tasks only
    "experiment_tracker",   // A/B testing is best-effort
];
```

### 5.3 Escalation Rules

Some errors escalate immediately regardless of retry policy.

```rust
fn should_escalate_immediately(error: &CellError) -> bool {
    match error.kind() {
        // Auth failure: cannot be fixed by retry
        ErrorKind::Catastrophic if error.is_auth() => true,

        // Budget exceeded: policy decision, not technical failure
        ErrorKind::Resource if error.is_budget() => true,

        // State corruption: risk of data loss requires human
        ErrorKind::Catastrophic if error.is_corruption() => true,

        // Config parse error: cannot start without valid config
        ErrorKind::Deterministic if error.is_config() => true,

        _ => false,
    }
}
```

---

## 6. Rate Limit Handling

Rate limits (HTTP 429) are a special case of Transient errors that deserve their own retry logic.

```rust
fn handle_rate_limit(
    response: &HttpResponse,
    policy: &RetryPolicy,
    attempt: u32,
) -> Duration {
    // Prefer provider's Retry-After header
    if let Some(retry_after) = response.header("Retry-After") {
        if let Ok(secs) = retry_after.parse::<u64>() {
            return Duration::from_secs(secs.min(120));
        }
    }

    // Fall back to exponential backoff
    policy.delay_for(attempt)
}
```

When rate limits persist across multiple requests, the circuit breaker opens. The Route protocol then routes to alternative providers. This creates a cascade: 429 -> retry -> circuit open -> route elsewhere.

---

## 7. Resilience Observability

The resilience system observes itself. Each component publishes health Pulses that the DegradationLens consumes.

### 7.1 The Resilience Loop

```
Circuit breaker emits:     circuit.{provider}.opened / closed / half_open
Degradation lens emits:    system.degradation.{level}
Route protocol consumes:   circuit.{provider}.*
Engine consumes:           system.degradation.*
Learning loops consume:    circuit.{provider}.* (for provider health tracking)
```

This is a Loop in the unified sense: a Graph with a feedback edge. The circuit breaker's output (Pulses) feeds back into the Route protocol's input, which affects future Cell executions, which produce outcomes that feed back into the circuit breaker.

### 7.2 Resilience Metrics

| Metric | Type | What it measures |
|---|---|---|
| `circuit_state` | Gauge per provider | Current circuit status (0=closed, 1=half-open, 2=open) |
| `circuit_transitions_total` | Counter per provider | Total state transitions |
| `retry_attempts_total` | Counter per error kind | Retries by error classification |
| `degradation_level` | Gauge | Current degradation level (0-6) |
| `error_kind_total` | Counter per kind | Errors by classification |
| `time_in_degradation_seconds` | Counter per level | Cumulative time at each level |

---

## 8. What This Enables

- **Algebraic error handling**: Error kinds compose predictably. A system of parallel Cells with mixed error kinds resolves to a single composite kind via the supremum rule.
- **Circuit breakers as first-class Cells**: Not utility code hidden in the agent dispatcher, but React-protocol Cells that publish Pulses consumed by the Route protocol.
- **Verify-gated degradation**: The degradation ladder transitions are guarded by observation counts (hysteresis), not raw threshold crossings.
- **Numerically stable hot path**: The f32/f64 decision table and serialization precision rules prevent drift in long-running agents.
- **Self-observing resilience**: The resilience system itself is a Loop that monitors its own health and publishes metrics.

---

## 9. Feedback Loops

| Loop | Observes | Adjusts |
|---|---|---|
| **Circuit breaker** | Provider failure/success outcomes | Provider availability for the Route protocol |
| **Degradation lens** | Budget utilization, provider health, disk state | System-wide behavioral restrictions |
| **Retry policy adaptation** | Success rate after N retries | Whether to increase/decrease max_retries (future: bandit over retry policies) |
| **Error classification refinement** | Errors initially classified as Transient that never recover | Reclassify as Deterministic after N failed retries |
| **Resilience metrics** | All of the above | Dashboard visibility; alerting thresholds |

---

## 10. Open Questions

1. **Partial success in parallel fan-outs**: When 3 of 4 parallel nodes succeed and one fails with a Deterministic error, should the Graph proceed with partial results? The current algebra says the composite error is Deterministic (escalate). But partial results may be valuable. Should there be a `PartialSuccess` state?

2. **Circuit breaker sharing across agents**: Should agents share circuit breaker state? If Agent A opens a circuit for a provider, should Agent B respect it? The current model is per-Engine, which means per-process. Multi-agent deployments may need a shared circuit breaker via the Bus.

3. **EMA warm-start on resume**: When a Flow resumes from snapshot, the EMA for gate thresholds may be stale (computed hours ago). Should the EMA incorporate a "staleness penalty" that widens the threshold until fresh data arrives?

4. **Catastrophic error recovery**: The current model says Catastrophic errors never retry. But some "catastrophic" failures are transient in disguise (e.g., auth token expired but auto-refreshes). Should there be a `MaybeCatastrophic` kind with a single probe retry?

5. **Numerical drift alerting**: The system logs NaN/Inf anomalies but does not proactively detect drift. Should there be a Lens that monitors f64 values for monotonic drift (e.g., EMA trending toward 0.0 or 1.0 without oscillation)?

---

## Cross-References

- [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md) -- Engine failure strategies, budget enforcement, snapshot format
- [02-CELL.md](../../unified/02-CELL.md) SS3.7 -- React protocol definition
- [04-SPECIALIZATIONS.md](../../unified/04-SPECIALIZATIONS.md) SS5 -- Lens as Observe-protocol Cell
- [07-AGENT-RUNTIME.md](../../unified/07-AGENT-RUNTIME.md) SS3 -- Vitality phases and degradation
- [cognitive-loop-as-graph.md](cognitive-loop-as-graph.md) -- How the cognitive loop uses failure strategies
