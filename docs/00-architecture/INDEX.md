# Architecture

> The Synapse Architecture is Roko's compositional foundation: one noun (Engram) and six verb
> traits (Substrate, Scorer, Gate, Router, Composer, Policy) that compose to express every
> capability in the system. This topic covers the complete architectural specification — from
> the core data type through the five-layer taxonomy, cognitive cross-cuts, collective
> intelligence metrics, crate map, cybernetic foundations, and the fourteen frontier innovations
> that distinguish Roko from every other agent framework.

**Part of**: [Roko PRD](../INDEX.md)
**Status**: Written
**Last generated**: 2026-04-11
**Prerequisites**: None (this is the foundational topic)

---

## Abstract

Roko is a Rust toolkit for building cognitive agents that build themselves. Its architecture
— the Synapse Architecture — is built from a single universal data type (the Engram) and six
composable traits that process Engrams. Every capability in the system, from code generation
to chain transaction execution to knowledge consolidation, is an implementation of one of
these six traits operating on Engrams.

The architecture is organized into five layers (Runtime, Framework, Scaffold, Harness,
Orchestration) with strictly downward dependencies. Three cognitive cross-cuts (Neuro for
knowledge, Daimon for motivation, Dreams for offline learning) are injected across multiple
layers via trait objects, providing the self-improving capabilities that make Roko more than
a static framework.

This topic is the entry point for understanding Roko. It covers the theoretical foundations
(cybernetics, active inference, autocatalytic improvement), the concrete data structures
(Engram, Score, Decay, Provenance, ContentHash), the trait composition model, the cognitive
loop, and the frontier innovations that emerge from the architecture.

---

## Contents

| # | Sub-doc | What It Covers |
|---|---|---|
| 00 | [Vision and Core Thesis](./00-vision-and-thesis.md) | "The scaffold IS the product" thesis, empirical evidence, CoALA mapping, active inference foundations, design principles overview |
| 01 | [Naming and Glossary](./01-naming-and-glossary.md) | Complete old→new naming map (Bardo→Roko, Golem→Agent, etc.), crate dissolution, glossary of all terms |
| 02 | [Engram Data Type](./02-engram-data-type.md) | The universal content-addressed data unit: struct fields, ContentHash (BLAKE3), Kind enum (28 variants), Body enum, lineage DAG, builder pattern |
| 03 | [Score: 7-Axis Appraisal](./03-score-7-axis-appraisal.md) | 4 stable axes (confidence, novelty, utility, reputation) + 3 extended (precision, salience, coherence), effective formula, arithmetic, constants |
| 04 | [Decay Variants](./04-decay-variants.md) | Four decay variants (None, HalfLife, Ttl, Ebbinghaus), formulas, pheromone constants, knowledge tier interaction, selection guidelines |
| 05 | [Provenance and Attestation](./05-provenance-and-attestation.md) | Provenance struct (author, trust, tainted, session), four constructors, taint analysis, planned Attestation (Ed25519 + chain), extended fields |
| 06 | [Synapse Traits](./06-synapse-traits.md) | Why six traits, the trait composition model, all six trait signatures, trait × layer map, composability example via loop_tick |
| 07 | [Substrate Trait (Deep Dive)](./07-substrate-trait.md) | Full Substrate specification: 7 methods, async rationale, implementations (Memory, File, HDC, Chain), concurrency, idempotence, pruning |
| 08 | [Scorer, Gate, Router, Composer, Policy](./08-scorer-gate-router-composer-policy.md) | Detailed specs for all 5 non-Substrate traits: Scorer composition, 11-gate pipeline, CascadeRouter feedback, Composer budget, Policy batch model |
| 09 | [Universal Cognitive Loop](./09-universal-cognitive-loop.md) | 9-step loop specification with trait mappings, shipping loop_tick code, TickOutcome struct, loop at three speeds, universality |
| 10 | [Three Cognitive Speeds](./10-three-cognitive-speeds.md) | Gamma (~5-15s), Theta (~75s), Delta (hours): frequency selection, adaptive clock, turn limits, neuroscience inspiration |
| 11 | [Dual-Process and Active Inference](./11-dual-process-and-active-inference.md) | T0/T1/T2 tiers, EFE formula, 16 T0 probes, Kahneman mapping, classical architectures (ACT-R, SOAR, CLARION, GWT, CoALA), LLM-era architectures |
| 12 | [Five-Layer Taxonomy](./12-five-layer-taxonomy.md) | L0-L4 detailed specifications, cognitive cross-cuts table, trait × layer map, dependency rules, 18-crate map by layer |
| 13 | [Cognitive Cross-Cuts](./13-cognitive-cross-cuts.md) | Neuro (6 knowledge types, 4 tiers, HDC), Daimon (PAD, 6 states, somatic markers), Dreams (NREM/REM/Integration, hypnagogia), cross-cut interactions |
| 14 | [C-Factor: Collective Intelligence](./14-c-factor-collective-intelligence.md) | Two-level metric (C-Factor ratio + C-Score composite), four diagnostic signals, 31.6× collective calibration, network flywheel |
| 15 | [Crate Map](./15-crate-map.md) | 18+ crates by layer with status, test counts, dependency graph, roko-golem dissolution, legacy naming |
| 16 | [Autocatalytic Improvement and Cybernetics](./16-autocatalytic-and-cybernetics.md) | Kauffman autocatalytic sets, compound improvement (0.9^4), Ashby's Law, Good Regulator Theorem, Beer VSM, Free Energy Principle, stigmergy |
| 17 | [Design Principles and Frontier Summary](./17-design-principles-and-frontier-summary.md) | 7 design principles (P1-P7), 14 Blue Ocean innovations with details, interconnection map, empirical validation status |

