# StateHub Projection Layer

> **Abstract:** This chapter keeps REF26 as one of the strongest interface directions. `StateHub` already exists and already serves shared dashboard state. The near-term work is to harden that live state path and, over time, evolve it toward smaller named projections with clearer query and replay contracts.

**Topic**: [12-interfaces](./INDEX.md)  
**Prerequisites**: [05-http-api-roko-serve.md](./05-http-api-roko-serve.md), [06-websocket-streaming.md](./06-websocket-streaming.md), [13-web-portal.md](./13-web-portal.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md)  
**Key sources**: `tmp/refinements/26-statehub-rearchitecture.md`, `tmp/refinements/30-rich-ux-primitives.md`

> **Implementation status - 2026-05-05**: `StateHub` lives in `roko-runtime`
> (`crates/roko-runtime/src/state_hub.rs`), exported as `roko_runtime::StateHub`,
> `roko_runtime::SharedStateHub`, and `roko_runtime::StateHubSender`. The move was
> completed by Task 104 which eliminated the previous `#[path]`-include hack in
> `roko-serve` and the fake `extern crate self as roko_core` alias. `roko-serve` and
> `roko-cli` both re-export the types from `roko-runtime` for downstream convenience.
> The `EventBus` used by `roko-serve` is a thin wrapper around
> `roko_runtime::event_bus::EventBus` (consolidated from a formerly separate
> implementation). This chapter describes the target evolution toward named
> projections; do not read the full projection catalog below as implemented today.

---

## 1. What StateHub Is

Today, `StateHub` is a shared dashboard hub that sits between the runtime event flow and the user-facing surfaces. Target-state, it evolves into a more explicit projection service with smaller typed read models.

The important boundary is simple:

- The **Bus** carries live Pulses.
- The **Substrate** holds durable Engrams.
- **StateHub** turns both into live projections that can be queried, streamed, replayed, restored, and used as the source of truth for rich UI primitives.

That makes StateHub the right abstraction for TUI panes, Web pages, dashboards, audit views, and analytics clients. It is not a rendering layer and not a transport layer. It is the shared state contract those layers consume, including the data needed for reasoning streams, tool banners, heuristic footnotes, uncertainty bars, replay scrubbers, and alternative renderings.

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
- `ProjectionContext` should expose cursor position, tenant scope, freshness, and any upstream snapshot needed to rebuild the view.

That keeps projection logic centralized and makes the same `apply()` function usable by the TUI, Web Portal, and any external client. It also keeps the rich UX layer honest: the UI can only render reasoning streams, uncertainty, or replay affordances if the projection contract exposes those fields in a typed way.

## 3. Canonical Projections

The kernel should ship with a small set of canonical projections that cover the high-value shared views.

| Name | State shape | Primary use |
|---|---|---|
| `cohort_health` | c-factor, roster, turn stats, delivery rates | team dashboards and collective intelligence |
| `active_tasks` | running tasks, progress, ETA, current agent | live work tracking |
| `alerts` | active warnings, gate failures, breaker trips, budget pressure | `Home / Pulse` summary and acknowledgement flows |
| `gate_pipeline` | rung status, pass/fail counts, pending checks, stale flag, last cursor | verification badges, release visibility, and explainable gate drilldown |
| `recent_episodes` | last N episodes, summaries, cursors, replay anchors, terminal state | TUI lists, replay pickers, browser history cards, and the replay scrubber |
| `replay_cursors` | episode cursor ranges, resume points, retention windows, seek targets | timeline scrubbing and deterministic resume |
| `heuristic_library` | calibration histogram, top hits, challenge history, footnote payloads, provenance | `Beliefs` review, heuristic footnotes, and tuning |
| `worldview_clusters` | clustered heuristic families, dominant beliefs, confidence windows | worldview browsing and skeptical inspection |
| `consensus_views` | grouped answers, minority views, confidence bands, evidence links | confidence-weighted aggregation views |
| `replication_ledger` | claim status, tests, witnesses, open questions | research and audit workflows |
| `plans_list` | selected plans, status counts, next checkpoints | `Plans` sidebar and mobile plan picker |
| `plan_detail/<id>` | DAG, task ordering, blockers, breakpoints, execution state | selected plan canvas and task focus |
| `cost_meter` | spend by model, role, and session | budget and usage dashboards |
| `config_current` | effective config values, profile overlays, gate thresholds | `Settings` forms and read-only summaries |
| `plugins_list` | installed plugins, versions, permissions, enabled state | plugin management in `Settings` |
| `secrets_status` | credential presence, rotation status, last validation result | secret management without exposing raw values |
| `bus_stats` | Pulses/sec by Topic, delivery rate, replay lag | ops and transport health |
| `substrate_stats` | tier sizes, balance distribution, retention windows | memory health and storage planning |
| `agent_trails` | per-agent timeline, current action, tool trace, reasoning markers | `Chat`, trace views, and reasoning streams |
| `explainability_state` | active heuristics, available tools, budget, pending gates, unknowns | shared explain panel and `roko explain`-style surfaces |
| `presentation_modes` | available renderings, chosen view, fallback view, user preference | alternative renderings across TUI and Web |
| `shortcut_registry` | registered keys, collisions, reserved keys, active context | keyboard help and discoverability |

