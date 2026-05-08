# Relay Wire Protocol

> Depth for [11-CONNECTIVITY.md](../../v2/11-CONNECTIVITY.md). Frame-level specification for the WebSocket relay: connection lifecycle, frame types, envelope structure, sequencing, ring buffer semantics, and request/response bridging.

**Depends on**: [01-SIGNAL](../../v2/01-SIGNAL.md) (Signal/Pulse duality, Bus), [09-FEEDS](../../v2/09-FEEDS.md) (Feed registration, Pulse streams), [10-GROUPS](../../v2/10-GROUPS.md) (Group Bus partitions)

---

## 1. Relay Overview

The relay is a standalone WebSocket pub/sub service. It sits between agents, workspaces, and dashboards as the central rendezvous point for real-time communication.

**Carries:** agent presence, chain events, feed data, marketplace signals.

**Does NOT carry:** settlement transactions (go directly to chain RPC) or MCP tool calls (flow over MCP between agents and tool servers).

**Payload-opaque:** the relay routes by topic only. It never inspects `msg_type` or `payload`. Adding a new payload type requires zero relay changes.

**Stateless from the agent's perspective:** the relay holds in-memory state (directory, ring buffers, pending responses) for operational purposes, but the chain is the source of truth for identity, reputation, and financial state. If the relay restarts, agents reconnect and re-announce.

---

## 2. Connection Lifecycle

An agent connects via WebSocket upgrade at `/relay/agents/ws`. The lifecycle follows a strict sequence:

```
Agent                                    Relay
  |---- WebSocket upgrade GET ----------->|
  |<--- 101 Switching Protocols ----------|
  |---- Hello frame --------------------->|
  |<--- Ack { event: "hello" } ----------|
  |  (connection is now live)              |
  |---- Subscribe/Publish/Ping --------->|
  |<--- TopicMessage/Pong/Ack -----------|
```

The first frame MUST be a `hello`. Any other frame type causes the relay to send an error and close the connection.

### 2.1 Hello Frame

```rust
/// Initial agent hello frame. MUST be the first frame after connection opens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHello {
    pub agent_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub rest_endpoint: Option<String>,
    #[serde(default)]
    pub card: Option<Value>,
    #[serde(default)]
    pub card_uri: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}
```

```json
{
  "type": "hello",
  "agent_id": "coder-1",
  "name": "Coding Agent",
  "capabilities": ["code-review", "refactor", "test-gen"],
  "card": { "description": "Rust specialist", "protocols": ["mcp", "a2a"] }
}
```

On success the relay responds `{ "type": "ack", "event": "hello" }`. If the hello includes an inline `card`, the relay stores it and generates a `card_uri` at `/relay/cards/{agent_id}`.

### 2.2 Card Update

After the handshake, agents can update card metadata at any time:

```json
{ "type": "card", "card": { "hdc_fingerprint": "base64:...", "vitality": 0.92 } }
```

### 2.3 Keep-Alive

Application-level ping/pong (distinct from WebSocket-level ping/pong):

```
Agent --> { "type": "ping" }
Relay --> { "type": "pong" }
```

### 2.4 Disconnection

When the WebSocket closes, the relay:

1. Removes the agent from the directory.
2. Unsubscribes the agent from all topics via `bus.unsubscribe_all(agent_id)`.
3. Fails pending request/response messages with `"agent disconnected"`.
4. Unregisters all feeds the agent had registered, emitting `FeedUnregistered` events.
5. Broadcasts `AgentDisconnected` on the events channel.

Cleanup is guarded by a session ID (UUID assigned at registration). If the same `agent_id` reconnects before old cleanup runs, the session check prevents tearing down the new connection.

---

## 3. Frame Types

All frames are JSON text messages discriminated by a `type` field using `snake_case` naming.

