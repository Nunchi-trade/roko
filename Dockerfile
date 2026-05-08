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
# Compiled binaries + Rust toolchain (for gate pipeline) + Claude CLI (for agent dispatch).
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        coreutils \
        curl \
        git \
        gosu \
        libssl3 \
        nodejs \
        npm \
        tini \
    && npm install -g @anthropic-ai/claude-code \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/* /root/.npm

COPY --from=builder /tmp/roko /usr/local/bin/roko
COPY --from=builder /tmp/mirage-rs /usr/local/bin/mirage-rs
COPY --from=builder /tmp/agent-relay /usr/local/bin/agent-relay

# Rust toolchain (needed for gate pipeline: cargo check/clippy/test)
COPY --from=builder /usr/local/rustup /usr/local/rustup
COPY --from=builder /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH="/usr/local/cargo/bin:${PATH}"

COPY docker/start-railway.sh /usr/local/bin/start-railway
# Railway config is injected via ROKO_* env vars (see: roko config export --env railway).
# A default roko.toml is generated at startup by start-railway if absent.
COPY roko.toml /workspace/roko.toml

RUN chmod +x /usr/local/bin/start-railway \
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

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/start-railway"]
