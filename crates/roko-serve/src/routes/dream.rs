//! Dream cycle endpoint.

use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson};
use crate::state::{AppState, OperationHandle, OperationStatus};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/dream/run", post(dream_run))
        .route("/dream/journal", get(dream_journal))
}

#[derive(Debug, Deserialize)]
struct DreamRunRequest {
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "full".to_string()
}

impl RequestPayload for DreamRunRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        Ok(())
    }
}

/// `POST /api/dream/run` — trigger a dream consolidation cycle in the background.
async fn dream_run(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<DreamRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let workdir = state.workdir.clone();
    let mode = body.mode.clone();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        async move {
            bus.publish(ServerEvent::OperationStarted {
                op_id: op_id.clone(),
                kind: format!("dream_run:{mode}"),
            });

            let result = tokio::task::spawn_blocking(move || {
                let effort = if mode == "quick" { "low" } else { "medium" };
                let config = roko_dreams::DreamLoopConfig {
                    auto_dream: true,
                    idle_threshold_mins: 0,
                    min_episodes_for_dream: 0,
                    agent: roko_dreams::DreamAgentConfig {
                        command: "cat".to_string(),
                        args: Vec::new(),
                        model: None,
                        bare_mode: true,
                        effort: effort.to_string(),
                        fallback_model: None,
                        timeout_ms: 120_000,
                        env: Vec::new(),
                    },
                };
                let mut runner = roko_dreams::DreamRunner::new(workdir, config);
                runner.consolidate_now()
            })
            .await;

            let success = match result {
                Ok(Ok(_report)) => true,
                Ok(Err(err)) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("dream run failed: {err}"),
                    });
                    false
                }
                Err(err) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("dream run panicked: {err}"),
                    });
                    false
                }
            };

            bus.publish(ServerEvent::OperationCompleted {
                op_id,
                kind: "dream_run".into(),
                success,
            });
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: "dream_run".into(),
        status: OperationStatus::Running,
        handle,
    };
    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

/// `GET /api/dream/journal` — return the dream journal shaped for the
/// `DreamPhaseViz` component.
///
/// Returns `{ last_cycle, cycle_count, phases: [{ name, status, ... }] }`.
async fn dream_journal(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let journal_path = state.workdir.join(".roko/dreams/journal.jsonl");

    // Parse journal entries if the file exists.
    let entries: Vec<serde_json::Value> = if journal_path.exists() {
        let content = tokio::fs::read_to_string(&journal_path)
            .await
            .unwrap_or_default();
        content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    } else {
        Vec::new()
    };

    let cycle_count = entries.len();
    let last_cycle = entries
        .last()
        .and_then(|e| e.get("timestamp").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    // Build per-phase summaries from the latest cycle (or zeros).
    let phase_names = ["Hypnagogia", "NREM", "REM", "Integration"];
    let phases: Vec<serde_json::Value> = phase_names
        .iter()
        .map(|name| {
            // Look for phase data in the latest journal entry.
            let phase_data = entries
                .last()
                .and_then(|e| e.get("phases"))
                .and_then(|p| p.as_array())
                .and_then(|arr| arr.iter().find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name)));

            if let Some(data) = phase_data {
                json!({
                    "name": name,
                    "status": data.get("status").and_then(|v| v.as_str()).unwrap_or("pending"),
                    "episodes_processed": data.get("episodes_processed").and_then(|v| v.as_u64()).unwrap_or(0),
                    "clusters_formed": data.get("clusters_formed").and_then(|v| v.as_u64()).unwrap_or(0),
                    "knowledge_entries_written": data.get("knowledge_entries_written").and_then(|v| v.as_u64()).unwrap_or(0),
                    "playbooks_created": data.get("playbooks_created").and_then(|v| v.as_u64()).unwrap_or(0),
                    "duration_secs": data.get("duration_secs").and_then(|v| v.as_u64()).unwrap_or(0),
                    "trend": data.get("trend").cloned().unwrap_or(json!([])),
                })
            } else {
                // Synthesize from aggregate stats if no per-phase breakdown exists.
                let episodes_total = entries
                    .last()
                    .and_then(|e| e.get("episodes_processed").and_then(|v| v.as_u64()))
                    .unwrap_or(0);
                let status = if cycle_count > 0 { "completed" } else { "pending" };
                json!({
                    "name": name,
                    "status": status,
                    "episodes_processed": episodes_total / 4,
                    "clusters_formed": 0,
                    "knowledge_entries_written": 0,
                    "playbooks_created": 0,
                    "duration_secs": 0,
                    "trend": [],
                })
            }
        })
        .collect();

    Ok(Json(json!({
        "last_cycle": last_cycle,
        "cycle_count": cycle_count,
        "phases": phases,
    })))
}
