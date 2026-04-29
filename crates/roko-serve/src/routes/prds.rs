//! PRD lifecycle endpoints — list, read, idea capture, draft, promote, plan.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use validator::Validate;

use crate::error::{ApiError, validate_path_segment};
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::state::{AppState, OperationHandle, OperationStatus};
use parking_lot::Mutex;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_runtime::event_bus::{EventBus, PublishOrigin, RokoEvent, global_event_bus};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/prds", get(list_prds))
        .route("/prds/ideas", post(post_idea))
        .route("/prds/status", get(prds_coverage))
        .route("/prd/consolidate", post(consolidate_prds))
        .route("/prds/consolidate", post(consolidate_prds))
        .route("/prds/{slug}", get(get_prd))
        .route("/prds/{slug}/draft", post(draft_prd))
        .route("/prds/{slug}/promote", post(promote_prd))
        .route("/prds/{slug}/plan", post(plan_from_prd))
}

const PRD_PUBLISH_AUDIT_KIND: &str = "prd_published";
const PRD_PUBLISH_DEDUPE_WINDOW: Duration = Duration::from_secs(60);

static RECENT_PUBLISHES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

fn recent_publishes() -> &'static Mutex<HashMap<String, Instant>> {
    RECENT_PUBLISHES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn clear_recent_publishes() {
    recent_publishes().lock().clear();
}

fn publish_dedupe_key(workdir: &std::path::Path, slug: &str) -> String {
    format!("{}::{slug}", workdir.display())
}

fn should_queue_publish(workdir: &std::path::Path, slug: &str, now: Instant) -> bool {
    let mut recent = recent_publishes().lock();
    recent.retain(|_, last_seen| {
        now.saturating_duration_since(*last_seen) < PRD_PUBLISH_DEDUPE_WINDOW
    });

    let key = publish_dedupe_key(workdir, slug);
    if recent.get(&key).is_some_and(|last_seen| {
        now.saturating_duration_since(*last_seen) < PRD_PUBLISH_DEDUPE_WINDOW
    }) {
        return false;
    }

    recent.insert(key, now);
    true
}

fn prd_publish_audit_path(workdir: &std::path::Path) -> PathBuf {
    workdir.join(".roko").join("episodes.jsonl")
}

pub(crate) async fn append_prd_published_episode(
    workdir: &std::path::Path,
    slug: &str,
    path: &std::path::Path,
    published_at: DateTime<Utc>,
    origin: PublishOrigin,
) {
    let logger = EpisodeLogger::new(prd_publish_audit_path(workdir));
    let mut episode = Episode::new("roko-serve", slug);
    episode.kind = PRD_PUBLISH_AUDIT_KIND.to_string();
    episode.agent_template = "serve".to_string();
    episode.trigger_kind = "prd_publish".to_string();
    episode.timestamp = published_at;
    episode.started_at = published_at;
    episode.completed_at = published_at;
    episode.success = true;
    episode.extra.insert(
        "slug".to_string(),
        serde_json::Value::String(slug.to_string()),
    );
    episode.extra.insert(
        "path".to_string(),
        serde_json::Value::String(path.display().to_string()),
    );
    episode.extra.insert(
        "origin".to_string(),
        serde_json::to_value(origin).unwrap_or(serde_json::Value::String("unknown".to_string())),
    );
    episode.extra.insert(
        "published_at".to_string(),
        serde_json::Value::String(published_at.to_rfc3339()),
    );

    if let Err(err) = logger.append(&episode).await {
        tracing::warn!(slug = %slug, error = %err, "failed to audit PRD publish event");
    }
}

async fn queue_plan_generation_after_publish(
    state: Arc<AppState>,
    slug: String,
    prd_path: PathBuf,
    prd_content: String,
) -> Option<String> {
    if !should_queue_publish(&state.workdir, &slug, Instant::now()) {
        tracing::trace!(slug = %slug, "skipping duplicate PRD publish within 60s");
        return None;
    }

    Some(queue_plan_generation_op(state, slug, prd_path, prd_content).await)
}

fn prd_published_event_from_episode(episode: &Episode) -> Option<RokoEvent> {
    if episode.kind != PRD_PUBLISH_AUDIT_KIND {
        return None;
    }

    let slug = episode.extra.get("slug")?.as_str()?.to_string();
    let path = PathBuf::from(episode.extra.get("path")?.as_str()?);
    let origin = serde_json::from_value::<PublishOrigin>(episode.extra.get("origin")?.clone())
        .ok()
        .unwrap_or(PublishOrigin::Import);
    let published_at = episode
        .extra
        .get("published_at")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or(episode.timestamp);

    Some(RokoEvent::PrdPublished {
        slug,
        path,
        published_at,
        origin,
    })
}

