//! ISFR rate source trait and types.
//!
//! Each source fetches a rate observation from a DeFi protocol.
//! The ISFRKeeper aggregates multiple sources via weighted median.

#[cfg(feature = "alloy-backend")]
pub mod aave_v3;
#[cfg(feature = "alloy-backend")]
pub mod compound_v3;
#[cfg(feature = "alloy-backend")]
pub mod ethena;
#[cfg(feature = "alloy-backend")]
pub mod lido;
pub mod mock;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Rate classification — which DeFi sector this rate represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateClass {
    /// Lending protocols (Aave, Compound).
    Lending,
    /// Structured products (Ethena sUSDe, yield tokens).
    Structured,
    /// Funding rates (perp exchanges, Hyperliquid).
    Funding,
    /// Native staking (ETH beacon chain, LSTs).
    Staking,
}

impl RateClass {
    /// Return the snake_case string representation of this rate class.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lending => "lending",
            Self::Structured => "structured",
            Self::Funding => "funding",
            Self::Staking => "staking",
        }
    }

    /// All rate class variants in declaration order.
    pub fn all() -> &'static [Self] {
        &[
            Self::Lending,
            Self::Structured,
            Self::Funding,
            Self::Staking,
        ]
    }
}

impl std::fmt::Display for RateClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for RateClass {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "lending" => Ok(Self::Lending),
            "structured" => Ok(Self::Structured),
            "funding" => Ok(Self::Funding),
            "staking" => Ok(Self::Staking),
            _ => anyhow::bail!("unknown rate class: {s}"),
        }
    }
}

/// A single rate observation from one source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReading {
    /// Source identifier (e.g. "aave-v3-usdc").
    pub source: String,
    /// Rate in basis points (1 bps = 0.01%).
    pub rate_bps: u64,
    /// When this reading was taken (unix ms).
    pub timestamp_ms: u64,
    /// Whether the source is live and responsive.
    pub is_live: bool,
    /// Additional metadata (protocol-specific).
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Source liveness status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    /// Source is responding normally.
    Live,
    /// Source hasn't responded within expected interval but isn't timed out.
    Stale,
    /// Source has exceeded liveness timeout.
    Offline,
}

/// Metadata about a source's current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMeta {
    /// Unique source identifier.
    pub name: String,
    /// Rate classification for this source.
    pub class: RateClass,
    /// Composite weight (0.0–1.0).
    pub weight: f64,
    /// Milliseconds before a non-responding source is considered offline.
    pub liveness_timeout_ms: u64,
    /// Most recent reading from this source, if any.
    pub last_reading: Option<SourceReading>,
    /// Current liveness status.
    pub status: SourceStatus,
    /// Number of consecutive fetch failures since last success.
    pub consecutive_failures: u32,
}

/// Pluggable rate source trait.
///
/// Implement this for each DeFi protocol you want to read rates from.
/// The ISFRKeeper calls `fetch_rate()` on all sources sequentially with
/// individual per-source timeouts.
#[async_trait]
pub trait ISFRSource: Send + Sync {
    /// Unique name for this source instance.
    fn name(&self) -> &str;

    /// Fetch the current rate reading.
    async fn fetch_rate(&self) -> anyhow::Result<SourceReading>;

    /// Weight of this source in the composite calculation (0.0–1.0).
    fn weight(&self) -> f64;

    /// Which rate class this source belongs to.
    fn rate_class(&self) -> RateClass;

    /// How long before a non-responding source is considered offline (ms).
    fn liveness_timeout_ms(&self) -> u64 {
        30_000 // 30s default
    }

    /// Whether this source is permanently offline (e.g. RPC unreachable at startup).
    /// Default: `false`. Overridden by [`mock::OfflineSource`].
    fn is_offline(&self) -> bool {
        false
    }
}

/// Composite rate computed from multiple sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeRate {
    /// Composite rate across all classes (bps).
    pub composite_bps: u64,
    /// Per-class rates (bps). Zero when no source for that class responded.
    pub lending_bps: u64,
    /// Structured product rate (bps).
    pub structured_bps: u64,
    /// Funding rate (bps).
    pub funding_bps: u64,
    /// Native staking rate (bps).
    pub staking_bps: u64,
    /// Confidence in the composite (bps, 0–10000 = 0–100%).
    /// Computed as `(live_source_count / total_source_count) * 10000`.
    pub confidence_bps: u64,
    /// When this composite was computed (unix ms).
    pub timestamp_ms: u64,
    /// Individual source readings that contributed.
    pub readings: Vec<SourceReading>,
}

