//! `ClaudeCliAgent` — choose this for the Claude CLI path with Roko's system
//! prompt, tool allowlist, safety settings, and session-aware behavior.
//!
//! This is the runtime-facing adapter for the `claude` executable. It keeps
//! the wire-specific flag construction in one place instead of scattering
//! command-building logic across the CLI entrypoints. Prefer
//! [`ExecAgent`](crate::ExecAgent) only for generic stdin/stdout CLIs where
//! Claude-specific resume and tool-loop wiring are not needed.

use crate::agent::{Agent, AgentResult};
use crate::mcp::find_mcp_config;
use crate::process::{
    GRACE_STDIN_CLOSE_MS, benign_stderr_warn_once, classify_benign_stderr, kill_tree,
    register_spawned_pid, set_process_group, unregister_pid,
};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Engram, Kind, OperatingFrequency, Provenance};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::{Duration, timeout};

/// Build the Claude CLI `--settings` JSON payload with safety hooks.
///
/// The hooks block the destructive commands that should never be launched by
/// a model in this workspace: branch checkout/switch/rename, branch pushes,
/// and common filesystem-destruction shells.
#[must_use]
pub fn build_settings_json() -> String {
    serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Bash",
                "hooks": [
                    {
                        "type": "command",
                        "if": "Bash(git checkout *)",
                        "command": "echo 'BLOCKED: git checkout forbidden in plan worktrees' >&2 && exit 2"
                    },
                    {
                        "type": "command",
                        "if": "Bash(git switch *)",
                        "command": "echo 'BLOCKED: git switch forbidden in plan worktrees' >&2 && exit 2"
                    },
                    {
                        "type": "command",
                        "if": "Bash(git branch -m *)",
                        "command": "echo 'BLOCKED: branch rename forbidden in plan worktrees' >&2 && exit 2"
                    },
                    {
                        "type": "command",
                        "if": "Bash(git push *)",
                        "command": "echo 'BLOCKED: agents must not push — roko handles merges' >&2 && exit 2"
                    },
                    {
                        "type": "command",
                        "if": "Bash(rm -rf *)",
                        "command": "echo 'BLOCKED: destructive file deletion forbidden' >&2 && exit 2"
                    },
                    {
                        "type": "command",
                        "if": "Bash(rm -fr *)",
                        "command": "echo 'BLOCKED: destructive file deletion forbidden' >&2 && exit 2"
                    },
                    {
                        "type": "command",
                        "if": "Bash(rm -r *)",
                        "command": "echo 'BLOCKED: destructive file deletion forbidden' >&2 && exit 2"
                    }
                ]
            }]
        }
    })
    .to_string()
}

/// Agent wrapper around the `claude` CLI.
#[derive(Debug, Clone)]
pub struct ClaudeCliAgent {
    program: PathBuf,
    current_dir: PathBuf,
    model: String,
    effort: String,
    fallback_model: Option<String>,
    bare_mode: bool,
    system_prompt: Option<String>,
    allowed_tools: Option<String>,
    max_turns: Option<u32>,
    settings_json: String,
    extra_args: Vec<String>,
    env: Vec<(String, String)>,
    mcp_config: Option<PathBuf>,
    resume: Option<String>,
    dangerously_skip_permissions: bool,
    timeout_ms: u64,
    name: String,
}

impl ClaudeCliAgent {
    /// Construct a new Claude CLI agent rooted at `current_dir`.
    #[must_use]
    pub fn new(
        program: impl Into<PathBuf>,
        current_dir: impl Into<PathBuf>,
        model: impl Into<String>,
    ) -> Self {
        let model = model.into();
        Self {
            program: program.into(),
            current_dir: current_dir.into(),
            model: model.clone(),
            effort: "medium".to_string(),
            fallback_model: Some("claude-haiku-4-5".to_string()),
            bare_mode: true,
            system_prompt: None,
            allowed_tools: None,
            max_turns: Some(OperatingFrequency::Theta.turn_limit()),
            settings_json: build_settings_json(),
            extra_args: Vec::new(),
            env: Vec::new(),
            mcp_config: None,
            resume: None,
            dangerously_skip_permissions: true,
            timeout_ms: 120_000,
            name: format!("claude-cli:{model}"),
        }
    }

