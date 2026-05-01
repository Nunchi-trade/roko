# Roko control plane + mirage-rs + agent-relay + demo-app SPA (embed at build time).

# ---- Frontend (Vite) ----
FROM node:22-bookworm-slim AS frontend
WORKDIR /app/demo/demo-app
COPY demo/demo-app/package.json demo/demo-app/package-lock.json* ./
RUN npm ci --prefer-offline
COPY demo/demo-app/ ./
RUN npm run build

# ---- Rust binary ----
FROM rust:1.91-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev git \
    && rm -rf /var/lib/apt/lists/*
RUN rustup component add clippy
WORKDIR /app
COPY . .
COPY --from=frontend /app/demo/demo-app/dist ./demo/demo-app/dist
# BuildKit cache mounts: cargo registry + target dir persist across builds.
# Only changed crates recompile instead of full rebuild every time.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p roko-cli \
    && strip target/release/roko \
    && cp target/release/roko /tmp/roko

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p mirage-rs --features "binary,roko" \
    && strip target/release/mirage-rs \
    && cp target/release/mirage-rs /tmp/mirage-rs

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p agent-relay \
    && strip target/release/agent-relay \
    && cp target/release/agent-relay /tmp/agent-relay

# ---- Runtime ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates libssl3 libssl-dev pkg-config \
       git bash gcc libc6-dev curl coreutils \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain — demo gate commands need cargo test / clippy
COPY --from=builder /usr/local/cargo /usr/local/cargo
COPY --from=builder /usr/local/rustup /usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH="/usr/local/cargo/bin:$PATH"

# Foundry (cast) — needed for on-chain interactions
RUN curl -L https://foundry.paradigm.xyz | bash \
    && /root/.foundry/bin/foundryup \
    && cp /root/.foundry/bin/cast /usr/local/bin/cast \
    && rm -rf /root/.foundry

COPY --from=builder /tmp/roko /usr/local/bin/roko
COPY --from=builder /tmp/mirage-rs /usr/local/bin/mirage-rs
COPY --from=builder /tmp/agent-relay /usr/local/bin/agent-relay
COPY --from=builder /app/roko.toml /workspace/roko.toml
# Docker containers always bind 0.0.0.0 — acknowledge public risk in config
RUN sed -i 's/acknowledge_public_risk = false/acknowledge_public_risk = true/' /workspace/roko.toml \
    # Providers: use anthropic API (not claude_cli which needs the claude binary)
    && sed -i '/^\[providers\.anthropic\]$/,/^\[/ { s/kind = "claude_cli"/kind = "anthropic_api"/; /^command = /d; }' /workspace/roko.toml \
    && sed -i '/^\[providers\.claude_cli\]$/,/^\[/ { s/kind = "claude_cli"/kind = "anthropic_api"/; }' /workspace/roko.toml \
    # Agent: use anthropic API backend, not CLI
    && sed -i '/^\[agent\]$/,/^\[/ { s/^command = "claude"/# command = "claude"/; }' /workspace/roko.toml

# Minimal terminal prompt
RUN echo 'export PS1="> "' >> /root/.bashrc

# Start script: launches agent-relay, mirage-rs, then roko serve
RUN cat <<'SCRIPT' > /usr/local/bin/start.sh
#!/usr/bin/env bash
set -e

cleanup() {
  echo "Shutting down..."
  kill $RELAY_PID $MIRAGE_PID 2>/dev/null || true
  wait $RELAY_PID $MIRAGE_PID 2>/dev/null || true
  exit 0
}
trap cleanup SIGTERM SIGINT

# Start agent-relay
agent-relay &
RELAY_PID=$!

# Build mirage-rs args
MIRAGE_ARGS="--bind 0.0.0.0 --port 8545"
MIRAGE_ARGS="$MIRAGE_ARGS --block-interval-ms ${MIRAGE_BLOCK_INTERVAL_MS:-50}"
MIRAGE_ARGS="$MIRAGE_ARGS --chain-id ${MIRAGE_CHAIN_ID:-88888}"
MIRAGE_ARGS="$MIRAGE_ARGS --enable-hdc --enable-knowledge --enable-stigmergy"
[ -n "$ETH_RPC_URL" ] && MIRAGE_ARGS="$MIRAGE_ARGS --rpc-url $ETH_RPC_URL"

# Start mirage-rs
mirage-rs $MIRAGE_ARGS &
MIRAGE_PID=$!

# Give services a moment to start
sleep 1

# Start roko serve (foreground, PID 1 semantics)
exec roko serve --bind 0.0.0.0 --port 6677
SCRIPT
RUN chmod +x /usr/local/bin/start.sh

VOLUME ["/workspace/.roko"]
WORKDIR /workspace

ENV RUST_LOG=info
ENV SHELL=/bin/bash
ENV MIRAGE_RPC_URL=http://127.0.0.1:8545
ENV MIRAGE_BLOCK_INTERVAL_MS=50
ENV MIRAGE_CHAIN_ID=88888
ENV ROKO_AGENT_RELAY_BIND=127.0.0.1:9011
ENV ROKO_AGENT_RELAY_URL=http://127.0.0.1:9011
EXPOSE 6677 8545

ENTRYPOINT ["/usr/local/bin/start.sh"]
CMD []
