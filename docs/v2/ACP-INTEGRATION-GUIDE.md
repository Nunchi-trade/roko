# ACP Integration Guide

ACP (Agent Client Protocol) is the JSON-RPC 2.0 protocol Roko uses to integrate
with editors and IDEs. Running `roko acp` starts a stdio server; the editor sends
JSON-RPC messages on stdin and receives responses and streaming notifications on
stdout.

This document covers everything you need to build an editor plugin.

---

## Overview

Roko implements ACP spec version **0.12.2**, protocol version **1**.

The server speaks JSON-RPC 2.0 over newline-delimited stdio — one JSON object per
line, with a trailing `\n` after every message. Stdout is the protocol channel.
All diagnostics and traces go to `.roko/acp.log` by default. Never read or write
any non-protocol content to stdout when ACP is active.

### Supported editors

Any editor that can spawn a child process and drive it via stdin/stdout supports
ACP. Known working targets:

- JetBrains AI Assistant
- Zed
- Neovim (via plugin)
- VS Code (via extension)
- Any editor that implements the ACP client spec

### Capabilities advertised by Roko

| Capability | Value | Notes |
|---|---|---|
| `loadSession` | `true` | Persisted sessions can be resumed |
| `promptCapabilities.image` | `false` | Image input not supported |
| `promptCapabilities.audio` | `false` | Audio input not supported |
| `promptCapabilities.embeddedContext` | `true` | Resource/diff blocks supported |
| `mcpCapabilities.http` | `true` | HTTP MCP servers supported |
| `mcpCapabilities.sse` | `true` | SSE MCP servers supported |

---

## Getting Started

### Running the server

```bash
# Minimal invocation (working directory = current dir)
roko acp

# Explicit working directory
roko acp --workdir /path/to/project

# With explicit config file
roko acp --config /path/to/roko.toml

# With custom log file
roko acp --log-file /tmp/roko-acp.log
```

The `AcpConfig` struct governs all server-side paths:

```rust
pub struct AcpConfig {
    pub workdir: PathBuf,       // default: current directory
    pub profile: String,        // default: "default"
    pub config_path: Option<PathBuf>,   // default: None (loads workdir/roko.toml)
    pub log_file: PathBuf,      // default: .roko/acp.log
}
```

On startup the server:

1. Initialises file logging at `log_file` (no ansi, level `roko_acp=debug`).
2. Loads `workdir/roko.toml` (or `config_path` if explicit). Falls back to
   defaults if the file is missing.
3. Garbage-collects persisted sessions older than 7 days from `.roko/sessions/`.
4. Enters the read-dispatch loop on stdin.

### Wire format

One JSON object per line. The server sends and receives three message shapes:

```
// Request (client → server)
{"jsonrpc":"2.0","id":1,"method":"session/new","params":{...}}\n

// Response (server → client)
{"jsonrpc":"2.0","id":1,"result":{...}}\n
{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"..."}}\n

// Notification (either direction, no id)
{"jsonrpc":"2.0","method":"session/update","params":{...}}\n
```

The `id` field can be a number (`u64`) or a string. Null id is used internally
for parse-level failures only.

---

## Error Codes

| Code | Constant | Meaning |
|---|---|---|
| `-32700` | `PARSE_ERROR` | Message could not be parsed as valid JSON |
| `-32600` | `INVALID_REQUEST` | Not a valid JSON-RPC 2.0 object |
| `-32601` | `METHOD_NOT_FOUND` | Method name is not implemented |
| `-32602` | `INVALID_PARAMS` | Params failed deserialization for the method |
| `-32603` | `INTERNAL_ERROR` | Unexpected server-side error |
| `-32000` | `SESSION_NOT_FOUND` | `session_id` does not exist in memory or on disk |
| `-32001` | `SESSION_BUSY` | A prompt is already in-flight for this session |

All error responses have this shape:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32001,
    "message": "session 'sess_abc' already has an active prompt",
    "data": null
  }
}
```

---

## Methods

### `initialize`

**Direction:** Client → Server
**Required:** Must be sent first before any other request.

#### Request

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": 1,
    "clientCapabilities": {
      "fs": {
        "readTextFile": true,
        "writeTextFile": false
      },
      "terminal": true,
      "mcpServers": true
    },
    "clientInfo": {
      "name": "my-editor",
      "version": "1.0.0",
      "title": "My Editor"
    }
  }
}
```

#### Params type

```rust
pub struct InitializeParams {
    pub protocol_version: u32,
    pub client_capabilities: ClientCapabilities,  // default: all false
    pub client_info: Option<ClientInfo>,
}

pub struct ClientCapabilities {
    pub fs: Option<FsCapabilities>,
    pub terminal: Option<bool>,
    pub mcp_servers: Option<bool>,
}

pub struct FsCapabilities {
    pub read_text_file: bool,
    pub write_text_file: bool,
}

pub struct ClientInfo {
    pub name: String,
    pub version: Option<String>,
    pub title: Option<String>,
}
```

#### Response

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": 1,
    "agentCapabilities": {
      "loadSession": true,
      "promptCapabilities": {
        "image": false,
        "audio": false,
        "embeddedContext": true
      },
      "mcpCapabilities": {
        "http": true,
        "sse": true
      }
    },
    "authMethods": [],
    "agentInfo": {
      "name": "roko",
      "version": "0.1.0",
      "title": "Roko"
    }
  }
}
```

#### Result type

```rust
pub struct InitializeResult {
    pub protocol_version: u32,
    pub agent_capabilities: AgentCapabilities,
    pub auth_methods: Vec<serde_json::Value>,  // always empty currently
    pub agent_info: Option<AgentInfo>,
}

