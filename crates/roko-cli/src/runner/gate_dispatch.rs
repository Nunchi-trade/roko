//! Verify dispatch — runs gate rungs as background tokio tasks and sends
//! results through a channel.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use roko_core::config::GatesConfig;
use roko_core::{Body, Engram, EngramBuilder, Kind, Provenance, Verdict, Verify};
use roko_fs::RokoLayout;
use roko_gate::classify_gate_failure;
use roko_gate::rung_dispatch::{GatePipelineBuilder, RungExecutionConfig, RungExecutionInputs};
use roko_gate::{GatePayload, PlanComplexity, ShellGate};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::{Duration, timeout};
use tracing::{error, info};

use crate::task_parser::VerifyStep;

use super::types::{GateCompletion, GateCompletionKind, GateVerdictSummary, RunnerFailureKind};

/// Sentinel rung value for plan-level verification (not a per-task rung).
pub const RUNG_PLAN_VERIFY: u32 = 1000;
/// Sentinel rung value for post-merge regression gates.
pub const RUNG_MERGE: u32 = 1001;

/// Spawn a gate rung as a background task. Sends `GateCompletion` when done.
pub fn spawn_gate(
    plan_id: String,
    task_id: String,
    rung: u32,
    workdir: PathBuf,
    gates_config: GatesConfig,
    complexity: PlanComplexity,
    verify_steps: Vec<VerifyStep>,
    timeout_secs: u64,
    gate_tx: mpsc::Sender<GateCompletion>,
    gate_sem: Arc<Semaphore>,
    target_crates: Vec<String>,
) {
    tokio::spawn(async move {
        let t_wait = Instant::now();
        let Ok(_permit) = gate_sem.acquire_owned().await else {
            return;
        };
        let wait_ms = t_wait.elapsed().as_millis() as u64;
        if wait_ms > 10 {
            info!(
                plan_id = %plan_id,
                task_id = %task_id,
                rung,
                wait_ms,
                "gate semaphore acquired"
            );
        }
        let completion = run_gate_once(
            plan_id,
            task_id,
            rung,
            workdir,
            gates_config,
            complexity,
            verify_steps,
            timeout_secs,
            target_crates,
        )
        .await;

        if let Err(e) = gate_tx.send(completion).await {
            error!(err = %e, "failed to send gate completion — channel closed");
            return;
        }
    });
}

