//! Knowledge store HTTP endpoints.

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use super::{
    ApiError, ApiState, MAX_K, MAX_LIMIT, PaginatedResponse, now_secs, with_cache_control,
};
use crate::chain::{KnowledgeKind, KnowledgeState, PheromoneKind, insight::InsightId};

// ---------------------------------------------------------------------------
// GET /api/knowledge/entries
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EntryFilter {
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "super::default_limit")]
    pub limit: usize,
    /// Filter by kind: "insight", "heuristic", "warning", "causal_link",
    /// "strategy_fragment", "anti_knowledge".
    pub kind: Option<String>,
    /// Filter by state: "created", "active", "confirmed", "decaying",
    /// "challenged", "pruned", "stale".
    pub state: Option<String>,
    /// Minimum weight to include (useful for hiding decayed entries).
    pub min_weight: Option<f32>,
    /// Sort field: "weight" (default), "created_at", "confirmations".
    pub sort: Option<String>,
    /// Sort order: "desc" (default) or "asc".
    pub order: Option<String>,
}

#[derive(Serialize)]
pub struct EntryItem {
    pub id: String,
    pub kind: KnowledgeKind,
    pub weight: f32,
    pub initial_weight: f32,
    pub state: KnowledgeState,
    pub confirmations: usize,
    pub challenges: usize,
    pub created_at: u64,
    pub content: String,
    pub author: String,
    pub enabled_by: Vec<String>,
    pub half_life_seconds: u64,
    pub effective_half_life_seconds: u64,
    /// Stake in wei, serialized as string to avoid JSON number precision loss.
    pub stake_wei: String,
}

pub async fn list_entries(
    State(state): State<ApiState>,
    Query(filter): Query<EntryFilter>,
) -> impl IntoResponse {
    let now = now_secs();
    let chain = state.chain.read();

    let limit = filter.limit.min(MAX_LIMIT);
    let kind_filter = filter.kind.as_deref().and_then(parse_knowledge_kind);
    let state_filter = filter.state.as_deref().and_then(parse_knowledge_state);
    let min_weight = filter.min_weight.unwrap_or(0.0);

    let mut items: Vec<EntryItem> = chain
        .knowledge
        .entries()
        .filter(|e| kind_filter.map_or(true, |k| e.kind == k))
        .filter(|e| state_filter.map_or(true, |s| e.state == s))
        .filter(|e| e.weight >= min_weight)
        .map(|e| EntryItem {
            id: e.id.to_hex(),
            kind: e.kind,
            weight: e.weight,
            initial_weight: e.initial_weight,
            state: e.state,
            confirmations: e.confirmations.len(),
            challenges: e.challenges.len(),
            created_at: e.created_at,
            content: e.content.clone(),
            author: String::from_utf8_lossy(&e.author).into_owned(),
            enabled_by: e.enabled_by.iter().map(|id| id.to_hex()).collect(),
            half_life_seconds: e.half_life_seconds,
            effective_half_life_seconds: e.effective_half_life_seconds(),
            stake_wei: e.stake_wei.to_string(),
        })
        .collect();

    let desc = filter.order.as_deref() != Some("asc");
    match filter.sort.as_deref() {
        Some("created_at") => items.sort_by(|a, b| {
            if desc {
                b.created_at.cmp(&a.created_at)
            } else {
                a.created_at.cmp(&b.created_at)
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
                b.weight.total_cmp(&a.weight)
            } else {
                a.weight.total_cmp(&b.weight)
            }
        }),
    }

    let total = items.len();
    let offset = filter.offset;
    let items: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
    let _ = now; // used by filter computations above

    with_cache_control(PaginatedResponse::new(items, total, offset, limit), 2)
}

// ---------------------------------------------------------------------------
// GET /api/knowledge/edges
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EdgeFilter {
    /// Minimum HDC Hamming similarity to include (default 0.5).
    pub similarity_threshold: Option<f32>,
    /// Maximum HDC-similarity edges per node (default 5).
    pub max_hdc_edges_per_node: Option<usize>,
    /// Include `enabled_by` dependency edges (default true).
    pub include_enabled_by: Option<bool>,
    /// Include HDC similarity edges (default true).
    pub include_hdc: Option<bool>,
}

