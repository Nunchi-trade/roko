//! Runtime feedback — single facade that fans runner events out to every
//! learning and knowledge sink.
//!
//! ## Architectural role
//!
//! The runner observes execution; the feedback layer is what closes the
//! cybernetic loop. Per the unified Roko model:
//!
//! - **Substrate** writes durable observations (episodes, efficiency).
//! - **Router** updates its state from observed outcomes.
//! - **Policy** (knowledge / dreams) consolidates observations into
//!   learned priors.
//!
//! Without this facade each subsystem would need its own listener on the
//! runner, leading to drift, inconsistent timing, and silent failures.
//! With it, the runner emits a single [`FeedbackEvent`] and the facade
//! decides who needs it.
//!
//! ## Design
//!
//! - [`FeedbackEvent`] — a small, provider-neutral event vocabulary.
//! - [`FeedbackSink`] — async trait every sink implements.
//! - [`FeedbackFacade`] — composes a list of sinks; `on_event` fans out.
//!
//! Errors are *contained per sink*: a failing sink does not abort the
//! event distribution. Errors surface through tracing and a per-sink
//! counter so observability can flag stuck subsystems.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

pub mod episodes;
pub mod knowledge;
pub mod routing;

pub use episodes::EpisodeSink;
pub use knowledge::{KnowledgeIngestionSink, KnowledgeIngestor, NeuroKnowledgeIngestor};
pub use routing::RoutingObservationSink;

use roko_learn::model_router::RoutingContext;

use crate::dispatch::{AgentOutcome, ModelChoiceSource};

// ─── Event vocabulary ──────────────────────────────────────────────────

/// Provider-neutral event vocabulary the runner emits to feedback sinks.
///
/// Variants are *what happened*, not *what to do*. The fan-out logic in
/// [`FeedbackFacade::on_event`] decides which sinks care about which
/// events.
#[derive(Debug, Clone)]
pub enum FeedbackEvent {
    /// One agent turn completed inside a still-running task.
    TurnCompleted {
        plan_id: String,
        task_id: String,
        attempt: u32,
        tokens_in: u64,
        tokens_out: u64,
        cost_usd: f64,
    },
    /// One task attempt completed (success or terminal failure).
    TaskCompleted {
        plan_id: String,
        task_id: String,
        outcome: AgentOutcome,
        model_source: ModelChoiceSource,
        succeeded: bool,
        /// The dispatch-time routing context, when available. Enables the
        /// routing sink to feed real task features into the LinUCB bandit.
        routing_context: Option<RoutingContext>,
    },
    /// A gate verdict landed for a task.
    GateOutcome {
        plan_id: String,
        task_id: String,
        rung: u32,
        passed: bool,
        duration_ms: u64,
    },
    /// A retry decision was made for a task.
    RetryDecision {
        plan_id: String,
        task_id: String,
        attempt: u32,
        backoff_secs: u64,
    },
    /// A plan completed (success or terminal failure).
    PlanCompleted {
        plan_id: String,
        succeeded: bool,
        tasks_completed: usize,
        tasks_failed: usize,
        total_cost_usd: f64,
    },
    /// Idle tick — used by dream / consolidation triggers when no other
    /// work is in flight.
    IdleTick { ticks_since_last_work: u32 },
}

impl FeedbackEvent {
    /// Stable category label for routing decisions inside the facade.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::TurnCompleted { .. } => "turn_completed",
            Self::TaskCompleted { .. } => "task_completed",
            Self::GateOutcome { .. } => "gate_outcome",
            Self::RetryDecision { .. } => "retry_decision",
            Self::PlanCompleted { .. } => "plan_completed",
            Self::IdleTick { .. } => "idle_tick",
        }
    }
}

// ─── Sink trait ────────────────────────────────────────────────────────

/// A feedback sink consumes [`FeedbackEvent`]s.
///
/// Sinks are **best-effort**: a failing sink is logged and counted but
/// must not block other sinks. Implementors should swallow recoverable
/// errors and return `Err` only on terminal misconfiguration the
/// operator must see.
#[async_trait]
pub trait FeedbackSink: Send + Sync + std::fmt::Debug {
    /// Stable name for diagnostics (`"episodes"`, `"routing"`, ...).
    fn name(&self) -> &'static str;

    /// Whether this sink is interested in `event`. Defaults to `true`
    /// (consume everything). Sinks that only care about a subset
    /// override this for efficiency.
    fn interested(&self, _event: &FeedbackEvent) -> bool {
        true
    }

    /// Consume one event.
    async fn on_event(&self, event: &FeedbackEvent) -> Result<(), anyhow::Error>;
}

// ─── Facade ────────────────────────────────────────────────────────────

/// Per-sink delivery counters surfaced through the projection layer.
#[derive(Debug, Default)]
struct SinkStats {
    delivered: AtomicU64,
    skipped: AtomicU64,
    failed: AtomicU64,
}

/// Composes a list of sinks and fans events to each one.
#[derive(Debug)]
pub struct FeedbackFacade {
    sinks: Vec<(Arc<dyn FeedbackSink>, Arc<SinkStats>)>,
}

/// Snapshot of per-sink delivery counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacadeStats {
    pub per_sink: Vec<SinkStatsSnapshot>,
}

/// Snapshot of one sink's counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkStatsSnapshot {
    pub name: &'static str,
    pub delivered: u64,
    pub skipped: u64,
    pub failed: u64,
}

impl Default for FeedbackFacade {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedbackFacade {
    /// Empty facade — sinks added via `with_sink`.
    #[must_use]
    pub fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    /// Builder-style sink registration.
    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn FeedbackSink>) -> Self {
        self.sinks.push((sink, Arc::new(SinkStats::default())));
        self
    }

