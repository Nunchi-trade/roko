//! Core event loop — drives the executor via `tokio::select!` over agent
//! events, gate completions, executor ticks, periodic flushes, and
//! cancellation.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::state_hub::StateHub;
use anyhow::Result;
use roko_core::{AgentRole, PhaseKind, PlanPhase, TaskCategory, TaskComplexityBand};
use roko_learn::contextual_bandit::{
    ActionSafetyBounds, BanditContextFeatures, BanditDecisionKind,
};
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, GateVerdict, Usage};
use roko_learn::model_router::RoutingContext;
use roko_learn::runtime_feedback::{
    CompletedRunInput, LearningPaths, LearningRuntime, RegressionConfig, RunnerFeedbackEvent,
};
use roko_orchestrator::{
    ExecutorAction, ExecutorConfig, ExecutorEvent, ExecutorSnapshot, GateResult, MergeQueue,
    MergeRequest, OrchestratorSnapshot, ParallelExecutor, PlanState as OrcPlanState,
    RecoveryEngine,
};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::dispatch::{
    AgentDispatchRequest, DispatchContext, Dispatcher, GateFeedback as DispatchGateFeedback,
    PromptAssembler, ResolvedAgentRuntime, WarmPool, resolve_agent_runtime,
    spawn_agent_result_bridge,
};
use crate::task_parser::TaskDef;

use super::agent_events::handle_agent_event;
use super::agent_stream::{AgentHandle, AgentSpawnConfig};
use super::gate_dispatch;
use super::merge::{MergeDispatch, PlanMerger, PlanMergerConfig};
use super::persist::{self, PersistPaths};
use super::plan_loader::Plan;
use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::{
    AgentCompletionSummary, AgentDispatchOutcome, AgentEvent, GateCompletion, GateCompletionKind,
    PlanOutcome, PlanRunSummary, PromptAssemblyDiagnostics, ResumeMarker, ResumeOutcome,
    RetryAction, RunConfig, RunOutcome, RunTotals, RunnerEvent, RunnerFailureKind,
    TaskAttemptOutcome, TaskAttemptRef,
};

// ─── RunReport ──────────────────────────────────────────────────────────

/// Summary of a completed run.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub plans: Vec<PlanReport>,
    pub total_tasks: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub total_cost_usd: f64,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub total_agent_calls: usize,
    pub duration: Duration,
}

/// Per-plan report.
#[derive(Debug, Clone)]
pub struct PlanReport {
    pub plan_id: String,
    pub completed: bool,
    pub tasks_total: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub gate_results: Vec<GateResult>,
}

impl RunReport {
    pub fn all_succeeded(&self) -> bool {
        self.tasks_failed == 0 && self.plans.iter().all(|p| p.completed)
    }
}

// ─── RunContext ──────────────────────────────────────────────────────────

/// Shared context for the dispatch loop, replacing 11 loose parameters.
struct RunContext<'a> {
    executor: &'a mut ParallelExecutor,
    task_index: &'a HashMap<String, HashMap<String, TaskDef>>,
    skip_enrichment: &'a HashMap<String, bool>,
    config: &'a RunConfig,
    tui: &'a TuiBridge,
    state: &'a mut RunState,
    agent_handle: &'a mut Option<AgentHandle>,
    agent_tx: &'a mpsc::Sender<AgentEvent>,
    gate_tx: &'a mpsc::Sender<GateCompletion>,
    paths: &'a PersistPaths,
    merge_queue: &'a MergeQueue,
}

// ─── Main Entry Point ───────────────────────────────────────────────────

