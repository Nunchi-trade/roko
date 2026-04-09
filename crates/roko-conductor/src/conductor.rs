//! Conductor: composite `Policy` that runs all watchers and applies
//! the intervention policy to produce a single decision.
//!
//! The conductor is the reactive intelligence layer of the Roko
//! orchestrator. Each tick, it runs all watchers against the signal
//! stream, collects their outputs, and merges them via the
//! [`InterventionPolicy`](crate::interventions::InterventionPolicy).

use crate::circuit_breaker::CircuitBreaker;
use crate::interventions::{
    InterventionPolicy, Severity, WatcherOutput, WorstSeverityPolicy, outputs_to_signals,
};
use crate::watchers::{
    CompileFailRepeatWatcher, ContextWindowPressureWatcher, CostOverrunWatcher, GhostTurnWatcher,
    IterationLoopWatcher, ReviewLoopWatcher, SpecDriftWatcher, StuckPatternWatcher,
    TestFailureBudgetWatcher, TimeOverrunWatcher,
};
use roko_core::{Body, ConductorDecision, Context, Kind, Policy, Signal};

/// Tag key on intervention signals for the plan ID.
pub const PLAN_ID_TAG: &str = "plan_id";

/// The conductor: runs all watchers, applies escalation policy, tracks
/// circuit breaker state.
///
/// Implements [`Policy`] so it can be composed into larger policy chains.
///
/// # Usage
///
/// ```rust,no_run
/// use roko_conductor::Conductor;
/// use roko_core::{Context, Policy};
///
/// let conductor = Conductor::default();
/// let signals = vec![]; // your signal stream
/// let ctx = Context::now();
/// let interventions = conductor.decide(&signals, &ctx);
/// ```
pub struct Conductor {
    /// The individual watchers, stored as boxed `Policy` impls.
    watchers: Vec<Box<dyn Policy>>,
    /// Intervention escalation policy.
    policy: Box<dyn InterventionPolicy>,
    /// Per-plan circuit breaker.
    circuit_breaker: CircuitBreaker,
}

impl std::fmt::Debug for Conductor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conductor")
            .field("watcher_count", &self.watchers.len())
            .field("policy", &self.policy.name())
            .finish_non_exhaustive()
    }
}

impl Default for Conductor {
    fn default() -> Self {
        Self::new()
    }
}

impl Conductor {
    /// Create a conductor with all default watchers and the worst-severity policy.
    #[must_use]
    pub fn new() -> Self {
        let watchers: Vec<Box<dyn Policy>> = vec![
            Box::new(GhostTurnWatcher::default()),
            Box::new(ReviewLoopWatcher::default()),
            Box::new(IterationLoopWatcher::default()),
            Box::new(TestFailureBudgetWatcher::default()),
            Box::new(CompileFailRepeatWatcher::default()),
            Box::new(ContextWindowPressureWatcher::default()),
            Box::new(SpecDriftWatcher::default()),
            Box::new(CostOverrunWatcher::default()),
            Box::new(TimeOverrunWatcher::new()),
            Box::new(StuckPatternWatcher::default()),
        ];

        Self {
            watchers,
            policy: Box::new(WorstSeverityPolicy),
            circuit_breaker: CircuitBreaker::default(),
        }
    }

    /// Convenience helper for periodic watcher checks.
    ///
    /// Uses the current context and returns the intervention signals
    /// produced by the conductor for the supplied signal stream.
    #[must_use]
    pub fn check_all(&self, stream: &[Signal]) -> Vec<Signal> {
        self.decide(stream, &Context::now())
    }

    /// Create a conductor with custom watchers.
    #[must_use]
    pub fn with_watchers(watchers: Vec<Box<dyn Policy>>) -> Self {
        Self {
            watchers,
            policy: Box::new(WorstSeverityPolicy),
            circuit_breaker: CircuitBreaker::default(),
        }
    }

    /// Set a custom intervention policy.
    #[must_use]
    pub fn with_policy(mut self, policy: Box<dyn InterventionPolicy>) -> Self {
        self.policy = policy;
        self
    }

