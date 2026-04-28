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

use crate::effect_driver::{EffectDriver, EffectServices, Result};
use crate::event_bus::emit_runtime_event;
pub use crate::pipeline_state::WorkflowOutcome;
use crate::pipeline_state::{PipelineInput, PipelineOutput, PipelineStateV2, WorkflowConfig};

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
        let run_id = generate_run_id();

        let mut pipeline = PipelineStateV2::new(config.workflow.clone(), config.prompt.clone());

        let driver = EffectDriver::new(
            EffectServices {
                model_caller: Arc::clone(&self.services.model_caller),
                prompt_assembler: Arc::clone(&self.services.prompt_assembler),
                feedback_sink: Arc::clone(&self.services.feedback_sink),
                gate_runner: Arc::clone(&self.services.gate_runner),
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
                PipelineOutput::SpawnReviewer { diff_context } => reviewer_input(
                    driver
                        .spawn_agent("reviewer", "Review the changes", diff_context.as_deref())
                        .await,
                ),
                PipelineOutput::RunGates => driver.run_gates(&config.enabled_gates).await,
                PipelineOutput::Commit => {
                    let message = commit_message(&config);
                    driver.commit(&message).await
                }
                PipelineOutput::Done { outcome } => {
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: runtime_workflow_outcome(outcome),
                    });

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

    fn emit(&self, event: RuntimeEvent) {
        for consumer in &self.consumers {
            consumer.consume(&event);
        }

        emit_runtime_event(event);
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

fn commit_message(config: &WorkflowRunConfig) -> String {
    let prompt = truncate(&config.prompt, 60);
    config.commit_prefix.as_deref().map_or_else(
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
}
