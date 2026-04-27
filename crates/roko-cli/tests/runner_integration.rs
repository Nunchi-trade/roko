//! Integration test wiring task_dag, merge, projection, and contract together.
//!
//! Exercises the new runner modules end-to-end to prove the primitives
//! compose correctly under realistic scheduling, merge, observability, and
//! safety scenarios.

use std::sync::Arc;
use std::time::Duration;

use roko_agent::safety::contract::{AgentContract, ContractLoadMode};
use roko_cli::runner::merge::{
    MergeBackend, MergeBackendOutcome, MergeDispatch, PlanMerger, PlanMergerConfig, RegressionGate,
    RegressionOutcome,
};
use roko_cli::runner::projection::{Projection, RawRuntimeEvent};
use roko_cli::runner::task_dag::{DagConfig, SkippedReason, TaskDag};
use roko_cli::runner::types::{
    AgentEvent, EventCategory, GateCompletion, RunnerEvent, RunnerFailureKind, StderrSeverity,
};
use roko_cli::task_parser::TaskDef;
use roko_core::tool::ToolCall;
use roko_orchestrator::{MergeQueue, MergeRequest};
use tempfile::tempdir;

fn task(id: &str, deps: &[&str]) -> TaskDef {
    TaskDef {
        id: id.into(),
        title: id.into(),
        description: None,
        role: None,
        status: "ready".into(),
        tier: "focused".into(),
        frequency: None,
        model_hint: None,
        replan_strategy: None,
        max_loc: None,
        files: vec![],
        allowed_tools: None,
        denied_tools: None,
        mcp_servers: None,
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        depends_on_plan: vec![],
        split_into: None,
        context: None,
        verify: vec![],
        timeout_secs: 60,
        max_retries: 1,
        acceptance: vec![],
        acceptance_contract: None,
        domain: None,
    }
}

