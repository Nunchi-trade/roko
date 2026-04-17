# StateHub Projection Layer

> **Abstract:** This chapter propagates `tmp/refinements/26-statehub-rearchitecture.md` into the canonical docs tree. StateHub is no longer a TUI-only cache; it is the kernel projection layer that turns Bus + Substrate into typed, queryable, live-updating views for the TUI, Web Portal, and external consumers. The stable contract is a named projection with `State`, `Delta`, a reducer, filters, and replay cursors. REF27 then carries that same contract over the shared realtime surface defined in [06-websocket-streaming.md](./06-websocket-streaming.md). See also [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) for the shared kernel vocabulary.

**Topic**: [12-interfaces](./INDEX.md)  
**Prerequisites**: [05-http-api-roko-serve.md](./05-http-api-roko-serve.md), [06-websocket-streaming.md](./06-websocket-streaming.md), [13-web-portal.md](./13-web-portal.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md)  
**Key sources**: `tmp/refinements/26-statehub-rearchitecture.md`

---

## 1. What StateHub Is

StateHub is the shared projection service that sits between the Bus and the user-facing surfaces. It listens to Pulses, folds them into typed state, and serves those states through a consistent read model.

The important boundary is simple:

- The **Bus** carries live Pulses.
- The **Substrate** holds durable Engrams.
- **StateHub** turns both into live projections that can be queried, streamed, replayed, and restored.

That makes StateHub the right abstraction for TUI panes, Web pages, dashboards, audit views, and analytics clients. It is not a rendering layer and not a transport layer. It is the shared state contract those layers consume.

## 2. Projection Contract

Every projection is defined once and shared by every consumer. The projection name is stable, the state shape is typed, and deltas are folded in one place.

```rust
pub trait Projection: Send + Sync + 'static {
    const NAME: &'static str;

    type State: Serialize + DeserializeOwned + Clone + Send + 'static;
    type Delta: Serialize + DeserializeOwned + Clone + Send + 'static;

    fn apply(state: &mut Self::State, delta: Self::Delta);

    fn topics() -> &'static [&'static str];

    async fn hydrate(ctx: &ProjectionContext) -> Result<Self::State>;

    fn reduce(pulse: &Pulse) -> Option<Self::Delta>;
}
```

The contract is intentionally narrow:

- `hydrate()` builds the initial state from historical Engrams and recent Pulses.
- `reduce()` converts incoming Pulses into a typed Delta.
- `apply()` folds a Delta into the current State.
- `topics()` declares which Bus Topics matter for the projection.

That keeps projection logic centralized and makes the same `apply()` function usable by the TUI, Web Portal, and any external client.

## 3. Canonical Projections

The kernel should ship with a small set of canonical projections that cover the high-value shared views.

| Name | State shape | Primary use |
|---|---|---|
| `cohort_health` | c-factor, roster, turn stats, delivery rates | team dashboards and collective intelligence |
| `active_tasks` | running tasks, progress, ETA, current agent | live work tracking |
| `gate_pipeline` | rung status, pass/fail counts, pending checks | verification and release visibility |
| `recent_episodes` | last N episodes, summaries, cursors | TUI lists and replay pickers |
| `heuristic_library` | calibration histogram, top hits, challenge history | belief review and tuning |
| `cost_meter` | spend by model, role, and session | budget and usage dashboards |
| `bus_stats` | Pulses/sec by Topic, delivery rate, replay lag | ops and transport health |
| `substrate_stats` | tier sizes, balance distribution, retention windows | memory health and storage planning |
| `agent_trails` | per-agent timeline, current action, tool trace | chat and trace views |
| `replication_ledger` | claim status, tests, witnesses, open questions | research and audit workflows |

These projections give every surface the same vocabulary. A Web page and a TUI pane should not invent separate ways to represent the same underlying state.

## 4. Query Plus Subscribe

StateHub should support a classic `query + subscribe` flow. A client asks for the current view, then attaches to the live stream using the same projection name and filter.

```http
GET /projections/cohort_health
GET /projections/cohort_health?filter=tenant:acme
GET /projections/recent_episodes?filter=user:me&limit=20
GET /projections/cohort_health/stream
```

The read side and the stream side are two faces of the same contract:

- `query` returns the current `State`.
- `stream` returns `Delta` updates with cursors.
- reconnecting clients resume from the last cursor instead of rebuilding from scratch.

The API can be transported over HTTP, WebSocket, SSE, gRPC, or in-process channels. The transport changes, but the projection name and the state shape do not.

On the wire, clients should `subscribe` to a projection-backed `channel` rather than inventing per-surface socket shapes. This chapter owns the projection semantics; [06-websocket-streaming.md](./06-websocket-streaming.md) owns the transport vocabulary and auth/resume behavior.

## 5. Filters And Delivery

Subscriptions need to be narrower than "everything." StateHub should accept server-side filters that slice the projection by tenant, role, user, lineage, Topic, or time range.

| Filter kind | What it narrows |
|---|---|
| `tenant` | only that tenant's state |
| `role` | only the views permitted for that role |
| `user` | only one principal's sessions or artifacts |
| `lineage` | only one Engram or episode chain |
| `topic` | only the matching Bus Topics |
| `time range` | only a bounded temporal window |

Delivery modes should be explicit too:

| Mode | Meaning |
|---|---|
| `AtMostOnce` | lossy delivery, fine for UIs |
| `AtLeastOnce` | retry until acknowledged |
| `ResumeFrom(cursor)` | continue from a known point |

This keeps StateHub useful for both low-latency rendering and durable replay.

## 6. In-Process And Remote

The local path should be zero-friction. If the consumer is in the same process as StateHub, it should receive typed State and Delta values directly without serializing them first.

The remote path should use the same contract over a stable wire format. JSON is the default; other encodings can be added when the client needs them.

```rust
let mut sub = statehub.subscribe::<CohortHealth>(TopicFilter::all()).await?;
let mut state = sub.initial().await?;
while let Some(delta) = sub.next().await {
    CohortHealth::apply(&mut state, delta);
    render_dashboard(&state);
}
```

That split matters because it lets the TUI stay efficient while still giving the Web Portal and external tools the same projection vocabulary.

## 7. Access Control

StateHub subscriptions must respect the same tenant and role rules as the rest of the control plane.

- Tenants should only see their own projections unless a policy explicitly widens access.
- Roles should only see projections permitted for that role.
- Sensitive projections such as cost or audit data should require stronger privileges.
- External clients should authenticate before they can query or stream projections.

Access control belongs in the projection layer because the projection itself defines what state exists to be observed. A client should never receive a view it is not allowed to read, even if the underlying data exists in the Bus or Substrate.

## 8. Snapshot, Replay, And Testing

StateHub needs a durable lifecycle so projection state can be inspected and recovered.

- `snapshot()` should write the current State to an Engram.
- `restore()` should rebuild the projection from that snapshot and then catch up from the Bus.
- `replay()` should reconstruct state over a historical cursor range.

That lifecycle makes postmortems and audits practical. It also gives projection authors a clean testing surface.

Each projection should ship with three tests:

- Hydration test: a frozen Substrate snapshot produces the expected State.
- Delta-fold test: folding a Pulse sequence matches the hydrated result after persistence.
- Cursor-resume test: reconnecting with a cursor produces the same result as a continuous subscription.

## 9. Why TUI, Web, And External Consumers Share It

StateHub exists so every consumer sees the same truth.

- The **TUI** needs fast, typed, in-process state for rendering panes.
- The **Web Portal** needs the same state over a remote transport.
- **External dashboards** need a stable schema that does not depend on TUI internals.
- **Audit and analytics backends** need replayable, filterable views that can be resumed later.

Without StateHub, each surface invents its own state cache and its own partial projection logic. With StateHub, the projection is computed once and consumed many times. That is the architectural point of the refactor.

## 10. Related Refinements

- [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) — canonical source for this chapter.
- [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md) — transport layer that carries these projections remotely.
- [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) — Bus, Topic, TopicFilter, Datum, and PulseSource vocabulary.
- [tmp/refinements/29-web-ui-architecture.md](../../tmp/refinements/29-web-ui-architecture.md) — web pages that should consume shared projections rather than reimplementing state.
- [tmp/refinements/30-rich-ux-primitives.md](../../tmp/refinements/30-rich-ux-primitives.md) — UI primitives that become simpler when projections are already typed and queryable.
