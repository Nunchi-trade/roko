# Provider Health and Circuit Breaker

> **Crate:** `roko-learn` · **Module:** `provider_health.rs`
> **Wiring:** `ProviderHealthRegistry` → `CascadeRouter::select()` (filters unhealthy providers)
> **Implementation plan:** `modelrouting/08-learning-loops.md` (tasks 2G.01–2G.06)
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [08-cost-normalization](08-cost-normalization.md), [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)


> **Implementation**: Shipping

---

## Purpose

The provider health module tracks the operational status of each LLM provider and implements a three-state circuit breaker that prevents routing requests to degraded or failing providers. When a provider starts returning errors (rate limits, timeouts, server errors), the circuit breaker opens, diverting traffic to healthy alternatives. After a cooldown period, the circuit breaker transitions to half-open, allowing a single probe request to test recovery before fully restoring traffic.

This is cybernetic feedback loop 1 (Health→Routing) from the eight missing feedback loops: provider health state directly influences routing decisions in the cascade router.

---

## Three-State Circuit Breaker

```
                 success
    ┌──────────────────────────┐
    │                          │
    ▼                          │
┌────────┐   failure threshold  ┌────────────┐
│ CLOSED │ ────────────────────►│   OPEN     │
│(normal)│                      │(no traffic)│
└────────┘                      └──────┬─────┘
    ▲                                  │
    │                          cooldown expires
    │         success                  │
    │  ┌──────────────────┐            │
    └──┤    HALF-OPEN     │◄───────────┘
       │(single probe req)│
       └──────────────────┘
              │
              │ failure
              │
              ▼
          OPEN (reset cooldown)
```

### States

| State | Behavior | Transition |
|-------|----------|------------|
| **Closed** | Normal operation. All requests are routed. Failures are counted. | → Open: when failure count exceeds threshold within window |
| **Open** | No requests are routed. Traffic is diverted to alternative providers. | → Half-Open: after cooldown period expires |
| **Half-Open** | A single probe request is allowed through. | → Closed: if probe succeeds · → Open: if probe fails (reset cooldown) |

---

## Error Classification

Not all errors are equal. The provider health module classifies errors by type to apply appropriate cooldown policies:

```rust
pub enum ErrorClass {
    /// HTTP 429 — provider rate limit.
    RateLimit,
    /// HTTP 401/403 — authentication or authorization failure.
    AuthFailure,
    /// Request or response timeout.
    Timeout,
    /// HTTP 5xx — provider server error.
    ServerError,
    /// Content policy violation (filtered response).
    ContentPolicy,
    /// Context window exceeded.
    ContextOverflow,
    /// Unclassified error.
    Unknown,
}
```

### Error-Specific Cooldowns

Each error class has a tailored cooldown strategy:

| Error Class | Cooldown | Rationale |
|-------------|----------|-----------|
| `RateLimit` | 60s (escalating with backoff) | Provider will recover after rate window resets |
| `AuthFailure` | 300s (long) | Requires manual intervention (API key rotation) |
| `Timeout` | 30s | Often transient network issues |
| `ServerError` | 120s | Provider-side issues, variable recovery |
| `ContentPolicy` | 0s (no cooldown, flag only) | Not a provider health issue — content-specific |
| `ContextOverflow` | 0s (no cooldown, route to larger model) | Not a provider issue — task-specific |
| `Unknown` | 60s (conservative default) | Unknown errors get conservative treatment |

### Failure Records

Each failure is recorded with its classification:

```rust
pub struct FailureRecord {
    /// When the failure occurred.
    pub timestamp: DateTime<Utc>,
    /// Error classification.
    pub error_class: ErrorClass,
    /// Raw error message (truncated to 256 chars).
    pub message: String,
    /// Model that was being used.
    pub model: String,
}
```

---

## ProviderHealth

Per-provider health state:

```rust
pub struct ProviderHealth {
    /// Provider identifier.
    pub provider_id: String,
    /// Current circuit breaker state.
    pub state: CircuitState,
    /// Recent failure records (bounded window).
    pub recent_failures: VecDeque<FailureRecord>,
    /// Total failure count since last reset.
    pub failure_count: u64,
    /// Total success count since last reset.
    pub success_count: u64,
    /// When the circuit breaker last opened.
    pub last_opened: Option<DateTime<Utc>>,
    /// When the circuit breaker will transition to half-open.
    pub cooldown_until: Option<DateTime<Utc>>,
}
```

### Threshold Configuration

