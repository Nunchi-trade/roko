//! Background job runner that auto-executes marketplace jobs.
//!
//! Spawns a background tokio task that polls `.roko/jobs/` for `open` jobs
//! with `auto_execute == true` and dispatches them through the appropriate
//! execution path (research, coding, chain monitor, chain analysis).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use chrono::Utc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use roko_core::MarketplaceJob;

use crate::events::ServerEvent;
use crate::runtime::PlanGenerationResult;
use crate::state::AppState;

#[derive(Debug, Clone)]
struct JobExecutionOutcome {
    summary: String,
    artifacts: Vec<serde_json::Value>,
    gate_results: Vec<serde_json::Value>,
}

impl JobExecutionOutcome {
    fn summary_only(summary: String) -> Self {
        Self {
            summary,
            artifacts: Vec::new(),
            gate_results: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FileSnapshot {
    modified: Option<SystemTime>,
    len: u64,
}

/// Spawn the background job runner. Returns a join handle that runs until
/// the server's cancel token fires.
pub fn start_job_runner(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(run_job_loop(state))
}

/// Main loop: poll for auto-executable jobs and react to `JobCreated` events.
async fn run_job_loop(state: Arc<AppState>) {
    let mut event_rx = state.event_bus.subscribe();
    let poll_interval = tokio::time::Duration::from_secs(5);

    loop {
        tokio::select! {
            _ = state.cancel.cancelled() => {
                info!("job runner shutting down");
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {
                if let Err(err) = poll_and_execute(&state).await {
                    warn!(error = %err, "job runner poll cycle failed");
                }
            }
            result = event_rx.recv() => {
                match result {
                    Ok(envelope) => {
                        if let ServerEvent::JobCreated { ref job } = envelope.payload {
                            // Fast-path: check if the newly created job should auto-execute.
                            if let Ok(parsed) = serde_json::from_value::<MarketplaceJob>(job.clone()) {
                                if parsed.auto_execute && is_open(&parsed) {
                                    let state = Arc::clone(&state);
                                    let job_id = parsed.id.clone();
                                    tokio::spawn(async move {
                                        if let Err(err) = execute_job(&state, &job_id).await {
                                            error!(job_id = %job_id, error = %err, "auto-execute failed");
                                        }
                                    });
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "job runner lagged behind event bus");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// Scan `.roko/jobs/` for open, auto-executable jobs and execute them.
async fn poll_and_execute(state: &AppState) -> anyhow::Result<()> {
    let dir = jobs_dir(&state.workdir);
    if !dir.is_dir() {
        return Ok(());
    }

    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let data = match tokio::fs::read_to_string(&path).await {
            Ok(d) => d,
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to read job file");
                continue;
            }
        };
        let job: MarketplaceJob = match serde_json::from_str(&data) {
            Ok(j) => j,
            Err(err) => {
                tracing::debug!(path = %path.display(), error = %err, "failed to parse job file");
                continue;
            }
        };

        if !job.auto_execute || !is_open(&job) {
            continue;
        }

        // Attempt to claim the job via a lock file.
        let stale_lock_ttl = state.load_roko_config().timeouts.agent_dispatch();
        if !try_claim_lock(&path, stale_lock_ttl).await {
            continue;
        }

        let job_id = job.id.clone();
        info!(job_id = %job_id, job_type = %job.job_type, "auto-executing job");
        if let Err(err) = execute_job(state, &job_id).await {
            error!(job_id = %job_id, error = %err, "job execution failed");
        }
        remove_lock(&path).await;
    }

    Ok(())
}

/// Execute a single job end-to-end: claim -> in_progress -> dispatch -> submit -> complete.
pub async fn execute_job(state: &AppState, job_id: &str) -> anyhow::Result<String> {
    let path = job_path(&state.workdir, job_id);
    let data = tokio::fs::read_to_string(&path).await?;
    let mut job: MarketplaceJob = serde_json::from_str(&data)?;

    // Transition: open -> in_progress
    let prev_status = effective_status(&job);
    job.status = "in_progress".to_string();
    job.assigned_to = "job-runner".to_string();
    job.updated_at = Utc::now().to_rfc3339();
    write_job(&path, &job).await?;
    publish_transition(state, &job, &prev_status);

    // Emit execution started event.
    state.event_bus.publish(ServerEvent::JobExecutionStarted {
        job_id: job_id.to_string(),
        job_type: job.job_type.clone(),
        agent_id: "job-runner".to_string(),
    });

    // Emit initial progress.
    let initial_progress = match job.job_type.as_str() {
        "research" => (0, "starting research"),
        "coding_task" | "coding" => (25, "planning"),
        _ => (0, "starting"),
    };
    state.event_bus.publish(ServerEvent::JobProgress {
        job_id: job_id.to_string(),
        percent: initial_progress.0,
        message: initial_progress.1.to_string(),
    });

    // Dispatch by job type.
    let result = match job.job_type.as_str() {
        "research" => execute_research_job(state, &job).await,
        "coding_task" | "coding" => execute_coding_job(state, &job).await,
        "chain_monitor" => execute_chain_monitor_job(state, &job).await,
        "chain_analysis" => execute_chain_analysis_job(state, &job).await,
        _ => {
            // Generic fallback: use description as prompt.
            let prompt = if job.description.is_empty() {
                job.title.clone()
            } else {
                job.description.clone()
            };
            state
                .runtime
                .run_once(&state.workdir, &prompt)
                .await
                .map(|r| {
                    JobExecutionOutcome::summary_only(
                        r.output_text.unwrap_or_else(|| "completed".to_string()),
                    )
                })
        }
    };

    // Emit midpoint progress for research jobs.
    if job.job_type == "research" && result.is_ok() {
        state.event_bus.publish(ServerEvent::JobProgress {
            job_id: job_id.to_string(),
            percent: 50,
            message: "researching".to_string(),
        });
    }
    if matches!(job.job_type.as_str(), "coding_task" | "coding") && result.is_ok() {
        state.event_bus.publish(ServerEvent::JobProgress {
            job_id: job_id.to_string(),
            percent: 75,
            message: "implementing".to_string(),
        });
    }

    match result {
        Ok(outcome) => {
            let summary = outcome.summary.clone();
            // Emit completion progress.
            state.event_bus.publish(ServerEvent::JobProgress {
                job_id: job_id.to_string(),
                percent: 100,
                message: "complete".to_string(),
            });

            // Transition: in_progress -> submitted -> completed
            let prev = job.status.clone();
            job.status = "submitted".to_string();
            job.submission = Some(serde_json::json!({
                "result_summary": outcome.summary,
                "artifacts": outcome.artifacts,
                "gate_results": outcome.gate_results,
                "submitted_at": Utc::now().to_rfc3339(),
            }));
            job.updated_at = Utc::now().to_rfc3339();
            write_job(&path, &job).await?;
            publish_transition(state, &job, &prev);

            let prev = job.status.clone();
            job.status = "completed".to_string();
            job.evaluation = Some(serde_json::json!({
                "accepted": true,
                "feedback": "auto-evaluated by job runner",
                "evaluated_at": Utc::now().to_rfc3339(),
            }));
            job.updated_at = Utc::now().to_rfc3339();
            write_job(&path, &job).await?;
            publish_transition(state, &job, &prev);

            info!(job_id = %job_id, "job completed successfully");
            Ok(summary)
        }
        Err(err) => {
            // Transition: in_progress -> failed
            let prev = job.status.clone();
            job.status = "failed".to_string();
            job.submission = Some(serde_json::json!({
                "error": err.to_string(),
                "failed_at": Utc::now().to_rfc3339(),
            }));
            job.updated_at = Utc::now().to_rfc3339();
            write_job(&path, &job).await?;
            publish_transition(state, &job, &prev);

            error!(job_id = %job_id, error = %err, "job failed");
            Err(err)
        }
    }
}

/// Execute a research job: build a research prompt and run it.
async fn execute_research_job(
    state: &AppState,
    job: &MarketplaceJob,
) -> anyhow::Result<JobExecutionOutcome> {
    let prompt = format!(
        "Research the following topic and produce a detailed report with citations:\n\n{}",
        job.description
    );
    let result = state.runtime.run_once(&state.workdir, &prompt).await?;
    let summary = result
        .output_text
        .unwrap_or_else(|| "research completed".to_string());

    // Save research output to `.roko/research/{job_id}.md`.
    let research_dir = state.workdir.join(".roko").join("research");
    tokio::fs::create_dir_all(&research_dir).await?;
    let output_path = research_dir.join(format!("{}.md", job.id));
    tokio::fs::write(&output_path, &summary).await?;
    info!(job_id = %job.id, path = %output_path.display(), "saved research output");

    Ok(JobExecutionOutcome {
        summary,
        artifacts: vec![artifact_value(
            &state.workdir,
            &output_path,
            "research_report",
            Some("Research report generated by job runner"),
        )],
        gate_results: vec![gate_result("runtime", true, "research runtime completed")],
    })
}

/// Execute a coding job and return the result summary plus evidence payloads.
async fn execute_coding_job(
    state: &AppState,
    job: &MarketplaceJob,
) -> anyhow::Result<JobExecutionOutcome> {
    let artifact_dir = state
        .workdir
        .join(".roko")
        .join("jobs")
        .join("artifacts")
        .join(&job.id);
    tokio::fs::create_dir_all(&artifact_dir).await?;

    let brief_path = artifact_dir.join("job-brief.md");
    let brief = render_coding_job_brief(job);
    tokio::fs::write(&brief_path, &brief).await?;

    let prd_path = materialize_coding_job_prd(&state.workdir, job, &brief).await?;
    let plan = prepare_coding_plan(state, job, &prd_path).await?;
    let before = snapshot_workspace_files(&state.workdir);

    let mut summaries = Vec::new();
    let mut gate_results = Vec::new();
    for target in &plan.plan_targets {
        let result = state.runtime.run_plan(&state.workdir, target).await?;
        let output = result
            .output_text
            .unwrap_or_else(|| format!("plan {} completed", target.display()));
        if !result.success {
            return Err(anyhow::anyhow!(output));
        }
        if result.gate_results.is_empty() {
            gate_results.extend(parse_gate_results(&output));
        } else {
            gate_results.extend(
                result
                    .gate_results
                    .into_iter()
                    .map(|gate| gate_result(&gate.gate, gate.passed, gate.detail.as_str())),
            );
        }
        summaries.push(output);
    }
    let summary = if summaries.is_empty() {
        "coding task completed without plan output".to_string()
    } else {
        summaries.join("\n\n")
    };

    let result_path = artifact_dir.join("result-summary.md");
    tokio::fs::write(&result_path, &summary).await?;

    let mut artifacts = vec![
        artifact_value(
            &state.workdir,
            &brief_path,
            "job_brief",
            Some("Generated job execution brief"),
        ),
        artifact_value(
            &state.workdir,
            &result_path,
            "result_summary",
            Some("Coding job plan execution summary"),
        ),
        artifact_value(
            &state.workdir,
            &prd_path,
            "prd",
            Some("Materialized PRD brief for coding job planning"),
        ),
    ];
    artifacts.extend(
        plan.artifacts
            .iter()
            .map(|path| artifact_value(&state.workdir, path, "plan", Some("Plan artifact"))),
    );
    artifacts.extend(plan_artifacts(&state.workdir, &job.plan_id));
    artifacts.extend(changed_artifacts(&state.workdir, &before, 25));
    dedupe_artifacts(&mut artifacts);

    if gate_results.is_empty() {
        gate_results.push(gate_result(
            "runtime",
            true,
            "plan execution completed without structured gate output",
        ));
    }

    Ok(JobExecutionOutcome {
        summary,
        artifacts,
        gate_results,
    })
}

/// Execute a chain monitor job: run the triage pipeline on synthetic events.
async fn execute_chain_monitor_job(
    state: &AppState,
    job: &MarketplaceJob,
) -> anyhow::Result<JobExecutionOutcome> {
    use roko_chain::observer::ObservedEvent;
    use roko_chain::triage::{TriageConfig, TriagePipeline};
    use roko_chain::types::LogEntry;

    let mut pipeline = TriagePipeline::new(TriageConfig::default());

    // Generate synthetic events from mock chain data.
    let client = roko_chain::MockChainClient::local();
    for i in 0..5 {
        client.insert_log(LogEntry {
            address: format!("0xmonitor{i:04x}"),
            topics: vec![format!("0xtopic{i:04x}")],
            data: vec![i as u8; 32],
        });
    }

    let events: Vec<ObservedEvent> = (0..5)
        .map(|i| ObservedEvent {
            block_number: 100 + i,
            block_hash: format!("0xblock{}", 100 + i),
            block_timestamp: 1_700_000_000 + i,
            log: LogEntry {
                address: format!("0xmonitor{i:04x}"),
                topics: vec![format!("0xtopic{i:04x}")],
                data: vec![i as u8; 32],
            },
        })
        .collect();

    let results = pipeline.triage_batch(events);

    // Publish progress.
    state.event_bus.publish(ServerEvent::JobTransitioned {
        job_id: job.id.clone(),
        from: "in_progress".to_string(),
        to: "in_progress".to_string(),
        assigned_to: Some("job-runner".to_string()),
    });

    let anomalous_count = results.iter().filter(|r| r.is_anomalous).count();
    let ingest_count = results
        .iter()
        .filter(|r| r.action == roko_chain::TriageAction::IngestKnowledge)
        .count();

    let summary = format!(
        "Chain monitor complete: {} events triaged, {} anomalous, {} routed to knowledge ingestion",
        results.len(),
        anomalous_count,
        ingest_count,
    );

    Ok(JobExecutionOutcome {
        summary,
        artifacts: Vec::new(),
        gate_results: vec![gate_result("chain_triage", true, "chain monitor completed")],
    })
}

/// Execute a chain analysis job: one-shot triage analysis.
async fn execute_chain_analysis_job(
    _state: &AppState,
    job: &MarketplaceJob,
) -> anyhow::Result<JobExecutionOutcome> {
    use roko_chain::observer::ObservedEvent;
    use roko_chain::triage::{TriageConfig, TriagePipeline};
    use roko_chain::types::LogEntry;

    let mut config = TriageConfig::default();
    // Parse any known_contracts from job tags (format: "contract:0xaddr:Label").
    for tag in &job.tags {
        if let Some(rest) = tag.strip_prefix("contract:") {
            if let Some((addr, label)) = rest.split_once(':') {
                config
                    .known_contracts
                    .insert(addr.to_lowercase(), label.to_string());
            }
        }
    }

    let mut pipeline = TriagePipeline::new(config);

    // Generate synthetic analysis events.
    let events: Vec<ObservedEvent> = (0..10)
        .map(|i| ObservedEvent {
            block_number: 200 + i,
            block_hash: format!("0xanalysis{}", 200 + i),
            block_timestamp: 1_700_000_200 + i,
            log: LogEntry {
                address: format!("0xanalysis{i:04x}"),
                topics: vec![format!("0xatopic{i:04x}")],
                data: vec![i as u8; 64],
            },
        })
        .collect();

    let results = pipeline.triage_batch(events);

    let anomalous_count = results.iter().filter(|r| r.is_anomalous).count();
    let curious_count = results.iter().filter(|r| r.curiosity_score >= 0.5).count();
    let rule_matched = results.iter().filter(|r| r.rule_matched).count();

    let summary = format!(
        "Chain analysis complete: {} events analyzed, {} anomalous, {} high-curiosity, {} rule-matched",
        results.len(),
        anomalous_count,
        curious_count,
        rule_matched,
    );

    Ok(JobExecutionOutcome {
        summary,
        artifacts: Vec::new(),
        gate_results: vec![gate_result(
            "chain_triage",
            true,
            "chain analysis completed",
        )],
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn jobs_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("jobs")
}

fn job_path(workdir: &Path, id: &str) -> PathBuf {
    jobs_dir(workdir).join(format!("{id}.json"))
}

/// Resolve the effective status from either `status` or legacy `state` field.
fn effective_status(job: &MarketplaceJob) -> String {
    let s = job.status.trim();
    if s.is_empty() {
        let fallback = job.state.trim();
        if fallback.is_empty() {
            "open".to_string()
        } else {
            fallback.to_ascii_lowercase()
        }
    } else {
        s.to_ascii_lowercase()
    }
}

fn is_open(job: &MarketplaceJob) -> bool {
    let s = effective_status(job);
    s == "open" || s == "pending"
}

async fn write_job(path: &Path, job: &MarketplaceJob) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(job)?;
    tokio::fs::write(path, json).await?;
    Ok(())
}

fn render_coding_job_brief(job: &MarketplaceJob) -> String {
    format!(
        "# Coding Job {}\n\nTitle: {}\n\nType: {}\n\nPlan: {}\n\nPriority: {}\n\nTags: {}\n\n## Description\n\n{}\n",
        job.id,
        job.title,
        job.job_type,
        if job.plan_id.is_empty() {
            "(none)"
        } else {
            job.plan_id.as_str()
        },
        if job.priority.is_empty() {
            "normal"
        } else {
            job.priority.as_str()
        },
        if job.tags.is_empty() {
            "(none)".to_string()
        } else {
            job.tags.join(", ")
        },
        job.description
    )
}

async fn materialize_coding_job_prd(
    workdir: &Path,
    job: &MarketplaceJob,
    brief: &str,
) -> anyhow::Result<PathBuf> {
    let slug = coding_job_slug(job);
    let prd_dir = workdir.join(".roko").join("prd").join("published");
    tokio::fs::create_dir_all(&prd_dir).await?;
    let prd_path = prd_dir.join(format!("{slug}.md"));
    tokio::fs::write(&prd_path, render_coding_job_prd(job, brief)).await?;
    Ok(prd_path)
}

async fn prepare_coding_plan(
    state: &AppState,
    job: &MarketplaceJob,
    prd_path: &Path,
) -> anyhow::Result<PlanGenerationResult> {
    if !job.plan_id.trim().is_empty() {
        let targets = resolve_plan_targets(&state.workdir, &job.plan_id);
        if !targets.is_empty() {
            return Ok(PlanGenerationResult {
                plans_root: targets
                    .first()
                    .and_then(|target| target.parent().map(Path::to_path_buf))
                    .unwrap_or_else(|| state.workdir.join(".roko").join("plans")),
                artifacts: collect_plan_artifact_paths(&targets),
                plan_targets: targets,
            });
        }
        warn!(
            job_id = %job.id,
            plan_id = %job.plan_id,
            "referenced coding job plan was not found; synthesizing fallback plan"
        );
        return synthesize_coding_plan(&state.workdir, job, prd_path).await;
    }

    let slug = coding_job_slug(job);
    match state
        .runtime
        .generate_plan_from_prd(&state.workdir, &slug, prd_path)
        .await
    {
        Ok(mut plan) => {
            if plan.plan_targets.is_empty() {
                plan.plan_targets.push(plan.plans_root.clone());
            }
            Ok(plan)
        }
        Err(err) => {
            warn!(
                job_id = %job.id,
                error = %err,
                "runtime PRD planning unavailable; synthesizing fallback coding plan"
            );
            synthesize_coding_plan(&state.workdir, job, prd_path).await
        }
    }
}

async fn synthesize_coding_plan(
    workdir: &Path,
    job: &MarketplaceJob,
    prd_path: &Path,
) -> anyhow::Result<PlanGenerationResult> {
    let slug = coding_job_slug(job);
    let plans_root = workdir.join(".roko").join("plans");
    let plan_dir = plans_root.join(&slug);
    tokio::fs::create_dir_all(&plan_dir).await?;
    let plan_md = plan_dir.join("plan.md");
    let tasks_toml = plan_dir.join("tasks.toml");
    tokio::fs::write(&plan_md, render_coding_plan_markdown(job, prd_path)).await?;
    tokio::fs::write(&tasks_toml, render_coding_tasks_toml(job, &slug, prd_path)).await?;
    Ok(PlanGenerationResult {
        plans_root,
        plan_targets: vec![plan_dir],
        artifacts: vec![plan_md, tasks_toml],
    })
}

fn resolve_plan_targets(root: &Path, plan_id: &str) -> Vec<PathBuf> {
    let plan_id = plan_id.trim();
    if plan_id.is_empty() {
        return Vec::new();
    }
    let raw = PathBuf::from(plan_id);
    let mut candidates = if raw.is_absolute() {
        vec![raw]
    } else {
        vec![
            root.join(&raw),
            root.join("plans").join(plan_id),
            root.join("plans").join(format!("{plan_id}.md")),
            root.join(".roko").join("plans").join(plan_id),
            root.join(".roko")
                .join("plans")
                .join(format!("{plan_id}.md")),
        ]
    };
    candidates.sort();
    candidates.dedup();
    candidates
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

fn collect_plan_artifact_paths(targets: &[PathBuf]) -> Vec<PathBuf> {
    let mut artifacts = Vec::new();
    for target in targets {
        if target.is_file() {
            artifacts.push(target.clone());
            continue;
        }
        for name in ["plan.md", "tasks.toml"] {
            let path = target.join(name);
            if path.exists() {
                artifacts.push(path);
            }
        }
    }
    artifacts
}

fn render_coding_job_prd(job: &MarketplaceJob, brief: &str) -> String {
    format!(
        "# PRD: {}\n\n## Problem\n\n{}\n\n## Requirements\n\n- REQ-001: Implement the coding job described below.\n- REQ-002: Preserve existing behavior outside the requested scope.\n- REQ-003: Collect changed files and gate results as submission evidence.\n\n## Acceptance Criteria\n\n- The requested code change is implemented in the workspace.\n- Relevant project gates are run and reported.\n- The job submission includes plan, result, gate, and changed-file artifacts.\n\n## Source Job Brief\n\n{}\n",
        job.title,
        if job.description.trim().is_empty() {
            "Complete the requested coding work."
        } else {
            job.description.trim()
        },
        brief
    )
}

fn render_coding_plan_markdown(job: &MarketplaceJob, prd_path: &Path) -> String {
    format!(
        "---\nplan: {}\ntitle: {}\npriority: 0\n---\n\n# {}\n\nSource PRD: `{}`\n\nImplement the marketplace coding job, then run the configured gates and preserve submission evidence.\n",
        coding_job_slug(job),
        toml_escape(&job.title),
        job.title,
        prd_path.display()
    )
}

fn render_coding_tasks_toml(job: &MarketplaceJob, slug: &str, prd_path: &Path) -> String {
    let title = if job.title.trim().is_empty() {
        "Implement coding job"
    } else {
        job.title.trim()
    };
    let description = if job.description.trim().is_empty() {
        "Complete the coding job."
    } else {
        job.description.trim()
    };
    let prd_rel = prd_path
        .strip_prefix(Path::new("."))
        .unwrap_or(prd_path)
        .to_string_lossy();
    format!(
        "[meta]\nplan = \"{}\"\niteration = 1\ntotal = 1\ndone = 0\nstatus = \"ready\"\nmax_parallel = 1\nestimated_total_minutes = 30\n\n[[task]]\nid = \"T1\"\ntitle = \"{}\"\nrole = \"implementer\"\nstatus = \"ready\"\ntier = \"focused\"\nmodel_hint = \"claude-sonnet-4-6\"\nmax_loc = 500\nfiles = []\nallowed_tools = []\ndepends_on = []\nverify = [{{ phase = \"runtime\", command = \"cargo check\", fail_msg = \"cargo check failed\", timeout_ms = 120000 }}]\n\n[task.context]\nread_files = [{{ path = \"{}\" }}]\nrequirements = [\"{}\"]\nanti_patterns = [\"Do not make unrelated refactors.\"]\n",
        toml_escape(slug),
        toml_escape(title),
        toml_escape(&prd_rel),
        toml_escape(description)
    )
}

fn coding_job_slug(job: &MarketplaceJob) -> String {
    let base = if job.id.trim().is_empty() {
        job.title.as_str()
    } else {
        job.id.as_str()
    };
    let slug = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "coding-job".to_string()
    } else if slug
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
    {
        format!("job-{slug}")
    } else {
        format!("job-coding-{slug}")
    }
}

fn toml_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn snapshot_workspace_files(root: &Path) -> BTreeMap<PathBuf, FileSnapshot> {
    let mut out = BTreeMap::new();
    collect_file_snapshots(root, root, &mut out);
    out
}

fn collect_file_snapshots(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, FileSnapshot>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        if should_skip_artifact_path(rel) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_file_snapshots(root, &path, out);
        } else if meta.is_file() {
            out.insert(
                rel.to_path_buf(),
                FileSnapshot {
                    modified: meta.modified().ok(),
                    len: meta.len(),
                },
            );
        }
    }
}

fn changed_artifacts(
    root: &Path,
    before: &BTreeMap<PathBuf, FileSnapshot>,
    limit: usize,
) -> Vec<serde_json::Value> {
    let after = snapshot_workspace_files(root);
    let mut changed = Vec::new();
    for (rel, snap) in after {
        let changed_file = before
            .get(&rel)
            .is_none_or(|old| old.modified != snap.modified || old.len != snap.len);
        if changed_file {
            let path = root.join(&rel);
            changed.push(artifact_value(
                root,
                &path,
                "workspace_change",
                Some("File changed during coding job execution"),
            ));
            if changed.len() >= limit {
                break;
            }
        }
    }
    changed
}

fn should_skip_artifact_path(rel: &Path) -> bool {
    let mut components = rel.components();
    let first = components
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .unwrap_or_default();
    if matches!(first, ".git" | "target" | "node_modules" | ".claude") {
        return true;
    }
    if first == ".roko" {
        let second = components
            .next()
            .and_then(|component| component.as_os_str().to_str())
            .unwrap_or_default();
        return !matches!(second, "jobs" | "plans" | "prd" | "research");
    }
    false
}

fn plan_artifacts(root: &Path, plan_id: &str) -> Vec<serde_json::Value> {
    if plan_id.trim().is_empty() {
        return Vec::new();
    }
    let candidates = [
        root.join("plans").join(plan_id),
        root.join("plans").join(format!("{plan_id}.md")),
        root.join(".roko").join("plans").join(plan_id),
        root.join(".roko")
            .join("plans")
            .join(format!("{plan_id}.md")),
    ];
    candidates
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| artifact_value(root, &path, "plan", Some("Referenced plan artifact")))
        .collect()
}

fn artifact_value(
    root: &Path,
    path: &Path,
    kind: &str,
    description: Option<&str>,
) -> serde_json::Value {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let size = std::fs::metadata(path).map_or(0, |meta| meta.len());
    serde_json::json!({
        "path": rel,
        "kind": kind,
        "size": size,
        "description": description.unwrap_or_default(),
    })
}

fn dedupe_artifacts(artifacts: &mut Vec<serde_json::Value>) {
    let mut seen = std::collections::BTreeSet::new();
    artifacts.retain(|artifact| {
        let key = artifact
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        !key.is_empty() && seen.insert(key)
    });
}

fn parse_gate_results(output: &str) -> Vec<serde_json::Value> {
    let mut gates = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        let passed = lower.contains("[pass]")
            || lower.contains(" pass ")
            || lower.starts_with("pass:")
            || lower.contains("\"passed\":true");
        let failed = lower.contains("[fail]")
            || lower.contains(" fail ")
            || lower.starts_with("fail:")
            || lower.contains("\"passed\":false");
        if !passed && !failed {
            continue;
        }
        let gate = extract_gate_name(trimmed).unwrap_or_else(|| "gate".to_string());
        gates.push(gate_result(&gate, passed && !failed, trimmed));
    }
    gates
}

fn extract_gate_name(line: &str) -> Option<String> {
    let cleaned = line
        .replace("[PASS]", "")
        .replace("[FAIL]", "")
        .replace("[pass]", "")
        .replace("[fail]", "")
        .replace("PASS:", "")
        .replace("FAIL:", "")
        .replace("pass:", "")
        .replace("fail:", "");
    let token = cleaned
        .split(|ch: char| ch == ':' || ch == '-' || ch.is_whitespace())
        .find(|part| !part.trim().is_empty())?;
    Some(
        token
            .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_')
            .to_string(),
    )
    .filter(|value| !value.is_empty())
}

fn gate_result(gate: &str, passed: bool, detail: &str) -> serde_json::Value {
    serde_json::json!({
        "gate": gate,
        "passed": passed,
        "detail": detail,
    })
}

fn publish_transition(state: &AppState, job: &MarketplaceJob, prev_status: &str) {
    state.event_bus.publish(ServerEvent::JobTransitioned {
        job_id: job.id.clone(),
        from: prev_status.to_string(),
        to: job.status.clone(),
        assigned_to: if job.assigned_to.is_empty() {
            None
        } else {
            Some(job.assigned_to.clone())
        },
    });
}

/// Simple file-based lock to prevent concurrent execution of the same job.
///
/// `stale_lock_ttl` controls how old a lock file must be before it's
/// considered stale and reclaimed. Derived from `TimeoutConfig::agent_dispatch`.
async fn try_claim_lock(job_path: &Path, stale_lock_ttl: std::time::Duration) -> bool {
    let lock_path = job_path.with_extension("json.lock");
    if lock_path.exists() {
        if let Ok(meta) = tokio::fs::metadata(&lock_path).await {
            if let Ok(modified) = meta.modified() {
                let age = modified.elapsed().unwrap_or_default();
                if age < stale_lock_ttl {
                    return false;
                }
                // Stale lock — remove and reclaim.
                let _ = tokio::fs::remove_file(&lock_path).await;
            }
        } else {
            return false;
        }
    }
    let pid = std::process::id().to_string();
    tokio::fs::write(&lock_path, pid).await.is_ok()
}

async fn remove_lock(job_path: &Path) {
    let lock_path = job_path.with_extension("json.lock");
    let _ = tokio::fs::remove_file(&lock_path).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_status_prefers_status_field() {
        let job = MarketplaceJob {
            status: "in_progress".into(),
            state: "open".into(),
            ..Default::default()
        };
        assert_eq!(effective_status(&job), "in_progress");
    }

    #[test]
    fn effective_status_falls_back_to_state() {
        let job = MarketplaceJob {
            status: String::new(),
            state: "assigned".into(),
            ..Default::default()
        };
        assert_eq!(effective_status(&job), "assigned");
    }

    #[test]
    fn effective_status_defaults_to_open() {
        let job = MarketplaceJob::default();
        assert_eq!(effective_status(&job), "open");
    }

    #[test]
    fn is_open_detects_open_and_pending() {
        let open = MarketplaceJob {
            status: "open".into(),
            ..Default::default()
        };
        assert!(is_open(&open));

        let pending = MarketplaceJob {
            status: "pending".into(),
            ..Default::default()
        };
        assert!(is_open(&pending));

        let running = MarketplaceJob {
            status: "in_progress".into(),
            ..Default::default()
        };
        assert!(!is_open(&running));
    }
}
