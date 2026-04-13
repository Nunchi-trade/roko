# Circuit Breaker

> A plan can fail a maximum of two times. After that, it requires
> human attention. This is not configurable. This is law.


> **Implementation**: Built

---

## The Problem It Solves

Without a circuit breaker, a fundamentally broken plan enters an
infinite retry loop:

```
Plan fails → orchestrator retries → plan fails the same way →
orchestrator retries → plan fails again → orchestrator retries → ...
```

Each retry costs tokens. Each retry burns wall-clock time that could
be spent on plans that might succeed. Each retry produces the same
failure output, adding noise to the signal stream without adding
information.

This was Issue #7 from production (circuit breaker for repeated
failures): "A plan fails, gets retried, fails the same way, gets
retried again, fails again. Infinite retry loop burning tokens."

The circuit breaker enforces a hard budget: two failures per plan.
After that, the plan is marked as requiring human intervention and
is never automatically retried.

---

## Implementation

The circuit breaker lives in `crates/roko-conductor/src/circuit_breaker.rs`.

```rust
use dashmap::DashMap;

pub const MAX_PLAN_FAILURES: u32 = 2;

pub struct CircuitBreaker {
    failures: DashMap<String, FailureRecord>,
}

struct FailureRecord {
    count: u32,
    // Additional metadata: timestamps, failure reasons, etc.
}
```

### Thread Safety

The `DashMap` provides lock-free concurrent reads and sharded writes.
This matters because the orchestrator may evaluate multiple plans in
parallel — each plan's conductor check should not block on other plans'
failure records.

`DashMap` is a concurrent hash map that shards its data across multiple
locks. Two plans with different IDs will almost always hit different
shards, enabling true parallel access. This is preferable to a
`Mutex<HashMap>` which would serialize all failure record access.

### API

```rust
impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            failures: DashMap::new(),
        }
    }

    /// Record a failure for a plan. Returns true if the plan is now tripped.
    pub fn record_failure(&self, plan_id: &str) -> bool {
        let mut entry = self.failures.entry(plan_id.to_string()).or_insert(FailureRecord { count: 0 });
        entry.count += 1;
        entry.count >= MAX_PLAN_FAILURES
    }

    /// Check if a plan has exceeded its failure budget.
    pub fn is_tripped(&self, plan_id: &str) -> bool {
        self.failures
            .get(plan_id)
            .map(|record| record.count >= MAX_PLAN_FAILURES)
            .unwrap_or(false)
    }

    /// Reset failure count for a plan (e.g., after manual intervention).
    pub fn reset(&self, plan_id: &str) {
        self.failures.remove(plan_id);
    }
}
```

---

## Three-State Model

The circuit breaker implements a classic three-state pattern, though
the implementation in roko-conductor uses a simplified two-state model
(tripped / not tripped). The full three-state model, implemented in
the provider health tracker (`roko-learn/src/provider_health.rs`),
provides additional granularity:

### State Transitions

```
Closed (Healthy)
  │
  │ consecutive failures >= threshold
  ▼
Open (Tripped)
  │
  │ cooldown period expires
  ▼
HalfOpen (Probing)
  │
  ├─ probe succeeds → Closed
  │
  └─ probe fails → Open (reset cooldown)
```

**Closed**: Normal operation. Failures are counted but requests proceed.
This is the initial state for every plan.

**Open**: All requests are blocked. The plan has exceeded its failure
budget. No automatic retry is permitted. In the conductor's simplified
model, this is the terminal state (tripped). In the provider health
model, the system waits for a cooldown period before transitioning to
HalfOpen.

**HalfOpen**: One probe request is permitted. If the probe succeeds,
the breaker returns to Closed. If the probe fails, the breaker returns
to Open with a fresh cooldown. This state exists in the provider health
tracker but not in the conductor's plan-level breaker — because plans
do not benefit from automatic probing (a plan that failed twice needs
a different approach, not another attempt at the same approach).

### Error-Type-Specific Cooldowns

The provider health tracker uses error classification to set cooldown
durations:

| Error Class | Cooldown | Rationale |
|------------|----------|-----------|
| RateLimit | 5 seconds | Transient; provider will accept again soon |
| Timeout | 10 seconds | Might indicate temporary load |
| ServerError | 30 seconds | Likely operational issue, needs more time |
| AuthFailure | 5 minutes | Likely persistent; manual fix needed |
| ContentPolicy | 5 minutes | Likely persistent |
| ContextOverflow | N/A | Not retryable; needs model switch |

This error-type-specific behavior lives in the provider health layer
(`roko-learn`), not in the conductor's plan-level breaker. The
conductor's plan-level breaker is simpler: two failures of any kind,
then trip.

---

## Integration with the Conductor

The circuit breaker is checked at the start of every `evaluate()` call:

```rust
impl Conductor {
    pub fn evaluate(&self, plan_id: &str, stream: &[Signal], ctx: &Context) -> ConductorDecision {
        // 1. Check circuit breaker FIRST
        if self.circuit_breaker.is_tripped(plan_id) {
            return ConductorDecision::Fail {
                reason: format!("plan {plan_id} tripped circuit breaker after {} failures", MAX_PLAN_FAILURES),
            };
        }

        // 2. Run watchers
        let watcher_outputs = self.check_all(stream, ctx);

        // 3. Apply intervention policy
        let decision = self.policy.evaluate(&watcher_outputs, ctx);

        // 4. Record failures
        if matches!(decision, ConductorDecision::Fail { .. }) {
            self.circuit_breaker.record_failure(plan_id);
        }

        decision
    }
}
```

The circuit breaker check happens before watcher evaluation. If a plan
is already tripped, there is no point running watchers — the decision
is predetermined. This short-circuit saves watcher evaluation time for
plans that are already done.

---

## Why Two Failures

The `MAX_PLAN_FAILURES = 2` constant is derived from production data:

**First failure**: Often caused by transient issues — API rate limit,
cold start, missing context. Retrying with a fresh agent and potentially
different context frequently succeeds.

**Second failure**: The same plan failing twice usually indicates a
structural problem — the task is beyond the agent's capability with
the given context, the acceptance criteria are contradictory, or the
codebase has changed in a way that makes the task impossible as
specified.

**Third failure (never reached)**: At this point, the probability of
success is negligible. The two previous attempts have already tried
the obvious approaches. A third attempt would likely repeat one of
the first two, producing the same failure at the cost of more tokens.

The math: if each attempt has a 30% success rate (typical for complex
plans that fail the first time), the probability of failing twice is
(0.7)² = 49%. The probability of failing three times is (0.7)³ = 34%.
But this assumes independence — in practice, the second failure is
correlated with the first (same root cause), so the conditional
probability of a third failure given two failures is much higher
than 70%. The expected cost of a third attempt almost always exceeds
its expected value.

---

## Relationship to Hard Guarantees

The circuit breaker implements two hard guarantees from the failure
prevention catalog:

### Hard Guarantee 3: Hard Iteration Cap

Each plan attempt includes up to 3 implementation iterations (implement
→ gate fail → retry). With 2 plan-level failures, the total maximum
is:

```
2 plan attempts × 3 iterations each = 6 total implementation cycles
```

After 6 cycles, the plan is permanently failed. This is the absolute
upper bound on token spend for any single plan.

### Hard Guarantee 7: Circuit Breaker

Direct implementation. The plan can fail a maximum of 2 times. After
2 failures, it is permanently marked as requiring human intervention
and never automatically retried.

```
MAX_PLAN_FAILURES (2) × MAX_ITERATION_LOOP (3) = 6 max attempts ever
```

This prevents:
- Infinite retry loops (max 2 failures, then stop)
- Token burn on doomed plans (6 attempts max, ever)
- Silent stuck plans (tripped state is surfaced prominently)

---

## Per-Plan Isolation

The circuit breaker is keyed by plan ID. This means:

- Plan A hitting its failure budget does not affect Plan B
- Resetting Plan A does not reset Plan B
- The breaker can track hundreds of plans concurrently

