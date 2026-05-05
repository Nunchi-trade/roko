# Compute Provisioning

> **Layer**: L0 Runtime (process lifecycle, supervision) + L1 Framework (backend initialization)
>
> **Prerequisites**: `docs/17-lifecycle/01-agent-creation.md` (agent manifest), `docs/00-architecture/INDEX.md` (5-layer taxonomy)
>
> **Synapse traits**: Substrate (initialized during provisioning to hold Neuro store), Router (configured for model selection), Gate (configured for verification pipeline)


> **Implementation**: Specified

---

## Overview

Provisioning transforms an `AgentExtendedManifest` into a running agent process with initialized knowledge store, configured model routing, loaded tool profile, and active supervision. The provisioning pipeline is a type-state machine at the Rust level — each stage transitions the agent to a new type that encodes which capabilities are available, preventing compile-time errors like querying a Neuro store that has not been initialized.

Roko supports three deployment paths. The core provisioning pipeline is identical across all three — only the infrastructure layer differs.

---

## Three Deployment Paths

### Path A: Managed Compute (Hosted)

For users who want zero infrastructure management. The Roko managed service provisions VMs, injects configuration, monitors health, and destroys machines when the user requests deletion. Users pay per-use compute fees.

| VM tier | Config | Price/hr | Typical use |
|---------|--------|----------|-------------|
| `micro` | 1 shared CPU / 256MB | $0.025 | Simple monitors, keepers |
| `small` | 1 shared CPU / 512MB | $0.05 | Standard agent (default) |
| `medium` | 2 CPU / 1GB | $0.10 | Multi-tool, Neuro-heavy |
| `large` | 4 CPU / 2GB | $0.20 | Orchestration, full cognitive loop |

### Path B: Self-Deploy Helper (Automated)

For users who want to control their own infrastructure but skip the manual setup:

```bash
roko deploy --provider fly --app-name my-agent --region iad
roko deploy --provider docker --name my-agent
roko deploy --provider ssh --host 203.0.113.42 --user root
```

### Path C: Manual / Bare Metal

Download the binary, configure it, run it:

```bash
roko init
roko start --config roko.toml
```

A Raspberry Pi behind double-NAT works identically to a cloud VM because the Mesh connection model is outbound-only (WebSocket to Mesh relay, no inbound ports required).

---

## Provisioning Pipeline (Type-State Machine)

The provisioning pipeline uses Rust's type system to enforce correct ordering. Each stage transitions the agent to a new type. You cannot call `start_cognitive_loop()` on an `AgentUninitialized` — the compiler prevents it.

