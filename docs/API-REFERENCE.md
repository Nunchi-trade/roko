# Roko HTTP API Reference

The roko HTTP control plane is started with `roko serve` (default: `127.0.0.1:9090`). All REST endpoints are prefixed with `/api/` except webhooks (`/webhooks/`) and WebSocket (`/ws`).

## Authentication

When `[serve.auth]` is enabled in `roko.toml`, all `/api/` endpoints require:

```
Authorization: Bearer <api_key>
```

The WebSocket endpoint at `/ws` also requires auth when enabled.

## OpenAPI

```
GET /api/openapi.json
```

Returns the auto-generated OpenAPI 3.0 specification.

---

## Health and Status

### `GET /api/health`

Liveness check. Always returns 200 when the server is running.

**Response:**
```json
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_secs": 3600,
  "active_plans": 2,
  "active_agents": 1
}
```

### `GET /api/status`

Session status overview.

**Response:**
```json
{
  "session_id": "abc-123",
  "workdir": "/path/to/project",
  "daemon_running": true,
  "signal_count": 42,
  "episode_count": 15,
  "last_episode_passed": true
}
```

### `GET /api/dashboard`

Full dashboard data snapshot for rendering.

---

## Metrics

### `GET /api/metrics`

Raw metric snapshots as a JSON array.

### `GET /api/metrics/summary`

Aggregate execution and learning metrics.

**Query parameters:**
| Parameter | Type | Description |
|---|---|---|
| `period` | string | Time period (e.g. "24h", "7d") |

### `GET /api/metrics/success_rate`

Task and gate success rates.

### `GET /api/metrics/engagement`

Agent engagement metrics (turns per task, etc).

### `GET /api/metrics/c_factor`

C-Factor (capability factor) metrics.

### `GET /api/metrics/model_efficiency`

Per-model efficiency breakdown (cost/token ratios).

### `GET /api/metrics/gate_rate`

Gate pass/fail rates by gate type.

### `GET /api/metrics/experiments`

Active experiment performance metrics.

### `GET /api/metrics/feedback_latency`

Latency between agent output and gate feedback.

### `GET /api/metrics/velocity`

Task completion velocity over time.

### `GET /api/metrics/coverage`

PRD-to-plan-to-task coverage ratios.

### `GET /api/metrics/prometheus`

Prometheus-compatible text exposition format.

```bash
curl http://localhost:9090/api/metrics/prometheus
```

---

## Gates

### `GET /api/gates/summary`

Aggregate gate results across all gates.

### `GET /api/gates/history`

Recent gate invocations across all gate types.

### `GET /api/gates/{gate_name}/history`

History for a specific gate (e.g. `compile`, `test`, `clippy`, `diff`).

---

## Episodes

### `GET /api/episodes`

List episodes from `.roko/episodes.jsonl`.

**Query parameters:**
| Parameter | Type | Description |
|---|---|---|
| `limit` | integer | Max episodes to return |
| `offset` | integer | Skip N episodes |

---

## Signals

### `GET /api/signals`

List signals from `.roko/signals.jsonl`.

---

## Operations

### `GET /api/operations/{id}`

Check the status of a background operation.

**Response:**
```json
{
  "id": "uuid",
  "kind": "plan_generate:slug",
  "status": "Running",
  "finished": false
}
```

---

## Plans

### `GET /api/plans`

List all plans from `.roko/plans/`.

**Response:**
```json
[
  {
    "id": "my-plan",
    "title": "Implement feature X",
    "task_count": 5,
    "completed": false
  }
]
```

### `POST /api/plans`

Create a new plan.

**Request body:**
```json
{
  "title": "My Plan",
  "description": "What this plan does",
  "tasks": [
    {
      "id": "T1",
      "description": "First task",
      "depends_on": [],
      "files": ["src/main.rs"]
    },
    {
      "id": "T2",
      "description": "Second task",
      "depends_on": ["T1"],
      "files": ["src/lib.rs"]
    }
  ]
}
```

**Response:** `201 Created`
```json
{ "id": "generated-uuid" }
```

### `GET /api/plans/{id}`

Get a specific plan by ID.

**Response:**
```json
{
  "id": "my-plan",
  "title": "My Plan",
  "description": "...",
  "tasks": [
    {
      "id": "T1",
      "description": "...",
      "depends_on": [],
      "files": [],
      "completed": false
    }
  ]
}
```

### `POST /api/plans/{id}/execute`

Start background execution of a plan. Returns 409 if the plan is already executing.

**Response:** `202 Accepted`
```json
{ "id": "run-uuid" }
```

