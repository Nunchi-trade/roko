//! Baseline computation from accumulated [`TaskMetric`] records.
//!
//! A baseline is a summary of historical performance — pass rate, average cost,
//! average duration, and average iterations — sliced by agent role and complexity
//! band. It answers: "given our current configuration, what should we expect for
//! this kind of task?"
//!
//! Baselines are computed from a `Vec<TaskMetric>` and stored as a simple struct
//! hierarchy. They are the "before" snapshot in every regression check.

use std::collections::HashMap;

use roko_core::metric::TaskMetric;
use serde::{Deserialize, Serialize};

// ─── Baseline ───────────────────────────────────────────────────────────────

/// A summary baseline for a specific (role, complexity) slice.
///
/// Represents historical performance characteristics derived from past gate
/// executions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliceBaseline {
    /// Agent role (e.g. `"Implementer"`, `"Reviewer"`).
    pub role: String,
    /// Complexity band (e.g. `"trivial"`, `"simple"`, `"standard"`, `"complex"`).
    pub complexity_band: String,
    /// Fraction of first-attempt gate runs that passed (`[0..1]`).
    pub pass_rate: f64,
    /// Average cost in USD per gate execution.
    pub avg_cost: f64,
    /// Average wall-clock milliseconds per gate execution.
    pub avg_duration_ms: f64,
    /// Average iteration count (max iteration per plan in this slice).
    pub avg_iterations: f64,
    /// Average input tokens per gate execution.
    pub avg_input_tokens: f64,
    /// Average output tokens per gate execution.
    pub avg_output_tokens: f64,
    /// Average cache hit rate across records.
    pub avg_cache_hit_rate: f64,
    /// Number of records contributing to this baseline.
    pub n_records: usize,
}

/// Aggregated baselines across all (role, complexity) slices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    /// Per-(role, complexity) baselines.
    pub slices: Vec<SliceBaseline>,
    /// Overall pass rate across all records.
    pub overall_pass_rate: f64,
    /// Overall average cost across all records.
    pub overall_avg_cost: f64,
    /// Overall average duration across all records.
    pub overall_avg_duration_ms: f64,
    /// Total number of records.
    pub total_records: usize,
    /// Minimum number of records that should be present before trusting the baseline.
    pub min_records_for_confidence: usize,
}

impl Baseline {
    /// Whether this baseline has enough data to be considered reliable.
    pub const fn is_confident(&self) -> bool {
        self.total_records >= self.min_records_for_confidence
    }

    /// Look up the slice baseline for a given (role, complexity) pair.
    pub fn lookup(&self, role: &str, complexity: &str) -> Option<&SliceBaseline> {
        self.slices
            .iter()
            .find(|s| s.role == role && s.complexity_band == complexity)
    }

    /// List all distinct roles present in the baseline.
    pub fn roles(&self) -> Vec<&str> {
        let mut roles: Vec<&str> = self.slices.iter().map(|s| s.role.as_str()).collect();
        roles.sort_unstable();
        roles.dedup();
        roles
    }

    /// List all distinct complexity bands present in the baseline.
    pub fn complexity_bands(&self) -> Vec<&str> {
        let mut bands: Vec<&str> = self
            .slices
            .iter()
            .map(|s| s.complexity_band.as_str())
            .collect();
        bands.sort_unstable();
        bands.dedup();
        bands
    }
}

// ─── Computation ────────────────────────────────────────────────────────────

/// Key for grouping records by (role, complexity).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SliceKey {
    role: String,
    complexity_band: String,
}

/// Accumulator for a single slice.
#[derive(Default)]
struct SliceAccum {
    first_attempt_passes: usize,
    first_attempt_total: usize,
    total_cost: f64,
    total_duration_ms: f64,
    total_input_tokens: f64,
    total_output_tokens: f64,
    total_cache_hit_rate: f64,
    plan_max_iters: HashMap<String, u32>,
    n_records: usize,
}

