# Unified Specification — Index

> **Version**: 3.0
> **Date**: 2026-04-26
> **Scope**: Protocol specification for the agent economy. Defines the vocabulary, composition rules, and behavioral contracts for Roko — a system where agents build themselves.

---

## What This Specification Is

This specification defines **standards for an agent economy**: persistent identity, trustless coordination, and composable computation. The design peers are Stripe (payment protocol), Ethereum (execution protocol), and ERC-20 (token standard) — not LangGraph, CrewAI, or AutoGen.

Three technical bottlenecks stand between today's agent demos and a functioning agent economy (cf. Sequoia Capital, "The Agentic Web," 2026):

| Bottleneck | How This Spec Addresses It |
|---|---|
| **Persistent identity** | ERC-8004 agent identities + HDC fingerprints (Kanerva 2009) + ZK attestation (PP-HDC) |
| **Agent communication protocol** | MCP (tool discovery, Linux Foundation) + A2A (agent cards) + Bus (ephemeral pub/sub) + stigmergic coordination (Dorigo 1992) |
| **Trust without face-to-face** | ZK proofs over HDC vectors + TraceRank reputation + demurrage-weighted knowledge (Gesell 1916) with on-chain provenance |

The four external protocols — **MCP**, **A2A**, **ERC-8004**, **x402** — form a fixed exoskeleton. This spec defines what flows through them.

---

## The One Rule

**Everything is a Graph of Cells processing Signals through Bus and Store.**

Every system, subsystem, and feature in this specification is expressed as a composition of the same five primitives: **Signal** (data), **Cell** (computation), **Graph** (composition), **Bus** (ephemeral transport), and **Store** (durable persistence). There are no special cases. If something seems to need special machinery, it means we need a new Cell specialization — not a new concept.

This rule eliminates god files, ad-hoc state management, and one-off infrastructure. It also means every subsystem composes with every other subsystem by construction, because they all speak the same protocol.

---

## Vocabulary

Roko is built from **5 primitives**, **9 protocols**, **4 universal patterns**, and **13 specializations**. A developer learns 14 concepts; everything else is a discoverable pattern composed from them.

### 5 Primitives

| Primitive | What It Is | Primary Doc |
|---|---|---|
| **Signal** | Durable datum. Content-addressed (SHA-256), typed (`Kind`), scored (5 axes), decayed via demurrage (Gesell 1916), lineage-tracked, HDC-fingerprinted (10,240-bit). Lives in **Store**. | [01](01-SIGNAL.md) |
| **Pulse** | Ephemeral event. Sequence-numbered, ring-buffered, broadcast via **Bus**. Signal's ephemeral sibling. **Graduation** promotes Pulse → Signal — the only path from transport to audit DAG. | [01](01-SIGNAL.md) |
| **Cell** | Atomic computation. Signals in, Signals out. Declares typed I/O, capabilities, protocol conformance. Every Cell is a learner via **predict-publish-correct** (Friston 2006). | [02](02-CELL.md) |
| **Graph** | Typed DAG of Cells connected by edges. TOML-defined, serializable, runtime-interpreted. **Hot Graphs** stay resident and re-fire per tick. Graphs are themselves Cells (fractal composition). | [03](03-GRAPH.md) |
| **Protocol** | Behavioral contract that a Cell conforms to. 9 protocols: Store, Score, Verify, Route, Compose, React, Observe, Connect, Trigger. Each supports predict-publish-correct. | [02](02-CELL.md) |

### Two Fabrics

| Fabric | Medium | Lifetime | Addressing | Access Pattern |
|---|---|---|---|---|
| **Store** | Signal (durable) | Persists until pruned by demurrage | Content-addressed (SHA-256) | Put/Get/Query/QuerySimilar |
| **Bus** | Pulse (ephemeral) | Ring-buffered, evicted by position | Sequence-numbered | Publish/Subscribe by topic |

These are the only two data transport mechanisms. Nothing else. No ad-hoc state channels, no hidden caches, no side-band communication. Every piece of data in the system flows through one of these two fabrics.

### 9 Protocols

