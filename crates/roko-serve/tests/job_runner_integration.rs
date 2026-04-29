//! Integration tests for the marketplace job runner and job lifecycle endpoints.
//!
//! Tests exercise job state transitions, auto-execute behaviour, cancellation,
//! and error paths through the HTTP API.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use roko_core::config::ServeAuthConfig;
use roko_core::config::schema::RokoConfig;
use roko_serve::deploy::create_backend;
use roko_serve::routes::build_router;
use roko_serve::runtime::{CliRuntime, DashboardInfo, RunResult, SessionStatusInfo};
use roko_serve::state::AppState;
use tempfile::tempdir;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal no-op runtime for integration tests.
struct TestRuntime;

#[async_trait::async_trait]
impl CliRuntime for TestRuntime {
    async fn run_once(
        &self,
        _workdir: &std::path::Path,
        _prompt: &str,
    ) -> anyhow::Result<RunResult> {
        Ok(RunResult {
            success: true,
            output_text: Some(
                "test runtime output\n[PASS] compile: cargo check passed\n[PASS] tests: focused tests passed"
                    .to_string(),
            ),
            usage: None,
            gate_results: Vec::new(),
        })
    }

    fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo {
        SessionStatusInfo {
            session_id: None,
            workdir,
            daemon_running: false,
            signal_count: Some(0),
            episode_count: Some(0),
            last_episode_passed: None,
        }
    }

    fn dashboard_scaffold(&self, _workdir: &std::path::Path) -> DashboardInfo {
        DashboardInfo {
            rendered: String::new(),
        }
    }
}

/// Build a test router and expose its shared app state.
fn test_app_state() -> (tempfile::TempDir, Arc<AppState>, axum::Router) {
    let dir = tempdir().expect("tempdir");
    let config = RokoConfig::default();
    let deploy = Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
    let state = Arc::new(
        AppState::new(
            dir.path().to_path_buf(),
            Arc::new(TestRuntime),
            config,
            deploy,
        )
        .expect("AppState::new"),
    );
    let auth = ServeAuthConfig::default();
    let router = build_router(Arc::clone(&state), &[], auth);
    (dir, state, router)
}

