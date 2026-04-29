# Provider Health and Pareto Pruning

> Depth for [07-LEARNING.md](../../unified/07-LEARNING.md). Provider health monitoring as a circuit breaker Loop (closed -> half-open -> open states), Pareto frontier pruning of dominated model-provider combinations, and anomaly detection as a React Cell subscribing to outcome Pulses -- all adjusting provider availability for the cascade router.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, Bus), [02-CELL](../../unified/02-CELL.md) (React protocol, Verify protocol, Route protocol), [04-EXECUTION](../../unified/04-EXECUTION.md) (Loop specialization), [07-LEARNING](../../unified/07-LEARNING.md) (L1 Parameter Tuning, L2 Strategy Routing)

**Source docs**: `docs/05-learning/09-provider-health-circuit-breaker.md`, `docs/05-learning/10-pareto-frontier-pruning.md`

---

## 1. Provider Health as a React Cell Loop

Provider health monitoring is a **React protocol Cell** that subscribes to outcome Pulses on the Bus and adjusts provider availability. It implements cybernetic feedback loop 1 (Health -> Routing): provider errors cause the circuit breaker to open, diverting traffic to healthy alternatives, until recovery is confirmed.

```toml
[graph]
name = "provider-health-loop"
loop = true

[[nodes]]
id = "outcome-subscriber"
cell = "roko:outcome-watcher"
protocol = "React"

[[nodes]]
id = "circuit-breaker"
cell = "roko:circuit-breaker"
protocol = "Verify"

[[nodes]]
id = "availability-publisher"
cell = "roko:provider-availability"
protocol = "React"

[[edges]]
from = "outcome-subscriber"
to = "circuit-breaker"

[[edges]]
from = "circuit-breaker"
to = "availability-publisher"

# Feedback: availability changes affect future routing,
# which changes the outcome distribution,
# which the outcome-subscriber observes
[[edges]]
from = "availability-publisher"
to = "outcome-subscriber"
condition = "provider_state_changed"
```

---

## 2. Three-State Circuit Breaker

The circuit breaker is a Verify Cell with three states. Each state has a distinct behavior and a declared transition condition:

```
                 success
    +------------------------------+
    |                              |
    v                              |
+--------+   failure >= threshold   +------------+
| CLOSED | ----------------------->|    OPEN     |
|(normal)|                         |(no traffic) |
+--------+                         +------+------+
    ^                                     |
    |                          cooldown expires
    |         success                     |
    |  +------------------+               |
    +--+    HALF-OPEN     |<--------------+
       |(single probe req)|
       +------------------+
              |
              | failure
              v
          OPEN (reset cooldown, escalate backoff)
```

### State Semantics

| State | Behavior | Transition Out |
|---|---|---|
| **Closed** | Normal. All requests routed. Failures counted. | -> Open: failure count exceeds threshold in window |
| **Open** | No requests routed. Traffic diverted. | -> Half-Open: after cooldown period expires |
| **Half-Open** | Single probe request allowed. | -> Closed: probe succeeds. -> Open: probe fails |

This is a Verify Cell in the sense of [02-CELL.md](../../unified/02-CELL.md): it checks a pre-condition (is the provider healthy?) and emits a Verdict (available or unavailable). The Verdict is conjunctive: the provider must pass the circuit breaker check AND not be in an error cooldown.

### Per-Provider State

```rust
/// React Cell: per-provider health tracking.
///
/// Subscribes to: Bus Pulses on "outcome.{provider}.*"
/// Publishes to:  Bus Pulse on "provider.health.{provider}"
pub struct ProviderHealth {
    pub provider_id: String,
    pub state: CircuitState,
    pub recent_failures: VecDeque<FailureRecord>,
    pub failure_count: u64,
    pub success_count: u64,
    pub last_opened: Option<DateTime<Utc>>,
    pub cooldown_until: Option<DateTime<Utc>>,
}

pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}
```

---

## 3. Error Classification

