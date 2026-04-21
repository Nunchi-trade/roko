# Four-Tier Distillation Pipeline

> Episodes are distilled into Insights, Insights are promoted into Heuristics, and validated Heuristics are compiled into human-readable PLAYBOOK.md files. Durable knowledge keeps its receipts today; explicit falsifier records, worldview history, and demurrage-governed freshness are target-state extensions described in this doc.
>
> **Implementation status**
> - **Shipping**: episode distillation, `InsightRecord`, `HeuristicRule`, and playbook compilation in `roko-neuro`.
> - **Target-state**: richer typed heuristics, explicit falsifier records, and worldview clustering.
> - **Deferred**: demurrage/balance freshness as the governing model, worldview cold-tier preservation, and HDC novelty-weighted reinforcement.

**Topic**: [Neuro - Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [01-six-knowledge-types.md](./01-six-knowledge-types.md), [02-four-validation-tiers.md](./02-four-validation-tiers.md)
**Key sources**:
- `crates/roko-neuro/src/tier_progression.rs` (TierProgression, InsightRecord, HeuristicRule)
- `crates/roko-neuro/src/distiller.rs` (DistillationBackend, Distiller)
- `crates/roko-neuro/src/episode_completion.rs` (spawn_episode_distillation)
- `docs/00-architecture/04-decay-variants.md` (retention model and Ebbinghaus shaping)
- `docs/00-architecture/18-decay-tier-matrix.md` (tier matrix and graduation rules)
- `docs/00-architecture/01-naming-and-glossary.md` (canonical Neuro vocabulary)
- `../../tmp/refinements/11-hyperdimensional-substrate.md` (HDC fingerprint proposal)
- `../../tmp/refinements/12-knowledge-demurrage.md` (demurrage refinement)
- `../../tmp/refinements/14-worldview-validation.md` (heuristic calibration, falsifiers, worldview clustering)

---

## Abstract

The distillation pipeline turns raw agent experience into durable, inspectable knowledge. In the expanded target-state design, `Heuristic` is a first-class durable knowledge kind rather than a derived side effect. A heuristic stores a claim, preconditions, a prediction, calibration history, episode receipts, and lineage. Episodes feed evidence into that record, and later episodes either confirm it, violate it, refine it, or refute it.

This chapter follows the refinement in `tmp/refinements/14-worldview-validation.md` and keeps the earlier HDC and demurrage changes in view. The shipping pipeline is narrower than that full design: it already promotes useful patterns and keeps receipts, while the richer falsifier, worldview, and demurrage layers remain target-state. The point is still to preserve what worked, preserve what was challenged, and make the difference explicit.

The pipeline still has three major stages:

1. **D1: Episodes -> Insights** - extract observations, warnings, and receipts from completed episodes
2. **D2: Insights -> Heuristics** - promote repeated patterns into durable heuristics with calibration and contradiction handling
3. **D3: Heuristics -> PLAYBOOK.md** - compile validated heuristics into a human-readable playbook without deleting the underlying durable records

This is CLS-style consolidation with more structure than a plain semantic memo. HDC fingerprints can keep the promotion path structural rather than textual where they are available. The deferred demurrage model would make durable knowledge earn its place through retrieval, citation, surprise, and gate survival. Worldviews are likewise a higher-order target-state cluster over heuristics rather than a current shipping object.

See also [tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md), [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md), [tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md), [HDC Knowledge Encoding](./06-hdc-knowledge-encoding.md), [Temporal Knowledge Topology](../00-architecture/27-temporal-knowledge-topology.md), [04-decay-variants.md](../00-architecture/04-decay-variants.md), [18-decay-tier-matrix.md](../00-architecture/18-decay-tier-matrix.md), and [Naming and Glossary](../00-architecture/01-naming-and-glossary.md).

---

## Stage D1: Episodes -> Insights

### Episode distillation

When an episode finishes, `spawn_episode_distillation()` triggers asynchronous extraction of knowledge candidates:

```rust
// From roko-neuro/src/episode_completion.rs (signature)
pub fn spawn_episode_distillation(
    episode: Episode,
    distiller: Arc<dyn DistillationBackend>,
    store: Arc<Mutex<KnowledgeStore>>,
) -> JoinHandle<Result<()>>;
```

The distillation runs in the background so the agent's main execution loop stays responsive. D1 extracts the raw evidence that later becomes Insight receipts, heuristic receipts, or falsifier records.

### DistillationBackend

```rust
// From roko-neuro/src/distiller.rs
pub trait DistillationBackend: Send + Sync {
    async fn distill(&self, episode: &Episode) -> Result<Vec<KnowledgeEntry>>;
}
```

The default `Distiller` uses an LLM to extract structured knowledge from episode transcripts. The prompt asks for:

- observations that could become Insights
- patterns that could become Heuristics or heuristic precursors
- dangers that could become Warnings
- contradictions to existing knowledge that could become AntiKnowledge or falsifiers

The LLM response is parsed as structured JSON and converted into `KnowledgeEntry` objects.

### InsightRecord

```rust
// From roko-neuro/src/tier_progression.rs
pub struct InsightRecord {
    pub pattern: String,
    pub support: usize,
    pub confidence: f64,
    pub source_episodes: Vec<String>,
}
```

An InsightRecord needs support from at least 3 episodes before it is considered stable enough for D1 output. Single-episode observations are often noise. The source episode list is important: it is the beginning of the receipt trail that later explains why a heuristic was ever trusted.

### D1 output

Distilled entries enter the NeuroStore at the Transient tier with initial confidence based on extraction confidence, typically 0.3-0.6. They also receive their HDC fingerprint at ingestion time and start carrying receipts back to the originating episodes so later calibration can point at concrete evidence instead of vague summary prose.

---

## Stage D2: Insights -> Heuristics

### Pattern mining

The D2 stage uses `PatternMiner` from `roko-learn` to identify clusters of related Insights that share a common pattern. The mining process is intentionally structural:

1. Collect all Insights with confidence >= 0.5
2. Cluster by HDC fingerprint similarity using the per-Engram fingerprint field
3. Filter clusters with at least 5 members and mean confidence >= 0.7
4. Extract the common pattern from each qualifying cluster
5. Emit a durable Heuristic record with calibration metadata and receipts

The HDC fingerprint matters twice in the fuller design. First, it keeps D2 structural instead of lexical. Second, it can provide a novelty signal for a future balance-aware reinforcement model. That is also why a heuristic must keep receipts: the system should be able to point back to the concrete episode trail that made the heuristic worth keeping.

### HeuristicRule

```rust
// From roko-neuro/src/tier_progression.rs
pub struct HeuristicRule {
    pub rule: String,
    pub support: usize,
    pub confidence: f64,
    pub source_insights: Vec<String>,
}
```

`HeuristicRule` is the outward-facing summary. The durable knowledge object behind it is the `Heuristic` entry itself: claim, preconditions, prediction, calibration, lineage, and receipts. The distinction matters because playbooks should be views over durable heuristics, not the only place the rule survives.

### Promotion criteria

| Criterion | Threshold | Rationale |
|---|---|---|
| Minimum support | 5 Insights | Ensures the pattern is robust across multiple observations |
| Minimum confidence | 0.7 | Ensures high reliability before promoting to a durable type |
| Cross-validation | At least 2 distinct contexts | Prevents overfitting to a single scenario |
| No unresolved contradictions | No active falsifier or AntiKnowledge entry refuting the pattern | Prevents promoting contested knowledge |

### Heuristic calibration

Once a heuristic is promoted, it does not become static. Every later episode can reinforce, violate, or refine it:

- Confirm - the heuristic predicted the right outcome in context
- Violate - the heuristic predicted the wrong outcome and should lose weight
- Refine - the heuristic was directionally right but too broad, so a narrower child heuristic should be spawned
- Generalize - the heuristic worked in a broader context than expected, so the claim can be lifted upward
- Refute - repeated contradictions should create an explicit falsifier record and cold-tier the contested version

Calibration state stays attached to the heuristic as a first-class record. The important fields are trials, confirmations, violations, the confidence interval, and the episode receipts that justify each update. That is what lets users browse current belief state and see which heuristics are battle-tested versus still provisional.

### replay_heuristics()

```rust
// From roko-neuro/src/tier_progression.rs (signature)
pub fn replay_heuristics(&mut self) -> Result<Vec<HeuristicAdjustment>>;
```

Replay re-evaluates existing heuristics against new evidence. It:

1. Retrieves all current heuristics
2. Checks each against recent episodes for confirmation or contradiction
3. Adjusts confidence up or down
4. Returns adjustments for logging, including any child heuristics or falsifier records that should be created

Replay runs during the Dreams cycle so it does not interfere with online task execution.

---

## Worldviews and cold tier preservation

Target-state: a worldview is a cluster of heuristics that repeatedly co-occur in successful episodes. The cluster is not manually declared; it is observed from the co-citation graph and from repeated episode overlap.

Worldviews matter because they let Neuro reason above a single heuristic:

- A worldview bundles heuristics that tend to travel together
- A worldview has a domain fingerprint that says where it applies
- A worldview has a coherence score and an effectiveness score
- A worldview can be preserved in a cold tier when it stops being active but still matters as a fallback

Cold-tier preservation is important for two reasons. First, it prevents good but currently quiet worldviews from being forgotten just because another domain is active today. Second, it lets the router thaw a preserved worldview when the current task fingerprint matches again. That keeps the knowledge store plural by construction instead of collapsing into monoculture.

The cold-tier rule in this design follows demurrage. That storage model is deferred, but the intent is clear: inactive worldviews would lose retrieval priority, not identity. Their receipts, calibration history, and lineage would remain valid even while they cool.

---

## Stage D3: Heuristics -> PLAYBOOK.md

### Playbook compilation

The D3 stage compiles validated heuristics into human-readable PLAYBOOK.md files:

```rust
// From roko-neuro/src/tier_progression.rs
pub struct PlaybookCompilation {
    pub title: String,
    pub rules: Vec<HeuristicRule>,
    pub markdown: String,
}
```

Compiled playbooks are not exempt from retention pressure, but the current system uses existing decay/confidence behavior rather than the full demurrage economy described here. In the deferred design, each rule would keep earning balance through retrieval, citation, surprise, or gate survival. The heuristic record itself would remain in Neuro even if the playbook projection gets rewritten.

### Freshness and balance

Neuro treats playbook compilation as the beginning of a freshness contract, not the end of one:

- Rules that are retrieved often keep their balance higher
- Rules that are cited by other knowledge entries gain reinforcement faster
- Rules that survive gates during real work keep the strongest balance
- Rules that explain novel surprises earn the largest novelty-weighted boost
- Rules that are never used eventually cool and stop dominating the playbook

Heuristics themselves follow the same contract, but they carry more structure than a playbook line. Their receipts and calibration history let the store preserve a precise record of why the heuristic existed, what episodes supported it, and what later falsified it.

### TierProgression orchestrator

```rust
// From roko-neuro/src/tier_progression.rs
pub struct TierProgression {
    knowledge_store: Arc<Mutex<KnowledgeStore>>,
    pattern_miner: Arc<PatternMiner>,
}

impl TierProgression {
    pub fn analyze(&self, episodes: &[Episode]) -> Result<Vec<InsightRecord>>;
    pub fn extract_insights(&self, records: Vec<InsightRecord>) -> Result<Vec<KnowledgeEntry>>;
    pub fn promote_heuristics(&self) -> Result<Vec<HeuristicRule>>;
    pub fn compile_playbook(&self) -> Result<PlaybookCompilation>;
    pub fn replay_heuristics(&mut self) -> Result<Vec<HeuristicAdjustment>>;
}
```

The orchestrator keeps the pipeline cohesive, but the durable record is the heuristic itself. The playbook is the projection; the heuristic is the source of truth.

### Playbook format

Compiled playbooks are written as Markdown files at `.roko/neuro/PLAYBOOK.md`. A playbook is the user-facing surface, but the backing store remains heuristic-rich:

```markdown
# Agent Playbook

Generated: 2026-04-10T22:00:00Z
Rules: 12
Source insights: 47
Mean confidence: 0.82
```

---

## Pipeline Flow Diagram

```
Episodes (raw agent turns)
    │
    ▼
┌─────────────────────────────────────────┐
│  D1: Episode Distillation               │
│  - LLM extracts observations            │
│  - records receipts and contradiction traces
│  - min_support = 3 episodes             │
│  - Output: KnowledgeEntry (Insight)     │
│  - Initial tier: Transient              │
│  - Initial confidence: 0.3-0.6         │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  D2: Heuristic Calibration              │
│  - HDC clusters repeated Insight sets   │
│  - promote to durable Heuristic records  │
│  - attach trials / confirmations / receipts
│  - contradictions become falsifier records
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  Worldview Clustering                   │
│  - co-citation graph over heuristics     │
│  - preserve hot and cold worldview sets  │
│  - thaw when domain fingerprints match   │
│  - keep receipts and calibration intact  │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  D3: Heuristic -> PLAYBOOK.md           │
│  - Compile validated Heuristics         │
│  - Render as human-readable Markdown    │
│  - Output: .roko/neuro/PLAYBOOK.md      │
│  - Serves as agent's operational manual │
└─────────────────────────────────────────┘
```

---

## Integration with Dreams

The distillation pipeline runs both online after episode completion and offline during Dreams consolidation. The Dreams cycle (see topic [10-dreams](../10-dreams/INDEX.md)) drives the pipeline during idle time:

1. NREM replay re-processes recent episodes, prioritized by the Mattar-Daw utility formula
2. Consolidation runs D1 and D2 on replayed episodes, with D2 grouping by fingerprint similarity and updating heuristic calibration
3. Target-state pruning charges demurrage, freezes cold knowledge, preserves cold-tier worldviews, and thaws only what is still earning balance
4. Playbook update runs D3 to recompile PLAYBOOK.md with new heuristics while keeping the underlying durable records intact

This mirrors sleep consolidation: fast episodic learning during the day, slow semantic consolidation during sleep (McClelland et al. 1995). In Neuro terms, the deferred balance model would also re-check which entries are still earning their place and which should move to colder storage.

---

## Cross-References

- See [tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md) for the HDC fingerprint proposal that drives D2 clustering
- See [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md) for the demurrage model reflected in this doc
- See [tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md) for the heuristic calibration, falsifier, and worldview-clustering refinement reflected here
- See [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md) for the HDC algebra behind fingerprint similarity
- See [06-hdc-knowledge-encoding.md](./06-hdc-knowledge-encoding.md) for the default encoder and per-Engram fingerprinting pipeline
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for the native similarity query surface
- See [04-decay-variants.md](../00-architecture/04-decay-variants.md) for the architecture-side retention model
- See [18-decay-tier-matrix.md](../00-architecture/18-decay-tier-matrix.md) for tier progression and cold-tier calibration
- See [Naming and Glossary](../00-architecture/01-naming-and-glossary.md) for canonical Neuro terminology, including `Heuristic`, `Pulse`, `Bus`, and `Neuro`

---

## Academic Foundations

- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Why there are complementary learning systems in the hippocampus and neocortex." *Psychological Review*, 102(3), 419-457.
- Mattar, M. G., & Daw, N. D. (2018). "Prioritized memory access explains planning and hippocampal replay." *Nature Neuroscience*, 21, 1609-1617.
- Lacaux, C., et al. (2021). "Sleep onset is a creative sweet spot." *Science Advances*, 7(50). (Creative insight during N1 sleep)
- Wang, L., et al. (2024). "A Survey on Large Language Model Based Autonomous Agents." (Skill extraction patterns)
- Festinger, L. (1957). *A Theory of Cognitive Dissonance*. Stanford University Press.
