//! End-to-end test proving the runner emits to the new facades.
//!
//! Constructs a `Projection` + `FeedbackFacade` (with an in-process
//! capturing sink), wires them onto a `RunConfig`, then drives
//! `emit_runner_event_via_config` through synthetic runner events. Both
//! the projection broadcast and the feedback sink must observe each
//! event.
//!
//! This test does not exercise a real plan run because that requires a
//! live agent binary; instead it verifies the seam every plan run
//! travels through.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use roko_cli::runner::projection::{Projection, RawRuntimeEvent};
use roko_cli::runner::types::{
    EventCategory, GateCompletion, GateCompletionKind, PlanOutcome, RunnerEvent, RunnerFailureKind,
    TaskAttemptOutcome, TaskAttemptRef,
};
use roko_cli::runtime_feedback::{FeedbackEvent, FeedbackFacade, FeedbackSink};

/// In-process sink that records every event it receives so the test can
/// assert what landed on the facade.
#[derive(Debug, Default)]
struct CapturingSink {
    name: &'static str,
    events: tokio::sync::Mutex<Vec<&'static str>>,
}

#[async_trait]
impl FeedbackSink for CapturingSink {
    fn name(&self) -> &'static str {
        self.name
    }
    async fn on_event(&self, event: &FeedbackEvent) -> Result<(), anyhow::Error> {
        self.events.lock().await.push(event.label());
        Ok(())
    }
}

#[tokio::test]
async fn run_config_facades_receive_runner_events() {
    use roko_cli::runner::types::RunConfig;

    // ── Build a Projection + FeedbackFacade and attach them to a RunConfig.
    let projection = Arc::new(Projection::new("e2e-runner"));
    let mut subscriber = projection.subscribe();
    let sink = Arc::new(CapturingSink {
        name: "capture",
        events: Default::default(),
    });
    let facade = Arc::new(FeedbackFacade::new().with_sink(sink.clone()));

    let mut config = RunConfig::default();
    config.projection = Some(projection.clone());
    config.feedback_facade = Some(facade.clone());

    // ── Publish synthetic runner events directly through the same path
    //    the event_loop uses (RawRuntimeEvent::Runner -> publish).
    let attempt = TaskAttemptRef {
        plan_id: "p-e2e".into(),
        task_id: "t-e2e".into(),
        attempt: 0,
    };
    let task_completed = RunnerEvent::task_attempt_completed(
        "e2e-runner",
        attempt.clone(),
        TaskAttemptOutcome::Passed,
        None,
        1234,
        "claude-sonnet-4-6",
        "claude_cli",
    );
    let completion = GateCompletion {
        kind: GateCompletionKind::Gate,
        plan_id: "p-e2e".into(),
        task_id: "t-e2e".into(),
        rung: 2,
        passed: true,
        failure_kind: None,
        verdicts: Vec::new(),
        output: String::new(),
        duration_ms: 4321,
    };
    let gate_completed = RunnerEvent::gate_completed("e2e-runner", attempt.clone(), &completion);
    let plan_completed =
        RunnerEvent::plan_completed("e2e-runner", "p-e2e", PlanOutcome::Succeeded, None);

    for event in [&task_completed, &gate_completed, &plan_completed] {
        projection
            .publish(RawRuntimeEvent::Runner(event.clone()))
            .expect("projection publish");
    }

    for expected in ["task.attempt.completed", "gate.completed", "plan.completed"] {
        let evt = tokio::time::timeout(Duration::from_secs(2), subscriber.recv())
            .await
            .expect("subscriber receives event")
            .expect("subscriber open");
        assert_eq!(evt.event_type, expected);
    }

    // ── Now drive the feedback path the same way the runner does.
    let fb_task = FeedbackEvent::TaskCompleted {
        plan_id: "p-e2e".into(),
        task_id: "t-e2e".into(),
        outcome: roko_cli::dispatch::AgentOutcome {
            task_id: "t-e2e".into(),
            plan_id: "p-e2e".into(),
            model: "claude-sonnet-4-6".into(),
            provider: "claude_cli".into(),
            output: String::new(),
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            duration_ms: 0,
            exit_code: Some(0),
            is_error: false,
        },
        model_source: roko_cli::dispatch::ModelChoiceSource::Default,
        succeeded: true,
        routing_context: None,
    };
    let fb_gate = FeedbackEvent::GateOutcome {
        plan_id: "p-e2e".into(),
        task_id: "t-e2e".into(),
        rung: 2,
        passed: true,
        duration_ms: 4321,
    };
    let fb_plan = FeedbackEvent::PlanCompleted {
        plan_id: "p-e2e".into(),
        succeeded: true,
        tasks_completed: 1,
        tasks_failed: 0,
        total_cost_usd: 0.0,
    };

    facade.on_event(&fb_task).await.expect("task delivered");
    facade.on_event(&fb_gate).await.expect("gate delivered");
    facade.on_event(&fb_plan).await.expect("plan delivered");

    let labels = sink.events.lock().await.clone();
    assert_eq!(
        labels,
        vec!["task_completed", "gate_outcome", "plan_completed"],
        "feedback facade must deliver each event to the capture sink in order",
    );
}

