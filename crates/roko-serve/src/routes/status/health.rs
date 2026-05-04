//! Health, relay, parity, retention, and state-hub snapshot endpoints.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::projection_contract::{ProjectionQuery, RuntimeProjectionSet};
use crate::state::AppState;

/// `GET /api/health` — liveness check with live telemetry.
pub async fn health(State(state): State<Arc<AppState>>) -> (axum::http::StatusCode, Json<Value>) {
    let uptime_secs = state.started_at.elapsed().as_secs();
    // Use try_read() to avoid blocking on RwLock contention during plan runs,
    // which caused health-check timeouts and false "SERVE OFFLINE" in the demo UI.
    let active_plans = state.active_plans.try_read().map(|r| r.len()).unwrap_or(0);
    let supervised = state.supervisor.count().await;
    let discovered = state
        .discovered_agents
        .try_read()
        .map(|r| r.len())
        .unwrap_or(0);
    let active_agents = supervised.max(discovered);
    let active_runs = state.active_runs.try_read().map(|r| r.len()).unwrap_or(0);

    // Build a compact provider health summary from the tracker.
    let provider_snapshot = state.provider_health.snapshot();
    let providers_total = provider_snapshot.len();
    let providers_healthy = provider_snapshot
        .iter()
        .filter(|ps| ps.consecutive_failures == 0)
        .count();
    let providers_unhealthy = providers_total.saturating_sub(providers_healthy);
    let provider_summary = json!({
        "total": providers_total,
        "healthy": providers_healthy,
        "unhealthy": providers_unhealthy,
    });

    // Determine status: "ok" / "degraded" / "down"
    let status = if providers_total > 0 && providers_healthy == 0 {
        "down"
    } else if providers_unhealthy > 0 {
        "degraded"
    } else {
        "ok"
    };

    (
        axum::http::StatusCode::OK,
        Json(json!({
            "status": status,
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_secs": uptime_secs,
            "active_plans": active_plans,
            "active_agents": active_agents,
            "active_runs": active_runs,
            "providers": provider_summary,
            "statehub": {
                "cursor": format!("0x{:x}", state.state_hub.total_published()),
                "events_retained": state.state_hub.ring_len(),
                "snapshot": snapshot_health_summary(&state.state_hub.current_snapshot()),
            },
        })),
    )
}

/// `GET /api/relay/health` — return relay connection diagnostics.
pub async fn relay_health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let health = state.relay_health.read().clone();
    Json(serde_json::to_value(&health).unwrap_or_default())
}

/// `GET /api/parity` — return cross-surface parity matrix.
pub async fn parity_handler() -> Json<Value> {
    let matrix = crate::parity::build_parity_matrix();
    Json(serde_json::to_value(&matrix).unwrap_or_default())
}

/// `GET /api/retention` — return retention policies and any current violations.
pub async fn retention_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    let policies = crate::retention::default_retention_policies();
    let violations = crate::retention::check_retention(&state.workdir);
    let status = crate::retention::RetentionStatus {
        policies,
        violations,
    };
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// `GET /api/statehub/snapshot` — return the current state-hub dashboard snapshot.
pub async fn statehub_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let projections = RuntimeProjectionSet::load(&state).await?;
    let dashboard = projections.project("dashboard", &ProjectionQuery::default())?;
    Ok(Json(projections.state_frame("dashboard", dashboard)))
}

#[derive(Debug, Default, Deserialize)]
pub struct StateHubEventsQuery {
    #[serde(default)]
    after_seq: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default, alias = "type")]
    event_type: Option<String>,
}

/// `GET /api/statehub/events` — bounded replay of retained dashboard events.
pub async fn statehub_events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StateHubEventsQuery>,
) -> Json<Value> {
    let after_seq = query.after_seq.unwrap_or(0);
    let limit = query.limit.unwrap_or(256).min(1024);
    let events = state
        .state_hub
        .replay_from(after_seq)
        .into_iter()
        .filter(|envelope| dashboard_event_matches_query(&envelope.payload, &query))
        .take(limit)
        .map(|envelope| {
            json!({
                "seq": envelope.seq,
                "cursor": format!("0x{:x}", envelope.seq),
                "ts_millis": envelope.ts_millis,
                "event": envelope.payload,
            })
        })
        .collect::<Vec<_>>();

    Json(json!({
        "after_seq": after_seq,
        "limit": limit,
        "cursor": format!("0x{:x}", state.state_hub.total_published()),
        "events": events,
    }))
}

fn snapshot_health_summary(snapshot: &roko_core::dashboard_snapshot::DashboardSnapshot) -> Value {
    json!({
        "plans_active": snapshot.stats.plans_active,
        "tasks_active": snapshot.stats.tasks_active,
        "agents_active": snapshot.stats.agents_active,
        "gates_passed": snapshot.stats.gates_passed,
        "gates_failed": snapshot.stats.gates_failed,
        "episodes_total": snapshot.stats.episodes_total,
        "errors_total": snapshot.stats.errors_total,
        "cost_usd_total": snapshot.stats.cost_usd_total,
    })
}

