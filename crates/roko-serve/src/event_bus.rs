//! Typed, bounded event bus for server events.
//!
//! This module wraps [`roko_runtime::event_bus::EventBus`] to provide the
//! `publish()` API that serve routes expect. The underlying broadcast,
//! replay ring, and sequence numbering are all delegated to the runtime
//! implementation -- there is no second ring or broadcast channel here.
//!
//! # History
//!
//! Before Task 104 this module contained a full duplicate of the runtime
//! event bus. The consolidation keeps the serve-facing `publish()` name
//! while eliminating the second implementation.

// Re-export the Envelope type so existing serve code that imports
// `crate::event_bus::Envelope` keeps compiling.
pub use roko_runtime::event_bus::Envelope;

/// Broadcast receiver for live server event envelopes.
pub type Receiver<E> = tokio::sync::broadcast::Receiver<Envelope<E>>;

/// Thin wrapper around [`roko_runtime::event_bus::EventBus`] that exposes a
/// `publish()` method (the name used throughout roko-serve) while delegating
/// to the runtime's `emit()`.
#[derive(Clone)]
pub struct EventBus<E: Clone + Send + Sync + 'static> {
    inner: roko_runtime::event_bus::EventBus<E>,
}

impl<E: Clone + Send + Sync + 'static> EventBus<E> {
    /// Create a new event bus with the given replay capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: roko_runtime::event_bus::EventBus::new(capacity),
        }
    }

    /// Publish an event to all live subscribers and record it for replay.
    pub fn publish(&self, event: E) {
        self.inner.emit(event);
    }

    /// Subscribe to live events.
    pub fn subscribe(&self) -> Receiver<E> {
        self.inner.subscribe()
    }

    /// Return a snapshot of events published after `after_seq`.
    pub fn replay_from(&self, after_seq: u64) -> Vec<Envelope<E>> {
        self.inner.replay_from(after_seq)
    }
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

    /// The SSE handler caps replay at 256 events via `.take(SSE_REPLAY_CAP)`.
    /// This test proves that even when the ring holds more than 256 events,
    /// the take(256) bound correctly limits the replay slice sent to clients.
    #[test]
    fn sse_replay_cap_bounds_at_256() {
        const SSE_REPLAY_CAP: usize = 256;

        let bus = EventBus::new(512);
        for i in 0..400u32 {
            bus.publish(TestEvent::Ping(i));
        }

        let all = bus.replay_from(0);
        assert_eq!(all.len(), 400);

        let capped: Vec<_> = bus
            .replay_from(0)
            .into_iter()
            .take(SSE_REPLAY_CAP)
            .collect();
        assert_eq!(capped.len(), SSE_REPLAY_CAP);
        assert_eq!(capped[0].seq, 0);
        assert_eq!(capped[SSE_REPLAY_CAP - 1].seq, 255);
    }

    #[test]
    fn sse_replay_cap_passes_through_when_under_limit() {
        const SSE_REPLAY_CAP: usize = 256;

        let bus = EventBus::new(512);
        for i in 0..100u32 {
            bus.publish(TestEvent::Ping(i));
        }

        let capped: Vec<_> = bus
            .replay_from(0)
            .into_iter()
            .take(SSE_REPLAY_CAP)
            .collect();
        assert_eq!(capped.len(), 100);
    }

    #[test]
    fn sse_replay_cap_with_last_event_id() {
        const SSE_REPLAY_CAP: usize = 256;

        let bus = EventBus::new(512);
        for i in 0..500u32 {
            bus.publish(TestEvent::Ping(i));
        }

        let from_100: Vec<_> = bus
            .replay_from(100)
            .into_iter()
            .take(SSE_REPLAY_CAP)
            .collect();

        assert_eq!(from_100.len(), SSE_REPLAY_CAP);
        assert_eq!(from_100[0].seq, 100);
        assert_eq!(from_100[SSE_REPLAY_CAP - 1].seq, 355);
    }

    #[test]
    fn sse_replay_cap_with_future_seq_returns_empty() {
        const SSE_REPLAY_CAP: usize = 256;

        let bus = EventBus::new(512);
        for i in 0..10u32 {
            bus.publish(TestEvent::Ping(i));
        }

        let replay: Vec<_> = bus
            .replay_from(9999)
            .into_iter()
            .take(SSE_REPLAY_CAP)
            .collect();
        assert!(replay.is_empty());
    }

    /// Wrapper delegates to the runtime implementation, so the same
    /// event should be visible through both the wrapper and the
    /// underlying runtime bus when constructed from the same inner.
    #[test]
    fn wrapper_delegates_to_runtime_bus() {
        let bus = EventBus::new(16);
        bus.publish(TestEvent::Ping(99));

        let events = bus.replay_from(0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload, TestEvent::Ping(99));
    }
}
