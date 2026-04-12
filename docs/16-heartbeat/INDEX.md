# Heartbeat: The Cognitive Clock

> The heartbeat is the agent's autonomous decision cycle — a continuous loop of observe-decide-act-learn running at three concurrent timescales (Gamma, Theta, Delta), with dual-process tier gating that makes ~80% of ticks free and active inference determining compute investment.

**Part of**: [Roko PRD](../INDEX.md)
**Status**: Written
**Last generated**: 2026-04-12
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md)

---

## Abstract

The heartbeat topic covers the core cognitive loop that every Roko agent executes autonomously. Unlike conversation-based agent frameworks where cognition is triggered by user messages, Roko agents run a continuous, timed decision cycle — the "heartbeat" — that operates whether or not a human is present. Approximately 80% of heartbeat ticks have no human input.

The heartbeat is organized around three key ideas:

1. **The 9-step CoALA-derived pipeline** (Sumers et al. 2023, arXiv:2309.02427): PERCEIVE → EVALUATE → ATTEND → INTEGRATE → ACT → VERIFY → PERSIST → ADAPT → META-COGNIZE. Every tick executes this loop, mapped to the six Synapse traits (Substrate, Scorer, Gate, Router, Composer, Policy) plus the Daimon cognitive cross-cut.

2. **Three cognitive speeds** (Buzsáki 2006): Gamma (~5-15s, reactive perception), Theta (~75s, reflective planning), and Delta (~hours, offline consolidation). All three run concurrently on separate async tasks managed by the adaptive clock in `roko-runtime`.

3. **Dual-process cognition** (Kahneman 2011): T0 (no LLM, deterministic probes, ~80% of ticks), T1 (fast model, ~15%), T2 (full model, ~5%). The 16 T0 probes drive tier suppression using FrugalGPT-inspired cascade routing (Chen et al. 2023).

The theoretical foundation draws from Friston's free energy principle (2010), Clark's predictive processing framework (2013), active inference for compute allocation (Friston et al. 2015), and VCG mechanism design (Vickrey 1961) for context budget allocation.

This topic spans L0 Runtime (adaptive clock, CorticalState), L1 Framework (tier routing, model selection), L2 Scaffold (context assembly, VCG auction), L3 Harness (gate verification), and L4 Orchestration (plan execution), plus the Daimon, Neuro, and Dreams cognitive cross-cuts.

---

## Contents

| # | Sub-doc | What it covers |
|---|---|---|
| 00 | [CoALA 9-Step Pipeline](./00-coala-9-step-pipeline.md) | The Cognitive Architectures for Language Agents framework. Academic predecessors (Soar, ACT-R, CLARION). Why decision cycles, not conversation turns. The 9-step pipeline specification. OODA loop mapping. |
| 01 | [Universal Loop Mapping](./01-universal-loop-mapping.md) | Side-by-side CoALA → Synapse translation. Which Synapse trait implements each step. Layer traversal. Domain parameterization (coding, chain, research). |
| 02 | [Chain Heartbeat Variant](./02-chain-heartbeat-variant.md) | Chain agents add SIMULATE (mirage-rs) and VALIDATE (PolicyCage) between ATTEND and ACT. The 11-step chain heartbeat. Three custody modes. Sleepwalker 3-step variant (OBSERVE → REFLECT → PUBLISH). |
| 03 | [Three Cognitive Speeds](./03-three-cognitive-speeds.md) | Gamma (5-15s reactive), Theta (75s reflective), Delta (hours consolidation). Nested hierarchy. Concurrency model with tokio tasks. Cost model per speed. |
| 04 | [Gamma Reactive Loop](./04-gamma-reactive-loop.md) | Main orchestration loop. Full 9-step execution per tick. Adaptive interval computation. DecisionCycleRecord. Ten cognitive mechanisms. OODA correspondence. |
| 05 | [Theta Reflective Loop](./05-theta-reflective-loop.md) | Five-phase reflection: summarize gamma, update Daimon affect (ALMA layers), check prediction calibration, re-evaluate plan, trigger interventions. Adaptive interval by regime. |
| 06 | [Delta Consolidation Loop](./06-delta-consolidation-loop.md) | Three-phase dream cycle: NREM replay (Mattar-Daw), REM imagination (Boden + Pearl), integration staging. Knowledge tier promotion. Playbook compilation. Non-blocking architecture. |
| 07 | [Adaptive Clock](./07-adaptive-clock.md) | The roko-runtime component managing three frequencies. Regime detection. Frequency adjustment rules. Budget-aware throttling. CognitiveSignal system. Event-driven wakeup. |
| 08 | [Dual-Process T0/T1/T2](./08-dual-process-t0-t1-t2.md) | LLM-Last architecture. InferenceTier enum. Adaptive gating threshold. Cost model. FrugalGPT validation. Interaction with Synapse traits. Active inference connection. |
| 09 | [16 T0 Probes](./09-16-t0-probes.md) | All 16 zero-LLM probes specified: 8 chain (price, TVL, position health, gas, credit, RSI, MACD, circuit breaker), 6 coding (build, tests, complexity, deps, coverage, error rate), 2 universal (world model drift, causal consistency). Probe trait. Registry extensibility. |
| 10 | [Active Inference: Compute Allocation](./10-active-inference-compute-allocation.md) | Expected Free Energy formula. Zero-hyperparameter exploration/exploitation. EFE for context selection. PredictiveScorer. Connection to CascadeRouter. Rational inattention (Sims 2003). |
| 11 | [Active Inference: State Space](./11-active-inference-state-space.md) | Factorized discrete POMDP: TaskPhase × ContextQuality × Uncertainty = 90 states. A/B/C/D matrices from pymdp. EFE computation in microseconds. Bayesian matrix learning. |
| 12 | [Attention Auction & CorticalState](./12-attention-auction-and-gating.md) | VCG truthful bidding for context budget. Eight bidding subsystems. Affect-modulated bids. Context governor. CorticalState 32-signal atomic struct. Meta-cognition hook. Frequency scheduler. |

