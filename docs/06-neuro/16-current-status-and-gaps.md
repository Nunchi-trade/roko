# Current Status and Implementation Gaps

> A comprehensive assessment of what exists in the roko-neuro codebase today, what is scaffolded, and what remains to be built — mapped against the refactoring-prd design and the implementation plan.

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [00-vision-and-grimoire-rename.md](./00-vision-and-grimoire-rename.md)
**Key sources**:
- `crates/roko-neuro/src/` (all source files)
- `crates/bardo-primitives/src/hdc.rs` (HDC implementation)
- `crates/roko-index/src/hdc.rs` (code symbol fingerprinting)
- `crates/roko-learn/src/hdc_clustering.rs` (K-medoids)
- `crates/roko-golem/src/grimoire.rs` (dissolved placeholder)
- `tmp/implementation-plans/12a-cognitive-layer.md` (72 implementation items)

---

## Abstract

Neuro's codebase has a solid foundation: the core data types (`KnowledgeEntry`, `KnowledgeKind`, `NeuroStore` trait), the JSONL storage backend (`KnowledgeStore`), the episode distillation pipeline (`Distiller`, `DistillationBackend`), and the tier progression system (`TierProgression` with D1/D2/D3 stages) are all implemented. The HDC vector library (`HdcVector` in `bardo-primitives`) is fully functional with bind, bundle, permute, similarity, and deterministic seeding.

However, significant gaps remain between the current implementation and the refactoring-prd design. The tier multiplier system (Transient/Working/Consolidated/Persistent) is designed but not implemented as a field on `KnowledgeEntry`. Several knowledge types from the design (Warning, CausalLink, StrategyFragment) are not yet in the `KnowledgeKind` enum. The ContextAssembler is a skeleton with no methods. And none of the frontier innovations (VCG attention auction, somatic landscape, active inference context selection) are implemented.

---

## Implemented Components

### roko-neuro (Knowledge Store)

| Component | File | Status | Lines |
|---|---|---|---|
| `KnowledgeKind` enum | `lib.rs` | **Implemented** — 7 variants: Fact, Insight, Procedure, Heuristic, Playbook, Constraint, AntiKnowledge | ~50 |
| `KnowledgeEntry` struct | `lib.rs` | **Implemented** — all fields including refuted_insight_id, refutation_evidence, hdc_vector | ~60 |
| `NeuroStore` trait | `lib.rs` | **Implemented** — init, query, ingest, decay, gc | ~20 |
| Half-life constants | `lib.rs` | **Implemented** — FACT=365d, INSIGHT=30d, HEURISTIC=90d; default 30d for others | ~30 |
| `refutation_warning()` | `lib.rs` | **Implemented** — generates warning text for AntiKnowledge entries | ~25 |
| `KnowledgeStore` (JSONL) | `knowledge_store.rs` | **Implemented** — append-only, with stats, optional HDC MemoryIndex | Large |
| `KnowledgeConfirmationRecord` | `knowledge_store.rs` | **Implemented** — tracks positive/negative confirmations | ~10 |
| `KnowledgeStats` | `knowledge_store.rs` | **Implemented** — total entries, per-kind counts, mean confidence | ~15 |
| `MemoryIndex` / `MemoryHit` | `knowledge_store.rs` | **Implemented** (feature-gated `hdc`) — HDC similarity search | ~40 |
| `Distiller` | `distiller.rs` | **Implemented** — LLM-based episode→knowledge extraction (Haiku default) | ~100 |
| `DistillationBackend` trait | `distiller.rs` | **Implemented** — async distill(episode) → Vec<KnowledgeEntry> | ~10 |
| `spawn_episode_distillation` | `episode_completion.rs` | **Implemented** — async task spawner | ~20 |
| `TierProgression` | `tier_progression.rs` | **Implemented** — analyze, extract_insights, promote_heuristics, compile_playbook, replay_heuristics | ~200 |
| `InsightRecord` | `tier_progression.rs` | **Implemented** — pattern, support, confidence, source_episodes | ~10 |
| `HeuristicRule` | `tier_progression.rs` | **Implemented** — rule, support, confidence, source_insights | ~10 |
| `PlaybookCompilation` | `tier_progression.rs` | **Implemented** — title, rules, markdown | ~10 |
| `ContextAssembler` struct | `context.rs` | **Skeleton** — struct defined (KnowledgeStore + EpisodeStore + budget), no methods | ~10 |

### bardo-primitives (HDC Vectors)

