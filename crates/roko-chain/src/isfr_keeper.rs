//! ISFR Keeper — orchestrates rate fetching, aggregation, and publication.
//!
//! Lifecycle:
//! 1. Constructed with sources + config via `new()`, `mock_keeper()`, or `from_config()`.
//! 2. Optionally attach a relay publish callback via `set_publish_fn()`.
//! 3. Call `run(cancel)` to enter the poll loop; it blocks until the token fires.
//! 4. Each tick: poll all sources (with per-source timeout) → compute composite →
//!    store as current rate → invoke publish callback if set.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::isfr_sources::{
    CompositeRate, ISFRSource, RateClass, SourceMeta, SourceReading, SourceStatus,
    compute_composite, mock::MockSource, mock::OfflineSource,
};

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for the ISFR keeper (corresponds to `[isfr]` in roko.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ISFRKeeperConfig {
    /// How often to poll sources (seconds). Default: 10.
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// Epoch duration (seconds). Used for range coordination. Default: 28800 (8h).
    #[serde(default = "default_epoch_duration")]
    pub epoch_duration_secs: u64,
    /// Minimum number of live source readings needed to publish. Default: 2.
    #[serde(default = "default_min_submissions")]
    pub min_submissions: u32,
    /// Sigma threshold for outlier rejection. Default: 3.0.
    #[serde(default = "default_outlier_sigma")]
    pub outlier_sigma: f64,
    /// Relay WebSocket URL (optional; if None, publish callback must be set).
    pub relay_url: Option<String>,
    /// Chain ID for relay topic naming. Default: "31337".
    #[serde(default = "default_chain_id")]
    pub chain_id: String,
}

fn default_poll_interval() -> u64 {
    10
}
fn default_epoch_duration() -> u64 {
    28_800
}
fn default_min_submissions() -> u32 {
    2
}
fn default_outlier_sigma() -> f64 {
    3.0
}
fn default_chain_id() -> String {
    "31337".to_string()
}

impl Default for ISFRKeeperConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_poll_interval(),
            epoch_duration_secs: default_epoch_duration(),
            min_submissions: default_min_submissions(),
            outlier_sigma: default_outlier_sigma(),
            relay_url: None,
            chain_id: default_chain_id(),
        }
    }
}

// ─── SourceConfig ─────────────────────────────────────────────────────────────

/// Source entry from `[[isfr.sources]]` in roko.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    /// Unique source identifier (e.g. "aave-v3-usdc").
    pub name: String,
    /// Source kind: "mock", "aave_v3", "compound_v3", "ethena", "staking".
    pub kind: String,
    /// Composite weight (0.0–1.0).
    pub weight: f64,
    /// Rate class: "lending", "structured", "funding", "staking".
    pub class: String,
    /// Base rate in bps (mock sources only).
    #[serde(default)]
    pub rate_bps: u64,
    /// Jitter range in bps (mock sources only).
    #[serde(default)]
    pub jitter_bps: u64,
    /// JSON-RPC endpoint (live sources only).
    #[serde(default)]
    pub rpc_url: Option<String>,
    /// Protocol pool/contract address (live sources only).
    #[serde(default)]
    pub pool_address: Option<String>,
}

// ─── ISFRKeeper ───────────────────────────────────────────────────────────────

/// Relay publish callback signature: `fn(topic, msg_type, payload)`.
pub type PublishFn = Arc<dyn Fn(&str, &str, serde_json::Value) + Send + Sync>;

/// The ISFR Keeper orchestrator.
pub struct ISFRKeeper {
    /// Keeper identifier (used in logs and relay topic).
    pub keeper_id: String,
    /// Rate sources (polled each tick).
    sources: Vec<Box<dyn ISFRSource>>,
    /// Most recently computed composite rate.
    current_rate: RwLock<Option<CompositeRate>>,
    /// Per-source metadata for status reporting.
    source_metas: RwLock<Vec<SourceMeta>>,
    /// Keeper configuration.
    config: ISFRKeeperConfig,
    /// Optional relay publish callback.
    publish_fn: RwLock<Option<PublishFn>>,
    /// Start time for epoch calculation.
    start_time: Instant,
    /// Current epoch number (updated each tick).
    current_epoch: AtomicU64,
}

