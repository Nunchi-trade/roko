# syntax=docker/dockerfile:1.7
#
# Roko worker container image.
#
# Runs `roko worker` inside a deployed container. Reads its agent template
# from the ROKO_TEMPLATE_JSON env var (base64-encoded JSON), starts a thin
# HTTP server, and executes tasks via the universal loop.
#
# Build context is the `roko/` workspace root.

ARG BUILDPLATFORM
ARG TARGETPLATFORM

FROM --platform=$BUILDPLATFORM rust:1.85-bookworm-slim AS builder
WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --bin roko && \
    cp target/release/roko /roko

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        nodejs \
        npm \
    && npm install -g @anthropic-ai/claude-code \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /roko /usr/local/bin/roko

# Environment variables (set at deploy time)
ENV ROKO_TEMPLATE_JSON=""
ENV ROKO_CONTROL_PLANE_URL=""
ENV ROKO_DEPLOYMENT_ID=""
ENV ANTHROPIC_API_KEY=""
ENV PORT=8080
ENV RUST_LOG=info

EXPOSE 8080

RUN useradd --create-home --shell /bin/bash roko
USER roko
WORKDIR /home/roko

CMD ["roko", "worker"]
