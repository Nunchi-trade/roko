# Current Status and Implementation Gaps

> A comprehensive assessment of what exists in the roko-neuro codebase today, what is scaffolded, and what remains to be built — mapped against the refactoring-prd design and the implementation plan.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [00-vision-and-grimoire-rename.md](./00-vision-and-grimoire-rename.md)
**Key sources**:
- `crates/roko-neuro/src/` (all source files)
- `crates/roko-primitives/src/hdc.rs` (HDC implementation)
- `crates/roko-learn/src/hdc_clustering.rs` (K-medoids)
- `tmp/integrate-prds/` (current integration tracker)

---

## Abstract

Neuro's codebase has a solid foundation: the core data types (`KnowledgeEntry`, `KnowledgeKind`, `NeuroStore` trait), the JSONL storage backend (`KnowledgeStore`), the episode distillation pipeline (`Distiller`, `DistillationBackend`), and the tier progression system (`TierProgression` with D1/D2/D3 stages) are all implemented. The HDC vector library (`HdcVector` in `roko-primitives`) is fully functional with bind, bundle, permute, similarity, and deterministic seeding.

However, significant gaps remain between the current implementation and the refactoring-prd design. The tier multiplier system is now implemented on `KnowledgeEntry`, the canonical knowledge types now match the PRD (`Insight`, `Heuristic`, `Warning`, `CausalLink`, `StrategyFragment`, `AntiKnowledge`) with legacy names preserved only as serde aliases, the canonical `ContextAssembler` now lives in `roko-neuro`, AntiKnowledge now enforces its 0.3 confidence floor during decay/GC, CausalLinks now use directional HDC role bindings during ingest and retrieval, `KnowledgeEntry` now carries emotional provenance transferred from episodes and orchestrator-produced engrams, and Neuro's local context allocator now performs auction-style budget selection plus mood-congruent scoring, a contrarian affect slice, and a modest emotional-diversity reliability boost instead of naive truncation. Daimon now also has a real somatic landscape used to bias routing, but the final direct fusion of somatic scores into Neuro retrieval, the full cross-subsystem VCG attention auction, and fuller active-inference retrieval are still open.

---

## Implemented Components

### roko-neuro (Knowledge Store)

| Component | File | Status | Lines |
|---|---|---|---|
| `KnowledgeKind` enum | `lib.rs` | **Implemented** — canonical variants are Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge; legacy names deserialize via aliases | ~50 |
| `KnowledgeEntry` struct | `lib.rs` | **Implemented** — includes tier, refuted_insight_id, refutation_evidence, emotional_tag, emotional_provenance, hdc_vector | ~60 |
| `NeuroStore` trait | `lib.rs` | **Implemented** — init, query, ingest, decay, gc | ~20 |
| Half-life constants | `lib.rs` | **Implemented** — INSIGHT=30d, HEURISTIC=90d, WARNING=7d, CAUSAL_LINK=60d, STRATEGY_FRAGMENT=14d | ~30 |
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
| `ContextAssembler` | `context.rs` | **Implemented** — canonical gather/rank/compress pipeline with PAD biasing, contrarian affect retention, and auction-style token allocation; re-exported by `roko-compose` | ~450 |

### roko-primitives (HDC Vectors)

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
| **AntiKnowledge confidence floor (0.3)** | `roko-neuro/src/knowledge_store.rs` | Implemented — decay clamps at 0.3 and GC preserves AntiKnowledge entries | D4 |
| **ContextAssembler methods** | `roko-neuro/src/context.rs` | Implemented — canonicalized into `roko-neuro`; local chunk auctioning is now present, but cross-subsystem auctioning is still missing | E1-E6 |
| **Combined retrieval scoring** | `roko-neuro/src/knowledge_store.rs` | Moderate — query now mixes confidence, confirmation, and emotional-diversity reliability, but still lacks the fuller decay × similarity × cross-subsystem PRD model | D7-D8 |

### Priority 2 — HDC Enhancement Gaps

| Gap | Where It Belongs | Blocking? |
|---|---|---|
| **BundleAccumulator** (incremental bundling) | `roko-primitives/src/hdc.rs` | No — `bundle()` exists for batch use |
| **ItemMemory** (concept codebook) | `roko-primitives/src/hdc.rs` | No — `from_seed()` serves as ad-hoc codebook |
| **ResonatorNetwork** (factor decomposition) | `roko-primitives/src/hdc.rs` | No — advanced feature |
| **DecayingBundleAccumulator** | `roko-primitives/src/hdc.rs` | No — controlled forgetting in bundles |
| **Three-tier search** (Bloom → approximate → exact) | `roko-neuro/src/knowledge_store.rs` | No — brute force is fast enough for <100K entries |
| **Automatic HDC encoding on ingest** | `roko-neuro/src/knowledge_store.rs` | Implemented — ingest populates `hdc_vector` when the `hdc` feature is enabled |
| **Role vector registry / typed HDC encoder** | `roko-neuro/src/hdc.rs` | Implemented at a lightweight level — directional CausalLink encoding and query probing now live behind a dedicated encoder module |

