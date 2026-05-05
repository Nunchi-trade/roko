//! Verify dispatch — runs gate rungs as background tokio tasks and sends
//! results through a channel.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use roko_core::config::GatesConfig;
use roko_core::{Body, Engram, EngramBuilder, Kind, Provenance, Verdict, Verify};
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
        let start = Instant::now();
        let signal = gate_signal(&plan_id, &task_id, rung, &workdir);
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

        let output_preview: String = output.chars().take(200).collect();
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

        let completion = GateCompletion {
            kind: GateCompletionKind::Gate,
            plan_id,
            task_id,
            rung,
            passed,
            failure_kind,
            verdicts: summaries,
            output,
            duration_ms,
        };

        if let Err(e) = gate_tx.send(completion).await {
            error!(err = %e, "failed to send gate completion — channel closed");
            return;
        }
    });
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

fn gate_signal(plan_id: &str, task_id: &str, rung: u32, workdir: &std::path::Path) -> Engram {
    let attempt_sentinel = workdir
        .join(".roko")
        .join("runtime")
        .join("gate-attempts")
        .join(format!(
            "{}-{}-{rung}.seen",
            sanitize_gate_env_segment(plan_id),
            sanitize_gate_env_segment(task_id)
        ));
    let payload = GatePayload::in_dir(workdir)
        .with_label(format!("{plan_id}:{task_id}:rung-{rung}"))
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
        let gate = ShellGate::new("sh", vec!["-c".into(), step.command.clone()])
            .with_name(format!("task-verify:{}:{}", task_id, step.phase))
            .with_timeout_ms(step.timeout_ms);
        let verdict = gate.verify(signal, ctx).await;
        info!(
            task_id = %task_id,
            step = i + 1,
            total_steps = verify_steps.len(),
            phase = %step.phase,
            command = %step.command,
            passed = verdict.passed,
            elapsed_ms = step_start.elapsed().as_millis() as u64,
            "verify step completed"
        );
        verdicts.push(verdict);
    }
    verdicts
}

fn render_output(verdicts: &[Verdict]) -> String {
    verdicts
        .iter()
        .map(|v| {
            let digest = v
                .error_digest
                .as_deref()
                .or(v.detail.as_deref())
                .unwrap_or("");
            if digest.is_empty() {
                format!("{}: {}", v.gate, if v.passed { "pass" } else { "FAIL" })
            } else {
                format!(
                    "{}: {} — {}",
                    v.gate,
                    if v.passed { "pass" } else { "FAIL" },
                    digest
                )
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
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
            } else if fallback == RunnerFailureKind::Unknown {
                RunnerFailureKind::Permanent
            } else {
                fallback
            }
        }
    }
}
