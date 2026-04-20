//! Additional [`Bus`] backend implementations.
//!
//! These complement the primary [`PulseBus`](crate::PulseBus) with
//! alternative transport semantics:
//!
//! - [`BroadcastBus`] — minimal in-process broadcast, no replay
//! - [`MemoryBus`] — in-memory with bounded replay ring
//! - [`MultiBus`] — fan-out to multiple backends simultaneously
//!
//! All implement the [`Bus`] trait from [`crate::traits`].

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use crate::{Bus, Pulse, TopicFilter, error::Result};

// ─── BroadcastBus ────────────────────────────────────────────────────────

/// A minimal in-process broadcast bus with no replay support.
///
/// Each subscriber receives all pulses published after subscription.
/// No history is retained -- if a subscriber is slow, pulses are dropped.
/// Useful for real-time event forwarding where history is not needed.
pub struct BroadcastBus {
    /// Monotonic sequence counter.
    seq: AtomicU64,
    /// Active subscriber channels.
    subscribers: RwLock<Vec<BroadcastSubscriber>>,
}

struct BroadcastSubscriber {
    filter: TopicFilter,
    tx: tokio::sync::mpsc::UnboundedSender<Pulse>,
}

/// Receiver for [`BroadcastBus`].
pub struct BroadcastBusReceiver {
    rx: tokio::sync::mpsc::UnboundedReceiver<Pulse>,
}

impl BroadcastBusReceiver {
    /// Receive the next matching pulse. Returns `None` if the bus is closed.
    pub async fn recv(&mut self) -> Option<Pulse> {
        self.rx.recv().await
    }
}

impl BroadcastBus {
    /// Create a new broadcast bus.
    #[must_use]
    pub fn new() -> Self {
        Self {
            seq: AtomicU64::new(0),
            subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Current count of active subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        let subs = self.subscribers.read();
        subs.iter().filter(|s| !s.tx.is_closed()).count()
    }
}

impl Default for BroadcastBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for BroadcastBus {
    type Receiver = BroadcastBusReceiver;

    fn publish(&self, pulse: Pulse) -> Result<u64> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);

        let subs = self.subscribers.read();
        for sub in subs.iter() {
            if sub.filter.matches(&pulse.topic) {
                // Best-effort: if the receiver is dropped, we skip it.
                let _ = sub.tx.send(pulse.clone());
            }
        }

        Ok(seq)
    }

    fn subscribe(&self, filter: TopicFilter) -> Result<BroadcastBusReceiver> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subs = self.subscribers.write();
        // Clean up closed subscribers while we're here.
        subs.retain(|s| !s.tx.is_closed());
        subs.push(BroadcastSubscriber { filter, tx });
        Ok(BroadcastBusReceiver { rx })
    }
}

// ─── MemoryBus ───────────────────────────────────────────────────────────

/// An in-memory bus with bounded replay ring.
///
/// Unlike [`BroadcastBus`], this retains the last N pulses in a ring buffer.
/// New subscribers can replay recent history before receiving live events.
/// Useful for testing, debugging, and scenarios where late joiners need catchup.
pub struct MemoryBus {
    /// Monotonic sequence counter.
    seq: AtomicU64,
    /// Replay ring buffer (bounded).
    ring: Mutex<VecDeque<(u64, Pulse)>>,
    /// Maximum ring capacity.
    capacity: usize,
    /// Active subscriber channels.
    subscribers: RwLock<Vec<BroadcastSubscriber>>,
}

/// Receiver for [`MemoryBus`].
pub struct MemoryBusReceiver {
    rx: tokio::sync::mpsc::UnboundedReceiver<Pulse>,
}

impl MemoryBusReceiver {
    /// Receive the next matching pulse. Returns `None` if the bus is closed.
    pub async fn recv(&mut self) -> Option<Pulse> {
        self.rx.recv().await
    }
}

