//! JSON-RPC surface for chain extensions (gated by the `chain` feature).
//!
//! This module exposes the chain knowledge + pheromone substrates.
//!
//! It rides on the same jsonrpsee server that hosts the `eth_*` and `mirage_*`
//! methods. Wiring is opt-in: callers must construct a [`ChainContext`] and
//! pass it through [`crate::rpc::start_rpc_server_with_chain`], otherwise
//! mirage behaves as a pure EVM fork simulator.
//!
//! # Methods
//!
//! | Method                    | Args                                             | Returns                       |
//! |---------------------------|--------------------------------------------------|-------------------------------|
//! | `chain_postInsight`       | `{author, kind, content, enabledBy?, stakeWei?}` | `{outcome, id, ...}`          |
//! | `chain_searchInsights`    | `{query, k, kind?}`                              | `[{id, similarity, weight}]`  |
//! | `chain_confirmInsight`    | `{id, confirmer}`                                | `{ok: true}`                  |
//! | `chain_challengeInsight`  | `{id, challenger}`                               | `{ok: true}`                  |
//! | `chain_applyDecay`        | `{nowSecs?}`                                     | `{prunedCount}`               |
//! | `chain_getInsight`        | `{id}`                                           | `InsightEntry | null`         |
//! | `chain_depositPheromone`  | `{kind, content, intensity?, halfLifeSeconds?}`  | `{id}`                        |
//! | `chain_queryPheromones`   | `{query, k}`                                     | `[{id, kind, similarity, …}]` |
//! | `chain_stats`             | `{}`                                             | `{insights, pheromones}`      |
//!
//! The `query` and `content` fields are projected to HDC via
//! [`crate::chain::projection::project_tokens`] — no external embedding model
//! is required. Callers that have embeddings can project them with a
//! [`ProjectionMatrix`](crate::chain::projection::ProjectionMatrix) and pass
//! `{queryVector: <base64 1280 bytes>}` instead of `query`.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};

use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::chain::{
    HnswConfig, KnowledgeKind, KnowledgeStore, PheromoneField, PheromoneKind, PostOutcome,
    insight::InsightId, projection::project_tokens,
};
use roko_primitives::HdcVector;

#[cfg(feature = "roko")]
use crate::roko_bridge::{InsightBus, InsightEvent, PheromoneBus};

/// Toggles governing which chain subsystems are active.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChainToggles {
    /// Whether the HDC index accepts reads/writes (`chain_postInsight`, `chain_searchInsights`).
    pub hdc: bool,
    /// Whether the knowledge state machine is live (confirmations, challenges, decay).
    pub knowledge: bool,
    /// Whether stigmergy pheromones are enabled.
    pub stigmergy: bool,
}

impl ChainToggles {
    /// All subsystems enabled.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            hdc: true,
            knowledge: true,
            stigmergy: true,
        }
    }

    /// Returns whether at least one toggle is enabled.
    #[must_use]
    pub const fn any_enabled(self) -> bool {
        self.hdc || self.knowledge || self.stigmergy
    }
}

/// Chain substrate wrapped for the RPC surface.
pub struct ChainContext {
    /// The knowledge store (InsightEntry graph + HDC index).
    pub knowledge: KnowledgeStore,
    /// Pheromone field (stigmergic signals).
    pub pheromones: PheromoneField,
    /// Subsystem enable bits.
    pub toggles: ChainToggles,
    /// Optional subscription hub for pheromone deposits (roko feature only).
    ///
    /// When `Some`, every successful `handle_deposit_pheromone` broadcasts the
    /// new pheromone to every registered subscriber. `None` disables the
    /// streaming surface entirely.
    #[cfg(feature = "roko")]
    pub pheromone_bus: Option<Arc<PheromoneBus>>,
    /// Optional subscription hub for knowledge-layer events (roko feature only).
    ///
    /// When `Some`, `handle_post_insight`, `handle_confirm_insight`,
    /// `handle_challenge_insight`, and `handle_apply_decay` broadcast
    /// `InsightEvent`s to every registered subscriber.
    #[cfg(feature = "roko")]
    pub insight_bus: Option<Arc<InsightBus>>,
    /// Agent registry for identity, trace, and stats tracking.
    pub agent_registry: crate::chain::AgentRegistry,
    /// Broadcast bus for agent lifecycle events (WebSocket streaming).
    pub agent_bus: tokio::sync::broadcast::Sender<crate::chain::AgentEvent>,
    /// Task store for agent work coordination.
    pub task_store: crate::chain::TaskStore,
    /// Broadcast bus for task lifecycle events (WebSocket streaming).
    pub task_bus: tokio::sync::broadcast::Sender<crate::chain::TaskEvent>,
    /// Prediction session and claim store.
    pub prediction_store: crate::chain::PredictionStore,
    /// Broadcast bus for prediction lifecycle events.
    pub prediction_bus: tokio::sync::broadcast::Sender<crate::chain::PredictionEvent>,
}

impl ChainContext {
    /// Constructs a chain context with brute-force HDC only (no HNSW).
    #[must_use]
    pub fn new(toggles: ChainToggles) -> Self {
        Self {
            knowledge: KnowledgeStore::new(),
            pheromones: PheromoneField::default(),
            toggles,
            #[cfg(feature = "roko")]
            pheromone_bus: None,
            #[cfg(feature = "roko")]
            insight_bus: None,
            agent_registry: crate::chain::AgentRegistry::new(),
            agent_bus: tokio::sync::broadcast::channel(1_024).0,
            task_store: crate::chain::TaskStore::new(),
            task_bus: tokio::sync::broadcast::channel(1_024).0,
            prediction_store: crate::chain::PredictionStore::new(),
            prediction_bus: tokio::sync::broadcast::channel(1_024).0,
        }
    }

    /// Constructs a chain context that auto-activates HNSW once `hnsw_threshold` entries are indexed.
    #[must_use]
    pub fn with_hnsw(toggles: ChainToggles, hnsw_threshold: usize) -> Self {
        Self {
            knowledge: KnowledgeStore::with_hnsw(HnswConfig::default(), hnsw_threshold),
            pheromones: PheromoneField::default(),
            toggles,
            #[cfg(feature = "roko")]
            pheromone_bus: None,
            #[cfg(feature = "roko")]
            insight_bus: None,
            agent_registry: crate::chain::AgentRegistry::new(),
            agent_bus: tokio::sync::broadcast::channel(1_024).0,
            task_store: crate::chain::TaskStore::new(),
            task_bus: tokio::sync::broadcast::channel(1_024).0,
            prediction_store: crate::chain::PredictionStore::new(),
            prediction_bus: tokio::sync::broadcast::channel(1_024).0,
        }
    }

    /// Constructs a chain context with subscription buses eagerly installed.
    ///
    /// Callers that want live streaming (§38.d) should prefer this constructor
    /// over [`Self::new`] / [`Self::with_hnsw`]. The returned context carries
    /// two empty, process-wide buses that downstream RPC handlers can
    /// `register` against.
    #[cfg(feature = "roko")]
    #[must_use]
    pub fn new_with_subs(toggles: ChainToggles) -> Self {
        Self {
            knowledge: KnowledgeStore::new(),
            pheromones: PheromoneField::default(),
            toggles,
            pheromone_bus: Some(Arc::new(PheromoneBus::new())),
            insight_bus: Some(Arc::new(InsightBus::new())),
            agent_registry: crate::chain::AgentRegistry::new(),
            agent_bus: tokio::sync::broadcast::channel(1_024).0,
            task_store: crate::chain::TaskStore::new(),
            task_bus: tokio::sync::broadcast::channel(1_024).0,
            prediction_store: crate::chain::PredictionStore::new(),
            prediction_bus: tokio::sync::broadcast::channel(1_024).0,
        }
    }

    /// Installs (or replaces) the pheromone + insight buses on an existing
    /// context. Useful when the buses must outlive the context (e.g. handed
    /// down from the RPC server entry point).
    #[cfg(feature = "roko")]
    pub fn set_buses(&mut self, pheromone_bus: Arc<PheromoneBus>, insight_bus: Arc<InsightBus>) {
        self.pheromone_bus = Some(pheromone_bus);
        self.insight_bus = Some(insight_bus);
    }
}

impl std::fmt::Debug for ChainContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("ChainContext");
        dbg.field("toggles", &self.toggles)
            .field("insights", &self.knowledge.len())
            .field("pheromones", &self.pheromones.len())
            .field("tasks", &self.task_store.len())
            .field(
                "prediction_sessions",
                &self.prediction_store.session_count(),
            )
            .field("prediction_claims", &self.prediction_store.claim_count());
        #[cfg(feature = "roko")]
        {
            dbg.field("pheromone_bus", &self.pheromone_bus.is_some())
                .field("insight_bus", &self.insight_bus.is_some());
        }
        dbg.finish_non_exhaustive()
    }
}

