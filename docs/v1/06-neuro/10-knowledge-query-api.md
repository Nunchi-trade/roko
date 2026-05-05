# Knowledge Query API

> The NeuroStore trait defines the storage and retrieval interface for Neuro's persistent knowledge — init, query, ingest, decay, and gc — with the KnowledgeStore JSONL implementation as the primary backend.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [01-six-knowledge-types.md](./01-six-knowledge-types.md), [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md)
**Key sources**:
- `crates/roko-neuro/src/lib.rs` (NeuroStore trait, KnowledgeEntry)
- `crates/roko-neuro/src/knowledge_store.rs` (KnowledgeStore JSONL implementation)
- `crates/roko-neuro/src/context.rs` (ContextAssembler skeleton)
- `docs/00-architecture/02-engram-data-type.md` (first-class fingerprint field)
- `docs/00-architecture/07-substrate-trait.md` (native similarity query surface)
- `../../tmp/refinements/11-hyperdimensional-substrate.md` (canonical refinement source)

---

## Abstract

The `NeuroStore` trait is the single entry point for all durable knowledge storage operations in Neuro. It defines six methods - `init`, `query`, `query_similar`, `ingest`, `decay`, and `gc` - that together provide a complete lifecycle for knowledge entries. The trait is implemented by `KnowledgeStore`, which uses append-only JSONL files at `.roko/neuro/knowledge.jsonl` as the storage backend.

The query API now treats HDC fingerprints as first-class record data. Topic-based retrieval remains available for lexical lookup, but native similarity queries operate directly on the stored Engram fingerprint and return nearest neighbors by Hamming distance. The API is designed to be called from the context assembly pipeline, where retrieved knowledge entries are injected into the agent's prompt alongside episode memory, playbook rules, and task context.

---

## The NeuroStore Trait

```rust
// From roko-neuro/src/lib.rs
/// Single entry point for durable knowledge storage backends.
pub trait NeuroStore: Sized {
    /// Initialize a store at the given path.
    fn init(path: &Path) -> Result<Self>;

    /// Query a topic for relevant knowledge entries.
    fn query(&self, topic: &str, limit: usize) -> Result<Vec<KnowledgeEntry>>;

    /// Query by HDC fingerprint for the nearest durable records.
    fn query_similar(
        &self,
        fingerprint: &HdcVector,
        limit: usize,
    ) -> Result<Vec<(KnowledgeEntry, f32)>>;

    /// Ingest a batch of knowledge entries.
    fn ingest(&mut self, entries: Vec<KnowledgeEntry>) -> Result<()>;

    /// Apply decay and return the number of entries processed.
    fn decay(&mut self) -> Result<usize>;

    /// Garbage-collect low-confidence entries and return the number removed.
    fn gc(&mut self, min_confidence: f64) -> Result<usize>;
}
```

### Method Details

**`init(path: &Path)`**: Creates or opens a NeuroStore at the given filesystem path. For `KnowledgeStore`, this opens or creates the JSONL file at `{path}/knowledge.jsonl` and loads all entries into memory. The store is both an in-memory index and an on-disk log.

**`query(topic: &str, limit: usize)`**: Returns up to `limit` knowledge entries relevant to the given topic string. Relevance is determined by:
1. Tag matching — entries whose tags contain words from the topic
2. Content matching — entries whose content matches the topic (substring or keyword)
3. HDC similarity — entries whose stored fingerprint has high Hamming similarity to the topic's HDC fingerprint

Results are sorted by a composite retrieval score. The current implementation uses simple keyword matching; the refactoring-prd design specifies a more sophisticated scoring function:

```
retrieval_score = confidence × decay_weight × (1 + hdc_similarity_bonus)
```

**`query_similar(fingerprint: &HdcVector, limit: usize)`**: Returns up to `limit` durable records ranked by raw similarity to the supplied fingerprint. This is the native similarity path for consensus, analogy, and structurally matched retrieval. The similarity index is an acceleration layer, not the source of truth.

