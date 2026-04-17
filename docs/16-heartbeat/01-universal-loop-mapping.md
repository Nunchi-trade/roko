# Universal Loop Mapping: CoALA → Synapse

> How the historical CoALA 9-step pipeline maps onto Roko's canonical seven-step universal loop - the domain-agnostic version every agent executes. See `tmp/refinements/05-loop-retold.md` and `docs/00-architecture/01-naming-and-glossary.md`.


> **Implementation**: Specified

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md), [00-architecture](../00-architecture/INDEX.md)
**Key sources**: `refactoring-prd/01-synapse-architecture.md` §3, `refactoring-prd/08-translation-guide.md` §7, `tmp/refinements/05-loop-retold.md`

---

## Abstract

The CoALA 9-step pipeline (OBSERVE → RETRIEVE → ANALYZE → GATE → SIMULATE → VALIDATE → EXECUTE → VERIFY → REFLECT) is the legacy framing that guided Roko's initial cognitive architecture. Roko's canonical universal loop is now the seven-step SENSE → ASSESS → COMPOSE → ACT → VERIFY → PERSIST + BROADCAST → REACT loop.

The universal Synapse loop is not a different pipeline. It is the same runtime expressed in terms of the composable Synapse traits (Substrate, Bus, Scorer, Gate, Router, Composer, Policy) plus the cognitive cross-cuts. The CoALA heartbeat is the theoretical frame; the Synapse loop is the implementation frame. Domain-specific heartbeat variants (chain, coding, research) are parameterizations of the universal loop, not separate architectures.

This document provides the complete side-by-side mapping between CoALA and Synapse, explains why the translation is not one-to-one but structurally faithful, and describes how domain-specific variants extend the universal loop without modifying it.

---

## Side-by-Side Mapping Table

The CoALA 9-step pipeline and the universal Synapse loop are structurally related but differ in granularity and naming. The table below shows the current canonical correspondence. Historical CoALA names are retained only where they clarify lineage.

| Historical framing | Canonical loop landing | Synapse trait(s) | Layer | Description |
|---|---|---|---|---|
| OBSERVE + RETRIEVE | **SENSE** | `Substrate.query()`, `Bus.subscribe()` | L0 Runtime | Read durable Engrams, live Pulses, and external I/O. The canonical loop makes all three sense sources explicit rather than hiding them inside storage-only observation. |
| ANALYZE + GATE | **ASSESS** | `Scorer.score()`, `Router.select()` | L1/L2 | Score candidates, compute surprise/confidence, and choose the next action or tier in one joint step. |
| _(implicit in historical retrieval/reasoning flow)_ | **COMPOSE** | `Composer.compose()` | L2 Scaffold | Build the prompt or action bundle under budget. This was previously implicit; the canonical loop makes context assembly a first-class step. |
| EXECUTE | **ACT** | `Agent.execute()` | L1 Framework | Execute the selected action: LLM, tool, or chain call. This is where live Pulses are emitted and final Engrams are produced. |
| SIMULATE + VALIDATE + VERIFY | **VERIFY** | `Gate.verify()`, domain gates, stream-gates | L3 Harness | Pre-flight checks, safety checks, stream-gates, and ground-truth verification all land in VERIFY. Domain-specific injections live here rather than adding universal steps. |
| _(historical persistence folded into REFLECT)_ | **PERSIST** | `Substrate.put()` | L0 Runtime | Store durable Engrams with lineage and provenance. |
| _(historically under-described)_ | **BROADCAST** | `Bus.publish()` | L0 Runtime | Publish ephemeral Pulses for subscribers. `BROADCAST` is co-equal with `PERSIST`, not a side effect hidden inside another step. |
| REFLECT | **REACT** | `Policy.decide()` | L3-L4 Harness/Orchestration | Observe the new Engram and Pulse stream, then emit follow-on Pulses and Engrams. Cross-cut self-regulation is injected around this step rather than modeled as a ninth stage. |

### Key Differences

1. **COMPOSE is explicit.** CoALA assumes context is assembled somehow; Synapse makes it a first-class step with a dedicated trait (`Composer`). This reflects Roko's thesis that context engineering is the primary determinant of agent performance (Meta-Harness, Lee et al. 2026, arXiv:2603.28052).

2. **PERSIST and BROADCAST are both explicit.** CoALA lumps persistence into REFLECT. Synapse separates durable storage from transport because content-addressed Engram storage and topic-addressed Pulse delivery are both foundational concerns. That separation keeps the Bus visible and makes live delivery first-class.

