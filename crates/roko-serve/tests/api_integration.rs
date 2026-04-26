//! HTTP API integration tests for the roko-serve control plane.
//!
//! These tests build the full axum router with a minimal (no-op) runtime and
//! exercise key endpoints using `tower::ServiceExt::oneshot`.

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

/// Build a test router with API-key auth enabled.
fn test_app_with_auth(api_key: &str) -> (tempfile::TempDir, axum::Router) {
    let dir = tempdir().expect("tempdir");
    let mut config = RokoConfig::default();
    let auth = ServeAuthConfig {
        enabled: true,
        api_key: api_key.to_string(),
        api_keys: Vec::new(),
        privy_app_id: None,
    };
    config.serve.auth = auth.clone();
    let deploy = Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
    let state = Arc::new(AppState::new(
        dir.path().to_path_buf(),
        Arc::new(TestRuntime),
        config,
        deploy,
    ));
    let router = build_router(Arc::clone(&state), &[], auth);
    (dir, router)
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

// ---------------------------------------------------------------------------
// Health & status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_returns_200_with_status_ok() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/health").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert!(body["uptime_secs"].is_number());
    assert!(body["version"].is_string());
}

#[tokio::test]
async fn session_status_returns_workdir() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/status").await;

    assert_eq!(status, StatusCode::OK);
    assert!(!body["workdir"].is_null());
    assert_eq!(body["daemon_running"], false);
}

