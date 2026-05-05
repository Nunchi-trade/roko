//! Serve lifecycle integration test: start, health check, shutdown, port release.
//!
//! Verifies the full server lifecycle without requiring real LLM keys or
//! external services.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use roko_core::config::schema::RokoConfig;
use roko_serve::deploy::create_backend;
use roko_serve::runtime::{CliRuntime, DashboardInfo, RunResult, SessionStatusInfo};
use roko_serve::state::AppState;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal no-op runtime for lifecycle tests.
struct LifecycleTestRuntime;

#[async_trait::async_trait]
impl CliRuntime for LifecycleTestRuntime {
    async fn run_once(
        &self,
        _workdir: &std::path::Path,
        _prompt: &str,
    ) -> anyhow::Result<RunResult> {
        Ok(RunResult {
            success: true,
            output_text: Some("lifecycle test output".to_string()),
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

/// Build a test `AppState` backed by a temporary directory.
async fn build_test_app_state(workdir: &std::path::Path) -> Arc<AppState> {
    // Ensure .roko/ dir exists so AppState can initialize its layout.
    std::fs::create_dir_all(workdir.join(".roko")).expect("create .roko dir");

    let mut config = RokoConfig::default();
    // Disable auth so /api/health is reachable without credentials in tests.
    config.serve.auth.enabled = false;
    let deploy =
        Arc::from(create_backend("manual", None, None, None).expect("manual deploy backend"));
    Arc::new(
        AppState::new(
            workdir.to_path_buf(),
            Arc::new(LifecycleTestRuntime),
            config,
            deploy,
        )
        .expect("AppState::new"),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full lifecycle: start server on random port, health check, shutdown, verify
/// port is released.
#[tokio::test]
async fn serve_start_health_shutdown() {
    // Bind to random port, capture port, release it.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Build app state with temp workdir.
    let tmp = tempfile::tempdir().unwrap();
    let state = build_test_app_state(tmp.path()).await;
    let cancel = state.cancel.clone();

    // Start server in background.
    let handle = tokio::spawn({
        let state = Arc::clone(&state);
        async move { roko_serve::run_server_with_state(state, "127.0.0.1", port).await }
    });

    // Poll readiness: wait for /api/health to return 200.
    let health_url = format!("http://127.0.0.1:{port}/api/health");
    let client = reqwest::Client::new();
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(resp) = client.get(&health_url).send().await {
                if resp.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        ready.is_ok(),
        "server did not become healthy at {health_url} within 5 seconds"
    );

    // Verify health response content.
    let resp = client.get(&health_url).send().await.unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    // Trigger graceful shutdown via the cancel token.
    cancel.cancel();

    // Server should exit cleanly within 5 seconds.
    let result = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(result.is_ok(), "server did not shut down within 5 seconds");

    // Verify port is released: we should be able to rebind it.
    let rebind = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(
        rebind.is_ok(),
        "port {port} should be released after server shutdown"
    );
}

/// Verify that multiple endpoints respond correctly before shutdown.
#[tokio::test]
async fn serve_endpoints_respond_before_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let tmp = tempfile::tempdir().unwrap();
    let state = build_test_app_state(tmp.path()).await;
    let cancel = state.cancel.clone();

    let handle = tokio::spawn({
        let state = Arc::clone(&state);
        async move { roko_serve::run_server_with_state(state, "127.0.0.1", port).await }
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    // Wait for readiness.
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(resp) = client.get(format!("{base_url}/api/health")).send().await {
                if resp.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(ready.is_ok(), "server did not start in time");

    // Check /api/status
    let resp = client
        .get(format!("{base_url}/api/status"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "/api/status should return 200");
    let status_body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        status_body.get("workdir").is_some(),
        "/api/status should contain workdir field"
    );

    // Check /api/plans (should be empty array)
    let resp = client
        .get(format!("{base_url}/api/plans"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "/api/plans should return 200");

    // Shutdown
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(result.is_ok(), "server did not shut down within 5 seconds");
}