/// Run all plans to completion (or cancellation).
pub async fn run(
    plans: Vec<Plan>,
    config: &RunConfig,
    state_hub: &StateHub,
    cancel: CancellationToken,
) -> Result<RunReport> {
    let max_concurrent_tasks = config.max_concurrent_tasks.max(1);

    let exec_config = ExecutorConfig {
        max_concurrent_plans: 4,
        max_concurrent_tasks,
        max_auto_fix_iterations: config.max_retries,
        task_timeout_secs: config.timeout_secs,
        ..Default::default()
    };

    let paths = PersistPaths::from_workdir(&config.workdir)?;
    persist::cleanup_orphaned_agents(&paths);

    // Ensure knowledge store directory exists for episode ingestion.
    let neuro_dir = config.workdir.join(".roko").join("neuro");
    if let Err(err) = std::fs::create_dir_all(&neuro_dir) {
        warn!(error = %err, "failed to create neuro directory");
    }

    // ── Strict resume validation + JSONL recovery ─────────────────────────
    //
    // Run before any state file is reopened. The validator:
    // 1. Loads `.roko/state/run-state.json` if present.
    // 2. Verifies every current task's fingerprint matches what the prior
    //    run recorded — drift is a hard error.
    // 3. Truncates `episodes.jsonl`, `events.jsonl`, and
    //    `efficiency.jsonl` after their last validated line (recovers
    //    from partial-append corruption left by a prior crash).
    //
    // On `ResumeError::TaskMismatch` / `PlanMissing` / `UnsupportedSchema`
    // the validator returns Err. We surface the failure and abort the
    // run so the operator can either edit the plan back into a known
    // state or discard the snapshot.
    {
        let mut plan_map: HashMap<String, Vec<TaskDef>> = HashMap::new();
        for plan in &plans {
            plan_map.insert(plan.id.clone(), plan.tasks.tasks.clone());
        }
        let prior_fingerprints = match persist::load_run_state(&paths) {
            Ok(Some(snapshot)) => snapshot.fingerprints,
            Ok(None) => Vec::new(),
            Err(err) => {
                warn!(error = %err, "failed to read prior run-state.json; treating as fresh run");
                Vec::new()
            }
        };
        match super::resume::prepare_resume(&paths, &plan_map, &prior_fingerprints) {
            Ok(report) => {
                if report.resumed {
                    info!(
                        prior_run_id = ?report.prior_run_id,
                        validated_tasks = report.validated_tasks,
                        "resume validated"
                    );
                }
                for f in &report.recovered_files {
                    use super::resume::JsonlRecoveryReport;
                    match &f.recovery {
                        JsonlRecoveryReport::Clean { .. } => {}
                        JsonlRecoveryReport::TruncatedTrailing {
                            truncated_bytes, ..
                        } => {
                            warn!(file = %f.path, truncated_bytes, "recovered partial JSONL");
                        }
                        JsonlRecoveryReport::DroppedInvalid { dropped_lines, .. } => {
                            warn!(file = %f.path, dropped_lines, "recovered malformed JSONL");
                        }
                    }
                }
            }
            Err(err) => {
                error!(error = %err, "resume validation failed; aborting run");
                return Err(anyhow::anyhow!("resume validation failed: {err}"));
            }
        }
    }

    // Build plan ID set for resume validation.
    let plan_ids: Vec<String> = plans.iter().map(|p| p.id.clone()).collect();

    // Only resume if snapshot exists AND its plans match the current run.
    let resume = load_executor(&paths, &exec_config, &plan_ids);
    let mut executor = resume.executor;
    let merge_queue = resume.merge_queue;

    // Index tasks by plan_id/task_id for lookup.
    let mut task_index: HashMap<String, HashMap<String, TaskDef>> = HashMap::new();
    let mut total_tasks = 0usize;

    for plan in &plans {
        // add_plan is a no-op if plan already exists (from snapshot).
        let orc_state = OrcPlanState::new(&plan.id);
        executor.add_plan(orc_state);

        let mut tasks_map = HashMap::new();
        for task in &plan.tasks.tasks {
            tasks_map.insert(task.id.clone(), task.clone());
            total_tasks += 1;
        }
        task_index.insert(plan.id.clone(), tasks_map);
    }

    // Channels.
    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(256);
    let (gate_tx, mut gate_rx) = mpsc::channel::<GateCompletion>(16);
    let learning_runtime = match LearningRuntime::open(
        LearningPaths::under(config.workdir.join(".roko").join("learn")),
        RegressionConfig::default(),
    )
    .await
    {
        Ok(runtime) => Some(runtime),
        Err(err) => {
            warn!(error = %err, "learning runtime unavailable; falling back to runner-local feedback logs");
            None
        }
    };

    // Seed playbooks if the store is empty (bootstrap chicken-and-egg).
    seed_playbooks_if_empty(&config.workdir).await;

    // State and TUI bridge.
    let tui = TuiBridge::new(state_hub.sender());
    let mut state = RunState::new(total_tasks);

    // Compute task fingerprints once at startup so every subsequent
    // `save_snapshot` writes them into `run-state.json` for the strict
    // resume validator to consume on the next run.
    state.task_fingerprints = plans
        .iter()
        .flat_map(|plan| {
            plan.tasks
                .tasks
                .iter()
                .map(move |task| persist::TaskDefFingerprint::from_task(task, &plan.id))
        })
        .collect();

    let mut agent_handle: Option<AgentHandle> = None;

    let skip_enrichment: HashMap<String, bool> = plans
        .iter()
        .map(|p| (p.id.clone(), p.tasks.meta.skip_enrichment))
        .collect();

    let mut tick_interval = interval(Duration::from_millis(100));
    let mut flush_interval = interval(Duration::from_secs(2));

    info!(
        plan_count = plans.len(),
        total_tasks, "starting runner v2 event loop"
    );
    let run_id = state.run_id().to_string();
    emit_runner_event(
        &paths,
        &mut state,
        &tui,
        config,
        RunnerEvent::resume_marker(&run_id, resume.marker.clone()),
    );
    emit_runner_event(
        &paths,
        &mut state,
        &tui,
        config,
        RunnerEvent::run_started(
            &run_id,
            plan_ids.clone(),
            total_tasks,
            matches!(resume.marker.outcome, ResumeOutcome::Resumed),
            config.resume_session.clone(),
        ),
    );

    // ─── Phase 0: Initialize subsystems ─────────────────────────────
    // Extension chain init (no-op with empty chain).
    if let Some(ext_chain) = &config.extension_chain {
        let mut chain = ext_chain.lock().await;
        let errors = chain.init_all().await;
        for (name, err) in &errors {
            warn!(extension = %name, error = %err, "extension init failed");
        }
        if !errors.is_empty() {
            info!(
                failed = errors.len(),
                "extension chain init completed with errors"
            );
        }
    }

    // Register MCP connectors in the connector registry.
    if let Some(registry) = &config.connector_registry {
        if let Some(mcp_path) = &config.mcp_config {
            if let Ok(contents) = std::fs::read_to_string(mcp_path) {
                if let Ok(mcp_json) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if let Some(servers) = mcp_json.get("mcpServers").and_then(|s| s.as_object()) {
                        if let Ok(mut reg) = registry.lock() {
                            for name in servers.keys() {
                                reg.register(roko_core::ConnectorInfo {
                                    name: name.clone(),
                                    kind: roko_core::ConnectorKind::Mcp,
                                    health: roko_core::ConnectorHealth {
                                        status: roko_core::ConnectorStatus::Connected,
                                        latency_ms: 0,
                                        last_check: chrono::Utc::now(),
                                    },
                                    created_at: chrono::Utc::now(),
                                    metadata: serde_json::Value::Null,
                                });
                            }
                            info!(count = servers.len(), "registered MCP connectors");
                        }
                    }
                }
            }
        }
    }

    loop {
        // Cancel-safety analysis:
        //   Branch 1 (agent_rx.recv): cancel-safe — mpsc::Receiver::recv drops no data.
        //   Branch 2 (gate_rx.recv):  cancel-safe — mpsc::Receiver::recv drops no data.
        //   Branch 3 (tick_interval): cancel-safe — Interval::tick is restartable.
        //   Branch 4 (flush_interval): cancel-safe — Interval::tick is restartable.
        //   Branch 5 (cancel.cancelled): cancel-safe — CancellationToken is idempotent.
        tokio::select! {
            // ─── Branch 1: Agent events ─────────────────────────────
            Some(event) = agent_rx.recv() => {
                let is_turn_done = matches!(&event, AgentEvent::TurnCompleted { .. });
                let is_exited = matches!(&event, AgentEvent::Exited { .. });
                let turn_completed_before_event = state.agent_turn_completed;
                let turn_error = matches!(&event, AgentEvent::TurnCompleted { is_error: true, .. });

                handle_agent_event(&event, &mut state, &tui);
                append_agent_event(&paths, &event, &state);

                // Per-turn budget check.
                if is_turn_done {
                    let max_turn = config.max_turn_usd;
                    if max_turn > 0.0 && state.cost_usd > max_turn {
                        warn!(
                            task = %state.current_task,
                            turn_cost = state.cost_usd,
                            limit = max_turn,
                            "single turn exceeded per-turn budget limit"
                        );
                    }
                }

                if is_turn_done {
                    if let AgentEvent::TurnCompleted {
                        session_id,
                        total_cost_usd,
                        num_turns,
                        is_error,
                    } = &event
                    {
                        let agent_id = format!("{}/{}", state.plan_id, state.current_task);
                        let outcome = if *is_error {
                            AgentDispatchOutcome::Failed
                        } else {
                            AgentDispatchOutcome::Completed
                        };
                        let attempt = state.current_attempt_ref();
                        let run_id = state.run_id().to_string();
                        emit_runner_event(
                            &paths,
                            &mut state,
                            &tui,
                            config,
                            RunnerEvent::agent_completed(
                                &run_id,
                                attempt,
                                &agent_id,
                                outcome,
                                AgentCompletionSummary {
                                    session_id: session_id.clone(),
                                    total_cost_usd: *total_cost_usd,
                                    turns: *num_turns,
                                    exit_code: None,
                                    message: (*is_error)
                                        .then(|| "agent reported an error result".to_string()),
                                },
                            ),
                        );
                    }

                    // Extension: post_inference hook.
                    fire_post_inference_hook(
                        config,
                        &state.plan_id,
                        &state.current_task,
                        &state.agent_model,
                        !turn_error,
                        state.cost_usd,
                        state.task_elapsed_ms(),
                        &tui,
                    )
                    .await;

                    let plan_id = state.plan_id.clone();
                    if !plan_id.is_empty() {
                        if turn_error {
                            let message = "agent reported an error result".to_string();
                            fire_on_error_hook(config, &message, "agent_turn", &tui, &state.plan_id, &state.current_task).await;
                            tui.error(&message);
                            let _ = executor.apply_event(&plan_id, &ExecutorEvent::Fatal(message));
                        } else {
                            apply_agent_completion(&mut executor, &plan_id, &tui);
                        }
                        save_snapshot(&executor, &paths, &mut state, &merge_queue);
                    }
                }

                if is_exited {
                    let exit_code = if let Some(handle) = agent_handle.take() {
                        let pid = handle.pid;
                        let code = handle.wait().await;
                        roko_agent::process::unregister_pid(pid);
                        code
                    } else if let AgentEvent::Exited { exit_code } = event {
                        exit_code
                    } else {
                        None
                    };

                    let plan_id = state.plan_id.clone();
                    if !turn_completed_before_event && !plan_id.is_empty() {
                        let agent_id = format!("{}/{}", state.plan_id, state.current_task);
                        if exit_code.unwrap_or(0) == 0 {
                            let attempt = state.current_attempt_ref();
                            let run_id = state.run_id().to_string();
                            emit_runner_event(
                                &paths,
                                &mut state,
                                &tui,
                                config,
                                RunnerEvent::agent_completed(
                                    &run_id,
                                    attempt,
                                    &agent_id,
                                    AgentDispatchOutcome::Exited,
                                    AgentCompletionSummary {
                                        exit_code,
                                        ..AgentCompletionSummary::default()
                                    },
                                ),
                            );
                            apply_agent_completion(&mut executor, &plan_id, &tui);
                        } else {
                            let message = format!(
                                "agent process exited unsuccessfully: exit_code={}",
                                exit_code.map_or_else(|| "unknown".into(), |code| code.to_string())
                            );
                            let attempt = state.current_attempt_ref();
                            let run_id = state.run_id().to_string();
                            emit_runner_event(
                                &paths,
                                &mut state,
                                &tui,
                                config,
                                RunnerEvent::agent_completed(
                                    &run_id,
                                    attempt,
                                    &agent_id,
                                    AgentDispatchOutcome::Failed,
                                    AgentCompletionSummary {
                                        exit_code,
                                        message: Some(message.clone()),
                                        ..AgentCompletionSummary::default()
                                    },
                                ),
                            );
                            tui.error(&message);
                            let _ = executor.apply_event(&plan_id, &ExecutorEvent::Fatal(message));
                        }
                    }

                    save_snapshot(&executor, &paths, &mut state, &merge_queue);
                }
            }

            // ─── Branch 2: Verify completions ─────────────────────────
            Some(completion) = gate_rx.recv() => {
                let effect_key = gate_effect_key(
                    &completion.plan_id,
                    &completion.task_id,
                    completion.rung,
                    completion.kind,
                );
                state.clear_gate_active(&effect_key);
                state.gate_output = completion.output.clone();
                let completion_attempt = TaskAttemptRef::new(
                    completion.plan_id.clone(),
                    completion.task_id.clone(),
                    state.iteration.max(1),
                );

                for v in &completion.verdicts {
                    tui.gate_result(
                        &completion.plan_id,
                        &completion.task_id,
                        &v.gate_name,
                        v.passed,
                    );
                }
                if completion.kind == GateCompletionKind::Gate {
                    if let Some(plan_state) = executor.plan_state_mut(&completion.plan_id) {
                        for verdict in &completion.verdicts {
                            plan_state.gate_results.push(GateResult {
                                gate_name: verdict.gate_name.clone(),
                                rung: completion.rung,
                                passed: verdict.passed,
                                summary: verdict.summary.clone(),
                                duration_ms: completion.duration_ms,
                                test_count: None,
                            });
                        }
                    }
                }
                let run_id = state.run_id().to_string();
                emit_runner_event(
                    &paths,
                    &mut state,
                    &tui,
                    config,
                    RunnerEvent::gate_completed(
                        &run_id,
                        completion_attempt.clone(),
                        &completion,
                    ),
                );

                if completion.kind == GateCompletionKind::Merge {
                    emit_runner_event(
                        &paths,
                        &mut state,
                        &tui,
                        config,
                        RunnerEvent::merge_backend_completed(
                            &run_id,
                            completion_attempt.clone(),
                            &completion,
                            merge_branch_from_task_id(&completion.task_id),
                            conflict_paths_from_merge_output(&completion.output),
                        ),
                    );
                }

                // Emit learning events for the completed agent+gate cycle.
                emit_feedback(
                    &completion,
                    &state,
                    &config.workdir,
                    &paths,
                    learning_runtime.as_ref(),
                    &tui,
                    config,
                )
                .await;

                // Extension: on_gate hook.
                fire_on_gate_hook(config, &completion, &tui).await;

                if completion.kind == GateCompletionKind::Merge {
                    handle_merge_completion(
                        &completion,
                        &mut executor,
                        &mut state,
                        &paths,
                        &merge_queue,
                        &gate_tx,
                        &config.workdir,
                        Duration::from_secs(config.timeout_secs),
                        &tui,
                        config,
                    );
                    continue;
                }

                if completion.kind == GateCompletionKind::PlanVerify {
                    handle_plan_verify_completion(
                        &completion,
                        &mut executor,
                        &mut state,
                        &paths,
                        &merge_queue,
                        &tui,
                        config,
                    );
                    continue;
                }

                if completion.passed && completion.rung < config.max_gate_rung {
                    state.clear_retry_backoff(&completion.plan_id);
                    info!(
                        plan_id = %completion.plan_id,
                        task_id = %completion.task_id,
                        rung = completion.rung,
                        max_gate_rung = config.max_gate_rung,
                        "gate rung passed — advancing to next configured rung"
                    );
                    continue;
                }

                if completion.passed {
                    // Mark this task completed in the DAG and check for more.
                    state.clear_retry_backoff(&completion.plan_id);
                    state.mark_task_completed(&completion.plan_id, &completion.task_id);
                    state.task_completed();
                    let run_id = state.run_id().to_string();
                    emit_runner_event(
                        &paths,
                        &mut state,
                        &tui,
                        config,
                        RunnerEvent::task_attempt_completed(
                            &run_id,
                            completion_attempt.clone(),
                            TaskAttemptOutcome::Passed,
                            None,
                            completion.duration_ms,
                        ),
                    );
                    tui.task_completed(&completion.plan_id, &completion.task_id, "passed");

                    let completed = state.plan_completed_tasks(&completion.plan_id);
                    let completed_plans = completed_plan_ids(&executor, &task_index);
                    let has_more = task_index
                        .get(completion.plan_id.as_str())
                        .map(|tasks| {
                            tasks
                                .values()
                                .any(|t| {
                                    !completed.contains(&t.id)
                                        && t.is_ready_with_plan_deps(completed, &completed_plans)
                                })
                        })
                        .unwrap_or(false);

                    if has_more {
                        // More tasks remain — force plan back to Implementing so
                        // the next tick resolves the next ready task.
                        if let Some(ps) = executor.plan_state_mut(&completion.plan_id) {
                            ps.gate_results.clear();
                            ps.current_phase = PlanPhase::Implementing;
                        }
                        let remaining = task_index.get(completion.plan_id.as_str())
                            .map(|t| t.len().saturating_sub(completed.len())).unwrap_or(0);
                        info!(
                            plan_id = %completion.plan_id,
                            remaining,
                            "task passed — advancing to next task"
                        );
                    } else {
                        // All tasks done — let the plan proceed to completion.
                        let _ = executor.apply_event(&completion.plan_id, &ExecutorEvent::GatePassed);
                        info!(plan_id = %completion.plan_id, "all tasks passed — plan completing");

                        // Trigger dream consolidation after plan completion.
                        tokio::spawn({
                            let workdir = config.workdir.clone();
                            async move {
                                info!("triggering dream consolidation after plan completion");
                                let dream_config = roko_dreams::DreamLoopConfig {
                                    auto_dream: true,
                                    idle_threshold_mins: 0,
                                    min_episodes_for_dream: 1,
                                    agent: roko_dreams::DreamAgentConfig {
                                        command: "claude".to_string(),
                                        args: Vec::new(),
                                        model: None,
                                        bare_mode: true,
                                        effort: "low".to_string(),
                                        fallback_model: None,
                                        timeout_ms: 120_000,
                                        env: Vec::new(),
                                    },
                                };
                                let mut dream_runner = roko_dreams::DreamRunner::new(
                                    workdir.join(".roko"),
                                    dream_config,
                                );
                                match dream_runner.consolidate_now() {
                                    Ok(report) => info!(
                                        knowledge_entries = report.knowledge_entries_written,
                                        playbooks = report.playbooks_created,
                                        "dream consolidation completed"
                                    ),
                                    Err(err) => warn!(error = %err, "dream consolidation failed"),
                                }
                            }
                        });
                    }
                } else {
                    let failure_kind = completion
                        .failure_kind
                        .unwrap_or_else(|| RunnerFailureKind::from_output(&completion.output));
                    let can_retry = executor
                        .plan_state(&completion.plan_id)
                        .map(|ps| ps.iteration <= config.max_retries && failure_kind.is_retryable())
                        .unwrap_or(false);
                    if can_retry {
                        match executor.apply_event(&completion.plan_id, &ExecutorEvent::GateFailed) {
                            Ok(phase) => {
                                let mut next_attempt = None;
                                let mut cooldown_ms = 0;
                                if let Some(ps) = executor.plan_state_mut(&completion.plan_id) {
                                    let attempt = ps.iteration;
                                    ps.reset_for_retry();
                                    state.set_retry_backoff(
                                        &completion.plan_id,
                                        failure_kind,
                                        attempt,
                                    );
                                    state.iteration = ps.iteration;
                                    next_attempt = Some(ps.iteration);
                                    cooldown_ms = state
                                        .retry_cooldown_remaining(&completion.plan_id)
                                        .map(|duration| duration.as_millis() as u64)
                                        .unwrap_or_default();
                                }
                                let run_id = state.run_id().to_string();
                                emit_runner_event(
                                    &paths,
                                    &mut state,
                                    &tui,
                                    config,
                                    RunnerEvent::retry_decision(
                                        &run_id,
                                        completion_attempt.clone(),
                                        RetryAction::RetryAfterBackoff,
                                        failure_kind,
                                        next_attempt,
                                        cooldown_ms,
                                        "gate failed and retry policy allows auto-fix".to_string(),
                                    ),
                                );
                                tui.phase_transition(&completion.plan_id, "gating", &format!("{phase:?}"));
                                info!(
                                    plan_id = %completion.plan_id,
                                    phase = ?phase,
                                    failure_kind = ?failure_kind,
                                    "gate failed — entering auto-fix"
                                );

                                // On 3rd+ retry, enrich the task prompt with failure analysis.
                                if state.iteration >= 3 {
                                    let replan_context = format!(
                                        "\n\n## IMPORTANT: Prior attempts failed\n\
                                         This is attempt {}. Previous gate failures:\n{}\n\
                                         Analyze WHY previous approaches failed and try a fundamentally different strategy.",
                                        state.iteration + 1,
                                        completion.output.chars().take(2000).collect::<String>(),
                                    );
                                    state.set_replan_context(
                                        &completion.plan_id,
                                        &completion.task_id,
                                        replan_context,
                                    );
                                }
                            }
                            Err(e) => {
                                warn!(plan_id = %completion.plan_id, err = %e, "transition error after gate failure");
                            }
                        }
                    } else {
                        state.task_failed();
                        tui.task_completed(&completion.plan_id, &completion.task_id, "failed");
                        let reason = if failure_kind.is_retryable() {
                            format!("gate failed and retries exhausted: {}", completion.output)
                        } else {
                            format!(
                                "gate failed with non-retryable {failure_kind:?} failure: {}",
                                completion.output
                            )
                        };
                        let run_id = state.run_id().to_string();
                        emit_runner_event(
                            &paths,
                            &mut state,
                            &tui,
                            config,
                            RunnerEvent::retry_decision(
                                &run_id,
                                completion_attempt.clone(),
                                if failure_kind.is_retryable() {
                                    RetryAction::Exhausted
                                } else {
                                    RetryAction::NotRetryable
                                },
                                failure_kind,
                                None,
                                0,
                                reason.clone(),
                            ),
                        );
                        let run_id = state.run_id().to_string();
                        emit_runner_event(
                            &paths,
                            &mut state,
                            &tui,
                            config,
                            RunnerEvent::task_attempt_completed(
                                &run_id,
                                completion_attempt.clone(),
                                if failure_kind.is_retryable() {
                                    TaskAttemptOutcome::Exhausted
                                } else {
                                    TaskAttemptOutcome::Failed
                                },
                                Some(failure_kind),
                                completion.duration_ms,
                            ),
                        );
                        let _ = executor.apply_event(
                            &completion.plan_id,
                            &ExecutorEvent::Fatal(reason.clone()),
                        );
                        tui.error(&reason);
                    }
                }

                save_snapshot(&executor, &paths, &mut state, &merge_queue);
            }

            // ─── Branch 3: Executor tick ────────────────────────────
            _ = tick_interval.tick() => {
                let actions = executor.tick();
                for action in actions {
                    let mut ctx = RunContext {
                        executor: &mut executor,
                        task_index: &task_index,
                        skip_enrichment: &skip_enrichment,
                        config,
                        tui: &tui,
                        state: &mut state,
                        agent_handle: &mut agent_handle,
                        agent_tx: &agent_tx,
                        gate_tx: &gate_tx,
                        paths: &paths,
                        merge_queue: &merge_queue,
                    };
                    dispatch_action(&action, &mut ctx).await;
                }
            }

            // ─── Branch 4: Periodic flush ───────────────────────────
            _ = flush_interval.tick() => {
                save_snapshot(&executor, &paths, &mut state, &merge_queue);
                if let Some(ref handle) = agent_handle {
                    let _ = persist::save_agent_pids(&paths, &[handle.pid]);
                }
            }

            // ─── Branch 5: Cancellation ─────────────────────────────
            _ = cancel.cancelled() => {
                warn!("cancellation requested — shutting down");
                if let Some(handle) = agent_handle.take() {
                    handle.kill(Duration::from_secs(3)).await;
                }
                save_snapshot(&executor, &paths, &mut state, &merge_queue);
                shutdown_subsystems(config, &tui).await;
                let event =
                    build_run_completed_event(&executor, &plans, &state, RunOutcome::Cancelled);
                emit_runner_event(&paths, &mut state, &tui, config, event);
                break;
            }
        }

        if all_plans_terminal(&executor, &plans) {
            save_snapshot(&executor, &paths, &mut state, &merge_queue);
            let outcome = if build_report(&executor, &plans, &state).all_succeeded() {
                RunOutcome::Succeeded
            } else {
                RunOutcome::Failed
            };
            let event = build_run_completed_event(&executor, &plans, &state, outcome);
            emit_runner_event(&paths, &mut state, &tui, config, event);
            info!("all plans terminal — exiting event loop");
            break;
        }
    }

    // Shutdown Phase 0 subsystems and persist learned state.
    shutdown_subsystems(config, &tui).await;

    Ok(build_report(&executor, &plans, &state))
}

