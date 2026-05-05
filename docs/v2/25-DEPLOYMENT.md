# 25 — Deployment

> Local development, cloud deployment, daemon lifecycle, WASM packaging, brain export/import with Merkle-CRDT sync, secrets management, and worker mode. The same binary runs everywhere; configuration selects the scale. Every deployment artifact is a Graph of Cells processing Signals through Bus and Store.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse duality), [02-CELL](02-CELL.md) (Cell and protocol conformance), [03-GRAPH](03-GRAPH.md) (Graph composition), [05-AGENT](05-AGENT.md) (Agent lifecycle, vitality), [15-TELEMETRY](15-TELEMETRY.md) (Lens system, StateHub projections), [16-SECURITY](16-SECURITY.md) (sandboxing by tier, CaMeL IFC), [19-CONFIG](19-CONFIG.md) (5-tier SPI, Tier 4 WASM)

---

## 1. Three Scaling Tiers

All tiers use the same binary. The difference is configuration: environment variables, execution mode, whether a relay is involved, and the Bus topology.

| Tier | Users | Deployment | Agents | Relay | Bus Topology |
|---|---|---|---|---|---|
| **Solo developer** | 1 | `roko serve` on localhost | 1-10 in-process | None | **In-process Bus**: all Pulses flow through a single `tokio::broadcast` ring buffer within the process. Zero serialization overhead, sub-microsecond delivery. |
| **Small team** | 2-10 | Railway or Fly.io single instance | 10-50 in-process | Optional | **Relay-backed Bus**: local Bus for in-process Agents, relay bridge for cross-instance Pulses. Local delivery stays fast; cross-instance delivery adds relay hop (~5ms). |
| **Production** | 10+ | Railway/Fly multi-instance | 50+ in-process + isolated | Required | **Relay-backed Bus** (required): all cross-instance Pulses route through relay. Isolated Agents connect their local Bus to relay via WebSocket bridge. Topic partitioning prevents broadcast storms. |

Each tier is a configuration of the same Engine interpreting the same Graphs. Scaling adds relay connectivity and isolated execution — it does not change the computation model. The Bus abstraction hides topology differences: a Cell publishing a Pulse does not know (or care) whether subscribers are in-process or across a relay.

### 1.1 Tier Selection Graph

Tier selection is itself a Graph of Cells. The `tier-advisor` Graph evaluates workspace configuration and recommends the appropriate tier:

```toml
[graph]
name = "tier-advisor"
description = "Recommend deployment tier based on workspace state"

[[nodes]]
id = "count-agents"
cell = "rule-scorer@^1"
[nodes.params]
rules = [
  { field = "agent_count", op = "lte", value = 10, dimension = "relevance", weight = 1.0 },
]

[[nodes]]
id = "check-relay"
cell = "rule-router@^1"
[nodes.params]
rules = [
  { condition = "agent_count <= 10 && !relay_configured", select = "solo" },
  { condition = "agent_count <= 50 && relay_configured", select = "team" },
  { condition = "true", select = "production" },
]

[[edges]]
from = "count-agents"
to = "check-relay"
```

```rust
pub struct TierRecommendation {
    pub tier: ScalingTier,                 // Solo | Team | Production
    pub bus_topology: BusTopology,         // InProcess | RelayBacked
    pub relay_required: bool,
    pub isolated_execution_recommended: bool,
    pub reason: String,
}

pub enum BusTopology {
    /// All Pulses in-process. tokio::broadcast ring buffer.
    InProcess,
    /// Local Bus + relay bridge for cross-instance Pulses.
    RelayBacked { relay_url: String },
}
```

---

## 2. Local Development

### 2.1 Getting Started

```bash
# Install
cargo install roko-cli

# Initialize workspace
roko init

# Set API key
echo "sk-ant-..." | roko config secrets set llm.anthropic

# Start control plane (insecure mode for local dev -- no auth required)
roko serve --insecure

# In another terminal: interactive TUI
roko dashboard
```

The control plane starts on `localhost:6677` with ~85 HTTP routes, SSE, and WebSocket. The TUI connects to the same port and displays real-time Agent status, plan progress, and learning metrics via StateHub projections.

### 2.2 Local Agent Workflow

```bash
# Create an Agent
roko agent create --profile coding --prompt "Fix the auth bug"

# Start it
roko agent start --name fix-auth-bug

# Watch progress
roko dashboard

# Or use the self-hosting loop
roko prd idea "Wire SystemPromptBuilder into orchestrate.rs"
roko prd draft new "system-prompt-wiring"
roko prd plan system-prompt-wiring
roko plan run plans/
```

### 2.3 Agent Creation UX

Agent creation is a Graph of Cells (the **agent-wizard** Graph). Every surface (CLI, API, dashboard) fires the same Graph with different input Signals:

```toml
[graph]
name = "agent-wizard"
description = "Create an Agent from user input via any surface"

[[nodes]]
id = "parse-intent"
cell = "agent-dispatch@^1"
[nodes.params]
prompt_template = "classify-agent-intent"
# Input: user prompt text. Output: { name, profile, mode, triggers, budget }.

[[nodes]]
id = "validate-config"
cell = "rule-scorer@^1"
[nodes.params]
rules = [
  { field = "name", op = "regex", value = "^[a-z][a-z0-9-]{1,63}$", dimension = "quality", weight = 1.0 },
  { field = "budget", op = "gte", value = 0.01, dimension = "utility", weight = 1.0 },
]

[[nodes]]
id = "select-tier"
cell = "rule-router@^1"
[nodes.params]
rules = [
  { condition = "execution == 'isolated'", select = "fly-machine" },
  { condition = "true", select = "in-process" },
]

[[nodes]]
id = "provision"
cell = "agent-provision@^1"
# Creates the Agent manifest, registers triggers, writes to Store.

[[nodes]]
id = "verify-created"
cell = "compile-gate@^1"
[nodes.params]
command = "roko agent status --name {{name}}"

[[edges]]
from = "parse-intent"
to = "validate-config"

[[edges]]
from = "validate-config"
to = "select-tier"

[[edges]]
from = "select-tier"
to = "provision"

[[edges]]
from = "provision"
to = "verify-created"
```

```rust
pub struct AgentWizardInput {
    pub source: WizardSource,          // Cli | Api | Dashboard
    pub prompt: Option<String>,        // Free-text description (CLI/dashboard)
    pub explicit: Option<AgentSpec>,   // Structured spec (API)
    pub template: Option<String>,      // Template name (any surface)
}

pub enum WizardSource { Cli, Api, Dashboard }
```

**CLI quick create** (auto-fills from prompt via `parse-intent` Cell):

