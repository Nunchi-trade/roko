//! WorkflowEngine -- top-level workflow execution facade.
//!
//! Ties together `PipelineStateV2` (decisions) and `EffectDriver` (effects)
//! into a run loop. This is the shared entry point for CLI, ACP, and HTTP.
//!
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tracing::warn;

use chrono::{DateTime, Utc};
use roko_core::RuntimeEvent;
use roko_core::foundation::{EventConsumer, FeedbackEvent, ShellGateCommand};
use roko_core::runtime_event::RuntimeEventEnvelope;
use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::effect_driver::{EffectDriver, EffectServices, Result, WorkflowFeedbackTotals};
use crate::event_bus::emit_runtime_event;
pub use crate::pipeline_state::WorkflowOutcome;
use crate::pipeline_state::{
    Phase, PipelineInput, PipelineOutput, PipelineStateV2, WorkflowConfig,
};
use crate::run_ledger::{EffectErrorKind, RunLedger};

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

/// Per-gate result included in a workflow run report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateOutcome {
    /// Gate name.
    pub name: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Optional gate output or failure details.
    pub output: Option<String>,
    /// Gate runtime in milliseconds.
    pub duration_ms: u64,
}

/// Summary returned by `WorkflowEngine` after a workflow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunReport {
    /// Workflow run id.
    pub run_id: String,
    /// Whether the workflow completed successfully.
    pub success: bool,
    /// Primary model used by the workflow.
    pub model: String,
    /// Provider used for the primary model, when known.
    pub provider: Option<String>,
    /// Short summary of the prompt.
    pub prompt_summary: String,
    /// Final workflow output.
    pub output: String,
    /// Number of agent turns used.
    pub agent_turns: u32,
    /// Total tokens used.
    pub token_usage: u64,
    /// Total cost, when known.
    pub cost: Option<f64>,
    /// Total runtime in seconds.
    pub duration_secs: f64,
    /// Gate outcomes collected during the run.
    pub gates: Vec<GateOutcome>,
    /// Runtime events emitted during the run.
    pub events: Vec<RuntimeEventEnvelope>,
    /// Last checkpoint path, when one was written.
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
        let started_at_ms = now_millis();
        let event_start_seq = crate::event_bus::runtime_event_bus::<RuntimeEvent>().total_emitted();

        let mut pipeline = PipelineStateV2::new(config.workflow.clone(), config.prompt.clone());
        let mut ledger = RunLedger::new(
            run_id.clone(),
            config.prompt.clone(),
            config.workflow.clone(),
            started_at_ms,
        );

        let driver = EffectDriver::new(
            EffectServices {
                default_model: self.services.default_model.clone(),
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
                let cancel_phase = pipeline.phase.clone();
                ledger.record_cancellation_requested(cancel_phase, now_millis());
                let cancel_output = pipeline.step(PipelineInput::UserCancel);
                if let PipelineOutput::Done { outcome } = cancel_output {
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: runtime_workflow_outcome(&outcome),
                    });
                    if let Err(err) = self
                        .record_workflow_feedback(&run_id, &outcome, &driver, started_at)
                        .await
                    {
                        warn!(run_id = %run_id, error = %err, "failed to record workflow feedback; continuing");
                    }
                    self.persist_affect_policy().await;
                    return Ok(Self::build_run_report_from_ledger(
                        &ledger,
                        &outcome,
                        started_at,
                        event_start_seq,
                    ));
                }
            }

            let old_phase = pipeline.phase.clone();
            let old_phase_label = old_phase.label();

            let input = match &output {
                PipelineOutput::SpawnStrategist { prompt } => {
                    let before = driver.workflow_feedback_totals().await;
                    let raw_input = driver.spawn_agent("strategist", prompt, None).await;
                    let after = driver.workflow_feedback_totals().await;
                    record_agent_input(
                        &mut ledger,
                        "strategist",
                        &self.services.default_model,
                        &before,
                        &after,
                        &raw_input,
                    );
                    strategy_input(raw_input)
                }
                PipelineOutput::SpawnImplementer { prompt, context } => {
                    let before = driver.workflow_feedback_totals().await;
                    let input = driver
                        .spawn_agent("implementer", prompt, context.as_deref())
                        .await;
                    let after = driver.workflow_feedback_totals().await;
                    record_agent_input(
                        &mut ledger,
                        "implementer",
                        &self.services.default_model,
                        &before,
                        &after,
                        &input,
                    );
                    input
                }
                PipelineOutput::SpawnAutoFixer { error_output } => {
                    let before = driver.workflow_feedback_totals().await;
                    let input = driver
                        .spawn_agent("autofix", "Fix the following errors", Some(error_output))
                        .await;
                    let after = driver.workflow_feedback_totals().await;
                    record_agent_input(
                        &mut ledger,
                        "autofix",
                        &self.services.default_model,
                        &before,
                        &after,
                        &input,
                    );
                    input
                }
                PipelineOutput::SpawnReviewer { diff_context } => reviewer_input({
                    let before = driver.workflow_feedback_totals().await;
                    let raw_input = driver
                        .spawn_agent("reviewer", "Review the changes", diff_context.as_deref())
                        .await;
                    let after = driver.workflow_feedback_totals().await;
                    record_agent_input(
                        &mut ledger,
                        "reviewer",
                        &self.services.default_model,
                        &before,
                        &after,
                        &raw_input,
                    );
                    raw_input
                }),
                PipelineOutput::RunGates => {
                    self.emit(RuntimeEvent::GateStarted {
                        run_id: run_id.clone(),
                        gate_name: "pipeline".to_string(),
                        rung: 0,
                    });
                    let input = driver
                        .run_gates(&config.enabled_gates, &config.shell_gates)
                        .await;
                    record_gate_input(&mut ledger, &config.enabled_gates, &input);
                    input
                }
                PipelineOutput::Commit => {
                    let message = commit_message(&config);
                    let input = driver.commit(&message).await;
                    record_commit_input(&mut ledger, &input);
                    input
                }
                PipelineOutput::Done { outcome } => {
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: runtime_workflow_outcome(outcome),
                    });

                    if let Err(err) = self
                        .record_workflow_feedback(&run_id, outcome, &driver, started_at)
                        .await
                    {
                        warn!(run_id = %run_id, error = %err, "failed to record workflow feedback; continuing");
                    }
                    self.persist_affect_policy().await;
                    return Ok(Self::build_run_report_from_ledger(
                        &ledger,
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

                    if let Err(err) = self
                        .record_workflow_feedback(&run_id, &outcome, &driver, started_at)
                        .await
                    {
                        warn!(run_id = %run_id, error = %err, "failed to record workflow feedback; continuing");
                    }
                    self.persist_affect_policy().await;
                    return Ok(Self::build_run_report_from_ledger(
                        &ledger,
                        &outcome,
                        started_at,
                        event_start_seq,
                    ));
                }
            };

            output = pipeline.step(input);
            let new_phase = pipeline.phase.clone();
            let new_phase_label = new_phase.label();

            if old_phase_label != new_phase_label {
                ledger.record_phase_transition(old_phase, new_phase, now_millis());
                self.emit_phase_transition(&run_id, old_phase_label, new_phase_label);
            }
        }
    }

    /// Resume a workflow run from a checkpoint.
    ///
    /// Deserializes the pipeline state from `checkpoint` JSON (produced by
    /// `PipelineStateV2::checkpoint()`) and continues the run loop from the
    /// saved phase. If the checkpoint is in a terminal phase (`Complete`,
    /// `Halted`, `Cancelled`), returns immediately with a completed report.
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
    ) -> Result<WorkflowRunReport> {
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

            let run_id = generate_run_id();
            let started_at = Instant::now();
            let event_start_seq =
                crate::event_bus::runtime_event_bus::<RuntimeEvent>().total_emitted();

            warn!(
                run_id = %run_id,
                phase = ?pipeline.phase,
                "resume called on already-terminal checkpoint; returning immediately"
            );
            self.emit(RuntimeEvent::WorkflowCompleted {
                run_id: run_id.clone(),
                outcome: runtime_workflow_outcome(&outcome),
            });

            return Ok(self.build_run_report(
                &config,
                &run_id,
                &outcome,
                started_at,
                event_start_seq,
            ));
        }

        let run_id = generate_run_id();
        let started_at = Instant::now();
        let event_start_seq = crate::event_bus::runtime_event_bus::<RuntimeEvent>().total_emitted();

        let driver = EffectDriver::new(
            EffectServices {
                default_model: self.services.default_model.clone(),
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

                    if let Err(err) = self
                        .record_workflow_feedback(&run_id, outcome, &driver, started_at)
                        .await
                    {
                        warn!(run_id = %run_id, error = %err, "failed to record workflow feedback; continuing");
                    }
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

                    if let Err(err) = self
                        .record_workflow_feedback(&run_id, &outcome, &driver, started_at)
                        .await
                    {
                        warn!(run_id = %run_id, error = %err, "failed to record workflow feedback; continuing");
                    }
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

        self.emit(event);
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
            &self.services.default_model,
            &config.prompt,
            started_at.elapsed().as_secs_f64(),
            events,
        )
    }

    fn build_run_report_from_ledger(
        ledger: &RunLedger,
        outcome: &WorkflowOutcome,
        started_at: Instant,
        event_start_seq: u64,
    ) -> WorkflowRunReport {
        let events = collect_run_events(&ledger.run_id, event_start_seq);
        ledger.to_report_compat(
            matches!(outcome, WorkflowOutcome::Success { .. }),
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

// Compatibility helper for legacy report/event projections. `run_with_cancel`
// builds report truth from `RunLedger` and only uses collected events for the
// report's legacy `events` field.
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

// Compatibility-only builder retained for resume/checkpoint tests until resume
// is migrated to `RunLedger`.
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

fn record_agent_input(
    ledger: &mut RunLedger,
    role: &str,
    requested_model: &str,
    before: &WorkflowFeedbackTotals,
    after: &WorkflowFeedbackTotals,
    input: &PipelineInput,
) {
    match input {
        PipelineInput::AgentCompleted {
            output,
            files_changed,
        } => {
            let final_model = after
                .primary_model
                .as_deref()
                .filter(|model| !model.trim().is_empty())
                .unwrap_or(requested_model);
            ledger.record_agent_completed(
                role,
                output,
                *files_changed,
                requested_model,
                final_model,
                None,
                roko_core::foundation::TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: after.total_tokens.saturating_sub(before.total_tokens),
                    cost_usd: (after.total_cost_usd - before.total_cost_usd).max(0.0),
                },
            );
        }
        PipelineInput::AgentFailed { error } => {
            ledger.record_agent_failed(role, EffectErrorKind::Unknown, error);
        }
        _ => {}
    }
}

fn record_gate_input(_ledger: &mut RunLedger, _enabled_gates: &[String], _input: &PipelineInput) {
    // Gate verdict details are not exposed to WorkflowEngine yet; recording the
    // collapsed pipeline input here would invent missing duration/verdict data.
}

fn record_commit_input(ledger: &mut RunLedger, input: &PipelineInput) {
    if let Some(outcome) = crate::pipeline_state::CommitOutcome::from_pipeline_input(input) {
        ledger.record_commit(outcome);
    }
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

fn now_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    u64::try_from(millis).unwrap_or(u64::MAX)
}

fn generate_run_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
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
    use std::path::Path;
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

    struct RecordingModelCaller {
        roles: Arc<Mutex<Vec<String>>>,
    }

    impl ModelCaller for RecordingModelCaller {
        fn call<'life0, 'async_trait>(
            &'life0 self,
            req: ModelCallRequest,
        ) -> Pin<Box<dyn Future<Output = roko_core::Result<ModelCallResponse>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            let roles = Arc::clone(&self.roles);
            Box::pin(async move {
                let role = req.role.unwrap_or_else(|| "unknown".to_string());
                roles.lock().push(role.clone());
                let content = if role == "reviewer" {
                    "approved"
                } else {
                    "done"
                };

                Ok(ModelCallResponse {
                    content: content.into(),
                    model: "mock".into(),
                    usage: TokenUsage {
                        input_tokens: 10,
                        output_tokens: 20,
                        total_tokens: 30,
                        cost_usd: 0.01,
                    },
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
    async fn test_v2_run_end_to_end() {
        let tempdir = isolated_git_workdir();
        let workdir = tempdir.path().to_path_buf();
        let roko_dir = workdir.join(".roko");
        let event_log = roko_dir.join("runtime-events.jsonl");
        let feedback_log = roko_dir.join("learn").join("efficiency.jsonl");

        let services = EffectServices {
            default_model: "mock".to_string(),
            model_caller: Arc::new(RecordingModelCaller {
                roles: Arc::new(Mutex::new(Vec::new())),
            }),
            prompt_assembler: Arc::new(roko_compose::PromptAssemblyService::new()),
            feedback_sink: Arc::new(roko_learn::FeedbackService::from_roko_dir_with_episodes(
                &roko_dir,
            )),
            gate_runner: Arc::new(roko_gate::GateService::new()),
            affect_policy: None,
        };

        let mut engine = WorkflowEngine::new(services);
        engine.add_consumer(Arc::new(crate::jsonl_logger::JsonlLogger::new(
            event_log.clone(),
        )));

        let report = engine
            .run(WorkflowRunConfig {
                prompt: "write the smallest useful change".to_string(),
                workdir,
                workflow: WorkflowConfig::standard(),
                enabled_gates: vec!["shell".to_string()],
                shell_gates: vec![ShellGateCommand {
                    program: "true".to_string(),
                    args: Vec::new(),
                    timeout_ms: 30_000,
                }],
                commit_prefix: None,
            })
            .await
            .expect("v2 workflow should complete with mock provider");

        assert!(report.success);
        assert_eq!(report.model, "mock");
        assert!(!report.output.trim().is_empty());
        assert!(report.token_usage > 0);
        assert!(
            report
                .events
                .iter()
                .any(|event| { matches!(event.payload, RuntimeEvent::WorkflowStarted { .. }) })
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| { matches!(event.payload, RuntimeEvent::WorkflowCompleted { .. }) })
        );

        let event_lines =
            std::fs::read_to_string(&event_log).expect("runtime event JSONL should be written");
        let envelopes = event_lines
            .lines()
            .map(|line| {
                serde_json::from_str::<RuntimeEventEnvelope>(line)
                    .expect("runtime event log line should be typed JSON")
            })
            .collect::<Vec<_>>();
        assert!(
            envelopes
                .iter()
                .any(|event| { matches!(event.payload, RuntimeEvent::WorkflowStarted { .. }) })
        );
        assert!(
            envelopes
                .iter()
                .any(|event| { matches!(event.payload, RuntimeEvent::WorkflowCompleted { .. }) })
        );

        let feedback_lines =
            std::fs::read_to_string(&feedback_log).expect("feedback JSONL should be written");
        let feedback = feedback_lines
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("feedback JSON"))
            .collect::<Vec<_>>();
        assert!(feedback.iter().any(|event| {
            event.get("kind").and_then(serde_json::Value::as_str) == Some("model_call")
        }));
        assert!(feedback.iter().any(|event| {
            event.get("kind").and_then(serde_json::Value::as_str) == Some("gate_result")
        }));
        assert!(feedback.iter().any(|event| {
            event.get("kind").and_then(serde_json::Value::as_str) == Some("workflow_completed")
        }));

        let summary = crate::projection::RuntimeProjection::for_run(&event_log, &report.run_id)
            .expect("projection should read typed runtime event JSONL")
            .expect("projection should include completed run");
        assert!(summary.is_complete);
        assert!(
            summary
                .outcome
                .as_deref()
                .is_some_and(|outcome| outcome.starts_with("success"))
        );
        assert_eq!(
            summary.prompt.as_deref(),
            Some("write the smallest useful change")
        );
    }

    #[tokio::test]
    async fn test_checkpoint_resume_round_trip() {
        let (config, _tempdir) = workflow_config();
        let roles = Arc::new(Mutex::new(Vec::new()));
        let engine = WorkflowEngine::new(recording_services(Arc::clone(&roles)));
        let run1_id = unique_test_run_id("checkpoint");
        let driver = EffectDriver::new(
            recording_services(Arc::clone(&roles)),
            run1_id.clone(),
            config.workdir.clone(),
        );

        let run1_start_seq = crate::event_bus::runtime_event_bus::<RuntimeEvent>().total_emitted();
        let mut pipeline = PipelineStateV2::new(config.workflow.clone(), config.prompt.clone());

        engine.emit(RuntimeEvent::WorkflowStarted {
            run_id: run1_id.clone(),
            template: template_name(&config.workflow).to_string(),
            prompt: config.prompt.clone(),
        });

        let old_phase = pipeline.phase.label();
        let output = pipeline.step(PipelineInput::Start);
        engine.emit_phase_transition(&run1_id, old_phase, pipeline.phase.label());

        let PipelineOutput::SpawnImplementer { prompt, context } = output else {
            panic!("express workflow should enter implementation");
        };
        let input = driver
            .spawn_agent("implementer", &prompt, context.as_deref())
            .await;
        assert!(matches!(input, PipelineInput::AgentCompleted { .. }));

        let old_phase = pipeline.phase.label();
        let output = pipeline.step(input);
        engine.emit_phase_transition(&run1_id, old_phase, pipeline.phase.label());
        assert!(matches!(output, PipelineOutput::RunGates));
        assert_eq!(pipeline.phase, Phase::Gating);

        let checkpoint_path = config.workdir.join(".roko/checkpoint-resume-test.json");
        driver
            .save_checkpoint(&pipeline, &checkpoint_path)
            .await
            .expect("checkpoint should be persisted");
        assert!(checkpoint_path.is_file());

        let checkpoint = tokio::fs::read_to_string(&checkpoint_path)
            .await
            .expect("checkpoint should be readable");
        let restored = PipelineStateV2::from_checkpoint(&checkpoint)
            .expect("checkpoint should restore pipeline state");
        assert_eq!(restored.phase, Phase::Gating);

        let run1_events = collect_run_events(&run1_id, run1_start_seq);
        assert_checkpoint_run_sequence(&run1_events, &checkpoint_path);
        assert_eq!(roles.lock().as_slice(), ["implementer"]);

        let resume_start_seq =
            crate::event_bus::runtime_event_bus::<RuntimeEvent>().total_emitted();
        let resumed = engine
            .resume(config.clone(), &checkpoint)
            .await
            .expect("resume should complete from checkpoint");
        assert!(
            resumed.success,
            "resumed workflow should complete successfully"
        );
        assert!(
            resumed.run_id.starts_with("run_"),
            "resumed run_id should have run_ prefix"
        );

        let resumed_events = collect_run_events(&resumed.run_id, resume_start_seq);
        assert_resume_run_sequence(&resumed_events);
        assert_eq!(
            roles.lock().as_slice(),
            ["implementer"],
            "resume should not rerun the completed implementation phase"
        );

        assert_eq!(resumed.run_id, resumed.run_id);
        assert!(resumed.success);
        assert_eq!(resumed.model, "mock");
        assert!(!resumed.events.is_empty());

        let run1_log = config.workdir.join(".roko/run1-events.jsonl");
        let run2_log = config.workdir.join(".roko/run2-events.jsonl");
        let full_log = config.workdir.join(".roko/all-events.jsonl");
        write_event_log(&run1_log, &run1_events);
        write_event_log(&run2_log, &resumed_events);
        write_event_log(
            &full_log,
            &run1_events
                .iter()
                .chain(resumed_events.iter())
                .cloned()
                .collect::<Vec<_>>(),
        );

        let run1_summary = crate::projection::RuntimeProjection::for_run(&run1_log, &run1_id)
            .expect("run1 projection should load")
            .expect("run1 summary should exist");
        assert!(!run1_summary.is_complete);
        assert_eq!(run1_summary.current_phase.as_deref(), Some("gating"));
        assert_eq!(run1_summary.agents_completed, 1);
        assert_eq!(
            run1_summary.last_checkpoint.as_deref(),
            Some(checkpoint_path.to_string_lossy().as_ref())
        );

        let run2_summary =
            crate::projection::RuntimeProjection::for_run(&run2_log, &resumed.run_id)
                .expect("run2 projection should load")
                .expect("run2 summary should exist");
        assert!(run2_summary.is_complete);
        assert_eq!(run2_summary.current_phase.as_deref(), Some("complete"));
        assert!(
            run2_summary
                .phases_visited
                .iter()
                .any(|phase| phase == "gating")
        );
        assert!(
            !run2_summary
                .phases_visited
                .iter()
                .any(|phase| phase == "implementing"),
            "resume should continue from the recovered phase, not from the beginning"
        );

        let full_projection = crate::projection::RuntimeProjection::from_file(&full_log)
            .expect("combined projection should load");
        assert!(full_projection.contains_key(&run1_id));
        assert!(full_projection.contains_key(&resumed.run_id));
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

    #[test]
    fn workflow_report_uses_ledger_when_events_empty() {
        let mut ledger = RunLedger::new(
            "run-ledger-empty-events",
            "fix the bug",
            WorkflowConfig::express(),
            1_700_000_000_000,
        );
        ledger.record_agent_completed(
            "implementer",
            "done",
            1,
            "requested",
            "actual",
            None,
            TokenUsage {
                input_tokens: 7,
                output_tokens: 11,
                total_tokens: 18,
                cost_usd: 0.03,
            },
        );

        let report = ledger.to_report_compat(true, 0.25, Vec::new());

        assert!(report.success);
        assert_eq!(report.model, "actual");
        assert_eq!(report.provider, None);
        assert_eq!(report.output, "done");
        assert_eq!(report.agent_turns, 1);
        assert_eq!(report.token_usage, 18);
        assert_eq!(report.cost, Some(0.03));
        assert!(report.events.is_empty());
    }

    #[tokio::test]
    async fn workflow_report_preserves_compat_for_pre_cancelled_run() {
        let (config, _tempdir) = workflow_config();
        let engine = WorkflowEngine::new(mock_services());
        let token = CancelToken::new();
        token.cancel();

        let report = engine
            .run_with_cancel(config, token)
            .await
            .expect("cancelled workflow should return a report");

        assert!(!report.success);
        assert_eq!(report.model, "unconfigured");
        assert_eq!(report.output, "workflow did not produce agent output");
        assert_eq!(report.agent_turns, 0);
        assert_eq!(report.token_usage, 0);
        assert!(report.gates.is_empty());
        assert!(report.events.iter().any(|event| matches!(
            event.payload,
            RuntimeEvent::WorkflowCompleted {
                outcome: roko_core::runtime_event::WorkflowOutcome::Cancelled,
                ..
            }
        )));
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
            default_model: "mock".to_string(),
            model_caller: Arc::new(MockModelCaller),
            prompt_assembler: Arc::new(MockPromptAssembler),
            feedback_sink: Arc::new(MockFeedbackSink),
            gate_runner: Arc::new(MockGateRunner),
            affect_policy: None,
        }
    }

    fn recording_services(roles: Arc<Mutex<Vec<String>>>) -> EffectServices {
        EffectServices {
            default_model: "mock".to_string(),
            model_caller: Arc::new(RecordingModelCaller { roles }),
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

    fn assert_checkpoint_run_sequence(events: &[RuntimeEventEnvelope], checkpoint_path: &Path) {
        let started = event_position(events, "workflow_started", |event| {
            matches!(event, RuntimeEvent::WorkflowStarted { .. })
        });
        let implementing = event_position(
            events,
            "phase_transition_to_implementing",
            |event| matches!(event, RuntimeEvent::PhaseTransition { to, .. } if to == "implementing"),
        );
        let agent_completed = event_position(events, "agent_completed", |event| {
            matches!(event, RuntimeEvent::AgentCompleted { .. })
        });
        let checkpointed = event_position(events, "state_checkpointed", |event| {
            matches!(
                event,
                RuntimeEvent::StateCheckpointed { path, .. }
                    if !path.is_empty() && Path::new(path).is_file() && path == &checkpoint_path.to_string_lossy()
            )
        });

        assert!(
            started < implementing
                && implementing < agent_completed
                && agent_completed < checkpointed,
            "run1 should start, enter implementation, complete the agent turn, then checkpoint"
        );
        assert!(
            !events
                .iter()
                .any(|envelope| matches!(envelope.payload, RuntimeEvent::WorkflowCompleted { .. })),
            "interrupted run should stop before workflow completion"
        );
    }

    fn assert_resume_run_sequence(events: &[RuntimeEventEnvelope]) {
        let recovered_phase = event_position(events, "recovered_phase_transition", |event| {
            matches!(
                event,
                RuntimeEvent::PhaseTransition { from, to, .. }
                    if from == "checkpoint" && to == "gating"
            )
        });
        let completed = event_position(events, "workflow_completed", |event| {
            matches!(event, RuntimeEvent::WorkflowCompleted { .. })
        });

        assert!(
            recovered_phase < completed,
            "run2 should emit the recovered phase transition before completion"
        );
        assert!(
            !events.iter().any(|envelope| matches!(
                &envelope.payload,
                RuntimeEvent::AgentSpawned { role, .. } if role == "implementer"
            )),
            "resume should not spawn a new implementer from a gating checkpoint"
        );
        assert!(
            !events
                .iter()
                .any(|envelope| matches!(&envelope.payload, RuntimeEvent::AgentCompleted { .. })),
            "resume should not replay the completed agent turn"
        );
    }

    fn event_position<F>(events: &[RuntimeEventEnvelope], label: &str, predicate: F) -> usize
    where
        F: Fn(&RuntimeEvent) -> bool,
    {
        events
            .iter()
            .position(|envelope| predicate(&envelope.payload))
            .unwrap_or_else(|| panic!("missing event {label}"))
    }

    fn write_event_log(path: &Path, events: &[RuntimeEventEnvelope]) {
        let mut lines = String::new();
        for event in events {
            lines.push_str(&serde_json::to_string(event).expect("event should serialize"));
            lines.push('\n');
        }
        std::fs::write(path, lines).expect("event log should be written");
    }

    fn unique_test_run_id(label: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("run_{label}_{nanos}")
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
