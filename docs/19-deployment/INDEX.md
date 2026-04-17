# 19 — Deployment

> How Roko is packaged, distributed, and deployed across five shapes: laptop-local,
> single-server, container, clustered, and edge. The chapter is profile-driven: one Rust
> binary plus packaging artifacts, with configuration selecting the shape instead of forking
> the codebase. See also `../../tmp/refinements/24-deployment-ux.md` and
> `../../tmp/refinements/27-realtime-event-surface.md`.

---

## Sub-Documents

| # | File | Title | Summary |
|---|---|---|---|
| 00 | [00-packaging-and-distribution.md](00-packaging-and-distribution.md) | Packaging and Distribution | Cargo workspace packaging, crates.io publishing, release pipeline (release-plz + cargo-dist + git-cliff), Homebrew tap, GitHub Releases, Docker images, self-update via axoupdater, shell completions. The path from source to a shape-specific artifact. |
| 01 | [01-native-x86-arm.md](01-native-x86-arm.md) | Native Deployment (x86_64 and aarch64) | Native build configuration for 6 platform targets (macOS Intel/ARM, Linux glibc/musl). Feature flags, cross-compilation (cargo-zigbuild, cross), release profile optimization, binary sizes, memory usage, and development build workflow. |
| 02 | [02-wasm-browser-edge.md](02-wasm-browser-edge.md) | WASM Deployment (Browser and Edge) | WebAssembly compilation of core cognitive primitives. What works (Engram, Score, Scorer, Router, Composer, HDC), what doesn't (filesystem, LLM backends, Tokio). MemorySubstrate, wasm-bindgen browser integration, edge function pattern, WASM module size budget (~500KB). |
| 03 | [03-docker.md](03-docker.md) | Docker Deployment | Container shape as a profile-driven packaging target: image variants, build caching, registry naming, compose bundles, state volumes, observability hooks, and operator access. |
| 04 | [04-daemon-launchd-macos.md](04-daemon-launchd-macos.md) | Daemon Mode: launchd (macOS) | Persistent background daemon on macOS. launchd plist generation (`dev.nunchi.roko`), lifecycle commands (install/start/stop/uninstall), IPC over Unix domain socket, DaemonCmd protocol, log management, graceful shutdown, environment variable handling. |
| 05 | [05-daemon-systemd-linux.md](05-daemon-systemd-linux.md) | Daemon Mode: systemd (Linux) | Persistent background daemon on Linux. systemd user unit file generation, exponential restart backoff, journald log integration, watchdog (sd_notify), security hardening (NoNewPrivileges, ProtectSystem), lingering for headless servers. Comparison table: launchd vs systemd. |
| 06 | [06-cloud-fly-io.md](06-cloud-fly-io.md) | Cloud Deployment: Fly.io | Fly.io deployment architecture. fly.toml per service (roko-cli, roko-serve, mirage-rs, console), deploy scripts (fly-deploy.sh, fly-secrets.sh, fly-status.sh), auto-stop/auto-start, persistent volumes, private networking (.internal DNS), custom domains, cost estimates, console web terminal service. |
| 07 | [07-edge-embedded.md](07-edge-embedded.md) | Edge and Embedded Deployment | Minimal ~500KB binary for resource-constrained environments. Feature exclusion strategy, binary size budget, use cases (edge scoring, local knowledge cache, HDC similarity, offline sync), edge-core communication protocol, relationship to WASM target. |
| 08 | [08-subscription-configuration.md](08-subscription-configuration.md) | Subscription Configuration | How the daemon monitors repositories. Three trigger types: cron (time-scheduled), watch (filesystem events via notify), webhook (HTTP POST). Configuration format in roko.toml, per-repo overrides, debouncing, changed-path filtering, ${VAR} interpolation, subscription lifecycle (add/remove/pause/resume). |
| 09 | [09-multi-repo-coordination.md](09-multi-repo-coordination.md) | Multi-Repo Coordination | Single daemon managing N repositories. Isolation model (filesystem, process, config), shared scheduler with priority queue (webhook > watch > cron), per-repo agent limits, cross-repo knowledge sharing via Agent Mesh, C-Factor tracking, git worktree isolation, error handling and resource exhaustion protection. |
| 10 | [10-secret-management.md](10-secret-management.md) | Secret Management | Layered secret resolution across shapes: env vars, config, OS keychain, secret stores, `_FILE` injection, `${VAR}` interpolation, scoped and role-based secrets, and audit checks. |
| 11 | [11-remote-orchestrator.md](11-remote-orchestrator.md) | Remote Orchestrator | Roko as a long-lived HTTP service. roko-serve crate, REST API, projection queries, shared realtime surface over WebSocket/SSE/optional gRPC, per-subscription auth, cursor resumption, remote-consumer patterns, webhook integration, and cost tracking. |
| 12 | [12-production-hardening.md](12-production-hardening.md) | Production Hardening | Resilience patterns for production operation. Adaptive timeouts, exponential backoff with full jitter, retry decisions, per-provider concurrency, context overflow handling, zero-downtime upgrades, observability, realtime-surface telemetry, multi-tenant safety, content-addressed dedup cache, and hedged requests. |
| 13 | [13-current-status-and-port-allocation.md](13-current-status-and-port-allocation.md) | Current Status and Port Allocation | Implementation status matrix for all deployment features, complete port allocation table (8080, 8443, 8545, 3000, 7681, 9090, 9100), Fly.io internal addressing, Tier 3H deployment roadmap (Tier 1 foundation → Tier 2 distribution → Tier 3H daemon/remote → P3 future), configuration file location summary, deployment target summary, test matrix. |

---

## Key Concepts

- **Synapse Architecture**: Roko's 6-trait composition system (Substrate, Scorer, Gate, Router, Composer, Policy) enables flexible deployment, and each trait can be implemented differently per target.
- **Profiles**: A profile bundles defaults for one of the five shapes so the same binary can move between laptop-local, single-server, container, clustered, and edge without code forks.
- **Engram**: The durable record flows through all deployment targets unchanged. Content-addressed hashing keeps identity consistent across native, container, clustered, and edge environments.
- **State portability**: Substrate state, bus queues, and config are exported as an archive so laptop-to-server and server-to-server moves are operational, not architectural.
- **Realtime surface**: Remote consumers use one cursor-aware protocol over WebSocket, SSE, and optional gRPC instead of deployment-specific socket APIs.
- **Operational hardening**: Observability, secret handling, and upgrade semantics are part of the deployment story, not separate afterthoughts.

---

## Cross-References

| Topic | See Also |
|---|---|
| REF24 deployment UX | `../../tmp/refinements/24-deployment-ux.md` |
| REF27 realtime surface | `../../tmp/refinements/27-realtime-event-surface.md` |
| Agent types and deployment flexibility | Agent Types documentation, §8 |
| Port allocation and interfaces | Interfaces documentation, §7 |
| Implementation priorities and tiers | Implementation Priorities, Tier 3H |
| Model routing (provider management) | Model Routing: Operational Surface |
| Production hardening (retry, backoff) | Model Routing: Production Hardening |
| Agent configuration (roko.toml) | Developer Guide, §5 Deployment Targets |