#[derive(Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f32>,
    #[serde(rename = "type")]
    pub edge_type: String,
}

pub async fn list_edges(
    State(state): State<ApiState>,
    Query(filter): Query<EdgeFilter>,
) -> impl IntoResponse {
    let now = now_secs();
    let chain = state.chain.read();
    let threshold = filter.similarity_threshold.unwrap_or(0.5);
    let max_per_node = filter.max_hdc_edges_per_node.unwrap_or(5);
    let include_enabled_by = filter.include_enabled_by.unwrap_or(true);
    let include_hdc = filter.include_hdc.unwrap_or(true);

    let entries: Vec<_> = chain.knowledge.entries().collect();
    let _node_count = entries.len();
    let mut edges = Vec::new();

    // Explicit dependency edges from enabled_by.
    if include_enabled_by {
        for entry in &entries {
            for dep_id in &entry.enabled_by {
                edges.push(Edge {
                    from: entry.id.to_hex(),
                    to: dep_id.to_hex(),
                    similarity: None,
                    edge_type: "enabled_by".to_owned(),
                });
            }
        }
    }

    // HDC-similarity edges (top-K neighbors above threshold per node).
    if include_hdc {
        for entry in &entries {
            if matches!(entry.state, KnowledgeState::Pruned | KnowledgeState::Stale) {
                continue;
            }
            let hits = chain.knowledge.search(&entry.vector, max_per_node + 1);
            for hit in hits {
                if hit.id == entry.id {
                    continue;
                }
                if hit.similarity >= threshold {
                    // Deduplicate: only emit edge from lower id to higher id.
                    if entry.id < hit.id {
                        edges.push(Edge {
                            from: entry.id.to_hex(),
                            to: hit.id.to_hex(),
                            similarity: Some(hit.similarity),
                            edge_type: "hdc".to_owned(),
                        });
                    }
                }
            }
        }
    }

    let total = edges.len();
    let _ = now;
    with_cache_control(PaginatedResponse::new(edges, total, 0, total), 2)
}

// ---------------------------------------------------------------------------
// GET /api/knowledge/search
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// Natural-language query text (projected to HDC internally).
    pub q: String,
    /// Number of results to return (default 10).
    #[serde(default = "default_k")]
    pub k: usize,
    /// Optional kind filter.
    pub kind: Option<String>,
}

fn default_k() -> usize {
    10
}

#[derive(Serialize)]
pub struct SearchHit {
    pub id: String,
    pub kind: KnowledgeKind,
    pub similarity: f32,
    pub weight: f32,
    pub score: f32,
    pub content: String,
    pub state: KnowledgeState,
    pub author: String,
    pub created_at: u64,
    pub confirmations: usize,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchHit>,
    pub query: String,
    pub timestamp: u64,
}

pub async fn search_knowledge(
    State(state): State<ApiState>,
    Query(params): Query<SearchParams>,
) -> Json<SearchResponse> {
    let now = now_secs();
    let vector = state.projection_cache.get_or_insert(&params.q);
    let chain = state.chain.read();
    let k = params.k.min(MAX_K);
    let hits = chain.knowledge.search(&vector, k);

    let kind_filter = params.kind.as_deref().and_then(parse_knowledge_kind);

    let results: Vec<SearchHit> = hits
        .into_iter()
        .filter_map(|hit| {
            let entry = chain.knowledge.get(hit.id)?;
            if let Some(k) = kind_filter {
                if entry.kind != k {
                    return None;
                }
            }
            Some(SearchHit {
                id: entry.id.to_hex(),
                kind: entry.kind,
                similarity: hit.similarity,
                weight: entry.weight,
                score: hit.similarity * entry.weight,
                content: entry.content.clone(),
                state: entry.state,
                author: String::from_utf8_lossy(&entry.author).into_owned(),
                created_at: entry.created_at,
                confirmations: entry.confirmations.len(),
            })
        })
        .collect();

    Json(SearchResponse {
        results,
        query: params.q,
        timestamp: now,
    })
}

