# Production Hardening

> When Roko operates in production — as a daemon, a cloud-deployed server, or an autonomous
> agent — it must handle provider failures, network issues, resource exhaustion, and unexpected
> load gracefully. This document covers adaptive timeouts, exponential backoff with full jitter,
> per-provider concurrency control, context overflow handling, graceful shutdown, content-addressed
> dedup caching, and hedged requests. These patterns are informed by Dean & Barroso's "The Tail
> at Scale" (CACM 2013).


> **Implementation**: Specified

---

## Adaptive Timeouts

Static timeouts are either too aggressive (causing spurious failures during load spikes) or too
lenient (wasting time waiting for dead providers). Roko uses adaptive timeouts that track
provider latency and adjust dynamically.

### Algorithm

The timeout for each provider is set to **p95 latency × 2**, clamped to a [5s, 300s] range:

```rust
/// Adaptive timeout calculation per provider.
pub struct AdaptiveTimeout {
    /// Recent latency samples (ring buffer, last 100 requests)
    samples: VecDeque<Duration>,
    /// Minimum timeout (floor)
    min_timeout: Duration,
    /// Maximum timeout (ceiling)
    max_timeout: Duration,
    /// Multiplier applied to p95
    multiplier: f64,
}

impl AdaptiveTimeout {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(100),
            min_timeout: Duration::from_secs(5),
            max_timeout: Duration::from_secs(300),
            multiplier: 2.0,
        }
    }

    /// Record a completed request's latency.
    pub fn record(&mut self, latency: Duration) {
        if self.samples.len() >= 100 {
            self.samples.pop_front();
        }
        self.samples.push_back(latency);
    }

    /// Calculate the current timeout.
    pub fn timeout(&self) -> Duration {
        if self.samples.is_empty() {
            return Duration::from_secs(30); // Default before any data
        }

        let mut sorted: Vec<_> = self.samples.iter().copied().collect();
        sorted.sort();

        let p95_idx = (sorted.len() as f64 * 0.95) as usize;
        let p95 = sorted[p95_idx.min(sorted.len() - 1)];

        let timeout = Duration::from_secs_f64(p95.as_secs_f64() * self.multiplier);
        timeout.clamp(self.min_timeout, self.max_timeout)
    }
}
```

### Per-Provider Tracking

Each LLM provider (Anthropic, OpenAI, OpenRouter) has its own `AdaptiveTimeout` instance. This
accounts for the significant latency differences between providers:

| Provider | Typical p95 Latency | Adaptive Timeout |
|---|---|---|
| Anthropic Claude Sonnet | ~3-8s | ~6-16s |
| Anthropic Claude Opus | ~15-45s | ~30-90s |
| OpenAI GPT-4o | ~5-15s | ~10-30s |
| OpenRouter (varies by model) | ~10-60s | ~20-120s |

The timeout tracker persists across daemon restarts via `.roko/learn/provider-timeouts.json`.

---

## Exponential Backoff with Full Jitter

When a provider request fails (timeout, rate limit, server error), Roko retries with
exponential backoff and full jitter. The jitter prevents thundering herd problems when multiple
agents retry simultaneously.

### Algorithm

Full jitter (from AWS Architecture Blog, "Exponential Backoff and Jitter"):

```
sleep = random_between(0, min(cap, base × 2^attempt))
```

```rust
use rand::Rng;

/// Calculate backoff duration with full jitter.
pub fn backoff_with_jitter(attempt: u32, base_ms: u64, cap_ms: u64) -> Duration {
    let exp_backoff = base_ms.saturating_mul(2u64.saturating_pow(attempt));
    let capped = exp_backoff.min(cap_ms);
    let jittered = rand::thread_rng().gen_range(0..=capped);
    Duration::from_millis(jittered)
}
```

### Retry Configuration

