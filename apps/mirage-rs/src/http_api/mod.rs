//! HTTP REST API for dashboard consumption (Issue #1).
//!
//! Provides JSON endpoints that expose dashboard-friendly chain data.
//!
//! These endpoints cover the pheromone field, knowledge graph, and agent
//! topology surfaces for kauri. They complement the existing JSON-RPC surface
//! with REST semantics optimized for UI consumption:
//!
//! - Pagination, filtering, and sorting on all list endpoints
//! - Decay projections and time-bucketed heatmap data for animated visualizations
//! - Force-directed graph data (nodes + edges) for d3.js/force-graph layouts
//! - Agent interaction topology derived from knowledge store confirmations
//! - WebSocket streaming of real-time pheromone and insight events (roko feature)
//! - Input validation with clamped query parameters
//! - Paginated response envelopes with total/offset/limit/has_more metadata
//! - Cache-Control headers on read-only endpoints
//! - Request tracing via tower-http TraceLayer and x-request-id propagation
//!
//! # Endpoint summary
//!
//! | Method | Path                    | Description                                |
//! |--------|-------------------------|--------------------------------------------|
//! | GET    | `/api/health`           | Server health with uptime and chain status |
//! | GET    | `/api/pheromones`       | List active pheromones (filter/sort/page)  |
//! | GET    | `/api/pheromones/summary` | Aggregate stats per kind                 |
//! | POST   | `/api/pheromones/query` | Top-K by HDC similarity × intensity        |
//! | GET    | `/api/pheromones/heatmap` | Time-bucketed deposit activity           |
//! | GET    | `/api/knowledge/entries`| List insight entries (filter/sort/page)    |
//! | GET    | `/api/knowledge/edges`  | Dependency + HDC similarity edges          |
//! | GET    | `/api/knowledge/search` | Semantic search over knowledge store       |
//! | GET    | `/api/knowledge/kinds`  | Enumerate knowledge + pheromone kinds      |
//! | GET    | `/api/agents/topology`  | Agent interaction graph (nodes + edges)    |
//! | GET    | `/api/stats`            | Combined dashboard statistics              |
//! | WS     | `/api/ws`               | Live event stream (roko feature)           |

#![allow(missing_docs)]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderName, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use parking_lot::RwLock;
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::chain_rpc::ChainContext;

// ---------------------------------------------------------------------------
// Validation constants
// ---------------------------------------------------------------------------

pub const MAX_LIMIT: usize = 1000;
pub const MAX_K: usize = 100;
pub const MIN_BUCKET_WIDTH: u64 = 60;
pub const MAX_HEATMAP_BUCKETS: usize = 500;

// ---------------------------------------------------------------------------
// Paginated response envelope
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub has_more: bool,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: usize, offset: usize, limit: usize) -> Self {
        Self {
            has_more: offset + items.len() < total,
            items,
            total,
            offset,
            limit,
        }
    }
}

// ---------------------------------------------------------------------------
// Cache-Control helper
// ---------------------------------------------------------------------------

pub fn with_cache_control<T: Serialize>(
    body: T,
    max_age: u32,
) -> ([(HeaderName, HeaderValue); 1], Json<T>) {
    let cache_control = HeaderValue::from_str(&format!("public, max-age={max_age}"))
        .unwrap_or_else(|_| HeaderValue::from_static("public, max-age=0"));
    ([(header::CACHE_CONTROL, cache_control)], Json(body))
}

// ---------------------------------------------------------------------------
// Request-id middleware
// ---------------------------------------------------------------------------

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

async fn request_id_middleware(mut request: axum::extract::Request, next: Next) -> Response {
    let id = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let value = HeaderValue::from_str(&format!("req-{id}"))
        .unwrap_or_else(|_| HeaderValue::from_static("req-invalid"));
    request
        .headers_mut()
        .insert(HeaderName::from_static("x-request-id"), value.clone());
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(HeaderName::from_static("x-request-id"), value);
    response
}

// ---------------------------------------------------------------------------
// WebSocket connection registry
// ---------------------------------------------------------------------------

/// Lock-free WebSocket connection registry using atomics.
///
/// Tracks connected, peak, and total connection counts without requiring locks
/// on the hot path. Enforces a configurable maximum connection limit.
pub struct WsRegistry {
    /// Currently connected WebSocket clients.
    connected: AtomicU64,
    /// High-water mark of concurrent connections.
    peak: AtomicU64,
    /// Lifetime total connections accepted.
    total: AtomicU64,
    /// Hard cap on simultaneous connections.
    max_connections: u64,
}

