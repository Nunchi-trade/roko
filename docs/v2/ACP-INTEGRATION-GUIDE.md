# ACP Integration Guide

## What is this document?

This guide explains how to build an editor plugin or IDE integration that talks
to Roko. If you have never heard of Roko, ACP, or JSON-RPC before, start here.
Every section explains the "why" before the "how."

By the end of this document you will be able to:
- Start a Roko ACP server and exchange your first messages with it.
- Understand the session model and how conversation state is managed.
- Stream live updates (text, tool calls, plans) to your UI as the agent works.
- Handle the full pipeline workflow (strategize → implement → gate → review → commit).
- Integrate with Roko's knowledge store and episode log.

---

## Table of Contents

1. [Background: What is ACP?](#1-background-what-is-acp)
2. [Quick Start (30 seconds)](#2-quick-start-30-seconds)
3. [Core Concepts](#3-core-concepts)
   - [Sessions](#sessions)
   - [The Wire Format (JSON-RPC)](#the-wire-format-json-rpc)
   - [Streaming via Notifications](#streaming-via-notifications)
   - [The Cognitive Loop](#the-cognitive-loop)
4. [Method Reference](#4-method-reference)
   - [`initialize`](#initialize)
   - [`session/new`](#sessionnew)
   - [`session/list`](#sessionlist)
   - [`session/load`](#sessionload)
   - [`session/prompt`](#sessionprompt)
   - [`session/config/update`](#sessionconfigupdate)
   - [`session/set_mode`](#sessionset_mode)
5. [Notifications Reference](#5-notifications-reference)
   - [`session/cancel` (client → server)](#sessioncancel-client--server)
   - [`session/update` (server → client)](#sessionupdate-server--client)
   - [All `session/update` variants](#all-sessionupdate-variants)
6. [Session Config Options](#6-session-config-options)
7. [The Pipeline: Multi-Phase Workflow](#7-the-pipeline-multi-phase-workflow)
8. [Knowledge Integration](#8-knowledge-integration)
9. [Episode Logging](#9-episode-logging)
10. [roko.toml Configuration](#10-rokotoml-configuration)
11. [Troubleshooting and FAQ](#11-troubleshooting-and-faq)
12. [Advanced: Transport Internals](#12-advanced-transport-internals)
13. [Known Limitations](#13-known-limitations)

---

## 1. Background: What is ACP?

### The problem ACP solves

Editors like VS Code, JetBrains, Zed, and Neovim all want to integrate AI
assistants. But each editor has its own plugin API, and each AI backend has its
own HTTP or subprocess interface. Without a shared protocol, every integration
is bespoke — you end up with an N×M matrix of editor-backend combinations.

ACP (Agent Client Protocol) is a simple, editor-neutral protocol that any
editor can implement once, and any AI agent can implement once. Roko implements
the server side. Your editor plugin implements the client side.

### How Roko fits in

Roko is an agent toolkit: it runs Claude (and other models), orchestrates
multi-step pipelines, validates code with gates (compile, test, lint), and
learns from past sessions. Running `roko acp` starts the ACP server — a
subprocess your editor spawns and communicates with over standard input and
output.

```
┌─────────────────────────────────────────────────────────────────┐
│  Your Editor                                                    │
│                                                                 │
│   Editor UI ──── Editor Plugin ──── roko acp (subprocess)      │
│       ▲                │                    │                   │
│       │           stdin/stdout          roko.toml               │
│       │           JSON-RPC 2.0          .roko/sessions/         │
│       │                │                .roko/episodes.jsonl    │
│       └── live updates ┘                .roko/acp.log           │
└─────────────────────────────────────────────────────────────────┘
```

Roko implements ACP spec version **0.12.2**, protocol version **1**.

### Supported editors

Any editor that can spawn a child process and drive it via stdin/stdout supports
ACP. Known working targets:

- JetBrains AI Assistant
- Zed
- Neovim (via plugin)
- VS Code (via extension)
- Any editor that implements the ACP client spec

---

## 2. Quick Start (30 seconds)

This section gets you to a working exchange as fast as possible. Copy and paste
these lines into a terminal. Each `→` line is sent to stdin; each `←` line is
what you should receive on stdout.

```bash
# Start the server (in your project directory)
roko acp --workdir /path/to/project
```

Then send these messages, one per line, to the process stdin:

```jsonc
// Step 1: Introduce yourself (required first message)
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{...},"agentInfo":{"name":"roko",...}}}

// Step 2: Open a conversation session
→ {"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}
← {"jsonrpc":"2.0","id":2,"result":{"sessionId":"sess_550e8400-...","modes":{...},...}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_...","update":{"sessionUpdate":"available_commands_update",...}}}

// Step 3: Ask the agent something
→ {"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"sess_...","prompt":[{"type":"text","text":"Hello, what can you do?"}],"includeContext":false}}

// (You will receive a stream of session/update notifications here, then...)
← {"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}
```

That is the complete minimal integration. The rest of this document explains
every detail of what is happening and how to build on it.

---

## 3. Core Concepts

### Sessions

A session is a persistent conversation between your editor and the Roko agent.
Think of it like a chat window: it remembers what was said before, it knows
which model to use, and it tracks whether a response is currently in flight.

Each session has:
- A unique **session ID** (e.g. `sess_550e8400-e29b-41d4-a716-446655440000`)
- A **conversation history** (up to 40 turns, auto-trimmed at 64,000 characters)
- A **config state** (which model, what effort level, which workflow to run)
- A **busy flag** (only one prompt in-flight at a time)

Sessions are automatically saved to disk after each prompt completes. This
means if your editor restarts, you can resume exactly where you left off using
`session/load`.

```
Session lifecycle:

   session/new ──► [Session created, idle]
                           │
   session/prompt ─────────┤
                           ▼
              [Busy: streaming notifications]
                           │
   session/cancel ──► or prompt completes
                           │
                           ▼
              [Idle again, history updated, saved to disk]
```

### The Wire Format (JSON-RPC)

JSON-RPC 2.0 is a simple call/response protocol on top of JSON. Think of it
like HTTP but lighter: you send a "request" object with a method name, and you
get back a "result" or "error" object with the same ID.

Roko uses **newline-delimited JSON over stdio**: one JSON object per line, with
a `\n` at the end. This is the simplest possible transport — no HTTP headers,
no framing, just text lines.

There are three message shapes:

```
// Request (you → server): "please do X"
{"jsonrpc":"2.0","id":1,"method":"session/new","params":{...}}\n

// Response (server → you): "X is done, here is the result"
{"jsonrpc":"2.0","id":1,"result":{...}}\n

// Or an error response if something went wrong:
{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"..."}}\n

// Notification (either direction, no id): "FYI, something happened"
{"jsonrpc":"2.0","method":"session/update","params":{...}}\n
```

The `id` field can be a number (`u64`) or a string. Always match responses to
requests by their `id`. Notifications have no `id` and require no response.

**Stdout is the protocol channel.** All diagnostics and traces go to
`.roko/acp.log`. Never write non-protocol content to stdout when ACP is active.

### Streaming via Notifications

The most important concept to understand is how Roko streams its output. When
you send a `session/prompt` request, the server does not wait until the agent
is completely done before responding. Instead:

1. You send the `session/prompt` request (with an `id`, e.g. `id: 5`).
2. The server immediately starts sending `session/update` notifications
   (no `id`) as the agent produces output.
3. When the agent finishes, the server sends the final `session/prompt` result
   (matching `id: 5`).

Your editor should treat each `session/update` as a live "push" event and the
final result as the completion signal. Do not wait for the result before
rendering the updates.

```
Timeline of a single session/prompt call:

  t=0ms   → session/prompt (id:5)
  t=50ms  ← session/update (tool_call: "Prior knowledge")
  t=55ms  ← session/update (tool_call_update: "Prior knowledge - completed")
  t=60ms  ← session/update (agent_message_chunk: "I'll add error handling")
  t=80ms  ← session/update (tool_call: "Edit src/auth/login.rs - in_progress")
  t=120ms ← session/update (tool_call_update: "Edit - completed", diff: "...")
  t=125ms ← session/update (agent_message_chunk: " by wrapping the DB call.")
  t=130ms ← session/update (usage_update: tokens=4250, cost=$0.009)
  t=131ms ← session/prompt result (id:5, stopReason:"end_turn")
```

### The Cognitive Loop

When a prompt arrives, Roko runs what it calls a "cognitive loop" — it sends
the prompt to the model, receives streaming tokens and tool calls, executes the
tools (file edits, shell commands), and feeds results back until the model
signals it is done.

The internal event type driving this is `CognitiveEvent`. You do not need to
interact with it directly, but knowing it exists helps you understand why
`session/update` notifications arrive in the shape they do. Each
`CognitiveEvent` produced by the model maps to exactly one `session/update`
variant sent to your editor.

<details>
<summary>CognitiveEvent → session/update mapping</summary>

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

The streaming loop uses `tokio::select!` with three arms simultaneously:

1. **Cancel token** — if cancelled, returns `Cancelled` stop reason.
2. **Event channel** — processes cognitive events from the model.
3. **Inbound transport** — handles `session/cancel` notifications mid-stream.

The loop terminates when `Complete` or `MaxTokens` arrives, or when the event
channel closes (treated as `EndTurn`).

</details>

---

## 4. Method Reference

### `initialize`

**Direction:** Client → Server
**Required:** Must be sent first, before any other request.

This is the handshake. You tell Roko what your editor can do (read files?
run terminals? connect to MCP servers?), and Roko tells you what it supports.
The response contains Roko's capabilities and version information.

You must send `initialize` before any other method. If you skip it, subsequent
requests will fail.

<details>
<summary>Full request/response example</summary>

**Request:**

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

**Response:**

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

</details>

<details>
<summary>Type definitions</summary>

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

</details>

**Capabilities advertised by Roko:**

| Capability | Value | Notes |
|---|---|---|
| `loadSession` | `true` | Persisted sessions can be resumed |
| `promptCapabilities.image` | `false` | Image input not supported |
| `promptCapabilities.audio` | `false` | Audio input not supported |
| `promptCapabilities.embeddedContext` | `true` | Resource/diff blocks supported |
| `mcpCapabilities.http` | `true` | HTTP MCP servers supported |
| `mcpCapabilities.sse` | `true` | SSE MCP servers supported |

---

### `session/new`

Creates a new conversation session. Think of this as "open a new chat window."
After the response, Roko immediately sends a `session/update` notification with
the list of slash commands your editor can offer as autocomplete suggestions.

You can optionally pass MCP (Model Context Protocol) server configs here. MCP
servers are external tools the agent can call — for example, a filesystem
server that lets the agent read any file, or a custom API server.

<details>
<summary>Full request/response example (including MCP servers)</summary>

**Request:**

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

**Response:**

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

Immediately after the response, Roko sends this notification:

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

</details>

<details>
<summary>Type definitions</summary>

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

</details>

The session ID has the prefix `sess_` followed by a UUID v4. Store this ID —
you will pass it with every subsequent request.

---

### `session/list`

Returns all sessions known to the server: those currently in memory, plus any
that were persisted to `.roko/sessions/` in previous runs.

Use this when your editor starts up and wants to let the user pick up where
they left off.

<details>
<summary>Full request/response example</summary>

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "session/list"
}
```

**Response:**

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

</details>

<details>
<summary>Type definitions</summary>

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

</details>

Sessions are sorted ascending by `created_at`, then by `session_id`.

---

### `session/load`

Loads an existing session by ID — either from memory or from the
`.roko/sessions/` directory on disk. Returns the same shape as `session/new`,
so your editor can restore its UI state (config options, modes) from the
response.

After loading, you can send new prompts and the agent will have access to the
full conversation history from the previous session.

<details>
<summary>Full request/response example</summary>

**Request:**

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

**Response:** Same shape as `session/new` result.

Returns error `-32000 SESSION_NOT_FOUND` if the session is not in memory and
not on disk.

</details>

**What is persisted on disk?**

After every successful `session/prompt`, the server writes the session to
`.roko/sessions/{session_id}.json`. This includes:

- `session_id`, `session_name`, `created_at`
- `config_state` (current model, effort, workflow, etc.)
- `conversation_history` (up to 40 turns / 64,000 chars)
- `mcp_servers` attachment list
- `config_options` list
- `active_run` (the last `WorkflowRun` if a pipeline was used)

Sessions 7 days old or older are automatically garbage-collected at server
startup.

---

### `session/prompt`

This is the primary method. It sends a user message to the session and starts
the agent. The method blocks until the agent completes (or is cancelled),
streaming `session/update` notifications throughout.

A prompt is not just text. It is a list of **content blocks**, which can be
text, file references, or diffs. This lets you include the contents of the
currently open file, or the diff of an uncommitted change, as context for the
agent.

When the `includeContext` flag is `true`, Roko also resolves any `@`-mentions
in the text into inline file content injected into the system prompt.

The server returns `-32001 SESSION_BUSY` if a prompt is already in-flight for
this session.

<details>
<summary>Full request/response example (with file context)</summary>

**Request:**

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

**Response (arrives after all notifications):**

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "result": {
    "stopReason": "end_turn"
  }
}
```

</details>

<details>
<summary>Type definitions</summary>

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

</details>

**Slash commands**

If the prompt text starts with `/`, Roko dispatches it as a slash command
instead of forwarding it to the model. Slash commands are instant — they
produce `agent_message_chunk` text events and return quickly, without writing
episodes or querying the knowledge store.

| Command | Effect |
|---|---|
| `/help` | Streams a text chunk listing available commands |
| `/status` | Streams the active `WorkflowRun` status summary |
| `/cancel` | Cancels the active prompt |
| `/mode [code\|plan\|research]` | Gets or sets the agent mode; clears history on change |
| `/workflow [none\|express\|standard\|full\|auto]` | Gets or sets pipeline workflow |
| `/model [key]` | Gets or sets the active model key |
| `/clear` | Clears conversation history |

---

### `session/config/update`

Updates a single configuration option for a session — for example, switching
to a different model, toggling Clippy checks, or changing the workflow. Returns
the full updated options list so your editor can refresh its settings UI.

The legacy alias `session/set_config_option` is also accepted.

<details>
<summary>Full request/response example</summary>

**Request:**

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

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "result": {
    "configOptions": [ ... ]
  }
}
```

</details>

<details>
<summary>Type definitions</summary>

```rust
pub struct ConfigUpdateParams {
    pub session_id: String,
    pub option_id: String,
    pub new_value: serde_json::Value,
}

pub struct ConfigUpdateResult {
    pub config_options: Vec<ConfigOption>,
}
```

</details>

See [Section 6](#6-session-config-options) for the full list of valid option IDs
and their accepted values.

---

### `session/set_mode`

Legacy method to switch the interaction mode (`code`, `plan`, or `research`).
Mode changes clear the conversation history.

Prefer `session/config/update` with `optionId: "workflow"` for pipeline
control. Use `session/set_mode` only if you need to stay compatible with older
integrations.

<details>
<summary>Full request/response example</summary>

**Request:**

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

**Response:** Same shape as `session/config/update` result.

</details>

<details>
<summary>Type definitions</summary>

```rust
pub struct SessionSetModeParams {
    pub session_id: String,
    pub mode_id: String,   // "code" | "plan" | "research"
}
```

</details>

---

## 5. Notifications Reference

Notifications are messages sent without an `id` field. They do not require a
response. Roko uses them for two things:

1. **`session/cancel`** — your editor sends this to tell Roko to stop.
2. **`session/update`** — Roko sends these to stream live updates to your editor.

### `session/cancel` (client → server)

Cooperative cancellation. Send this as a notification (no `id`) while a
`session/prompt` is in flight. Roko sets its cancel token, the streaming loop
detects it on the next iteration, and the `session/prompt` response arrives
with `stopReason: "cancelled"`.

```json
{
  "jsonrpc": "2.0",
  "method": "session/cancel",
  "params": {
    "sessionId": "sess_abc123"
  }
}
```

<details>
<summary>Type definition and cancellation internals</summary>

```rust
pub struct SessionCancelParams {
    pub session_id: String,
}
```

Cancellation is cooperative. The server's cancel token is an `Arc<AtomicBool>`
paired with a `Notify`. On `cancel()`, the flag is set and all waiters are
notified. The streaming loop polls it with `tokio::select!` on every iteration.
There is no forced kill — the agent finishes its current unit of work and then
stops cleanly.

</details>

---

### `session/update` (server → client)

The primary streaming notification. Sent repeatedly during `session/prompt`
execution. The `sessionUpdate` field tells you which variant it is.

The notification envelope always looks like this:

```json
{
  "jsonrpc": "2.0",
  "method": "session/update",
  "params": {
    "sessionId": "sess_abc123",
    "update": { "sessionUpdate": "...", ... }
  }
}
```

---

### All `session/update` variants

Each variant is described below with its mental model, an example, and its
type definition.

#### `agent_message_chunk`

The agent is writing its response. Accumulate these in order to reconstruct
the full text. Think of each chunk as one piece of a streaming response — like
watching a typewriter.

```json
{
  "sessionUpdate": "agent_message_chunk",
  "content": {
    "type": "text",
    "text": "Here is the fix for the login function:\n\n"
  }
}
```

<details>
<summary>Type definition</summary>

```rust
SessionUpdate::AgentMessageChunk {
    content: ContentBlock,
    _meta: Option<serde_json::Value>,   // reserved, always null
}
```

The assistant text is also stored in conversation history (truncated to 10,240
bytes per turn).

</details>

---

#### `agent_thought_chunk`

The model's internal reasoning — the "thinking" that happens before the visible
response. Not part of the final answer. You can display this in a collapsible
"Thinking..." section for power users, or hide it entirely.

```json
{
  "sessionUpdate": "agent_thought_chunk",
  "content": {
    "type": "text",
    "text": "I need to look at the current error handling..."
  }
}
```

<details>
<summary>Type definition</summary>

```rust
SessionUpdate::AgentThoughtChunk {
    content: ContentBlock,
}
```

</details>

---

#### `tool_call`

The agent is starting a new tool action — for example, editing a file or
running a command. Render this as a new card or row in your UI with the tool
name, kind, and a spinner showing it is in progress. The `toolCallId` is the
key that links this to future `tool_call_update` events.

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

<details>
<summary>Type definitions</summary>

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

</details>

---

#### `tool_call_update`

The previously announced tool action has finished (or changed state). Find the
matching card by `toolCallId` and update it. The `content` field may contain a
diff block showing exactly what changed.

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

<details>
<summary>Type definition</summary>

```rust
SessionUpdate::ToolCallUpdate {
    tool_call_id: String,
    status: ToolCallStatus,
    content: Vec<ContentBlock>,
}
```

</details>

---

#### `plan`

The agent has produced a structured multi-step plan. This is different from
free-form text — it is a machine-readable checklist of items with priorities
and statuses. Render this as a visual checklist that updates as items complete.

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

<details>
<summary>Type definitions</summary>

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

</details>

---

#### `available_commands_update`

The set of slash commands has changed. Use this to update your editor's
autocomplete list. Sent immediately after `session/new` and whenever the
command set changes (e.g. when a workflow starts).

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

<details>
<summary>Type definitions</summary>

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

</details>

---

#### `config_option_update`

A config option has changed mid-session. Re-render your settings panel with
the new values.

```json
{
  "sessionUpdate": "config_option_update",
  "configOptions": [ ... ]
}
```

<details>
<summary>Type definition</summary>

```rust
SessionUpdate::ConfigOptionUpdate {
    config_options: Vec<ConfigOption>,
}
```

</details>

---

#### `usage_update`

Token and cost information after a model call completes. Display this in your
status bar or a usage panel.

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

<details>
<summary>Type definitions</summary>

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

</details>

---

#### `session_info_update`

The session's display name has changed.

```json
{
  "sessionUpdate": "session_info_update",
  "sessionId": "sess_abc123",
  "sessionName": "New session name"
}
```

<details>
<summary>Type definition</summary>

```rust
SessionUpdate::SessionInfoUpdate {
    session_id: String,
    session_name: Option<String>,
}
```

</details>

---

## 6. Session Config Options

Config options are the knobs your editor exposes for users to tune how the
agent behaves. They are returned with every `session/new`, `session/load`, and
`session/config/update` response.

Think of them as the settings panel for a single session: model selection,
effort level, which quality gates to run, and what kind of workflow to use.

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

<details>
<summary>ConfigOption shape and server-side SessionConfigState</summary>

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

All option changes update the server-side `SessionConfigState`, which is
persisted with the session:

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

**Defaults** (from `roko.toml`, falling back to these if unconfigured):

| Field | Default |
|---|---|
| `agent_mode` | `"code"` |
| `effort` | `"medium"` |
| `temperament` | `"balanced"` |
| `routing_mode` | `"auto_override"` |
| `clippy_enabled` | `true` |
| `tests_enabled` | `true` |
| `workflow` | `"none"` |
| `review_strictness` | `"none"` |
| `max_iterations` | `2` |

</details>

---

## 7. The Pipeline: Multi-Phase Workflow

When `workflow` is set to anything other than `"none"`, a single user prompt
triggers a full multi-agent pipeline rather than a single model call. This is
Roko's most powerful feature and is worth understanding in depth.

### Why does a pipeline exist?

A naive agent loop is: user asks → model responds → done. This works for
simple questions but falls apart for software engineering tasks:

- The model might write code that does not compile.
- The code might pass compilation but fail tests.
- The diff might be technically correct but stylistically inconsistent.
- A large refactor needs upfront strategy before implementation.

The pipeline addresses each of these with specialized phases: strategy,
implementation, automated gate checks (compile/test/lint), auto-fix of
failures, peer review, and a final commit.

### Workflow templates

There are three templates, each a different subset of phases:

| Template | Phases | Best for |
|---|---|---|
| `express` | Implement → Gate → Commit | Small, unambiguous changes |
| `standard` | Implement → Gate → Review → Commit | Most tasks |
| `full` | Strategy → Implement → Gate → Review → Commit | Large or architectural changes |

When `workflow = "auto"`, Roko picks a template based on your prompt text:

- **Express**: prompt contains `fix`, `typo`, `rename`, `update`, or `bump`,
  AND the prompt is fewer than 15 words.
- **Full**: prompt contains `files`, `modules`, `system`, `architecture`, or
  `refactor`, OR the prompt is longer than 50 words.
- **Standard**: everything else.

### Phase state machine

```
Pending ──Start──► Strategizing (Full only)
Pending ──Start──► Implementing (Express, Standard)

Strategizing ──StrategyComplete──► Implementing
Strategizing ──StrategySkipped──► Implementing

Implementing ──AgentCompleted──► Gating
Implementing ──AgentFailed──► Implementing (retry, if iterations remain)
Implementing ──AgentFailed──► Halted (no iterations left)

Gating ──GatesPassed──► Reviewing (Standard, Full)
Gating ──GatesPassed──► Committing (Express)
Gating ──GateFailed──► AutoFixing (if iterations remain)
Gating ──GateFailed──► Halted (no iterations left)

AutoFixing ──AgentCompleted──► Gating
AutoFixing ──AgentFailed──► Implementing (retry with gate+autofix context)
AutoFixing ──AgentFailed──► Halted (no iterations left)

Reviewing ──ReviewApproved──► Committing
Reviewing ──ReviewRevise──► Implementing (retry with review feedback)
Reviewing ──ReviewRevise──► Committing (no iterations left: accept with caveats)

Committing ──CommitDone──► Complete

Any ──UserCancel──► Cancelled
Any ──Timeout──► Halted
Any ──BudgetExceeded──► Halted
```

Phase transitions appear in the stream as `agent_message_chunk` events:
`"[Phase: Implementing -> Gating]\n"`.

### What you see as session/update events during a pipeline

During a pipeline run, the `session/update` stream looks like:

```
← tool_call (title: "Prior knowledge - 2 results", kind: Other)
← tool_call_update (Prior knowledge - completed)
← tool_call (title: "Agent: strategy", kind: Other)
← agent_message_chunk "[Phase: Pending -> Strategizing]\n"
← tool_call_update (Agent: strategy - completed)
← tool_call (title: "Agent: implementer", kind: Other)
← agent_message_chunk "[Phase: Strategizing -> Implementing]\n"
← agent_message_chunk "I'll add error handling by..."
← tool_call (title: "Edit src/auth/login.rs", kind: Edit)
← tool_call_update (Edit - completed, diff: "...")
← tool_call_update (Agent: implementer - completed)
← tool_call (title: "Gate: compile", kind: Other)
← agent_message_chunk "[Phase: Implementing -> Gating]\n"
← tool_call_update (Gate: compile - completed)
← tool_call (title: "Agent: reviewer", kind: Other)
← agent_message_chunk "[Phase: Gating -> Reviewing]\n"
← tool_call_update (Agent: reviewer - completed)
← agent_message_chunk "[Phase: Reviewing -> Committing]\n"
← agent_message_chunk "chore: add error handling to login function"
← agent_message_chunk "[Phase: Committing -> Complete]\n"
← usage_update
← session/prompt result (stopReason: "end_turn")
```

<details>
<summary>PipelinePhase enum, WorkflowRun, PipelineConfig types</summary>

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

pub enum WorkflowTemplate {
    Express,   // Implement → Gate → Commit
    Standard,  // Implement → Gate → Review → Commit
    Full,      // Strategy → Implement → Gate → Review → Commit
}
```

`WorkflowRun` wraps `PipelineState` with timing and cost metadata. It is stored
as `session.active_run` after each pipeline execution:

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

</details>

<details>
<summary>Gate error classification and review finding types</summary>

When a gate fails, the runner classifies the error for targeted auto-fix
context:

```rust
enum GateErrorType {
    CompileError { file: String, line: u32, message: String },
    TestFailure  { test_name: String, expected: Option<String>, actual: Option<String> },
    ClippyWarning { lint: String, location: String },
    RuntimePanic  { message: String },
    Unknown,
}
```

Gate results are expressed as:

```rust
pub struct GateResult {
    pub gate: String,        // "compile" | "test" | "clippy" | "fmt"
    pub passed: bool,
    pub output: String,
    pub duration_ms: u64,
}
```

Review findings as:

```rust
pub struct ReviewFinding {
    pub severity: String,    // "major" | "minor" | "nit"
    pub description: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}
```

</details>

### AcpAdapter: workflow events → session/update

For pipeline dispatches, an internal `AcpAdapter` translates `RuntimeEvent`
from the workflow engine into `CognitiveEvent`, which then maps to
`session/update` notifications. Here is the full mapping:

<details>
<summary>RuntimeEvent → CognitiveEvent → session/update translation table</summary>

| `RuntimeEvent` | `CognitiveEvent` | `session/update` |
|---|---|---|
| `AgentOutput { chunk }` | `TokenChunk(chunk)` | `agent_message_chunk` |
| `AgentSpawned { agent_id, role }` | `ToolCallStart { title: "Agent: {role}", kind: Other }` | `tool_call` |
| `AgentCompleted { agent_id, output }` | `ToolCallComplete { status: Completed, content: [text] }` | `tool_call_update` |
| `AgentFailed { agent_id, error }` | `ToolCallComplete { status: Failed, content: [text] }` | `tool_call_update` |
| `GateStarted { gate_name }` | `ToolCallStart { title: "Gate: {gate_name}", kind: Other }` | `tool_call` |
| `GatePassed { gate_name }` | `ToolCallComplete { status: Completed, content: ["{gate_name} passed"] }` | `tool_call_update` |
| `GateFailed { gate_name, output }` | `ToolCallComplete { status: Failed, content: [output] }` | `tool_call_update` |
| `PhaseTransition { from, to }` | `TokenChunk("[Phase: {from} -> {to}]\n")` | `agent_message_chunk` |
| `WorkflowCompleted { outcome: Cancelled }` | `Complete { stop_reason: Cancelled }` | (terminal) |
| `WorkflowCompleted { outcome: Success/Halted }` | `Complete { stop_reason: EndTurn }` | (terminal) |
| `WorkflowStarted`, `FeedbackRecorded`, `StateCheckpointed` | (ignored) | — |

The adapter filters by `run_id` to avoid receiving events from concurrent runs.

</details>

**File change notifications after commit**

After a pipeline commit phase, Roko detects changed files via
`git diff --name-status HEAD~1 HEAD` and emits a `tool_call`/`tool_call_update`
pair per changed file (skipping lock files and images).

<details>
<summary>FileChangeNotification type</summary>

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

</details>

---

## 8. Knowledge Integration

Before every non-slash-command prompt, Roko automatically runs two parallel
knowledge lookups to give the agent relevant context from past work. You do not
need to do anything to trigger this — it is automatic.

Understanding it helps you build better UI for the "Prior knowledge" card that
appears before the agent's main response.

### What is the knowledge store?

`roko-neuro` is Roko's durable knowledge store. Over time, as the agent
completes tasks, it accumulates insights, heuristics, warnings, and causal
links. These are stored at `.roko/neuro/` and indexed for semantic search.

When you ask the agent to "add error handling to the login function," Roko
first searches the knowledge store for anything relevant — maybe a heuristic
about error handling patterns in this codebase, or a past anti-pattern to avoid.

### What are playbooks?

`roko-learn` maintains playbooks — recorded patterns of successful multi-step
tasks. For example, "how to resolve Send + Sync errors" might be a playbook
with steps like "replace HashMap with DashMap" and "run cargo test to confirm."
Playbooks have success/failure counts, so Roko can weight them by reliability.

### What your editor sees

Both lookups complete before the agent starts, and Roko sends them as a
`tool_call` / `tool_call_update` pair:

```
← tool_call (title: "Prior knowledge - 3 results", kind: Other, status: in_progress)
← tool_call_update (status: completed, content: [text block with formatted results])
```

The text block looks like:

```
**Playbooks:**
  - Resolve Send + Sync errors (75% success)
**Knowledge:**
  - [P] 0.91 - Prefer smaller retries after gate failures...
```

Where `[P]` is the knowledge tier label (see below).

<details>
<summary>Knowledge store types, tiers, and query details</summary>

**Knowledge store query:**

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
    pub kind: KnowledgeKind,
    pub content: String,
    pub confidence: f64,
    pub tier: KnowledgeTier,
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

**Knowledge tiers** (shown as single-letter labels in the card):

| `KnowledgeTier` | Label | Meaning |
|---|---|---|
| `Persistent` | `P` | Long-lived, high-confidence facts |
| `Consolidated` | `C` | Distilled from working memory |
| `Working` | `W` | Recent session knowledge |
| `Transient` | `T` | Ephemeral, not yet validated |

**Knowledge kinds:**

| `KnowledgeKind` | Label |
|---|---|
| `Insight` | `insight` |
| `Heuristic` | `heuristic` |
| `AntiKnowledge` | `anti-pattern` |
| `Warning` | `warning` |
| `CausalLink` | `causal` |
| `StrategyFragment` | `strategy` |

**Playbook store query:**

```rust
PlaybookStore::new(workdir.join(".roko/learn/playbooks"))
    .relevant(&prompt_text, 3)
    .await
```

Returns up to 3 relevant `Playbook` entries:

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

**Context injected into the system prompt:**

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

Both lookups run in parallel via `tokio::join!`. If either store is unavailable
or the query fails, the error is logged and an empty result is returned so
dispatch can continue.

</details>

---

## 9. Episode Logging

Every completed `session/prompt` (including pipeline dispatches) is appended
to `.roko/episodes.jsonl`. This is the same episode format used by Roko's
orchestrator — it is the audit trail and learning input for the whole system.

You do not need to do anything. This happens automatically. But understanding
the format helps if you want to build tooling that reads the episode log.

<details>
<summary>Episode fields written by ACP</summary>

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

`success` is `true` only when `stop_reason == EndTurn` and no errors occurred.

</details>

---

## 10. roko.toml Configuration

The ACP server loads `roko.toml` from the working directory at startup. This
file controls the initial session config values — model selection, effort,
gate settings, and routing. If the file is missing, Roko uses built-in defaults.

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

<details>
<summary>AcpConfig struct (server-side configuration)</summary>

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

</details>

---

## 11. Troubleshooting and FAQ

### Error codes

When something goes wrong, the server sends an error response instead of a
result. The `code` field tells you what happened:

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

### Full session lifecycle diagram

This shows the complete message sequence from start to finish:

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
  │◄── notification: session/update ───────── │  (agent_message_chunk: commit message)
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

### Conversation history limits

The server maintains in-session multi-turn history:

- Max turns: 40
- Max total characters: 64,000
- Oldest turns dropped first (FIFO) when either limit is exceeded
- History is cleared when the mode changes via `session/set_mode`
- Assistant response text is truncated to 10,240 bytes before storage

### Busy state: how does the server serialize concurrent prompts?

A session allows only one in-flight prompt at a time. The `busy` flag is an
`Arc<AtomicBool>`. On `session/prompt` arrival:

1. Check `busy` — if true, return `-32001 SESSION_BUSY`.
2. Reset the cancel token (a new `CancelToken` is created).
3. Set `busy = true`.
4. Run the cognitive task.
5. Set `busy = false` (even on error or cancellation).
6. Persist the session to disk.

If your editor needs to send multiple concurrent prompts, create multiple
sessions.

### How do I resume a session across editor restarts?

```json
// List persisted sessions (at startup)
{"jsonrpc":"2.0","id":1,"method":"session/list"}

// Load the one you want
{"jsonrpc":"2.0","id":2,"method":"session/load","params":{"sessionId":"sess_abc123"}}

// Continue prompting with history restored
{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
  "sessionId":"sess_abc123",
  "prompt":[{"type":"text","text":"What did we just implement?"}],
  "includeContext":false
}}
```

### What should I render for each update type?

| `sessionUpdate` value | Recommended UI action |
|---|---|
| `agent_message_chunk` | Append `content.text` to the chat message bubble |
| `agent_thought_chunk` | Show in a collapsible "Thinking..." section, or hide |
| `tool_call` | Create a new expandable card: tool name, kind icon, spinner |
| `tool_call_update` | Update the card matching `toolCallId`; show diff if present |
| `plan` | Render a checklist with priority badges and status indicators |
| `available_commands_update` | Update slash command autocomplete |
| `config_option_update` | Re-render the settings panel |
| `usage_update` | Show token count and cost in the status bar |
| `session_info_update` | Update the session tab title |

### How do I enable the full pipeline workflow?

```json
// Switch to full pipeline
{"jsonrpc":"2.0","id":10,"method":"session/config/update","params":{
  "sessionId":"sess_...",
  "optionId":"workflow",
  "newValue":"full"
}}

// Set thorough review
{"jsonrpc":"2.0","id":11,"method":"session/config/update","params":{
  "sessionId":"sess_...",
  "optionId":"review_strictness",
  "newValue":"thorough"
}}
```

### Where do logs go?

All server-side logging goes to `.roko/acp.log` (never stdout). The log level
is controlled by the `RUST_LOG` environment variable, defaulting to
`roko_acp=debug`. Logs are non-blocking via `tracing_appender`.

Key log events to look for when debugging:

| Log message | Meaning |
|---|---|
| `"ACP logging initialized"` | Startup with protocol_version, spec_version, log_file |
| `"loaded roko.toml configuration"` | With provider and model counts |
| `"handling ACP request"` | Method name and request id |
| `"handling ACP session prompt"` | session_id, prompt chars, model_key, workdir |
| `"failed to append ACP episode"` | Episode logging failed (non-fatal) |
| `"ACP client disconnected while prompt was active"` | EOF during streaming |

---

## 12. Advanced: Transport Internals

Most editor integration authors do not need to read this section. It documents
the internal mechanics of how the stdio transport works, which is useful if
you are debugging message ordering issues or building a non-standard transport.

<details>
<summary>StdioTransport implementation details</summary>

The `StdioTransport` uses:

- `BufReader` over stdin for newline-delimited JSON parsing.
- `Arc<AsyncMutex<W>>` for the writer, enabling clones to share it.
- `Arc<AtomicU64>` for auto-incrementing outbound request IDs (starting at 1).
- `Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>` for tracking
  pending outbound requests.

The transport can send requests *to the client* (for bridging scenarios) and
match incoming responses via `handle_incoming_response`. This is used for
`fs/read_text_file` and `fs/write_text_file` when `clientCapabilities.fs` is
set.

Every message is flushed immediately after writing. No framing headers are
used.

**Reading:**

```rust
let mut line = String::new();
reader.read_line(&mut line).await?;   // blocks until \n
let message = serde_json::from_str::<JsonRpcMessage>(&line)?;
```

**Writing:**

```rust
let bytes = serde_json::to_vec(message)?;
writer.write_all(&bytes).await?;
writer.write_all(b"\n").await?;
writer.flush().await?;
```

</details>

<details>
<summary>Conversation history wire format per provider type</summary>

For **CLI-based providers** (Claude CLI), history is serialised as XML:

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

For **API-based providers**, history becomes the standard messages array:

```json
[
  {"role": "system",    "content": "...system prompt..."},
  {"role": "user",      "content": "First user message"},
  {"role": "assistant", "content": "First assistant response"},
  {"role": "user",      "content": "Current prompt"}
]
```

</details>

<details>
<summary>Legacy pipeline runner</summary>

The `ROKO_ACP_LEGACY` environment variable selects the legacy pipeline runner
instead of the `WorkflowEngine`-based path. Set this only for debugging. The
default (workflow engine) path is the canonical one.

</details>

---

## 13. Known Limitations

- **Single transport only.** The ACP server is single-threaded on the stdio
  channel. Concurrent sessions are supported in memory, but concurrent
  *transports* (e.g. multiple TCP clients) are not. The `SessionManager` is
  not wrapped in `Arc<RwLock<_>>`.

- **No image or audio input.** `promptCapabilities.image = false`,
  `promptCapabilities.audio = false`.

- **WebSocket/SSE not on ACP path.** WebSocket and SSE streaming are available
  via the HTTP control plane (`roko serve` on `:6677`), not through the ACP
  stdio channel. ACP uses stdio only.

- **Outbound `fs/read_text_file` and `fs/write_text_file`** are implemented in
  the transport layer (pending request registry) but the bridge does not yet
  use them automatically. File content is resolved server-side by reading the
  workdir directly.

- **`ROKO_ACP_LEGACY`** env var selects the legacy pipeline runner instead of
  the `WorkflowEngine`-based path. Set this only for debugging; the default
  (workflow engine) path is the canonical one.
