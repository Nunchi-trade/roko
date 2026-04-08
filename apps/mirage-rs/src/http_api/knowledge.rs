//! Knowledge store HTTP endpoints.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};

use super::{ApiState, now_secs};
use crate::chain::{KnowledgeKind, KnowledgeState, PheromoneKind, projection::project_tokens};

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

#[derive(Serialize)]
pub struct EntryListResponse {
    pub entries: Vec<EntryItem>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub timestamp: u64,
}

pub async fn list_entries(
    State(state): State<ApiState>,
    Query(filter): Query<EntryFilter>,
) -> Json<EntryListResponse> {
    let now = now_secs();
    let chain = state.chain.read();

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
    let limit = filter.limit;
    let items: Vec<_> = items.into_iter().skip(offset).take(limit).collect();

    Json(EntryListResponse {
        entries: items,
        total,
        offset,
        limit,
        timestamp: now,
    })
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

#[derive(Serialize)]
pub struct EdgeListResponse {
    pub edges: Vec<Edge>,
    pub node_count: usize,
    pub timestamp: u64,
}

pub async fn list_edges(
    State(state): State<ApiState>,
    Query(filter): Query<EdgeFilter>,
) -> Json<EdgeListResponse> {
    let now = now_secs();
    let chain = state.chain.read();
    let threshold = filter.similarity_threshold.unwrap_or(0.5);
    let max_per_node = filter.max_hdc_edges_per_node.unwrap_or(5);
    let include_enabled_by = filter.include_enabled_by.unwrap_or(true);
    let include_hdc = filter.include_hdc.unwrap_or(true);

    let entries: Vec<_> = chain.knowledge.entries().collect();
    let node_count = entries.len();
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

    Json(EdgeListResponse {
        edges,
        node_count,
        timestamp: now,
    })
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
    let vector = project_tokens(&params.q);
    let chain = state.chain.read();
    let hits = chain.knowledge.search(&vector, params.k);

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
