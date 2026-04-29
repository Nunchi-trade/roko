# Circuit Breaker and Interventions

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How circuit breakers, graduated interventions, and AIMD concurrency emerge from state-machine Cells and predict-publish-correct feedback.

---

## 1. Overview

When an agent repeatedly fails at the same plan, the system needs to stop throwing resources at it. When errors begin trending upward, the system should react before the budget is exhausted. When recovery succeeds, the system should cautiously ramp back up. When multiple agents compete for finite LLM capacity, the system needs to adjust concurrency without oscillation.

All four of these behaviors -- circuit breaking, predictive tripping, graduated recovery, and concurrency control -- are instances of the same kernel pattern: a **state-machine Cell** that receives Signals, transitions between internal states, publishes state transitions as Pulses, and corrects its predictions via predict-publish-correct feedback. No bespoke structs are needed. The circuit breaker is a Cell. The AIMD controller is a Cell. They are registered in the Cell registry, composed into Graphs, and configured via TOML parameters like any other Cell.

---

## 2. Circuit Breaker as a State-Machine Cell

### Three states

The circuit breaker has three states. These are internal Cell state, not separate types or separate structs. The Cell persists its state to Store between ticks, and the Engine restores it on resume.

```
    +--------+    failure_count >= threshold    +------+
    | Closed |--------------------------------->| Open |
    +--------+                                  +------+
        ^                                          |
        |  probe succeeds                          | cooldown expires
        |                                          v
        +---------- HalfOpen <---------------------+
                    (probe_fraction)
```

- **Closed**: Normal operation. Failures are counted. When the count reaches the threshold (default: MAX_PLAN_FAILURES=2), the breaker transitions to Open.
- **Open**: No work is dispatched for this plan. After a cooldown period, the breaker transitions to HalfOpen.
- **HalfOpen**: A fraction of requests are allowed through as probes. If a probe succeeds, the breaker transitions to Closed (with the failure count reset). If a probe fails, the breaker transitions back to Open with a longer cooldown.

### The Cell implementation

```rust
/// Circuit breaker as a state-machine Cell.
///
/// One instance per plan. Receives outcome Signals (success or failure)
/// and publishes state-transition Pulses. The orchestrator subscribes
/// to these Pulses to decide whether to dispatch work.
pub struct CircuitBreakerCell {
    /// Plan ID this breaker protects.
    plan_id: String,

    /// Current state.
    state: BreakerState,

    /// Configuration (thresholds, cooldowns, ramp schedule).
    config: BreakerConfig,

    /// Holt forecaster for predictive breaking.
    forecaster: HoltForecaster,

    /// Total evaluations (successes + failures) for error rate.
    eval_count: (u32, u32),
}

/// The three states plus their associated data.
pub enum BreakerState {
    Closed {
        /// Per-plan failure record.
        failures: FailureRecord,
    },
    Open {
        /// When the breaker opened (unix ms).
        opened_at_ms: i64,
        /// Cooldown duration (ms). Increases on repeated failures.
        cooldown_ms: i64,
        /// Reason the breaker opened.
        reason: String,
    },
    HalfOpen {
        /// Fraction of requests to allow as probes (0.0 to 1.0).
        probe_fraction: f64,
        /// Number of probes sent in this half-open window.
        probes_sent: u32,
        /// Number of probes that succeeded.
        probes_succeeded: u32,
    },
}

/// Configuration for the circuit breaker Cell.
pub struct BreakerConfig {
    /// Maximum failures before tripping (default: 2).
    pub max_failures: u32,

    /// Base cooldown in milliseconds when breaker opens (default: 30_000).
    pub base_cooldown_ms: i64,

    /// Error-specific cooldown overrides.
    pub error_cooldowns: HashMap<ErrorKind, i64>,

    /// Ramp schedule for graduated half-open (default: [0.05, 0.10, 0.20, 0.40, 0.80, 1.00]).
    pub ramp_schedule: Vec<f64>,

    /// Ramp factor: multiplier for next probe fraction on success (default: 2.0).
    pub ramp_factor: f64,

    /// Whether predictive breaking is enabled (default: true).
    pub predictive: bool,

    /// Forecast trip threshold (default: 0.5). When the forecasted
    /// error rate at horizon 1 exceeds this, the breaker proactively opens.
    pub forecast_trip_threshold: f64,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            max_failures: 2,
            base_cooldown_ms: 30_000,
            error_cooldowns: default_error_cooldowns(),
            ramp_schedule: vec![0.05, 0.10, 0.20, 0.40, 0.80, 1.00],
            ramp_factor: 2.0,
            predictive: true,
            forecast_trip_threshold: 0.5,
        }
    }
}

/// Error-specific cooldown durations.
/// Different error types have different recovery characteristics.
fn default_error_cooldowns() -> HashMap<ErrorKind, i64> {
    let mut m = HashMap::new();
    m.insert(ErrorKind::RateLimit, 5_000);        // 5s: transient, retry soon
    m.insert(ErrorKind::Timeout, 10_000);          // 10s: likely load-related
    m.insert(ErrorKind::ServerError, 30_000);      // 30s: provider issue
    m.insert(ErrorKind::AuthFailure, 300_000);     // 5min: needs human intervention
    m.insert(ErrorKind::ContentPolicy, 300_000);   // 5min: prompt needs redesign
    m.insert(ErrorKind::ContextOverflow, i64::MAX); // No fallback: the context is too large
    m
}
```

