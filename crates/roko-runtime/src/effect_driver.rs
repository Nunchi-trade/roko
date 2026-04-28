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
    PromptAssembler, PromptSpec, ShellGateCommand, TokenBudget,
};

use crate::event_bus::emit_runtime_event;
use crate::pipeline_state::PipelineInput;

/// Fallible result type used by the effect driver.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 2048;
const MIN_TURN_LIMIT_FACTOR: f32 = 0.25;
const MAX_TURN_LIMIT_FACTOR: f32 = 2.0;
const BASE_TEMPERATURE: f32 = 0.2;
const EXPLORATION_TEMPERATURE_RANGE: f32 = 0.6;
const TIER_TEMPERATURE_RANGE: f32 = 0.1;
const CACHE_BYPASS_EXPLORATION_THRESHOLD: f32 = 0.5;

/// Services required by the `EffectDriver`.
pub struct EffectServices {
    /// Default model used for runtime-dispatched model calls.
    pub model: String,
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
        if modulation.turn_limit_factor <= 0.0 {
            tracing::debug!(
                run_id = %self.run_id,
                agent_id,
                role,
                tier_bias = modulation.tier_bias,
                turn_limit_factor = modulation.turn_limit_factor,
                exploration_rate = modulation.exploration_rate,
                "affect policy deferred agent dispatch"
            );
            return PipelineInput::ResourceExhausted {
                reason: format!("affect policy deferred {role} dispatch"),
            };
        }
        if self.services.model.trim().is_empty() {
            return PipelineInput::AgentFailed {
                error: format!("model is not configured for {role} dispatch"),
            };
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

        let request = model_call_request(
            &self.services.model,
            role,
            system_prompt,
            user_content,
            &modulation,
        );

        tracing::debug!(
            run_id = %self.run_id,
            agent_id,
            role,
            model = %request.model,
            max_tokens = request.max_tokens,
            temperature = request.temperature,
            budget = ?request.budget,
            cache_policy = ?request.cache_policy,
            tier_bias = modulation.tier_bias,
            turn_limit_factor = modulation.turn_limit_factor,
            exploration_rate = modulation.exploration_rate,
            "applied affect dispatch modulation"
        );

        emit_runtime_event(RuntimeEvent::AgentSpawned {
            run_id: self.run_id.clone(),
            agent_id: agent_id.clone(),
            role: role.to_string(),
            model: request.model.clone(),
        });

        let start = Instant::now();
        let request_model = request.model.clone();
        let result = self.services.model_caller.call(request).await;
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
                        model: request_model,
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

fn model_call_request(
    model: &str,
    role: &str,
    system_prompt: String,
    user_content: String,
    modulation: &DispatchModulation,
) -> ModelCallRequest {
    let max_tokens = modulated_max_tokens(modulation);
    ModelCallRequest {
        model: model.to_string(),
        system: Some(system_prompt),
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: user_content,
        }],
        max_tokens: Some(max_tokens),
        temperature: Some(modulated_temperature(modulation)),
        role: Some(role.to_string()),
        caller: None,
        budget: Some(TokenBudget {
            max_input: None,
            max_output: Some(u64::from(max_tokens)),
            max_cost_usd: None,
        }),
        cache_policy: modulated_cache_policy(modulation),
    }
}

fn modulated_max_tokens(modulation: &DispatchModulation) -> u32 {
    let factor = finite_or_default(modulation.turn_limit_factor, 1.0)
        .clamp(MIN_TURN_LIMIT_FACTOR, MAX_TURN_LIMIT_FACTOR);
    ((DEFAULT_MAX_OUTPUT_TOKENS as f32) * factor).round() as u32
}

fn modulated_temperature(modulation: &DispatchModulation) -> f32 {
    let exploration = finite_or_default(modulation.exploration_rate, 0.0).clamp(0.0, 1.0);
    let tier_bias = finite_or_default(modulation.tier_bias, 0.0).clamp(-1.0, 1.0);
    (BASE_TEMPERATURE
        + (exploration * EXPLORATION_TEMPERATURE_RANGE)
        + (tier_bias.max(0.0) * TIER_TEMPERATURE_RANGE))
        .clamp(0.0, 1.0)
}

