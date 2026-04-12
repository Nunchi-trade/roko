# Universal Loop Mapping: CoALA → Synapse

> How the CoALA 9-step pipeline maps to the universal Synapse loop — the domain-agnostic version that every Roko agent executes.

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md), [00-architecture](../00-architecture/INDEX.md)
**Key sources**: `refactoring-prd/01-synapse-architecture.md` §3, `refactoring-prd/08-translation-guide.md` §7

---

## Abstract

The CoALA 9-step pipeline (OBSERVE → RETRIEVE → ANALYZE → GATE → SIMULATE → VALIDATE → EXECUTE → VERIFY → REFLECT) is the legacy framework that guided Roko's initial cognitive architecture. The Synapse Architecture introduces a 9-step universal loop (PERCEIVE → EVALUATE → ATTEND → INTEGRATE → ACT → VERIFY → PERSIST → ADAPT → META-COGNIZE) that serves as the domain-agnostic abstraction.

The universal Synapse loop is NOT a different pipeline — it is the same pipeline expressed in terms of the six composable Synapse traits (Substrate, Scorer, Gate, Router, Composer, Policy) plus the Daimon cognitive cross-cut. The CoALA heartbeat is the theoretical frame; the Synapse loop is the implementation frame. Domain-specific heartbeat variants (chain, coding, research) are parameterizations of the universal loop, not separate architectures.

This document provides the complete side-by-side mapping between CoALA and Synapse, explains why the translation is not one-to-one but structurally faithful, and describes how domain-specific variants extend the universal loop without modifying it.

---

## Side-by-Side Mapping Table

The CoALA 9-step pipeline and the universal Synapse loop are structurally isomorphic but differ in granularity and naming. The table below shows the exact correspondence, including which Synapse trait implements each step and which architectural layer it lives at.

| # | CoALA Step | Synapse Step | Synapse Trait | Layer | Description |
|---|---|---|---|---|---|
| 1 | OBSERVE | **PERCEIVE** | `Substrate.query()` | L0 Runtime | Read environment state: run probes, fetch current observations, detect regime changes. The Substrate provides raw access to stored Engrams and external state. |
| 2 | RETRIEVE | **EVALUATE** | `Scorer.score()` | L2 Scaffold | Score retrieved Engrams by relevance, recency, emotional congruence, and confidence. Multi-factor scoring determines which knowledge is most valuable for the current situation. |
| 3 | ANALYZE | _(merged into EVALUATE + Daimon)_ | Daimon cross-cut | Cognitive cross-cut | Compute prediction error — how surprising is this observation? In the Synapse loop, this is split between the Scorer (which evaluates surprise/novelty) and the Daimon (which computes affect from prediction residuals). |
| 4 | GATE | **ATTEND** | `Router.select()` | L1 Framework | Decide what matters most right now. The Router selects the cognitive tier (T0/T1/T2) and the specific model to use. This is the System 1 / System 2 decision point. |
| 5 | SIMULATE | _(domain-specific)_ | Domain Gate impl | L3 Harness | Pre-flight verification. Not in the universal loop — chain agents add it via `TxSimGate`, coding agents may add dry-run gates. See [02-chain-heartbeat-variant.md](./02-chain-heartbeat-variant.md). |
| 6 | VALIDATE | _(domain-specific)_ | Domain Gate impl | L3 Harness | Safety constraint checking. Not a separate universal step — implemented through domain-specific Gate implementations that run as part of the Gate pipeline. |
| — | — | **INTEGRATE** | `Composer.compose()` | L2 Scaffold | Build the context window under token budget. This step does not exist in CoALA — it is Roko's explicit context engineering step. The Composer assembles the optimal prompt from scored Engrams, knowledge entries, playbook rules, and affect state. |
| 7 | EXECUTE | **ACT** | `Agent.execute()` | L1 Framework | Call the LLM backend, produce output. The framework dispatches to the appropriate backend (Claude CLI, HTTP API, Ollama, Cursor) based on the Router's selection. |
| 8 | VERIFY | **VERIFY** | `Gate.verify()` | L3 Harness | Check output against external ground truth. Compiler, test suite, linter, blockchain receipt — never self-assessment. |
| — | — | **PERSIST** | `Substrate.put()` | L0 Runtime | Store output as a new Engram with lineage (audit DAG). This step is implicit in CoALA's REFLECT but explicit in Synapse because content-addressed persistence is a first-class architectural concern. |
| 9 | REFLECT | **ADAPT** | `Policy.decide()` | L3-L4 Harness/Orchestration | Observe the Engram stream, detect patterns, emit new Engrams (episodes, retries, interventions). The Policy trait processes batches of recent Engrams and fires reactive behaviors. |
| — | — | **META-COGNIZE** | `Daimon.assess()` | Cognitive cross-cut | "Am I doing this well? Should I change approach?" Meta-cognition is CoALA's REFLECT expanded into explicit self-assessment. The Daimon updates its PAD vector, checks for stuck loops, evaluates whether to escalate or change strategy. |

