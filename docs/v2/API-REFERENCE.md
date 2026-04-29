# Roko HTTP API Reference

The roko control plane (`roko-serve`) exposes REST routes, SSE streams, and a WebSocket endpoint on port **6677** by default. All API routes live under `/api/`. The embedded React demo app is served as an SPA fallback from `/`. A bare liveness probe sits at `/health` (no `/api/` prefix).

## Table of Contents

1. [Starting the Server](#starting-the-server)
2. [Authentication](#authentication)
3. [Middleware](#middleware)
4. [Error Response Format](#error-response-format)
5. [SSE Streaming — `/api/events`](#sse-streaming)
6. [WebSocket — `/ws`](#websocket)
7. [StateHub Push Pattern](#statehub-push-pattern)
8. [Health and Status](#health-and-status)
9. [Plans](#plans)
10. [One-Shot Runs](#one-shot-runs)
11. [Dashboard Runs](#dashboard-runs)
12. [PRDs](#prds)
13. [Agents — Control Plane](#agents-control-plane)
14. [Agent Fleet Aggregation](#agent-fleet-aggregation)
15. [Gates](#gates)
16. [Episodes and Signals](#episodes-and-signals)
17. [Metrics](#metrics)
18. [Learning and Adaptation](#learning-and-adaptation)
19. [Benchmarks](#benchmarks)
20. [Dreams](#dreams)
21. [Knowledge (Neuro)](#knowledge-neuro)
22. [Research](#research)
23. [Jobs](#jobs)
24. [Deployments](#deployments)
25. [Inference Gateway](#inference-gateway)
26. [Providers and Models](#providers-and-models)
27. [Config](#config)
28. [Subscriptions and Workflows](#subscriptions-and-workflows)
29. [Heartbeats](#heartbeats)
30. [Secrets](#secrets)
31. [Chain](#chain)
32. [Webhooks](#webhooks)
33. [Terminal](#terminal)
34. [OpenAPI](#openapi)
35. [Per-Agent Sidecar API (roko-agent-server)](#per-agent-sidecar-api)

---

## Starting the Server

```bash
# Default: 127.0.0.1:6677, reads roko.toml from current directory
roko serve

# Custom bind and port
roko serve --bind 0.0.0.0 --port 8080

# Enable API key auth
roko serve --api-key sk-my-secret-key

# Enable PTY terminal (disabled by default)
roko serve --enable-terminal
```

Relevant `roko.toml` keys:

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

Authentication is **opt-in** and controlled by `serve.auth.enabled`. When enabled, all `/api/*` routes require a credential. The `/health` (top-level), `/webhook/*`, and `/runs/{id}` (shareable page) routes are always public.

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

### Error responses for auth

```json
{ "code": "unauthorized", "message": "missing X-Api-Key header or Authorization bearer token", "status": 401 }
{ "code": "insufficient_scope", "message": "scope 'read' is not sufficient for 'admin' on POST /api/secrets", "status": 403 }
```

---

## Middleware

All `/api/*` responses pass through a **secret-scrubbing middleware** (`scrub_secrets`) that redacts API key patterns (Anthropic, GitHub PATs, etc.) from JSON/text response bodies. Binary content types (`image/*`, `application/octet-stream`) are passed through unchanged. The scrubber operates on responses up to 16 MiB.

CORS is configured via `serve.cors_origins`. If empty, `CorsLayer::permissive()` is used. All routes include request tracing via `TraceLayer`.

---

## Error Response Format

All handlers return errors as JSON with the following shape:

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

## SSE Streaming

### `GET /api/events` and `GET /api/sse`

Handler: `sse::sse_handler`

Real-time SSE stream of `DashboardEvent` payloads. On connect, the server replays retained events from the ring buffer starting at the sequence number in `Last-Event-ID` (defaults to 0), then streams live events.

**Request headers:**
- `Last-Event-ID: <seq>` — resume from this sequence number (0 if absent)

**Event format:**
```
id: <monotonic-seq>
data: {"type":"task_started","plan_id":"...","task_id":"...","description":"..."}
```

Each `data:` frame is a JSON-serialized `DashboardEvent` (see [StateHub Push Pattern](#statehub-push-pattern) for all event types). The SSE connection uses keep-alive pings.

### `GET /api/workflow/events`

Handler: `workflow_sse_handler`

`RuntimeEvent`-typed SSE stream for the workflow engine. Payload is a `SseEvent` with fields `kind: String` and embedded event data. Formatted as:

```
event: <kind>
data: {"kind":"...","...event fields..."}
```

---

## WebSocket

### `GET /ws` and `GET /roko-ws`

Handler: `ws::ws_upgrade`

WebSocket connection for real-time event streaming. On connect, the server replays the full event backlog from the in-memory ring buffer, then streams live `ServerEvent` payloads as JSON text frames.

**Client control message** (optional, send after connect):

```json
{
  "subscribe": ["projection:gate_pipeline", "topic:agent.*"],
  "cursor": 42,
  "back_pressure": "at_most_once"
}
```

| Field | Type | Description |
|---|---|---|
| `subscribe` | `string[]` | Filter strings. Empty = accept all. Supports plain type substrings (`"agent"`), channel prefixes (`"projection:<name>"`, `"topic:<pattern>"`, `"engram-stream:<name>"`), and glob wildcard suffix (`"agent.*"`). |
| `cursor` | `u64` | Sequence number to replay from (catchup on reconnect). |
| `back_pressure` | `"at_most_once"` \| `"coalesce"` \| `"resume_required"` | Delivery semantics (default: `at_most_once`). |

**Outgoing frames:** JSON-serialized `ServerEvent` objects. See [events.rs](#server-events) for the full tagged union.

**Lag behavior:** If the server-side broadcast buffer overflows, lagged events are silently dropped. A warning is logged server-side at most every 5 seconds.

---

## StateHub Push Pattern

The StateHub is the central push-based state distribution hub. Orchestrator code calls `state.state_hub.publish(DashboardEvent)` or `publish_batch(...)`. The hub maintains a bounded ring buffer of recent events (default: 512) and fans out to all SSE/WS subscribers via a `tokio::sync::broadcast` channel.

### DashboardEvent variants

All `type` tags use `snake_case` unless noted. Events flow from the orchestrator → `StateHub` → SSE/WS clients.

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

### ServerEvent (event bus / WebSocket)

The WebSocket and legacy `ServerEvent` bus carry a superset of dashboard events. Additional types:

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

**Note:** `BenchLearningEvent` is serialized with its exact PascalCase type tag; all other events use `snake_case`.

---

## Health and Status

### `GET /health`

Handler: `top_level_health` (no `/api/` prefix, always public)

Bare liveness probe for load balancers. Returns `200 OK` unconditionally.

```json
{ "status": "ok" }
```

### `GET /api/health`

Handler: `health::health`

Rich health check with live telemetry.

**Response:**
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

Status is `"ok"` / `"degraded"` (some providers unhealthy) / `"down"` (all providers failed).

### `GET /api/status`

Handler: `dashboard::session_status`

Session overview including supervised processes and the process session ledger.

**Response:**
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

### `GET /api/dashboard`

Handler: `dashboard::dashboard`

Dashboard scaffold rendered by the runtime. Returns `{ "rendered": "..." }`.

### `GET /api/operations/{id}`

Handler: `dashboard::operation_status`

Look up a background operation by ID.

**Path parameter:** `id` — operation UUID

**Response:**
```json
{ "id": "op-uuid", "kind": "dream_run", "status": "Running" }
```

**Errors:** 404 if not found.

### `GET /api/relay/health`

Handler: `health::relay_health`

Relay connection diagnostics as JSON. Shape depends on the relay health tracker.

### `GET /api/parity`

Handler: `health::parity_handler`

Cross-surface parity matrix comparing TUI, HTTP API, and CLI feature coverage.

### `GET /api/retention`

Handler: `health::retention_handler`

Retention policies and any current violations for `.roko/` data files.

### `GET /api/statehub/snapshot`

Handler: `health::statehub_snapshot`

Current StateHub dashboard projection. Returns the canonical `DashboardProjection` state frame.

**Response shape:** `{ "state": {...}, "evidence": {...} }` (projection frame).

### `GET /api/statehub/events`

Handler: `health::statehub_events`

Bounded replay of retained dashboard events from the StateHub ring buffer.

**Query parameters:**
| Parameter | Type | Default | Description |
|---|---|---|---|
| `after_seq` | `u64` | `0` | Return events with seq > this value |
| `limit` | `usize` | `256` | Max events to return (capped at 1024) |
| `run_id` | `string` | — | Filter by plan/run ID |
| `plan_id` | `string` | — | Filter by plan ID |
| `task_id` | `string` | — | Filter by task ID |
| `type` (alias: `event_type`) | `string` | — | Filter by event type tag |

**Response:**
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

### `GET /api/truth_map`

Handler: `dashboard::truth_map_handler`

Entity truth-source registry — maps each data entity to its canonical source of truth.

---

## Plans

Handler module: `routes/plans.rs`

### `GET /api/plans`

List plans from `.roko/plans/`.

**Response:**
```json
{
  "plans": [
    { "id": "plan-1", "title": "...", "status": "...", "task_count": 5 }
  ]
}
```

### `POST /api/plans`

Create a new plan.

**Request body:**
```json
{ "title": "My Plan", "description": "...", "tasks": [] }
```

**Response:** `201 Created` with `{ "id": "plan-uuid" }`.

### `GET /api/plans/{id}`

Get full plan details.

**Path parameter:** `id` — plan slug or UUID

**Response:** Full plan object including tasks and metadata.

### `GET /api/plans/{id}/tasks`

List tasks for a plan.

**Response:** `{ "tasks": [...] }`

### `POST /api/plans/{id}/execute`

Execute a plan in the background.

**Response:** `202 Accepted` with `{ "operation_id": "op-uuid" }`.

### `GET /api/plans/{id}/status`

Get plan execution status.

**Response:** `{ "id": "...", "status": "running|paused|completed|failed", "progress": {...} }`

### `POST /api/plans/{id}/pause`

Pause a running plan execution.

**Response:** `200 OK`.

### `POST /api/plans/{id}/resume`

Resume a paused plan execution.

**Response:** `200 OK`.

### `GET /api/plans/{id}/gates`

Get gate results for a plan.

**Response:** Gate verdicts grouped by task.

### `GET /api/plans/{id}/reviews`

List human reviews submitted for a plan.

### `POST /api/plans/{id}/tasks/{task_id}/review`

Submit a human review for a task.

**Request body:**
```json
{ "approved": true, "comment": "Looks good" }
```

### `GET /api/plans/{id}/tasks/{task_id}/diff`

Get the code diff produced by a task agent.

### `POST /api/plans/{id}/chat`

Send a chat message in the context of a plan.

**Request body:**
```json
{ "message": "What's the current status?" }
```

### `POST /api/plans/{id}/estimate`

Estimate cost and duration for a plan before execution.

**Response:** `{ "estimated_cost_usd": 2.50, "estimated_duration_mins": 15, "task_count": 8 }`

### `POST /api/plans/generate`

Generate a plan from a prompt using an agent.

**Request body:**
```json
{ "prompt": "Implement rate limiting for the API", "context": "..." }
```

**Response:** `202 Accepted` with `{ "operation_id": "op-uuid" }`.

---

## One-Shot Runs

Handler module: `routes/run.rs`

### `POST /api/run`

Spawn a background `run_once()` invocation. Executes one prompt through the universal compose→agent→gate→persist loop. Emits `RunStarted`, `DashboardEvent::TaskStarted`, `DashboardEvent::AgentSpawned`, and `RunCompleted` events to the StateHub.

**Request body:**
```json
{
  "prompt": "Add a unit test for the parser",
  "workdir": "/optional/override/path"
}
```

**Validation:** `prompt` must be non-blank.

**Response:** `202 Accepted`
```json
{ "id": "run-uuid" }
```

### `GET /api/run/{id}/status`

Poll the status of a background run.

**Path parameter:** `id` — run UUID returned by `POST /api/run`

**Response:**
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

**Errors:** 404 if the run ID is not in the active runs map.

---

## Dashboard Runs

Handler module: `routes/runs.rs`

### `GET /api/dashboard/runs`

Summarize runs from the runtime event log (`.roko/runtime-events.jsonl`).

**Response:**
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

---

## PRDs

Handler module: `routes/prds.rs`

### `GET /api/prds`

List all PRDs from `.roko/prd/`.

**Response:** Array of PRD summary objects with `slug`, `title`, `status`, `created_at`.

### `GET /api/prds/status`

Coverage report: counts by lifecycle stage (idea, draft, planned, implemented).

### `POST /api/prds/ideas`

Capture a new work-item idea.

**Request body:**
```json
{ "title": "Add retry logic to the HTTP client", "description": "..." }
```

**Response:** `201 Created` with the new PRD slug.

### `GET /api/prds/{slug}`

Get a PRD by slug. Reads the markdown file from `.roko/prd/{slug}.md`.

**Response:** `{ "slug": "...", "title": "...", "content": "...", "status": "..." }`

### `POST /api/prds/{slug}/draft`

Agent-driven PRD drafting. Spawns a background agent that enriches the idea into a full PRD with requirements, acceptance criteria, and technical design.

**Response:** `202 Accepted` with `{ "operation_id": "op-uuid" }`.

### `POST /api/prds/{slug}/promote`

Promote a draft PRD to `planned` or `approved` status. Triggers `AtelierPrdsUpdated` StateHub event.

**Request body:**
```json
{ "status": "approved" }
```

### `POST /api/prds/{slug}/plan`

Generate an implementation plan (`tasks.toml`) from the PRD. Spawns a background agent. Publishes `AtelierPrdsUpdated`. If `prd.auto_plan` is enabled in `roko.toml`, this is triggered automatically on PRD publish.

**Response:** `202 Accepted` with `{ "operation_id": "op-uuid" }`.

### `POST /api/prd/consolidate` and `POST /api/prds/consolidate`

Scan all PRDs for duplicates and gaps. Spawns a background agent.

**Response:** `202 Accepted` with `{ "operation_id": "op-uuid" }`.

---

## Agents — Control Plane

Handler module: `routes/agents.rs`

### `GET /api/managed-agents`

List all managed agent processes and discovered agents. Used by the dashboard fleet roster. Merges locally supervised processes with self-registered remote sidecars.

**Response:** Array of agent dashboard payloads:
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

### `POST /api/agents/register`

Register a remote agent sidecar with the control plane.

**Request body:**
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

**Response:** `200 OK` with registration record.

### `POST /api/agents/create`

Create and spawn a new agent process from manifest config.

**Request body:**
```json
{
  "name": "implementer-2",
  "domain": "rust",
  "model": "claude-sonnet-4-6",
  "role": "Implementer"
}
```

**Response:** `202 Accepted`.

### `GET /api/agents/{id}`

Get agent details by ID (looks up in discovered agents registry).

### `GET /api/agents/{id}/profile`

Get the model profile assigned to an agent.

### `POST /api/agents/{id}/stop`

Stop a running agent process.

### `GET /api/agents/{id}/episodes`

Get episodes attributed to a specific agent.

**Query parameters:** `limit: usize` (optional)

### `GET /api/agents/{id}/logs`

Proxy logs from the agent sidecar's `/logs` endpoint.

### `POST /api/agents/{id}/message`

Send a prompt to an agent's sidecar and return the response inline (30-second timeout).

**Request body:**
```json
{ "prompt": "Explain what you're working on", "context": {} }
```

**Response:** Agent response object (mirrors sidecar `/message` response).

### `POST /api/agents/{id}/start`

Start (or restart) a stopped agent.

### `POST /api/agents/{id}/restart`

Restart an agent.

### `GET /api/agents/{id}/token`

Get the current token status for an agent (expiry, hash presence).

### `POST /api/agents/{id}/token`

Issue (or rotate) a bearer token for an agent. The token hash is stored in `DiscoveredAgent.token_hash`. Tokens expire after 24 hours by default.

**Response:**
```json
{ "token": "roko-agent-<uuid>", "expires_at": "2026-04-30T00:00:00Z" }
```

---

## Agent Fleet Aggregation

Handler module: `routes/aggregator.rs`

These routes aggregate data from all discovered agent sidecars in parallel.

### `GET /api/agents`

List all discovered agents with live stats (TTL-cached, 30s).

**Query parameters:**
- `owner: string` — filter by owner label

**Response:** Paginated response: `{ items, total, offset, limit, has_more }` where each item is an `AgentCard` extended with live stats.

### `GET /api/agents/topology`

Return agent topology as a graph. Shape matches the frontend `AgentTopology` component.

### `GET /api/agents/{id}/stats`

Fetch live stats from agent `{id}`'s sidecar `/stats` endpoint (TTL-cached, 5s).

### `GET /api/agents/{id}/skills`

Fetch skill manifest from agent `{id}`'s sidecar `/capabilities` endpoint.

### `GET /api/agents/{id}/heartbeat`

Get the latest heartbeat from agent `{id}`.

### `GET /api/agents/{id}/trace`

Proxy agent trace logs.

**Query parameters:**
- `limit: usize` (default: 50)
- `offset: usize` (default: 0)

### `GET /api/predictions/sessions`

List prediction sessions across all agents (TTL-cached, 10s).

### `GET /api/predictions/sessions/{id}`

Get a specific prediction session.

### `GET /api/predictions/claims`

List prediction claims across all agents.

### `GET /api/predictions/calibration/{agent_id}`

Get prediction calibration data for an agent.

### `GET /api/knowledge/entries`

List knowledge entries from the neuro store (TTL-cached, 30s).

### `GET /api/knowledge/edges`

List knowledge graph edges.

### `GET /api/knowledge/search`

**Query parameters:** `q: string` — search query

### `GET /api/knowledge/kinds`

List known knowledge kinds.

### `GET /api/tasks`

List tasks from all agent sidecars (TTL-cached, 30s).

**Query parameters:**
- `state: string` — filter by task state
- `kind: string` — filter by task kind
- `assignee: string` — filter by assignee agent
- `limit: usize` (default: 50)
- `offset: usize` (default: 0)

**Response:** `{ items, total, offset, limit, has_more }`

### `GET /api/tasks/stats`

Aggregate task statistics across all agents.

### `GET /api/tasks/{id}`

Get a specific task by ID.

### `GET /api/ws` (aggregator)

WebSocket that aggregates live event streams from all discovered agent sidecars. Reconnects automatically when agents join or leave (refresh interval: 10s, reconnect delay: 2s).

---

## Gates

Handler module: `routes/status/gates.rs`

### `GET /api/gates/summary`

Aggregate gate verdicts from canonical projections (`RuntimeProjectionSet`). Returns pass rate, average duration, and last run per gate, plus a rung breakdown.

**Response:**
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

Rung names: `0=compile`, `1=clippy`, `2=test`, `3=diff`, `4=fmt`, `5=custom`, `6=judge`.

### `GET /api/gates/history`

Recent gate verdicts across all gates.

**Query parameters:**
| Parameter | Type | Description |
|---|---|---|
| `gate` | `string` | Filter to one gate name |
| `limit` | `usize` | Max results |
| `format` | `"waterfall"` | Return waterfall shape (grouped by `task_id`) |

**Default response:**
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

**Waterfall response** (`?format=waterfall`):
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

### `GET /api/gates/{gate_name}/history`

Time series of pass/fail results for one specific gate.

**Path parameter:** `gate_name` — gate name (e.g. `compile`, `test`)

**Response:** Same shape as `GET /api/gates/history`. Returns 404 if no records exist for the gate.

---

## Episodes and Signals

Handler module: `routes/status/episodes.rs`

### `GET /api/episodes`

Normalized episode proof rows from canonical projections.

**Query parameters:** `ProjectionQuery` fields:
- `limit: usize`
- `plan_id: string`
- `task_id: string`
- `gate: string`

**Response:** JSON array of episode objects from `.roko/episodes.jsonl`.

### `GET /api/signals`

Read the signals JSONL as a JSON array (most recent first).

**Query parameters:**
- `limit: usize` — max results (default: 500, max: 500)

**Response:** JSON array of Engram signal objects from `.roko/engrams.jsonl`.

---

## Metrics

Handler module: `routes/status/metrics.rs`

### `GET /api/metrics`

Raw metric snapshots from the in-memory metrics registry.

**Response:** Array of metric snapshot objects.

### `GET /api/metrics/summary`

Aggregate recent execution and learning metrics for the dashboard.

**Query parameters:**
- `period: string` — `"last_7_days"` (default), `"last_30_days"`, `"last_90_days"`, or `"last_N_days"`

**Response:**
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

### `GET /api/metrics/success_rate`

Per-template success rate split by trigger kind.

**Response:**
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

### `GET /api/metrics/engagement`

Feedback acknowledgement ratio per template.

**Response:**
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

### `GET /api/metrics/c_factor`

Composite C-Factor, component sub-metrics, per-agent contributions, and per-fleet stats.

**Response:**
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

### `GET /api/metrics/model_efficiency`

Cost per successful episode for each routed model.

**Response:**
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

### `GET /api/metrics/gate_rate`

Gate pass/total per gate with a trend delta (improving/flat/declining).

**Response:**
```json
{
  "summary": { "compile": { ... }, "test": { ... } },
  "history": { ... },
  "evidence": { ... }
}
```

### `GET /api/metrics/experiments`

Best vs worst variant success-rate gap per experiment.

**Response:**
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

### `GET /api/metrics/feedback_latency`

Median hours from agent action to first gate feedback signal.

**Response:**
```json
{ "sample_count": 42, "median_hours": 0.003 }
```

### `GET /api/metrics/velocity`

Rate of change of success rate over time (self-improvement velocity in Δ%/day).

**Response:**
```json
{
  "velocity": 0.042,
  "sample_count": 127,
  "evidence": { ... }
}
```

### `GET /api/metrics/coverage`

Percentage of events in the bus backlog that matched a known subscription term.

**Response:**
```json
{
  "matched_events": 118,
  "unhandled_events": 9,
  "coverage": 0.929,
  "subscription_terms": ["plan", "task", "gate", "execution", "episode", "efficiency", "run", "operation", "deployment", "error", "server_shutdown", "agent"]
}
```

### `GET /api/metrics/prometheus`

Prometheus text exposition format (Content-Type: `text/plain; version=0.0.4; charset=utf-8`).

**Exposed metrics:**
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

---

## Learning and Adaptation

Handler module: `routes/learning/mod.rs`, `learning/router_state.rs`, `learning/experiments.rs`

All `learning/` routes have `/learn/` aliases (both prefix forms are mounted).

### `GET /api/learning/efficiency` and `GET /api/learn/efficiency`

Aggregate `.roko/learn/efficiency.jsonl` into task-level cost and timing metrics.

**Response:**
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

### `GET /api/learning/costs` and `GET /api/learn/costs`

Canonical runtime cost projection from `RuntimeProjectionSet`.

**Query parameters:** `ProjectionQuery` fields.

### `GET /api/learning/provider-outcomes` and `GET /api/learn/provider-outcomes`

Provider health data shaped for the `ProviderCell` component.

**Response:**
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

Status: `"healthy"` (≥97%), `"degraded"` (≥90%), `"unhealthy"` (<90%).

### `GET /api/learning/retries` and `GET /api/learn/retries`

Retry attempt proof surface (canonical `retry_state` projection).

### `GET /api/learning/runtime-feedback` and `GET /api/learn/runtime-feedback`

Joined feedback store overview (`runtime_feedback` projection).

### `GET /api/learning/cascade-router` and `GET /api/learn/cascade-router` and `GET /api/learn/router`

Read `.roko/learn/cascade-router.json` via `learning_policy_state` projection. Returns raw JSON plus `projection_state`, `source`, and `evidence`.

### `GET /api/learning/cascade` and `GET /api/learn/cascade`

Summarize `.roko/learn/cascade-router.json` into model weights, routing stats, and per-category recommendations.

**Response:**
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

Cascade stages: `static` (<20 observations), `confidence` (<200), `ucb` (≥200).

### `GET /api/learning/cost-tiers` and `GET /api/learn/cost-tiers`

T0/T1/T2 routing distribution by model tier.

**Response:**
```json
{
  "T0": 10, "T1": 50, "T2": 20,
  "total": 80, "sample_count": 80,
  "t0_pct": 12.5, "t1_pct": 62.5, "t2_pct": 25.0
}
```

T0=fast/haiku, T1=standard/sonnet, T2=premium/opus.

### `GET /api/learning/experiments` and `GET /api/learn/experiments`

Full experiment store from `.roko/learn/experiments.json`.

**Response:**
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

### `GET /api/learning/adaptive-thresholds` and `GET /api/learn/adaptive-thresholds`

EMA-based adaptive gate threshold summary.

**Response:**
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

### `GET /api/learning/gate-thresholds` and `GET /api/learn/gate-thresholds`

Raw gate threshold data from `learning_policy_state` projection with source evidence.

### `GET /api/c-factor/trend`

C-Factor trend series from `.roko/learn/c-factor.jsonl`.

**Query parameters:**
- `window: string` — `"24h"` (default, 24 hourly buckets) or `"7d"` (168 hourly buckets)

**Response:**
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

### `GET /api/executor/state`

Return the executor snapshot from `executor_state` projection (`.roko/state/executor.json`).

---

## Benchmarks

Handler module: `routes/bench.rs`

### `POST /api/bench/run` and `POST /api/bench/runs`

Start a new benchmark run.

**Request body:**
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

**Response:** `202 Accepted`
```json
{ "id": "bench-run-uuid" }
```

Publishes `BenchRunStarted`, `BenchTaskStarted`, `BenchTaskCompleted`, `BenchLearningEvent`, `BenchProgress`, and `BenchRunCompleted` events to the event bus.

### `GET /api/bench/runs`

List bench runs (from index file).

**Query parameters:**
- `suite_id: string` — filter by suite
- `status: string` — filter by status (`running`, `completed`, `failed`, `cancelled`)
- `limit: usize` (default: 50)
- `offset: usize` (default: 0)

**Response:**
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

### `GET /api/bench/run/{id}` and `GET /api/bench/runs/{id}`

Get full bench run details including all task results.

**Response:** Full `BenchRun` object:
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

### `GET /api/bench/run/{id}/status` — lightweight status poll

**Response:**
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

### `DELETE /api/bench/run/{id}` and `POST /api/bench/runs/{id}/cancel`

Cancel a running bench run or delete a completed one. Returns `204 No Content`.

### `GET /api/bench/runs/compare?ids=a,b`

Compare multiple runs side by side.

**Query parameters:** `ids: string` — comma-separated run IDs

**Response:** `{ "runs": [...] }`

### `GET /api/bench/suites`

List available benchmark suites. Ensures built-in suites exist on disk.

**Response:**
```json
{
  "suites": [
    { "id": "learnable-rust", "name": "Learnable Rust", "description": "...", "task_count": 5 }
  ]
}
```

### `GET /api/bench/suites/{id}`

Get full suite with tasks.

### `POST /api/bench/suites`

Upload a custom benchmark suite.

**Request body:** Full `BenchSuite` object with `id` and `tasks`.

**Response:** `201 Created` with `{ "id": "<suite-id>" }`.

### `GET /api/bench/models`

List available models with pricing info.

**Response:** Array of model objects:
```json
[
  {
    "id": "claude-sonnet-4-6",
    "name": "claude-sonnet-4-6",
    "provider": "Anthropic",
    "cost_per_1k_input": 0.003,
    "cost_per_1k_output": 0.015,
    "max_tokens": 8192,
    "context_window": 200000
  }
]
```

### `GET /api/bench/pareto`

Compute the Pareto frontier (pass rate vs cost) across all completed runs.

**Response:** `{ "frontier": [...] }`

### `GET /api/bench/export/{id}`

Export a bench run as JSON (same as full run details).

### `GET /api/bench/events` and `GET /api/bench/runs/{id}/events`

SSE stream filtered to bench events only (`BenchRunStarted`, `BenchTaskStarted`, `BenchTaskCompleted`, `BenchLearningEvent`, `BenchProgress`, `BenchRunCompleted`).

**Event format:**
```
id: <seq>
data: {"type":"BenchTaskCompleted","bench_id":"...","task_id":"...","result":{...}}
```

---

## Dreams

Handler module: `routes/dream.rs`

### `POST /api/dream/run`

Trigger a dream consolidation cycle in the background. Runs `roko-dreams::DreamRunner::consolidate_now()`. Publishes `OperationStarted` and `OperationCompleted` events.

**Request body:**
```json
{ "mode": "full" }
```

`mode`: `"full"` (medium effort, default) or `"quick"` (low effort, bare mode).

**Response:** `202 Accepted`
```json
{ "id": "op-uuid" }
```

### `GET /api/dream/journal`

Return the dream journal shaped for the `DreamPhaseViz` component.

**Response:**
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

Journal entries are read from `.roko/dreams/journal.jsonl`.

---

## Knowledge (Neuro)

Handler module: `routes/neuro.rs`

### `POST /api/neuro/query`

Query the neuro knowledge store via HDC similarity search.

**Request body:**
```json
{
  "query": "error handling patterns in async Rust",
  "limit": 10,
  "min_tier": "warm"
}
```

**Validation:** `query` must be non-blank.

**Response:**
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

### `GET /api/knowledge?q=<topic>&limit=N`

Alias for `POST /api/neuro/query` using query parameters.

**Query parameters:**
- `q: string` — search query (returns empty result if blank)
- `limit: usize` (default: 10)

**Response:** Same shape as `/api/neuro/query`.

---

## Research

Handler module: `routes/research.rs`

### `GET /api/research`

List research artifacts from `.roko/research/`.

**Response:** Array of `{ "name": "...", "size": 4096, "is_file": true }`.

### `POST /api/research/topic`

Conduct deep research on a topic (agent-driven, background).

**Request body:**
```json
{
  "topic": "Rust async runtime design patterns",
  "intent": "explore",
  "depth": "comprehensive"
}
```

`intent`: `"position"`, `"evaluate"`, `"monitor"`, `"explore"`, `"audit"`.

**Response:** `202 Accepted` with `{ "operation_id": "op-uuid" }`.

### `POST /api/research/enhance-prd/{slug}`

Enhance a PRD with research findings.

**Response:** `202 Accepted`.

### `POST /api/research/enhance-plan/{plan}`

Enhance a plan with research findings.

**Response:** `202 Accepted`.

### `POST /api/research/enhance-tasks/{plan}`

Enhance tasks in a plan with research findings.

**Response:** `202 Accepted`.

### `POST /api/research/analyze`

Analyze execution data and generate insights.

**Response:** `202 Accepted` with `{ "operation_id": "op-uuid" }`.

---

## Jobs

Handler module: `routes/jobs.rs`

Jobs are backed by `.roko/jobs/*.json`.

Valid statuses: `open`, `assigned`, `in_progress`, `submitted`, `completed`, `failed`, `cancelled`.

State machine: `open → assigned/in_progress/cancelled`, `assigned → in_progress/open/cancelled`, `in_progress → submitted/failed/cancelled`, `submitted → completed/failed`, terminal: `completed`, `failed`, `cancelled`.

### `GET /api/jobs`

List jobs.

**Query parameters:**
- `status: string` — filter by status
- `job_type: string` — filter by job type
- `limit: usize` (default: 50)
- `offset: usize` (default: 0)

**Response:** `{ "jobs": [...], "total": N, "offset": 0, "limit": 50 }`

### `POST /api/jobs`

Create a new job. Publishes `JobCreated` event.

**Request body:**
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

**Validation:** `title` and `description` must be non-blank.

**Response:** `201 Created` with the full job object.

### `GET /api/jobs/stats`

Aggregate job counts by status.

**Response:** `{ "open": 5, "assigned": 2, "completed": 42, ... }`

### `POST /api/jobs/match`

Match available jobs to an agent's capabilities.

**Request body:**
```json
{ "agent_id": "...", "capabilities": ["rust", "testing"] }
```

**Response:** Array of matching jobs.

### `GET /api/jobs/{id}`

Get a job by ID.

### `PATCH /api/jobs/{id}`

Update job fields. Publishes `JobUpdated` event.

### `DELETE /api/jobs/{id}` (alias: `POST /api/jobs/{id}/cancel`)

Cancel a job. Publishes `JobTransitioned` event.

### `POST /api/jobs/{id}/assign`

Assign a job to an agent. Publishes `JobTransitioned` and `JobPostedToCandidate` events.

**Request body:** `{ "agent_id": "..." }`

### `POST /api/jobs/{id}/start`

Transition a job to `in_progress`.

### `POST /api/jobs/{id}/submit`

Submit completed work for a job.

**Request body:** `{ "output": "...", "artifacts": [...] }`

### `POST /api/jobs/{id}/evaluate`

Evaluate submitted work. Publishes `JobEvaluated` event.

**Request body:** `{ "accepted": true, "feedback": "Looks good" }`

### `POST /api/jobs/{id}/execute`

Execute a job using a local agent (background task). Publishes `JobExecutionStarted`, `JobProgress`, `JobAgentOutput`, `ChainTriageResult` events.

**Response:** `202 Accepted`.

---

## Deployments

Handler module: `routes/deployments.rs`

### `POST /api/deployments`

Create a cloud deployment from a template. Publishes `DeploymentCreated` event.

**Request body:**
```json
{
  "template": "railway-worker",
  "params": { "name": "my-agent", "region": "us-west-2" },
  "backend": "railway-api",
  "railway_token": "optional-per-request-token"
}
```

**Response:** `201 Created` with deployment object.

### `GET /api/deployments`

List all deployments.

**Query parameters:**
- `status: string` — filter by status
- `template: string` — filter by template name

### `GET /api/deployments/{id}`

Get deployment details.

### `DELETE /api/deployments/{id}`

Tear down a deployment. Publishes `DeploymentTornDown` event.

### `GET /api/deployments/{id}/logs`

Fetch deployment logs.

### `POST /api/deployments/{id}/task`

Proxy a task to a deployed worker.

### `POST /api/deployments/{id}/callback`

Receive a callback from a deployed worker. Publishes `WorkerTaskCompleted`.

---

## Inference Gateway

Handler module: `routes/gateway.rs`

The gateway provides centralized inference dispatch so agents never hold API keys directly. All requests flow through model selection (CascadeRouter), provider health tracking, cost accounting, and event publishing.

### `POST /api/inference/complete`

Submit a completion request to the inference gateway.

**Request body:**
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

**Response:**
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

### `GET /api/gateway/stats`

Gateway request statistics (totals, success rates, cost, active requests).

### `GET /api/gateway/models`

List models available through the gateway with their routing weights.

### `POST /api/inference/batch/submit`

Submit a batch of inference requests.

### `GET /api/inference/batch/{id}`

Get batch status and results.

---

## Providers and Models

Handler module: `routes/providers.rs`

### `GET /api/providers`

List configured providers with health and model counts.

**Response:**
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

### `GET /api/providers/{id}/health`

Health details for a specific provider.

### `POST /api/providers/{id}/test`

Send a test completion (`"Say hello."`) to a provider and return the response.

### `GET /api/models`

List all configured models with tier and pricing info.

### `GET /api/routing/explain`

Explain the routing decision for a hypothetical request.

**Query parameters:**
- `role: string` — agent role
- `task_category: string` — e.g. `"implementation"`
- `complexity: string` — `"fast"`, `"standard"`, `"complex"`

**Response:** `{ "selected_model": "...", "stage": "ucb", "reason": "...", "alternatives": [...] }`

---

## Config

Handler module: `routes/config.rs`

### `GET /api/config`

Return the current `RokoConfig` as JSON. Secret fields (API keys, tokens) are masked to `"***"`.

### `PUT /api/config`

Deep-merge a partial JSON config update into the current config and write to `roko.toml`. Publishes `ConfigReloaded` event.

**Request body:** Partial `RokoConfig` JSON (any subset of fields).

**Response:** Updated config as JSON.

### `POST /api/config/reload`

Reload `roko.toml` from disk and hot-apply compatible sections (logging, providers, subscriptions). Sections requiring a restart are noted. Publishes `ConfigReloaded` event.

**Response:**
```json
{
  "success": true,
  "warnings": [],
  "timestamp": "2026-04-07T12:00:00Z"
}
```

---

## Subscriptions and Workflows

Handler module: `routes/subscriptions.rs`, `routes/workflows.rs`

### Subscription routes

| Method | Path | Description |
|---|---|---|
| GET | `/api/subscriptions` | List event subscriptions |
| POST | `/api/subscriptions` | Create a subscription |
| GET | `/api/subscriptions/{id}` | Get subscription |
| DELETE | `/api/subscriptions/{id}` | Delete subscription |

### Workflow routes

| Method | Path | Description |
|---|---|---|
| GET | `/api/workflows` | List workflows |
| POST | `/api/workflows` | Create workflow |
| GET | `/api/workflows/{id}` | Get workflow |
| PUT | `/api/workflows/{id}` | Update workflow |
| DELETE | `/api/workflows/{id}` | Delete workflow |
| POST | `/api/workflows/{id}/trigger` | Trigger workflow |

---

## Heartbeats

Handler module: `routes/heartbeats.rs`

### `POST /api/heartbeats`

Receive a heartbeat from a client or agent sidecar. Used by `roko-agent-server`'s background heartbeat loop. Publishes `HeartbeatReceived` event.

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

**Response:** `200 OK`.

---

## Secrets

Handler module: `routes/secrets.rs`

Requires `admin` scope for all mutations.

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

Blockchain witness and chain client routes (Phase 2+).

| Method | Path | Description |
|---|---|---|
| GET | `/api/chain/status` | Chain connection status |
| POST | `/api/chain/witness` | Submit an episode witness |
| GET | `/api/chain/witnesses` | List witness records |

---

## Webhooks

Handler module: `routes/webhooks.rs` (no `/api/` prefix, always public)

### `POST /webhook/{source}`

Receive an inbound webhook. Validates the source, constructs an `Engram` signal, and publishes it to the event bus. Publishes `WebhookReceived` event.

**Path parameter:** `source` — webhook source name (e.g. `github`, `slack`)

**Request body:** Arbitrary JSON payload.

**Response:** `200 OK` with `{ "accepted": true, "signal_id": "..." }`.

---

## Terminal

Terminal routes are gated by `serve.terminal_enabled = true` in `roko.toml` (or `--enable-terminal` CLI flag). When disabled, all terminal routes return `403 Forbidden` with `{ "error": "Terminal disabled", "hint": "Set serve.terminal_enabled=true or use --enable-terminal" }`.

When enabled on a non-loopback bind address, terminal routes require API key auth (same as `/api/*`).

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

`roko-agent-server` (`crates/roko-agent-server/`) runs a per-agent HTTP server on a dynamic port (default: `0.0.0.0:0`, assigned by OS). It's built via `AgentServer::builder()` with feature flags controlling which route groups are mounted.

Sidecars register themselves with the control plane via `POST /api/agents/register` and send periodic heartbeats to `POST /api/heartbeats`. The control plane discovers them via `DiscoveredAgent` entries.

### Authentication

When built with `.auth(BearerAuth::new("token"))`, all non-public routes require:

```
Authorization: Bearer <token>
```

Public routes (`/health`, `/capabilities`) are always accessible without auth.

### Public routes (always available)

#### `GET /health`

```json
{ "status": "ok", "agent_id": "agent-uuid", "uptime_s": 3600 }
```

#### `GET /capabilities`

Capabilities manifest including feature flags, live routes, and skill configs.

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

**Important:** Feature capabilities (`messaging`, `predictions`, `research`, `tasks`) are only advertised if the corresponding feature is enabled via the builder (`AgentServerBuilder::messaging()`, etc.).

### Protected routes (require auth when enabled)

#### `GET /stats`

Runtime stats snapshot.

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

Stream the agent sidecar log file. Returns `text/plain`.

### Messaging feature (`builder.messaging()`)

#### `POST /message`

Dispatch a prompt to the agent's LLM backend and return the full response.

**Request body:**
```json
{
  "prompt": "Explain what you're working on",
  "context": { "thread": "xyz" }
}
```

**Response:**
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

Streaming dispatch via WebSocket. Send a plain text prompt frame; receive streaming chunks:

**Chunk frames:**
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

### Predictions feature (`builder.predictions()`)

#### `GET /predictions`

List all predictions.

**Response:** Array of `AgentPrediction` objects.

#### `POST /predictions`

Create a new prediction.

**Request body** (`PredictionCreateRequest`):
```json
{
  "market": "ETH-USD",
  "direction": "up",
  "confidence": 0.75,
  "horizon_secs": 3600,
  "metadata": {}
}
```

**Response:** `200 OK` with the created `AgentPrediction`.

#### `GET /predictions/{id}`

Get a prediction by ID. Returns `404` if not found.

#### `GET /predictions/residuals`

Get prediction residuals (predicted vs actual outcomes).

### Research feature (`builder.research()`)

#### `POST /research`

Submit a research query to the agent's knowledge store.

**Request body** (`ResearchRequest`):
```json
{
  "query": "Rust async patterns",
  "limit": 10
}
```

**Response** (`ResearchResponse`):
```json
{
  "entries": [...],
  "total": 5
}
```

### Tasks feature (`builder.tasks()`)

#### `GET /tasks`

List tasks in the agent's task queue.

**Response:** Array of `TaskEntry` objects.

#### `POST /tasks/{id}/accept`

Accept a task from the queue.

**Path parameter:** `id` — task ID (u64)

**Response:** `200 OK` with accepted `TaskEntry`, or `404`.

#### `POST /tasks/{id}/complete`

Mark a task as completed.

**Request body** (`TaskCompletionRequest`):
```json
{
  "output": "...",
  "success": true,
  "artifacts": []
}
```

**Response:** `200 OK` with updated `TaskEntry`, or `404`.

### Registration and Heartbeat

`AgentServer` can be configured with an `AgentRegistration` that publishes an `AgentCard` to the control plane on startup. It also runs a background heartbeat loop posting to `POST /api/heartbeats` at a configurable interval (default: 30 seconds).

**AgentCard** (advertised to control plane):
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

---

## Shared Run Pages

### `GET /runs/{id}`

Shareable HTML page for a specific run. No auth required. Served by `shared_runs::routes()` outside the `/api/` namespace.

---

## Miscellaneous Routes

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
