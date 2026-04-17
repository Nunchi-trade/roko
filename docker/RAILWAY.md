# Railway Deployment

Deploy mirage-rs (with dashboard) to [Railway](https://railway.app).

## Prerequisites

```bash
npm install -g @railway/cli
railway login
railway link   # link to your Railway project
```

## Deploy

```bash
railway up
```

This uses `docker/mirage-demo.Dockerfile` (configured in `railway.toml`).

## What you get

| Endpoint | Description |
|---|---|
| `https://<service>.railway.app/dashboard` | Interactive dashboard UI |
| `https://<service>.railway.app/` | JSON-RPC (eth_blockNumber, etc.) |
| `https://<service>.railway.app/relay/health` | Relay health check |
| `https://<service>.railway.app/api/*` | REST API |

The dashboard auto-connects to `window.location.origin` — no manual URL entry needed.

## Environment variables

Set these in the Railway UI (Settings > Variables):

| Variable | Default | Description |
|---|---|---|
| `ETH_RPC_URL` | *(none)* | Upstream Ethereum RPC URL for mainnet fork (required for live blocks) |
| `PORT` | `8545` | Railway auto-injects this |
| `RUST_LOG` | `info` | Log level |
| `MIRAGE_SNAPSHOT_INTERVAL_SECS` | `15` | State snapshot frequency |

## Railway constraints

- **No `VOLUME` directive**: Railway ignores Docker `VOLUME` — use environment-configured paths instead
- **`PORT` is auto-injected**: Railway sets `PORT` automatically; the image respects it
- **Cache mount IDs**: Railway requires `--mount=type=cache,id=<name>` format — omit the `id` or use plain names (no service UUIDs)
- **No persistent disk by default**: State is ephemeral unless you attach a Railway volume

## Demo agents

Open `/dashboard/demo-agents.html` in a browser to spawn 3 demo agents (Sentinel, Strategist, Archivist) that connect to the relay via WebSocket. They auto-respond to messages and stay connected as long as the tab is open.

## Dockerfiles

| File | Purpose |
|---|---|
| `docker/mirage.Dockerfile` | Production — no dashboard |
| `docker/mirage-demo.Dockerfile` | Demo / Railway — includes dashboard |
