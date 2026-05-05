# Roko HTTP API Reference

## What is this API?

Roko runs a local HTTP control plane — `roko serve` — that exposes everything the system is doing. Dashboards, external tools, CI scripts, and other agents can all talk to it.

**The mental model:** Roko reads product requirement documents (PRDs), generates implementation plans, dispatches Claude agents to execute tasks, and validates results through a gate pipeline (compile, test, clippy, etc.). All of that activity is observable and controllable through this API. You can watch plans execute in real time via SSE or WebSocket, query what agents are doing, inspect what the system has learned, trigger new work, and manage configuration.

```
+------------------+         +-----------------------------+
|  Your client     |  HTTP   |  roko-serve  (port 6677)    |
|  (dashboard,     | ------> |  REST + SSE + WebSocket     |
|   CI, scripts)   |  SSE    |                             |
|                  | <------ |  AuthMiddleware             |
+------------------+  WS     |  SecretScrubber             |
                             |  TraceLayer                 |
                             +--------------+--------------+
                                            |
                             +--------------v--------------+
                             |  StateHub                   |
                             |  (ring buffer + broadcast)  |
                             +--------------+--------------+
                                            |
                    +---------------+-------+-------+----------------+
                    |               |               |                |
             +------v------+ +------v------+ +------v------+ +-------v------+
             |  SSE stream | |  WebSocket  | | HTTP routes | |  Orchestrator|
             |  /api/events| |  /ws        | |  /api/...   | |  (plans,     |
             +-------------+ +-------------+ +-------------+ |   agents,    |
                                                             |   gates)     |
                                                             +--------------+
```

**Base URL:** `http://127.0.0.1:6677` (default)

**All API routes live under `/api/`.** Exceptions: `/health` (liveness probe), `/webhook/*` (inbound webhooks), and `/runs/{id}` (shareable run pages) are outside `/api/` and always public.

---

## Table of Contents

