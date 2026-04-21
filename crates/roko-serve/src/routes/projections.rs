//! StateHub-backed projection routes for remote read and watch flows.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tracing::warn;

use roko_core::dashboard_snapshot::{DashboardEvent, DashboardSnapshot};

use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/projections/{name}", get(get_projection))
        .route("/projections/{name}/stream", get(stream_projection))
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProjectionQuery {
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn get_projection(
    Path(name): Path<String>,
    Query(query): Query<ProjectionQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let snapshot = state.state_hub.current_snapshot();
    let state_value = projection_state_value(&snapshot, &name, &query)?;
    Ok(Json(projection_state_frame(
        &name,
        state.state_hub.total_published(),
        state_value,
    )))
}

async fn stream_projection(
    Path(name): Path<String>,
    Query(query): Query<ProjectionQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let snapshot = state.state_hub.current_snapshot();
    let cursor = state.state_hub.total_published();
    let initial_state = projection_state_value(&snapshot, &name, &query)?;
    let initial = Event::default()
        .event("state")
        .id(cursor.to_string())
        .data(projection_state_frame(&name, cursor, initial_state).to_string());

    let name_for_stream = name.clone();
    let query_for_stream = query.clone();
    let delta_stream = stream::unfold(state.state_hub.subscribe_events(), move |mut rx| {
        let name = name_for_stream.clone();
        let query = query_for_stream.clone();
        async move {
            loop {
                match rx.recv().await {
                    Ok(envelope) => {
                        if !projection_accepts_event(&name, &query, &envelope.payload) {
                            continue;
                        }
                        let event = Event::default()
                            .event("delta")
                            .id(envelope.seq.to_string())
                            .data(
                                projection_delta_frame(&name, envelope.seq, &envelope.payload)
                                    .to_string(),
                            );
                        return Some((Ok(event), rx));
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(projection = %name, skipped, "projection stream lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    });

    Ok(
        Sse::new(stream::once(async move { Ok(initial) }).chain(delta_stream))
            .keep_alive(KeepAlive::default()),
    )
}

fn projection_state_frame(name: &str, cursor: u64, state: Value) -> Value {
    let cursor = format!("0x{cursor:x}");
    json!({
        "name": name,
        "channel": format!("projection:{name}"),
        "cursor": cursor,
        "freshness": {
            "state": "live",
            "cursor": cursor,
        },
        "state": state,
    })
}

fn projection_delta_frame(name: &str, cursor: u64, delta: &DashboardEvent) -> Value {
    json!({
        "type": "delta",
        "channel": format!("projection:{name}"),
        "cursor": format!("0x{cursor:x}"),
        "delta": delta,
    })
}

fn projection_state_value(
    snapshot: &DashboardSnapshot,
    name: &str,
    query: &ProjectionQuery,
) -> Result<Value, ApiError> {
    let value = match name {
        "dashboard" | "dashboard_snapshot" => json!(snapshot),
        "cohort_health" => json!({
            "stats": snapshot.stats,
            "agent_topology": snapshot.agent_topology,
            "cfactor_trend": snapshot.cfactor_trend,
            "efficiency_trend": snapshot.efficiency_trend,
            "roster_size": snapshot.agents.len(),
        }),
        "active_tasks" => json!({
            "items": snapshot
                .tasks
                .values()
                .filter(|task| task_matches_filter(task, query.filter.as_deref()))
                .take(query.limit.unwrap_or(usize::MAX))
                .cloned()
                .collect::<Vec<_>>(),
            "stats": snapshot.stats,
        }),
        "gate_pipeline" => json!({
            "gates": snapshot
                .gates
                .iter()
                .filter(|gate| gate_matches_filter(gate, query.filter.as_deref()))
                .take(query.limit.unwrap_or(usize::MAX))
                .cloned()
                .collect::<Vec<_>>(),
            "trends": snapshot.gate_trends,
            "recent_failures": snapshot.gate_recent_failures,
            "stats": {
                "passed": snapshot.stats.gates_passed,
                "failed": snapshot.stats.gates_failed,
            },
        }),
        "agent_trails" => json!({
            "items": snapshot
                .agents
                .values()
                .filter(|agent| agent_matches_filter(agent.agent_id.as_str(), query.filter.as_deref()))
                .take(query.limit.unwrap_or(usize::MAX))
                .cloned()
                .collect::<Vec<_>>(),
            "stats": snapshot.stats,
        }),
        "alerts" => json!({
            "diagnoses": snapshot.diagnoses,
            "recent_failures": snapshot.gate_recent_failures,
            "errors": snapshot.errors,
            "stats": snapshot.stats,
        }),
        "plans_list" => json!({
            "items": snapshot
                .plans
                .values()
                .take(query.limit.unwrap_or(usize::MAX))
                .cloned()
                .collect::<Vec<_>>(),
            "stats": snapshot.stats,
        }),
        "recent_episodes" => json!({
            "items": snapshot
                .episodes
                .iter()
                .filter(|ep| episode_matches_filter(ep, query.filter.as_deref()))
                .take(query.limit.unwrap_or(usize::MAX))
                .cloned()
                .collect::<Vec<_>>(),
            "stats": {
                "episodes_total": snapshot.stats.episodes_total,
            },
        }),
        _ => return Err(ApiError::not_found(format!("unknown projection '{name}'"))),
    };

    Ok(value)
}

fn projection_accepts_event(name: &str, query: &ProjectionQuery, event: &DashboardEvent) -> bool {
    match name {
        "dashboard" | "dashboard_snapshot" => true,
        "cohort_health" => matches!(
            event,
            DashboardEvent::PlanStarted { .. }
                | DashboardEvent::PlanCompleted { .. }
                | DashboardEvent::TaskStarted { .. }
                | DashboardEvent::TaskCompleted { .. }
                | DashboardEvent::AgentSpawned { .. }
                | DashboardEvent::EfficiencyEvent { .. }
                | DashboardEvent::CFactorTrendUpdated { .. }
                | DashboardEvent::Diagnosis { .. }
                | DashboardEvent::Error { .. }
        ),
        "active_tasks" => match event {
            DashboardEvent::TaskStarted { plan_id, .. }
            | DashboardEvent::TaskCompleted { plan_id, .. }
            | DashboardEvent::TaskPhaseChanged { plan_id, .. } => {
                plan_matches_filter(plan_id, query.filter.as_deref())
            }
            _ => false,
        },
        "gate_pipeline" => match event {
            DashboardEvent::GateResult { plan_id, .. } => {
                plan_matches_filter(plan_id, query.filter.as_deref())
            }
            _ => false,
        },
        "agent_trails" => match event {
            DashboardEvent::AgentSpawned { agent_id, .. }
            | DashboardEvent::AgentOutput { agent_id, .. } => {
                agent_matches_filter(agent_id, query.filter.as_deref())
            }
            _ => false,
        },
        "alerts" => matches!(
            event,
            DashboardEvent::Diagnosis { .. }
                | DashboardEvent::GateResult { passed: false, .. }
                | DashboardEvent::Error { .. }
        ),
        "plans_list" => matches!(
            event,
            DashboardEvent::PlanStarted { .. }
                | DashboardEvent::PlanCompleted { .. }
                | DashboardEvent::PhaseTransition { .. }
        ),
        "recent_episodes" => match event {
            DashboardEvent::EpisodeRecorded { role, .. } => {
                episode_role_matches_filter(role, query.filter.as_deref())
            }
            _ => false,
        },
        _ => false,
    }
}

fn task_matches_filter(
    task: &roko_core::dashboard_snapshot::TaskState,
    filter: Option<&str>,
) -> bool {
    filter.is_none_or(|value| match value.strip_prefix("plan:") {
        Some(plan_id) => task.plan_id == plan_id.trim(),
        None => true,
    })
}

fn gate_matches_filter(
    gate: &roko_core::dashboard_snapshot::GateVerdict,
    filter: Option<&str>,
) -> bool {
    filter.is_none_or(|value| {
        if let Some(plan_id) = value.strip_prefix("plan:") {
            return gate.plan_id == plan_id.trim();
        }
        if let Some(gate_name) = value.strip_prefix("gate:") {
            return gate.gate == gate_name.trim();
        }
        true
    })
}

fn plan_matches_filter(plan_id: &str, filter: Option<&str>) -> bool {
    filter.is_none_or(|value| value.strip_prefix("plan:").unwrap_or(value).trim() == plan_id)
}

fn agent_matches_filter(agent_id: &str, filter: Option<&str>) -> bool {
    filter.is_none_or(|value| value.strip_prefix("agent:").unwrap_or(value).trim() == agent_id)
}

fn episode_matches_filter(
    episode: &roko_core::dashboard_snapshot::EpisodeSummary,
    filter: Option<&str>,
) -> bool {
    filter.is_none_or(|value| {
        if let Some(role) = value.strip_prefix("role:") {
            return episode.role == role.trim();
        }
        if let Some(agent_id) = value.strip_prefix("agent:") {
            return episode.agent_id == agent_id.trim();
        }
        true
    })
}

fn episode_role_matches_filter(role: &str, filter: Option<&str>) -> bool {
    filter.is_none_or(|value| value.strip_prefix("role:").is_none_or(|r| r.trim() == role))
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use roko_core::config::ServeAuthConfig;
    use tower::ServiceExt;

    use crate::deploy::manual::ManualBackend;
    use crate::routes::build_router;
    use crate::runtime::NoOpRuntime;

    fn test_state() -> Arc<AppState> {
        let dir = tempfile::tempdir().expect("tempdir");
        Arc::new(AppState::new(
            dir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
            Arc::new(ManualBackend::default()),
        ))
    }

    #[tokio::test]
    async fn projection_query_returns_gate_pipeline_snapshot() {
        let state = test_state();
        state.state_hub.publish(DashboardEvent::GateResult {
            plan_id: "plan-1".into(),
            task_id: "task-1".into(),
            gate: "compile".into(),
            passed: true,
        });

        let response = build_router(Arc::clone(&state), &[], ServeAuthConfig::default())
            .oneshot(
                Request::builder()
                    .uri("/api/projections/gate_pipeline")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["state"]["gates"][0]["gate"], "compile");
        assert_eq!(payload["state"]["stats"]["passed"], 1);
    }

    #[test]
    fn projection_filters_agent_trails() {
        let snapshot = DashboardSnapshot {
            agents: [
                (
                    "agent-a".to_string(),
                    roko_core::dashboard_snapshot::AgentState {
                        agent_id: "agent-a".into(),
                        role: "implementer".into(),
                        active: true,
                        output_bytes: 1,
                    },
                ),
                (
                    "agent-b".to_string(),
                    roko_core::dashboard_snapshot::AgentState {
                        agent_id: "agent-b".into(),
                        role: "reviewer".into(),
                        active: false,
                        output_bytes: 2,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..DashboardSnapshot::default()
        };

        let value = projection_state_value(
            &snapshot,
            "agent_trails",
            &ProjectionQuery {
                filter: Some("agent:agent-b".into()),
                limit: None,
            },
        )
        .expect("projection");

        assert_eq!(value["items"].as_array().expect("items").len(), 1);
        assert_eq!(value["items"][0]["agent_id"], "agent-b");
    }
}