/// Run a gate rung to completion and return its summary.
pub async fn run_gate_once(
    plan_id: String,
    task_id: String,
    rung: u32,
    workdir: PathBuf,
    gates_config: GatesConfig,
    complexity: PlanComplexity,
    verify_steps: Vec<VerifyStep>,
    timeout_secs: u64,
    target_crates: Vec<String>,
) -> GateCompletion {
    let start = Instant::now();
    let signal = gate_signal(&plan_id, &task_id, rung, &workdir, &target_crates);
    let ctx = roko_core::Context::now();
    let limit = Duration::from_secs(timeout_secs.max(1));

    info!(
        plan_id = %plan_id,
        task_id = %task_id,
        rung,
        timeout_secs,
        verify_step_count = verify_steps.len(),
        "gate rung starting"
    );

    let workdir_for_run = workdir.clone();
    let run = async {
        let inputs = RungExecutionInputs::default();
        let config = RungExecutionConfig {
            source_roots: Some(vec![workdir_for_run]),
            timeout_ms: Some(timeout_secs.saturating_mul(1000)),
            ..Default::default()
        };
        let pipeline = if gates_config.has_custom_rungs() {
            GatePipelineBuilder::from_config(&gates_config, complexity)
        } else {
            GatePipelineBuilder::from_config_with_execution(
                &gates_config,
                complexity,
                inputs,
                config,
            )
        };

        let mut verdicts = vec![pipeline.verify(&signal, &ctx).await];
        verdicts.extend(run_verify_steps(&signal, &ctx, &task_id, verify_steps).await);
        verdicts
    };

    let verdicts = match timeout(limit, run).await {
        Ok(verdicts) => verdicts,
        Err(_) => vec![
            Verdict::fail(
                format!("gate-timeout:rung-{rung}"),
                format!("gate timed out after {timeout_secs}s"),
            )
            .with_error_digest(format!(
                "timeout: gate rung {rung} exceeded {timeout_secs}s"
            )),
        ],
    };
    let duration_ms = start.elapsed().as_millis() as u64;

    let passed = verdicts.iter().all(|v| v.passed);
    let output = render_output(&verdicts);
    let failure_kind = (!passed).then(|| classify_failure_kind(&verdicts, &output));

    let summaries: Vec<GateVerdictSummary> = verdicts
        .iter()
        .map(|v| GateVerdictSummary {
            gate_name: v.gate.clone(),
            passed: v.passed,
            summary: v.reason.clone(),
            error_digest: v.error_digest.clone(),
            failure_kind: (!v.passed)
                .then(|| classify_failure_kind(std::slice::from_ref(v), &v.reason)),
        })
        .collect();

    let preview_limit = if passed { 500 } else { 4_000 };
    let output_preview: String = output.chars().take(preview_limit).collect();
    let verdict_names: Vec<&str> = summaries.iter().map(|v| v.gate_name.as_str()).collect();
    info!(
        plan_id = %plan_id,
        task_id = %task_id,
        rung,
        passed,
        duration_ms,
        verdict_count = summaries.len(),
        verdicts = ?verdict_names,
        output_preview = %output_preview,
        "gate completed"
    );

    GateCompletion {
        kind: GateCompletionKind::Gate,
        plan_id,
        task_id,
        rung,
        passed,
        failure_kind,
        verdicts: summaries,
        output,
        duration_ms,
    }
}

/// Spawn plan-level verify steps as a background task.
pub fn spawn_plan_verify(
    plan_id: String,
    workdir: PathBuf,
    verify_steps: Vec<(String, Vec<VerifyStep>)>,
    timeout_secs: u64,
    gate_tx: mpsc::Sender<GateCompletion>,
    gate_sem: Arc<Semaphore>,
) {
    tokio::spawn(async move {
        let t_wait = Instant::now();
        let Ok(_permit) = gate_sem.acquire_owned().await else {
            return;
        };
        let wait_ms = t_wait.elapsed().as_millis() as u64;
        if wait_ms > 10 {
            info!(
                plan_id = %plan_id,
                wait_ms,
                "plan verify semaphore acquired"
            );
        }
        let start = Instant::now();
        let ctx = roko_core::Context::now();
        let limit = Duration::from_secs(timeout_secs.max(1));
        let plan_id_for_run = plan_id.clone();
        let workdir_for_run = workdir.clone();

        let run = async move {
            let mut all = Vec::new();
            for (task_id, steps) in verify_steps {
                let signal = gate_signal(
                    &plan_id_for_run,
                    &task_id,
                    RUNG_PLAN_VERIFY,
                    &workdir_for_run,
                    &[], // plan-level verify runs workspace-wide
                );
                all.extend(run_verify_steps(&signal, &ctx, &task_id, steps).await);
            }
            all
        };

        let verdicts = match timeout(limit, run).await {
            Ok(verdicts) => verdicts,
            Err(_) => vec![
                Verdict::fail(
                    "plan-verify-timeout",
                    format!("plan verify timed out after {timeout_secs}s"),
                )
                .with_error_digest(format!("timeout: plan verify exceeded {timeout_secs}s")),
            ],
        };
        let duration_ms = start.elapsed().as_millis() as u64;
        let passed = verdicts.iter().all(|v| v.passed);
        let output = render_output(&verdicts);
        let failure_kind = (!passed).then(|| classify_failure_kind(&verdicts, &output));
        let summaries = verdicts
            .iter()
            .map(|v| GateVerdictSummary {
                gate_name: v.gate.clone(),
                passed: v.passed,
                summary: v.reason.clone(),
                error_digest: v.error_digest.clone(),
                failure_kind: (!v.passed)
                    .then(|| classify_failure_kind(std::slice::from_ref(v), &v.reason)),
            })
            .collect();

        info!(
            plan_id = %plan_id,
            passed,
            duration_ms,
            "plan verify completed"
        );

        let completion = GateCompletion {
            kind: GateCompletionKind::PlanVerify,
            plan_id,
            task_id: "plan-verify".to_string(),
            rung: RUNG_PLAN_VERIFY,
            passed,
            failure_kind,
            verdicts: summaries,
            output,
            duration_ms,
        };

        if let Err(e) = gate_tx.send(completion).await {
            error!(err = %e, "failed to send plan verify completion — channel closed");
        }
    });
}