pub struct AgentCapabilities {
    pub load_session: bool,
    pub prompt_capabilities: PromptCapabilities,
    pub mcp_capabilities: McpCapabilities,
}

pub struct AgentInfo {
    pub name: String,      // always "roko"
    pub version: String,   // from CARGO_PKG_VERSION
    pub title: Option<String>,  // always "Roko"
}
```

---

### `session/new`

Creates a new conversation session. After the response, the server immediately
sends a `session/update` notification carrying the available slash commands.

#### Request

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "session/new",
  "params": {
    "sessionName": "My coding session",
    "clientCapabilities": {
      "terminal": true
    },
    "mcpServers": [
      {
        "name": "filesystem",
        "transport": {
          "type": "stdio",
          "command": "npx",
          "args": ["-y", "@modelcontextprotocol/server-filesystem", "/"]
        }
      },
      {
        "name": "my-api",
        "transport": {
          "type": "http",
          "url": "http://localhost:3000/mcp"
        }
      }
    ]
  }
}
```

#### Params type

```rust
pub struct SessionNewParams {
    pub session_name: Option<String>,
    pub client_capabilities: Option<ClientCapabilities>,
    pub mcp_servers: Vec<McpServerConfig>,   // default: []
}

pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransport,
}

// Tag: "type"
pub enum McpTransport {
    Http { url: String },
    Stdio { command: String, args: Vec<String> },
}
```

#### Response

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
    "modes": {
      "currentModeId": "code",
      "availableModes": [
        {"id": "code",     "name": "Code",     "description": "Implement and edit code directly"},
        {"id": "plan",     "name": "Plan",     "description": "Plan without writing code"},
        {"id": "research", "name": "Research", "description": "Gather context and analyze"}
      ]
    },
    "configOptions": [ ... ]
  }
}
```

Immediately after the response, the server sends:

```json
{
  "jsonrpc": "2.0",
  "method": "session/update",
  "params": {
    "sessionId": "sess_...",
    "update": {
      "sessionUpdate": "available_commands_update",
      "availableCommands": [
        {"name": "/help",     "description": "Show available slash commands"},
        {"name": "/status",   "description": "Show active workflow status"},
        {"name": "/cancel",   "description": "Cancel the active prompt"},
        {"name": "/mode",     "description": "Show or set the agent mode", "input": {"hint": "code | plan | research"}},
        {"name": "/workflow", "description": "Show or set the workflow pipeline"},
        {"name": "/model",    "description": "Show or set the active model"},
        {"name": "/clear",    "description": "Clear conversation history"}
      ]
    }
  }
}
```

#### Result type

```rust
pub struct SessionNewResult {
    pub session_id: String,
    pub modes: Option<ModesInfo>,
    pub config_options: Option<Vec<ConfigOption>>,
}

pub struct ModesInfo {
    pub current_mode_id: String,
    pub available_modes: Vec<ModeInfo>,
}

pub struct ModeInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}
```

The session ID has the prefix `sess_` followed by a UUID v4.

---

### `session/list`

Lists all sessions known to the server: in-memory plus any persisted in
`.roko/sessions/` that are not already loaded.

#### Request

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "session/list"
}
```

#### Response

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "sessions": [
      {
        "sessionId": "sess_abc123",
        "sessionName": "My coding session",
        "createdAt": "2025-01-15T10:30:00Z"
      }
    ]
  }
}
```

#### Result type

```rust
pub struct SessionListResult {
    pub sessions: Vec<SessionInfo>,
}

pub struct SessionInfo {
    pub session_id: String,
    pub session_name: Option<String>,
    pub created_at: String,   // RFC 3339
}
```

Sessions are sorted ascending by `created_at`, then by `session_id`.

---

### `session/load`

Loads an existing session (from memory or disk) and returns its state in
`SessionNewResult` shape. Enables session resume across editor restarts.

#### Request

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "session/load",
  "params": {
    "sessionId": "sess_abc123"
  }
}
```

#### Response

Same shape as `session/new` result. Returns `-32000 SESSION_NOT_FOUND` if the
session is not in memory and not on disk.

#### On-disk persistence

After every successful `session/prompt`, the server writes the session to:

```
.roko/sessions/{session_id}.json
```

This includes the full `AcpSession` struct:

- `session_id`, `session_name`, `created_at`
- `config_state` (current model, effort, workflow, etc.)
- `conversation_history` (up to 40 turns / 64,000 chars)
- `mcp_servers` attachment list
- `config_options` list
- `active_run` (the last `WorkflowRun` if a pipeline was used)

Sessions 7 days old or older are GC'd at server startup.

---

### `session/prompt`

Sends a user prompt to a session. This is the primary method. It blocks until
the agent completes or is cancelled, streaming `session/update` notifications
throughout.

#### Request

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "session/prompt",
  "params": {
    "sessionId": "sess_abc123",
    "prompt": [
      {
        "type": "text",
        "text": "Add error handling to the login function"
      },
      {
        "type": "resource",
        "resource": {
          "type": "file",
          "uri": "file:///path/to/project/src/auth/login.rs"
        }
      }
    ],
    "includeContext": false
  }
}
```

#### Params type

```rust
pub struct SessionPromptParams {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
    pub include_context: bool,   // default: false
}

// Tag: "type" (snake_case)
pub enum ContentBlock {
    Text { text: String },
    Resource { resource: ResourceRef },
    Diff { path: String, diff: String },
}

