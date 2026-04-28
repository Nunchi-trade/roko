//! SseAdapter - bridges RuntimeEvent -> Server-Sent Events.
//!
//! Implements EventConsumer to receive workflow events and forwards them
//! to connected SSE clients as JSON event data.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use roko_core::foundation::EventConsumer;
use roko_core::runtime_event::RuntimeEvent;
use roko_core::runtime_event::RuntimeEventEnvelope;
use serde::Serialize;
use tokio::sync::broadcast;

/// JSON-serializable event for SSE clients.
#[derive(Debug, Clone, Serialize)]
pub struct SseEvent {
    /// Event kind (matches RuntimeEvent::kind()).
    pub kind: String,
    /// Run ID.
    pub run_id: String,
    /// Event-specific data.
    pub data: serde_json::Value,
}

/// Adapter that translates RuntimeEvents into SSE-compatible JSON events.
///
/// Maintains a broadcast channel that SSE endpoint handlers can subscribe to.
pub struct SseAdapter {
    sender: broadcast::Sender<SseEvent>,
    state_hub_consumer: RwLock<Option<Arc<dyn EventConsumer>>>,
    runtime_subscription_started: AtomicBool,
}

impl SseAdapter {
    /// Create a new SseAdapter with the given channel capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            state_hub_consumer: RwLock::new(None),
            runtime_subscription_started: AtomicBool::new(false),
        }
    }

    /// Subscribe to the SSE event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.sender.subscribe()
    }

    /// Number of active SSE subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    pub fn set_state_hub_consumer(&self, consumer: Arc<dyn EventConsumer>) {
        let mut slot = self
            .state_hub_consumer
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(consumer);
    }

    pub fn start_runtime_event_subscription(self: &Arc<Self>) {
        if self
            .runtime_subscription_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.runtime_subscription_started
                .store(false, Ordering::Release);
            tracing::warn!("workflow SSE runtime event subscription requires a tokio runtime");
            return;
        };

        let adapter = Arc::clone(self);
        let mut rx = roko_runtime::event_bus::runtime_event_bus::<RuntimeEvent>().subscribe();
        handle.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(envelope) => adapter.consume(&envelope.payload),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(n, "workflow SSE runtime event bridge lagged");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub fn consume_envelope(&self, envelope: &RuntimeEventEnvelope) {
        self.consume(&envelope.payload);
    }

    /// Convert a RuntimeEvent to an SseEvent.
    fn to_sse_event(event: &RuntimeEvent) -> SseEvent {
        let run_id = event.run_id().to_string();
        let kind = event.kind().to_string();

        let data = match event {
            RuntimeEvent::WorkflowStarted {
                template, prompt, ..
            } => {
                serde_json::json!({
                    "template": template,
                    "prompt": prompt,
                })
            }
            RuntimeEvent::PhaseTransition { from, to, .. } => {
                serde_json::json!({
                    "from": from,
                    "to": to,
                })
            }
            RuntimeEvent::WorkflowCompleted { outcome, .. } => {
                serde_json::json!({
                    "outcome": outcome.to_string(),
                })
            }
            RuntimeEvent::AgentSpawned {
                agent_id,
                role,
                model,
                ..
            } => {
                serde_json::json!({
                    "agent_id": agent_id,
                    "role": role,
                    "model": model,
                })
            }
            RuntimeEvent::AgentOutput {
                agent_id, chunk, ..
            } => {
                serde_json::json!({
                    "agent_id": agent_id,
                    "chunk": chunk,
                })
            }
            RuntimeEvent::AgentCompleted {
                agent_id,
                tokens_used,
                cost_usd,
                ..
            } => {
                serde_json::json!({
                    "agent_id": agent_id,
                    "tokens_used": tokens_used,
                    "cost_usd": cost_usd,
                })
            }
            RuntimeEvent::AgentFailed {
                agent_id, error, ..
            } => {
                serde_json::json!({
                    "agent_id": agent_id,
                    "error": error,
                })
            }
            RuntimeEvent::GateStarted {
                gate_name, rung, ..
            } => {
                serde_json::json!({
                    "gate_name": gate_name,
                    "rung": rung,
                })
            }
            RuntimeEvent::GatePassed {
                gate_name,
                duration_ms,
                ..
            } => {
                serde_json::json!({
                    "gate_name": gate_name,
                    "duration_ms": duration_ms,
                })
            }
            RuntimeEvent::GateFailed {
                gate_name,
                output,
                duration_ms,
                ..
            } => {
                serde_json::json!({
                    "gate_name": gate_name,
                    "output": output,
                    "duration_ms": duration_ms,
                })
            }
            RuntimeEvent::FeedbackRecorded {
                kind: feedback_kind,
                summary,
                ..
            } => {
                serde_json::json!({
                    "feedback_kind": feedback_kind,
                    "summary": summary,
                })
            }
            RuntimeEvent::StateCheckpointed { path, .. } => {
                serde_json::json!({
                    "path": path,
                })
            }
        };

        SseEvent { kind, run_id, data }
    }
}

/// Get the SseAdapter as an EventConsumer for WorkflowEngine registration.
#[must_use]
pub fn sse_event_consumer(adapter: &Arc<SseAdapter>) -> Arc<dyn EventConsumer> {
    Arc::clone(adapter) as Arc<dyn EventConsumer>
}

impl EventConsumer for SseAdapter {
    fn consume(&self, event: &RuntimeEvent) {
        let sse_event = Self::to_sse_event(event);
        // Non-blocking: if no subscribers exist, the event is dropped.
        let _ = self.sender.send(sse_event);

        let consumer = self
            .state_hub_consumer
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(consumer) = consumer {
            consumer.consume(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_events_to_sse() {
        let event = RuntimeEvent::GatePassed {
            run_id: "r1".into(),
            gate_name: "compile".into(),
            duration_ms: 1500,
        };

        let sse = SseAdapter::to_sse_event(&event);
        assert_eq!(sse.kind, "gate_passed");
        assert_eq!(sse.run_id, "r1");
        assert_eq!(sse.data["gate_name"], "compile");
    }

    #[test]
    fn subscribe_receives_events() {
        let adapter = SseAdapter::new(16);
        let mut rx = adapter.subscribe();

        adapter.consume(&RuntimeEvent::WorkflowStarted {
            run_id: "r1".into(),
            template: "express".into(),
            prompt: "fix bug".into(),
        });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.kind, "workflow_started");
    }

    #[test]
    fn sse_adapter_broadcast_to_multiple_subscribers() {
        let adapter = SseAdapter::new(16);
        let mut rx1 = adapter.subscribe();
        let mut rx2 = adapter.subscribe();

        adapter.consume(&RuntimeEvent::WorkflowStarted {
            run_id: "r1".into(),
            template: "standard".into(),
            prompt: "test".into(),
        });

        let e1 = rx1.try_recv().unwrap();
        let e2 = rx2.try_recv().unwrap();
        assert_eq!(e1.run_id, "r1");
        assert_eq!(e2.run_id, "r1");
    }

    #[test]
    fn sse_adapter_subscriber_count() {
        let adapter = SseAdapter::new(16);
        assert_eq!(adapter.subscriber_count(), 0);
        let _rx = adapter.subscribe();
        assert_eq!(adapter.subscriber_count(), 1);
    }
}