| Component | File | Status |
|---|---|---|
| `HdcVector` struct | `hdc.rs` | **Implemented** — `[u64; 160]`, 10,240 bits, 1,280 bytes |
| `bind()` (XOR) | `hdc.rs` | **Implemented** — componentwise XOR |
| `bundle()` (majority vote) | `hdc.rs` | **Implemented** — per-bit majority, tie→0 |
| `permute()` (cyclic shift) | `hdc.rs` | **Implemented** — cyclic bit rotation |
| `similarity()` (Hamming) | `hdc.rs` | **Implemented** — XOR + popcount, returns [0,1] |
| `from_seed()` | `hdc.rs` | **Implemented** — FNV-1a + splitmix64, deterministic |
| `to_bytes()` / `from_bytes()` | `hdc.rs` | **Implemented** — 1,280-byte LE serialization |
| serde support | `hdc.rs` | **Implemented** — serialize as raw bytes |
| rkyv zero-copy | `hdc.rs` | **Implemented** (feature-gated) — `similarity_archived()` |
| `fingerprint()` | `hdc.rs` | **Implemented** — serde_json→from_seed |
| `text_fingerprint()` | `hdc.rs` | **Implemented** — text→from_seed |

### roko-index (Code Symbol HDC)

| Component | Status |
|---|---|
| `HdcFingerprint` for code symbols | **Implemented** |
| Role vectors per `SymbolKind` | **Implemented** |
| Trigram-based name encoding | **Implemented** |
| `fingerprint_symbol()` | **Implemented** |
| `fingerprint_file()` | **Implemented** |

### roko-learn (HDC Clustering)

| Component | Status |
|---|---|
| K-medoids (PAM) over `HdcVector` | **Implemented** |
| Farthest-first seeding | **Implemented** |
| `KMedoidsConfig`, `HdcCluster`, `ClusterResult` | **Implemented** |

---

## Gaps: Not Yet Implemented

### Priority 1 — Core Knowledge System Gaps

| Gap | Where It Belongs | Blocking? | Implementation Plan Reference |
|---|---|---|---|
| **Tier field on KnowledgeEntry** | `roko-neuro/src/lib.rs` | Yes — tiers are the core validation mechanism | `12a-cognitive-layer.md` D1-D4 |
| **Tier multiplier computation** | `roko-neuro/src/knowledge_store.rs` | Yes — effective_half_life = tier × base | D5-D6 |
| **Warning type (7-day half-life)** | `roko-neuro/src/lib.rs` | Moderate — currently approximated by Constraint | D1 |
| **CausalLink type (60-day half-life)** | `roko-neuro/src/lib.rs` | Moderate — no current equivalent | D2 |
| **StrategyFragment type (14-day half-life)** | `roko-neuro/src/lib.rs` | Low — approximated by Procedure | D3 |
| **AntiKnowledge confidence floor (0.3)** | `roko-neuro/src/knowledge_store.rs` | Moderate — currently decays to 0 | D4 |
| **ContextAssembler methods** | `roko-neuro/src/context.rs` | Yes — context assembly is not functional | E1-E6 |
| **Combined retrieval scoring** | `roko-neuro/src/knowledge_store.rs` | Moderate — query uses simple matching, not confidence × decay × similarity | D7-D8 |

### Priority 2 — HDC Enhancement Gaps

| Gap | Where It Belongs | Blocking? |
|---|---|---|
| **BundleAccumulator** (incremental bundling) | `roko-primitives/src/hdc.rs` | No — `bundle()` exists for batch use |
| **ItemMemory** (concept codebook) | `roko-primitives/src/hdc.rs` | No — `from_seed()` serves as ad-hoc codebook |
| **ResonatorNetwork** (factor decomposition) | `roko-primitives/src/hdc.rs` | No — advanced feature |
| **DecayingBundleAccumulator** | `roko-primitives/src/hdc.rs` | No — controlled forgetting in bundles |
| **Three-tier search** (Bloom → approximate → exact) | `roko-neuro/src/knowledge_store.rs` | No — brute force is fast enough for <100K entries |
| **Automatic HDC encoding on ingest** | `roko-neuro/src/knowledge_store.rs` | Moderate — hdc_vector field is optional and often empty |
| **Role vector registry** | New module | Moderate — needed for structured encoding |

### Priority 3 — Frontier Innovation Gaps