### Key Differences

1. **INTEGRATE is new.** CoALA assumes context is assembled somehow; Synapse makes it an explicit step with a dedicated trait (`Composer`). This reflects Roko's thesis that context engineering is the primary determinant of agent performance (Meta-Harness, Lee et al. 2026, arXiv:2603.28052).

2. **PERSIST is explicit.** CoALA lumps persistence into REFLECT. Synapse separates it because content-addressed Engram storage with lineage tracking is a foundational concern — every output gets a BLAKE3 hash, a lineage chain, and provenance metadata. This separation enables the Forensic AI capability (see topic [11-safety](../11-safety/INDEX.md)).

3. **META-COGNIZE is explicit.** CoALA's REFLECT covers both reactive adaptation and meta-cognitive assessment. Synapse separates them: ADAPT fires reactive policies (retries, episodes, interventions), while META-COGNIZE performs the higher-level self-assessment ("Am I stuck? Am I thrashing? Should I escalate?"). The Daimon cross-cut handles this.

4. **SIMULATE/VALIDATE are domain-specific.** The universal loop does not include them. They are injected by domain-specific agent types (chain agents add SIMULATE via mirage-rs, coding agents may add dry-run steps). The universal loop accommodates domain extensions without modification.

---

## The Synapse Loop Traverses All Layers

A single tick of the cognitive loop crosses the full five-layer stack. This is the key architectural insight — the six traits are distributed across layers, not concentrated at one level:

```
L0 Runtime     ──→ Substrate.query()      [PERCEIVE: fetch from storage]
L2 Scaffold    ──→ Scorer.score()         [EVALUATE: score relevance]
L1 Framework   ──→ Router.select()        [ATTEND: choose cognitive tier]
L2 Scaffold    ──→ Composer.compose()     [INTEGRATE: build context window]
L1 Framework   ──→ Agent.execute()        [ACT: call LLM backend]
L3 Harness     ──→ Gate.verify()          [VERIFY: check against ground truth]
L0 Runtime     ──→ Substrate.put()        [PERSIST: store with lineage]
L3-L4 Harness  ──→ Policy.decide()        [ADAPT: detect patterns, emit reactions]
Cross-cut      ──→ Daimon.assess()        [META-COGNIZE: self-assessment]
```

Every tick traverses L0 → L2 → L1 → L2 → L1 → L3 → L0 → L3-L4 → cross-cut. This is why layer boundaries must be clean — a single tick crosses four layers. Dependencies flow strictly downward. Cross-cutting concerns (Neuro, Daimon, Dreams) are injected via trait objects, never via direct imports of higher layers.

---

## Domain Parameterization

The universal loop is one loop, parameterized by domain. Domain-specific behavior comes from domain-specific trait implementations and configuration — not from architectural modifications.

### Coding Agent

Uses the universal loop as-is. No additional steps between ATTEND and ACT.

```
PERCEIVE  → FileSubstrate.query() [read codebase state]
EVALUATE  → RecencyScorer + ReputationScorer [rank by freshness and trust]
ATTEND    → CascadeRouter [T0/T1/T2 gating]
INTEGRATE → PromptComposer [code-aware templates, U-shape placement]
ACT       → Agent.execute() [call Claude/GPT, produce code changes]
VERIFY    → CompileGate → TestGate → ClippyGate → DiffGate
PERSIST   → FileSubstrate.put() [store with lineage]
ADAPT     → EpisodePolicy + DaimonPolicy + PredictionPolicy
META-COGNIZE → Daimon.assess() [am I stuck on this crate?]
```

### Chain Agent

Adds SIMULATE (mirage-rs pre-flight) and VALIDATE (position limits) between ATTEND and ACT. See [02-chain-heartbeat-variant.md](./02-chain-heartbeat-variant.md) for the full mapping.

```
PERCEIVE  → FileSubstrate.query() + ChainSubstrate.query() [market + on-chain state]
EVALUATE  → RecencyScorer + CatalystScorer + PredictiveScorer [multi-factor]
ATTEND    → Router.select() [T0/T1/T2 + active inference]
  → SIMULATE  → TxSimGate via mirage-rs [pre-flight in local EVM fork]
  → VALIDATE  → WalletGate [position limits, approved assets]
INTEGRATE → AttentionAuction [VCG bidding for context budget]
ACT       → Agent.execute() [submit transaction, invoke tools]
VERIFY    → VerifyChainGate [blockchain receipt, balance check]
PERSIST   → FileSubstrate.put() + ChainSubstrate.put() [on-chain + off-chain]
ADAPT     → EpisodePolicy + DaimonPolicy + PredictionPolicy + CFactorPolicy
META-COGNIZE → Daimon.assess() [should I reduce exposure?]
```

