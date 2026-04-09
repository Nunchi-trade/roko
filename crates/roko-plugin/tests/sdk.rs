use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use roko_core::{Body, Kind, Result, Signal};
use roko_plugin::{
    EventSource, EventSourceKind, FeedbackCollector, FeedbackSignal, SignalSender,
};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

struct MockEventSource;

struct MockFeedbackCollector;

#[async_trait]
impl EventSource for MockEventSource {
    fn name(&self) -> &str {
        "mock-event-source"
    }

    fn kind(&self) -> EventSourceKind {
        EventSourceKind::Custom("mock".to_string())
    }

    async fn start(&self, sender: SignalSender, cancel: CancellationToken) -> Result<()> {
        tokio::select! {
            _ = sleep(Duration::from_millis(100)) => {
                let signal = Signal::builder(Kind::Task)
                    .body(Body::text("mock signal"))
                    .build();
                sender.send(signal).await.expect("signal should be sent");
                cancel.cancelled().await;
                Ok(())
            }
            _ = cancel.cancelled() => Ok(()),
        }
    }
}

#[async_trait]
impl FeedbackCollector for MockFeedbackCollector {
    fn name(&self) -> &str {
        "mock-feedback-collector"
    }

    fn services(&self) -> Vec<String> {
        vec!["mock".to_string()]
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    async fn collect(&self, _since: chrono::DateTime<Utc>) -> Result<Vec<FeedbackSignal>> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn mock_event_source_emits_signal_after_100ms() {
    let source: Box<dyn EventSource> = Box::new(MockEventSource);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let cancel = CancellationToken::new();
    let runner = tokio::spawn({
        let cancel = cancel.clone();
        async move { source.start(sender, cancel).await }
    });

    let signal = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("signal should arrive")
        .expect("sender should stay open");

    assert_eq!(signal.body, Body::text("mock signal"));

    cancel.cancel();
    runner.await.expect("task should complete").expect("source should exit cleanly");
}

#[tokio::test]
async fn cancellation_token_stops_event_source() {
    let source: Box<dyn EventSource> = Box::new(MockEventSource);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let cancel = CancellationToken::new();
    let runner = tokio::spawn({
        let cancel = cancel.clone();
        async move { source.start(sender, cancel).await }
    });

    cancel.cancel();

    timeout(Duration::from_secs(1), runner)
        .await
        .expect("task should stop after cancellation")
        .expect("source should exit cleanly");

    assert!(receiver.try_recv().is_err(), "event source should not emit after cancellation");
}

#[test]
fn box_dyn_event_source_is_object_safe() {
    let _source: Box<dyn EventSource> = Box::new(MockEventSource);
}

#[tokio::test]
async fn feedback_collector_returns_empty_vec_when_no_feedback_exists() {
    let collector: Box<dyn FeedbackCollector> = Box::new(MockFeedbackCollector);

    let feedback = collector
        .collect(chrono::DateTime::<Utc>::UNIX_EPOCH)
        .await
        .expect("collector should succeed");

    assert!(feedback.is_empty());
}