fn apply_agent_completion(executor: &mut ParallelExecutor, plan_id: &str, tui: &TuiBridge) {
    let Some(phase_kind) = executor
        .plan_state(plan_id)
        .map(|state| state.current_phase.kind())
    else {
        warn!(plan_id = %plan_id, "agent completed for unknown plan");
        return;
    };

    let event = match phase_kind {
        PhaseKind::Enriching => ExecutorEvent::EnrichmentDone,
        PhaseKind::Implementing => ExecutorEvent::ImplementationDone,
        PhaseKind::AutoFixing => ExecutorEvent::AutoFixDone,
        PhaseKind::RegeneratingVerify => ExecutorEvent::VerifyRegenDone,
        PhaseKind::Reviewing => ExecutorEvent::ReviewApproved,
        PhaseKind::DocRevision => ExecutorEvent::DocRevisionDone,
        _ => {
            info!(
                plan_id = %plan_id,
                phase = ?phase_kind,
                "agent completion ignored for phase"
            );
            return;
        }
    };

    match executor.apply_event(plan_id, &event) {
        Ok(phase) => {
            tui.phase_transition(plan_id, &format!("{phase_kind:?}"), &format!("{phase:?}"));
            info!(plan_id = %plan_id, from = ?phase_kind, phase = ?phase, "agent phase completed");
        }
        Err(e) => {
            warn!(plan_id = %plan_id, err = %e, "transition error after agent completion");
        }
    }
}

fn handle_plan_verify_completion(
    completion: &GateCompletion,
    executor: &mut ParallelExecutor,
    state: &mut RunState,
    paths: &PersistPaths,
    merge_queue: &MergeQueue,
    tui: &TuiBridge,
    config: &RunConfig,
) {
    if completion.passed {
        state.clear_retry_backoff(&completion.plan_id);
        match executor.apply_event(&completion.plan_id, &ExecutorEvent::VerifyPassed) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "verifying", &format!("{phase:?}"));
                info!(plan_id = %completion.plan_id, phase = ?phase, "plan verify passed");
            }
            Err(e) => warn!(
                plan_id = %completion.plan_id,
                err = %e,
                "transition error after plan verify pass"
            ),
        }
    } else {
        let failure_kind = completion
            .failure_kind
            .unwrap_or_else(|| RunnerFailureKind::from_output(&completion.output));
        state.set_retry_backoff(&completion.plan_id, failure_kind, state.iteration);
        let cooldown_ms = state
            .retry_cooldown_remaining(&completion.plan_id)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default();
        let run_id = state.run_id().to_string();
        let attempt = TaskAttemptRef::new(
            completion.plan_id.clone(),
            completion.task_id.clone(),
            state.iteration.max(1),
        );
        let next_attempt = Some(state.iteration.saturating_add(1).max(1));
        emit_runner_event(
            paths,
            state,
            tui,
            config,
            RunnerEvent::retry_decision(
                &run_id,
                attempt,
                RetryAction::RetryAfterBackoff,
                failure_kind,
                next_attempt,
                cooldown_ms,
                "plan verify failed and verify regeneration is available".to_string(),
            ),
        );
        match executor.apply_event(&completion.plan_id, &ExecutorEvent::VerifyFailed) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "verifying", &format!("{phase:?}"));
                warn!(
                    plan_id = %completion.plan_id,
                    failure_kind = ?failure_kind,
                    phase = ?phase,
                    "plan verify failed"
                );
            }
            Err(e) => {
                let reason = format!("plan verify failed: {}", completion.output);
                warn!(
                    plan_id = %completion.plan_id,
                    err = %e,
                    "transition error after plan verify failure"
                );
                let _ = executor.apply_event(&completion.plan_id, &ExecutorEvent::Fatal(reason));
            }
        }
    }

    save_snapshot(executor, paths, state, merge_queue);
}

fn merge_branch_from_task_id(task_id: &str) -> Option<String> {
    task_id
        .strip_prefix("merge:")
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(ToOwned::to_owned)
}

fn conflict_paths_from_merge_output(output: &str) -> Vec<String> {
    output
        .lines()
        .find_map(|line| {
            line.split_once("conflicted paths:")
                .map(|(_, paths)| paths.to_string())
        })
        .map(|paths| {
            paths
                .split([',', ' ', '\t'])
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn handle_merge_completion(
    completion: &GateCompletion,
    executor: &mut ParallelExecutor,
    state: &mut RunState,
    paths: &PersistPaths,
    merge_queue: &MergeQueue,
    gate_tx: &mpsc::Sender<GateCompletion>,
    workdir: &Path,
    regression_timeout: Duration,
    tui: &TuiBridge,
    config: &RunConfig,
) {
    let run_id = state.run_id().to_string();
    if completion.passed {
        match executor.apply_event(&completion.plan_id, &ExecutorEvent::MergeSucceeded) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "merging", &format!("{phase:?}"));
                tui.plan_completed(&completion.plan_id, true);
                emit_runner_event(
                    paths,
                    state,
                    tui,
                    config,
                    RunnerEvent::plan_completed(
                        &run_id,
                        &completion.plan_id,
                        PlanOutcome::Succeeded,
                        None,
                    ),
                );
                info!(
                    plan_id = %completion.plan_id,
                    output = %completion.output,
                    "merge finalized and regression passed"
                );
            }
            Err(err) => {
                let reason = format!("executor rejected successful merge: {err}");
                let _ = executor.apply_event(&completion.plan_id, &ExecutorEvent::Fatal(reason));
            }
        }
    } else {
        let reason = format!("merge failed: {}", completion.output);
        match executor.apply_event(&completion.plan_id, &ExecutorEvent::MergeFailed) {
            Ok(phase) => {
                tui.phase_transition(&completion.plan_id, "merging", &format!("{phase:?}"));
                tui.plan_completed(&completion.plan_id, false);
            }
            Err(err) => {
                warn!(
                    plan_id = %completion.plan_id,
                    error = %err,
                    "transition error after merge failure"
                );
                let _ = executor
                    .apply_event(&completion.plan_id, &ExecutorEvent::Fatal(reason.clone()));
            }
        }
        emit_runner_event(
            paths,
            state,
            tui,
            config,
            RunnerEvent::plan_completed(
                &run_id,
                &completion.plan_id,
                PlanOutcome::Failed,
                Some(reason.clone()),
            ),
        );
        tui.error(&reason);
    }

    if let Some(next_plan_id) = PlanMerger::new(
        merge_queue.clone(),
        PlanMergerConfig::new(workdir.to_path_buf(), regression_timeout),
    )
    .drain_next(gate_tx.clone())
    {
        info!(plan_id = %next_plan_id, "started next queued merge");
    }
    save_snapshot(executor, paths, state, merge_queue);
}

fn append_agent_event(paths: &PersistPaths, event: &AgentEvent, state: &RunState) {
    let event_type = event.event_type();

    let payload = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "timestamp_ms": chrono::Utc::now().timestamp_millis().max(0) as u64,
        "type": event_type,
        "run_id": state.run_id(),
        "plan_id": state.plan_id.clone(),
        "task_id": state.current_task.clone(),
        "attempt": state.iteration.max(1),
        "agent_pid": state.agent_pid,
        "event": agent_event_json(event),
    });

    if let Err(err) = persist::append_jsonl(&paths.events_jsonl, &payload) {
        warn!(error = %err, "failed to append runner event");
    }
}

