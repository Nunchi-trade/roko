# 14 — roko-plugin SDK

> EventSource, FeedbackCollector, Integration traits. How to build domain plugins
> and extend Roko with new capabilities.


> **Implementation**: Specified

---

## Overview

The `roko-plugin` crate (planned) provides the SDK for extending Roko with new event sources,
feedback collectors, integrations, and domain plugins. It defines the traits that third-party
code implements to participate in the Roko runtime.

**Status:** Planned (trait definitions spec'd, crate not yet created)

**Target crate:** `crates/roko-plugin/`

**Design principle:** Plugins compose with the Synapse Architecture. A plugin implements one or
more traits, registers with the runtime, and participates in the cognitive loop without
modifying core code.

---

## Plugin Traits

### EventSource

Event sources produce events that trigger agent execution. They convert external signals
(cron ticks, file changes, webhooks, Slack messages, chain events) into Engrams that the
dispatch loop can route to agent templates.

```rust
#[async_trait]
pub trait EventSource: Send + Sync {
    /// Human-readable name for this event source.
    fn name(&self) -> &str;

    /// Start the event source. Emits events via the provided sender.
    async fn start(&self, sender: EventSender) -> Result<()>;

    /// Stop the event source gracefully.
    async fn stop(&self) -> Result<()>;

    /// Health check — is this event source still running?
    fn is_healthy(&self) -> bool;
}

/// Channel for emitting events from an event source.
pub struct EventSender {
    inner: tokio::sync::mpsc::Sender<SourceEvent>,
}

pub struct SourceEvent {
    /// Event kind (e.g., "scheduler.cron", "watcher.fs_change", "webhook.github.push").
    pub kind: String,
    /// Event payload as an Engram-compatible body.
    pub body: serde_json::Value,
    /// Source identifier (for routing to the correct subscription).
    pub source: String,
}
```

### FeedbackCollector

Feedback collectors gather outcome data from agent executions and feed it back into the
learning system. They bridge the VERIFY step of the cognitive loop with external outcome
sources.

```rust
#[async_trait]
pub trait FeedbackCollector: Send + Sync {
    /// Collector name.
    fn name(&self) -> &str;

    /// Collect feedback for a completed agent execution.
    async fn collect(
        &self,
        execution_id: &str,
        template_name: &str,
        result: &ExecutionResult,
    ) -> Result<Vec<FeedbackSignal>>;
}

pub struct FeedbackSignal {
    /// What was measured (e.g., "pr_merge_rate", "build_success", "user_satisfaction").
    pub metric: String,
    /// The measured value.
    pub value: f64,
    /// When the feedback was collected.
    pub timestamp: i64,
    /// Links to the execution that produced this feedback.
    pub execution_id: String,
}
```

Feedback signals are stored as Engrams and used by:
- **Prompt experiments** (A/B testing): Which prompt variant produces better outcomes?
- **CascadeRouter**: Which model tiers produce the best cost/quality tradeoff?
- **Adaptive gate thresholds**: Should gates be stricter or more lenient?

### Integration

The Integration trait is a higher-level abstraction that combines an EventSource with tool
registration:

```rust
#[async_trait]
pub trait Integration: Send + Sync {
    /// Integration name (used as MCP server namespace).
    fn name(&self) -> &str;

    /// Register tools provided by this integration.
    fn register_tools(&self, registry: &mut ToolRegistry);

    /// Event source (if this integration produces events).
    fn event_source(&self) -> Option<Box<dyn EventSource>>;

    /// Feedback collector (if this integration provides outcome feedback).
    fn feedback_collector(&self) -> Option<Box<dyn FeedbackCollector>>;

    /// Initialize the integration with configuration.
    async fn init(&self, config: &serde_json::Value) -> Result<()>;

    /// Shutdown the integration gracefully.
    async fn shutdown(&self) -> Result<()>;
}
```

---

## Domain Plugin Pattern

Adding a new domain to Roko follows the 8-step pattern from
`refactoring-prd/10-developer-guide.md`:

### Step 1: Create a Domain Crate

```bash
cargo new --lib crates/roko-domain-medical
```

```toml
# crates/roko-domain-medical/Cargo.toml
[dependencies]
roko-core = { path = "../roko-core" }
roko-plugin = { path = "../roko-plugin" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

### Step 2: Define Custom Kinds

```rust
use roko_core::kind::Kind;

// Use reverse-DNS prefix for custom kinds
pub const PATIENT_RECORD: Kind = Kind::Custom("medical.patient_record");
pub const DIAGNOSIS: Kind = Kind::Custom("medical.diagnosis");
pub const TREATMENT_PLAN: Kind = Kind::Custom("medical.treatment_plan");
pub const LAB_RESULT: Kind = Kind::Custom("medical.lab_result");
```

### Step 3: Implement Domain Gates

```rust
use roko_core::gate::{Gate, Verdict};

pub struct HipaaComplianceGate;

#[async_trait]
impl Gate for HipaaComplianceGate {
    async fn verify(&self, engram: &Engram) -> Result<Verdict> {
        // Check that output doesn't contain PII
        // Verify data handling complies with HIPAA
        // Return Pass/Fail with explanation
    }
}

pub struct MedicalAccuracyGate {
    /// Reference database for fact-checking.
    reference_db: Arc<MedicalReferenceDB>,
}

#[async_trait]
impl Gate for MedicalAccuracyGate {
    async fn verify(&self, engram: &Engram) -> Result<Verdict> {
        // Cross-reference claims against medical literature
        // Flag unsupported diagnoses or treatments
    }
}
```

### Step 4: Implement Domain Scorer

```rust
use roko_core::scorer::Scorer;

pub struct MedicalRelevanceScorer;

impl Scorer for MedicalRelevanceScorer {
    fn score(&self, engram: &Engram) -> Score {
        // Score based on medical relevance
        // Factor in patient context, condition severity, evidence quality
    }
}
```

### Step 5: Define 8D Somatic Strategy Space

```rust
pub struct MedicalSomaticSpace {
    // What dimensions matter for medical decision-making?
    pub diagnostic_confidence: f64,    // How certain is the diagnosis?
    pub treatment_urgency: f64,        // How urgent is intervention?
    pub patient_complexity: f64,       // How many comorbidities?
    pub evidence_strength: f64,        // How strong is the evidence?
    pub risk_tolerance: f64,           // Patient/provider risk preference
    pub resource_availability: f64,    // Available treatments/specialists
    pub regulatory_constraint: f64,    // HIPAA, insurance requirements
    pub patient_preference: f64,       // Patient's stated preferences
}
```

### Step 6: Add T0 Probes

```rust
pub fn medical_t0_probes() -> Vec<T0Probe> {
    vec![
        T0Probe::new("vitals_check", || {
            // Zero-LLM check: are patient vitals in normal range?
        }),
        T0Probe::new("allergy_check", || {
            // Zero-LLM check: does proposed treatment conflict with known allergies?
        }),
        T0Probe::new("interaction_check", || {
            // Zero-LLM check: drug interaction screening
        }),
    ]
}
```

### Step 7: Configure in roko.toml

```toml
[agent]
domain = "medical"
model = "claude-opus-4-6"
temperament = "conservative"  # medical domain defaults to conservative

[gates]
pipeline = ["hipaa_compliance", "medical_accuracy", "llm_judge"]

[substrate]
type = "encrypted_file"  # HIPAA requires encryption at rest
```

### Step 8: Register at Init

```rust
pub fn register_domain(engine: &mut Engine) {
    // Register custom kinds
    engine.register_kind(PATIENT_RECORD);
    engine.register_kind(DIAGNOSIS);

    // Register gates
    engine.register_gate(Box::new(HipaaComplianceGate));
    engine.register_gate(Box::new(MedicalAccuracyGate::new(reference_db)));

    // Register scorer
    engine.register_scorer(Box::new(MedicalRelevanceScorer));

    // Register T0 probes
    for probe in medical_t0_probes() {
        engine.register_probe(probe);
    }
}
```

**No core changes required.** The cognitive loop, Neuro knowledge tiers, Daimon affect, Dreams
consolidation, and C-Factor tracking all work automatically with the domain's trait
implementations.

---

## Plugin Loading Mechanisms

Three mechanisms for loading plugins, described in
`refactoring-prd/10-developer-guide.md` §6.4:

### 1. Cargo Workspace Members (Compile-Time)

Domain plugins implemented as workspace crates are compiled into the binary:

```toml
# Cargo.toml (workspace)
[workspace]
members = [
    "crates/roko-core",
    "crates/roko-std",
    "crates/roko-domain-chain",    # Chain domain plugin
    "crates/roko-domain-medical",  # Medical domain plugin
]
```

**Advantages:** Full type safety, no runtime overhead, IDE support.
**Disadvantages:** Requires recompilation for changes.

### 2. Config-Declared (Runtime)

Plugins declared in `roko.toml` are loaded at runtime:

```toml
[[plugins]]
name = "medical"
path = "./plugins/roko-domain-medical.so"  # Dynamic library
config = { reference_db = "/data/medical-refs.db" }
```

**Advantages:** No recompilation, hot-reloadable.
**Disadvantages:** Dynamic linking complexity, platform-specific.

### 3. MCP Tool Discovery (Runtime)

MCP servers are discovered and loaded at runtime via the MCP protocol:

```toml
[[agent.mcp_servers]]
name = "medical-tools"
command = "roko-mcp-medical"
env = { MEDICAL_DB = "/data/medical.db" }
```

**Advantages:** Language-agnostic, process isolation, easy deployment.
**Disadvantages:** IPC overhead (~1-5ms per call), no compile-time type checking.

---

## Plugin Lifecycle

```
Discovery → Validation → Initialization → Running → Shutdown
    |            |             |              |          |
    v            v             v              v          v
  Find       Validate      Call init()    Handle     Call shutdown()
  plugins    config/deps   with config    events     gracefully
```

### Validation

On load, plugins are validated:
- Config schema matches expected format
- Required dependencies are available
- Version compatibility with roko-core
- No conflicting tool names with existing plugins

### Health Monitoring

The runtime monitors plugin health:
- EventSource heartbeat (is it still producing events?)
- Error rate tracking (too many failures -> circuit breaker)
- Resource usage monitoring (memory, file descriptors)
- Automatic restart on crash (with exponential backoff)

---

## EventSource: built-in implementation patterns

Three built-in `EventSource` implementations ship with `roko-plugin`:

### Cron scheduler

```rust
/// Cron-based event source. Emits events on a schedule.
pub struct CronEventSource {
    /// Cron expression (e.g., "0 */5 * * * *" for every 5 minutes).
    schedule: cron::Schedule,
    /// Event kind to emit (e.g., "scheduler.cron.5min").
    event_kind: String,
    /// Payload template (static JSON injected into every event).
    payload: serde_json::Value,
    /// Cancellation token.
    cancel: tokio_util::sync::CancellationToken,
}

#[async_trait]
impl EventSource for CronEventSource {
    fn name(&self) -> &str { "cron" }

    async fn start(&self, sender: EventSender) -> Result<()> {
        loop {
            let next = self.schedule.upcoming(chrono::Utc).next()
                .ok_or_else(|| anyhow::anyhow!("No upcoming schedule"))?;
            let delay = (next - chrono::Utc::now()).to_std()?;

            tokio::select! {
                _ = tokio::time::sleep(delay) => {
                    sender.send(SourceEvent {
                        kind: self.event_kind.clone(),
                        body: self.payload.clone(),
                        source: "cron".into(),
                    }).await?;
                }
                _ = self.cancel.cancelled() => break,
            }
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.cancel.cancel();
        Ok(())
    }

    fn is_healthy(&self) -> bool {
        !self.cancel.is_cancelled()
    }
}
```

### Filesystem watcher

```rust
/// Watches directories for file changes, emits events on modification.
pub struct FsWatcherEventSource {
    watch_paths: Vec<PathBuf>,
    /// Debounce duration to coalesce rapid changes.
    debounce: Duration,
    cancel: tokio_util::sync::CancellationToken,
}

#[async_trait]
impl EventSource for FsWatcherEventSource {
    fn name(&self) -> &str { "fs_watcher" }

    async fn start(&self, sender: EventSender) -> Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.blocking_send(event);
        })?;

        for path in &self.watch_paths {
            watcher.watch(path, notify::RecursiveMode::Recursive)?;
        }

        // Debounce loop.
        let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
        loop {
            tokio::select! {
                Some(event) = rx.recv() => {
                    if let Ok(event) = event {
                        for path in event.paths {
                            pending.insert(path, Instant::now());
                        }
                    }
                }
                _ = tokio::time::sleep(self.debounce) => {
                    let now = Instant::now();
                    let ready: Vec<PathBuf> = pending.iter()
                        .filter(|(_, t)| now.duration_since(**t) >= self.debounce)
                        .map(|(p, _)| p.clone())
                        .collect();
                    for path in ready {
                        pending.remove(&path);
                        sender.send(SourceEvent {
                            kind: "watcher.fs_change".into(),
                            body: serde_json::json!({ "path": path.display().to_string() }),
                            source: "fs_watcher".into(),
                        }).await?;
                    }
                }
                _ = self.cancel.cancelled() => break,
            }
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.cancel.cancel();
        Ok(())
    }

    fn is_healthy(&self) -> bool { !self.cancel.is_cancelled() }
}
```

### Webhook receiver

```rust
/// HTTP webhook receiver. Listens for incoming POST requests
/// and converts them to SourceEvents.
pub struct WebhookEventSource {
    bind_addr: std::net::SocketAddr,
    /// HMAC secret for verifying webhook signatures (GitHub, Slack).
    hmac_secret: Option<String>,
    cancel: tokio_util::sync::CancellationToken,
}
```

---

## FeedbackCollector: full definition

```rust
use serde::{Deserialize, Serialize};

