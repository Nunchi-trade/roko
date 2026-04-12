# Vision and the Grimoire-to-Neuro Rename

> Neuro is the agent's persistent, tiered, HDC-indexed knowledge system — a semantic wrapper around Substrate that adds knowledge-specific logic for classification, decay, and similarity search.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Synapse Architecture concepts (Engrams, 6 traits, Substrate)
**Key sources**:
- `refactoring-prd/03-cognitive-subsystems.md` §1 (Neuro architecture note)
- `refactoring-prd/04-knowledge-and-mesh.md` §1 (Knowledge Architecture)
- `context-pack/01-naming-map.md` (Grimoire → Neuro rename)
- `context-pack/03-concepts-lifecycle.md` (concept evolution)
- `bardo-backup/prd/04-memory/01-grimoire.md` (original Grimoire design)
- `crates/roko-neuro/src/lib.rs` (current implementation)
- `crates/roko-golem/src/grimoire.rs` (dissolved placeholder)

---

## Abstract

Neuro (`roko-neuro`) is the knowledge subsystem of the Roko agent framework. It provides persistent, tiered, HDC-indexed storage for everything an agent learns during its operational lifetime — from transient observations through consolidated heuristics to compiled playbooks. Neuro is not a database; it is a cognitive layer that classifies knowledge by type, validates it through use, decays it over time via Ebbinghaus curves, and makes it retrievable via hyperdimensional computing similarity search.

The subsystem was originally called "Grimoire" (now Neuro) in the legacy Bardo (now Roko) architecture. The rename reflects a shift from mystical framing to neuroscience-inspired terminology. The underlying mechanisms — six knowledge types, tier progression, HDC encoding, exponential decay — are preserved intact. Only the naming and certain lifecycle concepts (succession, mortality-driven consolidation) have changed.

Neuro occupies a unique architectural position: it is a **semantic wrapper** around `Substrate` (the generic Engram storage trait from the Synapse Architecture), not a replacement for it. Neuro calls `Substrate.put()` and `Substrate.query()` underneath, but adds knowledge-specific logic on top: type classification (Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge), tier progression (Transient → Working → Consolidated → Persistent), HDC encoding for sub-millisecond similarity search, and Ebbinghaus decay with tier multipliers. Think of Neuro as a domain-specific indexing layer that gives raw Engram storage the semantics of a knowledge management system.

This document covers the vision for Neuro, the rename from Grimoire, the architectural relationship to the Synapse Architecture, and the dissolution of the `roko-golem` crate that formerly housed the Grimoire placeholder.

---

## The Grimoire-to-Neuro Rename

### Historical Context

In the original Bardo (now Roko) architecture, the knowledge subsystem was called **Grimoire** — a grimoire being a book of magical knowledge. The naming belonged to a broader mystical/occult naming theme: Bardo (intermediate state between lives), Golem (animated entity), Grimoire (book of spells), Styx (river of the underworld), and so on.

The rename to **Neuro** reflects several architectural decisions:

1. **Neuroscience grounding over mysticism**: Roko's cognitive architecture draws heavily from neuroscience research — Complementary Learning Systems theory (McClelland et al. 1995), Ebbinghaus forgetting curves (Ebbinghaus 1885), hippocampal replay (Mattar & Daw 2018), somatic markers (Damasio 1994). The name "Neuro" makes this intellectual lineage explicit.

2. **Domain-agnostic framing**: "Grimoire" implies a single entity's private spellbook. "Neuro" implies a general cognitive capability that any agent in any domain can use — a coding agent's Neuro works the same way as a research agent's Neuro or a chain agent's Neuro.

3. **Composability emphasis**: In the new architecture, Neuro is one of several cognitive cross-cuts (alongside Daimon for motivation/affect and Dreams for offline consolidation). The "Neuro" name signals that this is one component of a cognitive system, not a standalone monolith.

### Naming Map

| Old Name | New Name | Scope |
|---|---|---|
| Grimoire | **Neuro** | Subsystem name in prose and documentation |
| `GrimoireEngine` | **`NeuroStore`** | Primary Rust trait/struct |
| `grimoire.rs` | **`knowledge_store.rs`** (in `roko-neuro`) | Implementation file |
| `roko-golem/grimoire.rs` | **Dissolved** | Placeholder deleted; `roko-neuro` is the canonical location |
| `GrimoireEntry` | **`KnowledgeEntry`** | Core data type |
| `GrimoireKind` | **`KnowledgeKind`** | Entry type enum |
| Grimoire Curator | **Tier Progression pipeline** | Automated knowledge lifecycle |