impl WsRegistry {
    /// Creates a new registry with the given maximum connection limit.
    pub fn new(max_connections: u64) -> Self {
        Self {
            connected: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            total: AtomicU64::new(0),
            max_connections,
        }
    }

    /// Attempts to register a new connection.
    ///
    /// Returns `true` if the connection was accepted (under the limit),
    /// `false` if the limit has been reached (caller should return 503).
    pub fn try_connect(&self) -> bool {
        let prev = self.connected.fetch_add(1, Ordering::Relaxed);
        if prev >= self.max_connections {
            // Over limit — roll back.
            self.connected.fetch_sub(1, Ordering::Relaxed);
            return false;
        }
        self.total.fetch_add(1, Ordering::Relaxed);
        self.peak.fetch_max(prev + 1, Ordering::Relaxed);
        true
    }

    /// Records a connection drop.
    pub fn disconnect(&self) {
        self.connected.fetch_sub(1, Ordering::Relaxed);
    }

    /// Returns a snapshot of `(connected, peak, total)`.
    pub fn stats(&self) -> (u64, u64, u64) {
        (
            self.connected.load(Ordering::Relaxed),
            self.peak.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
}

// ---------------------------------------------------------------------------
// HDC projection cache
// ---------------------------------------------------------------------------

/// Thread-safe bounded LRU cache for HDC projections.
///
/// `project_tokens()` is deterministic but CPU-intensive. This cache avoids
/// recomputing the same projection for repeated queries.
#[derive(Clone)]
pub struct ProjectionCache {
    inner: Arc<std::sync::Mutex<lru::LruCache<String, roko_primitives::HdcVector>>>,
}

impl ProjectionCache {
    pub fn new(capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity).unwrap_or_else(|| {
            NonZeroUsize::new(1024).expect("default cache capacity is non-zero")
        });
        Self {
            inner: Arc::new(std::sync::Mutex::new(lru::LruCache::new(capacity))),
        }
    }

    /// Returns a cached projection or computes and inserts it.
    pub fn get_or_insert(&self, text: &str) -> roko_primitives::HdcVector {
        let mut cache = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(v) = cache.get(text) {
            return *v; // HdcVector is Copy
        }
        let v = crate::chain::projection::project_tokens(text);
        cache.put(text.to_owned(), v);
        v
    }
}

pub mod agent;
pub mod isfr;
pub mod knowledge;
pub mod pheromone;
pub mod prediction;
pub mod skills;
pub mod task;
pub mod topology;
#[cfg(feature = "roko")]
pub mod ws;

/// Callback that returns the current block number from the fork state.
///
/// In production this reads `MirageState.fork.local_block_number` behind
/// the server's `Arc<RwLock>`. In tests, callers can supply a simple closure.
pub type BlockNumberFn = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Shared state for HTTP API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Chain context holding the knowledge store and pheromone field.
    pub chain: Arc<RwLock<ChainContext>>,
    /// Returns the current block number. Used by heartbeat/liveness endpoints.
    pub current_block: BlockNumberFn,
    /// HDC projection cache for query endpoints.
    pub projection_cache: ProjectionCache,
    /// Server start time for uptime computation.
    pub started_at: Instant,
    /// WebSocket connection registry for tracking and limiting connections.
    pub ws_registry: Arc<WsRegistry>,
    /// Subscription manager for WebSocket streaming (roko feature only).
    #[cfg(feature = "roko")]
    pub subs: Option<crate::chain_rpc::SubscriptionManager>,
}

impl ApiState {
    /// Creates an `ApiState` with block number fixed at 0 (useful for tests).
    pub fn new(chain: Arc<RwLock<ChainContext>>) -> Self {
        Self {
            chain,
            current_block: Arc::new(|| 0),
            projection_cache: ProjectionCache::new(128),
            started_at: Instant::now(),
            ws_registry: Arc::new(WsRegistry::new(1000)),
            #[cfg(feature = "roko")]
            subs: None,
        }
    }
}

