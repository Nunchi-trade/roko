# WebSocket, SSE, and gRPC Realtime Surface

> **Abstract:** This chapter documents the target-state shared realtime protocol. Today, Roko ships WebSocket and SSE endpoints in `roko-serve`; gRPC is deferred. The useful near-term work is to harden the existing transports and document their shared cursor and replay behavior clearly.

> **Implementation**: Partial today, broader protocol target-state

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [05-http-api-roko-serve.md](./05-http-api-roko-serve.md), [13-web-portal.md](./13-web-portal.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md)
**Key sources**: `tmp/refinements/27-realtime-event-surface.md`, `tmp/refinements/26-statehub-rearchitecture.md`

> **Implementation status**: WebSocket and SSE are the current realtime transports. gRPC (`tonic`) is **deferred**; no `tonic` dependency or protobuf service exists in the workspace. Treat the gRPC parts of this chapter as target-state protocol notes, not current implementation.

---

## 1. Why Multiple Transports

REF27 turns realtime delivery into a first-class external contract instead of a handful of ad hoc socket endpoints. Today, that means two real transports and one deferred option.

The rule is simple:

- `WebSocket` is the default for interactive clients that both read and publish Pulses.
- `SSE` is the default for dashboards, lightweight observers, and browser contexts that benefit from plain HTTP plus native reconnect behavior.
- `gRPC` streaming is a deferred typed transport for server-to-server or audit consumers if demand justifies it later.

WebSocket and SSE are the current transports. The protocol notes below also sketch how a future gRPC transport could map onto the same conceptual contract:

- `query` current state
- `subscribe` to a channel
- receive `state`, `delta`, or a raw `pulse` frame
- resume from a cursor after reconnect
- optionally `publish` a Pulse back into the runtime when the transport is bidirectional

This keeps browser UIs, mobile feeds, Slack bots, dashboards, and another Roko instance on one vocabulary rather than forcing each consumer to learn a different integration story.

## 2. Realtime Surface in the Two-Fabric Model

The realtime surface is the wire layer that externalizes the kernel's two fabrics:

- `Bus` is still the transport fabric for live `Pulse` delivery inside the runtime.
- `Substrate` is still the storage fabric for durable `Engram` history.
- `StateHub` turns Bus plus Substrate into typed projections that external consumers can query and subscribe to.

Remote clients therefore do not tap arbitrary internal queues. They attach to one of three external shapes:

- a named `projection:*` channel from `StateHub`
- a filtered `topic:*` view over raw Bus Topics
- a filtered `engram-stream:*` view over live Substrate writes

That distinction matters operationally. Interactive surfaces mostly consume projections; bots and diagnostics may consume filtered Topics; replication or audit flows may consume live Engram streams.

## 3. Canonical Frame Vocabulary

The logical frame shape is transport-agnostic. JSON is the baseline encoding for `WebSocket` and `SSE`; `gRPC` maps the same fields into typed request and response structs.

```json
{
  "type": "subscribe",
  "id": "req-12345",
  "payload": {}
}
```

### 3.1 Frame Types

| `type` | Direction | Meaning |
|---|---|---|
| `subscribe` | client -> server | Open a live subscription on one channel |
| `unsubscribe` | client -> server | Close one live subscription |
| `query` | client -> server | One-shot read of a projection, Engram, or heuristic |
| `publish` | client -> server | Publish a user-originated Pulse; allowed on bidirectional transports |
| `state` | server -> client | Full state snapshot, usually the first reply for a projection |
| `delta` | server -> client | Incremental update for a projection |
| `pulse` | server -> client | Raw topic or stream item that is not a projection delta |
| `ack` | server -> client | Success confirmation for subscribe, unsubscribe, or publish |
| `error` | server -> client | Request-local failure without killing the connection |
| `ping` / `pong` | both | Heartbeat for long-lived sockets |

### 3.2 Subscribe

Every client must be able to `subscribe` with an explicit `channel` and an optional `cursor`:

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

The server replies with:

- `state` followed by `delta` for projection channels
- `ack` followed by `pulse` for raw topic channels
- `error` if the identity is not allowed to view that channel or filter

The key guarantee is transport parity: a browser client and a gRPC consumer should get the same logical stream when they subscribe to the same channel with the same cursor.

### 3.3 Query

`query` is the one-shot read complement to `subscribe`:

```json
{
  "type": "query",
  "id": "q-ghi",
  "payload": {
    "target": "projection:gate_pipeline",
    "filter": {"session": "sess_xyz"},
    "at_cursor": "0x04f1"
  }
}
```