### Priority 3 — Frontier Innovation Gaps

| Gap | Design Source | Status |
|---|---|---|
| **VCG attention auction** | `09-innovations.md` §II | Partially implemented — Neuro now does auction-style chunk allocation internally; the full multi-subsystem VCG market is still not implemented (Tier 2, P2) |
| **SomaticLandscape** (k-d tree, 8D) | `09-innovations.md` §III | Designed, not implemented |
| **Active inference context selection** | `09-innovations.md` §XIX.A-C | Partially implemented — track-record/uncertainty scoring, auction-style budgeting, and contrarian affect retention are present; full EFE-style subsystem integration is not |
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
| `bardo-primitives` | `roko-primitives` | **Completed** |
| `bardo-runtime` | `roko-runtime` | **Completed** |
| `roko-golem` | **Dissolved** | **Completed** — subsystems redistributed |

---

## Implementation Plan Mapping

The implementation plan (`12a-cognitive-layer.md`) specifies 72 items across four categories:

| Category | Items | Status |
|---|---|---|
| **D: Knowledge (D1-D18)** | KnowledgeEntry fields, tier system, decay, query, HDC encoding | ~40% implemented (core types and storage exist; tiers, advanced types, and encoding missing) |
| **E: Context (E1-E6)** | ContextAssembler methods, VCG auction, token budgeting | ~45% implemented (base retrieval pipeline exists; auctioning/frontier selection missing) |
| **F: Daimon (F1-F9)** | PAD vector, behavioral states, somatic markers | Separate crate (`roko-daimon`); not tracked here |
| **G: Dreams (G1-G8)** | Dream cycle, replay, consolidation | Separate crate (`roko-dreams`); not tracked here |
| **R: Crate Architecture (R1-R3)** | Crate restructuring, golem dissolution | ~50% implemented (neuro exists; primitives not yet renamed) |

---

## Recommended Implementation Order

Based on the gaps and their dependencies:

1. **Add tier field to KnowledgeEntry** — unblocks all tier-related features
2. **Add Warning, CausalLink, StrategyFragment types** — completes the type system
3. **Implement effective_half_life = tier × base** — activates the two-dimensional decay model
4. **Deepen structured HDC semantics** — directional causal encoding is in place; richer codebooks, multi-stage search, and resonance are still open
5. **Complete cross-subsystem VCG / higher-order context allocation** — base ContextAssembler is now present and local chunk auctioning has landed
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

## Cross-References

- See [00-vision-and-grimoire-rename.md](./00-vision-and-grimoire-rename.md) for the architectural vision
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for the current API surface
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for the pipeline that is already implemented
- See `tmp/implementation-plans/12a-cognitive-layer.md` for the full 72-item implementation plan

---

## Frontier Concepts: Knowledge Crystals and Knowledge Metabolism

### Knowledge Crystals: Ultra-Compressed Knowledge Units

A **knowledge crystal** is an ultra-compressed, high-value knowledge unit that has been distilled, validated, and optimized to the point where it carries maximum information in minimum space. Crystals are the final stage of knowledge evolution — beyond Playbooks.

**Biological analogy**: In neuroscience, highly practiced skills become "crystallized intelligence" (Cattell 1963) — they are fast, automatic, and resistant to decay. Knowledge crystals are the computational equivalent.

**Properties**:
- **Atomic**: Each crystal encodes exactly one actionable principle
- **Self-contained**: No external context needed to apply the crystal
- **Maximally compressed**: HDC vector + one-line natural language + metadata, total < 2 KB
- **Near-permanent**: Effective half-life > 10 years (requires explicit revocation to remove)
- **Cross-domain**: Crystals are abstract enough to transfer across domains

