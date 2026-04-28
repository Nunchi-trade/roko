//! WorkflowEngine -- top-level workflow execution facade.
//!
//! Ties together `PipelineStateV2` (decisions) and `EffectDriver` (effects)
//! into a run loop. This is the shared entry point for CLI, ACP, and HTTP.
//!
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use roko_core::RuntimeEvent;
use roko_core::foundation::{EventConsumer, FeedbackEvent, ShellGateCommand};
use roko_core::runtime_event::RuntimeEventEnvelope;
use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::effect_driver::{EffectDriver, EffectServices, Result};
use crate::event_bus::emit_runtime_event;
pub use crate::pipeline_state::WorkflowOutcome;
use crate::pipeline_state::{
    Phase, PipelineInput, PipelineOutput, PipelineStateV2, WorkflowConfig,
};

/// Configuration for a workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowRunConfig {
    /// User prompt.
    pub prompt: String,
    /// Working directory.
    pub workdir: PathBuf,
    /// Workflow configuration (express/standard/full).
    pub workflow: WorkflowConfig,
    /// Which gates to run.
    pub enabled_gates: Vec<String>,
    /// Shell command configs for shell/custom:shell gate entries.
    pub shell_gates: Vec<ShellGateCommand>,
    /// Commit message prefix.
    pub commit_prefix: Option<String>,
}

