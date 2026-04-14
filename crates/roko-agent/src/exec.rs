//! `ExecAgent` — runs any CLI that accepts a prompt on stdin and returns output on stdout.
//!
//! Works with tools like `ollama run`, `mods`, `llm`, or just `cat` / `echo`
//! for testing. This is the lowest-common-denominator LLM integration.

use crate::agent::{Agent, AgentResult};
use crate::process::{
    GRACE_STDIN_CLOSE_MS, benign_stderr_warn_once, classify_benign_stderr, kill_tree,
    register_spawned_pid, set_process_group, unregister_pid,
};
use crate::provider::current_safety_layer;
use crate::safety::SafetyLayer;
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Engram, Kind, Provenance};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

/// An agent that spawns a subprocess, pipes the input's text body to stdin,
/// and captures stdout as the output.
///
/// # Example
///
/// ```ignore
/// // Echo the prompt back (degenerate but demonstrates flow):
/// let agent = ExecAgent::new("cat", vec![]);
/// let prompt = Engram::builder(Kind::Prompt).body(Body::text("ping")).build();
/// let result = agent.run(&prompt, &Context::now()).await;
/// assert_eq!(result.output.body.as_text().unwrap().trim(), "ping");
/// ```
pub struct ExecAgent {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    current_dir: Option<PathBuf>,
    safety: Option<SafetyLayer>,
    timeout_ms: u64,
    name: String,
}

impl ExecAgent {
    /// An agent that invokes `program` with `args`, piping input on stdin.
    #[must_use]
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        let program = program.into();
        let name = format!("exec:{program}");
        Self {
            program,
            args,
            env: Vec::new(),
            current_dir: None,
            safety: None,
            timeout_ms: 120_000,
            name,
        }
    }

    /// Override the subprocess timeout in milliseconds (default 2 minutes).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Override the agent's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Add an env var to the spawned subprocess (e.g. `OLLAMA_NOPROGRESS=1`).
    #[must_use]
    pub fn with_env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Add multiple env vars at once.
    #[must_use]
    pub fn with_env<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in vars {
            self.env.push((k.into(), v.into()));
        }
        self
    }

    /// Run the subprocess from a specific working directory.
    #[must_use]
    pub fn with_current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    /// Attach a safety layer to the subprocess runtime.
    #[must_use]
    pub fn with_safety_layer(mut self, safety: Option<SafetyLayer>) -> Self {
        self.safety = safety;
        self
    }

    /// Attach the safety layer currently scoped to provider-backed construction.
    #[must_use]
    pub fn with_current_safety(mut self) -> Self {
        self.safety = current_safety_layer();
        self
    }
}