impl MemoryBus {
    /// Create a new memory bus with the given replay ring capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            seq: AtomicU64::new(0),
            ring: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Replay all pulses with sequence >= `after_seq`, optionally filtered.
    #[must_use]
    pub fn replay_from(&self, after_seq: u64, filter: Option<&TopicFilter>) -> Vec<Pulse> {
        let ring = self.ring.lock();
        ring.iter()
            .filter(|(seq, _)| *seq >= after_seq)
            .filter(|(_, pulse)| filter.map_or(true, |f| f.matches(&pulse.topic)))
            .map(|(_, pulse)| pulse.clone())
            .collect()
    }

    /// Total pulses ever published.
    #[must_use]
    pub fn total_published(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }

    /// Current ring occupancy.
    #[must_use]
    pub fn ring_len(&self) -> usize {
        self.ring.lock().len()
    }
}

impl Bus for MemoryBus {
    type Receiver = MemoryBusReceiver;

    fn publish(&self, pulse: Pulse) -> Result<u64> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);

        // Store in replay ring.
        {
            let mut ring = self.ring.lock();
            if ring.len() >= self.capacity {
                ring.pop_front();
            }
            ring.push_back((seq, pulse.clone()));
        }

        // Fan out to live subscribers.
        let subs = self.subscribers.read();
        for sub in subs.iter() {
            if sub.filter.matches(&pulse.topic) {
                let _ = sub.tx.send(pulse.clone());
            }
        }

        Ok(seq)
    }

    fn subscribe(&self, filter: TopicFilter) -> Result<MemoryBusReceiver> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subs = self.subscribers.write();
        subs.retain(|s| !s.tx.is_closed());
        subs.push(BroadcastSubscriber { filter, tx });
        Ok(MemoryBusReceiver { rx })
    }
}

// ─── MultiBus ────────────────────────────────────────────────────────────

/// A fan-out bus that publishes to multiple inner buses simultaneously.
///
/// Useful for sending pulses to both a local memory bus and a persistent bus,
/// or for bridging local and remote transports.
///
/// Subscribes are served from the first inner bus (the "primary").
pub struct MultiBus {
    /// The primary bus (used for subscriptions).
    primary: Arc<MemoryBus>,
    /// Additional buses that receive published pulses.
    secondaries: Vec<Arc<dyn BusErased>>,
}

/// Type-erased bus interface for MultiBus secondaries.
///
/// This trait is needed because the `Bus` trait has an associated type `Receiver`
/// that prevents object-safety. `BusErased` drops the subscribe method and only
/// exposes publish.
pub trait BusErased: Send + Sync {
    /// Publish a pulse to this bus. Returns the sequence number.
    fn publish_erased(&self, pulse: Pulse) -> Result<u64>;
}

impl<T: Bus> BusErased for T
where
    T: Send + Sync,
{
    fn publish_erased(&self, pulse: Pulse) -> Result<u64> {
        self.publish(pulse)
    }
}

impl MultiBus {
    /// Create a multi-bus with a primary memory bus and no secondaries.
    #[must_use]
    pub fn new(primary_capacity: usize) -> Self {
        Self {
            primary: Arc::new(MemoryBus::new(primary_capacity)),
            secondaries: Vec::new(),
        }
    }

    /// Add a secondary bus that receives all published pulses.
    pub fn add_secondary(&mut self, bus: Arc<dyn BusErased>) {
        self.secondaries.push(bus);
    }

    /// Number of secondary buses.
    #[must_use]
    pub fn secondary_count(&self) -> usize {
        self.secondaries.len()
    }

    /// Access the primary bus directly (e.g., for replay).
    #[must_use]
    pub fn primary(&self) -> &MemoryBus {
        &self.primary
    }
}

impl Bus for MultiBus {
    type Receiver = MemoryBusReceiver;

    fn publish(&self, pulse: Pulse) -> Result<u64> {
        let seq = self.primary.publish(pulse.clone())?;

        // Fan out to secondaries. Errors on secondaries are logged but don't
        // fail the primary publish.
        for secondary in &self.secondaries {
            if let Err(e) = secondary.publish_erased(pulse.clone()) {
                tracing::warn!("MultiBus secondary publish failed: {e}");
            }
        }

        Ok(seq)
    }

