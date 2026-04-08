//! Histogram primitive tuned for LLM-latency observations (§40.4).
//!
//! Buckets are in **seconds**. The cumulative layout matches the Prometheus
//! text-format contract: each `_bucket{le="X"}` counts observations with
//! `value <= X`, and a final `+Inf` bucket counts every observation.
//!
//! Internally we store an atomic `u64` counter per bucket plus a `u64`
//! sum-of-bits-cast-f64 for the sum (updated via compare-and-swap) so the
//! histogram is lock-free on the hot path.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::obs::metrics::LabelSet;

/// Latency buckets tuned for LLM calls: p50 ≈ 500ms, p99 ≈ 60s.
///
/// Units are seconds. The final `+Inf` bucket is implicit and added by
/// [`Histogram::render_prometheus`]; do not include it here.
pub const LLM_LATENCY_BUCKETS: &[f64] =
    &[0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0];

/// Lock-free histogram with fixed, caller-supplied bucket boundaries.
///
/// Cumulative counts are computed on snapshot from the per-bucket counters.
/// `observe` is O(buckets) — typical use is ≤ 12 buckets, so this is a
/// handful of branches on the hot path.
#[derive(Debug)]
pub struct Histogram {
    /// Upper bounds (exclusive of `+Inf`). Sorted ascending.
    buckets: Vec<f64>,
    /// Atomic counter per bucket. `buckets[i]` counts observations with
    /// `value <= buckets[i]` AND `value > buckets[i-1]` (or no lower bound
    /// when `i == 0`). Rendering converts to cumulative counts.
    counts: Vec<AtomicU64>,
    /// Observations with `value > buckets.last()` (the `+Inf` bucket).
    inf_count: AtomicU64,
    /// Sum of all observed values, stored as `f64::to_bits` for
    /// compare-and-swap updates.
    sum_bits: AtomicU64,
    /// Total number of observations.
    count: AtomicU64,
}

impl Histogram {
    /// Construct a new histogram.
    ///
    /// # Panics
    /// Panics if `buckets` is empty or not strictly ascending.
    #[must_use]
    pub fn new(buckets: Vec<f64>) -> Self {
        assert!(!buckets.is_empty(), "histogram buckets must be non-empty");
        for window in buckets.windows(2) {
            assert!(
                window[0] < window[1],
                "histogram buckets must be strictly ascending",
            );
        }
        let counts = (0..buckets.len()).map(|_| AtomicU64::new(0)).collect();
        Self {
            buckets,
            counts,
            inf_count: AtomicU64::new(0),
            sum_bits: AtomicU64::new(0f64.to_bits()),
            count: AtomicU64::new(0),
        }
    }

    /// Record a single observation. `value_seconds` is the value being
    /// observed; negative values are accepted verbatim (Prometheus
    /// tolerates them).
    pub fn observe(&self, value_seconds: f64) {
        // Find the first bucket whose upper bound is >= value.
        let mut placed = false;
        for (i, upper) in self.buckets.iter().enumerate() {
            if value_seconds <= *upper {
                self.counts[i].fetch_add(1, Ordering::Relaxed);
                placed = true;
                break;
            }
        }
        if !placed {
            self.inf_count.fetch_add(1, Ordering::Relaxed);
        }
        self.count.fetch_add(1, Ordering::Relaxed);

        // CAS-add on the sum. Retries on contention.
        let mut cur = self.sum_bits.load(Ordering::Relaxed);
        loop {
            let new = f64::from_bits(cur) + value_seconds;
            match self.sum_bits.compare_exchange_weak(
                cur,
                new.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
    }

    /// Return an immutable snapshot of the histogram state.
    ///
    /// The returned `counts` are **cumulative** (Prometheus convention):
    /// `counts[i]` is the number of observations with `value <= buckets[i]`,
    /// and the final entry corresponds to `+Inf` and equals `count`.
    #[must_use]
    pub fn snapshot(&self) -> HistogramSnapshot {
        let mut cum = Vec::with_capacity(self.buckets.len() + 1);
        let mut running = 0u64;
        for c in &self.counts {
            running = running.saturating_add(c.load(Ordering::Relaxed));
            cum.push(running);
        }
        // +Inf cumulative count:
        running = running.saturating_add(self.inf_count.load(Ordering::Relaxed));
        cum.push(running);

        HistogramSnapshot {
            buckets: self.buckets.clone(),
            counts: cum,
            sum: f64::from_bits(self.sum_bits.load(Ordering::Relaxed)),
            count: self.count.load(Ordering::Relaxed),
        }
    }

    /// Render this histogram in Prometheus text-exposition format.
    ///
    /// Produces `# HELP` / `# TYPE` lines, then one `<name>_bucket{...,le="X"}`
    /// line per bucket (including `+Inf`), then `_sum` and `_count` lines.
    /// Each line is terminated with `\n`.
    #[must_use]
    pub fn render_prometheus(&self, name: &str, help: &str, labels: &LabelSet) -> String {
        let snap = self.snapshot();
        let mut out = String::new();
        let _ = writeln!(out, "# HELP {name} {}", escape_help(help));
        let _ = writeln!(out, "# TYPE {name} histogram");

        let base_labels = labels.render_inner();
        let sep = if base_labels.is_empty() { "" } else { "," };

        for (upper, cum) in self.buckets.iter().zip(snap.counts.iter()) {
            let _ = writeln!(
                out,
                "{name}_bucket{{{base_labels}{sep}le=\"{}\"}} {cum}",
                format_f64(*upper),
            );
        }
        // +Inf bucket
        let inf_cum = snap.counts.last().copied().unwrap_or(0);
        let _ = writeln!(
            out,
            "{name}_bucket{{{base_labels}{sep}le=\"+Inf\"}} {inf_cum}"
        );

        // sum and count
        if base_labels.is_empty() {
            let _ = writeln!(out, "{name}_sum {}", format_f64(snap.sum));
            let _ = writeln!(out, "{name}_count {}", snap.count);
        } else {
            let _ = writeln!(out, "{name}_sum{{{base_labels}}} {}", format_f64(snap.sum));
            let _ = writeln!(out, "{name}_count{{{base_labels}}} {}", snap.count);
        }
        out
    }
}

/// Immutable snapshot of a histogram. `counts` is **cumulative** (includes
/// the `+Inf` bucket as its final entry, so `counts.len() == buckets.len() + 1`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HistogramSnapshot {
    /// Upper bucket bounds (does NOT include `+Inf`).
    pub buckets: Vec<f64>,
    /// Cumulative observation counts per bucket; final entry is the `+Inf`
    /// cumulative count.
    pub counts: Vec<u64>,
    /// Sum of all observed values.
    pub sum: f64,
    /// Total number of observations.
    pub count: u64,
}

/// Render an `f64` such that integral values drop their `.0` suffix
/// (Prometheus clients accept both but `le="1"` is friendlier than
/// `le="1.0"` when composing alerts).
pub(crate) fn format_f64(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v.is_sign_negative() { "-Inf" } else { "+Inf" }.into();
    }
    // Integral check without direct float equality: compare truncation bits.
    #[allow(clippy::float_cmp)]
    let is_integral = v == v.trunc();
    if is_integral && v.abs() < 1e16 {
        // Safe because of the 1e16 guard above: |v| fits in i64.
        #[allow(clippy::cast_possible_truncation)]
        let as_i = v as i64;
        format!("{as_i}")
    } else {
        format!("{v}")
    }
}