/// Write a MarketplaceJob JSON file directly to .roko/jobs/.
fn write_job_file(workdir: &std::path::Path, job: &serde_json::Value) {
    let id = job["id"].as_str().expect("job must have id");
    let dir = workdir.join(".roko").join("jobs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{id}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(job).unwrap()).unwrap();
}

/// Read a job file back from disk.
fn read_job_file(workdir: &std::path::Path, id: &str) -> serde_json::Value {
    let path = workdir
        .join(".roko")
        .join("jobs")
        .join(format!("{id}.json"));
    let data = std::fs::read_to_string(&path).unwrap();
    serde_json::from_str(&data).unwrap()
}

/// Send a GET request and return `(StatusCode, serde_json::Value)`.
async fn get_json(router: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("build request");
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let body = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Send a POST request with a JSON body and return `(StatusCode, serde_json::Value)`.
async fn post_json(
    router: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("build request");
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Send a PATCH request with a JSON body and return `(StatusCode, serde_json::Value)`.
async fn patch_json(
    router: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("PATCH")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("build request");
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Send a DELETE request and return `(StatusCode, serde_json::Value)`.
async fn delete_json(router: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("DELETE")
        .uri(uri)
        .body(Body::empty())
        .expect("build request");
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ---------------------------------------------------------------------------
// Tests: Research job creates artifact
// ---------------------------------------------------------------------------

#[tokio::test]
async fn research_job_creates_artifact_via_api() {
    let (dir, _state, app) = test_app_state();

    // Create a research job.
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "title": "Research DeFi lending protocols",
            "description": "Analyze top 5 DeFi lending protocols.",
            "job_type": "research",
            "posted_by": "operator"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let job_id = created["id"].as_str().expect("job id");
    assert_eq!(created["state"], "open");
    assert_eq!(created["job_type"], "research");

    // Verify the job file was written to disk.
    let disk_job = read_job_file(dir.path(), job_id);
    assert_eq!(disk_job["title"], "Research DeFi lending protocols");
    assert_eq!(disk_job["state"], "open");
}

// ---------------------------------------------------------------------------
// Tests: Coding job transitions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn coding_job_full_lifecycle_transitions() {
    let (_dir, _state, app) = test_app_state();

    // Create a coding job.
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "title": "Implement marketplace filters",
            "description": "Add filtering to the marketplace view.",
            "job_type": "coding_task",
            "posted_by": "operator",
            "plan_id": "plan-42"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let job_id = created["id"].as_str().expect("job id");
    assert_eq!(created["state"], "open");

    // Transition: open -> assigned
    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "assigned", "assigned_to": "agent-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["state"], "assigned");
    assert_eq!(updated["assigned_to"], "agent-1");

    // Transition: assigned -> in_progress
    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["state"], "in_progress");

    // Transition: in_progress -> submitted
    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "submitted" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["state"], "submitted");

    // Transition: submitted -> completed
    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "completed" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["state"], "completed");

    // Terminal state: no further transitions allowed.
    let (status, _) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "open" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn coding_job_execution_persists_artifacts_and_gate_results() {
    let (dir, state, _app) = test_app_state();
    let job_json = serde_json::json!({
        "id": "coding-payload",
        "title": "Implement payload collection",
        "description": "Collect artifacts and gate results for coding jobs.",
        "job_type": "coding_task",
        "status": "open",
        "plan_id": "plan-42",
        "created_at": "2026-04-22T00:00:00Z",
        "updated_at": "2026-04-22T00:00:00Z"
    });
    write_job_file(dir.path(), &job_json);
    std::fs::create_dir_all(dir.path().join("plans")).unwrap();
    std::fs::write(dir.path().join("plans").join("plan-42.md"), "# Plan 42\n").unwrap();

    let summary = roko_serve::job_runner::execute_job(&state, "coding-payload")
        .await
        .expect("execute coding job");
    assert!(summary.contains("test runtime output"));

    let final_job = read_job_file(dir.path(), "coding-payload");
    assert_eq!(final_job["status"], "completed");
    let submission = final_job["submission"]
        .as_object()
        .expect("submission object");
    assert!(
        submission["result_summary"]
            .as_str()
            .unwrap_or_default()
            .contains("test runtime output")
    );
    let artifacts = submission["artifacts"].as_array().expect("artifacts array");
    assert!(
        artifacts
            .iter()
            .any(|artifact| artifact["path"] == ".roko/jobs/artifacts/coding-payload/job-brief.md"),
        "job brief artifact missing: {artifacts:?}"
    );
    assert!(
        artifacts
            .iter()
            .any(|artifact| artifact["path"] == "plans/plan-42.md"),
        "plan artifact missing: {artifacts:?}"
    );
    let gates = submission["gate_results"]
        .as_array()
        .expect("gate results array");
    assert!(
        gates
            .iter()
            .any(|gate| gate["gate"] == "compile" && gate["passed"] == true),
        "compile gate missing: {gates:?}"
    );
    assert!(
        gates
            .iter()
            .any(|gate| gate["gate"] == "tests" && gate["passed"] == true),
        "tests gate missing: {gates:?}"
    );
}

#[tokio::test]
async fn coding_job_without_plan_materializes_prd_and_synthetic_plan() {
    let (dir, state, _app) = test_app_state();
    let job_json = serde_json::json!({
        "id": "coding-no-plan",
        "title": "Implement generated plan path",
        "description": "Exercise PRD to synthetic plan fallback for coding jobs.",
        "job_type": "coding_task",
        "status": "open",
        "created_at": "2026-04-22T00:00:00Z",
        "updated_at": "2026-04-22T00:00:00Z"
    });
    write_job_file(dir.path(), &job_json);

    roko_serve::job_runner::execute_job(&state, "coding-no-plan")
        .await
        .expect("execute coding job");

    let final_job = read_job_file(dir.path(), "coding-no-plan");
    assert_eq!(final_job["status"], "completed");
    assert!(
        dir.path()
            .join(".roko/prd/published/job-coding-no-plan.md")
            .exists(),
        "coding job PRD should be materialized"
    );
    assert!(
        dir.path()
            .join(".roko/plans/job-coding-no-plan/tasks.toml")
            .exists(),
        "fallback plan tasks should be materialized"
    );

    let artifacts = final_job["submission"]["artifacts"]
        .as_array()
        .expect("artifacts array");
    assert!(
        artifacts.iter().any(|artifact| artifact["kind"] == "prd"),
        "PRD artifact missing: {artifacts:?}"
    );
    assert!(
        artifacts
            .iter()
            .any(|artifact| artifact["path"] == ".roko/plans/job-coding-no-plan/tasks.toml"),
        "synthetic plan artifact missing: {artifacts:?}"
    );
}

// ---------------------------------------------------------------------------
// Tests: auto_execute false stays open
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auto_execute_false_stays_open() {
    let (dir, _state, app) = test_app_state();

    // Write a job file directly with auto_execute=false.
    let job_json = serde_json::json!({
        "id": "job-no-auto",
        "title": "Manual review needed",
        "description": "This job should not auto-execute.",
        "job_type": "coding_task",
        "status": "open",
        "auto_execute": false,
        "created_at": "2026-04-22T00:00:00Z",
        "updated_at": "2026-04-22T00:00:00Z"
    });
    write_job_file(dir.path(), &job_json);

    // Fetch via API — should still be open.
    let (status, fetched) = get_json(&app, "/api/jobs/job-no-auto").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["state"], "open");
    assert_eq!(fetched["id"], "job-no-auto");
}

// ---------------------------------------------------------------------------
// Tests: Cancel prevents execution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_prevents_execution() {
    let (_dir, _state, app) = test_app_state();

    // Create a job.
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "job-cancel-test",
            "title": "Cancel me",
            "description": "This job will be cancelled."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["state"], "open");

    // Cancel the job via DELETE.
    let (status, cancelled) = delete_json(&app, "/api/jobs/job-cancel-test").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["state"], "cancelled");

    // Trying to execute a cancelled job should fail.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs/job-cancel-test/execute",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("cancelled"),
        "error should mention cancelled status: {err_body}"
    );
}

