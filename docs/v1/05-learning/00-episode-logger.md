# Episode Logger

> **Crate:** `roko-learn` · **Module:** `episode_logger.rs`
> **Persistence:** `.roko/learn/episodes.jsonl` (append-only JSONL)
> **Wiring:** `LearningRuntime::record_completed_run()` → `EpisodeLogger::append()`
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [05-pattern-discovery-trigram](05-pattern-discovery-trigram.md), [06-task-metrics-and-baselines](06-task-metrics-and-baselines.md)


> **Implementation**: Shipping

---

## Purpose

The episode logger is the foundational data substrate for all learning in Roko. Every agent turn — regardless of outcome — produces exactly one `Episode` record that is appended to a JSONL file on disk. This append-only log is the raw material from which every other learning subsystem draws its observations: pattern discovery mines trigrams from episode sequences, the cascade router updates bandit arms from episode outcomes, the regression detector computes baselines from episode metrics, and the skill library extracts reusable capabilities from successful episodes.

The design prioritizes durability and simplicity over query performance. Episodes are never modified in place. Concurrent writers are serialized through a process-wide mutex. The reader is tolerant: lines that fail to parse (a common outcome of a crash mid-write or of forward-compatible schema changes) are surfaced through a dedicated error variant rather than corrupting the whole stream.

---

## Episode Schema

The canonical `Episode` struct captures the full context of a single agent turn:

```rust
pub struct Episode {
    /// Unique episode identifier (UUID v4).
    pub id: String,
    /// Agent identifier that produced this episode.
    pub agent_id: String,
    /// Task identifier this episode belongs to.
    pub task_id: String,
    /// Plan identifier containing the task.
    pub plan_id: String,
    /// Agent role (e.g. "Implementer", "Reviewer").
    pub role: String,
    /// Model slug used for this turn (e.g. "claude-sonnet-4-20250514").
    pub model: String,
    /// Backend provider (e.g. "anthropic", "openrouter").
    pub backend: String,
    /// Whether the episode ended in a successful gate pass.
    pub success: bool,
    /// Zero-based iteration index within the task.
    pub iteration: u32,
    /// Input token count from the provider response.
    pub input_tokens: u64,
    /// Output token count from the provider response.
    pub output_tokens: u64,
    /// Actual cost in USD after cache discounts.
    pub cost_usd: f64,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Gate verdicts produced by the verification pipeline.
    pub gate_verdicts: Vec<GateVerdict>,
    /// Timestamp when the episode was recorded.
    pub timestamp: DateTime<Utc>,
    /// 10,240-bit HDC fingerprint of the episode content.
    pub hdc_fingerprint: Option<HdcVector>,
    /// Free-form metadata map (capped at 16 KB serialized).
    pub extra: HashMap<String, Value>,
}
```

### GateVerdict

Each gate execution within an episode produces a verdict:

```rust
pub struct GateVerdict {
    /// Gate identifier ("compile", "test", "lint", "diff", etc.).
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Optional short diagnostic (hashed, never raw output).
    pub signature: Option<String>,
}
```

The `signature` field stores a content hash of the error output rather than the raw text. This serves two purposes: it keeps the log compact (error outputs can be megabytes), and it enables exact-match deduplication across episodes without exposing potentially sensitive build output.

---

## Append Pipeline

The append path is designed for crash-safety and concurrency:

```
Agent Turn Completes
    │
    ▼
EpisodeLogger::append(&episode)
    │
    ├── 1. Validate: extra field ≤ MAX_EXTRA_BYTES (16 KB)
    │       → LoggerError::ExtraTooLarge if exceeded
    │
    ├── 2. Compute HDC fingerprints:
    │       text_fingerprint: bardo_primitives::hdc::text_fingerprint(content)
    │       metadata_fingerprint: text_fingerprint(agent_id + task_id + role)
    │       → stored in episode.extra["text_fingerprint"] and
    │         episode.extra["metadata_fingerprint"]
    │
    ├── 3. Serialize: serde_json::to_string(&episode) + "\n"
    │
    ├── 4. Acquire process-wide parking_lot::Mutex
    │
    ├── 5. Open file with O_APPEND | O_CREAT
    │
    ├── 6. Write serialized line
    │
    └── 7. Release mutex
```

