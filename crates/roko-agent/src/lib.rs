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
//!
//! Future: `ClaudeAgent`, `CodexAgent`, `CursorAgent`, `OllamaAgent`.

#![allow(clippy::module_name_repetitions)]

pub mod agent;
pub mod claude_agent;
pub mod claude_cli_agent;
pub mod codex_agent;
pub mod cursor_agent;
pub mod dispatcher;
pub mod exec;
pub mod format;
pub mod gemini;
pub mod http;
pub mod mcp;
pub mod mock;
pub mod multi_pool;
pub mod nl_to_format;
pub mod ollama;
pub mod ollama_agent;
pub mod ollama_backend;
pub mod openai_agent;
pub mod perplexity;
pub mod pointer;
pub mod pool;
pub mod process;
pub mod provider;
pub mod safety;
pub mod tool_loop;
pub mod translate;
pub mod usage;

pub use agent::{Agent, AgentResult};
pub use claude_cli_agent::ClaudeCliAgent;
pub use exec::ExecAgent;
pub use mock::MockAgent;
pub use multi_pool::MultiAgentPool;
pub use ollama_backend::OllamaLlmBackend;
pub use perplexity::{
    Annotation, PerplexityChatAgent, PerplexityDeepResearchAgent, PerplexityEmbedAgent,
    PerplexityMetadata, PerplexitySearchClient, SearchOptions, SearchResult,
};
pub use pool::{AgentInstanceId, AgentPool};
pub use provider::{ProviderAdapter, adapter_for_kind, create_agent_for_model};
pub use safety::SafetyLayer;
pub use usage::Usage;