```bash
roko agent create --prompt "Review PRs for security issues"
```

**Explicit configuration** (bypasses `parse-intent`, feeds `validate-config` directly):

```bash
roko agent create \
  --name pr-reviewer \
  --profile coding \
  --mode reactive \
  --trigger "webhook:/hooks/github-pr" \
  --trigger "schedule:0 9 * * MON" \
  --budget 10.00
```

**From a template**:

```bash
roko agent create --template code-reviewer --repo https://github.com/org/repo
```

**API**:

```
POST /api/agents
{
  "name": "pr-reviewer",
  "prompt": "Review pull requests for security issues",
  "profile": "coding",
  "mode": "reactive",
  "triggers": [
    { "type": "webhook", "path": "/hooks/github-pr" },
    { "type": "schedule", "cron": "0 9 * * MON" }
  ],
  "execution": "in-process",
  "budget": { "daily_limit_usd": 10.0 },
  "model_routing": {
    "gamma_model": "claude-haiku-4-5",
    "theta_model": "claude-sonnet-4-6",
    "delta_model": "claude-opus-4-6"
  }
}
```

**Dashboard wizard**:

```
Step 1: What does this agent do?
+-----------------------------------------------------+
| Describe your agent's purpose:                       |
| +---------------------------------------------------+|
| | Review pull requests on the main repo, check for  ||
| | security issues, and post comments.               ||
| +---------------------------------------------------+|
|                                                       |
| Or choose a template:                                 |
| [Code reviewer]  [Chain monitor]  [Research assistant] |
| [PR automator]   [Security audit] [Data pipeline]     |
+-----------------------------------------------------+

Step 2: Configuration (auto-filled from description)
+-----------------------------------------------------+
| Name:     [pr-reviewer        ]                      |
| Profile:  [Coding           v ]                      |
| Mode:     [Reactive          v]                      |
|                                                       |
| Triggers:                                             |
|  [x] GitHub webhook: push to main                    |
|  [ ] Schedule: ______                                |
|  [ ] Chain event: ______                             |
|                                                       |
| Execution:                                            |
|  (o) In-process (recommended for most agents)        |
|  ( ) Isolated (Fly Machine -- separate compute)      |
|                                                       |
| Model:                                                |
|  (o) Auto (CascadeRouter selects per-task)           |
|  ( ) Force: [______________]                         |
|                                                       |
| Budget: [$10.00/day   ] (inference cost limit)       |
+-----------------------------------------------------+

Step 3: Review and create
+-----------------------------------------------------+
| Agent: pr-reviewer                                   |
| Profile: Coding                                      |
| Mode: Reactive (wakes on GitHub push)                |
| Execution: In-process                                |
| Model: Auto                                          |
| Budget: $10/day                                      |
| Extensions: git, compiler, test-runner, lsp          |
|                                                       |
| [Create agent]                                       |
+-----------------------------------------------------+
```

All three surfaces (CLI, API, dashboard) produce an `AgentWizardInput` Signal and fire the same `agent-wizard` Graph. The Graph validates, provisions, and verifies the Agent. No surface-specific logic exists outside the Graph.

### 2.4 Local Chain Development (Mirage)

For on-chain features, start a local Mirage devnet:

```bash
# Start Mirage (anvil + contracts)
cd contracts/
npx hardhat node  # localhost:8545

# Deploy contracts
npx hardhat deploy --network mirage

# Configure roko to use Mirage
# (roko.toml defaults to chain.network = "mirage")
```

---

## 3. Daemon Lifecycle

The daemon wraps `roko serve` as a managed background process with proper lifecycle control. The lifecycle is a Graph of Cells, not ad-hoc shell scripting.

### 3.1 Commands

| Command | What It Does |
|---|---|
| `roko daemon start` | Start roko serve as a background daemon (writes PID to `~/.roko/daemon.pid`) |
| `roko daemon stop` | Send SIGTERM to the daemon PID; wait up to 10s for graceful shutdown, then SIGKILL |
| `roko daemon status` | Check if daemon is running (PID alive + health check at `/api/health`) |
| `roko daemon logs` | Tail daemon stdout/stderr from `~/.roko/daemon.log` |
| `roko daemon install` | Install systemd unit (Linux) or launchd plist (macOS) for auto-start |

### 3.2 Daemon Lifecycle Graph

The daemon lifecycle is expressed as a **Loop Graph** with typed Cells:

```toml
[graph]
name = "daemon-lifecycle"
description = "Manage roko serve as a background daemon with health monitoring"

[[nodes]]
id = "command-input"
cell = "manual-trigger@^1"
# DaemonCommand Signal enters here.

[[nodes]]
id = "dispatch"
cell = "rule-router@^1"
[nodes.params]
rules = [
  { condition = "command == 'start'",   select = "spawn" },
  { condition = "command == 'stop'",    select = "shutdown" },
  { condition = "command == 'status'",  select = "health-check" },
  { condition = "command == 'install'", select = "install-service" },
]

[[nodes]]
id = "spawn"
cell = "daemon-spawn@^1"
# Fork roko serve, write PID, redirect stdout/stderr to log file.

[[nodes]]
id = "health-check"
cell = "daemon-health@^1"
# Check PID alive + GET /api/health.

[[nodes]]
id = "shutdown"
cell = "daemon-shutdown@^1"
# SIGTERM -> wait shutdown_timeout -> SIGKILL if needed.

[[nodes]]
id = "install-service"
cell = "daemon-install@^1"
# Generate systemd unit or launchd plist based on platform.

[[nodes]]
id = "emit-status"
cell = "artifact-persist@^1"
# Publish DaemonStatus Signal to Bus.

[[edges]]
from = "command-input"
to = "dispatch"

[[edges]]
from = "dispatch"
to = "spawn"
condition = "selected == 'spawn'"

[[edges]]
from = "dispatch"
to = "shutdown"
condition = "selected == 'shutdown'"

[[edges]]
from = "dispatch"
to = "health-check"
condition = "selected == 'health-check'"

[[edges]]
from = "dispatch"
to = "install-service"
condition = "selected == 'install-service'"

[[edges]]
from = "spawn"
to = "emit-status"

[[edges]]
from = "shutdown"
to = "emit-status"

[[edges]]
from = "health-check"
to = "emit-status"

[[edges]]
from = "install-service"
to = "emit-status"
```

### 3.3 Daemon Cells: Typed I/O

