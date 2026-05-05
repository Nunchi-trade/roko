# Episodes and Playbooks

> Depth for [07-LEARNING.md](../../unified/07-LEARNING.md). Episode logging as a Store Cell, playbook rules as Heuristic Signals with demurrage-driven retention, and the Voyager-style skill library as a Memory of verified reusable programs -- all expressed as a Loop from raw observation to injectable knowledge.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, demurrage, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (Store protocol, Score protocol, Verify protocol, predict-publish-correct), [06-MEMORY](../../unified/06-MEMORY.md) (Memory specialization, demurrage economics, Heuristics), [07-LEARNING](../../unified/07-LEARNING.md) (L1-L4 Loop taxonomy)

**Source docs**: `docs/05-learning/00-episode-logger.md`, `docs/05-learning/01-playbook-system.md`, `docs/05-learning/02-skill-library-voyager.md`

---

## 1. The Episode-to-Heuristic Loop

Learning starts with observation and ends with injectable knowledge. The pipeline has four tiers, each a Cell processing Signals from the tier below:

```
Tier 1: Episodes         Store Cell (append-only JSONL)
          |
          v
Tier 2: Patterns          Score Cell (trigram mining + HDC clustering)
          |
          v
Tier 3: Heuristics        Signal kind (when/then + mandatory falsifier + calibration)
          |
          v
Tier 4: Playbook Rules    Compose input (injected into agent prompts)
```

This is a **Loop**: playbook rules injected into prompts affect agent behavior, which produces new episodes, which feed pattern extraction, which updates heuristics, which revises playbook rules. The feedback edge runs from Tier 4 output back to Tier 1 input.

```toml
[graph]
name = "episode-to-heuristic-loop"
loop = true

[[nodes]]
id = "episode-store"
cell = "roko:episode-logger"
protocol = "Store"

[[nodes]]
id = "pattern-scorer"
cell = "roko:pattern-miner"
protocol = "Score"

[[nodes]]
id = "heuristic-verifier"
cell = "roko:heuristic-calibrator"
protocol = "Verify"

[[nodes]]
id = "playbook-composer"
cell = "roko:playbook-rules"
protocol = "Compose"

[[edges]]
from = "episode-store"
to = "pattern-scorer"

[[edges]]
from = "pattern-scorer"
to = "heuristic-verifier"

[[edges]]
from = "heuristic-verifier"
to = "playbook-composer"

[[edges]]
from = "playbook-composer"
to = "episode-store"
condition = "agent_turn_completed"
```

---

## 2. Episodes as Store Cells

Every agent turn produces exactly one `Episode` Signal, appended to `.roko/learn/episodes.jsonl`. The episode logger is a **Store protocol Cell** -- it puts Signals in and gets them out, nothing more.

### Episode Signal Schema

```rust
/// An Episode is a Signal that records one complete agent turn.
/// It is content-addressed (SHA-256 of serialized content),
/// HDC-fingerprinted (10,240-bit), and scored along 5 axes.
///
/// See [01-SIGNAL.md](../../unified/01-SIGNAL.md) for the Signal type.
pub struct Episode {
    pub id: String,
    pub agent_id: String,
    pub task_id: String,
    pub plan_id: String,
    pub role: String,
    pub model: String,
    pub backend: String,
    pub success: bool,
    pub iteration: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub gate_verdicts: Vec<GateVerdict>,
    pub timestamp: DateTime<Utc>,
    /// 10,240-bit HDC fingerprint for sub-microsecond similarity search.
    pub hdc_fingerprint: Option<HdcVector>,
    pub extra: HashMap<String, Value>,
}

pub struct GateVerdict {
    pub gate: String,
    pub passed: bool,
    /// Content hash of error output (compact, dedup-friendly).
    pub signature: Option<String>,
}
```

### Why JSONL

The Store Cell persists episodes as one JSON object per line. This design choice maps directly to the Store protocol's crash-safety requirement:

| Property | JSONL | SQLite | Parquet |
|---|---|---|---|
| Append-safe | Yes (O_APPEND) | No (WAL) | No |
| Corruption isolation | Per-line | Whole-DB | Whole-file |
| Schema flexibility | Yes | Limited | Limited |

A crash mid-write corrupts at most one line. The next line parses independently. This is the Store protocol's durability guarantee made concrete.

### Append Pipeline