### What Changed and What Stayed

**Preserved intact** (mechanisms and research are identical):
- Six knowledge types with distinct half-lives
- Four validation tiers with multiplicative decay
- HDC encoding for similarity search (10,240-bit Binary Spatter Codes)
- Ebbinghaus exponential decay model
- AntiKnowledge as a first-class knowledge type
- Confidence-weighted retrieval
- Provenance tracking via source episodes
- Append-only JSONL storage format

**Changed** (framing and lifecycle):
- Succession (dying agent transfers knowledge to offspring) → **Backup/Restore** (user exports NeuroStore, creates new agent, imports selected pieces)
- Mortality-driven consolidation → **Idle-time and scheduled consolidation** (Dreams subsystem)
- Generational inheritance → **User-controlled data management**
- Vitality-phase-aware retrieval → **Behavioral-state-aware retrieval** (Daimon PAD vector)
- Death-accelerated decay → **Standard Ebbinghaus decay** (no mortality input)

---

## Architectural Position: Semantic Wrapper Around Substrate

### The Synapse Architecture Context

Roko's kernel is the **Synapse Architecture**: one noun (Engram) and six verb traits (Substrate, Scorer, Gate, Router, Composer, Policy). Every capability in Roko flows through these six traits. Neuro is not an exception — it builds on top of them.

The `Substrate` trait provides generic, content-addressed Engram storage:

```rust
/// Generic Engram storage and retrieval.
pub trait Substrate: Send + Sync {
    /// Store an Engram, returning its content-addressed hash.
    fn put(&mut self, engram: Engram) -> Result<ContentHash>;

    /// Query Engrams by predicate.
    fn query(&self, predicate: &Query) -> Result<Vec<Engram>>;

    /// Retrieve a specific Engram by hash.
    fn get(&self, hash: &ContentHash) -> Result<Option<Engram>>;
}
```

`Substrate` knows nothing about knowledge types, tiers, decay rates, or HDC vectors. It stores and retrieves Engrams — that is all.

### Where Neuro Sits

Neuro adds a knowledge-specific semantic layer on top of `Substrate`:

```
┌─────────────────────────────────────────────────────┐
│                   Application Layer                  │
│  (Orchestrator, Agents, Dreams, Context Assembly)    │
├─────────────────────────────────────────────────────┤
│                   Neuro (roko-neuro)                 │  ← Knowledge semantics
│  KnowledgeEntry, KnowledgeKind, tier progression,   │
│  HDC encoding, Ebbinghaus decay, NeuroStore trait    │
├─────────────────────────────────────────────────────┤
│                   Substrate (roko-core)              │  ← Generic Engram storage
│  put(), query(), get() — content-addressed, typed    │
├─────────────────────────────────────────────────────┤
│                   FileSubstrate (roko-fs)            │  ← Physical storage
│  Append-only JSONL at .roko/signals.jsonl            │
└─────────────────────────────────────────────────────┘
```

Neuro uses `Substrate.put()` to persist knowledge entries as Engrams, and `Substrate.query()` to retrieve them. It adds:

1. **Type classification**: Every knowledge entry has a `KnowledgeKind` (Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge) that determines its base half-life and retrieval behavior.

2. **Tier progression**: Every entry has a validation tier (Transient, Working, Consolidated, Persistent) that multiplies the base half-life. Tiers promote and demote based on outcome feedback.

3. **HDC encoding**: Every entry optionally carries a 10,240-bit hyperdimensional computing vector for sub-millisecond similarity search via Hamming distance.

4. **Ebbinghaus decay**: Entries decay exponentially over time. The decay rate is `tier_multiplier × type_base_half_life`. Low-confidence entries are garbage-collected.

5. **Confidence tracking**: Each entry has a confidence score (0.0–1.0) and a confidence weight (signed retrieval weight). Confirmation boosts confidence; contradiction reduces it.

### Neuro Across Layers

In the five-layer taxonomy (L0 Runtime → L4 Orchestration), Neuro spans multiple layers via trait objects:

| Layer | Neuro's Role |
|---|---|
| **L0 Runtime** | NeuroStore persistence (JSONL file I/O, garbage collection) |
| **L1 Framework** | KnowledgeEntry types, HDC encoding, decay computation |
| **L2 Scaffold** | Context assembly draws from Neuro to populate agent prompts |
| **L3 Harness** | Gate results feed back to Neuro for tier promotion/demotion |
| **L4 Orchestration** | Orchestrator wires episode completion → distillation → Neuro ingestion |

