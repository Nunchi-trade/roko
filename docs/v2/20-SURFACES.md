# 20 -- Surfaces

> Five named surfaces -- Workbench, Agent Inbox, Generative Canvas, Stigmergy Minimap, Autonomy Slider -- define protocol-level data contracts between system and user. Surfaces are projections from the StateHub plus interaction contracts. Four rendering targets (CLI, TUI, Dashboard, Visual Editor) implement all five surfaces. Third parties build new surfaces consuming the same projections.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse, Bus), [02-CELL](02-CELL.md) (Cell protocol), [03-GRAPH](03-GRAPH.md) (Graph composition), [05-AGENT](05-AGENT.md) (vitality, CorticalState), [15-TELEMETRY](15-TELEMETRY.md) (StateHub, Lenses, c-factor, 7 core projections), [16-SECURITY](16-SECURITY.md) (autonomy levels 0-4, CaMeL), [19-CONFIG](19-CONFIG.md) (TOML schemas, domain profiles)

---

## 1. Design Principles

A **surface** is a projection from the StateHub ([15-TELEMETRY](15-TELEMETRY.md)) plus an interaction contract. The projection defines what data the surface consumes. The interaction contract defines what events the surface emits. Third parties can build new surfaces by consuming the same projections and emitting the same events.

Five named surfaces. Four rendering targets. Every surface can be rendered by any target -- but each target has natural affinities.

| Surface | What it is | Primary target | Secondary targets |
|---|---|---|---|
| **Workbench** | Structured task delegation (Linear/Notion pattern, not blank chat) | Dashboard | TUI, CLI |
| **Agent Inbox** | Ambient notification (calm technology) | Dashboard, TUI | CLI |
| **Generative Canvas** | Visual Graph editor (nodes-as-cards, typed cables, drag-and-drop) | Dashboard (Visual Editor) | TUI (state-graph view) |
| **Stigmergy Minimap** | RTS-style coordination (fog-of-war, group selection) | Dashboard | TUI |
| **Autonomy Slider** | Progressive trust (5 levels 0-4 on the Slider, per-capability granularity) | Dashboard | TUI, CLI |

Every operation that one rendering target supports, every other target supports -- possibly through different UX. The contract is the projections and events. The rendering is unconstrained.

### 1.1 Surface-to-TUI-Tab Mapping

The TUI (`roko dashboard`) has seven tabs (F1-F7). Each tab renders one or more named surfaces. Two tabs (F5 Knowledge, F6 System) do not correspond to named surfaces -- they are rendering-target-specific views that consume StateHub projections directly without defining interaction contracts of their own.

| TUI Tab | Surfaces Rendered | Projections Consumed | Notes |
|---|---|---|---|
| **F1 Workbench** | Workbench, Agent Inbox (badge) | `active_tasks`, `gate_pipeline`, `cost_meter`, `agent_vitality` | Inbox items at urgency 2-3 appear as badges; full Inbox is on F4 |
| **F2 Canvas** | Generative Canvas | `active_tasks`, `gate_pipeline` | Graph library + state-graph viewer |
| **F3 Flows** | Workbench (detail view) | `active_tasks`, `gate_pipeline`, `cost_meter` | Flow inspector -- deep-dive into individual Flows |
| **F4 Inbox** | Agent Inbox | `agent_vitality`, `cohort_health` | Full Inbox surface with quick-action keys |
| **F5 Knowledge** | *(rendering-target view)* | `knowledge_health` | Not a named surface. Reads `knowledge_health` projection from StateHub ([15-TELEMETRY](15-TELEMETRY.md) SS7). Displays tier distribution, demurrage balances, dream cycle history, lineage graph. |
| **F6 System** | Autonomy Slider | `agent_vitality`, `c_factor`, `cohort_health`, `cost_meter` | Autonomy controls plus rendering-target-specific daemon/provider/cost views that read projections directly. |
| **F7 Agents** | Stigmergy Minimap | `c_factor`, `cohort_health`, `knowledge_health`, `agent_vitality` | Fleet overview with compact minimap |

### 1.2 Surface-to-StateHub Projection Cross-Reference

Each surface consumes a subset of the 7 core StateHub projections defined in [15-TELEMETRY](15-TELEMETRY.md) SS7. This table is the canonical mapping between surfaces and telemetry.

| StateHub Projection (15-TELEMETRY) | Type | Source Lenses | Surfaces That Consume It |
|---|---|---|---|
| `cohort_health` | `CohortHealthProjection` | EfficiencyLens, ErrorLens, BudgetLens | Agent Inbox, Stigmergy Minimap |
| `active_tasks` | `ActiveTasksProjection` | QualityLens, LatencyLens | Workbench, Generative Canvas |
| `gate_pipeline` | `GatePipelineProjection` | QualityLens | Workbench, Generative Canvas |
| `cost_meter` | `CostMeterProjection` | CostLens, BudgetLens | Workbench |
| `knowledge_health` | `KnowledgeHealthProjection` | DriftLens | Stigmergy Minimap |
| `c_factor` | `CFactorProjection` | CollectiveIntelligenceLens | Stigmergy Minimap, Autonomy Slider |
| `agent_vitality` | `AgentVitalityProjection` | BudgetLens, EfficiencyLens | Agent Inbox, Autonomy Slider |

Surfaces never read raw Lens output. The projection schemas (defined in [15-TELEMETRY](15-TELEMETRY.md) SS7) are the stable API between telemetry and UX.

---

## 2. The 12 Primitive Object Types

The authoring system treats every object as a typed composition of primitives. No special-case configuration blobs. The DAW analogy: a DAW has audio, MIDI, bus, send. Every song is a composition. The authoring system works the same way.

