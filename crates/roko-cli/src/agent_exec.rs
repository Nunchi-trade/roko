//! Agent execution helper — drive the Claude CLI through the real runtime adapter.
//!
//! Used by `roko prd`, `roko research`, and `roko plan generate` to invoke
//! an agent that can read/write files while preserving Roko's Claude wiring
//! (system prompt, settings hooks, MCP discovery, resume, PID tracking, and
//! stderr filtering).

use std::path::Path;

use anyhow::{Context as _, Result};
use roko_agent::provider::{AgentOptions, create_agent_for_model};
use roko_core::agent::ProviderKind;
use roko_core::agent::resolve_model;
use roko_core::{Body, Context, Kind, Signal};

/// Options for agent execution.
pub struct AgentExecOpts<'a> {
    /// The prompt to send to the agent.
    pub prompt: &'a str,
    /// Working directory for the agent.
    pub workdir: &'a Path,
    /// Model to use (e.g. "claude-sonnet-4-6"). If None, uses CLI default.
    pub model: Option<&'a str>,
    /// Reasoning effort label to pass to Claude.
    pub effort: Option<&'a str>,
    /// Additional system prompt to append.
    pub system_prompt: Option<&'a str>,
    /// Claude session id to resume, if any.
    pub resume_session: Option<&'a str>,
    /// Extra env vars for the child process (gateway config, etc).
    pub env_vars: &'a [(String, String)],
}

/// Drive `claude` with the given prompt and return just the exit code.
///
/// Convenience wrapper around [`run_agent_capture`] for callers that
/// don't need the agent's text output.
pub async fn run_agent(opts: AgentExecOpts<'_>) -> Result<i32> {
    run_agent_capture(opts).await.map(|(code, _)| code)
}

/// Drive `claude` with the given prompt and return `(exit_code, output_text)`.
///
/// The Claude CLI adapter handles system prompt wiring, settings hooks,
/// MCP discovery, resume session threading, and stderr filtering.
pub async fn run_agent_capture(opts: AgentExecOpts<'_>) -> Result<(i32, String)> {
    let mut routing_config = roko_core::config::load_config(opts.workdir)
        .with_context(|| format!("load routing config from {}", opts.workdir.display()))?;
    routing_config.apply_process_env();
    let routing_enabled = !routing_config.providers.is_empty() || !routing_config.models.is_empty();
    let model = opts
        .model
        .map(str::to_string)
        .or_else(|| model_from_config(opts.workdir))
        .unwrap_or_else(|| {
            if routing_enabled {
                routing_config.agent.default_model.clone()
            } else {
                "claude-opus-4-6".to_string()
            }
        });
    let resolved = resolve_model(&routing_config, &model);
    let mut extra_args = Vec::new();
    if resolved.provider_kind == ProviderKind::ClaudeCli
        && let Some(session_id) = opts.resume_session
    {
        extra_args.push("--resume".to_string());
        extra_args.push(session_id.to_string());
    }
    let agent = create_agent_for_model(
        &routing_config,
        &model,
        AgentOptions {
            command: routing_config.agent.command.clone(),
            timeout_ms: Some(600_000), // 10 min for plan generation / research tasks
            system_prompt: opts.system_prompt.map(str::to_string),
            cached_content: None,
            tools: None,
            mcp_config: None,
            provider_semaphores: None,
            env: opts.env_vars.to_vec(),
            extra_args,
            effort: Some(opts.effort.unwrap_or("medium").to_string()),
            bare_mode: true,
            dangerously_skip_permissions: true,
            name: format!("{}:{model}", resolved.provider_kind.label()),
        },
    )
    .with_context(|| format!("create agent for model {model}"))?;

    let prompt = Signal::builder(Kind::Prompt)
        .body(Body::text(opts.prompt))
        .build();
    let result = agent.run(&prompt, &Context::now()).await;

    let rendered = result.output.body.as_text().unwrap_or("").to_string();
    if !rendered.is_empty() {
        print!("{rendered}");
    }

    Ok((i32::from(!result.success), rendered))
}

/// Read model from roko.toml config if available.
pub fn model_from_config(workdir: &Path) -> Option<String> {
    let config_path = workdir.join("roko.toml");
    if !config_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&config_path).ok()?;
    // Simple extraction — avoid pulling in full config parsing
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("model") {
            let rest = rest.trim().strip_prefix('=')?;
            let rest = rest.trim().trim_matches('"');
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Gateway env vars extracted from roko.toml agent.env.
pub struct GatewayEnv {
    /// Key-value pairs to set on child processes.
    pub vars: Vec<(String, String)>,
}

/// Load gateway env vars from roko.toml's agent.env entries.
/// Returns them as key-value pairs to pass to child processes (avoids unsafe `set_var`).
pub fn load_gateway_env(workdir: &Path) -> GatewayEnv {
    let mut vars = Vec::new();
    let config_path = workdir.join("roko.toml");
    if !config_path.exists() {
        return GatewayEnv { vars };
    }
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return GatewayEnv { vars };
    };
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.contains("ANTHROPIC_") {
            let inner = line.trim_matches(|c| c == '[' || c == ']');
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() == 2 {
                let key = parts[0].trim().trim_matches('"');
                let val = parts[1].trim().trim_matches('"');
                if !key.is_empty() {
                    vars.push((key.to_string(), val.to_string()));
                }
            }
        }
    }
    GatewayEnv { vars }
}
