//! Append-only JSONL episode logger.
//!
//! Implements the episode-logger component described in
//! `tmp/roko-progress/COMPONENTS/learn/episode-logger.md` and parity
//! checklist §16.1.1–§16.1.3. Each agent turn produces one [`Episode`]
//! record that is persisted as a single line of JSON on disk. The log is
//! append-only: records are never modified in place, and concurrent
//! writers are serialized through a process-wide [`parking_lot::Mutex`].
//!
//! The reader is tolerant: lines that fail to parse (a common outcome of
//! a crash mid-write or of forward-compatible schema changes) are
//! surfaced through a dedicated error variant rather than corrupting the
//! whole stream — callers choose whether to stop or continue.
//!
//! # Example
//!
//! ```no_run
//! use roko_learn::episode_logger::{Episode, EpisodeLogger};
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let logger = EpisodeLogger::new("/tmp/episodes.jsonl");
//! let ep = Episode::new("agent-1", "task-42");
//! logger.append(&ep).await?;
//! let all = EpisodeLogger::read_all("/tmp/episodes.jsonl").await?;
//! assert_eq!(all.len(), 1);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex as SyncMutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;

/// Maximum serialized size (in bytes) of a single episode's `extra`
/// field. Enforced in [`EpisodeLogger::append`] so that a runaway
/// optimizer cannot blow up the log.
const MAX_EXTRA_BYTES: usize = 16 * 1024;

/// Errors that can occur while appending to or reading from an episode
/// log.
#[derive(Debug, Error)]
pub enum LoggerError {
    /// An underlying filesystem call failed.
    #[error("episode logger i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization of an [`Episode`] to JSON failed. In practice this
    /// can only happen if a caller stuffs a non-serializable value into
    /// the `extra` map.
    #[error("episode logger serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// A JSONL line could not be deserialized as an [`Episode`]. The
    /// offending 1-based line number and the parser diagnostic are
    /// attached.
    #[error("episode logger parse error on line {line}: {source}")]
    Parse {
        /// 1-based line index within the JSONL file.
        line: usize,
        /// Underlying `serde_json` error.
        #[source]
        source: serde_json::Error,
    },
    /// The caller's `extra` map exceeds [`MAX_EXTRA_BYTES`] once
    /// serialized.
    #[error("episode `extra` field too large: {size} bytes (max {max})")]
    ExtraTooLarge {
        /// Serialized size in bytes.
        size: usize,
        /// Configured maximum.
        max: usize,
    },
}

/// Verdict produced by a single gate run on behalf of an agent turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateVerdict {
    /// Gate identifier ("compile", "test", "lint", …).
    #[serde(default)]
    pub gate: String,
    /// Whether the gate passed.
    #[serde(default)]
    pub passed: bool,
    /// Optional short diagnostic (hashed, never raw output).
    #[serde(default)]
    pub signature: Option<String>,
}

impl GateVerdict {
    /// Construct a new verdict.
    #[must_use]
    pub fn new(gate: impl Into<String>, passed: bool) -> Self {
        Self {
            gate: gate.into(),
            passed,
            signature: None,
        }
    }

    /// Attach an error signature to the verdict.
    #[must_use]
    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }
}

/// Token / cost / wall-clock accounting for one agent turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    /// Prompt/input tokens consumed.
    #[serde(default)]
    pub input_tokens: u64,
    /// Completion/output tokens produced.
    #[serde(default)]
    pub output_tokens: u64,
    /// Tokens read from the provider-side cache.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Tokens written to the provider-side cache.
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Dollar cost after cache discounts.
    #[serde(default)]
    pub cost_usd: f64,
    /// Dollar cost if the cache were cold (for regret accounting).
    #[serde(default)]
    pub cost_usd_without_cache: f64,
    /// Wall-clock latency, in milliseconds.
    #[serde(default)]
    pub wall_ms: u64,
}

impl Usage {
    /// Convenience constructor for the two most common fields.
    #[must_use]
    pub const fn tokens(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.0,
            cost_usd_without_cache: 0.0,
            wall_ms: 0,
        }
    }
}