    /// Number of registered sinks.
    #[must_use]
    pub fn sink_count(&self) -> usize {
        self.sinks.len()
    }

    /// Fan an event out to every interested sink.
    ///
    /// Errors are caught per-sink and counted. The function returns
    /// `Ok(())` unless every sink failed; in that case the first error
    /// is surfaced so the operator sees something rather than silence.
    pub async fn on_event(&self, event: &FeedbackEvent) -> Result<(), anyhow::Error> {
        let mut last_err: Option<anyhow::Error> = None;
        let mut delivered = 0_u64;
        for (sink, stats) in &self.sinks {
            if !sink.interested(event) {
                stats.skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            match sink.on_event(event).await {
                Ok(()) => {
                    stats.delivered.fetch_add(1, Ordering::Relaxed);
                    delivered += 1;
                }
                Err(err) => {
                    stats.failed.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        sink = sink.name(),
                        event = event.label(),
                        %err,
                        "feedback sink reported error"
                    );
                    last_err = Some(err);
                }
            }
        }
        if delivered == 0 {
            if let Some(err) = last_err {
                return Err(err);
            }
        }
        Ok(())
    }

    /// Snapshot per-sink counters.
    #[must_use]
    pub fn stats(&self) -> FacadeStats {
        FacadeStats {
            per_sink: self
                .sinks
                .iter()
                .map(|(sink, stats)| SinkStatsSnapshot {
                    name: sink.name(),
                    delivered: stats.delivered.load(Ordering::Relaxed),
                    skipped: stats.skipped.load(Ordering::Relaxed),
                    failed: stats.failed.load(Ordering::Relaxed),
                })
                .collect(),
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[derive(Debug)]
    struct CountingSink {
        name: &'static str,
        seen: AtomicU32,
        only: Option<&'static str>,
        fail_on: Option<&'static str>,
    }

    #[async_trait]
    impl FeedbackSink for CountingSink {
        fn name(&self) -> &'static str {
            self.name
        }
        fn interested(&self, event: &FeedbackEvent) -> bool {
            match self.only {
                Some(label) => event.label() == label,
                None => true,
            }
        }
        async fn on_event(&self, event: &FeedbackEvent) -> Result<(), anyhow::Error> {
            if let Some(fail_label) = self.fail_on {
                if event.label() == fail_label {
                    anyhow::bail!("forced failure");
                }
            }
            self.seen.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn turn() -> FeedbackEvent {
        FeedbackEvent::TurnCompleted {
            plan_id: "p".into(),
            task_id: "t".into(),
            attempt: 0,
            tokens_in: 1,
            tokens_out: 1,
            cost_usd: 0.001,
        }
    }

    fn plan_done() -> FeedbackEvent {
        FeedbackEvent::PlanCompleted {
            plan_id: "p".into(),
            succeeded: true,
            tasks_completed: 1,
            tasks_failed: 0,
            total_cost_usd: 0.001,
        }
    }

    #[tokio::test]
    async fn fanout_visits_every_interested_sink() {
        let s1 = Arc::new(CountingSink {
            name: "all",
            seen: AtomicU32::new(0),
            only: None,
            fail_on: None,
        });
        let s2 = Arc::new(CountingSink {
            name: "plan-only",
            seen: AtomicU32::new(0),
            only: Some("plan_completed"),
            fail_on: None,
        });
        let facade = FeedbackFacade::new()
            .with_sink(s1.clone())
            .with_sink(s2.clone());

        facade.on_event(&turn()).await.unwrap();
        facade.on_event(&plan_done()).await.unwrap();

        assert_eq!(s1.seen.load(Ordering::Relaxed), 2, "all sink sees both");
        assert_eq!(
            s2.seen.load(Ordering::Relaxed),
            1,
            "plan-only sink sees one"
        );
    }

    #[tokio::test]
    async fn failing_sink_does_not_block_others_when_at_least_one_succeeds() {
        let s1 = Arc::new(CountingSink {
            name: "succeeds",
            seen: AtomicU32::new(0),
            only: None,
            fail_on: None,
        });
        let s2 = Arc::new(CountingSink {
            name: "fails-on-turn",
            seen: AtomicU32::new(0),
            only: None,
            fail_on: Some("turn_completed"),
        });
        let facade = FeedbackFacade::new()
            .with_sink(s1.clone())
            .with_sink(s2.clone());

        // Even though s2 panics on turn, s1 still gets the event.
        facade.on_event(&turn()).await.unwrap();
        assert_eq!(s1.seen.load(Ordering::Relaxed), 1);
        assert_eq!(s2.seen.load(Ordering::Relaxed), 0);

        let stats = facade.stats();
        let s2_stats = stats
            .per_sink
            .iter()
            .find(|s| s.name == "fails-on-turn")
            .unwrap();
        assert_eq!(s2_stats.failed, 1);
    }

    #[tokio::test]
    async fn all_sinks_failing_surfaces_an_error() {
        let s = Arc::new(CountingSink {
            name: "always-fails",
            seen: AtomicU32::new(0),
            only: None,
            fail_on: Some("turn_completed"),
        });
        let facade = FeedbackFacade::new().with_sink(s);
        let err = facade.on_event(&turn()).await.unwrap_err();
        assert!(err.to_string().contains("forced failure"));
    }

    #[tokio::test]
    async fn skipped_events_increment_counter() {
        let s = Arc::new(CountingSink {
            name: "plan-only",
            seen: AtomicU32::new(0),
            only: Some("plan_completed"),
            fail_on: None,
        });
        let facade = FeedbackFacade::new().with_sink(s);
        facade.on_event(&turn()).await.unwrap();
        let stats = facade.stats();
        assert_eq!(stats.per_sink[0].skipped, 1);
        assert_eq!(stats.per_sink[0].delivered, 0);
    }
}
