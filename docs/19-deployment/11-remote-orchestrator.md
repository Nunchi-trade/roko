# Remote Orchestrator

> Roko can run as a long-lived HTTP service, letting you interact with it from anywhere —
> submit plans, trigger runs, stream events, and manage projects via REST API. This document
> covers the server mode, the HTTP API surface, authentication, multi-project management,
> webhook integration, the local-to-remote transition, and the `roko-serve` crate.

---

## Overview

The remote orchestrator transforms Roko from a local CLI tool into a deployed service. The
same orchestration engine (DAG executor, agent dispatch, gate pipeline, signal persistence)
runs behind an HTTP API instead of a terminal interface.

Use cases:
- **Team usage**: Deploy one Roko instance, give team members API keys, everyone can view
  progress, trigger runs, and upload PRDs from their own machines
- **CI integration**: Trigger plan runs from GitHub Actions, Jenkins, or any CI system
- **Autonomous operation**: The remote orchestrator runs continuously, processing webhooks
  and scheduled subscriptions without human intervention
- **Mobile/tablet access**: Interact with Roko from any device with a browser

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

# Signals and episodes
GET    /v1/projects/:id/signals       # Query signals (Engrams)
GET    /v1/projects/:id/episodes      # Query episode log
```

### Real-Time Streaming

```
GET    /v1/projects/:id/events        # SSE stream of agent events
WS     /v1/projects/:id/ws            # WebSocket for bidirectional control
```

The SSE endpoint streams events as they happen: agent turn starts, tool calls, gate results,
task completions, errors. This is the remote equivalent of watching the TUI.

The WebSocket endpoint adds bidirectional control: inject messages to agents, pause/resume
runs, change configuration mid-run.

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
| `read` | View projects, plans, runs, artifacts, event streams |
| `write` | Create projects, start runs, upload PRDs, inject messages |
| `admin` | Manage API keys, server config, delete projects |

Keys are stored in the server's state directory (`.roko/auth.db` — SQLite). They are
rotatable, revocable, and auditable.

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
- Signal and episode logs

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

### Streaming Events

```bash
curl -N https://roko-serve.fly.dev/v1/projects/proj_abc123/events \
  -H "Authorization: Bearer roko_sk_..."

# SSE stream:
# data: {"type":"agent_started","agent_id":"a1","task":"01-setup","model":"claude-sonnet-4-6"}
# data: {"type":"tool_call","agent_id":"a1","tool":"write_file","path":"src/main.rs"}
# data: {"type":"gate_result","task":"01-setup","gate":"compile","verdict":"pass"}
# data: {"type":"task_completed","task":"01-setup","status":"success","duration_ms":12345}
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

1. **Push event**: When code is pushed to a monitored branch, the server pulls the latest
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

    // Parse the event
    let event_type = headers.get("x-github-event")
        .and_then(|v| v.to_str().ok());

    match event_type {
        Some("push") => handle_push_event(&body, &state).await,
        Some("pull_request") => handle_pr_event(&body, &state).await,
        Some("issue_comment") => handle_comment_event(&body, &state).await,
        _ => StatusCode::OK, // Acknowledge but ignore other events
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
                    (same DAG executor, same gates, same signals)

5. Stream events:   curl -N .../projects/:id/events
                    (SSE stream, same events as TUI would show)
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

When a budget limit is reached, the server pauses the run and notifies via the event stream.
The run can be resumed after the budget is increased or the daily reset occurs.

---

## Port Allocation

The remote orchestrator and related services use these ports:

| Port | Service | Protocol | Notes |
|---|---|---|---|
| 8080 | roko-serve HTTP API | HTTPS | Primary API endpoint |
| 8443 | roko-serve WebSocket | WSS | Bidirectional event stream |
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
- **SSE streaming**: Not yet implemented
- **Authentication**: Designed, not yet implemented
- **Multi-project management**: Depends on the server API being wired
- **Webhook integration**: Designed, depends on server API
- **Cost tracking**: Partially implemented in `roko-learn` (per-run efficiency events exist)

The prerequisite is wiring the `roko-serve` Axum server and connecting it to the existing
orchestration engine in `roko-cli/src/orchestrate.rs`. The orchestration engine itself is
fully functional — it needs an HTTP interface, not a reimplementation.
