# Docker Deployment

> Roko services are packaged as Docker images for server deployments, Fly.io, and local
> development stacks. This document covers the Dockerfile architecture, the cargo-chef build
> pattern, slim vs full image variants, the Docker registry strategy, the CI workflow for
> automated image builds, and the docker-compose configuration for running the full stack
> locally.


> **Implementation**: Specified

---

## Image Registry and Naming

All images are published to GitHub Container Registry (ghcr.io). Naming convention:

```
ghcr.io/nunchi/roko-cli:latest              # Slim, latest
ghcr.io/nunchi/roko-cli:0.3.0               # Slim, pinned version
ghcr.io/nunchi/roko-cli:0.3.0-full          # Full (tmux + ttyd), pinned version
ghcr.io/nunchi/roko-serve:latest
ghcr.io/nunchi/roko-serve:0.3.0
ghcr.io/nunchi/mirage-rs:latest
ghcr.io/nunchi/mirage-rs:0.3.0
```

Each service produces two image variants:

- **Slim** — Binary in a distroless or scratch image. For production deployments where
  interactive access is unnecessary. ~5-20MB compressed depending on base image.
- **Full** — Binary + tmux + ttyd (web terminal server). For deployments where operators need
  to attach to the running process via a browser. Uses `debian:bookworm-slim` as the base
  (~50MB compressed).

---

## Two Image Variants

### Slim Images

Slim images contain only the statically-linked binary and minimal system dependencies. They
are the default for production deployments.

**Base image hierarchy** (from practical to minimal):

| Base | Size | Shell | Use case |
|---|---|---|---|
| `debian:bookworm-slim` | ~80MB | Yes | Full images (needs tmux, ttyd, shell) |
| `gcr.io/distroless/cc-debian12` | ~20MB | No | Slim images for dynamically-linked Rust |
| `cgr.dev/chainguard/static` | ~2MB | No | Slim images for statically-linked (musl) |
| `scratch` | 0 | No | Absolute minimum for musl binaries |

For `roko-cli` and `roko-serve` slim images, use `distroless` (needs glibc for alloy/revm
dependencies). For crates with no C FFI dependencies, use `scratch` with musl for the absolute
minimum image size.

All containers run as non-root (`USER 65532`) with `--read-only --cap-drop=ALL` for security.

### Full Images (with Web Terminal)

Full images add tmux (terminal multiplexer), ttyd (WebSocket-to-PTY bridge), and an entrypoint
script that ties them together. The service runs inside a tmux session. ttyd serves that session
over WebSocket on port 7681. A browser can connect to ttyd and see the TUI in real time.

This is critical for `roko-cli` which has a ratatui-based TUI. In production, operators want to
see the TUI from a browser without SSH access.

---

## Directory Structure

All Docker-related files live in `docker/` at the repo root:

```
docker/
  roko-cli.Dockerfile                # Slim: binary + ca-certificates + git
  roko-cli-full.Dockerfile           # Slim + tmux + ttyd + entrypoint script
  roko-serve.Dockerfile              # Slim: binary + ca-certificates
  roko-serve-full.Dockerfile
  mirage-rs.Dockerfile               # Slim: binary + ca-certificates
  mirage-rs-full.Dockerfile
  console.Dockerfile                 # Web terminal multiplexer (Caddy + xterm.js)
  entrypoints/
    roko-cli.sh                      # Starts roko-cli in tmux, starts ttyd alongside
    roko-serve.sh
    mirage-rs.sh
  docker-compose.yml                 # Full local stack
  docker-compose.dev.yml             # Dev overrides (mount source, rebuild on change)
```

---

## Dockerfiles

### Slim Dockerfile: roko-cli