Every protocol supports **predict-publish-correct**. A Cell publishes its prediction as a Pulse, reality publishes the outcome, a CalibrationPolicy joins by lineage and computes error, and the Cell subscribes to its error topic to update. Learning is structural.

| Protocol | Verb | What It Does | Key Property |
|---|---|---|---|
| **Store** | put / get / query / query_similar / prune | Persist and retrieve Signals | Content-addressed, demurrage-decayed |
| **Score** | rate along 5 dimensions | Evaluate Signal quality | Calibrated via Beta-Binomial tracker |
| **Verify** | check → Verdict | Validate correctness, safety, quality | Conjunctive hard + Pareto soft (Goodhart-resistant) |
| **Route** | select among candidates | Choose Cell/model/path for task | EFE (Friston): epistemic + pragmatic − cost |
| **Compose** | assemble under budget → Signal | Combine context for LLM calls | VCG auction with section effect tracking |
| **React** | watch Pulses → emit Signals/Pulses | Real-time event response | Operates on ephemeral Bus stream |
| **Observe** | read-only → observation Signals | Monitor without mutation | Lens pattern for telemetry |
| **Connect** | connect / query / execute / disconnect | External system I/O | Lifecycle-managed with health checks |
| **Trigger** | listen for events → fire Graphs | Declarative event ingress | Debounce, filter, conditional |

### 4 Universal Patterns

These are recurring Graph topologies that appear across the spec. Every concrete system is an instance of one (or a composition of several) of these patterns.

| Pattern | Topology | Key Property | Instances |
|---|---|---|---|
| **Pipeline** | Linear chain of Cells. Each can reject (Verify), transform (Compose), or redirect (Route). | Sequential processing with early exit. | Inference gateway, gate pipeline, immune system, auth chain |
| **Loop** | Graph with feedback edge from output back to input. React Cell watches outcomes and adjusts. | Self-improving via predict-publish-correct. | Cognitive loop, calibration loop, dream cycle, adaptive thresholds, cascade router |
| **Functor** | Cross-cut that enriches Signals pre/post a Cell without changing the Graph's topology. Endofunctor F: Signal → Signal. | Composable orthogonal concerns. | Memory enrichment, Daimon affect bias, Dreams consolidation, Safety constraints |
| **Space** | Graph that owns a Bus partition + Store partition. Members share these resources under access control. | Isolation + collaboration boundary. | Agent, Group, Workspace, Namespace, Sandbox |

**Fractal composition**: Graphs are Cells. A Graph of Graphs is just a Graph. A Pipeline of Pipelines is just a Pipeline. A Loop containing a Pipeline is just a Loop. This eliminates special glue code between subsystems.

### 13 Specializations

Named patterns of protocol combinations. Conventions, not new primitives.

| Name | Built From | What It Adds |
|---|---|---|
| **Flow** | Graph + RunId + snapshots | Runtime instance with resume capability |
| **Hot Flow** | Flow that stays resident | Re-fires per tick (agent pipelines) |
| **Rack** | Graph + Macros (knobs) + Slots (jacks) | DAW-inspired parameterization |
| **Lens** | Cell + Observe protocol | Read-only observation; StateHub projections |
| **Loop** | Graph + feedback edge | Self-improving pattern |
| **Memory** | Store Cell + demurrage + dreams | Self-trimming knowledge with consolidation |
| **Extension** | Cell intercepting another Cell's pipeline | 8 layers, 22 hooks, CaMeL IFC |
| **Agent** | Space + Extensions + Memory + clock + vitality | Full cognitive runtime |
| **Connector** | Cell + Connect protocol + lifecycle | External system I/O |
| **Feed** | Cell + Connect + Trigger + Store | Continuous data stream with subscription |
| **Recipe** | Graph of Score Cells (pure data pipeline) | Composable data transformations (no LLM) |
| **Group** | Space + membership + coordination mode | Persistent agent collective |
| **Pipeline** | Linear Graph of Verify + Compose + Route Cells | Sequential processing with early exit |

---

## Design Principles

### 1. Two mediums, two fabrics

