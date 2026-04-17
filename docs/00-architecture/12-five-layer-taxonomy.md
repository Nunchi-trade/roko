# The Five-Layer Taxonomy

> **Abstract:** Roko's crates are organized into five architectural layers with strictly
> downward dependencies. This document specifies each layer, maps the six Synapse traits
> to their layer assignments, describes the dependency rules, and provides the complete
> layer diagram. L0 is the two-fabric runtime/kernel surface: `Substrate` is the storage
> fabric for durable Engrams, and the `Bus` trait is the transport fabric for ephemeral
> Pulses. `Topic` and `TopicFilter` live at this surface, and Bus replay is bounded by the
> ring. The five layers map to Beer's Viable System Model (Beer 1972) and provide the
> structural skeleton for all 18+ crates. See
> also `tmp/refinements/03-bus-as-first-class.md` and
> [01-naming-and-glossary.md](./01-naming-and-glossary.md).


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
│  Process lifecycle, Substrate, Bus, I/O, clock       │
└──────────────────────────────────────────────────────┘

  COGNITIVE CROSS-CUTS (injected into multiple layers):
  Neuro (knowledge) | Daimon (motivation) | Dreams (offline learning)
  + Inference Optimization | Safety & Provenance | Observability & Telemetry
```

**Dependencies flow STRICTLY downward.** Layer 4 may depend on Layer 3, never the reverse.
Cross-cutting concerns are injected via trait objects, never via direct imports of higher
layers. Higher-layer communication should flow through `Substrate` and/or `Bus`, not
through direct crate coupling.

---

## 2. Layer 0: Runtime

**Purpose**: Process lifecycle, the two-fabric kernel surface, supervision, cancellation,
I/O, adaptive clock.

**Key Crates**:
- `roko-core` — `Substrate`, `Bus`, `Topic`, `TopicFilter`, and shared kernel traits
- `roko-primitives` — HDC vectors, Hamming similarity, inference tiers, shared primitive types
- `roko-runtime` — process supervision, cancellation tokens, adaptive clock, Bus-backed lifecycle

**What Lives Here**:
- Process spawning and lifecycle management (`ProcessSupervisor`)
- `Substrate` for durable Engram persistence and query
- `Bus` for topic-addressed Pulse transport and bounded replay; it is the transport fabric paired with `Substrate` at L0
- `Topic` as the routing handle for Pulse publication and subscription
- `TopicFilter` as the subscription and replay selector used by Bus consumers
- Cancellation tokens and graceful shutdown
- The adaptive clock that manages Gamma/Theta/Delta frequencies
- Basic I/O primitives

**Synapse Traits at L0**: `Substrate`, `Bus` (persistence and transport are runtime concerns)

This is the kernel two-fabric surface: durable state stays on `Substrate`, live coordination
stays on `Bus`, and higher layers communicate through `Topic` / `TopicFilter` instead of
direct crate imports.

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
| **Substrate** | FileSubstrate, MemorySubstrate, HdcSubstrate, ChainSubstrate | — | — | — | — |
| **Bus** | BroadcastBus, MemoryBus | — | — | — | — |
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
// Example: higher-layer knowledge and live coordination injected through fabrics.
fn compose_with_knowledge(
    composer: &dyn Composer,
    knowledge: &dyn Substrate,
    bus: &dyn Bus,
    budget: &Budget,
    scorer: &dyn Scorer,
    ctx: &Context,
) -> Result<Engram> {
    // Retrieve relevant durable knowledge.
    let knowledge_engrams = knowledge
        .query(&Query::of_kind(Kind::Insight).limit(5), ctx)
        .await?;
    // Retrieve live coordination without importing a higher layer directly.
    let recent_pulses = bus
        .replay_since(ctx.checkpoint_seq, &TopicFilter::Glob("gate.verdict.*".into()))
        .await?;
    composer.compose_with(knowledge_engrams, recent_pulses, budget, scorer, ctx)
}
```

### 9.3 Why This Matters

Strict layering prevents circular dependencies, ensures each layer can be tested
independently, and allows layer-level replacement. A team could replace the entire L1
Framework (e.g., switching LLM backends) without touching L0, L2, L3, or L4. When a
higher layer needs durable state or live coordination, it should talk through
`Substrate` and/or `Bus`, not by importing peer or lower-layer crates directly.

---

## 10. The 18-Crate Map by Layer

| Layer | Crate | Status | Purpose |
|---|---|---|---|
| **Runtime (L0)** | `roko-core` | Built (376 tests) | Engram, Substrate, Bus, and 6 Synapse traits |
| **Runtime (L0)** | `roko-primitives` | Built | HDC vectors, Hamming similarity, shared types |
| **Runtime (L0)** | `roko-runtime` | Built | Process supervision, cancellation, Bus-backed lifecycle, adaptive clock |
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

