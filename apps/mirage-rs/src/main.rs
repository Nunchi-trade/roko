//! `mirage-rs` binary entrypoint.

#![allow(
    clippy::redundant_pub_crate,
    clippy::doc_markdown,
    clippy::too_many_lines
)]

use std::{fs, path::PathBuf, sync::Arc, time::Duration};

use alloy_primitives::{Address, Bytes, U256, address, hex};
use anyhow::{Context, bail};
use clap::Parser;
use mirage_rs::{
    ClassificationConfig, DiffClassifier, MirageError,
    events::MirageTelemetryEvent,
    fork::{EvmExecutor, ForkState, HybridDB, MirageFork},
    persist,
    provider::UpstreamRpc,
    replay::{FollowerConfig, TargetedFollower},
    resources::{MirageMode, Profile, ResourceModel},
    rpc::start_rpc_server,
};
use roko_runtime::event_bus::EventBus;
use tokio::sync::broadcast;

/// Default telemetry `EventBus` capacity.
///
/// `roko_runtime::event_bus::EventBus` uses a bounded broadcast + replay ring; producers never
/// block: the replay ring drops the oldest event when full, and live `broadcast::send` errors are
/// ignored when no subscribers are connected (mirage does not back-pressure JSON-RPC on overflow).
const TELEMETRY_BUS_CAPACITY: usize = 10_000;

