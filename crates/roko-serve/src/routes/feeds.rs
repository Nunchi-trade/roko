//! Feed management routes.
//!
//! Descriptor CRUD (static feed registry):
//! - `GET    /api/feeds`                — list feeds (with optional `?kind=` and `?agent_id=` filters)
//! - `POST   /api/feeds`               — register a feed
//! - `GET    /api/feeds/{id}`          — get feed detail
//! - `DELETE /api/feeds/{id}`          — unregister a feed
//!
//! Runtime feeds (live status from the serve layer):
//! - `GET    /api/feeds/runtime`       — list all runtime feeds with status
//! - `GET    /api/feeds/runtime/{id}`  — get detailed runtime status for a feed

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use roko_core::feed::{FeedAccess, FeedInfo, FeedKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/feeds", get(list_feeds).post(create_feed))
        // Catalog must be registered before the wildcard `/feeds/{id}`.
        .route("/feeds/catalog", get(get_feed_catalog))
        // Runtime feed routes must be registered before the wildcard `/feeds/{id}`
        // so that "/feeds/runtime" is not captured as id="runtime".
        .route("/feeds/runtime", get(list_runtime_feeds))
        .route("/feeds/runtime/{id}", get(get_runtime_feed_status))
        .route("/feeds/{id}", get(get_feed).delete(delete_feed))
}

// ── Request / Response types ──────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateFeedRequest {
    name: String,
    kind: FeedKind,
    #[serde(default = "default_access")]
    access: FeedAccess,
    agent_id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    schema: Option<Value>,
}

fn default_access() -> FeedAccess {
    FeedAccess::Public
}

#[derive(Debug, Deserialize)]
struct FeedQuery {
    #[serde(default)]
    kind: Option<FeedKind>,
    #[serde(default)]
    agent_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FeedListResponse {
    feeds: Vec<FeedInfo>,
    total: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateFeedResponse {
    id: String,
    feed: FeedInfo,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeleteFeedResponse {
    id: String,
    deleted: bool,
}

// ── Feed catalog types ───────────────────────────────────────────

use crate::state::{FeedCatalogAgent, FeedCatalogEntry};

#[derive(Debug, Serialize)]
struct FeedCatalogResponse {
    agents: Vec<FeedCatalogAgent>,
    feeds: Vec<FeedCatalogEntry>,
    stats: FeedCatalogStats,
}

#[derive(Debug, Serialize)]
struct FeedCatalogStats {
    total_agents: usize,
    total_feeds: usize,
    messages_per_sec: f64,
}

// ── Handlers ──────────────────────────────────────────────────────

/// `GET /api/feeds` — list feeds with optional kind and agent_id filters.
async fn list_feeds(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FeedQuery>,
) -> Json<FeedListResponse> {
    let reg = state.feeds.read().await;

    let feeds: Vec<FeedInfo> = match (&query.kind, &query.agent_id) {
        (Some(kind), Some(agent_id)) => reg
            .list()
            .iter()
            .filter(|f| f.kind == *kind && f.agent_id == *agent_id)
            .cloned()
            .collect(),
        (Some(kind), None) => reg
            .list_by_kind(kind.clone())
            .into_iter()
            .cloned()
            .collect(),
        (None, Some(agent_id)) => reg.list_by_agent(agent_id).into_iter().cloned().collect(),
        (None, None) => reg.list().to_vec(),
    };

    let total = feeds.len();
    Json(FeedListResponse { feeds, total })
}

/// `POST /api/feeds` — register a new feed.
async fn create_feed(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateFeedRequest>,
) -> Result<(StatusCode, Json<CreateFeedResponse>), ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("feed name must not be empty"));
    }
    if req.agent_id.trim().is_empty() {
        return Err(ApiError::bad_request("agent_id must not be empty"));
    }

    let info = FeedInfo {
        id: String::new(), // assigned by registry
        name: req.name,
        kind: req.kind,
        access: req.access,
        agent_id: req.agent_id,
        description: req.description,
        schema: req.schema,
        created_at: Utc::now(),
    };

    let mut reg = state.feeds.write().await;
    let id = reg.register(info);
    let feed = reg.get(&id).expect("just registered").clone();

    Ok((StatusCode::CREATED, Json(CreateFeedResponse { id, feed })))
}

