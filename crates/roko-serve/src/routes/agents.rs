//! Agent registration, token, and process management endpoints.

use std::collections::BTreeMap;
use std::path::{Component, Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Query;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use validator::Validate;

use alloy::primitives::{Address, FixedBytes};
use alloy::sol;

use roko_chain::alloy_impl::AlloyChainWallet;
use roko_core::HeartbeatPayload;
use roko_core::config::schema::{ModelProfile, RokoConfig};
use roko_learn::provider_health::HealthState;
use roko_runtime::process::{
    ProcessId, ProcessSessionConfig, SpawnConfig, default_process_session_ledger_path,
};

use crate::error::ApiError;
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::routes::run::spawn_background_run;
use crate::runtime::RunResult;
use crate::sanitize::sanitize_agent_content;
use crate::state::{AgentRegistrationRecord, AppState, DiscoveredAgent, OperationStatus};

const AGENT_MESSAGE_INLINE_TIMEOUT: Duration = Duration::from_secs(30);

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/managed-agents", get(list_managed_agents))
        .route("/agents/register", post(register_agent))
        .route("/agents/create", post(create_agent))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/profile", get(get_agent_profile))
        .route("/agents/{id}/config", get(get_agent_config))
        .route("/agents/{id}/stop", post(stop_agent))
        .route("/agents/{id}/episodes", get(agent_episodes))
        .route("/agents/{id}/logs", get(proxy_agent_logs))
        .route("/agents/{id}/message", post(send_message))
        .route("/agents/{id}/start", post(start_agent))
        .route("/agents/{id}/restart", post(restart_agent))
        .route("/agents/{id}/token", get(token_status).post(issue_token))
}

/// `GET /api/managed-agents` — list all managed agent processes **and** registered
/// (discovered) agents that aren't supervised locally.  The dashboard's `useAgents()`
/// hook relies on this endpoint for the fleet roster and live-dot indicator.
async fn list_managed_agents(State(state): State<Arc<AppState>>) -> Json<Value> {
    let entries = state.supervisor.list().await;
    let discovered = state.discovered_agents.read().await;
    let config = state.load_roko_config();
    let heartbeats = state.heartbeats.read().await.clone();

    let mut seen_agent_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut items: Vec<Value> = Vec::with_capacity(entries.len() + discovered.len());

    // 1. Supervised processes (locally spawned via `roko agent start`).
    for (id, label) in &entries {
        let agent_info = discovered
            .values()
            .find(|a| a.agent_id == *label)
            .or_else(|| discovered.get(label.as_str()));
        if let Some(info) = agent_info {
            seen_agent_ids.insert(info.agent_id.clone());
        }
        let agent_id = agent_info.map_or_else(|| label.clone(), |a| a.agent_id.clone());
        let label = agent_info
            .and_then(|a| a.label.clone())
            .unwrap_or_else(|| label.clone());
        let status = agent_info.map_or_else(
            || "running".to_string(),
            |a| {
                if a.status.is_empty() {
                    "running".to_string()
                } else {
                    a.status.clone()
                }
            },
        );
        items.push(agent_dashboard_payload(
            &state,
            &config,
            &heartbeats,
            AgentDashboardInput {
                agent_id,
                label,
                process_id: Some(id.0),
                status,
                current_task: Value::Null,
                agent: agent_info,
            },
        ));
    }

    // 2. Discovered-only agents (registered via POST /api/agents/register or
    //    self-registered by a remote sidecar) that aren't already supervised.
    for agent in discovered.values() {
        if seen_agent_ids.contains(&agent.agent_id) {
            continue;
        }
        items.push(agent_dashboard_payload(
            &state,
            &config,
            &heartbeats,
            AgentDashboardInput {
                agent_id: agent.agent_id.clone(),
                label: agent
                    .label
                    .clone()
                    .unwrap_or_else(|| agent.agent_id.clone()),
                process_id: agent.process_id,
                status: if agent.status.is_empty() {
                    "registered".to_string()
                } else {
                    agent.status.clone()
                },
                current_task: Value::Null,
                agent: Some(agent),
            },
        ));
    }

    Json(Value::Array(items))
}

fn dashboard_default_model(config: &RokoConfig) -> Option<String> {
    let default_model = config.agent.default_model.trim();
    (!default_model.is_empty()).then(|| default_model.to_string())
}

struct AgentDashboardInput<'a> {
    agent_id: String,
    label: String,
    process_id: Option<u64>,
    status: String,
    current_task: Value,
    agent: Option<&'a DiscoveredAgent>,
}

fn agent_dashboard_payload(
    state: &AppState,
    config: &RokoConfig,
    heartbeats: &std::collections::VecDeque<HeartbeatPayload>,
    input: AgentDashboardInput<'_>,
) -> Value {
    let (model, model_source) =
        resolve_agent_model(config, input.agent.and_then(|a| a.model.as_deref()));
    let profile = model
        .as_deref()
        .and_then(|model| model_profile_for(config, model));
    let provider = profile
        .as_ref()
        .map(|(_, profile)| profile.provider.clone());
    let model_profile = profile
        .as_ref()
        .map(|(key, profile)| model_profile_json(key, profile));
    let provider_health = provider
        .as_deref()
        .map(|provider| provider_health_json(state, provider));

    let heartbeat = latest_heartbeat_for_agent(
        heartbeats,
        &[
            input.agent_id.as_str(),
            input.label.as_str(),
            input.agent.and_then(|a| a.label.as_deref()).unwrap_or(""),
        ],
    );
    let heartbeat_summary = heartbeat.map(heartbeat_summary_json);
    let performance = agent_performance_json(input.agent, heartbeat);
    let learning = agent_learning_json(input.agent, heartbeat);
    let costs = agent_cost_json(heartbeat, model_profile.as_ref());
    let endpoints = input.agent.map(|agent| {
        json!({
            "rest": agent.endpoints.rest,
            "websocket": agent.endpoints.websocket,
            "a2a": agent.endpoints.a2a,
            "mcp": agent.endpoints.mcp,
        })
    });
    let stream_url = input.agent.and_then(stream_url_for_agent);
    let capabilities = input
        .agent
        .map(|agent| agent.capabilities.clone())
        .unwrap_or_default();
    let domain_tags = input
        .agent
        .map(|agent| agent.domain_tags.clone())
        .unwrap_or_default();
    let skills = input
        .agent
        .map(|agent| agent.skills.clone())
        .unwrap_or_default();
    let role = capabilities
        .first()
        .cloned()
        .or_else(|| domain_tags.first().cloned());
    let message_endpoint = format!("/api/agents/{}/message", input.agent_id);

    json!({
        "id": input.agent_id.clone(),
        "agent_id": input.agent_id,
        "process_id": input.process_id,
        "label": input.label,
        "status": input.status,
        "role": role,
        "model": model,
        "model_source": model_source,
        "provider": provider,
        "provider_health": provider_health,
        "model_profile": model_profile,
        "tier": input.agent.and_then(|agent| agent.tier.clone()),
        "current_task": input.current_task,
        "owner": input.agent.map(|agent| agent.owner.clone()).unwrap_or_default(),
        "registered_at": input.agent.map(|agent| agent.registered_at),
        "last_seen_at": input.agent.map(|agent| agent.last_seen_at),
        "card_uri": input.agent.and_then(|agent| agent.card_uri.clone()),
        "capabilities": capabilities,
        "domain_tags": domain_tags,
        "skills": skills,
        "reputation": input.agent.map_or(0, |agent| agent.reputation),
        "past_jobs_completed": input.agent.map_or(0, |agent| agent.past_jobs_completed),
        "max_concurrent_jobs": input.agent.map_or(0, |agent| agent.max_concurrent_jobs),
        "endpoints": endpoints,
        "heartbeat": heartbeat_summary,
        "performance": performance,
        "learning": learning,
        "costs": costs,
        "chat": {
            "message_endpoint": message_endpoint,
            "streaming_supported": stream_url.is_some(),
            "stream_endpoint": stream_url,
            "inline_timeout_ms": AGENT_MESSAGE_INLINE_TIMEOUT.as_millis(),
            "correlation": "run_id",
        },
    })
}

fn resolve_agent_model(
    config: &RokoConfig,
    agent_model: Option<&str>,
) -> (Option<String>, &'static str) {
    if let Some(model) = agent_model.map(str::trim).filter(|model| !model.is_empty()) {
        return (Some(model.to_string()), "agent");
    }
    if let Some(model) = dashboard_default_model(config) {
        return (Some(model), "default");
    }
    (None, "none")
}

fn model_profile_for(config: &RokoConfig, model: &str) -> Option<(String, ModelProfile)> {
    let models = config.effective_models();
    models
        .get(model)
        .cloned()
        .map(|profile| (model.to_string(), profile))
        .or_else(|| {
            models
                .into_iter()
                .find(|(_, profile)| profile.slug == model)
        })
}

