//! Agent execution helper — spawn Claude CLI for interactive tasks.
//!
//! Used by `roko prd`, `roko research`, and `roko plan generate` to invoke
//! an agent that can read/write files. Reads model config from `roko.toml`.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context as _, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Options for agent execution.
pub struct AgentExecOpts<'a> {
    /// The prompt to send to the agent.
    pub prompt: &'a str,
    /// Working directory for the agent.
    pub workdir: &'a Path,
    /// Model to use (e.g. "claude-sonnet-4-6"). If None, uses CLI default.
    pub model: Option<&'a str>,
    /// Additional system prompt to append.
    pub system_prompt: Option<&'a str>,
    /// Extra env vars for the child process (gateway config, etc).
    pub env_vars: &'a [(String, String)],
}

/// Spawn `claude` CLI with the given prompt, streaming output to stdout.
///
/// Returns the exit code. The agent has full tool access (Read, Write, Bash, Edit)
/// via `--dangerously-skip-permissions`.
pub async fn run_agent(opts: AgentExecOpts<'_>) -> Result<i32> {
    let mut cmd = Command::new("claude");
    cmd.arg("-p").arg(opts.prompt);
    cmd.arg("--dangerously-skip-permissions");

    if let Some(model) = opts.model {
        cmd.arg("--model").arg(model);
    }

    if let Some(sys) = opts.system_prompt {
        cmd.arg("--append-system-prompt").arg(sys);
    }

    // Inherit gateway env vars from current process
    if let Ok(base_url) = std::env::var("ANTHROPIC_BASE_URL") {
        cmd.env("ANTHROPIC_BASE_URL", base_url);
    }
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        cmd.env("ANTHROPIC_API_KEY", api_key);
    }
    // Plus any extras from config
    for (k, v) in opts.env_vars {
        cmd.env(k, v);
    }

    cmd.current_dir(opts.workdir);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::inherit());

    let mut child = cmd.spawn().context("spawn claude CLI — is `claude` in PATH?")?;

    // Stream stdout to terminal
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Some(line) = lines.next_line().await? {
            println!("{line}");
        }
    }

    let status = child.wait().await.context("wait for claude")?;
    Ok(status.code().unwrap_or(1))
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
/// Returns them as key-value pairs to pass to child processes (avoids unsafe set_var).
pub fn load_gateway_env(workdir: &Path) -> GatewayEnv {
    let mut vars = Vec::new();
    let config_path = workdir.join("roko.toml");
    if !config_path.exists() {
        return GatewayEnv { vars };
    }
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return GatewayEnv { vars },
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
