//! Extended task metric types and JSONL writer for the learning subsystem.
//!
//! The canonical [`TaskMetric`] struct lives in `roko-core::metric` and is
//! re-exported here. This module adds:
//!
//! - [`MetricsWriter`] — thread-safe, append-only JSONL writer that batches
//!   records in memory and flushes to an `AsyncWrite` sink. No `std::fs` in
//!   library code — callers supply a tokio-compatible writer.
//! - [`MetricsReader`] — parse JSONL lines from bytes, tolerant of corrupted
//!   lines.
//! - [`MetricFilter`] — declarative filter over records by role, complexity
//!   band, plan, gate name, etc.
//!
//! # Design
//!
//! `TaskMetric` is the atomic unit of the learning system — every gate
//! execution produces one. Records are immutable; `.jsonl` is append-only.

use std::collections::HashSet;

use parking_lot::Mutex;
use roko_core::metric::{ConfigHash, TaskMetric};
use serde::{Deserialize, Serialize};

// Re-export the core type so downstream code can `use roko_learn::task_metric::TaskMetric`.
pub use roko_core::metric::{Headlines, TaskMetric as CoreTaskMetric, compute_headlines};

// ─── MetricFilter ───────────────────────────────────────────────────────────

/// Declarative filter over a stream of [`TaskMetric`] records.
///
/// All predicates are AND-combined: a record must match every non-empty
/// filter field to be included.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricFilter {
    /// If non-empty, record's `role` must be in this set.
    pub roles: HashSet<String>,
    /// If non-empty, record's `complexity_band` must be in this set.
    pub complexity_bands: HashSet<String>,
    /// If non-empty, record's `plan_id` must be in this set.
    pub plan_ids: HashSet<String>,
    /// If non-empty, record's `gate` must be in this set.
    pub gates: HashSet<String>,
    /// If non-empty, record's `model` must be in this set.
    pub models: HashSet<String>,
    /// If non-empty, record's `backend` must be in this set.
    pub backends: HashSet<String>,
    /// If `Some`, only include records where `gate_passed` matches.
    pub gate_passed: Option<bool>,
    /// If `Some`, only include records with `iteration` in `[min..=max]`.
    pub iteration_range: Option<(u32, u32)>,
    /// If `Some`, only include records with `cost_usd >= min`.
    pub min_cost_usd: Option<f64>,
    /// If `Some`, only include records with `cost_usd <= max`.
    pub max_cost_usd: Option<f64>,
    /// If non-empty, record's `config_hash` must be in this set.
    pub config_hashes: HashSet<String>,
}

impl MetricFilter {
    /// Create a filter that passes everything.
    pub fn all() -> Self {
        Self::default()
    }

    /// Test whether a single record passes all filter predicates.
    pub fn matches(&self, r: &TaskMetric) -> bool {
        if !self.roles.is_empty() && !self.roles.contains(&r.role) {
            return false;
        }
        if !self.complexity_bands.is_empty() && !self.complexity_bands.contains(&r.complexity_band)
        {
            return false;
        }
        if !self.plan_ids.is_empty() && !self.plan_ids.contains(&r.plan_id) {
            return false;
        }
        if !self.gates.is_empty() && !self.gates.contains(&r.gate) {
            return false;
        }
        if !self.models.is_empty() && !self.models.contains(&r.model) {
            return false;
        }
        if !self.backends.is_empty() && !self.backends.contains(&r.backend) {
            return false;
        }
        if let Some(passed) = self.gate_passed {
            if r.gate_passed != passed {
                return false;
            }
        }
        if let Some((min, max)) = self.iteration_range {
            if r.iteration < min || r.iteration > max {
                return false;
            }
        }
        if let Some(min) = self.min_cost_usd {
            if r.cost_usd < min {
                return false;
            }
        }
        if let Some(max) = self.max_cost_usd {
            if r.cost_usd > max {
                return false;
            }
        }
        if !self.config_hashes.is_empty() && !self.config_hashes.contains(r.config_hash.as_str()) {
            return false;
        }
        true
    }

