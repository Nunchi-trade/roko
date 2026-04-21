//! Jobs CRUD endpoints backed by `.roko/jobs/*.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{ApiError, validate_path_segment};
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson, validate_string_items_non_blank};
use crate::state::AppState;
use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/jobs", get(list_jobs).post(create_job))
        .route("/jobs/{id}", get(get_job).patch(update_job))
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
        if self
            .status
            .as_ref()
            .is_some_and(|status| status.trim().is_empty())
        {
            return Err(ApiError::bad_request("job status must not be blank"));
        }
        Ok(())
    }
}

/// `GET /api/jobs` — list all jobs from `.roko/jobs/`.
async fn list_jobs(State(state): State<Arc<AppState>>) -> Result<Json<Vec<JobRecord>>, ApiError> {
    let dir = jobs_dir(&state.workdir);
    if !dir.is_dir() {
        return Ok(Json(Vec::new()));
    }

    let mut jobs = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .map_err(|error| ApiError::internal(format!("read jobs dir: {error}")))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| ApiError::internal(format!("read jobs entry: {error}")))?
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let data = tokio::fs::read_to_string(&path).await.map_err(|error| {
            ApiError::internal(format!("read job '{}': {error}", path.display()))
        })?;
        jobs.push(JobRecord::from_path(&path, &data)?);
    }

    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    Ok(Json(jobs))
}

/// `GET /api/jobs/:id` — load a single job record.
async fn get_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;
    Ok(Json(load_job(&state.workdir, &id).await?))
}

/// `POST /api/jobs` — create a new durable job record.
async fn create_job(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<CreateJobRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let id = body
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
    };

    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Created, &job)?;

    Ok((axum::http::StatusCode::CREATED, Json(job)))
}

/// `PATCH /api/jobs/:id` — update job status and/or assignment.
async fn update_job(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    ValidJson(body): ValidJson<UpdateJobRequest>,
) -> Result<Json<JobRecord>, ApiError> {
    validate_path_segment(&id, "job id")?;

    let path = job_path(&state.workdir, &id);
    let mut job = load_job(&state.workdir, &id).await?;

    if let Some(status) = body.status {
        job.status = status.trim().to_string();
    }
    if let Some(assigned_to) = body.assigned_to {
        job.assigned_to = assigned_to.trim().to_string();
    }
    job.updated_at = Utc::now().to_rfc3339();

    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Updated, &job)?;

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
        .map_err(|error| ApiError::internal(format!("create jobs dir: {error}")))?;

    let rendered = serde_json::to_string_pretty(job)
        .map_err(|error| ApiError::internal(format!("serialize job: {error}")))?;
    tokio::fs::write(path, rendered)
        .await
        .map_err(|error| ApiError::internal(format!("write job '{}': {error}", path.display())))?;
    Ok(())
}

fn publish_job_event(
    state: &AppState,
    kind: ServerEventKind,
    job: &JobRecord,
) -> Result<(), ApiError> {
    let payload = serde_json::to_value(job)
        .map_err(|error| ApiError::internal(format!("serialize job event: {error}")))?;
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

fn non_empty_or_default(value: &str, default: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn trim_items(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}