/// JSON-RPC error codes used by the chain namespace.
pub mod err_code {
    /// Subsystem is disabled via toggles.
    pub const DISABLED: i32 = -32600;
    /// Required field missing or ill-typed.
    pub const INVALID: i32 = -32602;
    /// Entry not found.
    pub const NOT_FOUND: i32 = -32100;
    /// Duplicate operation (already confirmed / challenged).
    pub const DUPLICATE: i32 = -32101;
    /// Entry is in a terminal state.
    pub const IMMUTABLE: i32 = -32102;
    /// Posting was collapsed into an existing entry.
    pub const DUPLICATE_CONTENT: i32 = -32103;
}

fn rpc_err(code: i32, message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObject::owned::<()>(code, message.into(), None)
}

fn disabled(subsystem: &str) -> ErrorObjectOwned {
    rpc_err(
        err_code::DISABLED,
        format!(
            "chain subsystem '{subsystem}' is disabled (set --enable-{subsystem} or override via toggles)"
        ),
    )
}

fn parse_kind(value: &str) -> Result<KnowledgeKind, ErrorObjectOwned> {
    match value {
        "insight" => Ok(KnowledgeKind::Insight),
        "heuristic" => Ok(KnowledgeKind::Heuristic),
        "warning" => Ok(KnowledgeKind::Warning),
        "causal_link" | "causalLink" => Ok(KnowledgeKind::CausalLink),
        "strategy_fragment" | "strategyFragment" => Ok(KnowledgeKind::StrategyFragment),
        "anti_knowledge" | "antiKnowledge" => Ok(KnowledgeKind::AntiKnowledge),
        other => Err(rpc_err(
            err_code::INVALID,
            format!("unknown knowledge kind: {other}"),
        )),
    }
}

fn parse_pheromone_kind(value: &str) -> Result<PheromoneKind, ErrorObjectOwned> {
    match value {
        "threat" => Ok(PheromoneKind::Threat),
        "opportunity" => Ok(PheromoneKind::Opportunity),
        "wisdom" => Ok(PheromoneKind::Wisdom),
        other => Err(rpc_err(
            err_code::INVALID,
            format!("unknown pheromone kind: {other}"),
        )),
    }
}

fn parse_insight_id(s: &str) -> Result<InsightId, ErrorObjectOwned> {
    let trimmed = s.strip_prefix("insight:").unwrap_or(s);
    if trimmed.len() != 32 {
        return Err(rpc_err(
            err_code::INVALID,
            format!("insight id must be 32 hex chars (got {})", trimmed.len()),
        ));
    }
    let mut bytes = [0u8; 16];
    for i in 0..16 {
        let hi = hex_nibble(trimmed.as_bytes()[i * 2])?;
        let lo = hex_nibble(trimmed.as_bytes()[i * 2 + 1])?;
        bytes[i] = (hi << 4) | lo;
    }
    Ok(InsightId(bytes))
}

fn hex_nibble(byte: u8) -> Result<u8, ErrorObjectOwned> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(rpc_err(err_code::INVALID, "invalid hex character in id")),
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn project_query(params: &JsonValue) -> Result<HdcVector, ErrorObjectOwned> {
    if let Some(text) = params.get("query").and_then(|v| v.as_str()) {
        return Ok(project_tokens(text));
    }
    if let Some(vec_bytes) = params.get("queryVector").and_then(|v| v.as_array()) {
        if vec_bytes.len() != 1280 {
            return Err(rpc_err(
                err_code::INVALID,
                format!("queryVector must be 1280 bytes (got {})", vec_bytes.len()),
            ));
        }
        let mut bytes = [0u8; 1280];
        for (i, byte_val) in vec_bytes.iter().enumerate() {
            bytes[i] = byte_val
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| rpc_err(err_code::INVALID, "queryVector byte out of range"))?;
        }
        return Ok(HdcVector::from_bytes(&bytes));
    }
    Err(rpc_err(
        err_code::INVALID,
        "expected 'query' (text) or 'queryVector' (1280-byte array)",
    ))
}

/// Request payload for `chain_postInsight`.
#[derive(Clone, Debug, Deserialize)]
pub struct PostInsightParams {
    /// Hex or UTF-8 encoded author id.
    pub author: String,
    /// Knowledge kind (snake_case).
    pub kind: String,
    /// Content to post.
    pub content: String,
    /// Optional parent ids.
    #[serde(default)]
    pub enabled_by: Vec<String>,
    /// Stake in wei (defaults to 0).
    #[serde(default)]
    pub stake_wei: Option<u128>,
}

/// Response payload for `chain_postInsight`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostInsightResult {
    /// `"accepted"`, `"duplicate"`, or `"exact_match"`.
    pub outcome: String,
    /// Content-addressed id of the relevant entry.
    pub id: String,
    /// When `outcome == "duplicate"`, the Hamming similarity to the existing entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f32>,
}

/// Handler for `chain_postInsight`.
///
/// # Errors
///
/// Returns an `INVALID` JSON-RPC error if the knowledge layer is disabled,
/// the kind is unknown, or any `enabled_by` id is malformed.
pub fn handle_post_insight(
    chain: &Arc<RwLock<ChainContext>>,
    params: PostInsightParams,
) -> Result<PostInsightResult, ErrorObjectOwned> {
    let mut chain_lock = chain.write();
    if !chain_lock.toggles.knowledge {
        return Err(disabled("knowledge"));
    }
    let kind = parse_kind(&params.kind)?;
    let enabled_by: Result<Vec<_>, _> = params
        .enabled_by
        .iter()
        .map(|s| parse_insight_id(s))
        .collect();
    let enabled_by = enabled_by?;
    let vector = project_tokens(&params.content);
    let author_bytes = params.author.into_bytes();
    #[cfg(feature = "roko")]
    let broadcast_author = author_bytes.clone();
    #[cfg(feature = "roko")]
    let broadcast_content = params.content.clone();
    let created_at = now_seconds();
    let outcome = chain_lock.knowledge.post(
        author_bytes,
        kind,
        params.content,
        vector,
        enabled_by,
        created_at,
        params.stake_wei.unwrap_or(0),
    );
    #[cfg(feature = "roko")]
    if let PostOutcome::Accepted { id } = &outcome {
        if let Some(bus) = &chain_lock.insight_bus {
            bus.broadcast(InsightEvent::Posted {
                id: *id,
                kind,
                content: broadcast_content,
                author: broadcast_author,
                created_at,
            });
        }
    }
    let response = match outcome {
        PostOutcome::Accepted { id } => PostInsightResult {
            outcome: "accepted".into(),
            id: id.to_hex(),
            similarity: None,
        },
        PostOutcome::Duplicate {
            existing_id,
            similarity,
        } => PostInsightResult {
            outcome: "duplicate".into(),
            id: existing_id.to_hex(),
            similarity: Some(similarity),
        },
        PostOutcome::ExactMatch { id } => PostInsightResult {
            outcome: "exact_match".into(),
            id: id.to_hex(),
            similarity: None,
        },
    };
    Ok(response)
}

/// Request payload for `chain_searchInsights`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchInsightsParams {
    /// Free-text query to project into HDC.
    #[serde(default)]
    pub query: Option<String>,
    /// Alternate 1280-byte HDC query vector.
    #[serde(default)]
    pub query_vector: Option<Vec<u8>>,
    /// Number of results.
    #[serde(default = "default_k")]
    pub k: usize,
    /// Optional kind filter.
    #[serde(default)]
    pub kind: Option<String>,
}

const fn default_k() -> usize {
    10
}

/// Handler for `chain_searchInsights`.
///
/// # Errors
///
/// Returns an `INVALID` JSON-RPC error if the HDC layer is disabled, the
/// query payload is malformed, or `queryVector` is not 1280 bytes long.
pub fn handle_search_insights(
    chain: &Arc<RwLock<ChainContext>>,
    params: SearchInsightsParams,
) -> Result<JsonValue, ErrorObjectOwned> {
    let chain_lock = chain.read();
    if !chain_lock.toggles.hdc {
        return Err(disabled("hdc"));
    }
    let kind_filter = params.kind.as_deref().map(parse_kind).transpose()?;
    let query = if let Some(text) = params.query {
        project_tokens(&text)
    } else if let Some(bytes) = params.query_vector {
        if bytes.len() != 1280 {
            return Err(rpc_err(
                err_code::INVALID,
                format!("queryVector must be 1280 bytes (got {})", bytes.len()),
            ));
        }
        let mut arr = [0u8; 1280];
        arr.copy_from_slice(&bytes);
        HdcVector::from_bytes(&arr)
    } else {
        return Err(rpc_err(
            err_code::INVALID,
            "expected 'query' (text) or 'queryVector'",
        ));
    };
    let hits = chain_lock.knowledge.search(&query, params.k);
    let results: Vec<JsonValue> = hits
        .into_iter()
        .filter_map(|hit| {
            let entry = chain_lock.knowledge.get(hit.id)?;
            if let Some(k) = kind_filter {
                if entry.kind != k {
                    return None;
                }
            }
            Some(json!({
                "id": format!("insight:{}", hit.id.to_hex()),
                "kind": serde_json::to_value(entry.kind).ok(),
                "content": entry.content,
                "similarity": hit.similarity,
                "weight": hit.weight,
                "score": hit.score,
                "confirmations": entry.confirmations.len(),
                "challenges": entry.challenges.len(),
                "state": serde_json::to_value(entry.state).ok(),
            }))
        })
        .collect();
    Ok(json!({ "results": results }))
}

