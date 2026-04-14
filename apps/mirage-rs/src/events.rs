//! Telemetry events emitted by mirage internals.
//!
//! Mirage publishes cross-subsystem telemetry on a [`roko_runtime::event_bus::EventBus`]
//! so that observers (other roko crates, monitoring daemons, TUI frontends) can subscribe
//! to resource pressure, cache behaviour, and proxy-mode transitions without polling.
//!
//! This enum replaces the earlier `golem-core::event::EventPayload` coupling so that
//! `mirage-rs` can live in the roko workspace without pulling in `golem-core`.
//! The roko integration bridge (feature `roko`) can re-route these events into a
//! `roko-core` `Engram` stream if needed.
//!
//! ```no_run
//! use roko_runtime::event_bus::EventBus;
//! use mirage_rs::events::MirageTelemetryEvent;
//!
//! let bus = EventBus::<MirageTelemetryEvent>::new(1024);
//! let sender = bus.sender();
//! sender.emit(MirageTelemetryEvent::ResourceWarning {
//!     resource: "memory".into(),
//!     utilization: 0.62,
//! });
//! ```

use serde::{Deserialize, Serialize};

/// Telemetry event emitted by mirage subsystems.
///
/// Variants are additive — downstream consumers should handle unknown variants
/// defensively. Emissions are non-blocking (dropped when the ring is saturated).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MirageTelemetryEvent {
    /// Resource pressure warning (memory, cache, etc.).
    ///
    /// `utilization` is in `[0.0, 1.0]`. Emitted from the resource-pressure tier
    /// transitions (see `resources::ResourceUsage::pressure_action`).
    ResourceWarning {
        /// Name of the constrained resource (`"memory"`, `"cache"`, ...).
        resource: String,
        /// Fraction of the ceiling currently in use.
        utilization: f64,
    },
    /// Mirage transitioned into proxy mode (all simulation state dropped, passthrough reads).
    ProxyDemoted {
        /// Reason that triggered the demotion.
        reason: String,
    },
    /// Mirage recovered out of proxy mode.
    ProxyRestored,
    /// A batch of read-cache entries was evicted.
    CacheEvicted {
        /// Number of entries dropped.
        evicted: u64,
    },
    /// An upstream RPC fetch was retried.
    UpstreamFetchRetry {
        /// Name of the upstream method being retried.
        method: String,
        /// Attempt number (1-based).
        attempt: u32,
    },
    /// A transaction was committed to the local fork.
    TransactionCommitted {
        /// Transaction hash (hex-encoded, `0x`-prefixed).
        tx_hash: String,
        /// Gas used by the transaction.
        gas_used: u64,
    },
}

impl MirageTelemetryEvent {
    /// Returns a stable string tag for each variant (useful for metrics/logging).
    #[must_use]
    pub const fn tag(&self) -> &'static str {
        match self {
            Self::ResourceWarning { .. } => "resource_warning",
            Self::ProxyDemoted { .. } => "proxy_demoted",
            Self::ProxyRestored => "proxy_restored",
            Self::CacheEvicted { .. } => "cache_evicted",
            Self::UpstreamFetchRetry { .. } => "upstream_fetch_retry",
            Self::TransactionCommitted { .. } => "transaction_committed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MirageTelemetryEvent;
    use roko_runtime::event_bus::EventBus;

    #[test]
    fn resource_warning_roundtrips_through_bus() {
        let bus = EventBus::<MirageTelemetryEvent>::new(4);
        let sender = bus.sender();
        sender.emit(MirageTelemetryEvent::ResourceWarning {
            resource: "memory".into(),
            utilization: 0.62,
        });
        let replay = bus.replay_from(0);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].payload.tag(), "resource_warning");
        assert_eq!(
            replay[0].payload,
            MirageTelemetryEvent::ResourceWarning {
                resource: "memory".into(),
                utilization: 0.62,
            }
        );
    }

    #[test]
    fn telemetry_event_serializes_as_tagged_json() {
        let event = MirageTelemetryEvent::ProxyDemoted {
            reason: "memory_emergency".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"kind\":\"proxy_demoted\""));
        assert!(json.contains("\"reason\":\"memory_emergency\""));
    }

    #[test]
    fn telemetry_tags_are_stable() {
        assert_eq!(MirageTelemetryEvent::ProxyRestored.tag(), "proxy_restored");
        assert_eq!(
            MirageTelemetryEvent::CacheEvicted { evicted: 10 }.tag(),
            "cache_evicted"
        );
        assert_eq!(
            MirageTelemetryEvent::TransactionCommitted {
                tx_hash: "0xdead".into(),
                gas_used: 21_000,
            }
            .tag(),
            "transaction_committed"
        );
    }
}
