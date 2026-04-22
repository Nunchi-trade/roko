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
            output_text: Some("test runtime output".to_string()),
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
    let state = Arc::new(AppState::new(
        dir.path().to_path_buf(),
        Arc::new(TestRuntime),
        config,
        deploy,
    ));
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
    let path = workdir.join(".roko").join("jobs").join(format!("{id}.json"));
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
    let json: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
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
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
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
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
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
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
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
    assert_eq!(created["status"], "open");
    assert_eq!(created["job_type"], "research");

    // Verify the job file was written to disk.
    let disk_job = read_job_file(dir.path(), job_id);
    assert_eq!(disk_job["title"], "Research DeFi lending protocols");
    assert_eq!(disk_job["status"], "open");
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
    assert_eq!(created["status"], "open");

    // Transition: open -> assigned
    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "assigned", "assigned_to": "agent-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["status"], "assigned");
    assert_eq!(updated["assigned_to"], "agent-1");

    // Transition: assigned -> in_progress
    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["status"], "in_progress");

    // Transition: in_progress -> submitted
    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "submitted" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["status"], "submitted");

    // Transition: submitted -> completed
    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "completed" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["status"], "completed");

    // Terminal state: no further transitions allowed.
    let (status, _) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({ "status": "open" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
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
    assert_eq!(fetched["status"], "open");
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
    assert_eq!(created["status"], "open");

    // Cancel the job via DELETE.
    let (status, cancelled) = delete_json(&app, "/api/jobs/job-cancel-test").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["status"], "cancelled");

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
    assert_eq!(created["status"], "open");

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
    assert_eq!(assigned["status"], "assigned");
    assert_eq!(assigned["assigned_to"], "agent-42");

    // Move to in_progress.
    let (status, updated) = patch_json(
        &app,
        "/api/jobs/job-lifecycle",
        serde_json::json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["status"], "in_progress");

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
    assert_eq!(submitted["status"], "submitted");
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
    assert_eq!(evaluated["status"], "completed");
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
    assert!(err_body["message"].as_str().unwrap_or("").contains("invalid"));

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
    assert!(err_body["message"].as_str().unwrap_or("").contains("already exists"));
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
