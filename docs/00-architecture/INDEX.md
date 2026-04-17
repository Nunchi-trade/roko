# Architecture

> The Synapse Architecture is Roko's compositional foundation: two mediums (Engram, durable
> content-addressed and balance-bearing under demurrage; Pulse, ephemeral topic-addressed and sequenced), two fabrics (Substrate
> for storage; Bus for transport), six operators, five layers, three speeds, and three
> cross-cuts. The canonical wire vocabulary is `Engram`, `Pulse`, `Bus`, `Topic`,
> `TopicFilter`, `Datum`, and `PulseSource`; see also `tmp/refinements/07-naming.md` for the
> naming decisions that standardize them, `tmp/refinements/01-critique-one-noun.md` for the
> original framing critique, `tmp/refinements/02-engram-vs-pulse.md` for the medium split,
> `tmp/refinements/03-bus-as-first-class.md` for the Bus promotion and two-fabric kernel,
> `tmp/refinements/04-operators-generalized.md` for `Datum` and `PolicyOutputs`,
> `tmp/refinements/05-loop-retold.md` for the seven-step loop, co-equal PERSIST/BROADCAST, and
> `tmp/refinements/10-self-learning-cybernetic-loops.md` for Bus-backed prediction, outcome,
> calibration, and `prediction.error.*` loops across operators,
> `tmp/refinements/12-knowledge-demurrage.md` for the attention-economy view of durable memory,
> cross-cut injection model, `tmp/refinements/09-phase-2-implications.md` for how Chain,
> Dreams, Coordination, and Heartbeat collapse onto that same kernel, and
> `tmp/refinements/13-collective-intelligence-c-factor.md` for continuous c-factor
> measurement from cohort Bus/Substrate statistics and conditional Policy intervention,
> `tmp/refinements/06-refactoring-plan.md` plus `tmp/refinements/21-from-scratch-redesigns.md`
> for the rollout and rewrite-track sequencing. This topic covers the complete architectural
> specification from the core data types through the layer taxonomy, cognitive cross-cuts,
> collective intelligence metrics, research-to-runtime provenance, crate map, cybernetic
> foundations, and the frontier innovations that distinguish Roko from every other agent
> framework. See also `tmp/refinements/16-research-to-runtime.md` for the paper → claim →
> heuristic → trial → calibration pipeline and replication ledger, and
> `tmp/refinements/18-competitive-moat.md` for the synthesis that turns those primitives into
> a structural moat rather than a feature list.

**Part of**: [Roko PRD](../INDEX.md)
**Status**: Written
**Last generated**: 2026-04-16
**Prerequisites**: None (this is the foundational topic)

---

## Abstract

Roko is a Rust toolkit for building cognitive agents that build themselves. Its architecture
— the Synapse Architecture — is built around two mediums: `Engram`, the durable
content-addressed record whose effective weight is shaped by demurrage `balance`, and `Pulse`, the ephemeral topic-addressed transport medium. Those mediums
move through two fabrics, `Substrate` for storage and `Bus` for transport, with six operators
coordinating the system across five layers, three speeds, and three cross-cuts. `Topic` and
`TopicFilter` govern routing on the Bus, `Datum` abstracts over Engram-or-Pulse inputs where
operators need to handle both, and `PulseSource` names the producer side of the ephemeral
stream. Engrams carry lineage through a DAG; Pulses carry ordering through Bus sequence
numbers; Pulses graduate to Engrams when lineage matters. See also
`tmp/refinements/07-naming.md` and `tmp/refinements/12-knowledge-demurrage.md`.

The architecture is organized into five layers (Runtime, Framework, Scaffold, Harness,
Orchestration) with strictly downward dependencies. Three cognitive cross-cuts (Neuro for
knowledge, Daimon for motivation, Dreams for offline learning) are injected across multiple
layers via trait objects, providing the self-improving capabilities that make Roko more than a
static framework.

This topic is the entry point for understanding Roko. It covers the theoretical foundations
(cybernetics, active inference, autocatalytic improvement), the concrete data structures
(Engram, Score, Decay, Provenance, ContentHash), the operator composition model, the cognitive
loop, and the frontier innovations that emerge from the architecture.

