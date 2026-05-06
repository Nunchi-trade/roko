# Task 013: Add SSE Keepalive + Bound Replay Buffer

```toml
id = 13
title = "Add 8s SSE keepalive and .take(256) bound on SSE replay"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-serve/src/routes/sse.rs",
]
exclusive_files = ["crates/roko-serve/src/routes/sse.rs"]
estimated_minutes = 45
```

## Context

Two SSE issues:
1. No keepalive — connections drop silently after proxy/LB idle timeout.
2. SSE replay is unbounded — on first subscribe, the entire event history is replayed,
   causing memory spikes.

Sources:
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` — W14-B: SSE keepalive + replay unbounded
- `tmp/solutions/demo-running/archive/batches-executed/W14-B-serve-fixes.md` — original mechanical target: `KeepAlive::new().interval(Duration::from_secs(8)).text("keepalive")` and replay `.take(256)`

## Background

Read these paths before changing anything:
1. `crates/roko-serve/src/routes/sse.rs`
   - `routes()` exposes `/events` and `/sse`.
   - `sse_handler(...)` reads `Last-Event-ID`, replays from `state.state_hub`, chains live broadcast events, and returns `Sse<_>`.
2. `crates/roko-serve/src/routes/mod.rs`
   - `build_router(...)` merges `sse::routes()` into the API router and nests it under `/api`, so the runtime endpoints are `GET /api/events` and `GET /api/sse`.
   - There is also a separate workflow SSE helper in this file using `KeepAlive::default()`; that is not the target for this task unless a failing test proves the dashboard `/api/events` route is not the one being exercised.
3. `crates/roko-serve/src/state.rs`
   - `AppState::state_hub_for_workdir(...)` creates the `SharedStateHub` used by `sse_handler`.
4. `crates/roko-cli/src/main.rs`
   - `Command::Serve` constructs the `AppState` and starts `roko_serve::ServerBuilder`.

Current code may already contain the intended `.take(256)` and 8-second keepalive. If so, do not rewrite it; add/adjust regression coverage and leave the implementation unchanged except for minimal testability fixes.

## What to Change

1. In `sse_handler(...)`, ensure replay uses:
   - `state.state_hub.replay_from(last_event_id).into_iter().take(256)`.
   - The bound must be applied before mapping events into JSON/SSE frames.
2. Ensure the returned stream uses:
   - `Sse::new(stream::iter(replay).chain(live)).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(8)).text("keepalive"))`.
3. Add focused tests in `crates/roko-serve/src/routes/sse.rs` under `#[cfg(test)]` if coverage is missing:
   - A replay unit/helper test that seeds more than 256 state-hub events, connects with `Last-Event-ID: 0`, and asserts only 256 replay frames are emitted before live events.
   - A keepalive configuration test if Axum exposes the needed observable through the response; otherwise rely on an integration-style curl/manual verification and document why a direct unit assertion is not possible.
4. Keep the runtime path intact: `roko serve` -> `Command::Serve` in `crates/roko-cli/src/main.rs` -> `ServerBuilder` -> `routes::build_router` -> `sse::routes()` -> `sse_handler`.

## What NOT to Do

- Don't change the SSE event format.
- Don't change the event bus or TuiBridge.
- Don't add new event types.
- Don't edit `workflow_sse_from_adapter(...)` in `crates/roko-serve/src/routes/mod.rs` just because it uses `KeepAlive::default()`; this task is for the dashboard/state-hub SSE route.
- Don't increase the state-hub ring buffer size to hide the replay problem.
- Don't replace `Last-Event-ID` handling or change event IDs.

## Wire Target

```bash
# Start serve, connect with curl, wait 10s — should see keepalive
cargo run -p roko-cli -- serve &
curl -N http://localhost:6677/api/events 2>&1 | head -20
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test -p roko-serve sse`
- [ ] `cargo test --workspace`
- [ ] SSE connection receives keepalive comments within 10 seconds
- [ ] Replay is bounded (`.take(256)` or equivalent)
- [ ] `rg -n 'take\\(256\\)|Duration::from_secs\\(8\\)|text\\("keepalive"\\)' crates/roko-serve/src/routes/sse.rs`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