### Cell trait implementation

The circuit breaker implements the `Cell` trait. Its `execute` method receives outcome Signals and produces state-transition Signals. It publishes Pulses on Bus for real-time notification.

```rust
impl Cell for CircuitBreakerCell {
    fn id(&self) -> CellId {
        CellId::from_name_version(
            &format!("circuit_breaker.{}", self.plan_id),
            "1.0.0",
        )
    }

    fn name(&self) -> &str { "circuit_breaker" }

    fn input_schema(&self) -> Option<&TypeSchema> {
        // Accepts: TaskOutcome signals (success or failure with error kind)
        Some(&TASK_OUTCOME_SCHEMA)
    }

    fn output_schema(&self) -> Option<&TypeSchema> {
        // Produces: BreakerTransition signals
        Some(&BREAKER_TRANSITION_SCHEMA)
    }

    fn protocols(&self) -> &[ProtocolId] {
        // The circuit breaker is a state machine. It does not neatly fit
        // a single protocol -- it observes (Observe), reacts (React),
        // and verifies (Verify). We register it as Verify because its
        // primary output is "should this plan be allowed to proceed?"
        &[ProtocolId::Verify]
    }

    fn estimated_cost(&self) -> MicroCents {
        MicroCents(0) // Pure state machine, no external calls
    }
}

impl CircuitBreakerCell {
    /// Process a task outcome and return the breaker's verdict.
    pub fn process(&mut self, outcome: &TaskOutcome, now_ms: i64, bus: &dyn Bus) -> BreakerVerdict {
        match &mut self.state {
            BreakerState::Closed { failures } => {
                self.handle_closed(outcome, failures, now_ms, bus)
            }
            BreakerState::Open { opened_at_ms, cooldown_ms, .. } => {
                self.handle_open(*opened_at_ms, *cooldown_ms, now_ms, bus)
            }
            BreakerState::HalfOpen { probe_fraction, probes_sent, probes_succeeded } => {
                self.handle_half_open(
                    outcome, *probe_fraction, probes_sent, probes_succeeded, now_ms, bus,
                )
            }
        }
    }

    fn handle_closed(
        &mut self,
        outcome: &TaskOutcome,
        failures: &mut FailureRecord,
        now_ms: i64,
        bus: &dyn Bus,
    ) -> BreakerVerdict {
        match outcome {
            TaskOutcome::Success => {
                // Update forecaster with success observation (0.0).
                if self.config.predictive {
                    self.forecaster.update(0.0);
                    self.eval_count.1 += 1;
                }
                BreakerVerdict::Allow
            }
            TaskOutcome::Failure { error_kind, reason } => {
                failures.count += 1;
                failures.last_failure_ms = Some(now_ms);
                failures.reasons.push(reason.clone());

                // Update forecaster with failure observation (1.0).
                if self.config.predictive {
                    self.forecaster.update(1.0);
                    self.eval_count.0 += 1;
                    self.eval_count.1 += 1;
                }

                // Count-based trip.
                if failures.count >= self.config.max_failures {
                    return self.trip(error_kind, reason, now_ms, bus);
                }

                // Predictive trip: forecast at horizon 1 exceeds threshold.
                if self.config.predictive
                    && self.forecaster.observation_count() >= 2
                    && self.forecaster.forecast(1) >= self.config.forecast_trip_threshold
                {
                    return self.trip(
                        error_kind,
                        &format!("predictive trip: forecast={:.2}", self.forecaster.forecast(1)),
                        now_ms,
                        bus,
                    );
                }

                BreakerVerdict::Allow
            }
        }
    }

    fn trip(
        &mut self,
        error_kind: &ErrorKind,
        reason: &str,
        now_ms: i64,
        bus: &dyn Bus,
    ) -> BreakerVerdict {
        let cooldown_ms = self.config.error_cooldowns
            .get(error_kind)
            .copied()
            .unwrap_or(self.config.base_cooldown_ms);

        self.state = BreakerState::Open {
            opened_at_ms: now_ms,
            cooldown_ms,
            reason: reason.to_string(),
        };

        // Publish state transition as Pulse.
        bus.publish(Pulse {
            kind: PulseKind::CircuitBreakerTripped {
                plan_id: self.plan_id.clone(),
                reason: reason.to_string(),
                cooldown_ms,
            },
            source: self.id(),
        });

        BreakerVerdict::Reject {
            reason: reason.to_string(),
        }
    }

    fn handle_open(
        &mut self,
        opened_at_ms: i64,
        cooldown_ms: i64,
        now_ms: i64,
        bus: &dyn Bus,
    ) -> BreakerVerdict {
        // Check if cooldown has expired.
        if now_ms - opened_at_ms >= cooldown_ms {
            // Transition to HalfOpen with the first ramp step.
            let initial_fraction = self.config.ramp_schedule
                .first()
                .copied()
                .unwrap_or(0.05);

            self.state = BreakerState::HalfOpen {
                probe_fraction: initial_fraction,
                probes_sent: 0,
                probes_succeeded: 0,
            };

            bus.publish(Pulse {
                kind: PulseKind::CircuitBreakerHalfOpen {
                    plan_id: self.plan_id.clone(),
                    probe_fraction: initial_fraction,
                },
                source: self.id(),
            });

            BreakerVerdict::Probe { fraction: initial_fraction }
        } else {
            BreakerVerdict::Reject {
                reason: "circuit breaker open, cooldown not expired".to_string(),
            }
        }
    }

    fn handle_half_open(
        &mut self,
        outcome: &TaskOutcome,
        probe_fraction: f64,
        probes_sent: &mut u32,
        probes_succeeded: &mut u32,
        now_ms: i64,
        bus: &dyn Bus,
    ) -> BreakerVerdict {
        *probes_sent += 1;

        match outcome {
            TaskOutcome::Success => {
                *probes_succeeded += 1;

                // Ramp up: increase probe fraction.
                let next_fraction = (probe_fraction * self.config.ramp_factor).min(1.0);

                if next_fraction >= 1.0 {
                    // Fully recovered. Transition to Closed.
                    self.state = BreakerState::Closed {
                        failures: FailureRecord::default(),
                    };
                    self.forecaster = HoltForecaster::default();
                    self.eval_count = (0, 0);

                    bus.publish(Pulse {
                        kind: PulseKind::CircuitBreakerClosed {
                            plan_id: self.plan_id.clone(),
                        },
                        source: self.id(),
                    });

                    BreakerVerdict::Allow
                } else {
                    // Continue half-open with higher probe fraction.
                    self.state = BreakerState::HalfOpen {
                        probe_fraction: next_fraction,
                        probes_sent: *probes_sent,
                        probes_succeeded: *probes_succeeded,
                    };

                    BreakerVerdict::Probe { fraction: next_fraction }
                }
            }
            TaskOutcome::Failure { error_kind, reason } => {
                // Probe failed. Reset to Open with longer cooldown.
                let base = self.config.error_cooldowns
                    .get(error_kind)
                    .copied()
                    .unwrap_or(self.config.base_cooldown_ms);
                let extended_cooldown = (base as f64 * 1.5) as i64; // 50% longer on repeated failure

                self.state = BreakerState::Open {
                    opened_at_ms: now_ms,
                    cooldown_ms: extended_cooldown,
                    reason: reason.clone(),
                };

                // Reset ramp to initial fraction for next half-open.
                bus.publish(Pulse {
                    kind: PulseKind::CircuitBreakerTripped {
                        plan_id: self.plan_id.clone(),
                        reason: format!("probe failed: {reason}"),
                        cooldown_ms: extended_cooldown,
                    },
                    source: self.id(),
                });

                BreakerVerdict::Reject { reason: reason.clone() }
            }
        }
    }
}

/// The breaker's verdict for a given outcome.
pub enum BreakerVerdict {
    /// Allow the request through (Closed state or HalfOpen probe succeeded).
    Allow,
    /// Reject the request (Open state or HalfOpen probe failed).
    Reject { reason: String },
    /// Allow a fraction of requests as probes (HalfOpen state).
    Probe { fraction: f64 },
}
```