fn model_profile_json(key: &str, profile: &ModelProfile) -> Value {
    json!({
        "key": key,
        "slug": profile.slug,
        "provider": profile.provider,
        "context_window": profile.context_window,
        "max_output": profile.max_output,
        "tool_format": profile.tool_format,
        "thinking_level": profile.thinking_level,
        "supports": {
            "tools": profile.supports_tools,
            "thinking": profile.supports_thinking,
            "vision": profile.supports_vision,
            "web_search": profile.supports_web_search || profile.supports_search,
            "mcp_tools": profile.supports_mcp_tools,
            "partial": profile.supports_partial,
            "grounding": profile.supports_grounding,
            "code_execution": profile.supports_code_execution,
            "caching": profile.supports_caching,
            "citations": profile.supports_citations,
            "async": profile.supports_async,
            "embedding": profile.is_embedding_model,
        },
        "pricing": {
            "input_per_m": profile.cost_input_per_m,
            "output_per_m": profile.cost_output_per_m,
            "input_per_m_high": profile.cost_input_per_m_high,
            "output_per_m_high": profile.cost_output_per_m_high,
            "cache_read_per_m": profile.cost_cache_read_per_m,
            "cache_write_per_m": profile.cost_cache_write_per_m,
            "per_request": profile.cost_per_request,
        },
    })
}

fn provider_health_json(state: &AppState, provider: &str) -> Value {
    let status = state.provider_health.get(provider);
    json!({
        "state": provider_state_label(status.state),
        "consecutive_failures": status.consecutive_failures,
        "total_attempts": status.total_attempts,
        "total_successes": status.total_successes,
        "error_rate": status.error_rate(),
        "last_failure_at": status.last_failure_at,
        "last_success_at": status.last_success_at,
    })
}

fn provider_state_label(state: HealthState) -> &'static str {
    match state {
        HealthState::Healthy => "healthy",
        HealthState::Unhealthy { .. } => "unhealthy",
        HealthState::Probing => "probing",
    }
}

fn latest_heartbeat_for_agent<'a>(
    heartbeats: &'a std::collections::VecDeque<HeartbeatPayload>,
    candidates: &[&str],
) -> Option<&'a HeartbeatPayload> {
    heartbeats.iter().rev().find(|heartbeat| {
        candidates
            .iter()
            .any(|candidate| !candidate.is_empty() && heartbeat.sender_id == *candidate)
    })
}

fn heartbeat_summary_json(heartbeat: &HeartbeatPayload) -> Value {
    let age_seconds = heartbeat_age_seconds(&heartbeat.timestamp);
    json!({
        "sender_id": heartbeat.sender_id,
        "timestamp": heartbeat.timestamp,
        "age_seconds": age_seconds,
        "stale": age_seconds.is_none_or(|age| age > 300),
        "active_tasks": heartbeat.active_tasks,
        "completed_tasks": heartbeat.completed_tasks,
        "failed_tasks": heartbeat.failed_tasks,
        "active_agents": heartbeat.active_agents,
        "frequency": heartbeat.frequency,
        "metrics": heartbeat.metrics,
    })
}

fn heartbeat_age_seconds(timestamp: &str) -> Option<i64> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) {
        return Some(
            (chrono::Utc::now() - parsed.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0),
        );
    }
    let raw = timestamp.parse::<i64>().ok()?;
    let seconds = if raw > 9_999_999_999 {
        raw / 1_000
    } else {
        raw
    };
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|parsed| (chrono::Utc::now() - parsed).num_seconds().max(0))
}

fn agent_performance_json(
    agent: Option<&DiscoveredAgent>,
    heartbeat: Option<&HeartbeatPayload>,
) -> Value {
    let metrics = heartbeat.map(|heartbeat| &heartbeat.metrics);
    json!({
        "active_tasks": heartbeat.map(|heartbeat| heartbeat.active_tasks).unwrap_or_default(),
        "completed_tasks": heartbeat.map(|heartbeat| heartbeat.completed_tasks).unwrap_or_default(),
        "failed_tasks": heartbeat.map(|heartbeat| heartbeat.failed_tasks).unwrap_or_default(),
        "frequency": heartbeat.map(|heartbeat| heartbeat.frequency).unwrap_or_default(),
        "gate_pass_rate": metric_value(metrics, &["gate_pass_rate", "gate_rate", "success_rate"]),
        "context_utilization": metric_value(metrics, &["context_utilization", "context_utilization_pct"]),
        "token_burn_rate": metric_value(metrics, &["token_burn_rate", "tokens_per_minute", "tokens_min"]),
        "latency_ms": metric_value(metrics, &["latency_ms", "p50_latency_ms"]),
        "throughput": metric_value(metrics, &["throughput", "tasks_per_hour"]),
        "reputation": agent.map_or(0, |agent| agent.reputation),
        "past_jobs_completed": agent.map_or(0, |agent| agent.past_jobs_completed),
        "max_concurrent_jobs": agent.map_or(0, |agent| agent.max_concurrent_jobs),
    })
}

fn agent_learning_json(
    agent: Option<&DiscoveredAgent>,
    heartbeat: Option<&HeartbeatPayload>,
) -> Value {
    let metrics = heartbeat.map(|heartbeat| &heartbeat.metrics);
    json!({
        "episode_count": metric_value(metrics, &["episode_count", "episodes"]),
        "playbook_size": metric_value(metrics, &["playbook_size", "patterns"]),
        "insight_count": metric_value(metrics, &["insight_count", "insights"]),
        "gate_pass_rate": metric_value(metrics, &["gate_pass_rate", "gate_rate", "success_rate"]),
        "context_lift": metric_value(metrics, &["context_lift", "section_lift", "vcg_lift"]),
        "memory_freshness": metric_value(metrics, &["memory_freshness", "knowledge_freshness"]),
        "skills": agent.map(|agent| agent.skills.clone()).unwrap_or_default(),
        "capabilities": agent.map(|agent| agent.capabilities.clone()).unwrap_or_default(),
    })
}

fn agent_cost_json(heartbeat: Option<&HeartbeatPayload>, model_profile: Option<&Value>) -> Value {
    let metrics = heartbeat.map(|heartbeat| &heartbeat.metrics);
    json!({
        "cumulative_usd": metric_value(metrics, &["cumulative_cost_usd", "cost_usd", "total_cost_usd"]),
        "burn_rate_usd_per_hour": metric_value(metrics, &["burn_rate_usd_per_hour", "usd_per_hour"]),
        "token_burn_rate": metric_value(metrics, &["token_burn_rate", "tokens_per_minute", "tokens_min"]),
        "pricing": model_profile
            .and_then(|profile| profile.get("pricing"))
            .cloned()
            .unwrap_or(Value::Null),
    })
}

fn metric_value(metrics: Option<&std::collections::HashMap<String, f64>>, keys: &[&str]) -> Value {
    metrics
        .and_then(|metrics| keys.iter().find_map(|key| metrics.get(*key).copied()))
        .map_or(Value::Null, Value::from)
}

/// `POST /api/agents/register` — upsert a discovery entry for an agent server.
async fn register_agent(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<RegisterAgentRequest>,
) -> Result<Json<Value>, ApiError> {
    let agent = state
        .upsert_discovered_agent(AgentRegistrationRecord {
            agent_id: req.agent_id.clone(),
            label: req.label,
            process_id: req.process_id,
            owner: req.owner.unwrap_or_default(),
            endpoints: crate::state::AgentEndpoints {
                rest: req.rest_endpoint,
                websocket: req.websocket_endpoint,
                a2a: req.a2a_endpoint,
                mcp: req.mcp_endpoint,
            },
            card_uri: req.card_uri,
            capabilities: req.capabilities,
            domain_tags: req.domain_tags,
            tier: req.tier,
            model: req.model,
            reputation: req.reputation,
            skills: req.skills,
            past_jobs_completed: req.past_jobs_completed,
            max_concurrent_jobs: req.max_concurrent_jobs,
        })
        .await;

    // Dual-write: non-blocking on-chain registration when chain wallet is configured.
    if let Some(wallet) = state.chain_wallet.as_ref() {
        let wallet = Arc::clone(wallet);
        let config = state.load_roko_config();
        let agent_id = agent.agent_id.clone();
        let capabilities = agent.capabilities.join(",");
        if let Some(registry_addr) = config.chain.agent_registry.clone() {
            tokio::spawn(async move {
                if let Err(e) =
                    chain_register_agent(&wallet, &registry_addr, &agent_id, &capabilities).await
                {
                    tracing::warn!(agent_id, error = %e, "on-chain agent registration failed (non-blocking)");
                }
            });
        }
    }

    let token = if req.issue_token.unwrap_or(false) {
        state.rotate_agent_token(&agent.agent_id).await
    } else {
        None
    };

    Ok(Json(json!({
        "agent": agent,
        "token": token,
    })))
}

