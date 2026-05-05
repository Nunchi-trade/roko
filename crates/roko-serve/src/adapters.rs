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
        let (kind, run_id, data) = match event {
            RuntimeEvent::WorkflowStarted {
                run_id,
                template,
                prompt,
            } => (
                "workflow_started",
                run_id.as_str(),
                serde_json::json!({
                    "template": template,
                    "prompt": prompt,
                }),
            ),
            RuntimeEvent::PhaseTransition { run_id, from, to } => (
                "phase_transition",
                run_id.as_str(),
                serde_json::json!({
                    "from": from,
                    "to": to,
                }),
            ),
            RuntimeEvent::WorkflowCompleted { run_id, outcome } => (
                "workflow_completed",
                run_id.as_str(),
                serde_json::json!({
                    "outcome": outcome.to_string(),
                }),
            ),
            RuntimeEvent::AgentSpawned {
                run_id,
                agent_id,
                role,
                model,
            } => (
                "agent_spawned",
                run_id.as_str(),
                serde_json::json!({
                    "agent_id": agent_id,
                    "role": role,
                    "model": model,
                }),
            ),
            RuntimeEvent::AgentOutput {
                run_id,
                agent_id,
                chunk,
            } => (
                "agent_output",
                run_id.as_str(),
                serde_json::json!({
                    "agent_id": agent_id,
                    "chunk": chunk,
                }),
            ),
            RuntimeEvent::AgentCompleted {
                run_id,
                agent_id,
                tokens_used,
                cost_usd,
                ..
            } => (
                "agent_completed",
                run_id.as_str(),
                serde_json::json!({
                    "agent_id": agent_id,
                    "tokens_used": tokens_used,
                    "cost_usd": cost_usd,
                }),
            ),
            RuntimeEvent::AgentFailed {
                run_id,
                agent_id,
                error,
            } => (
                "agent_failed",
                run_id.as_str(),
                serde_json::json!({
                    "agent_id": agent_id,
                    "error": error,
                }),
            ),
            RuntimeEvent::GateStarted {
                run_id,
                gate_name,
                rung,
                ..
            } => (
                "gate_started",
                run_id.as_str(),
                serde_json::json!({
                    "gate_name": gate_name,
                    "rung": rung,
                }),
            ),
            RuntimeEvent::GatePassed {
                run_id,
                gate_name,
                duration_ms,
            } => (
                "gate_passed",
                run_id.as_str(),
                serde_json::json!({
                    "gate_name": gate_name,
                    "duration_ms": duration_ms,
                }),
            ),
            RuntimeEvent::GateFailed {
                run_id,
                gate_name,
                output,
                duration_ms,
            } => (
                "gate_failed",
                run_id.as_str(),
                serde_json::json!({
                    "gate_name": gate_name,
                    "output": output,
                    "duration_ms": duration_ms,
                }),
            ),
            RuntimeEvent::FeedbackRecorded {
                run_id,
                kind: feedback_kind,
                summary,
            } => (
                "feedback_recorded",
                run_id.as_str(),
                serde_json::json!({
                    "feedback_kind": feedback_kind,
                    "summary": summary,
                }),
            ),
            RuntimeEvent::StateCheckpointed { run_id, path } => (
                "state_checkpointed",
                run_id.as_str(),
                serde_json::json!({
                    "path": path,
                }),
            ),
            RuntimeEvent::InferenceStarted {
                request_id,
                model,
                agent_id,
                auto_routed,
                ..
            } => (
                "inference_started",
                "",
                serde_json::json!({
                    "request_id": request_id,
                    "model": model,
                    "agent_id": agent_id,
                    "auto_routed": auto_routed,
                }),
            ),
            RuntimeEvent::InferenceCompleted {
                request_id,
                model,
                agent_id,
                input_tokens,
                output_tokens,
                cost_usd,
                duration_ms,
                ..
            } => (
                "inference_completed",
                "",
                serde_json::json!({
                    "request_id": request_id,
                    "model": model,
                    "agent_id": agent_id,
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "cost_usd": cost_usd,
                    "duration_ms": duration_ms,
                }),
            ),
            RuntimeEvent::InferenceFailed {
                request_id,
                model,
                agent_id,
                error,
                ..
            } => (
                "inference_failed",
                "",
                serde_json::json!({
                    "request_id": request_id,
                    "model": model,
                    "agent_id": agent_id,
                    "error": error,
                }),
            ),
            RuntimeEvent::AgentTrace {
                agent_id,
                turn,
                tool_calls,
                reasoning,
                usage,
                ..
            } => (
                "agent_trace",
                "",
                serde_json::json!({
                    "agent_id": agent_id,
                    "turn": turn,
                    "tool_calls": tool_calls,
                    "reasoning": reasoning,
                    "usage": usage,
                }),
            ),
            RuntimeEvent::TaskFailed {
                plan_id,
                task_id,
                error,
                gate_failure,
                ..
            } => (
                "task_failed",
                plan_id.as_str(),
                serde_json::json!({
                    "plan_id": plan_id,
                    "task_id": task_id,
                    "error": error,
                    "gate_failure": gate_failure,
                }),
            ),
            RuntimeEvent::RunStarted {
                run_id,
                prompt,
                complexity,
                ..
            } => (
                "run_started",
                run_id.as_str(),
                serde_json::json!({
                    "run_id": run_id,
                    "prompt": prompt,
                    "complexity": complexity,
                }),
            ),
            RuntimeEvent::RunCompleted {
                run_id,
                success,
                cost_usd,
                duration_ms,
                ..
            } => (
                "run_completed",
                run_id.as_str(),
                serde_json::json!({
                    "run_id": run_id,
                    "success": success,
                    "cost_usd": cost_usd,
                    "duration_ms": duration_ms,
                }),
            ),
            RuntimeEvent::KnowledgeIngested {
                entry_id,
                topic,
                source_agent,
                ..
            } => (
                "knowledge_ingested",
                "",
                serde_json::json!({
                    "entry_id": entry_id,
                    "topic": topic,
                    "source_agent": source_agent,
                }),
            ),
            RuntimeEvent::KnowledgeConsumed {
                entry_id,
                topic,
                consuming_agent,
                ..
            } => (
                "knowledge_consumed",
                "",
                serde_json::json!({
                    "entry_id": entry_id,
                    "topic": topic,
                    "consuming_agent": consuming_agent,
                }),
            ),
        };

        SseEvent {
            kind: kind.to_string(),
            run_id: run_id.to_string(),
            data,
        }
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
