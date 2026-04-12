# Cloud Deployment: Fly.io

> Roko services deploy to Fly.io as Firecracker microVMs with persistent volumes, automatic
> TLS, private networking, auto-stop on idle, and auto-start on request. This document covers
> the fly.toml configuration per service, the deploy scripts, secret management, scaling,
> cost estimates, the web terminal console service, and the deploy command workflow.


> **Implementation**: Specified

---

## Architecture on Fly.io

Fly.io runs each service as a Firecracker microVM (called a "machine"). Each service gets its
own app, its own domain (`<app>.fly.dev`), and its own configuration. Services in the same Fly
organization communicate over a private IPv6 network (6PN) using `.internal` DNS names.

For example, the Roko CLI orchestrator reaches the API server at
`http://roko-serve.internal:8080`. This traffic stays on Fly's private network — it never
touches the public internet.

```
                    ┌───────────────────────────────────────────┐
                    │              Fly.io Organization           │
                    │                                           │
                    │  ┌─────────────┐    ┌──────────────────┐ │
  HTTPS ──────────▶ │  │ roko-serve  │◀──▶│   roko-cli       │ │
  (public)          │  │ HTTP API    │    │   orchestrator   │ │
                    │  │ port 8080   │    │   port 8080      │ │
                    │  └─────────────┘    │   volume: /data   │ │
                    │         ▲            └──────────────────┘ │
                    │         │ .internal                       │
                    │         ▼                                 │
                    │  ┌─────────────┐    ┌──────────────────┐ │
                    │  │ mirage-rs   │    │   roko-console   │ │
                    │  │ EVM fork    │    │   web terminal   │ │
                    │  │ port 8545   │    │   port 3000      │ │
                    │  │ volume:state│    │   (Caddy proxy)  │ │
                    │  └─────────────┘    └──────────────────┘ │
                    │                                           │
                    └───────────────────────────────────────────┘
```

---

## Directory Structure

All Fly.io configuration lives in `deploy/`:

```
deploy/
  fly/
    roko-cli/fly.toml           # Roko CLI orchestrator
    roko-serve/fly.toml         # Roko HTTP API server
    mirage-rs/fly.toml          # mirage-rs EVM fork simulator
    console/fly.toml            # Web terminal console
  console/
    Caddyfile                   # Reverse proxy for web terminals
    static/index.html           # xterm.js web terminal UI
  scripts/
    fly-deploy.sh               # Deploy one or all services
    fly-secrets.sh              # Set secrets per service
    fly-status.sh               # Health check all services
    fly-logs.sh                 # Tail logs with service picker
```

---

## fly.toml Per Service

### roko-cli (Orchestrator)

```toml
# deploy/fly/roko-cli/fly.toml
app = "roko-cli"
primary_region = "iad"

[build]
  dockerfile = "docker/roko-cli-full.Dockerfile"

[env]
  RUST_LOG = "roko=info"

[mounts]
  source = "roko_data"
  destination = "/data"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = "stop"
  auto_start_machines = true
  min_machines_running = 0

  [http_service.concurrency]
    type = "requests"
    hard_limit = 100
    soft_limit = 50

[[http_service.checks]]
  grace_period = "15s"
  interval = "30s"
  method = "GET"
  path = "/health"
  timeout = "5s"

[[services]]
  internal_port = 7681
  protocol = "tcp"

  [[services.ports]]
    port = 7681
    handlers = ["tls", "http"]

[[vm]]
  size = "shared-cpu-2x"
  memory = "2gb"
```

**Key settings**:
- `auto_stop_machines = "stop"` with `min_machines_running = 0` — the machine stops when
  idle. No CPU/RAM charges while stopped. When a request arrives, Fly starts the machine
  (~2-3 second cold start for a Rust binary).
- `[mounts]` — persistent Fly volume for `.roko/` state directory. Survives machine stops
  and restarts. The volume is created by the deploy script on first run.
- Memory is 2GB because the code index (tree-sitter AST + HDC fingerprints + symbol graph)
  can grow for large codebases.
- Port 7681 is exposed for ttyd (web terminal access via the full image).

### roko-serve (HTTP API)

