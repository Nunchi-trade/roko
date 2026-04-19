//! `FormatCheckGate` — verifies code formatting (rustfmt, prettier, etc.).
//!
//! Stub gate that currently passes through. Designed to run `rustfmt --check`
//! or equivalent formatters and fail when files are not properly formatted.

use crate::payload::GatePayload;
use async_trait::async_trait;
use roko_core::{Context, Engram, Gate, Verdict};
use std::time::Instant;
use tokio::process::Command;

/// A gate that checks code formatting via `cargo fmt --check`.
pub struct FormatCheckGate {
    name: String,
}

impl FormatCheckGate {
    /// Create a format-check gate using `cargo fmt`.
    #[must_use]
    pub fn cargo() -> Self {
        Self {
            name: "format_check:cargo".to_string(),
        }
    }
}

#[async_trait]
impl Gate for FormatCheckGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let payload: Option<GatePayload> = signal.body.as_json().ok();
        let working_dir = payload.as_ref().map(|p| p.working_dir.clone());

        let mut cmd = Command::new("cargo");
        cmd.args(["fmt", "--check"]);
        if let Some(dir) = &working_dir {
            cmd.current_dir(dir);
        }

        let elapsed_ms = || {
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
        };

        let result = match cmd.output().await {
            Ok(output) => output,
            Err(e) => {
                return Verdict::fail(&self.name, format!("failed to run cargo fmt: {e}"))
                    .with_duration(elapsed_ms());
            }
        };

        if result.status.success() {
            Verdict::pass(&self.name)
                .with_detail("all files properly formatted")
                .with_duration(elapsed_ms())
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let stdout = String::from_utf8_lossy(&result.stdout);
            let detail = if stdout.is_empty() {
                stderr.to_string()
            } else {
                format!("{stdout}\n{stderr}")
            };
            Verdict::fail(&self.name, "formatting issues detected")
                .with_detail(detail)
                .with_duration(elapsed_ms())
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

    fn signal() -> Engram {
        Engram::builder(Kind::Task).body(Body::empty()).build()
    }

    fn ctx() -> Context {
        Context::at(0)
    }

    #[tokio::test]
    async fn format_check_gate_has_correct_name() {
        let gate = FormatCheckGate::cargo();
        assert_eq!(gate.name(), "format_check:cargo");
    }

    #[tokio::test]
    async fn format_check_gate_produces_verdict() {
        let gate = FormatCheckGate::cargo();
        let verdict = gate.verify(&signal(), &ctx()).await;
        // The verdict depends on the actual project state, so just check it produces one.
        assert!(!verdict.gate.is_empty());
    }
}