/// Result of an agent execution, passed to feedback collectors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Whether the execution succeeded.
    pub success: bool,
    /// Gate verdicts from the verification pipeline.
    pub gate_verdicts: Vec<GateVerdict>,
    /// Total tokens consumed.
    pub tokens_used: u64,
    /// Wall-clock duration.
    pub duration_ms: u64,
    /// Output artifacts (file paths, PR URLs, etc.).
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateVerdict {
    pub gate_name: String,
    pub passed: bool,
    pub confidence: f64,
    pub message: String,
}

#[async_trait]
pub trait FeedbackCollector: Send + Sync {
    /// Collector name.
    fn name(&self) -> &str;

    /// Collect feedback for a completed agent execution.
    /// Returns zero or more feedback signals. Zero signals means
    /// the collector has no feedback for this execution type.
    async fn collect(
        &self,
        execution_id: &str,
        template_name: &str,
        result: &ExecutionResult,
    ) -> Result<Vec<FeedbackSignal>>;

    /// Whether this collector applies to a given template.
    /// Default: applies to all templates.
    fn applies_to(&self, template_name: &str) -> bool {
        let _ = template_name;
        true
    }
}

/// A measured feedback signal. Stored as Engrams and consumed by
/// the learning subsystem (prompt experiments, cascade router, adaptive gates).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackSignal {
    /// What was measured (e.g., "pr_merge_rate", "build_success").
    pub metric: String,
    /// The measured value. Interpretation depends on metric:
    /// - Boolean metrics: 0.0 = false, 1.0 = true
    /// - Rate metrics: 0.0..1.0
    /// - Duration metrics: milliseconds
    /// - Cost metrics: USD
    pub value: f64,
    /// When the feedback was collected (Unix ms).
    pub timestamp: i64,
    /// Links to the execution that produced this feedback.
    pub execution_id: String,
    /// Optional metadata (e.g., PR number, test name).
    pub metadata: serde_json::Value,
}
```

---

## Integration: full definition

```rust
#[async_trait]
pub trait Integration: Send + Sync {
    /// Integration name (used as MCP server namespace).
    fn name(&self) -> &str;