```rust
/// Input Signal for the daemon lifecycle Graph.
pub struct DaemonCommand {
    pub action: DaemonAction,
}

pub enum DaemonAction {
    Start { insecure: bool, port: u16 },
    Stop,
    Status,
    Logs { follow: bool, lines: usize },
    Install,
}

/// Output Signal from the daemon lifecycle Graph.
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime: Option<Duration>,
    pub health: Option<HealthCheckResult>,
    pub log_path: PathBuf,
}

/// Individual daemon Cells.

pub struct DaemonSpawnCell {
    pub pid_file: PathBuf,       // ~/.roko/daemon.pid
    pub log_file: PathBuf,       // ~/.roko/daemon.log
    pub port: u16,               // 6677 default
}
// Input: DaemonCommand { action: Start }
// Output: DaemonStatus { running: true, pid: Some(pid), ... }

pub struct DaemonHealthCell {
    pub health_url: String,      // http://localhost:6677/api/health
    pub pid_file: PathBuf,
}
// Input: DaemonCommand { action: Status }
// Output: DaemonStatus { running, pid, uptime, health }

pub struct DaemonShutdownCell {
    pub pid_file: PathBuf,
    pub shutdown_timeout: Duration, // 10s default
}
// Input: DaemonCommand { action: Stop }
// Output: DaemonStatus { running: false, ... }

pub struct DaemonInstallCell {
    pub service_name: String,    // "dev.nunchi.roko"
}
// Input: DaemonCommand { action: Install }
// Output: Signal { kind: ServiceConfig } (the generated systemd/launchd config)
```

### 3.4 Systemd Unit (Linux)

```ini
[Unit]
Description=Roko Agent Control Plane
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/roko serve
Restart=on-failure
RestartSec=5
WorkingDirectory=/workspace
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
```

### 3.5 Launchd Plist (macOS)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>dev.nunchi.roko</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/roko</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/Users/will/.roko/daemon.log</string>
  <key>StandardErrorPath</key>
  <string>/Users/will/.roko/daemon.log</string>
</dict>
</plist>
```

### 3.6 Self-Healing Supervisor Graph

Production crash recovery is a **Graph of Cells** with a circuit-breaker edge, not ad-hoc shell scripting. The supervisor Graph runs outside the main roko process (as the daemon wrapper or as a sidecar) so it survives the crash it is recovering from.

```toml
[graph]
name = "self-healing-supervisor"
description = "Detect crashes, deduplicate errors, diagnose, apply fixes, restart"

[[nodes]]
id = "crash-detect"
cell = "crash-detect-cell@^1"
# Monitors roko process. On non-zero exit, extracts panic signature from stderr.
# Input: ProcessHandle
# Output: CrashReport { exit_code, panic_signature, stderr_tail, timestamp }

[[nodes]]
id = "error-dedup"
cell = "error-dedup-cell@^1"
# Tracks error signatures in Store. Skips already-seen-and-unfixed errors.
# Input: CrashReport
# Output: DeduplicatedCrash { crash: CrashReport, seen_count: u32, is_new: bool }

[[nodes]]
id = "diagnose"
cell = "diagnose-cell@^1"
# Feeds crash report + recent logs to LLM for root-cause analysis.
# Only runs when ROKO_SUPERVISOR_AUTOFIX=true.
# Input: DeduplicatedCrash
# Output: Diagnosis { root_cause, suggested_fix: FixProposal, confidence }

[[nodes]]
id = "apply-fix"
cell = "apply-fix-cell@^1"
# Applies the suggested fix. CONSTRAINT: limited to config changes only.
# Code changes require human approval (emits escalation Pulse instead).
# Input: Diagnosis
# Output: FixResult { applied: bool, change_type: ConfigChange | CodeChange, diff }

[[nodes]]
id = "restart"
cell = "daemon-spawn@^1"
# Restarts the roko process. Reuses the DaemonSpawnCell from section 3.3.
# Input: FixResult | CrashReport (when autofix is disabled)
# Output: DaemonStatus

[[edges]]
from = "crash-detect"
to = "error-dedup"

[[edges]]
from = "error-dedup"
to = "diagnose"
condition = "is_new == true && autofix_enabled"

[[edges]]
from = "error-dedup"
to = "restart"
condition = "is_new == false || !autofix_enabled"
# Skip diagnosis for known errors or when autofix is off. Just restart.

[[edges]]
from = "diagnose"
to = "apply-fix"
condition = "suggested_fix.change_type == 'config'"

[[edges]]
from = "diagnose"
to = "restart"
condition = "suggested_fix.change_type == 'code'"
# Code changes cannot be auto-applied. Escalate and restart without fix.

[[edges]]
from = "apply-fix"
to = "restart"

# Circuit-breaker edge: after max_restarts within window, the Graph halts.
[[edges]]
from = "restart"
to = "crash-detect"
circuit_breaker = { max_fires = 3, window_secs = 300 }
# When breaker opens: emit alert Signal on Bus topic "supervisor.circuit-open",
# stop restarting, require human intervention.
```

#### Supervisor Cells: Typed I/O

```rust
/// CrashDetectCell — monitors child process.
pub struct CrashDetectCell {
    pub pid_file: PathBuf,
    pub stderr_tail_lines: usize,  // 100 default
}
// Input: Signal { kind: ProcessHandle }
// Output: Signal { kind: CrashReport }

pub struct CrashReport {
    pub exit_code: i32,
    pub panic_signature: Option<String>,  // demangled panic message hash
    pub stderr_tail: String,
    pub timestamp: DateTime<Utc>,
    pub uptime_before_crash: Duration,
}

/// ErrorDedupCell — tracks signatures in Store.
pub struct ErrorDedupCell {
    pub store_path: PathBuf,  // .roko/supervisor-errors.json
    pub dedup_window: Duration,
}
// Input: Signal { kind: CrashReport }
// Output: Signal { kind: DeduplicatedCrash }

pub struct DeduplicatedCrash {
    pub crash: CrashReport,
    pub seen_count: u32,
    pub is_new: bool,
    pub first_seen: DateTime<Utc>,
}

/// DiagnoseCell — LLM-based root-cause analysis.
pub struct DiagnoseCell {
    pub model: String,       // default: claude-sonnet-4-6
    pub max_context_lines: usize,
}
// Input: Signal { kind: DeduplicatedCrash }
// Output: Signal { kind: Diagnosis }
// Capabilities: Llm

pub struct Diagnosis {
    pub root_cause: String,
    pub suggested_fix: FixProposal,
    pub confidence: f64,
}

pub struct FixProposal {
    pub change_type: ChangeType,
    pub description: String,
    pub diff: Option<String>,       // for config changes
    pub escalation: Option<String>, // for code changes
}

pub enum ChangeType {
    /// Config-only change (env vars, TOML, feature flags).
    /// Can be auto-applied.
    Config,
    /// Code change. Cannot be auto-applied. Requires human approval.
    Code,
}

