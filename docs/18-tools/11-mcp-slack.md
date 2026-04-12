# 11 — roko-mcp-slack

> 8 Slack API tools: messaging, channels, threads, reactions, files, user lookup.
> Socket Mode and HTTP Mode operation.


> **Implementation**: Scaffold

---

## Overview

`roko-mcp-slack` is an MCP server that exposes 8 Slack API tools via the Model Context
Protocol. It enables agents to send messages, manage threads, post files, and interact with
Slack channels for notifications and collaboration.

**Status:** Planned (spec complete, implementation pending)

**Crate:** `crates/roko-mcp-slack/`

**Protocol:** MCP (JSON-RPC 2.0 over stdio)

**Authentication:** Slack Bot Token + Signing Secret

**Agent templates using this:** slack-notify-agent, pm-health-agent, action-tracker-agent,
sync-agent, digest-agent

---

## Connection Modes

### Socket Mode (Preferred)

WebSocket-based connection. No public endpoint needed. The MCP server connects to Slack's
WebSocket endpoint and receives events directly.

**Advantages:**
- No public URL required (works behind NAT/firewall)
- Real-time event delivery
- Automatic reconnection
- Suitable for self-hosted deployments

**Configuration:**

```toml
[[agent.mcp_servers]]
name = "slack"
command = "roko-mcp-slack"
args = ["--mode", "socket"]
env = {
    SLACK_BOT_TOKEN = "${SLACK_BOT_TOKEN}",
    SLACK_APP_TOKEN = "${SLACK_APP_TOKEN}",
}
```

### HTTP Mode (Fallback)

Webhook-based connection. Requires a publicly accessible URL for Slack to POST events to.

**Configuration:**

```toml
[[agent.mcp_servers]]
name = "slack"
command = "roko-mcp-slack"
args = ["--mode", "http", "--port", "3001"]
env = {
    SLACK_BOT_TOKEN = "${SLACK_BOT_TOKEN}",
    SLACK_SIGNING_SECRET = "${SLACK_SIGNING_SECRET}",
}
```

---

## The 8 Tools

### 1. `slack.post_message`

Post a message to a Slack channel.

```json
{
  "name": "post_message",
  "description": "Post a message to a Slack channel. Supports Block Kit formatting.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "channel": {
        "type": "string",
        "description": "Channel name (e.g., '#roko-activity') or channel ID"
      },
      "text": {
        "type": "string",
        "description": "Message text (used as fallback for Block Kit)"
      },
      "blocks": {
        "type": "array",
        "description": "Block Kit blocks for rich formatting (optional)",
        "items": { "type": "object" }
      },
      "thread_ts": {
        "type": "string",
        "description": "Thread timestamp to reply in a thread (optional)"
      }
    },
    "required": ["channel", "text"]
  }
}
```

### 2. `slack.update_message`

Update an existing message.

```json
{
  "name": "update_message",
  "description": "Update a previously sent message. Requires the message timestamp.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "channel": { "type": "string" },
      "ts": { "type": "string", "description": "Message timestamp (from post_message response)" },
      "text": { "type": "string" },
      "blocks": { "type": "array", "items": { "type": "object" } }
    },
    "required": ["channel", "ts", "text"]
  }
}
```

### 3. `slack.reply_thread`

Reply in a specific thread.

```json
{
  "name": "reply_thread",
  "description": "Reply in a thread. Use the parent message's ts as thread_ts.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "channel": { "type": "string" },
      "thread_ts": { "type": "string", "description": "Parent message timestamp" },
      "text": { "type": "string" },
      "blocks": { "type": "array", "items": { "type": "object" } }
    },
    "required": ["channel", "thread_ts", "text"]
  }
}
```

### 4. `slack.add_reaction`

Add an emoji reaction to a message.

```json
{
  "name": "add_reaction",
  "description": "Add an emoji reaction to a message.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "channel": { "type": "string" },
      "ts": { "type": "string", "description": "Message timestamp" },
      "emoji": { "type": "string", "description": "Emoji name without colons (e.g., 'thumbsup')" }
    },
    "required": ["channel", "ts", "emoji"]
  }
}
```

### 5. `slack.upload_file`

Upload a file to a channel.

