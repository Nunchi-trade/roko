//! Typed, bounded event bus for server events.
//!
//! This bus fans out live events to subscribers and keeps a small replay ring
//! so WebSocket clients can catch up after connecting.

use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::broadcast;
use tracing::trace;

/// A sequenced, timestamped envelope wrapping an event payload.
#[derive(Debug, Clone)]
pub struct Envelope<E> {
    /// Monotonically increasing sequence number.
    pub seq: u64,
    /// Unix timestamp in milliseconds when the event was published.
    pub ts_millis: u64,
    /// The wrapped event payload.
    pub payload: E,
}

/// Broadcast receiver for live server event envelopes.
pub type Receiver<E> = broadcast::Receiver<Envelope<E>>;

struct Shared<E> {
    tx: broadcast::Sender<Envelope<E>>,
    ring: Mutex<VecDeque<Envelope<E>>>,
    seq: AtomicU64,
    capacity: usize,
}

impl<E: Clone + Send + Sync + 'static> Shared<E> {
    fn publish_inner(&self, event: E) {
        let envelope = Envelope {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            ts_millis: current_ts_millis(),
            payload: event,
        };

        {
            let mut ring = self.ring.lock();
            if ring.len() >= self.capacity {
                ring.pop_front();
            }
            ring.push_back(envelope.clone());
        }

        trace!(seq = envelope.seq, "event published");
        let _ = self.tx.send(envelope);
    }
}

/// Shared event backbone for the HTTP server.
#[derive(Clone)]
pub struct EventBus<E: Clone + Send + Sync + 'static> {
    shared: Arc<Shared<E>>,
}

impl<E: Clone + Send + Sync + 'static> EventBus<E> {
    /// Create a new event bus with the given replay capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            shared: Arc::new(Shared {
                tx,
                ring: Mutex::new(VecDeque::with_capacity(capacity)),
                seq: AtomicU64::new(0),
                capacity,
            }),
        }
    }

    /// Publish an event to all live subscribers and record it for replay.
    pub fn publish(&self, event: E) {
        self.shared.publish_inner(event);
    }

    /// Subscribe to live events.
    pub fn subscribe(&self) -> Receiver<E> {
        self.shared.tx.subscribe()
    }

    /// Return a snapshot of events published after `after_seq`.
    pub fn replay_from(&self, after_seq: u64) -> Vec<Envelope<E>> {
        self.shared
            .ring
            .lock()
            .iter()
            .filter(|e| e.seq >= after_seq)
            .cloned()
            .collect()
    }
}

#[allow(clippy::cast_possible_truncation)]
fn current_ts_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestEvent {
        Ping(u32),
        Pong(String),
    }

    #[test]
    fn publish_and_replay() {
        let bus = EventBus::new(16);
        bus.publish(TestEvent::Ping(1));
        bus.publish(TestEvent::Pong("hello".into()));
        bus.publish(TestEvent::Ping(2));

        let all = bus.replay_from(0);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].seq, 0);
        assert_eq!(all[1].seq, 1);
        assert_eq!(all[2].seq, 2);

        let partial = bus.replay_from(2);
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].payload, TestEvent::Ping(2));
    }

    #[test]
    fn ring_eviction() {
        let bus = EventBus::new(3);
        bus.publish(TestEvent::Ping(1));
        bus.publish(TestEvent::Ping(2));
        bus.publish(TestEvent::Ping(3));
        bus.publish(TestEvent::Ping(4));

        let all = bus.replay_from(0);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].seq, 1);
    }

    #[tokio::test]
    async fn subscribe_receives_live_events() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.publish(TestEvent::Ping(42));

        let env = rx.recv().await.unwrap();
        assert_eq!(env.payload, TestEvent::Ping(42));
        assert_eq!(env.seq, 0);
        assert!(env.ts_millis > 0);
    }
}
