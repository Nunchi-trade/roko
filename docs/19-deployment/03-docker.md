# Docker Deployment

> Roko's container shape is one profile in a five-shape deployment model: laptop-local,
> single-server, container, clustered, and edge. The same Rust binary powers them all;
> Docker is the packaging layer for the container shape, not a fork of the runtime.
> See also `../../tmp/refinements/24-deployment-ux.md`.

> **Implementation**: Specified

---

## Deployment Shape Model

Docker is the runtime packaging target for the container shape, which in turn supports two
common deployment profiles:

| Shape | Typical profile | Role of Docker |
|---|---|---|
| laptop-local | `laptop` | Local dev stack and reproducible test harness |
| single-server | `single-server` | One shared service on a box or VM |
| container | `container` | Primary artifact: image, volume, probes, env wiring |
| clustered | `clustered` | Repeatable node image behind an orchestrator |
| edge | `edge` | Usually not Docker; edge uses separate packaging targets |

The container story is intentionally boring: one image, one config layer, one volume layout,
and the same observability contract everywhere.

For shared deployment vocabulary, see [the glossary](../00-architecture/01-naming-and-glossary.md).

---

## Image Registry and Naming

Images are published to GitHub Container Registry (`ghcr.io`). Naming keeps the service and
version visible without encoding the deployment shape into the codebase:

```text
ghcr.io/nunchi/roko-cli:latest
ghcr.io/nunchi/roko-cli:0.3.0
ghcr.io/nunchi/roko-cli:0.3.0-full
ghcr.io/nunchi/roko-serve:latest
ghcr.io/nunchi/mirage-rs:latest
```

The shape is chosen by config:

```toml
profile = "container"

[profile.container]
substrate = { kind = "sqlite", path = "/var/lib/roko/state.db" }
bus = { kind = "in-memory" }
listen = "0.0.0.0:6677"
```

The binary stays the same; the profile controls defaults for storage, transport, and
listening behavior.

---

## Two Image Variants

### Slim Images

Slim images contain the statically linked binary and the minimum runtime dependencies. They
are the default for production containers and clustered nodes.

| Base | Size | Shell | Use case |
|---|---|---|---|
| `gcr.io/distroless/cc-debian12` | small | No | Dynamically linked slim images |
| `cgr.dev/chainguard/static` | smaller | No | Statically linked binaries |
| `scratch` | smallest | No | Absolute minimum for musl builds |

Use slim images for laptop-local service emulation, single-server production, and clustered
nodes where operator interaction happens through logs, metrics, and the API rather than an
interactive shell.

### Full Images (with Web Terminal)

Full images add `tmux`, `ttyd`, and an entrypoint that keeps a terminal session attached to the
service. They are operator-friendly packaging artifacts for demos, debugging, and controlled
shared servers.

The important distinction is operational, not architectural: full images are a convenience
layer on top of the same binary and profile system.

---

## State and Observability

Container deployments rely on a mounted state directory, usually `/var/lib/roko`, plus a
portable archive format for moving state between shapes.

- `roko state export <archive>` captures substrate state, bus queues, and config.
- `roko state import <archive>` restores the same bundle elsewhere.
- The container volume is the working copy; the archive is the portable form.

Observability is part of the container contract:

- Structured logs go to stderr by default.
- `/metrics` exposes Prometheus-compatible metrics.
- `/healthz` and `/readyz` match orchestrator probes.
- OpenTelemetry can be enabled through standard environment variables.

For shared container hosts, multi-tenancy is enforced by the service layer, not the image:
run one tenant per volume when isolation matters, or share the image while scoping state and
auth by tenant.

---

## Directory Structure

Docker-related packaging artifacts live under `docker/` at the repo root:

```text
docker/
  roko-cli.Dockerfile
  roko-cli-full.Dockerfile
  roko-serve.Dockerfile
  roko-serve-full.Dockerfile
  mirage-rs.Dockerfile
  mirage-rs-full.Dockerfile
  console.Dockerfile
  entrypoints/
    roko-cli.sh
    roko-serve.sh
    mirage-rs.sh
  docker-compose.yml
  docker-compose.dev.yml
```

These files are packaging artifacts, not alternate code paths.

---

## Dockerfiles

### Slim Dockerfile: roko-cli

```dockerfile
FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash -d /data roko \
    && mkdir -p /data/.roko/state /data/.roko/prd \
    && chown -R roko:roko /data

COPY target/x86_64-unknown-linux-musl/release/roko-cli /usr/local/bin/roko

USER roko
WORKDIR /data
EXPOSE 8080
ENV RUST_LOG=info
ENTRYPOINT ["roko"]
CMD ["plan", "run", "--port", "8080", "--bind", "0.0.0.0"]
```

### Slim Dockerfile: roko-serve

```dockerfile
FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash roko

COPY target/x86_64-unknown-linux-musl/release/roko-serve /usr/local/bin/roko-serve

USER roko
EXPOSE 8080
ENV RUST_LOG=info
ENTRYPOINT ["roko-serve"]
CMD ["--port", "8080", "--bind", "0.0.0.0"]
```

### Full Dockerfile: roko-cli (with Web Terminal)

The full variant layers `tmux` and `ttyd` on top of the same binary so operators can attach to
the running process without SSH.

---

## Entrypoint Scripts

Entry points launch the service, keep the terminal session alive, and fail the container if the
service dies so the orchestrator can restart it.

---

## Docker Build Pattern: cargo-chef

cargo-chef splits dependency compilation from source compilation so cached Docker layers stay
hot across source-only changes. That keeps the container profile fast to rebuild in CI and on
remote builders.

---

### Build Strategy: Pre-Compiled Binaries

The preferred CI path cross-compiles the Rust binary first, then copies it into the image. That
keeps Docker builds small, reproducible, and independent of the Rust toolchain inside the final
image.

---

## CI Workflow for Docker Builds

The CI workflow builds and publishes service images to `ghcr.io` for the container profile. The
same build matrix can publish slim and full variants without changing runtime code.

---

## docker-compose.yml

`docker-compose.yml` is the local packaging bundle for laptop-local and single-server testing:

- Service containers reuse the same profile logic as production.
- A shared volume gives the container a durable state directory.
- Health checks use `/readyz`, so Compose, Helm, and Fly.io all agree on readiness.
- Multi-tenant experiments should use separate Compose projects or separate volumes.

---

## .dockerignore

`.dockerignore` excludes build outputs, workspace caches, and local state so the image context
stays small and deterministic.