async fn handle_prd_published_event(
    state: Arc<AppState>,
    slug: String,
    path: PathBuf,
    origin: PublishOrigin,
) {
    let config = state.load_roko_config();
    if !config.serve.auto_orchestrate || !config.prd.auto_plan {
        tracing::trace!(slug = %slug, ?origin, "auto-orchestrate disabled; skipping PRD publish");
        return;
    }

    tracing::info!(slug = %slug, ?origin, "auto-orchestrating published PRD");

    let prd_content = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(err) => {
            tracing::warn!(
                slug = %slug,
                path = %path.display(),
                error = %err,
                "auto-orchestrate skipped because the published PRD could not be read"
            );
            return;
        }
    };

    let _ = queue_plan_generation_after_publish(state, slug, path, prd_content).await;
}

async fn follow_prd_published_audit(state: Arc<AppState>) {
    let episodes_path = prd_publish_audit_path(&state.workdir);
    let mut processed = EpisodeLogger::read_all_lossy(&episodes_path)
        .await
        .map_or(0, |episodes| episodes.len());
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = state.cancel.cancelled() => break,
            _ = interval.tick() => {
                let Ok(episodes) = EpisodeLogger::read_all_lossy(&episodes_path).await else {
                    continue;
                };
                if episodes.len() < processed {
                    processed = 0;
                }
                for episode in episodes.iter().skip(processed) {
                    let Some(RokoEvent::PrdPublished { slug, path, origin, .. }) =
                        prd_published_event_from_episode(episode)
                    else {
                        continue;
                    };
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        handle_prd_published_event(state, slug, path, origin).await;
                    });
                }
                processed = episodes.len();
            }
        }
    }
}

pub(crate) fn spawn_prd_publish_subscriber(
    state: Arc<AppState>,
    bus: &EventBus<RokoEvent>,
) -> JoinHandle<()> {
    let mut rx = bus.subscribe();

    tokio::spawn(async move {
        loop {
            let env = tokio::select! {
                _ = state.cancel.cancelled() => break,
                event = rx.recv() => match event {
                    Ok(env) => env,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "PRD publish subscriber lagged");
                        continue;
                    }
                },
            };

            if let RokoEvent::PrdPublished {
                slug, path, origin, ..
            } = env.payload
            {
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    handle_prd_published_event(state, slug, path, origin).await;
                });
            }
        }
    })
}

pub(crate) fn start_prd_publish_subscriber(state: Arc<AppState>) -> JoinHandle<()> {
    let audit_state = Arc::clone(&state);
    tokio::spawn(async move {
        follow_prd_published_audit(audit_state).await;
    });
    spawn_prd_publish_subscriber(state, global_event_bus())
}

/// Extract title from frontmatter or first heading.
fn extract_title(content: &str) -> String {
    let (fm, body) = split_frontmatter(content);
    if let Some(title) = fm.get("title").and_then(|v| v.as_str()) {
        if !title.is_empty() {
            return title.to_string();
        }
    }
    // Fall back to first markdown heading
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("# ") {
            return heading.trim().to_string();
        }
    }
    String::new()
}

/// Extract optional section from frontmatter.
fn extract_section(content: &str) -> Option<String> {
    let (fm, _) = split_frontmatter(content);
    fm.get("section")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Check if a plan exists for this slug.
async fn has_plan_for_slug(workdir: &std::path::Path, slug: &str) -> bool {
    let plans_dir = workdir.join(".roko").join("plans");
    if !plans_dir.is_dir() {
        return false;
    }
    // Check for plan file with matching slug
    for ext in &["json", "toml"] {
        if plans_dir.join(format!("{slug}.{ext}")).is_file() {
            return true;
        }
    }
    false
}

/// `GET /api/prds` — list PRDs from ideas/, drafts/, and published/.
async fn list_prds(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let prd_dir = state.workdir.join(".roko").join("prd");
    let mut entries = Vec::new();

    for (status, subdir) in [
        ("idea", "ideas"),
        ("draft", "drafts"),
        ("published", "published"),
    ] {
        let dir = prd_dir.join(subdir);
        if !dir.is_dir() {
            continue;
        }
        let mut rd = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| ApiError::internal(format!("read {subdir}: {e}")))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| ApiError::internal(format!("read entry: {e}")))?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
            let title = extract_title(&content);
            let section = extract_section(&content);
            let has_plan = has_plan_for_slug(&state.workdir, &slug).await;
            entries.push(json!({
                "slug": slug,
                "title": if title.is_empty() { slug.clone() } else { title },
                "status": status,
                "section": section,
                "has_plan": has_plan,
            }));
        }
    }

    Ok(Json(Value::Array(entries)))
}

