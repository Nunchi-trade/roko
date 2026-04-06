//! `ShellGate` — runs an arbitrary shell command; passes on exit code 0.
//!
//! `ShellGate` is the simplest real gate. It's useful as a building block and
//! for bespoke checks (custom lints, site-specific invariants, pre-commit-style
//! hooks). It never consults the input signal's body beyond reading a
//! [`GatePayload`] if present (for `working_dir` and environment).

use crate::payload::GatePayload;
use async_trait::async_trait;
use roko_core::{Context, Gate, Signal, Verdict};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

/// A gate that runs a fixed shell command; pass = exit code 0.
///
/// The signal's body (if a [`GatePayload`]) provides `working_dir` and
/// `extra_env`. If the body is missing or malformed, the gate uses the
/// current process's cwd and no extra env.
pub struct ShellGate {
    program: String,
    args: Vec<String>,
    timeout_ms: u64,
    name: String,
}

impl ShellGate {
    /// Construct a shell gate that runs `program` with `args`.
    #[must_use]
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        let program = program.into();
        let name = format!("shell:{program}");
        Self {
            program,
            args,
            timeout_ms: 300_000, // 5 minutes
            name,
        }
    }

    /// Override the timeout in milliseconds (default: 5 minutes).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Override the gate's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait]
impl Gate for ShellGate {
    async fn verify(&self, signal: &Signal, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let payload: Option<GatePayload> = signal.body.as_json().ok();

        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        cmd.kill_on_drop(true);

        if let Some(ref p) = payload {
            cmd.current_dir(&p.working_dir);
            if let Some(ref tgt) = p.target_dir {
                cmd.env("CARGO_TARGET_DIR", tgt);
            }
            for (k, v) in &p.extra_env {
                cmd.env(k, v);
            }
        }

        let output_future = async { cmd.output().await };
        let result = timeout(Duration::from_millis(self.timeout_ms), output_future).await;
        #[allow(clippy::cast_possible_truncation)]
        let elapsed = started.elapsed().as_millis() as u64;

        match result {
            Err(_timeout) => Verdict::fail(
                &self.name,
                format!("timed out after {} ms", self.timeout_ms),
            )
            .with_duration(elapsed),
            Ok(Err(io_err)) => Verdict::fail(
                &self.name,
                format!("spawn failed: {io_err}"),
            )
            .with_duration(elapsed),
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let combined = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{stdout}\n---stderr---\n{stderr}")
                };
                if output.status.success() {
                    Verdict::pass(&self.name)
                        .with_detail(combined)
                        .with_duration(elapsed)
                } else {
                    let code = output
                        .status
                        .code()
                        .map_or_else(|| "terminated by signal".into(), |c| c.to_string());
                    Verdict::fail(&self.name, format!("exit code: {code}"))
                        .with_detail(combined)
                        .with_duration(elapsed)
                }
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    fn empty_signal() -> Signal {
        Signal::builder(Kind::Task).body(Body::empty()).build()
    }

    #[tokio::test]
    async fn true_command_passes() {
        let gate = ShellGate::new("true", vec![]);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(v.passed);
        assert_eq!(v.gate, "shell:true");
    }

    #[tokio::test]
    async fn false_command_fails() {
        let gate = ShellGate::new("false", vec![]);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("exit code"));
    }

    #[tokio::test]
    async fn echo_command_captures_output() {
        let gate = ShellGate::new("echo", vec!["hello from gate".into()]);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(v.passed);
        let detail = v.detail.as_deref().unwrap();
        assert!(detail.contains("hello from gate"));
    }

    #[tokio::test]
    async fn nonexistent_command_fails_gracefully() {
        let gate = ShellGate::new("definitely_not_a_real_command_xyz", vec![]);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("spawn failed"));
    }

    #[tokio::test]
    async fn timeout_causes_failure() {
        let gate = ShellGate::new("sleep", vec!["10".into()]).with_timeout_ms(100);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("timed out"));
    }

    #[tokio::test]
    async fn records_duration() {
        let gate = ShellGate::new("true", vec![]);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(v.passed);
        // Duration should be non-zero but small.
        assert!(v.duration_ms < 5000);
    }

    #[tokio::test]
    async fn custom_name_appears_in_verdict() {
        let gate = ShellGate::new("true", vec![]).with_name("my_custom_gate");
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert_eq!(v.gate, "my_custom_gate");
    }
}