The `MAX_EXTRA_BYTES` guard (16 KB) prevents a runaway optimizer from blowing up the log by stuffing arbitrary data into the `extra` map. This is a hard limit enforced at write time — the episode is rejected with `LoggerError::ExtraTooLarge` if the serialized `extra` field exceeds 16,384 bytes.

### Concurrency Model

The logger uses `parking_lot::Mutex` (not `tokio::Mutex`) for the write serialization lock. This is deliberate: the critical section is a single `write_all` syscall, which is fast enough that the synchronous mutex avoids the overhead of task scheduling. The mutex is process-wide (held by the `EpisodeLogger` instance), so concurrent agent tasks within the same process are serialized, while separate processes append independently (the OS guarantees atomicity for `O_APPEND` writes below `PIPE_BUF`).

---

## HDC Fingerprinting

Every episode is fingerprinted with a 10,240-bit hyperdimensional computing (HDC) vector from `bardo_primitives::hdc`. Two fingerprints are computed:

1. **Text fingerprint** — encodes the semantic content of the episode (task description, gate verdicts, etc.) into a binary vector using `bardo_primitives::hdc::text_fingerprint`.
2. **Metadata fingerprint** — encodes structural identity (agent_id, task_id, role) for structural similarity matching.

HDC fingerprints enable sub-microsecond similarity search: comparing two 10,240-bit vectors via Hamming distance takes ~50ns, compared to ~1μs for cosine distance on 768-dimensional float embeddings. This speed advantage is critical for real-time pattern matching during task dispatch, where the system must scan hundreds of historical episodes to find relevant patterns before the agent begins work.

The fingerprints are stored in the `extra` map under reserved keys (`text_fingerprint` and `metadata_fingerprint`) rather than as top-level fields. This keeps the `Episode` struct backward-compatible with older log entries that predate HDC support.

### Template Suggestion

The episode logger also supports template suggestion via HDC similarity. Given a new task context, the system can scan recent episodes (within `TEMPLATE_SUGGESTION_MAX_AGE_DAYS` = 30 days, up to `TEMPLATE_SUGGESTION_MAX_CANDIDATES` = 256 candidates) and find episodes with HDC similarity above `TEMPLATE_SUGGESTION_MIN_SIMILARITY` = 0.7. Successful episodes matching this threshold can be used to suggest prompt templates or skill patterns for the new task.

---

## Reading and Tolerance

The reader is designed to be tolerant of corruption:

```rust
impl EpisodeLogger {
    pub async fn read_all(path: impl AsRef<Path>) -> Result<Vec<Episode>, LoggerError> {
        // Opens file, reads line-by-line
        // Each line: serde_json::from_str::<Episode>(line)
        // On parse failure: LoggerError::Parse { line, source }
        // Caller decides whether to skip or abort
    }
}
```

The `LoggerError::Parse` variant includes the 1-based line number and the `serde_json` diagnostic, so callers can decide whether to skip corrupt lines or abort. In practice, the `LearningRuntime` skips corrupt lines and logs a warning — this is the right default for a system that must remain operational even after a crash mid-write.

### Why JSONL

The choice of JSONL (one JSON object per line) over alternatives:

| Format | Append-safe | Schema-flexible | Grep-friendly | Corruption-isolated |
|--------|-------------|-----------------|---------------|---------------------|
| JSONL  | Yes         | Yes             | Yes           | Yes (per-line)      |
| SQLite | No (WAL)    | Limited         | No            | No (whole-DB)       |
| Parquet| No          | Limited         | No            | No (whole-file)     |
| CSV    | Yes         | No              | Yes           | Yes (per-line)      |