fn gate_signal(
    plan_id: &str,
    task_id: &str,
    rung: u32,
    workdir: &std::path::Path,
    target_crates: &[String],
) -> Engram {
    let attempt_sentinel = RokoLayout::for_project(workdir)
        .gate_attempts_dir()
        .join(format!(
            "{}-{}-{rung}.seen",
            sanitize_gate_env_segment(plan_id),
            sanitize_gate_env_segment(task_id)
        ));
    let payload = GatePayload::in_dir(workdir)
        .with_label(format!("{plan_id}:{task_id}:rung-{rung}"))
        .with_target_crates(target_crates.to_vec())
        .with_env("ROKO_GATE_PLAN_ID", plan_id)
        .with_env("ROKO_GATE_TASK_ID", task_id)
        .with_env("ROKO_GATE_RUNG", rung.to_string())
        .with_env(
            "ROKO_GATE_ATTEMPT_SENTINEL",
            attempt_sentinel.to_string_lossy().to_string(),
        );

    EngramBuilder::new(Kind::Task)
        .body(Body::from_json(&payload).unwrap_or_else(|_| Body::empty()))
        .provenance(Provenance::trusted("runner"))
        .tag("plan_id", plan_id.to_string())
        .tag("task_id", task_id.to_string())
        .tag("rung", rung.to_string())
        .build()
}

fn sanitize_gate_env_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

async fn run_verify_steps(
    signal: &Engram,
    ctx: &roko_core::Context,
    task_id: &str,
    verify_steps: Vec<VerifyStep>,
) -> Vec<Verdict> {
    let mut verdicts = Vec::new();
    for (i, step) in verify_steps.iter().enumerate() {
        let step_start = Instant::now();
        let gate = verify_step_gate(task_id, step);
        let verdict = gate.verify(signal, ctx).await;
        info!(
            task_id = %task_id,
            step = i + 1,
            total_steps = verify_steps.len(),
            phase = %step.phase,
            command = %step.command,
            timeout_ms = step.timeout_ms,
            passed = verdict.passed,
            elapsed_ms = step_start.elapsed().as_millis() as u64,
            "verify step completed"
        );
        verdicts.push(verdict);
    }
    verdicts
}

fn verify_step_gate(task_id: &str, step: &VerifyStep) -> ShellGate {
    ShellGate::new(
        "bash",
        vec![
            "-o".into(),
            "pipefail".into(),
            "-c".into(),
            step.command.clone(),
        ],
    )
    .with_name(format!("task-verify:{}:{}", task_id, step.phase))
    .with_timeout_ms(step.timeout_ms)
}

fn render_output(verdicts: &[Verdict]) -> String {
    verdicts
        .iter()
        .map(render_verdict_output)
        .collect::<Vec<_>>()
        .join("; ")
}

fn render_verdict_output(v: &Verdict) -> String {
    let status = if v.passed { "pass" } else { "FAIL" };
    let detail = v.detail.as_deref().unwrap_or("").trim();
    let digest = v.error_digest.as_deref().unwrap_or("").trim();
    let reason = v.reason.trim();

    let message = if v.passed {
        first_non_empty([detail, reason, digest])
    } else if !detail.is_empty() && !digest.is_empty() {
        format!("{detail}\n\nclassification:\n{digest}")
    } else {
        first_non_empty([detail, reason, digest])
    };

    if message.is_empty() {
        format!("{}: {status}", v.gate)
    } else {
        format!("{}: {status} — {message}", v.gate)
    }
}

