# Railway Deployment Guide

The recommended Railway path is the root `Dockerfile`: one Railway service, one
public HTTP port, with `roko serve` as the public process and `mirage-rs` plus
`agent-relay` as loopback sidecars in the same container.

This keeps first deploys simple:

```text
Railway Service
├── roko serve    public  0.0.0.0:$PORT
├── mirage-rs     private 127.0.0.1:8545
└── agent-relay   private 127.0.0.1:9011
```

The image build is strict: `roko`, `mirage-rs`, and `agent-relay` are required
artifacts. If any of them fails to build, the deployment fails instead of
shipping a control plane with missing sidecars.

The older multi-service layout is still useful for larger deployments with
independent workers, but it should not be the default path for a simple Railway
demo.

## Architecture

### Single-service default

- **Dockerfile:** root `Dockerfile`
- **Railway config:** root `railway.toml`
- **Healthcheck:** `/health`
- **Public port:** Railway `$PORT`
- **Internal chain URL:** `http://127.0.0.1:8545`
- **Internal relay URL:** `http://127.0.0.1:9011`
- **Startup script:** `docker/start-railway.sh`
- **State:** `/workspace/.roko`, or `RAILWAY_VOLUME_MOUNT_PATH` symlinked to
  `/workspace/.roko` when Railway mounts a volume elsewhere

### Multi-service layout

```
Railway Project
├── roko          (control plane — roko serve on :6677)
├── mirage        (chain relay — optional, --with-mirage)
└── roko-worker-* (agent workers — one per template, --workers)
```

All services are deployed from Dockerfiles in this repo via Railway's GitHub integration. The CLI calls Railway's GraphQL API directly (`RailwayApiBackend`, 831 LOC).

## Prerequisites

1. A Railway account with a team token (Settings > Tokens > Team Token)
2. `ANTHROPIC_API_KEY` (or other LLM provider keys) for agent workers
3. Roko built locally: `cargo build --release -p roko-cli`

## Quick Start

```bash
# Set your Railway API token
export RAILWAY_API_TOKEN="your-token-here"

# Deploy just the control plane
roko deploy railway

# Deploy control plane + mirage + a code-implementer worker
roko deploy railway --with-mirage --workers code-implementer

# Deploy with multiple workers
roko deploy railway --workers code-implementer,pr-review,gate-fixer
```

## Configuration

### roko.toml

```toml
[deploy]
backend = "railway-api"
railway_api_token = "..."          # or use RAILWAY_API_TOKEN env var
worker_image = "ghcr.io/nunchi-trade/roko-worker:latest"
# project_id = "..."              # optional: reuse existing project
# environment_id = "..."          # optional: reuse existing environment
# default_region = "us-west1"     # optional
```

### Project context persistence

After the first deploy, roko writes `.roko/state/railway.json`:

```json
{
  "project_id": "abc-123",
  "environment_id": "def-456"
}
```

Subsequent deploys (including `--workers`) reuse this project automatically. This is how multiple agents deploy into the same Railway project without manual ID passing.

## Images

Published to GHCR on every push to `main` by `.github/workflows/docker-publish.yml`:

| Image | Dockerfile | Purpose |
|---|---|---|
| `ghcr.io/nunchi-trade/roko:latest` | `docker/roko.Dockerfile` | Control plane (`roko serve`) |
| `ghcr.io/nunchi-trade/roko-worker:latest` | `docker/worker.Dockerfile` | Agent workers (Claude, OpenAI, Ollama) |
| `ghcr.io/nunchi-trade/mirage:latest` | `docker/mirage.Dockerfile` | Chain relay (EVM fork + agent relay) |

Tags: `latest` (main only), `sha-<short>`, `v<version>` (on version tags).

## Services

### roko-serve (control plane)

- **Dockerfile:** `docker/roko.Dockerfile`
- **Healthcheck:** `/api/health`
- **Port:** `6677` (Railway auto-maps via `PORT`)
- **Volume:** `/workspace/.roko` (state, signals, episodes, learning data)
- **Endpoints:** ~85 REST routes + SSE + WebSocket

### mirage (chain relay)

- **Dockerfile:** `docker/mirage.Dockerfile`
- **Healthcheck:** `/relay/health`
- **Port:** `8545`
- **Volume:** `/workspace/.roko` (chain snapshots)
- **Entrypoint:** `entrypoint.sh` — starts as root, fixes volume permissions via `gosu`, drops to `mirage` user
- **Demo block production:** set `MIRAGE_BLOCK_INTERVAL_MS=1000` for one local block per second when no live upstream websocket is attached; set it empty to disable auto-mining.

### roko-worker-* (agent workers)

- **Dockerfile:** `docker/worker.Dockerfile`
- **Healthcheck:** `/health`
- **Port:** `8080`
- **Runtime:** Claude CLI, Python3 + openai (for OpenAI-compat backends), curl (for Ollama HTTP)

Workers receive their configuration as a base64-encoded `ROKO_TEMPLATE_JSON` env var. The control plane URL is passed via `ROKO_CONTROL_PLANE_URL`.