/// Command-line configuration for `mirage-rs`.
#[derive(Debug, Clone, Parser)]
#[command(author, version, about)]
#[allow(clippy::struct_excessive_bools)]
struct Cli {
    /// Bind host (default `127.0.0.1`).
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Bind port (default `8545`).
    #[arg(long, default_value_t = 8545)]
    port: u16,
    /// Optional upstream HTTP RPC URL (`eth_blockNumber` probes connectivity before serve).
    #[arg(long)]
    rpc_url: Option<String>,
    /// Optional upstream WebSocket URL.
    #[arg(long)]
    ws_url: Option<String>,
    /// Upstream request budget per second (default `100`).
    #[arg(long, default_value_t = 100)]
    upstream_rps: u32,
    /// Upstream burst multiplier (default `200`).
    #[arg(long, default_value_t = 200)]
    upstream_burst: u32,
    /// Effective chain ID (default `1`).
    #[arg(long, default_value_t = 1)]
    chain_id: u64,
    /// Read-cache capacity.
    #[arg(long, default_value_t = 10_000)]
    cache_size: usize,
    /// Read-cache TTL in seconds.
    #[arg(long, default_value_t = 12)]
    cache_ttl_secs: u64,
    /// Resource profile.
    #[arg(long, value_enum, default_value_t = ProfileArg::Standard)]
    profile: ProfileArg,
    /// Fork mode: live lazy-read, historical pinned view, or proxy-only under pressure.
    #[arg(long, value_enum, default_value_t = ModeArg::Live)]
    mode: ModeArg,
    /// Inactivity watchdog timeout in seconds (optional).
    #[arg(long)]
    watchdog_timeout: Option<u64>,
    /// Enforce strict nonce checks (default off).
    #[arg(long, default_value_t = false)]
    strict_nonce: bool,
    /// Enforce strict balance checks (default off).
    #[arg(long, default_value_t = false)]
    strict_balance: bool,
    /// Enable signature verification (default off).
    #[arg(long, default_value_t = false)]
    verify_signatures: bool,
    /// Auto-mine a block every N milliseconds (e.g. 50 for 20 blocks/sec).
    #[arg(long)]
    block_interval_ms: Option<u64>,
    /// Enable the HDC index subsystem (required for chain_searchInsights / chain_postInsight).
    /// Only effective with --features chain. Enabled by default.
    #[cfg(feature = "chain")]
    #[arg(long, default_value_t = true)]
    enable_hdc: bool,
    /// Enable the knowledge layer (InsightEntry state machine, confirmations, challenges, decay).
    /// Only effective with --features chain. Enabled by default.
    #[cfg(feature = "chain")]
    #[arg(long, default_value_t = true)]
    enable_knowledge: bool,
    /// Enable stigmergy (THREAT/OPPORTUNITY/WISDOM pheromones with time decay).
    /// Only effective with --features chain. Enabled by default.
    #[cfg(feature = "chain")]
    #[arg(long, default_value_t = true)]
    enable_stigmergy: bool,
    /// Switchover threshold — above this entry count, the HDC index auto-upgrades to HNSW.
    /// Only effective with --features chain.
    #[cfg(feature = "chain")]
    #[arg(long, default_value_t = 100_000)]
    chain_hnsw_threshold: usize,
    /// Interval in seconds between periodic state snapshots (default 30).
    #[arg(long, default_value_t = 30)]
    snapshot_interval_secs: u64,
    /// Directory for persistent state snapshots (default `.roko/state/`).
    #[arg(long)]
    state_dir: Option<PathBuf>,
    /// Disable disk persistence entirely.
    #[arg(long, default_value_t = false)]
    no_persist: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ProfileArg {
    Micro,
    Standard,
    Power,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ModeArg {
    Live,
    Historical,
    Proxy,
}

#[derive(Debug, Clone)]
struct Erc8004Contracts {
    identity_registry: Address,
    reputation_registry: Address,
    validation_registry: Address,
    source: &'static str,
}

const ERC8004_IDENTITY_REGISTRY: Address = address!("0x8004A818BFB912233c491871b3d84c89A494BD9e");
const ERC8004_REPUTATION_REGISTRY: Address = address!("0x8004A818BFB912233c491871b3d84c89A494BD9f");
const ERC8004_VALIDATION_REGISTRY: Address = address!("0x8004A818BFB912233c491871b3d84c89A494BDA0");
const ERC8004_BOOTSTRAP_ADMIN: Address = address!("0x8004000000000000000000000000000000000001");
const ERC8004_BOOTSTRAP_DEPLOYER: Address = address!("0x8004000000000000000000000000000000000002");
const ERC8004_IDENTITY_INIT_HEX: &str = include_str!("../static/erc8004/IdentityRegistry.init.hex");
const ERC8004_REPUTATION_INIT_HEX: &str =
    include_str!("../static/erc8004/ReputationRegistry.init.hex");
const ERC8004_VALIDATION_INIT_HEX: &str =
    include_str!("../static/erc8004/ValidationRegistry.init.hex");

impl From<ModeArg> for MirageMode {
    fn from(value: ModeArg) -> Self {
        match value {
            ModeArg::Live => Self::Live,
            ModeArg::Historical => Self::Historical,
            ModeArg::Proxy => Self::Proxy,
        }
    }
}

impl From<ProfileArg> for Profile {
    fn from(value: ProfileArg) -> Self {
        match value {
            ProfileArg::Micro => Self::Micro,
            ProfileArg::Standard => Self::Standard,
            ProfileArg::Power => Self::Power,
        }
    }
}

fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let cli = Cli::parse();

    let resource_model = ResourceModel::for_profile(
        Profile::from(cli.profile),
        Duration::from_secs(cli.cache_ttl_secs),
    );
    if let Err(error) = resource_model
        .ensure_spawn_budget()
        .context("resource budget check failed")
    {
        let exit_code = startup_exit_code(&error).unwrap_or(1);
        tracing::error!(error = %format!("{error:#}"), exit_code, "mirage startup failed");
        std::process::exit(exit_code);
    }

    // reqwest::blocking::Client::new() panics when called from inside a Tokio
    // async context, so build the upstream (and its blocking HTTP client) here
    // in sync context before starting the runtime.
    let upstream = Arc::new(UpstreamRpc::new_with_limits(
        cli.rpc_url.clone(),
        cli.ws_url.clone(),
        cli.chain_id,
        cli.upstream_rps,
        cli.upstream_burst,
    ));

    // --- Persistence + fork init: all sync work runs BEFORE the Tokio runtime ---
    // bootstrap_erc8004_contracts() and resolve_initial_head() use blocking I/O
    // (reqwest::blocking::Client) that panics inside an async context. Keep them here.
    let state_dir = cli
        .state_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(".roko/state"));
    tracing::info!(
        state_dir = %state_dir.display(),
        volume_mount = %std::env::var("RAILWAY_VOLUME_MOUNT_PATH").unwrap_or_else(|_| "(not set)".into()),
        no_persist = cli.no_persist,
        snapshot_file_exists = state_dir.join("mirage-snapshot.json").exists(),
        "persistence config"
    );

