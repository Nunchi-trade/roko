//! Convenience wiring for filesystem-backed observability sinks.
//!
//! Runtime callers usually need both a persistent trace sink and a persistent
//! tool-metrics sink. [`FsObservabilitySinks`] constructs both from either a
//! workspace root or an existing `.roko/` directory and exposes typed and
//! trait-object handles.

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
}
