//! Agent process spawning and stream-JSON parsing.
//!
//! Spawns `claude` as a child process with `--output-format stream-json`,
//! parses each stdout line into [`AgentEvent`]s, and sends them through
//! a tokio mpsc channel.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};

use roko_agent::claude_cli_agent::build_settings_json;
use roko_agent::process::{kill_tree, set_process_group};

use super::types::{
    AgentEvent, ClaudeContentBlock, ClaudeStreamEvent, RunConfig,
};

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
            max_turns: 50,
            program: config.claude_program.clone(),
            dangerously_skip_permissions: config.dangerously_skip_permissions,
            mcp_config: config.mcp_config.clone(),
            resume_session: config.resume_session.clone(),
            agent_id,
        }
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
}

/// Parse a single line of `--output-format stream-json` into an `AgentEvent`.
///
/// Returns `None` for empty lines or unparseable content.
pub fn parse_stream_line(line: &str) -> Option<AgentEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let event: ClaudeStreamEvent = match serde_json::from_str(line) {
        Ok(e) => e,
        Err(e) => {
            debug!(line_len = line.len(), err = %e, "ignoring unparseable stream line");
            return None;
        }
    };

    match event {
        ClaudeStreamEvent::System(sys) => Some(AgentEvent::SystemInit {
            session_id: sys.session_id,
            model: sys.model,
        }),

        ClaudeStreamEvent::Assistant(asst) => {
            // An assistant event can have multiple content blocks and usage.
            // We emit one event per content block, plus a TokenUsage if present.
            // For simplicity in the channel, we flatten into the most important:
            // - First text block → MessageDelta
            // - First tool_use block → ToolCall
            // - Usage → TokenUsage
            //
            // The event handler accumulates, so emitting the first is fine.
            let mut result = None;

            for block in &asst.message.content {
                match block {
                    ClaudeContentBlock::Text { text } => {
                        if result.is_none() {
                            result = Some(AgentEvent::MessageDelta {
                                text: text.clone(),
                            });
                        }
                    }
                    ClaudeContentBlock::ToolUse { id, name, .. } => {
                        if result.is_none() {
                            result = Some(AgentEvent::ToolCall {
                                id: id.clone(),
                                name: name.clone(),
                            });
                        }
                    }
                }
            }

            // If we got usage but no content, emit TokenUsage.
            if let Some(usage) = &asst.message.usage {
                if result.is_none() {
                    result = Some(AgentEvent::TokenUsage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        cache_read_tokens: usage.cache_read_input_tokens,
                        cache_write_tokens: usage.cache_creation_input_tokens,
                    });
                }
            }

            result
        }

        ClaudeStreamEvent::Tool(tool) => {
            let output = if tool.content.len() > 4096 {
                format!("{}… [truncated]", &tool.content[..4096])
            } else {
                tool.content
            };
            Some(AgentEvent::ToolOutput {
                id: tool.tool_use_id,
                output,
            })
        }

        ClaudeStreamEvent::Result(res) => Some(AgentEvent::TurnCompleted {
            session_id: Some(res.session_id).filter(|s| !s.is_empty()),
            total_cost_usd: res.total_cost_usd,
            num_turns: res.num_turns,
            is_error: res.is_error,
        }),
    }
}