    let loaded_snapshot = if cli.no_persist {
        None
    } else {
        match persist::load_snapshot(&state_dir) {
            Ok(Some(snap)) => {
                tracing::info!(
                    version = snap.version,
                    created_at = snap.created_at,
                    block = snap.fork.local_block_number,
                    "restored snapshot from {}",
                    state_dir.display(),
                );
                Some(snap)
            }
            Ok(None) => {
                tracing::info!("no prior snapshot found in {}", state_dir.display());
                None
            }
            Err(e) => {
                tracing::warn!("failed to load snapshot, starting fresh: {e}");
                None
            }
        }
    };

    let head = if let Some(ref snap) = loaded_snapshot {
        snap.fork.local_block_number
    } else {
        let require_probe = cli.rpc_url.is_some();
        match resolve_initial_head(&upstream, require_probe)
            .context("upstream RPC health check failed")
        {
            Ok(h) => h,
            Err(error) => {
                let exit_code = startup_exit_code(&error).unwrap_or(1);
                tracing::error!(error = %format!("{error:#}"), exit_code, "mirage startup failed");
                std::process::exit(exit_code);
            }
        }
    };

    let mut fork = ForkState::new(
        HybridDB::new(
            Arc::clone(&upstream),
            cli.cache_size,
            Duration::from_secs(cli.cache_ttl_secs),
            resource_model.bytecode_cache_capacity(),
            cli.chain_id,
        ),
        head,
        cli.chain_id,
    );
    fork.strict_nonce = cli.strict_nonce;
    fork.strict_balance = cli.strict_balance;
    fork.verify_signatures = cli.verify_signatures;

    if let Some(ref snap) = loaded_snapshot {
        persist::apply_fork_snapshot(&mut fork, snap.fork.clone());
        fork.chain_id = cli.chain_id;
    }

    fork.db
        .set_balance(ERC8004_BOOTSTRAP_DEPLOYER, U256::from(10_u128.pow(18)));

    let erc8004_contracts = match bootstrap_erc8004_contracts(&mut fork)
        .context("bootstrap ERC-8004 contracts")
    {
        Ok(c) => c,
        Err(error) => {
            let exit_code = startup_exit_code(&error).unwrap_or(1);
            tracing::error!(error = %format!("{error:#}"), exit_code, "mirage startup failed");
            std::process::exit(exit_code);
        }
    };

    #[allow(clippy::expect_used)]
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");

    if let Err(error) = rt.block_on(run(cli, upstream, fork, erc8004_contracts, loaded_snapshot, state_dir, head)) {
        let exit_code = startup_exit_code(&error).unwrap_or(1);
        tracing::error!(error = %format!("{error:#}"), exit_code, "mirage startup failed");
        std::process::exit(exit_code);
    }
}

