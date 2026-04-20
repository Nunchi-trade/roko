//! [`PulseBus`] — a [`Bus`] implementation backed by [`EventBus<Pulse>`].
//!
//! Wraps the generic [`EventBus`] from `roko-runtime` and implements the
//! [`Bus`] trait from `roko-core::traits`, bridging the typed event bus with
//! topic-filtered pulse subscriptions.
//!
//! # Architecture
//!
//! ```text
//!   Publisher ──publish()──► PulseBus ──broadcast──► FilteredReceiver₁
//!                              │                    FilteredReceiver₂
//!                              ▼
//!                         EventBus<Pulse> (replay ring)
//! ```
//!
//! The `PulseBus` publishes every pulse to the underlying `EventBus`. Each
//! subscriber gets a [`PulseBusReceiver`] that filters incoming pulses by
//! its [`TopicFilter`], so subscribers only see pulses matching their filter.

use crate::{Bus, Pulse, TopicFilter, error::Result};
use roko_runtime::event_bus::{Envelope, EventBus};
use std::sync::Arc;
use tokio::sync::broadcast;

/// A [`Bus`] implementation for [`Pulse`] traffic.
///
/// Internally wraps an `EventBus<Pulse>` from `roko-runtime`, adding
/// topic-based filtering on subscription.
pub struct PulseBus {
    inner: Arc<EventBus<Pulse>>,
}

impl PulseBus {
    /// Create a new `PulseBus` with the given replay ring capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(EventBus::new(capacity)),
        }
    }

    /// Returns the total number of pulses ever published.
    pub fn total_published(&self) -> u64 {
        self.inner.total_emitted()
    }

    /// Returns the current number of pulses in the replay ring.
    pub fn ring_len(&self) -> usize {
        self.inner.ring_len()
    }

    /// Replay pulses with `seq >= after_seq`, optionally filtered by topic.
    pub fn replay_from(&self, after_seq: u64, filter: Option<&TopicFilter>) -> Vec<Pulse> {
        self.inner
            .replay_from(after_seq)
            .into_iter()
            .filter(|env| filter.is_none_or(|f| f.matches(&env.payload.topic)))
            .map(|env| env.payload)
            .collect()
    }
}

impl Bus for PulseBus {
    type Receiver = PulseBusReceiver;

    fn publish(&self, pulse: Pulse) -> Result<u64> {
        let seq = self.inner.total_emitted();
        self.inner.emit(pulse);
        Ok(seq)
    }

    fn subscribe(&self, filter: TopicFilter) -> Result<PulseBusReceiver> {
        let rx = self.inner.subscribe();
        Ok(PulseBusReceiver { rx, filter })
    }
}

/// A filtered receiver for [`Pulse`]s from a [`PulseBus`].
///
/// Only yields pulses whose topic matches the [`TopicFilter`] provided
/// at subscription time.
pub struct PulseBusReceiver {
    rx: broadcast::Receiver<Envelope<Pulse>>,
    filter: TopicFilter,
}

impl PulseBusReceiver {
    /// Receive the next pulse matching the filter.
    ///
    /// Blocks until a matching pulse arrives. Pulses that don't match the
    /// filter are silently skipped. Returns `None` if the bus is closed.
    pub async fn recv(&mut self) -> Option<Pulse> {
        loop {
            match self.rx.recv().await {
                Ok(envelope) => {
                    if self.filter.matches(&envelope.payload.topic) {
                        return Some(envelope.payload);
                    }
                    // Skip non-matching pulses.
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "PulseBusReceiver lagged, skipped pulses");
                    // Continue receiving.
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    }

    /// Returns a reference to the topic filter for this receiver.
    pub fn filter(&self) -> &TopicFilter {
        &self.filter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Body, Kind, Topic};

    #[test]
    fn publish_and_replay() {
        let bus = PulseBus::new(16);

        let p1 = Pulse::new(
            0,
            Topic::new("gate.compile"),
            Kind::GateVerdict,
            Body::text("ok"),
        );
        let p2 = Pulse::new(
            0,
            Topic::new("agent.turn"),
            Kind::Episode,
            Body::text("turn 1"),
        );
        let p3 = Pulse::new(
            0,
            Topic::new("gate.test"),
            Kind::GateVerdict,
            Body::text("fail"),
        );

        bus.publish(p1).unwrap();
        bus.publish(p2).unwrap();
        bus.publish(p3).unwrap();

        assert_eq!(bus.total_published(), 3);
        assert_eq!(bus.ring_len(), 3);

        // Replay all.
        let all = bus.replay_from(0, None);
        assert_eq!(all.len(), 3);

        // Replay with filter.
        let gates = bus.replay_from(0, Some(&TopicFilter::Prefix("gate.".into())));
        assert_eq!(gates.len(), 2);
    }

    #[tokio::test]
    async fn subscribe_with_filter() {
        let bus = PulseBus::new(16);
        let mut rx = bus.subscribe(TopicFilter::Prefix("gate.".into())).unwrap();

        let p1 = Pulse::new(
            0,
            Topic::new("agent.turn"),
            Kind::Episode,
            Body::text("turn"),
        );
        let p2 = Pulse::new(
            0,
            Topic::new("gate.compile"),
            Kind::GateVerdict,
            Body::text("ok"),
        );
        bus.publish(p1).unwrap();
        bus.publish(p2).unwrap();

        // Should skip the agent.turn pulse and return the gate.compile pulse.
        let received = rx.recv().await.unwrap();
        assert_eq!(received.topic, Topic::new("gate.compile"));
    }

    #[tokio::test]
    async fn subscribe_all() {
        let bus = PulseBus::new(16);
        let mut rx = bus.subscribe(TopicFilter::All).unwrap();

        bus.publish(Pulse::new(0, Topic::new("a"), Kind::Task, Body::text("1")))
            .unwrap();
        bus.publish(Pulse::new(
            0,
            Topic::new("b"),
            Kind::Metric,
            Body::text("2"),
        ))
        .unwrap();

        let first = rx.recv().await.unwrap();
        let second = rx.recv().await.unwrap();
        assert_eq!(first.topic, Topic::new("a"));
        assert_eq!(second.topic, Topic::new("b"));
    }
}
