//! ISFRFeed — relay-to-bus bridge for ISFR rate data.
//!
//! Implements a simple adapter that receives topic messages from the relay and
//! republishes them as Pulses on the local PulseBus. This enables agents running
//! locally to react to ISFR rate updates published by the keeper via the relay.
//!
//! # Design
//!
//! ISFRFeed is a relay-to-bus bridge, not a Feed in the `FeedRegistry` sense.
//! The actual data transport is the Bus (PulseBus/BroadcastBus). When wired to
//! a relay client's TopicHandler interface (from the `roko-agent-server` A6 task),
//! it receives topic messages from the relay and publishes them as Pulses to the
//! local bus.
//!
//! # Usage
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use roko_core::bus_backends::BroadcastBus;
//! use roko_core::isfr_feed::ISFRFeed;
//!
//! let bus = Arc::new(BroadcastBus::new());
//! let feed = ISFRFeed::new(bus);
//!
//! // Pass Arc::new(feed) as the TopicHandler when connecting the relay client.
//! // Then subscribe to relay topics: "isfr:rates", "isfr:epochs", "chain:31337"
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Body;
use crate::Kind;
use crate::bus_backends::BusErased;
use crate::pulse::{Pulse, Topic};

/// Relay-to-bus bridge for ISFR data.
///
/// Receives topic messages from the relay (via `TopicHandler` interface in
/// `roko-agent-server`) and publishes them as Pulses on the local bus.
///
/// The adapter pattern is used at the integration point (CLI or serve) to avoid
/// coupling `roko-core` to `roko-agent-server`. Define the `TopicHandler` impl
/// that delegates to [`ISFRFeed::handle_message`] in the crate that performs
/// the wiring.
pub struct ISFRFeed {
    bus: Arc<dyn BusErased>,
    seq: AtomicU64,
    messages_received: AtomicU64,
    pulses_published: AtomicU64,
}

impl ISFRFeed {
    /// Create a new ISFRFeed that publishes to the given bus.
    pub fn new(bus: Arc<dyn BusErased>) -> Self {
        Self {
            bus,
            seq: AtomicU64::new(1),
            messages_received: AtomicU64::new(0),
            pulses_published: AtomicU64::new(0),
        }
    }

    /// Handle a topic message from the relay.
    ///
    /// Maps the relay topic to a local bus topic, constructs a Pulse, and
    /// publishes it. Call this from a `TopicHandler` implementation.
    ///
    /// # Arguments
    ///
    /// - `topic` — the relay topic name (e.g. `"isfr:rates"`, `"chain:31337"`)
    /// - `msg_type` — the message type field from the relay frame
    /// - `payload` — the JSON payload from the relay frame
    /// - `publisher_id` — optional relay publisher identifier
    /// - `_relay_seq` — relay sequence number (not forwarded; local seq is used)
    pub fn handle_message(
        &self,
        topic: &str,
        msg_type: &str,
        payload: serde_json::Value,
        publisher_id: Option<&str>,
        _relay_seq: u64,
    ) {
        self.messages_received.fetch_add(1, Ordering::Relaxed);

        // Map relay topic to local bus topic.
        let bus_topic = self.map_topic(topic);

        // Determine the Kind from msg_type.
        let kind = Kind::Custom(msg_type.to_string());

        // Build the pulse.
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut builder =
            Pulse::builder(seq, Topic::new(bus_topic), kind).body(Body::Json(payload));

        // Add metadata tags.
        builder = builder
            .tag("source", "relay")
            .tag("relay_topic", topic)
            .tag("msg_type", msg_type);

        if let Some(pub_id) = publisher_id {
            builder = builder.tag("publisher", pub_id);
        }

        let pulse = builder.build();

        // Publish to local bus.
        match self.bus.publish_erased(pulse) {
            Ok(_) => {
                self.pulses_published.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::warn!(error = %e, topic, "ISFRFeed: failed to publish pulse to bus");
            }
        }
    }

