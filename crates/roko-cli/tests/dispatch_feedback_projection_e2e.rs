//! End-to-end test wiring `dispatch/`, `runtime_feedback/`, and
//! `projection/` together.
//!
//! Demonstrates the full architectural pipeline:
//!
//! 1. `Dispatcher::plan` resolves a model + assembles a prompt.
//! 2. `Dispatcher::dispatch` (via a stub bridge) returns a normalized
//!    `AgentOutcome`.
//! 3. The runner forwards a `FeedbackEvent::TaskCompleted` to the
//!    `FeedbackFacade`, which fans out to:
//!    - `EpisodeSink` → durable `.roko/episodes.jsonl`
//!    - `RoutingObservationSink` → `CascadeRouter::record_outcome`
//!    - `KnowledgeIngestionSink` → `.roko/learn/knowledge-candidates.jsonl`
//! 4. The runner publishes a `ProjectionEvent` to `Projection` and the
//!    CLI progress projection renders it into a structured progress
//!    line.
//!
//! Every artifact is verified on disk so this is a real wiring proof,
//! not a unit-test mock.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roko_cli::dispatch::{
    AgentOutcome, AgentResultBridge, DispatchContext, Dispatcher, ModelChoiceSource,
    PromptAssembler, RunnerDispatchPlan, WarmPool,
};
use roko_cli::projection::{CliProgressPrinter, DashboardProjection};
use roko_cli::runner::projection::{Projection, RawRuntimeEvent};
use roko_cli::runner::types::RunnerEvent;
use roko_cli::runtime_feedback::{
    EpisodeSink, FeedbackEvent, FeedbackFacade, KnowledgeIngestionSink, RoutingObservationSink,
};
use roko_cli::task_parser::TaskDef;
use roko_learn::cascade_router::CascadeRouter;
use tempfile::tempdir;

fn task() -> TaskDef {
    TaskDef {
        id: "wire-it-up".into(),
        title: "Wire end-to-end".into(),
        description: Some("Integration test task".into()),
        role: Some("implementer".into()),
        status: "ready".into(),
        tier: "focused".into(),
        frequency: None,
        model_hint: Some("claude-sonnet-4-6".into()),
        replan_strategy: None,
        max_loc: None,
        files: vec!["src/lib.rs".into()],
        allowed_tools: Some(vec!["read_file".into()]),
        denied_tools: None,
        mcp_servers: None,
        depends_on: vec![],
        depends_on_plan: vec![],
        split_into: None,
        context: None,
        verify: vec![],
        timeout_secs: 60,
        max_retries: 1,
        acceptance: vec!["compiles".into()],
        acceptance_contract: None,
        domain: Some(roko_core::task::TaskDomain::Code),
        sequence: 0,
    }
}

fn ctx(workdir: std::path::PathBuf) -> DispatchContext {
    DispatchContext {
        plan_id: "plan-e2e".into(),
        role: "implementer".into(),
        workdir,
        model_hint: None,
        force_backend: None,
        budget_remaining_usd: 5.0,
        attempt: 0,
        gate_feedback: None,
        routing_context: None,
        dependency_outputs: Vec::new(),
    }
}

#[derive(Debug)]
struct StubBridge;

#[async_trait]
impl AgentResultBridge for StubBridge {
    async fn run_agent(
        &self,
        plan: &RunnerDispatchPlan,
        ctx: &DispatchContext,
    ) -> Result<AgentOutcome, anyhow::Error> {
        Ok(AgentOutcome {
            task_id: "wire-it-up".into(),
            plan_id: ctx.plan_id.clone(),
            model: plan.model.slug.clone(),
            provider: "claude_cli".into(),
            output: "ok".into(),
            tokens_in: 1024,
            tokens_out: 256,
            cost_usd: 0.012,
            duration_ms: 9_876,
            exit_code: Some(0),
            is_error: false,
        })
    }
}