/// ApplyFixCell — applies config-only fixes.
pub struct ApplyFixCell;
// Input: Signal { kind: Diagnosis }
// Output: Signal { kind: FixResult }
// Capabilities: FsWrite (config files only)
// CONSTRAINT: rejects any FixProposal where change_type == Code.
//             Emits escalation Pulse on Bus topic "supervisor.escalation".

pub struct FixResult {
    pub applied: bool,
    pub change_type: ChangeType,
    pub diff: Option<String>,
}
```

#### Constraints

1. **Auto-fix limited to config changes.** The `ApplyFixCell` rejects code changes. When the `DiagnoseCell` proposes a code fix, the Graph routes to `restart` without applying the fix and emits an escalation Pulse for human review.
2. **Code changes require human approval.** Escalation Pulses on `supervisor.escalation` are consumed by `escalation-reactor` (if configured) for Slack/email/dashboard notification.
3. **Disabled by default.** `ROKO_SUPERVISOR_AUTOFIX=false` is the default. When disabled, the Graph skips the `diagnose` and `apply-fix` Cells entirely and goes straight from `error-dedup` to `restart`.
4. **Circuit breaker prevents crash loops.** The `restart -> crash-detect` feedback edge has a circuit breaker: after 3 consecutive restarts within 5 minutes, the breaker opens. The Graph halts and emits a `supervisor.circuit-open` Pulse.

#### Configuration

```bash
ROKO_SUPERVISOR_MAX_RESTARTS=3      # circuit breaker threshold
ROKO_SUPERVISOR_WINDOW_SECS=300     # circuit breaker window
ROKO_SUPERVISOR_AUTOFIX=false       # disabled by default; config-only when enabled
```

---

## 4. WASM + Native Packaging

The same Roko core compiles to **native** and **WASM** targets. Progressive enhancement: start with the full native binary, deploy lightweight WASM components where sandboxing or portability matters.

### 4.1 What Compiles to WASM

| Component | WASM Target | Use Case |
|---|---|---|
| Cell implementations | `wasm32-wasi` | Marketplace distribution (5-tier SPI Tier 4) |
| Scoring functions | `wasm32-wasi` | Portable eval scoring for arenas |
| Gate implementations | `wasm32-wasi` | Custom verification logic |
| Extension hooks | `wasm32-wasi` | Third-party Extension distribution |
| Signal processing pipelines | `wasm32-wasi` | Edge deployment, browser-side processing |
| HDC vector operations | `wasm32-unknown-unknown` | Client-side similarity search |

### 4.2 Three Capability Levels

```
Full native (default)
  - All crates compiled natively
  - Full filesystem, network, LLM access
  - Maximum performance
  - Deployment: server, desktop

WASM runtime (embedded)
  - Native host + WASM guest Cells
  - Host mediates capabilities (fuel-metered)
  - Sandboxed third-party code
  - Deployment: server with untrusted plugins

WASM standalone (portable)
  - Core engine as WASM module
  - Runs in any WASI-compatible runtime
  - Limited capabilities (no direct fs/net)
  - Deployment: edge, browser, serverless
```

### 4.3 Build Targets

```bash
# Native (default)
cargo build --release -p roko-cli

# WASM Cell (for marketplace publication)
cargo build --release -p my-cell --target wasm32-wasi

# WASM core (for portable deployment)
cargo build --release -p roko-core --target wasm32-wasi
```

### 4.4 Fuel Metering

WASM Cells run with fuel limits to prevent runaway computation:

```toml
# Cell manifest
[cell.impl]
tier      = "wasm"
path      = "my-cell.wasm"
fuel      = 100_000_000    # execution fuel cap
memory_mb = 64             # memory limit
```

The host runtime (wasmtime) tracks fuel consumption and terminates the WASM instance when the limit is reached. 100M fuel is approximately 1 second of computation on modern hardware.

### 4.5 ABI Contract

WASM Cells communicate with the host via `wit-bindgen` interfaces:

```wit
// roko-cell.wit
interface cell {
    record signal {
        id: string,
        kind: string,
        payload: string,
        score: tuple<f64, f64, f64, f64, f64>,
    }

    record cell-input {
        signals: list<signal>,
        macros: list<tuple<string, string>>,
    }

    record cell-output {
        signals: list<signal>,
        persist: list<signal>,
    }

    run: func(input: cell-input) -> result<cell-output, string>
}
```

The host grants capabilities to the WASM guest based on the Cell's declared capabilities intersected with the Space grants (three-layer capability intersection). CaMeL tags are applied at the host function boundary — the WASM guest cannot strip or modify tags.

---

## 5. Brain Export and Import

An Agent's learned state — its routing preferences, heuristics, calibration data, knowledge graph, and adaptive thresholds — can be exported as a portable **brain** and imported into a new instance. This enables knowledge transfer between deployments, backup/restore, and Agent cloning.

### 5.1 What a Brain Contains

```
brain-export-2026-04-26.roko-brain
+-- manifest.toml              # metadata, version, source agent, export time
+-- knowledge/
|   +-- signals.jsonl          # Knowledge Signals (Heuristic, Insight, etc.)
|   +-- hdc-index.bin          # HDC fingerprint index (binary, compact)
+-- learning/
|   +-- cascade-router.json    # CascadeRouter state (EFE posteriors)
|   +-- gate-thresholds.json   # Adaptive gate thresholds (EMA per rung)
|   +-- experiments.json       # Prompt experiment state
|   +-- efficiency.jsonl       # Efficiency event history
|   +-- calibration.json       # Per-operator calibration state
+-- episodes/
|   +-- episodes.jsonl         # Episode history (summarized, not full turns)
+-- profile/
    +-- profile.toml           # Domain profile snapshot
    +-- extensions.toml        # Extension configuration
```

### 5.2 Export Size

A brain export is compact — typically **100KB to 1MB**:

| Component | Typical Size | Notes |
|---|---|---|
| Knowledge Signals | 50-500 KB | Only Consolidated+ tier Signals exported by default |
| HDC index | 10-100 KB | Binary, compact |
| Learning state | 5-50 KB | JSON, small |
| Episode summaries | 20-200 KB | Summarized, not full turns |
| Profile + config | 2-10 KB | TOML |

Full episode history (with complete turns) is excluded by default. Include it with `--include-episodes=full`, which increases size to 1-10 MB.

### 5.3 Export CLI

```bash
# Export current Agent's brain
roko knowledge backup --agent coder-1 --output coder-brain.roko-brain

# Export with filters
roko knowledge backup --agent coder-1 \
  --min-tier consolidated \     # only high-confidence knowledge
  --since 2026-04-01 \          # recent learning only
  --include-episodes=summary \  # episode summaries, not full turns
  --output coder-brain.roko-brain