#[allow(clippy::cognitive_complexity, clippy::too_many_arguments)]
async fn run(
    cli: Cli,
    upstream: Arc<UpstreamRpc>,
    fork: ForkState,
    erc8004_contracts: Erc8004Contracts,
    loaded_snapshot: Option<persist::MirageSnapshot>,
    state_dir: PathBuf,
    head: u64,
) -> anyhow::Result<()> {
    let follower_upstream = Arc::clone(&upstream);
    let resource_model = ResourceModel::for_profile(
        Profile::from(cli.profile),
        Duration::from_secs(cli.cache_ttl_secs),
    );

    let mode = MirageMode::from(cli.mode);
    let telemetry_bus = EventBus::<MirageTelemetryEvent>::new(TELEMETRY_BUS_CAPACITY);
    let mirage = MirageFork::with_telemetry(fork, resource_model, mode, telemetry_bus.sender());
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel(8);
    let bind = format!("{}:{}", cli.host, cli.port);

    #[cfg(feature = "chain")]
    let (addr, handle, chain_ctx_for_persist) = {
        let toggles = mirage_rs::chain_rpc::ChainToggles {
            hdc: cli.enable_hdc,
            knowledge: cli.enable_knowledge,
            stigmergy: cli.enable_stigmergy,
        };
        if toggles.any_enabled() {
            let chain_ctx = {
                // Restore from snapshot if available, otherwise create fresh.
                #[allow(unused_mut)]
                let mut ctx = if let Some(chain_snap) =
                    loaded_snapshot.as_ref().and_then(|s| s.chain.clone())
                {
                    tracing::info!("restoring chain context from snapshot");
                    persist::chain_context_from_snapshot(chain_snap, cli.chain_hnsw_threshold)
                } else {
                    mirage_rs::chain_rpc::ChainContext::with_hnsw(toggles, cli.chain_hnsw_threshold)
                };
                // Install subscription buses so WebSocket streaming (/api/ws) and
                // JSON-RPC chain_subscribe* methods are available.
                #[cfg(feature = "roko")]
                {
                    ctx.set_buses(
                        std::sync::Arc::new(mirage_rs::roko_bridge::PheromoneBus::new()),
                        std::sync::Arc::new(mirage_rs::roko_bridge::InsightBus::new()),
                    );
                }
                // Seed ERC-8004 bootstrap addresses into the contract registry
                // so GET /api/deployment always includes them.
                ctx.contract_registry.insert(
                    "IdentityRegistry".into(),
                    format!("{:#x}", erc8004_contracts.identity_registry),
                );
                ctx.contract_registry.insert(
                    "ReputationRegistry".into(),
                    format!("{:#x}", erc8004_contracts.reputation_registry),
                );
                ctx.contract_registry.insert(
                    "ValidationRegistry".into(),
                    format!("{:#x}", erc8004_contracts.validation_registry),
                );
                std::sync::Arc::new(parking_lot::RwLock::new(ctx))
            };
            tracing::info!(
                hdc = toggles.hdc,
                knowledge = toggles.knowledge,
                stigmergy = toggles.stigmergy,
                hnsw_threshold = cli.chain_hnsw_threshold,
                "chain extensions enabled"
            );
            let persist_ctx = std::sync::Arc::clone(&chain_ctx);
            let (addr, handle) = mirage_rs::rpc::start_rpc_server_with_chain(
                &bind,
                mirage.clone(),
                shutdown_tx.clone(),
                chain_ctx,
            )
            .await
            .with_context(|| format!("failed to bind {bind}"))?;
            (addr, handle, Some(persist_ctx))
        } else {
            let (addr, handle) = start_rpc_server(&bind, mirage.clone(), shutdown_tx.clone())
                .await
                .with_context(|| format!("failed to bind {bind}"))?;
            (addr, handle, None)
        }
    };

    #[cfg(not(feature = "chain"))]
    let (addr, handle) = start_rpc_server(&bind, mirage.clone(), shutdown_tx.clone())
        .await
        .with_context(|| format!("failed to bind {bind}"))?;

    if cli.rpc_url.is_some() || cli.ws_url.is_some() {
        // When the auto-miner is also running, the follower should only apply
        // upstream state diffs — not overwrite the local block number.
        let sync_state_only = cli.block_interval_ms.is_some();
        if sync_state_only {
            tracing::info!(
                "auto-miner + upstream fork: follower will sync state without advancing block number"
            );
        }
        let follower = TargetedFollower::new(
            follower_upstream,
            &mirage,
            DiffClassifier::new(ClassificationConfig::default()),
            FollowerConfig {
                ws_url: cli.ws_url.clone().unwrap_or_default(),
                http_url: cli.rpc_url.clone().unwrap_or_default(),
                block_budget: Duration::from_secs(10),
                filter_addresses: None,
                filter_selectors: None,
                sync_state_only,
            },
            head,
        );
        let shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move {
            if let Err(error) = follower.run(shutdown).await {
                tracing::warn!("targeted follower exited with error: {error}");
            }
        });
    }

    write_artifacts(addr.port(), cli.chain_id, &erc8004_contracts)
        .context("failed to write startup artifacts")?;
    tracing::info!("mirage ready port={} chain={}", addr.port(), cli.chain_id);
    tracing::info!(
        source = erc8004_contracts.source,
        identity_registry = %erc8004_contracts.identity_registry,
        reputation_registry = %erc8004_contracts.reputation_registry,
        validation_registry = %erc8004_contracts.validation_registry,
        "ERC-8004 identity surface ready; Rust AgentRegistry remains runtime-only metadata"
    );

    // --- Spawn periodic persistence loop ---
    if !cli.no_persist {
        let interval = Duration::from_secs(cli.snapshot_interval_secs);
        tracing::info!(
            interval_secs = cli.snapshot_interval_secs,
            dir = %state_dir.display(),
            "persistence loop started"
        );
        #[cfg(feature = "chain")]
        persist::spawn_persistence_loop(
            mirage.clone(),
            chain_ctx_for_persist.clone(),
            state_dir.clone(),
            interval,
            shutdown_tx.subscribe(),
        );
        #[cfg(not(feature = "chain"))]
        persist::spawn_persistence_loop(
            mirage.clone(),
            state_dir.clone(),
            interval,
            shutdown_tx.subscribe(),
        );
    }

    if let Some(timeout_secs) = cli.watchdog_timeout {
        let mirage = mirage.clone();
        let shutdown = shutdown_tx.clone();
        tokio::spawn(async move {
            let timeout = Duration::from_secs(timeout_secs);
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let idle = mirage.idle_for();
                if idle >= timeout {
                    tracing::warn!(
                        idle_secs = idle.as_secs(),
                        timeout_secs,
                        "watchdog idle timeout reached — initiating shutdown"
                    );
                    let _ = shutdown.send(());
                    break;
                }
            }
        });
    }

    tracing::info!("mirage event loop ready");

    if let Some(interval_ms) = cli.block_interval_ms {
        let miner = mirage.clone();
        tokio::spawn(async move {
            let interval = Duration::from_millis(interval_ms);
            tracing::info!(interval_ms, "auto-miner started");
            loop {
                tokio::time::sleep(interval).await;
                miner.mine_block().await;
            }
        });
    }

    // Hold shutdown_tx alive through the select so the broadcast channel
    // stays open until an explicit signal or Ctrl+C.  Without this, Rust may
    // drop shutdown_tx early (last syntactic use is subscribe() above), which
    // closes the channel and causes recv() to return Err(Closed) immediately.
    #[cfg(unix)]
    {
        let shutdown = shutdown_tx.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(stream) => stream,
                Err(error) => {
                    tracing::warn!(%error, "failed to register SIGTERM handler");
                    return;
                }
            };
            sigterm.recv().await;
            tracing::info!("mirage SIGTERM received");
            let _ = shutdown.send(());
        });
    }

    let shutdown_guard = shutdown_tx;
    tokio::select! {
        result = shutdown_rx.recv() => {
            match result {
                Ok(()) => tracing::warn!("mirage shutdown signal received — exiting"),
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::error!("mirage shutdown channel closed unexpectedly (all senders dropped) — exiting");
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("mirage shutdown receiver lagged by {n} messages — exiting");
                }
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("mirage ctrl+c received");
        }
    }
    drop(shutdown_guard);

    // --- Final snapshot on shutdown ---
    if !cli.no_persist {
        tracing::info!("writing final snapshot before exit");
        #[cfg(feature = "chain")]
        {
            let snap = chain_ctx_for_persist.as_ref().map_or_else(
                || persist::capture_snapshot(&mirage, None),
                |ctx| {
                    let chain = ctx.read();
                    persist::capture_snapshot(&mirage, Some(&*chain))
                },
            );
            if let Err(e) = persist::write_snapshot(&snap, &state_dir) {
                tracing::warn!("final snapshot failed: {e}");
            }
        }
        #[cfg(not(feature = "chain"))]
        {
            let snap = persist::capture_snapshot(&mirage);
            if let Err(e) = persist::write_snapshot(&snap, &state_dir) {
                tracing::warn!("final snapshot failed: {e}");
            }
        }
    }

    cleanup_artifacts(addr.port());
    handle.stop().context("failed to stop RPC server")?;
    Ok(())
}