    /// Filter a slice in-place, retaining only matching records.
    pub fn apply(&self, records: &[TaskMetric]) -> Vec<TaskMetric> {
        records
            .iter()
            .filter(|r| self.matches(r))
            .cloned()
            .collect()
    }
}

// ─── MetricsWriter ──────────────────────────────────────────────────────────

/// Thread-safe, append-only JSONL metrics writer.
///
/// Accumulates serialized lines in an internal buffer. Call [`flush_to`] to
/// drain the buffer into any `impl std::io::Write`. The writer never touches
/// the filesystem directly — the caller decides where bytes go.
pub struct MetricsWriter {
    /// Internal buffer of serialized JSONL lines (each ends with `\n`).
    buffer: Mutex<Vec<String>>,
}

impl MetricsWriter {
    /// Create a new empty writer.
    pub const fn new() -> Self {
        Self {
            buffer: Mutex::new(Vec::new()),
        }
    }

    /// Append a single metric record. Returns the serialized line (without
    /// trailing newline) on success.
    ///
    /// # Errors
    ///
    /// Returns an error if JSON serialization fails (should never happen for
    /// `TaskMetric`).
    pub fn append(&self, metric: &TaskMetric) -> Result<String, serde_json::Error> {
        let line = serde_json::to_string(metric)?;
        self.buffer.lock().push(format!("{line}\n"));
        Ok(line)
    }

    /// Drain the internal buffer and write all accumulated lines to `sink`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if writing fails.
    pub fn flush_to(&self, sink: &mut dyn std::io::Write) -> std::io::Result<usize> {
        let lines: Vec<String> = {
            let mut buf = self.buffer.lock();
            std::mem::take(&mut *buf)
        };
        let mut total = 0;
        for line in &lines {
            sink.write_all(line.as_bytes())?;
            total += 1;
        }
        Ok(total)
    }

    /// Number of records waiting to be flushed.
    pub fn pending(&self) -> usize {
        self.buffer.lock().len()
    }

    /// Drain the buffer and return the accumulated lines (for testing or
    /// in-memory consumers).
    pub fn take(&self) -> Vec<String> {
        let mut buf = self.buffer.lock();
        std::mem::take(&mut *buf)
    }
}

impl Default for MetricsWriter {
    fn default() -> Self {
        Self::new()
    }
}

// ─── MetricsReader ──────────────────────────────────────────────────────────

/// Parse JSONL bytes into `TaskMetric` records, tolerant of corrupted lines.
///
/// Lines that fail to parse are counted but not included in the result.
#[derive(Debug, Default)]
pub struct ParseResult {
    /// Successfully parsed records.
    pub records: Vec<TaskMetric>,
    /// Number of lines that failed to parse.
    pub errors: usize,
    /// Total lines attempted.
    pub total_lines: usize,
}

/// Tolerant JSONL reader for [`TaskMetric`] records.
///
/// This complements [`MetricsWriter`] for call sites that want a small
/// type-driven read surface instead of directly invoking
/// [`parse_metrics_jsonl`].
#[derive(Debug, Default, Clone, Copy)]
pub struct MetricsReader;

impl MetricsReader {
    /// Parse JSONL text into task metrics, skipping corrupted lines.
    #[must_use]
    pub fn parse_str(text: &str) -> ParseResult {
        parse_metrics_jsonl(text)
    }

    /// Parse UTF-8 JSONL bytes into task metrics.
    ///
    /// Invalid UTF-8 yields an empty result rather than aborting the caller.
    #[must_use]
    pub fn parse_bytes(bytes: &[u8]) -> ParseResult {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self::parse_str(text),
            Err(_) => ParseResult::default(),
        }
    }
}