fn agent_event_json(event: &AgentEvent) -> serde_json::Value {
    match event {
        AgentEvent::Started {
            agent_id,
            provider,
            model,
            pid,
        } => serde_json::json!({
            "agent_id": agent_id,
            "provider": provider,
            "model": model,
            "pid": pid,
        }),
        AgentEvent::SystemInit { session_id, model } => {
            serde_json::json!({"session_id": session_id, "model": model})
        }
        AgentEvent::MessageDelta { text } => serde_json::json!({"text": text}),
        AgentEvent::ToolCall { id, name } => serde_json::json!({"id": id, "name": name}),
        AgentEvent::ToolOutput { id, output } => serde_json::json!({"id": id, "output": output}),
        AgentEvent::TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        } => serde_json::json!({
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cache_read_tokens": cache_read_tokens,
            "cache_write_tokens": cache_write_tokens,
        }),
        AgentEvent::TurnCompleted {
            session_id,
            total_cost_usd,
            num_turns,
            is_error,
        } => serde_json::json!({
            "session_id": session_id,
            "total_cost_usd": total_cost_usd,
            "num_turns": num_turns,
            "is_error": is_error,
        }),
        AgentEvent::Error { message } => serde_json::json!({"message": message}),
        AgentEvent::Exited { exit_code } => serde_json::json!({"exit_code": exit_code}),
    }
}

/// Single emit path for runner lifecycle events.
///
/// Owns:
/// - state apply (`RunState::apply_runner_event`)
/// - TUI dashboard publish (`TuiBridge::runner_event`)
/// - durable JSONL append (`persist::append_runner_event`)
/// - **projection broadcast** (`config.projection`)
/// - **feedback fan-out** (`config.feedback_facade`, fire-and-forget)
///
/// Helpers that do not have `&RunConfig` in scope use
/// [`emit_runner_event_facadeless`] which is equivalent to passing
/// `None`/`None` for projection + feedback.
fn emit_runner_event(
    paths: &PersistPaths,
    state: &mut RunState,
    tui: &TuiBridge,
    config: &RunConfig,
    event: RunnerEvent,
) {
    emit_runner_event_with_facades(
        paths,
        state,
        tui,
        config.projection.as_ref(),
        config.feedback_facade.as_ref(),
        event,
    );
}

/// Drop-in for emit sites that do not hold a `&RunConfig` (helpers
/// invoked outside `run()`). Skips projection + feedback fan-out; the
/// runner-level emits still cover the lifecycle events these helpers
/// produce because the helpers themselves only emit on their plan's
/// completion which is also republished from `run()`.
fn emit_runner_event_facadeless(
    paths: &PersistPaths,
    state: &mut RunState,
    tui: &TuiBridge,
    event: RunnerEvent,
) {
    emit_runner_event_with_facades(paths, state, tui, None, None, event);
}

/// Internal variant accepting the optional projection + feedback facades.
fn emit_runner_event_with_facades(
    paths: &PersistPaths,
    state: &mut RunState,
    tui: &TuiBridge,
    projection: Option<&Arc<super::projection::Projection>>,
    feedback_facade: Option<&Arc<crate::runtime_feedback::FeedbackFacade>>,
    event: RunnerEvent,
) {
    state.apply_runner_event(&event);
    tui.runner_event(&event);
    if let Err(err) = persist::append_runner_event(paths, &event) {
        warn!(
            event_type = event.event_type(),
            error = %err,
            "failed to append runner lifecycle event"
        );
    }

    // ── Mirror to projection facade ─────────────────────────────────────
    if let Some(proj) = projection {
        let raw = super::projection::RawRuntimeEvent::Runner(event.clone());
        match proj.publish(raw) {
            Ok(()) => {}
            Err(super::projection::ProjectionError::NoSubscribers) => {
                // Publishing without live subscribers is normal during smoke
                // runs — the projection facade tracks the dropped-event
                // counter for diagnostics.
            }
        }
    }

    // ── Translate to FeedbackEvent and fan out (fire-and-forget) ────────
    if let Some(facade) = feedback_facade {
        if let Some(feedback) = runner_event_to_feedback(&event) {
            let facade = Arc::clone(facade);
            tokio::spawn(async move {
                if let Err(err) = facade.on_event(&feedback).await {
                    warn!(
                        event_type = feedback.label(),
                        %err,
                        "feedback facade returned terminal error",
                    );
                }
            });
        }
    }
}

/// Translate a [`RunnerEvent`] into a [`FeedbackEvent`] when the runner
/// has enough information for one. Returns `None` for variants that do
/// not map to the feedback layer (e.g. `RunStarted`, `ResumeMarker`).
fn runner_event_to_feedback(event: &RunnerEvent) -> Option<crate::runtime_feedback::FeedbackEvent> {
    use crate::dispatch::{AgentOutcome, ModelChoiceSource};
    use crate::runtime_feedback::FeedbackEvent;

    match event {
        RunnerEvent::TaskAttemptCompleted {
            attempt, outcome, ..
        } => {
            let succeeded = matches!(outcome, TaskAttemptOutcome::Passed);
            // The runner-level event does not carry per-attempt usage;
            // the dispatch layer overlays the real numbers when it is on
            // the hot loop. For now this fills `model` / `provider` /
            // tokens / cost with empty defaults — episodes still get
            // written; the routing sink dampens its observation
            // accordingly when the model slug is empty.
            let agent_outcome = AgentOutcome {
                task_id: attempt.task_id.clone(),
                plan_id: attempt.plan_id.clone(),
                model: String::new(),
                provider: String::new(),
                output: String::new(),
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: 0.0,
                duration_ms: 0,
                exit_code: None,
                is_error: !succeeded,
            };
            Some(FeedbackEvent::TaskCompleted {
                plan_id: attempt.plan_id.clone(),
                task_id: attempt.task_id.clone(),
                outcome: agent_outcome,
                model_source: ModelChoiceSource::Default,
                succeeded,
            })
        }
        RunnerEvent::GateCompleted {
            attempt,
            rung,
            passed,
            duration_ms,
            ..
        } => Some(FeedbackEvent::GateOutcome {
            plan_id: attempt.plan_id.clone(),
            task_id: attempt.task_id.clone(),
            rung: *rung,
            passed: *passed,
            duration_ms: *duration_ms,
        }),
        RunnerEvent::RetryDecision {
            attempt,
            cooldown_ms,
            current_attempt,
            ..
        } => Some(FeedbackEvent::RetryDecision {
            plan_id: attempt.plan_id.clone(),
            task_id: attempt.task_id.clone(),
            attempt: *current_attempt,
            backoff_secs: cooldown_ms / 1000,
        }),
        RunnerEvent::PlanCompleted {
            plan_id, outcome, ..
        } => {
            let succeeded = matches!(outcome, PlanOutcome::Succeeded);
            Some(FeedbackEvent::PlanCompleted {
                plan_id: plan_id.clone(),
                succeeded,
                tasks_completed: 0,
                tasks_failed: 0,
                total_cost_usd: 0.0,
            })
        }
        _ => None,
    }
}

fn build_run_completed_event(
    executor: &ParallelExecutor,
    plans: &[Plan],
    state: &RunState,
    outcome: RunOutcome,
) -> RunnerEvent {
    let report = build_report(executor, plans, state);
    RunnerEvent::run_completed(
        state.run_id(),
        outcome,
        RunTotals {
            total_tasks: report.total_tasks,
            tasks_completed: report.tasks_completed,
            tasks_failed: report.tasks_failed,
            total_agent_calls: report.total_agent_calls,
            total_cost_usd: report.total_cost_usd,
            duration_ms: report.duration.as_millis() as u64,
        },
        report
            .plans
            .into_iter()
            .map(|plan| PlanRunSummary {
                plan_id: plan.plan_id,
                completed: plan.completed,
                tasks_total: plan.tasks_total,
                tasks_completed: plan.tasks_completed,
                tasks_failed: plan.tasks_failed,
            })
            .collect(),
    )
}

// ─── Snapshot Helper ────────────────────────────────────────────────────

/// Save executor snapshot and track consecutive failures.
///
/// Writes three files atomically:
/// - `.roko/state/orchestrator.json` (aggregate)
/// - `.roko/state/executor.json` (orchestrator snapshot)
/// - `.roko/state/run-state.json` (runner-owned: cost, tokens,
///   completed-task set, run_id, fingerprints — used by
///   `runner::resume::prepare_resume`)
fn save_snapshot(
    executor: &ParallelExecutor,
    paths: &PersistPaths,
    state: &mut RunState,
    merge_queue: &MergeQueue,
) {
    let timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;
    let snapshot = executor.snapshot(timestamp_ms);
    let orchestrator_snapshot = OrchestratorSnapshot::new(snapshot.clone(), timestamp_ms)
        .with_merge_queue(merge_queue.snapshot());
    if let Err(err) = persist::save_orchestrator_snapshot(paths, &orchestrator_snapshot) {
        error!(error = %err, "failed to save orchestrator snapshot");
        state.snapshot_failed();
        return;
    }
    if let Err(e) = persist::save_executor_snapshot(paths, &snapshot) {
        error!(err = %e, "failed to save executor snapshot");
        state.snapshot_failed();
        return;
    }

    // Runner-owned run-state.json — cost, tokens, completed tasks,
    // fingerprints. Without this file the strict resume validator
    // cannot detect drift.
    let run_state = persist::RunStateSnapshot {
        schema_version: persist::RUN_STATE_SCHEMA_VERSION,
        run_id: state.run_id().to_string(),
        started_at_ms: state.started_at.elapsed().as_millis().saturating_sub(0) as u64,
        timestamp_ms,
        tasks_total: state.tasks_total,
        tasks_completed: state.tasks_completed,
        tasks_failed: state.tasks_failed,
        total_tokens_in: state.total_tokens_in,
        total_tokens_out: state.total_tokens_out,
        total_cost_usd: state.total_cost_usd,
        total_agent_calls: state.total_agent_calls,
        plan_costs: state.plan_costs.clone(),
        completed_tasks: state.completed_tasks.clone(),
        snapshot_fail_streak: state.snapshot_fail_streak,
        fingerprints: state.task_fingerprints.clone(),
    };
    match persist::save_run_state(paths, &run_state) {
        Ok(()) => state.snapshot_succeeded(),
        Err(e) => {
            error!(err = %e, "failed to save run-state snapshot");
            state.snapshot_failed();
        }
    }
}

// ─── Resume ─────────────────────────────────────────────────────────────

struct ResumeLoad {
    executor: ParallelExecutor,
    merge_queue: MergeQueue,
    marker: ResumeMarker,
}