```rust
/// A knowledge crystal: the most compressed, highest-value unit of knowledge.
///
/// Crystals emerge from the tier progression pipeline when:
///   1. A Persistent-tier Heuristic has been confirmed 50+ times
///   2. It has never been contradicted by AntiKnowledge
///   3. It has been independently confirmed by 3+ agents
///   4. Its content has been compressed to a single actionable principle
///
/// Crystals are stored separately from regular knowledge entries for
/// fast lookup — they are always in memory, never paged out.
pub struct KnowledgeCrystal {
    /// Unique identifier.
    pub id: String,
    /// The principle, in one sentence. Max 200 characters.
    pub principle: String,
    /// HDC vector encoding the crystal's semantic structure.
    pub hdc_vector: HdcVector,
    /// Confidence: always >= 0.95 for crystals.
    pub confidence: f64,
    /// Number of independent confirmations.
    pub confirmation_count: usize,
    /// Domains where this crystal has been validated.
    pub validated_domains: Vec<String>,
    /// Source heuristic IDs that were crystallized.
    pub source_heuristics: Vec<String>,
    /// Creation timestamp.
    pub crystallized_at: DateTime<Utc>,
    /// Provenance chain.
    pub provenance: ProvenanceChain,
}

/// Crystal store: always-in-memory, fast-lookup knowledge.
///
/// Maximum size: 1,000 crystals (beyond this, the least-used crystals
/// are de-crystallized back to Persistent Heuristics).
///
/// Memory footprint: 1,000 × ~2 KB = ~2 MB — small enough to stay
/// resident in L3 cache on modern processors.
pub struct CrystalStore {
    crystals: Vec<KnowledgeCrystal>,
    /// Maximum crystals to retain. Default: 1000.
    pub max_crystals: usize,
    /// Minimum confirmations for crystallization. Default: 50.
    pub min_confirmations: usize,
    /// Minimum confidence for crystallization. Default: 0.95.
    pub min_confidence: f64,
}

impl CrystalStore {
    /// Attempt to crystallize a Persistent-tier Heuristic.
    ///
    /// Criteria:
    ///   1. Tier == Persistent
    ///   2. Confirmation count >= min_confirmations (50)
    ///   3. Confidence >= min_confidence (0.95)
    ///   4. No AntiKnowledge contradictions
    ///   5. Confirmed by >= 3 independent agents (or >= 3 distinct contexts)
    pub fn try_crystallize(
        &mut self,
        entry: &KnowledgeEntry,
        stats: &EntryStats,
        anti_entries: &[KnowledgeEntry],
    ) -> Option<KnowledgeCrystal> {
        if stats.confirmation_count < self.min_confirmations {
            return None;
        }
        if entry.confidence < self.min_confidence {
            return None;
        }
        // Check no AntiKnowledge contradictions
        if let Some(hv) = entry.hdc_vector.as_ref()
            .and_then(|b| HdcVector::from_bytes(b)) {
            for anti in anti_entries {
                if let Some(anti_hv) = anti.hdc_vector.as_ref()
                    .and_then(|b| HdcVector::from_bytes(b)) {
                    if hv.similarity(&anti_hv) > 0.526 {
                        return None; // Contradicted
                    }
                }
            }
        }

        Some(KnowledgeCrystal {
            id: format!("crystal_{}", uuid::Uuid::new_v4()),
            principle: entry.content.chars().take(200).collect(),
            hdc_vector: entry.hdc_vector.as_ref()
                .and_then(|b| HdcVector::from_bytes(b))
                .unwrap_or_else(HdcVector::zeros),
            confidence: entry.confidence,
            confirmation_count: stats.confirmation_count,
            validated_domains: vec![], // populated from source episodes
            source_heuristics: vec![entry.id.clone()],
            crystallized_at: Utc::now(),
            provenance: ProvenanceChain::new_crystallized(&entry.id),
        })
    }
}
```

**References**: Cattell, R.B. (1963). "Theory of fluid and crystallized intelligence." *Journal of Educational Psychology*, 54(1), 1-22.

---

### Knowledge Metabolism: Knowledge as a Living Substrate

Knowledge is not static data — it is a **metabolizing substrate** that consumes resources (attention, storage, compute), produces outputs (better decisions), and generates waste (stale entries, false beliefs). The metabolic model provides a unified framework for understanding knowledge base health.

**Biological analogy**: Living cells maintain homeostasis through metabolic processes — anabolism (building complex molecules from simple ones) and catabolism (breaking down complex molecules for energy). Knowledge metabolism operates similarly:

| Metabolic Process | Biological | Knowledge |
|---|---|---|
| **Anabolism** (building up) | Protein synthesis | Distillation: episodes → insights → heuristics → crystals |
| **Catabolism** (breaking down) | Digestion, waste removal | Decay + GC: stale entries → garbage collected |
| **Homeostasis** | Temperature regulation | Tier system: balance between acquisition and pruning |
| **Energy** | ATP | Agent compute budget (LLM tokens, CPU cycles) |
| **Nutrients** | Food | New episodes, external knowledge imports |
| **Waste** | CO2, urea | Decayed entries below GC threshold |
| **Immune system** | White blood cells | AntiKnowledge, reactive checking |
| **Growth** | Cell division | Knowledge base expansion through distillation |
| **Aging** | Cellular senescence | Ebbinghaus decay |