    /// Semantic version of this integration.
    fn version(&self) -> &str;

    /// Register tools provided by this integration.
    /// Tools are namespaced: "{integration_name}.{tool_name}".
    fn register_tools(&self, registry: &mut ToolRegistry);

    /// Event source (if this integration produces events).
    fn event_source(&self) -> Option<Box<dyn EventSource>>;

    /// Feedback collector (if this integration provides outcome feedback).
    fn feedback_collector(&self) -> Option<Box<dyn FeedbackCollector>>;

    /// Initialize the integration with configuration.
    /// Called once at startup. Must not block.
    async fn init(&self, config: &serde_json::Value) -> Result<()>;

    /// Shutdown the integration gracefully.
    /// Called on runtime shutdown. Must complete within 10 seconds.
    async fn shutdown(&self) -> Result<()>;

    /// Report integration health.
    fn health(&self) -> IntegrationHealth;

    /// Configuration schema (JSON Schema) for validation.
    fn config_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    /// Declare dependencies on other integrations.
    /// The runtime ensures dependencies are initialized first.
    fn dependencies(&self) -> Vec<String> {
        vec![]
    }
}

#[derive(Debug, Clone)]
pub struct IntegrationHealth {
    pub status: HealthStatus,
    pub last_event_at: Option<i64>,
    pub error_count: u64,
    pub uptime_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String },
    Unhealthy { reason: String },
}
```

---

## Eight-step domain plugin automation

The 8-step pattern from the Domain Plugin Pattern section above is automated via `roko plugin init`:

```
roko plugin init <domain-name>
  |
  Step 1: Create crate at crates/roko-domain-<name>/
  Step 2: Generate Kind constants with reverse-DNS prefix
  Step 3: Scaffold Gate implementations (compile, domain-specific)
  Step 4: Scaffold Scorer implementation
  Step 5: Generate 8D somatic strategy space template
  Step 6: Generate T0 probe stubs
  Step 7: Add roko.toml domain configuration section
  Step 8: Register in workspace Cargo.toml + generate init code