// Minimal sol! binding for on-chain agent registration.
sol! {
    #[sol(rpc)]
    contract OnChainAgentRegistry {
        function register(string calldata capabilities, bytes32 passportHash) external;
    }
}

/// Non-blocking on-chain agent registration via alloy.
async fn chain_register_agent(
    wallet: &AlloyChainWallet,
    registry_addr: &str,
    agent_id: &str,
    capabilities: &str,
) -> Result<(), String> {
    let addr: Address = registry_addr.parse().map_err(|e| format!("{e}"))?;
    let registry = OnChainAgentRegistry::new(addr, wallet.provider());
    let passport_hash = FixedBytes::ZERO;
    let pending = registry
        .register(capabilities.to_string(), passport_hash)
        .send()
        .await
        .map_err(|e| format!("send failed: {e}"))?;
    let tx_hash = pending.tx_hash();
    tracing::info!(
        agent_id,
        tx_hash = %tx_hash,
        "on-chain agent registration tx submitted"
    );
    Ok(())
}

/// Request payload for `POST /api/agents/create`.
#[derive(Debug, Deserialize, Validate)]
struct CreateAgentRequest {
    /// Agent name / identifier (required, 1–128 chars).
    #[validate(
        length(min = 1, max = 128),
        custom(function = "crate::extract::validate_non_blank")
    )]
    name: String,
    /// Agent domain: coding, research, chain, or general.
    #[serde(default = "default_domain")]
    domain: String,
    /// Natural-language prompt describing what the agent should do.
    #[serde(default)]
    prompt: Option<String>,
    /// Skill tags for matchmaking.
    #[serde(default)]
    skills: Vec<String>,
    /// Agent tier label.
    #[serde(default)]
    tier: Option<String>,
    /// Optional per-agent model override.
    #[serde(default)]
    model: Option<String>,
    /// Reputation score (0–100).
    #[serde(default)]
    reputation: u32,
    /// Maximum concurrent jobs.
    #[serde(default)]
    max_concurrent_jobs: u32,
    /// Capabilities (e.g. messaging, tasks, research).
    #[serde(default)]
    capabilities: Vec<String>,
}

fn default_domain() -> String {
    "general".to_string()
}

impl RequestPayload for CreateAgentRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)?;
        let valid_domains = ["coding", "research", "chain", "general"];
        if !valid_domains.contains(&self.domain.as_str()) {
            return Err(ApiError::bad_request(format!(
                "unknown domain '{}'; valid: {}",
                self.domain,
                valid_domains.join(", ")
            )));
        }
        Ok(())
    }
}

/// `POST /api/agents/create` — create an agent manifest on disk and register it.
///
/// Writes a minimal manifest to `.roko/agents/<name>/manifest.toml` and upserts
/// a discovery entry so the agent appears in the fleet roster immediately.
///
/// T3-27: the agent name must be a single, traversal-free filesystem segment
/// and the resolved path is verified to live inside the workspace's
/// `.roko/agents` root before any directory is touched. The manifest body is
/// serialised through `toml::to_string_pretty` so user-controlled strings
/// (prompts, domain labels, …) cannot inject sibling tables.
async fn create_agent(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<CreateAgentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let agents_root = state.workdir.join(".roko").join("agents");
    let agents_dir = resolve_agent_dir(&agents_root, &req.name)?;
    let manifest_path = agents_dir.join("manifest.toml");

    // Don't overwrite an existing agent.
    if manifest_path.exists() {
        return Err(ApiError::conflict(format!(
            "agent '{}' already exists at {}",
            req.name,
            manifest_path.display()
        )));
    }

    let prompt = req
        .prompt
        .clone()
        .unwrap_or_else(|| "You are a helpful autonomous agent.".to_string());
    let manifest_struct = AgentManifest {
        schema_version: 1,
        core: AgentManifestCore {
            prompt,
            mode: "self_hosted".to_string(),
            domain: BTreeMap::from([(req.domain.clone(), toml::value::Table::new())]),
        },
    };
    let mut manifest_toml = String::from("# Auto-generated by POST /api/agents/create\n");
    manifest_toml.push_str(
        &toml::to_string_pretty(&manifest_struct)
            .map_err(|e| ApiError::internal(format!("serialize manifest: {e}")))?,
    );

    tokio::fs::create_dir_all(&agents_dir)
        .await
        .map_err(|e| ApiError::internal(format!("create agent dir: {e}")))?;
    tokio::fs::write(&manifest_path, &manifest_toml)
        .await
        .map_err(|e| ApiError::internal(format!("write manifest: {e}")))?;

    // Also register in the discovery registry.
    let capabilities = if req.capabilities.is_empty() {
        match req.domain.as_str() {
            "research" => vec!["messaging".to_string(), "research".to_string()],
            _ => vec!["messaging".to_string(), "tasks".to_string()],
        }
    } else {
        req.capabilities
    };

    let agent = state
        .upsert_discovered_agent(AgentRegistrationRecord {
            agent_id: req.name.clone(),
            label: Some(req.name.clone()),
            capabilities,
            domain_tags: vec![req.domain.clone()],
            skills: req.skills,
            tier: req.tier,
            model: req.model,
            reputation: req.reputation,
            max_concurrent_jobs: req.max_concurrent_jobs,
            ..Default::default()
        })
        .await;

    // Dual-write: non-blocking on-chain registration when chain wallet is configured.
    if let Some(wallet) = state.chain_wallet.as_ref() {
        let wallet = Arc::clone(wallet);
        let config = state.load_roko_config();
        let agent_id = agent.agent_id.clone();
        let capabilities = agent.capabilities.join(",");
        if let Some(registry_addr) = config.chain.agent_registry.clone() {
            tokio::spawn(async move {
                if let Err(e) =
                    chain_register_agent(&wallet, &registry_addr, &agent_id, &capabilities).await
                {
                    tracing::warn!(agent_id, error = %e, "on-chain agent registration failed (non-blocking)");
                }
            });
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "agent": agent,
            "manifest_path": manifest_path.display().to_string(),
            "domain": req.domain,
        })),
    ))
}

/// Strongly-typed manifest written to disk by `POST /api/agents/create`. The
/// fields exactly mirror the legacy hand-rolled TOML so existing manifest
/// readers do not have to change. Serialising through `toml::to_string_pretty`
/// (rather than `format!`) is what defeats the prompt-driven TOML-injection
/// attack: every string is encoded as a quoted TOML value, so a user-supplied
/// `prompt = "x\"\n[malicious]\nsecret = \"y\""` becomes a single multi-line
/// quoted string instead of a sibling `[malicious]` table.
#[derive(Debug, Serialize)]
struct AgentManifest {
    schema_version: u32,
    core: AgentManifestCore,
}

#[derive(Debug, Serialize)]
struct AgentManifestCore {
    prompt: String,
    mode: String,
    // Each domain section is a TOML sub-table so the rendered manifest
    // contains a `[core.domain.<name>]` header even when the body is
    // empty. We use `toml::value::Table` (a `BTreeMap<String, toml::Value>`)
    // so the value type is not zero-sized — clippy's
    // `zero_sized_map_values` lint flags maps whose value type carries
    // no information, which would otherwise trip on a marker struct.
    domain: BTreeMap<String, toml::value::Table>,
}

/// Validate `name` as a single safe filesystem segment under `agents_root`
/// and return the resolved (still possibly non-existent) directory.
///
/// We refuse the request if any of the following hold:
/// * the name is empty after trimming;
/// * the name contains a path separator (`/` or platform-native);
/// * the name contains `..` or `.` segments, or any non-`Normal` component;
/// * the name is exactly `..` or `.`;
/// * the resolved absolute path does not start with the canonicalised
///   `agents_root` (defence-in-depth against any escape we missed above).
///
/// Canonicalisation is performed on the parent directory only, because the
/// agent directory does not exist at request time.
fn resolve_agent_dir(agents_root: &StdPath, name: &str) -> Result<PathBuf, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("agent name must not be empty"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(ApiError::bad_request(
            "agent name must not contain path separators",
        ));
    }
    if trimmed == "." || trimmed == ".." {
        return Err(ApiError::bad_request("agent name must not be '.' or '..'"));
    }
    let candidate = StdPath::new(trimmed);
    let mut components = candidate.components();
    match components.next() {
        Some(Component::Normal(_)) if components.next().is_none() => {}
        _ => {
            return Err(ApiError::bad_request(
                "agent name must be a single non-empty path segment",
            ));
        }
    }

    // Defence-in-depth: ensure the resolved path actually lives under the
    // canonical agents root. We canonicalise the workdir's `.roko/agents`
    // parent (which we create lazily on the very first agent registration).
    if !agents_root.exists() {
        std::fs::create_dir_all(agents_root)
            .map_err(|e| ApiError::internal(format!("create agents root: {e}")))?;
    }
    let canonical_root = agents_root
        .canonicalize()
        .map_err(|e| ApiError::internal(format!("canonicalize agents root: {e}")))?;
    let resolved = canonical_root.join(trimmed);
    if !resolved.starts_with(&canonical_root) {
        return Err(ApiError::bad_request(
            "resolved agent path escapes the workspace agents root",
        ));
    }
    Ok(resolved)
}

