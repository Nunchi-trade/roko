# 03 — Plugin SPI as Extension

> The five-tier plugin system as Extension specialization with graduated sandboxing.
> Each tier corresponds to a different power level: data-only Signals through
> WASM Cells with capability sandbox and fuel metering.

**Parent spec**: [12-EXTENSIONS.md](../../unified/12-EXTENSIONS.md), [14-TOOLS.md](../../unified/14-TOOLS.md)

---

## 1. Core Insight

Roko's extension system (8 layers, 22 hooks, CaMeL IFC) provides the mechanism for intercepting
and enriching Cell behavior. The **plugin SPI** is the user-facing surface of this system — a
graduated ladder of five tiers, each with a fixed power envelope, discovery path, and sandbox.

The key constraint: **the loader selects the lowest tier that satisfies the requested capability.**
A prompt template is Tier 1 (pure data). A subprocess wrapper is Tier 3 (declarative). Native
trait implementations are Tier 4. WASM logic with bounded authority is Tier 5. You never use a
higher tier when a lower one suffices.

In unified vocabulary:
- A plugin manifest is a **Signal** (content-addressed, versioned, schema-typed)
- A loaded plugin is a **Cell** (with protocol conformance declared in the manifest)
- The discovery-validate-sandbox-instantiate flow is a **Pipeline pattern**
- Plugin health is a **Lens Cell** (Observe protocol, read-only monitoring)
- The WASM sandbox host imports are a restricted **Store + Bus interface**

---

## 2. Five Tiers as Extension Power Levels

Each tier adds execution capability while the sandbox tightens proportionally:

| Tier | Extension Shape | Cell Analogy | Sandbox | Example |
|---|---|---|---|---|
| 1 | Prompt/template bundle | Signal (pure data, no execution) | None | Role prompts, system overlays |
| 2 | Configuration profile | Signal (parameterizes Graphs) | None | Team presets, domain bundles |
| 3 | Declarative tool / MCP | Connect Cell (subprocess proxy) | Tool safety layer | Shell wrapper, MCP server |
| 4 | Native trait implementation | Any protocol Cell (in-process) | ABI bridge + policy | Custom Store, Gate, Router |
| 5 | WASM sandboxed extension | Any protocol Cell (capability-bound) | Fuel + memory + imports | Untrusted third-party logic |

### Power Progression

```
Tier 1: Can inject text into prompts
Tier 2: Can parameterize Graph behavior via config Signals
Tier 3: Can execute external processes and expose their results as Cells
Tier 4: Can implement kernel protocols (Store, Verify, Route, etc.) natively
Tier 5: Can run arbitrary logic with bounded resources and restricted host access
```

Each step up requires more trust from the platform and more isolation from the runtime.

---

## 3. Manifest as Signal

Every plugin declares itself via a manifest file. The manifest is a **Signal** in the unified
sense: it has content (the declaration), it is versioned (semver), it can be content-addressed
(hash of manifest), and it carries provenance (who published it, when).

```toml
# Manifest shape — same structure across all tiers
id = "org.example.cargo-udeps"
version = "0.1.0"
tier = 3
kind = "tool"
description = "Find unused dependencies in a Rust workspace"

[entrypoint]
type = "subprocess"
command = "cargo"
args = ["+nightly", "udeps", "--workspace"]
cwd = "{{workspace_root}}"

[capabilities]
provides = ["Tool"]

[permissions]
roles = ["researcher", "implementer"]
network = false
files_read = ["{{workspace_root}}/**"]
files_write = []
bus_subscribe = ["gate.verdict.emitted"]
bus_publish = ["tool.audit.*"]
timeout_ms = 300000
```

### Manifest Fields

| Field | Purpose | Tier Requirement |
|---|---|---|
| `id` | Globally unique identifier (reverse-domain) | All tiers |
| `version` | Semantic version | All tiers |
| `tier` | Power level (1-5) | All tiers |
| `kind` | What it provides (prompt, profile, tool, mcp, native, wasm) | All tiers |
| `entrypoint` | How to load/execute | Tier 3-5 |
| `capabilities` | What Cell protocols it implements | Tier 3-5 |
| `permissions` | What resources it needs access to | Tier 2-5 |

---

## 4. Tier 1 and 2: Data-Only Signals

### Tier 1 — Prompt and Template Bundles

