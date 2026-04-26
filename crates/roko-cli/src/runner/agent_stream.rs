//! Agent process spawning and stream-JSON parsing.
//!
//! Spawns the configured CLI provider, parses stdout lines into
//! [`AgentEvent`]s, and sends them through a tokio mpsc channel.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use roko_compose::{Complexity, GateFeedback, RoleSystemPromptSpec, TaskContext};
use roko_core::AgentRole;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};

use roko_agent::process::{kill_tree, set_process_group};

use crate::dispatch_v2::{CliDispatchProvider, CliDispatchRequest, CliProviderConfig};

use super::types::{AgentEvent, ClaudeContentBlock, ClaudeStreamEvent, RunConfig};

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
            max_turns: 50,
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
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }

    let event: ClaudeStreamEvent = match serde_json::from_str(line) {
        Ok(e) => e,
        Err(e) => {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                return parse_generic_json_line(&value);
            }
            debug!(line_len = line.len(), err = %e, "ignoring unparseable stream line");
            return Vec::new();
        }
    };

    match event {
        ClaudeStreamEvent::System(sys) => vec![AgentEvent::SystemInit {
            session_id: sys.session_id,
            model: sys.model,
        }],

        ClaudeStreamEvent::Assistant(asst) => {
            // An assistant event can have content blocks AND usage in the same
            // message. Emit ALL of them — content block first, then usage.
            let mut events = Vec::new();

            for block in &asst.message.content {
                match block {
                    ClaudeContentBlock::Text { text } => {
                        events.push(AgentEvent::MessageDelta { text: text.clone() });
                    }
                    ClaudeContentBlock::ToolUse { id, name, .. } => {
                        events.push(AgentEvent::ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                        });
                    }
                }
            }

            // Always emit TokenUsage when present — even alongside content.
            if let Some(usage) = &asst.message.usage {
                events.push(AgentEvent::TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_read_tokens: usage.cache_read_input_tokens,
                    cache_write_tokens: usage.cache_creation_input_tokens,
                });
            }

            events
        }

        ClaudeStreamEvent::Tool(tool) => {
            let output = if tool.content.len() > 4096 {
                format!("{}… [truncated]", &tool.content[..4096])
            } else {
                tool.content
            };
            vec![AgentEvent::ToolOutput {
                id: tool.tool_use_id,
                output,
            }]
        }

        ClaudeStreamEvent::Result(res) => {
            let mut events = vec![AgentEvent::TurnCompleted {
                session_id: Some(res.session_id).filter(|s| !s.is_empty()),
                total_cost_usd: res.total_cost_usd,
                num_turns: res.num_turns,
                is_error: res.is_error,
            }];
            // Result events also carry final usage — capture it.
            if let Some(usage) = &res.usage {
                events.push(AgentEvent::TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_read_tokens: usage.cache_read_input_tokens,
                    cache_write_tokens: usage.cache_creation_input_tokens,
                });
            }
            events
        }
    }
}