## 11. Dependency Violation Audit (2026-04-12)

An analysis of actual Cargo.toml dependencies across all 28 crates reveals the following
layer violations:

### 11.1 Confirmed Violations

| Violation | Severity | Description |
|---|---|---|
| `roko-conductor` (L3/L4) → `roko-learn` (L2/Cross-cut) | **Medium** | Conductor imports learning types for circuit breaker state tracking. It should instead subscribe to Bus topics such as `gate.failure.rate` and `gate.verdict.emitted` on the L0 Bus trait, dissolving the direct dependency. |
| `roko-agent` (L1) → `roko-learn` (L2/Cross-cut) [dev-dependency] | **Low** | Framework layer test code depends on Scaffold layer. Move tests to integration test crate. |

### 11.2 Unclassified Crates

Six crates exist outside the formal 5-layer taxonomy and need official classification:

| Crate | Actual Dependencies | Recommended Layer |
|---|---|---|
| `roko-neuro` | roko-core (L0), roko-fs (L0), roko-agent (L1), roko-learn (L2) | **Cross-cut** (bridges L0-L2; inject via trait objects) |
| `roko-daimon` | roko-core (L0) | **Cross-cut** (inject via trait objects; no violations) |
| `roko-dreams` | roko-agent, roko-neuro, roko-learn | **Cross-cut** (depends on multiple layers; inject via trait objects) |
| `agent session runtime` | roko-core (L0) | **Phase 2+** (umbrella for future agent subsystems) |
| `roko-chain` | roko-core (L0) | **Domain plugin** (L1 equivalent for chain domain) |
| `roko-plugin` | roko-core (L0) | **L1 Framework** (plugin SDK) |

### 11.3 Remediation Plan

1. **Immediate**: Move the shared failure-rate stream onto `Bus` topics in `roko-core`
   (L0), so `roko-conductor` can subscribe without importing `roko-learn` directly.
   This removes the L3→L2 violation by routing the shared state through the L0 Bus trait
   instead of a compile-time crate dependency.
2. **Short-term**: Move `roko-agent` dev-dependency tests into a dedicated integration
   test crate at L4 (where cross-layer dependencies are expected).
3. **Medium-term**: Add all unclassified crates to this document's layer map with explicit
   dependency rules.

### 11.4 Healthy Patterns Observed

- **L0 crates** (`roko-core`, `roko-runtime`, `roko-primitives`, `roko-fs`, `roko-std`):
  Zero upward dependencies. Clean.
- **L1 crates** (`roko-agent`, `roko-index`, `roko-lang-*`): Depend only on L0. Clean
  (except dev-dependency noted above).
- **L4 crate** (`roko-cli`): Depends on all layers. Expected for the entry-point binary.
- **MCP crates** (`roko-mcp-*`): Zero internal dependencies. Clean utility layer.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Beer 1972, Brain of the Firm | Viable System Model: 5 recursive subsystems for viable organizations. |
| Ashby 1956, An Introduction to Cybernetics | Law of Requisite Variety: motivates compositional layer design. |
| Ousterhout 2018, A Philosophy of Software Design | Information hiding and deep module design. |
| Parnas 1972, CACM 15(12) | Information hiding: modules hide design decisions. Each layer hides its implementation. |
| Hocking & Hetherington 2024 | Layer violation detection in modular monolith architectures: automated tooling for enforcing dependency rules. |

---

## Current Status and Gaps

All layers have built crates with passing tests. The primary gaps are:
- **Cognitive cross-cuts**: `roko-dreams` is scaffold-only.
- **Chain layer**: Not yet integrated into the main cognitive loop.
- **Layer violation**: `roko-conductor` (L3/L4) has a direct dependency on `roko-learn` (L2/Cross-cut).
  It should be dissolved by moving the shared state onto Bus topics rather than direct imports.
  See Section 11 for remediation plan.
- **Unclassified crates**: Six crates need formal layer assignment (Section 11.2).

---

## Cross-References

- [00-vision-and-thesis.md](00-vision-and-thesis.md) — Why five layers
- [01-naming-and-glossary.md](./01-naming-and-glossary.md) — Canonical naming map and glossary
- [06-synapse-traits.md](06-synapse-traits.md) — Traits distributed across layers
- [07-substrate-trait.md](07-substrate-trait.md) — Substrate deep dive and kernel fabric details
- [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md) — Cross-cuts injected into layers
- [15-crate-map.md](15-crate-map.md) — Full crate inventory
- [23-architectural-analysis-improvements.md](23-architectural-analysis-improvements.md) — Full architectural analysis
- `tmp/refinements/03-bus-as-first-class.md` — Refinement source for the Bus kernel fabric