```rust
/// Retry policy for provider requests.
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Base delay for exponential backoff (milliseconds)
    pub base_delay_ms: u64,
    /// Maximum delay cap (milliseconds)
    pub max_delay_ms: u64,
    /// Which errors are retryable
    pub retryable: RetryableErrors,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 1000,   // 1 second base
            max_delay_ms: 60_000,  // 60 second cap
            retryable: RetryableErrors::default(),
        }
    }
}

/// Which errors trigger a retry.
#[derive(Debug, Default)]
pub struct RetryableErrors {
    pub timeout: bool,            // Request timed out → retry
    pub rate_limit: bool,         // 429 Too Many Requests → retry with backoff
    pub server_error: bool,       // 500/502/503 → retry
    pub connection_error: bool,   // TCP/TLS failure → retry
    pub auth_error: bool,         // 401/403 → DO NOT retry (key is wrong)
    pub client_error: bool,       // 400 → DO NOT retry (request is malformed)
}
```

### Retry Sequence Example

```
Attempt 1: Request to Anthropic → 503 Service Unavailable
  → sleep random(0, min(60000, 1000 × 2^0)) = random(0, 1000) → sleep 423ms

Attempt 2: Request to Anthropic → 503 Service Unavailable
  → sleep random(0, min(60000, 1000 × 2^1)) = random(0, 2000) → sleep 1847ms

Attempt 3: Request to Anthropic → 200 OK
  → Success, record latency for adaptive timeout
```

### RetryAction Enum

The `should_retry()` function in `roko-agent` returns a structured decision:

```rust
/// Decision from the retry policy.
pub enum RetryAction {
    /// Retry after the specified delay
    Retry { delay: Duration, reason: String },
    /// Do not retry — the error is permanent
    DoNotRetry { reason: String },
    /// Switch to a different provider and retry
    Failover { provider: String, reason: String },
}

/// Determine the retry action for a provider error.
pub fn should_retry(
    error: &ProviderError,
    attempt: u32,
    policy: &RetryPolicy,
) -> RetryAction {
    if attempt >= policy.max_retries {
        return RetryAction::DoNotRetry {
            reason: format!("Max retries ({}) exceeded", policy.max_retries),
        };
    }

    match error {
        ProviderError::RateLimit { retry_after } => {
            let delay = retry_after.unwrap_or_else(|| {
                backoff_with_jitter(attempt, policy.base_delay_ms, policy.max_delay_ms)
            });
            RetryAction::Retry {
                delay,
                reason: "Rate limited".to_string(),
            }
        }
        ProviderError::ServerError(_) => {
            RetryAction::Retry {
                delay: backoff_with_jitter(attempt, policy.base_delay_ms, policy.max_delay_ms),
                reason: "Server error".to_string(),
            }
        }
        ProviderError::Timeout => {
            RetryAction::Failover {
                provider: "next_available".to_string(),
                reason: "Timeout — try different provider".to_string(),
            }
        }
        ProviderError::AuthError(_) => {
            RetryAction::DoNotRetry {
                reason: "Authentication failed — check API key".to_string(),
            }
        }
        _ => RetryAction::DoNotRetry {
            reason: format!("Non-retryable error: {error}"),
        },
    }
}
```

---

## Per-Provider Concurrency Control

Each provider has a maximum number of concurrent requests, enforced by a Tokio semaphore. This
prevents overwhelming a single provider and respects rate limits proactively.

```rust
use tokio::sync::Semaphore;

/// Per-provider concurrency limiter.
pub struct ProviderConcurrency {
    /// Semaphore controlling max concurrent requests
    semaphore: Semaphore,
    /// Current count of in-flight requests
    in_flight: AtomicU32,
}

impl ProviderConcurrency {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Semaphore::new(max_concurrent),
            in_flight: AtomicU32::new(0),
        }
    }

    /// Acquire a permit before making a request.
    pub async fn acquire(&self) -> SemaphorePermit<'_> {
        let permit = self.semaphore.acquire().await.unwrap();
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        permit
    }
}
```

Default concurrency limits:

| Provider | Max Concurrent | Rationale |
|---|---|---|
| Anthropic | 10 | Anthropic rate limits are per-API-key |
| OpenAI | 20 | OpenAI allows higher concurrency |
| OpenRouter | 15 | Varies by underlying model |

These limits are configurable in `roko.toml`:

```toml
[agent.providers.anthropic]
max_concurrent = 10

[agent.providers.openai]
max_concurrent = 20
```

---

## Context Overflow Handling

When an agent's context window approaches capacity, the system must handle the overflow
gracefully rather than failing with a truncation error.

### 80% Trigger Threshold

The Composer monitors context window usage and triggers overflow handling at 80% capacity:

```rust
/// Check if context is approaching overflow.
fn check_context_overflow(
    current_tokens: usize,
    max_tokens: usize,
) -> ContextState {
    let usage_ratio = current_tokens as f64 / max_tokens as f64;

    if usage_ratio >= 0.95 {
        ContextState::Critical // Must shed context immediately
    } else if usage_ratio >= 0.80 {
        ContextState::Warning  // Begin proactive context reduction
    } else {
        ContextState::Normal
    }
}
```

### Overflow Response

When context reaches the warning threshold:

1. **Summarize old context**: The Composer replaces older, lower-utility Engrams with a
   summary Engram that captures the key information in fewer tokens
2. **Decay acceleration**: Engrams below the utility threshold have their decay rates
   accelerated, causing them to drop out of the active context sooner
3. **Priority shedding**: The Router's VCG attention auction (see the context engineering
   documentation) naturally handles this — lower-bid context items are evicted first

When context reaches the critical threshold:

1. **Force eviction**: Drop the lowest-utility 20% of context regardless of bids
2. **Log the eviction**: Record which Engrams were dropped (for audit trail)
3. **Continue execution**: The agent continues with reduced context rather than failing

---

## Graceful Shutdown

When the server or daemon receives a shutdown signal (SIGTERM, SIGINT), it performs a 3-phase
graceful shutdown:

### Phase 1: Stop Accepting (immediate)

```rust
// Set the shutdown flag — new requests get 503 Service Unavailable
state.shutting_down.store(true, Ordering::SeqCst);

// Stop accepting new plan runs
state.run_queue.lock().close();

// Health check endpoint returns 503 (Fly/Docker stops routing traffic)
```

### Phase 2: Drain (up to 30 seconds)

```rust
// Wait for in-flight requests to complete
let drain_timeout = Duration::from_secs(30);
let drain_start = Instant::now();

loop {
    let in_flight = state.in_flight_count.load(Ordering::Relaxed);
    if in_flight == 0 {
        break; // All requests completed
    }
    if drain_start.elapsed() > drain_timeout {
        tracing::warn!(
            in_flight,
            "Drain timeout exceeded, forcing shutdown"
        );
        break;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
}
```

### Phase 3: Checkpoint and Exit

```rust
// Kill any remaining agent processes
state.supervisor.shutdown_all().await;

// Flush pending signals to disk
state.substrate.flush().await?;

// Save executor state for resume
state.executor.checkpoint(".roko/state/executor.json").await?;

// Save learning data
state.cascade_router.save().await?;
state.experiment_store.save().await?;
state.gate_thresholds.save().await?;

// Close the IPC socket
state.ipc_server.shutdown().await;

// Exit cleanly
std::process::exit(0);
```

### Shutdown Timeline

```
SIGTERM received
  │
  ├─ t=0s     Stop accepting new requests (503 for new traffic)
  │           Health check returns 503 (load balancer drains traffic)
  │
  ├─ t=0-30s  Drain in-flight requests
  │           Agents complete current turns
  │           Gates complete current verifications
  │
  ├─ t=30s    Force-kill remaining agents
  │           Checkpoint executor state
  │           Flush signal log
  │           Save learning data
  │
  └─ t=31s    Process exits (exit code 0)
```

---

## Content-Addressed Dedup Cache

LLM responses are cached using content-addressed keys to deduplicate identical requests:

```rust
use blake3::Hasher;

/// Generate a cache key from the request parameters.
fn cache_key(model: &str, messages: &[Message], temperature: f32) -> ContentHash {
    let mut hasher = Hasher::new();
    hasher.update(model.as_bytes());
    for msg in messages {
        hasher.update(msg.role.as_bytes());
        hasher.update(msg.content.as_bytes());
    }
    hasher.update(&temperature.to_le_bytes());
    ContentHash::from(hasher.finalize())
}
```

