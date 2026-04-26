//! Standalone gate combinators: ParallelGate, VotingGate, FallbackGate (GATE-04).
//!
//! Each combinator wraps inner gates and itself implements [`Verify`], enabling
//! algebraic composition of verification pipelines.
//!
//! | Combinator | Strategy | Aggregate |
//! |---|---|---|
//! | [`ParallelGate`] | Run all gates concurrently | min score; fail if any fails |
//! | [`VotingGate`] | Run all gates, require N-of-M pass | mean of passing scores |
//! | [`FallbackGate`] | Try primary; on failure try fallback | first passing verdict |

use async_trait::async_trait;
use roko_core::{Context, Engram, Verdict, Verify};
use std::fmt;

// ─── ParallelGate ────────────────────────────────────────────────────────────

/// Runs N gates concurrently and aggregates verdicts by taking the minimum score.
///
/// If any gate fails, the aggregate fails. Use when inner gates are independent
/// and can safely run simultaneously (e.g., CompileGate + LintGate).
pub struct ParallelGate {
    gates: Vec<Box<dyn Verify>>,
    name: String,
}

impl ParallelGate {
    /// Create a new parallel gate with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            gates: Vec::new(),
            name: name.into(),
        }
    }

    /// Append an inner gate.
    pub fn push(&mut self, gate: Box<dyn Verify>) {
        self.gates.push(gate);
    }

    /// Chainable gate append.
    #[must_use]
    pub fn with_gate(mut self, gate: Box<dyn Verify>) -> Self {
        self.push(gate);
        self
    }

    /// Number of inner gates.
    #[must_use]
    pub fn len(&self) -> usize {
        self.gates.len()
    }

    /// Whether no inner gates are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }
}

impl fmt::Debug for ParallelGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParallelGate")
            .field("name", &self.name)
            .field("gates", &self.gates.len())
            .finish()
    }
}

#[async_trait]
impl Verify for ParallelGate {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict {
        let started = std::time::Instant::now();

        if self.gates.is_empty() {
            return Verdict::pass(&self.name)
                .with_detail("ParallelGate: no inner gates")
                .with_duration(elapsed_ms(started));
        }

        // Run all gates (sequentially here; true tokio::join_all would require
        // Pin<Box<dyn Future>> which is complex with the trait object. The key
        // semantic difference is that all gates run regardless of failures.)
        let mut verdicts = Vec::with_capacity(self.gates.len());
        for gate in &self.gates {
            verdicts.push(gate.verify(signal, ctx).await);
        }

        // Aggregate: min score, fail if any failed.
        let min_score = verdicts
            .iter()
            .map(|v| v.score)
            .fold(f32::INFINITY, f32::min);

        let failed: Vec<&str> = verdicts
            .iter()
            .filter(|v| !v.passed)
            .map(|v| v.gate.as_str())
            .collect();

        let detail = verdicts
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let status = if v.passed { "pass" } else { "fail" };
                format!("  {}. [{status}] {} (score={:.2})", i + 1, v.gate, v.score)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let elapsed = elapsed_ms(started);

        if failed.is_empty() {
            Verdict::pass(&self.name)
                .with_score(min_score)
                .with_detail(format!(
                    "ParallelGate: {}/{} passed\n{detail}",
                    verdicts.len(),
                    verdicts.len()
                ))
                .with_duration(elapsed)
        } else {
            let reason = format!(
                "{} of {} gates failed: {}",
                failed.len(),
                verdicts.len(),
                failed.join(", ")
            );
            Verdict::fail(&self.name, reason)
                .with_score(min_score)
                .with_detail(format!(
                    "ParallelGate: {}/{} passed\n{detail}",
                    verdicts.len() - failed.len(),
                    verdicts.len()
                ))
                .with_duration(elapsed)
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ─── VotingGate ──────────────────────────────────────────────────────────────

/// Runs M gates and requires N-of-M to pass.
///
/// Aggregate score = mean of passing verdicts' scores. Use when multiple
/// reviewers must agree (e.g., 2-of-3 code review gates).
pub struct VotingGate {
    gates: Vec<Box<dyn Verify>>,
    required_passes: usize,
    name: String,
}

impl VotingGate {
    /// Create a new voting gate requiring `required_passes` out of M gates to pass.
    #[must_use]
    pub fn new(name: impl Into<String>, required_passes: usize) -> Self {
        Self {
            gates: Vec::new(),
            required_passes: required_passes.max(1),
            name: name.into(),
        }
    }

    /// Append an inner gate.
    pub fn push(&mut self, gate: Box<dyn Verify>) {
        self.gates.push(gate);
    }

    /// Chainable gate append.
    #[must_use]
    pub fn with_gate(mut self, gate: Box<dyn Verify>) -> Self {
        self.push(gate);
        self
    }

    /// Number of inner gates.
    #[must_use]
    pub fn len(&self) -> usize {
        self.gates.len()
    }

    /// Whether no inner gates are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }
}

impl fmt::Debug for VotingGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VotingGate")
            .field("name", &self.name)
            .field("gates", &self.gates.len())
            .field("required_passes", &self.required_passes)
            .finish()
    }
}

#[async_trait]
impl Verify for VotingGate {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict {
        let started = std::time::Instant::now();

