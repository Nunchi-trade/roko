# Task 055: Fix Dockerfile Multi-Stage Build and CORS Config

```toml
id = 55
title = "Proper multi-stage Dockerfile without Rust toolchain in runtime, CORS from config"
track = "infrastructure"
wave = "wave-2"
priority = "medium"
blocked_by = []
touches = [
    "Dockerfile",
    "roko.toml",
    "docker-compose.dev.yml",
    "crates/roko-serve/src/lib.rs",
    "crates/roko-serve/src/routes/middleware.rs",
    "crates/roko-core/src/config/serve.rs",
    "crates/roko-core/src/config/schema.rs",
]
exclusive_files = [
    "Dockerfile",
]
estimated_minutes = 120
```

## Context

The audit (S5) identified multiple Docker/deployment issues:
1. **Entire Rust toolchain in runtime image**: Dockerfile copies the toolchain into
   the runtime stage, adding ~1-2GB to the image.
2. **CORS hardcoded**: `CorsLayer` allows all origins. In prod, this should be
   restricted.
3. **Bind-mount mismatch**: `docker-compose.dev.yml` mounts `./target/release/roko`
   but dev builds are debug → binary is stale.

The health/readiness endpoints and `roko up` lifecycle were already fixed (batch 37).
This task addresses the remaining deployment concerns.

## Background

Read:
- `Dockerfile` — current build
- `roko.toml` — config currently copied into the runtime image
- `docker-compose.dev.yml` — dev compose
- `crates/roko-core/src/config/serve.rs` — flat `[server]` CORS config fields
- `crates/roko-serve/src/routes/middleware.rs` — active CORS layer
- `crates/roko-serve/src/lib.rs` — server binding and any stale helpers

Grep for CORS:
```bash
grep -rn 'CorsLayer\|cors\|allowed_origins' crates/roko-serve/src/ --include='*.rs' | grep -v target/
```

## Current Code Reality - 2026-05-05

- The Docker runtime stage currently copies `/usr/local/cargo` and `/usr/local/rustup`
  from the builder and installs compiler/build packages (`gcc`, `libc6-dev`,
  `pkg-config`, `libssl-dev`). The runtime image should not contain any Rust toolchain
  or compiler packages.
- The current image does more than run `roko serve`: it copies `roko`, `mirage-rs`, and
  `agent-relay`, uses `docker/start-railway.sh`, exposes the Railway config, and has a
  `/health` healthcheck. Keep those runtime behaviors unless you prove they are unused.
- `docker/start-railway.sh` requires `bash`, `curl`, `git`, and `gosu` when running as
  root. Keep runtime packages that the script actually calls; remove the toolchain and
  compiler packages.
- There is no `docker/railway.roko.toml` in this branch. The Dockerfile currently copies
  root `roko.toml` to `/workspace/roko.toml`. Create a Railway-specific config only if
  you also update the Dockerfile/start script and document why root `roko.toml` is wrong.
- CORS is already implemented in `crates/roko-serve/src/routes/middleware.rs` using flat
  `[server]` fields: `cors_origins: Vec<String>` and `unsafe_public_cors: bool` from
  `crates/roko-core/src/config/serve.rs`. Default empty `cors_origins` is local-only,
  not permissive. Do not add a new `[server.cors]` table.
- `crates/roko-serve/src/lib.rs` still contains a stale private `build_cors_layer`
  helper that is not used by the active router and falls back to permissive CORS. Remove
  it or make it delegate to the active middleware so future changes cannot accidentally
  reintroduce wildcard CORS.
- `docker-compose.dev.yml` currently says and mounts release binaries. This task should
  make dev compose explicitly consistent with the chosen dev build mode; for this task,
  prefer debug mounts for `target/debug/{roko,mirage-rs,agent-relay}`.

## What to Change

### 1. Fix Dockerfile Multi-Stage

The Dockerfile should have three clean stages:

```dockerfile
# Stage 1: Frontend
FROM node:22-alpine AS frontend
WORKDIR /app/demo/demo-app
COPY demo/demo-app/package*.json .
RUN npm ci --production=false
COPY demo/demo-app/ .
RUN npm run build

# Stage 2: Rust build
FROM rust:1.91-slim-bookworm AS backend
WORKDIR /app
COPY . .
COPY --from=frontend /app/demo/demo-app/dist demo/demo-app/dist
RUN cargo build --release -p roko-cli --bin roko \
    && cargo build --release -p mirage-rs --bin mirage-rs --features "binary,roko" \
    && cargo build --release -p agent-relay --bin agent-relay \
    && strip target/release/roko target/release/mirage-rs target/release/agent-relay

# Stage 3: Runtime (NO toolchain)
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash ca-certificates coreutils curl git gosu libssl3 tini && \
    rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/roko /usr/local/bin/roko
COPY --from=backend /app/target/release/mirage-rs /usr/local/bin/mirage-rs
COPY --from=backend /app/target/release/agent-relay /usr/local/bin/agent-relay
COPY docker/start-railway.sh /usr/local/bin/start-railway
COPY roko.toml /workspace/roko.toml
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/start-railway"]
EXPOSE 6677
```

Key points:
- Treat the snippet as the stage/package shape, not a full-file replacement. Preserve the
  existing `WORKDIR`, `ENV` values, user/state-directory setup, and healthcheck unless you
  have a concrete reason to change them.
- **No Rust toolchain in runtime stage** — only compiled binaries plus required scripts/config
- **tini as PID 1** — proper signal forwarding
- **Minimal runtime deps** — keep only packages needed by the binaries, the healthcheck,
  and `start-railway.sh`; currently that includes `bash`, `curl`, `git`, and `gosu`
- **Sidecars preserved** — do not drop `mirage-rs` or `agent-relay` while the Railway
  entrypoint still manages them

### 2. CORS from Config

Keep the existing flat config in `crates/roko-core/src/config/serve.rs`:

```toml
[server]
# Empty cors_origins = local browser origins only.
cors_origins = []
unsafe_public_cors = false
```

Active behavior in `routes/middleware.rs` must remain:
- non-empty `cors_origins`: allow only exact configured origins
- `unsafe_public_cors = true`: allow wildcard/permissive CORS and emit a warning
- default empty config: allow localhost/127.0.0.1 origins only, reject non-local origins

Update `roko.toml` comments or deployment docs to show how to set production origins on
the existing `[server]` table. Do not document empty origins as "allow all".

### 3. Fix docker-compose.dev.yml

Change bind mounts from release to debug for all mounted binaries:

```yaml
./target/debug/roko:/usr/local/bin/roko:ro
./target/debug/mirage-rs:/usr/local/bin/mirage-rs:ro
./target/debug/agent-relay:/usr/local/bin/agent-relay:ro
```

Update adjacent comments to say `cargo build` rather than `cargo build --release`. Do not
leave comments and mount paths disagreeing.

### 4. Add/confirm tests

- `crates/roko-serve/src/routes/middleware.rs`: keep or add tests for exact-origin
  allow, method/header restrictions, default local-origin allow, default non-local
  reject, and unsafe-public wildcard behavior.
- `crates/roko-serve/src/lib.rs`: if the stale helper is retained, test that it delegates
  to the same local-only default. Prefer removing it if no caller exists.

## What NOT to Do

- Don't add Docker health checks in this task (already done via /health route).
- Don't add Railway-specific environment variables to the Dockerfile.
- Don't change `start-railway.sh` sidecar management (separate concern).
- Don't change the SPA embedding (`rust-embed`) — that is a deliberate design choice.
- Don't add process supervision for sidecars (separate task).
- Don't add a nested `[server.cors]` schema; it conflicts with the current flat config.
- Don't make empty `cors_origins` permissive. Wildcard CORS requires
  `unsafe_public_cors = true`.

## Wire Target

```bash
# Build Docker image
docker build -t roko-test .
# Verify image size is not toolchain-sized
docker images roko-test --format '{{.Size}}'
# Verify no Rust toolchain leaked into runtime
docker run --rm --entrypoint sh roko-test -lc '! command -v rustc && ! command -v cargo && ! command -v rustup'
# Verify runtime works
docker run -d --name roko-test -p 6677:6677 roko-test
sleep 3
curl http://localhost:6677/health
docker stop roko-test && docker rm roko-test
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] Docker image builds successfully
- [ ] Runtime image does not contain `rustc`, `cargo`, or `rustup`
  (`docker run --rm --entrypoint sh roko-test -lc '! command -v rustc'` succeeds)
- [ ] CORS respects `[server] cors_origins`/`unsafe_public_cors`
  (empty = local-only, explicit list = restricted, unsafe flag = wildcard)
- [ ] `docker-compose.dev.yml` mounts debug binaries (or builds from source) consistently

## Status Log

| Time | Agent | Action |
|------|-------|--------|
