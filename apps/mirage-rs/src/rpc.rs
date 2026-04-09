//! JSON-RPC server surface for `mirage-rs`.

#![allow(
    clippy::default_trait_access,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::significant_drop_tightening,
    clippy::too_many_lines,
    clippy::uninlined_format_args
)]

use std::{
    convert::Infallible,
    net::{SocketAddr, ToSocketAddrs},
    num::NonZeroUsize,
    sync::Arc,
    time::Duration,
};

use alloy_primitives::{Address, B256, Bytes, U256, hex, keccak256};
use axum::{
    Router,
    body::Body,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{Request, Response, StatusCode},
    response::IntoResponse,
    routing::{delete, get},
};
use jsonrpsee::{
    RpcModule,
    core::{RegisterMethodError, SubscriptionError},
    server::{ServerBuilder, ServerHandle},
    types::ErrorObjectOwned,
};
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tower::Service;
use tower_http::cors::CorsLayer;

use crate::{
    Bytecode, MirageError, Result, TransactionRequest,
    events::MirageTelemetryEvent,
    fork::{
        ClassificationConfig, DiffClassifier, EvmExecutor, ForkState, HybridDB, LocalBlock,
        LocalReceipt, LocalTransaction, MirageFork, MirageState, WatchSource, lock_state_writes,
        with_state_write,
    },
    integration::{
        EventFilter, EventSource, MIRAGE_BEGIN_SCENARIO_SET_METHOD, MIRAGE_DEFINE_SCENARIO_METHOD,
        MIRAGE_GET_POSITION_METHOD, MIRAGE_GET_RESOURCE_USAGE_METHOD,
        MIRAGE_GET_SCENARIO_RESULTS_METHOD, MIRAGE_RUN_SCENARIO_SET_METHOD, MIRAGE_SHUTDOWN_METHOD,
        MIRAGE_STATUS_METHOD, MIRAGE_SUBSCRIBE_EVENTS_METHOD, MIRAGE_WATCH_CONTRACT_METHOD,
        MirageEvent, PositionRequest, PositionSnapshot,
    },
    provider::{BlockTag, UpstreamRpc},
    resources::{MirageMode, PressureAction, Profile, ResourceModel},
    scenario::{
        JobStatus, RunMode, Scenario, ScenarioJob, ScenarioRunner, ScenarioSet, ScenarioSetStatus,
        rank_scenario_results,
    },
};

#[derive(Clone)]
struct ServerContext {
    state: Arc<RwLock<MirageState>>,
    shutdown: broadcast::Sender<()>,
    /// Optional chain substrate for `chain_*` RPC methods. Present iff the
    /// server was started via `start_rpc_server_with_chain`.
    #[cfg(feature = "chain")]
    chain: Option<Arc<RwLock<crate::chain_rpc::ChainContext>>>,
    /// Optional subscription manager for `chain_subscribe*` WS streams
    /// (§38.d). Present when the attached `ChainContext` carries buses.
    #[cfg(feature = "roko")]
    chain_subs: Option<crate::chain_rpc::SubscriptionManager>,
}

#[derive(Debug)]
struct StagedErc20Mint {
    owner: Address,
    balance: U256,
    balance_slot: Option<U256>,
    storage_writes: Vec<(U256, U256)>,
}

/// Starts a JSON-RPC server on the provided address.
///
/// When the `chain` feature is enabled, use [`start_rpc_server_with_chain`] to
/// attach an [`crate::chain_rpc::ChainContext`] that exposes `chain_*` methods.
pub async fn start_rpc_server(
    address: impl ToSocketAddrs,
    mirage: MirageFork,
    shutdown: broadcast::Sender<()>,
) -> Result<(SocketAddr, ServerHandle)> {
    let address = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| MirageError::Unsupported("no socket address resolved".to_owned()))?;
    let local_state = mirage.state();
    let module = build_rpc_module(ServerContext {
        state: Arc::clone(&local_state),
        shutdown,
        #[cfg(feature = "chain")]
        chain: None,
        #[cfg(feature = "roko")]
        chain_subs: None,
    })
    .map_err(|error| MirageError::Unsupported(error.to_string()))?;
    finish_start_rpc_server(address, module, local_state, None).await
}

/// Starts a JSON-RPC server with an attached chain substrate.
///
/// Registers all `chain_*` methods described in [`crate::chain_rpc`] on top
/// of the standard `eth_*` / `mirage_*` surface.
#[cfg(feature = "chain")]
pub async fn start_rpc_server_with_chain(
    address: impl ToSocketAddrs,
    mirage: MirageFork,
    shutdown: broadcast::Sender<()>,
    chain: Arc<RwLock<crate::chain_rpc::ChainContext>>,
) -> Result<(SocketAddr, ServerHandle)> {
    let address = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| MirageError::Unsupported("no socket address resolved".to_owned()))?;
    let local_state = mirage.state();
    #[cfg(feature = "roko")]
    let chain_subs = {
        let guard = chain.read();
        match (guard.pheromone_bus.clone(), guard.insight_bus.clone()) {
            (Some(p), Some(i)) => Some(crate::chain_rpc::SubscriptionManager::new(p, i)),
            _ => None,
        }
    };
    let api_router = {
        let api_state = crate::http_api::ApiState {
            chain: chain.clone(),
            mirage_state: local_state.clone(),
            projection_cache: crate::http_api::ProjectionCache::new(4096),
            started_at: std::time::Instant::now(),
            #[cfg(feature = "roko")]
            subs: chain_subs.clone(),
        };
        Some(crate::http_api::build_router(api_state))
    };
    let module = build_rpc_module(ServerContext {
        state: Arc::clone(&local_state),
        shutdown,
        chain: Some(chain),
        #[cfg(feature = "roko")]
        chain_subs,
    })
    .map_err(|error| MirageError::Unsupported(error.to_string()))?;
    finish_start_rpc_server(address, module, local_state, api_router).await
}

/// Middleware that adds `Cache-Control: no-cache` headers to `/dashboard` responses.
async fn dashboard_cache_control(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response<Body> {
    let is_dashboard = request.uri().path().starts_with("/dashboard");
    let mut response = next.run(request).await;
    if is_dashboard {
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache, must-revalidate"),
        );
    }
    response
}

async fn finish_start_rpc_server(
    address: SocketAddr,
    module: RpcModule<ServerContext>,
    local_state: Arc<RwLock<MirageState>>,
    api_router: Option<Router>,
) -> Result<(SocketAddr, ServerHandle)> {
    let (stop_handle, server_handle) = jsonrpsee::server::stop_channel();
    let rpc_service = ServerBuilder::default()
        .to_service_builder()
        .build(module, stop_handle);
    let rpc_fallback = tower::service_fn(move |request: Request<Body>| {
        let mut rpc_service = rpc_service.clone();
        async move {
            match Service::call(&mut rpc_service, request).await {
                Ok(response) => Ok::<Response<Body>, Infallible>(response.map(Body::new)),
                Err(error) => {
                    tracing::warn!("jsonrpsee rpc service failed: {error}");
                    let mut response =
                        Response::new(Body::from("internal server error".to_owned()));
                    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    Ok(response)
                }
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|_| MirageError::BindFailed(address.port()))?;
    let local_addr = listener
        .local_addr()
        .map_err(|_| MirageError::BindFailed(address.port()))?;
    let mut app = Router::new()
        .route("/health", get(health_handler))
        .route("/events/{stream_id}", get(event_ws_handler))
        .route("/events/{stream_id}", delete(unsubscribe_event_handler))
        .with_state(local_state);
    if let Some(api) = api_router {
        app = app.nest("/api", api);
    }
    // Fallback MUST be registered after /api nest — otherwise Axum's fallback
    // catches /api/* requests before the nested router can match them, causing
    // all REST endpoints to return the JSON-RPC "POST is required" error.
    app = app.fallback_service(rpc_fallback);
    // Serve the dashboard UI from the static/ directory if present.
    // Checks: $MIRAGE_DASHBOARD_DIR, ./static/, and the binary's sibling static/.
    let dashboard_dir = std::env::var("MIRAGE_DASHBOARD_DIR").ok().or_else(|| {
        let candidates = [
            std::path::PathBuf::from("static"),
            std::path::PathBuf::from("apps/mirage-rs/static"),
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("static")))
                .unwrap_or_default(),
        ];
        candidates
            .into_iter()
            .find(|p| p.join("index.html").exists())
            .map(|p| p.to_string_lossy().into_owned())
    });
    if let Some(dir) = dashboard_dir {
        let serve_dir =
            tower_http::services::ServeDir::new(&dir).append_index_html_on_directories(true);
        app = app.nest_service("/dashboard", serve_dir);
        // Add no-cache middleware for dashboard static files
        app = app.layer(axum::middleware::from_fn(dashboard_cache_control));
        tracing::info!("dashboard UI served at /dashboard from {dir}");
    }
    let app = app.layer(CorsLayer::permissive());
    let shutdown_handle = server_handle.clone();
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(shutdown_handle.stopped())
            .await
        {
            tracing::warn!("mirage http server exited with error: {error}");
        }
    });
    Ok((local_addr, server_handle))
}

/// Starts an ephemeral server for tests.
pub async fn spawn_rpc_server_for_tests() -> Result<(String, ServerHandle)> {
    let upstream = Arc::new(UpstreamRpc::mock(1));
    let db = HybridDB::new(upstream, 64, Duration::from_secs(12), NonZeroUsize::MIN, 1);
    let fork = ForkState::new(db, 0, 1);
    let mirage = MirageFork::new(
        fork,
        ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
        MirageMode::Live,
    );
    let (shutdown, _) = broadcast::channel(4);
    let (addr, handle) = start_rpc_server("127.0.0.1:0", mirage, shutdown).await?;
    Ok((format!("http://{addr}"), handle))
}

/// Maps an Ethereum JSON-RPC block selector to [`ForkState`]'s pinned upstream block for reads.
fn apply_eth_block_param(fork: &mut ForkState, block: &serde_json::Value) -> Result<()> {
    fork.db.pinned_block = match block {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => {
            let s = s.trim();
            if matches!(s, "latest" | "pending" | "finalized" | "safe") {
                None
            } else if s == "earliest" {
                Some(0)
            } else {
                Some(parse_hex_quantity(s)?)
            }
        }
        serde_json::Value::Number(n) => {
            let n = n.as_u64().ok_or_else(|| {
                MirageError::InvalidParams("block number does not fit in u64".to_owned())
            })?;
            Some(n)
        }
        _ => {
            return Err(MirageError::InvalidParams(
                "invalid block parameter".to_owned(),
            ));
        }
    };
    Ok(())
}

/// Resolves bytecode for `eth_getCode`: prefer embedded code, else [`HybridDB::code_by_hash`].
fn bytecode_for_eth_get_code(fork: &mut ForkState, address: Address) -> Result<Bytecode> {
    let info = fork.db.basic(address)?.unwrap_or_default();
    if let Some(code) = info.code {
        return Ok(code);
    }
    if info.code_hash.is_zero() {
        return Ok(Bytecode::default());
    }
    fork.db.code_by_hash(info.code_hash)
}

/// Stub `eth_feeHistory` payload with array lengths matching EIP-1559 client expectations.
fn build_fee_history_response(
    block_count_raw: serde_json::Value,
    _newest_block: serde_json::Value,
    reward_percentiles: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let block_count = match block_count_raw {
        serde_json::Value::Number(n) => {
            let raw = n
                .as_u64()
                .ok_or_else(|| MirageError::InvalidParams("feeHistory blockCount".to_owned()))?;
            usize::try_from(raw.clamp(1, 1024))
                .map_err(|_| MirageError::InvalidParams("feeHistory blockCount".to_owned()))?
        }
        serde_json::Value::String(s) => {
            let raw = parse_hex_quantity(s.trim())?;
            usize::try_from(raw.clamp(1, 1024))
                .map_err(|_| MirageError::InvalidParams("feeHistory blockCount".to_owned()))?
        }
        _ => {
            return Err(MirageError::InvalidParams(
                "feeHistory blockCount must be a number or hex quantity".to_owned(),
            ));
        }
    };
    let reward_tiers = reward_percentiles
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|a| a.len().max(1))
        .unwrap_or(1);
    let base_fee_per_gas: Vec<String> = (0..=block_count).map(|_| "0x1".to_owned()).collect();
    let gas_used_ratio: Vec<f64> = std::iter::repeat_n(0.5, block_count).collect();
    let reward: Vec<Vec<String>> = std::iter::repeat_n(
        std::iter::repeat_n("0x0".to_owned(), reward_tiers).collect(),
        block_count,
    )
    .collect();
    Ok(serde_json::json!({
        "oldestBlock": "0x0",
        "baseFeePerGas": base_fee_per_gas,
        "gasUsedRatio": gas_used_ratio,
        "reward": reward,
    }))
}