    /// Override the display name used in traces.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Override the per-request timeout in milliseconds.
    #[must_use]
    pub const fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Override the reasoning-effort label passed to Claude.
    #[must_use]
    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = effort.into();
        self
    }

    /// Override the fallback model passed to Claude.
    #[must_use]
    pub fn with_fallback_model(mut self, fallback_model: impl Into<String>) -> Self {
        self.fallback_model = Some(fallback_model.into());
        self
    }

    /// Disable `--bare` if the caller wants the full Claude Code shell.
    #[must_use]
    pub const fn with_bare_mode(mut self, bare_mode: bool) -> Self {
        self.bare_mode = bare_mode;
        self
    }

    /// Attach a system prompt generated by `SystemPromptBuilder`.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Attach a Claude tool allowlist, formatted as `Read,Edit,Bash`.
    #[must_use]
    pub fn with_tools(mut self, tools: impl Into<String>) -> Self {
        self.allowed_tools = Some(tools.into());
        self
    }

    /// Attach a Claude `--allowedTools` allowlist.
    #[must_use]
    pub fn with_allowed_tools(mut self, tools: impl Into<String>) -> Self {
        self.allowed_tools = Some(tools.into());
        self
    }

    /// Set the maximum number of turns Claude may take.
    #[must_use]
    pub const fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = Some(max_turns);
        self
    }

    /// Override the settings JSON passed via `--settings`.
    #[must_use]
    pub fn with_settings_json(mut self, json: impl Into<String>) -> Self {
        self.settings_json = json.into();
        self
    }

    /// Pass through additional CLI args before the canonical Claude flags.
    #[must_use]
    pub fn with_extra_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extra_args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Add a process environment variable.
    #[must_use]
    pub fn with_env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Attach an explicit MCP config path.
    #[must_use]
    pub fn with_mcp_config(mut self, path: impl Into<PathBuf>) -> Self {
        self.mcp_config = Some(path.into());
        self
    }

    /// Resume the given Claude session id.
    #[must_use]
    pub fn with_resume(mut self, session_id: impl Into<String>) -> Self {
        self.resume = Some(session_id.into());
        self
    }

    /// Resume a session id only when present.
    #[must_use]
    pub fn with_optional_resume(mut self, session_id: Option<String>) -> Self {
        self.resume = session_id.filter(|id| !id.trim().is_empty());
        self
    }

    /// Toggle `--dangerously-skip-permissions` for role-gated policy.
    #[must_use]
    pub const fn with_dangerously_skip_permissions(mut self, enabled: bool) -> Self {
        self.dangerously_skip_permissions = enabled;
        self
    }

    fn failure(&self, input: &Engram, reason: &str, started: Instant) -> AgentResult {
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let output = input
            .derive(Kind::AgentOutput, Body::text(reason))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("failed", "true")
            .build();
        AgentResult::fail(output).with_usage(Usage {
            wall_ms,
            ..Default::default()
        })
    }

    fn discovered_mcp_config(&self) -> Option<PathBuf> {
        if let Some(path) = &self.mcp_config {
            return Some(path.clone());
        }
        match find_mcp_config(&self.current_dir) {
            Some(Ok((path, _))) => Some(path),
            Some(Err(err)) => {
                eprintln!("[claude-cli] ignoring invalid MCP config: {err}");
                None
            }
            None => None,
        }
    }

    fn parse_stream_events(stdout: &str) -> Option<Vec<Value>> {
        let mut events = Vec::new();
        for raw in stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let value = serde_json::from_str::<Value>(raw).ok()?;
            events.push(value);
        }
        if events.is_empty() {
            None
        } else {
            Some(events)
        }
    }

    fn prompt_text_from_input(input: &Engram) -> Result<String, String> {
        input.body.as_text().map(str::to_string).or_else(|_| {
            serde_json::to_string(&input.body)
                .map_err(|e| format!("input body not readable as text or json: {e}"))
        })
    }

    fn build_command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.extra_args);
        // NOTE: --bare was removed from Claude CLI; skip it.
        cmd.arg("--print")
            .arg("--verbose")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--model")
            .arg(&self.model)
            .arg("--effort")
            .arg(&self.effort)
            .arg("--settings")
            .arg(&self.settings_json);
        if self.dangerously_skip_permissions {
            cmd.arg("--dangerously-skip-permissions");
        }
        if let Some(max_turns) = self.max_turns {
            cmd.arg("--max-turns").arg(max_turns.to_string());
        }

        if let Some(fallback_model) = &self.fallback_model
            && fallback_model != &self.model
        {
            cmd.arg("--fallback-model").arg(fallback_model);
        }
        if let Some(system_prompt) = &self.system_prompt {
            cmd.arg("--append-system-prompt").arg(system_prompt);
        }
        if let Some(tools) = &self.allowed_tools
            && !tools.is_empty()
        {
            cmd.arg("--tools").arg(tools);
        }
        if let Some(mcp_config) = self.discovered_mcp_config() {
            cmd.arg("--mcp-config").arg(mcp_config);
            cmd.arg("--strict-mcp-config");
        }
        if let Some(resume) = &self.resume {
            cmd.arg("--resume").arg(resume);
        }

        cmd.current_dir(&self.current_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        set_process_group(&mut cmd);

        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        cmd.env("CARGO_INCREMENTAL", "0");
        cmd.env("CARGO_BUILD_JOBS", "2");
        // Prevent "nested session" detection when spawning from within Claude Code.
        cmd.env_remove("CLAUDECODE");
        cmd
    }

    fn output_text(stdout: &str) -> String {
        Self::parse_stream_events(stdout).map_or_else(
            || stdout.trim().to_string(),
            |events| {
                let response = crate::translate::BackendResponse::StreamJson(events);
                let extracted = response.extract_text();
                if extracted.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    extracted
                }
            },
        )
    }

    fn stderr_trace(&self, stderr: &str) -> Vec<Engram> {
        stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter(|line| !self.warn_and_filter_benign(line))
            .map(|line| {
                Engram::builder(Kind::AgentMessage)
                    .body(Body::text(line))
                    .provenance(Provenance::agent(&self.name))
                    .tag("stream", "stderr")
                    .build()
            })
            .collect()
    }

    fn warn_and_filter_benign(&self, line: &str) -> bool {
        if let Some(benign) = classify_benign_stderr(line) {
            if benign_stderr_warn_once(benign.key) {
                eprintln!("[{}] {}", self.name, benign.summary);
            }
            return true;
        }
        false
    }
}

