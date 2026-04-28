//! WorkflowEngine -- top-level workflow execution facade.
//!
//! Ties together `PipelineStateV2` (decisions) and `EffectDriver` (effects)
//! into a run loop. This is the shared entry point for CLI, ACP, and HTTP.
//!
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use roko_core::RuntimeEvent;
use roko_core::foundation::EventConsumer;

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
    pub async fn run(&self, config: WorkflowRunConfig) -> Result<WorkflowResult> {
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
    pub async fn run_with_cancel(
        &self,
        config: WorkflowRunConfig,
        token: CancelToken,
    ) -> Result<WorkflowResult> {
        let run_id = generate_run_id();

        let mut pipeline = PipelineStateV2::new(config.workflow.clone(), config.prompt.clone());

        let driver = EffectDriver::new(
            EffectServices {
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
                    self.persist_affect_policy().await;
                    return Ok(WorkflowResult {
                        run_id,
                        outcome,
                        iterations: pipeline.iteration,
                    });
                }
            }

            let old_phase = pipeline.phase.label();

            let input = match &output {
                PipelineOutput::SpawnStrategist { prompt } => {
                    self.emit(RuntimeEvent::AgentSpawned {
                        run_id: run_id.clone(),
                        agent_id: String::new(),
                        role: "strategist".to_string(),
                        model: String::new(),
                    });
                    strategy_input(driver.spawn_agent("strategist", prompt, None).await)
                }
                PipelineOutput::SpawnImplementer { prompt, context } => {
                    self.emit(RuntimeEvent::AgentSpawned {
                        run_id: run_id.clone(),
                        agent_id: String::new(),
                        role: "implementer".to_string(),
                        model: String::new(),
                    });
                    driver
                        .spawn_agent("implementer", prompt, context.as_deref())
                        .await
                }
                PipelineOutput::SpawnAutoFixer { error_output } => {
                    self.emit(RuntimeEvent::AgentSpawned {
                        run_id: run_id.clone(),
                        agent_id: String::new(),
                        role: "autofix".to_string(),
                        model: String::new(),
                    });
                    driver
                        .spawn_agent("autofix", "Fix the following errors", Some(error_output))
                        .await
                }
                PipelineOutput::SpawnReviewer { diff_context } => reviewer_input({
                    self.emit(RuntimeEvent::AgentSpawned {
                        run_id: run_id.clone(),
                        agent_id: String::new(),
                        role: "reviewer".to_string(),
                        model: String::new(),
                    });
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
                    driver.run_gates(&config.enabled_gates).await
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

                    self.persist_affect_policy().await;
                    // TODO(arch): Record final workflow feedback once the local
                    // `FeedbackEvent` includes `WorkflowComplete`.
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

                    self.persist_affect_policy().await;
                    // TODO(arch): Record final workflow feedback once the local
                    // `FeedbackEvent` includes `WorkflowComplete`.
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
                self.emit(RuntimeEvent::PhaseTransition {
                    run_id: run_id.clone(),
                    from: old_phase.to_string(),
                    to: new_phase.to_string(),
                });
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

        let driver = EffectDriver::new(
            EffectServices {
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
        let mut output = resumed_output(&mut pipeline);

        loop {
            let old_phase = pipeline.phase.label();

            let input = match &output {
                PipelineOutput::SpawnStrategist { prompt } => {
                    self.emit(RuntimeEvent::AgentSpawned {
                        run_id: run_id.clone(),
                        agent_id: String::new(),
                        role: "strategist".to_string(),
                        model: String::new(),
                    });
                    strategy_input(driver.spawn_agent("strategist", prompt, None).await)
                }
                PipelineOutput::SpawnImplementer { prompt, context } => {
                    self.emit(RuntimeEvent::AgentSpawned {
                        run_id: run_id.clone(),
                        agent_id: String::new(),
                        role: "implementer".to_string(),
                        model: String::new(),
                    });
                    driver
                        .spawn_agent("implementer", prompt, context.as_deref())
                        .await
                }
                PipelineOutput::SpawnAutoFixer { error_output } => {
                    self.emit(RuntimeEvent::AgentSpawned {
                        run_id: run_id.clone(),
                        agent_id: String::new(),
                        role: "autofix".to_string(),
                        model: String::new(),
                    });
                    driver
                        .spawn_agent("autofix", "Fix the following errors", Some(error_output))
                        .await
                }
                PipelineOutput::SpawnReviewer { diff_context } => reviewer_input({
                    self.emit(RuntimeEvent::AgentSpawned {
                        run_id: run_id.clone(),
                        agent_id: String::new(),
                        role: "reviewer".to_string(),
                        model: String::new(),
                    });
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
                    driver.run_gates(&config.enabled_gates).await
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
                self.emit(RuntimeEvent::PhaseTransition {
                    run_id: run_id.clone(),
                    from: old_phase.to_string(),
                    to: new_phase.to_string(),
                });

                if pipeline.checkpoint().is_ok() {
                    self.emit(RuntimeEvent::StateCheckpointed {
                        run_id: run_id.clone(),
                        path: String::new(),
                    });
                }
            }
        }
    }

    fn emit(&self, event: RuntimeEvent) {
        for consumer in &self.consumers {
            consumer.consume(&event);
        }

        emit_runtime_event(event);
    }

    async fn persist_affect_policy(&self) {
        if let Some(ref affect) = self.services.affect_policy {
            let policy = affect.lock().await;
            let _ = policy.persist().await;
        }
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
        PipelineInput::AgentCompleted { output, .. } => {
            // TODO(arch): Replace this default approval with a structured reviewer
            // effect outcome that can also express `ReviewRevise`.
            PipelineInput::ReviewApproved { summary: output }
        }
        input => input,
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
        &s[..s.floor_char_boundary(max)]
    }
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
            commit_prefix: Some("fix".to_string()),
        };

        assert_eq!(commit_message(&config), "fix: short prompt");
    }

    struct MockModelCaller;

    impl ModelCaller for MockModelCaller {
        fn call<'life0, 'async_trait>(
            &'life0 self,
            _req: ModelCallRequest,
        ) -> Pin<Box<dyn Future<Output = roko_core::Result<ModelCallResponse>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async {
                Ok(ModelCallResponse {
                    content: "done".into(),
                    model: "mock".into(),
                    usage: TokenUsage::default(),
                    stop_reason: None,
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
        assert!(matches!(result.outcome, WorkflowOutcome::Success { .. }));
        assert_eq!(result.iterations, 1);
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
        assert!(matches!(result.outcome, WorkflowOutcome::Success { .. }));
        assert_eq!(result.iterations, 1);
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
