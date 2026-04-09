//! [`PheromoneSubscription`] + [`PheromoneBus`] — live push of pheromone
//! deposits from mirage into roko chain-watchers.
//!
//! A `PheromoneField::deposit` call in mirage would invoke
//! [`PheromoneBus::broadcast`], fanning out the event to every registered
//! subscription. Each subscription owns its own sink + back-pressure policy;
//! a slow subscriber never stalls mirage's write path or its peers.

use std::collections::HashMap;
use std::sync::Arc;

use bardo_primitives::HdcVector;
use parking_lot::RwLock;

use super::{
    SubscriptionId,
    backpressure::{BackpressurePolicy, SubscriptionCounters, SubscriptionStats},
    sink::{SinkError, SubscriptionSink},
};

use crate::chain::PheromoneKind;

/// An event describing a newly deposited pheromone.
///
/// Re-uses mirage's [`PheromoneKind`] and [`HdcVector`] so downstream
/// subscribers see the exact on-chain representation.
#[derive(Clone, Debug)]
#[must_use]
pub struct PheromoneEvent {
    /// Monotonic event id, unique within a single subscription bus.
    pub id: u64,
    /// Kind of the pheromone (drives default half-life).
    pub kind: PheromoneKind,
    /// 10,240-bit HDC vector of the pheromone.
    pub vector: HdcVector,
    /// Initial intensity at deposit time.
    pub intensity: f32,
    /// Unix timestamp (seconds) of the deposit.
    pub deposited_at: u64,
}

impl PheromoneEvent {
    /// Constructs a new event. The `id` is assigned by the bus; constructors
    /// of standalone events may use any value.
    #[must_use]
    pub fn new(
        id: u64,
        kind: PheromoneKind,
        vector: HdcVector,
        intensity: f32,
        deposited_at: u64,
    ) -> Self {
        Self {
            id,
            kind,
            vector,
            intensity,
            deposited_at,
        }
    }
}

/// A single active subscription on the pheromone bus.
///
/// Pairs a [`SubscriptionSink`] with a [`BackpressurePolicy`] and a set of
/// [`SubscriptionCounters`]. Events are pushed via [`Self::emit_deposit`].
#[must_use]
pub struct PheromoneSubscription {
    id: SubscriptionId,
    sink: Arc<dyn SubscriptionSink<PheromoneEvent>>,
    policy: BackpressurePolicy,
    counters: SubscriptionCounters,
}

impl PheromoneSubscription {
    /// Constructs a new subscription with the given sink + back-pressure policy.
    /// The caller typically obtains the `id` from [`PheromoneBus::register`].
    pub fn new(
        id: SubscriptionId,
        sink: Arc<dyn SubscriptionSink<PheromoneEvent>>,
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

    /// Returns a snapshot of current counters.
    #[must_use]
    pub fn stats(&self) -> SubscriptionStats {
        self.counters.snapshot()
    }

    /// Whether this subscription has been closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.counters.is_closed() || self.sink.is_closed()
    }

    /// Emits one pheromone event through the sink, applying the configured
    /// back-pressure policy on overflow.
    ///
    /// Returns `Ok(())` when the push was delivered (possibly at the cost of
    /// dropping older events), or `Err(SinkError)` when the policy refused
    /// to enqueue the event or the subscription closed.
    pub fn emit_deposit(&self, event: PheromoneEvent) -> Result<(), SinkError> {
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
                // Net effective deliveries = 0 (old out, new in). `delivered`
                // already reflects the evicted events, so only `dropped_oldest`
                // advances here.
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

impl std::fmt::Debug for PheromoneSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PheromoneSubscription")
            .field("id", &self.id)
            .field("policy", &self.policy)
            .field("stats", &self.stats())
            .finish()
    }
}

/// Central fan-out point for pheromone events.
///
/// Mirage's [`PheromoneField::deposit`](crate::chain::PheromoneField::deposit)
/// would call [`PheromoneBus::broadcast`] with a freshly built
/// [`PheromoneEvent`]. Each registered subscription receives a copy through
/// its own sink.
#[must_use]
pub struct PheromoneBus {
    subs: RwLock<HashMap<SubscriptionId, PheromoneSubscription>>,
    next_id: RwLock<u64>,
    next_event_id: RwLock<u64>,
}

impl PheromoneBus {
    /// Constructs an empty bus.
    pub fn new() -> Self {
        Self {
            subs: RwLock::new(HashMap::new()),
            next_id: RwLock::new(1),
            next_event_id: RwLock::new(1),
        }
    }