| Gap | Design Source | Status |
|---|---|---|
| **VCG attention auction** | `09-innovations.md` §II | Designed, not implemented (Tier 2, P2) |
| **SomaticLandscape** (k-d tree, 8D) | `09-innovations.md` §III | Designed, not implemented |
| **Active inference context selection** | `09-innovations.md` §XIX.A-C | Designed, not implemented |
| **Cross-domain resonance detection** | `09-innovations.md` §XIII | Designed, not implemented |
| **Pheromone system** | `04-knowledge-and-mesh.md` §4 | Types designed, not implemented (Tier 5E, P2) |
| **Dream engine integration** | `03-cognitive-subsystems.md` §3 | Distillation is wired; Dreams cycle is not |
| **Backup/Restore CLI commands** | `04-knowledge-and-mesh.md` §5 | Not implemented |
| **Mesh sync** | `04-knowledge-and-mesh.md` §3 | Not implemented |
| **Korai chain integration** | `04-knowledge-and-mesh.md` §2 | Not implemented |

---

## Dissolved Components

| Component | Former Location | Status |
|---|---|---|
| `GrimoireEngine` | `roko-golem/src/grimoire.rs` | **Dissolved** — 44-line placeholder, superseded by `roko-neuro` |
| `GolemScaffold` | `roko-golem/src/lib.rs` | **Dissolved** — umbrella struct removed |
| `ScaffoldEngine` trait | `roko-golem/src/lib.rs` | **Dissolved** — each subsystem defines its own trait |
| Mortality engine | `roko-golem/src/mortality.rs` | **Deleted entirely** — no mortality in new architecture |

---

## Crate Rename Status

| Current Name | Target Name | Status |
|---|---|---|
| `bardo-primitives` | `roko-primitives` | **Pending** — crate still uses old name |
| `bardo-runtime` | `roko-runtime` | **Pending** — crate still uses old name |
| `roko-golem` | **Dissolved** | **In progress** — subsystems redistributed |

---

## Implementation Plan Mapping

The implementation plan (`12a-cognitive-layer.md`) specifies 72 items across four categories:

| Category | Items | Status |
|---|---|---|
| **D: Knowledge (D1-D18)** | KnowledgeEntry fields, tier system, decay, query, HDC encoding | ~40% implemented (core types and storage exist; tiers, advanced types, and encoding missing) |
| **E: Context (E1-E6)** | ContextAssembler methods, VCG auction, token budgeting | ~5% implemented (struct exists, no methods) |
| **F: Daimon (F1-F9)** | PAD vector, behavioral states, somatic markers | Separate crate (`roko-daimon`); not tracked here |
| **G: Dreams (G1-G8)** | Dream cycle, replay, consolidation | Separate crate (`roko-dreams`); not tracked here |
| **R: Crate Architecture (R1-R3)** | Crate restructuring, golem dissolution | ~50% implemented (neuro exists; primitives not yet renamed) |

---

## Recommended Implementation Order

Based on the gaps and their dependencies:

1. **Add tier field to KnowledgeEntry** — unblocks all tier-related features
2. **Add Warning, CausalLink, StrategyFragment types** — completes the type system
3. **Implement effective_half_life = tier × base** — activates the two-dimensional decay model
4. **Implement AntiKnowledge confidence floor** — prevents GC of negative knowledge
5. **Implement ContextAssembler methods** — enables knowledge-augmented prompts
6. **Add automatic HDC encoding on ingest** — enables similarity search
7. **Implement combined retrieval scoring** — confidence × decay × similarity
8. **Add SomaticLandscape** — enables emotional fast-path routing
9. **Add backup/restore CLI commands** — enables user-controlled knowledge management
10. **Add VCG attention auction** — enables optimal context allocation

---

## Academic Foundations

- McClelland, J. L., et al. (1995). "Complementary learning systems." *Psychological Review*, 102(3). (Theoretical basis for the dual-store architecture)
- Kanerva, P. (2009). "Hyperdimensional Computing." *Cognitive Computation*, 1(2). (HDC foundation)
- Park, J. S., et al. (2023). "Generative Agents." *UIST 2023*. (Agent memory architecture reference)
- Sumers, T. R., et al. (2023). "Cognitive Architectures for Language Agents." *arXiv:2309.02427*. (CoALA framework mapping)

---

## Cross-references

- See [00-vision-and-grimoire-rename.md](./00-vision-and-grimoire-rename.md) for the architectural vision
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for the current API surface
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for the pipeline that is already implemented
- See `tmp/implementation-plans/12a-cognitive-layer.md` for the full 72-item implementation plan