Not all errors indicate the same problem. The React Cell classifies errors to apply appropriate cooldown policies:

```rust
pub enum ErrorClass {
    RateLimit,       // HTTP 429
    AuthFailure,     // HTTP 401/403
    Timeout,         // request/response timeout
    ServerError,     // HTTP 5xx
    ContentPolicy,   // filtered response
    ContextOverflow, // context window exceeded
    Unknown,         // unclassified
}
```

### Error-Specific Cooldowns

| Error Class | Cooldown | Rationale |
|---|---|---|
| RateLimit | 60s (escalating) | Rate window resets |
| AuthFailure | 300s | Requires manual API key rotation |
| Timeout | 30s | Transient network issue |
| ServerError | 120s | Variable recovery |
| ContentPolicy | 0s (flag only) | Not a provider health issue |
| ContextOverflow | 0s (route to larger model) | Task-specific, not provider |
| Unknown | 60s | Conservative default |

ContentPolicy and ContextOverflow do not trigger the circuit breaker because they are not provider health problems. They are routed to the cascade router as context (ContextOverflow -> escalate to larger context window model).

---

## 4. Exponential Backoff

When a half-open probe fails, the cooldown escalates exponentially:

```
cooldown(n) = base_cooldown * 2^(n-1)
```

Where `n` is the number of consecutive open -> half-open -> open cycles. Capped at 480 seconds (8 minutes).

| Cycle | RateLimit | ServerError |
|---|---|---|
| 1 | 60s | 120s |
| 2 | 120s | 240s |
| 3 | 240s | 480s |
| 4+ | 480s (max) | 480s (max) |

This prevents the system from hammering a persistently failing provider with probe requests.

---

## 5. Integration with Cascade Router

The ProviderHealthRegistry feeds the Route Cell pipeline:

```
CascadeRouter::select(context)
    |
    +-- 1. Compute candidate scores (per-stage algorithm)
    |
    +-- 2. Filter: ProviderHealthRegistry::is_available(model.provider)
    |       Remove models whose provider circuit breaker is Open
    |
    +-- 3. Filter: Pareto frontier pruning
    |       Remove dominated models (section 7)
    |
    +-- 4. Select highest-scoring non-filtered model
```

If all providers for a desired model tier are unavailable, the router escalates to the next tier or returns the pre-computed fallback from the `CascadeModel`.

The `is_available()` check returns `true` for Closed and Half-Open states. Half-Open allows exactly one probe request to test recovery.

---

## 6. Anomaly Detection as a React Cell

Beyond the circuit breaker, the `AnomalyDetector` React Cell provides secondary health checks:

```rust
/// React Cell: detect anomalous patterns in the outcome stream.
///
/// Subscribes to: Bus Pulses on "outcome.*"
/// Publishes to:  Bus Pulse on "anomaly.detected"
pub struct AnomalyDetector {
    prompt_hash_window: VecDeque<u64>,  // last 20 prompt hashes
    cost_ewma: EwmaState,              // EWMA cost baseline
    quality_history: VecDeque<f64>,    // rolling quality scores
    session_cost_usd: f64,
}
```

Three anomaly types:

| Anomaly | Detection | Threshold |
|---|---|---|
| **Prompt loop** | Same prompt hash 5+ times in last 20 | `PROMPT_LOOP_THRESHOLD = 5` |
| **Cost spike** | Z-score > 3.0 against EWMA baseline | `COST_SPIKE_Z_THRESHOLD = 3.0` |
| **Quality degradation** | Last 5 scores avg < 0.5 AND drop > 0.15 vs prior 10 | Composite |

### EWMA Cost Baseline

```
ewma_new = alpha * observation + (1 - alpha) * ewma_old
z_score = (observation - ewma) / ewma_stddev
```

Where alpha = 0.2. The observation is compared against the EWMA *before* updating, keeping sudden spikes visible.