/// Parse JSONL text into `TaskMetric` records.
///
/// Empty lines are silently skipped. Lines that fail to parse increment
/// `errors` in the returned [`ParseResult`].
pub fn parse_metrics_jsonl(text: &str) -> ParseResult {
    let mut result = ParseResult::default();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        result.total_lines += 1;
        match serde_json::from_str::<TaskMetric>(trimmed) {
            Ok(m) => result.records.push(m),
            Err(_) => result.errors += 1,
        }
    }
    result
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Create a test fixture [`TaskMetric`] with sensible defaults.
///
/// Public so other test modules in the crate can reuse it.
pub fn make_test_metric(plan_id: &str, task_id: &str) -> TaskMetric {
    TaskMetric::new(
        ConfigHash::from("test_hash_0000".to_string()),
        plan_id,
        task_id,
    )
}

/// Create a fully-populated test metric with custom field values.
#[allow(clippy::too_many_arguments)]
pub fn make_rich_metric(
    plan_id: &str,
    task_id: &str,
    role: &str,
    model: &str,
    complexity: &str,
    gate: &str,
    passed: bool,
    iteration: u32,
    cost: f64,
    input_tokens: u64,
    output_tokens: u64,
    wall_time_ms: u64,
) -> TaskMetric {
    let mut m = make_test_metric(plan_id, task_id);
    m.role = role.to_string();
    m.model = model.to_string();
    m.complexity_band = complexity.to_string();
    m.gate = gate.to_string();
    m.gate_passed = passed;
    m.iteration = iteration;
    m.cost_usd = cost;
    m.input_tokens = input_tokens;
    m.output_tokens = output_tokens;
    m.wall_time_ms = wall_time_ms;
    m.backend = "claude".to_string();
    m.timestamp = "2026-04-06T12:00:00Z".to_string();
    m
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── MetricFilter tests ──────────────────────────────────────────

    #[test]
    fn task_metric_filter_all_passes_everything() {
        let m = make_test_metric("p1", "t1");
        let filter = MetricFilter::all();
        assert!(filter.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_role() {
        let mut m = make_test_metric("p1", "t1");
        m.role = "Implementer".into();

        let mut f = MetricFilter::all();
        f.roles.insert("Implementer".into());
        assert!(f.matches(&m));

        f.roles.clear();
        f.roles.insert("Reviewer".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_complexity() {
        let mut m = make_test_metric("p1", "t1");
        m.complexity_band = "complex".into();

        let mut f = MetricFilter::all();
        f.complexity_bands.insert("complex".into());
        assert!(f.matches(&m));

        f.complexity_bands.clear();
        f.complexity_bands.insert("trivial".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_gate_passed() {
        let mut m = make_test_metric("p1", "t1");
        m.gate_passed = true;

        let mut f = MetricFilter::all();
        f.gate_passed = Some(true);
        assert!(f.matches(&m));

        f.gate_passed = Some(false);
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_iteration_range() {
        let mut m = make_test_metric("p1", "t1");
        m.iteration = 3;

        let mut f = MetricFilter::all();
        f.iteration_range = Some((1, 5));
        assert!(f.matches(&m));

        f.iteration_range = Some((4, 10));
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_cost_range() {
        let mut m = make_test_metric("p1", "t1");
        m.cost_usd = 0.50;

        let mut f = MetricFilter::all();
        f.min_cost_usd = Some(0.10);
        f.max_cost_usd = Some(1.00);
        assert!(f.matches(&m));

        f.min_cost_usd = Some(0.60);
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_plan_id() {
        let m = make_test_metric("plan-42", "t1");

        let mut f = MetricFilter::all();
        f.plan_ids.insert("plan-42".into());
        assert!(f.matches(&m));

        f.plan_ids.clear();
        f.plan_ids.insert("plan-99".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_gate_name() {
        let mut m = make_test_metric("p1", "t1");
        m.gate = "compile".into();

        let mut f = MetricFilter::all();
        f.gates.insert("compile".into());
        assert!(f.matches(&m));

        f.gates.clear();
        f.gates.insert("test".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_model() {
        let mut m = make_test_metric("p1", "t1");
        m.model = "claude-sonnet-4-5".into();

        let mut f = MetricFilter::all();
        f.models.insert("claude-sonnet-4-5".into());
        assert!(f.matches(&m));

        f.models.clear();
        f.models.insert("gpt-4o".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_backend() {
        let mut m = make_test_metric("p1", "t1");
        m.backend = "claude".into();

        let mut f = MetricFilter::all();
        f.backends.insert("claude".into());
        assert!(f.matches(&m));

        f.backends.clear();
        f.backends.insert("codex".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_by_config_hash() {
        let m = make_test_metric("p1", "t1");

        let mut f = MetricFilter::all();
        f.config_hashes.insert("test_hash_0000".into());
        assert!(f.matches(&m));

        f.config_hashes.clear();
        f.config_hashes.insert("other_hash".into());
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_combined_and() {
        let m = make_rich_metric(
            "p1",
            "t1",
            "Implementer",
            "sonnet",
            "standard",
            "compile",
            true,
            1,
            0.50,
            1000,
            200,
            5000,
        );

        let mut f = MetricFilter::all();
        f.roles.insert("Implementer".into());
        f.gate_passed = Some(true);
        f.min_cost_usd = Some(0.10);
        assert!(f.matches(&m));

        // Flip one predicate
        f.gate_passed = Some(false);
        assert!(!f.matches(&m));
    }

    #[test]
    fn task_metric_filter_apply_filters_vec() {
        let records = vec![
            make_rich_metric(
                "p1",
                "t1",
                "Implementer",
                "s",
                "simple",
                "compile",
                true,
                1,
                0.10,
                100,
                50,
                1000,
            ),
            make_rich_metric(
                "p1", "t2", "Reviewer", "s", "simple", "compile", false, 1, 0.20, 200, 100, 2000,
            ),
            make_rich_metric(
                "p2",
                "t1",
                "Implementer",
                "s",
                "complex",
                "test",
                true,
                1,
                0.30,
                300,
                150,
                3000,
            ),
        ];

        let mut f = MetricFilter::all();
        f.roles.insert("Implementer".into());
        let filtered = f.apply(&records);
        assert_eq!(filtered.len(), 2);
    }

    // ── MetricsWriter tests ─────────────────────────────────────────

    #[test]
    fn task_metric_writer_append_and_take() {
        let writer = MetricsWriter::new();
        let m = make_test_metric("p1", "t1");
        writer.append(&m).expect("serialization should succeed");
        assert_eq!(writer.pending(), 1);

        let lines = writer.take();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with('\n'));
        assert_eq!(writer.pending(), 0);
    }

    #[test]
    fn task_metric_writer_flush_to_buffer() {
        let writer = MetricsWriter::new();
        let m1 = make_test_metric("p1", "t1");
        let m2 = make_test_metric("p2", "t2");
        writer.append(&m1).expect("ok");
        writer.append(&m2).expect("ok");

        let mut buf = Vec::new();
        let count = writer.flush_to(&mut buf).expect("flush should succeed");
        assert_eq!(count, 2);
        assert_eq!(writer.pending(), 0);

        let text = String::from_utf8(buf).expect("valid utf8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn task_metric_writer_multiple_flushes() {
        let writer = MetricsWriter::new();
        writer.append(&make_test_metric("p1", "t1")).expect("ok");

        let mut buf1 = Vec::new();
        writer.flush_to(&mut buf1).expect("ok");
        assert_eq!(writer.pending(), 0);

        writer.append(&make_test_metric("p2", "t2")).expect("ok");
        let mut buf2 = Vec::new();
        writer.flush_to(&mut buf2).expect("ok");

        // Each flush only contains records appended since last flush.
        let text1 = String::from_utf8(buf1).expect("valid utf8");
        let text2 = String::from_utf8(buf2).expect("valid utf8");
        assert_eq!(text1.lines().count(), 1);
        assert_eq!(text2.lines().count(), 1);
    }

    #[test]
    fn task_metric_writer_default() {
        let writer = MetricsWriter::default();
        assert_eq!(writer.pending(), 0);
    }

    // ── MetricsReader / parse tests ─────────────────────────────────

    #[test]
    fn task_metric_parse_empty_string() {
        let result = parse_metrics_jsonl("");
        assert_eq!(result.records.len(), 0);
        assert_eq!(result.errors, 0);
        assert_eq!(result.total_lines, 0);
    }

    #[test]
    fn task_metric_parse_valid_lines() {
        let m1 = make_test_metric("p1", "t1");
        let m2 = make_test_metric("p2", "t2");
        let text = format!(
            "{}\n{}\n",
            m1.to_jsonl().expect("ok"),
            m2.to_jsonl().expect("ok")
        );

        let result = parse_metrics_jsonl(&text);
        assert_eq!(result.records.len(), 2);
        assert_eq!(result.errors, 0);
        assert_eq!(result.total_lines, 2);
    }

    #[test]
    fn task_metric_parse_tolerates_bad_lines() {
        let m1 = make_test_metric("p1", "t1");
        let text = format!(
            "{}\nnot-valid-json\n{}\n",
            m1.to_jsonl().expect("ok"),
            m1.to_jsonl().expect("ok")
        );

        let result = parse_metrics_jsonl(&text);
        assert_eq!(result.records.len(), 2);
        assert_eq!(result.errors, 1);
        assert_eq!(result.total_lines, 3);
    }

    #[test]
    fn task_metric_parse_skips_blank_lines() {
        let m1 = make_test_metric("p1", "t1");
        let text = format!("\n\n{}\n\n", m1.to_jsonl().expect("ok"));

        let result = parse_metrics_jsonl(&text);
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.errors, 0);
        assert_eq!(result.total_lines, 1);
    }

    // ── JSONL roundtrip ─────────────────────────────────────────────

    #[test]
    fn task_metric_roundtrip_through_writer_and_parser() {
        let writer = MetricsWriter::new();
        let m = make_rich_metric(
            "p1",
            "t1",
            "Implementer",
            "sonnet",
            "standard",
            "compile",
            true,
            1,
            0.42,
            1500,
            300,
            45000,
        );
        writer.append(&m).expect("ok");

        let mut buf = Vec::new();
        writer.flush_to(&mut buf).expect("ok");
        let text = String::from_utf8(buf).expect("valid utf8");
        let result = parse_metrics_jsonl(&text);
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0], m);
    }

    #[test]
    fn task_metric_rich_metric_has_all_fields() {
        let m = make_rich_metric(
            "p1",
            "t1",
            "Implementer",
            "sonnet",
            "standard",
            "compile",
            true,
            2,
            0.42,
            1500,
            300,
            45000,
        );
        assert_eq!(m.role, "Implementer");
        assert_eq!(m.model, "sonnet");
        assert_eq!(m.complexity_band, "standard");
        assert_eq!(m.gate, "compile");
        assert!(m.gate_passed);
        assert_eq!(m.iteration, 2);
        assert!((m.cost_usd - 0.42).abs() < 1e-9);
        assert_eq!(m.input_tokens, 1500);
        assert_eq!(m.output_tokens, 300);
        assert_eq!(m.wall_time_ms, 45000);
    }

    #[test]
    fn task_metric_make_test_metric_defaults() {
        let m = make_test_metric("plan-42", "t3");
        assert_eq!(m.plan_id, "plan-42");
        assert_eq!(m.task_id, "t3");
        assert_eq!(m.iteration, 1);
        assert!(!m.gate_passed);
        assert_eq!(m.cost_usd, 0.0);
    }
}
