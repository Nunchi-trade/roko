# roko-serve

The Roko HTTP control plane. Single port (6677 by default), ~85 REST routes,
SSE + WebSocket streams, and webhook ingestion. Designed so a dashboard,
CLI, or external integration can talk to one base URL and reach every agent,
plan, PRD, metric, and learning signal.

## Start it

```bash
roko serve                                     # default 127.0.0.1:6677
roko serve --bind 0.0.0.0 --port 9090
```

Or embed it:

```rust
use roko_serve::{RokoServer, ServerConfig};

let server = RokoServer::new(ServerConfig {
    bind: "0.0.0.0:6677".parse()?,
    data_dir: Some(".roko".into()),
    ..Default::default()
}).await?;
server.serve().await?;
```

## Route surface

### Health + metrics

| Method | Path | What |
|--------|------|------|
| `GET` | `/api/health` | liveness probe |
| `GET` | `/api/status` | full status JSON |
| `GET` | `/api/metrics` | all metrics (Prometheus-style) |
| `GET` | `/api/metrics/summary` | compact dashboard summary |
| `GET` | `/api/metrics/c_factor` | composite capability score |
| `GET` | `/api/metrics/{success_rate|engagement|gate_rate|experiments|feedback_latency|velocity|coverage|model_efficiency}` | specific dimensions |
| `GET` | `/api/dashboard` | pre-rolled dashboard payload |
| `GET` | `/api/gates/summary`, `/api/gates/history`, `/api/gates/{name}/history` | gate stats |
| `GET` | `/api/episodes`, `/api/signals`, `/api/operations/{id}` | event-log views |

### Plans

| Method | Path | What |
|--------|------|------|
| `GET` | `/api/plans` | list |
| `POST` | `/api/plans` | create |
| `GET` | `/api/plans/{id}` | detail |
| `POST` | `/api/plans/{id}/execute` | start execution |
| `GET` | `/api/plans/{id}/status` | progress |
| `POST` | `/api/plans/generate` | agent-driven plan generation from a PRD |

### PRDs

| Method | Path | What |
|--------|------|------|
| `GET` | `/api/prds` | list |
| `POST` | `/api/prds/ideas` | capture an idea |
| `GET` | `/api/prds/status` | coverage report |
| `GET` | `/api/prds/{slug}` | detail |
| `POST` | `/api/prds/{slug}/draft` | create/update draft |
| `POST` | `/api/prds/{slug}/promote` | promote draft → published |
| `POST` | `/api/prds/{slug}/plan` | generate plan from PRD |

### Run

| Method | Path | What |
|--------|------|------|
| `POST` | `/api/run` | execute a single prompt through the universal loop |
| `GET` | `/api/run/{id}/status` | progress |

### Research

| Method | Path | What |
|--------|------|------|
| `GET` | `/api/research` | list past research runs |
| `POST` | `/api/research/topic` | deep-research a topic |
| `POST` | `/api/research/enhance-prd/{slug}` | enhance a PRD with research |
| `POST` | `/api/research/enhance-plan/{plan}` | enhance a plan |
| `POST` | `/api/research/enhance-tasks/{plan}` | split / optimize tasks |
| `POST` | `/api/research/analyze` | analyze execution data |

### Agents — registration + management

| Method | Path | What |
|--------|------|------|
| `GET` | `/api/managed-agents` | list roko-managed agents |
| `POST` | `/api/agents/register` | register an external agent |
| `POST` | `/api/agents/{id}/stop` | graceful shutdown |
| `POST` | `/api/agents/{id}/message` | proxied to sidecar `POST /message` |
| `GET` | `/api/agents/{id}` | detail |
| `GET` | `/api/agents/{id}/episodes` | per-agent episode log |
| `GET/POST` | `/api/agents/{id}/token` | agent auth token |

### Agents — aggregator (discovery + fan-out)