// ---------------------------------------------------------------------------
// Tests: Execute nonexistent job fails
// ---------------------------------------------------------------------------

#[tokio::test]
async fn execute_nonexistent_job_fails() {
    let (_dir, _state, app) = test_app_state();

    // Try to get a job that doesn't exist.
    let (status, err_body) = get_json(&app, "/api/jobs/nonexistent-job-xyz").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("not found"),
        "error should mention not found: {err_body}"
    );

    // Try to execute a job that doesn't exist.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs/nonexistent-job-xyz/execute",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("not found"),
        "error should mention not found: {err_body}"
    );
}

// ---------------------------------------------------------------------------
// Tests: Job file lock prevents double execution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn job_file_lock_prevents_state_conflict() {
    let (dir, _state, app) = test_app_state();

    // Create a job and move it to in_progress.
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "job-lock-test",
            "title": "Lock test",
            "description": "Testing lock behavior."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["state"], "open");

    // Write a lock file to simulate an in-progress execution.
    let lock_path = dir
        .path()
        .join(".roko")
        .join("jobs")
        .join("job-lock-test.json.lock");
    std::fs::write(&lock_path, "12345").unwrap();
    assert!(lock_path.exists());

    // The job can still be read via the API.
    let (status, fetched) = get_json(&app, "/api/jobs/job-lock-test").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["id"], "job-lock-test");

    // But executing it should fail since it's already being executed
    // (the execute endpoint checks the current status, not the lock file).
    // First move it out of open state by patching to in_progress.
    let (status, _) = patch_json(
        &app,
        "/api/jobs/job-lock-test",
        serde_json::json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Now trying to execute should fail — it's already in_progress.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs/job-lock-test/execute",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("in_progress"),
        "error should mention current status: {err_body}"
    );
}