This cross-layer presence is characteristic of cognitive cross-cuts — Neuro is not confined to a single layer but is injected into multiple layers through interfaces and callbacks.

---

## Dissolution of roko-golem

### The Former Umbrella Crate

In the legacy architecture, `roko-golem` served as an umbrella crate that aggregated multiple cognitive subsystems under a single `GolemScaffold` struct:

```rust
// roko-golem/src/lib.rs (DISSOLVED — historical reference only)
pub struct GolemScaffold {
    pub daimon: DaimonEngine,
    pub grimoire: GrimoireEngine,  // now Neuro
    pub dreams: DreamEngine,
    pub chain_witness: ChainWitness,
    pub mortality: MortalityEngine,  // REMOVED entirely
}
```

This umbrella pattern created tight coupling between subsystems that should be independent. The `roko-golem` crate has been dissolved, and its subsystems redistributed:

| Subsystem | Former Location | New Location | Status |
|---|---|---|---|
| Grimoire (44 lines, placeholder) | `roko-golem/grimoire.rs` | **`roko-neuro`** | `roko-neuro` is the canonical implementation; placeholder deleted |
| Daimon (972 lines, full impl) | `roko-golem/daimon.rs` | **`roko-daimon`** | Full implementation moved |
| Dreams (43 lines, placeholder) | `roko-golem/dreams.rs` | **`roko-dreams`** | Placeholder deleted; `roko-dreams` expanded |
| Chain Witness (43 lines, placeholder) | `roko-golem/chain_witness.rs` | **`roko-chain`** | Moved as `chain_witness` module |
| Mortality (44 lines, placeholder) | `roko-golem/mortality.rs` | **Deleted entirely** | No mortality in the new architecture |
| Hypnagogia (42 lines, placeholder) | `roko-golem/hypnagogia.rs` | **`roko-dreams`** | Moved as `hypnagogia` module |
| `ScaffoldEngine` trait | `roko-golem/lib.rs` | **Deleted** | Each subsystem defines its own trait |
| `GolemScaffold` aggregator | `roko-golem/lib.rs` | **Deleted** | Composition at application layer via config |

### Composability Principle

After dissolution, any subsystem can pipe to any other through the 6 Synapse traits:

- **Daimon** emits Engrams (affect state changes) → **Neuro** stores them (as knowledge about the agent's own cognitive patterns).
- **Dreams** reads from **Neuro** (knowledge entries to consolidate) → produces new Engrams (synthesized insights, promoted heuristics).
- **Chain** posts Engrams on-chain (publishing knowledge to Korai) → other agents query them.
- **Neuro** receives entries from episode distillation, Dream consolidation, mesh sharing, and user restore — all through the same `NeuroStore.ingest()` interface.

No umbrella crate is needed. Everything flows through the 6 Synapse traits. Composition happens at the application layer (in `roko.toml` configuration and the orchestrator's wiring), not in a monolithic aggregator struct.

---

## The Grimoire Placeholder (Historical)

For reference, the `roko-golem/src/grimoire.rs` file that has been superseded contained only a scaffold:

```rust
// roko-golem/src/grimoire.rs (DISSOLVED — historical reference)
// This was a 44-line placeholder with no actual implementation.
// The real implementation lives in roko-neuro/src/knowledge_store.rs.

pub struct GrimoireEngine {
    // Empty — all fields were placeholders
}

impl GrimoireEngine {
    pub fn new() -> Self {
        Self {}
    }
}
```

This scaffold had no functional code. The actual knowledge store implementation — with JSONL persistence, HDC indexing, decay computation, confidence tracking, and garbage collection — has always lived in `roko-neuro`. The scaffold existed only because the `GolemScaffold` umbrella struct required all subsystems to be present.

---

## Neuro's Design Principles

### 1. Knowledge as First-Class Data

Knowledge entries are not raw text blobs. They are typed, structured, versioned data objects with:
- A semantic category (`KnowledgeKind`) that determines behavior
- A validation tier that tracks reliability
- A provenance chain (source episodes) that enables forensic replay
- A confidence score that reflects accumulated evidence
- An optional HDC vector that enables sub-millisecond similarity search
- A temporal half-life that models knowledge staleness

### 2. Ebbinghaus Over Heuristics

Knowledge decay follows Hermann Ebbinghaus's forgetting curve (1885), not ad-hoc expiration rules. The exponential decay model `weight = exp(-age / (strength × scale))` is grounded in over a century of memory research. Tier multipliers and type-specific half-lives provide two orthogonal knobs for controlling decay rate, composing multiplicatively into an effective half-life that reflects both the nature of the knowledge (type) and the confidence in it (tier).

### 3. HDC for Retrieval, Not Embeddings

Neuro uses 10,240-bit Binary Spatter Code vectors (Kanerva 2009, Kleyko et al. 2022) for similarity search — not neural network embeddings. HDC vectors have several advantages for this use case:
- **Deterministic encoding**: Same input always produces the same vector. No model drift.
- **Algebraic composability**: Bind (XOR), Bundle (majority vote), and Permute (cyclic shift) enable structured encoding of relationships.
- **Sub-millisecond comparison**: Hamming distance on 10,240-bit vectors takes ~13ns with SIMD (XOR 160 u64 words + popcount).
- **No external dependency**: HDC encoding runs locally with zero LLM cost. No API calls, no GPU, no vector database.
- **Cross-domain transfer**: Structural analogies are naturally detected because compositional structure is preserved in the vector space.

### 4. User-Controlled Lifecycle

Knowledge lifecycle is user-controlled, not agent-directed. Users can:
- **Back up** the full NeuroStore (`roko neuro backup`)
- **Restore** selected entries into a new agent
- **Publish** non-sensitive entries to the Agent Mesh or Korai chain
- **Delete** entries that are incorrect or outdated

Agents do not "die" and do not "choose successors." Knowledge transfer is an explicit, auditable data management operation. This replaces the legacy succession model where a dying agent (formerly "Golem") selected knowledge to pass to an offspring.

### 5. Domain-Agnostic Core

Neuro's six knowledge types, four tiers, HDC encoding, and Ebbinghaus decay are domain-agnostic. A coding agent stores insights about borrow checker patterns; a chain agent stores insights about gas price correlations; a research agent stores insights about source reliability. The types and mechanisms are identical — only the content differs.

Domain-specific behavior is provided by:
- **Domain-specific knowledge templates** in `roko.toml` configuration
- **Domain-specific HDC role vectors** in the encoding layer (see `roko-index/src/hdc.rs` for code symbol encoding)
- **Domain-specific somatic landscape axes** (8D strategy space is configurable per domain — see `09-innovations.md` §XIX.F)

---

## Neuroscience Foundations

Neuro's design draws from several neuroscience research traditions:

### Complementary Learning Systems (CLS) Theory

McClelland, McNaughton, and O'Reilly (1995) proposed that the brain uses two complementary learning systems: a fast-learning hippocampal system for episodic memory and a slow-learning neocortical system for semantic memory. Consolidation transfers patterns from fast to slow storage during sleep.

Neuro implements CLS directly:
- **Fast system**: Episode logs (`.roko/episodes.jsonl`) capture raw agent turns immediately
- **Slow system**: NeuroStore (`.roko/neuro/knowledge.jsonl`) stores consolidated knowledge entries
- **Consolidation**: The distillation pipeline and tier progression move patterns from episodes to knowledge entries, with the Dreams subsystem driving offline consolidation during idle time

### Ebbinghaus Forgetting Curve

Hermann Ebbinghaus (1885) demonstrated that memory strength decays exponentially over time, with the rate depending on the strength of the original encoding and the number of successful retrievals. Neuro implements this directly:

```
weight(entry) = exp(-age_days / (half_life_days × ln(2)))
```

where `half_life_days = tier_multiplier × type_base_half_life`. Successful use of a knowledge entry strengthens it (confirmation boost of 1.5× to confidence), while unsuccessful use weakens it (tier demotion).

### Somatic Marker Hypothesis

Antonio Damasio (1994) proposed that emotions provide fast heuristic signals ("gut feelings") that guide decision-making before conscious deliberation. Neuro integrates with the Daimon subsystem to implement somatic markers: the 8-dimensional somatic landscape (a k-d tree over strategy space) stores emotional valence alongside knowledge entries, enabling fast affective filtering during retrieval.

### Hippocampal Replay

Mattar and Daw (2018) showed that the hippocampus preferentially replays memories that are most useful for future decisions, prioritized by: `Utility = Gain × Need × (1/spacing_penalty)`. Neuro's tier progression pipeline uses a similar prioritization scheme when selecting which episodes to distill into knowledge entries.

---

## Academic Foundations

- Ebbinghaus, H. (1885). *Über das Gedächtnis* (On Memory).
- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Why there are complementary learning systems in the hippocampus and neocortex." *Psychological Review*, 102(3), 419–457.
- Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction to Computing in Distributed Representation with High-Dimensional Random Vectors." *Cognitive Computation*, 1(2), 139–159.
- Kleyko, D., Rachkovskij, D. A., Osipov, E., & Rahimi, A. (2022). "A Survey on Hyperdimensional Computing: Theory, Architecture, and Applications." *ACM Computing Surveys*, 54(6).
- Damasio, A. R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam.
- Mattar, M. G., & Daw, N. D. (2018). "Prioritized memory access explains planning and hippocampal replay." *Nature Neuroscience*, 21, 1609–1617.
- Bower, G. H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148.

---

## Current Status and Gaps

### What Exists in the Codebase

| Component | Crate | Status |
|---|---|---|
| `KnowledgeEntry` struct | `roko-neuro/src/lib.rs` | Implemented with id, kind, source, content, confidence, confidence_weight, refuted_insight_id, refutation_evidence, source_episodes, tags, created_at, half_life_days, hdc_vector |
| `KnowledgeKind` enum | `roko-neuro/src/lib.rs` | 7 variants: Fact, Insight, Procedure, Heuristic, Playbook, Constraint, AntiKnowledge |
| `NeuroStore` trait | `roko-neuro/src/lib.rs` | init, query, ingest, decay, gc |
| `KnowledgeStore` (JSONL impl) | `roko-neuro/src/knowledge_store.rs` | Append-only, with confirmation records, stats, optional HDC index |
| `Distiller` | `roko-neuro/src/distiller.rs` | Episode → knowledge via LLM (Claude Haiku default) |
| `TierProgression` | `roko-neuro/src/tier_progression.rs` | 3-stage: episodes→insights, insights→heuristics, heuristics→PLAYBOOK.md |
| `ContextAssembler` | `roko-neuro/src/context.rs` | Skeleton (struct defined, no methods) |
| `HdcVector` (10,240-bit BSC) | `roko-primitives/src/hdc.rs` | Full: bind, bundle, permute, similarity, from_seed |
| `HdcFingerprint` (code symbols) | `roko-index/src/hdc.rs` | Trigram encoding, role vectors per SymbolKind |
| `KMedoids` clustering | `roko-learn/src/hdc_clustering.rs` | PAM algorithm over HdcVector |

### What's Missing

- **Warning, CausalLink, StrategyFragment knowledge types**: The `KnowledgeKind` enum in the current codebase has Fact/Insight/Procedure/Heuristic/Playbook/Constraint/AntiKnowledge — but not Warning, CausalLink, or StrategyFragment from the refactoring-prd design. The refactoring-prd's six types do not exactly match the current code's seven variants. Reconciliation is needed (see `12a-cognitive-layer.md` tasks D1-D4).
- **Four validation tiers (Transient/Working/Consolidated/Persistent)**: Not implemented. The current code has half-life constants but no tier multiplier system.
- **ContextAssembler methods**: Only the struct is defined; no context assembly logic exists.
- **VCG attention auction**: Designed but not implemented (Tier 2, P2).
- **Active inference context selection**: Designed but not implemented (Tier 2, P2).
- **Somatic landscape integration**: Designed in `09-innovations.md` §III; not yet wired into Neuro retrieval.
- **Backup/Restore CLI commands**: `roko neuro backup` / `roko neuro restore` not yet implemented.
- **Pheromone system**: Types are designed (`Threat`, `Opportunity`, `Wisdom`) but not implemented. Currently tracked as Tier 5E (P2).

---

## Cross-references

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for detailed coverage of all knowledge types
- See [02-four-validation-tiers.md](./02-four-validation-tiers.md) for the tier progression system
- See [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md) for HDC/VSA theory and implementation
- See [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for decay mechanics
- See [15-knowledge-backup-restore.md](./15-knowledge-backup-restore.md) for the backup/restore model
- See [16-current-status-and-gaps.md](./16-current-status-and-gaps.md) for detailed implementation status
- See topic [00-architecture](../00-architecture/INDEX.md) for the Synapse Architecture (Engrams, 6 traits)
- See topic [09-daimon](../09-daimon/INDEX.md) for the Daimon motivation/affect subsystem
- See topic [10-dreams](../10-dreams/INDEX.md) for the Dreams offline consolidation subsystem