/// `GET /api/feeds/{id}` — get a single feed by ID.
async fn get_feed(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<FeedInfo>, ApiError> {
    let reg = state.feeds.read().await;
    let info = reg
        .get(&id)
        .ok_or_else(|| ApiError::not_found(format!("feed '{id}' not found")))?;
    Ok(Json(info.clone()))
}

/// `DELETE /api/feeds/{id}` — unregister a feed.
async fn delete_feed(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DeleteFeedResponse>, ApiError> {
    let mut reg = state.feeds.write().await;
    let deleted = reg.unregister(&id);
    if !deleted {
        return Err(ApiError::not_found(format!("feed '{id}' not found")));
    }
    Ok(Json(DeleteFeedResponse { id, deleted }))
}

// ── Feed catalog handler ─────────────────────────────────────────

/// `GET /api/feeds/catalog` — aggregated feed catalog from feed agents.
async fn get_feed_catalog(State(state): State<Arc<AppState>>) -> Json<FeedCatalogResponse> {
    let snapshot = state.feed_agent_catalog.read().await;
    Json(FeedCatalogResponse {
        agents: snapshot.agents.clone(),
        feeds: snapshot.feeds.clone(),
        stats: FeedCatalogStats {
            total_agents: snapshot.agents.len(),
            total_feeds: snapshot.feeds.len(),
            messages_per_sec: snapshot.messages_per_sec,
        },
    })
}

// ── Runtime feed handlers ────────────────────────────────────────

/// `GET /api/feeds/runtime` -- list all runtime feeds with their current status.
async fn list_runtime_feeds(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<roko_core::FeedRuntimeStatus>> {
    Json(state.runtime_feeds.list())
}

/// `GET /api/feeds/runtime/{id}` -- get detailed status for a single runtime feed.
async fn get_runtime_feed_status(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<roko_core::FeedRuntimeStatus>, StatusCode> {
    state
        .runtime_feeds
        .get(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use roko_core::config::schema::RokoConfig;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::runtime::NoOpRuntime;

    fn test_state(workdir: std::path::PathBuf) -> Arc<AppState> {
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        Arc::new(
            AppState::new(
                workdir,
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                deploy_backend,
            )
            .expect("AppState::new"),
        )
    }

    #[tokio::test]
    async fn list_feeds_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/feeds")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: FeedListResponse = serde_json::from_slice(&body).expect("parse");
        assert!(payload.feeds.is_empty());
        assert_eq!(payload.total, 0);
    }

    #[tokio::test]
    async fn create_then_get_feed() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());
        let app = routes().with_state(Arc::clone(&state));

        // Create a feed.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/feeds")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "name": "eth-prices",
                            "kind": "raw",
                            "agent_id": "agent-1",
                            "description": "ETH/USD price feed"
                        }))
                        .unwrap(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let created: CreateFeedResponse = serde_json::from_slice(&body).expect("parse");
        assert_eq!(created.feed.name, "eth-prices");
        let feed_id = created.id;

        // Get by ID.
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&format!("/feeds/{feed_id}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let fetched: FeedInfo = serde_json::from_slice(&body).expect("parse");
        assert_eq!(fetched.name, "eth-prices");
        assert_eq!(fetched.agent_id, "agent-1");
    }

    #[tokio::test]
    async fn list_feeds_with_kind_filter() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());
        let app = routes().with_state(Arc::clone(&state));

        // Create two feeds of different kinds.
        for (name, kind) in [("raw-feed", "raw"), ("derived-feed", "derived")] {
            let _ = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/feeds")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::to_string(&serde_json::json!({
                                "name": name,
                                "kind": kind,
                                "agent_id": "agent-x"
                            }))
                            .unwrap(),
                        ))
                        .expect("request"),
                )
                .await
                .expect("response");
        }

        // Filter by kind=raw.
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/feeds?kind=raw")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: FeedListResponse = serde_json::from_slice(&body).expect("parse");
        assert_eq!(payload.total, 1);
        assert_eq!(payload.feeds[0].name, "raw-feed");
    }

    #[tokio::test]
    async fn list_feeds_with_agent_filter() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());
        let app = routes().with_state(Arc::clone(&state));

        // Create feeds from different agents.
        for (name, agent) in [("f1", "agent-a"), ("f2", "agent-b"), ("f3", "agent-a")] {
            let _ = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/feeds")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::to_string(&serde_json::json!({
                                "name": name,
                                "kind": "raw",
                                "agent_id": agent
                            }))
                            .unwrap(),
                        ))
                        .expect("request"),
                )
                .await
                .expect("response");
        }

        // Filter by agent_id=agent-a.
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/feeds?agent_id=agent-a")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: FeedListResponse = serde_json::from_slice(&body).expect("parse");
        assert_eq!(payload.total, 2);
    }

    #[tokio::test]
    async fn delete_feed_success() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());
        let app = routes().with_state(Arc::clone(&state));

        // Create first.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/feeds")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"temp","kind":"meta","agent_id":"a1"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let created: CreateFeedResponse = serde_json::from_slice(&body).expect("parse");

        // Delete.
        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&format!("/feeds/{}", created.id))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: DeleteFeedResponse = serde_json::from_slice(&body).expect("parse");
        assert!(payload.deleted);
    }

    #[tokio::test]
    async fn delete_feed_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/feeds/feed-999")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_feed_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/feeds/feed-999")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // ── Runtime feed tests ───────────────────────────────────────

    #[tokio::test]
    async fn list_runtime_feeds_returns_two_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/feeds/runtime")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let feeds: Vec<roko_core::FeedRuntimeStatus> =
            serde_json::from_slice(&body).expect("parse");
        assert_eq!(feeds.len(), 3);
        assert_eq!(feeds[0].id, "file-watch-roko-dir");
        assert_eq!(feeds[0].topic, "fs.changed");
        assert_eq!(feeds[1].id, "provider-health-feed");
        assert_eq!(feeds[1].topic, "provider.health");
        assert_eq!(feeds[2].id, "isfr-keeper");
        assert_eq!(feeds[2].topic, "isfr.rates");
    }

    #[tokio::test]
    async fn get_runtime_feed_status_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/feeds/runtime/file-watch-roko-dir")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let status: roko_core::FeedRuntimeStatus = serde_json::from_slice(&body).expect("parse");
        assert_eq!(status.id, "file-watch-roko-dir");
        assert_eq!(status.kind, "Raw");
        // .roko/ dir was created above so it should be connected.
        assert!(status.connected);
    }

    #[tokio::test]
    async fn get_runtime_feed_status_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".roko")).expect("create .roko");
        let state = test_state(dir.path().to_path_buf());

        let response = routes()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/feeds/runtime/nonexistent")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