pub(crate) fn escape_help(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_empty_buckets() {
        let result = std::panic::catch_unwind(|| Histogram::new(vec![]));
        assert!(result.is_err());
    }

    #[test]
    fn new_rejects_unsorted_buckets() {
        let result = std::panic::catch_unwind(|| Histogram::new(vec![1.0, 0.5]));
        assert!(result.is_err());
    }

    #[test]
    fn observe_places_into_correct_bucket() {
        let h = Histogram::new(LLM_LATENCY_BUCKETS.to_vec());
        h.observe(0.05); // first bucket
        h.observe(0.2); // fits in 0.25 bucket
        h.observe(45.0); // fits in 60 bucket
        h.observe(10_000.0); // +Inf

        let snap = h.snapshot();
        assert_eq!(snap.count, 4);
        // cumulative counts: index 0 covers 0.05 → 1
        assert_eq!(snap.counts[0], 1);
        // index for 0.25 is 2 in LLM_LATENCY_BUCKETS (0.05, 0.1, 0.25)
        assert_eq!(snap.counts[2], 2);
        // 60s is at index 9 (0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60)
        assert_eq!(snap.counts[9], 3);
        // +Inf (last) picks up the 10000s outlier
        assert_eq!(*snap.counts.last().expect("has inf bucket"), 4);
    }

    #[test]
    fn observe_accumulates_sum() {
        let h = Histogram::new(vec![1.0, 10.0]);
        h.observe(0.5);
        h.observe(0.25);
        h.observe(5.0);
        let snap = h.snapshot();
        assert!((snap.sum - 5.75).abs() < 1e-9);
        assert_eq!(snap.count, 3);
    }

    #[test]
    fn render_prometheus_includes_buckets_sum_count() {
        let h = Histogram::new(vec![0.5, 1.0]);
        h.observe(0.25);
        h.observe(0.75);
        h.observe(2.0);
        let labels = LabelSet::new();
        let out = h.render_prometheus("roko_latency_seconds", "Agent latency", &labels);
        assert!(out.contains("# HELP roko_latency_seconds Agent latency\n"));
        assert!(out.contains("# TYPE roko_latency_seconds histogram\n"));
        assert!(out.contains("roko_latency_seconds_bucket{le=\"0.5\"} 1\n"));
        assert!(out.contains("roko_latency_seconds_bucket{le=\"1\"} 2\n"));
        assert!(out.contains("roko_latency_seconds_bucket{le=\"+Inf\"} 3\n"));
        assert!(out.contains("roko_latency_seconds_count 3\n"));
    }

    #[test]
    fn render_prometheus_with_labels_preserves_label_set() {
        let h = Histogram::new(vec![1.0]);
        h.observe(0.5);
        let labels = LabelSet::from_pairs(&[("backend", "claude"), ("role", "coder")]);
        let out = h.render_prometheus("roko_agent_duration_seconds", "duration", &labels);
        assert!(out.contains(
            "roko_agent_duration_seconds_bucket{backend=\"claude\",role=\"coder\",le=\"1\"} 1\n"
        ));
        assert!(out.contains(
            "roko_agent_duration_seconds_bucket{backend=\"claude\",role=\"coder\",le=\"+Inf\"} 1\n"
        ));
        assert!(
            out.contains(
                "roko_agent_duration_seconds_count{backend=\"claude\",role=\"coder\"} 1\n"
            )
        );
    }

    #[test]
    fn snapshot_counts_are_cumulative() {
        let h = Histogram::new(vec![1.0, 5.0, 10.0]);
        h.observe(0.5);
        h.observe(0.5);
        h.observe(3.0);
        h.observe(50.0);
        let snap = h.snapshot();
        // 2 <= 1; 3 <= 5; 3 <= 10; 4 (+Inf)
        assert_eq!(snap.counts, vec![2, 3, 3, 4]);
    }
}