    /// Set a custom circuit breaker.
    #[must_use]
    pub fn with_circuit_breaker(mut self, cb: CircuitBreaker) -> Self {
        self.circuit_breaker = cb;
        self
    }

    /// Access the circuit breaker.
    #[must_use]
    pub const fn circuit_breaker(&self) -> &CircuitBreaker {
        &self.circuit_breaker
    }

    /// Run all watchers and produce a conductor decision.
    ///
    /// This is the core function: it runs each watcher, collects
    /// intervention signals, converts them to `WatcherOutput`s, and
    /// applies the escalation policy.
    #[must_use]
    pub fn evaluate(&self, stream: &[Signal], ctx: &Context) -> ConductorDecision {
        // Check circuit breaker first.
        if let Some(plan_id) = extract_plan_id(stream) {
            if self.circuit_breaker.is_tripped(&plan_id) {
                return ConductorDecision::fail(
                    "circuit-breaker",
                    roko_core::FailureKind::MaxIterations,
                );
            }
        }

        // Run all watchers and collect outputs.
        let watcher_outputs = collect_watcher_outputs(&self.watchers, stream, ctx);

        // Apply escalation policy.
        let decision = self.policy.evaluate(&watcher_outputs, ctx);

        // Record failures in circuit breaker.
        if let ConductorDecision::Fail { watcher, reason } = &decision {
            if let Some(plan_id) = extract_plan_id(stream) {
                self.circuit_breaker.record_failure(
                    &plan_id,
                    format!("{watcher}: {reason}"),
                    ctx.now_ms,
                );
            }
        }

        decision
    }
}

/// Extract the plan ID from the signal stream (most recent `PlanPhase` tag).
fn extract_plan_id(stream: &[Signal]) -> Option<String> {
    stream
        .iter()
        .rev()
        .find(|s| s.kind == Kind::PlanPhase)
        .and_then(|s| s.tag(PLAN_ID_TAG))
        .map(str::to_owned)
}

/// Run all watchers and collect their outputs as `WatcherOutput` values.
fn collect_watcher_outputs(
    watchers: &[Box<dyn Policy>],
    stream: &[Signal],
    ctx: &Context,
) -> Vec<WatcherOutput> {
    let mut outputs = Vec::new();
    for watcher in watchers {
        let signals = watcher.decide(stream, ctx);
        for s in &signals {
            let severity = match s.tag("severity") {
                Some("critical") => Severity::Critical,
                Some("warning") => Severity::Warning,
                _ => Severity::Info,
            };
            let watcher_name = s.tag("watcher").unwrap_or_else(|| watcher.name());
            let description = match &s.body {
                Body::Text(t) => t.clone(),
                _ => format!("intervention from {watcher_name}"),
            };
            outputs.push(WatcherOutput::new(watcher_name, severity, description));
        }
    }
    outputs
}

impl Policy for Conductor {
    fn decide(&self, stream: &[Signal], ctx: &Context) -> Vec<Signal> {
        // Run all watchers and collect outputs.
        let watcher_outputs = collect_watcher_outputs(&self.watchers, stream, ctx);

        // Convert non-info outputs to signals for the substrate.
        let mut result = outputs_to_signals(&watcher_outputs);

        // Also include the decision as a signal.
        let decision = self.policy.evaluate(&watcher_outputs, ctx);
        if !decision.is_continue() {
            if let Ok(body) = Body::from_json(&decision) {
                result.push(
                    Signal::builder(Kind::Custom("conductor.decision".into()))
                        .body(body)
                        .tag("decision", decision.label())
                        .build(),
                );
            }
        }

        result
    }

    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "conductor"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ghost_turn_signal(cost_usd: f64) -> Signal {
        Signal::builder(Kind::Custom(
            crate::watchers::ghost_turn::TURN_SIGNAL_KIND.into(),
        ))
        .body(
            Body::from_json(&serde_json::json!({
                "plan_id": "plan-1",
                "task": "task-1",
                "role": "Implementer",
                "model": "claude-sonnet-4-6",
                "cost_usd": cost_usd,
                "duration_ms": 1234,
                "changed_files_before": [],
                "changed_files_after": [],
                "net_new_changes": 0,
                "output_meaningful": false,
                "wasted_cost": true,
            }))
            .expect("serialize ghost turn event"),
        )
        .build()
    }

