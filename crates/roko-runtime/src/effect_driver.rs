//! EffectDriver -- executes pipeline actions via foundation-style services.
//!
//! The state machine (`PipelineStateV2`) decides what to do by returning
//! `PipelineOutput`. The `EffectDriver` performs the requested side effects and
//! returns `PipelineInput` values that callers can feed back into the state
//! machine.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub use roko_core::RuntimeEvent;
pub use roko_core::foundation::{
    AffectPolicy, ChatMessage, DispatchModulation, GateReport, MessageRole, ModelCallRequest,
    ModelCallResponse, TokenUsage,
};
use roko_core::foundation::{
    CachePolicy, FeedbackEvent, FeedbackSink, GateConfig, GateRunner, GateVerdict, ModelCaller,
    PromptAssembler, PromptSpec, ShellGateCommand,
};

use crate::event_bus::emit_runtime_event;
use crate::pipeline_state::PipelineInput;

/// Fallible result type used by the effect driver.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

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
    /// Optional affect policy for behavioral modulation.
    pub affect_policy: Option<Arc<tokio::sync::Mutex<dyn AffectPolicy>>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowFeedbackTotals {
    pub primary_model: Option<String>,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
}

/// Drives workflow execution by translating state-machine actions into effects.
pub struct EffectDriver {
    services: EffectServices,
    run_id: String,
    workdir: PathBuf,
    feedback_totals: tokio::sync::Mutex<WorkflowFeedbackTotals>,
}

impl EffectDriver {
    /// Create a new `EffectDriver` with the given services and workflow context.
    pub fn new(services: EffectServices, run_id: String, workdir: PathBuf) -> Self {
        Self {
            services,
            run_id,
            workdir,
            feedback_totals: tokio::sync::Mutex::new(WorkflowFeedbackTotals::default()),
        }
    }

    pub(crate) async fn workflow_feedback_totals(&self) -> WorkflowFeedbackTotals {
        self.feedback_totals.lock().await.clone()
    }

