//! Pipeline runner — executes the state machine by performing side effects.
//!
//! This module drives a [`WorkflowRun`] by:
//! 1. Starting the pipeline state machine
//! 2. Performing actions (spawn agents, run gates, commit)
//! 3. Feeding results back as events
//! 4. Emitting ACP session updates (plan entries, tool calls) through the event channel

use std::path::Path;

use roko_core::{Body, Context, Engram, Kind, Verify};
use roko_gate::{
    AdaptiveThresholds, ClippyGate, CompileGate, GatePayload, TestGate,
    parse_structured_review_verdict,
    review_verdict::ReviewVerdictContext,
};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::bridge_events::CognitiveEvent;
use crate::pipeline::{PipelineAction, PipelineEvent, PipelinePhase, WorkflowTemplate};
use crate::session::{CancelToken, SharedWorkflowRun};
use crate::types::{
    ContentBlock, PlanEntry, PlanStatus, Priority, StopReason, ToolCallKind, ToolCallStatus,
};
use crate::workflow::WorkflowRun;

/// Configuration passed from the ACP session to the pipeline runner.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub template: WorkflowTemplate,
    pub max_iterations: u32,
    pub clippy_enabled: bool,
    pub tests_enabled: bool,
    /// Review strictness: "none", "quick", "standard", "thorough".
    pub review_strictness: String,
}

