# 28 — Roadmap

> Phased implementation mapping the unified spec to existing crates. Phase 0 is what works today. Phase 1 upgrades the kernel. Phase 2 builds the Graph engine and Agent runtime. Phase 3 adds autonomy, safety, and the agent economy. Each phase is a Graph of Cells delivering Signals through Bus and Store.

**Depends on**: All preceding documents. This document is a delivery plan, not a specification. It cross-references every other document in the unified spec.

---

## 1. Phase 0 — Current State

What is already built, working, and wired. This is the foundation the roadmap builds on.

### 1.1 Crate Mapping

| Crate | Unified Concepts | Status |
|---|---|---|
| `roko-core` | Signal (Engram), 6 protocols (Store/Score/Verify/Route/Compose/React) | Kernel, stable |
| `roko-agent` | Agent specialization: 9-step pipeline, 5+ LLM backends, MCP, tool loop, safety | Dispatch wired |
| `roko-agent-server` | Agent sidecar: `/message`, `/stream`, `/predictions`, `/research`, `/tasks` | Wired |
| `roko-orchestrator` | Graph execution (plan DAG, parallel executor, merge queue) | Wired via orchestrate.rs |
| `roko-gate` | Verify protocol: 11 gates, 7-rung pipeline, adaptive thresholds | Wired, called per-task |
| `roko-compose` | Compose protocol: prompt assembly, 9 templates, VCG auction, enrichment | Wired |
| `roko-learn` | Learning Loops 1+2: episodes, cascade router, experiments, efficiency, bandits | Fully wired |
| `roko-neuro` | Memory specialization: knowledge store, tiers, HDC fingerprints, distillation | Wired |
| `roko-dreams` | Loop 3 partial: NREM/REM/Integration phases built, no runtime trigger | Built, not triggered |
| `roko-conductor` | 10 watchers, circuit breaker, diagnosis | Wired into executor |
| `roko-runtime` | ProcessSupervisor, event bus, cancellation | Wired into PlanRunner |
| `roko-primitives` | HDC vectors, tier routing | Fully wired |
| `roko-daimon` | Affect engine, somatic markers, dispatch modulation | Wired per-task |
| `roko-serve` | ~85 HTTP routes, SSE, WebSocket on :6677 | Wired |
| `roko-cli` | All CLI commands + ratatui TUI (F1-F7 tabs) | Main entry point |
| `roko-fs` | FileSubstrate (JSONL), GC, layout | Stable |
| `roko-std` | 19 builtin tools, mock dispatcher | Stable |
| `roko-mcp-code` | Code-intelligence MCP server | Wired |
| `roko-index` | Parser + graph + HDC indexing | Built |
| `roko-chain` | Chain witness primitives, marketplace, validation registry | Partial |

### 1.2 Working End-to-End Flows

The following flows work today via CLI:

1. **Self-hosting loop**: `prd idea` -> `prd draft` -> `prd plan` -> `plan run` -> gate validate -> persist results -> `plan run --resume`
2. **Research-enhanced planning**: `research enhance-prd` -> `prd plan` with research context
3. **Automatic replan**: Gate failure triggers `build_gate_failure_plan_revision` (Loop 2)
4. **Auto-plan on publish**: `prd.auto_plan` config triggers plan generation when PRD is published
5. **Interactive monitoring**: `roko dashboard` TUI with F1-F7 tabs
6. **HTTP control plane**: `roko serve` exposes ~85 routes for external callers
7. **Agent sidecar**: `roko agent serve` with real LLM dispatch

### 1.3 What Phase 0 Lacks (In Unified Vocabulary)

- No **Pulse/Bus kernel** — event bus exists in roko-runtime but is not promoted to a kernel trait alongside Store
- No **predict-publish-correct** pattern — learning happens but is not structural via Bus pub/sub
- No **demurrage** on knowledge Signals — tiers exist but no balance/decay mechanics
- No **Heuristic kind** — knowledge Signals have no when/then/falsifier structure
- No **EFE routing** — CascadeRouter uses LinUCB bandits, not Expected Free Energy (Friston 2006)
- No **Observe protocol** (Lens system) — monitoring exists but is not protocol-based
- No **Trigger protocol** — triggers are hardcoded, not declarative
- No **Connect protocol** formalization — connectors exist but are not protocol-based
- No **Graph authoring** — plans are TOML task lists, not typed Graphs with edges
- No **Rack** abstraction — no parameterized Graphs
- No **TypeSchema validation** at load time
- **Loop 3** (dream cycle) is built but has no runtime trigger
- **Loop 4** (structural adaptation) does not exist
- On-chain registries are specified but not deployed

