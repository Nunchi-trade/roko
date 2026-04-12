# The Five-Layer Taxonomy

> **Abstract:** Roko's crates are organized into five architectural layers with strictly
> downward dependencies. This document specifies each layer, maps the six Synapse traits
> to their layer assignments, describes the dependency rules, and provides the complete
> layer diagram. The five layers map to Beer's Viable System Model (Beer 1972) and provide
> the structural skeleton for all 18+ crates.


> **Implementation**: Shipping

---

## 1. The Layer Diagram

```
┌──────────────────────────────────────────────────────┐
│                   Applications                       │
│  (coding agent, chain agent, research agent, custom) │
├──────────────────────────────────────────────────────┤
│  Layer 4: ORCHESTRATION                              │
│  DAGs, scheduling, state machines, multi-agent coord │
├──────────────────────────────────────────────────────┤
│  Layer 3: HARNESS                                    │
│  Gates, conductor, monitoring, interventions, eval   │
├──────────────────────────────────────────────────────┤
│  Layer 2: SCAFFOLD                                   │
│  Context engineering, prompts, enrichment, memory    │
├──────────────────────────────────────────────────────┤
│  Layer 1: FRAMEWORK                                  │
│  Connections, roles, tools, model routing, safety    │
├──────────────────────────────────────────────────────┤
│  Layer 0: RUNTIME                                    │
│  Process lifecycle, events, supervision, I/O, clock  │
└──────────────────────────────────────────────────────┘

  COGNITIVE CROSS-CUTS (injected into multiple layers):
  Neuro (knowledge) | Daimon (motivation) | Dreams (offline learning)
  + Inference Optimization | Safety & Provenance | Observability & Telemetry
```

**Dependencies flow STRICTLY downward.** Layer 4 may depend on Layer 3, never the reverse.
Cross-cutting concerns are injected via trait objects, never via direct imports of higher
layers.

---

## 2. Layer 0: Runtime

**Purpose**: Process lifecycle, event bus, supervision, cancellation, I/O, adaptive clock.

**Key Crates**:
- `roko-primitives` — HDC vectors, Hamming similarity, inference tiers, shared primitive types
- `roko-runtime` — Event bus, process supervision, cancellation tokens, adaptive clock

**What Lives Here**:
- Process spawning and lifecycle management (`ProcessSupervisor`)
- Event bus for inter-component communication
- Cancellation tokens and graceful shutdown
- The adaptive clock that manages Gamma/Theta/Delta frequencies
- Basic I/O primitives

**Synapse Traits at L0**: `Substrate` (persistence is a runtime concern)

**Beer VSM Mapping**: System 1 (Operations) — the primary activities of the organization.

---

## 3. Layer 1: Framework

**Purpose**: Connections to external systems (LLMs, tools, MCP), roles, model routing, safety.

**Key Crates**:
- `roko-agent` — Five LLM backends, connection pools, MCP client, tool dispatch loop, safety
- `roko-std` — Default trait implementations, 19 built-in tools, mock dispatcher

**What Lives Here**:
- LLM backend connections (Claude, OpenAI, local models, Ollama, ExecAgent)
- Tool registry and tool dispatch
- MCP (Model Context Protocol) client for external tool integration
- Model routing logic (CascadeRouter decisions propagate here)
- Safety layer (role authorization, pre/post-execution checks)

**Synapse Traits at L1**: `Router` (model/tool selection), `Scorer` (tool relevance)

**Beer VSM Mapping**: System 2 (Coordination) — anti-oscillation, ensuring components work
together without conflict.

---

## 4. Layer 2: Scaffold

**Purpose**: Context engineering, prompt assembly, enrichment, memory access.

**Key Crates**:
- `roko-compose` — Prompt assembly, 9 templates, SystemPromptBuilder, context enrichment

**What Lives Here**:
- SystemPromptBuilder (6-layer prompt assembly with role templates)
- Prompt templates for different agent roles (coder, researcher, planner, etc.)
- Context enrichment (injecting relevant knowledge, history, and tool descriptions)
- Token budget management within prompts

