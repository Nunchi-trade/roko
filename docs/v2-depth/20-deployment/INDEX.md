# 20-deployment — Depth Index

Depth for [20-DEPLOYMENT.md](../../unified/20-DEPLOYMENT.md)

Core insight: **Deployment shape is a config Signal that parameterizes the same Graph.** One
Rust binary, five modes (laptop, server, container, clustered, edge), selected by
configuration. The daemon IS the production runtime. Brain export via Merkle-CRDT enables
portable agents.

---

## Source docs (15) — All Absorbed

### Packaging and distribution

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/19-deployment/00-packaging-and-distribution.md` | Absorbed | 01 (shape model context) |
| `docs/19-deployment/01-native-x86-arm.md` | Absorbed | 01 (feature flags, build config) |
| `docs/19-deployment/02-wasm-browser-edge.md` | Absorbed | 01 (WASM as sixth shape), 03 (edge kernel) |
| `docs/19-deployment/03-docker.md` | Absorbed | 01 (container shape), 03 (Fly.io images) |

### Daemon and OS integration

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/19-deployment/04-daemon-launchd-macos.md` | Absorbed | 02 (launchd Connector Cell) |
| `docs/19-deployment/05-daemon-systemd-linux.md` | Absorbed | 02 (systemd Connector Cell) |

### Cloud and edge

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/19-deployment/06-cloud-fly-io.md` | Absorbed | 03 (Fly.io as Connect Cell) |
| `docs/19-deployment/07-edge-embedded.md` | Absorbed | 03 (edge cognitive kernel) |

### Configuration and coordination

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/19-deployment/08-subscription-configuration.md` | Absorbed | 02 (Trigger Cell types) |
| `docs/19-deployment/09-multi-repo-coordination.md` | Absorbed | 02 (Scheduler, Agent Mesh) |
| `docs/19-deployment/10-secret-management.md` | Absorbed | 02 (hierarchical Store) |

### Production hardening and operations

| Source doc | Status | Absorbed into |
|---|---|---|
| `docs/19-deployment/11-remote-orchestrator.md` | Absorbed | 03 (roko-serve Connect Cell) |
| `docs/19-deployment/12-production-hardening.md` | Absorbed | 04 (all hardening patterns) |
| `docs/19-deployment/13-current-status-and-port-allocation.md` | Absorbed | 03 (port table), 04 (implementation status) |
| `docs/19-deployment/14-observability-and-telemetry.md` | Absorbed | 04 (three Lens families) |

---

## Depth docs (4)

| # | Document | Core Concept |
|---|---|---|
| [01](01-five-shape-deployment-as-config-signal.md) | Five-Shape Deployment as Config Signal | One binary, five modes. Shape is a Signal that controls Cell instantiation. Feature flags (compile-time) compose with shape config (runtime). |
| [02](02-daemon-and-subscription-system.md) | Daemon and Subscription System | Daemon as Hot Flow Graph owning Trigger Cells. 3 trigger types (cron, watch, webhook). Multi-repo scheduling with priority queue. Agent Mesh for cross-repo knowledge. Platform Connector Cells (launchd/systemd). |
| [03](03-cloud-and-edge-deployment.md) | Cloud and Edge Deployment | Two extremes of same Shape Signal. Fly.io (auto-stop, zero-cost idle, 6PN networking). Edge (~500KB musl, MemorySubstrate, HDC at 50us, offline sync via MQTT/BLE). Brain export as Merkle-CRDT. Remote orchestrator as Connect Cell with auth Pipeline. |
| [04](04-production-hardening-and-observability.md) | Production Hardening and Observability | Hardening as Verify + React + Lens Cells. Adaptive timeouts (Loop: predict-observe-correct). Backoff (React state machine). Concurrency (Verify pre-conditions). Graceful shutdown (4-phase protocol). Observability: logs as Bus Pulses, metrics as Lens output, traces as lineage-annotated Signals. StateHub projections for dashboards. |

---

## Key Relationships

```
Shape Signal (01) ──> determines which Cells instantiate
     │
     ├── Laptop shape: no daemon, interactive CLI
     │
     ├── Server shape: daemon (02) manages subscriptions
     │         │
     │         ├── Trigger Cells fire Graph templates
     │         ├── Scheduler enforces concurrency limits
     │         └── Agent Mesh shares knowledge across repos
     │
     ├── Container shape: cloud (03) with remote Connect Cell
     │         │
     │         ├── Fly.io: auto-stop, persistent volumes
     │         ├── roko-serve: HTTP API + auth Pipeline
     │         └── Webhooks close the automation loop
     │
     ├── Edge shape: minimal kernel (03) with offline sync
     │         │
     │         ├── ~500KB binary, MemorySubstrate
     │         ├── HDC similarity at 50us
     │         └── Sync via MQTT/BLE when connected
     │
     └── All shapes: production hardening (04) as Cell composition
               │
               ├── Adaptive timeouts (Loop pattern)
               ├── Backoff + failover (React pattern)
               ├── Concurrency control (Verify pattern)
               ├── Graceful shutdown (termination protocol)
               └── Observability (Lens pattern: logs, metrics, traces)
```

---

## Reading Order

1. Start with **01** to understand the shape model (everything else references it)
2. Read **02** if interested in autonomous operation (daemon, subscriptions, multi-repo)
3. Read **03** if deploying to cloud or constrained devices
4. Read **04** for operational concerns (monitoring, alerting, resilience)
