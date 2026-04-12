# Four-Tier Distillation Pipeline

> Episodes are distilled into Insights, Insights are promoted to Heuristics, and validated Heuristics are compiled into human-readable PLAYBOOK.md files — a three-stage pipeline implementing Complementary Learning Systems theory.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [01-six-knowledge-types.md](./01-six-knowledge-types.md), [02-four-validation-tiers.md](./02-four-validation-tiers.md)
**Key sources**:
- `crates/roko-neuro/src/tier_progression.rs` (TierProgression, InsightRecord, HeuristicRule)
- `crates/roko-neuro/src/distiller.rs` (DistillationBackend, Distiller)
- `crates/roko-neuro/src/episode_completion.rs` (spawn_episode_distillation)
- `refactoring-prd/03-cognitive-subsystems.md` §3 (Dreams produce knowledge promotions)

---

## Abstract

The distillation pipeline transforms raw agent experiences (episodes) into progressively more abstract and durable knowledge. It operates in three stages:

1. **D1: Episodes → Insights** — Pattern detection extracts observations from completed episodes
2. **D2: Insights → Heuristics** — Clusters of 5+ confirmed Insights with ≥0.7 confidence are promoted to Heuristics
3. **D3: Heuristics → PLAYBOOK.md** — Validated Heuristics are compiled into human-readable playbook files

This pipeline directly implements Complementary Learning Systems (CLS) theory (McClelland et al. 1995): fast episodic memory (episodes) consolidates into slow semantic memory (Insights, Heuristics, Playbooks) through repeated extraction and validation. Each stage increases abstraction, durability, and generalizability while decreasing specificity and recency.

---

## Stage D1: Episodes → Insights

### Episode Distillation

When an episode (a completed task run) finishes, the `spawn_episode_distillation` function triggers asynchronous extraction of knowledge candidates:

```rust
// From roko-neuro/src/episode_completion.rs (signature)
pub fn spawn_episode_distillation(
    episode: Episode,
    distiller: Arc<dyn DistillationBackend>,
    store: Arc<Mutex<KnowledgeStore>>,
) -> JoinHandle<Result<()>>;
```

The distillation runs on a background task to avoid blocking the agent's main execution loop.

### The DistillationBackend Trait

```rust
// From roko-neuro/src/distiller.rs
pub trait DistillationBackend: Send + Sync {
    /// Extract knowledge candidates from a completed episode.
    async fn distill(&self, episode: &Episode) -> Result<Vec<KnowledgeEntry>>;
}
```

The default implementation (`Distiller`) uses an LLM (Claude Haiku by default) to extract structured knowledge from episode transcripts. The LLM is prompted to identify:

- **Observations** that could become Insights
- **Patterns** that could become Heuristics
- **Dangers** that could become Warnings
- **Contradictions** to existing knowledge that could become AntiKnowledge

The LLM response is parsed as structured JSON and converted to `KnowledgeEntry` objects.

### InsightRecord

The tier progression system uses `InsightRecord` to track extracted insights:

```rust
// From roko-neuro/src/tier_progression.rs
pub struct InsightRecord {
    pub pattern: String,        // The observed pattern
    pub support: usize,         // Number of episodes supporting this pattern
    pub confidence: f64,        // Accumulated confidence
    pub source_episodes: Vec<String>,  // Episode IDs that contributed
}
```

**Minimum support**: An InsightRecord needs support from at least 3 episodes to be considered for D1 output. Single-episode observations are too noisy — they may reflect idiosyncratic circumstances rather than genuine patterns.

### D1 Output

Distilled knowledge entries enter the NeuroStore at **Transient tier** with initial confidence based on the extraction confidence (typically 0.3–0.6). They must be validated through use before promotion.

---

## Stage D2: Insights → Heuristics

### Pattern Mining

The D2 stage uses the `PatternMiner` from `roko-learn` to identify clusters of related Insights that share a common pattern. The mining process:

1. **Collect** all Insights with confidence ≥ 0.5
2. **Cluster** by HDC similarity (if vectors are available) or by tag overlap
3. **Filter** clusters with ≥ 5 members and mean confidence ≥ 0.7
4. **Extract** the common pattern from each qualifying cluster

### HeuristicRule

```rust
// From roko-neuro/src/tier_progression.rs
pub struct HeuristicRule {
    pub rule: String,           // The generalized rule
    pub support: usize,         // Number of Insights supporting this rule
    pub confidence: f64,        // Aggregated confidence
    pub source_insights: Vec<String>,  // Insight IDs that contributed
}
```

### Promotion Criteria

| Criterion | Threshold | Rationale |
|---|---|---|
| Minimum support | 5 Insights | Ensures the pattern is robust across multiple observations |
| Minimum confidence | 0.7 | Ensures high reliability before promoting to a durable type |
| Cross-validation | At least 2 distinct contexts | Prevents overfitting to a single scenario |
| No contradictions | No active AntiKnowledge refuting the pattern | Prevents promoting contested knowledge |

### replay_heuristics()

The tier progression system includes a `replay_heuristics()` method that re-evaluates existing Heuristics against new evidence:

```rust
// From roko-neuro/src/tier_progression.rs (signature)
pub fn replay_heuristics(&mut self) -> Result<Vec<HeuristicAdjustment>>;
```

This method:
1. Retrieves all current Heuristics
2. Checks each against recent episodes for confirmation or contradiction
3. Adjusts confidence up (confirmed) or down (contradicted)
4. Returns a list of adjustments for logging

Replay runs during the Dreams cycle (offline consolidation) to avoid impacting online task execution.

---

## Stage D3: Heuristics → PLAYBOOK.md

### Playbook Compilation

The D3 stage compiles validated Heuristics into human-readable PLAYBOOK.md files:

```rust
// From roko-neuro/src/tier_progression.rs
pub struct PlaybookCompilation {
    pub title: String,
    pub rules: Vec<HeuristicRule>,
    pub markdown: String,       // Rendered PLAYBOOK.md content
}
```

### TierProgression Orchestrator

The `TierProgression` struct orchestrates all three stages:

```rust
// From roko-neuro/src/tier_progression.rs
pub struct TierProgression {
    knowledge_store: Arc<Mutex<KnowledgeStore>>,
    pattern_miner: Arc<PatternMiner>,  // from roko-learn
}

impl TierProgression {
    /// D1: Analyze episodes and extract InsightRecords.
    pub fn analyze(&self, episodes: &[Episode]) -> Result<Vec<InsightRecord>>;

    /// D1 continued: Extract KnowledgeEntry Insights from InsightRecords.
    pub fn extract_insights(&self, records: Vec<InsightRecord>) -> Result<Vec<KnowledgeEntry>>;

    /// D2: Promote clusters of Insights to HeuristicRules.
    pub fn promote_heuristics(&self) -> Result<Vec<HeuristicRule>>;

    /// D3: Compile validated Heuristics into PLAYBOOK.md.
    pub fn compile_playbook(&self) -> Result<PlaybookCompilation>;

    /// Replay: Re-evaluate existing Heuristics against new evidence.
    pub fn replay_heuristics(&mut self) -> Result<Vec<HeuristicAdjustment>>;
}
```

### Playbook Format

Compiled playbooks are written as Markdown files at `.roko/neuro/PLAYBOOK.md`:

```markdown
# Agent Playbook

Generated: 2026-04-10T22:00:00Z
Rules: 12
Source insights: 47
Mean confidence: 0.82

## Rust Development Rules

### Rule 1: Always run clippy before committing
- **Confidence**: 0.91
- **Support**: 8 insights from 15 episodes
- **Evidence**: Gate pass rate improved 23% when clippy was run pre-commit

### Rule 2: Use tokio::time::pause() in async tests
- **Confidence**: 0.85
- **Support**: 5 insights from 9 episodes
- **Evidence**: Flaky test rate dropped from 12% to 1% with time mocking

## Error Handling Rules

### Rule 3: Prefer concrete error types over Box<dyn Error>
...
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
│  - min_support = 3 episodes             │
│  - Output: KnowledgeEntry (Insight)     │
│  - Initial tier: Transient              │
│  - Initial confidence: 0.3-0.6         │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  Validation through Use                  │
│  - Agent retrieves entry, uses it       │
│  - Gate checks outcome                  │
│  - Positive → tier promotion + boost    │
│  - Negative → tier demotion             │
│  - Entries accumulate confirmations     │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  D2: Insight → Heuristic Promotion      │
│  - Cluster related Insights             │
│  - min_support = 5 insights             │
│  - min_confidence = 0.7                 │
│  - Cross-validation required            │
│  - Output: KnowledgeEntry (Heuristic)   │
│  - Initial tier: Working or Consolidated│
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  D3: Heuristic → PLAYBOOK.md            │
│  - Compile validated Heuristics         │
│  - Render as human-readable Markdown    │
│  - Output: .roko/neuro/PLAYBOOK.md      │
│  - Serves as agent's operational manual │
└─────────────────────────────────────────┘
```

---

## Integration with Dreams

The distillation pipeline runs both online (after episode completion) and offline (during Dreams consolidation). The Dreams cycle (see topic [10-dreams](../10-dreams/INDEX.md)) drives the pipeline during idle time:

1. **NREM Replay**: Re-process recent episodes (prioritized by Mattar-Daw utility formula)
2. **Consolidation**: Run D1 and D2 on replayed episodes
3. **Pruning**: Run decay + GC to remove stale knowledge
4. **Playbook update**: Run D3 to recompile PLAYBOOK.md with new Heuristics

This mirrors the neuroscience of sleep consolidation: fast episodic learning during the day, slow semantic consolidation during sleep (McClelland et al. 1995).

---

## Academic Foundations

- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Why there are complementary learning systems in the hippocampus and neocortex." *Psychological Review*, 102(3), 419–457.
- Mattar, M. G., & Daw, N. D. (2018). "Prioritized memory access explains planning and hippocampal replay." *Nature Neuroscience*, 21, 1609–1617.
- Lacaux, C., et al. (2021). "Sleep onset is a creative sweet spot." *Science Advances*, 7(50). (Creative insight during N1 sleep)
- Wang, L., et al. (2024). "A Survey on Large Language Model Based Autonomous Agents." (Skill extraction patterns)

---

## Current Status and Gaps

**Implemented**:
- `DistillationBackend` trait and `Distiller` struct (LLM-based extraction)
- `spawn_episode_distillation()` for async extraction
- `TierProgression` struct with `analyze()`, `extract_insights()`, `promote_heuristics()`, `compile_playbook()`, `replay_heuristics()`
- `InsightRecord`, `HeuristicRule`, `PlaybookCompilation` types
- `PatternMiner` integration from `roko-learn`

**Missing**:
- HDC-based clustering in D2 (currently uses PatternMiner, which may use simpler methods)
- Cross-validation enforcement in D2 promotion criteria
- AntiKnowledge contradiction check before promotion
- Dreams integration (pipeline runs only on episode completion, not during idle consolidation)
- Automatic Warning extraction in D1

---

## Cross-references

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for the types produced at each stage
- See [02-four-validation-tiers.md](./02-four-validation-tiers.md) for how tiers are assigned during promotion
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for how ingested entries are stored and queried
- See topic [10-dreams](../10-dreams/INDEX.md) for offline consolidation during Dreams
- See topic [05-learning](../05-learning/INDEX.md) for the episode logging that feeds D1
