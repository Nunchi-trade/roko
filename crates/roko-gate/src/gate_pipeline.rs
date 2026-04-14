//! `GatePipeline` — sequentially composes inner [`Gate`]s behind a single
//! [`Gate`] impl.
//!
//! This module implements parity §10.15: the orchestrator's "ask every gate
//! in order" verb. The pipeline is itself a `Gate`, so it can be stacked, fed
//! to a registry, or wrapped by higher-level composition just like any leaf
//! gate. Inner gates are invoked strictly in push-order and the pipeline
//! short-circuits on the first failure by default. With
//! [`GatePipeline::without_short_circuit`] every inner gate is exercised and
//! the aggregate verdict records *all* failures.
//!
//! The returned [`Verdict`] is an aggregate:
//!
//! - `passed` is true iff every inner gate passed.
//! - `gate` is the pipeline's configurable display name.
//! - `reason` (on failure) is a compact summary naming the failing gates.
//! - `detail` is a bullet list of every executed step with its status.
//! - `duration_ms` is the wall-clock sum of the executed steps.
//! - `test_count` aggregates inner `test_count`s when any inner gate reported
//!   one, so downstream policies still see cumulative figures.
//!
//! The pipeline never runs gates concurrently — convergence loops rely on
//! compile failures short-circuiting before tests launch. Fan-out of
//! *independent* pipelines is a concern one level up.

use async_trait::async_trait;
use roko_core::{Context, Engram, Gate, TestCount, Verdict};
use std::fmt;
use std::time::Instant;

/// A [`Gate`] that runs a fixed sequence of inner gates.
///
/// Construct with [`GatePipeline::new`] and append inner gates via
/// [`GatePipeline::push`] (or the chaining [`GatePipeline::with_gate`]).
/// The pipeline is empty by default; an empty pipeline passes trivially.
pub struct GatePipeline {
    gates: Vec<Box<dyn Gate>>,
    short_circuit: bool,
    name: String,
}

impl GatePipeline {
    /// Construct an empty pipeline named `name` with short-circuit enabled.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            gates: Vec::new(),
            short_circuit: true,
            name: name.into(),
        }
    }

    /// Append an inner gate to the pipeline.
    pub fn push(&mut self, gate: Box<dyn Gate>) {
        self.gates.push(gate);
    }

    /// Chainable [`Self::push`].
    #[must_use]
    pub fn with_gate(mut self, gate: Box<dyn Gate>) -> Self {
        self.push(gate);
        self
    }

    /// Disable short-circuiting: every inner gate runs, even after a failure.
    #[must_use]
    pub const fn without_short_circuit(mut self) -> Self {
        self.short_circuit = false;
        self
    }

    /// Re-enable short-circuiting (the default).
    #[must_use]
    pub const fn with_short_circuit(mut self) -> Self {
        self.short_circuit = true;
        self
    }

    /// True if short-circuit mode is active.
    #[must_use]
    pub const fn short_circuit(&self) -> bool {
        self.short_circuit
    }

    /// Number of inner gates registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.gates.len()
    }

    /// True when no inner gates are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }
}

impl fmt::Debug for GatePipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatePipeline")
            .field("name", &self.name)
            .field("gates", &self.gates.len())
            .field("short_circuit", &self.short_circuit)
            .finish()
    }
}

/// Wall-clock milliseconds since `started`, saturating at `u64::MAX`.
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Sum two optional [`TestCount`]s, treating `None` as zero on either side
/// but returning `None` iff both inputs were `None`.
const fn merge_test_count(acc: Option<TestCount>, next: Option<TestCount>) -> Option<TestCount> {
    match (acc, next) {
        (None, None) => None,
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (Some(a), Some(b)) => Some(TestCount::new(
            a.passed.saturating_add(b.passed),
            a.failed.saturating_add(b.failed),
            a.ignored.saturating_add(b.ignored),
        )),
    }
}

/// Render a per-step line for the aggregate detail bullet list.
fn render_step_line(index: usize, inner: &Verdict) -> String {
    let status = if inner.passed { "pass" } else { "fail" };
    let reason = if inner.passed {
        String::new()
    } else {
        format!(" — {}", inner.reason)
    };
    format!(
        "{index}. [{status}] {gate} ({ms} ms){reason}",
        index = index + 1,
        gate = inner.gate,
        ms = inner.duration_ms,
    )
}