---

## 3. Predictive Breaking as Predict-Publish-Correct

### The kernel pattern

Every Cell in Roko is a learner via predict-publish-correct (Friston 2006). The circuit breaker uses this pattern explicitly: the Holt forecaster **predicts** the future error rate, the actual outcome **publishes** reality, and the EWMA update **corrects** the prediction.

```
    Predict: forecast(1) = level + 1 * trend
       |
       v
    Publish: actual outcome arrives (0.0 = success, 1.0 = failure)
       |
       v
    Correct: level' = alpha * actual + (1 - alpha) * (level + trend)
             trend' = beta * (level' - level) + (1 - beta) * trend
```

### Holt double exponential smoothing

The Holt forecaster maintains two components: a smoothed level and a trend. The level tracks the current error rate. The trend tracks the direction (rising or falling). Together they produce a forecast at any horizon h: `forecast(h) = level + h * trend`.

```rust
/// Holt double exponential smoothing forecaster.
/// Used by the circuit breaker Cell for predictive tripping.
///
/// Two parameters:
/// - alpha (0.3): level smoothing. Higher = more responsive to recent observations.
/// - beta (0.1): trend smoothing. Higher = faster trend response.
///
/// The forecaster needs at least 2 observations to have a meaningful
/// trend estimate. Before that, the forecast is just the level.
pub struct HoltForecaster {
    /// Smoothed level component (current error rate estimate).
    pub level: f64,
    /// Trend component (rate of change of error rate).
    pub trend: f64,
    /// Level smoothing factor.
    pub alpha: f64,
    /// Trend smoothing factor.
    pub beta: f64,
    /// Total observations fed to the forecaster.
    pub observations: u32,
}

impl HoltForecaster {
    /// Update the forecaster with a new observation.
    /// observation = 1.0 for failure, 0.0 for success.
    pub fn update(&mut self, observation: f64) {
        if self.observations == 0 {
            // First observation: initialize level, no trend.
            self.level = observation;
            self.trend = 0.0;
        } else {
            // Holt update equations.
            let prev_level = self.level;
            self.level = self.alpha * observation
                + (1.0 - self.alpha) * (self.level + self.trend);
            self.trend = self.beta * (self.level - prev_level)
                + (1.0 - self.beta) * self.trend;
        }
        self.observations += 1;
    }

    /// Forecast the error rate at `horizon` steps ahead.
    pub fn forecast(&self, horizon: usize) -> f64 {
        self.level + (horizon as f64) * self.trend
    }
}
```

### Proactive trip signals

