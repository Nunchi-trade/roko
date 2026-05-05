# Cloud and Edge Deployment

> Cloud and edge are two extremes of the same Shape Signal. Cloud (Fly.io) gives full
> resources with zero-cost idle. Edge gives ~500KB musl binary with offline autonomy.
> Between them sits the same cognitive kernel, the same Signal format, and the same
> Graph execution model. The difference is resource budget and connectivity assumptions.

---

## Two Extremes, One Protocol

Cloud deployment maximizes resources: unlimited memory, persistent volumes, always-connected
networking, auto-scaling. Edge deployment minimizes footprint: kilobytes of RAM, volatile
storage, intermittent connectivity, battery constraints.

Both run the same cognitive primitives. The shape Signal determines which Cells instantiate
and how the two fabrics (Store and Bus) are backed.

```
Cloud (Fly.io):                      Edge (musl binary):
  Store = SQLite on persistent vol     Store = MemorySubstrate (bounded)
  Bus = in-memory ring (64K+)          Bus = in-memory ring (256 slots)
  Connect = HTTP + WebSocket           Connect = MQTT / BLE / serial
  Full Graph: all Cells                Kernel Graph: Score + Route + HDC
  Auto-stop on idle, auto-start        Always running (low power)
  ~$3-7/month                          ~$0/month (hardware cost only)
```

---

## Cloud: Fly.io as Connect Cell

Fly.io runs Roko services as Firecracker microVMs. Each service is an independent Graph
instance with its own persistent volume, network identity, and auto-lifecycle.

### Architecture

```
                ┌────────────────────────────────────────────────┐
                │              Fly.io Organization                │
                │              (private 6PN network)              │
                │                                                 │
  HTTPS ──────>│  ┌─────────────────┐   ┌──────────────────┐    │
  (public)     │  │  roko-serve     │   │  roko-cli        │    │
               │  │  Connect Cell:  │◄─►│  Full Graph:     │    │
               │  │  HTTP API       │   │  Orchestrator    │    │
               │  │  :8080          │   │  + Agent Pool    │    │
               │  └─────────────────┘   │  Vol: /data      │    │
               │          ▲             └──────────────────┘    │
               │          │ .internal DNS                       │
               │          ▼                                      │
               │  ┌─────────────────┐                           │
               │  │  roko-console   │                           │
               │  │  Connect Cell:  │                           │
               │  │  WebSocket proxy│                           │
               │  │  :3000          │                           │
               │  └─────────────────┘                           │
               └────────────────────────────────────────────────┘
```

Key properties:
- **6PN internal networking**: Services communicate via `.internal` DNS over private IPv6.
  Traffic never touches the public internet.
- **Auto-stop on idle**: `min_machines_running = 0` means zero CPU/RAM cost when no
  requests arrive. Cold start is ~2-3s for a Rust binary.
- **Persistent volumes**: Store data survives machine restarts. Mounted at `/data`.
- **Auto-TLS**: Fly provisions Let's Encrypt certificates automatically.

### roko-serve as Remote Connect Cell

The remote orchestrator (`roko-serve`) is a Connect Cell that exposes the internal Graph
execution API over HTTP. It bridges external requests into the Bus/Store fabric.

```rust
/// roko-serve: Connect Cell exposing Graph execution over HTTP.
/// Transforms HTTP requests into Pulses on the internal Bus,
/// reads results from Store, streams progress via SSE/WebSocket.
pub struct ServeConnectCell {
    /// Axum router with ~85 routes
    router: Router,
    /// Bus subscription for live event streaming
    bus: Arc<Bus>,
    /// Store handle for durable state queries
    store: Arc<dyn Store>,
    /// Auth Pipeline (Verify Cells for API key validation)
    auth: AuthPipeline,
}

/// Auth Pipeline: sequence of Verify Cells.
/// Request -> [VerifyApiKey] -> [VerifyScope] -> [VerifyRateLimit] -> Allowed
pub struct AuthPipeline {
    verify_key: VerifyApiKeyCell,
    verify_scope: VerifyScopeCell,
    verify_rate: VerifyRateLimitCell,
}
```