fn startup_exit_code(error: &anyhow::Error) -> Option<i32> {
    error.chain().find_map(|cause| {
        cause
            .downcast_ref::<MirageError>()
            .and_then(|mirage_error| match mirage_error {
                MirageError::Unsupported(message)
                    if message.starts_with("insufficient memory:") =>
                {
                    Some(2)
                }
                _ => None,
            })
    })
}

fn write_artifacts(port: u16, chain_id: u64, erc8004: &Erc8004Contracts) -> anyhow::Result<()> {
    let pid_path = PathBuf::from(format!("/tmp/mirage-{port}.pid"));
    let status_path = PathBuf::from(format!("/tmp/mirage-{port}-status.json"));
    fs::write(pid_path, std::process::id().to_string())?;
    fs::write(
        status_path,
        serde_json::json!({
            "status": "ready",
            "ready": true,
            "port": port,
            "chainId": chain_id,
            "erc8004": {
                "source": erc8004.source,
                "identityRegistry": format!("{}", erc8004.identity_registry),
                "reputationRegistry": format!("{}", erc8004.reputation_registry),
                "validationRegistry": format!("{}", erc8004.validation_registry),
            },
        })
        .to_string(),
    )?;
    Ok(())
}

fn cleanup_artifacts(port: u16) {
    let pid_path = PathBuf::from(format!("/tmp/mirage-{port}.pid"));
    let status_path = PathBuf::from(format!("/tmp/mirage-{port}-status.json"));
    let _ = fs::remove_file(pid_path);
    let _ = fs::remove_file(status_path);
}