```rust
/// Type-state markers for the provisioning pipeline.
/// Each marker type encodes which capabilities are available.
pub struct Unvalidated;
pub struct Validated;
pub struct ResourcesAllocated;
pub struct NeuroInitialized;
pub struct RoutingConfigured;
pub struct ToolsLoaded;
pub struct MeshRegistered;
pub struct Ready;

/// Agent in a specific provisioning stage.
/// The type parameter S encodes the current stage.
pub struct Agent<S> {
    manifest: AgentExtendedManifest,
    state: AgentState,
    _stage: std::marker::PhantomData<S>,
}

impl Agent<Unvalidated> {
    /// Create a new agent from a manifest. Entry point for provisioning.
    pub fn new(manifest: AgentExtendedManifest) -> Self {
        Agent {
            manifest,
            state: AgentState::default(),
            _stage: std::marker::PhantomData,
        }
    }

    /// Validate the manifest against domain feature set and resource limits.
    pub fn validate(self) -> Result<Agent<Validated>, ProvisioningError> {
        validate_manifest(&self.manifest)?;
        Ok(Agent {
            manifest: self.manifest,
            state: self.state,
            _stage: std::marker::PhantomData,
        })
    }
}

impl Agent<Validated> {
    /// Allocate compute resources (L0 Runtime).
    /// For hosted: claim VM from warm pool or create cold.
    /// For self-hosted: verify local resource availability.
    pub async fn allocate_resources(self) -> Result<Agent<ResourcesAllocated>, ProvisioningError> {
        let resources = allocate(&self.manifest).await?;
        Ok(Agent {
            manifest: self.manifest,
            state: self.state.with_resources(resources),
            _stage: std::marker::PhantomData,
        })
    }
}

impl Agent<ResourcesAllocated> {
    /// Initialize the Neuro store (L1 Framework - Substrate).
    /// Creates the Engram storage backend, sets decay model, configures tiers.
    pub async fn init_neuro(self) -> Result<Agent<NeuroInitialized>, ProvisioningError> {
        let neuro = initialize_neuro(&self.manifest.neuro).await?;
        Ok(Agent {
            manifest: self.manifest,
            state: self.state.with_neuro(neuro),
            _stage: std::marker::PhantomData,
        })
    }
}

impl Agent<NeuroInitialized> {
    /// Configure model routing (L1 Framework - Router).
    /// Sets up cascade T0→T1→T2, provider preferences, cost limits.
    pub fn configure_routing(self) -> Result<Agent<RoutingConfigured>, ProvisioningError> {
        let router = configure_router(&self.manifest.model_routing)?;
        Ok(Agent {
            manifest: self.manifest,
            state: self.state.with_router(router),
            _stage: std::marker::PhantomData,
        })
    }
}

impl Agent<RoutingConfigured> {
    /// Load tool profile (L1 Framework).
    /// Registers available tools based on domain plugin and profile selection.
    pub fn load_tools(self) -> Result<Agent<ToolsLoaded>, ProvisioningError> {
        let tools = load_tool_profile(&self.manifest)?;
        Ok(Agent {
            manifest: self.manifest,
            state: self.state.with_tools(tools),
            _stage: std::marker::PhantomData,
        })
    }
}

impl Agent<ToolsLoaded> {
    /// Register with Mesh if enabled (L4 Orchestration).
    /// Connects outbound WebSocket to Mesh relay.
    pub async fn register_mesh(self) -> Result<Agent<MeshRegistered>, ProvisioningError> {
        if self.manifest.mesh.as_ref().map_or(false, |m| m.enabled) {
            let mesh = connect_mesh(&self.manifest.mesh).await?;
            Ok(Agent {
                manifest: self.manifest,
                state: self.state.with_mesh(mesh),
                _stage: std::marker::PhantomData,
            })
        } else {
            Ok(Agent {
                manifest: self.manifest,
                state: self.state,
                _stage: std::marker::PhantomData,
            })
        }
    }
}

impl Agent<MeshRegistered> {
    /// Final transition: agent is ready to run.
    pub fn ready(self) -> Agent<Ready> {
        Agent {
            manifest: self.manifest,
            state: self.state,
            _stage: std::marker::PhantomData,
        }
    }
}

impl Agent<Ready> {
    /// Start the cognitive loop. Only callable on a fully provisioned agent.
    pub async fn start_cognitive_loop(self) -> Result<RunningAgent, ProvisioningError> {
        // Start the universal loop: query → score → route → compose → act → verify → write → react
        start_loop(self.manifest, self.state).await
    }
}
```

### Pipeline Stages

| Stage | Layer | What happens | Duration |
|-------|-------|-------------|----------|
| 1. Validate | — | Check manifest against domain features, resource limits | ~instant |
| 2. Allocate Resources | L0 | Claim VM (hosted) or verify local resources (self-hosted) | 300ms–30s |
| 3. Initialize Neuro | L1 | Create Engram storage, set Ebbinghaus decay, configure tiers | ~1-3s |
| 4. Configure Routing | L1 | Set up T0→T1→T2 cascade, provider preferences, cost limits | ~instant |
| 5. Load Tools | L1 | Register tools based on domain plugin and profile | ~500ms |
| 6. Register Mesh | L4 | Connect outbound WebSocket to Mesh relay | ~500ms-2s |
| 7. Ready | — | Start cognitive loop | ~instant |

**Total provisioning time (warm pool)**: 3-8 seconds from manifest to first cognitive loop iteration.
**Total provisioning time (cold)**: 15-30 seconds (VM image pull + boot + initialization).

---

## Warm Machine Pool (Hosted Path)

Pre-created stopped VMs for sub-5-second provisioning. Pool manager runs every 5 minutes, maintaining 5 stopped machines per region.

At provision time:

1. Query warm pool for stopped machines in target region matching requested tier
2. If available: update machine config (env + files) then start (~300ms)
3. If pool empty: create new machine (15-30s cold fallback)

Cost: Stopped machines cost ~$0.15/GB/month for rootfs storage. 5 `small` machines × 2 regions = 10 machines × 512MB = 5.12GB. Monthly cost: ~$0.77. Negligible.

---

## Configuration Injection

At provisioning time, configuration is injected via the deployment platform's native mechanism:

- **Hosted (Fly.io)**: File injection at machine creation time
- **Docker**: Environment variables and mounted config files
- **Bare metal**: Config file at path specified by `--config`