/// `GET /api/prds/:slug` — read a PRD file and parse frontmatter.
async fn get_prd(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (status, content) = read_prd_file(&state.workdir, &slug).await?;

    // Attempt to split YAML frontmatter from body.
    let (frontmatter, body) = split_frontmatter(&content);

    Ok(Json(json!({
        "slug": slug,
        "status": status,
        "frontmatter": frontmatter,
        "body": body,
    })))
}

#[derive(Deserialize, Validate)]
struct IdeaRequest {
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
    text: String,
}

impl RequestPayload for IdeaRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// Generate a slug from text: first 5 words lowercased + 4-char random suffix.
fn slugify(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().take(5).collect();
    let base: String = words
        .join("-")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase();
    let suffix: String = uuid::Uuid::new_v4().to_string()[..4].to_string();
    if base.is_empty() {
        format!("idea-{suffix}")
    } else {
        format!("{base}-{suffix}")
    }
}

/// `POST /api/prds/ideas` — create a per-idea file and return its slug.
async fn post_idea(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<IdeaRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let prd_dir = state.workdir.join(".roko").join("prd");
    let ideas_dir = prd_dir.join("ideas");
    tokio::fs::create_dir_all(&ideas_dir)
        .await
        .map_err(|e| ApiError::internal(format!("create ideas dir: {e}")))?;

    let slug = slugify(&body.text);
    let idea_path = ideas_dir.join(format!("{slug}.md"));
    let now = chrono::Local::now().format("%Y-%m-%d");
    let content = format!(
        "---\ntitle: {title}\nstatus: idea\ncreated: {now}\n---\n\n{title}\n",
        title = body.text.trim(),
        now = now,
    );
    tokio::fs::write(&idea_path, content)
        .await
        .map_err(|e| ApiError::internal(format!("write idea file: {e}")))?;

    // Also append to legacy ideas.md for backwards compat
    let ideas_path = prd_dir.join("ideas.md");
    if ideas_path.exists() || !ideas_dir.join("..").join("ideas.md").exists() {
        use tokio::io::AsyncWriteExt;
        if !ideas_path.exists() {
            let header = "# Ideas\n\nQuick captures. Run `roko prd idea \"text\"` to append.\n\n";
            tokio::fs::write(&ideas_path, header)
                .await
                .map_err(|e| ApiError::internal(format!("write ideas header: {e}")))?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&ideas_path)
            .await
            .map_err(|e| ApiError::internal(format!("open ideas for append: {e}")))?;
        let line = format!("- {}\n", body.text);
        file.write_all(line.as_bytes())
            .await
            .map_err(|e| ApiError::internal(format!("append idea: {e}")))?;
    }

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "slug": slug })),
    ))
}