Durable **Signals** in **Store**, ephemeral **Pulses** on **Bus**. The system has two data shapes because reality has two timescales: things that persist and things that flow. **Graduation** converts Pulse → Signal (the only path into the audit DAG). **Projection** converts Signal → Pulse (lossy broadcast).

### 2. Every operator is a learner

**Predict-publish-correct** via Bus (Friston 2006). Every Cell publishes its prediction as a Pulse, subscribes to its own error topic, and adjusts. Learning is structural — it emerges from the same pub/sub fabric that carries heartbeats and gate verdicts, not from a separate bolted-on subsystem.

### 3. Demurrage is default

Signals decay via attention-weighted holding cost (Gesell 1916). Balance starts at 1.0, decreases unless actively reinforced by retrieval, citation, surprise, or gate-pass. Self-trimming knowledge: unique insights stay warm, duplicates fade.

### 4. Verify is load-bearing

The Verify protocol serves four roles simultaneously: **reward function** (continuous `Verdict.reward: f64`), **relabeling oracle** (hindsight relabeling of failed trajectories), **safety boundary** (`verify_pre()` can veto), **economic attestation** (reputation via ERC-8004). Conjunctive hard + Pareto soft resists Goodhart's Law.

### 5. Mortality is a feature

Agents have finite **vitality** creating behavioral phases (Thriving → Stable → Conservation → Declining → Terminal). Economic pressure drives efficient resource use and honest self-assessment (Jonas 1966).

### 6. Collective intelligence is measurable

The **c-factor** (Woolley et al. 2010) is computed from Bus/Store statistics. It is a **covariate, not an objective** — optimizing c directly can be gamed. It gates L4 evolution: only evolve configurations that increase genuine collective intelligence.

### 7. Everything composes from primitives

No special machinery. An Agent is Space + Extensions + Memory + clock. A dream cycle is a Loop. The inference gateway is a Pipeline. The cascade router is a Loop. Feeds are Cells with Connect+Trigger+Store protocols. Groups are Spaces with membership. Auth is a Pipeline of Verify Cells. If you need a new concept, express it as a composition of existing primitives.

### 8. Cost falls mechanically with volume

Wright's-law on rails — Route = EFE balances quality vs cost; Compose = budget-constrained VCG; Verify = cost attribution; Demurrage = even memory has cost pressure; T0 gating = ~80% of ticks cost $0. Stacked: caching (5×) × routing (3×) × gating (2×) = **10–30× cost reduction**.

### 9. Protocol, not framework

Signal/Cell/Graph are standards. Each new conforming Cell multiplies combinations with every existing Cell, Graph, and Signal channel (ERC-20 precedent). The spec defines what flows through MCP, A2A, ERC-8004, and x402. It does not replace them.

### 10. The spec is a runtime artifact

These documents are readable by agents at startup, queryable as MCP tools, evolvable through L4, and signed under ERC-8004.

### 11. Safety scales with autonomy

CaMeL capability-tagged IFC on Extensions. Nayebi 5-head lexicographic corrigibility (deference > switch > truth > impact > task). Verify gates sit **outside the modifiable surface** — the agent cannot modify its own verification pipeline.

---

## Anti-Principles

| Anti-Pattern | Response |
|---|---|
| Standalone destination app | Embed in existing surfaces |
| Naïve multi-agent debate | Require heterogeneity + structured indirection (Surowiecki 2004) |
| Opaque marketplace economics | Publish all metrics, transparent take-rates |
| "We have the most data" moat | Protocol + workflow embedding + cross-side marketplace |
| Weighted-sum verification | Conjunctive hard + Pareto soft |
| LLM-judging-itself | Variance Inequality: verifier spectrally cleaner than generator |
| Token speculation | ERC-8004 agent identity and utility, not token price |
| God files | Composition of small Cells; max ~500 LOC per Cell |
| Ad-hoc state | Everything through Bus or Store; no hidden channels |
| One-off infrastructure | Express as Cell specialization; reuse patterns |

---

## Five Compounding Mechanisms