```
Agent Turn Completes
    |
    v
Store Cell: episode-logger
    |-- 1. Validate: extra field <= 16 KB
    |-- 2. Compute HDC fingerprints (text + metadata)
    |-- 3. Serialize to JSON line
    |-- 4. Acquire process-wide Mutex (parking_lot, not tokio)
    |-- 5. Open with O_APPEND | O_CREAT
    |-- 6. Write line
    |-- 7. Release mutex
    |
    v
Pulse on Bus: topic "episode.appended"
```

The mutex is synchronous because the critical section is a single `write_all` syscall. Concurrent agent tasks serialize at the process level; separate processes rely on OS `O_APPEND` atomicity.

### Tiered Storage

Long-running instances use tiered compression inspired by prioritized experience replay (Schaul et al. 2016):

```
Hot tier   (0-7 days)   -> episodes.jsonl          Raw JSONL, full fidelity
Warm tier  (7-90 days)  -> episodes-warm.jsonl.zst  Zstandard compressed
Cold tier  (90+ days)   -> episodes-cold.bin         HDC superposition summaries
```

The cold tier exploits HDC superposition: merging N episode fingerprints into a single 10,240-bit vector preserves statistical similarity properties while discarding individual records. Compression ratio: ~400x vs raw JSONL.

### Episode Importance Scoring

Not all episodes carry equal learning signal. Importance is a Score protocol computation over five dimensions:

```rust
/// Score Cell: rate episode importance along 5 axes.
/// See [02-CELL.md](../../unified/02-CELL.md) for the Score protocol.
pub struct EpisodeImportance {
    pub score: f64,
    pub components: ImportanceComponents,
}

pub struct ImportanceComponents {
    pub surprisal: f64,         // |predicted_prob - actual_outcome|
    pub novelty: f64,           // HDC distance to nearest recent neighbor
    pub difficulty_signal: f64, // hard success or easy failure
    pub information_gain: f64,  // gradient magnitude for bandit updates
    pub diversity: f64,         // 1/sqrt(count in same slice)
}
```

Default weights: surprisal 0.30, novelty 0.20, difficulty 0.15, information gain 0.20, diversity 0.15. Surprisal dominates because it is the predict-publish-correct error signal -- the "TD error" of the episode stream.

---

## 3. Playbook Rules as Heuristic Signals

Playbook rules are the actionable output of the learning system. In unified terms, each rule is a **Heuristic Signal** -- a Signal of kind `Heuristic` with a when/then clause, mandatory falsifier, and calibration record (see [06-MEMORY.md](../../unified/06-MEMORY.md)).

### Rule Schema (Unified Vocabulary)

```rust
/// A playbook rule is a Heuristic Signal with demurrage-driven retention.
///
/// The when/then structure maps to the Heuristic kind from
/// [06-MEMORY.md](../../unified/06-MEMORY.md):
///   when: Triggers (file globs, tags, categories, error signatures, roles)
///   then: body text injected into agent prompt
///   falsifier: contradiction by gate failure when rule was active
///   calibration: (validations, contradictions, confidence)
pub struct PlaybookRule {
    pub rule_id: String,
    pub title: String,
    pub body: String,
    pub triggers: Triggers,

    // -- Demurrage economics (see [01-SIGNAL.md](../../unified/01-SIGNAL.md)) --
    pub balance: f64,
    pub demurrage_paid: f64,

    // -- Calibration (predict-publish-correct) --
    pub confidence: f64,       // bounded [0.0, 0.95]
    pub validations: u32,
    pub contradictions: u32,

    pub last_applied: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub source_episodes: Vec<String>,
}
```

### Confidence Dynamics

The asymmetric update rate encodes epistemic humility:

| Event | Confidence change | Balance change |
|---|---|---|
| Validation (rule predicted correctly) | +0.05 | Reinforcement bonus |
| Contradiction (rule predicted incorrectly) | -0.10 | Reinforcement loss + cooling |
| Successful reuse / citation | None | Reinforcement bonus |
| Demurrage tick | None | Holding cost (flat tax) |
| Balance or confidence below floor | None | Rule pruned |

The 2x asymmetry (contradictions penalize twice what validations reward) ensures that inaccurate rules are demoted faster than accurate ones are promoted. The 0.95 confidence ceiling prevents epistemic closure -- every rule retains a 5% doubt margin.

### Retention via Demurrage, Not Age

Traditional systems prune rules by age. Roko uses demurrage: a rule that is actively cited, validated, and reused has its balance replenished. A rule that sits idle loses balance through the flat holding tax. Stale-but-once-good rules do not petrify in prompts forever -- they cool into cold storage as their balance drains.