JSONL's key advantage is corruption isolation: a crash during write corrupts at most one line. The next line is a fresh JSON object that parses independently. This property is essential for an append-only log that may be written to during agent crashes, OOM kills, or power failures.

---

## Retention and Compaction

The current implementation does not compact or rotate the episode log. The log grows monotonically. For a system running 100 tasks per day with ~2 KB per episode, this produces ~200 KB/day or ~73 MB/year — well within filesystem limits.

Future compaction strategies under consideration:

1. **Time-based rotation** — archive episodes older than 90 days to a compressed file, keeping the active log small for fast `read_all` scans.
2. **Summary compaction** — replace old episodes with aggregate summaries (pass rate, cost distribution, pattern counts) that preserve learning signal without individual records.
3. **HDC compaction** — merge similar episodes into a single representative episode using HDC superposition (element-wise majority of fingerprint bits), reducing storage while preserving the similarity search index.

None of these are implemented. The current approach is sufficient for the expected scale of self-hosted development.

---

## Integration with LearningRuntime

The episode logger is the first subsystem updated by `LearningRuntime::record_completed_run()`. The runtime constructs an `Episode` from the `CompletedRunInput` payload and appends it before updating any downstream subsystem:

```
CompletedRunInput
    │
    ├── 1. EpisodeLogger::append(episode)          ← you are here
    ├── 2. CostsLog::append(cost_record)
    ├── 3. PlaybookStore::record_outcome()
    ├── 4. PlaybookRules::validate() / contradict()
    ├── 5. SkillLibrary::record_use()
    ├── 6. TaskMetric → regression history
    ├── 7. ExperimentStore::record_outcome()
    ├── 8. PatternMiner::ingest_episode()
    ├── 9. CascadeRouter::update()
    └── 10. CFactor::compute()
```

This ordering ensures that the raw episode is always persisted before any derived computation runs. If the process crashes during step 5 (skill library update), the episode is already on disk and can be replayed on restart to reconstruct downstream state.

---

## Error Handling

The logger defines three error variants:

| Variant | When | Recovery |
|---------|------|----------|
| `LoggerError::Io` | Filesystem call failed (disk full, permissions) | Retry or alert operator |
| `LoggerError::Serde` | Episode serialization failed (non-serializable value in `extra`) | Fix the caller — this is a programming error |
| `LoggerError::ExtraTooLarge` | `extra` map exceeds 16 KB serialized | Trim the `extra` map before appending |
| `LoggerError::Parse` | A JSONL line could not be deserialized (corruption or schema change) | Skip the line and continue reading |

The `Parse` variant is the most common in practice. It occurs when:
- A crash interrupted a write, leaving a partial JSON line.
- A schema change added new fields that the current deserializer doesn't recognize (forward compatibility).
- Manual editing of the log file introduced syntax errors.

The tolerant reader handles all three cases by surfacing the error with the line number, letting the caller decide whether to skip or abort.

---

## Episode Compression and Tiered Storage

Long-running Roko instances generate thousands of episodes. Naive storage (keep everything in one JSONL) degrades read performance linearly. Episode compression provides a tiered storage architecture inspired by experience replay buffers in deep reinforcement learning (Mnih et al. 2015, Schaul et al. 2016).

### Tiered Architecture

```
Hot tier   (0-7 days)   → episodes.jsonl          Raw JSONL, full fidelity
Warm tier  (7-90 days)  → episodes-warm.jsonl.zst  Zstandard compressed, full fidelity
Cold tier  (90+ days)   → episodes-cold.bin         HDC superposition summaries only
```

### Rust Types