```rust
/// Knowledge metabolism metrics.
///
/// Tracks the metabolic health of the knowledge base.
/// Computed periodically and logged for trend analysis.
pub struct MetabolismMetrics {
    /// Anabolic rate: new knowledge entries per day.
    pub anabolic_rate: f64,
    /// Catabolic rate: entries garbage-collected per day.
    pub catabolic_rate: f64,
    /// Metabolic balance: anabolic - catabolic.
    /// Positive = growing. Negative = shrinking. Near zero = homeostasis.
    pub metabolic_balance: f64,
    /// Metabolic efficiency: fraction of new entries that survive to Working tier.
    /// Target: 0.3-0.5. Below 0.2 = wasteful (too many false extractions).
    /// Above 0.7 = too conservative (missing patterns).
    pub metabolic_efficiency: f64,
    /// Energy expenditure: LLM tokens consumed for distillation per day.
    pub daily_token_expenditure: usize,
    /// Knowledge density: mean confidence × mean tier_multiplier.
    /// Higher density = more concentrated, validated knowledge.
    pub knowledge_density: f64,
    /// Waste ratio: fraction of entries below 0.2 confidence.
    /// High waste ratio = GC is not running frequently enough.
    pub waste_ratio: f64,
    /// Immune load: fraction of entries that are AntiKnowledge.
    /// Target: 0.05-0.15. Below 0.05 = under-defended. Above 0.20 = too many
    /// false beliefs being tracked (possible overactive immune system).
    pub immune_load: f64,
}

/// Metabolic state classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetabolicState {
    /// Healthy: balanced anabolism/catabolism, good efficiency.
    Homeostasis,
    /// Growing: anabolic rate exceeds catabolic rate. Normal during active work.
    Growth,
    /// Consolidating: catabolic rate exceeds anabolic rate. Normal during idle/Dreams.
    Consolidation,
    /// Starving: no new knowledge being created. Agent needs more episodes.
    Starvation,
    /// Bloated: too many low-confidence entries accumulating. Needs GC.
    Bloat,
    /// Inflamed: high immune load (too much AntiKnowledge). May indicate
    /// systematic distillation errors or adversarial injection.
    Inflammation,
}

impl MetabolismMetrics {
    /// Classify the current metabolic state.
    pub fn state(&self) -> MetabolicState {
        if self.anabolic_rate < 0.1 {
            return MetabolicState::Starvation;
        }
        if self.waste_ratio > 0.3 {
            return MetabolicState::Bloat;
        }
        if self.immune_load > 0.20 {
            return MetabolicState::Inflammation;
        }
        if self.metabolic_balance > 5.0 {
            return MetabolicState::Growth;
        }
        if self.metabolic_balance < -5.0 {
            return MetabolicState::Consolidation;
        }
        MetabolicState::Homeostasis
    }

    /// Recommend actions based on metabolic state.
    pub fn recommendations(&self) -> Vec<String> {
        let mut recs = Vec::new();
        match self.state() {
            MetabolicState::Starvation => {
                recs.push("Knowledge base is starving. Run more tasks to generate episodes.".into());
            }
            MetabolicState::Bloat => {
                recs.push(format!(
                    "Knowledge base is bloated ({:.0}% waste). Run GC immediately.",
                    self.waste_ratio * 100.0
                ));
            }
            MetabolicState::Inflammation => {
                recs.push(format!(
                    "High immune load ({:.0}% AntiKnowledge). Check for distillation errors or poisoning.",
                    self.immune_load * 100.0
                ));
            }
            _ => {}
        }
        if self.metabolic_efficiency < 0.2 {
            recs.push("Low metabolic efficiency. Distillation is producing too many false patterns.".into());
        }
        if self.daily_token_expenditure > 100_000 {
            recs.push("High token expenditure on distillation. Consider batching or reducing D1 frequency.".into());
        }
        recs
    }
}
```

---

### Neurosymbolic Integration: Bridging HDC and Knowledge Graphs

Neuro's HDC encoding captures structural relationships algebraically. Knowledge graphs (Neo4j, TypeDB, RDF/OWL) capture relationships symbolically. The neurosymbolic integration layer bridges both, using each for its strength.

**Research context**: The neurosymbolic AI field has matured rapidly (Shams et al. 2024 surveyed 158 papers from 2020-2024). AlphaGeometry (DeepMind 2024) demonstrated that combining neural pattern recognition (System 1) with symbolic deduction (System 2) can achieve IMO silver-medal-equivalent performance. GraphRAG (Microsoft 2024) showed that knowledge graphs + vector search improves LLM accuracy by 20%+ over vector-only RAG.

