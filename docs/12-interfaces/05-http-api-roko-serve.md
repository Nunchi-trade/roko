# HTTP API — `roko-serve`

> The `roko-serve` crate provides the HTTP server exposing REST endpoints plus the shared realtime surface over WebSocket, SSE, and optional gRPC streaming for remote control of Roko.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md), [00-architecture](../00-architecture/INDEX.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §2, `roko-serve/src/lib.rs`, `roko-serve/src/routes/mod.rs`, `bardo-backup/prd/25-mori/mori-interfaces.md`, `implementation-plans/11-sections/phase-0-1.md`

---

## Abstract

`roko-serve` is the HTTP server crate that exposes the Roko cognitive agent framework as a REST API with a shared realtime surface. It is started via `roko serve` or `roko daemon --start` and provides endpoints for agent management, plan orchestration, PRD lifecycle, knowledge queries, provider monitoring, and StateHub projection delivery over WebSocket, SSE, and optional gRPC streaming.

The server is built on `axum` (Tokio-based async HTTP framework), uses `tower-http` for middleware (CORS, tracing), and should expose the kernel's two-fabric reality directly: writes land in `Substrate`, live changes travel on the `Bus`, and remote consumers read typed `StateHub` projections instead of bespoke per-surface fanout. The architecture follows the `ServerBuilder` pattern, allowing configuration of auth, CORS origins, and projection transports before binding.

`roko-serve` sits at the Application layer and consumes the same runtime abstractions that `roko-cli` uses. The `CliRuntime` trait bridges the server to the CLI's `run_once`, status, and dashboard functions. This means the HTTP API drives the exact same cognitive loop as the CLI — same Engram pipeline, same gates, same learning.

REF23 makes this more explicit: the HTTP API is the transport layer behind the Web surface and the bridge that keeps CLI, TUI, Chat, and Web on the same verb set and the same live progress stream. The API should expose the same `ask`, `plan`, `do`, `watch`, `inspect`, `replay`, `learn`, `tune`, and `connect` actions instead of inventing a separate mental model. See [21-user-ux-running-agents.md](./21-user-ux-running-agents.md) and [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md).

REF24 adds the deployment-side constraint: `roko serve` is not a separate product tier, but the
same Rust binary expressed through different deployment runtime shapes. In practice that means
the HTTP surface must remain runtime-shape-aware for laptop, single-server, container,
clustered, and edge deployments; it must surface readiness, tenancy, budgets, and portable
state flows in a way that does not fork the core runtime. See [../19-deployment/INDEX.md](../19-deployment/INDEX.md),
[../19-deployment/10-secret-management.md](../19-deployment/10-secret-management.md),
[../19-deployment/12-production-hardening.md](../19-deployment/12-production-hardening.md),
[../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md),
and [tmp/refinements/24-deployment-ux.md](../../tmp/refinements/24-deployment-ux.md).

REF26 adds the missing middle layer: HTTP clients should usually talk to `StateHub` projections, not raw internal Pulses. `roko-serve` therefore becomes the transport binding for `query + subscribe` over named projections such as `active_tasks`, `gate_pipeline`, `agent_trails`, `cohort_health`, and `cost_meter`. See [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md).

REF27 adds the missing wire contract: those projections and filtered Topic views should ride one realtime protocol with shared `query`, `subscribe`, and `publish` semantics, shared cursors, shared auth rules, and transport-specific bindings for WebSocket, SSE, and optional gRPC. See [06-websocket-streaming.md](./06-websocket-streaming.md) and [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md).

---

## Surface Parity Contract

`roko-serve` exists so the Web surface can be first-party without becoming conceptually separate. The parity rule is:

| User verb | HTTP / stream shape | Notes |
|---|---|---|
| `ask` | `POST /api/run` plus stream subscription | Single-turn query with optional live output. |
| `plan` | `POST /api/plans` or plan-generation route | Proposal without mandatory execution. |
| `do` | `POST /api/plans/:id/run` or equivalent task execution route | Starts real work. |
| `watch` | `GET /projections/:name` plus `GET /projections/:name/stream` | Progress is queried, then streamed from typed projections with cursors. |
| `inspect` | Episode, Engram, heuristic, and agent detail reads | Durable artifact drill-down. |
| `replay` | Episode replay endpoint family | Re-run a prior episode from stored inputs and context. |
| `learn` | Learning and heuristic endpoints | Curate heuristics, playbooks, experiments, and calibration state. |
| `tune` | Config and threshold routes | Operator-level settings. |
| `connect` | Plugin, MCP, provider, and credential-management routes | Integration surface. |

This keeps the Web UI small and legible: it renders the shared verb set over the same data instead of growing a bespoke page-by-page control plane.

---

## Deployment Surface Contract

`roko-serve` is the control plane used by several REF24 deployment shapes:

| Shape | API expectation | Notes |
|---|---|---|
| `laptop` | Optional and local-only by default | `roko serve` is explicit, not ambient. |
| `single-server` | Stable LAN or VPN endpoint with auth | The first shared-team deployment shape. |
| `container` | Same API, environment-configured | Health probes and structured logs become mandatory. |
| `clustered` | Horizontally replicated API nodes | Sticky sessions where needed; otherwise stateless request handling. |
| `edge` | Minimal feature subset | Usually read-only or short-lived; may omit durable state writes. |

The consequence is architectural rather than cosmetic:

- Health and readiness endpoints are part of the contract, not optional extras.
- Tenant identity and role context must flow through request handling and tracing.
- Streaming cursors need to survive reconnects across container and clustered restarts.
- State export/import has to work whether the request originated from CLI, Web, or an automation client.

This chapter defines the HTTP-facing implications; the deployment chapter owns the operational
shapes themselves. See [../19-deployment/INDEX.md](../19-deployment/INDEX.md) and
[tmp/refinements/24-deployment-ux.md](../../tmp/refinements/24-deployment-ux.md).

---

## Projection Surface Contract

REF26 makes projections the canonical live-read surface for remote clients:

```text
GET /projections/cohort_health
GET /projections/cohort_health?filter=tenant:acme
GET /projections/active_tasks?filter=user:me
GET /projections/gate_pipeline/stream
GET /projections/agent_trails/stream?filter=lineage:<engram-hash>
```

The rule is:

- `GET /projections/:name` returns the current `State` for a named projection.
- `GET /projections/:name/stream` upgrades to WebSocket or SSE and emits typed `Delta` envelopes.
- Filters execute server-side by tenant, role, user, lineage, topic, or time range.
- Each envelope carries a cursor so reconnecting clients resume per projection rather than per raw socket.
- Projection updates are trace-linked back to the causing `Engram` or `Pulse`.

This keeps `roko-serve` small: command verbs mutate the runtime; projections report the runtime.

## Realtime Transport Contract

The projection registry is the semantic contract; the realtime surface is the delivery contract.

`roko-serve` should therefore expose:

- HTTP reads for `query`
- WebSocket for bidirectional `subscribe` plus `publish`
- SSE for server-to-client `subscribe`
- optional gRPC streaming for typed remote consumers that want the same cursor and auth behavior

The important constraint is that `/ws/*` is not the product contract by itself. It is one transport binding over the same named channels and projections documented in [06-websocket-streaming.md](./06-websocket-streaming.md).

Shared rules across transports:

- every subscription names a `channel`
- filters run server-side
- every outbound frame carries a cursor
- reconnecting clients resume from that cursor when retained history still exists
- auth is checked per subscription, not only once per socket
- `publish` is rate-limited, topic-allow-listed, and audited

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
│  ┌───────────┬────────┬───────────┬───────┐ │
│  │ Substrate │  Bus   │ StateHub  │ Config│ │
│  │ Engrams   │ Pulses │ projections│      │ │
│  └───────────┴────────┴───────────┴───────┘ │
├─────────��─────────────────────────────────��──┤
│  CliRuntime (bridge to roko-cli)             │
│  TemplateAgentDispatcher                     │
│  Projection transport bindings               │
│  Source dispatch loop                        │
│  Feedback loop                               │
└──────────────────────────────���───────────────┘
```

### Route Groups

The router is assembled in `roko-serve/src/routes/mod.rs` from route modules including:

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
| `subscriptions` | `/api/subscriptions` | Compatibility route family; should resolve to named projection subscriptions |
| `projections` | `/projections/*` | Query current projection state and subscribe to typed deltas |
| `deployments` | `/api/deployments` | Cloud deployments |
| `sse` | `/api/sse` | Server-Sent Events binding for projection streams |
| `ws` | `/ws/*` | WebSocket binding for projection streams and bidirectional controls |
| `grpc` | service definition | Optional typed binding for the same realtime surface |
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
POST   /api/agents/:id/input         # Send operator input to running agent
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

The server supports API-key, token, and browser-session authentication:

```toml
[serve.auth]
enabled = true
api_key = "roko_sk_..."
```

Supported auth forms:

- API key in `Authorization: Bearer ...` or `x-api-key`
- OIDC bearer token for user-scoped control-plane clients
- session cookie for first-party browser flows
- trusted-header auth only when explicitly enabled behind a controlled reverse proxy

Webhook routes (`/webhooks/*`) are exempt from this path because they use their own signature verification.

The auth middleware is implemented in `roko-serve/src/routes/middleware.rs` using `axum::middleware::from_fn_with_state`. A secret-scrubbing middleware layer also runs on all API responses, redacting API keys and tokens from JSON output.

REF24 extends this into multi-tenant deployment auth for single-server and clustered shapes. The
important API-facing additions are:

- OIDC and personal access token flows should both produce a `TenantCtx` attached to the request.
- Tenant identity should scope durable `Substrate` access, budget enforcement, and audit fields.
- Trusted-header auth is an explicit opt-in for air-gapped or reverse-proxy deployments, not the default path.
- Realtime subscriptions must authorize each requested `channel` and filter before streaming begins.

The deployment chapter covers the mapping rules and `TenantCtx` examples in more detail. See
[../19-deployment/INDEX.md](../19-deployment/INDEX.md),
[../19-deployment/10-secret-management.md](../19-deployment/10-secret-management.md),
and [tmp/refinements/24-deployment-ux.md](../../tmp/refinements/24-deployment-ux.md).

---

## Deployment Endpoints and Probes

Even when the API surface stays small, deployment-facing endpoints need to be first-class:

```text
GET /healthz
GET /readyz
GET /metrics
POST /api/state/export
POST /api/state/import
GET /api/cost/report
```

- `/healthz` is liveness: the process is up enough for the supervisor to keep it running.
- `/readyz` is readiness: the selected runtime shape has its `Substrate`, `Bus`, and auth dependencies ready.
- `/metrics` exposes Prometheus-compatible metrics using stable `roko.*` names across all shapes.
- State export/import endpoints mirror the CLI portability flow so laptop-to-server promotion does not require out-of-band tooling.
- Cost reporting belongs here because deployment trust depends on visible spend, especially in shared or clustered environments.

These routes complement, rather than replace, the CLI workflow. They exist so Web, automation,
and cluster operators all exercise the same runtime behavior.

---

## StateHub Projection Layer

`roko-serve` should treat StateHub as the authoritative live-read boundary:

- The `Bus` remains the raw transport fabric for Pulses inside the runtime.
- `StateHub` subscribes to those Topics, folds them with durable `Substrate` reads, and computes typed projection state.
- HTTP, WebSocket, and SSE then expose the projection registry rather than documenting raw transport internals as the public contract.

For example:

- `cohort_health` serves c-factor, roster, and delivery metrics to dashboards.
- `active_tasks` serves progress, ETA, and operator-facing task state to CLI, TUI, Chat, and Web `watch`.
- `gate_pipeline` serves rung counts and failures to CI-style views.
- `agent_trails` serves per-agent live timelines to chat and replay tooling.

One-off endpoint families may still exist, but they should be treated as compatibility shims or convenience aliases over the same projection registry.

---

## Dispatch Loop

The `TemplateAgentDispatcher` (`roko-serve/src/dispatch.rs`) monitors the Substrate for incoming task Engrams and dispatches them to agent templates. The dispatch loop:

1. Polls the Substrate for unprocessed Engrams of kind `Task` or `WebhookReceived`
2. Matches the Engram to a subscription rule
3. Instantiates the appropriate agent template
4. Runs the cognitive loop
5. Persists results and publishes Pulses plus final Engrams

Built-in input sources (cron scheduler, file watcher) are started automatically from `roko.toml` configuration. As durable task Engrams are created and Pulses are emitted during execution, StateHub folds them into `active_tasks`, `recent_episodes`, and `agent_trails` so every consumer sees the same task state.

---

## Current Status and Gaps

**Built (scaffold):**
- Server framework (axum, CORS, auth middleware, secret scrubbing)
- Core route structure
- Bus-backed streaming internals
- Template agent dispatcher with dispatch loop
- Cron and file-watch input sources
- Deployment backends (Railway, manual)
- Feedback loop

**Not yet complete:**
- Full implementation of all route handlers (many return placeholder responses)
- StateHub projection registry and typed transport bindings
- Server-side projection filters, replay cursors, and projection auth scoping
- Mesh endpoints (requires Agent Mesh, Tier 5)
- Spectre state endpoint (requires Spectre implementation)
- WebSocket bidirectional control

---

## Cross-References

- See [06-websocket-streaming.md](./06-websocket-streaming.md) for the shared realtime transport contract
- See [22-statehub-projection-layer.md](./22-statehub-projection-layer.md) for the projection contract
- See [00-cli-overview.md](./00-cli-overview.md) for `roko serve` command
- See [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) for `Pulse`, `Bus`, `StateHub`, and `projection` terminology
- See topic [01-orchestration](../01-orchestration/INDEX.md) for plan execution
- See topic [19-deployment](../19-deployment/INDEX.md) for cloud deployment
- See [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) for the canonical REF26 proposal
- See [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md) for the canonical REF27 proposal