#[async_trait]
#[allow(clippy::too_many_lines)]
impl Agent for ExecAgent {
    async fn run(&self, input: &Engram, _ctx: &Context) -> AgentResult {
        let started = Instant::now();
        if let Some(safety) = &self.safety
            && let Err(err) = safety.check_exec_command(&self.program, &self.args)
        {
            return self.failure_signal(
                input,
                &format!("exec blocked by safety layer: {err}"),
                started,
            );
        }

        let prompt_text = match input.body.as_text() {
            Ok(s) => s.to_string(),
            Err(_) => {
                // Attempt JSON fallback: serialize body as JSON string.
                match serde_json::to_string(&input.body) {
                    Ok(s) => s,
                    Err(e) => {
                        return self.failure_signal(
                            input,
                            &format!("input body not readable as text or json: {e}"),
                            started,
                        );
                    }
                }
            }
        };

        // Spawn subprocess.
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        if let Some(dir) = &self.current_dir {
            cmd.current_dir(dir);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        set_process_group(&mut cmd);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return self.failure_signal(input, &format!("spawn failed: {e}"), started);
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

        // Write prompt to stdin, then close it.
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(prompt_text.as_bytes()).await {
                let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
                if track_pids()
                    && let Some(pid) = pid
                {
                    unregister_pid(pid);
                }
                return self.failure_signal(input, &format!("stdin write failed: {e}"), started);
            }
            drop(stdin);
        }

        eprintln!(
            "[{}] agent started (pid {}, timeout {}s)",
            self.name,
            pid.unwrap_or(0),
            self.timeout_ms / 1000
        );

        let has_activity = Arc::new(AtomicBool::new(false));

        // Stream stdout in chunks, reporting progress without altering content.
        let stdout_name = self.name.clone();
        let stdout_activity = has_activity.clone();
        let stdout_handle = tokio::spawn(async move {
            let Some(mut pipe) = stdout_pipe else {
                return String::new();
            };
            let mut buf = [0u8; 8192];
            let mut collected = Vec::new();
            let mut last_report: usize = 0;
            loop {
                match pipe.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        collected.extend_from_slice(&buf[..n]);
                        stdout_activity.store(true, Ordering::Relaxed);
                        // Report progress every ~4KB.
                        if collected.len() - last_report >= 4096 || last_report == 0 {
                            eprintln!(
                                "[{stdout_name}] receiving output... ({} bytes so far)",
                                collected.len()
                            );
                            last_report = collected.len();
                        }
                    }
                    Err(_) => break,
                }
            }
            String::from_utf8_lossy(&collected).into_owned()
        });

        // Stream stderr in real time.
        let stderr_name = self.name.clone();
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
                    stderr_activity.store(true, Ordering::Relaxed);
                    eprintln!("[{stderr_name}] {line}");
                }
                collected.push_str(&line);
                collected.push('\n');
            }
            collected
        });

        // Heartbeat when no output activity.
        let heartbeat_name = self.name.clone();
        let heartbeat_started = started;
        let heartbeat_activity = has_activity.clone();
        let heartbeat_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.tick().await;
            loop {
                interval.tick().await;
                if !heartbeat_activity.swap(false, Ordering::Relaxed) {
                    let elapsed = heartbeat_started.elapsed().as_secs();
                    eprintln!("[{heartbeat_name}] waiting for response... ({elapsed}s elapsed)");
                }
            }
        });

        // Wait for exit with timeout.
        let status = match timeout(Duration::from_millis(self.timeout_ms), child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                heartbeat_handle.abort();
                if track_pids()
                    && let Some(pid) = pid
                {
                    unregister_pid(pid);
                }
                return self.failure_signal(input, &format!("wait failed: {e}"), started);
            }
            Err(_) => {
                heartbeat_handle.abort();
                let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
                if track_pids()
                    && let Some(pid) = pid
                {
                    unregister_pid(pid);
                }
                return self.failure_signal(
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

        let stdout = self.scrub_text(&stdout_handle.await.unwrap_or_default());
        let stderr = self.scrub_text(&stderr_handle.await.unwrap_or_default());
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        if !status.success() {
            let code = status
                .code()
                .map_or_else(|| "signal".into(), |c| c.to_string());
            eprintln!("[{}] failed (exit {code}) after {elapsed_secs}s", self.name);
            return self.failure_signal(
                input,
                &format!("exit {code}: {}", first_line(&stderr)),
                started,
            );
        }

        eprintln!(
            "[{}] completed successfully ({elapsed_secs}s, {} bytes)",
            self.name,
            stdout.len()
        );

        let out_signal = input
            .derive(Kind::AgentOutput, Body::text(stdout.clone()))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("exit_code", "0")
            .build();

        // Trace: one signal per non-empty stderr line (as AgentMessage events).
        let trace = stderr_trace(&self.name, &stderr);

        AgentResult::ok(out_signal)
            .with_trace(trace)
            .with_usage(Usage {
                input_tokens: u32::try_from(prompt_text.len() / 4).unwrap_or(u32::MAX),
                output_tokens: u32::try_from(stdout.len() / 4).unwrap_or(u32::MAX),
                wall_ms,
                ..Default::default()
            })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        // We collect all output before returning — not streaming.
        false
    }
}