// Tag: "type" (snake_case)
pub enum ResourceRef {
    File { uri: String },
}
```

When `include_context` is `true`, the server resolves all `@`-mentions and
file references in the prompt into inline file content injected into the system
prompt. When `false`, only explicit `resource` blocks are resolved.

The server returns an error with code `-32001 SESSION_BUSY` if a prompt is
already in-flight.

#### Response

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "result": {
    "stopReason": "end_turn"
  }
}
```

```rust
pub struct SessionPromptResult {
    pub stop_reason: StopReason,
}

pub enum StopReason {
    EndTurn,          // Agent completed normally
    MaxTokens,        // Model hit token limit
    MaxTurnRequests,  // Max turn loop exceeded
    Refusal,          // Agent refused to answer
    Cancelled,        // session/cancel notification was received
}
```

All `session/update` notifications are streamed before the response arrives.
The editor should treat the response as the completion signal.

#### Slash commands

If the prompt text starts with `/`, it is dispatched as a slash command instead
of forwarded to the model:

| Command | Effect |
|---|---|
| `/help` | Streams a text chunk listing available commands |
| `/status` | Streams the active `WorkflowRun` status summary |
| `/cancel` | Cancels the active prompt |
| `/mode [code\|plan\|research]` | Gets or sets the agent mode; clears history on change |
| `/workflow [none\|express\|standard\|full\|auto]` | Gets or sets pipeline workflow |
| `/model [key]` | Gets or sets the active model key |
| `/clear` | Clears conversation history |

Slash commands produce `TokenChunk` events (plain text) and do not write
episodes or query the knowledge store.

---

### `session/config/update`

Updates a single config option for a session. Also accepted as the legacy alias
`session/set_config_option`.

#### Request

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "session/config/update",
  "params": {
    "sessionId": "sess_abc123",
    "optionId": "model",
    "newValue": "sonnet"
  }
}
```

#### Params type

```rust
pub struct ConfigUpdateParams {
    pub session_id: String,
    pub option_id: String,
    pub new_value: serde_json::Value,
}
```

#### Response

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "result": {
    "configOptions": [ ... ]
  }
}
```

```rust
pub struct ConfigUpdateResult {
    pub config_options: Vec<ConfigOption>,
}
```

Returns the full updated options list so the editor can redraw its settings UI.

---

### `session/set_mode`

Legacy method to change the interaction mode. Prefer `session/config/update`
with `optionId: "workflow"` for pipeline control. Mode change clears conversation
history.

#### Request

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "session/set_mode",
  "params": {
    "sessionId": "sess_abc123",
    "modeId": "plan"
  }
}
```

#### Params type

```rust
pub struct SessionSetModeParams {
    pub session_id: String,
    pub mode_id: String,   // "code" | "plan" | "research"
}
```

#### Response

Same shape as `session/config/update` result.

---

## Notifications

### `session/cancel` (client → server)

Cooperative cancellation. Sent as a notification (no `id`), so no response is
expected. During an active `session/prompt`, the server reads incoming messages
concurrently; it processes `session/cancel` immediately and sets the cancel
token. The `session/prompt` response will then arrive with
`stopReason: "cancelled"`.

```json
{
  "jsonrpc": "2.0",
  "method": "session/cancel",
  "params": {
    "sessionId": "sess_abc123"
  }
}
```

```rust
pub struct SessionCancelParams {
    pub session_id: String,
}
```

Cancellation is cooperative. The server's cancel token is an `Arc<AtomicBool>`
paired with a `Notify`. On `cancel()`, the flag is set and all waiters are
notified. The streaming loop polls it with `tokio::select!` on every iteration.

---

### `session/update` (server → client)

The primary streaming notification. Sent repeatedly during `session/prompt`
execution. All variants are discriminated by the `sessionUpdate` field.

The notification envelope:

```json
{
  "jsonrpc": "2.0",
  "method": "session/update",
  "params": {
    "sessionId": "sess_abc123",
    "update": { ... }
  }
}
```

---

## Session Update Variants (`SessionUpdate`)

All variants share the tag field `sessionUpdate` (snake_case).

### `agent_message_chunk`

A streamed text token from the agent.

```json
{
  "sessionUpdate": "agent_message_chunk",
  "content": {
    "type": "text",
    "text": "Here is the fix for the login function:\n\n"
  }
}
```

```rust
SessionUpdate::AgentMessageChunk {
    content: ContentBlock,
    _meta: Option<serde_json::Value>,   // reserved, always null
}
```

Accumulate these in order to reconstruct the full response. The assistant text
is also stored in conversation history (truncated to 10,240 bytes per turn).

### `agent_thought_chunk`

Internal reasoning/thinking text from the model. Not part of the visible
response.

```json
{
  "sessionUpdate": "agent_thought_chunk",
  "content": {
    "type": "text",
    "text": "I need to look at the current error handling..."
  }
}
```

```rust
SessionUpdate::AgentThoughtChunk {
    content: ContentBlock,
}
```

### `tool_call`

A new tool call card has started. The editor should render this as an expandable
card showing the tool name and status.

```json
{
  "sessionUpdate": "tool_call",
  "toolCallId": "toolu_01abc",
  "title": "Edit src/auth/login.rs",
  "kind": "edit",
  "status": "in_progress",
  "content": []
}
```

```rust
SessionUpdate::ToolCall {
    tool_call_id: String,
    title: String,
    kind: ToolCallKind,
    status: ToolCallStatus,
    content: Vec<ContentBlock>,
}

