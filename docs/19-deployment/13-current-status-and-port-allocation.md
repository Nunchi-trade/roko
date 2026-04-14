# Current Status and Port Allocation

> This document tracks the implementation status of all deployment features described in the
> 19-deployment section, the port allocation table for all Roko services, and the Tier 3H
> roadmap for deployment-related work items.


> **Implementation**: Specified

---

## Implementation Status Overview

| Feature | Status | Priority | Blocking Dependencies |
|---|---|---|---|
| Native build (x86_64, aarch64) | **Working** | Tier 0 | None — the workspace builds and tests |
| Docker images | **Not built** | Tier 1 | Need static Dockerfiles in `docker/` |
| cargo install | **Not published** | Tier 1 | Need publish metadata in Cargo.toml |
| Homebrew tap | **Not created** | Tier 2 | Needs cargo-dist + release pipeline |
| Release pipeline (release-plz + cargo-dist) | **Not configured** | Tier 2 | Need release-plz.toml, cliff.toml, dist-workspace.toml |
| Daemon mode (launchd) | **Scaffolded** | Tier 3H | IPC protocol, subscription system |
| Daemon mode (systemd) | **Designed** | Tier 3H | Same as launchd |
| Fly.io deployment | **Not configured** | Tier 2 | Need fly.toml per service, deploy scripts |
| WASM target | **Feature flags exist** | Tier 3H | Need end-to-end validation |
| Edge target | **Designed** | P3 | Need edge build profile, validation |
| Subscription configuration | **Designed** | Tier 3H | Daemon mode |
| Multi-repo coordination | **Designed** | Tier 3H | Daemon mode, subscription system |
| Secret management (env vars) | **Working** | Tier 0 | None — roko-agent reads env vars |
| Secret management (dotenvy) | **Not wired** | Tier 2 | Need dotenvy dependency |
| Secret management (keychain) | **Not wired** | Tier 3H | Need keyring dependency |
| Secret management (${VAR} interpolation) | **Not implemented** | Tier 2 | Config parser update |
| Remote orchestrator (roko-serve) | **Scaffold** | Tier 3H | Need HTTP API wiring |
| Production hardening: adaptive timeouts | **Implemented** | Tier 2G | In roko-agent |
| Production hardening: backoff + retry | **Implemented** | Tier 2G | In roko-agent |
| Production hardening: RetryAction enum | **Implemented** | Tier 2G.17 | In roko-agent |
| Production hardening: per-provider semaphores | **Scaffolded** | Tier 2G | Designed, not wired |
| Production hardening: graceful shutdown | **Partial** | Tier 2 | ProcessSupervisor handles agents |
| Production hardening: dedup cache | **Designed** | Tier 3H | Need implementation |
| Production hardening: hedged requests | **Designed** | P3 | Need implementation |
| Console (web terminal) | **Not built** | Tier 2 | Need Caddyfile, xterm.js frontend |
| Self-update (axoupdater) | **Not wired** | Tier 2 | Needs cargo-dist release pipeline |
| Shell completions | **Not wired** | Tier 2 | Need clap_complete integration |
| Doctor subcommand | **Scaffold** | Tier 2 | Need full health checks |

---

## Port Allocation Table

All Roko services and their assigned ports:

| Port | Service | Protocol | Binding | Notes |
|---|---|---|---|---|
| 3000 | Roko Console (web terminal) | HTTPS | Public | Caddy proxy → ttyd |
| 7681 | ttyd (per-service, internal) | WSS | Internal | WebSocket-to-PTY bridge |
| 8080 | roko-serve HTTP API | HTTPS | Public | REST + SSE |
| 8080 | roko-cli server mode | HTTPS | Public | Same API as roko-serve |
| 8443 | roko-serve WebSocket | WSS | Public | Bidirectional event stream |
| 8545 | mirage-rs JSON-RPC | HTTPS | Public | Anvil-compatible EVM RPC |
| 9090 | Webhook ingress | HTTPS | Public | GitHub/GitLab webhooks |
| 9100 | Roko TUI headless RPC | JSON-RPC | Local | For programmatic TUI control |

### Fly.io Internal Ports

On Fly.io, services communicate over the private 6PN network using `.internal` DNS:

| Internal Address | Port | Service |
|---|---|---|
| `roko-cli.internal` | 8080 | Roko CLI orchestrator |
| `roko-serve.internal` | 8080 | Roko HTTP API |
| `roko-mirage.internal` | 8545 | mirage-rs EVM fork |
| `roko-console.internal` | 3000 | Web terminal console |

### Port Conflicts

When running multiple services locally (via docker-compose or direct), map to non-conflicting
host ports:

```yaml
# docker-compose port mapping
services:
  roko-cli:
    ports: ["8080:8080"]     # CLI on 8080
  roko-serve:
    ports: ["8081:8080"]     # Serve on 8081 (remapped)
  mirage:
    ports: ["8545:8545"]     # mirage-rs on 8545
  console:
    ports: ["8443:3000"]     # Console on 8443
```

---

## Tier 3H Deployment Roadmap

The deployment-related work items in the implementation plan, organized by priority:

### Tier 1 — Foundation (Can Do Now)

These items have no dependencies beyond the existing codebase:

| Item | Description | Estimated Effort |
|---|---|---|
| Static Dockerfiles | Create `docker/*.Dockerfile` (slim + full) | 1 day |
| Deploy scripts | Create `deploy/scripts/fly-*.sh` | Half day |
| fly.toml configs | Create `deploy/fly/*/fly.toml` per service | Half day |
| docker-compose.yml | Create `docker/docker-compose.yml` | 2 hours |
| Publish metadata | Add `publish = true`, description, keywords to Cargo.toml | 2 hours |
| .dockerignore | Create `docker/.dockerignore` | 15 minutes |

