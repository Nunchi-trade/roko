#
# mirage-rs container image (§42.2).
#
# Multi-stage build:
#   builder : rust:1.91-slim-bookworm — compiles `mirage-rs` with the `roko`
#             feature set (REST API + WebSocket + chain surface) plus the
#             standalone `agent-relay` binary
#   runtime : debian:bookworm-slim — minimal, non-root runtime with a small
#             launcher script so mirage can front a loopback relay on one origin
#
# The `mirage-rs` crate's binary is named `mirage-rs` (see apps/mirage-rs/Cargo.toml).
# Build context is expected to be the `roko/` workspace root.

ARG BUILDPLATFORM

FROM --platform=$BUILDPLATFORM rust:1.91-slim-bookworm AS builder
WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY . .

# Build mirage-rs with the `binary` + `roko` features so the resulting image
# keeps the JSON-RPC chain surface while also mounting the full REST + WebSocket
# API from main. Build `agent-relay` alongside it for same-origin `/relay/*`.
RUN cargo build --release -p mirage-rs --bin mirage-rs --features "binary,roko" \
    && cargo build --release -p agent-relay --bin agent-relay \
    && cp target/release/mirage-rs /mirage-rs \
    && cp target/release/agent-relay /agent-relay

FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/nunchi/roko"
LABEL org.opencontainers.image.description="mirage-rs with full roko APIs plus same-origin agent-relay reachability"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
LABEL org.opencontainers.image.title="mirage-rs"
LABEL org.opencontainers.image.vendor="Roko"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        gosu \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /mirage-rs /usr/local/bin/mirage-rs
COPY --from=builder /agent-relay /usr/local/bin/agent-relay
COPY docker/start-mirage-with-relay.sh /usr/local/bin/start-mirage-with-relay
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh

ENV PORT=8545
ENV ROKO_AGENT_RELAY_BIND=127.0.0.1:9011
ENV ROKO_AGENT_RELAY_URL=http://127.0.0.1:9011
ENV MIRAGE_STATE_DIR=/workspace/.roko/state
ENV MIRAGE_SNAPSHOT_INTERVAL_SECS=15
ENV RUST_LOG=info

RUN useradd --create-home --shell /bin/bash mirage \
    && mkdir -p /workspace/.roko/state \
    && chown -R mirage:mirage /workspace \
    && chmod +x /usr/local/bin/start-mirage-with-relay \
    && chmod +x /usr/local/bin/entrypoint.sh

WORKDIR /workspace
# Default single-origin ingress port. Relay stays on loopback only.
EXPOSE 8545

# entrypoint.sh runs as root, fixes volume permissions, then drops to mirage via gosu.
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["/usr/local/bin/start-mirage-with-relay"]
