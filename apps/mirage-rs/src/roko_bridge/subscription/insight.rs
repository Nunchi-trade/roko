//! [`InsightSubscription`] + [`InsightBus`] — live push of insight events
//! (posts, state transitions, confirmations, challenges, decay) from mirage's
//! knowledge layer into roko chain-watchers.
//!
//! Mirage's [`KnowledgeStore`](crate::chain::KnowledgeStore) invokes
//! [`InsightBus::broadcast`] on every lifecycle event. Each subscription sees
//! a copy through its own sink + policy, mirroring the pheromone subscription
//! surface in [`super::pheromone`].

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use super::{
    SubscriptionId,
    backpressure::{BackpressurePolicy, SubscriptionCounters, SubscriptionStats},
    sink::{SinkError, SubscriptionSink},
};

use crate::chain::{InsightId, KnowledgeKind, KnowledgeState};

/// Lifecycle event emitted by a mirage knowledge store.
#[derive(Clone, Debug)]
pub enum InsightEvent {
    /// A fresh entry was posted and indexed.
    Posted {
        /// Content-addressed id of the new entry.
        id: InsightId,
        /// Knowledge kind (insight, heuristic, warning, …).
        kind: KnowledgeKind,
        /// Text content of the entry.
        content: String,
        /// Author bytes (generic — caller picks encoding).
        author: Vec<u8>,
        /// Unix timestamp (seconds) of creation.
        created_at: u64,
    },
    /// The entry's lifecycle state changed.
    StateTransition {
        /// Entry id.
        id: InsightId,
        /// Previous state.
        from: KnowledgeState,
        /// New state.
        to: KnowledgeState,
        /// Unix timestamp (seconds) of the transition.
        at: u64,
    },
    /// A confirmation was recorded.
    Confirmed {
        /// Entry id.
        id: InsightId,
        /// Confirmer address bytes.
        by: Vec<u8>,
        /// Unix timestamp (seconds) of the confirmation.
        at: u64,
    },
    /// A challenge was opened.
    Challenged {
        /// Entry id.
        id: InsightId,
        /// Challenger address bytes.
        by: Vec<u8>,
        /// Unix timestamp (seconds) of the challenge.
        at: u64,
    },
    /// Decay updated the entry's weight.
    Decayed {
        /// Entry id.
        id: InsightId,
        /// Refreshed weight after decay.
        new_weight: f32,
        /// Unix timestamp (seconds) of the decay sweep.
        at: u64,
    },
}

impl InsightEvent {
    /// Returns the id of the entry this event describes.
    #[must_use]
    pub fn entry_id(&self) -> InsightId {
        match self {
            Self::Posted { id, .. }
            | Self::StateTransition { id, .. }
            | Self::Confirmed { id, .. }
            | Self::Challenged { id, .. }
            | Self::Decayed { id, .. } => *id,
        }
    }
}

/// A single active subscription on the insight bus.
#[must_use]
pub struct InsightSubscription {
    id: SubscriptionId,
    sink: Arc<dyn SubscriptionSink<InsightEvent>>,
    policy: BackpressurePolicy,
    counters: SubscriptionCounters,
}

impl InsightSubscription {
    /// Constructs a new subscription.
    pub fn new(
        id: SubscriptionId,
        sink: Arc<dyn SubscriptionSink<InsightEvent>>,
        policy: BackpressurePolicy,
    ) -> Self {
        Self {
            id,
            sink,
            policy,
            counters: SubscriptionCounters::new(),
        }
    }

    /// Returns this subscription's id.
    #[must_use]
    pub fn subscription_id(&self) -> SubscriptionId {
        self.id
    }

    /// Returns the back-pressure policy in use.
    #[must_use]
    pub fn policy(&self) -> BackpressurePolicy {
        self.policy
    }

    /// Current counters snapshot.
    #[must_use]
    pub fn stats(&self) -> SubscriptionStats {
        self.counters.snapshot()
    }

    /// Whether this subscription has been closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.counters.is_closed() || self.sink.is_closed()
    }

    /// Emits one event through the sink, applying back-pressure.
    pub fn emit(&self, event: InsightEvent) -> Result<(), SinkError> {
        if self.counters.is_closed() {
            return Err(SinkError::Closed);
        }
        match self.sink.push(event) {
            Ok(()) => {
                self.counters.record_delivered();
                Ok(())
            }
            Err(SinkError::Closed) => {
                self.counters.mark_closed();
                Err(SinkError::Closed)
            }
            Err(SinkError::Full { dropped }) => self.handle_overflow(dropped),
        }
    }

    fn handle_overflow(&self, dropped: usize) -> Result<(), SinkError> {
        match self.policy {
            BackpressurePolicy::DropOldest => {
                // The sink accepted the new event but evicted `dropped` old ones.
                // Net effective deliveries = 0, so only `dropped_oldest` advances.
                self.counters.record_dropped_oldest(dropped as u64);
                Ok(())
            }
            BackpressurePolicy::DropNewest => {
                self.counters.record_dropped_newest(dropped as u64);
                Err(SinkError::Full { dropped })
            }
            BackpressurePolicy::CloseOnOverflow => {
                self.counters.record_dropped_newest(dropped as u64);
                self.counters.mark_closed();
                Err(SinkError::Closed)
            }
        }
    }
}

impl std::fmt::Debug for InsightSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsightSubscription")
            .field("id", &self.id)
            .field("policy", &self.policy)
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

/// Fan-out point for knowledge-layer events.
#[must_use]
pub struct InsightBus {
    subs: RwLock<HashMap<SubscriptionId, InsightSubscription>>,
    next_id: RwLock<u64>,
}