/// Request payload for `chain_confirmInsight`.
#[derive(Clone, Debug, Deserialize)]
pub struct ConfirmInsightParams {
    /// Insight id (hex).
    pub id: String,
    /// Confirmer address (UTF-8 bytes).
    pub confirmer: String,
}

/// Handler for `chain_confirmInsight`.
///
/// # Errors
///
/// Returns `INVALID`, `NOT_FOUND`, `DUPLICATE`, or `IMMUTABLE` JSON-RPC
/// errors when the knowledge layer is disabled, the id is malformed, or the
/// store rejects the confirmation.
pub fn handle_confirm_insight(
    chain: &Arc<RwLock<ChainContext>>,
    params: ConfirmInsightParams,
) -> Result<JsonValue, ErrorObjectOwned> {
    let mut chain_lock = chain.write();
    if !chain_lock.toggles.knowledge {
        return Err(disabled("knowledge"));
    }
    let id = parse_insight_id(&params.id)?;
    let confirmer_bytes = params.confirmer.into_bytes();
    #[cfg(feature = "roko")]
    let broadcast_confirmer = confirmer_bytes.clone();
    #[cfg(feature = "roko")]
    let at = now_seconds();
    chain_lock
        .knowledge
        .confirm(id, confirmer_bytes)
        .map_err(|e| match e {
            crate::chain::KnowledgeError::NotFound(_) => {
                rpc_err(err_code::NOT_FOUND, e.to_string())
            }
            crate::chain::KnowledgeError::DuplicateConfirmation(_) => {
                rpc_err(err_code::DUPLICATE, e.to_string())
            }
            crate::chain::KnowledgeError::Immutable(_, _) => {
                rpc_err(err_code::IMMUTABLE, e.to_string())
            }
            other @ crate::chain::KnowledgeError::DuplicateChallenge(_) => {
                rpc_err(err_code::INVALID, other.to_string())
            }
        })?;
    #[cfg(feature = "roko")]
    if let Some(bus) = &chain_lock.insight_bus {
        bus.broadcast(InsightEvent::Confirmed {
            id,
            by: broadcast_confirmer,
            at,
        });
    }
    Ok(json!({ "ok": true }))
}

/// Request payload for `chain_challengeInsight`.
#[derive(Clone, Debug, Deserialize)]
pub struct ChallengeInsightParams {
    /// Insight id (hex).
    pub id: String,
    /// Challenger address.
    pub challenger: String,
}

/// Handler for `chain_challengeInsight`.
///
/// # Errors
///
/// Returns `INVALID`, `NOT_FOUND`, `DUPLICATE`, or `IMMUTABLE` JSON-RPC
/// errors when the knowledge layer is disabled, the id is malformed, or the
/// store rejects the challenge.
pub fn handle_challenge_insight(
    chain: &Arc<RwLock<ChainContext>>,
    params: ChallengeInsightParams,
) -> Result<JsonValue, ErrorObjectOwned> {
    let mut chain_lock = chain.write();
    if !chain_lock.toggles.knowledge {
        return Err(disabled("knowledge"));
    }
    let id = parse_insight_id(&params.id)?;
    let challenger_bytes = params.challenger.into_bytes();
    #[cfg(feature = "roko")]
    let broadcast_challenger = challenger_bytes.clone();
    #[cfg(feature = "roko")]
    let at = now_seconds();
    chain_lock
        .knowledge
        .challenge(id, challenger_bytes)
        .map_err(|e| match e {
            crate::chain::KnowledgeError::NotFound(_) => {
                rpc_err(err_code::NOT_FOUND, e.to_string())
            }
            crate::chain::KnowledgeError::DuplicateChallenge(_) => {
                rpc_err(err_code::DUPLICATE, e.to_string())
            }
            crate::chain::KnowledgeError::Immutable(_, _) => {
                rpc_err(err_code::IMMUTABLE, e.to_string())
            }
            other @ crate::chain::KnowledgeError::DuplicateConfirmation(_) => {
                rpc_err(err_code::INVALID, other.to_string())
            }
        })?;
    #[cfg(feature = "roko")]
    if let Some(bus) = &chain_lock.insight_bus {
        bus.broadcast(InsightEvent::Challenged {
            id,
            by: broadcast_challenger,
            at,
        });
    }
    Ok(json!({ "ok": true }))
}

/// Handler for `chain_getInsight`.
///
/// # Errors
///
/// Returns an `INVALID` JSON-RPC error if `id` is missing or malformed.
pub fn handle_get_insight(
    chain: &Arc<RwLock<ChainContext>>,
    params: JsonValue,
) -> Result<JsonValue, ErrorObjectOwned> {
    let id_str = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_err(err_code::INVALID, "missing field: id"))?;
    let id = parse_insight_id(id_str)?;
    let chain_lock = chain.read();
    let entry = chain_lock.knowledge.get(id);
    match entry {
        Some(e) => Ok(json!({
            "id": format!("insight:{}", e.id.to_hex()),
            "author": String::from_utf8_lossy(&e.author),
            "kind": serde_json::to_value(e.kind).ok(),
            "content": e.content,
            "state": serde_json::to_value(e.state).ok(),
            "createdAt": e.created_at,
            "halfLifeSeconds": e.half_life_seconds,
            "initialWeight": e.initial_weight,
            "weight": e.weight,
            "confirmations": e.confirmations.len(),
            "challenges": e.challenges.len(),
            "stakeWei": e.stake_wei.to_string(),
            "enabledBy": e.enabled_by.iter().map(|id| format!("insight:{}", id.to_hex())).collect::<Vec<_>>(),
        })),
        None => Ok(JsonValue::Null),
    }
}

/// Handler for `chain_applyDecay`.
///
/// # Errors
///
/// Returns an `INVALID` JSON-RPC error if the knowledge layer is disabled or
/// the request payload cannot be interpreted.
pub fn handle_apply_decay(
    chain: &Arc<RwLock<ChainContext>>,
    params: JsonValue,
) -> Result<JsonValue, ErrorObjectOwned> {
    let mut chain_lock = chain.write();
    if !chain_lock.toggles.knowledge {
        return Err(disabled("knowledge"));
    }
    let now_secs = params
        .get("nowSecs")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(now_seconds);
    let before = chain_lock.knowledge.len();
    chain_lock.knowledge.apply_decay(now_secs);
    let after_active: usize = chain_lock
        .knowledge
        .entries()
        .filter(|e| {
            !matches!(
                e.state,
                crate::chain::KnowledgeState::Pruned | crate::chain::KnowledgeState::Stale
            )
        })
        .count();
    // §38.d: broadcast a Decayed event per surviving entry so subscribers
    // observe fresh weights after each decay sweep. Entries that were pruned
    // this sweep are not emitted (subscribers should track prior weight).
    #[cfg(feature = "roko")]
    if let Some(bus) = &chain_lock.insight_bus {
        let events: Vec<InsightEvent> = chain_lock
            .knowledge
            .entries()
            .filter(|e| {
                !matches!(
                    e.state,
                    crate::chain::KnowledgeState::Pruned | crate::chain::KnowledgeState::Stale
                )
            })
            .map(|e| InsightEvent::Decayed {
                id: e.id,
                new_weight: e.weight,
                at: now_secs,
            })
            .collect();
        for ev in events {
            bus.broadcast(ev);
        }
    }
    Ok(json!({
        "total": before,
        "activeAfter": after_active,
        "prunedCount": before.saturating_sub(after_active),
    }))
}

/// Request payload for `chain_depositPheromone`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositPheromoneParams {
    /// Pheromone kind (`"threat"`, `"opportunity"`, `"wisdom"`).
    pub kind: String,
    /// Content to project into an HDC vector.
    pub content: String,
    /// Initial intensity in `[0, 1]`. Default 1.0.
    #[serde(default = "default_intensity")]
    pub intensity: f32,
    /// Custom half-life override (seconds).
    #[serde(default)]
    pub half_life_seconds: Option<u64>,
}

const fn default_intensity() -> f32 {
    1.0
}

/// Handler for `chain_depositPheromone`.
///
/// # Errors
///
/// Returns an `INVALID` JSON-RPC error if the stigmergy layer is disabled or
/// the pheromone kind is unknown.
pub fn handle_deposit_pheromone(
    chain: &Arc<RwLock<ChainContext>>,
    params: DepositPheromoneParams,
) -> Result<JsonValue, ErrorObjectOwned> {
    let mut chain_lock = chain.write();
    if !chain_lock.toggles.stigmergy {
        return Err(disabled("stigmergy"));
    }
    let kind = parse_pheromone_kind(&params.kind)?;
    let vector = project_tokens(&params.content);
    let deposited_at = now_seconds();
    #[cfg(feature = "roko")]
    let broadcast_vector = vector.clone();
    let id = if let Some(tau) = params.half_life_seconds {
        chain_lock.pheromones.deposit_with_half_life(
            kind,
            vector,
            params.intensity,
            deposited_at,
            tau,
        )
    } else {
        chain_lock
            .pheromones
            .deposit(kind, vector, params.intensity, deposited_at)
    };
    #[cfg(feature = "roko")]
    if let Some(bus) = &chain_lock.pheromone_bus {
        bus.broadcast(kind, broadcast_vector, params.intensity, deposited_at);
    }
    Ok(json!({ "id": id.0 }))
}

