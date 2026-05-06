# Task 105: Runtime Event Ingest E2E Hardening

```toml
id = 105
title = "Harden RuntimeEvent ingest, HttpEventSink, and ACP forwarding with E2E verification"
track = "engine-events"
wave = "wave-6"
priority = "high"
blocked_by = [104]
touches = [
    "crates/roko-serve/src/routes/event_ingest.rs",
    "crates/roko-serve/src/routes/mod.rs",
    "crates/roko-serve/src/adapters.rs",
    "crates/roko-runtime/src/http_event_sink.rs",
    "crates/roko-acp/src/event_forward.rs",
    "crates/roko-acp/src/bridge_events.rs",
    "tmp/solutions/demo-running/01-WAVE-A-ENGINE.md",
    "tmp/solutions/demo-running/CHECKLIST.md",
]
exclusive_files = []
estimated_minutes = 240
```

## Context

A5/A6/A8 from the demo-running plan are now implemented in code:

- `POST /api/events/ingest` and `/api/events/ingest/batch` accept canonical
  `RuntimeEvent` JSON and return `202 Accepted`.
- `roko-runtime::HttpEventSink` batches events with a 50ms window and 32-event max batch,
  posts to `/api/events/ingest/batch`, reads `ROKO_SERVE_URL`, and supports bearer auth via
  `ROKO_SERVER_AUTH_TOKEN`.
- The v2 runner auto-creates the sink from environment and forwards runner lifecycle events.
- ACP maps `CognitiveEvent` to `RuntimeEvent` through `crates/roko-acp/src/event_forward.rs`.
- PTY sessions inject `ROKO_SERVE_URL`, `ROKO_SESSION_ID`, and `ROKO_SERVER_AUTH_TOKEN`.

The gap is not "build the feature"; the gap is hardening, tests, and documentation accuracy.

## Known Gaps

- Route-level tests cover only helper behavior; they do not prove RuntimeEvent ingest reaches
  SSE / StateHub / JSONL logging.
- `HttpEventSink` lacks HTTP/batching/auth tests.
- ACP forwarding lacks an end-to-end test from `CognitiveEvent` to serve SSE.
- Source docs still describe A5/A6/A8 as future work and mention an ingest-specific
  `1000/sec` rate limit that does not exist. Current code enforces a 1000-event batch max and
  inherits the global HTTP rate limit.
- Some old examples show stale `CognitiveEvent` and `RuntimeEvent::AgentOutput` field shapes.

## Implementation Detail

### Current Source Facts

1. Event ingest routes live in `crates/roko-serve/src/routes/event_ingest.rs`. The router exposes `/events/ingest` and `/events/ingest/batch`; when mounted by the serve router the external paths are `/api/events/ingest` and `/api/events/ingest/batch`.
2. `RuntimeEvent` uses serde `#[serde(tag = "kind", content = "data", rename_all = "snake_case")]`. A canonical agent output request body is:
   ```json
   {"kind":"agent_output","data":{"run_id":"task105","agent_id":"manual","chunk":"hello-ingest"}}
   ```
3. `consume_runtime_event` currently forwards each event to `state.sse_adapter.consume(event)` and `state.runtime_event_logger.consume(event)`. `SseAdapter::consume` converts runtime events to SSE events, broadcasts them, and forwards to the StateHub consumer when `routes/mod.rs::build_router` has attached one.
4. The runtime event JSONL path is `.roko/runtime-events.jsonl`, via `JsonlLogger::from_roko_dir`; do not assert against `.roko/events.jsonl` for this path.
5. Batch ingest rejects more than 1000 events. `HttpEventSink` in `crates/roko-runtime/src/http_event_sink.rs` batches up to 32 events, uses a 50 ms flush window, trims `ROKO_SERVE_URL`, posts to `/api/events/ingest/batch`, sends bearer auth when configured, and uses `try_send` so `emit` is non-blocking.
6. ACP forwarding lives in `crates/roko-acp/src/event_forward.rs`; the private `map_event` function can be tested from a child `#[cfg(test)]` module in that file.

### Mechanical Test Plan

1. Add route-level ingest tests in `crates/roko-serve/src/routes/event_ingest.rs` or the existing route test module. Prefer the established `build_test_state_and_router` pattern from `crates/roko-serve/src/routes/mod.rs` so the real `AppState`, SSE adapter, runtime event logger, and StateHub wiring are exercised.
2. For axum `.oneshot` tests that hit auth/remote-address logic, insert `ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345)))` into the request extensions before dispatch. Add at least one non-loopback remote-address rejection test when auth is disabled and the bind/allowlist policy should deny ingest.
3. Cover single ingest: POST the canonical `agent_output` body to `/api/events/ingest`, assert `202 Accepted`, assert the SSE subscriber receives an `agent_output` event, and assert `.roko/runtime-events.jsonl` contains the serialized `RuntimeEvent`.
4. Cover batch ingest: POST two mixed runtime events to `/api/events/ingest/batch`, assert `202 Accepted`, assert both are broadcast/logged in order. Add an over-limit test with 1001 events and assert client error without partial consumption.
5. Add StateHub projection coverage where the in-process router configures StateHub. Assert the ingested event reaches the StateHub snapshot/event log path described in `docs/v1/12-interfaces/22-statehub-projection-layer.md`.
6. Add `HttpEventSink` tests in `crates/roko-runtime/src/http_event_sink.rs` with a local axum/TCP test server. Assert the sink posts to `/api/events/ingest/batch`, includes the bearer token when configured, preserves event shape, batches by max-size/window, and does not block or panic when the channel is saturated. Use a `#[cfg(test)]` helper if needed; do not expose testing knobs in the public API.
7. Add ACP mapping tests in `crates/roko-acp/src/event_forward.rs` for token/thinking chunks, tool completion/failure, end-turn completion, cancellation, failure, and max-token termination. These should test `map_event` directly and not require an LLM, network, or spawned subprocess.