/// `POST /api/prds/:slug/draft` — spawn a background draft operation.
async fn draft_prd(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    validate_path_segment(&slug, "slug")?;

    let prd_dir = state.workdir.join(".roko").join("prd");
    let drafts_dir = prd_dir.join("drafts");
    tokio::fs::create_dir_all(&drafts_dir)
        .await
        .map_err(|e| ApiError::internal(format!("create drafts dir: {e}")))?;

    let draft_path = drafts_dir.join(format!("{slug}.md"));
    if !draft_path.exists() {
        tokio::fs::write(&draft_path, draft_scaffold(&slug))
            .await
            .map_err(|e| ApiError::internal(format!("write draft scaffold: {e}")))?;
    }

    let draft_content = tokio::fs::read_to_string(&draft_path)
        .await
        .map_err(|e| ApiError::internal(format!("read draft scaffold: {e}")))?;
    if draft_content.trim().is_empty() || is_prd_scaffold(&draft_content) {
        tokio::fs::write(&draft_path, draft_scaffold(&slug))
            .await
            .map_err(|e| ApiError::internal(format!("refresh draft scaffold: {e}")))?;
    }
    let draft_content = tokio::fs::read_to_string(&draft_path)
        .await
        .map_err(|e| ApiError::internal(format!("read draft scaffold: {e}")))?;
    let prompt = build_draft_prompt(&state.workdir, &slug, &draft_path, &draft_content);

    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        let slug = slug.clone();
        async move {
            bus.publish(ServerEvent::OperationStarted {
                op_id: op_id.clone(),
                kind: "prd_draft".into(),
            });

            match runtime.run_once(&workdir, &prompt).await {
                Ok(result) => {
                    bus.publish(ServerEvent::OperationCompleted {
                        op_id,
                        kind: "prd_draft".into(),
                        success: result.success,
                    });
                }
                Err(err) => {
                    tracing::warn!(
                        slug = %slug,
                        error = %err,
                        "PRD draft operation failed"
                    );
                    bus.publish(ServerEvent::Error {
                        message: format!("PRD draft failed for {slug}: {err}"),
                    });
                    bus.publish(ServerEvent::OperationCompleted {
                        op_id,
                        kind: "prd_draft".into(),
                        success: false,
                    });
                }
            }
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: format!("prd_draft:{slug}"),
        status: OperationStatus::Running,
        handle,
    };

    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

/// `POST /api/prds/:slug/promote` — move a PRD from drafts/ to published/.
async fn promote_prd(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    validate_path_segment(&slug, "slug")?;
    let prd_dir = state.workdir.join(".roko").join("prd");
    let src = prd_dir.join("drafts").join(format!("{slug}.md"));
    let dst = prd_dir.join("published").join(format!("{slug}.md"));

    if !src.is_file() {
        return Err(ApiError::not_found(format!("draft '{slug}' not found")));
    }
    if dst.is_file() {
        return Err(ApiError::conflict(format!(
            "published '{slug}' already exists"
        )));
    }

    tokio::fs::create_dir_all(prd_dir.join("published"))
        .await
        .map_err(|e| ApiError::internal(format!("create published dir: {e}")))?;

    tokio::fs::rename(&src, &dst)
        .await
        .map_err(|e| ApiError::internal(format!("promote: {e}")))?;

    let response_slug = slug.clone();
    let plan_slug = slug.clone();
    let mut response = json!({
        "slug": response_slug,
        "status": "published",
    });

    let published_at = Utc::now();
    append_prd_published_episode(
        &state.workdir,
        &slug,
        &dst,
        published_at,
        PublishOrigin::Http,
    )
    .await;

    let config = state.load_roko_config();
    if config.prd.auto_plan && config.serve.auto_orchestrate {
        match tokio::fs::read_to_string(&dst).await {
            Ok(prd_content) => {
                if let Some(plan_op_id) = queue_plan_generation_after_publish(
                    Arc::clone(&state),
                    plan_slug,
                    dst.clone(),
                    prd_content,
                )
                .await
                {
                    response["plan_generation"] = json!("queued");
                    response["plan_operation_id"] = json!(plan_op_id);
                }
            }
            Err(err) => {
                tracing::warn!(
                    slug = %slug,
                    path = %dst.display(),
                    error = %err,
                    "auto plan generation skipped because the promoted PRD could not be read"
                );
            }
        }
    }

    global_event_bus().emit(RokoEvent::PrdPublished {
        slug,
        path: dst,
        published_at,
        origin: PublishOrigin::Http,
    });

    Ok(Json(response))
}

/// `POST /api/prds/:slug/plan` — spawn background plan generation from PRD.
async fn plan_from_prd(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Verify the PRD exists.
    let (status, content) = read_prd_file(&state.workdir, &slug).await?;
    let prd_path = state
        .workdir
        .join(".roko")
        .join("prd")
        .join(match status.as_str() {
            "published" => "published",
            _ => "drafts",
        })
        .join(format!("{slug}.md"));
    let op_id = queue_plan_generation_op(Arc::clone(&state), slug, prd_path, content).await;

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

/// `GET /api/prds/status` — coverage report (draft/published/plan counts).
async fn prds_coverage(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let prd_dir = state.workdir.join(".roko").join("prd");
    let drafts = count_md_files(&prd_dir.join("drafts")).await;
    let published = count_md_files(&prd_dir.join("published")).await;
    let plans = count_entries(&state.workdir.join(".roko").join("plans")).await;

    Ok(Json(json!({
        "drafts": drafts,
        "published": published,
        "plans": plans,
        "total_prds": drafts + published,
    })))
}

/// `POST /api/prd/consolidate` — spawn background PRD consolidation.
async fn consolidate_prds(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        async move {
            bus.publish(ServerEvent::OperationStarted {
                op_id: op_id.clone(),
                kind: "prd_consolidate".into(),
            });

            let prompt = format!(
                "Scan all PRDs under {prd_dir} for gaps, duplicates, and missing coverage. \
                 Output a consolidation report summarizing findings.",
                prd_dir = workdir.join(".roko").join("prd").display(),
            );

            let success = match runtime.run_once(&workdir, &prompt).await {
                Ok(result) => result.success,
                Err(err) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("PRD consolidation failed: {err}"),
                    });
                    false
                }
            };

            bus.publish(ServerEvent::OperationCompleted {
                op_id,
                kind: "prd_consolidate".into(),
                success,
            });
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: "prd_consolidate".into(),
        status: OperationStatus::Running,
        handle,
    };
    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