fn modulated_cache_policy(modulation: &DispatchModulation) -> CachePolicy {
    let exploration = finite_or_default(modulation.exploration_rate, 0.0).clamp(0.0, 1.0);
    if exploration >= CACHE_BYPASS_EXPLORATION_THRESHOLD {
        CachePolicy::Bypass
    } else {
        CachePolicy::Default
    }
}

fn finite_or_default(value: f32, default: f32) -> f32 {
    if value.is_finite() { value } else { default }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;

    use parking_lot::Mutex;
    use roko_core::BehavioralState;
    use roko_core::foundation::AffectContext;

    struct RecordingModelCaller {
        captured: Arc<Mutex<Option<ModelCallRequest>>>,
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
            let captured = Arc::clone(&self.captured);
            Box::pin(async move {
                captured.lock().replace(req);
                Ok(ModelCallResponse {
                    content: "done".to_string(),
                    model: "mock-model".to_string(),
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

    struct StaticPromptAssembler;

    impl PromptAssembler for StaticPromptAssembler {
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

    struct RecordingFeedbackSink;

    impl FeedbackSink for RecordingFeedbackSink {
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

    struct UnusedGateRunner;

    impl GateRunner for UnusedGateRunner {
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

    struct ModulatingAffectPolicy;

    impl AffectPolicy for ModulatingAffectPolicy {
        fn pre_dispatch(&self, _task_id: &str, _role: &str) -> AffectContext {
            AffectContext {
                behavioral_state: BehavioralState::Exploring,
                pad: [0.0, 0.3, 0.1],
                emotional_tag: Some("exploring".to_string()),
            }
        }

        fn on_task_outcome(
            &mut self,
            _task_id: &str,
            _succeeded: bool,
            _tokens_used: u64,
            _cost_usd: f64,
        ) {
        }

        fn on_gate_result(&mut self, _gate_name: &str, _passed: bool, _rung: u8, _confidence: f64) {
        }

        fn modulate_dispatch(&self, _role: &str, params: &mut DispatchModulation) {
            params.tier_bias = 0.4;
            params.turn_limit_factor = 1.5;
            params.exploration_rate = 0.75;
        }

        fn behavioral_state(&self) -> BehavioralState {
            BehavioralState::Exploring
        }

        fn persist<'life0, 'async_trait>(
            &'life0 self,
        ) -> Pin<Box<dyn Future<Output = roko_core::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn spawn_agent_applies_affect_modulation_to_model_request() {
        let captured = Arc::new(Mutex::new(None));
        let services = EffectServices {
            model: "mock-model".to_string(),
            model_caller: Arc::new(RecordingModelCaller {
                captured: Arc::clone(&captured),
            }),
            prompt_assembler: Arc::new(StaticPromptAssembler),
            feedback_sink: Arc::new(RecordingFeedbackSink),
            gate_runner: Arc::new(UnusedGateRunner),
            affect_policy: Some(Arc::new(tokio::sync::Mutex::new(ModulatingAffectPolicy))),
        };
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let driver = EffectDriver::new(services, "run-test".to_string(), tempdir.path().into());

        let input = driver.spawn_agent("implementer", "do work", None).await;

        assert!(matches!(input, PipelineInput::AgentCompleted { .. }));
        let request = captured
            .lock()
            .clone()
            .expect("model caller should capture request");
        assert_eq!(request.model, "mock-model");
        assert_eq!(request.max_tokens, Some(3072));
        assert_eq!(
            request.budget.and_then(|budget| budget.max_output),
            Some(3072)
        );
        assert_eq!(request.cache_policy, CachePolicy::Bypass);
        assert!(
            request
                .temperature
                .is_some_and(|temperature| { (temperature - 0.69).abs() < f32::EPSILON }),
            "expected modulated temperature, got {:?}",
            request.temperature
        );
    }
}