// ---------------------------------------------------------------------------
// GET /api/knowledge/kinds
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct KindInfo {
    pub name: String,
    pub default_half_life_seconds: u64,
    pub base_reward_wei: String,
    pub count: usize,
}

#[derive(Serialize)]
pub struct PheromoneKindInfo {
    pub name: String,
    pub default_half_life_seconds: u64,
    pub count: usize,
}

#[derive(Serialize)]
pub struct KindsResponse {
    pub knowledge_kinds: Vec<KindInfo>,
    pub pheromone_kinds: Vec<PheromoneKindInfo>,
    pub timestamp: u64,
}

pub async fn list_kinds(State(state): State<ApiState>) -> Json<KindsResponse> {
    let now = now_secs();
    let chain = state.chain.read();

    let knowledge_variants = [
        KnowledgeKind::Insight,
        KnowledgeKind::Heuristic,
        KnowledgeKind::Warning,
        KnowledgeKind::CausalLink,
        KnowledgeKind::StrategyFragment,
        KnowledgeKind::AntiKnowledge,
    ];
    let knowledge_kinds: Vec<KindInfo> = knowledge_variants
        .into_iter()
        .map(|k| {
            let count = chain.knowledge.by_kind(k).len();
            let name = serde_json::to_value(k)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| format!("{k:?}"));
            KindInfo {
                name,
                default_half_life_seconds: k.default_half_life_seconds(),
                base_reward_wei: k.base_reward_wei().to_string(),
                count,
            }
        })
        .collect();

    let pheromone_variants = [
        PheromoneKind::Threat,
        PheromoneKind::Opportunity,
        PheromoneKind::Wisdom,
    ];
    let pheromone_kinds: Vec<PheromoneKindInfo> = pheromone_variants
        .into_iter()
        .map(|k| {
            let count = chain.pheromones.iter().filter(|p| p.kind == k).count();
            let name = serde_json::to_value(k)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| format!("{k:?}"));
            PheromoneKindInfo {
                name,
                default_half_life_seconds: k.default_half_life_seconds(),
                count,
            }
        })
        .collect();

    Json(KindsResponse {
        knowledge_kinds,
        pheromone_kinds,
        timestamp: now,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_knowledge_kind(s: &str) -> Option<KnowledgeKind> {
    match s {
        "insight" => Some(KnowledgeKind::Insight),
        "heuristic" => Some(KnowledgeKind::Heuristic),
        "warning" => Some(KnowledgeKind::Warning),
        "causal_link" => Some(KnowledgeKind::CausalLink),
        "strategy_fragment" => Some(KnowledgeKind::StrategyFragment),
        "anti_knowledge" => Some(KnowledgeKind::AntiKnowledge),
        _ => None,
    }
}

fn parse_knowledge_state(s: &str) -> Option<KnowledgeState> {
    match s {
        "created" => Some(KnowledgeState::Created),
        "active" => Some(KnowledgeState::Active),
        "confirmed" => Some(KnowledgeState::Confirmed),
        "decaying" => Some(KnowledgeState::Decaying),
        "challenged" => Some(KnowledgeState::Challenged),
        "pruned" => Some(KnowledgeState::Pruned),
        "stale" => Some(KnowledgeState::Stale),
        _ => None,
    }
}

/// Parses a 32-hex-char insight id, optionally prefixed with "insight:".
fn parse_insight_id(s: &str) -> Result<InsightId, ApiError> {
    let trimmed = s.strip_prefix("insight:").unwrap_or(s);
    if trimmed.len() != 32 {
        return Err(ApiError {
            error: format!("insight id must be 32 hex chars (got {})", trimmed.len()),
            code: 400,
        });
    }
    let mut bytes = [0u8; 16];
    for i in 0..16 {
        let hi = hex_nibble(trimmed.as_bytes()[i * 2])?;
        let lo = hex_nibble(trimmed.as_bytes()[i * 2 + 1])?;
        bytes[i] = (hi << 4) | lo;
    }
    Ok(InsightId(bytes))
}

fn hex_nibble(byte: u8) -> Result<u8, ApiError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ApiError {
            error: "invalid hex character in insight id".into(),
            code: 400,
        }),
    }
}

