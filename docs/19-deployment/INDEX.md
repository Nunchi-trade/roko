# 19 — Deployment

> How Roko is packaged, distributed, and deployed across five shapes: laptop-local,
> single-server, container, clustered, and edge. The chapter is profile-driven: one Rust
> binary plus packaging artifacts, with configuration selecting the shape instead of forking
> the codebase. See also `../../tmp/refinements/24-deployment-ux.md` and
> `../../tmp/refinements/27-realtime-event-surface.md`, and
> `../../tmp/refinements/33-observability-telemetry.md`.

> **Implementation status**: Only the laptop-local shape is currently tested end to end. Docker and server-oriented packaging exist in partial form, but the full five-shape profile story remains a target-state deployment model. Read single-server, clustered, WASM, and edge sections as design targets unless a subdocument says otherwise.

---

## Sub-Documents

| # | File | Title | Summary |
|---|---|---|---|
| 00 | [00-packaging-and-distribution.md](00-packaging-and-distribution.md) | Packaging and Distribution | Cargo workspace packaging, crates.io publishing, release pipeline (release-plz + cargo-dist + git-cliff), Homebrew tap, GitHub Releases, Docker images, self-update via axoupdater, shell completions. The path from source to a shape-specific artifact. |
| 01 | [01-native-x86-arm.md](01-native-x86-arm.md) | Native Deployment (x86_64 and aarch64) | Native build configuration for 6 platform targets (macOS Intel/ARM, Linux glibc/musl). Feature flags, cross-compilation (cargo-zigbuild, cross), release profile optimization, binary sizes, memory usage, and development build workflow. |
| 02 | [02-wasm-browser-edge.md](02-wasm-browser-edge.md) | WASM Deployment (Browser and Edge) | Target-state WASM deployment sketch: core primitives that might eventually compile, current gaps, and size-budget constraints. |
| 03 | [03-docker.md](03-docker.md) | Docker Deployment | Container shape as a profile-driven packaging target: image variants, build caching, registry naming, compose bundles, state volumes, observability hooks, and operator access. |
| 04 | [04-daemon-launchd-macos.md](04-daemon-launchd-macos.md) | Daemon Mode: launchd (macOS) | Persistent background daemon on macOS. launchd plist generation (`dev.nunchi.roko`), lifecycle commands (install/start/stop/uninstall), IPC over Unix domain socket, DaemonCmd protocol, log management, graceful shutdown, environment variable handling. |
| 05 | [05-daemon-systemd-linux.md](05-daemon-systemd-linux.md) | Daemon Mode: systemd (Linux) | Persistent background daemon on Linux. systemd user unit file generation, exponential restart backoff, journald log integration, watchdog (sd_notify), security hardening (NoNewPrivileges, ProtectSystem), lingering for headless servers. Comparison table: launchd vs systemd. |
| 06 | [06-cloud-fly-io.md](06-cloud-fly-io.md) | Cloud Deployment: Fly.io | Target-state Fly.io deployment plan: proposed service topology, scripts, and runtime layout rather than a verified current deployment. |
| 07 | [07-edge-embedded.md](07-edge-embedded.md) | Edge and Embedded Deployment | Minimal ~500KB binary for resource-constrained environments. Feature exclusion strategy, binary size budget, use cases (edge scoring, local knowledge cache, HDC similarity, offline sync), edge-core communication protocol, relationship to WASM target. |
| 08 | [08-subscription-configuration.md](08-subscription-configuration.md) | Subscription Configuration | How the daemon monitors repositories. Three trigger types: cron (time-scheduled), watch (filesystem events via notify), webhook (HTTP POST). Configuration format in roko.toml, per-repo overrides, debouncing, changed-path filtering, ${VAR} interpolation, subscription lifecycle (add/remove/pause/resume). |
| 09 | [09-multi-repo-coordination.md](09-multi-repo-coordination.md) | Multi-Repo Coordination | Target-state daemon scheduler design for multiple repositories, including isolation, prioritization, and future coordination hooks. |
| 10 | [10-secret-management.md](10-secret-management.md) | Secret Management | Layered secret resolution across shapes: env vars, config, OS keychain, secret stores, `_FILE` injection, `${VAR}` interpolation, scoped and role-based secrets, and audit checks. |
| 11 | [11-remote-orchestrator.md](11-remote-orchestrator.md) | Remote Orchestrator | Roko as a long-lived HTTP service. WebSocket and SSE are current transports; optional gRPC remains deferred target-state protocol work. |
| 12 | [12-production-hardening.md](12-production-hardening.md) | Production Hardening | Resilience patterns for production operation. Some retry and timeout work exists; other items here remain target-state hardening rather than current deployment guarantees. |
| 13 | [13-current-status-and-port-allocation.md](13-current-status-and-port-allocation.md) | Current Status and Port Allocation | Implementation status matrix for all deployment features, complete port allocation table (8080, 8443, 8545, 3000, 7681, 9090, 9100), Fly.io internal addressing, Tier 3H deployment roadmap (Tier 1 foundation → Tier 2 distribution → Tier 3H daemon/remote → P3 future), configuration file location summary, deployment target summary, test matrix. |
| 14 | [14-observability-and-telemetry.md](14-observability-and-telemetry.md) | Observability and Telemetry | Current baseline plus target-state observability plan: structured logs, JSONL artifacts, StateHub, and future Prometheus/OTLP surfaces. See also `../../tmp/refinements/33-observability-telemetry.md`. |