```

### 5.4 Import CLI

```bash
# Import into a new Agent
roko knowledge restore --agent coder-2 --input coder-brain.roko-brain

# Import with decay (older knowledge starts at lower balance)
roko knowledge restore --agent coder-2 \
  --input coder-brain.roko-brain \
  --decay-factor 0.8            # imported Signals start at 80% balance
```

### 5.5 Merkle-CRDT Sync Protocol

When two Agent instances share a brain lineage (e.g., one was cloned from the other), they can sync learning state via **Merkle-CRDT merge**. This produces convergent state without central coordination.

```
Agent A (original)          Agent B (clone)
    |                           |
    v                           v
Learn from task X           Learn from task Y
    |                           |
    v                           v
Brain state A'              Brain state B'
    |                           |
    +--- Merkle-CRDT sync ---+
              |
              v
    Merged state (A' + B')
    Both agents converge
```

#### CRDT Operations

Each learning update maps to a conflict-free replicated data type:

| Component | CRDT Type | Merge Behavior |
|---|---|---|
| CascadeRouter model counts | **GCounter** (grow-only counter) | Sum of per-node increments. Monotonic. |
| Gate thresholds | **LWW-Register** (last-writer-wins) | Most recent Lamport timestamp wins. |
| Knowledge Signals | **Add-only set** with demurrage | Union of both sets. Duplicate Signals deduplicated by content hash. Balance is a GCounter. |
| Experiment state | **LWW-Register** | Most recent wins. |
| Episode summaries | **Add-only set** | Union. |

```rust
pub enum CrdtOp {
    GCounterIncrement { key: String, delta: u64, node_id: NodeId },
    LwwRegisterSet { key: String, value: Value, timestamp: LamportClock },
    SetAdd { key: String, element: ContentHash },
}
```

#### Merkle Tree Indexing

Each Agent maintains a Merkle tree over its brain state. The root hash summarizes the entire learning state in 32 bytes.

```rust
pub struct BrainMerkleTree {
    pub root: H256,
    pub nodes: HashMap<H256, MerkleNode>,
}

pub enum MerkleNode {
    Leaf { key: String, value_hash: H256 },
    Branch { left: H256, right: H256 },
}
```

#### Incremental Sync

Two Agents exchange Merkle roots. If roots differ, they walk the tree to find divergent subtrees and exchange only the differing CRDT operations. Typical sync payload: **1-10 KB** for incremental updates.

```bash
# One-shot sync
roko knowledge sync --peer wss://other-instance.example.com/sync

# Continuous sync (background)
roko knowledge sync --peer wss://other-instance.example.com/sync --continuous
```

#### Conflict-Free Convergence

CRDTs are conflict-free by construction. Two Agents that learned different things from different tasks converge to a state that contains both learnings. No manual conflict resolution. GCounters merge via component-wise max. LWW-Registers merge via timestamp comparison. Add-only sets merge via union.

### 5.6 Use Cases

| Scenario | How Brain Export Helps |
|---|---|
| **Backup/restore** | Export brain before risky changes, restore if things go wrong |
| **Agent cloning** | Clone a well-trained Agent for a new workspace |
| **Knowledge transfer** | Import coding heuristics from a senior Agent into a junior one |
| **Multi-instance sync** | Two instances developing the same codebase share learning |
| **Deployment migration** | Move an Agent from local to cloud without losing learning state |
| **Arena bootstrapping** | Import brain from meta-arena into a new coding arena |

---

## 6. Secrets Management

### 6.1 Three-Tier Priority

Secrets are resolved in priority order:

| Priority | Source | When |
|---|---|---|
| 1 (highest) | Environment variables | Cloud deployment (Railway, Fly.io, etc.) |
| 2 | `roko.toml` `[secrets]` section | Config-file references (paths or encrypted values) |
| 3 (lowest) | Encrypted local store | `~/.roko/secrets/` with age encryption |

The first source that provides a value wins. This means environment variables always override local secrets, which is the correct behavior for cloud deployment.

### 6.2 Age Encryption

Local secrets are encrypted at rest using age (https://age-encryption.org):

```bash
# Set a secret (encrypts with age, stores in ~/.roko/secrets/)
roko config secrets set llm.anthropic

# List secrets (names only, not values)
roko config secrets list

# Rotate a secret
roko config secrets rotate llm.anthropic

# Check which secrets are configured
roko config check-secrets
```

Secrets are stored as age-encrypted files in `~/.roko/secrets/`. The encryption key is derived from the user's system keychain (macOS Keychain, Linux Secret Service, or Windows Credential Manager). On systems without a keychain, a passphrase is required.

### 6.3 Cloud Deployment Secrets

For cloud deployment, secrets are set as environment variables in the deployment platform (Railway, Fly.io, etc.). The `roko serve` command reads them on startup.

### 6.4 Provider API Keys

| Provider | Env Variable | Used By |
|---|---|---|
| Anthropic | `ANTHROPIC_API_KEY` | Primary LLM backend |
| Perplexity | `PERPLEXITY_API_KEY` | Research Agent |
| Gemini | `GEMINI_API_KEY` | Gemini backend |
| OpenRouter | `OPENROUTER_API_KEY` | OpenRouter multi-model |
| GitHub | `GITHUB_TOKEN` | GitHub MCP integration |
| Fly.io | `FLY_API_TOKEN` | Isolated Agent execution |

### 6.5 Isolated Agent Security

Secrets are **never** passed in environment variables to child Agents when using isolated execution. Agents use the inference proxy instead. This ensures that a compromised isolated Agent cannot exfiltrate API keys.

```
Control plane (has keys)
    |
    +-- Inference proxy (/api/inference/proxy)
    |       |
    +-- Isolated Agent (no keys, uses proxy)
```

---

## 7. Railway Deployment

### 7.1 One-Click Deploy

```
1. Click "Deploy on Railway"               (~30 seconds)
2. Railway asks for env vars               (paste Anthropic key)
3. roko builds and starts                  (~2 minutes)
4. Visit the URL -> setup wizard           (~30 seconds)
5. Create account
6. Onboarding: create first Agent          (~1 minute)
7. Agent is running, visible in dashboard

Total: ~4 minutes from zero to running Agent.
```

### 7.2 Railway Template

```toml
# railway.toml
[build]
builder = "DOCKERFILE"
dockerfilePath = "docker/roko.Dockerfile"

[deploy]
healthcheckPath = "/api/health"
healthcheckTimeout = 30
restartPolicyType = "ON_FAILURE"