pub enum ToolCallKind {
    Edit,
    Create,
    Delete,
    Terminal,
    Other,
}

pub enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}
```

### `tool_call_update`

An update to an existing tool call card. Match by `toolCallId`.

```json
{
  "sessionUpdate": "tool_call_update",
  "toolCallId": "toolu_01abc",
  "status": "completed",
  "content": [
    {
      "type": "diff",
      "path": "src/auth/login.rs",
      "diff": "@@ -10,6 +10,10 @@\n..."
    }
  ]
}
```

```rust
SessionUpdate::ToolCallUpdate {
    tool_call_id: String,
    status: ToolCallStatus,
    content: Vec<ContentBlock>,
}
```

The `content` field can include `diff` blocks for file edits, showing the actual
unified diff applied.

### `plan`

A structured plan update. Sent when the agent produces a multi-step action plan.

```json
{
  "sessionUpdate": "plan",
  "entries": [
    {"content": "Read the current login function",       "priority": "high",   "status": "completed"},
    {"content": "Add try/catch around the DB call",      "priority": "high",   "status": "in_progress"},
    {"content": "Add tests for the error path",          "priority": "medium", "status": "pending"}
  ]
}
```

```rust
SessionUpdate::Plan {
    entries: Vec<PlanEntry>,
}

pub struct PlanEntry {
    pub content: String,
    pub priority: Priority,   // high | medium | low
    pub status: PlanStatus,   // pending | in_progress | completed
}
```

### `available_commands_update`

Sent after `session/new` and whenever the command set changes.

```json
{
  "sessionUpdate": "available_commands_update",
  "availableCommands": [
    {"name": "/help",   "description": "Show available slash commands"},
    {"name": "/status", "description": "Show active workflow status"},
    {"name": "/mode",   "description": "Show or set the agent mode",
     "input": {"hint": "code | plan | research"}}
  ]
}
```

```rust
SessionUpdate::AvailableCommandsUpdate {
    available_commands: Vec<SlashCommand>,
}

pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub input: Option<CommandInput>,
}

pub struct CommandInput {
    pub hint: Option<String>,
}
```

### `config_option_update`

Sent when config options change mid-session.

```json
{
  "sessionUpdate": "config_option_update",
  "configOptions": [ ... ]
}
```

```rust
SessionUpdate::ConfigOptionUpdate {
    config_options: Vec<ConfigOption>,
}
```

### `usage_update`

Token and cost update, emitted after model calls complete.

```json
{
  "sessionUpdate": "usage_update",
  "used": 4250,
  "size": 200000,
  "cost": {
    "amount": 0.0085,
    "currency": "USD"
  }
}
```

```rust
SessionUpdate::UsageUpdate {
    used: u64,
    size: u64,
    cost: Option<CostInfo>,
}

pub struct CostInfo {
    pub amount: f64,
    pub currency: String,   // ISO code, e.g. "USD"
}
```

### `session_info_update`

Metadata update for the session itself.

```json
{
  "sessionUpdate": "session_info_update",
  "sessionId": "sess_abc123",
  "sessionName": "New session name"
}
```

```rust
SessionUpdate::SessionInfoUpdate {
    session_id: String,
    session_name: Option<String>,
}
```

---

## Session Config Options

The session exposes a fixed set of configurable options. The current values
come from `roko.toml` at session creation time. All options are returned in
`session/new`, `session/load`, and `session/config/update` responses.

### Option IDs and valid values

| `optionId` | Category | Type | Valid values |
|---|---|---|---|
| `model` | `agent` | select | Keys from `[models.*]` in `roko.toml` |
| `effort` | `agent` | select | `low`, `medium`, `high`, `max` |
| `temperament` | `agent` | select | `conservative`, `balanced`, `aggressive`, `exploratory` |
| `routing_mode` | `routing` | select | `auto_override`, `manual` |
| `clippy` | `gates` | select | `on`, `off` |
| `tests` | `gates` | select | `on`, `off` |
| `workflow` | `execution` | select | `none`, `express`, `standard`, `full`, `auto` |
| `review_strictness` | `execution` | select | `none`, `quick`, `standard`, `thorough` |
| `max_iterations` | `execution` | select | `"1"`, `"2"`, `"3"` |

### ConfigOption shape

```rust
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub option_type: ConfigOptionType,   // select | toggle
    pub category: String,
    pub current_value: serde_json::Value,
    pub description: Option<String>,
    pub options: Option<Vec<ConfigOptionValue>>,
}

pub struct ConfigOptionValue {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
}
```

### `SessionConfigState` (server-side)

All option changes update this struct, which is persisted with the session:

```rust
pub struct SessionConfigState {
    pub agent_mode: String,       // "code" | "plan" | "research"
    pub model: String,            // model key from roko.toml
    pub effort: String,           // "low" | "medium" | "high" | "max"
    pub temperament: String,      // "conservative" | "balanced" | "aggressive" | "exploratory"
    pub routing_mode: String,     // "auto_override" | "manual"
    pub clippy_enabled: bool,
    pub tests_enabled: bool,
    pub workflow: String,         // "none" | "express" | "standard" | "full" | "auto"
    pub review_strictness: String, // "none" | "quick" | "standard" | "thorough"
    pub max_iterations: u32,      // 1–3
}
```

Defaults (from `roko.toml`, falling back to these if unconfigured):

- `agent_mode`: `"code"`
- `effort`: `"medium"`
- `temperament`: `"balanced"`
- `routing_mode`: `"auto_override"`
- `clippy_enabled`: `true`
- `tests_enabled`: `true`
- `workflow`: `"none"`
- `review_strictness`: `"none"`
- `max_iterations`: `2`

---

## CognitiveEvent Enum

`CognitiveEvent` is the internal event type produced by the cognitive loop and
consumed by the stream bridge. Understanding it helps trace how model output
maps to `session/update` notifications.

```rust
pub enum CognitiveEvent {
    /// A streamed agent-visible text chunk (maps to AgentMessageChunk)
    TokenChunk(String),

