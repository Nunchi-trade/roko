//! Foundation traits for the workflow engine.
//!
//! These define the contracts between the engine and its services:
//! - `ModelCaller` - call LLMs (implemented by roko-agent)
//! - `PromptAssembler` - build system prompts (implemented by roko-compose)
//! - `FeedbackSink` - record feedback (implemented by roko-learn)
//! - `GateRunner` - run verification gates (implemented by roko-gate)
//! - `EventConsumer` - observe runtime events (implemented by adapters)
//! - `EffectExecutor` - execute side-effects (implemented by roko-runtime)

use crate::runtime_event::RuntimeEvent;
use crate::{Result, RokoError};
use async_trait::async_trait;
use std::path::PathBuf;

// -- ModelCaller --

/// Request to call an LLM model.
#[derive(Debug, Clone, Default)]
pub struct ModelCallRequest {
    /// Model identifier (e.g., "claude-sonnet-4-20250514").
    pub model: String,
    /// System prompt.
    pub system: Option<String>,
    /// User messages.
    pub messages: Vec<ChatMessage>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Temperature (0.0-1.0).
    pub temperature: Option<f32>,
    /// Role for model routing.
    pub role: Option<String>,
    /// Caller surface that originated this request.
    pub caller: Option<CallerIdentity>,
    /// Per-call token and cost budget.
    pub budget: Option<TokenBudget>,
    /// Cache behavior for this request.
    pub cache_policy: CachePolicy,
}

/// Who originated this model call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CallerIdentity {
    Cli,
    Acp,
    Serve,
    Research,
    Dreams,
    Test,
}

/// Cache behaviour for this request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CachePolicy {
    /// Use the default L1 cache behaviour.
    #[default]
    Default,
    /// Skip cache lookup but still store the result.
    Bypass,
    /// Skip cache lookup AND discard any prior cached result for this key.
    ForceRefresh,
}

/// Token and cost budget for a single model call.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct TokenBudget {
    /// Maximum input tokens the gateway should accept.
    pub max_input: Option<u64>,
    /// Maximum output tokens requested from the provider.
    pub max_output: Option<u64>,
    /// Maximum cost in USD for this single call.
    pub max_cost_usd: Option<f64>,
}

/// Errors specific to the gateway pipeline (not provider errors).
#[derive(Debug, Clone, thiserror::Error)]
pub enum GatewayError {
    #[error("provider error: {0}")]
    ProviderError(String),
    #[error("budget exceeded: {detail}")]
    BudgetExceeded { detail: String },
    #[error("rate limited: retry after {retry_after_ms:?} ms")]
    RateLimited { retry_after_ms: Option<u64> },
    #[error("cache error: {0}")]
    CacheError(String),
    #[error("request cancelled")]
    Cancelled,
    #[error("convergence detected after {consecutive} identical outputs")]
    ConvergenceDetected { consecutive: u32 },
}

impl From<GatewayError> for RokoError {
    fn from(error: GatewayError) -> Self {
        // TODO(converge): Map to RokoError::Other once that variant exists in roko-core.
        RokoError::invalid(error.to_string())
    }
}

/// A single chat message.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

/// Message role in a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

/// Response from a model call.
#[derive(Debug, Clone)]
pub struct ModelCallResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub stop_reason: Option<String>,
    /// Gateway request id, set by ModelCallService when the call flows through the gateway.
    pub request_id: Option<String>,
}

/// Token usage and cost from a model call.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// Call an LLM model. Wraps provider selection, streaming, cost tracking.
#[async_trait]
pub trait ModelCaller: Send + Sync {
    /// Single-shot model call, returns complete response.
    async fn call(&self, req: ModelCallRequest) -> Result<ModelCallResponse>;
}

// -- PromptAssembler --

/// Specification for assembling a system prompt.
#[derive(Debug, Clone, Default)]
pub struct PromptSpec {
    /// Agent role (determines identity layer).
    pub role: Option<String>,
    /// Task description.
    pub task: Option<String>,
    /// Working directory for convention detection.
    pub workdir: Option<PathBuf>,
    /// Gate feedback from prior iterations.
    pub gate_feedback: Vec<String>,
    /// Anti-patterns to include.
    pub anti_patterns: Vec<String>,
}