    /// Registers a new subscription backed by `sink` and using `policy`.
    /// Returns the assigned [`SubscriptionId`].
    pub fn register(
        &self,
        sink: Arc<dyn SubscriptionSink<PheromoneEvent>>,
        policy: BackpressurePolicy,
    ) -> SubscriptionId {
        let id = {
            let mut n = self.next_id.write();
            let id = SubscriptionId(*n);
            *n = n.saturating_add(1);
            id
        };
        let sub = PheromoneSubscription::new(id, sink, policy);
        self.subs.write().insert(id, sub);
        id
    }

    /// Unregisters the subscription with the given id. Returns true if a
    /// subscription was removed.
    pub fn unregister(&self, id: SubscriptionId) -> bool {
        self.subs.write().remove(&id).is_some()
    }

    /// Number of currently active subscriptions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.subs.read().len()
    }

    /// Whether the bus has no subscriptions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.subs.read().is_empty()
    }

    /// Emits a deposit event to every registered subscription.
    ///
    /// Builds a fresh event id from the bus's monotonic counter, then calls
    /// [`PheromoneSubscription::emit_deposit`] on each subscription. Returns
    /// the number of subscriptions the event was delivered to (including
    /// drop-oldest deliveries that evicted older events).
    pub fn broadcast(
        &self,
        kind: PheromoneKind,
        vector: HdcVector,
        intensity: f32,
        deposited_at: u64,
    ) -> usize {
        let event_id = {
            let mut n = self.next_event_id.write();
            let id = *n;
            *n = n.saturating_add(1);
            id
        };
        let template = PheromoneEvent::new(event_id, kind, vector, intensity, deposited_at);
        self.broadcast_event(template)
    }

    /// Emits a pre-built event to every registered subscription. Useful for
    /// tests and when replaying historical events.
    pub fn broadcast_event(&self, event: PheromoneEvent) -> usize {
        let subs = self.subs.read();
        let mut delivered = 0usize;
        for sub in subs.values() {
            if sub.emit_deposit(event.clone()).is_ok() {
                delivered += 1;
            }
        }
        delivered
    }

    /// Returns the stats snapshot for a given subscription.
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

    /// Sweeps closed subscriptions from the bus. Returns the number removed.
    pub fn prune_closed(&self) -> usize {
        let mut subs = self.subs.write();
        let before = subs.len();
        subs.retain(|_, s| !s.is_closed());
        before - subs.len()
    }
}

impl Default for PheromoneBus {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PheromoneBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PheromoneBus")
            .field("subscriptions", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roko_bridge::subscription::sink::{BroadcastSink, MpscSink, VecSink};

    fn sample_event(id: u64) -> PheromoneEvent {
        PheromoneEvent::new(
            id,
            PheromoneKind::Threat,
            HdcVector::from_seed(b"rug pull alpha"),
            1.0,
            1_700_000_000,
        )
    }

    #[test]
    fn vec_sink_subscription_records_events() {
        let sink: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let sub = PheromoneSubscription::new(
            SubscriptionId(1),
            sink.clone(),
            BackpressurePolicy::DropOldest,
        );
        for i in 0..3 {
            sub.emit_deposit(sample_event(i)).unwrap();
        }
        assert_eq!(sink.len(), 3);
        assert_eq!(sub.stats().delivered, 3);
        assert_eq!(sub.subscription_id(), SubscriptionId(1));
    }

    #[test]
    fn bus_register_and_broadcast_reaches_all_subs() {
        let bus = PheromoneBus::new();
        let sink_a: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let sink_b: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let _id_a = bus.register(sink_a.clone(), BackpressurePolicy::DropOldest);
        let _id_b = bus.register(sink_b.clone(), BackpressurePolicy::DropOldest);
        assert_eq!(bus.len(), 2);

        let delivered = bus.broadcast(
            PheromoneKind::Wisdom,
            HdcVector::from_seed(b"bridge trick"),
            0.9,
            1_700_000_100,
        );
        assert_eq!(delivered, 2);
        assert_eq!(sink_a.len(), 1);
        assert_eq!(sink_b.len(), 1);
    }

    #[test]
    fn unregister_silences_sub() {
        let bus = PheromoneBus::new();
        let sink_a: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let sink_b: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let id_a = bus.register(sink_a.clone(), BackpressurePolicy::DropOldest);
        let _id_b = bus.register(sink_b.clone(), BackpressurePolicy::DropOldest);

        assert!(bus.unregister(id_a));
        assert!(!bus.unregister(id_a));
        assert_eq!(bus.len(), 1);

        bus.broadcast(
            PheromoneKind::Opportunity,
            HdcVector::from_seed(b"fresh mev"),
            0.5,
            1_700_000_200,
        );
        assert_eq!(sink_a.len(), 0, "unregistered sink must be silent");
        assert_eq!(sink_b.len(), 1);
    }

    #[tokio::test]
    async fn drop_oldest_counts_evictions_on_broadcast_sink() {
        // broadcast capacity is next-power-of-two >= requested; force 4 then
        // push 150 items.
        let (sink, mut rx) = BroadcastSink::<PheromoneEvent>::new(4);
        let sub = PheromoneSubscription::new(
            SubscriptionId(42),
            Arc::new(sink),
            BackpressurePolicy::DropOldest,
        );
        // Drain-as-we-go isn't the target — we want the sink to LAG.
        for i in 0..16 {
            sub.emit_deposit(sample_event(i)).unwrap();
        }
        // Now attempt to drain the receiver; we expect a `Lagged` report.
        let mut saw_lag = false;
        loop {
            match rx.try_recv() {
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    saw_lag = true;
                }
                Err(_) => break,
            }
        }
        // Broadcast sink never reports Full (send returns Ok as long as >=1
        // receiver), so delivered counts every push and no drops are recorded
        // at the subscription layer — the lag is observed only at recv.
        assert_eq!(sub.stats().delivered, 16);
        assert_eq!(sub.stats().dropped_oldest, 0);
        // Receiver observed lag.
        assert!(saw_lag, "broadcast receiver should have observed lag");
    }