1. [Quick Start](#quick-start)
2. [Authentication](#authentication)
3. [Middleware and Error Handling](#middleware-and-error-handling)
4. [Real-Time Streams — SSE and WebSocket](#real-time-streams)
5. [StateHub Push Pattern](#statehub-push-pattern)
6. [Health and Status](#health-and-status)
7. [Plans](#plans)
8. [One-Shot Runs](#one-shot-runs)
9. [Dashboard Runs](#dashboard-runs)
10. [PRDs](#prds)
11. [Agents — Control Plane](#agents-control-plane)
12. [Agent Fleet Aggregation](#agent-fleet-aggregation)
13. [Gates](#gates)
14. [Episodes and Signals](#episodes-and-signals)
15. [Metrics](#metrics)
16. [Learning and Adaptation](#learning-and-adaptation)
17. [Benchmarks](#benchmarks)
18. [Dreams](#dreams)
19. [Knowledge (Neuro)](#knowledge-neuro)
20. [Research](#research)
21. [Jobs](#jobs)
22. [Deployments](#deployments)
23. [Inference Gateway](#inference-gateway)
24. [Providers and Models](#providers-and-models)
25. [Config](#config)
26. [Subscriptions and Workflows](#subscriptions-and-workflows)
27. [Heartbeats](#heartbeats)
28. [Secrets](#secrets)
29. [Chain](#chain)
30. [Webhooks](#webhooks)
31. [Terminal](#terminal)
32. [OpenAPI](#openapi)
33. [Per-Agent Sidecar API](#per-agent-sidecar-api)

---

## Quick Start

### 1. Start the server

```bash
# Default: binds to 127.0.0.1:6677, reads roko.toml from current directory
roko serve

# Custom bind and port
roko serve --bind 0.0.0.0 --port 8080

# Enable API key authentication
roko serve --api-key sk-my-secret-key

# Enable PTY terminal (disabled by default for security)
roko serve --enable-terminal
```

### 2. Verify it's running

```bash
curl http://127.0.0.1:6677/health
# {"status":"ok"}

curl http://127.0.0.1:6677/api/health
# {"status":"ok","version":"0.1.0","uptime_secs":3,"active_plans":0,...}
```

### 3. Watch the event stream

Open a second terminal and subscribe to real-time events before you trigger any work:

```bash
curl -N http://127.0.0.1:6677/api/events
```

### 4. Trigger a one-shot run

```bash
curl -X POST http://127.0.0.1:6677/api/run \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Add a unit test for the parser module"}'
# {"id":"run-uuid"}
```

Watch the SSE stream — you'll see `task_started`, `agent_spawned`, `gate_result`, and `task_completed` events flow through as the agent works.

### 5. Relevant `roko.toml` keys

```toml
[server]
bind = "127.0.0.1"
port = 6677

[serve]
terminal_enabled = false
cors_origins = []

[serve.auth]
enabled = false
api_key = ""
privy_app_id = ""   # optional, for Privy JWT dashboard auth

[[serve.auth.api_keys]]
name = "ci-bot"
key_hash = "<sha256-hex>"
scope = "agent:write"
expires_at = "2027-01-01T00:00:00Z"  # optional
```

---

## Authentication

Authentication is **opt-in**. When `serve.auth.enabled = false` (the default), all routes are open. Enable it when you expose the server beyond localhost.

When enabled, all `/api/*` routes require a credential. The `/health` (top-level), `/webhook/*`, and `/runs/{id}` routes are always public.

<details>
<summary>Credential sources and scope enforcement</summary>

### Credential sources (checked in order)

| Header | Format | Notes |
|---|---|---|
| `X-Api-Key` | plaintext key | Matched via SHA-256 hash. Sets `X-Auth-Method: api_key`. |
| `Authorization: Bearer <token>` | API key | Falls back from named keys to legacy `api_key`. Sets `X-Auth-Method: bearer`. |
| `Authorization: Bearer <jwt>` | Privy JWT | 3-segment base64url structure validated against JWKS cache. Sets `X-Auth-Method: jwt`. |
| `Authorization: Bearer <token>` | Agent token | Issued via `POST /api/agents/{id}/token`. Scope: `agent:write`. |

On success, an `X-Auth-Method` response header is set (`api_key`, `bearer`, or `jwt`) and an `AuthContext` is injected into request extensions.

### Scope enforcement

For mutating requests (POST/PUT/PATCH/DELETE), the caller's scope is checked against the required scope for the route:

| Route prefix | Required scope |
|---|---|
| GET/HEAD/OPTIONS | `read` (always allowed) |
| `/api/secrets`, `/api/config`, `/api/api-keys` | `admin` |
| `/api/agents/*` | `agent:write` |
| `/api/plans/*`, `/api/prd*` | `plan:write` |
| all other POST/PUT/PATCH/DELETE | `read` |

Scope hierarchy: `admin` > `agent:write` > `plan:write` > `read`.

### Error responses

```json
{ "code": "unauthorized", "message": "missing X-Api-Key header or Authorization bearer token", "status": 401 }
{ "code": "insufficient_scope", "message": "scope 'read' is not sufficient for 'admin' on POST /api/secrets", "status": 403 }
```

</details>

---

## Middleware and Error Handling

### Secret scrubbing

All `/api/*` responses pass through a **secret-scrubbing middleware** (`scrub_secrets`) that automatically redacts API key patterns (Anthropic keys, GitHub PATs, etc.) from JSON and text response bodies. Binary content types (`image/*`, `application/octet-stream`) pass through unchanged. The scrubber operates on responses up to 16 MiB.

### CORS

Configured via `serve.cors_origins`. If empty, `CorsLayer::permissive()` is used. All routes include request tracing via `TraceLayer`.

### Error format

All handlers return errors as JSON:

```json
{
  "code": "not_found",
  "message": "plan 'missing-plan' not found",
  "status": 404,
  "details": {}
}
```

Common error codes: `not_found` (404), `bad_request` (400), `unauthorized` (401), `forbidden` (403), `internal` (500), `insufficient_scope` (403).

---

## Real-Time Streams

Roko has two complementary push mechanisms. Use **SSE** for simple read-only dashboards and **WebSocket** when you need filtering, backpressure control, or bidirectional communication.

### Server-Sent Events (SSE)

SSE is the simplest way to watch Roko in real time. Open a connection and events arrive as newline-delimited JSON frames. On reconnect, send `Last-Event-ID` to replay from where you left off.

**`GET /api/events`** and **`GET /api/sse`** — Main dashboard event stream.

On connect, the server replays retained events from the ring buffer starting at the sequence number in `Last-Event-ID` (defaults to 0), then streams live events.

```
id: <monotonic-seq>
data: {"type":"task_started","plan_id":"...","task_id":"...","description":"..."}
```

Each `data:` frame is a JSON-serialized `DashboardEvent`. The connection is kept alive with periodic pings.

**`GET /api/workflow/events`** — `RuntimeEvent`-typed SSE stream for the workflow engine.

```
event: <kind>
data: {"kind":"...","...event fields..."}
```

<details>
<summary>Example: watching events from curl</summary>

```bash
# Connect and watch all events (Ctrl-C to stop)
curl -N http://127.0.0.1:6677/api/events

# Resume from sequence 42 after a disconnect
curl -N -H "Last-Event-ID: 42" http://127.0.0.1:6677/api/events

# In JavaScript (browser or Node)
const es = new EventSource('http://127.0.0.1:6677/api/events');
es.onmessage = (e) => {
  const event = JSON.parse(e.data);
  console.log(event.type, event);
};
```

</details>

### WebSocket

The WebSocket endpoint supports filtering and backpressure control. The server replays the full event backlog from the ring buffer on connect, then streams live `ServerEvent` payloads as JSON text frames.

**`GET /ws`** and **`GET /roko-ws`**

After connecting, optionally send a JSON control message to narrow what you receive:

```json
{
  "subscribe": ["projection:gate_pipeline", "topic:agent.*"],
  "cursor": 42,
  "back_pressure": "at_most_once"
}
```

<details>
<summary>Control message fields and filter syntax</summary>

| Field | Type | Description |
|---|---|---|
| `subscribe` | `string[]` | Filter strings. Empty = accept all. Supports plain type substrings (`"agent"`), channel prefixes (`"projection:<name>"`, `"topic:<pattern>"`, `"engram-stream:<name>"`), and glob wildcard suffix (`"agent.*"`). |
| `cursor` | `u64` | Sequence number to replay from (for catchup on reconnect). |
| `back_pressure` | `"at_most_once"` \| `"coalesce"` \| `"resume_required"` | Delivery semantics (default: `at_most_once`). |

**Filter examples:**
- `"agent"` — all events whose type contains "agent"
- `"topic:plan.*"` — all events published to topics matching `plan.*`
- `"projection:gate_pipeline"` — events from the gate_pipeline projection channel
- `"BenchRunStarted"` — exact-match on the PascalCase event type

**Lag behavior:** If the server-side broadcast buffer overflows, lagged events are silently dropped. A warning is logged server-side at most every 5 seconds.

</details>

**Outgoing frames:** JSON-serialized `ServerEvent` objects. See the [StateHub Push Pattern](#statehub-push-pattern) section for the full event catalog.

**`GET /api/ws`** (aggregator variant) — Aggregates live event streams from all discovered agent sidecars, not just the control plane. Reconnects automatically when agents join or leave (refresh interval: 10s, reconnect delay: 2s).

---

## StateHub Push Pattern

The StateHub is the central nervous system for real-time state distribution. All orchestrator activity flows through it: plans start, tasks execute, agents produce output, gates pass or fail — every significant transition is a `DashboardEvent` published to the StateHub.

The StateHub maintains a bounded ring buffer (default: 512 events) and fans out to all SSE and WebSocket subscribers via a `tokio::sync::broadcast` channel. New subscribers get a replay of retained events on connect, so they can reconstruct the current state without polling.

```
Orchestrator
    |
    | publish(DashboardEvent)
    v
  StateHub
    | ring buffer (512 events)
    | broadcast channel
    |
    +---> SSE clients (/api/events)
    +---> WS clients (/ws)
    +---> HTTP snapshot (/api/statehub/snapshot)
```

### DashboardEvent catalog

All `type` tags use `snake_case` unless noted. Events flow from the orchestrator through the StateHub to SSE/WS clients.

<details>
<summary>Orchestrator events (plans, tasks, agents, gates)</summary>

| `type` | Key fields | Source |
|---|---|---|
| `plan_started` | `plan_id` | Orchestrator |
| `plan_completed` | `plan_id`, `success` (bool), `outcome` (str), `stats` (obj) | Orchestrator |
| `task_started` | `plan_id`, `task_id`, `title`, `phase` | Orchestrator |
| `task_completed` | `plan_id`, `task_id`, `outcome` | Orchestrator |
| `task_phase_changed` | `plan_id`, `task_id`, `old_phase`, `new_phase` | Orchestrator |
| `agent_spawned` | `agent_id`, `role`, `model` | Orchestrator |
| `agent_output` | `agent_id`, `content` | Orchestrator / run.rs |
| `agent_completed` | `agent_id`, `role`, `episode_id`, `passed` | Orchestrator |
| `gate_result` | `plan_id`, `task_id`, `gate`, `rung` (u32), `passed` | Orchestrator |
| `phase_transition` | `plan_id`, `from`, `to` | Orchestrator |
| `efficiency_event` | `plan_id`, `task_id`, `agent_id`, `cost_usd`, `tokens`, `duration_ms` | Orchestrator |
| `episode_recorded` | `agent_id`, `role`, `episode_id`, `passed` | Orchestrator |
| `task_output_appended` | `task_id`, `lines` (str[]) | run.rs |
| `event_log_entry` | `timestamp_ms`, `event_type`, `plan_id`, `task_id`, `message` | run.rs |

</details>

<details>
<summary>Learning and subsystem events</summary>

| `type` | Key fields | Source |
|---|---|---|
| `cascade_router_updated` | router snapshot fields | Learning subsystem |
| `gate_thresholds_updated` | threshold map | Learning subsystem |
| `experiment_winners_updated` | experiment data | Learning subsystem |
| `c_factor_trend_updated` | trend data | Learning subsystem |
| `marketplace_jobs_updated` | jobs array | Jobs subsystem |
| `atelier_prds_updated` | PRD list | PRD subsystem |
| `knowledge_entries_updated` | entries array | Neuro subsystem |
| `job_execution_started` | `job_id`, `job_type`, `agent_id` | Jobs subsystem |
| `job_progress` | `job_id`, `percent`, `message` | Jobs subsystem |
| `diagnosis` | diagnostic fields | Conductor |
| `error` | `message` | Various |

</details>

### ServerEvent catalog (WebSocket / event bus)

The WebSocket and event bus carry a superset of dashboard events. Additional types include inference lifecycle, deployments, jobs, chain, and system events.

<details>
<summary>Full ServerEvent type catalog</summary>

| `type` | Key fields |
|---|---|
| `agent_output` | `agent_id`, `run_id?`, `content`, `done`, `metadata?` |
| `agent_trace` | `agent_id`, `run_id?`, `content`, `tool_calls?`, `reasoning?`, `usage?`, `done` |
| `agent_started` | `agent_id` |
| `agent_stopped` | `agent_id`, `reason` |
| `run_started` | `run_id`, `prompt_preview` |
| `run_completed` | `run_id`, `success` |
| `operation_started` | `op_id`, `kind` |
| `operation_completed` | `op_id`, `kind`, `success` |
| `inference_started` | `request_id`, `model`, `agent_id`, `auto_routed` |
| `inference_completed` | `request_id`, `model`, `agent_id`, `input_tokens`, `output_tokens`, `cost_usd`, `duration_ms` |
| `inference_failed` | `request_id`, `model`, `agent_id`, `error` |
| `deployment_created` | `id`, `name` |
| `deployment_ready` | `id`, `url` |
| `deployment_failed` | `id`, `reason` |
| `deployment_torn_down` | `id` |
| `job_created` | `job` (full job object) |
| `job_updated` | `job` |
| `job_transitioned` | `job_id`, `from`, `to`, `assigned_to?` |
| `job_posted_to_candidate` | `job_id`, `agent_id`, `reward` |
| `job_submitted` | `job_id`, `agent_id` |
| `job_evaluated` | `job_id`, `accepted`, `feedback` |
| `job_state_changed` | `job_id`, `from`, `to` |
| `job_execution_started` | `job_id`, `job_type`, `agent_id` |
| `job_progress` | `job_id`, `percent` (u8), `message` |
| `job_agent_output` | `job_id`, `agent_id`, `content`, `done` |
| `chain_triage_result` | `job_id`, `event_count`, `anomaly_count`, `summary` |
| `worker_task_started` | `deployment_id`, `task_id` |
| `worker_task_completed` | `deployment_id`, `task_id`, `success` |
| `heartbeat_received` | `sender_id`, `active_tasks`, `active_agents` |
| `heartbeat` | `agent_id`, `block_number?` |
| `somatic_marker_fired` | `plan_id`, `task_id`, `valence`, `intensity`, `source_episodes`, `strategy_param` |
| `webhook_received` | `signal` (full Engram) |
| `vision_loop_iteration` | `run_id`, `iteration`, `score`, `notes` |
| `vision_loop_completed` | `run_id`, `iterations`, `best_score`, `stop_reason` |
| `config_reloaded` | `applied_sections` (str[]), `restart_required` (str[]) |
| `strategy_reloaded` | `goals_count`, `tactics_count` |
| `BenchRunStarted` | `bench_id`, `suite_id`, `total_tasks` |
| `BenchTaskStarted` | `bench_id`, `task_id`, `task_name`, `task_index`, `total_tasks` |
| `BenchTaskCompleted` | `bench_id`, `task_id`, `result` (BenchTaskResult) |
| `BenchLearningEvent` | `bench_id`, `task_id`, `playbooks_created`, `anti_patterns_created`, `total_playbooks`, `total_anti_patterns` |
| `BenchProgress` | `bench_id`, `completed`, `total`, `cost_so_far` |
| `BenchRunCompleted` | `bench_id`, `summary` (BenchRunSummary) |
| `server_shutdown` | — |
| `error` | `message` |

**Note:** `BenchRunStarted`, `BenchTaskStarted`, `BenchTaskCompleted`, `BenchLearningEvent`, `BenchProgress`, and `BenchRunCompleted` are serialized with their exact PascalCase type tags. All other events use `snake_case`.

</details>

---

## Health and Status

These routes tell you whether the server is alive, what's currently running, and how the overall system is faring. Start here when debugging.

| Method | Path | Description |
|---|---|---|
| GET | `/health` | Bare liveness probe — always public, no `/api/` prefix |
| GET | `/api/health` | Rich health check with telemetry |
| GET | `/api/status` | Session overview and supervised processes |
| GET | `/api/dashboard` | Dashboard scaffold from the runtime |
| GET | `/api/operations/{id}` | Look up a background operation by ID |
| GET | `/api/relay/health` | Relay connection diagnostics |
| GET | `/api/parity` | TUI / HTTP API / CLI feature coverage matrix |
| GET | `/api/retention` | Retention policies and violations for `.roko/` data |
| GET | `/api/statehub/snapshot` | Current StateHub projection snapshot |
| GET | `/api/statehub/events` | Bounded replay from the ring buffer |
| GET | `/api/truth_map` | Entity-to-canonical-source registry |

<details>
<summary>GET /health — liveness probe</summary>

Handler: `top_level_health` (no `/api/` prefix, always public)

Returns `200 OK` unconditionally. Use this for load balancer health checks.

```json
{ "status": "ok" }
```

</details>

<details>
<summary>GET /api/health — rich health check</summary>

Handler: `health::health`

```json
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_secs": 3600,
  "active_plans": 2,
  "active_agents": 5,
  "active_runs": 1,
  "providers": {
    "total": 3,
    "healthy": 3,
    "unhealthy": 0
  },
  "statehub": {
    "cursor": "0x1a4",
    "events_retained": 64,
    "snapshot": {
      "plans_active": 2,
      "tasks_active": 8,
      "agents_active": 5,
      "gates_passed": 42,
      "gates_failed": 3,
      "episodes_total": 127,
      "errors_total": 1,
      "cost_usd_total": 12.45
    }
  }
}
```

`status` is `"ok"` / `"degraded"` (some providers unhealthy) / `"down"` (all providers failed).

</details>

<details>
<summary>GET /api/status — session overview</summary>

Handler: `dashboard::session_status`

```json
{
  "session_id": "sess-abc123",
  "workdir": "/Users/will/dev/nunchi/roko/roko",
  "daemon_running": true,
  "signal_count": 1024,
  "episode_count": 42,
  "last_episode_passed": true,
  "supervised_processes": [...],
  "process_session_ledger": "/path/.roko/state/process-sessions.json",
  "process_sessions": { ... }
}
```

</details>

<details>
<summary>GET /api/operations/{id} — background operation status</summary>

Handler: `dashboard::operation_status`

Background operations (dream runs, plan generation, PRD drafting, etc.) are tracked by an operation UUID. Use this to poll their status.

```json
{ "id": "op-uuid", "kind": "dream_run", "status": "Running" }
```

Returns `404` if the operation ID is not found.

</details>

<details>
<summary>GET /api/statehub/events — ring buffer replay</summary>

Handler: `health::statehub_events`

Query the ring buffer directly (without establishing an SSE connection). Useful for one-shot polling or catching up after a gap.

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `after_seq` | `u64` | `0` | Return events with seq > this value |
| `limit` | `usize` | `256` | Max events to return (capped at 1024) |
| `run_id` | `string` | — | Filter by plan/run ID |
| `plan_id` | `string` | — | Filter by plan ID |
| `task_id` | `string` | — | Filter by task ID |
| `type` (alias: `event_type`) | `string` | — | Filter by event type tag |

```json
{
  "after_seq": 42,
  "limit": 256,
  "cursor": "0x1f7",
  "events": [
    {
      "seq": 43,
      "cursor": "0x2b",
      "ts_millis": 1745942400000,
      "event": { "type": "task_started", "plan_id": "...", "task_id": "...", ... }
    }
  ]
}
```

</details>

---

## Plans

Plans are the primary unit of work in Roko. A plan is a collection of tasks, each of which is executed by an agent and validated through the gate pipeline. Plans live in `.roko/plans/` as `tasks.toml` files and can be generated automatically from PRDs.

| Method | Path | Description |
|---|---|---|
| GET | `/api/plans` | List all plans |
| POST | `/api/plans` | Create a new plan |
| GET | `/api/plans/{id}` | Get full plan details |
| GET | `/api/plans/{id}/tasks` | List tasks for a plan |
| POST | `/api/plans/{id}/execute` | Execute a plan in the background |
| GET | `/api/plans/{id}/status` | Get execution status |
| POST | `/api/plans/{id}/pause` | Pause execution |
| POST | `/api/plans/{id}/resume` | Resume execution |
| GET | `/api/plans/{id}/gates` | Get gate results grouped by task |
| GET | `/api/plans/{id}/reviews` | List human reviews |
| POST | `/api/plans/{id}/tasks/{task_id}/review` | Submit a human review |
| GET | `/api/plans/{id}/tasks/{task_id}/diff` | Get code diff from a task agent |
| POST | `/api/plans/{id}/chat` | Send a chat message in plan context |
| POST | `/api/plans/{id}/estimate` | Estimate cost and duration |
| POST | `/api/plans/generate` | Generate a plan from a prompt |

<details>
<summary>Route details and request/response shapes</summary>

#### `GET /api/plans`

```json
{
  "plans": [
    { "id": "plan-1", "title": "...", "status": "...", "task_count": 5 }
  ]
}
```

#### `POST /api/plans`

```json
{ "title": "My Plan", "description": "...", "tasks": [] }
```

Response: `201 Created` with `{ "id": "plan-uuid" }`.

#### `POST /api/plans/{id}/execute`

Executes the plan in a background task. Response: `202 Accepted` with `{ "operation_id": "op-uuid" }`.

#### `GET /api/plans/{id}/status`

```json
{ "id": "...", "status": "running|paused|completed|failed", "progress": {...} }
```

#### `POST /api/plans/{id}/tasks/{task_id}/review`

```json
{ "approved": true, "comment": "Looks good" }
```

#### `POST /api/plans/{id}/estimate`

```json
{ "estimated_cost_usd": 2.50, "estimated_duration_mins": 15, "task_count": 8 }
```

#### `POST /api/plans/generate`

Generate a plan from a natural language prompt using an agent.

```json
{ "prompt": "Implement rate limiting for the API", "context": "..." }
```

Response: `202 Accepted` with `{ "operation_id": "op-uuid" }`.

</details>

<details>
<summary>Example: create and execute a plan</summary>

```bash
# Create a plan
curl -X POST http://127.0.0.1:6677/api/plans \
  -H "Content-Type: application/json" \
  -d '{"title":"Add rate limiting","description":"Rate limit the HTTP API","tasks":[]}'

# Execute it
curl -X POST http://127.0.0.1:6677/api/plans/plan-uuid/execute

# Poll status
curl http://127.0.0.1:6677/api/plans/plan-uuid/status

# Or watch SSE stream (already open) for real-time updates
```

</details>

---

## One-Shot Runs

A one-shot run is the simplest way to get Roko to do something: pass a prompt, and Roko executes one full cycle — compose system prompt → dispatch agent → run gate pipeline → persist episode. This corresponds to `roko run "<prompt>"` on the CLI.

| Method | Path | Description |
|---|---|---|
| POST | `/api/run` | Spawn a background run |
| GET | `/api/run/{id}/status` | Poll run status |

<details>
<summary>Route details and request/response shapes</summary>

#### `POST /api/run`

Executes one prompt through the universal compose → agent → gate → persist loop. Emits `RunStarted`, `TaskStarted`, `AgentSpawned`, and `RunCompleted` events to the StateHub.

```json
{
  "prompt": "Add a unit test for the parser",
  "workdir": "/optional/override/path"
}
```

`prompt` must be non-blank.

Response: `202 Accepted`
```json
{ "id": "run-uuid" }
```

#### `GET /api/run/{id}/status`

```json
{
  "id": "run-uuid",
  "prompt": "Add a unit test for the parser",
  "status": "running|completed|failed",
  "success": true,
  "output_text": "Created test in src/parser_test.rs...",
  "error": null,
  "finished": true
}
```

Returns `404` if the run ID is not in the active runs map.

</details>

<details>
<summary>Example: fire-and-forget with status polling</summary>

```bash
# Start the run
RUN_ID=$(curl -s -X POST http://127.0.0.1:6677/api/run \
  -H "Content-Type: application/json" \
  -d '{"prompt":"Write a summary of the codebase architecture"}' | jq -r .id)

# Poll until done
while true; do
  STATUS=$(curl -s http://127.0.0.1:6677/api/run/$RUN_ID/status)
  FINISHED=$(echo $STATUS | jq -r .finished)
  if [ "$FINISHED" = "true" ]; then
    echo $STATUS | jq .
    break
  fi
  sleep 2
done
```

</details>

---

## Dashboard Runs

Handler module: `routes/runs.rs`

`GET /api/dashboard/runs` summarizes runs from the runtime event log (`.roko/runtime-events.jsonl`). This is what the TUI's run history view reads.

<details>
<summary>Response shape</summary>

```json
{
  "runs": [
    {
      "run_id": "run-1",
      "template": "implementer",
      "prompt": "Add unit tests...",
      "current_phase": "gate",
      "phases_visited": ["compose", "dispatch", "gate"],
      "gates_passed": 3,
      "gates_failed": 1,
      "agents_spawned": 2,
      "is_complete": true,
      "outcome": "success"
    }
  ]
}
```

</details>

---

## PRDs

PRDs (Product Requirement Documents) are the top of Roko's self-hosting funnel. You capture an idea, Roko drafts a PRD using an agent, generates an implementation plan, and executes it. These routes expose the full lifecycle.

| Method | Path | Description |
|---|---|---|
| GET | `/api/prds` | List all PRDs |
| GET | `/api/prds/status` | Coverage report by lifecycle stage |
| POST | `/api/prds/ideas` | Capture a new work-item idea |
| GET | `/api/prds/{slug}` | Get a PRD by slug |
| POST | `/api/prds/{slug}/draft` | Agent-driven PRD drafting |
| POST | `/api/prds/{slug}/promote` | Promote to planned/approved status |
| POST | `/api/prds/{slug}/plan` | Generate implementation plan from PRD |
| POST | `/api/prd/consolidate` | Scan for duplicates and gaps |
| POST | `/api/prds/consolidate` | (alias) |

<details>
<summary>Route details and request/response shapes</summary>

#### `GET /api/prds`

```json
[
  { "slug": "rate-limiting", "title": "Add Rate Limiting", "status": "draft", "created_at": "..." }
]
```

#### `GET /api/prds/status`

Coverage report: counts by lifecycle stage (`idea`, `draft`, `planned`, `implemented`).

#### `POST /api/prds/ideas`

```json
{ "title": "Add retry logic to the HTTP client", "description": "..." }
```

Response: `201 Created` with the new PRD slug.

#### `GET /api/prds/{slug}`

Reads the markdown file from `.roko/prd/{slug}.md`.

```json
{ "slug": "rate-limiting", "title": "...", "content": "...", "status": "draft" }
```

#### `POST /api/prds/{slug}/draft`

Spawns a background agent that enriches the idea into a full PRD with requirements, acceptance criteria, and technical design.

Response: `202 Accepted` with `{ "operation_id": "op-uuid" }`.

#### `POST /api/prds/{slug}/promote`

Triggers `AtelierPrdsUpdated` StateHub event.

```json
{ "status": "approved" }
```

#### `POST /api/prds/{slug}/plan`

Generates a `tasks.toml` from the PRD using a background agent. If `prd.auto_plan` is enabled in `roko.toml`, this is triggered automatically on PRD publish.

Response: `202 Accepted` with `{ "operation_id": "op-uuid" }`.

</details>

<details>
<summary>Example: full PRD lifecycle via API</summary>

```bash
# 1. Capture an idea
curl -X POST http://127.0.0.1:6677/api/prds/ideas \
  -H "Content-Type: application/json" \
  -d '{"title":"Add request deduplication","description":"Deduplicate concurrent identical requests"}'
# returns slug: "add-request-deduplication"

# 2. Draft the PRD (agent-driven)
curl -X POST http://127.0.0.1:6677/api/prds/add-request-deduplication/draft

# 3. Generate implementation plan
curl -X POST http://127.0.0.1:6677/api/prds/add-request-deduplication/plan

# 4. Check status
curl http://127.0.0.1:6677/api/prds/status
```

</details>

---

## Agents — Control Plane

Handler module: `routes/agents.rs`

These routes manage the lifecycle of agent processes: creating them, starting and stopping them, sending them messages, and inspecting their state. The control plane maintains a registry of "discovered agents" — both locally supervised processes and self-registered remote sidecars.

| Method | Path | Description |
|---|---|---|
| GET | `/api/managed-agents` | List all managed agent processes |
| POST | `/api/agents/register` | Register a remote sidecar with the control plane |
| POST | `/api/agents/create` | Create and spawn a new agent from manifest |
| GET | `/api/agents/{id}` | Get agent details |
| GET | `/api/agents/{id}/profile` | Get the model profile assigned to this agent |
| POST | `/api/agents/{id}/stop` | Stop a running agent |
| GET | `/api/agents/{id}/episodes` | Get episodes attributed to this agent |
| GET | `/api/agents/{id}/logs` | Proxy logs from the agent sidecar |
| POST | `/api/agents/{id}/message` | Send a prompt to an agent inline |
| POST | `/api/agents/{id}/start` | Start or restart a stopped agent |
| POST | `/api/agents/{id}/restart` | Restart an agent |
| GET | `/api/agents/{id}/token` | Get token status |
| POST | `/api/agents/{id}/token` | Issue or rotate an agent bearer token |

<details>
<summary>Route details and request/response shapes</summary>

#### `GET /api/managed-agents`

Merges locally supervised processes with self-registered remote sidecars. Used by the dashboard fleet roster.

```json
[
  {
    "id": "agent-uuid",
    "label": "implementer-1",
    "status": "running",
    "process_id": 12345,
    "model": "claude-sonnet-4-6",
    "role": "Implementer",
    "capabilities": ["messaging", "research"],
    "sidecar_url": "http://127.0.0.1:7001",
    "last_heartbeat_ms": 1745942400000,
    "heartbeat_lag_secs": 5
  }
]
```

#### `POST /api/agents/register`

```json
{
  "agent_id": "agent-uuid",
  "label": "my-agent",
  "sidecar_url": "http://10.0.0.5:7001",
  "capabilities": ["messaging"],
  "model": "claude-sonnet-4-6",
  "role": "Reviewer"
}
```

Response: `200 OK` with registration record.

#### `POST /api/agents/create`

```json
{
  "name": "implementer-2",
  "domain": "rust",
  "model": "claude-sonnet-4-6",
  "role": "Implementer"
}
```

Response: `202 Accepted`.

#### `POST /api/agents/{id}/message`

Sends a prompt to an agent's sidecar and returns the response inline. 30-second timeout.

```json
{ "prompt": "Explain what you're working on", "context": {} }
```

Response: agent response object (mirrors sidecar `/message` response).

#### `GET /api/agents/{id}/episodes`

**Query parameters:** `limit: usize` (optional)

#### `POST /api/agents/{id}/token`

Issues (or rotates) a bearer token for an agent. The token hash is stored in `DiscoveredAgent.token_hash`. Tokens expire after 24 hours by default.

```json
{ "token": "roko-agent-<uuid>", "expires_at": "2026-04-30T00:00:00Z" }
```

</details>

---

## Agent Fleet Aggregation

Handler module: `routes/aggregator.rs`

These routes aggregate data from all discovered agent sidecars in parallel. Where the control plane routes above manage the lifecycle of agents, these routes pull live operational data — stats, skills, tasks, knowledge — from all agents at once and present a unified view.

| Method | Path | Description |
|---|---|---|
| GET | `/api/agents` | List all discovered agents with live stats (TTL-cached 30s) |
| GET | `/api/agents/topology` | Agent topology as a graph |
| GET | `/api/agents/{id}/stats` | Live stats from a specific agent's sidecar (TTL-cached 5s) |
| GET | `/api/agents/{id}/skills` | Skill manifest from sidecar |
| GET | `/api/agents/{id}/heartbeat` | Latest heartbeat |
| GET | `/api/agents/{id}/trace` | Proxy agent trace logs |
| GET | `/api/predictions/sessions` | Prediction sessions across all agents (TTL-cached 10s) |
| GET | `/api/predictions/sessions/{id}` | Specific prediction session |
| GET | `/api/predictions/claims` | Prediction claims across all agents |
| GET | `/api/predictions/calibration/{agent_id}` | Prediction calibration for an agent |
| GET | `/api/knowledge/entries` | Knowledge entries from neuro store (TTL-cached 30s) |
| GET | `/api/knowledge/edges` | Knowledge graph edges |
| GET | `/api/knowledge/search` | Search knowledge (`?q=<query>`) |
| GET | `/api/knowledge/kinds` | Known knowledge kinds |
| GET | `/api/tasks` | Tasks from all sidecars (TTL-cached 30s) |
| GET | `/api/tasks/stats` | Aggregate task statistics |
| GET | `/api/tasks/{id}` | Specific task by ID |

<details>
<summary>Route details and query parameters</summary>

#### `GET /api/agents`

**Query parameters:** `owner: string` — filter by owner label

**Response:** Paginated response: `{ items, total, offset, limit, has_more }` where each item is an `AgentCard` extended with live stats.

#### `GET /api/agents/{id}/trace`

**Query parameters:** `limit: usize` (default: 50), `offset: usize` (default: 0)

#### `GET /api/tasks`

**Query parameters:**
- `state: string` — filter by task state
- `kind: string` — filter by task kind
- `assignee: string` — filter by assignee agent
- `limit: usize` (default: 50)
- `offset: usize` (default: 0)

**Response:** `{ items, total, offset, limit, has_more }`

</details>

---

## Gates

Handler module: `routes/status/gates.rs`

Roko validates every agent's output through a 7-rung gate pipeline before accepting it. The gates run sequentially: compile → clippy → test → diff → fmt → custom → judge. These routes let you inspect pass rates, trends, and the history of every gate check.

Rung numbers: `0=compile`, `1=clippy`, `2=test`, `3=diff`, `4=fmt`, `5=custom`, `6=judge`.

| Method | Path | Description |
|---|---|---|
| GET | `/api/gates/summary` | Aggregate pass rates and durations per gate, plus rung breakdown |
| GET | `/api/gates/history` | Recent gate verdicts (flat or waterfall format) |
| GET | `/api/gates/{gate_name}/history` | Time series for one specific gate |

<details>
<summary>GET /api/gates/summary — response shape</summary>

Aggregates gate verdicts from canonical projections (`RuntimeProjectionSet`).

```json
{
  "compile": {
    "total_runs": 42,
    "pass_rate": 0.857,
    "avg_duration_ms": 4200.0,
    "last_run": { ... }
  },
  "test": { ... },
  "rungs": [
    { "rung": 0, "passed_runs": 36, "failed_runs": 6, "total_runs": 42, "pass_rate": 0.857 },
    { "rung": 1, "passed_runs": 40, "failed_runs": 2, "total_runs": 42, "pass_rate": 0.952 }
  ]
}
```

</details>

<details>
<summary>GET /api/gates/history — flat and waterfall formats</summary>

**Query parameters:**

| Parameter | Type | Description |
|---|---|---|
| `gate` | `string` | Filter to one gate name |
| `limit` | `usize` | Max results |
| `format` | `"waterfall"` | Return waterfall shape (grouped by `task_id`) |

**Default (flat) response:**
```json
{
  "sources": ["/path/.roko/engrams.jsonl", "/path/.roko/events.jsonl"],
  "total": 84,
  "limit": 100,
  "history": [
    {
      "signal_id": "...",
      "created_at_ms": 1745942400000,
      "gate": "compile",
      "passed": true,
      "duration_ms": 3800,
      "plan_id": "plan-1",
      "task_id": "task-1",
      "rung": 0
    }
  ]
}
```

**Waterfall response** (`?format=waterfall`) — groups rungs by task, useful for timeline visualizations:

```json
[
  {
    "task_id": "task-1",
    "timestamp": 1745942400000,
    "rungs": [
      { "name": "compile", "rung": 0, "status": "passed", "duration_ms": 3800 },
      { "name": "test", "rung": 2, "status": "failed", "duration_ms": 12000 }
    ]
  }
]
```

</details>

---

## Episodes and Signals

Handler module: `routes/status/episodes.rs`

Episodes are the fundamental record of agent work. Each time an agent completes a task and the gate pipeline runs, an episode is recorded to `.roko/episodes.jsonl` with the agent's output, the gate results, cost, tokens, and a HDC fingerprint. Signals (`Engram` objects) are the raw event log from which episodes are derived.

| Method | Path | Description |
|---|---|---|
| GET | `/api/episodes` | Normalized episode proof rows |
| GET | `/api/signals` | Raw signals JSONL as JSON array (most recent first) |

<details>
<summary>Route details and query parameters</summary>

#### `GET /api/episodes`

**Query parameters:** `limit: usize`, `plan_id: string`, `task_id: string`, `gate: string`

Returns JSON array of episode objects from `.roko/episodes.jsonl`.

#### `GET /api/signals`

**Query parameters:** `limit: usize` — max results (default: 500, max: 500)

Returns JSON array of Engram signal objects from `.roko/engrams.jsonl`.

</details>

---

## Metrics

Handler module: `routes/status/metrics.rs`

These routes provide aggregated views over the data Roko has collected. Use them for dashboards, alerting, or inspecting system health over time.

| Method | Path | Description |
|---|---|---|
| GET | `/api/metrics` | Raw metric snapshots from the in-memory registry |
| GET | `/api/metrics/summary` | Aggregate execution and learning metrics |
| GET | `/api/metrics/success_rate` | Per-template success rate by trigger kind |
| GET | `/api/metrics/engagement` | Feedback acknowledgement ratio per template |
| GET | `/api/metrics/c_factor` | C-Factor, sub-metrics, per-agent contributions |
| GET | `/api/metrics/model_efficiency` | Cost per successful episode per model |
| GET | `/api/metrics/gate_rate` | Gate pass/total with trend delta |
| GET | `/api/metrics/experiments` | Best vs worst variant gap per experiment |
| GET | `/api/metrics/feedback_latency` | Median hours from action to gate feedback |
| GET | `/api/metrics/velocity` | Self-improvement velocity (Δ%/day) |
| GET | `/api/metrics/coverage` | Event bus subscription coverage |
| GET | `/api/metrics/prometheus` | Prometheus text exposition format |

<details>
<summary>GET /api/metrics/summary — dashboard overview</summary>

**Query parameters:** `period: string` — `"last_7_days"` (default), `"last_30_days"`, `"last_90_days"`, or `"last_N_days"`

```json
{
  "period": "last_7_days",
  "agents_run": 127,
  "success_rate": 0.874,
  "feedback_engagement_rate": 0.921,
  "avg_cost_per_episode_cents": 12,
  "experiments_active": 2,
  "best_experiment_lift": {
    "name": "system_prompt",
    "lift": 0.12,
    "winning": "Verbose"
  },
  "gate_pass_rate": 0.857,
  "self_improvement_velocity": 0.03,
  "c_factor": 0.71,
  "active_plans": 2,
  "top_templates": [
    { "name": "implementer", "runs": 45, "success_rate": 0.91 }
  ]
}
```

</details>

<details>
<summary>GET /api/metrics/c_factor — composite C-Factor</summary>

The C-Factor is Roko's composite self-improvement score — a weighted combination of gate pass rate, cost efficiency, speed, knowledge growth, and social/coordination metrics. Higher is better.

```json
{
  "source": {
    "composite_history_path": "/path/.roko/learn/c-factor.jsonl",
    "efficiency_events_path": "/path/.roko/learn/efficiency.jsonl",
    "composite_history_count": 24,
    "efficiency_event_count": 127
  },
  "composite": {
    "overall": 0.71,
    "computed_at": "2026-04-07T12:00:00Z",
    "episode_count": 127,
    "history_count": 24
  },
  "sub_metrics": {
    "gate_pass_rate": 0.857,
    "cost_efficiency": 0.72,
    "speed": 0.68,
    "information_flow_rate": 0.55,
    "first_try_rate": 0.79,
    "knowledge_growth": 0.42,
    "knowledge_integration_rate": 0.38,
    "hdc_diversity": 0.61,
    "convergence_velocity": 0.53,
    "turn_taking_equality": 0.74,
    "social_perceptiveness": 0.69
  },
  "per_agent": [
    {
      "agent_id": "agent-a",
      "episode_count": 45,
      "without_agent_overall": 0.58,
      "contribution_score": 0.13,
      "dispatch_bias": "prefer_cheaper"
    }
  ],
  "per_fleet": { "plan_count": 8, "agent_count": 3, "observation_count": 127 }
}
```

</details>

<details>
<summary>GET /api/metrics/model_efficiency — cost per successful episode</summary>

```json
{
  "source": "/path/.roko/learn/cascade-router.json",
  "current_stage": "confidence",
  "total_observations": 80,
  "models": [
    {
      "model": "claude-sonnet-4-6",
      "total_episodes": 50,
      "successful_episodes": 45,
      "total_cost_usd": 12.50,
      "cost_per_successful_episode_usd": 0.278,
      "success_rate": 0.9
    }
  ]
}
```

</details>

<details>
<summary>GET /api/metrics/prometheus — Prometheus scrape endpoint</summary>

Content-Type: `text/plain; version=0.0.4; charset=utf-8`

| Metric | Type | Description |
|---|---|---|
| `roko_uptime_seconds` | gauge | Seconds since roko-serve started |
| `roko_agents_active` | gauge | Currently active agents |
| `roko_plans_active` | gauge | Currently executing plans |
| `roko_plans_completed_total` | counter | Plans completed successfully |
| `roko_plans_failed_total` | counter | Plans that failed |
| `roko_tasks_completed_total` | counter | Tasks completed |
| `roko_tasks_failed_total` | counter | Tasks that failed |
| `roko_tasks_active` | gauge | Currently executing tasks |
| `roko_gate_pass_total` | counter | Gate checks that passed |
| `roko_gate_fail_total` | counter | Gate checks that failed |
| `roko_errors_total` | counter | Error events recorded |
| `roko_episodes_total` | counter | Episodes recorded |

</details>

<details>
<summary>Other metrics routes — shapes</summary>

#### `GET /api/metrics/success_rate`

```json
{
  "templates": [
    {
      "template": "implementer",
      "triggers": [
        {
          "trigger_kind": "plan_dispatch",
          "successful_episodes": 38,
          "total_episodes": 42,
          "success_rate": 0.905
        }
      ]
    }
  ]
}
```

#### `GET /api/metrics/engagement`

```json
{
  "templates": [
    {
      "template": "reviewer",
      "acknowledged_actions": 30,
      "total_actions": 35,
      "engagement_rate": 0.857
    }
  ]
}
```

#### `GET /api/metrics/gate_rate`

```json
{
  "summary": { "compile": { ... }, "test": { ... } },
  "history": { ... },
  "evidence": { ... }
}
```

#### `GET /api/metrics/experiments`

```json
{
  "source": "/path/.roko/learn/experiments.json",
  "experiments": [
    {
      "experiment_id": "exp-1",
      "section_name": "system_prompt",
      "status": "running",
      "best_variant": { "id": "baseline", "name": "Baseline", "success_rate": 0.8, "trials": 10 },
      "worst_variant": { "id": "verbose", "name": "Verbose", "success_rate": 0.5, "trials": 10 },
      "metric_difference": 0.3
    }
  ]
}
```

#### `GET /api/metrics/feedback_latency`

```json
{ "sample_count": 42, "median_hours": 0.003 }
```

#### `GET /api/metrics/velocity`

```json
{
  "velocity": 0.042,
  "sample_count": 127,
  "evidence": { ... }
}
```

#### `GET /api/metrics/coverage`

```json
{
  "matched_events": 118,
  "unhandled_events": 9,
  "coverage": 0.929,
  "subscription_terms": ["plan", "task", "gate", "execution", "episode", "efficiency", "run", "operation", "deployment", "error", "server_shutdown", "agent"]
}
```

</details>

---

## Learning and Adaptation

Handler module: `routes/learning/mod.rs`, `learning/router_state.rs`, `learning/experiments.rs`

The Learning routes let you inspect how Roko adapts over time — which models it's learned to prefer, what patterns it has discovered from episodes, how gate thresholds have shifted, and which prompt experiments are running. All data originates from the `.roko/learn/` directory.

All routes have both `/learning/` and `/learn/` prefix forms (both are mounted).

| Method | Path | Description |
|---|---|---|
| GET | `/api/learning/efficiency` | Task-level cost and timing metrics |
| GET | `/api/learning/costs` | Canonical runtime cost projection |
| GET | `/api/learning/provider-outcomes` | Provider health shaped for dashboards |
| GET | `/api/learning/retries` | Retry attempt proof surface |
| GET | `/api/learning/runtime-feedback` | Joined feedback store overview |
| GET | `/api/learning/cascade-router` | Raw cascade router state |
| GET | `/api/learning/cascade` | Summarized model weights and routing stats |
| GET | `/api/learning/cost-tiers` | T0/T1/T2 routing distribution |
| GET | `/api/learning/experiments` | Full experiment store |
| GET | `/api/learning/adaptive-thresholds` | EMA-based adaptive gate thresholds |
| GET | `/api/learning/gate-thresholds` | Raw gate threshold data |
| GET | `/api/c-factor/trend` | C-Factor trend series |
| GET | `/api/executor/state` | Executor snapshot |

<details>
<summary>GET /api/learning/efficiency — cost and timing per task</summary>

Aggregates `.roko/learn/efficiency.jsonl`.

```json
{
  "total_cost": 24.50,
  "cost_per_task": 0.82,
  "tokens_per_task": 1240.0,
  "avg_task_duration": 4200.0,
  "cost_trend": [
    { "timestamp": "2026-04-07T12:00:00Z", "cost_usd": 0.85, "cumulative_cost_usd": 0.85 }
  ],
  "tasks": [
    { "plan_id": "plan-1", "task_id": "task-1", "timestamp": "...", "cost_usd": 0.85, "tokens": 1200, "duration_ms": 4100, "sequence": 0 }
  ],
  "evidence": { "state": "loaded", "sources": [...] }
}
```

</details>

<details>
<summary>GET /api/learning/cascade — model routing intelligence</summary>

Roko uses a CascadeRouter to learn which models work best. It starts with static weights, moves to confidence intervals, then UCB (Upper Confidence Bound) selection as it accumulates observations.

Cascade stages: `static` (<20 observations), `confidence` (<200), `ucb` (≥200).

```json
{
  "source": "/path/.roko/learn/cascade-router.json",
  "current_stage": "confidence",
  "model_weights": [
    {
      "model": "claude-sonnet-4-6",
      "trials": 50,
      "successes": 45,
      "pass_rate": 0.9,
      "upper_confidence_bound": 0.952,
      "normalized_weight": 0.68
    }
  ],
  "routing_stats": {
    "current_stage": "confidence",
    "total_observations": 80,
    "registered_models": 3,
    "observed_models": 2,
    "best_model": "claude-sonnet-4-6"
  },
  "recommended_models": [
    { "task_category": "implementation", "complexity_band": "standard", "recommended_model": "claude-sonnet-4-6", "weight": 0.68 }
  ]
}
```

</details>

<details>
<summary>GET /api/learning/cost-tiers — T0/T1/T2 distribution</summary>

T0=fast/haiku, T1=standard/sonnet, T2=premium/opus.

```json
{
  "T0": 10, "T1": 50, "T2": 20,
  "total": 80, "sample_count": 80,
  "t0_pct": 12.5, "t1_pct": 62.5, "t2_pct": 25.0
}
```

</details>

<details>
<summary>GET /api/learning/experiments — A/B prompt experiments</summary>

Roko runs A/B experiments on system prompt sections to learn which variants produce better outcomes.

```json
{
  "running_experiments": 1,
  "concluded_experiments": 2,
  "active_experiments": [
    {
      "experiment_id": "exp-1",
      "section_name": "system_prompt",
      "status": "running",
      "variants": [
        { "id": "baseline", "name": "Baseline", "section_name": "system_prompt", "active": true }
      ],
      "stats": {
        "baseline": { "trials": 10, "successes": 8 }
      }
    }
  ]
}
```

</details>

<details>
<summary>GET /api/learning/adaptive-thresholds — EMA gate thresholds</summary>

Gate thresholds adapt over time using Exponential Moving Average (EMA) of historical pass rates. This prevents thresholds from being too strict early on (when the system is learning) or too lenient later.

```json
{
  "source": "/path/.roko/learn/gate-thresholds.json",
  "tracked_rungs": 3,
  "thresholds": {
    "compile": { "mean_pass_rate": 0.857, "ema_threshold": 0.7, "rung_count": 42, "consecutive_passes": 7 },
    "test": { "mean_pass_rate": 0.91, "ema_threshold": 0.7, "rung_count": 38, "consecutive_passes": 12 }
  }
}
```

</details>

<details>
<summary>GET /api/learning/provider-outcomes — provider health</summary>

```json
{
  "providers": [
    {
      "name": "Anthropic",
      "status": "healthy",
      "models": ["claude-sonnet-4-6", "claude-haiku-4-5"],
      "success_rate": 0.98,
      "avg_latency_ms": 3200,
      "p95_latency_ms": 4800,
      "cost_per_1k_tokens": 0.42,
      "total_requests": 127,
      "errors_24h": 2,
      "last_error": ""
    }
  ]
}
```

Status: `"healthy"` (≥97% success rate), `"degraded"` (≥90%), `"unhealthy"` (<90%).

</details>

<details>
<summary>GET /api/c-factor/trend — C-Factor over time</summary>

**Query parameters:** `window: string` — `"24h"` (default, 24 hourly buckets) or `"7d"` (168 hourly buckets)

```json
{
  "trend": [
    { "bucket_start": "2026-04-07T11:00:00Z", "avg": 0.71, "min": 0.65, "max": 0.78, "samples": 3 }
  ],
  "woolley": {
    "turn_taking_equality": [0.74, 0.75, ...],
    "social_perceptiveness": [0.67, 0.68, ...],
    "citation_reciprocity": [0.72, 0.73, ...],
    "delivery_rate": [0.70, 0.71, ...],
    "hdc_diversity": [0.63, 0.64, ...]
  }
}
```

</details>

---

## Benchmarks

Handler module: `routes/bench.rs`

Benchmarks let you measure Roko's performance on defined task suites. Run the same suite across different model configurations, compare results side by side, and find the Pareto frontier between cost and performance. All events during a bench run are also streamed via SSE.

| Method | Path | Description |
|---|---|---|
| POST | `/api/bench/run` | Start a new benchmark run |
| POST | `/api/bench/runs` | (alias) |
| GET | `/api/bench/runs` | List bench runs |
| GET | `/api/bench/run/{id}` | Full bench run with all task results |
| GET | `/api/bench/runs/{id}` | (alias) |
| GET | `/api/bench/run/{id}/status` | Lightweight status poll |
| DELETE | `/api/bench/run/{id}` | Cancel or delete a run |
| POST | `/api/bench/runs/{id}/cancel` | Cancel a running run |
| GET | `/api/bench/runs/compare` | Compare multiple runs (`?ids=a,b`) |
| GET | `/api/bench/suites` | List available suites |
| GET | `/api/bench/suites/{id}` | Get full suite with tasks |
| POST | `/api/bench/suites` | Upload a custom suite |
| GET | `/api/bench/models` | Available models with pricing |
| GET | `/api/bench/pareto` | Pareto frontier (pass rate vs cost) |
| GET | `/api/bench/export/{id}` | Export a run as JSON |
| GET | `/api/bench/events` | SSE stream of bench events only |
| GET | `/api/bench/runs/{id}/events` | SSE stream for a specific run |

<details>
<summary>POST /api/bench/run — start a benchmark</summary>

```json
{
  "suite_id": "learnable-rust",
  "label": "optional-label",
  "overrides": {
    "model": "claude-sonnet-4-6",
    "strategy": "full"
  }
}
```

`overrides.strategy`: `"full"` (with learning stores, default) or `"minimal"` (skip learning). Also accepts `"config"` as alias for `"overrides"`.

Response: `202 Accepted`
```json
{ "id": "bench-run-uuid" }
```

Publishes `BenchRunStarted`, `BenchTaskStarted`, `BenchTaskCompleted`, `BenchLearningEvent`, `BenchProgress`, and `BenchRunCompleted` events to the event bus.

</details>

<details>
<summary>GET /api/bench/runs — list runs</summary>

**Query parameters:** `suite_id: string`, `status: string` (`running`, `completed`, `failed`, `cancelled`), `limit: usize` (default: 50), `offset: usize` (default: 0)

```json
{
  "total": 10,
  "offset": 0,
  "limit": 50,
  "runs": [
    { "id": "...", "suite_id": "...", "suite_name": "...", "status": "completed", "started_at": 1745942400, "finished_at": 1745943000, "pass_rate": 0.8, "total_cost_usd": 2.50 }
  ]
}
```

</details>

<details>
<summary>GET /api/bench/run/{id} — full run details</summary>

```json
{
  "id": "bench-run-uuid",
  "suite_id": "learnable-rust",
  "suite_name": "Learnable Rust",
  "kind": "manual",
  "status": "completed",
  "started_at": 1745942400,
  "finished_at": 1745943000,
  "current_task_index": 5,
  "total_tasks": 5,
  "results": [
    {
      "task_id": "task-1",
      "task_name": "Implement format_greeting",
      "passed": true,
      "duration_ms": 8200,
      "model_used": "claude-sonnet-4-6",
      "input_tokens": 1200,
      "output_tokens": 350,
      "cost_usd": 0.42,
      "output_preview": "fn format_greeting(name: &str)...",
      "error": null
    }
  ],
  "summary": {
    "pass_rate": 0.8,
    "total_cost_usd": 2.50,
    "avg_duration_ms": 7400,
    "total_tasks": 5,
    "passed_tasks": 4
  }
}
```

</details>

<details>
<summary>GET /api/bench/run/{id}/status — lightweight poll</summary>

```json
{
  "id": "bench-run-uuid",
  "status": "running",
  "current_task_index": 3,
  "total_tasks": 5,
  "passed": 3,
  "failed": 0,
  "summary": null
}
```

</details>

<details>
<summary>Bench SSE stream format</summary>

The `/api/bench/events` and `/api/bench/runs/{id}/events` endpoints stream bench events only:

```
id: <seq>
data: {"type":"BenchTaskCompleted","bench_id":"...","task_id":"...","result":{...}}
```

Event types: `BenchRunStarted`, `BenchTaskStarted`, `BenchTaskCompleted`, `BenchLearningEvent`, `BenchProgress`, `BenchRunCompleted`.

</details>

---

## Dreams

Handler module: `routes/dream.rs`

Dreams are Roko's offline consolidation cycles. During a dream run, the system processes recent episodes in phases (Hypnagogia → NREM → REM → Integration), clusters related experiences, extracts knowledge, creates playbooks, and prunes anti-patterns. Think of it as the system processing what it learned while it was "awake."

| Method | Path | Description |
|---|---|---|
| POST | `/api/dream/run` | Trigger a dream consolidation cycle |
| GET | `/api/dream/journal` | Dream journal for visualization |

<details>
<summary>POST /api/dream/run — trigger a dream cycle</summary>

Runs `roko-dreams::DreamRunner::consolidate_now()` in the background. Publishes `OperationStarted` and `OperationCompleted` events.

```json
{ "mode": "full" }
```

`mode`: `"full"` (medium effort, default) or `"quick"` (low effort, bare mode).

Response: `202 Accepted`
```json
{ "id": "op-uuid" }
```

</details>

<details>
<summary>GET /api/dream/journal — phase visualization data</summary>

Journal entries are read from `.roko/dreams/journal.jsonl`.

```json
{
  "last_cycle": "2026-04-07T03:00:00Z",
  "cycle_count": 8,
  "phases": [
    {
      "name": "Hypnagogia",
      "status": "completed",
      "episodes_processed": 32,
      "clusters_formed": 7,
      "knowledge_entries_written": 14,
      "playbooks_created": 3,
      "duration_secs": 45,
      "trend": []
    },
    { "name": "NREM", ... },
    { "name": "REM", ... },
    { "name": "Integration", ... }
  ]
}
```

</details>

---

## Knowledge (Neuro)

Handler module: `routes/neuro.rs`

The neuro store is Roko's durable, searchable knowledge base. Episodes and dream cycles write to it; agents query it before executing tasks. Knowledge entries are stored with tier designations (Hot/Warm/Cold) and HDC (Hyperdimensional Computing) fingerprints for similarity search.

| Method | Path | Description |
|---|---|---|
| POST | `/api/neuro/query` | Query via HDC similarity search |
| GET | `/api/knowledge` | Alias using query parameters (`?q=<topic>&limit=N`) |

<details>
<summary>POST /api/neuro/query — knowledge search</summary>

```json
{
  "query": "error handling patterns in async Rust",
  "limit": 10,
  "min_tier": "warm"
}
```

`query` must be non-blank.

```json
{
  "results": [
    {
      "id": "entry-uuid",
      "content": "Use tokio::select! with cancellation tokens...",
      "kind": "Playbook",
      "tier": "Warm",
      "relevance": 0.87,
      "created_at": "2026-04-07T12:00:00Z"
    }
  ],
  "total": 3
}
```

</details>

---

## Research

Handler module: `routes/research.rs`

Research routes let you trigger agent-driven research, enhance existing PRDs and plans with findings, and analyze execution data for insights.

| Method | Path | Description |
|---|---|---|
| GET | `/api/research` | List research artifacts |
| POST | `/api/research/topic` | Conduct deep research on a topic |
| POST | `/api/research/enhance-prd/{slug}` | Enhance a PRD with research findings |
| POST | `/api/research/enhance-plan/{plan}` | Enhance a plan with research findings |
| POST | `/api/research/enhance-tasks/{plan}` | Enhance plan tasks with research |
| POST | `/api/research/analyze` | Analyze execution data and generate insights |

<details>
<summary>POST /api/research/topic — deep research</summary>

All research operations are background operations (agent-driven).

```json
{
  "topic": "Rust async runtime design patterns",
  "intent": "explore",
  "depth": "comprehensive"
}
```

`intent` values: `"position"`, `"evaluate"`, `"monitor"`, `"explore"`, `"audit"`.

Response: `202 Accepted` with `{ "operation_id": "op-uuid" }`.

</details>

---

## Jobs

Handler module: `routes/jobs.rs`

Jobs are discrete units of work that can be created, assigned to agents, executed, submitted, and evaluated. They implement a marketplace model where agents bid on and complete work items.

Jobs are backed by `.roko/jobs/*.json`.

**State machine:** `open → assigned/in_progress/cancelled`, `assigned → in_progress/open/cancelled`, `in_progress → submitted/failed/cancelled`, `submitted → completed/failed`. Terminal states: `completed`, `failed`, `cancelled`.

| Method | Path | Description |
|---|---|---|
| GET | `/api/jobs` | List jobs (filter by `status`, `job_type`) |
| POST | `/api/jobs` | Create a new job |
| GET | `/api/jobs/stats` | Aggregate counts by status |
| POST | `/api/jobs/match` | Match available jobs to an agent's capabilities |
| GET | `/api/jobs/{id}` | Get a job |
| PATCH | `/api/jobs/{id}` | Update job fields |
| DELETE | `/api/jobs/{id}` | Cancel a job |
| POST | `/api/jobs/{id}/cancel` | (alias) |
| POST | `/api/jobs/{id}/assign` | Assign to an agent |
| POST | `/api/jobs/{id}/start` | Transition to `in_progress` |
| POST | `/api/jobs/{id}/submit` | Submit completed work |
| POST | `/api/jobs/{id}/evaluate` | Evaluate submitted work |
| POST | `/api/jobs/{id}/execute` | Execute using a local agent |

<details>
<summary>Route details and request/response shapes</summary>

#### `POST /api/jobs`

`title` and `description` must be non-blank.

```json
{
  "title": "Implement rate limiting",
  "description": "...",
  "job_type": "implementation",
  "reward": "0.1",
  "required_skills": ["rust"],
  "tags": []
}
```

Response: `201 Created` with the full job object. Publishes `JobCreated` event.

#### `GET /api/jobs`

**Query parameters:** `status: string`, `job_type: string`, `limit: usize` (default: 50), `offset: usize` (default: 0)

Response: `{ "jobs": [...], "total": N, "offset": 0, "limit": 50 }`

#### `POST /api/jobs/match`

```json
{ "agent_id": "...", "capabilities": ["rust", "testing"] }
```

#### `POST /api/jobs/{id}/assign`

```json
{ "agent_id": "..." }
```

Publishes `JobTransitioned` and `JobPostedToCandidate` events.

#### `POST /api/jobs/{id}/submit`

```json
{ "output": "...", "artifacts": [...] }
```

#### `POST /api/jobs/{id}/evaluate`

```json
{ "accepted": true, "feedback": "Looks good" }
```

Publishes `JobEvaluated` event.

#### `POST /api/jobs/{id}/execute`

Executes using a local agent in the background. Publishes `JobExecutionStarted`, `JobProgress`, `JobAgentOutput`, `ChainTriageResult` events.

Response: `202 Accepted`.

</details>

---

## Deployments

Handler module: `routes/deployments.rs`

Deployments let you spin up cloud workers (Railway, Fly, etc.) and route tasks to them.

| Method | Path | Description |
|---|---|---|
| POST | `/api/deployments` | Create a cloud deployment |
| GET | `/api/deployments` | List all deployments |
| GET | `/api/deployments/{id}` | Get deployment details |
| DELETE | `/api/deployments/{id}` | Tear down a deployment |
| GET | `/api/deployments/{id}/logs` | Fetch deployment logs |
| POST | `/api/deployments/{id}/task` | Proxy a task to a deployed worker |
| POST | `/api/deployments/{id}/callback` | Receive callback from a worker |

<details>
<summary>POST /api/deployments — create a deployment</summary>

```json
{
  "template": "railway-worker",
  "params": { "name": "my-agent", "region": "us-west-2" },
  "backend": "railway-api",
  "railway_token": "optional-per-request-token"
}
```

Response: `201 Created` with deployment object. Publishes `DeploymentCreated` event.

</details>

---

## Inference Gateway

Handler module: `routes/gateway.rs`

The inference gateway is a centralized dispatch layer for all LLM requests. Instead of agents holding API keys directly, they route through the gateway, which handles model selection (via CascadeRouter), provider health tracking, cost accounting, and event publishing.

| Method | Path | Description |
|---|---|---|
| POST | `/api/inference/complete` | Submit a completion request |
| GET | `/api/gateway/stats` | Gateway request statistics |
| GET | `/api/gateway/models` | Models available through gateway with routing weights |
| POST | `/api/inference/batch/submit` | Submit a batch of requests |
| GET | `/api/inference/batch/{id}` | Get batch status and results |

<details>
<summary>POST /api/inference/complete — submit a completion</summary>

```json
{
  "messages": [
    { "role": "user", "content": "Implement the parse() function" }
  ],
  "model": "claude-sonnet-4-6",
  "max_tokens": 4096,
  "temperature": 0.7,
  "task_category": "implementation",
  "complexity": "standard",
  "agent_id": "agent-uuid",
  "auto_route": true
}
```

If `auto_route: true` and no `model` is specified, the CascadeRouter selects the model. Publishes `InferenceStarted` and `InferenceCompleted` / `InferenceFailed` events.

```json
{
  "content": "fn parse(input: &str) -> Result<...",
  "model_used": "claude-sonnet-4-6",
  "input_tokens": 1200,
  "output_tokens": 450,
  "cost_usd": 0.042,
  "duration_ms": 3800,
  "request_id": "req-uuid",
  "auto_routed": true
}
```

</details>

---

## Providers and Models

Handler module: `routes/providers.rs`

These routes expose the configured LLM providers and models, along with health diagnostics and routing explanations.

| Method | Path | Description |
|---|---|---|
| GET | `/api/providers` | List providers with health and model counts |
| GET | `/api/providers/{id}/health` | Health details for a specific provider |
| POST | `/api/providers/{id}/test` | Test a provider with a live completion |
| GET | `/api/models` | List all models with tier and pricing |
| GET | `/api/routing/explain` | Explain routing decision for a hypothetical request |

<details>
<summary>Route details and response shapes</summary>

#### `GET /api/providers`

```json
{
  "providers": [
    {
      "id": "anthropic",
      "name": "Anthropic",
      "status": "healthy",
      "model_count": 4,
      "healthy_models": 4,
      "consecutive_failures": 0,
      "last_error": null,
      "last_checked_at": "2026-04-07T12:00:00Z"
    }
  ]
}
```

#### `POST /api/providers/{id}/test`

Sends `"Say hello."` to the provider and returns the response. Useful for diagnosing connectivity issues.

#### `GET /api/routing/explain`

**Query parameters:** `role: string`, `task_category: string`, `complexity: string` (`"fast"`, `"standard"`, `"complex"`)

```json
{ "selected_model": "...", "stage": "ucb", "reason": "...", "alternatives": [...] }
```

</details>

---

## Config

Handler module: `routes/config.rs`

These routes let you inspect and update Roko's runtime configuration without restarting the server.

| Method | Path | Description |
|---|---|---|
| GET | `/api/config` | Current config (secrets masked to `"***"`) |
| PUT | `/api/config` | Deep-merge a partial config update |
| POST | `/api/config/reload` | Reload from disk and hot-apply compatible sections |

<details>
<summary>Route details</summary>

#### `PUT /api/config`

Accepts a partial `RokoConfig` JSON (any subset of fields) and deep-merges it into the current config, then writes to `roko.toml`. Publishes `ConfigReloaded` event.

#### `POST /api/config/reload`

Hot-applies compatible sections (logging, providers, subscriptions). Sections requiring a restart are noted in the response.

```json
{
  "success": true,
  "warnings": [],
  "timestamp": "2026-04-07T12:00:00Z"
}
```

</details>

---

## Subscriptions and Workflows

Handler module: `routes/subscriptions.rs`, `routes/workflows.rs`

Subscriptions let you register event listeners that trigger actions when specific events occur. Workflows are sequences of steps that can be triggered manually or by subscriptions.

| Method | Path | Description |
|---|---|---|
| GET | `/api/subscriptions` | List event subscriptions |
| POST | `/api/subscriptions` | Create a subscription |
| GET | `/api/subscriptions/{id}` | Get subscription |
| DELETE | `/api/subscriptions/{id}` | Delete subscription |
| GET | `/api/workflows` | List workflows |
| POST | `/api/workflows` | Create workflow |
| GET | `/api/workflows/{id}` | Get workflow |
| PUT | `/api/workflows/{id}` | Update workflow |
| DELETE | `/api/workflows/{id}` | Delete workflow |
| POST | `/api/workflows/{id}/trigger` | Trigger workflow |

---

## Heartbeats

Handler module: `routes/heartbeats.rs`

Agents and sidecars use this endpoint to report liveness. The control plane uses heartbeat data to determine which agents are healthy.

### `POST /api/heartbeats`

Used by `roko-agent-server`'s background heartbeat loop. Publishes `HeartbeatReceived` event.

**Request body** (`HeartbeatPayload`):
```json
{
  "sender_id": "agent-uuid",
  "timestamp": "2026-04-07T12:00:00Z",
  "active_tasks": 2,
  "completed_tasks": 45,
  "failed_tasks": 1,
  "active_agents": 1,
  "frequency": 0.033,
  "metrics": {}
}
```

Response: `200 OK`.

---

## Secrets

Handler module: `routes/secrets.rs`

All mutations require `admin` scope.

| Method | Path | Description |
|---|---|---|
| GET | `/api/secrets` | List secrets (names only, values masked) |
| POST | `/api/secrets` | Store a secret |
| GET | `/api/secrets/{name}` | Get a secret |
| PUT | `/api/secrets/{name}` | Update a secret |
| DELETE | `/api/secrets/{name}` | Delete a secret |
| POST | `/api/api-keys` | Create an API key entry |
| DELETE | `/api/api-keys/{name}` | Revoke an API key |

---

## Chain

Handler module: `routes/chain.rs`

Blockchain witness and chain client routes. These are Phase 2+ features — the API surface is defined but chain integration requires a backend.

| Method | Path | Description |
|---|---|---|
| GET | `/api/chain/status` | Chain connection status |
| POST | `/api/chain/witness` | Submit an episode witness |
| GET | `/api/chain/witnesses` | List witness records |

---

## Webhooks

Handler module: `routes/webhooks.rs`

Webhooks are always public (no `/api/` prefix, no auth required). They allow external services to push events into Roko's event bus.

### `POST /webhook/{source}`

Validates the source, constructs an `Engram` signal, and publishes it to the event bus. Publishes a `WebhookReceived` event.

**Path parameter:** `source` — webhook source name (e.g. `github`, `slack`)

**Request body:** Arbitrary JSON payload.

```json
{ "accepted": true, "signal_id": "..." }
```

---

## Terminal

Terminal routes provide a PTY (pseudo-terminal) interface to the server's shell. They are **disabled by default** for security.

Enable with `serve.terminal_enabled = true` in `roko.toml` or `--enable-terminal` on the CLI. When disabled, all terminal routes return `403 Forbidden` with `{ "error": "Terminal disabled", "hint": "Set serve.terminal_enabled=true or use --enable-terminal" }`.

When enabled on a non-loopback bind address, terminal routes require API key auth.

| Method | Path | Description |
|---|---|---|
| GET | `/api/terminal/sessions` | List active PTY sessions |
| POST | `/api/terminal/sessions` | Create a PTY session |
| GET | `/api/terminal/sessions/{id}` | Get session details |
| DELETE | `/api/terminal/sessions/{id}` | Kill a PTY session |
| GET | `/api/terminal/sessions/{id}/stream` | WebSocket PTY stream |

---

## OpenAPI

### `GET /api/openapi.json`

Returns the OpenAPI 3.0 spec for the control plane API as JSON. Generated from route annotations.

---

## Per-Agent Sidecar API

`roko-agent-server` (`crates/roko-agent-server/`) runs a per-agent HTTP server on a dynamic port (default: `0.0.0.0:0`, assigned by the OS). Unlike the control plane which manages the whole system, each sidecar serves a single agent — you talk to it to send that specific agent a message, get its stats, or query its local knowledge.

Sidecars register themselves with the control plane via `POST /api/agents/register` and send periodic heartbeats to `POST /api/heartbeats`. The control plane discovers them via `DiscoveredAgent` entries.

The sidecar is built via `AgentServer::builder()` with feature flags controlling which route groups are mounted.

### Authentication

When built with `.auth(BearerAuth::new("token"))`, all non-public routes require:

```
Authorization: Bearer <token>
```

Public routes (`/health`, `/capabilities`) are always accessible without auth.

### Public routes

<details>
<summary>GET /health and GET /capabilities</summary>

#### `GET /health`

```json
{ "status": "ok", "agent_id": "agent-uuid", "uptime_s": 3600 }
```

#### `GET /capabilities`

Capabilities manifest including feature flags, live routes, and skill configs. Feature capabilities (`messaging`, `predictions`, `research`, `tasks`) are only advertised if the corresponding feature is enabled via the builder.

```json
{
  "agent_id": "agent-uuid",
  "version": "0.1.0",
  "features": ["messaging", "research", "custom-skill"],
  "routes": ["/health", "/capabilities", "/stats", "/logs", "/message", "/stream", "/predictions", "/research", "/tasks"],
  "skills": {
    "custom-skill": { "enabled": true, "config": {} }
  }
}
```

</details>

### Protected routes (always available when auth passes)

<details>
<summary>GET /stats and GET /logs</summary>

#### `GET /stats`

```json
{
  "agent_id": "agent-uuid",
  "uptime_s": 3600,
  "messages_processed": 142,
  "predictions_count": 7,
  "tasks_pending": 2,
  "tasks_completed": 38
}
```

#### `GET /logs`

Streams the agent sidecar log file. Returns `text/plain`.

</details>

### Messaging feature (`builder.messaging()`)

<details>
<summary>POST /message and GET /stream (WebSocket)</summary>

#### `POST /message`

Dispatches a prompt to the agent's LLM backend and returns the full response.

```json
{
  "prompt": "Explain what you're working on",
  "context": { "thread": "xyz" }
}
```

```json
{
  "response": "I'm currently implementing...",
  "reasoning": null,
  "usage": { "input_tokens": 800, "output_tokens": 240 },
  "session": {
    "session_id": "sess-uuid",
    "thread_id": "thread-uuid",
    "conversation_id": "conv-uuid"
  },
  "finish_reason": "stop",
  "engram_id": "engram-uuid",
  "context": { "thread": "xyz" }
}
```

**Errors:**
- `503 Service Unavailable`: agent has no configured dispatcher
- `502 Bad Gateway`: dispatch failed (LLM backend error)

#### `GET /stream` (WebSocket)

Streaming dispatch. Send a plain text prompt frame; receive streaming chunks:

```json
{ "chunk": "I'm cur", "done": false }
{ "chunk": "rently...", "done": false }
{ "reasoning": "thinking...", "done": false }
{ "tool_call": { "index": 0, "name_delta": "bash", "arguments_delta": "{" }, "done": false }
{ "usage": { "input_tokens": 800, "output_tokens": 240 }, "done": false }
```

**Final frame:**
```json
{
  "done": true,
  "session": { "session_id": "...", "thread_id": "...", "conversation_id": "..." },
  "usage": { "input_tokens": 800, "output_tokens": 240 },
  "finish_reason": "stop"
}
```

**Error frame:**
```json
{ "error": "dispatch failed: ...", "done": true }
```

</details>

### Predictions feature (`builder.predictions()`)

<details>
<summary>Predictions routes</summary>

#### `GET /predictions`

List all predictions. Returns array of `AgentPrediction` objects.

#### `POST /predictions`

```json
{
  "market": "ETH-USD",
  "direction": "up",
  "confidence": 0.75,
  "horizon_secs": 3600,
  "metadata": {}
}
```

Response: `200 OK` with the created `AgentPrediction`.

#### `GET /predictions/{id}`

Returns `404` if not found.

#### `GET /predictions/residuals`

Predicted vs actual outcomes.

</details>

### Research feature (`builder.research()`)

<details>
<summary>POST /research — knowledge query</summary>

```json
{ "query": "Rust async patterns", "limit": 10 }
```

```json
{ "entries": [...], "total": 5 }
```

</details>

### Tasks feature (`builder.tasks()`)

<details>
<summary>Tasks queue routes</summary>

#### `GET /tasks`

List tasks in the agent's task queue. Returns array of `TaskEntry` objects.

#### `POST /tasks/{id}/accept`

**Path parameter:** `id` — task ID (u64)

Response: `200 OK` with accepted `TaskEntry`, or `404`.

#### `POST /tasks/{id}/complete`

```json
{
  "output": "...",
  "success": true,
  "artifacts": []
}
```

Response: `200 OK` with updated `TaskEntry`, or `404`.

</details>

### Registration and Heartbeat

`AgentServer` can be configured with an `AgentRegistration` that publishes an `AgentCard` to the control plane on startup. It also runs a background heartbeat loop posting to `POST /api/heartbeats` at a configurable interval (default: 30 seconds).

<details>
<summary>AgentCard shape (advertised to control plane)</summary>

```json
{
  "agent_id": "agent-uuid",
  "version": "0.1.0",
  "owner": "roko-orchestrator",
  "capabilities": ["messaging", "predictions"],
  "endpoints": {
    "health": "http://127.0.0.1:7001/health",
    "message": "http://127.0.0.1:7001/message",
    "stream": "ws://127.0.0.1:7001/stream"
  }
}
```

</details>

---

## Miscellaneous Routes

### Shareable run pages

`GET /runs/{id}` — shareable HTML page for a specific run. No auth required. Served outside the `/api/` namespace.

### Additional control plane routes

| Method | Path | Handler module | Description |
|---|---|---|---|
| GET | `/api/diagnosis` | `diagnosis.rs` | Conductor diagnosis report |
| GET | `/api/projections` | `projections.rs` | List runtime projections |
| GET | `/api/projections/{name}` | `projections.rs` | Get named projection |
| GET/POST | `/api/integrations/*` | `integrations.rs` | External integrations (GitHub, Slack, etc.) |
| GET | `/api/connectors` | `connectors.rs` | MCP connector inventory |
| GET | `/api/feeds` | `feeds.rs` | Data feed subscriptions |
| GET | `/api/templates` | `templates.rs` | System prompt templates |
| GET | `/api/templates/{name}` | `templates.rs` | Get a template |
| GET/POST | `/api/team` | `team.rs` | Team membership and coordination |
| GET/POST | `/api/vision-loop` | `vision_loop.rs` | Vision loop control |
| GET | `/api/auth/me` | `auth.rs` | Current authenticated user |
| POST | `/api/auth/login` | `auth.rs` | Exchange credentials for token |

### Vision loop events

The vision loop publishes `VisionLoopIteration` and `VisionLoopCompleted` `ServerEvent` objects to the event bus as each iteration completes.
