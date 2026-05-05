# Integration Topology: The System as a Typed Directed Graph

> Depth for [00-INDEX.md](../../unified/00-INDEX.md), the cross-section integration map, and the synergy integration map. This doc models the entire system as a typed directed graph where nodes are Cells and edges are Signal/Pulse flows, identifies strongly-connected components, disconnected subgraphs, and bottleneck edges, and adds Lenses that observe integration health.

---

## 1. The System Graph

Every Roko subsystem is a Cell. Every data flow between subsystems is a typed edge carrying Signals (durable) or Pulses (ephemeral). The entire architecture is therefore a directed graph `G = (V, E)` where:

- `V` = the set of subsystem Cells
- `E` = typed, directed edges labeled with Signal kind or Pulse topic

This is not a metaphor. The Graph is the actual runtime structure. See [03-GRAPH.md](../../unified/03-GRAPH.md) for the Graph specification.

### 1.1 Node inventory

We identify 22 primary nodes corresponding to the documentation sections, but the important analytical units are the *strongly connected components* -- groups of nodes where every node can reach every other node through directed edges.

```rust
// The 22 nodes, annotated with layer and protocol
enum SystemNode {
    // L0 Runtime
    Store,              // Store protocol: Signal persistence
    Bus,                // Bus: Pulse transport
    Hdc,                // HDC fingerprinting

    // L1 Framework
    AgentConnect,       // Connect protocol: LLM backends, MCP
    ModelRoute,         // Route protocol: CascadeRouter, LinUCBRouter
    ToolDispatch,       // Connect protocol: tool execution

    // L2 Scaffold
    ContextCompose,     // Compose protocol: SystemPromptBuilder
    ContextScore,       // Score protocol: relevance, recency scoring

    // L3 Harness
    GateVerify,         // Verify protocol: 14 gates, 7-rung pipeline
    ConductorReact,     // React protocol: circuit breaker, 10 watchers
    Observe,            // Observe protocol: Lenses, StateHub projections

    // L4 Orchestration
    PlanReact,          // React protocol: plan state machine, DAG executor
    TriggerFire,        // Trigger protocol: CLI commands, event-driven firing

    // Cross-cuts
    NeuroMemory,        // Store + Score: knowledge, tier progression
    DaimonAffect,       // Score + React: PAD, behavioral states
    DreamConsolidate,   // React: offline consolidation
    LearnRecord,        // Score + React: episodes, playbooks, bandits

    // Domain + Interface
    ChainConnect,       // Connect protocol: EVM, chain witness
    CodeIndex,          // Score: symbol graph, HDC fingerprints
    CliInterface,       // Trigger: user commands
    ServeInterface,     // Trigger: HTTP API, WebSocket
    TuiInterface,       // Observe: ratatui dashboard
}
```

### 1.2 Edge inventory

Each edge has a type (Signal kind or Pulse topic), a direction, and a status (wired, partial, or missing).

```rust
struct SystemEdge {
    from: SystemNode,
    to: SystemNode,
    medium: Medium,      // Signal (durable) or Pulse (ephemeral)
    label: String,       // Signal kind or Pulse topic
    status: EdgeStatus,  // Wired, Partial, Missing, TargetState
}

enum Medium { Signal, Pulse }
enum EdgeStatus { Wired, Partial, Missing, TargetState }
```

---

## 2. The Wired Subgraph

The currently-wired data flows form the operational core. This is the subgraph where `status == Wired`:

```
PlanReact ──[Kind::Task]──> AgentConnect
AgentConnect ──[Kind::AgentOutput]──> GateVerify
GateVerify ──[Kind::GateVerdict]──> PlanReact
GateVerify ──[Kind::GateVerdict + efficiency]──> LearnRecord
LearnRecord ──[cascade routing]──> ModelRoute
ModelRoute ──[model selection]──> AgentConnect
PlanReact ──[role spec]──> ContextCompose
ContextCompose ──[system prompt]──> AgentConnect
AgentConnect ──[tool calls]──> ToolDispatch
PlanReact ──[system load]──> ConductorReact
ConductorReact ──[circuit state]──> PlanReact
CliInterface ──[commands]──> PlanReact
LearnRecord ──[episodes]──> CliInterface
DaimonAffect ──[PAD vector]──> ModelRoute        (wired)
DaimonAffect ──[affect bias]──> ContextCompose   (wired)
DaimonAffect ──[behavioral state]──> PlanReact   (wired)
NeuroMemory ──[knowledge entries]──> ContextCompose  (partial)
```

