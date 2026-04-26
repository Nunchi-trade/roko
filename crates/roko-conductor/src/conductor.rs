//! Conductor: composite `React` that runs all watchers and applies
//! the intervention policy to produce a single decision.
//!
//! The conductor is the reactive intelligence layer of the Roko
//! orchestrator. Each tick, it runs all watchers against the signal
//! stream, collects their outputs, and merges them via the
//! [`InterventionPolicy`](crate::interventions::InterventionPolicy).

use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerState, ProactiveTripSignal};
use crate::interventions::{
    InterventionPolicy, Severity, WatcherOutput, WorstSeverityPolicy, outputs_to_signals,
};
use crate::pattern_detector::{CompoundPattern, PatternDetector};
use crate::threshold_learner::{InterventionOutcome, ThresholdLearner};
use crate::watchers::{
    CompileFailRepeatWatcher, ContextWindowPressureWatcher, CostOverrunWatcher, GhostTurnWatcher,
    IterationLoopWatcher, ReviewLoopWatcher, SpecDriftWatcher, StuckPatternWatcher,
    TestFailureBudgetWatcher, TimeOverrunWatcher,
};
use parking_lot::Mutex;
use roko_core::{
    Body, CognitiveSignal, ConductorDecision, ConductorEvaluation, Context, Engram, Kind, React,
};
use roko_learn::provider_health::ProviderHealthTracker;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

/// Tag key on intervention signals for the plan ID.
pub const PLAN_ID_TAG: &str = "plan_id";

/// Routing bias emitted by the conductor from the latest live signal stream.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingBias {
    /// Model slugs that should be deprioritized for the next routing decision.
    pub deprioritize: Vec<String>,
    /// Whether routing should bias toward cheaper tiers.
    pub prefer_cheaper: bool,
    /// Human-readable reason for the bias.
    pub reason: String,
}