Projection targets return `state`. Raw stream targets may return a single `pulse` frame or a bounded list, depending on the target family.

### 3.4 Publish

`publish` is the asymmetric inbound path that makes external UIs and bots first-class participants rather than passive observers.

```json
{
  "type": "publish",
  "id": "pub-def",
  "payload": {
    "topic": "user.prompt",
    "body": {"text": "Focus on auth.rs first"},
    "source": "portal"
  }
}
```

Rules:

- `publish` is required for `WebSocket`.
- `publish` is optional for bidirectional `gRPC`.
- `SSE` remains receive-only.
- published Pulses are rate-limited, topic-allow-listed, schema-validated, and audited.

## 4. Channel Taxonomy

The subscription unit is a `channel` string. The channel is not a replacement for `Topic`; it is the external selector that points at a projection, filtered Topic view, or stream.

| Prefix | Meaning | Example |
|---|---|---|
| `projection:` | Named `StateHub` projection | `projection:cohort_health` |
| `topic:` | Raw Bus Topic stream, pattern allowed | `topic:gate.failed.*` |
| `engram-stream:` | Live filtered Substrate writes | `engram-stream:kind=heuristic` |
| `agent:` | Per-agent activity feed | `agent:agt_042` |
| `session:` | Per-session conversation and progress | `session:sess_xyz` |

Consumers may mix multiple channel kinds on one connection. The server owns the fanout; the client should not open one transport connection per subscription unless it has a very specific reason.

### 4.1 Projection-First Default

Most first-party consumers should subscribe to projections first:

- `projection:active_tasks`
- `projection:agent_trails`
- `projection:gate_pipeline`
- `projection:cohort_health`
- `projection:recent_episodes`

That preserves a stable external contract even when internal Topics evolve.

### 4.2 Compatibility Aliases

Historical route families such as per-agent sockets can remain as convenience aliases, but they should resolve onto the same channel registry. For example:

- `/ws/agent/:id` is an alias over `agent:<id>` or `projection:agent_trails` with an agent filter
- `/ws/cfactor` is an alias over `projection:cohort_health`
- renderer-specific views still fold through the same cursor and auth model

The public contract is the channel registry, not an ever-growing list of bespoke endpoints.

## 5. Auth, Authorization, and Safety

Connections authenticate once and subscribe many times.

Supported identity forms:

- API key for automation and machine consumers
- OIDC bearer token for user-scoped remote clients
- session cookie for first-party browser flows
- per-tenant scoped token when a deployment exposes only part of the channel space

Authorization is per subscription, not just per connection. The server checks:

- whether this identity may see the requested channel
- whether the supplied filter narrows access safely
- whether `publish` to the requested Topic is allowed

If a subscription is denied, the server returns `error` for that request id rather than dropping the whole connection. One bad subscription should not kill the session.

Safety rules on the surface:

- no user-supplied filter functions cross process boundaries; filters stay declarative
- `publish` applies topic allow-lists, payload size limits, and schema validation
- secret-bearing fields are stripped before messages leave the runtime
- auth failures and replay denials are rate-limited and traced for audit

See also [../11-safety/INDEX.md](../11-safety/INDEX.md), [../00-architecture/05-provenance-and-attestation.md](../00-architecture/05-provenance-and-attestation.md), and [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md).

## 6. Cursors, Replay, and Back-Pressure

Every outbound frame carries a cursor. Clients persist the last cursor they have successfully applied and present it again on reconnect.

### 6.1 Resume Rules

1. client reconnects
2. client resends `subscribe` with the last known cursor for that channel
3. server replays from retained history when possible
4. if the cursor is too old:
   - projection channels return a fresh `state` plus the current cursor
   - raw topic channels return `error` plus the current cursor so the client can re-query or accept a gap

`SSE` uses its standard resume header to carry the cursor automatically. `WebSocket` and `gRPC` carry it in the subscribe payload.

### 6.2 Back-Pressure Modes

Back-pressure is explicit because different consumers need different tradeoffs:

| Mode | Behavior | Typical use |
|---|---|---|
| `AtMostOnce` | Oldest messages may drop under pressure | live dashboards and decorative views |
| `Coalesce` | Projection deltas may collapse into a newer delta or fresh state | browser dashboards and mobile feeds |
| `ResumeRequired` | Server disconnects or errors once the client falls too far behind | audit, replication, and compliance flows |

Recommended defaults:

- `agent_trails` token chunks coalesce in short windows
- `cohort_health` metric updates coalesce aggressively
- `gate_pipeline` rung transitions should not be dropped
- `engram-stream:*` for audit or replication should prefer `ResumeRequired`