| # | Type | What it represents |
|---|---|---|
| 1 | **Agent** | Configured runtime: domain + tool profiles + gate pipelines + model preferences + budget |
| 2 | **Extension** | Modular behavior unit: Pi-compatible (JS/TS), Roko-enhanced (JS/TS + heartbeat), Roko-native (Rust, 22 hooks, 8 layers) |
| 3 | **Connector** | External system I/O adapter: chain RPC, exchange API, MCP server, database, webhook |
| 4 | **Gate** | Verification step: shell command, Rust function, chain simulation, risk check |
| 5 | **Feed** | Continuous data stream: price feeds, block events, CI status, file changes, webhooks |
| 6 | **Recipe** | Data transform pipeline: map, filter, window, aggregate, score over feeds |
| 7 | **Plan** | Task DAG with dependencies, checkpoints, error policy, budget |
| 8 | **Scorer** | Quality evaluator: metric computation along specific dimensions |
| 9 | **Arena** | Evaluation environment: task source + scoring function + leaderboard rules |
| 10 | **Group** | Agent collective: cluster topology, coordination policy, resource sharing |
| 11 | **Knowledge** | Curated knowledge bundle: Signal collections with provenance |
| 12 | **Config** | Configuration as Signal: content-addressed, versioned, lineage-tracked |

---

## 3. Surface Contracts

Each surface is defined by three things:
1. **Projections consumed** -- typed data from StateHub that the surface reads.
2. **Events emitted** -- typed actions the surface sends back to the system.
3. **Invariants** -- behavioral contracts that all implementations must satisfy.

### 3.1 Workbench

The primary interaction surface. Delegates structured work to agents -- not a blank chat box, but a task-oriented surface modeled on Linear/Notion.

**Projections consumed:**

| Projection | StateHub Core Projection | Shape |
|---|---|---|
| Active Flows | `active_tasks` (ActiveTasksProjection) | `Vec<FlowSummary>` -- id, graph name, progress %, cost, duration, status |
| Agent Slots | `agent_vitality` (AgentVitalityProjection) | `Vec<SlotState>` -- agent id, slot index, occupied/free, current task, vitality |
| Graph Topology | GraphRegistry via StateHub | `GraphSummary` -- nodes, edges, Macros, Slots, estimated cost |
| Pending Human Input | `active_tasks` (ActiveTasksProjection) | `Vec<HumanInputRequest>` -- run id, cell id, prompt, urgency, deadline |
| Recent Completions | Store | `Vec<FlowResult>` -- last N completed Flows with verdicts |
| Gate Status | `gate_pipeline` (GatePipelineProjection) | `GatePipelineProjection` -- rung snapshots, pass rates, avg reward |
| Cost | `cost_meter` (CostMeterProjection) | `CostMeterProjection` -- total, remaining, burn rate, trend |

**Events emitted:**

| Event | Payload | Effect |
|---|---|---|
| `TaskAssign` | `{ graph, inputs, macros, slots, budget, deadline }` | Starts a new Flow |
| `SlotFill` | `{ agent_id, slot_index, cell_ref }` | Fills an Agent's Slot with a Cell |
| `MacroAdjust` | `{ run_id, macro_name, new_value }` | Adjusts a Macro on a running Flow |
| `FlowCancel` | `{ run_id }` | Cancels an active Flow |
| `FlowPause` | `{ run_id }` | Pauses an active Flow |
| `FlowResume` | `{ run_id }` | Resumes a paused Flow |
| `HumanRespond` | `{ run_id, cell_id, response }` | Answers a human-input prompt |

```rust
pub struct FlowSummary {
    pub run_id: RunId,
    pub graph_name: String,
    pub progress_pct: f64,
    pub cost_usd: f64,
    pub elapsed: Duration,
    pub status: FlowStatus,
    pub active_nodes: Vec<NodeId>,
    pub pending_human: Option<HumanInputRequest>,
}

pub enum FlowStatus {
    Running,
    Paused,
    WaitingHuman,
    Completed { verdict: Verdict },
    Failed { error: String },
    Cancelled,
}
```

**Invariants:**
- Active Flows are always visible. A Workbench that hides running work is broken.
- Pending human input is surfaced with urgency. The user must never miss a decision request.
- Slot filling supports type-checked drag-and-drop (or equivalent selection in CLI/TUI).
- Cost and duration are live, not polled. Updates arrive via Bus subscription.

### 3.2 Agent Inbox

Ambient notification surface. Calm technology -- peripheral attention until something needs focus. Modeled on notification center, not chat. Three urgency levels at three priority bands.

**Projections consumed:**

| Projection | StateHub Core Projection | Shape |
|---|---|---|
| Attention Pulses | Bus (filtered by `tagged_for_human: true`) | `Vec<InboxItem>` |
| Agent Health | `agent_vitality` (AgentVitalityProjection) | Agent vitality + phase for context |
| Cohort Overview | `cohort_health` (CohortHealthProjection) | Error rate, regime distribution for triage |

**InboxCategory enum:**

```rust
/// Categories for inbox items. Determines routing behavior:
/// - Transport strip: Question and Review items appear in the persistent
///   Transport strip at the bottom of the TUI for immediate visibility.
/// - Badge-only: Notify items increment the badge count on F1/F4 tabs
///   but do not appear in the Transport strip.
/// - Full Inbox panel: ALL categories appear in the F4 Inbox tab and
///   the Dashboard Inbox queue, sorted by urgency then recency.
pub enum InboxCategory {
    /// Gate verdict requiring human decision (approve/reject deploy).
    GateVerdict,
    /// Agent requesting clarification or input to continue.
    AgentQuestion,
    /// Budget threshold crossed (BudgetLens alert).
    BudgetAlert,
    /// Agent completed a task or Flow.
    TaskCompletion,
    /// Structural change proposed (new Graph, config modification).
    StructuralChange,
    /// Security event (capability violation, quarantine, anomaly).
    SecurityEvent,
    /// Knowledge event (dream cycle complete, heuristic falsified).
    KnowledgeEvent,
    /// System event (daemon status, provider health, deployment).
    SystemEvent,
}
```