The circuit breaker Cell checks the forecast at two horizons after each outcome:

- **Horizon 3 warning**: If `forecast(3) >= threshold`, the breaker publishes a `ProactiveWarning` Pulse. The conductor's Gamma loop receives this and emits a `Cooldown` cognitive signal.
- **Horizon 1 trip**: If `forecast(1) >= threshold`, the breaker proactively opens. This avoids the cost of the Nth failure -- the system stops before the count-based threshold is reached.

```rust
impl CircuitBreakerCell {
    /// Check for proactive signals after updating the forecaster.
    fn check_proactive(&self, bus: &dyn Bus) {
        if !self.config.predictive || self.forecaster.observations < 2 {
            return;
        }

        let h1 = self.forecaster.forecast(1);
        let h3 = self.forecaster.forecast(3);

        if h1 >= self.config.forecast_trip_threshold {
            // Proactive trip at horizon 1.
            bus.publish(Pulse {
                kind: PulseKind::ProactiveTrip {
                    plan_id: self.plan_id.clone(),
                    forecast_h1: h1,
                },
                source: self.id(),
            });
        } else if h3 >= self.config.forecast_trip_threshold {
            // Warning at horizon 3.
            bus.publish(Pulse {
                kind: PulseKind::ProactiveWarning {
                    plan_id: self.plan_id.clone(),
                    forecast_h3: h3,
                },
                source: self.id(),
            });
        }
    }
}
```

The count-based trip is always retained as a fallback. The forecaster can miss sudden failures (e.g., an auth token expires). The count-based check catches these.

---

## 4. Feature-Level Breakers: Same Cell, Different Instances

### One Cell, many instantiations

The circuit breaker is not plan-specific in its logic -- only in its state. The same `CircuitBreakerCell` is instantiated once per capability, each with its own configuration. This eliminates the separate `FeatureLevelBreaker` concept from the old design.

| Instance | Protects | Config override | Has fallback? |
|---|---|---|---|
| `circuit_breaker.plan.{id}` | Plan-level execution | max_failures=2, base_cooldown=30s | No (plan aborts) |
| `circuit_breaker.gate_rung` | Gate rung evaluation | max_failures=3, base_cooldown=10s | No (compile/test must pass) |
| `circuit_breaker.context_enrichment` | Context enrichment pipeline | max_failures=5, base_cooldown=5s | Yes (skip enrichment, use raw context) |
| `circuit_breaker.research` | Research/web search | max_failures=3, base_cooldown=15s | Yes (skip research, use cached) |
| `circuit_breaker.doc_update` | Documentation updates | max_failures=5, base_cooldown=5s | Yes (skip docs, proceed without) |
| `circuit_breaker.review_cycle` | Review/revision cycles | max_failures=3, base_cooldown=20s | Yes (accept current output) |

```rust
/// Instantiate feature-level circuit breakers from config.
fn build_feature_breakers(config: &ConductorConfig) -> Vec<CircuitBreakerCell> {
    config.feature_breakers.iter().map(|fb| {
        CircuitBreakerCell {
            plan_id: fb.name.clone(),
            state: BreakerState::Closed {
                failures: FailureRecord::default(),
            },
            config: BreakerConfig {
                max_failures: fb.max_failures,
                base_cooldown_ms: fb.base_cooldown_ms,
                error_cooldowns: fb.error_cooldowns.clone()
                    .unwrap_or_else(default_error_cooldowns),
                ramp_schedule: fb.ramp_schedule.clone()
                    .unwrap_or_else(|| vec![0.05, 0.10, 0.20, 0.40, 0.80, 1.00]),
                ramp_factor: fb.ramp_factor.unwrap_or(2.0),
                predictive: fb.predictive.unwrap_or(true),
                forecast_trip_threshold: fb.forecast_trip_threshold.unwrap_or(0.5),
            },
            forecaster: HoltForecaster::default(),
            eval_count: (0, 0),
        }
    }).collect()
}
```

### No-fallback gates

Compile and test gates have **no fallback**. When their circuit breaker trips, the plan fails. The breaker's `Reject` verdict propagates up through the Verify Pipeline as a terminal `Verdict`. This is enforced by the absence of a fallback Cell in the Graph:

```toml
# Gate rung breaker: no fallback node
[[graph.nodes]]
id = "gate_rung_breaker"
cell = "circuit_breaker"
params = { name = "gate_rung", max_failures = 3, base_cooldown_ms = 10000 }
# No fallback edge -- Reject verdict is terminal
```

### Fallback-capable features

Context enrichment, research, doc updates, and review cycles have fallback Cells. When their breaker trips, the Graph routes around them:

```toml
# Research breaker with fallback
[[graph.nodes]]
id = "research_breaker"
cell = "circuit_breaker"
params = { name = "research", max_failures = 3, base_cooldown_ms = 15000 }

[[graph.nodes]]
id = "research_cell"
cell = "research.web_search"

[[graph.nodes]]
id = "research_fallback"
cell = "research.cached_only"

# Branch: if breaker allows, do research. If rejected, use cache.
[[graph.edges]]
from = "research_breaker"
to = "research_cell"
condition = "verdict.allow"

[[graph.edges]]
from = "research_breaker"
to = "research_fallback"
condition = "verdict.reject"
```

---

## 5. Graduated Half-Open: The Ramp as a Rack Parameter

### The ramp schedule

