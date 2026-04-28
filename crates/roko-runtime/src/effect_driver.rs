//! EffectDriver -- executes pipeline actions via foundation-style services.
//!
//! The state machine (`PipelineStateV2`) decides what to do by returning
//! `PipelineOutput`. The `EffectDriver` performs the requested side effects and
//! returns `PipelineInput` values that callers can feed back into the state
//! machine.
//!
//! TODO(arch): Replace the local foundation-compatible contracts below with
//! `roko_core::foundation` and `roko_core::RuntimeEvent` after the manifest
//! dependency direction is corrected. This checkout currently has
//! `roko-core -> roko-runtime`, so importing `roko_core` here would create a
//! crate cycle, and Cargo.toml changes are out of scope for this batch.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::event_bus::emit_runtime_event;
use crate::pipeline_state::PipelineInput;

/// Fallible result type used by the effect driver service contracts.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Boxed future returned by async service trait methods.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Request to call an LLM model.
#[derive(Debug, Clone)]
pub struct ModelCallRequest {
    /// Model identifier. An empty value lets the model service route by role.
    pub model: String,
    /// System prompt assembled for the agent.
    pub system: Option<String>,
    /// Conversation messages.
    pub messages: Vec<ChatMessage>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Role hint for model routing.
    pub role: Option<String>,
}

/// A single chat message.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// Message role.
    pub role: MessageRole,
    /// Message content.
    pub content: String,
}

/// Message role in a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    /// System-authored message.
    System,
    /// User-authored message.
    User,
    /// Assistant-authored message.
    Assistant,
}

/// Response from a model call.
#[derive(Debug, Clone)]
pub struct ModelCallResponse {
    /// Complete model output.
    pub content: String,
    /// Resolved model identifier.
    pub model: String,
    /// Token and cost accounting.
    pub usage: TokenUsage,
    /// Provider stop reason, when available.
    pub stop_reason: Option<String>,
}

/// Token usage and cost from a model call.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Prompt/input tokens.
    pub input_tokens: u64,
    /// Completion/output tokens.
    pub output_tokens: u64,
    /// Total tokens.
    pub total_tokens: u64,
    /// Estimated call cost in USD.
    pub cost_usd: f64,
}

/// Call an LLM model.
pub trait ModelCaller: Send + Sync {
    /// Single-shot model call, returning the complete response.
    fn call(&self, req: ModelCallRequest) -> BoxFuture<'_, Result<ModelCallResponse>>;
}

/// Specification for assembling a system prompt.
#[derive(Debug, Clone, Default)]
pub struct PromptSpec {
    /// Agent role.
    pub role: Option<String>,
    /// Task description.
    pub task: Option<String>,
    /// Working directory for convention detection.
    pub workdir: Option<PathBuf>,
    /// Gate feedback from prior attempts.
    pub gate_feedback: Vec<String>,
    /// Anti-patterns to include.
    pub anti_patterns: Vec<String>,
}

/// Assemble a system prompt for a given role and context.
pub trait PromptAssembler: Send + Sync {
    /// Build a complete system prompt from the spec.
    fn assemble(&self, spec: PromptSpec) -> BoxFuture<'_, Result<String>>;
}

/// A feedback event to record.
#[derive(Debug, Clone)]
pub enum FeedbackEvent {
    /// Feedback from a model call.
    ModelCall {
        /// Workflow run id.
        run_id: String,
        /// Resolved model identifier.
        model: String,
        /// Agent role.
        role: String,
        /// Prompt/input tokens.
        input_tokens: u64,
        /// Completion/output tokens.
        output_tokens: u64,
        /// Estimated call cost in USD.
        cost_usd: f64,
        /// End-to-end call latency.
        latency_ms: u64,
        /// Whether the call succeeded.
        success: bool,
    },
    /// Feedback from a gate execution.
    GateResult {
        /// Workflow run id.
        run_id: String,
        /// Gate name.
        gate_name: String,
        /// Whether the gate passed.
        passed: bool,
        /// Gate runtime.
        duration_ms: u64,
    },
}

/// Record feedback from model calls and gate results.
pub trait FeedbackSink: Send + Sync {
    /// Record a feedback event.
    fn record(&self, event: FeedbackEvent) -> BoxFuture<'_, Result<()>>;
}

/// Configuration for a gate run.
#[derive(Debug, Clone)]
pub struct GateConfig {
    /// Working directory to verify.
    pub workdir: PathBuf,
    /// Gate names to run.
    pub enabled_gates: Vec<String>,
    /// Maximum rung to run.
    pub max_rung: Option<u8>,
}

/// Result from a single gate.
#[derive(Debug, Clone)]
pub struct GateVerdict {
    /// Gate name.
    pub gate_name: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Gate output or diagnostics.
    pub output: String,
    /// Gate runtime.
    pub duration_ms: u64,
}

/// Report from running gates.
#[derive(Debug, Clone)]
pub struct GateReport {
    /// Individual verdicts.
    pub verdicts: Vec<GateVerdict>,
}

impl GateReport {
    /// Returns true when every reported gate passed.
    pub fn all_passed(&self) -> bool {
        self.verdicts.iter().all(|verdict| verdict.passed)
    }

    /// Returns the first failing gate.
    pub fn first_failure(&self) -> Option<&GateVerdict> {
        self.verdicts.iter().find(|verdict| !verdict.passed)
    }

    /// Collects all failure outputs for agent feedback.
    pub fn failure_summary(&self) -> String {
        self.verdicts
            .iter()
            .filter(|verdict| !verdict.passed)
            .map(|verdict| format!("{}: {}", verdict.gate_name, verdict.output))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Run verification gates against a working directory.
pub trait GateRunner: Send + Sync {
    /// Execute gates per the config.
    fn run_gates(&self, config: GateConfig) -> BoxFuture<'_, Result<GateReport>>;
}

/// Runtime events emitted by the effect driver.
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    /// An agent was spawned.
    AgentSpawned {
        /// Workflow run id.
        run_id: String,
        /// Agent instance id.
        agent_id: String,
        /// Agent role.
        role: String,
        /// Resolved or requested model.
        model: String,
    },
    /// An agent completed successfully.
    AgentCompleted {
        /// Workflow run id.
        run_id: String,
        /// Agent instance id.
        agent_id: String,
        /// Final output.
        output: String,
        /// Tokens used by the call.
        tokens_used: u64,
        /// Estimated call cost in USD.
        cost_usd: f64,
    },
    /// An agent failed.
    AgentFailed {
        /// Workflow run id.
        run_id: String,
        /// Agent instance id.
        agent_id: String,
        /// Error text.
        error: String,
    },
    /// A gate passed.
    GatePassed {
        /// Workflow run id.
        run_id: String,
        /// Gate name.
        gate_name: String,
        /// Gate runtime.
        duration_ms: u64,
    },
    /// A gate failed.
    GateFailed {
        /// Workflow run id.
        run_id: String,
        /// Gate name.
        gate_name: String,
        /// Gate output.
        output: String,
        /// Gate runtime.
        duration_ms: u64,
    },
}

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