### Tier 2 — Distribution Pipeline

Depends on Tier 1 (having something to deploy):

| Item | Description | Estimated Effort |
|---|---|---|
| release-plz setup | Create `release-plz.toml`, `cliff.toml` | Half day |
| cargo-dist setup | Run `cargo dist init`, configure targets | Half day |
| CI workflows | Create release-plz.yml, release.yml, docker.yml | 1 day |
| Homebrew tap | Create `nunchi/homebrew-roko` repo | 2 hours |
| axoupdater | Wire self-update notifications + `roko update` | Half day |
| clap_complete | Wire shell completions for all binaries | 2 hours |
| dotenvy integration | Add .env file loading to CLI main() | 1 hour |
| ${VAR} interpolation | Implement in config parser | 2 hours |
| Console service | Create Caddyfile + xterm.js frontend + Dockerfile | 1 day |

### Tier 3H — Daemon and Remote (P2)

Depends on the core framework being stable:

| Item | Description | Estimated Effort |
|---|---|---|
| Daemon infrastructure | IPC socket, event loop, signal handling | 2-3 days |
| launchd integration | Plist generation, lifecycle commands | 1 day |
| systemd integration | Unit file generation, journald, watchdog | 1 day |
| Subscription system | Cron, file watch, webhook triggers | 2-3 days |
| Multi-repo scheduler | Priority queue, per-repo isolation, agent limits | 1-2 days |
| roko-serve HTTP API | Wire Axum endpoints to orchestration engine | 2-3 days |
| Authentication | API key generation, scopes, rate limiting | 1-2 days |
| Webhook ingress | GitHub/GitLab webhook handling | 1 day |
| Keychain integration | Add keyring crate, wire into secret resolution | Half day |

### P3 — Future

These are designed but not in the near-term roadmap:

| Item | Description |
|---|---|
| WASM validation | End-to-end WASM build, wasm-bindgen wrapper crate |
| Edge build profile | Create `[profile.edge]`, measure binary size, example agent |
| Hedged requests | Implement dual-provider hedging with cost limits |
| Playground infrastructure | Per-user Fly Machines, warm pool, landing page |
| Content-addressed cache | LRU dedup cache for LLM responses |

---

## Configuration File Locations

Summary of all configuration file paths for deployment:

| File | Location | Purpose |
|---|---|---|
| `roko.toml` | Project root | Per-project configuration |
| `.roko/config.toml` | Project `.roko/` dir | Per-project overrides (alternative location) |
| `~/.config/roko/config.toml` | XDG config | Global configuration |
| `~/.config/roko/daemon.env` | XDG config | Daemon environment variables (Linux systemd) |
| `~/.local/state/roko/` | XDG state | Daemon logs, subscription state, cost history |
| `~/.local/share/roko/` | XDG data | Learned patterns, playbooks (portable across projects) |
| `.env` | Project root | Per-project secrets (gitignored) |
| `release-plz.toml` | Workspace root | Release pipeline configuration |
| `cliff.toml` | Workspace root | Changelog generation configuration |
| `dist-workspace.toml` | Workspace root | cargo-dist binary distribution configuration |
| `deploy/fly/*/fly.toml` | Deploy directory | Per-service Fly.io configuration |
| `docker/*.Dockerfile` | Docker directory | Per-service Docker build definitions |
| `docker/docker-compose.yml` | Docker directory | Local development stack |

---

## Deployment Targets Summary

| Target | Binary | Features | Binary Size | Status |
|---|---|---|---|---|
| Native x86_64 macOS | roko-cli | All | ~30MB | **Working** |
| Native aarch64 macOS | roko-cli | All | ~30MB | **Working** |
| Native x86_64 Linux (glibc) | roko-cli | All | ~30MB | **Working** |
| Native x86_64 Linux (musl) | roko-cli | All | ~35MB | **Working** (CI cross-compile) |
| Native aarch64 Linux (musl) | roko-cli | All | ~35MB | **Working** (CI cross-compile) |
| Docker slim | roko-cli | All | ~20MB image | **Designed** |
| Docker full (tmux+ttyd) | roko-cli | All + web terminal | ~80MB image | **Designed** |
| WASM (wasm32-wasi) | roko-core | Core traits, HDC, serde | ~500KB | **Feature flags exist** |
| Edge (aarch64 musl, size-opt) | roko-core | Core traits, HDC | ~500KB | **Designed** |
| Daemon (macOS launchd) | roko-cli | All + IPC | ~30MB | **Scaffolded** |
| Daemon (Linux systemd) | roko-cli | All + IPC | ~30MB | **Designed** |
| Cloud (Fly.io) | roko-serve | All | Docker image | **Designed** |

---

## Test Matrix

Deployment features should be validated against this test matrix:

| Test | What It Validates | Automation |
|---|---|---|
| `cargo build --workspace --release` | Native build succeeds | CI (existing) |
| `cargo test --workspace` | All 1,000+ tests pass | CI (existing) |
| `cargo build -p roko-core --no-default-features` | Minimal build works | CI (add) |
| `docker build -f docker/roko-cli.Dockerfile .` | Docker slim image builds | CI (add) |
| `docker compose up --wait` | Local stack starts healthy | CI (add) |
| `fly deploy --config deploy/fly/roko-serve/fly.toml` | Fly.io deployment works | Manual initially |
| `roko doctor` | Installation health check | Manual |
| `roko daemon install && roko daemon status` | Daemon installs and runs | Manual |
| `curl https://roko-serve.fly.dev/health` | Remote health check | Uptime monitor |