This per-plan isolation is critical for batch runs where 20+ plans
execute in parallel. A single broken plan should not cascade to
affect healthy plans.

---

## Manual Reset

The `reset()` method exists for operator override. When a human
examines a failed plan, determines the root cause, applies a fix
(updated context, different model, modified acceptance criteria), they
can reset the circuit breaker to allow the plan to retry.

This is deliberately a manual operation. The system does not auto-reset
breakers because the whole point of the breaker is to prevent automatic
retry of plans that need human judgment. If auto-reset were possible,
the breaker would be bypassed on every failure.

---

## Persistence

The circuit breaker state is part of the executor snapshot. When the
orchestrator checkpoints to `.roko/state/executor.json`, failure records
are included. On resume, the circuit breaker is restored from the
snapshot, preserving failure counts across restarts.

This prevents a circumvention where restarting the orchestrator would
reset all breakers, allowing previously-failed plans to retry. The
breaker survives crashes.

---

## Future: Adaptive Failure Budget

The current `MAX_PLAN_FAILURES = 2` is a constant. A future enhancement
is adaptive failure budgets based on plan complexity:

| Complexity | Failure Budget | Rationale |
|-----------|---------------|-----------|
| Trivial | 1 | If a trivial task fails once, something is fundamentally wrong |
| Simple | 2 | Standard budget |
| Standard | 2 | Standard budget |
| Complex | 3 | Complex tasks have higher variance; third attempt with different strategy may succeed |

This would require wiring the plan's complexity classification (from
the task TOML frontmatter) into the circuit breaker's failure threshold.
The infrastructure exists — the cascade router already uses complexity
classification for model selection.

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/circuit_breaker.rs` | CircuitBreaker struct, DashMap-based tracking |
| `crates/roko-conductor/src/conductor.rs` | Integration point — breaker checked in evaluate() |
| `crates/roko-learn/src/provider_health.rs` | Extended 3-state model for provider health |
| `crates/roko-core/src/agent.rs` | ConductorDecision enum consumed by orchestrator |

---

## Predictive Circuit Breaking

The current breaker is reactive: it counts failures after they happen.
A predictive breaker trips the circuit *before* failures cascade, based
on the trajectory of leading indicators.

### Gradient-Based Trip

Trip when the failure rate derivative exceeds a threshold. Instead of
waiting for N failures, detect the trajectory toward failure and
preempt it.

```rust
/// Predictive circuit breaker extension that trips based on failure rate trends.
/// Instead of waiting for N failures, detect the trajectory toward failure.
pub struct PredictiveBreaker {
    /// EWMA-smoothed error rate per plan.
    error_rate_ewma: DashMap<String, EwmaState>,
    /// Slope threshold: trip when d(error_rate)/dt exceeds this value.
    /// Default: 0.05 (5% per evaluation cycle).
    slope_threshold: f64,
    /// Minimum observations before predictive logic activates.
    /// Prevents false trips on insufficient data.
    min_observations: usize,
    /// Lookahead window: how far ahead to project the error rate.
    lookahead_cycles: usize,
}