```bash
curl -X POST http://localhost:9090/api/plans/my-plan/execute
```

### `GET /api/plans/{id}/status`

Check execution status for an active plan run.

**Response:**
```json
{
  "id": "run-uuid",
  "plan_dir": "/path/.roko/plans",
  "status": "Running",
  "finished": false
}
```

### `POST /api/plans/generate`

Generate a plan from a published PRD. Spawns a background agent.

**Request body:**
```json
{ "slug": "my-prd-slug" }
```

**Response:** `202 Accepted`
```json
{ "id": "operation-uuid" }
```

---

## PRDs

### `GET /api/prds`

List all PRDs (ideas, drafts, published).

### `GET /api/prds/status`

PRD coverage report (plans per PRD, task completion ratios).

### `GET /api/prds/{slug}`

Get a specific PRD by slug.

### `POST /api/prds/ideas`

Capture a new idea.

**Request body:**
```json
{ "text": "Wire SystemPromptBuilder into orchestrate.rs" }
```

### `POST /api/prds/{slug}/draft`

Create or update a draft PRD from an idea.

### `POST /api/prds/{slug}/promote`

Promote a draft PRD to published status.

### `POST /api/prds/{slug}/plan`

Generate an implementation plan from a PRD (alias for `/api/plans/generate`).

---

## Run (Single-Prompt Execution)

### `POST /api/run`

Execute a single prompt through the universal loop (compose -> agent -> gate -> persist).

**Request body:**
```json
{
  "prompt": "Fix the bug in src/main.rs",
  "workdir": "/optional/override"
}
```

**Response:** `202 Accepted`
```json
{ "id": "run-uuid" }
```

```bash
curl -X POST http://localhost:9090/api/run \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Add error handling to the parse function"}'
```

### `GET /api/run/{id}/status`

Check status of a background run.

**Response:**
```json
{
  "id": "run-uuid",
  "prompt": "Fix the bug...",
  "status": "Running",
  "finished": false
}
```

---

## Agents

### `GET /api/managed-agents`

List all managed agent processes (tracked by the ProcessSupervisor).

**Response:**
```json
[
  { "id": "process-id", "label": "implementer-T1" }
]
```

### `POST /api/agents/register`

Register or update an agent in the discovery registry.

**Request body:**
```json
{
  "agent_id": "my-agent",
  "label": "Code Reviewer",
  "rest_endpoint": "http://localhost:8081",
  "websocket_endpoint": "ws://localhost:8081/ws",
  "capabilities": ["code_review", "testing"],
  "domain_tags": ["rust", "backend"],
  "issue_token": true
}
```

### `GET /api/agents/{id}`

Get details for a registered agent.

### `POST /api/agents/{id}/stop`

Stop a managed agent process.

### `GET /api/agents/{id}/episodes`

Get episodes associated with an agent.

### `GET /api/agents/{id}/logs`

Proxy logs from an agent's sidecar server.

### `POST /api/agents/{id}/message`

Send a message to an agent (proxied to its sidecar `/message` endpoint).

**Request body:**
```json
{ "content": "Please review this diff..." }
```

### `GET /api/agents/{id}/token`

Check token status for an agent.

### `POST /api/agents/{id}/token`

Issue a new auth token for an agent.

---

## Agent Aggregation (Fleet View)

These routes aggregate data across all discovered agent servers.

### `GET /api/agents`

List all discovered agents (both managed and externally registered).

**Query parameters:**
| Parameter | Type | Description |
|---|---|---|
| `owner` | string | Filter by owner |

### `GET /api/agents/topology`

Agent dependency topology graph.

### `GET /api/agents/{id}/stats`

Per-agent statistics (turns, tokens, cost).

### `GET /api/agents/{id}/skills`

List skills/capabilities for an agent.

### `GET /api/agents/{id}/heartbeat`

Agent liveness heartbeat data.

### `GET /api/agents/{id}/trace`

Execution trace for an agent.

**Query parameters:**
| Parameter | Type | Description |
|---|---|---|
| `limit` | integer | Max trace entries (default: 50) |
| `offset` | integer | Skip N entries |

---

## Predictions

### `GET /api/predictions/sessions`

List prediction market sessions.

### `GET /api/predictions/sessions/{id}`

Get a specific prediction session.

### `GET /api/predictions/claims`

List prediction claims across all agents.

### `GET /api/predictions/calibration/{agent_id}`

Calibration data for an agent's predictions.

---

## Knowledge

### `GET /api/knowledge/entries`

List knowledge store entries.

### `GET /api/knowledge/edges`

List knowledge graph edges (relations between entries).

### `GET /api/knowledge/search`

