# Surfaces as Lens Composition

> Depth for [20-SURFACES.md](../../unified/20-SURFACES.md). Covers the architectural insight that every surface -- CLI, TUI, HTTP, Web -- is a Lens Cell reading the same StateHub, and how the 9-verb model maps to Graph operations.

---

## 1. The Core Insight

A **surface** in Roko is not a separate UI. It is a composition of Lens Cells reading the same StateHub Store. The CLI, TUI, HTTP API, and web dashboard all consume identical typed projections from the same source of truth. They differ only in rendering -- what they show and how they show it -- never in the data they observe.

This means adding a new surface (a mobile app, a Slack bot, a VS Code panel) requires **zero backend changes**. The new surface subscribes to existing StateHub projections, renders them in its medium, and emits surface events back through the Bus. The kernel never learns that a new consumer exists.

See [02-CELL.md](../../unified/02-CELL.md) for the Observe protocol that Lens Cells implement. See [15-TELEMETRY.md](../../unified/15-TELEMETRY.md) for how StateHub computes projections from Bus Pulses and Store Signals.

---

## 2. StateHub as a Store with Named Projections

StateHub sits between the kernel's two fabrics (Bus and Store) and the user-facing surfaces. It subscribes to Bus topics, reads durable Signals from Store, and folds both into typed **projections** -- named, versioned, queryable views of system state.

Each projection is defined by a single Rust trait:

```rust
/// A named, typed projection computed from Bus Pulses and Store Signals.
/// See [02-CELL.md](../../unified/02-CELL.md) for the Observe protocol.
pub trait Projection: Send + Sync + 'static {
    /// Stable name used by all surfaces to subscribe.
    const NAME: &'static str;

    /// The full state snapshot type.
    type State: Serialize + DeserializeOwned + Clone + Send + 'static;

    /// An incremental update type.
    type Delta: Serialize + DeserializeOwned + Clone + Send + 'static;

    /// Fold a delta into the current state (pure function).
    fn apply(state: &mut Self::State, delta: Self::Delta);

    /// Which Bus topics this projection listens to.
    fn topics() -> &'static [&'static str];

    /// Build initial state from historical data.
    async fn hydrate(ctx: &ProjectionContext) -> Result<Self::State>;

    /// Convert an incoming Pulse into a typed delta (or None if irrelevant).
    fn reduce(pulse: &Pulse) -> Option<Self::Delta>;
}
```

This contract is intentionally narrow. A projection declares what it needs (topics), how to build itself (hydrate), how to update (reduce + apply), and what it produces (State + Delta). Every surface -- TUI, HTTP, CLI -- calls the same `apply()` function. The projection is computed once and consumed many times.

---

## 3. The Projection Catalog

The kernel ships approximately 18 named projections. Every surface draws from this catalog:

| Projection Name | State Shape | What It Shows | Primary Consumers |
|---|---|---|---|
| `cohort_health` | c-factor, roster, turn stats, delivery rates | Team coordination quality | Stigmergy Minimap, Agent Inbox |
| `active_tasks` | Running tasks, progress, ETA, current agent | Live work tracking | Workbench, Generative Canvas |
| `alerts` | Warnings, gate failures, breaker trips, budget pressure | Urgent notifications | Agent Inbox |
| `gate_pipeline` | Rung status, pass/fail counts, pending checks | Verification state | Workbench, Generative Canvas |
| `recent_episodes` | Last N episodes with summaries and cursors | History and replay | Episode browser |
| `cost_meter` | Spend by model, role, and session | Budget visibility | Workbench, System tab |
| `agent_vitality` | Per-agent vitality, phase, current task | Agent health | Agent Inbox, Autonomy Slider |
| `knowledge_health` | Tier distribution, demurrage balances, dream cycles | Memory state | Knowledge tab |
| `c_factor` | Composite c-factor, turn-taking entropy, diversity | Collective intelligence | Stigmergy Minimap, Autonomy Slider |
| `agent_trails` | Per-agent timeline, tool trace, reasoning markers | Live agent detail | Chat, trace views |
| `plans_list` | Selected plans, status counts, next checkpoints | Plan overview | Plans sidebar |
| `plan_detail/<id>` | DAG, task ordering, blockers, execution state | Plan canvas | Flow inspector |
| `config_current` | Effective config values, profile overlays | Settings | System tab |
| `bus_stats` | Pulses/sec by topic, delivery rate | Transport health | System tab |
| `substrate_stats` | Tier sizes, balance distribution | Storage health | System tab |
| `heuristic_library` | Calibration histogram, top hits, challenge history | Belief review | Learn tab |
| `plugins_list` | Installed plugins, versions, permissions | Plugin management | Settings |
| `secrets_status` | Credential presence, rotation, last validation | Secret management | Settings |