impl InsightBus {
    /// Constructs an empty bus.
    pub fn new() -> Self {
        Self {
            subs: RwLock::new(HashMap::new()),
            next_id: RwLock::new(1),
        }
    }

    /// Registers a new subscription. Returns its assigned id.
    pub fn register(
        &self,
        sink: Arc<dyn SubscriptionSink<InsightEvent>>,
        policy: BackpressurePolicy,
    ) -> SubscriptionId {
        let id = {
            let mut n = self.next_id.write();
            let id = SubscriptionId(*n);
            *n = n.saturating_add(1);
            id
        };
        let sub = InsightSubscription::new(id, sink, policy);
        self.subs.write().insert(id, sub);
        id
    }

    /// Unregisters the subscription with `id`. Returns whether anything was removed.
    pub fn unregister(&self, id: SubscriptionId) -> bool {
        self.subs.write().remove(&id).is_some()
    }

    /// Number of active subscriptions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.subs.read().len()
    }

    /// Whether the bus has no subscriptions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.subs.read().is_empty()
    }

    /// Broadcasts one event to every registered subscription. Returns the
    /// number of successful deliveries.
    pub fn broadcast(&self, event: InsightEvent) -> usize {
        let subs = self.subs.read();
        let mut delivered = 0usize;
        for sub in subs.values() {
            if sub.emit(event.clone()).is_ok() {
                delivered += 1;
            }
        }
        delivered
    }

    /// Returns the stats for one subscription.
    #[must_use]
    pub fn stats(&self, id: SubscriptionId) -> Option<SubscriptionStats> {
        self.subs.read().get(&id).map(|s| s.stats())
    }

    /// Returns `(id, stats)` for every active subscription.
    #[must_use]
    pub fn all_stats(&self) -> Vec<(SubscriptionId, SubscriptionStats)> {
        self.subs
            .read()
            .iter()
            .map(|(id, s)| (*id, s.stats()))
            .collect()
    }

    /// Drops closed subscriptions. Returns the number removed.
    pub fn prune_closed(&self) -> usize {
        let mut subs = self.subs.write();
        let before = subs.len();
        subs.retain(|_, s| !s.is_closed());
        before - subs.len()
    }
}

impl Default for InsightBus {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InsightBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsightBus")
            .field("subscriptions", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roko_bridge::subscription::sink::{MpscSink, VecSink};

    fn posted_event(tag: u8) -> InsightEvent {
        InsightEvent::Posted {
            id: InsightId([tag; 16]),
            kind: KnowledgeKind::Insight,
            content: format!("insight #{tag}"),
            author: b"alice".to_vec(),
            created_at: 1_700_000_000,
        }
    }

    #[test]
    fn bus_broadcasts_to_all_subs() {
        let bus = InsightBus::new();
        let sink_a: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
        let sink_b: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
        bus.register(sink_a.clone(), BackpressurePolicy::DropOldest);
        bus.register(sink_b.clone(), BackpressurePolicy::DropOldest);

        let delivered = bus.broadcast(posted_event(1));
        assert_eq!(delivered, 2);
        assert_eq!(sink_a.len(), 1);
        assert_eq!(sink_b.len(), 1);
    }

    #[test]
    fn unregister_stops_delivery() {
        let bus = InsightBus::new();
        let sink_a: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
        let sink_b: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());
        let id_a = bus.register(sink_a.clone(), BackpressurePolicy::DropOldest);
        bus.register(sink_b.clone(), BackpressurePolicy::DropOldest);

        assert!(bus.unregister(id_a));
        bus.broadcast(posted_event(2));
        assert!(sink_a.is_empty());
        assert_eq!(sink_b.len(), 1);
    }

    #[test]
    fn state_transition_records_both_states() {
        let ev = InsightEvent::StateTransition {
            id: InsightId([0; 16]),
            from: KnowledgeState::Active,
            to: KnowledgeState::Challenged,
            at: 1_000,
        };
        match ev {
            InsightEvent::StateTransition { from, to, .. } => {
                assert_eq!(from, KnowledgeState::Active);
                assert_eq!(to, KnowledgeState::Challenged);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn entry_id_is_accessible_on_every_variant() {
        let id = InsightId([7; 16]);
        let variants = [
            InsightEvent::Posted {
                id,
                kind: KnowledgeKind::Warning,
                content: "w".into(),
                author: Vec::new(),
                created_at: 0,
            },
            InsightEvent::StateTransition {
                id,
                from: KnowledgeState::Active,
                to: KnowledgeState::Decaying,
                at: 1,
            },
            InsightEvent::Confirmed {
                id,
                by: Vec::new(),
                at: 2,
            },
            InsightEvent::Challenged {
                id,
                by: Vec::new(),
                at: 3,
            },
            InsightEvent::Decayed {
                id,
                new_weight: 0.5,
                at: 4,
            },
        ];
        for v in variants {
            assert_eq!(v.entry_id(), id);
        }
    }

    #[tokio::test]
    async fn close_on_overflow_policy_latches() {
        let (sink, _rx) = MpscSink::<InsightEvent>::new(2);
        let bus = InsightBus::new();
        let id = bus.register(Arc::new(sink), BackpressurePolicy::CloseOnOverflow);
        bus.broadcast(posted_event(1));
        bus.broadcast(posted_event(2));
        // Third broadcast hits the cap → subscription closes silently (delivery=0).
        let delivered = bus.broadcast(posted_event(3));
        assert_eq!(delivered, 0);
        let stats = bus.stats(id).unwrap();
        assert!(stats.closed);
        assert_eq!(stats.delivered, 2);
        assert_eq!(stats.dropped_newest, 1);
    }
}