When the circuit breaker transitions from Open to HalfOpen, it does not immediately allow 100% of requests. It follows a graduated ramp schedule, allowing an increasing fraction of requests through as probes:

```
5% --> 10% --> 20% --> 40% --> 80% --> 100%
       (ramp_factor = 2.0 between steps)
```

Each successful probe advances to the next ramp step. A single failure at any step resets the breaker to Open with a 50% longer cooldown. This prevents a recovering system from being overwhelmed by a sudden flood of requests.

### Ramp as a Rack parameter

The ramp schedule and ramp factor are **Rack parameters** -- macro knobs that control micro behavior. They cascade down from the Delta loop (strategic timescale) to the circuit breaker Cell (gamma timescale). The Delta loop can adjust these based on historical recovery patterns:

```rust
/// The Delta loop adjusts ramp parameters based on recovery history.
impl DeltaLoopCell {
    fn adjust_ramp_params(&self, breaker: &mut CircuitBreakerCell) {
        // If recent recoveries have been successful (>80% probes pass),
        // increase the ramp factor to recover faster.
        let recovery_rate = breaker.recent_recovery_rate();
        if recovery_rate > 0.8 {
            breaker.config.ramp_factor = (breaker.config.ramp_factor * 1.1).min(4.0);
        }

        // If recent recoveries have failed (>50% probes fail),
        // decrease the ramp factor to recover more cautiously.
        if recovery_rate < 0.5 {
            breaker.config.ramp_factor = (breaker.config.ramp_factor * 0.8).max(1.2);
        }
    }
}
```

The operator can also override the ramp schedule via configuration:

```toml
[conductor.circuit_breaker]
max_plan_failures = 2
predictive = true
forecast_trip_threshold = 0.5

# Graduated half-open ramp
ramp_schedule = [0.05, 0.10, 0.20, 0.40, 0.80, 1.00]
ramp_factor = 2.0

# Error-specific cooldowns (milliseconds)
[conductor.circuit_breaker.error_cooldowns]
rate_limit = 5000
timeout = 10000
server_error = 30000
auth_failure = 300000
content_policy = 300000
```

---

## 6. AIMD Concurrency Control as a Route Cell

### The problem

When multiple agents run in parallel, they compete for finite LLM provider capacity. If too many agents hit the same provider simultaneously, rate limits trigger, which causes failures, which triggers circuit breakers, which kills plans unnecessarily. The system needs adaptive concurrency control.

### AIMD (Additive Increase, Multiplicative Decrease)

AIMD is the classic TCP congestion control algorithm. On success, the concurrency limit increases slowly (additive). On failure, it decreases quickly (multiplicative). This produces a sawtooth pattern that converges to the optimal concurrency level.

The AIMD controller is a **Route Cell**. It receives dispatch requests and either allows them (routes to the agent Cell) or queues them (routes to a wait Cell). The Route decision is based on the current concurrency limit.

```rust
/// AIMD concurrency controller as a Route Cell.
///
/// Controls how many simultaneous agent dispatches are allowed.
/// Adjusts the limit based on success/failure feedback.
pub struct AimdConcurrencyCell {
    /// Current concurrency limit (fractional for smooth adjustment).
    limit: f64,

    /// Currently active dispatches.
    active: u32,

    /// Additive increase: limit += 1/limit on success.
    increase_factor: f64,

    /// Multiplicative decrease: limit *= decrease_factor on failure.
    decrease_factor: f64,

    /// Floor: minimum concurrency limit.
    floor: f64,

    /// Ceiling: maximum concurrency limit.
    ceiling: f64,
}

impl Default for AimdConcurrencyCell {
    fn default() -> Self {
        Self {
            limit: 3.0,           // Start with 3 concurrent dispatches
            active: 0,
            increase_factor: 1.0, // limit += 1/limit on success
            decrease_factor: 0.9, // limit *= 0.9 on failure
            floor: 1.0,           // Never drop below 1
            ceiling: 10.0,        // Never exceed 10
        }
    }
}

impl Cell for AimdConcurrencyCell {
    fn id(&self) -> CellId {
        CellId::from_name_version("route.aimd_concurrency", "1.0.0")
    }

    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Route]
    }

    fn estimated_cost(&self) -> MicroCents {
        MicroCents(0) // Pure routing logic
    }
}

impl Route for AimdConcurrencyCell {
    fn route(&self, input: &[Signal], ctx: &CellContext) -> RouteDecision {
        // Determine the request type from the input Signal.
        let request = extract_dispatch_request(input);

        match request {
            DispatchRequest::New { agent_id } => {
                if (self.active as f64) < self.limit {
                    // Below concurrency limit: allow dispatch.
                    RouteDecision::single(Route::to("agent_dispatch"))
                } else {
                    // At limit: queue the request.
                    RouteDecision::single(Route::to("dispatch_queue"))
                }
            }
            DispatchRequest::Complete { success } => {
                // Agent finished. Update concurrency limit.
                self.record_outcome(success, ctx.bus());
                RouteDecision::noop()
            }
        }
    }
}

impl AimdConcurrencyCell {
    /// Record an outcome and adjust the concurrency limit.
    fn record_outcome(&mut self, success: bool, bus: &dyn Bus) {
        self.active = self.active.saturating_sub(1);

        if success {
            // Additive increase: limit += 1/limit.
            // This produces slow, linear growth.
            self.limit += self.increase_factor / self.limit;
            self.limit = self.limit.min(self.ceiling);
        } else {
            // Multiplicative decrease: limit *= decrease_factor.
            // This produces fast, exponential shrinkage.
            self.limit *= self.decrease_factor;
            self.limit = self.limit.max(self.floor);
        }

        // Publish the new limit as a Pulse for observability.
        bus.publish(Pulse {
            kind: PulseKind::ConcurrencyLimitChanged {
                new_limit: self.limit,
                active: self.active,
            },
            source: self.id(),
        });
    }

    /// Start tracking a new dispatch.
    fn record_dispatch(&mut self) {
        self.active += 1;
    }

    /// Current utilization as a fraction (active / limit).
    pub fn utilization(&self) -> f64 {
        self.active as f64 / self.limit
    }
}
```