fn dashboard_event_matches_query(
    event: &roko_core::dashboard_snapshot::DashboardEvent,
    query: &StateHubEventsQuery,
) -> bool {
    if let Some(expected_type) = query.event_type.as_deref() {
        if dashboard_event_type(event) != expected_type {
            return false;
        }
    }
    if let Some(expected_plan) = query.plan_id.as_deref().or(query.run_id.as_deref()) {
        if dashboard_event_plan_id(event) != Some(expected_plan) {
            return false;
        }
    }
    if let Some(expected_task) = query.task_id.as_deref() {
        if dashboard_event_task_id(event) != Some(expected_task) {
            return false;
        }
    }
    true
}

fn dashboard_event_type(event: &roko_core::dashboard_snapshot::DashboardEvent) -> &'static str {
    use roko_core::dashboard_snapshot::DashboardEvent;
    match event {
        DashboardEvent::PlanStarted { .. } => "plan_started",
        DashboardEvent::PlanCompleted { .. } => "plan_completed",
        DashboardEvent::TaskStarted { .. } => "task_started",
        DashboardEvent::TaskCompleted { .. } => "task_completed",
        DashboardEvent::TaskPhaseChanged { .. } => "task_phase_changed",
        DashboardEvent::AgentSpawned { .. } => "agent_spawned",
        DashboardEvent::AgentOutput { .. } => "agent_output",
        DashboardEvent::GateResult { .. } => "gate_result",
        DashboardEvent::PhaseTransition { .. } => "phase_transition",
        DashboardEvent::EfficiencyEvent { .. } => "efficiency_event",
        DashboardEvent::Diagnosis { .. } => "diagnosis",
        DashboardEvent::ExperimentWinnersUpdated { .. } => "experiment_winners_updated",
        DashboardEvent::CFactorTrendUpdated { .. } => "c_factor_trend_updated",
        DashboardEvent::EpisodeRecorded { .. } => "episode_recorded",
        DashboardEvent::TaskOutputAppended { .. } => "task_output_appended",
        DashboardEvent::EventLogEntry { .. } => "event_log_entry",
        DashboardEvent::CascadeRouterUpdated { .. } => "cascade_router_updated",
        DashboardEvent::GateThresholdsUpdated { .. } => "gate_thresholds_updated",
        DashboardEvent::AgentCompleted { .. } => "agent_completed",
        DashboardEvent::MarketplaceJobsUpdated { .. } => "marketplace_jobs_updated",
        DashboardEvent::AtelierPrdsUpdated { .. } => "atelier_prds_updated",
        DashboardEvent::KnowledgeEntriesUpdated { .. } => "knowledge_entries_updated",
        DashboardEvent::EfficiencyTrendUpdated { .. } => "efficiency_trend_updated",
        DashboardEvent::JobExecutionStarted { .. } => "job_execution_started",
        DashboardEvent::JobProgress { .. } => "job_progress",
        DashboardEvent::Error { .. } => "error",
    }
}

fn dashboard_event_plan_id(event: &roko_core::dashboard_snapshot::DashboardEvent) -> Option<&str> {
    use roko_core::dashboard_snapshot::DashboardEvent;
    match event {
        DashboardEvent::PlanStarted { plan_id }
        | DashboardEvent::PlanCompleted { plan_id, .. }
        | DashboardEvent::TaskStarted { plan_id, .. }
        | DashboardEvent::TaskCompleted { plan_id, .. }
        | DashboardEvent::TaskPhaseChanged { plan_id, .. }
        | DashboardEvent::GateResult { plan_id, .. }
        | DashboardEvent::PhaseTransition { plan_id, .. }
        | DashboardEvent::EfficiencyEvent { plan_id, .. }
        | DashboardEvent::EventLogEntry { plan_id, .. } => Some(plan_id),
        _ => None,
    }
}

fn dashboard_event_task_id(event: &roko_core::dashboard_snapshot::DashboardEvent) -> Option<&str> {
    use roko_core::dashboard_snapshot::DashboardEvent;
    match event {
        DashboardEvent::TaskStarted { task_id, .. }
        | DashboardEvent::TaskCompleted { task_id, .. }
        | DashboardEvent::TaskPhaseChanged { task_id, .. }
        | DashboardEvent::GateResult { task_id, .. }
        | DashboardEvent::EfficiencyEvent { task_id, .. }
        | DashboardEvent::TaskOutputAppended { task_id, .. }
        | DashboardEvent::EventLogEntry { task_id, .. } => Some(task_id),
        _ => None,
    }
}