These projections give every surface the same vocabulary. A Web page and a TUI pane should not invent separate ways to represent the same underlying state. For the REF29 browser surface, the important point is explicit page ownership: `Home / Pulse`, `Chat`, `Plans`, `Beliefs`, and `Settings` compose from shared projections instead of shipping page-local shadow state. For the rich UX primitives in REF30, the projection layer is where the UI learns whether to show a reasoning stream, a heuristic footnote, an uncertainty bar, a replay cursor, or an alternative rendering.

## 4. Query Plus Subscribe

StateHub should support a classic `query + subscribe` flow. A client asks for the current view, then attaches to the live stream using the same projection name and filter.

```http
GET /projections/gate_pipeline
GET /projections/gate_pipeline?filter=tenant:acme
GET /projections/recent_episodes?filter=user:me&limit=20
GET /projections/replay_cursors?filter=episode:ep_123
GET /projections/gate_pipeline/stream
```

The read side and the stream side are two faces of the same contract:

- `query` returns the current `State`.
- `stream` returns `Delta` updates with cursors.
- reconnecting clients resume from the last cursor instead of rebuilding from scratch.
- every response should carry freshness metadata so the UI can decide whether to show live state, a stale badge, or a degraded placeholder.

The API can be transported over HTTP, WebSocket, SSE, gRPC, or in-process channels. The transport changes, but the projection name and the state shape do not.

On the wire, clients should `subscribe` to a projection-backed `channel` rather than inventing per-surface socket shapes. This chapter owns the projection semantics; [06-websocket-streaming.md](./06-websocket-streaming.md) owns the transport vocabulary and auth/resume behavior.

## 5. Filters And Delivery

Subscriptions need to be narrower than "everything." StateHub should accept server-side filters that slice the projection by tenant, role, user, lineage, Topic, agent, episode, cursor range, or time range.

| Filter kind | What it narrows |
|---|---|
| `tenant` | only that tenant's state |
| `role` | only the views permitted for that role |
| `user` | only one principal's sessions or artifacts |
| `lineage` | only one Engram or episode chain |
| `topic` | only the matching Bus Topics |
| `agent` | only one agent trail or consensus participant |
| `episode` | only one episode's replay data |
| `cursor range` | only a bounded replay window |
| `time range` | only a bounded temporal window |

Delivery modes should be explicit too:

| Mode | Meaning |
|---|---|
| `AtMostOnce` | lossy delivery, fine for UIs |
| `AtLeastOnce` | retry until acknowledged |
| `ResumeFrom(cursor)` | continue from a known point |
| `SnapshotThenStream` | send a fresh state first, then follow with deltas |

The delivery contract matters for the rich primitives. A replay scrubber needs stable cursor ranges. A reasoning stream can fall back to a static summary when live Pulses stop. A heuristic footnote should disappear gracefully if the library is unavailable. A consensus view should still render the primary answer even when some minority evidence is stale. The projection layer should make those states explicit rather than letting the UI guess.