### 1.4 Remaining Work Items (from CLAUDE.md)

These are the immediate priorities before entering Phase 1:

| Item | Description | Status |
|---|---|---|
| 13 | Knowledge-informed agent routing — neuro store not yet consulted for model selection in CascadeRouter | Not started |
| 14 | Cold substrate archival — built but not instantiated at runtime (no cron/trigger) | Not started |
| 15 | UX34: force_backend override learning — cascade router doesn't learn from manual overrides | Not started |
| 16 | Chain runtime integration — Phase 2+ (needs blockchain backend for witness anchoring) | Deferred |

---

## 2. Phase 1 — Kernel Upgrade (Near-Term)

Promote Pulse and Bus to kernel-level, wire predict-publish-correct, add demurrage, introduce Heuristic kind, upgrade routing to EFE, trigger the dream cycle, and formalize the Observe/Trigger/Connect protocols.

### 2.1 Pulse/Bus Kernel

**Goal**: Pulse is a first-class data type alongside Signal. Bus is a kernel trait alongside Store. All real-time behavior flows through Bus.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define `Pulse` struct in roko-core | `crates/roko-core/src/pulse.rs` | S | -- |
| Define `Bus` trait in roko-core alongside Store | `crates/roko-core/src/bus.rs` | S | Pulse struct |
| Implement `BroadcastBus` (in-process, tokio broadcast) | `crates/roko-runtime/src/bus/` | M | Bus trait |
| Implement `MemoryBus` (testing) | `crates/roko-runtime/src/bus/` | S | Bus trait |
| Define `TopicFilter` enum (Exact, Glob, AnyOf, All, And, Or, Not) | `crates/roko-core/src/bus.rs` | S | Bus trait |
| Wire Bus into CellContext | `crates/roko-core/src/block.rs` | M | BroadcastBus |
| Implement `Pulse::graduate()` -> Signal | `crates/roko-core/src/pulse.rs` | S | Pulse + Signal |
| Implement `Signal::to_pulse()` (lossy projection) | `crates/roko-core/src/signal.rs` | S | Pulse + Signal |
| Topic taxonomy: define standard topic hierarchy | `crates/roko-core/src/topics.rs` | S | -- |
| Wire Cell lifecycle events as Pulses on Bus | `crates/roko-orchestrator/` | M | BroadcastBus |

### 2.2 Predict-Publish-Correct

**Goal**: Every operator publishes predictions as Pulses, subscribes to its error topic, and updates. Learning is structural via Bus pub/sub (Friston 2006, active inference).

| Task | Target | Size | Depends On |
|---|---|---|---|
| Implement `CalibrationPolicy` React Cell | `crates/roko-learn/src/calibration.rs` | M | Bus |
| Wire Scorer prediction -> Pulse("prediction.scorer") | `crates/roko-learn/src/scorer/` | S | Bus |
| Wire Gate verdict -> Pulse("outcome.scorer") | `crates/roko-gate/src/` | S | Bus |
| Wire Router prediction -> Pulse("prediction.router") | `crates/roko-learn/src/routing/` | S | Bus |
| Wire Composer prediction -> Pulse("prediction.composer") | `crates/roko-compose/src/` | S | Bus |
| CalibrationPolicy joins prediction + outcome by lineage_hint | `crates/roko-learn/src/calibration.rs` | M | All predictions |
| Per-operator calibration state persistence | `crates/roko-learn/src/calibration.rs` | S | CalibrationPolicy |

### 2.3 Demurrage

**Goal**: Knowledge Signals decay via attention-weighted holding cost (Gesell 1916). Retrieval, citation, gate-pass, and surprise restore balance. Self-trimming knowledge.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Add `balance`, `demurrage_paid`, `last_touched_at` fields to Engram | `crates/roko-core/src/engram.rs` | S | -- |
| Implement demurrage rate law: `balance(t+dt) = balance(t) - r*dt - beta*balance(t)*dt` | `crates/roko-neuro/src/demurrage.rs` | M | Balance fields |
| Implement reinforcement kinds (Retrieved, Cited, GatePassed, Surprised, AgentQuoted) | `crates/roko-neuro/src/demurrage.rs` | S | Demurrage |
| Novelty-weighted reinforcement: `bonus * (1 - max_similarity)` | `crates/roko-neuro/src/demurrage.rs` | M | HDC index |
| Tier multipliers (Transient 0.1x, Working 0.5x, Consolidated 1.0x, Persistent 5.0x) | `crates/roko-neuro/src/demurrage.rs` | S | Demurrage |
| Cold threshold: balance < 0.01 triggers archive to cold storage | `crates/roko-neuro/src/demurrage.rs` | S | Demurrage |
| Wire demurrage into knowledge store retrieval and storage paths | `crates/roko-neuro/src/store.rs` | M | All demurrage |

