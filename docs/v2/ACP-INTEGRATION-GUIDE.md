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
All diagnostics and traces go to a log file (`.roko/acp.log` by default). Never
read or write any non-protocol content to stdout when ACP is active.

### Supported editors

Any editor that can spawn a child process and drive it via stdin/stdout supports
ACP. Known working targets:

- JetBrains AI Assistant
- Zed
- Neovim (via plugin)
- VS Code (via extension)
- Any editor that implements the ACP client spec

### Capabilities advertised by Roko

| Capability | Value |
|---|---|
| `loadSession` | `true` — persisted sessions can be resumed |
| `promptCapabilities.image` | `false` |
| `promptCapabilities.audio` | `false` |
| `promptCapabilities.embeddedContext` | `true` |
| `mcpCapabilities.http` | `true` |
| `mcpCapabilities.sse` | `true` |

---

## Getting Started

### Running the server

```bash
# Minimal invocation (working directory = current dir)
roko acp

# Explicit working directory
roko acp --workdir /path/to/project

# With explicit config file
roko acp --workdir /path/to/project --config /path/to/roko.toml

# Override log file location
roko acp --workdir /path/to/project --log-file /tmp/roko-acp.log

# Named profile (matches a section in roko.toml)
roko acp --profile staging
```

The process exits when stdin reaches EOF. Exit code 0 means clean shutdown;
non-zero means a fatal transport or initialization error.

### Connecting from an editor plugin

Your editor plugin should:

1. Spawn `roko acp --workdir <project-root>` as a child process.
2. Send one JSON object per line on stdin.
3. Read one JSON object per line from stdout.
4. Send `initialize` first, before any other message.
5. Call `session/new` to get a session ID for the workspace.
6. Use `session/prompt` to drive the agent. Handle `session/update` notifications
   while the prompt is in flight.

---

## Protocol

### Wire format

Every message is a complete JSON object on a single line terminated by `\n`.
Messages may arrive interleaved (notifications can come between a request and its
response). Parse each line independently.

```
{...}\n
{...}\n
```

### Message types

**Request** (client → server, expects a response):

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}
```

**Response** (server → client, success):

```json
{"jsonrpc":"2.0","id":1,"result":{...}}
```

**Response** (server → client, failure):

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"..."}}
```

**Notification** (either direction, no response expected):

```json
{"jsonrpc":"2.0","method":"session/update","params":{...}}
```

The `id` field is a number or string. Null is only used in parse-level error
responses when no id could be parsed.

---

## Session Lifecycle

The complete lifecycle for a single coding session looks like this:

```
Client                                  Server
  |                                       |
  |--- initialize --->                    |
  |<-- initialize result ---              |
  |                                       |
  |--- session/new --->                   |
  |<-- session/new result ---             |
  |<-- session/update (commands) ---      |
  |                                       |
  |--- session/prompt --->                |
  |<-- session/update (tool_call) ---     |  (knowledge card)
  |<-- session/update (chunk) ---         |  (streaming tokens)
  |<-- session/update (tool_call) ---     |  (tool invocations)
  |<-- session/update (plan) ---          |  (progress plan)
  |<-- session/prompt result ---          |
  |                                       |
  |--- session/prompt --->                |  (follow-up turn)
  |   ...                                 |
  |                                       |
  |--- session/cancel (notification) -->  |  (optional, any time during prompt)
  |<-- session/prompt result (cancelled)  |
  |                                       |
  |   [stdin closed / process exits]      |
```

---

## Request Types

### `initialize`

Sent once, before any other request. Negotiates protocol version and exchanges
capabilities.

**Request params:**

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
        "writeTextFile": true
      },
      "terminal": true,
      "mcpServers": true
    },
    "clientInfo": {
      "name": "my-editor-plugin",
      "version": "0.1.0",
      "title": "My Editor"
    }
  }
}
```

`clientCapabilities` is optional. Omit `clientInfo` for anonymous clients.

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

---

### `session/new`

Creates a new ACP session. Returns a session ID used in all subsequent calls.
After a successful `session/new`, the server immediately sends a
`session/update` notification containing the available slash commands.

**Request params:**

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "session/new",
  "params": {
    "sessionName": "my-workspace",
    "clientCapabilities": {
      "terminal": true
    },
    "mcpServers": [
      {
        "name": "code-intel",
        "transport": {
          "type": "stdio",
          "command": "roko",
          "args": ["mcp-code"]
        }
      },
      {
        "name": "remote-api",
        "transport": {
          "type": "http",
          "url": "http://localhost:9000"
        }
      }
    ]
  }
}
```