**`ingest(entries: Vec<KnowledgeEntry>)`**: Appends new entries to the store. Each entry is:
1. Assigned a unique ID (if not already set)
2. Timestamped with `created_at` (if not already set)
3. Given a default `half_life_days` based on its `KnowledgeKind` (if not explicitly set)
4. Appended to the JSONL file
5. Added to the in-memory index

Ingestion is append-only — existing entries are never modified by ingestion. To update an entry's confidence or tier, use the confirmation mechanism.

**`decay(&mut self)`**: Applies time-based exponential decay to all entries. For each entry:
```
new_confidence = confidence × 2^(-elapsed_days / half_life_days)
```
Returns the number of entries processed. Decay runs periodically, triggered by the orchestrator or by a manual command.

**`gc(&mut self, min_confidence: f64)`**: Removes entries whose confidence has fallen below `min_confidence`. The default threshold is `DEFAULT_GC_MIN_CONFIDENCE = 0.05`. AntiKnowledge entries with confidence ≥ 0.3 are exempt (confidence floor). Returns the number of entries removed.

---

## KnowledgeStore Implementation

### Storage Format

Knowledge entries are stored as append-only JSONL (one JSON object per line) at `.roko/neuro/knowledge.jsonl`:

```jsonl
{"id":"ke_001","kind":"insight","content":"Rust borrow checker errors often mean you need Arc","confidence":0.8,"tags":["rust","borrow-checker"],"created_at":"2026-04-01T10:00:00Z","half_life_days":30.0,"fingerprint":"<10,240-bit HDC vector>"}
{"id":"ke_002","kind":"heuristic","content":"Always run clippy before committing","confidence":0.9,"tags":["rust","workflow"],"created_at":"2026-04-02T14:30:00Z","half_life_days":90.0,"fingerprint":"<10,240-bit HDC vector>"}
```

### Key Constants

```rust
// From roko-neuro/src/knowledge_store.rs
pub const DEFAULT_GC_MIN_CONFIDENCE: f64 = 0.05;
pub const CONFIRMATION_BOOST: f64 = 1.5;
```

### KnowledgeStats

The store tracks aggregate statistics:

```rust
pub struct KnowledgeStats {
    pub total_entries: usize,
    pub entries_by_kind: HashMap<KnowledgeKind, usize>,
    pub mean_confidence: f64,
    pub entries_above_threshold: usize,  // entries with confidence > GC threshold
}
```

### KnowledgeConfirmationRecord

When an entry is used and the outcome is known, a confirmation record is created:

```rust
pub struct KnowledgeConfirmationRecord {
    pub entry_id: String,
    pub confirmed: bool,       // true = positive outcome, false = negative
    pub episode_id: String,    // which episode triggered the confirmation
    pub timestamp: DateTime<Utc>,
}
```

Confirmations feed the tier progression system (see [02-four-validation-tiers.md](./02-four-validation-tiers.md)): positive confirmations count toward tier promotion; negative confirmations trigger tier demotion.

### Native HDC MemoryIndex

`KnowledgeStore` includes an HDC-based memory index derived from the stored fingerprints:

```rust
#[cfg(feature = "hdc")]
pub struct MemoryIndex {
    vectors: Vec<HdcVector>,      // one per entry
    entry_ids: Vec<String>,       // corresponding entry IDs
}

#[cfg(feature = "hdc")]
pub struct MemoryHit {
    pub entry_id: String,
    pub similarity: f32,
}
```

The `MemoryIndex` provides HDC-based similarity search alongside the standard keyword matching. When both are available, the query API combines their results using the composite retrieval score, but the fingerprint on the record remains the source of truth.

---

## ContextAssembler

The `ContextAssembler` sits above the NeuroStore and assembles context from both knowledge and episode memory under a token budget:

```rust
// From roko-neuro/src/context.rs
pub struct ContextAssembler {
    knowledge_store: Arc<KnowledgeStore>,
    episode_store: Arc<EpisodeStore>,  // EpisodeLogger from roko-learn
    max_context_tokens: usize,
}
```