## 6. In-Process And Remote

The local path should be zero-friction. If the consumer is in the same process as StateHub, it should receive typed State and Delta values directly without serializing them first.

The remote path should use the same contract over a stable wire format. JSON is the default; other encodings can be added when the client needs them.

```rust
let mut sub = statehub.subscribe::<GatePipeline>(TopicFilter::all()).await?;
let mut state = sub.initial().await?;
while let Some(delta) = sub.next().await {
    GatePipeline::apply(&mut state, delta);
    render_gate_badges(&state);
}
```

That split matters because it lets the TUI stay efficient while still giving the Web Portal and external tools the same projection vocabulary. It also means alternative renderings stay thin: the projection owns the facts, while each surface chooses whether to show a timeline, a list, a DAG, or a collapsed summary.

## 7. Access Control

StateHub subscriptions must respect the same tenant and role rules as the rest of the control plane.

- Tenants should only see their own projections unless a policy explicitly widens access.
- Roles should only see projections permitted for that role.
- Sensitive projections such as cost, audit, or raw trace data should require stronger privileges.
- External clients should authenticate before they can query or stream projections.

Access control belongs in the projection layer because the projection itself defines what state exists to be observed. A client should never receive a view it is not allowed to read, even if the underlying data exists in the Bus or Substrate. The same rule applies to explainability views: if a user can see a decision, they should only see the active heuristics, tool traces, or rationale slices they are authorized to inspect.

## 8. Snapshot, Replay, And Testing

StateHub needs a durable lifecycle so projection state can be inspected and recovered.

- `snapshot()` should write the current State to an Engram.
- `restore()` should rebuild the projection from that snapshot and then catch up from the Bus.
- `replay()` should reconstruct state over a historical cursor range.
- `replay_cursors` should expose the seek points and window bounds that make the scrubber deterministic.

That lifecycle makes postmortems and audits practical. It also gives projection authors a clean testing surface.

Each projection should ship with three tests:

- Hydration test: a frozen Substrate snapshot produces the expected State.
- Delta-fold test: folding a Pulse sequence matches the hydrated result after persistence.
- Cursor-resume test: reconnecting with a cursor produces the same result as a continuous subscription.
- Staleness test: a missing upstream Pulse or delayed projection marks the view stale without breaking the page.

The replay scrubber depends on these guarantees. It should be able to jump between cursor anchors, repaint the current frame, and keep the surrounding timeline legible even when the full episode stream is not currently live.

## 9. Why TUI, Web, And External Consumers Share It

StateHub exists so every consumer sees the same truth.

- The **TUI** needs fast, typed, in-process state for rendering panes and keyboard-driven overlays.
- The **Web Portal** needs the same state over a remote transport, including explainability drawers and alternative renderings.
- **External dashboards** need a stable schema that does not depend on TUI internals.
- **Audit and analytics backends** need replayable, filterable views that can be resumed later.
- **Aggregation views** need the same typed consensus data whether they are rendered as a chart, a list, or a minority-view inspector.

Without StateHub, each surface invents its own state cache and its own partial projection logic. With StateHub, the projection is computed once and consumed many times. That is the architectural point of the refactor, and it is what makes the rich UX primitives practical instead of decorative.

## 10. Related Refinements

- [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) — canonical source for this chapter.
- [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md) — transport layer that carries these projections remotely.
- [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) — Bus, Topic, TopicFilter, Datum, and PulseSource vocabulary.
- [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md) — explainability panel and progressive disclosure behavior.
- [21-user-ux-running-agents.md](./21-user-ux-running-agents.md) — shared verb set and interface expectations for replay, inspect, and watch flows.
- [13-web-portal.md](./13-web-portal.md) — the first-party browser surface that consumes these projections.
- [tmp/refinements/29-web-ui-architecture.md](../../tmp/refinements/29-web-ui-architecture.md) — web pages that should consume shared projections rather than reimplementing state.
- [tmp/refinements/30-rich-ux-primitives.md](../../tmp/refinements/30-rich-ux-primitives.md) — UI primitives that become simpler when projections are already typed, queryable, and freshness-aware.