/// Assemble a system prompt for a given role and context.
#[async_trait]
pub trait PromptAssembler: Send + Sync {
    /// Build a complete system prompt from the spec.
    async fn assemble(&self, spec: PromptSpec) -> Result<String>;
}

// -- FeedbackSink --

/// A feedback event to record.
#[derive(Debug, Clone)]
pub enum FeedbackEvent {
    /// Feedback from a model call.
    ModelCall {
        run_id: String,
        model: String,
        role: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        latency_ms: u64,
        success: bool,
    },
    /// Feedback from a gate execution.
    GateResult {
        run_id: String,
        gate_name: String,
        passed: bool,
        duration_ms: u64,
    },
    /// Feedback from a workflow completion.
    WorkflowComplete {
        run_id: String,
        outcome: String,
        total_cost_usd: f64,
        total_tokens: u64,
        duration_ms: u64,
    },
}

/// Record feedback from model calls, gate results, and workflow outcomes.
#[async_trait]
pub trait FeedbackSink: Send + Sync {
    /// Record a feedback event.
    async fn record(&self, event: FeedbackEvent) -> Result<()>;
}

// -- GateRunner --

/// Configuration for a gate run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShellGateCommand {
    /// Program to invoke.
    pub program: String,
    /// Args to pass.
    pub args: Vec<String>,
    /// Timeout in milliseconds.
    pub timeout_ms: u64,
}

/// Configuration for a gate run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GateConfig {
    /// Working directory to verify.
    pub workdir: PathBuf,
    /// Which gates to run (e.g., ["compile", "test", "clippy"]).
    pub enabled_gates: Vec<String>,
    /// Configured shell commands, consumed by shell/custom:shell gate entries.
    pub shell_gates: Vec<ShellGateCommand>,
    /// Maximum rung to run (0-6).
    pub max_rung: Option<u8>,
}

/// Result from a single gate.
#[derive(Debug, Clone)]
pub struct GateVerdict {
    pub gate_name: String,
    pub passed: bool,
    pub output: String,
    pub duration_ms: u64,
}

/// Report from running a set of gates.
#[derive(Debug, Clone)]
pub struct GateReport {
    pub verdicts: Vec<GateVerdict>,
}

