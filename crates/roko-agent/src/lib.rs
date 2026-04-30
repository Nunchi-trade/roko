//! Agent backends — async executors that take a prompt and emit output signals.
//!
//! # Why a dedicated trait?
//!
//! The six core Roko traits (Store, Score, Verify, Route, Compose, React)
//! capture composition, verification, and decision-making. An **Agent** is
//! different: it's an async executor with potentially long-running side
//! effects (subprocess management, file edits, LLM API calls).
//!
//! Rather than contort an agent into a Verify or Compose, Roko adds the
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
    clippy::doc_lazy_continuation,
    clippy::double_must_use,
    clippy::expect_used,
    clippy::large_enum_variant,
    clippy::module_name_repetitions,
    clippy::needless_borrow,
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
/// File-backed response cache for demo determinism.
pub mod file_cache;
pub mod format;
pub mod gateway_events;
pub mod gemini;
pub mod http;
pub mod introspection;
pub mod lifecycle;
pub mod mcp;
pub mod metamorphosis;
pub mod mock;
pub mod model_call_service;
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
pub mod runtime_events;
pub mod safety;
pub mod session;
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
pub use http::{shared_http_client, shared_http_client_from, HttpPoster, ReqwestPoster};
pub use gateway_events::{AggregateStats, GatewayEvent, GatewayEventWriter, GatewayProjection};
pub use gemini::{
    GeminiCompatAgent, GeminiEmbedAgent, GeminiMetadata, GeminiNativeAgent, GenerateContentRequest,
    GenerateContentResponse, GroundingMetadata,
};
pub use introspection::{AgentIdentity, Intervention, MetacognitiveMonitor, Turn};
pub use lifecycle::*;
pub use metamorphosis::{MorphError, MorphableAgent, RoleProfile};
pub use mock::MockAgent;
pub use model_call_service::ModelCallService;
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
pub use roko_core::{
    BUILTIN_ROLE_POLICY_MANIFEST_PATH, BUILTIN_ROLE_POLICY_MANIFEST_TOML,
    MANIFEST_BACKED_BUILTIN_ROLE_IDS, PromptPolicy, RolePolicyManifest,
    RoleProfile as ManifestRoleProfile, ToolCapabilityPolicy,
};
pub use runtime_events::{AgentEventStream, AgentRuntimeEvent};
pub use safety::{
    AgentWarrant, Capability, CapabilityError, DataSink, HookDecision, SafetyAuditRecord,
    SafetyHook, SafetyLayer, SafetyViolation, TaintLabel, TaintedString, ViolationSeverity,
    ViolationType, check_capability, delegate,
};
pub use session::{
    AgentInvocationSession, InvocationState, ResumeValidationError, ReuseScope, WarmReusePolicy,
    WarmReuseRequest, fingerprint_text, validate_resume_request,
};
pub use streaming::{
    ClaudeCliParser, OpenAiSseParser, StreamAccumulator, StreamChunk, StreamJsonParser,
    UnifiedStreamEvent,
};
pub use task_runner::{
    AgentEvent, Anomaly, AnomalyDetector, BudgetAction, BudgetGuardrail, ConductorAction,
    ConductorBandit, CostTable, EventBus, ModelPricing, TaskResult, TaskRunner, TaskRunnerError,
};
pub use tool_loop::{OnTurnCallback, ToolLoopAgent, TurnProgress};
pub use usage::{Usage, UsageObservation, UsageSource};