impl ISFRKeeper {
    /// Create a keeper with the given sources and config.
    pub fn new(
        keeper_id: impl Into<String>,
        sources: Vec<Box<dyn ISFRSource>>,
        config: ISFRKeeperConfig,
    ) -> Self {
        let metas = sources
            .iter()
            .map(|s| SourceMeta {
                name: s.name().to_string(),
                class: s.rate_class(),
                weight: s.weight(),
                liveness_timeout_ms: s.liveness_timeout_ms(),
                last_reading: None,
                status: SourceStatus::Live,
                consecutive_failures: 0,
            })
            .collect();

        Self {
            keeper_id: keeper_id.into(),
            sources,
            current_rate: RwLock::new(None),
            source_metas: RwLock::new(metas),
            config,
            publish_fn: RwLock::new(None),
            start_time: Instant::now(),
            current_epoch: AtomicU64::new(0),
        }
    }

    /// Create a mock keeper with 4 standard dev sources.
    pub fn mock_keeper(keeper_id: &str, config: ISFRKeeperConfig) -> Self {
        let sources: Vec<Box<dyn ISFRSource>> = vec![
            Box::new(MockSource::aave_mock()),
            Box::new(MockSource::compound_mock()),
            Box::new(MockSource::ethena_mock()),
            Box::new(MockSource::staking_mock()),
        ];
        Self::new(keeper_id, sources, config)
    }