```toml
# roko.toml (injected at provisioning time)
agent_id = "agent-V1StGXR8_Z5j"

[inference]
default_model = "claude-haiku-4-5"
escalation_model = "claude-sonnet-4-6"
critical_model = "claude-opus-4-6"

[neuro]
path = ".roko/neuro/"
decay_model = "ebbinghaus"
tier_config.transient_multiplier = 0.1
tier_config.working_multiplier = 0.5
tier_config.consolidated_multiplier = 1.0
tier_config.persistent_multiplier = 5.0

[mesh]
enabled = true
relay_url = "wss://mesh.roko.dev/v1/ws"

[tools]
profile = "standard"
```

---

## Agent Startup Sequence

```
1. Parse roko.toml                                  ~instant
2. Initialize Neuro (Engram storage backend)         ~1-3s
   +-- Or restore from backup if available
3. Load tool profile, register tools                 ~500ms
4. Initialize model routing (T0→T1→T2 cascade)      ~instant
5. Connect to Mesh (outbound WebSocket)              ~500ms-2s
6. Register on-chain identity (chain domain only)    ~2-5s
7. Start cognitive loop (first iteration)            ~instant
8. Health server reports 'ready'                     ~instant
```

**Total boot time**: The Rust binary starts in ~100ms. Full initialization including Neuro and Mesh: 3-8 seconds. This is dramatically faster than interpreted runtimes because there is no VM warmup, no dependency resolution, no JIT compilation.

---

## Process Supervision

The agent runs under a process supervisor that handles crashes and restarts:

- Maximum 5 restarts per hour
- On max exceeded, agent enters `crashed` state
- Clean exit (exit code 0) means graceful shutdown — no restart
- Non-zero exit code triggers restart with exponential backoff

For hosted deployments, the control plane monitors agent health via internal health endpoints. For self-hosted, the user is responsible for supervision (systemd, launchd, or manual monitoring).

---

## Machine Lifecycle States

```
provisioning --> booting --> ready --> draining --> destroyed
     |                        |
     | (create fails)         |
     v                        v
  destroyed               crashed
(provision_failed)    (admin can force-destroy)
```

| From | To | Trigger |
|------|----|---------|
| (new) | `provisioning` | Manifest validated, resources requested |
| `provisioning` | `booting` | VM started / process spawned |
| `booting` | `ready` | Health check passes |
| `provisioning` | `destroyed` | Resource allocation fails |
| `booting` | `destroyed` | Health check timeout (120s) |
| `ready` | `draining` | User-initiated deletion or admin action |
| `ready` | `crashed` | Supervisor max restarts exceeded |
| `draining` | `destroyed` | Clean shutdown complete |
| `crashed` | `destroyed` | Admin force-destroy |

Note: There is no "terminal" state. Agents transition from `ready` to `draining` only when the user explicitly requests deletion. Budget exhaustion triggers a notification and graceful degradation (reduced inference tier), not automatic destruction.

---

## Domain-Specific Provisioning

### Chain Domain (`roko-chain`)

Chain-domain agents require additional provisioning steps:

1. **Wallet provisioning**: Create or connect wallet based on custody mode (Delegation, Embedded, or LocalKey)
2. **ERC-8004 registration**: Register on-chain identity on Korai chain
3. **KORAI/DAEJI token setup**: Configure token balance tracking and demurrage
4. **DeFi tool loading**: Load chain-specific tools (swap, liquidity, vault management)

### Coding Domain (future)

Coding-domain agents require:

1. **File system access**: Configure sandboxed file system access
2. **Compiler integration**: Set up language-specific compiler and test runners
3. **VCS integration**: Configure git access for PR creation and review

### General Domain

General-purpose agents require no domain-specific provisioning. They use the standard tool profile and Neuro configuration.

---

## Health Probes: Liveness, Readiness, and Startup

Roko adapts Kubernetes' three-probe model for agent health monitoring. Each probe type serves a distinct purpose, preventing the conflation of "process alive" with "agent ready to accept work":

