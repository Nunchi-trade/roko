//! Unified event types shared across learning subsystems.
//!
//! These events are intentionally lightweight and provider-agnostic so runtime
//! components can publish one stream that downstream learning systems consume.

use crate::anomaly::Anomaly;
use crate::provider_health::ErrorClass;
use roko_agent::chat_types::FinishReason;
use roko_agent::{StreamChunk, Usage};
use tokio::sync::broadcast;

/// Canonical event payload emitted by the learning/runtime feedback pipeline.
#[allow(missing_docs)]
#[derive(Debug, Clone)]
pub enum AgentEvent {
    TurnStarted {
        task_id: String,
        model: String,
        provider: String,
        timestamp_ms: i64,
    },
    ToolCallExecuted {
        tool_name: String,
        duration_ms: u64,
        success: bool,
        result_tokens: u64,
    },
    TurnCompleted {
        turn: u32,
        usage: Usage,
        tool_call_count: usize,
        gate_passed: Option<bool>,
        finish_reason: FinishReason,
    },
    GateResult {
        gate_name: String,
        passed: bool,
        score: f32,
        duration_ms: u64,
    },
    ProviderError {
        provider_id: String,
        error_class: ErrorClass,
        status: u16,
    },
    CostRecorded {
        model: String,
        provider: String,
        cost_usd: f64,
        tokens: u64,
    },
    AnomalyDetected {
        anomaly: Anomaly,
    },
    ExperimentAssigned {
        experiment_id: String,
        variant_id: String,
    },
    SessionEstablished {
        session_id: String,
        provider: String,
    },
    ModelSelected {
        model: String,
        stage: String,
        score: f64,
    },
    StreamChunk {
        chunk: StreamChunk,
    },
}

/// Pub/sub bus for broadcasting learning events to multiple subscribers.
pub struct EventBus {
    tx: broadcast::Sender<AgentEvent>,
}

impl EventBus {
    /// Create a new event bus with the given broadcast channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish an event to all live subscribers.
    ///
    /// If there are no subscribers, the event is dropped.
    pub fn publish(&self, event: AgentEvent) {
        let _ = self.tx.send(event);
    }

    /// Subscribe to future events from this bus.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.tx.subscribe()
    }
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentEvent, EventBus};

    #[tokio::test]
    async fn event_bus_broadcasts_to_multiple_subscribers() {
        let bus = EventBus::new(16);
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();

        bus.publish(AgentEvent::SessionEstablished {
            session_id: "session-123".into(),
            provider: "zai".into(),
        });

        let first_event = first
            .recv()
            .await
            .expect("first subscriber should receive event");
        let second_event = second
            .recv()
            .await
            .expect("second subscriber should receive event");

        assert!(matches!(
            first_event,
            AgentEvent::SessionEstablished {
                session_id,
                provider,
            } if session_id == "session-123" && provider == "zai"
        ));
        assert!(matches!(
            second_event,
            AgentEvent::SessionEstablished {
                session_id,
                provider,
            } if session_id == "session-123" && provider == "zai"
        ));
    }

    #[test]
    fn event_bus_publish_with_no_subscribers_does_not_error() {
        let bus = EventBus::new(16);
        bus.publish(AgentEvent::SessionEstablished {
            session_id: "session-123".into(),
            provider: "zai".into(),
        });
    }
}
