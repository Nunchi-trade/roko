//! Pheromone field HTTP endpoints.

use std::collections::{BTreeMap, HashMap};

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use super::{
    ApiError, ApiState, MAX_K, MAX_LIMIT, MIN_BUCKET_WIDTH, PaginatedResponse, now_secs,
    with_cache_control,
};
use crate::chain::{PheromoneKind, pheromone::PheromoneId};

// ---------------------------------------------------------------------------
// GET /api/pheromones
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PheromoneFilter {
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "super::default_limit")]
    pub limit: usize,
    /// Filter by kind: "threat", "opportunity", "wisdom".
    pub kind: Option<String>,
    /// Minimum current intensity to include.
    pub min_intensity: Option<f32>,
    /// Sort field: "intensity" (default), "deposited_at", "confirmations".
    pub sort: Option<String>,
    /// Sort order: "desc" (default) or "asc".
    pub order: Option<String>,
}

/// Decay projection at future timestamps for animated UI visualizations.
#[derive(Serialize)]
pub struct DecayProjection {
    pub in_1h: f32,
    pub in_4h: f32,
    pub in_24h: f32,
}

#[derive(Serialize)]
pub struct PheromoneItem {
    pub id: u64,
    pub kind: PheromoneKind,
    pub intensity: f32,
    pub base_intensity: f32,
    pub deposited_at: u64,
    pub confirmations: u32,
    pub half_life_seconds: u64,
    pub effective_half_life_seconds: u64,
    pub bucket: u8,
    /// Projected future intensity for UI animations.
    pub decay_projection: DecayProjection,
}

pub async fn list_pheromones(
    State(state): State<ApiState>,
    Query(filter): Query<PheromoneFilter>,
) -> impl IntoResponse {
    let now = now_secs();
    let chain = state.chain.read();

    let limit = filter.limit.min(MAX_LIMIT);
    let kind_filter = filter.kind.as_deref().and_then(parse_pheromone_kind);
    let min_intensity = filter.min_intensity.unwrap_or(0.0);

    let mut items: Vec<PheromoneItem> = chain
        .pheromones
        .iter()
        .filter(|p| kind_filter.map_or(true, |k| p.kind == k))
        .filter_map(|p| {
            let intensity = p.current_intensity(now);
            if intensity < min_intensity {
                return None;
            }
            Some(PheromoneItem {
                id: p.id.0,
                kind: p.kind,
                intensity,
                base_intensity: p.base_intensity,
                deposited_at: p.deposited_at,
                confirmations: p.confirmations,
                half_life_seconds: p.half_life_seconds,
                effective_half_life_seconds: p.effective_half_life_seconds(),
                bucket: p.bucket,
                decay_projection: DecayProjection {
                    in_1h: p.current_intensity(now + 3600),
                    in_4h: p.current_intensity(now + 4 * 3600),
                    in_24h: p.current_intensity(now + 24 * 3600),
                },
            })
        })
        .collect();

    // Sort
    let desc = filter.order.as_deref() != Some("asc");
    match filter.sort.as_deref() {
        Some("deposited_at") => items.sort_by(|a, b| {
            if desc {
                b.deposited_at.cmp(&a.deposited_at)
            } else {
                a.deposited_at.cmp(&b.deposited_at)
            }
        }),
        Some("confirmations") => items.sort_by(|a, b| {
            if desc {
                b.confirmations.cmp(&a.confirmations)
            } else {
                a.confirmations.cmp(&b.confirmations)
            }
        }),
        _ => items.sort_by(|a, b| {
            if desc {
                b.intensity.total_cmp(&a.intensity)
            } else {
                a.intensity.total_cmp(&b.intensity)
            }
        }),
    }

    let total = items.len();
    let offset = filter.offset;
    let items: Vec<_> = items.into_iter().skip(offset).take(limit).collect();

    with_cache_control(PaginatedResponse::new(items, total, offset, limit), 2)
}

// ---------------------------------------------------------------------------
// GET /api/pheromones/summary
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct KindSummary {
    pub count: usize,
    pub total_intensity: f64,
    pub avg_intensity: f64,
    pub min_intensity: f64,
    pub max_intensity: f64,
}

#[derive(Serialize)]
pub struct PheromoneSummaryResponse {
    pub by_kind: HashMap<String, KindSummary>,
    pub total_count: usize,
    pub total_intensity: f64,
    pub timestamp: u64,
}