**Inbox routing by urgency and category:**

| InboxCategory | Typical Urgency | Transport Strip | Badge (F1/F4) | Full Inbox Panel |
|---|---|---|---|---|
| `GateVerdict` | Review (L3) | Yes | Yes (red dot) | Yes |
| `AgentQuestion` | Question (L2) | Yes | Yes | Yes |
| `BudgetAlert` | Question (L2) | Yes | Yes | Yes |
| `TaskCompletion` | Notify (L1) | No | Yes (count) | Yes |
| `StructuralChange` | Review (L3) | Yes | Yes (red dot) | Yes |
| `SecurityEvent` | Review (L3) | Yes | Yes (red dot) | Yes |
| `KnowledgeEvent` | Notify (L1) | No | Yes (count) | Yes |
| `SystemEvent` | Notify (L1) | No | Yes (count) | Yes |

**Urgency levels:**

| Level | Name | Behavior | Example |
|---|---|---|---|
| 1 | **Notify** | Badge count. No interruption. | "Agent completed code review." |
| 2 | **Question** | Gentle chime. Requires answer within deadline. | "Agent found 3 security issues. Deploy anyway?" |
| 3 | **Review** | Persistent banner. Blocks progress until resolved. | "Structural change proposed: new Graph. Approve?" |

```rust
pub struct InboxItem {
    pub id: PulseId,
    pub urgency: UrgencyLevel,
    pub category: InboxCategory,
    pub summary: String,
    pub detail: Value,
    pub source_agent: Option<AgentId>,
    pub timestamp: DateTime<Utc>,
    pub deadline: Option<DateTime<Utc>>,   // for Level 2 Questions
    pub blocking_run: Option<RunId>,       // for Level 3 Reviews
}

pub enum UrgencyLevel {
    Notify,     // badge only
    Question,   // chime + deadline
    Review,     // persistent, blocks progress
}
```

**Events emitted:**

| Event | Payload | Effect |
|---|---|---|
| `Approve` | `{ pulse_id }` | Approves a Question or Review item |
| `Reject` | `{ pulse_id, reason }` | Rejects with reason |
| `Defer` | `{ pulse_id, defer_until }` | Defers decision to a later time |
| `Dismiss` | `{ pulse_id }` | Dismisses a Notify item |

**Invariants:**
- Level 3 (Review) items block relevant Flows until resolved. The surface must make them unmissable.
- Level 2 (Question) items have deadlines. The surface must show countdown.
- Level 1 (Notify) items do not interrupt focus. Badge count only.
- Items expire naturally via demurrage ([01-SIGNAL](01-SIGNAL.md)). Old unresolved items fade.

### 3.3 Generative Canvas

Visual Graph editor. Nodes as cards, typed cables, drag-and-drop composition. The authoring surface for Graphs, Racks, Triggers, and Profiles.

**Design lineage**: Ableton Live (session view, macro knobs), Bitwig Grid (modular patching, typed cables), n8n (node-based automation, visual data flow).

**Projections consumed:**

| Projection | StateHub Core Projection | Shape |
|---|---|---|
| Graph TOML | GraphRegistry | `GraphDefinition` -- full TOML-parsed Graph |
| Cell Catalog | CellRegistry via StateHub | `Vec<CellManifest>` -- all available Cells with I/O schemas |
| TypeSchema Registry | TypeRegistry | `Vec<TypeSchema>` -- all known types for edge compatibility |
| Live Flow State | `active_tasks` (ActiveTasksProjection) | `FlowState` -- node statuses, edge traversals, for live overlay |
| Gate Overlay | `gate_pipeline` (GatePipelineProjection) | Per-node pass/fail indicators from gate pipeline |

**Events emitted:**

| Event | Payload | Effect |
|---|---|---|
| `NodeAdd` | `{ cell_ref, position }` | Adds a Cell node to the Graph |
| `NodeRemove` | `{ node_id }` | Removes a node |
| `EdgeCreate` | `{ from_node, from_port, to_node, to_port }` | Wires a typed edge |
| `EdgeRemove` | `{ edge_id }` | Removes an edge |
| `MacroPromote` | `{ node_id, param_name, macro_def }` | Promotes a Cell parameter to a Rack Macro |
| `SlotDeclare` | `{ slot_name, type_constraint, default }` | Declares a new Slot |
| `GraphSave` | `{ graph_toml }` | Persists the Graph |
| `GraphPublish` | `{ graph_name, marketplace_metadata }` | Publishes to marketplace ([21-MARKETPLACE](21-MARKETPLACE.md)) |

**Invariants:**
- Edge type compatibility is checked continuously. Mismatches render inline (red cable), not modal error.
- Every mouse interaction has a keyboard equivalent.
- Three views of the same data: Recipe view (linear), Graph view (DAG), Timeline view (Gantt).
- The Canvas operates on the same TOML format as the CLI. Round-trip: edit in Canvas, save, load in CLI, modify, load back in Canvas -- zero data loss.

### 3.4 Stigmergy Minimap

RTS-style coordination visualization. Shows the agent population as a spatial field with fog-of-war for unknown regions and group-selection for batch operations.

**Projections consumed:**

