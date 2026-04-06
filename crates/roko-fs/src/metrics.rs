//! Append-only JSONL writer for [`TaskMetric`] records.
//!
//! Every gate verdict produces one record; records accumulate into
//! `.roko/runs/{run_id}/metrics.jsonl`. Records are consumed by the five
//! continuous-tuning loops in `tmp/roko-progress/roko-continuous-tuning.md`.
//!
//! # Why JSONL + append-only
//!
//! - **Crash-safe**: if the process dies mid-write, worst case is a partial
//!   last line which [`MetricsLog::read_all`] skips.
//! - **Immutable**: records are never rewritten; bad data is filtered at
//!   read time via `config_hash`, not mutated.
//! - **Streamable**: downstream Python tuners can `tail -f` the file.
//! - **Consistent** with [`FileSubstrate`](crate::FileSubstrate).
//!
//! Writes are **fsync-on-append by default** so a sudden process death
//! loses at most one in-flight record. Callers that need higher throughput
//! can construct with [`MetricsLog::without_fsync`].

use roko_core::metric::TaskMetric;
use std::io;
use std::path::{Path, PathBuf};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Append-only writer for `TaskMetric` records.
///
/// Cheap to construct: no file handles are held open. Each `append` call
/// opens → writes → syncs → closes. This mirrors `FileSubstrate`'s design
/// for robustness at the cost of some throughput — acceptable because
/// metric records are produced at most a few per second per plan.
#[derive(Debug, Clone)]
pub struct MetricsLog {
    path: PathBuf,
    fsync: bool,
}