See also [tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md) for the learning-side heuristic,
falsifier, and worldview framing, and [Naming and Glossary](./01-naming-and-glossary.md) for
the canonical vocabulary. The developer-facing Rust SDK framing that maps these kernel concepts
onto one-liner, builder, trait impl, and runtime impl entry points lives in
[Topic 12: Rust SDK Developer UX](../12-interfaces/19-rust-sdk-developer-ux.md) and
[Topic 02: Extensibility](../02-agents/12-extensibility.md); see also
[tmp/refinements/22-developer-ux-rust.md](../../tmp/refinements/22-developer-ux-rust.md). REF23 then carries the same kernel model up into user-facing operation: four surfaces, one unified verb set, and one live event stream over named sessions. See [../12-interfaces/21-user-ux-running-agents.md](../12-interfaces/21-user-ux-running-agents.md) and [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md).

---

## Contents

| # | Sub-doc | What It Covers |
|---|---|---|
| 00 | [Vision and Core Thesis](./00-vision-and-thesis.md) | "The scaffold IS the product" thesis, empirical evidence, modern agent-systems mapping, active inference foundations, design principles overview |
| 01 | [Naming and Glossary](./01-naming-and-glossary.md) | Complete retired→current naming map, HDC fingerprint vocabulary, crate dissolution, glossary of all terms |
| 02 | [Engram Data Type](./02-engram-data-type.md) | The durable content-addressed record medium: struct fields, ContentHash (BLAKE3), HDC fingerprint, Kind enum, Body enum, lineage DAG, and builder pattern |
| 02b | [Pulse Ephemeral Event Medium](./02b-pulse-ephemeral-event.md) | The durable-versus-ephemeral medium split: Pulse, graduation rules, and the storage/stream boundary between Substrate and Bus |
| 03 | [Score: 7-Axis Appraisal](./03-score-7-axis-appraisal.md) | 4 stable axes (confidence, novelty, utility, reputation) + 3 extended (precision, salience, coherence), effective formula, arithmetic, constants |
| 04 | [Decay Variants](./04-decay-variants.md) | Demurrage superseding decay-first durable-memory framing: `balance`, reinforcement, novelty weighting, cold-tier freeze/thaw, and legacy rate-shaping mechanisms |
| 05 | [Provenance and Attestation](./05-provenance-and-attestation.md) | Provenance struct (author, trust, tainted, session), four constructors, taint analysis, planned Attestation (Ed25519 + chain), extended fields |
| 06 | [Synapse Traits](./06-synapse-traits.md) | Six operators across two mediums and two fabrics, the trait composition model, all six trait signatures, trait × layer map, composability example via loop_tick |
| 07 | [Substrate Trait (Deep Dive)](./07-substrate-trait.md) | The durable storage fabric: Substrate's trait surface, fingerprint population, `query_similar`, backends, concurrency, pruning, and its relationship to the Bus sibling fabric |
| 07b | [Bus Transport Fabric](./07b-bus-transport-fabric.md) | The transport fabric: Bus trait, Topic and TopicFilter routing, replay/ring semantics, backend families, and topic-driven decoupling |
| 08 | [Scorer, Gate, Router, Composer, Policy](./08-scorer-gate-router-composer-policy.md) | Detailed specs for the five non-Substrate operators after REF04: `Datum`-aware scoring/composition, stream Gates, Pulse routing, and `PolicyOutputs` |
| 09 | [Universal Cognitive Loop](./09-universal-cognitive-loop.md) | Seven-step loop specification with SENSE, ASSESS, COMPOSE, ACT, VERIFY, co-equal PERSIST/BROADCAST, REACT, plus three-speed and shipping `loop_tick` guidance |
| 10 | [Three Cognitive Speeds](./10-three-cognitive-speeds.md) | Gamma (~5-15s), Theta (~75s), Delta (hours): frequency selection, adaptive clock, turn limits, neuroscience inspiration |
| 11 | [Dual-Process and Active Inference](./11-dual-process-and-active-inference.md) | T0/T1/T2 tiers, EFE formula, 16 T0 probes, literal predict/publish/correct active inference on the Bus, per-operator calibration, Kahneman mapping, classical architectures, LLM-era architectures |
| 12 | [Five-Layer Taxonomy](./12-five-layer-taxonomy.md) | L0-L4 detailed specifications, cognitive cross-cuts table, operator × layer map, dependency rules, and the target dep graph boundaries for `roko-bus`, `roko-hdc`, `roko-spi`, and the std/compose splits |
| 13 | [Cognitive Cross-Cuts](./13-cognitive-cross-cuts.md) | Neuro, Daimon, and Dreams as injected cross-cuts rather than loop steps: knowledge retrieval/composition, affect-biased assessment and act gating, and Delta-speed consolidation |
| 14 | [C-Factor: Collective Intelligence](./14-c-factor-collective-intelligence.md) | Continuous c-factor measurement from cohort Bus and Substrate statistics, five-axis `CohortMetrics`, conditional Policy levers, WisdomGate, and anti-groupthink controls |
| 15 | [Crate Map](./15-crate-map.md) | Current workspace crates plus the target dep graph: `roko-bus`, `roko-hdc`, `roko-spi`, `roko-defaults`, `roko-tools`, `roko-compose-core`, `roko-templates`, migration phases, and enforcement rules |
| 16 | [Autocatalytic Improvement and Cybernetics](./16-autocatalytic-and-cybernetics.md) | REF15 compounding architecture: seven superlinear loops, two-medium/two-fabric integration, seven-step loop mapping, KPI and anti-metric panels, failure modes, and cybernetic foundations |
| 17 | [Design Principles and Frontier Summary](./17-design-principles-and-frontier-summary.md) | 7 design principles plus the REF19 net-new innovations catalog: primitive/pattern/API claims, prior-art honesty, structural-moat synthesis, and validation priorities |
| 18 | [Decay-Tier Matrix](./18-decay-tier-matrix.md) | Tier-specific demurrage behavior: hot-to-cold graduation, reinforcement stickiness, tier floors, and promotion/demotion under use, contradiction, and surprise |
| 19 | [Compositional Kinds](./19-compositional-kinds.md) | Kind::Compound variant for cross-domain records, filter matching, scoring, migration path |
| 20 | [Configuration Schema](./20-configuration-schema.md) | RokoConfig plus the specified demurrage tuning surface: taxes, reinforcement bonuses, cold-floor thresholds, and validation notes |
| 21 | [Performance and Numerical Stability](./21-performance-numerical-stability.md) | f32/f64 decision matrix, hot-path complexity targets, memory budgets, NaN/Inf handling |
| 22 | [Error Handling and Recovery](./22-error-handling-recovery.md) | Four error classes, exponential backoff, circuit breaker, 7-level graceful degradation |
| 23 | [Architectural Analysis and Improvements](./23-architectural-analysis-improvements.md) | Coherence analysis of the current architecture plus the v2 rewrite path: layer violations, dep-graph audit items, refactor boundaries, category theory, and from-scratch decision points |
| 24 | [Cross-Section Integration Map](./24-cross-section-integration-map.md) | Full 22×22 dependency matrix, 20 missing integrations (prioritized), REF09 two-fabric implications for ChainBus/Dreams/Coordination/Heartbeat, data flow diagrams, integration roadmap (~2,070 LOC) |
| 25 | [Attention as Universal Cognitive Currency](./25-attention-as-currency.md) | Attention tokens for online spend plus demurrage for idle durable memory: VCG selection, budget-aware routing, and attention-economy observability across speeds |
| 26 | [Cognitive Immune System](./26-cognitive-immune-system.md) | 5-layer defense against knowledge corruption: taint propagation, anomaly detection, quarantine, red-team probes, immune memory with HDC signatures |
| 27 | [Temporal Knowledge Topology](./27-temporal-knowledge-topology.md) | Allen's 13 interval relations, temporal calculus, HDC-clustered episode/entity/community tiers, and temporal queries |
| 28 | [Emergent Goal Structures](./28-emergent-goal-structures.md) | Goals emerge from affect×knowledge×experience interaction, 5 built-in detectors, intrinsic motivation scoring, EFE ranking, somatic markers, goal lifecycle |
| 29 | [Cognitive Energy Model](./29-cognitive-energy-model.md) | Explicit energy pool with depletion and recovery, 5 energy zones, affect-energy bidirectional coupling, capacity growth, Delta sleep cycles |
| 30 | [Cross-Pollination Innovations](./30-cross-pollination-innovations.md) | Innovation transfer patterns across sections, with REF19's composition-over-isolation framing and the coherence-as-moat claim for the reinforcing architectural weave |
| 31 | [Implementation Readiness Audit](./31-implementation-readiness-audit.md) | Section scorecard, readiness criteria, rewrite-track candidates, gap analysis, crate status, and implementation sequencing |
| 32 | [Comprehensive Test Strategy](./32-comprehensive-test-strategy.md) | Per-crate unit test specs, property-based testing (proptest), integration matrix, performance benchmarks (criterion/iai), adversarial/safety testing (ATLAS-mapped), regression prevention for self-improving agents, test count roadmap 1,568→2,810 |
| 33 | [Refactor Plan Phases](./33-refactor-plan-phases.md) | Phase A/B/C/D rollout sequence, rollback plan, risk notes, and dependency ordering; see `tmp/refinements/06-refactoring-plan.md` |