impl GateReport {
    /// Returns true if all gates passed.
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.verdicts.iter().all(|v| v.passed)
    }

    /// Returns the first failing gate, if any.
    #[must_use]
    pub fn first_failure(&self) -> Option<&GateVerdict> {
        self.verdicts.iter().find(|v| !v.passed)
    }

    /// Collects all failure outputs for agent feedback.
    #[must_use]
    pub fn failure_summary(&self) -> String {
        self.verdicts
            .iter()
            .filter(|v| !v.passed)
            .map(|v| format!("{}: {}", v.gate_name, v.output))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Run a set of verification gates against a working directory.
#[async_trait]
pub trait GateRunner: Send + Sync {
    /// Execute gates per the config, returning a report.
    async fn run_gates(&self, config: GateConfig) -> Result<GateReport>;
}

// -- EventConsumer --

/// Consume RuntimeEvents for side-effects (logging, UI updates, etc).
///
/// Consumers must be non-blocking. If they need async work, they should
/// buffer internally and process asynchronously.
pub trait EventConsumer: Send + Sync {
    /// Called for each event emitted by the workflow engine.
    fn consume(&self, event: &RuntimeEvent);
}

// -- EffectExecutor --

/// A side-effect the workflow engine needs to execute.
#[derive(Debug, Clone)]
pub enum Effect {
    /// Spawn an agent with the given role and prompt.
    SpawnAgent {
        run_id: String,
        role: String,
        model: String,
        system_prompt: String,
        user_prompt: String,
        workdir: PathBuf,
    },
    /// Run verification gates.
    RunGates { run_id: String, config: GateConfig },
    /// Create a git commit.
    Commit {
        run_id: String,
        workdir: PathBuf,
        message: String,
    },
    /// Persist a state checkpoint.
    Checkpoint {
        run_id: String,
        state_json: String,
        path: PathBuf,
    },
}

/// Outcome from executing an effect.
#[derive(Debug, Clone)]
pub enum EffectOutcome {
    /// Agent completed with output.
    AgentDone {
        agent_id: String,
        output: String,
        tokens_used: u64,
        cost_usd: f64,
        files_changed: Vec<String>,
    },
    /// Gates completed.
    GatesDone { report: GateReport },
    /// Commit created.
    CommitDone { hash: String, message: String },
    /// Checkpoint saved.
    CheckpointDone { path: String },
    /// Effect failed.
    Failed { error: String },
}

/// Execute a side-effect (spawn agent, run gates, commit, checkpoint).
///
/// The state machine decides WHAT to do; the EffectExecutor decides HOW.
#[async_trait]
pub trait EffectExecutor: Send + Sync {
    /// Execute the given effect, returning the outcome.
    async fn execute(&self, effect: Effect) -> Result<EffectOutcome>;
}

// -- AffectPolicy --

/// Behavioral state of the affect engine.
///
/// Mirrors `roko_core::BehavioralState` from `affect.rs` but is re-exported
/// here for self-contained trait signatures. Use the canonical enum from
/// `roko_core::BehavioralState` — do NOT duplicate the definition.
/// (The type is already `pub` in roko-core via `affect.rs`.)
/// Affect context snapshot provided before dispatching a task.
#[derive(Debug, Clone)]
pub struct AffectContext {
    /// Current behavioral state classification.
    pub behavioral_state: crate::BehavioralState,
    /// Current PAD vector: [Pleasure, Arousal, Dominance], each in [-1.0, 1.0].
    pub pad: [f32; 3],
    /// Human-readable emotional label, if available.
    pub emotional_tag: Option<String>,
}

/// Modulation parameters applied to dispatch configuration.
///
/// The affect policy fills these in; the effect driver applies them.
#[derive(Debug, Clone)]
pub struct DispatchModulation {
    /// Tier bias: -1.0 (prefer cheapest model) to +1.0 (prefer most capable model).
    pub tier_bias: f32,
    /// Multiplier on the default turn limit. 1.0 = no change.
    pub turn_limit_factor: f32,
    /// Exploration rate in [0.0, 1.0]. Higher = more exploratory routing.
    pub exploration_rate: f32,
}

impl Default for DispatchModulation {
    fn default() -> Self {
        Self {
            tier_bias: 0.0,
            turn_limit_factor: 1.0,
            exploration_rate: 0.0,
        }
    }
}

/// Policy trait for behavioral affect modulation in workflow execution.
///
/// The canonical implementation is `DaimonPolicy` in `roko-daimon`.
/// When affect is disabled, use `NoOpAffectPolicy` which returns neutral defaults.
#[async_trait]
pub trait AffectPolicy: Send + Sync {
    /// Called before dispatching a task. Returns an affect context snapshot.
    fn pre_dispatch(&self, task_id: &str, role: &str) -> AffectContext;

    /// Called after a task completes (success or failure).
    fn on_task_outcome(&mut self, task_id: &str, succeeded: bool, tokens_used: u64, cost_usd: f64);

    /// Called after a gate verdict.
    fn on_gate_result(&mut self, gate_name: &str, passed: bool, rung: u8, confidence: f64);

    /// Modulate dispatch parameters based on current affect state.
    fn modulate_dispatch(&self, role: &str, params: &mut DispatchModulation);

    /// Get the current behavioral state for logging/display.
    fn behavioral_state(&self) -> crate::BehavioralState;

    /// Persist affect state to disk.
    async fn persist(&self) -> Result<()>;
}

/// No-op implementation of `AffectPolicy` for when affect modulation is disabled.
///
/// All methods return neutral defaults. No state is tracked or persisted.
pub struct NoOpAffectPolicy;

#[async_trait]
impl AffectPolicy for NoOpAffectPolicy {
    fn pre_dispatch(&self, _task_id: &str, _role: &str) -> AffectContext {
        AffectContext {
            behavioral_state: crate::BehavioralState::Engaged,
            pad: [0.0, 0.0, 0.0],
            emotional_tag: None,
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

    fn on_gate_result(&mut self, _gate_name: &str, _passed: bool, _rung: u8, _confidence: f64) {}

    fn modulate_dispatch(&self, _role: &str, _params: &mut DispatchModulation) {}

    fn behavioral_state(&self) -> crate::BehavioralState {
        crate::BehavioralState::Engaged
    }

    async fn persist(&self) -> Result<()> {
        Ok(())
    }
}