#[async_trait]
impl Gate for GatePipeline {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict {
        let started = Instant::now();

        // Empty pipeline trivially passes.
        if self.gates.is_empty() {
            return Verdict::pass(&self.name)
                .with_detail("GatePipeline: no inner gates")
                .with_duration(elapsed_ms(started));
        }

        let mut detail_lines: Vec<String> = Vec::with_capacity(self.gates.len());
        let mut failed_names: Vec<String> = Vec::new();
        let mut aggregate_test_count: Option<TestCount> = None;
        let mut steps_run: usize = 0;

        for (idx, gate) in self.gates.iter().enumerate() {
            let inner = gate.verify(signal, ctx).await;
            steps_run += 1;
            detail_lines.push(render_step_line(idx, &inner));
            aggregate_test_count = merge_test_count(aggregate_test_count, inner.test_count);

            if !inner.passed {
                failed_names.push(inner.gate.clone());
                if self.short_circuit {
                    // Record the remaining gates as skipped so the detail
                    // transcript stays honest.
                    for (skip_idx, skipped) in self.gates.iter().enumerate().skip(idx + 1) {
                        detail_lines.push(format!(
                            "{pos}. [skip] {gate} (short-circuit)",
                            pos = skip_idx + 1,
                            gate = skipped.name(),
                        ));
                    }
                    break;
                }
            }
        }

        let elapsed = elapsed_ms(started);
        let passed = failed_names.is_empty();
        let detail = {
            let header = format!(
                "GatePipeline '{}' — {}/{} executed, short_circuit={}",
                self.name,
                steps_run,
                self.gates.len(),
                self.short_circuit,
            );
            let mut out = String::with_capacity(
                header.len() + detail_lines.iter().map(|l| l.len() + 1).sum::<usize>(),
            );
            out.push_str(&header);
            for line in &detail_lines {
                out.push('\n');
                out.push_str(line);
            }
            out
        };

        let mut verdict = if passed {
            Verdict::pass(&self.name).with_detail(detail)
        } else {
            let reason = if failed_names.len() == 1 {
                format!("inner gate failed: {}", failed_names[0])
            } else {
                format!(
                    "{} inner gates failed: {}",
                    failed_names.len(),
                    failed_names.join(", ")
                )
            };
            Verdict::fail(&self.name, reason).with_detail(detail)
        };
        verdict = verdict.with_duration(elapsed);
        if let Some(tc) = aggregate_test_count {
            verdict = verdict.with_test_count(tc);
        }
        verdict
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use roko_core::{Body, Context, Engram, Gate, Kind, TestCount, Verdict};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A configurable mock gate for pipeline tests. Records every invocation
    /// in a shared counter so tests can assert "did gate N actually run?".
    struct MockGate {
        name: String,
        pass: bool,
        reason: String,
        duration_ms: u64,
        test_count: Option<TestCount>,
        calls: Arc<AtomicUsize>,
    }

