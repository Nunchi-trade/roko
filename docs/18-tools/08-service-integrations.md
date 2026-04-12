# 08 — Service Integrations

> MetaMask, Venice, Bankr, AgentCash, Uniswap, Slack, GitHub, Linear — integration
> architecture and chain domain plugin service dependencies.

---

## Overview

Service integrations connect Roko agents to external platforms. These integrations fall into
two categories:

1. **Chain domain plugin services** — DeFi protocols, wallet providers, and chain
   infrastructure used by chain agents (MetaMask, Uniswap Trading API, etc.)
2. **Operations platform adapters** — Collaboration and productivity tools used by operations
   agents (Slack, GitHub, Linear)

Chain domain services are documented here for completeness. Operations platform adapters are
also accessible via MCP servers (see `09-mcp-architecture.md` and `10-mcp-github.md`,
`11-mcp-slack.md`).

---

## Integration Architecture

All service integrations follow a three-layer architecture:

```
Layer 1: Event Reception
    ├── Webhook endpoints (GitHub, Slack)
    ├── Polling adapters (Linear, chain events)
    └── WebSocket streams (chain data, Slack Socket Mode)

Layer 2: Agent Execution
    ├── Event → Engram conversion
    ├── Template matching (which agent handles this?)
    └── Agent spawn with ToolContext

Layer 3: MCP Tool Adapters
    ├── github.* tools (via roko-mcp-github)
    ├── slack.* tools (via roko-mcp-slack)
    └── scripts.* tools (via roko-mcp-scripts)
```

Events arrive at Layer 1, are converted to Engrams (the universal data type), matched to
agent templates via subscription configuration, and the agent executes using tools from
Layer 3.

---

## Chain Domain Plugin Services

### MetaMask Integration

MetaMask integration enables user-supervised wallet sessions. The agent can request
transaction signing through MetaMask's Snap interface.

| Aspect | Details |
|---|---|
| **Integration type** | Wallet provider (Delegation custody mode) |
| **Protocol** | MetaMask Snap RPC |
| **Authentication** | User approval per transaction |
| **Use case** | Human-in-the-loop trading sessions |

The MetaMask Snap provides a browser extension interface where the user can approve or reject
each transaction the agent proposes. This is the most conservative custody mode — the agent
proposes, the human approves.

### Uniswap Trading API

The Uniswap Trading API provides Smart Order Routing and UniswapX integration:

| Aspect | Details |
|---|---|
| **Integration type** | Chain domain tool dependency |
| **Protocol** | REST API (HTTPS) |
| **Authentication** | API key (three tiers: read, feedback, write) |
| **Tools using this** | `uniswap_get_quote`, `uniswap_execute_swap`, `uniswap_submit_uniswapx_order` |

Three-tier API key model:
- **Read**: Quote fetching, pool data, route computation
- **Feedback**: Read + execution quality reporting (improves routing)
- **Write**: Read + Feedback + order submission (UniswapX, Smart Order Router)

### Venice (Privacy-Preserving AI)

Venice provides privacy-preserving LLM inference for sensitive chain operations:

| Aspect | Details |
|---|---|
| **Integration type** | LLM provider |
| **Protocol** | OpenAI-compatible API |
| **Authentication** | API key |
| **Use case** | Strategy analysis that shouldn't be visible to centralized providers |

Useful when chain agents need to reason about proprietary trading strategies without exposing
the full context to cloud AI providers.

### Bankr (Agent Banking)

Bankr provides fiat on/off-ramp for agent operations:

| Aspect | Details |
|---|---|
| **Integration type** | Financial service |
| **Protocol** | REST API |
| **Use case** | Agent self-funding, fiat settlements |

### AgentCash (x402 Micropayments)

AgentCash implements the x402 micropayment protocol (Coinbase/Linux Foundation) for
machine-to-machine payments:

| Aspect | Details |
|---|---|
| **Integration type** | Payment protocol |
| **Protocol** | x402 HTTP header-based payments |
| **Use case** | Agent self-funding, API access payments, knowledge marketplace |

This supports the x402 Micropayments innovation (see `docs/09-innovations/`) — agents can
pay for services and receive payment for knowledge production without human intervention.

---

## Operations Platform Adapters

### Slack Integration

Slack integration provides bidirectional communication between agents and team channels.

| Aspect | Details |
|---|---|
| **Integration type** | Operations platform adapter |
| **Protocols** | Socket Mode (preferred), HTTP webhook (fallback) |
| **Authentication** | Bot token + signing secret |
| **MCP server** | `roko-mcp-slack` (8 tools) |
| **Agent templates using this** | slack-notify-agent, pm-health-agent, action-tracker-agent, sync-agent |

