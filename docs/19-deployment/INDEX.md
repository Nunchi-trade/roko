# 19 — Deployment

> How Roko is packaged, distributed, and deployed across all target environments: native
> binaries, Docker images, WASM modules, daemon services, cloud infrastructure, and edge
> devices. Covers the full lifecycle from source code to running production service.

---

## Sub-Documents

| # | File | Title | Summary |
|---|---|---|---|
| 00 | [00-packaging-and-distribution.md](00-packaging-and-distribution.md) | Packaging and Distribution | Cargo workspace packaging, crates.io publishing, release pipeline (release-plz + cargo-dist + git-cliff), Homebrew tap, GitHub Releases, Docker images, self-update via axoupdater, shell completions. The complete path from `git push` to installable artifact. |
| 01 | [01-native-x86-arm.md](01-native-x86-arm.md) | Native Deployment (x86_64 and aarch64) | Native build configuration for 6 platform targets (macOS Intel/ARM, Linux glibc/musl). Feature flags, cross-compilation (cargo-zigbuild, cross), release profile optimization, binary sizes, memory usage, and development build workflow. |
| 02 | [02-wasm-browser-edge.md](02-wasm-browser-edge.md) | WASM Deployment (Browser and Edge) | WebAssembly compilation of core cognitive primitives. What works (Engram, Score, Scorer, Router, Composer, HDC), what doesn't (filesystem, LLM backends, Tokio). MemorySubstrate, wasm-bindgen browser integration, edge function pattern, WASM module size budget (~500KB). |
| 03 | [03-docker.md](03-docker.md) | Docker Deployment | Dockerfile architecture: slim (distroless) and full (tmux + ttyd) image variants. cargo-chef build caching, CI workflow for multi-arch builds, ghcr.io registry, entrypoint scripts, docker-compose for local development stack. |
| 04 | [04-daemon-launchd-macos.md](04-daemon-launchd-macos.md) | Daemon Mode: launchd (macOS) | Persistent background daemon on macOS. launchd plist generation (`dev.nunchi.roko`), lifecycle commands (install/start/stop/uninstall), IPC over Unix domain socket, DaemonCmd protocol, log management, graceful shutdown, environment variable handling. |
| 05 | [05-daemon-systemd-linux.md](05-daemon-systemd-linux.md) | Daemon Mode: systemd (Linux) | Persistent background daemon on Linux. systemd user unit file generation, exponential restart backoff, journald log integration, watchdog (sd_notify), security hardening (NoNewPrivileges, ProtectSystem), lingering for headless servers. Comparison table: launchd vs systemd. |
| 06 | [06-cloud-fly-io.md](06-cloud-fly-io.md) | Cloud Deployment: Fly.io | Fly.io deployment architecture. fly.toml per service (roko-cli, roko-serve, mirage-rs, console), deploy scripts (fly-deploy.sh, fly-secrets.sh, fly-status.sh), auto-stop/auto-start, persistent volumes, private networking (.internal DNS), custom domains, cost estimates, console web terminal service. |
| 07 | [07-edge-embedded.md](07-edge-embedded.md) | Edge and Embedded Deployment | Minimal ~500KB binary for resource-constrained environments. Feature exclusion strategy, binary size budget, use cases (edge scoring, local knowledge cache, HDC similarity, offline sync), edge-core communication protocol, relationship to WASM target. |
| 08 | [08-subscription-configuration.md](08-subscription-configuration.md) | Subscription Configuration | How the daemon monitors repositories. Three trigger types: cron (time-scheduled), watch (filesystem events via notify), webhook (HTTP POST). Configuration format in roko.toml, per-repo overrides, debouncing, changed-path filtering, ${VAR} interpolation, subscription lifecycle (add/remove/pause/resume). |
| 09 | [09-multi-repo-coordination.md](09-multi-repo-coordination.md) | Multi-Repo Coordination | Single daemon managing N repositories. Isolation model (filesystem, process, config), shared scheduler with priority queue (webhook > watch > cron), per-repo agent limits, cross-repo knowledge sharing via Agent Mesh, C-Factor tracking, git worktree isolation, error handling and resource exhaustion protection. |
| 10 | [10-secret-management.md](10-secret-management.md) | Secret Management | Layered secret resolution: CLI flags → env vars → .env (dotenvy) → OS keychain (macOS Keychain, Linux Secret Service) → config files. ${VAR} interpolation in roko.toml, scoped secrets per repo, Docker _FILE suffix convention, Fly.io secret injection, audit trail, roko doctor secret check. |
| 11 | [11-remote-orchestrator.md](11-remote-orchestrator.md) | Remote Orchestrator | Roko as a long-lived HTTP service. roko-serve crate, REST API (projects, plans, runs, PRDs, artifacts, signals), SSE event streaming, WebSocket bidirectional control, API key authentication with scopes (read/write/admin), rate limiting, multi-project management, webhook integration (GitHub), cost tracking, budget limits. |
| 12 | [12-production-hardening.md](12-production-hardening.md) | Production Hardening | Resilience patterns for production operation. Adaptive timeouts (p95 × 2, clamped [5s, 300s]), exponential backoff with full jitter, RetryAction enum (retry/failover/do-not-retry), per-provider semaphore concurrency control, context overflow handling (80% trigger), 3-phase graceful shutdown, content-addressed dedup cache, hedged requests (Dean & Barroso 2013). |
| 13 | [13-current-status-and-port-allocation.md](13-current-status-and-port-allocation.md) | Current Status and Port Allocation | Implementation status matrix for all deployment features, complete port allocation table (8080, 8443, 8545, 3000, 7681, 9090, 9100), Fly.io internal addressing, Tier 3H deployment roadmap (Tier 1 foundation → Tier 2 distribution → Tier 3H daemon/remote → P3 future), configuration file location summary, deployment target summary, test matrix. |

---

## Key Concepts

- **Synapse Architecture**: Roko's 6-trait composition system (Substrate, Scorer, Gate, Router, Composer, Policy) enables flexible deployment — each trait can be implemented differently per target (e.g., MemorySubstrate for WASM, FileSubstrate for native).
- **Engram**: The universal data unit flows through all deployment targets unchanged. Content-addressed hashing ensures identity consistency across native, WASM, and edge environments.
- **Five Layers**: Deployment targets map to architectural layers — edge/WASM runs L0-L2 (Runtime, Framework, Scaffold), native/Docker runs all five layers, cloud adds L4 Orchestration over HTTP.
- **Dual-Process Cognition**: Edge deployment implements T0 (zero-LLM) processing locally, forwarding T1/T2 to core — the 16 T0 Probes pattern applied to deployment architecture.

---

## Cross-References

| Topic | See Also |
|---|---|
| Agent types and deployment flexibility | Agent Types documentation, §8 |
| Port allocation and interfaces | Interfaces documentation, §7 |
| Implementation priorities and tiers | Implementation Priorities, Tier 3H |
| Model routing (provider management) | Model Routing: Operational Surface |
| Production hardening (retry, backoff) | Model Routing: Production Hardening |
| Agent configuration (roko.toml) | Developer Guide, §5 Deployment Targets |