#[async_trait]
impl Agent for ClaudeCliAgent {
    async fn run(&self, input: &Engram, _ctx: &Context) -> AgentResult {
        let started = Instant::now();

        let prompt_text = match Self::prompt_text_from_input(input) {
            Ok(text) => text,
            Err(reason) => return self.failure(input, &reason, started),
        };

        let mut cmd = self.build_command();

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                return self.failure(input, &format!("spawn failed: {e}"), started);
            }
        };
        let pid = child.id();
        if track_pids()
            && let Some(pid) = pid
        {
            register_spawned_pid(pid);
        }
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(prompt_text.as_bytes()).await {
                let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
                if track_pids()
                    && let Some(pid) = pid
                {
                    unregister_pid(pid);
                }
                return self.failure(input, &format!("stdin write failed: {e}"), started);
            }
        }

        eprintln!(
            "[{}] agent started (pid {}, timeout {}s)",
            self.name,
            pid.unwrap_or(0),
            self.timeout_ms / 1000
        );

        // Track activity across stdout and stderr for heartbeat messages.
        let has_activity = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Stream stdout in real time, parsing stream-json events for progress.
        // Accumulate the raw output for final processing by output_text().
        let stdout_name = self.name.clone();
        let stdout_activity = has_activity.clone();
        let stdout_handle = tokio::spawn(async move {
            let Some(pipe) = stdout_pipe else {
                return String::new();
            };
            let reader = BufReader::new(pipe);
            let mut lines = reader.lines();
            let mut collected = String::new();
            let mut last_tool: Option<String> = None;
            let mut text_bytes: usize = 0;
            let mut tool_count: usize = 0;

            while let Ok(Some(line)) = lines.next_line().await {
                collected.push_str(&line);
                collected.push('\n');
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                stdout_activity.store(true, std::sync::atomic::Ordering::Relaxed);

                // Parse stream-json events for progress reporting.
                // Non-JSON output (raw text from other agents) is fine — we
                // just skip the progress parsing.
                if let Ok(event) = serde_json::from_str::<Value>(trimmed) {
                    match event.get("type").and_then(Value::as_str) {
                        Some("assistant") => {
                            // New turn — check for tool_use in content
                            if let Some(content) = event
                                .get("message")
                                .and_then(|m| m.get("content"))
                                .and_then(Value::as_array)
                            {
                                for block in content {
                                    if block.get("type").and_then(Value::as_str) == Some("tool_use")
                                    {
                                        let name = block
                                            .get("name")
                                            .and_then(Value::as_str)
                                            .unwrap_or("unknown");
                                        tool_count += 1;
                                        last_tool = Some(name.to_string());
                                        eprintln!("[{stdout_name}] tool: {name}");
                                    }
                                }
                            }
                        }
                        Some("content_block_start") => {
                            if let Some(block) = event.get("content_block") {
                                match block.get("type").and_then(Value::as_str) {
                                    Some("tool_use") => {
                                        let name = block
                                            .get("name")
                                            .and_then(Value::as_str)
                                            .unwrap_or("unknown");
                                        tool_count += 1;
                                        last_tool = Some(name.to_string());
                                        eprintln!("[{stdout_name}] tool: {name}");
                                    }
                                    Some("text") => {
                                        eprintln!("[{stdout_name}] generating text...");
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Some("content_block_delta") => {
                            if let Some(delta) = event.get("delta") {
                                if let Some(text) = delta.get("text").and_then(Value::as_str) {
                                    text_bytes += text.len();
                                }
                            }
                        }
                        Some("result") => {
                            let summary = if tool_count > 0 {
                                format!("{text_bytes} bytes text, {tool_count} tool calls")
                            } else {
                                format!("{text_bytes} bytes text")
                            };
                            eprintln!("[{stdout_name}] result received ({summary})");
                        }
                        _ => {}
                    }
                }
            }
            collected
        });

        // Stream stderr to the terminal in real time for user feedback,
        // while accumulating lines for the trace.
        let agent_name = self.name.clone();
        let stderr_activity = has_activity.clone();
        let stderr_handle = tokio::spawn(async move {
            let Some(pipe) = stderr_pipe else {
                return String::new();
            };
            let reader = BufReader::new(pipe);
            let mut lines = reader.lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    stderr_activity.store(true, std::sync::atomic::Ordering::Relaxed);
                    eprintln!("[{agent_name}] {line}");
                }
                collected.push_str(&line);
                collected.push('\n');
            }
            collected
        });

        // Heartbeat: print elapsed time every 15s when there's no other
        // output, so the user knows the agent is still running.
        let heartbeat_name = self.name.clone();
        let heartbeat_started = started;
        let heartbeat_activity = has_activity.clone();
        let heartbeat_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                // Only print heartbeat when there's been no recent stdout/stderr activity.
                if !heartbeat_activity.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    let elapsed = heartbeat_started.elapsed().as_secs();
                    eprintln!("[{heartbeat_name}] waiting for response... ({elapsed}s elapsed)");
                }
            }
        });

        let status = match timeout(Duration::from_millis(self.timeout_ms), child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                heartbeat_handle.abort();
                if track_pids()
                    && let Some(pid) = pid
                {
                    unregister_pid(pid);
                }
                return self.failure(input, &format!("wait failed: {e}"), started);
            }
            Err(_) => {
                heartbeat_handle.abort();
                let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
                if track_pids()
                    && let Some(pid) = pid
                {
                    unregister_pid(pid);
                }
                return self.failure(
                    input,
                    &format!("timed out after {} ms", self.timeout_ms),
                    started,
                );
            }
        };
        if track_pids()
            && let Some(pid) = pid
        {
            unregister_pid(pid);
        }

        heartbeat_handle.abort();
        let elapsed_secs = started.elapsed().as_secs();

        let stdout = stdout_handle.await.unwrap_or_default();
        let stderr = stderr_handle.await.unwrap_or_default();
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        if !status.success() {
            let code = status
                .code()
                .map_or_else(|| "signal".to_string(), |c| c.to_string());
            eprintln!("[{}] failed (exit {code}) after {elapsed_secs}s", self.name);
            return self.failure(
                input,
                &format!(
                    "exit {code}: {}",
                    stderr.lines().next().unwrap_or("claude failed")
                ),
                started,
            );
        }

        let text = Self::output_text(&stdout);
        if text.trim().is_empty() {
            eprintln!(
                "[{}] finished after {elapsed_secs}s but produced empty output",
                self.name
            );
            return self.failure(input, "claude produced an empty response", started);
        }

        eprintln!(
            "[{}] completed successfully ({elapsed_secs}s, {} bytes)",
            self.name,
            text.len()
        );

        let output_signal = input
            .derive(Kind::AgentOutput, Body::text(text))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("model", &self.model)
            .build();

        AgentResult::ok(output_signal)
            .with_trace(self.stderr_trace(&stderr))
            .with_usage(Usage {
                wall_ms,
                ..Default::default()
            })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_id(&self) -> &'static str {
        "claude_cli"
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