/// Result of a workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    /// Workflow run id.
    pub run_id: String,
    /// Final workflow outcome.
    pub outcome: WorkflowOutcome,
    /// Number of implementation iterations used.
    pub iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateOutcome {
    pub name: String,
    pub passed: bool,
    pub output: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunReport {
    pub run_id: String,
    pub success: bool,
    pub model: String,
    pub provider: Option<String>,
    pub prompt_summary: String,
    pub output: String,
    pub agent_turns: u32,
    pub token_usage: u64,
    pub cost: Option<f64>,
    pub duration_secs: f64,
    pub gates: Vec<GateOutcome>,
    pub events: Vec<RuntimeEventEnvelope>,
    pub checkpoint_path: Option<String>,
}

/// The top-level workflow execution engine.
///
/// Usage:
/// ```ignore
/// let engine = WorkflowEngine::new(services);
/// let result = engine.run(config).await?;
/// ```
pub struct WorkflowEngine {
    services: EffectServices,
    /// Optional event consumers to notify.
    consumers: Vec<Arc<dyn EventConsumer>>,
}

impl WorkflowEngine {
    /// Create a new `WorkflowEngine` with the given services.
    pub fn new(services: EffectServices) -> Self {
        Self {
            services,
            consumers: Vec::new(),
        }
    }

    /// Add an event consumer that will be notified of workflow lifecycle events.
    pub fn add_consumer(&mut self, consumer: Arc<dyn EventConsumer>) {
        self.consumers.push(consumer);
    }

    /// Execute a workflow run.
    ///
    /// This is the main entry point. It:
    /// 1. Creates a `PipelineStateV2` from the config.
    /// 2. Creates an `EffectDriver` from the services.
    /// 3. Runs the state machine loop: step -> execute -> feed back -> repeat.
    /// 4. Returns the outcome.
    pub async fn run(&self, config: WorkflowRunConfig) -> Result<WorkflowRunReport> {
        self.run_with_cancel(config, CancelToken::new()).await
    }

    /// Execute a workflow run with cooperative cancellation.
    ///
    /// The token is checked at the top of each iteration of the run loop. If the
    /// token is cancelled, the pipeline transitions to `Cancelled` state and the
    /// method returns with `WorkflowOutcome::Cancelled`. Any in-flight effect
    /// (agent call, gate run) is awaited to completion before the cancellation
    /// check takes effect -- this is cooperative, not preemptive.
    ///
    /// If `token` is already cancelled before the call, the workflow starts and
    /// then cancels at the first iteration check.
    #[allow(clippy::too_many_lines)]
    pub async fn run_with_cancel(
        &self,
        config: WorkflowRunConfig,
        token: CancelToken,
    ) -> Result<WorkflowRunReport> {
        let run_id = generate_run_id();
        let started_at = Instant::now();
        let event_start_seq = crate::event_bus::runtime_event_bus::<RuntimeEvent>().total_emitted();

        let mut pipeline = PipelineStateV2::new(config.workflow.clone(), config.prompt.clone());

        let driver = EffectDriver::new(
            EffectServices {
                model: self.services.model.clone(),
                model_caller: Arc::clone(&self.services.model_caller),
                prompt_assembler: Arc::clone(&self.services.prompt_assembler),
                feedback_sink: Arc::clone(&self.services.feedback_sink),
                gate_runner: Arc::clone(&self.services.gate_runner),
                affect_policy: self.services.affect_policy.clone(),
            },
            run_id.clone(),
            config.workdir.clone(),
        );

        self.emit(RuntimeEvent::WorkflowStarted {
            run_id: run_id.clone(),
            template: template_name(&config.workflow).to_string(),
            prompt: config.prompt.clone(),
        });

        let mut output = pipeline.step(PipelineInput::Start);

        loop {
            if token.is_cancelled() {
                let cancel_output = pipeline.step(PipelineInput::UserCancel);
                if let PipelineOutput::Done { outcome } = cancel_output {
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: runtime_workflow_outcome(&outcome),
                    });
                    self.record_workflow_feedback(&run_id, &outcome, &driver, started_at)
                        .await?;
                    self.persist_affect_policy().await;
                    return Ok(self.build_run_report(
                        &config,
                        &run_id,
                        &outcome,
                        started_at,
                        event_start_seq,
                    ));
                }
            }

            let old_phase = pipeline.phase.label();

            let input = match &output {
                PipelineOutput::SpawnStrategist { prompt } => {
                    strategy_input(driver.spawn_agent("strategist", prompt, None).await)
                }
                PipelineOutput::SpawnImplementer { prompt, context } => {
                    driver
                        .spawn_agent("implementer", prompt, context.as_deref())
                        .await
                }
                PipelineOutput::SpawnAutoFixer { error_output } => {
                    driver
                        .spawn_agent("autofix", "Fix the following errors", Some(error_output))
                        .await
                }
                PipelineOutput::SpawnReviewer { diff_context } => reviewer_input({
                    driver
                        .spawn_agent("reviewer", "Review the changes", diff_context.as_deref())
                        .await
                }),
                PipelineOutput::RunGates => {
                    self.emit(RuntimeEvent::GateStarted {
                        run_id: run_id.clone(),
                        gate_name: "pipeline".to_string(),
                        rung: 0,
                    });
                    driver
                        .run_gates(&config.enabled_gates, &config.shell_gates)
                        .await
                }
                PipelineOutput::Commit => {
                    let message = commit_message(&config);
                    driver.commit(&message).await
                }
                PipelineOutput::Done { outcome } => {
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: runtime_workflow_outcome(outcome),
                    });

                    self.record_workflow_feedback(&run_id, outcome, &driver, started_at)
                        .await?;
                    self.persist_affect_policy().await;
                    return Ok(self.build_run_report(
                        &config,
                        &run_id,
                        outcome,
                        started_at,
                        event_start_seq,
                    ));
                }
                PipelineOutput::Halt { reason } => {
                    let outcome = WorkflowOutcome::Halted {
                        reason: reason.clone(),
                    };
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: runtime_workflow_outcome(&outcome),
                    });

                    self.record_workflow_feedback(&run_id, &outcome, &driver, started_at)
                        .await?;
                    self.persist_affect_policy().await;
                    return Ok(self.build_run_report(
                        &config,
                        &run_id,
                        &outcome,
                        started_at,
                        event_start_seq,
                    ));
                }
            };

            output = pipeline.step(input);
            let new_phase = pipeline.phase.label();

            if old_phase != new_phase {
                self.emit_phase_transition(&run_id, old_phase, new_phase);
            }
        }
    }

    /// Resume a workflow run from a checkpoint.
    ///
    /// Deserializes the pipeline state from `checkpoint` JSON (produced by
    /// `PipelineStateV2::checkpoint()`) and continues the run loop from the
    /// saved phase. If the checkpoint is in a terminal phase (`Complete`,
    /// `Halted`, `Cancelled`), returns immediately with the terminal outcome.
    ///
    /// The `config` provides the working directory and enabled gates for the
    /// resumed run. The workflow config (template, max iterations) comes from
    /// the checkpoint itself, not from `config` -- this ensures the resumed
    /// run uses the same settings as the original.
    ///
    /// Returns an error if the checkpoint JSON is malformed.
    #[allow(clippy::too_many_lines)]
    pub async fn resume(
        &self,
        config: WorkflowRunConfig,
        checkpoint: &str,
    ) -> Result<WorkflowResult> {
        let pipeline = PipelineStateV2::from_checkpoint(checkpoint).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid checkpoint: {err}"),
            )
        })?;

        if pipeline.phase.is_terminal() {
            let outcome = match &pipeline.phase {
                Phase::Complete => WorkflowOutcome::Success {
                    commit_hash: pipeline.commit_hash.clone(),
                },
                Phase::Halted { reason } => WorkflowOutcome::Halted {
                    reason: reason.clone(),
                },
                Phase::Cancelled => WorkflowOutcome::Cancelled,
                _ => unreachable!(),
            };

            return Ok(WorkflowResult {
                run_id: generate_run_id(),
                outcome,
                iterations: pipeline.iteration,
            });
        }

        let run_id = generate_run_id();
        let started_at = Instant::now();

        let driver = EffectDriver::new(
            EffectServices {
                model: self.services.model.clone(),
                model_caller: Arc::clone(&self.services.model_caller),
                prompt_assembler: Arc::clone(&self.services.prompt_assembler),
                feedback_sink: Arc::clone(&self.services.feedback_sink),
                gate_runner: Arc::clone(&self.services.gate_runner),
                affect_policy: self.services.affect_policy.clone(),
            },
            run_id.clone(),
            config.workdir.clone(),
        );

        self.emit(RuntimeEvent::WorkflowStarted {
            run_id: run_id.clone(),
            template: "resumed".to_string(),
            prompt: pipeline.original_prompt.clone(),
        });

        let mut pipeline = pipeline;
        self.emit_phase_transition(&run_id, "checkpoint", pipeline.phase.label());

        let mut output = resumed_output(&mut pipeline);

        loop {
            let old_phase = pipeline.phase.label();

            let input = match &output {
                PipelineOutput::SpawnStrategist { prompt } => {
                    strategy_input(driver.spawn_agent("strategist", prompt, None).await)
                }
                PipelineOutput::SpawnImplementer { prompt, context } => {
                    driver
                        .spawn_agent("implementer", prompt, context.as_deref())
                        .await
                }
                PipelineOutput::SpawnAutoFixer { error_output } => {
                    driver
                        .spawn_agent("autofix", "Fix the following errors", Some(error_output))
                        .await
                }
                PipelineOutput::SpawnReviewer { diff_context } => reviewer_input({
                    driver
                        .spawn_agent("reviewer", "Review the changes", diff_context.as_deref())
                        .await
                }),
                PipelineOutput::RunGates => {
                    self.emit(RuntimeEvent::GateStarted {
                        run_id: run_id.clone(),
                        gate_name: "pipeline".to_string(),
                        rung: 0,
                    });
                    driver
                        .run_gates(&config.enabled_gates, &config.shell_gates)
                        .await
                }
                PipelineOutput::Commit => {
                    let message = commit_message_for(
                        &pipeline.original_prompt,
                        config.commit_prefix.as_deref(),
                    );
                    driver.commit(&message).await
                }
                PipelineOutput::Done { outcome } => {
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: runtime_workflow_outcome(outcome),
                    });

                    self.record_workflow_feedback(&run_id, outcome, &driver, started_at)
                        .await?;
                    self.persist_affect_policy().await;
                    return Ok(WorkflowResult {
                        run_id,
                        outcome: outcome.clone(),
                        iterations: pipeline.iteration,
                    });
                }
                PipelineOutput::Halt { reason } => {
                    let outcome = WorkflowOutcome::Halted {
                        reason: reason.clone(),
                    };
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: runtime_workflow_outcome(&outcome),
                    });

                    self.record_workflow_feedback(&run_id, &outcome, &driver, started_at)
                        .await?;
                    self.persist_affect_policy().await;
                    return Ok(WorkflowResult {
                        run_id,
                        outcome,
                        iterations: pipeline.iteration,
                    });
                }
            };

            output = pipeline.step(input);
            let new_phase = pipeline.phase.label();

            if old_phase != new_phase {
                self.emit_phase_transition(&run_id, old_phase, new_phase);
            }
        }
    }

    fn emit(&self, event: RuntimeEvent) -> u64 {
        for consumer in &self.consumers {
            consumer.consume(&event);
        }

        emit_runtime_event(event)
    }

    fn emit_phase_transition(&self, run_id: &str, from: &str, to: &str) {
        let event = RuntimeEvent::PhaseTransition {
            run_id: run_id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
        };

        let seq = self.emit(event.clone());
        let envelope = RuntimeEventEnvelope::new(run_id, seq, "workflow_engine", event);
        emit_runtime_event(envelope);
    }

    fn build_run_report(
        &self,
        config: &WorkflowRunConfig,
        run_id: &str,
        outcome: &WorkflowOutcome,
        started_at: Instant,
        event_start_seq: u64,
    ) -> WorkflowRunReport {
        let events = collect_run_events(run_id, event_start_seq);
        report_from_events(
            run_id,
            matches!(outcome, WorkflowOutcome::Success { .. }),
            &self.services.model,
            &config.prompt,
            started_at.elapsed().as_secs_f64(),
            events,
        )
    }

    async fn persist_affect_policy(&self) {
        if let Some(ref affect) = self.services.affect_policy {
            let policy = affect.lock().await;
            let _ = policy.persist().await;
            drop(policy);
        }
    }

    async fn record_workflow_feedback(
        &self,
        run_id: &str,
        outcome: &WorkflowOutcome,
        driver: &EffectDriver,
        started_at: Instant,
    ) -> Result<()> {
        let totals = driver.workflow_feedback_totals().await;
        let success = matches!(outcome, WorkflowOutcome::Success { .. });
        let event_type = if success {
            "workflow_completed"
        } else {
            "workflow_failed"
        };

        self.services
            .feedback_sink
            .record(FeedbackEvent::WorkflowComplete {
                event_type: event_type.to_string(),
                run_id: run_id.to_string(),
                model: totals.primary_model,
                success,
                outcome: workflow_feedback_outcome(outcome).to_string(),
                total_cost_usd: totals.total_cost_usd,
                total_tokens: totals.total_tokens,
                duration_ms: duration_millis(started_at),
            })
            .await?;
        self.services.feedback_sink.flush().await?;
        Ok(())
    }
}