/// One episode per completed agent turn.
///
/// The schema is intentionally forward-compatible: every field carries
/// `#[serde(default)]` so that older log lines continue to deserialize
/// after new fields are added.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Episode {
    /// Stable episode identifier (hash-derived).
    #[serde(default)]
    pub id: String,
    /// Wall-clock timestamp captured when the episode was constructed.
    #[serde(default = "Utc::now")]
    pub timestamp: DateTime<Utc>,
    /// Agent that produced the turn (e.g. `"claude-implementer"`).
    #[serde(default)]
    pub agent_id: String,
    /// Task identifier the agent was working on.
    #[serde(default)]
    pub task_id: String,
    /// Hash of the input signal that seeded the turn.
    #[serde(default)]
    pub input_signal_hash: String,
    /// Hash of the output signal the agent produced.
    #[serde(default)]
    pub output_signal_hash: String,
    /// Individual gate verdicts observed for the turn.
    #[serde(default)]
    pub gate_verdicts: Vec<GateVerdict>,
    /// Token / cost / latency accounting.
    #[serde(default)]
    pub usage: Usage,
    /// Whether the turn is considered successful overall.
    #[serde(default)]
    pub success: bool,
    /// Optional short failure reason (hashed, never raw output).
    #[serde(default)]
    pub failure_reason: Option<String>,
    /// Mark this episode as a headline — headline episodes are never
    /// pruned by [`EpisodeLogger::compact`], regardless of age or count.
    #[serde(default)]
    pub headline: bool,
    /// Forward-compat extension bag. Must serialize to ≤
    /// [`MAX_EXTRA_BYTES`].
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Episode {
    /// Construct a minimal episode for `agent_id` / `task_id` with a
    /// hash-derived id and `timestamp = Utc::now()`. All other fields
    /// take their defaults.
    #[must_use]
    pub fn new(agent_id: impl Into<String>, task_id: impl Into<String>) -> Self {
        let agent_id = agent_id.into();
        let task_id = task_id.into();
        let timestamp = Utc::now();
        let id = derive_id(&agent_id, &task_id, timestamp);
        Self {
            id,
            timestamp,
            agent_id,
            task_id,
            input_signal_hash: String::new(),
            output_signal_hash: String::new(),
            gate_verdicts: Vec::new(),
            usage: Usage::default(),
            success: false,
            failure_reason: None,
            headline: false,
            extra: HashMap::new(),
        }
    }

    /// Record the turn as successful.
    #[must_use]
    pub const fn succeeded(mut self) -> Self {
        self.success = true;
        self
    }

    /// Attach a failure reason and mark the turn as failed.
    #[must_use]
    pub fn failed(mut self, reason: impl Into<String>) -> Self {
        self.success = false;
        self.failure_reason = Some(reason.into());
        self
    }
}

/// Derive a stable id by hashing `(agent_id, task_id, timestamp)` with
/// Rust's default hasher. Not cryptographic — collisions are acceptable
/// because ids are scoped to a single log file.
fn derive_id(agent_id: &str, task_id: &str, timestamp: DateTime<Utc>) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    agent_id.hash(&mut hasher);
    task_id.hash(&mut hasher);
    timestamp.timestamp_nanos_opt().unwrap_or(0).hash(&mut hasher);
    format!("ep_{:016x}", hasher.finish())
}

/// Append-only JSONL episode logger.
///
/// Cheap to clone: the inner mutex lives behind an [`Arc`], so multiple
/// tasks can share a single logger and serialize their writes through
/// the same lock. A logger does *not* keep a file handle open between
/// calls — each `append` opens, writes, fsyncs, and closes. That keeps
/// the surface area tiny and avoids the "forgot to flush on drop"
/// footgun.
#[derive(Debug, Clone)]
pub struct EpisodeLogger {
    inner: Arc<LoggerInner>,
}

#[derive(Debug)]
struct LoggerInner {
    path: PathBuf,
    /// Counter of successful appends — protected by `parking_lot` for
    /// synchronous introspection even off the tokio runtime.
    writes: SyncMutex<u64>,
    /// Async mutex that serializes `append` across `.await` points so
    /// concurrent tasks never interleave bytes mid-line.
    write_gate: AsyncMutex<()>,
}