fn bootstrap_erc8004_contracts(fork: &mut ForkState) -> anyhow::Result<Erc8004Contracts> {
    let bootstrapped_identity = ensure_erc8004_contract(
        fork,
        ERC8004_IDENTITY_REGISTRY,
        ERC8004_IDENTITY_INIT_HEX,
        &[ERC8004_BOOTSTRAP_ADMIN, Address::ZERO],
    )
    .context("bootstrap identity registry")?;
    let bootstrapped_reputation = ensure_erc8004_contract(
        fork,
        ERC8004_REPUTATION_REGISTRY,
        ERC8004_REPUTATION_INIT_HEX,
        &[ERC8004_IDENTITY_REGISTRY, ERC8004_BOOTSTRAP_ADMIN],
    )
    .context("bootstrap reputation registry")?;
    let bootstrapped_validation = ensure_erc8004_contract(
        fork,
        ERC8004_VALIDATION_REGISTRY,
        ERC8004_VALIDATION_INIT_HEX,
        &[ERC8004_IDENTITY_REGISTRY, ERC8004_BOOTSTRAP_ADMIN],
    )
    .context("bootstrap validation registry")?;
    let source = if bootstrapped_identity || bootstrapped_reputation || bootstrapped_validation {
        "bootstrapped"
    } else {
        "upstream"
    };

    Ok(Erc8004Contracts {
        identity_registry: ERC8004_IDENTITY_REGISTRY,
        reputation_registry: ERC8004_REPUTATION_REGISTRY,
        validation_registry: ERC8004_VALIDATION_REGISTRY,
        source,
    })
}