        if self.gates.is_empty() {
            return Verdict::pass(&self.name)
                .with_detail("VotingGate: no inner gates")
                .with_duration(elapsed_ms(started));
        }

        // Run all gates and collect verdicts.
        let mut verdicts = Vec::with_capacity(self.gates.len());
        for gate in &self.gates {
            verdicts.push(gate.verify(signal, ctx).await);
        }

        let pass_count = verdicts.iter().filter(|v| v.passed).count();
        let passing_scores: Vec<f32> = verdicts
            .iter()
            .filter(|v| v.passed)
            .map(|v| v.score)
            .collect();

        let mean_passing_score = if passing_scores.is_empty() {
            0.0
        } else {
            passing_scores.iter().sum::<f32>() / passing_scores.len() as f32
        };

        let detail = verdicts
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let status = if v.passed { "pass" } else { "fail" };
                format!("  {}. [{status}] {} (score={:.2})", i + 1, v.gate, v.score)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let elapsed = elapsed_ms(started);
        let overall_passed = pass_count >= self.required_passes;

        if overall_passed {
            Verdict::pass(&self.name)
                .with_score(mean_passing_score)
                .with_detail(format!(
                    "VotingGate: {pass_count}/{total} passed (required {required})\n{detail}",
                    total = self.gates.len(),
                    required = self.required_passes,
                ))
                .with_duration(elapsed)
        } else {
            let reason = format!(
                "voting failed: {pass_count}/{total} passed, required {required}",
                total = self.gates.len(),
                required = self.required_passes,
            );
            Verdict::fail(&self.name, reason)
                .with_score(mean_passing_score)
                .with_detail(format!(
                    "VotingGate: {pass_count}/{total} passed (required {required})\n{detail}",
                    total = self.gates.len(),
                    required = self.required_passes,
                ))
                .with_duration(elapsed)
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ─── FallbackGate ────────────────────────────────────────────────────────────

/// Tries a primary gate; if it fails, tries a fallback.
///
/// The first passing verdict wins. Use when you want to try a fast check
/// first and fall back to a more thorough one on failure.
pub struct FallbackGate {
    primary: Box<dyn Verify>,
    fallback: Box<dyn Verify>,
    name: String,
}

impl FallbackGate {
    /// Create a fallback gate with the given primary and fallback.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        primary: Box<dyn Verify>,
        fallback: Box<dyn Verify>,
    ) -> Self {
        Self {
            primary,
            fallback,
            name: name.into(),
        }
    }
}

impl fmt::Debug for FallbackGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FallbackGate")
            .field("name", &self.name)
            .field("primary", &self.primary.name())
            .field("fallback", &self.fallback.name())
            .finish()
    }
}

#[async_trait]
impl Verify for FallbackGate {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict {
        let started = std::time::Instant::now();

        // Try primary first.
        let primary_verdict = self.primary.verify(signal, ctx).await;
        if primary_verdict.passed {
            return primary_verdict.with_duration(elapsed_ms(started));
        }

        // Primary failed — try fallback.
        let fallback_verdict = self.fallback.verify(signal, ctx).await;
        let elapsed = elapsed_ms(started);