    #[tokio::test]
    async fn drop_newest_refuses_over_mpsc_full() {
        let (sink, _rx) = MpscSink::<PheromoneEvent>::new(3);
        let sub = PheromoneSubscription::new(
            SubscriptionId(7),
            Arc::new(sink),
            BackpressurePolicy::DropNewest,
        );
        for i in 0..3 {
            sub.emit_deposit(sample_event(i)).unwrap();
        }
        // Next one is refused.
        let err = sub.emit_deposit(sample_event(99)).unwrap_err();
        assert_eq!(err, SinkError::Full { dropped: 1 });
        assert_eq!(sub.stats().delivered, 3);
        assert_eq!(sub.stats().dropped_newest, 1);
    }

    #[tokio::test]
    async fn close_on_overflow_latches_closed() {
        let (sink, _rx) = MpscSink::<PheromoneEvent>::new(2);
        let sub = PheromoneSubscription::new(
            SubscriptionId(3),
            Arc::new(sink),
            BackpressurePolicy::CloseOnOverflow,
        );
        sub.emit_deposit(sample_event(0)).unwrap();
        sub.emit_deposit(sample_event(1)).unwrap();
        // Overflow closes the subscription.
        let err = sub.emit_deposit(sample_event(2)).unwrap_err();
        assert_eq!(err, SinkError::Closed);
        assert!(sub.is_closed());
        // Subsequent pushes still closed.
        let err = sub.emit_deposit(sample_event(3)).unwrap_err();
        assert_eq!(err, SinkError::Closed);
        assert_eq!(sub.stats().delivered, 2);
        assert!(sub.stats().closed);
    }

    #[test]
    fn prune_closed_drops_closed_subs() {
        let bus = PheromoneBus::new();
        let sink_a: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let sink_b: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let _id_a = bus.register(sink_a.clone(), BackpressurePolicy::DropOldest);
        let _id_b = bus.register(sink_b.clone(), BackpressurePolicy::DropOldest);

        sink_a.close();
        // One broadcast to discover the close.
        bus.broadcast(PheromoneKind::Threat, HdcVector::from_seed(b"x"), 1.0, 10);
        let removed = bus.prune_closed();
        assert_eq!(removed, 1);
        assert_eq!(bus.len(), 1);
    }

    /// A ring-buffer sink that accepts every push but evicts the oldest
    /// event once capacity is exceeded. Used to exercise DropOldest policy
    /// with deterministic counters.
    struct RingSink {
        cap: usize,
        buf: parking_lot::Mutex<std::collections::VecDeque<PheromoneEvent>>,
    }
    impl RingSink {
        fn new(cap: usize) -> Self {
            Self {
                cap,
                buf: parking_lot::Mutex::new(std::collections::VecDeque::with_capacity(cap)),
            }
        }
        fn len(&self) -> usize {
            self.buf.lock().len()
        }
    }
    impl SubscriptionSink<PheromoneEvent> for RingSink {
        fn push(&self, event: PheromoneEvent) -> Result<(), SinkError> {
            let mut buf = self.buf.lock();
            if buf.len() >= self.cap {
                let _ = buf.pop_front();
                buf.push_back(event);
                Err(SinkError::Full { dropped: 1 })
            } else {
                buf.push_back(event);
                Ok(())
            }
        }
        fn is_closed(&self) -> bool {
            false
        }
    }