    /// Map relay topic names to local bus topic names.
    ///
    /// All known patterns return a `&'static str` — no heap allocation needed.
    fn map_topic(&self, relay_topic: &str) -> &'static str {
        match relay_topic {
            "isfr:rates" => "isfr.rates",
            "isfr:epochs" => "isfr.epochs",
            _ if relay_topic.starts_with("chain:") => "isfr.chain_event",
            _ => "isfr.unknown",
        }
    }

    /// Number of messages received from the relay.
    pub fn messages_received(&self) -> u64 {
        self.messages_received.load(Ordering::Relaxed)
    }

    /// Number of pulses successfully published to the local bus.
    pub fn pulses_published(&self) -> u64 {
        self.pulses_published.load(Ordering::Relaxed)
    }

    /// Topics this feed should subscribe to on the relay.
    ///
    /// Pass each returned topic to the relay client's subscribe method after
    /// connecting. The `chain_id` parameter specifies which chain to watch
    /// (e.g. `"31337"` for Anvil/Hardhat local, `"1"` for Ethereum mainnet).
    pub fn relay_topics(chain_id: &str) -> Vec<String> {
        vec![
            "isfr:rates".to_string(),
            "isfr:epochs".to_string(),
            format!("chain:{chain_id}"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus_backends::BroadcastBus;

    #[test]
    fn feed_publishes_to_bus() {
        let bus = Arc::new(BroadcastBus::new());
        let feed = ISFRFeed::new(bus.clone());

        feed.handle_message(
            "isfr:rates",
            "composite_rate",
            serde_json::json!({"bps": 620}),
            Some("keeper-1"),
            42,
        );

        assert_eq!(feed.messages_received(), 1);
        assert_eq!(feed.pulses_published(), 1);
    }

    #[test]
    fn feed_maps_topics() {
        let bus = Arc::new(BroadcastBus::new());
        let feed = ISFRFeed::new(bus);

        assert_eq!(feed.map_topic("isfr:rates"), "isfr.rates");
        assert_eq!(feed.map_topic("isfr:epochs"), "isfr.epochs");
        assert_eq!(feed.map_topic("chain:31337"), "isfr.chain_event");
        assert_eq!(feed.map_topic("chain:1"), "isfr.chain_event");
        assert_eq!(feed.map_topic("random"), "isfr.unknown");
    }

    #[test]
    fn relay_topics_includes_chain() {
        let topics = ISFRFeed::relay_topics("31337");
        assert_eq!(topics.len(), 3);
        assert!(topics.contains(&"isfr:rates".to_string()));
        assert!(topics.contains(&"isfr:epochs".to_string()));
        assert!(topics.contains(&"chain:31337".to_string()));
    }

    #[test]
    fn relay_topics_uses_chain_id() {
        let topics = ISFRFeed::relay_topics("1");
        assert!(topics.contains(&"chain:1".to_string()));
        assert!(!topics.contains(&"chain:31337".to_string()));
    }

    #[test]
    fn feed_tracks_counters_independently() {
        let bus = Arc::new(BroadcastBus::new());
        let feed = ISFRFeed::new(bus);

        assert_eq!(feed.messages_received(), 0);
        assert_eq!(feed.pulses_published(), 0);

        feed.handle_message(
            "isfr:rates",
            "composite_rate",
            serde_json::json!({"composite_bps": 580}),
            None,
            1,
        );
        feed.handle_message(
            "isfr:epochs",
            "epoch_advance",
            serde_json::json!({"epoch": 7}),
            Some("keeper-1"),
            2,
        );

        assert_eq!(feed.messages_received(), 2);
        assert_eq!(feed.pulses_published(), 2);
    }

    #[test]
    fn feed_tags_pulse_with_source_metadata() {
        // Use a MemoryBus so we can inspect the published pulse.
        use crate::TopicFilter;
        use crate::bus_backends::MemoryBus;

        let bus = Arc::new(MemoryBus::new(16));
        let feed = ISFRFeed::new(bus.clone());

        feed.handle_message(
            "isfr:rates",
            "composite_rate",
            serde_json::json!({"bps": 750}),
            Some("keeper-42"),
            99,
        );

        let pulses = bus.replay_from(0, None);
        assert_eq!(pulses.len(), 1);
        let pulse = &pulses[0];

        assert_eq!(pulse.topic, Topic::new("isfr.rates"));
        assert_eq!(pulse.tag("source"), Some("relay"));
        assert_eq!(pulse.tag("relay_topic"), Some("isfr:rates"));
        assert_eq!(pulse.tag("msg_type"), Some("composite_rate"));
        assert_eq!(pulse.tag("publisher"), Some("keeper-42"));

        // Verify body is JSON.
        let _ = TopicFilter::All; // just ensuring import works
        match &pulse.body {
            Body::Json(v) => assert_eq!(v["bps"], 750),
            other => panic!("expected JSON body, got {}", other.kind_hint()),
        }
    }

    #[test]
    fn feed_no_publisher_tag_when_absent() {
        use crate::bus_backends::MemoryBus;

        let bus = Arc::new(MemoryBus::new(16));
        let feed = ISFRFeed::new(bus.clone());

        feed.handle_message(
            "isfr:rates",
            "rate_update",
            serde_json::json!({}),
            None, // no publisher
            1,
        );

        let pulses = bus.replay_from(0, None);
        assert_eq!(pulses.len(), 1);
        // No "publisher" tag should be set.
        assert_eq!(pulses[0].tag("publisher"), None);
    }

    #[test]
    fn feed_chain_event_mapping() {
        use crate::bus_backends::MemoryBus;

        let bus = Arc::new(MemoryBus::new(16));
        let feed = ISFRFeed::new(bus.clone());

        feed.handle_message(
            "chain:31337",
            "block_produced",
            serde_json::json!({"block": 100}),
            None,
            5,
        );

        let pulses = bus.replay_from(0, None);
        assert_eq!(pulses.len(), 1);
        assert_eq!(pulses[0].topic, Topic::new("isfr.chain_event"));
        assert_eq!(pulses[0].tag("relay_topic"), Some("chain:31337"));
    }
}