```toml
# deploy/fly/roko-serve/fly.toml
app = "roko-serve"
primary_region = "iad"

[build]
  dockerfile = "docker/roko-serve.Dockerfile"

[env]
  RUST_LOG = "roko_serve=info"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = "stop"
  auto_start_machines = true
  min_machines_running = 0

  [http_service.concurrency]
    type = "requests"
    hard_limit = 500
    soft_limit = 250

[[http_service.checks]]
  grace_period = "10s"
  interval = "30s"
  method = "GET"
  path = "/health"
  timeout = "5s"

[[vm]]
  size = "shared-cpu-2x"
  memory = "1gb"
```

### mirage-rs (EVM Fork Simulator)

```toml
# deploy/fly/mirage-rs/fly.toml
app = "roko-mirage"
primary_region = "iad"

[build]
  dockerfile = "docker/mirage-rs.Dockerfile"

[env]
  RUST_LOG = "mirage_rs=info"

[mounts]
  source = "mirage_state"
  destination = "/data"

[http_service]
  internal_port = 8545
  force_https = true
  auto_stop_machines = "stop"
  auto_start_machines = true
  min_machines_running = 0

  [http_service.concurrency]
    type = "requests"
    hard_limit = 200
    soft_limit = 100

[[http_service.checks]]
  grace_period = "15s"
  interval = "30s"
  method = "POST"
  path = "/"
  timeout = "5s"
  headers = { "Content-Type" = "application/json" }
  body = '{"jsonrpc":"2.0","method":"web3_clientVersion","params":[],"id":1}'

[[vm]]
  size = "shared-cpu-2x"
  memory = "2gb"
```

mirage-rs uses a JSON-RPC health check (standard Ethereum `web3_clientVersion` call) because
it does not expose a separate HTTP health endpoint — the JSON-RPC server is the primary
interface.

### Console (Web Terminal)

```toml
# deploy/fly/console/fly.toml
app = "roko-console"
primary_region = "iad"

[build]
  dockerfile = "docker/console.Dockerfile"

[env]
  SERVICES = "roko-cli:roko-cli.internal:7681,roko-serve:roko-serve.internal:7681"

[http_service]
  internal_port = 3000
  force_https = true
  auto_stop_machines = "stop"
  auto_start_machines = true
  min_machines_running = 0

[[http_service.checks]]
  grace_period = "5s"
  interval = "30s"
  method = "GET"
  path = "/health"
  timeout = "3s"

[[vm]]
  size = "shared-cpu-1x"
  memory = "256mb"
```

The console is a lightweight Caddy reverse proxy that forwards WebSocket connections to each
service's ttyd instance over Fly's internal network. It serves a static HTML page with xterm.js
that renders the terminal in the browser.

---

## Deploy Scripts

### `fly-deploy.sh`

```bash
#!/bin/bash
# deploy/scripts/fly-deploy.sh
#
# Deploy one or all Roko services to Fly.io.
#
# Usage:
#   ./deploy/scripts/fly-deploy.sh all             # Deploy everything
#   ./deploy/scripts/fly-deploy.sh roko-cli        # Deploy just the CLI
#   ./deploy/scripts/fly-deploy.sh roko-serve      # Deploy just the API server
#   ./deploy/scripts/fly-deploy.sh mirage-rs       # Deploy just mirage-rs
#   ./deploy/scripts/fly-deploy.sh console         # Deploy just the web console

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

deploy_service() {
    local service="$1"
    local fly_dir="$REPO_ROOT/deploy/fly/$service"

    if [ ! -f "$fly_dir/fly.toml" ]; then
        echo "Error: No fly.toml found at $fly_dir/fly.toml"
        exit 1
    fi

    echo "=== Deploying $service ==="

    # Create the app if it doesn't exist
    local app_name
    app_name=$(grep '^app = ' "$fly_dir/fly.toml" | sed 's/app = "\(.*\)"/\1/')

    if ! fly apps list --json | jq -e ".[] | select(.Name == \"$app_name\")" > /dev/null 2>&1; then
        echo "Creating app $app_name..."
        fly apps create "$app_name" --org personal
    fi

    # Create volume if the service needs one
    if grep -q '\[mounts\]' "$fly_dir/fly.toml"; then
        local vol_name
        vol_name=$(grep 'source = ' "$fly_dir/fly.toml" | head -1 | sed 's/.*source = "\(.*\)"/\1/')
        local region
        region=$(grep 'primary_region' "$fly_dir/fly.toml" | sed 's/.*= "\(.*\)"/\1/')

        if ! fly volumes list --app "$app_name" --json | jq -e '.[0]' > /dev/null 2>&1; then
            echo "Creating volume $vol_name in $region..."
            fly volumes create "$vol_name" --app "$app_name" --region "$region" --size 1
        fi
    fi

    # Deploy
    fly deploy --config "$fly_dir/fly.toml" --app "$app_name" --remote-only

    echo "=== $service deployed ==="
    echo ""
}

case "${1:-all}" in
    all)
        deploy_service roko-serve
        deploy_service roko-cli
        deploy_service console
        ;;
    *)
        deploy_service "$1"
        ;;
esac
```