    #[test]
    fn drop_oldest_100_capacity_150_pushes() {
        // §37.14 spec: 100-capacity ring, 150 pushes, delivered == 100, dropped == 50.
        // Ring sink emulates the eviction model: first 100 fill, next 50 each evict one.
        let ring = Arc::new(RingSink::new(100));
        let sub = PheromoneSubscription::new(
            SubscriptionId(11),
            ring.clone(),
            BackpressurePolicy::DropOldest,
        );
        for i in 0..150u64 {
            sub.emit_deposit(sample_event(i)).unwrap();
        }
        let stats = sub.stats();
        // Net effective delivery: 100 in buffer, 50 evicted. `delivered`
        // counts first 100 successful pushes; the next 50 report Full{dropped:1}
        // and DropOldest policy increments only `dropped_oldest`.
        assert_eq!(stats.delivered, 100);
        assert_eq!(stats.dropped_oldest, 50);
        assert_eq!(stats.dropped_newest, 0);
        assert_eq!(ring.len(), 100);
    }

    #[tokio::test]
    async fn drop_newest_100_capacity_150_pushes() {
        // §37.14 spec: 100-capacity sink, 150 pushes, delivered == 100, dropped == 50.
        // MpscSink has hard capacity; DropNewest policy refuses the 101st..150th.
        let (sink, _rx) = MpscSink::<PheromoneEvent>::new(100);
        let sub = PheromoneSubscription::new(
            SubscriptionId(1),
            Arc::new(sink),
            BackpressurePolicy::DropNewest,
        );
        for i in 0..150u64 {
            let _ = sub.emit_deposit(sample_event(i));
        }
        let stats = sub.stats();
        assert_eq!(stats.delivered, 100);
        assert_eq!(stats.dropped_newest, 50);
        assert_eq!(stats.dropped_oldest, 0);
    }

    #[tokio::test]
    async fn drop_newest_keeps_first_100() {
        // §37.14 spec: DropNewest should keep the FIRST 100 events, not the last.
        let (sink, mut rx) = MpscSink::<PheromoneEvent>::new(100);
        let sub = PheromoneSubscription::new(
            SubscriptionId(2),
            Arc::new(sink),
            BackpressurePolicy::DropNewest,
        );
        for i in 0..150u64 {
            let _ = sub.emit_deposit(sample_event(i));
        }
        // Drain and verify the first 100 ids survived.
        let mut seen_ids = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            seen_ids.push(ev.id);
        }
        assert_eq!(seen_ids.len(), 100);
        assert_eq!(seen_ids[0], 0);
        assert_eq!(*seen_ids.last().unwrap(), 99);
    }

    #[tokio::test]
    async fn close_on_overflow_100_capacity_fails_at_101() {
        // §37.14 spec: 100-cap sink, push 101st -> subscription closes, further pushes fail.
        let (sink, _rx) = MpscSink::<PheromoneEvent>::new(100);
        let sub = PheromoneSubscription::new(
            SubscriptionId(3),
            Arc::new(sink),
            BackpressurePolicy::CloseOnOverflow,
        );
        for i in 0..100u64 {
            sub.emit_deposit(sample_event(i)).unwrap();
        }
        // 101st triggers close.
        let err = sub.emit_deposit(sample_event(100)).unwrap_err();
        assert_eq!(err, SinkError::Closed);
        assert!(sub.is_closed());
        // Further pushes also fail.
        let err = sub.emit_deposit(sample_event(101)).unwrap_err();
        assert_eq!(err, SinkError::Closed);
        let stats = sub.stats();
        assert_eq!(stats.delivered, 100);
        assert!(stats.closed);
    }

    #[test]
    fn all_stats_returns_each_sub() {
        let bus = PheromoneBus::new();
        let sink_a: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        let sink_b: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
        bus.register(sink_a, BackpressurePolicy::DropOldest);
        bus.register(sink_b, BackpressurePolicy::DropNewest);
        bus.broadcast(PheromoneKind::Threat, HdcVector::from_seed(b"q"), 1.0, 0);
        let stats = bus.all_stats();
        assert_eq!(stats.len(), 2);
        for (_, s) in stats {
            assert_eq!(s.delivered, 1);
        }
    }
}