fn parse_generic_json_line(value: &serde_json::Value) -> Vec<AgentEvent> {
    let event_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if event_type.contains("error") {
        let message = value
            .get("message")
            .or_else(|| value.get("error"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("agent emitted an error event");
        return vec![AgentEvent::Error {
            message: message.to_string(),
        }];
    }

    if event_type.contains("message") || event_type.contains("output") {
        for key in ["text", "message", "content", "delta"] {
            if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
                return vec![AgentEvent::MessageDelta {
                    text: text.to_string(),
                }];
            }
        }
    }

    Vec::new()
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

    // Write prompt to stdin, then close it.
    if let Some(mut stdin) = child.stdin.take() {
        let prompt_to_send = invocation.stdin.clone();
        tokio::spawn(async move {
            if let Err(e) = stdin.write_all(prompt_to_send.as_bytes()).await {
                error!(err = %e, "writing prompt to agent stdin");
            }
            drop(stdin);
        });
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
pub fn build_minimal_system_prompt(task: &crate::task_parser::TaskDef, plan_id: &str) -> String {
    build_composed_system_prompt(task, plan_id, 16_000)
        .unwrap_or_else(|_| build_legacy_system_prompt(task, plan_id))
}

/// Build the task system prompt through the shared 9-layer role prompt builder.
pub fn build_composed_system_prompt(
    task: &crate::task_parser::TaskDef,
    plan_id: &str,
    context_window_tokens: usize,
) -> Result<String> {
    let role_text = task.role.as_deref().unwrap_or("implementer");
    let role = parse_runner_agent_role(role_text).unwrap_or(AgentRole::Implementer);
    let mut spec = RoleSystemPromptSpec::new(
        role,
        TaskContext::new(task_system_context(task))
            .with_plan_id(plan_id)
            .with_workspace("roko runner v2"),
        "",
    )
    .with_complexity(prompt_complexity(task))
    .with_cache_markers();

    if let Some(conventions) = task_scope_conventions(task, role_text) {
        spec = spec.with_extra_conventions(conventions);
    }
    if let Some(ctx) = &task.context {
        for anti_pattern in &ctx.anti_patterns {
            spec = spec.add_anti_pattern(anti_pattern.clone());
        }
        for prior_failure in &ctx.prior_failures {
            spec =
                spec.add_anti_pattern(format!("Prior failure to avoid repeating: {prior_failure}"));
        }
    }

    Ok(spec.build_with_context_window(context_window_tokens.max(1))?)
}

/// Format previous gate output as bounded, actionable retry context.
pub fn format_gate_feedback_for_prompt(raw_output: &str, rung: u32) -> Option<String> {
    GateFeedback::from_raw(raw_output, rung).map(|feedback| feedback.render_prompt_section())
}

fn build_legacy_system_prompt(task: &crate::task_parser::TaskDef, plan_id: &str) -> String {
    let role = task.role.as_deref().unwrap_or("implementer");
    format!(
        "You are a {role} agent working on plan `{plan_id}`, task `{}`.\n\n## Constraints\n- Make minimal, targeted changes.\n- Do not modify files outside the task scope.\n- Ensure verification passes before finishing.\n",
        task.id
    )
}

fn task_system_context(task: &crate::task_parser::TaskDef) -> String {
    let mut context = format!("Task ID: {}\nTitle: {}", task.id, task.title);
    if let Some(description) = task.description.as_deref().filter(|s| !s.trim().is_empty()) {
        context.push_str("\nDescription: ");
        context.push_str(description.trim());
    }
    if !task.acceptance.is_empty() {
        context.push_str("\n\nAcceptance criteria:");
        for criterion in &task.acceptance {
            context.push_str("\n- ");
            context.push_str(criterion);
        }
    }
    if !task.verify.is_empty() {
        context.push_str("\n\nVerification gates:");
        for step in &task.verify {
            context.push_str("\n- `");
            context.push_str(&step.command);
            context.push_str("` (");
            context.push_str(&step.phase);
            context.push(')');
        }
    }
    context
}

fn task_scope_conventions(task: &crate::task_parser::TaskDef, role_text: &str) -> Option<String> {
    let mut sections = Vec::new();
    if !task.files.is_empty() {
        let mut scope = String::from(
            "Honor the declared write scope strictly. Only create, edit, move, or delete files in this allowlist unless the user explicitly expands it:",
        );
        for file in &task.files {
            scope.push_str("\n- ");
            scope.push_str(file);
        }
        sections.push(scope);
    }
    if let Some(max_loc) = task.max_loc {
        sections.push(format!(
            "Keep the total code delta within roughly {max_loc} lines unless verification requires a tightly scoped follow-up."
        ));
    }
    if parse_runner_agent_role(role_text).is_none() {
        sections.push(format!("Treat the task role hint literally: {role_text}"));
    }
    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

fn prompt_complexity(task: &crate::task_parser::TaskDef) -> Complexity {
    match task.tier.as_str() {
        "mechanical" | "fast" => Complexity::Trivial,
        "integrative" | "architectural" | "complex" | "premium" => Complexity::Complex,
        _ => Complexity::Standard,
    }
}

fn parse_runner_agent_role(role: &str) -> Option<AgentRole> {
    let normalized = role.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    Some(match normalized.as_str() {
        "conductor" => AgentRole::Conductor,
        "strategist" => AgentRole::Strategist,
        "implementer" | "engineer" | "coder" => AgentRole::Implementer,
        "architect" => AgentRole::Architect,
        "researcher" => AgentRole::Researcher,
        "auditor" => AgentRole::Auditor,
        "quick-reviewer" | "quickreviewer" => AgentRole::QuickReviewer,
        "scribe" => AgentRole::Scribe,
        "critic" => AgentRole::Critic,
        "auto-fixer" | "autofixer" => AgentRole::AutoFixer,
        "pattern-extractor" | "patternextractor" => AgentRole::PatternExtractor,
        "snapshot-comparator" | "snapshotcomparator" => AgentRole::SnapshotComparator,
        "full-loop-validator" | "fullloopvalidator" => AgentRole::FullLoopValidator,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_parser::{TaskContext as ParserTaskContext, TaskDef, VerifyStep};

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
    fn composed_system_prompt_contains_expected_layers() {
        let task = sample_task();
        let prompt = build_composed_system_prompt(&task, "plan-a", 16_000).unwrap();

        assert!(prompt.contains("<!-- cache:system -->"));
        assert!(prompt.contains("## Project Conventions"));
        assert!(prompt.contains("## Tool Instructions"));
        assert!(prompt.contains("## Current Task"));
        assert!(prompt.contains("Plan: plan-a"));
        assert!(prompt.contains("Task ID: T1"));
        assert!(prompt.contains("Acceptance criteria"));
        assert!(prompt.contains("Verification gates"));
        assert!(prompt.contains("cargo test -p roko-compose"));
        assert!(prompt.contains("Honor the declared write scope strictly"));
        assert!(prompt.contains("crates/roko-compose/src/prompt.rs"));
        assert!(prompt.contains("## Anti-Patterns"));
        assert!(prompt.contains("Do not rewrite the runner"));
    }

    #[test]
    fn gate_feedback_is_structured_and_bounded() {
        let raw =
            "noise\nerror[E0308]: mismatched types\n --> src/lib.rs:9:1\nwarning: unused import\n";
        let feedback = format_gate_feedback_for_prompt(raw, 2).unwrap();

        assert!(feedback.contains("## Previous Verify Failure"));
        assert!(feedback.contains("Gate rung: 2"));
        assert!(feedback.contains("error[E0308]"));
        assert!(feedback.contains("src/lib.rs:9:1"));
        assert!(feedback.contains("warning: unused import"));
        assert!(!feedback.contains("\nnoise\n"));
    }

    fn sample_task() -> TaskDef {
        TaskDef {
            id: "T1".into(),
            title: "Wire prompt assembly".into(),
            description: Some("Replace the ad hoc prompt path.".into()),
            role: Some("implementer".into()),
            status: "ready".into(),
            tier: "focused".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: Some(80),
            files: vec!["crates/roko-compose/src/prompt.rs".into()],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: Vec::new(),
            depends_on_plan: Vec::new(),
            split_into: None,
            context: Some(ParserTaskContext {
                anti_patterns: vec!["Do not rewrite the runner".into()],
                ..ParserTaskContext::default()
            }),
            verify: vec![VerifyStep {
                phase: "test".into(),
                command: "cargo test -p roko-compose".into(),
                fail_msg: None,
                timeout_ms: 60_000,
            }],
            timeout_secs: 600,
            max_retries: 3,
            acceptance: vec!["Prompt contains task context".into()],
            acceptance_contract: None,
            domain: None,
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