```rust
/// Neurosymbolic knowledge layer: HDC vectors + symbolic graph.
///
/// Each knowledge entry exists in two representations:
///   1. HDC vector (10,240-bit BSC): fast similarity, cross-domain transfer
///   2. Symbolic triple (subject, predicate, object): precise queries, inference
///
/// The symbolic layer enables:
///   - Multi-hop queries: "what causes X which causes Y?"
///   - Logical inference: "if A implies B and B implies C, then A implies C"
///   - Constraint checking: "this new entry contradicts rule R"
///
/// The HDC layer enables:
///   - Fast approximate similarity: "what entries are related to this query?"
///   - Cross-domain transfer: "what structural analogy exists between domains?"
///   - Memory compression: "summarize these 100 entries in one vector"
pub struct NeurosymbolicStore {
    /// HDC vector index (the existing NeuroStore).
    pub vector_store: NeuroStore,
    /// Symbolic triple store (knowledge graph).
    pub graph_store: SymbolicGraph,
    /// Mapping between entry IDs and graph node IDs.
    pub id_mapping: HashMap<String, GraphNodeId>,
}

/// A symbolic triple in the knowledge graph.
pub struct SymbolicTriple {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    /// Source knowledge entry ID.
    pub source_entry_id: String,
    /// Confidence inherited from the knowledge entry.
    pub confidence: f64,
}

/// Symbolic graph interface (abstraction over graph backends).
pub trait SymbolicGraph: Send + Sync {
    /// Add a triple to the graph.
    fn add_triple(&mut self, triple: SymbolicTriple) -> Result<GraphNodeId>;
    /// Query triples matching a pattern (None = wildcard).
    fn query(&self, subject: Option<&str>, predicate: Option<&str>, object: Option<&str>)
        -> Result<Vec<SymbolicTriple>>;
    /// Multi-hop query: follow predicate chains up to max_hops.
    fn multi_hop(&self, start: &str, predicate: &str, max_hops: usize)
        -> Result<Vec<Vec<SymbolicTriple>>>;
    /// Inference: apply transitive closure over a predicate.
    fn transitive_closure(&self, predicate: &str) -> Result<Vec<SymbolicTriple>>;
}

impl NeurosymbolicStore {
    /// Hybrid query: HDC similarity + graph traversal.
    ///
    /// Pipeline:
    ///   1. HDC similarity search → top-K candidates by structure
    ///   2. Graph expansion → follow relationships from candidates
    ///   3. Re-rank by combined score (HDC sim × graph relevance)
    pub fn hybrid_query(
        &self,
        query_text: &str,
        limit: usize,
    ) -> Result<Vec<HybridResult>> {
        // 1. HDC similarity search
        let hdc_candidates = self.vector_store.query(query_text, limit * 2)?;

        // 2. Graph expansion: for each candidate, find related triples
        let mut expanded = Vec::new();
        for candidate in &hdc_candidates {
            if let Some(node_id) = self.id_mapping.get(&candidate.id) {
                let related = self.graph_store.multi_hop(
                    &candidate.id, "relates_to", 2
                )?;
                expanded.extend(related.into_iter().flatten());
            }
        }

        // 3. Re-rank and deduplicate
        // ... scoring logic combining HDC similarity and graph distance

        Ok(vec![]) // placeholder
    }
}
```

**When to use HDC vs Graph vs Both**:

| Query Type | Best Approach | Example |
|---|---|---|
| Semantic similarity | HDC | "find entries about async performance" |
| Exact relationship | Graph | "what causes borrow checker errors?" |
| Cross-domain analogy | HDC | "what in DeFi is like borrow checker in Rust?" |
| Multi-hop reasoning | Graph | "what causes X which leads to Y which affects Z?" |
| Approximate + precise | Both (hybrid) | "find performance-related entries and their causal chains" |

**References**:
- Shams, Z. et al. (2024). "Neuro-Symbolic AI in 2024: A Systematic Review." arXiv:2501.05435.
- Trinh, T.H. et al. (2024). "Solving olympiad geometry without human demonstrations." *Nature*, 625, 476-482. (AlphaGeometry)
- Edge, D. et al. (2024). "From Local to Global: A Graph RAG Approach." arXiv:2404.16130. (GraphRAG)
- Zhang, X. et al. (2024). "Neural-Symbolic Methods for Knowledge Graph Reasoning." *ACM TKDD*. DOI:10.1145/3686806.