### 2.1 Strongly connected components in the wired subgraph

**SCC-1: The core execution loop** (5 nodes):
```
PlanReact ↔ AgentConnect ↔ GateVerify ↔ LearnRecord ↔ ModelRoute
```
Every node can reach every other node. This is the autocatalytic core:
- Plans dispatch agents
- Agents produce outputs
- Gates verify outputs, producing verdicts
- Verdicts feed learning, producing routing updates
- Routing updates improve agent selection, improving outputs

**SCC-2: The observation loop** (3 nodes):
```
ConductorReact ↔ PlanReact ↔ CliInterface
```
The conductor watches plan health, the plan publishes status, the CLI displays it.

**SCC-3: The affect-context loop** (3 nodes):
```
DaimonAffect → ContextCompose → AgentConnect → (verdicts reach DaimonAffect via LearnRecord)
```
This is *almost* strongly connected but the DaimonAffect ← GateVerify edge is **partial** (wired for PAD updates but not for all gate event types). Completing this edge would close the affect learning loop.

### 2.2 The adjacency matrix (wired edges only)

```
             Store Bus Hdc AgCon Route Tool  Comp Score Gate Cond  Obs  Plan Trig Neuro Daim Dream Learn Chain Code CLI  Serve TUI
Store         -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
Bus           -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
Hdc           -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
AgentConnect  -    -   -   -     -     W     -    -     W    -     -    -    -    -     -    -     -     -     -    -    -     -
ModelRoute    -    -   -   W     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
ToolDispatch  -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
Compose       -    -   -   W     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
Score         -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
GateVerify    -    -   -   -     -     -     -    -     -    -     -    W    -    -     -    -     W     -     -    -    -     -
ConductorR    -    -   -   -     -     -     -    -     -    -     -    W    -    -     -    -     -     -     -    -    -     -
Observe       -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
PlanReact     -    -   -   W     -     -     W    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
TriggerFire   -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
NeuroMemory   -    -   -   -     -     -     P    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
DaimonAffect  -    -   -   -     W     -     W    -     -    -     -    W    -    -     -    -     -     -     -    -    -     -
DreamConsol   -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
LearnRecord   -    -   -   -     W     -     -    -     -    -     -    -    -    -     -    -     -     -     -    W    -     -
ChainConnect  -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
CodeIndex     -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
CliInterface  -    -   -   -     -     -     -    -     -    -     -    W    -    -     -    -     -     -     -    -    -     -
ServeIntf     -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -
TuiInterface  -    -   -   -     -     -     -    -     -    -     -    -    -    -     -    -     -     -     -    -    -     -

W = Wired, P = Partial
```

---

## 3. Disconnected Subgraphs

Several nodes have **zero inbound or zero outbound edges** in the wired subgraph. These are integration gaps.

### 3.1 Fully disconnected nodes (zero wired edges)

| Node | Layer | Analysis |
|---|---|---|
| **Bus** | L0 | The Bus fabric has no wired consumer edges in the current system. `EventBus<E>` exists in code but is not yet the canonical transport path for cross-subsystem coordination. All current data flow uses direct function calls or Store. **This is the single largest integration gap.** |
| **Hdc** | L0 | HDC vectors exist in `roko-primitives` but are not wired into the main execution loop. Episode fingerprinting is wired, but HDC-based retrieval in Compose and similarity-based Store queries are not. |
| **DreamConsolidate** | Cross-cut | The Dreams subsystem has no wired edges to any other node. `DreamRunner` exists but is not triggered from any runtime path. |
| **ChainConnect** | Domain | Chain integration is scaffold-only. No runtime path invokes chain operations. |
| **ServeInterface** | L4 | `roko-serve` has ~85 routes but they read from the same Store as the CLI. No Bus-mediated live update path exists. |

### 3.2 Sink nodes (inbound edges only, no outbound)

| Node | Inbound From | Missing Outbound To |
|---|---|---|
| **ToolDispatch** | AgentConnect | Should publish `tool.call.started` / `tool.call.finished` Pulses to Bus for Safety and Observe |
| **TuiInterface** | LearnRecord (episodes) | Should subscribe to Bus topics for live updates instead of polling Store |

