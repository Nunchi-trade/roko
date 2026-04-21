# ---- Builder stage ----
FROM rust:1.91-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p roko-cli

# ---- Runtime stage ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        git \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/roko /usr/local/bin/roko

# Default state directory for agent persistence.
VOLUME ["/workspace/.roko"]
WORKDIR /workspace

ENTRYPOINT ["/usr/local/bin/roko"]
CMD ["serve", "--bind", "0.0.0.0", "--port", "6677"]
EXPOSE 6677