```rust
pub struct EpisodeStorageConfig {
    /// Days before moving to warm tier (default: 7).
    pub hot_retention_days: u32,
    /// Days before moving to cold tier (default: 90).
    pub warm_retention_days: u32,
    /// Zstandard compression level for warm tier (default: 3).
    pub zstd_level: i32,
    /// Maximum cold-tier summary count per (role, complexity) slice (default: 1000).
    pub cold_max_summaries: usize,
}

pub struct CompressedEpisodeSummary {
    /// HDC superposition of all episode fingerprints in this summary.
    pub hdc_superposition: HdcVector,
    /// Number of episodes merged into this summary.
    pub episode_count: u32,
    /// Aggregate pass rate.
    pub pass_rate: f64,
    /// Aggregate cost statistics.
    pub total_cost_usd: f64,
    pub avg_duration_ms: f64,
    /// Time range covered.
    pub earliest: DateTime<Utc>,
    pub latest: DateTime<Utc>,
    /// Role and complexity for this summary slice.
    pub role: String,
    pub complexity_band: String,
}
```

### Compression Algorithm (Pseudocode)

```
fn compact_tier(hot_path, warm_path, cold_path, config):
    episodes = read_all(hot_path)
    now = Utc::now()

    for ep in episodes:
        age_days = (now - ep.timestamp).num_days()
        if age_days > config.warm_retention_days:
            // Move to cold: merge into HDC superposition
            key = (ep.role, ep.complexity_band)
            cold_summaries[key].hdc_superposition |= ep.hdc_fingerprint  // bitwise OR (HDC superposition)
            cold_summaries[key].episode_count += 1
            cold_summaries[key].pass_rate = running_mean(...)
        elif age_days > config.hot_retention_days:
            // Move to warm: compress with zstd
            warm_buffer.push(ep)

    write_zstd(warm_path, warm_buffer, config.zstd_level)
    write_binary(cold_path, cold_summaries)
    truncate_hot(hot_path, episodes_still_hot)
```

### Space Savings

| Tier | Per-episode size | 10,000 episodes | Compression ratio |
|------|-----------------|-----------------|-------------------|
| Hot (raw JSONL) | ~2 KB | 20 MB | 1x (baseline) |
| Warm (zstd) | ~0.4 KB | 4 MB | 5x |
| Cold (HDC summary) | ~1.3 KB per slice | ~50 KB total | 400x |

The cold tier achieves extreme compression by exploiting HDC superposition: merging N episode fingerprints into a single 10,240-bit vector preserves the statistical properties (similarity search still works on the superposition) while discarding individual records. This is analogous to the "compressed sensing" property of high-dimensional random projections (Johnson-Lindenstrauss lemma).

### Integration with Existing Read Path

The tiered storage is transparent to consumers. `EpisodeLogger::read_recent(days)` reads only from hot + warm tiers (fast). `EpisodeLogger::similarity_search(query_hdc)` searches all three tiers, using the cold-tier superpositions for approximate matching against old episodes.

---

## Episode Importance Scoring

Not all episodes are equally valuable for learning. A routine successful episode that matches known patterns contributes little new information, while a surprising failure or an unexpected success on a hard task carries high learning signal. Episode importance scoring quantifies this, inspired by prioritized experience replay (Schaul et al. 2016) and the information-theoretic concept of surprisal.

### Importance Score Components

```rust
pub struct EpisodeImportance {
    /// Overall importance score in [0.0, 1.0].
    pub score: f64,
    /// Component breakdown.
    pub components: ImportanceComponents,
}

pub struct ImportanceComponents {
    /// Surprisal: how unexpected was this outcome given predictions?
    /// High when predicted pass but failed, or predicted fail but passed.
    /// = |predicted_probability - actual_outcome|
    pub surprisal: f64,

    /// Novelty: how different is this episode from recent episodes?
    /// Measured via HDC Hamming distance to nearest neighbor in last 100 episodes.
    /// Range [0.0, 1.0] where 1.0 = maximally novel.
    pub novelty: f64,

    /// Difficulty: was this a hard task that succeeded or an easy task that failed?
    /// Hard successes and easy failures are both highly informative.
    /// = |complexity_adjusted_expected_rate - actual_outcome|
    pub difficulty_signal: f64,

    /// Recency-weighted information gain: how much would including this episode
    /// change the current model parameters (bandit arms, pattern counts)?
    /// Approximated via gradient magnitude for bandit updates.
    pub information_gain: f64,

    /// Diversity contribution: does this episode cover an underrepresented
    /// region of the (role, complexity, model) space?
    /// = 1.0 / sqrt(count_in_same_slice)
    pub diversity: f64,
}
```