---

## Prerequisites

This is the foundational topic. No prerequisites are required. All other topics in the Roko
PRD assume familiarity with the concepts introduced here — particularly the Engram data type,
the six Synapse traits, and the five-layer taxonomy.

**Recommended reading order within this topic**:
1. Start with [00-vision-and-thesis](./00-vision-and-thesis.md) for the high-level thesis
2. Read [01-naming-and-glossary](./01-naming-and-glossary.md) for terminology
3. Read [02-engram-data-type](./02-engram-data-type.md) through [05-provenance-and-attestation](./05-provenance-and-attestation.md) for the core data types
4. Read [06-synapse-traits](./06-synapse-traits.md) through [08-scorer-gate-router-composer-policy](./08-scorer-gate-router-composer-policy.md) for the trait system
5. Read [09-universal-cognitive-loop](./09-universal-cognitive-loop.md) through [11-dual-process-and-active-inference](./11-dual-process-and-active-inference.md) for the cognitive loop
6. Read [12-five-layer-taxonomy](./12-five-layer-taxonomy.md) through [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) for the architectural layers
7. Read [14-c-factor-collective-intelligence](./14-c-factor-collective-intelligence.md) through [17-design-principles-and-frontier-summary](./17-design-principles-and-frontier-summary.md) for metrics, crates, theory, and innovations

---

## Cross-References

This topic connects to:

- [Topic 01: Orchestration](../01-orchestration/INDEX.md) — L4 orchestration layer details (plan DAG, parallel executor, merge queue)
- [Topic 02: Agents](../02-agents/INDEX.md) — Agent types built on the Synapse Architecture
- [Topic 03: Composition](../03-composition/INDEX.md) — Context engineering and the Composer trait in depth
- [Topic 04: Verification](../04-verification/INDEX.md) — Gate pipeline and verification strategies
- [Topic 05: Learning](../05-learning/INDEX.md) — Episodes, playbooks, skills, bandits
- [Topic 06: Neuro](../06-neuro/INDEX.md) — Knowledge management cross-cut in depth
- [Topic 09: Daimon](../09-daimon/INDEX.md) — Motivation and affect cross-cut in depth
- [Topic 10: Dreams](../10-dreams/INDEX.md) — Offline learning cross-cut in depth
- [Topic 08: Chain](../08-chain/INDEX.md) — Korai chain and on-chain capabilities
- [Topic 13: Coordination](../13-coordination/INDEX.md) — Multi-agent coordination, Agent Mesh, stigmergy
- [Topic 14: Identity and Economy](../14-identity-economy/INDEX.md) — ERC-8004, KORAI token, x402

---

## Key Academic Foundations

- Sumers et al. 2023, arXiv:2309.02427 — CoALA: Cognitive Architectures for Language Agents
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
- **roko-core** (376 tests): Engram type (as `Signal`), all 6 Synapse traits, Score, Decay,
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
- Signal → Engram rename (Tier 0D).
- roko-golem dissolution (specified, not executed).
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
- **Sub-docs produced**: 18
- **Total lines**: ~6,500+
- **Primary sources consulted**:
  - `/Users/will/dev/nunchi/roko/refactoring-prd/00-overview.md` through `09-innovations.md`
  - `/Users/will/dev/nunchi/roko/roko/crates/roko-core/src/` (signal.rs, traits.rs, score.rs, decay.rs, kind.rs, provenance.rs, loop_tick.rs, query.rs, verdict.rs, context.rs, body.rs, hash.rs, operating_frequency.rs, lib.rs)
  - `/Users/will/dev/nunchi/roko/bardo-backup/prd/00-vision/` (04-trust.md, 05-manifesto.md)
  - `/Users/will/dev/nunchi/roko/bardo-backup/prd/shared/glossary.md`
  - `/Users/will/dev/nunchi/roko/roko/tmp/prd-migration/context-pack/` (all 7 files)
  - `/Users/will/dev/nunchi/roko/roko/docs/00-architecture/13-cognitive-cross-cuts.md` (existing architecture doc)
- **Decisions requiring judgment**:
  - Organized 18 sub-docs in logical reading order: thesis → naming → data types → traits → loop → speeds → layers → cross-cuts → metrics → crates → theory → innovations
  - Kept all academic citations from both refactoring-prd and legacy sources
  - Applied naming map consistently (Golem→Agent, Grimoire→Neuro, Signal→Engram, GNOS→KORAI, Clade→Collective)
  - Applied reframe rules: removed all mortality/death framing, reframed as budget/confidence/time constraints
  - Included full Rust code from shipping codebase with comments noting Signal→Engram rename (Tier 0D)
  - Maintained 31.6× collective calibration claim with explicit caveats about it being a heuristic
- **Open questions**:
  - Exact line count of the 14 previously written sub-docs was not re-verified after context window compaction (estimated 5,500+ lines for sub-docs 00-13, ~1,000+ for sub-docs 14-17)
  - The `roko-plugin` crate is listed as "Built" in refactoring-prd but "(to create)" in context-pack; followed refactoring-prd as source of truth
  - Cross-references to other topics (01-orchestration through 21-references) use forward references that cannot be verified until those topics are generated