    /// A streamed internal reasoning chunk (maps to AgentThoughtChunk)
    ThinkingChunk(String),

    /// A tool call has started (maps to ToolCall)
    ToolCallStart {
        tool_call_id: String,
        title: String,
        kind: ToolCallKind,
    },

    /// A tool call has finished (maps to ToolCallUpdate)
    ToolCallComplete {
        tool_call_id: String,
        status: ToolCallStatus,
        content: Vec<ContentBlock>,
    },

    /// A plan update with structured entries (maps to Plan)
    PlanUpdate {
        entries: Vec<PlanEntry>,
    },

    /// Prompt execution completed normally (terminal)
    Complete {
        stop_reason: StopReason,
        usage: Option<UsageInfo>,
    },

    /// Token budget exhausted (terminal)
    MaxTokens,
}
```

The streaming loop uses `tokio::select!` with three arms:
1. Cancel token — returns `Cancelled` stop reason.
2. Event channel receive — processes cognitive events.
3. Inbound transport read — handles `session/cancel` notifications mid-stream.

The loop terminates when a `Complete` or `MaxTokens` event arrives, or when
the event channel closes without a terminal event (treated as `EndTurn`).

### AcpAdapter: RuntimeEvent → CognitiveEvent

For workflow pipeline dispatches, `AcpAdapter` implements `EventConsumer` and
translates `RuntimeEvent` from the workflow engine into `CognitiveEvent`:

| `RuntimeEvent` | `CognitiveEvent` |
|---|---|
| `AgentOutput { chunk }` | `TokenChunk(chunk)` |
| `AgentSpawned { agent_id, role }` | `ToolCallStart { title: "Agent: {role}", kind: Other }` |
| `AgentCompleted { agent_id, output }` | `ToolCallComplete { status: Completed, content: [text] }` |
| `AgentFailed { agent_id, error }` | `ToolCallComplete { status: Failed, content: [text] }` |
| `GateStarted { gate_name }` | `ToolCallStart { title: "Gate: {gate_name}", kind: Other }` |
| `GatePassed { gate_name }` | `ToolCallComplete { status: Completed, content: ["{gate_name} passed"] }` |
| `GateFailed { gate_name, output }` | `ToolCallComplete { status: Failed, content: [output] }` |
| `PhaseTransition { from, to }` | `TokenChunk("[Phase: {from} -> {to}]\n")` |
| `WorkflowCompleted { outcome: Cancelled }` | `Complete { stop_reason: Cancelled }` |
| `WorkflowCompleted { outcome: Success/Halted }` | `Complete { stop_reason: EndTurn }` |
| `WorkflowStarted`, `FeedbackRecorded`, `StateCheckpointed` | (ignored) |

The adapter filters by `run_id` to avoid receiving events from concurrent runs.

---

## Pipeline State Machine

When `workflow` is set to anything other than `"none"`, a `PipelineState`
is created and drives the multi-agent execution loop.

### `PipelinePhase` states

```rust
pub enum PipelinePhase {
    Pending,        // Created, not started
    Strategizing,   // Strategist agent analysing the prompt
    Implementing,   // Implementer agent writing code
    AutoFixing,     // Auto-fixer agent patching gate failures
    Gating,         // Gates (compile, test, clippy) running
    Reviewing,      // Reviewer agent(s) checking the diff
    Committing,     // Creating a git commit
    Complete,       // Terminal: pipeline succeeded
    Halted { reason: String },  // Terminal: timeout, budget, or max iterations exceeded
    Cancelled,      // Terminal: user cancelled
}
```

Terminal states: `Complete`, `Halted { .. }`, `Cancelled`.

### `WorkflowTemplate` and phases run

```rust
pub enum WorkflowTemplate {
    Express,   // Implement → Gate → Commit
    Standard,  // Implement → Gate → Review → Commit
    Full,      // Strategy → Implement → Gate → Review → Commit
}
```

Auto-selection heuristics (used when `workflow = "auto"`):

- `Express`: prompt contains `fix`, `typo`, `rename`, `update`, or `bump`
  AND word count < 15.
- `Full`: prompt contains `files`, `modules`, `system`, `architecture`, or
  `refactor`, OR word count > 50.
- `Standard`: everything else.

### State transitions

```
Pending ──Start──► Strategizing (Full)
Pending ──Start──► Implementing (Express, Standard)

Strategizing ──StrategyComplete──► Implementing
Strategizing ──StrategySkipped──► Implementing

Implementing ──AgentCompleted──► Gating
Implementing ──AgentFailed──► Implementing (retry, if iterations remain)
Implementing ──AgentFailed──► Halted (no iterations left)

AutoFixing ──AgentCompleted──► Gating
AutoFixing ──AgentFailed──► Implementing (retry with gate+autofix context)
AutoFixing ──AgentFailed──► Halted (no iterations left)

