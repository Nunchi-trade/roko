# Five-Shape Deployment as Config Signal

> Deployment shape is not an architectural fork. It is a Signal that propagates through the
> Graph, controlling which Cells instantiate, how fabrics are backed, and what resources are
> available. One Rust binary, five modes, selected by configuration.

---

## Core Insight

Traditional deployment stories produce different binaries or code paths per environment.
Roko's deployment model rejects this: the binary is identical everywhere. What changes is a
**Shape Signal** -- a config-level datum that parameterizes the universal Graph at startup.

The Shape Signal answers: "What backing implementations should Store, Bus, and Connect Cells
use?" Everything else follows mechanically.

```
Shape Signal: "container"
  -> Store = SQLite (mounted volume)
  -> Bus = in-memory ring (process-local)
  -> Connect = network (HTTP + WebSocket)
  -> Trigger = webhook + cron
  -> Observe = stdout JSON + /metrics + OTLP
```

---

## The Five Shapes

| Shape | Store Backing | Bus Backing | Connect | Typical Resources |
|---|---|---|---|---|
| **Laptop** | FileSubstrate (JSONL) | in-memory ring | localhost | 8-64GB RAM, interactive user |
| **Server** | SQLite or FileSubstrate | in-memory ring | Unix socket + TCP | 4-32GB RAM, persistent process |
| **Container** | SQLite (mounted volume) | in-memory ring | network (HTTP) | 1-4GB RAM, orchestrator-managed |
| **Clustered** | distributed Store (replicated) | networked Bus (NATS/Redis) | inter-node mesh | N nodes, shared state |
| **Edge** | MemorySubstrate (volatile) | local ring (tiny) | MQTT/BLE/serial | <64MB RAM, intermittent network |

A sixth shape exists for **WASM**: only the cognitive kernel Cells compile. The rest are
stubbed or proxied to a host runtime.

---

## Shape as Signal Propagation

When the binary starts, it reads the shape from config and constructs a Shape Signal:

```rust
/// The deployment shape, resolved from config at startup.
/// This Signal propagates through the Graph, controlling Cell instantiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeploymentShape {
    Laptop,
    Server,
    Container,
    Clustered,
    Edge,
    Wasm,
}

/// Config-level shape selection. Lives in roko.toml or environment.
/// ```toml
/// [deployment]
/// shape = "container"
///
/// [deployment.container]
/// store = { kind = "sqlite", path = "/var/lib/roko/state.db" }
/// bus = { kind = "memory", ring_size = 65536 }
/// listen = "0.0.0.0:6677"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct ShapeConfig {
    pub shape: DeploymentShape,
    pub store: StoreConfig,
    pub bus: BusConfig,
    pub listen: Option<SocketAddr>,
    pub observe: ObserveConfig,
}

/// Resolve shape from layered config sources.
/// Priority: CLI flag > env > TOML > compiled default.
pub fn resolve_shape() -> ShapeConfig {
    // 1. CLI flag: --shape container
    // 2. Env: ROKO_SHAPE=container
    // 3. TOML: [deployment] shape = "container"
    // 4. Default: detect from environment (Docker? launchd? systemd?)
    //    - if /run/user/$UID exists and systemd user session -> Server
    //    - if /.dockerenv exists -> Container
    //    - if running under launchd -> Server
    //    - else -> Laptop
    todo!()
}
```

The Shape Signal flows into the Graph factory, which selects concrete Cell implementations:

```rust
/// Instantiate the runtime Graph based on the Shape Signal.
pub fn build_runtime_graph(shape: &ShapeConfig) -> Graph {
    let store: Box<dyn Store> = match &shape.store {
        StoreConfig::File { path } => Box::new(FileSubstrate::new(path)),
        StoreConfig::Sqlite { path } => Box::new(SqliteStore::new(path)),
        StoreConfig::Memory => Box::new(MemorySubstrate::new()),
        StoreConfig::Distributed { nodes } => Box::new(DistributedStore::new(nodes)),
    };

    let bus: Box<dyn Bus> = match &shape.bus {
        BusConfig::Memory { ring_size } => Box::new(InMemoryBus::new(*ring_size)),
        BusConfig::Nats { url } => Box::new(NatsBus::connect(url)),
    };

    let observe: Vec<Box<dyn Observe>> = match shape.shape {
        DeploymentShape::Laptop => vec![
            Box::new(HumanLogObserver::new()),
        ],
        DeploymentShape::Container => vec![
            Box::new(JsonLogObserver::new()),
            Box::new(PrometheusObserver::new()),
        ],
        DeploymentShape::Edge => vec![
            Box::new(CompactLogObserver::new()),
        ],
        _ => vec![Box::new(JsonLogObserver::new())],
    };

    Graph::builder()
        .store(store)
        .bus(bus)
        .observers(observe)
        .build()
}
```

---

## Feature Flags Mirror Shape Selection

Cargo feature flags compose at compile time what shape config composes at runtime. The
relationship is: features determine what _can_ instantiate; shape determines what _does_
instantiate.

```toml
# Cargo.toml features for roko-cli
[features]
default = ["full"]
full = ["tui", "serve", "all-backends", "all-gates", "daemon"]

