#
# Roko CLI container image (§42.1).
#
# Multi-stage build:
#   builder : rust:1.91-slim-bookworm — compiles the workspace's `roko` binary
#   runtime : distroless/cc-debian12:nonroot — minimal, non-root, no shell
#
# The `roko-cli` crate produces a binary named `roko` (see crates/roko-cli/Cargo.toml).
# Build context is expected to be the `roko/` workspace root.

ARG BUILDPLATFORM=linux/amd64

FROM --platform=$BUILDPLATFORM rust:1.91-slim-bookworm AS builder
WORKDIR /src

# System deps commonly required by crates in this workspace (openssl-sys, etc).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY . .

# Build the `roko` binary from the roko-cli crate.
RUN cargo build --release --bin roko && \
    cp target/release/roko /roko

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

LABEL org.opencontainers.image.source="https://github.com/nunchi/roko"
LABEL org.opencontainers.image.description="Roko orchestration CLI"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
LABEL org.opencontainers.image.title="roko"
LABEL org.opencontainers.image.vendor="Roko"

COPY --from=builder /roko /usr/local/bin/roko

# Environment variables for Railway/cloud deployment
ENV PORT=3000
ENV RUST_LOG=info

RUN useradd --create-home --shell /bin/bash roko

# Persist the daemon's `.roko/` tree across deploys.
RUN mkdir -p /workspace/.roko/learn \
    /workspace/.roko/state \
    /workspace/.roko/neuro \
    /workspace/.roko/dreams \
    && chown -R roko:roko /workspace/.roko

USER roko
WORKDIR /workspace

ENTRYPOINT ["/usr/local/bin/roko"]
CMD ["serve"]