/// Compute the weighted median of `(value, weight)` pairs.
///
/// Pairs are sorted by value, then the first pair whose cumulative weight
/// reaches `total_weight / 2` is the median. Returns 0 if input is empty.
pub fn weighted_median(values: &mut [(u64, f64)]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_by_key(|(v, _)| *v);
    let total_weight: f64 = values.iter().map(|(_, w)| w).sum();
    if total_weight <= 0.0 {
        return values[values.len() / 2].0;
    }
    let mut cumulative = 0.0;
    for &(value, weight) in values.iter() {
        cumulative += weight;
        if cumulative >= total_weight / 2.0 {
            return value;
        }
    }
    values.last().map(|(v, _)| *v).unwrap_or(0)
}

/// Compute a composite rate from multiple source readings.
///
/// Groups readings by rate class, computes weighted median per class,
/// then averages non-zero classes for the overall composite.
pub fn compute_composite(readings: &[SourceReading], sources: &[&dyn ISFRSource]) -> CompositeRate {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut class_rates: std::collections::HashMap<&str, Vec<(u64, f64)>> =
        std::collections::HashMap::new();

    for reading in readings {
        if !reading.is_live {
            continue;
        }
        if let Some(src) = sources.iter().find(|s| s.name() == reading.source) {
            class_rates
                .entry(src.rate_class().as_str())
                .or_default()
                .push((reading.rate_bps, src.weight()));
        }
    }

    let lending_bps = class_rates
        .get_mut("lending")
        .map(|v| weighted_median(v))
        .unwrap_or(0);
    let structured_bps = class_rates
        .get_mut("structured")
        .map(|v| weighted_median(v))
        .unwrap_or(0);
    let funding_bps = class_rates
        .get_mut("funding")
        .map(|v| weighted_median(v))
        .unwrap_or(0);
    let staking_bps = class_rates
        .get_mut("staking")
        .map(|v| weighted_median(v))
        .unwrap_or(0);

    // Composite = equal-weighted average of non-zero classes.
    let class_values: Vec<u64> = [lending_bps, structured_bps, funding_bps, staking_bps]
        .iter()
        .copied()
        .filter(|&v| v > 0)
        .collect();
    let composite_bps = class_values
        .iter()
        .sum::<u64>()
        .checked_div(class_values.len() as u64)
        .unwrap_or(0);

    // Confidence = proportion of live sources * 10000.
    let total_sources = sources.len() as u64;
    let live_sources = readings.iter().filter(|r| r.is_live).count() as u64;
    let confidence_bps = (live_sources * 10_000)
        .checked_div(total_sources)
        .unwrap_or(0);

    CompositeRate {
        composite_bps,
        lending_bps,
        structured_bps,
        funding_bps,
        staking_bps,
        confidence_bps,
        timestamp_ms: now,
        readings: readings.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mock::MockSource;

    #[tokio::test]
    async fn mock_sources_return_live_readings() {
        let sources: Vec<Box<dyn ISFRSource>> = vec![
            Box::new(MockSource::aave_mock()),
            Box::new(MockSource::compound_mock()),
            Box::new(MockSource::ethena_mock()),
            Box::new(MockSource::staking_mock()),
        ];

        let mut readings = Vec::new();
        for src in &sources {
            let reading = src.fetch_rate().await.unwrap();
            assert!(reading.is_live);
            assert!(reading.rate_bps > 0);
            readings.push(reading);
        }

        let src_refs: Vec<&dyn ISFRSource> = sources.iter().map(|s| s.as_ref()).collect();
        let composite = compute_composite(&readings, &src_refs);
        assert!(composite.composite_bps > 0);
        assert_eq!(composite.confidence_bps, 10_000); // 4/4 live
        assert!(composite.lending_bps > 0);
        assert!(composite.staking_bps > 0);
    }

    #[test]
    fn weighted_median_equal_weights() {
        let mut values = vec![(100u64, 1.0f64), (200, 1.0), (300, 1.0)];
        assert_eq!(weighted_median(&mut values), 200);
    }

    #[test]
    fn weighted_median_heavy_low() {
        // Weight of 100 is 3x; should pull median to 100.
        let mut values = vec![(100u64, 3.0f64), (500, 1.0)];
        assert_eq!(weighted_median(&mut values), 100);
    }

    #[test]
    fn weighted_median_empty() {
        let mut values: Vec<(u64, f64)> = vec![];
        assert_eq!(weighted_median(&mut values), 0);
    }

    #[test]
    fn rate_class_roundtrip() {
        for class in RateClass::all() {
            let s = class.as_str();
            let parsed: RateClass = s.parse().unwrap();
            assert_eq!(*class, parsed);
        }
    }
}
