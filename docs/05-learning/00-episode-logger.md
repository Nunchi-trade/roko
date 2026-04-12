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

## Relationship to Other Documents

- **[05-pattern-discovery-trigram](05-pattern-discovery-trigram.md)** — The pattern miner consumes episodes via the `EpisodeView` trait, extracting trigrams from the `gate_verdicts` sequence.
- **[06-task-metrics-and-baselines](06-task-metrics-and-baselines.md)** — Task metrics are derived from episode data and written to a separate JSONL file for regression detection.
- **[04-cascade-router](04-cascade-router.md)** — The cascade router updates its bandit arms from episode outcomes (model, success, cost).
- **[02-skill-library-voyager](02-skill-library-voyager.md)** — The skill library extracts reusable capabilities from successful episodes.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Episodes are the data substrate for all 8 cybernetic feedback loops.

See also: [00-architecture](../00-architecture/INDEX.md) for the Engram/Signal data model that episodes extend, and [04-verification](../04-verification/INDEX.md) for the gate pipeline that produces `GateVerdict` records.