### AIMD in the dispatch Graph

The AIMD Cell sits at the entry of the dispatch Graph, gating how many agents can be active simultaneously:

```
    Dispatch Request
          |
          v
    +------------------+
    | route.aimd       |----> Allow ---> agent_dispatch ---> Agent Cell
    +------------------+
          |
          +-------------> Queue ---> dispatch_queue (waits for capacity)
```

When an agent completes, the AIMD Cell receives a `DispatchRequest::Complete` signal and adjusts the limit. If the queue has pending requests and the limit allows, they are released.

---

## 7. Graduated Interventions

### The three intervention levels

When the conductor decides to intervene, the intervention is graduated. There are three levels, each more disruptive:

| Level | Decision | What happens | When |
|---|---|---|---|
| **Continue** | No intervention | Cognitive Pulses may still be emitted (Cooldown, Escalate). | Severity::Info |
| **Restart** | Restart the agent | Terminate agent process. Preserve error context (the Signals that caused the restart). Spawn new agent with the error context injected into its system prompt. Increment restart counter. | Severity::Warning |
| **Fail** | Abort the plan | Cancel all agent Flows. Set plan phase to Failed. Record failure in circuit breaker. Move to next plan/task. | Severity::Critical |

### Restart mechanics as a Cell operation

The Restart intervention is a composition of three Cell operations:

1. **Terminate**: Send `Shutdown` Pulse to the agent's Bus partition. The agent's Loop Graph exits.
2. **Preserve context**: Extract the last N Signals from the agent's Store partition. These become evidence for the restart.
3. **Spawn**: Start a new agent Flow with the evidence injected as initial input Signals. The new agent sees what went wrong.

```rust
/// The restart intervention as a React Cell.
pub struct RestartInterventionCell;

impl React for RestartInterventionCell {
    fn react(&self, input: &[Signal], ctx: &CellContext) -> Vec<Signal> {
        let restart_verdict = extract_restart_verdict(input);
        let agent_id = restart_verdict.agent_id();

        // 1. Terminate the existing agent.
        ctx.bus().publish(Pulse {
            kind: PulseKind::Shutdown {
                target: agent_id.clone(),
                reason: restart_verdict.reason.clone(),
            },
            source: self.id(),
        });

        // 2. Preserve error context.
        let error_context: Vec<Signal> = ctx.store().query(
            StoreQuery::by_agent(agent_id)
                .latest(20) // Last 20 signals
                .with_kind("conductor.intervention")
        );

        // Build a context injection Signal.
        let context_signal = Signal::new("restart_context")
            .with_body(RestartContext {
                reason: restart_verdict.reason.clone(),
                evidence: restart_verdict.evidence.clone(),
                attempt: restart_verdict.attempt_number(),
                prior_errors: error_context.iter()
                    .filter_map(|s| s.body.get("reason").map(|r| r.to_string()))
                    .collect(),
            })
            .with_tag("intervention", "restart")
            .with_tag("attempt", restart_verdict.attempt_number().to_string());

        // 3. Emit the context Signal. The orchestrator subscribes to
        //    this and spawns a new agent with the context injected.
        vec![context_signal]
    }
}
```

### Fail mechanics

The Fail intervention cancels all agent Flows for the plan and records the failure in the circuit breaker:

```rust
impl React for FailInterventionCell {
    fn react(&self, input: &[Signal], ctx: &CellContext) -> Vec<Signal> {
        let fail_verdict = extract_fail_verdict(input);

        // Cancel all agent Flows for this plan.
        ctx.bus().publish(Pulse {
            kind: PulseKind::CancelPlan {
                plan_id: fail_verdict.plan_id.clone(),
                reason: fail_verdict.reason.clone(),
            },
            source: self.id(),
        });

        // Record in circuit breaker (via Bus -- the breaker subscribes).
        ctx.bus().publish(Pulse {
            kind: PulseKind::PlanFailed {
                plan_id: fail_verdict.plan_id.clone(),
                reason: fail_verdict.reason.clone(),
            },
            source: self.id(),
        });

        // Emit terminal Signal.
        vec![Signal::plan_failed(&fail_verdict.plan_id, &fail_verdict.reason)]
    }
}
```

### Cooldown between interventions

To prevent oscillation (the conductor restarts an agent, which immediately triggers another restart), there is a cooldown period. The cooldown is implemented in the self-healing state tracked by the conductor's Gamma Loop. The specific rule: 120 seconds between interventions from the same watcher on the same plan.