Gating ──GatesPassed──► Reviewing (Standard, Full)
Gating ──GatesPassed──► Committing (Express)
Gating ──GateFailed──► AutoFixing (if iterations remain)
Gating ──GateFailed──► Halted (no iterations left)

Reviewing ──ReviewApproved──► Committing
Reviewing ──ReviewRevise──► Implementing (retry with review feedback)
Reviewing ──ReviewRevise──► Committing (no iterations left: accept with caveats)

Committing ──CommitDone──► Complete

Any ──UserCancel──► Cancelled
Any ──Timeout──► Halted
Any ──BudgetExceeded──► Halted
```

### `PipelineConfig`

The runner receives this configuration per-dispatch:

```rust
pub struct PipelineConfig {
    pub template: WorkflowTemplate,
    pub max_iterations: u32,     // from session config_state.max_iterations (1–3)
    pub clippy_enabled: bool,
    pub tests_enabled: bool,
    pub review_strictness: String,  // "none" | "quick" | "standard" | "thorough"
}
```

### `WorkflowRun`

A `WorkflowRun` wraps `PipelineState` with timing and cost metadata. It is
stored as `session.active_run` after each pipeline execution:

```rust
pub struct WorkflowRun {
    pub run_id: String,          // "run_{uuid}"
    pub pipeline: PipelineState,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub agents_spawned: u32,
}
```

### Gate error classification

When a gate fails, the runner classifies the error for targeted auto-fix context:

```rust
enum GateErrorType {
    CompileError { file: String, line: u32, message: String },
    TestFailure  { test_name: String, expected: Option<String>, actual: Option<String> },
    ClippyWarning { lint: String, location: String },
    RuntimePanic  { message: String },
    Unknown,
}
```

Gate results are also expressed as `GateResult` in `workflow.rs`:

```rust
pub struct GateResult {
    pub gate: String,        // "compile" | "test" | "clippy" | "fmt"
    pub passed: bool,
    pub output: String,
    pub duration_ms: u64,
}
```

And review findings as:

```rust
pub struct ReviewFinding {
    pub severity: String,    // "major" | "minor" | "nit"
    pub description: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}
```

---

## Session Lifecycle

### Full message sequence

```
Client                                     Server
  │                                           │
  │──initialize ──────────────────────────► │
  │◄── result: InitializeResult ──────────── │
  │                                           │
  │──session/new ──────────────────────────► │
  │◄── result: SessionNewResult ──────────── │
  │◄── notification: session/update ───────── │  (available_commands_update)
  │                                           │
  │──session/config/update ───────────────► │  (set workflow = "standard")
  │◄── result: ConfigUpdateResult ─────────── │
  │                                           │
  │──session/prompt ───────────────────────► │
  │◄── notification: session/update ───────── │  (tool_call: knowledge card)
  │◄── notification: session/update ───────── │  (tool_call: Agent: implementer — in_progress)
  │◄── notification: session/update ───────── │  (agent_message_chunk: "...")
  │◄── notification: session/update ───────── │  (tool_call_update: Agent — completed)
  │◄── notification: session/update ───────── │  (tool_call: Gate: compile — in_progress)
  │◄── notification: session/update ───────── │  (tool_call_update: Gate: compile — completed)
  │◄── notification: session/update ───────── │  (tool_call: Agent: reviewer — in_progress)
  │◄── notification: session/update ───────── │  (tool_call_update: Agent: reviewer — completed)
  │◄── notification: session/update ───────── │  (token_chunk: commit message)
  │◄── notification: session/update ───────── │  (usage_update)
  │◄── result: SessionPromptResult ────────── │  (stopReason: "end_turn")
  │                                           │
  │──session/cancel (notification) ────────► │  (if user cancels mid-prompt)
  │                                           │
  │──session/list ─────────────────────────► │
  │◄── result: SessionListResult ─────────── │
  │                                           │
  │──session/load ─────────────────────────► │
  │◄── result: SessionNewResult ──────────── │
  │                                           │
  EOF (stdin close)                           │ server shuts down gracefully
```

### Conversation history

The server maintains in-session multi-turn history:

- Max turns: 40
- Max total characters: 64,000
- Oldest turns dropped first (FIFO) when either limit is exceeded
- History is cleared when the mode changes via `session/set_mode`
- Assistant response text is truncated to 10,240 bytes before storage

For CLI-based providers (Claude CLI), history is serialised as XML:

```xml
<conversation_history>
<user>
First user message
</user>
<assistant>
First assistant response
</assistant>
</conversation_history>
```

For API-based providers, history becomes the standard messages array:

```json
[
  {"role": "system", "content": "...system prompt..."},
  {"role": "user",   "content": "First user message"},
  {"role": "assistant", "content": "First assistant response"},
  {"role": "user",   "content": "Current prompt"}
]
```

### Busy state

A session allows only one in-flight prompt at a time. The `busy` flag is an
`Arc<AtomicBool>`. On `session/prompt` arrival:

1. Check `busy` — if true, return `-32001 SESSION_BUSY`.
2. Reset the cancel token (a new `CancelToken` is created).
3. Set `busy = true`.
4. Run the cognitive task.
5. Set `busy = false` (even on error/cancel).
6. Persist the session to disk.

---

## Knowledge Integration (roko-neuro)

Before every non-slash-command `session/prompt`, the server runs two parallel
knowledge lookups:

### 1. roko-neuro knowledge store

```rust
KnowledgeStore::for_workdir(&workdir).query_hits(&prompt_text, 5)
```

Returns up to 5 `KnowledgeQueryHit` entries ranked by `total_score`:

```rust
pub struct KnowledgeQueryHit {
    pub entry: KnowledgeEntry,
    pub total_score: f64,
    pub breakdown: KnowledgeQueryBreakdown,
}