impl EpisodeLogger {
    /// Create a logger that writes to `path`. The file is created lazily
    /// on first `append`.
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            inner: Arc::new(LoggerInner {
                path: path.as_ref().to_path_buf(),
                writes: SyncMutex::new(0),
                write_gate: AsyncMutex::new(()),
            }),
        }
    }

    /// Return the path the logger writes to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Number of episodes successfully appended through this logger
    /// instance. Safe to call from any thread.
    #[must_use]
    pub fn write_count(&self) -> u64 {
        *self.inner.writes.lock()
    }

    /// Append a single episode to the log. The write is held under a
    /// process-local mutex, so concurrent callers never interleave
    /// bytes. The caller's task is suspended until the data has been
    /// flushed to the OS.
    ///
    /// # Errors
    ///
    /// Returns [`LoggerError::Io`] on any filesystem failure,
    /// [`LoggerError::Serde`] if the episode cannot be encoded, and
    /// [`LoggerError::ExtraTooLarge`] if `episode.extra` exceeds
    /// [`MAX_EXTRA_BYTES`] bytes when serialized.
    pub async fn append(&self, episode: &Episode) -> Result<(), LoggerError> {
        let extra_size = serde_json::to_vec(&episode.extra)?.len();
        if extra_size > MAX_EXTRA_BYTES {
            return Err(LoggerError::ExtraTooLarge {
                size: extra_size,
                max: MAX_EXTRA_BYTES,
            });
        }
        let mut line = serde_json::to_string(episode)?;
        line.push('\n');
        // Serialize writers within this process so concurrent appends
        // cannot interleave bytes across a single JSONL record.
        let gate = self.inner.write_gate.lock().await;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.inner.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        // Durability: a crash mid-write can leave at most a partial
        // trailing line, which the reader tolerates.
        file.sync_all().await?;
        drop(gate);
        *self.inner.writes.lock() += 1;
        Ok(())
    }

    /// Read every well-formed episode from `path`, preserving write
    /// order.
    ///
    /// If the file does not exist, an empty vector is returned.
    /// Malformed lines (truncated tail, schema drift, …) produce a
    /// [`LoggerError::Parse`] with the offending line number. Callers
    /// that want to tolerate partial tails can match on that variant
    /// and recover.
    ///
    /// # Errors
    ///
    /// Returns [`LoggerError::Io`] if the file cannot be opened/read
    /// (other than "missing"), or [`LoggerError::Parse`] on the first
    /// unparseable line.
    pub async fn read_all(path: impl AsRef<Path>) -> Result<Vec<Episode>, LoggerError> {
        let path = path.as_ref();
        let bytes = match tokio::fs::read(path).await {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(LoggerError::Io(err)),
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut out = Vec::new();
        for (idx, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            let episode: Episode = serde_json::from_str(raw).map_err(|source| {
                LoggerError::Parse {
                    line: idx + 1,
                    source,
                }
            })?;
            out.push(episode);
        }
        Ok(out)
    }

    /// Like [`Self::read_all`] but silently drops any line that fails
    /// to parse. Useful for tolerating a truncated tail after a crash.
    ///
    /// # Errors
    ///
    /// Returns [`LoggerError::Io`] on filesystem failure. Parse errors
    /// are swallowed.
    pub async fn read_all_lossy(path: impl AsRef<Path>) -> Result<Vec<Episode>, LoggerError> {
        let path = path.as_ref();
        let bytes = match tokio::fs::read(path).await {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(LoggerError::Io(err)),
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut out = Vec::new();
        for raw in text.lines() {
            if raw.trim().is_empty() {
                continue;
            }
            if let Ok(ep) = serde_json::from_str::<Episode>(raw) {
                out.push(ep);
            }
        }
        Ok(out)
    }

    /// Run age-based and size-based retention, pruning oldest episodes
    /// first while preserving those marked as [`Episode::headline`].
    ///
    /// The compaction is atomic: survivors are written to a temporary
    /// `.compacting` sibling, fsynced, then renamed over the original
    /// file.
    ///
    /// # Errors
    ///
    /// Returns [`LoggerError::Io`] on filesystem failure or
    /// [`LoggerError::Serde`] if a surviving episode cannot be
    /// re-serialized.
    pub async fn compact(
        &self,
        now: DateTime<Utc>,
        policy: &RetentionPolicy,
    ) -> Result<CompactStats, LoggerError> {
        let _gate = self.inner.write_gate.lock().await;

        let episodes = Self::read_all_lossy(&self.inner.path).await?;
        let before = episodes.len();

        let age_cutoff = now - chrono::Duration::days(i64::from(policy.max_age_days));

        // Phase 1: age-based pruning — drop episodes older than cutoff
        // unless they are headlines.
        let mut survivors: Vec<Episode> = episodes
            .into_iter()
            .filter(|ep| ep.headline || ep.timestamp >= age_cutoff)
            .collect();

        // Phase 2: size-based pruning — if still over max_episodes, drop
        // the oldest non-headline episodes first.
        if survivors.len() > policy.max_episodes {
            // Partition into headlines (always kept) and normals.
            let (headlines, mut normals): (Vec<Episode>, Vec<Episode>) =
                survivors.into_iter().partition(|ep| ep.headline);

            // Sort normals by timestamp descending so we can truncate the
            // tail (oldest).
            normals.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

            let keep_normals = policy.max_episodes.saturating_sub(headlines.len());
            normals.truncate(keep_normals);

            // Recombine and sort by timestamp ascending (original write
            // order) for the rewritten file.
            survivors = headlines.into_iter().chain(normals).collect();
            survivors.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        }

        let after = survivors.len();
        let removed = before.saturating_sub(after);

        // Write survivors to a temporary sibling.
        let compacting_path = self.inner.path.with_extension("compacting");
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&compacting_path)
                .await?;
            for ep in &survivors {
                let mut line = serde_json::to_string(ep)?;
                line.push('\n');
                file.write_all(line.as_bytes()).await?;
            }
            file.flush().await?;
            file.sync_all().await?;
        }

        // Compute bytes reclaimed.
        let original_size = tokio::fs::metadata(&self.inner.path)
            .await
            .map_or(0, |m| m.len());
        let new_size = tokio::fs::metadata(&compacting_path)
            .await
            .map_or(0, |m| m.len());
        let bytes_reclaimed = original_size.saturating_sub(new_size);

        // Atomic rename over the original.
        tokio::fs::rename(&compacting_path, &self.inner.path).await?;

        Ok(CompactStats {
            before,
            after,
            removed,
            bytes_reclaimed,
        })
    }
}

