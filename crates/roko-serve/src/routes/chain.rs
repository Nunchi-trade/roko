//! On-chain proxy routes.
//!
//! These endpoints read from the mirage-rs / anvil chain via the alloy backend
//! so that the dashboard can fetch chain data through roko-serve (single origin,
//! no CORS issues).

use std::sync::Arc;

use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::sol;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::AppState;

// Minimal sol! bindings — just the view functions we need.
sol! {
    #[sol(rpc)]
    contract AgentRegistryReader {
        function registeredCount() external view returns (uint256);
        function isActive(address agent) external view returns (bool);
    }

    #[sol(rpc)]
    contract BountyMarketReader {
        function nextJobId() external view returns (uint256);
        function stateOf(uint256 id) external view returns (uint8);
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/chain/agents", get(chain_agents))
        .route("/chain/bounties", get(chain_bounties))
        .route("/chain/status", get(chain_status))
        .route("/chain/blocks", get(chain_blocks))
        .route("/chain/transactions", get(chain_txs))
        .route("/chain/events", get(chain_events))
        .route("/chain/watcher", get(chain_watcher_status))
}

/// `GET /api/chain/agents` — read on-chain agent count and liveness info.
async fn chain_agents(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let client = state
        .chain_client
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("chain client not configured"))?;

    let config = state.load_roko_config();
    let registry_addr: Address = config
        .chain
        .agent_registry
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("[chain].agent_registry not configured in roko.toml"))?
        .parse()
        .map_err(|e| ApiError::bad_request(format!("invalid agent_registry address: {e}")))?;

    let registry = AgentRegistryReader::new(registry_addr, client.provider());

    // Single-return-value sol! calls return the raw type directly.
    let count: U256 = registry
        .registeredCount()
        .call()
        .await
        .map_err(|e| ApiError::internal(format!("registeredCount call failed: {e}")))?;

    Ok(Json(json!({
        "source": "on-chain",
        "agent_registry": format!("{registry_addr:#x}"),
        "registered_count": count.to_string(),
    })))
}

/// `GET /api/chain/bounties` — read on-chain bounty/job info.
async fn chain_bounties(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let client = state
        .chain_client
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("chain client not configured"))?;

    let config = state.load_roko_config();
    let market_addr: Address = config
        .chain
        .bounty_market
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("[chain].bounty_market not configured in roko.toml"))?
        .parse()
        .map_err(|e| ApiError::bad_request(format!("invalid bounty_market address: {e}")))?;

    let market = BountyMarketReader::new(market_addr, client.provider());

    let next_id: U256 = market
        .nextJobId()
        .call()
        .await
        .map_err(|e| ApiError::internal(format!("nextJobId call failed: {e}")))?;

    // Enumerate open jobs (state < 5 means not yet terminal).
    let total: u64 = next_id.to::<u64>();
    let mut jobs = Vec::new();
    for jid in 0..total.min(100) {
        match market.stateOf(U256::from(jid)).call().await {
            Ok(s) => {
                jobs.push(json!({
                    "id": jid,
                    "state": s,
                    "state_label": job_state_label(s),
                }));
            }
            Err(_) => continue,
        }
    }

    Ok(Json(json!({
        "source": "on-chain",
        "bounty_market": format!("{market_addr:#x}"),
        "total_jobs": total,
        "jobs": jobs,
    })))
}

/// `GET /api/chain/status` — basic chain connectivity check.
async fn chain_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let client = state
        .chain_client
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("chain client not configured"))?;

    let block = client
        .provider()
        .get_block_number()
        .await
        .map_err(|e| ApiError::internal(format!("block_number call failed: {e}")))?;

    let chain_id = client
        .provider()
        .get_chain_id()
        .await
        .map_err(|e| ApiError::internal(format!("chain_id call failed: {e}")))?;

    let wallet_addr = state
        .chain_wallet
        .as_ref()
        .map(|w| format!("{:#x}", w.address_typed()));

    Ok(Json(json!({
        "connected": true,
        "block_number": block,
        "chain_id": chain_id,
        "wallet": wallet_addr,
    })))
}

/// Query params for chain list endpoints.
#[derive(Debug, Deserialize)]
struct ChainListParams {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    64
}

/// `GET /api/chain/blocks` — recent blocks from ring buffer.
async fn chain_blocks(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ChainListParams>,
) -> Result<Json<Value>, ApiError> {
    let ring = state.chain.recent_blocks.read().await;
    let limit = params.limit.min(64);
    let blocks: Vec<_> = ring.iter().rev().take(limit).collect();
    Ok(Json(json!({ "blocks": blocks })))
}

/// `GET /api/chain/transactions` — recent transactions from ring buffer.
async fn chain_txs(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ChainListParams>,
) -> Result<Json<Value>, ApiError> {
    let ring = state.chain.recent_txs.read().await;
    let limit = params.limit.min(128);
    let txs: Vec<_> = ring.iter().rev().take(limit).collect();
    Ok(Json(json!({ "transactions": txs })))
}

/// `GET /api/chain/events` — recent decoded contract events from ring buffer.
async fn chain_events(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ChainListParams>,
) -> Result<Json<Value>, ApiError> {
    let ring = state.chain.recent_events.read().await;
    let limit = params.limit.min(128);
    let events: Vec<_> = ring.iter().rev().take(limit).collect();
    Ok(Json(json!({ "events": events })))
}

/// `GET /api/chain/watcher` — watcher running status.
async fn chain_watcher_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let running = state
        .chain
        .watcher_running
        .load(std::sync::atomic::Ordering::Relaxed);
    let latest = state.chain.latest_block.read().await;
    let latest_number = latest.as_ref().map(|b| b.number);
    let blocks_buffered = state.chain.recent_blocks.read().await.len();
    let txs_buffered = state.chain.recent_txs.read().await.len();
    let events_buffered = state.chain.recent_events.read().await.len();

    Ok(Json(json!({
        "watcher_running": running,
        "latest_block": latest_number,
        "blocks_buffered": blocks_buffered,
        "txs_buffered": txs_buffered,
        "events_buffered": events_buffered,
    })))
}

fn job_state_label(state: u8) -> &'static str {
    match state {
        0 => "None",
        1 => "Open",
        2 => "Funded",
        3 => "Assigned",
        4 => "Submitted",
        5 => "Resolved",
        _ => "Unknown",
    }
}