## Environment Variables

### All services

| Variable | Description |
|---|---|
| `RUST_LOG` | Log level (default: `info`) |
| `PORT` | Railway auto-injects this |

### roko-serve

| Variable | Description |
|---|---|
| `ANTHROPIC_API_KEY` | LLM provider key (passed through to agents) |
| `GITHUB_TOKEN` | For GitHub integrations |
| `ROKO_SERVER_AUTH_TOKEN` | API authentication token |

### Workers

| Variable | Description |
|---|---|
| `ROKO_TEMPLATE_JSON` | Base64-encoded agent template (set automatically by deploy) |
| `ROKO_CONTROL_PLANE_URL` | Control plane URL for callbacks (set automatically) |
| `ROKO_DEPLOYMENT_ID` | Deployment tracking ID |
| `ANTHROPIC_API_KEY` | LLM provider key |
| `OPENAI_API_KEY` | For OpenAI-compat backends |

### mirage

| Variable | Description |
|---|---|
| `MIRAGE_STATE_DIR` | Snapshot directory (default: `/workspace/.roko/state`) |
| `MIRAGE_SNAPSHOT_INTERVAL_SECS` | Snapshot frequency (default: `15`) |
| `MIRAGE_BLOCK_INTERVAL_MS` | Local auto-mining interval for demo deployments (default: `1000` in `docker/mirage-demo.Dockerfile`; unset/empty disables it) |
| `ETH_RPC_URL` | Upstream Ethereum RPC for mainnet fork |
| `ISFR_SERVICE_URL` | Optional upstream ISFR service URL. If unset, `/api/isfr/*` returns local `no_data` fallback responses instead of 502s. |
| `ISFR_STRICT_PROXY` | Set to `true` to return 502 when the configured ISFR service is unavailable. |

## Per-User Railway Tokens (Dashboard API)

The REST API at `POST /api/deployments` accepts per-request Railway tokens so dashboard users can bring their own:

```bash
# Via header (preferred)
curl -X POST https://<roko-serve>/api/deployments \
  -H 'X-Railway-Token: user-railway-token' \
  -H 'Content-Type: application/json' \
  -d '{"template": "pr-review", "params": {"repo": "org/repo", "pr_number": "42"}}'

# Via body field
curl -X POST https://<roko-serve>/api/deployments \
  -H 'Content-Type: application/json' \
  -d '{"template": "pr-review", "railway_token": "user-token", "params": {}}'
```

Priority: `X-Railway-Token` header > `railway_token` body > server config token.

## Available Worker Templates

Built-in templates (always available):

| Template | Role | Description |
|---|---|---|
| `code-implementer` | implementer | Writes code, runs tests, iterates until gates pass |
| `pr-review` | reviewer | Reads diff, leaves inline comments (requires `github` MCP) |
| `auto-plan` | planner | Generates implementation plans from PRDs |
| `gate-fixer` | implementer | Diagnoses and fixes gate failures |
| `doc-lifecycle` | scribe | Transforms notes into structured PRDs |
| `slack-notify` | operator | Posts deployment notifications (requires `slack` MCP) |

Custom templates: add TOML files to `.roko/templates/` or `templates/`.

## Multi-Backend Workers

Workers support multiple LLM providers via the `provider` field in templates:

```toml
# .roko/templates/openai-implementer.toml
name = "openai-implementer"
description = "Code implementation via OpenAI"
model = "gpt-4o"
role = "implementer"
provider = "openai"
system_prompt = "..."
max_turns = 30
```

Available providers: `claude` (default), `openai`, `ollama`, `gemini`, `perplexity`.

## Railway Constraints

- **No `VOLUME` directive**: Railway ignores Docker `VOLUME` — attach volumes via the Railway UI or API
- **`PORT` is auto-injected**: Railway sets `PORT` automatically
- **Volume permissions**: `entrypoint.sh` handles this — starts as root, chowns the volume mount, drops to non-root via `gosu`
- **Build caching**: Railway's builder does not support `--mount=type=cache` (fails with "missing cacheKey prefix"). GHCR builds use GHA cache layer instead (`cache-from: type=gha` in the workflow)

## Troubleshooting

### Snapshot permission denied

If mirage logs show `periodic snapshot failed: Permission denied`, the volume mount permissions are wrong. The `entrypoint.sh` fix handles this automatically for new deploys. For existing deploys:

1. Check `MIRAGE_STATE_DIR` matches the volume mount path
2. Ensure the container starts as root (no `USER` directive before `ENTRYPOINT`)
3. `entrypoint.sh` will chown the volume and drop to the `mirage` user

### Workers not connecting

1. Check `ROKO_CONTROL_PLANE_URL` is set and reachable
2. Check `ROKO_TEMPLATE_JSON` is valid base64 (decode and parse as JSON)
3. Check worker logs: `roko deploy railway` → Railway dashboard → service logs

### Project ID not found

If `roko deploy railway` fails with "project not found", delete `.roko/state/railway.json` to force a fresh project creation.
