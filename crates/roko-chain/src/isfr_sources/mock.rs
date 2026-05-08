//! Mock ISFR source for local development and testing.
//!
//! Uses `SystemTime::subsec_nanos()` for deterministic-enough jitter
//! without pulling in the `rand` crate.
//!
//! Also contains [`OfflineSource`] — a permanently-offline source that
//! replaces mock fallbacks when the RPC endpoint is unreachable at startup.

use async_trait::async_trait;

use super::{ISFRSource, RateClass, SourceReading};

/// Mock source that returns a configurable rate with optional nanosecond-jitter.
pub struct MockSource {
    name: String,
    class: RateClass,
    weight: f64,
    base_rate_bps: u64,
    /// Max jitter in bps (applied in ±direction using subsec_nanos).
    jitter_bps: u64,
}

impl MockSource {
    /// Create a new mock source with explicit parameters.
    pub fn new(
        name: &str,
        class: RateClass,
        weight: f64,
        base_rate_bps: u64,
        jitter_bps: u64,
    ) -> Self {
        Self {
            name: name.to_string(),
            class,
            weight,
            base_rate_bps,
            jitter_bps,
        }
    }

    /// Pre-configured mock for Aave V3 USDC lending rate (~6.2%).
    pub fn aave_mock() -> Self {
        Self::new("mock-aave-v3", RateClass::Lending, 0.30, 620, 15)
    }

    /// Pre-configured mock for Compound V3 USDC lending rate (~5.8%).
    pub fn compound_mock() -> Self {
        Self::new("mock-compound-v3", RateClass::Lending, 0.25, 580, 10)
    }

    /// Pre-configured mock for Ethena sUSDe structured yield (~8.5%).
    pub fn ethena_mock() -> Self {
        Self::new("mock-ethena-susde", RateClass::Structured, 0.20, 850, 25)
    }

    /// Pre-configured mock for ETH staking rate (~3.5%).
    pub fn staking_mock() -> Self {
        Self::new("mock-eth-staking", RateClass::Staking, 0.25, 350, 5)
    }
}

#[async_trait]
impl ISFRSource for MockSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn fetch_rate(&self) -> anyhow::Result<SourceReading> {
        // Deterministic jitter from wall-clock subsecond nanos — no rand dep needed.
        let jitter: i64 = if self.jitter_bps > 0 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let range = self.jitter_bps * 2 + 1;
            (nanos as u64 % range) as i64 - self.jitter_bps as i64
        } else {
            0
        };
        let rate_bps = (self.base_rate_bps as i64 + jitter).max(0) as u64;

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(SourceReading {
            source: self.name.clone(),
            rate_bps,
            timestamp_ms,
            is_live: true,
            metadata: serde_json::json!({
                "mock": true,
                "base_rate_bps": self.base_rate_bps,
                "jitter_bps": self.jitter_bps,
            }),
        })
    }

    fn weight(&self) -> f64 {
        self.weight
    }

    fn rate_class(&self) -> RateClass {
        self.class
    }
}

// ─── OfflineSource ───────────────────────────────────────────────────────────

/// A permanently-offline source that replaces mock fallbacks when the RPC
/// endpoint is unreachable at startup. Always returns an error from
/// `fetch_rate()` and reports `is_offline() = true`.
pub struct OfflineSource {
    name: String,
    class: RateClass,
    weight: f64,
    /// The original source kind that couldn't be built (for diagnostics).
    original_kind: String,
}

impl OfflineSource {
    /// Create a new offline source.
    pub fn new(name: &str, class: RateClass, weight: f64, original_kind: &str) -> Self {
        Self {
            name: name.to_string(),
            class,
            weight,
            original_kind: original_kind.to_string(),
        }
    }
}

#[async_trait]
impl ISFRSource for OfflineSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn fetch_rate(&self) -> anyhow::Result<SourceReading> {
        anyhow::bail!(
            "source offline: RPC unreachable at startup (original kind: {})",
            self.original_kind
        )
    }

    fn weight(&self) -> f64 {
        self.weight
    }

    fn rate_class(&self) -> RateClass {
        self.class
    }

    fn is_offline(&self) -> bool {
        true
    }
}
