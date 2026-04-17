//! Append-only structured metric recording (JSONL).
//!
//! Provides the Phase 0 instrumentation primitives from the refactor plan.
//! All metrics are written as single-line JSON to a JSONL file, flushed
//! after every write for crash safety.
//!
//! # Design
//!
//! - Generic over any `M: Serialize` metric type.
//! - Thread-safe: uses `parking_lot::Mutex` for the file handle.
//! - Crash-safe: every `record()` call flushes to disk.
//! - Zero dependencies on mori domain types.

use parking_lot::Mutex;
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde::Serialize;
use tracing::{debug, error};

/// Errors from metric recording.
#[derive(Debug, thiserror::Error)]
pub enum MetricError {
    /// An I/O error occurred while writing metrics.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A serialization error occurred while encoding a metric as JSON.
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// A timestamped metric envelope, written as a single JSONL line.
#[derive(Debug, Serialize)]
struct MetricLine<'a, M> {
    /// ISO 8601 timestamp.
    ts: String,
    /// The metric payload.
    #[serde(flatten)]
    metric: &'a M,
}

/// Append-only JSONL metric recorder.
///
/// Thread-safe (uses interior `Mutex`). Each `record()` call serialises the
/// metric as a single JSON line and flushes immediately.
pub struct MetricRecorder {
    path: PathBuf,
    writer: Mutex<BufWriter<File>>,
}

impl MetricRecorder {
    /// Open (or create) a JSONL metric file at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MetricError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        debug!(path = %path.display(), "opened metric recorder");
        Ok(Self {
            path,
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    /// Record a single metric. Serialises as JSON, appends newline, flushes.
    pub fn record<M: Serialize>(&self, metric: &M) -> Result<(), MetricError> {
        let line = MetricLine {
            ts: Utc::now().to_rfc3339(),
            metric,
        };
        let json = serde_json::to_string(&line)?;

        let mut writer = self.writer.lock();
        writeln!(writer, "{json}")?;
        writer.flush()?;
        drop(writer);
        Ok(())
    }

    /// The path this recorder writes to.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for MetricRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricRecorder")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Convenience: record a metric, logging errors instead of propagating them.
///
/// Use this in hot paths where metric recording failure should not crash the system.
pub fn record_or_log<M: Serialize>(recorder: &MetricRecorder, metric: &M) {
    if let Err(e) = recorder.record(metric) {
        error!(error = %e, path = %recorder.path().display(), "failed to record metric");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestMetric {
        plan_id: String,
        cost_usd: f64,
        gate_passed: bool,
    }

    #[test]
    fn record_and_read_back() {
        let dir = std::env::temp_dir().join(format!("roko-runtime-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .expect("invariant: metric test should be able to create its temp directory");

        let path = dir.join("metrics.jsonl");
        let recorder = MetricRecorder::open(&path)
            .expect("invariant: metric recorder should open within the temp directory");

        recorder
            .record(&TestMetric {
                plan_id: "plan-01".into(),
                cost_usd: 1.23,
                gate_passed: true,
            })
            .expect("invariant: first metric should serialize and append");
        recorder
            .record(&TestMetric {
                plan_id: "plan-02".into(),
                cost_usd: 0.45,
                gate_passed: false,
            })
            .expect("invariant: second metric should serialize and append");

        // Read back.
        let contents = std::fs::read_to_string(&path)
            .expect("invariant: metric test should be able to read back its JSONL file");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);

        // Parse each line.
        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line)
                .expect("invariant: recorded metric lines should contain valid JSON");
            assert!(v.get("ts").is_some());
            assert!(v.get("plan_id").is_some());
        }

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