### `fly-secrets.sh`

```bash
#!/bin/bash
# deploy/scripts/fly-secrets.sh
#
# Set secrets for Roko services on Fly.io.
# Reads from .env file or environment variables.
#
# Usage:
#   ./deploy/scripts/fly-secrets.sh

set -euo pipefail

# Load .env if it exists
if [ -f .env ]; then
    set -a
    source .env
    set +a
fi

echo "Setting secrets for roko-serve..."
fly secrets set \
    ANTHROPIC_API_KEY="${ANTHROPIC_API_KEY:?Set ANTHROPIC_API_KEY}" \
    --app roko-serve

echo "Setting secrets for roko-cli..."
fly secrets set \
    ANTHROPIC_API_KEY="${ANTHROPIC_API_KEY}" \
    --app roko-cli

if [ -n "${CONSOLE_AUTH_TOKEN:-}" ]; then
    echo "Setting secrets for roko-console..."
    fly secrets set \
        CONSOLE_AUTH_TOKEN="$CONSOLE_AUTH_TOKEN" \
        --app roko-console
fi

echo "Done. Secrets set."
```

### `fly-status.sh`

```bash
#!/bin/bash
# deploy/scripts/fly-status.sh
#
# Check health of all deployed Roko services.

set -euo pipefail

services=("roko-cli" "roko-serve" "roko-console" "roko-mirage")

printf "%-20s %-12s %-40s %-10s\n" "SERVICE" "STATUS" "URL" "MACHINES"
printf "%-20s %-12s %-40s %-10s\n" "-------" "------" "---" "--------"

for svc in "${services[@]}"; do
    if fly apps list --json 2>/dev/null | jq -e ".[] | select(.Name == \"$svc\")" > /dev/null 2>&1; then
        url="https://$svc.fly.dev"
        machines=$(fly machines list --app "$svc" --json 2>/dev/null | jq 'length')
        status=$(curl -sf "$url/health" -o /dev/null -w "%{http_code}" 2>/dev/null || echo "down")
        if [ "$status" = "200" ]; then
            printf "%-20s %-12s %-40s %-10s\n" "$svc" "healthy" "$url" "started:$machines"
        else
            printf "%-20s %-12s %-40s %-10s\n" "$svc" "unhealthy" "$url" "$machines"
        fi
    else
        printf "%-20s %-12s %-40s %-10s\n" "$svc" "not deployed" "-" "-"
    fi
done
```

---

## Secret Management on Fly.io

Provider API keys live as Fly secrets. They become environment variables inside the machine.
They never appear in fly.toml, Docker images, or logs.

### Secret Flow

```
deploy command
  → reads --anthropic-key flag, or $ANTHROPIC_API_KEY, or prompts
  → runs: fly secrets set ANTHROPIC_API_KEY=<value> --app roko-cli
  → secrets are encrypted at rest on Fly's infrastructure
  → injected as env vars when the machine starts
```

### Detecting Existing Secrets

On redeploy, the deploy script checks what secrets are already set:

```bash
fly secrets list --app roko-cli --json
```

It only prompts for or sets missing secrets. If the user deploys without `--anthropic-key` and
the secret already exists on Fly, it skips that secret.

---

## Cost Estimates

Fly.io pricing for Roko services with auto-stop enabled (prices as of early 2026):

| Service | VM Size | Memory | Auto-Stop | Estimated Cost |
|---|---|---|---|---|
| roko-cli | shared-cpu-2x | 2GB | Yes | ~$3-7/mo (active hours only) |
| roko-serve | shared-cpu-2x | 1GB | Yes | ~$2-5/mo (active hours only) |
| mirage-rs | shared-cpu-2x | 2GB | Yes | ~$3-7/mo (active hours only) |
| console | shared-cpu-1x | 256MB | Yes | ~$1-2/mo |
| Volumes (1GB each) | — | — | — | ~$0.15/mo per volume |

