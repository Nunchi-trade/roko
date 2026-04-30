//! AcpAdapter — bridges RuntimeEvent → CognitiveEvent for ACP sessions.
//!
//! Implements EventConsumer to receive workflow events and maps them
//! to the ACP session update protocol.

use roko_core::foundation::EventConsumer;
use roko_core::runtime_event::{RuntimeEvent, WorkflowOutcome};
use tokio::sync::mpsc;

use crate::bridge_events::CognitiveEvent;
use crate::types::{ContentBlock, StopReason, ToolCallKind, ToolCallStatus};

/// Adapter that translates RuntimeEvents into ACP CognitiveEvents.
///
/// Created per-session and registered as an EventConsumer on the
/// WorkflowEngine. When the engine emits events, this adapter filters
/// for the relevant run_id and forwards mapped events to the ACP session.
pub struct AcpAdapter {
    /// Session ID this adapter is associated with.
    session_id: String,
    /// Run ID to filter events for.
    run_id: String,
    /// Channel to send mapped events to the ACP session handler.
    sender: mpsc::Sender<CognitiveEvent>,
}

impl AcpAdapter {
    /// Create a new AcpAdapter for the given session and run.
    pub fn new(session_id: String, run_id: String, sender: mpsc::Sender<CognitiveEvent>) -> Self {
        Self {
            session_id,
            run_id,
            sender,
        }
    }

    /// Map a RuntimeEvent to an optional CognitiveEvent.
    fn map_event(&self, event: &RuntimeEvent) -> Option<CognitiveEvent> {
        if event.run_id() != self.run_id {
            return None;
        }

        match event {
            RuntimeEvent::AgentOutput { chunk, .. } => {
                Some(CognitiveEvent::TokenChunk(chunk.clone()))
            }
            RuntimeEvent::AgentSpawned { agent_id, role, .. } => {
                Some(CognitiveEvent::ToolCallStart {
                    tool_call_id: agent_id.clone(),
                    title: format!("Agent: {role}"),
                    kind: ToolCallKind::Other,
                    locations: None,
                })
            }
            RuntimeEvent::AgentCompleted {
                agent_id, output, ..
            } => Some(CognitiveEvent::ToolCallComplete {
                tool_call_id: agent_id.clone(),
                status: ToolCallStatus::Completed,
                content: vec![text_block(output.clone())],
            }),
            RuntimeEvent::AgentFailed {
                agent_id, error, ..
            } => Some(CognitiveEvent::ToolCallComplete {
                tool_call_id: agent_id.clone(),
                status: ToolCallStatus::Failed,
                content: vec![text_block(error.clone())],
            }),
            RuntimeEvent::GateStarted { gate_name, .. } => Some(CognitiveEvent::ToolCallStart {
                tool_call_id: gate_call_id(gate_name),
                title: format!("Gate: {gate_name}"),
                kind: ToolCallKind::Other,
                locations: None,
            }),
            RuntimeEvent::GatePassed { gate_name, .. } => Some(CognitiveEvent::ToolCallComplete {
                tool_call_id: gate_call_id(gate_name),
                status: ToolCallStatus::Completed,
                content: vec![text_block(format!("{gate_name} passed"))],
            }),
            RuntimeEvent::GateFailed {
                gate_name, output, ..
            } => Some(CognitiveEvent::ToolCallComplete {
                tool_call_id: gate_call_id(gate_name),
                status: ToolCallStatus::Failed,
                content: vec![text_block(output.clone())],
            }),
            RuntimeEvent::PhaseTransition { from, to, .. } => Some(CognitiveEvent::TokenChunk(
                format!("[Phase: {from} -> {to}]\n"),
            )),
            RuntimeEvent::WorkflowCompleted { outcome, .. } => Some(CognitiveEvent::Complete {
                stop_reason: stop_reason_for_outcome(outcome),
                usage: None,
            }),
            RuntimeEvent::WorkflowStarted { .. }
            | RuntimeEvent::FeedbackRecorded { .. }
            | RuntimeEvent::StateCheckpointed { .. } => None,
        }
    }

    /// Session ID this adapter forwards events for.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Run ID this adapter accepts events for.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

impl EventConsumer for AcpAdapter {
    fn consume(&self, event: &RuntimeEvent) {
        if let Some(cognitive_event) = self.map_event(event) {
            let _ = self.sender.try_send(cognitive_event);
        }
    }
}

fn gate_call_id(gate_name: &str) -> String {
    format!("gate-{gate_name}")
}

fn text_block(text: String) -> ContentBlock {
    ContentBlock::Text { text }
}

fn stop_reason_for_outcome(outcome: &WorkflowOutcome) -> StopReason {
    match outcome {
        WorkflowOutcome::Cancelled => StopReason::Cancelled,
        WorkflowOutcome::Success { .. } | WorkflowOutcome::Halted { .. } => StopReason::EndTurn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_run_id() {
        let (tx, _rx) = mpsc::channel(16);
        let adapter = AcpAdapter::new("sess1".into(), "run1".into(), tx);

        let event = RuntimeEvent::AgentOutput {
            run_id: "run1".into(),
            agent_id: "a1".into(),
            chunk: "hello".into(),
        };
        assert!(adapter.map_event(&event).is_some());

        let event = RuntimeEvent::AgentOutput {
            run_id: "run2".into(),
            agent_id: "a1".into(),
            chunk: "hello".into(),
        };
        assert!(adapter.map_event(&event).is_none());
    }

    #[test]
    fn maps_gate_events() {
        let (tx, _rx) = mpsc::channel(16);
        let adapter = AcpAdapter::new("sess1".into(), "run1".into(), tx);

        let event = RuntimeEvent::GatePassed {
            run_id: "run1".into(),
            gate_name: "compile".into(),
            duration_ms: 1000,
        };

        let mapped = adapter.map_event(&event).unwrap();
        match mapped {
            CognitiveEvent::ToolCallComplete { status, .. } => {
                assert_eq!(status, ToolCallStatus::Completed);
            }
            _ => panic!("Expected ToolCallComplete"),
        }
    }

    #[test]
    fn maps_cancelled_workflow_to_cancelled_stop_reason() {
        let (tx, _rx) = mpsc::channel(16);
        let adapter = AcpAdapter::new("sess1".into(), "run1".into(), tx);

        let event = RuntimeEvent::WorkflowCompleted {
            run_id: "run1".into(),
            outcome: WorkflowOutcome::Cancelled,
        };

        let mapped = adapter.map_event(&event).unwrap();
        match mapped {
            CognitiveEvent::Complete { stop_reason, .. } => {
                assert_eq!(stop_reason, StopReason::Cancelled);
            }
            _ => panic!("Expected Complete"),
        }
    }
}