```dockerfile
# docker/roko-cli.Dockerfile
#
# Slim roko-cli image. Just the binary. No terminal access.
# Use roko-cli-full.Dockerfile if you want web terminal support.
#
# Build: docker build -f docker/roko-cli.Dockerfile -t roko-cli:latest .
# Requires: target/x86_64-unknown-linux-musl/release/roko-cli (pre-compiled)

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user with home directory for state
RUN useradd -m -s /bin/bash -d /data roko \
    && mkdir -p /data/.roko/state /data/.roko/prd \
    && chown -R roko:roko /data

COPY target/x86_64-unknown-linux-musl/release/roko-cli /usr/local/bin/roko

USER roko
WORKDIR /data

EXPOSE 8080
ENV RUST_LOG=info

# Default: run roko in server mode.
# Override CMD to run interactively or with different flags.
ENTRYPOINT ["roko"]
CMD ["plan", "run", "--port", "8080", "--bind", "0.0.0.0"]
```

### Slim Dockerfile: roko-serve

```dockerfile
# docker/roko-serve.Dockerfile
#
# Slim roko-serve image. Stateless HTTP API server.
#
# Build: docker build -f docker/roko-serve.Dockerfile -t roko-serve:latest .

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash roko

COPY target/x86_64-unknown-linux-musl/release/roko-serve /usr/local/bin/roko-serve

USER roko
EXPOSE 8080
ENV RUST_LOG=info

ENTRYPOINT ["roko-serve"]
CMD ["--port", "8080", "--bind", "0.0.0.0"]
```

### Full Dockerfile: roko-cli (with Web Terminal)

```dockerfile
# docker/roko-cli-full.Dockerfile
#
# Roko CLI with web terminal access. Runs roko inside tmux,
# serves the tmux session via ttyd on port 7681.
#
# Port 8080: roko server API
# Port 7681: web terminal (ttyd WebSocket)

FROM debian:trixie-slim

# System dependencies
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates git curl tmux procps \
    && rm -rf /var/lib/apt/lists/*

# ttyd: WebSocket-to-PTY bridge. Single static binary, ~1.5MB.
# Serves a terminal session over HTTP/WebSocket.
# https://github.com/tsl0922/ttyd
ARG TTYD_VERSION=1.7.7
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
      amd64) TTYD_ARCH="x86_64" ;; \
      arm64) TTYD_ARCH="aarch64" ;; \
      *) echo "unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac \
    && curl -fsSL \
       "https://github.com/tsl0922/ttyd/releases/download/${TTYD_VERSION}/ttyd.${TTYD_ARCH}" \
       -o /usr/local/bin/ttyd \
    && chmod +x /usr/local/bin/ttyd

# Binary
COPY target/release/roko-cli /usr/local/bin/roko

# Entrypoint script: starts roko in tmux, starts ttyd alongside
COPY docker/entrypoints/roko-cli.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Data volume mount point
RUN useradd -m -s /bin/bash -d /data roko \
    && mkdir -p /data/.roko/state /data/.roko/prd \
    && chown -R roko:roko /data

USER roko
WORKDIR /data

EXPOSE 8080 7681
ENV RUST_LOG=info

CMD ["entrypoint.sh"]
```

---

## Entrypoint Scripts

These scripts start the service inside a tmux session and run ttyd alongside it. If the service
crashes, the container exits so the orchestrator (Docker, Fly.io) can restart it.

### roko-cli Entrypoint

