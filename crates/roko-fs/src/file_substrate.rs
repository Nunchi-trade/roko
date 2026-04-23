//! File-backed substrate — durable JSONL log with in-memory index.

use async_trait::async_trait;
use parking_lot::RwLock;
use roko_core::{
    ContentHash, Context, Engram, Query, Substrate,
    error::{Result, RokoError},
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

#[cfg(any(test, feature = "hdc"))]
const HDC_TAG: &str = "hdc_fingerprint";

/// A substrate that persists signals to a JSONL log on disk.
///
/// Thread-safety: reads go through a `parking_lot::RwLock`-protected `HashMap`
/// (no blocking). Writes serialize through a tokio `Mutex` around the log
/// file, so concurrent `put`s are appended in order.
pub struct FileSubstrate {
    /// Directory containing `engrams.jsonl`.
    root: PathBuf,
    /// In-memory index: `ContentHash` → `Engram`.
    index: RwLock<HashMap<ContentHash, Engram>>,
    /// Serializes writes to the log file.
    log_writer: Mutex<File>,
    /// Human-readable name (kept for Debug / logging).
    #[allow(dead_code)]
    name: String,
}

impl FileSubstrate {
    /// Open (or create) a file substrate rooted at `root`.
    ///
    /// Creates `root/engrams.jsonl` if missing, and replays existing entries
    /// into the in-memory index.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory can't be created, the log file
    /// can't be opened, or the log is corrupt beyond recovery.
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).await?;
        let log_path = root.join("engrams.jsonl");

        // Replay: read any existing entries into the in-memory index.
        let index = replay_log(&log_path).await?;

        // Open for append — all subsequent writes go to the end.
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;

        let name = format!("file:{}", root.display());
        Ok(Self {
            root,
            index: RwLock::new(index),
            log_writer: Mutex::new(file),
            name,
        })
    }

    /// Path to the JSONL log file.
    #[must_use]
    pub fn log_path(&self) -> PathBuf {
        self.root.join("engrams.jsonl")
    }

    /// The root directory containing this substrate's storage.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Compact the log file: rewrite it containing only currently-live signals.
    /// This is the durable form of [`Substrate::prune`] — it shrinks the log
    /// file on disk after pruning decayed signals from the in-memory index.
    ///
    /// # Errors
    ///
    /// Returns an error if the temp-file swap fails.
    pub async fn compact(&self) -> Result<()> {
        let snapshot: Vec<Engram> = self.index.read().values().cloned().collect();
        let log_path = self.log_path();
        let tmp_path = self.root.join("engrams.jsonl.tmp");

        {
            let mut tmp = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&tmp_path)
                .await?;
            for sig in &snapshot {
                let line = serde_json::to_string(sig).map_err(RokoError::body_encode)?;
                tmp.write_all(line.as_bytes()).await?;
                tmp.write_all(b"\n").await?;
            }
            tmp.flush().await?;
            tmp.sync_all().await?;
        }

        fs::rename(&tmp_path, &log_path).await?;

        // Re-open the log writer to point at the compacted file.
        let new_writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;
        *self.log_writer.lock().await = new_writer;
        Ok(())
    }
}