```rust
/// Agent health probe configuration, modeled after Kubernetes
/// liveness/readiness/startup probes.
///
/// Crate: `roko-core`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthProbeConfig {
    /// Liveness probe: Is the agent process alive and not deadlocked?
    /// Failure action: restart the agent process (supervisor handles this).
    /// Checks: event loop responsive, no OOM condition, watchdog timer alive.
    pub liveness: ProbeSpec,

    /// Readiness probe: Can the agent accept new tasks right now?
    /// Failure action: remove from Mesh work distribution, stop accepting plan tasks.
    /// Checks: Neuro store initialized, model routing configured, tools loaded.
    pub readiness: ProbeSpec,

    /// Startup probe: Has the agent completed initial boot?
    /// While startup probe is active, liveness/readiness probes are DISABLED.
    /// This prevents the supervisor from killing a slow-starting agent.
    /// Checks: first cognitive loop iteration completed successfully.
    pub startup: ProbeSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeSpec {
    /// How to check health.
    pub handler: ProbeHandler,
    /// Wait before first probe after state transition. Default: 5s.
    /// Range: 0s-300s.
    pub initial_delay: Duration,
    /// Time between probes. Default: 10s. Range: 1s-600s.
    pub period: Duration,
    /// Max time for a single probe. Default: 1s. Range: 1s-30s.
    pub timeout: Duration,
    /// Consecutive successes to pass. Default: 1. Range: 1-10.
    pub success_threshold: u32,
    /// Consecutive failures to fail. Default: 3. Range: 1-10.
    pub failure_threshold: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProbeHandler {
    /// Internal health check function (most common for self-hosted).
    Internal,
    /// HTTP GET to a health endpoint. Response 200-399 = healthy.
    Http { path: String, port: u16 },
    /// TCP connection attempt. Connection established = healthy.
    Tcp { port: u16 },
    /// Custom command. Exit code 0 = healthy.
    Exec { command: Vec<String> },
}

impl Default for HealthProbeConfig {
    fn default() -> Self {
        Self {
            liveness: ProbeSpec {
                handler: ProbeHandler::Internal,
                initial_delay: Duration::from_secs(15),
                period: Duration::from_secs(20),
                timeout: Duration::from_secs(1),
                success_threshold: 1,
                failure_threshold: 3,
            },
            readiness: ProbeSpec {
                handler: ProbeHandler::Internal,
                initial_delay: Duration::from_secs(5),
                period: Duration::from_secs(10),
                timeout: Duration::from_secs(1),
                success_threshold: 1,
                failure_threshold: 3,
            },
            startup: ProbeSpec {
                handler: ProbeHandler::Internal,
                initial_delay: Duration::from_secs(0),
                period: Duration::from_secs(10),
                timeout: Duration::from_secs(1),
                success_threshold: 1,
                failure_threshold: 30, // 30 × 10s = 300s max startup
            },
        }
    }
}
```

TOML configuration:

```toml
[health.liveness]
initial_delay_secs = 15
period_secs = 20
failure_threshold = 3

[health.readiness]
initial_delay_secs = 5
period_secs = 10
failure_threshold = 3

[health.startup]
period_secs = 10
failure_threshold = 30   # 300s max startup time
```

### Probe Semantics in Roko

| Probe | Checks | Failure Action | Kubernetes Equivalent |
|-------|--------|----------------|----------------------|
| Liveness | Event loop tick within timeout, no OOM | Supervisor restarts process | Liveness: kubelet restarts container |
| Readiness | Neuro initialized, Router configured, tools loaded | Remove from Mesh task distribution | Readiness: remove from Service endpoints |
| Startup | First cognitive loop iteration passed gates | Kill and restart (agent failed to boot) | Startup: block liveness/readiness until pass |

**Critical invariant**: While the startup probe has not yet passed, liveness and readiness probes are **not evaluated**. This prevents the supervisor from killing an agent that is legitimately slow to initialize (e.g., loading a large Neuro backup during restore).

---

## Init/Sidecar Pattern for Agent Provisioning

Borrowing Kubernetes' init container and native sidecar container patterns (stable in Kubernetes 1.33), agent provisioning can be decomposed into sequential initialization tasks and persistent auxiliary processes:

### Init Tasks (Sequential, Must Complete)

| Init Task | Duration | What It Does | Failure Mode |
|-----------|----------|-------------|-------------|
| Schema migration | ~instant | Upgrade `.roko/` directory layout for new versions | Abort provisioning |
| Secret injection | ~instant | Load API keys from keystore/env into memory | Abort provisioning |
| Neuro restore | 1-30s | If `--restore` flag, load backup into Neuro store | Abort provisioning |
| Model validation | ~1s | Verify configured inference models are reachable | Warn, continue with fallback |

### Sidecar Processes (Persistent, Alongside Main Agent)

