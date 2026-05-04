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
use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;
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
            fallback_model: Some(roko_core::defaults::MODEL_FAST.to_string()),
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
            timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
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
        let stream_usage = StreamUsage::default();
        self.failure_with_stream_usage(input, reason, started, &stream_usage)
    }

    fn failure_with_stream_usage(
        &self,
        input: &Engram,
        reason: &str,
        started: Instant,
        stream_usage: &StreamUsage,
    ) -> AgentResult {
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut output = input
            .derive(Kind::AgentOutput, Body::text(reason))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("failed", "true");
        if let Some(model) = stream_usage
            .model
            .as_deref()
            .filter(|model| !model.trim().is_empty())
        {
            output = output.tag("model", model);
        }
        let output = output.build();
        AgentResult::fail(output).with_usage(Self::usage_from_stream(stream_usage, wall_ms))
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
        let events: Vec<Value> = stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter_map(Self::parse_stream_event)
            .collect();
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

    fn debug_enabled() -> bool {
        std::env::var_os("ROKO_DEBUG")
            .map(|value| {
                let value = value.to_string_lossy().trim().to_ascii_lowercase();
                matches!(value.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false)
    }

    fn parse_stream_event(line: &str) -> Option<Value> {
        let value = serde_json::from_str::<Value>(line.trim()).ok()?;
        if value.get("type").and_then(Value::as_str).is_some() {
            Some(value)
        } else {
            None
        }
    }

    fn parse_stream_usage(stdout: &str) -> StreamUsage {
        let mut usage = StreamUsage::default();
        for line in stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let Some(event) = Self::parse_stream_event(line) else {
                continue;
            };
            if event.get("type").and_then(Value::as_str) != Some("result") {
                continue;
            }

            usage.source = UsageSource::ProviderReported;
            if let Some(model) = event
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
            {
                usage.model = Some(model.to_string());
            }
            if let Some(cost) = event.get("total_cost_usd").and_then(Value::as_f64) {
                usage.cost_usd = Some(cost);
            }
            if let Some(result_usage) = event.get("usage") {
                Self::update_stream_usage_field(
                    &mut usage.input_tokens,
                    Self::stream_usage_u64(result_usage, &["input_tokens"]),
                );
                Self::update_stream_usage_field(
                    &mut usage.output_tokens,
                    Self::stream_usage_u64(result_usage, &["output_tokens"]),
                );
                Self::update_stream_usage_field(
                    &mut usage.cache_creation_tokens,
                    Self::stream_usage_u64(
                        result_usage,
                        &["cache_creation_input_tokens", "cache_creation_tokens"],
                    ),
                );
                Self::update_stream_usage_field(
                    &mut usage.cache_read_tokens,
                    Self::stream_usage_u64(
                        result_usage,
                        &["cache_read_input_tokens", "cache_read_tokens"],
                    ),
                );
            }
        }
        usage
    }

    fn usage_from_stream(stream_usage: &StreamUsage, wall_ms: u64) -> Usage {
        Usage {
            input_tokens: Self::saturating_u64_to_u32(stream_usage.input_tokens),
            output_tokens: Self::saturating_u64_to_u32(stream_usage.output_tokens),
            cache_read_tokens: Self::saturating_u64_to_u32(stream_usage.cache_read_tokens),
            cache_create_tokens: Self::saturating_u64_to_u32(stream_usage.cache_creation_tokens),
            cost_usd: stream_usage.cost_usd.unwrap_or(0.0) as f32,
            wall_ms,
        }
    }

    fn stream_usage_u64(usage: &Value, keys: &[&str]) -> Option<u64> {
        keys.iter()
            .find_map(|key| usage.get(*key).and_then(Value::as_u64))
    }

    fn update_stream_usage_field<T>(slot: &mut Option<T>, value: Option<T>) {
        if let Some(value) = value {
            *slot = Some(value);
        }
    }

    fn saturating_u64_to_u32(value: Option<u64>) -> u32 {
        value
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0)
    }

    fn tool_summary(block: &Value) -> String {
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let mut summary = format!("tool: {name}");

        if let Some(input) = block.get("input") {
            for key in ["path", "file_path", "filename"] {
                if let Some(path) = input.get(key).and_then(Value::as_str) {
                    summary.push_str(&format!(" path={path}"));
                    return summary;
                }
            }
            if let Some(command) = input.get("command").and_then(Value::as_str) {
                summary.push_str(&format!(" command={command}"));
            }
        }

        summary
    }

    fn emit_stream_summary(
        agent_name: &str,
        event: &Value,
        text_bytes: &mut usize,
        tool_count: &mut usize,
    ) {
        match event.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                if let Some(content) = event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for block in content {
                        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                            *tool_count += 1;
                            eprintln!("[{agent_name}] {}", Self::tool_summary(block));
                        }
                    }
                }
            }
            Some("content_block_start") => {
                if let Some(block) = event.get("content_block") {
                    match block.get("type").and_then(Value::as_str) {
                        Some("tool_use") => {
                            *tool_count += 1;
                            eprintln!("[{agent_name}] {}", Self::tool_summary(block));
                        }
                        Some("text") => {
                            eprintln!("[{agent_name}] generating text...");
                        }
                        _ => {}
                    }
                }
            }
            Some("content_block_delta") => {
                if let Some(delta) = event.get("delta")
                    && let Some(text) = delta.get("text").and_then(Value::as_str)
                {
                    *text_bytes += text.len();
                }
            }
            Some("result") => {
                let summary = if *tool_count > 0 {
                    format!("{text_bytes} bytes text, {tool_count} tool calls")
                } else {
                    format!("{text_bytes} bytes text")
                };
                eprintln!("[{agent_name}] result received ({summary})");
            }
            _ => {}
        }
    }

    fn first_human_stderr_line(stderr: &str) -> Option<&str> {
        stderr.lines().find(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && Self::parse_stream_event(trimmed).is_none()
                && classify_benign_stderr(line).is_none()
        })
    }

    fn stream_requested_tool_use(events: &[Value]) -> bool {
        events
            .iter()
            .any(|event| match event.get("type").and_then(Value::as_str) {
                Some("assistant") => event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                    .is_some_and(|content| {
                        content.iter().any(|block| {
                            block.get("type").and_then(Value::as_str) == Some("tool_use")
                        })
                    }),
                Some("content_block_start") => {
                    event
                        .get("content_block")
                        .and_then(|block| block.get("type").and_then(Value::as_str))
                        == Some("tool_use")
                }
                Some("tool") => true,
                _ => false,
            })
    }

    fn output_text(stdout: &str) -> String {
        Self::parse_stream_events(stdout).map_or_else(
            || stdout.trim().to_string(),
            |events| {
                let requested_tool_use = Self::stream_requested_tool_use(&events);
                let response = crate::translate::BackendResponse::StreamJson(events);
                let extracted = response.extract_text();
                if extracted.trim().is_empty() {
                    if requested_tool_use {
                        "assistant requested tool use".to_string()
                    } else {
                        String::new()
                    }
                } else {
                    extracted
                }
            },
        )
    }

    fn stderr_trace(&self, stderr: &str) -> Vec<Engram> {
        stderr
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && Self::parse_stream_event(trimmed).is_none()
            })
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
        let debug_enabled = Self::debug_enabled();

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
                if debug_enabled {
                    eprintln!("{line}");
                }

                // Parse stream-json events for progress reporting.
                // Non-JSON output (raw text from other agents) is fine — we
                // just skip the progress parsing.
                if let Some(event) = Self::parse_stream_event(trimmed) {
                    Self::emit_stream_summary(
                        &stdout_name,
                        &event,
                        &mut text_bytes,
                        &mut tool_count,
                    );
                }
            }
            collected
        });

        // Stream stderr in real time. Raw stream JSON stays hidden in normal
        // mode, but debug mode echoes it verbatim for inspection.
        let agent_name = self.name.clone();
        let stderr_agent = self.clone();
        let stderr_activity = has_activity.clone();
        let stderr_handle = tokio::spawn(async move {
            let Some(pipe) = stderr_pipe else {
                return String::new();
            };
            let reader = BufReader::new(pipe);
            let mut lines = reader.lines();
            let mut collected = String::new();
            let mut text_bytes: usize = 0;
            let mut tool_count: usize = 0;
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                stderr_activity.store(true, std::sync::atomic::Ordering::Relaxed);
                collected.push_str(&line);
                collected.push('\n');
                if let Some(event) = Self::parse_stream_event(trimmed) {
                    if debug_enabled {
                        eprintln!("{line}");
                    }
                    Self::emit_stream_summary(
                        &agent_name,
                        &event,
                        &mut text_bytes,
                        &mut tool_count,
                    );
                } else if debug_enabled {
                    eprintln!("{line}");
                } else if !stderr_agent.warn_and_filter_benign(&line) {
                    eprintln!("[{agent_name}] {line}");
                }
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
        let stream_usage =
            Self::parse_stream_usage(&stdout).merge(Self::parse_stream_usage(&stderr));

        if !status.success() {
            let code = status
                .code()
                .map_or_else(|| "signal".to_string(), |c| c.to_string());
            eprintln!("[{}] failed (exit {code}) after {elapsed_secs}s", self.name);
            let stderr_reason = Self::first_human_stderr_line(&stderr).unwrap_or("claude failed");
            return self.failure_with_stream_usage(
                input,
                &format!("exit {code}: {stderr_reason}"),
                started,
                &stream_usage,
            );
        }

        let text = {
            let text = Self::output_text(&stdout);
            if text.trim().is_empty() {
                Self::output_text(&stderr)
            } else {
                text
            }
        };
        if text.trim().is_empty() {
            eprintln!(
                "[{}] finished after {elapsed_secs}s but produced empty output",
                self.name
            );
            return self.failure_with_stream_usage(
                input,
                "claude produced an empty response",
                started,
                &stream_usage,
            );
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
            .tag(
                "model",
                stream_usage.model.as_deref().unwrap_or(&self.model),
            )
            .build();

        AgentResult::ok(output_signal)
            .with_trace(self.stderr_trace(&stderr))
            .with_usage(Self::usage_from_stream(&stream_usage, wall_ms))
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

/// Whether usage was reported by the final Claude CLI result event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum UsageSource {
    ProviderReported,
    #[default]
    Unknown,
}

/// Parsed usage metadata from Claude CLI `result` events.
#[derive(Debug, Clone, PartialEq, Default)]
struct StreamUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cost_usd: Option<f64>,
    model: Option<String>,
    source: UsageSource,
}

impl StreamUsage {
    fn merge(mut self, other: Self) -> Self {
        if self.source == UsageSource::Unknown {
            return other;
        }
        if other.source == UsageSource::ProviderReported {
            self.input_tokens = self.input_tokens.or(other.input_tokens);
            self.output_tokens = self.output_tokens.or(other.output_tokens);
            self.cache_creation_tokens = self.cache_creation_tokens.or(other.cache_creation_tokens);
            self.cache_read_tokens = self.cache_read_tokens.or(other.cache_read_tokens);
            self.cost_usd = self.cost_usd.or(other.cost_usd);
            self.model = self.model.or(other.model);
        }
        self
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

    #[test]
    fn parse_stream_usage_extracts_result_event_fields_and_model() {
        let usage = ClaudeCliAgent::parse_stream_usage(
            r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":999,"output_tokens":888,"cache_creation_input_tokens":777,"cache_read_input_tokens":666}}}
{"type":"result","session_id":"sess-1","model":"claude-sonnet-4-6","total_cost_usd":0.25,"usage":{"input_tokens":11,"output_tokens":22,"cache_creation_input_tokens":33,"cache_read_input_tokens":44}}"#,
        );
        assert_eq!(usage.source, UsageSource::ProviderReported);
        assert_eq!(usage.input_tokens, Some(11));
        assert_eq!(usage.output_tokens, Some(22));
        assert_eq!(usage.cache_creation_tokens, Some(33));
        assert_eq!(usage.cache_read_tokens, Some(44));
        assert_eq!(usage.cost_usd, Some(0.25));
        assert_eq!(usage.model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn parse_stream_usage_leaves_missing_fields_none_and_keeps_zeroes() {
        let usage = ClaudeCliAgent::parse_stream_usage(
            r#"{"type":"result","session_id":"sess-2","model":"claude-sonnet-4-6","total_cost_usd":0,"usage":{"input_tokens":0,"cache_read_input_tokens":5}}"#,
        );
        assert_eq!(usage.source, UsageSource::ProviderReported);
        assert_eq!(usage.input_tokens, Some(0));
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.cache_creation_tokens, None);
        assert_eq!(usage.cache_read_tokens, Some(5));
        assert_eq!(usage.cost_usd, Some(0.0));
        assert_eq!(usage.model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn parse_stream_usage_accepts_cache_alias_fields() {
        let usage = ClaudeCliAgent::parse_stream_usage(
            r#"{"type":"result","session_id":"sess-3","model":"claude-sonnet-4-6","total_cost_usd":0.5,"usage":{"input_tokens":1,"output_tokens":2,"cache_creation_tokens":3,"cache_read_tokens":4}}"#,
        );
        assert_eq!(usage.cache_creation_tokens, Some(3));
        assert_eq!(usage.cache_read_tokens, Some(4));
        assert_eq!(usage.cost_usd, Some(0.5));
    }

    #[test]
    fn parse_stream_usage_stays_unknown_without_result_event() {
        let usage = ClaudeCliAgent::parse_stream_usage(
            r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":1,"output_tokens":2,"cache_creation_input_tokens":3,"cache_read_input_tokens":4}}}
{"type":"tool","subtype":"result","tool_name":"Bash","tool_use_id":"tu_1","content":"done"}"#,
        );
        assert_eq!(usage, StreamUsage::default());
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
        assert!(args_text.contains("--tools"));
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
    async fn result_event_usage_is_threaded_into_agent_result() {
        let tmp = tempdir().unwrap();
        let script = tmp.path().join("claude-fake.sh");
        let script_body = r#"#!/bin/sh
set -eu
cat >/dev/null
printf '%s\n' '{"type":"content_block_delta","delta":{"text":"hello"}}'
printf '%s\n' '{"type":"result","session_id":"sess-1","model":"claude-sonnet-4-6","total_cost_usd":0.25,"usage":{"input_tokens":11,"output_tokens":22,"cache_creation_input_tokens":33,"cache_read_input_tokens":44}}'
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
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap().trim(), "hello");
        assert_eq!(result.output.tag("model"), Some("claude-sonnet-4-6"));
        assert_eq!(result.usage.input_tokens, 11);
        assert_eq!(result.usage.output_tokens, 22);
        assert_eq!(result.usage.cache_read_tokens, 44);
        assert_eq!(result.usage.cache_create_tokens, 33);
        assert!((result.usage.cost_usd - 0.25).abs() < 0.0001);
    }

    #[tokio::test]
    async fn nonzero_exit_still_carries_result_event_usage() {
        let tmp = tempdir().unwrap();
        let script = tmp.path().join("claude-fake.sh");
        let script_body = r#"#!/bin/sh
set -eu
cat >/dev/null
printf '%s\n' '{"type":"result","session_id":"sess-2","model":"claude-sonnet-4-6","total_cost_usd":0.5,"usage":{"input_tokens":9,"output_tokens":8,"cache_creation_input_tokens":7,"cache_read_input_tokens":6}}'
exit 1
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
        assert!(!result.success);
        assert_eq!(result.output.tag("model"), Some("claude-sonnet-4-6"));
        assert_eq!(result.usage.input_tokens, 9);
        assert_eq!(result.usage.output_tokens, 8);
        assert_eq!(result.usage.cache_read_tokens, 6);
        assert_eq!(result.usage.cache_create_tokens, 7);
        assert!((result.usage.cost_usd - 0.5).abs() < 0.0001);
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

    #[test]
    fn stderr_trace_skips_stream_json_lines() {
        let agent = ClaudeCliAgent::new("claude", ".", "claude-test-model");
        let trace = agent.stderr_trace(
            "unexpected stderr line\n{\"type\":\"content_block_delta\",\"delta\":{\"text\":\"ok\"}}\n",
        );
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].body.as_text().unwrap(), "unexpected stderr line");
    }
}