#[tokio::test]
async fn end_to_end_dag_merge_projection_pipeline() {
    // ────────────────────────────────────────────────────────────────────
    // 1. DAG scheduling — A and B run in parallel; C waits for both.
    //    Verify ready-task resolution + double-dispatch guard + cleanup.
    // ────────────────────────────────────────────────────────────────────
    let mut dag = TaskDag::new(DagConfig::default());
    let a = task("A", &[]);
    let b = task("B", &[]);
    let c = task("C", &["A", "B"]);
    let tasks: Vec<&TaskDef> = vec![&a, &b, &c];

    let ready = dag.ready_tasks("p1", &tasks, &[], &[]);
    let ids: Vec<&str> = ready.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["A", "B"], "A and B must be ready first");

    assert!(dag.mark_running("p1", "A"));
    assert!(dag.mark_running("p1", "B"));
    assert!(
        !dag.mark_running("p1", "A"),
        "double-dispatch guard must reject re-running A"
    );

    let ready = dag.ready_tasks("p1", &tasks, &[], &[]);
    assert!(ready.is_empty(), "C must wait for A,B");

    dag.mark_complete("p1", "A");
    dag.mark_complete("p1", "B");
    let ready = dag.ready_tasks("p1", &tasks, &["A".into(), "B".into()], &[]);
    let ids: Vec<&str> = ready.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["C"]);

    // ────────────────────────────────────────────────────────────────────
    // 2. Failure propagation — a terminal A failure cascades skip to B,C.
    // ────────────────────────────────────────────────────────────────────
    let mut dag2 = TaskDag::new(DagConfig::default());
    let a2 = task("A", &[]);
    let b2 = task("B", &["A"]);
    let c2 = task("C", &["B"]);
    let tasks2: Vec<&TaskDef> = vec![&a2, &b2, &c2];
    let skipped = dag2.mark_failed_blocking_downstream("p1", "A", &tasks2);
    let mut sorted = skipped.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["B".to_string(), "C".to_string()],
        "transitive skipped propagation must reach C"
    );
    assert!(matches!(
        dag2.plan("p1").unwrap().skipped.get("B"),
        Some(SkippedReason::PrerequisiteFailed { .. })
    ));

    // ────────────────────────────────────────────────────────────────────
    // 3. PlanMerger — first plan reserved, second plan touching the same
    //    file is blocked. Stub regression gate fails the reserved plan.
    // ────────────────────────────────────────────────────────────────────
    let workdir = tempdir().expect("tempdir");
    let mut config = PlanMergerConfig::new(workdir.path().to_path_buf(), Duration::from_secs(30));
    let stub_gate: Arc<dyn RegressionGate> = Arc::new(StubFailingGate);
    let stub_merge: Arc<dyn MergeBackend> = Arc::new(StubPassingMerge);
    config = config.with_regression_gate(stub_gate).with_merge_backend(stub_merge);

    let queue = MergeQueue::default();
    let merger = PlanMerger::new(queue, config);

    let req1 = MergeRequest::new("alpha", "alpha-branch", vec!["src/foo.rs".into()], 10);
    let req2 = MergeRequest::new("beta", "beta-branch", vec!["src/foo.rs".into()], 10);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<GateCompletion>(8);
    match merger.submit(req1, tx.clone()) {
        MergeDispatch::Reserved { plan_id, .. } => assert_eq!(plan_id, "alpha"),
        other => panic!("first merge must be reserved, got {other:?}"),
    }

    // Second plan competes for the same file lock — must block.
    match merger.submit(req2, tx) {
        MergeDispatch::Blocked { plan_id } => assert_eq!(plan_id, "beta"),
        other => panic!("second merge must be blocked, got {other:?}"),
    }

    // The stub gate emits a failure GateCompletion for the reserved plan.
    let completion = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("regression gate emits completion within 5s")
        .expect("channel still open");
    assert_eq!(completion.plan_id, "alpha");
    assert!(!completion.passed, "stub gate forces failure");
    assert!(matches!(
        completion.failure_kind,
        Some(RunnerFailureKind::Permanent)
    ));

    // ────────────────────────────────────────────────────────────────────
    // 4. Projection facade — runner event → ProjectionEvent on broadcast.
    // ────────────────────────────────────────────────────────────────────
    let projection = Projection::new("test-run");
    let mut subscriber = projection.subscribe();
    let raw = RawRuntimeEvent::Runner(RunnerEvent::PlanStarted {
        timestamp: "2026-04-26T00:00:00Z".into(),
        timestamp_ms: 1_700_000_000_000,
        run_id: "test-run".into(),
        plan_id: "p1".into(),
    });
    projection.publish(raw).expect("publish");
    let event = tokio::time::timeout(Duration::from_secs(2), subscriber.recv())
        .await
        .expect("subscriber receives event")
        .expect("channel open");
    assert_eq!(event.run_id, "test-run");
    assert_eq!(event.category, EventCategory::Plan);
    assert_eq!(event.event_type, "plan.started");
    assert_eq!(event.plan_id.as_deref(), Some("p1"));

    // Bounded dashboard — pump 250 events, only last 200 retained.
    for i in 0..250 {
        let _ = projection.publish(RawRuntimeEvent::Runner(RunnerEvent::PlanStarted {
            timestamp: "2026-04-26T00:00:00Z".into(),
            timestamp_ms: 1_700_000_000_000 + i as u64,
            run_id: "test-run".into(),
            plan_id: format!("p{i}"),
        }));
    }
    let snapshot = projection.dashboard_snapshot();
    assert!(
        snapshot.events.len() <= 200,
        "dashboard must be bounded to 200 (got {})",
        snapshot.events.len()
    );

    // Stderr severity classifier.
    assert_eq!(
        StderrSeverity::from_message("WARN deprecated flag"),
        StderrSeverity::Warning
    );
    assert_eq!(
        StderrSeverity::from_message("error: panic in foo"),
        StderrSeverity::Error
    );
    assert_eq!(
        StderrSeverity::from_message("INFO loading config"),
        StderrSeverity::Infra
    );

    // ────────────────────────────────────────────────────────────────────
    // 5. Safety contract — bundled architect contract enforces capability
    //    intersection; restricted fallback denies every tool.
    // ────────────────────────────────────────────────────────────────────
    let architect = AgentContract::load_for_role_with_mode("architect", ContractLoadMode::Strict)
        .expect("architect.yaml bundled");
    assert_eq!(architect.role, "architect");
    assert!(!architect.permits_tool("write_file"));
    assert!(!architect.permits_tool("edit_file"));

    let restricted =
        AgentContract::load_for_role_with_mode("nonexistent", ContractLoadMode::RestrictedFallback)
            .expect("fallback always succeeds");
    assert_eq!(restricted.role, "nonexistent");
    for tool in ["read_file", "write_file", "bash"] {
        assert!(
            !restricted.permits_tool(tool),
            "restricted must reject `{tool}`",
        );
    }

    // Capability intersection enforcement at check_pre_execution.
    let role = AgentContract {
        role: "auditor".into(),
        invariants: Vec::new(),
        governance: Vec::new(),
        recovery: Vec::new(),
        allowed_tools: Some(vec!["read_file".into()]),
    };
    assert!(role.permits_tool("read_file"));
    assert!(!role.permits_tool("write_file"));
}

// ─── Stub regression gate that always fails ──────────────────────────────

#[derive(Debug)]
struct StubFailingGate;

#[async_trait::async_trait]
impl RegressionGate for StubFailingGate {
    async fn run(&self, _request: &MergeRequest, _config: &PlanMergerConfig) -> RegressionOutcome {
        RegressionOutcome::fail(
            "stub regression gate forced failure",
            RunnerFailureKind::Permanent,
            42,
        )
    }
}

/// Stub merge backend that always reports a successful merge so the
/// regression gate (which only runs on a successful merge) is exercised.
#[derive(Debug)]
struct StubPassingMerge;