| Projection | StateHub Core Projection | Shape |
|---|---|---|
| Pheromone Field | Bus (pheromone Pulses) | `PheromoneGrid` -- 2D field of signal intensities |
| Agent Positions | `agent_vitality` (AgentVitalityProjection) | `Vec<AgentPosition>` -- id, spatial embedding, status, vitality |
| c-factor Scores | `c_factor` (CFactorProjection) | `CFactorSummary` -- turn-taking entropy, peer prediction accuracy, diversity |
| Cluster Membership | `cohort_health` (CohortHealthProjection) | `Vec<ClusterState>` -- which agents form which coalitions |
| Knowledge Landscape | `knowledge_health` (KnowledgeHealthProjection) | Tier distribution, cold entries -- fog density correlates with knowledge gaps |

```rust
pub struct AgentPosition {
    pub id: AgentId,
    pub x: f64,                        // spatial embedding (2D projection of HDC space)
    pub y: f64,
    pub status: AgentStatus,
    pub vitality: f64,                 // 0.0..=1.0
    pub profile: String,
    pub current_task: Option<String>,
}

pub struct CFactorSummary {
    pub overall: f64,                  // composite c-factor
    pub turn_taking_entropy: f64,
    pub peer_prediction_accuracy: f64,
    pub citation_reciprocity: f64,
    pub hdc_diversity: f64,
}
```

**Events emitted:**

| Event | Payload | Effect |
|---|---|---|
| `SpawnAgent` | `{ profile, position_hint }` | Spawns a new Agent |
| `CullAgent` | `{ agent_id }` | Gracefully terminates an Agent |
| `GroupSelect` | `{ agent_ids }` | Selects a group for batch operations |
| `TopologyAdjust` | `{ cluster_id, new_topology }` | Reconfigures cluster shape |
| `PheromoneDeposit` | `{ position, kind, intensity }` | Manual pheromone signal (human-guided coordination) |

**Invariants:**
- The minimap updates in real time (Bus subscription, not polling).
- Agent vitality is visible (color saturation maps to vitality level).
- Fog-of-war covers regions where no agent has explored. Exploration lifts fog.
- Group selection enables batch operations (send all selected agents the same task, adjust all budgets).

### 3.5 Autonomy Slider

Progressive trust control. The Slider exposes five autonomy levels (0-4) with per-capability granularity. Level 5 (structural evolution) is defined in [16-SECURITY](16-SECURITY.md) SS11 but is not part of the Slider -- structural evolution requires a separate L4 approval flow and cannot be granted via the Slider UI.

**Projections consumed:**

| Projection | StateHub Core Projection | Shape |
|---|---|---|
| Agent Capabilities | AgentRuntime via StateHub | `Vec<CapabilityDeclaration>` |
| Current Autonomy Levels | `agent_vitality` (AgentVitalityProjection) | `AutonomyConfig` |
| CaMeL Tags | Extension system via StateHub | `Vec<CamelTag>` |
| Safety Violations | SecurityEventStream | `Vec<SafetyViolation>` |
| Collective Intelligence | `c_factor` (CFactorProjection) | c-factor trend informs trust calibration |

**Slider levels (0-4):**

| Level | Name | System behavior | Human involvement |
|---|---|---|---|
| 0 | **Observe** | Read-only. No mutations. | None needed |
| 1 | **Suggest** | Proposes actions as Signals. Does not execute. | Approves each action |
| 2 | **Act-with-review** | Executes actions. Human reviews before persist. | Post-action review |
| 3 | **Act-with-guardrails** | Executes within declared parameter ranges. | Review on bound violations |
| 4 | **Full autonomy** | Full execution within capability grant. Escalates novel situations. | Review on escalation only |

**Level 5 (Structural Evolution)** is defined in [16-SECURITY](16-SECURITY.md) SS11 and operates outside the Slider. An agent at L4 may *propose* structural changes (Graph modifications, Cell additions, config evolution), but these proposals are routed through the L4 approval flow: the proposal appears as a Review-urgency InboxItem (SS3.2), the human approves or rejects, and only then does the structural change take effect. The Slider never shows or allows setting Level 5.

**Per-capability granularity:** the slider is not a single global knob. Each capability has its own level:

```rust
pub struct AutonomyConfig {
    pub agent_id: AgentId,
    /// Per-capability autonomy. Each capability maps to a Slider level (0-4).
    pub per_capability: HashMap<Capability, AutonomyLevel>,
    /// Default level for capabilities not explicitly configured.
    pub default_level: AutonomyLevel,
}

pub enum AutonomyLevel {
    Observe    = 0,
    Suggest    = 1,
    ActReview  = 2,
    Guardrails = 3,
    Full       = 4,
    // Level 5 (StructuralEvolution) is NOT representable here.
    // It follows the L4 approval flow in 16-SECURITY SS11.
}
```

A user might set `FsRead = 4` (full autonomy) and `Chain.write = 1` (suggest only).

**Events emitted:**

| Event | Payload | Effect |
|---|---|---|
| `AutonomyLevelChange` | `{ agent_id, capability, new_level }` | Changes autonomy for a capability (0-4 only) |
| `CapabilityGrant` | `{ agent_id, capability, constraints }` | Grants a new capability |
| `CapabilityRevoke` | `{ agent_id, capability }` | Revokes a capability |
| `BulkAutonomySet` | `{ agent_id, level }` | Sets all capabilities to same level (0-4 only) |