    impl MockGate {
        fn new(name: impl Into<String>, pass: bool) -> Self {
            Self {
                name: name.into(),
                pass,
                reason: "boom".into(),
                duration_ms: 0,
                test_count: None,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn with_duration(mut self, ms: u64) -> Self {
            self.duration_ms = ms;
            self
        }

        fn with_test_count(mut self, tc: TestCount) -> Self {
            self.test_count = Some(tc);
            self
        }

        fn with_reason(mut self, reason: impl Into<String>) -> Self {
            self.reason = reason.into();
            self
        }

        fn with_shared_counter(mut self, counter: Arc<AtomicUsize>) -> Self {
            self.calls = counter;
            self
        }

        fn calls_handle(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }
    }

    #[async_trait]
    impl Gate for MockGate {
        async fn verify(&self, _signal: &Engram, _ctx: &Context) -> Verdict {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut v = if self.pass {
                Verdict::pass(&self.name)
            } else {
                Verdict::fail(&self.name, &self.reason)
            };
            v = v.with_duration(self.duration_ms);
            if let Some(tc) = self.test_count {
                v = v.with_test_count(tc);
            }
            v
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    fn signal() -> Engram {
        Engram::builder(Kind::Task).body(Body::empty()).build()
    }

    fn ctx() -> Context {
        Context::at(0)
    }

    #[tokio::test]
    async fn empty_pipeline_passes_trivially() {
        let pipeline = GatePipeline::new("empty");
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert_eq!(v.gate, "empty");
        let detail = v.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("no inner gates"));
        assert!(pipeline.is_empty());
        assert_eq!(pipeline.len(), 0);
    }

    #[tokio::test]
    async fn single_passing_gate_yields_pass() {
        let a = MockGate::new("a", true);
        let counter = a.calls_handle();
        let pipeline = GatePipeline::new("pipe").with_gate(Box::new(a));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(pipeline.len(), 1);
        let detail = v.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("[pass] a"));
    }

    #[tokio::test]
    async fn single_failing_gate_yields_fail() {
        let a = MockGate::new("a", false).with_reason("nope");
        let pipeline = GatePipeline::new("pipe").with_gate(Box::new(a));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(!v.passed);
        assert_eq!(v.gate, "pipe");
        assert!(v.reason.contains('a'));
        let detail = v.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("[fail] a"));
        assert!(detail.contains("nope"));
    }

    #[tokio::test]
    async fn all_passing_gates_return_aggregate_pass() {
        let a = MockGate::new("compile", true);
        let b = MockGate::new("lint", true);
        let c = MockGate::new("test", true);
        let ac = a.calls_handle();
        let bc = b.calls_handle();
        let cc = c.calls_handle();
        let pipeline = GatePipeline::new("full")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .with_gate(Box::new(c));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert_eq!(ac.load(Ordering::SeqCst), 1);
        assert_eq!(bc.load(Ordering::SeqCst), 1);
        assert_eq!(cc.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn short_circuits_after_first_failure() {
        let a = MockGate::new("first", true);
        let b = MockGate::new("second", false);
        let c = MockGate::new("third", true);
        let ac = a.calls_handle();
        let bc = b.calls_handle();
        let cc = c.calls_handle();
        let pipeline = GatePipeline::new("sc")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .with_gate(Box::new(c));
        assert!(pipeline.short_circuit());
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(!v.passed);
        assert_eq!(ac.load(Ordering::SeqCst), 1, "first ran");
        assert_eq!(bc.load(Ordering::SeqCst), 1, "second ran");
        assert_eq!(cc.load(Ordering::SeqCst), 0, "third skipped");
        let detail = v.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("[skip] third"));
    }

    #[tokio::test]
    async fn without_short_circuit_runs_every_gate() {
        let first = MockGate::new("first", true);
        let second = MockGate::new("second", false);
        let third = MockGate::new("third", false);
        let fourth = MockGate::new("fourth", true);
        let first_calls = first.calls_handle();
        let second_calls = second.calls_handle();
        let third_calls = third.calls_handle();
        let fourth_calls = fourth.calls_handle();
        let pipeline = GatePipeline::new("fan")
            .with_gate(Box::new(first))
            .with_gate(Box::new(second))
            .with_gate(Box::new(third))
            .with_gate(Box::new(fourth))
            .without_short_circuit();
        assert!(!pipeline.short_circuit());
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(!v.passed);
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.load(Ordering::SeqCst), 1);
        assert_eq!(third_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fourth_calls.load(Ordering::SeqCst), 1);
        assert!(v.reason.contains("second"));
        assert!(v.reason.contains("third"));
    }