#[tokio::test]
async fn run_status_returns_terminal_output_text() {
    let (_dir, app) = test_app();
    let (status, body) =
        post_json(&app, "/api/run", serde_json::json!({ "prompt": "hello" })).await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let run_id = body["id"].as_str().expect("run id");

    for _ in 0..20 {
        let (status, body) = get_json(&app, &format!("/api/run/{run_id}/status")).await;
        assert_eq!(status, StatusCode::OK);
        if body["finished"] == true {
            assert_eq!(body["status"], "completed");
            assert_eq!(body["output_text"], "test runtime output");
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    panic!("timed out waiting for run completion");
}

// ---------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_plans_empty() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/plans").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn jobs_create_list_get_and_update_round_trip() {
    let (dir, app) = test_app();
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "title": "Implement marketplace filters",
            "description": "Add durable jobs API support.",
            "job_type": "coding_task",
            "posted_by": "operator",
            "priority": "high",
            "tags": ["marketplace", "serve"],
            "reward": "bounty-7",
            "plan_id": "plan-42"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    let job_id = created["id"].as_str().expect("job id");
    assert_eq!(created["title"], "Implement marketplace filters");
    assert_eq!(created["state"], "open");
    assert_eq!(created["job_type"], "coding_task");

    let persisted = dir
        .path()
        .join(".roko")
        .join("jobs")
        .join(format!("{job_id}.json"));
    assert!(persisted.exists());

    let (status, listed) = get_json(&app, "/api/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let jobs = listed.as_array().expect("jobs array");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["id"], job_id);
    assert_eq!(jobs[0]["state"], "open");
    assert_eq!(jobs[0]["posted_by"], "operator");

    let (status, fetched) = get_json(&app, &format!("/api/jobs/{job_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["plan_id"], "plan-42");
    assert_eq!(fetched["reward"], "bounty-7");

    let (status, updated) = patch_json(
        &app,
        &format!("/api/jobs/{job_id}"),
        serde_json::json!({
            "status": "in_progress",
            "assigned_to": "implementer-1"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["state"], "in_progress");
    assert_eq!(updated["assigned_to"], "implementer-1");

    let (status, fetched_again) = get_json(&app, &format!("/api/jobs/{job_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched_again["state"], "in_progress");
    assert_eq!(fetched_again["assigned_to"], "implementer-1");
}

#[tokio::test]
async fn jobs_events_are_visible_over_websocket() {
    let (_dir, _state, app) = test_app_state();

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

    let (mut socket, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("connect websocket");

    let (_status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "id": "job-ws-1",
            "title": "Broadcast me",
            "description": "Verify websocket visibility."
        }),
    )
    .await;
    assert_eq!(created["id"], "job-ws-1");

    let create_event: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse create event");
    assert_eq!(create_event["type"], "job_created");
    assert_eq!(create_event["job"]["id"], "job-ws-1");
    assert_eq!(create_event["job"]["state"], "open");

    let (_status, _updated) = patch_json(
        &app,
        "/api/jobs/job-ws-1",
        serde_json::json!({
            "status": "assigned",
            "assigned_to": "agent-7"
        }),
    )
    .await;

    let update_event: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse update event");
    assert_eq!(update_event["type"], "job_updated");
    assert_eq!(update_event["job"]["id"], "job-ws-1");
    assert_eq!(update_event["job"]["state"], "assigned");
    assert_eq!(update_event["job"]["assigned_to"], "agent-7");

    let _ = socket.close(None).await;
    server.abort();
}

// ---------------------------------------------------------------------------
// Managed agents
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_managed_agents_empty() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/managed-agents").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn signals_returns_empty_array() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/signals").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());
}

// ---------------------------------------------------------------------------
// Episodes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn episodes_returns_empty_array() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/episodes").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_returns_json() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/metrics").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());
}

// ---------------------------------------------------------------------------
// Research
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_research_empty() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/research").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_run_returns_accepted() {
    let (_dir, app) = test_app();
    let (status, body) = post_json(
        &app,
        "/api/run",
        serde_json::json!({ "prompt": "hello world" }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body["id"].is_string());
}

#[tokio::test]
async fn post_run_rejects_empty_prompt() {
    let (_dir, app) = test_app();
    let (status, body) = post_json(&app, "/api/run", serde_json::json!({ "prompt": "" })).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_error");
}

#[tokio::test]
async fn post_run_rejects_missing_prompt() {
    let (_dir, app) = test_app();
    let (status, _body) = post_json(&app, "/api/run", serde_json::json!({})).await;

    // Missing required field — either 400 (validation) or 422 (parse).
    assert!(status.is_client_error());
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_rejects_missing_key() {
    let (_dir, app) = test_app_with_auth("secret-key-123");
    let (status, body) = get_json(&app, "/api/health").await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");
}

#[tokio::test]
async fn auth_rejects_wrong_key() {
    let (_dir, app) = test_app_with_auth("secret-key-123");

    let req = Request::builder()
        .uri("/api/health")
        .header("X-Api-Key", "wrong-key")
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_accepts_correct_key() {
    let (_dir, app) = test_app_with_auth("secret-key-123");

    let req = Request::builder()
        .uri("/api/health")
        .header("X-Api-Key", "secret-key-123")
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 404 for unknown routes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_route_returns_404() {
    let (_dir, app) = test_app();

    let req = Request::builder()
        .uri("/api/nonexistent")
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Gates summary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gate_summary_returns_ok() {
    let (_dir, app) = test_app();
    let (status, _body) = get_json(&app, "/api/gates/summary").await;

    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dashboard_returns_ok() {
    let (_dir, app) = test_app();
    let (status, _body) = get_json(&app, "/api/dashboard").await;

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn projection_catalog_exposes_stable_dashboard_state_contracts() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/projections/catalog").await;

    assert_eq!(status, StatusCode::OK);
    let projections = body["projections"].as_array().expect("projection catalog");
    for name in [
        "agent_state",
        "plan_state",
        "gate_state",
        "learning_policy_state",
    ] {
        let entry = projections
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("missing projection contract {name}"));
        assert_eq!(entry["version"], 1);
        assert!(entry["policy"]["max_age_secs"].as_u64().unwrap_or(0) > 0);
        assert!(
            !entry["policy"]["invalidation_triggers"]
                .as_array()
                .expect("triggers")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn stable_projection_frames_include_version_and_explicit_missing_state() {
    let (_dir, state, app) = test_app_state();
    state
        .state_hub
        .publish(roko_core::DashboardEvent::PlanStarted {
            plan_id: "plan-1".into(),
        });
    state
        .state_hub
        .publish(roko_core::DashboardEvent::TaskStarted {
            plan_id: "plan-1".into(),
            task_id: "work-a".into(),
            phase: "dispatch".into(),
        });
    state
        .state_hub
        .publish(roko_core::DashboardEvent::GateResult {
            plan_id: "plan-1".into(),
            task_id: "work-a".into(),
            gate: "compile".into(),
            passed: false,
        });

    let (status, plan) = get_json(&app, "/api/projections/plan_state?filter=plan:plan-1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plan["name"], "plan_state");
    assert_eq!(plan["version"], 1);
    assert!(plan["computed_at"].as_str().is_some());
    assert_eq!(plan["recovered"], false);
    assert_eq!(plan["state"]["plans"][0]["plan_id"], "plan-1");
    assert_eq!(plan["state"]["tasks"][0]["task_id"], "work-a");
    assert_eq!(plan["state"]["availability"]["state"], "available");

    let (status, gates) = get_json(&app, "/api/projections/gate_state?filter=plan:plan-1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(gates["name"], "gate_state");
    assert_eq!(gates["state"]["gates"][0]["gate"], "compile");
    assert_eq!(gates["state"]["stats"]["failed"], 1);
    assert_eq!(gates["state"]["thresholds"]["state"], "missing");

    let (status, learning) = get_json(&app, "/api/projections/learning_policy_state").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(learning["name"], "learning_policy_state");
    assert_eq!(learning["state"]["cascade_router"]["state"], "missing");
    assert_eq!(
        learning["state"]["policy_updates"]["state"],
        "unavailable_in_statehub"
    );
    assert_eq!(
        learning["state"]["policy_updates"]["endpoint"],
        "/api/learning/policy-updates"
    );
}

// ---------------------------------------------------------------------------
// OpenAPI spec
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openapi_spec_returns_json() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/openapi.json").await;

    assert_eq!(status, StatusCode::OK);
    // Should have standard OpenAPI top-level keys.
    assert!(body.get("openapi").is_some() || body.get("paths").is_some());
}

// ---------------------------------------------------------------------------
// EventBus ↔ StateHub bridge
// ---------------------------------------------------------------------------

/// Verify that a `DashboardEvent` published to `StateHub` arrives on the
/// `EventBus` and is visible to a WebSocket client via the orchestrator bridge.
#[tokio::test]
async fn orchestrator_events_reach_websocket_via_bridge() {
    let (_dir, state, app) = test_app_state();
    let _bridge = roko_serve::start_orchestrator_event_bridge(Arc::clone(&state));

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

    let (mut socket, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("connect websocket");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publish a DashboardEvent directly to StateHub (simulating orchestrate.rs).
    let sender = state.state_hub.sender();
    sender.publish(roko_core::DashboardEvent::GateResult {
        plan_id: "test-plan-1".to_string(),
        task_id: "task-A".to_string(),
        gate: "compile".to_string(),
        passed: true,
    });

    // The bridge converts it to ServerEvent::GateResult → WS client sees it.
    let event: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse gate event");
    assert_eq!(event["type"], "gate_result");
    assert_eq!(event["plan_id"], "test-plan-1");
    assert_eq!(event["task_id"], "task-A");
    assert_eq!(event["gate"], "compile");
    assert_eq!(event["passed"], true);

    let _ = socket.close(None).await;
    server.abort();
}

/// Verify multiple `DashboardEvent` types bridge correctly in sequence.
#[tokio::test]
async fn bridge_converts_multiple_event_types() {
    let (_dir, state, app) = test_app_state();
    let _bridge = roko_serve::start_orchestrator_event_bridge(Arc::clone(&state));

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

    let (mut socket, _) = connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("connect websocket");

    tokio::time::sleep(Duration::from_millis(50)).await;
    let sender = state.state_hub.sender();

    // 1. PlanStarted
    sender.publish(roko_core::DashboardEvent::PlanStarted {
        plan_id: "plan-bridge".to_string(),
    });
    let ev: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse plan_started");
    assert_eq!(ev["type"], "plan_started");
    assert_eq!(ev["plan_id"], "plan-bridge");

    // 2. TaskStarted (wrapped in Execution)
    sender.publish(roko_core::DashboardEvent::TaskStarted {
        plan_id: "plan-bridge".to_string(),
        task_id: "task-1".to_string(),
        phase: "implementing".to_string(),
    });
    let ev: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse task_started");
    assert_eq!(ev["type"], "execution");
    assert_eq!(ev["plan_id"], "plan-bridge");
    assert_eq!(ev["event"]["type"], "task_started");
    assert_eq!(ev["event"]["task_id"], "task-1");

    // 3. PlanCompleted
    sender.publish(roko_core::DashboardEvent::PlanCompleted {
        plan_id: "plan-bridge".to_string(),
        success: true,
    });
    let ev: serde_json::Value =
        serde_json::from_str(&next_ws_text(&mut socket).await).expect("parse plan_completed");
    assert_eq!(ev["type"], "plan_completed");
    assert_eq!(ev["success"], true);

    let _ = socket.close(None).await;
    server.abort();
}

// ---------------------------------------------------------------------------
// Relay health
// ---------------------------------------------------------------------------

#[tokio::test]
async fn relay_health_returns_local_default() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/relay/health").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["connection"]["mode"], "local");
    assert_eq!(body["freshness"]["stale"], false);
}

// ---------------------------------------------------------------------------
// Truth map
// ---------------------------------------------------------------------------

#[tokio::test]
async fn truth_map_returns_all_entity_kinds() {
    let (_dir, app) = test_app();
    let (status, body) = get_json(&app, "/api/truth_map").await;

    assert_eq!(status, StatusCode::OK);
    let entries = body.as_array().expect("truth_map should be an array");
    assert!(entries.len() >= 10, "expected at least 10 entity kinds");
    // Verify each entry has the expected fields.
    for entry in entries {
        assert!(entry.get("kind").is_some(), "missing kind field");
        assert!(entry.get("source").is_some(), "missing source field");
        assert!(entry.get("read_path").is_some(), "missing read_path field");
    }
}

// ---------------------------------------------------------------------------
// Server state persistence roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_persistence_roundtrip() {
    let (dir, state, app) = test_app_state();

    // Create a job via the API.
    let (status, created) = post_json(
        &app,
        "/api/jobs",
        serde_json::json!({
            "title": "Persistence test job",
            "description": "Should survive a roundtrip."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let job_id = created["id"].as_str().expect("job id");

    // Verify the job file exists on disk.
    let job_path = dir
        .path()
        .join(".roko")
        .join("jobs")
        .join(format!("{job_id}.json"));
    assert!(job_path.exists(), "job file should be persisted to disk");

    // Read back via the API.
    let (status, fetched) = get_json(&app, &format!("/api/jobs/{job_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["title"], "Persistence test job");

    state.shutdown().await;
}

/// Verify unmapped `DashboardEvent` variants are silently dropped (no panic).
#[tokio::test]
async fn bridge_drops_unmapped_events_without_panic() {
    let (_dir, state, _app) = test_app_state();
    let _bridge = roko_serve::start_orchestrator_event_bridge(Arc::clone(&state));

    let mut rx = state.event_bus.subscribe();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let sender = state.state_hub.sender();

    // Publish an unmapped event (CascadeRouterUpdated has no ServerEvent).
    sender.publish(roko_core::DashboardEvent::CascadeRouterUpdated {
        snapshot_json: "{}".to_string(),
    });

    // Then publish a mapped event that WILL come through.
    sender.publish(roko_core::DashboardEvent::Error {
        message: "sentinel".to_string(),
    });

    // The first event on EventBus should be the Error, not CascadeRouterUpdated.
    let envelope = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("should receive within 2s")
        .expect("recv should succeed");
    match &envelope.payload {
        roko_serve::events::ServerEvent::Error { message } => {
            assert_eq!(message, "sentinel");
        }
        other => panic!("expected Error, got: {other:?}"),
    }
}
