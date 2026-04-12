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

## Implementation Details: D1 Warning Extraction

### Automatic Warning extraction algorithm

Warnings flag dangerous patterns that the agent should avoid. The D1 stage extracts Warnings from structural analysis of episode data, without requiring an LLM call.

```rust
/// Categories of automatically extracted Warnings.
pub enum WarningCategory {
    /// Gate failure pattern: repeated failures in the same area.
    GateFailure,
    /// Performance regression: slower execution over time.
    PerformanceRegression,
    /// Error pattern: same error recurring across episodes.
    RecurringError,
    /// Timeout pattern: tasks consistently hitting time limits.
    TimeoutPattern,
}

/// Extract Warnings from a completed episode.
///
/// Scans the episode transcript for failure patterns and generates
/// Warning-type KnowledgeEntry candidates.
pub fn extract_warnings(episode: &Episode) -> Vec<KnowledgeEntry> {
    let mut warnings = Vec::new();

    // Pattern 1: Gate failures
    for gate_result in &episode.gate_results {
        if !gate_result.passed {
            warnings.push(KnowledgeEntry {
                id: format!("warn_{}", uuid::Uuid::new_v4()),
                kind: KnowledgeKind::Warning,
                content: format!(
                    "Gate '{}' failed during task '{}': {}",
                    gate_result.gate_name,
                    episode.task_id,
                    gate_result.failure_reason,
                ),
                confidence: 0.4,
                source_episodes: vec![episode.id.clone()],
                tags: vec![
                    format!("gate:{}", gate_result.gate_name),
                    format!("task:{}", episode.task_id),
                ],
                ..Default::default()
            });
        }
    }

    // Pattern 2: Excessive retries (>2 indicates a problem)
    let retry_count = episode.turns.iter()
        .filter(|t| t.is_retry)
        .count();
    if retry_count > 2 {
        warnings.push(KnowledgeEntry {
            id: format!("warn_{}", uuid::Uuid::new_v4()),
            kind: KnowledgeKind::Warning,
            content: format!(
                "Task '{}' required {} retries. The approach may be unreliable.",
                episode.task_id, retry_count,
            ),
            confidence: 0.3 + (retry_count as f64 * 0.05).min(0.3),
            source_episodes: vec![episode.id.clone()],
            tags: vec!["retries".to_string(), format!("task:{}", episode.task_id)],
            ..Default::default()
        });
    }

    warnings
}
```

**Integration**: Call `extract_warnings()` inside `spawn_episode_distillation()`, alongside the LLM-based insight extraction. Warning extraction is deterministic and does not require an LLM call.

### D2: HDC-based clustering algorithm

The current D2 stage uses `PatternMiner`. HDC-based clustering provides a structural alternative that groups entries by role-filler similarity rather than text or tag overlap.

```rust
/// Cluster Insights by HDC vector similarity using k-medoids (PAM).
pub struct HdcClusterer {
    /// Minimum cluster size for Heuristic promotion. Default: 5.
    pub min_cluster_size: usize,
    /// Similarity threshold for cluster membership. Default: 0.53.
    pub membership_threshold: f32,
    /// Maximum number of clusters to produce. Default: 50.
    pub max_clusters: usize,
}

impl Default for HdcClusterer {
    fn default() -> Self {
        Self {
            min_cluster_size: 5,
            membership_threshold: 0.53,
            max_clusters: 50,
        }
    }
}

/// A cluster of related Insights.
pub struct InsightCluster {
    /// Centroid vector (bundle of all members).
    pub centroid: HdcVector,
    /// Member entry IDs.
    pub members: Vec<String>,
    /// Mean pairwise similarity within the cluster.
    pub cohesion: f32,
    /// Mean confidence of members.
    pub mean_confidence: f64,
}
```

**Algorithm**: k-medoids (PAM) over Hamming distance, with k estimated as `sqrt(n/2)` capped at `max_clusters`. Delegates to `roko-learn::hdc_clustering::k_medoids_pam()` which already operates on `HdcVector`.

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `min_cluster_size` | 5 | 3 - 10 | Smaller = more sensitive, more false promotions |
| `membership_threshold` | 0.53 | 0.51 - 0.56 | Must be above noise floor (0.50) |
| `max_clusters` | 50 | 10 - 200 | Caps compute cost |

### Promotion criteria: cross-validation and AntiKnowledge check

Two gates prevent premature promotion.

**Cross-validation enforcement**:

```rust
/// Check that a cluster's members come from at least min_contexts distinct contexts.
///
/// A "context" = episode task type + domain. Prevents overfitting to one scenario.
pub fn cross_validation_check(
    cluster: &InsightCluster,
    entries: &[KnowledgeEntry],
    min_contexts: usize, // Default: 2
) -> bool {
    let contexts: HashSet<String> = cluster.members.iter()
        .filter_map(|id| {
            let entry = entries.iter().find(|e| e.id == *id)?;
            entry.tags.iter()
                .find(|t| t.starts_with("context:"))
                .cloned()
        })
        .collect();

    contexts.len() >= min_contexts
}
```

**AntiKnowledge contradiction check**:

```rust
/// Check that no active AntiKnowledge entry contradicts this cluster's pattern.
pub fn anti_knowledge_check(
    cluster: &InsightCluster,
    anti_entries: &[KnowledgeEntry],
    threshold: f32, // Default: 0.526
) -> bool {
    for anti in anti_entries {
        if anti.kind != KnowledgeKind::AntiKnowledge {
            continue;
        }
        if let Some(anti_hv) = anti.hdc_vector.as_ref()
            .and_then(|b| HdcVector::from_bytes(b)) {
            if cluster.centroid.similarity(&anti_hv) > threshold {
                return false; // Blocked
            }
        }
    }
    true
}
```