### 2.4 Heuristic Kind

**Goal**: First-class Signal kind with when/then clause, mandatory falsifier, and live calibration from Bus events.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define `Kind::Heuristic` with `HeuristicPayload` | `crates/roko-core/src/kind.rs` | S | -- |
| Define `Calibration` struct (trials, confirmations, violations, Brier score, Wilson CI) | `crates/roko-core/src/kind.rs` | S | Heuristic kind |
| Implement heuristic calibration from gate verdicts via Bus | `crates/roko-learn/src/heuristic_calibration.rs` | M | Bus + Heuristic |
| Wire heuristic query into Compose protocol context assembly | `crates/roko-compose/src/bidders/` | S | Heuristic kind |

### 2.5 EFE Routing

**Goal**: Replace LinUCB bandits in CascadeRouter with Expected Free Energy (Friston 2006). EFE naturally balances exploration (epistemic value) and exploitation (pragmatic value) while being cost-aware.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Implement EFE computation (epistemic + pragmatic + cost terms) | `crates/roko-learn/src/routing/efe.rs` | L | -- |
| Add regime conditioning: Route receives `regime: Regime` (Calm/Normal/Volatile/Crisis) | `crates/roko-learn/src/routing/` | S | EFE |
| Replace LinUCB with EFE in CascadeRouter | `crates/roko-learn/src/routing/cascade.rs` | M | EFE |
| Wire regime Signal from roko-conductor into RouteContext | `crates/roko-conductor/` | S | Regime conditioning |

### 2.6 Dream Cycle Runtime Trigger (Loop 3)

**Goal**: The dream cycle runs automatically, not just when manually invoked.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Add CronTrigger for dream cycle | `crates/roko-dreams/src/trigger.rs` | S | -- |
| Wire dream trigger into `roko serve` startup | `crates/roko-serve/src/lib.rs` | S | Dream trigger |
| Add `roko knowledge dream schedule` CLI | Already exists, wire to trigger | S | Dream trigger |

### 2.7 Observe Protocol + 10 Lenses

**Goal**: Every Cell, Graph, Agent, and Space can have Lenses attached. Observation never modifies what it observes. StateHub projections consumed by all surfaces.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define `Observe` trait in roko-core | `crates/roko-core/src/traits.rs` | S | -- |
| Define `Lens` specialization struct | `crates/roko-core/src/lens.rs` | S | Observe trait |
| Implement 6 core Lenses (Agent, Plan, Gate, Router, Memory, Cost) | `crates/roko-conductor/src/lenses/` | M | Lens struct |
| Implement 4 additional Lenses (Health, Error, Throughput, Dream) | `crates/roko-conductor/src/lenses/` | M | Lens struct |
| Wire Lenses into TUI dashboard | `crates/roko-cli/src/tui/` | M | All Lenses |
| Wire Lenses into HTTP routes | `crates/roko-serve/src/routes/` | M | All Lenses |

### 2.8 Trigger and Connect Protocols

**Goal**: Declarative event-driven Graph firing. Formalize the connector pattern.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define `Trigger` trait in roko-core | `crates/roko-core/src/traits.rs` | S | -- |
| Implement CronTrigger, WebhookTrigger, SignalTrigger, ChainTrigger | Various crates | M | Trigger trait |
| Define `Connect` trait in roko-core | `crates/roko-core/src/traits.rs` | S | -- |
| Refactor existing connectors to implement Connect | `crates/roko-agent/src/connector.rs` | M | Connect trait |

### 2.9 TypeSchema Validation

**Goal**: All Cell inputs and outputs are schema-validated at load time and run time.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define `TypeSchema` enum (primitives, collections, named structs, unions) | `crates/roko-core/src/schema.rs` | M | -- |
| Implement validation at Graph-load time (type check edges) | `crates/roko-orchestrator/src/graph/` | M | TypeSchema |
| Implement runtime validation (check Cell outputs match schema) | `crates/roko-orchestrator/src/graph/` | S | TypeSchema |