1. **Protocol composability** (ERC-20 precedent): each new Cell multiplies combinations combinatorially.
2. **Reed's-law group formation**: stigmergic coordination lets coalitions form without permission. Corrected value ∝ N·log(N).
3. **Wright's-law cost curve**: LLM prices fell 9–900× per year. Primitives convert savings to more usage.
4. **Knowledge compounding with attribution**: HDC + ERC-8004 + demurrage = compounding semantic memory with cryptographic provenance.
5. **Recursive self-improvement**: L4 makes the spec itself an agent in the evolutionary archive, bounded by Variance Inequality + c-factor gate.

---

## Document Map

### Kernel (learn the type system)

| # | Document | What It Defines |
|---|---|---|
| **[01](01-SIGNAL.md)** | Signal and Pulse | Two mediums: durable Signal and ephemeral Pulse. Bus and Store. Demurrage. HDC. Kind system. Graduation/Projection. Taint and provenance. |
| **[02](02-CELL.md)** | Cell and Protocols | Universal computation unit. 9 protocol traits. TypeSchema. Capabilities. Predict-publish-correct. Protocol composition. Cost estimation. |
| **[03](03-GRAPH.md)** | Graph | Universal composition. Typed DAG. TOML definition. Hot Graphs. Workflow/Activity split. Snapshot/resume. Merge queue. Edge validation. |
| **[04](04-EXECUTION.md)** | Execution Engine | Runtime interpreter. Cognitive loop as graph. 7 Cells with typed I/O. T0 short-circuit. Nested loops (gamma/theta/delta). Crash recovery. Numerics. Error taxonomy. |

### Cognition (agent intelligence)

| # | Document | What It Defines |
|---|---|---|
| **[05](05-AGENT.md)** | Agent Runtime | Dual-process EFE routing. 3 cognitive timescales. Cognitive energy and vitality. Type-state lifecycle. CorticalState. Somatic markers. PAD affect. Emergent goals. |
| **[06](06-MEMORY.md)** | Memory and Knowledge | Neuro store. Demurrage economics. Dreams and consolidation. HDC algebra. Temporal knowledge graph (Allen intervals). Heuristics with mandatory falsifiers. |
| **[07](07-LEARNING.md)** | Learning Loops | 4 loops (Demurrage-retrieval, Heuristic calibration, HDC cleanup, c-factor feedback). Autocatalytic compounding. Playbooks. Episodes. Cascade router. Experiments. Adaptive thresholds. |

### Infrastructure (product systems — each is a Graph of Cells)

| # | Document | What It Defines |
|---|---|---|
| **[08](08-GATEWAY.md)** | Inference Gateway | 9-stage Pipeline Graph: loop detection → cache → tool pruning → output budgeting → thinking cap → convergence → provider call → cache store → cost tracking. InferenceHandle. Batch API. CascadeRouter fallback. |
| **[09](09-FEEDS.md)** | Feeds and Recipes | Continuous data streams as Cell specialization (Connect+Trigger+Store). Raw/derived/composite/meta feeds. Feed registry. Recipes as pure data Graphs. Dynamic registration. On-chain advertisement. |
| **[10](10-GROUPS.md)** | Groups and Coordination | Persistent agent collectives as Space specialization. 4 coordination modes (stigmergic, pipeline, broadcast, leader-follower). Membership protocol. Cross-user invitation. Shared knowledge and pheromone fields. |
| **[11](11-CONNECTIVITY.md)** | Connectivity and Relay | Relay wire protocol (rooms, envelopes, heartbeat, reconnection). Exoskeleton protocols (MCP, A2A, ERC-8004, x402). Multi-chain. WebSocket subscriptions. Backpressure. |
| **[12](12-EXTENSIONS.md)** | Extension System | 8 layers, 22 hooks. CaMeL IFC. Discovery and dependency resolution. Connector primitive. Extension lifecycle. |
| **[13](13-TRIGGERS.md)** | Trigger System | Declarative event ingress. Event sources. Trigger bindings. Debounce and filter. Conductor watchers (10 rules). |
| **[14](14-TOOLS.md)** | Tool Catalog | 45+ shipped Cells by protocol. MCP integration. Tool dispatch. Safety hooks. Plugin ecosystem. |