All fields are optional. `sessionName` is displayed in session lists.
`mcpServers` attaches MCP tools to the session.

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
        {
          "id": "code",
          "name": "Code",
          "description": "Implement and edit code directly."
        },
        {
          "id": "plan",
          "name": "Plan",
          "description": "Focus on planning before execution."
        },
        {
          "id": "research",
          "name": "Research",
          "description": "Gather context and analyze options."
        }
      ]
    },
    "configOptions": [
      {
        "id": "model",
        "name": "Model",
        "type": "select",
        "category": "agent",
        "currentValue": "sonnet",
        "description": "Language model",
        "options": [
          {"value": "sonnet", "name": "sonnet (anthropic)", "description": "claude-sonnet-4-6"}
        ]
      },
      {
        "id": "workflow",
        "name": "Workflow",
        "type": "select",
        "category": "execution",
        "currentValue": "none",
        "description": "Pipeline workflow for prompts",
        "options": [
          {"value": "none", "name": "None", "description": "Single agent, no pipeline"},
          {"value": "express", "name": "Express", "description": "Implement → gate → commit (fastest)"},
          {"value": "standard", "name": "Standard", "description": "Implement → gate → review → commit"},
          {"value": "full", "name": "Full", "description": "Strategy → implement → gate → multi-review → commit"},
          {"value": "auto", "name": "Auto", "description": "Select pipeline based on complexity"}
        ]
      }
    ]
  }
}
```

Session IDs always start with `sess_` followed by a UUID.

**Follow-up notification (sent immediately after response):**

```json
{
  "jsonrpc": "2.0",
  "method": "session/update",
  "params": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
    "update": {
      "sessionUpdate": "available_commands_update",
      "availableCommands": [
        {"name": "status", "description": "Workspace status: signals, agents, runs, knowledge"},
        {"name": "research", "description": "Deep research a topic with citations (Perplexity)", "input": {"hint": "topic to research"}},
        ...
      ]
    }
  }
}
```

---

### `session/prompt`

Sends a user message to the agent. This is the main interaction method.

During execution the server sends `session/update` notifications on stdout.
The `session/prompt` response arrives after all notifications for that turn
are complete.

**Request params:**

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "session/prompt",
  "params": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
    "prompt": [
      {
        "type": "text",
        "text": "Add input validation to the login form"
      }
    ],
    "includeContext": true
  }
}
```

**Prompt content blocks:**

The `prompt` array can contain multiple blocks:

```json
[
  {"type": "text", "text": "Review this file for security issues:"},
  {"type": "resource", "resource": {"type": "file", "uri": "file:///path/to/auth.rs"}},
  {"type": "diff", "path": "src/main.rs", "diff": "--- a/src/main.rs\n+++ b/src/main.rs\n..."}
]
```

- `text` — plain text instruction
- `resource` — file reference (the server reads the file at `uri`)
- `diff` — unified diff for the agent to review

`includeContext` tells the server to automatically include workspace context
(current file, open buffers) in the agent's context window. Set `false` to send
only what you explicitly provide in `prompt`.

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "stopReason": "end_turn"
  }
}
```

`stopReason` values:

| Value | Meaning |
|---|---|
| `end_turn` | Agent completed normally |
| `max_tokens` | Hit token limit |
| `max_turn_requests` | Hit max turns limit |
| `refusal` | Agent refused to answer |
| `cancelled` | Cancelled via `session/cancel` |

---

### `session/list`

Returns all known sessions (in-memory and persisted on disk).

**Request:**

```json
{"jsonrpc":"2.0","id":4,"method":"session/list","params":{}}
```

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "sessions": [
      {
        "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
        "sessionName": "my-workspace",
        "createdAt": "2026-04-29T10:30:00Z"
      },
      {
        "sessionId": "sess_661f9511-f30c-52e5-b827-557766551111",
        "sessionName": null,
        "createdAt": "2026-04-28T14:22:00Z"
      }
    ]
  }
}
```

Sessions are sorted by creation time, oldest first.

---

### `session/load`

Loads a persisted session back into memory. Returns the same shape as
`session/new`. Use this to resume a previous session after restart.

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "session/load",
  "params": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Response:** Same shape as `session/new` result, including `modes` and
`configOptions`.

**Error:** `-32000` (`SESSION_NOT_FOUND`) if the session does not exist in memory
or on disk.

---

### `session/config/update`

Updates a single session configuration option. The server applies the change
immediately and returns the full updated set of options.

Also accepted as `session/set_config_option` (legacy alias).

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "session/config/update",
  "params": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
    "optionId": "workflow",
    "newValue": "standard"
  }
}
```

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "result": {
    "configOptions": [
      {
        "id": "workflow",
        "name": "Workflow",
        "type": "select",
        "category": "execution",
        "currentValue": "standard",
        ...
      },
      ...
    ]
  }
}
```