pub struct KnowledgeEntry {
    pub id: String,
    pub kind: KnowledgeKind,     // see below
    pub content: String,
    pub confidence: f64,
    pub tier: KnowledgeTier,     // see below
    pub created_at: DateTime<Utc>,
    // ...
}

pub struct KnowledgeQueryBreakdown {
    pub keyword_score: f64,
    pub effective_confidence: f64,
    pub recency_factor: f64,
    pub emotional_boost: f64,
    pub hdc_similarity: Option<f64>,
}
```

Knowledge tiers (shown as single-letter labels):

| `KnowledgeTier` | Label | Meaning |
|---|---|---|
| `Persistent` | `P` | Long-lived, high-confidence facts |
| `Consolidated` | `C` | Distilled from working memory |
| `Working` | `W` | Recent session knowledge |
| `Transient` | `T` | Ephemeral, not yet validated |

Knowledge kinds:

| `KnowledgeKind` | Label |
|---|---|
| `Insight` | `insight` |
| `Heuristic` | `heuristic` |
| `AntiKnowledge` | `anti-pattern` |
| `Warning` | `warning` |
| `CausalLink` | `causal` |
| `StrategyFragment` | `strategy` |

### 2. Playbook store (roko-learn)

```rust
PlaybookStore::new(workdir.join(".roko/learn/playbooks"))
    .relevant(&prompt_text, 3)
    .await
```

Returns up to 3 relevant `Playbook` entries. A `Playbook` records a
successful multi-step task pattern:

```rust
pub struct Playbook {
    pub name: String,
    pub goal: String,
    pub steps: Vec<PlaybookStep>,
    pub success_count: u32,
    pub failure_count: u32,
}

pub struct PlaybookStep {
    pub index: u32,
    pub description: String,
    pub action_kind: String,        // "edit_file" | "run_command" | etc.
    pub expected_signals: Vec<String>,
}
```

### Rendering

Results are rendered two ways:

**Visible card** — sent as a `tool_call` / `tool_call_update` pair before the
main agent turn:

```
Title: "Prior knowledge - 3 results"
Body:
  **Playbooks:**
    - Resolve Send + Sync errors (75% success)
  **Knowledge:**
    - [P] 0.91 - Prefer smaller retries after gate failures...
```

**Prompt context** — injected into the system prompt:

```
## Relevant playbooks from past tasks:

### fix-concurrency
Goal: Resolve Send + Sync errors
Success rate: 75% success
Steps:
  1. Replace shared HashMap with DashMap [edit_file] -> compile_ok
  2. Run cargo test to confirm the fix [run_command] -> tests_pass

## Relevant knowledge:

- [heuristic / P] 0.91
  Prefer smaller retries after gate failures because they keep the feedback loop tight.
```

Knowledge and playbook lookups run in parallel via `tokio::join!`. If either
store is unavailable or the query fails, the error is logged and an empty result
is returned so dispatch can continue.

---

## Episode Logging

After every completed `session/prompt` (including pipeline dispatches), the
server appends an `Episode` to `.roko/episodes.jsonl`. This is the same
episode format used by the orchestrator.

### Episode fields written by ACP

```rust
pub struct Episode {
    pub id: String,
    pub episode_id: String,
    pub kind: String,          // "acp-dispatch" or "acp-pipeline-{workflow}"
    pub agent_template: String, // same as agent_mode ("code" | "plan" | "research")
    pub model: String,          // resolved model slug
    pub backend: String,        // provider label (e.g. "claude-cli", "anthropic")
    pub trigger_kind: String,   // "acp_dispatch" or "acp_pipeline"
    pub trigger_signal_hash: String,  // SHA-256 of prompt bytes (hex)
    pub input_signal_hash: String,
    pub output_signal_hash: String,   // SHA-256 of assistant response bytes
    pub duration_secs: f64,
    pub success: bool,
    pub failure_reason: Option<String>,
    pub usage: EpUsage {
        pub wall_ms: u64,
        // ... other usage fields default to 0
    },
    pub extra: HashMap<String, serde_json::Value>,
}
```

The `extra` map contains:

| Key | Value |
|---|---|
| `entry_point` | `"acp"` |
| `model` | resolved model slug |
| `mode` | agent mode string |
| `session_id` | session ID |
| `routing_mode` | `"auto_override"` or `"manual"` |
| `workflow` | workflow config string |
| `provider_kind` | provider label |

The `kind` field encodes the dispatch type:
- Single-agent: `"acp-dispatch"`
- Pipeline express: `"acp-pipeline-express"`
- Pipeline standard: `"acp-pipeline-standard"`
- Pipeline full: `"acp-pipeline-full"`

Success is `true` only when `stop_reason == EndTurn` and no errors occurred.

---

## File Change Notifications

After a pipeline commit phase, the server detects changed files via:

```
git diff --name-status HEAD~1 HEAD
```

And emits a `FileChangeNotification` per changed file (skipping lock files and
images). These are rendered as `tool_call` updates in the stream, but they are
also available in the internal struct for integration testing:

```rust
pub struct FileChangeNotification {
    pub path: String,
    pub change_type: FileChangeType,
}