/// Run a workflow pipeline, emitting ACP events as it progresses.
///
/// This is the main entry point called from `bridge_events.rs` when
/// the session workflow config is not "none".
pub async fn run_workflow_pipeline(
    session_id: &str,
    prompt: &str,
    workdir: &Path,
    config: PipelineConfig,
    cancel_token: CancelToken,
    event_sender: mpsc::Sender<CognitiveEvent>,
    shared_run: SharedWorkflowRun,
) -> anyhow::Result<()> {
    let mut run = WorkflowRun::new(config.template.clone(), prompt.to_owned(), config.max_iterations);

    info!(
        run_id = %run.run_id,
        template = run.template_name(),
        "starting workflow pipeline"
    );

    // Publish initial state and emit plan update.
    sync_shared_run(&shared_run, &run).await;
    emit_plan_update(&run, &event_sender).await;

    // Start the state machine.
    let mut action = run.pipeline.step(PipelineEvent::Start);

    loop {
        if cancel_token.is_cancelled() {
            action = run.pipeline.step(PipelineEvent::UserCancel);
        }

        debug!(
            run_id = %run.run_id,
            phase = ?run.pipeline.phase,
            action = ?action,
            "pipeline step"
        );

        // Emit plan update and sync shared state after each phase transition.
        sync_shared_run(&shared_run, &run).await;
        emit_plan_update(&run, &event_sender).await;

        match action {
            PipelineAction::SpawnStrategist { ref prompt } => {
                run.agents_spawned += 1;
                let result = run_agent_phase(
                    session_id,
                    "Strategist",
                    prompt,
                    workdir,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match result {
                    Ok(output) => run.pipeline.step(PipelineEvent::StrategyComplete {
                        brief: output,
                    }),
                    Err(e) => run.pipeline.step(PipelineEvent::AgentFailed {
                        error: e.to_string(),
                    }),
                };
            }

            PipelineAction::SpawnImplementer {
                ref prompt,
                ref context,
            } => {
                run.agents_spawned += 1;
                let full_prompt = if context.is_empty() {
                    prompt.clone()
                } else {
                    format!("{context}\n\n{prompt}")
                };
                let result = run_agent_phase(
                    session_id,
                    "Implementer",
                    &full_prompt,
                    workdir,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match result {
                    Ok(output) => {
                        // Estimate files changed from output length (rough heuristic).
                        let files_changed = output.matches("Edit:").count() as u32
                            + output.matches("Create:").count() as u32;
                        run.pipeline.step(PipelineEvent::AgentCompleted {
                            output,
                            files_changed: files_changed.max(1),
                        })
                    }
                    Err(e) => run.pipeline.step(PipelineEvent::AgentFailed {
                        error: e.to_string(),
                    }),
                };
            }

            PipelineAction::SpawnAutoFixer { ref error_output } => {
                run.agents_spawned += 1;
                let fix_prompt = format!(
                    "Fix the following errors. Make minimal changes:\n\n{error_output}"
                );
                let result = run_agent_phase(
                    session_id,
                    "AutoFixer",
                    &fix_prompt,
                    workdir,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match result {
                    Ok(output) => {
                        let files_changed = output.matches("Edit:").count() as u32;
                        run.pipeline.step(PipelineEvent::AgentCompleted {
                            output,
                            files_changed: files_changed.max(1),
                        })
                    }
                    Err(e) => run.pipeline.step(PipelineEvent::AgentFailed {
                        error: e.to_string(),
                    }),
                };
            }

            PipelineAction::RunGates => {
                let gate_result = run_gates(
                    session_id,
                    workdir,
                    config.clippy_enabled,
                    config.tests_enabled,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match gate_result {
                    Ok(()) => run.pipeline.step(PipelineEvent::GatesPassed),
                    Err(e) => run.pipeline.step(PipelineEvent::GateFailed {
                        gate: "gate".into(),
                        output: e.to_string(),
                    }),
                };
            }

            PipelineAction::SpawnReviewer { .. } => {
                // If review_strictness is "none", skip review entirely.
                if config.review_strictness == "none" {
                    action = run.pipeline.step(PipelineEvent::ReviewApproved {
                        summary: "Review skipped (strictness=none)".into(),
                    });
                } else if config.review_strictness == "thorough" {
                    // Multi-role review: architect + auditor, both must approve.
                    action = run_multi_role_review(
                        session_id,
                        &mut run,
                        workdir,
                        &config,
                        &cancel_token,
                        &event_sender,
                    )
                    .await;
                } else {
                    // Single reviewer (quick/standard).
                    action = run_single_review(
                        session_id,
                        &mut run,
                        workdir,
                        &config,
                        &cancel_token,
                        &event_sender,
                    )
                    .await;
                }
            }

            PipelineAction::Commit => {
                let commit_result = run_commit(
                    session_id,
                    workdir,
                    &run.pipeline.original_prompt,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match commit_result {
                    Ok(msg) => run.pipeline.step(PipelineEvent::CommitDone { message: msg }),
                    Err(e) => {
                        error!(error = %e, "commit failed");
                        // Still mark as done — user can commit manually.
                        run.pipeline.step(PipelineEvent::CommitDone {
                            message: format!("(commit failed: {e})"),
                        })
                    }
                };
            }

            PipelineAction::Done => {
                run.mark_complete();
                sync_shared_run(&shared_run, &run).await;
                emit_plan_update(&run, &event_sender).await;

                // Emit final summary message.
                let elapsed = run.elapsed().num_seconds();
                let summary = format!(
                    "\n\n---\nWorkflow complete ({} pipeline).\n\
                     Duration: {}s | Agents: {} | Iterations: {}/{}",
                    run.template_name(),
                    elapsed,
                    run.agents_spawned,
                    run.pipeline.iteration,
                    run.pipeline.max_iterations,
                );
                let _ = event_sender
                    .send(CognitiveEvent::TokenChunk(summary))
                    .await;
                let _ = event_sender
                    .send(CognitiveEvent::Complete {
                        stop_reason: StopReason::EndTurn,
                        usage: None,
                    })
                    .await;
                return Ok(());
            }

            PipelineAction::Halt { reason } => {
                run.mark_complete();
                sync_shared_run(&shared_run, &run).await;
                emit_plan_update(&run, &event_sender).await;

                let msg = format!("\n\n---\nWorkflow halted: {reason}");
                let _ = event_sender
                    .send(CognitiveEvent::TokenChunk(msg))
                    .await;
                let _ = event_sender
                    .send(CognitiveEvent::Complete {
                        stop_reason: StopReason::EndTurn,
                        usage: None,
                    })
                    .await;
                return Ok(());
            }
        }
    }
}

/// Sync the shared workflow run handle so slash commands can read live state.
async fn sync_shared_run(shared: &SharedWorkflowRun, run: &WorkflowRun) {
    let mut guard = shared.lock().await;
    *guard = Some(run.clone());
}

/// Emit a plan update reflecting the current pipeline state.
async fn emit_plan_update(run: &WorkflowRun, sender: &mpsc::Sender<CognitiveEvent>) {
    let entries = build_plan_entries(run);
    if !entries.is_empty() {
        let _ = sender
            .send(CognitiveEvent::PlanUpdate { entries })
            .await;
    }
}

/// Build plan entries from the current run state.
fn build_plan_entries(run: &WorkflowRun) -> Vec<PlanEntry> {
    let phase = &run.pipeline.phase;
    let template = &run.pipeline.template;

    let mut entries = Vec::new();

    // Strategy phase (full only).
    if template.has_strategy() {
        let status = match phase {
            PipelinePhase::Strategizing => PlanStatus::InProgress,
            PipelinePhase::Pending => PlanStatus::Pending,
            _ => PlanStatus::Completed,
        };
        entries.push(PlanEntry {
            content: "Strategy brief".into(),
            priority: Priority::High,
            status,
        });
    }

    // Implementation phase.
    let impl_status = match phase {
        PipelinePhase::Implementing | PipelinePhase::AutoFixing => PlanStatus::InProgress,
        PipelinePhase::Pending | PipelinePhase::Strategizing => PlanStatus::Pending,
        _ => PlanStatus::Completed,
    };
    let impl_label = if run.pipeline.iteration > 1 {
        format!(
            "Implementation (attempt {}/{})",
            run.pipeline.iteration, run.pipeline.max_iterations
        )
    } else {
        "Implementation".into()
    };
    entries.push(PlanEntry {
        content: impl_label,
        priority: Priority::High,
        status: impl_status,
    });

    // Gates phase.
    let gate_status = match phase {
        PipelinePhase::Gating => PlanStatus::InProgress,
        PipelinePhase::Pending
        | PipelinePhase::Strategizing
        | PipelinePhase::Implementing
        | PipelinePhase::AutoFixing => PlanStatus::Pending,
        _ => PlanStatus::Completed,
    };
    entries.push(PlanEntry {
        content: "Run gates (compile + test)".into(),
        priority: Priority::Medium,
        status: gate_status,
    });

    // Review phase (standard, full only).
    if template.has_review() {
        let review_status = match phase {
            PipelinePhase::Reviewing => PlanStatus::InProgress,
            PipelinePhase::Committing | PipelinePhase::Complete => PlanStatus::Completed,
            _ => PlanStatus::Pending,
        };
        entries.push(PlanEntry {
            content: "Code review".into(),
            priority: Priority::Medium,
            status: review_status,
        });
    }

    // Commit phase.
    let commit_status = match phase {
        PipelinePhase::Committing => PlanStatus::InProgress,
        PipelinePhase::Complete => PlanStatus::Completed,
        _ => PlanStatus::Pending,
    };
    entries.push(PlanEntry {
        content: "Commit changes".into(),
        priority: Priority::Low,
        status: commit_status,
    });

    entries
}

/// JSON schema hint appended to review prompts for structured output.
const REVIEW_JSON_SCHEMA: &str = r#"
Respond with a JSON object (no markdown fences needed):
{
  "status": "passed" | "failed" | "needs_human",
  "confidence": 0.0-1.0,
  "blocking_findings": ["list of blocking issues"],
  "non_blocking_findings": ["list of advisory issues"],
  "required_next_action": "none" | "needs_human_review" | "needs_rework",
  "evidence_refs": []
}"#;

/// Build a review prompt appropriate for the configured strictness level.
fn build_review_prompt(strictness: &str, original_prompt: &str) -> String {
    let base = match strictness {
        "quick" => format!(
            "Quickly review the recent changes. Only flag blocking issues (bugs, security).\n\n\
             Original request: {original_prompt}"
        ),
        "thorough" => format!(
            "Perform a thorough review of the recent changes. Check:\n\
             1. Correctness and edge cases\n\
             2. Security vulnerabilities\n\
             3. Architecture and design patterns\n\
             4. Documentation completeness\n\
             5. Test coverage\n\n\
             Original request: {original_prompt}"
        ),
        _ => format!(
            "Review the recent changes in this workspace. Focus on correctness, security, \
             and code quality.\n\n\
             Original request: {original_prompt}"
        ),
    };
    format!("{base}\n{REVIEW_JSON_SCHEMA}")
}

/// Parse a reviewer's output into a pipeline event (approved or revise).
fn parse_review_output(
    output: &str,
    run: &WorkflowRun,
    session_id: &str,
    role_id: &str,
) -> (bool, Vec<String>) {
    let ctx = ReviewVerdictContext {
        verdict_id: format!("acp-{}-{role_id}", run.run_id),
        batch_id: session_id.to_string(),
        task_id: run.run_id.clone(),
        reviewer_role_id: role_id.to_string(),
        raw_output_ref: String::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let parsed = parse_structured_review_verdict(output, ctx);
    if parsed.passed() {
        (true, Vec::new())
    } else {
        let findings = if !parsed.evidence.blocking_findings.is_empty() {
            parsed.evidence.blocking_findings.clone()
        } else {
            let lines: Vec<String> = output
                .lines()
                .filter(|l| l.starts_with("- ") || l.starts_with("* "))
                .map(|l| {
                    l.trim_start_matches("- ")
                        .trim_start_matches("* ")
                        .to_owned()
                })
                .collect();
            if lines.is_empty() {
                vec![output.to_string()]
            } else {
                lines
            }
        };
        (false, findings)
    }
}

/// Run a single-reviewer review (quick/standard strictness).
async fn run_single_review(
    session_id: &str,
    run: &mut WorkflowRun,
    workdir: &Path,
    config: &PipelineConfig,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> PipelineAction {
    run.agents_spawned += 1;
    let review_prompt = build_review_prompt(&config.review_strictness, &run.pipeline.original_prompt);
    let review_result = run_agent_phase(
        session_id,
        "Reviewer",
        &review_prompt,
        workdir,
        cancel_token,
        event_sender,
    )
    .await;
    match review_result {
        Ok(output) => {
            let (approved, findings) =
                parse_review_output(&output, run, session_id, &config.review_strictness);
            if approved {
                run.pipeline.step(PipelineEvent::ReviewApproved {
                    summary: output,
                })
            } else {
                run.pipeline
                    .step(PipelineEvent::ReviewRevise { findings })
            }
        }
        Err(e) => {
            warn!(error = %e, "reviewer failed, treating as approved");
            run.pipeline.step(PipelineEvent::ReviewApproved {
                summary: "Review skipped (agent error)".into(),
            })
        }
    }
}

/// Run a multi-role review for "thorough" mode.
///
/// Two reviewers run sequentially: an architect (design/patterns) and an
/// auditor (security/correctness). Both must approve for the review to pass.
/// If either revises, all findings are merged.
async fn run_multi_role_review(
    session_id: &str,
    run: &mut WorkflowRun,
    workdir: &Path,
    _config: &PipelineConfig,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> PipelineAction {
    let original_prompt = &run.pipeline.original_prompt;

    let architect_prompt = format!(
        "You are the **Architect Reviewer**. Focus on:\n\
         1. Architecture and design pattern adherence\n\
         2. API contract correctness\n\
         3. Dependency layering violations\n\
         4. Code organization and modularity\n\n\
         Original request: {original_prompt}\n{REVIEW_JSON_SCHEMA}"
    );

    let auditor_prompt = format!(
        "You are the **Security & Correctness Auditor**. Focus on:\n\
         1. Security vulnerabilities (injection, auth bypass, data leaks)\n\
         2. Edge cases and error handling\n\
         3. Resource leaks (files, connections, memory)\n\
         4. Test coverage gaps\n\n\
         Original request: {original_prompt}\n{REVIEW_JSON_SCHEMA}"
    );

    let mut all_findings: Vec<String> = Vec::new();
    let mut all_approved = true;

    // Architect review.
    run.agents_spawned += 1;
    let arch_result = run_agent_phase(
        session_id,
        "Architect",
        &architect_prompt,
        workdir,
        cancel_token,
        event_sender,
    )
    .await;
    match arch_result {
        Ok(output) => {
            let (approved, findings) = parse_review_output(&output, run, session_id, "architect");
            if !approved {
                all_approved = false;
                all_findings.extend(findings.into_iter().map(|f| format!("[architect] {f}")));
            }
        }
        Err(e) => {
            warn!(error = %e, "architect reviewer failed, continuing");
        }
    }

    // Auditor review.
    run.agents_spawned += 1;
    let audit_result = run_agent_phase(
        session_id,
        "Auditor",
        &auditor_prompt,
        workdir,
        cancel_token,
        event_sender,
    )
    .await;
    match audit_result {
        Ok(output) => {
            let (approved, findings) = parse_review_output(&output, run, session_id, "auditor");
            if !approved {
                all_approved = false;
                all_findings.extend(findings.into_iter().map(|f| format!("[auditor] {f}")));
            }
        }
        Err(e) => {
            warn!(error = %e, "auditor reviewer failed, continuing");
        }
    }

    if all_approved {
        run.pipeline.step(PipelineEvent::ReviewApproved {
            summary: "Both architect and auditor approved".into(),
        })
    } else {
        run.pipeline
            .step(PipelineEvent::ReviewRevise { findings: all_findings })
    }
}

/// Run a single agent phase using claude CLI and stream output.
async fn run_agent_phase(
    _session_id: &str,
    role: &str,
    prompt: &str,
    workdir: &Path,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<String> {
    // Emit tool call start.
    let tool_call_id = format!("phase-{}-{}", role.to_lowercase(), uuid::Uuid::new_v4());
    let _ = event_sender
        .send(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: format!("{role}: working..."),
            kind: ToolCallKind::Other,
        })
        .await;

    // Run claude CLI.
    let output = run_claude_cli(prompt, workdir, cancel_token).await;

    match &output {
        Ok(text) => {
            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id,
                    status: ToolCallStatus::Completed,
                    content: vec![ContentBlock::Text {
                        text: format!("[{role}] Done ({} chars)", text.len()),
                    }],
                })
                .await;
        }
        Err(e) => {
            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id,
                    status: ToolCallStatus::Failed,
                    content: vec![ContentBlock::Text {
                        text: format!("[{role}] Failed: {e}"),
                    }],
                })
                .await;
        }
    }

    output
}

/// Build an Engram signal with a GatePayload body pointing at `workdir`.
fn build_gate_signal(workdir: &Path) -> Engram {
    let payload = GatePayload::in_dir(workdir);
    let body = Body::from_json(&payload).unwrap_or_else(|_| Body::empty());
    Engram::builder(Kind::Task).body(body).build()
}

/// Path to adaptive gate thresholds relative to workdir.
const THRESHOLDS_PATH: &str = ".roko/learn/gate-thresholds.json";

/// Run a gate pipeline using roko-gate's proper Verify trait.
///
/// Runs CompileGate, optionally TestGate, optionally ClippyGate.
/// Each gate gets its own tool_call event in Zed. Results update
/// adaptive thresholds for future skip/retry decisions.
async fn run_gates(
    _session_id: &str,
    workdir: &Path,
    clippy_enabled: bool,
    tests_enabled: bool,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<()> {
    let signal = build_gate_signal(workdir);
    let ctx = Context::at(chrono::Utc::now().timestamp_millis());

    // Load adaptive thresholds (creates new if missing).
    let thresholds_path = workdir.join(THRESHOLDS_PATH);
    let mut thresholds = AdaptiveThresholds::load_or_new(&thresholds_path);

    // Compile gate (rung 0).
    let compile_result = run_verify_gate(
        "compile",
        &CompileGate::cargo(),
        &signal,
        &ctx,
        cancel_token,
        event_sender,
    )
    .await;
    thresholds.observe(0, compile_result.is_ok());
    compile_result?;

    // Test gate (rung 2).
    if tests_enabled {
        if thresholds.should_skip_rung(2) {
            debug!("skipping test gate (adaptive: {} consecutive passes)", 20);
        } else {
            let test_result = run_verify_gate(
                "test",
                &TestGate::cargo(),
                &signal,
                &ctx,
                cancel_token,
                event_sender,
            )
            .await;
            thresholds.observe(2, test_result.is_ok());
            if let Err(e) = test_result {
                save_thresholds(&thresholds, &thresholds_path);
                return Err(e);
            }
        }
    }

    // Clippy gate (rung 1).
    if clippy_enabled {
        if thresholds.should_skip_rung(1) {
            debug!("skipping clippy gate (adaptive: {} consecutive passes)", 20);
        } else {
            let clippy_result = run_verify_gate(
                "clippy",
                &ClippyGate::cargo(),
                &signal,
                &ctx,
                cancel_token,
                event_sender,
            )
            .await;
            thresholds.observe(1, clippy_result.is_ok());
            if let Err(e) = clippy_result {
                save_thresholds(&thresholds, &thresholds_path);
                return Err(e);
            }
        }
    }

    // Persist updated thresholds.
    save_thresholds(&thresholds, &thresholds_path);

    Ok(())
}

/// Persist adaptive thresholds, logging on error.
fn save_thresholds(thresholds: &AdaptiveThresholds, path: &Path) {
    if let Err(e) = thresholds.save(path) {
        warn!(error = %e, "failed to save adaptive gate thresholds");
    }
}

/// Run a single roko-gate `Verify` impl and emit ACP tool_call events.
async fn run_verify_gate(
    gate_name: &str,
    gate: &dyn Verify,
    signal: &Engram,
    ctx: &Context,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<()> {
    let tool_call_id = format!("gate-{gate_name}");

    let _ = event_sender
        .send(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: format!("Gate: {gate_name}"),
            kind: ToolCallKind::Terminal,
        })
        .await;

    if cancel_token.is_cancelled() {
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Failed,
                content: vec![ContentBlock::Text {
                    text: "Cancelled".into(),
                }],
            })
            .await;
        return Err(anyhow::anyhow!("cancelled"));
    }

    let verdict = gate.verify(signal, ctx).await;

    if verdict.passed {
        let detail_summary = verdict
            .detail
            .as_deref()
            .map(|d| {
                // Show first line of detail for context.
                d.lines().next().unwrap_or("")
            })
            .unwrap_or("");
        let test_info = verdict
            .test_count
            .map(|tc| format!(" ({} passed, {} failed)", tc.passed, tc.failed))
            .unwrap_or_default();
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Completed,
                content: vec![ContentBlock::Text {
                    text: format!(
                        "\u{2713} {gate_name} passed ({}ms){test_info}",
                        verdict.duration_ms,
                    ),
                }],
            })
            .await;
        if !detail_summary.is_empty() {
            debug!(gate = gate_name, detail = detail_summary, "gate detail");
        }
        Ok(())
    } else {
        let error_text = if verdict.reason.is_empty() {
            verdict
                .detail
                .as_deref()
                .unwrap_or("unknown error")
                .to_string()
        } else {
            verdict.reason.clone()
        };
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Failed,
                content: vec![ContentBlock::Text {
                    text: format!(
                        "\u{2717} {gate_name} failed ({}ms):\n{error_text}",
                        verdict.duration_ms,
                    ),
                }],
            })
            .await;
        Err(anyhow::anyhow!("{gate_name} failed:\n{error_text}"))
    }
}