    /// Spawn an agent with the given role and prompt.
    ///
    /// Returns a `PipelineInput::AgentCompleted` or `PipelineInput::AgentFailed`
    /// that should be fed back into the state machine.
    #[allow(clippy::too_many_lines)]
    pub async fn spawn_agent(
        &self,
        role: &str,
        user_prompt: &str,
        context: Option<&str>,
    ) -> PipelineInput {
        let agent_id = format!("{role}_{}", uuid_short());

        let mut modulation = DispatchModulation::default();
        if let Some(ref affect) = self.services.affect_policy {
            let policy = affect.lock().await;
            let _ctx = policy.pre_dispatch(&agent_id, role);
            policy.modulate_dispatch(role, &mut modulation);
        }

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
                caller: None,
                budget: None,
                cache_policy: CachePolicy::Default,
            })
            .await;
        let latency_ms = duration_millis(start);

        match result {
            Ok(response) => {
                self.record_model_totals(role, &response).await;

                if let Some(ref affect) = self.services.affect_policy {
                    let mut policy = affect.lock().await;
                    policy.on_task_outcome(
                        &agent_id,
                        true,
                        response.usage.total_tokens,
                        response.usage.cost_usd,
                    );
                }

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

                let files_changed = count_changed_files(&self.workdir).await;
                PipelineInput::AgentCompleted {
                    output: response.content,
                    files_changed,
                }
            }
            Err(err) => {
                let error = err.to_string();
                if let Some(ref affect) = self.services.affect_policy {
                    let mut policy = affect.lock().await;
                    policy.on_task_outcome(&agent_id, false, 0, 0.0);
                }

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
    pub async fn run_gates(
        &self,
        enabled_gates: &[String],
        shell_gates: &[ShellGateCommand],
    ) -> PipelineInput {
        let config = GateConfig {
            workdir: self.workdir.clone(),
            enabled_gates: enabled_gates.to_vec(),
            shell_gates: shell_gates.to_vec(),
            max_rung: None,
        };

        let result = self.services.gate_runner.run_gates(config).await;

        match result {
            Ok(report) => {
                for verdict in &report.verdicts {
                    self.record_gate_verdict(verdict).await;
                }

                if let Some(ref affect) = self.services.affect_policy {
                    let mut policy = affect.lock().await;
                    for verdict in &report.verdicts {
                        policy.on_gate_result(&verdict.gate_name, verdict.passed, 0, 0.0);
                    }
                }

                report
                    .first_failure()
                    .map_or(PipelineInput::GatesPassed, |failure| {
                        PipelineInput::GateFailed {
                            gate: failure.gate_name.clone(),
                            output: report.failure_summary(),
                        }
                    })
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
            self.emit(RuntimeEvent::FeedbackRecorded {
                run_id: self.run_id.clone(),
                kind: "commit_error".to_string(),
                summary: format!("git add failed: {err}"),
            });
            // Return CommitDone (not AgentFailed) because the state machine is in
            // Phase::Committing which only handles CommitDone transitions.
            return PipelineInput::CommitDone {
                hash: format!("error: git add failed: {err}"),
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

                self.emit(RuntimeEvent::FeedbackRecorded {
                    run_id: self.run_id.clone(),
                    kind: "commit".to_string(),
                    summary: format!("committed {hash}: {}", truncate_message(message, 72)),
                });

                PipelineInput::CommitDone { hash }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("nothing to commit") {
                    self.emit(RuntimeEvent::FeedbackRecorded {
                        run_id: self.run_id.clone(),
                        kind: "commit_noop".to_string(),
                        summary: "nothing to commit, working tree clean".to_string(),
                    });
                    PipelineInput::CommitDone {
                        hash: "noop".to_string(),
                    }
                } else {
                    PipelineInput::CommitDone {
                        hash: format!("error: git commit failed: {stderr}"),
                    }
                }
            }
            Err(err) => PipelineInput::CommitDone {
                hash: format!("error: git commit failed: {err}"),
            },
        }
    }

    /// Serialize `state` to JSON and write it atomically to `path`.
    ///
    /// The write is atomic: the JSON is first written to `<path>.tmp`, then
    /// renamed to `path`. If the parent directory does not exist, it is created.
    ///
    /// On success, emits `RuntimeEvent::StateCheckpointed`.
    pub async fn save_checkpoint(
        &self,
        state: &crate::pipeline_state::PipelineStateV2,
        path: &std::path::Path,
    ) -> Result<()> {
        let json = state.checkpoint()?;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let tmp_path = path.with_extension("tmp");
        tokio::fs::write(&tmp_path, &json).await?;
        tokio::fs::rename(&tmp_path, path).await?;

        self.emit(RuntimeEvent::FeedbackRecorded {
            run_id: self.run_id.clone(),
            kind: "checkpoint".to_string(),
            summary: format!("state saved to {}", path.display()),
        });
        self.emit(RuntimeEvent::StateCheckpointed {
            run_id: self.run_id.clone(),
            path: path.display().to_string(),
        });

        Ok(())
    }

    /// Emit a runtime event directly.
    pub fn emit(&self, event: RuntimeEvent) {
        emit_runtime_event(event);
    }

    async fn record_gate_verdict(&self, verdict: &GateVerdict) {
        let event = if verdict.passed {
            RuntimeEvent::GatePassed {
                run_id: self.run_id.clone(),
                gate_name: verdict.gate_name.clone(),
                duration_ms: verdict.duration_ms,
            }
        } else {
            RuntimeEvent::GateFailed {
                run_id: self.run_id.clone(),
                gate_name: verdict.gate_name.clone(),
                output: verdict.output.clone(),
                duration_ms: verdict.duration_ms,
            }
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

    async fn record_model_totals(&self, role: &str, response: &ModelCallResponse) {
        let mut totals = self.feedback_totals.lock().await;
        if !response.model.is_empty() && (totals.primary_model.is_none() || role == "implementer") {
            totals.primary_model = Some(response.model.clone());
        }
        totals.total_tokens = totals
            .total_tokens
            .saturating_add(response.usage.total_tokens);
        totals.total_cost_usd += response.usage.cost_usd;
    }
}

fn duration_millis(start: Instant) -> u64 {
    let millis = start.elapsed().as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

fn truncate_message(s: &str, max: usize) -> &str {
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

/// Count the number of files changed in the working directory via `git diff --name-only`.
///
/// Returns 0 on any error (git not available, not a repo, etc.) -- this is a best-effort
/// enrichment, not a gate.
async fn count_changed_files(workdir: &std::path::Path) -> u32 {
    let result = tokio::process::Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .current_dir(workdir)
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            u32::try_from(stdout.lines().filter(|l| !l.trim().is_empty()).count())
                .unwrap_or(u32::MAX)
        }
        _ => 0,
    }
}

/// Generate a short unique ID for agent instances.
fn uuid_short() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    format!("{:x}", millis & 0xFFFF_FFFF)
}