Prompt loop detection catches a common failure mode: the agent retries the same action repeatedly, consuming tokens without progress. The React Cell emits a Pulse that the orchestrator can use to abort and replan.

---

## 7. Pareto Frontier Pruning

Pareto pruning is a pre-filtering step that removes dominated models before presenting candidates to the bandit in the cascade router. It operates as a Score Cell that evaluates each model on two objectives simultaneously.

### Dominance Definition

Model A dominates model B when:
- A has pass_rate >= B's pass_rate, AND
- A has cost_per_success <= B's cost_per_success, AND
- At least one inequality is strict.

```rust
/// Score Cell: compute Pareto frontier over model observations.
///
/// Recomputed every PARETO_RECOMPUTE_INTERVAL = 50 observations.
/// O(n^2) where n = number of models. Negligible for typical 3-10 models.
pub fn compute_pareto_frontier(
    stats: &HashMap<String, ModelObservation>,
) -> Vec<String> {
    let mut frontier = Vec::new();
    for (slug_a, obs_a) in stats {
        let dominated = stats.iter().any(|(slug_b, obs_b)| {
            slug_b != slug_a
                && obs_b.pass_rate >= obs_a.pass_rate
                && obs_b.cost_per_success <= obs_a.cost_per_success
                && (obs_b.pass_rate > obs_a.pass_rate
                    || obs_b.cost_per_success < obs_a.cost_per_success)
        });
        if !dominated {
            frontier.push(slug_a.clone());
        }
    }
    frontier.sort();
    frontier
}

pub struct ModelObservation {
    pub pass_rate: f64,
    pub cost_per_success: f64,
    pub avg_latency_ms: f64,     // tracked but not in dominance check yet
    pub observations: u64,
}
```

### Visualization

```
Pass Rate
    1.0 |         * A (Pareto-optimal)
        |
    0.8 |    * C (Pareto-optimal)
        |
    0.7 |              x B (dominated by A)
        |
    0.0 +-----------------------------------> Cost/Success
        $0   $5    $9   $10   $12   $15
```

The Pareto frontier is the upper-left boundary. Points below and to the right of any frontier point are dominated.

### Frontier Evolution

| Phase | Behavior |
|---|---|
| Cold start (0 obs) | All models on frontier (no data to dominate) |
| Convergence (50-200 obs) | Dominated models fall off; typically 2-3 remain |
| Steady state (200+ obs) | Frontier stable. Changes when providers update or new models added |

### New Model Entry