# Minimal: core loop only (for containers that don't need TUI)
headless = ["all-backends", "all-gates"]

# Edge: cognitive kernel only
edge = []

# Individual capabilities
tui = ["dep:ratatui", "dep:crossterm"]
serve = ["dep:axum", "dep:tower"]
daemon = ["dep:notify", "dep:cron"]
all-backends = ["anthropic", "openai", "openrouter"]
all-gates = ["compile-gate", "test-gate", "clippy-gate", "diff-gate"]
```

The mapping between features (compile-time) and shapes (runtime):

| Feature Set | Supported Shapes |
|---|---|
| `full` | Laptop, Server, Container, Clustered |
| `headless` | Server, Container, Clustered |
| `edge` | Edge |
| `serve` | Container, Clustered (HTTP control plane) |
| `daemon` | Server (persistent background process) |

A container image built with `--features headless` can still select `shape = "container"` or
`shape = "server"` at runtime. The shape Signal works within the capabilities that features
compiled in.

---

## Per-Shape Cell Instantiation

Each shape activates different Cells in the runtime Graph:

### Laptop

```toml
[deployment]
shape = "laptop"

[deployment.laptop]
store = { kind = "file", path = ".roko/signals.jsonl" }
bus = { kind = "memory", ring_size = 16384 }
# No listen address -- interactive CLI only
```

- Store: JSONL files in `.roko/`
- Bus: in-memory ring buffer (16K slots)
- Connect: none (no network listener)
- Trigger: manual CLI invocation only
- Observe: human-readable logs to stderr

### Server (Daemon)

```toml
[deployment]
shape = "server"

[deployment.server]
store = { kind = "sqlite", path = "~/.local/state/roko/state.db" }
bus = { kind = "memory", ring_size = 65536 }
listen = "127.0.0.1:6677"
ipc_socket = "/tmp/roko-daemon.sock"
```

- Store: SQLite for durability across restarts
- Bus: larger in-memory ring (65K slots)
- Connect: Unix socket IPC + localhost HTTP
- Trigger: cron + filesystem watch + webhook
- Observe: structured JSON logs + journald/file rotation

### Container

```toml
[deployment]
shape = "container"

[deployment.container]
store = { kind = "sqlite", path = "/var/lib/roko/state.db" }
bus = { kind = "memory", ring_size = 65536 }
listen = "0.0.0.0:6677"
```

- Store: SQLite on a mounted volume
- Bus: in-memory ring (lost on container restart, rebuilt from Store)
- Connect: network listener on all interfaces
- Trigger: webhook ingress (external events)
- Observe: JSON to stdout + `/metrics` + `/healthz` + `/readyz`

### Clustered

```toml
[deployment]
shape = "clustered"

[deployment.clustered]
store = { kind = "distributed", nodes = ["node-1.internal:6677", "node-2.internal:6677"] }
bus = { kind = "nats", url = "nats://nats.internal:4222" }
listen = "0.0.0.0:6677"
```

- Store: replicated across nodes (consensus or CRDT)
- Bus: NATS for cross-node Pulse delivery
- Connect: inter-node mesh + external HTTP
- Trigger: any node can receive webhooks
- Observe: centralized log aggregation + distributed traces

### Edge

```toml
[deployment]
shape = "edge"

[deployment.edge]
store = { kind = "memory", max_entries = 10000 }
bus = { kind = "memory", ring_size = 256 }
sync = { kind = "mqtt", broker = "mqtt://core.example.com:1883", interval_sec = 300 }
```

- Store: MemorySubstrate (volatile, bounded)
- Bus: tiny ring buffer (256 slots)
- Connect: MQTT/BLE/serial for periodic sync
- Trigger: local sensor events
- Observe: compact binary logs, deferred export

---

## WASM: The Sixth Shape

WASM is unique: it compiles only the cognitive kernel Cells. The I/O fabric is provided by
the host runtime (browser, edge function, wasmtime).

```
Native Binary (full):
  [Store] [Bus] [Score] [Verify] [Route] [Compose] [React] [Connect] [Trigger] [Observe]
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  All Cells instantiate from the same binary

WASM Module (kernel only):
  [Score] [Route] [Compose] [HDC]
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  Only pure-computation Cells compile
  Store/Bus/Connect provided by host imports