This is the demurrage economics from [01-SIGNAL.md](../../unified/01-SIGNAL.md) applied to procedural knowledge.

### Rule Lifecycle Loop

```
Episodes (Store Cell)
    |
    v
Pattern Discovery (Score Cell: trigram mining)
    |  support_count >= 5?
    v
Promoted to Rule (Heuristic Signal)
    |
    +---> Injected into agent prompt (Compose protocol)
    |          |
    |          v
    |     Agent produces Episode
    |          |
    |          v
    +---> Validation / Contradiction (Verify protocol)
    |          |
    |     confidence update + balance adjustment
    |          |
    +---> Demurrage tick (balance -= flat_tax)
    |          |
    |     balance < floor?
    |          |
    |          v
    +---> Pruned (Signal moves to cold Store tier)
```

This is a concrete instance of the predict-publish-correct pattern: the rule predicts "if these conditions hold, follow this advice." The gate verdict is the outcome. The calibration update is the correction.

---

## 4. Skill Library as Memory

The skill library implements the Voyager insight (Wang et al. 2023): agent systems should monotonically accumulate verified procedures. In unified terms, the skill library is a **Memory specialization** -- a Store Cell with demurrage and consolidation -- that holds reusable agent programs.

### Skill Signal Schema

```rust
/// A Skill is a Signal of kind Procedure stored in the skill library Memory.
///
/// Skills capture "how to do X" (offensive knowledge).
/// Playbook rules capture "watch out for Y" (defensive knowledge).
/// Both are injected into prompts via the Compose protocol.
pub struct Skill {
    pub name: String,
    pub summary: String,
    pub prompt_template: String,
    pub required_tools: Vec<String>,
    pub pattern: String,         // numbered-step recipe, <= 750 chars
    pub tags: Vec<String>,
    pub files: Vec<String>,

    // -- Usage telemetry (predict-publish-correct) --
    pub success_rate: f64,
    pub usage_count: u64,
    pub match_count: u32,
    pub validated_count: u32,

    pub score: f64,
    pub task_category: String,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_matched: Option<DateTime<Utc>>,
}
```

### Extraction Pipeline (Episode -> Skill)

```
Successful Episode (gate pass)
    |
    v
Analyze execution trace:
    What files? What tools? What recipe?
    |
    v
Construct Skill Signal
    |
    v
Dedup check: >= 70% tag overlap + same category?
    |           |
    YES         NO
    |           |
    v           v
Merge into    Register new
existing      skill in Memory
```

### Monotonic Growth Property

Unlike playbook rules (which are pruned by demurrage), skills grow monotonically. The cost of storing unused skills is negligible (~few KB each), while re-extraction requires a successful episode. This mirrors the Voyager observation: accumulated capability should only increase.

The only reduction mechanisms are deduplication (merge lower-scoring duplicates) and manual pruning.

### Skill Retrieval at Compose Time

```rust
/// At prompt composition time, the Compose protocol queries the
/// skill library Memory for relevant skills.
///
/// Filter: success_rate >= 0.5, usage_count >= 2
/// Rank by: score * success_rate * recency_bonus
/// Inject top-3 into prompt as "Recommended approach"
fn compose_skills(task: &TaskSpec, memory: &SkillLibrary) -> Vec<Skill> {
    let candidates = memory.search_by_tags(&task.tags)
        .chain(memory.search_by_files(&task.files))
        .filter(|s| s.success_rate >= 0.5 && s.usage_count >= 2)
        .sorted_by(|a, b| {
            let score_a = a.score * a.success_rate * recency_bonus(a.last_matched);
            let score_b = b.score * b.success_rate * recency_bonus(b.last_matched);
            score_b.partial_cmp(&score_a).unwrap()
        })
        .take(3)
        .collect();
    candidates
}
```

---

## 5. The Complete Learning Runtime Pipeline

All three subsystems are invoked in sequence by `LearningRuntime::record_completed_run()`. The ordering ensures raw episodes are always persisted before derived computations:

```
CompletedRunInput
    |
    +-- 1. EpisodeLogger::append(episode)         <- Store protocol
    +-- 2. CostsLog::append(cost_record)           <- Store protocol
    +-- 3. PlaybookStore::record_outcome()          <- Score protocol
    +-- 4. PlaybookRules::validate() / contradict() <- Verify protocol
    +-- 5. SkillLibrary::record_use()               <- Store protocol
    +-- 6. TaskMetric -> regression history          <- Score protocol
    +-- 7. ExperimentStore::record_outcome()        <- Score protocol
    +-- 8. PatternMiner::ingest_episode()           <- Score protocol
    +-- 9. CascadeRouter::update()                  <- Route protocol
    +-- 10. CFactor::compute()                      <- Observe (Lens)
```

If the process crashes at step 5, the episode (step 1) is already on disk. On restart, the pipeline can replay from the episode log to reconstruct downstream state.

---

## 6. Mori-Diffs Reality

The mori-diffs document (`tmp/mori-diffs/04-LEARNING.md`) identifies specific gaps between spec and runtime:

| Gap | Status | Impact |
|---|---|---|
| CascadeRouter never consulted at dispatch | Identified (L1) | Model routing is static, not learned |
| No routing observations recorded | Identified (L2) | Router never learns from outcomes |
| Episodes missing key fields (role, files_changed, provider) | Identified (L3) | Downstream learning subsystems get incomplete data |
| Efficiency events per-task not per-turn | Identified (L4) | Prompt section attribution impossible |
| Adaptive gate thresholds not loaded from disk | Identified (L5) | Thresholds reset on restart |
| No knowledge ingestion on gate success | Identified (L6) | Winning patterns not stored in neuro |
| force_backend not recorded as observation | Identified (L7) | Manual overrides don't feed learning |

The `LearningCollector` design in the mori-diffs document addresses these by collecting per-task learning data and flushing it atomically on gate completion. This replaces the ad-hoc `emit_episode` / `emit_efficiency_event` pattern.

---

## 7. Cross-Project Transfer

Both playbook rules and skills support cross-project transfer. Structural patterns (error signatures, trait implementation patterns) are project-agnostic. The transfer mechanism resets confidence and balance to neutral priors, forcing rules and skills to re-earn trust in the new context.

This maps to the heuristic commons (Loop 6) described in [autocatalytic-compounding.md](autocatalytic-compounding.md). Each deployment contributes once but benefits many times.

---

## What This Enables

1. **Structural learning**: episodes, patterns, heuristics, and skills form a four-tier Memory hierarchy that converts raw observation into injectable knowledge.
2. **Self-trimming rules**: demurrage ensures playbook rules stay fresh -- stale rules cool automatically without manual cleanup.
3. **Monotonic skill accumulation**: the Voyager property ensures capability only increases over time.
4. **Crash-safe persistence**: append-only JSONL with per-line corruption isolation means the learning log survives crashes.
5. **Sub-microsecond retrieval**: HDC fingerprints on episodes enable ~50ns similarity search for template suggestion and pattern matching.

## Feedback Loops

- **L1 (gamma)**: gate threshold EMA adjusts the Verify Cells that validate playbook rules.
- **L2 (theta)**: the cascade router uses playbook hit rate as a context feature for model routing.
- **L3 (delta)**: dream consolidation compresses old episodes into cold-tier HDC superpositions, merging similar episodes into representative Signals.
- **Predict-publish-correct**: each playbook rule predicts success when its triggers match. Gate verdicts are the outcome. Confidence updates are the correction.
- **Demurrage feedback**: retrieved/cited rules get balance bonuses; idle rules lose balance through holding tax. The retrieval surface sharpens over time.

## Open Questions

1. **Episode retention policy**: the current implementation never compacts. At ~200 KB/day this is fine for single-project use, but multi-project deployments may need the tiered compression pipeline. When should compaction trigger?
2. **Playbook rule ceiling at 0.95**: is 5% doubt margin the right number? If the codebase is extremely stable (e.g., a mature library with few changes), should the ceiling be higher?
3. **Skill extraction quality**: current extraction is heuristic-based (analyze tool calls and file modifications). LLM-based extraction (ask a cheap model to summarize the episode into a recipe) may produce higher-quality skills. What is the quality/cost tradeoff?
4. **Cross-project trust**: when importing rules from another project, the confidence reset to 0.50 may be too aggressive (useful rules take many validations to re-earn trust) or too generous (wrong rules survive too long). Should the reset be calibrated to the source project's similarity to the target?
5. **Relationship to c-factor-as-lens.md**: the c-factor Lens computes collective intelligence from cohort metrics. Should episode importance scoring be weighted by the c-factor of the cohort that produced the episode? High c-factor cohorts produce more trustworthy episodes.