    fn ghost_stream(count: usize) -> Vec<Signal> {
        (0..count)
            .map(|i| ghost_turn_signal(1.0 - (i as f64 * 0.1)))
            .collect()
    }

    fn healthy_stream() -> Vec<Signal> {
        vec![
            Signal::builder(Kind::AgentOutput)
                .body(Body::text("implementing feature X"))
                .build(),
            Signal::builder(Kind::AgentOutput)
                .body(Body::text("running tests"))
                .build(),
        ]
    }

    fn plan_phase_stream(plan_id: &str) -> Vec<Signal> {
        vec![
            Signal::builder(Kind::PlanPhase)
                .body(Body::text("implementing"))
                .tag(PLAN_ID_TAG, plan_id)
                .tag("phase", "implementing")
                .tag("phase_entered_ms", "0")
                .build(),
        ]
    }

    #[test]
    fn empty_stream_continues() {
        let c = Conductor::default();
        let d = c.evaluate(&[], &Context::at(0));
        assert!(d.is_continue());
    }

    #[test]
    fn healthy_stream_continues() {
        let c = Conductor::default();
        let d = c.evaluate(&healthy_stream(), &Context::at(0));
        assert!(d.is_continue());
    }

    #[test]
    fn ghost_turns_trigger_restart() {
        let c = Conductor::default();
        let d = c.evaluate(&ghost_stream(3), &Context::at(0));
        assert_eq!(d.label(), "restart");
    }

    #[test]
    fn circuit_breaker_aborts_tripped_plan() {
        let c = Conductor::default();
        let plan_stream = plan_phase_stream("plan-1");

        // Record enough failures to trip.
        c.circuit_breaker().record_failure("plan-1", "err1", 100);
        c.circuit_breaker().record_failure("plan-1", "err2", 200);

        let d = c.evaluate(&plan_stream, &Context::at(300));
        assert!(d.is_terminal());
    }

    #[test]
    fn conductor_as_policy() {
        let c = Conductor::default();
        let signals = c.decide(&healthy_stream(), &Context::at(0));
        // Healthy stream = no interventions.
        assert!(signals.is_empty());
    }

    #[test]
    fn conductor_policy_emits_on_anomaly() {
        let c = Conductor::default();
        let signals = c.decide(&ghost_stream(3), &Context::at(0));
        // Should emit intervention + decision signals.
        assert!(!signals.is_empty());
        // At least one decision signal.
        let has_decision = signals
            .iter()
            .any(|s| s.kind == Kind::Custom("conductor.decision".into()));
        assert!(has_decision);
    }

    #[test]
    fn custom_watchers() {
        // Conductor with only the ghost-turn watcher.
        let c = Conductor::with_watchers(vec![Box::new(GhostTurnWatcher::default())]);
        let d = c.evaluate(&ghost_stream(3), &Context::at(0));
        assert_eq!(d.label(), "restart");
    }

    #[test]
    fn multiple_watchers_worst_wins() {
        // Stream that triggers both ghost-turn (warning) and iteration-loop (critical).
        let mut stream: Vec<Signal> = ghost_stream(3);
        for _ in 0..3 {
            stream.push(
                Signal::builder(Kind::GateVerdict)
                    .body(Body::Json(serde_json::json!({
                        "plan_id": "plan-1",
                        "gate": "compile",
                        "passed": false,
                    })))
                    .tag("plan_id", "plan-1")
                    .build(),
            );
            stream.push(
                Signal::builder(Kind::PlanPhase)
                    .body(Body::Json(serde_json::json!({
                        "plan_id": "plan-1",
                        "event": "GateFailed",
                    })))
                    .tag("plan_id", "plan-1")
                    .build(),
            );
        }

        let c = Conductor::default();
        let d = c.evaluate(&stream, &Context::at(0));
        // iteration-loop fires critical, which should produce a Fail.
        assert!(d.is_terminal());
    }

    #[test]
    fn watcher_count() {
        let c = Conductor::default();
        assert_eq!(c.watchers.len(), 10);
    }
}
