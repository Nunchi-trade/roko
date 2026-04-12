# HTTP API — `roko-serve`

> The `roko-serve` crate provides the HTTP server exposing REST endpoints, WebSocket streaming, and SSE event feeds for remote control of Roko.

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md), [00-architecture](../00-architecture/INDEX.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §2, `roko-serve/src/lib.rs`, `roko-serve/src/routes/mod.rs`, `bardo-backup/prd/25-mori/mori-interfaces.md`, `implementation-plans/11-sections/phase-0-1.md`

---

## Abstract

`roko-serve` is the HTTP server crate that exposes the Roko cognitive agent framework as a REST API with real-time streaming. It is started via `roko serve` or `roko daemon --start` and provides endpoints for agent management, plan orchestration, PRD lifecycle, knowledge queries, provider monitoring, and real-time event streaming via WebSocket and SSE.

The server is built on `axum` (Tokio-based async HTTP framework), uses `tower-http` for middleware (CORS, tracing), and provides an event bus for broadcasting internal events to connected clients. The architecture follows the `ServerBuilder` pattern, allowing configuration of auth, CORS origins, and event sources before binding.

`roko-serve` sits at the Application layer and consumes the same runtime abstractions that `roko-cli` uses. The `CliRuntime` trait bridges the server to the CLI's `run_once`, status, and dashboard functions. This means the HTTP API drives the exact same cognitive loop as the CLI — same Engram pipeline, same gates, same learning.

---

## Architecture

```
┌───────────────���───────────────���─────────────┐
│                 roko-serve                    │
├──────────┬──────────┬───────────┬───────────┤
│ REST API │ WebSocket│   SSE     │ Webhooks  │
│ /api/*   │ /ws/*    │ /api/sse  │ /webhooks │
├──────────┴���─────────┴───────────┴───────────┤
│              axum Router                     │
│  ┌─────────────────────────────────────────┐ │
│  │  Middleware: CORS, Auth, SecretScrubber │ │
│  └────────────────��────────────────────────┘ │
├──────────────────────────────────────────────┤
│              AppState                        │
│  ┌────────────┬──────────┬────────────────┐ │
│  │ SignalStore │ EventBus │ RokoConfig     │ │
│  │ (Substrate)│ (pubsub) │ (hot-reloadable│ │
│  └────────────┴──────��───┴────────────────┘ │
├─────────��─────────────────────────────────��──┤
│  CliRuntime (bridge to roko-cli)             │
│  TemplateAgentDispatcher                     │
│  EventSource dispatch loop                   │
│  Feedback loop                               │
└──────────────────────────────���───────────────┘
```

### Route Groups

The router is assembled in `roko-serve/src/routes/mod.rs` from 12 route modules:

| Module | Prefix | Purpose |
|---|---|---|
| `status` | `/api/status` | System status, C-Factor |
| `plans` | `/api/plans` | Plan CRUD and execution |
| `prds` | `/api/prds` | PRD lifecycle management |
| `run` | `/api/run` | Execute prompts |
| `research` | `/api/research` | Research operations |
| `agents` | `/api/agents` | Agent CRUD and messaging |
| `learning` | `/api/learning` | Episodes, routing, experiments |
| `config` | `/api/config` | Configuration management |
| `templates` | `/api/templates` | Agent templates |
| `subscriptions` | `/api/subscriptions` | Event subscriptions |
| `deployments` | `/api/deployments` | Cloud deployments |
| `sse` | `/api/sse` | Server-Sent Events stream |
| `ws` | `/ws/*` | WebSocket endpoints |
| `webhooks` | `/webhooks/*` | Incoming webhook handlers |

---

## Core Endpoints

### Run

```
POST   /api/run                      # Run prompt through cognitive loop
```

Request body:
```json
{
  "prompt": "Add error handling to the auth module",
  "model": "claude-sonnet-4-6",
  "role": "implementer",
  "effort": "medium"
}
```

Response: The Engram produced by the cognitive loop, with gate verdicts and episode ID.

### Orchestrate

```
POST   /api/orchestrate              # Start plan execution
GET    /api/plans                    # List plans
GET    /api/plans/:id               # Plan details
POST   /api/plans/:id/run           # Execute a specific plan
```

### Status

```
GET    /api/status                   # System status + C-Factor
```

Returns: Engram count, episode stats, gate pass rate, cost summary, model usage distribution, C-Factor value, and Neuro tier distribution.

---

## Agent Management

```
GET    /api/agents                   # List all agents
GET    /api/agents/:id               # Agent details + Daimon state
POST   /api/agents                   # Create agent from template
DELETE /api/agents/:id               # Delete agent
POST   /api/agents/:id/message       # Send message to running agent
```

Agent detail response includes:
- Agent ID, name, domain, template
- Current status (running, idle, resting)
- Daimon PAD vector (pleasure, arousal, dominance)
- Behavioral state (Engaged, Struggling, Coasting, Exploring, Focused, Resting)
- Current task and progress
- Token usage and cost
- Neuro entry count by tier

---

## Knowledge Endpoints

```
GET    /api/neuro/entries            # Query knowledge store
POST   /api/neuro/backup             # Export knowledge
POST   /api/neuro/restore            # Import knowledge
GET    /api/neuro/stats              # Tier distribution, HDC stats
GET    /api/neuro/cfactor            # Collective intelligence metrics
```

The knowledge query endpoint supports:
- Full-text search
- HDC similarity search (pass a vector)
- Filter by tier (Transient, Working, Consolidated, Persistent)
- Filter by type (Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge)

---

## Provider Endpoints

```
GET    /api/providers                # Configured providers
GET    /api/providers/:id/health     # Health status + circuit breaker state
POST   /api/providers/:id/test       # Test connectivity
GET    /api/models                   # Available models across providers
GET    /api/routing/explain          # Explain routing decision
```

The `/api/routing/explain` endpoint is particularly useful for debugging — it takes a hypothetical task and returns which model the cascade router would select and why, including confidence scores, cost estimates, and circuit breaker states.

---

## Mesh Endpoints

```
GET    /api/mesh/peers               # Connected agents in the mesh
GET    /api/mesh/pheromones          # Active pheromones (typed Engrams)
POST   /api/mesh/publish             # Publish Engram to mesh
```

**Status**: Not yet implemented. Requires Agent Mesh infrastructure (Tier 5).

---

## Authentication

The server supports optional API key authentication:

```toml
[serve.auth]
enabled = true
api_key = "roko_sk_..."
```

When enabled, all `/api/*` routes require a `Authorization: Bearer <key>` header. Webhook routes (`/webhooks/*`) are exempt from API key auth as they use their own signature verification.

The auth middleware is implemented in `roko-serve/src/routes/middleware.rs` using `axum::middleware::from_fn_with_state`. A secret-scrubbing middleware layer also runs on all API responses, redacting API keys and tokens from JSON output.

---

## Event Bus

The server maintains an internal event bus (`roko-serve/src/event_bus.rs`) for broadcasting events to connected WebSocket and SSE clients. The bus uses a publish-subscribe pattern:

```rust
pub enum ServerEvent {
    AgentSpawned { agent_id: String },
    AgentOutput { agent_id: String, text: String },
    AgentExited { agent_id: String, code: i32 },
    GateResult { plan: String, gate: String, passed: bool },
    PlanPhaseChange { plan: String, phase: String },
    WebhookReceived { signal: Signal },
    CFactorUpdate { value: f64 },
    // ... more event types
}
```

Clients subscribe to event categories. On reconnection, the server replays recent events from a ring buffer.

---

## Dispatch Loop

The `TemplateAgentDispatcher` (`roko-serve/src/dispatch.rs`) monitors the Substrate for incoming task Engrams and dispatches them to agent templates. The dispatch loop:

1. Polls the Substrate for unprocessed Engrams of kind `Task` or `WebhookReceived`
2. Matches the Engram to a subscription rule
3. Instantiates the appropriate agent template
4. Runs the cognitive loop
5. Persists results and publishes events

Built-in event sources (cron scheduler, file watcher) are started automatically from `roko.toml` configuration.

---

## Current Status and Gaps

**Built (scaffold):**
- Server framework (axum, CORS, auth middleware, secret scrubbing)
- Route structure (all 12 modules exist)
- Event bus with pub/sub
- Template agent dispatcher with dispatch loop
- Cron and file-watch event sources
- Deployment backends (Railway, manual)
- Feedback loop

**Not yet complete:**
- Full implementation of all route handlers (many return placeholder responses)
- Mesh endpoints (requires Agent Mesh, Tier 5)
- Spectre state endpoint (requires Spectre implementation)
- WebSocket bidirectional control

---

## Cross-references

- See [06-websocket-streaming.md](./06-websocket-streaming.md) for real-time streaming
- See [00-cli-overview.md](./00-cli-overview.md) for `roko serve` command
- See topic [01-orchestration](../01-orchestration/INDEX.md) for plan execution
- See topic [19-deployment](../19-deployment/INDEX.md) for cloud deployment