---

## 3. Phase 2 — Graph Engine + Agent Runtime (Medium-Term)

Replace the current plan executor with a proper Graph engine. Introduce type-state Agent lifecycle, CognitiveWorkspace, StateHub, the 5 named surfaces, Rack, the 5-tier SPI, and Marketplace v1.

### 3.1 Hot Graph Engine

**Goal**: Graphs stay resident and re-fire per tick. The engine handles typed edges, conditional branching, retry strategies, snapshot/resume, and Hot Graph semantics.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define Graph TOML schema with typed edges | `crates/roko-orchestrator/src/graph/schema.rs` | M | Phase 1 TypeSchema |
| Implement Graph loader with full validation | `crates/roko-orchestrator/src/graph/loader.rs` | L | Graph schema |
| Implement Graph executor with state machine | `crates/roko-orchestrator/src/graph/executor.rs` | L | Graph loader |
| Add Flow (runtime Graph instance) with RunId | `crates/roko-orchestrator/src/graph/flow.rs` | M | Graph executor |
| Add Hot Flow variant (tick-driven, stays resident) | `crates/roko-orchestrator/src/graph/hot.rs` | M | Flow |
| Wire Graph executor into `plan run` | `crates/roko-cli/src/orchestrate.rs` | L | Flow |
| Snapshot/resume for Graph execution | `crates/roko-orchestrator/src/graph/snapshot.rs` | M | Flow |
| Migration tool: convert existing plans to Graph TOML | `crates/roko-cli/src/migrate.rs` | M | Graph loader |

### 3.2 Type-State Agent Lifecycle

**Goal**: Compile-time enforced Agent state transitions: `Provisioning -> Active <-> Dreaming -> Terminal`.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define type-state Agent struct: `Agent<Provisioning>`, `Agent<Active>`, etc. | `crates/roko-agent/src/lifecycle.rs` | M | -- |
| Implement vitality model: `remaining_budget / initial_budget` | `crates/roko-agent/src/vitality.rs` | S | Type-state Agent |
| Behavioral phases: Thriving (1.0-0.7) / Stable (0.7-0.4) / Conservation (0.4-0.2) / Declining (0.2-0.05) / Terminal (<0.05) | `crates/roko-agent/src/vitality.rs` | S | Vitality model |
| Multi-slot state: Agent manages N concurrent slots with shared limits | `crates/roko-agent/src/slots.rs` | M | Type-state Agent |

### 3.3 CognitiveWorkspace

**Goal**: Learnable context assembly via VCG auction with section effect tracking.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Implement CognitiveWorkspace with VCG auction | `crates/roko-compose/src/workspace.rs` | L | Phase 1 Compose |
| Implement section effect tracking (beta-distribution posteriors) | `crates/roko-compose/src/section_effects.rs` | M | CognitiveWorkspace |
| Implement 8+ context bidders | `crates/roko-compose/src/bidders/` | M | CognitiveWorkspace |
| Wire section effects into predict-publish-correct | `crates/roko-learn/src/calibration.rs` | S | Section effects + Bus |

### 3.4 StateHub Projections

**Goal**: Universal typed projections consumed by all surfaces.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define StateHub projection types | `crates/roko-core/src/statehub.rs` | M | -- |
| Implement StateHub from Lens outputs | `crates/roko-conductor/src/statehub.rs` | M | Phase 1 Lenses |
| Wire StateHub into TUI, HTTP, SSE/WebSocket | Various crates | M | StateHub |

### 3.5 Five Named Surfaces

**Goal**: Protocol-level UX primitives: Workbench, Agent Inbox, Generative Canvas, Stigmergy Minimap, Autonomy Slider.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define surface protocols (projections + events + invariants) | `crates/roko-core/src/surfaces.rs` | M | StateHub |
| Implement Workbench, Agent Inbox, Autonomy Slider in TUI | `crates/roko-cli/src/tui/` | L | Surface protocols |

### 3.6 Rack (Parameterized Graphs)

**Goal**: Graphs with exposed Macros (knobs) and Slots (jacks) for reusability.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define Rack struct (Graph + Macros + Slots) | `crates/roko-orchestrator/src/graph/rack.rs` | M | Graph schema |
| Implement Macro substitution at load time | `crates/roko-orchestrator/src/graph/macro_expand.rs` | M | Rack struct |
| Implement Slot binding | `crates/roko-orchestrator/src/graph/slot.rs` | S | Rack struct |

