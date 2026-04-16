//! Tail the sidecar log file.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use roko_core::obs::LogScrubber;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::AgentState;

const DEFAULT_TAIL: usize = 200;
const MAX_TAIL: usize = 2000;

/// Logs routes.
pub fn router() -> Router<Arc<AgentState>> {
    Router::new().route("/logs", get(logs))
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    #[serde(default)]
    tail: Option<usize>,
}

#[derive(Debug, Serialize)]
struct LogsResponse {
    lines: Vec<String>,
    path: String,
}

async fn logs(State(state): State<Arc<AgentState>>, Query(query): Query<LogsQuery>) -> Response {
    state.metrics().record_request();

    let tail = query.tail.unwrap_or(DEFAULT_TAIL).min(MAX_TAIL);
    let log_path = state.log_path().to_path_buf();
    let response_path = log_path.display().to_string();

    match tokio::task::spawn_blocking(move || tail_file(&log_path, tail)).await {
        Ok(Ok(lines)) => {
            let scrubber = LogScrubber::default();
            let lines = lines
                .into_iter()
                .map(|line| scrubber.scrub(&line))
                .collect::<Vec<_>>();
            (
                StatusCode::OK,
                Json(LogsResponse {
                    lines,
                    path: response_path,
                }),
            )
                .into_response()
        }
        Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => {
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

fn tail_file(path: &Path, tail: usize) -> io::Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    if tail == 0 {
        return Ok(Vec::new());
    }

    let reader = BufReader::new(file);
    let mut lines = VecDeque::with_capacity(tail);
    for line in reader.lines() {
        let line = line?;
        if lines.len() == tail {
            let _ = lines.pop_front();
        }
        lines.push_back(line);
    }
    Ok(lines.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use roko_agent::chat_types::{ChatRequest, ChatResponse, FinishReason};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::features::messaging;
    use crate::state::{DispatchError, DispatchLike};

    fn log_path_for(agent_id: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join("roko-agent-server-tests")
            .join(agent_id)
            .join("log")
    }

    fn test_state(agent_id: &str, dispatcher: Option<Arc<dyn DispatchLike>>) -> Arc<AgentState> {
        let mut state = AgentState::new(
            agent_id.to_string(),
            None,
            "0.1.0".to_string(),
            vec!["messaging".to_string()],
            None,
            None,
            None,
        )
        .with_log_path(log_path_for(agent_id));
        if let Some(dispatcher) = dispatcher {
            state = state.with_message_dispatcher(dispatcher);
        }
        Arc::new(state)
    }

    #[derive(Clone)]
    struct MockDispatcher;

    #[async_trait]
    impl DispatchLike for MockDispatcher {
        async fn dispatch(&self, _request: ChatRequest) -> Result<ChatResponse, DispatchError> {
            Ok(ChatResponse {
                content: "logged response".to_string(),
                finish_reason: FinishReason::Stop,
                ..Default::default()
            })
        }
    }

    fn logs_request(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request")
    }

    fn message_request(prompt: &str) -> Request<Body> {
        Request::builder()
            .uri("/message")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "prompt": prompt }).to_string()))
            .expect("request")
    }

    #[tokio::test]
    async fn missing_log_file_returns_no_content() {
        let state = test_state(&format!("agent-{}", Uuid::new_v4()), None);
        let app = router().with_state(state);

        let response = app
            .oneshot(logs_request("/logs?tail=50"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn logs_tail_is_bounded_and_scrubbed() {
        let agent_id = format!("agent-{}", Uuid::new_v4());
        let state = test_state(&agent_id, None);
        let log_path = state.log_path().to_path_buf();
        let parent = log_path.parent().expect("parent");
        std::fs::create_dir_all(parent).expect("create log dir");

        let mut content = String::new();
        for index in 1..=2101 {
            content.push_str(&format!("line-{index}\n"));
        }
        std::fs::write(&log_path, content).expect("write log");

        let app = router().with_state(state);
        let response = app
            .oneshot(logs_request("/logs?tail=5000"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let lines = payload["lines"].as_array().expect("lines");
        assert_eq!(lines.len(), 2000);
        assert_eq!(
            lines.first().and_then(serde_json::Value::as_str),
            Some("line-102")
        );
        assert_eq!(
            lines.last().and_then(serde_json::Value::as_str),
            Some("line-2101")
        );
        assert_eq!(
            payload["path"].as_str(),
            Some(log_path.to_str().expect("path"))
        );
    }

    #[tokio::test]
    async fn message_request_emits_log_line_visible_via_logs_endpoint() {
        let state = test_state(
            &format!("agent-{}", Uuid::new_v4()),
            Some(Arc::new(MockDispatcher)),
        );
        let app = messaging::router().merge(router()).with_state(state);

        let message_response = app
            .clone()
            .oneshot(message_request("ping"))
            .await
            .expect("message response");
        assert_eq!(message_response.status(), StatusCode::OK);

        let logs_response = tokio::time::timeout(Duration::from_millis(100), async move {
            app.oneshot(logs_request("/logs?tail=50")).await
        })
        .await
        .expect("logs response within 100ms")
        .expect("logs response");
        assert_eq!(logs_response.status(), StatusCode::OK);

        let body = to_bytes(logs_response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let lines = payload["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("message") && line.contains("ping")),
            "expected a message log line, got {lines:?}"
        );
    }
}