Pure text injected into the Compose protocol output. No code executes. The loader reads
Markdown or front-matter files and merges them into the appropriate prompt surface.

```
plugins/prompts/compliance-reviewer/
  manifest.toml          # Tier 1, kind = "prompt"
  prompt.md              # System prompt content
  overlays/              # Context-conditional overlays
    reviewing.md
    planning.md
```

In unified terms: a Tier 1 plugin creates a Signal of Kind `prompt-template` that the
Compose Cell includes when assembling context.

### Tier 2 — Configuration Profiles

Opinionated presets that parameterize Graph behavior. Still pure data, but with structural
implications: which Cells are available, what thresholds apply, what gates run.

```
plugins/profiles/rust-oss/
  manifest.toml          # Tier 2, kind = "profile"
  profile.toml           # Domain-specific settings
```

A Tier 2 bundle packages:
- **Tools** to expose (by category or name)
- **Roles** that should exist at boot
- **Gates** that wrap tool execution
- **Heuristics** and starter prompts
- **Typed context schema** that downstream tools expect

In unified terms: a Tier 2 plugin is a Signal of Kind `config-profile` that the runtime merges
into Graph parameterization at boot time.

### Merge Semantics for Tier 2

Multiple profiles can be active simultaneously. Resolution rules:

```rust
/// Profile composition rules when multiple bundles are active.
pub struct ProfileMerge {
    /// Tools: union of all bundles' tool sets
    tools: MergeStrategy::Union,
    /// Roles: union with collision warning (same role from two bundles)
    roles: MergeStrategy::UnionWarnCollision,
    /// Gates: stack unless explicitly scoped to a profile name
    gates: MergeStrategy::Stack,
    /// Heuristics: coexist, routed by situation fit
    heuristics: MergeStrategy::Coexist,
    /// Key conflicts: profile priority resolves (explicit ordering)
    overrides: MergeStrategy::PriorityOrder,
}
```

---

## 5. Tier 3: Declarative Cells via Manifests

Tier 3 is the biggest extensibility win. A third party ships a useful tool without writing Rust —
just a manifest pointing to a subprocess or MCP server.

### Subprocess Pattern

```toml
id = "org.example.cargo-udeps"
version = "0.1.0"
tier = 3
kind = "tool"

[entrypoint]
type = "subprocess"
command = "cargo"
args = ["+nightly", "udeps", "--workspace"]
cwd = "{{workspace_root}}"
```

The loader:
1. Parses the manifest
2. Validates permissions against policy
3. Wraps the subprocess as a Connect Cell
4. Registers it in the merged tool registry
5. Applies the standard safety hook chain (all 7 Verify Cells)

### MCP Server Pattern

```toml
id = "org.example.github-search"
version = "1.4.0"
tier = 3
kind = "mcp"

[entrypoint]
type = "mcp"
command = "roko-mcp-github"
args = ["--token-env", "GITHUB_TOKEN"]
```

The loader spawns the MCP server, discovers tools via `tools/list`, and registers each
as a Connect Cell (see [02-mcp-as-connect-protocol.md](02-mcp-as-connect-protocol.md)).

### Safety Envelope for Tier 3

Tier 3 extensions operate within the existing tool safety layer:
- Role allowlists (only declared roles can invoke)
- File scope bounds (only declared paths accessible)
- Network bounds (if `network = false`, no outbound connections)
- Timeout enforcement (kill process after `timeout_ms`)
- Bus topic restrictions (can only subscribe/publish to declared topics)

---

## 6. Tier 4: Native Protocol Implementations

Tier 4 is for extensions that must participate directly in kernel protocols — implementing
Store, Verify, Score, Route, Compose, or React.

### Use Cases

- A custom vector Store implementation (e.g., Qdrant-backed)
- A domain-specific Gate (medical compliance checker)
- A routing strategy (custom EFE variant)
- A scoring axis (domain-specific quality metric)

### ABI Bridge

Rust does not guarantee a stable plugin ABI. Tier 4 uses a narrow bridge crate:

```toml
id = "org.example.medical-gate"
version = "0.2.1"
tier = 4
kind = "native"

[entrypoint]
type = "cdylib"
path = "./plugins/native/medical_gate.so"
abi = "roko-extension-abi/1"

[capabilities]
provides = ["Gate"]
```

