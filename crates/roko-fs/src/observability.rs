//! Convenience wiring for filesystem-backed observability sinks.
//!
//! Runtime callers usually need both a persistent trace sink and a persistent
//! tool-metrics sink. [`FsObservabilitySinks`] constructs both from either a
//! workspace root or an existing `.roko/` directory and exposes typed and
//! trait-object handles.

use std::io;
use std::path::Path;
use std::sync::Arc;

use roko_core::tool::{MetricsSink, TraceSink};

use crate::{JsonlMetricsSink, JsonlTraceSink};

/// Paired filesystem-backed sinks used by runtime dispatch code.
#[derive(Debug, Clone)]
pub struct FsObservabilitySinks {
    /// Persistent JSONL trace sink.
    pub trace_sink: Arc<JsonlTraceSink>,
    /// Persistent JSONL metrics sink.
    pub metrics_sink: Arc<JsonlMetricsSink>,
}

impl FsObservabilitySinks {
    /// Build sinks rooted at a workspace directory.
    ///
    /// - traces: `<workdir>/.roko/traces/`
    /// - tool metrics: `<workdir>/.roko/metrics/tool_metrics.jsonl`
    #[must_use]
    pub fn for_workdir(workdir: impl AsRef<Path>) -> Self {
        let trace_sink = Arc::new(JsonlTraceSink::for_workdir(workdir.as_ref()));
        let metrics_sink = Arc::new(JsonlMetricsSink::for_workdir(workdir.as_ref()));
        Self {
            trace_sink,
            metrics_sink,
        }
    }

    /// Build sinks rooted at an existing `.roko/` directory.
    ///
    /// - traces: `<roko_dir>/traces/`
    /// - tool metrics: `<roko_dir>/metrics/tool_metrics.jsonl`
    #[must_use]
    pub fn for_roko_dir(roko_dir: impl AsRef<Path>) -> Self {
        let trace_sink = Arc::new(JsonlTraceSink::for_roko_dir(roko_dir.as_ref()));
        let metrics_sink = Arc::new(JsonlMetricsSink::for_roko_dir(roko_dir.as_ref()));
        Self {
            trace_sink,
            metrics_sink,
        }
    }

    /// Build sinks rooted at a workspace directory and create their
    /// backing directories immediately.
    ///
    /// This is idempotent: calling it repeatedly only re-validates that the
    /// directory structure exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the trace directory or metrics parent directory
    /// cannot be created.
    pub fn initialized_for_workdir(workdir: impl AsRef<Path>) -> io::Result<Self> {
        let sinks = Self::for_workdir(workdir);
        sinks.initialize()?;
        Ok(sinks)
    }

    /// Build sinks rooted at an existing `.roko/` directory and create their
    /// backing directories immediately.
    ///
    /// This is idempotent: calling it repeatedly only re-validates that the
    /// directory structure exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the trace directory or metrics parent directory
    /// cannot be created.
    pub fn initialized_for_roko_dir(roko_dir: impl AsRef<Path>) -> io::Result<Self> {
        let sinks = Self::for_roko_dir(roko_dir);
        sinks.initialize()?;
        Ok(sinks)
    }

    /// Create the backing directories for both sinks.
    ///
    /// This makes startup initialization explicit for callers that want to
    /// prepare observability before the first trace or metrics write.
    ///
    /// The operation is idempotent: existing directories are left untouched.
    ///
    /// # Errors
    ///
    /// Returns an error if the trace directory or metrics parent directory
    /// cannot be created.
    pub fn initialize(&self) -> io::Result<()> {
        std::fs::create_dir_all(self.trace_sink.root())?;
        if let Some(parent) = self.metrics_sink.path().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    /// Clone as a dynamic trace sink trait object.
    #[must_use]
    pub fn trace_sink_dyn(&self) -> Arc<dyn TraceSink> {
        self.trace_sink.clone()
    }

    /// Clone as a dynamic metrics sink trait object.
    #[must_use]
    pub fn metrics_sink_dyn(&self) -> Arc<dyn MetricsSink> {
        self.metrics_sink.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_workdir_builds_expected_paths() {
        let sinks = FsObservabilitySinks::for_workdir("/repo");
        assert_eq!(sinks.trace_sink.root(), Path::new("/repo/.roko/traces"),);
        assert_eq!(
            sinks.metrics_sink.path(),
            Path::new("/repo/.roko/metrics/tool_metrics.jsonl"),
        );
    }

    #[test]
    fn dyn_accessors_return_trait_objects() {
        let sinks = FsObservabilitySinks::for_roko_dir("/repo/.roko");
        let _trace: Arc<dyn TraceSink> = sinks.trace_sink_dyn();
        let _metrics: Arc<dyn MetricsSink> = sinks.metrics_sink_dyn();
    }

    #[test]
    fn initialize_creates_trace_and_metrics_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sinks = FsObservabilitySinks::for_workdir(tmp.path());

        sinks.initialize().expect("initialize observability");

        assert!(tmp.path().join(".roko").join("traces").is_dir());
        assert!(tmp.path().join(".roko").join("metrics").is_dir());
    }

    #[test]
    fn initialized_for_roko_dir_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let roko_dir = tmp.path().join(".roko");

        let sinks = FsObservabilitySinks::initialized_for_roko_dir(&roko_dir)
            .expect("initialize from roko dir");
        sinks.initialize().expect("reinitialize");

        assert!(roko_dir.join("traces").is_dir());
        assert!(roko_dir.join("metrics").is_dir());
    }
}