**Synapse Traits at L2**: `Scorer` (relevance for context selection), `Composer` (prompt assembly)

**Beer VSM Mapping**: System 3 (Control) — resource allocation and internal management.

---

## 5. Layer 3: Harness

**Purpose**: Verification, monitoring, interventions, evaluation.

**Key Crates**:
- `roko-gate` — 11+ verification gates, 6-rung pipeline, adaptive thresholds
- `roko-fs` — JSONL substrate persistence, garbage collection, file layout

**What Lives Here**:
- Gate pipeline (compile, test, clippy, diff, format, schema, judge, simulation)
- Adaptive gate thresholds (EMA-based)
- FileSubstrate (JSONL persistence)
- Monitoring and health checks

**Synapse Traits at L3**: `Gate` (verification), `Policy` (conductor watchers, circuit breakers)

**Beer VSM Mapping**: System 3* (Audit) — monitoring and verification of operations.

---

## 6. Layer 4: Orchestration

**Purpose**: Plan DAGs, parallel execution, state machines, multi-agent coordination.

**Key Crates**:
- `roko-orchestrator` — Plan DAG execution, parallel task runner, merge queue, worktrees
- `roko-conductor` — Reactive watchers, circuit breakers, health monitoring

**What Lives Here**:
- Plan discovery and DAG construction
- Parallel task execution with dependency ordering
- State machine for plan phases (Pending → Running → Gated → Complete)
- Session persistence and resumption
- Merge queue for coordinating concurrent agents
- Worktree management for parallel code modifications

**Synapse Traits at L4**: `Policy` (state machine transitions, plan reactions)

**Beer VSM Mapping**: System 4 (Intelligence) — environmental scanning and adaptation.

---

## 7. Cognitive Cross-Cuts

Cross-cutting concerns are injected across multiple layers rather than living at any single
level. They are provided as trait objects, never as direct imports:

| Cross-Cut | Crate | Injected Into | Role |
|---|---|---|---|
| **Neuro** | `roko-neuro` | L2 (context), L3 (knowledge gates), L4 (planning) | Knowledge management, tier decay |
| **Daimon** | `roko-daimon` | L0 (clock), L1 (routing), L2 (context bidding) | Motivation, PAD vector, behavioral states |
| **Dreams** | `roko-dreams` | L0 (scheduling), Neuro (consolidation) | Offline learning, hypothesis generation |
| **Learning** | `roko-learn` | L1 (routing), L3 (gate thresholds), L4 (plan adaptation) | Episodes, playbooks, bandits |
| **Safety** | `roko-agent/safety` | L1 (dispatch), L3 (gates) | Role auth, taint tracking |
| **Observability** | `roko-core/obs` | All layers | Metrics, tracing, telemetry |

---

## 8. Trait × Layer Map

| Trait | L0 Runtime | L1 Framework | L2 Scaffold | L3 Harness | L4 Orchestration |
|---|---|---|---|---|---|
| **Substrate** | FileSubstrate, MemorySubstrate | — | — | — | — |
| **Scorer** | — | ToolRelevanceScorer | RelevanceScorer, RecencyScorer | — | — |
| **Gate** | — | — | — | CompileGate, TestGate, etc. | — |
| **Router** | — | CascadeRouter, LinUCBRouter | — | — | — |
| **Composer** | — | — | SystemPromptBuilder, ContextComposer | — | PlanComposer |
| **Policy** | — | — | — | CircuitBreakerPolicy, ConductorPolicy | PlanPhasePolicy |

---

## 9. Dependency Rules

### 9.1 Strict Downward Dependencies

```
L4 depends on → L3, L2, L1, L0
L3 depends on → L2, L1, L0
L2 depends on → L1, L0
L1 depends on → L0
L0 depends on → (nothing above)
```

### 9.2 Cross-Cut Injection

Cross-cutting crates are NOT layer-bound. They are injected as `&dyn Trait` objects:

```rust
// Example: Neuro knowledge injected into Composer via trait object
fn compose_with_knowledge(
    composer: &dyn Composer,
    knowledge: &dyn Substrate, // Neuro's Substrate implementation
    signals: &[Signal],
    budget: &Budget,
    scorer: &dyn Scorer,
    ctx: &Context,
) -> Result<Signal> {
    // Retrieve relevant knowledge
    let knowledge_signals = knowledge.query(
        &Query::of_kind(Kind::Insight).limit(5),
        ctx,
    ).await?;
    // Combine task signals with knowledge
    let all = [signals, &knowledge_signals].concat();
    composer.compose(&all, budget, scorer, ctx)
}
```

### 9.3 Why This Matters

Strict layering prevents circular dependencies, ensures each layer can be tested
independently, and allows layer-level replacement. A team could replace the entire L1
Framework (e.g., switching LLM backends) without touching L0, L2, L3, or L4.

---

## 10. The 18-Crate Map by Layer

| Layer | Crate | Status | Purpose |
|---|---|---|---|
| **Runtime (L0)** | `roko-primitives` | Built | HDC vectors, Hamming similarity, shared types |
| **Runtime (L0)** | `roko-runtime` | Built | Event bus, supervision, cancellation, adaptive clock |
| **Kernel** | `roko-core` | Built (376 tests) | Engram + 6 Synapse traits |
| **Framework (L1)** | `roko-std` | Built (96 tests) | Default trait impls, 19 built-in tools |
| **Framework (L1)** | `roko-agent` | Built (346 tests) | LLM backends, tool dispatch, MCP client |
| **Scaffold (L2)** | `roko-compose` | Built (23 tests) | Prompt assembly, context engineering |
| **Harness (L3)** | `roko-gate` | Built (200 tests) | Verification pipeline (11+ gates) |
| **Harness (L3)** | `roko-fs` | Built (37 tests) | JSONL substrate persistence |
| **Orchestration (L4)** | `roko-orchestrator` | Built (158 tests) | Plan DAG, parallel executor, worktrees |
| **Orchestration (L4)** | `roko-conductor` | Built | Reactive watchers, circuit breakers |
| **Cognitive** | `roko-learn` | Built (101 tests) | Episodes, playbooks, skills, bandits |
| **Cognitive** | `roko-neuro` | Built | Knowledge store, tier progression, HDC |
| **Cognitive** | `roko-daimon` | Built | Affect/motivation (PAD vectors) |
| **Cognitive** | `roko-dreams` | Scaffold | Offline learning, consolidation, hypnagogia |
| **Chain** | `roko-chain` | Built (52 tests) | ChainClient/ChainWallet, chain witness |
| **Plugin** | `roko-index` | Built | Code parsing, symbol graphs, HDC fingerprints |
| **Lang** | `roko-lang-{rust,ts,go}` | Built | Language-specific support |
| **CLI** | `roko-cli` | Built (38 tests) | User-facing binary |
| **App** | `mirage-rs` | Built (141 tests) | In-process EVM simulator |

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Beer 1972, Brain of the Firm | Viable System Model: 5 recursive subsystems for viable organizations. |
| Ashby 1956, An Introduction to Cybernetics | Law of Requisite Variety: motivates compositional layer design. |
| Ousterhout 2018, A Philosophy of Software Design | Information hiding and deep module design. |
| Parnas 1972, CACM 15(12) | Information hiding: modules hide design decisions. Each layer hides its implementation. |

---

## Current Status and Gaps

All layers have built crates with passing tests. The primary gaps are in the Cognitive
cross-cuts (`roko-dreams` is scaffold-only) and the Chain layer (not yet integrated into
the main cognitive loop).

---

## Cross-References

- [00-vision-and-thesis.md](00-vision-and-thesis.md) — Why five layers
- [06-synapse-traits.md](06-synapse-traits.md) — Traits distributed across layers
- [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md) — Cross-cuts injected into layers
- [15-crate-map.md](15-crate-map.md) — Full crate inventory
