//! Jobs CRUD endpoints backed by `.roko/jobs/*.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{ApiError, validate_path_segment};
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson, validate_string_items_non_blank};
use crate::state::AppState;
use axum::extract::{Path as AxumPath, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use roko_core::JobStatus;
use serde::{Deserialize, Serialize};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/jobs", get(list_jobs).post(create_job))
        .route("/jobs/stats", get(job_stats))
        .route(
            "/jobs/{id}",
            get(get_job).patch(update_job).delete(cancel_job),
        )
        .route("/jobs/{id}/assign", post(assign_job))
        .route("/jobs/{id}/start", post(start_job))
        .route("/jobs/{id}/submit", post(submit_job))
        .route("/jobs/{id}/evaluate", post(evaluate_job))
        .route("/jobs/{id}/execute", post(execute_job_endpoint))
        .route("/jobs/{id}/cancel", post(cancel_job_endpoint))
}

const VALID_STATUSES: &[&str] = &[
    "open",
    "assigned",
    "in_progress",
    "submitted",
    "completed",
    "failed",
    "cancelled",
];
const TERMINAL_STATUSES: &[&str] = &["completed", "failed", "cancelled"];

fn valid_transitions(current: &str) -> &'static [&'static str] {
    match current {
        "open" => &["assigned", "in_progress", "cancelled"],
        "assigned" => &["in_progress", "open", "cancelled"],
        "in_progress" => &["submitted", "failed", "cancelled"],
        "submitted" => &["completed", "in_progress", "failed"],
        _ => &[],
    }
}

fn can_transition(current: &str, next: &str) -> bool {
    valid_transitions(current).contains(&next)
}
fn normalise_status(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}
fn is_terminal(status: &str) -> bool {
    TERMINAL_STATUSES.contains(&status)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JobRecord {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    job_type: String,
    #[serde(default, alias = "state")]
    status: String,
    #[serde(default)]
    posted_by: String,
    #[serde(default, alias = "assignee")]
    assigned_to: String,
    #[serde(default)]
    priority: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reward: String,
    #[serde(default)]
    plan_id: String,
    #[serde(default)]
    auto_execute: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    submission: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evaluation: Option<serde_json::Value>,
}

impl JobRecord {
    fn from_path(path: &Path, data: &str) -> Result<Self, ApiError> {
        let mut job: Self = serde_json::from_str(data).map_err(|error| {
            ApiError::internal(format!("parse job file '{}': {error}", path.display()))
        })?;
        if job.id.is_empty() {
            job.id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
        }
        Ok(job)
    }
}

#[derive(Debug, Deserialize)]
struct CreateJobRequest {
    #[serde(default)]
    id: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    job_type: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    posted_by: String,
    #[serde(default, alias = "assignee")]
    assigned_to: String,
    #[serde(default)]
    priority: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reward: String,
    #[serde(default)]
    plan_id: String,
    #[serde(default)]
    auto_execute: bool,
}

impl RequestPayload for CreateJobRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        if self.title.trim().is_empty() {
            return Err(ApiError::bad_request("job title must not be blank"));
        }
        if let Some(id) = self.id.as_deref() {
            if id.trim().is_empty() {
                return Err(ApiError::bad_request("job id must not be blank"));
            }
            validate_path_segment(id, "job id")?;
        }
        validate_string_items_non_blank(&self.tags)
            .map_err(|_| ApiError::bad_request("job tags must not contain blank entries"))?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct UpdateJobRequest {
    #[serde(default)]
    status: Option<String>,
    #[serde(default, alias = "assignee")]
    assigned_to: Option<String>,
}

impl RequestPayload for UpdateJobRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        if self.status.is_none() && self.assigned_to.is_none() {
            return Err(ApiError::bad_request(
                "request body must include status or assigned_to",
            ));
        }
        if self.status.as_ref().is_some_and(|s| s.trim().is_empty()) {
            return Err(ApiError::bad_request("job status must not be blank"));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct AssignJobRequest {
    agent_id: String,
}
impl RequestPayload for AssignJobRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        if self.agent_id.trim().is_empty() {
            return Err(ApiError::bad_request("agent_id must not be blank"));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct SubmitJobRequest {
    #[serde(default)]
    result_summary: String,
    #[serde(default)]
    artifacts: Vec<serde_json::Value>,
    #[serde(default)]
    gate_results: Vec<serde_json::Value>,
}
impl RequestPayload for SubmitJobRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct EvaluateJobRequest {
    accepted: bool,
    #[serde(default)]
    feedback: String,
}
impl RequestPayload for EvaluateJobRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        Ok(())
    }
}

#[derive(Debug, Default, Deserialize)]
struct JobListQuery {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    job_type: Option<String>,
    #[serde(default)]
    assigned_to: Option<String>,
}

async fn list_jobs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<JobListQuery>,
) -> Result<Json<Vec<JobRecord>>, ApiError> {
    let dir = jobs_dir(&state.workdir);
    if !dir.is_dir() {
        return Ok(Json(Vec::new()));
    }
    let filter_status = query.state.as_deref().and_then(JobStatus::parse);
    let mut jobs = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| ApiError::internal(format!("read jobs dir: {e}")))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| ApiError::internal(format!("read jobs entry: {e}")))?
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let data = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ApiError::internal(format!("read job '{}': {e}", path.display())))?;
        let job = JobRecord::from_path(&path, &data)?;
        if let Some(fs) = filter_status {
            if JobStatus::parse(&job.status) != Some(fs) {
                continue;
            }
        }
        if let Some(ref jt) = query.job_type {
            if !jt.is_empty() && job.job_type != *jt {
                continue;
            }
        }
        if let Some(ref assignee) = query.assigned_to {
            if !assignee.is_empty() && job.assigned_to != *assignee {
                continue;
            }
        }
        jobs.push(job);
    }
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    Ok(Json(jobs))
}