impl ExecAgent {
    fn failure_signal(&self, input: &Engram, reason: &str, started: Instant) -> AgentResult {
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

    fn scrub_text(&self, content: &str) -> String {
        self.safety
            .as_ref()
            .map_or_else(|| content.to_string(), |safety| safety.scrub_text(content))
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

const fn track_pids() -> bool {
    !cfg!(test)
}

fn maybe_warn_and_filter_benign(name: &str, line: &str) -> bool {
    if let Some(benign) = classify_benign_stderr(line) {
        if benign_stderr_warn_once(benign.key) {
            eprintln!("[{name}] {}", benign.summary);
        }
        return true;
    }
    false
}

fn stderr_trace(name: &str, stderr: &str) -> Vec<Engram> {
    stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !maybe_warn_and_filter_benign(name, line))
        .map(|line| {
            Engram::builder(Kind::AgentMessage)
                .body(Body::text(line))
                .provenance(Provenance::agent(name))
                .tag("stream", "stderr")
                .build()
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    #[tokio::test]
    async fn env_vars_reach_subprocess() {
        let agent = ExecAgent::new("sh", vec!["-c".into(), "echo $ROKO_TEST_VAR".into()])
            .with_env_var("ROKO_TEST_VAR", "hello_from_env");
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(result.success);
        let out = result.output.body.as_text().unwrap();
        assert_eq!(out.trim(), "hello_from_env");
    }

    #[tokio::test]
    async fn with_env_accepts_iterator() {
        let agent = ExecAgent::new("sh", vec!["-c".into(), "echo $A-$B".into()])
            .with_env([("A", "alpha"), ("B", "beta")]);
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap().trim(), "alpha-beta");
    }

    #[tokio::test]
    async fn current_dir_reaches_subprocess() {
        let temp = tempfile::tempdir().expect("tempdir");
        let agent = ExecAgent::new("pwd", vec![]).with_current_dir(temp.path());
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(result.success);
        let expected = std::fs::canonicalize(temp.path()).expect("canonical tempdir");
        let actual = std::fs::canonicalize(result.output.body.as_text().unwrap().trim())
            .expect("canonical subprocess pwd");
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn cat_echoes_prompt() {
        let agent = ExecAgent::new("cat", vec![]);
        let result = agent.run(&prompt("roundtrip text"), &Context::now()).await;
        assert!(result.success);
        let out = result.output.body.as_text().unwrap();
        assert_eq!(out, "roundtrip text");
    }

    #[tokio::test]
    async fn exit_code_failure_marks_result_failed() {
        // `false` always exits non-zero.
        let agent = ExecAgent::new("false", vec![]);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert_eq!(result.output.tag("failed"), Some("true"));
    }

    #[tokio::test]
    async fn nonexistent_binary_fails_gracefully() {
        let agent = ExecAgent::new("definitely_not_a_real_binary_xyz", vec![]);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap()
                .contains("spawn failed")
        );
    }

    #[tokio::test]
    async fn timeout_fails_result() {
        let agent = ExecAgent::new("sleep", vec!["10".into()]).with_timeout_ms(100);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(result.output.body.as_text().unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn output_tracks_input_as_lineage() {
        let agent = ExecAgent::new("cat", vec![]);
        let input = prompt("lineage test");
        let input_id = input.id;
        let result = agent.run(&input, &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.lineage, vec![input_id]);
    }

    #[tokio::test]
    async fn usage_has_estimated_tokens() {
        let agent = ExecAgent::new("cat", vec![]);
        let text = "x".repeat(400); // ~100 tokens
        let result = agent.run(&prompt(&text), &Context::now()).await;
        assert!(result.success);
        assert!(result.usage.input_tokens >= 90);
        assert!(result.usage.input_tokens <= 110);
        assert!(result.usage.wall_ms > 0);
    }

    #[tokio::test]
    async fn stderr_becomes_trace_signals() {
        // Use `sh -c` to emit both stdout and stderr.
        let agent = ExecAgent::new(
            "sh",
            vec!["-c".into(), "echo hello; echo 'warning: x' 1>&2".into()],
        );
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(result.success);
        assert!(result.output.body.as_text().unwrap().contains("hello"));
        assert_eq!(result.trace.len(), 1);
        assert_eq!(result.trace[0].kind, Kind::AgentMessage);
        assert!(result.trace[0].body.as_text().unwrap().contains("warning"));
    }

    #[tokio::test]
    async fn safety_blocks_dangerous_shell_before_spawn() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sentinel = temp.path().join("sentinel");
        let command = format!("touch {}; rm -rf /", sentinel.display());
        let agent = ExecAgent::new("sh", vec!["-c".into(), command])
            .with_current_dir(temp.path())
            .with_safety_layer(Some(SafetyLayer::with_defaults()));
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap()
                .contains("blocked by safety layer")
        );
        assert!(!sentinel.exists());
    }

    #[tokio::test]
    async fn safety_allows_safe_shell_wrapper() {
        let agent = ExecAgent::new("sh", vec!["-c".into(), "echo ok".into()])
            .with_safety_layer(Some(SafetyLayer::with_defaults()));
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap().trim(), "ok");
    }

    #[tokio::test]
    async fn safety_scrubs_stdout_and_stderr() {
        let secret = "sk-ant-api03-abcdefghij1234567890abcdefghij1234567890abcdefghij1234567890abcdefghij1234-AAAAAA";
        let command = format!("printf '%s' '{secret}'; printf '%s\\n' '{secret}' 1>&2");
        let agent = ExecAgent::new("sh", vec!["-c".into(), command])
            .with_safety_layer(Some(SafetyLayer::with_defaults()));
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(result.success);
        let output = result.output.body.as_text().unwrap();
        assert!(!output.contains(secret));
        assert!(output.contains("[REDACTED]"));
        assert_eq!(result.trace.len(), 1);
        let stderr = result.trace[0].body.as_text().unwrap();
        assert!(!stderr.contains(secret));
        assert!(stderr.contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn benign_stderr_is_suppressed_from_trace() {
        let agent = ExecAgent::new(
            "sh",
            vec![
                "-c".into(),
                "echo ok; echo 'Claude CLI is starting up...' 1>&2".into(),
            ],
        );
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(result.success);
        assert!(result.trace.is_empty());
    }
}