```

The WASM module size budget: ~500KB gzipped for the cognitive kernel. This includes:
- Signal type + serialization (~50KB)
- Score protocol implementation (~30KB)
- Route protocol implementation (~30KB)
- Compose protocol implementation (~40KB)
- HDC vectors + Hamming similarity (~20KB)
- BLAKE3 content hashing (~30KB)
- serde infrastructure (~200KB)
- WASM runtime glue (~50KB)

---

## Shape Detection Heuristic

When no explicit shape is configured, the binary auto-detects:

```rust
/// Detect deployment shape from environment signals.
pub fn detect_shape() -> DeploymentShape {
    // Container indicators
    if std::path::Path::new("/.dockerenv").exists()
        || std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
    {
        return DeploymentShape::Container;
    }

    // Fly.io indicator
    if std::env::var("FLY_APP_NAME").is_ok() {
        return DeploymentShape::Container;
    }

    // Daemon indicator (running under service manager)
    if std::env::var("INVOCATION_ID").is_ok() // systemd
        || std::env::var("__CFBundleIdentifier").is_ok() // launchd
    {
        return DeploymentShape::Server;
    }

    // Edge indicator (resource-constrained)
    if cfg!(target_arch = "wasm32") {
        return DeploymentShape::Wasm;
    }

    // Default: laptop (interactive)
    DeploymentShape::Laptop
}
```

---

## Config TOML: Complete Shape Example

```toml
# roko.toml -- deployment section

[deployment]
# Explicit shape selection (overrides auto-detection)
shape = "container"

# Shape-specific overrides
[deployment.container]
store = { kind = "sqlite", path = "/var/lib/roko/state.db" }
bus = { kind = "memory", ring_size = 65536 }
listen = "0.0.0.0:6677"

[deployment.container.observe]
log_format = "json"          # json | human | compact
log_level = "info"           # trace | debug | info | warn | error
metrics_endpoint = "/metrics"
healthz_endpoint = "/healthz"
readyz_endpoint = "/readyz"
otlp_endpoint = ""           # empty = disabled

[deployment.container.limits]
max_concurrent_agents = 4
max_memory_mb = 2048
max_store_entries = 1_000_000
shutdown_drain_sec = 30
```

---

## What This Enables

1. **One binary, many environments**: The same `roko-cli` binary runs on a developer laptop,
   in a Docker container, behind Fly.io, and (with edge features) on a Raspberry Pi.

2. **Config-driven scaling**: Moving from laptop to server to clustered requires only config
   changes, not code changes or recompilation.

3. **Testability**: Integration tests can override the shape to "laptop" with MemorySubstrate,
   making the full Graph testable without filesystem or network dependencies.

4. **Portable state**: Because the Shape Signal only changes the fabric backing (not the
   Signal format), state can be exported from one shape and imported into another. A developer
   can `roko state export` from their laptop and `roko state import` into a container.

---

## Feedback Loops

- **Shape-aware cost tracking**: Container and clustered shapes track LLM costs per-request
  for budget enforcement. Laptop shape tracks for developer awareness but does not enforce.

- **Adaptive Bus sizing**: The Bus ring size grows or shrinks based on observed Pulse
  throughput (Loop pattern: measure throughput -> adjust ring_size -> observe drops -> correct).

- **Store pressure → demurrage**: When Store approaches configured limits, demurrage rates
  increase, naturally pruning low-value Signals without operator intervention.

---

## Open Questions

1. **Clustered Store consensus**: Should clustered shape use Raft consensus (strong consistency)
   or Merkle-CRDT (eventual consistency with conflict-free merge)? Current design leans CRDT
   for availability, but gate verdicts may need strong consistency.

2. **Shape transitions at runtime**: Can a running instance change shape (e.g., laptop -> server
   when `roko daemon install` is run)? Current answer: no, shape is fixed at startup. The
   daemon is a separate process invocation.

3. **Mixed-shape clusters**: Can a cluster mix container nodes and edge nodes? If so, how does
   the Bus bridge between networked Bus (NATS) and edge sync (MQTT)?

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Define `DeploymentShape` enum and `ShapeConfig` struct | `crates/roko-core/src/config/shape.rs` | Not started |
| Implement `detect_shape()` auto-detection | `crates/roko-cli/src/lib.rs` | Not started |
| Wire shape into Graph factory | `crates/roko-cli/src/orchestrate.rs` | Not started |
| Add `[deployment]` section to config schema | `crates/roko-core/src/config/mod.rs` | Not started |
| Feature-flag composition validation | `crates/roko-cli/build.rs` | Not started |
| WASM cognitive kernel crate | `crates/roko-wasm/` | Not started |
| MemorySubstrate bounded mode for edge | `crates/roko-std/src/memory_substrate.rs` | Partial |
| SQLite Store implementation | `crates/roko-fs/src/sqlite_store.rs` | Not started |
| Shape-aware observe factory | `crates/roko-cli/src/observe.rs` | Not started |
