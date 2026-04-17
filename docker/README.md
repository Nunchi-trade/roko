# roko/docker

Container images and local-stack compose file for Roko (§42.a of the Mori parity checklist).

## Contents

| File | Purpose | Checklist |
| --- | --- | --- |
| `roko.Dockerfile` | Multi-stage build of the `roko` CLI on a distroless base | §42.1 |
| `mirage.Dockerfile` | `mirage-rs` (EVM fork simulator) with `binary,chain` features | §42.2 |
| `gateway.Dockerfile` | Placeholder `roko-gateway` image (TODO until the crate exists) | §42.3 |
| `docker-compose.yml` | roko + gateway + mirage + prometheus + grafana topology | §42.4 |
| `prometheus.yml` | Scrape config consumed by the Prometheus service | §42.4 |

Multi-arch build and GHCR publishing live in `.github/workflows/docker-publish.yml` (§42.5, §42.6).

## Quick start

```bash
cd roko/docker
docker compose up --build
```

First build pulls the Rust toolchain and warms the cargo registry/target caches
(takes a few minutes). Subsequent builds reuse BuildKit cache mounts.

## Services and ports

| Service | Port(s) | Notes |
| --- | --- | --- |
| `mirage` | `8545` JSON-RPC, `9091` metrics | Default CMD enables `--enable-hdc --enable-knowledge --enable-stigmergy` and binds `0.0.0.0`. |
| `roko` | `9092` | Long-running `roko serve` so operators can `docker compose exec roko roko <cmd>`. |
| `gateway` | `8080` | Placeholder — see the TODO in `gateway.Dockerfile`. |
| `prometheus` | `9090` | Scrapes mirage/roko/gateway + itself. |
| `grafana` | `3000` | Login `admin / admin` unless overridden. |

## Environment variables

| Variable | Service | Default | Description |
| --- | --- | --- | --- |
| `RUST_LOG` | roko, mirage, gateway | `info` | `tracing-subscriber` env-filter. |
| `ROKO_MIRAGE_URL` | roko, gateway | `http://mirage:8545` | Upstream RPC endpoint. |
| `GF_SECURITY_ADMIN_USER` | grafana | `admin` | Grafana admin user. |
| `GF_SECURITY_ADMIN_PASSWORD` | grafana | `admin` | Grafana admin password — override in prod. |

## State persistence

mirage-rs supports periodic atomic snapshots of all in-memory state (dirty accounts, deployed contracts, chain extension data) to a single JSON file. This lets seeded data survive container restarts.

To enable persistence in Docker Compose, add a named volume and pass `--state-dir`:

```yaml
services:
  mirage:
    volumes:
      - mirage-state:/workspace/.roko
    command:
      - "--host"
      - "0.0.0.0"
      - "--port"
      - "8545"
      - "--enable-hdc"
      - "--enable-knowledge"
      - "--enable-stigmergy"
      - "--state-dir"
      - "/workspace/.roko/state"
      - "--snapshot-interval-secs"
      - "15"

volumes:
  mirage-state:
```

See `apps/mirage-rs/README.md` > "State persistence" for full details on what is persisted, Railway deployment, and configuration options.

## Healthcheck

`mirage` has a healthcheck that invokes `mirage-rs healthcheck --url http://127.0.0.1:8545`,
which issues an `eth_blockNumber` RPC. Other services `depends_on` mirage so
they wait for it to come up.

## Publishing

Images are published to GHCR by `.github/workflows/docker-publish.yml` on pushes
to `main` and on `v*` tags, for `linux/amd64` and `linux/arm64`. See the workflow
for tag conventions (`sha-<short>`, `<branch>`, semver from git tag, and `latest`
on main).

## TODO

- Replace the `roko-gateway` placeholder once the crate is scaffolded.
- Add a Grafana provisioning volume for default dashboards.
- Add alertmanager once §40 alerting lands.
