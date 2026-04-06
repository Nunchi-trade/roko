//! Per-tool-call metric emission (checklist items 36.44, 36.63).
//!
//! Every tool invocation produces a [`ToolCallMetric`] record capturing
//! wall-clock duration, success/failure, and token counts. These records
//! feed both the live failure monitor ([`super::alert`]) and the
//! persistent metric sinks.

use serde::{Deserialize, Serialize};

// ─── ToolCallMetric ──────────────────────────────────────────────────────

/// A single tool-call measurement emitted after each invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallMetric {
    /// Canonical tool name (e.g. `"read_file"`).
    pub tool_name: String,
    /// Wall-clock duration of the call in milliseconds.
    pub duration_ms: u64,
    /// Whether the call succeeded.
    pub success: bool,
    /// Input tokens consumed (0 if not tracked).
    pub input_tokens: u64,
    /// Output tokens produced (0 if not tracked).
    pub output_tokens: u64,
    /// Timestamp of the call in milliseconds since epoch.
    pub timestamp_ms: i64,
}

// ─── ToolCallSummary ─────────────────────────────────────────────────────

/// Aggregated summary over a batch of [`ToolCallMetric`] records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallSummary {
    /// Total number of calls.
    pub total_calls: u64,
    /// Number of successful calls.
    pub success_count: u64,
    /// Number of failed calls.
    pub failure_count: u64,
    /// Mean duration across all calls (ms).
    pub mean_duration_ms: f64,
    /// Maximum duration observed (ms).
    pub max_duration_ms: u64,
    /// Total input tokens across all calls.
    pub total_input_tokens: u64,
    /// Total output tokens across all calls.
    pub total_output_tokens: u64,
}

// ─── ToolMetricEmitter ───────────────────────────────────────────────────

/// Records per-tool-call metrics and emits [`ToolCallMetric`] records.
///
/// Callers invoke [`Self::emit`] after each tool call; the emitter
/// constructs a timestamped metric record. Batch aggregation is
/// available via the free function [`aggregate`].
#[derive(Debug, Clone, Default)]
pub struct ToolMetricEmitter {
    /// Rolling history of emitted metrics.
    history: Vec<ToolCallMetric>,
}

impl ToolMetricEmitter {
    /// Create a new emitter with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            history: Vec::new(),
        }
    }

    /// Record a tool call and return the metric record.
    pub fn emit(
        &mut self,
        tool_name: &str,
        duration_ms: u64,
        success: bool,
        input_tokens: u64,
        output_tokens: u64,
    ) -> ToolCallMetric {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let metric = ToolCallMetric {
            tool_name: tool_name.to_owned(),
            duration_ms,
            success,
            input_tokens,
            output_tokens,
            timestamp_ms: now_ms,
        };
        self.history.push(metric.clone());
        metric
    }

    /// Return a read-only view of all recorded metrics.
    #[must_use]
    pub fn history(&self) -> &[ToolCallMetric] {
        &self.history
    }

    /// Clear the recorded history.
    pub fn clear(&mut self) {
        self.history.clear();
    }
}

// ─── aggregate ───────────────────────────────────────────────────────────