---

## 4. The Query + Subscribe Protocol

Every surface interacts with StateHub through two operations:

1. **Query**: Request the current state of a projection (one-shot read).
2. **Subscribe**: Attach to the live delta stream for a projection (continuous).

```
Surface                              StateHub
  |--- query(cohort_health) -------->|
  |<-- State snapshot ---------------|
  |                                   |
  |--- subscribe(cohort_health) ---->|
  |<-- Delta .........................| (continuous)
  |<-- Delta .........................|
  |<-- Delta .........................|
```

When a surface reconnects (browser reload, TUI restart, WebSocket drop), it resumes from its last known cursor rather than rebuilding from scratch. The cursor is carried in every delta frame. See [06-websocket-streaming.md](../../docs/12-interfaces/06-websocket-streaming.md) for the wire protocol that transports this.

In-process consumers (the TUI, running in the same binary) receive typed `State` and `Delta` values directly -- no serialization. Remote consumers (the web dashboard, external tools) receive the same data over JSON/WebSocket/SSE.

---

## 5. The Five Named Surfaces

The spec defines five named surfaces. Each is a composition of projections consumed plus events emitted:

| Surface | Projections Consumed | Events Emitted | What It Does |
|---|---|---|---|
| **Workbench** | `active_tasks`, `gate_pipeline`, `cost_meter`, `agent_vitality` | TaskAssign, FlowCancel, HumanRespond, MacroAdjust | Structured task delegation (Linear/Notion pattern) |
| **Agent Inbox** | Bus (filtered by `tagged_for_human`), `agent_vitality`, `cohort_health` | Approve, Reject, Defer, Dismiss | Calm notification center |
| **Generative Canvas** | GraphRegistry, CellRegistry, `active_tasks`, `gate_pipeline` | NodeAdd, EdgeCreate, MacroPromote, GraphSave | Visual Graph editor |
| **Stigmergy Minimap** | Bus (pheromone Pulses), `agent_vitality`, `c_factor`, `cohort_health`, `knowledge_health` | SpawnAgent, GroupSelect, PheromoneDeposit | RTS-style coordination view |
| **Autonomy Slider** | AgentRuntime, `agent_vitality`, `c_factor`, SecurityEvents | AutonomyLevelChange, CapabilityGrant/Revoke | Progressive trust control (levels 0-4) |

Surfaces are **not rendering targets**. A surface is a data contract. Four rendering targets (CLI, TUI, Dashboard/Web, Visual Editor) implement these five surfaces. Every rendering target can render every surface, though each has natural affinities.

---

## 6. The 9-Verb Model as Graph Operations

The 9 canonical verbs map directly to Cell protocol invocations. Every verb, on every surface, triggers the same underlying Graph operation:

| Verb | What It Means | Cell Protocol | Graph Pattern |
|---|---|---|---|
| **ask** | Single-turn query | Compose + Execute | Cognitive Loop (one iteration) |
| **plan** | Propose without executing | Route (select strategy) | Plan generation Graph |
| **do** | Execute a task or plan | Execute (full loop) | Executor Graph with gates |
| **watch** | Observe live progress | Observe (subscribe to projections) | Lens Graph over `active_tasks` |
| **inspect** | Drill into a durable artifact | Observe + Store query | Lens Graph over `recent_episodes` / `agent_trails` |
| **replay** | Re-run a prior episode | Store query + Execute | Replay Graph (historical inputs, new execution) |
| **learn** | Browse and curate heuristics | Score (read calibration state) | Lens Graph over `heuristic_library` |
| **tune** | Adjust thresholds and config | React (emit config change Pulses) | Config mutation Graph |
| **connect** | Add plugins, providers, MCP | Connect protocol | Integration Graph |

The important consequence: when a user says `ask` in the CLI, `/ask` in the TUI chat pane, or clicks "Ask" in the web dashboard, the same Graph fires. The surface determines how the result renders. The verb determines what happens.

---

## 7. How Adding a New Surface Works

To build a new surface (say, a Slack bot):

1. **Choose projections**: The Slack bot needs `alerts` (for gate failures) and `active_tasks` (for progress).
2. **Subscribe**: Connect to StateHub via WebSocket or SSE and subscribe to those two projection channels.
3. **Render**: Convert `State` and `Delta` messages into Slack message blocks.
4. **Emit events**: When a user reacts with a checkmark emoji, emit an `Approve` surface event back through the Bus.

