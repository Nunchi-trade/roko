# Task 023: Add Degradation Detection to Health Check

```toml
id = 23
title = "Health endpoint returns 503 only on full outage — add degraded state detection"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-serve/src/routes/status/health.rs",
    "crates/roko-serve/src/routes/status/mod.rs",
]
exclusive_files = [
    "crates/roko-serve/src/routes/status/health.rs",
    "crates/roko-serve/src/routes/status/mod.rs",
]
estimated_minutes = 60
```

## Context

The rich health check currently returns 503 only when all providers are fully down. When
providers are degraded (slow, intermittent errors, or partial provider outage), the body
does not clearly expose degraded status. This task adds deterministic degradation
detection without changing liveness/readiness semantics.

Sources:
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` — W14-B: Health 503 partial
- `tmp/infrastructure-audit.md` — Health/readiness endpoints

## Background

Read:
- `crates/roko-serve/src/routes/status/health.rs` — `/api/health` implementation.
- `crates/roko-serve/src/routes/status/mod.rs` — status route tests.
- `crates/roko-serve/src/routes/mod.rs` — top-level `/health` and `/ready`.
- `crates/roko-learn/src/provider_health.rs` — `ProviderHealthTracker`,
  `ProviderStatus`, `HealthState`, `error_rate()`, and `snapshot()`.
- `crates/roko-learn/src/latency.rs` — `LatencyRegistry::get_all_for_provider` and
  `LatencyStats::p95_ms()`.
- `crates/roko-serve/src/routes/providers.rs` — example use of provider health plus
  latency stats.

Route clarification: implement this on `GET /api/health` in
`routes/status/health.rs`. Do not change the top-level `GET /health` liveness endpoint;
it intentionally returns 200 while the process is alive. Do not change `GET /ready`
unless tests show the existing shutdown behavior is broken.

## What to Change

1. **Add degraded state detection** to `/api/health` using provider snapshots and latency
   registry data.
2. **Return HTTP 200 with `"status": "degraded"`** when at least one provider is usable
   but one or more providers are degraded/unhealthy, or when latency/error-rate
   thresholds are exceeded.
3. **Return HTTP 503 with `"status": "unhealthy"`** only when every known provider is
   unhealthy/down.
4. **Keep `"status": "ok"` with HTTP 200** when there are no known providers or every
   known provider is healthy and below thresholds.
5. **Include enough provider summary data** for operators/tests to see why the aggregate
   status was chosen.

## Deterministic Thresholds

Add local constants in `routes/status/health.rs` unless an existing config surface already
defines equivalent values:

```rust
const DEGRADED_ERROR_RATE_MIN_ATTEMPTS: u64 = 5;
const DEGRADED_ERROR_RATE_THRESHOLD: f64 = 0.20;
const DEGRADED_P95_LATENCY_MIN_OBSERVATIONS: u64 = 3;
const DEGRADED_P95_LATENCY_MS: f64 = 30_000.0;
```

Provider classification:

- `unhealthy`: provider snapshot state is `HealthState::Unhealthy { .. }` or
  `HealthState::Probing { .. }`.
- `degraded`: not unhealthy, and any of these is true:
  - `consecutive_failures > 0`;
  - `total_attempts >= DEGRADED_ERROR_RATE_MIN_ATTEMPTS` and
    `error_rate() >= DEGRADED_ERROR_RATE_THRESHOLD`;
  - latency observations for the provider are at least
    `DEGRADED_P95_LATENCY_MIN_OBSERVATIONS` and `p95_ms() > DEGRADED_P95_LATENCY_MS`.
- `ok`: none of the above.

Aggregate status:

- no providers: `ok`, HTTP 200;
- all providers classified `unhealthy`: `unhealthy`, HTTP 503;
- any provider classified `degraded` or `unhealthy`, with at least one provider not
  unhealthy: `degraded`, HTTP 200;
- otherwise: `ok`, HTTP 200.

If the current code still returns `"down"` for full outage, replace it with
`"unhealthy"` and update tests. Do not use both spellings in new assertions.

## Mechanical Implementation Notes

- Use `state.provider_health.snapshot()` to avoid mutating health state during a health
  request. Do not call `ProviderHealthTracker::is_healthy()` from the HTTP route.
- Use `state.latency_registry.get_all_for_provider(&provider_id)` for p95 data.
- Keep nonblocking lock behavior already used by `/api/health` for active terminal and
  relay counts; do not introduce awaited locks on request-critical paths.
- Suggested JSON body additions:
  - `providers.total`
  - `providers.healthy`
  - `providers.degraded`
  - `providers.unhealthy`
  - optional `providers.details[]` with provider id, state, error rate, p95 latency, and
    reason.
- Preserve existing fields that callers may already consume, such as uptime, active
  terminals, active relays, and provider counts, unless a test proves they are unused and
  removal is explicitly approved.

## What NOT to Do

- Don't add authentication to health endpoints.
- Don't change the provider health check mechanism itself.
- Don't change top-level `/health`; it is liveness and should stay 200 while the process
  is running.
- Don't add a new `/ready` endpoint; it already exists in `routes/mod.rs`.
- Don't return 503 for partial degradation. Partial degradation should be HTTP 200 with
  body `"status": "degraded"`.
- Don't move threshold logic into `ProviderHealthTracker`; this task is route-level
  reporting.

## Wire Target

```bash
cargo run -p roko-cli -- serve --port 6677
curl -s -i http://localhost:6677/api/health
# Body status should be "ok", "degraded", or "unhealthy".
# HTTP 503 should appear only when every known provider is unhealthy.

curl -s -i http://localhost:6677/health
# Top-level liveness remains HTTP 200 with {"status":"ok"} while the process is alive.
```

Expected observable behavior: `/api/health` reports `degraded` for partial provider
outage, high error rate, or high p95 latency; it reports HTTP 503 only for full provider
outage. `/health` and `/ready` keep their existing liveness/readiness roles.

## Tests to Add or Update

Add tests in `crates/roko-serve/src/routes/status/mod.rs` near existing health tests:

- `health_reports_degraded_when_error_rate_above_threshold`: record enough attempts for
  one provider to exceed the configured error-rate threshold without tripping full
  outage, expect HTTP 200 and body status `degraded`.
- `health_reports_degraded_when_latency_p95_above_threshold`: record at least three high
  latencies for a provider, expect HTTP 200 and body status `degraded`.
- `health_reports_unhealthy_503_when_all_providers_unhealthy`: mark every known provider
  unhealthy by recording consecutive failures, expect HTTP 503 and body status
  `unhealthy`.
- `health_reports_200_degraded_when_some_provider_unhealthy`: one provider healthy and
  one unhealthy, expect HTTP 200 and body status `degraded`.
- Keep existing top-level `/health` and `/ready` tests unchanged; add regression coverage
  if this task accidentally changes those endpoints.

## Verification

- [ ] `cargo test -p roko-serve health_reports -- --nocapture`
- [ ] `cargo test -p roko-serve top_level -- --nocapture`
- [ ] `cargo build -p roko-serve`
- [ ] Health endpoint reports degraded state when some providers are down
- [ ] `/api/health` returns HTTP 503 only for full provider outage
- [ ] `/health` still returns HTTP 200 while the process is alive

## Status Log

| Time | Agent | Action |
|------|-------|--------|
