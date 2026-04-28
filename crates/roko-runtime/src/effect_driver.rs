//! EffectDriver -- executes pipeline actions via foundation-style services.
//!
//! The state machine (`PipelineStateV2`) decides what to do by returning
//! `PipelineOutput`. The `EffectDriver` performs the requested side effects and
//! returns `PipelineInput` values that callers can feed back into the state
//! machine.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub use roko_core::RuntimeEvent;
pub use roko_core::foundation::{
    ChatMessage, GateReport, MessageRole, ModelCallResponse, TokenUsage,
};
use roko_core::foundation::{
    FeedbackEvent, FeedbackSink, GateConfig, GateRunner, GateVerdict, ModelCallRequest,
    ModelCaller, PromptAssembler, PromptSpec,
};

use crate::event_bus::emit_runtime_event;
use crate::pipeline_state::PipelineInput;

/// Fallible result type used by the effect driver.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Boxed future (kept for internal use).
pub type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Services required by the `EffectDriver`.
pub struct EffectServices {
    /// Model call service.
    pub model_caller: Arc<dyn ModelCaller>,
    /// Prompt assembly service.
    pub prompt_assembler: Arc<dyn PromptAssembler>,
    /// Feedback recording service.
    pub feedback_sink: Arc<dyn FeedbackSink>,
    /// Gate execution service.
    pub gate_runner: Arc<dyn GateRunner>,
}

/// Drives workflow execution by translating state-machine actions into effects.
pub struct EffectDriver {
    services: EffectServices,
    run_id: String,
    workdir: PathBuf,
}

impl EffectDriver {
    /// Create a new `EffectDriver` with the given services and workflow context.
    pub fn new(services: EffectServices, run_id: String, workdir: PathBuf) -> Self {
        Self {
            services,
            run_id,
            workdir,
        }
    }