```rust
/// Cooldown filter for intervention throttling.
/// Prevents the same watcher from triggering multiple interventions
/// on the same plan within the cooldown window.
pub struct CooldownFilter {
    /// (watcher_name, plan_id) -> last intervention timestamp (ms).
    last_intervention: HashMap<(String, String), i64>,
    /// Cooldown duration in milliseconds (default: 120_000).
    cooldown_ms: i64,
}

impl CooldownFilter {
    /// Check if an intervention is allowed (cooldown has expired).
    pub fn is_allowed(&self, watcher: &str, plan_id: &str, now_ms: i64) -> bool {
        let key = (watcher.to_string(), plan_id.to_string());
        self.last_intervention
            .get(&key)
            .map_or(true, |&last| now_ms - last >= self.cooldown_ms)
    }

    /// Record an intervention.
    pub fn record(&mut self, watcher: &str, plan_id: &str, now_ms: i64) {
        let key = (watcher.to_string(), plan_id.to_string());
        self.last_intervention.insert(key, now_ms);
    }
}
```

---

## 8. Self-Healing as Predict-Publish-Correct

The conductor itself can get stuck. A watcher that oscillates (fires, then does not fire, then fires, repeatedly) produces a stream of restarts that never converge. The self-healing system detects this and intervenes.

Self-healing follows the same predict-publish-correct pattern as the circuit breaker:

- **Predict**: The watcher should converge (stop oscillating) within N ticks.
- **Publish**: The actual observation arrives (watcher fired / did not fire).
- **Correct**: If the watcher is oscillating (alternating fire/no-fire for more than `max_oscillations` ticks), the self-healing system resets the watcher and puts it in cooldown.

```rust
/// Self-healing as a Verify Cell that monitors conductor health.
pub struct SelfHealingCell {
    policy: SelfHealingPolicy,
    state: SelfHealingState,
}

impl Verify for SelfHealingCell {
    fn verify(&self, input: &[Signal], _ctx: &CellContext) -> Verdict {
        // input contains the Verdicts from the 10 watcher Cells
        // across multiple recent ticks.
        let verdicts = extract_verdicts(input);

        // Check each watcher for oscillation.
        for (watcher_name, recent_verdicts) in group_by_watcher(&verdicts) {
            let firing_pattern: Vec<bool> = recent_verdicts.iter()
                .map(|v| !v.passed)
                .collect();

            // Count alternations.
            let oscillations = firing_pattern.windows(2)
                .filter(|w| w[0] != w[1])
                .count();

            if oscillations >= self.policy.max_oscillations as usize {
                return Verdict {
                    passed: false,
                    severity: Severity::Warning,
                    source: self.id(),
                    reason: format!(
                        "Watcher {watcher_name} oscillating ({oscillations} alternations)"
                    ),
                    evidence: recent_verdicts.into_iter().cloned().collect(),
                    remediation: Some(Remediation::Restart {
                        context: format!(
                            "Reset oscillating watcher {watcher_name} and cooldown for {} ticks",
                            self.policy.cooldown_ticks
                        ),
                    }),
                    metric: Some(oscillations as f64),
                };
            }
        }

        // Check for consecutive conductor failures (plans that failed
        // even after intervention).
        if self.state.consecutive_failures() >= self.policy.auto_restart_threshold {
            return Verdict {
                passed: false,
                severity: Severity::Critical,
                source: self.id(),
                reason: format!(
                    "Conductor has {} consecutive failures -- auto-restart recommended",
                    self.state.consecutive_failures()
                ),
                evidence: vec![],
                remediation: Some(Remediation::Abort {
                    reason: "Conductor auto-restart threshold reached".into(),
                }),
                metric: Some(self.state.consecutive_failures() as f64),
            };
        }

        Verdict::pass(self.id())
    }
}
```

---

## 9. Persistence and Snapshot/Resume

Circuit breaker state must survive process restarts. The state is written to Store by the Delta loop Cell and restored on startup:

```rust
/// Serializable circuit breaker state for persistence.
#[derive(Serialize, Deserialize)]
pub struct CircuitBreakerSnapshot {
    /// Plan ID.
    pub plan_id: String,
    /// Current state.
    pub state: BreakerState,
    /// Configuration (so the restored breaker uses the same thresholds).
    pub config: BreakerConfig,
    /// Forecaster state (level, trend, observation count).
    pub forecaster: HoltForecaster,
    /// Evaluation counts (failures, total).
    pub eval_count: (u32, u32),
}

impl CircuitBreakerCell {
    /// Capture a snapshot for persistence.
    pub fn snapshot(&self) -> CircuitBreakerSnapshot {
        CircuitBreakerSnapshot {
            plan_id: self.plan_id.clone(),
            state: self.state.clone(),
            config: self.config.clone(),
            forecaster: self.forecaster.clone(),
            eval_count: self.eval_count,
        }
    }

    /// Restore from a snapshot.
    pub fn from_snapshot(snap: CircuitBreakerSnapshot) -> Self {
        Self {
            plan_id: snap.plan_id,
            state: snap.state,
            config: snap.config,
            forecaster: snap.forecaster,
            eval_count: snap.eval_count,
        }
    }
}
```

The Delta loop writes snapshots for all active breakers:

```rust
// In the Delta loop Cell:
let snapshots: Vec<CircuitBreakerSnapshot> = breakers.iter()
    .map(|b| b.snapshot())
    .collect();

ctx.store().write(&Signal::from_body(&snapshots)
    .with_kind("circuit_breaker.snapshot"));
```