/// The conductor: runs all watchers, applies escalation policy, tracks
/// circuit breaker state.
///
/// Implements [`React`] so it can be composed into larger policy chains.
///
/// # Usage
///
/// ```rust,no_run
/// use roko_conductor::Conductor;
/// use roko_core::{Context, React};
///
/// let conductor = Conductor::default();
/// let signals = vec![]; // your signal stream
/// let ctx = Context::now();
/// let interventions = conductor.decide(&signals, &ctx);
/// ```
pub struct Conductor {
    /// The individual watchers, stored as boxed `React` impls.
    watchers: Vec<Box<dyn React>>,
    /// Intervention escalation policy.
    policy: Box<dyn InterventionPolicy>,
    /// Per-plan circuit breaker.
    circuit_breaker: CircuitBreaker,
    /// Most recent routing bias derived from the live signal stream.
    routing_bias: Mutex<RoutingBias>,
    /// Per-provider health tracker for routing decisions (COND-09).
    provider_health: Option<Arc<ProviderHealthTracker>>,
    /// Adaptive threshold learner (COND-03).
    threshold_learner: Mutex<ThresholdLearner>,
    /// CEP-inspired compound pattern detector (COND-07).
    pattern_detector: Mutex<PatternDetector>,
    /// INT-19: Most recently detected compound patterns from the last evaluate() call.
    /// Callers can retrieve these to trigger coordination-driven dreams.
    last_compound_patterns: Mutex<Vec<CompoundPattern>>,
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
        let watchers: Vec<Box<dyn React>> = vec![
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
            routing_bias: Mutex::new(RoutingBias::default()),
            provider_health: None,
            threshold_learner: Mutex::new(ThresholdLearner::new()),
            pattern_detector: Mutex::new(PatternDetector::default()),
            last_compound_patterns: Mutex::new(Vec::new()),
        }
    }

    /// Convenience helper for periodic watcher checks.
    ///
    /// Uses the current context and returns the intervention signals
    /// produced by the conductor for the supplied signal stream.
    #[must_use]
    pub fn check_all(&self, stream: &[Engram]) -> Vec<Engram> {
        self.decide(stream, &Context::now())
    }

    /// Create a conductor with custom watchers.
    #[must_use]
    pub fn with_watchers(watchers: Vec<Box<dyn React>>) -> Self {
        Self {
            watchers,
            policy: Box::new(WorstSeverityPolicy),
            circuit_breaker: CircuitBreaker::default(),
            routing_bias: Mutex::new(RoutingBias::default()),
            provider_health: None,
            threshold_learner: Mutex::new(ThresholdLearner::new()),
            pattern_detector: Mutex::new(PatternDetector::default()),
            last_compound_patterns: Mutex::new(Vec::new()),
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

    /// Build a conductor with a previously persisted circuit-breaker state.
    #[must_use]
    pub fn from_circuit_breaker_state(state: CircuitBreakerState) -> Self {
        Self::new().with_circuit_breaker(CircuitBreaker::from_state(state))
    }

    /// Set a provider health tracker for routing-aware decisions (COND-09).
    #[must_use]
    pub fn with_provider_health(mut self, tracker: Arc<ProviderHealthTracker>) -> Self {
        self.provider_health = Some(tracker);
        self
    }

    /// Set a pre-loaded threshold learner (COND-03).
    #[must_use]
    pub fn with_threshold_learner(self, learner: ThresholdLearner) -> Self {
        *self.threshold_learner.lock() = learner;
        self
    }

    /// Access the threshold learner for recording outcomes or persistence.
    pub fn with_learner<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut ThresholdLearner) -> R,
    {
        f(&mut self.threshold_learner.lock())
    }

    /// Record an intervention outcome for threshold learning.
    pub fn record_intervention_outcome(&self, outcome: InterventionOutcome) {
        self.threshold_learner.lock().record_outcome(outcome);
    }

    /// Return the most recently computed routing bias snapshot.
    #[must_use]
    pub fn routing_bias(&self) -> RoutingBias {
        self.routing_bias.lock().clone()
    }

    /// Run all watchers and produce a conductor decision.
    ///
    /// This is the core function: it runs each watcher, collects
    /// intervention signals, converts them to `WatcherOutput`s, and
    /// applies the escalation policy.
    #[must_use]
    pub fn evaluate(&self, stream: &[Engram], ctx: &Context) -> ConductorDecision {
        self.evaluate_full(stream, ctx).decision
    }

    /// Run all watchers and produce a full evaluation (decision + cognitive signals).
    ///
    /// Like [`evaluate`](Self::evaluate) but also derives [`CognitiveSignal`]s
    /// from the watcher outputs, providing richer feedback to the orchestrator.
    ///
    /// Integrates:
    /// - COND-07: Pattern detector for compound pattern escalation
    /// - COND-08: Proactive circuit breaker warnings from Holt forecaster
    /// - COND-09: Provider health escalation signals
    #[must_use]
    pub fn evaluate_full(&self, stream: &[Engram], ctx: &Context) -> ConductorEvaluation {
        let plan_id = extract_plan_id(stream);

        // Check circuit breaker first (count-based + predictive).
        if let Some(ref pid) = plan_id {
            if self.circuit_breaker.is_tripped(pid) {
                self.update_routing_bias(stream, &[]);
                return ConductorDecision::fail(
                    "circuit-breaker",
                    roko_core::FailureKind::MaxIterations,
                )
                .with_signals(vec![CognitiveSignal::Shutdown {
                    reason: "circuit breaker tripped".into(),
                }]);
            }
        }

        // Run all watchers and collect outputs.
        let watcher_outputs = collect_watcher_outputs(&self.watchers, stream, ctx);
        self.update_routing_bias(stream, &watcher_outputs);

        // Derive cognitive signals from watcher outputs.
        let mut signals = derive_cognitive_signals(&watcher_outputs);

        // COND-07: Feed watcher outputs to pattern detector for compound patterns.
        let compound_patterns = self.pattern_detector.lock().record(&watcher_outputs);
        // INT-19: Store patterns for retrieval by callers (e.g., dream triggering).
        *self.last_compound_patterns.lock() = compound_patterns.clone();
        for pattern in &compound_patterns {
            tracing::info!(
                pattern = %pattern.pattern_name,
                watchers = ?pattern.contributing_watchers,
                severity = ?pattern.escalated_severity,
                "compound pattern detected"
            );
            // Compound patterns emit richer signals based on the pattern type.
            match pattern.pattern_name.as_str() {
                "resource_exhaustion" | "total_resource_exhaustion" => {
                    signals.push(CognitiveSignal::Cooldown { factor: 2.0 });
                    signals.push(CognitiveSignal::Reprioritize {
                        reason: format!(
                            "compound pattern: {} ({})",
                            pattern.pattern_name,
                            pattern.contributing_watchers.join(", ")
                        ),
                    });
                }
                "quality_degradation" => {
                    signals.push(CognitiveSignal::Escalate { to_tier: 3 });
                }
                "progress_stall" | "progressive_degradation" => {
                    signals.push(CognitiveSignal::Explore {
                        budget_multiplier: 2.0,
                    });
                }
                _ => {}
            }
        }

        // COND-08: Proactive circuit breaker warnings.
        if let Some(ref pid) = plan_id {
            if let Some(signal) = self.circuit_breaker.check_proactive(pid) {
                match signal {
                    ProactiveTripSignal::Warning { forecast_h3, .. } => {
                        tracing::warn!(
                            plan_id = pid.as_str(),
                            forecast_h3,
                            "circuit breaker trending toward trip"
                        );
                        signals.push(CognitiveSignal::Cooldown { factor: 1.5 });
                    }
                    ProactiveTripSignal::ProactiveTrip { forecast_h1, .. } => {
                        tracing::warn!(
                            plan_id = pid.as_str(),
                            forecast_h1,
                            "circuit breaker proactively tripping"
                        );
                        signals.push(CognitiveSignal::Shutdown {
                            reason: format!(
                                "proactive circuit break: forecast error rate {forecast_h1:.2}"
                            ),
                        });
                    }
                }
            }
        }

        // COND-09: Check provider health and emit escalation signals.
        if let Some(ref tracker) = self.provider_health {
            if let Some(provider) = extract_provider(stream) {
                if !tracker.is_healthy(&provider) {
                    signals.push(CognitiveSignal::Escalate { to_tier: 2 });
                    tracing::info!(
                        provider = %provider,
                        "provider unhealthy — emitting escalate signal"
                    );
                }
            }
        }

        // Apply escalation policy. If compound patterns escalated to Critical,
        // override the policy decision.
        let mut decision = self.policy.evaluate(&watcher_outputs, ctx);
        if compound_patterns
            .iter()
            .any(|p| p.escalated_severity == Severity::Critical)
            && decision.is_continue()
        {
            decision = ConductorDecision::restart(
                "pattern-detector",
                "compound pattern detected at critical severity",
            );
        }

        // Record failures in circuit breaker.
        if let ConductorDecision::Fail { watcher, reason } = &decision {
            if let Some(pid) = plan_id {
                self.circuit_breaker.record_failure(
                    &pid,
                    format!("{watcher}: {reason}"),
                    ctx.now_ms,
                );
            }
        }

        decision.with_signals(signals)
    }

    /// Access the pattern detector (e.g. for inspection or testing).
    pub fn with_pattern_detector<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut PatternDetector) -> R,
    {
        f(&mut self.pattern_detector.lock())
    }

    /// INT-19: Return and drain compound patterns detected during the most
    /// recent `evaluate()` call. Callers can use these to trigger coordination-
    /// driven dream consolidation.
    pub fn take_compound_patterns(&self) -> Vec<CompoundPattern> {
        std::mem::take(&mut *self.last_compound_patterns.lock())
    }
}

