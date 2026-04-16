//! Shared helpers for direct command-backed agent creation.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use roko_agent::provider::{AgentOptions, with_safety_layer};
use roko_agent::{Agent, SafetyLayer, create_agent_for_model, with_scoped_safety_layer};
use roko_core::config::schema::RokoConfig;

/// Owned spec for creating a direct agent through provider-backed routing.
pub struct SpawnAgentSpec {
    /// Model slug to resolve.
    pub model: String,
    /// Optional command override for the child agent.
    pub command: Option<String>,
    /// Optional timeout for the child agent.
    pub timeout_ms: Option<u64>,
    /// Optional system prompt override.
    pub system_prompt: Option<String>,
    /// Optional cached content hint.
    pub cached_content: Option<String>,
    /// Optional hosted-backend tool allowlist.
    pub tools: Option<String>,
    /// Optional MCP config path.
    pub mcp_config: Option<PathBuf>,
    /// Optional working directory.
    pub working_dir: Option<PathBuf>,
    /// Child env vars.
    pub env: Vec<(String, String)>,
    /// Extra CLI args.
    pub extra_args: Vec<String>,
    /// Optional effort label.
    pub effort: Option<String>,
    /// Whether to use bare mode.
    pub bare_mode: bool,
    /// Whether to skip permissions prompts.
    pub dangerously_skip_permissions: bool,
    /// Optional logical agent name.
    pub name: String,
}

impl SpawnAgentSpec {
    fn into_agent_options(self) -> AgentOptions {
        AgentOptions {
            command: self.command,
            timeout_ms: self.timeout_ms,
            system_prompt: self.system_prompt,
            cached_content: self.cached_content,
            tools: self.tools,
            mcp_config: self.mcp_config,
            working_dir: self.working_dir,
            provider_semaphores: None,
            env: self.env,
            extra_args: self.extra_args,
            effort: self.effort,
            bare_mode: self.bare_mode,
            dangerously_skip_permissions: self.dangerously_skip_permissions,
            name: self.name,
        }
    }
}

/// Create an agent under the current scoped safety layer.
pub fn spawn_agent_scoped(
    config: &RokoConfig,
    spec: SpawnAgentSpec,
    error_context: impl Into<String>,
) -> Result<Box<dyn Agent>> {
    let model = spec.model.clone();
    let context = error_context.into();
    with_scoped_safety_layer(|| {
        create_agent_for_model(config, &model, spec.into_agent_options())
            .with_context(|| context.clone())
    })
}

/// Create an agent under an explicit safety layer.
pub fn spawn_agent_with_layer(
    config: &RokoConfig,
    safety_layer: Option<SafetyLayer>,
    spec: SpawnAgentSpec,
    error_context: impl Into<String>,
) -> Result<Box<dyn Agent>> {
    let model = spec.model.clone();
    let context = error_context.into();
    with_safety_layer(safety_layer, || {
        create_agent_for_model(config, &model, spec.into_agent_options())
            .with_context(|| context.clone())
    })
}
