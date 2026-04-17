# Production Hardening

> When Roko runs in laptop-local, single-server, container, clustered, or edge profiles, the
> runtime still has to behave like production software: bounded latency, safe retries, clean
> shutdown, upgradeability, observability, and tenant isolation. See also
> `../../tmp/refinements/24-deployment-ux.md` and `../../tmp/refinements/27-realtime-event-surface.md`.

> **Implementation**: Specified

---

## Adaptive Timeouts

Static timeouts are brittle. Roko should track per-provider latency and set timeouts from
recent observations.

### Algorithm

Use `p95 latency × 2`, clamped to a sane range:

```rust
pub fn timeout_for(samples: &[Duration]) -> Duration {
    if samples.is_empty() {
        return Duration::from_secs(30);
    }

    let mut sorted = samples.to_vec();
    sorted.sort();
    let p95_idx = (sorted.len() as f64 * 0.95) as usize;
    let p95 = sorted[p95_idx.min(sorted.len() - 1)];
    Duration::from_secs_f64((p95.as_secs_f64() * 2.0).clamp(5.0, 300.0))
}
```

### Per-Provider Tracking

Keep separate timeout histories for each provider. Container and clustered deployments benefit
most because they run many requests concurrently, but the same logic applies in laptop-local
and single-server profiles.

---

## Exponential Backoff with Full Jitter

When a request fails, retry with exponential backoff and full jitter:

```text
sleep = random_between(0, min(cap, base × 2^attempt))
```

The jitter prevents synchronized retries from stampeding a provider during transient failures.

### Retry Configuration

Retryable failures should be explicit. Authentication and malformed requests should not be
retried; timeouts, server errors, and rate limits can be retried or failed over.

### RetryAction Enum

Retried requests should return a structured decision so the caller can either retry, fail over,
or stop immediately.

---

## Per-Provider Concurrency Control

Enforce provider-specific concurrency limits with semaphores or equivalent back-pressure.
That protects both the provider and the local process.

Default limits should be profile-aware:

| Profile | Typical concurrency posture |
|---|---|
| laptop-local | conservative, interactive |
| single-server | moderate, shared-machine safe |
| container | tuned for one instance per node |
| clustered | horizontal scale with per-node caps |
| edge | minimal, request-scoped |

Per-tenant quotas layer on top of per-provider concurrency in shared deployments.

---

## Context Overflow Handling

When context approaches capacity, reduce it before the model fails.

### 80% Trigger Threshold

At roughly 80% usage, start summarizing or shedding lower-value context. At critical usage,
force eviction and continue with the reduced state.

### Overflow Response

1. Summarize older context into a smaller prompt Engram.
2. Accelerate demurrage or decay for low-value material.
3. Evict the least useful items first and log the decision.

---

## Graceful Shutdown

Shutdown should drain work, checkpoint durable state, and then exit.

### Phase 1: Stop Accepting

Mark the service unavailable for new work and flip readiness to false.

### Phase 2: Drain

Wait for in-flight requests to finish within a bounded window.

### Phase 3: Checkpoint and Exit

Flush durable state, persist executor progress, and close transports cleanly.

The same shutdown path supports regular exits and rolling upgrades.

For long-lived realtime subscribers, draining needs one extra rule: readiness should fail before
liveness so new subscriptions stop landing on a node while existing WebSocket and SSE clients
either finish or reconnect elsewhere with their last cursor.

---

## Zero-Downtime Upgrades

Single-server and clustered deployments should upgrade without losing in-flight work.

- Drain traffic before terminating the old process.
- Resume from the last checkpoint or saved state archive.
- Keep health checks and readiness probes aligned so orchestrators can replace one node at a
  time.
- For clustered deployments, do rolling replacement behind the load balancer.

Container deployments should treat upgrades as a new image plus a state handoff, not a manual
reinstall.

Clustered deployments should avoid treating sticky sessions as the main continuity mechanism.
Shared cursor retention and replayable projection state matter more than pinning every browser to
one node.

---

## Observability

Production deployments need the same observability contract in every shape:

- Structured logs to stderr by default.
- Prometheus-compatible metrics on `/metrics`.
- OpenTelemetry traces around the operator pipeline.
- Readiness and liveness probes for orchestrators.

Useful Roko-specific metrics include:

| Metric | Meaning |
|---|---|
| `roko.c_factor` | Collective-intelligence health |
| `roko.bus.pulses_per_second` | Bus throughput |
| `roko.gate.pass_rate` | Gate success rate |
| `roko.substrate.query_latency_p99` | Storage latency |
| `roko.tenant.quota_utilization` | Tenant pressure in shared deployments |

These metrics should carry shape and tenant labels where cardinality is safe.

### Realtime Surface Telemetry

The realtime surface needs its own operational signals when it is exposed remotely:

| Metric | Meaning |
|---|---|
| `roko.realtime.connections` | open connections by transport |
| `roko.realtime.subscriptions` | active subscriptions by channel family |
| `roko.realtime.messages_per_second` | inbound and outbound traffic rate |
| `roko.realtime.cursor_lag` | how far behind subscribers are |
| `roko.realtime.reconnects` | reconnect churn during outages or deploys |
| `roko.realtime.backpressure_coalesced_total` | updates coalesced under pressure |
| `roko.realtime.backpressure_dropped_total` | updates dropped in lossy modes |
| `roko.realtime.auth_denied_total` | subscribe or publish denials |

These are the signals operators need to decide whether the remote surface is healthy, overloaded,
or misconfigured behind a proxy.

---

## Content-Addressed Dedup Cache

Duplicate requests should reuse cached responses when the request, model, and parameters match.
That reduces cost and improves latency in every profile, especially clustered and container
deployments.

---

## Hedged Requests

Hedged requests send the same work to multiple providers when latency matters more than token
cost. Use them sparingly and only when the deployment profile can afford the duplicate work.

---

## Health Check Patterns

`/healthz` and `/readyz` should mean the same thing across Docker, Compose, Fly.io, systemd,
and clustered orchestrators.

### Readiness vs Liveness

- Readiness answers “should traffic be sent here now?”
- Liveness answers “is the process still healthy enough to stay up?”

During shutdown or upgrade, readiness should fail before liveness does so traffic drains
cleanly.

For realtime traffic, this also means:

- `SSE` endpoints must disable proxy buffering
- `WebSocket` endpoints must preserve upgrade headers through ingress
- replay retention must outlive short node restarts so reconnecting clients do not fall off the log immediately

---

## Multi-Tenant Safety

Shared single-server and clustered deployments need explicit tenant boundaries:

- Scope substrate keys by tenant.
- Enforce per-tenant quotas for tokens, spend, and episode counts.
- Keep auth and role checks tenant-aware.
- Label metrics and traces with tenant identifiers only where that remains low-cardinality.

The point is isolation without creating a separate codepath per tenant.

---

## Current Status

The production-hardening model is profile-aware and shape-aware. The actionable part of the
deployment chapter is that timeout handling, retries, shutdown, upgrade flow, observability,
and tenant controls should all work the same way across the five deployment shapes.
