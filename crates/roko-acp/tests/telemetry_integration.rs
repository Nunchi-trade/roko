mod helpers;

use helpers::{
    MockPhaseResponse, MockResponse, create_test_session, create_test_session_with_workflow,
};
use roko_acp::types::JsonRpcNotification;
use roko_learn::cascade_router::CascadeRouter;
use serde_json::{Value, json};
use tempfile::TempDir;

fn has_usage_update(notification: &JsonRpcNotification) -> bool {
    if notification.method != "session/update" {
        return false;
    }

    notification
        .params
        .as_ref()
        .map(|params| params.get("update").unwrap_or(params))
        .and_then(|update| update.get("sessionUpdate"))
        .and_then(Value::as_str)
        == Some("usage_update")
}

#[tokio::test]
async fn single_dispatch_produces_episode() {
    let tmp = TempDir::new().expect("create tmpdir");
    let session = create_test_session(tmp.path());

    let result = session
        .mock_dispatch(
            "Fix the bug",
            MockResponse {
                text: "Done. Fixed the null check.".to_string(),
                input_tokens: 1_500,
                output_tokens: 200,
            },
        )
        .await
        .expect("dispatch should succeed");

    assert!(has_usage_update(
        result
            .notifications
            .iter()
            .find(|notification| has_usage_update(notification))
            .expect("usage update must be emitted")
    ));

    let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
    assert!(episodes_path.exists(), "episodes.jsonl must be created");

    let content = std::fs::read_to_string(&episodes_path).expect("read episodes");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one episode expected");

    let ep: serde_json::Value = serde_json::from_str(lines[0]).expect("parse episode");
    assert_eq!(ep["kind"], "acp-dispatch");
    assert_eq!(ep["extra"]["entry_point"], json!("acp"));
    assert_eq!(ep["extra"]["model"], json!("gpt-5.4"));
    assert_eq!(ep["extra"]["mode"], json!("code"));
    assert_eq!(ep["success"], json!(true));
    assert_eq!(ep["tokens_used"], json!(1_700));
    assert_eq!(ep["usage"]["input_tokens"], json!(1_500));
    assert_eq!(ep["usage"]["output_tokens"], json!(200));
    assert!(ep["usage"]["wall_ms"].as_u64().unwrap() > 0);
    assert!(ep["usage"]["cost_usd"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn single_dispatch_feeds_cascade_router() {
    let tmp = TempDir::new().expect("create tmpdir");
    let session = create_test_session(tmp.path());

    session
        .mock_dispatch(
            "Add error handling",
            MockResponse {
                text: "Added try-catch blocks.".to_string(),
                input_tokens: 2_000,
                output_tokens: 500,
            },
        )
        .await
        .expect("dispatch should succeed");

    let router_path = tmp
        .path()
        .join(".roko")
        .join("learn")
        .join("cascade-router.json");
    assert!(router_path.exists(), "cascade-router.json must be created");

    let router = CascadeRouter::load_or_new(&router_path, vec!["gpt-5.4".to_string()]);
    assert_eq!(router.total_observations(), 1);
    assert_eq!(router.confidence_snapshot().get("gpt-5.4"), Some(&(1, 1)));
}

#[tokio::test]
async fn single_dispatch_reports_usage() {
    let tmp = TempDir::new().expect("create tmpdir");
    let session = create_test_session(tmp.path());

    let result = session
        .mock_dispatch(
            "Refactor function",
            MockResponse {
                text: "Refactored.".to_string(),
                input_tokens: 3_000,
                output_tokens: 800,
            },
        )
        .await
        .expect("dispatch should succeed");

    assert!(result.total_tokens.is_some());
    assert!(result.total_tokens.unwrap() > 0);
    assert!(result.cost_usd.is_some());
    assert!(result.cost_usd.unwrap() > 0.0);
    assert!(result.notifications.iter().any(has_usage_update));
}

#[tokio::test]
async fn pipeline_produces_combined_telemetry() {
    let tmp = TempDir::new().expect("create tmpdir");
    let session = create_test_session_with_workflow(tmp.path(), "standard");

    let result = session
        .mock_pipeline_dispatch(
            "Add retry logic",
            vec![
                MockPhaseResponse::Implement("Added retry with backoff.".into(), 2_000, 600),
                MockPhaseResponse::GatePass,
                MockPhaseResponse::ReviewApprove("LGTM".into(), 1_000, 100),
            ],
        )
        .await
        .expect("pipeline dispatch should succeed");

    let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
    let content = std::fs::read_to_string(&episodes_path).expect("read episodes");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1, "pipeline = one episode");

    let ep: serde_json::Value = serde_json::from_str(lines[0]).expect("parse episode");
    assert_eq!(ep["kind"], "acp-pipeline-standard");
    assert_eq!(ep["success"], json!(true));
    assert!(ep["extra"]["phases_completed"].as_u64().unwrap() >= 2);
    assert_eq!(ep["tokens_used"], json!(3_700));
    assert_eq!(result.total_tokens, Some(3_700));
    assert!(result.cost_usd.unwrap() > 0.0);
    assert!(result.notifications.iter().any(has_usage_update));

    let router_path = tmp
        .path()
        .join(".roko")
        .join("learn")
        .join("cascade-router.json");
    let router = CascadeRouter::load_or_new(&router_path, vec!["gpt-5.4".to_string()]);
    assert!(router.total_observations() >= 2);
    assert!(
        router
            .confidence_snapshot()
            .get("gpt-5.4")
            .map(|(trials, _)| *trials >= 2)
            .unwrap_or(false)
    );
}

#[tokio::test]
async fn failed_dispatch_still_logs_episode() {
    let tmp = TempDir::new().expect("create tmpdir");
    let session = create_test_session(tmp.path());

    let result = session
        .mock_dispatch_failure("Fix bug", "connection timeout")
        .await
        .expect("failed model dispatch should still return an ACP prompt result");
    assert_eq!(result.total_tokens, None);

    let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
    let content = std::fs::read_to_string(&episodes_path).expect("read episodes");
    assert!(!content.is_empty());

    let ep: serde_json::Value =
        serde_json::from_str(content.lines().last().unwrap()).expect("parse failure episode");
    assert_eq!(ep["success"], json!(false));
    let failure_reason = ep["failure_reason"].as_str().expect("failure reason");
    assert!(!failure_reason.trim().is_empty());
}
