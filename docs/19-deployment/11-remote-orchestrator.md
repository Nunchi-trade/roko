# Remote Orchestrator

> Roko can run as a long-lived HTTP service, letting you interact with it from anywhere —
> submit plans, trigger runs, query durable state, and subscribe to the shared realtime surface.
> This document covers the server mode, the HTTP API surface, authentication, remote-consumer
> patterns, webhook integration, the local-to-remote transition, and the `roko-serve` crate.


> **Implementation**: Specified

---

## Overview

The remote orchestrator transforms Roko from a local CLI tool into a deployed service. The
same orchestration engine (DAG executor, agent dispatch, gate pipeline, Engram persistence,
and Bus-backed live progress) runs behind an HTTP API instead of a terminal interface.

Use cases:
- **Team usage**: Deploy one Roko instance, give team members API keys, everyone can view
  progress, trigger runs, and upload PRDs from their own machines
- **CI integration**: Trigger plan runs from GitHub Actions, Jenkins, or any CI system
- **Autonomous operation**: The remote orchestrator runs continuously, processing webhooks
  and scheduled subscriptions without human intervention
- **Mobile/tablet access**: Interact with Roko from any device with a browser

REF27 tightens the remote-consumer story: the deployed service should expose one realtime
surface over WebSocket, SSE, and optional gRPC streaming, with the same channel names, the same
cursor/resume rules, and the same auth model across transports. See
[../12-interfaces/06-websocket-streaming.md](../12-interfaces/06-websocket-streaming.md) and
[../../tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md).

---

## Starting the Server

### roko-serve (Dedicated HTTP Server)

```bash
# Start the HTTP API server
roko-serve --port 8080 --bind 0.0.0.0

# With configuration
roko-serve --port 8080 --data-dir /data --config ~/.config/roko/config.toml
```

`roko-serve` is a dedicated binary built from the `crates/roko-serve/` crate. It exposes the
full Roko API over HTTP without the TUI. This is the preferred deployment for servers.

### roko-cli Server Mode

```bash
# Start the CLI in server mode (API + optional TUI)
roko plan run --server --port 8080 --bind 0.0.0.0
```

The CLI can also expose an HTTP API alongside its TUI. This is useful for development — you
see the TUI locally while the API is accessible remotely.

---

## HTTP API

### REST Endpoints

```
GET    /health                        # Server health check
GET    /v1/status                     # Server status (uptime, active runs, agent count)

# Project management
GET    /v1/projects                   # List managed projects
POST   /v1/projects                   # Create a new project (clone repo)
GET    /v1/projects/:id               # Project details
DELETE /v1/projects/:id               # Remove a project

# Plan execution
GET    /v1/projects/:id/plans         # List plans in a project
POST   /v1/projects/:id/run           # Start a plan run
GET    /v1/projects/:id/runs          # List runs (history)
GET    /v1/projects/:id/runs/:run_id  # Run status and details
DELETE /v1/projects/:id/runs/:run_id  # Cancel a running plan

# PRD management
GET    /v1/projects/:id/prds          # List PRDs
POST   /v1/projects/:id/prds          # Upload a PRD
PUT    /v1/projects/:id/prds/:name    # Update a PRD (triggers re-enrichment)
POST   /v1/projects/:id/prds/:name/promote  # Promote draft to published

# Artifacts
GET    /v1/projects/:id/artifacts/:plan  # Download artifacts for a plan

# Engrams and episodes
GET    /v1/projects/:id/engrams       # Query durable Engrams
GET    /v1/projects/:id/episodes      # Query episode log
```

### Real-Time Streaming

```
GET    /projections/:name             # Query current projection state
GET    /projections/:name/stream      # SSE or WebSocket stream for one projection
WS     /ws/*                          # Bidirectional binding for subscribe + publish
gRPC   RealtimeSurface.Subscribe      # Optional typed binding for the same contract
```

The deployment contract is projection-first:

- remote clients `query` the current projection state over HTTP
- remote clients `subscribe` to a `channel` over WebSocket, SSE, or optional gRPC
- every outbound frame carries a cursor
- reconnecting clients resume from that cursor when retained history still exists
- bidirectional transports may `publish` user-originated Pulses back into the Bus

Typical remote consumers include:

- browser pages using `projection:active_tasks` and `projection:agent_trails`
- Slack or chat bots using filtered `topic:*` subscriptions
- audit or replication sinks using `engram-stream:*`
- another Roko instance following the same channel registry for cross-instance sync

Deployment guidance:

- disable proxy buffering for `SSE`
- preserve upgrade headers for `WebSocket`
- keep cursor retention in shared storage for clustered nodes
- authorize each requested `channel` and filter rather than relying on connection-level auth alone

### Provider and Model Management

From the model routing implementation plan:

```
GET    /v1/providers                  # List configured providers and health
GET    /v1/providers/:name/health     # Detailed provider health
POST   /v1/providers/:name/test       # Test provider connectivity

GET    /v1/models                     # List available models across providers
GET    /v1/models/route               # Explain routing decision for a hypothetical request

GET    /v1/routing/decisions           # Recent routing decisions (audit log)
GET    /v1/routing/stats              # Routing statistics (provider usage, latency, costs)
```

---

## Authentication

### API Keys

```bash
roko auth create-key --scope admin --label "my-laptop"
# → roko_sk_a1b2c3d4e5f6...
```

API keys are bearer tokens. Three scopes:

| Scope | Capabilities |
|---|---|
| `read` | View projects, plans, runs, artifacts, and realtime subscriptions |
| `write` | Create projects, start runs, upload PRDs, and publish user-originated Pulses |
| `admin` | Manage API keys, server config, delete projects |

Keys are stored in the server's state directory (`.roko/auth.db` — SQLite). They are
rotatable, revocable, and auditable.

User-scoped deployments may also layer OIDC bearer tokens or browser session cookies on top of
the same policy engine. The important deployment rule is that authorization happens per
subscription request: a client that authenticated successfully may still be denied access to a
specific `channel` or filter.

### Authentication Header

```bash
# Bearer token in Authorization header
curl -H "Authorization: Bearer roko_sk_a1b2c3d4..." \
  https://roko-serve.fly.dev/v1/projects

# Or x-api-key header (convenience)
curl -H "x-api-key: roko_sk_a1b2c3d4..." \
  https://roko-serve.fly.dev/v1/projects
```

### Rate Limiting

The server enforces rate limits per API key:

| Scope | Limit |
|---|---|
| `read` | 1000 requests/minute |
| `write` | 100 requests/minute |
| `admin` | 50 requests/minute |

Rate limits are enforced per-key using a token bucket algorithm. The `x-ratelimit-remaining`
and `x-ratelimit-reset` headers are included in responses.

---

## Multi-Project Management

One roko-serve instance can manage multiple projects. Each project has its own:

