# Deploy Roko & Mirage

Docker images:

| Image | Description |
|---|---|
| `ghcr.io/nunchi-trade/mirage:latest` | Mirage EVM fork with agent relay |
| `ghcr.io/nunchi-trade/roko:latest` | Roko control plane (`roko serve`) |
| `ghcr.io/nunchi-trade/roko-worker:latest` | Roko agent worker |

## Railway (1-click deploy)

### Mirage

[![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/deploy/EpAhfE?referralCode=U6jTsr)

### Roko Control Plane

[![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/deploy/XBJ94u?referralCode=U6jTsr)

### Roko Agent Worker

[![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/deploy/Nv9MnH?referralCode=U6jTsr)

Required env vars:

| Variable | Description |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic API key for Claude |
| `ROKO_CONTROL_PLANE_URL` | URL of the roko control plane |
| `ROKO_TEMPLATE_JSON` | Base64-encoded agent template JSON |

## Fly.io

### Mirage

```bash
cd deploy/mirage
fly launch --now
```

### Roko Control Plane

```bash
# From repo root
fly launch --now
```

### Roko Agent Worker

```bash
cd deploy/roko-agent
fly secrets set ANTHROPIC_API_KEY=sk-...
fly secrets set ROKO_TEMPLATE_JSON=$(echo '{"name":"worker"}' | base64)
fly launch --now
```

## Docker (local)

```bash
# Mirage
docker run -p 8545:8545 ghcr.io/nunchi-trade/mirage:latest

# Roko control plane
docker run -p 3000:3000 ghcr.io/nunchi-trade/roko:latest

# Roko agent worker
docker run -p 8080:8080 \
  -e ANTHROPIC_API_KEY=sk-... \
  -e ROKO_CONTROL_PLANE_URL=http://host.docker.internal:3000 \
  ghcr.io/nunchi-trade/roko-worker:latest
```

## Environment Variables

### Mirage

| Variable | Default | Description |
|---|---|---|
| `PORT` | `8545` | HTTP listen port |
| `MIRAGE_DASHBOARD_DIR` | — | Path to static dashboard files (demo image only) |
| `MIRAGE_STATE_DIR` | `/workspace/.roko/state` | Persistent state directory |
| `MIRAGE_SNAPSHOT_INTERVAL_SECS` | `15` | State snapshot interval |
| `MIRAGE_BLOCK_INTERVAL_MS` | `1000` | Block production interval |
| `RUST_LOG` | `info` | Log level |

### Roko Control Plane

| Variable | Default | Description |
|---|---|---|
| `PORT` | `3000` | HTTP listen port |
| `RUST_LOG` | `info` | Log level |

### Roko Agent Worker

| Variable | Default | Description |
|---|---|---|
| `PORT` | `8080` | HTTP listen port |
| `ROKO_TEMPLATE_JSON` | — | Base64-encoded agent template |
| `ROKO_CONTROL_PLANE_URL` | — | Control plane URL |
| `ROKO_DEPLOYMENT_ID` | — | Deployment identifier |
| `ANTHROPIC_API_KEY` | — | Anthropic API key |
| `RUST_LOG` | `info` | Log level |
