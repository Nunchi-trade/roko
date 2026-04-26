//! `CompileGate` — verifies a project compiles cleanly via its build system.
//!
//! `CompileGate` is a build-system-aware wrapper around [`ShellGate`]. It
//! reads a [`GatePayload`] from the signal body to determine the working
//! directory, then runs the appropriate check command (e.g. `cargo check
//! --workspace`).
//!
//! This is the "Rung 1" gate from Mori's 6-rung verification ladder: the
//! cheapest check that proves the code at least compiles.

use crate::compile_errors::{classify_gate_failure, render_failure_classification};
use crate::payload::{BuildSystem, GatePayload};
use async_trait::async_trait;
use roko_core::{Context, Engram, Verify, Verdict};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

/// Verifies a project compiles via its build system.
pub struct CompileGate {
    build_system: BuildSystem,
    extra_args: Vec<String>,
    timeout_ms: u64,
    name: String,
}

impl CompileGate {
    /// A compile gate for the given build system with default args.
    #[must_use]
    pub fn new(build_system: BuildSystem) -> Self {
        Self {
            build_system,
            extra_args: Vec::new(),
            timeout_ms: 600_000, // 10 minutes
            name: format!("compile:{}", build_system.program()),
        }
    }

    /// Shortcut: a cargo-based compile gate.
    #[must_use]
    pub fn cargo() -> Self {
        Self::new(BuildSystem::Cargo)
    }

    /// Add extra args to the check command.
    #[must_use]
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args.extend(args);
        self
    }

    /// Override the timeout in milliseconds.
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
}

impl roko_core::Cell for CompileGate {
    fn cell_id(&self) -> &str { "compile-gate" }
    fn cell_name(&self) -> &str { "CompileGate" }
    fn protocols(&self) -> &[&str] { &["Verify"] }
}

#[async_trait]
#[allow(clippy::cast_possible_truncation)]
impl Verify for CompileGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let payload: GatePayload = match signal.body.as_json() {
            Ok(p) => p,
            Err(e) => {
                return Verdict::fail(&self.name, format!("signal body is not a GatePayload: {e}"))
                    .with_duration(started.elapsed().as_millis() as u64);
            }
        };

        let mut cmd = Command::new(self.build_system.program());
        for arg in self.build_system.check_args() {
            cmd.arg(arg);
        }
        for arg in &self.extra_args {
            cmd.arg(arg);
        }
        if self.build_system == BuildSystem::Cargo
            && !self
                .extra_args
                .iter()
                .any(|arg| arg.starts_with("--message-format"))
        {
            cmd.arg("--message-format=json");
        }
        cmd.current_dir(&payload.working_dir);
        cmd.kill_on_drop(true);

        if let Some(ref tgt) = payload.target_dir {
            cmd.env("CARGO_TARGET_DIR", tgt);
        }
        for (k, v) in &payload.extra_env {
            cmd.env(k, v);
        }

        let output = match timeout(Duration::from_millis(self.timeout_ms), cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Verdict::fail(&self.name, format!("spawn failed: {e}"))
                    .with_duration(started.elapsed().as_millis() as u64);
            }
            Err(_) => {
                return Verdict::fail(
                    &self.name,
                    format!("timed out after {} ms", self.timeout_ms),
                )
                .with_duration(started.elapsed().as_millis() as u64);
            }
        };

        let elapsed = started.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let detail = if stdout.is_empty() {
            stderr.clone()
        } else {
            format!("{stdout}\n{stderr}")
        };

        if output.status.success() {
            Verdict::pass(&self.name)
                .with_detail(detail)
                .with_duration(elapsed)
        } else {
            let reason = summarize_errors(&detail, 3);
            let classification = classify_gate_failure(&self.name, &detail);
            Verdict::fail(&self.name, reason)
                .with_detail(detail)
                .with_error_digest(render_failure_classification(&classification))
                .with_duration(elapsed)
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Extract up to `max` error-level diagnostics from stderr for a concise reason.
///
/// Looks for lines starting with "error" (cargo/rustc conventions) and
/// joins them with "; ". Falls back to the first non-empty stderr line.
fn summarize_errors(stderr: &str, max: usize) -> String {
    let errors: Vec<&str> = stderr
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("error:") || t.starts_with("error[")
        })
        .take(max)
        .collect();
    if !errors.is_empty() {
        return errors.join("; ");
    }
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("compilation failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_errors_joins_error_lines() {
        let stderr = "warning: unused var\nerror: bad thing\nerror[E0425]: symbol not found\n";
        let s = summarize_errors(stderr, 3);
        assert!(s.contains("error: bad thing"));
        assert!(s.contains("symbol not found"));
        assert!(!s.contains("unused var"));
    }

    #[test]
    fn summarize_errors_limits_count() {
        let stderr = (0..10)
            .map(|i| format!("error: problem {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let s = summarize_errors(&stderr, 3);
        assert_eq!(s.matches("problem").count(), 3);
    }

    #[test]
    fn summarize_errors_falls_back_to_first_line() {
        let stderr = "no errors here\nbut something went wrong\n";
        let s = summarize_errors(stderr, 3);
        assert_eq!(s, "no errors here");
    }

    #[test]
    fn summarize_errors_handles_empty() {
        let s = summarize_errors("", 3);
        assert_eq!(s, "compilation failed");
    }

    #[test]
    fn cargo_shortcut_names() {
        let g = CompileGate::cargo();
        assert_eq!(g.name(), "compile:cargo");
    }
}