// ─── Agent lifecycle: start / restart ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct StartAgentRequest {
    /// Override bind address (default: "127.0.0.1:0" for auto-port).
    #[serde(default)]
    bind: Option<String>,
    /// Override the default model for this agent.
    #[serde(default)]
    model_override: Option<String>,
}

#[derive(Debug, Serialize)]
struct StartAgentResponse {
    status: String,
    process_id: u64,
    agent_id: String,
}

/// `POST /api/agents/{id}/start` — spawn an agent sidecar process.
///
/// Mirrors the logic of `roko agent start`: verifies a manifest exists,
/// checks the agent isn't already supervised, then spawns
/// `roko agent serve --agent-id <id> --bind <addr>` under the [`ProcessSupervisor`].
/// The sidecar will self-register back to `/api/agents/register` with its port.
async fn start_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(opts): Json<StartAgentRequest>,
) -> Result<Json<StartAgentResponse>, ApiError> {
    // 1. Verify agent manifest exists.
    let manifest_path = state
        .workdir
        .join(".roko")
        .join("agents")
        .join(&agent_id)
        .join("manifest.toml");
    let known_in_discovery = state.discovered_agent(&agent_id).await.is_some();

    if !manifest_path.exists() && !known_in_discovery {
        return Err(ApiError::not_found(format!(
            "agent '{agent_id}' not found (no manifest at {} and not in discovery)",
            manifest_path.display()
        )));
    }

    // Check for a DELETED marker.
    let deleted_marker = manifest_path
        .parent()
        .map(|p| p.join("DELETED"))
        .filter(|p| p.exists());
    if deleted_marker.is_some() {
        return Err(ApiError::bad_request(format!(
            "agent '{agent_id}' has been deleted"
        )));
    }

    // 2. Check agent isn't already running in the supervisor.
    if state.find_process_by_label(&agent_id).await.is_some() {
        return Err(ApiError::conflict(format!(
            "agent '{agent_id}' is already running"
        )));
    }

    // 3. Build spawn config.
    let roko_bin = std::env::current_exe()
        .map_err(|e| ApiError::internal(format!("determine roko binary path: {e}")))?;
    let bind = opts.bind.unwrap_or_else(|| "127.0.0.1:0".to_string());

    let mut args = vec![
        "agent".to_string(),
        "serve".to_string(),
        "--agent-id".to_string(),
        agent_id.clone(),
        "--bind".to_string(),
        bind,
    ];

    // Pass the serve URL so the sidecar can self-register.
    let config = state.load_roko_config();
    let port = config.server.port;
    let serve_url = format!("http://127.0.0.1:{port}");
    args.extend(["--serve-url".to_string(), serve_url]);

    if let Some(ref model) = opts.model_override {
        args.extend(["--model".to_string(), model.clone()]);
    }

    let spawn_config = SpawnConfig {
        program: roko_bin.to_string_lossy().into_owned(),
        args,
        working_dir: Some(state.workdir.clone()),
        session: Some(ProcessSessionConfig {
            session_id: format!("agent:{agent_id}"),
            invocation_id: uuid::Uuid::new_v4().to_string(),
            backend_id: "roko-agent-sidecar".to_string(),
            task_id: Some(agent_id.clone()),
            reuse_policy_id: Some("serve-agent-sidecar".to_string()),
            resumable: true,
            timeout_ms: None,
            ledger_path: default_process_session_ledger_path(&state.workdir),
        }),
        label: agent_id.clone(),
        ..Default::default()
    };

    // 4. Spawn via supervisor.
    let process_id = state
        .supervisor
        .spawn(spawn_config)
        .await
        .map_err(|e| ApiError::internal(format!("spawn agent: {e}")))?;

    tracing::info!(agent_id = %agent_id, process_id = %process_id, "agent started via HTTP");

    Ok(Json(StartAgentResponse {
        status: "starting".to_string(),
        process_id: process_id.0,
        agent_id,
    }))
}

/// `POST /api/agents/{id}/restart` — shut down and re-spawn an agent.
async fn restart_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    // 1. Find the running process.
    let (pid, _, _) = state
        .find_process_by_label(&agent_id)
        .await
        .ok_or_else(|| {
            ApiError::not_found(format!("agent '{agent_id}' is not running in supervisor"))
        })?;

    // 2. Shut down gracefully.
    state.supervisor.shutdown(pid).await;

    // 3. Re-spawn with default options.
    let restart_result = start_agent(
        State(Arc::clone(&state)),
        Path(agent_id.clone()),
        Json(StartAgentRequest {
            bind: None,
            model_override: None,
        }),
    )
    .await;

    match restart_result {
        Ok(Json(resp)) => Ok(Json(json!({
            "status": "restarting",
            "old_process_id": pid.0,
            "new_process_id": resp.process_id,
            "agent_id": agent_id,
        }))),
        Err(e) => Err(ApiError::internal(format!(
            "agent '{agent_id}' stopped but failed to restart: {e:?}"
        ))),
    }
}

/// `GET /api/agents/{id}` — get info about a discovered or supervised agent.
///
/// The response is enriched with `process_status`, `uptime_secs`, `os_pid`,
/// and `last_heartbeat` when the agent is supervised locally.
async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let config = state.load_roko_config();
    let heartbeats = state.heartbeats.read().await.clone();

    // Look up process lifecycle info from supervisor.
    let process_info = state.find_process_by_label(&id).await;
    let (process_status, uptime_secs, os_pid) = match &process_info {
        Some((_, os, uptime)) => ("running", Some(uptime.as_secs()), *os),
        None => ("stopped", None, None),
    };

    // Find last heartbeat for this agent.
    let last_hb = latest_heartbeat_for_agent(&heartbeats, &[id.as_str()]);
    let last_heartbeat_ts = last_hb.map(|hb| hb.timestamp.clone());

    if let Some(agent) = state.discovered_agent(&id).await {
        let mut payload = agent_dashboard_payload(
            &state,
            &config,
            &heartbeats,
            AgentDashboardInput {
                agent_id: agent.agent_id.clone(),
                label: agent
                    .label
                    .clone()
                    .unwrap_or_else(|| agent.agent_id.clone()),
                process_id: agent.process_id,
                status: if agent.status.is_empty() {
                    "registered".to_string()
                } else {
                    agent.status.clone()
                },
                current_task: Value::Null,
                agent: Some(&agent),
            },
        );
        // Enrich with process lifecycle fields.
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("process_status".to_string(), json!(process_status));
            if let Some(up) = uptime_secs {
                obj.insert("uptime_secs".to_string(), json!(up));
            }
            if let Some(pid) = os_pid {
                obj.insert("os_pid".to_string(), json!(pid));
            }
            if let Some(ref ts) = last_heartbeat_ts {
                obj.insert("last_heartbeat".to_string(), json!(ts));
            }
        }
        return Ok(Json(payload));
    }

    let parsed_id = id
        .parse::<u64>()
        .map_err(|_| ApiError::not_found(format!("agent {id} not found")))?;
    let entries = state.supervisor.list().await;
    let found = entries.into_iter().find(|(pid, _)| pid.0 == parsed_id);

    match found {
        Some((pid, label)) => {
            let mut payload = agent_dashboard_payload(
                &state,
                &config,
                &heartbeats,
                AgentDashboardInput {
                    agent_id: label.clone(),
                    label,
                    process_id: Some(pid.0),
                    status: "running".to_string(),
                    current_task: Value::Null,
                    agent: None,
                },
            );
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("process_status".to_string(), json!("running"));
            }
            Ok(Json(payload))
        }
        None => Err(ApiError::not_found(format!("agent {id} not found"))),
    }
}

/// `GET /api/agents/{id}/profile` — alias for the enriched agent detail payload.
async fn get_agent_profile(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    get_agent(State(state), Path(id)).await
}