/// Builds the `/api` router with all dashboard endpoints.
#[must_use]
pub fn build_router(state: ApiState) -> Router {
    let router = Router::new()
        // Health
        .route("/health", get(health))
        // Pheromone field
        .route(
            "/pheromones",
            get(pheromone::list_pheromones).post(pheromone::deposit_pheromone),
        )
        .route("/pheromones/summary", get(pheromone::pheromone_summary))
        .route("/pheromones/query", post(pheromone::query_pheromones))
        .route("/pheromones/heatmap", get(pheromone::pheromone_heatmap))
        .route(
            "/pheromones/{id}/projection",
            get(pheromone::pheromone_projection),
        )
        // Knowledge graph
        .route(
            "/knowledge/entries",
            get(knowledge::list_entries).post(knowledge::post_insight),
        )
        .route(
            "/knowledge/entries/{id}/confirm",
            post(knowledge::confirm_entry),
        )
        .route(
            "/knowledge/entries/{id}/challenge",
            post(knowledge::challenge_entry),
        )
        .route("/knowledge/decay", post(knowledge::trigger_decay))
        .route("/knowledge/edges", get(knowledge::list_edges))
        .route("/knowledge/search", get(knowledge::search_knowledge))
        .route("/knowledge/kinds", get(knowledge::list_kinds))
        // Agent topology
        .route("/agents/topology", get(topology::agent_topology))
        // Agent registry
        .route(
            "/agents",
            get(agent::list_agents).post(agent::register_agent),
        )
        .route(
            "/agents/{id}/trace",
            get(agent::get_agent_trace).post(agent::post_agent_trace),
        )
        .route(
            "/agents/{id}/heartbeat",
            get(agent::get_agent_heartbeat).post(agent::agent_heartbeat),
        )
        .route("/agents/{id}/stats", get(agent::get_agent_stats))
        .route(
            "/agents/{id}/skills",
            get(skills::get_agent_skills).put(skills::update_agent_skills),
        )
        .route(
            "/agents/{id}/skills/{skill}",
            axum::routing::put(skills::update_single_skill),
        )
        // Task tracking
        .route("/tasks", get(task::list_tasks).post(task::create_task))
        .route("/tasks/stats", get(task::task_stats))
        .route("/tasks/{id}", get(task::get_task))
        .route("/tasks/{id}/assign", post(task::assign_task))
        .route("/tasks/{id}/start", post(task::start_task))
        .route("/tasks/{id}/complete", post(task::complete_task))
        .route("/tasks/{id}/artifacts", get(task::get_task_artifacts))
        .route("/tasks/{id}/improve", post(task::improve_task))
        .route("/tasks/{id}/fail", post(task::fail_task))
        .route("/tasks/{id}/cancel", post(task::cancel_task))
        // External service proxies
        .route("/isfr/current", get(isfr::isfr_current))
        .route("/isfr/history", get(isfr::isfr_history))
        // Predictions
        .route(
            "/predictions/sessions",
            get(prediction::list_sessions).post(prediction::create_session),
        )
        .route("/predictions/sessions/{id}", get(prediction::get_session))
        .route(
            "/predictions/sessions/{id}/resolve",
            post(prediction::resolve_session),
        )
        .route(
            "/predictions/claims",
            get(prediction::list_claims).post(prediction::submit_claim),
        )
        .route(
            "/predictions/calibration/{agent_id}",
            get(prediction::get_calibration),
        )
        // Combined stats
        .route("/stats", get(combined_stats))
        // Contract deployment registry
        .route("/deployment", get(get_deployment).post(post_deployment));

    #[cfg(feature = "roko")]
    let router = router.route("/ws", get(ws::ws_handler));

    router
        .layer(middleware::from_fn(request_id_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(tower::limit::ConcurrencyLimitLayer::new(200))
        .with_state(state)
}

/// Current timestamp in seconds since UNIX epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// JSON error response returned by API endpoints.
#[derive(Serialize)]
pub struct ApiError {
    /// Human-readable error message.
    pub error: String,
    /// HTTP status code.
    pub code: u16,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self)).into_response()
    }
}

fn default_limit() -> usize {
    100
}

/// `GET /api/health` — server health with uptime and chain status.
async fn health(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let uptime = state.started_at.elapsed().as_secs();
    let chain = state.chain.read();

    let insight_count = chain.knowledge.len();
    let pheromone_count = chain.pheromones.len();
    let agent_count = chain.agent_registry.list_agents().len();
    let task_count = chain.task_store.len();
    let prediction_session_count = chain.prediction_store.session_count();
    let prediction_claim_count = chain.prediction_store.claim_count();

    let (ws_connected, ws_peak, ws_total) = state.ws_registry.stats();

    Json(serde_json::json!({
        "status": "ok",
        "uptime_secs": uptime,
        "chain": {
            "toggles": {
                "hdc": chain.toggles.hdc,
                "knowledge": chain.toggles.knowledge,
                "stigmergy": chain.toggles.stigmergy,
            },
            "counts": {
                "insights": insight_count,
                "pheromones": pheromone_count,
                "agents": agent_count,
                "tasks": task_count,
                "prediction_sessions": prediction_session_count,
                "prediction_claims": prediction_claim_count,
            }
        },
        "websockets": {
            "connected": ws_connected,
            "peak": ws_peak,
            "total": ws_total,
        }
    }))
}