fn build_rpc_module(
    context: ServerContext,
) -> std::result::Result<RpcModule<ServerContext>, RegisterMethodError> {
    let mut module = RpcModule::new(context);

    module.register_async_method("web3_clientVersion", |_params, _ctx, _| async {
        Ok::<_, ErrorObjectOwned>("mirage-rs/2.0.0".to_owned())
    })?;

    module.register_async_method("net_version", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(state.fork.chain_id.to_string())
    })?;

    module.register_async_method("eth_chainId", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(hex_u64(state.fork.chain_id))
    })?;

    module.register_async_method("eth_blockNumber", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(hex_u64(state.fork.local_block_number))
    })?;

    module.register_async_method("eth_gasPrice", |_params, _ctx, _| async {
        Ok::<_, ErrorObjectOwned>("0x1".to_owned())
    })?;

    module.register_async_method("eth_maxPriorityFeePerGas", |_params, _ctx, _| async {
        Ok::<_, ErrorObjectOwned>("0x0".to_owned())
    })?;

    module.register_async_method("eth_feeHistory", |params, _ctx, _| async move {
        let args: Vec<serde_json::Value> = params.parse().map_err(invalid_params)?;
        if args.is_empty() {
            return Err(invalid_params_message("eth_feeHistory requires blockCount"));
        }
        let block_count = args[0].clone();
        let newest_block = args.get(1).cloned().unwrap_or(serde_json::json!("latest"));
        let reward_pct = args.get(2).cloned();
        let json =
            build_fee_history_response(block_count, newest_block, reward_pct).map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(json)
    })?;

    module.register_async_method("eth_getBalance", |params, ctx, _| async move {
        let (address, block): (Address, serde_json::Value) =
            params.parse().map_err(invalid_params)?;
        let balance = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            Ok(fork.db.basic(address)?.unwrap_or_default().balance)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(hex_u256(balance))
    })?;

    module.register_async_method("eth_getTransactionCount", |params, ctx, _| async move {
        let (address, block): (Address, serde_json::Value) =
            params.parse().map_err(invalid_params)?;
        let nonce = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            Ok(fork.db.basic(address)?.unwrap_or_default().nonce)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(hex_u64(nonce))
    })?;

    module.register_async_method("eth_getStorageAt", |params, ctx, _| async move {
        let (address, slot, block): (Address, U256, serde_json::Value) =
            params.parse().map_err(invalid_params)?;
        let value = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            fork.db.storage(address, slot)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(format!("0x{:064x}", value))
    })?;

    module.register_async_method("eth_getCode", |params, ctx, _| async move {
        let (address, block): (Address, serde_json::Value) =
            params.parse().map_err(invalid_params)?;
        let code = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            bytecode_for_eth_get_code(&mut fork, address)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(format!("0x{}", hex::encode(code.bytecode())))
    })?;

    module.register_async_method("eth_call", |params, ctx, _| async move {
        // Accept `[tx]` or `[tx, blockTag]`. `from` is optional (defaults to
        // zero-address) to match the JSON-RPC spec's view-call semantics.
        let (request, block): (TransactionRequest, serde_json::Value) = {
            let raw: serde_json::Value = params.parse().map_err(invalid_params)?;
            let arr = raw
                .as_array()
                .ok_or_else(|| invalid_params_message("eth_call: expected array"))?
                .clone();
            let first = arr.first().cloned().unwrap_or(serde_json::json!({}));
            let tx: TransactionRequest = serde_json::from_value(first).map_err(invalid_params)?;
            let blk = arr
                .get(1)
                .cloned()
                .unwrap_or_else(|| serde_json::json!("latest"));
            (tx, blk)
        };
        let from = request.from.unwrap_or(Address::ZERO);
        let to = extract_to(request.to).ok_or_else(|| invalid_params_message("missing to"))?;
        let data = request.data.unwrap_or_default();
        let value = request.value.unwrap_or(U256::ZERO);
        // Default to block gas limit for view calls; 21_000 is too low for
        // anything that touches storage.
        let gas = request.gas.unwrap_or(30_000_000);
        let result = run_fork_snapshot(&ctx.state, false, move |mut fork| {
            apply_eth_block_param(&mut fork, &block)?;
            EvmExecutor::call(&fork, from, to, data, value, gas)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(format!("0x{}", hex::encode(&result.output)))
    })?;

    module.register_async_method("eth_estimateGas", |params, ctx, _| async move {
        // Accept either `[tx]` or `[tx, blockTag]` — some clients (alloy) always send
        // the block tag, others (cast) omit it. Block tag is ignored: estimation is
        // always against the pending tip in mirage.
        let request: TransactionRequest = {
            let raw: serde_json::Value = params.parse().map_err(invalid_params)?;
            match raw {
                serde_json::Value::Array(mut arr) if !arr.is_empty() => {
                    let first = arr.remove(0);
                    serde_json::from_value(first).map_err(invalid_params)?
                }
                serde_json::Value::Object(_) => {
                    serde_json::from_value(raw).map_err(invalid_params)?
                }
                _ => {
                    return Err(invalid_params(MirageError::InvalidParams(
                        "eth_estimateGas: expected [tx] or [tx, blockTag]".to_owned(),
                    )));
                }
            }
        };
        let state = Arc::clone(&ctx.state);
        let fork = { state.read().fork.clone() };
        let executor = Arc::clone(&state.read().speculative_executor);
        let result = tokio::task::spawn_blocking(move || {
            let mut exec = executor.lock();
            exec.execute(&fork, &request)
        })
        .await
        .map_err(|error| rpc_error(MirageError::BackgroundTask(error.to_string())))?
        .map_err(rpc_error)?;
        let estimate = result.state_diff.gas_used.saturating_mul(12) / 10;
        Ok::<_, ErrorObjectOwned>(hex_u64(estimate.max(21_000)))
    })?;

    module.register_async_method("eth_sendTransaction", |params, ctx, _| async move {
        let (request,): (TransactionRequest,) = params.parse().map_err(invalid_params)?;
        let tx_hash = commit_transaction_request(&ctx.state, request, None)
            .await
            .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(tx_hash)
    })?;

    module.register_async_method("eth_sendRawTransaction", |params, ctx, _| async move {
        let (raw,): (Bytes,) = params.parse().map_err(invalid_params)?;
        let decoded = decode_signed_raw_transaction(&raw).map_err(rpc_error)?;
        let tx_hash =
            commit_transaction_request(&ctx.state, decoded.request, Some(decoded.tx_hash))
                .await
                .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(tx_hash)
    })?;

    module.register_async_method("eth_getTransactionReceipt", |params, ctx, _| async move {
        let (tx_hash,): (B256,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let receipt = state.fork.receipts.get(&tx_hash).map(receipt_json);
        Ok::<_, ErrorObjectOwned>(receipt)
    })?;

    module.register_async_method("eth_getTransactionByHash", |params, ctx, _| async move {
        let (tx_hash,): (B256,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let tx = state.fork.transactions.get(&tx_hash).map(transaction_json);
        Ok::<_, ErrorObjectOwned>(tx)
    })?;

    module.register_async_method("eth_getLogs", |params, ctx, _| async move {
        // Accept a single `{ fromBlock?, toBlock?, address?, topics? }` filter
        // object. Range bounds accept block tags (`latest`, `earliest`,
        // `pending`) or hex numbers.
        let (filter,): (serde_json::Value,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let tip = state.fork.local_block_number;
        let (from, to) = {
            let fblock = filter.get("fromBlock");
            let tblock = filter.get("toBlock");
            let from = resolve_block_tag(fblock, tip).unwrap_or(0);
            let to = resolve_block_tag(tblock, tip).unwrap_or(tip);
            (from.min(to), to.max(from))
        };
        // Filter: address(es) + topic0. We only match topic0 (event signature)
        // for now — it covers 99% of client use and keeps the scan O(logs).
        let addresses: Vec<String> = match filter.get("address") {
            Some(serde_json::Value::String(s)) => vec![s.to_ascii_lowercase()],
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_ascii_lowercase))
                .collect(),
            _ => Vec::new(),
        };
        let topic0_filter: Vec<String> = match filter.get("topics") {
            Some(serde_json::Value::Array(arr)) if !arr.is_empty() => match &arr[0] {
                serde_json::Value::String(s) => vec![s.to_ascii_lowercase()],
                serde_json::Value::Array(sub) => sub
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_ascii_lowercase))
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let mut out: Vec<serde_json::Value> = Vec::new();
        for (num, block) in state.fork.blocks_by_number.range(from..=to) {
            for tx_hash in &block.transactions {
                let Some(receipt) = state.fork.receipts.get(tx_hash) else {
                    continue;
                };
                let addr_lower = format!("{:#x}", receipt.from).to_ascii_lowercase();
                let _ = addr_lower;
                for log in &receipt.logs {
                    let log_addr = format!("{:#x}", log.address).to_ascii_lowercase();
                    if !addresses.is_empty() && !addresses.contains(&log_addr) {
                        continue;
                    }
                    if !topic0_filter.is_empty() {
                        let Some(t0) = log.topics.first() else {
                            continue;
                        };
                        let t0_lower = format!("{:#x}", t0).to_ascii_lowercase();
                        if !topic0_filter.contains(&t0_lower) {
                            continue;
                        }
                    }
                    out.push(serde_json::json!({
                        "address": log.address,
                        "topics": log.topics,
                        "data": format!("0x{}", hex::encode(log.data.as_ref())),
                        "blockNumber": hex_u64(*num),
                        "blockHash": block.hash,
                        "transactionHash": tx_hash,
                        "transactionIndex": "0x0",
                        "logIndex": hex_u64(log.log_index as u64),
                        "removed": false,
                    }));
                }
            }
        }
        Ok::<_, ErrorObjectOwned>(out)
    })?;

    module.register_async_method("eth_getBlockByNumber", |params, ctx, _| async move {
        let (number, _full): (String, bool) = params.parse().map_err(invalid_params)?;
        let full_transactions = _full;
        let local_block = {
            let state = ctx.state.read();
            if number == "latest" {
                state
                    .fork
                    .blocks_by_number
                    .get(&state.fork.local_block_number)
                    .map(block_json)
            } else {
                parse_hex_quantity(&number)
                    .ok()
                    .and_then(|number| state.fork.blocks_by_number.get(&number))
                    .map(block_json)
            }
        };
        if let Some(block) = local_block {
            return Ok::<_, ErrorObjectOwned>(Some(block));
        }

        let block_tag = if number == "latest" {
            BlockTag::Latest
        } else {
            BlockTag::Number(parse_hex_quantity(&number).map_err(invalid_params)?)
        };
        let upstream_block = run_fork_snapshot(&ctx.state, false, move |fork| {
            fork.db
                .upstream
                .get_block_by_number(block_tag, full_transactions)
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(upstream_block)
    })?;

    module.register_async_method("eth_getBlockByHash", |params, ctx, _| async move {
        let (hash, _full): (B256, bool) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(state.fork.blocks_by_hash.get(&hash).map(block_json))
    })?;

    register_impersonation_methods(&mut module)?;
    register_state_mutation_methods(&mut module)?;
    register_snapshot_methods(&mut module)?;
    register_mirage_methods(&mut module)?;
    #[cfg(feature = "chain")]
    register_chain_methods(&mut module)?;

    Ok(module)
}

#[cfg(feature = "chain")]
fn register_chain_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    use crate::chain_rpc::{
        ChallengeInsightParams, ConfirmInsightParams, DepositPheromoneParams, PostInsightParams,
        SearchInsightsParams, handle_apply_decay, handle_challenge_insight, handle_confirm_insight,
        handle_deposit_pheromone, handle_get_insight, handle_list_kinds, handle_method_schema,
        handle_post_insight, handle_query_pheromones, handle_search_insights, handle_stats,
        handle_version,
    };

    fn require_chain(
        ctx: &ServerContext,
    ) -> std::result::Result<Arc<RwLock<crate::chain_rpc::ChainContext>>, ErrorObjectOwned> {
        ctx.chain.clone().ok_or_else(|| {
            ErrorObjectOwned::owned::<()>(
                crate::chain_rpc::err_code::DISABLED,
                "chain subsystem not attached to this server",
                None,
            )
        })
    }

    module.register_async_method("chain_postInsight", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: PostInsightParams = params.parse().map_err(invalid_params)?;
        let result = handle_post_insight(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_searchInsights", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: SearchInsightsParams = params.parse().map_err(invalid_params)?;
        let result = handle_search_insights(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_confirmInsight", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: ConfirmInsightParams = params.parse().map_err(invalid_params)?;
        let result = handle_confirm_insight(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_challengeInsight", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: ChallengeInsightParams = params.parse().map_err(invalid_params)?;
        let result = handle_challenge_insight(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_getInsight", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_get_insight(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_applyDecay", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_apply_decay(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_depositPheromone", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: DepositPheromoneParams = params.parse().map_err(invalid_params)?;
        let result = handle_deposit_pheromone(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_queryPheromones", |params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_query_pheromones(&chain, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    module.register_async_method("chain_stats", |_params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(handle_stats(&chain))
    })?;

    module.register_async_method("chain_version", |_params, ctx, _| async move {
        let chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(handle_version(&chain))
    })?;

    module.register_async_method("chain_listKinds", |_params, ctx, _| async move {
        let _chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(handle_list_kinds())
    })?;

    module.register_async_method("chain_methodSchema", |params, ctx, _| async move {
        let _chain = require_chain(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_method_schema(payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    // Agent registry RPC methods
    module.register_async_method("chain_registerAgent", |params, ctx, _| async move {
        let (id, address_hex, role): (String, String, String) =
            params.parse().map_err(invalid_params)?;
        let chain = require_chain(&ctx)?;
        crate::chain_rpc::handle_register_agent(&chain, id, address_hex, role)
    })?;

    module.register_async_method("chain_agentHeartbeat", |params, ctx, _| async move {
        let (id,): (String,) = params.parse().map_err(invalid_params)?;
        let chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(crate::chain_rpc::handle_agent_heartbeat(&chain, id))
    })?;

    module.register_async_method("chain_agentTrace", |params, ctx, _| async move {
        let (id, phase, reads, reasoning, action): (String, String, Vec<String>, String, String) =
            params.parse().map_err(invalid_params)?;
        let chain = require_chain(&ctx)?;
        crate::chain_rpc::handle_agent_trace(&chain, id, phase, reads, reasoning, action)
    })?;

    module.register_async_method("chain_agentStats", |params, ctx, _| async move {
        let (id, delta): (String, crate::chain::AgentStats) =
            params.parse().map_err(invalid_params)?;
        let chain = require_chain(&ctx)?;
        Ok::<_, ErrorObjectOwned>(crate::chain_rpc::handle_agent_stats(&chain, id, delta))
    })?;

    #[cfg(feature = "roko")]
    register_chain_subscription_methods(module)?;

    Ok(())
}

/// §38.d — registers `chain_subscribePheromones`, `chain_subscribeInsights`,
/// and `chain_unsubscribe` on the shared RPC module.
///
/// **Subscription transport decision**: we use jsonrpsee's native
/// `register_subscription` machinery (same pattern as `eth_subscribe` in this
/// file, see line ~1100). Each subscription:
///
/// 1. Accepts the incoming WS upgrade via `pending.accept().await`.
/// 2. Registers an [`MpscSink`](crate::roko_bridge::MpscSink) with the
///    corresponding [`SubscriptionManager`] bus so the write-path handlers
///    (`handle_deposit_pheromone`, `handle_post_insight`, etc.) broadcast
///    through it.
/// 3. Bridges the mpsc receiver into jsonrpsee's `SubscriptionSink` in a
///    `tokio::select!` loop that exits when either side closes.
/// 4. Unregisters the bus subscription on loop exit so stats stay accurate.
///
/// The `chain_unsubscribe` method is registered as a regular async method
/// (not as the "unsubscribe" hook tied to jsonrpsee's subscription lifecycle)
/// because our external id format (`pher:N` / `insi:N`) is namespaced across
/// both buses. Jsonrpsee's own per-connection unsubscribe still fires on WS
/// disconnect via the `is_closed()` check in the select loop.
#[cfg(feature = "roko")]
fn register_chain_subscription_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    use crate::chain_rpc::{
        INSIGHT_SUB_PREFIX, PHEROMONE_SUB_PREFIX, handle_unsubscribe, insight_event_to_json,
        pheromone_event_to_json,
    };
    use crate::roko_bridge::{BackpressurePolicy, InsightEvent, MpscSink, PheromoneEvent};

    fn require_subs(
        ctx: &ServerContext,
    ) -> std::result::Result<crate::chain_rpc::SubscriptionManager, ErrorObjectOwned> {
        ctx.chain_subs.clone().ok_or_else(|| {
            ErrorObjectOwned::owned::<()>(
                crate::chain_rpc::err_code::DISABLED,
                "chain subscriptions not attached to this server",
                None,
            )
        })
    }

    module.register_subscription::<std::result::Result<(), SubscriptionError>, _, _>(
        "chain_subscribePheromones",
        "chain_pheromoneEvent",
        "chain_unsubscribePheromones",
        |_params, pending, ctx, _| async move {
            let manager = match require_subs(&ctx) {
                Ok(m) => m,
                Err(e) => {
                    pending.reject(e).await;
                    return Ok(());
                }
            };
            let (mpsc_sink, mut rx) = MpscSink::<PheromoneEvent>::new(128);
            let bus_id = manager
                .pheromones()
                .register(Arc::new(mpsc_sink), BackpressurePolicy::DropNewest);
            let external_id = format!("{}{}", PHEROMONE_SUB_PREFIX, bus_id.0);
            let sink = match pending.accept().await {
                Ok(s) => s,
                Err(_) => {
                    manager.pheromones().unregister(bus_id);
                    return Ok(());
                }
            };

            // First message: tell the client its external id so it can call
            // chain_unsubscribe(id) later.
            if let Ok(handshake) = serde_json::value::to_raw_value(
                &serde_json::json!({"subscriptionId": external_id}),
            ) {
                let _ = sink.send(handshake).await;
            }

            loop {
                tokio::select! {
                    _ = sink.closed() => break,
                    msg = rx.recv() => {
                        match msg {
                            Some(event) => {
                                let payload = pheromone_event_to_json(&event);
                                let Ok(raw) = serde_json::value::to_raw_value(&payload) else { break };
                                if sink.send(raw).await.is_err() { break; }
                            }
                            None => break,
                        }
                    }
                }
            }

            manager.pheromones().unregister(bus_id);
            Ok(())
        },
    )?;

    module.register_subscription::<std::result::Result<(), SubscriptionError>, _, _>(
        "chain_subscribeInsights",
        "chain_insightEvent",
        "chain_unsubscribeInsights",
        |_params, pending, ctx, _| async move {
            let manager = match require_subs(&ctx) {
                Ok(m) => m,
                Err(e) => {
                    pending.reject(e).await;
                    return Ok(());
                }
            };
            let (mpsc_sink, mut rx) = MpscSink::<InsightEvent>::new(128);
            let bus_id = manager
                .insights()
                .register(Arc::new(mpsc_sink), BackpressurePolicy::DropNewest);
            let external_id = format!("{}{}", INSIGHT_SUB_PREFIX, bus_id.0);
            let sink = match pending.accept().await {
                Ok(s) => s,
                Err(_) => {
                    manager.insights().unregister(bus_id);
                    return Ok(());
                }
            };

            if let Ok(handshake) = serde_json::value::to_raw_value(
                &serde_json::json!({"subscriptionId": external_id}),
            ) {
                let _ = sink.send(handshake).await;
            }

            loop {
                tokio::select! {
                    _ = sink.closed() => break,
                    msg = rx.recv() => {
                        match msg {
                            Some(event) => {
                                let payload = insight_event_to_json(&event);
                                let Ok(raw) = serde_json::value::to_raw_value(&payload) else { break };
                                if sink.send(raw).await.is_err() { break; }
                            }
                            None => break,
                        }
                    }
                }
            }

            manager.insights().unregister(bus_id);
            Ok(())
        },
    )?;

    module.register_async_method("chain_unsubscribe", |params, ctx, _| async move {
        let manager = require_subs(&ctx)?;
        let payload: serde_json::Value = params.parse().map_err(invalid_params)?;
        let result = handle_unsubscribe(&manager, payload)?;
        Ok::<_, ErrorObjectOwned>(result)
    })?;

    Ok(())
}

/// Full fork reset for `hardhat_reset` / `anvil_reset`: dirty store + read cache + watch lists,
/// local tx indexes, and impersonation set (mirrors Hardhat/Anvil semantics).
fn apply_hardhat_anvil_reset(state: &mut MirageState) {
    state.fork.db.reset();
    state.fork.db.pinned_block = None;
    state.fork.db.dirty.demote_protocols_to_slot_only = false;
    state.fork.impersonated_accounts.clear();
    state.fork.receipts.clear();
    state.fork.transactions.clear();
    state.fork.blocks_by_hash.clear();
    state.fork.blocks_by_number.clear();
    state.last_committed_state_diff = None;
}

/// Parses `hardhat_mine` / `anvil_mine` first parameter: hex quantity string or JSON integer.
fn parse_mine_block_count(values: &[serde_json::Value]) -> u64 {
    let Some(first) = values.first() else {
        return 1;
    };
    match first {
        serde_json::Value::String(text) => parse_hex_quantity(text.trim()).unwrap_or(1).max(1),
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(1).max(1),
        _ => 1,
    }
}

fn register_impersonation_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    for method in ["hardhat_impersonateAccount", "anvil_impersonateAccount"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address,): (Address,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.impersonated_accounts.insert(address);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in [
        "hardhat_stopImpersonatingAccount",
        "anvil_stopImpersonatingAccount",
    ] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address,): (Address,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.impersonated_accounts.remove(&address);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    Ok(())
}

fn register_state_mutation_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    for method in [
        "hardhat_setBalance",
        "anvil_setBalance",
        "mirage_setBalance",
    ] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address, balance): (Address, U256) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.db.set_balance(address, balance);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in [
        "hardhat_setStorageAt",
        "anvil_setStorageAt",
        "mirage_setStorageAt",
    ] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address, slot, value): (Address, U256, U256) =
                params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.db.set_storage(address, slot, value);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_setCode", "anvil_setCode", "mirage_setCode"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address, code): (Address, Bytes) = params.parse().map_err(invalid_params)?;
            let bytecode = Bytecode::new_raw(code);
            with_state_write(&ctx.state, |state| {
                state.fork.db.set_code(address, bytecode);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_setNonce", "anvil_setNonce"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (address, nonce): (Address, u64) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.db.set_nonce(address, nonce);
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_mine", "anvil_mine", "evm_mine"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let count = params
                .parse::<Vec<serde_json::Value>>()
                .map(|values| parse_mine_block_count(&values))
                .unwrap_or(1);
            with_state_write(&ctx.state, |state| {
                for _ in 0..count {
                    state.fork.local_block_number = state.fork.local_block_number.saturating_add(1);
                    let block_hash = keccak256(state.fork.local_block_number.to_le_bytes());
                    let block = LocalBlock {
                        hash: block_hash,
                        number: state.fork.local_block_number,
                        timestamp: state.fork.timestamp,
                        gas_used: 0,
                        gas_limit: 30_000_000,
                        base_fee_per_gas: state.fork.next_base_fee_per_gas,
                        coinbase: state.fork.coinbase,
                        prev_randao: state.fork.prev_randao,
                        transactions: Vec::new(),
                    };
                    state.fork.blocks_by_hash.insert(block_hash, block.clone());
                    state
                        .fork
                        .blocks_by_number
                        .insert(state.fork.local_block_number, block);
                }
                state.fork.prune_old_blocks();
                let _ = state.new_heads_tx.send(crate::fork::NewHeadBroadcast {
                    number: state.fork.local_block_number,
                    timestamp: state.fork.timestamp,
                    gas_used: 0,
                    gas_limit: 30_000_000,
                    base_fee_per_gas: state.fork.next_base_fee_per_gas,
                    coinbase: state.fork.coinbase,
                    prev_randao: state.fork.prev_randao,
                });
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_reset", "anvil_reset"] {
        module.register_async_method(method, |_params, ctx, _| async move {
            with_state_write(&ctx.state, apply_hardhat_anvil_reset).await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in [
        "hardhat_setNextBlockBaseFeePerGas",
        "anvil_setNextBlockBaseFeePerGas",
    ] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (value,): (u128,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.next_base_fee_per_gas = value;
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_setCoinbase", "anvil_setCoinbase"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (value,): (Address,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.coinbase = value;
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    for method in ["hardhat_setPrevRandao", "anvil_setPrevRandao"] {
        module.register_async_method(method, |params, ctx, _| async move {
            let (value,): (B256,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                state.fork.prev_randao = value;
            })
            .await;
            Ok::<_, ErrorObjectOwned>(true)
        })?;
    }
    // tevm_setAccount — TEVM's combined account mutation used when forking from a custom RPC.
    // Params: { address, nonce?, balance?, deployedBytecode?, state? }
    module.register_async_method("tevm_setAccount", |params, ctx, _| async move {
        let obj: serde_json::Value = params.parse().map_err(invalid_params)?;
        // params arrive as a single-element array containing the object
        let obj = if obj.is_array() {
            obj.as_array()
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or(obj)
        } else {
            obj
        };
        let address: Address = obj
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid_params("missing address"))?
            .parse()
            .map_err(|_| invalid_params("invalid address"))?;
        with_state_write(&ctx.state, |state| {
            if let Some(balance_val) = obj.get("balance").and_then(|v| v.as_str()) {
                if let Ok(balance) = U256::from_str_radix(balance_val.trim_start_matches("0x"), 16)
                {
                    state.fork.db.set_balance(address, balance);
                }
            }
            if let Some(nonce_val) = obj.get("nonce").and_then(|v| v.as_str()) {
                if let Ok(nonce) = u64::from_str_radix(nonce_val.trim_start_matches("0x"), 16) {
                    state.fork.db.set_nonce(address, nonce);
                }
            }
            if let Some(code_val) = obj.get("deployedBytecode").and_then(|v| v.as_str()) {
                if let Ok(bytes) = hex::decode(code_val.trim_start_matches("0x")) {
                    state
                        .fork
                        .db
                        .set_code(address, Bytecode::new_raw(Bytes::from(bytes)));
                }
            }
            // state: { "0xslot": "0xvalue", ... }
            if let Some(storage) = obj.get("state").and_then(|v| v.as_object()) {
                for (slot_hex, val) in storage {
                    if let (Ok(slot), Some(value)) = (
                        U256::from_str_radix(slot_hex.trim_start_matches("0x"), 16),
                        val.as_str().and_then(|s| {
                            U256::from_str_radix(s.trim_start_matches("0x"), 16).ok()
                        }),
                    ) {
                        state.fork.db.set_storage(address, slot, value);
                    }
                }
            }
        })
        .await;
        Ok::<_, ErrorObjectOwned>(serde_json::json!({ "errors": [] }))
    })?;
    Ok(())
}

fn register_snapshot_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    module.register_async_method("evm_snapshot", |_params, ctx, _| async move {
        let snapshot = with_state_write(&ctx.state, |state| hex_u64(state.fork.snapshot())).await;
        Ok::<_, ErrorObjectOwned>(snapshot)
    })?;
    module.register_async_method("evm_revert", |params, ctx, _| async move {
        let (snapshot_id,): (String,) = params.parse().map_err(invalid_params)?;
        let id = parse_hex_quantity(&snapshot_id).map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| state.fork.revert(id).map_err(rpc_error)).await
    })?;
    module.register_async_method("evm_increaseTime", |params, ctx, _| async move {
        let (seconds,): (u64,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            state.fork.timestamp = state.fork.timestamp.saturating_add(seconds);
        })
        .await;
        Ok::<_, ErrorObjectOwned>(hex_u64(seconds))
    })?;
    module.register_async_method("evm_setNextBlockTimestamp", |params, ctx, _| async move {
        let (timestamp,): (u64,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            state.fork.timestamp = timestamp;
        })
        .await;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    Ok(())
}

fn register_mirage_methods(
    module: &mut RpcModule<ServerContext>,
) -> std::result::Result<(), RegisterMethodError> {
    module.register_async_method("mirage_mintERC20", |params, ctx, _| async move {
        let (token, owner, amount): (Address, Address, U256) =
            params.parse().map_err(invalid_params)?;
        let staged = stage_erc20_mint(&ctx.state, token, owner, amount)
            .await
            .map_err(rpc_error)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            {
                let account = state.fork.db.dirty.accounts.entry(token).or_default();
                if let Some(slot) = staged.balance_slot {
                    account.erc20_balance_slot = Some(slot);
                }
                account.erc20_balances.insert(staged.owner, staged.balance);
            }
            for (slot, value) in &staged.storage_writes {
                state.fork.db.set_storage(token, *slot, *value);
            }
            let added_at_block = state.fork.local_block_number;
            state
                .fork
                .db
                .dirty
                .watch_list
                .entry(token)
                .or_insert_with(|| crate::fork::WatchEntry {
                    source: crate::fork::WatchSource::Manual,
                    added_at_block,
                    initial_slot_count: 1,
                    replay_count: 0,
                });
            Ok::<_, ErrorObjectOwned>(())
        })
        .await?;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("mirage_prefetchAccount", |params, ctx, _| async move {
        let (address,): (Address,) = params.parse().map_err(invalid_params)?;
        let account = run_fork_snapshot(&ctx.state, true, move |mut fork| fork.db.basic(address))
            .await
            .map_err(rpc_error)?;
        if let Some(account) = account {
            let mut state = ctx.state.write();
            state.fork.db.read_cache.insert_account(address, account);
        }
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("mirage_prefetchSlots", |params, ctx, _| async move {
        let (address, slots): (Address, Vec<U256>) = params.parse().map_err(invalid_params)?;
        let prefetched = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            slots
                .into_iter()
                .map(|slot| fork.db.storage(address, slot).map(|value| (slot, value)))
                .collect::<Result<Vec<_>>>()
        })
        .await
        .map_err(rpc_error)?;
        let mut state = ctx.state.write();
        for (slot, value) in prefetched {
            state
                .fork
                .db
                .read_cache
                .insert_storage(address, slot, value);
        }
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method(MIRAGE_WATCH_CONTRACT_METHOD, |params, ctx, _| async move {
        let (address,): (Address,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            let added_at_block = state.fork.local_block_number;
            state.fork.db.dirty.watch_list.insert(
                address,
                crate::fork::WatchEntry {
                    source: crate::fork::WatchSource::Manual,
                    added_at_block,
                    initial_slot_count: 0,
                    replay_count: 0,
                },
            );
        })
        .await;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("mirage_unwatchContract", |params, ctx, _| async move {
        let (address,): (Address,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            state.fork.db.dirty.watch_list.remove(&address);
            state.fork.db.dirty.unwatch_list.insert(address);
        })
        .await;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method("mirage_getWatchList", |_params, ctx, _| async move {
        let state = ctx.state.read();
        let watch_list = state
            .fork
            .db
            .dirty
            .watch_list
            .iter()
            .map(|(address, entry)| serde_json::json!({"address": address, "entry": entry}))
            .collect::<Vec<_>>();
        Ok::<_, ErrorObjectOwned>(watch_list)
    })?;
    module.register_async_method("mirage_getDirtySlots", |params, ctx, _| async move {
        let (address,): (Address,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let slots = state
            .fork
            .db
            .dirty
            .accounts
            .get(&address)
            .map(|account| account.storage.clone())
            .unwrap_or_default();
        Ok::<_, ErrorObjectOwned>(slots)
    })?;
    module.register_async_method("mirage_getLastStateDiff", |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(state.last_committed_state_diff.clone())
    })?;
    module.register_async_method(MIRAGE_STATUS_METHOD, |_params, ctx, _| async move {
        let state = ctx.state.read();
        Ok::<_, ErrorObjectOwned>(state.fork.status(state.mode))
    })?;
    module.register_async_method(
        MIRAGE_GET_RESOURCE_USAGE_METHOD,
        |_params, ctx, _| async move {
            let usage = with_state_write(&ctx.state, |state| {
                touch_request(state);
                apply_resource_pressure(state);
                state.fork.resource_usage(&state.resource_model, state.mode)
            })
            .await;
            Ok::<_, ErrorObjectOwned>(usage)
        },
    )?;
    module.register_async_method("mirage_setResourceLimits", |params, ctx, _| async move {
        let (profile,): (Option<Profile>,) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            if let Some(profile) = profile {
                state.resource_model =
                    ResourceModel::for_profile(profile, state.resource_model.cache_ttl);
            }
        })
        .await;
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method(MIRAGE_GET_POSITION_METHOD, |params, ctx, _| async move {
        let (request,): (PositionRequest,) = params.parse().map_err(invalid_params)?;
        let snapshot = run_fork_snapshot(&ctx.state, true, move |mut fork| {
            let balances = request
                .token_addresses
                .iter()
                .map(|address| {
                    let balance = fork
                        .db
                        .erc20_balance_of(*address, request.owner)
                        .or_else(|_| fork.db.basic(*address).map(|info| info.unwrap_or_default().balance))
                        .unwrap_or(U256::ZERO);
                    (*address, hex_u256(balance))
                })
                .collect::<Vec<_>>();
            let data = match request.protocol_type.as_str() {
                "raw-balances" => serde_json::json!({"balances": balances}),
                "uniswap-v3-position" => serde_json::json!({
                    "balances": balances,
                    "positionNftBalance": request
                        .contract
                        .and_then(|contract| fork.db.erc20_balance_of(contract, request.owner).ok())
                        .map_or_else(|| hex_u256(U256::ZERO), hex_u256),
                    "pool": request.contract,
                }),
                "aave-v3-account" => serde_json::json!({
                    "balances": balances,
                    "market": request.contract,
                    "healthFactor": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    "debtBalance": hex_u256(U256::ZERO),
                }),
                unknown => return Err(MirageError::UnknownProtocolType(unknown.to_owned())),
            };
            Ok(PositionSnapshot {
                owner: request.owner,
                protocol_type: request.protocol_type,
                data,
            })
        })
        .await
        .map_err(rpc_error)?;
        Ok::<_, ErrorObjectOwned>(snapshot)
    })?;
    module.register_async_method(
        MIRAGE_SUBSCRIBE_EVENTS_METHOD,
        |params, ctx, _| async move {
            let (filter,): (EventFilter,) = params.parse().map_err(invalid_params)?;
            let id = register_event_subscription(&ctx.state, filter);
            Ok::<_, ErrorObjectOwned>(id)
        },
    )?;
    module.register_subscription::<std::result::Result<(), SubscriptionError>, _, _>(
        "eth_subscribe",
        "eth_subscription",
        "eth_unsubscribe",
        |params, pending, ctx, _| async move {
            let params_vec: Vec<serde_json::Value> = params.parse().unwrap_or_default();
            let sub_type = params_vec
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();

            if sub_type != "newHeads" {
                pending
                    .reject(ErrorObjectOwned::owned(
                        -32602,
                        format!("unsupported subscription type: {sub_type}"),
                        None::<()>,
                    ))
                    .await;
                return Ok(());
            }

            let mut rx = ctx.state.read().new_heads_tx.subscribe();
            let sink = match pending.accept().await {
                Ok(s) => s,
                Err(_) => return Ok(()),
            };

            loop {
                tokio::select! {
                    _ = sink.closed() => break,
                    result = rx.recv() => {
                        match result {
                            Ok(head) => {
                                let header = new_heads_json(
                                    head.number,
                                    head.timestamp,
                                    head.gas_used,
                                    head.gas_limit,
                                    head.base_fee_per_gas,
                                    head.coinbase,
                                    head.prev_randao,
                                );
                                let Ok(msg) = serde_json::value::to_raw_value(&header) else {
                                    break;
                                };
                                if sink.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                // Fell behind — continue receiving from the latest
                            }
                            Err(_) => break,
                        }
                    }
                }
            }

            Ok(())
        },
    )?;
    module.register_async_method(
        MIRAGE_BEGIN_SCENARIO_SET_METHOD,
        |params, ctx, _| async move {
            let (baseline,): (String,) = params.parse().map_err(invalid_params)?;
            with_state_write(&ctx.state, |state| {
                touch_request(state);
                if state.reject_new_forks {
                    return Err::<String, _>(rpc_error(MirageError::Unsupported(
                        "resource pressure is refusing new scenario forks".to_owned(),
                    )));
                }
                let (baseline_snapshot_id, baseline_fork) = if baseline == "latest" {
                    let snapshot_id = state.fork.snapshot();
                    (snapshot_id, Some(state.fork.clone()))
                } else {
                    let snapshot_id = parse_hex_quantity(&baseline).map_err(invalid_params)?;
                    let mut baseline_fork = state.fork.clone();
                    baseline_fork.revert(snapshot_id).map_err(rpc_error)?;
                    let refreshed_snapshot = baseline_fork.snapshot();
                    (refreshed_snapshot, Some(baseline_fork))
                };
                let set_id = format!("set-{}", state.scenarios.len() + 1);
                state.scenarios.insert(
                    set_id.clone(),
                    ScenarioSet {
                        id: set_id.clone(),
                        baseline_snapshot_id,
                        baseline_fork,
                        scenarios: Vec::new(),
                        status: ScenarioSetStatus::Draft,
                    },
                );
                Ok::<_, ErrorObjectOwned>(set_id)
            })
            .await
        },
    )?;
    module.register_async_method(MIRAGE_DEFINE_SCENARIO_METHOD, |params, ctx, _| async move {
        let (set_id, scenario): (String, Scenario) = params.parse().map_err(invalid_params)?;
        with_state_write(&ctx.state, |state| {
            touch_request(state);
            let set = state
                .scenarios
                .get_mut(&set_id)
                .ok_or_else(|| rpc_error(MirageError::SetNotFound(set_id.clone())))?;
            if set.status != ScenarioSetStatus::Draft {
                return Err(rpc_error(MirageError::SetAlreadyRunning(set_id.clone())));
            }
            set.scenarios.push(scenario.clone());
            Ok::<_, ErrorObjectOwned>(scenario.id)
        })
        .await
    })?;
    module.register_async_method(
        MIRAGE_RUN_SCENARIO_SET_METHOD,
        |params, ctx, _| async move {
            let (set_id, mode): (String, RunMode) = params.parse().map_err(invalid_params)?;
            let job_id = {
                let mut state = ctx.state.write();
                let set = state
                    .scenarios
                    .get_mut(&set_id)
                    .ok_or_else(|| rpc_error(MirageError::SetNotFound(set_id.clone())))?;
                if set.status == ScenarioSetStatus::Running {
                    return Err(rpc_error(MirageError::SetAlreadyRunning(set_id.clone())));
                }
                if set.scenarios.is_empty() {
                    return Err(rpc_error(MirageError::SetHasNoScenarios(set_id.clone())));
                }
                set.status = ScenarioSetStatus::Running;
                let job_id = format!("job-{}", state.jobs.len() + 1);
                state.jobs.insert(
                    job_id.clone(),
                    ScenarioJob {
                        job_id: job_id.clone(),
                        set_id: set_id.clone(),
                        status: JobStatus::Running,
                        results: None,
                        total_wall_time_ms: None,
                    },
                );
                job_id
            };
            let state = Arc::clone(&ctx.state);
            let job_id_for_task = job_id.clone();
            tokio::spawn(async move {
                let started = tokio::time::Instant::now();
                let set = { state.read().scenarios.get(&set_id).cloned() };
                if let Some(set) = set {
                    let runner = ScenarioRunner::new(Arc::clone(&state));
                    let results = match mode {
                        RunMode::Sequential => runner.run_sequential(&set).await,
                        RunMode::Parallel => runner.run_parallel(&set).await,
                    };
                    let mut state = state.write();
                    if let Some(job) = state.jobs.get_mut(&job_id_for_task) {
                        job.status = JobStatus::Complete;
                        job.total_wall_time_ms =
                            Some(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX));
                        job.results = Some(results);
                    }
                    if let Some(set) = state.scenarios.get_mut(&set_id) {
                        set.status = ScenarioSetStatus::Complete;
                    }
                }
            });
            Ok::<_, ErrorObjectOwned>(job_id)
        },
    )?;
    module.register_async_method(
        MIRAGE_GET_SCENARIO_RESULTS_METHOD,
        |params, ctx, _| async move {
            let (job_id,): (String,) = params.parse().map_err(invalid_params)?;
            let state = ctx.state.read();
            let job = state
                .jobs
                .get(&job_id)
                .cloned()
                .ok_or_else(|| rpc_error(MirageError::JobNotFound(job_id.clone())))?;
            Ok::<_, ErrorObjectOwned>(job)
        },
    )?;
    module.register_async_method("mirage_compareScenarios", |params, ctx, _| async move {
        let (job_id,): (String,) = params.parse().map_err(invalid_params)?;
        let state = ctx.state.read();
        let job = state
            .jobs
            .get(&job_id)
            .ok_or_else(|| rpc_error(MirageError::JobNotFound(job_id.clone())))?;
        Ok::<_, ErrorObjectOwned>(rank_scenario_results(
            job.results.clone().unwrap_or_default(),
        ))
    })?;
    module.register_async_method(
        "mirage_computeDomainSeparator",
        |params, ctx, _| async move {
            let (contract,): (Address,) = params.parse().map_err(invalid_params)?;
            let result = run_fork_snapshot(&ctx.state, false, move |fork| {
                EvmExecutor::call(
                    &fork,
                    Address::ZERO,
                    contract,
                    Bytes::from_static(&[0x36, 0x44, 0xe5, 0x15]),
                    U256::ZERO,
                    100_000,
                )
            })
            .await
            .map_err(rpc_error)?;
            Ok::<_, ErrorObjectOwned>(format!("0x{}", hex::encode(&result.output)))
        },
    )?;
    module.register_async_method("mirage_cleanup", |_params, _ctx, _| async {
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    module.register_async_method(MIRAGE_SHUTDOWN_METHOD, |_params, ctx, _| async move {
        tracing::warn!("mirage_shutdown RPC called — initiating shutdown");
        let _ = ctx.shutdown.send(());
        Ok::<_, ErrorObjectOwned>(true)
    })?;
    Ok(())
}

async fn run_fork_snapshot<T, F>(
    state: &Arc<RwLock<MirageState>>,
    touch: bool,
    task: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(ForkState) -> Result<T> + Send + 'static,
{
    let fork = if touch {
        let mut state = state.write();
        touch_request(&mut state);
        state.fork.clone()
    } else {
        state.read().fork.clone()
    };
    tokio::task::spawn_blocking(move || task(fork))
        .await
        .map_err(|error| MirageError::BackgroundTask(error.to_string()))?
}

async fn stage_erc20_mint(
    state: &Arc<RwLock<MirageState>>,
    token: Address,
    owner: Address,
    amount: U256,
) -> Result<StagedErc20Mint> {
    let fork = {
        let _writer_guard = lock_state_writes(state).await;
        let mut state = state.write();
        touch_request(&mut state);
        state.fork.clone()
    };
    tokio::task::spawn_blocking(move || stage_erc20_mint_on_snapshot(fork, token, owner, amount))
        .await
        .map_err(|error| MirageError::BackgroundTask(error.to_string()))?
}

fn stage_erc20_mint_on_snapshot(
    mut fork: ForkState,
    token: Address,
    owner: Address,
    amount: U256,
) -> Result<StagedErc20Mint> {
    let current_balance = fork.db.erc20_balance_of(token, owner).unwrap_or(U256::ZERO);
    let next_balance = current_balance.saturating_add(amount);
    let _ = fork.db.set_erc20_balance(token, owner, next_balance)?;
    let token_account = fork.db.dirty.accounts.get(&token);
    let balance_slot = token_account.and_then(|account| account.erc20_balance_slot);
    let storage_writes = token_account
        .map(|account| {
            account
                .storage
                .iter()
                .map(|(slot, value)| (*slot, *value))
                .collect()
        })
        .unwrap_or_default();
    let balance = token_account
        .and_then(|account| account.erc20_balances.get(&owner))
        .copied()
        .unwrap_or(next_balance);
    Ok(StagedErc20Mint {
        owner,
        balance,
        balance_slot,
        storage_writes,
    })
}

fn touch_request(state: &mut MirageState) {
    state.last_request_at = std::time::Instant::now();
    apply_resource_pressure(state);
}

fn apply_resource_pressure(state: &mut MirageState) {
    let usage = state.fork.resource_usage(&state.resource_model, state.mode);
    let action = usage.pressure_action();
    if action != PressureAction::None {
        // Non-blocking: BusSender::emit writes to a bounded broadcast + ring buffer
        // (drops the oldest replay entry when saturated; live send errors are ignored
        // when no subscribers are connected).
        state.telemetry.emit(MirageTelemetryEvent::ResourceWarning {
            resource: "memory".to_owned(),
            utilization: usage.resource_pressure.clamp(0.0, 1.0),
        });
    }
    apply_pressure_action(state, action);
}

fn apply_pressure_action(state: &mut MirageState, action: PressureAction) {
    match action {
        PressureAction::None => {
            state.reject_new_forks = false;
            state.fork.db.dirty.demote_protocols_to_slot_only = false;
        }
        PressureAction::EvictCache => {
            state.reject_new_forks = false;
            state.fork.db.dirty.demote_protocols_to_slot_only = false;
            let target_entries = state.resource_model.cache_capacity / 2;
            state.fork.db.evict_read_cache_to(target_entries);
        }
        PressureAction::Throttle => {
            state.reject_new_forks = false;
            state.fork.db.dirty.demote_protocols_to_slot_only = true;
            let target_entries = state.resource_model.cache_capacity / 4;
            state.fork.db.evict_read_cache_to(target_entries);
        }
        PressureAction::DemoteToProxy => {
            state.reject_new_forks = true;
            state.fork.db.dirty.demote_protocols_to_slot_only = true;
            state.fork.db.evict_read_cache_to(0);
            state
                .fork
                .db
                .dirty
                .watch_list
                .retain(|_, entry| matches!(entry.source, WatchSource::Manual));
            state.jobs.clear();
            state.scenarios.clear();
            state.mode = MirageMode::Proxy;
            let _ = state.mode_change.send(state.mode);
        }
    }
}

fn register_event_subscription(state: &Arc<RwLock<MirageState>>, filter: EventFilter) -> String {
    let mut state = state.write();
    state.last_request_at = std::time::Instant::now();
    state.next_event_subscription_id = state.next_event_subscription_id.saturating_add(1);
    let stream_id = format!("stream-{}", state.next_event_subscription_id);
    state.event_subscriptions.insert(stream_id.clone(), filter);
    stream_id
}

fn publish_receipt_events(state: &MirageState, receipt: &LocalReceipt) {
    for log in &receipt.logs {
        let event = MirageEvent {
            block_number: receipt.block_number,
            tx_hash: receipt.transaction_hash,
            log_index: log.log_index,
            contract: log.address,
            topics: log.topics.clone(),
            data: log.data.clone(),
            source: EventSource::LocalTx,
            decoded: None,
        };
        // Keep event publication non-blocking like golem-core's bounded event fan-out:
        // producers never wait on consumers, and lagging subscribers may miss events.
        let _ = state.event_bus.send(event);
    }
}

async fn health_handler(State(state): State<Arc<RwLock<MirageState>>>) -> impl IntoResponse {
    let state = state.read();
    axum::Json(state.fork.status(state.mode))
}

async fn event_ws_handler(
    Path(stream_id): Path<String>,
    State(state): State<Arc<RwLock<MirageState>>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let filter = {
        let state = state.read();
        state.event_subscriptions.get(&stream_id).cloned()
    };
    filter.map_or_else(
        || StatusCode::NOT_FOUND.into_response(),
        |filter| ws.on_upgrade(move |socket| handle_event_socket(socket, state, filter)),
    )
}

async fn unsubscribe_event_handler(
    Path(stream_id): Path<String>,
    State(state): State<Arc<RwLock<MirageState>>>,
) -> impl IntoResponse {
    let removed = state
        .write()
        .event_subscriptions
        .remove(&stream_id)
        .is_some();
    axum::Json(removed)
}

async fn handle_event_socket(
    mut socket: WebSocket,
    state: Arc<RwLock<MirageState>>,
    filter: EventFilter,
) {
    let mut receiver = state.read().event_bus.subscribe();
    while let Ok(event) = receiver.recv().await {
        if !event_matches_filter(&event, &filter) {
            continue;
        }
        let payload = match serde_json::to_string(&event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!("failed to serialize mirage event: {error}");
                continue;
            }
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            break;
        }
    }
}

fn event_matches_filter(event: &MirageEvent, filter: &EventFilter) -> bool {
    let address_match = filter
        .addresses
        .as_ref()
        .is_none_or(|addresses| addresses.contains(&event.contract));
    let topic_match = filter
        .topics
        .as_ref()
        .is_none_or(|topics| event.topics.iter().any(|topic| topics.contains(topic)));
    address_match && topic_match
}

fn extract_to(kind: Option<Address>) -> Option<Address> {
    kind
}

fn resolve_block_tag(v: Option<&serde_json::Value>, tip: u64) -> Option<u64> {
    match v? {
        serde_json::Value::String(s) => match s.as_str() {
            "latest" | "pending" | "safe" | "finalized" => Some(tip),
            "earliest" => Some(0),
            hex => parse_hex_quantity(hex).ok(),
        },
        serde_json::Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

fn receipt_json(receipt: &LocalReceipt) -> serde_json::Value {
    // For contract-creation txs, the diff.output carries the deployed address
    // (20 bytes); surface it as `contractAddress` so standard clients see it.
    let contract_address = if receipt.to.is_none() && receipt.state_diff.output.len() == 20 {
        Some(format!(
            "0x{}",
            hex::encode(receipt.state_diff.output.as_ref())
        ))
    } else {
        None
    };
    // Zero-bloom (no indexed-log prefilter) — sufficient for tests and roko agents.
    let zero_bloom = format!("0x{}", "0".repeat(512));
    // Enrich log entries with block+tx context so alloy-style receipt
    // deserialization succeeds.
    let logs_full: Vec<serde_json::Value> = receipt
        .logs
        .iter()
        .map(|l| {
            serde_json::json!({
                "address": l.address,
                "topics": l.topics,
                "data": format!("0x{}", hex::encode(l.data.as_ref())),
                "logIndex": hex_u64(l.log_index as u64),
                "blockHash": receipt.block_hash,
                "blockNumber": hex_u64(receipt.block_number),
                "transactionHash": receipt.transaction_hash,
                "transactionIndex": "0x0",
                "removed": false,
            })
        })
        .collect();
    serde_json::json!({
        "type": "0x2",
        "transactionHash": receipt.transaction_hash,
        "transactionIndex": "0x0",
        "blockHash": receipt.block_hash,
        "blockNumber": hex_u64(receipt.block_number),
        "from": receipt.from,
        "to": receipt.to,
        "cumulativeGasUsed": hex_u64(receipt.gas_used),
        "gasUsed": hex_u64(receipt.gas_used),
        "effectiveGasPrice": "0x1",
        "contractAddress": contract_address,
        "logs": logs_full,
        "logsBloom": zero_bloom,
        "status": if receipt.success { "0x1" } else { "0x0" },
    })
}

fn transaction_json(tx: &LocalTransaction) -> serde_json::Value {
    serde_json::json!({
        "hash": tx.hash,
        "from": tx.from,
        "to": tx.to,
        "value": hex_u256(tx.value),
        "input": format!("0x{}", hex::encode(&tx.input)),

        "gas": hex_u64(tx.gas),
        "nonce": hex_u64(tx.nonce),
        "blockNumber": hex_u64(tx.block_number),
    })
}

fn block_json(block: &LocalBlock) -> serde_json::Value {
    let parent_hash = if block.number > 0 {
        keccak256((block.number - 1).to_le_bytes())
    } else {
        B256::ZERO
    };
    serde_json::json!({
        "hash": block.hash,
        "parentHash": format!("{parent_hash}"),
        "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        "miner": block.coinbase,
        "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "difficulty": "0x0",
        "number": hex_u64(block.number),
        "gasLimit": hex_u64(block.gas_limit),
        "gasUsed": hex_u64(block.gas_used),
        "timestamp": hex_u64(block.timestamp),
        "extraData": "0x",
        "mixHash": format!("{}", block.prev_randao),
        "nonce": "0x0000000000000000",
        "baseFeePerGas": format!("0x{:x}", block.base_fee_per_gas),
        "transactions": block.transactions,
    })
}

fn new_heads_json(
    block_num: u64,
    timestamp: u64,
    gas_used: u64,
    gas_limit: u64,
    base_fee: u128,
    coinbase: Address,
    prev_randao: B256,
) -> serde_json::Value {
    let hash = keccak256(block_num.to_le_bytes());
    serde_json::json!({
        "hash": format!("{hash}"),
        "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        "miner": coinbase,
        "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "difficulty": "0x0",
        "number": hex_u64(block_num),
        "gasLimit": hex_u64(gas_limit),
        "gasUsed": hex_u64(gas_used),
        "timestamp": hex_u64(timestamp),
        "extraData": "0x",
        "mixHash": format!("{prev_randao}"),
        "nonce": "0x0000000000000000",
        "baseFeePerGas": format!("0x{base_fee:x}"),
    })
}

fn hex_u64(value: u64) -> String {
    format!("0x{value:x}")
}

fn hex_u256(value: U256) -> String {
    format!("0x{value:x}")
}

const Q96: f64 = (1_u128 << 96) as f64;

/// Converts a human-readable price into a Uniswap V3 `sqrtPriceX96` value.
///
/// The helper is pure and returns zero for non-finite or non-positive inputs.
#[must_use]
pub fn to_sqrt_price_x96(price: f64) -> U256 {
    if !price.is_finite() || price <= 0.0 {
        return U256::ZERO;
    }

    let sqrt_price_x96 = (price.sqrt() * Q96).round();
    if !sqrt_price_x96.is_finite() || sqrt_price_x96 <= 0.0 || sqrt_price_x96 > u128::MAX as f64 {
        return U256::ZERO;
    }

    U256::from(sqrt_price_x96 as u128)
}

/// Converts a Uniswap V3 `sqrtPriceX96` value back into a human-readable price.
///
/// The helper is pure and returns zero for values it cannot parse.
#[must_use]
pub fn from_sqrt_price_x96(sqrt_price_x96: U256) -> f64 {
    if sqrt_price_x96.is_zero() {
        return 0.0;
    }

    let sqrt_price = sqrt_price_x96.to_string().parse::<f64>().unwrap_or(0.0);
    let ratio = sqrt_price / Q96;
    ratio * ratio
}

/// Spawns a Mirage test child process for integration tests.
///
/// Reads the listening port from the `MIRAGE_TEST_PORT` environment variable when set to a valid
/// `u16`. If unset or unparsable, binds **18552**.
pub async fn mirage_instance_or_env() -> crate::Result<crate::MirageTestInstance> {
    let port = std::env::var("MIRAGE_TEST_PORT")
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or(18_552);
    crate::spawn_mirage_test_instance(None, Some(port)).await
}

fn parse_hex_quantity(value: &str) -> Result<u64> {
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .map_err(|error| MirageError::InvalidParams(format!("invalid hex quantity: {error}")))
}

#[derive(Debug, Clone)]
struct CommittedTransaction {
    tx_hash: B256,
    block_number: u64,
    diff: crate::replay::StateDiff,
    transaction: LocalTransaction,
    receipt: LocalReceipt,
    block: LocalBlock,
}

async fn commit_transaction_request(
    state: &Arc<RwLock<MirageState>>,
    request: TransactionRequest,
    override_hash: Option<B256>,
) -> Result<B256> {
    let fork = {
        let _writer_guard = lock_state_writes(state).await;
        let mut state = state.write();
        touch_request(&mut state);
        state.fork.clone()
    };
    let committed = tokio::task::spawn_blocking(move || {
        commit_transaction_on_snapshot(fork, request, override_hash)
    })
    .await
    .map_err(|error| MirageError::BackgroundTask(error.to_string()))??;
    let receipt = committed.receipt.clone();
    let classifier = DiffClassifier::new(ClassificationConfig::default());
    let CommittedTransaction {
        diff,
        transaction,
        receipt: committed_receipt,
        block,
        block_number,
        tx_hash,
    } = committed;
    let invalidate_request = TransactionRequest {
        from: Some(transaction.from),
        to: transaction.to,
        gas: Some(transaction.gas),
        value: Some(transaction.value),
        data: Some(transaction.input.clone()),
        ..Default::default()
    };
    let _writer_guard = lock_state_writes(state).await;
    let mut state = state.write();
    state
        .fork
        .commit_local_transaction(&diff, transaction, committed_receipt, block);
    classifier.apply(&mut state.fork.db.dirty, &diff, block_number)?;
    state
        .speculative_executor
        .lock()
        .invalidate_for_request(&invalidate_request);
    state.last_committed_state_diff = Some(diff);
    publish_receipt_events(&state, &receipt);
    Ok(tx_hash)
}

fn commit_transaction_on_snapshot(
    mut fork: ForkState,
    request: TransactionRequest,
    override_hash: Option<B256>,
) -> Result<CommittedTransaction> {
    let from = request
        .from
        .ok_or_else(|| MirageError::InvalidParams("missing from".to_owned()))?;
    let to = extract_to(request.to);
    let data = request.data.unwrap_or_default();
    let value = request.value.unwrap_or(U256::ZERO);
    let gas = request.gas.unwrap_or(21_000);
    let (_result, diff) = EvmExecutor::transact(&mut fork, from, to, data, value, gas)?;
    let current_hash = latest_local_tx_hash(&fork)?;
    let tx_hash = if let Some(expected_hash) = override_hash {
        adopt_latest_transaction_hash(&mut fork, current_hash, expected_hash)?;
        expected_hash
    } else {
        current_hash
    };
    let transaction = fork
        .transactions
        .get(&tx_hash)
        .cloned()
        .ok_or_else(|| MirageError::Unsupported("missing tx after commit".to_owned()))?;
    let receipt = fork
        .receipts
        .get(&tx_hash)
        .cloned()
        .ok_or_else(|| MirageError::Unsupported("missing receipt after commit".to_owned()))?;
    let block = fork
        .blocks_by_number
        .get(&transaction.block_number)
        .cloned()
        .ok_or_else(|| MirageError::Unsupported("missing block after commit".to_owned()))?;

    Ok(CommittedTransaction {
        tx_hash,
        block_number: transaction.block_number,
        diff,
        transaction,
        receipt,
        block,
    })
}

fn latest_local_tx_hash(state: &ForkState) -> Result<B256> {
    state
        .transactions
        .iter()
        .max_by_key(|(_, tx)| tx.block_number)
        .map(|(hash, _)| *hash)
        .ok_or_else(|| MirageError::Unsupported("missing tx after commit".to_owned()))
}

fn adopt_latest_transaction_hash(
    state: &mut ForkState,
    current_hash: B256,
    new_hash: B256,
) -> Result<()> {
    if current_hash == new_hash {
        return Ok(());
    }

    let mut tx = state.transactions.remove(&current_hash).ok_or_else(|| {
        MirageError::Unsupported("latest transaction missing from store".to_owned())
    })?;
    tx.hash = new_hash;
    let block_number = tx.block_number;
    state.transactions.insert(new_hash, tx);

    let mut receipt = state
        .receipts
        .remove(&current_hash)
        .ok_or_else(|| MirageError::Unsupported("latest receipt missing from store".to_owned()))?;
    receipt.transaction_hash = new_hash;
    state.receipts.insert(new_hash, receipt);

    if let Some(block) = state.blocks_by_number.get_mut(&block_number) {
        for hash in &mut block.transactions {
            if *hash == current_hash {
                *hash = new_hash;
            }
        }
    }
    if let Some(block) = state
        .blocks_by_hash
        .values_mut()
        .find(|block| block.number == block_number)
    {
        for hash in &mut block.transactions {
            if *hash == current_hash {
                *hash = new_hash;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct DecodedRawTransaction {
    tx_hash: B256,
    request: TransactionRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RlpValue {
    Bytes(Vec<u8>),
    List(Vec<Self>),
}

fn decode_signed_raw_transaction(raw: &Bytes) -> Result<DecodedRawTransaction> {
    if raw.is_empty() {
        return Err(MirageError::InvalidParams(
            "raw transaction is empty".to_owned(),
        ));
    }

    let tx_hash = keccak256(raw);
    match raw[0] {
        0x01 => decode_typed_raw_transaction(0x01, &raw[1..], tx_hash),
        0x02 => decode_typed_raw_transaction(0x02, &raw[1..], tx_hash),
        0x03 => decode_typed_raw_transaction(0x03, &raw[1..], tx_hash),
        _ => decode_legacy_raw_transaction(raw, tx_hash),
    }
}

fn decode_legacy_raw_transaction(raw: &[u8], tx_hash: B256) -> Result<DecodedRawTransaction> {
    let fields = rlp_decode_top_list(raw)?;
    if fields.len() != 9 {
        return Err(MirageError::InvalidParams(format!(
            "legacy transaction must have 9 fields, found {}",
            fields.len()
        )));
    }

    let nonce = rlp_u64(&fields[0])?;
    let gas_price = rlp_u128(&fields[1])?;
    let gas = rlp_u64(&fields[2])?;
    let to = rlp_address_opt(&fields[3])?;
    let value = rlp_u256(&fields[4])?;
    let data = Bytes::from(rlp_bytes(&fields[5])?);
    let v = rlp_u64(&fields[6])?;
    let r = rlp_u256(&fields[7])?;
    let s = rlp_u256(&fields[8])?;
    let (chain_id, recovery_id) = decode_legacy_v(v)?;

    let mut signing_fields = fields[..6].to_vec();
    if let Some(chain_id) = chain_id {
        signing_fields.push(rlp_from_u64(chain_id));
        signing_fields.push(RlpValue::Bytes(Vec::new()));
        signing_fields.push(RlpValue::Bytes(Vec::new()));
    }
    let signing_hash = keccak256(rlp_encode(&RlpValue::List(signing_fields)));
    let from = recover_address(signing_hash, r, s, recovery_id)?;

    Ok(DecodedRawTransaction {
        tx_hash,
        request: TransactionRequest {
            from: Some(from),
            to,
            gas: Some(gas),
            value: Some(value),
            data: Some(data),
            gas_price: Some(gas_price),
            nonce: Some(nonce),
            chain_id,
        },
    })
}

fn decode_typed_raw_transaction(
    tx_type: u8,
    raw: &[u8],
    tx_hash: B256,
) -> Result<DecodedRawTransaction> {
    let fields = rlp_decode_top_list(raw)?;
    let (chain_id, nonce, gas_price, gas, to, value, data, signing_fields, recovery_id, r, s) =
        match tx_type {
            0x01 => {
                if fields.len() != 11 {
                    return Err(MirageError::InvalidParams(format!(
                        "type 1 transaction must have 11 fields, found {}",
                        fields.len()
                    )));
                }
                (
                    Some(rlp_u64(&fields[0])?),
                    Some(rlp_u64(&fields[1])?),
                    Some(rlp_u128(&fields[2])?),
                    Some(rlp_u64(&fields[3])?),
                    rlp_address_opt(&fields[4])?,
                    Some(rlp_u256(&fields[5])?),
                    Bytes::from(rlp_bytes(&fields[6])?),
                    fields[..8].to_vec(),
                    rlp_u8(&fields[8])?,
                    rlp_u256(&fields[9])?,
                    rlp_u256(&fields[10])?,
                )
            }
            0x02 => {
                if fields.len() != 12 {
                    return Err(MirageError::InvalidParams(format!(
                        "type 2 transaction must have 12 fields, found {}",
                        fields.len()
                    )));
                }
                (
                    Some(rlp_u64(&fields[0])?),
                    Some(rlp_u64(&fields[1])?),
                    Some(rlp_u128(&fields[3])?),
                    Some(rlp_u64(&fields[4])?),
                    rlp_address_opt(&fields[5])?,
                    Some(rlp_u256(&fields[6])?),
                    Bytes::from(rlp_bytes(&fields[7])?),
                    fields[..9].to_vec(),
                    rlp_u8(&fields[9])?,
                    rlp_u256(&fields[10])?,
                    rlp_u256(&fields[11])?,
                )
            }
            0x03 => {
                if fields.len() != 14 {
                    return Err(MirageError::InvalidParams(format!(
                        "type 3 transaction must have 14 fields, found {}",
                        fields.len()
                    )));
                }
                (
                    Some(rlp_u64(&fields[0])?),
                    Some(rlp_u64(&fields[1])?),
                    Some(rlp_u128(&fields[3])?),
                    Some(rlp_u64(&fields[4])?),
                    rlp_address_opt(&fields[5])?,
                    Some(rlp_u256(&fields[6])?),
                    Bytes::from(rlp_bytes(&fields[7])?),
                    fields[..11].to_vec(),
                    rlp_u8(&fields[11])?,
                    rlp_u256(&fields[12])?,
                    rlp_u256(&fields[13])?,
                )
            }
            _ => {
                return Err(MirageError::InvalidParams(format!(
                    "unsupported typed transaction {tx_type:#x}"
                )));
            }
        };

    let mut payload = vec![tx_type];
    payload.extend_from_slice(&rlp_encode(&RlpValue::List(signing_fields)));
    let signing_hash = keccak256(payload);
    let from = recover_address(signing_hash, r, s, recovery_id)?;

    Ok(DecodedRawTransaction {
        tx_hash,
        request: TransactionRequest {
            from: Some(from),
            to,
            gas,
            value,
            data: Some(data),
            gas_price,
            nonce,
            chain_id,
        },
    })
}

fn decode_legacy_v(v: u64) -> Result<(Option<u64>, u8)> {
    match v {
        27 => Ok((None, 0)),
        28 => Ok((None, 1)),
        value if value >= 35 => {
            let adjusted = value - 35;
            let parity = u8::from(adjusted % 2 != 0);
            Ok((Some(adjusted / 2), parity))
        }
        _ => Err(MirageError::InvalidParams(format!(
            "invalid legacy v value {v}"
        ))),
    }
}

fn recover_address(signing_hash: B256, r: U256, s: U256, recovery_id: u8) -> Result<Address> {
    let recovery_id = RecoveryId::from_byte(recovery_id)
        .ok_or_else(|| MirageError::InvalidParams(format!("invalid recovery id {recovery_id}")))?;
    let signature =
        Signature::from_scalars(r.to_be_bytes::<32>(), s.to_be_bytes::<32>()).map_err(|error| {
            MirageError::InvalidParams(format!("invalid signature scalars: {error}"))
        })?;
    let verifying_key =
        VerifyingKey::recover_from_prehash(signing_hash.as_slice(), &signature, recovery_id)
            .map_err(|error| {
                MirageError::InvalidParams(format!("failed to recover sender: {error}"))
            })?;
    let encoded = verifying_key.to_encoded_point(false);
    let hash = keccak256(&encoded.as_bytes()[1..]);
    Ok(Address::from_slice(&hash.as_slice()[12..]))
}

fn rlp_decode_top_list(input: &[u8]) -> Result<Vec<RlpValue>> {
    let (value, consumed) = rlp_decode(input)?;
    if consumed != input.len() {
        return Err(MirageError::InvalidParams(
            "unexpected trailing RLP bytes".to_owned(),
        ));
    }
    match value {
        RlpValue::List(fields) => Ok(fields),
        RlpValue::Bytes(_) => Err(MirageError::InvalidParams(
            "expected top-level RLP list".to_owned(),
        )),
    }
}

fn rlp_decode(input: &[u8]) -> Result<(RlpValue, usize)> {
    let first = *input
        .first()
        .ok_or_else(|| MirageError::InvalidParams("unexpected end of RLP input".to_owned()))?;
    match first {
        0x00..=0x7f => Ok((RlpValue::Bytes(vec![first]), 1)),
        0x80..=0xb7 => {
            let len = usize::from(first - 0x80);
            let end = 1 + len;
            let bytes = input
                .get(1..end)
                .ok_or_else(|| MirageError::InvalidParams("short RLP string".to_owned()))?;
            Ok((RlpValue::Bytes(bytes.to_vec()), end))
        }
        0xb8..=0xbf => {
            let len_of_len = usize::from(first - 0xb7);
            let len = rlp_len(input, 1, len_of_len)?;
            let start = 1 + len_of_len;
            let end = start + len;
            let bytes = input
                .get(start..end)
                .ok_or_else(|| MirageError::InvalidParams("short long RLP string".to_owned()))?;
            Ok((RlpValue::Bytes(bytes.to_vec()), end))
        }
        0xc0..=0xf7 => {
            let len = usize::from(first - 0xc0);
            let start = 1;
            let end = start + len;
            let payload = input
                .get(start..end)
                .ok_or_else(|| MirageError::InvalidParams("short RLP list".to_owned()))?;
            Ok((RlpValue::List(rlp_decode_list_payload(payload)?), end))
        }
        0xf8..=0xff => {
            let len_of_len = usize::from(first - 0xf7);
            let len = rlp_len(input, 1, len_of_len)?;
            let start = 1 + len_of_len;
            let end = start + len;
            let payload = input
                .get(start..end)
                .ok_or_else(|| MirageError::InvalidParams("short long RLP list".to_owned()))?;
            Ok((RlpValue::List(rlp_decode_list_payload(payload)?), end))
        }
    }
}

fn rlp_decode_list_payload(mut payload: &[u8]) -> Result<Vec<RlpValue>> {
    let mut values = Vec::new();
    while !payload.is_empty() {
        let (value, consumed) = rlp_decode(payload)?;
        values.push(value);
        payload = &payload[consumed..];
    }
    Ok(values)
}

fn rlp_len(input: &[u8], start: usize, len_of_len: usize) -> Result<usize> {
    let end = start + len_of_len;
    let bytes = input
        .get(start..end)
        .ok_or_else(|| MirageError::InvalidParams("short RLP length".to_owned()))?;
    bytes.iter().try_fold(0_usize, |acc, byte| {
        acc.checked_mul(256)
            .and_then(|value| value.checked_add(usize::from(*byte)))
            .ok_or_else(|| MirageError::InvalidParams("RLP length overflow".to_owned()))
    })
}

fn rlp_bytes(value: &RlpValue) -> Result<Vec<u8>> {
    match value {
        RlpValue::Bytes(bytes) => Ok(bytes.clone()),
        RlpValue::List(_) => Err(MirageError::InvalidParams("expected RLP bytes".to_owned())),
    }
}

fn rlp_u64(value: &RlpValue) -> Result<u64> {
    let bytes = rlp_bytes(value)?;
    if bytes.is_empty() {
        return Ok(0);
    }
    if bytes.len() > 8 {
        return Err(MirageError::InvalidParams(
            "integer does not fit in u64".to_owned(),
        ));
    }
    Ok(bytes
        .into_iter()
        .fold(0_u64, |acc, byte| (acc << 8) | u64::from(byte)))
}

fn rlp_u128(value: &RlpValue) -> Result<u128> {
    let bytes = rlp_bytes(value)?;
    if bytes.is_empty() {
        return Ok(0);
    }
    if bytes.len() > 16 {
        return Err(MirageError::InvalidParams(
            "integer does not fit in u128".to_owned(),
        ));
    }
    Ok(bytes
        .into_iter()
        .fold(0_u128, |acc, byte| (acc << 8) | u128::from(byte)))
}

fn rlp_u8(value: &RlpValue) -> Result<u8> {
    let value = rlp_u64(value)?;
    u8::try_from(value)
        .map_err(|_| MirageError::InvalidParams(format!("integer does not fit in u8: {value}")))
}

fn rlp_u256(value: &RlpValue) -> Result<U256> {
    Ok(U256::from_be_slice(&rlp_bytes(value)?))
}

fn rlp_address_opt(value: &RlpValue) -> Result<Option<Address>> {
    let bytes = rlp_bytes(value)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() != 20 {
        return Err(MirageError::InvalidParams(format!(
            "address must be 20 bytes, found {}",
            bytes.len()
        )));
    }
    Ok(Some(Address::from_slice(&bytes)))
}

fn rlp_from_u64(value: u64) -> RlpValue {
    if value == 0 {
        return RlpValue::Bytes(Vec::new());
    }
    RlpValue::Bytes(trim_leading_zeros(value.to_be_bytes().to_vec()))
}

fn rlp_encode(value: &RlpValue) -> Vec<u8> {
    match value {
        RlpValue::Bytes(bytes) => rlp_encode_bytes(bytes),
        RlpValue::List(items) => {
            let payload = items.iter().flat_map(rlp_encode).collect::<Vec<_>>();
            rlp_encode_with_offset(&payload, 0xc0, 0xf7)
        }
    }
}

fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        vec![bytes[0]]
    } else {
        rlp_encode_with_offset(bytes, 0x80, 0xb7)
    }
}

fn rlp_encode_with_offset(payload: &[u8], short_offset: u8, long_offset: u8) -> Vec<u8> {
    if payload.len() <= 55 {
        let mut encoded = Vec::with_capacity(1 + payload.len());
        let short_len = u8::try_from(payload.len())
            .unwrap_or_else(|_| unreachable!("short RLP payload length always fits in u8"));
        encoded.push(short_offset + short_len);
        encoded.extend_from_slice(payload);
        encoded
    } else {
        let length_bytes = trim_leading_zeros((payload.len() as u64).to_be_bytes().to_vec());
        let mut encoded = Vec::with_capacity(1 + length_bytes.len() + payload.len());
        let length_of_length = u8::try_from(length_bytes.len())
            .unwrap_or_else(|_| unreachable!("RLP length-of-length always fits in u8"));
        encoded.push(long_offset + length_of_length);
        encoded.extend_from_slice(&length_bytes);
        encoded.extend_from_slice(payload);
        encoded
    }
}

fn trim_leading_zeros(mut bytes: Vec<u8>) -> Vec<u8> {
    let first_non_zero = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    bytes.drain(..first_non_zero);
    bytes
}

fn invalid_params(error: impl std::fmt::Display) -> ErrorObjectOwned {
    invalid_params_message(error.to_string())
}

fn invalid_params_message(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(-32602, message.into(), None::<()>)
}

fn rpc_error(error: MirageError) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(error.rpc_code(), error.to_string(), None::<()>)
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Arc, time::Duration};

    use alloy_primitives::{Address, B256, Bytes, U256, address, keccak256};
    use k256::{
        FieldBytes,
        ecdsa::{SigningKey, hazmat::SignPrimitive},
        sha2,
    };
    use tokio::{sync::broadcast, time::sleep};

    use super::{
        RlpValue, ServerContext, apply_pressure_action, build_rpc_module,
        commit_transaction_request, decode_signed_raw_transaction, from_sqrt_price_x96,
        parse_hex_quantity, rlp_encode, rlp_from_u64, rpc_error, run_fork_snapshot,
        stage_erc20_mint, to_sqrt_price_x96,
    };
    use crate::{
        MirageError, TransactionRequest,
        fork::{
            ClassificationConfig, DiffClassifier, ForkState, HybridDB, MirageFork, WatchEntry,
            WatchSource, with_state_write,
        },
        integration::MIRAGE_WATCH_CONTRACT_METHOD,
        provider::UpstreamRpc,
        resources::{MirageMode, PressureAction, Profile, ResourceModel},
        scenario::{JobStatus, Scenario, ScenarioJob},
    };

    fn test_rpc_module() -> (jsonrpsee::RpcModule<ServerContext>, ServerContext) {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(1);
        let context = ServerContext {
            state: mirage.state(),
            shutdown,
            #[cfg(feature = "chain")]
            chain: None,
            #[cfg(feature = "roko")]
            chain_subs: None,
        };
        let module = build_rpc_module(context.clone())
            .unwrap_or_else(|error| panic!("build rpc module: {error}"));
        (module, context)
    }

    #[test]
    fn test_rpc_error_codes_match_plan_table() {
        let cases = [
            (MirageError::SnapshotNotFound(1), -32001),
            (MirageError::InvalidFrom(Address::ZERO), -32010),
            (MirageError::SlotDetectionFailed(Address::ZERO), -32020),
            (MirageError::WatchListFull, -32030),
            (MirageError::UnknownProtocolType("x".to_owned()), -32040),
            (MirageError::SetNotFound("set".to_owned()), -32050),
            (MirageError::SetAlreadyRunning("set".to_owned()), -32051),
            (MirageError::SetHasNoScenarios("set".to_owned()), -32052),
            (MirageError::JobNotFound("job".to_owned()), -32054),
            (MirageError::JobNotComplete("job".to_owned()), -32055),
            (MirageError::Upstream("err".to_owned()), -32099),
        ];

        for (error, expected_code) in cases {
            assert_eq!(rpc_error(error).code(), expected_code);
        }
    }

    #[tokio::test]
    async fn scenario_run_empty_set_returns_minus_32052() {
        use jsonrpsee::core::server::MethodsError;

        let (module, _context) = test_rpc_module();
        let set_id: String = module
            .call("mirage_beginScenarioSet", ("latest",))
            .await
            .expect("begin scenario set");
        let err = module
            .call::<_, String>("mirage_runScenarioSet", (set_id, "sequential"))
            .await
            .expect_err("run without scenarios");
        match err {
            MethodsError::JsonRpc(obj) => assert_eq!(obj.code(), -32052),
            other => panic!("expected JSON-RPC error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scenario_define_after_set_completes_returns_minus_32051() {
        use jsonrpsee::core::server::MethodsError;
        use std::time::Duration;

        let (module, _context) = test_rpc_module();
        let set_id: String = module
            .call("mirage_beginScenarioSet", ("latest",))
            .await
            .expect("begin scenario set");
        let scenario = Scenario {
            id: "s1".to_owned(),
            name: "noop".to_owned(),
            transactions: Vec::new(),
            track_addresses: Vec::new(),
            max_gas: None,
            timeout: Duration::from_secs(10),
            assertions: Default::default(),
        };
        module
            .call::<_, String>("mirage_defineScenario", (set_id.clone(), scenario))
            .await
            .expect("define scenario");
        let job_id: String = module
            .call("mirage_runScenarioSet", (set_id.clone(), "sequential"))
            .await
            .expect("run scenario set");
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let job: ScenarioJob = module
                .call("mirage_getScenarioResults", (job_id.clone(),))
                .await
                .expect("job status");
            if job.status == JobStatus::Complete {
                break;
            }
        }
        let second = Scenario {
            id: "s2".to_owned(),
            name: "late".to_owned(),
            transactions: Vec::new(),
            track_addresses: Vec::new(),
            max_gas: None,
            timeout: Duration::from_secs(10),
            assertions: Default::default(),
        };
        let err = module
            .call::<_, String>("mirage_defineScenario", (set_id, second))
            .await
            .expect_err("define after set left draft");
        match err {
            MethodsError::JsonRpc(obj) => assert_eq!(obj.code(), -32051),
            other => panic!("expected JSON-RPC error, got {other:?}"),
        }
    }

    #[test]
    fn build_rpc_module_registers_required_eth_methods() {
        let (module, _context) = test_rpc_module();

        for method in [
            "eth_blockNumber",
            "eth_chainId",
            "eth_getBalance",
            "eth_getStorageAt",
            "eth_getCode",
            "eth_getTransactionCount",
            "eth_call",
            "eth_sendTransaction",
            "eth_sendRawTransaction",
            "eth_getTransactionReceipt",
            "eth_getTransactionByHash",
            "eth_getLogs",
            "eth_getBlockByNumber",
            "eth_getBlockByHash",
            "eth_estimateGas",
            "eth_gasPrice",
            "eth_feeHistory",
            "eth_maxPriorityFeePerGas",
        ] {
            assert!(
                module.method(method).is_some(),
                "{method} should be registered"
            );
        }
    }

    #[test]
    fn hardhat_mine_count_parses_hex_string_or_json_number() {
        assert_eq!(super::parse_mine_block_count(&[]), 1);
        assert_eq!(
            super::parse_mine_block_count(&[serde_json::json!("0x4")]),
            4
        );
        assert_eq!(super::parse_mine_block_count(&[serde_json::json!(7)]), 7);
    }

    #[test]
    fn eth_fee_history_response_matches_block_count() {
        let value = super::build_fee_history_response(
            serde_json::json!(3),
            serde_json::json!("latest"),
            Some(serde_json::json!([25, 75])),
        )
        .expect("fee history");
        assert_eq!(value["baseFeePerGas"].as_array().expect("baseFee").len(), 4);
        assert_eq!(value["gasUsedRatio"].as_array().expect("ratio").len(), 3);
        let reward = value["reward"].as_array().expect("reward");
        assert_eq!(reward.len(), 3);
        assert_eq!(reward[0].as_array().expect("tier").len(), 2);
    }

    #[test]
    fn build_rpc_module_registers_required_hardhat_anvil_methods() {
        let (module, _context) = test_rpc_module();

        for method in [
            "hardhat_impersonateAccount",
            "anvil_impersonateAccount",
            "hardhat_stopImpersonatingAccount",
            "anvil_stopImpersonatingAccount",
            "hardhat_setBalance",
            "anvil_setBalance",
            "hardhat_setStorageAt",
            "anvil_setStorageAt",
            "hardhat_setCode",
            "anvil_setCode",
            "hardhat_setNonce",
            "anvil_setNonce",
            "hardhat_mine",
            "anvil_mine",
            "hardhat_reset",
            "anvil_reset",
            "hardhat_setNextBlockBaseFeePerGas",
            "anvil_setNextBlockBaseFeePerGas",
            "hardhat_setCoinbase",
            "anvil_setCoinbase",
            "hardhat_setPrevRandao",
            "anvil_setPrevRandao",
        ] {
            assert!(
                module.method(method).is_some(),
                "{method} should be registered"
            );
        }
    }

    #[test]
    fn build_rpc_module_registers_required_evm_and_mirage_methods() {
        let (module, _context) = test_rpc_module();

        for method in [
            "evm_snapshot",
            "evm_revert",
            "evm_mine",
            "evm_increaseTime",
            "evm_setNextBlockTimestamp",
            "mirage_setBalance",
            "mirage_setCode",
            "mirage_setStorageAt",
            "mirage_mintERC20",
            "mirage_prefetchSlots",
            "mirage_prefetchAccount",
            "mirage_watchContract",
            "mirage_unwatchContract",
            "mirage_getWatchList",
            "mirage_getDirtySlots",
            "mirage_getLastStateDiff",
            "mirage_status",
            "mirage_getResourceUsage",
            "mirage_setResourceLimits",
            "mirage_getPosition",
            "mirage_subscribeEvents",
            "mirage_beginScenarioSet",
            "mirage_defineScenario",
            "mirage_runScenarioSet",
            "mirage_getScenarioResults",
            "mirage_compareScenarios",
            "mirage_computeDomainSeparator",
            "mirage_cleanup",
            "mirage_shutdown",
        ] {
            assert!(
                module.method(method).is_some(),
                "{method} should be registered"
            );
        }
    }

    #[tokio::test]
    async fn test_account_impersonation_validity() {
        let (module, context) = test_rpc_module();
        let sender = address!("0x6100000000000000000000000000000000000001");
        let receiver = address!("0x6100000000000000000000000000000000000002");
        let target = address!("0x6100000000000000000000000000000000000003");
        let storage_slot = U256::from(7_u64);
        let hardhat_balance = U256::from(42_u64);
        let anvil_balance = U256::from(99_u64);
        let hardhat_nonce = 7_u64;
        let anvil_nonce = 11_u64;
        let hardhat_code = Bytes::from_static(&[0x60, 0x01, 0x60, 0x00, 0x55]);
        let anvil_code = Bytes::from_static(&[0x60, 0x02, 0x60, 0x00, 0x55]);
        let hardhat_base_fee = 123_u128;
        let anvil_base_fee = 456_u128;
        let hardhat_coinbase = address!("0x6100000000000000000000000000000000000004");
        let anvil_coinbase = address!("0x6100000000000000000000000000000000000005");
        let hardhat_prev_randao = B256::from([0x11; 32]);
        let anvil_prev_randao = B256::from([0x22; 32]);

        assert!(
            module
                .call::<_, bool>("hardhat_impersonateAccount", (sender,))
                .await
                .unwrap_or_else(|error| panic!("hardhat impersonation succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_impersonateAccount", (receiver,))
                .await
                .unwrap_or_else(|error| panic!("anvil impersonation succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert!(state.fork.impersonated_accounts.contains(&sender));
            assert!(state.fork.impersonated_accounts.contains(&receiver));
        }

        let sender_baseline_balance: String = module
            .call("eth_getBalance", (sender, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender baseline balance: {error}"));
        let receiver_baseline_balance: String = module
            .call("eth_getBalance", (receiver, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver baseline balance: {error}"));

        assert!(
            module
                .call::<_, bool>("hardhat_setBalance", (sender, hardhat_balance))
                .await
                .unwrap_or_else(|error| panic!("hardhat setBalance succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_setBalance", (receiver, anvil_balance))
                .await
                .unwrap_or_else(|error| panic!("anvil setBalance succeeds: {error}"))
        );
        let sender_balance: String = module
            .call("eth_getBalance", (sender, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender balance: {error}"));
        let receiver_balance: String = module
            .call("eth_getBalance", (receiver, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver balance: {error}"));
        assert_eq!(sender_balance, format!("0x{:x}", hardhat_balance));
        assert_eq!(receiver_balance, format!("0x{:x}", anvil_balance));

        assert!(
            module
                .call::<_, bool>(
                    "hardhat_setStorageAt",
                    (sender, storage_slot, hardhat_balance)
                )
                .await
                .unwrap_or_else(|error| panic!("hardhat setStorageAt succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>(
                    "anvil_setStorageAt",
                    (receiver, storage_slot, anvil_balance)
                )
                .await
                .unwrap_or_else(|error| panic!("anvil setStorageAt succeeds: {error}"))
        );
        let sender_storage: String = module
            .call("eth_getStorageAt", (sender, storage_slot, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender storage: {error}"));
        let receiver_storage: String = module
            .call("eth_getStorageAt", (receiver, storage_slot, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver storage: {error}"));
        assert_eq!(sender_storage, format!("0x{:064x}", hardhat_balance));
        assert_eq!(receiver_storage, format!("0x{:064x}", anvil_balance));

        assert!(
            module
                .call::<_, bool>("hardhat_setCode", (sender, hardhat_code.clone()))
                .await
                .unwrap_or_else(|error| panic!("hardhat setCode succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_setCode", (receiver, anvil_code.clone()))
                .await
                .unwrap_or_else(|error| panic!("anvil setCode succeeds: {error}"))
        );
        let sender_code: String = module
            .call("eth_getCode", (sender, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender code: {error}"));
        let receiver_code: String = module
            .call("eth_getCode", (receiver, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver code: {error}"));
        assert_eq!(sender_code, "0x6001600055");
        assert_eq!(receiver_code, "0x6002600055");

        assert!(
            module
                .call::<_, bool>("hardhat_setNonce", (sender, hardhat_nonce))
                .await
                .unwrap_or_else(|error| panic!("hardhat setNonce succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_setNonce", (receiver, anvil_nonce))
                .await
                .unwrap_or_else(|error| panic!("anvil setNonce succeeds: {error}"))
        );
        let sender_nonce: String = module
            .call("eth_getTransactionCount", (sender, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read sender nonce: {error}"));
        let receiver_nonce: String = module
            .call("eth_getTransactionCount", (receiver, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read receiver nonce: {error}"));
        assert_eq!(sender_nonce, format!("0x{hardhat_nonce:x}"));
        assert_eq!(receiver_nonce, format!("0x{anvil_nonce:x}"));

        assert!(
            module
                .call::<_, bool>("hardhat_setNextBlockBaseFeePerGas", (hardhat_base_fee,))
                .await
                .unwrap_or_else(|error| panic!("hardhat base fee succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_setNextBlockBaseFeePerGas", (anvil_base_fee,))
                .await
                .unwrap_or_else(|error| panic!("anvil base fee succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.next_base_fee_per_gas, anvil_base_fee);
        }

        assert!(
            module
                .call::<_, bool>("hardhat_setCoinbase", (hardhat_coinbase,))
                .await
                .unwrap_or_else(|error| panic!("hardhat coinbase succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.coinbase, hardhat_coinbase);
        }
        assert!(
            module
                .call::<_, bool>("anvil_setCoinbase", (anvil_coinbase,))
                .await
                .unwrap_or_else(|error| panic!("anvil coinbase succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.coinbase, anvil_coinbase);
        }

        assert!(
            module
                .call::<_, bool>("hardhat_setPrevRandao", (hardhat_prev_randao,))
                .await
                .unwrap_or_else(|error| panic!("hardhat prevRandao succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.prev_randao, hardhat_prev_randao);
        }
        assert!(
            module
                .call::<_, bool>("anvil_setPrevRandao", (anvil_prev_randao,))
                .await
                .unwrap_or_else(|error| panic!("anvil prevRandao succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert_eq!(state.fork.prev_randao, anvil_prev_randao);
        }

        let block_before: u64 = parse_hex_quantity(
            &module
                .call::<_, String>("eth_blockNumber", Vec::<u8>::new())
                .await
                .unwrap_or_else(|error| panic!("read block number before mine: {error}")),
        )
        .unwrap_or_else(|error| panic!("parse block number before mine: {error}"));
        assert!(
            module
                .call::<_, bool>("hardhat_mine", ("0x2",))
                .await
                .unwrap_or_else(|error| panic!("hardhat mine succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_mine", ("0x3",))
                .await
                .unwrap_or_else(|error| panic!("anvil mine succeeds: {error}"))
        );
        let block_after: u64 = parse_hex_quantity(
            &module
                .call::<_, String>("eth_blockNumber", Vec::<u8>::new())
                .await
                .unwrap_or_else(|error| panic!("read block number after mine: {error}")),
        )
        .unwrap_or_else(|error| panic!("parse block number after mine: {error}"));
        assert_eq!(block_after, block_before + 5);

        let tx_hash: B256 = module
            .call(
                "eth_sendTransaction",
                (TransactionRequest {
                    from: Some(sender),
                    to: Some(target),
                    gas: Some(21_000),
                    value: Some(U256::ZERO),
                    data: None,
                    gas_price: None,
                    nonce: None,
                    chain_id: None,
                },),
            )
            .await
            .unwrap_or_else(|error| panic!("impersonated transaction succeeds: {error}"));
        assert_ne!(tx_hash, B256::ZERO);

        assert!(
            module
                .call::<_, bool>("hardhat_stopImpersonatingAccount", (sender,))
                .await
                .unwrap_or_else(|error| panic!("hardhat stop impersonation succeeds: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_stopImpersonatingAccount", (receiver,))
                .await
                .unwrap_or_else(|error| panic!("anvil stop impersonation succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert!(state.fork.impersonated_accounts.is_empty());
        }

        assert!(
            module
                .call::<_, bool>(MIRAGE_WATCH_CONTRACT_METHOD, (target,))
                .await
                .unwrap_or_else(|error| panic!("mirage watch contract succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert!(state.fork.db.dirty.watch_list.contains_key(&target));
        }

        assert!(
            module
                .call::<_, bool>("hardhat_reset", Vec::<u8>::new())
                .await
                .unwrap_or_else(|error| panic!("hardhat reset succeeds: {error}"))
        );
        {
            let state = context.state.read();
            assert!(
                state.fork.db.dirty.watch_list.is_empty(),
                "reset clears watch list"
            );
            assert!(
                state.fork.db.dirty.unwatch_list.is_empty(),
                "reset clears unwatch list"
            );
            assert!(
                state.fork.impersonated_accounts.is_empty(),
                "reset clears impersonation set"
            );
        }
        assert_eq!(
            module
                .call::<_, String>("eth_getBalance", (sender, "latest"))
                .await
                .unwrap_or_else(|error| panic!("balance after hardhat reset: {error}")),
            sender_baseline_balance
        );
        assert_eq!(
            module
                .call::<_, String>("eth_getCode", (sender, "latest"))
                .await
                .unwrap_or_else(|error| panic!("code after hardhat reset: {error}")),
            "0x"
        );
        assert_eq!(
            module
                .call::<_, String>("eth_getStorageAt", (sender, storage_slot, "latest"))
                .await
                .unwrap_or_else(|error| panic!("storage after hardhat reset: {error}")),
            format!("0x{:064x}", U256::ZERO)
        );
        assert_eq!(
            module
                .call::<_, String>("eth_getTransactionCount", (sender, "latest"))
                .await
                .unwrap_or_else(|error| panic!("nonce after hardhat reset: {error}")),
            "0x0"
        );

        assert!(
            module
                .call::<_, bool>("anvil_setBalance", (receiver, anvil_balance))
                .await
                .unwrap_or_else(|error| panic!("reapply receiver balance: {error}"))
        );
        assert!(
            module
                .call::<_, bool>("anvil_reset", Vec::<u8>::new())
                .await
                .unwrap_or_else(|error| panic!("anvil reset succeeds: {error}"))
        );
        assert_eq!(
            module
                .call::<_, String>("eth_getBalance", (receiver, "latest"))
                .await
                .unwrap_or_else(|error| panic!("balance after anvil reset: {error}")),
            receiver_baseline_balance
        );
    }

    #[test]
    fn decode_raw_legacy_transaction() {
        let signing_key = SigningKey::from_bytes((&[7_u8; 32]).into())
            .unwrap_or_else(|error| panic!("signing key: {error}"));
        let from = signing_key_address(&signing_key);
        let to = address!("0x1000000000000000000000000000000000000001");
        let raw = sign_legacy(&signing_key, 1, 5, 21_000, Some(to), 9, &[0xde, 0xad]);
        let decoded = decode_signed_raw_transaction(&raw)
            .unwrap_or_else(|error| panic!("decode legacy: {error}"));
        assert_transaction(
            decoded.request,
            from,
            Some(to),
            21_000,
            9,
            &[0xde, 0xad],
            Some(1),
            Some(5),
        );
    }

    #[test]
    fn decode_raw_typed_transactions() {
        let signing_key = SigningKey::from_bytes((&[9_u8; 32]).into())
            .unwrap_or_else(|error| panic!("signing key: {error}"));
        let from = signing_key_address(&signing_key);
        let to = address!("0x2000000000000000000000000000000000000002");

        let type1 = sign_typed(&signing_key, 0x01, Some(to));
        let decoded1 = decode_signed_raw_transaction(&type1)
            .unwrap_or_else(|error| panic!("decode type1: {error}"));
        assert_transaction(
            decoded1.request,
            from,
            Some(to),
            80_000,
            11,
            &[0x01, 0x02],
            Some(1),
            Some(3),
        );

        let type2 = sign_typed(&signing_key, 0x02, Some(to));
        let decoded2 = decode_signed_raw_transaction(&type2)
            .unwrap_or_else(|error| panic!("decode type2: {error}"));
        assert_transaction(
            decoded2.request,
            from,
            Some(to),
            80_000,
            11,
            &[0x01, 0x02],
            Some(1),
            Some(4),
        );

        let type3 = sign_typed(&signing_key, 0x03, None);
        let decoded3 = decode_signed_raw_transaction(&type3)
            .unwrap_or_else(|error| panic!("decode type3: {error}"));
        assert_transaction(
            decoded3.request,
            from,
            None,
            80_000,
            11,
            &[0x01, 0x02],
            Some(1),
            Some(4),
        );
    }

    #[test]
    fn sqrt_price_x96_round_trip_matches_expected_price() {
        let price = 1_800.0;
        let encoded = to_sqrt_price_x96(price);
        let decoded = from_sqrt_price_x96(encoded);
        let relative_error = (decoded - price).abs() / price;
        assert!(
            relative_error < 1e-9,
            "decoded={decoded} price={price} relative_error={relative_error}"
        );
    }

    #[tokio::test]
    async fn run_fork_snapshot_reads_dirty_state_without_holding_server_lock() {
        let address = address!("0x3000000000000000000000000000000000000003");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(1);
        let context = ServerContext {
            state: mirage.state(),
            shutdown,
            #[cfg(feature = "chain")]
            chain: None,
            #[cfg(feature = "roko")]
            chain_subs: None,
        };
        context
            .state
            .write()
            .fork
            .db
            .set_balance(address, U256::from(42_u64));

        let observed = run_fork_snapshot(&context.state, true, move |mut fork| {
            Ok(fork.db.basic(address)?.unwrap_or_default().balance)
        })
        .await
        .unwrap_or_else(|error| panic!("snapshot read succeeds: {error}"));

        assert_eq!(observed, U256::from(42_u64));
    }

    #[tokio::test]
    async fn commit_transaction_request_runs_without_holding_state_write_lock() {
        let from = address!("0x3100000000000000000000000000000000000001");
        let to = address!("0x3100000000000000000000000000000000000002");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(150));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );

        let state = mirage.state();
        let request = TransactionRequest {
            from: Some(from),
            to: Some(to),
            gas: Some(21_000),
            value: Some(U256::from(1_u64)),
            data: None,
            gas_price: None,
            nonce: None,
            chain_id: None,
        };

        let state_for_task = Arc::clone(&state);
        let task = tokio::spawn(async move {
            commit_transaction_request(&state_for_task, request, None).await
        });
        sleep(Duration::from_millis(20)).await;

        let started = std::time::Instant::now();
        let read_guard = state.read();
        assert!(started.elapsed() < Duration::from_millis(75));
        drop(read_guard);

        let tx_hash = task
            .await
            .unwrap_or_else(|error| panic!("join tx task: {error}"))
            .unwrap_or_else(|error| panic!("commit transaction: {error}"));
        assert_ne!(tx_hash, B256::ZERO);
    }

    #[tokio::test]
    async fn commit_transaction_request_releases_writer_gate_during_blocking_execution() {
        let from = address!("0x3200000000000000000000000000000000000001");
        let to = address!("0x3200000000000000000000000000000000000002");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(150));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );

        let state = mirage.state();
        let request = TransactionRequest {
            from: Some(from),
            to: Some(to),
            gas: Some(21_000),
            value: Some(U256::from(1_u64)),
            data: None,
            gas_price: None,
            nonce: None,
            chain_id: None,
        };

        let state_for_task = Arc::clone(&state);
        let task = tokio::spawn(async move {
            commit_transaction_request(&state_for_task, request, None).await
        });
        sleep(Duration::from_millis(20)).await;

        let started = std::time::Instant::now();
        with_state_write(&state, |state| {
            state.reject_new_forks = !state.reject_new_forks;
        })
        .await;
        assert!(started.elapsed() < Duration::from_millis(75));

        let tx_hash = task
            .await
            .unwrap_or_else(|error| panic!("join tx task: {error}"))
            .unwrap_or_else(|error| panic!("commit transaction: {error}"));
        assert_ne!(tx_hash, B256::ZERO);
    }

    #[tokio::test]
    async fn stage_erc20_mint_releases_writer_gate_during_blocking_reads() {
        let token = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let owner = address!("0x3300000000000000000000000000000000000002");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(150));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );

        let state = mirage.state();
        let state_for_task = Arc::clone(&state);
        let task = tokio::spawn(async move {
            stage_erc20_mint(&state_for_task, token, owner, U256::from(5_u64)).await
        });
        sleep(Duration::from_millis(20)).await;

        let started = std::time::Instant::now();
        with_state_write(&state, |state| {
            state.reject_new_forks = !state.reject_new_forks;
        })
        .await;
        assert!(started.elapsed() < Duration::from_millis(75));

        let staged = task
            .await
            .unwrap_or_else(|error| panic!("join mint task: {error}"))
            .unwrap_or_else(|error| panic!("stage mint: {error}"));
        assert!(!staged.storage_writes.is_empty());
    }

    #[test]
    fn throttle_pressure_demotes_new_contracts_to_slot_only() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let state_handle = mirage.state();
        let mut state = state_handle.write();
        let manual = address!("0x4000000000000000000000000000000000000004");
        let auto = address!("0x5000000000000000000000000000000000000005");
        state.fork.db.dirty.watch_list.insert(
            manual,
            WatchEntry {
                source: WatchSource::Manual,
                added_at_block: 0,
                initial_slot_count: 0,
                replay_count: 0,
            },
        );
        state.fork.db.dirty.watch_list.insert(
            auto,
            WatchEntry {
                source: WatchSource::AutoClassified,
                added_at_block: 0,
                initial_slot_count: 0,
                replay_count: 0,
            },
        );
        state.reject_new_forks = false;
        state.fork.db.dirty.demote_protocols_to_slot_only = false;

        let classifier = DiffClassifier::new(ClassificationConfig::default());
        let mut diff = crate::StateDiff::success(21_000, Bytes::default());
        diff.accounts.insert(
            address!("0x4100000000000000000000000000000000000004"),
            crate::AccountDiff {
                info_changed: true,
                new_balance: None,
                new_nonce: None,
                new_code: None,
                storage_written: [
                    (U256::from(1), U256::from(1)),
                    (U256::from(2), U256::from(2)),
                    (U256::from(3), U256::from(3)),
                ]
                .into_iter()
                .collect(),
                storage_read: Default::default(),
            },
        );

        apply_pressure_action(&mut state, PressureAction::Throttle);
        classifier
            .apply(&mut state.fork.db.dirty, &diff, 1)
            .unwrap_or_else(|error| panic!("classifier apply succeeds: {error}"));

        assert!(!state.reject_new_forks);
        assert!(state.fork.db.dirty.demote_protocols_to_slot_only);
        assert!(state.fork.db.dirty.watch_list.contains_key(&manual));
        assert!(state.fork.db.dirty.watch_list.contains_key(&auto));
        assert!(
            !state
                .fork
                .db
                .dirty
                .watch_list
                .contains_key(&address!("0x4100000000000000000000000000000000000004"))
        );
        assert_eq!(state.mode, MirageMode::Live);
    }

    fn assert_transaction(
        request: TransactionRequest,
        from: Address,
        to: Option<Address>,
        gas: u64,
        value: u64,
        data: &[u8],
        chain_id: Option<u64>,
        gas_price: Option<u128>,
    ) {
        assert_eq!(request.from, Some(from));
        assert_eq!(request.to, to);
        assert_eq!(request.gas, Some(gas));
        assert_eq!(request.value, Some(U256::from(value)));
        assert_eq!(request.data.as_ref().map(Bytes::as_ref), Some(data));
        assert_eq!(request.chain_id, chain_id);
        assert_eq!(request.gas_price, gas_price);
    }

    fn signing_key_address(signing_key: &SigningKey) -> Address {
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let hash = keccak256(&encoded.as_bytes()[1..]);
        Address::from_slice(&hash.as_slice()[12..])
    }

    fn sign_legacy(
        signing_key: &SigningKey,
        chain_id: u64,
        gas_price: u64,
        gas_limit: u64,
        to: Option<Address>,
        value: u64,
        data: &[u8],
    ) -> Bytes {
        let unsigned = RlpValue::List(vec![
            rlp_from_u64(0),
            rlp_from_u64(gas_price),
            rlp_from_u64(gas_limit),
            to.map_or_else(
                || RlpValue::Bytes(Vec::new()),
                |value| RlpValue::Bytes(value.as_slice().to_vec()),
            ),
            rlp_from_u64(value),
            RlpValue::Bytes(data.to_vec()),
            rlp_from_u64(chain_id),
            RlpValue::Bytes(Vec::new()),
            RlpValue::Bytes(Vec::new()),
        ]);
        let unsigned_rlp = rlp_encode(&unsigned);
        let hash = keccak256(&unsigned_rlp);
        let mut field_bytes = FieldBytes::default();
        field_bytes.copy_from_slice(hash.as_slice());
        let (signature, recovery_id) = signing_key
            .as_nonzero_scalar()
            .try_sign_prehashed_rfc6979::<sha2::Sha256>(&field_bytes, &[])
            .unwrap_or_else(|error| panic!("sign legacy prehash: {error}"));
        let recovery_id = recovery_id.unwrap_or_else(|| panic!("legacy recovery id present"));
        let v = chain_id * 2 + 35 + u64::from(recovery_id.to_byte());
        let signed = RlpValue::List(vec![
            rlp_from_u64(0),
            rlp_from_u64(gas_price),
            rlp_from_u64(gas_limit),
            to.map_or_else(
                || RlpValue::Bytes(Vec::new()),
                |value| RlpValue::Bytes(value.as_slice().to_vec()),
            ),
            rlp_from_u64(value),
            RlpValue::Bytes(data.to_vec()),
            rlp_from_u64(v),
            RlpValue::Bytes(signature.r().to_bytes().to_vec()),
            RlpValue::Bytes(signature.s().to_bytes().to_vec()),
        ]);
        Bytes::from(rlp_encode(&signed))
    }

    #[tokio::test]
    async fn evm_snapshot_captures_dirty_store_and_revert_is_single_use() {
        let addr = address!("0x6000000000000000000000000000000000000006");
        let (module, _context) = test_rpc_module();

        assert!(
            module
                .call::<_, bool>("mirage_setBalance", (addr, U256::from(100_u64)))
                .await
                .unwrap_or_else(|error| panic!("set initial balance: {error}"))
        );

        let snapshot_raw: String = module
            .call("evm_snapshot", Vec::<u8>::new())
            .await
            .unwrap_or_else(|error| panic!("take snapshot via rpc: {error}"));
        let snapshot_id = parse_hex_quantity(&snapshot_raw)
            .unwrap_or_else(|error| panic!("parse snapshot id {snapshot_raw}: {error}"));

        assert!(
            module
                .call::<_, bool>("mirage_setBalance", (addr, U256::from(999_u64)))
                .await
                .unwrap_or_else(|error| panic!("set modified balance: {error}"))
        );

        let balance_after_modify: String = module
            .call("eth_getBalance", (addr, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read modified balance via rpc: {error}"));
        assert_eq!(balance_after_modify, format!("0x{:x}", U256::from(999_u64)));

        let reverted: bool = module
            .call("evm_revert", (format!("0x{snapshot_id:x}"),))
            .await
            .unwrap_or_else(|error| panic!("revert snapshot via rpc: {error}"));
        assert!(reverted);

        let balance_after_revert: String = module
            .call("eth_getBalance", (addr, "latest"))
            .await
            .unwrap_or_else(|error| panic!("read reverted balance via rpc: {error}"));
        assert_eq!(balance_after_revert, format!("0x{:x}", U256::from(100_u64)));

        let second_revert = module
            .call::<_, bool>("evm_revert", (format!("0x{snapshot_id:x}"),))
            .await
            .unwrap_err();
        let second_revert_message = second_revert.to_string();
        assert!(
            second_revert_message.contains("-32001")
                || second_revert_message.contains("snapshot not found"),
            "expected snapshot-not-found error, got: {second_revert_message}"
        );
    }

    fn sign_typed(signing_key: &SigningKey, tx_type: u8, to: Option<Address>) -> Bytes {
        let unsigned = match tx_type {
            0x01 => RlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(3),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || RlpValue::Bytes(Vec::new()),
                    |value| RlpValue::Bytes(value.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                RlpValue::Bytes(vec![0x01, 0x02]),
                RlpValue::List(Vec::new()),
            ]),
            0x02 => RlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(1),
                rlp_from_u64(4),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || RlpValue::Bytes(Vec::new()),
                    |value| RlpValue::Bytes(value.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                RlpValue::Bytes(vec![0x01, 0x02]),
                RlpValue::List(Vec::new()),
            ]),
            0x03 => RlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(1),
                rlp_from_u64(4),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || RlpValue::Bytes(Vec::new()),
                    |value| RlpValue::Bytes(value.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                RlpValue::Bytes(vec![0x01, 0x02]),
                RlpValue::List(Vec::new()),
                rlp_from_u64(5),
                RlpValue::List(Vec::new()),
            ]),
            _ => panic!("unsupported tx type"),
        };
        let unsigned_rlp = rlp_encode(&unsigned);
        let mut payload = vec![tx_type];
        payload.extend_from_slice(&unsigned_rlp);
        let hash = keccak256(payload);
        let mut field_bytes = FieldBytes::default();
        field_bytes.copy_from_slice(hash.as_slice());
        let (signature, recovery_id) = signing_key
            .as_nonzero_scalar()
            .try_sign_prehashed_rfc6979::<sha2::Sha256>(&field_bytes, &[])
            .unwrap_or_else(|error| panic!("sign typed prehash: {error}"));
        let recovery_id = recovery_id.unwrap_or_else(|| panic!("typed recovery id present"));

        let mut fields = match unsigned {
            RlpValue::List(fields) => fields,
            _ => unreachable!(),
        };
        fields.push(rlp_from_u64(u64::from(recovery_id.to_byte())));
        fields.push(RlpValue::Bytes(signature.r().to_bytes().to_vec()));
        fields.push(RlpValue::Bytes(signature.s().to_bytes().to_vec()));
        let mut encoded = vec![tx_type];
        encoded.extend_from_slice(&rlp_encode(&RlpValue::List(fields)));
        Bytes::from(encoded)
    }

    #[test]
    fn sqrt_price_x96_round_trip() {
        // Verify that to_sqrt_price_x96 and from_sqrt_price_x96 round-trip within
        // the tolerance used by the integration test (INV-004, price = 1800.0).
        let price = 1800.0_f64;
        let encoded = to_sqrt_price_x96(price);
        assert!(
            !encoded.is_zero(),
            "encoded sqrtPriceX96 must be non-zero for price={price}"
        );

        let decoded = from_sqrt_price_x96(encoded);
        let rel_error = (decoded - price).abs() / price;
        assert!(
            rel_error < 1e-9,
            "round-trip relative error too large: decoded={decoded} price={price} rel_error={rel_error}"
        );
    }

    #[test]
    fn sqrt_price_x96_edge_cases() {
        // Zero input produces zero output.
        assert_eq!(to_sqrt_price_x96(0.0), U256::ZERO);
        assert_eq!(to_sqrt_price_x96(-1.0), U256::ZERO);
        assert_eq!(to_sqrt_price_x96(f64::NAN), U256::ZERO);
        assert_eq!(to_sqrt_price_x96(f64::INFINITY), U256::ZERO);
        assert_eq!(to_sqrt_price_x96(f64::NEG_INFINITY), U256::ZERO);

        // Zero sqrtPriceX96 decodes as 0.0.
        assert_eq!(from_sqrt_price_x96(U256::ZERO), 0.0);
    }

    #[tokio::test]
    async fn test_jsonrpc_unknown_method_returns_32601() {
        use jsonrpsee::core::server::MethodsError;

        let (module, _context) = test_rpc_module();
        assert!(
            module.method("eth_fooNonExistent").is_none(),
            "eth_fooNonExistent should not be registered"
        );
        let error = module
            .call::<_, serde_json::Value>("eth_fooNonExistent", Vec::<()>::new())
            .await
            .expect_err("unknown method should return an error");
        match error {
            MethodsError::JsonRpc(obj) => {
                assert_eq!(
                    obj.code(),
                    -32601,
                    "wire code must be JSON-RPC method not found"
                );
            }
            other => panic!("expected JSON-RPC error object, got {other:?}"),
        }
    }

    #[cfg(feature = "chain")]
    fn test_rpc_module_with_chain() -> (jsonrpsee::RpcModule<ServerContext>, ServerContext) {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(1);
        let chain = Arc::new(parking_lot::RwLock::new(
            crate::chain_rpc::ChainContext::new(crate::chain_rpc::ChainToggles::default()),
        ));
        let context = ServerContext {
            state: mirage.state(),
            shutdown,
            chain: Some(chain),
            #[cfg(feature = "roko")]
            chain_subs: None,
        };
        let module = build_rpc_module(context.clone())
            .unwrap_or_else(|error| panic!("build rpc module with chain: {error}"));
        (module, context)
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn chain_register_agent_rpc() {
        let (module, _context) = test_rpc_module_with_chain();
        let result: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "researcher"))
            .await
            .expect("register agent");
        assert!(result, "first registration should succeed");

        let duplicate: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "researcher"))
            .await
            .expect("duplicate register");
        assert!(!duplicate, "duplicate registration should return false");
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn chain_agent_heartbeat_rpc() {
        let (module, _context) = test_rpc_module_with_chain();
        let _: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "worker"))
            .await
            .unwrap();

        let result: bool = module
            .call("chain_agentHeartbeat", ("agent-1",))
            .await
            .expect("heartbeat");
        assert!(result);

        let missing: bool = module
            .call("chain_agentHeartbeat", ("nonexistent",))
            .await
            .expect("heartbeat nonexistent");
        assert!(!missing);
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn chain_agent_trace_rpc() {
        let (module, _context) = test_rpc_module_with_chain();
        let _: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "coder"))
            .await
            .unwrap();

        let result: bool = module
            .call(
                "chain_agentTrace",
                (
                    "agent-1",
                    "reason",
                    vec!["file.rs"],
                    "thinking about it",
                    "edit file",
                ),
            )
            .await
            .expect("trace");
        assert!(result);

        let missing: bool = module
            .call(
                "chain_agentTrace",
                (
                    "nonexistent",
                    "act",
                    Vec::<String>::new(),
                    "doing something",
                    "run cmd",
                ),
            )
            .await
            .expect("trace for missing agent");
        assert!(!missing);
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn chain_agent_stats_rpc() {
        let (module, _context) = test_rpc_module_with_chain();
        let _: bool = module
            .call("chain_registerAgent", ("agent-1", "0xdead", "analyst"))
            .await
            .unwrap();

        let delta = crate::chain::AgentStats {
            confirmations_given: 5,
            challenges_given: 2,
            warnings_posted: 1,
            insights_posted: 3,
            tasks_completed: 0,
            tasks_failed: 0,
            delta_cycles: 10,
            total_cost_usd: 0.5,
            total_tokens: 1000,
        };
        let result: bool = module
            .call("chain_agentStats", ("agent-1", delta))
            .await
            .expect("stats update");
        assert!(result);

        // Verify stats were accumulated in the chain context.
        let chain = _context.chain.as_ref().expect("chain present");
        let guard = chain.read();
        let stats = guard
            .agent_registry
            .get_stats("agent-1")
            .expect("agent should exist");
        assert_eq!(stats.confirmations_given, 5);
        assert_eq!(stats.total_tokens, 1000);
    }

    #[cfg(feature = "chain")]
    #[tokio::test]
    async fn agent_http_endpoints_via_full_server() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let (shutdown, _) = broadcast::channel(4);
        let chain = Arc::new(parking_lot::RwLock::new(
            crate::chain_rpc::ChainContext::new(crate::chain_rpc::ChainToggles::default()),
        ));
        let (addr, _handle) =
            super::start_rpc_server_with_chain("127.0.0.1:0", mirage, shutdown, chain)
                .await
                .expect("start server");
        let url = format!("http://{addr}");
        let http = reqwest::Client::new();

        // Register an agent via JSON-RPC.
        let resp = http
            .post(&url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "method": "chain_registerAgent",
                "params": ["agent-1", "0xdead", "researcher"]
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["result"], true);

        // Heartbeat.
        let resp = http
            .post(&url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "id": 2,
                "method": "chain_agentHeartbeat",
                "params": ["agent-1"]
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["result"], true);

        // Add a trace.
        let resp = http
            .post(&url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "id": 3,
                "method": "chain_agentTrace",
                "params": ["agent-1", "retrieve", ["doc.md"], "reading docs", "read file"]
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["result"], true);

        // GET /api/agents — list agents.
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let agents = resp["items"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["id"], "agent-1");
        assert_eq!(agents[0]["role"], "researcher");

        // GET /api/agents/agent-1/trace
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents/agent-1/trace?limit=10&offset=0"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["total"], 1);
        assert_eq!(resp["items"].as_array().unwrap().len(), 1);

        // GET /api/agents/agent-1/heartbeat
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents/agent-1/heartbeat"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["agent_id"], "agent-1");
        assert!(resp.get("alive").is_some());

        // GET /api/agents/agent-1/stats
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents/agent-1/stats"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["agent_id"], "agent-1");
        assert_eq!(resp["registered_at"].as_u64().is_some(), true);

        // Non-existent agent returns error.
        let resp: serde_json::Value = http
            .get(format!("{url}/api/agents/nobody/stats"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["error"], "agent not found");
    }
}