**Socket Mode** (preferred): WebSocket connection, no public endpoint needed. Suitable for
self-hosted deployments.

**HTTP Mode** (fallback): Webhook endpoint at `/hooks/slack`. Requires public URL. Used when
Socket Mode is not available.

Supported Slack capabilities:
- Incoming: message events, slash commands, interactive components
- Outgoing: post messages, update messages, thread replies, Block Kit formatting
- Rate limits: 1 message/second per channel (Tier 3)

See `11-mcp-slack.md` for the full tool specification.

### GitHub Integration

GitHub integration enables PR review, issue management, and repository operations.

| Aspect | Details |
|---|---|
| **Integration type** | Operations platform adapter |
| **Protocol** | REST API v3 + GraphQL v4 + Webhooks |
| **Authentication** | GitHub App (installation token) or PAT |
| **MCP server** | `roko-mcp-github` (17 tools) |
| **Agent templates using this** | pr-review-agent, triage-agent, auto-plan-agent, code-implementer-agent, gate-fixer-agent |

Webhook events consumed:
- `push` — file changes trigger doc-lifecycle, enrichment, auto-planning agents
- `pull_request` — PR open/update triggers review, triage agents
- `pull_request_review` — review submission triggers review-response agent
- `issues` — issue open triggers triage agent

See `10-mcp-github.md` for the full tool specification.

### Linear Integration

Linear integration enables project management and issue tracking:

| Aspect | Details |
|---|---|
| **Integration type** | Operations platform adapter |
| **Protocol** | GraphQL API + Webhooks |
| **Authentication** | API key |
| **Use case** | Syncing PM state, creating/updating issues, sprint management |

---

## Bounty Program

The integration bounty program incentivizes community contributions for new service adapters:

| Integration | Bounty | Priority | Status |
|---|---|---|---|
| Slack (Socket Mode + HTTP) | $3,000 | P1 | Planned (roko-mcp-slack) |
| GitHub (full API + webhooks) | $3,000 | P1 | Planned (roko-mcp-github) |
| Linear (GraphQL + webhooks) | $2,500 | P2 | Planned |
| Discord | $2,000 | P3 | Not started |
| Notion | $2,000 | P3 | Not started |
| Jira | $2,500 | P3 | Not started |
| Telegram | $1,500 | P3 | Not started |
| Custom webhook (generic) | $1,500 | P2 | Planned (roko-mcp-scripts) |
| Twitter/X (API v2) | $2,000 | P3 | Not started |
| Email (SMTP/IMAP) | $1,500 | P3 | Not started |
| Stripe (payments) | $2,000 | P3 | Not started |
| **Total** | **$23,500** | | |

Each bounty requires:
1. MCP server implementation following the `roko-mcp-*` crate pattern
2. Full tool schema with JSON Schema validation
3. Unit tests + integration tests with mock server
4. Documentation following the sub-doc format

---

## Service Platform Architecture

Service integrations share a common command pattern for the event reception layer:

```rust
/// Events from platform webhooks are converted to platform-specific commands.
pub enum ServicePlatformCommand {
    // GitHub events
    GitHubPush { repo: String, ref_: String, commits: Vec<CommitInfo> },
    GitHubPullRequest { repo: String, number: u64, action: String },
    GitHubIssue { repo: String, number: u64, action: String },
    GitHubReview { repo: String, pr_number: u64, action: String },

    // Slack events
    SlackMessage { channel: String, user: String, text: String, thread_ts: Option<String> },
    SlackSlashCommand { channel: String, command: String, text: String },

    // Generic
    WebhookPayload { source: String, payload: serde_json::Value },
    CronTick { schedule: String, template: String },
    FileChanged { paths: Vec<PathBuf>, watch_root: PathBuf },
}
```

Each `ServicePlatformCommand` is converted to an Engram with appropriate `Kind` and `Body`,
then matched against subscription patterns to determine which agent template should handle it.

---

## Structural vs. Decorative Classification

Integrations are classified by their impact on the agent's cognitive capabilities:

**Structural integrations** change what the agent CAN do:
- Chain providers (enable on-chain interaction)
- Wallet providers (enable transaction signing)
- MCP servers (add new tools)
- LLM providers (enable inference)

**Decorative integrations** change how the agent PRESENTS its work:
- Slack notifications (report results)
- TUI rendering (display progress)
- Telemetry export (emit metrics)

Structural integrations are required for the agent to function in its domain. Decorative
integrations enhance observability and collaboration but the agent works without them. This
classification helps prioritize which integrations to implement first.