---

## Key Concepts

- **Synapse Architecture**: Roko's 6-trait composition system (Substrate, Scorer, Gate, Router, Composer, Policy) enables flexible deployment, and each trait can be implemented differently per target.
- **Profiles**: A profile is the intended packaging model for the five shapes; today, laptop-local is the only fully tested shape.
- **Engram**: The durable record flows through all deployment targets unchanged. Content-addressed hashing keeps identity consistent across native, container, clustered, and edge environments.
- **Current observability baseline**: JSONL episode logs, efficiency events, tracing-based logs, and the existing `StateHub`/dashboard path are the real operator surfaces today. Broader telemetry and projection catalogs remain future work.
- **State portability**: Substrate state, bus queues, and config are exported as an archive so laptop-to-server and server-to-server moves are operational, not architectural.
- **Realtime surface**: WebSocket and SSE are current. Optional gRPC is deferred.
- **Operational hardening**: Observability, telemetry, secret handling, and upgrade semantics are part of the deployment story, not separate afterthoughts.

---

## Cross-References

| Topic | See Also |
|---|---|
| REF24 deployment UX | `../../tmp/refinements/24-deployment-ux.md` |
| REF27 realtime surface | `../../tmp/refinements/27-realtime-event-surface.md` |
| REF33 observability and telemetry | `../../tmp/refinements/33-observability-telemetry.md` |
| Deployment telemetry chapter | [14-observability-and-telemetry.md](14-observability-and-telemetry.md) |
| Canonical vocabulary for Engram, Pulse, Bus, Substrate, and StateHub | [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) |
| Agent types and deployment flexibility | [../02-agents/04-agent-roles.md](../02-agents/04-agent-roles.md) |
| Port allocation and interfaces | [13-current-status-and-port-allocation.md](13-current-status-and-port-allocation.md), [../12-interfaces/INDEX.md](../12-interfaces/INDEX.md) |
| Implementation priorities and tiers | [13-current-status-and-port-allocation.md](13-current-status-and-port-allocation.md) |
| Model routing (provider management) | [../02-agents/01-provider-registry.md](../02-agents/01-provider-registry.md), [../02-agents/14-provider-integrations.md](../02-agents/14-provider-integrations.md) |
| Production hardening (retry, backoff) | [12-production-hardening.md](12-production-hardening.md) |
| Agent configuration (roko.toml) | [../17-lifecycle/03-configuration-and-operator-model.md](../17-lifecycle/03-configuration-and-operator-model.md) |
