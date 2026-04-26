//! Core event loop — drives the executor via `tokio::select!` over agent
//! events, gate completions, executor ticks, periodic flushes, and
//! cancellation.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use roko_core::state_hub::StateHub;
use roko_core::{PhaseKind, PlanPhase};
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, GateVerdict, Usage};
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

use crate::dispatch_v2::{
    AgentDispatchRequest, AgentDispatcherV2, CliProviderConfig, DispatchEvent,
    ProviderDispatchResolver, ProviderRuntime,
};
use crate::task_parser::TaskDef;

use super::agent_events::handle_agent_event;
use super::agent_stream::{self, AgentHandle, AgentSpawnConfig};
use super::gate_dispatch;
use super::persist::{self, PersistPaths};
use super::plan_loader::Plan;
use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::{
    AgentCompletionSummary, AgentDispatchOutcome, AgentEvent, GateCompletion, GateCompletionKind,
    PlanOutcome, PlanRunSummary, ResumeMarker, ResumeOutcome, RetryAction, RunConfig, RunOutcome,
    RunTotals, RunnerEvent, RunnerFailureKind, TaskAttemptOutcome, TaskAttemptRef,
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
    let exec_config = ExecutorConfig {
        max_concurrent_plans: 4,
        max_concurrent_tasks: 1,
        max_auto_fix_iterations: config.max_retries,
        task_timeout_secs: config.timeout_secs,
        ..Default::default()
    };

    let paths = PersistPaths::from_workdir(&config.workdir)?;
    persist::cleanup_orphaned_agents(&paths);

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

    // State and TUI bridge.
    let tui = TuiBridge::new(state_hub.sender());
    let mut state = RunState::new(total_tasks);
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
        RunnerEvent::resume_marker(&run_id, resume.marker.clone()),
    );
    emit_runner_event(
        &paths,
        &mut state,
        &tui,
        RunnerEvent::run_started(
            &run_id,
            plan_ids.clone(),
            total_tasks,
            matches!(resume.marker.outcome, ResumeOutcome::Resumed),
            config.resume_session.clone(),
        ),
    );

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

                    let plan_id = state.plan_id.clone();
                    if !plan_id.is_empty() {
                        if turn_error {
                            let message = "agent reported an error result".to_string();
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
                    RunnerEvent::gate_completed(
                        &run_id,
                        completion_attempt.clone(),
                        &completion,
                    ),
                );

                // Emit learning events for the completed agent+gate cycle.
                emit_feedback(
                    &completion,
                    &state,
                    &config.workdir,
                    &paths,
                    learning_runtime.as_ref(),
                    &tui,
                )
                .await;

                if completion.kind == GateCompletionKind::PlanVerify {
                    handle_plan_verify_completion(
                        &completion,
                        &mut executor,
                        &mut state,
                        &paths,
                        &merge_queue,
                        &tui,
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
                let event =
                    build_run_completed_event(&executor, &plans, &state, RunOutcome::Cancelled);
                emit_runner_event(&paths, &mut state, &tui, event);
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
            emit_runner_event(&paths, &mut state, &tui, event);
            info!("all plans terminal — exiting event loop");
            break;
        }
    }

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

fn emit_runner_event(
    paths: &PersistPaths,
    state: &mut RunState,
    tui: &TuiBridge,
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

enum ResolvedAgentDispatch {
    Cli {
        model: String,
        cli_provider: Option<CliProviderConfig>,
    },
    Bridge {
        model: String,
        provider_id: String,
        roko_config: Arc<roko_core::config::schema::RokoConfig>,
    },
}

fn resolve_agent_dispatch(
    config: &RunConfig,
    requested_model: &str,
) -> Result<ResolvedAgentDispatch, String> {
    let Some(roko_config) = &config.roko_config else {
        return Ok(ResolvedAgentDispatch::Cli {
            model: requested_model.to_string(),
            cli_provider: None,
        });
    };

    let resolver = ProviderDispatchResolver::new(roko_config.clone());
    let spec = resolver.resolve(requested_model);
    match spec.runtime {
        ProviderRuntime::Cli(provider) => Ok(ResolvedAgentDispatch::Cli {
            model: spec.model_slug,
            cli_provider: Some(provider),
        }),
        ProviderRuntime::AgentResultBridge { .. } => Ok(ResolvedAgentDispatch::Bridge {
            model: spec.model_slug,
            provider_id: spec.provider_id,
            roko_config: roko_config.clone(),
        }),
        ProviderRuntime::Unsupported(unsupported) => Err(format!(
            "model `{requested_model}` resolved to unsupported provider `{}`: {}",
            spec.provider_id, unsupported.detail
        )),
    }
}

fn spawn_agent_result_bridge(
    roko_config: Arc<roko_core::config::schema::RokoConfig>,
    request: AgentDispatchRequest,
    agent_tx: mpsc::Sender<AgentEvent>,
) {
    tokio::spawn(async move {
        let dispatcher = AgentDispatcherV2::new(roko_config);
        match dispatcher.run_agent_result_bridge(request).await {
            Ok(dispatch) => {
                for event in dispatch.events {
                    if agent_tx
                        .send(agent_event_from_dispatch(event))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Err(err) => {
                let _ = agent_tx
                    .send(AgentEvent::Error {
                        message: err.to_string(),
                    })
                    .await;
                let _ = agent_tx
                    .send(AgentEvent::Exited { exit_code: Some(1) })
                    .await;
            }
        }
    });
}

fn agent_event_from_dispatch(event: DispatchEvent) -> AgentEvent {
    match event {
        DispatchEvent::Started {
            agent_id,
            provider,
            model,
            pid,
        } => AgentEvent::Started {
            agent_id,
            provider,
            model,
            pid,
        },
        DispatchEvent::MessageDelta { text } => AgentEvent::MessageDelta { text },
        DispatchEvent::TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        } => AgentEvent::TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        },
        DispatchEvent::TurnCompleted {
            total_cost_usd,
            is_error,
        } => AgentEvent::TurnCompleted {
            session_id: None,
            total_cost_usd,
            num_turns: Some(1),
            is_error,
        },
        DispatchEvent::Error { message } => AgentEvent::Error { message },
        DispatchEvent::Exited { exit_code } => AgentEvent::Exited { exit_code },
    }
}

// ─── Snapshot Helper ────────────────────────────────────────────────────

/// Save executor snapshot and track consecutive failures.
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
    match persist::save_executor_snapshot(paths, &snapshot) {
        Ok(()) => state.snapshot_succeeded(),
        Err(e) => {
            error!(err = %e, "failed to save executor snapshot");
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

            let prompt = agent_stream::build_task_prompt(task_def, plan_id, &ctx.config.workdir);
            let system_prompt = agent_stream::build_minimal_system_prompt(task_def, plan_id);

            let requested_model = task_def
                .model_hint
                .as_deref()
                .unwrap_or(&ctx.config.model)
                .to_string();
            let dispatch = match resolve_agent_dispatch(ctx.config, &requested_model) {
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

            let role = task_def.role.as_deref().unwrap_or("implementer");
            let agent_id = format!("{plan_id}/{task_id}");
            let attempt_ref = TaskAttemptRef::new(plan_id.clone(), task_id.clone(), attempt_num);
            let run_id = ctx.state.run_id().to_string();
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                RunnerEvent::task_attempt_started(&run_id, attempt_ref.clone(), &task_def.title),
            );
            emit_runner_event(
                ctx.paths,
                ctx.state,
                ctx.tui,
                RunnerEvent::agent_dispatch_started(
                    &run_id,
                    attempt_ref.clone(),
                    &agent_id,
                    role,
                    &requested_model,
                ),
            );

            // Prepend gate feedback if this is a retry.
            let final_prompt = if let Some(feedback) =
                agent_stream::format_gate_feedback_for_prompt(&previous_gate_output, 0)
            {
                format!("{feedback}\n\n---\n\n{prompt}")
            } else {
                prompt
            };

            match dispatch {
                ResolvedAgentDispatch::Cli {
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

                    match agent_stream::spawn_agent(&spawn_config, ctx.agent_tx.clone()).await {
                        Ok(handle) => {
                            ctx.state.agent_active = true;
                            ctx.state.agent_pid = Some(handle.pid);
                            emit_runner_event(
                                ctx.paths,
                                ctx.state,
                                ctx.tui,
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
                        }
                        Err(e) => {
                            error!(err = %e, "failed to spawn agent");
                            let message = format!("agent spawn failed: {e}");
                            emit_runner_event(
                                ctx.paths,
                                ctx.state,
                                ctx.tui,
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
                ResolvedAgentDispatch::Bridge {
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
                }
            }
        }

        ExecutorAction::RunGate { plan_id, rung } => {
            // Honor gates config: skip clippy rung (1) if disabled, skip test rung (2) if skip_tests.
            if *rung == 1 && !ctx.config.clippy_enabled {
                info!(plan_id = %plan_id, rung = rung, "skipping clippy gate (disabled in config)");
                let _ = ctx
                    .executor
                    .apply_event(plan_id, &ExecutorEvent::GatePassed);
                return;
            }
            if *rung == 2 && ctx.config.skip_tests {
                info!(plan_id = %plan_id, rung = rung, "skipping test gate (skip_tests in config)");
                let _ = ctx
                    .executor
                    .apply_event(plan_id, &ExecutorEvent::GatePassed);
                return;
            }

            info!(plan_id = %plan_id, rung = rung, "dispatching gate");
            let task_id = ctx.state.current_task.clone();
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
            ctx.merge_queue.enqueue(MergeRequest::new(
                plan_id.clone(),
                format!("roko/{plan_id}"),
                files_changed,
                0,
            ));
            let Some(request) = ctx.merge_queue.reserve_next_mergeable() else {
                info!(
                    plan_id = %plan_id,
                    blocked_conflicts = ?ctx.merge_queue.blocked_conflicts(),
                    "merge queued but currently blocked by file locks"
                );
                save_snapshot(ctx.executor, ctx.paths, ctx.state, ctx.merge_queue);
                return;
            };

            info!(
                plan_id = %request.plan_id,
                branch = %request.branch_name,
                files = request.files_changed.len(),
                "reserved merge queue request"
            );
            if ctx
                .executor
                .apply_event(&request.plan_id, &ExecutorEvent::MergeSucceeded)
                .is_ok()
            {
                ctx.merge_queue.mark_complete(&request.plan_id);
                ctx.tui.plan_completed(&request.plan_id, true);
                let run_id = ctx.state.run_id().to_string();
                emit_runner_event(
                    ctx.paths,
                    ctx.state,
                    ctx.tui,
                    RunnerEvent::plan_completed(
                        &run_id,
                        &request.plan_id,
                        PlanOutcome::Succeeded,
                        None,
                    ),
                );
                save_snapshot(ctx.executor, ctx.paths, ctx.state, ctx.merge_queue);
            } else {
                ctx.merge_queue
                    .mark_failed(&request.plan_id, "executor rejected merge transition");
                save_snapshot(ctx.executor, ctx.paths, ctx.state, ctx.merge_queue);
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