/// `GET /api/agents/{id}/config` — return the agent manifest plus runtime metadata.
async fn get_agent_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let agents_root = state.workdir.join(".roko").join("agents");
    let agent_dir = resolve_agent_dir(&agents_root, &id)?;
    let manifest_path = agent_dir.join("manifest.toml");
    let deleted = agent_dir.join("DELETED").exists();
    let manifest_text = match tokio::fs::read_to_string(&manifest_path).await {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(ApiError::internal(format!("read agent manifest: {error}"))),
    };

    let discovered = state.discovered_agent(&id).await;
    if manifest_text.is_none() && discovered.is_none() {
        return Err(ApiError::not_found(format!(
            "agent '{id}' not found (no manifest at {})",
            manifest_path.display()
        )));
    }

    let manifest = manifest_text
        .as_deref()
        .and_then(|text| toml::from_str::<toml::Value>(text).ok())
        .and_then(|value| serde_json::to_value(value).ok());
    let process_info = state.find_process_by_label(&id).await;
    let (process_status, uptime_secs, os_pid) = match process_info {
        Some((_, os, uptime)) => ("running", Some(uptime.as_secs()), os),
        None => ("stopped", None, None),
    };

    Ok(Json(json!({
        "agent_id": id,
        "manifest_path": manifest_path.display().to_string(),
        "manifest_exists": manifest_text.is_some(),
        "deleted": deleted,
        "manifest_toml": manifest_text,
        "manifest": manifest,
        "runtime": {
            "process_status": process_status,
            "uptime_secs": uptime_secs,
            "os_pid": os_pid,
        },
        "registration": discovered,
    })))
}

/// `POST /api/agents/{id}/stop` — shut down a specific supervised process.
async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    state.supervisor.shutdown(ProcessId(id)).await.map_or_else(
        || Err(ApiError::not_found(format!("agent {id} not found"))),
        |o| {
            Ok(Json(json!({
                "id": id,
                "outcome": format!("{o:?}"),
            })))
        },
    )
}

/// `GET /api/agents/{id}/episodes` — filter episodes for a specific agent.
///
/// Accepts both string agent IDs (`sam-local-qwen`) and numeric process IDs (`42`).
async fn agent_episodes(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let path = state.layout.episodes_path();
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Json(json!([]))),
        Err(e) => {
            return Err(ApiError::internal(format!("read episodes: {e}")));
        }
    };

    let numeric_id = id.parse::<u64>().ok();
    let mut filtered: Vec<Value> = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line)
            .map_err(|e| ApiError::internal(format!("parse episodes line {}: {e}", line_no + 1)))?;
        let matches = value.get("agent_id").is_some_and(|a| {
            a.as_str() == Some(id.as_str()) || numeric_id.is_some_and(|n| a.as_u64() == Some(n))
        });
        if matches {
            filtered.push(value);
        }
    }

    Ok(Json(Value::Array(filtered)))
}

#[derive(Debug, Deserialize, Default)]
struct LogsQuery {
    #[serde(default)]
    tail: Option<usize>,
}

/// `GET /api/agents/{id}/logs` — proxy agent-sidecar logs, preserving upstream status.
async fn proxy_agent_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Response, ApiError> {
    let agent = state
        .discovered_agent(&id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("agent {id} not found")))?;
    let rest = agent
        .endpoints
        .rest
        .ok_or_else(|| ApiError::bad_request(format!("agent {id} has no rest endpoint")))?;

    let url = format!("{}/logs", rest.trim_end_matches('/'));
    let mut request = state.http_client.get(url);
    if let Some(tail) = query.tail {
        request = request.query(&[("tail", tail)]);
    }
    if let Some(token) = agent.proxy_token.as_ref() {
        request = request.bearer_auth(token);
    }

    let response = request
        .send()
        .await
        .map_err(|error| ApiError::internal(format!("proxy to agent logs failed: {error}")))?;
    let status = response.status();
    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
    let body = response
        .bytes()
        .await
        .map_err(|error| ApiError::internal(format!("read proxied agent logs failed: {error}")))?;

    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }

    builder.body(Body::from(body)).map_err(|error| {
        ApiError::internal(format!("build proxied agent logs response failed: {error}"))
    })
}

#[derive(Debug, Deserialize, Validate)]
struct SendMessageRequest {
    #[serde(alias = "content")]
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
    message: String,
    #[serde(default)]
    context: Option<Value>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    response_mode: Option<String>,
}

impl RequestPayload for SendMessageRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

/// `POST /api/agents/{id}/message` — send a message to a registered agent or fall back to a run.
async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    ValidJson(req): ValidJson<SendMessageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(agent) = state.discovered_agent(&agent_id).await {
        if let Some(ws_url) = stream_url_for_agent(&agent) {
            let (run_id, rx) =
                spawn_sidecar_stream(Arc::clone(&state), agent_id.clone(), ws_url, &agent, &req);
            match wait_for_sidecar_stream(rx, AGENT_MESSAGE_INLINE_TIMEOUT).await {
                Some(Ok(response_text)) => {
                    return Ok((
                        StatusCode::OK,
                        Json(json!({
                            "run_id": run_id,
                            "agent_id": agent_id,
                            "status": "completed",
                            "response": response_text,
                        })),
                    ));
                }
                Some(Err(error)) => {
                    tracing::warn!(agent_id, %error, "streaming agent message proxy failed, trying direct message proxy");
                }
                None => {
                    return Ok((
                        StatusCode::ACCEPTED,
                        Json(json!({
                            "run_id": run_id,
                            "agent_id": agent_id,
                            "conversation_id": req.conversation_id,
                            "response_mode": req.response_mode,
                            "status": "running",
                        })),
                    ));
                }
            }
        }

        if let Some(rest) = agent.endpoints.rest {
            let url = format!("{}/message", rest.trim_end_matches('/'));
            let mut request = state.http_client.post(url).json(&json!({
                "prompt": req.message,
                "context": req.context.clone(),
            }));

            if let Some(token) = agent.proxy_token {
                request = request.bearer_auth(token);
            }

            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    let body = response
                        .json::<Value>()
                        .await
                        .unwrap_or_else(|_| json!({ "status": "proxy_error" }));

                    // Extract text from sidecar response.  If the response
                    // field contains raw JSON (e.g. Claude CLI streaming
                    // protocol), try to extract the result text.
                    let response_text = body
                        .get("response")
                        .and_then(Value::as_str)
                        .map(extract_response_text)
                        .unwrap_or_default();

                    let run_id = uuid::Uuid::new_v4().to_string();

                    // Emit sanitized agent_output for consumers.
                    let clean_text = sanitize_agent_content(&response_text);
                    state
                        .event_bus
                        .publish(crate::events::ServerEvent::AgentOutput {
                            agent_id: agent_id.clone(),
                            run_id: Some(run_id.clone()),
                            content: clean_text.clone(),
                            done: true,
                            metadata: None,
                        });
                    // Emit raw trace for debug subscribers.
                    state
                        .event_bus
                        .publish(crate::events::ServerEvent::AgentTrace {
                            agent_id: agent_id.clone(),
                            run_id: Some(run_id.clone()),
                            content: response_text.clone(),
                            tool_calls: None,
                            reasoning: None,
                            usage: None,
                            done: true,
                        });

                    return Ok((
                        StatusCode::OK,
                        Json(json!({
                            "run_id": run_id,
                            "agent_id": agent_id,
                            "status": "completed",
                            "response": clean_text,
                        })),
                    ));
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response
                        .json::<Value>()
                        .await
                        .unwrap_or_else(|_| json!({ "status": "proxy_error" }));
                    return Ok((
                        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                        Json(body),
                    ));
                }
                Err(error) => {
                    tracing::warn!(agent_id, %error, "direct agent message proxy failed, falling back to background run");
                }
            }
        }
    }

    let prompt = build_agent_prompt(&agent_id, &req.message, req.context.as_ref());
    let run_id = spawn_background_run(&state, prompt, None, Some(agent_id.clone())).await;

    if let Some(completion) =
        wait_for_background_run(&state, &run_id, AGENT_MESSAGE_INLINE_TIMEOUT).await
    {
        return match completion {
            RunCompletion::Completed { response } => Ok((
                StatusCode::OK,
                Json(json!({
                    "run_id": run_id,
                    "agent_id": agent_id,
                    "status": "completed",
                    "response": response,
                })),
            )),
            RunCompletion::Failed { error } => Ok((
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "run_id": run_id,
                    "agent_id": agent_id,
                    "status": "failed",
                    "error": error,
                })),
            )),
        };
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "run_id": run_id,
            "agent_id": agent_id,
            "conversation_id": req.conversation_id,
            "response_mode": req.response_mode,
            "status": "running",
        })),
    ))
}

fn stream_url_for_agent(agent: &DiscoveredAgent) -> Option<String> {
    if let Some(url) = agent.endpoints.websocket.as_deref() {
        return Some(url.to_string());
    }

    let rest = agent.endpoints.rest.as_deref()?.trim_end_matches('/');
    if let Some(pathless) = rest.strip_prefix("https://") {
        return Some(format!("wss://{pathless}/stream"));
    }
    if let Some(pathless) = rest.strip_prefix("http://") {
        return Some(format!("ws://{pathless}/stream"));
    }
    None
}