impl MetricsLog {
    /// Create a log pointed at `path`. The file is created on first
    /// `append` if it doesn't exist. Parent directories must already
    /// exist — use [`MetricsLog::open_creating`] to create them.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), fsync: true }
    }

    /// Like [`at`], but creates parent directories as a side-effect.
    ///
    /// Convenient for `.roko/runs/{run_id}/metrics.jsonl` where the run
    /// dir may not exist yet.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent directory can't be created.
    pub async fn open_creating(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Ok(Self { path, fsync: true })
    }

    /// Path to the underlying JSONL file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Disable fsync-on-append for higher throughput. Use only when you
    /// can tolerate losing the last batch of records on process death.
    #[must_use]
    pub const fn without_fsync(mut self) -> Self {
        self.fsync = false;
        self
    }

    /// Append one record. Creates the file if missing.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails, the file can't be opened,
    /// or the write fails.
    pub async fn append(&self, record: &TaskMetric) -> io::Result<()> {
        let mut line = record
            .to_jsonl()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        if self.fsync {
            file.sync_data().await?;
        }
        Ok(())
    }

    /// Append many records in one file open. Faster than looping `append`.
    ///
    /// # Errors
    ///
    /// Same as [`append`]. On error, *some* records may have been written
    /// — the caller should treat this as a partial-write situation.
    pub async fn append_all(&self, records: &[TaskMetric]) -> io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut buf = String::new();
        for r in records {
            let line = r
                .to_jsonl()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            buf.push_str(&line);
            buf.push('\n');
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(buf.as_bytes()).await?;
        if self.fsync {
            file.sync_data().await?;
        }
        Ok(())
    }

    /// Read all records from the log. Malformed lines are skipped
    /// (logged via `tracing::warn` when a `tracing` feature is on —
    /// currently silent to avoid a dependency). This makes the reader
    /// robust to partial writes from prior crashes.
    ///
    /// # Errors
    ///
    /// Returns an error only on file-open/read failure. A file that
    /// doesn't exist returns `Ok(vec![])`.
    pub async fn read_all(&self) -> io::Result<Vec<TaskMetric>> {
        let file = match tokio::fs::File::open(&self.path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut out = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(m) = TaskMetric::from_jsonl(&line) {
                out.push(m);
            }
            // else: Partial or corrupt line — skip. This is the
            // crash-recovery contract: worst case we lose the
            // last in-flight record.
        }
        Ok(out)
    }

    /// Read records and filter to one `config_hash`. Convenience for
    /// the per-config analysis every tuning loop performs.
    ///
    /// # Errors
    ///
    /// Same as [`read_all`].
    pub async fn read_for_config(&self, config_hash: &str) -> io::Result<Vec<TaskMetric>> {
        let all = self.read_all().await?;
        Ok(all
            .into_iter()
            .filter(|r| r.config_hash.as_str() == config_hash)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::metric::ConfigHash;
    use tempfile::TempDir;

    fn make_record(hash: &str, plan: &str, task: &str, passed: bool) -> TaskMetric {
        let mut m = TaskMetric::new(ConfigHash::from(hash.to_string()), plan, task);
        m.iteration = 1;
        m.gate_passed = passed;
        m.cost_usd = 0.05;
        m.input_tokens = 1000;
        m
    }

    #[tokio::test]
    async fn append_and_read_single_record() {
        let tmp = TempDir::new().unwrap();
        let log = MetricsLog::at(tmp.path().join("metrics.jsonl"));

        let r = make_record("h1", "p1", "t1", true);
        log.append(&r).await.unwrap();

        let records = log.read_all().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0], r);
    }

    #[tokio::test]
    async fn append_all_batches_records() {
        let tmp = TempDir::new().unwrap();
        let log = MetricsLog::at(tmp.path().join("m.jsonl"));
        let batch = vec![
            make_record("h", "p1", "t1", true),
            make_record("h", "p1", "t2", false),
            make_record("h", "p2", "t1", true),
        ];
        log.append_all(&batch).await.unwrap();
        let records = log.read_all().await.unwrap();
        assert_eq!(records.len(), 3);
    }

    #[tokio::test]
    async fn read_all_returns_empty_for_missing_file() {
        let tmp = TempDir::new().unwrap();
        let log = MetricsLog::at(tmp.path().join("never-created.jsonl"));
        let records = log.read_all().await.unwrap();
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn open_creating_makes_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("a").join("b").join("c").join("m.jsonl");
        let log = MetricsLog::open_creating(&nested).await.unwrap();
        log.append(&make_record("h", "p", "t", true)).await.unwrap();
        assert!(nested.exists());
    }

    #[tokio::test]
    async fn read_skips_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("m.jsonl");
        let log = MetricsLog::at(&path);
        log.append(&make_record("h", "p1", "t1", true)).await.unwrap();

        // Corrupt the file: append a partial line at the end.
        tokio::fs::write(
            &path,
            format!("{}{{\"timestamp\"", tokio::fs::read_to_string(&path).await.unwrap()),
        )
        .await
        .unwrap();

        // Reading should yield the one good record and skip the partial.
        let records = log.read_all().await.unwrap();
        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn read_for_config_filters_by_hash() {
        let tmp = TempDir::new().unwrap();
        let log = MetricsLog::at(tmp.path().join("m.jsonl"));
        log.append_all(&[
            make_record("h1", "p1", "t1", true),
            make_record("h2", "p1", "t2", true),
            make_record("h1", "p2", "t1", false),
        ])
        .await
        .unwrap();

        let subset = log.read_for_config("h1").await.unwrap();
        assert_eq!(subset.len(), 2);
        assert!(subset.iter().all(|r| r.config_hash.as_str() == "h1"));
    }

    #[tokio::test]
    async fn appends_accumulate_across_calls() {
        let tmp = TempDir::new().unwrap();
        let log = MetricsLog::at(tmp.path().join("m.jsonl"));
        for i in 0..5 {
            let mut r = make_record("h", "p1", &format!("t{i}"), true);
            r.iteration = i + 1;
            log.append(&r).await.unwrap();
        }
        let records = log.read_all().await.unwrap();
        assert_eq!(records.len(), 5);
        assert_eq!(records[0].iteration, 1);
        assert_eq!(records[4].iteration, 5);
    }

    #[tokio::test]
    async fn without_fsync_still_writes() {
        let tmp = TempDir::new().unwrap();
        let log = MetricsLog::at(tmp.path().join("m.jsonl")).without_fsync();
        log.append(&make_record("h", "p", "t", true)).await.unwrap();
        let records = log.read_all().await.unwrap();
        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn empty_batch_is_noop() {
        let tmp = TempDir::new().unwrap();
        let log = MetricsLog::at(tmp.path().join("m.jsonl"));
        log.append_all(&[]).await.unwrap();
        // File should not have been created.
        assert!(!tmp.path().join("m.jsonl").exists());
    }
}