The cache is a bounded LRU with configurable maximum entries:

```toml
[agent.cache]
enabled = true
max_entries = 10000
ttl_seconds = 3600  # 1 hour
```

Cache hit rates in practice:

| Scenario | Typical Hit Rate | Savings |
|---|---|---|
| Repeated gate checks (same code, same test) | ~60-80% | Major cost reduction |
| Retried agent turns (same context) | ~30-50% | Latency reduction |
| Cross-agent shared prompts (system prompts) | ~20-30% | Minor savings |

The cache key includes the model name and temperature, so different model configurations
produce different cache entries. Temperature > 0 requests are not cached by default (non-
deterministic outputs).

---

## Hedged Requests

For latency-critical operations, hedged requests send the same request to multiple providers
simultaneously and use the first response, canceling the others. This technique is from
Dean & Barroso's "The Tail at Scale" (Communications of the ACM, 2013).

### Implementation

```rust
/// Send a hedged request to multiple providers.
async fn hedged_request(
    request: &LlmRequest,
    providers: &[Provider],
    hedge_delay: Duration,
) -> Result<LlmResponse> {
    // Start with the primary provider
    let primary = providers[0].send(request);

    // After hedge_delay, also send to the secondary provider
    let hedged = async {
        tokio::time::sleep(hedge_delay).await;
        providers[1].send(request).await
    };

    // Use whichever completes first
    tokio::select! {
        result = primary => result,
        result = hedged => result,
    }
}
```

### When to Hedge

Hedging is not free — it consumes tokens from multiple providers. Use it only when:

1. **The request is latency-critical** (e.g., a gate check blocking the pipeline)
2. **The primary provider is showing elevated latency** (adaptive timeout reports p95 > 2×
   normal)
3. **The cost is acceptable** (simple requests with low token counts)

The hedge delay is set to the primary provider's p50 latency — this means the hedge only fires
if the primary is slower than median, limiting unnecessary duplicate requests.

```toml
[agent.hedging]
enabled = false           # Off by default (opt-in per deployment)
hedge_delay_percentile = 50  # Fire hedge after p50 latency
max_hedge_cost_usd = 0.01   # Don't hedge if request would cost more than this
```

---

## Health Check Patterns

Production deployments expose health check endpoints for load balancers and orchestrators:

```rust
/// Health check endpoint for load balancers.
async fn health_check(State(state): State<Arc<ServerState>>) -> StatusCode {
    if state.shutting_down.load(Ordering::Relaxed) {
        return StatusCode::SERVICE_UNAVAILABLE; // Draining
    }

    // Check critical subsystems
    let substrate_ok = state.substrate.ping().await.is_ok();
    let agents_ok = state.supervisor.healthy();

    if substrate_ok && agents_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
```

### Readiness vs Liveness

| Endpoint | Purpose | Returns 503 When |
|---|---|---|
| `/health` (liveness) | "Is the process alive?" | Process is dead or hung |
| `/ready` (readiness) | "Can it accept work?" | Shutting down, overloaded, or subsystem failure |

Kubernetes and Fly.io use liveness to decide whether to restart the container and readiness to
decide whether to route traffic to it.

---

## Current Status

Production hardening features are partially implemented:

| Feature | Status | Location |
|---|---|---|
| Adaptive timeouts | **Implemented** | `roko-agent/src/provider/` |
| Exponential backoff | **Implemented** | `roko-agent/src/provider/` |
| RetryAction enum | **Implemented** | `roko-agent/src/provider/` (Tier 2G.17) |
| Per-provider semaphores | **Scaffolded** | Designed, not wired |
| Context overflow | **Partial** | Composer has budget limits, no 80% trigger |
| Graceful shutdown | **Partial** | ProcessSupervisor handles agent cleanup |
| Dedup cache | **Not implemented** | Designed |
| Hedged requests | **Not implemented** | Designed |
| Health check endpoints | **Scaffolded** | roko-serve has /health stub |
