//! Mirage-compatible aggregation routes backed by discovered agent servers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use futures::future::join_all;
use futures::{SinkExt, StreamExt};
use roko_agent_server::registration::AgentCard;
// AgentTopology/AgentTopologyNode no longer used — topology handler returns
// raw JSON matching the frontend shape.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tracing::warn;

use crate::error::ApiError;
use crate::state::{AppState, DiscoveredAgent};

const AGENT_LIST_TTL: Duration = Duration::from_secs(30);
const AGENT_STATS_TTL: Duration = Duration::from_secs(5);
const PREDICTIONS_TTL: Duration = Duration::from_secs(10);
const KNOWLEDGE_TTL: Duration = Duration::from_secs(30);
const TASKS_TTL: Duration = Duration::from_secs(30);
const STREAM_DISCOVERY_REFRESH: Duration = Duration::from_secs(10);
const STREAM_RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents))
        .route("/agents/topology", get(agent_topology))
        .route("/agents/{id}/stats", get(agent_stats))
        .route("/agents/{id}/skills", get(agent_skills))
        .route("/agents/{id}/heartbeat", get(agent_heartbeat))
        .route("/agents/{id}/trace", get(agent_trace))
        .route("/predictions/sessions", get(list_prediction_sessions))
        .route("/predictions/sessions/{id}", get(get_prediction_session))
        .route("/predictions/claims", get(list_prediction_claims))
        .route(
            "/predictions/calibration/{agent_id}",
            get(prediction_calibration),
        )
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

#[derive(Debug, Deserialize)]
struct TraceQuery {
    #[serde(default = "default_trace_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
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

#[derive(Debug, Deserialize)]
struct StreamCommand {
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    targets: Vec<String>,
    #[serde(default)]
    message: Option<Value>,
}

#[derive(Debug)]
struct MuxEnvelope {
    source: String,
    event: Value,
}

struct AgentStreamHandle {
    sender: mpsc::UnboundedSender<String>,
    task: JoinHandle<()>,
}

fn default_limit() -> usize {
    20
}

fn default_trace_limit() -> usize {
    10
}

async fn list_agents(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AgentListQuery>,
) -> Result<Json<Value>, ApiError> {
    let cache_key = format!(
        "aggregator:agents:{}",
        query.owner.clone().unwrap_or_default()
    );
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
                .is_none_or(|owner| agent.owner == owner)
        })
        .map(|(agent, health, capabilities)| {
            let stats = capabilities
                .as_ref()
                .and_then(|payload| payload.get("stats"))
                .cloned()
                .unwrap_or_else(default_agent_stats);
            let skills = capabilities
                .as_ref()
                .and_then(|payload| payload.get("skills"))
                .cloned()
                .unwrap_or_else(|| features_to_skills(&agent.capabilities));
            let role = agent_role(&agent, capabilities.as_ref());
            let last_heartbeat_ts = health
                .as_ref()
                .and_then(last_seen_from_health)
                .unwrap_or(agent.last_seen_at);
            json!({
                "id": agent.agent_id,
                "role": role,
                "owner": agent.owner,
                "registered_at": agent.registered_at,
                "last_heartbeat_block": 0,
                "last_heartbeat_ts": last_heartbeat_ts,
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

    let agent = find_known_agent(&state, &id).await?;
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

async fn agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let agent = find_known_agent(&state, &id).await?;
    let capabilities = fetch_agent_json(&state, &agent, "capabilities").await;
    let skills = capabilities
        .as_ref()
        .and_then(|payload| payload.get("skills"))
        .cloned()
        .unwrap_or_else(|| features_to_skills(&agent.capabilities));

    Ok(Json(json!({
        "agent_id": id,
        "skills": skills,
    })))
}

async fn agent_heartbeat(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let agent = find_known_agent(&state, &id).await?;
    let health = fetch_agent_json(&state, &agent, "health").await;
    let tasks = fetch_agent_json(&state, &agent, "tasks").await;
    let last_timestamp = health
        .as_ref()
        .and_then(last_seen_from_health)
        .unwrap_or(agent.last_seen_at);
    let busy = tasks
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|task| {
                task.get("state")
                    .and_then(Value::as_str)
                    .is_some_and(|state| state == "accepted" || state == "in_progress")
            })
        });

    Ok(Json(json!({
        "agent_id": id,
        "alive": health
            .as_ref()
            .and_then(|payload| payload.get("status"))
            .and_then(Value::as_str)
            == Some("ok"),
        "busy": busy,
        "last_block": 0,
        "last_timestamp": last_timestamp,
        "blocks_since": 0,
        "timeout_blocks": 0,
    })))
}