### 3.3 Source nodes (outbound edges only, no inbound)

| Node | Outbound To | Missing Inbound From |
|---|---|---|
| **CliInterface** | PlanReact | Should receive live status Pulses from Bus instead of querying Store |
| **CodeIndex** | (none wired) | Should feed ContextCompose with code-aware context; highest-leverage unwired edge |

---

## 4. Bottleneck Edges

A bottleneck edge is one whose removal disconnects a strongly-connected component or increases the graph's diameter significantly.

### 4.1 Critical bottleneck: GateVerify → PlanReact

If this edge fails (the gate pipeline cannot deliver verdicts to the orchestrator), the entire core execution loop stalls. Every task blocks waiting for verification. There is no bypass, fallback, or degraded mode.

```rust
// Current: synchronous, blocking edge
let verdict = gate_pipeline.verify(&output).await?;
plan_runner.handle_verdict(task_id, verdict).await?;

// Proposed: add a timeout-based degraded mode
let verdict = tokio::time::timeout(
    Duration::from_secs(300),
    gate_pipeline.verify(&output)
).await;

match verdict {
    Ok(Ok(v)) => plan_runner.handle_verdict(task_id, v).await?,
    Ok(Err(e)) => {
        // Gate error: record as inconclusive, do not promote
        plan_runner.handle_verdict(task_id, Verdict::inconclusive(e)).await?;
    }
    Err(_timeout) => {
        // Gate timeout: record as timeout, allow plan to continue
        // with reduced confidence
        bus.publish(Pulse::new(
            "gate.timeout",
            PulseBody::GateTimeout { task_id, elapsed: 300 },
        )).await;
        plan_runner.handle_verdict(task_id, Verdict::timeout()).await?;
    }
}
```

### 4.2 Critical bottleneck: LearnRecord → ModelRoute

If the learning subsystem cannot deliver routing updates to the cascade router, the system falls back to static model selection. This is not catastrophic (the system still works) but loses the cost optimization that routing provides.

**Current status**: This edge is wired and persistent (`roko-learn` writes cascade router state to `.roko/learn/cascade-router.json`). The bottleneck risk is data corruption of the router state file, not edge failure.

### 4.3 Structural bottleneck: PlanReact node degree

`PlanReact` (the orchestrator) has the highest node degree in the graph: 6 inbound edges, 4 outbound edges. It is the hub through which most data flows. This creates:

1. **Coupling risk**: changes to the orchestrator ripple to 10 adjacent nodes
2. **Scaling bottleneck**: the orchestrator is single-threaded for plan state transitions
3. **Testing burden**: integration tests must mock 10 interfaces

```rust
// The orchestrator's current interface surface
struct PlanReact {
    // Inbound
    cli_commands: Receiver<Command>,           // from CliInterface
    gate_verdicts: Receiver<Verdict>,          // from GateVerify
    conductor_state: Receiver<CircuitState>,   // from ConductorReact
    daimon_state: Receiver<BehavioralState>,   // from DaimonAffect
    learning_updates: Receiver<RouteUpdate>,   // from LearnRecord
    serve_commands: Receiver<ApiCommand>,      // from ServeInterface

    // Outbound
    agent_dispatch: Sender<TaskDispatch>,      // to AgentConnect
    compose_request: Sender<RoleSpec>,         // to ContextCompose
    conductor_load: Sender<SystemLoad>,        // to ConductorReact
    status_updates: Sender<PlanStatus>,        // to CliInterface
}
```

**Proposed mitigation**: Move coordination onto Bus topics. Instead of the orchestrator holding 10 channels, it subscribes to `task.dispatch.requested`, `gate.verdict.emitted`, `conductor.circuit.*`, etc. The Hub-and-spoke topology becomes a pub/sub topology with lower coupling.

---

## 5. The Target-State Graph

When all planned edges are wired, the graph has qualitatively different properties:

### 5.1 Bus becomes the backbone

In the target state, Bus carries coordination Pulses between every pair of subsystems. The direct function-call edges become Bus-mediated:

```
Current:  PlanReact --[direct call]--> AgentConnect
Target:   PlanReact --[bus: task.dispatch.requested]--> Bus --[subscribe]--> AgentConnect
```