    /// Spawn an agent with the given role and prompt.
    ///
    /// Returns a `PipelineInput::AgentCompleted` or `PipelineInput::AgentFailed`
    /// that should be fed back into the state machine.
    pub async fn spawn_agent(
        &self,
        role: &str,
        user_prompt: &str,
        context: Option<&str>,
    ) -> PipelineInput {
        let agent_id = format!("{role}_{}", uuid_short());

        let system_prompt = match self
            .services
            .prompt_assembler
            .assemble(PromptSpec {
                role: Some(role.to_string()),
                task: Some(user_prompt.to_string()),
                workdir: Some(self.workdir.clone()),
                gate_feedback: Vec::new(),
                anti_patterns: Vec::new(),
            })
            .await
        {
            Ok(prompt) => prompt,
            Err(err) => {
                return PipelineInput::AgentFailed {
                    error: format!("Failed to assemble prompt: {err}"),
                };
            }
        };

        let user_content = context.map_or_else(
            || user_prompt.to_string(),
            |ctx| format!("{user_prompt}\n\n## Additional Context\n\n{ctx}"),
        );

        emit_runtime_event(RuntimeEvent::AgentSpawned {
            run_id: self.run_id.clone(),
            agent_id: agent_id.clone(),
            role: role.to_string(),
            model: String::new(),
        });

        let start = Instant::now();
        let result = self
            .services
            .model_caller
            .call(ModelCallRequest {
                model: String::new(),
                system: Some(system_prompt),
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: user_content,
                }],
                max_tokens: None,
                temperature: None,
                role: Some(role.to_string()),
            })
            .await;
        let latency_ms = duration_millis(start);

        match result {
            Ok(response) => {
                let _record_result = self
                    .services
                    .feedback_sink
                    .record(FeedbackEvent::ModelCall {
                        run_id: self.run_id.clone(),
                        model: response.model.clone(),
                        role: role.to_string(),
                        input_tokens: response.usage.input_tokens,
                        output_tokens: response.usage.output_tokens,
                        cost_usd: response.usage.cost_usd,
                        latency_ms,
                        success: true,
                    })
                    .await;

                emit_runtime_event(RuntimeEvent::AgentCompleted {
                    run_id: self.run_id.clone(),
                    agent_id,
                    output: response.content.clone(),
                    tokens_used: response.usage.total_tokens,
                    cost_usd: response.usage.cost_usd,
                });

                PipelineInput::AgentCompleted {
                    output: response.content,
                    files_changed: 0, // TODO(arch): detect from git diff once file tracking lands.
                }
            }
            Err(err) => {
                let error = err.to_string();
                let _record_result = self
                    .services
                    .feedback_sink
                    .record(FeedbackEvent::ModelCall {
                        run_id: self.run_id.clone(),
                        model: String::new(),
                        role: role.to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: 0.0,
                        latency_ms,
                        success: false,
                    })
                    .await;

                emit_runtime_event(RuntimeEvent::AgentFailed {
                    run_id: self.run_id.clone(),
                    agent_id,
                    error: error.clone(),
                });

                PipelineInput::AgentFailed { error }
            }
        }
    }

    /// Run verification gates.
    ///
    /// Returns `PipelineInput::GatesPassed` or `PipelineInput::GateFailed`.
    pub async fn run_gates(&self, enabled_gates: &[String]) -> PipelineInput {
        let config = GateConfig {
            workdir: self.workdir.clone(),
            enabled_gates: enabled_gates.to_vec(),
            max_rung: None,
        };

        let result = self.services.gate_runner.run_gates(config).await;

        match result {
            Ok(report) => {
                for verdict in &report.verdicts {
                    self.record_gate_verdict(verdict).await;
                }

                match report.first_failure() {
                    Some(failure) => PipelineInput::GateFailed {
                        gate: failure.gate_name.clone(),
                        output: report.failure_summary(),
                    },
                    None => PipelineInput::GatesPassed,
                }
            }
            Err(err) => PipelineInput::GateFailed {
                gate: "gate_runner".to_string(),
                output: err.to_string(),
            },
        }
    }

    /// Create a git commit.
    ///
    /// Returns `PipelineInput::CommitDone` when a commit is created, or a noop
    /// hash when there is nothing to commit.
    pub async fn commit(&self, message: &str) -> PipelineInput {
        let add_result = tokio::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.workdir)
            .output()
            .await;

        if let Err(err) = add_result {
            return PipelineInput::AgentFailed {
                error: format!("git add failed: {err}"),
            };
        }

        let commit_result = tokio::process::Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.workdir)
            .output()
            .await;

        match commit_result {
            Ok(output) if output.status.success() => {
                let hash_output = tokio::process::Command::new("git")
                    .args(["rev-parse", "--short", "HEAD"])
                    .current_dir(&self.workdir)
                    .output()
                    .await;

                let hash = hash_output
                    .ok()
                    .and_then(|output| String::from_utf8(output.stdout).ok())
                    .map_or_else(|| "unknown".to_string(), |hash| hash.trim().to_string());

                PipelineInput::CommitDone { hash }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("nothing to commit") {
                    PipelineInput::CommitDone {
                        hash: "noop".to_string(),
                    }
                } else {
                    PipelineInput::AgentFailed {
                        error: format!("git commit failed: {stderr}"),
                    }
                }
            }
            Err(err) => PipelineInput::AgentFailed {
                error: format!("git commit failed: {err}"),
            },
        }
    }

    /// Emit a runtime event directly.
    pub fn emit(&self, event: RuntimeEvent) {
        emit_runtime_event(event);
    }

    async fn record_gate_verdict(&self, verdict: &GateVerdict) {
        let event = match verdict.passed {
            true => RuntimeEvent::GatePassed {
                run_id: self.run_id.clone(),
                gate_name: verdict.gate_name.clone(),
                duration_ms: verdict.duration_ms,
            },
            false => RuntimeEvent::GateFailed {
                run_id: self.run_id.clone(),
                gate_name: verdict.gate_name.clone(),
                output: verdict.output.clone(),
                duration_ms: verdict.duration_ms,
            },
        };
        emit_runtime_event(event);

        let _record_result = self
            .services
            .feedback_sink
            .record(FeedbackEvent::GateResult {
                run_id: self.run_id.clone(),
                gate_name: verdict.gate_name.clone(),
                passed: verdict.passed,
                duration_ms: verdict.duration_ms,
            })
            .await;
    }
}

fn duration_millis(start: Instant) -> u64 {
    let millis = start.elapsed().as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

/// Generate a short unique ID for agent instances.
fn uuid_short() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    format!("{:x}", millis & 0xFFFF_FFFF)
}