/// Handler for `chain_queryPheromones`.
///
/// # Errors
///
/// Returns an `INVALID` JSON-RPC error if the stigmergy layer is disabled or
/// the query payload is malformed.
pub fn handle_query_pheromones(
    chain: &Arc<RwLock<ChainContext>>,
    params: JsonValue,
) -> Result<JsonValue, ErrorObjectOwned> {
    let chain_lock = chain.read();
    if !chain_lock.toggles.stigmergy {
        return Err(disabled("stigmergy"));
    }
    let query = project_query(&params)?;
    let k = params.get("k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let hits = chain_lock.pheromones.query_top_k(&query, k, now_seconds());
    let results: Vec<JsonValue> = hits
        .into_iter()
        .map(|h| {
            json!({
                "id": h.id.0,
                "kind": serde_json::to_value(h.kind).ok(),
                "similarity": h.similarity,
                "intensity": h.intensity,
                "score": h.score,
            })
        })
        .collect();
    Ok(json!({ "results": results }))
}

/// Handler for `chain_stats`.
pub fn handle_stats(chain: &Arc<RwLock<ChainContext>>) -> JsonValue {
    let chain_lock = chain.read();
    #[cfg(feature = "roko")]
    let subscriptions = subscription_stats_json(&chain_lock);
    #[cfg(not(feature = "roko"))]
    let subscriptions = json!({
        "pheromoneCount": 0,
        "insightCount": 0,
        "droppedOldest": 0,
        "droppedNewest": 0,
        "closedByOverflow": 0,
    });
    json!({
        "insights": chain_lock.knowledge.len(),
        "pheromones": chain_lock.pheromones.len(),
        "toggles": {
            "hdc": chain_lock.toggles.hdc,
            "knowledge": chain_lock.toggles.knowledge,
            "stigmergy": chain_lock.toggles.stigmergy,
        },
        "subscriptions": subscriptions,
    })
}

/// §38.16 — aggregates back-pressure counters across every registered
/// subscription on both buses.
///
/// `pheromoneCount` / `insightCount` report the live subscription headcounts;
/// `droppedOldest` / `droppedNewest` are summed across all subscriptions (and
/// buses); `closedByOverflow` counts subscriptions whose counters report
/// `closed == true`.
#[cfg(feature = "roko")]
fn subscription_stats_json(chain: &ChainContext) -> JsonValue {
    let mut pheromone_count = 0usize;
    let mut insight_count = 0usize;
    let mut dropped_oldest = 0u64;
    let mut dropped_newest = 0u64;
    let mut closed_by_overflow = 0usize;
    if let Some(bus) = &chain.pheromone_bus {
        for (_, stats) in bus.all_stats() {
            pheromone_count += 1;
            dropped_oldest += stats.dropped_oldest;
            dropped_newest += stats.dropped_newest;
            if stats.closed {
                closed_by_overflow += 1;
            }
        }
    }
    if let Some(bus) = &chain.insight_bus {
        for (_, stats) in bus.all_stats() {
            insight_count += 1;
            dropped_oldest += stats.dropped_oldest;
            dropped_newest += stats.dropped_newest;
            if stats.closed {
                closed_by_overflow += 1;
            }
        }
    }
    json!({
        "pheromoneCount": pheromone_count,
        "insightCount": insight_count,
        "droppedOldest": dropped_oldest,
        "droppedNewest": dropped_newest,
        "closedByOverflow": closed_by_overflow,
    })
}

/// Prefix tagging a pheromone-bus subscription id in its string form.
#[cfg(feature = "roko")]
pub const PHEROMONE_SUB_PREFIX: &str = "pher:";
/// Prefix tagging an insight-bus subscription id in its string form.
#[cfg(feature = "roko")]
pub const INSIGHT_SUB_PREFIX: &str = "insi:";

/// §38.d — orchestrates per-connection `WebSocket` subscriptions on top of the
/// process-wide [`PheromoneBus`] / [`InsightBus`].
///
/// The manager doesn't own the buses; it holds `Arc` handles so the RPC
/// `register_subscription` closures can register new mpsc sinks without
/// touching the `ChainContext` write lock.
///
/// Every registered bus subscription is tagged with a prefix (see
/// [`PHEROMONE_SUB_PREFIX`] / [`INSIGHT_SUB_PREFIX`]) so a single
/// `chain_unsubscribe(subscriptionId)` call can route to the correct bus.
#[cfg(feature = "roko")]
#[derive(Clone)]
#[must_use]
pub struct SubscriptionManager {
    pheromone_bus: Arc<PheromoneBus>,
    insight_bus: Arc<InsightBus>,
}

#[cfg(feature = "roko")]
impl SubscriptionManager {
    /// Constructs a manager that references the two buses.
    pub fn new(pheromone_bus: Arc<PheromoneBus>, insight_bus: Arc<InsightBus>) -> Self {
        Self {
            pheromone_bus,
            insight_bus,
        }
    }

    /// Constructs a manager whose buses are fresh (process-local) instances.
    /// Useful for tests and standalone invocations.
    pub fn with_fresh_buses() -> Self {
        Self::new(Arc::new(PheromoneBus::new()), Arc::new(InsightBus::new()))
    }

    /// Returns the underlying pheromone bus.
    pub fn pheromones(&self) -> &Arc<PheromoneBus> {
        &self.pheromone_bus
    }

    /// Returns the underlying insight bus.
    pub fn insights(&self) -> &Arc<InsightBus> {
        &self.insight_bus
    }

    /// Registers a pheromone sink with the bus and returns the **external**
    /// subscription id (prefixed so `chain_unsubscribe` can route).
    pub fn register_pheromone_sink(
        &self,
        sink: Arc<dyn crate::roko_bridge::SubscriptionSink<crate::roko_bridge::PheromoneEvent>>,
        policy: crate::roko_bridge::BackpressurePolicy,
    ) -> String {
        let id = self.pheromone_bus.register(sink, policy);
        format!("{}{}", PHEROMONE_SUB_PREFIX, id.0)
    }

    /// Registers an insight sink with the bus and returns the external id.
    pub fn register_insight_sink(
        &self,
        sink: Arc<dyn crate::roko_bridge::SubscriptionSink<crate::roko_bridge::InsightEvent>>,
        policy: crate::roko_bridge::BackpressurePolicy,
    ) -> String {
        let id = self.insight_bus.register(sink, policy);
        format!("{}{}", INSIGHT_SUB_PREFIX, id.0)
    }

    /// Parses a tagged external id and unregisters the matching bus
    /// subscription. Returns `true` if an active subscription was removed.
    pub fn unsubscribe(&self, external_id: &str) -> bool {
        if let Some(rest) = external_id.strip_prefix(PHEROMONE_SUB_PREFIX) {
            if let Ok(n) = rest.parse::<u64>() {
                return self
                    .pheromone_bus
                    .unregister(crate::roko_bridge::SubscriptionId(n));
            }
        } else if let Some(rest) = external_id.strip_prefix(INSIGHT_SUB_PREFIX) {
            if let Ok(n) = rest.parse::<u64>() {
                return self
                    .insight_bus
                    .unregister(crate::roko_bridge::SubscriptionId(n));
            }
        }
        false
    }
}

/// Handler for `chain_unsubscribe` — routes based on the id prefix, removing
/// the subscription from the appropriate bus.
///
/// Returns `{"ok": true}` when a subscription was removed, or an error with
/// code [`err_code::NOT_FOUND`] when the id is malformed or unknown.
#[cfg(feature = "roko")]
pub fn handle_unsubscribe(
    manager: &SubscriptionManager,
    params: JsonValue,
) -> Result<JsonValue, ErrorObjectOwned> {
    let id = params
        .get("subscriptionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_err(err_code::INVALID, "missing field: subscriptionId"))?;
    if manager.unsubscribe(id) {
        Ok(json!({ "ok": true }))
    } else {
        Err(rpc_err(
            err_code::NOT_FOUND,
            format!("no subscription with id: {id}"),
        ))
    }
}

/// Serializes a [`crate::roko_bridge::PheromoneEvent`] to JSON for WS delivery.
#[cfg(feature = "roko")]
pub fn pheromone_event_to_json(event: &crate::roko_bridge::PheromoneEvent) -> JsonValue {
    json!({
        "id": event.id,
        "kind": serde_json::to_value(event.kind).ok(),
        "intensity": event.intensity,
        "depositedAt": event.deposited_at,
        // vector omitted from wire (1280 bytes); subscribers that need it call
        // chain_queryPheromones with the embedded id.
    })
}

/// Serializes an [`crate::roko_bridge::InsightEvent`] to JSON for WS delivery.
#[cfg(feature = "roko")]
pub fn insight_event_to_json(event: &crate::roko_bridge::InsightEvent) -> JsonValue {
    use crate::roko_bridge::InsightEvent as E;
    match event {
        E::Posted {
            id,
            kind,
            content,
            author,
            created_at,
        } => json!({
            "type": "posted",
            "id": format!("insight:{}", id.to_hex()),
            "kind": serde_json::to_value(kind).ok(),
            "content": content,
            "author": String::from_utf8_lossy(author),
            "createdAt": created_at,
        }),
        E::StateTransition { id, from, to, at } => json!({
            "type": "stateTransition",
            "id": format!("insight:{}", id.to_hex()),
            "from": serde_json::to_value(from).ok(),
            "to": serde_json::to_value(to).ok(),
            "at": at,
        }),
        E::Confirmed { id, by, at } => json!({
            "type": "confirmed",
            "id": format!("insight:{}", id.to_hex()),
            "by": String::from_utf8_lossy(by),
            "at": at,
        }),
        E::Challenged { id, by, at } => json!({
            "type": "challenged",
            "id": format!("insight:{}", id.to_hex()),
            "by": String::from_utf8_lossy(by),
            "at": at,
        }),
        E::Decayed { id, new_weight, at } => json!({
            "type": "decayed",
            "id": format!("insight:{}", id.to_hex()),
            "newWeight": new_weight,
            "at": at,
        }),
    }
}

/// Canonical list of every `chain_*` JSON-RPC method exposed by this surface.
///
/// Used by [`handle_version`] and [`handle_method_schema`].
///
/// If you add a new `chain_*` method in [`crate::rpc`], add it here too. The
/// unit tests assert that every entry has a schema.
pub const CHAIN_METHOD_NAMES: &[&str] = &[
    "chain_postInsight",
    "chain_searchInsights",
    "chain_confirmInsight",
    "chain_challengeInsight",
    "chain_getInsight",
    "chain_applyDecay",
    "chain_depositPheromone",
    "chain_queryPheromones",
    "chain_stats",
    "chain_version",
    "chain_listKinds",
    "chain_methodSchema",
    "chain_subscribePheromones",
    "chain_subscribeInsights",
    "chain_unsubscribe",
];

/// Stable version of the `chain_*` RPC surface (`SemVer`).
///
/// Bump the major when a method's request/response shape changes in a
/// backwards-incompatible way; bump the minor for additive changes.
pub const CHAIN_RPC_VERSION: &str = "1.0.0";

/// Monotonic integer identifying the serialized layout of chain types
/// (`InsightEntry`, `Pheromone`, `HdcVector`) as emitted by these handlers.
pub const CHAIN_SCHEMA_VERSION: u32 = 1;

/// Handler for `chain_version`.
///
/// Returns the RPC surface version, the crate version of `mirage-rs`, the
/// chain schema version, the full list of supported `chain_*` methods, and
/// the current toggle state so clients can plan around disabled subsystems.
pub fn handle_version(chain: &Arc<RwLock<ChainContext>>) -> JsonValue {
    let chain_lock = chain.read();
    json!({
        "rpcVersion": CHAIN_RPC_VERSION,
        "mirageVersion": env!("CARGO_PKG_VERSION"),
        "schemaVersion": CHAIN_SCHEMA_VERSION,
        "supportedMethods": CHAIN_METHOD_NAMES,
        "features": {
            "hdc": chain_lock.toggles.hdc,
            "knowledge": chain_lock.toggles.knowledge,
            "stigmergy": chain_lock.toggles.stigmergy,
        },
    })
}

/// Handler for `chain_listKinds`.
///
/// Enumerates the serde-snake-case spellings of `KnowledgeKind`,
/// `PheromoneKind`, and `KnowledgeState`. The lists are hand-coded because
/// the enums do not derive a `Variants`-style trait; they are kept in sync
/// with the enum definitions in [`crate::chain::insight`] and
/// [`crate::chain::pheromone`] (see unit tests for the arity assertions).
pub fn handle_list_kinds() -> JsonValue {
    // Variants of KnowledgeKind — see src/chain/insight.rs
    // (#[serde(rename_all = "snake_case")]).
    let knowledge_kinds = [
        "insight",
        "heuristic",
        "warning",
        "causal_link",
        "strategy_fragment",
        "anti_knowledge",
    ];
    // Variants of PheromoneKind — see src/chain/pheromone.rs.
    let pheromone_kinds = ["threat", "opportunity", "wisdom"];
    // Variants of KnowledgeState — see src/chain/insight.rs.
    let insight_states = [
        "created",
        "active",
        "confirmed",
        "decaying",
        "challenged",
        "pruned",
        "stale",
    ];
    json!({
        "knowledgeKinds": knowledge_kinds,
        "pheromoneKinds": pheromone_kinds,
        "insightStates": insight_states,
    })
}

/// Handler for `chain_methodSchema`.
///
/// Takes `{"method": "chain_postInsight"}` and returns a small JSON-Schema-ish
/// document describing the request params and response shape. Returns
/// [`err_code::NOT_FOUND`] if `method` is not a known `chain_*` method.
///
/// # Errors
///
/// Returns `INVALID` if `method` is missing from `params`, or `NOT_FOUND` if
/// the method name is unknown.
pub fn handle_method_schema(params: JsonValue) -> Result<JsonValue, ErrorObjectOwned> {
    let method = params
        .get("method")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_err(err_code::INVALID, "missing field: method"))?;
    METHOD_SCHEMAS.get(method).map_or_else(
        || {
            Err(rpc_err(
                err_code::NOT_FOUND,
                format!("unknown chain method: {method}"),
            ))
        },
        |schema| Ok(schema.clone()),
    )
}

/// Lookup table of hand-written schemas for every `chain_*` method.
///
/// Keys match [`CHAIN_METHOD_NAMES`] one-to-one. Each entry records the
/// `method` name, a loose JSON-Schema fragment for params + result, and a
/// human-readable description. See the unit tests for coverage assertions.
static METHOD_SCHEMAS: LazyLock<HashMap<&'static str, JsonValue>> = LazyLock::new(|| {
    let mut m: HashMap<&'static str, JsonValue> = HashMap::new();

    m.insert(
        "chain_postInsight",
        json!({
            "method": "chain_postInsight",
            "params": {
                "type": "object",
                "required": ["author", "kind", "content"],
                "properties": {
                    "author": {"type": "string", "description": "Author id (utf-8 or hex)."},
                    "kind": {"type": "string", "enum": [
                        "insight", "heuristic", "warning",
                        "causal_link", "strategy_fragment", "anti_knowledge",
                    ]},
                    "content": {"type": "string"},
                    "enabledBy": {"type": "array", "items": {"type": "string"}},
                    "stakeWei": {"type": ["integer", "null"]},
                },
            },
            "result": {
                "type": "object",
                "properties": {
                    "outcome": {"type": "string", "enum": ["accepted", "duplicate", "exact_match"]},
                    "id": {"type": "string"},
                    "similarity": {"type": ["number", "null"]},
                },
            },
            "description": "Post a new insight entry to the knowledge layer.",
        }),
    );

    m.insert(
        "chain_searchInsights",
        json!({
            "method": "chain_searchInsights",
            "params": {
                "type": "object",
                "properties": {
                    "query": {"type": ["string", "null"]},
                    "queryVector": {"type": ["array", "null"], "items": {"type": "integer"}},
                    "k": {"type": "integer", "default": 10},
                    "kind": {"type": ["string", "null"]},
                },
            },
            "result": {
                "type": "object",
                "properties": {
                    "results": {"type": "array", "items": {"type": "object"}},
                },
            },
            "description": "Top-K HDC similarity search over the knowledge store.",
        }),
    );

    m.insert(
        "chain_confirmInsight",
        json!({
            "method": "chain_confirmInsight",
            "params": {
                "type": "object",
                "required": ["id", "confirmer"],
                "properties": {
                    "id": {"type": "string"},
                    "confirmer": {"type": "string"},
                },
            },
            "result": {
                "type": "object",
                "properties": {"ok": {"type": "boolean"}},
            },
            "description": "Record a confirmation for an insight.",
        }),
    );

    m.insert(
        "chain_challengeInsight",
        json!({
            "method": "chain_challengeInsight",
            "params": {
                "type": "object",
                "required": ["id", "challenger"],
                "properties": {
                    "id": {"type": "string"},
                    "challenger": {"type": "string"},
                },
            },
            "result": {
                "type": "object",
                "properties": {"ok": {"type": "boolean"}},
            },
            "description": "Open a challenge against an insight.",
        }),
    );

    m.insert(
        "chain_getInsight",
        json!({
            "method": "chain_getInsight",
            "params": {
                "type": "object",
                "required": ["id"],
                "properties": {"id": {"type": "string"}},
            },
            "result": {
                "type": ["object", "null"],
                "description": "InsightEntry projection or null if not found.",
            },
            "description": "Fetch a single insight entry by id.",
        }),
    );

    m.insert(
        "chain_applyDecay",
        json!({
            "method": "chain_applyDecay",
            "params": {
                "type": "object",
                "properties": {
                    "nowSecs": {"type": ["integer", "null"]},
                },
            },
            "result": {
                "type": "object",
                "properties": {
                    "total": {"type": "integer"},
                    "activeAfter": {"type": "integer"},
                    "prunedCount": {"type": "integer"},
                },
            },
            "description": "Run exponential decay + state-machine sweep across knowledge entries.",
        }),
    );

    m.insert(
        "chain_depositPheromone",
        json!({
            "method": "chain_depositPheromone",
            "params": {
                "type": "object",
                "required": ["kind", "content"],
                "properties": {
                    "kind": {"type": "string", "enum": ["threat", "opportunity", "wisdom"]},
                    "content": {"type": "string"},
                    "intensity": {"type": "number", "default": 1.0},
                    "halfLifeSeconds": {"type": ["integer", "null"]},
                },
            },
            "result": {
                "type": "object",
                "properties": {"id": {"type": "integer"}},
            },
            "description": "Deposit a pheromone signal with HDC content addressing.",
        }),
    );

    m.insert(
        "chain_queryPheromones",
        json!({
            "method": "chain_queryPheromones",
            "params": {
                "type": "object",
                "properties": {
                    "query": {"type": ["string", "null"]},
                    "queryVector": {"type": ["array", "null"], "items": {"type": "integer"}},
                    "k": {"type": "integer", "default": 10},
                },
            },
            "result": {
                "type": "object",
                "properties": {"results": {"type": "array", "items": {"type": "object"}}},
            },
            "description": "Top-K pheromone retrieval with decayed intensity.",
        }),
    );

    m.insert(
        "chain_stats",
        json!({
            "method": "chain_stats",
            "params": {"type": "object", "properties": {}},
            "result": {
                "type": "object",
                "properties": {
                    "insights": {"type": "integer"},
                    "pheromones": {"type": "integer"},
                    "toggles": {"type": "object"},
                },
            },
            "description": "Return counts of knowledge entries, pheromones, and toggle state.",
        }),
    );

    m.insert(
        "chain_version",
        json!({
            "method": "chain_version",
            "params": {"type": "object", "properties": {}},
            "result": {
                "type": "object",
                "properties": {
                    "rpcVersion": {"type": "string"},
                    "mirageVersion": {"type": "string"},
                    "schemaVersion": {"type": "integer"},
                    "supportedMethods": {"type": "array", "items": {"type": "string"}},
                    "features": {"type": "object"},
                },
            },
            "description": "Return RPC surface version, mirage version, and toggle flags.",
        }),
    );

    m.insert(
        "chain_listKinds",
        json!({
            "method": "chain_listKinds",
            "params": {"type": "object", "properties": {}},
            "result": {
                "type": "object",
                "properties": {
                    "knowledgeKinds": {"type": "array", "items": {"type": "string"}},
                    "pheromoneKinds": {"type": "array", "items": {"type": "string"}},
                    "insightStates": {"type": "array", "items": {"type": "string"}},
                },
            },
            "description": "Enumerate KnowledgeKind, PheromoneKind, and KnowledgeState variants.",
        }),
    );

    m.insert(
        "chain_methodSchema",
        json!({
            "method": "chain_methodSchema",
            "params": {
                "type": "object",
                "required": ["method"],
                "properties": {"method": {"type": "string"}},
            },
            "result": {
                "type": "object",
                "description": "JSON Schema fragment for the named method.",
            },
            "description": "Introspect the params/result schema of a chain_* method.",
        }),
    );

    m.insert(
        "chain_subscribePheromones",
        json!({
            "method": "chain_subscribePheromones",
            "params": {"type": "object", "properties": {}},
            "result": {
                "type": "object",
                "properties": {
                    "subscriptionId": {"type": "string"},
                },
            },
            "description": "WS-only: stream new pheromone deposits as they happen.",
        }),
    );

    m.insert(
        "chain_subscribeInsights",
        json!({
            "method": "chain_subscribeInsights",
            "params": {"type": "object", "properties": {}},
            "result": {
                "type": "object",
                "properties": {
                    "subscriptionId": {"type": "string"},
                },
            },
            "description": "WS-only: stream knowledge-layer lifecycle events (post, confirm, challenge, decay).",
        }),
    );

    m.insert(
        "chain_unsubscribe",
        json!({
            "method": "chain_unsubscribe",
            "params": {
                "type": "object",
                "required": ["subscriptionId"],
                "properties": {"subscriptionId": {"type": "string"}},
            },
            "result": {
                "type": "object",
                "properties": {"ok": {"type": "boolean"}},
            },
            "description": "Cancel a chain_subscribe* stream by id.",
        }),
    );

    m
});

