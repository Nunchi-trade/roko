//! Serializable output types for dry-run preview.
//!
//! This module provides the data structures emitted when `--dry-run` is passed
//! to `roko run`. A dry-run resolves config, model, phases, and gates without
//! dispatching to any LLM or executing any gate, then prints the preview and exits.

use serde::{Deserialize, Serialize};

/// Serializable gate descriptor in a dry-run preview.
///
/// Contains minimal metadata about a gate as it would be executed,
/// without actually running it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunGate {
    /// Human-readable name of the gate (e.g., "compile", "test").
    pub name: String,
    /// Gate kind/type (e.g., "compile_gate", "test_gate").
    pub kind: String,
    /// Working directory where the gate would run.
    pub workdir: String,
}

/// Serializable preview of a dry-run workflow execution.
///
/// This struct captures everything that would happen during a real run —
/// config loading, model selection, prompt assembly, gate pipeline setup —
/// without any side effects (no LLM dispatch, no gate execution, no filesystem mutations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunPreview {
    /// Workflow engine variant (e.g., "graph", "runner-v2").
    pub engine: String,
    /// Selected LLM model name (e.g., "claude-3-5-sonnet").
    pub model: String,
    /// Model provider name (e.g., "anthropic", "openrouter").
    pub provider: String,
    /// Workflow template/pipeline name (e.g., "express", "standard", "full").
    pub pipeline_template: String,
    /// List of phase names that would be executed (e.g., ["compose", "agent", "gate"]).
    pub phases: Vec<String>,
    /// Gate pipeline descriptors that would be invoked.
    pub gates: Vec<DryRunGate>,
    /// Estimated token count for the system prompt.
    pub estimated_prompt_tokens: u64,
    /// Preview of the system prompt (truncated to first 500 chars).
    pub system_prompt_preview: String,
    /// SHA256 hash of the resolved configuration.
    pub config_hash: String,
    /// Whether all required secrets are configured and accessible.
    pub secrets_ok: bool,
    /// List of non-blocking warnings discovered during resolution.
    pub warnings: Vec<String>,
}