The loader:
1. Checks ABI version compatibility
2. Loads the shared library via `dlopen`
3. Resolves the entry point symbol
4. Validates the declared capabilities against the actual exports
5. Registers the implementation with the relevant protocol subsystem
6. Wraps in policy enforcement (same as built-in Cells)

### Native Extension Contract

```rust
/// The stable ABI contract for Tier 4 extensions.
/// This is the only interface a native extension sees.
#[repr(C)]
pub struct ExtensionAbi {
    pub version: u32,  // Must match loader's expected version
    pub init: unsafe extern "C" fn(*const ExtensionConfig) -> *mut dyn Extension,
    pub shutdown: unsafe extern "C" fn(*mut dyn Extension),
}
```

---

## 7. Tier 5: WASM Cells with Capability Sandbox

Tier 5 is the safest path for untrusted third-party logic. A WASM module runs inside a
capability sandbox with explicit host imports and bounded resources.

### Resource Limits

| Resource | Limit | Mechanism |
|---|---|---|
| CPU | Fuel metering (10M units ~ 100ms) | Wasmtime fuel system |
| Wall clock | 5 second timeout | Epoch interruption (background task) |
| Memory | 256MB maximum | Wasmtime memory limit |
| Bus calls | Rate-limited per window | Host import accounting |
| Store calls | Rate-limited per window | Host import accounting |

### Host Import Surface (Intentionally Minimal)

The WASM module sees only six host functions — a restricted Store + Bus interface:

```rust
/// The complete host import surface for Tier 5 WASM extensions.
/// Nothing else is accessible. No filesystem. No network. No keys.
pub mod host {
    /// Publish a Pulse on Bus (restricted to declared topics).
    pub fn bus_publish(pulse_ptr: u32, pulse_len: u32) -> i64;

    /// Subscribe to a Bus topic (restricted to declared topics).
    pub fn bus_subscribe(filter_ptr: u32, filter_len: u32) -> i32;

    /// Receive next Pulse from a subscription handle.
    pub fn bus_recv(handle: i32, buf_ptr: u32, buf_cap: u32) -> i64;

    /// Query Store for similar Signals (HDC similarity search).
    pub fn substrate_query_similar(
        fp_ptr: u32, radius_bits: u32, limit: u32,
        out_ptr: u32, out_cap: u32
    ) -> i64;

    /// Emit a log message (level: 0=trace, 1=debug, 2=info, 3=warn, 4=error).
    pub fn log(level: u32, msg_ptr: u32, msg_len: u32);

    /// Current time in milliseconds (monotonic, no wall-clock manipulation).
    pub fn now_ms() -> i64;
}
```

### Sandbox Security Properties

- **No filesystem access** — cannot read or write host files
- **No network access** — cannot make HTTP requests or open sockets
- **No credential access** — cannot access signing keys or API tokens
- **No ambient authority** — every capability must be explicitly granted in manifest
- **Deterministic termination** — fuel exhaustion or epoch timeout guarantees halt
- **Write operations mediated** — any write must be returned as a request that the host
  validates through the normal safety hook Pipeline

### WASM Extension Manifest

```toml
id = "com.vendor.sentiment-scorer"
version = "1.0.0"
tier = 5
kind = "wasm"

[entrypoint]
type = "wasm"
module = "./plugins/wasm/sentiment_scorer.wasm"

[capabilities]
provides = ["Scorer"]

[permissions]
bus_subscribe = ["signal.graduated"]
bus_publish = ["score.sentiment.*"]
substrate_read = true
substrate_write = false
memory_mb = 64
fuel = 10_000_000
timeout_ms = 5000
```

---

## 8. Discovery-First Loading Pipeline

Plugin loading follows a Pipeline pattern — a linear sequence of Cells that can reject at any step:

```
[1] Discover ─→ [2] Parse ─→ [3] Validate ─→ [4] Sandbox ─→ [5] Instantiate ─→ [6] Register
       │              │             │               │                │                │
       v              v             v               v                v                v
  Scan plugin    Parse TOML   Check tier +      Select tier-    Load module,       Add to
  roots for      manifest,    capabilities +    specific        create Cell        runtime
  manifest.toml  resolve      permissions       sandbox         instance           registry
  files          entrypoint   against policy
```

### Pipeline Rejection Points