The circuit breaker opens when:
- **Failure count** exceeds the threshold within the observation window, OR
- **Failure rate** (failures / total requests) exceeds the rate threshold.

Default values:
- Failure count threshold: 5 failures
- Observation window: 60 seconds
- Failure rate threshold: 50%

---

## ProviderHealthRegistry

The registry manages health state for all providers:

```rust
pub struct ProviderHealthRegistry {
    providers: Mutex<HashMap<String, ProviderHealth>>,
}
```

Key operations:

| Method | What it does |
|--------|-------------|
| `record_success(provider)` | Increment success count. If half-open, transition to closed. |
| `record_failure(provider, error_class)` | Record failure. Check threshold. If exceeded, open circuit. |
| `is_available(provider)` | Returns `true` if circuit is Closed or Half-Open. |
| `available_providers()` | Returns all providers with Closed or Half-Open circuits. |

### Integration with Cascade Router

The cascade router calls `is_available()` before scoring each candidate model:

```
CascadeRouter::select(context)
    │
    ├── For each candidate model:
    │     │
    │     ├── ProviderHealthRegistry::is_available(model.provider)?
    │     │     YES → include in candidate set
    │     │     NO  → exclude (circuit is Open)
    │     │
    │     └── Score candidate using stage algorithm
    │
    └── Select highest-scoring available candidate
```

If all providers for a desired model tier are unavailable, the router escalates to the next tier or returns the fallback model from the `CascadeModel`.

---

## Exponential Backoff

When a circuit breaker reopens after a failed half-open probe, the cooldown period increases exponentially:

```
cooldown(n) = base_cooldown × 2^(n-1)
```

where `n` is the number of consecutive open→half-open→open cycles. This prevents the system from hammering a persistently failing provider with probe requests.

| Cycle | Cooldown (RateLimit) | Cooldown (ServerError) |
|-------|---------------------|----------------------|
| 1 | 60s | 120s |
| 2 | 120s | 240s |
| 3 | 240s | 480s |
| 4 | 480s (max) | 480s (max) |

The maximum cooldown is capped at 480 seconds (8 minutes) to ensure eventual re-probing even for persistently failing providers.

---

## ProviderHealthTracker

The `ProviderHealthTracker` extends the registry with time-series health metrics for dashboard visualization:

```
Provider: anthropic
├── State: Closed
├── Success rate (1h): 98.2%
├── Failure rate (1h): 1.8%
├── Recent errors: [Timeout × 1, RateLimit × 2]
├── Avg latency (1h): 1,240ms
└── Circuit opens (24h): 2
```

This data feeds into the learning dashboard described in [16-heartbeat](../16-heartbeat/INDEX.md) and the conductor subsystem described in [07-conductor](../07-conductor/INDEX.md).

---

## Anomaly Detection Integration

The `AnomalyDetector` in `anomaly.rs` provides additional provider-health-adjacent checks:

```rust
pub struct AnomalyDetector {
    prompt_hash_window: VecDeque<u64>,    // last 20 prompt hashes
    cost_ewma: EwmaState,                 // EWMA cost baseline
    quality_history: VecDeque<f64>,        // rolling quality scores
    session_cost_usd: f64,
    session_start_ms: i64,
}
```

Three anomaly types:

| Anomaly | Detection | Threshold |
|---------|-----------|-----------|
| **Prompt loop** | Same prompt hash appears 5+ times in last 20 | `PROMPT_LOOP_THRESHOLD = 5` |
| **Cost spike** | Z-score against EWMA baseline > 3.0 | `COST_SPIKE_Z_THRESHOLD = 3.0` |
| **Quality degradation** | Recent 5 scores average < 0.5 AND drop > 0.15 vs prior 10 | Composite check |

### EWMA Cost Baseline

The cost spike detector uses an Exponential Weighted Moving Average with α = 0.2:

```
ewma_new = α × observation + (1 − α) × ewma_old
z_score = (observation − ewma) / ewma_stddev
```

The observation is compared against the EWMA *before* the state is updated, keeping sudden spikes visible instead of folding them into the baseline immediately.

---

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — The cascade router filters candidates using `ProviderHealthRegistry::is_available()`.
- **[08-cost-normalization](08-cost-normalization.md)** — Provider health affects cost indirectly through retry patterns.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Loop 1 (Health→Routing) is the primary feedback path from provider health to routing decisions.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — The circuit breaker is itself a stability mechanism (negative feedback loop).
- **[07-conductor](../07-conductor/INDEX.md)** — The conductor subsystem uses provider health data for its circuit breaker watchers.