fn collect_run_events(run_id: &str, event_start_seq: u64) -> Vec<RuntimeEventEnvelope> {
    crate::event_bus::runtime_event_bus::<RuntimeEvent>()
        .replay_from(event_start_seq)
        .into_iter()
        .filter(|envelope| envelope.payload.run_id() == run_id)
        .map(|envelope| RuntimeEventEnvelope {
            run_id: run_id.to_string(),
            seq: envelope.seq,
            ts: event_timestamp(envelope.ts_millis),
            schema_version: 1,
            source: event_source(&envelope.payload).to_string(),
            payload: envelope.payload,
        })
        .collect()
}

fn report_from_events(
    run_id: &str,
    success: bool,
    default_model: &str,
    prompt: &str,
    duration_secs: f64,
    events: Vec<RuntimeEventEnvelope>,
) -> WorkflowRunReport {
    let mut model = non_empty(default_model).map(ToOwned::to_owned);
    let mut implementer_model = None;
    let mut output = None;
    let mut agent_turns = 0_u32;
    let mut token_usage = 0_u64;
    let mut cost_total = 0.0_f64;
    let mut saw_cost = false;
    let mut gates = Vec::new();
    let mut checkpoint_path = None;

    for envelope in &events {
        match &envelope.payload {
            RuntimeEvent::AgentSpawned {
                role,
                model: event_model,
                ..
            } => {
                if let Some(event_model) = non_empty(event_model) {
                    if role == "implementer" {
                        implementer_model = Some(event_model.to_string());
                    }
                    model = Some(event_model.to_string());
                }
            }
            RuntimeEvent::AgentCompleted {
                output: event_output,
                tokens_used,
                cost_usd,
                ..
            } => {
                agent_turns = agent_turns.saturating_add(1);
                output = Some(event_output.clone());
                token_usage = token_usage.saturating_add(*tokens_used);
                cost_total += *cost_usd;
                saw_cost = true;
            }
            RuntimeEvent::AgentFailed { error, .. } => {
                agent_turns = agent_turns.saturating_add(1);
                if output.is_none() {
                    output = Some(error.clone());
                }
            }
            RuntimeEvent::GatePassed {
                gate_name,
                duration_ms,
                ..
            } => gates.push(GateOutcome {
                name: gate_name.clone(),
                passed: true,
                output: None,
                duration_ms: *duration_ms,
            }),
            RuntimeEvent::GateFailed {
                gate_name,
                output: gate_output,
                duration_ms,
                ..
            } => gates.push(GateOutcome {
                name: gate_name.clone(),
                passed: false,
                output: Some(gate_output.clone()),
                duration_ms: *duration_ms,
            }),
            RuntimeEvent::StateCheckpointed { path, .. } => {
                checkpoint_path = Some(path.clone());
            }
            _ => {}
        }
    }

    WorkflowRunReport {
        run_id: run_id.to_string(),
        success,
        model: implementer_model
            .or(model)
            .unwrap_or_else(|| "unconfigured".to_string()),
        provider: None,
        prompt_summary: summarize_text(prompt, 120),
        output: output.unwrap_or_else(|| {
            if success {
                "success".to_string()
            } else {
                "workflow did not produce agent output".to_string()
            }
        }),
        agent_turns,
        token_usage,
        cost: saw_cost.then_some(cost_total),
        duration_secs,
        gates,
        events,
        checkpoint_path,
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn summarize_text(text: &str, max_chars: usize) -> String {
    text.char_indices()
        .nth(max_chars)
        .map_or_else(|| text.to_string(), |(idx, _)| text[..idx].to_string())
}

fn event_timestamp(ts_millis: u64) -> DateTime<Utc> {
    let ts_millis = i64::try_from(ts_millis).unwrap_or(i64::MAX);
    DateTime::<Utc>::from_timestamp_millis(ts_millis).unwrap_or_else(Utc::now)
}

fn event_source(event: &RuntimeEvent) -> &'static str {
    match event {
        RuntimeEvent::WorkflowStarted { .. }
        | RuntimeEvent::PhaseTransition { .. }
        | RuntimeEvent::WorkflowCompleted { .. }
        | RuntimeEvent::GateStarted { .. } => "workflow_engine",
        RuntimeEvent::AgentSpawned { .. }
        | RuntimeEvent::AgentOutput { .. }
        | RuntimeEvent::AgentCompleted { .. }
        | RuntimeEvent::AgentFailed { .. }
        | RuntimeEvent::GatePassed { .. }
        | RuntimeEvent::GateFailed { .. }
        | RuntimeEvent::FeedbackRecorded { .. }
        | RuntimeEvent::StateCheckpointed { .. } => "effect_driver",
    }
}

fn runtime_workflow_outcome(
    outcome: &WorkflowOutcome,
) -> roko_core::runtime_event::WorkflowOutcome {
    // TODO(converge): Remove this adapter once PipelineStateV2 uses
    // roko_core::runtime_event::WorkflowOutcome directly.
    match outcome {
        WorkflowOutcome::Success { commit_hash } => {
            roko_core::runtime_event::WorkflowOutcome::Success {
                commit_hash: commit_hash.clone(),
            }
        }
        WorkflowOutcome::Halted { reason } => roko_core::runtime_event::WorkflowOutcome::Halted {
            reason: reason.clone(),
        },
        WorkflowOutcome::Cancelled => roko_core::runtime_event::WorkflowOutcome::Cancelled,
    }
}

fn strategy_input(input: PipelineInput) -> PipelineInput {
    match input {
        PipelineInput::AgentCompleted { output, .. } => {
            PipelineInput::StrategyComplete { brief: output }
        }
        input => input,
    }
}

fn reviewer_input(input: PipelineInput) -> PipelineInput {
    match input {
        PipelineInput::AgentCompleted { output, .. } => review_output_input(output),
        input => input,
    }
}

fn review_output_input(output: String) -> PipelineInput {
    if review_requests_revision(&output) {
        PipelineInput::ReviewRejected { reason: output }
    } else if review_approves(&output) {
        PipelineInput::ReviewApproved { summary: output }
    } else {
        PipelineInput::ReviewUnclear { summary: output }
    }
}

fn review_requests_revision(output: &str) -> bool {
    if structured_review_decision(output).is_some_and(|approved| !approved) {
        return true;
    }

    let normalized = output.to_ascii_lowercase();
    contains_any(
        &normalized,
        &[
            "changes requested",
            "request changes",
            "requested changes",
            "needs changes",
            "needs revision",
            "needs revisions",
            "requires revision",
            "requires revisions",
            "please revise",
            "must fix",
            "required changes",
            "blocking issue",
            "blocking issues",
            "not approved",
            "do not approve",
            "cannot approve",
            "rejected",
        ],
    )
}

fn review_approves(output: &str) -> bool {
    if structured_review_decision(output).is_some_and(|approved| approved) {
        return true;
    }

    let normalized = output.to_ascii_lowercase();
    contains_any(
        &normalized,
        &[
            "approved",
            "approve",
            "lgtm",
            "looks good to me",
            "no issues found",
            "ready to merge",
            "ship it",
        ],
    )
}

fn structured_review_decision(output: &str) -> Option<bool> {
    let value = parse_structured_review(output)?;

    if let Some(approved) = value.get("approved").and_then(serde_json::Value::as_bool) {
        return Some(approved);
    }

    for key in ["decision", "verdict", "status", "review", "outcome"] {
        let Some(decision) = value.get(key).and_then(serde_json::Value::as_str) else {
            continue;
        };
        if decision_requests_revision(decision) {
            return Some(false);
        }
        if decision_approves(decision) {
            return Some(true);
        }
    }

    None
}

fn parse_structured_review(output: &str) -> Option<serde_json::Value> {
    let trimmed = output.trim();
    serde_json::from_str(trimmed)
        .ok()
        .or_else(|| parse_fenced_json_review(trimmed))
}

fn parse_fenced_json_review(output: &str) -> Option<serde_json::Value> {
    let mut lines = output.lines();
    let first = lines.next()?.trim();
    if !first.starts_with("```") {
        return None;
    }

    let mut body = Vec::new();
    for line in lines {
        if line.trim_start().starts_with("```") {
            break;
        }
        body.push(line);
    }

    serde_json::from_str(&body.join("\n")).ok()
}

fn decision_requests_revision(decision: &str) -> bool {
    let normalized = decision.to_ascii_lowercase();
    contains_any(
        &normalized,
        &[
            "changes_requested",
            "request_changes",
            "changes requested",
            "revise",
            "revision",
            "reject",
            "rejected",
            "not approved",
            "fail",
            "failed",
        ],
    )
}

fn decision_approves(decision: &str) -> bool {
    let normalized = decision.to_ascii_lowercase();
    contains_any(
        &normalized,
        &[
            "approved", "approve", "accepted", "pass", "passed", "lgtm", "ready",
        ],
    )
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn resumed_output(pipeline: &mut PipelineStateV2) -> PipelineOutput {
    match &pipeline.phase {
        Phase::Pending => pipeline.step(PipelineInput::Start),
        Phase::Strategizing => PipelineOutput::SpawnStrategist {
            prompt: pipeline.original_prompt.clone(),
        },
        Phase::Implementing => PipelineOutput::SpawnImplementer {
            prompt: pipeline.original_prompt.clone(),
            context: resumed_implementer_context(pipeline),
        },
        Phase::Gating => PipelineOutput::RunGates,
        Phase::AutoFixing => PipelineOutput::SpawnAutoFixer {
            error_output: pipeline.last_gate_failure.clone().unwrap_or_default(),
        },
        Phase::Reviewing => PipelineOutput::SpawnReviewer { diff_context: None },
        Phase::Committing => PipelineOutput::Commit,
        Phase::Complete | Phase::Halted { .. } | Phase::Cancelled => PipelineOutput::Halt {
            reason: format!("cannot resume from phase {:?}", pipeline.phase),
        },
    }
}

fn resumed_implementer_context(pipeline: &PipelineStateV2) -> Option<String> {
    if !pipeline.review_findings.is_empty() {
        let feedback = pipeline.review_findings.join("\n- ");
        Some(format!("Review findings:\n- {feedback}"))
    } else if let Some(gate_failure) = &pipeline.last_gate_failure {
        Some(gate_failure.clone())
    } else {
        pipeline.strategist_brief.clone()
    }
}

fn commit_message(config: &WorkflowRunConfig) -> String {
    commit_message_for(&config.prompt, config.commit_prefix.as_deref())
}

fn commit_message_for(prompt: &str, prefix: Option<&str>) -> String {
    let prompt = truncate(prompt, 60);
    prefix.map_or_else(
        || format!("feat: {prompt}"),
        |prefix| format!("{prefix}: {prompt}"),
    )
}

fn template_name(config: &WorkflowConfig) -> &'static str {
    if config.has_strategy {
        "full"
    } else if config.has_review {
        "standard"
    } else {
        "express"
    }
}

fn workflow_feedback_outcome(outcome: &WorkflowOutcome) -> &'static str {
    match outcome {
        WorkflowOutcome::Success { .. } => "success",
        WorkflowOutcome::Halted { .. } => "failed",
        WorkflowOutcome::Cancelled => "cancelled",
    }
}

