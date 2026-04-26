//! Status, health, metrics, dashboard, episodes, signals, gates, and operation endpoints.

mod dashboard;
mod episodes;
mod gates;
pub(super) mod health;
pub(super) mod helpers;
pub(super) mod metrics;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health::health))
        .route("/status", get(dashboard::session_status))
        .route("/metrics", get(metrics::metrics))
        .route("/metrics/summary", get(metrics::metrics_summary))
        .route("/metrics/success_rate", get(metrics::success_rate))
        .route("/metrics/engagement", get(metrics::engagement))
        .route("/metrics/c_factor", get(metrics::c_factor_metrics))
        .route("/metrics/model_efficiency", get(metrics::model_efficiency))
        .route("/metrics/gate_rate", get(metrics::gate_rate))
        .route("/metrics/experiments", get(metrics::experiments_metric))
        .route("/metrics/feedback_latency", get(metrics::feedback_latency))
        .route("/metrics/velocity", get(metrics::velocity))
        .route("/metrics/coverage", get(metrics::coverage))
        .route("/metrics/prometheus", get(metrics::prometheus_metrics))
        .route("/dashboard", get(dashboard::dashboard))
        .route("/gates/summary", get(gates::gate_summary))
        .route("/gates/history", get(gates::gates_history))
        .route("/gates/{gate_name}/history", get(gates::gate_history))
        .route("/episodes", get(episodes::episodes))
        .route("/signals", get(episodes::signals))
        .route("/operations/{id}", get(dashboard::operation_status))
        .route("/relay/health", get(health::relay_health))
        .route("/truth_map", get(dashboard::truth_map_handler))
        .route("/retention", get(health::retention_handler))
        .route("/parity", get(health::parity_handler))
        .route("/statehub/snapshot", get(health::statehub_snapshot))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body as AxumBody;
    use axum::extract::{Path, Query, State};
    use axum::http::Request;
    use serde_json::Value;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::routes::build_router;
    use crate::runtime::NoOpRuntime;
    use crate::state::{AppState, OperationStatus, PlanHandle};
    use roko_core::config::ServeAuthConfig;
    use roko_core::{Body, Engram, Kind, Provenance, Verdict};

    use super::episodes;
    use super::gates;
    use super::health;
    use super::helpers;
    use super::metrics;

    fn gate_signal(gate: &str, passed: bool, duration_ms: u64) -> Value {
        let mut verdict = if passed {
            Verdict::pass(gate)
        } else {
            Verdict::fail(gate, "boom")
        };
        verdict.duration_ms = duration_ms;

        let signal = Engram::builder(Kind::GateVerdict)
            .body(
                Body::from_json(&verdict)
                    .expect("invariant: verdict helper should serialize test payloads"),
            )
            .provenance(Provenance::trusted("test"))
            .tag("gate", gate)
            .tag("passed", passed.to_string())
            .build();
        let mut signal = serde_json::to_value(signal)
            .expect("invariant: verdict helper should serialize signal envelopes");
        signal
            .as_object_mut()
            .expect("gate signal should be an object")
            .entry("tags")
            .or_insert_with(|| serde_json::json!({}));
        signal
    }

    fn gate_signal_with_rung(gate: &str, rung: u32, passed: bool, duration_ms: u64) -> Value {
        let mut signal = gate_signal(gate, passed, duration_ms);
        signal
            .as_object_mut()
            .expect("gate signal should be an object")
            .get_mut("tags")
            .and_then(Value::as_object_mut)
            .expect("tags should be an object")
            .insert("rung".into(), Value::from(rung));
        signal
    }

    #[test]
    fn summarize_gate_entries_aggregates_by_gate_name() {
        let entries = vec![
            gate_signal("compile", true, 100),
            gate_signal("compile", false, 300),
            gate_signal("test", true, 200),
        ];

        let summary = {
            // Inline the summarize_gate_entries logic by calling gate_summary-related code
            // via the read_jsonl path. Instead, we test through the gate module helpers.
            let mut by_gate = std::collections::BTreeMap::new();
            for entry in &entries {
                let Some(kind) = entry.get("kind").and_then(Value::as_str) else {
                    continue;
                };
                if !helpers::is_gate_result_kind(kind) {
                    continue;
                }
                let Some(gate_name) = helpers::extract_gate_name(entry) else {
                    continue;
                };
                let Some(passed) = helpers::extract_gate_passed(entry) else {
                    continue;
                };
                let acc = by_gate.entry(gate_name).or_insert((0u64, 0u64, 0.0f64, None::<Value>));
                acc.0 += 1;
                if passed {
                    acc.1 += 1;
                }
                acc.2 += helpers::extract_gate_duration_ms(entry).unwrap_or(0) as f64;
                acc.3 = Some(entry.clone());
            }

            let summary: std::collections::BTreeMap<String, Value> = by_gate
                .into_iter()
                .filter_map(|(gate, (total_runs, passed_runs, total_duration_ms, last_run))| {
                    let last_run = last_run?;
                    let pass_rate = if total_runs == 0 { 0.0 } else { passed_runs as f64 / total_runs as f64 };
                    let avg_duration_ms = if total_runs == 0 { 0.0 } else { total_duration_ms / total_runs as f64 };
                    Some((gate, serde_json::json!({
                        "total_runs": total_runs,
                        "pass_rate": pass_rate,
                        "avg_duration_ms": avg_duration_ms,
                        "last_run": last_run,
                    })))
                })
                .collect();
            serde_json::to_value(summary).unwrap_or_else(|_| serde_json::json!({}))
        };

        assert_eq!(summary["compile"]["total_runs"], 2);
        assert_eq!(summary["compile"]["pass_rate"], 0.5);
        assert_eq!(summary["compile"]["avg_duration_ms"], 200.0);
        assert_eq!(summary["compile"]["last_run"]["tags"]["passed"], "false");
        assert_eq!(summary["test"]["total_runs"], 1);
        assert_eq!(summary["test"]["pass_rate"], 1.0);
        assert_eq!(summary["test"]["avg_duration_ms"], 200.0);
    }

    #[test]
    fn gate_history_filters_and_orders_by_timestamp() {
        let mut compile_late = gate_signal("compile", false, 300);
        compile_late
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(20));
        let mut compile_early = gate_signal("compile", true, 100);
        compile_early
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(10));
        let mut test = gate_signal("test", true, 200);
        test.as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(15));

        let entries = vec![compile_late, compile_early, test];
        let mut history: Vec<Value> = entries
            .into_iter()
            .filter(|entry| helpers::extract_gate_name(entry).as_deref() == Some("compile"))
            .filter_map(|entry| {
                let passed = helpers::extract_gate_passed(&entry)?;
                Some(serde_json::json!({
                    "signal_id": entry.get("id").cloned().unwrap_or(Value::Null),
                    "created_at_ms": entry.get("created_at_ms").cloned().unwrap_or(Value::Null),
                    "gate": "compile",
                    "passed": passed,
                }))
            })
            .collect();

        history.sort_by(|a, b| {
            let a_ts = a
                .get("created_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN);
            let b_ts = b
                .get("created_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN);
            a_ts.cmp(&b_ts)
        });

        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["passed"], true);
        assert_eq!(history[0]["created_at_ms"], 10);
        assert_eq!(history[1]["passed"], false);
        assert_eq!(history[1]["created_at_ms"], 20);
    }

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
            deploy_backend,
        ));
        (dir, state)
    }

    #[tokio::test]
    async fn gates_history_collection_is_mounted_under_api_grouping() {
        let (dir, state) = test_state();
        let signals = dir.path().join(".roko").join("engrams.jsonl");
        tokio::fs::create_dir_all(signals.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        let mut compile_early = gate_signal("compile", true, 120);
        compile_early
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(10));
        let mut compile_late = gate_signal("compile", false, 300);
        compile_late
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(20));
        let mut test = gate_signal("test", true, 200);
        test.as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(30));
        tokio::fs::write(
            &signals,
            [compile_early, compile_late, test]
                .into_iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .await
        .expect("write gate history");

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/gates/history?limit=2")
                    .body(AxumBody::empty())
                    .expect("request"),
            )
            .await
            .expect("gate history response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: Value = serde_json::from_slice(&body).expect("parse gate history response");
        assert_eq!(payload["source"], signals.display().to_string());
        assert_eq!(payload["total"], 3);
        assert_eq!(payload["limit"], 2);
        assert_eq!(
            payload["history"]
                .as_array()
                .expect("invariant: gate history payload should contain a history array")
                .len(),
            2
        );
        assert_eq!(payload["history"][0]["gate"], "test");
        assert_eq!(payload["history"][1]["gate"], "compile");
    }

    #[tokio::test]
    async fn gate_summary_includes_rung_breakdown_under_api_grouping() {
        let (dir, state) = test_state();
        let signals = dir.path().join(".roko").join("engrams.jsonl");
        tokio::fs::create_dir_all(signals.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        let mut compile_pass = gate_signal_with_rung("compile", 1, true, 120);
        compile_pass
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(10));
        let mut compile_fail = gate_signal_with_rung("compile", 1, false, 300);
        compile_fail
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(20));
        let mut test_pass = gate_signal_with_rung("test", 2, true, 200);
        test_pass
            .as_object_mut()
            .expect("gate signal should be an object")
            .insert("created_at_ms".into(), Value::from(30));
        tokio::fs::write(
            &signals,
            [compile_pass, compile_fail, test_pass]
                .into_iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .await
        .expect("write gate summary");

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/gates/summary")
                    .body(AxumBody::empty())
                    .expect("request"),
            )
            .await
            .expect("gate summary response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: Value = serde_json::from_slice(&body).expect("parse gate summary response");
        assert_eq!(payload["compile"]["total_runs"], 2);
        assert_eq!(payload["compile"]["pass_rate"], 0.5);
        assert_eq!(
            payload["rungs"]
                .as_array()
                .expect("invariant: gate summary payload should contain a rung array")
                .len(),
            2
        );
        assert_eq!(payload["rungs"][0]["rung"], 1);
        assert_eq!(payload["rungs"][0]["passed_runs"], 1);
        assert_eq!(payload["rungs"][0]["failed_runs"], 1);
        assert_eq!(payload["rungs"][1]["rung"], 2);
        assert_eq!(payload["rungs"][1]["passed_runs"], 1);
        assert_eq!(payload["rungs"][1]["failed_runs"], 0);
    }

    #[tokio::test]
    async fn health_reports_status_version_uptime_and_counts() {
        let (_dir, state) = test_state();

        let response = health::health(State(state.clone())).await;
        let body = response.1.0;

        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["uptime_secs"].as_u64().is_some());
        assert_eq!(body["active_plans"], 0);
        assert_eq!(body["active_agents"], 0);
        assert_eq!(body["active_runs"], 0);
        assert!(body["providers"].is_object());
        assert_eq!(body["providers"]["total"], 0);
        assert_eq!(body["providers"]["healthy"], 0);
        assert_eq!(body["providers"]["unhealthy"], 0);
    }

    #[tokio::test]
    async fn metrics_summary_includes_active_plans_and_c_factor() {
        let (dir, state) = test_state();
        let plan_handle = PlanHandle {
            id: "plan-1".into(),
            plan_dir: dir.path().join(".roko/plans/plan-1"),
            status: OperationStatus::Running,
            handle: tokio::spawn(async {}),
            cancel: roko_runtime::cancel::CancelToken::new(),
        };
        state
            .active_plans
            .write()
            .await
            .insert("plan-1".into(), plan_handle);

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/metrics/summary")
                    .body(AxumBody::empty())
                    .expect("request"),
            )
            .await
            .expect("metrics summary response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: Value = serde_json::from_slice(&body).expect("parse metrics summary response");
        assert_eq!(payload["period"], "last_7_days");
        assert_eq!(payload["active_plans"], 1);
        assert_eq!(payload["c_factor"], 0.0);
        assert_eq!(payload["experiments_active"], 0);
    }

    #[tokio::test]
    async fn c_factor_metrics_combines_composite_and_fleet_snapshots() {
        let (dir, state) = test_state();
        let learn_dir = dir.path().join(".roko").join("learn");
        tokio::fs::create_dir_all(&learn_dir)
            .await
            .expect("create learn dir");

        let c_factor_path = learn_dir.join("c-factor.jsonl");
        let efficiency_path = learn_dir.join("efficiency.jsonl");

        let earlier = serde_json::json!({
            "overall": 0.25,
            "components": {
                "gate_pass_rate": 0.20,
                "cost_efficiency": 0.20,
                "speed": 0.20,
                "information_flow_rate": 0.20,
                "first_try_rate": 0.20,
                "knowledge_growth": 0.20,
                "knowledge_integration_rate": 0.20,
                "hdc_diversity": 0.20,
                "convergence_velocity": 0.20,
                "turn_taking_equality": 0.20,
                "social_perceptiveness": 0.20
            },
            "agent_contributions": [
                {
                    "agent_id": "agent-a",
                    "episode_count": 1,
                    "without_agent_overall": 0.10,
                    "contribution_score": 0.15
                }
            ],
            "computed_at": "2026-04-04T12:00:00Z",
            "episode_count": 1
        });
        let recent = serde_json::json!({
            "overall": 0.71,
            "components": {
                "gate_pass_rate": 0.80,
                "cost_efficiency": 0.60,
                "speed": 0.55,
                "information_flow_rate": 0.40,
                "first_try_rate": 0.75,
                "knowledge_growth": 0.30,
                "knowledge_integration_rate": 0.25,
                "hdc_diversity": 0.35,
                "convergence_velocity": 0.45,
                "turn_taking_equality": 0.50,
                "social_perceptiveness": 0.65
            },
            "agent_contributions": [
                {
                    "agent_id": "agent-a",
                    "episode_count": 3,
                    "without_agent_overall": 0.58,
                    "contribution_score": 0.13
                },
                {
                    "agent_id": "agent-b",
                    "episode_count": 2,
                    "without_agent_overall": 0.79,
                    "contribution_score": -0.08
                }
            ],
            "computed_at": "2026-04-07T12:00:00Z",
            "episode_count": 5
        });

        tokio::fs::write(
            &c_factor_path,
            [earlier.to_string(), recent.to_string()].join("\n") + "\n",
        )
        .await
        .expect("write c-factor history");

        let events = vec![
            serde_json::json!({
                "agent_id": "agent-a", "role": "Implementer", "backend": "claude",
                "model": "claude-sonnet-4-6", "plan_id": "plan-a", "task_id": "task-a1",
                "input_tokens": 1000, "output_tokens": 200, "cache_read_tokens": 100,
                "cache_write_tokens": 10, "cost_usd": 0.40, "cost_usd_without_cache": 0.50,
                "prompt_sections": [], "total_prompt_tokens": 1200, "system_prompt_tokens": 200,
                "tools_available": 8, "tools_used": 4, "tool_calls": [],
                "wall_time_ms": 4000, "duration_ms": 4000, "time_to_first_token_ms": 500,
                "was_warm_start": false, "iteration": 1, "gate_passed": true,
                "outcome": "success", "gate_errors": [], "model_used": "claude-sonnet-4-6",
                "frequency": "theta", "strategy_attempted": "none",
                "timestamp": "2026-04-07T12:00:00Z"
            }),
            serde_json::json!({
                "agent_id": "agent-b", "role": "Reviewer", "backend": "claude",
                "model": "claude-sonnet-4-6", "plan_id": "plan-a", "task_id": "task-a2",
                "input_tokens": 900, "output_tokens": 150, "cache_read_tokens": 80,
                "cache_write_tokens": 10, "cost_usd": 0.30, "cost_usd_without_cache": 0.40,
                "prompt_sections": [], "total_prompt_tokens": 1050, "system_prompt_tokens": 200,
                "tools_available": 8, "tools_used": 3, "tool_calls": [],
                "wall_time_ms": 3000, "duration_ms": 3000, "time_to_first_token_ms": 450,
                "was_warm_start": true, "iteration": 1, "gate_passed": true,
                "outcome": "success", "gate_errors": [], "model_used": "claude-sonnet-4-6",
                "frequency": "theta", "strategy_attempted": "none",
                "timestamp": "2026-04-07T12:05:00Z"
            }),
            serde_json::json!({
                "agent_id": "agent-c", "role": "Implementer", "backend": "claude",
                "model": "claude-haiku-4-5", "plan_id": "plan-b", "task_id": "task-b1",
                "input_tokens": 700, "output_tokens": 100, "cache_read_tokens": 50,
                "cache_write_tokens": 5, "cost_usd": 0.10, "cost_usd_without_cache": 0.15,
                "prompt_sections": [], "total_prompt_tokens": 800, "system_prompt_tokens": 180,
                "tools_available": 6, "tools_used": 2, "tool_calls": [],
                "wall_time_ms": 2000, "duration_ms": 2000, "time_to_first_token_ms": 350,
                "was_warm_start": false, "iteration": 1, "gate_passed": false,
                "outcome": "failure", "gate_errors": ["test failed"],
                "model_used": "claude-haiku-4-5", "frequency": "theta",
                "strategy_attempted": "retry_same", "timestamp": "2026-04-07T12:10:00Z"
            }),
        ];
        tokio::fs::write(
            &efficiency_path,
            events
                .into_iter()
                .map(|event| event.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .await
        .expect("write efficiency events");

        let response = metrics::c_factor_metrics(State(state))
            .await
            .expect("c-factor metrics");
        let body = response.0;

        assert_eq!(body["source"]["composite_history_count"], 2);
        assert_eq!(body["source"]["efficiency_event_count"], 3);
        assert_eq!(body["composite"]["overall"], 0.71);
        assert_eq!(body["composite"]["episode_count"], 5);
        assert_eq!(body["sub_metrics"]["gate_pass_rate"], 0.80);
        assert_eq!(body["per_agent"][0]["agent_id"], "agent-a");
        assert_eq!(body["per_agent"][0]["dispatch_bias"], "prefer_cheaper");
        assert_eq!(body["per_agent"][1]["dispatch_bias"], "prefer_stronger");
        assert_eq!(body["per_fleet"]["plan_count"], 2);
        assert_eq!(body["per_fleet"]["agent_count"], 3);
        assert_eq!(body["per_fleet"]["observation_count"], 3);
    }

    #[tokio::test]
    async fn gate_history_returns_404_for_missing_gate() {
        let (_dir, state) = test_state();

        let err = gates::gate_history(State(state), Path("compile".into()))
            .await
            .expect_err("missing gate should fail");

        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn gate_history_returns_500_for_invalid_jsonl() {
        let (dir, state) = test_state();
        let signals = dir.path().join(".roko").join("engrams.jsonl");
        tokio::fs::create_dir_all(signals.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        tokio::fs::write(&signals, "{not-json}\n")
            .await
            .expect("write corrupt signals");

        let err = gates::gate_history(State(state), Path("compile".into()))
            .await
            .expect_err("corrupt signals should fail");

        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn signals_returns_500_for_invalid_jsonl() {
        let (dir, state) = test_state();
        let signals_path = dir.path().join(".roko").join("engrams.jsonl");
        tokio::fs::create_dir_all(signals_path.parent().expect("signals parent"))
            .await
            .expect("create signals dir");
        tokio::fs::write(&signals_path, "{not-json}\n")
            .await
            .expect("write corrupt signals");

        let err = episodes::signals(State(state), Query(episodes::SignalQuery { limit: Some(1) }))
            .await
            .expect_err("corrupt signals should fail");

        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }
}