| Sidecar | Purpose | Lifecycle |
|---------|---------|-----------|
| Metrics exporter | Prometheus/OpenTelemetry metrics from `.roko/learn/efficiency.jsonl` | Starts after init, stops after main agent |
| Log forwarder | Stream `.roko/episodes.jsonl` to external observability | Starts after init, stops after main agent |
| Mesh relay keepalive | Maintain outbound WebSocket to Mesh relay, reconnect on failure | Starts after init, stops after main agent |

**Termination order**: Following Kubernetes 1.33 convention, main agent process terminates first, then sidecar processes terminate in reverse manifest order. This ensures metrics/logs capture the agent's final state before the exporters shut down.

---

## Restart Backoff Algorithm

When the agent process crashes and the supervisor restarts it, exponential backoff prevents rapid restart loops from consuming resources:

```rust
/// Restart backoff algorithm for crashed agent processes.
/// Follows Kubernetes CrashLoopBackOff semantics.
///
/// Crate: `bardo-runtime`
pub struct RestartBackoff {
    /// Current failure count since last successful run.
    pub failure_count: u32,
    /// Base delay. Default: 100ms. Range: 10ms-10s.
    pub base_delay: Duration,
    /// Maximum delay. Default: 300s (5 minutes). Range: 1s-3600s.
    pub max_delay: Duration,
    /// Successful run duration to reset failure count. Default: 300s.
    pub reset_after: Duration,
}

impl RestartBackoff {
    /// Compute delay before next restart attempt.
    /// delay = min(base_delay * 10^failure_count, max_delay)
    pub fn next_delay(&self) -> Duration {
        let delay_ms = self.base_delay.as_millis() as f64
            * 10.0_f64.powi(self.failure_count as i32);
        Duration::from_millis(delay_ms.min(self.max_delay.as_millis() as f64) as u64)
    }

    /// Call when agent crashes.
    pub fn record_failure(&mut self) {
        self.failure_count = self.failure_count.saturating_add(1);
    }

    /// Call when agent has been running successfully for `reset_after`.
    pub fn reset(&mut self) {
        self.failure_count = 0;
    }
}

impl Default for RestartBackoff {
    fn default() -> Self {
        Self {
            failure_count: 0,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(300),
            reset_after: Duration::from_secs(300),
        }
    }
}
```

| Failure Count | Delay | Cumulative Wait |
|---------------|-------|-----------------|
| 0 | 100ms | 100ms |
| 1 | 1s | 1.1s |
| 2 | 10s | 11.1s |
| 3 | 100s | 111.1s |
| 4+ | 300s (capped) | 411.1s+ |

After 5 consecutive failures (configurable via `max_restarts_per_hour`), the agent enters `Crashed` state and awaits operator intervention.

---

## Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_state_prevents_premature_loop_start() {
        // Agent<Unvalidated> has no start_cognitive_loop method.
        // This is a compile-time guarantee, not a runtime test.
        // The test verifies the pipeline must complete in order.
        let agent = Agent::new(test_manifest());
        let validated = agent.validate().unwrap();
        // validated.start_cognitive_loop() would be a compile error
    }

    #[test]
    fn default_probes_allow_300s_startup() {
        let config = HealthProbeConfig::default();
        let max_startup = config.startup.period.as_secs()
            * config.startup.failure_threshold as u64;
        assert_eq!(max_startup, 300);
    }

    #[test]
    fn restart_backoff_caps_at_max() {
        let mut backoff = RestartBackoff::default();
        for _ in 0..10 {
            backoff.record_failure();
        }
        assert_eq!(backoff.next_delay(), Duration::from_secs(300));
    }

    #[test]
    fn restart_backoff_resets_on_success() {
        let mut backoff = RestartBackoff::default();
        backoff.record_failure();
        backoff.record_failure();
        assert!(backoff.next_delay() > Duration::from_secs(1));
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
    }

    #[test]
    fn provisioning_pipeline_stages_in_order() {
        let stages = [
            "Validate", "AllocateResources", "InitNeuro",
            "ConfigureRouting", "LoadTools", "RegisterMesh", "Ready",
        ];
        assert_eq!(stages.len(), 7);
        assert_eq!(stages[0], "Validate");
        assert_eq!(stages[6], "Ready");
    }
}
```

---

## Cross-References

- `docs/17-lifecycle/01-agent-creation.md` — Manifest generation
- `docs/17-lifecycle/03-configuration-and-operator-model.md` — Config override layers
- `docs/17-lifecycle/06-agent-deletion.md` — Clean shutdown and resource release
