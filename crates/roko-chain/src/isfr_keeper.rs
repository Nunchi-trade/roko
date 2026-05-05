//! ISFR Keeper — orchestrates rate fetching, aggregation, and publication.
//!
//! Lifecycle:
//! 1. Constructed with sources + config via `new()`, `mock_keeper()`, or `from_config()`.
//! 2. Optionally attach a relay publish callback via `set_publish_fn()`.
//! 3. Call `run(cancel)` to enter the poll loop; it blocks until the token fires.
//! 4. Each tick: poll all sources (with per-source timeout) → compute composite →
//!    store as current rate → invoke publish callback if set.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::isfr_sources::{
    CompositeRate, ISFRSource, RateClass, SourceMeta, SourceReading, SourceStatus,
    compute_composite, mock::MockSource,
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
    pub fn from_config(
        keeper_id: &str,
        config: ISFRKeeperConfig,
        source_configs: &[SourceConfig],
    ) -> Self {
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
                match sc.kind.as_str() {
                    "mock" => Box::new(MockSource::new(
                        &sc.name,
                        class,
                        sc.weight,
                        sc.rate_bps,
                        sc.jitter_bps,
                    )),
                    // Phase 2: "aave_v3", "compound_v3", "ethena", "staking" backends.
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

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!(keeper = %self.keeper_id, "ISFR keeper shutting down");
                    return;
                }
                _ = tokio::time::sleep(sleep_dur) => {
                    self.tick().await;
                }
            }
        }
    }

    /// Single poll-compute-publish cycle. Public for direct testing.
    pub async fn tick(&self) {
        let readings = self.poll_sources().await;

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

        for (i, source) in self.sources.iter().enumerate() {
            let name = source.name().to_string();
            let timeout = Duration::from_millis(source.liveness_timeout_ms());

            match tokio::time::timeout(timeout, source.fetch_rate()).await {
                Ok(Ok(reading)) => {
                    {
                        let mut metas = self.source_metas.write();
                        if let Some(meta) = metas.get_mut(i) {
                            meta.last_reading = Some(reading.clone());
                            meta.status = SourceStatus::Live;
                            meta.consecutive_failures = 0;
                        }
                    }
                    readings.push(reading);
                }
                Ok(Err(e)) => {
                    warn!(source = %name, error = %e, "source fetch failed");
                    let mut metas = self.source_metas.write();
                    if let Some(meta) = metas.get_mut(i) {
                        meta.consecutive_failures += 1;
                        meta.status = if meta.consecutive_failures >= 3 {
                            SourceStatus::Offline
                        } else {
                            SourceStatus::Stale
                        };
                    }
                }
                Err(_) => {
                    warn!(source = %name, "source fetch timed out");
                    let mut metas = self.source_metas.write();
                    if let Some(meta) = metas.get_mut(i) {
                        meta.consecutive_failures += 1;
                        meta.status = SourceStatus::Stale;
                    }
                }
            }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

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