/// Compute a [`Baseline`] from a slice of [`TaskMetric`] records.
///
/// Groups records by `(role, complexity_band)` and computes per-slice
/// statistics. The `min_records` parameter sets the confidence threshold —
/// baselines computed from fewer records are flagged as unreliable.
///
/// Returns a baseline with empty slices if `records` is empty.
#[allow(clippy::cast_precision_loss)]
pub fn compute_baseline(records: &[TaskMetric], min_records: usize) -> Baseline {
    if records.is_empty() {
        return Baseline {
            slices: Vec::new(),
            overall_pass_rate: 0.0,
            overall_avg_cost: 0.0,
            overall_avg_duration_ms: 0.0,
            total_records: 0,
            min_records_for_confidence: min_records,
        };
    }

    let mut groups: HashMap<SliceKey, SliceAccum> = HashMap::new();

    for r in records {
        let key = SliceKey {
            role: r.role.clone(),
            complexity_band: r.complexity_band.clone(),
        };
        let acc = groups.entry(key).or_default();
        acc.n_records += 1;
        acc.total_cost += r.cost_usd;
        acc.total_duration_ms += r.wall_time_ms as f64;
        acc.total_input_tokens += r.input_tokens as f64;
        acc.total_output_tokens += r.output_tokens as f64;
        acc.total_cache_hit_rate += r.cache_hit_rate;

        if r.iteration == 1 {
            acc.first_attempt_total += 1;
            if r.gate_passed {
                acc.first_attempt_passes += 1;
            }
        }

        let iter_entry = acc
            .plan_max_iters
            .entry(r.plan_id.clone())
            .or_insert(0);
        *iter_entry = (*iter_entry).max(r.iteration);
    }

    let mut slices: Vec<SliceBaseline> = groups
        .into_iter()
        .map(|(key, acc)| {
            let n = acc.n_records as f64;
            let pass_rate = if acc.first_attempt_total > 0 {
                acc.first_attempt_passes as f64 / acc.first_attempt_total as f64
            } else {
                0.0
            };
            let avg_iters = if acc.plan_max_iters.is_empty() {
                0.0
            } else {
                let sum: f64 = acc.plan_max_iters.values().map(|&v| f64::from(v)).sum();
                sum / acc.plan_max_iters.len() as f64
            };

            SliceBaseline {
                role: key.role,
                complexity_band: key.complexity_band,
                pass_rate,
                avg_cost: acc.total_cost / n,
                avg_duration_ms: acc.total_duration_ms / n,
                avg_iterations: avg_iters,
                avg_input_tokens: acc.total_input_tokens / n,
                avg_output_tokens: acc.total_output_tokens / n,
                avg_cache_hit_rate: acc.total_cache_hit_rate / n,
                n_records: acc.n_records,
            }
        })
        .collect();

    // Sort slices for deterministic output.
    slices.sort_by(|a, b| (&a.role, &a.complexity_band).cmp(&(&b.role, &b.complexity_band)));

    // Overall aggregates.
    let total_records = records.len();
    let first_attempt: Vec<&TaskMetric> = records.iter().filter(|r| r.iteration == 1).collect();
    let overall_pass_rate = if first_attempt.is_empty() {
        0.0
    } else {
        first_attempt.iter().filter(|r| r.gate_passed).count() as f64 / first_attempt.len() as f64
    };
    let overall_avg_cost: f64 =
        records.iter().map(|r| r.cost_usd).sum::<f64>() / total_records as f64;
    let overall_avg_duration_ms: f64 =
        records.iter().map(|r| r.wall_time_ms as f64).sum::<f64>() / total_records as f64;

    Baseline {
        slices,
        overall_pass_rate,
        overall_avg_cost,
        overall_avg_duration_ms,
        total_records,
        min_records_for_confidence: min_records,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_metric::make_rich_metric;

    #[test]
    fn baseline_empty_records() {
        let b = compute_baseline(&[], 20);
        assert_eq!(b.total_records, 0);
        assert!(b.slices.is_empty());
        assert!(!b.is_confident());
    }

    #[test]
    fn baseline_single_slice() {
        let records = vec![
            make_rich_metric("p1", "t1", "Implementer", "sonnet", "standard", "compile", true, 1, 0.50, 1000, 200, 10000),
            make_rich_metric("p1", "t2", "Implementer", "sonnet", "standard", "compile", false, 1, 0.30, 800, 150, 8000),
            make_rich_metric("p2", "t1", "Implementer", "sonnet", "standard", "compile", true, 1, 0.40, 900, 180, 9000),
        ];

        let b = compute_baseline(&records, 2);
        assert_eq!(b.total_records, 3);
        assert!(b.is_confident());
        assert_eq!(b.slices.len(), 1);

        let s = &b.slices[0];
        assert_eq!(s.role, "Implementer");
        assert_eq!(s.complexity_band, "standard");
        // 2 out of 3 first-attempt passes
        assert!((s.pass_rate - 2.0 / 3.0).abs() < 1e-9);
        assert!((s.avg_cost - 0.4).abs() < 1e-9);
        assert_eq!(s.n_records, 3);
    }

    #[test]
    fn baseline_multiple_slices() {
        let records = vec![
            make_rich_metric("p1", "t1", "Implementer", "s", "simple", "compile", true, 1, 0.10, 100, 50, 1000),
            make_rich_metric("p1", "t2", "Reviewer", "s", "complex", "review", false, 1, 0.20, 200, 100, 2000),
        ];

        let b = compute_baseline(&records, 1);
        assert_eq!(b.slices.len(), 2);

        let impl_slice = b.lookup("Implementer", "simple");
        assert!(impl_slice.is_some());
        assert!((impl_slice.expect("should exist").pass_rate - 1.0).abs() < 1e-9);

        let rev_slice = b.lookup("Reviewer", "complex");
        assert!(rev_slice.is_some());
        assert!((rev_slice.expect("should exist").pass_rate).abs() < 1e-9);
    }

    #[test]
    fn baseline_overall_aggregates() {
        let records = vec![
            make_rich_metric("p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.20, 100, 50, 2000),
            make_rich_metric("p2", "t1", "Impl", "s", "std", "compile", false, 1, 0.40, 200, 100, 4000),
        ];

        let b = compute_baseline(&records, 5);
        assert!(!b.is_confident());
        assert!((b.overall_pass_rate - 0.5).abs() < 1e-9);
        assert!((b.overall_avg_cost - 0.30).abs() < 1e-9);
        assert!((b.overall_avg_duration_ms - 3000.0).abs() < 1e-9);
    }

    #[test]
    fn baseline_roles_and_bands() {
        let records = vec![
            make_rich_metric("p1", "t1", "Implementer", "s", "simple", "compile", true, 1, 0.10, 100, 50, 1000),
            make_rich_metric("p1", "t2", "Reviewer", "s", "complex", "review", true, 1, 0.20, 200, 100, 2000),
            make_rich_metric("p2", "t1", "Implementer", "s", "complex", "test", false, 1, 0.30, 300, 150, 3000),
        ];

        let b = compute_baseline(&records, 1);
        let roles = b.roles();
        assert_eq!(roles, vec!["Implementer", "Reviewer"]);

        let bands = b.complexity_bands();
        assert_eq!(bands, vec!["complex", "simple"]);
    }

    #[test]
    fn baseline_iterations_per_plan() {
        // Plan p1 has iterations 1, 2, 3 → max = 3
        // Plan p2 has iteration 1 → max = 1
        // avg_iterations = (3 + 1) / 2 = 2.0
        let records = vec![
            make_rich_metric("p1", "t1", "Impl", "s", "std", "compile", false, 1, 0.10, 100, 50, 1000),
            make_rich_metric("p1", "t1", "Impl", "s", "std", "compile", false, 2, 0.10, 100, 50, 1000),
            make_rich_metric("p1", "t1", "Impl", "s", "std", "compile", true, 3, 0.10, 100, 50, 1000),
            make_rich_metric("p2", "t1", "Impl", "s", "std", "compile", true, 1, 0.10, 100, 50, 1000),
        ];

        let b = compute_baseline(&records, 1);
        let s = b.lookup("Impl", "std").expect("slice should exist");
        assert!((s.avg_iterations - 2.0).abs() < 1e-9);
    }

    #[test]
    fn baseline_serialization_roundtrip() {
        let records = vec![
            make_rich_metric("p1", "t1", "Impl", "s", "std", "compile", true, 1, 0.50, 1000, 200, 10000),
        ];
        let b = compute_baseline(&records, 1);

        let json = serde_json::to_string(&b).expect("serialize");
        let b2: Baseline = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, b2);
    }
}