### Cost Model

With auto-stop enabled (zero-cost idle):

| Service | VM | Memory | Monthly Cost |
|---|---|---|---|
| roko-serve | shared-cpu-2x | 1GB | ~$2-5 (active hours only) |
| roko-cli orchestrator | shared-cpu-2x | 2GB | ~$3-7 (active hours only) |
| roko-console | shared-cpu-1x | 256MB | ~$1-2 |
| Volumes (1GB each) | -- | -- | ~$0.15/month per volume |
| **Total (intermittent use)** | | | **$5-15/month** |

For always-on production: set `min_machines_running = 1` to eliminate cold starts,
approximately $15-30/month.

### Configuration

```toml
# deploy/fly/roko-serve/fly.toml
app = "roko-serve"
primary_region = "iad"

[build]
  dockerfile = "docker/roko-serve.Dockerfile"

[env]
  RUST_LOG = "roko_serve=info"
  ROKO_SHAPE = "container"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = "stop"
  auto_start_machines = true
  min_machines_running = 0

  [http_service.concurrency]
    type = "requests"
    hard_limit = 500
    soft_limit = 250

[[http_service.checks]]
  grace_period = "10s"
  interval = "30s"
  method = "GET"
  path = "/healthz"
  timeout = "5s"

[[vm]]
  size = "shared-cpu-2x"
  memory = "1gb"
```

### Webhook Ingress

The remote orchestrator receives webhooks from GitHub/GitLab, closing the loop from external
events to autonomous plan execution:

```
GitHub push -> Fly.io webhook endpoint -> Verify (HMAC) -> Bus Pulse -> Scheduler -> PlanRunner
```

```toml
# Webhook configuration in roko.toml (server mode)
[webhooks.github]
events = ["push", "pull_request.opened"]
secret = "${GITHUB_WEBHOOK_SECRET}"
```

---

## Edge: Minimal Cognitive Kernel

Edge deployment compiles only the pure-computation Cells into a ~500KB static binary. The
I/O-dependent components (LLM backends, filesystem persistence, process supervision) are
excluded. The edge agent operates autonomously with periodic sync to a core instance.

### What Compiles for Edge

| Cell | Included | Size | Purpose |
|---|---|---|---|
| Score | Yes | ~30KB | Evaluate Signal quality (pure math) |
| Route | Yes | ~30KB | Select action path (pure computation) |
| Compose | Yes | ~40KB | Assemble context under budget |
| HDC | Yes | ~20KB | Similarity search (50us per comparison) |
| BLAKE3 | Yes | ~30KB | Content addressing |
| MemorySubstrate | Yes | ~40KB | Volatile Signal storage |
| serde | Yes | ~200KB | Serialization for sync protocol |
| **Total** | | **~500KB** | Cognitive kernel |

| Cell | Excluded | Reason |
|---|---|---|
| Connect (HTTP) | No | reqwest + TLS too large |
| Verify (compile/test gates) | No | Requires shell execution |
| Agent dispatch | No | Requires process spawning + LLM |
| TUI | No | No display |
| Tokio (multi-threaded) | No | Single-threaded suffices |

### Edge Agent Pattern

The edge agent runs a simplified Loop: sense -> score -> decide -> accumulate -> (sync when connected).