async fn get_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    Ok(Json(load_job(&state.workdir, &id).await?))
}

async fn create_job(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<CreateJobRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let id = body
        .id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    validate_path_segment(&id, "job id")?;
    let path = job_path(&state.workdir, &id);
    if path.exists() {
        return Err(ApiError::conflict(format!("job '{id}' already exists")));
    }
    let now = Utc::now().to_rfc3339();
    let job = JobRecord {
        id,
        title: body.title.trim().to_string(),
        description: body.description.trim().to_string(),
        job_type: non_empty_or_default(&body.job_type, "other"),
        status: non_empty_or_default(&body.status, "open"),
        posted_by: body.posted_by.trim().to_string(),
        assigned_to: body.assigned_to.trim().to_string(),
        priority: body.priority.trim().to_string(),
        created_at: now.clone(),
        updated_at: now,
        tags: trim_items(body.tags),
        reward: body.reward.trim().to_string(),
        plan_id: body.plan_id.trim().to_string(),
        auto_execute: body.auto_execute,
        submission: None,
        evaluation: None,
    };
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Created, &job)?;
    Ok((axum::http::StatusCode::CREATED, Json(job)))
}

async fn update_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    ValidJson(body): ValidJson<UpdateJobRequest>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    let path = job_path(&state.workdir, &id);
    let mut job = load_job(&state.workdir, &id).await?;
    let prev_status = job.status.clone();
    if let Some(ref new_status_raw) = body.status {
        let next = normalise_status(new_status_raw);
        if !VALID_STATUSES.contains(&next.as_str()) {
            return Err(ApiError::unprocessable_with_hint(
                format!("unknown job status '{next}'"),
                format!("valid statuses: {}", VALID_STATUSES.join(", ")),
            ));
        }
        let current = normalise_status(&prev_status);
        if current != next && !can_transition(&current, &next) {
            let allowed = valid_transitions(&current);
            let hint = if allowed.is_empty() {
                format!("'{current}' is a terminal state with no valid transitions")
            } else {
                format!(
                    "allowed transitions from '{}': {}",
                    current,
                    allowed.join(", ")
                )
            };
            return Err(ApiError::unprocessable_with_hint(
                format!("invalid status transition from '{current}' to '{next}'"),
                hint,
            ));
        }
        job.status = next;
    }
    if let Some(assigned_to) = body.assigned_to {
        job.assigned_to = assigned_to.trim().to_string();
    }
    job.updated_at = Utc::now().to_rfc3339();
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Updated, &job)?;
    if job.status != prev_status {
        publish_transition(&state, &job, &prev_status);
    }
    Ok(Json(job))
}