/// Age-based + size-based retention configuration.
///
/// Used by [`EpisodeLogger::compact`] to decide which episodes to keep.
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    /// Maximum number of episodes retained after compaction.
    pub max_episodes: usize,
    /// Maximum age in days — episodes older than this are pruned (unless
    /// marked as [`Episode::headline`]).
    pub max_age_days: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_episodes: 200,
            max_age_days: 90,
        }
    }
}

/// Statistics returned by [`EpisodeLogger::compact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactStats {
    /// Number of episodes before compaction.
    pub before: usize,
    /// Number of episodes after compaction.
    pub after: usize,
    /// Number of episodes removed.
    pub removed: usize,
    /// Approximate bytes reclaimed on disk.
    pub bytes_reclaimed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_log() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join("episodes.jsonl");
        (dir, path)
    }

    fn sample(agent: &str, task: &str, success: bool) -> Episode {
        let mut ep = Episode::new(agent, task);
        ep.success = success;
        ep.usage = Usage::tokens(100, 50);
        ep.gate_verdicts.push(GateVerdict::new("compile", success));
        ep
    }

    #[tokio::test]
    async fn empty_log_returns_empty_vec() {
        let (_dir, path) = tmp_log();
        let episodes = EpisodeLogger::read_all(&path).await.expect("read empty");
        assert!(episodes.is_empty());
    }

    #[tokio::test]
    async fn single_append_and_read() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let ep = sample("agent-a", "task-1", true);
        logger.append(&ep).await.expect("append");
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].agent_id, "agent-a");
        assert_eq!(all[0].task_id, "task-1");
        assert!(all[0].success);
        assert_eq!(all[0].gate_verdicts.len(), 1);
        assert_eq!(all[0].gate_verdicts[0].gate, "compile");
    }

    #[tokio::test]
    async fn multi_append_preserves_order() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        for i in 0..5 {
            let ep = sample("agent-a", &format!("task-{i}"), i % 2 == 0);
            logger.append(&ep).await.expect("append");
        }
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 5);
        for (i, ep) in all.iter().enumerate() {
            assert_eq!(ep.task_id, format!("task-{i}"));
            assert_eq!(ep.success, i % 2 == 0);
        }
    }

    #[tokio::test]
    async fn persists_across_reopens() {
        let (_dir, path) = tmp_log();
        {
            let logger = EpisodeLogger::new(&path);
            logger
                .append(&sample("a", "first", true))
                .await
                .expect("append 1");
        }
        {
            let logger = EpisodeLogger::new(&path);
            logger
                .append(&sample("a", "second", false))
                .await
                .expect("append 2");
        }
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].task_id, "first");
        assert_eq!(all[1].task_id, "second");
    }

    #[tokio::test]
    async fn invalid_line_returns_parse_error() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        logger
            .append(&sample("a", "ok", true))
            .await
            .expect("append");
        // Hand-append a malformed line to simulate a crash tail.
        tokio::fs::write(
            &path,
            format!(
                "{}\n{{not json\n",
                serde_json::to_string(&sample("a", "ok", true)).expect("serialize")
            ),
        )
        .await
        .expect("write");
        let err = EpisodeLogger::read_all(&path).await.unwrap_err();
        match err {
            LoggerError::Parse { line, .. } => assert_eq!(line, 2),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_all_lossy_tolerates_bad_lines() {
        let (_dir, path) = tmp_log();
        let good = serde_json::to_string(&sample("a", "ok", true)).expect("serialize");
        tokio::fs::write(&path, format!("{good}\n{{broken\n{good}\n"))
            .await
            .expect("write");
        let all = EpisodeLogger::read_all_lossy(&path)
            .await
            .expect("lossy read");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn concurrent_appends_do_not_interleave() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let mut handles = Vec::new();
        for worker in 0..8_u32 {
            let logger = logger.clone();
            handles.push(tokio::spawn(async move {
                for i in 0..10_u32 {
                    let ep = sample(&format!("worker-{worker}"), &format!("t-{i}"), true);
                    logger.append(&ep).await.expect("append");
                }
            }));
        }
        for h in handles {
            h.await.expect("join");
        }
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 80);
        // Every line parsed successfully → no interleaving.
        for ep in &all {
            assert!(ep.agent_id.starts_with("worker-"));
        }
    }

    #[tokio::test]
    async fn extra_too_large_is_rejected() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let mut ep = sample("a", "big", true);
        let big_string: String = "x".repeat(MAX_EXTRA_BYTES + 1);
        ep.extra.insert(
            "payload".to_string(),
            serde_json::Value::String(big_string),
        );
        let err = logger.append(&ep).await.unwrap_err();
        match err {
            LoggerError::ExtraTooLarge { size, max } => {
                assert!(size > max);
                assert_eq!(max, MAX_EXTRA_BYTES);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        // Nothing should have been written.
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn failure_reason_round_trips() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let ep = Episode::new("a", "t").failed("E0277:Send+Sync");
        logger.append(&ep).await.expect("append");
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
        assert!(!all[0].success);
        assert_eq!(all[0].failure_reason.as_deref(), Some("E0277:Send+Sync"));
    }

    #[tokio::test]
    async fn empty_and_whitespace_lines_ignored() {
        let (_dir, path) = tmp_log();
        let good = serde_json::to_string(&sample("a", "ok", true)).expect("serialize");
        tokio::fs::write(&path, format!("\n{good}\n\n\n"))
            .await
            .expect("write");
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn ids_are_populated_and_distinct() {
        let a = Episode::new("agent", "t1");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let b = Episode::new("agent", "t2");
        assert!(a.id.starts_with("ep_"));
        assert!(b.id.starts_with("ep_"));
        assert_ne!(a.id, b.id);
    }

    // ---- Retention / GC tests (§16.1.3) ----

    /// Helper: build an episode with a specific timestamp.
    fn episode_at(agent: &str, task: &str, ts: DateTime<Utc>) -> Episode {
        let mut ep = sample(agent, task, true);
        ep.timestamp = ts;
        // Re-derive id so it's unique per timestamp.
        ep.id = format!("ep_{agent}_{task}_{}", ts.timestamp());
        ep
    }

    #[tokio::test]
    async fn compact_size_exactly_n() {
        // Exactly max_episodes → nothing pruned.
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 5,
            max_age_days: 365,
        };
        for i in 0..5u32 {
            let ep = episode_at("a", &format!("t{i}"), now - chrono::Duration::hours(i64::from(i)));
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 5);
        assert_eq!(stats.after, 5);
        assert_eq!(stats.removed, 0);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 5);
    }

    #[tokio::test]
    async fn compact_size_n_plus_one() {
        // max_episodes + 1 → oldest one pruned.
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 5,
            max_age_days: 365,
        };
        for i in 0..6u32 {
            let ep = episode_at("a", &format!("t{i}"), now - chrono::Duration::hours(i64::from(5 - i)));
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 6);
        assert_eq!(stats.after, 5);
        assert_eq!(stats.removed, 1);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 5);
        // The oldest episode (t0, earliest timestamp) should be gone.
        // Episodes were appended in ascending timestamp order (5-i hours ago),
        // so t0 is the oldest.
        assert!(remaining.iter().all(|ep| ep.task_id != "t0"));
    }

    #[tokio::test]
    async fn compact_size_n_minus_one() {
        // max_episodes - 1 → nothing pruned.
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 5,
            max_age_days: 365,
        };
        for i in 0..4u32 {
            let ep = episode_at("a", &format!("t{i}"), now - chrono::Duration::hours(i64::from(i)));
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 4);
        assert_eq!(stats.after, 4);
        assert_eq!(stats.removed, 0);
    }

    #[tokio::test]
    async fn compact_age_prunes_old_episodes() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 1000,
            max_age_days: 30,
        };
        // 3 recent, 2 old (> 30 days).
        for i in 0..3u32 {
            let ep = episode_at("a", &format!("recent-{i}"), now - chrono::Duration::days(i64::from(i)));
            logger.append(&ep).await.unwrap();
        }
        for i in 0..2u32 {
            let ep = episode_at("a", &format!("old-{i}"), now - chrono::Duration::days(31 + i64::from(i)));
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 5);
        assert_eq!(stats.after, 3);
        assert_eq!(stats.removed, 2);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert!(remaining.iter().all(|ep| ep.task_id.starts_with("recent-")));
    }

    #[tokio::test]
    async fn compact_preserves_headlines() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 2,
            max_age_days: 10,
        };
        // 1 headline that is old (would be pruned by age) and 3 normals.
        let mut headline_ep = episode_at("a", "headline-old", now - chrono::Duration::days(100));
        headline_ep.headline = true;
        logger.append(&headline_ep).await.unwrap();
        for i in 0..3u32 {
            let ep = episode_at("a", &format!("normal-{i}"), now - chrono::Duration::hours(i64::from(i)));
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        // Headline survives age and size pruning. max_episodes=2 means
        // 1 headline + at most 1 normal (the most recent one).
        assert_eq!(stats.before, 4);
        assert_eq!(stats.after, 2);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().any(|ep| ep.task_id == "headline-old"));
        // The kept normal should be the most recent one (normal-0,
        // which is 0 hours ago).
        assert!(remaining.iter().any(|ep| ep.task_id == "normal-0"));
    }

    #[tokio::test]
    async fn compact_combined_age_and_size() {
        // Age removes some, then size cap removes more.
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 3,
            max_age_days: 30,
        };
        // 2 old episodes (pruned by age).
        for i in 0..2u32 {
            let ep = episode_at("a", &format!("old-{i}"), now - chrono::Duration::days(60 + i64::from(i)));
            logger.append(&ep).await.unwrap();
        }
        // 5 recent episodes → after age pruning only 5 remain, then
        // size cap prunes to 3.
        for i in 0..5u32 {
            let ep = episode_at("a", &format!("recent-{i}"), now - chrono::Duration::hours(i64::from(i)));
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 7);
        assert_eq!(stats.after, 3);
        assert_eq!(stats.removed, 4);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 3);
        // Should have the 3 most-recent episodes.
        let ids: Vec<&str> = remaining.iter().map(|ep| ep.task_id.as_str()).collect();
        assert!(ids.contains(&"recent-0"));
        assert!(ids.contains(&"recent-1"));
        assert!(ids.contains(&"recent-2"));
    }

    #[tokio::test]
    async fn compact_empty_log() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy::default();
        // Compact on a non-existent file should succeed gracefully.
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 0);
        assert_eq!(stats.after, 0);
        assert_eq!(stats.removed, 0);
    }

    #[tokio::test]
    async fn compact_preserves_write_order() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 3,
            max_age_days: 365,
        };
        // Write 5 episodes with ascending timestamps.
        for i in 0..5u32 {
            let ep = episode_at("a", &format!("t{i}"), now - chrono::Duration::hours(i64::from(4 - i)));
            logger.append(&ep).await.unwrap();
        }
        logger.compact(now, &policy).await.unwrap();
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 3);
        // Should be in ascending timestamp order (most recent 3).
        for pair in remaining.windows(2) {
            assert!(pair[0].timestamp <= pair[1].timestamp);
        }
    }

    #[tokio::test]
    async fn retention_policy_defaults() {
        let policy = RetentionPolicy::default();
        assert_eq!(policy.max_episodes, 200);
        assert_eq!(policy.max_age_days, 90);
    }
}