```bash
#!/bin/bash
# docker/entrypoints/roko-cli.sh
#
# Starts roko inside a tmux session with web terminal access via ttyd.
#
# Environment variables:
#   ROKO_PORT       - roko server port (default: 8080)
#   ROKO_DATA_DIR   - data directory (default: /data)
#   TTYD_PORT       - ttyd WebSocket port (default: 7681)
#   TTYD_AUTH       - ttyd basic auth, format "user:pass" (optional)
#                     If not set, ttyd has no auth. Use network-level
#                     isolation (Fly internal network, Docker network)
#                     to restrict access.
#   ROKO_EXTRA_ARGS - additional roko flags (optional)

set -euo pipefail

DATA_DIR="${ROKO_DATA_DIR:-/data}"
ROKO_PORT="${ROKO_PORT:-8080}"
TTYD_PORT="${TTYD_PORT:-7681}"

# Ensure state directories exist on the volume
mkdir -p "$DATA_DIR/.roko/state" "$DATA_DIR/.roko/prd"

# ── Start roko inside tmux ───────────────────────────────────────────
# -x 200 -y 50: initial terminal size (large enough for the TUI).
# When a viewer attaches with a different size, tmux resizes to match.
tmux new-session -d -s roko -x 200 -y 50

# exec replaces the shell so roko is PID 1 inside the tmux pane.
# tee captures output to a log file alongside the TUI rendering.
tmux send-keys -t roko "exec roko plan run \
  --port $ROKO_PORT \
  --bind 0.0.0.0 \
  ${ROKO_EXTRA_ARGS:-} \
  2>&1 | tee $DATA_DIR/.roko/roko.log" Enter

# ── Start ttyd ───────────────────────────────────────────────────────
# ttyd serves the tmux session as a WebSocket on TTYD_PORT.
# -W: enable WebSocket writing (allows keyboard input from browser)
# -c: basic auth credentials (if TTYD_AUTH is set)
TTYD_ARGS=(-p "$TTYD_PORT" -W)
if [ -n "${TTYD_AUTH:-}" ]; then
  TTYD_ARGS+=(-c "$TTYD_AUTH")
fi

ttyd "${TTYD_ARGS[@]}" tmux attach -t roko &
TTYD_PID=$!

# ── Health loop ──────────────────────────────────────────────────────
# If the tmux session dies (roko crashed or exited), kill ttyd and
# exit the container. The orchestrator (Docker, Fly) restarts it.
while tmux has-session -t roko 2>/dev/null; do
  sleep 5
done

echo "roko tmux session exited, shutting down"
kill $TTYD_PID 2>/dev/null || true
exit 1
```

---

## Docker Build Pattern: cargo-chef

cargo-chef achieves ~5× speedup for Docker builds by caching dependency compilation in a
separate layer from source code. The three-stage pattern:

```dockerfile
# Stage 1: Plan — extract dependency graph
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Build — compile deps (cached layer), then source
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json  # Cached!
COPY . .
RUN cargo build --release --bin roko-cli

# Stage 3: Runtime — minimal base image
FROM gcr.io/distroless/cc-debian12 AS runtime
COPY --from=builder /app/target/release/roko-cli /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/roko"]
```

The key insight: stage 2's `cargo chef cook` compiles all dependencies without the source code.
This layer is cached by Docker. When you change source code but not dependencies, the rebuild
only recompiles your code — not all of Tokio, Axum, ratatui, alloy, etc.

First build: ~10-15 minutes (full Rust compile). Subsequent builds with only source changes:
~2-5 minutes (dependencies cached).

### Build Strategy: Pre-Compiled Binaries

The preferred build strategy for CI does not compile Rust inside Docker. Instead, CI
cross-compiles the binary first (using the musl targets), then copies the static binary into
the Docker image:

```bash
# CI step 1: Cross-compile
cross build --release --target x86_64-unknown-linux-musl -p roko-cli
cross build --release --target aarch64-unknown-linux-musl -p roko-cli

# CI step 2: Build Docker image (just copies the binary, no Rust compile)
docker build -f docker/roko-cli.Dockerfile -t roko-cli:latest .
```

This means Docker builds are fast (seconds, not minutes), require no Rust toolchain in the
image, and support multi-arch via Docker buildx.

The cargo-chef pattern is used for the remote builder path (Fly.io), where pre-compiled binaries
are not available and the Docker build must compile from source on the remote builder.

---

## CI Workflow for Docker Builds

`.github/workflows/docker.yml` — Triggers on tag push, builds multi-arch images, pushes to
ghcr.io:

```yaml
name: Docker Images

on:
  push:
    tags: ['*-v*']  # Triggers on: roko-cli-v0.3.0, mirage-rs-v0.5.0, etc.

jobs:
  docker:
    strategy:
      matrix:
        include:
          - service: roko-cli
            binary: roko-cli
            package: roko-cli
          - service: roko-serve
            binary: roko-serve
            package: roko-serve
          - service: mirage-rs
            binary: mirage-rs
            package: mirage-rs

    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write

    steps:
      - uses: actions/checkout@v4

      # Cross-compile the binary for both architectures
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-unknown-linux-musl,aarch64-unknown-linux-musl
      - name: Install cross-compilation tools
        run: cargo install cross
      - name: Build amd64
        run: cross build --release --target x86_64-unknown-linux-musl -p ${{ matrix.package }}
      - name: Build arm64
        run: cross build --release --target aarch64-unknown-linux-musl -p ${{ matrix.package }}

      # Build and push multi-arch Docker images
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      # Slim image
      - uses: docker/build-push-action@v5
        with:
          context: .
          file: docker/${{ matrix.service }}.Dockerfile
          platforms: linux/amd64,linux/arm64
          push: true
          tags: |
            ghcr.io/nunchi/${{ matrix.service }}:${{ github.ref_name }}
            ghcr.io/nunchi/${{ matrix.service }}:latest
          build-args: |
            BINARY=${{ matrix.binary }}

      # Full image (with tmux + ttyd)
      - uses: docker/build-push-action@v5
        with:
          context: .
          file: docker/${{ matrix.service }}-full.Dockerfile
          platforms: linux/amd64,linux/arm64
          push: true
          tags: |
            ghcr.io/nunchi/${{ matrix.service }}:${{ github.ref_name }}-full
            ghcr.io/nunchi/${{ matrix.service }}:latest-full
          build-args: |
            BINARY=${{ matrix.binary }}
```

---

## docker-compose.yml

For running the full stack locally. Services discover each other by Docker Compose service
names (e.g., roko-cli reaches the gateway proxy at `http://gateway:4000`).

```yaml
# docker/docker-compose.yml
#
# Full Roko stack for local development.
#
# Usage:
#   cd docker
#   cp ../.env .env              # Copy your API keys
#   docker compose up            # Start core services
#   docker compose --profile full up  # + console web terminal

services:
  # ── Roko CLI (orchestrator) ──────────────────────────────────────
  roko:
    image: ghcr.io/nunchi/roko-cli:latest
    ports:
      - "8080:8080"                  # Roko server API
    volumes:
      - roko-data:/data              # Persistent state (plans, state, indexes)
    environment:
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY:?Set ANTHROPIC_API_KEY in .env}
      RUST_LOG: ${RUST_LOG:-info}
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:8080/health"]
      interval: 10s
      timeout: 5s
      retries: 3

  # ── Roko Serve (HTTP API) ────────────────────────────────────────
  roko-serve:
    image: ghcr.io/nunchi/roko-serve:latest
    ports:
      - "8081:8080"                  # HTTP API
    environment:
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY:?Set ANTHROPIC_API_KEY in .env}
      RUST_LOG: ${RUST_LOG:-info}
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:8080/health"]
      interval: 10s
      timeout: 5s
      retries: 3

  # ── mirage-rs (EVM fork) ─────────────────────────────────────────
  mirage:
    image: ghcr.io/nunchi/mirage-rs:latest
    ports:
      - "8545:8545"                  # JSON-RPC (Anvil-compatible)
    volumes:
      - mirage-data:/data
    environment:
      MIRAGE_RPC_URL: ${MIRAGE_RPC_URL:-}
    profiles:
      - full

  # ── Console (web terminal) ──────────────────────────────────────
  console:
    image: ghcr.io/nunchi/roko-console:latest
    ports:
      - "8443:3000"                  # Web UI
    environment:
      CONSOLE_AUTH_TOKEN: ${CONSOLE_AUTH_TOKEN:-}
      SERVICES: "roko:roko:7681"
    profiles:
      - full

volumes:
  roko-data:
  mirage-data:
```

### Usage

```bash
cd docker
cp ../.env .env

# Core stack (roko + serve)
docker compose up

# Full stack (everything including web terminal and mirage-rs)
docker compose --profile full up

# Just roko (for local development)
docker compose up roko

# Rebuild after code changes
docker compose build roko
docker compose up roko
```

---

## .dockerignore

Every Docker build context must include a `.dockerignore` to exclude build artifacts and
large directories:

```
target/
.git/
.roko/
*.log
node_modules/
docs/
tmp/
```

Without this, the Docker context can include hundreds of MB of build artifacts, causing slow
builds or outright failures due to context size limits.