impl PredictiveBreaker {
    /// Returns true if the projected error rate exceeds the trip threshold.
    pub fn should_preempt(&self, plan_id: &str) -> bool {
        if let Some(ewma) = self.error_rate_ewma.get(plan_id) {
            if ewma.observations < self.min_observations { return false; }
            let slope = ewma.derivative();
            let projected = ewma.mean + slope * self.lookahead_cycles as f64;
            projected > 0.60 && slope > self.slope_threshold
        } else {
            false
        }
    }
}
```

The `should_preempt` check runs alongside `is_tripped` in the
conductor's `evaluate()` path. If the projected error rate exceeds 60%
and the slope exceeds 5% per cycle, the breaker trips preemptively.
The `min_observations` guard prevents false trips on plans that have
only run a handful of times — the slope estimate is unreliable with
fewer than ~10 data points.

### Leading Indicators

These signals predict failures before they occur. Each maps to an
existing watcher or metric in roko-conductor:

| Indicator | What it measures | Trip condition |
|-----------|-----------------|----------------|
| Latency percentile creep | p99/p50 ratio rising | Ratio > 5x and increasing |
| Retry rate acceleration | Retries/turn increasing | d(retry_rate)/dt > 0.1 |
| Context growth rate | Tokens/turn increasing | Growth rate > 10% per turn |
| TTFT degradation | Time-to-first-token rising | TTFT > 3x baseline |
| Quality score decline | Gate scores trending down | Holt-Winters forecast < threshold |

These indicators correlate with imminent failure because they reveal
resource exhaustion and quality degradation before the final failure
event. A plan whose context grows 10% per turn will hit the context
window limit within a few turns — the breaker can trip before the
overflow, saving the wasted tokens of a doomed turn.

### Time-Series Forecasting

Holt's method (double exponential smoothing) extends the existing EWMA
with a trend component. Where EWMA tracks level only, Holt's method
tracks level and slope, enabling forward projection.

```rust
/// Holt's double exponential smoothing for trend-aware forecasting.
/// Extends the existing EWMA with a trend component.
pub struct HoltForecaster {
    /// Level component (smoothed value).
    level: f64,
    /// Trend component (smoothed rate of change).
    trend: f64,
    /// Level smoothing factor (default: 0.3).
    alpha: f64,
    /// Trend smoothing factor (default: 0.1).
    beta: f64,
    /// Number of observations seen.
    observations: usize,
}

impl HoltForecaster {
    pub fn update(&mut self, value: f64) {
        if self.observations == 0 {
            self.level = value;
            self.trend = 0.0;
        } else {
            let prev_level = self.level;
            self.level = self.alpha * value + (1.0 - self.alpha) * (prev_level + self.trend);
            self.trend = self.beta * (self.level - prev_level) + (1.0 - self.beta) * self.trend;
        }
        self.observations += 1;
    }

    /// Forecast h steps ahead.
    pub fn forecast(&self, h: usize) -> f64 {
        self.level + self.trend * h as f64
    }
}
```

The `alpha` parameter controls how quickly the level responds to new
observations; `beta` controls how quickly the trend responds. Lower
values produce smoother estimates that resist noise but lag behind
real changes. The defaults (0.3 / 0.1) bias toward smooth trend
estimation — appropriate for circuit breaker decisions where false
trips are more costly than late trips.

---

## Partial Circuit Breaking — Graceful Degradation

A full circuit trip halts all work on a plan. Partial circuit breaking
degrades individual capabilities while keeping core execution running.
This is the difference between "stop everything" and "stop the parts
that are failing."

### Feature-Level Breakers

Each plan capability has its own circuit. When context enrichment fails
three times, the enrichment circuit opens — but compilation, testing,
and implementation continue uninterrupted.

```rust
/// Feature-level circuit breaker: break individual plan capabilities
/// while the core execution continues.
pub struct FeatureBreaker {
    /// Per-feature failure tracking.
    features: DashMap<String, FeatureCircuit>,
}

pub struct FeatureCircuit {
    pub feature: PlanFeature,
    pub state: CircuitState,
    pub failures: u32,
    pub max_failures: u32,
    /// Fallback behavior when this feature is broken.
    pub fallback: FeatureFallback,
}

/// Plan capabilities that can be independently circuit-broken.
pub enum PlanFeature {
    GateRung(String),      // Individual gate rung (clippy, coverage, etc.)
    ContextEnrichment,     // Adding related code context
    ResearchEnhancement,   // Research-based task enrichment
    DocUpdate,             // Documentation generation
    ReviewCycle,           // Code review by reviewer agent
}