pub enum FileChangeType {
    Added,
    Modified,
    Deleted,
    Renamed,
}
```

---

## Transport Implementation Details

The `StdioTransport` uses:
- `BufReader` over stdin for newline-delimited JSON parsing.
- `Arc<AsyncMutex<W>>` for the writer, enabling clones to share it.
- `Arc<AtomicU64>` for auto-incrementing outbound request IDs (starting at 1).
- `Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>` for tracking
  pending outbound requests.

The transport can send requests *to the client* (for bridging scenarios) and
match incoming responses via `handle_incoming_response`. This is used for
`fs/read_text_file` and `fs/write_text_file` when `clientCapabilities.fs` is set.

Every message is flushed immediately after writing. No framing headers are used.

### Reading

```rust
let mut line = String::new();
reader.read_line(&mut line).await?;   // blocks until \n
let message = serde_json::from_str::<JsonRpcMessage>(&line)?;
```

### Writing

```rust
let bytes = serde_json::to_vec(message)?;
writer.write_all(&bytes).await?;
writer.write_all(b"\n").await?;
writer.flush().await?;
```

---

## Building an Editor Integration: Quick Reference

### Minimum viable sequence

```jsonc
// 1. Start: roko acp --workdir /path/to/project

// 2. Initialize
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}

// 3. Create session
{"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}

// 4. Send prompt
{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
  "sessionId":"sess_...",
  "prompt":[{"type":"text","text":"Add error handling to the main function"}],
  "includeContext":false
}}

// 5. Stream updates until result arrives

// 6. Cancel if needed (notification, no id)
{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"sess_..."}}
```

### What to render from updates

| `sessionUpdate` value | UI action |
|---|---|
| `agent_message_chunk` | Append `content.text` to the chat message |
| `agent_thought_chunk` | Optionally show in a collapsible "Thinking..." section |
| `tool_call` | Create a new tool card with the given `title`, `kind`, `status` |
| `tool_call_update` | Update the card matching `toolCallId` |
| `plan` | Render a checklist of plan entries |
| `available_commands_update` | Update slash command autocomplete list |
| `config_option_update` | Re-render the settings panel |
| `usage_update` | Show token/cost usage in status bar |
| `session_info_update` | Update the session display name |

### Config option update example

To switch to the full pipeline workflow with thorough review:

```json
{"jsonrpc":"2.0","id":10,"method":"session/config/update","params":{
  "sessionId":"sess_...",
  "optionId":"workflow",
  "newValue":"full"
}}

{"jsonrpc":"2.0","id":11,"method":"session/config/update","params":{
  "sessionId":"sess_...",
  "optionId":"review_strictness",
  "newValue":"thorough"
}}
```

### Resuming across restarts

```json
// List persisted sessions
{"jsonrpc":"2.0","id":1,"method":"session/list"}

// Load a specific one
{"jsonrpc":"2.0","id":2,"method":"session/load","params":{"sessionId":"sess_abc123"}}

// Continue prompting with history restored
{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
  "sessionId":"sess_abc123",
  "prompt":[{"type":"text","text":"What did we just implement?"}],
  "includeContext":false
}}
```

---

## roko.toml Configuration for ACP

The ACP server loads `roko.toml` from the working directory. Relevant sections:

```toml
[agent]
default_effort = "medium"    # Sets initial effort config option
temperament = "balanced"     # Sets initial temperament config option

[routing]
mode = "auto_override"       # Sets initial routing_mode config option

[gates]
clippy_enabled = true        # Sets initial clippy option
skip_tests = false           # skip_tests = true → tests option defaults to "off"

[models.sonnet]
provider = "anthropic"
slug = "claude-sonnet-4-6"

[models.haiku]
provider = "anthropic"
slug = "claude-haiku-3-5"
```

Model keys in `[models.*]` become the selectable values for the `model` config
option. The server picks the first available key from this preference list at
session creation: `glm51`, `glm4`, `kimi-k26`, `sonnet`, then alphabetical
first key, then `"sonnet"` as the final fallback constant.

---

## Logging and Diagnostics

All server-side logging goes to `.roko/acp.log` (never stdout). The log level
is controlled by the `RUST_LOG` environment variable, defaulting to
`roko_acp=debug`. Logs are non-blocking via `tracing_appender`.

Key log events:
- `"ACP logging initialized"` — startup with protocol_version, spec_version, log_file
- `"loaded roko.toml configuration"` — with provider and model counts
- `"handling ACP request"` — method name and request id
- `"handling ACP session prompt"` — session_id, prompt chars, model_key, workdir
- `"failed to append ACP episode"` — if episode logging fails (non-fatal)
- `"ACP client disconnected while prompt was active"` — EOF during streaming

---

## Known Limitations

- **Single transport only.** The ACP server is single-threaded on the stdio
  channel. Concurrent sessions are supported in memory, but concurrent
  *transports* (e.g., multiple TCP clients) are not. The
  `SessionManager` is not wrapped in `Arc<RwLock<_>>`.

- **No image or audio input.** `promptCapabilities.image = false`,
  `promptCapabilities.audio = false`.

- **WebSocket/SSE not on ACP path.** WebSocket and SSE streaming are available
  via the HTTP control plane (`roko serve` on `:6677`), not through the ACP
  stdio channel. ACP uses stdio only.

- **Outbound `fs/read_text_file` and `fs/write_text_file`** are implemented in
  the transport layer (pending request registry) but the bridge does not yet
  use them automatically; file content is resolved server-side by reading
  the workdir directly.

- **`ROKO_ACP_LEGACY`** env var selects the legacy pipeline runner instead of
  the `WorkflowEngine`-based path. Set this only for debugging; the default
  (workflow engine) path is the canonical one.
