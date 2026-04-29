# Roko Control Plane HTTP API Reference

The roko control plane (`roko-serve`) exposes approximately 85 REST routes plus SSE and WebSocket streams on port **6677** by default. All API routes live under `/api/`. The embedded React demo app is served from `/` (SPA fallback). A bare liveness probe lives at `/health` (no `/api/` prefix).

## Table of Contents

1. [Starting the Server](#starting-the-server)
2. [Authentication](#authentication)
3. [Common Patterns](#common-patterns)
4. [SSE Streaming](#sse-streaming)
5. [Health and Status](#health-and-status)
6. [Plans](#plans)
7. [Agents](#agents)
8. [Episodes and Signals](#episodes-and-signals)
9. [Gates](#gates)
10. [Metrics](#metrics)
11. [Learning](#learning)
12. [C-Factor](#c-factor)
13. [Benchmarks](#benchmarks)
14. [Dreams](#dreams)
15. [Knowledge](#knowledge)
16. [PRDs](#prds)
17. [Runs](#runs)
18. [Jobs](#jobs)
19. [Deployments](#deployments)
20. [Providers and Models](#providers-and-models)
21. [Config](#config)
22. [Research](#research)
23. [Subscriptions and Workflows](#subscriptions-and-workflows)
24. [Agent Sidecar Aggregation](#agent-sidecar-aggregation)
25. [WebSocket](#websocket)
26. [Terminal](#terminal)
27. [OpenAPI](#openapi)
28. [Error Responses](#error-responses)

---

## Starting the Server

```bash
# Default: binds to 127.0.0.1:6677, reads roko.toml from current directory
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

[serve.auth]
enabled = true
api_key = "sk-my-secret-key"

# Named multi-key entries (optional)
[[serve.auth.api_keys]]
name = "ci-pipeline"
key_hash = "<sha256-hex-of-key>"
scope = "agent:write"
expires_at = "2027-01-01T00:00:00Z"
```

The `PORT` environment variable overrides the configured port and forces binding to `0.0.0.0`.

---

## Authentication

Authentication is **optional** and controlled by `serve.auth.enabled`. When enabled every `/api/` request must carry credentials. The bare `/health` probe and webhook endpoints bypass auth.

### Methods

| Method | Header | Example |
|---|---|---|
| API key | `X-Api-Key: <key>` | `X-Api-Key: sk-my-secret-key` |
| Bearer token | `Authorization: Bearer <token>` | `Authorization: Bearer sk-my-secret-key` |
| JWT (Privy) | `Authorization: Bearer <jwt>` | `Authorization: Bearer eyJ...` |
| Agent token | `Authorization: Bearer <agent-token>` | issued via `POST /api/agents/{id}/token` |

On success the server sets `X-Auth-Method: api_key | jwt | bearer` in the response.

Named API keys support scopes (`admin`, `agent:write`, `read`). The legacy single `api_key` always gets scope `admin`. Privy JWT users get scope `admin`. Agent tokens get scope `agent:write`.

### Scope enforcement

The `require_scope` middleware rejects requests where the caller's scope is insufficient for the operation. Most read routes accept `read` or higher; mutation routes require `agent:write` or `admin`.

### curl examples

```bash
# X-Api-Key header
curl -H "X-Api-Key: sk-secret" http://localhost:6677/api/health

# Bearer token
curl -H "Authorization: Bearer sk-secret" http://localhost:6677/api/health

# No auth (auth disabled — loopback default)
curl http://localhost:6677/api/health
```

---

## Common Patterns

### Pagination

Many list endpoints accept `?limit=N&offset=M`. Defaults vary per endpoint (typically `limit=20–50`). Responses include `total`, `limit`, `offset`, and optionally `has_more`.

### Projection query

Several endpoints accept a shared `ProjectionQuery` set of parameters:

| Parameter | Type | Description |
|---|---|---|
| `gate` | string | Filter by gate name |
| `limit` | usize | Max results |
| `plan_id` | string | Filter by plan |
| `task_id` | string | Filter by task |

### JSON content type

All request bodies must be `Content-Type: application/json`. All responses are `application/json` unless noted (Prometheus endpoint returns `text/plain`).

### Secret scrubbing

A response-layer middleware redacts API keys and tokens from JSON response bodies. Patterns matching known secret shapes are replaced with `[REDACTED]`.

---

## SSE Streaming

### `GET /api/events` (alias: `GET /api/sse`)

Real-time dashboard event stream backed by the StateHub ring buffer. Supports reconnection via the standard `Last-Event-ID` header — the server replays retained events from that sequence number before switching to live delivery.

**Request headers:**
- `Last-Event-ID: <seq>` — resume from sequence number (optional; defaults to 0, no replay)

**Response:** `text/event-stream`

Each frame:
```
id: <seq>
data: <json>
```

The `data` payload is a serialized `DashboardEvent`. Event types (the `type` field):

| Type | Description |
|---|---|
| `plan_started` | Plan execution began |
| `plan_completed` | Plan execution finished |
| `task_started` | Task entered active phase |
| `task_completed` | Task completed |
| `task_phase_changed` | Task moved between phases |
| `agent_spawned` | Agent process spawned |
| `agent_output` | Incremental agent output (sanitized) |
| `agent_completed` | Agent process finished |
| `gate_result` | Gate check completed |
| `phase_transition` | Plan changed execution phase |
| `efficiency_event` | Per-turn cost/token metric recorded |
| `episode_recorded` | Episode (turn + gate) recorded |
| `task_output_appended` | Task stdout appended |
| `event_log_entry` | Structured log entry |
| `cascade_router_updated` | Model routing table changed |
| `gate_thresholds_updated` | Adaptive thresholds changed |
| `marketplace_jobs_updated` | Job listings changed |
| `atelier_prds_updated` | PRD state changed |
| `knowledge_entries_updated` | Knowledge store changed |
| `job_execution_started` | Job runner started |
| `job_progress` | Job execution progress |
| `experiment_winners_updated` | A/B experiment resolved |
| `c_factor_trend_updated` | C-Factor recalculated |
| `diagnosis` | Conductor watcher fired |
| `error` | Server-side error |

**curl example:**
```bash
curl -N -H "Accept: text/event-stream" http://localhost:6677/api/events
```

**Reconnection:**
```bash
# Resume from sequence 42
curl -N -H "Last-Event-ID: 42" http://localhost:6677/api/events
```

Keep-alive pings are sent automatically (30 s interval).

---

### `GET /api/workflow/events`

SSE stream typed as `RuntimeEvent` — used by the WorkflowEngine surface. Same frame format as `/api/events`. Each event carries a `kind` field used as the SSE event name.

---

### `GET /api/bench/events` and `GET /api/bench/runs/:id/events`

SSE stream filtered to benchmark events only. Event types:

| Type | Fields |
|---|---|
| `bench_run_started` | `bench_id`, `suite_id`, `total_tasks` |
| `bench_task_started` | `bench_id`, `task_id`, `task_name`, `task_index`, `total_tasks` |
| `bench_task_completed` | `bench_id`, `task_id`, `result` (BenchTaskResult object) |
| `BenchLearningEvent` | `bench_id`, `task_id`, `playbooks_created`, `anti_patterns_created`, `total_playbooks`, `total_anti_patterns` |
| `bench_progress` | `bench_id`, `completed`, `total`, `cost_so_far` |
| `bench_run_completed` | `bench_id`, `summary` (BenchRunSummary object) |

---

## Health and Status

### `GET /health`

Bare liveness probe. No auth required. Returns `{"status":"ok"}` unconditionally. Use this for load balancer health checks.

```bash
curl http://localhost:6677/health
```

Response:
```json
{"status": "ok"}
```

---

### `GET /api/health`

Rich liveness check with live telemetry.

```bash
curl http://localhost:6677/api/health
```

Response:
```json
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_secs": 3721,
  "active_plans": 2,
  "active_agents": 4,
  "active_runs": 1,
  "providers": {
    "total": 3,
    "healthy": 3,
    "unhealthy": 0
  },
  "statehub": {
    "cursor": "0x1a4",
    "events_retained": 512,
    "snapshot": {
      "plans_active": 2,
      "tasks_active": 5,
      "agents_active": 4,
      "gates_passed": 128,
      "gates_failed": 7,
      "episodes_total": 312,
      "errors_total": 2,
      "cost_usd_total": 14.72
    }
  }
}
```

`status` is one of: `"ok"` (all providers healthy), `"degraded"` (some providers unhealthy), `"down"` (no healthy providers).

---

### `GET /api/status`

Session status overview: signal count, episode count, active processes, and the process session ledger.

```bash
curl http://localhost:6677/api/status
```

Response:
```json
{
  "session_id": "sess-abc123",
  "workdir": "/Users/will/project",
  "daemon_running": true,
  "signal_count": 4201,
  "episode_count": 312,
  "last_episode_passed": true,
  "supervised_processes": [],
  "process_session_ledger": "/Users/will/project/.roko/state/process-sessions.json",
  "process_sessions": {
    "active": 3,
    "completed_last_24h": 41
  }
}
```

---

### `GET /api/dashboard`

Dashboard scaffold rendered by the runtime. Returns minimal `{ rendered: string }`.

---

### `GET /api/operations/:id`

Look up a background operation by its UUID (e.g. from `POST /api/dream/run`).

Response:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "kind": "dream_run",
  "status": "Running"
}
```

Status is one of: `Running`, `Completed`, `Failed`.

---

### `GET /api/relay/health`

Relay connection diagnostics for multi-node deployments.

---

### `GET /api/parity`

Cross-surface parity matrix showing which features are implemented across CLI, TUI, API, and demo surfaces.

---

### `GET /api/retention`

Retention policy status. Returns configured policies and any current violations.

---

### `GET /api/statehub/snapshot`

Current StateHub dashboard snapshot as a projection state frame.

---

### `GET /api/statehub/events`

Bounded replay of retained StateHub events. Useful for polling clients that cannot maintain an SSE connection.

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `after_seq` | u64 | 0 | Return events after this sequence number |
| `limit` | usize | 256 | Max events (capped at 1024) |
| `run_id` | string | — | Alias for `plan_id` |
| `plan_id` | string | — | Filter to events for a plan |
| `task_id` | string | — | Filter to events for a task |
| `type` | string | — | Filter by event type (e.g. `gate_result`) |

Response:
```json
{
  "after_seq": 100,
  "limit": 256,
  "cursor": "0x1a4",
  "events": [
    {
      "seq": 101,
      "cursor": "0x65",
      "ts_millis": 1714406400000,
      "event": { "type": "gate_result", "plan_id": "...", "task_id": "...", "gate": "compile", "passed": true }
    }
  ]
}
```

---

## Plans

### `GET /api/plans`

List all plans from `.roko/plans/`. Returns plan summaries.

```bash
curl http://localhost:6677/api/plans
```

Response:
```json
[
  {
    "id": "prd-wire-system-prompt",
    "title": "Wire SystemPromptBuilder into orchestrate.rs",
    "task_count": 8,
    "completed": false,
    "completed_task_count": 6
  }
]
```

---

### `GET /api/plans/:id`

Load a specific plan by ID (matches the plan filename stem or plan `id` field).

```bash
curl http://localhost:6677/api/plans/prd-wire-system-prompt
```

Response: full plan object with `id`, `title`, and `tasks` array.

---

### `POST /api/plans`

Create a new plan.

Request body:
```json
{
  "title": "My new plan",
  "tasks": [
    { "id": "task-1", "title": "First task", "description": "..." }
  ]
}
```

---

### `GET /api/plans/:id/tasks`

List tasks for a plan.

---

### `POST /api/plans/:id/execute`

Start executing a plan. Spawns the orchestration loop in the background.

Response:
```json
{ "op_id": "550e8400-..." }
```

---

### `GET /api/plans/:id/status`

Get execution status of a running plan.

---

### `POST /api/plans/:id/pause`

Pause a running plan.

---

### `POST /api/plans/:id/resume`

Resume a paused plan.

---

### `GET /api/plans/:id/gates`

Gate results for all tasks in a plan.

---

### `GET /api/plans/:id/reviews`

List human review requests for a plan's tasks.

---

### `POST /api/plans/:id/tasks/:task_id/review`

Submit a human review decision for a task.

---

### `GET /api/plans/:id/tasks/:task_id/diff`

Get the git diff produced by a task.

---

### `POST /api/plans/:id/chat`

Send a follow-up message to the plan's agent.

---

### `POST /api/plans/:id/estimate`

Estimate cost and token usage for a plan without executing it.

---

### `POST /api/plans/generate`

Generate a plan from a natural-language prompt.

Request body:
```json
{ "prompt": "Add rate limiting to all API endpoints" }
```

---

## Agents

### `GET /api/agents`

List agents aggregated from all discovered agent sidecars. Uses a 30 s TTL cache.

**Query parameters:**

| Parameter | Type | Description |
|---|---|---|
| `owner` | string | Filter by owner ID |

Response:
```json
{
  "agents": [
    {
      "id": "agent-abc",
      "name": "Implementer",
      "role": "Implementer",
      "status": "active",
      "sidecar_url": "http://127.0.0.1:7001",
      "last_heartbeat_at": "2026-04-29T12:00:00Z"
    }
  ]
}
```

---

### `GET /api/managed-agents`

List locally supervised agent processes plus registered/discovered agents. Used by the dashboard fleet roster.

Response:
```json
{
  "agents": [
    {
      "agent_id": "agent-abc",
      "label": "Implementer",
      "pid": 12345,
      "status": "running",
      "url": "http://127.0.0.1:7001",
      "last_seen": "2026-04-29T12:00:00Z"
    }
  ]
}
```

---

### `POST /api/agents/register`

Register a remote agent sidecar with the control plane.

Request body:
```json
{
  "agent_id": "agent-abc",
  "role": "Implementer",
  "url": "http://127.0.0.1:7001"
}
```

---

### `POST /api/agents/create`

Create a new agent from a manifest.

---

### `GET /api/agents/:id`

Get agent details.

---

### `GET /api/agents/:id/profile`

Get an agent's model profile (role, model, MCP config, etc.).

---

### `POST /api/agents/:id/start`

Start an agent process.

---

### `POST /api/agents/:id/stop`

Stop an agent process.

---

### `POST /api/agents/:id/restart`

Restart an agent process.

---

### `GET /api/agents/:id/episodes`

Episodes recorded by a specific agent.

---

### `GET /api/agents/:id/logs`

Proxy to the agent sidecar's logs endpoint.

---

### `POST /api/agents/:id/message`

Send a message to an agent inline (30 s timeout) and return the response.

Request body:
```json
{ "content": "What is the current state of task-3?" }
```

---

### `GET /api/agents/:id/token`

Get token status for an agent.

---

### `POST /api/agents/:id/token`

Issue a new agent authentication token.

Response:
```json
{ "token": "roko-agent-<uuid>" }
```

---

### `GET /api/agents/topology`

Agent fleet topology as a graph. Returns nodes and edges derived from episode lineage.

Response:
```json
{
  "nodes": [
    { "id": "agent-abc", "role": "Implementer", "status": "active" }
  ],
  "edges": [
    { "source": "agent-abc", "target": "agent-def", "kind": "handoff" }
  ]
}
```

---

### `GET /api/agents/:id/stats`

Stats for a specific agent from its sidecar (5 s TTL cache).

---

### `GET /api/agents/:id/skills`

Skill declarations from the agent sidecar.

---

### `GET /api/agents/:id/heartbeat`

Most recent heartbeat from the agent.

---

### `GET /api/agents/:id/trace`

Recent agent trace entries.

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `limit` | usize | 10 | Max entries |
| `offset` | usize | 0 | Skip N entries |

---

## Episodes and Signals

### `GET /api/episodes`

Normalized episode proof rows from canonical projections. Episodes represent one completed agent turn plus gate verdicts.

**Query parameters:** `ProjectionQuery` fields (see [Common Patterns](#common-patterns)).

```bash
curl http://localhost:6677/api/episodes
```

Response: JSON array of episode objects.
```json
[
  {
    "id": "ep-abc123",
    "agent_id": "agent-1",
    "plan_id": "plan-abc",
    "task_id": "task-1",
    "timestamp": "2026-04-29T10:00:00Z",
    "gate_passed": true,
    "cost_usd": 0.42,
    "input_tokens": 8000,
    "output_tokens": 1200,
    "hdc_fingerprint": "0xdeadbeef"
  }
]
```

---

### `GET /api/signals`

Read raw signals (engrams) JSONL as a JSON array. Signals are the fundamental persisted records backing all analytics.

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `limit` | usize | 1000 | Max signals (server cap applies) |

```bash
curl "http://localhost:6677/api/signals?limit=50"
```

---

## Gates

### `GET /api/gates/summary`

Aggregate gate verdicts across all gates. Returns pass rates, run counts, and per-rung breakdown.

```bash
curl http://localhost:6677/api/gates/summary
```

Response:
```json
{
  "compile": {
    "total_runs": 48,
    "pass_rate": 0.9375,
    "avg_duration_ms": 4200.0,
    "last_run": { ... }
  },
  "test": {
    "total_runs": 42,
    "pass_rate": 0.857,
    "avg_duration_ms": 18200.0,
    "last_run": { ... }
  },
  "rungs": [
    { "rung": 0, "passed_runs": 45, "failed_runs": 3, "total_runs": 48, "pass_rate": 0.9375 },
    { "rung": 1, "passed_runs": 40, "failed_runs": 2, "total_runs": 42, "pass_rate": 0.952 }
  ]
}
```

Rung-to-gate mapping: `0=compile`, `1=clippy`, `2=test`, `3=diff`, `4=fmt`, `5=custom`, `6=judge`.

---

### `GET /api/gates/history`

Recent gate verdicts across all gates, newest first.

**Query parameters:**

| Parameter | Type | Description |
|---|---|---|
| `gate` | string | Filter to a specific gate name |
| `limit` | usize | Max results |
| `format` | string | Pass `waterfall` for grouped format (see below) |

```bash
curl "http://localhost:6677/api/gates/history?limit=20"
```

Response:
```json
{
  "sources": ["/path/.roko/engrams.jsonl"],
  "total": 312,
  "limit": 20,
  "history": [
    {
      "signal_id": "sig-abc",
      "created_at_ms": 1714406400000,
      "gate": "compile",
      "passed": true,
      "duration_ms": 3800,
      "plan_id": "plan-abc",
      "task_id": "task-1",
      "rung": 0
    }
  ]
}
```

**Waterfall format** (`?format=waterfall`): groups results by `task_id` into run objects with nested rung arrays — the shape expected by the `GateWaterfall` demo component.

```bash
curl "http://localhost:6677/api/gates/history?format=waterfall&limit=10"
```

Response:
```json
[
  {
    "task_id": "task-1",
    "timestamp": 1714406400000,
    "rungs": [
      { "name": "compile", "rung": 0, "status": "passed", "duration_ms": 3800 },
      { "name": "clippy",  "rung": 1, "status": "passed", "duration_ms": 1200 },
      { "name": "test",    "rung": 2, "status": "failed", "duration_ms": 22100 }
    ]
  }
]
```

---

### `GET /api/gates/:gate_name/history`

Time series of pass/fail results for a specific gate.

```bash
curl http://localhost:6677/api/gates/compile/history
```

Returns `404` if the gate has no recorded verdicts.

---

## Metrics

### `GET /api/metrics`

Raw metric snapshots as JSON (internal `MetricStore` format).

---

### `GET /api/metrics/summary`

Aggregate recent execution and learning metrics for a time window.

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `period` | string | `last_7_days` | `last_7_days`, `last_30_days`, `last_90_days`, or `last_N_days` |

```bash
curl "http://localhost:6677/api/metrics/summary?period=last_30_days"
```

Response:
```json
{
  "period": "last_7_days",
  "agents_run": 312,
  "success_rate": 0.874,
  "feedback_engagement_rate": 0.91,
  "avg_cost_per_episode_cents": 14,
  "experiments_active": 2,
  "best_experiment_lift": {
    "name": "system_prompt",
    "lift": 0.12,
    "winning": "Verbose"
  },
  "gate_pass_rate": 0.921,
  "self_improvement_velocity": 0.008,
  "c_factor": 0.71,
  "active_plans": 2,
  "top_templates": [
    { "name": "implementer", "runs": 148, "success_rate": 0.89 }
  ]
}
```

---

### `GET /api/metrics/success_rate`

Per-template success rate split by trigger kind.

Response:
```json
{
  "templates": [
    {
      "template": "implementer",
      "triggers": [
        { "trigger_kind": "plan", "successful_episodes": 130, "total_episodes": 148, "success_rate": 0.878 }
      ]
    }
  ]
}
```

---

### `GET /api/metrics/engagement`

Feedback acknowledgement ratio per template.

---

### `GET /api/metrics/c_factor`

Composite C-Factor, component sub-metrics, per-agent contributions, and per-fleet roll-up.

```bash
curl http://localhost:6677/api/metrics/c_factor
```

Response:
```json
{
  "source": {
    "composite_history_path": "/path/.roko/learn/c-factor.jsonl",
    "efficiency_events_path": "/path/.roko/learn/efficiency.jsonl",
    "composite_history_count": 48,
    "efficiency_event_count": 312
  },
  "composite": {
    "overall": 0.71,
    "computed_at": "2026-04-29T12:00:00Z",
    "episode_count": 312,
    "history_count": 48
  },
  "sub_metrics": {
    "gate_pass_rate": 0.80,
    "cost_efficiency": 0.60,
    "speed": 0.55,
    "information_flow_rate": 0.40,
    "first_try_rate": 0.75,
    "knowledge_growth": 0.30,
    "knowledge_integration_rate": 0.25,
    "hdc_diversity": 0.35,
    "convergence_velocity": 0.45,
    "turn_taking_equality": 0.50,
    "social_perceptiveness": 0.65
  },
  "per_agent": [
    {
      "agent_id": "agent-abc",
      "episode_count": 148,
      "without_agent_overall": 0.58,
      "contribution_score": 0.13,
      "dispatch_bias": "prefer_cheaper"
    }
  ],
  "per_fleet": {
    "plan_count": 12,
    "agent_count": 4,
    "observation_count": 312
  }
}
```

---

### `GET /api/metrics/model_efficiency`

Cost per successful episode for each routed model.

Response:
```json
{
  "source": "/path/.roko/learn/cascade-router.json",
  "current_stage": "confidence",
  "total_observations": 80,
  "models": [
    {
      "model": "claude-haiku-4-5",
      "total_episodes": 120,
      "successful_episodes": 104,
      "total_cost_usd": 2.40,
      "cost_per_successful_episode_usd": 0.023,
      "success_rate": 0.867
    }
  ]
}
```

---

### `GET /api/metrics/gate_rate`

Gate pass/total with trend delta (improving / flat / declining).

---

### `GET /api/metrics/experiments`

Best vs worst variant gap for each active experiment.

---

### `GET /api/metrics/feedback_latency`

Median hours from agent action to first feedback (gate verdict) signal.

Response:
```json
{
  "sample_count": 87,
  "median_hours": 0.0042
}
```

---

### `GET /api/metrics/velocity`

Rate of change of success rate over time (success-rate improvement per day).

---

### `GET /api/metrics/coverage`

Percentage of event bus events that matched a known subscription term.

---

### `GET /api/metrics/prometheus`

Prometheus text exposition format (`text/plain; version=0.0.4`). Suitable for scraping by a Prometheus server.

```bash
curl http://localhost:6677/api/metrics/prometheus
```

Exposed metrics:
```
roko_uptime_seconds (gauge)
roko_agents_active (gauge)
roko_plans_active (gauge)
roko_plans_completed_total (counter)
roko_plans_failed_total (counter)
roko_tasks_completed_total (counter)
roko_tasks_failed_total (counter)
roko_tasks_active (gauge)
roko_gate_pass_total (counter)
roko_gate_fail_total (counter)
roko_errors_total (counter)
roko_episodes_total (counter)
```

---

## Learning

All learning endpoints have aliases — both `/api/learning/` and `/api/learn/` prefixes work identically.

### `GET /api/learn/efficiency` (alias: `/api/learning/efficiency`)

Aggregate efficiency events from `.roko/learn/efficiency.jsonl`.

```bash
curl http://localhost:6677/api/learn/efficiency
```

Response:
```json
{
  "total_cost": 14.72,
  "cost_per_task": 0.184,
  "tokens_per_task": 4200.0,
  "avg_task_duration": 18500.0,
  "cost_trend": [
    {
      "timestamp": "2026-04-29T10:00:00Z",
      "cost_usd": 0.42,
      "cumulative_cost_usd": 0.42
    }
  ],
  "tasks": [
    {
      "plan_id": "plan-abc",
      "task_id": "task-1",
      "timestamp": "2026-04-29T10:00:00Z",
      "cost_usd": 0.42,
      "tokens": 9200,
      "duration_ms": 22000,
      "sequence": 0
    }
  ],
  "evidence": { "state": "loaded" }
}
```

---

### `GET /api/learn/costs` (alias: `/api/learning/costs`)

Canonical runtime cost projection with breakdown by plan, task, and agent.

---

### `GET /api/learn/provider-outcomes` (alias: `/api/learning/provider-outcomes`)

Provider health data. Returns per-provider success rate, latency, and cost stats.

```bash
curl http://localhost:6677/api/learn/provider-outcomes
```

Response:
```json
{
  "providers": [
    {
      "name": "anthropic",
      "status": "healthy",
      "models": ["claude-sonnet-4-6", "claude-haiku-4-5"],
      "success_rate": 0.982,
      "avg_latency_ms": 4200,
      "p95_latency_ms": 6300,
      "cost_per_1k_tokens": 0.003,
      "total_requests": 312,
      "errors_24h": 2,
      "last_error": ""
    }
  ]
}
```

Status is `"healthy"` (≥97%), `"degraded"` (≥90%), or `"unhealthy"` (<90%).

---

### `GET /api/learn/router` (alias: `/api/learning/cascade-router`, `/api/learn/cascade-router`)

Cascade router snapshot from `.roko/learn/cascade-router.json`.

```bash
curl http://localhost:6677/api/learn/router
```

Response:
```json
{
  "source": "/path/.roko/learn/cascade-router.json",
  "projection_state": "loaded",
  "model_slugs": ["claude-sonnet-4-6", "claude-haiku-4-5"],
  "routing_stats": {
    "total_observations": 80,
    "stage": "confidence"
  },
  "model_weights": [
    { "model": "claude-sonnet-4-6", "weight": 0.75, "success_rate": 0.90, "trials": 50 }
  ]
}
```

---

### `GET /api/learning/cascade` (alias: `/api/learn/cascade`)

Full cascade routing summary with model weights and recommendations.

---

### `GET /api/learn/experiments` (alias: `/api/learning/experiments`)

Active and concluded A/B prompt experiments from `.roko/learn/experiments.json`.

```bash
curl http://localhost:6677/api/learn/experiments
```

Response:
```json
{
  "running_experiments": 2,
  "concluded_experiments": 1,
  "active_experiments": [
    {
      "experiment_id": "exp-001",
      "section_name": "system_prompt",
      "status": "Running",
      "variants": [
        {
          "id": "baseline",
          "name": "Baseline",
          "active": true,
          "trials": 40,
          "successes": 34,
          "success_rate": 0.85
        },
        {
          "id": "verbose",
          "name": "Verbose",
          "active": true,
          "trials": 38,
          "successes": 36,
          "success_rate": 0.947
        }
      ]
    }
  ]
}
```

---

### `GET /api/learn/adaptive-thresholds` (alias: `/api/learning/adaptive-thresholds`)

Per-rung adaptive gate threshold state from `.roko/learn/gate-thresholds.json`.

```bash
curl http://localhost:6677/api/learn/adaptive-thresholds
```

Response:
```json
{
  "source": "/path/.roko/learn/gate-thresholds.json",
  "tracked_rungs": 4,
  "thresholds": {
    "compile": {
      "mean_pass_rate": 0.94,
      "ema_threshold": 0.70,
      "rung_count": 48,
      "consecutive_passes": 12
    },
    "clippy": {
      "mean_pass_rate": 0.88,
      "ema_threshold": 0.70,
      "rung_count": 42,
      "consecutive_passes": 7
    },
    "test": {
      "mean_pass_rate": 0.81,
      "ema_threshold": 0.70,
      "rung_count": 38,
      "consecutive_passes": 3
    }
  }
}
```

---

### `GET /api/learn/gate-thresholds` (alias: `/api/learning/gate-thresholds`)

Raw gate thresholds from the projection store (different view than `adaptive-thresholds`).

---

### `GET /api/learn/cost-tiers` (alias: `/api/learning/cost-tiers`)

Model cost tier breakdown (T0 = haiku-class, T1 = sonnet-class, T2 = opus-class).

```bash
curl http://localhost:6677/api/learn/cost-tiers
```

Response:
```json
{
  "total": 80,
  "T0": 30,
  "T1": 45,
  "T2": 5
}
```

---

### `GET /api/learning/retries` (alias: `/api/learn/retries`)

Retry attempt proof surface from the runtime projection.

---

### `GET /api/learning/runtime-feedback` (alias: `/api/learn/runtime-feedback`)

Joined feedback store overview from the runtime projection.

---

### `GET /api/executor/state`

Executor snapshot from the runtime projection — task DAG, phase state, and merge queue status.

---

## C-Factor

### `GET /api/c-factor/trend`

C-Factor trend series from `.roko/learn/c-factor.jsonl`, bucketed by hour.

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `window` | string | `24h` | `24h` or `7d` |

```bash
curl "http://localhost:6677/api/c-factor/trend?window=7d"
```

Response:
```json
{
  "trend": [
    {
      "bucket_start": "2026-04-29T00:00:00Z",
      "avg": 0.68,
      "min": 0.61,
      "max": 0.74,
      "samples": 4
    }
  ],
  "woolley": {
    "turn_taking_equality": [0.71, 0.72, 0.74],
    "social_perceptiveness": [0.68, 0.70, 0.69],
    "citation_reciprocity": [0.70, 0.71, 0.72],
    "delivery_rate": [0.67, 0.69, 0.68],
    "hdc_diversity": [0.72, 0.73, 0.75]
  }
}
```

Returns `{ "trend": [], "woolley": {} }` when no data exists yet.

---

## Benchmarks

### `POST /api/bench/runs`

Start a new benchmark run. Returns immediately (HTTP 202) with a run ID; execution is asynchronous.

Request body:
```json
{
  "suite_id": "learnable-rust",
  "label": "main-branch-baseline",
  "overrides": {
    "model": "claude-haiku-4-5",
    "strategy": "standard"
  }
}
```

The `overrides`/`config` fields (aliased) accept:
- `model` — model slug override
- `strategy` — `"standard"`, `"minimal"`, or `"full"`

Response (202 Accepted):
```json
{ "id": "550e8400-e29b-41d4-a716-446655440000" }
```

```bash
curl -X POST http://localhost:6677/api/bench/runs \
  -H "Content-Type: application/json" \
  -d '{"suite_id":"learnable-rust","overrides":{"model":"claude-haiku-4-5"}}'
```

---

### `GET /api/bench/runs`

List bench runs in reverse chronological order.

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `suite_id` | string | — | Filter by suite |
| `status` | string | — | Filter by status (`running`, `completed`, `failed`, `cancelled`) |
| `limit` | usize | 50 | Max runs |
| `offset` | usize | 0 | Pagination offset |

Response:
```json
{
  "total": 12,
  "offset": 0,
  "limit": 50,
  "runs": [
    {
      "id": "550e8400-...",
      "suite_id": "learnable-rust",
      "suite_name": "Learnable Rust",
      "status": "completed",
      "started_at": 1714406400,
      "finished_at": 1714407200,
      "label": "main-branch-baseline",
      "model": "claude-haiku-4-5",
      "pass_rate": 0.8,
      "total_cost_usd": 0.24
    }
  ]
}
```

---

### `GET /api/bench/runs/:id`

Full bench run details including per-task results.

```bash
curl http://localhost:6677/api/bench/runs/550e8400-e29b-41d4-a716-446655440000
```

Response:
```json
{
  "id": "550e8400-...",
  "suite_id": "learnable-rust",
  "suite_name": "Learnable Rust",
  "kind": "manual",
  "status": "completed",
  "started_at": 1714406400,
  "finished_at": 1714407200,
  "current_task_index": 5,
  "total_tasks": 5,
  "results": [
    {
      "task_id": "task-1",
      "task_name": "Implement format_greeting",
      "passed": true,
      "duration_ms": 4200,
      "model_used": "claude-haiku-4-5",
      "input_tokens": 1800,
      "output_tokens": 320,
      "cost_usd": 0.048,
      "output_preview": "pub fn format_greeting...",
      "error": null
    }
  ],
  "summary": {
    "total_tasks": 5,
    "passed": 4,
    "failed": 1,
    "pass_rate": 0.8,
    "total_cost_usd": 0.24,
    "total_duration_ms": 18400
  }
}
```

---

### `GET /api/bench/runs/:id/events`

SSE stream for a specific bench run. See [SSE Streaming — Bench events](#get-apibenchevents-and-get-apibenchrunsrideviews).

---

### `POST /api/bench/runs/:id/cancel`

Cancel a running bench run. Returns 204 No Content.

```bash
curl -X POST http://localhost:6677/api/bench/runs/550e8400-.../cancel
```

---

### `GET /api/bench/runs/compare`

Compare multiple bench runs side by side.

**Query parameters:**

| Parameter | Type | Description |
|---|---|---|
| `ids` | string | Comma-separated run IDs |

```bash
curl "http://localhost:6677/api/bench/runs/compare?ids=run-a,run-b"
```

Response:
```json
{
  "runs": [ { ...full run object... }, { ...full run object... } ]
}
```

---

### `GET /api/bench/suites`

List available benchmark suites.

```bash
curl http://localhost:6677/api/bench/suites
```

Response:
```json
{
  "suites": [
    {
      "id": "learnable-rust",
      "name": "Learnable Rust",
      "description": "5-task Rust implementation suite that tests learning transfer",
      "task_count": 5
    }
  ]
}
```

---

### `GET /api/bench/suites/:id`

Get a specific suite including its full task list.

---

### `POST /api/bench/suites`

Upload a custom benchmark suite.

Request body: full `BenchSuite` object with `id`, `name`, `description`, and `tasks[]`.

Response (201 Created):
```json
{ "id": "my-custom-suite" }
```

---

### `GET /api/bench/models`

List available models with pricing information.

```bash
curl http://localhost:6677/api/bench/models
```

Response:
```json
[
  {
    "id": "claude-haiku-4-5",
    "name": "claude-haiku-4-5",
    "provider": "Anthropic",
    "cost_per_1k_input": 0.00025,
    "cost_per_1k_output": 0.00125,
    "max_tokens": 8192,
    "context_window": 200000
  }
]
```

---

### `GET /api/bench/pareto`

Compute Pareto frontier across all completed bench runs (pass rate vs. cost).

```bash
curl http://localhost:6677/api/bench/pareto
```

Response:
```json
{
  "frontier": [
    {
      "run_id": "run-abc",
      "model": "claude-haiku-4-5",
      "pass_rate": 0.8,
      "total_cost_usd": 0.24
    }
  ]
}
```

---

### `GET /api/bench/export/:id`

Export a bench run as JSON (same shape as `GET /api/bench/runs/:id` but always fully loaded from disk).

```bash
curl http://localhost:6677/api/bench/export/550e8400-... > bench-run.json
```

---

## Dreams

### `POST /api/dream/run`

Trigger a dream consolidation cycle (Hypnagogia → NREM → REM → Integration) in the background.

Request body:
```json
{ "mode": "full" }
```

`mode` is `"full"` (default) or `"quick"` (lower effort).

Response (202 Accepted):
```json
{ "id": "550e8400-..." }
```

Poll `GET /api/operations/:id` for completion status.

```bash
curl -X POST http://localhost:6677/api/dream/run \
  -H "Content-Type: application/json" \
  -d '{"mode":"quick"}'
```

---

### `GET /api/dream/journal`

Dream journal shaped for the `DreamPhaseViz` component.

```bash
curl http://localhost:6677/api/dream/journal
```

Response:
```json
{
  "last_cycle": "2026-04-29T02:00:00Z",
  "cycle_count": 12,
  "phases": [
    {
      "name": "Hypnagogia",
      "status": "completed",
      "episodes_processed": 42,
      "clusters_formed": 8,
      "knowledge_entries_written": 24,
      "playbooks_created": 6,
      "duration_secs": 14,
      "trend": []
    },
    { "name": "NREM", "status": "completed", ... },
    { "name": "REM", "status": "completed", ... },
    { "name": "Integration", "status": "completed", ... }
  ]
}
```

---

## Knowledge

### `GET /api/knowledge/entries`

List knowledge store entries aggregated from all agent sidecars (30 s TTL cache).

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `limit` | usize | 20 | Max entries |
| `offset` | usize | 0 | Pagination |

Response:
```json
{
  "items": [
    {
      "id": "kn-abc",
      "kind": "playbook",
      "title": "Always check borrow lifetimes before pattern-matching",
      "source_agent_id": "agent-1",
      "created_at": "2026-04-29T10:00:00Z"
    }
  ],
  "total": 184,
  "offset": 0,
  "limit": 20,
  "has_more": true
}
```

---

### `GET /api/knowledge/edges`

Knowledge graph edges (relationships between knowledge entries).

---

### `GET /api/knowledge/search`

Full-text knowledge search across all agent sidecars.

**Query parameters:**

| Parameter | Type | Description |
|---|---|---|
| `q` | string | Search query (required) |

```bash
curl "http://localhost:6677/api/knowledge/search?q=borrow+checker"
```

---

### `GET /api/knowledge/kinds`

Distinct knowledge entry kinds in the store.

---

## PRDs

### `GET /api/prds`

List PRDs.

---

### `POST /api/prds`

Create a new PRD.

---

### `GET /api/prds/:id`

Get a specific PRD.

---

### `PATCH /api/prds/:id`

Update a PRD.

---

### `POST /api/prds/:id/publish`

Publish a PRD. If `prd.auto_plan` is enabled in `roko.toml`, triggers automatic plan generation.

---

## Runs

### `POST /api/run`

Start a one-shot run (the universal loop: compose → agent → gate → persist).

Request body:
```json
{
  "prompt": "Add a docstring to every public function in src/lib.rs",
  "model": "claude-sonnet-4-6"
}
```

Response (202 Accepted):
```json
{ "run_id": "run-550e8400-..." }
```

---

### `GET /api/runs`

List recent one-shot runs.

---

### `GET /api/runs/:id`

Get a specific run's status and output.

---

### `GET /runs/:id`

Shareable run page — returns HTML (the embedded SPA) for this run ID. No auth required (serves public share pages).

---

## Jobs

### `GET /api/jobs`

List marketplace jobs.

---

### `POST /api/jobs`

Create a new job.

---

### `GET /api/jobs/:id`

Get a specific job.

---

### `POST /api/jobs/:id/execute`

Execute a job.

---

### `POST /api/jobs/:id/cancel`

Cancel a running job.

---

## Deployments

### `GET /api/deployments`

List cloud deployments.

---

### `POST /api/deployments`

Create a new deployment (Railway, Fly, Docker).

---

### `GET /api/deployments/:id`

Get deployment status.

---

### `DELETE /api/deployments/:id`

Tear down a deployment.

---

## Providers and Models

### `GET /api/providers`

List configured LLM providers with health status.

Response:
```json
{
  "providers": [
    {
      "name": "anthropic",
      "kind": "claude_api",
      "status": "healthy",
      "consecutive_failures": 0,
      "models": ["claude-sonnet-4-6", "claude-haiku-4-5"]
    }
  ]
}
```

---

### `GET /api/providers/:name/health`

Health check for a specific provider.

---

### `POST /api/providers/:name/test`

Test a provider with a minimal request.

---

### `GET /api/models`

List all available models across all providers.

---

### `GET /api/models/route`

Show model routing decision for a hypothetical request.

---

### `GET /api/routing`

Current routing policy and weights.

---

## Config

### `GET /api/config`

Return current `roko.toml` as JSON (secrets are redacted).

---

### `POST /api/config/reload`

Hot-reload config from disk. Publishes a `ConfigReloaded` SSE event.

---

### `GET /api/config/subscriptions`

List event subscriptions.

---

### `POST /api/config/subscriptions`

Add an event subscription.

---

### `DELETE /api/config/subscriptions/:id`

Remove an event subscription.

---

### `GET /api/secrets`

List secret keys (values are never returned).

---

### `POST /api/secrets`

Set a secret.

---

## Research

### `POST /api/research`

Start a research task.

---

### `GET /api/research/:id`

Get research task status and results.

---

## Subscriptions and Workflows

### `GET /api/subscriptions`

List webhook/event subscriptions.

---

### `GET /api/workflows`

List workflow definitions.

---

### `GET /api/workflow/events`

SSE stream for workflow engine events. See [SSE Streaming](#sse-streaming).

---

## Agent Sidecar Aggregation

These routes aggregate data from all discovered agent sidecars and are used by the demo dashboard.

### `GET /api/tasks`

Tasks across all agent sidecars.

**Query parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `state` | string | — | Filter by task state |
| `kind` | string | — | Filter by task kind |
| `assignee` | string | — | Filter by assignee agent ID |
| `limit` | usize | 20 | Max tasks |
| `offset` | usize | 0 | Pagination |

---

### `GET /api/tasks/stats`

Aggregated task statistics.

---

### `GET /api/tasks/:id`

Get a specific task.

---

### `GET /api/predictions/sessions`

Prediction sessions across all agent sidecars.

---

### `GET /api/predictions/sessions/:id`

A specific prediction session.

---

### `GET /api/predictions/claims`

Prediction claims (scored predictions) across the fleet.

---

### `GET /api/predictions/calibration/:agent_id`

Calibration metrics for a specific agent's predictions.

---

## WebSocket

### `GET /ws`

WebSocket upgrade endpoint. After upgrade, clients can:
- Send `{ "type": "subscribe", "targets": ["agent-id-1", "agent-id-2"] }` to multiplex streams from multiple agent sidecars
- Receive muxed messages as `{ "source": "agent-id-1", "event": { ... } }`
- Send `{ "type": "message", "agent_id": "agent-id-1", "message": { ... } }` to forward a message to a sidecar

When auth is enabled the WebSocket upgrade requires a valid API key.

---

## Terminal

Terminal endpoints are disabled by default. Enable with `serve.terminal_enabled = true` in `roko.toml` or `--enable-terminal` flag.

When the server binds to a public address (`0.0.0.0` or non-loopback), terminal routes require API key auth even when auth is otherwise disabled.

### `GET /api/terminal/sessions`

List active PTY terminal sessions.

Response: `{ "sessions": [] }` when none active, or `403` when terminal is disabled.

### `POST /api/terminal/sessions`

Create a new PTY session.

### `DELETE /api/terminal/sessions/:id`

Destroy a terminal session.

---

## OpenAPI

### `GET /api/openapi.json`

OpenAPI 3.0 schema for the full API.

```bash
curl http://localhost:6677/api/openapi.json | jq '.info'
```

---

## Error Responses

All error responses follow a consistent shape:

```json
{
  "error": "human readable message",
  "code": "machine_readable_code",
  "status": 404
}
```

Common status codes:

| Status | Code | Meaning |
|---|---|---|
| 400 | `bad_request` | Invalid input or missing required field |
| 401 | `unauthorized` | Missing or invalid credentials |
| 403 | `forbidden` | Authenticated but insufficient scope |
| 404 | `not_found` | Resource does not exist |
| 409 | `conflict` | State conflict (e.g. plan already running) |
| 422 | `unprocessable` | Validation failed |
| 500 | `internal_error` | Server-side error |
| 503 | `unavailable` | Downstream dependency unavailable |

The `X-Auth-Method` response header is set on authenticated requests.

---

## Demo Mode

`roko init --demo` creates a workspace pre-seeded with synthetic signals, episodes, and efficiency events. All endpoints behave identically in demo mode — the data simply comes from the pre-seeded JSONL files rather than live execution. This lets you explore the dashboard and API without running any agents.

---

## Quick Reference — All Routes

| Method | Path | Description |
|---|---|---|
| GET | `/health` | Bare liveness probe (no auth) |
| GET | `/api/health` | Rich health with provider status |
| GET | `/api/status` | Session overview |
| GET | `/api/dashboard` | Dashboard scaffold |
| GET | `/api/operations/:id` | Background operation status |
| GET | `/api/relay/health` | Relay diagnostics |
| GET | `/api/parity` | Feature parity matrix |
| GET | `/api/retention` | Retention policy status |
| GET | `/api/statehub/snapshot` | StateHub snapshot |
| GET | `/api/statehub/events` | StateHub event replay |
| GET | `/api/events` | SSE dashboard stream |
| GET | `/api/sse` | SSE dashboard stream (alias) |
| GET | `/api/workflow/events` | SSE workflow stream |
| GET | `/api/plans` | List plans |
| POST | `/api/plans` | Create plan |
| GET | `/api/plans/:id` | Get plan |
| GET | `/api/plans/:id/tasks` | Plan tasks |
| POST | `/api/plans/:id/execute` | Execute plan |
| GET | `/api/plans/:id/status` | Plan status |
| POST | `/api/plans/:id/pause` | Pause plan |
| POST | `/api/plans/:id/resume` | Resume plan |
| GET | `/api/plans/:id/gates` | Plan gate results |
| GET | `/api/plans/:id/reviews` | Review requests |
| POST | `/api/plans/:id/tasks/:tid/review` | Submit review |
| GET | `/api/plans/:id/tasks/:tid/diff` | Task diff |
| POST | `/api/plans/:id/chat` | Chat with plan agent |
| POST | `/api/plans/:id/estimate` | Cost estimate |
| POST | `/api/plans/generate` | Generate plan from prompt |
| GET | `/api/agents` | Aggregated agent list |
| GET | `/api/managed-agents` | Locally managed agents |
| POST | `/api/agents/register` | Register agent |
| POST | `/api/agents/create` | Create agent |
| GET | `/api/agents/:id` | Agent details |
| GET | `/api/agents/:id/profile` | Agent profile |
| POST | `/api/agents/:id/start` | Start agent |
| POST | `/api/agents/:id/stop` | Stop agent |
| POST | `/api/agents/:id/restart` | Restart agent |
| GET | `/api/agents/:id/episodes` | Agent episodes |
| GET | `/api/agents/:id/logs` | Agent logs |
| POST | `/api/agents/:id/message` | Send message to agent |
| GET | `/api/agents/:id/token` | Token status |
| POST | `/api/agents/:id/token` | Issue token |
| GET | `/api/agents/topology` | Fleet topology graph |
| GET | `/api/agents/:id/stats` | Agent stats |
| GET | `/api/agents/:id/skills` | Agent skills |
| GET | `/api/agents/:id/heartbeat` | Agent heartbeat |
| GET | `/api/agents/:id/trace` | Agent trace |
| GET | `/api/episodes` | Episodes list |
| GET | `/api/signals` | Raw signals |
| GET | `/api/gates/summary` | Gate aggregate |
| GET | `/api/gates/history` | Gate history (flat or waterfall) |
| GET | `/api/gates/:name/history` | Single gate history |
| GET | `/api/metrics` | Raw metrics |
| GET | `/api/metrics/summary` | Aggregated summary |
| GET | `/api/metrics/success_rate` | Template success rate |
| GET | `/api/metrics/engagement` | Feedback engagement |
| GET | `/api/metrics/c_factor` | C-Factor full breakdown |
| GET | `/api/metrics/model_efficiency` | Cost-per-success by model |
| GET | `/api/metrics/gate_rate` | Gate pass rate with trend |
| GET | `/api/metrics/experiments` | Experiment metric gaps |
| GET | `/api/metrics/feedback_latency` | Median feedback latency |
| GET | `/api/metrics/velocity` | Self-improvement velocity |
| GET | `/api/metrics/coverage` | Event coverage |
| GET | `/api/metrics/prometheus` | Prometheus scrape endpoint |
| GET | `/api/learn/efficiency` | Efficiency events aggregate |
| GET | `/api/learn/costs` | Cost projection |
| GET | `/api/learn/provider-outcomes` | Provider health |
| GET | `/api/learn/retries` | Retry surface |
| GET | `/api/learn/runtime-feedback` | Runtime feedback overview |
| GET | `/api/learn/router` | Cascade router snapshot |
| GET | `/api/learn/cascade` | Cascade routing summary |
| GET | `/api/learn/experiments` | A/B experiments |
| GET | `/api/learn/adaptive-thresholds` | Adaptive gate thresholds |
| GET | `/api/learn/gate-thresholds` | Gate thresholds (raw) |
| GET | `/api/learn/cost-tiers` | Model cost tiers |
| GET | `/api/c-factor/trend` | C-Factor trend series |
| GET | `/api/executor/state` | Executor DAG snapshot |
| POST | `/api/bench/runs` | Start bench run |
| GET | `/api/bench/runs` | List bench runs |
| GET | `/api/bench/runs/:id` | Bench run details |
| GET | `/api/bench/runs/:id/events` | Bench SSE stream |
| POST | `/api/bench/runs/:id/cancel` | Cancel bench run |
| GET | `/api/bench/runs/compare` | Compare runs |
| GET | `/api/bench/suites` | List suites |
| GET | `/api/bench/suites/:id` | Suite details |
| POST | `/api/bench/suites` | Upload suite |
| GET | `/api/bench/models` | Available models |
| GET | `/api/bench/pareto` | Pareto frontier |
| GET | `/api/bench/export/:id` | Export run as JSON |
| GET | `/api/bench/events` | Bench SSE stream (global) |
| POST | `/api/dream/run` | Trigger dream cycle |
| GET | `/api/dream/journal` | Dream journal |
| GET | `/api/knowledge/entries` | Knowledge entries |
| GET | `/api/knowledge/edges` | Knowledge graph edges |
| GET | `/api/knowledge/search` | Knowledge search |
| GET | `/api/knowledge/kinds` | Knowledge kinds |
| GET | `/api/tasks` | Tasks (aggregated) |
| GET | `/api/tasks/stats` | Task statistics |
| GET | `/api/tasks/:id` | Task details |
| GET | `/api/predictions/sessions` | Prediction sessions |
| GET | `/api/predictions/sessions/:id` | Prediction session |
| GET | `/api/predictions/claims` | Prediction claims |
| GET | `/api/predictions/calibration/:agent_id` | Agent calibration |
| GET | `/api/openapi.json` | OpenAPI schema |
| GET | `/ws` | WebSocket upgrade |
| GET | `/api/terminal/sessions` | Terminal sessions |
| POST | `/api/terminal/sessions` | New terminal session |
| DELETE | `/api/terminal/sessions/:id` | Destroy terminal session |