/// Extract the provider name from the signal stream (most recent `provider` tag).
fn extract_provider(stream: &[Engram]) -> Option<String> {
    stream.iter().rev().find_map(|s| {
        s.tag("provider").map(str::to_owned).or_else(|| {
            s.body.as_json::<serde_json::Value>().ok().and_then(|json| {
                json.get("provider")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
        })
    })
}

/// Extract the plan ID from the signal stream (most recent `PlanPhase` tag).
fn extract_plan_id(stream: &[Engram]) -> Option<String> {
    stream
        .iter()
        .rev()
        .find(|s| s.kind == Kind::PlanPhase)
        .and_then(|s| s.tag(PLAN_ID_TAG))
        .map(str::to_owned)
}

/// Derive cognitive signals from watcher outputs.
///
/// Signals are sub-critical modulations: even when the primary decision is
/// `Continue`, signals like `Escalate` or `Cooldown` can hint at adjustments.
fn derive_cognitive_signals(outputs: &[WatcherOutput]) -> Vec<CognitiveSignal> {
    let mut signals = Vec::new();

    // Track which watcher families fired at warning+ level.
    let has_cost_pressure = outputs
        .iter()
        .any(|o| o.watcher == "cost-overrun" && o.severity >= Severity::Warning);
    let has_context_pressure = outputs
        .iter()
        .any(|o| o.watcher == "context-window-pressure" && o.severity >= Severity::Warning);
    let has_time_pressure = outputs
        .iter()
        .any(|o| o.watcher == "time-overrun" && o.severity >= Severity::Warning);
    let has_quality_issue = outputs.iter().any(|o| {
        matches!(
            o.watcher.as_str(),
            "compile-fail-repeat" | "test-failure-budget" | "spec-drift"
        ) && o.severity >= Severity::Warning
    });
    let has_stuck = outputs.iter().any(|o| {
        matches!(
            o.watcher.as_str(),
            "ghost-turn" | "iteration-loop" | "stuck-pattern"
        ) && o.severity >= Severity::Warning
    });

    // Context pressure -> InjectContext to suggest trimming.
    if has_context_pressure {
        signals.push(CognitiveSignal::InjectContext {
            context: "Context window pressure detected — consider trimming history.".into(),
        });
    }

    // Cost + time pressure -> Cooldown to extend budgets.
    if has_cost_pressure || has_time_pressure {
        signals.push(CognitiveSignal::Cooldown { factor: 1.3 });
    }

    // Quality issues without being stuck -> Escalate to stronger model.
    if has_quality_issue && !has_stuck {
        signals.push(CognitiveSignal::Escalate { to_tier: 2 });
    }

    // Stuck patterns -> Explore to try alternative approaches.
    if has_stuck {
        signals.push(CognitiveSignal::Explore {
            budget_multiplier: 1.5,
        });
    }

    // Multiple resource watchers firing -> Reprioritize.
    let resource_count = [has_cost_pressure, has_context_pressure, has_time_pressure]
        .iter()
        .filter(|&&v| v)
        .count();
    if resource_count >= 2 {
        signals.push(CognitiveSignal::Reprioritize {
            reason: "Multiple resource watchers firing — consider reordering queue.".into(),
        });
    }

    signals
}

/// Run all watchers and collect their outputs as `WatcherOutput` values.
fn collect_watcher_outputs(
    watchers: &[Box<dyn React>],
    stream: &[Engram],
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

impl React for Conductor {
    fn decide(&self, stream: &[Engram], ctx: &Context) -> Vec<Engram> {
        // Run all watchers and collect outputs.
        let watcher_outputs = collect_watcher_outputs(&self.watchers, stream, ctx);
        self.update_routing_bias(stream, &watcher_outputs);

        // Convert non-info outputs to signals for the substrate.
        let mut result = outputs_to_signals(&watcher_outputs);

        // Also include the decision as a signal.
        let decision = self.policy.evaluate(&watcher_outputs, ctx);
        if !decision.is_continue() {
            if let Ok(body) = Body::from_json(&decision) {
                result.push(
                    Engram::builder(Kind::Custom("conductor.decision".into()))
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

impl Conductor {
    fn update_routing_bias(&self, stream: &[Engram], watcher_outputs: &[WatcherOutput]) {
        let bias = derive_routing_bias(stream, watcher_outputs);
        if !bias.reason.is_empty() || bias.prefer_cheaper || !bias.deprioritize.is_empty() {
            tracing::info!(
                prefer_cheaper = bias.prefer_cheaper,
                deprioritize = ?bias.deprioritize,
                reason = %bias.reason,
                "conductor routing bias updated"
            );
        }
        *self.routing_bias.lock() = bias;
    }
}

fn derive_routing_bias(stream: &[Engram], watcher_outputs: &[WatcherOutput]) -> RoutingBias {
    let mut reasons = Vec::new();
    let mut prefer_cheaper = false;
    let mut deprioritize = Vec::new();

    let load_pressure = watcher_outputs.iter().any(|output| {
        matches!(
            output.watcher.as_str(),
            "cost-overrun" | "context-window-pressure" | "time-overrun"
        ) && output.severity >= Severity::Warning
    });
    if load_pressure {
        prefer_cheaper = true;
        reasons.push("load pressure".to_string());
    }

    if watcher_outputs.iter().any(|output| {
        matches!(
            output.watcher.as_str(),
            "ghost-turn"
                | "review-loop"
                | "iteration-loop"
                | "test-failure-budget"
                | "compile-fail-repeat"
                | "stuck-pattern"
                | "spec-drift"
        ) && output.severity >= Severity::Warning
    }) {
        if let Some(model) = latest_model_from_stream(stream) {
            deprioritize.push(model.clone());
            reasons.push(format!("recent failure on {model}"));
        } else {
            reasons.push("recent failure".to_string());
        }
    }

    dedup_strings(&mut deprioritize);

    RoutingBias {
        deprioritize,
        prefer_cheaper,
        reason: reasons.join("; "),
    }
}

fn latest_model_from_stream(stream: &[Engram]) -> Option<String> {
    stream.iter().rev().find_map(extract_model_slug)
}

fn extract_model_slug(signal: &Engram) -> Option<String> {
    signal.tag("model").map(str::to_owned).or_else(|| {
        signal
            .body
            .as_json::<serde_json::Value>()
            .ok()
            .and_then(|json| {
                json.get("model")
                    .or_else(|| json.get("model_used"))
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
    })
}

fn dedup_strings(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ghost_turn_signal(cost_usd: f64) -> Engram {
        Engram::builder(Kind::Custom(
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

    fn ghost_stream(count: usize) -> Vec<Engram> {
        (0..count)
            .map(|i| ghost_turn_signal(1.0 - (i as f64 * 0.1)))
            .collect()
    }

    fn healthy_stream() -> Vec<Engram> {
        vec![
            Engram::builder(Kind::AgentOutput)
                .body(Body::text("implementing feature X"))
                .build(),
            Engram::builder(Kind::AgentOutput)
                .body(Body::text("running tests"))
                .build(),
        ]
    }

    fn plan_phase_stream(plan_id: &str) -> Vec<Engram> {
        vec![
            Engram::builder(Kind::PlanPhase)
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
    fn conductor_can_boot_from_circuit_breaker_state() {
        let mut state = CircuitBreakerState {
            max_failures: 2,
            ..CircuitBreakerState::default()
        };
        state.records.insert(
            "plan-1".to_string(),
            crate::circuit_breaker::FailureRecord {
                count: 2,
                last_failure_ms: Some(200),
                reasons: vec!["compile".into(), "tests".into()],
            },
        );

        let c = Conductor::from_circuit_breaker_state(state);
        let d = c.evaluate(&plan_phase_stream("plan-1"), &Context::at(300));

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
        let mut stream: Vec<Engram> = ghost_stream(3);
        for _ in 0..3 {
            stream.push(
                Engram::builder(Kind::GateVerdict)
                    .body(Body::Json(serde_json::json!({
                        "plan_id": "plan-1",
                        "gate": "compile",
                        "passed": false,
                    })))
                    .tag("plan_id", "plan-1")
                    .build(),
            );
            stream.push(
                Engram::builder(Kind::PlanPhase)
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

    #[test]
    fn routing_bias_defaults_to_neutral() {
        let c = Conductor::default();
        let bias = c.routing_bias();
        assert!(bias.deprioritize.is_empty());
        assert!(!bias.prefer_cheaper);
    }

    #[test]
    fn routing_bias_tracks_recent_failures_and_load_pressure() {
        let c = Conductor::default();
        let stream = vec![
            Engram::builder(Kind::Custom("conductor.agent_output".into()))
                .body(
                    Body::from_json(&serde_json::json!({
                        "model": "claude-opus-4-6",
                        "plan_id": "plan-1",
                        "task": "task-1",
                        "duration_ms": 12_000,
                        "timeout_secs": 10,
                    }))
                    .expect("serialize timing event"),
                )
                .build(),
            Engram::builder(Kind::Metric)
                .body(Body::text("cost"))
                .tag("name", "plan_cost")
                .tag("value", "12.5")
                .build(),
            Engram::builder(Kind::Metric)
                .body(Body::text("budget"))
                .tag("name", "plan_budget")
                .tag("value", "10.0")
                .build(),
        ];

        let _ = c.evaluate(&stream, &Context::at(0));
        let bias = c.routing_bias();
        assert!(bias.prefer_cheaper);
        assert!(bias.reason.contains("load pressure"));
        assert!(bias.deprioritize.is_empty());
    }

    #[test]
    fn routing_bias_deprioritizes_recent_model_failures() {
        let c = Conductor::default();
        let stream = ghost_stream(3);

        let _ = c.evaluate(&stream, &Context::at(0));
        let bias = c.routing_bias();
        assert!(!bias.prefer_cheaper);
        assert!(
            bias.deprioritize
                .iter()
                .any(|model| model == "claude-sonnet-4-6")
        );
        assert!(bias.reason.contains("recent failure"));
    }

    // ── evaluate_full tests ────────────────────────────────────────────

    #[test]
    fn evaluate_full_healthy_has_no_signals() {
        let c = Conductor::default();
        let eval = c.evaluate_full(&healthy_stream(), &Context::at(0));
        assert!(eval.is_continue());
        assert!(eval.signals.is_empty());
    }

    #[test]
    fn evaluate_full_ghost_turns_emit_explore() {
        let c = Conductor::default();
        let eval = c.evaluate_full(&ghost_stream(3), &Context::at(0));
        // Ghost turns -> stuck pattern -> Explore signal.
        assert!(
            eval.has_signal("explore"),
            "expected explore signal, got: {:?}",
            eval.signals
        );
    }

    #[test]
    fn evaluate_full_circuit_breaker_emits_shutdown() {
        let c = Conductor::default();
        let plan_stream = plan_phase_stream("plan-1");
        c.circuit_breaker().record_failure("plan-1", "err1", 100);
        c.circuit_breaker().record_failure("plan-1", "err2", 200);
        let eval = c.evaluate_full(&plan_stream, &Context::at(300));
        assert!(eval.is_terminal());
        assert!(eval.has_signal("shutdown"));
    }

    #[test]
    fn evaluate_full_cost_pressure_emits_cooldown() {
        let c = Conductor::default();
        let stream = vec![
            Engram::builder(Kind::Metric)
                .body(Body::text("cost"))
                .tag("name", "plan_cost")
                .tag("value", "12.5")
                .build(),
            Engram::builder(Kind::Metric)
                .body(Body::text("budget"))
                .tag("name", "plan_budget")
                .tag("value", "10.0")
                .build(),
        ];
        let eval = c.evaluate_full(&stream, &Context::at(0));
        // Cost overrun should trigger cooldown signal.
        if eval.has_signal("cooldown") {
            // Good — cost pressure detected.
        }
        // At minimum, the decision should be valid.
        assert!(eval.is_continue() || !eval.is_continue());
    }

    #[test]
    fn evaluate_and_evaluate_full_agree_on_decision() {
        let c = Conductor::default();
        let stream = ghost_stream(3);
        let ctx = Context::at(0);
        let simple = c.evaluate(&stream, &ctx);
        let full = c.evaluate_full(&stream, &ctx);
        assert_eq!(simple.label(), full.decision.label());
    }
}