// ─── Agent registry handlers ────────────────────────────────────────────────

/// Handle `chain_registerAgent(id, address_hex, role)`.
///
/// # Errors
///
/// Returns an `INVALID` JSON-RPC error if `address_hex` is not valid hex.
pub fn handle_register_agent(
    chain: &Arc<RwLock<ChainContext>>,
    id: String,
    address_hex: String,
    role: String,
) -> Result<bool, ErrorObjectOwned> {
    let address = alloy_primitives::hex::decode(address_hex.trim_start_matches("0x"))
        .map_err(|e| ErrorObjectOwned::owned(err_code::INVALID, e.to_string(), None::<()>))?;
    let mut chain = chain.write();
    let timestamp = now_seconds();
    let registered =
        chain
            .agent_registry
            .register(id.clone(), address, role.clone(), String::new(), timestamp);
    if registered {
        let _ = chain
            .agent_bus
            .send(crate::chain::AgentEvent::Registered { agent_id: id, role });
    }
    Ok(registered)
}

/// Handle `chain_agentHeartbeat(id)`.
pub fn handle_agent_heartbeat(
    chain: &Arc<RwLock<ChainContext>>,
    id: String,
    current_block: u64,
) -> bool {
    let mut chain = chain.write();
    let timestamp = now_seconds();
    let ok = chain
        .agent_registry
        .heartbeat(&id, current_block, timestamp);
    if ok {
        let _ = chain.agent_bus.send(crate::chain::AgentEvent::Heartbeat {
            agent_id: id,
            block: current_block,
            timestamp,
        });
    }
    ok
}