async fn replay_log(log_path: &Path) -> Result<HashMap<ContentHash, Engram>> {
    let mut index = HashMap::new();
    if !log_path.exists() {
        return Ok(index);
    }
    let file = File::open(log_path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut line_no = 0usize;
    while let Some(line) = lines.next_line().await? {
        line_no += 1;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Engram>(&line) {
            Ok(sig) => {
                index.insert(sig.id, sig);
            }
            Err(e) => {
                // Last line may be a partial write after a crash — skip and
                // continue. For other malformed lines, log and keep going;
                // we prefer partial availability over total failure.
                tracing_line_error(log_path, line_no, &e);
            }
        }
    }
    Ok(index)
}

fn tracing_line_error(_path: &Path, _line: usize, _err: &serde_json::Error) {
    // Silently skip malformed lines. These are typically from older schema
    // versions or interrupted writes. Logging here would corrupt the TUI
    // since roko-fs doesn't depend on tracing.
}

fn matches_query(signal: &Engram, q: &Query, ctx: &Context) -> bool {
    if let Some(kinds) = &q.kinds {
        if !kinds.contains(&signal.kind) {
            return false;
        }
    }
    if let Some(author) = &q.author {
        if &signal.provenance.author != author {
            return false;
        }
    }
    if let Some(session) = &q.session {
        if signal.provenance.session.as_ref() != Some(session) {
            return false;
        }
    }
    if let Some(since) = q.since_ms {
        if signal.created_at_ms < since {
            return false;
        }
    }
    if let Some(until) = q.until_ms {
        if signal.created_at_ms > until {
            return false;
        }
    }
    if let Some(min_w) = q.min_weight {
        if signal.weight_at(ctx.now_ms) < min_w {
            return false;
        }
    }
    for (k, v) in &q.tags {
        match signal.tags.get(k) {
            Some(value) if value == v => {}
            _ => return false,
        }
    }
    true
}

#[async_trait]
impl Substrate for FileSubstrate {
    async fn put(&self, signal: Engram) -> Result<ContentHash> {
        // Dedupe: skip write if already present.
        if self.index.read().contains_key(&signal.id) {
            return Ok(signal.id);
        }
        // Attach HDC fingerprint when the feature is enabled and the signal
        // does not already carry one. The fingerprint is stored as a
        // base64-encoded tag so it survives JSON serialization without
        // inflating the line with raw bytes.
        let signal = attach_hdc_fingerprint(signal);
        let id = signal.id;
        // Serialize and append.
        let line = serde_json::to_string(&signal).map_err(RokoError::body_encode)?;
        let mut guard = self.log_writer.lock().await;
        guard.write_all(line.as_bytes()).await?;
        guard.write_all(b"\n").await?;
        guard.flush().await?;
        drop(guard);
        // Update index.
        self.index.write().insert(id, signal);
        Ok(id)
    }

    async fn get(&self, id: &ContentHash) -> Result<Option<Engram>> {
        Ok(self.index.read().get(id).cloned())
    }

    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Engram>> {
        let mut matching: Vec<Engram> = self
            .index
            .read()
            .values()
            .filter(|s| matches_query(s, q, ctx))
            .cloned()
            .collect();
        matching.sort_by(|a, b| {
            b.weight_at(ctx.now_ms)
                .partial_cmp(&a.weight_at(ctx.now_ms))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(limit) = q.limit {
            matching.truncate(limit);
        }
        Ok(matching)
    }

    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize> {
        let mut index = self.index.write();
        let before = index.len();
        index.retain(|_, s| s.weight_at(ctx.now_ms) > threshold);
        Ok(before - index.len())
        // Note: the log file is not rewritten here; call `compact()` to
        // reclaim disk space. Pruning from memory is the hot path; compaction
        // is a maintenance task.
    }

    async fn len(&self) -> Result<usize> {
        Ok(self.index.read().len())
    }

    fn name(&self) -> &'static str {
        "file_substrate"
    }
}

/// Attach a deterministic HDC fingerprint to a signal's tags.
///
/// The fingerprint is derived from `kind|body` so that signals with
/// identical semantic content produce identical vectors. Signals that
/// already carry the tag are returned unchanged. Fingerprinting never
/// fails: if anything goes wrong the signal is returned as-is.
#[cfg(feature = "hdc")]
fn attach_hdc_fingerprint(mut signal: Engram) -> Engram {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    if signal.tags.contains_key(HDC_TAG) {
        return signal;
    }
    let fingerprint = roko_primitives::hdc::fingerprint(&signal.body);
    signal
        .tags
        .insert(HDC_TAG.into(), BASE64.encode(fingerprint.to_bytes()));
    // Recompute the content hash since tags are identity-bearing.
    signal.id = signal.content_hash();
    signal
}

#[cfg(not(feature = "hdc"))]
const fn attach_hdc_fingerprint(signal: Engram) -> Engram {
    signal
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Decay, Kind, Score};
    use tempfile::TempDir;

    fn sig(kind: Kind, body: &str, t: i64) -> Engram {
        Engram::builder(kind)
            .body(Body::text(body))
            .created_at_ms(t)
            .build()
    }

    #[tokio::test]
    async fn open_creates_directory_and_log() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("fresh");
        assert!(!root.exists());
        let sub = FileSubstrate::open(&root).await.unwrap();
        assert!(root.exists());
        assert!(sub.log_path().exists());
    }

    #[tokio::test]
    async fn put_persists_and_get_retrieves() {
        let tmp = TempDir::new().unwrap();
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();
        let s = sig(Kind::Task, "hi", 0);
        let id = sub.put(s.clone()).await.unwrap();
        let got = sub.get(&id).await.unwrap();
        assert_eq!(got, Some(s));
    }

    #[tokio::test]
    async fn persistence_survives_restart() {
        let tmp = TempDir::new().unwrap();
        let s = sig(Kind::Episode, "survives reboot", 42);
        let id = s.id;
        {
            let sub = FileSubstrate::open(tmp.path()).await.unwrap();
            sub.put(s.clone()).await.unwrap();
        }
        // New instance — must replay the log.
        let sub2 = FileSubstrate::open(tmp.path()).await.unwrap();
        assert_eq!(sub2.len().await.unwrap(), 1);
        assert_eq!(sub2.get(&id).await.unwrap(), Some(s));
    }

    #[tokio::test]
    async fn multiple_puts_persist() {
        let tmp = TempDir::new().unwrap();
        {
            let sub = FileSubstrate::open(tmp.path()).await.unwrap();
            for i in 0..10 {
                sub.put(sig(Kind::Task, &format!("t{i}"), i)).await.unwrap();
            }
        }
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();
        assert_eq!(sub.len().await.unwrap(), 10);
    }

    #[tokio::test]
    async fn idempotent_put() {
        let tmp = TempDir::new().unwrap();
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();
        let s = sig(Kind::Task, "idem", 0);
        sub.put(s.clone()).await.unwrap();
        sub.put(s.clone()).await.unwrap();
        sub.put(s.clone()).await.unwrap();
        assert_eq!(sub.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn query_filters_apply() {
        let tmp = TempDir::new().unwrap();
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();
        sub.put(sig(Kind::Task, "a", 100)).await.unwrap();
        sub.put(sig(Kind::Task, "b", 200)).await.unwrap();
        sub.put(sig(Kind::Episode, "e1", 150)).await.unwrap();

        let ctx = Context::at(500);
        let tasks = sub.query(&Query::of_kind(Kind::Task), &ctx).await.unwrap();
        assert_eq!(tasks.len(), 2);

        let in_range = sub
            .query(&Query::all().since(140).until(210), &ctx)
            .await
            .unwrap();
        assert_eq!(in_range.len(), 2); // b (200) + e1 (150)
    }

    #[tokio::test]
    async fn prune_respects_decay() {
        let tmp = TempDir::new().unwrap();
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();

        sub.put(
            Engram::builder(Kind::Pheromone)
                .body(Body::text("transient"))
                .score(Score::new(1.0, 0.0, 0.0, 1.0))
                .decay(Decay::HalfLife { half_life_ms: 100 })
                .created_at_ms(0)
                .build(),
        )
        .await
        .unwrap();
        sub.put(
            Engram::builder(Kind::Task)
                .body(Body::text("eternal"))
                .score(Score::new(1.0, 0.0, 0.0, 1.0))
                .decay(Decay::None)
                .created_at_ms(0)
                .build(),
        )
        .await
        .unwrap();

        let ctx = Context::at(10_000);
        let pruned = sub.prune(0.01, &ctx).await.unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(sub.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn compact_shrinks_log_file() {
        let tmp = TempDir::new().unwrap();
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();

        // Write 5 signals with different content.
        for i in 0..5 {
            sub.put(sig(Kind::Pheromone, &format!("p{i}"), 0))
                .await
                .unwrap();
        }
        let size_before = fs::metadata(sub.log_path()).await.unwrap().len();

        // Prune all of them (threshold bigger than any signal's weight).
        let ctx = Context::at(0);
        let pruned = sub.prune(1_000_000.0, &ctx).await.unwrap();
        assert_eq!(pruned, 5);
        assert_eq!(sub.len().await.unwrap(), 0);

        // Before compaction, the log still has all the old entries.
        let size_mid = fs::metadata(sub.log_path()).await.unwrap().len();
        assert_eq!(size_mid, size_before);

        // Compact: log becomes empty (no signals remaining).
        sub.compact().await.unwrap();
        let size_after = fs::metadata(sub.log_path()).await.unwrap().len();
        assert!(size_after < size_before);
    }

    #[tokio::test]
    async fn compact_preserves_live_signals() {
        let tmp = TempDir::new().unwrap();
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();

        sub.put(sig(Kind::Task, "keep_me", 0)).await.unwrap();
        sub.compact().await.unwrap();

        // Still queryable after compaction.
        assert_eq!(sub.len().await.unwrap(), 1);

        // Still queryable after restart.
        drop(sub);
        let sub2 = FileSubstrate::open(tmp.path()).await.unwrap();
        assert_eq!(sub2.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn replay_skips_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        // Write a valid signal first.
        {
            let sub = FileSubstrate::open(tmp.path()).await.unwrap();
            sub.put(sig(Kind::Task, "valid", 0)).await.unwrap();
        }
        // Corrupt the log: append a partial / bad line.
        {
            let log = tmp.path().join("engrams.jsonl");
            let mut f = OpenOptions::new().append(true).open(&log).await.unwrap();
            f.write_all(b"{partial_bad_json\n").await.unwrap();
            f.flush().await.unwrap();
        }
        // Should open successfully, skip bad line, retain valid entry.
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();
        assert_eq!(sub.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn concurrent_puts_all_persist() {
        use std::sync::Arc;
        let tmp = TempDir::new().unwrap();
        let sub = Arc::new(FileSubstrate::open(tmp.path()).await.unwrap());

        let mut handles = vec![];
        for i in 0..20 {
            let sub = sub.clone();
            handles.push(tokio::spawn(async move {
                sub.put(sig(Kind::Task, &format!("concurrent_{i}"), i))
                    .await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }
        assert_eq!(sub.len().await.unwrap(), 20);
    }

    #[tokio::test]
    async fn hdc_fingerprint_is_attached_on_put() {
        let tmp = TempDir::new().unwrap();
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();
        let s = sig(Kind::Task, "fingerprint me", 0);
        assert!(s.tags.get(HDC_TAG).is_none());
        let id = sub.put(s).await.unwrap();
        let stored = sub.get(&id).await.unwrap().expect("signal must exist");
        // When the hdc feature is enabled the tag is present; otherwise
        // the signal is stored unmodified.
        #[cfg(feature = "hdc")]
        {
            assert!(stored.tags.contains_key(HDC_TAG));
            // Fingerprint is a base64-encoded 1280-byte vector.
            let encoded = stored.tags.get(HDC_TAG).unwrap();
            let decoded =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                    .expect("valid base64");
            assert_eq!(decoded.len(), 1280);
        }
        #[cfg(not(feature = "hdc"))]
        {
            assert!(!stored.tags.contains_key(HDC_TAG));
        }
    }

    #[tokio::test]
    async fn hdc_fingerprint_not_overwritten_if_present() {
        let tmp = TempDir::new().unwrap();
        let sub = FileSubstrate::open(tmp.path()).await.unwrap();
        let mut s = sig(Kind::Task, "already tagged", 0);
        s.tags.insert(HDC_TAG.into(), "pre-existing".into());
        s.id = s.content_hash();
        let id = sub.put(s).await.unwrap();
        let stored = sub.get(&id).await.unwrap().expect("signal must exist");
        assert_eq!(stored.tags.get(HDC_TAG).unwrap(), "pre-existing");
    }
}