| Method | Path | What |
|--------|------|------|
| `GET` | `/api/agents` (supports `?owner=`) | discovery union across all sidecars |
| `GET` | `/api/agents/topology` | graph of agents + their relationships |
| `GET` | `/api/agents/{id}/stats`, `/skills`, `/heartbeat`, `/trace` | sidecar read-through |
| `GET` | `/api/predictions/sessions`, `/sessions/{id}`, `/claims`, `/calibration/{agent_id}` | prediction union |
| `GET` | `/api/knowledge/entries`, `/edges`, `/search`, `/kinds` | knowledge graph |
| `GET` | `/api/tasks`, `/tasks/stats`, `/tasks/{id}` | task union |
| `GET` | `/api/ws` | aggregated per-agent event stream |

### Learning (aliases: `/api/learn/*` and `/api/learning/*`)

| Method | Path | What |
|--------|------|------|
| `GET` | `/api/learn/efficiency` | cost/token/latency rollup |
| `GET` | `/api/learn/cascade-router`, `/cascade` | router state |
| `GET` | `/api/learn/cost-tiers` | tier distribution |
| `GET` | `/api/learn/experiments` | A/B variants + win rates |
| `GET` | `/api/learn/adaptive-thresholds`, `/gate-thresholds` | EMA gate thresholds |

### Subscriptions / templates / deployments / config / providers

| Area | Paths |
|------|-------|
| Subscriptions | `GET/POST /api/subscriptions`, `PUT/DELETE /api/subscriptions/{id}`, `POST /api/subscriptions/{id}/{enable|disable}` |
| Templates | `GET/POST /api/templates`, `GET/DELETE /api/templates/{name}`, `POST /api/templates/{name}/deploy` |
| Deployments | `GET/POST /api/deployments`, `GET/DELETE /api/deployments/{id}`, `GET /api/deployments/{id}/logs`, `POST /api/deployments/{id}/task`, `POST /api/deployments/callback` |
| Config | `GET/PUT /api/config`, `POST /api/config/reload` |
| Providers | `GET /api/providers/`, `GET /api/providers/{id}/health`, `POST /api/providers/{id}/test`, `GET /api/models/`, `GET /api/models/routing/explain` |

### Streaming + webhooks

| Method | Path | What |
|--------|------|------|
| `GET` | `/ws` | top-level WebSocket (system events) |
| `GET` | `/api/ws` | aggregator WebSocket (per-agent union) |
| `GET` | `/api/events` | SSE stream of platform events |
| `POST` | `/webhooks/github`, `/webhooks/slack`, `/webhooks/generic` | webhook ingestion |

## Smoke-test recipe

```bash
# start the server
roko serve &

# health
curl http://localhost:6677/api/health

# discover agents
curl http://localhost:6677/api/agents

# fetch dashboard payload
curl http://localhost:6677/api/dashboard | jq .

# subscribe to SSE
curl -N http://localhost:6677/api/events

# POST a prompt through the universal loop
curl -X POST http://localhost:6677/api/run \
  -H "Content-Type: application/json" \
  -d '{"prompt": "hello", "agent": "nunchi-intelligence"}'
```

## Architecture notes

```
┌───────────────┐
│ Dashboard /   │   single base URL
│ External CLI  │   (:6677 or any bind)
└──────┬────────┘
       │
┌──────▼────────────────────────────────────┐
│   roko-serve (Axum router)                │
│   - /api/* REST                           │
│   - /ws, /api/ws, /api/events streams     │
│   - /webhooks/* ingestion                 │
└──────┬────────────────────────────────────┘
       │
       ├──► roko-core (signals, config, episodes)
       ├──► roko-learn (efficiency, c-factor, experiments)
       ├──► roko-gate (gate history, thresholds)
       ├──► roko-orchestrator (plan DAG, tasks)
       │
       └──► aggregator ──► roko-agent-server (per agent)
                           /message, /stream, /predictions, ...
```

## Tests

```bash
cargo test -p roko-serve
cargo clippy -p roko-serve --no-deps -- -D warnings
```

Routes added in T18 (`deployments`, `templates`, `mcp-code`) have
dedicated integration tests that exercise happy and error paths.

## What it is not

- **Not an agent**, it only proxies to them. Messaging happens in
  `roko-agent-server`.
- **Not a scheduler**, it only triggers. Execution runs in `roko-cli`
  via `roko-orchestrator`.

## Related

- `crates/roko-agent-server/README.md` — per-agent sidecar contract
- Top-level `README.md` — full Deployment section with recipes for
  daemon, worker, and cloud targets