[[services]]
name = "roko"
internalPort = 6677
```

### 7.3 Environment Variables

| Variable | Default | Required? | Notes |
|---|---|---|---|
| `ANTHROPIC_API_KEY` | -- | Yes | Primary LLM provider |
| `PERPLEXITY_API_KEY` | -- | No | Research agent |
| `GEMINI_API_KEY` | -- | No | Gemini backend |
| `OPENROUTER_API_KEY` | -- | No | OpenRouter backend |
| `GITHUB_TOKEN` | -- | No | GitHub MCP integration |
| `FLY_API_TOKEN` | -- | No | Enables isolated Agent execution |
| `RELAY_URL` | `wss://relay.nunchi.dev` | No | Relay for multi-instance |
| `PORT` | `6677` | No | HTTP port |
| `RUST_LOG` | `info` | No | Log level |

### 7.4 Scaling on Railway

For higher load, run multiple Railway services behind Railway's internal load balancer. Each instance connects to a shared relay for Agent presence deduplication and message routing.

```
Railway Service 1 (roko serve) --> Relay (wss://relay.nunchi.dev)
Railway Service 2 (roko serve) --/
Railway Service 3 (roko serve) --/
```

Brain sync (section 5.5) keeps learning state convergent across instances.

---

## 8. Fly.io Deployment

### 8.1 fly.toml

```toml
app = "roko"
primary_region = "iad"

[build]
  dockerfile = "docker/roko.Dockerfile"

[http_service]
  internal_port = 6677
  force_https = true
  auto_start_machines = true
  auto_stop_machines = true
  min_machines_running = 1

[[vm]]
  cpu_kind = "shared"
  cpus = 2
  memory_mb = 2048

[mounts]
  source = "roko_data"
  destination = "/workspace/.roko"
```

### 8.2 Machine Sizing

| Workload | CPUs | Memory | Notes |
|---|---|---|---|
| Solo (1-5 Agents) | 1 shared | 512 MB | Minimum viable |
| Small team (5-20 Agents) | 2 shared | 2 GB | Default |
| Production (20+ Agents) | 4 dedicated | 4 GB | For heavy inference loads |

### 8.3 Regions

For lowest latency to LLM providers:

- `iad` (Ashburn, Virginia) — closest to Anthropic, OpenAI
- `sjc` (San Jose) — West Coast alternative
- `lhr` (London) — EU presence

### 8.4 Isolated Agent Execution via Fly Machines

Fly Machines enable true isolation for untrusted workloads. The control plane creates a Fly Machine per Agent:

```
roko process (control plane)
    |
    +-- POST https://api.machines.dev/v1/machines
    |   -> Create Fly Machine with:
    |     - roko agent run --relay ... --inference-proxy ...
    |     - Volume for persistent state
    |     - Network: outbound only (connects to relay)
    |
    +-- Lifecycle managed by control plane:
        - Create on agent.create
        - Suspend on agent.sleep (reactive mode)
        - Destroy on agent.delete
```

Fly Machines bill per-second. Reactive Agents cost $0 while sleeping.

```rust
pub struct FlyMachineManager {
    api_token: String,
    app_name: String,
    http: reqwest::Client,
}

impl FlyMachineManager {
    async fn create_agent(&self, spec: &AgentSpec) -> Result<MachineId> {
        let body = json!({
            "config": {
                "image": "ghcr.io/nunchi/roko-agent:latest",
                "env": {
                    "ROKO_AGENT_NAME": spec.name,
                    "ROKO_RELAY_URL": spec.relay_url,
                    "ROKO_INFERENCE_PROXY": spec.inference_proxy_url,
                    "ROKO_AGENT_TOKEN": spec.token,
                },
                "guest": {
                    "cpu_kind": "shared",
                    "cpus": 1,
                    "memory_mb": 512,
                },
                "auto_destroy": true,
            }
        });

        let resp = self.http
            .post(format!(
                "https://api.machines.dev/v1/apps/{}/machines",
                self.app_name
            ))
            .bearer_auth(&self.api_token)
            .json(&body)
            .send()
            .await?;

        let machine: FlyMachine = resp.json().await?;
        Ok(machine.id)
    }
}
```

---

## 9. Docker Deployment

### 9.1 Multi-Stage Dockerfile

```dockerfile
# Build stage
FROM rust:1.91 AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p roko-cli

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/roko /usr/local/bin/roko
EXPOSE 6677
VOLUME ["/workspace/.roko"]
HEALTHCHECK CMD curl -f http://localhost:6677/api/health || exit 1
ENTRYPOINT ["roko", "serve"]
```

### 9.2 Docker Compose

```yaml
version: "3.8"
services:
  roko:
    build:
      context: .
      dockerfile: docker/roko.Dockerfile
    ports:
      - "6677:6677"
    volumes:
      - roko_data:/workspace/.roko
    environment:
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
      - RUST_LOG=info
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:6677/api/health"]
      interval: 30s
      timeout: 10s
      retries: 3

volumes:
  roko_data:
```

### 9.3 Cloud Deployment Commands

```bash
# Railway
roko deploy railway                # Interactive Railway deployment
roko deploy railway --from-config  # Use existing railway.toml

# Fly.io
roko deploy fly                    # Interactive Fly deployment
roko deploy fly --region iad       # Specify region

# Docker
roko deploy docker                 # Build + tag Docker image
roko deploy docker --push          # Build + push to registry
```

### 9.4 Deployment Pipeline Graph

Every `roko deploy` invocation fires the **deploy-pipeline** Graph. The Graph is a Pipeline (linear chain with early exit on Verify failure):

```toml
[graph]
name = "deploy-pipeline"
description = "Build, deploy, verify, notify — same Graph for all targets"

[[nodes]]
id = "build"
cell = "build@^1"
[nodes.params]
command = "docker build -f docker/roko.Dockerfile -t roko:latest ."

[[nodes]]
id = "pre-deploy-gate"
cell = "compile-gate@^1"
[nodes.params]
command = "docker run --rm roko:latest roko doctor"

[[nodes]]
id = "deploy"
cell = "rule-router@^1"
[nodes.params]
rules = [
  { condition = "target == 'railway'", select = "deploy-railway" },
  { condition = "target == 'fly'",     select = "deploy-fly" },
  { condition = "target == 'docker'",  select = "deploy-shell" },
]

[[nodes]]
id = "deploy-railway"
cell = "deploy-railway@^1"

[[nodes]]
id = "deploy-fly"
cell = "deploy-fly@^1"

[[nodes]]
id = "deploy-shell"
cell = "deploy-shell@^1"

[[nodes]]
id = "smoke"
cell = "smoke-test@^1"
[nodes.params]
endpoints = ["/api/health"]
timeout_secs = 30

[[nodes]]
id = "notify"
cell = "slack-notify@^1"

[[nodes]]
id = "rollback"
cell = "rollback@^1"
# Only reached if smoke-test fails.

[[edges]]
from = "build"
to = "pre-deploy-gate"

[[edges]]
from = "pre-deploy-gate"
to = "deploy"

[[edges]]
from = "deploy"
to = "deploy-railway"
condition = "selected == 'deploy-railway'"

[[edges]]
from = "deploy"
to = "deploy-fly"
condition = "selected == 'deploy-fly'"

[[edges]]
from = "deploy"
to = "deploy-shell"
condition = "selected == 'deploy-shell'"

[[edges]]
from = "deploy-railway"
to = "smoke"

[[edges]]
from = "deploy-fly"
to = "smoke"

[[edges]]
from = "deploy-shell"
to = "smoke"

[[edges]]
from = "smoke"
to = "notify"
condition = "passed == true"

[[edges]]
from = "smoke"
to = "rollback"
condition = "passed == false"

[[edges]]
from = "rollback"
to = "notify"
```