3. **REACT absorbs the old REFLECT tail.** CoALA's REFLECT covered both reactive adaptation and meta-cognitive assessment. In the canonical loop, the reactive part becomes REACT, while self-assessment is handled by injected cross-cuts rather than a ninth step.

4. **SIMULATE/VALIDATE are domain-specific extensions.** The universal loop does not add extra steps for them. They are injected by domain-specific agent types as part of VERIFY, not as separate universal phases.

---

## The Synapse Loop Traverses All Layers

A single tick of the cognitive loop crosses the full five-layer stack. This is the key architectural insight - the traits are distributed across layers, not concentrated at one level:

```
L0 Runtime     ──→ Substrate.query()      [SENSE: fetch from storage]
L0 Runtime     ──→ Bus.subscribe()        [SENSE: listen for live Pulses]
L2 Scaffold    ──→ Scorer.score()         [ASSESS: score relevance]
L1 Framework   ──→ Router.select()        [ASSESS: choose cognitive tier]
L2 Scaffold    ──→ Composer.compose()     [COMPOSE: build context window]
L1 Framework   ──→ Agent.execute()        [ACT: call LLM backend]
L3 Harness     ──→ Gate.verify()          [VERIFY: check against ground truth]
L0 Runtime     ──→ Substrate.put()        [PERSIST: store with lineage]
L0 Runtime     ──→ Bus.publish()         [BROADCAST: deliver Pulses]
L3-L4 Harness  ──→ Policy.decide()        [REACT: detect patterns, emit reactions]
Cross-cut      ──→ Daimon.assess()        [injected into ASSESS/ACT rather than a loop step]
```

Every tick traverses L0 → L2 → L1 → L2 → L1 → L3 → L0 → L0 → L3-L4 with cross-cut injections, not a ninth meta-cognition step. Dependencies flow strictly downward. Cross-cutting concerns (Neuro, Daimon, Dreams) are injected via trait objects, never via direct imports of higher layers.

---

## Domain Parameterization

The universal loop is one loop, parameterized by domain. Domain-specific behavior comes from domain-specific trait implementations and configuration - not from architectural modifications.

### Coding Agent

Uses the universal loop as-is. No additional steps between ASSESS and ACT.

```
SENSE     → FileSubstrate.query() + Bus.subscribe() [read codebase state and live Pulses]
ASSESS    → RecencyScorer + ReputationScorer + CascadeRouter [rank and route]
COMPOSE   → PromptComposer [code-aware templates, U-shape placement]
ACT       → Agent.execute() [call Claude/GPT, produce code changes]
VERIFY    → CompileGate → TestGate → ClippyGate → DiffGate
PERSIST   → FileSubstrate.put() [store with lineage]
BROADCAST → Bus.publish() [emit Pulses for subscribers]
REACT     → EpisodePolicy + DaimonPolicy + PredictionPolicy
```

### Chain Agent

Adds SIMULATE (mirage-rs pre-flight) and VALIDATE (position limits) inside VERIFY rather than as standalone universal steps. See [02-chain-heartbeat-variant.md](./02-chain-heartbeat-variant.md) for the full mapping.

```
SENSE     → FileSubstrate.query() + ChainSubstrate.query() + Bus.subscribe() [market, on-chain state, live Pulses]
ASSESS    → RecencyScorer + CatalystScorer + PredictiveScorer + Router.select() [multi-factor]
COMPOSE   → AttentionAuction + PromptComposer [VCG bidding for context budget]
ACT       → Agent.execute() [submit transaction, invoke tools]
VERIFY    → VerifyChainGate + TxSimGate + WalletGate [blockchain receipt, pre-flight, limits]
PERSIST   → FileSubstrate.put() + ChainSubstrate.put() [on-chain + off-chain]
BROADCAST → Bus.publish() [topic-addressed live updates]
REACT     → EpisodePolicy + DaimonPolicy + PredictionPolicy + CFactorPolicy
```

### Research Agent

Simplified loop - SIMULATE and VALIDATE are typically skipped because they are injected into VERIFY only when needed.

```
SENSE     → MemorySubstrate.query() + Bus.subscribe() [current research state and live signals]
ASSESS    → RecencyScorer + Router.select() [rank and route]
COMPOSE   → PromptComposer [research-focused templates, large budget]
ACT       → Agent.execute() [web search, synthesis, citation tracking]
VERIFY    → LlmJudgeGate [subjective quality check]
PERSIST   → MemorySubstrate.put() [store findings]
BROADCAST → Bus.publish() [emit research Pulses]
REACT     → EpisodePolicy + DaimonPolicy [react and re-evaluate]
```

---

## The Translation from Legacy Architecture