### 3.7 5-Tier SPI

**Goal**: The 5-tier package SPI (Prompts, Config, Declarative Tools, WASM, Rust) is fully implemented.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Tier 1: Prompt loader (Markdown + TOML front-matter) | `crates/roko-compose/src/prompts/` | S | -- |
| Tier 2: Config profile loader and deep merge | `crates/roko-cli/src/config/` | M | -- |
| Tier 3: Declarative tool loader (subprocess/HTTP/MCP) | `crates/roko-std/src/tools/` | M | -- |
| Tier 4: WASM Cell ABI + wasmtime runtime | `crates/roko-core/` + `crates/roko-runtime/` | L | -- |

### 3.8 Marketplace v1

**Goal**: Publish, install, and fork Graphs and Cells.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Cell manifest format (TOML) | `crates/roko-core/src/manifest.rs` | S | -- |
| Local Cell registry | `crates/roko-orchestrator/src/registry.rs` | M | Cell manifest |
| `roko marketplace publish/install/fork` CLI | `crates/roko-cli/src/marketplace.rs` | L | Local registry |
| Marketplace HTTP routes | `crates/roko-serve/src/routes/marketplace.rs` | M | Local registry |

---

## 4. Phase 3 — Autonomy, Safety, and Economy (Long-Term)

Full autonomous operation with CaMeL IFC, 5-head corrigibility, on-chain anchoring, arenas, brain export, and cross-agent knowledge sharing.

### 4.1 Learning Loop 4 (L4 Self-Evolution)

**Goal**: The system proposes structural changes to its own Graphs and Cells, subject to human approval.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define structural change proposals | `crates/roko-learn/src/structural.rs` | M | Phase 2 Graph engine |
| Implement approval workflow via Agent Inbox | `crates/roko-serve/src/routes/approvals.rs` | M | Structural proposals |
| Wire Loop 4 into dream cycle | `crates/roko-dreams/src/structural.rs` | L | Approval workflow |
| Clade-Metaproductivity (HGM): score variants by descendant performance | `crates/roko-learn/src/hgm.rs` | L | L4 proposals |

### 4.2 CaMeL IFC

**Goal**: Capability-tagged information flow control on Extensions. Extensions cannot launder capabilities.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define `CamelTag` types in roko-core | `crates/roko-core/src/camel.rs` | S | -- |
| Implement tag propagation in Extension hook dispatch | `crates/roko-agent/src/extensions/dispatch.rs` | M | CamelTag |
| Implement CaMeL monitor (Verify-protocol Cell) | `crates/roko-gate/src/camel_monitor.rs` | M | Tag propagation |
| Wire CaMeL into Extension loading validation | `crates/roko-agent/src/extensions/` | S | CaMeL monitor |

### 4.3 5-Head Corrigibility

**Goal**: Lexicographic safety ordering: deference > switch > truth > impact > task (Nayebi 2024). Implemented as a Verify-protocol chain where each head can veto.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Define 5-head corrigibility as Verify chain | `crates/roko-gate/src/corrigibility.rs` | L | Phase 2 Verify |
| Implement 5 Verify heads | `crates/roko-gate/src/corrigibility/` | L | Corrigibility chain |
| Implement RecursiveSafetyMonitor | `crates/roko-gate/src/recursive_safety.rs` | L | Corrigibility chain |
| Wire into Graph executor as mandatory wrapper | `crates/roko-orchestrator/src/graph/safety.rs` | M | RecursiveSafetyMonitor |

### 4.4 On-Chain Registry Deployment

**Goal**: Deploy 8 registries to Mirage.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Finalize Solidity contracts | `contracts/src/` | L | -- |
| Deploy to Mirage testnet | `contracts/deploy/` | M | Contracts |
| Implement Rust clients (alloy) | `crates/roko-chain/src/clients/` | L | Deployed contracts |
| Wire identity registration into Agent startup | `crates/roko-agent/src/identity.rs` | M | Identity client |
| Wire knowledge publication from neuro store | `crates/roko-neuro/src/publish.rs` | M | InsightStore client |
| Implement event indexer (chain -> PostgreSQL -> REST API) | `crates/roko-chain/src/indexer/` | L | Deployed contracts |

### 4.5 Arena System

