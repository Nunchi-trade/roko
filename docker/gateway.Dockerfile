# syntax=docker/dockerfile:1.7
#
# roko-gateway container image (§42.3).
#
# TODO(roko-gateway): the `roko-gateway` crate does not yet exist in this
# workspace. Until it is scaffolded, this image builds the `roko` binary from
# `roko-cli` as a stand-in so the compose topology, GHCR publish matrix, and
# multi-arch tooling can be exercised end-to-end. When `roko-gateway` lands,
# replace the `--bin roko` / `/roko` / ENTRYPOINT lines below with
# `--bin roko-gateway` / `/roko-gateway` / `/usr/local/bin/roko-gateway`.
#
# Multi-stage build:
#   builder : rust:1.91-bookworm-slim
#   runtime : distroless/cc-debian12:nonroot — minimal, non-root, no shell
#
# Build context is expected to be the `roko/` workspace root.

ARG BUILDPLATFORM
ARG TARGETPLATFORM

FROM --platform=$BUILDPLATFORM rust:1.91-bookworm-slim AS builder
WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY . .

# TODO(roko-gateway): replace --bin roko with --bin roko-gateway once the crate exists.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --bin roko && \
    cp target/release/roko /roko-gateway

FROM gcr.io/distroless/cc-debian12:nonroot
LABEL org.opencontainers.image.source="https://github.com/nunchi/roko"
LABEL org.opencontainers.image.description="Roko gateway (placeholder: currently shipping roko-cli until roko-gateway crate lands)"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
LABEL org.opencontainers.image.title="roko-gateway"
LABEL org.opencontainers.image.vendor="Roko"

COPY --from=builder --chown=nonroot:nonroot /roko-gateway /usr/local/bin/roko-gateway

USER nonroot
WORKDIR /workspace

# Placeholder port; finalise once the real gateway crate defines its listen port.
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/roko-gateway"]
CMD ["--help"]