/// Fallback behavior when a feature circuit opens.
pub enum FeatureFallback {
    Skip,                  // Omit this feature entirely
    UseCached,             // Use last successful result
    Downgrade(String),     // Use simpler version (e.g., Opus -> Haiku reviewer)
    WarnAndContinue,       // Log warning, proceed without feature
}
```

The degradation hierarchy defines how each feature fails gracefully:

| Feature | Fallback 1 | Fallback 2 | Fallback 3 |
|---------|-----------|-----------|-----------|
| Clippy gate | WarnAndContinue | Skip | Skip |
| Context enrichment | UseCached | Downgrade (minimal context) | Skip |
| Research enhancement | UseCached | Skip | Skip |
| Review cycle | Downgrade (Haiku reviewer) | WarnAndContinue | Skip |
| Compile gate | (no fallback — always required) | — | — |
| Test gate | (no fallback — always required) | — | — |

Compile and test gates have no fallback because they enforce
correctness. A plan that does not compile is not a plan. Everything
else — linting, enrichment, review — is valuable but not essential.
The feature breaker encodes this distinction: some capabilities are
negotiable, some are not.

### Graduated Probe Traffic (Half-Open Enhancement)

The standard half-open state is binary: one probe request, pass or
fail. Graduated probing ramps traffic from 5% to 100%, reducing the
risk that a single lucky probe declares the breaker healthy when the
underlying problem persists.

```rust
/// Enhanced half-open state with graduated probe traffic.
/// Instead of binary open/closed, ramp traffic from 5% to 100%.
pub struct GraduatedHalfOpen {
    /// Current probe fraction (0.0 = fully open, 1.0 = fully closed).
    probe_fraction: f64,
    /// Multiplier on each successful probe (default: 2.0).
    ramp_factor: f64,
    /// Initial probe fraction when entering half-open (default: 0.05).
    initial_fraction: f64,
    /// Sleep window before first probe (default: 300s).
    base_sleep_ms: u64,
    /// Maximum sleep window after repeated failures (default: 1800s = 30 min).
    max_sleep_ms: u64,
    /// Current sleep window (doubles on each re-open).
    current_sleep_ms: u64,
}

impl GraduatedHalfOpen {
    pub fn on_probe_success(&mut self) {
        self.probe_fraction = (self.probe_fraction * self.ramp_factor).min(1.0);
    }

    pub fn on_probe_failure(&mut self) {
        self.probe_fraction = self.initial_fraction;
        self.current_sleep_ms = (self.current_sleep_ms * 2).min(self.max_sleep_ms);
    }

    pub fn is_fully_recovered(&self) -> bool {
        self.probe_fraction >= 1.0
    }
}
```

The ramp sequence with default settings: 5% -> 10% -> 20% -> 40% ->
80% -> 100%. Each step requires a successful probe at the current
traffic level. A failure at any step resets to 5% and doubles the
sleep window (capped at 30 minutes). This exponential backoff on the
sleep window prevents rapid re-probing of a persistently broken
dependency.

### Load Shedding Under Pressure

When the cost budget is tight, the system sheds low-priority work
first. This mirrors how production systems handle overload: degrade
gracefully rather than fail completely.

The shedding tiers:

- When cost > 70% budget: defer doc-update and enrichment tasks
- When cost > 85% budget: defer all non-critical-path tasks
- When cost > 95% budget: only execute core implementation + required gates

```rust
/// Load shedding policy: which tasks to defer under budget pressure.
pub struct LoadSheddingPolicy {
    /// Budget utilization thresholds for each shedding tier.
    tiers: Vec<SheddingTier>,
}

