//! Append-only JSONL persistence for [`crate::costs_db::CostRecord`].
//!
//! `CostsDb` is intentionally in-memory. This module provides the durable,
//! file-backed companion used by runtime wiring: append each completed call as
//! one JSON line, then reload on process start.

use std::io;
use std::path::{Path, PathBuf};

use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::costs_db::CostRecord;

/// Append-only JSONL log for [`CostRecord`] values.
#[derive(Debug, Clone)]
pub struct CostsLog {
    path: PathBuf,
    fsync: bool,
}

impl CostsLog {
    /// Construct a log at `path`.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            fsync: true,
        }
    }

    /// Create parent directories and return a log at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when parent directories cannot be created.
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

    /// Disable fsync after appends.
    #[must_use]
    pub const fn without_fsync(mut self) -> Self {
        self.fsync = false;
        self
    }

    /// Append one [`CostRecord`] as one JSON line.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization or file I/O failures.
    pub async fn append(&self, record: &CostRecord) -> io::Result<()> {
        let mut line = serde_json::to_string(record)
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

    /// Append many records with one open/close cycle.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization or file I/O failures.
    pub async fn append_all(&self, records: &[CostRecord]) -> io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut buf = String::new();
        for record in records {
            let line = serde_json::to_string(record)
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

    /// Read all valid records; malformed lines are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error only for file open/read failures.
    pub async fn read_all(&self) -> io::Result<Vec<CostRecord>> {
        let file = match tokio::fs::File::open(&self.path).await {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut out = Vec::new();
        while let Some(line) = lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<CostRecord>(trimmed) {
                out.push(record);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn record(task: &str, cost: f64) -> CostRecord {
        CostRecord {
            timestamp: "2026-04-08T00:00:00Z".to_string(),
            model: "claude-opus-4-6".to_string(),
            provider: "anthropic".to_string(),
            role: "Implementer".to_string(),
            plan_id: "plan-1".to_string(),
            task_id: task.to_string(),
            complexity_band: "standard".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: 0,
            cost_usd: cost,
            duration_ms: 1234,
            success: true,
            session_id: "sess-1".to_string(),
        }
    }

    #[tokio::test]
    async fn append_and_read_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("costs.jsonl");
        let log = CostsLog::at(&path);
        let r1 = record("t1", 0.1);
        let r2 = record("t2", 0.2);

        log.append(&r1).await.unwrap();
        log.append(&r2).await.unwrap();

        let all = log.read_all().await.unwrap();
        assert_eq!(all, vec![r1, r2]);
    }

    #[tokio::test]
    async fn append_all_writes_batch() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("costs.jsonl");
        let log = CostsLog::at(&path);
        let batch = vec![record("t1", 0.1), record("t2", 0.2), record("t3", 0.3)];
        log.append_all(&batch).await.unwrap();
        let all = log.read_all().await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn read_all_skips_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("costs.jsonl");
        tokio::fs::write(
            &path,
            format!(
                "{}\n{}\n{}\n",
                serde_json::to_string(&record("ok-1", 0.1)).unwrap(),
                "{ malformed json",
                serde_json::to_string(&record("ok-2", 0.2)).unwrap()
            ),
        )
        .await
        .unwrap();
        let log = CostsLog::at(&path);
        let all = log.read_all().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].task_id, "ok-1");
        assert_eq!(all[1].task_id, "ok-2");
    }
}