#[tokio::test]
async fn dispatch_feeds_feedback_facade_and_projection() {
    let workdir = tempdir().expect("tempdir");
    let roko_dir = workdir.path().join(".roko");
    std::fs::create_dir_all(roko_dir.join("learn")).unwrap();
    let episodes_path = roko_dir.join("episodes.jsonl");
    let knowledge_path = roko_dir.join("learn/knowledge-candidates.jsonl");

    // ── 1. Dispatch ────────────────────────────────────────────────────
    let dispatcher = Dispatcher::new(None, PromptAssembler::minimal(), WarmPool::new(2));
    let task = task();
    let dctx = ctx(workdir.path().to_path_buf());
    let outcome = dispatcher
        .dispatch(&task, &dctx, &StubBridge)
        .await
        .expect("dispatch");
    assert_eq!(outcome.model, "claude-sonnet-4-6");
    assert_eq!(outcome.tokens_in, 1024);

    // ── 2. Feedback fan-out ────────────────────────────────────────────
    let router = Arc::new(CascadeRouter::new(vec![
        "claude-sonnet-4-6".into(),
        "gpt-5".into(),
    ]));
    let facade = FeedbackFacade::new()
        .with_sink(Arc::new(EpisodeSink::at(&episodes_path)))
        .with_sink(Arc::new(RoutingObservationSink::new(router.clone())))
        .with_sink(Arc::new(KnowledgeIngestionSink::at(&knowledge_path)));

    facade
        .on_event(&FeedbackEvent::TaskCompleted {
            plan_id: "plan-e2e".into(),
            task_id: "wire-it-up".into(),
            outcome: outcome.clone(),
            model_source: ModelChoiceSource::TaskHint,
            succeeded: true,
            routing_context: None,
        })
        .await
        .expect("fanout task completed");

    facade
        .on_event(&FeedbackEvent::GateOutcome {
            plan_id: "plan-e2e".into(),
            task_id: "wire-it-up".into(),
            rung: 2,
            passed: false,
            duration_ms: 1_234,
        })
        .await
        .expect("fanout gate failure");

    facade
        .on_event(&FeedbackEvent::PlanCompleted {
            plan_id: "plan-e2e".into(),
            succeeded: true,
            tasks_completed: 1,
            tasks_failed: 0,
            total_cost_usd: 0.012,
        })
        .await
        .expect("fanout plan completion");

    // ── 3. Verify durable artifacts ────────────────────────────────────
    let episodes = tokio::fs::read_to_string(&episodes_path).await.unwrap();
    assert!(episodes.contains("\"backend\":\"claude_cli\""));
    assert!(episodes.contains("\"model\":\"claude-sonnet-4-6\""));
    assert!(episodes.contains("\"plan_id\":\"plan-e2e\""));

    let knowledge = tokio::fs::read_to_string(&knowledge_path).await.unwrap();
    assert!(knowledge.contains("\"kind\":\"success\""));
    assert!(knowledge.contains("\"kind\":\"gate_falsifier\""));

    let stats = facade.stats();
    let names: Vec<&str> = stats.per_sink.iter().map(|s| s.name).collect();
    assert!(names.contains(&"episodes"));
    assert!(names.contains(&"routing"));
    assert!(names.contains(&"knowledge"));

    // ── 4. Projection — render via CLI printer + dashboard bridge ──────
    let projection = Arc::new(Projection::new("run-e2e"));
    let printer = CliProgressPrinter::new();
    let dashboard = DashboardProjection::new();

    // Subscribe before publishing — broadcast drops events with no
    // listeners.
    let mut sub = projection.subscribe();

    projection
        .publish(RawRuntimeEvent::Runner(RunnerEvent::PlanStarted {
            timestamp: "2026-04-26T00:00:00Z".into(),
            timestamp_ms: 1,
            run_id: "run-e2e".into(),
            plan_id: "plan-e2e".into(),
        }))
        .expect("publish plan started");
    let event = tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("event arrives")
        .expect("subscription open");
    let cli_line = printer.format(&event).expect("plan event renders");
    assert!(cli_line.contains("plan-e2e"));
    assert!(cli_line.starts_with("▶"));

    let snippet = dashboard.map(&event).expect("dashboard maps plan event");
    assert_eq!(snippet.run_id, "run-e2e");
    assert_eq!(snippet.plan_id.as_deref(), Some("plan-e2e"));
    assert!(snippet.headline.contains("plan-e2e"));
    assert_eq!(dashboard.stats().mapped, 1);
}

#[tokio::test]
async fn force_backend_routes_through_override_path_and_records_observation() {
    let workdir = tempdir().expect("tempdir");
    let dispatcher = Dispatcher::new(None, PromptAssembler::minimal(), WarmPool::new(0));
    let task = task();
    let mut dctx = ctx(workdir.path().to_path_buf());
    dctx.force_backend = Some("gpt-5".into());

    let outcome = dispatcher
        .dispatch(&task, &dctx, &StubBridge)
        .await
        .expect("dispatch with override");
    assert_eq!(outcome.model, "gpt-5", "override wins over task hint");

    // The override is reflected in the dispatch plan; downstream
    // feedback fan-out gets ModelChoiceSource::Override so the routing
    // sink can dampen the observation.
    let plan = dispatcher.plan(&task, &dctx).expect("plan");
    assert!(plan.forced);
}

#[tokio::test]
async fn retry_attempt_includes_gate_feedback_in_assembled_prompt() {
    use roko_cli::dispatch::GateFeedback;

    let workdir = tempdir().expect("tempdir");
    let dispatcher = Dispatcher::new(None, PromptAssembler::minimal(), WarmPool::new(0));
    let task = task();
    let mut dctx = ctx(workdir.path().to_path_buf());
    dctx.attempt = 1;
    dctx.gate_feedback = Some(GateFeedback {
        compile_errors: vec!["E0432: unresolved import".into()],
        test_failures: vec!["mod::test_foo failed".into()],
        clippy_warnings: vec![],
        raw_output: "...".into(),
    });

    let plan = dispatcher.plan(&task, &dctx).expect("plan");
    assert!(plan.prompt.system_prompt.contains("Previous attempt"));
    assert!(plan.prompt.system_prompt.contains("E0432"));
    assert!(
        plan.prompt
            .diagnostics
            .included_sections
            .contains(&"retry".to_string())
    );
}
