//! `ExecAgent` — runs any CLI that accepts a prompt on stdin and returns output on stdout.
//!
//! Works with tools like `ollama run`, `mods`, `llm`, or just `cat` / `echo`
//! for testing. This is the lowest-common-denominator LLM integration.

use crate::agent::{Agent, AgentResult};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Kind, Provenance, Signal};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
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
/// let prompt = Signal::builder(Kind::Prompt).body(Body::text("ping")).build();
/// let result = agent.run(&prompt, &Context::now()).await;
/// assert_eq!(result.output.body.as_text().unwrap().trim(), "ping");
/// ```
pub struct ExecAgent {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
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
}

#[async_trait]
#[allow(clippy::too_many_lines)]
impl Agent for ExecAgent {
    async fn run(&self, input: &Signal, _ctx: &Context) -> AgentResult {
        let started = Instant::now();
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
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return self.failure_signal(
                    input,
                    &format!("spawn failed: {e}"),
                    started,
                );
            }
        };

        // Write prompt to stdin, then close it.
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(prompt_text.as_bytes()).await {
                return self.failure_signal(
                    input,
                    &format!("stdin write failed: {e}"),
                    started,
                );
            }
            drop(stdin);
        }

        // Wait for exit with timeout.
        let output = match timeout(
            Duration::from_millis(self.timeout_ms),
            child.wait_with_output(),
        )
        .await
        {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return self.failure_signal(
                    input,
                    &format!("wait failed: {e}"),
                    started,
                );
            }
            Err(_) => {
                return self.failure_signal(
                    input,
                    &format!("timed out after {} ms", self.timeout_ms),
                    started,
                );
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        if !output.status.success() {
            let code = output
                .status
                .code()
                .map_or_else(|| "signal".into(), |c| c.to_string());
            return self.failure_signal(
                input,
                &format!("exit {code}: {}", first_line(&stderr)),
                started,
            );
        }

        let out_signal = input
            .derive(Kind::AgentOutput, Body::text(stdout.clone()))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .tag("exit_code", "0")
            .build();

        // Trace: one signal per non-empty stderr line (as AgentMessage events).
        let trace: Vec<Signal> = stderr
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                Signal::builder(Kind::AgentMessage)
                    .body(Body::text(line))
                    .provenance(Provenance::agent(&self.name))
                    .tag("stream", "stderr")
                    .build()
            })
            .collect();

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
    fn failure_signal(&self, input: &Signal, reason: &str, started: Instant) -> AgentResult {
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
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
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
        let agent = ExecAgent::new(
            "sh",
            vec!["-c".into(), "echo $A-$B".into()],
        )
        .with_env([("A", "alpha"), ("B", "beta")]);
        let result = agent.run(&prompt(""), &Context::now()).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap().trim(), "alpha-beta");
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
        assert!(result
            .output
            .body
            .as_text()
            .unwrap()
            .contains("spawn failed"));
    }

    #[tokio::test]
    async fn timeout_fails_result() {
        let agent = ExecAgent::new("sleep", vec!["10".into()]).with_timeout_ms(100);
        let result = agent.run(&prompt("x"), &Context::now()).await;
        assert!(!result.success);
        assert!(result
            .output
            .body
            .as_text()
            .unwrap()
            .contains("timed out"));
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
        assert!(result.trace[0]
            .body
            .as_text()
            .unwrap()
            .contains("warning"));
    }
}