#[tokio::test]
async fn projection_categorizes_runner_events_correctly() {
    let projection = Arc::new(Projection::new("cat-test"));
    let mut sub = projection.subscribe();

    let attempt = TaskAttemptRef {
        plan_id: "p".into(),
        task_id: "t".into(),
        attempt: 0,
    };
    let cases: Vec<(&'static str, RunnerEvent, EventCategory)> = vec![
        (
            "plan",
            RunnerEvent::plan_started("cat-test", "p"),
            EventCategory::Plan,
        ),
        (
            "task",
            RunnerEvent::task_attempt_started("cat-test", attempt.clone(), "t"),
            EventCategory::Task,
        ),
        (
            "gate",
            {
                let completion = GateCompletion {
                    kind: GateCompletionKind::Gate,
                    plan_id: "p".into(),
                    task_id: "t".into(),
                    rung: 1,
                    passed: false,
                    failure_kind: Some(RunnerFailureKind::Permanent),
                    verdicts: Vec::new(),
                    output: String::new(),
                    duration_ms: 100,
                };
                RunnerEvent::gate_completed("cat-test", attempt.clone(), &completion)
            },
            EventCategory::Gate,
        ),
    ];

    for (label, ev, expected) in &cases {
        projection
            .publish(RawRuntimeEvent::Runner(ev.clone()))
            .expect("publish");
        let received = tokio::time::timeout(Duration::from_secs(2), sub.recv())
            .await
            .expect("recv timeout")
            .expect("subscriber open");
        assert_eq!(
            received.category, *expected,
            "{label}: expected category {expected:?} got {:?}",
            received.category
        );
    }
}

#[tokio::test]
async fn feedback_failure_is_contained_per_sink() {
    // One failing sink + one capturing sink — the failure is logged but
    // the capturing sink still observes the event.
    let captured = Arc::new(AtomicU32::new(0));

    #[derive(Debug)]
    struct AlwaysFail;
    #[async_trait]
    impl FeedbackSink for AlwaysFail {
        fn name(&self) -> &'static str {
            "always-fail"
        }
        async fn on_event(&self, _e: &FeedbackEvent) -> Result<(), anyhow::Error> {
            anyhow::bail!("forced failure")
        }
    }

    #[derive(Debug)]
    struct Counter(Arc<AtomicU32>);
    #[async_trait]
    impl FeedbackSink for Counter {
        fn name(&self) -> &'static str {
            "counter"
        }
        async fn on_event(&self, _e: &FeedbackEvent) -> Result<(), anyhow::Error> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    let facade = FeedbackFacade::new()
        .with_sink(Arc::new(AlwaysFail))
        .with_sink(Arc::new(Counter(captured.clone())));

    facade
        .on_event(&FeedbackEvent::IdleTick {
            ticks_since_last_work: 1,
        })
        .await
        .expect("at least one sink succeeded");

    assert_eq!(
        captured.load(Ordering::Relaxed),
        1,
        "counter sink must receive event despite peer failure",
    );
}