    struct OrderedGate {
        name: String,
        expected_position: usize,
        counter: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Gate for OrderedGate {
        async fn verify(&self, _s: &Engram, _c: &Context) -> Verdict {
            let position = self.counter.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                position, self.expected_position,
                "gate {} ran out of order",
                self.name
            );
            Verdict::pass(&self.name)
        }
        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn gates_execute_in_push_order() {
        let order: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
        let pipeline = GatePipeline::new("order")
            .with_gate(Box::new(OrderedGate {
                name: "first".into(),
                expected_position: 0,
                counter: Arc::clone(&order),
            }))
            .with_gate(Box::new(OrderedGate {
                name: "second".into(),
                expected_position: 1,
                counter: Arc::clone(&order),
            }))
            .with_gate(Box::new(OrderedGate {
                name: "third".into(),
                expected_position: 2,
                counter: Arc::clone(&order),
            }));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert_eq!(order.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn aggregates_test_counts_across_gates() {
        let a = MockGate::new("unit", true).with_test_count(TestCount::new(12, 0, 1));
        let b = MockGate::new("integration", true).with_test_count(TestCount::new(5, 0, 0));
        let c = MockGate::new("lint", true); // no test count
        let pipeline = GatePipeline::new("agg")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .with_gate(Box::new(c));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        let tc = v.test_count.expect("aggregate present");
        assert_eq!(tc.passed, 17);
        assert_eq!(tc.failed, 0);
        assert_eq!(tc.ignored, 1);
    }

    #[tokio::test]
    async fn aggregate_test_count_absent_when_no_inner_counts() {
        let a = MockGate::new("a", true);
        let b = MockGate::new("b", true);
        let pipeline = GatePipeline::new("noagg")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert!(v.test_count.is_none());
    }

    #[tokio::test]
    async fn detail_enumerates_every_executed_step() {
        let a = MockGate::new("alpha", true).with_duration(3);
        let b = MockGate::new("beta", true).with_duration(4);
        let pipeline = GatePipeline::new("detail")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        let detail = v.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("1. [pass] alpha"));
        assert!(detail.contains("2. [pass] beta"));
        assert!(detail.contains("2/2 executed"));
        assert!(detail.contains("short_circuit=true"));
    }

    #[tokio::test]
    async fn name_is_preserved_on_verdict() {
        let pipeline = GatePipeline::new("my_pipeline");
        assert_eq!(pipeline.name(), "my_pipeline");
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert_eq!(v.gate, "my_pipeline");
    }

    #[tokio::test]
    async fn push_method_mutates_in_place() {
        let mut pipeline = GatePipeline::new("mutate");
        assert!(pipeline.is_empty());
        pipeline.push(Box::new(MockGate::new("a", true)));
        pipeline.push(Box::new(MockGate::new("b", true)));
        assert_eq!(pipeline.len(), 2);
        assert!(!pipeline.is_empty());
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
    }

    #[tokio::test]
    async fn with_short_circuit_reenables_flag() {
        let pipeline = GatePipeline::new("flip")
            .without_short_circuit()
            .with_short_circuit();
        assert!(pipeline.short_circuit());
    }

    #[tokio::test]
    async fn short_circuit_reports_single_failure_reason() {
        let a = MockGate::new("only_bad", false).with_reason("kaboom");
        let b = MockGate::new("never_runs", true);
        let pipeline = GatePipeline::new("sfr")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(!v.passed);
        assert_eq!(v.reason, "inner gate failed: only_bad");
    }

    #[tokio::test]
    async fn fan_out_reports_multiple_failures_in_reason() {
        let a = MockGate::new("g1", false);
        let b = MockGate::new("g2", false);
        let pipeline = GatePipeline::new("multi")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .without_short_circuit();
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(!v.passed);
        assert!(v.reason.starts_with("2 inner gates failed"));
        assert!(v.reason.contains("g1"));
        assert!(v.reason.contains("g2"));
    }

    #[tokio::test]
    async fn shared_counter_tracks_every_call() {
        let counter = Arc::new(AtomicUsize::new(0));
        let a = MockGate::new("a", true).with_shared_counter(Arc::clone(&counter));
        let b = MockGate::new("b", true).with_shared_counter(Arc::clone(&counter));
        let c = MockGate::new("c", true).with_shared_counter(Arc::clone(&counter));
        let pipeline = GatePipeline::new("shared")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .with_gate(Box::new(c));
        let v = pipeline.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn debug_output_contains_name_and_gate_count() {
        let pipeline = GatePipeline::new("dbg")
            .with_gate(Box::new(MockGate::new("x", true)))
            .with_gate(Box::new(MockGate::new("y", true)));
        let formatted = format!("{pipeline:?}");
        assert!(formatted.contains("dbg"));
        assert!(formatted.contains("gates: 2"));
    }
}