**Goal**: Full arena, eval, and bounty market with the 7-step flywheel and 8 concrete arenas.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Arena types + registry | `crates/roko-chain/src/arena.rs` | L | On-chain contracts |
| 7-step flywheel pipeline | `crates/roko-learn/src/arena/` | L | Arena types |
| Cross-arena transfer detection via HDC fingerprint correlation | `crates/roko-primitives/src/transfer.rs` | M | Arena types + HDC |
| Arena API routes | `crates/roko-serve/src/routes/arenas.rs` | L | Arena types |
| VCG batch matching for bounties | `crates/roko-serve/src/routes/bounties.rs` | M | Bounty routes |

### 4.6 Brain Export and Import

**Goal**: Portable Agent knowledge via Merkle-CRDT merge (~100KB-1MB).

| Task | Target | Size | Depends On |
|---|---|---|---|
| Brain export format (manifest + knowledge + learning + episodes) | `crates/roko-neuro/src/brain/format.rs` | M | -- |
| Brain export with filters (min-tier, since, include-episodes) | `crates/roko-neuro/src/brain/export.rs` | M | Export format |
| Brain import with decay factor | `crates/roko-neuro/src/brain/import.rs` | M | Export format |
| Merkle tree over brain state | `crates/roko-neuro/src/brain/merkle.rs` | L | Export format |
| CRDT operations (GCounter, LWW-Register, Add-only set) | `crates/roko-neuro/src/brain/crdt.rs` | L | Merkle tree |
| Merkle-CRDT sync protocol | `crates/roko-neuro/src/brain/sync.rs` | L | CRDT ops |
| Wire into `roko knowledge backup/restore/sync` CLI | `crates/roko-cli/src/knowledge.rs` | M | Sync protocol |

### 4.7 Cross-Agent Knowledge Sharing

**Goal**: Agents share knowledge Signals through the relay and on-chain InsightStore.

| Task | Target | Size | Depends On |
|---|---|---|---|
| Knowledge Signal broadcast via relay | `crates/roko-runtime/src/knowledge_sync.rs` | M | Relay |
| Selective sync based on HDC similarity | `crates/roko-neuro/src/sync.rs` | M | HDC indexing |
| On-chain knowledge discovery via InsightStore | `crates/roko-chain/src/knowledge_discovery.rs` | M | InsightStore client |

---

## 5. Crate-to-Concepts Mapping

Complete mapping of every crate to the unified concepts it owns.

| Crate | Fundamentals | Protocols | Specializations | Loops |
|---|---|---|---|---|
| `roko-core` | Signal, Pulse, Cell, Graph (types), Bus (trait) | Store, Score, Verify, Route, Compose, React, Observe, Connect, Trigger (traits) | -- | -- |
| `roko-agent` | -- | Connect (LLM backends) | Agent (type-state lifecycle, 9-step pipeline, CaMeL tags) | -- |
| `roko-agent-server` | -- | -- | Agent (HTTP sidecar) | -- |
| `roko-orchestrator` | Graph (execution), Hot Graph | -- | Flow, Hot Flow, Rack | -- |
| `roko-gate` | -- | Verify (11 impl, CaMeL monitor, 5-head corrigibility) | -- | -- |
| `roko-compose` | -- | Compose (9 templates, VCG, CognitiveWorkspace, section effects) | -- | -- |
| `roko-learn` | -- | Score, Route (EFE), CalibrationPolicy (React) | Loop (Loops 1+2), Arena flywheel | L1, L2 |
| `roko-neuro` | Signal (knowledge, demurrage) | Store (knowledge tier) | Memory, Brain export/import | -- |
| `roko-dreams` | -- | -- | Loop (Loop 3), L4 structural proposals | L3 |
| `roko-conductor` | -- | Observe (watchers), StateHub | Lens | -- |
| `roko-runtime` | Pulse, Bus (BroadcastBus) | Trigger, Connect | Trigger, Connector | -- |
| `roko-primitives` | Signal (HDC), Heuristic kind | -- | -- | -- |
| `roko-daimon` | -- | React (affect modulation) | Extension | -- |
| `roko-serve` | -- | Observe (health) | Lens (HTTP), Surfaces | -- |
| `roko-cli` | -- | -- | Agent (CLI), Lens (TUI), Surfaces (TUI) | -- |
| `roko-fs` | -- | Store (JSONL) | -- | -- |
| `roko-std` | Cell (19 tools), Declarative tools (Tier 3) | -- | -- | -- |
| `roko-chain` | -- | Store (on-chain), Connect (chain) | Connector, Arena, Eval, Bounty | -- |
| `roko-index` | -- | Store (code graph) | -- | -- |
| `roko-mcp-code` | -- | Connect (MCP) | Connector (code intel) | -- |