---

## Prerequisites

This is the foundational topic. No prerequisites are required. All other topics in the Roko
PRD assume familiarity with the concepts introduced here — particularly the Engram and Pulse
mediums, the six operators, the two fabrics, and the five-layer taxonomy.

**Recommended reading order within this topic**:
1. Start with [00-vision-and-thesis](./00-vision-and-thesis.md) for the high-level thesis
2. Read [01-naming-and-glossary](./01-naming-and-glossary.md) for terminology
3. Read [02-engram-data-type](./02-engram-data-type.md) and [02b-pulse-ephemeral-event](./02b-pulse-ephemeral-event.md) through [05-provenance-and-attestation](./05-provenance-and-attestation.md) for the core data types and medium split
4. Read [06-synapse-traits](./06-synapse-traits.md) through [08-scorer-gate-router-composer-policy](./08-scorer-gate-router-composer-policy.md) for the trait system, including the Substrate/Bus fabric split
5. Read [09-universal-cognitive-loop](./09-universal-cognitive-loop.md) through [11-dual-process-and-active-inference](./11-dual-process-and-active-inference.md) for the cognitive loop
6. Read [12-five-layer-taxonomy](./12-five-layer-taxonomy.md) through [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) for the architectural layers
7. Read [14-c-factor-collective-intelligence](./14-c-factor-collective-intelligence.md) through [17-design-principles-and-frontier-summary](./17-design-principles-and-frontier-summary.md) for metrics, crates, theory, and innovations
8. Read [31-implementation-readiness-audit](./31-implementation-readiness-audit.md) and [33-refactor-plan-phases](./33-refactor-plan-phases.md) for the current-state scorecard and phased landing sequence