This Graph composes the domain Cells from section 11.4 (Deploy Cells) with standard Verify Cells. The same Graph works for all deploy targets -- the `rule-router` selects the target-specific Cell.

---

## 10. Worker Mode

When deployed as a worker (`roko worker`), roko connects to a relay and executes tasks dispatched by a coordinator. No local serve, no TUI — just a computation node.

```bash
# Start as worker
roko worker --relay wss://relay.nunchi.dev --token $WORKER_TOKEN

# Worker registers with relay, receives task assignments
# Executes Graphs, reports results back through relay
```

The worker is a Cell that:
- Connects to relay via Bus (Subscribe to `worker.{id}.tasks`)
- Executes assigned Graphs via the Engine
- Publishes results as Signals through Bus
- Reports health via periodic Pulses on `worker.{id}.health`

---

## 11. Agent Execution Tiers

```
Tier          Where              When to Use
----          -----              -----------
In-process    tokio task         Default. Fast. Shares memory and Route protocol.
              inside roko        Best for trusted code, small teams.

Isolated      Fly Machine or     Untrusted code, heavy compute,
              Railway service    multi-tenant, customer-facing Agents.
```

### 11.1 Execution Tier Selection Graph

The choice between in-process and isolated execution is a Route decision, expressed as a Cell:

```toml
[graph]
name = "execution-tier-selector"
description = "Route an Agent to in-process or isolated execution"

[[nodes]]
id = "assess"
cell = "rule-scorer@^1"
[nodes.params]
rules = [
  { field = "trust_level", op = "eq", value = "untrusted", dimension = "relevance", weight = 1.0 },
  { field = "memory_estimate_mb", op = "gte", value = 512, dimension = "quality", weight = 0.5 },
  { field = "multi_tenant", op = "eq", value = "true", dimension = "utility", weight = 1.0 },
]

[[nodes]]
id = "route"
cell = "rule-router@^1"
[nodes.params]
rules = [
  { condition = "trust_level == 'untrusted'",  select = "isolated" },
  { condition = "multi_tenant == true",         select = "isolated" },
  { condition = "memory_estimate_mb >= 2048",   select = "isolated" },
  { condition = "true",                         select = "in-process" },
]

[[edges]]
from = "assess"
to = "route"
```

```rust
pub struct ExecutionTierDecision {
    pub tier: ExecutionTier,
    pub reason: String,
    pub estimated_memory_mb: u64,
}

pub enum ExecutionTier {
    InProcess,                              // tokio task, shared memory
    Isolated { provider: IsolationProvider }, // Fly Machine, Railway service
}

pub enum IsolationProvider {
    FlyMachine { region: String },
    RailwayService,
    DockerContainer,
}
```

### 11.2 In-Process Scaling

A single roko process can run 50-100 in-process Agents concurrently. Each Agent is a tokio task consuming ~1MB of stack + working memory. The bottleneck is inference throughput, not Agent count.

For higher Agent counts, run multiple roko processes behind a load balancer, each connected to the same relay. Brain sync (section 5.5) keeps learning state convergent.

### 11.3 Agent Clusters with Pipeline Graphs

Groups of Agents with shared context and coordinated Graphs:

```
POST /api/clusters
{
  "name": "feature-build",
  "agents": [
    { "profile": "research", "name": "researcher", "mode": "ephemeral" },
    { "profile": "coding", "name": "impl-1", "mode": "ephemeral", "execution": "isolated" },
    { "profile": "coding", "name": "impl-2", "mode": "ephemeral", "execution": "isolated" },
    { "profile": "coding", "name": "reviewer", "mode": "ephemeral" }
  ],
  "pipeline": [
    { "stage": "research", "agents": ["researcher"] },
    { "stage": "implement", "agents": ["impl-1", "impl-2"], "depends_on": ["research"] },
    { "stage": "review", "agents": ["reviewer"], "depends_on": ["implement"] }
  ]
}
```

Pipeline visualization (TUI and dashboard):

```
researcher --> impl-1 --> reviewer
               impl-2 --/
```

Each node shows: Agent name, status (waiting/working/done), current tier, cost so far. The pipeline is a Graph — the same Engine runs both single-Agent tasks and multi-Agent clusters.

---

## 12. Backbone: Relay + Mirage

The backbone is always-on infrastructure shared across all users. Deployed as a single container:

| Service | Image | What |
|---|---|---|
| Mirage | `ghcr.io/nunchi/mirage:latest` | Devnet chain (anvil) + relay WebSocket |
| Relay | Built into Mirage | Agent presence, message routing, Signal stream registry |

The relay is embedded in the Mirage container. One deployment covers both chain and relay. The roko workspace is optional — the relay and chain operate independently.

---

## 13. Monitoring and Health

### 13.1 Health Endpoints

| Endpoint | What |
|---|---|
| `GET /api/health` | Basic health check (status, version, uptime) |
| `GET /api/status` | Detailed status (Agents, plans, learning state) |
| `GET /api/metrics` | Prometheus-format metrics |

```
GET /api/health

Response:
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_secs": 3600,
  "agents_running": 3,
  "plans_active": 1
}
```

### 13.2 Lens-Based Monitoring

The Observe protocol (Lens system) provides built-in observability:

- **AgentLens**: Per-Agent metrics (turns, tokens, cost, latency)
- **PlanLens**: Plan execution progress (tasks completed, failed, pending)
- **GateLens**: Verify-protocol pass rates, threshold drift
- **RouterLens**: Model selection distribution, cost per model
- **MemoryLens**: Knowledge Signal counts, tier distribution, decay rates
- **CostLens**: Real-time cost telemetry per Cell, per Graph, per Agent