```rust
/// Edge agent: autonomous cognitive kernel with periodic sync.
/// ~500KB binary, runs on ARM/x86 musl, <64MB RAM.
pub struct EdgeAgent {
    /// Volatile signal store (bounded to max_entries)
    store: MemorySubstrate,
    /// HDC fingerprints for known patterns
    known_patterns: Vec<(String, HdcVector)>,
    /// Accumulated signals awaiting sync
    outbox: Vec<Signal>,
    /// Sync configuration
    sync: SyncConfig,
}

impl EdgeAgent {
    /// Main loop: sense -> score -> accumulate -> maybe sync
    pub async fn run(&mut self) {
        loop {
            // Sense: read from local input (sensor, serial, stdin)
            let observation = self.sense().await;

            // Score: evaluate novelty and utility
            let score = self.score(&observation);

            // Decide: forward to core or handle locally
            if score.novelty > 0.5 || score.utility > 0.7 {
                // High value: queue for sync to core
                self.outbox.push(observation);
            } else {
                // Low value: handle locally (cache, log, ignore)
                self.store.put(observation).await;
            }

            // Sync: if connected and outbox has content
            if self.is_connected() && !self.outbox.is_empty() {
                self.sync_to_core().await;
            }

            tokio::time::sleep(self.sync.tick_interval).await;
        }
    }

    /// HDC similarity search: ~50us per 10,000-bit vector
    fn find_similar(&self, query: &HdcVector) -> Option<&str> {
        self.known_patterns.iter()
            .map(|(label, vec)| (label.as_str(), query.hamming_similarity(vec)))
            .filter(|(_, sim)| *sim > 0.526) // Cross-domain resonance threshold
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(label, _)| label)
    }
}
```

### Edge-Core Sync Protocol

Edge agents sync with core instances over any available transport. The protocol is
transport-agnostic: the same JSON-RPC messages flow over MQTT, BLE, serial, or HTTP.

```rust
/// Sync protocol: edge -> core (upload accumulated signals)
///                core -> edge (download updated knowledge)
pub struct SyncProtocol {
    transport: Box<dyn Transport>, // MQTT, BLE, serial, HTTP
}

/// Transport-agnostic Connect Cell for edge sync.
pub trait Transport: Send + Sync {
    async fn send(&self, msg: &SyncMessage) -> Result<()>;
    async fn recv(&self) -> Result<SyncMessage>;
    fn is_connected(&self) -> bool;
}

/// Three Connect Cell implementations for edge transport.
pub struct MqttTransport { broker: String, topic: String }
pub struct BleTransport { characteristic: Uuid }
pub struct SerialTransport { port: String, baud: u32 }
```

Sync messages:

```rust
#[derive(Serialize, Deserialize)]
pub enum SyncMessage {
    /// Edge -> Core: upload accumulated high-value signals
    Upload {
        signals: Vec<Signal>,
        since: u64, // timestamp of last successful sync
    },
    /// Core -> Edge: download knowledge updates
    Download {
        signals: Vec<Signal>,
        patterns: Vec<(String, HdcVector)>, // Updated HDC fingerprints
    },
    /// Core -> Edge: acknowledgment with new knowledge
    Ack {
        received: usize,
        updates: Vec<Signal>,
    },
}
```

### Build Configuration for Edge

```toml
# Cargo.toml profile for edge builds
[profile.edge]
inherits = "release"
opt-level = "z"       # Optimize for size (not speed)
lto = "fat"           # Full LTO for maximum dead code elimination
codegen-units = 1     # Single codegen unit
strip = true          # Strip all symbols
panic = "abort"       # No unwinding tables
```

```bash
# Build for ARM edge device (Raspberry Pi, embedded Linux)
cargo build --profile edge \
    -p roko-core \
    --no-default-features \
    --features "serde,hdc,decay" \
    --target aarch64-unknown-linux-musl

# Result: ~500KB static binary, zero runtime dependencies
```

---

## Brain Export: Portable Store Snapshot

Brain export enables moving an agent's knowledge between shapes. A laptop agent can export
its brain, deploy it to cloud, then sync updates back to a laptop or edge device.

The export format uses **Merkle-CRDT** for conflict-free merging when multiple instances
diverge and reconverge.