fn first_non_empty<const N: usize>(values: [&str; N]) -> String {
    values
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

fn classify_failure_kind(verdicts: &[Verdict], output: &str) -> RunnerFailureKind {
    let combined = verdicts
        .iter()
        .filter(|v| !v.passed)
        .map(|v| {
            format!(
                "{}\n{}\n{}",
                v.reason,
                v.detail.as_deref().unwrap_or(""),
                v.error_digest.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = if combined.trim().is_empty() {
        output
    } else {
        &combined
    };
    let classification = classify_gate_failure("runner", text);
    let rendered = serde_json::to_string(&classification).unwrap_or_default();
    let fallback = RunnerFailureKind::from_output(text);
    match classification.recommended_action {
        roko_gate::GateFailureAction::Blocked => RunnerFailureKind::Resource,
        roko_gate::GateFailureAction::NeedsHuman => RunnerFailureKind::Permanent,
        roko_gate::GateFailureAction::NeedsReplan => RunnerFailureKind::Structural,
        roko_gate::GateFailureAction::Retry => {
            if rendered.contains("external_environment") {
                RunnerFailureKind::Transient
            } else {
                match fallback {
                    RunnerFailureKind::Resource | RunnerFailureKind::Transient => fallback,
                    RunnerFailureKind::Permanent
                    | RunnerFailureKind::Structural
                    | RunnerFailureKind::Unknown => RunnerFailureKind::Structural,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_recommended_gate_digest_remains_retryable() {
        let digest = r#"{
  "gate": "task-verify:C01:structural",
  "primary": "unknown",
  "failure_kind": "permanent",
  "retry_policy": {
    "retryable": true,
    "cooldown_secs": 0,
    "include_error_digest": true,
    "generate_reflection": true,
    "regenerate_verify": false
  },
  "summary": "exit code: 1",
  "classes": ["unknown"],
  "compile_errors": [],
  "error_count": 0,
  "warning_count": 0,
  "cargo_fix_candidate": false,
  "agent_retry_needed": true,
  "recommended_action": "retry",
  "replan_candidate": false,
  "blocking_findings": [],
  "duration_ms": 10,
  "raw_excerpt": ""
}"#;
        let verdict =
            Verdict::fail("task-verify:C01:structural", "exit code: 1").with_error_digest(digest);

        let kind = classify_failure_kind(&[verdict], "");

        assert_eq!(kind, RunnerFailureKind::Structural);
        assert!(kind.is_retryable());
    }

    #[test]
    fn failed_gate_output_prefers_command_detail_before_classification() {
        let verdict = Verdict::fail("task-verify:V03:test", "exit code: 1")
            .with_detail("failures:\n    workspace_tests::regression\n")
            .with_error_digest(r#"{"recommended_action":"retry"}"#);

        let rendered = render_output(&[verdict]);

        assert!(rendered.contains("workspace_tests::regression"));
        assert!(rendered.contains("classification:"));
        assert!(
            rendered.find("workspace_tests::regression").unwrap()
                < rendered.find("classification:").unwrap()
        );
    }

    #[tokio::test]
    async fn verify_steps_fail_when_a_piped_command_fails_before_tail() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let signal = gate_signal("plan", "task", 2, tempdir.path(), &[]);
        let ctx = roko_core::Context::now();
        let step = VerifyStep {
            phase: "test".to_string(),
            command: "false | tail -1".to_string(),
            fail_msg: None,
            timeout_ms: 10_000,
        };

        let verdicts = run_verify_steps(&signal, &ctx, "T01", vec![step]).await;

        assert_eq!(verdicts.first().map(|verdict| verdict.passed), Some(false));
    }
}