pub async fn pheromone_summary(State(state): State<ApiState>) -> impl IntoResponse {
    let now = now_secs();
    let chain = state.chain.read();

    // (count, sum, min, max)
    let mut by_kind: HashMap<&str, (usize, f64, f64, f64)> = HashMap::new();
    let mut total_count = 0usize;
    let mut total_intensity = 0.0f64;

    for p in chain.pheromones.iter() {
        let intensity = p.current_intensity(now) as f64;
        total_count += 1;
        total_intensity += intensity;

        let kind_str = pheromone_kind_str(p.kind);
        let entry = by_kind
            .entry(kind_str)
            .or_insert((0, 0.0, f64::MAX, f64::MIN));
        entry.0 += 1;
        entry.1 += intensity;
        entry.2 = entry.2.min(intensity);
        entry.3 = entry.3.max(intensity);
    }

    let by_kind = by_kind
        .into_iter()
        .map(|(k, (count, total, min, max))| {
            let avg = if count > 0 { total / count as f64 } else { 0.0 };
            (
                k.to_owned(),
                KindSummary {
                    count,
                    total_intensity: total,
                    avg_intensity: avg,
                    min_intensity: if count > 0 { min } else { 0.0 },
                    max_intensity: if count > 0 { max } else { 0.0 },
                },
            )
        })
        .collect();

    with_cache_control(
        PheromoneSummaryResponse {
            by_kind,
            total_count,
            total_intensity,
            timestamp: now,
        },
        5,
    )
}

// ---------------------------------------------------------------------------
// POST /api/pheromones/query
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PheromoneQueryRequest {
    /// Natural-language query text (projected to HDC internally).
    pub query: Option<String>,
    /// Number of results to return (default 10).
    #[serde(default = "default_k")]
    pub k: usize,
}

fn default_k() -> usize {
    10
}

#[derive(Serialize)]
pub struct PheromoneQueryHit {
    pub id: u64,
    pub kind: PheromoneKind,
    pub similarity: f32,
    pub intensity: f32,
    pub score: f32,
    pub deposited_at: u64,
    pub confirmations: u32,
}

#[derive(Serialize)]
pub struct PheromoneQueryResponse {
    pub results: Vec<PheromoneQueryHit>,
    pub timestamp: u64,
}