See the [Configuration](#configuration) section for all option IDs and values.

---

### `session/set_mode`

Legacy method. Changes the agent interaction mode. Switching modes clears
conversation history.

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "session/set_mode",
  "params": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
    "modeId": "plan"
  }
}
```

**Response:** Same shape as `session/config/update` — returns updated
`configOptions`.

Mode IDs: `code`, `plan`, `research`.

---

### `session/cancel` (notification)

Sent by the client to cancel the in-flight `session/prompt` request. This is a
notification — no response is expected or sent.

```json
{
  "jsonrpc": "2.0",
  "method": "session/cancel",
  "params": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000"
  }
}
```

The server signals the running cognitive task to stop. The pending
`session/prompt` response will arrive with `stopReason: "cancelled"` (or
`end_turn` if the task finished before the cancel was processed).

You can send `session/cancel` at any point while a `session/prompt` is in flight.
The server reads from stdin concurrently during streaming, so this notification
is processed without waiting for the current turn to finish.

---

## Session Update Notifications

While a `session/prompt` is executing, the server sends `session/update`
notifications for every significant event. Each notification looks like:

```json
{
  "jsonrpc": "2.0",
  "method": "session/update",
  "params": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
    "update": { ... }
  }
}
```

The `update` object has a `sessionUpdate` discriminant field that identifies the
variant.

### `agent_message_chunk`

Streaming text output from the agent. Concatenate these to build the full
assistant response.

```json
{
  "sessionUpdate": "agent_message_chunk",
  "content": {
    "type": "text",
    "text": "I'll add input validation to the login "
  }
}
```

```json
{
  "sessionUpdate": "agent_message_chunk",
  "content": {
    "type": "text",
    "text": "form by checking..."
  }
}
```

### `agent_thought_chunk`

Internal reasoning from the model (extended thinking mode). Render separately
from message chunks, typically collapsed or hidden by default.

```json
{
  "sessionUpdate": "agent_thought_chunk",
  "content": {
    "type": "text",
    "text": "The user wants validation. I should check what fields exist..."
  }
}
```

### `tool_call`

A new tool card has been created. The tool is pending or in progress.

```json
{
  "sessionUpdate": "tool_call",
  "toolCallId": "gate-compile",
  "title": "Gate: compile",
  "kind": "other",
  "status": "pending",
  "content": []
}
```

Tool call kinds: `edit`, `create`, `delete`, `terminal`, `other`.

Tool call statuses: `pending`, `in_progress`, `completed`, `failed`.

Common tool call IDs you will see:

| `toolCallId` | What it means |
|---|---|
| `gate-compile` | `cargo build` gate running |
| `gate-test` | `cargo test` gate running |
| `gate-clippy` | `cargo clippy` gate running |
| `gate-fmt` | `cargo fmt` gate running |
| `<uuid>` (agent ID) | An agent is running (implementer, reviewer, etc.) |
| `knowledge` | Prior knowledge lookup |
| `provenance` | Source provenance lookup |

### `tool_call_update`

Status or content update for an existing tool card.

```json
{
  "sessionUpdate": "tool_call_update",
  "toolCallId": "gate-compile",
  "status": "completed",
  "content": [
    {"type": "text", "text": "compile passed"}
  ]
}
```

```json
{
  "sessionUpdate": "tool_call_update",
  "toolCallId": "gate-test",
  "status": "failed",
  "content": [
    {"type": "text", "text": "test failed\nerror[E0001]: ..."}
  ]
}
```

### `plan`

A structured plan update. Emitted when a workflow pipeline produces a plan of
work. Render this as a progress list in your UI.

```json
{
  "sessionUpdate": "plan",
  "entries": [
    {
      "content": "Analyze login form fields",
      "priority": "high",
      "status": "completed"
    },
    {
      "content": "Add validation for email field",
      "priority": "high",
      "status": "in_progress"
    },
    {
      "content": "Add validation for password field",
      "priority": "medium",
      "status": "pending"
    }
  ]
}
```

Plan entry priorities: `high`, `medium`, `low`.
Plan entry statuses: `pending`, `in_progress`, `completed`.

### `available_commands_update`

Sent once after `session/new` completes, listing all available slash commands
for the session.

```json
{
  "sessionUpdate": "available_commands_update",
  "availableCommands": [
    {
      "name": "status",
      "description": "Workspace status: signals, agents, runs, knowledge"
    },
    {
      "name": "research",
      "description": "Deep research a topic with citations (Perplexity)",
      "input": {"hint": "topic to research"}
    }
  ]
}
```

Use this list to drive slash-command autocompletion in your editor.

### `config_option_update`

Sent when config options change (e.g., after model routing selects a different
model automatically).

```json
{
  "sessionUpdate": "config_option_update",
  "configOptions": [
    {
      "id": "model",
      "name": "Model",
      "type": "select",
      "category": "agent",
      "currentValue": "opus",
      ...
    }
  ]
}
```

### `usage_update`

Token usage and cost information.

```json
{
  "sessionUpdate": "usage_update",
  "used": 4821,
  "size": 200000,
  "cost": {
    "amount": 0.0241,
    "currency": "USD"
  }
}
```

`used` is tokens consumed, `size` is the context window capacity.

### `session_info_update`

Session metadata update (session name change, etc.).

```json
{
  "sessionUpdate": "session_info_update",
  "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
  "sessionName": "login-validation-work"
}
```

---

## CognitiveEvent to SessionUpdate Mapping

Internally, the server runs cognitive tasks that emit `CognitiveEvent` values.
These are mapped to `session/update` notifications for the editor:

| CognitiveEvent | SessionUpdate variant |
|---|---|
| `TokenChunk(text)` | `agent_message_chunk` with `text` content |
| `ThinkingChunk(text)` | `agent_thought_chunk` with `text` content |
| `ToolCallStart { id, title, kind }` | `tool_call` with `status: pending` |
| `ToolCallComplete { id, status, content }` | `tool_call_update` |
| `PlanUpdate { entries }` | `plan` |
| `Complete { stop_reason, usage }` | (triggers final `session/prompt` response) |
| `MaxTokens` | (triggers response with `stopReason: max_tokens`) |

The `RuntimeEvent` adapter additionally maps workflow engine events:

| RuntimeEvent | CognitiveEvent |
|---|---|
| `AgentOutput { chunk }` | `TokenChunk(chunk)` |
| `AgentSpawned { agent_id, role }` | `ToolCallStart` with `kind: other` |
| `AgentCompleted { agent_id, output }` | `ToolCallComplete` with `status: completed` |
| `AgentFailed { agent_id, error }` | `ToolCallComplete` with `status: failed` |
| `GateStarted { gate_name }` | `ToolCallStart` with `kind: other` |
| `GatePassed { gate_name }` | `ToolCallComplete` with `status: completed` |
| `GateFailed { gate_name, output }` | `ToolCallComplete` with `status: failed` |
| `PhaseTransition { from, to }` | `TokenChunk("[Phase: {from} -> {to}]\n")` |
| `WorkflowCompleted { outcome }` | `Complete` |

---

## Configuration

Every session has a `SessionConfigState` that controls model selection,
workflow pipelines, gates, and agent behavior. Config options are returned in
`session/new` and updated with `session/config/update`.

### Option reference

**Category: `agent`**

| `optionId` | Type | Values | Default | Description |
|---|---|---|---|---|
| `model` | `select` | Keys from `[models.*]` in `roko.toml` | First available | Language model to use |
| `effort` | `select` | `low`, `medium`, `high`, `max` | `medium` | Agent reasoning depth |
| `temperament` | `select` | `conservative`, `balanced`, `aggressive`, `exploratory` | `balanced` | Risk appetite |

**Category: `routing`**

| `optionId` | Type | Values | Default | Description |
|---|---|---|---|---|
| `routing_mode` | `select` | `auto_override`, `manual` | `auto_override` | Model routing strategy |

`auto_override` lets the cascade router pick the best model based on task
complexity and past performance. `manual` always uses the selected model.

**Category: `gates`**

| `optionId` | Type | Values | Default | Description |
|---|---|---|---|---|
| `clippy` | `select` | `on`, `off` | `on` | Run `cargo clippy` after changes |
| `tests` | `select` | `on`, `off` | `on` | Run `cargo test` after changes |

**Category: `execution`**

| `optionId` | Type | Values | Default | Description |
|---|---|---|---|---|
| `workflow` | `select` | `none`, `express`, `standard`, `full`, `auto` | `none` | Pipeline workflow |
| `review_strictness` | `select` | `none`, `quick`, `standard`, `thorough` | `none` | Review depth |
| `max_iterations` | `select` | `1`, `2`, `3` | `2` | Max retry iterations on failure |

### Example: switch to express workflow

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "session/config/update",
  "params": {
    "sessionId": "sess_550e8400-e29b-41d4-a716-446655440000",
    "optionId": "workflow",
    "newValue": "express"
  }
}
```

