# WebSocket and SSE Streaming

> Real-time event streaming via WebSocket and Server-Sent Events — live agent output, C-Factor updates, and Spectre creature state.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [05-http-api-roko-serve.md](./05-http-api-roko-serve.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §2, `roko-serve/src/routes/ws.rs`, `roko-serve/src/routes/sse.rs`, `roko-serve/src/event_bus.rs`

---

## Abstract

Roko provides two real-time streaming mechanisms: WebSocket for bidirectional communication and Server-Sent Events (SSE) for unidirectional event feeds. Both consume the same internal event bus, ensuring consistent event delivery regardless of transport. WebSocket endpoints support live agent interaction, while SSE provides a lightweight alternative for headless operation and monitoring dashboards.

The streaming architecture is designed for multiple concurrent consumers — the TUI, Web Portal, external dashboards, and CI systems can all subscribe to the same event stream simultaneously without interference.

---

## WebSocket Endpoints

### `/ws/events` — System-Wide Event Stream

All server events as a unified stream. Clients receive `ServerEvent` variants serialized as JSON:

```json
{"type": "agent_spawned", "agent_id": "rust-impl-01", "template": "code-implementer"}
{"type": "agent_output", "agent_id": "rust-impl-01", "text": "Analyzing auth module..."}
{"type": "gate_result", "plan": "01", "gate": "compile", "passed": true, "duration_ms": 4200}
{"type": "plan_phase", "plan": "01", "phase": "complete", "iterations": 2}
{"type": "cfactor_update", "value": 1.23, "delta": 0.04}
```

Clients can filter by event type using a subscription message on connect:

```json
{"subscribe": ["agent_output", "gate_result", "cfactor_update"]}
```

Without a subscription message, the client receives all events.

### `/ws/agent/:id` — Agent Output Stream

Live output from a specific agent. Includes:
- Raw text output (stdout/stderr)
- Tool call traces (tool name, arguments, result)
- Gate verdicts as they complete
- Daimon state changes (PAD vector updates, behavioral state transitions)
- Token usage per turn

This endpoint is bidirectional — clients can send messages to the agent:

```json
{"type": "inject", "content": "Focus on the error handling in auth.rs first"}
{"type": "pause"}
{"type": "resume"}
```

### `/ws/cfactor` — Live C-Factor Dashboard

Real-time collective intelligence metrics:

```json
{
  "cfactor": 1.23,
  "cscore": {
    "gate_pass": 0.94,
    "cost_efficiency": 0.82,
    "speed": 0.76,
    "first_try_rate": 0.88,
    "knowledge_growth": 0.65
  },
  "diagnostics": {
    "turn_taking_equality": 0.91,
    "knowledge_flow_rate": 0.73,
    "cross_domain_transfer": 0.45,
    "emergent_coordination": 0.62
  },
  "agent_contributions": [
    {"agent": "rust-impl-01", "contribution": 0.34},
    {"agent": "reviewer-01", "contribution": 0.28}
  ]
}
```

Updates push every time the C-Factor snapshot is refreshed (typically every 30 seconds during active work).

### `/ws/spectre/:id` — Live Spectre Creature State

Real-time Spectre visualization state for a specific agent. Provides the data needed for any renderer (TUI ASCII, Web Portal WebGL, or custom) to display the agent's Spectre creature:

```json
{
  "agent_id": "rust-impl-01",
  "behavioral_state": "Engaged",
  "pad": {"pleasure": 0.7, "arousal": 0.5, "dominance": 0.8},
  "body": {
    "shape_seed": "a3f2b1c4d5e6f789",
    "symmetry": "bilateral",
    "limb_count": 4,
    "domain_texture": "geometric"
  },
  "animation": {
    "breathing_rate": 0.7,
    "eye_state": "open",
    "glow_color": "#c77d8f",
    "glow_intensity": 0.8,
    "tendril_activity": 0.3
  },
  "knowledge": {
    "persistent": 23,
    "consolidated": 0,
    "working": 89,
    "transient": 30
  },
  "mesh_connections": ["reviewer-01", "researcher-01"],
  "pheromone_emission": {"type": "Wisdom", "intensity": 0.4}
}
```

**Status**: Endpoint exists as scaffold. Full Spectre state model not yet implemented.

---

## Server-Sent Events

### `/api/sse/events` — SSE Event Stream

The SSE endpoint provides the same event stream as `/ws/events` but over HTTP SSE. This is preferred for:
- Headless monitoring (curl, httpie)
- CI/CD pipelines
- Environments where WebSocket support is limited
- One-directional event consumption

```bash
curl -N -H "Authorization: Bearer roko_sk_..." \
  http://localhost:8080/api/sse/events
```

Output:
```
event: agent_spawned
data: {"agent_id":"rust-impl-01","template":"code-implementer"}

event: agent_output
data: {"agent_id":"rust-impl-01","text":"Analyzing auth module..."}

event: gate_result
data: {"plan":"01","gate":"compile","passed":true,"duration_ms":4200}
```

SSE respects the same event filtering as WebSocket — pass `?filter=agent_output,gate_result` as a query parameter.

---

## Reconnection Protocol

Both WebSocket and SSE support seamless reconnection:

1. On initial connect, the server assigns a sequence number starting at the latest event
2. Each event includes a monotonically increasing `seq` field
3. On reconnect, the client sends `{"resume_from": <last_seq>}` (WebSocket) or `Last-Event-ID: <last_seq>` (SSE)
4. The server replays events from the ring buffer starting at the requested sequence

The ring buffer retains the last 10,000 events by default. Events older than the buffer are lost — clients that disconnect for extended periods receive events from the buffer start, not from their last sequence.

---

## Event Coalescing

For high-frequency events (agent output, token counting), the server coalesces updates to prevent flooding:

- Agent output: batched into 100ms windows
- Token counts: accumulated and sent every 500ms
- Daimon state: sent on state transitions, not on every PAD vector update
- C-Factor: sent on snapshot refresh (every 30s)

This keeps WebSocket bandwidth reasonable even when multiple agents are producing output simultaneously.

---

## Backpressure and Flow Control

### Ring Buffer for Reconnection Replay

```rust
/// Fixed-capacity ring buffer for event replay on reconnection.
/// Retains last N events; older events are overwritten.
pub struct EventRingBuffer {
    events: Vec<(u64, ServerEvent)>,  // (seq, event) pairs
    head: usize,
    capacity: usize,                   // default: 10_000
    next_seq: u64,
}

impl EventRingBuffer {
    pub fn push(&mut self, event: ServerEvent) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        if self.events.len() < self.capacity {
            self.events.push((seq, event));
        } else {
            self.events[self.head] = (seq, event);
        }
        self.head = (self.head + 1) % self.capacity;
        seq
    }

    /// Replay events since `after_seq` for reconnecting clients.
    pub fn replay_from(&self, after_seq: u64) -> Vec<&(u64, ServerEvent)> {
        self.events.iter().filter(|(seq, _)| *seq > after_seq).collect()
    }
}
```

### Adaptive Event Priority

Under backpressure, low-priority events are coalesced or dropped while high-priority events are always delivered:

| Event Type | Priority | Coalesce Window | Drop Under Pressure? |
|---|---|---|---|
| `GateResult` | 10 | None | Never |
| `PlanPhaseChange` | 9 | None | Never |
| `CFactorUpdate` | 7 | 30s | Never |
| `AgentSpawned` | 6 | None | Never |
| `AgentOutput` | 3 | 100ms | Yes (batch) |

---

## Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_wraps_correctly() {
        let mut buf = EventRingBuffer::new(3);
        for i in 0..5 { buf.push(ServerEvent::CFactorUpdate { value: i as f64 }); }
        let replay = buf.replay_from(2);
        assert_eq!(replay.len(), 2);
    }

    #[test]
    fn ring_buffer_replay_from_zero_returns_all() {
        let mut buf = EventRingBuffer::new(100);
        for i in 0..10 { buf.push(ServerEvent::CFactorUpdate { value: i as f64 }); }
        assert_eq!(buf.replay_from(0).len(), 9);
    }

    #[test]
    fn sse_filter_respects_query_param() {
        let filter = parse_sse_filter("agent_output,gate_result");
        assert!(filter.accepts("agent_output"));
        assert!(filter.accepts("gate_result"));
        assert!(!filter.accepts("cfactor_update"));
    }

    #[test]
    fn ws_subscription_message_parses() {
        let msg = r#"{"subscribe":["agent_output","gate_result"]}"#;
        let sub: Subscription = serde_json::from_str(msg).unwrap();
        assert_eq!(sub.events.len(), 2);
    }
}
```

---

## Current Status and Gaps

**Built:**
- WebSocket route handler (`roko-serve/src/routes/ws.rs`)
- SSE route handler (`roko-serve/src/routes/sse.rs`)
- Event bus with publish-subscribe

**Not yet complete:**
- Bidirectional agent control via WebSocket (inject, pause, resume)
- Spectre state endpoint (requires Spectre implementation)
- Event coalescing (events sent raw without batching)
- Reconnection ring buffer

---

## Cross-references

- See [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for REST API
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre state model
- See [13-web-portal.md](./13-web-portal.md) for WebSocket consumption in the Portal
