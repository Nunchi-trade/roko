//! PRD lifecycle endpoints — list, read, idea capture, draft, promote, plan.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::serve::error::ApiError;
use crate::serve::events::ServerEvent;
use crate::serve::state::{AppState, OperationHandle, OperationStatus};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/prds", get(list_prds))
        .route("/prds/ideas", post(post_idea))
        .route("/prds/status", get(prds_coverage))
        .route("/prds/{slug}", get(get_prd))
        .route("/prds/{slug}/draft", post(draft_prd))
        .route("/prds/{slug}/promote", post(promote_prd))
        .route("/prds/{slug}/plan", post(plan_from_prd))
}

/// `GET /api/prds` — list PRD slugs from drafts/ and published/.
async fn list_prds(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let prd_dir = state.workdir.join(".roko").join("prd");
    let mut entries = Vec::new();

    for (status, subdir) in [("draft", "drafts"), ("published", "published")] {
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
            entries.push(json!({ "slug": slug, "status": status }));
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

#[derive(Deserialize)]
struct IdeaRequest {
    text: String,
}

/// `POST /api/prds/ideas` — append a line to `.roko/prd/ideas.md`.
async fn post_idea(
    State(state): State<Arc<AppState>>,
    Json(body): Json<IdeaRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let prd_dir = state.workdir.join(".roko").join("prd");
    tokio::fs::create_dir_all(&prd_dir)
        .await
        .map_err(|e| ApiError::internal(format!("create prd dir: {e}")))?;

    let ideas_path = prd_dir.join("ideas.md");
    let line = format!("- {}\n", body.text);

    // Create the file if it doesn't exist, otherwise append.
    if ideas_path.exists() {
        tokio::fs::write(
            &ideas_path,
            format!(
                "{}{}",
                tokio::fs::read_to_string(&ideas_path)
                    .await
                    .unwrap_or_default(),
                line
            ),
        )
        .await
        .map_err(|e| ApiError::internal(format!("append idea: {e}")))?;
    } else {
        let header = "# Ideas\n\nQuick captures. Run `roko prd idea \"text\"` to append.\n\n";
        tokio::fs::write(&ideas_path, format!("{header}{line}"))
            .await
            .map_err(|e| ApiError::internal(format!("write ideas: {e}")))?;
    }

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "status": "appended" })),
    ))
}

/// `POST /api/prds/:slug/draft` — spawn a background draft operation.
async fn draft_prd(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.sender();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        async move {
            // TODO: Wire agent-driven PRD drafting.
            bus.emit(ServerEvent::OperationCompleted {
                op_id,
                kind: "prd_draft".into(),
                success: true,
            });
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

    Ok(Json(json!({ "slug": slug, "status": "published" })))
}

/// `POST /api/prds/:slug/plan` — spawn background plan generation from PRD.
async fn plan_from_prd(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Verify the PRD exists.
    let _content = read_prd_file(&state.workdir, &slug).await?;

    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.sender();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        async move {
            // TODO: Wire `prd plan <slug>` agent generation.
            bus.emit(ServerEvent::OperationCompleted {
                op_id,
                kind: "prd_plan".into(),
                success: true,
            });
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: format!("prd_plan:{slug}"),
        status: OperationStatus::Running,
        handle,
    };

    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

/// `GET /api/prds/status` — coverage report (draft/published/plan counts).
async fn prds_coverage(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
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

// ── helpers ──────────────────────────────────────────────────────────

/// Read a PRD file, checking published/ first, then drafts/.
async fn read_prd_file(
    workdir: &std::path::Path,
    slug: &str,
) -> Result<(String, String), ApiError> {
    let prd_dir = workdir.join(".roko").join("prd");

    let published = prd_dir.join("published").join(format!("{slug}.md"));
    if published.is_file() {
        let content = tokio::fs::read_to_string(&published)
            .await
            .map_err(|e| ApiError::internal(format!("read prd: {e}")))?;
        return Ok(("published".into(), content));
    }

    let draft = prd_dir.join("drafts").join(format!("{slug}.md"));
    if draft.is_file() {
        let content = tokio::fs::read_to_string(&draft)
            .await
            .map_err(|e| ApiError::internal(format!("read prd: {e}")))?;
        return Ok(("draft".into(), content));
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
async fn count_entries_with(
    dir: &std::path::Path,
    pred: impl Fn(&str) -> bool,
) -> usize {
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