/// Load a resumable executor snapshot when compatible, otherwise start fresh
/// and emit a structured resume marker explaining the decision.
fn load_executor(paths: &PersistPaths, config: &ExecutorConfig, plan_ids: &[String]) -> ResumeLoad {
    let snapshot_path = if paths.orchestrator_json.exists() {
        paths.orchestrator_json.display().to_string()
    } else {
        paths.executor_json.display().to_string()
    };
    if !paths.orchestrator_json.exists() && !paths.executor_json.exists() {
        return ResumeLoad {
            executor: ParallelExecutor::new(config.clone()),
            merge_queue: MergeQueue::new(),
            marker: ResumeMarker {
                outcome: ResumeOutcome::Fresh,
                snapshot_path,
                snapshot_plan_ids: Vec::new(),
                current_plan_ids: plan_ids.to_vec(),
                message: Some("no executor snapshot found".to_string()),
            },
        };
    }

    let (snapshot, merge_queue) = match load_orchestrator_checkpoint(paths) {
        Ok(Some((snapshot, merge_queue))) => (snapshot, merge_queue),
        Ok(None) => {
            let json = match std::fs::read_to_string(&paths.executor_json) {
                Ok(j) => j,
                Err(e) => {
                    warn!(err = %e, "failed to read executor snapshot");
                    return ResumeLoad {
                        executor: ParallelExecutor::new(config.clone()),
                        merge_queue: MergeQueue::new(),
                        marker: ResumeMarker {
                            outcome: ResumeOutcome::ReadFailed,
                            snapshot_path,
                            snapshot_plan_ids: Vec::new(),
                            current_plan_ids: plan_ids.to_vec(),
                            message: Some(format!("failed to read executor snapshot: {e}")),
                        },
                    };
                }
            };
            match ExecutorSnapshot::from_json(&json) {
                Ok(snapshot) => (snapshot, MergeQueue::new()),
                Err(e) => {
                    warn!(err = %e, "corrupt executor snapshot — starting fresh");
                    return ResumeLoad {
                        executor: ParallelExecutor::new(config.clone()),
                        merge_queue: MergeQueue::new(),
                        marker: ResumeMarker {
                            outcome: ResumeOutcome::Corrupt,
                            snapshot_path,
                            snapshot_plan_ids: Vec::new(),
                            current_plan_ids: plan_ids.to_vec(),
                            message: Some(format!("corrupt executor snapshot: {e}")),
                        },
                    };
                }
            }
        }
        Err(e) => {
            warn!(err = %e, "corrupt orchestrator snapshot — starting fresh");
            return ResumeLoad {
                executor: ParallelExecutor::new(config.clone()),
                merge_queue: MergeQueue::new(),
                marker: ResumeMarker {
                    outcome: ResumeOutcome::Corrupt,
                    snapshot_path,
                    snapshot_plan_ids: Vec::new(),
                    current_plan_ids: plan_ids.to_vec(),
                    message: Some(format!("corrupt orchestrator snapshot: {e}")),
                },
            };
        }
    };

    // Validate: snapshot must contain at least one of the current plan IDs.
    let snap_plan_ids: Vec<String> = snapshot.plan_states.keys().cloned().collect();
    let has_overlap = plan_ids
        .iter()
        .any(|id| snapshot.plan_states.contains_key(id));

    if snap_plan_ids.is_empty() || !has_overlap {
        info!(
            snapshot_plans = ?snap_plan_ids,
            current_plans = ?plan_ids,
            "stale executor snapshot (no plan overlap) — starting fresh"
        );
        return ResumeLoad {
            executor: ParallelExecutor::new(config.clone()),
            merge_queue: MergeQueue::new(),
            marker: ResumeMarker {
                outcome: ResumeOutcome::IgnoredStale,
                snapshot_path,
                snapshot_plan_ids: snap_plan_ids,
                current_plan_ids: plan_ids.to_vec(),
                message: Some("snapshot has no overlap with current plans".to_string()),
            },
        };
    }

    info!(
        path = %snapshot_path,
        plans = ?snap_plan_ids,
        "resuming from orchestrator snapshot"
    );
    let mut executor = ParallelExecutor::from_snapshot(config.clone(), snapshot.clone());
    let recovery = RecoveryEngine::new().recover_from_executor_snapshot(snapshot);
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let resume_plan = recovery.resume_plan(now_ms);
    for warning in &resume_plan.warnings {
        warn!(
            plan_id = %warning.plan_id,
            message = %warning.message,
            severity = ?warning.severity,
            "orchestrator recovery warning"
        );
    }
    for plan in &resume_plan.retryable_terminal {
        if executor.requeue_retryable_terminal(&plan.plan_id).is_some() {
            info!(
                plan_id = %plan.plan_id,
                retry_after_ms = ?plan.retry_after_ms,
                "requeued retryable terminal plan after recovery"
            );
        }
    }
    ResumeLoad {
        executor,
        merge_queue,
        marker: ResumeMarker {
            outcome: ResumeOutcome::Resumed,
            snapshot_path,
            snapshot_plan_ids: snap_plan_ids,
            current_plan_ids: plan_ids.to_vec(),
            message: Some("resumed from compatible executor snapshot".to_string()),
        },
    }
}

fn load_orchestrator_checkpoint(
    paths: &PersistPaths,
) -> Result<Option<(ExecutorSnapshot, MergeQueue)>, String> {
    if !paths.orchestrator_json.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&paths.orchestrator_json)
        .map_err(|err| format!("failed to read aggregate snapshot: {err}"))?;
    let snapshot = OrchestratorSnapshot::from_json(&json)
        .map_err(|err| format!("failed to parse aggregate snapshot: {err}"))?;
    let merge_queue = snapshot
        .merge_queue
        .map(MergeQueue::from_snapshot)
        .unwrap_or_else(MergeQueue::new);
    Ok(Some((snapshot.executor, merge_queue)))
}

// ─── Action Dispatcher ──────────────────────────────────────────────────

fn record_skipped_gate_rung(
    ctx: &mut RunContext<'_>,
    plan_id: &str,
    task_id: &str,
    rung: u32,
    gate_name: &str,
    summary: &str,
) {
    if let Some(plan_state) = ctx.executor.plan_state_mut(plan_id) {
        plan_state.gate_results.push(GateResult {
            gate_name: gate_name.to_string(),
            rung,
            passed: true,
            summary: summary.to_string(),
            duration_ms: 0,
            test_count: None,
        });
    }
    ctx.tui.gate_result(plan_id, task_id, gate_name, true);

    if rung >= ctx.config.max_gate_rung {
        if let Err(err) = ctx
            .executor
            .apply_event(plan_id, &ExecutorEvent::GatePassed)
        {
            warn!(plan_id = %plan_id, rung, error = %err, "failed to advance after skipped final gate");
        }
    } else {
        debug!(
            plan_id = %plan_id,
            task_id = %task_id,
            rung,
            max_gate_rung = ctx.config.max_gate_rung,
            "skipped gate rung recorded; advancing to next rung"
        );
    }
}