---

## Cross-References

This topic connects to:

- [Topic 01: Orchestration](../01-orchestration/INDEX.md) — L4 orchestration layer details (plan DAG, parallel executor, merge queue)
- [Topic 02: Agents](../02-agents/INDEX.md) — Agent types built on the Synapse Architecture
- [Topic 12: Interfaces](../12-interfaces/INDEX.md) — Interface surfaces, including the four surfaces user-UX chapter, unified verb set, and the four-layer Rust SDK
- [Topic 03: Composition](../03-composition/INDEX.md) — Context engineering and the Composer trait in depth
- [Topic 04: Verification](../04-verification/INDEX.md) — Gate pipeline and verification strategies
- [Topic 05: Learning](../05-learning/INDEX.md) — Episodes, playbooks, heuristics, falsifier-driven calibration, worldview clustering, bandits, predict/publish/correct loops
- [Topic 06: Neuro](../06-neuro/INDEX.md) — Knowledge management cross-cut in depth, including the heuristic library and distillation pipeline
- [Topic 21: References](../21-references/INDEX.md) — Research-to-runtime pipeline, paper/claim vocabulary, and the replication ledger
- [Topic 09: Daimon](../09-daimon/INDEX.md) — Motivation and affect cross-cut in depth
- [Topic 10: Dreams](../10-dreams/INDEX.md) — Offline learning cross-cut in depth
- [Topic 08: Chain](../08-chain/INDEX.md) — Korai chain and on-chain capabilities
- [Topic 13: Coordination](../13-coordination/INDEX.md) — Multi-agent coordination, Agent Mesh, stigmergy
- [Topic 14: Identity and Economy](../14-identity-economy/INDEX.md) — ERC-8004, KORAI token, x402

---

## Key Academic Foundations

- Sumers et al. 2023, arXiv:2309.02427 — Cognitive Architectures for Language Agents
- Friston 2010, Nature Reviews Neuroscience 11(2) — Free Energy Principle
- Conant & Ashby 1970, IJSS 1(2) — Good Regulator Theorem
- Beer 1972, "Brain of the Firm" — Viable System Model
- Ashby 1956, "An Introduction to Cybernetics" — Law of Requisite Variety
- Kauffman 1993, "The Origins of Order" — Autocatalytic sets
- Kanerva 2009, Cognitive Computation 1(2) — Hyperdimensional Computing
- Woolley et al. 2010, Science 330(6004) — Collective Intelligence Factor
- Lee et al. 2026, arXiv:2603.28052 — Meta-Harness
- Chen et al. 2023, arXiv:2305.05176 — FrugalGPT
- Mehrabian & Russell 1974 — PAD emotional model
- Damasio 1994, "Descartes' Error" — Somatic marker hypothesis
- Mattar & Daw 2018, Nature Neuroscience 21 — Prioritized replay
- Lacaux et al. 2021, Science Advances 7(50) — Hypnagogia and creativity
- Pearl 2009, "Causality" — Structural Causal Models
- Hu et al. 2025, ICLR — ADAS: meta-agent architecture search
- Grassé 1959 — Stigmergy
- Dorigo et al. 2000 — Ant colony optimization