### Observation and operations

| # | Document | What It Defines |
|---|---|---|
| **[15](15-TELEMETRY.md)** | Telemetry | Lens system. StateHub projections. c-factor computation. Metric collection. Dashboard data sources. |
| **[16](16-SECURITY.md)** | Security Model | Taint lattice IFC. Immune system as 5-layer Pipeline Graph. Capability intersection. Sandbox levels. CaMeL. 5-head corrigibility. Quarantine. |
| **[17](17-AUTH.md)** | Authentication | 4 auth paths (Privy, API keys, agent tokens, relay). Team workspace sharing. Invitation flow. JWKS caching. Token lifecycle. Expressed as Pipeline of Verify Cells. |
| **[18](18-PAYMENTS.md)** | Payments | x402 (per-request). MPP (session-based streaming). Reputation pricing. Settlement batching. Feed marketplace economics. Expressed as Store Cells with economic semantics. |

### Configuration and surfaces

| # | Document | What It Defines |
|---|---|---|
| **[19](19-CONFIG.md)** | Configuration | Config-as-Signal. Schema versioning. Migration. Priority merging (CLI > env > TOML). 7 invariants. Config reload as Trigger Graph. Domain profiles. |
| **[20](20-SURFACES.md)** | Surfaces | CLI, HTTP API, TUI, web dashboard. Visual composition / authoring with 12 primitive types (DAW pattern). Workbench. Agent Inbox. Generative Canvas. |

### Economy and domains

| # | Document | What It Defines |
|---|---|---|
| **[21](21-MARKETPLACE.md)** | Marketplace | Agent identity/passport. Reputation (TraceRank). Commerce. DAW composability. Fork chains. Transparent take-rates. |
| **[22](22-REGISTRIES.md)** | On-Chain Registries | ERC-8004. ZK-HDC proofs. On-chain InsightStore. Chain witness. Gossip networking. Job market. |
| **[23](23-ARENAS.md)** | Arenas and Evals | Universal measurement surface. 7-step flywheel. Task sources. Scoring functions. Leaderboards. Bounty market. Meta-arena. |
| **[24](24-DEFI.md)** | DeFi Infrastructure | ISFR oracle. Yield perpetuals. VCG clearing. VenueAdapter. DeFiRiskEngine. TradingReflect. Multi-chain. Affect-modulated sizing. All as domain-specific Cell specializations. |

### Meta

| # | Document | What It Defines |
|---|---|---|
| **[25](25-DEPLOYMENT.md)** | Deployment | Railway/Fly/Docker. Daemon lifecycle. Brain export. Merkle-CRDT. WASM+native. Secrets management. |
| **[26](26-CROSS-CUTS.md)** | Cross-Cut Functors | Memory/Daimon/Dreams as endofunctors. VCG arbitration. Safety wrapper. Natural transformations. Commuting triangle. Gate failure cascade. |
| **[27](27-ORCHESTRATOR.md)** | Orchestrator | Plan runner v2 (event-driven). Mori parity: structured review, auto-fix, error sharing, reflection loop, context scoping, warm spawn. |
| **[28](28-ROADMAP.md)** | Roadmap | Phased delivery. Implementation priorities. Phase dependencies. Current state reconciliation. |

---

## Naming Decisions