async fn dispatch_action(action: &ExecutorAction, ctx: &mut RunContext<'_>) {
    match action {
        ExecutorAction::DispatchPlan { plan_id } => {
            info!(plan_id = %plan_id, "dispatching plan");
            ctx.tui.plan_started(plan_id);

            if let Err(e) = ctx.executor.apply_event(plan_id, &ExecutorEvent::Start) {
                error!(plan_id = %plan_id, err = %e, "failed to start plan");
                return;
            }
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::plan_started(&run_id, plan_id),
            );

            if ctx
                .skip_enrichment
                .get(plan_id.as_str())
                .copied()
                .unwrap_or(false)
            {
                if let Err(e) = ctx
                    .executor
                    .apply_event(plan_id, &ExecutorEvent::EnrichmentDone)
                {
                    error!(plan_id = %plan_id, err = %e, "failed to skip enrichment");
                }
                ctx.tui
                    .phase_transition(plan_id, "enriching", "implementing");
            }
        }

        ExecutorAction::SpawnAgent { plan_id, task, .. } => {
            // Resolve sentinel task names ("next", "fix", etc.) to actual task IDs
            // by walking the plan's DAG and finding the first ready task.
            let resolved_task = if task == "next" || task == "fix" || task == "regen-verify" {
                let completed = ctx.state.plan_completed_tasks(plan_id);
                let completed_plans = completed_plan_ids(ctx.executor, ctx.task_index);
                let plan_tasks = ctx.task_index.get(plan_id.as_str());
                plan_tasks.and_then(|tasks| {
                    // Collect all TaskDefs, then find the first ready one in definition order.
                    let mut all_tasks: Vec<&TaskDef> = tasks.values().collect();
                    all_tasks.sort_by(|a, b| a.id.cmp(&b.id));
                    all_tasks
                        .iter()
                        .find(|t| {
                            !completed.contains(&t.id)
                                && t.is_ready_with_plan_deps(completed, &completed_plans)
                        })
                        .map(|t| t.id.clone())
                })
            } else if matches!(task.as_str(), "review" | "doc-revision" | "docs" | "enrich") {
                if ctx.state.current_task.is_empty() {
                    ctx.task_index
                        .get(plan_id.as_str())
                        .and_then(|tasks| tasks.values().min_by(|a, b| a.id.cmp(&b.id)))
                        .map(|t| t.id.clone())
                } else {
                    Some(ctx.state.current_task.clone())
                }
            } else {
                Some(task.clone())
            };

            let task_id = match resolved_task {
                Some(id) => id,
                None => {
                    // No more ready tasks — all done for this plan.
                    info!(plan_id = %plan_id, "no more ready tasks — implementation complete");
                    let _ = ctx
                        .executor
                        .apply_event(plan_id, &ExecutorEvent::ImplementationDone);
                    return;
                }
            };

            if ctx.state.agent_active || ctx.agent_handle.is_some() {
                debug!(
                    plan_id = %plan_id,
                    task = %task_id,
                    current_plan = %ctx.state.plan_id,
                    current_task = %ctx.state.current_task,
                    "agent already active — suppressing duplicate spawn"
                );
                return;
            }

            if let Some(remaining) = ctx.state.retry_cooldown_remaining(plan_id) {
                debug!(
                    plan_id = %plan_id,
                    task = %task_id,
                    cooldown_ms = remaining.as_millis(),
                    "retry backoff active — delaying spawn"
                );
                return;
            }

            info!(plan_id = %plan_id, task = %task_id, "spawning agent");

            // Per-plan budget check.
            let max_plan_usd = ctx.config.max_plan_usd;
            let plan_spent = ctx.state.plan_cost(plan_id);
            if max_plan_usd > 0.0 && plan_spent >= max_plan_usd {
                warn!(
                    plan_id = %plan_id,
                    spent = plan_spent,
                    limit = max_plan_usd,
                    "plan budget exceeded — aborting"
                );
                ctx.tui.error(&format!(
                    "budget exceeded: ${plan_spent:.2} >= ${max_plan_usd:.2}"
                ));
                let _ = ctx.executor.apply_event(
                    plan_id,
                    &ExecutorEvent::Fatal(format!(
                        "budget exceeded: ${plan_spent:.2} >= ${max_plan_usd:.2}"
                    )),
                );
                return;
            }

            let task_def = match ctx
                .task_index
                .get(plan_id.as_str())
                .and_then(|m| m.get(task_id.as_str()))
            {
                Some(t) => t,
                None => {
                    error!(plan_id = %plan_id, task = %task_id, "task not found in index");
                    let _ = ctx.executor.apply_event(
                        plan_id,
                        &ExecutorEvent::Fatal(format!("task {task_id} not found")),
                    );
                    return;
                }
            };

            let previous_gate_output = ctx.state.gate_output.clone();
            let attempt_num = ctx
                .executor
                .plan_state(plan_id)
                .map(|state| state.iteration)
                .unwrap_or(1);
            ctx.state.reset_for_task(plan_id, &task_id);
            ctx.state.iteration = attempt_num;
            ctx.state.total_agent_calls += 1;
            ctx.state.task_agent_calls += 1;

            let role = task_def.role.as_deref().unwrap_or("implementer");
            let gate_feedback = DispatchGateFeedback::from_raw(&previous_gate_output);
            let dispatch_ctx = DispatchContext {
                plan_id: plan_id.clone(),
                role: role.to_string(),
                workdir: ctx.config.workdir.clone(),
                model_hint: Some(ctx.config.model.clone()),
                force_backend: ctx.config.cli_model_override.clone(),
                budget_remaining_usd: if ctx.config.max_plan_usd > 0.0 {
                    (ctx.config.max_plan_usd - ctx.state.plan_cost(plan_id)).max(0.0)
                } else {
                    f64::INFINITY
                },
                attempt: attempt_num.saturating_sub(1),
                gate_feedback,
            };
            let dispatcher = Dispatcher::new(
                ctx.config.cascade_router.clone(),
                PromptAssembler::new(),
                WarmPool::new(0),
            );
            let dispatch_plan = match dispatcher.plan(task_def, &dispatch_ctx) {
                Ok(plan) => plan,
                Err(err) => {
                    let message = format!("dispatch planning failed: {err}");
                    error!(plan_id = %plan_id, task = %task_id, error = %message);
                    let _ = ctx
                        .executor
                        .apply_event(plan_id, &ExecutorEvent::Fatal(message.clone()));
                    ctx.tui.error(&message);
                    return;
                }
            };
            let requested_model = dispatch_plan.model.slug.clone();
            let prompt_diagnostics = dispatch_plan.prompt.diagnostics.clone();
            ctx.tui
                .model_selected(plan_id, &task_id, &requested_model, "dispatcher");
            let system_prompt = dispatch_plan.prompt.system_prompt;
            let mut final_prompt = dispatch_plan.prompt.user_prompt;
            debug!(
                plan_id = %plan_id,
                task = %task_id,
                model = %requested_model,
                included_sections = ?dispatch_plan.prompt.diagnostics.included_sections,
                dropped_sections = ?dispatch_plan.prompt.diagnostics.dropped_sections,
                knowledge_ids = ?dispatch_plan.prompt.diagnostics.knowledge_ids,
                playbook_ids = ?dispatch_plan.prompt.diagnostics.playbook_ids,
                "dispatch prompt assembled"
            );

            // Append replan context before prompt diagnostics so the durable
            // event captures the actual prompt shape sent to the runtime.
            if let Some(replan) = ctx.state.take_replan_context(plan_id, &task_id) {
                final_prompt.push_str(&replan);
            }

            // Extension: pre-inference hook.
            fire_pre_inference_hook(ctx.config, plan_id, &task_id, &requested_model, ctx.tui).await;

            let dispatch = match resolve_agent_runtime(
                ctx.config.roko_config.as_ref(),
                &requested_model,
            ) {
                Ok(selection) => selection,
                Err(message) => {
                    error!(plan_id = %plan_id, task = %task_id, error = %message, "agent provider resolution failed");
                    let _ = ctx
                        .executor
                        .apply_event(plan_id, &ExecutorEvent::Fatal(message.clone()));
                    ctx.tui.error(&message);
                    return;
                }
            };

            let agent_id = format!("{plan_id}/{task_id}");
            let attempt_ref = TaskAttemptRef::new(plan_id.clone(), task_id.clone(), attempt_num);
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::task_attempt_started(&run_id, attempt_ref.clone(), &task_def.title),
            );
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::prompt_assembled(
                    &run_id,
                    attempt_ref.clone(),
                    role,
                    &requested_model,
                    system_prompt.len(),
                    final_prompt.len(),
                    PromptAssemblyDiagnostics {
                        included_sections: prompt_diagnostics.included_sections,
                        dropped_sections: prompt_diagnostics.dropped_sections,
                        estimated_tokens: prompt_diagnostics.estimated_tokens,
                        knowledge_ids: prompt_diagnostics.knowledge_ids,
                        playbook_ids: prompt_diagnostics.playbook_ids,
                    },
                ),
            );
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::agent_dispatch_started(
                    &run_id,
                    attempt_ref.clone(),
                    &agent_id,
                    role,
                    &requested_model,
                ),
            );

            match dispatch {
                ResolvedAgentRuntime::Cli {
                    model,
                    cli_provider,
                } => {
                    let model_display = model.clone();
                    let mut spawn_config = AgentSpawnConfig::from_run_config(
                        ctx.config,
                        final_prompt,
                        system_prompt,
                        model,
                        agent_id.clone(),
                    );
                    if let Some(provider) = cli_provider {
                        spawn_config = spawn_config.with_cli_provider(provider);
                    }

                    match dispatcher
                        .spawn_streaming_cli_agent(&spawn_config, ctx.agent_tx.clone())
                        .await
                    {
                        Ok(handle) => {
                            ctx.state.agent_active = true;
                            ctx.state.agent_pid = Some(handle.pid);
                            emit_runner_event(
                                ctx.paths,
                                ctx.state,
                                ctx.tui,
                                ctx.config,
                                RunnerEvent::agent_dispatch_completed(
                                    &run_id,
                                    attempt_ref,
                                    &agent_id,
                                    AgentDispatchOutcome::Spawned,
                                    Some(model_display.clone()),
                                    Some(handle.pid),
                                    None,
                                ),
                            );
                            ctx.tui.agent_spawned(&agent_id, role, &model_display);
                            ctx.tui.task_started(
                                plan_id,
                                &task_id,
                                &task_def.title,
                                "implementing",
                            );
                            *ctx.agent_handle = Some(handle);
                            register_agent_feed(ctx.config, plan_id, &task_id, &agent_id, ctx.tui);
                        }
                        Err(e) => {
                            error!(err = %e, "failed to spawn agent");
                            let message = format!("agent spawn failed: {e}");
                            emit_runner_event(
                                ctx.paths,
                                ctx.state,
                                ctx.tui,
                                ctx.config,
                                RunnerEvent::agent_dispatch_completed(
                                    &run_id,
                                    attempt_ref.clone(),
                                    &agent_id,
                                    AgentDispatchOutcome::SpawnFailed,
                                    Some(model_display),
                                    None,
                                    Some(message.clone()),
                                ),
                            );
                            emit_runner_event(
                                ctx.paths,
                                ctx.state,
                                ctx.tui,
                                ctx.config,
                                RunnerEvent::task_attempt_completed(
                                    &run_id,
                                    attempt_ref,
                                    TaskAttemptOutcome::Failed,
                                    Some(RunnerFailureKind::Resource),
                                    0,
                                ),
                            );
                            ctx.tui.error(&message);
                            let _ = ctx.executor.apply_event(
                                plan_id,
                                &ExecutorEvent::Fatal(format!("spawn failed: {e}")),
                            );
                        }
                    }
                }
                ResolvedAgentRuntime::Bridge {
                    model,
                    provider_id,
                    roko_config,
                } => {
                    ctx.state.agent_active = true;
                    ctx.state.agent_pid = None;
                    let request = AgentDispatchRequest {
                        model_key: requested_model.clone(),
                        prompt: final_prompt,
                        system_prompt,
                        workdir: ctx.config.workdir.clone(),
                        agent_id: agent_id.clone(),
                        command: None,
                        timeout_ms: Some(ctx.config.timeout_secs.saturating_mul(1000)),
                        mcp_config: ctx.config.mcp_config.clone(),
                        env: vec![
                            ("CARGO_INCREMENTAL".to_string(), "0".to_string()),
                            ("CARGO_BUILD_JOBS".to_string(), "2".to_string()),
                        ],
                        extra_args: Vec::new(),
                        effort: None,
                        tools: None,
                        bare_mode: false,
                        dangerously_skip_permissions: ctx.config.dangerously_skip_permissions,
                    };
                    spawn_agent_result_bridge(roko_config, request, ctx.agent_tx.clone());
                    emit_runner_event(
                        ctx.paths,
                        ctx.state,
                        ctx.tui,
                        ctx.config,
                        RunnerEvent::agent_dispatch_completed(
                            &run_id,
                            attempt_ref,
                            &agent_id,
                            AgentDispatchOutcome::Spawned,
                            Some(model.clone()),
                            None,
                            None,
                        ),
                    );
                    ctx.tui
                        .agent_spawned(&agent_id, role, &format!("{provider_id}:{model}"));
                    ctx.tui
                        .task_started(plan_id, &task_id, &task_def.title, "implementing");
                    register_agent_feed(ctx.config, plan_id, &task_id, &agent_id, ctx.tui);
                }
            }
        }

        ExecutorAction::RunGate { plan_id, rung } => {
            let task_id = ctx.state.current_task.clone();
            // Honor gates config: skip clippy rung (1) if disabled, skip test rung (2) if skip_tests.
            if *rung == 1 && !ctx.config.clippy_enabled {
                info!(plan_id = %plan_id, rung = rung, "skipping clippy gate (disabled in config)");
                record_skipped_gate_rung(
                    ctx,
                    plan_id,
                    &task_id,
                    *rung,
                    "clippy",
                    "skipped: clippy disabled in config",
                );
                return;
            }
            if *rung == 2 && ctx.config.skip_tests {
                info!(plan_id = %plan_id, rung = rung, "skipping test gate (skip_tests in config)");
                record_skipped_gate_rung(
                    ctx,
                    plan_id,
                    &task_id,
                    *rung,
                    "test",
                    "skipped: tests disabled in config",
                );
                return;
            }

            info!(plan_id = %plan_id, rung = rung, "dispatching gate");
            let effect_key = gate_effect_key(plan_id, &task_id, *rung, GateCompletionKind::Gate);
            if !ctx.state.mark_gate_active(effect_key.clone()) {
                debug!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    rung = rung,
                    "gate already active — suppressing duplicate dispatch"
                );
                return;
            }
            let run_id = ctx.state.run_id().to_string();
            let attempt_ref =
                TaskAttemptRef::new(plan_id.clone(), task_id.clone(), ctx.state.iteration.max(1));
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::gate_dispatch_started(
                    &run_id,
                    attempt_ref,
                    GateCompletionKind::Gate,
                    *rung,
                ),
            );
            let verify_steps = ctx
                .task_index
                .get(plan_id.as_str())
                .and_then(|tasks| tasks.get(task_id.as_str()))
                .map(|task| task.verify.clone())
                .unwrap_or_default();
            gate_dispatch::spawn_gate(
                plan_id.clone(),
                task_id,
                *rung,
                ctx.config.workdir.clone(),
                verify_steps,
                ctx.config.timeout_secs,
                ctx.gate_tx.clone(),
            );
        }

        ExecutorAction::RunVerify { plan_id } => {
            let verify_steps = ctx
                .task_index
                .get(plan_id.as_str())
                .map(|tasks| {
                    let mut tasks: Vec<_> = tasks.values().collect();
                    tasks.sort_by(|a, b| a.id.cmp(&b.id));
                    tasks
                        .into_iter()
                        .filter(|task| !task.verify.is_empty())
                        .map(|task| (task.id.clone(), task.verify.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            if verify_steps.is_empty() {
                info!(plan_id = %plan_id, "no declared plan verify steps — passing verify phase");
                let _ = ctx
                    .executor
                    .apply_event(plan_id, &ExecutorEvent::VerifyPassed);
                return;
            }

            let effect_key = gate_effect_key(
                plan_id,
                "plan-verify",
                u32::MAX,
                GateCompletionKind::PlanVerify,
            );
            if !ctx.state.mark_gate_active(effect_key.clone()) {
                debug!(
                    plan_id = %plan_id,
                    "plan verify already active — suppressing duplicate dispatch"
                );
                return;
            }
            let run_id = ctx.state.run_id().to_string();
            let attempt_ref =
                TaskAttemptRef::new(plan_id.clone(), "plan-verify", ctx.state.iteration.max(1));
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::gate_dispatch_started(
                    &run_id,
                    attempt_ref,
                    GateCompletionKind::PlanVerify,
                    u32::MAX,
                ),
            );

            info!(
                plan_id = %plan_id,
                task_count = verify_steps.len(),
                "dispatching plan verify"
            );
            gate_dispatch::spawn_plan_verify(
                plan_id.clone(),
                ctx.config.workdir.clone(),
                verify_steps,
                ctx.config.timeout_secs,
                ctx.gate_tx.clone(),
            );
        }

        ExecutorAction::CompletePlan { plan_id } => {
            info!(plan_id = %plan_id, "plan completed");
            ctx.tui.plan_completed(plan_id, true);
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::plan_completed(&run_id, plan_id, PlanOutcome::Succeeded, None),
            );
            save_snapshot(ctx.executor, ctx.paths, ctx.state, ctx.merge_queue);
        }

        ExecutorAction::FailPlan { plan_id, reason } => {
            warn!(plan_id = %plan_id, reason = %reason, "plan failed");
            ctx.state.task_failed();
            ctx.tui
                .task_completed(&ctx.state.plan_id, &ctx.state.current_task, "failed");
            ctx.tui.plan_completed(plan_id, false);
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                ctx.config,
                RunnerEvent::plan_completed(
                    &run_id,
                    plan_id,
                    PlanOutcome::Failed,
                    Some(reason.clone()),
                ),
            );
        }

        ExecutorAction::MergeBranch { plan_id } => {
            let files_changed = ctx
                .executor
                .plan_state(plan_id)
                .map(|state| state.files_changed.clone())
                .unwrap_or_default();
            let request = MergeRequest::new(
                plan_id.clone(),
                format!("roko/plan/{plan_id}"),
                files_changed,
                0,
            );
            let merger = PlanMerger::new(
                ctx.merge_queue.clone(),
                PlanMergerConfig::new(
                    ctx.config.workdir.clone(),
                    Duration::from_secs(ctx.config.timeout_secs),
                ),
            );
            match merger.submit(request, ctx.gate_tx.clone()) {
                MergeDispatch::Reserved {
                    plan_id,
                    branch_name,
                } => {
                    info!(
                        plan_id = %plan_id,
                        branch = %branch_name,
                        "reserved merge queue request"
                    );
                    save_snapshot(ctx.executor, ctx.paths, ctx.state, ctx.merge_queue);
                }
                MergeDispatch::Blocked { plan_id } => {
                    info!(
                        plan_id = %plan_id,
                        blocked_conflicts = ?ctx.merge_queue.blocked_conflicts(),
                        "merge queued but currently blocked by file locks"
                    );
                    save_snapshot(ctx.executor, ctx.paths, ctx.state, ctx.merge_queue);
                }
            }
        }

        _ => {
            info!(action = ?action, "auto-advancing action");
        }
    }
}

// ─── Learning Emission ──────────────────────────────────────────────────