### 3.1 Inbound Frames (Agent to Relay)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentInboundFrame {
    Hello(AgentHello),
    Card { card: Value, card_uri: Option<String> },
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Publish { topic: String, msg_type: String, payload: Value },
    RegisterFeed { feed: FeedDescriptor },
    UnregisterFeed { feed_id: String },
    Ping,
    Response { message_id: String, response: Value },
    Error { message_id: Option<String>, error: String },
}
```

Wire examples:

```json
{ "type": "subscribe", "topic": "isfr.rates" }
{ "type": "unsubscribe", "topic": "isfr.rates" }
{ "type": "publish", "topic": "isfr.rates", "msg_type": "composite_rate", "payload": { "bps": 620 } }
{ "type": "register_feed", "feed": { "feed_id": "eth-gas-trend", "topic": "feed.eth-gas-trend", "name": "Ethereum Gas Trend", "kind": "derived", "rate": "0.1hz" } }
{ "type": "unregister_feed", "feed_id": "eth-gas-trend" }
{ "type": "response", "message_id": "abc-123", "response": { "result": "ok" } }
{ "type": "error", "message_id": "abc-123", "error": "compile error" }
```

### 3.2 Outbound Frames (Relay to Agent)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayOutboundFrame {
    Ack { event: String },
    TopicMessage { topic: String, msg_type: String, payload: Value, publisher_id: Option<String>, seq: u64 },
    Message { message_id: String, message: Value },
    Pong,
    Error { message_id: Option<String>, error: String },
}
```

Wire examples:

```json
{ "type": "ack", "event": "subscribed:isfr.rates" }
{ "type": "ack", "event": "published:isfr.rates:42" }
{ "type": "topic_message", "topic": "isfr.rates", "msg_type": "composite_rate", "payload": { "bps": 620 }, "publisher_id": "keeper-1", "seq": 42 }
{ "type": "message", "message_id": "abc-123", "message": { "action": "code_review" } }
{ "type": "error", "error": "invalid frame: missing field `topic`" }
```

### 3.3 Ack Event Strings

| Ack event | Triggered by |
|---|---|
| `hello` | Successful handshake |
| `card` | Card metadata update |
| `subscribed:{topic}` | Topic subscription |
| `unsubscribed:{topic}` | Topic unsubscription |
| `published:{topic}:{seq}` | Publish (includes assigned seq) |
| `feed_registered` | Feed registration |
| `feed_unregistered` | Feed unregistration |

---

## 4. Topic Message Envelope

Every pub/sub message uses a `TopicEnvelope`. This is the internal representation; agents receive it serialized as a `TopicMessage` outbound frame.

```rust
/// Internal representation of a published topic message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicEnvelope {
    pub topic: String,
    pub msg_type: String,
    pub payload: Value,
    pub publisher_id: Option<String>,
    pub seq: u64,
    pub timestamp_ms: i64,
}
```

```json
{
  "seq": 42,
  "timestamp_ms": 1715000000000,
  "topic": "isfr.rates",
  "publisher_id": "keeper-1",
  "msg_type": "composite_rate",
  "payload": { "bps": 620, "components": { "eth_staking": 340, "lst_spread": 180 } }
}
```

### 4.1 Field Semantics

| Field | Type | Assigned by | Description |
|---|---|---|---|
| `seq` | `u64` | Relay | Global monotonic counter starting at 1. Atomically incremented on every publish across all topics. |
| `timestamp_ms` | `i64` | Relay | Unix milliseconds from the relay clock (not agent-provided). |
| `topic` | `String` | Agent | Dot-separated name. Convention: `{domain}.{specifier}`. |
| `publisher_id` | `Option<String>` | Relay | Agent ID set from the authenticated connection, not self-reported. Prevents spoofing. |
| `msg_type` | `String` | Agent | Application-level discriminant. Opaque to relay. |
| `payload` | `Value` | Agent | Arbitrary JSON. Opaque to relay. |

### 4.2 Topic Naming Convention

```
isfr.rates              ISFR rate feed
chain.31337             Chain events for chain ID 31337
chain.1                 Ethereum mainnet
feed.eth-gas-trend      Named data feed
group.abc123            Group Bus partition
agent.coder-1           Agent lifecycle events
system.health           Relay system health
```

The relay does not enforce naming. These are application-level agreements.

---

## 5. Ring Buffer and Replay

### 5.1 Per-Topic Ring Buffer

The relay maintains a bounded ring buffer per topic. Default capacity: 128 entries. On overflow, the oldest entry is evicted.

```rust
pub struct TopicBus {
    subscriptions: RwLock<HashMap<String, Vec<String>>>,
    rings: RwLock<HashMap<String, VecDeque<TopicEnvelope>>>,
    seq: AtomicU64,
    ring_capacity: usize,   // default: 128
}
```