---

## Prerequisites

Before reading this topic, we recommend:

- [Topic 00: Architecture](../00-architecture/INDEX.md) — for the core Synapse Architecture concepts (Engrams, 6 traits, cognitive loop, 5 layers)
- [Topic 09: Daimon](../09-daimon/INDEX.md) — for the PAD affect model, ALMA layers, and behavioral states
- [Topic 06: Neuro](../06-neuro/INDEX.md) — for knowledge tiers, HDC vectors, and tier progression
- [Topic 05: Learning](../05-learning/INDEX.md) — for episodes, CascadeRouter, playbooks, and calibration

---

## Cross-references

This topic connects to:

- [Topic 00: Architecture](../00-architecture/INDEX.md) — Synapse traits, universal loop, Engram definition
- [Topic 01: Orchestration](../01-orchestration/INDEX.md) — plan execution, DAG scheduler, the runtime that hosts the heartbeat
- [Topic 02: Agents](../02-agents/INDEX.md) — agent types, LLM backends, dispatch
- [Topic 03: Composition](../03-composition/INDEX.md) — context engineering, prompt assembly, Composer trait
- [Topic 04: Verification](../04-verification/INDEX.md) — gate pipeline, adaptive thresholds, ratcheting
- [Topic 05: Learning](../05-learning/INDEX.md) — episodes, CascadeRouter, playbooks, bandits, calibration
- [Topic 06: Neuro](../06-neuro/INDEX.md) — knowledge store, tier progression, HDC encoding
- [Topic 07: Conductor](../07-conductor/INDEX.md) — reactive watchers, circuit breakers
- [Topic 08: Chain](../08-chain/INDEX.md) — chain-specific heartbeat, SIMULATE/VALIDATE steps, mirage-rs
- [Topic 09: Daimon](../09-daimon/INDEX.md) — PAD vectors, ALMA layers, somatic markers, behavioral states
- [Topic 10: Dreams](../10-dreams/INDEX.md) — offline consolidation, NREM/REM/integration, hypnagogia
- [Topic 13: Coordination](../13-coordination/INDEX.md) — pheromone field, mesh coordination, C-Factor
- [Topic 15: Code Intelligence](../15-code-intelligence/INDEX.md) — coding domain probes, symbol graphs

---

## Key Academic Foundations

- Sumers et al. 2023 — CoALA (arXiv:2309.02427). Cognitive architectures for language agents.
- Buzsáki 2006 — "Rhythms of the Brain" (Oxford University Press). Oscillatory hierarchies in biological cognition.
- Friston 2010 — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Hierarchical prediction, active inference.
- Kahneman 2011 — "Thinking, Fast and Slow". System 1/System 2 dual-process theory.
- Chen et al. 2023 — FrugalGPT (arXiv:2305.05176). Cascade architectures for cost-optimal LLM routing.
- Mattar & Daw 2018 — "Prioritized memory access" (Nature Neuroscience 21). Utility-based dream replay.
- Vickrey 1961 — Second-price auction mechanism for truthful bidding.
- Baddeley 2000 — Working memory model (episodic buffer) for context assembly.
- Barrett 2017 — Constructed emotion from prediction residuals.
- Sun et al. 2005 — CLARION dual-level cognitive architecture.
- Koudahl et al. 2024 — Factorized discrete POMDP for tractable active inference (arXiv:2412.10425).
- McClelland et al. 1995 — Complementary learning systems. Fast/slow memory consolidation.
- Clark 2013 — "Whatever Next?" Predictive processing at multiple timescales.
- Sims 2003 — Rational inattention (Journal of Monetary Economics).
- Boden 2004 — Three creativity modes. Combinational, exploratory, transformational.
- Pearl 2009 — Structural causal models for counterfactual generation.