#[async_trait::async_trait]
impl MergeBackend for StubPassingMerge {
    async fn merge(
        &self,
        request: &MergeRequest,
        _config: &PlanMergerConfig,
    ) -> MergeBackendOutcome {
        MergeBackendOutcome::pass(format!("stub merge accepted {}", request.branch_name), 5)
    }
}

#[test]
fn projection_event_categories_round_trip_serialization() {
    let categories = [
        EventCategory::Run,
        EventCategory::Plan,
        EventCategory::Task,
        EventCategory::AgentLifecycle,
        EventCategory::AgentMessage,
        EventCategory::AgentTool,
        EventCategory::Token,
        EventCategory::Cost,
        EventCategory::Gate,
        EventCategory::Retry,
        EventCategory::Dream,
        EventCategory::Other,
    ];
    for cat in categories {
        let s = serde_json::to_string(&cat).expect("serialize");
        let back: EventCategory = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back, cat, "round-trip for {cat:?}");
        assert!(!cat.as_str().is_empty());
    }
}

#[test]
fn task_dag_exponential_backoff_caps_at_30s() {
    let dag = TaskDag::new(DagConfig {
        plan_timeout: Duration::from_secs(3600),
        retry_base: Duration::from_secs(1),
        retry_max: Duration::from_secs(30),
    });
    assert_eq!(dag.backoff_for_attempt(0), Duration::from_secs(1));
    assert_eq!(dag.backoff_for_attempt(1), Duration::from_secs(2));
    assert_eq!(dag.backoff_for_attempt(4), Duration::from_secs(16));
    assert_eq!(dag.backoff_for_attempt(5), Duration::from_secs(30));
    assert_eq!(dag.backoff_for_attempt(99), Duration::from_secs(30));
}

#[test]
fn agent_event_to_event_category_is_provider_neutral() {
    use AgentEvent::*;
    assert_eq!(
        EventCategory::from_agent_event(&Started {
            agent_id: "x".into(),
            provider: "claude_cli".into(),
            model: "claude-sonnet-4-6".into(),
            pid: Some(1234),
        }),
        EventCategory::AgentLifecycle
    );
    assert_eq!(
        EventCategory::from_agent_event(&ToolCall {
            id: "t1".into(),
            name: "read_file".into(),
        }),
        EventCategory::AgentTool
    );
    assert_eq!(
        EventCategory::from_agent_event(&MessageDelta { text: "hi".into() }),
        EventCategory::AgentMessage
    );
    assert_eq!(
        EventCategory::from_agent_event(&TokenUsage {
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }),
        EventCategory::Token
    );
    assert_eq!(
        EventCategory::from_agent_event(&TurnCompleted {
            session_id: None,
            total_cost_usd: Some(0.001),
            num_turns: Some(1),
            is_error: false,
        }),
        EventCategory::Cost
    );
}

#[test]
fn allowed_tools_intersects_with_forbidden_tools_correctly() {
    use roko_agent::safety::contract::GovernanceRule;
    // Even with a permissive allowlist, ForbiddenTools wins.
    let contract = AgentContract {
        role: "test".into(),
        invariants: Vec::new(),
        governance: vec![GovernanceRule::ForbiddenTools(vec!["bash".into()])],
        recovery: Vec::new(),
        allowed_tools: Some(vec!["bash".into(), "read_file".into()]),
    };
    assert!(
        !contract.permits_tool("bash"),
        "ForbiddenTools must override allowlist"
    );
    assert!(contract.permits_tool("read_file"));
}

#[test]
fn agent_contract_check_pre_execution_rejects_unknown_tool() {
    use roko_core::tool::handler::NoopAuditSink;
    use roko_core::tool::metrics::NoopMetricsSink;
    use roko_core::tool::trace::NoopTraceSink;
    use roko_core::tool::{ToolContext, ToolPermission};

    let contract = AgentContract {
        role: "auditor".into(),
        invariants: Vec::new(),
        governance: Vec::new(),
        recovery: Vec::new(),
        allowed_tools: Some(vec!["read_file".into()]),
    };
    let ctx = ToolContext::new(
        "/tmp/integration-tests",
        Duration::from_secs(5),
        ToolPermission {
            read: true,
            write: true,
            exec: true,
            git: true,
            network: true,
        },
        Arc::new(NoopAuditSink),
        Arc::new(NoopTraceSink),
        Arc::new(NoopMetricsSink),
        Arc::new(roko_core::tool::NeverCancel),
    );

    let call = ToolCall::new("c1", "write_file", serde_json::json!({}));
    let err = contract
        .check_pre_execution(&call, &ctx)
        .expect_err("write_file must be rejected");
    assert_eq!(err.rule, "AllowedTools");

    let call = ToolCall::new("c2", "read_file", serde_json::json!({"path": "/tmp"}));
    contract.check_pre_execution(&call, &ctx).unwrap();
}