| Step | Rejection Cause | Recovery |
|---|---|---|
| Parse | Invalid TOML, missing required fields | Fix manifest |
| Validate | Tier mismatch, permission exceeds policy | Downgrade tier or reduce permissions |
| Sandbox | Platform lacks WASM support, ABI version mismatch | Update platform or bridge |
| Instantiate | Module load failure, entry point missing | Fix module |
| Register | Name collision with higher-precedence Cell | Rename or scope |

---

## 9. Health as Lens Cell

Every loaded plugin is monitored by a health Lens Cell (Observe protocol):

```rust
/// Plugin health observable via Lens Cell.
pub struct PluginHealth {
    pub id: String,
    pub tier: Tier,
    pub status: PluginStatus,
    pub metrics: PluginMetrics,
}

pub enum PluginStatus {
    Healthy,
    Degraded { reason: String },
    Failed { error: String, since: DateTime<Utc> },
    Disabled { by: String },
}

pub struct PluginMetrics {
    pub invocations: u64,
    pub errors: u64,
    pub avg_latency_ms: f64,
    pub fuel_consumed: u64,      // Tier 5 only
    pub memory_peak_mb: f64,     // Tier 5 only
    pub last_invocation: Option<DateTime<Utc>>,
}
```

The health Lens publishes Pulses on `plugin.{id}.health` at regular intervals. React Cells
can auto-disable plugins that exceed error thresholds or consume excessive resources.

---

## What This Enables

1. **Progressive extensibility** — start with a prompt file (Tier 1), graduate to config
   (Tier 2), then declarative tools (Tier 3), and only reach for native/WASM when necessary.
2. **Safety by default** — each tier has a fixed sandbox that cannot be exceeded. Tier 3
   cannot access the filesystem beyond declared paths. Tier 5 cannot access the network at all.
3. **Third-party ecosystem** — Tier 3 and 5 require no Rust knowledge. Declarative tools are
   TOML manifests pointing at shell commands. WASM modules can be compiled from any language.
4. **Domain composability** — Tier 2 bundles package coherent domain configurations (tools +
   gates + heuristics + templates) as installable profiles.
5. **Auditable trust** — `roko plugin audit` reports every extension's permissions, sandbox
   requirements, and policy conformance.

---

## Feedback Loops

- **Plugin health → auto-disable**: Lens Cell monitors error rate → React Cell disables plugin
  when threshold exceeded → alert Pulse fired → operator notified.
- **Invocation patterns → discovery suggestions**: frequently-used Tier 3 tools that cause
  latency → suggest Tier 4 native reimplementation for performance.
- **WASM fuel consumption → budget adjustment**: actual fuel use vs declared limit feeds
  adaptive fuel allocation for future invocations.
- **Profile composition conflicts → surface via Lens**: when two Tier 2 bundles declare
  conflicting settings, the health Lens surfaces the conflict for operator resolution.

---

## Open Questions

1. **Plugin registry / marketplace** — should there be a centralized registry for discovering
   published plugins, or is filesystem-based discovery sufficient?
2. **Tier 4 ABI stability** — Rust has no stable ABI. Is `cdylib` + C FFI the right bridge,
   or should Tier 4 be workspace-local crates only?
3. **WASM module signing** — should Tier 5 modules require cryptographic signatures for
   provenance verification before loading?
4. **Hot-reload for Tier 1/2** — can prompt and profile bundles be reloaded without restarting
   the agent? (Manifest watcher + registry refresh.)
5. **Tier escalation** — if a Tier 3 plugin needs Bus access, does it automatically become
   Tier 5, or is there a Tier 3.5 (declarative + Bus permissions)?

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Extension trait (SPI surface) | `crates/roko-core/src/extension.rs` | Partial |
| Manifest parser (TOML schema) | `crates/roko-core/src/extension.rs` | Planned |
| Tier 1 loader (prompt bundles) | `crates/roko-compose/src/` | Planned |
| Tier 2 loader (profile bundles) | `crates/roko-core/src/config/` | Planned |
| Tier 3 loader (subprocess tools) | `crates/roko-agent/src/mcp/` | Partial (via MCP) |
| Tier 4 loader (native ABI bridge) | (not yet created) | Aspirational |
| Tier 5 loader (WASM sandbox) | `crates/roko-std/src/tool/` | Planned |
| Plugin health Lens Cell | `crates/roko-conductor/src/` | Planned |
| `roko plugin` CLI commands | `crates/roko-cli/src/` | Planned |
| Discovery-first Pipeline | `crates/roko-core/src/extension.rs` | Planned |