        if fallback_verdict.passed {
            let mut v = fallback_verdict;
            v = v.with_detail(format!(
                "FallbackGate '{}': primary '{}' failed, fallback '{}' passed\nPrimary reason: {}",
                self.name,
                self.primary.name(),
                self.fallback.name(),
                primary_verdict.reason,
            ));
            v.with_duration(elapsed)
        } else {
            // Both failed.
            Verdict::fail(
                &self.name,
                format!(
                    "both primary '{}' and fallback '{}' failed",
                    self.primary.name(),
                    self.fallback.name(),
                ),
            )
            .with_score(fallback_verdict.score.min(primary_verdict.score))
            .with_detail(format!(
                "FallbackGate '{}': both gates failed\n  Primary ({}): {}\n  Fallback ({}): {}",
                self.name,
                self.primary.name(),
                primary_verdict.reason,
                self.fallback.name(),
                fallback_verdict.reason,
            ))
            .with_duration(elapsed)
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

fn elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use roko_core::{Body, Context, Engram, Kind, Verdict, Verify};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockGate {
        gate_name: String,
        pass: bool,
        calls: Arc<AtomicUsize>,
    }

    impl MockGate {
        fn new(name: &str, pass: bool) -> Self {
            Self {
                gate_name: name.to_string(),
                pass,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn calls_handle(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }
    }

    #[async_trait]
    impl Verify for MockGate {
        async fn verify(&self, _signal: &Engram, _ctx: &Context) -> Verdict {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.pass {
                Verdict::pass(&self.gate_name).with_score(0.9)
            } else {
                Verdict::fail(&self.gate_name, "mock failure").with_score(0.2)
            }
        }

        fn name(&self) -> &str {
            &self.gate_name
        }
    }

    fn signal() -> Engram {
        Engram::builder(Kind::Task).body(Body::empty()).build()
    }

    fn ctx() -> Context {
        Context::at(0)
    }

    // ─── ParallelGate tests ──────────────────────────────────────────

    #[tokio::test]
    async fn parallel_all_pass() {
        let a = MockGate::new("a", true);
        let b = MockGate::new("b", true);
        let ac = a.calls_handle();
        let bc = b.calls_handle();
        let gate = ParallelGate::new("par")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert_eq!(ac.load(Ordering::SeqCst), 1);
        assert_eq!(bc.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn parallel_any_fail_causes_failure() {
        let a = MockGate::new("a", true);
        let b = MockGate::new("b", false);
        let c = MockGate::new("c", true);
        let ac = a.calls_handle();
        let bc = b.calls_handle();
        let cc = c.calls_handle();
        let gate = ParallelGate::new("par")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .with_gate(Box::new(c));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(!v.passed);
        // All gates should have run.
        assert_eq!(ac.load(Ordering::SeqCst), 1);
        assert_eq!(bc.load(Ordering::SeqCst), 1);
        assert_eq!(cc.load(Ordering::SeqCst), 1);
        assert!(v.reason.contains("b"));
    }

    #[tokio::test]
    async fn parallel_aggregate_uses_min_score() {
        let a = MockGate::new("a", true);
        let b = MockGate::new("b", true);
        let gate = ParallelGate::new("par")
            .with_gate(Box::new(a))
            .with_gate(Box::new(b));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert_eq!(v.score, 0.9); // both gates return 0.9
    }

    #[tokio::test]
    async fn parallel_empty_passes() {
        let gate = ParallelGate::new("empty");
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert!(gate.is_empty());
    }

    // ─── VotingGate tests ────────────────────────────────────────────

    #[tokio::test]
    async fn voting_passes_with_enough_votes() {
        let a = MockGate::new("a", true);
        let b = MockGate::new("b", false);
        let c = MockGate::new("c", true);
        // 2-of-3 required.
        let gate = VotingGate::new("vote", 2)
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .with_gate(Box::new(c));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(v.passed, "2/3 should pass with required=2");
    }

    #[tokio::test]
    async fn voting_fails_without_enough_votes() {
        let a = MockGate::new("a", true);
        let b = MockGate::new("b", false);
        let c = MockGate::new("c", false);
        // 2-of-3 required but only 1 passes.
        let gate = VotingGate::new("vote", 2)
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .with_gate(Box::new(c));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(!v.passed, "1/3 should fail with required=2");
    }

    #[tokio::test]
    async fn voting_score_is_mean_of_passing() {
        let a = MockGate::new("a", true);
        let b = MockGate::new("b", true);
        let c = MockGate::new("c", false);
        let gate = VotingGate::new("vote", 2)
            .with_gate(Box::new(a))
            .with_gate(Box::new(b))
            .with_gate(Box::new(c));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        // Both passing mocks return 0.9, mean = 0.9.
        assert!((v.score - 0.9).abs() < 0.01);
    }

    #[tokio::test]
    async fn voting_empty_passes() {
        let gate = VotingGate::new("empty", 1);
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(v.passed);
    }

    // ─── FallbackGate tests ──────────────────────────────────────────

    #[tokio::test]
    async fn fallback_uses_primary_when_it_passes() {
        let primary = MockGate::new("primary", true);
        let fallback = MockGate::new("fallback", true);
        let fc = fallback.calls_handle();
        let gate = FallbackGate::new("fb", Box::new(primary), Box::new(fallback));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(v.passed);
        assert_eq!(fc.load(Ordering::SeqCst), 0, "fallback should not run");
    }

    #[tokio::test]
    async fn fallback_uses_fallback_on_primary_failure() {
        let primary = MockGate::new("primary", false);
        let fallback = MockGate::new("fallback", true);
        let fc = fallback.calls_handle();
        let gate = FallbackGate::new("fb", Box::new(primary), Box::new(fallback));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(v.passed, "fallback should rescue");
        assert_eq!(fc.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fallback_both_fail() {
        let primary = MockGate::new("primary", false);
        let fallback = MockGate::new("fallback", false);
        let gate = FallbackGate::new("fb", Box::new(primary), Box::new(fallback));
        let v = gate.verify(&signal(), &ctx()).await;
        assert!(!v.passed);
        assert!(v.reason.contains("primary"));
        assert!(v.reason.contains("fallback"));
    }
}
