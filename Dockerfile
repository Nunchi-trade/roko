FROM rust:1.91-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p roko-cli