async fn assign_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    ValidJson(body): ValidJson<AssignJobRequest>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    let path = job_path(&state.workdir, &id);
    let mut job = load_job(&state.workdir, &id).await?;
    let current = normalise_status(&job.status);
    if current != "open" {
        return Err(ApiError::unprocessable_with_hint(
            format!("cannot assign job '{id}': current status is '{current}', expected 'open'"),
            format!("only jobs in 'open' state can be assigned; current state is '{current}'"),
        ));
    }
    let prev_status = job.status.clone();
    job.status = "assigned".to_string();
    job.assigned_to = body.agent_id.trim().to_string();
    job.updated_at = Utc::now().to_rfc3339();
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Updated, &job)?;
    publish_transition(&state, &job, &prev_status);
    Ok(Json(job))
}

async fn submit_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    ValidJson(body): ValidJson<SubmitJobRequest>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    let path = job_path(&state.workdir, &id);
    let mut job = load_job(&state.workdir, &id).await?;
    let current = normalise_status(&job.status);
    if current != "in_progress" {
        return Err(ApiError::unprocessable_with_hint(
            format!(
                "cannot submit job '{id}': current status is '{current}', expected 'in_progress'"
            ),
            format!(
                "only jobs in 'in_progress' state can be submitted; current state is '{current}'"
            ),
        ));
    }
    let prev_status = job.status.clone();
    job.status = "submitted".to_string();
    job.submission = Some(
        serde_json::json!({"result_summary": body.result_summary.trim(), "artifacts": body.artifacts, "gate_results": body.gate_results, "submitted_at": Utc::now().to_rfc3339()}),
    );
    job.updated_at = Utc::now().to_rfc3339();
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Updated, &job)?;
    publish_transition(&state, &job, &prev_status);
    Ok(Json(job))
}

async fn evaluate_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    ValidJson(body): ValidJson<EvaluateJobRequest>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    let path = job_path(&state.workdir, &id);
    let mut job = load_job(&state.workdir, &id).await?;
    let current = normalise_status(&job.status);
    if current != "submitted" {
        return Err(ApiError::unprocessable_with_hint(
            format!(
                "cannot evaluate job '{id}': current status is '{current}', expected 'submitted'"
            ),
            format!(
                "only jobs in 'submitted' state can be evaluated; current state is '{current}'"
            ),
        ));
    }
    let prev_status = job.status.clone();
    job.status = if body.accepted {
        "completed".to_string()
    } else {
        "in_progress".to_string()
    };
    job.evaluation = Some(
        serde_json::json!({"accepted": body.accepted, "feedback": body.feedback.trim(), "evaluated_at": Utc::now().to_rfc3339()}),
    );
    job.updated_at = Utc::now().to_rfc3339();
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Updated, &job)?;
    publish_transition(&state, &job, &prev_status);
    Ok(Json(job))
}

async fn cancel_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    let path = job_path(&state.workdir, &id);
    let mut job = load_job(&state.workdir, &id).await?;
    let current = normalise_status(&job.status);
    if is_terminal(&current) {
        return Err(ApiError::unprocessable_with_hint(
            format!("cannot cancel job '{id}': current status '{current}' is terminal"),
            format!("'{current}' is a terminal state with no valid transitions"),
        ));
    }
    let prev_status = job.status.clone();
    job.status = "cancelled".to_string();
    job.updated_at = Utc::now().to_rfc3339();
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Updated, &job)?;
    publish_transition(&state, &job, &prev_status);
    Ok(Json(job))
}

enum ServerEventKind {
    Created,
    Updated,
}

fn jobs_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("jobs")
}
fn job_path(workdir: &Path, id: &str) -> PathBuf {
    jobs_dir(workdir).join(format!("{id}.json"))
}

async fn load_job(workdir: &Path, id: &str) -> Result<JobRecord, ApiError> {
    let path = job_path(workdir, id);
    let data = tokio::fs::read_to_string(&path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ApiError::not_found(format!("job '{id}' not found")),
            _ => ApiError::internal(format!("read job '{}': {error}", path.display())),
        })?;
    JobRecord::from_path(&path, &data)
}

async fn write_job(path: &Path, job: &JobRecord) -> Result<(), ApiError> {
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::internal("invalid jobs path"))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| ApiError::internal(format!("create jobs dir: {e}")))?;
    let rendered = serde_json::to_string_pretty(job)
        .map_err(|e| ApiError::internal(format!("serialize job: {e}")))?;
    tokio::fs::write(path, rendered)
        .await
        .map_err(|e| ApiError::internal(format!("write job '{}': {e}", path.display())))?;
    Ok(())
}