---

## Current Status and Implementation Gaps

### What's Built
- **roko-core** (376 tests): Engram type, all 6 Synapse traits, Score, Decay,
  Kind, Body, ContentHash, Provenance, loop_tick, OperatingFrequency, config schema.
- **roko-agent** (346 tests): 5 LLM backends, CascadeRouter, MCP client, tool dispatch, safety.
- **roko-gate** (200 tests): 11+ gates, 6-rung pipeline, adaptive thresholds.
- **roko-orchestrator** (158 tests): Plan DAG, parallel executor, state persistence + resume.
- **roko-learn** (101 tests): Episodes, playbooks, skills, bandits, experiments, efficiency.
- **roko-std** (96 tests): Default trait impls, 19 built-in tools.
- **roko-chain** (52 tests): ChainClient/ChainWallet, chain witness.
- **roko-daimon**: PAD vector, 6 behavioral states, somatic markers (972 lines).
- **roko-neuro**: Knowledge store, 6 types, 4 tiers, HDC encoding.
- Total: ~1,568 tests, ~177K LOC across 18+ crates.

### What's Scaffold
- **roko-dreams**: Three-phase cycle specified, not shipping.
- **roko-serve**: HTTP API not wired.
- **roko-mcp-***: Basic transport only.
- **TUI**: Text-mode dashboard only, no interactive terminal UI.

### What's Not Started
- Kernel v2 cutover decision and vocabulary alignment (Tier 0D).
- Legacy umbrella-crate dissolution (specified, not executed).
- VCG Attention Auction (specified, not wired).
- Somatic Landscape k-d tree (specified, not built).
- ADAS meta-search.
- Cognitive Kernel primitives (partial: SIGPAUSE exists).
- Knowledge Futures Market (P3, deferred).
- Generative Interfaces / A2UI.

---

## Generation Notes

- **Generated**: 2026-04-11
- **Model**: Claude Opus 4.6
- **Sub-docs produced**: 29
- **Total lines**: ~10,500+
- **Primary sources consulted**:
  - `/Users/will/dev/nunchi/roko/refactoring-prd/00-overview.md` through `09-innovations.md`
  - `/Users/will/dev/nunchi/roko/roko/crates/roko-core/src/` (core Engram/trait/score/decay/query/verdict sources)
  - historical archive: pre-refactor vision notes (04-trust.md, 05-manifesto.md)
  - historical archive: shared glossary from the pre-refactor source corpus
  - `/Users/will/dev/nunchi/roko/roko/tmp/prd-migration/context-pack/` (all 7 files)
  - `/Users/will/dev/nunchi/roko/roko/docs/00-architecture/13-cognitive-cross-cuts.md` (existing architecture doc)
- **Decisions requiring judgment**:
  - Organized 18 sub-docs in logical reading order: thesis → naming → data types → traits → loop → speeds → layers → cross-cuts → metrics → crates → theory → innovations
  - Kept all academic citations from both refactoring-prd and legacy sources
  - Applied the current naming map consistently and marked legacy names only in explicitly retired contexts
  - Applied reframe rules consistently, favoring budget/confidence/time constraints over older lifecycle metaphors
  - Included full Rust code from the shipping codebase with comments aligned to current Engram terminology
  - Maintained 31.6× collective calibration claim with explicit caveats about it being a heuristic
- **Open questions**:
  - Exact line count of the 14 previously written sub-docs was not re-verified after context window compaction (estimated 5,500+ lines for sub-docs 00-13, ~1,000+ for sub-docs 14-17)
  - The `roko-plugin` crate is listed as "Built" in refactoring-prd but "(to create)" in context-pack; followed refactoring-prd as source of truth
  - Cross-references to other topics (01-orchestration through 21-references) use forward references that cannot be verified until those topics are generated