// ---------------------------------------------------------------------------
// POST /api/knowledge/entries — post a new insight
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PostInsightRequest {
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub enabled_by: Vec<String>,
    #[serde(default)]
    pub stake_wei: u128,
}

/// `POST /api/knowledge/entries` — post a new insight entry.
///
/// # Errors
///
/// Returns `400` if the knowledge kind is unknown, the content is empty, or
/// any enabled-by id is malformed, and `503` if the knowledge subsystem is
/// disabled.
pub async fn post_insight(
    State(state): State<ApiState>,
    Json(req): Json<PostInsightRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let kind = parse_knowledge_kind(&req.kind).ok_or(ApiError {
        error: format!("unknown knowledge kind: {}", req.kind),
        code: 400,
    })?;
    if req.content.is_empty() {
        return Err(ApiError {
            error: "content must not be empty".into(),
            code: 400,
        });
    }
    let enabled_by: Result<Vec<InsightId>, _> =
        req.enabled_by.iter().map(|s| parse_insight_id(s)).collect();
    let enabled_by = enabled_by?;
    let vector = state.projection_cache.get_or_insert(&req.content);
    let author_bytes = req.author.into_bytes();
    let now = now_secs();

    let mut chain = state.chain.write();
    if !chain.toggles.knowledge {
        return Err(ApiError {
            error: "knowledge subsystem is disabled".into(),
            code: 503,
        });
    }

    let broadcast_author_for_stats = author_bytes.clone();
    #[cfg(feature = "roko")]
    let broadcast_author = author_bytes.clone();
    #[cfg(feature = "roko")]
    let broadcast_content = req.content.clone();

    let outcome = chain.knowledge.post(
        author_bytes,
        kind,
        req.content,
        vector,
        enabled_by,
        now,
        req.stake_wei,
    );

    let (outcome_str, id_hex, similarity) = match &outcome {
        crate::chain::knowledge::PostOutcome::Accepted { id } => ("accepted", id.to_hex(), None),
        crate::chain::knowledge::PostOutcome::Duplicate {
            existing_id,
            similarity,
        } => ("duplicate", existing_id.to_hex(), Some(*similarity)),
        crate::chain::knowledge::PostOutcome::ExactMatch { id } => {
            ("exact_match", id.to_hex(), None)
        }
    };

    // Update author's insights_posted stat
    if matches!(
        outcome,
        crate::chain::knowledge::PostOutcome::Accepted { .. }
    ) {
        let author_str = String::from_utf8_lossy(&broadcast_author_for_stats).into_owned();
        chain.agent_registry.add_stats_delta(
            &author_str,
            &crate::chain::agent::AgentStats {
                insights_posted: 1,
                ..Default::default()
            },
        );
    }

    #[cfg(feature = "roko")]
    if matches!(
        outcome,
        crate::chain::knowledge::PostOutcome::Accepted { .. }
    ) {
        if let Some(bus) = &chain.insight_bus {
            if let Ok(event_id) = parse_insight_id(&id_hex) {
                bus.broadcast(crate::roko_bridge::InsightEvent::Posted {
                    id: event_id,
                    kind,
                    content: broadcast_content,
                    author: broadcast_author,
                    created_at: now,
                });
            }
        }
    }

    let mut resp = serde_json::json!({
        "outcome": outcome_str,
        "id": id_hex,
    });
    if let Some(sim) = similarity {
        resp["similarity"] = serde_json::json!(sim);
    }
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// POST /api/knowledge/entries/:id/confirm
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ConfirmRequest {
    pub confirmer: String,
}

/// `POST /api/knowledge/entries/:id/confirm` — confirm an insight entry.
///
/// # Errors
///
/// Returns `400` if the id is malformed, `404` if the entry is missing, `409`
/// if the entry is immutable or the confirmer is already recorded, and `503`
/// if the knowledge subsystem is disabled.
pub async fn confirm_entry(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<ConfirmRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let insight_id = parse_insight_id(&id)?;
    let confirmer_str = req.confirmer.clone();
    let confirmer_bytes = req.confirmer.into_bytes();

    let mut chain = state.chain.write();
    if !chain.toggles.knowledge {
        return Err(ApiError {
            error: "knowledge subsystem is disabled".into(),
            code: 503,
        });
    }

    #[cfg(feature = "roko")]
    let broadcast_confirmer = confirmer_bytes.clone();

    chain
        .knowledge
        .confirm(insight_id, confirmer_bytes)
        .map_err(knowledge_error_to_api)?;

    // Update confirmer's stats
    chain.agent_registry.add_stats_delta(
        &confirmer_str,
        &crate::chain::agent::AgentStats {
            confirmations_given: 1,
            ..Default::default()
        },
    );

    #[cfg(feature = "roko")]
    if let Some(bus) = &chain.insight_bus {
        bus.broadcast(crate::roko_bridge::InsightEvent::Confirmed {
            id: insight_id,
            by: broadcast_confirmer,
            at: now_secs(),
        });
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// POST /api/knowledge/entries/:id/challenge
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ChallengeRequest {
    pub challenger: String,
}

/// `POST /api/knowledge/entries/:id/challenge` — challenge an insight entry.
///
/// # Errors
///
/// Returns `400` if the id is malformed, `404` if the entry is missing, `409`
/// if the entry is immutable or the challenger is already recorded, and `503`
/// if the knowledge subsystem is disabled.
pub async fn challenge_entry(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<ChallengeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let insight_id = parse_insight_id(&id)?;
    let challenger_str = req.challenger.clone();
    let challenger_bytes = req.challenger.into_bytes();

    let mut chain = state.chain.write();
    if !chain.toggles.knowledge {
        return Err(ApiError {
            error: "knowledge subsystem is disabled".into(),
            code: 503,
        });
    }

    #[cfg(feature = "roko")]
    let broadcast_challenger = challenger_bytes.clone();

    chain
        .knowledge
        .challenge(insight_id, challenger_bytes)
        .map_err(knowledge_error_to_api)?;

    // Update challenger's stats
    chain.agent_registry.add_stats_delta(
        &challenger_str,
        &crate::chain::agent::AgentStats {
            challenges_given: 1,
            ..Default::default()
        },
    );

    #[cfg(feature = "roko")]
    if let Some(bus) = &chain.insight_bus {
        bus.broadcast(crate::roko_bridge::InsightEvent::Challenged {
            id: insight_id,
            by: broadcast_challenger,
            at: now_secs(),
        });
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// POST /api/knowledge/decay — trigger decay sweep
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DecayRequest {
    pub now_secs: Option<u64>,
}

/// `POST /api/knowledge/decay` — trigger a decay sweep on the knowledge store.
///
/// # Errors
///
/// Returns `503` if the knowledge subsystem is disabled.
pub async fn trigger_decay(
    State(state): State<ApiState>,
    Json(req): Json<DecayRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let now = req.now_secs.unwrap_or_else(now_secs);

    let mut chain = state.chain.write();
    if !chain.toggles.knowledge {
        return Err(ApiError {
            error: "knowledge subsystem is disabled".into(),
            code: 503,
        });
    }

    let before = chain
        .knowledge
        .entries()
        .filter(|e| !matches!(e.state, KnowledgeState::Pruned | KnowledgeState::Stale))
        .count();

    chain.knowledge.apply_decay(now);

    let after = chain
        .knowledge
        .entries()
        .filter(|e| !matches!(e.state, KnowledgeState::Pruned | KnowledgeState::Stale))
        .count();

    let pruned = before.saturating_sub(after);

    Ok(Json(serde_json::json!({
        "ok": true,
        "pruned": pruned,
        "remaining": after,
        "timestamp": now,
    })))
}

fn knowledge_error_to_api(e: crate::chain::KnowledgeError) -> ApiError {
    match e {
        crate::chain::KnowledgeError::NotFound(_) => ApiError {
            error: e.to_string(),
            code: 404,
        },
        crate::chain::KnowledgeError::DuplicateConfirmation(_)
        | crate::chain::KnowledgeError::DuplicateChallenge(_)
        | crate::chain::KnowledgeError::Immutable(_, _) => ApiError {
            error: e.to_string(),
            code: 409,
        },
    }
}