    fn subscribe(&self, filter: TopicFilter) -> Result<MemoryBusReceiver> {
        self.primary.subscribe(filter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Body, Kind, Topic};

    fn test_pulse(topic: &str) -> Pulse {
        Pulse::new(0, Topic::new(topic), Kind::Metric, Body::text("data"))
    }

    // ─── BroadcastBus tests ──────────────────────────────────────────

    #[tokio::test]
    async fn broadcast_bus_publish_and_receive() {
        let bus = BroadcastBus::new();
        let mut rx = bus.subscribe(TopicFilter::All).unwrap();

        bus.publish(test_pulse("a")).unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.topic, Topic::new("a"));
    }

    #[tokio::test]
    async fn broadcast_bus_filter() {
        let bus = BroadcastBus::new();
        let mut rx = bus.subscribe(TopicFilter::Prefix("gate.".into())).unwrap();

        bus.publish(test_pulse("agent.turn")).unwrap();
        bus.publish(test_pulse("gate.compile")).unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.topic, Topic::new("gate.compile"));
    }

    #[test]
    fn broadcast_bus_seq_monotonic() {
        let bus = BroadcastBus::new();
        let s1 = bus.publish(test_pulse("a")).unwrap();
        let s2 = bus.publish(test_pulse("b")).unwrap();
        assert_eq!(s1, 0);
        assert_eq!(s2, 1);
    }

    #[test]
    fn broadcast_bus_subscriber_count() {
        let bus = BroadcastBus::new();
        assert_eq!(bus.subscriber_count(), 0);
        let _rx = bus.subscribe(TopicFilter::All).unwrap();
        assert_eq!(bus.subscriber_count(), 1);
    }

    // ─── MemoryBus tests ─────────────────────────────────────────────

    #[test]
    fn memory_bus_replay() {
        let bus = MemoryBus::new(16);
        bus.publish(test_pulse("a")).unwrap();
        bus.publish(test_pulse("b")).unwrap();
        bus.publish(test_pulse("c")).unwrap();

        let all = bus.replay_from(0, None);
        assert_eq!(all.len(), 3);

        let from_1 = bus.replay_from(1, None);
        assert_eq!(from_1.len(), 2);

        let filtered = bus.replay_from(0, Some(&TopicFilter::Exact(Topic::new("b"))));
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn memory_bus_ring_bounded() {
        let bus = MemoryBus::new(3);
        for i in 0..5 {
            bus.publish(test_pulse(&format!("t{}", i))).unwrap();
        }
        assert_eq!(bus.ring_len(), 3);
        assert_eq!(bus.total_published(), 5);

        // Ring should contain only the last 3 pulses.
        let all = bus.replay_from(0, None);
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn memory_bus_subscribe_and_receive() {
        let bus = MemoryBus::new(16);
        let mut rx = bus.subscribe(TopicFilter::All).unwrap();

        bus.publish(test_pulse("live")).unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.topic, Topic::new("live"));
    }

    // ─── MultiBus tests ──────────────────────────────────────────────

    #[test]
    fn multi_bus_fanout() {
        let mut multi = MultiBus::new(16);
        let secondary = Arc::new(MemoryBus::new(16));
        multi.add_secondary(secondary.clone());
        assert_eq!(multi.secondary_count(), 1);

        multi.publish(test_pulse("x")).unwrap();

        // Both primary and secondary should have the pulse.
        assert_eq!(multi.primary().ring_len(), 1);
        assert_eq!(secondary.ring_len(), 1);
    }

    #[tokio::test]
    async fn multi_bus_subscribe_from_primary() {
        let multi = MultiBus::new(16);
        let mut rx = multi.subscribe(TopicFilter::All).unwrap();

        multi.publish(test_pulse("m")).unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.topic, Topic::new("m"));
    }

    #[test]
    fn multi_bus_no_secondaries() {
        let multi = MultiBus::new(16);
        assert_eq!(multi.secondary_count(), 0);
        multi.publish(test_pulse("solo")).unwrap();
        assert_eq!(multi.primary().ring_len(), 1);
    }
}
