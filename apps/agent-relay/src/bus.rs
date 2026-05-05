//! Topic-based pub/sub bus for the agent relay.
//!
//! Agents subscribe to topics via WebSocket frames. Published messages
//! are fanned out to all subscribers of the matching topic.

use crate::protocol::TopicEnvelope;
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

/// Configuration for the topic bus.
pub struct TopicBusConfig {
    /// Max messages retained per topic for replay.
    pub ring_capacity: usize,
}

impl Default for TopicBusConfig {
    fn default() -> Self {
        Self { ring_capacity: 128 }
    }
}

/// Topic-based pub/sub bus.
///
/// Thread-safe. Designed to be wrapped in `Arc` and shared with handler tasks.
pub struct TopicBus {
    /// topic → set of subscribed agent_ids.
    subscriptions: RwLock<HashMap<String, Vec<String>>>,
    /// topic → ring buffer of recent envelopes.
    rings: RwLock<HashMap<String, VecDeque<TopicEnvelope>>>,
    /// Monotonically increasing sequence counter.
    seq: AtomicU64,
    /// Max messages per topic ring.
    ring_capacity: usize,
}

impl TopicBus {
    /// Create a new `TopicBus` with the given configuration.
    pub fn new(config: TopicBusConfig) -> Self {
        Self {
            subscriptions: RwLock::new(HashMap::new()),
            rings: RwLock::new(HashMap::new()),
            seq: AtomicU64::new(1),
            ring_capacity: config.ring_capacity,
        }
    }

    /// Subscribe an agent to a topic. Returns recent messages for replay.
    pub fn subscribe(&self, agent_id: &str, topic: &str) -> Vec<TopicEnvelope> {
        // Add to subscription set.
        {
            let mut subs = self.subscriptions.write();
            let agents = subs.entry(topic.to_string()).or_default();
            if !agents.contains(&agent_id.to_string()) {
                agents.push(agent_id.to_string());
            }
        }

        // Return ring contents for replay.
        let rings = self.rings.read();
        rings
            .get(topic)
            .map(|ring| ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Unsubscribe an agent from a topic.
    pub fn unsubscribe(&self, agent_id: &str, topic: &str) {
        let mut subs = self.subscriptions.write();
        if let Some(agents) = subs.get_mut(topic) {
            agents.retain(|id| id != agent_id);
            if agents.is_empty() {
                subs.remove(topic);
            }
        }
    }

    /// Remove all subscriptions for an agent (called on disconnect).
    pub fn unsubscribe_all(&self, agent_id: &str) {
        let mut subs = self.subscriptions.write();
        subs.retain(|_topic, agents| {
            agents.retain(|id| id != agent_id);
            !agents.is_empty()
        });
    }

    /// Publish a message to a topic.
    ///
    /// Assigns a monotonically increasing sequence number to the envelope,
    /// stores it in the ring buffer, and returns `(seq, subscriber_ids)`.
    /// The caller is responsible for actually delivering the frame to each agent.
    pub fn publish(&self, mut envelope: TopicEnvelope) -> (u64, Vec<String>) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        envelope.seq = seq;

        // Store in ring buffer.
        {
            let mut rings = self.rings.write();
            let ring = rings
                .entry(envelope.topic.clone())
                .or_insert_with(|| VecDeque::with_capacity(self.ring_capacity));
            if ring.len() >= self.ring_capacity {
                ring.pop_front();
            }
            ring.push_back(envelope.clone());
        }

        // Collect subscribers.
        let subscribers = {
            let subs = self.subscriptions.read();
            subs.get(&envelope.topic).cloned().unwrap_or_default()
        };

        (seq, subscribers)
    }

    /// Read ring buffer contents for a topic without modifying subscriptions.
    ///
    /// Used by the HTTP metadata endpoint to inspect recent messages without
    /// creating a phantom subscription entry.
    pub fn peek_ring(&self, topic: &str) -> Vec<TopicEnvelope> {
        let rings = self.rings.read();
        rings
            .get(topic)
            .map(|ring| ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all topics with their subscriber counts.
    pub fn topic_stats(&self) -> Vec<(String, usize)> {
        let subs = self.subscriptions.read();
        subs.iter()
            .map(|(topic, agents)| (topic.clone(), agents.len()))
            .collect()
    }

    /// Get subscriber count for a specific topic.
    pub fn subscriber_count(&self, topic: &str) -> usize {
        let subs = self.subscriptions.read();
        subs.get(topic).map(Vec::len).unwrap_or(0)
    }

    /// Current sequence number (for diagnostics).
    pub fn current_seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_and_publish() {
        let bus = TopicBus::new(TopicBusConfig::default());

        // Subscribe agent — no replay yet.
        let replay = bus.subscribe("agent-1", "isfr:rates");
        assert!(replay.is_empty());

        // Publish one message.
        let envelope =
            TopicEnvelope::new("isfr:rates", "rate_update", serde_json::json!({"bps": 620}));
        let (seq, subscribers) = bus.publish(envelope);
        assert_eq!(seq, 1);
        assert_eq!(subscribers, vec!["agent-1"]);

        // New subscriber gets replay of existing message.
        let replay = bus.subscribe("agent-2", "isfr:rates");
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].seq, 1);
    }

    #[test]
    fn unsubscribe_removes_from_fanout() {
        let bus = TopicBus::new(TopicBusConfig::default());
        bus.subscribe("agent-1", "chain:31337");
        bus.unsubscribe("agent-1", "chain:31337");

        let envelope = TopicEnvelope::new("chain:31337", "block", serde_json::json!({}));
        let (_seq, subscribers) = bus.publish(envelope);
        assert!(subscribers.is_empty());
    }

    #[test]
    fn ring_bounded() {
        let bus = TopicBus::new(TopicBusConfig { ring_capacity: 2 });
        for i in 0..5_u64 {
            let env = TopicEnvelope::new("t", "x", serde_json::json!(i));
            bus.publish(env);
        }
        let replay = bus.subscribe("a", "t");
        assert_eq!(replay.len(), 2);
        // Seq starts at 1, so after 5 publishes the retained ones are seq 4 and 5.
        assert_eq!(replay[0].seq, 4);
        assert_eq!(replay[1].seq, 5);
    }

    #[test]
    fn unsubscribe_all_on_disconnect() {
        let bus = TopicBus::new(TopicBusConfig::default());
        bus.subscribe("agent-1", "topic-a");
        bus.subscribe("agent-1", "topic-b");
        bus.unsubscribe_all("agent-1");
        assert_eq!(bus.subscriber_count("topic-a"), 0);
        assert_eq!(bus.subscriber_count("topic-b"), 0);
    }
}
