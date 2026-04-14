# Neuro — Cognitive Knowledge Layer

> Neuro (`roko-neuro`) is the agent's persistent, tiered, HDC-indexed knowledge system. It classifies knowledge by type (Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge), validates it through four tiers (Transient → Working → Consolidated → Persistent), encodes it as 10,240-bit hyperdimensional computing vectors for sub-millisecond similarity search, and decays it over time using Ebbinghaus forgetting curves. Neuro is a semantic wrapper around the Synapse Architecture's Substrate trait — it adds knowledge-specific logic on top of generic Engram storage.

**Part of**: [Roko PRD](../INDEX.md)
**Status**: Written
**Last generated**: 2026-04-12
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Synapse Architecture concepts (Engrams, 6 traits)

---

## Abstract

Neuro is the cognitive knowledge layer of the Roko agent framework. While episode logs capture raw agent turns and Dreams drives offline consolidation, Neuro is the subsystem that transforms experience into **durable, typed, validated, searchable knowledge**. Every piece of knowledge an agent retains passes through Neuro's classification, validation, encoding, and decay pipeline.

The subsystem was originally called "Grimoire" (now Neuro) in the legacy Bardo (now Roko) architecture. The rename reflects a shift from mystical framing to neuroscience-inspired terminology, aligned with the research traditions that underpin Neuro's design: Complementary Learning Systems theory (McClelland et al. 1995), Ebbinghaus forgetting curves (1885), hyperdimensional computing (Kanerva 2009, Kleyko et al. 2022), somatic markers (Damasio 1994), and hippocampal replay (Mattar & Daw 2018).

Neuro spans all five architectural layers (L0 Runtime through L4 Orchestration) via trait objects and callback interfaces. It is one of three cognitive cross-cuts (alongside Daimon for motivation/affect and Dreams for offline consolidation) that are injected into the system at multiple points, never hardcoded to a single layer.

---

## Contents

| # | Sub-doc | What it covers |
|---|---|---|
| 00 | [Vision and Grimoire Rename](./00-vision-and-grimoire-rename.md) | Neuro's architectural position, the rename from Grimoire, dissolution of roko-golem, design principles |
| 01 | [Six Knowledge Types](./01-six-knowledge-types.md) | Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge — definitions, examples, Rust types |
| 02 | [Four Validation Tiers](./02-four-validation-tiers.md) | Transient (0.1×), Working (0.5×), Consolidated (1.0×), Persistent (5.0×) — promotion/demotion mechanics |
| 03 | [Type Half-Lives](./03-type-half-lives.md) | Base half-life rationale for each type (7d–365d), Ebbinghaus model, current code constants |
| 04 | [HDC/VSA Foundations](./04-hdc-vsa-foundations.md) | BSC algebra, D=10,240, capacity bounds, SIMD performance, Johnson-Lindenstrauss |
| 05 | [HDC Operations](./05-hdc-operations.md) | Bind (XOR), Bundle (majority vote), Permute (cyclic shift), Similarity (Hamming) — Rust implementation |
| 06 | [HDC Knowledge Encoding](./06-hdc-knowledge-encoding.md) | Text→BSC encoding pipeline, role-filler bindings, three-tier search, structured queries |
| 07 | [Ebbinghaus Decay with Tier](./07-ebbinghaus-decay-with-tier.md) | Full decay formula, worked examples, GC schedule, reinforcement mechanics |
| 08 | [Cross-Domain HDC Transfer](./08-cross-domain-hdc-transfer.md) | Structural analogy detection, abstract role vectors, insight resonance, analogical reasoning |
| 09 | [False Positive Math](./09-false-positive-math.md) | Threshold selection (0.526), Bonferroni correction, Johnson-Lindenstrauss validation |
| 10 | [Knowledge Query API](./10-knowledge-query-api.md) | NeuroStore trait, KnowledgeStore JSONL backend, ContextAssembler, integration points |
| 11 | [AntiKnowledge Challenge](./11-antiknowledge-challenge.md) | Challenge mechanism, refutation warnings, epistemic parasite detection, Price equation |
| 12 | [4-Tier Distillation Pipeline](./12-4-tier-distillation-pipeline.md) | Episodes→Insights→Heuristics→PLAYBOOK.md, D1/D2/D3 stages, TierProgression struct |
| 13 | [Somatic Integration](./13-somatic-integration.md) | SomaticLandscape k-d tree, PAD vector, mood-congruent retrieval, 15% contrarian |
| 14 | [Library of Babel](./14-library-of-babel.md) | Cross-collective knowledge, 5 inflow channels, confidence discounting, publishing policies |
| 15 | [Knowledge Backup/Restore](./15-knowledge-backup-restore.md) | 4-step BACKUP→DELETE→CREATE→RESTORE, replacing succession, mesh sharing |
| 16 | [Current Status and Gaps](./16-current-status-and-gaps.md) | Implemented components, missing features, implementation plan mapping |