The capacity is intentionally small. The relay is ephemeral transport; durable state belongs in Store.

### 5.2 Global Monotonic Sequence

A single `AtomicU64` counter shared across all topics provides total ordering:

- **Gap detection:** seq 40 then seq 43 means two messages were missed.
- **Replay ordering:** ring buffer contents are already in seq order.
- **Cross-topic ordering:** messages on different topics can be interleaved by seq.

```rust
pub fn publish(&self, mut envelope: TopicEnvelope) -> (u64, Vec<String>) {
    let seq = self.seq.fetch_add(1, Ordering::Relaxed);
    envelope.seq = seq;
    // Store in ring, evicting oldest if at capacity.
    // Return (seq, list of subscriber agent_ids).
}
```

### 5.3 Replay on Subscribe

When an agent subscribes, the relay returns the current ring contents as replay before the subscribe ack:

```
Agent                                    Relay
  |---- Subscribe { "isfr.rates" } ----->|
  |<--- TopicMessage { seq: 38 } --------|  (replay)
  |<--- TopicMessage { seq: 39 } --------|  (replay)
  |<--- TopicMessage { seq: 41 } --------|  (replay)
  |<--- Ack { "subscribed:isfr.rates" } -|
  |<--- TopicMessage { seq: 44 } --------|  (live)
```

### 5.4 Future: Resume After Reconnection

A planned `resume_after` field will allow reconnecting agents to specify their last-seen seq:

```json
{ "type": "subscribe", "topic": "isfr.rates", "resume_after": 41 }
```

Until implemented, agents must handle idempotent processing of replayed messages.

---

## 6. Request/Response Bridge

`POST /relay/messages` bridges HTTP request/response to WebSocket-connected agents. External callers (dashboards, services) can send a message to a specific agent and receive a response without maintaining their own WebSocket.

### 6.1 Flow

```
HTTP Client                  Relay                      Agent (WebSocket)
    |-- POST /relay/messages ->|                              |
    |   { agent_id, message }  |-- Message { message_id } -->|
    |                          |<-- Response { message_id } --|
    |<-- 200 { response }   --|                              |
```

### 6.2 Timeout

Default: 15 seconds. Configurable per-request via `timeout_ms` (clamped to `[1ms, 60s]`):

```rust
pub const MAX_MESSAGE_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_MESSAGE_TIMEOUT_MS: u64 = 15_000;

impl RelayMessageRequest {
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
            .unwrap_or(DEFAULT_MESSAGE_TIMEOUT_MS)
            .clamp(1, MAX_MESSAGE_TIMEOUT_MS)
    }
}
```

### 6.3 Oneshot Channel Pattern

Internally, a `tokio::sync::oneshot` channel is created per request. The sender is stored in `HashMap<String, PendingResponse>` keyed by `message_id`. The receiver is awaited with `tokio::time::timeout`. When the agent sends a `Response` or `Error` frame referencing that `message_id`, the relay resolves the oneshot.

### 6.4 Error Cases

| Condition | HTTP status | Error |
|---|---|---|
| Agent not in directory | 404 | `"unknown agent"` |
| Agent connection dead | 502 | `"agent connection is not writable"` |
| Agent returns error frame | 502 | Agent's error string |
| Agent does not respond in time | 504 | `"agent response timed out"` |

---

## 7. Event Broadcasting

The relay broadcasts lifecycle events on `/relay/events/ws` for dashboards and monitoring.

### 7.1 Channel

Events flow via `tokio::sync::broadcast` channel (capacity 256). Slow consumers receive a `{ "type": "lagged", "skipped": N }` notification.

