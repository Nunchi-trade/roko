//! Integration tests for workspace persistence across server restarts.
//!
//! Validates that workspace registry is persisted to `.roko/workspaces.json`
//! and survives construction of a new `AppState` using the same workdir.

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
use roko_serve::state::{workspace_registry_path_for, AppState};
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

/// Build a test app state and router backed by the given directory path.
fn build_app_at(workdir: &std::path::Path) -> (Arc<AppState>, axum::Router) {
    let config = RokoConfig::default();
    let deploy = Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
    let state = Arc::new(
        AppState::new(
            workdir.to_path_buf(),
            Arc::new(TestRuntime),
            config,
            deploy,
        )
        .expect("AppState::new"),
    );
    let auth = ServeAuthConfig {
        enabled: false,
        ..ServeAuthConfig::default()
    };
    let router = build_router(Arc::clone(&state), &[], auth);
    (state, router)
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
// Tests
// ---------------------------------------------------------------------------

/// POST /api/workspaces persists the registry to .roko/workspaces.json.
#[tokio::test]
async fn create_workspace_persists_registry() {
    let dir = tempdir().expect("tempdir");
    let (_state, router) = build_app_at(dir.path());

    let (status, body) = post_json(
        &router,
        "/api/workspaces",
        serde_json::json!({"prefix": "test-persist"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create workspace failed: {body:?}");

    let ws_id = body["id"].as_str().expect("workspace id");

    // Registry file should exist and contain the workspace.
    let registry_path = workspace_registry_path_for(dir.path());
    assert!(registry_path.exists(), "workspaces.json should exist");

    let registry_data = std::fs::read_to_string(&registry_path).expect("read registry");
    let registry: serde_json::Value =
        serde_json::from_str(&registry_data).expect("parse registry");

    assert!(
        registry["workspaces"][ws_id].is_object(),
        "workspace entry should be in registry"
    );
    assert_eq!(
        registry["workspaces"][ws_id]["id"].as_str(),
        Some(ws_id),
    );
}

/// Workspace survives construction of a new AppState with the same workdir.
#[tokio::test]
async fn workspace_survives_app_state_restart() {
    let dir = tempdir().expect("tempdir");

    // First "session": create a workspace.
    let (_state1, router1) = build_app_at(dir.path());
    let (status, body) = post_json(
        &router1,
        "/api/workspaces",
        serde_json::json!({"prefix": "restart-test"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ws_id = body["id"].as_str().expect("workspace id").to_string();

    // Second "session": build fresh AppState from same workdir.
    let (_state2, router2) = build_app_at(dir.path());
    let (status, body) = get_json(&router2, "/api/workspaces").await;
    assert_eq!(status, StatusCode::OK);

    let workspaces = body["workspaces"].as_array().expect("workspaces array");
    let found = workspaces
        .iter()
        .any(|ws| ws["id"].as_str() == Some(&ws_id));
    assert!(found, "workspace {ws_id} should be present after restart");
}

/// GET /api/workspaces/:id recreates a missing workspace path.
#[tokio::test]
async fn get_workspace_recreates_missing_path() {
    let dir = tempdir().expect("tempdir");
    let (_state, router) = build_app_at(dir.path());

    // Create a workspace.
    let (status, body) = post_json(
        &router,
        "/api/workspaces",
        serde_json::json!({"prefix": "recreate-test"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ws_id = body["id"].as_str().expect("workspace id").to_string();
    let ws_path = PathBuf::from(body["path"].as_str().expect("workspace path"));

    // Remove the workspace directory.
    std::fs::remove_dir_all(&ws_path).expect("remove workspace dir");
    assert!(!ws_path.exists());

    // GET should recreate it and return 200.
    let (status, body) = get_json(&router, &format!("/api/workspaces/{ws_id}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "workspace should be recreated: {body:?}"
    );

    // Directory should exist again.
    assert!(ws_path.exists(), "workspace path should be recreated");
}

/// GET /api/workspaces/:id returns 410 when the path cannot be recreated.
#[tokio::test]
async fn get_workspace_returns_gone_when_recreate_fails() {
    let dir = tempdir().expect("tempdir");
    let (state, router) = build_app_at(dir.path());

    // Seed a workspace entry whose parent path cannot be created
    // (place a file where the parent directory should be).
    let blocker_path = dir.path().join("blocker-file");
    std::fs::write(&blocker_path, b"not a directory").expect("write blocker");
    let fake_path = blocker_path.join("impossible-child").join("workspace");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let info = roko_serve::state::WorkspaceInfo {
        id: "gone-test-ws".to_string(),
        path: fake_path,
        created_at: now,
        last_accessed_at: now,
        status: roko_serve::state::WorkspaceStatus::Stale,
    };
    // Insert directly into the map (simulates a loaded stale entry).
    {
        let mut map = state.ephemeral_workspaces.write().await;
        map.insert("gone-test-ws".to_string(), info);
    }

    // GET should return 410 Gone.
    let (status, body) = get_json(&router, "/api/workspaces/gone-test-ws").await;
    assert_eq!(status, StatusCode::GONE, "expected 410 Gone: {body:?}");
    assert!(body["error"].as_str().unwrap_or("").contains("create a new workspace"));
    assert_eq!(body["id"].as_str(), Some("gone-test-ws"));
}

/// DELETE removes the workspace from the persisted registry.
#[tokio::test]
async fn delete_workspace_removes_registry_entry() {
    let dir = tempdir().expect("tempdir");
    let (_state, router) = build_app_at(dir.path());

    // Create a workspace.
    let (status, body) = post_json(
        &router,
        "/api/workspaces",
        serde_json::json!({"prefix": "delete-test"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ws_id = body["id"].as_str().expect("workspace id").to_string();

    // Delete it.
    let (status, _) = delete_json(&router, &format!("/api/workspaces/{ws_id}")).await;
    assert_eq!(status, StatusCode::OK);

    // Fresh AppState should not contain the deleted workspace.
    let (_state2, router2) = build_app_at(dir.path());
    let (status, body) = get_json(&router2, "/api/workspaces").await;
    assert_eq!(status, StatusCode::OK);

    let workspaces = body["workspaces"].as_array().expect("workspaces array");
    let found = workspaces
        .iter()
        .any(|ws| ws["id"].as_str() == Some(&ws_id));
    assert!(
        !found,
        "workspace {ws_id} should NOT be present after deletion"
    );
}