This transforms the graph from **hub-and-spoke** (everything through the orchestrator) to **publish-subscribe** (everything through the Bus). The Bus becomes the node with the highest betweenness centrality, but it is infrastructure, not logic. A Bus failure degrades live coordination but does not corrupt durable state (which lives in Store).

### 5.2 New strongly-connected components

With Bus-mediated edges, three new SCCs form:

**SCC-4: The knowledge loop**:
```
NeuroMemory → ContextCompose → AgentConnect → GateVerify → LearnRecord → DreamConsolidate → NeuroMemory
```
Dreams consolidate episodes into knowledge entries, which feed context composition, which improves agent outputs, which produce better episodes, which Dreams consolidate. This loop only closes when DreamConsolidate has wired edges (currently disconnected).

**SCC-5: The collective intelligence loop**:
```
LearnRecord → Bus (c-factor metrics) → Observe (CollectiveIntelligenceLens) →
ModelRoute (diversity-aware routing) → AgentConnect → GateVerify → LearnRecord
```
This loop requires the c-factor Lens, which is partially built but not wired into routing decisions.

**SCC-6: The self-evolution loop**:
```
PlanReact → GateVerify → LearnRecord → (pattern quality trends) →
SpecProposal → SpecVerify → SpecApproval → PlanReact
```
This loop only exists at L5 (see [architectural-thesis.md](./architectural-thesis.md) section 5). It is entirely target-state.

---

## 6. Integration Health Lenses

Each class of integration gap needs a Lens that observes it:

### 6.1 Edge health Lens

```rust
/// Observes the health of every wired edge in the system graph.
/// Publishes `lens.edge.health` Pulses with per-edge metrics.
struct EdgeHealthLens {
    /// Expected edges from the system graph definition
    expected_edges: Vec<SystemEdge>,
    /// Observation window
    window: Duration,
}

impl Observe for EdgeHealthLens {
    fn observe(&self, store: &dyn Store, bus: &dyn Bus) -> Vec<Signal> {
        let mut observations = vec![];
        for edge in &self.expected_edges {
            let metrics = EdgeMetrics {
                edge_id: edge.id(),
                // How many Signals/Pulses traversed this edge in the window?
                throughput: self.count_traversals(edge, bus),
                // What fraction of traversals completed within latency SLA?
                latency_p99: self.latency_percentile(edge, bus, 0.99),
                // Is the edge still alive? (last traversal within window)
                alive: self.last_traversal(edge, bus) < self.window,
                // Error rate on this edge
                error_rate: self.error_fraction(edge, bus),
            };
            observations.push(Signal::new(
                Kind::Observation,
                Body::Json(serde_json::to_value(&metrics).unwrap()),
            ));
        }
        observations
    }
}
```

### 6.2 SCC health Lens

```rust
/// Observes whether strongly-connected components are cycling.
/// An SCC that stops cycling is a stalled feedback loop.
struct SccHealthLens {
    sccs: Vec<StronglyConnectedComponent>,
    min_cycle_rate: f64,  // minimum cycles per hour
}

impl Observe for SccHealthLens {
    fn observe(&self, store: &dyn Store, bus: &dyn Bus) -> Vec<Signal> {
        self.sccs.iter().map(|scc| {
            let cycle_rate = self.measure_cycle_rate(scc, bus);
            let status = if cycle_rate >= self.min_cycle_rate {
                SccStatus::Healthy
            } else if cycle_rate > 0.0 {
                SccStatus::Degraded
            } else {
                SccStatus::Stalled
            };

            Signal::new(
                Kind::Observation,
                Body::Json(json!({
                    "scc": scc.name,
                    "nodes": scc.nodes,
                    "cycle_rate_per_hour": cycle_rate,
                    "status": status,
                })),
            )
        }).collect()
    }
}
```

### 6.3 Disconnection Lens

```rust
/// Observes which nodes are disconnected from the main graph.
/// A disconnected node is a subsystem that is built but not integrated.
struct DisconnectionLens {
    system_graph: SystemGraph,
}

impl Observe for DisconnectionLens {
    fn observe(&self, store: &dyn Store, bus: &dyn Bus) -> Vec<Signal> {
        let reachable = self.system_graph.bfs_from(SystemNode::PlanReact);
        let disconnected: Vec<_> = self.system_graph.nodes()
            .filter(|n| !reachable.contains(n))
            .collect();

        vec![Signal::new(
            Kind::Observation,
            Body::Json(json!({
                "total_nodes": self.system_graph.node_count(),
                "reachable_nodes": reachable.len(),
                "disconnected_nodes": disconnected,
                "integration_ratio": reachable.len() as f64
                    / self.system_graph.node_count() as f64,
            })),
        )]
    }
}
```