fn ensure_erc8004_contract(
    fork: &mut ForkState,
    canonical_address: Address,
    init_code_hex: &str,
    constructor_args: &[Address],
) -> anyhow::Result<bool> {
    if contract_has_code(fork, canonical_address)? {
        return Ok(false);
    }

    let init_code = build_init_code(init_code_hex, constructor_args)?;
    let mut bootstrap_state = fork.clone();
    let (result, diff) = EvmExecutor::transact(
        &mut bootstrap_state,
        ERC8004_BOOTSTRAP_DEPLOYER,
        None,
        init_code,
        U256::ZERO,
        30_000_000,
    )
    .with_context(|| format!("deploy bootstrap contract for {canonical_address}"))?;
    if !result.success {
        bail!("bootstrap deployment reverted for {canonical_address}");
    }

    let deployed_address = decode_created_address(diff.output.as_ref())?;
    let deployed_account = bootstrap_state
        .db
        .dirty
        .accounts
        .get(&deployed_address)
        .cloned()
        .with_context(|| {
            format!("bootstrap account missing for deployed contract {deployed_address}")
        })?;
    let runtime_code = deployed_account.code.clone().with_context(|| {
        format!("bootstrap runtime code missing for deployed contract {deployed_address}")
    })?;

    fork.db.set_code(canonical_address, runtime_code);
    if let Some(balance) = deployed_account.balance {
        fork.db.set_balance(canonical_address, balance);
    }
    if let Some(nonce) = deployed_account.nonce {
        fork.db.set_nonce(canonical_address, nonce);
    }
    for (slot, value) in deployed_account.storage {
        fork.db.set_storage(canonical_address, slot, value);
    }

    Ok(true)
}

fn contract_has_code(fork: &mut ForkState, address: Address) -> anyhow::Result<bool> {
    let info = fork.db.basic(address)?.unwrap_or_default();
    let code = fork.db.code_by_hash(info.code_hash)?;
    Ok(!code.bytecode().is_empty())
}

fn build_init_code(init_code_hex: &str, constructor_args: &[Address]) -> anyhow::Result<Bytes> {
    let hex_text = init_code_hex.trim();
    let decoded = hex::decode(hex_text.trim_start_matches("0x"))
        .with_context(|| "decode ERC-8004 init bytecode")?;
    let mut init_code = decoded;
    for address in constructor_args {
        init_code.extend_from_slice(&encode_address_word(*address));
    }
    Ok(Bytes::from(init_code))
}

fn encode_address_word(address: Address) -> [u8; 32] {
    let mut encoded = [0_u8; 32];
    encoded[12..].copy_from_slice(address.as_slice());
    encoded
}

fn decode_created_address(bytes: &[u8]) -> anyhow::Result<Address> {
    match bytes.len() {
        20 => Ok(Address::from_slice(bytes)),
        len if len > 20 => Ok(Address::from_slice(&bytes[len - 20..])),
        len => bail!("unexpected create output length: {len}"),
    }
}

