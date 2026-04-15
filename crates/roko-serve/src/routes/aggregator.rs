//! Mirage-compatible aggregation routes backed by discovered agent servers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use futures::future::join_all;
use futures::{SinkExt, StreamExt};
use roko_agent_server::registration::AgentCard;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::{AppState, DiscoveredAgent};

const AGENT_LIST_TTL: Duration = Duration::from_secs(30);
const AGENT_STATS_TTL: Duration = Duration::from_secs(5);
const PREDICTIONS_TTL: Duration = Duration::from_secs(10);
const KNOWLEDGE_TTL: Duration = Duration::from_secs(30);
const TASKS_TTL: Duration = Duration::from_secs(30);

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents))
        .route("/agents/topology", get(agent_topology))
        .route("/agents/{id}/stats", get(agent_stats))
        .route("/predictions/sessions", get(list_prediction_sessions))
        .route("/predictions/claims", get(list_prediction_claims))
        .route("/predictions/calibration/{agent_id}", get(prediction_calibration))
        .route("/knowledge/entries", get(list_knowledge_entries))
        .route("/knowledge/edges", get(list_knowledge_edges))
        .route("/knowledge/search", get(search_knowledge))
        .route("/knowledge/kinds", get(list_knowledge_kinds))
        .route("/tasks", get(list_tasks))
        .route("/tasks/stats", get(task_stats))
        .route("/tasks/{id}", get(get_task))
        .route("/ws", get(ws_upgrade))
}

#[derive(Debug, Clone, Serialize)]
struct PaginatedResponse<T: Serialize> {
    items: Vec<T>,
    total: usize,
    offset: usize,
    limit: usize,
    has_more: bool,
}