These rules keep the external surface honest about delivery guarantees instead of hiding drops behind best-effort transport behavior.

## 7. Heartbeats and Presence

Long-lived subscriptions need an explicit liveness contract:

- `WebSocket` uses `ping` / `pong`
- `SSE` uses comment heartbeats so proxies do not treat the stream as idle
- `gRPC` uses transport keep-alive plus application-level stream timeouts

Presence is reserved as a namespaced channel family rather than a one-off feature:

- `topic:presence.*` for low-level connection presence
- `projection:session_presence` for UI-friendly summaries such as "3 viewers on this plan"

Presence is useful, but it is not the core of the protocol. The core remains query, subscribe, resume, and publish.

## 8. First-Party Client Shape

The first-party client libraries should hide the transport choice behind one subscription API.

Common responsibilities:

- choose `WebSocket`, `SSE`, or `gRPC` based on environment and use case
- expose typed `subscribe(channel, filter, handler)` and `query(target, filter)` helpers
- reconnect with exponential backoff
- resume with the last applied cursor
- translate `error` frames into idiomatic exceptions or callbacks
- rate-limit publishes client-side before the server rejects them

The first three client targets are:

- TypeScript for Web and browser extensions
- Python for scripts, bots, and analytics jobs
- Rust for native apps and server-to-server consumers

Those clients should share the same schema definitions as the StateHub projection layer so projection state does not get hand-transcribed in multiple places.

## 9. Recommended Consumer Patterns

| Consumer | Preferred transport | Typical channels |
|---|---|---|
| Web UI | `WebSocket` | `projection:*`, plus `publish` for user input |
| Mobile or passive browser view | `SSE` | `projection:cohort_health`, `projection:active_tasks` |
| Slack or chat bot | `WebSocket` | `topic:gate.failed.*`, `topic:safety.approval.requested` |
| Grafana or metrics-tail consumer | `SSE` | `projection:bus_stats`, `projection:substrate_stats` |
| Audit log or compliance sink | `gRPC` | `engram-stream:*`, `topic:safety.*` |
| Cross-instance replication | `gRPC` | `engram-stream:*` with kind filters |
| Browser extension | `WebSocket` | `session:*`, `projection:active_tasks` |

The transport choice is a deployment detail. The channel vocabulary is the product contract.

## 10. Edge Cases Worth Documenting Once

The server and first-party clients should absorb the common operational sharp edges:

- browsers limit concurrent `SSE` connections per origin, so multiplex subscriptions
- many reverse proxies buffer `SSE` unless buffering is disabled
- reconnect backoff should cap instead of growing forever
- the cursor format is opaque to clients even though it must remain valid for the retention window
- unknown fields and unknown frame kinds must be ignored so additive protocol evolution stays non-breaking

This is where the wire contract becomes a force multiplier: custom surfaces can inherit sane behavior instead of rediscovering it.

## 11. Observability and Stability

The realtime surface is part of the production control plane and needs its own telemetry:

| Metric | Meaning |
|---|---|
| `roko.realtime.connections` | open transport connections |
| `roko.realtime.subscriptions` | active subscriptions by channel family |
| `roko.realtime.messages_per_second` | inbound and outbound traffic rate |
| `roko.realtime.cursor_lag` | how far behind consumers are |
| `roko.realtime.auth_failures` | failed auth or subscribe attempts |

The wire contract also needs an explicit stability bar:

- adding fields is non-breaking
- removing or renaming public fields is breaking
- frame meaning must not silently change under an existing `type`
- clients must ignore unknown fields
- servers must continue accepting previously emitted cursors for the retention window

That is the only way downstream consumers can build against the surface with confidence.

## 12. Cross-References

- [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md) — canonical source for this chapter.
- [22-statehub-projection-layer.md](./22-statehub-projection-layer.md) — projection contract carried by the realtime surface.
- [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) — control-plane routes and auth integration.
- [13-web-portal.md](./13-web-portal.md) — first-party browser consumer of the realtime surface.
- [21-user-ux-running-agents.md](./21-user-ux-running-agents.md) — `watch` as the cross-surface live-progress verb.
- [../19-deployment/11-remote-orchestrator.md](../19-deployment/11-remote-orchestrator.md) — deployment-facing guidance for exposing the surface remotely.
- [../19-deployment/12-production-hardening.md](../19-deployment/12-production-hardening.md) — operational concerns such as proxying, cursor retention, and telemetry.