**Invariants:**
- Autonomy can only be increased by the user, never by the system.
- Reducing autonomy takes effect immediately. In-flight operations complete at old level; new operations use new level.
- The Slider range is 0-4. Level 5 (structural evolution) requires the separate L4 approval flow defined in [16-SECURITY](16-SECURITY.md) SS11. Structural change proposals arrive as Review-urgency InboxItems, not as Slider positions.
- Recent safety violations are visible next to the slider to inform trust calibration.

---

## 4. Rendering Targets

Four rendering targets implement the five surfaces.

### 4.1 CLI Surface

The `roko` CLI is Graph-centric: every meaningful operation is a Graph run or a registry operation.

#### Workbench commands

```
roko run <graph> [args]              # run a Graph (TaskAssign)
roko run cancel <run-id>              # cancel (FlowCancel)
roko run respond <run-id> [args]      # answer human-input (HumanRespond)
roko run resume <run-id>              # resume snapshotted Flow (FlowResume)
roko run list [--status]              # active and recent Flows
roko run show <run-id>                # detailed Flow inspection
roko run logs <run-id> [--follow]     # stream Flow logs
roko run replay <run-id>              # rerun with same inputs
```

#### Canvas commands

```
roko graph list [--installed | --catalog]
roko graph show <name>
roko graph validate <name>
roko graph new <name>
roko graph edit <name>
roko graph fork <source> <new>
```

#### Inbox commands

```
roko inbox list [--urgency <level>] [--category <cat>]
roko inbox show <id>
roko inbox approve <id>
roko inbox reject <id> [--reason <text>]
roko inbox dismiss <id>
```

#### Autonomy commands

```
roko autonomy show [--agent <name>]
roko autonomy set <agent> <capability> <level>    # level: 0-4
roko autonomy set <agent> --all <level>            # level: 0-4
```

#### Verb sugar

Common Graph runs get top-level aliases. Every verb expands to `roko run <graph>`:

```
roko ingest <dir>           = roko run doc-ingest --input source_dir=<dir>
roko deploy [target]        = roko run deploy --macro target=<target>
roko research <topic>       = roko run research-sweep --input topic=<topic>
roko review [pr|diff]       = roko run code-review --input ...
roko audit                  = roko run security-audit
roko test [scope]           = roko run test-run --input target=<scope>
```

#### Exit codes

| Code | Meaning |
|---|---|
| 0 | Flow succeeded |
| 1 | Generic failure |
| 2 | Bad CLI usage |
| 10 | Graph validation failed |
| 11 | Capability denied |
| 12 | Budget exceeded |
| 13 | Deadline exceeded |
| 14 | Human input timeout |
| 15 | Cancelled |
| 16 | Space error |

### 4.2 TUI Surface

The TUI (`roko dashboard`) is a ratatui-based terminal application. Keyboard-driven, no server required. Seven tabs plus a persistent transport strip.

#### Layout

```
 roko - Space: nunchi-dashboard - providers: ok - daemon: ok - cost today: $4.12

  [F1 Workbench] [F2 Canvas] [F3 Flows] [F4 Inbox] [F5 Knowledge] [F6 System] [F7 Agents]

  +------------------------------------------------------------------------+
  |                                                                        |
  |                          (active tab content)                          |
  |                                                                        |
  +------------------------------------------------------------------------+

  Transport:
  * doc-ingest   53%  $1.84/$10  4m 12s  [pause] [cancel] [detach]
  | deploy-rc   pending human input  [respond]

  [?] help  [/] palette  [g] go to  [w] space  [c] create  [q] quit
```

The bottom **Transport** strip is always visible: shows up to 3 active Flows with one-press controls (pause, cancel, detach, respond-to-human-input). The Workbench surface in its most compact form. Urgency 2-3 Inbox items also appear here (see SS3.2 Inbox routing table).

#### Tabs

**F1 Workbench** -- Task delegation and Flow overview. Renders the **Workbench** surface plus **Agent Inbox** badges for urgency 2-3. Shows Space health, active Flows, pending Triggers, recent completions, cost summary. Auto-refreshes every 1s.

**F2 Canvas** -- Graph library and state-graph viewer. Renders the **Generative Canvas** surface. Two-pane: Graph list (left, 30%), detail + inline edit (right, 70%). Shows Macros, Slots, capabilities, estimated cost, last 5 runs. Press `r` to launch. Press `->` for Graph View (state-graph via ratatui-canvas).

**F3 Flows** -- Flow inspector. Renders a detail view of the **Workbench** surface. List of active and recent Flows (left), detail (right). Sub-views: Overview, Graph (live node colors), Artifacts, Episodes, Logs, Trace (timing waterfall).

**F4 Inbox** -- Renders the full **Agent Inbox** surface. Items sorted by urgency then recency. Quick-action keys: `a` approve, `r` reject, `d` defer, `x` dismiss. Shows urgency level, category, source agent, summary, deadline countdown.

**F5 Knowledge** -- Knowledge browser. Rendering-target-specific view consuming the `knowledge_health` StateHub projection. Entries with type, confidence, age, decay state. Resonance graph. Lineage walker. Dream cycle history.

**F6 System** -- Space + Daemon + Providers + Costs + **Autonomy Slider** surface. Space details, daemon status, provider health, cost breakdown. Per-agent autonomy levels (0-4) with inline adjustment.

**F7 Agents** -- Fleet overview with compact **Stigmergy Minimap** surface. Agent cards with status, vitality, current task, cost rate. Minimap shows spatial embedding and c-factor components.

#### Universal keys

```
?            help overlay
/            command palette (fuzzy across Graphs, Flows, artifacts, pages)
g <letter>   go to: gw workbench, gc canvas, gf flows, gi inbox, gk knowledge, gs system
w            Space switcher (overlay)
c            create new (overlay: graph, trigger, rack, space)
.            quick run launcher (last-used Graph, edit args, run)
:            command line (typed commands)
q            quit
^c           cancel current focus action
F1-F7        jump to tab
Tab / S-Tab  cycle panes within active tab
```