/// Spawn a claude agent process and stream its output through the channel.
pub async fn spawn_agent(
    config: &AgentSpawnConfig,
    event_tx: mpsc::Sender<AgentEvent>,
) -> Result<AgentHandle> {
    let settings_json = build_settings_json();

    let mut cmd = Command::new(&config.program);
    cmd.current_dir(&config.workdir);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    // Core flags.
    cmd.args(["--print", "--output-format", "stream-json"]);
    cmd.args(["--verbose"]);
    cmd.args(["--model", &config.model]);
    cmd.args(["--max-turns", &config.max_turns.to_string()]);
    cmd.args(["--settings", &settings_json]);

    if config.dangerously_skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
    }

    if !config.system_prompt.is_empty() {
        cmd.args(["--append-system-prompt", &config.system_prompt]);
    }

    if let Some(ref mcp) = config.mcp_config {
        cmd.args(["--mcp-config", &mcp.to_string_lossy()]);
    }

    if let Some(ref session) = config.resume_session {
        cmd.args(["--resume", session]);
    }

    // Environment.
    cmd.env("CARGO_INCREMENTAL", "0");
    cmd.env("CARGO_BUILD_JOBS", "2");

    // Process group isolation.
    set_process_group(&mut cmd);

    let mut child = cmd.spawn().context("spawning claude CLI")?;
    let pid = child
        .id()
        .context("agent process exited before PID could be read")?;

    // Write prompt to stdin, then close it.
    if let Some(mut stdin) = child.stdin.take() {
        let prompt = config.prompt.clone();
        tokio::spawn(async move {
            if let Err(e) = stdin.write_all(prompt.as_bytes()).await {
                error!(err = %e, "writing prompt to agent stdin");
            }
            drop(stdin);
        });
    }

    // Spawn reader task for stdout.
    let stdout = child
        .stdout
        .take()
        .context("agent stdout not captured")?;

    let agent_id = config.agent_id.clone();
    let reader_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(event) = parse_stream_line(&line) {
                if event_tx.send(event).await.is_err() {
                    debug!(agent_id = %agent_id, "event channel closed, stopping reader");
                    break;
                }
            }
        }

        // Send Exited — we don't know the exit code yet, the event loop
        // will reap it from the child handle.
        let _ = event_tx.send(AgentEvent::Exited { exit_code: None }).await;
    });

    // Spawn stderr reader (log and discard).
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    debug!(stderr = %line, "agent stderr");
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

/// Build the task prompt from a task definition.
pub fn build_task_prompt(
    task: &crate::task_parser::TaskDef,
    plan_id: &str,
    workdir: &Path,
) -> String {
    task.build_prompt(plan_id, workdir)
}

/// Build a minimal system prompt for a task.
///
/// TODO: Replace with `RoleSystemPromptSpec` 9-layer builder (Phase 5 R028-R029).
pub fn build_minimal_system_prompt(
    task: &crate::task_parser::TaskDef,
    plan_id: &str,
) -> String {
    let role = task.role.as_deref().unwrap_or("implementer");
    let mut prompt = format!(
        "You are a {role} agent working on plan `{plan_id}`, task `{}`.\n\n",
        task.id
    );

    prompt.push_str("## Constraints\n");
    prompt.push_str("- Make minimal, targeted changes.\n");
    prompt.push_str("- Do not modify files outside the task scope.\n");
    prompt.push_str("- Ensure `cargo check` passes before finishing.\n");

    if !task.acceptance.is_empty() {
        prompt.push_str("\n## Acceptance Criteria\n");
        for criterion in &task.acceptance {
            prompt.push_str(&format!("- {criterion}\n"));
        }
    }

    if !task.verify.is_empty() {
        prompt.push_str("\n## Verification\nAfter implementation, these checks will run:\n");
        for step in &task.verify {
            prompt.push_str(&format!("- `{}` ({})\n", step.command, step.phase));
        }
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_system_event() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-sonnet-4-6","tools":[]}"#;
        let event = parse_stream_line(line).unwrap();
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
        let event = parse_stream_line(line).unwrap();
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
        let event = parse_stream_line(line).unwrap();
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
        let event = parse_stream_line(line).unwrap();
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
        let event = parse_stream_line(line).unwrap();
        match event {
            AgentEvent::TurnCompleted { session_id, total_cost_usd, num_turns, is_error } => {
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
        assert!(parse_stream_line("").is_none());
        assert!(parse_stream_line("   ").is_none());
    }

    #[test]
    fn parse_malformed_json() {
        assert!(parse_stream_line("{not json}").is_none());
    }

    #[test]
    fn tool_output_truncation() {
        let long_content = "x".repeat(5000);
        let line = format!(
            r#"{{"type":"tool","subtype":"result","tool_name":"Bash","tool_use_id":"tu_3","content":"{long_content}"}}"#
        );
        let event = parse_stream_line(&line).unwrap();
        match event {
            AgentEvent::ToolOutput { output, .. } => {
                assert!(output.len() < 5000);
                assert!(output.ends_with("… [truncated]"));
            }
            _ => panic!("expected ToolOutput"),
        }
    }
}