fn spawn_sidecar_stream(
    state: Arc<AppState>,
    agent_id: String,
    ws_url: String,
    agent: &DiscoveredAgent,
    req: &SendMessageRequest,
) -> (
    String,
    oneshot::Receiver<std::result::Result<String, String>>,
) {
    let run_id = uuid::Uuid::new_v4().to_string();
    let prompt = req.message.clone();
    let token = agent.proxy_token.clone();
    let (tx, rx) = oneshot::channel();
    let run_id_for_task = run_id.clone();

    tokio::spawn(async move {
        let result =
            proxy_sidecar_stream(state, agent_id, run_id_for_task, ws_url, token, prompt).await;
        let _ = tx.send(result);
    });

    (run_id, rx)
}

async fn wait_for_sidecar_stream(
    rx: oneshot::Receiver<std::result::Result<String, String>>,
    timeout: Duration,
) -> Option<std::result::Result<String, String>> {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(result)) => Some(result),
        Ok(Err(_closed)) => Some(Err("stream task ended without a result".to_string())),
        Err(_elapsed) => None,
    }
}

async fn proxy_sidecar_stream(
    state: Arc<AppState>,
    agent_id: String,
    run_id: String,
    ws_url: String,
    token: Option<String>,
    prompt: String,
) -> std::result::Result<String, String> {
    state
        .event_bus
        .publish(crate::events::ServerEvent::AgentOutput {
            agent_id: agent_id.clone(),
            run_id: Some(run_id.clone()),
            content: String::new(),
            done: false,
            metadata: Some(json!({ "status": "started" })),
        });

    let mut request = ws_url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("build stream request: {error}"))?;
    if let Some(token) = token {
        let header_value = format!("Bearer {token}")
            .parse()
            .map_err(|error| format!("build authorization header: {error}"))?;
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, header_value);
    }

    let connect_timeout = state.load_roko_config().timeouts.health_check();
    let (mut socket, _response) =
        tokio::time::timeout(connect_timeout, connect_async(request))
            .await
            .map_err(|_| "connect sidecar stream timed out".to_string())?
            .map_err(|error| format!("connect sidecar stream: {error}"))?;
    socket
        .send(WsMessage::Text(prompt.into()))
        .await
        .map_err(|error| format!("send stream prompt: {error}"))?;

    let mut response_text = String::new();
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| format!("read stream message: {error}"))?;
        match message {
            WsMessage::Text(text) => {
                let value = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| {
                    json!({
                        "chunk": text.to_string(),
                        "done": false,
                    })
                });
                if let Some(error) = value.get("error").and_then(Value::as_str) {
                    state
                        .event_bus
                        .publish(crate::events::ServerEvent::AgentOutput {
                            agent_id,
                            run_id: Some(run_id),
                            content: String::new(),
                            done: true,
                            metadata: Some(json!({
                                "status": "failed",
                                "error": error,
                            })),
                        });
                    return Err(error.to_string());
                }
                if let Some(chunk) = stream_content_chunk(&value) {
                    response_text.push_str(chunk);
                    let clean_chunk = sanitize_agent_content(chunk);
                    if !clean_chunk.is_empty() {
                        state
                            .event_bus
                            .publish(crate::events::ServerEvent::AgentOutput {
                                agent_id: agent_id.clone(),
                                run_id: Some(run_id.clone()),
                                content: clean_chunk,
                                done: false,
                                metadata: None,
                            });
                    }
                    // Always emit the raw chunk as a trace event.
                    state
                        .event_bus
                        .publish(crate::events::ServerEvent::AgentTrace {
                            agent_id: agent_id.clone(),
                            run_id: Some(run_id.clone()),
                            content: chunk.to_string(),
                            tool_calls: value
                                .get("tool_calls")
                                .cloned()
                                .map(|v| if let Value::Array(a) = v { a } else { vec![v] }),
                            reasoning: value
                                .get("reasoning")
                                .and_then(Value::as_str)
                                .map(String::from),
                            usage: value.get("usage").cloned(),
                            done: false,
                        });
                }
                if value.get("done").and_then(Value::as_bool).unwrap_or(false) {
                    state
                        .event_bus
                        .publish(crate::events::ServerEvent::AgentOutput {
                            agent_id: agent_id.clone(),
                            run_id: Some(run_id.clone()),
                            content: String::new(),
                            done: true,
                            metadata: Some(json!({
                                "status": "completed",
                                "session": value.get("session").cloned().unwrap_or(Value::Null),
                                "usage": value.get("usage").cloned().unwrap_or(Value::Null),
                                "finish_reason": value
                                    .get("finish_reason")
                                    .cloned()
                                    .unwrap_or(Value::Null),
                            })),
                        });
                    // Final trace event with usage/session metadata.
                    state
                        .event_bus
                        .publish(crate::events::ServerEvent::AgentTrace {
                            agent_id,
                            run_id: Some(run_id),
                            content: String::new(),
                            tool_calls: None,
                            reasoning: None,
                            usage: value.get("usage").cloned(),
                            done: true,
                        });
                    return Ok(response_text);
                }
            }
            WsMessage::Close(_) => break,
            WsMessage::Ping(_)
            | WsMessage::Pong(_)
            | WsMessage::Binary(_)
            | WsMessage::Frame(_) => {}
        }
    }

    if response_text.is_empty() {
        Err("sidecar stream closed before producing output".to_string())
    } else {
        state
            .event_bus
            .publish(crate::events::ServerEvent::AgentOutput {
                agent_id,
                run_id: Some(run_id),
                content: String::new(),
                done: true,
                metadata: Some(json!({ "status": "completed" })),
            });
        Ok(response_text)
    }
}

fn stream_content_chunk(value: &Value) -> Option<&str> {
    value
        .get("chunk")
        .and_then(Value::as_str)
        .or_else(|| value.get("content").and_then(Value::as_str))
        .filter(|chunk| !chunk.is_empty())
}

enum RunCompletion {
    Completed { response: String },
    Failed { error: String },
}

async fn wait_for_background_run(
    state: &AppState,
    run_id: &str,
    timeout: Duration,
) -> Option<RunCompletion> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(completion) = background_run_completion(state, run_id).await {
            return Some(completion);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn background_run_completion(state: &AppState, run_id: &str) -> Option<RunCompletion> {
    let runs = state.active_runs.read().await;
    let handle = runs.get(run_id)?;
    match &handle.status {
        OperationStatus::Completed { result } => {
            let response = handle
                .result
                .as_ref()
                .and_then(|result| result.output_text.clone())
                .or_else(|| result.clone())
                .unwrap_or_default();
            Some(RunCompletion::Completed { response })
        }
        OperationStatus::Failed { error } => Some(RunCompletion::Failed {
            error: error.clone(),
        }),
        OperationStatus::Running => handle
            .handle
            .is_finished()
            .then(|| completed_from_finished_handle(handle.result.as_ref())),
    }
}

fn completed_from_finished_handle(result: Option<&RunResult>) -> RunCompletion {
    match result {
        Some(result) if result.success => RunCompletion::Completed {
            response: result.output_text.clone().unwrap_or_default(),
        },
        Some(result) => RunCompletion::Failed {
            error: result
                .output_text
                .clone()
                .unwrap_or_else(|| "run failed".to_string()),
        },
        None => RunCompletion::Failed {
            error: "run finished without recording a result".to_string(),
        },
    }
}

/// `POST /api/agents/{id}/token` — issue or rotate a bearer token.
async fn issue_token(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let issued = state
        .rotate_agent_token(&id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("agent {id} not found")))?;

    Ok(Json(json!(issued)))
}

/// `GET /api/agents/{id}/token` — check whether a token exists.
async fn token_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let status = state
        .agent_token_status(&id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("agent {id} not found")))?;
    Ok(Json(json!(status)))
}