| Decision | Rationale |
|---|---|
| **Signal** (not Engram) | Immediately meaningful. `type Signal = Engram;` in code. |
| **Pulse** (not Envelope) | Names the ephemeral sibling. Code has `Envelope<E>`. |
| **Bus** (not EventBus) | Promoted to kernel fabric alongside Store. |
| **Cell** (not Module/Block) | Composable, small, pluggable — Eurorack module, Scratch block. |
| **Graph** (not Workflow) | Precise: nodes + edges. "Flow" is the runtime instance. |
| **Rack** (not Parameterized Workflow) | DAW: macro knobs and patch jacks. |
| **Lens** (not Monitor/Probe) | Read-only by definition. Stacking gives different views. |
| **Loop** (not Feedback/Cycle) | Direct and unambiguous. |
| **Space** (not Workspace) | Clean isolation + grants. |
| **Feed** (not Stream) | Continuous data stream. Cell specialization for always-on data. |
| **Recipe** (not Pipeline/Transform) | Pure data composition — no LLM, no agent. Distinct from Plan (task DAG) and Compose (prompt assembly). |
| **Group** (not Fleet/Swarm) | Persistent collective with identity. |
| **Demurrage** (not Decay) | Active economic mechanism — use restores value (Gesell 1916). |
| **Vitality** (not Budget/Lifetime) | Captures both economic pressure and behavioral modulation. |
| **Heuristic** (not Playbook/Rule) | Testable prediction with calibration + mandatory falsifier. |

---

## Concept Migration

### From roko-core code

| Code | Spec | Notes |
|---|---|---|
| `Engram` | Signal (durable) | Direct map. Rust struct stays `Engram`. |
| `Envelope<E>` / EventBus | Pulse / Bus | Promoted from impl to kernel. |
| `Substrate` | Store protocol | + `query_similar`. |
| `Scorer` | Score protocol | + calibration. |
| `Gate` | Verify protocol | + pre/post/stream, continuous reward, evidence typing, conjunctive/Pareto. |
| `Router` | Route protocol | + EFE, regime conditioning. |
| `Composer` | Compose protocol | + VCG, section effects. |
| `Policy` | React protocol | **Breaking**: operates on Pulses. |

### From architecture docs

| Old | New | Key Change |
|---|---|---|
| Extension (22 hooks) | Extension specialization | + CaMeL IFC. |
| Connector (5 methods) | Connector specialization | Connect protocol + lifecycle. |
| Feed (ad-hoc) | Feed specialization | Cell with Connect+Trigger+Store protocols. |
| Recipe (ad-hoc) | Recipe specialization | Graph of Score Cells (pure data pipeline). |
| Group (ad-hoc) | Group specialization | Space + membership + coordination mode. |
| InferenceGateway | Gateway Pipeline Graph | 9 Cells in a Pipeline; not a bespoke service. |
| Auth system | Auth Pipeline | Pipeline of Verify Cells. |
| Payment (x402/MPP) | Payment Store Cells | Store protocol with economic semantics. |
| DeFi types | DeFi Cell specializations | Domain-specific Cells implementing standard protocols. |

---

## Vocabulary Table