The current implementation already performs the base retrieval pipeline:

1. **Query** the knowledge store for entries relevant to the current task
2. **Query** the episode store for recent relevant episodes
3. **Rank** all results by composite retrieval score, including fingerprint similarity
4. **Budget** — run an auction-style allocator that weighs retrieval value against token cost, dampens repeated source families, and stops when marginal gain falls below the running average
5. **Format** — render selected entries as structured text for the LLM prompt

The canonical implementation now lives in `roko-neuro/src/context.rs`, and
`roko-compose` re-exports those primitives rather than maintaining a parallel
copy. PAD-based affect biasing is also wired into ranking via `PadState`, and
the allocator now enforces a small contrarian slice for affect-heavy retrieval
so one mood does not monopolize the recalled knowledge set.

The context assembly pipeline now feeds a shared bidder-aware auction in `roko-compose::PromptComposer`, where different subsystems (Neuro, Daimon, iteration memory, code intelligence, playbooks, research, task context, and oracles) can bid for token budget via `PromptSection.bidder`. Neuro's bid is still based on the expected value of including knowledge context for the current task, but the remaining gap is a more literal VCG settlement / price model and fuller bidder coverage.

---

## Integration Points

### With the Orchestrator

The orchestrator (`roko-cli/src/orchestrate.rs`) calls Neuro at several points:

1. **Before task execution**: Query knowledge store for relevant context → inject into agent prompt
2. **After task execution**: If gate passes → create positive confirmation record
3. **After task execution**: If gate fails → create negative confirmation record
4. **After episode completion**: Trigger distillation → ingest new knowledge entries with fresh fingerprints
5. **Periodically**: Run decay + gc to maintain knowledge store health

### With the Distiller

The distiller (`roko-neuro/src/distiller.rs`) extracts knowledge entries from completed episodes:

```rust
pub trait DistillationBackend: Send + Sync {
    async fn distill(&self, episode: &Episode) -> Result<Vec<KnowledgeEntry>>;
}
```

Distilled entries are ingested into the NeuroStore at Transient tier.

### With the Tier Progression Pipeline

The tier progression system (`roko-neuro/src/tier_progression.rs`) operates on the NeuroStore:

1. **D1**: Analyze episodes → extract InsightRecords
2. **D2**: Cluster Insights by HDC fingerprint similarity → promote to HeuristicRules (min_support=5, min_confidence=0.7)
3. **D3**: Compile Heuristics → generate PLAYBOOK.md

See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for details.

---

## Academic Foundations

- McClelland, J. L., et al. (1995). "Complementary learning systems." *Psychological Review*, 102(3). (Dual-store architecture)
- Park, J. S., et al. (2023). "Generative Agents: Interactive Simulacra of Human Behavior." *UIST 2023*. (Memory retrieval scoring in agent systems)
- Ebbinghaus, H. (1885). *Über das Gedächtnis*. (Decay-based retrieval weighting)

---

## Current Status and Gaps

**Implemented**: `NeuroStore` trait, `KnowledgeStore` JSONL backend, `KnowledgeConfirmationRecord`, `KnowledgeStats`, HDC fingerprints on durable records, `MemoryIndex`, and the canonical `ContextAssembler` retrieval/ranking/compression pipeline, including cost-aware auction-style chunk allocation inside Neuro.

**Missing**: Exact welfare-maximizing VCG settlement and fuller bidder coverage. Richer combined retrieval scoring. Tier-aware query filtering. Within-domain vs. cross-domain threshold selection. Full somatic-state retrieval and resonance-driven selection.

---

## Cross-References

- See [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for how decay affects retrieval scoring
- See [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md) for HDC-based similarity search
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for the distillation → ingestion pipeline
- See topic [03-composition](../03-composition/INDEX.md) for context engineering and prompt assembly
- See [tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md) for the full HDC fingerprint proposal