---

## Prerequisites

Before reading this topic, we recommend:

- [Topic 00: Architecture](../00-architecture/INDEX.md) — for the Synapse Architecture concepts (Engrams, 6 traits, cognitive loop) that Neuro builds on
- [Topic 05: Learning](../05-learning/INDEX.md) — for the episode logging system that feeds Neuro's distillation pipeline

---

## Cross-References

This topic connects to:

- [Topic 09: Daimon](../09-daimon/INDEX.md) — Daimon's PAD vector drives somatic marker integration and mood-congruent retrieval in Neuro
- [Topic 10: Dreams](../10-dreams/INDEX.md) — Dreams drives offline consolidation, tier promotion, and emotional depotentiation for Neuro entries
- [Topic 05: Learning](../05-learning/INDEX.md) — Episode logs from roko-learn are the raw input to Neuro's distillation pipeline
- [Topic 03: Composition](../03-composition/INDEX.md) — ContextAssembler draws from Neuro to populate agent prompts
- [Topic 04: Verification](../04-verification/INDEX.md) — Gate results feed back to Neuro for tier promotion/demotion
- [Topic 08: Chain](../08-chain/INDEX.md) — Korai chain hosts on-chain HDC vectors for collective knowledge
- [Topic 11: Safety](../11-safety/INDEX.md) — Knowledge ingestion safety (quarantine → consensus → sandbox → adopt)
- [Topic 13: Coordination](../13-coordination/INDEX.md) — Agent Mesh connectivity for cross-agent knowledge sync
- [Topic 15: Code Intelligence](../15-code-intelligence/INDEX.md) — roko-index uses HDC for code symbol fingerprinting

---

## Key Academic Foundations

- Ebbinghaus, H. (1885). *Über das Gedächtnis* (On Memory).
- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Why there are complementary learning systems in the hippocampus and neocortex." *Psychological Review*, 102(3), 419–457.
- Kanerva, P. (2009). "Hyperdimensional Computing." *Cognitive Computation*, 1(2), 139–159.
- Kleyko, D., Rachkovskij, D. A., Osipov, E., & Rahimi, A. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6).
- Damasio, A. R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam.
- Mattar, M. G., & Daw, N. D. (2018). "Prioritized memory access explains planning and hippocampal replay." *Nature Neuroscience*, 21, 1609–1617.
- Bower, G. H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148.
- Walker, M. P., & van der Helm, E. (2009). "Overnight therapy?" *Psychological Bulletin*, 135(5), 731–748.
- Frady, E. P., Kleyko, D., & Sommer, F. T. (2020). "Resonator Networks." *Neural Computation*, 32(12).
- Johnson, W. B., & Lindenstrauss, J. (1984). "Extensions of Lipschitz mappings." *Contemporary Mathematics*, 26.
- Thomas, A., Dasgupta, S., & Bhatt, T. (2021). "A Theoretical Perspective on HDC." *JAIR*, 72.
- Pearl, J. (2000). *Causality: Models, Reasoning, and Inference*. Cambridge University Press.
- Dawkins, R. (1976). *The Selfish Gene*. Oxford University Press.
- Price, G. R. (1970). "Selection and covariance." *Nature*, 227, 520–521.
- Plate, T. A. (2003). *Holographic Reduced Representations*. CSLI Publications.
- Lacaux, C., et al. (2021). "Sleep onset is a creative sweet spot." *Science Advances*, 7(50).
- Woolley, A. W., et al. (2010). "Collective Intelligence Factor." *Science*, 330(6004).
- Nader, K., et al. (2000). "Fear memories require protein synthesis for reconsolidation." *Nature*, 406.
- Murre, J. M. J., & Dros, J. (2015). "Replication of Ebbinghaus' Forgetting Curve." *PLOS ONE*, 10(7).
- Gentner, D. (1983). "Structure-mapping: A theoretical framework for analogy." *Cognitive Science*, 7(2).