### Composite Scoring

```
importance = w_s * surprisal + w_n * novelty + w_d * difficulty_signal
           + w_i * information_gain + w_v * diversity

Default weights:
    w_s = 0.30  (surprisal is the strongest signal)
    w_n = 0.20  (novelty prevents redundant learning)
    w_d = 0.15  (difficulty calibrates expectations)
    w_i = 0.20  (information gain is directly actionable)
    w_v = 0.15  (diversity prevents blind spots)
```

### Applications

| Consumer | How importance is used |
|----------|----------------------|
| Pattern discovery | Weight trigram support by episode importance (important episodes count more) |
| Skill extraction | Prioritize skill extraction from high-importance successful episodes |
| Cascade router | Weight bandit updates by importance (surprising outcomes update more) |
| Compaction | Keep high-importance episodes in hot tier longer before compacting |
| Dashboard | Surface high-importance episodes as "Notable events" |

### Connection to Prioritized Experience Replay

In DQN-style reinforcement learning (Schaul et al. 2016), transitions are sampled from a replay buffer with probability proportional to their TD error — the difference between expected and observed reward. Episode importance scoring is the agent-system analogue: episodes with high surprisal (the "TD error" of the prediction system) receive higher priority in all downstream learning loops. Recent work on diversity-based experience replay (IJCAI 2025) further motivates the diversity component: ensuring the learning pipeline sees a representative sample of the episode space, not just the most surprising examples.

Uncertainty-based prioritization (Clements et al. 2019, extended in 2024) suggests an additional refinement: episodes where the system's uncertainty is highest (wide confidence intervals in the bandit, low pattern support) should be prioritized because they carry the most information about unexplored regions of the decision space. The `information_gain` component approximates this by measuring how much the episode would change model parameters.

---

## Episode Clustering and Automatic Pattern Discovery

Beyond trigram mining (see [05-pattern-discovery-trigram](05-pattern-discovery-trigram.md)), episode clustering provides a complementary pattern discovery mechanism that operates on the full episode representation rather than just action sequences. Clustering discovers structural groupings — families of episodes that share common characteristics across multiple dimensions simultaneously.

### Clustering Algorithm: Incremental DBSCAN over HDC Space

Traditional k-medoids (already implemented for cross-episode consolidation) requires a fixed k. For automatic pattern discovery, DBSCAN (Density-Based Spatial Clustering of Applications with Noise) is preferred because it discovers the number of clusters automatically and identifies outliers.

```rust
pub struct EpisodeClusterConfig {
    /// Minimum HDC similarity to consider two episodes "neighbors" (default: 0.72).
    pub eps_similarity: f64,
    /// Minimum episodes in a neighborhood to form a cluster (default: 3).
    pub min_points: usize,
    /// Maximum episodes to cluster per batch (default: 500).
    pub max_batch_size: usize,
    /// Similarity metric: "hamming" (default) or "cosine".
    pub metric: String,
}

pub struct EpisodeCluster {
    /// Cluster identifier (auto-assigned).
    pub cluster_id: u32,
    /// Medoid episode (most central member).
    pub medoid: Episode,
    /// HDC superposition of all members.
    pub superposition: HdcVector,
    /// Member count.
    pub size: usize,
    /// Aggregate statistics.
    pub pass_rate: f64,
    pub avg_cost_usd: f64,
    pub avg_duration_ms: f64,
    /// Dominant characteristics.
    pub dominant_role: String,
    pub dominant_model: String,
    pub dominant_complexity: String,
    /// Distinguishing features (what makes this cluster unique).
    pub distinguishing_features: Vec<String>,
}
```