---

## 6. Dependency Graph

```
Phase 0 (Current State)
    |
    v
Phase 1 (Kernel Upgrade)
    |-- 1.1 Pulse/Bus kernel
    |-- 1.2 Predict-publish-correct ----------> depends on 1.1
    |-- 1.3 Demurrage
    |-- 1.4 Heuristic kind
    |-- 1.5 EFE routing
    |-- 1.6 Dream cycle trigger
    |-- 1.7 Observe protocol + 10 Lenses
    |-- 1.8 Trigger + Connect protocols
    |-- 1.9 TypeSchema validation
    |
    v
Phase 2 (Graph Engine + Agent Runtime)
    |-- 2.1 Hot Graph engine -----------------> depends on 1.1, 1.7, 1.8, 1.9
    |-- 2.2 Type-state Agent
    |-- 2.3 CognitiveWorkspace --------------> depends on 1.2
    |-- 2.4 StateHub -------------------------> depends on 1.7
    |-- 2.5 Five named surfaces --------------> depends on 2.4
    |-- 2.6 Rack -----------------------------> depends on 2.1
    |-- 2.7 5-tier SPI
    |-- 2.8 Marketplace v1 ------------------> depends on 2.1, 2.7
    |
    v
Phase 3 (Autonomy + Safety + Economy)
    |-- 3.1 L4 self-evolution ----------------> depends on 2.1
    |-- 3.2 CaMeL IFC -----------------------> depends on 1.1
    |-- 3.3 5-head corrigibility -------------> depends on 3.2
    |-- 3.4 On-chain registries
    |-- 3.5 Arena system ---------------------> depends on 3.4, 1.2
    |-- 3.6 Brain export/import --------------> depends on 1.3
    |-- 3.7 Cross-agent knowledge ------------> depends on 3.4, 3.6
```

### Parallel Tracks

- Phase 1 tasks (1.1-1.9) are largely independent except 1.2 depends on 1.1.
- Phase 2.2, 2.3, 2.7 are independent of 2.1 (Graph engine).
- Phase 3.2 (CaMeL IFC) and 3.4 (on-chain registries) are independent and can start early.

### Critical Path

```
Phase 1.1 (Pulse/Bus) -> Phase 1.2 (predict-publish-correct)
    -> Phase 2.1 (Hot Graph) -> Phase 2.3 (CognitiveWorkspace)
    -> Phase 3.1 (L4 self-evolution)
```

This is the shortest path to full self-evolution capability.

---

## 7. Migration Strategy

### 7.1 Naming: Code vs Spec

The unified vocabulary applies at the spec and documentation level. Existing Rust code retains its current names for backward compatibility. Type aliases bridge the gap:

| Spec Name | Code Name | When to Rename | Bridge |
|---|---|---|---|
| Signal | `Engram` | Phase 2 (Graph engine migration point) | `type Signal = Engram;` |
| Pulse | `Envelope<E>` | Phase 1 (promote to first-class) | New struct replaces |
| Bus | `EventBus` | Phase 1 (promote to kernel trait) | New trait replaces |
| Cell | Module/trait impl | Phase 2 (Cell trait introduced) | New trait |
| Graph | Plan/tasks.toml | Phase 2 (Graph TOML replaces plan format) | Migration tool |
| Store | `Substrate` | Phase 1 (add type alias) | `type Store = Substrate;` |
| Score | `Scorer` | Phase 1 (add type alias) | `type Score = Scorer;` |
| Verify | `Gate` | Phase 1 (add type alias) | `type Verify = Gate;` |
| Route | `Router` | Phase 1 (add type alias) | `type Route = Router;` |
| Compose | `Composer` | Phase 1 (add type alias) | `type Compose = Composer;` |
| React | `Policy` | Phase 1 (note: now takes Pulses) | Breaking change |
| Observe | -- (new) | Phase 1 (new trait) | -- |
| Connect | `Connector` | Phase 1 (formalize existing) | -- |
| Trigger | -- (new) | Phase 1 (new trait) | -- |

### 7.2 Plan-to-Graph Migration

Existing `tasks.toml` plans are automatically convertible to Graph TOML:

```toml
# Old: tasks.toml
[[task]]
name = "implement-feature"
prompt = "..."
depends_on = ["research"]

# New: graph.toml
[[block]]
id = "implement-feature"
type = "agent"
config = { prompt = "..." }
inputs = [{ from = "research", edge = "output" }]
```