async fn agent_trace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<TraceQuery>,
) -> Result<Json<Value>, ApiError> {
    let _ = find_known_agent(&state, &id).await?;
    let limit = query.limit.min(1000);
    Ok(Json(json!({
        "agent_id": id,
        "items": [],
        "total": 0,
        "offset": query.offset,
        "limit": limit,
        "has_more": false,
    })))
}

async fn agent_topology(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let agents = known_agents(&state).await?;

    // Build nodes with the fields the frontend `AgentFleet` component expects:
    //   { agent_id, role, endpoints }
    let nodes: Vec<Value> = agents
        .iter()
        .map(|agent| {
            let role = agent
                .capabilities
                .first()
                .cloned()
                .unwrap_or_else(|| "agent".to_string());
            let endpoint = agent
                .endpoints
                .rest
                .clone()
                .unwrap_or_else(|| agent.card_uri.clone().unwrap_or_default());
            json!({
                "agent_id": agent.agent_id,
                "role": role,
                "endpoints": [endpoint],
            })
        })
        .collect();

    // Compute edges: agents that share domain tags are connected.
    let mut edges: Vec<Value> = Vec::new();
    for (i, a) in agents.iter().enumerate() {
        for b in agents.iter().skip(i + 1) {
            let shared = a
                .domain_tags
                .iter()
                .filter(|tag| b.domain_tags.contains(tag))
                .count();
            if shared > 0 || a.domain_tags.is_empty() || b.domain_tags.is_empty() {
                // Connect agents that share domain context, or all agents if
                // domain tags are absent (sparse graphs are boring).
                edges.push(json!({
                    "from": a.agent_id,
                    "to": b.agent_id,
                    "weight": shared.max(1),
                }));
            }
        }
    }

    Ok(Json(json!({
        "nodes": nodes,
        "edges": edges,
    })))
}

async fn list_prediction_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let cache_key = "aggregator:prediction-sessions";
    if let Some(cached) = state.cached_json(cache_key).await {
        return Ok(Json(cached));
    }

    let predictions = collect_agent_predictions(&state).await?;
    let items: Vec<Value> = predictions.iter().map(prediction_to_session).collect();
    let total = items.len();
    let body = json!(PaginatedResponse::new(items, total, 0, total));
    state
        .put_cached_json(cache_key, PREDICTIONS_TTL, body.clone())
        .await;
    Ok(Json(body))
}

async fn get_prediction_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let prediction = collect_agent_predictions(&state)
        .await?
        .into_iter()
        .find(|prediction| value_matches_id(prediction.get("id"), &id))
        .ok_or_else(|| ApiError::not_found(format!("prediction session {id} not found")))?;

    Ok(Json(json!({
        "session": prediction_to_session(&prediction),
        "claims": [prediction_to_claim(&prediction)],
    })))
}

async fn list_prediction_claims(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let cache_key = "aggregator:prediction-claims";
    if let Some(cached) = state.cached_json(cache_key).await {
        return Ok(Json(cached));
    }

    let predictions = collect_agent_predictions(&state).await?;
    let items: Vec<Value> = predictions.iter().map(prediction_to_claim).collect();
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
    let agent = find_known_agent(&state, &agent_id).await?;
    let calibration = fetch_agent_json(&state, &agent, "predictions/residuals")
        .await
        .unwrap_or_else(|| json!({"mse": 0.0, "hit_rate": 0.0, "residuals": []}));
    Ok(Json(calibration))
}