#### State-graph view

ratatui-canvas-rendered graph with live node colors:
- **Jade** -- node completed
- **Cyan flashing** -- node in flight
- **Amber** -- node queued
- **Crimson** -- node failed
- **Violet** -- node awaiting human input
- **Dimmed** -- node pruned by failed conditional

#### File watcher integration

The existing `notify::RecommendedWatcher` (`tui/fs_watch.rs`) watches `.roko/runs/`, `episodes.jsonl`, and `artifacts/`. Triggers re-render on change -- reactive to daemon progress without polling.

### 4.3 Dashboard (Web)

The web dashboard (`roko serve` on :6677) is the primary visual surface. All five surfaces present.

#### Navigation

| Destination | Surfaces | Key pages |
|---|---|---|
| **Workbench** | Workbench + Agent Inbox | Task board, Event Stream, Inbox queue |
| **Agents** | Stigmergy Minimap + Autonomy Slider | Fleet minimap, Detail, Create, Groups, Autonomy |
| **Work** | Generative Canvas + Workbench | Library, Editor, Flows, Triggers, Marketplace |
| **Knowledge** | -- | Store, Resonance, Lineage, Dreams |
| **Arena** | -- | Browser, Leaderboard, Experiments |
| **System** | Autonomy Slider | Spaces, Providers, Costs, Deployments, Settings |

#### Real-time plumbing

- **WebSocket `/ws/events`** -- every Cell lifecycle event for the active Space
- **WebSocket `/ws/runs/<run-id>`** -- focused stream for one Flow
- **SSE `/sse/triggers`** -- Trigger fire and dispatch events
- **SSE `/sse/cost`** -- live cost ticks
- **SSE `/sse/inbox`** -- Inbox items for Agent Inbox surface
- **HTTP REST** -- `/api/v1/graphs`, `/runs`, `/triggers`, `/spaces`, `/artifacts`, `/episodes`

Every page is fully reactive: Flow list updates without refresh, Graph view animates state transitions, cost gauge ticks live.

#### Data layer architecture

Three components manage the flow from WebSocket events to rendered pixels:

**SubscriptionManager**: Multiplexes connections to agent, chain, relay, and workspace event streams. Each page declares subscriptions on mount and releases on unmount. Single WebSocket to relay and single WebSocket to roko-serve, using room-based subscription messages.

**EventAggregator**: Batches burst events with a 100ms flush window. Ring buffer (200 events) supports replay for late-mounting components.

**RenderScheduler**: DOM updates coalesced in rAF callbacks. Canvas/WebGL at 60fps on separate requestAnimationFrame loop.

#### Adaptive information density

The dashboard adjusts based on the system's `CorticalState`:

| Regime | Trigger | What changes |
|---|---|---|
| **Cruise** | All agents calm, no active plans, PE < 0.15 | Minimal display. Aggregated metrics. Agent cards collapsed. |
| **Volatile** | 1+ agents in T2, active gate failures, PE 0.15-0.40 | Affected agents expand. Event stream highlights anomalies. |
| **Crisis** | Multiple gate failures, agent errors, PE > 0.40 | Full traces visible. Remediation suggestions inline. All agents expand. |

### 4.4 Visual Editor (Generative Canvas)

The dashboard's drag-and-drop authoring environment. The full Generative Canvas surface.

#### Three-column layout

- **Palette** (left, ~240px): draggable Cells, control-flow primitives (Branches, Loops, FanOut, FanIn, HumanInput), Macros, Slots, Triggers, Snippets, Criteria
- **Canvas** (center): the state graph -- the playground
- **Inspector** (right, ~360px, collapsible): properties of selected node / edge / Macro / Slot

#### Node cards

Each node is a card with:
- Header strip with Cell name + version + status badge
- Input port row (top edge, one per typed input)
- Body: collapsed compact info, expandable for full param list
- Output port row (bottom edge, one per typed output)
- Footer with cost/time estimates and capability badges
- Quick actions on hover: pin, fork-this-step, replace-cell, mute (skip), solo (run only this)

Node colors: **Sapphire** (Cell), **Violet** (Sub-Graph), **Rose** (HumanInput/Wait), **Amber** (Branch/FanOut/FanIn/Loop), **Slate** (Slot placeholder), **Glass** (pruned-by-condition).

#### Cable colors

Cable colors encode payload type: rose (doc), jade (code), sapphire (structured data), amber (evidence), violet (knowledge), white (generic). Cable thickness encodes batch size.

Cable behavior:
- Hover: last value preview in tooltip
- Click in-flight: stream viewer of flowing values
- Drag output-to-input: creates cable. Type match = instant. Mismatch = adapter prompt from registry.

#### Macro promotion

The signature gesture: right-click an internal Cell parameter -> "Promote to Macro". Inline modal for name, label, kind, range, default. The parameter value is replaced with `{{ macro.name }}` and a MacroDef is added to the Rack.

Macro controls render as: floats/money as rotary knob, integers as stepper, booleans as toggle, enums as segmented control, strings as text input, ModelRef as searchable dropdown, AgentRef as avatar dropdown.

#### Live preview

- **Last-value preview**: after any run, hovering a node shows its last output (markdown rendered, JSON pretty-printed, images thumbnailed, diffs highlighted)
- **Test-isolated**: right-click any node -> "Test in isolation". Runs only that Cell in a sandbox.
- **Run-up-to-here**: right-click any node -> "Run up to here". Engine runs partial Graph, halts.

#### Multiple views

