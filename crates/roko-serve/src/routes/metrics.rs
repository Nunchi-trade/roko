//! Top-level `GET /metrics` handler -- standard Prometheus scrape endpoint.
//!
//! Combines output from the [`MetricRegistry`] (labelled counters and histograms,
//! populated by provider dispatch and gate pipeline) with aggregate stats derived
//! from the state hub and runtime state.
//!
//! Prometheus expects this endpoint at the root `/metrics` path without an `/api`
//! prefix. The existing `/api/metrics/prometheus` route (in `routes/status/metrics`)
//! is preserved for backward compatibility.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;

use crate::state::AppState;

/// Prometheus text exposition format content type (OpenMetrics compatible).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Handler for `GET /metrics`.
///
/// Returns Prometheus text format output combining:
/// 1. `MetricRegistry` output (labelled per-provider/model/gate counters and histograms)
/// 2. State-hub and runtime aggregate stats (uptime, active agents, active plans)
///
/// Metric names emitted in part 2 are intentionally distinct from those registered in
/// `MetricRegistry` to avoid duplicate `# HELP` / `# TYPE` lines.
pub async fn metrics_handler(State(state): State<Arc<AppState>>) -> Response {
    let mut output = String::with_capacity(4096);

    // Part 1: MetricRegistry labelled output (standard + observability metrics).
    output.push_str(&state.metrics.render_prometheus());

    // Part 2: Runtime aggregate stats not covered by MetricRegistry.
    // These use metric names distinct from those in MetricRegistry to avoid
    // duplicate HELP/TYPE lines in the output.
    let uptime_secs = state.started_at.elapsed().as_secs();
    let _ = writeln!(output, "# HELP roko_uptime_seconds Server uptime in seconds");
    let _ = writeln!(output, "# TYPE roko_uptime_seconds gauge");
    let _ = writeln!(output, "roko_uptime_seconds {uptime_secs}");

    let active_agents = state.supervisor.count().await;
    let _ = writeln!(
        output,
        "# HELP roko_agents_active Number of currently active supervised agents"
    );
    let _ = writeln!(output, "# TYPE roko_agents_active gauge");
    let _ = writeln!(output, "roko_agents_active {active_agents}");

    let active_plans = state.active_plans.read().await.len();
    let _ = writeln!(
        output,
        "# HELP roko_plans_active Number of currently executing plans"
    );
    let _ = writeln!(output, "# TYPE roko_plans_active gauge");
    let _ = writeln!(output, "roko_plans_active {active_plans}");

    // Snapshot stats from the state hub (plans/tasks completed/failed, gate verdicts).
    let snapshot = state.state_hub.current_snapshot();
    let s = &snapshot.stats;
    let _ = writeln!(
        output,
        "# HELP roko_plans_completed_total Total plans completed successfully"
    );
    let _ = writeln!(output, "# TYPE roko_plans_completed_total counter");
    let _ = writeln!(output, "roko_plans_completed_total {}", s.plans_completed);

    let _ = writeln!(
        output,
        "# HELP roko_plans_failed_total Total plans that failed"
    );
    let _ = writeln!(output, "# TYPE roko_plans_failed_total counter");
    let _ = writeln!(output, "roko_plans_failed_total {}", s.plans_failed);

    let _ = writeln!(
        output,
        "# HELP roko_tasks_completed_total Total tasks completed"
    );
    let _ = writeln!(output, "# TYPE roko_tasks_completed_total counter");
    let _ = writeln!(output, "roko_tasks_completed_total {}", s.tasks_completed);

    let _ = writeln!(
        output,
        "# HELP roko_tasks_failed_total Total tasks that failed"
    );
    let _ = writeln!(output, "# TYPE roko_tasks_failed_total counter");
    let _ = writeln!(output, "roko_tasks_failed_total {}", s.tasks_failed);

    let _ = writeln!(
        output,
        "# HELP roko_errors_total Total error events recorded"
    );
    let _ = writeln!(output, "# TYPE roko_errors_total counter");
    let _ = writeln!(output, "roko_errors_total {}", s.errors_total);

    Response::builder()
        .status(StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            PROMETHEUS_CONTENT_TYPE,
        )
        .body(axum::body::Body::from(output))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::empty())
                .unwrap()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt as _;
    use roko_core::config::{RokoConfig, ServeAuthConfig};
    use tempfile::tempdir;
    use tower::ServiceExt as _;

    use crate::deploy::create_backend;
    use crate::routes::build_router;
    use crate::runtime::NoOpRuntime;

    fn build_test_state_and_router(
        config: RokoConfig,
    ) -> (tempfile::TempDir, Arc<AppState>, axum::Router) {
        let dir = tempdir().expect("tempdir");
        let deploy =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(
            AppState::new(
                dir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                config.clone(),
                deploy,
            )
            .expect("AppState::new"),
        );
        let router = build_router(Arc::clone(&state), &[], config.serve.auth.clone());
        (dir, state, router)
    }

    #[tokio::test]
    async fn get_metrics_returns_200_with_prometheus_content_type() {
        let config = RokoConfig {
            serve: roko_core::config::ServeConfig {
                auth: ServeAuthConfig {
                    enabled: false,
                    ..ServeAuthConfig::default()
                },
                ..Default::default()
            },
            ..RokoConfig::default()
        };
        let (_dir, _state, app) = build_test_state_and_router(config);

        let req = Request::builder()
            .uri("/metrics")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .expect("content-type header")
            .to_str()
            .expect("header str");
        assert!(
            ct.contains("text/plain"),
            "expected text/plain content type, got: {ct}"
        );

        let body = resp
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let text = String::from_utf8_lossy(&body);

        // MetricRegistry output should contain the registered standard + foundation metrics.
        assert!(
            text.contains("# HELP roko_llm_calls_total"),
            "missing roko_llm_calls_total in output"
        );
        assert!(
            text.contains("# TYPE roko_gate_verdicts_total counter"),
            "missing roko_gate_verdicts_total type line"
        );
        assert!(
            text.contains("roko_active_agents"),
            "missing roko_active_agents gauge"
        );

        // State-hub aggregate stats should be present.
        assert!(
            text.contains("roko_uptime_seconds"),
            "missing roko_uptime_seconds"
        );
        assert!(
            text.contains("roko_agents_active"),
            "missing roko_agents_active"
        );
        assert!(
            text.contains("roko_plans_active"),
            "missing roko_plans_active"
        );
    }

    #[tokio::test]
    async fn existing_api_metrics_prometheus_still_works() {
        let config = RokoConfig {
            serve: roko_core::config::ServeConfig {
                auth: ServeAuthConfig {
                    enabled: false,
                    ..ServeAuthConfig::default()
                },
                ..Default::default()
            },
            ..RokoConfig::default()
        };
        let (_dir, _state, app) = build_test_state_and_router(config);

        let req = Request::builder()
            .uri("/api/metrics/prometheus")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("roko_uptime_seconds"),
            "existing /api/metrics/prometheus should still work"
        );
    }

    #[tokio::test]
    async fn metrics_sink_returns_same_registry() {
        let dir = tempdir().expect("tempdir");
        let deploy =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = AppState::new(
            dir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            deploy,
        )
        .expect("AppState::new");

        let sink = state.metrics_sink();

        // The sink and state.metrics should point to the same registry.
        assert!(Arc::ptr_eq(&sink, &state.metrics));

        // Standard metrics should be registered.
        use roko_core::obs::metrics::LabelSet;
        assert!(
            state
                .metrics
                .get_counter("roko_llm_calls_total", &LabelSet::new())
                .is_some(),
            "roko_llm_calls_total should be registered"
        );
        assert!(
            state
                .metrics
                .get_gauge("roko_active_agents", &LabelSet::new())
                .is_some(),
            "roko_active_agents should be registered"
        );
        assert!(
            state
                .metrics
                .get_histogram("roko_llm_ttft_seconds", &LabelSet::new())
                .is_some(),
            "roko_llm_ttft_seconds should be registered"
        );
    }
}
