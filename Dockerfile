# Roko Railway image.
#
# One public Railway service:
#   - roko serve listens on 0.0.0.0:$PORT
#   - mirage-rs listens on 127.0.0.1:8545
#   - agent-relay listens on 127.0.0.1:9011
#
# The sidecars are required build artifacts. A deploy must fail if they do not
# build, instead of silently shipping a half-functional control plane.

# ---- Frontend (Vite) -------------------------------------------------------
FROM node:22-bookworm-slim AS frontend
WORKDIR /app/demo/demo-app
COPY demo/demo-app/package.json demo/demo-app/package-lock.json* ./
RUN npm ci --prefer-offline
COPY demo/demo-app/ ./
RUN npm run build

# ---- Rust binaries --------------------------------------------------------
FROM rust:1.91-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
        libssl-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add clippy rustfmt

COPY . .
COPY --from=frontend /app/demo/demo-app/dist ./demo/demo-app/dist

RUN cargo build --release -p roko-cli --bin roko \
    && cargo build --release -p mirage-rs --bin mirage-rs --features "binary,roko" \
    && cargo build --release -p agent-relay --bin agent-relay \
    && strip target/release/roko target/release/mirage-rs target/release/agent-relay \
    && cp target/release/roko /tmp/roko \
    && cp target/release/mirage-rs /tmp/mirage-rs \
    && cp target/release/agent-relay /tmp/agent-relay

# ---- Runtime ---------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        coreutils \
        curl \
        gcc \
        git \
        gosu \
        libc6-dev \
        libssl-dev \
        libssl3 \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain: some server-launched coding/demo tasks need cargo, clippy, and
# rustfmt at runtime.
COPY --from=builder /usr/local/cargo /usr/local/cargo
COPY --from=builder /usr/local/rustup /usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH="/usr/local/cargo/bin:$PATH"

# Foundry cast is used by chain/demo helpers. Keep this explicit so failures
# happen at image build time, not during a Railway deployment.
RUN curl -L https://foundry.paradigm.xyz | bash \
    && /root/.foundry/bin/foundryup \
    && cp /root/.foundry/bin/cast /usr/local/bin/cast \
    && rm -rf /root/.foundry

COPY --from=builder /tmp/roko /usr/local/bin/roko
COPY --from=builder /tmp/mirage-rs /usr/local/bin/mirage-rs
COPY --from=builder /tmp/agent-relay /usr/local/bin/agent-relay
COPY docker/start-railway.sh /usr/local/bin/start-railway
COPY --from=builder /app/roko.toml /workspace/roko.toml

# Docker/Railway containers intentionally bind the public HTTP server to
# 0.0.0.0. Use API providers by default because the Claude CLI is not installed
# in this image.
RUN sed -i 's/acknowledge_public_risk = false/acknowledge_public_risk = true/' /workspace/roko.toml \
    && sed -i '/^\[providers\.anthropic\]$/,/^\[/ { s/kind = "claude_cli"/kind = "anthropic_api"/; /^command = /d; }' /workspace/roko.toml \
    && sed -i '/^\[providers\.claude_cli\]$/,/^\[/ { s/kind = "claude_cli"/kind = "anthropic_api"/; }' /workspace/roko.toml \
    && sed -i '/^\[agent\]$/,/^\[/ { s/^command = "claude"/# command = "claude"/; }' /workspace/roko.toml \
    && chmod +x /usr/local/bin/start-railway \
    && useradd --create-home --shell /bin/bash --uid 1000 roko \
    && mkdir -p \
        /workspace/.roko/dreams \
        /workspace/.roko/learn \
        /workspace/.roko/neuro \
        /workspace/.roko/state \
    && chown -R roko:roko /workspace

WORKDIR /workspace

ENV RUST_LOG=info
ENV SHELL=/bin/bash
ENV ROKO_BIND=0.0.0.0
ENV ROKO_PORT=6677
ENV MIRAGE_HOST=127.0.0.1
ENV MIRAGE_PORT=8545
ENV MIRAGE_CHAIN_ID=31337
ENV MIRAGE_BLOCK_INTERVAL_MS=1000
ENV MIRAGE_SNAPSHOT_INTERVAL_SECS=15
ENV ROKO_AGENT_RELAY_BIND=127.0.0.1:9011
ENV ROKO_AGENT_RELAY_URL=http://127.0.0.1:9011
ENV ROKO_MIRAGE_URL=http://127.0.0.1:8545
ENV MIRAGE_RPC_URL=http://127.0.0.1:8545

EXPOSE 6677

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD public_port="${PORT:-${ROKO_PORT:-6677}}" \
    && curl -fsS "http://127.0.0.1:${public_port}/health" >/dev/null \
    && curl -fsS "http://127.0.0.1:${MIRAGE_PORT:-8545}/health" >/dev/null \
    && curl -fsS "http://${ROKO_AGENT_RELAY_BIND:-127.0.0.1:9011}/relay/health" >/dev/null

ENTRYPOINT ["/usr/local/bin/start-railway"]
