#
# roko-demo container image — bundles the roko-demo orchestrator binary, the
# demo/ config tree, and the contracts/ Foundry project (pre-built bytecode).
# Uses alloy for deployment, so no forge is required at runtime.
#
# Build context: `roko/` workspace root.

ARG BUILDPLATFORM
ARG TARGETPLATFORM

# --- Stage 1: compile contracts via forge ----------------------------------
FROM ghcr.io/foundry-rs/foundry:latest AS contracts
WORKDIR /contracts
# Foundry image drops privileges by default — revert to root to fetch libs.
USER root
COPY contracts/foundry.toml contracts/remappings.txt contracts/.gitignore ./
COPY contracts/src ./src
COPY contracts/script ./script
COPY contracts/test ./test
COPY contracts/lib ./lib
RUN forge build --silent

# --- Stage 2: build the roko-demo binary -----------------------------------
FROM --platform=$BUILDPLATFORM rust:1.91-bookworm-slim AS builder
WORKDIR /src
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release --bin roko-demo && \
    cp target/release/roko-demo /roko-demo

# --- Stage 3: runtime ------------------------------------------------------
FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/nunchi/roko"
LABEL org.opencontainers.image.description="Roko demo-environment orchestrator"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
LABEL org.opencontainers.image.title="roko-demo"
LABEL org.opencontainers.image.vendor="Roko"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 roko

# Binary + prebuilt contract artifacts + demo config.
COPY --from=builder /roko-demo /usr/local/bin/roko-demo
COPY --from=contracts /contracts/out /app/contracts/out
COPY contracts/foundry.toml /app/contracts/foundry.toml
COPY demo /app/demo

WORKDIR /app
USER roko

ENTRYPOINT ["/usr/local/bin/roko-demo", "--demo-dir", "/app/demo", "--runtime-dir", "/app/demo/.runtime"]
CMD ["list"]
