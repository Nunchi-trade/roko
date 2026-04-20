//! Event subscriber abstraction. Ports `cli/jobs/events.py`.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::types::ChainEvent;

/// Delivers on-chain events to the keeper engine.
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// Receive the next matching event or `None` when the subscription is closed.
    async fn next(&mut self) -> Option<ChainEvent>;
}

/// A `tokio::mpsc` backed subscriber — useful for tests and for wiring the
/// engine to `roko-runtime::event_bus::EventBus` (which fans out on a
/// broadcast channel internally).
pub struct ChannelSubscriber {
    rx: mpsc::Receiver<ChainEvent>,
    accepted_types: Vec<String>,
}

impl ChannelSubscriber {
    #[must_use]
    pub fn new(rx: mpsc::Receiver<ChainEvent>, accepted_types: Vec<String>) -> Self {
        Self { rx, accepted_types }
    }

    /// Pair builder — returns the subscriber and a sender wrapped in a tx-only
    /// handle tests can use to inject events.
    #[must_use]
    pub fn pair(accepted_types: Vec<String>, buffer: usize) -> (Self, mpsc::Sender<ChainEvent>) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self::new(rx, accepted_types), tx)
    }
}

#[async_trait]
impl EventSubscriber for ChannelSubscriber {
    async fn next(&mut self) -> Option<ChainEvent> {
        while let Some(event) = self.rx.recv().await {
            if self.accepted_types.is_empty() || self.accepted_types.contains(&event.event_type) {
                return Some(event);
            }
        }
        None
    }
}