### Manual E2E Verification

1. Terminal 1:
   ```bash
   cargo run -p roko-cli -- serve
   ```
2. Terminal 2:
   ```bash
   curl -N http://127.0.0.1:6677/api/workflow/events
   ```
3. Terminal 3:
   ```bash
   curl -i -X POST http://127.0.0.1:6677/api/events/ingest \
     -H 'content-type: application/json' \
     -d '{"kind":"agent_output","data":{"run_id":"task105","agent_id":"manual","chunk":"hello-ingest"}}'
   ```
4. Expected observable behavior: the POST returns `202`, the SSE stream receives an `agent_output` event, `.roko/runtime-events.jsonl` receives one JSON line, and the StateHub-backed dashboard/event view updates when the app router is using StateHub.
5. For subprocess forwarding, run a CLI path that actually emits runtime events with `ROKO_SERVE_URL=http://127.0.0.1:6677` and the matching auth token if serve auth is enabled. Do not rely on a dry-run-only path unless source inspection confirms it emits through `roko_runtime::event_bus`.

### Anti-Patterns

1. Do not confuse `RuntimeEvent` with `ServerEvent`. `ServerEvent::AgentOutput` may still use `content`/`done`; ingest must accept and test the `RuntimeEvent::AgentOutput { chunk }` shape.
2. Do not make route tests depend on an external `roko serve` process; keep CI tests in-process and reserve subprocess checks for an ignored/manual E2E if needed.
3. Do not introduce sleeps longer than the sink batching window plus a small timeout; use `tokio::time::timeout` around expected receives.
4. Do not bypass `consume_runtime_event` in tests when asserting end-to-end ingest behavior; the point is to prove one HTTP event reaches SSE, JSONL, and StateHub through the same route path.

## What to Change

1. Add route-level ingest tests:
   - single RuntimeEvent POST returns 202
   - batch POST returns 202
   - batch > 1000 returns a clear error
   - unauthorized / non-local request behavior matches the configured security model
2. Add `HttpEventSink` tests:
   - trims `ROKO_SERVE_URL`
   - includes bearer token when `ROKO_SERVER_AUTH_TOKEN` is set
   - batches up to 32 events or 50ms, whichever comes first
   - drops instead of blocking when the queue is full
3. Add ACP forwarding tests:
   - map representative `CognitiveEvent` variants to canonical `RuntimeEvent` variants
   - verify no stale `content/done` AgentOutput shape remains in examples
4. Update demo-running docs to current status:
   - A5, A6, A7, A8 are implemented; E2E verification pending
   - rate-limit language says "1000-event batch max; global HTTP rate limit applies"
   - docs point to actual files and current event shapes

## What NOT to Do

- Do not introduce `ServerEvent` into ingest.
- Do not create a second HTTP sink in ACP or CLI.
- Do not bypass `EventConsumer`; runtime events should continue through the existing adapters.
- Do not mark the feature fully done until SSE and JSONL/StateHub behavior is observed.

## Wire Target

```bash
cargo run -p roko-cli -- serve
curl -i -X POST http://127.0.0.1:6677/api/events/ingest \
  -H 'content-type: application/json' \
  -d '{"kind":"agent_output","data":{"run_id":"task105","agent_id":"manual","chunk":"hello-ingest"}}'
curl -N http://127.0.0.1:6677/api/workflow/events
```

Expected observable behavior: the ingest POST returns `202 Accepted`, the workflow SSE stream
receives the canonical `agent_output` runtime event, `.roko/runtime-events.jsonl` records the
event, and StateHub-backed views update when the serve router is using StateHub.

## Verification

Compilation and tests can be deferred until merge coalescing if the batch policy says so, but
the final task is not done until these checks are clean:

- [ ] Route-level ingest tests pass.
- [ ] `HttpEventSink` batching/auth tests pass.
- [ ] ACP mapping tests pass.
- [ ] Manual E2E: start `roko serve`, POST a RuntimeEvent to `/api/events/ingest`, observe it
      on `/api/workflow/events` or the canonical SSE endpoint.
- [ ] Manual E2E: run a subprocess/PTY/ACP command with `ROKO_SERVE_URL`, observe events in SSE.
- [ ] Demo-running docs no longer describe A5/A6/A8 as unimplemented.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
| 2026-05-05 | wp-arch2 audit | Created hardening task after audit found event ingest, HttpEventSink, PTY env, and ACP bridge implemented but under-tested and stale in docs. |
