# HTTP API and Realtime

> Depth for [20-SURFACES.md](../../unified/20-SURFACES.md). Covers the HTTP API as a Connect Cell with StateHub backing, the ~85 routes grouped by resource, the shared realtime protocol (WebSocket + SSE) as Bus subscriptions through Connect Cells, the canonical frame vocabulary, cursor semantics, and deployment-shape-aware configuration.

---

## 1. The HTTP API as a Connect Cell

`roko-serve` is an axum-based HTTP server that exposes the Roko runtime as a REST API with realtime streaming. In unified vocabulary, the HTTP API is a **Connect Cell** -- it implements the Connect protocol (connect/query/execute/disconnect) with lifecycle management and health checks. See [02-CELL.md](../../unified/02-CELL.md) for the Connect protocol definition.

The server sits at the application layer and consumes the same runtime abstractions as the CLI. The `CliRuntime` trait bridges the server to the CLI's `run_once`, status, and dashboard functions. This means the HTTP API drives the **exact same cognitive loop** as the CLI -- same Signal pipeline, same gates, same learning.

**Source**: `crates/roko-serve/src/routes/mod.rs` (route assembly), `crates/roko-serve/src/state.rs` (AppState).

---

## 2. Route Groups

The router assembles approximately 85 routes from modules:

| Module | Prefix | Purpose | Verb Mapping |
|---|---|---|---|
| `run` | `/api/run` | Execute prompts | ask, do |
| `plans` | `/api/plans` | Plan CRUD and execution | plan, do |
| `prds` | `/api/prds` | PRD lifecycle | plan |
| `agents` | `/api/agents` | Agent CRUD and messaging | inspect |
| `learning` | `/api/learning` | Episodes, routing, experiments | learn |
| `research` | `/api/research` | Research operations | ask (research mode) |
| `config` | `/api/config` | Configuration management | tune |
| `templates` | `/api/templates` | Agent templates | tune |
| `status` | `/api/status` | System status, c-factor | watch |
| `projections` | `/projections/*` | Query/subscribe to StateHub | watch, inspect |
| `subscriptions` | `/api/subscriptions` | Compatibility aliases | watch |
| `deployments` | `/api/deployments` | Cloud deployments | connect |
| `sse` | `/api/sse` | Server-Sent Events | watch (stream) |
| `ws` | `/ws/*` | WebSocket bidirectional | watch, publish |
| `webhooks` | `/webhooks/*` | Incoming webhook handlers | trigger |

### Surface Parity Contract

Every user verb has an HTTP equivalent:

| Verb | HTTP Shape | Notes |
|---|---|---|
| ask | `POST /api/run` + stream subscription | Single-turn with optional live output |
| plan | `POST /api/plans` or plan-generation route | Proposal without execution |
| do | `POST /api/plans/:id/run` | Starts real work |
| watch | `GET /projections/:name` + `/stream` | Query, then stream |
| inspect | Episode, Signal, heuristic detail reads | Durable artifact drill-down |
| replay | Episode replay endpoint | Re-run from stored inputs |
| learn | Learning and heuristic endpoints | Curate heuristics |
| tune | Config and threshold routes | Operator settings |
| connect | Plugin, MCP, provider routes | Integration surface |

---

## 3. The Projection Surface

StateHub projections are the canonical live-read surface for remote clients:

```
GET /projections/cohort_health
GET /projections/cohort_health?filter=tenant:acme
GET /projections/active_tasks?filter=user:me
GET /projections/gate_pipeline/stream
```

The rule:
- `GET /projections/:name` returns the current `State` for a named projection.
- `GET /projections/:name/stream` upgrades to WebSocket or SSE and emits typed `Delta` envelopes.
- Filters execute server-side (tenant, role, user, lineage, topic, time range).
- Each envelope carries a **cursor** for reconnection.
- Updates are trace-linked back to the causing Signal or Pulse.

This keeps `roko-serve` small: command verbs mutate the runtime; projections report the runtime.

---

## 4. The Realtime Protocol

