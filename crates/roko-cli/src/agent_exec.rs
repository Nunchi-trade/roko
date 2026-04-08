//! Agent execution helper — drive the Claude CLI through the real runtime adapter.
//!
//! Used by `roko prd`, `roko research`, and `roko plan generate` to invoke
//! an agent that can read/write files while preserving Roko's Claude wiring
//! (system prompt, settings hooks, MCP discovery, resume, PID tracking, and
//! stderr filtering).

use std::path::Path;

use anyhow::Result;
use roko_agent::{Agent, ClaudeCliAgent};
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

/// Drive `claude` with the given prompt and print the final text output.
///
/// Returns the exit code. The Claude CLI adapter handles system prompt wiring,
/// settings hooks, MCP discovery, resume session threading, and stderr
/// filtering.
pub async fn run_agent(opts: AgentExecOpts<'_>) -> Result<i32> {
    let model = opts.model.unwrap_or("claude-opus-4-6");
    let mut agent = ClaudeCliAgent::new("claude", opts.workdir, model)
        .with_dangerously_skip_permissions(true)
        .with_effort(opts.effort.unwrap_or("medium"));
    if let Some(system_prompt) = opts.system_prompt {
        agent = agent.with_system_prompt(system_prompt);
    }
    if let Some(session_id) = opts.resume_session {
        agent = agent.with_optional_resume(Some(session_id.to_string()));
    }
    for (key, value) in opts.env_vars {
        agent = agent.with_env_var(key, value);
    }

    let prompt = Signal::builder(Kind::Prompt)
        .body(Body::text(opts.prompt))
        .build();
    let result = agent.run(&prompt, &Context::now()).await;

    let rendered = result.output.body.as_text().unwrap_or("");
    if !rendered.is_empty() {
        print!("{rendered}");
    }

    Ok(i32::from(!result.success))
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