---

## Workflow Templates

When `workflow` is set to anything other than `none`, the server runs a
multi-phase pipeline instead of a single agent dispatch.

### Express (`express`)

Fastest path. Suitable for small focused changes.

```
Implement → Gate (compile + test + clippy) → Commit
```

- No strategist phase
- No review phase
- Single agent does the work
- Gates validate before commit

### Standard (`standard`)

Balanced quality/speed. Good for feature additions and bug fixes.

```
Implement → Gate → Review → Commit
```

- No strategist phase
- Single review pass before commit
- If review finds issues, implementation retries (up to `max_iterations`)

### Full (`full`)

Highest quality. Suitable for complex, cross-file changes.

```
Strategy → Implement → Gate → Multi-review → Commit
```

- Strategist agent analyzes the prompt and produces a brief
- Implementation receives the brief as context
- Review can request revisions (up to `max_iterations`)

### Auto (`auto`)

The server selects Express, Standard, or Full automatically based on prompt
characteristics:

- **Express**: prompt contains "fix", "typo", "rename", "update", "bump" and is
  fewer than 15 words
- **Full**: prompt contains "files", "modules", "system", "architecture",
  "refactor", or is more than 50 words
- **Standard**: everything else

### Pipeline phases

| Phase | Description |
|---|---|
| `Pending` | Created but not started |
| `Strategizing` | Strategist agent analyzing the prompt |
| `Implementing` | Implementer agent writing code |
| `AutoFixing` | Auto-fixer agent patching gate failures |
| `Gating` | Gates (compile, test, clippy) running |
| `Reviewing` | Reviewer agent analyzing changes |
| `Committing` | Creating the commit |
| `Complete` | Success |
| `Halted` | Stopped due to timeout, budget, or too many failures |
| `Cancelled` | Cancelled by user |