```rust
/// Brain export: portable snapshot of an agent's knowledge.
/// Uses Merkle-CRDT for conflict-free sync across instances.
pub struct BrainExport {
    /// Export metadata
    pub version: u32,
    pub exported_at: SystemTime,
    pub source_shape: DeploymentShape,
    /// Merkle root of all included signals
    pub merkle_root: ContentHash,
    /// Signals with their Merkle proofs
    pub signals: Vec<SignalWithProof>,
    /// HDC fingerprints for similarity search
    pub fingerprints: Vec<(String, HdcVector)>,
    /// Heuristics and calibration state
    pub heuristics: Vec<Heuristic>,
}

/// Merkle-CRDT merge: conflict-free union of two brain exports.
/// - Signals identified by content hash (natural dedup)
/// - Scores merged via max (higher confidence wins)
/// - Heuristics merged by comparing calibration quality (lower Brier wins)
pub fn merge_brains(local: &BrainExport, remote: &BrainExport) -> BrainExport {
    let mut merged_signals = HashMap::new();

    // Union of signals (content-addressed = natural dedup)
    for sig in local.signals.iter().chain(remote.signals.iter()) {
        merged_signals
            .entry(sig.id.clone())
            .and_modify(|existing: &mut SignalWithProof| {
                // Merge scores: take higher confidence
                if sig.score.confidence > existing.score.confidence {
                    *existing = sig.clone();
                }
            })
            .or_insert_with(|| sig.clone());
    }

    // Rebuild Merkle tree over merged set
    let merkle_root = compute_merkle_root(&merged_signals);

    BrainExport {
        version: local.version.max(remote.version),
        exported_at: SystemTime::now(),
        source_shape: DeploymentShape::Laptop, // export is shape-neutral
        merkle_root,
        signals: merged_signals.into_values().collect(),
        fingerprints: merge_fingerprints(&local.fingerprints, &remote.fingerprints),
        heuristics: merge_heuristics(&local.heuristics, &remote.heuristics),
    }
}
```

Export/import CLI:

```bash
# Export brain from laptop
roko knowledge backup --output brain-2026-04-26.roko

# Import into cloud instance
curl -X POST https://roko-serve.fly.dev/v1/brain/import \
  -H "Authorization: Bearer roko_sk_..." \
  -F "brain=@brain-2026-04-26.roko"

# Sync edge device (over MQTT)
roko knowledge sync --transport mqtt --broker mqtt://core:1883
```

Typical export sizes:
- Minimal (few signals, no fingerprints): ~10KB
- Active project (1K signals, HDC fingerprints): ~100KB-500KB
- Large workspace (10K+ signals, full heuristic set): ~1-5MB

---

## Remote Orchestrator API (roko-serve)

roko-serve is the HTTP-facing Connect Cell for cloud deployment. It exposes the internal
Graph execution API with authentication, rate limiting, cost tracking, and webhook ingress.

### API Surface

```
Health:
  GET  /healthz              -> readiness probe
  GET  /readyz               -> liveness probe

Projects:
  GET  /v1/projects          -> list managed projects
  POST /v1/projects          -> create (clone repo)
  GET  /v1/projects/:id      -> project details

Execution:
  POST /v1/projects/:id/run  -> start plan run
  GET  /v1/projects/:id/runs -> run history
  DEL  /v1/projects/:id/runs/:rid -> cancel run

Knowledge:
  GET  /v1/projects/:id/signals  -> query Store
  POST /v1/brain/import          -> import brain export

Streaming:
  GET  /v1/events (SSE)      -> real-time event stream
  WS   /ws                   -> bidirectional WebSocket

Webhooks:
  POST /webhooks/github      -> GitHub webhook ingress
  POST /webhooks/:provider   -> generic webhook ingress

Cost:
  GET  /v1/costs             -> aggregate cost breakdown
  GET  /v1/projects/:id/costs -> per-project costs
```

### Auth Pipeline

Authentication is a Pipeline of Verify Cells:

```
Request -> [ExtractKey] -> [ValidateKey] -> [CheckScope] -> [EnforceRate] -> Allowed
                                                                    |
                                                               [Reject: 429]
```

Three scopes: `read` (view), `write` (execute), `admin` (manage keys/config).

### Cost Tracking

Every LLM request is metered. Cost data flows through the Bus as efficiency Pulses and
persists in Store as cost Signals.

```toml
# Budget limits in server config
[budgets]
max_per_run_usd = 5.00
max_per_project_daily_usd = 50.00
max_daily_usd = 200.00
```

When a budget is exceeded, the Scheduler pauses the run and publishes a budget-exceeded
Pulse. The run can resume after budget increase or daily reset.

---

## Port Allocation