/// Emit all runner-owned feedback after a gate cycle completes.
async fn emit_feedback(
    completion: &GateCompletion,
    state: &RunState,
    workdir: &Path,
    paths: &PersistPaths,
    learning_runtime: Option<&LearningRuntime>,
    tui: &TuiBridge,
    config: &RunConfig,
) {
    let episode = build_episode(completion, state);
    let efficiency_event = build_efficiency_event(completion, state);

    if let Err(err) = persist::append_jsonl(&paths.episodes_jsonl, &episode) {
        error!(error = %err, "failed to append runner episode");
    } else {
        tui.efficiency_event(
            &completion.plan_id,
            &completion.task_id,
            "episode_logged",
            1.0,
        );
    }

    if let Err(err) = persist::append_jsonl(&paths.efficiency_jsonl, &efficiency_event) {
        error!(error = %err, "failed to append runner efficiency event");
    }

    if let Some(runtime) = learning_runtime {
        let mut input = CompletedRunInput::from_episode(episode.clone());
        input.provider = Some(runtime_backend(state));
        if let Err(err) = runtime
            .record_runner_event(RunnerFeedbackEvent::CompletedRun {
                input: Box::new(input),
            })
            .await
        {
            warn!(error = %err, "learning runtime rejected completed-run feedback");
        }
    }

    let lifecycle = roko_neuro::RuntimeKnowledgeLifecycle::for_workdir(workdir);
    match lifecycle.ingest_episode(&episode) {
        Ok(record) => {
            debug!(
                record_id = %record.record_id,
                episode_id = %record.episode_id,
                "knowledge lifecycle ingested runner episode"
            );
        }
        Err(err) => {
            warn!(error = %err, "knowledge lifecycle ingestion failed");
        }
    }

    // CascadeRouter observation: record gate outcome for learned model selection.
    observe_cascade_router(config, state, completion, tui);

    // Bandit feedback: record decision context and outcome.
    observe_bandit_policy(config, state, completion, paths, tui);

    // Update adaptive gate thresholds based on this verdict.
    update_gate_thresholds(workdir, completion.rung, completion.passed);
}

/// Update EMA-based adaptive gate thresholds for a given rung.
fn update_gate_thresholds(workdir: &Path, rung: u32, passed: bool) {
    let path = workdir
        .join(".roko")
        .join("learn")
        .join("gate-thresholds.json");
    let mut thresholds: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"rungs": {}}));

    let rungs = thresholds.get_mut("rungs").and_then(|r| r.as_object_mut());
    if let Some(rungs) = rungs {
        let key = rung.to_string();
        let entry = rungs.entry(key).or_insert_with(
            || serde_json::json!({"pass_count": 0, "total_count": 0, "ema_pass_rate": 0.5}),
        );
        if let Some(obj) = entry.as_object_mut() {
            let total = obj.get("total_count").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
            let passes = obj.get("pass_count").and_then(|v| v.as_u64()).unwrap_or(0)
                + if passed { 1 } else { 0 };
            let alpha = 0.1;
            let old_ema = obj
                .get("ema_pass_rate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5);
            let new_ema = alpha * (if passed { 1.0 } else { 0.0 }) + (1.0 - alpha) * old_ema;
            obj.insert("pass_count".into(), serde_json::json!(passes));
            obj.insert("total_count".into(), serde_json::json!(total));
            obj.insert("ema_pass_rate".into(), serde_json::json!(new_ema));
        }
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&thresholds).unwrap_or_default(),
    );
}

/// Record gate outcome in the cascade router for learned model selection.
fn observe_cascade_router(
    config: &RunConfig,
    state: &RunState,
    completion: &GateCompletion,
    tui: &TuiBridge,
) {
    let Some(router) = &config.cascade_router else {
        return;
    };
    let model = &state.agent_model;
    if model.is_empty() {
        return;
    }
    let Some(model_idx) = router.model_index_for_slug(model) else {
        debug!(model = %model, "cascade router: model not in slug list, skipping observation");
        return;
    };

    let quality = if completion.passed { 1.0 } else { 0.0 };
    let normalized_cost = (state.cost_usd / 1.0).clamp(0.0, 1.0); // Normalize against $1 reference
    let wall_secs = state.task_elapsed_ms() as f64 / 1000.0;
    let normalized_latency = (wall_secs / 300.0).clamp(0.0, 1.0); // Normalize against 5min reference

    let ctx = RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity: TaskComplexityBand::Standard,
        iteration: state.iteration.saturating_sub(1),
        role: AgentRole::Implementer,
        crate_familiarity: 0.5,
        has_prior_failure: state.iteration > 1,
        conductor_load: 0.0,
        active_agents: 1,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: roko_core::DaimonPolicy::default(),
        thinking_level: None,
        temperament: None,
        previous_model: None,
        plan_context_tokens: None,
        tier_thresholds: None,
    };
    let weights = roko_core::config::schema::RewardWeights::default();
    router.observe_multi_objective(
        ctx.to_features(),
        model_idx,
        quality,
        normalized_cost,
        normalized_latency,
        &weights,
    );
    debug!(
        model = %model,
        quality,
        cost = normalized_cost,
        latency = normalized_latency,
        "cascade router: recorded observation"
    );

    tui.cascade_router_updated(&router.snapshot_json());
}

/// Record bandit feedback for model-selection decisions.
fn observe_bandit_policy(
    config: &RunConfig,
    state: &RunState,
    completion: &GateCompletion,
    _paths: &PersistPaths,
    tui: &TuiBridge,
) {
    let Some(bandit) = &config.bandit_policy else {
        return;
    };
    let model = &state.agent_model;
    if model.is_empty() {
        return;
    }

    let context = BanditContextFeatures::new(
        BanditDecisionKind::ProviderModelRouting,
        "implementation",
        &completion.plan_id,
        "implementer",
    );
    let observation = roko_learn::contextual_bandit::BanditRewardObservation {
        action_id: format!("model:{model}"),
        context_key: context.context_key(),
        success: completion.passed,
        quality: if completion.passed { 1.0 } else { 0.0 },
        metrics: roko_learn::contextual_bandit::RewardMetrics {
            latency_ms: Some(state.task_elapsed_ms()),
            cost_usd: Some(state.cost_usd),
            total_tokens: Some(state.tokens_in + state.tokens_out),
            retry_count: state.iteration.saturating_sub(1),
        },
    };
    let bounds = ActionSafetyBounds::default();

    if let Ok(mut policy) = bandit.try_lock() {
        if let Some(candidate) = policy.record_reward(observation, bounds) {
            // Persist bandit decision to JSONL for offline analysis.
            let bandit_log = config
                .workdir
                .join(".roko")
                .join("learn")
                .join("bandit-decisions.jsonl");
            if let Err(err) = persist::append_jsonl(&bandit_log, &candidate) {
                warn!(error = %err, "failed to append bandit decision");
            }
        }
        debug!(model = %model, passed = completion.passed, "bandit policy: recorded reward");
        tui.extension_hook(
            &completion.plan_id,
            &completion.task_id,
            "bandit_reward",
            completion.passed,
        );
    } else {
        warn!("bandit policy lock contended, skipping feedback");
    }
}

// ─── Extension Chain Hooks ───────────────────────────────────────────────

/// Fire pre_inference extension hook (non-blocking try_lock to avoid stalling select).
async fn fire_pre_inference_hook(
    config: &RunConfig,
    plan_id: &str,
    task_id: &str,
    model: &str,
    tui: &TuiBridge,
) {
    let Some(ext_chain) = &config.extension_chain else {
        return;
    };
    let Ok(chain) = ext_chain.try_lock() else {
        warn!("extension chain lock contended, skipping pre_inference hook");
        return;
    };
    let mut req = roko_core::extension::InferenceRequest {
        plan_id: plan_id.to_string(),
        task: task_id.to_string(),
        role: "implementer".to_string(),
        model: model.to_string(),
        prompt_tokens: 0,
        extra: serde_json::Value::Null,
    };
    let success = chain.run_pre_inference(&mut req).await.is_ok();
    if !success {
        warn!("extension pre_inference hook failed");
    }
    tui.extension_hook(plan_id, task_id, "pre_inference", success);
}

/// Fire post_inference extension hook.
async fn fire_post_inference_hook(
    config: &RunConfig,
    plan_id: &str,
    task_id: &str,
    model: &str,
    success: bool,
    cost_usd: f64,
    wall_ms: u64,
    tui: &TuiBridge,
) {
    let Some(ext_chain) = &config.extension_chain else {
        return;
    };
    let Ok(chain) = ext_chain.try_lock() else {
        warn!("extension chain lock contended, skipping post_inference hook");
        return;
    };
    let mut resp = roko_core::extension::InferenceResponse {
        plan_id: plan_id.to_string(),
        task: task_id.to_string(),
        role: "implementer".to_string(),
        model: model.to_string(),
        success,
        cost_usd,
        wall_ms,
        extra: serde_json::Value::Null,
    };
    let hook_ok = chain.run_post_inference(&mut resp).await.is_ok();
    if !hook_ok {
        warn!("extension post_inference hook failed");
    }
    tui.extension_hook(plan_id, task_id, "post_inference", hook_ok);
}

/// Fire on_gate extension hook.
async fn fire_on_gate_hook(config: &RunConfig, completion: &GateCompletion, tui: &TuiBridge) {
    let Some(ext_chain) = &config.extension_chain else {
        return;
    };
    let Ok(chain) = ext_chain.try_lock() else {
        warn!("extension chain lock contended, skipping on_gate hook");
        return;
    };
    for verdict in &completion.verdicts {
        let mut event = roko_core::extension::GateEvent {
            plan_id: completion.plan_id.clone(),
            gate_name: verdict.gate_name.clone(),
            passed: verdict.passed,
            rung: format!("rung-{}", completion.rung),
            duration_ms: completion.duration_ms,
            details: serde_json::Value::Null,
        };
        let hook_ok = chain.run_on_gate(&mut event).await.is_ok();
        if !hook_ok {
            warn!(gate = %verdict.gate_name, "extension on_gate hook failed");
        }
        tui.extension_hook(
            &completion.plan_id,
            &completion.task_id,
            &format!("on_gate:{}", verdict.gate_name),
            hook_ok,
        );
    }
}

/// Fire on_error extension hook.
async fn fire_on_error_hook(
    config: &RunConfig,
    message: &str,
    source: &str,
    tui: &TuiBridge,
    plan_id: &str,
    task_id: &str,
) {
    let Some(ext_chain) = &config.extension_chain else {
        return;
    };
    let Ok(chain) = ext_chain.try_lock() else {
        warn!("extension chain lock contended, skipping on_error hook");
        return;
    };
    let event = roko_core::extension::ErrorEvent {
        error_message: message.to_string(),
        source: source.to_string(),
        extra: serde_json::Value::Null,
    };
    let hook_ok = chain.run_on_error(&event).await.is_ok();
    tui.extension_hook(plan_id, task_id, "on_error", hook_ok);
}

/// Shutdown extension chain + persist cascade router.
async fn shutdown_subsystems(config: &RunConfig, tui: &TuiBridge) {
    // Extension chain shutdown.
    if let Some(ext_chain) = &config.extension_chain {
        let mut chain = ext_chain.lock().await;
        let errors = chain.shutdown_all().await;
        for (name, err) in &errors {
            warn!(extension = %name, error = %err, "extension shutdown failed");
        }
    }

    // Persist cascade router learned state.
    if let Some(router) = &config.cascade_router {
        let router_path = config
            .workdir
            .join(".roko")
            .join("learn")
            .join("cascade-router.json");
        if let Err(err) = router.save(&router_path) {
            warn!(error = %err, "failed to persist cascade router");
        } else {
            info!("cascade router state persisted");
            tui.cascade_router_updated(&router.snapshot_json());
        }
    }
}

/// Register an agent feed entry after successful spawn.
fn register_agent_feed(
    config: &RunConfig,
    plan_id: &str,
    task_id: &str,
    agent_id: &str,
    tui: &TuiBridge,
) {
    let Some(registry) = &config.feed_registry else {
        return;
    };
    if let Ok(mut reg) = registry.try_lock() {
        reg.register(roko_core::FeedInfo {
            id: String::new(), // Auto-assigned by registry
            name: format!("{plan_id}/{task_id}"),
            agent_id: agent_id.to_string(),
            kind: roko_core::FeedKind::Raw,
            access: roko_core::FeedAccess::Private,
            description: String::new(),
            schema: None,
            created_at: chrono::Utc::now(),
        });
        tui.extension_hook(plan_id, task_id, "feed_registered", true);
    }
}