Search the knowledge store.

**Query parameters:**
| Parameter | Type | Description |
|---|---|---|
| `q` | string | Search query |
| `limit` | integer | Max results |

### `GET /api/knowledge/kinds`

List distinct knowledge entry kinds.

---

## Tasks

### `GET /api/tasks`

List tasks across all active plans.

### `GET /api/tasks/stats`

Task execution statistics.

### `GET /api/tasks/{id}`

Get a specific task by ID.

---

## Research

### `GET /api/research`

List research artifacts from `.roko/research/`.

**Response:**
```json
[
  { "name": "topic-report.md", "size": 4096, "is_file": true }
]
```

### `POST /api/research/topic`

Research a topic with citations.

**Request body:**
```json
{
  "topic": "Rust async error handling patterns",
  "intent": "explore"
}
```

Valid intents: `position`, `evaluate`, `monitor`, `explore`, `audit`.

### `POST /api/research/enhance-prd/{slug}`

Enhance a PRD with research context.

### `POST /api/research/enhance-plan/{plan}`

Optimize a plan with research insights.

### `POST /api/research/enhance-tasks/{plan}`

Split and optimize plan tasks based on research.

### `POST /api/research/analyze`

Analyze execution data for patterns and improvements.

---

## Learning

### `GET /api/learning/efficiency`
### `GET /api/learn/efficiency`

Aggregate efficiency data from `.roko/learn/efficiency.jsonl`.

### `GET /api/learning/cascade-router`
### `GET /api/learn/cascade-router`

Cascade router state and routing decisions.

### `GET /api/learning/cascade`
### `GET /api/learn/cascade`

Cascade stage progression data.

### `GET /api/learning/cost-tiers`
### `GET /api/learn/cost-tiers`

Cost breakdown by model tier.

### `GET /api/learn/experiments`
### `GET /api/learning/experiments`

Active prompt experiments and their results.

### `GET /api/learn/adaptive-thresholds`
### `GET /api/learning/adaptive-thresholds`

Current adaptive threshold values.

### `GET /api/learning/gate-thresholds`
### `GET /api/learn/gate-thresholds`

Gate-specific threshold data.

### `GET /api/c-factor/trend`

C-Factor trend over time.

**Query parameters:**
| Parameter | Type | Description |
|---|---|---|
| `window` | string | Time window for bucketing |

---

## Configuration

### `GET /api/config`

Return the current resolved `RokoConfig` as JSON (secrets masked).

```bash
curl http://localhost:9090/api/config
```

### `PUT /api/config`

Merge partial config JSON into the current config and write to `roko.toml`.

**Request body:** Any subset of the config structure:
```json
{
  "budget": { "max_plan_usd": 50.0 },
  "gates": { "skip_tests": true }
}
```

### `POST /api/config/reload`

Reload config from disk (hot-reload for supported sections).

**Response:**
```json
{
  "success": true,
  "warnings": [],
  "timestamp": "2026-04-20T12:00:00Z"
}
```

---

## Templates

### `GET /api/templates`

List all agent templates.

**Response:**
```json
[
  {
    "name": "code-reviewer",
    "description": "Reviews code changes",
    "model": "claude-sonnet-4-6",
    "role": "reviewer",
    "output_format": "markdown"
  }
]
```

### `POST /api/templates`

Create a new agent template.

**Request body:**
```json
{
  "name": "implementer",
  "description": "Implements code changes",
  "model": "claude-opus-4-6",
  "role": "implementer",
  "system_prompt": "You are a skilled engineer...",
  "output_format": "json",
  "tools": ["read", "edit", "bash"]
}
```

### `GET /api/templates/{name}`

Get a specific template by name.

### `DELETE /api/templates/{name}`

Delete a template.

### `POST /api/templates/{name}/deploy`

Deploy (spawn) an agent from a template.

```bash
curl -X POST http://localhost:9090/api/templates/code-reviewer/deploy
```

---

## Subscriptions

Event subscriptions trigger template deployments in response to events.

### `GET /api/subscriptions`

List all subscriptions.

**Response:**
```json
[
  {
    "id": "sub-1",
    "template": "code-reviewer",
    "trigger": "webhook:github:pull_request",
    "enabled": true,
    "concurrency_limit": 3,
    "cooldown_secs": 60
  }
]
```

### `POST /api/subscriptions`

Create a new subscription.

### `PUT /api/subscriptions/{id}`

Update an existing subscription.

### `DELETE /api/subscriptions/{id}`

Delete a subscription.

### `POST /api/subscriptions/{id}/enable`

Enable a disabled subscription.

