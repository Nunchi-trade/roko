//! `ClippyGate` — Rung 0.5 of the 6-rung verification ladder (§10.6).
//!
//! Runs the language-appropriate linter (`cargo clippy -- -D warnings` on
//! Cargo, `npm run lint` on Npm, `go vet` on Go, etc.) and treats any
//! non-zero exit as a failure. Designed to slot before [`TestGate`] in a
//! short-circuit gate pipeline so cheap lint failures preempt expensive
//! test runs.

use crate::compile_errors::{classify_gate_failure, render_failure_classification};
use crate::payload::{BuildSystem, GatePayload};
use async_trait::async_trait;
use roko_core::{Context, Engram, Verify, Verdict};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

/// Rung 0.5 gate: lint check.
pub struct ClippyGate {
    build_system: BuildSystem,
    extra_args: Vec<String>,
    timeout_ms: u64,
    name: String,
}

impl ClippyGate {
    /// Construct a lint gate for `build_system`.
    #[must_use]
    pub fn new(build_system: BuildSystem) -> Self {
        Self {
            build_system,
            extra_args: Vec::new(),
            timeout_ms: 5 * 60 * 1000, // 5 minutes
            name: format!("clippy:{}", build_system.program()),
        }
    }

    /// Shortcut: a cargo-clippy gate.
    #[must_use]
    pub fn cargo() -> Self {
        Self::new(BuildSystem::Cargo)
    }

    /// Append arguments (inserted after the default `lint_args`).
    #[must_use]
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args.extend(args);
        self
    }

    /// Override the timeout in milliseconds (default: 5 minutes).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
}

impl roko_core::Cell for ClippyGate {
    fn cell_id(&self) -> &str { "clippy-gate" }
    fn cell_name(&self) -> &str { "ClippyGate" }
    fn protocols(&self) -> &[&str] { &["Verify"] }
}

#[async_trait]
impl Verify for ClippyGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let payload: GatePayload = match signal.body.as_json() {
            Ok(p) => p,
            Err(e) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Verdict::fail(&self.name, format!("signal body is not a GatePayload: {e}"))
                    .with_duration(elapsed);
            }
        };

        // For cargo, `lint_args` already embeds `-- -D warnings`; splice
        // extra_args before the `--` sentinel so they apply to the
        // invocation, not to clippy itself.
        let base: Vec<&str> = self.build_system.lint_args().to_vec();
        let mut cmd = Command::new(self.build_system.program());
        let dash_idx = base.iter().position(|a| *a == "--");
        if let Some(idx) = dash_idx {
            for arg in &base[..idx] {
                cmd.arg(arg);
            }
            for arg in &self.extra_args {
                cmd.arg(arg);
            }
            for arg in &base[idx..] {
                cmd.arg(arg);
            }
        } else {
            for arg in &base {
                cmd.arg(arg);
            }
            for arg in &self.extra_args {
                cmd.arg(arg);
            }
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
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Verdict::fail(&self.name, format!("spawn failed: {e}"))
                    .with_duration(elapsed);
            }
            Err(_) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Verdict::fail(
                    &self.name,
                    format!("timed out after {} ms", self.timeout_ms),
                )
                .with_duration(elapsed);
            }
        };

        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
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
            let reason = summarize_lint_issues(&detail, 3);
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

fn summarize_lint_issues(stderr: &str, max: usize) -> String {
    let issues: Vec<&str> = stderr
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("warning:") || t.starts_with("error:") || t.starts_with("error[")
        })
        .take(max)
        .collect();
    if !issues.is_empty() {
        return issues.join("; ");
    }
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("lint failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_joins_issues() {
        let err = "warning: unused var\nerror: missing field\nwarning: dead code";
        let s = summarize_lint_issues(err, 3);
        assert!(s.contains("unused var"));
        assert!(s.contains("missing field"));
    }

    #[test]
    fn summarize_limits_count() {
        let err = (0..10)
            .map(|i| format!("warning: issue {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let s = summarize_lint_issues(&err, 2);
        assert_eq!(s.matches("issue").count(), 2);
    }

    #[test]
    fn summarize_falls_back_to_first_line() {
        let err = "nothing structured here";
        assert_eq!(summarize_lint_issues(err, 3), "nothing structured here");
    }

    #[test]
    fn summarize_handles_empty_input() {
        assert_eq!(summarize_lint_issues("", 3), "lint failed");
    }

    #[test]
    fn cargo_shortcut_name() {
        let g = ClippyGate::cargo();
        assert_eq!(g.name(), "clippy:cargo");
    }

    #[test]
    fn builder_chaining() {
        let g = ClippyGate::new(BuildSystem::Cargo)
            .with_extra_args(vec!["--features".into(), "ci".into()])
            .with_timeout_ms(120_000);
        assert_eq!(g.timeout_ms, 120_000);
    }
}