    /// Create a keeper from TOML source configurations.
    ///
    /// Unknown `kind` values fall back to mock with a warning — this lets
    /// config for future live sources be written before the backend exists.
    /// If an RPC endpoint is unreachable, alloy sources for that endpoint
    /// are replaced with mocks so the keeper always produces data.
    #[allow(clippy::too_many_lines)]
    pub fn from_config(
        keeper_id: &str,
        config: ISFRKeeperConfig,
        source_configs: &[SourceConfig],
    ) -> Self {
        // Shared provider cache: one alloy provider per rpc_url (avoids redundant connections).
        #[cfg(feature = "alloy-backend")]
        let mut provider_cache: std::collections::HashMap<
            String,
            Arc<alloy::providers::DynProvider>,
        > = std::collections::HashMap::new();

        // Pre-check which RPC URLs are reachable (quick TCP connect test).
        // Sources pointing to unreachable RPCs will try ETH_RPC_URL as
        // fallback before going offline — this lets the keeper fetch live
        // mainnet data when mirage is down.
        #[cfg(feature = "alloy-backend")]
        let eth_rpc_fallback: Option<String> = std::env::var("ETH_RPC_URL").ok().filter(|v| !v.trim().is_empty());
        #[cfg(feature = "alloy-backend")]
        let (reachable_rpcs, rpc_rewrites): (std::collections::HashSet<String>, std::collections::HashMap<String, String>) = {
            let mut set = std::collections::HashSet::new();
            let mut rewrites = std::collections::HashMap::new();
            let mut checked = std::collections::HashSet::new();

            // Also probe the fallback URL if present.
            let mut fallback_reachable = false;
            if let Some(ref fb) = eth_rpc_fallback {
                if let Ok(parsed) = reqwest::Url::parse(fb) {
                    let host = parsed.host_str().unwrap_or("127.0.0.1").to_string();
                    let port = parsed.port().unwrap_or(443);
                    if let Ok(addr) = format!("{host}:{port}").parse::<std::net::SocketAddr>() {
                        if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).is_ok() {
                            info!(rpc = %fb, "ETH_RPC_URL fallback reachable");
                            set.insert(fb.clone());
                            fallback_reachable = true;
                        } else {
                            warn!(rpc = %fb, "ETH_RPC_URL fallback unreachable");
                        }
                    }
                }
            }

            for sc in source_configs {
                let url = sc.rpc_url.as_deref().unwrap_or("http://127.0.0.1:8545");
                if !checked.insert(url.to_string()) {
                    continue;
                }
                if set.contains(url) {
                    continue;
                }
                if let Ok(parsed) = reqwest::Url::parse(url) {
                    let host = parsed.host_str().unwrap_or("127.0.0.1");
                    let port = parsed.port().unwrap_or(8545);
                    match std::net::TcpStream::connect_timeout(
                        &format!("{host}:{port}")
                            .parse()
                            .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], port))),
                        std::time::Duration::from_millis(500),
                    ) {
                        Ok(_) => {
                            info!(rpc = %url, "RPC endpoint reachable");
                            set.insert(url.to_string());
                        }
                        Err(_) => {
                            if fallback_reachable {
                                let fb = eth_rpc_fallback.as_deref().unwrap();
                                info!(rpc = %url, fallback = %fb, "RPC unreachable, falling back to ETH_RPC_URL");
                                rewrites.insert(url.to_string(), fb.to_string());
                            } else {
                                warn!(rpc = %url, "RPC endpoint unreachable, sources will go offline");
                            }
                        }
                    }
                }
            }
            (set, rewrites)
        };

        let sources: Vec<Box<dyn ISFRSource>> = source_configs
            .iter()
            .map(|sc| -> Box<dyn ISFRSource> {
                let class = match sc.class.as_str() {
                    "lending" => RateClass::Lending,
                    "structured" => RateClass::Structured,
                    "funding" => RateClass::Funding,
                    "staking" => RateClass::Staking,
                    _ => RateClass::Lending,
                };

                // Apply RPC rewrite if the configured URL was unreachable
                // but ETH_RPC_URL fallback is available.
                #[cfg(feature = "alloy-backend")]
                let effective_sc: std::borrow::Cow<'_, SourceConfig> = {
                    let rpc = sc.rpc_url.as_deref().unwrap_or("http://127.0.0.1:8545");
                    if let Some(rewritten) = rpc_rewrites.get(rpc) {
                        let mut patched = sc.clone();
                        patched.rpc_url = Some(rewritten.clone());
                        std::borrow::Cow::Owned(patched)
                    } else {
                        std::borrow::Cow::Borrowed(sc)
                    }
                };
                #[cfg(feature = "alloy-backend")]
                let sc = effective_sc.as_ref();

                match sc.kind.as_str() {
                    "mock" => Box::new(MockSource::new(
                        &sc.name,
                        class,
                        sc.weight,
                        sc.rate_bps,
                        sc.jitter_bps,
                    )),
                    #[cfg(feature = "alloy-backend")]
                    "aave_v3" => {
                        let rpc = sc.rpc_url.as_deref().unwrap_or("http://127.0.0.1:8545");
                        if !reachable_rpcs.contains(rpc) {
                            info!(source = %sc.name, "RPC unreachable, marking source offline");
                            Box::new(OfflineSource::new(&sc.name, class, sc.weight, "aave_v3"))
                        } else {
                            match build_alloy_source_aave_v3(sc, &mut provider_cache) {
                                Ok(s) => s,
                                Err(e) => {
                                    warn!(source = %sc.name, error = %e, "failed to build aave_v3 source, marking offline");
                                    Box::new(OfflineSource::new(&sc.name, class, sc.weight, "aave_v3"))
                                }
                            }
                        }
                    }
                    #[cfg(feature = "alloy-backend")]
                    "compound_v3" => {
                        let rpc = sc.rpc_url.as_deref().unwrap_or("http://127.0.0.1:8545");
                        if !reachable_rpcs.contains(rpc) {
                            info!(source = %sc.name, "RPC unreachable, marking source offline");
                            Box::new(OfflineSource::new(&sc.name, class, sc.weight, "compound_v3"))
                        } else {
                            match build_alloy_source_compound_v3(sc, &mut provider_cache) {
                                Ok(s) => s,
                                Err(e) => {
                                    warn!(source = %sc.name, error = %e, "failed to build compound_v3 source, marking offline");
                                    Box::new(OfflineSource::new(&sc.name, class, sc.weight, "compound_v3"))
                                }
                            }
                        }
                    }
                    #[cfg(feature = "alloy-backend")]
                    "ethena" => {
                        let rpc = sc.rpc_url.as_deref().unwrap_or("http://127.0.0.1:8545");
                        if !reachable_rpcs.contains(rpc) {
                            info!(source = %sc.name, "RPC unreachable, marking source offline");
                            Box::new(OfflineSource::new(&sc.name, class, sc.weight, "ethena"))
                        } else {
                            match build_alloy_source_ethena(sc, &mut provider_cache) {
                                Ok(s) => s,
                                Err(e) => {
                                    warn!(source = %sc.name, error = %e, "failed to build ethena source, marking offline");
                                    Box::new(OfflineSource::new(&sc.name, class, sc.weight, "ethena"))
                                }
                            }
                        }
                    }
                    #[cfg(feature = "alloy-backend")]
                    "eth_staking" => {
                        let rpc = sc.rpc_url.as_deref().unwrap_or("http://127.0.0.1:8545");
                        if !reachable_rpcs.contains(rpc) {
                            info!(source = %sc.name, "RPC unreachable, marking source offline");
                            Box::new(OfflineSource::new(&sc.name, class, sc.weight, "eth_staking"))
                        } else {
                            match build_alloy_source_lido(sc, &mut provider_cache) {
                                Ok(s) => s,
                                Err(e) => {
                                    warn!(source = %sc.name, error = %e, "failed to build lido source, marking offline");
                                    Box::new(OfflineSource::new(&sc.name, class, sc.weight, "eth_staking"))
                                }
                            }
                        }
                    }
                    _ => {
                        warn!(
                            source = %sc.name,
                            kind = %sc.kind,
                            "unknown source kind, falling back to mock"
                        );
                        Box::new(MockSource::new(
                            &sc.name,
                            class,
                            sc.weight,
                            sc.rate_bps,
                            sc.jitter_bps,
                        ))
                    }
                }
            })
            .collect();

        Self::new(keeper_id, sources, config)
    }

    /// Whether any source is live (not permanently offline).
    ///
    /// When all sources are offline (RPC unreachable at startup), the keeper
    /// should not run — it would only produce empty/zero composites.
    pub fn has_live_sources(&self) -> bool {
        self.sources.iter().any(|s| !s.is_offline())
    }

    /// Set the relay publish callback. Called after each successful tick.
    pub fn set_publish_fn(&self, f: PublishFn) {
        *self.publish_fn.write() = Some(f);
    }

    /// Run the keeper loop until `cancel` fires.
    ///
    /// Uses `tokio::time::sleep` between ticks (not `interval`) so a slow
    /// tick doesn't cause drift accumulation.
    pub async fn run(&self, cancel: tokio_util::sync::CancellationToken) {
        info!(keeper = %self.keeper_id, sources = self.sources.len(), "ISFR keeper starting");
        let sleep_dur = Duration::from_secs(self.config.poll_interval_secs);
        let mut all_offline_ticks: u32 = 0;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!(keeper = %self.keeper_id, "ISFR keeper shutting down");
                    return;
                }
                _ = tokio::time::sleep(sleep_dur) => {
                    self.tick().await;

                    // If fewer than min_submissions sources are healthy for
                    // 3+ consecutive ticks, suspend — the composite is unreliable.
                    let metas = self.source_metas.read();
                    let healthy = metas.iter().filter(|m| m.consecutive_failures == 0).count();
                    let total = self.sources.len();
                    let min_needed = (self.config.min_submissions.max(1) as usize)
                        .max(total.div_ceil(4)); // at least 25% of sources
                    drop(metas);

                    if healthy >= min_needed {
                        all_offline_ticks = 0;
                    } else {
                        all_offline_ticks += 1;
                        if all_offline_ticks >= 3 {
                            info!(
                                keeper = %self.keeper_id,
                                healthy,
                                min_needed,
                                "insufficient healthy sources for 3 ticks; suspending keeper"
                            );
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Single poll-compute-publish cycle. Public for direct testing.
    pub async fn tick(&self) {
        // Update epoch from wall-clock drift since start.
        let epoch_dur = self.config.epoch_duration_secs.max(1);
        let new_epoch = self.start_time.elapsed().as_secs() / epoch_dur;
        self.current_epoch.store(new_epoch, Ordering::Relaxed);

        let readings = self.poll_sources().await;

        // Skip if too few sources reported — composite would be unreliable.
        let total = self.sources.len();
        let min_needed = (self.config.min_submissions.max(1) as usize).max(total.div_ceil(4));
        if readings.len() < min_needed {
            tracing::debug!(
                keeper = %self.keeper_id,
                readings = readings.len(),
                min_needed,
                "insufficient readings, skipping publish"
            );
            return;
        }

        let src_refs: Vec<&dyn ISFRSource> = self.sources.iter().map(|s| s.as_ref()).collect();
        let composite = compute_composite(&readings, &src_refs);

        *self.current_rate.write() = Some(composite.clone());

        if let Some(publish_fn) = self.publish_fn.read().as_ref() {
            let payload = serde_json::to_value(&composite).unwrap_or_default();
            // Topic MUST be "isfr:rates" — matches B1's ISFRFeed::relay_topics() subscription.
            (publish_fn)("isfr:rates", "rate_update", payload);
        }

        info!(
            keeper = %self.keeper_id,
            epoch = new_epoch,
            composite_bps = composite.composite_bps,
            confidence = composite.confidence_bps,
            sources = readings.len(),
            "published composite rate"
        );
    }

    /// Poll all sources sequentially with per-source timeout.
    ///
    /// Uses sequential (not spawned) polling because trait objects are not
    /// easily moved across spawn boundaries without Arc wrapping. Each
    /// source has its own `liveness_timeout_ms`.
    async fn poll_sources(&self) -> Vec<SourceReading> {
        let mut readings = Vec::with_capacity(self.sources.len());
        let mut new_failures: Vec<String> = Vec::new();

        for (i, source) in self.sources.iter().enumerate() {
            let name = source.name().to_string();
            let timeout = Duration::from_millis(source.liveness_timeout_ms());

            match tokio::time::timeout(timeout, source.fetch_rate()).await {
                Ok(Ok(reading)) => {
                    {
                        let mut metas = self.source_metas.write();
                        if let Some(meta) = metas.get_mut(i) {
                            if meta.consecutive_failures > 0 {
                                info!(source = %name, "source recovered after {} failures", meta.consecutive_failures);
                            }
                            meta.last_reading = Some(reading.clone());
                            meta.status = SourceStatus::Live;
                            meta.consecutive_failures = 0;
                        }
                    }
                    readings.push(reading);
                }
                Ok(Err(_e)) => {
                    let mut metas = self.source_metas.write();
                    if let Some(meta) = metas.get_mut(i) {
                        let was_ok = meta.consecutive_failures == 0;
                        meta.consecutive_failures += 1;
                        meta.status = if meta.consecutive_failures >= 3 {
                            SourceStatus::Offline
                        } else {
                            SourceStatus::Stale
                        };
                        if was_ok {
                            new_failures.push(name);
                        }
                    }
                }
                Err(_) => {
                    let mut metas = self.source_metas.write();
                    if let Some(meta) = metas.get_mut(i) {
                        let was_ok = meta.consecutive_failures == 0;
                        meta.consecutive_failures += 1;
                        meta.status = SourceStatus::Stale;
                        if was_ok {
                            new_failures.push(format!("{name} (timeout)"));
                        }
                    }
                }
            }
        }

        if !new_failures.is_empty() {
            warn!(
                failed = new_failures.len(),
                ok = readings.len(),
                total = self.sources.len(),
                sources = %new_failures.join(", "),
                "ISFR sources failed on first poll"
            );
        }

        readings
    }

    /// Get the most recently computed composite rate.
    pub fn current_rate(&self) -> Option<CompositeRate> {
        self.current_rate.read().clone()
    }

    /// Get source metadata for status/health reporting.
    pub fn source_metas(&self) -> Vec<SourceMeta> {
        self.source_metas.read().clone()
    }

    /// Current epoch number (derived from start_time and epoch_duration_secs).
    pub fn current_epoch(&self) -> u64 {
        self.current_epoch.load(Ordering::Relaxed)
    }
}

// ─── Alloy source builders (feature-gated) ───────────────────────────────────

#[cfg(feature = "alloy-backend")]
fn get_or_create_provider(
    sc: &SourceConfig,
    cache: &mut std::collections::HashMap<String, Arc<alloy::providers::DynProvider>>,
) -> anyhow::Result<Arc<alloy::providers::DynProvider>> {
    use alloy::providers::Provider as _;
    let rpc_url = sc.rpc_url.as_deref().unwrap_or("http://127.0.0.1:8545");

    if let Some(p) = cache.get(rpc_url) {
        return Ok(Arc::clone(p));
    }

    let url: reqwest::Url = rpc_url
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid rpc_url '{}': {}", rpc_url, e))?;
    let provider = alloy::providers::ProviderBuilder::new()
        .connect_http(url)
        .erased();
    let provider = Arc::new(provider);
    cache.insert(rpc_url.to_string(), Arc::clone(&provider));
    Ok(provider)
}

#[cfg(feature = "alloy-backend")]
fn parse_address(s: &str) -> anyhow::Result<alloy::primitives::Address> {
    s.parse::<alloy::primitives::Address>()
        .map_err(|e| anyhow::anyhow!("invalid address '{}': {}", s, e))
}

#[cfg(feature = "alloy-backend")]
fn build_alloy_source_aave_v3(
    sc: &SourceConfig,
    cache: &mut std::collections::HashMap<String, Arc<alloy::providers::DynProvider>>,
) -> anyhow::Result<Box<dyn ISFRSource>> {
    use crate::isfr_sources::aave_v3::AaveV3Source;

    let provider = get_or_create_provider(sc, cache)?;
    let pool = parse_address(
        sc.pool_address
            .as_deref()
            .unwrap_or("0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"),
    )?;
    // Default USDC asset for Aave V3
    let asset = parse_address("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")?;

    Ok(Box::new(AaveV3Source::new(
        sc.name.clone(),
        sc.weight,
        provider,
        pool,
        asset,
    )))
}

#[cfg(feature = "alloy-backend")]
fn build_alloy_source_compound_v3(
    sc: &SourceConfig,
    cache: &mut std::collections::HashMap<String, Arc<alloy::providers::DynProvider>>,
) -> anyhow::Result<Box<dyn ISFRSource>> {
    use crate::isfr_sources::compound_v3::CompoundV3Source;

    let provider = get_or_create_provider(sc, cache)?;
    let comet = parse_address(
        sc.pool_address
            .as_deref()
            .unwrap_or("0xc3d688B66703497DAA19211EEdff47f25384cdc3"),
    )?;

    Ok(Box::new(CompoundV3Source::new(
        sc.name.clone(),
        sc.weight,
        provider,
        comet,
    )))
}

#[cfg(feature = "alloy-backend")]
fn build_alloy_source_ethena(
    sc: &SourceConfig,
    cache: &mut std::collections::HashMap<String, Arc<alloy::providers::DynProvider>>,
) -> anyhow::Result<Box<dyn ISFRSource>> {
    use crate::isfr_sources::ethena::EthenaSource;

    let provider = get_or_create_provider(sc, cache)?;
    let susde = parse_address(
        sc.pool_address
            .as_deref()
            .unwrap_or("0x9D39A5DE30e57443BfF2A8307A4256c8797A3497"),
    )?;

    Ok(Box::new(EthenaSource::new(
        sc.name.clone(),
        sc.weight,
        provider,
        susde,
    )))
}

#[cfg(feature = "alloy-backend")]
fn build_alloy_source_lido(
    sc: &SourceConfig,
    cache: &mut std::collections::HashMap<String, Arc<alloy::providers::DynProvider>>,
) -> anyhow::Result<Box<dyn ISFRSource>> {
    use crate::isfr_sources::lido::LidoStakingSource;

    let provider = get_or_create_provider(sc, cache)?;
    let steth = parse_address(
        sc.pool_address
            .as_deref()
            .unwrap_or("0xae7ab96520DE3A18E5e111B5EaAb095312D7fE84"),
    )?;

    Ok(Box::new(LidoStakingSource::new(
        sc.name.clone(),
        sc.weight,
        provider,
        steth,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_keeper_produces_composite() {
        let keeper = ISFRKeeper::mock_keeper("test-keeper", ISFRKeeperConfig::default());
        keeper.tick().await;
        let rate = keeper.current_rate().expect("should have rate after tick");
        assert!(rate.composite_bps > 0);
        assert!(rate.lending_bps > 0);
        assert!(rate.staking_bps > 0);
        // All 4 mock sources are live, so confidence = 10000.
        assert_eq!(rate.confidence_bps, 10_000);
    }

    #[tokio::test]
    async fn keeper_publishes_to_callback() {
        let published = Arc::new(AtomicU64::new(0));
        let published_clone = published.clone();

        let keeper = ISFRKeeper::mock_keeper("test-keeper", ISFRKeeperConfig::default());
        keeper.set_publish_fn(Arc::new(move |_topic, _msg_type, _payload| {
            published_clone.fetch_add(1, Ordering::Relaxed);
        }));

        keeper.tick().await;
        assert_eq!(published.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn keeper_tracks_source_metas() {
        let keeper = ISFRKeeper::mock_keeper("test-keeper", ISFRKeeperConfig::default());
        keeper.tick().await;
        let metas = keeper.source_metas();
        assert_eq!(metas.len(), 4);
        assert!(metas.iter().all(|m| m.status == SourceStatus::Live));
        assert!(metas.iter().all(|m| m.last_reading.is_some()));
    }

    #[tokio::test]
    async fn from_config_builds_mock_sources() {
        let configs = vec![
            SourceConfig {
                name: "test-aave".to_string(),
                kind: "mock".to_string(),
                weight: 0.5,
                class: "lending".to_string(),
                rate_bps: 600,
                jitter_bps: 0,
                rpc_url: None,
                pool_address: None,
            },
            SourceConfig {
                name: "test-staking".to_string(),
                kind: "mock".to_string(),
                weight: 0.5,
                class: "staking".to_string(),
                rate_bps: 350,
                jitter_bps: 0,
                rpc_url: None,
                pool_address: None,
            },
        ];
        let keeper = ISFRKeeper::from_config("cfg-keeper", ISFRKeeperConfig::default(), &configs);
        keeper.tick().await;
        let rate = keeper.current_rate().unwrap();
        assert!(rate.composite_bps > 0);
        assert_eq!(rate.confidence_bps, 10_000);
    }
}