fn publish_job_event(
    state: &AppState,
    kind: ServerEventKind,
    job: &JobRecord,
) -> Result<(), ApiError> {
    let payload = serde_json::to_value(job)
        .map_err(|e| ApiError::internal(format!("serialize job event: {e}")))?;
    match kind {
        ServerEventKind::Created => state
            .event_bus
            .publish(ServerEvent::JobCreated { job: payload }),
        ServerEventKind::Updated => state
            .event_bus
            .publish(ServerEvent::JobUpdated { job: payload }),
    }
    Ok(())
}

fn publish_transition(state: &AppState, job: &JobRecord, prev_status: &str) {
    state.event_bus.publish(ServerEvent::JobTransitioned {
        job_id: job.id.clone(),
        from: prev_status.to_string(),
        to: job.status.clone(),
        assigned_to: if job.assigned_to.is_empty() {
            None
        } else {
            Some(job.assigned_to.clone())
        },
    });
}

async fn job_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let dir = jobs_dir(&state.workdir);
    if !dir.is_dir() {
        return Ok(Json(
            serde_json::json!({"total": 0, "by_state": {}, "by_type": {}}),
        ));
    }
    let mut total = 0usize;
    let mut by_state: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut by_type: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| ApiError::internal(format!("read jobs dir: {e}")))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| ApiError::internal(format!("read jobs entry: {e}")))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let data = match tokio::fs::read_to_string(&path).await {
            Ok(d) => d,
            Err(_) => continue,
        };
        let job = match JobRecord::from_path(&path, &data) {
            Ok(j) => j,
            Err(_) => continue,
        };
        total += 1;
        let sk = if job.status.is_empty() {
            "open".to_string()
        } else {
            job.status.clone()
        };
        *by_state.entry(sk).or_default() += 1;
        let tk = if job.job_type.is_empty() {
            "other".to_string()
        } else {
            job.job_type.clone()
        };
        *by_type.entry(tk).or_default() += 1;
    }
    Ok(Json(
        serde_json::json!({"total": total, "by_state": by_state, "by_type": by_type}),
    ))
}

async fn start_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    let path = job_path(&state.workdir, &id);
    let mut job = load_job(&state.workdir, &id).await?;
    let current = normalise_status(&job.status);
    if current != "assigned" {
        return Err(ApiError::unprocessable_with_hint(
            format!("cannot start job '{id}': current status is '{current}', expected 'assigned'"),
            "only jobs in 'assigned' state can be started".to_string(),
        ));
    }
    let prev_status = job.status.clone();
    job.status = "in_progress".to_string();
    job.updated_at = Utc::now().to_rfc3339();
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Updated, &job)?;
    publish_transition(&state, &job, &prev_status);
    Ok(Json(job))
}

async fn cancel_job_endpoint(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    let path = job_path(&state.workdir, &id);
    let mut job = load_job(&state.workdir, &id).await?;
    let current = normalise_status(&job.status);
    if is_terminal(&current) {
        return Err(ApiError::unprocessable_with_hint(
            format!("cannot cancel job '{id}': current status '{current}' is terminal"),
            format!("'{current}' is a terminal state with no valid transitions"),
        ));
    }
    let prev_status = job.status.clone();
    job.status = "cancelled".to_string();
    job.updated_at = Utc::now().to_rfc3339();
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Updated, &job)?;
    publish_transition(&state, &job, &prev_status);
    Ok(Json(job))
}

async fn execute_job_endpoint(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    validate_path_segment(&id, "job id")?;
    let job = load_job(&state.workdir, &id).await?;
    let current = normalise_status(&job.status);
    if current != "open" && current != "assigned" {
        return Err(ApiError::unprocessable_with_hint(
            format!("cannot execute job '{id}': current status is '{current}'"),
            "only jobs in 'open' or 'assigned' state can be executed".to_string(),
        ));
    }

    let state_clone = Arc::clone(&state);
    let job_id = id.clone();
    tokio::spawn(async move {
        if let Err(err) = crate::job_runner::execute_job(&state_clone, &job_id).await {
            tracing::error!(job_id = %job_id, error = %err, "job execution failed");
        }
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "id": id,
            "status": "executing"
        })),
    ))
}

fn non_empty_or_default(value: &str, default: &str) -> String {
    let t = value.trim();
    if t.is_empty() {
        default.to_string()
    } else {
        t.to_string()
    }
}
fn trim_items(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}