/// Handle `chain_agentTrace(id, phase, reads, reasoning, action)`.
///
/// # Errors
///
/// Returns an `INVALID` JSON-RPC error if `phase` is not one of `retrieve`,
/// `reason`, `act`, or `verify`.
pub fn handle_agent_trace(
    chain: &Arc<RwLock<ChainContext>>,
    id: String,
    phase: String,
    reads: Vec<String>,
    reasoning: String,
    action: String,
) -> Result<bool, ErrorObjectOwned> {
    let cognitive_phase = match phase.as_str() {
        "retrieve" => crate::chain::CognitivePhase::Retrieve,
        "reason" => crate::chain::CognitivePhase::Reason,
        "act" => crate::chain::CognitivePhase::Act,
        "verify" => crate::chain::CognitivePhase::Verify,
        _ => {
            return Err(ErrorObjectOwned::owned(
                err_code::INVALID,
                format!("invalid phase: {phase}; expected retrieve, reason, act, or verify"),
                None::<()>,
            ));
        }
    };
    let mut chain = chain.write();
    let timestamp = now_seconds();
    let trace = crate::chain::AgentTrace {
        cycle: 0,
        phase: cognitive_phase,
        reads,
        reasoning,
        action,
        action_id: format!("{id}-{timestamp}"),
        timestamp,
    };
    let _ = chain.agent_bus.send(crate::chain::AgentEvent::Trace {
        agent_id: id.clone(),
        trace: trace.clone(),
    });
    Ok(chain.agent_registry.add_trace(&id, trace))
}