// ── helpers ──────────────────────────────────────────────────────────

/// Read a PRD file, checking published/ first, then drafts/, then ideas/.
async fn read_prd_file(
    workdir: &std::path::Path,
    slug: &str,
) -> Result<(String, String), ApiError> {
    validate_path_segment(slug, "slug")?;
    let prd_dir = workdir.join(".roko").join("prd");

    for (status, subdir) in [
        ("published", "published"),
        ("draft", "drafts"),
        ("idea", "ideas"),
    ] {
        let path = prd_dir.join(subdir).join(format!("{slug}.md"));
        if path.is_file() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| ApiError::internal(format!("read prd: {e}")))?;
            return Ok((status.into(), content));
        }
    }

    Err(ApiError::not_found(format!("PRD '{slug}' not found")))
}

/// Split YAML frontmatter (between `---` delimiters) from the markdown body.
fn split_frontmatter(content: &str) -> (Value, &str) {
    if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let yaml_str = &rest[..end];
            let body = rest[end + 4..].trim_start_matches('\n');
            // Try to parse YAML as a JSON value; fall back to raw string.
            let fm = serde_json::from_str::<Value>(
                &serde_json::to_string(
                    &yaml_str
                        .lines()
                        .filter_map(|l| {
                            let mut parts = l.splitn(2, ':');
                            let k = parts.next()?.trim();
                            let v = parts.next()?.trim();
                            Some((k.to_string(), Value::String(v.to_string())))
                        })
                        .collect::<serde_json::Map<String, Value>>(),
                )
                .unwrap_or_default(),
            )
            .unwrap_or_else(|_| Value::String(yaml_str.to_string()));
            return (fm, body);
        }
    }
    (Value::Null, content)
}

/// Count `.md` files in a directory.
async fn count_md_files(dir: &std::path::Path) -> usize {
    count_entries_with(dir, |name| {
        std::path::Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    })
    .await
}

/// Count all entries in a directory.
async fn count_entries(dir: &std::path::Path) -> usize {
    count_entries_with(dir, |_| true).await
}

/// Count entries in a directory matching a predicate.
async fn count_entries_with(dir: &std::path::Path, pred: impl Fn(&str) -> bool) -> usize {
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    let mut count = 0;
    while let Ok(Some(entry)) = rd.next_entry().await {
        if let Some(name) = entry.file_name().to_str() {
            if pred(name) {
                count += 1;
            }
        }
    }
    count
}

/// Build the prompt used for PRD-to-plan generation.
fn build_plan_generation_prompt(
    workdir: &std::path::Path,
    prd_path: &std::path::Path,
    prd_content: &str,
) -> String {
    let plans_root = workdir.join(".roko").join("plans");
    format!(
        "Read the published PRD at {prd_path} and generate implementation plan directories under {plans_root}.\n\
         Each requirement should become one or more tasks.\n\
         Each acceptance criterion should become a task verification command.\n\
         Search the codebase first to understand what already exists.\n\
         Create or update plan.md and tasks.toml files directly, including per-task mcp_servers when a task needs a specific MCP server.\n\n\
         Project workspace: {workdir}\n\n\
         PRD content:\n{prd_content}\n",
        prd_path = prd_path.display(),
        plans_root = plans_root.display(),
        workdir = workdir.display(),
        prd_content = prd_content,
    )
}