On startup, the Engine reads the snapshot from Store and restores the breaker Cells before starting the conductor Loop Graph.

---

## 10. Complete Architecture Diagram

```
                     Agent Signal Stream (from Store)
                               |
                               v
          +--------------------------------------------+
          |        Conductor Gamma Loop (5s tick)       |
          |                                             |
          |    Observe (Lens) ----> Verify Pipeline     |
          |                         |                   |
          |    +------ FanOut ------+------- ...        |
          |    |        |          |                    |
          |    v        v          v                    |
          |  ghost   compile    cost  ... (10 watchers) |
          |    |        |          |                    |
          |    +------ FanIn ------+------- ...        |
          |                |                            |
          |         pattern_detector                    |
          |                |                            |
          |         route.intervention                  |
          |                |                            |
          |    +--- Act (React) ---+                    |
          |    |                   |                    |
          |    | publish Pulses    | write to Store     |
          |    | on Bus            |                    |
          |    +------- feedback edge -------> Observe  |
          +--------------------------------------------+
                               |
                   ConductorDecision Pulse
                               |
              +----------------+----------------+
              |                |                |
              v                v                v
     circuit_breaker    route.aimd      orchestrator
      (per-plan)     (concurrency)    (subscribes to
                                       cognitive Pulses)
```

---

## What This Enables

1. **Unified failure handling**: Circuit breakers, graduated interventions, and AIMD concurrency are all Cell instances composed into Graphs. There is one failure-handling pattern (predict-publish-correct), not three separate mechanisms.

2. **Feature-level isolation**: Each capability (gates, research, enrichment, docs) has its own circuit breaker instance. A research API outage does not trip the compile gate breaker. The breaker configuration is per-instance, in TOML, not in code.

3. **Predictive cost savings**: The Holt forecaster detects rising error trends and trips the breaker before the budget is exhausted. In the current codebase, predictive mode exists (`CircuitBreaker::with_predictive`) but is not enabled by default. The redesign enables it as the default.

4. **Stable recovery**: Graduated half-open prevents the thundering herd problem. Instead of going from 0% to 100% traffic after cooldown, the system ramps 5% -> 10% -> 20% -> 40% -> 80% -> 100%. A single probe failure at any step resets to Open, preventing cascade failures during recovery.

5. **Observable concurrency**: The AIMD Cell publishes concurrency limit changes as Pulses. The TUI can display the sawtooth curve. The Delta loop can correlate concurrency drops with specific error patterns.

6. **Replay and audit**: Because circuit breaker state transitions are Signals in Store, the entire breaker history is queryable and replayable. `roko replay` can walk through every trip, recovery, and ramp step.

---

## Feedback Loops

- **Circuit breaker -> Conductor**: When a breaker trips, it publishes a `CircuitBreakerTripped` Pulse. The conductor's Act Cell receives this and emits a `Shutdown` cognitive signal for the affected plan.
- **Conductor -> Circuit breaker**: When the conductor's Route Cell decides `Fail`, the Act Cell publishes a `PlanFailed` Pulse. The circuit breaker Cell subscribes and records the failure.
- **AIMD -> Conductor**: When the AIMD Cell reduces the concurrency limit, fewer agents run in parallel. This reduces the pressure that the conductor's watchers observe (fewer context-pressure and cost-overrun warnings). The conductor's Yerkes-Dodson model may shift toward under-stimulation, triggering an `Explore` signal to increase throughput.
- **Conductor -> AIMD**: When the conductor emits a `Cooldown` cognitive signal, the AIMD Cell can proactively reduce its limit (multiplicative decrease without waiting for a failure).
- **Forecaster correction**: Every outcome (success or failure) updates the Holt forecaster via predict-publish-correct. The forecaster's level and trend converge toward the true error rate over time. Alpha=0.3 and beta=0.1 balance responsiveness with stability.
- **Ramp learning**: The Delta loop adjusts the ramp factor based on historical recovery rates. Systems that recover quickly get a higher ramp factor (faster recovery). Systems that frequently fail probes get a lower ramp factor (more cautious recovery).

---

## Open Questions

1. **Breaker state sharing across plans**: Currently each plan has its own circuit breaker. If two plans use the same LLM provider and one plan's breaker trips due to rate limiting, should the other plan's breaker be notified? This would require a provider-level breaker in addition to the plan-level one, adding a second dimension of breaker instances.

2. **AIMD per-provider vs. global**: The current AIMD Cell controls total concurrency. Should there be per-provider AIMD Cells? A rate limit from Claude should not reduce concurrency for agents using Ollama. But per-provider AIMD adds N more Route Cells to the Graph.

3. **Holt forecaster warm-up bias**: The first observation sets the level with zero trend. If the first observation is a failure (1.0), the level starts high and the forecaster may proactively trip before enough data exists. Should there be a minimum observation count before predictive tripping is allowed? The current code uses 2 observations; is this enough?

4. **Cooldown escalation**: The current design uses a flat 50% cooldown increase on repeated failures. Should cooldown grow exponentially (doubling each time)? Exponential backoff converges faster in theory but can leave a recovering system idle for too long if the cooldown grows to minutes.

5. **Half-open probe selection**: When the breaker is in HalfOpen, which requests should be used as probes? Currently any request can be a probe. Should probes preferentially be low-cost or low-risk tasks, to minimize the cost of probe failures?