/// Best-effort extraction of clean text from a sidecar response that may
/// contain raw Claude CLI streaming-protocol JSONL.
fn extract_response_text(raw: &str) -> String {
    let trimmed = raw.trim();
    // Fast path: not JSON at all → return as-is.
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return raw.to_string();
    }
    // Single JSON object with a `result` or `content` text field.
    if let Ok(obj) = serde_json::from_str::<Value>(trimmed) {
        if let Some(t) = obj
            .get("result")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return t.to_string();
        }
        if let Some(t) = obj
            .get("content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return t.to_string();
        }
    }
    // Multi-line JSONL (Claude CLI streaming protocol).
    if trimmed.contains('\n') {
        let mut parts = Vec::new();
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(obj) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let event_type = obj.get("type").and_then(Value::as_str);
            match event_type {
                Some("result") => {
                    if let Some(t) = obj
                        .get("result")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(t.to_string());
                    }
                }
                Some("assistant") => {
                    if let Some(blocks) = obj.pointer("/message/content").and_then(Value::as_array)
                    {
                        for block in blocks {
                            if block.get("type").and_then(Value::as_str) == Some("text") {
                                if let Some(t) = block.get("text").and_then(Value::as_str) {
                                    parts.push(t.to_string());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !parts.is_empty() {
            return parts.join("");
        }
    }
    raw.to_string()
}

fn build_agent_prompt(agent_id: &str, message: &str, context: Option<&Value>) -> String {
    let mut prompt = format!("[agent:{agent_id}] {message}");
    if let Some(context) = context {
        prompt.push_str("\n\nContext:\n");
        prompt.push_str(
            &serde_json::to_string_pretty(context).unwrap_or_else(|_| context.to_string()),
        );
    }
    prompt
}

fn validate_agent_url(url: &str) -> Result<(), ApiError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| ApiError::bad_request("invalid URL"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        s => {
            return Err(ApiError::bad_request(format!("unsupported scheme: {s}")));
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ApiError::bad_request("URL has no host"))?;

    if host.eq_ignore_ascii_case("localhost") {
        return Err(ApiError::bad_request("internal/private URLs not allowed"));
    }

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if blocked_agent_endpoint_ip(ip) {
            return Err(ApiError::bad_request("internal/private URLs not allowed"));
        }
    }

    Ok(())
}

fn blocked_agent_endpoint_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
    }
}

#[derive(Debug, Deserialize, Validate)]
struct RegisterAgentRequest {
    #[validate(
        length(min = 1, max = 128),
        custom(function = "crate::extract::validate_non_blank")
    )]
    agent_id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    process_id: Option<u64>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    domain_tags: Vec<String>,
    #[serde(default)]
    card_uri: Option<String>,
    #[serde(default)]
    rest_endpoint: Option<String>,
    #[serde(default)]
    websocket_endpoint: Option<String>,
    #[serde(default)]
    a2a_endpoint: Option<String>,
    #[serde(default)]
    mcp_endpoint: Option<String>,
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    reputation: u32,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    #[serde(alias = "pastJobsCompleted")]
    past_jobs_completed: u32,
    #[serde(default)]
    #[serde(alias = "maxConcurrentJobs")]
    max_concurrent_jobs: u32,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    issue_token: Option<bool>,
}

