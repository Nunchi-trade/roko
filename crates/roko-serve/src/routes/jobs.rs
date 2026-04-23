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
        .route("/jobs/match", post(match_jobs))
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
    #[serde(default, rename = "state", alias = "status")]
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
    #[serde(
        default,
        serialize_with = "serialize_reward",
        deserialize_with = "deserialize_reward"
    )]
    reward: String,
    #[serde(default)]
    plan_id: String,
    #[serde(default)]
    auto_execute: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    committed_candidates: Vec<String>,
    #[serde(default)]
    metadata: serde_json::Value,
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deadline: Option<String>,
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
    #[serde(default, deserialize_with = "deserialize_reward")]
    reward: String,
    #[serde(default)]
    plan_id: String,
    #[serde(default)]
    auto_execute: bool,
    #[serde(default)]
    committed_candidates: Vec<String>,
    #[serde(default)]
    metadata: serde_json::Value,
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default)]
    deadline: Option<String>,
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
        validate_string_items_non_blank(&self.required_capabilities).map_err(|_| {
            ApiError::bad_request("job required_capabilities must not contain blank entries")
        })?;
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
    score: Option<f64>,
    #[serde(default)]
    feedback: String,
}
impl RequestPayload for EvaluateJobRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct MatchJobRequest {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default, alias = "language")]
    language: Option<String>,
    #[serde(default, alias = "minTier")]
    min_tier: Option<String>,
    #[serde(default, deserialize_with = "deserialize_reward")]
    reward: String,
    #[serde(default)]
    skills: Vec<String>,
}

