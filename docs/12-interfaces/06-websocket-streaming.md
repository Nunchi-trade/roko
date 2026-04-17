# WebSocket and SSE Streaming

> Real-time StateHub projection streaming via WebSocket and Server-Sent Events for live agent output, c-factor dashboards, gate views, and Spectre state.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [05-http-api-roko-serve.md](./05-http-api-roko-serve.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §2, `roko-serve/src/routes/ws.rs`, `roko-serve/src/routes/sse.rs`, `roko-serve/src/event_bus.rs`

---

## Abstract

Roko provides two real-time streaming mechanisms: WebSocket for bidirectional communication and Server-Sent Events (SSE) for unidirectional feeds. Under REF26, both are transport bindings for StateHub projections rather than raw internal event fanout. WebSocket endpoints support live agent interaction and projection subscription; SSE provides a lightweight alternative for headless operation and monitoring dashboards.

The streaming architecture is designed for multiple concurrent consumers. The TUI, Web Portal, external dashboards, and CI systems should all subscribe to the same named projections and see the same typed `State` plus `Delta` sequence, with reconnection handled by per-projection cursors rather than transport-local sequence IDs.

REF23 turns that into a product rule rather than an implementation detail: CLI, TUI, Chat, and Web all render the same progress stream, so `watch` is a shared surface capability rather than a one-off endpoint feature. See [21-user-ux-running-agents.md](./21-user-ux-running-agents.md) and [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md).

REF26 adds the missing contract for that stream: `watch` should usually attach to a projection such as `active_tasks`, `agent_trails`, `gate_pipeline`, or `cohort_health`, with current state fetched first and later updates streamed as deltas. See [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md).

---

## Canonical `watch` Stream

The shared `watch` experience should surface the same categories regardless of transport, but the wire contract is projection-first rather than event-first:

| Category | What the user sees |
|---|---|
| Token streaming | Partial model output with a visible cursor while generation is active |
| Tool call banners | Short, literal progress banners such as file reads, commands, and external calls |
| Gate feedback | Immediate pass/fail/pending updates with counts and next-step hints |
| Episode and heuristic events | Episode creation, heuristic application, challenge, or calibration signals |
| Checkpoint prompts | Permission or ambiguity checkpoints that pause work pending user choice |

In practice:

- `active_tasks` carries task progress, ETAs, and state transitions.
- `agent_trails` carries token chunks, tool banners, and current action context.
- `gate_pipeline` carries rung status and pass/fail counts.
- `recent_episodes` carries newly completed or resumed work.

The transport may differ by surface, but the semantics must not. A user switching from CLI to TUI or Web mid-task should see the same session continue from the same projection cursor and the same folded state.

---

## Projection Stream Endpoints

### Canonical path

The canonical external shape is a projection stream, not a raw event tap:

```text
GET /projections/:name
GET /projections/:name/stream
```

The server answers `GET /projections/:name` with the current typed `State`, then upgrades `GET /projections/:name/stream` to WebSocket or SSE and emits `Delta` envelopes for the same projection. One-off WebSocket families can survive as compatibility aliases, but the public contract should resolve to the same projection registry.

### Envelope

Every streamed message should carry the same projection envelope:

```json
{
  "projection": "cohort_health",
  "cursor": "0x1a2b...",
  "kind": "state",
  "timestamp": "2026-04-16T12:00:00Z",
  "payload": {
    "c_factor": 1.23,
    "agent_roster": [],
    "delivery_rate": 0.98
  }
}
```

Later messages switch `kind` to `delta` and send the projection-specific delta payload. This is the transport-level contract that lets the TUI, Web UI, Slack adapters, and external dashboards share code and resume logic.

### Filters

Clients may scope subscriptions by the same server-side filters defined by StateHub:

```json
{
  "projection": "agent_trails",
  "filter": {"user": "me"},
  "cursor": "0x1a2a..."
}
```

Allowed filters should include tenant, role, user, lineage, topic, and time range. The client does not subscribe to everything and filter locally.

### `/ws/agent/:id` — Agent Output Stream

This endpoint is best treated as a convenience alias over `agent_trails` with an agent-specific filter. It remains useful for bidirectional control because the client may stream state and send commands on the same socket.

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

This endpoint is best treated as a convenience alias over `cohort_health`. The durable contract is the `cohort_health` projection; `/ws/cfactor` is just a narrower transport affordance.

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

This endpoint can remain renderer-friendly, but it should still fold through StateHub so TUI and Web renderers share the same cursor, auth rules, and replay semantics.

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

### `/projections/:name/stream` — SSE Projection Stream

The SSE binding should provide the same projection stream as WebSocket but over HTTP SSE. This is preferred for:
- Headless monitoring (curl, httpie)
- CI/CD pipelines
- Environments where WebSocket support is limited
- One-directional event consumption

```bash
curl -N -H "Authorization: Bearer roko_sk_..." \
  http://localhost:8080/projections/gate_pipeline/stream
```

Output:
```
event: state
data: {"projection":"gate_pipeline","cursor":"0x10","kind":"state","payload":{"rungs":[]}}

event: delta
data: {"projection":"gate_pipeline","cursor":"0x11","kind":"delta","payload":{"rung":"compile","passed":true,"duration_ms":4200}}
```

SSE respects the same projection filtering as WebSocket. A browser or dashboard can query the current projection state on mount, then attach to the stream and fold deltas as they arrive.

---

## Reconnection Protocol

Both WebSocket and SSE support seamless reconnection:

1. On initial connect, the server emits a per-projection cursor with the initial state.
2. Each later delta includes a monotonically increasing cursor for that projection.
3. On reconnect, the client sends the last cursor it has seen for that projection.
4. The server replays deltas from the projection log or rehydrates state and resumes from the nearest retained cursor.

The replay window is a projection concern, not just a raw socket ring buffer. If a cursor is too old, the server should send a fresh `state` envelope and continue from there.

These cursors are also the continuity mechanism for REF23's named sessions and shareable replays. A session handoff between surfaces should reuse the same stream position rather than starting a fresh observer window.

---

## Event Coalescing

For high-frequency updates, the server coalesces deltas rather than flooding clients with every raw Pulse:

- `agent_trails` token chunks may batch into short windows.
- `active_tasks` progress deltas may collapse to the latest progress per task.
- `gate_pipeline` should preserve rung transitions exactly.
- `cohort_health` may publish on metric refresh rather than every intermediate calculation.

This keeps WebSocket bandwidth reasonable even when multiple agents are producing output simultaneously.

---

## Backpressure and Flow Control

### Projection Cursor Log for Reconnection Replay

```rust
/// Fixed-capacity replay log for one projection.
/// Retains last N deltas; older entries are overwritten.
pub struct ProjectionReplayLog<D> {
    deltas: Vec<(Cursor, D)>,
    head: usize,
    capacity: usize,
    next_cursor: Cursor,
}

impl<D: Clone> ProjectionReplayLog<D> {
    pub fn push(&mut self, delta: D) -> Cursor {
        let cursor = self.next_cursor;
        self.next_cursor = cursor.next();
        if self.deltas.len() < self.capacity {
            self.deltas.push((cursor, delta));
        } else {
            self.deltas[self.head] = (cursor, delta);
        }
        self.head = (self.head + 1) % self.capacity;
        cursor
    }

    pub fn replay_from(&self, after: Cursor) -> Vec<&(Cursor, D)> {
        self.deltas.iter().filter(|(cursor, _)| *cursor > after).collect()
    }
}
```

### Adaptive Delta Priority

Under backpressure, low-priority projection deltas are coalesced or dropped while high-priority transitions are always delivered:

| Projection / delta kind | Priority | Coalesce Window | Drop Under Pressure? |
|---|---|---|---|
| `gate_pipeline` rung transition | 10 | None | Never |
| `active_tasks` phase transition | 9 | None | Never |
| `cohort_health` metric update | 7 | 30s | Never |
| `recent_episodes` append | 6 | None | Never |
| `agent_trails` token chunk | 3 | 100ms | Yes (batch) |

---

## Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_log_wraps_correctly() {
        let mut buf = ProjectionReplayLog::new(3);
        for i in 0..5 { buf.push(CohortHealthDelta::MetricRefresh { seq: i }); }
        let replay = buf.replay_from(Cursor::from(2));
        assert_eq!(replay.len(), 2);
    }

    #[test]
    fn projection_log_replay_from_zero_returns_all() {
        let mut buf = ProjectionReplayLog::new(100);
        for i in 0..10 { buf.push(CohortHealthDelta::MetricRefresh { seq: i }); }
        assert_eq!(buf.replay_from(Cursor::from(0)).len(), 9);
    }

    #[test]
    fn projection_filter_respects_query_param() {
        let filter = parse_projection_filter("user:me,topic:gate.verdict.emitted");
        assert!(filter.accepts_user("me"));
        assert!(filter.accepts_topic("gate.verdict.emitted"));
        assert!(!filter.accepts_topic("agent.msg.chunk"));
    }

    #[test]
    fn projection_subscription_message_parses() {
        let msg = r#"{"projection":"gate_pipeline","filter":{"user":"me"},"cursor":"0x11"}"#;
        let sub: ProjectionSubscription = serde_json::from_str(msg).unwrap();
        assert_eq!(sub.projection, "gate_pipeline");
    }
}
```

---

## Current Status and Gaps

**Built:**
- WebSocket route handler (`roko-serve/src/routes/ws.rs`)
- SSE route handler (`roko-serve/src/routes/sse.rs`)
- Bus-backed transport scaffolding

**Not yet complete:**
- StateHub-backed projection registry and typed envelopes
- Query-plus-stream projection endpoints
- Bidirectional agent control via WebSocket (inject, pause, resume)
- Spectre state endpoint (requires Spectre implementation)
- Projection-delta coalescing
- Projection cursor replay

---

## Cross-References

- See [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for REST API
- See [22-statehub-projection-layer.md](./22-statehub-projection-layer.md) for the projection contract
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre state model
- See [13-web-portal.md](./13-web-portal.md) for WebSocket consumption in the Portal
- See [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) for `Pulse`, `Bus`, `StateHub`, and `projection` vocabulary
- See [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) for the canonical REF26 proposal