| Concept | What It Is | Primary Doc |
|---|---|---|
| Signal (durable) | Content-addressed, lineage-bearing, scored, decayed via demurrage, persisted in Store | 01 |
| Pulse (ephemeral) | Sequence-numbered, ring-buffered, broadcast via Bus | 01 |
| Graduation | Pulse → Signal. Only path from transport to audit DAG | 01 |
| Bus | Ephemeral pub/sub transport fabric | 01 |
| Demurrage | Attention-weighted retention (Gesell 1916) | 01, 06 |
| Heuristic | Signal kind: when/then + mandatory falsifier + calibration | 01, 06 |
| AntiKnowledge | Signal kind that repels future Signals in same HDC region | 01, 06 |
| Predict-publish-correct | Every operator predicts → publishes → corrects via Bus (Friston 2006) | 02, 07 |
| Pre-action Verify | `verify_pre()` — check before execution, can veto | 02 |
| Continuous reward | `Verdict.reward: f64` — domain-specific learning signal | 02 |
| Evidence typing | Separate from Criterion, 19 evidence kinds | 02 |
| Conjunctive/Pareto | Hard criteria (AND) vs soft criteria (Pareto, never weighted-sum) | 02 |
| Bradley-Terry judges | Pairwise comparison via BT MLE, disjoint-family panels | 02 |
| EFE gating | Expected Free Energy (Friston) for T0/T1/T2 routing | 02, 05, 07 |
| Regime conditioning | Calm/Normal/Volatile/Crisis affects routing and caching | 05, 07 |
| VCG auction | Budget-constrained context assembly | 02, 05 |
| Section effect | Beta-distribution: which sections correlate with gate success | 05, 07 |
| Hot Graph | Tick-driven Flow that stays resident between firings | 03, 04 |
| Workflow/Activity split | Deterministic orchestration vs non-deterministic execution (cf. Temporal) | 04 |
| Vitality | remaining_budget / initial_budget — behavioral modulation scalar | 05 |
| Behavioral phases | Thriving / Stable / Conservation / Declining / Terminal | 05 |
| Type-state lifecycle | Compile-time enforced Agent state transitions | 05 |
| CorticalState | Lock-free atomic shared perception surface | 05 |
| Somatic markers | PAD affect + prospect theory (Kahneman-Tversky λ=2.25) | 05 |
| Hindsight relabeling | Failed trajectories relabeled for achieved sub-goals (≥45% recovery) | 07 |
| Variance Inequality | Verifier spectrally cleaner than generator | 07 |
| c-factor | Collective intelligence as runtime observable (Woolley et al. 2010) | 07, 15 |
| Resonator Networks | HDC factorization — recover constituents from bundles | 06 |
| CaMeL IFC | Capability-tagged information flow control on Extensions | 12, 16 |
| 5-head corrigibility | Lexicographic: deference > switch > truth > impact > task (Nayebi) | 16 |
| StateHub projections | Universal typed projections for all surfaces | 15 |
| Exoskeleton protocols | MCP + A2A + ERC-8004 + x402 | 11 |
| Feed | Continuous data Cell (Connect+Trigger+Store) | 09 |
| Recipe | Pure data Graph (no LLM, no agent) | 09 |
| Group | Space + membership + coordination mode | 10 |
| InferenceHandle | Channel sender to gateway — agents never hold API keys | 08 |
| Pipeline | Linear Graph with reject/transform/redirect | 08, 16, 17 |
| Arena | Universal measurement surface + 7-step flywheel | 23 |
| Brain export | Portable agent knowledge via Merkle-CRDT (~100KB–1MB) | 25 |
| Workbench | Structured task delegation surface (Linear/Notion pattern) | 20 |
| Generative Canvas | Visual Graph editor with typed cables (Bitwig/n8n pattern) | 20 |
| Autonomy Slider | Progressive trust control (5 levels, per-capability) | 20 |

---

## Reading Order

### Essential — learn the type system
1. **[01-SIGNAL](01-SIGNAL.md)** → Two mediums, two fabrics.
2. **[02-CELL](02-CELL.md)** → Universal computation, 9 protocols.
3. **[03-GRAPH](03-GRAPH.md)** → Universal composition.
4. **[04-EXECUTION](04-EXECUTION.md)** → How Graphs run.

### Core systems
5. **[05-AGENT](05-AGENT.md)** → The cognitive runtime.
6. **[06-MEMORY](06-MEMORY.md)** → Self-trimming knowledge.
7. **[07-LEARNING](07-LEARNING.md)** → How everything improves.

### Product layer — each is a Graph of Cells
8–14. Gateway, Feeds, Groups, Connectivity, Extensions, Triggers, Tools.

### Operations
15–18. Telemetry, Security, Auth, Payments.

### Everything else
19–28. Config, Surfaces, Marketplace, Registries, Arenas, DeFi, Deployment, Cross-Cuts, Orchestrator, Roadmap.

---

## Version History

| Version | Date | Changes |
|---|---|---|
| 1.0 | 2026-04-20 | Initial unified spec (22 docs). |
| 2.0 | 2026-04-24 | Cell rename from Block, protocol refinements, plan runner v2. |
| 3.0 | 2026-04-26 | **Major restructure**: 29 docs (00-28). Absorbs `architecture/` (gateway, feeds, groups, auth, payments, DeFi, visual composition, orchestrator gaps). Introduces 4 universal patterns (Pipeline, Loop, Functor, Space). Enforces "everything is a Graph of Cells" universally. All concrete systems expressed as compositions of kernel primitives. Previous unified docs archived to `v2-archive/`. Architecture docs archived to `architecture-archive/`. |