pub async fn query_pheromones(
    State(state): State<ApiState>,
    Json(req): Json<PheromoneQueryRequest>,
) -> Result<Json<PheromoneQueryResponse>, ApiError> {
    let query_text = req.query.ok_or(ApiError {
        error: "missing field: query".into(),
        code: 400,
    })?;
    let now = now_secs();
    let vector = state.projection_cache.get_or_insert(&query_text);
    let chain = state.chain.read();
    let k = req.k.min(MAX_K);
    let hits = chain.pheromones.query_top_k(&vector, k, now);

    let results = hits
        .into_iter()
        .map(|h| {
            let p = chain.pheromones.get(PheromoneId(h.id.0));
            PheromoneQueryHit {
                id: h.id.0,
                kind: h.kind,
                similarity: h.similarity,
                intensity: h.intensity,
                score: h.score,
                deposited_at: p.map_or(0, |p| p.deposited_at),
                confirmations: p.map_or(0, |p| p.confirmations),
            }
        })
        .collect();

    Ok(Json(PheromoneQueryResponse {
        results,
        timestamp: now,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/pheromones/heatmap
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HeatmapParams {
    /// Bucket width in seconds (default 3600 = 1 hour, minimum 60).
    #[serde(default = "default_bucket_seconds")]
    pub bucket_seconds: u64,
    /// Only include pheromones deposited after this timestamp (default: 24h ago).
    pub since: Option<u64>,
}

fn default_bucket_seconds() -> u64 {
    3600
}

#[derive(Serialize)]
pub struct HeatmapBucket {
    pub timestamp: u64,
    pub threat: usize,
    pub opportunity: usize,
    pub wisdom: usize,
    pub total_intensity: f64,
}

#[derive(Serialize)]
pub struct HeatmapResponse {
    pub buckets: Vec<HeatmapBucket>,
    pub bucket_seconds: u64,
    pub timestamp: u64,
}

pub async fn pheromone_heatmap(
    State(state): State<ApiState>,
    Query(params): Query<HeatmapParams>,
) -> Json<HeatmapResponse> {
    let now = now_secs();
    let since = params.since.unwrap_or(now.saturating_sub(24 * 3600));
    let bucket_secs = params.bucket_seconds.max(MIN_BUCKET_WIDTH);
    let chain = state.chain.read();

    let mut bucket_map: BTreeMap<u64, (usize, usize, usize, f64)> = BTreeMap::new();
    for p in chain.pheromones.iter() {
        if p.deposited_at < since {
            continue;
        }
        let bucket_ts = (p.deposited_at / bucket_secs) * bucket_secs;
        let intensity = p.current_intensity(now) as f64;
        let entry = bucket_map.entry(bucket_ts).or_default();
        match p.kind {
            PheromoneKind::Threat => entry.0 += 1,
            PheromoneKind::Opportunity => entry.1 += 1,
            PheromoneKind::Wisdom => entry.2 += 1,
        }
        entry.3 += intensity;
    }

    let buckets = bucket_map
        .into_iter()
        .map(|(ts, (t, o, w, i))| HeatmapBucket {
            timestamp: ts,
            threat: t,
            opportunity: o,
            wisdom: w,
            total_intensity: i,
        })
        .collect();

    Json(HeatmapResponse {
        buckets,
        bucket_seconds: bucket_secs,
        timestamp: now,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_pheromone_kind(s: &str) -> Option<PheromoneKind> {
    match s {
        "threat" => Some(PheromoneKind::Threat),
        "opportunity" => Some(PheromoneKind::Opportunity),
        "wisdom" => Some(PheromoneKind::Wisdom),
        _ => None,
    }
}

fn pheromone_kind_str(kind: PheromoneKind) -> &'static str {
    match kind {
        PheromoneKind::Threat => "threat",
        PheromoneKind::Opportunity => "opportunity",
        PheromoneKind::Wisdom => "wisdom",
    }
}

// ---------------------------------------------------------------------------
// POST /api/pheromones — deposit a pheromone
// ---------------------------------------------------------------------------

fn default_half_life() -> Option<u64> {
    None
}

#[derive(Debug, Deserialize)]
pub struct DepositRequest {
    /// Pheromone kind: "threat", "opportunity", or "wisdom".
    pub kind: String,
    /// Text content to project into HDC vector.
    pub content: String,
    /// Initial intensity.
    pub intensity: f32,
    /// Optional custom half-life in seconds (defaults to kind default).
    #[serde(default = "default_half_life")]
    pub half_life_secs: Option<u64>,
}

/// `POST /api/pheromones` — deposit a new pheromone.
pub async fn deposit_pheromone(
    State(state): State<ApiState>,
    Json(req): Json<DepositRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let kind = parse_pheromone_kind(&req.kind).ok_or(ApiError {
        error: format!("unknown pheromone kind: {}", req.kind),
        code: 400,
    })?;
    if req.content.is_empty() {
        return Err(ApiError {
            error: "content must not be empty".into(),
            code: 400,
        });
    }
    let vector = state.projection_cache.get_or_insert(&req.content);
    let now = now_secs();

    let mut chain = state.chain.write();
    if !chain.toggles.stigmergy {
        return Err(ApiError {
            error: "stigmergy subsystem is disabled".into(),
            code: 503,
        });
    }

    #[cfg(feature = "roko")]
    let broadcast_vector = vector.clone();

    let id = if let Some(tau) = req.half_life_secs {
        chain
            .pheromones
            .deposit_with_half_life(kind, vector, req.intensity, now, tau)
    } else {
        chain.pheromones.deposit(kind, vector, req.intensity, now)
    };

    #[cfg(feature = "roko")]
    if let Some(bus) = &chain.pheromone_bus {
        bus.broadcast(kind, broadcast_vector, req.intensity, now);
    }

    Ok(Json(serde_json::json!({
        "id": id.0,
        "kind": pheromone_kind_str(kind),
        "intensity": req.intensity,
        "deposited_at": now,
    })))
}

// ---------------------------------------------------------------------------
// GET /api/pheromones/:id/projection — decay projection for a single pheromone
// ---------------------------------------------------------------------------

fn default_projection_duration() -> u64 {
    3600
}

fn default_projection_steps() -> usize {
    12
}

#[derive(Debug, Deserialize)]
pub struct ProjectionQuery {
    /// Projection duration in seconds (default 3600).
    #[serde(default = "default_projection_duration")]
    pub duration_secs: u64,
    /// Number of steps in the projection (default 12).
    #[serde(default = "default_projection_steps")]
    pub steps: usize,
}

/// `GET /api/pheromones/:id/projection` — project decay over time for a pheromone.
pub async fn pheromone_projection(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
    Query(params): Query<ProjectionQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let chain = state.chain.read();
    let pheromone = chain.pheromones.get(PheromoneId(id)).ok_or(ApiError {
        error: format!("pheromone {id} not found"),
        code: 404,
    })?;

    let steps = params.steps.max(1).min(100);
    let duration = params.duration_secs.max(1);
    let step_secs = duration / steps as u64;
    let now = now_secs();
    let tau = pheromone.effective_half_life_seconds() as f64;

    let points: Vec<serde_json::Value> = (0..=steps)
        .map(|i| {
            let offset = step_secs * i as u64;
            let elapsed = now.saturating_sub(pheromone.deposited_at) as f64 + offset as f64;
            let projected = if tau > 0.0 {
                pheromone.base_intensity as f64 * (-elapsed / tau * std::f64::consts::LN_2).exp()
            } else {
                0.0
            };
            serde_json::json!({
                "offset_secs": offset,
                "projected_intensity": projected,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "id": id,
        "kind": pheromone_kind_str(pheromone.kind),
        "base_intensity": pheromone.base_intensity,
        "half_life_secs": pheromone.half_life_seconds,
        "effective_half_life_secs": pheromone.effective_half_life_seconds(),
        "points": points,
    })))
}