No kernel code changes. No new endpoints. No new state management. The Slack bot is a Lens Cell that reads projections and emits Pulses -- the same pattern as every other surface.

```rust
// Pseudocode for a Slack bot surface
async fn slack_bot(statehub: &StateHub, slack: &SlackClient) {
    let mut alerts = statehub.subscribe::<Alerts>(TopicFilter::all()).await?;
    let mut state = alerts.initial().await?;

    while let Some(delta) = alerts.next().await {
        Alerts::apply(&mut state, delta);
        for alert in &state.active_alerts {
            if alert.urgency >= UrgencyLevel::Question {
                slack.post_message(render_alert_block(alert)).await?;
            }
        }
    }
}
```

---

## 8. Surface-to-TUI Tab Mapping

The TUI (`roko dashboard`) has seven tabs. Each tab renders one or more named surfaces by subscribing to the corresponding projections:

| TUI Tab | Surfaces Rendered | Key Projections |
|---|---|---|
| **F1 Workbench** | Workbench + Agent Inbox (badge) | `active_tasks`, `gate_pipeline`, `cost_meter` |
| **F2 Canvas** | Generative Canvas | `active_tasks`, `gate_pipeline` |
| **F3 Flows** | Workbench (detail view) | `active_tasks`, `gate_pipeline`, `cost_meter` |
| **F4 Inbox** | Agent Inbox | `agent_vitality`, `cohort_health` |
| **F5 Knowledge** | (rendering-target view) | `knowledge_health` |
| **F6 System** | Autonomy Slider | `agent_vitality`, `c_factor`, `cost_meter` |
| **F7 Agents** | Stigmergy Minimap | `c_factor`, `cohort_health`, `knowledge_health` |

F5 Knowledge and parts of F6 System are rendering-target-specific views that consume projections directly without mapping to a named surface contract. This is permitted -- surfaces are contracts, not constraints.

---

## What This Enables

- **Zero-backend new surfaces**: A mobile app, Slack bot, VS Code panel, or tmux dashboard can be built by subscribing to existing projections.
- **Guaranteed consistency**: Every surface sees the same state because they read the same projections. No stale caches, no divergent views.
- **Independent evolution**: The kernel can add new projections without changing any surface. Surfaces can add new renderings without changing the kernel.
- **Replay and testing**: Projections can be snapshot, replayed, and tested in isolation. A frozen projection snapshot produces a deterministic surface render.

---

## Feedback Loops

- **Projection staleness**: If a projection falls behind the Bus, it marks itself stale. Surfaces render a degraded indicator rather than silently showing old data.
- **Surface event processing**: Events emitted by surfaces (Approve, TaskAssign, AutonomyLevelChange) flow back through the Bus, which triggers projection updates, which flow back to surfaces. The loop is: user action -> Bus Pulse -> projection delta -> surface render.
- **Cursor-based resume**: When surfaces reconnect, they resume from their last cursor. The projection layer tracks retention windows and can either replay from history or send a fresh state snapshot when the cursor is too old.

---

## Open Questions

1. **Projection cardinality**: Should `plan_detail/<id>` be one projection per plan, or a single `plan_detail` projection with a required filter parameter? The former is simpler; the latter scales better.
2. **Cross-surface session continuity**: When a user starts a task in CLI and switches to TUI, how does session context transfer? The current answer is shared `.roko/` state, but a more explicit handoff protocol may be needed.
3. **Projection versioning**: When a projection's `State` type changes, how do older clients handle the new schema? The current plan is additive-only changes with `#[serde(default)]`, but a formal versioning scheme may be warranted.
4. **Access control granularity**: Should projection access be all-or-nothing, or should individual fields within a projection be filterable by role?

---

## Implementation Tasks

| Task | Where | What |
|---|---|---|
| Harden `StateHub` with typed projection registry | `crates/roko-core/src/state_hub.rs` | Currently publishes `DashboardSnapshot`; evolve toward smaller named projections with the `Projection` trait |
| Implement `query + subscribe` for HTTP surfaces | `crates/roko-serve/src/routes/` | Add `GET /projections/:name` and `GET /projections/:name/stream` endpoints |
| Wire TUI tabs to projection subscriptions | `crates/roko-cli/src/tui/` | Replace direct state reads with in-process projection subscriptions |
| Define canonical projection schemas | `crates/roko-core/src/projections/` | New module with typed `State` and `Delta` structs for each projection |
| Implement cursor-based resume | `crates/roko-serve/src/routes/ws.rs` | Carry cursor in every delta frame; support resume on reconnect |
| Add staleness tracking to projections | `crates/roko-core/src/state_hub.rs` | Each projection tracks freshness; surfaces render stale indicator when behind |