Models with zero observations are excluded from Pareto computation but always included in the candidate set (the bandit gives them maximum exploration priority via UCB1's infinite score for unpulled arms). After accumulating enough observations, they enter the Pareto evaluation.

### Provider Update Handling

When a provider deploys a new model version:
1. Model version change detected (slug comparison).
2. Old observations discounted (partial stats reset).
3. Model re-included in Pareto with reduced weight.

This ensures improved models re-enter the frontier rather than being permanently dominated by stale statistics.

---

## 8. The Health-Routing-Health Cybernetic Loop

Provider health and Pareto pruning form a multi-layered feedback system:

```
Outer Loop (provider health):
    Provider errors detected (React Cell)
    -> Circuit breaker opens (Verify Cell)
    -> Traffic diverted to healthy providers (Route Cell)
    -> Healthy providers receive more load
    -> Eventually: unhealthy provider recovers
    -> Half-open probe succeeds
    -> Circuit breaker closes
    -> Traffic returns to recovered provider

Inner Loop (Pareto pruning):
    Model observations accumulate (Score Cell)
    -> Pareto frontier recomputed every 50 observations
    -> Dominated models pruned from candidate set
    -> Bandit focuses exploration on frontier models
    -> Better routing decisions
    -> More observations accumulate

Cross-loop coupling:
    Circuit breaker open -> model excluded from Pareto computation
    (no observations while open, so stats freeze)
    On recovery -> model re-enters with stale stats
    -> Needs fresh observations to update position on frontier
```

---

## 9. Practical Example

System with four models after 300 observations:

```
Model               Pass Rate   Cost/Success   Provider    Circuit
claude-haiku-4.5     0.78        $0.12         anthropic   Closed
claude-sonnet-4      0.86        $0.95         anthropic   Closed
claude-opus-4        0.91        $2.40         anthropic   Closed
deepseek-chat        0.72        $0.45         deepseek    Closed
```

**Pareto analysis**: deepseek is dominated by haiku (haiku: higher pass rate AND lower cost). Frontier: [haiku, sonnet, opus].

**Then deepseek provider starts returning 5xx errors:**

1. ProviderHealth records 5 ServerError failures in 60s.
2. Circuit breaker for deepseek opens. Cooldown: 120s.
3. CascadeRouter excludes deepseek from candidates.
4. After 120s, breaker transitions to half-open.
5. Single probe request sent to deepseek.
6. Probe succeeds -> breaker closes.
7. Deepseek re-enters candidate set (still dominated, but available).

**Then deepseek improves to 0.85 pass rate after provider update:**

After 50 new observations, Pareto recomputes:
- deepseek (0.85, $0.45) now dominates sonnet (0.86, $0.95) -- nearly same pass rate at half the cost.
- New frontier: [haiku, deepseek, opus]. Sonnet pruned.

---

## What This Enables

1. **Automatic failure isolation**: circuit breaker prevents routing to degraded providers, maintaining system throughput even during provider outages.
2. **Exponential backoff recovery**: failing providers are probed with increasing intervals, preventing wasted requests while ensuring eventual recovery detection.
3. **Exploration efficiency**: Pareto pruning focuses bandit exploration on genuinely competitive models, eliminating waste on dominated alternatives.
4. **Dynamic frontier adaptation**: the frontier evolves as model capabilities change, automatically promoting improved models and demoting degraded ones.
5. **Anomaly early warning**: prompt loops, cost spikes, and quality degradation are detected in real-time via React Cell subscriptions.

## Feedback Loops

- **Health -> Routing (Loop 1)**: provider errors open circuit breaker, diverting traffic to alternatives. Recovery closes breaker, restoring traffic. Negative feedback stabilizes routing.
- **Pareto -> Exploration**: frontier pruning reduces arm count, focusing bandit exploration. Fewer arms -> faster convergence -> better routing -> different Pareto statistics.
- **Backoff escalation**: each failed probe doubles cooldown. This is negative feedback preventing retry storms.
- **Cost -> Routing (Loop 6)**: unhealthy providers cause retries that increase cost. Budget guardrails then force cheaper routing. Cost signal propagates through provider health into routing decisions.
- **Quality degradation -> Replan**: when the anomaly detector fires a quality degradation alert, the orchestrator can trigger gate failure replanning (see [07-LEARNING.md](../../unified/07-LEARNING.md) L3).

## Open Questions

1. **Multi-provider models**: some models are available from multiple providers (e.g., Claude from Anthropic direct vs OpenRouter). Should the circuit breaker track providers independently, allowing the same model to be routed through an alternative provider when the primary is unhealthy?
2. **Pareto with latency**: the current frontier uses two objectives (pass_rate, cost_per_success). Adding latency as a third dimension creates a 3D Pareto surface. Is the increased complexity justified by the routing improvement?
3. **Circuit breaker for content policy**: ContentPolicy errors currently do not trigger the breaker. But if a provider consistently filters responses for a specific task type, should the system learn to avoid routing that task type to that provider?
4. **Probe request cost**: half-open probes consume real tokens and cost money. Should probes use a minimal "health check" prompt instead of a real task to reduce probe cost?
5. **Relationship to autocatalytic-compounding.md**: Bus partition (delivery rate drop) is identified as a single point of failure in the autocatalytic cycle. The circuit breaker is the primary defense against this failure mode. Is the 480s max cooldown sufficient, or should persistent Bus partitions trigger structural escalation (L4)?
