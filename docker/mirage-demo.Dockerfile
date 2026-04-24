#
# mirage-rs DEMO container image — includes the static dashboard.
#
# This is the Railway deployment Dockerfile. It bundles the dashboard static
# files so the UI is available at /dashboard on the deployed service.
#
# For production (no dashboard), use mirage.Dockerfile instead.
#
# Build context is expected to be the `roko/` workspace root.

ARG BUILDPLATFORM=linux/amd64

FROM --platform=$BUILDPLATFORM rust:1.91-slim-bookworm AS builder
WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build --release -p mirage-rs --bin mirage-rs --features "binary,roko" \
    && cargo build --release -p agent-relay --bin agent-relay \
    && cp target/release/mirage-rs /mirage-rs \
    && cp target/release/agent-relay /agent-relay

FROM --platform=$BUILDPLATFORM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/nunchi/roko"
LABEL org.opencontainers.image.description="mirage-rs demo with dashboard, roko APIs, and same-origin agent-relay"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
LABEL org.opencontainers.image.title="mirage-rs-demo"
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
COPY apps/mirage-rs/static/ /usr/local/share/mirage-dashboard/

ENV PORT=8545
ENV MIRAGE_DASHBOARD_DIR=/usr/local/share/mirage-dashboard
ENV ROKO_AGENT_RELAY_BIND=127.0.0.1:9011
ENV ROKO_AGENT_RELAY_URL=http://127.0.0.1:9011
ENV MIRAGE_STATE_DIR=/workspace/.roko/state
ENV MIRAGE_SNAPSHOT_INTERVAL_SECS=15
ENV MIRAGE_BLOCK_INTERVAL_MS=1000
ENV RUST_LOG=info

RUN useradd --create-home --shell /bin/bash mirage \
    && mkdir -p /workspace/.roko/state \
    && chown -R mirage:mirage /workspace \
    && chmod +x /usr/local/bin/start-mirage-with-relay \
    && chmod +x /usr/local/bin/entrypoint.sh

WORKDIR /workspace
# Default single-origin ingress port. Relay stays on loopback only.
EXPOSE 8545

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["/usr/local/bin/start-mirage-with-relay", "--chain-id", "1"]