---

## Current Status and Implementation Gaps

**Core knowledge system**: The `KnowledgeEntry`, `KnowledgeKind`, and `NeuroStore` trait are implemented. The JSONL storage backend (`KnowledgeStore`) is functional with decay, GC, and optional HDC indexing. The distillation pipeline (`Distiller`, `TierProgression`) provides three-stage episode→insight→heuristic→playbook progression.

**HDC subsystem**: The `HdcVector` (10,240-bit BSC) is fully implemented with all four operations, deterministic seeding, serde, and rkyv zero-copy support. Code symbol fingerprinting is implemented in `roko-index`.

**Key gaps**: The foundational refactor items are now in place: `KnowledgeEntry` has tiers plus emotional provenance, the PRD-native knowledge kinds are present, `ContextAssembler` is implemented in `roko-neuro`, ingest persists HDC vectors, CausalLinks now carry directional HDC encodings, and Neuro's local context allocator now uses auction-style budget selection plus mood-congruent scoring, direct somatic re-ranking, a contrarian affect slice, and a modest emotional-diversity reliability boost. Daimon now also owns a real 8D `SomaticLandscape`, config-backed strategy-space registration, and a shared strategy-space computer for routing-time affective bias, runtime somatic events, and dream-time depotentiation. The remaining gaps are the higher-order features: the full cross-subsystem VCG market, broader consolidation policy, fuller active inference, dedicated non-coding extractors, and cross-domain resonance.

**Recommended priority**: (1) complete the cross-subsystem VCG market, (2) add dedicated non-coding strategy-space extractors, (3) expand active-inference scoring, (4) add cross-domain resonance, (5) deepen the HDC stack with codebooks and multi-stage search.

See [16-current-status-and-gaps.md](./16-current-status-and-gaps.md) for the complete assessment.

---

## Generation Notes

- **Generated**: 2026-04-12
- **Model**: claude-opus-4-6
- **Sub-docs produced**: 17
- **Total lines**: ~4,750 (including INDEX.md)
- **Primary sources consulted**:
  - `refactoring-prd/03-cognitive-subsystems.md` (Neuro §1, Daimon §2, Dreams §3)
  - `refactoring-prd/04-knowledge-and-mesh.md` (Knowledge Architecture, Korai, Agent Mesh, Backup/Restore)
  - `refactoring-prd/09-innovations.md` (Somatic Landscape, Cross-Domain Resonance, False Positive Math, VCG)
  - `bardo-backup/prd/04-memory/` (00-overview, 01-grimoire, 01b-memetic, 01c-hdc, 02-emotional, 06-economy, 09-safety, 13-library-of-babel)
  - `bardo-backup/prd/shared/` (hdc-vsa, hdc-applications, hdc-fingerprints)
  - `bardo-backup/tmp/mori-refactor/` (09-memory-and-knowledge, 12-cognitive-architecture)
  - `bardo-backup/tmp/agent-chain/` (04-hdc, 05-knowledge-layer, 15-dynamic-context-assembly)
  - `crates/roko-neuro/src/` (lib.rs, knowledge_store.rs, distiller.rs, tier_progression.rs, context.rs)
  - `crates/bardo-primitives/src/hdc.rs`
  - `crates/roko-index/src/hdc.rs`
  - `crates/roko-learn/src/hdc_clustering.rs`
  - `crates/roko-golem/src/grimoire.rs` (dissolved placeholder)
- **Decisions requiring judgment**:
  - The refactoring-prd specifies 6 types while the code has 7 variants. Sub-doc 01 documents both and proposes reconciliation (keep all, add missing).
  - Warning (7d) is not yet in the enum; sub-doc 01 notes this gap and the overlap with Constraint.
  - The 0.526 threshold is documented as the recommendation per the refactoring-prd, with the full statistical derivation.
  - AntiKnowledge's "never decay" behavior is documented as confidence floor 0.3, matching the memetic evolution spec.
- **Open questions**:
  - How should the `KnowledgeKind` enum evolve? Should Fact, Procedure, Playbook, and Constraint remain alongside the 6 refactoring-prd types, or should they be consolidated?
  - Should the tier field be an enum on `KnowledgeEntry` or tracked separately in tier_metadata?
  - Should `ContextAssembler` use VCG auction from the start, or should a simpler priority-based approach be implemented first?
  - At what knowledge base size should the three-tier search (Bloom → approximate → exact) be activated?