```json
{
  "name": "upload_file",
  "description": "Upload a file to a Slack channel.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "channel": { "type": "string" },
      "content": { "type": "string", "description": "File content (text)" },
      "filename": { "type": "string" },
      "filetype": { "type": "string", "description": "File type (e.g., 'markdown', 'json', 'text')" },
      "title": { "type": "string" },
      "initial_comment": { "type": "string" }
    },
    "required": ["channel", "content", "filename"]
  }
}
```

### 6. `slack.get_channel_history`

Get recent messages from a channel.

```json
{
  "name": "get_channel_history",
  "description": "Get recent messages from a channel. Returns up to 'limit' messages.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "channel": { "type": "string" },
      "limit": { "type": "integer", "default": 20, "maximum": 100 },
      "oldest": { "type": "string", "description": "Unix timestamp — only messages after this time" }
    },
    "required": ["channel"]
  }
}
```

### 7. `slack.get_thread`

Get all replies in a thread.

```json
{
  "name": "get_thread",
  "description": "Get all replies in a thread.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "channel": { "type": "string" },
      "thread_ts": { "type": "string", "description": "Parent message timestamp" },
      "limit": { "type": "integer", "default": 50 }
    },
    "required": ["channel", "thread_ts"]
  }
}
```

### 8. `slack.lookup_user`

Look up a Slack user by email or display name.

```json
{
  "name": "lookup_user",
  "description": "Look up a Slack user by email or display name.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "email": { "type": "string", "description": "User's email address" },
      "name": { "type": "string", "description": "User's display name (fuzzy match)" }
    }
  }
}
```

---

## Rate Limiting

Slack API rate limits are handled transparently by the MCP server:

| API Method | Rate Limit | Tier |
|---|---|---|
| `chat.postMessage` | 1 per second per channel | Tier 3 |
| `chat.update` | 1 per second per channel | Tier 3 |
| `files.upload` | 20 per minute | Tier 2 |
| `conversations.history` | 50 per minute | Tier 3 |
| `conversations.replies` | 50 per minute | Tier 3 |
| `users.lookupByEmail` | 50 per minute | Tier 3 |
| `reactions.add` | 50 per minute | Tier 3 |

The server implements a token bucket rate limiter per endpoint. When rate-limited, it queues
the request and retries with exponential backoff.

---

## Slack Bot Permissions (Scopes)

Required bot token scopes:

| Scope | Used By |
|---|---|
| `chat:write` | post_message, update_message, reply_thread |
| `files:write` | upload_file |
| `reactions:write` | add_reaction |
| `channels:history` | get_channel_history, get_thread |
| `channels:read` | Channel ID resolution |
| `users:read` | lookup_user |
| `users:read.email` | lookup_user (by email) |

---

## Block Kit Integration

The `post_message`, `update_message`, and `reply_thread` tools support Slack's Block Kit for
rich message formatting. When `blocks` is provided, `text` serves as the fallback for
notifications and accessibility.

Example Block Kit usage by the slack-notify-agent:

```json
{
  "channel": "#roko-alerts",
  "text": "Gate failure for code-implementer-agent",
  "blocks": [
    {
      "type": "header",
      "text": { "type": "plain_text", "text": "Gate Failure" }
    },
    {
      "type": "section",
      "fields": [
        { "type": "mrkdwn", "text": "*Agent:* code-implementer-agent" },
        { "type": "mrkdwn", "text": "*Gate:* CompileGate" },
        { "type": "mrkdwn", "text": "*Error:* `error[E0308]: mismatched types`" }
      ]
    },
    {
      "type": "actions",
      "elements": [
        {
          "type": "button",
          "text": { "type": "plain_text", "text": "View PR" },
          "url": "https://github.com/nunchi/roko/pull/42"
        }
      ]
    }
  ]
}
```

---

## Agents & Assistants API Integration

Slack's Agents & Assistants API (available since 2024) provides a framework for building
conversational agents within Slack. Future versions of `roko-mcp-slack` may support this API
for interactive agent sessions directly in Slack threads — enabling users to converse with
Roko agents without leaving Slack.

Current scope: one-way notifications and channel operations. Interactive sessions are a future
enhancement.
