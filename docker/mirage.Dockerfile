# syntax=docker/dockerfile:1.7
#
# mirage-rs container image (§42.2).
#
# Multi-stage build:
#   builder : rust:1.88-slim-bookworm — compiles `mirage-rs` with the `chain` feature
#             plus the standalone `agent-relay` binary
#   runtime : debian:bookworm-slim — minimal, non-root runtime with a small
#             launcher script so mirage can front a loopback relay on one origin
#
# The `mirage-rs` crate's binary is named `mirage-rs` (see apps/mirage-rs/Cargo.toml).
# Build context is expected to be the `roko/` workspace root.

ARG BUILDPLATFORM=linux/amd64
ARG TARGETPLATFORM=linux/amd64

FROM --platform=$BUILDPLATFORM rust:1.88-slim-bookworm AS builder
WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY . .

# Build mirage-rs with the `binary` + `chain` features so the resulting image
# exposes the chain_* JSON-RPC methods and --enable-hdc/knowledge/stigmergy flags.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p mirage-rs --bin mirage-rs --features "binary,chain" \
    && cargo build --release -p agent-relay --bin agent-relay \
    && cp target/release/mirage-rs /mirage-rs \
    && cp target/release/agent-relay /agent-relay

FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/wpank/bardo"
LABEL org.opencontainers.image.description="mirage-rs and agent-relay co-deployed behind a single public origin"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
LABEL org.opencontainers.image.title="mirage-rs"
LABEL org.opencontainers.image.vendor="Bardo"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /mirage-rs /usr/local/bin/mirage-rs
COPY --from=builder /agent-relay /usr/local/bin/agent-relay
COPY docker/start-mirage-with-relay.sh /usr/local/bin/start-mirage-with-relay

ENV PORT=8545
ENV ROKO_AGENT_RELAY_BIND=127.0.0.1:9011
ENV ROKO_AGENT_RELAY_URL=http://127.0.0.1:9011
ENV MIRAGE_STATE_DIR=/workspace/.roko/state
ENV MIRAGE_SNAPSHOT_INTERVAL_SECS=15
ENV RUST_LOG=info

RUN useradd --create-home --shell /bin/bash mirage \
    && mkdir -p /workspace/.roko/state \
    && chown -R mirage:mirage /workspace \
    && chmod +x /usr/local/bin/start-mirage-with-relay

USER mirage
WORKDIR /workspace
VOLUME ["/workspace/.roko"]

# Default single-origin ingress port. Relay stays on loopback only.
EXPOSE 8545

ENTRYPOINT ["/usr/local/bin/start-mirage-with-relay"]
CMD []