fn resolve_initial_head(upstream: &UpstreamRpc, require_probe: bool) -> anyhow::Result<u64> {
    let health = upstream.health_check();
    if require_probe {
        return health.context("upstream RPC health check failed");
    }
    Ok(health.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{
        Cli, Erc8004Contracts, ModeArg, UpstreamRpc, cleanup_artifacts, decode_created_address,
        encode_address_word, resolve_initial_head, startup_exit_code, write_artifacts,
    };
    use crate::MirageError;

    #[test]
    fn initial_head_requires_probe_when_http_upstream_is_configured() {
        let upstream =
            UpstreamRpc::new_with_limits(Some("http://127.0.0.1:9".to_owned()), None, 1, 1, 1);

        let error = resolve_initial_head(&upstream, true).expect_err("probe should fail");
        let message = error.to_string();
        assert!(message.contains("upstream RPC health check failed"));
    }

    #[test]
    fn low_memory_startup_error_exits_with_code_two() {
        let error = anyhow::Error::new(MirageError::Unsupported(
            "insufficient memory: available=1 required=2".to_owned(),
        ))
        .context("resource budget check failed");

        assert_eq!(startup_exit_code(&error), Some(2));
    }

    #[test]
    fn unrelated_startup_errors_use_default_exit_code() {
        let error = anyhow::Error::new(MirageError::BindFailed(8545)).context("failed to bind");

        assert_eq!(startup_exit_code(&error), None);
    }

    #[test]
    fn cli_defaults_bind_to_local_8545() {
        let cli = <Cli as clap::Parser>::parse_from(["mirage-rs"]);

        assert_eq!(cli.host, "127.0.0.1");
        assert_eq!(cli.port, 8545);
        assert_eq!(format!("{}:{}", cli.host, cli.port), "127.0.0.1:8545");
    }

    #[test]
    fn cli_defaults_mode_live_upstream_limits_and_verify_signatures() {
        let cli = <Cli as clap::Parser>::parse_from(["mirage-rs"]);

        assert!(matches!(cli.mode, ModeArg::Live));
        assert_eq!(cli.upstream_rps, 100);
        assert_eq!(cli.upstream_burst, 200);
        assert_eq!(cli.chain_id, 1);
        assert!(!cli.verify_signatures);
        assert!(cli.watchdog_timeout.is_none());
    }

    #[test]
    fn test_startup_status_artifact_json() {
        // Avoid fixed 18545: other harnesses or parallel `cargo test` runs may reuse `/tmp/mirage-*.pid`.
        let port = 19_000_u16.wrapping_add((std::process::id() % 6_000) as u16);
        let pid_path = PathBuf::from(format!("/tmp/mirage-{port}.pid"));
        let status_path = PathBuf::from(format!("/tmp/mirage-{port}-status.json"));

        let _ = fs::remove_file(&pid_path);
        let _ = fs::remove_file(&status_path);

        let chain_id = 42_u64;
        let erc8004 = Erc8004Contracts {
            identity_registry: super::ERC8004_IDENTITY_REGISTRY,
            reputation_registry: super::ERC8004_REPUTATION_REGISTRY,
            validation_registry: super::ERC8004_VALIDATION_REGISTRY,
            source: "bootstrapped",
        };
        write_artifacts(port, chain_id, &erc8004).expect("artifacts should be written");

        let pid_text = fs::read_to_string(&pid_path).expect("pid artifact should exist");
        assert_eq!(pid_text, std::process::id().to_string());
        assert!(
            !pid_text.contains('\n'),
            "pid file should be a single line with no embedded newlines"
        );

        let status = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(&status_path).expect("status artifact should exist"),
        )
        .expect("status artifact should be valid json");
        assert_eq!(status["status"], "ready");
        assert_eq!(status["ready"], true);
        assert_eq!(status["port"], serde_json::json!(port));
        assert_eq!(status["chainId"], serde_json::json!(chain_id));
        assert_eq!(status["erc8004"]["source"], "bootstrapped");
        assert_eq!(
            status["erc8004"]["identityRegistry"],
            serde_json::json!(format!("{}", super::ERC8004_IDENTITY_REGISTRY))
        );

        cleanup_artifacts(port);

        assert!(!pid_path.exists());
        assert!(!status_path.exists());
    }

    #[test]
    fn address_word_encoding_is_left_padded() {
        let encoded = encode_address_word(super::ERC8004_IDENTITY_REGISTRY);
        assert!(encoded[..12].iter().all(|byte| *byte == 0));
        assert_eq!(&encoded[12..], super::ERC8004_IDENTITY_REGISTRY.as_slice());
    }

    #[test]
    fn create_output_decodes_last_twenty_bytes() {
        let mut output = vec![0_u8; 32];
        output[12..].copy_from_slice(super::ERC8004_REPUTATION_REGISTRY.as_slice());
        let decoded = decode_created_address(&output).expect("decode address");
        assert_eq!(decoded, super::ERC8004_REPUTATION_REGISTRY);
    }
}