---

## 7. Priority Edges to Wire

Sorted by impact (how many new SCCs or paths they enable):

| Priority | Edge | Medium | Impact |
|---|---|---|---|
| **1** | Bus ← all subsystems (publish) + Bus → all subsystems (subscribe) | Pulse | Transforms hub-and-spoke into pub/sub; enables SCC-4, SCC-5 |
| **2** | CodeIndex → ContextCompose | Signal (code-aware context) | Highest-leverage single edge: gives agents code structure awareness for free |
| **3** | DreamConsolidate → NeuroMemory | Signal (consolidated knowledge) | Closes the knowledge loop (SCC-4); enables retroactive insight |
| **4** | GateVerify → DaimonAffect | Pulse (gate.verdict.emitted) | Closes the affect learning loop; Daimon learns from verification outcomes |
| **5** | NeuroMemory → ModelRoute | Signal (knowledge-informed routing) | CascadeRouter consults knowledge store for model selection; currently TODO in CLAUDE.md |
| **6** | ServeInterface ← Bus (subscribe to projections) | Pulse | HTTP API gets live updates instead of polling Store |
| **7** | ToolDispatch → Bus (tool.call.*) | Pulse | Safety and Observe subsystems can monitor tool execution in real time |

---

## 8. What This Enables

1. **Formal integration tracking** -- the system graph is a first-class artifact, not informal prose. Every edge has a type, direction, and status.
2. **Automated integration health** -- Lenses observe edge throughput, SCC cycling, and disconnection. Degradation is detected before it causes visible failure.
3. **Priority-ordered wiring plan** -- edges are ranked by structural impact (SCC enablement, path count increase), not by subjective importance.
4. **Bus migration roadmap** -- the transformation from hub-and-spoke to pub/sub has a clear before/after graph topology.

## 9. Feedback Loops

| Loop | Input | Output | Cadence |
|---|---|---|---|
| Edge health monitoring | Per-edge throughput and latency Pulses | Edge health Signals in Store; alerts on dead edges | Gamma (continuous) |
| SCC cycling check | Per-SCC traversal counts | SCC status Signals; alerts on stalled feedback loops | Theta (plan-level) |
| Integration ratio tracking | Disconnection Lens output | Integration ratio trend; target: monotonically increasing | Delta (daily) |
| Bottleneck detection | Node degree and betweenness centrality | Bottleneck alerts when any node exceeds degree threshold | Theta (plan-level) |

## 10. Open Questions

1. **Bus as single point of failure**: In the target-state pub/sub topology, the Bus has the highest betweenness centrality. What happens when the Bus implementation fails? Current mitigation: Bus is in-process (`tokio::sync::broadcast`), so failure = process crash, which is already handled by the process supervisor. But a `MultiBus` with network transport would need partition tolerance.

2. **Graph schema validation at runtime**: Can the system validate its own integration graph against the declared schema at startup? This would catch wiring regressions: "this build is missing the GateVerify → LearnRecord edge that was present in the last release."

3. **Edge weight learning**: Should edges have learned weights representing their importance to outcome quality? A Route Cell could use edge weights to prioritize which subsystems to consult (e.g., "the CodeIndex → ContextCompose edge improved gate pass rate by 12%, so allocate more budget to code-aware context").

4. **Temporal graph analysis**: The integration graph changes over a plan execution lifecycle. At plan start, the PlanReact → AgentConnect edge is hot. At plan end, the LearnRecord → DreamConsolidate edge should be hot. Does the system's actual temporal edge activation pattern match the expected pattern? A `TemporalGraphLens` could detect anomalies.

5. **The 85-route blind spot**: `roko-serve` exposes ~85 HTTP routes, but these routes read from Store synchronously. They are not modeled as edges in the integration graph because they are request/response, not data flow. Should they be modeled as edges? If so, the API surface becomes a node with 85 outbound edges to Store, which changes the bottleneck analysis significantly.