impl RequestPayload for MatchJobRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        if self.title.trim().is_empty() {
            return Err(ApiError::bad_request("match title must not be blank"));
        }
        if self
            .min_tier
            .as_deref()
            .is_some_and(|tier| tier_index(tier).is_none())
        {
            return Err(ApiError::bad_request(format!(
                "unknown minimum tier '{}'; valid tiers: {}",
                self.min_tier.as_deref().unwrap_or_default(),
                TIER_ORDER.join(", ")
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MatchJobResponse {
    candidates: Vec<MatchCandidate>,
    total_fee: String,
    eta_hours: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MatchCandidate {
    agent_id: String,
    label: String,
    tier: String,
    reputation: u32,
    past_jobs: u32,
    inflight_jobs: u32,
    max_concurrent_jobs: u32,
    matched_skills: Vec<String>,
    bid_share: String,
}

const TIER_ORDER: &[&str] = &["Unverified", "Verified", "Trusted", "Expert", "Pioneer"];

fn tier_index(tier: &str) -> Option<usize> {
    TIER_ORDER.iter().position(|t| t.eq_ignore_ascii_case(tier))
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

async fn match_jobs(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<MatchJobRequest>,
) -> Result<Json<MatchJobResponse>, ApiError> {
    let agents = state.list_discovered_agents().await;
    let min_tier_idx = body.min_tier.as_deref().and_then(tier_index).unwrap_or(0);

    let mut req_skills: Vec<String> = body
        .skills
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(language) = body.language.as_deref() {
        let language = language.trim().to_ascii_lowercase();
        if !language.is_empty() && !req_skills.iter().any(|skill| skill == &language) {
            req_skills.push(language);
        }
    }

    let mut scored: Vec<(crate::state::DiscoveredAgent, f64, u32, u32, Vec<String>)> = Vec::new();
    for agent in agents {
        // Filter by tier
        let agent_tier_idx = agent.tier.as_deref().and_then(tier_index).unwrap_or(0);
        if agent_tier_idx < min_tier_idx {
            continue;
        }

        // Filter by skill overlap
        let agent_skills: Vec<String> = agent
            .skills
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let mut matched_skills = Vec::new();
        if !req_skills.is_empty() {
            matched_skills = req_skills
                .iter()
                .filter(|rs| agent_skills.iter().any(|agent_skill| agent_skill == *rs))
                .cloned()
                .collect();
            if matched_skills.is_empty() {
                continue;
            }
        }

        // Calculate load factor
        let inflight = count_agent_inflight_jobs(&state.workdir, &agent.agent_id).await;
        let max = if agent.max_concurrent_jobs > 0 {
            agent.max_concurrent_jobs
        } else {
            5
        };
        if inflight >= max {
            continue;
        }
        let load_factor = 1.0 - (inflight as f64 / max as f64).min(1.0);
        let tier_bonus = 1.0 + (agent_tier_idx as f64 * 0.05);
        let experience_bonus = 1.0 + ((agent.past_jobs_completed as f64).ln_1p() / 10.0);
        let skill_bonus = if req_skills.is_empty() {
            1.0
        } else {
            1.0 + (matched_skills.len() as f64 / req_skills.len() as f64)
        };
        let reputation = agent.reputation.max(1) as f64;
        let score = reputation * load_factor * tier_bonus * experience_bonus * skill_bonus;

        scored.push((agent, score, inflight, max, matched_skills));
    }

    // Sort descending by score, truncate to 5
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.0.reputation.cmp(&a.0.reputation))
            .then_with(|| a.0.agent_id.cmp(&b.0.agent_id))
    });
    scored.truncate(5);

    // Split reward proportionally by the same score used to rank candidates.
    let total_score: f64 = scored.iter().map(|(_, score, _, _, _)| *score).sum();
    let reward_str = body.reward.trim();
    // Try to extract numeric portion from reward like "2500 KORAI"
    let (reward_num, reward_suffix) = parse_reward(reward_str);

    let candidates: Vec<MatchCandidate> = scored
        .iter()
        .map(|(agent, score, inflight, max, matched_skills)| {
            let share = if total_score > 0.0 {
                (score / total_score * reward_num).round() as u64
            } else {
                0
            };
            MatchCandidate {
                agent_id: agent.agent_id.clone(),
                label: agent
                    .label
                    .clone()
                    .unwrap_or_else(|| agent.agent_id.clone()),
                tier: agent
                    .tier
                    .clone()
                    .unwrap_or_else(|| "Unverified".to_string()),
                reputation: agent.reputation,
                past_jobs: agent.past_jobs_completed,
                inflight_jobs: *inflight,
                max_concurrent_jobs: *max,
                matched_skills: matched_skills.clone(),
                bid_share: if reward_suffix.is_empty() {
                    share.to_string()
                } else {
                    format!("{share} {reward_suffix}")
                },
            }
        })
        .collect();

    // ETA heuristic
    let avg_rep = if candidates.is_empty() {
        0.0
    } else {
        candidates.iter().map(|c| c.reputation as f64).sum::<f64>() / candidates.len() as f64
    };
    let description_factor = (body.description.len() as f64 / 1200.0).min(1.0);
    let eta_hours = (48.0 - avg_rep / 100.0 * 24.0 + description_factor * 12.0)
        .max(4.0)
        .round() as u32;

    Ok(Json(MatchJobResponse {
        candidates,
        total_fee: reward_str.to_string(),
        eta_hours,
    }))
}

fn parse_reward(reward: &str) -> (f64, &str) {
    let reward = reward.trim();
    if reward.is_empty() {
        return (0.0, "");
    }
    // Find where the number ends and the suffix begins
    let num_end = reward
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(reward.len());
    let num_str = &reward[..num_end];
    let suffix = reward[num_end..].trim();
    let num = num_str.parse::<f64>().unwrap_or(0.0);
    (num, suffix)
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
        committed_candidates: body.committed_candidates.clone(),
        metadata: body.metadata.clone(),
        required_capabilities: trim_items(body.required_capabilities),
        deadline: body
            .deadline
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned),
        submission: None,
        evaluation: None,
    };
    write_job(&path, &job).await?;
    publish_job_event(&state, ServerEventKind::Created, &job)?;
    for candidate_id in &job.committed_candidates {
        state.event_bus.publish(ServerEvent::JobPostedToCandidate {
            job_id: job.id.clone(),
            agent_id: candidate_id.clone(),
            reward: job.reward.clone(),
        });
    }
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
    job.evaluation = Some(serde_json::json!({
        "accepted": body.accepted,
        "score": body.score,
        "feedback": body.feedback.trim(),
        "evaluated_at": Utc::now().to_rfc3339(),
    }));
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

/// Count in-flight (assigned or in_progress) jobs for a specific agent by scanning job files.
pub async fn count_agent_inflight_jobs(workdir: &Path, agent_id: &str) -> u32 {
    let dir = jobs_dir(workdir);
    if !dir.is_dir() {
        return 0;
    }
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut count = 0u32;
    while let Ok(Some(entry)) = entries.next_entry().await {
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
        let status = normalise_status(&job.status);
        if (status == "assigned" || status == "in_progress") && job.assigned_to == agent_id {
            count += 1;
        }
    }
    count
}

/// Serialize reward as a string, preserving any currency suffix (e.g. "2500 KORAI").
/// Empty values serialize as null for cleaner JSON.
fn serialize_reward<S: serde::Serializer>(value: &str, serializer: S) -> Result<S::Ok, S::Error> {
    if value.is_empty() {
        serializer.serialize_none()
    } else {
        serializer.serialize_str(value)
    }
}

/// Deserialize reward from either a number or a string.
fn deserialize_reward<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    use serde::de;
    struct RewardVisitor;
    impl de::Visitor<'_> for RewardVisitor {
        type Value = String;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a number or string")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(v.to_string())
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(v)
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(v.to_string())
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(v.to_string())
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(v.to_string())
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(String::new())
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(String::new())
        }
    }
    deserializer.deserialize_any(RewardVisitor)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    use crate::deploy::manual::ManualBackend;
    use crate::runtime::NoOpRuntime;
    use crate::state::{AgentRegistrationRecord, AppState};
    use roko_core::config::schema::RokoConfig;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let state = Arc::new(AppState::new(
            dir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            Arc::new(ManualBackend::default()),
        ));
        (dir, state)
    }

    #[tokio::test]
    async fn count_inflight_empty_dir() {
        let dir = tempdir().expect("tempdir");
        let count = count_agent_inflight_jobs(dir.path(), "agent-1").await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn count_inflight_counts_assigned_and_in_progress() {
        let dir = tempdir().expect("tempdir");
        let jobs = dir.path().join(".roko").join("jobs");
        tokio::fs::create_dir_all(&jobs).await.expect("jobs dir");

        // assigned to agent-1 → counted
        tokio::fs::write(
            jobs.join("j1.json"),
            r#"{"id":"j1","status":"assigned","assigned_to":"agent-1"}"#,
        )
        .await
        .unwrap();
        // in_progress for agent-1 → counted
        tokio::fs::write(
            jobs.join("j2.json"),
            r#"{"id":"j2","status":"in_progress","assigned_to":"agent-1"}"#,
        )
        .await
        .unwrap();
        // completed for agent-1 → NOT counted
        tokio::fs::write(
            jobs.join("j3.json"),
            r#"{"id":"j3","status":"completed","assigned_to":"agent-1"}"#,
        )
        .await
        .unwrap();
        // assigned to agent-2 → NOT counted
        tokio::fs::write(
            jobs.join("j4.json"),
            r#"{"id":"j4","status":"assigned","assigned_to":"agent-2"}"#,
        )
        .await
        .unwrap();

        let count = count_agent_inflight_jobs(dir.path(), "agent-1").await;
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn match_jobs_filters_by_tier_and_skills() {
        let (_dir, state) = test_state();

        // Register a Rust/Expert agent
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "rust-expert".into(),
                label: Some("Rust Expert".into()),
                tier: Some("Expert".into()),
                reputation: 95,
                skills: vec!["rust".into(), "networking".into()],
                max_concurrent_jobs: 5,
                ..Default::default()
            })
            .await;

        // Register a JS/Verified agent
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "js-dev".into(),
                label: Some("JS Dev".into()),
                tier: Some("Verified".into()),
                reputation: 70,
                skills: vec!["javascript".into(), "react".into()],
                max_concurrent_jobs: 3,
                ..Default::default()
            })
            .await;

        let result = match_jobs(
            axum::extract::State(Arc::clone(&state)),
            ValidJson(MatchJobRequest {
                title: "Implement p2p transport".into(),
                description: String::new(),
                language: None,
                min_tier: None,
                reward: "2500 KORAI".into(),
                skills: vec!["rust".into()],
            }),
        )
        .await
        .expect("match_jobs");

        assert_eq!(result.0.candidates.len(), 1);
        assert_eq!(result.0.candidates[0].agent_id, "rust-expert");
        assert_eq!(result.0.candidates[0].tier, "Expert");
        assert_eq!(result.0.total_fee, "2500 KORAI");
        assert!(result.0.eta_hours > 0);
    }

    #[tokio::test]
    async fn match_jobs_treats_language_as_required_skill_and_skips_loaded_agents() {
        let (dir, state) = test_state();

        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "busy-rust".into(),
                label: Some("Busy Rust".into()),
                tier: Some("Expert".into()),
                reputation: 99,
                skills: vec!["rust".into()],
                max_concurrent_jobs: 1,
                ..Default::default()
            })
            .await;
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "available-rust".into(),
                label: Some("Available Rust".into()),
                tier: Some("Trusted".into()),
                reputation: 80,
                skills: vec!["rust".into()],
                max_concurrent_jobs: 2,
                ..Default::default()
            })
            .await;

        let jobs = dir.path().join(".roko").join("jobs");
        tokio::fs::create_dir_all(&jobs).await.expect("jobs dir");
        tokio::fs::write(
            jobs.join("busy.json"),
            r#"{"id":"busy","status":"assigned","assigned_to":"busy-rust"}"#,
        )
        .await
        .unwrap();

        let result = match_jobs(
            axum::extract::State(Arc::clone(&state)),
            ValidJson(MatchJobRequest {
                title: "Implement Rust transport".into(),
                description: String::new(),
                language: Some("rust".into()),
                min_tier: Some("Trusted".into()),
                reward: "1000 KORAI".into(),
                skills: Vec::new(),
            }),
        )
        .await
        .expect("match_jobs");

        assert_eq!(result.0.candidates.len(), 1);
        assert_eq!(result.0.candidates[0].agent_id, "available-rust");
        assert_eq!(result.0.candidates[0].matched_skills, vec!["rust"]);
        assert_eq!(result.0.candidates[0].inflight_jobs, 0);
        assert_eq!(result.0.candidates[0].max_concurrent_jobs, 2);
    }

    #[test]
    fn match_job_validation_rejects_unknown_min_tier() {
        let req = MatchJobRequest {
            title: "Job".into(),
            description: String::new(),
            language: None,
            min_tier: Some("Principal".into()),
            reward: String::new(),
            skills: Vec::new(),
        };

        let err = req.validate_payload().expect_err("invalid tier");
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_reward_splits_number_and_suffix() {
        assert_eq!(parse_reward("2500 KORAI"), (2500.0, "KORAI"));
        assert_eq!(parse_reward("42"), (42.0, ""));
        assert_eq!(parse_reward(""), (0.0, ""));
    }

    // ── Integration test: full matchmaking flow ──

    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_router(state: Arc<AppState>) -> axum::Router {
        axum::Router::new()
            .nest("/api", super::routes())
            .nest("/api", crate::routes::agents::routes())
            .with_state(state)
    }

    async fn post_json(
        router: &axum::Router,
        uri: &str,
        body: serde_json::Value,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, val)
    }

    async fn get_json(
        router: &axum::Router,
        uri: &str,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, val)
    }

    #[tokio::test]
    async fn matchmaking_full_flow() {
        use serde_json::json;

        let (_dir, state) = test_state();
        let router = test_router(Arc::clone(&state));

        // Step 1: Register 3 agents
        let agents = vec![
            json!({
                "agent_id": "agent-alpha",
                "label": "Alpha Agent",
                "tier": "Expert",
                "reputation": 95,
                "skills": ["rust", "networking"],
                "max_concurrent_jobs": 5,
            }),
            json!({
                "agent_id": "agent-beta",
                "label": "Beta Agent",
                "tier": "Verified",
                "reputation": 70,
                "skills": ["rust", "testing"],
                "max_concurrent_jobs": 3,
            }),
            json!({
                "agent_id": "agent-gamma",
                "label": "Gamma Agent",
                "tier": "Verified",
                "reputation": 80,
                "skills": ["javascript", "react"],
                "max_concurrent_jobs": 4,
            }),
        ];
        for agent in &agents {
            let (status, _) = post_json(&router, "/api/agents/register", agent.clone()).await;
            assert_eq!(status, axum::http::StatusCode::OK, "register agent");
        }

        // Step 2: Match — only rust-skilled agents should appear
        let (status, match_result) = post_json(
            &router,
            "/api/jobs/match",
            json!({
                "title": "Build networking module",
                "skills": ["rust"],
                "reward": "2500 KORAI",
            }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let candidates = match_result["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        // Alpha ranks first (higher reputation)
        assert_eq!(candidates[0]["agentId"], "agent-alpha");
        assert_eq!(candidates[1]["agentId"], "agent-beta");
        // Gamma excluded (no rust skill)
        assert!(candidates.iter().all(|c| c["agentId"] != "agent-gamma"));
        assert_eq!(match_result["totalFee"], "2500 KORAI");
        assert!(match_result["etaHours"].as_u64().unwrap() > 0);

        // Step 3: Create job with committed_candidates from match
        let candidate_ids: Vec<String> = candidates
            .iter()
            .map(|c| c["agentId"].as_str().unwrap().to_string())
            .collect();
        let (status, job) = post_json(
            &router,
            "/api/jobs",
            json!({
                "title": "Build networking module",
                "description": "Implement p2p transport",
                "reward": "2500",
                "committed_candidates": candidate_ids,
            }),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::CREATED,
            "create_job response: {job}"
        );
        let job_id = job["id"].as_str().unwrap().to_string();
        assert_eq!(job["state"], "open", "full job body: {job}");
        let persisted_candidates = job["committed_candidates"].as_array().unwrap();
        assert_eq!(persisted_candidates.len(), 2);

        // Step 4: Assign to agent-alpha
        let (status, job) = post_json(
            &router,
            &format!("/api/jobs/{job_id}/assign"),
            json!({ "agent_id": "agent-alpha" }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(job["state"], "assigned");
        assert_eq!(job["assigned_to"], "agent-alpha");

        // Step 5: Start → Submit → Evaluate
        let (status, _) = post_json(&router, &format!("/api/jobs/{job_id}/start"), json!({})).await;
        assert_eq!(status, axum::http::StatusCode::OK);

        let (status, job) = post_json(
            &router,
            &format!("/api/jobs/{job_id}/submit"),
            json!({ "result_summary": "Networking module complete" }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(job["state"], "submitted");

        let (status, job) = post_json(
            &router,
            &format!("/api/jobs/{job_id}/evaluate"),
            json!({ "accepted": true, "feedback": "Great work" }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(job["state"], "completed");

        // Step 6: Verify final state via GET
        let (status, final_job) = get_json(&router, &format!("/api/jobs/{job_id}")).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(final_job["state"], "completed");
        assert_eq!(final_job["assigned_to"], "agent-alpha");
        assert!(final_job["submission"].is_object());
        assert!(final_job["evaluation"].is_object());

        // Verify events were emitted
        let events = state.event_bus.replay_from(0);
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.payload, ServerEvent::JobCreated { .. }))
        );
        assert!(events.iter().any(|e| matches!(
            &e.payload,
            ServerEvent::JobPostedToCandidate { agent_id, .. } if agent_id == "agent-alpha"
        )));
    }
}
