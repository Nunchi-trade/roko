//! Agent process spawning and stream-JSON parsing.
//!
//! Spawns the configured CLI provider, parses stdout lines into
//! [`AgentEvent`]s, and sends them through a tokio mpsc channel.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};

use roko_agent::process::{kill_tree, set_process_group};
use roko_core::defaults::DEFAULT_AGENT_TURN_LIMIT;

use crate::dispatch_v2::{CliDispatchProvider, CliDispatchRequest, CliProviderConfig};

use super::types::{AgentEvent, RunConfig};

/// Configuration for spawning a single agent.
#[derive(Debug, Clone)]
pub struct AgentSpawnConfig {
    /// The prompt to send to the agent.
    pub prompt: String,
    /// System prompt appended via --append-system-prompt.
    pub system_prompt: String,
    /// Model to use.
    pub model: String,
    /// Working directory for the agent.
    pub workdir: PathBuf,
    /// Maximum turns the agent can take.
    pub max_turns: u32,
    /// Optional reasoning effort hint for providers that support it.
    pub effort: Option<String>,
    /// Claude CLI binary path.
    pub program: PathBuf,
    /// Whether to skip permission checks.
    pub dangerously_skip_permissions: bool,
    /// Optional MCP config path.
    pub mcp_config: Option<PathBuf>,
    /// Optional session ID to resume.
    pub resume_session: Option<String>,
    /// Agent identifier for logging.
    pub agent_id: String,
    /// Materialized CLI provider selected by provider/model resolution.
    pub cli_provider: Option<CliProviderConfig>,
}

impl AgentSpawnConfig {
    /// Create a spawn config from a `RunConfig` and task-specific details.
    pub fn from_run_config(
        config: &RunConfig,
        prompt: String,
        system_prompt: String,
        model: String,
        agent_id: String,
    ) -> Self {
        Self {
            prompt,
            system_prompt,
            model,
            workdir: config.workdir.clone(),
            max_turns: DEFAULT_AGENT_TURN_LIMIT,
            effort: None,
            program: config.claude_program.clone(),
            dangerously_skip_permissions: config.dangerously_skip_permissions,
            mcp_config: config.mcp_config.clone(),
            resume_session: config.resume_session.clone(),
            agent_id,
            cli_provider: None,
        }
    }

    /// Attach a resolved CLI provider.
    #[must_use]
    pub fn with_cli_provider(mut self, provider: CliProviderConfig) -> Self {
        self.cli_provider = Some(provider);
        self
    }
}

/// Handle to a running agent process.
pub struct AgentHandle {
    /// PID of the agent process.
    pub pid: u32,
    /// The child process.
    child: Child,
    /// Task reading stdout lines.
    reader_task: JoinHandle<()>,
}

impl AgentHandle {
    /// Kill the agent and all descendants. Sends SIGTERM to the process group,
    /// waits for `grace`, then SIGKILL.
    pub async fn kill(mut self, grace: Duration) {
        // Cancel the reader task.
        self.reader_task.abort();

        // Use roko-agent's kill_tree which handles process groups properly.
        if let Err(e) = kill_tree(&mut self.child, grace).await {
            warn!(pid = self.pid, err = %e, "error killing agent");
        }
    }

    /// Wait for the process to exit and return its exit code.
    pub async fn wait(self) -> Option<i32> {
        let mut child = self.child;
        let reader_task = self.reader_task;
        let _ = reader_task.await;
        child.wait().await.ok().and_then(|status| status.code())
    }
}

/// Parse a single line of `--output-format stream-json` into `AgentEvent`(s).
///
/// Returns an empty vec for empty lines or unparseable content.
/// May return multiple events (e.g., a MessageDelta AND a TokenUsage from the
/// same assistant message).
pub fn parse_stream_line(line: &str) -> Vec<AgentEvent> {
    roko_agent::provider::claude_cli::stream::parse_stream_line(line)
}