impl RequestPayload for RegisterAgentRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)?;
        if let Some(ref url) = self.rest_endpoint {
            validate_agent_url(url)?;
        }
        if let Some(ref url) = self.websocket_endpoint {
            validate_agent_url(url)?;
        }
        if let Some(ref url) = self.a2a_endpoint {
            validate_agent_url(url)?;
        }
        if let Some(ref url) = self.mcp_endpoint {
            validate_agent_url(url)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use axum::body::Body;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use roko_core::config::schema::RokoConfig;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    use crate::deploy::manual::ManualBackend;
    use crate::events::ServerEvent;
    use crate::runtime::NoOpRuntime;
    use crate::state::{AgentEndpoints, AgentRegistrationRecord, AppState};

    #[test]
    fn validate_agent_url_rejects_internal_and_private_hosts() {
        assert!(validate_agent_url("http://169.254.169.254/").is_err());
        assert!(validate_agent_url("http://10.0.0.1/").is_err());
        assert!(validate_agent_url("http://localhost/").is_err());
        assert!(validate_agent_url("http://127.0.0.1/").is_err());
        assert!(validate_agent_url("https://api.example.com/v1").is_ok());
    }

    // ---- T3-27: agent_manifest path-traversal & TOML-injection ---------

    #[test]
    fn agent_manifest_resolve_agent_dir_rejects_dotdot_segments() {
        let tempdir = tempdir().expect("tempdir");
        let agents_root = tempdir.path().join(".roko").join("agents");

        for hostile in [
            "..",
            "../",
            "../etc",
            "../../../etc",
            "..\\..\\windows",
            "/etc/passwd",
            "./hidden",
            "name/with/slashes",
            "",
            "   ",
        ] {
            let err = resolve_agent_dir(&agents_root, hostile)
                .err()
                .unwrap_or_else(|| panic!("expected rejection for {hostile:?}"));
            assert_eq!(
                err.status,
                StatusCode::BAD_REQUEST,
                "{hostile:?} should be 4xx, got {err:?}"
            );
        }
    }

    #[test]
    fn agent_manifest_resolve_agent_dir_accepts_simple_names() {
        let tempdir = tempdir().expect("tempdir");
        let agents_root = tempdir.path().join(".roko").join("agents");
        let resolved = resolve_agent_dir(&agents_root, "research-bot.v2").expect("resolve");
        let canonical_root = agents_root.canonicalize().expect("canonicalize root");
        assert_eq!(resolved, canonical_root.join("research-bot.v2"));
    }

    #[tokio::test]
    async fn agent_manifest_create_rejects_traversal_name() {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );

        let result = create_agent(
            State(Arc::clone(&state)),
            ValidJson(CreateAgentRequest {
                name: "../../../etc".into(),
                domain: "general".into(),
                prompt: None,
                skills: Vec::new(),
                tier: None,
                model: None,
                reputation: 0,
                max_concurrent_jobs: 0,
                capabilities: Vec::new(),
            }),
        )
        .await;

        let err = result.err().expect("traversal name must be rejected");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        // No directories should have been created outside the workspace.
        let escaped = tempdir.path().parent().unwrap().join("etc");
        assert!(
            !escaped.exists(),
            "traversal must not create {}",
            escaped.display()
        );
    }

    #[tokio::test]
    async fn agent_manifest_prompt_cannot_inject_toml_table() {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );

        let hostile_prompt = "innocent prompt\"\n[malicious]\nsecret = \"x\"\n# tail";
        let response = create_agent(
            State(Arc::clone(&state)),
            ValidJson(CreateAgentRequest {
                name: "injection-bot".into(),
                domain: "general".into(),
                prompt: Some(hostile_prompt.to_string()),
                skills: Vec::new(),
                tier: None,
                model: None,
                reputation: 0,
                max_concurrent_jobs: 0,
                capabilities: Vec::new(),
            }),
        )
        .await
        .expect("create_agent")
        .into_response();

        assert_eq!(response.status(), StatusCode::CREATED);

        let manifest_path = tempdir
            .path()
            .join(".roko")
            .join("agents")
            .join("injection-bot")
            .join("manifest.toml");
        let on_disk = std::fs::read_to_string(&manifest_path).expect("read manifest");
        let parsed: toml::Value = toml::from_str(&on_disk).expect("parse manifest");

        assert!(
            parsed.get("malicious").is_none(),
            "TOML injection succeeded, manifest contained `[malicious]`:\n{on_disk}"
        );
        let prompt = parsed
            .get("core")
            .and_then(|c| c.get("prompt"))
            .and_then(|p| p.as_str())
            .expect("core.prompt is a string");
        assert_eq!(prompt, hostile_prompt);
    }

    #[tokio::test]
    async fn agent_config_returns_manifest_and_runtime_metadata() {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );
        let agent_dir = tempdir.path().join(".roko").join("agents").join("demo");
        std::fs::create_dir_all(&agent_dir).expect("agent dir");
        std::fs::write(
            agent_dir.join("manifest.toml"),
            r#"
schema_version = 1

[core]
prompt = "hello"
mode = "self_hosted"
"#,
        )
        .expect("write manifest");

        let Json(payload) = get_agent_config(State(state), Path("demo".to_string()))
            .await
            .expect("agent config");

        assert_eq!(payload["agent_id"], "demo");
        assert_eq!(payload["manifest_exists"], true);
        assert_eq!(payload["runtime"]["process_status"], "stopped");
        assert_eq!(payload["manifest"]["core"]["prompt"], "hello");
    }

    #[tokio::test]
    async fn agent_config_rejects_path_traversal_ids() {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );

        let err = get_agent_config(State(state), Path("../escape".to_string()))
            .await
            .expect_err("path traversal id should be rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[derive(Debug)]
    struct MockLogsServerState {
        response: Value,
        status: StatusCode,
        seen_tails: Mutex<Vec<Option<usize>>>,
    }

    async fn mock_logs_handler(
        State(state): State<Arc<MockLogsServerState>>,
        Query(query): Query<LogsQuery>,
    ) -> impl IntoResponse {
        state.seen_tails.lock().await.push(query.tail);
        (state.status, Json(state.response.clone()))
    }

    async fn spawn_mock_logs_server(
        status: StatusCode,
        response: Value,
    ) -> Result<(
        String,
        Arc<MockLogsServerState>,
        tokio::task::JoinHandle<()>,
    )> {
        let state = Arc::new(MockLogsServerState {
            response,
            status,
            seen_tails: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .route("/logs", get(mock_logs_handler))
            .with_state(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| anyhow!("failed to bind mock logs listener: {error}"))?;
        let addr = listener
            .local_addr()
            .map_err(|error| anyhow!("failed to read mock logs address: {error}"))?;
        let handle = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                panic!("mock logs server stopped unexpectedly: {error}");
            }
        });
        Ok((format!("http://{addr}"), state, handle))
    }

    fn router(state: Arc<AppState>) -> Router {
        Router::new().nest("/api", routes()).with_state(state)
    }

    async fn json_body(response: axum::response::Response) -> Result<Value> {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|error| anyhow!("failed to read response body bytes: {error}"))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| anyhow!("failed to parse JSON response body: {error}"))
    }

    #[test]
    fn build_agent_prompt_embeds_context() {
        let prompt = build_agent_prompt(
            "agent-7",
            "hello",
            Some(&json!({ "task": "research", "priority": "high" })),
        );

        assert!(prompt.starts_with("[agent:agent-7] hello"));
        assert!(prompt.contains("\"task\": \"research\""));
    }

    #[tokio::test]
    async fn send_message_creates_tracked_run_and_events() {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );

        let response = send_message(
            State(Arc::clone(&state)),
            Path("agent-1".to_string()),
            ValidJson(SendMessageRequest {
                message: "hello".into(),
                context: Some(json!({ "source": "dashboard" })),
                conversation_id: Some("conv-1".into()),
                response_mode: Some("stream".into()),
            }),
        )
        .await
        .expect("send message")
        .into_response();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let payload: Value = serde_json::from_slice(&body).expect("json body");
        let run_id = payload["run_id"].as_str().expect("run_id").to_string();
        assert_eq!(payload["status"], "completed");

        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(state.active_runs.read().await.contains_key(&run_id));
        let events = state.event_bus.replay_from(0);
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            ServerEvent::RunStarted { run_id: event_run_id, .. } if event_run_id == &run_id
        )));
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            ServerEvent::AgentOutput {
                agent_id,
                run_id: Some(event_run_id),
                done: true,
                ..
            } if agent_id == "agent-1" && event_run_id == &run_id
        )));
    }

    #[tokio::test]
    async fn register_and_issue_token() {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );

        let _ = register_agent(
            State(Arc::clone(&state)),
            ValidJson(RegisterAgentRequest {
                agent_id: "agent-2".into(),
                label: Some("agent-two".into()),
                process_id: None,
                owner: Some("owner".into()),
                capabilities: vec!["research".into()],
                domain_tags: vec!["roko".into()],
                card_uri: None,
                rest_endpoint: Some("https://example.com:9001".into()),
                websocket_endpoint: None,
                a2a_endpoint: None,
                mcp_endpoint: None,
                tier: None,
                model: None,
                reputation: 0,
                skills: Vec::new(),
                past_jobs_completed: 0,
                max_concurrent_jobs: 0,
                issue_token: Some(true),
            }),
        )
        .await
        .expect("register");

        let status = state
            .agent_token_status("agent-2")
            .await
            .expect("token status");
        assert!(status.exists);
    }

    #[tokio::test]
    async fn managed_agents_include_model_with_default_fallback() {
        let tempdir = tempdir().expect("tempdir");
        let mut config = RokoConfig::default();
        config.agent.default_model = "claude-sonnet-4-20250514".into();
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                config,
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );

        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "fallback-agent".into(),
                label: Some("fallback-agent".into()),
                capabilities: vec!["coding".into()],
                ..Default::default()
            })
            .await;
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "override-agent".into(),
                label: Some("override-agent".into()),
                capabilities: vec!["research".into()],
                model: Some("custom-model".into()),
                ..Default::default()
            })
            .await;
        let mut metrics = HashMap::new();
        metrics.insert("gate_pass_rate".to_string(), 0.93);
        metrics.insert("token_burn_rate".to_string(), 42.0);
        metrics.insert("cumulative_cost_usd".to_string(), 1.25);
        state.heartbeats.write().await.push_back(HeartbeatPayload {
            sender_id: "fallback-agent".into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            active_tasks: 1,
            completed_tasks: 7,
            failed_tasks: 1,
            active_agents: 2,
            frequency: 0.5,
            metrics,
        });

        let Json(payload) = list_managed_agents(State(state)).await;
        let agents = payload.as_array().expect("agents array");
        let fallback = agents
            .iter()
            .find(|agent| agent["id"] == "fallback-agent")
            .expect("fallback agent");
        let override_agent = agents
            .iter()
            .find(|agent| agent["id"] == "override-agent")
            .expect("override agent");

        assert_eq!(fallback["model"], "claude-sonnet-4-20250514");
        assert_eq!(fallback["model_source"], "default");
        assert_eq!(fallback["provider"], "claude_cli");
        assert_eq!(
            fallback["model_profile"]["slug"],
            "claude-sonnet-4-20250514"
        );
        assert_eq!(fallback["heartbeat"]["active_tasks"], 1);
        assert_eq!(fallback["performance"]["gate_pass_rate"], 0.93);
        assert_eq!(fallback["costs"]["cumulative_usd"], 1.25);
        assert_eq!(fallback["chat"]["correlation"], "run_id");
        assert_eq!(override_agent["model"], "custom-model");
        assert_eq!(override_agent["model_source"], "agent");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_logs_proxy_forwards_tail_and_body() -> std::result::Result<(), Box<dyn Error>> {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );
        let (logs_url, logs_state, _handle) = spawn_mock_logs_server(
            StatusCode::OK,
            json!({
                "lines": ["alpha", "bravo"],
                "path": "/tmp/agent.log",
            }),
        )
        .await?;
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-logs".into(),
                label: Some("agent-logs".into()),
                process_id: None,
                owner: String::new(),
                endpoints: AgentEndpoints {
                    rest: Some(logs_url),
                    websocket: None,
                    a2a: None,
                    mcp: None,
                },
                card_uri: None,
                capabilities: Vec::new(),
                domain_tags: Vec::new(),
                ..Default::default()
            })
            .await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/agents/agent-logs/logs?tail=2")
                    .body(Body::empty())
                    .map_err(|error| anyhow!("failed to build agent logs request: {error}"))?,
            )
            .await
            .map_err(|error| anyhow!("agent logs request failed: {error}"))?;

        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_body(response).await?;
        assert_eq!(payload["lines"], json!(["alpha", "bravo"]));
        assert_eq!(payload["path"], "/tmp/agent.log");

        let seen_tails = logs_state.seen_tails.lock().await;
        assert_eq!(seen_tails.as_slice(), &[Some(2)]);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_logs_missing_agent_returns_404() -> std::result::Result<(), Box<dyn Error>> {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );

        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/agents/missing/logs")
                    .body(Body::empty())
                    .map_err(|error| {
                        anyhow!("failed to build missing-agent logs request: {error}")
                    })?,
            )
            .await
            .map_err(|error| anyhow!("missing-agent logs request failed: {error}"))?;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let payload = json_body(response).await?;
        assert_eq!(payload["code"], "not_found");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_logs_sidecar_not_found_is_propagated() -> std::result::Result<(), Box<dyn Error>>
    {
        let tempdir = tempdir().expect("tempdir");
        let state = Arc::new(
            AppState::new(
                tempdir.path().to_path_buf(),
                Arc::new(NoOpRuntime),
                RokoConfig::default(),
                Arc::new(ManualBackend::default()),
            )
            .expect("AppState::new"),
        );
        let (logs_url, logs_state, _handle) = spawn_mock_logs_server(
            StatusCode::NOT_FOUND,
            json!({ "error": "log file missing" }),
        )
        .await?;
        state
            .upsert_discovered_agent(AgentRegistrationRecord {
                agent_id: "agent-404".into(),
                label: Some("agent-404".into()),
                process_id: None,
                owner: String::new(),
                endpoints: AgentEndpoints {
                    rest: Some(logs_url),
                    websocket: None,
                    a2a: None,
                    mcp: None,
                },
                card_uri: None,
                capabilities: Vec::new(),
                domain_tags: Vec::new(),
                ..Default::default()
            })
            .await;

        let response = router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/agents/agent-404/logs?tail=9")
                    .body(Body::empty())
                    .map_err(|error| {
                        anyhow!("failed to build sidecar-404 logs request: {error}")
                    })?,
            )
            .await
            .map_err(|error| anyhow!("sidecar-404 logs request failed: {error}"))?;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let payload = json_body(response).await?;
        assert_eq!(payload["error"], "log file missing");

        let seen_tails = logs_state.seen_tails.lock().await;
        assert_eq!(seen_tails.as_slice(), &[Some(9)]);
        Ok(())
    }
}