fn draft_scaffold(slug: &str) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d");
    format!(
        "---\n\
         id: prd-{slug}\n\
         title: {slug}\n\
         status: draft\n\
         version: 1\n\
         created: {today}\n\
         updated: {today}\n\
         depends_on: []\n\
         crates: []\n\
         plans_generated: []\n\
         coverage: 0\n\
         tags: []\n\
         ---\n\n\
         # {slug}\n\n\
         ## Overview\n\n\
         ## Requirements\n\n\
         ## Acceptance criteria\n\n\
         ## Design\n\n\
         ## References\n"
    )
}

fn is_prd_scaffold(content: &str) -> bool {
    content
        .lines()
        .filter(|line| {
            !line.starts_with("---")
                && !line.starts_with('#')
                && !line.starts_with("##")
                && !line.trim().is_empty()
        })
        .count()
        == 0
}

fn build_draft_prompt(
    workdir: &std::path::Path,
    slug: &str,
    draft_path: &std::path::Path,
    draft_content: &str,
) -> String {
    let mut prompt = String::new();
    let _ = writeln!(prompt, "You are drafting a PRD inside a roko workspace.");
    let _ = writeln!(prompt, "Workspace: {}", workdir.display());
    let _ = writeln!(prompt, "Draft file: {}", draft_path.display());
    let _ = writeln!(prompt, "PRD slug: {slug}\n");
    let _ = writeln!(
        prompt,
        "Keep the document in `.roko/prd/drafts/{slug}.md` with YAML frontmatter and these sections:\n\
         Overview\nRequirements\nAcceptance criteria\nDesign\nReferences\n"
    );
    let _ = writeln!(
        prompt,
        "Search the codebase first, then write the completed PRD directly to the draft file. \
         Preserve the `.roko/prd` layout and keep the frontmatter in draft status."
    );
    let _ = writeln!(prompt, "\nCurrent draft content:\n{draft_content}\n");
    let _ = writeln!(
        prompt,
        "If the existing content is only scaffold text, fill it out. If it already contains substance, refine it without changing the file location."
    );
    prompt
}

