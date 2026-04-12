use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use serde_json::Value;

use crate::tool_loop::{LlmBackend, LlmError};
use crate::translate::{BackendResponse, RenderedTools, SessionState};

/// Sends to a primary backend first and fires a backup only when the
/// primary exceeds the configured hedge threshold.
pub struct HedgedBackend {
    primary: Arc<dyn LlmBackend>,
    backup: Arc<dyn LlmBackend>,
    hedge_after_ms: u64,
}

impl HedgedBackend {
    /// Build a hedged backend with a primary, backup, and hedge delay.
    #[must_use]
    pub const fn new(
        primary: Arc<dyn LlmBackend>,
        backup: Arc<dyn LlmBackend>,
        hedge_after_ms: u64,
    ) -> Self {
        Self {
            primary,
            backup,
            hedge_after_ms,
        }
    }
}

#[async_trait]
impl LlmBackend for HedgedBackend {
    async fn send_turn(
        &self,
        messages: &[Value],
        tools: &RenderedTools,
        session: &SessionState,
    ) -> Result<BackendResponse, LlmError> {
        let primary_fut = self.primary.send_turn(messages, tools, session);
        tokio::pin!(primary_fut);

        tokio::select! {
            biased;
            result = &mut primary_fut => result,
            _ = tokio::time::sleep(Duration::from_millis(self.hedge_after_ms)) => {
                let backup_fut = self.backup.send_turn(messages, tools, session);
                tokio::pin!(backup_fut);

                tokio::select! {
                    biased;
                    result = &mut primary_fut => result,
                    result = &mut backup_fut => result,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HedgedBackend;
    use crate::tool_loop::{LlmBackend, LlmError};
    use crate::translate::{BackendResponse, RenderedTools, SessionState};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tokio::sync::Notify;

    struct DelayedBackend {
        response_text: &'static str,
        delay_ms: u64,
        calls: AtomicUsize,
        started: Notify,
    }

    impl DelayedBackend {
        fn new(response_text: &'static str, delay_ms: u64) -> Self {
            Self {
                response_text,
                delay_ms,
                calls: AtomicUsize::new(0),
                started: Notify::new(),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        async fn wait_until_called(&self) {
            while self.calls() == 0 {
                self.started.notified().await;
            }
        }
    }

    #[async_trait]
    impl LlmBackend for DelayedBackend {
        async fn send_turn(
            &self,
            _messages: &[Value],
            _tools: &RenderedTools,
            _session: &SessionState,
        ) -> Result<BackendResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_waiters();
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            Ok(BackendResponse::Text(self.response_text.to_string()))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn hedged_backend_primary_response_within_threshold_does_not_fire_backup() {
        let primary = Arc::new(DelayedBackend::new("primary", 20));
        let backup = Arc::new(DelayedBackend::new("backup", 5));
        let backend = HedgedBackend::new(primary.clone(), backup.clone(), 50);
        let messages = [serde_json::json!({"role": "user", "content": "ping"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));

        let call = tokio::spawn(async move { backend.send_turn(&messages, &tools, &SessionState::default()).await });

        primary.wait_until_called().await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(20)).await;

        let response = call
            .await
            .expect("task should complete")
            .expect("backend should succeed");
        assert!(matches!(response, BackendResponse::Text(ref text) if text == "primary"));
        assert_eq!(primary.calls(), 1);
        assert_eq!(backup.calls(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn hedged_backend_primary_slow_fires_backup_and_returns_first_response() {
        let primary = Arc::new(DelayedBackend::new("primary", 100));
        let backup = Arc::new(DelayedBackend::new("backup", 10));
        let backend = HedgedBackend::new(primary.clone(), backup.clone(), 50);
        let messages = [serde_json::json!({"role": "user", "content": "ping"})];
        let tools = RenderedTools::JsonArray(serde_json::json!([]));

        let call = tokio::spawn(async move { backend.send_turn(&messages, &tools, &SessionState::default()).await });

        primary.wait_until_called().await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(50)).await;
        backup.wait_until_called().await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(10)).await;

        let response = call
            .await
            .expect("task should complete")
            .expect("backend should succeed");
        assert!(matches!(response, BackendResponse::Text(ref text) if text == "backup"));
        assert_eq!(primary.calls(), 1);
        assert_eq!(backup.calls(), 1);
    }
}