Every Graph has three views, toggled by keyboard or button:
- **Recipe view** (linear, Apple-Shortcuts-like step list) -- beginner-friendly
- **Graph view** (DAG canvas) -- power authoring
- **Timeline view** (Gantt waterfall of last run) -- parallelism and bottleneck visualization

#### Sound (Web Audio)

Generated tones, off by default:
1. Cable plug-in -- perfect fifth, 60ms triangle
2. Cable mismatch -- minor second, 80ms
3. Validation success -- short major chord arpeggio
4. Validation failure -- low resonant tone, 200ms
5. Flow start -- octave, 120ms with reverb tail
6. Flow complete -- high sine ping, 60ms

Principle: juice the user's actions and the agent's completions, never ambient state.

#### Motion (spring physics)

- Node drag: stiffness 500, damping 30
- Node insert: stiffness 200, damping 28, scale 0.9 -> 1.0
- Cable connect: 200ms ease, brief flash
- Validation error: shake with limited amplitude
- Macro promotion: source-param card morphs into Macro card via FLIP
- Recipe <-> Graph toggle: 400ms crossfade with positional FLIP
- Flow state changes: edges flash rose during traversal, nodes pulse cyan in flight
- All animations respect `prefers-reduced-motion`

#### Keyboard shortcuts

```
Cmd+K               command palette (filtered to authoring actions)
Cmd+/               search palette of Cells to add
Cmd+E               toggle Recipe / Graph view
Cmd+R               run the Graph
Cmd+Shift+R         test-isolated the selected node
Cmd+G               group selection into sub-Graph
Cmd+Shift+G         ungroup sub-Graph
Cmd+D               duplicate selected node(s)
Cmd+Z / Shift+Cmd+Z undo / redo
Cmd+S               save
Cmd+Shift+P         publish to marketplace
Cmd+M               promote selected param to Macro
Tab                 cycle selection through outgoing edges
Arrow keys          move selected node
Esc                 deselect / collapse modal
.                   focus palette search
[ / ]               collapse / expand inspector
```

---

## 5. Conversation-as-Plan-Editor

The plan mutation protocol enables chat-driven plan creation. The user talks to an agent in a floating chat drawer; the agent generates structured mutations applied to the plan canvas.

### 5.1 Mutation types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PlanMutation {
    AddTask { task: TaskSpec, after: Option<TaskId> },
    RemoveTask { id: TaskId },
    UpdateTask { id: TaskId, patch: TaskPatch },
    AddDependency { from: TaskId, to: TaskId },
    RemoveDependency { from: TaskId, to: TaskId },
    Reorder { task_ids: Vec<TaskId> },
    SetParallel { task_ids: Vec<TaskId> },
    AddCheckpoint { after: TaskId, name: String },
    UpdatePlanMeta { patch: PlanMetaPatch },
}
```

### 5.2 Chat endpoint

```
POST /api/plans/{id}/chat
{ "message": "...", "context": { "selected_tasks": [...], "viewport": "lane_view" } }
```

Response includes `reply` (natural language), `mutations` (structured changes), `rejected` (with reasons), `plan_state`, and `cost_estimate`.

### 5.3 Mutation validation rules

1. `AddTask` with a duplicate `id` is rejected.
2. `RemoveTask` for a non-existent `id` is rejected.
3. `AddDependency` that would create a cycle is rejected (topological sort check).
4. `SetParallel` tasks must share at least one common predecessor.
5. Rejected mutations return in a `rejected` array. Valid mutations in the same batch still apply.

### 5.4 Three visual abstraction levels

- **Card stack**: vertical list, drag to reorder. Best for linear pipelines, mobile.
- **Lane view**: parallel tasks in side-by-side lanes. Best for 2-4 branches.
- **Node graph**: full DAG with directed edges. Best for complex multi-branch plans.

The backend always returns the full `PlanSpec`. The view is a client-side preference.

---

## 6. Cross-Surface Linking

Surfaces link to each other. The CLI prints clickable URLs (OSC 8 hyperlinks) and `roko://` references:

```
$ roko run doc-ingest ...
Run id: wf_01HGZK7B...
Dashboard: http://localhost:6677/runs/wf_01HGZK7B...
TUI: roko tui --run wf_01HGZK7B...
```

`roko tui --run <id>` jumps directly to the Flow inspector. Dashboard URLs deep-link to specific Flows, Graphs, and Triggers. All rendering targets share the same run-id and entity-id namespace.

---

## 7. Third-Party Surfaces

Third parties build new surfaces by consuming StateHub projections and emitting surface events. The five named surfaces define the contracts; implementations are open.

Examples:
- **Mobile app**: renders Workbench + Agent Inbox for on-the-go task delegation.
- **Slack bot**: renders Agent Inbox into Slack messages with reaction-based approve/reject.
- **VSCode extension**: renders Generative Canvas as a panel + Workbench as a sidebar.
- **CLI dashboard (tmux)**: renders all five surfaces in tmux panes with curses widgets.
- **Custom monitoring**: renders only CostLens and BudgetLens projections for finance teams.

---

## 8. Theming

### TUI

ROSEDUST palette adapted for terminal: rose accent on active elements, jade for success, amber for warning, crimson for error, violet for knowledge, sapphire for active Cells. Monospace throughout. User-toggleable density (compact / comfortable / spacious) via `~/.roko/config.toml`.

### Dashboard

Glass morphism on panels (3 levels). Spring-physics motion. Same ROSEDUST palette. Tabular nums. Stagger-children at 40ms on list mounts. State graph node colors match the TUI scheme, edge animations (traversed = rose flash, pruned = ghost fade), Macro sliders as DAW-style controls.