---

## Current Status and Implementation Gaps

**What exists:**
- The orchestration loop in `roko-cli/src/orchestrate.rs` is effectively a gamma loop.
- `InferenceTier` enum (T0/T1/T2) and `TierRouter` in `bardo-primitives/src/tier.rs`.
- `CascadeRouter` three-stage model routing in `roko-learn/src/cascade_router.rs`.
- Episode logging, efficiency events, adaptive gate thresholds — all wired.
- `bardo-runtime` provides process supervision, event bus, cancellation.
- `roko-dreams` crate exists as a scaffold.

**What is missing (implementation-plans/12a-cognitive-layer.md §I):**
- I1: Formal gamma loop with adaptive interval.
- I2: Theta loop with periodic reflection.
- I3: Delta loop triggering dreams, playbook compilation, meta-cognition.
- I4: Frequency scheduler deciding which loop to run.
- I5: Meta-cognition hook.
- CorticalState shared perception surface.
- T0 probe registry and 16 concrete probe implementations.
- VCG attention auction for context allocation.
- Active inference POMDP for tier selection (target: Stage 3).
- ALMA three-layer affect model in roko-daimon.
- Behavioral state machine (Engaged/Struggling/Coasting/Exploring/Focused/Resting).
- CognitiveSignal dispatch between loops.

---

## Generation Notes

- **Generated**: 2026-04-12
- **Model**: claude-opus-4-6
- **Sub-docs produced**: 13
- **Total lines**: ~4,200+
- **Primary sources consulted**:
  - `refactoring-prd/01-synapse-architecture.md` (Engram, 6 traits, universal loop, dual-process)
  - `refactoring-prd/02-five-layers.md` (adaptive clock, layer taxonomy)
  - `refactoring-prd/03-cognitive-subsystems.md` (Neuro, Daimon, Dreams)
  - `refactoring-prd/05-agent-types.md` (chain heartbeat, sleepwalker)
  - `refactoring-prd/08-translation-guide.md` (old→new mapping)
  - `refactoring-prd/09-innovations.md` (T0 probes, VCG, active inference, somatic landscape)
  - `bardo-backup/prd/01-golem/02-heartbeat.md` (full tick pipeline, DecisionCycleRecord, cost model)
  - `bardo-backup/prd/01-golem/18-cortical-state.md` (CorticalState, adaptive clock, Daimon PAD)
  - `bardo-backup/prd/01-golem/03-mind.md` (10 cognitive mechanisms)
  - `bardo-backup/prd/01-golem/01-cognition.md` (inference engine, cognitive workspace)
  - `bardo-backup/prd/01-golem/15-sleepwalker.md` (observer phenotype, 3-step variant)
  - `bardo-primitives/src/tier.rs` (InferenceTier, TierRouter)
  - `roko-learn/src/cascade_router.rs` (CascadeRouter, LinUCB)
  - `bardo-runtime/src/lib.rs` (event_bus, process, cancel)
  - `implementation-plans/12a-cognitive-layer.md` §I (I1-I5)
- **Decisions requiring judgment**:
  - CorticalState resource signals renamed from "mortality" framing (economic_vitality → resource_health, behavioral_phase → behavioral_state) per reframe rules. Underlying mechanisms preserved.
  - Sleepwalker 3-step variant included in chain heartbeat variant (02) rather than a separate sub-doc, as it's a specialization of the chain heartbeat.
  - Active inference split into two sub-docs (10: theory, 11: state space) rather than one, to keep each focused and under the recommended length.
  - VCG auction, CorticalState, meta-cognition, and frequency scheduler combined in sub-doc 12 as they are closely related aspects of the heartbeat's governance mechanism.
- **Open questions**:
  - The exact CorticalState field list will likely evolve as more subsystems are implemented. The 32-signal count is from the legacy spec and may need updating.
  - The 90-state POMDP is the target architecture; the heuristic threshold (current implementation) may coexist with it for a transition period.
  - Probe weights and thresholds need empirical tuning across domains — the values in this documentation are starting points from the design spec.