fn duration_millis(start: Instant) -> u64 {
    let millis = start.elapsed().as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

fn generate_run_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("run_{ts:x}")
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..floor_char_boundary(s, max)]
    }
}

/// Manual implementation of `str::floor_char_boundary` for MSRV < 1.91.
fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::process::Command;

    use parking_lot::Mutex;
    use roko_core::foundation::{
        FeedbackEvent, FeedbackSink, GateConfig, GateReport, GateRunner, ModelCallRequest,
        ModelCallResponse, ModelCaller, PromptAssembler, PromptSpec, TokenUsage,
    };

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("éclair", 1), "");
        assert_eq!(truncate("éclair", 2), "é");
    }

    #[test]
    fn commit_message_uses_prefix_when_present() {
        let config = WorkflowRunConfig {
            prompt: "short prompt".to_string(),
            workdir: PathBuf::from("."),
            workflow: WorkflowConfig::express(),
            enabled_gates: Vec::new(),
            shell_gates: Vec::new(),
            commit_prefix: Some("fix".to_string()),
        };

        assert_eq!(commit_message(&config), "fix: short prompt");
    }

    struct MockModelCaller;

    impl ModelCaller for MockModelCaller {
        fn call<'life0, 'async_trait>(
            &'life0 self,
            req: ModelCallRequest,
        ) -> Pin<Box<dyn Future<Output = roko_core::Result<ModelCallResponse>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            let content = if req.role.as_deref() == Some("reviewer") {
                "approved"
            } else {
                "done"
            };
            Box::pin(async {
                Ok(ModelCallResponse {
                    content: content.into(),
                    model: "mock".into(),
                    usage: TokenUsage::default(),
                    stop_reason: None,
                    request_id: None,
                })
            })
        }
    }

    struct MockPromptAssembler;

    impl PromptAssembler for MockPromptAssembler {
        fn assemble<'life0, 'async_trait>(
            &'life0 self,
            _spec: PromptSpec,
        ) -> Pin<Box<dyn Future<Output = roko_core::Result<String>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async { Ok("system prompt".to_string()) })
        }

        fn last_prompt_section_ids(&self) -> Vec<String> {
            vec!["role_identity".to_string()]
        }

        fn last_knowledge_ids(&self) -> Vec<String> {
            Vec::new()
        }
    }

    struct MockFeedbackSink;

    impl FeedbackSink for MockFeedbackSink {
        fn record<'life0, 'async_trait>(
            &'life0 self,
            _event: FeedbackEvent,
        ) -> Pin<Box<dyn Future<Output = roko_core::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async { Ok(()) })
        }

        fn flush<'life0, 'async_trait>(
            &'life0 self,
        ) -> Pin<Box<dyn Future<Output = roko_core::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async { Ok(()) })
        }
    }

    struct MockGateRunner;

    impl GateRunner for MockGateRunner {
        fn run_gates<'life0, 'async_trait>(
            &'life0 self,
            _config: GateConfig,
        ) -> Pin<Box<dyn Future<Output = roko_core::Result<GateReport>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async {
                Ok(GateReport {
                    verdicts: Vec::new(),
                })
            })
        }
    }

    struct RecordingConsumer {
        events: Arc<Mutex<Vec<RuntimeEvent>>>,
    }

    impl EventConsumer for RecordingConsumer {
        fn consume(&self, event: &RuntimeEvent) {
            self.events.lock().push(event.clone());
        }
    }

    #[tokio::test]
    async fn workflow_engine_express_completes_with_success() {
        let (config, _tempdir) = workflow_config();
        let engine = WorkflowEngine::new(mock_services());

        let result = engine.run(config).await;

        assert!(result.is_ok());
        let result = match result {
            Ok(result) => result,
            Err(err) => panic!("workflow should complete successfully: {err}"),
        };
        assert!(result.success);
        assert_eq!(result.agent_turns, 1);
        assert_eq!(result.model, "mock");
        assert!(result.run_id.starts_with("run_"));
    }

    #[tokio::test]
    async fn workflow_engine_express_emits_lifecycle_events() {
        let (config, _tempdir) = workflow_config();
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut engine = WorkflowEngine::new(mock_services());
        engine.add_consumer(Arc::new(RecordingConsumer {
            events: Arc::clone(&events),
        }));

        let result = engine.run(config).await;

        assert!(result.is_ok());
        let events = events.lock().clone();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    RuntimeEvent::WorkflowStarted { template, .. } if template == "express"
                ))
                .count(),
            1
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::PhaseTransition { .. }))
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    RuntimeEvent::WorkflowCompleted {
                        outcome: roko_core::runtime_event::WorkflowOutcome::Success { .. },
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn workflow_engine_standard_passes_through_review() {
        let (config, _tempdir) = standard_workflow_config();
        let engine = WorkflowEngine::new(mock_services());

        let result = engine.run(config).await;

        assert!(result.is_ok());
        let result = match result {
            Ok(result) => result,
            Err(err) => panic!("workflow should complete successfully: {err}"),
        };
        assert!(result.success);
        assert_eq!(result.agent_turns, 2);
    }

    #[tokio::test]
    async fn workflow_engine_standard_gate_pass_triggers_review_phase() {
        let (config, _tempdir) = standard_workflow_config();
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut engine = WorkflowEngine::new(mock_services());
        engine.add_consumer(Arc::new(RecordingConsumer {
            events: Arc::clone(&events),
        }));

        let result = engine.run(config).await;

        assert!(result.is_ok());
        let events = events.lock().clone();
        assert!(events.iter().any(|event| matches!(
            event,
            RuntimeEvent::PhaseTransition { from, to, .. }
                if from == "implementing" && to == "gating"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            RuntimeEvent::PhaseTransition { from, to, .. }
                if from == "gating" && to == "reviewing"
        )));
    }

    fn mock_services() -> EffectServices {
        EffectServices {
            model: "mock".to_string(),
            model_caller: Arc::new(MockModelCaller),
            prompt_assembler: Arc::new(MockPromptAssembler),
            feedback_sink: Arc::new(MockFeedbackSink),
            gate_runner: Arc::new(MockGateRunner),
            affect_policy: None,
        }
    }

    fn workflow_config() -> (WorkflowRunConfig, tempfile::TempDir) {
        let tempdir = isolated_git_workdir();
        let config = WorkflowRunConfig {
            prompt: "fix the bug".into(),
            workdir: tempdir.path().to_path_buf(),
            workflow: WorkflowConfig::express(),
            enabled_gates: Vec::new(),
            shell_gates: Vec::new(),
            commit_prefix: None,
        };
        (config, tempdir)
    }

    fn standard_workflow_config() -> (WorkflowRunConfig, tempfile::TempDir) {
        let tempdir = isolated_git_workdir();
        let config = WorkflowRunConfig {
            prompt: "fix the bug".into(),
            workdir: tempdir.path().to_path_buf(),
            workflow: WorkflowConfig::standard(),
            enabled_gates: vec!["compile".to_string()],
            shell_gates: Vec::new(),
            commit_prefix: None,
        };
        (config, tempdir)
    }

    fn isolated_git_workdir() -> tempfile::TempDir {
        let tempdir = match tempfile::tempdir() {
            Ok(tempdir) => tempdir,
            Err(err) => panic!("create temp dir: {err}"),
        };
        let workdir = tempdir.path();

        run_git(workdir, &["init"]);
        run_git(workdir, &["config", "user.email", "test@example.com"]);
        run_git(workdir, &["config", "user.name", "Roko Test"]);

        if let Err(err) = std::fs::write(workdir.join("change.txt"), "change\n") {
            panic!("write test change: {err}");
        }
        tempdir
    }

    fn run_git(workdir: &std::path::Path, args: &[&str]) {
        let output = match Command::new("git").args(args).current_dir(workdir).output() {
            Ok(output) => output,
            Err(err) => panic!("run git command: {err}"),
        };

        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
