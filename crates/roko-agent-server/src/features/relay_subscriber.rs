//! Thin pub/sub wrapper around [`RelayHandle`].
//!
//! [`RelaySubscriber`] provides a higher-level interface over the raw
//! subscribe/unsubscribe/publish methods on [`RelayHandle`].  Callers can
//! bundle a handle + their topic subscriptions into a single value that is
//! easy to pass around without exposing the full relay-client API.
//!
//! Also provides [`ISFRTopicAdapter`], which adapts the relay [`TopicHandler`]
//! interface to [`roko_core::isfr_feed::ISFRFeed`] so ISFR relay messages are
//! automatically republished as Pulses on the local bus.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use roko_core::isfr_feed::ISFRFeed;
use tokio::sync::mpsc;

use super::relay_client::{RelayHandle, TopicHandler};

/// A received topic message.
#[derive(Debug, Clone)]
pub struct TopicMessage {
    /// The topic the message arrived on.
    pub topic: String,
    /// Application-defined message type discriminant.
    pub msg_type: String,
    /// Arbitrary JSON payload.
    pub payload: serde_json::Value,
    /// Agent ID of the publisher, if the relay provided one.
    pub publisher_id: Option<String>,
    /// Monotonically increasing sequence number assigned by the relay bus.
    pub seq: u64,
}

/// Receives topic messages through an mpsc channel and forwards them to a
/// caller-supplied [`mpsc::UnboundedSender<TopicMessage>`].
struct ChannelTopicHandler {
    tx: mpsc::UnboundedSender<TopicMessage>,
}

#[async_trait]
impl TopicHandler for ChannelTopicHandler {
    async fn on_topic_message(
        &self,
        topic: &str,
        msg_type: &str,
        payload: serde_json::Value,
        publisher_id: Option<&str>,
        seq: u64,
    ) {
        let msg = TopicMessage {
            topic: topic.to_owned(),
            msg_type: msg_type.to_owned(),
            payload,
            publisher_id: publisher_id.map(ToOwned::to_owned),
            seq,
        };
        // A send error just means the receiver was dropped; nothing to do.
        let _ = self.tx.send(msg);
    }
}

/// High-level pub/sub wrapper around [`RelayHandle`].
///
/// Use [`RelaySubscriber::make_handler`] to create a `(handler, receiver)` pair
/// before calling `relay_client::connect`.  Pass the handler to `connect`; keep
/// the receiver to consume incoming messages.  Then wrap the returned
/// `RelayHandle` with [`RelaySubscriber::from_handle`] for ergonomic
/// subscribe/publish calls.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use roko_agent_server::features::relay_subscriber::RelaySubscriber;
/// // (relay_handle obtained from relay_client::connect)
/// # async fn example(relay_handle: roko_agent_server::features::relay_client::RelayHandle) -> anyhow::Result<()> {
/// let (handler, mut rx) = RelaySubscriber::make_handler();
/// // pass handler to relay_client::connect(…, Some(handler))
/// let subscriber = RelaySubscriber::from_handle(relay_handle);
/// subscriber.subscribe("isfr:rates")?;
/// while let Some(msg) = rx.recv().await {
///     println!("topic={} seq={}", msg.topic, msg.seq);
/// }
/// # Ok(())
/// # }
/// ```
pub struct RelaySubscriber {
    handle: RelayHandle,
}

impl RelaySubscriber {
    /// Create a `(TopicHandler, receiver)` pair for channel-based message delivery.
    ///
    /// Pass the returned `handler` to `relay_client::connect` as `topic_handler`.
    /// All incoming topic messages will be forwarded to the returned `receiver`.
    #[must_use]
    pub fn make_handler() -> (Arc<dyn TopicHandler>, mpsc::UnboundedReceiver<TopicMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handler: Arc<dyn TopicHandler> = Arc::new(ChannelTopicHandler { tx });
        (handler, rx)
    }

    /// Wrap an existing [`RelayHandle`] for ergonomic pub/sub calls.
    ///
    /// Use this after calling `relay_client::connect` to get a handle that
    /// provides named `subscribe`/`unsubscribe`/`publish` methods.
    #[must_use]
    pub fn from_handle(handle: RelayHandle) -> Self {
        Self { handle }
    }

    /// Subscribe to `topic` on the relay.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying relay connection has been closed.
    pub fn subscribe(&self, topic: impl Into<String>) -> Result<()> {
        self.handle.subscribe(topic)
    }

    /// Unsubscribe from `topic` on the relay.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying relay connection has been closed.
    pub fn unsubscribe(&self, topic: impl Into<String>) -> Result<()> {
        self.handle.unsubscribe(topic)
    }

    /// Publish `payload` to `topic` with the given `msg_type`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying relay connection has been closed.
    pub fn publish(
        &self,
        topic: impl Into<String>,
        msg_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.handle.publish(topic, msg_type, payload)
    }

    /// Access the underlying [`RelayHandle`] for ad-hoc frame sending.
    #[must_use]
    pub fn handle(&self) -> &RelayHandle {
        &self.handle
    }
}

// ---------------------------------------------------------------------------
// ISFRTopicAdapter
// ---------------------------------------------------------------------------

/// Adapts the relay [`TopicHandler`] interface to [`ISFRFeed`].
///
/// When the relay delivers a [`TopicMessage`] on an ISFR topic, this adapter
/// calls [`ISFRFeed::handle_message`] which republishes the data as a
/// [`roko_core::pulse::Pulse`] on the local bus.
///
/// # Usage
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use roko_core::bus_backends::BroadcastBus;
/// use roko_core::isfr_feed::ISFRFeed;
/// use roko_agent_server::features::relay_subscriber::ISFRTopicAdapter;
///
/// let bus = Arc::new(BroadcastBus::new());
/// let feed = Arc::new(ISFRFeed::new(bus));
/// let handler = ISFRTopicAdapter::make_handler(feed);
/// // pass `Some(handler)` to relay_client::connect(...)
/// ```
pub struct ISFRTopicAdapter {
    feed: Arc<ISFRFeed>,
}

impl ISFRTopicAdapter {
    /// Wrap an [`ISFRFeed`] in an [`Arc`] and return it as a boxed
    /// [`TopicHandler`] ready to pass to `relay_client::connect`.
    #[must_use]
    pub fn make_handler(feed: Arc<ISFRFeed>) -> Arc<dyn TopicHandler> {
        Arc::new(Self { feed })
    }
}

#[async_trait]
impl TopicHandler for ISFRTopicAdapter {
    async fn on_topic_message(
        &self,
        topic: &str,
        msg_type: &str,
        payload: serde_json::Value,
        publisher_id: Option<&str>,
        seq: u64,
    ) {
        self.feed
            .handle_message(topic, msg_type, payload, publisher_id, seq);
    }
}