The legacy system had a domain-specific heartbeat that influenced the current mapping. The refactoring-prd translation guide (`08-translation-guide.md` §7) still helps as historical context, but the canonical translation now looks like this:

```
Historical pipeline:              Canonical universal loop:
1. Observe
2. Retrieve                    →  SENSE

3. Analyze
4. Gate                        →  ASSESS

(implicit context assembly)     →  COMPOSE

5. Simulate
6. Validate
7. Execute
8. Verify                      →  ACT + VERIFY

9. Reflect                     →  PERSIST + BROADCAST + REACT
```

The key change: the old heartbeat was a monolithic chain-specific pipeline. The canonical universal loop is a composable, domain-agnostic architecture where each step is a pluggable trait implementation, and where broadcast is co-equal with persist. This enables:

- **Cross-domain agents**: A single agent can write Solidity contracts (coding domain), simulate deployment (chain domain), and research competing protocols (research domain) — all using the same universal loop with domain-specific trait implementations.
- **New domains without kernel changes**: Adding medical, legal, or scientific domains requires implementing domain-specific traits (Gates, Scorers, Probes) but no modifications to the universal loop.
- **Composable verification**: Gates from different domains can be chained: `CompileGate → TestGate → TxSimGate → VerifyChainGate` for cross-domain agents.

---

## Formal Relationship: CoALA as Theory, Synapse as Implementation

CoALA (Sumers et al. 2023) is the theoretical framework. The Synapse loop is the concrete implementation. They are related as:

| Dimension | CoALA | Synapse Loop |
|---|---|---|
| **Level** | Theoretical taxonomy | Concrete trait-based implementation |
| **Composability** | Describes phases | Each phase is a pluggable trait implementation |
| **Domain** | Agnostic (by design) | Agnostic (by implementation) |
| **Memory** | "Internal actions: retrieval, reasoning, learning" | Explicit Substrate + Neuro + Scorer chain |
| **Verification** | "Grounding actions" | Explicit Gate pipeline with domain-specific injections |
| **Affect** | Not modeled | Daimon cross-cut injected into ASSESS and ACT |
| **Multi-scale** | Single cycle | Three concurrent scales (Gamma/Theta/Delta) |
| **Cost optimization** | Not modeled | T0/T1/T2 gating (~80% free ticks) |
| **Meta-cognition** | Part of "learning" | Cross-cut regulation injected into ASSESS, ACT, and speed selection rather than a separate step |

CoALA provides the intellectual justification. The Synapse loop provides the engineering realization. The two are not in tension — they are layers of the same architecture.

---

## Academic Foundations

- **Sumers, Yao, Narasimhan & Griffiths 2023** — "Cognitive Architectures for Language Agents" (arXiv:2309.02427). The CoALA framework.
- **Lee et al. 2026** — "Meta-Harness: Optimizing Agent Scaffolds" (arXiv:2603.28052). Evidence that scaffold optimization matters more than model selection.
- **Conant & Ashby 1970** — "Every Good Regulator of a System Must Be a Model of That System" (International Journal of Systems Science 1(2)). The Good Regulator Theorem — justifies explicit self-modeling and cross-cut regulation.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Prediction error as the organizing signal for cognitive architecture.

---

## Current Status and Gaps

**What exists:**
- The orchestration loop in `roko-cli/src/orchestrate.rs` implements a simplified subset of the canonical heartbeat centered on sensing from durable state, routing work, acting, verifying, and persisting outcomes.
- All six Synapse traits are defined in `roko-core/src/traits.rs`.
- Multiple implementations exist for each trait across the crate ecosystem.

**What is missing:**
- Full three-source `SENSE` wiring that combines Substrate reads, Bus subscriptions, and external I/O into one canonical entry point.
- The full `ASSESS` path that unifies multi-factor scoring, routing, and live Pulse awareness.
- The explicit `COMPOSE` path with VCG attention auction across all callers (see `refactoring-prd/09-innovations.md` §II).
- First-class `BROADCAST` / policy-driven `REACT` plumbing across the orchestration code.

---

## Cross-References

- See [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md) for the CoALA theoretical foundation
- See [02-chain-heartbeat-variant.md](./02-chain-heartbeat-variant.md) for the chain domain extension
- See [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md) for the VCG COMPOSE mechanism
- See `tmp/refinements/05-loop-retold.md` for the seven-step retelling
- See [Naming and Glossary](../00-architecture/01-naming-and-glossary.md) for canonical heartbeat terms
- See topic [00-architecture](../00-architecture/INDEX.md) for the Synapse Architecture and six traits
- See topic [03-composition](../03-composition/INDEX.md) for context engineering (the COMPOSE step)
