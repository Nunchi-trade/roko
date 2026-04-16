# roko-agent-server

Per-agent HTTP sidecar. Every Roko agent you register with the control plane
runs one of these — a small Axum server that exposes the agent's behaviour
over REST + WebSocket. The control plane (`roko-serve`) fans discovery and
messaging requests out to each sidecar.

## Why a sidecar per agent

Roko agents are long-lived processes with their own state (sessions,
prediction history, knowledge caches). Colocating HTTP surface with process
state avoids cross-process coordination on every request and lets each agent
evolve its own schema without version-locking the whole fleet.

## Endpoints

### Always-on

| Method | Path | Response shape |
|--------|------|----------------|
| `GET` | `/health` | `{"status": "ok", "agent_id", "version"}` |
| `GET` | `/capabilities` | `{"features": [...], "version": ...}` |
| `GET` | `/stats` (protected) | counters for message/turn/error, memory footprint |

### Messaging (feature-gated)

| Method | Path | What it does |
|--------|------|--------------|
| `POST` | `/message` | One turn through the configured `LlmBackend`. Returns full response with usage, session, finish reason. |
| `GET` | `/stream` | WebSocket streaming turn. Server emits `{chunk}`, `{reasoning}`, `{tool_call}`, `{usage}`, and terminal `{done: true}` frames. |

`POST /message` wire shape:

```json
// request
{
  "prompt": "Analyse the latest gate failures",
  "context": { "extra": { "thread": "abc" } }
}

// 200 OK response
{
  "response": "The last 5 failures share a pattern...",
  "reasoning": null,
  "usage": { "prompt_tokens": 421, "completion_tokens": 183 },
  "session": { "session_id": "sess-1", "thread_id": null, "conversation_id": null },
  "finish_reason": "stop",
  "engram_id": "engram-f2...",
  "context": { "extra": { "thread": "abc" } }
}

// 503 — no dispatcher configured on this agent
{ "error": "agent has no configured dispatcher" }

// 502 — backend returned an error
{ "error": "dispatch failed: <backend error>" }
```

`GET /stream` (WebSocket) frame shapes:

```json
{ "chunk": "Hello", "done": false }
{ "reasoning": "Analysing inputs...", "done": false }
{ "tool_call": { "index": 0, "id_delta": null, "name_delta": "shell", "arguments_delta": "{\"cmd\":\"ls\"}" }, "done": false }
{ "usage": { "prompt_tokens": 421, "completion_tokens": 42 }, "done": false }
{ "done": true, "session": {...}, "usage": {...}, "finish_reason": "stop" }

// error termination
{ "error": "dispatch failed: ...", "done": true }
```

### Predictions (feature-gated)

| Method | Path | What it does |
|--------|------|--------------|
| `GET` | `/predictions` | List prediction sessions known to this agent |
| `POST` | `/predictions` | Record a new prediction |
| `GET` | `/predictions/residuals` | Recent prediction residuals (actual vs predicted) |
| `GET` | `/predictions/{id}` | Single prediction detail |

### Research (feature-gated)

`POST /research` — run a scoped research task inside this agent, returning
a bundle of citations and a synthesis.

### Tasks (feature-gated)

| Method | Path | What it does |
|--------|------|--------------|
| `GET` | `/tasks` | List agent-owned tasks |
| `POST` | `/tasks/{id}/accept` | Accept an assigned task |
| `POST` | `/tasks/{id}/complete` | Complete with typed `Artifact` (file diff, knowledge entry, signal ref) |

## Running standalone

Normally a sidecar is started by the agent registration flow, but for
development you can spin one up directly:

```rust
use roko_agent_server::{AgentServer, AgentState};
use std::sync::Arc;

let state = AgentState::new(
    "my-agent".into(),
    None,
    "0.1.0".into(),
    vec!["messaging".into()],
    /* knowledge */ None,
    /* llm_backend */ Some(backend),
    /* registration */ None,
);

let server = AgentServer::builder()
    .bind("127.0.0.1:7788")
    .state(Arc::new(state))
    .with_dispatcher(dispatcher)
    .build()
    .await?;

server.serve().await?;
```

## Feature flags on a `Capability` list

At build time the `AgentState` carries a `Vec<String>` of enabled features.
A feature name in the list turns on its route group. Default build enables:
`messaging`, `predictions`, `research`, `tasks`.

Absent features return `404`, not `503`. `503` is reserved for *configured
but misconfigured* paths (e.g. messaging enabled, dispatcher missing).

## Aggregation via the control plane

When `roko-serve` is running with the aggregator, every request to
`/api/agents/{id}/*` is proxied to the matching sidecar by `agent_id`.
Discovery (`GET /api/agents`) returns the union of all sidecar health +
capability reports. This lets a dashboard call a single base URL while
each agent remains independent.

## Testing

```bash
cargo test -p roko-agent-server --lib
cargo test -p roko-agent-server --tests
cargo clippy -p roko-agent-server --no-deps -- -D warnings
```

The integration tests (added in T19) boot a real Axum server on a random
port, hit it via `reqwest`, and verify every branch of `/message` and
`/stream` against a mock `LlmBackend`.

## What it is not

- **Not a control plane.** It has no knowledge of other agents, plans, or
  PRDs. All cross-agent coordination lives in `roko-serve`.
- **Not a scheduler.** Task dispatch into the sidecar is the caller's job;
  this crate only exposes the REST surface.

## Related docs

- `crates/roko-serve/README.md` — the control plane that aggregates
  sidecars and exposes a unified API
- Top-level `README.md` — architecture diagram + end-to-end setup