### Research Agent

Simplified loop — SIMULATE and VALIDATE are typically skipped (research actions are reversible).

```
PERCEIVE  → MemorySubstrate.query() [current research state]
EVALUATE  → RecencyScorer [rank by freshness]
ATTEND    → Router.select() [typically T1/T2, research benefits from reasoning]
INTEGRATE → PromptComposer [research-focused templates, large budget]
ACT       → Agent.execute() [web search, synthesis, citation tracking]
VERIFY    → LlmJudgeGate [subjective quality check]
PERSIST   → MemorySubstrate.put() [store findings]
ADAPT     → EpisodePolicy
META-COGNIZE → Daimon.assess() [am I going in circles?]
```

---

## The Translation from Legacy Architecture

The legacy system (under the old "Bardo" naming) had a "Golem Heartbeat" that was functionally equivalent but domain-specific to chain agents. The refactoring-prd translation guide (`08-translation-guide.md` §7) specifies the mapping:

```
Old Golem Heartbeat:          New Universal Loop:
1. Observe                →   PERCEIVE (Substrate.query)
2. Retrieve               →   EVALUATE (Scorer.score)
3. Analyze                →   (part of scoring + Daimon cross-cut)
4. Gate                   →   ATTEND (Router.select)
5. Simulate               →   (domain-specific pre-ACT)
6. Validate               →   (safety check, part of Gate)
7. Execute                →   ACT (Agent.execute)
8. Verify                 →   VERIFY (Gate.verify)
9. Reflect                →   ADAPT (Policy.decide) + META-COGNIZE (Daimon.assess)
```

The key change: the old heartbeat was a monolithic chain-specific pipeline. The new universal loop is a composable, domain-agnostic architecture where each step is a pluggable trait implementation. This enables:

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
| **Verification** | "Grounding actions" | Explicit Gate pipeline with 11+ implementations |
| **Affect** | Not modeled | Daimon cross-cut (PAD vector modulates all steps) |
| **Multi-scale** | Single cycle | Three concurrent scales (Gamma/Theta/Delta) |
| **Cost optimization** | Not modeled | T0/T1/T2 gating (~80% free ticks) |
| **Meta-cognition** | Part of "learning" | Explicit META-COGNIZE step via Daimon |

CoALA provides the intellectual justification. The Synapse loop provides the engineering realization. The two are not in tension — they are layers of the same architecture.

---

## Academic Foundations

- **Sumers, Yao, Narasimhan & Griffiths 2023** — "Cognitive Architectures for Language Agents" (arXiv:2309.02427). The CoALA framework.
- **Lee et al. 2026** — "Meta-Harness: Optimizing Agent Scaffolds" (arXiv:2603.28052). Evidence that scaffold optimization matters more than model selection.
- **Conant & Ashby 1970** — "Every Good Regulator of a System Must Be a Model of That System" (International Journal of Systems Science 1(2)). The Good Regulator Theorem — justifies META-COGNIZE step.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Prediction error as the organizing signal for cognitive architecture.

---

## Current Status and Gaps

**What exists:**
- The orchestration loop in `roko-cli/src/orchestrate.rs` implements a simplified version of the Synapse loop (PERCEIVE → ATTEND → ACT → VERIFY → PERSIST → ADAPT).
- All six Synapse traits are defined in `roko-core/src/traits.rs`.
- Multiple implementations exist for each trait across the crate ecosystem.

**What is missing:**
- The full EVALUATE step with multi-factor Neuro scoring is not yet wired (see `12a-cognitive-layer.md` §E).
- The INTEGRATE step with VCG attention auction is not yet implemented (see `refactoring-prd/09-innovations.md` §II).
- The META-COGNIZE step with Daimon self-assessment is scaffolded but not wired into the orchestration loop.
- Domain-specific SIMULATE/VALIDATE injection points are not yet formalized in the orchestration code.

---

## Cross-References

- See [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md) for the CoALA theoretical foundation
- See [02-chain-heartbeat-variant.md](./02-chain-heartbeat-variant.md) for the chain domain extension
- See [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md) for the VCG INTEGRATE mechanism
- See topic [00-architecture](../00-architecture/INDEX.md) for the Synapse Architecture and six traits
- See topic [03-composition](../03-composition/INDEX.md) for context engineering (the INTEGRATE step)