/// Spawn a background plan-generation operation from a PRD body.
async fn queue_plan_generation_op(
    state: Arc<AppState>,
    slug: String,
    prd_path: std::path::PathBuf,
    prd_content: String,
) -> String {
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();
    let prompt = build_plan_generation_prompt(&workdir, &prd_path, &prd_content);

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        let slug = slug.clone();
        async move {
            bus.publish(ServerEvent::OperationStarted {
                op_id: op_id.clone(),
                kind: "prd_plan".into(),
            });
            match runtime.run_once(&workdir, &prompt).await {
                Ok(result) => {
                    bus.publish(ServerEvent::OperationCompleted {
                        op_id,
                        kind: "prd_plan".into(),
                        success: result.success,
                    });
                }
                Err(err) => {
                    tracing::warn!(
                        slug = %slug,
                        error = %err,
                        "plan generation failed"
                    );
                    bus.publish(ServerEvent::Error {
                        message: format!("plan generation failed for {slug}: {err}"),
                    });
                    bus.publish(ServerEvent::OperationCompleted {
                        op_id,
                        kind: "prd_plan".into(),
                        success: false,
                    });
                }
            }
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: format!("prd_plan:{slug}"),
        status: OperationStatus::Running,
        handle,
    };

    state.operations.write().await.insert(op_id.clone(), op);
    op_id
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use tempfile::tempdir;
    use tokio::sync::Notify;

    use crate::deploy::manual::ManualBackend;
    use crate::runtime::NoOpRuntime;
    use crate::runtime::{CliRuntime, DashboardInfo, RunResult, SessionStatusInfo};
    use roko_runtime::event_bus::{EventBus, PublishOrigin, RokoEvent};

    #[derive(Clone)]
    struct RecordingRuntime {
        calls: Arc<Mutex<Vec<(PathBuf, String)>>>,
        notify: Arc<Notify>,
        success: bool,
        call_count: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CliRuntime for RecordingRuntime {
        async fn run_once(
            &self,
            workdir: &std::path::Path,
            prompt: &str,
        ) -> anyhow::Result<RunResult> {
            self.calls
                .lock()
                .expect("lock calls")
                .push((workdir.to_path_buf(), prompt.to_string()));
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.notify.notify_waiters();
            Ok(RunResult {
                success: self.success,
                output_text: None,
                usage: None,
            })
        }

        fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo {
            SessionStatusInfo {
                session_id: None,
                workdir,
                daemon_running: false,
                signal_count: None,
                episode_count: None,
                last_episode_passed: None,
            }
        }

        fn dashboard_scaffold(&self, _workdir: &std::path::Path) -> DashboardInfo {
            DashboardInfo {
                rendered: String::new(),
            }
        }
    }

    fn test_state(auto_plan: bool) -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let mut roko_config = roko_core::config::schema::RokoConfig::default();
        roko_config.prd.auto_plan = auto_plan;
        let state = Arc::new(AppState::new(
            workdir,
            Arc::new(NoOpRuntime),
            roko_config,
            Arc::new(ManualBackend::default()),
        ).expect("AppState::new"));
        (dir, state)
    }

    fn test_state_with_runtime(runtime: Arc<dyn CliRuntime>) -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let mut roko_config = roko_core::config::schema::RokoConfig::default();
        roko_config.prd.auto_plan = true;
        let state = Arc::new(AppState::new(
            workdir,
            runtime,
            roko_config,
            Arc::new(ManualBackend::default()),
        ).expect("AppState::new"));
        (dir, state)
    }

    async fn wait_for_events(state: &Arc<AppState>, expected: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if state.event_bus.replay_from(0).len() >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for background PRD draft job");
    }

    #[test]
    fn publish_dedupe_allows_retry_after_window() {
        clear_recent_publishes();
        let workdir = std::path::Path::new("/tmp/roko-prd-publish-dedupe");
        let base = Instant::now();

        assert!(should_queue_publish(workdir, "alpha", base));
        assert!(!should_queue_publish(
            workdir,
            "alpha",
            base + Duration::from_secs(10)
        ));
        assert!(should_queue_publish(
            workdir,
            "alpha",
            base + Duration::from_secs(61)
        ));
    }

    #[tokio::test]
    async fn prd_publish_subscriber_queues_plan_generation_and_audits() {
        clear_recent_publishes();
        let runtime = Arc::new(RecordingRuntime {
            calls: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            success: true,
            call_count: Arc::new(AtomicUsize::new(0)),
        });
        let notify = Arc::clone(&runtime.as_ref().notify);
        let calls = Arc::clone(&runtime.as_ref().calls);
        let (_dir, state) = test_state_with_runtime(runtime);
        let published_path = state
            .workdir
            .join(".roko")
            .join("prd")
            .join("published")
            .join("alpha.md");
        if let Some(parent) = published_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .expect("create published dir");
        }
        tokio::fs::write(&published_path, "# Alpha\n")
            .await
            .expect("write published prd");

        let bus = EventBus::new(16);
        let _subscriber = spawn_prd_publish_subscriber(Arc::clone(&state), &bus);

        bus.emit(RokoEvent::PrdPublished {
            slug: "alpha".into(),
            path: published_path.clone(),
            published_at: Utc::now(),
            origin: PublishOrigin::Cli,
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), notify.notified())
            .await
            .expect("subscriber should queue plan generation");
        tokio::task::yield_now().await;

        let calls = calls.lock().expect("lock calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, state.workdir);
        assert!(calls[0].1.contains(".roko/prd/published/alpha.md"));

        let ops = state.operations.read().await;
        assert_eq!(ops.len(), 1);
    }

    #[tokio::test]
    async fn prd_publish_subscriber_dedupes_repeated_events() {
        clear_recent_publishes();
        let runtime = Arc::new(RecordingRuntime {
            calls: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            success: true,
            call_count: Arc::new(AtomicUsize::new(0)),
        });
        let notify = Arc::clone(&runtime.as_ref().notify);
        let calls = Arc::clone(&runtime.as_ref().calls);
        let (_dir, state) = test_state_with_runtime(runtime);
        let published_path = state
            .workdir
            .join(".roko")
            .join("prd")
            .join("published")
            .join("beta.md");
        if let Some(parent) = published_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .expect("create published dir");
        }
        tokio::fs::write(&published_path, "# Beta\n")
            .await
            .expect("write published prd");

        let bus = EventBus::new(16);
        let _subscriber = spawn_prd_publish_subscriber(Arc::clone(&state), &bus);

        let event = RokoEvent::PrdPublished {
            slug: "beta".into(),
            path: published_path,
            published_at: Utc::now(),
            origin: PublishOrigin::Http,
        };
        bus.emit(event.clone());
        bus.emit(event);

        tokio::time::timeout(std::time::Duration::from_secs(1), notify.notified())
            .await
            .expect("first publish should queue plan generation");
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let calls = calls.lock().expect("lock calls");
        assert_eq!(calls.len(), 1);

        let ops = state.operations.read().await;
        assert_eq!(ops.len(), 1);
    }

    #[tokio::test]
    async fn promote_prd_queues_plan_generation_when_auto_plan_is_enabled() {
        let (dir, state) = test_state(true);
        let drafts = dir.path().join(".roko").join("prd").join("drafts");
        tokio::fs::create_dir_all(&drafts).await.expect("draft dir");
        tokio::fs::write(
            drafts.join("alpha.md"),
            "---\nstatus: draft\n---\n\n# Alpha\n",
        )
        .await
        .expect("write draft");

        let response = promote_prd(State(Arc::clone(&state)), Path("alpha".into()))
            .await
            .expect("promote");
        let body: Value = response.0;

        assert_eq!(body["slug"], "alpha");
        assert_eq!(body["status"], "published");
        assert_eq!(body["plan_generation"], "queued");
        assert!(body["plan_operation_id"].is_string());
        assert!(
            dir.path()
                .join(".roko")
                .join("prd")
                .join("published")
                .join("alpha.md")
                .is_file()
        );
        assert_eq!(state.operations.read().await.len(), 1);
    }

    #[tokio::test]
    async fn promote_prd_skips_plan_generation_when_auto_plan_is_disabled() {
        let (dir, state) = test_state(false);
        let drafts = dir.path().join(".roko").join("prd").join("drafts");
        tokio::fs::create_dir_all(&drafts).await.expect("draft dir");
        tokio::fs::write(
            drafts.join("beta.md"),
            "---\nstatus: draft\n---\n\n# Beta\n",
        )
        .await
        .expect("write draft");

        let response = promote_prd(State(Arc::clone(&state)), Path("beta".into()))
            .await
            .expect("promote");
        let body: Value = response.0;

        assert_eq!(body["slug"], "beta");
        assert_eq!(body["status"], "published");
        assert!(body.get("plan_generation").is_none());
        assert!(body.get("plan_operation_id").is_none());
        assert!(
            dir.path()
                .join(".roko")
                .join("prd")
                .join("published")
                .join("beta.md")
                .is_file()
        );
        assert!(state.operations.read().await.is_empty());
    }

    #[tokio::test]
    async fn draft_prd_runs_runtime_with_draft_scaffold() {
        let runtime = Arc::new(RecordingRuntime {
            calls: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            success: true,
            call_count: Arc::new(AtomicUsize::new(0)),
        });
        let notify = Arc::clone(&runtime.as_ref().notify);
        let calls = Arc::clone(&runtime.as_ref().calls);
        let (_dir, state) = test_state_with_runtime(runtime);

        let response = draft_prd(State(Arc::clone(&state)), Path("alpha".into()))
            .await
            .expect("draft prd")
            .into_response();

        assert_eq!(response.status(), axum::http::StatusCode::ACCEPTED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body: Value = serde_json::from_slice(&body).expect("parse response body");
        assert!(body["id"].is_string());

        tokio::time::timeout(std::time::Duration::from_secs(1), notify.notified())
            .await
            .expect("runtime should be called");
        wait_for_events(&state, 2).await;

        let draft_path = state
            .workdir
            .join(".roko")
            .join("prd")
            .join("drafts")
            .join("alpha.md");
        assert!(draft_path.is_file());
        let content = tokio::fs::read_to_string(&draft_path)
            .await
            .expect("read draft");
        assert!(content.contains("id: prd-alpha"));
        assert!(content.contains("status: draft"));
        assert!(content.contains("## Overview"));

        let calls = calls.lock().expect("lock calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, state.workdir);
        assert!(calls[0].1.contains(".roko/prd/drafts/alpha.md"));
        assert!(calls[0].1.contains("Search the codebase first"));

        let events = state.event_bus.replay_from(0);
        assert!(matches!(
            events[0].payload,
            ServerEvent::OperationStarted { ref kind, .. } if kind == "prd_draft"
        ));
        assert!(matches!(
            events[1].payload,
            ServerEvent::OperationCompleted { success: true, ref kind, .. } if kind == "prd_draft"
        ));

        let ops = state.operations.read().await;
        let op = ops.values().next().expect("operation stored");
        assert!(op.handle.is_finished());
        assert_eq!(op.kind, "prd_draft:alpha");
    }
}