### `POST /api/subscriptions/{id}/disable`

Disable a subscription without deleting it.

---

## Deployments

Cloud deployment management (Railway, manual, etc.).

### `GET /api/deployments`

List all deployments.

### `POST /api/deployments`

Create a new deployment from a template.

**Request body:**
```json
{
  "template": "worker-template",
  "params": { "branch": "main" },
  "backend": "railway-api"
}
```

### `GET /api/deployments/{id}`

Get deployment details.

### `DELETE /api/deployments/{id}`

Tear down a deployment.

### `GET /api/deployments/{id}/logs`

Get deployment logs.

### `POST /api/deployments/{id}/task`

Proxy a task to a deployed worker.

### `POST /api/deployments/{id}/callback`

Receive a callback from a deployed worker.

---

## Providers

### `GET /api/providers`

List configured LLM providers with health status and model counts.

```bash
curl http://localhost:9090/api/providers
```

### `GET /api/providers/{id}/health`

Health check for a specific provider (circuit breaker state, latency).

### `POST /api/providers/{id}/test`

Send a test prompt to a provider to verify connectivity.

```bash
curl -X POST http://localhost:9090/api/providers/anthropic/test
```

---

## Models

### `GET /api/models`

List all configured models with capabilities and cost data.

---

## Routing

### `GET /api/routing/explain`

Explain the current routing decision for a given context.

---

## Integrations

### `GET /api/integrations`

List all registered service integrations.

### `GET /api/integrations/{name}`

Get details for a specific integration.

---

## Diagnosis

### `GET /api/diagnosis/recent`

Recent conductor diagnoses (anomalies, circuit breaker events).

---

## Projections

StateHub-backed projection routes for remote read and watch flows.

### `GET /api/projections/{name}`

Get the current state of a projection.

**Query parameters:**
| Parameter | Type | Description |
|---|---|---|
| `filter` | string | Optional filter expression |
| `limit` | integer | Max entries |

Projection names include: `gate_pipeline`, `agent_fleet`, `plan_progress`, etc.

### `GET /api/projections/{name}/stream`

SSE stream of projection updates. Reconnectable with `Last-Event-ID`.

---

## Real-Time Events

### `GET /api/events` (alias: `GET /api/sse`)

Server-Sent Events stream for dashboard updates. Supports reconnection via `Last-Event-ID` header.

```bash
curl -N http://localhost:9090/api/events
```

Events are JSON payloads with monotonic `id:` fields:
```
id: 42
data: {"type":"plan_started","plan_id":"abc"}
```

---

## WebSocket

### `GET /ws`

Upgrade to WebSocket connection for bidirectional real-time events.

**Connection:**
```javascript
const ws = new WebSocket("ws://localhost:9090/ws");
ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log(data.type, data);
};
```

**Client control message (optional filtering):**
```json
{
  "subscribe": ["projection:gate_pipeline", "topic:agent.*"],
  "cursor": 42,
  "back_pressure": "at_most_once"
}
```

Back-pressure modes: `at_most_once` (default), `coalesce`, `resume_required`.

On connection, the server replays recent events from the ring buffer, then streams live events.

---

## Webhooks

Webhook ingress endpoints live outside `/api/` (no auth middleware by default).

### `POST /webhooks/github`

GitHub webhook receiver. Verifies `X-Hub-Signature-256`, converts the payload to an engram, persists it, and publishes to the event bus.

Requires `[webhooks.github].secret` to be configured.

**Headers required:**
- `X-Hub-Signature-256`: HMAC signature from GitHub

### `POST /webhooks/slack`

Slack webhook receiver.

### `POST /webhooks/generic`

Generic webhook ingress for arbitrary JSON payloads.

---

## Error Responses

All errors follow a consistent format:

```json
{
  "code": "not_found",
  "message": "plan 'xyz' not found"
}
```

Common status codes:
| Code | Meaning |
|---|---|
| 400 | Bad request / validation error |
| 401 | Unauthorized (missing or invalid API key) |
| 404 | Resource not found |
| 409 | Conflict (e.g. plan already executing) |
| 500 | Internal server error |

Validation errors include field-level details:
```json
{
  "code": "validation_error",
  "message": "request body validation failed",
  "fields": { "title": "must not be blank" }
}
```

---

## CORS

Cross-origin requests are supported. Configure allowed origins in `roko.toml`:

```toml
[serve]
cors_origins = ["http://localhost:3000", "https://your-dashboard.com"]
```

---

## Secret Scrubbing

All API responses pass through a secret-scrubbing middleware that redacts API keys and tokens from JSON output. Sensitive fields are replaced with `"***"`.
