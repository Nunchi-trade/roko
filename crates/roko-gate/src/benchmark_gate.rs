//! `BenchmarkRegressionGate` — detects benchmark regressions.
//!
//! Compares current benchmark results against a baseline to detect
//! performance regressions. Uses `cargo bench` output when available.
//! Currently a stub that passes through; will be filled in when baseline
//! infrastructure is added.

use async_trait::async_trait;
use roko_core::{Context, Signal, Verify, Verdict};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Threshold for regression detection (percentage slowdown allowed).
const DEFAULT_REGRESSION_THRESHOLD_PCT: f64 = 10.0;

/// A single benchmark comparison result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchmarkComparison {
    /// Benchmark name.
    pub name: String,
    /// Baseline time in nanoseconds.
    pub baseline_ns: f64,
    /// Current time in nanoseconds.
    pub current_ns: f64,
    /// Percentage change (positive = slower).
    pub change_pct: f64,
}

/// A gate that detects benchmark regressions.
pub struct BenchmarkRegressionGate {
    name: String,
    /// Maximum allowed slowdown percentage before failing.
    threshold_pct: f64,
}

impl BenchmarkRegressionGate {
    /// Create a benchmark regression gate with the default threshold.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: "benchmark_regression".to_string(),
            threshold_pct: DEFAULT_REGRESSION_THRESHOLD_PCT,
        }
    }

    /// Override the regression threshold percentage.
    #[must_use]
    pub fn with_threshold_pct(mut self, pct: f64) -> Self {
        self.threshold_pct = pct;
        self
    }
}

impl Default for BenchmarkRegressionGate {
    fn default() -> Self {
        Self::new()
    }
}

impl roko_core::Cell for BenchmarkRegressionGate {
    fn cell_id(&self) -> &str { "benchmark-gate" }
    fn cell_name(&self) -> &str { "BenchmarkRegressionGate" }
    fn protocols(&self) -> &[&str] { &["Verify"] }
}

#[async_trait]
impl Verify for BenchmarkRegressionGate {
    async fn verify(&self, _signal: &Signal, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let elapsed_ms = || {
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
        };

        // Stub: no baseline infrastructure yet. Pass through.
        Verdict::pass(&self.name)
            .with_detail(format!(
                "benchmark regression gate (stub); threshold={:.1}%; no baseline available",
                self.threshold_pct
            ))
            .with_duration(elapsed_ms())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    fn signal() -> Signal {
        Signal::builder(Kind::Task).body(Body::empty()).build()
    }

    fn ctx() -> Context {
        Context::at(0)
    }

    #[tokio::test]
    async fn benchmark_gate_passes_as_stub() {
        let gate = BenchmarkRegressionGate::new();
        let verdict = gate.verify(&signal(), &ctx()).await;
        assert!(verdict.passed);
        assert_eq!(verdict.gate, "benchmark_regression");
    }

    #[tokio::test]
    async fn benchmark_gate_custom_threshold() {
        let gate = BenchmarkRegressionGate::new().with_threshold_pct(5.0);
        let verdict = gate.verify(&signal(), &ctx()).await;
        assert!(verdict.passed);
        let detail = verdict.detail.unwrap_or_default();
        assert!(detail.contains("5.0%"));
    }

    #[tokio::test]
    async fn benchmark_gate_default() {
        let gate = BenchmarkRegressionGate::default();
        assert_eq!(gate.name(), "benchmark_regression");
    }
}
