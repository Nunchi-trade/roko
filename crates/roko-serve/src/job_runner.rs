//! Background job runner that auto-executes marketplace jobs.
//!
//! Spawns a background tokio task that polls `.roko/jobs/` for `open` jobs
//! with `auto_execute == true` and dispatches them through the appropriate
//! execution path (research, coding, chain monitor, chain analysis).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use roko_core::MarketplaceJob;

use crate::events::ServerEvent;
use crate::state::AppState;

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
                warn!(path = %path.display(), error = %err, "failed to parse job file");
                continue;
            }
        };

        if !job.auto_execute || !is_open(&job) {
            continue;
        }

        // Attempt to claim the job via a lock file.
        if !try_claim_lock(&path).await {
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
                .map(|r| r.output_text.unwrap_or_else(|| "completed".to_string()))
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
        Ok(summary) => {
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
                "result_summary": summary,
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
) -> anyhow::Result<String> {
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

    Ok(summary)
}

/// Execute a coding job: use plan_id if available, otherwise description.
async fn execute_coding_job(
    state: &AppState,
    job: &MarketplaceJob,
) -> anyhow::Result<String> {
    let prompt = if job.plan_id.is_empty() {
        job.description.clone()
    } else {
        format!(
            "Execute plan '{}' in the current workspace",
            job.plan_id
        )
    };
    let result = state.runtime.run_once(&state.workdir, &prompt).await?;
    Ok(result
        .output_text
        .unwrap_or_else(|| "coding task completed".to_string()))
}

/// Execute a chain monitor job: run the triage pipeline on synthetic events.
async fn execute_chain_monitor_job(
    state: &AppState,
    job: &MarketplaceJob,
) -> anyhow::Result<String> {
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

    Ok(summary)
}

/// Execute a chain analysis job: one-shot triage analysis.
async fn execute_chain_analysis_job(
    _state: &AppState,
    job: &MarketplaceJob,
) -> anyhow::Result<String> {
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
    let curious_count = results
        .iter()
        .filter(|r| r.curiosity_score >= 0.5)
        .count();
    let rule_matched = results.iter().filter(|r| r.rule_matched).count();

    let summary = format!(
        "Chain analysis complete: {} events analyzed, {} anomalous, {} high-curiosity, {} rule-matched",
        results.len(),
        anomalous_count,
        curious_count,
        rule_matched,
    );

    Ok(summary)
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
async fn try_claim_lock(job_path: &Path) -> bool {
    let lock_path = job_path.with_extension("json.lock");
    if lock_path.exists() {
        // Check if the lock is stale (older than 10 minutes).
        if let Ok(meta) = tokio::fs::metadata(&lock_path).await {
            if let Ok(modified) = meta.modified() {
                let age = modified.elapsed().unwrap_or_default();
                if age < std::time::Duration::from_secs(600) {
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