/// Spawn a configured CLI agent process and stream its output through the channel.
pub async fn spawn_agent(
    config: &AgentSpawnConfig,
    event_tx: mpsc::Sender<AgentEvent>,
) -> Result<AgentHandle> {
    let provider = config
        .cli_provider
        .clone()
        .unwrap_or_else(|| CliProviderConfig::from_legacy_runner_program(config.program.clone()));
    let invocation = provider.build_invocation(&CliDispatchRequest {
        prompt: config.prompt.clone(),
        system_prompt: config.system_prompt.clone(),
        model: config.model.clone(),
        workdir: config.workdir.clone(),
        max_turns: config.max_turns,
        effort: config.effort.clone(),
        dangerously_skip_permissions: config.dangerously_skip_permissions,
        mcp_config: config.mcp_config.clone(),
        resume_session: config.resume_session.clone(),
        env: Vec::new(),
        agent_id: config.agent_id.clone(),
    })?;

    let mut cmd = Command::new(&invocation.program);
    cmd.current_dir(&invocation.workdir);
    cmd.args(&invocation.args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    // Environment.
    for (key, value) in &invocation.env {
        cmd.env(key, value);
    }
    // Unset all Claude Code env vars to prevent "nested session" detection
    // when spawning agents from within a Claude Code session.
    cmd.env_remove("CLAUDECODE");
    cmd.env_remove("CLAUDE_CODE_ENTRYPOINT");
    cmd.env_remove("CLAUDE_CODE_SSE_PORT");
    cmd.env_remove("CLAUDE_CODE_MAX_OUTPUT_TOKENS");
    cmd.env_remove("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS");
    cmd.env_remove("CLAUDE_CODE_EFFORT_LEVEL");

    // Process group isolation.
    set_process_group(&mut cmd);

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning {} CLI", invocation.event_provider))?;
    let pid = child
        .id()
        .context("agent process exited before PID could be read")?;

    let _ = event_tx
        .send(AgentEvent::Started {
            agent_id: config.agent_id.clone(),
            provider: invocation.event_provider.clone(),
            model: invocation.model.clone(),
            pid: Some(pid),
        })
        .await;

    // Write prompt to stdin synchronously, then close it (matching mori's pattern).
    // Must complete BEFORE spawning reader tasks to avoid race conditions.
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(invocation.stdin.as_bytes()).await {
            error!(err = %e, "writing prompt to agent stdin");
        }
        drop(stdin); // EOF signals end of input to Claude CLI
    }

    // Spawn reader task for stdout.
    let stdout = child.stdout.take().context("agent stdout not captured")?;

    let agent_id = config.agent_id.clone();
    let stdout_tx = event_tx.clone();
    let reader_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            for event in parse_stream_line(&line) {
                if stdout_tx.send(event).await.is_err() {
                    debug!(agent_id = %agent_id, "event channel closed, stopping reader");
                    break;
                }
            }
        }

        // Send Exited — we don't know the exit code yet, the event loop
        // will reap it from the child handle.
        let _ = stdout_tx.send(AgentEvent::Exited { exit_code: None }).await;
    });

    // Spawn stderr reader and surface it as durable agent events.
    if let Some(stderr) = child.stderr.take() {
        let stderr_tx = event_tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    debug!(stderr = %line, "agent stderr");
                    let _ = stderr_tx
                        .send(AgentEvent::Error {
                            message: line.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    // Register PID for orphan cleanup.
    roko_agent::process::register_spawned_pid(pid);

    Ok(AgentHandle {
        pid,
        child,
        reader_task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_system_event() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-sonnet-4-6","tools":[]}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentEvent::SystemInit { session_id, model } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(model, "claude-sonnet-4-6");
            }
            _ => panic!("expected SystemInit"),
        }
    }

    #[test]
    fn parse_assistant_text() {
        let line = r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"hello world"}],"usage":null}}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentEvent::MessageDelta { text } => {
                assert_eq!(text, "hello world");
            }
            _ => panic!("expected MessageDelta"),
        }
    }

    #[test]
    fn parse_assistant_tool_use() {
        let line = r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"tool_use","id":"tu_1","name":"Read","input":{"path":"foo"}}],"usage":null}}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentEvent::ToolCall { id, name } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "Read");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn parse_tool_event() {
        let line = r#"{"type":"tool","subtype":"result","tool_name":"Bash","tool_use_id":"tu_2","content":"output here"}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentEvent::ToolOutput { id, output } => {
                assert_eq!(id, "tu_2");
                assert_eq!(output, "output here");
            }
            _ => panic!("expected ToolOutput"),
        }
    }

    #[test]
    fn parse_result_event() {
        let line = r#"{"type":"result","session_id":"sess-1","total_cost_usd":0.05,"num_turns":3,"is_error":false}"#;
        let event = parse_stream_line(line).into_iter().next().unwrap();
        match event {
            AgentEvent::TurnCompleted {
                session_id,
                total_cost_usd,
                num_turns,
                is_error,
            } => {
                assert_eq!(session_id.unwrap(), "sess-1");
                assert!((total_cost_usd.unwrap() - 0.05).abs() < f64::EPSILON);
                assert_eq!(num_turns.unwrap(), 3);
                assert!(!is_error);
            }
            _ => panic!("expected TurnCompleted"),
        }
    }

    #[test]
    fn parse_empty_line() {
        assert!(parse_stream_line("").is_empty());
        assert!(parse_stream_line("   ").is_empty());
    }

    #[test]
    fn parse_malformed_json() {
        assert!(parse_stream_line("{not json}").is_empty());
    }

    #[test]
    fn tool_output_truncation() {
        let long_content = "x".repeat(5000);
        let line = format!(
            r#"{{"type":"tool","subtype":"result","tool_name":"Bash","tool_use_id":"tu_3","content":"{long_content}"}}"#
        );
        let event = parse_stream_line(&line).into_iter().next().unwrap();
        match event {
            AgentEvent::ToolOutput { output, .. } => {
                assert!(output.len() < 5000);
                assert!(output.ends_with("… [truncated]"));
            }
            _ => panic!("expected ToolOutput"),
        }
    }
}
