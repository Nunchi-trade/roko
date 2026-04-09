//! Plugin SDK for Roko event sources and feedback collectors.

use async_trait::async_trait;
use roko_core::{Result, Signal};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

/// Cloneable sender used by event sources to publish signals into Roko.
pub type SignalSender = UnboundedSender<Signal>;

/// Kinds of event sources supported by the plugin SDK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventSourceKind {
    /// HTTP webhook source.
    Webhook,
    /// Scheduled source.
    Cron,
    /// Filesystem watcher source.
    FileWatch,
    /// Custom source type provided by a plugin.
    Custom,
}

/// An asynchronous source of signals.
///
/// Implementors are expected to run until `cancel` fires, publishing
/// [`Signal`]s via `sender`. The trait is object-safe, so sources can be
/// stored and driven as `Box<dyn EventSource>`.
#[async_trait]
pub trait EventSource: Send + Sync + 'static {
    /// Human-readable source name.
    fn name(&self) -> &str;

    /// The source kind.
    fn kind(&self) -> EventSourceKind;

    /// Start the source and keep running until cancellation is requested.
    async fn start(&self, sender: SignalSender, cancel: CancellationToken) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind, Signal};

    struct DummyEventSource;

    #[async_trait]
    impl EventSource for DummyEventSource {
        fn name(&self) -> &str {
            "dummy"
        }

        fn kind(&self) -> EventSourceKind {
            EventSourceKind::Custom
        }

        async fn start(&self, sender: SignalSender, cancel: CancellationToken) -> Result<()> {
            let signal = Signal::builder(Kind::Task)
                .body(Body::text("hello"))
                .build();
            let _ = sender.send(signal);
            cancel.cancelled().await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn event_source_is_object_safe() {
        let source: Box<dyn EventSource> = Box::new(DummyEventSource);
        assert_eq!(source.name(), "dummy");
        assert_eq!(source.kind(), EventSourceKind::Custom);

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();
        let runner = tokio::spawn(async move { source.start(sender, cancel_for_task).await });

        let signal = receiver.recv().await.expect("signal should be sent");
        assert_eq!(signal.body, Body::text("hello"));

        cancel.cancel();
        runner.await.expect("task should complete").expect("source should exit cleanly");
    }
}