| Port | Service | Protocol | Access |
|---|---|---|---|
| 3000 | roko-console (web terminal) | HTTPS | Public |
| 6677 | roko-serve (daemon mode) | HTTP | Local / internal |
| 7681 | ttyd (per-service terminal) | WSS | Internal only |
| 8080 | roko-serve (cloud mode) | HTTPS | Public |
| 8443 | WebSocket event stream | WSS | Public |
| 8545 | mirage-rs JSON-RPC | HTTPS | Public |
| 9090 | Webhook ingress | HTTPS | Public |

On Fly.io, internal services communicate via `.internal` DNS over private 6PN:
- `roko-serve.internal:8080`
- `roko-cli.internal:8080`
- `roko-mirage.internal:8545`

---

## What This Enables

1. **Zero-cost idle for cloud**: With Fly.io auto-stop, a personal Roko deployment costs
   $5-15/month for intermittent use. No continuous charges when idle.

2. **Edge autonomy**: A 500KB binary on a Raspberry Pi can classify events, maintain a
   local knowledge cache, and sync insights to the cloud when connected.

3. **Portable agents**: Brain export via Merkle-CRDT means an agent's knowledge is not
   trapped in one deployment shape. Export from laptop, import to cloud, sync to edge.

4. **Closed-loop automation**: Webhooks from GitHub -> cloud orchestrator -> plan execution
   -> results posted back. No human needed after initial setup.

5. **Graceful degradation**: Edge agents work fully offline, accumulating insights for
   later sync. No hard dependency on connectivity.

---

## Feedback Loops

- **Cloud cost tracking**: Per-request cost metering feeds back into the CascadeRouter,
  which learns to route cheaper for low-stakes tasks (Loop pattern).

- **Edge sync quality**: The core tracks which edge-originated signals pass gates. If edge
  pre-filtering is too aggressive (missing novel signals) or too loose (flooding core with
  noise), the score threshold auto-adjusts (predict-publish-correct).

- **Brain export freshness**: Demurrage continues running on exported brains. Old exports
  naturally decay low-value signals, keeping imports lean.

---

## Open Questions

1. **Multi-region on Fly.io**: Should roko-serve deploy to multiple regions with request
   routing? Current answer: single region (iad) is sufficient for personal/team use. Scale
   to multi-region only for latency-sensitive production deployments.

2. **Edge fleet management**: How does a user manage 10+ edge devices? Current answer: each
   syncs independently to core. A fleet dashboard is future work.

3. **Offline conflict resolution**: What happens when two edge devices modify the same
   signal offline and then sync? Merkle-CRDT handles signal-level conflicts (content-addressed
   = natural dedup), but score conflicts need a merge strategy.

4. **Edge-to-edge direct sync**: Can two edge devices sync without going through core?
   Possible via BLE or local mesh, but not currently modeled.

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| fly.toml per service | `deploy/fly/*/fly.toml` | Not started |
| Deploy scripts (fly-deploy.sh, fly-secrets.sh) | `deploy/scripts/` | Not started |
| Dockerfile slim variant | `docker/roko-serve.Dockerfile` | Not started |
| roko-serve HTTP API wiring | `crates/roko-serve/src/routes/` | Partial (~85 routes scaffolded) |
| Auth Pipeline (API key verify) | `crates/roko-serve/src/auth.rs` | Not started |
| Webhook ingress handler | `crates/roko-serve/src/webhooks.rs` | Not started |
| Cost tracking per-request | `crates/roko-serve/src/cost.rs` | Partial |
| Brain export format | `crates/roko-core/src/brain_export.rs` | Not started |
| Merkle-CRDT merge algorithm | `crates/roko-core/src/merkle_crdt.rs` | Not started |
| Edge build profile (`[profile.edge]`) | `Cargo.toml` (workspace root) | Not started |
| Edge agent example | `examples/edge-agent/` | Not started |
| MQTT transport for edge sync | `crates/roko-runtime/src/transport/mqtt.rs` | Not started |
| BLE transport for edge sync | `crates/roko-runtime/src/transport/ble.rs` | Not started |
| Edge binary size validation | CI job | Not started |