pub struct SheddingTier {
    /// Budget utilization threshold (0.0 to 1.0) to activate this tier.
    pub threshold: f64,
    /// Task priorities that are shed at this tier (lower = shed first).
    pub shed_below_priority: TaskPriority,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Optional = 0,      // Doc updates, style fixes
    Enhancement = 1,   // Enrichment, research
    Standard = 2,      // Review cycles, optional gates
    Required = 3,      // Core implementation
    Critical = 4,      // Required gates (compile, test)
}
```

The priority ordering matters. `TaskPriority` derives `Ord`, so the
shedding policy can compare priorities directly: at threshold 0.70,
shed everything below `Standard`; at 0.85, shed everything below
`Required`; at 0.95, shed everything below `Critical`. The compile
and test gates survive all shedding tiers — they are never deferred.

### Adaptive Concurrency (AIMD)

Fixed concurrency limits are either too conservative (wasting
parallelism) or too aggressive (overloading the provider). AIMD
(Additive Increase, Multiplicative Decrease) self-tunes to the
optimal concurrency, using the same algorithm as TCP congestion
control.

- On success: `concurrency += 1 / concurrency` (additive increase)
- On failure: `concurrency *= 0.9` (multiplicative decrease)

```rust
/// AIMD-based adaptive concurrency limiter.
/// Self-tunes to optimal concurrency without fixed configuration.
pub struct AdaptiveConcurrency {
    /// Current concurrency limit (float for smooth adjustment).
    limit: f64,
    /// Minimum concurrency (default: 1.0).
    floor: f64,
    /// Maximum concurrency (default: 10.0).
    ceiling: f64,
    /// Multiplicative decrease factor on failure (default: 0.9).
    decrease_factor: f64,
}

impl AdaptiveConcurrency {
    pub fn on_success(&mut self) {
        self.limit = (self.limit + 1.0 / self.limit).min(self.ceiling);
    }
    pub fn on_failure(&mut self) {
        self.limit = (self.limit * self.decrease_factor).max(self.floor);
    }
    pub fn current_limit(&self) -> usize {
        self.limit.ceil() as usize
    }
}
```

The additive increase is inversely proportional to the current limit.
At concurrency 2, each success adds 0.5. At concurrency 8, each
success adds 0.125. This produces slow, cautious growth at high
concurrency — exactly the behavior you want when approaching the
provider's rate limit. The multiplicative decrease (10% reduction per
failure) drops the limit fast enough to relieve pressure without
collapsing to 1.

---

## Chaos Engineering for Circuit Breaker Validation

Circuit breakers that are never tested in failure conditions are
circuit breakers that might not work when they matter. Chaos
engineering validates the breaker by injecting controlled failures
and observing whether the system responds correctly.

### Chaos Experiment Types

Each experiment type maps from established chaos engineering practice
(Netflix Simian Army, Gremlin) to the agent orchestration domain:

| Chaos type | Agent equivalent | Tests |
|-----------|-----------------|-------|
| Process kill | Kill agent mid-task | ProcessSupervisor, ghost-turn watcher |
| Latency injection | Artificial API delay | TimeOverrunWatcher, circuit breaker |
| Error injection | Force compile errors | CompileFailRepeatWatcher, stuck detector |
| Resource saturation | Fill context window | ContextWindowPressureWatcher |
| Cost spike | Inject expensive turns | CostOverrunWatcher, anomaly detector |
| Rate limit | Throttle API calls | Cascade router fallback |

### Steady-State Hypothesis

Before injecting chaos, define what "normal" looks like. The
experiment succeeds if the system returns to this steady state after
the injection ends.

```
gate_pass_rate > 0.8 over rolling 10-run window
agent_cost_per_task < $0.50
p95_task_completion_time < 300 seconds
zero plans in CIRCUIT_TRIPPED state
```

If the system cannot return to these baselines after chaos injection,
the circuit breaker (or one of its supporting watchers) has a gap.

### Principles

Three rules for chaos experiments in this system:

1. **Start small.** Run against one plan in isolation. Never inject
   chaos into a full batch run until single-plan experiments pass.
2. **Minimize blast radius.** Use synthetic plans with throwaway
   tasks. Never inject failures into plans that produce real code
   changes.
3. **Run in synthetic plans first.** Build a suite of canary plans
   whose sole purpose is chaos testing. These plans have known-good
   tasks (for baseline) and intentionally flawed tasks (for failure
   injection). Run them on every release before promoting to
   production orchestration.

### References

- Nygard (2007) — *Release It!*, circuit breaker pattern
- Netflix Hystrix — rolling window metrics, health calculation
- Resilience4j — sliding window, slow-call detection
- Netflix Principles of Chaos Engineering (2018)
- Patterson et al. (2002) — Recovery-Oriented Computing
- Candea & Fox (2003) — Micro-reboots
