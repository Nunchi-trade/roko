## Batch P3A: AcpAdapter (EventConsumer → ACP)

### Write Scope
- **CREATE**: `crates/roko-acp/src/acp_adapter.rs`
- **MODIFY**: `crates/roko-acp/src/lib.rs` (add `pub mod acp_adapter;`)

### Dependencies
- P0B (EventConsumer trait)
- P0C (RuntimeEvent bus)

### DO NOT
- Modify any other files (especially not `bridge_events.rs` or `runner.rs`)
- Add Cargo.toml dependencies
- Replace existing ACP event handling — this is an ADDITION
- Create a new crate

### Existing Code Context

The ACP module already has `CognitiveEvent` for its internal event protocol:
```rust
pub enum CognitiveEvent {
    TokenChunk { session_id: String, content: String },
    ToolCallStart { session_id: String, call_id: String, title: String },
    ToolCallComplete { session_id: String, call_id: String, output: String, success: bool },
    PlanUpdate { session_id: String, entries: Vec<PlanEntry> },
    Complete { session_id: String, stop_reason: String },
}
```

### Task

Create `AcpAdapter` — implements `EventConsumer` to bridge `RuntimeEvent`s into the existing
ACP event protocol (`CognitiveEvent`). When the WorkflowEngine emits runtime events, this
adapter maps them to ACP session updates.

#### File: `crates/roko-acp/src/acp_adapter.rs`

```rust
//! AcpAdapter — bridges RuntimeEvent → CognitiveEvent for ACP sessions.
//!
//! Implements EventConsumer to receive workflow events and maps them
//! to the ACP session update protocol.

use roko_core::foundation::EventConsumer;
use roko_core::runtime_event::RuntimeEvent;
use tokio::sync::mpsc;

use crate::bridge_events::CognitiveEvent;
use crate::types::{PlanEntry, PlanStatus, Priority};

/// Adapter that translates RuntimeEvents into ACP CognitiveEvents.
///
/// Created per-session and registered as an EventConsumer on the
/// WorkflowEngine. When the engine emits events, this adapter filters
/// for the relevant run_id and forwards mapped events to the ACP session.
pub struct AcpAdapter {
    /// Session ID this adapter is associated with
    session_id: String,
    /// Run ID to filter events for
    run_id: String,
    /// Channel to send mapped events to the ACP session handler
    sender: mpsc::Sender<CognitiveEvent>,
}

impl AcpAdapter {
    /// Create a new AcpAdapter for the given session and run.
    pub fn new(
        session_id: String,
        run_id: String,
        sender: mpsc::Sender<CognitiveEvent>,
    ) -> Self {
        Self {
            session_id,
            run_id,
            sender,
        }
    }

    /// Map a RuntimeEvent to an optional CognitiveEvent.
    fn map_event(&self, event: &RuntimeEvent) -> Option<CognitiveEvent> {
        // Only process events for our run
        if event.run_id() != self.run_id {
            return None;
        }

        match event {
            RuntimeEvent::AgentOutput { chunk, .. } => {
                Some(CognitiveEvent::TokenChunk {
                    session_id: self.session_id.clone(),
                    content: chunk.clone(),
                })
            }

            RuntimeEvent::AgentSpawned { agent_id, role, .. } => {
                Some(CognitiveEvent::ToolCallStart {
                    session_id: self.session_id.clone(),
                    call_id: agent_id.clone(),
                    title: format!("Agent: {}", role),
                })
            }

            RuntimeEvent::AgentCompleted { agent_id, output, .. } => {
                Some(CognitiveEvent::ToolCallComplete {
                    session_id: self.session_id.clone(),
                    call_id: agent_id.clone(),
                    output: output.clone(),
                    success: true,
                })
            }

            RuntimeEvent::AgentFailed { agent_id, error, .. } => {
                Some(CognitiveEvent::ToolCallComplete {
                    session_id: self.session_id.clone(),
                    call_id: agent_id.clone(),
                    output: error.clone(),
                    success: false,
                })
            }

            RuntimeEvent::GateStarted { gate_name, .. } => {
                Some(CognitiveEvent::ToolCallStart {
                    session_id: self.session_id.clone(),
                    call_id: format!("gate-{}", gate_name),
                    title: format!("Gate: {}", gate_name),
                })
            }

            RuntimeEvent::GatePassed { gate_name, .. } => {
                Some(CognitiveEvent::ToolCallComplete {
                    session_id: self.session_id.clone(),
                    call_id: format!("gate-{}", gate_name),
                    output: format!("✓ {} passed", gate_name),
                    success: true,
                })
            }

            RuntimeEvent::GateFailed { gate_name, output, .. } => {
                Some(CognitiveEvent::ToolCallComplete {
                    session_id: self.session_id.clone(),
                    call_id: format!("gate-{}", gate_name),
                    output: output.clone(),
                    success: false,
                })
            }

            RuntimeEvent::PhaseTransition { from, to, .. } => {
                Some(CognitiveEvent::TokenChunk {
                    session_id: self.session_id.clone(),
                    content: format!("[Phase: {} → {}]\n", from, to),
                })
            }

            RuntimeEvent::WorkflowCompleted { outcome, .. } => {
                Some(CognitiveEvent::Complete {
                    session_id: self.session_id.clone(),
                    stop_reason: outcome.to_string(),
                })
            }

            // Events that don't need ACP mapping
            RuntimeEvent::WorkflowStarted { .. }
            | RuntimeEvent::FeedbackRecorded { .. }
            | RuntimeEvent::StateCheckpointed { .. } => None,
        }
    }
}

impl EventConsumer for AcpAdapter {
    fn consume(&self, event: &RuntimeEvent) {
        if let Some(cognitive_event) = self.map_event(event) {
            // Non-blocking send — if the channel is full, drop the event
            let _ = self.sender.try_send(cognitive_event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_run_id() {
        let (tx, _rx) = mpsc::channel(16);
        let adapter = AcpAdapter::new("sess1".into(), "run1".into(), tx);

        // Event for our run — should produce a mapping
        let event = RuntimeEvent::AgentOutput {
            run_id: "run1".into(),
            agent_id: "a1".into(),
            chunk: "hello".into(),
        };
        assert!(adapter.map_event(&event).is_some());

        // Event for different run — should be filtered out
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
            CognitiveEvent::ToolCallComplete { success, .. } => assert!(success),
            _ => panic!("Expected ToolCallComplete"),
        }
    }
}
```

#### Modification: `crates/roko-acp/src/lib.rs`

Add:
```rust
pub mod acp_adapter;
```

### Done Criteria
```bash
grep -q 'pub struct AcpAdapter' crates/roko-acp/src/acp_adapter.rs
grep -q 'impl EventConsumer for AcpAdapter' crates/roko-acp/src/acp_adapter.rs
grep -q 'pub mod acp_adapter' crates/roko-acp/src/lib.rs
cargo check -p roko-acp
```
