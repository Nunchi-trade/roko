# Checklist: Agent messaging — `POST /api/agents/{id}/message` + WS streaming

**Priority**: P0 — replaces OpenRouter hack in Ask panel
**Estimated LOC**: ~120 lines (roko-serve) + ~30 lines (mirage-rs)
**Source**: `workspace/sdb/agent-messaging-architecture.md`, `workspace/sdb/prds/ask-prd.md`, [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)

## Problem

The dashboard Ask panel currently calls OpenRouter directly from the browser. This is architecturally wrong — the LLM call should happen inside the agent (via roko-serve), not in the dashboard. Agents need to query InsightStore, use their own context/memory, and provide reasoning traces + citations.

## What already exists

- `crates/roko-serve/src/routes/run.rs`: `POST /api/run` spawns a one-shot `run_once()`. Returns `run_id`. Status via `GET /api/run/{id}/status`.
- `crates/roko-serve/src/routes/ws.rs`: WebSocket at `/ws` streams `ServerEvent` payloads from ring buffer.
- `crates/roko-serve/src/routes/sse.rs`: SSE endpoint for event streaming.
- `crates/roko-serve/src/events.rs`: `ServerEvent` enum with `RunStarted`, `RunCompleted`, `Error` variants.

## Approach

Extend the existing `/api/run` pattern with agent targeting. NOT building a new messaging system from scratch. The run handler already spawns execution and emits events to the WS ring buffer.

## Files to modify

### 1. `crates/roko-serve/src/routes/agents.rs`

Current routes (line 15):
```rust
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents))
        .route("/agents/{id}", get(get_agent))
        ...
}
```

- [ ] Add message endpoint: `.route("/agents/{id}/message", post(send_message))`

- [ ] Add handler:
```rust
#[derive(Deserialize)]
struct MessageRequest {
    content: String,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    response_mode: Option<String>, // "stream" | "complete"
}

/// `POST /api/agents/{id}/message` — send a message to a specific agent.
/// Internally creates a run targeting the specified agent.
async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();

    // Prefix prompt with agent targeting directive
    let prompt = format!("[agent:{}] {}", agent_id, req.content);

    let handle = tokio::spawn({
        let run_id = run_id.clone();
        let agent_id = agent_id.clone();
        async move {
            bus.publish(ServerEvent::RunStarted {
                run_id: run_id.clone(),
                prompt: prompt.clone(),
            });
            match runtime.run_once(&workdir, &prompt).await {
                Ok(result) => {
                    bus.publish(ServerEvent::RunCompleted {
                        run_id,
                        success: result.success,
                    });
                }
                Err(e) => {
                    bus.publish(ServerEvent::Error {
                        message: format!("agent {agent_id} message failed: {e}"),
                    });
                }
            }
        }
    });

    state.runs.lock().insert(run_id.clone(), RunHandle { handle, status: OperationStatus::Running });

    Ok(Json(json!({
        "run_id": run_id,
        "agent_id": agent_id,
        "conversation_id": req.conversation_id,
        "status": "running",
    })))
}
```

### 2. `crates/roko-serve/src/events.rs`

- [ ] Add `AgentOutput` variant to `ServerEvent`:
```rust
AgentOutput {
    run_id: String,
    agent_id: String,
    chunk: String,
    done: bool,
    metadata: Option<Value>, // entries_used, reasoning_trace, cost on final chunk
},
```

### 3. `apps/mirage-rs/src/http_api/agent.rs`

- [ ] Extend `get_agent_heartbeat` response with `busy` field:
```rust
// Add to heartbeat response JSON:
"busy": chain.task_store.list(
    Some(TaskState::InProgress), None,
    Some(&id), 1, 0
).1 > 0,
```

## Response shapes

### `POST /api/agents/{id}/message`
```json
{
  "run_id": "uuid-here",
  "agent_id": "golem-alpha-7f",
  "conversation_id": "conv-123",
  "status": "running"
}
```

### WS event (streamed via existing `/ws`)
```json
{
  "type": "agent_output",
  "run_id": "uuid-here",
  "agent_id": "golem-alpha-7f",
  "chunk": "Based on the InsightStore, ISFR is currently...",
  "done": false
}
```

### Final WS event
```json
{
  "type": "agent_output",
  "run_id": "uuid-here",
  "agent_id": "golem-alpha-7f",
  "chunk": "",
  "done": true,
  "metadata": {
    "entries_used": ["insight-id-1", "insight-id-2"],
    "reasoning_trace": [...],
    "tokens_used": 1200,
    "cost_usd": 0.02
  }
}
```

## Testing

- [ ] `POST /api/agents/{id}/message` with valid agent → returns run_id
- [ ] WS subscriber receives `agent_output` events for the run
- [ ] `GET /api/run/{run_id}/status` works for message-initiated runs
- [ ] `GET /api/agents/{id}/heartbeat` includes `busy` field

## Dashboard impact

Replace in Ask panel:
```typescript
// OLD: direct OpenRouter call
const response = await fetch('https://openrouter.ai/...', { body: message });

// NEW: agent message via roko-serve
const { run_id } = await sendAgentMessage(agentId, message);
// Listen on WS for agent_output events with matching run_id
```