- Git repository (cloned to the server's persistent storage)
- PRDs, plans, and context artifacts
- Provider configuration and API keys (inherits from server defaults, overridable per-project)
- Run history and artifacts
- Engram and episode logs

Projects are isolated. A run in project A does not affect project B.

### Creating a Project

```bash
curl -X POST https://roko-serve.fly.dev/v1/projects \
  -H "Authorization: Bearer roko_sk_..." \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-app",
    "repo_url": "https://github.com/user/my-app.git",
    "branch": "main"
  }'

# Response:
# {
#   "id": "proj_abc123",
#   "name": "my-app",
#   "status": "cloning",
#   "created_at": "2026-04-12T10:00:00Z"
# }
```

### Starting a Run

```bash
curl -X POST https://roko-serve.fly.dev/v1/projects/proj_abc123/run \
  -H "Authorization: Bearer roko_sk_..." \
  -H "Content-Type: application/json" \
  -d '{
    "plan_dir": "plans/",
    "max_agents": 4,
    "resume": false
  }'

# Response:
# {
#   "run_id": "run_def456",
#   "status": "started",
#   "tasks": 5,
#   "agents": 4
# }
```

### Streaming Activity

```bash
curl -N https://roko-serve.fly.dev/projections/active_tasks/stream?filter=project:proj_abc123 \
  -H "Authorization: Bearer roko_sk_..."

# initial state plus later deltas:
# data: {"projection":"active_tasks","cursor":"0x10","kind":"state","payload":{"tasks":[]}}
# data: {"projection":"active_tasks","cursor":"0x11","kind":"delta","payload":{"task":"01-setup","status":"running","agent":"a1"}}
# data: {"projection":"active_tasks","cursor":"0x12","kind":"delta","payload":{"task":"01-setup","status":"success","duration_ms":12345}}
```

---

## Cloud Execution Configuration

The remote orchestrator uses `CloudExecutionConfig` to manage its deployment-specific settings:

```rust
/// Configuration for cloud-deployed Roko instances.
#[derive(Debug, Clone, Deserialize)]
pub struct CloudExecutionConfig {
    /// Server bind address
    pub bind: String,
    /// Server port
    pub port: u16,
    /// Data directory for project storage
    pub data_dir: PathBuf,
    /// Maximum concurrent plan runs across all projects
    pub max_concurrent_runs: usize,
    /// Maximum total agent processes
    pub max_total_agents: usize,
    /// API key database path
    pub auth_db: PathBuf,
    /// Enable webhook ingress
    pub webhooks_enabled: bool,
    /// Webhook listen port (separate from API port)
    pub webhook_port: Option<u16>,
    /// TLS configuration (if not behind a reverse proxy)
    pub tls: Option<TlsConfig>,
}
```

---

## Webhook Integration

The remote orchestrator can receive webhooks from GitHub, GitLab, or any HTTP source:

```toml
# roko.toml (server config)
[webhooks.github]
events = ["push", "pull_request.opened", "issue_comment.created"]
secret = "${GITHUB_WEBHOOK_SECRET}"
```

### GitHub Webhook Flow

1. **Push webhook**: When code is pushed to a monitored branch, the server pulls the latest
   changes and triggers a plan run for affected plans.

2. **Pull request opened**: The server can auto-run plans against the PR branch, posting
   results as PR comments.

3. **Issue comment `/roko run 03-05`**: When someone comments with a `/roko` command on an
   issue, the server parses the command and triggers the specified plans. Results stream back
   as issue comments.

```rust
async fn handle_github_webhook(
    headers: HeaderMap,
    body: Bytes,
    State(state): State<Arc<ServerState>>,
) -> StatusCode {
    // Verify HMAC-SHA256 signature
    let signature = headers.get("x-hub-signature-256");
    if !verify_github_signature(&state.webhook_secret, &body, signature) {
        return StatusCode::UNAUTHORIZED;
    }

    // Parse the webhook kind
    let hook_kind = headers.get("x-github-event")
        .and_then(|v| v.to_str().ok());

    match hook_kind {
        Some("push") => handle_push_hook(&body, &state).await,
        Some("pull_request") => handle_pr_hook(&body, &state).await,
        Some("issue_comment") => handle_comment_hook(&body, &state).await,
        _ => StatusCode::OK, // Acknowledge but ignore other webhook kinds
    }
}
```

---

## The Local-to-Remote Story

The workflow for transitioning from local development to remote operation:

```
1. Start local:     roko init → write PRDs → roko plan run
                    (same tool, same config, same pipeline)

2. Deploy server:   roko-serve deployed to Fly.io
                    (same engine, HTTP interface instead of TUI)

3. Push projects:   curl -X POST .../projects -d '{"repo_url": "..."}'
                    (server clones the repo, loads .roko/ config)

4. Remote runs:     curl -X POST .../projects/:id/run
                    (same DAG executor, same gates, same Engrams and Pulses)

5. Watch remotely:  GET /projections/active_tasks + subscribe
                    (same live progress state rendered by TUI, Web, or a bot)
```

The critical property: **same tool, same config, same artifacts**. The only difference is where
the binary runs. A project's `.roko/` directory, plan files, and config work identically
whether executed by the local CLI or the remote server.

---

## Cost Tracking

The remote orchestrator tracks LLM costs per project, per run, and per API key:

```
GET /v1/projects/:id/costs           # Cost breakdown for a project
GET /v1/projects/:id/runs/:id/costs  # Cost for a specific run
GET /v1/costs                        # Aggregate costs across all projects
```

Cost data is persisted in `.roko/learn/costs.jsonl` per project and aggregated in the server's
state directory. The cost tracker records:

- Model used (and provider)
- Input tokens, output tokens
- Cost per request (using provider pricing tables)
- Cumulative cost per task, per plan, per run, per project

### Budget Limits

The server can enforce budget limits per project or per API key:

```toml
# roko.toml (server config)
[budgets]
# Maximum spend per run
max_per_run_usd = 5.00
# Maximum spend per project per day
max_per_project_daily_usd = 50.00
# Maximum total spend per day
max_daily_usd = 200.00
```

When a budget limit is reached, the server pauses the run and notifies subscribers on the
realtime surface.
The run can be resumed after the budget is increased or the daily reset occurs.

---

## Port Allocation

The remote orchestrator and related services use these ports:

| Port | Service | Protocol | Notes |
|---|---|---|---|
| 8080 | roko-serve HTTP API | HTTPS | Primary API endpoint |
| 8443 | roko-serve WebSocket | WSS | Bidirectional realtime stream |
| 8545 | mirage-rs JSON-RPC | HTTPS | Anvil-compatible EVM RPC |
| 3000 | roko-console | HTTPS | Web terminal UI |
| 7681 | ttyd (per service) | WSS | Internal, proxied through console |
| 9090 | Webhook ingress | HTTPS | Optional, for GitHub/GitLab webhooks |

See `13-current-status-and-port-allocation.md` for the complete port allocation table.

---

## Current Status

The remote orchestrator is at **Tier 3H** priority (P2 — planned):

- **roko-serve crate**: Scaffold exists at `crates/roko-serve/`, but the HTTP API is not wired
- **REST endpoints**: Designed and documented here, not yet implemented
- **Realtime surface**: Designed as a shared WebSocket/SSE/gRPC contract, not yet implemented
- **Authentication**: Designed, not yet implemented
- **Multi-project management**: Depends on the server API being wired
- **Webhook integration**: Designed, depends on server API
- **Cost tracking**: Partially implemented in `roko-learn` (per-run efficiency Pulses exist)

The prerequisite is wiring the `roko-serve` Axum server and connecting it to the existing
orchestration engine in `roko-cli/src/orchestrate.rs`. The orchestration engine itself is
fully functional — it needs an HTTP interface, not a reimplementation.