impl<T: Serialize> PaginatedResponse<T> {
    fn new(items: Vec<T>, total: usize, offset: usize, limit: usize) -> Self {
        Self {
            has_more: offset + items.len() < total,
            items,
            total,
            offset,
            limit,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct AgentListQuery {
    #[serde(default)]
    owner: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct TaskListQuery {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
}

fn default_limit() -> usize {
    20
}

#[derive(Serialize)]
struct AgentNode {
    id: String,
    address: String,
    insights_posted: usize,
    confirmations_given: usize,
    challenges_given: usize,
    total_weight: f64,
}

#[derive(Serialize)]
struct AgentEdge {
    from: String,
    to: String,
    weight: usize,
    #[serde(rename = "type")]
    edge_type: String,
}

#[derive(Serialize)]
struct TopologyResponse {
    nodes: Vec<AgentNode>,
    edges: Vec<AgentEdge>,
    timestamp: u64,
}

async fn list_agents(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AgentListQuery>,
) -> Result<Json<Value>, ApiError> {
    let cache_key = format!("aggregator:agents:{}", query.owner.clone().unwrap_or_default());
    if let Some(cached) = state.cached_json(&cache_key).await {
        return Ok(Json(cached));
    }

    let agents = known_agents(&state).await?;
    let futures = agents.iter().cloned().map({
        let state = Arc::clone(&state);
        move |agent| {
            let state = Arc::clone(&state);
            async move {
                let health = fetch_agent_json(&state, &agent, "health").await;
                let capabilities = fetch_agent_json(&state, &agent, "capabilities").await;
                (agent, health, capabilities)
            }
        }
    });
    let rows = join_all(futures).await;

    let items: Vec<Value> = rows
        .into_iter()
        .filter(|(agent, _, _)| {
            query
                .owner
                .as_deref()
                .is_none_or(|owner| agent.owner.is_empty() || agent.owner == owner)
        })
        .map(|(agent, health, capabilities)| {
            let stats = capabilities
                .as_ref()
                .and_then(|capabilities| capabilities.get("stats"))
                .cloned()
                .unwrap_or_else(default_agent_stats);
            let skills = capabilities
                .as_ref()
                .and_then(|capabilities| capabilities.get("skills"))
                .cloned()
                .unwrap_or_else(|| features_to_skills(&agent.capabilities));
            let role = agent
                .capabilities
                .first()
                .cloned()
                .unwrap_or_else(|| "agent".to_string());
            json!({
                "id": agent.agent_id,
                "role": role,
                "owner": agent.owner,
                "registered_at": agent.registered_at,
                "last_heartbeat_block": 0,
                "last_heartbeat_ts": health
                    .as_ref()
                    .and_then(|payload| payload.get("uptime_s"))
                    .and_then(Value::as_u64)
                    .map_or(agent.last_seen_at, |uptime| now_secs().saturating_sub(uptime)),
                "stats": stats,
                "skills": skills,
            })
        })
        .collect();

    let total = items.len();
    let body = json!(PaginatedResponse::new(items, total, 0, total));
    state
        .put_cached_json(cache_key, AGENT_LIST_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn agent_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let cache_key = format!("aggregator:agent-stats:{id}");
    if let Some(cached) = state.cached_json(&cache_key).await {
        return Ok(Json(cached));
    }

    let agent = known_agents(&state)
        .await?
        .into_iter()
        .find(|agent| agent.agent_id == id)
        .ok_or_else(|| ApiError::not_found(format!("agent {id} not found")))?;

    let payload = fetch_agent_json(&state, &agent, "stats")
        .await
        .unwrap_or_else(|| {
            json!({
                "agent_id": id,
                "owner": agent.owner,
                "confirmations_given": 0,
                "challenges_given": 0,
                "warnings_posted": 0,
                "insights_posted": 0,
                "delta_cycles": 0,
                "total_cost_usd": 0.0,
                "total_tokens": 0,
                "registered_at": agent.registered_at,
                "operating_frequency": "idle",
            })
        });

    state
        .put_cached_json(cache_key, AGENT_STATS_TTL, payload.clone())
        .await;
    Ok(Json(payload))
}

async fn agent_topology(State(state): State<Arc<AppState>>) -> Result<Json<TopologyResponse>, ApiError> {
    let agents = known_agents(&state).await?;
    let nodes = agents
        .iter()
        .map(|agent| AgentNode {
            id: agent.agent_id.clone(),
            address: agent
                .endpoints
                .rest
                .clone()
                .unwrap_or_else(|| agent.card_uri.clone().unwrap_or_default()),
            insights_posted: 0,
            confirmations_given: 0,
            challenges_given: 0,
            total_weight: 0.0,
        })
        .collect();

    Ok(Json(TopologyResponse {
        nodes,
        edges: Vec::new(),
        timestamp: now_secs(),
    }))
}

async fn list_prediction_sessions(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let cache_key = "aggregator:prediction-sessions";
    if let Some(cached) = state.cached_json(cache_key).await {
        return Ok(Json(cached));
    }

    let predictions = collect_agent_predictions(&state).await?;
    let items: Vec<Value> = predictions
        .into_iter()
        .map(|prediction| {
            json!({
                "id": prediction["id"],
                "question": prediction["market"],
                "creator": prediction["agent_id"],
                "staked_points": 0,
                "target_block": 0,
                "category": prediction["category"].clone(),
                "context": "",
                "metric": "value",
                "state": if prediction.get("actual_value").and_then(Value::as_f64).is_some() { "resolved" } else { "registered" },
            })
        })
        .collect();
    let total = items.len();
    let body = json!(PaginatedResponse::new(items, total, 0, total));
    state
        .put_cached_json(cache_key, PREDICTIONS_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn list_prediction_claims(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let cache_key = "aggregator:prediction-claims";
    if let Some(cached) = state.cached_json(cache_key).await {
        return Ok(Json(cached));
    }

    let predictions = collect_agent_predictions(&state).await?;
    let items: Vec<Value> = predictions
        .into_iter()
        .map(|prediction| {
            json!({
                "id": prediction["id"],
                "session_id": prediction["id"],
                "agent_id": prediction["agent_id"],
                "predicted_value": prediction["predicted_value"],
                "interval_width": prediction["interval_width"],
                "confidence": prediction["confidence"],
                "entries_in_context": [],
                "registered_block": 0,
                "created_at": prediction["ts"],
            })
        })
        .collect();
    let total = items.len();
    let body = json!(PaginatedResponse::new(items, total, 0, total));
    state
        .put_cached_json(cache_key, PREDICTIONS_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn prediction_calibration(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let agent = known_agents(&state)
        .await?
        .into_iter()
        .find(|agent| agent.agent_id == agent_id)
        .ok_or_else(|| ApiError::not_found(format!("agent {agent_id} not found")))?;

    let calibration = fetch_agent_json(&state, &agent, "predictions/residuals")
        .await
        .unwrap_or_else(|| json!({"mse": 0.0, "hit_rate": 0.0, "residuals": []}));
    Ok(Json(calibration))
}

async fn list_knowledge_entries(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let cache_key = "aggregator:knowledge:entries";
    if let Some(cached) = state.cached_json(cache_key).await {
        return Ok(Json(cached));
    }
    let body = json!(PaginatedResponse::<Value>::new(Vec::new(), 0, 0, 0));
    state
        .put_cached_json(cache_key, KNOWLEDGE_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn list_knowledge_edges(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let cache_key = "aggregator:knowledge:edges";
    if let Some(cached) = state.cached_json(cache_key).await {
        return Ok(Json(cached));
    }
    let body = json!(PaginatedResponse::<Value>::new(Vec::new(), 0, 0, 0));
    state
        .put_cached_json(cache_key, KNOWLEDGE_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn search_knowledge(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Json<Value> {
    Json(json!({
        "results": [],
        "query": query.q,
        "timestamp": now_secs(),
    }))
}

async fn list_knowledge_kinds(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let cache_key = "aggregator:knowledge:kinds";
    if let Some(cached) = state.cached_json(cache_key).await {
        return Ok(Json(cached));
    }
    let body = json!({
        "knowledge_kinds": [
            { "name": "insight", "default_half_life_seconds": 2_592_000_u64, "base_reward_wei": "0", "count": 0 },
            { "name": "heuristic", "default_half_life_seconds": 7_776_000_u64, "base_reward_wei": "0", "count": 0 },
            { "name": "warning", "default_half_life_seconds": 604_800_u64, "base_reward_wei": "0", "count": 0 },
            { "name": "causal_link", "default_half_life_seconds": 5_184_000_u64, "base_reward_wei": "0", "count": 0 },
            { "name": "strategy_fragment", "default_half_life_seconds": 1_209_600_u64, "base_reward_wei": "0", "count": 0 },
            { "name": "anti_knowledge", "default_half_life_seconds": 2_592_000_u64, "base_reward_wei": "0", "count": 0 }
        ],
        "pheromone_kinds": [
            { "name": "threat", "default_half_life_seconds": 3_600_u64, "count": 0 },
            { "name": "opportunity", "default_half_life_seconds": 3_600_u64, "count": 0 },
            { "name": "wisdom", "default_half_life_seconds": 3_600_u64, "count": 0 }
        ],
        "timestamp": now_secs(),
    });
    state
        .put_cached_json(cache_key, KNOWLEDGE_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TaskListQuery>,
) -> Result<Json<Value>, ApiError> {
    let cache_key = format!(
        "aggregator:tasks:{}:{}:{}:{}:{}",
        query.state.clone().unwrap_or_default(),
        query.kind.clone().unwrap_or_default(),
        query.assignee.clone().unwrap_or_default(),
        query.limit,
        query.offset
    );
    if let Some(cached) = state.cached_json(&cache_key).await {
        return Ok(Json(cached));
    }

    let tasks = collect_agent_tasks(&state).await?;
    let filtered: Vec<Value> = tasks
        .into_iter()
        .filter(|task| {
            query
                .state
                .as_deref()
                .is_none_or(|state_filter| task.get("state").and_then(Value::as_str) == Some(state_filter))
        })
        .filter(|task| {
            query
                .kind
                .as_deref()
                .is_none_or(|kind| task.get("kind").and_then(Value::as_str) == Some(kind))
        })
        .filter(|task| {
            query.assignee.as_deref().is_none_or(|assignee| {
                task.get("assignee").and_then(Value::as_str) == Some(assignee)
            })
        })
        .collect();

    let total = filtered.len();
    let items = filtered
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect::<Vec<_>>();
    let body = json!(PaginatedResponse::new(items, total, query.offset, query.limit));
    state
        .put_cached_json(cache_key, TASKS_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn task_stats(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let cache_key = "aggregator:tasks:stats";
    if let Some(cached) = state.cached_json(cache_key).await {
        return Ok(Json(cached));
    }

    let tasks = collect_agent_tasks(&state).await?;
    let mut counts = HashMap::<String, usize>::new();
    for task in &tasks {
        if let Some(state) = task.get("state").and_then(Value::as_str) {
            *counts.entry(state.to_string()).or_default() += 1;
        }
    }
    let body = json!({
        "open": counts.get("open").copied().unwrap_or(0),
        "assigned": counts.get("assigned").copied().unwrap_or(0),
        "in_progress": counts.get("in_progress").copied().unwrap_or(0),
        "completed": counts.get("completed").copied().unwrap_or_else(|| counts.get("accepted").copied().unwrap_or(0)),
        "failed": counts.get("failed").copied().unwrap_or(0),
        "cancelled": counts.get("cancelled").copied().unwrap_or(0),
        "total_stake_wei": 0,
        "total_reward_wei": 0,
    });
    state
        .put_cached_json(cache_key, TASKS_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let tasks = collect_agent_tasks(&state).await?;
    let matched = tasks.into_iter().find(|task| {
        task.get("id")
            .map(|value| value == &Value::String(id.clone()) || value.as_u64().is_some_and(|value| value.to_string() == id))
            .unwrap_or(false)
    });

    matched
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("task {id} not found")))
}

async fn ws_upgrade(State(state): State<Arc<AppState>>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();
    let backlog = state.event_bus.replay_from(0);
    for envelope in &backlog {
        if let Ok(payload) = serde_json::to_string(&json!({
            "source": "roko-serve",
            "event": &envelope.payload,
        })) {
            if sink.send(Message::Text(payload.into())).await.is_err() {
                return;
            }
        }
    }

    let mut rx = state.event_bus.subscribe();
    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(envelope) => {
                        if let Ok(payload) = serde_json::to_string(&json!({
                            "source": "roko-serve",
                            "event": envelope.payload,
                        })) {
                            if sink.send(Message::Text(payload.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

async fn collect_agent_predictions(state: &Arc<AppState>) -> Result<Vec<Value>, ApiError> {
    let agents = known_agents(state).await?;
    let futures = agents.into_iter().map(|agent| async move {
        fetch_agent_json(state, &agent, "predictions")
            .await
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
    });
    Ok(join_all(futures).await.into_iter().flatten().collect())
}

async fn collect_agent_tasks(state: &Arc<AppState>) -> Result<Vec<Value>, ApiError> {
    let agents = known_agents(state).await?;
    let futures = agents.into_iter().map(|agent| async move {
        fetch_agent_json(state, &agent, "tasks")
            .await
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
    });
    Ok(join_all(futures).await.into_iter().flatten().collect())
}

async fn known_agents(state: &Arc<AppState>) -> Result<Vec<DiscoveredAgent>, ApiError> {
    let mut agents = state.list_discovered_agents().await;
    let mut known_ids: HashSet<String> = agents.iter().map(|agent| agent.agent_id.clone()).collect();

    for (pid, label) in state.supervisor.list().await {
        if known_ids.contains(&label) {
            continue;
        }
        known_ids.insert(label.clone());
        agents.push(DiscoveredAgent {
            agent_id: label.clone(),
            label: Some(label),
            process_id: Some(pid.0),
            owner: String::new(),
            registered_at: now_secs(),
            last_seen_at: now_secs(),
            endpoints: Default::default(),
            card_uri: None,
            capabilities: Vec::new(),
            domain_tags: Vec::new(),
            token_hash: None,
            token_expires_at: None,
            status: "managed".to_string(),
            proxy_token: None,
        });
    }

    let hydrated = join_all(agents.into_iter().map(|agent| async move {
        hydrate_agent_card(state, agent).await
    }))
    .await;

    Ok(hydrated)
}

async fn hydrate_agent_card(state: &Arc<AppState>, mut agent: DiscoveredAgent) -> DiscoveredAgent {
    if agent.endpoints.rest.is_some() && !agent.capabilities.is_empty() {
        return agent;
    }

    let Some(card_uri) = agent.card_uri.clone() else {
        return agent;
    };
    let card = if let Some(payload) = card_uri.strip_prefix("data:application/json;base64,") {
        base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(payload)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AgentCard>(&bytes).ok())
    } else if card_uri.starts_with("http://") || card_uri.starts_with("https://") {
        match state.http_client.get(&card_uri).send().await {
            Ok(response) => response.json::<AgentCard>().await.ok(),
            Err(_) => None,
        }
    } else {
        None
    };

    if let Some(card) = card {
        agent.capabilities = card.capabilities;
        agent.domain_tags = card.domain_tags;
        agent.endpoints.rest = card.endpoints.rest;
        agent.endpoints.websocket = card.endpoints.websocket;
        agent.endpoints.a2a = card.endpoints.a2a;
        agent.endpoints.mcp = card.endpoints.mcp;
        state.store_discovered_agent(agent.clone()).await;
    }
    agent
}

async fn fetch_agent_json(
    state: &Arc<AppState>,
    agent: &DiscoveredAgent,
    path: &str,
) -> Option<Value> {
    let rest = agent.endpoints.rest.as_ref()?;
    let url = format!("{}/{}", rest.trim_end_matches('/'), path.trim_start_matches('/'));
    let mut request = state.http_client.get(url);
    if let Some(token) = agent.proxy_token.as_ref() {
        request = request.bearer_auth(token);
    }
    request
        .send()
        .await
        .ok()?
        .json::<Value>()
        .await
        .ok()
}

fn features_to_skills(features: &[String]) -> Value {
    let map = features
        .iter()
        .map(|feature| {
            (
                feature.clone(),
                json!({
                    "enabled": true,
                    "config": {},
                }),
            )
        })
        .collect::<serde_json::Map<String, Value>>();
    Value::Object(map)
}

fn default_agent_stats() -> Value {
    json!({
        "confirmations_given": 0,
        "challenges_given": 0,
        "warnings_posted": 0,
        "insights_posted": 0,
        "delta_cycles": 0,
        "total_cost_usd": 0.0,
        "total_tokens": 0,
        "operating_frequency": "idle",
    })
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn features_to_skills_marks_features_enabled() {
        let value = features_to_skills(&["research".to_string(), "tasks".to_string()]);
        assert_eq!(value["research"]["enabled"], Value::Bool(true));
        assert_eq!(value["tasks"]["enabled"], Value::Bool(true));
    }

    #[tokio::test]
    async fn pagination_tracks_has_more() {
        let page = PaginatedResponse::new(vec![1, 2], 3, 0, 2);
        assert!(page.has_more);
    }
}
