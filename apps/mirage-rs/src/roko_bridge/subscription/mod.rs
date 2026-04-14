//! §37.c subscription surface — push-based delivery of mirage chain events to
//! roko chain-watchers.
//!
//! `chain-substrate.md` specifies a `subscribe_pheromones() -> PheromoneStream`
//! contract, but before this module only a polling path (`chain_queryPheromones`)
//! existed. This module closes that gap with a synchronous push interface
//! ([`SubscriptionSink`]) that mirage's knowledge-store / pheromone-field write
//! paths can call into without awaiting.
//!
//! # Layout
//!
//! - [`sink`] — [`SubscriptionSink`] trait + stock impls ([`BroadcastSink`],
//!   [`MpscSink`], [`VecSink`]).
//! - [`backpressure`] — [`BackpressurePolicy`] + per-subscription counters.
//! - [`pheromone`] — [`PheromoneEvent`], [`PheromoneSubscription`],
//!   [`PheromoneBus`].
//! - [`insight`] — [`InsightEvent`], [`InsightSubscription`], [`InsightBus`].
//!
//! # Wiring
//!
//! Intended call-site integrations (not performed here — this module is
//! additive):
//!
//! ```ignore
//! let bus = Arc::new(PheromoneBus::new());
//! let (sink, mut rx) = BroadcastSink::<PheromoneEvent>::new(128);
//! let _id = bus.register(Arc::new(sink), BackpressurePolicy::DropOldest);
//! // ...later, inside PheromoneField::deposit:
//! bus.broadcast(kind, vector, intensity, now_secs);
//! ```
//!
//! Subscribers pull from `rx` on their own tasks; mirage's write path never
//! blocks on a slow subscriber because `push` is non-blocking and the bus
//! applies the per-subscription back-pressure policy.

pub mod backpressure;
pub mod insight;
pub mod pheromone;
pub mod sink;

pub use backpressure::{BackpressurePolicy, SubscriptionStats};
pub use insight::{InsightBus, InsightEvent, InsightSubscription};
pub use pheromone::{PheromoneBus, PheromoneEvent, PheromoneSubscription};
pub use sink::{BroadcastSink, MpscSink, SinkError, SubscriptionSink, VecSink};

/// Opaque identifier for a subscription within a single bus.
///
/// Assigned monotonically by [`PheromoneBus::register`] /
/// [`InsightBus::register`]. `0` is reserved; live ids begin at 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SubscriptionId(pub u64);

impl SubscriptionId {
    /// Sentinel id used for tests and placeholder subscriptions.
    pub const SENTINEL: Self = Self(0);

    /// Whether this id is the reserved sentinel.
    #[must_use]
    pub const fn is_sentinel(self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sub#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::PheromoneKind;
    use roko_primitives::HdcVector;
    use std::sync::Arc;

    #[test]
    fn subscription_id_is_copy_eq_hash() {
        let a = SubscriptionId(5);
        let b = a; // Copy
        assert_eq!(a, b);
        let mut set = std::collections::HashSet::new();
        set.insert(a);
        set.insert(b);
        assert_eq!(set.len(), 1, "Copy + Eq + Hash contract");
    }

    #[test]
    fn sentinel_is_zero_and_labeled() {
        assert!(SubscriptionId::SENTINEL.is_sentinel());
        assert!(!SubscriptionId(1).is_sentinel());
        assert_eq!(format!("{}", SubscriptionId(7)), "sub#7");
    }

    #[test]
    fn ids_are_monotonic_within_bus() {
        let bus = PheromoneBus::new();
        let sink_a: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let sink_b: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let id_a = bus.register(sink_a, BackpressurePolicy::DropOldest);
        let id_b = bus.register(sink_b, BackpressurePolicy::DropOldest);
        assert!(id_b.0 > id_a.0);
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn full_and_closed_sink_errors_distinct() {
        assert_ne!(SinkError::Closed, SinkError::Full { dropped: 1 });
    }

    #[tokio::test]
    async fn concurrent_broadcast_from_multiple_tasks() {
        let bus = Arc::new(PheromoneBus::new());
        let sink_a: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let sink_b: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        bus.register(sink_a.clone(), BackpressurePolicy::DropOldest);
        bus.register(sink_b.clone(), BackpressurePolicy::DropOldest);

        let mut handles = Vec::new();
        for worker in 0..4u64 {
            let bus = bus.clone();
            handles.push(tokio::task::spawn_blocking(move || {
                for i in 0..25 {
                    bus.broadcast(
                        PheromoneKind::Threat,
                        HdcVector::from_seed(&worker.to_le_bytes()),
                        (i as f32) / 25.0,
                        1_700_000_000 + worker * 100 + i as u64,
                    );
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // 4 workers × 25 pushes = 100 events per sub.
        assert_eq!(sink_a.len(), 100);
        assert_eq!(sink_b.len(), 100);
        let all = bus.all_stats();
        assert_eq!(all.len(), 2);
        for (_, stats) in all {
            assert_eq!(stats.delivered, 100);
        }
    }

    #[test]
    fn insight_bus_and_pheromone_bus_are_independent() {
        let p_bus = PheromoneBus::new();
        let i_bus = InsightBus::new();
        assert_eq!(p_bus.len(), 0);
        assert_eq!(i_bus.len(), 0);
        assert!(p_bus.is_empty());
        assert!(i_bus.is_empty());
    }

    #[test]
    fn subscription_stats_zero_defaults() {
        let z = SubscriptionStats::zero();
        assert_eq!(z.delivered, 0);
        assert_eq!(z.dropped_oldest, 0);
        assert_eq!(z.dropped_newest, 0);
        assert_eq!(z.total_dropped(), 0);
        assert!(!z.closed);
    }
}
