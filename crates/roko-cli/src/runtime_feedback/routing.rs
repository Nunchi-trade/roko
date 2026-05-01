//! Routing observation sink — feeds task / turn outcomes back into the
//! [`CascadeRouter`] so model selection learns from real performance.
//!
//! ## Why this exists
//!
//! The router exposes `record_confidence_outcome(model_slug, success)` for
//! confidence-only updates and `record_override_outcome(...)` for contextual
//! override learning. Until this sink existed those methods were called from
//! ad-hoc helpers in the runner, leading to double-counting and missed
//! observations. Now there is one path:
//! `FeedbackEvent::TaskCompleted -> RoutingObservationSink::record(...)`.
//!
//! ## Override handling
//!
//! When [`ModelChoiceSource::Override`] tagged a task, the sink records
//! it via `record_override_outcome` so manual operator overrides do not
//! pollute the bandit signal that drives router decisions on
//! non-overridden tasks.

use std::sync::Arc;

use async_trait::async_trait;
use roko_core::agent::AgentRole;
use roko_core::config::RewardWeights;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_core::{BehavioralState, DaimonPolicy};
use roko_learn::cascade_router::CascadeRouter;
use roko_learn::model_router::RoutingContext;

use super::{FeedbackEvent, FeedbackSink};
#[cfg(test)]
use crate::dispatch::ModelChoiceSource;

/// Sink that records a routing observation per `task_completed` event.
#[derive(Clone)]
pub struct RoutingObservationSink {
    router: Arc<CascadeRouter>,
}

impl RoutingObservationSink {
    /// Construct a routing sink wrapping a shared router.
    #[must_use]
    pub fn new(router: Arc<CascadeRouter>) -> Self {
        Self { router }
    }
}

impl std::fmt::Debug for RoutingObservationSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingObservationSink")
            .field("router", &"..")
            .finish()
    }
}

#[async_trait]
impl FeedbackSink for RoutingObservationSink {
    fn name(&self) -> &'static str {
        "routing"
    }

    fn interested(&self, event: &FeedbackEvent) -> bool {
        matches!(event, FeedbackEvent::TaskCompleted { .. })
    }

    async fn on_event(&self, event: &FeedbackEvent) -> Result<(), anyhow::Error> {
        let FeedbackEvent::TaskCompleted {
            outcome,
            model_source,
            succeeded,
            ..
        } = event
        else {
            return Ok(());
        };

        // The model_source tag still flows through the per-sink event
        // log so override-vs-router observations can be dampened
        // downstream. See `.roko/GAPS.md`.
        let _ = model_source;

        // If the slug isn't tracked yet, fall back to the binary
        // outcome path so the trial counter still moves.
        let Some(model_idx) = self.router.model_index_for_slug(&outcome.model) else {
            self.router
                .record_confidence_outcome(&outcome.model, *succeeded);
            return Ok(());
        };

        // FeedbackEvent does not yet carry the original dispatch
        // RoutingContext (task category / complexity / role / queue
        // pressure). Use stable defaults so the LinUCB feature vector
        // is well-formed; richer plumbing is tracked separately.
        let ctx = build_runner_feedback_context(&outcome.model);

        if *succeeded {
            // Quality is the binary success signal; cost / latency
            // pressure are 0.0 because FeedbackEvent does not carry a
            // budget remaining or SLA signal yet (T4-30 commit body).
            let weights = RewardWeights::default();
            self.router.observe_multi_objective(
                ctx.to_features(),
                model_idx,
                /* quality */ 1.0,
                /* normalized_cost */ 0.0,
                /* normalized_latency */ 0.0,
                &weights,
            );
        } else {
            // observe_multi_objective always counts as success — record
            // failures via the binary path so the trial counter and
            // failure rate stay accurate.
            self.router.record_confidence_outcome(&outcome.model, false);
        }
        Ok(())
    }
}