async fn read_pipe_to_string<R>(pipe: &mut Option<R>) -> String
where
    R: AsyncRead + Unpin,
{
    let Some(reader) = pipe.as_mut() else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if reader.read_to_end(&mut bytes).await.is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

const fn track_pids() -> bool {
    !cfg!(test)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    #[test]
    fn settings_json_contains_expected_hooks() {
        let value: Value = serde_json::from_str(&build_settings_json()).unwrap();
        let hooks = value
            .pointer("/hooks/PreToolUse/0/hooks")
            .and_then(Value::as_array)
            .expect("hooks array");
        assert!(hooks.len() >= 4);
        let matcher_strings: Vec<&str> = hooks
            .iter()
            .filter_map(|hook| hook.get("if").and_then(Value::as_str))
            .collect();
        assert!(matcher_strings.contains(&"Bash(git checkout *)"));
        assert!(matcher_strings.contains(&"Bash(git switch *)"));
        assert!(matcher_strings.contains(&"Bash(git branch -m *)"));
        assert!(matcher_strings.contains(&"Bash(git push *)"));
    }

    #[tokio::test]
    async fn runs_fake_claude_binary_and_passes_flags() {
        let tmp = tempdir().unwrap();
        let capture_args = tmp.path().join("args.txt");
        let capture_prompt = tmp.path().join("prompt.txt");
        let script = tmp.path().join("claude-fake.sh");
        let script_body = format!(
            r#"#!/bin/sh
set -eu
args_file="{args_file}"
prompt_file="{prompt_file}"
printf '%s\n' "$@" > "$args_file"
cat > "$prompt_file"
printf '%s\n' '{{"type":"content_block_delta","delta":{{"text":"hello"}}}}'
"#,
            args_file = capture_args.display(),
            prompt_file = capture_prompt.display(),
        );
        fs::write(&script, script_body).unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }

        let agent = ClaudeCliAgent::new(&script, tmp.path(), "claude-test-model")
            .with_system_prompt("system guidance")
            .with_allowed_tools("Read,Edit")
            .with_resume("session-123")
            .with_bare_mode(true);

        let result = agent.run(&prompt("hi there"), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap().trim(), "hello");

        let args_text = fs::read_to_string(&capture_args).unwrap();
        assert!(args_text.contains("--print"));
        assert!(args_text.contains("--verbose"));
        assert!(args_text.contains("--output-format"));
        assert!(args_text.contains("stream-json"));
        assert!(args_text.contains("--model"));
        assert!(args_text.contains("claude-test-model"));
        assert!(args_text.contains("--effort"));
        assert!(args_text.contains("medium"));
        assert!(args_text.contains("--max-turns"));
        assert!(args_text.contains("20"));
        assert!(args_text.contains("--append-system-prompt"));
        assert!(args_text.contains("system guidance"));
        assert!(args_text.contains("--settings"));
        assert!(args_text.contains("--dangerously-skip-permissions"));
        assert!(args_text.contains("--allowedTools"));
        assert!(args_text.contains("Read,Edit"));
        assert!(args_text.contains("--resume"));
        assert!(args_text.contains("session-123"));

        let prompt_text = fs::read_to_string(&capture_prompt).unwrap();
        assert_eq!(prompt_text, "hi there");
    }

    #[tokio::test]
    async fn can_disable_dangerous_skip_permissions_flag() {
        let tmp = tempdir().unwrap();
        let capture_args = tmp.path().join("args.txt");
        let script = tmp.path().join("claude-fake.sh");
        let script_body = format!(
            r#"#!/bin/sh
set -eu
args_file="{args_file}"
printf '%s\n' "$@" > "$args_file"
cat >/dev/null
printf '%s\n' '{{"type":"content_block_delta","delta":{{"text":"ok"}}}}'
"#,
            args_file = capture_args.display(),
        );
        fs::write(&script, script_body).unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }

        let agent = ClaudeCliAgent::new(&script, tmp.path(), "claude-test-model")
            .with_dangerously_skip_permissions(false);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );

        let args_text = fs::read_to_string(&capture_args).unwrap();
        assert!(!args_text.contains("--dangerously-skip-permissions"));
    }

    #[tokio::test]
    async fn optional_resume_none_omits_resume_flag() {
        let tmp = tempdir().unwrap();
        let capture_args = tmp.path().join("args.txt");
        let script = tmp.path().join("claude-fake.sh");
        let script_body = format!(
            r#"#!/bin/sh
set -eu
args_file="{args_file}"
printf '%s\n' "$@" > "$args_file"
cat >/dev/null
printf '%s\n' '{{"type":"content_block_delta","delta":{{"text":"ok"}}}}'
"#,
            args_file = capture_args.display(),
        );
        fs::write(&script, script_body).unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }

        let agent = ClaudeCliAgent::new(&script, tmp.path(), "claude-test-model")
            .with_optional_resume(None);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );

        let args_text = fs::read_to_string(&capture_args).unwrap();
        assert!(!args_text.contains("--resume"));
    }

    #[tokio::test]
    async fn benign_stderr_is_filtered_from_trace() {
        let tmp = tempdir().unwrap();
        let script = tmp.path().join("claude-fake.sh");
        let script_body = r#"#!/bin/sh
set -eu
cat >/dev/null
echo 'Claude CLI is starting up...' 1>&2
printf '%s\n' '{"type":"content_block_delta","delta":{"text":"ok"}}'
"#;
        fs::write(&script, script_body).unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }

        let agent = ClaudeCliAgent::new(&script, tmp.path(), "claude-test-model");
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );
        assert!(result.trace.is_empty());
    }
}
