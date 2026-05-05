//! Server-side events emitted during plan execution, agent runs, and other
//! operations. These flow through the shared event bus and are streamed to
//! connected SSE / WebSocket clients.

use roko_core::{ContentHash, Engram};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Progress emitted by the execution loop as plans move through phases,
/// complete tasks, and finish gate checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    /// A plan has begun execution.
    PlanStarted,

    /// A task has entered its first active phase.
    TaskStarted {
        /// Task identifier.
        task_id: String,
        /// Human-readable task title.
        #[serde(default)]
        title: String,
        /// Phase the task is starting in.
        phase: String,
    },

    /// A task transitioned between phases.
    TaskPhaseChanged {
        /// Task identifier.
        task_id: String,
        /// Previous phase name.
        old_phase: String,
        /// New phase name.
        new_phase: String,
    },

    /// A gate completed for a task.
    GateResult {
        /// Task identifier.
        task_id: String,
        /// Verify name.
        gate: String,
        /// Whether the gate passed.
        passed: bool,
        /// Human-readable message or failure summary.
        message: String,
    },

    /// A task has completed.
    TaskCompleted {
        /// Task identifier.
        task_id: String,
        /// Outcome summary.
        outcome: String,
    },

    /// A plan has completed.
    PlanCompleted {
        /// Plan outcome summary.
        outcome: String,
        /// Execution statistics for the plan.
        stats: serde_json::Value,
    },

    /// A re-plan was triggered for a task.
    ReplanTriggered {
        /// Task identifier that caused the re-plan.
        task_id: String,
        /// Re-plan strategy or reason.
        strategy: String,
    },

    /// A watcher emitted an alert.
    WatcherAlert {
        /// Watcher name.
        watcher: String,
        /// Alert message.
        message: String,
    },
}