---

## Agent Modes

Each session has an active mode that controls the system prompt and agent
behavior. Switch with `session/set_mode` or by including `/plan` or `/research`
in the prompt as a slash command.

| Mode ID | Behavior |
|---|---|
| `code` | Expert code implementer: minimal targeted changes, follows existing patterns |
| `plan` | Software architect: decomposes tasks, no implementation, structured plans with numbered steps |
| `research` | Technical researcher: broad search, cites files and line numbers, recommends options, no changes |

Switching modes clears conversation history.

---

## Knowledge Integration

On every non-slash-command prompt, the server automatically queries:

1. **Durable knowledge store** (`roko-neuro`): `.roko/learn/` — ranked hits from
   the workspace knowledge base. Hit scores include keyword matching, confidence,
   recency, and HDC vector similarity.

2. **Playbook store** (`roko-learn`): `.roko/learn/playbooks/` — past successful
   task sequences, up to 3 most relevant playbooks.

The results appear as a tool card in the editor:

```json
{
  "sessionUpdate": "tool_call",
  "toolCallId": "knowledge",
  "title": "Prior knowledge - 3 results",
  "kind": "other",
  "status": "completed",
  "content": [
    {
      "type": "text",
      "text": "**Playbooks:**\n  - fix-concurrency: Resolve Send + Sync errors (75% success)\n**Knowledge:**\n  - [P] 0.91 - Prefer smaller retries after gate failures..."
    }
  ]
}
```

The knowledge context is also injected into the agent's system prompt
(invisible to the editor). The injection is bounded to keep prompts compact.

Knowledge tiers:

| Tier label | Tier |
|---|---|
| `P` | Persistent — high-confidence long-term knowledge |
| `C` | Consolidated — distilled from multiple episodes |
| `W` | Working — recent session knowledge |
| `T` | Transient — ephemeral, not persisted |

---

## Session Persistence

Sessions are automatically persisted to disk after every `session/prompt`
completes. The storage location is:

```
<workdir>/.roko/sessions/<session_id>.json
```

For example:
```
/path/to/project/.roko/sessions/sess_550e8400-e29b-41d4-a716-446655440000.json
```

Each session file contains the full `AcpSession` state as JSON, including:
- Session ID, name, and creation timestamp
- Config state (model, workflow, gates, etc.)
- Conversation history (up to 40 turns, capped at 64,000 characters)
- Active workflow run state (pipeline phase, iteration count, etc.)

**Session GC:** At startup the server garbage-collects session files older than
7 days.

**Resuming a session:**

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}

{"jsonrpc":"2.0","id":2,"method":"session/list","params":{}}
```

Pick the session ID you want from the `sessions` array, then:

```json
{"jsonrpc":"2.0","id":3,"method":"session/load","params":{"sessionId":"sess_550e8400..."}}
```

The server loads the session from disk into memory and returns its config
options and modes. Continue with `session/prompt` as normal.

---

## Error Handling

All errors follow the JSON-RPC 2.0 error object format:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32601,
    "message": "method 'nope/method' is not supported"
  }
}
```

### Error codes

| Code | Constant | Cause |
|---|---|---|
| `-32700` | `PARSE_ERROR` | Malformed JSON on stdin |
| `-32600` | `INVALID_REQUEST` | Not a valid JSON-RPC request |
| `-32601` | `METHOD_NOT_FOUND` | Unknown method name |
| `-32602` | `INVALID_PARAMS` | Request params failed to deserialize |
| `-32603` | `INTERNAL_ERROR` | Unexpected server-side failure |
| `-32000` | `SESSION_NOT_FOUND` | Session ID not in memory or on disk |
| `-32001` | `SESSION_BUSY` | Session already has an active prompt in flight |