// ---------------------------------------------------------------------------
// Tests: Job lifecycle — assign, submit, evaluate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn assign_submit_evaluate_lifecycle() {
    let (_dir, _state, app) = test_app_state();

    // Create a job.
    let (status, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "job-lifecycle",
            "title": "Full lifecycle test",
            "description": "Test the full assign-submit-evaluate flow."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Assign it.
    let (status, assigned) = post_json(
        &app,
        "/api/jobs/job-lifecycle/assign",
        serde_json::json!({ "agent_id": "agent-42" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assigned["state"], "assigned");
    assert_eq!(assigned["assigned_to"], "agent-42");

    // Move to in_progress.
    let (status, updated) = patch_json(
        &app,
        "/api/jobs/job-lifecycle",
        serde_json::json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["state"], "in_progress");

    // Submit.
    let (status, submitted) = post_json(
        &app,
        "/api/jobs/job-lifecycle/submit",
        serde_json::json!({
            "result_summary": "Implemented the feature.",
            "artifacts": [{"path": "src/main.rs"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(submitted["state"], "submitted");
    assert!(submitted["submission"].is_object());

    // Evaluate (accept).
    let (status, evaluated) = post_json(
        &app,
        "/api/jobs/job-lifecycle/evaluate",
        serde_json::json!({
            "accepted": true,
            "feedback": "Looks good!"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(evaluated["state"], "completed");
    assert!(evaluated["evaluation"].is_object());
}

// ---------------------------------------------------------------------------
// Tests: Invalid status transition
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invalid_status_transition_rejected() {
    let (_dir, _state, app) = test_app_state();

    // Create a job (status=open).
    let (status, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "job-invalid-tx",
            "title": "Invalid transition test"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Try to go directly from open to submitted (invalid).
    let (status, err_body) = patch_json(
        &app,
        "/api/jobs/job-invalid-tx",
        serde_json::json!({ "status": "submitted" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("invalid")
    );

    // Try to go directly from open to completed (invalid).
    let (status, _) = patch_json(
        &app,
        "/api/jobs/job-invalid-tx",
        serde_json::json!({ "status": "completed" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ---------------------------------------------------------------------------
// Tests: Duplicate job id rejected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn duplicate_job_id_rejected() {
    let (_dir, _state, app) = test_app_state();

    let (status, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "job-dup",
            "title": "First job"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Creating with the same id should fail.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "job-dup",
            "title": "Duplicate job"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("already exists")
    );
}

// ---------------------------------------------------------------------------
// Tests: Cancel terminal job fails
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_terminal_job_fails() {
    let (dir, _state, app) = test_app_state();

    // Write a completed job directly.
    let job_json = serde_json::json!({
        "id": "job-terminal",
        "title": "Already done",
        "status": "completed",
        "created_at": "2026-04-22T00:00:00Z",
        "updated_at": "2026-04-22T00:00:00Z"
    });
    write_job_file(dir.path(), &job_json);

    // Try to cancel it.
    let (status, err_body) = delete_json(&app, "/api/jobs/job-terminal").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("terminal"),
        "error should mention terminal state: {err_body}"
    );
}

// ---------------------------------------------------------------------------
// Tests: Execute job through HTTP endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn execute_open_job_returns_accepted() {
    let (_dir, _state, app) = test_app_state();

    // Create a job (status=open).
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "job-exec-http",
            "title": "Execute via HTTP",
            "description": "Test execute endpoint."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["state"], "open");

    // Execute through HTTP.
    let (status, body) = post_json(
        &app,
        "/api/jobs/job-exec-http/execute",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["id"], "job-exec-http");
    assert_eq!(body["status"], "executing");
}

// ---------------------------------------------------------------------------
// Tests: Blank title rejected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn blank_title_rejected() {
    let (_dir, _state, app) = test_app_state();

    let (status, err_body) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "title": "   ",
            "description": "blank title test"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("blank"),
        "error should mention blank: {err_body}"
    );
}

// ---------------------------------------------------------------------------
// Tests: Job stats endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn job_stats_reflect_created_jobs() {
    let (_dir, _state, app) = test_app_state();

    // Stats should start empty.
    let (status, stats) = get_json(&app, "/api/jobs/stats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["total"], 0);

    // Create two coding jobs and one research job.
    post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "title": "Job A", "job_type": "coding_task" }),
    )
    .await;
    post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "title": "Job B", "job_type": "coding_task" }),
    )
    .await;
    post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "title": "Job C", "job_type": "research" }),
    )
    .await;

    let (status, stats) = get_json(&app, "/api/jobs/stats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["total"], 3);
    assert_eq!(stats["by_state"]["open"], 3);
    assert_eq!(stats["by_type"]["coding_task"], 2);
    assert_eq!(stats["by_type"]["research"], 1);
}

// ---------------------------------------------------------------------------
// Tests: Heartbeat lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn heartbeat_post_and_list() {
    let (_dir, _state, app) = test_app_state();

    // Post a heartbeat.
    let req = Request::builder()
        .method("POST")
        .uri("/api/heartbeats")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "sender_id": "agent-1",
                "timestamp": "2026-04-22T12:00:00Z",
                "active_tasks": 3,
                "active_agents": 1
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // Post a second heartbeat from a different sender.
    let req = Request::builder()
        .method("POST")
        .uri("/api/heartbeats")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "sender_id": "agent-2",
                "timestamp": "2026-04-22T12:01:00Z",
                "active_tasks": 1,
                "active_agents": 1
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // List heartbeats — should return both.
    let (status, list) = get_json(&app, "/api/heartbeats").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("heartbeats should be array");
    assert_eq!(arr.len(), 2);
}

#[tokio::test]
async fn network_stats_aggregates_heartbeats() {
    let (_dir, _state, app) = test_app_state();

    // Post two heartbeats from the same sender.
    for ts in ["2026-04-22T12:00:00Z", "2026-04-22T12:01:00Z"] {
        let req = Request::builder()
            .method("POST")
            .uri("/api/heartbeats")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "sender_id": "agent-stats",
                    "timestamp": ts,
                    "active_tasks": 4,
                    "active_agents": 2
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    // Get network stats.
    let (status, stats) = get_json(&app, "/api/network/stats").await;
    assert_eq!(status, StatusCode::OK);
    let arr = stats.as_array().expect("network stats should be array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["sender_id"], "agent-stats");
    assert_eq!(arr[0]["heartbeat_count"], 2);
    assert_eq!(arr[0]["avg_active_tasks"], 4.0);
}

// ---------------------------------------------------------------------------
// Tests: Filtering by state, type, assigned_to
// ---------------------------------------------------------------------------

#[tokio::test]
async fn filter_jobs_by_state() {
    let (_dir, _state, app) = test_app_state();

    // Create two jobs.
    let (_, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "filter-a", "title": "Filter A" }),
    )
    .await;
    assert_eq!(created["state"], "open");

    let (_, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "filter-b", "title": "Filter B" }),
    )
    .await;
    assert_eq!(created["state"], "open");

    // Assign one.
    let _ = post_json(
        &app,
        "/api/jobs/filter-b/assign",
        serde_json::json!({ "agent_id": "agent-x" }),
    )
    .await;

    // Filter by state=open — should get only filter-a.
    let (status, list) = get_json(&app, "/api/jobs?state=open").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs should be array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "filter-a");

    // Filter by state=assigned — should get only filter-b.
    let (status, list) = get_json(&app, "/api/jobs?state=assigned").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs should be array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "filter-b");
}

#[tokio::test]
async fn filter_jobs_by_type() {
    let (_dir, _state, app) = test_app_state();

    post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "type-a", "title": "Coding", "job_type": "coding_task" }),
    )
    .await;
    post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "type-b", "title": "Research", "job_type": "research" }),
    )
    .await;

    let (status, list) = get_json(&app, "/api/jobs?job_type=research").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs should be array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "type-b");
}

#[tokio::test]
async fn filter_jobs_by_assigned_to() {
    let (_dir, _state, app) = test_app_state();

    post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "assign-a", "title": "Job A" }),
    )
    .await;
    post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "assign-b", "title": "Job B" }),
    )
    .await;

    // Assign one.
    post_json(
        &app,
        "/api/jobs/assign-a/assign",
        serde_json::json!({ "agent_id": "agent-filter" }),
    )
    .await;

    let (status, list) = get_json(&app, "/api/jobs?assigned_to=agent-filter").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs should be array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "assign-a");
}
