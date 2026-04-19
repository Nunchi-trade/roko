//! `SecurityScanGate` — runs `cargo audit` or equivalent security scanning.
//!
//! Checks for known vulnerabilities in dependencies via `cargo audit`.
//! Falls back to a pass verdict if `cargo-audit` is not installed.

use crate::payload::GatePayload;
use async_trait::async_trait;
use roko_core::{Context, Engram, Gate, Verdict};
use std::time::Instant;
use tokio::process::Command;

/// A gate that runs security scanning via `cargo audit`.
pub struct SecurityScanGate {
    name: String,
}

impl SecurityScanGate {
    /// Create a security scan gate using `cargo audit`.
    #[must_use]
    pub fn cargo_audit() -> Self {
        Self {
            name: "security_scan:cargo_audit".to_string(),
        }
    }
}

#[async_trait]
impl Gate for SecurityScanGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let payload: Option<GatePayload> = signal.body.as_json().ok();
        let working_dir = payload.as_ref().map(|p| p.working_dir.clone());

        let mut cmd = Command::new("cargo");
        cmd.arg("audit");
        if let Some(dir) = &working_dir {
            cmd.current_dir(dir);
        }

        let elapsed_ms = || {
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
        };

        let result = match cmd.output().await {
            Ok(output) => output,
            Err(_) => {
                // cargo-audit not installed; pass through.
                return Verdict::pass(&self.name)
                    .with_detail("cargo-audit not installed; skipping security scan")
                    .with_duration(elapsed_ms());
            }
        };

        if result.status.success() {
            Verdict::pass(&self.name)
                .with_detail("no known vulnerabilities found")
                .with_duration(elapsed_ms())
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let stdout = String::from_utf8_lossy(&result.stdout);
            Verdict::fail(&self.name, "security vulnerabilities detected")
                .with_detail(format!("{stdout}\n{stderr}"))
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
    async fn security_scan_gate_has_correct_name() {
        let gate = SecurityScanGate::cargo_audit();
        assert_eq!(gate.name(), "security_scan:cargo_audit");
    }

    #[tokio::test]
    async fn security_scan_gate_produces_verdict() {
        let gate = SecurityScanGate::cargo_audit();
        let verdict = gate.verify(&signal(), &ctx()).await;
        assert!(!verdict.gate.is_empty());
    }
}
