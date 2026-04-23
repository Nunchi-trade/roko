//! File-based tests for job operations exercised by `roko job` subcommands.
//!
//! These tests verify job file I/O and status logic without spawning the full
//! CLI binary or running an HTTP server.

use roko_core::{JobStatus, MarketplaceJob};
use tempfile::tempdir;

/// Write a job to `.roko/jobs/{id}.json`.
fn write_job(workdir: &std::path::Path, job: &MarketplaceJob) {
    let dir = workdir.join(".roko").join("jobs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.json", job.id));
    std::fs::write(&path, serde_json::to_string_pretty(job).unwrap()).unwrap();
}

/// Read a job from `.roko/jobs/{id}.json`.
fn read_job(workdir: &std::path::Path, id: &str) -> MarketplaceJob {
    let path = workdir
        .join(".roko")
        .join("jobs")
        .join(format!("{id}.json"));
    let data = std::fs::read_to_string(&path).unwrap();
    serde_json::from_str(&data).unwrap()
}

/// List all jobs in `.roko/jobs/`.
fn list_jobs(workdir: &std::path::Path) -> Vec<MarketplaceJob> {
    let dir = workdir.join(".roko").join("jobs");
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut jobs = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let data = std::fs::read_to_string(&path).unwrap();
        if let Ok(job) = serde_json::from_str::<MarketplaceJob>(&data) {
            jobs.push(job);
        }
    }
    jobs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn job_list_empty() {
    let dir = tempdir().expect("tempdir");
    let jobs = list_jobs(dir.path());
    assert!(jobs.is_empty(), "expected no jobs, got {}", jobs.len());
}

#[test]
fn job_create_and_show() {
    let dir = tempdir().expect("tempdir");

    let job = MarketplaceJob {
        id: "test-job-1".into(),
        title: "Implement feature X".into(),
        description: "Add feature X to the codebase.".into(),
        job_type: "coding_task".into(),
        status: "open".into(),
        posted_by: "operator".into(),
        priority: "high".into(),
        tags: vec!["coding".into(), "feature".into()],
        plan_id: "plan-99".into(),
        auto_execute: false,
        ..Default::default()
    };
    write_job(dir.path(), &job);

    // Read it back and verify fields.
    let loaded = read_job(dir.path(), "test-job-1");
    assert_eq!(loaded.id, "test-job-1");
    assert_eq!(loaded.title, "Implement feature X");
    assert_eq!(loaded.description, "Add feature X to the codebase.");
    assert_eq!(loaded.job_type, "coding_task");
    assert_eq!(loaded.status, "open");
    assert_eq!(loaded.posted_by, "operator");
    assert_eq!(loaded.priority, "high");
    assert_eq!(loaded.tags, vec!["coding", "feature"]);
    assert_eq!(loaded.plan_id, "plan-99");
    assert!(!loaded.auto_execute);

    // List should return exactly one job.
    let jobs = list_jobs(dir.path());
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "test-job-1");
}

#[test]
fn job_cancel_terminal_fails() {
    // Verify that a completed job cannot be cancelled via status transitions.
    let completed = JobStatus::Completed;
    assert!(completed.is_terminal());
    assert!(completed.valid_transitions().is_empty());

    // Similarly for failed and cancelled.
    assert!(JobStatus::Failed.is_terminal());
    assert!(JobStatus::Failed.valid_transitions().is_empty());
    assert!(JobStatus::Cancelled.is_terminal());
    assert!(JobStatus::Cancelled.valid_transitions().is_empty());
}

#[test]
fn job_status_lifecycle_via_enum() {
    // Open can transition to assigned, in_progress, cancelled.
    let open = JobStatus::Open;
    assert!(!open.is_terminal());
    let transitions = open.valid_transitions();
    assert!(transitions.contains(&JobStatus::Assigned));
    assert!(transitions.contains(&JobStatus::InProgress));
    assert!(transitions.contains(&JobStatus::Cancelled));

    // Assigned can transition to in_progress, open, cancelled.
    let assigned = JobStatus::Assigned;
    let transitions = assigned.valid_transitions();
    assert!(transitions.contains(&JobStatus::InProgress));
    assert!(transitions.contains(&JobStatus::Open));
    assert!(transitions.contains(&JobStatus::Cancelled));

    // InProgress can transition to submitted, failed, cancelled.
    let in_progress = JobStatus::InProgress;
    let transitions = in_progress.valid_transitions();
    assert!(transitions.contains(&JobStatus::Submitted));
    assert!(transitions.contains(&JobStatus::Failed));
    assert!(transitions.contains(&JobStatus::Cancelled));
}

#[test]
fn job_serde_roundtrip_with_all_fields() {
    let job = MarketplaceJob {
        id: "roundtrip-1".into(),
        title: "Roundtrip test".into(),
        description: "Full serde roundtrip.".into(),
        job_type: "research".into(),
        status: "open".into(),
        posted_by: "tester".into(),
        assigned_to: "agent-1".into(),
        priority: "medium".into(),
        created_at: "2026-04-22T00:00:00Z".into(),
        updated_at: "2026-04-22T01:00:00Z".into(),
        tags: vec!["test".into(), "serde".into()],
        reward: "bounty-5".into(),
        plan_id: "plan-7".into(),
        submission: Some(serde_json::json!({"result": "done"})),
        evaluation: Some(serde_json::json!({"accepted": true})),
        auto_execute: true,
        ..Default::default()
    };

    let json = serde_json::to_string(&job).unwrap();
    let parsed: MarketplaceJob = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.id, "roundtrip-1");
    assert_eq!(parsed.title, "Roundtrip test");
    assert_eq!(parsed.reward, "bounty-5");
    assert!(parsed.auto_execute);
    assert!(parsed.submission.is_some());
    assert!(parsed.evaluation.is_some());
}

#[test]
fn multiple_jobs_list() {
    let dir = tempdir().expect("tempdir");

    for i in 0..5 {
        let job = MarketplaceJob {
            id: format!("job-{i}"),
            title: format!("Job {i}"),
            status: "open".into(),
            ..Default::default()
        };
        write_job(dir.path(), &job);
    }

    let jobs = list_jobs(dir.path());
    assert_eq!(jobs.len(), 5);
}