### 7.2 Event Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayEvent {
    AgentConnected { agent: ConnectedAgent },
    AgentDisconnected { agent_id: String },
    CardUpdated { agent_id: String, card_uri: String },
    MessageDelivered { agent_id: String, message_id: String },
    MessageResponded { agent_id: String, message_id: String },
    AgentError { agent_id: String, message_id: Option<String>, error: String },
    WorkspaceConnected { workspace: ConnectedWorkspace },
    WorkspaceDisconnected { workspace_id: String },
    WorkspaceHeartbeat { workspace_id: String, agents_count: u32 },
    FeedRegistered { agent_id: String, feed: FeedDescriptor },
    FeedUnregistered { agent_id: String, feed_id: String },
}
```

The events WebSocket is read-only from the relay's perspective. Incoming frames from the client are ignored except close frames.

### 7.3 Workspace Events

Workspaces (roko-serve instances) register separately from agents via `POST /relay/workspaces/register` and send periodic heartbeats. Stale workspaces are expired when heartbeat age exceeds a configurable threshold:

```rust
pub struct WorkspaceHello {
    pub workspace_id: String,
    pub name: Option<String>,
    pub url: String,
    pub version: Option<String>,
    pub owner_wallet: Option<String>,
    pub agents_count: u32,
}
```

---

## 8. HTTP API Surface

All endpoints are unauthenticated in the current implementation (auth planned for Phase 2).

### 8.1 Core Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/relay/health` | Returns `"ok"` (200) |
| `GET` | `/relay/agents` | List connected agents (sorted by `agent_id`) |
| `GET` | `/relay/cards/{id}` | Agent card JSON (404 if none) |
| `POST` | `/relay/messages` | Forward HTTP request to WebSocket agent (see S6) |
| `GET` | `/relay/agents/ws` | WebSocket upgrade: agent connection |
| `GET` | `/relay/events/ws` | WebSocket upgrade: event stream |

### 8.2 Workspace Directory

| Method | Path | Description |
|---|---|---|
| `GET` | `/relay/workspaces` | List connected workspaces |
| `POST` | `/relay/workspaces/register` | Register a workspace |
| `POST` | `/relay/workspaces/{id}/heartbeat` | Workspace heartbeat |
| `DELETE` | `/relay/workspaces/{id}` | Unregister a workspace |

### 8.3 Feed Registry

| Method | Path | Description |
|---|---|---|
| `GET` | `/relay/feeds` | List all feeds across all agents |
| `GET` | `/relay/feeds/{agent_id}` | List feeds for a specific agent |

### 8.4 Topic Introspection

| Method | Path | Description |
|---|---|---|
| `GET` | `/relay/topics` | List active topics with subscriber counts |
| `GET` | `/relay/topics/{topic}/messages` | Recent ring buffer messages (`?limit=N`, default 50, max 200) |
| `GET` | `/relay/topics/{topic}/subscribers` | Subscriber count for a topic |

The messages endpoint reads from the ring buffer without creating a subscription. It is for debugging, not production consumption.

---

## 9. Crate Mapping

### 9.1 Relay Server (`apps/agent-relay/`)

| File | Responsibility |
|---|---|
| `src/main.rs` | Entry point, axum server startup |
| `src/lib.rs` | Router, REST handlers, WebSocket upgrade, frame dispatch loop |
| `src/protocol.rs` | `AgentInboundFrame`, `RelayOutboundFrame`, `TopicEnvelope`, `AgentHello`, `ConnectedAgent`, `RelayEvent`, `FeedDescriptor`, `WorkspaceHello` |
| `src/state.rs` | `RelayState` (agent directory, card store, pending responses, workspace directory, feed registry) |
| `src/bus.rs` | `TopicBus` (subscriptions, ring buffers, sequence counter) |
| `src/chain_watcher.rs` | Publishes chain events to the topic bus |

### 9.2 Relay Client (`crates/roko-agent-server/`)

| File | Responsibility |
|---|---|
| `src/features/relay_client.rs` | `RelayHandle` for connecting as a client, sending frames, subscribing |
| `src/features/relay_subscriber.rs` | `RelaySubscriber` (high-level pub/sub API), `ISFRTopicAdapter` (bridges relay messages to ISFR feed Pulses) |
| `src/registration.rs` | Workspace registration on startup |

### 9.3 Dependency Direction

```
apps/agent-relay           (standalone binary, no roko-* dependency)
     |
     v  (WebSocket protocol)
crates/roko-agent-server   (relay client, depends on roko-core for Pulse types)
     |
     v
crates/roko-serve          (registers workspace with relay on startup)
```

The relay binary has no dependency on any `roko-*` crate. It is a generic WebSocket pub/sub server. All Roko-specific semantics (ISFR feeds, Pulse conversion, Signal graduation) happen on the client side in `roko-agent-server`.
