## Batch P3B: SseAdapter + REST Panel Endpoints

### Write Scope
- **CREATE**: `crates/roko-serve/src/adapters.rs`
- **MODIFY**: `crates/roko-serve/src/lib.rs` (add `pub mod adapters;`)

### Dependencies
- P0B (EventConsumer trait)
- P0C (RuntimeEvent bus)

### DO NOT
- Modify any other files (especially not existing routes)
- Add Cargo.toml dependencies
- Create a new crate
- Duplicate existing SSE infrastructure

### Task

Create `SseAdapter` — implements `EventConsumer` to bridge RuntimeEvents to SSE clients.
Also provides helper functions that can be wired into the existing route infrastructure.

#### File: `crates/roko-serve/src/adapters.rs`

```rust
//! SseAdapter — bridges RuntimeEvent → Server-Sent Events.
//!
//! Implements EventConsumer to receive workflow events and forwards them
//! to connected SSE clients as JSON event data.

use roko_core::foundation::EventConsumer;
use roko_core::runtime_event::RuntimeEvent;
use serde::Serialize;
use tokio::sync::broadcast;

/// JSON-serializable event for SSE clients.
#[derive(Debug, Clone, Serialize)]
pub struct SseEvent {
    /// Event kind (matches RuntimeEvent::kind())
    pub kind: String,
    /// Run ID
    pub run_id: String,
    /// Event-specific data
    pub data: serde_json::Value,
}

/// Adapter that translates RuntimeEvents into SSE-compatible JSON events.
///
/// Maintains a broadcast channel that SSE endpoint handlers can subscribe to.
pub struct SseAdapter {
    sender: broadcast::Sender<SseEvent>,
}

impl SseAdapter {
    /// Create a new SseAdapter with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Subscribe to the SSE event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.sender.subscribe()
    }

    /// Convert a RuntimeEvent to an SseEvent.
    fn to_sse_event(event: &RuntimeEvent) -> SseEvent {
        let run_id = event.run_id().to_string();
        let kind = event.kind().to_string();

        let data = match event {
            RuntimeEvent::WorkflowStarted { template, prompt, .. } => {
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
            RuntimeEvent::AgentSpawned { agent_id, role, model, .. } => {
                serde_json::json!({
                    "agent_id": agent_id,
                    "role": role,
                    "model": model,
                })
            }
            RuntimeEvent::AgentOutput { agent_id, chunk, .. } => {
                serde_json::json!({
                    "agent_id": agent_id,
                    "chunk": chunk,
                })
            }
            RuntimeEvent::AgentCompleted { agent_id, tokens_used, cost_usd, .. } => {
                serde_json::json!({
                    "agent_id": agent_id,
                    "tokens_used": tokens_used,
                    "cost_usd": cost_usd,
                })
            }
            RuntimeEvent::AgentFailed { agent_id, error, .. } => {
                serde_json::json!({
                    "agent_id": agent_id,
                    "error": error,
                })
            }
            RuntimeEvent::GateStarted { gate_name, rung, .. } => {
                serde_json::json!({
                    "gate_name": gate_name,
                    "rung": rung,
                })
            }
            RuntimeEvent::GatePassed { gate_name, duration_ms, .. } => {
                serde_json::json!({
                    "gate_name": gate_name,
                    "duration_ms": duration_ms,
                })
            }
            RuntimeEvent::GateFailed { gate_name, output, duration_ms, .. } => {
                serde_json::json!({
                    "gate_name": gate_name,
                    "output": output,
                    "duration_ms": duration_ms,
                })
            }
            RuntimeEvent::FeedbackRecorded { kind: fk, summary, .. } => {
                serde_json::json!({
                    "feedback_kind": fk,
                    "summary": summary,
                })
            }
            RuntimeEvent::StateCheckpointed { path, .. } => {
                serde_json::json!({
                    "path": path,
                })
            }
        };

        SseEvent {
            kind,
            run_id,
            data,
        }
    }
}

impl EventConsumer for SseAdapter {
    fn consume(&self, event: &RuntimeEvent) {
        let sse_event = Self::to_sse_event(event);
        // Non-blocking — if no subscribers, the event is dropped
        let _ = self.sender.send(sse_event);
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
}
```

#### Modification: `crates/roko-serve/src/lib.rs`

Add:
```rust
pub mod adapters;
```

### Done Criteria
```bash
grep -q 'pub struct SseAdapter' crates/roko-serve/src/adapters.rs
grep -q 'impl EventConsumer for SseAdapter' crates/roko-serve/src/adapters.rs
cargo check -p roko-serve
```