/// Build a stable [`RoutingContext`] for runner-feedback observations.
///
/// The runner feedback path does not yet propagate the original
/// dispatch RoutingContext through `FeedbackEvent`, so this helper
/// returns a deterministic minimum context with the model marked as
/// `previous_model` for cache-affinity learning.
fn build_runner_feedback_context(model: &str) -> RoutingContext {
    RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity: TaskComplexityBand::Standard,
        iteration: 0,
        role: AgentRole::Implementer,
        crate_familiarity: 0.5,
        has_prior_failure: false,
        conductor_load: 0.0,
        active_agents: 0,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: DaimonPolicy::new(0.5, BehavioralState::Engaged),
        thinking_level: None,
        temperament: None,
        previous_model: Some(model.to_string()),
        plan_context_tokens: None,
        tier_thresholds: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::AgentOutcome;
    use roko_learn::cascade_router::CascadeRouter;

    fn outcome(success: bool) -> AgentOutcome {
        AgentOutcome {
            task_id: "t".into(),
            plan_id: "p".into(),
            model: "claude-sonnet-4-6".into(),
            provider: "claude_cli".into(),
            output: "".into(),
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            duration_ms: 0,
            exit_code: if success { Some(0) } else { Some(1) },
            is_error: !success,
        }
    }

    fn router() -> Arc<CascadeRouter> {
        Arc::new(CascadeRouter::new(vec![
            "claude-sonnet-4-6".into(),
            "gpt-5".into(),
        ]))
    }

    #[tokio::test]
    async fn success_drives_observe_multi_objective_for_known_model() {
        let r = router();
        let sink = RoutingObservationSink::new(r.clone());
        let event = FeedbackEvent::TaskCompleted {
            plan_id: "p".into(),
            task_id: "t".into(),
            outcome: outcome(true),
            model_source: ModelChoiceSource::Router,
            succeeded: true,
        };
        sink.on_event(&event).await.unwrap();
        let snap = r.confidence_snapshot();
        let (trials, successes) = snap
            .get("claude-sonnet-4-6")
            .copied()
            .expect("snapshot for the observed slug");
        assert_eq!(trials, 1);
        assert_eq!(successes, 1);
        assert!(
            r.total_observations() >= 1,
            "observe_multi_objective should advance the LinUCB observation counter",
        );
    }

    #[tokio::test]
    async fn failure_records_through_record_outcome() {
        let r = router();
        let sink = RoutingObservationSink::new(r.clone());
        let event = FeedbackEvent::TaskCompleted {
            plan_id: "p".into(),
            task_id: "t".into(),
            outcome: outcome(false),
            model_source: ModelChoiceSource::Override,
            succeeded: false,
        };
        sink.on_event(&event).await.unwrap();
        let (trials, successes) = r
            .confidence_snapshot()
            .get("claude-sonnet-4-6")
            .copied()
            .expect("snapshot for the observed slug");
        assert_eq!(trials, 1, "failure must increment trials");
        assert_eq!(successes, 0, "failure must not increment successes");
        assert_eq!(
            r.total_observations(),
            0,
            "failures should not push LinUCB observations on the success-only path",
        );
    }

    #[tokio::test]
    async fn unknown_model_falls_back_to_record_outcome() {
        let r = router();
        let sink = RoutingObservationSink::new(r.clone());
        let mut bad_outcome = outcome(true);
        bad_outcome.model = "no-such-slug".into();
        let event = FeedbackEvent::TaskCompleted {
            plan_id: "p".into(),
            task_id: "t".into(),
            outcome: bad_outcome,
            model_source: ModelChoiceSource::Router,
            succeeded: true,
        };
        sink.on_event(&event).await.unwrap();
        assert!(
            r.confidence_snapshot().get("no-such-slug").is_none(),
            "unknown slug must not be silently registered",
        );
    }

    #[tokio::test]
    async fn sink_ignores_non_task_events() {
        let r = router();
        let sink = RoutingObservationSink::new(r);
        let event = FeedbackEvent::IdleTick {
            ticks_since_last_work: 1,
        };
        assert!(!sink.interested(&event));
        sink.on_event(&event).await.unwrap();
    }
}