### Parse errors

If the server cannot parse a line of JSON, it responds with a parse error and
continues. The server does not exit on bad input.

```
Client sends: {bad json
Server sends: {"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"failed to parse JSON-RPC message: ..."}}
```

### Session busy

Attempting to send a second `session/prompt` while one is in flight:

```json
{
  "error": {
    "code": -32001,
    "message": "session 'sess_550e8400...' already has an active prompt"
  }
}
```

Cancel the in-flight prompt with `session/cancel`, wait for its response, then
send the next prompt.

### Recovery strategy

- **`PARSE_ERROR`**: Fix the JSON encoding in your client.
- **`SESSION_NOT_FOUND`**: Call `session/new` to create a fresh session, or
  `session/load` if you have a persisted session ID.
- **`SESSION_BUSY`**: Send `session/cancel` and wait for the in-flight response
  before retrying.
- **Internal errors**: Log the full error message. The server continues running;
  retry the request.
- **EOF/process exit**: Restart `roko acp`.

---

## Slash Commands

Slash commands are sent as plain text prompts starting with `/`. The server
handles them internally (they do not dispatch to the LLM) and stream results
back as `agent_message_chunk` notifications.

Format: `/command-name [argument]`

Example prompt block:

```json
{
  "type": "text",
  "text": "/research async runtime design patterns in Rust"
}
```

### Status and diagnostics

| Command | Arguments | Description |
|---|---|---|
| `/status` | — | Workspace status: signals, agents, runs, knowledge |
| `/doctor` | — | Diagnose workspace bootstrap state |
| `/config` | — | Show `roko.toml` configuration |
| `/learn` | — | Learning state: episodes, routing, experiments, efficiency |

### Research (foraging phase)

| Command | Arguments | Description |
|---|---|---|
| `/research` | `<topic>` | Deep research with citations via Perplexity |
| `/search` | `<query>` | Quick web search |
| `/enhance-prd` | `<slug>` | Enrich a PRD with web research |
| `/analyze` | — | Analyze execution data |

### Specification (PRD lifecycle)

| Command | Arguments | Description |
|---|---|---|
| `/prd-idea` | `<text>` | Capture a new work item idea |
| `/prd-draft` | `<slug>` | Draft a new PRD from an idea |
| `/prd-list` | — | List all PRDs and their status |
| `/prd-status` | — | PRD pipeline coverage report |
| `/prd-plan` | `<slug>` | Generate implementation plan from a published PRD |
| `/prd-consolidate` | — | Scan PRDs for gaps and duplicates |

### Planning

| Command | Arguments | Description |
|---|---|---|
| `/plan-list` | — | List all plans in the workspace |
| `/plan-show` | `<name>` | Show a specific plan |
| `/plan-generate` | `<description>` | Generate a plan from a prompt |
| `/plan-validate` | `<path>` | Lint `tasks.toml` without executing |
| `/plan-run` | `<path>` | Execute a plan (orchestrate agents, gates, persistence) |
| `/plan-resume` | `<state-path>` | Resume an interrupted plan run |

### Implementation and execution

| Command | Arguments | Description |
|---|---|---|
| `/run` | `<prompt>` | Single prompt → universal loop (compose→agent→gate→persist) |
| `/agents` | — | List agents and their status |
| `/agent-chat` | `<name>` | Interactive chat REPL with a specific agent |
| `/agent-start` | `<name>` | Start a named agent |
| `/agent-stop` | `<name>` | Stop a running agent |

### Verification and gates

| Command | Arguments | Description |
|---|---|---|
| `/review` | `[ref]` | Review recent changes (`git diff`, default `HEAD~1`) |
| `/build` | — | `cargo build --workspace` |
| `/test` | — | `cargo test --workspace` |
| `/clippy` | — | `cargo clippy --workspace --no-deps -- -D warnings` |
| `/fmt` | — | `cargo +nightly fmt --all --check` |
| `/gate` | — | Run full gate pipeline (compile + test + clippy + diff) |

### Knowledge and dreams

| Command | Arguments | Description |
|---|---|---|
| `/knowledge` | `<topic>` | Query the durable knowledge store |
| `/knowledge-stats` | — | Knowledge store statistics and health |
| `/knowledge-gc` | — | Garbage collect knowledge store |
| `/knowledge-backup` | — | Backup knowledge store |
| `/dream` | — | Run dream consolidation cycle (NREM → REM → integration) |

### Code intelligence

