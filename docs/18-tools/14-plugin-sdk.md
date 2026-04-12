# 14 — roko-plugin SDK

> EventSource, FeedbackCollector, Integration traits. How to build domain plugins
> and extend Roko with new capabilities.

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
- Error rate tracking (too many failures → circuit breaker)
- Resource usage monitoring (memory, file descriptors)
- Automatic restart on crash (with exponential backoff)
