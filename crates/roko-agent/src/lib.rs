//! Agent backends — async executors that take a prompt and emit output signals.
//!
//! # Why a dedicated trait?
//!
//! The six core Roko traits (Substrate, Scorer, Gate, Router, Composer, Policy)
//! capture composition, verification, and decision-making. An **Agent** is
//! different: it's an async executor with potentially long-running side
//! effects (subprocess management, file edits, LLM API calls).
//!
//! Rather than contort an agent into a Gate or Composer, Roko adds the
//! [`Agent`] trait as a capability extension. The core stays clean; agent
//! impls live in this crate.
//!
//! # Implementations
//!
//! - [`MockAgent`] — deterministic, for tests
//! - [`ExecAgent`] — spawns an external CLI, pipes prompt to stdin, captures stdout
//! - [`ClaudeCliAgent`] — Claude CLI adapter with tool allowlists and resume support
//! - [`OllamaAgent`] — direct Ollama `/api/chat` adapter
//! - [`OllamaLlmBackend`] — Ollama tool-loop backend

#![allow(
    dead_code,
    missing_docs,
    unused_assignments,
    unused_variables,
    clippy::borrowed_box,
    clippy::double_must_use,
    clippy::expect_used,
    clippy::large_enum_variant,
    clippy::module_name_repetitions,
    clippy::never_loop,
    clippy::nursery,
    clippy::pedantic,
    clippy::ptr_arg,
    clippy::too_many_arguments,
    clippy::unnecessary_lazy_evaluations,
    clippy::unnecessary_sort_by,
    clippy::unwrap_used
)]

pub mod agent;
/// Short-lived content-addressed response cache for identical backend requests.
pub mod cache;
pub mod chat_types;
pub mod claude_agent;
pub mod claude_cli_agent;
pub mod codex_agent;
pub mod composition;
pub mod cursor_agent;
pub mod dispatcher;
pub mod exec;
pub mod format;
pub mod gemini;
pub mod http;
pub mod introspection;
pub mod lifecycle;
pub mod mcp;
pub mod metamorphosis;
pub mod mock;
pub mod multi_pool;
pub mod nl_to_format;
pub mod ollama;
pub mod openai_agent;
pub mod openai_compat_backend;
pub mod perplexity;
pub mod pointer;
pub mod pool;
pub mod process;
pub mod provider;
pub mod rate_limit;
pub mod retry;
pub mod safety;
pub mod streaming;
pub mod task_runner;
pub mod testutil;
pub mod tool_loop;
pub mod translate;
pub mod usage;

/// Deprecated compatibility shim for the former flat `ollama_agent` module.
#[deprecated(note = "use crate::ollama::agent::OllamaAgent or crate::OllamaAgent instead")]
pub mod ollama_agent {
    pub use crate::ollama::agent::OllamaAgent;
}

/// Deprecated compatibility shim for the former flat `ollama_backend` module.
#[deprecated(
    note = "use crate::ollama::agent::OllamaLlmBackend or crate::OllamaLlmBackend instead"
)]
pub mod ollama_backend {
    pub use crate::ollama::agent::OllamaLlmBackend;
}

pub use agent::{Agent, AgentResult};
pub use chat_types::{ChatRequest, RequestOptions, ResponseFormat, ToolChoice};
pub use claude_cli_agent::ClaudeCliAgent;
pub use composition::{AgentComposition, CompositeAgent, MergeStrategy, SkillSelector};
pub use exec::ExecAgent;
pub use gemini::{
    GeminiCompatAgent, GeminiEmbedAgent, GeminiMetadata, GeminiNativeAgent, GenerateContentRequest,
    GenerateContentResponse, GroundingMetadata,
};
pub use introspection::{AgentIdentity, Intervention, MetacognitiveMonitor, Turn};
pub use lifecycle::*;
pub use metamorphosis::{MorphError, MorphableAgent, RoleProfile};
pub use mock::MockAgent;
pub use multi_pool::{KillReport, MultiAgentPool, WarmEntry};
pub use ollama::agent::{OllamaAgent, OllamaLlmBackend};
pub use openai_compat_backend::OpenAiCompatLlmBackend;
pub use perplexity::{
    Annotation, PerplexityChatAgent, PerplexityDeepResearchAgent, PerplexityEmbedAgent,
    PerplexityMetadata, PerplexitySearchClient, SearchOptions, SearchResult,
};
pub use pool::{AgentInstanceId, AgentPool, AgentTask, InstanceStatus, TaskOutcome};
pub use provider::{
    ProviderAdapter, adapter_for_kind, create_agent_for_model, current_safety_layer,
    with_scoped_safety_layer,
};
pub use rate_limit::ProviderRateLimiter;
pub use safety::{
    AgentWarrant, Capability, CapabilityError, DataSink, HookDecision, SafetyAuditRecord,
    SafetyHook, SafetyLayer, SafetyViolation, TaintLabel, TaintedString, ViolationSeverity,
    ViolationType, check_capability, delegate,
};
pub use streaming::{StreamAccumulator, StreamChunk};
pub use task_runner::{
    AgentEvent, Anomaly, AnomalyDetector, BudgetAction, BudgetGuardrail, ConductorAction,
    ConductorBandit, CostTable, EventBus, ModelPricing, TaskResult, TaskRunner, TaskRunnerError,
};
pub use tool_loop::ToolLoopAgent;
pub use usage::Usage;