WebSocket and SSE are the two shipping realtime transports. Both share a single logical protocol.

### Canonical Frame Vocabulary

Transport-agnostic JSON frames:

| Frame Type | Direction | Meaning |
|---|---|---|
| `subscribe` | client -> server | Open a live subscription on a channel |
| `unsubscribe` | client -> server | Close a subscription |
| `query` | client -> server | One-shot read of a projection |
| `publish` | client -> server | Emit a user-originated Pulse (WebSocket only) |
| `state` | server -> client | Full state snapshot (first reply for projections) |
| `delta` | server -> client | Incremental projection update |
| `pulse` | server -> client | Raw topic or stream item |
| `ack` | server -> client | Success confirmation |
| `error` | server -> client | Request-local failure (does not kill connection) |
| `ping` / `pong` | both | Heartbeat |

### Subscribe with Cursor

```json
{
  "type": "subscribe",
  "id": "sub-abc",
  "payload": {
    "channel": "projection:cohort_health",
    "filter": {"tenant": "acme"},
    "cursor": "0x04f1",
    "mode": "Coalesce"
  }
}
```

The server replies with `state` (full snapshot) followed by `delta` (incremental). If the cursor is too old, the server sends a fresh `state` plus the current cursor.

### Channel Taxonomy

| Prefix | Meaning | Example |
|---|---|---|
| `projection:` | Named StateHub projection | `projection:cohort_health` |
| `topic:` | Raw Bus topic stream | `topic:gate.failed.*` |
| `signal-stream:` | Live Store writes | `signal-stream:kind=heuristic` |
| `agent:` | Per-agent activity feed | `agent:agt_042` |
| `session:` | Per-session progress | `session:sess_xyz` |

Consumers may mix multiple channel kinds on one connection. The server handles fanout; clients should not open one connection per subscription.

### Back-Pressure Modes

| Mode | Behavior | Use Case |
|---|---|---|
| `AtMostOnce` | Oldest messages may drop | Live dashboards |
| `Coalesce` | Deltas collapse into newer delta or fresh state | Browser dashboards, mobile |
| `ResumeRequired` | Server errors when client falls behind | Audit, replication |

---

## 5. Cursor Semantics for Reconnection

Every outbound frame carries a cursor. The reconnection protocol:

1. Client reconnects
2. Client resends `subscribe` with last known cursor
3. Server replays from retained history if possible
4. If cursor is too old:
   - Projection channels: fresh `state` + current cursor
   - Raw topic channels: `error` + current cursor (client accepts gap)

SSE uses the standard `Last-Event-ID` header. WebSocket carries cursor in the subscribe payload.

---

## 6. Authentication and Authorization

The server supports multiple auth forms:

| Auth Form | Use Case |
|---|---|
| API key (`Authorization: Bearer` or `x-api-key`) | Automation, machine consumers |
| OIDC bearer token | User-scoped remote clients |
| Session cookie | First-party browser flows |
| Trusted-header | Air-gapped reverse-proxy deployments (explicit opt-in) |

Authorization is **per subscription**, not just per connection. The server checks whether the identity may see the requested channel and filter before streaming begins. A denied subscription returns `error` for that request ID without killing the connection.

**Source**: `crates/roko-serve/src/routes/middleware.rs` (auth middleware), secret-scrubbing middleware.

---

## 7. Deployment-Shape-Aware Configuration

The same HTTP API serves five deployment shapes. The shape determines defaults, not capabilities:

| Shape | API Expectation | Notes |
|---|---|---|
| `laptop` | Optional, local-only | `roko serve` is explicit |
| `single-server` | Stable LAN/VPN endpoint with auth | First shared-team shape |
| `container` | Same API, environment-configured | Health probes mandatory |
| `clustered` | Horizontally replicated | Sticky sessions where needed |
| `edge` | Minimal feature subset | May omit durable writes |

### Deployment Endpoints