/// A tagged union of all events the HTTP server can emit.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// A plan execution has started.
    PlanStarted { plan_id: String },

    /// A plan execution has completed.
    PlanCompleted { plan_id: String, success: bool },

    /// An agent process was spawned.
    AgentSpawned {
        agent_id: String,
        role: String,
        #[serde(default)]
        model: String,
    },

    /// Incremental agent output (streamed, sanitized for consumers).
    AgentOutput {
        agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        content: String,
        #[serde(default)]
        done: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
    },

    /// Raw agent trace output for debugging / inspection.
    ///
    /// Contains unsanitized content plus optional structured fields
    /// (tool calls, reasoning, token usage).  Only consumers that
    /// explicitly subscribe to `agent_trace` see these events.
    AgentTrace {
        agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Value>,
        #[serde(default)]
        done: bool,
    },

    /// A gate check completed for a task.
    GateResult {
        plan_id: String,
        task_id: String,
        /// Verify name (also exposed as `rung` for dashboard compat).
        gate: String,
        /// Numeric rung index for dashboard display.
        #[serde(default)]
        rung: u32,
        passed: bool,
    },

    /// Execution progress update streamed from the orchestrator.
    Execution {
        /// Plan identifier for the execution payload.
        plan_id: String,
        /// Nested execution event.
        event: ExecutionEvent,
    },

    /// The plan transitioned between execution phases.
    PhaseTransition {
        plan_id: String,
        from: String,
        to: String,
    },

    /// An episode (agent turn + gate result) was recorded.
    Episode {
        plan_id: String,
        task_id: String,
        passed: bool,
    },

    /// An efficiency metric was recorded for a task.
    EfficiencyEvent {
        plan_id: String,
        task_id: String,
        metric: String,
        value: f64,
    },

    /// A gateway inference request was started.
    InferenceStarted {
        /// Unique request identifier.
        request_id: String,
        /// Model slug selected for this request.
        model: String,
        /// Agent that initiated the request.
        #[serde(default)]
        agent_id: String,
        /// Whether the model was explicitly requested or auto-selected.
        #[serde(default)]
        auto_routed: bool,
    },

    /// A gateway inference request completed successfully.
    InferenceCompleted {
        /// Unique request identifier.
        request_id: String,
        /// Model that actually served the request.
        model: String,
        /// Agent that initiated the request.
        #[serde(default)]
        agent_id: String,
        /// Input tokens consumed.
        input_tokens: u64,
        /// Output tokens generated.
        output_tokens: u64,
        /// Estimated cost in USD.
        cost_usd: f64,
        /// Wall-clock duration in milliseconds.
        duration_ms: u64,
    },

    /// A gateway inference request failed.
    InferenceFailed {
        /// Unique request identifier.
        request_id: String,
        /// Model that was targeted.
        model: String,
        /// Agent that initiated the request.
        #[serde(default)]
        agent_id: String,
        /// Error description.
        error: String,
    },

    /// A strong somatic marker fired for the current task situation.
    SomaticMarkerFired {
        plan_id: String,
        task_id: String,
        valence: f64,
        intensity: f64,
        source_episodes: Vec<ContentHash>,
        strategy_param: String,
    },

    /// A one-shot run was started.
    RunStarted {
        run_id: String,
        #[serde(rename = "prompt_preview")]
        prompt: String,
    },

    /// A one-shot run completed.
    RunCompleted { run_id: String, success: bool },

    /// A generic operation was started.
    OperationStarted { op_id: String, kind: String },

    /// A generic operation completed.
    OperationCompleted {
        op_id: String,
        kind: String,
        success: bool,
    },

    /// A cloud deployment was created.
    DeploymentCreated { id: String, name: String },

    /// A cloud deployment is ready and reachable.
    DeploymentReady { id: String, url: String },

    /// A cloud deployment failed.
    DeploymentFailed { id: String, reason: String },

    /// A cloud deployment was torn down.
    DeploymentTornDown { id: String },

    /// A marketplace job was created.
    JobCreated { job: Value },

    /// A marketplace job was posted to a committed candidate.
    JobPostedToCandidate {
        job_id: String,
        agent_id: String,
        reward: String,
    },

    /// A marketplace job was updated.
    JobUpdated { job: Value },

    /// A marketplace job transitioned between lifecycle states.
    JobTransitioned {
        job_id: String,
        from: String,
        to: String,
        assigned_to: Option<String>,
    },

    /// A worker started executing a task.
    WorkerTaskStarted {
        deployment_id: String,
        task_id: String,
    },

    /// A worker completed a task.
    WorkerTaskCompleted {
        deployment_id: String,
        task_id: String,
        success: bool,
    },

    /// A job execution started (from job runner).
    JobExecutionStarted {
        job_id: String,
        job_type: String,
        agent_id: String,
    },

    /// Job execution progress update.
    JobProgress {
        job_id: String,
        percent: u8,
        message: String,
    },

    /// Agent output for a specific job.
    JobAgentOutput {
        job_id: String,
        agent_id: String,
        content: String,
        done: bool,
    },

    /// Chain triage results for a job.
    ChainTriageResult {
        job_id: String,
        event_count: usize,
        anomaly_count: usize,
        summary: String,
    },

    /// A heartbeat was received from a client or agent.
    HeartbeatReceived {
        sender_id: String,
        active_tasks: usize,
        active_agents: usize,
    },

    /// A top-level task started event (dashboard-facing).
    TaskStarted {
        plan_id: String,
        task_id: String,
        description: String,
    },

    /// A top-level task completed event (dashboard-facing).
    TaskCompleted {
        plan_id: String,
        task_id: String,
        success: bool,
    },

    /// A top-level task failed event (dashboard-facing).
    TaskFailed {
        plan_id: String,
        task_id: String,
        error: String,
    },

    /// An agent process was started.
    AgentStarted { agent_id: String },

    /// An agent process was stopped.
    AgentStopped { agent_id: String, reason: String },

    /// A job submission was received.
    JobSubmitted { job_id: String, agent_id: String },

    /// A job evaluation completed.
    JobEvaluated {
        job_id: String,
        accepted: bool,
        feedback: String,
    },

    /// A job state changed (aliases job_transitioned for dashboard compat).
    JobStateChanged {
        job_id: String,
        from: String,
        to: String,
    },

    /// A periodic heartbeat.
    Heartbeat {
        agent_id: String,
        block_number: Option<u64>,
    },

    /// New ISFR composite rate computed by the keeper.
    IsfrRateComputed {
        /// Overall composite rate in basis points.
        composite_bps: u64,
        /// Lending-class weighted median in basis points.
        lending_bps: u64,
        /// Structured-product weighted median in basis points.
        structured_bps: u64,
        /// Funding-rate weighted median in basis points.
        funding_bps: u64,
        /// Staking-rate weighted median in basis points.
        staking_bps: u64,
        /// Confidence score (0–10000, where 10000 = 100% of sources live).
        confidence_bps: u64,
        /// Number of source readings that contributed to this composite.
        source_count: usize,
        /// Unix millisecond timestamp when this composite was computed.
        timestamp_ms: i64,
    },

    /// ISFR source health status changed.
    IsfrSourceHealthChanged {
        /// Unique source identifier.
        source_id: String,
        /// Health status string: "live", "stale", or "offline".
        health: String,
        /// Most recent rate from this source in basis points, if available.
        last_rate_bps: Option<u64>,
    },

    /// ISFR keeper started or stopped.
    IsfrKeeperStateChanged {
        /// Whether the keeper is now running.
        running: bool,
    },

    /// The server is shutting down.
    ServerShutdown,

    /// An error occurred.
    Error { message: String },

    /// A webhook signal was accepted and published for downstream processing.
    WebhookReceived { signal: Engram },

    /// A vision-loop iteration completed.
    VisionLoopIteration {
        run_id: String,
        iteration: u32,
        score: f64,
        notes: String,
    },

    /// A vision-loop run completed.
    VisionLoopCompleted {
        run_id: String,
        iterations: u32,
        best_score: f64,
        stop_reason: String,
    },

    /// Configuration was reloaded from disk (LIFE-07).
    ConfigReloaded {
        /// Summaries of sections that were hot-reloaded.
        applied_sections: Vec<String>,
        /// Summaries of sections that require a restart to take effect.
        restart_required: Vec<String>,
    },

    /// STRATEGY.md was reloaded from disk (LIFE-07).
    StrategyReloaded {
        /// Number of parsed goals.
        goals_count: usize,
        /// Number of parsed tactics.
        tactics_count: usize,
    },

    /// A bench run was started.
    #[serde(rename = "BenchRunStarted")]
    BenchRunStarted {
        bench_id: String,
        suite_id: String,
        total_tasks: usize,
    },

    /// A bench task started executing.
    #[serde(rename = "BenchTaskStarted")]
    BenchTaskStarted {
        bench_id: String,
        task_id: String,
        task_name: String,
        task_index: usize,
        total_tasks: usize,
    },

    /// A bench task completed — includes the full `BenchTaskResult` object
    /// that the frontend expects under the `result` key.
    #[serde(rename = "BenchTaskCompleted")]
    BenchTaskCompleted {
        bench_id: String,
        task_id: String,
        result: serde_json::Value,
    },

    /// Learning artifacts created while finishing a bench task.
    #[serde(rename = "BenchLearningEvent")]
    BenchLearningEvent {
        /// Bench run identifier.
        bench_id: String,
        /// Bench task identifier.
        task_id: String,
        /// Playbooks created for this task.
        playbooks_created: u32,
        /// Anti-patterns created for this task.
        anti_patterns_created: u32,
        /// Total playbooks observed after this task.
        total_playbooks: u32,
        /// Total anti-patterns observed after this task.
        total_anti_patterns: u32,
    },

    /// Overall progress of a bench run.
    #[serde(rename = "BenchProgress")]
    BenchProgress {
        bench_id: String,
        completed: usize,
        total: usize,
        cost_so_far: f64,
    },

    /// A bench run completed — includes the full `BenchRunSummary` that
    /// the frontend expects under the `summary` key.
    #[serde(rename = "BenchRunCompleted")]
    BenchRunCompleted {
        bench_id: String,
        summary: serde_json::Value,
    },

    /// A bench regression report was produced after a run completed.
    #[serde(rename = "BenchRegressionReport")]
    BenchRegressionReport {
        bench_id: String,
        has_regressions: bool,
        report: serde_json::Value,
    },

    /// A matrix (multi-lane) bench run was started.
    #[serde(rename = "MatrixRunStarted")]
    MatrixRunStarted {
        matrix_id: String,
        suite_id: String,
        lane_ids: Vec<String>,
        total_lanes: usize,
    },

    /// A single lane of a matrix run completed.
    #[serde(rename = "MatrixLaneCompleted")]
    MatrixLaneCompleted {
        matrix_id: String,
        lane_id: String,
        pass_rate: f64,
        cost_usd: f64,
    },

    /// A matrix run completed — includes per-lane summaries.
    #[serde(rename = "MatrixRunCompleted")]
    MatrixRunCompleted {
        matrix_id: String,
        summary: Vec<serde_json::Value>,
    },

    /// A gate verdict for a bench task.
    #[serde(rename = "BenchGateVerdict")]
    BenchGateVerdict {
        bench_id: String,
        task_id: String,
        gate: String,
        passed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        duration_ms: u64,
    },

    /// Token throughput for a bench task.
    #[serde(rename = "BenchTokenVelocity")]
    BenchTokenVelocity {
        bench_id: String,
        task_id: String,
        tokens_per_second: f64,
        tokens_in: u64,
        tokens_out: u64,
        duration_ms: u64,
    },

    /// Agent output snapshot for a bench task.
    #[serde(rename = "BenchAgentOutput")]
    BenchAgentOutput {
        bench_id: String,
        task_id: String,
        agent_id: String,
        content: String,
        done: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },

    /// A SWE-bench run was started.
    #[serde(rename = "SweRunStarted")]
    SweRunStarted {
        run_id: String,
        dataset: String,
        total_instances: usize,
    },

    /// A SWE-bench instance completed.
    #[serde(rename = "SweInstanceCompleted")]
    SweInstanceCompleted {
        run_id: String,
        instance_id: String,
        resolved: bool,
        duration_ms: u64,
    },

    /// A SWE-bench run completed with summary stats.
    #[serde(rename = "SweRunCompleted")]
    SweRunCompleted {
        run_id: String,
        resolved: u32,
        total: u32,
        pass_rate: f64,
    },

    /// New block observed on the connected chain.
    ChainBlock {
        number: u64,
        hash: String,
        parent_hash: String,
        timestamp: u64,
        gas_used: u64,
        gas_limit: u64,
        tx_count: u32,
        base_fee_per_gas: Option<u64>,
    },

    /// Transaction included in a block.
    ChainTx {
        block_number: u64,
        tx_hash: String,
        from: String,
        to: Option<String>,
        value_wei: String,
        gas_used: u64,
        method_sig: Option<String>,
        success: bool,
    },

    /// Decoded contract event log.
    ChainContractEvent {
        block_number: u64,
        tx_hash: String,
        log_index: u32,
        contract: String,
        event_name: String,
        decoded: serde_json::Value,
    },

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_event_serializes_with_type_tag() {
        let event = ExecutionEvent::GateResult {
            task_id: "task-1".into(),
            gate: "compile".into(),
            passed: false,
            message: "compile failed".into(),
        };

        let json = serde_json::to_value(event).expect("serialize execution event");
        assert_eq!(json["type"], "gate_result");
        assert_eq!(json["task_id"], "task-1");
        assert_eq!(json["gate"], "compile");
        assert_eq!(json["passed"], false);
        assert_eq!(json["message"], "compile failed");
    }

    #[test]
    fn all_dashboard_event_types_serialize_as_snake_case() {
        let events: Vec<ServerEvent> = vec![
            ServerEvent::PlanStarted {
                plan_id: "p1".into(),
            },
            ServerEvent::PlanCompleted {
                plan_id: "p1".into(),
                success: true,
            },
            ServerEvent::TaskStarted {
                plan_id: "p1".into(),
                task_id: "t1".into(),
                description: "desc".into(),
            },
            ServerEvent::TaskCompleted {
                plan_id: "p1".into(),
                task_id: "t1".into(),
                success: true,
            },
            ServerEvent::TaskFailed {
                plan_id: "p1".into(),
                task_id: "t1".into(),
                error: "err".into(),
            },
            ServerEvent::GateResult {
                plan_id: "p1".into(),
                task_id: "t1".into(),
                gate: "compile".into(),
                rung: 1,
                passed: true,
            },
            ServerEvent::RunStarted {
                run_id: "r1".into(),
                prompt: "p".into(),
            },
            ServerEvent::RunCompleted {
                run_id: "r1".into(),
                success: true,
            },
            ServerEvent::HeartbeatReceived {
                sender_id: "s".into(),
                active_tasks: 1,
                active_agents: 2,
            },
            ServerEvent::Heartbeat {
                agent_id: "a".into(),
                block_number: Some(42),
            },
            ServerEvent::JobCreated {
                job: serde_json::json!({}),
            },
            ServerEvent::JobStateChanged {
                job_id: "j1".into(),
                from: "open".into(),
                to: "assigned".into(),
            },
            ServerEvent::JobSubmitted {
                job_id: "j1".into(),
                agent_id: "a".into(),
            },
            ServerEvent::JobEvaluated {
                job_id: "j1".into(),
                accepted: true,
                feedback: "ok".into(),
            },
            ServerEvent::AgentStarted {
                agent_id: "a".into(),
            },
            ServerEvent::AgentStopped {
                agent_id: "a".into(),
                reason: "done".into(),
            },
            ServerEvent::OperationCompleted {
                op_id: "o".into(),
                kind: "k".into(),
                success: true,
            },
            ServerEvent::Error {
                message: "err".into(),
            },
            ServerEvent::VisionLoopIteration {
                run_id: "v1".into(),
                iteration: 1,
                score: 7.5,
                notes: "good".into(),
            },
            ServerEvent::VisionLoopCompleted {
                run_id: "v1".into(),
                iterations: 5,
                best_score: 9.0,
                stop_reason: "target_reached".into(),
            },
            ServerEvent::InferenceStarted {
                request_id: "req-1".into(),
                model: "sonnet".into(),
                agent_id: "a".into(),
                auto_routed: true,
            },
            ServerEvent::InferenceCompleted {
                request_id: "req-1".into(),
                model: "sonnet".into(),
                agent_id: "a".into(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                duration_ms: 1500,
            },
            ServerEvent::InferenceFailed {
                request_id: "req-1".into(),
                model: "sonnet".into(),
                agent_id: "a".into(),
                error: "timeout".into(),
            },
            ServerEvent::ServerShutdown,
        ];

        for event in &events {
            let json = serde_json::to_value(event).expect("serialize event");
            let event_type = json["type"].as_str().expect("type field");
            // All type tags must be snake_case (no uppercase letters)
            assert!(
                !event_type.chars().any(|c| c.is_ascii_uppercase()),
                "event type '{event_type}' is not snake_case"
            );
        }
    }

    #[test]
    fn agent_output_serializes_optional_run_fields() {
        let event = ServerEvent::AgentOutput {
            agent_id: "agent-7".into(),
            run_id: Some("run-1".into()),
            content: "hello".into(),
            done: true,
            metadata: Some(serde_json::json!({ "cost_usd": 0.1 })),
        };

        let json = serde_json::to_value(event).expect("serialize agent output");
        assert_eq!(json["type"], "agent_output");
        assert_eq!(json["agent_id"], "agent-7");
        assert_eq!(json["run_id"], "run-1");
        assert_eq!(json["content"], "hello");
        assert_eq!(json["done"], true);
        assert_eq!(json["metadata"]["cost_usd"], 0.1);
    }

    #[test]
    fn bench_learning_event_serializes_with_counts() {
        let event = ServerEvent::BenchLearningEvent {
            bench_id: "run-1".into(),
            task_id: "task-7".into(),
            playbooks_created: 1,
            anti_patterns_created: 2,
            total_playbooks: 9,
            total_anti_patterns: 4,
        };

        let json = serde_json::to_value(event).expect("serialize bench learning event");
        assert_eq!(json["type"], "BenchLearningEvent");
        assert_eq!(json["bench_id"], "run-1");
        assert_eq!(json["task_id"], "task-7");
        assert_eq!(json["playbooks_created"], 1);
        assert_eq!(json["anti_patterns_created"], 2);
        assert_eq!(json["total_playbooks"], 9);
        assert_eq!(json["total_anti_patterns"], 4);
    }

    #[test]
    fn bench_gate_verdict_serializes() {
        let event = ServerEvent::BenchGateVerdict {
            bench_id: "run-1".into(),
            task_id: "task-3".into(),
            gate: "compile".into(),
            passed: true,
            message: Some("ok".into()),
            duration_ms: 1200,
        };

        let json = serde_json::to_value(event).expect("serialize bench gate verdict");
        assert_eq!(json["type"], "BenchGateVerdict");
        assert_eq!(json["bench_id"], "run-1");
        assert_eq!(json["gate"], "compile");
        assert_eq!(json["passed"], true);
        assert_eq!(json["message"], "ok");
        assert_eq!(json["duration_ms"], 1200);
    }

    #[test]
    fn bench_token_velocity_serializes() {
        let event = ServerEvent::BenchTokenVelocity {
            bench_id: "run-1".into(),
            task_id: "task-3".into(),
            tokens_per_second: 42.5,
            tokens_in: 100,
            tokens_out: 50,
            duration_ms: 3529,
        };

        let json = serde_json::to_value(event).expect("serialize bench token velocity");
        assert_eq!(json["type"], "BenchTokenVelocity");
        assert_eq!(json["tokens_per_second"], 42.5);
        assert_eq!(json["tokens_in"], 100);
        assert_eq!(json["tokens_out"], 50);
        assert_eq!(json["duration_ms"], 3529);
    }

    #[test]
    fn bench_agent_output_serializes() {
        let event = ServerEvent::BenchAgentOutput {
            bench_id: "run-1".into(),
            task_id: "task-3".into(),
            agent_id: "sonnet".into(),
            content: "hello world".into(),
            done: true,
            tool_calls: None,
            reasoning: Some("thought about it".into()),
        };

        let json = serde_json::to_value(event).expect("serialize bench agent output");
        assert_eq!(json["type"], "BenchAgentOutput");
        assert_eq!(json["agent_id"], "sonnet");
        assert_eq!(json["content"], "hello world");
        assert_eq!(json["done"], true);
        assert!(json.get("tool_calls").is_none());
        assert_eq!(json["reasoning"], "thought about it");
    }
}
