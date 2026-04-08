//! Persistent JSONL sink for tool-call aggregate metrics.
//!
//! This module provides [`JsonlMetricsSink`], a concrete implementation of
//! [`roko_core::tool::MetricsSink`] that writes one JSON line per metrics
//! snapshot. It is intentionally best-effort: write failures are reported to
//! stderr and do not panic the agent runtime.
//!
//! # Default location
//!
//! For a workspace root, use [`JsonlMetricsSink::for_workdir`] to write to:
//!
//! ```text
//! <workdir>/.roko/metrics/tool_metrics.jsonl
//! ```

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use roko_core::tool::{MetricsKey, MetricsSink, ToolMetrics};
use serde::{Deserialize, Serialize};

/// Relative path of the default tool-metrics log from a workspace root.
pub const DEFAULT_TOOL_METRICS_REL_PATH: &str = ".roko/metrics/tool_metrics.jsonl";

/// One persisted metrics snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolMetricsRecord {
    /// Unix-millis timestamp when this record was emitted.
    pub at_ms: i64,
    /// Aggregation key (`tool × model × role × format`).
    pub key: MetricsKey,
    /// Aggregated metrics values after this update.
    pub metrics: ToolMetrics,
}

impl ToolMetricsRecord {
    /// Build a new metrics record stamped with the current UTC time.
    #[must_use]
    pub fn now(key: MetricsKey, metrics: ToolMetrics) -> Self {
        Self {
            at_ms: chrono::Utc::now().timestamp_millis(),
            key,
            metrics,
        }
    }
}

/// JSONL-backed [`MetricsSink`] for persistent tool metrics.
///
/// Writes are synchronized with an in-process mutex to keep concurrent
/// `record(...)` calls from interleaving bytes.
#[derive(Debug)]
pub struct JsonlMetricsSink {
    path: PathBuf,
    fsync: bool,
    write_lock: Mutex<()>,
}

impl JsonlMetricsSink {
    /// Construct a sink for an explicit JSONL file path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            fsync: true,
            write_lock: Mutex::new(()),
        }
    }

    /// Construct a sink rooted at a workspace directory.
    ///
    /// Writes to `<workdir>/.roko/metrics/tool_metrics.jsonl`.
    #[must_use]
    pub fn for_workdir(workdir: impl AsRef<Path>) -> Self {
        Self::new(workdir.as_ref().join(DEFAULT_TOOL_METRICS_REL_PATH))
    }

    /// Construct a sink rooted at an existing `.roko/` directory.
    ///
    /// Writes to `<roko_dir>/metrics/tool_metrics.jsonl`.
    #[must_use]
    pub fn for_roko_dir(roko_dir: impl AsRef<Path>) -> Self {
        Self::new(roko_dir.as_ref().join("metrics").join("tool_metrics.jsonl"))
    }

    /// Disable fsync-on-write for higher throughput.
    #[must_use]
    pub const fn without_fsync(mut self) -> Self {
        self.fsync = false;
        self
    }

    /// Path to the underlying JSONL file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if parent directories cannot be created, the file
    /// cannot be opened, the record cannot be serialized, or the write fails.
    pub fn append(&self, record: &ToolMetricsRecord) -> io::Result<()> {
        let _guard = self.write_lock.lock();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(record)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        if self.fsync {
            file.sync_data()?;
        }
        Ok(())
    }

    /// Read all parseable records from disk.
    ///
    /// Malformed lines are skipped so partial writes from abrupt process
    /// termination do not poison the entire log.
    ///
    /// # Errors
    ///
    /// Returns an error on file-open/read failures. If the file does not
    /// exist, returns an empty vector.
    pub fn read_all(&self) -> io::Result<Vec<ToolMetricsRecord>> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let reader = std::io::BufReader::new(file);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<ToolMetricsRecord>(trimmed) {
                out.push(record);
            }
        }
        Ok(out)
    }
}

impl MetricsSink for JsonlMetricsSink {
    fn record(&self, key: &MetricsKey, metrics: &ToolMetrics) {
        let record = ToolMetricsRecord::now(key.clone(), *metrics);
        if let Err(err) = self.append(&record) {
            eprintln!(
                "JsonlMetricsSink: failed to append {}: {err}",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::AgentRole;
    use roko_core::tool::ToolFormat;

    fn key() -> MetricsKey {
        MetricsKey::new(
            "read_file",
            "claude-opus-4-6",
            AgentRole::Implementer,
            ToolFormat::AnthropicBlocks,
        )
    }

    fn metrics() -> ToolMetrics {
        let mut metrics = ToolMetrics::empty();
        metrics.observe(false, false, true, true, true, 0.9);
        metrics
    }

    #[test]
    fn appends_and_reads_records() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sink = JsonlMetricsSink::new(tmp.path().join("tool_metrics.jsonl"));
        let record = ToolMetricsRecord::now(key(), metrics());
        sink.append(&record).expect("append");

        let rows = sink.read_all().expect("read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, record.key);
        assert_eq!(rows[0].metrics, record.metrics);
    }

    #[test]
    fn trait_record_writes_line() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sink: Box<dyn MetricsSink> =
            Box::new(JsonlMetricsSink::new(tmp.path().join("tool_metrics.jsonl")));
        sink.record(&key(), &metrics());

        let concrete = JsonlMetricsSink::new(tmp.path().join("tool_metrics.jsonl"));
        let rows = concrete.read_all().expect("read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key.tool, "read_file");
    }

    #[test]
    fn skips_malformed_lines() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("tool_metrics.jsonl");
        std::fs::write(&path, "{bad json}\n").expect("write malformed");
        let sink = JsonlMetricsSink::new(&path);
        sink.record(&key(), &metrics());

        let rows = sink.read_all().expect("read");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn for_workdir_uses_default_path() {
        let sink = JsonlMetricsSink::for_workdir("/repo");
        assert_eq!(
            sink.path(),
            Path::new("/repo/.roko/metrics/tool_metrics.jsonl")
        );
    }

    #[test]
    fn for_roko_dir_uses_metrics_subdir() {
        let sink = JsonlMetricsSink::for_roko_dir("/repo/.roko");
        assert_eq!(
            sink.path(),
            Path::new("/repo/.roko/metrics/tool_metrics.jsonl")
        );
    }
}