```
GET /healthz          # Liveness (process is up)
GET /readyz           # Readiness (Store, Bus, auth ready)
GET /metrics          # Prometheus-compatible (roko.* names)
POST /api/state/export  # Portable state export
POST /api/state/import  # State import (laptop-to-server migration)
GET /api/cost/report    # Cost reporting for shared environments
```

---

## 8. The Dispatch Loop

The `TemplateAgentDispatcher` monitors the Store for incoming task Signals and dispatches them to agent templates:

1. Poll Store for unprocessed Signals of kind `Task` or `WebhookReceived`
2. Match Signal to a subscription rule
3. Instantiate the appropriate agent template
4. Run the cognitive loop
5. Persist results and publish Pulses
6. StateHub folds results into `active_tasks`, `recent_episodes`, and `agent_trails`

Built-in input sources (cron scheduler, file watcher) start automatically from `roko.toml` configuration.

---

## 9. Observability

The realtime surface has its own telemetry:

| Metric | Meaning |
|---|---|
| `roko.realtime.connections` | Open transport connections |
| `roko.realtime.subscriptions` | Active subscriptions by channel family |
| `roko.realtime.messages_per_second` | Inbound and outbound traffic |
| `roko.realtime.cursor_lag` | How far behind consumers are |
| `roko.realtime.auth_failures` | Failed auth or subscribe attempts |

### Wire Stability Contract

- Adding fields is non-breaking
- Removing or renaming fields is breaking
- Frame meaning must not change under an existing `type`
- Clients must ignore unknown fields
- Servers must accept previously emitted cursors for the retention window

---

## What This Enables

- **Same data, many transports**: WebSocket for interactive web, SSE for dashboards, HTTP for one-shot queries. All read the same projections.
- **Cursor-based resume**: Clients survive disconnections without losing state. The cursor is the session bookmark.
- **Deployment portability**: The same binary serves all five deployment shapes. Shape selection changes defaults, not code.
- **Third-party surfaces**: Any HTTP/WebSocket client can subscribe to projections and build a custom surface.

---

## Feedback Loops

- **Publish -> projection -> render**: A `publish` frame from the web UI becomes a Bus Pulse, which updates a projection, which pushes a `delta` frame back to all subscribers.
- **Cost tracking -> budget alerts**: The `cost_meter` projection tracks spend. When budget thresholds are crossed, the `alerts` projection fires, which the Agent Inbox surface renders as a notification.
- **Health probes -> deployment confidence**: `/healthz` and `/readyz` feed container orchestrators, which restart unhealthy instances, which re-establish Bus connections and projection state.

---

## Open Questions

1. **gRPC transport**: The spec reserves gRPC as a deferred typed transport for server-to-server consumers. No `tonic` dependency exists. Should gRPC wait until there is a concrete consumer, or should the protobuf definitions be written now to stabilize the schema?
2. **Projection auth granularity**: Should individual fields within a projection be filterable by role, or is projection-level access sufficient?
3. **Rate limiting strategy**: What are the right rate limits for `publish` frames from external clients? Per-topic, per-client, or global?

---

## Implementation Tasks

| Task | Where | What |
|---|---|---|
| Implement projection endpoints | `crates/roko-serve/src/routes/` | `GET /projections/:name` and `/stream` with cursor support |
| Wire WebSocket to projection subscriptions | `crates/roko-serve/src/routes/ws.rs` | Replace ad-hoc socket endpoints with channel-based subscriptions |
| Implement cursor-based reconnection | `crates/roko-serve/src/routes/ws.rs`, `sse.rs` | Carry cursor in every frame; support resume from last cursor |
| Add server-side projection filters | `crates/roko-serve/src/routes/` | Tenant, role, user, lineage, topic, time range |
| Implement per-subscription auth | `crates/roko-serve/src/routes/middleware.rs` | Check channel access per subscribe, not just per connection |
| Add deployment endpoints | `crates/roko-serve/src/routes/` | `/healthz`, `/readyz`, `/metrics`, state export/import |
| Add realtime telemetry | `crates/roko-serve/src/` | Connection counts, subscription counts, cursor lag, auth failures |