```

Each step produces a compilable Rust file. The developer fills in the domain logic.

---

## Plugin validation and dependency checking

```rust
/// Validate a plugin before initialization.
pub fn validate_plugin(
    plugin: &dyn Integration,
    existing: &[Box<dyn Integration>],
    config: &serde_json::Value,
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    // Check version compatibility.
    if !semver_compatible(plugin.version(), ROKO_PLUGIN_API_VERSION) {
        errors.push(ValidationError::IncompatibleVersion {
            plugin: plugin.version().to_string(),
            required: ROKO_PLUGIN_API_VERSION.to_string(),
        });
    }

    // Check configuration schema.
    let schema = plugin.config_schema();
    if let Err(e) = validate_json_schema(config, &schema) {
        errors.push(ValidationError::InvalidConfig {
            details: e.to_string(),
        });
    }

    // Check dependencies are available.
    let available: HashSet<String> = existing.iter().map(|p| p.name().to_string()).collect();
    for dep in plugin.dependencies() {
        if !available.contains(&dep) {
            errors.push(ValidationError::MissingDependency {
                plugin: plugin.name().to_string(),
                dependency: dep,
            });
        }
    }

    // Check for tool name conflicts.
    // (Tool names are checked during register_tools, not here.)

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub enum ValidationError {
    IncompatibleVersion { plugin: String, required: String },
    InvalidConfig { details: String },
    MissingDependency { plugin: String, dependency: String },
    ToolNameConflict { tool: String, existing_plugin: String },
}
```

---

## Plugin health monitoring

```rust
/// Monitors plugin health with heartbeats, error rates, and auto-restart.
pub struct PluginHealthMonitor {
    /// Per-plugin health state.
    plugins: HashMap<String, PluginState>,
    /// Interval between health checks.
    check_interval: Duration,
    /// Error rate threshold for circuit breaker (errors per minute).
    error_rate_threshold: f64,
    /// Maximum restart attempts before giving up.
    max_restarts: u32,
}

struct PluginState {
    /// Last heartbeat timestamp.
    last_heartbeat: Instant,
    /// Heartbeat timeout. If exceeded, plugin is considered unhealthy.
    heartbeat_timeout: Duration,
    /// Sliding window error counter.
    error_window: Vec<Instant>,
    /// Resource usage snapshot.
    resource_usage: ResourceUsage,
    /// Restart count (resets after sustained healthy period).
    restart_count: u32,
    /// Backoff for restart attempts.
    restart_backoff: Duration,
}

struct ResourceUsage {
    memory_bytes: u64,
    open_file_descriptors: u32,
    cpu_percent: f32,
}

impl PluginHealthMonitor {
    /// Run a health check cycle. Returns plugins that need attention.
    pub fn check(&mut self) -> Vec<HealthAction> {
        let mut actions = Vec::new();
        let now = Instant::now();

        for (name, state) in &mut self.plugins {
            // Heartbeat check.
            if now.duration_since(state.last_heartbeat) > state.heartbeat_timeout {
                actions.push(HealthAction::Restart {
                    plugin: name.clone(),
                    reason: "Heartbeat timeout".into(),
                });
                continue;
            }

            // Error rate check.
            let one_min_ago = now - Duration::from_secs(60);
            state.error_window.retain(|t| *t > one_min_ago);
            let error_rate = state.error_window.len() as f64;
            if error_rate > self.error_rate_threshold {
                actions.push(HealthAction::CircuitBreak {
                    plugin: name.clone(),
                    error_rate,
                });
                continue;
            }

            // Resource usage check.
            if state.resource_usage.memory_bytes > 500_000_000 { // 500MB
                actions.push(HealthAction::Warn {
                    plugin: name.clone(),
                    reason: format!(
                        "High memory usage: {} MB",
                        state.resource_usage.memory_bytes / 1_000_000
                    ),
                });
            }
        }

        actions
    }
}

pub enum HealthAction {
    Restart { plugin: String, reason: String },
    CircuitBreak { plugin: String, error_rate: f64 },
    Warn { plugin: String, reason: String },
}
```

**Auto-restart with exponential backoff:**

| Restart attempt | Delay |
|----------------|-------|
| 1 | 1 second |
| 2 | 2 seconds |
| 3 | 4 seconds |
| 4 | 8 seconds |
| 5 (max) | Give up, emit alert |

After a plugin runs healthy for 5 minutes, the restart counter resets to 0.

**Configuration:**

```toml
[plugins.health]
check_interval_secs = 10        # Health check frequency. Range: 1..60.
heartbeat_timeout_secs = 30     # Max time without heartbeat. Range: 5..120.
error_rate_threshold = 10.0     # Errors per minute before circuit break. Range: 1.0..100.0.
max_restarts = 5                # Max restart attempts. Range: 1..20.
memory_limit_mb = 500           # Warning threshold. Range: 50..5000.
healthy_reset_secs = 300        # Sustained healthy time to reset restart counter.
```

### Loading mechanism details

**Workspace (compile-time):**
- Plugin crate added as workspace member in `Cargo.toml`
- `register_domain()` called from `roko-cli/src/main.rs` at startup
- Full type safety, inlined at compile time
- Requires `cargo build` for changes

**Config-declared (runtime):**
- Plugin shared library (`.so`/`.dylib`/`.dll`) loaded via `libloading`
- Exports a C-ABI function: `extern "C" fn roko_plugin_init() -> Box<dyn Integration>`
- Runtime validates ABI version before calling
- Hot-reloadable: replace `.so` file, send SIGHUP to runtime

**MCP (runtime):**
- Plugin runs as a separate process, communicates via JSON-RPC over stdio
- Discovered via `[[agent.mcp_servers]]` in `roko.toml`
- Process lifecycle managed by `bardo-runtime` ProcessSupervisor
- Language-agnostic: any language that speaks MCP works

---

## Mechanism 4: WASM Plugin System

WASM plugins provide sandboxed, language-agnostic plugin execution with capability-based
security. This is the recommended mechanism for untrusted third-party tools.

### Architecture

WASM plugins are compiled to `.wasm` modules and loaded at runtime via Wasmtime (Bytecode
Alliance) or Extism (which wraps Wasmtime). Each plugin runs in an isolated linear memory
space with explicit capability grants — no ambient authority.

```
Host (roko runtime)
    │
    ├── Wasmtime Engine (shared across plugins)
    │   ├── Module Cache (compiled .wasm → native code)
    │   └── Epoch Interruption (wall-clock timeout)
    │
    ├── Plugin Instance A
    │   ├── Linear Memory (256MB max)
    │   ├── WASI Capabilities: [read: /data/*, net: api.example.com]
    │   └── Fuel Metering: 10M instructions
    │
    └── Plugin Instance B
        ├── Linear Memory (256MB max)
        ├── WASI Capabilities: [none]
        └── Fuel Metering: 5M instructions
```

### WIT (WebAssembly Interface Types) Contract

The tool interface is defined using WIT, the Component Model's IDL:

```wit
// wit/roko-tool.wit
package roko:tool@0.1.0;

interface types {
    record tool-input {
        name: string,
        arguments: string,  // JSON-encoded
        context: option<string>,  // Optional context JSON
    }

    record tool-output {
        content: string,  // JSON-encoded result
        is-error: bool,
        metadata: option<string>,  // Optional metadata JSON
    }

    record tool-definition {
        name: string,
        description: string,
        input-schema: string,  // JSON Schema
        version: string,       // semver
        read-only: bool,
    }
}

interface tool {
    use types.{tool-input, tool-output, tool-definition};

    /// Return the tool definitions this plugin provides.
    list-tools: func() -> list<tool-definition>;

    /// Execute a tool with the given input.
    execute: func(input: tool-input) -> tool-output;
}

world roko-plugin {
    export tool;

    /// Optional: import host functions for logging and metrics.
    import roko:host/log;
    import roko:host/metrics;
}
```

### WASM Plugin Loader

```rust
use wasmtime::{Engine, Module, Store, Linker, Config as WasmConfig};
use wasmtime_wasi::preview2::WasiCtxBuilder;

/// WASM plugin loader with capability-based security.
pub struct WasmPluginLoader {
    engine: Engine,
    /// Compiled module cache (avoid recompilation).
    module_cache: HashMap<PathBuf, Module>,
}

impl WasmPluginLoader {
    pub fn new() -> Result<Self> {
        let mut config = WasmConfig::new();
        config.consume_fuel(true);          // Enable fuel metering
        config.epoch_interruption(true);    // Enable timeout
        config.wasm_component_model(true);  // Enable Component Model
        config.cache_config_load_default()?; // Enable module cache

        Ok(Self {
            engine: Engine::new(&config)?,
            module_cache: HashMap::new(),
        })
    }

    /// Load a WASM plugin with specific capability grants.
    pub async fn load(
        &mut self,
        wasm_path: &Path,
        permissions: &WasmPermissions,
    ) -> Result<WasmPlugin> {
        // Compile or retrieve from cache
        let module = match self.module_cache.get(wasm_path) {
            Some(m) => m.clone(),
            None => {
                let bytes = tokio::fs::read(wasm_path).await?;
                let module = Module::new(&self.engine, &bytes)?;
                self.module_cache.insert(wasm_path.to_path_buf(), module.clone());
                module
            }
        };

        // Build WASI context with capability grants
        let mut wasi_builder = WasiCtxBuilder::new();

        // Filesystem access: only explicitly granted paths
        for path in &permissions.read_paths {
            wasi_builder.preopened_dir(
                path, path.to_str().unwrap(),
                wasmtime_wasi::DirPerms::READ,
                wasmtime_wasi::FilePerms::READ,
            )?;
        }
        for path in &permissions.write_paths {
            wasi_builder.preopened_dir(
                path, path.to_str().unwrap(),
                wasmtime_wasi::DirPerms::all(),
                wasmtime_wasi::FilePerms::all(),
            )?;
        }

        // Network access: only explicitly granted hosts
        if !permissions.allowed_hosts.is_empty() {
            wasi_builder.allow_ip_name_lookup(true);
            // Host allowlist enforced at the WASI layer
        }

        let wasi = wasi_builder.build();
        let mut store = Store::new(&self.engine, wasi);

        // Set resource limits
        store.set_fuel(permissions.fuel_limit)?;
        store.epoch_deadline_trap();

        // Start epoch timer for wall-clock timeout
        let engine = self.engine.clone();
        let timeout = permissions.timeout;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            engine.increment_epoch();
        });

        // Instantiate
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::preview2::command::add_to_linker(&mut linker)?;
        let instance = linker.instantiate(&mut store, &module)?;

        Ok(WasmPlugin { store, instance, module })
    }
}

/// Capability grants for a WASM plugin.
pub struct WasmPermissions {
    /// Filesystem paths this plugin can read.
    pub read_paths: Vec<PathBuf>,
    /// Filesystem paths this plugin can write.
    pub write_paths: Vec<PathBuf>,
    /// HTTP hosts this plugin can contact.
    pub allowed_hosts: Vec<String>,
    /// Maximum WASM instructions (fuel).
    pub fuel_limit: u64,       // default: 10_000_000 (~100ms)
    /// Wall-clock timeout.
    pub timeout: Duration,     // default: 5s
    /// Maximum linear memory.
    pub max_memory: usize,     // default: 256MB
}

impl Default for WasmPermissions {
    fn default() -> Self {
        Self {
            read_paths: vec![],
            write_paths: vec![],
            allowed_hosts: vec![],
            fuel_limit: 10_000_000,
            timeout: Duration::from_secs(5),
            max_memory: 256 * 1024 * 1024,
        }
    }
}
```

### Extism Integration (Simplified Path)

For simpler plugin development, the Extism framework wraps Wasmtime with a higher-level API:

```rust
use extism::{Plugin, Manifest, Wasm};

/// Load a WASM tool plugin via Extism.
pub fn load_extism_plugin(
    wasm_path: &Path,
    permissions: &WasmPermissions,
) -> Result<Plugin> {
    let manifest = Manifest::new([Wasm::file(wasm_path)])
        .with_timeout(permissions.timeout);

    // Add allowed hosts for HTTP access
    let manifest = permissions.allowed_hosts.iter()
        .fold(manifest, |m, host| m.with_allowed_host(host));

    let plugin = Plugin::new(&manifest, [], true)?; // WASI enabled
    Ok(plugin)
}

/// Call a tool in an Extism plugin.
pub async fn call_extism_tool(
    plugin: &mut Plugin,
    input: &ToolInput,
) -> Result<ToolOutput> {
    let input_json = serde_json::to_string(input)?;
    let output_json: String = plugin.call("execute", &input_json)?;
    let output: ToolOutput = serde_json::from_str(&output_json)?;
    Ok(output)
}
```

### Writing a WASM Plugin (Rust PDK)

```rust
// my-tool-plugin/src/lib.rs
use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Input {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct Output {
    content: String,
    is_error: bool,
}

#[plugin_fn]
pub fn execute(input: String) -> FnResult<String> {
    let input: Input = serde_json::from_str(&input)?;
    let args: serde_json::Value = serde_json::from_str(&input.arguments)?;

    let result = match input.name.as_str() {
        "my_custom_tool" => {
            let query = args["query"].as_str().unwrap_or("");
            let processed = process_query(query);
            Output { content: serde_json::to_string(&processed)?, is_error: false }
        }
        _ => Output {
            content: format!("Unknown tool: {}", input.name),
            is_error: true,
        }
    };

    Ok(serde_json::to_string(&result)?)
}
```

Build and deploy:
```bash
cargo build --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/my_tool_plugin.wasm plugins/
```

### WASM Plugin Configuration

```toml
# roko.toml
[[plugins.wasm]]
name = "custom-analytics"
path = "plugins/analytics.wasm"
version = "1.2.0"

[plugins.wasm.permissions]
read_paths = ["/data/analytics"]
write_paths = []
allowed_hosts = ["api.analytics.com"]
fuel_limit = 50_000_000        # ~500ms of computation
timeout_secs = 10
max_memory_mb = 128
```

### Security Properties

| Property | Guarantee |
|---|---|
| Memory isolation | WASM linear memory — plugin cannot read host memory |
| No ambient authority | Only explicitly granted capabilities (WASI) |
| Computation limits | Fuel metering prevents infinite loops |
| Time limits | Epoch interruption enforces wall-clock timeout |
| Filesystem isolation | Only pre-opened directories accessible |
| Network isolation | Only allowlisted hosts reachable |
| No raw syscalls | WASI abstraction prevents direct kernel access |

---

## Test Criteria

- `CronEventSource` emits events at the correct schedule interval
- `FsWatcherEventSource` debounces rapid file changes into a single event
- `FeedbackCollector::collect()` returns feedback signals with correct execution_id linkage
- `Integration::init()` fails with a clear error when config doesn't match `config_schema()`
- `validate_plugin()` rejects plugins with missing dependencies
- `validate_plugin()` rejects plugins with incompatible versions
- `PluginHealthMonitor::check()` triggers `Restart` after heartbeat timeout
- `PluginHealthMonitor::check()` triggers `CircuitBreak` when error rate exceeds threshold
- Auto-restart backoff doubles on each attempt and gives up at `max_restarts`
- Restart counter resets after `healthy_reset_secs` of healthy operation
- WASM plugin with no filesystem permissions cannot read any files
- WASM plugin with no network permissions cannot make HTTP requests
- WASM plugin exceeding fuel limit is terminated with `FuelExhausted` error
- WASM plugin exceeding wall-clock timeout is terminated with `EpochInterruption`
- WASM plugin returning write requests are validated through safety hook chain
- WIT interface correctly serializes/deserializes tool-input and tool-output

---

## References

- **Extism** — Cross-language WASM plugin framework.
  [docs](https://extism.org/docs/concepts/plug-in-system/)
- **Wasmtime** (Bytecode Alliance) — Production WASM runtime for Rust.
  [docs](https://docs.wasmtime.dev/)
- **WASI Preview 2 Component Model** — Capability-based system interface.
  [spec](https://component-model.bytecodealliance.org/)
- **WIT (WebAssembly Interface Types)** — IDL for component interfaces.
  [docs](https://component-model.bytecodealliance.org/design/wit.html)
- **Sy Brand, "Building Native Plugin Systems with WebAssembly Components"** (2024) —
  Practical guide to WASM plugins in Rust.
  [blog](https://tartanllama.xyz/posts/wasm-plugins/)
- **Capsicum: Practical Capabilities for UNIX** (Watson et al., 2010) — Capability-based
  security model that inspired WASI.
  [paper](https://papers.freebsd.org/2010/rwatson-capsicum.files/rwatson-capsicum-paper.pdf)