**Full promotion gate**: Requires (1) >= 5 members, (2) mean confidence >= 0.7, (3) >= 2 distinct contexts, (4) no AntiKnowledge contradictions. All four must pass.

### Dreams integration: distillation during idle time

```rust
/// Trigger mechanism for Dreams-driven distillation.
pub struct DreamsDistillationTrigger {
    /// Minimum idle time before Dreams activates. Default: 5 minutes.
    pub idle_threshold: Duration,
    /// Maximum episodes to replay per session. Default: 20.
    pub max_replay_episodes: usize,
    /// Whether to run D2 during Dreams. Default: true.
    pub run_d2: bool,
    /// Whether to run D3 during Dreams. Default: true.
    pub run_d3: bool,
}

impl DreamsDistillationTrigger {
    /// Execute a Dreams distillation session.
    ///
    /// Pipeline:
    ///   1. Select episodes for replay (Mattar-Daw priority)
    ///   2. Run D1 on each (extract Insights + Warnings)
    ///   3. Run D2 (cluster Insights, promote Heuristics)
    ///   4. Run decay + GC
    ///   5. Run D3 (recompile PLAYBOOK.md)
    pub async fn run(
        &self,
        tier_progression: &mut TierProgression,
        episode_store: &EpisodeStore,
    ) -> Result<DreamsReport> {
        let episodes = episode_store.select_for_replay(self.max_replay_episodes)?;

        let mut all_insights = Vec::new();
        let mut all_warnings = Vec::new();
        for episode in &episodes {
            let insights = tier_progression.analyze(&[episode.clone()])?;
            all_insights.extend(tier_progression.extract_insights(insights)?);
            all_warnings.extend(extract_warnings(episode));
        }

        let new_heuristics = if self.run_d2 {
            tier_progression.promote_heuristics()?
        } else {
            vec![]
        };

        let playbook = if self.run_d3 {
            Some(tier_progression.compile_playbook()?)
        } else {
            None
        };

        Ok(DreamsReport {
            episodes_replayed: episodes.len(),
            insights_extracted: all_insights.len(),
            warnings_extracted: all_warnings.len(),
            heuristics_promoted: new_heuristics.len(),
            playbook_updated: playbook.is_some(),
        })
    }
}
```

**Trigger mechanism**: When no tasks have been dispatched for `idle_threshold` (default 5 minutes), Dreams activates and runs the distillation pipeline. Maximum duration: 30 seconds, to avoid blocking if a new task arrives.

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `idle_threshold` | 5 min | 1 - 30 min | Lower = more frequent Dreams |
| `max_replay_episodes` | 20 | 5 - 100 | More = deeper consolidation |
| Dreams max duration | 30 sec | 5 - 120 sec | Hard cap |

### End-to-end test scenario

**Setup**: 10 completed episodes involving Rust async code. 7 contain a gate failure related to `tokio::time` in tests. 3 succeed when using `tokio::time::pause()`.

**D1 output** (expected):
- 7 Warning entries (one per gate failure)
- 2+ Insight entries: "tokio::time causes flaky tests" and "tokio::time::pause() stabilizes tests"

**D2 output** (expected):
- 1 cluster of 7+ Insights about tokio::time
- Initial mean confidence ~0.47 (below 0.7, not yet promoted)
- After 3 more confirming episodes push confidence to >= 0.7: Heuristic promoted
- Cross-validation passes (contexts: "async-handler", "stream-processor", "rate-limiter")
- AntiKnowledge check passes

**D3 output** (expected):
```markdown
# Agent Playbook

## Rust Async Testing Rules

### Rule 1: Use tokio::time::pause() in async tests
- **Confidence**: 0.72
- **Support**: 10 insights from 10 episodes
- **Evidence**: Flaky test rate dropped from 70% to 0% with time mocking
```

**Test assertions**:
- D1 produces >= 7 Warnings and >= 2 Insights
- D2 produces 1 cluster with >= 7 members
- D2 does not promote until mean confidence >= 0.7
- D3 produces valid Markdown with the promoted Heuristic
- Cross-validation and AntiKnowledge checks pass

---

## Current Status and Gaps

**Implemented**:
- `DistillationBackend` trait and `Distiller` struct (LLM-based extraction)
- `spawn_episode_distillation()` for async extraction
- `TierProgression` struct with `analyze()`, `extract_insights()`, `promote_heuristics()`, `compile_playbook()`, `replay_heuristics()`
- `InsightRecord`, `HeuristicRule`, `PlaybookCompilation` types
- `PatternMiner` integration from `roko-learn`

**Missing**:
- Automatic Warning extraction in D1 (designed above; `extract_warnings()`)
- HDC-based clustering in D2 (designed above; `HdcClusterer`)
- Cross-validation enforcement in D2 (designed above; `cross_validation_check()`)
- AntiKnowledge contradiction check before promotion (designed above; `anti_knowledge_check()`)
- Dreams integration (designed above; `DreamsDistillationTrigger`)
- End-to-end integration test (scenario above; not yet automated)

---

## Cross-references

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for the types produced at each stage
- See [02-four-validation-tiers.md](./02-four-validation-tiers.md) for how tiers are assigned during promotion
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for how ingested entries are stored and queried
- See topic [10-dreams](../10-dreams/INDEX.md) for offline consolidation during Dreams
- See topic [05-learning](../05-learning/INDEX.md) for the episode logging that feeds D1