### Epistemic aesthetics

| Visual property | Data source | Encoding |
|---|---|---|
| Glow intensity | Epistemic confidence (gate pass rate, neuro store match quality) | Brighter = higher confidence |
| Fade / decay | Knowledge staleness (demurrage balance) | Faded entries need re-validation |
| Turbulence | Contested knowledge entries (challenged in neuro store) | Shimmer/jitter indicates active dispute |
| Velocity streaks | Active agent output (tokens/sec) | Faster streaks = higher throughput |
| Heartbeat pulse | Per-agent tick cadence (gamma/theta/delta) | Visible rhythm matches agent's clock |
| Saturation | Validation strength (gate rung depth) | Deeper validation = richer color |

---

## 9. Authoring API Contracts

Each of the 12 authoring surfaces follows a consistent REST pattern.

### CRUD

```
POST   /api/{object_type}              -- create (from template or blank)
GET    /api/{object_type}              -- list (with pagination, filtering)
GET    /api/{object_type}/{id}         -- read (full detail)
PUT    /api/{object_type}/{id}         -- update (full replacement)
PATCH  /api/{object_type}/{id}         -- partial update
DELETE /api/{object_type}/{id}         -- delete (soft delete for deployed)
```

### Validation

```
POST /api/{object_type}/{id}/validate
```

Three severity levels: **error** (blocks deploy), **warning** (flags risk), **suggestion** (advisory).

### Deploy

```
POST /api/{object_type}/{id}/deploy
{ "target": "local", "register_on_chain": false }
```

Deploy transitions the object from draft to live. The `register_on_chain` flag triggers ERC-8004 registration.

### Publish as template

```
POST /api/{object_type}/{id}/publish
{ "template_name": "...", "description": "...", "tags": [...], "visibility": "community" }
```

---

## 10. Acceptance Criteria

| Criterion | Verification |
|---|---|
| **Surface contracts**: Workbench, Inbox, Canvas, Minimap, Slider each have typed projections and events | Schema validation test on each surface contract |
| **Projection cross-ref**: every surface projection maps to a named StateHub core projection from 15-TELEMETRY | Cross-reference validation against 15-TELEMETRY SS7 |
| **InboxCategory**: all 8 categories route correctly per urgency/routing table | Category routing test |
| **Third-party surface**: external consumer subscribes to StateHub projections and emits surface events | Integration test: mock surface consumes projections, emits events |
| **CLI**: `roko run <graph>` runs in foreground with live progress (Workbench) | Manual test on doc-ingest |
| **CLI**: `roko inbox list` shows pending items (Agent Inbox) | Inbox population test |
| **CLI**: `roko autonomy set` changes levels 0-4 (Autonomy Slider) | Autonomy change test |
| **CLI**: `roko autonomy set` rejects level 5 | Negative test: slider refuses structural evolution |
| **CLI**: `roko --json` produces valid JSON for all commands | Schema tests |
| **CLI**: Tab completion enumerates Graphs, Cells, Spaces, run-ids | Manual test in zsh + fish |
| **CLI**: Verb sugar maps correctly to Graph runs | Equivalence test |
| **TUI**: Launches via `roko dashboard`, renders F1 Workbench | Smoke test |
| **TUI**: F1 auto-refreshes every 1s without flicker | Visual / load test |
| **TUI**: F3 streams live Flow output within 100ms | Latency test |
| **TUI**: F4 Inbox shows items with urgency levels, categories, and quick-action keys | Inbox rendering test |
| **TUI**: F5 Knowledge consumes `knowledge_health` projection | Projection subscription test |
| **TUI**: F6 System renders Autonomy Slider (0-4 range) | Slider range test |
| **TUI**: State-graph view renders with live node colors | Visual snapshot |
| **TUI**: Space switcher changes context; tab content updates | Multi-Space test |
| **TUI**: Surface-to-tab mapping matches SS1.1 table | Cross-reference test |
| **Dashboard**: Flow inspector receives WebSocket updates within 200ms | Latency test |
| **Dashboard**: State-graph node colors update live during Flow | Visual regression |
| **Dashboard**: Stigmergy Minimap renders agent positions and pheromone field | Minimap rendering test |
| **Dashboard**: Autonomy Slider per-capability controls (0-4) update agent config | Slider round-trip test |
| **Dashboard**: Inbox badge shows count with red dot for Level 3 | Badge rendering test |
| **Visual Editor**: Drag Cell from palette -> node appears with correct ports | Manual + e2e |
| **Visual Editor**: Promote-to-Macro replaces param and adds MacroDef | Round-trip test |
| **Visual Editor**: Recipe <-> Graph toggle preserves all data | Round-trip test |
| **Visual Editor**: Test-isolated runs selected Cell within 30s | Latency test |
| **Visual Editor**: Validation errors render inline; bottom strip aggregates | Synthetic invalid Graph |
| **Visual Editor**: Three views (Recipe, Graph, Timeline) all render same data | View equivalence test |
| **Plan mutations**: Chat endpoint returns valid mutations applied to canvas | Round-trip test |
| **Plan mutations**: Cycle detection rejects cyclic dependencies | Negative test |
| **Cross-surface**: CLI prints dashboard URL; URL deep-links to correct Flow | Link test |
| **Autonomy Slider**: User increases trust, system never autonomously increases | Invariant test |
| **Autonomy Slider**: Reducing autonomy takes effect on next operation | Mid-flow level change test |
| **Autonomy Slider**: Level 5 not settable via Slider; requires L4 approval flow | Structural evolution separation test |
| **Authoring API**: All 12 object types follow CRUD + validate + deploy pattern | Per-type round-trip test |