/// Build the canonical episode for a completed agent+gate cycle.
fn build_episode(completion: &GateCompletion, state: &RunState) -> Episode {
    let agent_id = format!("{}/{}", completion.plan_id, completion.task_id);
    let gate_verdicts: Vec<GateVerdict> = completion
        .verdicts
        .iter()
        .map(|v| {
            let mut gv = GateVerdict::new(&v.gate_name, v.passed);
            if !v.summary.is_empty() {
                gv = gv.with_signature(&v.summary);
            }
            gv
        })
        .collect();

    let task_wall_ms = state.task_elapsed_ms();
    let mut ep = Episode::new(&agent_id, &completion.task_id);
    ep.kind = "runner_task_gate".to_string();
    ep.model = state.agent_model.clone();
    ep.backend = runtime_backend(state);
    ep.agent_template = "implementer".to_string();
    ep.trigger_kind = format!("{:?}", completion.kind).to_ascii_lowercase();
    ep.duration_secs = task_wall_ms as f64 / 1000.0;
    ep.usage = Usage {
        input_tokens: state.tokens_in,
        output_tokens: state.tokens_out,
        cache_read_tokens: state.cache_read_tokens,
        cache_write_tokens: state.cache_write_tokens,
        cost_usd: state.cost_usd,
        wall_ms: task_wall_ms,
        ..Usage::default()
    };
    ep.gate_verdicts = gate_verdicts;
    ep.turns = u64::from(state.task_agent_calls);
    ep.tokens_used = state.tokens_in + state.tokens_out;
    ep.extra.insert(
        "plan_id".to_string(),
        serde_json::Value::String(completion.plan_id.clone()),
    );
    ep.extra.insert(
        "run_id".to_string(),
        serde_json::Value::String(state.run_id().to_string()),
    );
    ep.extra.insert(
        "iteration".to_string(),
        serde_json::json!(state.iteration.max(1)),
    );
    ep.extra
        .insert("rung".to_string(), serde_json::json!(completion.rung));
    ep.extra.insert(
        "gate_kind".to_string(),
        serde_json::Value::String(format!("{:?}", completion.kind).to_ascii_lowercase()),
    );
    ep.extra.insert(
        "gate_duration_ms".to_string(),
        serde_json::json!(completion.duration_ms),
    );
    if let Some(failure_kind) = completion.failure_kind {
        ep.extra.insert(
            "failure_kind".to_string(),
            serde_json::Value::String(format!("{failure_kind:?}").to_ascii_lowercase()),
        );
        ep.extra.insert(
            "retryable".to_string(),
            serde_json::json!(failure_kind.is_retryable()),
        );
        ep.extra.insert(
            "retry_status".to_string(),
            serde_json::Value::String(
                if completion.passed {
                    "succeeded"
                } else if failure_kind.is_retryable() {
                    "scheduled"
                } else {
                    "not_retryable"
                }
                .to_string(),
            ),
        );
        ep.extra.insert(
            "retry_attempt".to_string(),
            serde_json::json!(state.iteration.max(1)),
        );
        ep.extra.insert(
            "retry_scheduled".to_string(),
            serde_json::json!(!completion.passed && failure_kind.is_retryable()),
        );
    } else if completion.passed && state.iteration > 1 {
        ep.extra.insert(
            "retry_status".to_string(),
            serde_json::Value::String("succeeded".to_string()),
        );
        ep.extra.insert(
            "retry_attempt".to_string(),
            serde_json::json!(state.iteration.max(1)),
        );
    }

    if completion.passed {
        ep = ep.succeeded();
    } else {
        ep = ep.failed(&completion.output);
    }

    ep.attach_all_fingerprints();
    ep
}

/// Build the legacy raw efficiency event consumed by existing dashboards.
fn build_efficiency_event(completion: &GateCompletion, state: &RunState) -> AgentEfficiencyEvent {
    let model = runtime_model(state);
    let task_wall_ms = state.task_elapsed_ms();

    let gate_errors: Vec<String> = completion
        .verdicts
        .iter()
        .filter(|v| !v.passed)
        .map(|v| format!("{}: {}", v.gate_name, v.summary))
        .collect();

    AgentEfficiencyEvent {
        agent_id: format!("{}/{}", completion.plan_id, completion.task_id),
        role: "implementer".to_string(),
        backend: runtime_backend(state),
        model: model.clone(),
        plan_id: completion.plan_id.clone(),
        task_id: completion.task_id.clone(),
        input_tokens: state.tokens_in,
        output_tokens: state.tokens_out,
        cache_read_tokens: state.cache_read_tokens,
        cache_write_tokens: state.cache_write_tokens,
        cost_usd: state.cost_usd,
        wall_time_ms: task_wall_ms,
        duration_ms: task_wall_ms,
        iteration: state.iteration,
        gate_passed: completion.passed,
        outcome: if completion.passed {
            "success"
        } else {
            "failure"
        }
        .to_string(),
        gate_errors,
        model_used: model,
        timestamp: chrono::Utc::now().to_rfc3339(),
        ..AgentEfficiencyEvent::default()
    }
}

fn runtime_model(state: &RunState) -> String {
    if state.agent_model.trim().is_empty() {
        "unknown".to_string()
    } else {
        state.agent_model.clone()
    }
}

fn runtime_backend(state: &RunState) -> String {
    if state.agent_provider.trim().is_empty() {
        "unknown".to_string()
    } else {
        state.agent_provider.clone()
    }
}

// ─── Playbook Seeding ────────────────────────────────────────────────────

/// Seed the playbook store with starter templates when empty.
///
/// This solves the chicken-and-egg problem: playbooks are normally only
/// saved on task SUCCESS, but without playbooks the system has no guidance
/// from the start. These seeds give the first few runs structured advice.
async fn seed_playbooks_if_empty(workdir: &Path) {
    use roko_learn::playbook::{Playbook, PlaybookStep, PlaybookStore};

    let pb_dir = workdir.join(".roko").join("learn").join("playbooks");

    // Quick check: if the directory exists and has any .json files, skip.
    if pb_dir.exists() {
        if let Ok(mut entries) = tokio::fs::read_dir(&pb_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                    debug!("playbook store already has entries, skipping seed");
                    return;
                }
            }
        }
    }

    info!("playbook store empty — seeding with starter templates");

    let store = PlaybookStore::new(&pb_dir);

    let seeds: Vec<Playbook> = vec![
        {
            let mut pb = Playbook::new(
                "minimal-edit",
                "Make targeted edits to existing code. Keep diffs under 30 lines. Do not create new files unless explicitly required.",
            );
            pb.name = "Minimal Edit".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Search codebase for the relevant function/type",
                    "search",
                    vec!["file_found".into()],
                ),
                PlaybookStep::new(
                    1,
                    "Read the target file to understand context",
                    "read_file",
                    vec!["context_loaded".into()],
                ),
                PlaybookStep::new(
                    2,
                    "Make the minimal edit that satisfies the requirement",
                    "edit_file",
                    vec!["file_modified".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Verify the change compiles",
                    "run_command",
                    vec!["compile_success".into()],
                ),
            ];
            pb
        },
        {
            let mut pb = Playbook::new(
                "test-first",
                "Write or update tests first, then implement. Verify tests pass before finishing.",
            );
            pb.name = "Test First".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Identify the test file for the target module",
                    "search",
                    vec!["test_file_found".into()],
                ),
                PlaybookStep::new(
                    1,
                    "Write a failing test that captures the requirement",
                    "edit_file",
                    vec!["test_added".into()],
                ),
                PlaybookStep::new(
                    2,
                    "Implement the code to make the test pass",
                    "edit_file",
                    vec!["implementation_done".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Run the test suite and verify all tests pass",
                    "run_command",
                    vec!["tests_pass".into()],
                ),
            ];
            pb
        },
        {
            let mut pb = Playbook::new(
                "grep-before-write",
                "Search the codebase before writing new code. Check if the function/type already exists.",
            );
            pb.name = "Grep Before Write".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Search for existing implementations of the target",
                    "search",
                    vec!["search_complete".into()],
                ),
                PlaybookStep::new(
                    1,
                    "If found, extend or modify rather than duplicate",
                    "read_file",
                    vec!["existing_found".into()],
                ),
                PlaybookStep::new(
                    2,
                    "Implement changes in the existing location",
                    "edit_file",
                    vec!["change_applied".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Verify no duplicate definitions introduced",
                    "search",
                    vec!["no_duplicates".into()],
                ),
            ];
            pb
        },
        {
            let mut pb = Playbook::new(
                "wire-not-build",
                "Connect existing code rather than reimplementing. Check what already exists before creating anything new.",
            );
            pb.name = "Wire Not Build".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Search for the target struct/function in the codebase",
                    "search",
                    vec!["target_found".into()],
                ),
                PlaybookStep::new(
                    1,
                    "Trace the call chain to find where it should be wired",
                    "read_file",
                    vec!["call_site_found".into()],
                ),
                PlaybookStep::new(
                    2,
                    "Add the function call or import at the correct call site",
                    "edit_file",
                    vec!["wired".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Verify the feature is accessible via CLI or API",
                    "run_command",
                    vec!["feature_reachable".into()],
                ),
            ];
            pb
        },
        {
            let mut pb = Playbook::new(
                "compile-check-loop",
                "After every edit, run cargo check. Fix errors immediately before proceeding to the next change.",
            );
            pb.name = "Compile Check Loop".to_string();
            pb.steps = vec![
                PlaybookStep::new(
                    0,
                    "Make a single logical change",
                    "edit_file",
                    vec!["change_made".into()],
                ),
                PlaybookStep::new(
                    1,
                    "Run cargo check to verify compilation",
                    "run_command",
                    vec!["compile_success".into()],
                ),
                PlaybookStep::new(
                    2,
                    "If errors, fix them before proceeding",
                    "edit_file",
                    vec!["errors_fixed".into()],
                ),
                PlaybookStep::new(
                    3,
                    "Repeat until all changes are applied and compiling",
                    "run_command",
                    vec!["all_clean".into()],
                ),
            ];
            pb
        },
    ];

    for pb in &seeds {
        if let Err(err) = store.save(pb).await {
            warn!(playbook = %pb.id, error = %err, "failed to seed playbook");
        } else {
            debug!(playbook = %pb.id, "seeded playbook");
        }
    }

    info!(
        count = seeds.len(),
        "playbook store seeded with starter templates"
    );
}

// ─── Helpers ────────────────────────────────────────────────────────────

fn all_plans_terminal(executor: &ParallelExecutor, plans: &[Plan]) -> bool {
    plans
        .iter()
        .all(|p| executor.plan_state(&p.id).map_or(true, |s| s.is_terminal()))
}

fn completed_plan_ids(
    executor: &ParallelExecutor,
    task_index: &HashMap<String, HashMap<String, TaskDef>>,
) -> Vec<String> {
    task_index
        .keys()
        .filter(|plan_id| {
            executor
                .plan_state(plan_id)
                .is_some_and(|state| matches!(state.current_phase, PlanPhase::Complete))
        })
        .cloned()
        .collect()
}

fn gate_effect_key(plan_id: &str, task_id: &str, rung: u32, kind: GateCompletionKind) -> String {
    format!("{kind:?}:{plan_id}:{task_id}:{rung}")
}

fn build_report(executor: &ParallelExecutor, plans: &[Plan], state: &RunState) -> RunReport {
    let plan_reports: Vec<PlanReport> = plans
        .iter()
        .map(|p| {
            let orc_state = executor.plan_state(&p.id);
            let completed = orc_state
                .map(|s| matches!(s.current_phase, PlanPhase::Complete))
                .unwrap_or(false);
            PlanReport {
                plan_id: p.id.clone(),
                completed,
                tasks_total: p.tasks.tasks.len(),
                tasks_completed: if completed { p.tasks.tasks.len() } else { 0 },
                tasks_failed: if !completed && orc_state.map_or(false, |s| s.is_terminal()) {
                    1
                } else {
                    0
                },
                gate_results: orc_state
                    .map(|state| state.gate_results.clone())
                    .unwrap_or_default(),
            }
        })
        .collect();

    RunReport {
        plans: plan_reports,
        total_tasks: state.tasks_total,
        tasks_completed: state.tasks_completed,
        tasks_failed: state.tasks_failed,
        total_cost_usd: state.total_cost_usd,
        total_tokens_in: state.total_tokens_in,
        total_tokens_out: state.total_tokens_out,
        total_agent_calls: state.total_agent_calls,
        duration: state.elapsed(),
    }
}