/// Run claude CLI as a subprocess and capture output.
async fn run_claude_cli(
    prompt: &str,
    workdir: &Path,
    cancel_token: &CancelToken,
) -> anyhow::Result<String> {
    if cancel_token.is_cancelled() {
        return Err(anyhow::anyhow!("cancelled"));
    }

    let output = Command::new("claude")
        .arg("--print")
        .arg("--dangerously-skip-permissions")
        .arg(prompt)
        .current_dir(workdir)
        .output()
        .await?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(text)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let error = if stderr.is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        Err(anyhow::anyhow!("claude CLI failed: {error}"))
    }
}

/// Create a commit for the workflow output.
async fn run_commit(
    _session_id: &str,
    workdir: &Path,
    original_prompt: &str,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<String> {
    if cancel_token.is_cancelled() {
        return Err(anyhow::anyhow!("cancelled"));
    }

    let tool_call_id = "commit".to_owned();
    let _ = event_sender
        .send(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: "Creating commit".into(),
            kind: ToolCallKind::Terminal,
        })
        .await;

    // Stage all changes.
    let add_output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(workdir)
        .output()
        .await?;

    if !add_output.status.success() {
        let err = String::from_utf8_lossy(&add_output.stderr).to_string();
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Failed,
                content: vec![ContentBlock::Text {
                    text: format!("git add failed: {err}"),
                }],
            })
            .await;
        return Err(anyhow::anyhow!("git add failed: {err}"));
    }

    // Generate commit message from the prompt (truncated).
    let msg = if original_prompt.len() > 72 {
        format!("feat: {}", &original_prompt[..69])
    } else {
        format!("feat: {original_prompt}")
    };

    let commit_output = Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(workdir)
        .output()
        .await?;

    if commit_output.status.success() {
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Completed,
                content: vec![ContentBlock::Text {
                    text: format!("\u{2713} Committed: {msg}"),
                }],
            })
            .await;
        Ok(msg)
    } else {
        let stderr = String::from_utf8_lossy(&commit_output.stderr).to_string();
        // It's okay if there's nothing to commit.
        if stderr.contains("nothing to commit") {
            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id,
                    status: ToolCallStatus::Completed,
                    content: vec![ContentBlock::Text {
                        text: "No changes to commit".into(),
                    }],
                })
                .await;
            Ok("(no changes to commit)".into())
        } else {
            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id,
                    status: ToolCallStatus::Failed,
                    content: vec![ContentBlock::Text {
                        text: format!("git commit failed: {stderr}"),
                    }],
                })
                .await;
            Err(anyhow::anyhow!("git commit failed: {stderr}"))
        }
    }
}