// ---------------------------------------------------------------------------
// Contract deployment registry
// ---------------------------------------------------------------------------

/// `GET /api/deployment` — return all registered contract addresses.
///
/// The dashboard calls this at startup to discover deployed contract addresses
/// instead of hardcoding them. Addresses are seeded with ERC-8004 bootstrap
/// contracts and updated by `POST /api/deployment` after Foundry deploys.
async fn get_deployment(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let chain = state.chain.read();
    let contracts = &chain.contract_registry;
    Json(serde_json::json!({
        "chain_id": 1,
        "contracts": contracts,
    }))
}

/// `POST /api/deployment` — register or update contract addresses.
///
/// Accepts `{ "contracts": { "AgentRegistry": "0x...", ... } }`.
/// Merges into the existing registry (does not replace it).
async fn post_deployment(
    State(state): State<ApiState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let contracts = body
        .get("contracts")
        .and_then(|v| v.as_object())
        .ok_or_else(|| ApiError {
            error: "expected { \"contracts\": { \"Name\": \"0x...\" } }".into(),
            code: 400,
        })?;

    let mut chain = state.chain.write();
    let mut updated = 0usize;
    for (name, addr) in contracts {
        if let Some(addr_str) = addr.as_str() {
            chain
                .contract_registry
                .insert(name.clone(), addr_str.to_string());
            updated += 1;
        }
    }

    let total = chain.contract_registry.len();
    Ok(Json(serde_json::json!({
        "updated": updated,
        "total": total,
    })))
}

/// Combined dashboard statistics.
async fn combined_stats(State(state): State<ApiState>) -> impl IntoResponse {
    let now = now_secs();
    let chain = state.chain.read();

    let insight_count = chain.knowledge.len();
    let pheromone_count = chain.pheromones.len();

    let mut threat_count = 0usize;
    let mut opportunity_count = 0usize;
    let mut wisdom_count = 0usize;
    let mut total_intensity = 0.0f64;
    for p in chain.pheromones.iter() {
        match p.kind {
            crate::chain::PheromoneKind::Threat => threat_count += 1,
            crate::chain::PheromoneKind::Opportunity => opportunity_count += 1,
            crate::chain::PheromoneKind::Wisdom => wisdom_count += 1,
        }
        total_intensity += p.current_intensity(now) as f64;
    }

    // Knowledge state breakdown
    let mut active = 0usize;
    let mut confirmed = 0usize;
    let mut challenged = 0usize;
    let mut decaying = 0usize;
    for entry in chain.knowledge.entries() {
        match entry.state {
            crate::chain::KnowledgeState::Active => active += 1,
            crate::chain::KnowledgeState::Confirmed => confirmed += 1,
            crate::chain::KnowledgeState::Challenged => challenged += 1,
            crate::chain::KnowledgeState::Decaying => decaying += 1,
            _ => {}
        }
    }

    let task_stats = chain.task_store.stats();
    let prediction_session_count = chain.prediction_store.session_count();
    let prediction_claim_count = chain.prediction_store.claim_count();

    with_cache_control(
        serde_json::json!({
            "insights": {
                "total": insight_count,
                "active": active,
                "confirmed": confirmed,
                "challenged": challenged,
                "decaying": decaying,
            },
            "pheromones": {
                "total": pheromone_count,
                "threat": threat_count,
                "opportunity": opportunity_count,
                "wisdom": wisdom_count,
                "total_intensity": total_intensity,
            },
            "tasks": {
                "open": task_stats.open,
                "assigned": task_stats.assigned,
                "in_progress": task_stats.in_progress,
                "completed": task_stats.completed,
                "failed": task_stats.failed,
                "cancelled": task_stats.cancelled,
                "total_stake_wei": task_stats.total_stake_wei,
                "total_reward_wei": task_stats.total_reward_wei,
            },
            "predictions": {
                "sessions": prediction_session_count,
                "claims": prediction_claim_count,
            },
            "toggles": {
                "hdc": chain.toggles.hdc,
                "knowledge": chain.toggles.knowledge,
                "stigmergy": chain.toggles.stigmergy,
            },
            "timestamp": now,
        }),
        5,
    )
}