Lenses emit observation Signals consumed by the dashboard, TUI, or external monitoring systems. StateHub projections provide typed, universal views consumed by all surfaces.

### 13.3 Alerts

Alerts are configured in `roko.toml`:

```toml
[monitoring.alerts]
# Alert when any Agent exceeds daily budget
budget_exceeded = { threshold = 1.0, action = "pause_agent" }

# Alert when gate pass rate drops below threshold
gate_pass_rate = { threshold = 0.5, window = "1h", action = "notify" }

# Alert when inference latency exceeds threshold
inference_latency = { threshold_ms = 30000, action = "notify" }
```

---

## 14. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| D-1 | `roko serve` starts on :6677 with health check responding | Integration test: start, hit /api/health |
| D-2 | `roko daemon start/stop/status` lifecycle works via daemon-lifecycle Graph | Integration test: start daemon, check status, stop, verify stopped |
| D-3 | `roko daemon install` generates valid systemd/launchd config | Unit test: validate generated config syntax |
| D-4 | Daemon lifecycle Graph: each command routes to correct Cell | Unit test: DaemonCommand dispatches to spawn/shutdown/health/install |
| D-5 | Railway deploy: Dockerfile builds, health check passes | CI: build Docker image, run health check |
| D-6 | Fly.io deploy: fly.toml valid, machines start and respond | CI: validate config |
| D-7 | Docker Compose: services start, volumes mount correctly | Integration test |
| D-8 | Deploy-pipeline Graph: build -> gate -> deploy -> smoke -> notify | Integration test: full pipeline with mock deploy target |
| D-9 | Deploy-pipeline rollback: smoke-test failure triggers rollback Cell | Integration test: failing smoke -> rollback -> notify |
| D-10 | In-process: 50 concurrent Agents start without OOM | Load test: start 50 Agents, measure memory |
| D-11 | Isolated: Fly Machine created and connected to relay | Integration test with mock Fly API |
| D-12 | Execution-tier-selector Graph: untrusted Agents route to isolated | Unit test: untrusted -> isolated, trusted -> in-process |
| D-13 | WASM Cell loads, runs with fuel limit, terminates on exhaustion | Integration test: WASM Cell exceeds fuel -> terminated |
| D-14 | WASM ABI uses `cell-input`/`cell-output` (not `block-input`/`block-output`) | Compile check: wit-bindgen against `roko-cell.wit` |
| D-15 | WASM ABI: Cell input/output round-trips through wit-bindgen | Unit test |
| D-16 | WASM sandbox prevents unauthorized fs/net access | Security test: WASM Cell attempts unauthorized syscall -> trapped |
| D-17 | Progressive enhancement: native -> WASM runtime -> WASM standalone | Build all three targets, verify each runs |
| D-18 | Brain export: Agent state serialized to ~100KB-1MB file | Unit test: export, verify size and contents |
| D-19 | Brain export manifest includes version and source agent | Unit test: verify manifest fields |
| D-20 | Brain import: Imported state restores routing, thresholds, knowledge | Integration test: export A, import into B, verify B has A's learning |
| D-21 | Brain import with decay: Older knowledge starts at reduced balance | Unit test: import with decay-factor 0.8, verify balances at 80% |
| D-22 | Merkle-CRDT sync: Two instances converge after divergent learning | Integration test: A learns X, B learns Y, sync, both have X+Y |
| D-23 | Merkle-CRDT incremental sync: Only divergent subtrees exchanged | Unit test: measure sync payload size after small update (~1-10KB) |
| D-24 | CRDT GCounter: merge produces component-wise max | Unit test: two counters with different increments |
| D-25 | CRDT LWW-Register: merge selects most recent timestamp | Unit test: two registers with different timestamps |
| D-26 | CRDT Add-only set: merge produces union | Unit test: two sets with different elements |
| D-27 | Secrets resolved in 3-tier priority order (env > config > encrypted) | Unit test: set all three, verify env wins |
| D-28 | Secrets never in env vars for isolated Agents | Integration test: verify inference proxy used, no ANTHROPIC_API_KEY in child env |
| D-29 | Age encryption: secrets encrypted at rest, decrypted on read | Unit test: set secret, verify file is encrypted, read back matches |
| D-30 | Multi-instance: Two roko processes share relay, no Agent duplication | Integration test with relay mock |
| D-31 | Agent cluster pipeline: stages execute in dependency order | Integration test: 3-stage pipeline completes correctly |
| D-32 | Worker mode: `roko worker` connects to relay, executes assigned tasks | Integration test: spawn worker, dispatch task, verify result |
| D-33 | Health endpoints return correct data | Integration test: /api/health, /api/status, /api/metrics |
| D-34 | Alerts fire when thresholds exceeded | Unit test: simulate budget overrun -> pause_agent action triggered |
| D-35 | Self-healing supervisor Graph: crash -> dedup -> diagnose -> fix -> restart | Integration test: simulate crash, verify Graph fires correct Cells |
| D-36 | Supervisor circuit breaker: opens after 3 restarts in 5 minutes | Integration test: crash 3x, verify breaker opens, Graph halts |
| D-37 | Supervisor autofix limited to config changes | Unit test: ApplyFixCell rejects ChangeType::Code |
| D-38 | Supervisor autofix disabled by default | Unit test: default config has ROKO_SUPERVISOR_AUTOFIX=false |
| D-39 | Agent-wizard Graph: CLI/API/dashboard all fire same Graph | Integration test: create Agent via each surface, verify identical outcomes |
| D-40 | Bus topology: Solo uses in-process Bus, production uses relay-backed Bus | Unit test: verify BusTopology matches tier |
| D-41 | Tier-advisor Graph recommends correct tier based on agent count and relay config | Unit test: various inputs -> expected tier recommendations |

---

## Version History

| Version | Date | Changes |
|---|---|---|
| 3.1 | 2026-04-26 | Full kernel conformance: self-healing supervisor as Graph of Cells with circuit-breaker edge and config-only autofix constraint. DaemonCell expanded to daemon-lifecycle Graph. WASM ABI renamed block->cell (`cell-input`/`cell-output`, `[cell.impl]`). Bus topology documented per scaling tier. Agent creation wizard, deployment pipeline, execution tier selection, and tier advisor expressed as Graphs of Cells. Acceptance criteria numbered and expanded. |
| 3.0 | 2026-04-26 | Unified spec: added daemon lifecycle, secrets 3-tier priority, worker mode, self-healing supervisor. Expressed WASM packaging and brain export as Cell/Signal compositions. |
| 2.0 | 2026-04-20 | Merged deployment + scaling + WASM + brain export into single doc. |
| 1.0 | 2026-04-15 | Initial deployment architecture. |
