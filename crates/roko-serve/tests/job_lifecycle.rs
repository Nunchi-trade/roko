//! Comprehensive end-to-end integration tests for the roko-serve job lifecycle.
//!
//! These tests prove every job route works, state transitions are enforced,
//! events fire correctly, and error cases return the expected status codes.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::StreamExt;
use http_body_util::BodyExt;
use roko_core::config::ServeAuthConfig;
use roko_core::config::schema::RokoConfig;
use roko_serve::deploy::create_backend;
use roko_serve::routes::build_router;
use roko_serve::runtime::{CliRuntime, DashboardInfo, RunResult, SessionStatusInfo};
use roko_serve::state::AppState;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
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

/// Build a test router (no auth) backed by a temp directory.
fn test_app() -> (tempfile::TempDir, axum::Router) {
    let (dir, _state, router) = test_app_state();
    (dir, router)
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
    // T3-22: library default is now `enabled = true`; tests that exercise
    // unauthenticated routes opt back out explicitly.
    let auth = ServeAuthConfig {
        enabled: false,
        ..ServeAuthConfig::default()
    };
    let router = build_router(Arc::clone(&state), &[], auth);
    (dir, state, router)
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

/// Read the next text frame from a WebSocket, with a timeout.
async fn next_ws_text(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> String {
    loop {
        let message = timeout(Duration::from_secs(3), socket.next())
            .await
            .expect("wait for websocket message");
        match message {
            Some(Ok(message)) if message.is_text() => {
                return message.into_text().expect("text frame").to_string();
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => panic!("websocket error: {error}"),
            None => panic!("websocket closed"),
        }
    }
}

// =========================================================================
// Happy Path Tests
// =========================================================================

// -------------------------------------------------------------------------
// 1. Full job lifecycle happy path
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_full_job_lifecycle_happy_path() {
    let (dir, app) = test_app();

    // -- Step 1: POST /api/jobs -> create job (201, status=open)
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "lifecycle-happy",
            "title": "Happy path lifecycle job",
            "description": "Proves the complete job lifecycle.",
            "job_type": "coding_task",
            "posted_by": "operator",
            "priority": "high",
            "tags": ["lifecycle", "e2e"],
            "reward": "bounty-1",
            "plan_id": "plan-99"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create should return 201");
    assert_eq!(created["id"], "lifecycle-happy");
    assert_eq!(created["state"], "open");
    assert_eq!(created["title"], "Happy path lifecycle job");
    assert_eq!(created["job_type"], "coding_task");
    assert_eq!(created["posted_by"], "operator");
    assert_eq!(created["priority"], "high");
    assert_eq!(created["reward"], "bounty-1");
    assert_eq!(created["plan_id"], "plan-99");
    assert!(created["created_at"].is_string());
    assert!(created["updated_at"].is_string());

    // Verify file was persisted to disk.
    let persisted = dir
        .path()
        .join(".roko")
        .join("jobs")
        .join("lifecycle-happy.json");
    assert!(persisted.exists(), "job JSON must be persisted to disk");

    // -- Step 2: GET /api/jobs -> verify job in list
    let (status, listed) = get_json(&app, "/api/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let jobs = listed.as_array().expect("jobs array");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["id"], "lifecycle-happy");
    assert_eq!(jobs[0]["state"], "open");

    // -- Step 3: GET /api/jobs/{id} -> verify job details
    let (status, fetched) = get_json(&app, "/api/jobs/lifecycle-happy").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["id"], "lifecycle-happy");
    assert_eq!(fetched["description"], "Proves the complete job lifecycle.");
    assert_eq!(fetched["plan_id"], "plan-99");
    assert_eq!(fetched["reward"], "bounty-1");

    // -- Step 4: POST /api/jobs/{id}/assign -> status becomes assigned
    let (status, assigned) = post_json(
        &app,
        "/api/jobs/lifecycle-happy/assign",
        serde_json::json!({ "agent_id": "agent-alpha" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "assign should return 200");
    assert_eq!(assigned["state"], "assigned");
    assert_eq!(assigned["assigned_to"], "agent-alpha");

    // -- Step 5: POST /api/jobs/{id}/start -> status becomes in_progress
    let (status, started) = post_json(
        &app,
        "/api/jobs/lifecycle-happy/start",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "start should return 200");
    assert_eq!(started["state"], "in_progress");
    assert_eq!(
        started["assigned_to"], "agent-alpha",
        "assigned_to preserved"
    );

    // -- Step 6: POST /api/jobs/{id}/submit -> status becomes submitted
    let (status, submitted) = post_json(
        &app,
        "/api/jobs/lifecycle-happy/submit",
        serde_json::json!({
            "result_summary": "All tests pass, feature implemented.",
            "artifacts": [{"path": "src/feature.rs", "size": 4096}],
            "gate_results": [{"gate": "compile", "passed": true}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "submit should return 200");
    assert_eq!(submitted["state"], "submitted");
    let submission = &submitted["submission"];
    assert!(submission.is_object(), "submission data must be present");
    assert_eq!(
        submission["result_summary"],
        "All tests pass, feature implemented."
    );
    assert!(submission["artifacts"].is_array());
    assert!(submission["gate_results"].is_array());
    assert!(submission["submitted_at"].is_string());

    // -- Step 7: POST /api/jobs/{id}/evaluate with accepted=true -> status becomes completed
    let (status, evaluated) = post_json(
        &app,
        "/api/jobs/lifecycle-happy/evaluate",
        serde_json::json!({
            "accepted": true,
            "feedback": "Excellent work, merging now."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "evaluate should return 200");
    assert_eq!(evaluated["state"], "completed");
    let evaluation = &evaluated["evaluation"];
    assert!(evaluation.is_object(), "evaluation data must be present");
    assert_eq!(evaluation["accepted"], true);
    assert_eq!(evaluation["feedback"], "Excellent work, merging now.");
    assert!(evaluation["evaluated_at"].is_string());

    // -- Step 8: GET /api/jobs/{id} -> verify final state
    let (status, final_state) = get_json(&app, "/api/jobs/lifecycle-happy").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(final_state["state"], "completed");
    assert_eq!(final_state["assigned_to"], "agent-alpha");
    assert!(final_state["submission"].is_object());
    assert!(final_state["evaluation"].is_object());
    assert_eq!(final_state["evaluation"]["accepted"], true);

    // Verify the persisted file on disk also reflects the final state.
    let disk_data = std::fs::read_to_string(&persisted).expect("read persisted job");
    let disk_job: serde_json::Value =
        serde_json::from_str(&disk_data).expect("parse persisted job");
    assert_eq!(disk_job["state"], "completed");
    assert_eq!(disk_job["evaluation"]["accepted"], true);
}

// -------------------------------------------------------------------------
// 2. Rejection and resubmit
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_job_rejection_and_resubmit() {
    let (_dir, app) = test_app();

    // Create -> assign -> start.
    let (status, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "reject-resubmit",
            "title": "Rejection test job"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = post_json(
        &app,
        "/api/jobs/reject-resubmit/assign",
        serde_json::json!({ "agent_id": "agent-beta" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = post_json(
        &app,
        "/api/jobs/reject-resubmit/start",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Submit first attempt.
    let (status, submitted) = post_json(
        &app,
        "/api/jobs/reject-resubmit/submit",
        serde_json::json!({
            "result_summary": "First attempt, needs work."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(submitted["state"], "submitted");

    // Evaluate: REJECT (accepted=false) -> status goes back to in_progress.
    let (status, rejected) = post_json(
        &app,
        "/api/jobs/reject-resubmit/evaluate",
        serde_json::json!({
            "accepted": false,
            "feedback": "Missing error handling, please add."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rejection should return 200");
    assert_eq!(
        rejected["state"], "in_progress",
        "rejected job returns to in_progress for rework"
    );
    assert_eq!(rejected["evaluation"]["accepted"], false);
    assert_eq!(
        rejected["evaluation"]["feedback"],
        "Missing error handling, please add."
    );

    // Resubmit after rejection
    let (status, resubmitted) = post_json(
        &app,
        "/api/jobs/reject-resubmit/submit",
        serde_json::json!({
            "result_summary": "Second attempt with error handling."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resubmitted["state"], "submitted");

    // Accept the second attempt
    let (status, accepted) = post_json(
        &app,
        "/api/jobs/reject-resubmit/evaluate",
        serde_json::json!({
            "accepted": true,
            "feedback": "Looks great now!"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(accepted["state"], "completed");
    assert_eq!(accepted["evaluation"]["accepted"], true);
}

#[tokio::test]
async fn test_evaluate_accepted_sets_completed_state() {
    let (_dir, app) = test_app();

    // Create, assign, start, submit a fresh job for the accept path.
    post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "eval-accept",
            "title": "Evaluate accept test",
        }),
    )
    .await;
    post_json(
        &app,
        "/api/jobs/eval-accept/assign",
        serde_json::json!({ "agent_id": "agent-1" }),
    )
    .await;
    post_json(&app, "/api/jobs/eval-accept/start", serde_json::json!({})).await;
    post_json(
        &app,
        "/api/jobs/eval-accept/submit",
        serde_json::json!({ "result_summary": "Done." }),
    )
    .await;

    let (status, accepted) = post_json(
        &app,
        "/api/jobs/eval-accept/evaluate",
        serde_json::json!({
            "accepted": true,
            "feedback": "Looks great now!"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(accepted["state"], "completed");
    assert_eq!(accepted["evaluation"]["accepted"], true);
}

// -------------------------------------------------------------------------
// 3. Cancellation from open state (DELETE)
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_job_cancellation_from_open() {
    let (dir, app) = test_app();

    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "cancel-open",
            "title": "Cancel me from open"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["state"], "open");

    // DELETE /api/jobs/{id} cancels from open.
    let (status, cancelled) = delete_json(&app, "/api/jobs/cancel-open").await;
    assert_eq!(status, StatusCode::OK, "DELETE cancel should return 200");
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["id"], "cancel-open");

    // Verify persisted on disk.
    let path = dir
        .path()
        .join(".roko")
        .join("jobs")
        .join("cancel-open.json");
    let disk_data = std::fs::read_to_string(&path).expect("read persisted job");
    let disk_job: serde_json::Value =
        serde_json::from_str(&disk_data).expect("parse persisted job");
    assert_eq!(disk_job["state"], "cancelled");

    // Verify the cancelled job still appears in the list.
    let (status, listed) = get_json(&app, "/api/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let jobs = listed.as_array().expect("jobs array");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["state"], "cancelled");
}

// -------------------------------------------------------------------------
// 4. Cancellation from in_progress state (POST /cancel)
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_job_cancellation_from_in_progress() {
    let (_dir, app) = test_app();

    // Create -> assign -> start -> in_progress.
    let (status, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "cancel-ip",
            "title": "Cancel from in_progress"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = post_json(
        &app,
        "/api/jobs/cancel-ip/assign",
        serde_json::json!({ "agent_id": "agent-gamma" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, started) =
        post_json(&app, "/api/jobs/cancel-ip/start", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(started["state"], "in_progress");

    // POST /api/jobs/{id}/cancel from in_progress.
    let (status, cancelled) =
        post_json(&app, "/api/jobs/cancel-ip/cancel", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "POST cancel should return 200");
    assert_eq!(cancelled["state"], "cancelled");

    // Verify via GET.
    let (status, fetched) = get_json(&app, "/api/jobs/cancel-ip").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["state"], "cancelled");
}

// -------------------------------------------------------------------------
// 5. Job stats reflect state
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_job_stats_reflect_state() {
    let (_dir, app) = test_app();

    // Stats should start empty.
    let (status, stats) = get_json(&app, "/api/jobs/stats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["total"], 0);

    // Create 3 jobs.
    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "stats-a", "title": "Stats A", "job_type": "coding_task" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "stats-b", "title": "Stats B", "job_type": "research" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "stats-c", "title": "Stats C", "job_type": "coding_task" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // Advance stats-b: open -> assigned -> in_progress.
    let (s, _) = post_json(
        &app,
        "/api/jobs/stats-b/assign",
        serde_json::json!({ "agent_id": "agent-1" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = post_json(&app, "/api/jobs/stats-b/start", serde_json::json!({})).await;
    assert_eq!(s, StatusCode::OK);

    // Advance stats-c: open -> cancelled.
    let (s, _) = delete_json(&app, "/api/jobs/stats-c").await;
    assert_eq!(s, StatusCode::OK);

    // Now: stats-a=open, stats-b=in_progress, stats-c=cancelled.
    let (status, stats) = get_json(&app, "/api/jobs/stats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["total"], 3);
    assert_eq!(stats["by_state"]["open"], 1);
    assert_eq!(stats["by_state"]["in_progress"], 1);
    assert_eq!(stats["by_state"]["cancelled"], 1);
    assert_eq!(stats["by_type"]["coding_task"], 2);
    assert_eq!(stats["by_type"]["research"], 1);
}

// =========================================================================
// Error Case Tests
// =========================================================================

// -------------------------------------------------------------------------
// 6. Assign non-open job fails 422
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_assign_non_open_job_fails_422() {
    let (_dir, app) = test_app();

    // Create and assign the job (now status=assigned).
    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "assign-err", "title": "Assign error test" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs/assign-err/assign",
        serde_json::json!({ "agent_id": "agent-first" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Attempt to assign again while already assigned -> 422.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs/assign-err/assign",
        serde_json::json!({ "agent_id": "agent-second" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "assigning an already-assigned job should fail with 422"
    );
    assert_eq!(err_body["code"], "unprocessable_entity");
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("cannot assign"),
        "error message should explain the failure: {err_body}"
    );
}

// -------------------------------------------------------------------------
// 7. Submit non-in_progress job fails 422
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_submit_non_in_progress_fails_422() {
    let (_dir, app) = test_app();

    // Create a job (status=open).
    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "submit-err", "title": "Submit error test" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // Attempt to submit while still open -> 422.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs/submit-err/submit",
        serde_json::json!({ "result_summary": "should fail" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "submitting an open job should fail with 422"
    );
    assert_eq!(err_body["code"], "unprocessable_entity");
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("cannot submit"),
        "error message should explain the failure: {err_body}"
    );
}

// -------------------------------------------------------------------------
// 8. Evaluate non-submitted job fails 422
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_evaluate_non_submitted_fails_422() {
    let (_dir, app) = test_app();

    // Create -> assign -> start (status=in_progress).
    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "eval-err", "title": "Evaluate error test" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs/eval-err/assign",
        serde_json::json!({ "agent_id": "agent-x" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, _) = post_json(&app, "/api/jobs/eval-err/start", serde_json::json!({})).await;
    assert_eq!(s, StatusCode::OK);

    // Attempt to evaluate while in_progress (not submitted) -> 422.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs/eval-err/evaluate",
        serde_json::json!({ "accepted": true, "feedback": "should fail" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "evaluating an in_progress job should fail with 422"
    );
    assert_eq!(err_body["code"], "unprocessable_entity");
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("cannot evaluate"),
        "error message should explain the failure: {err_body}"
    );
}

// -------------------------------------------------------------------------
// 9. Cancel terminal job fails 422
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_cancel_terminal_job_fails_422() {
    let (_dir, app) = test_app();

    // Create -> assign -> start -> submit -> evaluate(accept) -> completed.
    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "cancel-term", "title": "Cancel terminal test" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs/cancel-term/assign",
        serde_json::json!({ "agent_id": "agent-z" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, _) = post_json(&app, "/api/jobs/cancel-term/start", serde_json::json!({})).await;
    assert_eq!(s, StatusCode::OK);

    let (s, _) = post_json(
        &app,
        "/api/jobs/cancel-term/submit",
        serde_json::json!({ "result_summary": "done" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, completed) = post_json(
        &app,
        "/api/jobs/cancel-term/evaluate",
        serde_json::json!({ "accepted": true }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(completed["state"], "completed");

    // Attempt to cancel via DELETE -> 422 (terminal state).
    let (status, err_body) = delete_json(&app, "/api/jobs/cancel-term").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "cancelling a completed job should fail with 422"
    );
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("terminal"),
        "error should mention terminal state: {err_body}"
    );

    // Also try POST /cancel -> same 422.
    let (status, err_body) =
        post_json(&app, "/api/jobs/cancel-term/cancel", serde_json::json!({})).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "POST cancel on completed job should also fail with 422"
    );
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("terminal"),
        "error should mention terminal state: {err_body}"
    );
}

// -------------------------------------------------------------------------
// 10. Invalid status transition via PATCH fails 422 with hint
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_invalid_status_transition_via_patch_fails_422() {
    let (_dir, app) = test_app();

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "bad-transition", "title": "Bad transition test" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // open -> submitted is not a valid transition.
    let (status, err_body) = patch_json(
        &app,
        "/api/jobs/bad-transition",
        serde_json::json!({ "status": "submitted" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid transition should return 422"
    );
    assert_eq!(err_body["code"], "unprocessable_entity");
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("invalid status transition"),
        "message should describe the invalid transition: {err_body}"
    );
    // The hint should list valid transitions.
    assert!(
        err_body["details"]["hint"].is_string(),
        "details should contain a hint: {err_body}"
    );

    // Also test with a totally unknown status value.
    let (status, err_body) = patch_json(
        &app,
        "/api/jobs/bad-transition",
        serde_json::json!({ "status": "foobar" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown status should return 422"
    );
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("unknown job status"),
        "message should mention unknown status: {err_body}"
    );
}

// -------------------------------------------------------------------------
// 11. GET nonexistent job returns 404
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_get_nonexistent_job_returns_404() {
    let (_dir, app) = test_app();

    let (status, err_body) = get_json(&app, "/api/jobs/no-such-job").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "nonexistent job should return 404"
    );
    assert_eq!(err_body["code"], "not_found");
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("not found"),
        "message should say not found: {err_body}"
    );
}

// -------------------------------------------------------------------------
// 12. Create duplicate job returns 409
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_create_duplicate_job_returns_409() {
    let (_dir, app) = test_app();

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "dup-job", "title": "First creation" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // Second creation with the same ID -> 409.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "dup-job", "title": "Duplicate attempt" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "duplicate job ID should return 409"
    );
    assert_eq!(err_body["code"], "conflict");
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("already exists"),
        "message should say already exists: {err_body}"
    );
}

// -------------------------------------------------------------------------
// 13. Create job with blank title returns 400
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_create_job_blank_title_returns_400() {
    let (_dir, app) = test_app();

    let (status, err_body) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "title": "   ", "description": "blank title test" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "blank title should return 400"
    );
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("blank"),
        "error should mention blank: {err_body}"
    );

    // Also test with completely empty title.
    let (status, err_body) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "title": "", "description": "empty title" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty title should return 400"
    );
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("blank"),
        "error should mention blank: {err_body}"
    );
}

// =========================================================================
// Filter Tests
// =========================================================================

// -------------------------------------------------------------------------
// 14. List jobs filter by state
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_list_jobs_filter_by_state() {
    let (_dir, app) = test_app();

    // Create three jobs.
    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "state-a", "title": "State A" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "state-b", "title": "State B" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "state-c", "title": "State C" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // Assign state-b (open -> assigned).
    let (s, _) = post_json(
        &app,
        "/api/jobs/state-b/assign",
        serde_json::json!({ "agent_id": "agent-1" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Cancel state-c.
    let (s, _) = delete_json(&app, "/api/jobs/state-c").await;
    assert_eq!(s, StatusCode::OK);

    // Filter by state=open -> only state-a.
    let (status, list) = get_json(&app, "/api/jobs?state=open").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs array");
    assert_eq!(arr.len(), 1, "only one open job");
    assert_eq!(arr[0]["id"], "state-a");

    // Filter by state=assigned -> only state-b.
    let (status, list) = get_json(&app, "/api/jobs?state=assigned").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs array");
    assert_eq!(arr.len(), 1, "only one assigned job");
    assert_eq!(arr[0]["id"], "state-b");

    // Filter by state=cancelled -> only state-c.
    let (status, list) = get_json(&app, "/api/jobs?state=cancelled").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs array");
    assert_eq!(arr.len(), 1, "only one cancelled job");
    assert_eq!(arr[0]["id"], "state-c");

    // No filter -> all 3.
    let (status, list) = get_json(&app, "/api/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs array");
    assert_eq!(arr.len(), 3, "all jobs returned without filter");
}

// -------------------------------------------------------------------------
// 15. List jobs filter by type
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_list_jobs_filter_by_type() {
    let (_dir, app) = test_app();

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "type-coding", "title": "Coding job", "job_type": "coding_task" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "type-research", "title": "Research job", "job_type": "research" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "type-audit", "title": "Audit job", "job_type": "audit" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // Filter by job_type=research.
    let (status, list) = get_json(&app, "/api/jobs?job_type=research").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "type-research");
    assert_eq!(arr[0]["job_type"], "research");

    // Filter by job_type=coding_task.
    let (status, list) = get_json(&app, "/api/jobs?job_type=coding_task").await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().expect("jobs array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "type-coding");
}

// =========================================================================
// Event Tests
// =========================================================================

// -------------------------------------------------------------------------
// 16. Job events fire on WebSocket
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_job_events_fire_on_websocket() {
    let (_dir, _state, app) = test_app_state();

    // Bind a TCP listener and serve the app.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ws server");
    let addr = listener.local_addr().expect("listener addr");
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app)
            .await
            .expect("serve test app");
    });

    // Connect a WebSocket client.
    let (mut socket, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("connect websocket");

    // -- Create a job: should fire JobCreated.
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "ws-events-job",
            "title": "WebSocket event test"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["id"], "ws-events-job");

    let create_event: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse create event");
    assert_eq!(
        create_event["type"], "job_created",
        "first event should be job_created"
    );
    assert_eq!(create_event["job"]["id"], "ws-events-job");
    assert_eq!(create_event["job"]["state"], "open");

    // -- Assign the job: should fire JobUpdated then JobTransitioned.
    let (status, _) = post_json(
        &app,
        "/api/jobs/ws-events-job/assign",
        serde_json::json!({ "agent_id": "ws-agent" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let update_event: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse update event");
    assert_eq!(
        update_event["type"], "job_updated",
        "assignment should emit job_updated"
    );
    assert_eq!(update_event["job"]["id"], "ws-events-job");
    assert_eq!(update_event["job"]["state"], "assigned");
    assert_eq!(update_event["job"]["assigned_to"], "ws-agent");

    let transition_event: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse transition event");
    assert_eq!(
        transition_event["type"], "job_transitioned",
        "assignment should emit job_transitioned"
    );
    assert_eq!(transition_event["job_id"], "ws-events-job");
    assert_eq!(transition_event["from"], "open");
    assert_eq!(transition_event["to"], "assigned");
    assert_eq!(transition_event["assigned_to"], "ws-agent");

    // -- Start the job: should fire JobUpdated then JobTransitioned.
    let (status, _) = post_json(&app, "/api/jobs/ws-events-job/start", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let start_update: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse start update");
    assert_eq!(start_update["type"], "job_updated");
    assert_eq!(start_update["job"]["state"], "in_progress");

    let start_transition: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse start transition");
    assert_eq!(start_transition["type"], "job_transitioned");
    assert_eq!(start_transition["from"], "assigned");
    assert_eq!(start_transition["to"], "in_progress");

    let _ = socket.close(None).await;
    server.abort();
}

// =========================================================================
// Additional edge case tests
// =========================================================================

// -------------------------------------------------------------------------
// Start non-assigned job fails 422
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_start_non_assigned_job_fails_422() {
    let (_dir, app) = test_app();

    // Create a job (status=open).
    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "start-err", "title": "Start error test" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // Attempt to start while still open -> 422.
    let (status, err_body) =
        post_json(&app, "/api/jobs/start-err/start", serde_json::json!({})).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "starting an open job should fail with 422"
    );
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("cannot start"),
        "error should mention cannot start: {err_body}"
    );
}

// -------------------------------------------------------------------------
// PATCH with only assigned_to (no status change) succeeds
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_patch_only_assigned_to_succeeds() {
    let (_dir, app) = test_app();

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "patch-assign", "title": "Patch assignee only" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // PATCH with only assigned_to (no status transition).
    let (status, patched) = patch_json(
        &app,
        "/api/jobs/patch-assign",
        serde_json::json!({ "assigned_to": "new-owner" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(patched["assigned_to"], "new-owner");
    assert_eq!(patched["state"], "open", "status should remain open");
}

// -------------------------------------------------------------------------
// PATCH with missing both status and assigned_to returns 400
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_patch_empty_body_returns_400() {
    let (_dir, app) = test_app();

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "patch-empty", "title": "Patch empty body" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // PATCH with neither status nor assigned_to -> validation error.
    let (status, err_body) = patch_json(&app, "/api/jobs/patch-empty", serde_json::json!({})).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty PATCH body should return 400"
    );
    assert!(
        err_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("must include"),
        "error should explain what is required: {err_body}"
    );
}

// -------------------------------------------------------------------------
// Cancel via DELETE and POST /cancel on the same job both work for non-terminal
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_cancel_from_assigned_state() {
    let (_dir, app) = test_app();

    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "cancel-assigned", "title": "Cancel from assigned" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs/cancel-assigned/assign",
        serde_json::json!({ "agent_id": "agent-1" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Cancel from assigned state via POST /cancel.
    let (status, cancelled) = post_json(
        &app,
        "/api/jobs/cancel-assigned/cancel",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["state"], "cancelled");
}

// -------------------------------------------------------------------------
// Cancel from submitted state works
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_cancel_from_submitted_state() {
    let (_dir, app) = test_app();

    // Walk through open -> assigned -> in_progress -> submitted.
    let (s, _) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({ "id": "cancel-sub", "title": "Cancel from submitted" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _) = post_json(
        &app,
        "/api/jobs/cancel-sub/assign",
        serde_json::json!({ "agent_id": "agent-1" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, _) = post_json(&app, "/api/jobs/cancel-sub/start", serde_json::json!({})).await;
    assert_eq!(s, StatusCode::OK);

    let (s, _) = post_json(
        &app,
        "/api/jobs/cancel-sub/submit",
        serde_json::json!({ "result_summary": "done" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Cancel from submitted via DELETE.
    let (status, cancelled) = delete_json(&app, "/api/jobs/cancel-sub").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["state"], "cancelled");
}
