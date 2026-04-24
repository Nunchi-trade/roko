#
# Roko worker container image.
#
# Runs `roko worker` inside a deployed container. Reads its agent template
# from the ROKO_TEMPLATE_JSON env var (base64-encoded JSON), starts a thin
# HTTP server, and executes tasks via the universal loop.
#
# Build context is the `roko/` workspace root.

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

RUN cargo build --release --bin roko && \
    cp target/release/roko /roko

# --- Runtime stage ---
FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/nunchi/roko"
LABEL org.opencontainers.image.description="Roko worker agent container"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
LABEL org.opencontainers.image.title="roko-worker"
LABEL org.opencontainers.image.vendor="Roko"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        nodejs \
        npm \
        python3 \
        python3-pip \
    && npm install -g @anthropic-ai/claude-code \
    && pip3 install --no-cache-dir --break-system-packages openai \
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