| Command | Arguments | Description |
|---|---|---|
| `/index` | `build \| search <q> \| stats` | Code intelligence index operations |
| `/explain` | `<topic>` | Explain a codebase concept at 3 depth levels |
| `/replay` | `<hash>` | Walk signal DAG by hash (episode replay) |

### Feedback and learning

| Command | Arguments | Description |
|---|---|---|
| `/learn-router` | — | Inspect cascade router state and model routing |
| `/learn-episodes` | — | Recent episode log (agent turns + gate results) |
| `/learn-tune` | `gates \| routing \| budget` | Tune adaptive thresholds |

### Workflow

| Command | Arguments | Description |
|---|---|---|
| `/workflow` | `list \| status \| cancel \| resume` | Workflow management |
| `/express` | `<prompt>` | Run express pipeline: implement → gate → commit |
| `/full` | `<prompt>` | Run full pipeline: strategy → implement → gate → multi-review → commit |
| `/review-this` | — | Run review pipeline on current uncommitted changes |
| `/pipeline` | `<name>` | Run a named workflow pipeline |

### Help

| Command | Arguments | Description |
|---|---|---|
| `/help` | — | List all available commands |

---

## Integration Examples

### Example 1: Minimal session with a single prompt

This is the minimum required for any editor plugin.

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,...},"agentInfo":{"name":"roko","version":"0.4.0","title":"Roko"}}}

→ {"jsonrpc":"2.0","id":2,"method":"session/new","params":{"sessionName":"my-project","mcpServers":[]}}
← {"jsonrpc":"2.0","id":2,"result":{"sessionId":"sess_abc123","modes":{...},"configOptions":[...]}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"available_commands_update","availableCommands":[...]}}}

→ {"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"sess_abc123","prompt":[{"type":"text","text":"What does the main function do?"}],"includeContext":false}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"The main function "}}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"initializes the runtime..."}}}}
← {"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}
```

### Example 2: Prompt with file context

```
→ {"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{
    "sessionId":"sess_abc123",
    "prompt":[
      {"type":"text","text":"Add error handling to this function:"},
      {"type":"resource","resource":{"type":"file","uri":"file:///path/to/project/src/auth.rs"}}
    ],
    "includeContext":false
  }}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"tool_call","toolCallId":"knowledge","title":"Prior knowledge - 2 results","kind":"other","status":"completed","content":[{"type":"text","text":"**Knowledge:**\n  - [P] 0.88 - Always handle auth errors explicitly..."}]}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"I'll add error handling..."}}}}
← {"jsonrpc":"2.0","id":4,"result":{"stopReason":"end_turn"}}
```

### Example 3: Express workflow pipeline

```
→ {"jsonrpc":"2.0","id":5,"method":"session/config/update","params":{"sessionId":"sess_abc123","optionId":"workflow","newValue":"express"}}
← {"jsonrpc":"2.0","id":5,"result":{"configOptions":[...]}}

→ {"jsonrpc":"2.0","id":6,"method":"session/prompt","params":{"sessionId":"sess_abc123","prompt":[{"type":"text","text":"Fix the off-by-one error in src/parser.rs line 42"}],"includeContext":false}}

← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"tool_call","toolCallId":"agent-impl-abc","title":"Agent: implementer","kind":"other","status":"pending","content":[]}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"[Phase: Pending -> Implementing]\n"}}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Fixing the off-by-one..."}}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"tool_call_update","toolCallId":"agent-impl-abc","status":"completed","content":[{"type":"text","text":"Changed `i < n` to `i <= n`"}]}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"tool_call","toolCallId":"gate-compile","title":"Gate: compile","kind":"other","status":"in_progress","content":[]}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"tool_call_update","toolCallId":"gate-compile","status":"completed","content":[{"type":"text","text":"compile passed"}]}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"tool_call","toolCallId":"gate-test","title":"Gate: test","kind":"other","status":"in_progress","content":[]}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"tool_call_update","toolCallId":"gate-test","status":"completed","content":[{"type":"text","text":"test passed"}]}}}
← {"jsonrpc":"2.0","id":6,"result":{"stopReason":"end_turn"}}
```

### Example 4: Cancellation

```
→ {"jsonrpc":"2.0","id":7,"method":"session/prompt","params":{"sessionId":"sess_abc123","prompt":[{"type":"text","text":"Refactor the entire auth system"}],"includeContext":false}}

← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Starting refactor..."}}}}

