//! Thin pub/sub wrapper around [`RelayHandle`].
//!
//! [`RelaySubscriber`] provides a higher-level interface over the raw
//! subscribe/unsubscribe/publish methods on [`RelayHandle`].  Callers can
//! bundle a handle + their topic subscriptions into a single value that is
//! easy to pass around without exposing the full relay-client API.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
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
/// Create via [`RelaySubscriber::new`], subscribe to topics with
/// [`RelaySubscriber::subscribe`], and receive messages via the
/// [`mpsc::UnboundedReceiver`] returned by [`RelaySubscriber::new`].
///
/// # Example
///
/// ```no_run
/// use roko_agent_server::features::relay_subscriber::RelaySubscriber;
/// // (relay_handle obtained from relay_client::connect)
/// let (subscriber, mut rx) = RelaySubscriber::new(relay_handle);
/// subscriber.subscribe("isfr:rates")?;
/// while let Some(msg) = rx.recv().await {
///     println!("got {:?}", msg);
/// }
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct RelaySubscriber {
    handle: RelayHandle,
}

impl RelaySubscriber {
    /// Wrap a [`RelayHandle`] for pub/sub use.
    ///
    /// Returns a `(RelaySubscriber, receiver)` pair.  All topic messages
    /// delivered to the relay client will be forwarded to the receiver.
    ///
    /// To use the channel-based handler, build the relay connection with
    /// [`relay_client::connect`] passing the [`Arc<dyn TopicHandler>`]
    /// produced by [`RelaySubscriber::make_handler`]:
    ///
    /// ```no_run
    /// use roko_agent_server::features::relay_subscriber::RelaySubscriber;
    /// let (handler, rx) = RelaySubscriber::make_handler();
    /// // pass handler to relay_client::connect(…, Some(handler))
    /// // then wrap the returned handle:
    /// // let subscriber = RelaySubscriber::from_handle(handle);
    /// ```
    #[must_use]
    pub fn make_handler() -> (Arc<dyn TopicHandler>, mpsc::UnboundedReceiver<TopicMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handler: Arc<dyn TopicHandler> = Arc::new(ChannelTopicHandler { tx });
        (handler, rx)
    }

    /// Wrap an existing [`RelayHandle`] without a dedicated receiver channel.
    ///
    /// Use this when you only need to publish/subscribe without consuming
    /// messages through this wrapper (e.g. the handler was supplied separately
    /// via [`RelaySubscriber::make_handler`]).
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