Migration CLI: `roko plan migrate <dir>`. Existing plan format continues to work during Phase 2.

### 7.3 React Protocol Breaking Change

The `React` (formerly `Policy`) protocol changes from operating on Signals to operating on **Pulses**. Migration path:

1. Phase 1: `React` trait defined with Pulse-based API alongside existing `Policy` trait.
2. Phase 1: Existing policies migrated incrementally.
3. Phase 2: `Policy` trait deprecated.

---

## 8. Success Criteria

### Phase 1 Complete When

- [ ] `Pulse` struct exists in roko-core as a first-class type alongside `Engram`
- [ ] `Bus` trait exists in roko-core alongside `Substrate`
- [ ] `BroadcastBus` implementation passes pub/sub integration tests
- [ ] Cell lifecycle events emitted as Pulses on Bus
- [ ] Predict-publish-correct: CalibrationPolicy joins predictions with outcomes via Bus
- [ ] Per-operator calibration state persists across restarts
- [ ] Demurrage: knowledge Signals have `balance` field that decays over time
- [ ] Demurrage reinforcement: retrieval and gate-pass restore balance
- [ ] Cold threshold: balance < 0.01 triggers archive
- [ ] `Kind::Heuristic` exists with when/then/falsifier/calibration
- [ ] Heuristic calibration from gate verdicts via Bus works end-to-end
- [ ] EFE routing replaces LinUCB in CascadeRouter
- [ ] Regime conditioning: Route receives regime Signal
- [ ] Dream cycle runs on a configurable schedule without manual invocation
- [ ] `Observe`, `Trigger`, and `Connect` traits exist in roko-core
- [ ] 10 Lenses implemented and wired into TUI + HTTP
- [ ] TypeSchema validation at Graph-load time
- [ ] All existing tests pass (no regressions)

### Phase 2 Complete When

- [ ] A Graph can be authored in TOML and executed via `roko plan run`
- [ ] Hot Graph stays resident and re-fires per tick
- [ ] Existing plans automatically convertible via `roko plan migrate`
- [ ] Type-state Agent enforces lifecycle transitions at compile time
- [ ] Vitality model drives behavioral phases (Thriving through Terminal)
- [ ] CognitiveWorkspace assembles context via VCG auction
- [ ] Section effect tracking correlates context sections with gate success
- [ ] StateHub projections consumed by TUI, HTTP, and SSE
- [ ] At least 3 of 5 named surfaces implemented in TUI
- [ ] Rack substitution works (Macros expanded, Slots bound)
- [ ] 5-tier SPI: all 5 tiers load and run correctly
- [ ] A Cell can be published to and installed from a local registry

### Phase 3 Complete When

- [ ] Loop 4 proposes a structural change and it is approved/rejected via Agent Inbox
- [ ] CaMeL IFC: capability tags propagate through Extension hooks, no laundering
- [ ] CaMeL monitor detects and flags capability tag violations
- [ ] 5-head corrigibility chain enforces lexicographic safety ordering
- [ ] RecursiveSafetyMonitor prevents a deliberately crafted safety bypass
- [ ] 8 registries deployed on Mirage
- [ ] An Agent registers ERC-8004 identity, publishes knowledge Signal, receives reputation attestation
- [ ] Arena 7-step flywheel runs end-to-end
- [ ] Meta-arena: PR merge rate, gate pass rate, cost per task measured and tracked
- [ ] Cross-arena transfer detected via HDC fingerprint correlation
- [ ] Brain export produces ~100KB-1MB portable file
- [ ] Brain import restores learning state with optional decay
- [ ] Merkle-CRDT sync: two instances converge after divergent learning
- [ ] Two Agents discover each other's knowledge Signals through the relay
- [ ] Variance Inequality enforced: L4 pauses when generator improves faster than verifier

---

## Version History

| Version | Date | Changes |
|---|---|---|
| 3.0 | 2026-04-26 | Unified spec: merged Phase 0 current state (from CLAUDE.md), Phase 1-3 tasks with sizes and dependencies, crate-to-concepts mapping, dependency graph with critical path, migration strategy with naming table, success criteria per phase. |
| 2.0 | 2026-04-22 | Roadmap v2 with Pulse/Bus kernel, predict-publish-correct, Hot Graph engine, type-state Agent. |
| 1.0 | 2026-04-15 | Initial roadmap from architecture review. |