→ {"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"sess_abc123"}}

← {"jsonrpc":"2.0","id":7,"result":{"stopReason":"cancelled"}}
```

### Example 5: Resume a persisted session

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}
← {"jsonrpc":"2.0","id":1,"result":{...}}

→ {"jsonrpc":"2.0","id":2,"method":"session/list","params":{}}
← {"jsonrpc":"2.0","id":2,"result":{"sessions":[{"sessionId":"sess_abc123","sessionName":"my-project","createdAt":"2026-04-29T10:30:00Z"}]}}

→ {"jsonrpc":"2.0","id":3,"method":"session/load","params":{"sessionId":"sess_abc123"}}
← {"jsonrpc":"2.0","id":3,"result":{"sessionId":"sess_abc123","modes":{...},"configOptions":[...]}}

→ {"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":"sess_abc123","prompt":[{"type":"text","text":"Continue where we left off"}],"includeContext":false}}
```

### Example 6: Using a slash command

```
→ {"jsonrpc":"2.0","id":8,"method":"session/prompt","params":{
    "sessionId":"sess_abc123",
    "prompt":[{"type":"text","text":"/status"}],
    "includeContext":false
  }}
← {"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_abc123","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Signals: 142\nAgents: 0 running\nKnowledge: 38 entries\nEpisodes: 71\n"}}}}
← {"jsonrpc":"2.0","id":8,"result":{"stopReason":"end_turn"}}
```

### Example 7: Config update and mode switch

```
# Switch to plan mode
→ {"jsonrpc":"2.0","id":9,"method":"session/set_mode","params":{"sessionId":"sess_abc123","modeId":"plan"}}
← {"jsonrpc":"2.0","id":9,"result":{"configOptions":[...]}}

# Update model selection
→ {"jsonrpc":"2.0","id":10,"method":"session/config/update","params":{"sessionId":"sess_abc123","optionId":"model","newValue":"opus"}}
← {"jsonrpc":"2.0","id":10,"result":{"configOptions":[{"id":"model","name":"Model","currentValue":"opus",...}]}}

# Disable clippy gate
→ {"jsonrpc":"2.0","id":11,"method":"session/config/update","params":{"sessionId":"sess_abc123","optionId":"clippy","newValue":"off"}}
← {"jsonrpc":"2.0","id":11,"result":{"configOptions":[...]}}

# Enable max effort
→ {"jsonrpc":"2.0","id":12,"method":"session/config/update","params":{"sessionId":"sess_abc123","optionId":"effort","newValue":"max"}}
← {"jsonrpc":"2.0","id":12,"result":{"configOptions":[...]}}
```

### Example 8: MCP server attachment

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "session/new",
  "params": {
    "sessionName": "with-mcp",
    "mcpServers": [
      {
        "name": "roko-code",
        "transport": {
          "type": "stdio",
          "command": "roko",
          "args": ["mcp-code", "--workdir", "/path/to/project"]
        }
      },
      {
        "name": "github",
        "transport": {
          "type": "http",
          "url": "http://localhost:9001"
        }
      }
    ]
  }
}
```

---

## Log File

ACP server logs go to `.roko/acp.log` by default (override with `--log-file`).
The log file is created at startup. All `tracing` output at `debug` level and
above for `roko_acp` is written there.

```bash
# Tail the log while debugging
tail -f /path/to/project/.roko/acp.log
```

Log entries are plain text with timestamps, levels, and structured fields.

---

## Episode Recording

Every `session/prompt` invocation (excluding slash commands) is recorded as an
episode in:

```
<workdir>/.roko/episodes.jsonl
```

Each episode captures:
- Session ID and model used
- Provider kind (claude_cli, openai_compat, etc.)
- Whether a pipeline was active and which template
- Routing mode in effect
- Input and output content hashes (for deduplication)
- Wall-clock duration
- Whether the turn succeeded
- Failure reason if unsuccessful

This feed is consumed by `roko learn` for adaptive routing and gate threshold
tuning.

---

## Quick Reference

### Startup sequence

```
1. Spawn: roko acp --workdir <project-root>
2. Send:  initialize
3. Recv:  initialize result
4. Send:  session/new (or session/load for resume)
5. Recv:  session/new result
6. Recv:  session/update (available_commands_update)
7. Ready for session/prompt calls
```

### Per-turn sequence

```
1. Send:  session/prompt
2. Recv:  session/update (knowledge card, if results found)
3. Recv:  session/update (agent_message_chunk, ..., tool_call, tool_call_update, plan, ...)
4. Recv:  session/prompt result (stopReason)
5. [Optional] session is auto-persisted to .roko/sessions/<id>.json
```

### Key invariants

- One JSON object per line, `\n` terminated.
- stdout is the protocol channel. Never write non-JSON to stdout.
- `initialize` before anything else.
- `session/cancel` is a notification (no response). The corresponding
  `session/prompt` response still arrives after cancellation.
- Sessions are single-threaded: one prompt at a time per session.
- Config options come from `roko.toml`. The `model` dropdown reflects only
  models defined in `[models.*]`.