/// Compute an aggregated summary over a slice of [`ToolCallMetric`] records.
///
/// Returns `None` if the slice is empty.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn aggregate(metrics: &[ToolCallMetric]) -> Option<ToolCallSummary> {
    if metrics.is_empty() {
        return None;
    }
    let total_calls = metrics.len() as u64;
    let success_count = metrics.iter().filter(|m| m.success).count() as u64;
    let failure_count = total_calls - success_count;
    let sum_duration: u64 = metrics.iter().map(|m| m.duration_ms).sum();
    let max_duration_ms = metrics.iter().map(|m| m.duration_ms).max().unwrap_or(0);
    let mean_duration_ms = sum_duration as f64 / total_calls as f64;
    let total_input_tokens: u64 = metrics.iter().map(|m| m.input_tokens).sum();
    let total_output_tokens: u64 = metrics.iter().map(|m| m.output_tokens).sum();
    Some(ToolCallSummary {
        total_calls,
        success_count,
        failure_count,
        mean_duration_ms,
        max_duration_ms,
        total_input_tokens,
        total_output_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_produces_valid_metric() {
        let mut emitter = ToolMetricEmitter::new();
        let m = emitter.emit("read_file", 150, true, 100, 200);
        assert_eq!(m.tool_name, "read_file");
        assert_eq!(m.duration_ms, 150);
        assert!(m.success);
        assert_eq!(m.input_tokens, 100);
        assert_eq!(m.output_tokens, 200);
        assert!(m.timestamp_ms > 0);
    }

    #[test]
    fn emit_history_accumulates() {
        let mut emitter = ToolMetricEmitter::new();
        emitter.emit("read_file", 100, true, 10, 20);
        emitter.emit("bash", 200, false, 30, 40);
        emitter.emit("grep", 50, true, 5, 10);
        assert_eq!(emitter.history().len(), 3);
        assert_eq!(emitter.history()[0].tool_name, "read_file");
        assert_eq!(emitter.history()[1].tool_name, "bash");
        assert_eq!(emitter.history()[2].tool_name, "grep");
    }

    #[test]
    fn emit_clear_resets_history() {
        let mut emitter = ToolMetricEmitter::new();
        emitter.emit("read_file", 100, true, 10, 20);
        assert_eq!(emitter.history().len(), 1);
        emitter.clear();
        assert!(emitter.history().is_empty());
    }

    #[test]
    fn emit_failure_records_correctly() {
        let mut emitter = ToolMetricEmitter::new();
        let m = emitter.emit("bash", 5000, false, 0, 0);
        assert!(!m.success);
        assert_eq!(m.duration_ms, 5000);
    }

    #[test]
    fn emit_timestamps_are_monotonic() {
        let mut emitter = ToolMetricEmitter::new();
        let m1 = emitter.emit("a", 1, true, 0, 0);
        let m2 = emitter.emit("b", 1, true, 0, 0);
        assert!(m2.timestamp_ms >= m1.timestamp_ms);
    }

    #[test]
    fn aggregate_empty_returns_none() {
        assert!(aggregate(&[]).is_none());
    }

    #[test]
    fn aggregate_single_metric() {
        let metrics = vec![ToolCallMetric {
            tool_name: "read_file".into(),
            duration_ms: 100,
            success: true,
            input_tokens: 50,
            output_tokens: 200,
            timestamp_ms: 1_000_000,
        }];
        let summary = aggregate(&metrics).unwrap();
        assert_eq!(summary.total_calls, 1);
        assert_eq!(summary.success_count, 1);
        assert_eq!(summary.failure_count, 0);
        assert!((summary.mean_duration_ms - 100.0).abs() < f64::EPSILON);
        assert_eq!(summary.max_duration_ms, 100);
        assert_eq!(summary.total_input_tokens, 50);
        assert_eq!(summary.total_output_tokens, 200);
    }

    #[test]
    fn aggregate_mixed_success_and_failure() {
        let metrics = vec![
            ToolCallMetric {
                tool_name: "bash".into(),
                duration_ms: 200,
                success: true,
                input_tokens: 10,
                output_tokens: 20,
                timestamp_ms: 1_000,
            },
            ToolCallMetric {
                tool_name: "bash".into(),
                duration_ms: 400,
                success: false,
                input_tokens: 30,
                output_tokens: 40,
                timestamp_ms: 2_000,
            },
            ToolCallMetric {
                tool_name: "bash".into(),
                duration_ms: 300,
                success: true,
                input_tokens: 20,
                output_tokens: 30,
                timestamp_ms: 3_000,
            },
        ];
        let summary = aggregate(&metrics).unwrap();
        assert_eq!(summary.total_calls, 3);
        assert_eq!(summary.success_count, 2);
        assert_eq!(summary.failure_count, 1);
        assert!((summary.mean_duration_ms - 300.0).abs() < f64::EPSILON);
        assert_eq!(summary.max_duration_ms, 400);
        assert_eq!(summary.total_input_tokens, 60);
        assert_eq!(summary.total_output_tokens, 90);
    }

    #[test]
    fn aggregate_all_failures() {
        let metrics: Vec<ToolCallMetric> = (0..5)
            .map(|i| ToolCallMetric {
                tool_name: "write_file".into(),
                duration_ms: (i + 1) * 100,
                success: false,
                input_tokens: 0,
                output_tokens: 0,
                timestamp_ms: i as i64 * 1_000,
            })
            .collect();
        let summary = aggregate(&metrics).unwrap();
        assert_eq!(summary.total_calls, 5);
        assert_eq!(summary.success_count, 0);
        assert_eq!(summary.failure_count, 5);
        assert_eq!(summary.max_duration_ms, 500);
    }

    #[test]
    fn metric_serde_roundtrip() {
        let m = ToolCallMetric {
            tool_name: "grep".into(),
            duration_ms: 42,
            success: true,
            input_tokens: 7,
            output_tokens: 13,
            timestamp_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&m).unwrap();
        let decoded: ToolCallMetric = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, m);
    }

    #[test]
    fn summary_serde_roundtrip() {
        let s = ToolCallSummary {
            total_calls: 10,
            success_count: 8,
            failure_count: 2,
            mean_duration_ms: 123.4,
            max_duration_ms: 500,
            total_input_tokens: 100,
            total_output_tokens: 200,
        };
        let json = serde_json::to_string(&s).unwrap();
        let decoded: ToolCallSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, s);
    }
}