With `auto_stop_machines = "stop"` and `min_machines_running = 0`, machines stop when idle.
No CPU/RAM charges while stopped. The machine starts in ~2-3 seconds when a request arrives
(cold start for a Rust binary is fast).

For a personal deployment where services are used intermittently, the total cost is typically
$5-15/month. For always-on production deployments, set `min_machines_running = 1` to eliminate
cold starts, at the cost of continuous charges.

---

## Custom Domains

When deploying with a custom domain:

```bash
# Attach a custom domain to a Fly app
fly certs create roko-serve api.example.com

# Fly prints DNS instructions:
#   Add CNAME: api.example.com → roko-serve.fly.dev
#   Certificate provisions automatically once DNS propagates.
```

Fly handles TLS via Let's Encrypt. The user points a CNAME at `<app>.fly.dev` and Fly
provisions the certificate automatically (HTTP-01 challenge). No ACME DNS records, no cert
files, no renewal cron.

### Check Certificate Status

```bash
$ fly certs show roko-serve api.example.com

Hostname:     api.example.com
Status:       Ready
Certificate:  Issued (expires 2026-09-22)
```

---

## Console Service: Web Terminal

The console service provides browser-based access to any service's terminal via xterm.js. It
is a lightweight Caddy reverse proxy that forwards WebSocket connections to each service's ttyd
instance over Fly's internal network.

### Caddyfile

```
# deploy/console/Caddyfile
{
    admin off
}

:3000 {
    # Health check
    handle /health {
        respond "ok" 200
    }

    # WebSocket proxy to service ttyd instances
    handle /ws/roko-cli {
        reverse_proxy roko-cli.internal:7681
    }

    handle /ws/roko-serve {
        reverse_proxy roko-serve.internal:7681
    }

    # Static frontend
    handle {
        root * /srv/static
        file_server
    }
}
```

### Frontend (xterm.js)

The console frontend is a single HTML page with xterm.js that connects to the WebSocket
endpoint for a selected service:

```html
<!-- deploy/console/static/index.html (simplified) -->
<!DOCTYPE html>
<html>
<head>
    <title>Roko Console</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm/css/xterm.css">
</head>
<body>
    <nav>
        <button onclick="connect('roko-cli')">Roko CLI</button>
        <button onclick="connect('roko-serve')">Roko Serve</button>
    </nav>
    <div id="terminal"></div>
    <script src="https://cdn.jsdelivr.net/npm/xterm/lib/xterm.js"></script>
    <script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit/lib/xterm-addon-fit.js"></script>
    <script src="https://cdn.jsdelivr.net/npm/xterm-addon-attach/lib/xterm-addon-attach.js"></script>
    <script>
        const term = new Terminal({ theme: { background: '#0a0a0a' } });
        const fitAddon = new FitAddon.FitAddon();
        term.loadAddon(fitAddon);
        term.open(document.getElementById('terminal'));
        fitAddon.fit();

        function connect(service) {
            const ws = new WebSocket(`wss://${location.host}/ws/${service}`);
            const attachAddon = new AttachAddon.AttachAddon(ws);
            term.loadAddon(attachAddon);
        }
    </script>
</body>
</html>
```

### Deliverable

```
https://roko-console.fly.dev/
  ├── [Tab: Roko CLI]   → Live TUI in the browser
  └── [Tab: Roko Serve] → Live server logs in the browser
```

---

## Deployment as a Gate Step

A plan can include deployment as a verification step in the gate pipeline:

```markdown
### Verification
#### INV-005: API Deployment
- Deploy to staging: `roko deploy --target fly --env staging`
- Health check passes within 30 seconds
- Smoke test: `curl https://staging-api.example.com/health` returns 200
```

The Roko Orchestrator treats deployment like any other gate. If the deploy fails or the health
check does not pass, the plan does not advance. This closes the loop from PRD to running
software.

---

## Future Deployment Targets

Fly.io is the primary cloud target because it has the best CLI tooling and the simplest mental
model (one machine = one service). Planned additions:

- **Railway**: Similar model to Fly, different CLI
- **Bare VPS via SSH**: `roko deploy --target ssh --host user@ip` would rsync the binary and
  manage it via systemd
- **Docker Compose**: For local multi-service testing before cloud deployment (already
  implemented — see `03-docker.md`)

The deployment interface is designed as a trait. Adding a target means implementing `deploy()`,
`teardown()`, `status()`, and `logs()`.