/// Handle `chain_agentStats(id, stats_delta)`.
pub fn handle_agent_stats(
    chain: &Arc<RwLock<ChainContext>>,
    id: String,
    delta: crate::chain::AgentStats,
) -> bool {
    let mut chain = chain.write();
    let _ = chain.agent_bus.send(crate::chain::AgentEvent::Stats {
        agent_id: id.clone(),
        delta: delta.clone(),
    });
    chain.agent_registry.add_stats_delta(&id, &delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Arc<RwLock<ChainContext>> {
        Arc::new(RwLock::new(ChainContext::new(ChainToggles::all())))
    }

    #[test]
    fn post_insight_then_search_returns_match() {
        let c = ctx();
        let res = handle_post_insight(
            &c,
            PostInsightParams {
                author: "alice".into(),
                kind: "insight".into(),
                content: "uniswap v3 STF revert means insufficient allowance".into(),
                enabled_by: Vec::new(),
                stake_wei: None,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "accepted");

        let search = handle_search_insights(
            &c,
            SearchInsightsParams {
                query: Some("uniswap v3 STF revert means insufficient allowance".into()),
                query_vector: None,
                k: 5,
                kind: None,
            },
        )
        .unwrap();
        let results = search.get("results").and_then(|r| r.as_array()).unwrap();
        assert!(!results.is_empty());
        assert_eq!(
            results[0]["content"],
            "uniswap v3 STF revert means insufficient allowance"
        );
    }

    #[test]
    fn disabled_toggle_returns_error() {
        let c = Arc::new(RwLock::new(ChainContext::new(ChainToggles::default())));
        let err = handle_post_insight(
            &c,
            PostInsightParams {
                author: "a".into(),
                kind: "insight".into(),
                content: "x".into(),
                enabled_by: Vec::new(),
                stake_wei: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), err_code::DISABLED);
    }

    #[test]
    fn confirm_and_get_insight_roundtrip() {
        let c = ctx();
        let posted = handle_post_insight(
            &c,
            PostInsightParams {
                author: "a".into(),
                kind: "heuristic".into(),
                content: "set arbitrum gas 3x estimate".into(),
                enabled_by: Vec::new(),
                stake_wei: None,
            },
        )
        .unwrap();
        let id_str = format!("insight:{}", posted.id);
        let confirm_result = handle_confirm_insight(
            &c,
            ConfirmInsightParams {
                id: id_str.clone(),
                confirmer: "bob".into(),
            },
        )
        .unwrap();
        assert_eq!(confirm_result, json!({ "ok": true }));
        let fetched = handle_get_insight(&c, json!({ "id": id_str })).unwrap();
        assert_eq!(fetched["confirmations"], 1);
    }

    #[test]
    fn deposit_and_query_pheromone() {
        let c = ctx();
        let dep = handle_deposit_pheromone(
            &c,
            DepositPheromoneParams {
                kind: "threat".into(),
                content: "rug in pool X".into(),
                intensity: 1.0,
                half_life_seconds: None,
            },
        )
        .unwrap();
        assert!(dep.get("id").is_some());

        let q = handle_query_pheromones(&c, json!({ "query": "rug in pool X", "k": 3 })).unwrap();
        let results = q.get("results").and_then(|r| r.as_array()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["kind"], "threat");
    }

    #[test]
    fn parse_insight_id_accepts_both_prefixes() {
        let raw = "0011223344556677889900aabbccddeeff";
        // 34 chars with "insight:" prefix + 32 hex
        let prefixed = format!("insight:{}", &raw[..32]);
        assert!(parse_insight_id(&prefixed).is_ok());
        assert!(parse_insight_id(&raw[..32]).is_ok());
    }

    #[test]
    fn stats_reports_counts_and_toggles() {
        let c = ctx();
        handle_deposit_pheromone(
            &c,
            DepositPheromoneParams {
                kind: "opportunity".into(),
                content: "arb window".into(),
                intensity: 1.0,
                half_life_seconds: None,
            },
        )
        .unwrap();
        let s = handle_stats(&c);
        assert_eq!(s["pheromones"], 1);
        assert_eq!(s["toggles"]["stigmergy"], true);
    }

    #[test]
    fn version_reports_pkg_version_and_toggles() {
        let c = ctx();
        let v = handle_version(&c);
        assert_eq!(v["rpcVersion"], CHAIN_RPC_VERSION);
        assert_eq!(v["mirageVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["schemaVersion"], CHAIN_SCHEMA_VERSION);
        assert_eq!(v["features"]["hdc"], true);
        assert_eq!(v["features"]["knowledge"], true);
        assert_eq!(v["features"]["stigmergy"], true);
        let methods = v["supportedMethods"].as_array().expect("array");
        assert_eq!(methods.len(), CHAIN_METHOD_NAMES.len());
        assert!(methods.iter().any(|m| m == "chain_postInsight"));
        assert!(methods.iter().any(|m| m == "chain_version"));
        assert!(methods.iter().any(|m| m == "chain_listKinds"));
        assert!(methods.iter().any(|m| m == "chain_methodSchema"));

        // Toggle state flows through.
        let disabled_ctx = Arc::new(RwLock::new(ChainContext::new(ChainToggles::default())));
        let v2 = handle_version(&disabled_ctx);
        assert_eq!(v2["features"]["hdc"], false);
        assert_eq!(v2["features"]["knowledge"], false);
        assert_eq!(v2["features"]["stigmergy"], false);
    }

    #[test]
    fn list_kinds_returns_six_knowledge_kinds() {
        let v = handle_list_kinds();
        let kinds = v["knowledgeKinds"].as_array().expect("array");
        assert_eq!(kinds.len(), 6);
        for want in [
            "insight",
            "heuristic",
            "warning",
            "causal_link",
            "strategy_fragment",
            "anti_knowledge",
        ] {
            assert!(kinds.iter().any(|k| k == want), "missing kind: {want}");
        }
        let states = v["insightStates"].as_array().expect("array");
        assert_eq!(states.len(), 7);
    }

    #[test]
    fn list_kinds_returns_three_pheromone_kinds() {
        let v = handle_list_kinds();
        let kinds = v["pheromoneKinds"].as_array().expect("array");
        assert_eq!(kinds.len(), 3);
        assert!(kinds.iter().any(|k| k == "threat"));
        assert!(kinds.iter().any(|k| k == "opportunity"));
        assert!(kinds.iter().any(|k| k == "wisdom"));
    }

    #[test]
    fn method_schema_known_method_returns_schema() {
        let s =
            handle_method_schema(json!({ "method": "chain_postInsight" })).expect("known method");
        assert_eq!(s["method"], "chain_postInsight");
        assert_eq!(s["params"]["type"], "object");
        let required = s["params"]["required"].as_array().expect("required array");
        assert!(required.iter().any(|r| r == "author"));
        assert!(required.iter().any(|r| r == "kind"));
        assert!(required.iter().any(|r| r == "content"));
        assert_eq!(s["result"]["type"], "object");
        assert!(s["description"].is_string());
    }

    #[test]
    fn method_schema_unknown_method_returns_not_found() {
        let err = handle_method_schema(json!({ "method": "chain_nonexistent" }))
            .expect_err("should error");
        assert_eq!(err.code(), err_code::NOT_FOUND);

        // Missing field also fails (with INVALID, not NOT_FOUND).
        let err2 = handle_method_schema(json!({})).expect_err("should error");
        assert_eq!(err2.code(), err_code::INVALID);
    }

    #[test]
    fn method_schema_includes_all_methods() {
        assert_eq!(CHAIN_METHOD_NAMES.len(), 15);
        assert_eq!(METHOD_SCHEMAS.len(), 15);
        for name in CHAIN_METHOD_NAMES {
            let schema = handle_method_schema(json!({ "method": name }))
                .unwrap_or_else(|_| panic!("missing schema for {name}"));
            assert_eq!(schema["method"], *name, "method field must self-reference");
            assert!(
                schema["description"].is_string(),
                "missing description for {name}"
            );
            assert!(schema["params"].is_object(), "missing params for {name}");
            assert!(!schema["result"].is_null(), "missing result for {name}");
        }
    }

    #[cfg(feature = "roko")]
    mod subs {
        //! §38.d — live-stream subscription wiring tests.
        //!
        //! These tests exercise the bus broadcasts that fire from the RPC
        //! handler layer (`handle_post_insight`, `handle_confirm_insight`,
        //! `handle_challenge_insight`, `handle_deposit_pheromone`,
        //! `handle_apply_decay`) without starting an actual jsonrpsee server.
        //! A `VecSink` records every event so assertions can inspect exact
        //! payloads.

        use super::*;
        use crate::roko_bridge::{BackpressurePolicy, InsightEvent, PheromoneEvent, VecSink};

        fn ctx_with_subs() -> Arc<RwLock<ChainContext>> {
            Arc::new(RwLock::new(
                ChainContext::new_with_subs(ChainToggles::all()),
            ))
        }

        #[test]
        fn deposit_pheromone_broadcasts_to_bus() {
            let c = ctx_with_subs();
            let sink: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
            let bus = c.read().pheromone_bus.clone().expect("bus present");
            bus.register(sink.clone(), BackpressurePolicy::DropOldest);

            handle_deposit_pheromone(
                &c,
                DepositPheromoneParams {
                    kind: "threat".into(),
                    content: "rug in pool X".into(),
                    intensity: 0.8,
                    half_life_seconds: None,
                },
            )
            .unwrap();

            let events = sink.events();
            assert_eq!(events.len(), 1, "bus should have received one event");
            assert_eq!(events[0].kind, PheromoneKind::Threat);
            assert!((events[0].intensity - 0.8).abs() < 1e-6);
        }

        #[test]
        fn post_insight_broadcasts_posted_event() {
            let c = ctx_with_subs();
            let sink: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
            let bus = c.read().insight_bus.clone().expect("bus present");
            bus.register(sink.clone(), BackpressurePolicy::DropOldest);

            handle_post_insight(
                &c,
                PostInsightParams {
                    author: "alice".into(),
                    kind: "insight".into(),
                    content: "rebalance before 18:00 UTC".into(),
                    enabled_by: Vec::new(),
                    stake_wei: None,
                },
            )
            .unwrap();

            let events = sink.events();
            assert_eq!(events.len(), 1);
            match &events[0] {
                InsightEvent::Posted {
                    kind,
                    content,
                    author,
                    ..
                } => {
                    assert_eq!(*kind, KnowledgeKind::Insight);
                    assert_eq!(content, "rebalance before 18:00 UTC");
                    assert_eq!(author, b"alice");
                }
                other => panic!("expected Posted, got {other:?}"),
            }
        }

        #[test]
        fn confirm_insight_broadcasts_confirmed_event() {
            let c = ctx_with_subs();
            let posted = handle_post_insight(
                &c,
                PostInsightParams {
                    author: "alice".into(),
                    kind: "heuristic".into(),
                    content: "use 3x gas on arbitrum".into(),
                    enabled_by: Vec::new(),
                    stake_wei: None,
                },
            )
            .unwrap();

            let sink: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
            let bus = c.read().insight_bus.clone().expect("bus present");
            bus.register(sink.clone(), BackpressurePolicy::DropOldest);

            handle_confirm_insight(
                &c,
                ConfirmInsightParams {
                    id: format!("insight:{}", posted.id),
                    confirmer: "bob".into(),
                },
            )
            .unwrap();

            let events = sink.events();
            assert_eq!(events.len(), 1);
            match &events[0] {
                InsightEvent::Confirmed { by, .. } => assert_eq!(by, b"bob"),
                other => panic!("expected Confirmed, got {other:?}"),
            }
        }

        #[test]
        fn challenge_insight_broadcasts_challenged_event() {
            let c = ctx_with_subs();
            let posted = handle_post_insight(
                &c,
                PostInsightParams {
                    author: "alice".into(),
                    kind: "warning".into(),
                    content: "watch for oracle manipulation".into(),
                    enabled_by: Vec::new(),
                    stake_wei: None,
                },
            )
            .unwrap();

            let sink: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
            let bus = c.read().insight_bus.clone().expect("bus present");
            bus.register(sink.clone(), BackpressurePolicy::DropOldest);

            handle_challenge_insight(
                &c,
                ChallengeInsightParams {
                    id: format!("insight:{}", posted.id),
                    challenger: "eve".into(),
                },
            )
            .unwrap();

            let events = sink.events();
            assert_eq!(events.len(), 1);
            match &events[0] {
                InsightEvent::Challenged { by, .. } => assert_eq!(by, b"eve"),
                other => panic!("expected Challenged, got {other:?}"),
            }
        }

        #[test]
        fn apply_decay_broadcasts_decayed_events() {
            let c = ctx_with_subs();
            handle_post_insight(
                &c,
                PostInsightParams {
                    author: "a".into(),
                    kind: "insight".into(),
                    content: "alpha 1".into(),
                    enabled_by: Vec::new(),
                    stake_wei: None,
                },
            )
            .unwrap();
            handle_post_insight(
                &c,
                PostInsightParams {
                    author: "b".into(),
                    kind: "insight".into(),
                    content: "alpha 2".into(),
                    enabled_by: Vec::new(),
                    stake_wei: None,
                },
            )
            .unwrap();

            let sink: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
            let bus = c.read().insight_bus.clone().expect("bus present");
            bus.register(sink.clone(), BackpressurePolicy::DropOldest);

            handle_apply_decay(&c, json!({"nowSecs": 1})).unwrap();
            let events = sink.events();
            assert!(
                events
                    .iter()
                    .all(|e| matches!(e, InsightEvent::Decayed { .. })),
                "expected only Decayed events, got {events:?}"
            );
            assert!(events.len() >= 2, "expected one event per live entry");
        }

        #[test]
        fn subscription_manager_register_unregister_pheromone() {
            let manager = SubscriptionManager::with_fresh_buses();
            let sink: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
            let id = manager.register_pheromone_sink(sink, BackpressurePolicy::DropOldest);
            assert!(id.starts_with(PHEROMONE_SUB_PREFIX));
            assert_eq!(manager.pheromones().len(), 1);

            assert!(manager.unsubscribe(&id));
            assert_eq!(manager.pheromones().len(), 0);
            // second unsubscribe is a no-op
            assert!(!manager.unsubscribe(&id));
        }

        #[test]
        fn subscription_manager_register_unregister_insight() {
            let manager = SubscriptionManager::with_fresh_buses();
            let sink: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
            let id = manager.register_insight_sink(sink, BackpressurePolicy::DropOldest);
            assert!(id.starts_with(INSIGHT_SUB_PREFIX));
            assert_eq!(manager.insights().len(), 1);

            assert!(manager.unsubscribe(&id));
            assert_eq!(manager.insights().len(), 0);
        }

        #[test]
        fn handle_unsubscribe_rejects_unknown_id() {
            let manager = SubscriptionManager::with_fresh_buses();
            let err = handle_unsubscribe(&manager, json!({"subscriptionId": "bogus"}))
                .expect_err("should error");
            assert_eq!(err.code(), err_code::NOT_FOUND);

            let err2 =
                handle_unsubscribe(&manager, json!({})).expect_err("should error on missing field");
            assert_eq!(err2.code(), err_code::INVALID);
        }

        #[test]
        fn handle_unsubscribe_routes_by_prefix() {
            let manager = SubscriptionManager::with_fresh_buses();
            let p_sink: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
            let i_sink: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
            let p_id = manager.register_pheromone_sink(p_sink, BackpressurePolicy::DropOldest);
            let i_id = manager.register_insight_sink(i_sink, BackpressurePolicy::DropOldest);

            let ok = handle_unsubscribe(&manager, json!({"subscriptionId": p_id})).unwrap();
            assert_eq!(ok, json!({"ok": true}));
            assert_eq!(manager.pheromones().len(), 0);
            assert_eq!(manager.insights().len(), 1);

            handle_unsubscribe(&manager, json!({"subscriptionId": i_id})).unwrap();
            assert_eq!(manager.insights().len(), 0);
        }

        #[test]
        fn stats_reports_subscription_counts() {
            let c = ctx_with_subs();
            let bus_p = c.read().pheromone_bus.clone().unwrap();
            let bus_i = c.read().insight_bus.clone().unwrap();
            let ps: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
            let is_: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
            bus_p.register(ps, BackpressurePolicy::DropOldest);
            bus_i.register(is_, BackpressurePolicy::DropOldest);

            let s = handle_stats(&c);
            let subs = &s["subscriptions"];
            assert_eq!(subs["pheromoneCount"], 1);
            assert_eq!(subs["insightCount"], 1);
            assert_eq!(subs["droppedOldest"], 0);
            assert_eq!(subs["droppedNewest"], 0);
            assert_eq!(subs["closedByOverflow"], 0);
        }

        #[test]
        fn pheromone_event_json_contains_expected_fields() {
            let event = crate::roko_bridge::PheromoneEvent::new(
                7,
                PheromoneKind::Opportunity,
                roko_primitives::HdcVector::from_seed(b"q"),
                0.42,
                1_700_000_000,
            );
            let v = pheromone_event_to_json(&event);
            assert_eq!(v["id"], 7);
            assert_eq!(v["kind"], "opportunity");
            assert!((v["intensity"].as_f64().unwrap() - 0.42).abs() < 1e-5);
            assert_eq!(v["depositedAt"], 1_700_000_000u64);
        }

        #[test]
        fn insight_event_json_handles_every_variant() {
            let id = InsightId([9; 16]);
            let posted = InsightEvent::Posted {
                id,
                kind: KnowledgeKind::Heuristic,
                content: "c".into(),
                author: b"a".to_vec(),
                created_at: 10,
            };
            assert_eq!(insight_event_to_json(&posted)["type"], "posted");

            let decayed = InsightEvent::Decayed {
                id,
                new_weight: 0.3,
                at: 11,
            };
            let v = insight_event_to_json(&decayed);
            assert_eq!(v["type"], "decayed");
            assert!((v["newWeight"].as_f64().unwrap() - 0.3).abs() < 1e-5);
        }

        #[test]
        fn set_buses_retrofits_existing_context() {
            let mut ctx = ChainContext::new(ChainToggles::all());
            assert!(ctx.pheromone_bus.is_none());
            assert!(ctx.insight_bus.is_none());
            ctx.set_buses(Arc::new(PheromoneBus::new()), Arc::new(InsightBus::new()));
            assert!(ctx.pheromone_bus.is_some());
            assert!(ctx.insight_bus.is_some());
        }

        #[test]
        fn deposit_without_bus_does_not_panic() {
            let c = Arc::new(RwLock::new(ChainContext::new(ChainToggles::all())));
            // No buses attached — should still succeed.
            handle_deposit_pheromone(
                &c,
                DepositPheromoneParams {
                    kind: "wisdom".into(),
                    content: "z".into(),
                    intensity: 1.0,
                    half_life_seconds: None,
                },
            )
            .unwrap();
        }
    }
}