### Incremental Clustering

Full DBSCAN is O(n^2) which is acceptable for batch processing but too slow for per-episode updates. Incremental DBSCAN (Ester et al. 1998, updated in Kranen et al. 2011) maintains clusters incrementally:

```
On new episode:
    1. Compute HDC fingerprint
    2. Find nearest cluster (HDC similarity to each cluster superposition)
    3. If similarity > eps_similarity:
        a. Add episode to cluster
        b. Update cluster superposition (bitwise OR)
        c. Update cluster statistics
    4. If no cluster matches:
        a. Add to "noise" buffer
        b. When noise buffer reaches min_points similar episodes:
           -> Form new cluster
```

### Automatic Pattern Extraction from Clusters

Each cluster represents a natural grouping of episodes. The system extracts interpretable patterns by analyzing what cluster members share:

```
Cluster #7: "Cross-crate config modifications" (42 episodes)
    Common features:
        - Files: crates/roko-core/src/config/*.rs (100%)
        - Role: Implementer (95%)
        - Model: claude-sonnet-4 (72%)
    Performance:
        - Pass rate: 0.62 (below baseline 0.75)
        - Avg iterations: 2.3 (above baseline 1.4)
    Suggested action:
        -> Create playbook rule: "Config modifications in roko-core require
           checking serde derives and TOML schema compatibility"
        -> Route to opus for this cluster (low pass rate with sonnet)
```

### Cluster Evolution Tracking

Clusters are not static. As the system improves, cluster characteristics change:

```rust
pub struct ClusterEvolution {
    pub cluster_id: u32,
    /// Pass rate trend (positive = improving).
    pub pass_rate_trend: f64,
    /// Cost trend (negative = getting cheaper).
    pub cost_trend: f64,
    /// Is this cluster shrinking (fewer new episodes match)?
    pub is_shrinking: bool,
    /// Episodes since last cluster member was added.
    pub episodes_since_last_member: u32,
}
```

A cluster that is shrinking and whose pass rate is improving indicates a problem that the system has learned to handle — the pattern is no longer causing failures. Conversely, a growing cluster with declining pass rate indicates an emerging problem that needs attention.

### Relationship to Hindsight Experience Replay

Hindsight experience replay (HER, Andrychowicz et al. 2017) re-labels failed episodes with alternative goals that were actually achieved, turning failures into successes for learning purposes. In the episode clustering context, this principle applies when a failed episode partially achieved a sub-goal: the cluster can identify which sub-goals were achieved and extract partial skills from the failure. For example, a task that failed the test gate but passed compile and lint gates still demonstrates successful compilation patterns that can be extracted as partial skills.

---

## Relationship to Other Documents

- **[05-pattern-discovery-trigram](05-pattern-discovery-trigram.md)** — The pattern miner consumes episodes via the `EpisodeView` trait, extracting trigrams from the `gate_verdicts` sequence.
- **[06-task-metrics-and-baselines](06-task-metrics-and-baselines.md)** — Task metrics are derived from episode data and written to a separate JSONL file for regression detection.
- **[04-cascade-router](04-cascade-router.md)** — The cascade router updates its bandit arms from episode outcomes (model, success, cost).
- **[02-skill-library-voyager](02-skill-library-voyager.md)** — The skill library extracts reusable capabilities from successful episodes.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Episodes are the data substrate for all 8 cybernetic feedback loops.

See also: [00-architecture](../00-architecture/INDEX.md) for the Engram/Signal data model that episodes extend, and [04-verification](../04-verification/INDEX.md) for the gate pipeline that produces `GateVerdict` records.