async fn list_knowledge_entries(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
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
            query.state.as_deref().is_none_or(|state_filter| {
                task.get("state").and_then(Value::as_str) == Some(state_filter)
            })
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
    let body = json!(PaginatedResponse::new(
        items,
        total,
        query.offset,
        query.limit
    ));
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
        "completed": counts
            .get("completed")
            .copied()
            .unwrap_or_else(|| counts.get("accepted").copied().unwrap_or(0)),
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
            .is_some_and(|value| value_matches_id(Some(value), &id))
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
        if !send_mux_event(&mut sink, "roko-serve", &envelope.payload).await {
            return;
        }
    }

    let (mux_tx, mut mux_rx) = mpsc::unbounded_channel::<MuxEnvelope>();
    let mut agent_streams = HashMap::<String, AgentStreamHandle>::new();
    sync_agent_streams(&state, &mux_tx, &mut agent_streams).await;

    let mut roko_rx = state.event_bus.subscribe();
    let mut discovery_tick = tokio::time::interval(STREAM_DISCOVERY_REFRESH);
    discovery_tick.tick().await;

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        route_client_stream_message(&text, &mut agent_streams, &mut sink).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        warn!(%error, "aggregator websocket receive failed");
                        break;
                    }
                }
            }
            Some(envelope) = mux_rx.recv() => {
                if !send_mux_event(&mut sink, &envelope.source, &envelope.event).await {
                    break;
                }
            }
            event = roko_rx.recv() => {
                match event {
                    Ok(envelope) => {
                        if !send_mux_event(&mut sink, "roko-serve", &envelope.payload).await {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        if !send_mux_event(&mut sink, "roko-serve", &json!({
                            "type": "lagged",
                            "missed": n,
                        })).await {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = discovery_tick.tick() => {
                sync_agent_streams(&state, &mux_tx, &mut agent_streams).await;
            }
        }
    }

    for (_, handle) in agent_streams.drain() {
        handle.task.abort();
    }
    let _ = sink.close().await;
}

async fn route_client_stream_message(
    text: &str,
    agent_streams: &mut HashMap<String, AgentStreamHandle>,
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
) {
    let parsed = serde_json::from_str::<StreamCommand>(text).ok();
    let payload = parsed
        .as_ref()
        .and_then(|command| command.message.as_ref())
        .map_or_else(|| text.to_string(), value_to_stream_payload);

    let target_ids = parsed
        .as_ref()
        .map(stream_command_targets)
        .unwrap_or_default();
    let mut delivered = 0usize;

    if target_ids.is_empty() {
        for handle in agent_streams.values() {
            if handle.sender.send(payload.clone()).is_ok() {
                delivered += 1;
            }
        }
    } else {
        for target in target_ids {
            if let Some(handle) = agent_streams.get(&target)
                && handle.sender.send(payload.clone()).is_ok()
            {
                delivered += 1;
            }
        }
    }

    if delivered == 0 {
        let _ = send_mux_event(
            sink,
            "roko-serve",
            &json!({
                "type": "mux_error",
                "message": "no connected agent stream accepted the outbound message",
            }),
        )
        .await;
    }
}

async fn sync_agent_streams(
    state: &Arc<AppState>,
    mux_tx: &mpsc::UnboundedSender<MuxEnvelope>,
    streams: &mut HashMap<String, AgentStreamHandle>,
) {
    let Ok(agents) = known_agents(state).await else {
        return;
    };

    let desired: HashMap<String, DiscoveredAgent> = agents
        .into_iter()
        .filter(|agent| agent_stream_url(agent).is_some())
        .map(|agent| (agent.agent_id.clone(), agent))
        .collect();

    let removed: Vec<String> = streams
        .keys()
        .filter(|agent_id| !desired.contains_key(*agent_id))
        .cloned()
        .collect();
    for agent_id in removed {
        if let Some(handle) = streams.remove(&agent_id) {
            handle.task.abort();
        }
    }

    for (agent_id, agent) in desired {
        if streams.contains_key(&agent_id) {
            continue;
        }

        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let tx = mux_tx.clone();
        let task = tokio::spawn(async move {
            forward_agent_stream(agent, input_rx, tx).await;
        });
        streams.insert(
            agent_id,
            AgentStreamHandle {
                sender: input_tx,
                task,
            },
        );
    }
}

async fn forward_agent_stream(
    agent: DiscoveredAgent,
    mut input_rx: mpsc::UnboundedReceiver<String>,
    mux_tx: mpsc::UnboundedSender<MuxEnvelope>,
) {
    let Some(url) = agent_stream_url(&agent) else {
        return;
    };

    loop {
        let request = match build_agent_stream_request(&agent, &url) {
            Some(request) => request,
            None => return,
        };

        match connect_async(request).await {
            Ok((stream, _)) => {
                let _ = mux_tx.send(MuxEnvelope {
                    source: agent.agent_id.clone(),
                    event: json!({
                        "type": "stream_status",
                        "state": "connected",
                    }),
                });

                let (mut write, mut read) = stream.split();

                loop {
                    tokio::select! {
                        outbound = input_rx.recv() => {
                            match outbound {
                                Some(payload) => {
                                    if write.send(WsMessage::Text(payload.into())).await.is_err() {
                                        break;
                                    }
                                }
                                None => return,
                            }
                        }
                        inbound = read.next() => {
                            match inbound {
                                Some(Ok(WsMessage::Text(text))) => {
                                    let _ = mux_tx.send(MuxEnvelope {
                                        source: agent.agent_id.clone(),
                                        event: parse_stream_event(&text),
                                    });
                                }
                                Some(Ok(WsMessage::Binary(bytes))) => {
                                    let text = String::from_utf8(bytes.to_vec()).ok();
                                    let event = text
                                        .as_deref()
                                        .map(parse_stream_event)
                                        .unwrap_or_else(|| json!({
                                            "binary_base64": base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes),
                                        }));
                                    let _ = mux_tx.send(MuxEnvelope {
                                        source: agent.agent_id.clone(),
                                        event,
                                    });
                                }
                                Some(Ok(WsMessage::Ping(payload))) => {
                                    if write.send(WsMessage::Pong(payload)).await.is_err() {
                                        break;
                                    }
                                }
                                Some(Ok(WsMessage::Close(_))) | None => break,
                                Some(Ok(_)) => {}
                                Some(Err(error)) => {
                                    warn!(agent_id = %agent.agent_id, %error, "agent websocket stream failed");
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(error) => {
                warn!(agent_id = %agent.agent_id, %url, %error, "failed to connect to agent websocket");
            }
        }

        if input_rx.is_closed() {
            return;
        }

        let _ = mux_tx.send(MuxEnvelope {
            source: agent.agent_id.clone(),
            event: json!({
                "type": "stream_status",
                "state": "reconnecting",
            }),
        });
        tokio::time::sleep(STREAM_RECONNECT_DELAY).await;
    }
}

fn build_agent_stream_request(
    agent: &DiscoveredAgent,
    url: &str,
) -> Option<tokio_tungstenite::tungstenite::http::Request<()>> {
    let mut request = url.to_string().into_client_request().ok()?;
    if let Some(token) = agent.proxy_token.as_deref() {
        let header_value = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, header_value);
    }
    Some(request)
}

fn stream_command_targets(command: &StreamCommand) -> Vec<String> {
    if let Some(agent_id) = &command.agent_id {
        vec![agent_id.clone()]
    } else {
        command.targets.clone()
    }
}

fn value_to_stream_payload(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), ToOwned::to_owned)
}

async fn send_mux_event(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    source: &str,
    event: &impl Serialize,
) -> bool {
    let Ok(payload) = serde_json::to_string(&json!({
        "source": source,
        "event": event,
    })) else {
        return true;
    };
    sink.send(Message::Text(payload.into())).await.is_ok()
}

fn parse_stream_event(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
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

async fn find_known_agent(
    state: &Arc<AppState>,
    agent_id: &str,
) -> Result<DiscoveredAgent, ApiError> {
    known_agents(state)
        .await?
        .into_iter()
        .find(|agent| agent.agent_id == agent_id)
        .ok_or_else(|| ApiError::not_found(format!("agent {agent_id} not found")))
}

async fn known_agents(state: &Arc<AppState>) -> Result<Vec<DiscoveredAgent>, ApiError> {
    let mut agents = state.list_discovered_agents().await;
    let mut known_ids: HashSet<String> =
        agents.iter().map(|agent| agent.agent_id.clone()).collect();

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
            status: "managed".to_string(),
            ..Default::default()
        });
    }

    let hydrated = join_all(
        agents
            .into_iter()
            .map(|agent| async move { hydrate_agent_card(state, agent).await }),
    )
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
    let url = format!(
        "{}/{}",
        rest.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let mut request = state.http_client.get(url);
    if let Some(token) = agent.proxy_token.as_ref() {
        request = request.bearer_auth(token);
    }
    request.send().await.ok()?.json::<Value>().await.ok()
}

fn agent_stream_url(agent: &DiscoveredAgent) -> Option<String> {
    if let Some(websocket) = agent.endpoints.websocket.as_ref() {
        return Some(websocket.clone());
    }

    let rest = agent.endpoints.rest.as_ref()?;
    let websocket_base = if let Some(rest) = rest.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if let Some(rest) = rest.strip_prefix("https://") {
        format!("wss://{rest}")
    } else {
        return None;
    };
    Some(format!("{}/stream", websocket_base.trim_end_matches('/')))
}

fn last_seen_from_health(payload: &Value) -> Option<u64> {
    payload
        .get("uptime_s")
        .and_then(Value::as_u64)
        .map(|uptime| now_secs().saturating_sub(uptime))
}

fn agent_role(agent: &DiscoveredAgent, capabilities: Option<&Value>) -> String {
    capabilities
        .and_then(|payload| payload.get("features"))
        .and_then(Value::as_array)
        .and_then(|features| features.first())
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| agent.capabilities.first().cloned())
        .unwrap_or_else(|| "agent".to_string())
}

fn prediction_to_session(prediction: &Value) -> Value {
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
}

fn prediction_to_claim(prediction: &Value) -> Value {
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
}

fn value_matches_id(value: Option<&Value>, expected: &str) -> bool {
    value.is_some_and(|value| {
        value.as_str() == Some(expected)
            || value
                .as_u64()
                .is_some_and(|numeric| numeric.to_string() == expected)
    })
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::{Body, to_bytes};
    use axum::extract::ws::WebSocketUpgrade;
    use axum::http::Request;
    use roko_core::config::schema::RokoConfig;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message as ClientWsMessage;
    use tower::ServiceExt;

    use crate::deploy::manual::ManualBackend;
    use crate::events::ServerEvent;
    use crate::runtime::NoOpRuntime;
    use crate::state::{AgentEndpoints, AgentRegistrationRecord};

    #[tokio::test]
    async fn compatibility_agent_routes_match_mirage_shapes() {
        let (rest_endpoint, websocket_endpoint, _agent_handle) = spawn_mock_agent_server().await;
        let state = test_state();
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-1".into(),
                owner: "owner-1".into(),
                endpoints: AgentEndpoints {
                    rest: Some(rest_endpoint),
                    websocket: Some(websocket_endpoint),
                    a2a: None,
                    mcp: None,
                },
                ..AgentRegistrationRecord::default()
            })
            .await;

        let router = Router::new()
            .nest("/api", routes())
            .with_state(Arc::clone(&state));

        let skills = call_json(&router, "/api/agents/agent-1/skills").await;
        assert_eq!(skills["agent_id"], "agent-1");
        assert_eq!(skills["skills"]["messaging"]["enabled"], true);

        let heartbeat = call_json(&router, "/api/agents/agent-1/heartbeat").await;
        assert_eq!(heartbeat["agent_id"], "agent-1");
        assert_eq!(heartbeat["alive"], true);
        assert_eq!(heartbeat["busy"], true);

        let trace = call_json(&router, "/api/agents/agent-1/trace?limit=10&offset=0").await;
        assert_eq!(trace["agent_id"], "agent-1");
        assert_eq!(trace["total"], 0);
        assert_eq!(trace["limit"], 10);
        assert_eq!(trace["items"].as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn agent_list_cache_is_invalidated_when_agents_register() {
        let state = test_state();
        let router = Router::new()
            .nest("/api", routes())
            .with_state(Arc::clone(&state));

        let empty = call_json(&router, "/api/agents").await;
        assert_eq!(empty["total"], 0);

        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-fresh".into(),
                owner: "owner-1".into(),
                capabilities: vec!["messaging".into(), "tasks".into()],
                ..AgentRegistrationRecord::default()
            })
            .await;

        let listed = call_json(&router, "/api/agents").await;
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["items"][0]["id"], "agent-fresh");
    }

    #[tokio::test]
    async fn agent_list_owner_filter_excludes_unowned_agents() {
        let state = test_state();
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-owned".into(),
                owner: "owner-1".into(),
                capabilities: vec!["messaging".into()],
                ..AgentRegistrationRecord::default()
            })
            .await;
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-unowned".into(),
                capabilities: vec!["messaging".into()],
                ..AgentRegistrationRecord::default()
            })
            .await;

        let router = Router::new()
            .nest("/api", routes())
            .with_state(Arc::clone(&state));
        let filtered = call_json(&router, "/api/agents?owner=owner-1").await;

        assert_eq!(filtered["total"], 1);
        assert_eq!(filtered["items"][0]["id"], "agent-owned");
    }

    #[tokio::test]
    async fn ws_mux_forwards_roko_and_agent_events() {
        let (rest_endpoint, websocket_endpoint, _agent_handle) = spawn_mock_agent_server().await;
        let state = test_state();
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-1".into(),
                owner: "owner-1".into(),
                endpoints: AgentEndpoints {
                    rest: Some(rest_endpoint),
                    websocket: Some(websocket_endpoint),
                    a2a: None,
                    mcp: None,
                },
                ..AgentRegistrationRecord::default()
            })
            .await;
        state.event_bus.publish(ServerEvent::Error {
            message: "local-backlog".into(),
        });

        let router = Router::new()
            .nest("/api", routes())
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind aggregator");
        let addr = listener.local_addr().expect("aggregator addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve aggregator");
        });

        let (mut socket, _) = connect_async(format!("ws://{addr}/api/ws"))
            .await
            .expect("connect aggregator ws");

        let backlog = next_ws_text(&mut socket).await;
        let backlog_json: Value = serde_json::from_str(&backlog).expect("backlog json");
        assert_eq!(backlog_json["source"], "roko-serve");
        assert_eq!(backlog_json["event"]["type"], "error");

        tokio::time::sleep(Duration::from_millis(150)).await;
        socket
            .send(ClientWsMessage::Text(
                json!({
                    "agent_id": "agent-1",
                    "message": "ping"
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send stream payload");

        let forwarded = next_message_for_source_with_field(&mut socket, "agent-1", "echo").await;
        let forwarded_json: Value = serde_json::from_str(&forwarded).expect("forwarded json");
        assert_eq!(forwarded_json["source"], "agent-1");
        assert_eq!(forwarded_json["event"]["echo"], "ping");

        let _ = socket.close(None).await;
        server.abort();
    }

    fn test_state() -> Arc<AppState> {
        let dir = tempdir().expect("tempdir");
        Arc::new(
            AppState::new(
                dir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        )
    }

    async fn call_json(router: &Router, uri: &str) -> Value {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert!(response.status().is_success());
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        serde_json::from_slice(&body).expect("json body")
    }

    async fn spawn_mock_agent_server() -> (String, String, JoinHandle<()>) {
        async fn mock_health() -> Json<Value> {
            Json(json!({
                "status": "ok",
                "agent_id": "agent-1",
                "uptime_s": 5,
            }))
        }

        async fn mock_capabilities() -> Json<Value> {
            Json(json!({
                "agent_id": "agent-1",
                "features": ["messaging", "predictions", "tasks"],
                "skills": {
                    "messaging": { "enabled": true, "config": {} },
                    "predictions": { "enabled": true, "config": {} },
                    "tasks": { "enabled": true, "config": {} }
                },
                "stats": {
                    "agent_id": "agent-1",
                    "owner": "owner-1",
                    "confirmations_given": 2,
                    "challenges_given": 1,
                    "warnings_posted": 0,
                    "insights_posted": 4,
                    "delta_cycles": 3,
                    "total_cost_usd": 1.25,
                    "total_tokens": 42,
                    "registered_at": 10,
                    "operating_frequency": "active"
                }
            }))
        }

        async fn mock_stats() -> Json<Value> {
            Json(json!({
                "agent_id": "agent-1",
                "owner": "owner-1",
                "confirmations_given": 2,
                "challenges_given": 1,
                "warnings_posted": 0,
                "insights_posted": 4,
                "delta_cycles": 3,
                "total_cost_usd": 1.25,
                "total_tokens": 42,
                "registered_at": 10,
                "operating_frequency": "active"
            }))
        }

        async fn mock_predictions() -> Json<Value> {
            Json(json!([{
                "id": "pred-1",
                "agent_id": "agent-1",
                "market": "Will batch C2 land cleanly?",
                "category": "execution",
                "direction": "up",
                "confidence": 0.82,
                "predicted_value": 0.82,
                "interval_width": 0.1,
                "actual_value": null,
                "ts": 1234
            }]))
        }

        async fn mock_residuals() -> Json<Value> {
            Json(json!({
                "mse": 0.0,
                "hit_rate": 1.0,
                "residuals": []
            }))
        }

        async fn mock_tasks() -> Json<Value> {
            Json(json!([{
                "id": 1,
                "title": "Ship mux",
                "kind": "implementation",
                "state": "accepted",
                "assignee": "agent-1"
            }]))
        }

        async fn mock_stream(ws: WebSocketUpgrade) -> impl IntoResponse {
            ws.on_upgrade(|socket| async move {
                let (mut sink, mut stream) = socket.split();
                while let Some(message) = stream.next().await {
                    match message {
                        Ok(Message::Text(text)) => {
                            let payload = json!({
                                "agent_id": "agent-1",
                                "echo": parse_stream_command_echo(&text),
                            });
                            if sink
                                .send(Message::Text(payload.to_string().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
            })
        }

        let router = Router::new()
            .route("/health", get(mock_health))
            .route("/capabilities", get(mock_capabilities))
            .route("/stats", get(mock_stats))
            .route("/predictions", get(mock_predictions))
            .route("/predictions/residuals", get(mock_residuals))
            .route("/tasks", get(mock_tasks))
            .route("/stream", get(mock_stream));

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind agent");
        let addr = listener.local_addr().expect("agent addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock agent");
        });

        (
            format!("http://{addr}"),
            format!("ws://{addr}/stream"),
            handle,
        )
    }

    fn parse_stream_command_echo(text: &str) -> String {
        serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| value.get("message").cloned())
            .map_or_else(|| text.to_string(), |value| value_to_stream_payload(&value))
    }

    async fn next_ws_text(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> String {
        loop {
            match socket.next().await {
                Some(Ok(ClientWsMessage::Text(text))) => return text.to_string(),
                Some(Ok(_)) => continue,
                Some(Err(error)) => panic!("websocket error: {error}"),
                None => panic!("websocket closed"),
            }
        }
    }

    async fn next_message_for_source_with_field(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        source: &str,
        field: &str,
    ) -> String {
        loop {
            let text = next_ws_text(socket).await;
            let payload: Value = serde_json::from_str(&text).expect("mux payload");
            if payload["source"] == source && payload["event"].get(field).is_some() {
                return text;
            }
        }
    }
}
