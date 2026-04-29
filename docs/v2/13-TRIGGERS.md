# 13 — Trigger System

> Event-driven Graph firing. Triggers listen for events, evaluate filters, and start Flows. Every trigger event is a Pulse on Bus. Triggers are persistent, declarative, and composable. Conductor watchers provide 10 battle-tested detection rules for agent stalls, loops, and resource exhaustion.

**Subsumes**: Cron jobs, webhooks, file watchers, event subscriptions, manual triggers, chain event listeners, conductor watchers, intervention system.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse duality, Bus), [02-CELL](02-CELL.md) (Cell trait, Trigger protocol), [03-GRAPH](03-GRAPH.md) (Graph/Flow), [11-CONNECTIVITY](11-CONNECTIVITY.md) (Connectors, chain events, finality)

---

## 1. Trigger Protocol

The Trigger protocol ([doc-02](02-CELL.md)) defines two operations: `arm` and `disarm`. A Trigger Cell is **push-based**: `arm()` sets up the event subscription and the Trigger publishes a `TriggerFired` Pulse on Bus when the event occurs. There is no `poll()` -- the system is event-driven end to end.

### 1.1 Push-Based Design

Each trigger kind uses the appropriate push mechanism:

| Kind | `arm()` action | Push mechanism |
|---|---|---|
| **Bus** | Subscribes to Bus topic | Bus subscription callback |
| **Webhook** | Registers Axum route on `:6677` | HTTP handler fires on request |
| **FileWatch** | Sets up `notify::RecommendedWatcher` | OS filesystem event callback |
| **Cron** | Registers timer with tokio interval | Timer fires at scheduled time |
| **ChainEvent** | Subscribes to chain indexer WebSocket | Indexer pushes matching events |
| **Manual** | No-op (awaits explicit API call) | API handler fires on request |
| **SignalPattern** | Subscribes to Store graduation topic | Evaluates pattern on each new Signal |

When an event fires, the Trigger publishes a `TriggerFired` Pulse on `trigger:{name}:fired`. The Trigger Engine subscribes to `trigger:*:fired` and spawns Flows. This replaces the poll loop with Bus subscription, consistent with the push-based Bus design throughout the system.

```rust
#[async_trait]
pub trait TriggerProtocol: Cell {
    /// Arm the trigger. Sets up event subscription (Bus topic, Axum route,
    /// notify watcher, timer, etc.). When the event fires, the Trigger
    /// publishes a `TriggerFired` Pulse on `trigger:{name}:fired`.
    async fn arm(&self, binding: &TriggerBinding, bus: Arc<dyn Bus>) -> Result<TriggerHandle>;

    /// Disarm the trigger. Unsubscribes, removes routes, cleans up resources.
    async fn disarm(&self, handle: TriggerHandle) -> Result<()>;
}

/// Handle to an armed trigger.
pub struct TriggerHandle {
    pub id: TriggerId,
    pub binding: TriggerBinding,
    pub armed_at: DateTime<Utc>,
    pub state: TriggerState,
}

pub enum TriggerState {
    Armed,
    Firing,
    Cooldown { until: DateTime<Utc> },
    Disarmed,
    Failed { error: String },
}

/// An event that fired from a trigger. Published as a Pulse on
/// `trigger:{name}:fired`. The Trigger Engine subscribes to
/// `trigger:*:fired` and spawns the bound Graph as a Flow.
pub struct TriggerEvent {
    pub trigger_id: TriggerId,
    pub fired_at: DateTime<Utc>,
    pub payload: Value,
    pub source: TriggerSource,
    /// Space this event originated in (for Space scoping, see S1.2).
    pub space_id: Option<SpaceId>,
    /// Trace ID for correlating the trigger event with the resulting Flow.
    pub trace_id: TraceId,
}

pub enum TriggerSource {
    Cron { expression: String },
    Webhook { method: String, path: String, headers: BTreeMap<String, String> },
    FileWatch { path: PathBuf, event: FileWatchEvent },
    Bus { topic: Topic, pulse_seq: u64 },
    ChainEvent { chain_id: u64, block_number: u64, tx_hash: String },
    Manual { user: Author },
    SignalPattern { matched_signals: Vec<SignalRef> },
}
```

### 1.2 Space Scoping

A Trigger defined within a Space is **scoped to that Space's partitions**:

1. **Bus observation**: the Trigger can only subscribe to Bus topics within the Space's Bus partition. A Trigger in `space:alpha` cannot observe topics in `space:beta`.
2. **Graph visibility**: the Trigger can only fire Graphs that are visible within the Space. This prevents a Trigger in a sandboxed Space from spawning Flows with elevated capabilities.
3. **Capability grants**: the resulting Flow runs with the Space's capability grants, not the global capability set. This ensures the Flow cannot exceed the Space's permission boundary.

The `space_id` field on `TriggerEvent` records which Space the event originated in. The Trigger Engine checks Space membership before spawning a Flow: if the binding has `space: Some(space_id)`, the Flow is spawned within that Space's capability boundary.

```rust
// Space-scoped trigger: can only observe and fire within space:alpha
[[triggers]]
name = "alpha-gate-failure"
kind = "bus"
filter = "gate.verdict.emitted"
space = "alpha"                     # scoped to space:alpha
graph = "plans/alpha-replan.toml"   # must be visible in space:alpha
```

---

## 2. TriggerBinding

A TriggerBinding is the persistent, TOML-defined configuration that connects an event source to a Graph. Bindings survive process restarts — they are stored in `.roko/triggers/` and re-armed on startup.

```rust
pub struct TriggerBinding {
    /// Unique name for this binding.
    pub name: String,

    /// The trigger kind and its configuration.
    pub kind: TriggerKind,

    /// The Graph to fire when the trigger activates.
    pub graph: GraphRef,

    /// Input mapping: how to transform the trigger event payload
    /// into input Signals for the Graph.
    pub input_mapping: Option<TriggerInputMapping>,

    /// Concurrency policy: what to do if the trigger fires while
    /// a previous Flow from this trigger is still running.
    pub concurrency: ConcurrencyPolicy,

    /// Event filter: conditions that must be met for the trigger
    /// to actually fire (beyond the kind-specific matching).
    pub filter: Option<TriggerFilter>,

    /// Whether this binding is currently enabled.
    pub enabled: bool,

    /// Space this trigger runs in (capability scoping).
    pub space: Option<SpaceId>,

    /// Auth configuration for the trigger source.
    pub auth: Option<TriggerAuth>,
}

pub struct TriggerInputMapping {
    /// Transform the trigger event payload into Graph input Signals.
    pub mappings: Vec<InputFieldMapping>,
}

pub struct InputFieldMapping {
    /// JSONPath expression selecting from the trigger event payload.
    pub from: String,
    /// Target field in the input Signal.
    pub to: String,
    /// Optional transformation.
    pub transform: Option<Expr>,
}
```

### Persistence

```
.roko/triggers/
  on-pr-opened.toml
  nightly-consolidation.toml
  file-watcher.toml
  bus-gate-failure.toml
```

On startup, the engine reads all `.toml` files in `.roko/triggers/` and arms each enabled binding. On shutdown, trigger state (e.g., last cron execution time) is written back.

---

## 3. Seven Built-in Trigger Kinds

### 3.1 Cron

Time-based triggers using cron expressions. Supports second-resolution cron syntax (6 fields) and timezone awareness.

```rust
pub struct CronTrigger {
    /// Cron expression (6 fields: sec min hour day month weekday).
    pub expression: String,

    /// Timezone for evaluation (default: UTC).
    pub timezone: String,

    /// Last execution time (for preventing double-fire on restart).
    pub last_fired: Option<DateTime<Utc>>,
}
```

```toml
[[triggers]]
name = "nightly-consolidation"
kind = "cron"
expression = "0 0 3 * * *"   # 3:00 AM daily
timezone = "America/New_York"
graph = "plans/dream-consolidation.toml"
```

### 3.2 Webhook

HTTP endpoint triggers. The engine registers routes on the HTTP control plane (`:6677`) for each webhook trigger.

```rust
pub struct WebhookTrigger {
    /// HTTP path to listen on (relative to /webhooks/).
    pub path: String,

    /// Allowed HTTP methods.
    pub methods: Vec<String>,

    /// Secret for signature verification (HMAC-SHA256).
    pub secret: Option<SecretRef>,

    /// IP allowlist (empty = allow all).
    pub ip_allowlist: Vec<String>,
}
```

```toml
[[triggers]]
name = "github-pr-opened"
kind = "webhook"
path = "/github/pr"
methods = ["POST"]
secret = { env = "GITHUB_WEBHOOK_SECRET" }
graph = "plans/code-review.toml"

[triggers.filter]
where = "payload.action == 'opened'"
```

### 3.3 FileWatch

File system event triggers using `notify::RecommendedWatcher`. Watches files or directories for changes.

```rust
pub struct FileWatchTrigger {
    /// Path to watch (file or directory).
    pub path: PathBuf,

    /// Whether to watch recursively (for directories).
    pub recursive: bool,

    /// File events to trigger on.
    pub events: Vec<FileWatchEvent>,

    /// Glob pattern for file name filtering.
    pub glob: Option<String>,
}

pub enum FileWatchEvent {
    Created,
    Modified,
    Deleted,
    Renamed,
    Any,
}
```

```toml
[[triggers]]
name = "plan-file-changed"
kind = "file_watch"
path = "plans/"
recursive = true
glob = "*.toml"
events = ["modified", "created"]
graph = "plans/validate-plan.toml"
concurrency = "skip"
```

### 3.4 Bus

Trigger on Pulses matching a topic filter. This is the primary mechanism for trigger chaining: one Flow publishes a Pulse, which triggers another Flow.

```rust
pub struct BusTrigger {
    /// Topic filter to match.
    pub filter: TopicFilter,

    /// Optional body predicate (Expr evaluated against Pulse body).
    pub body_predicate: Option<Expr>,
}
```

```toml
[[triggers]]
name = "on-gate-failure"
kind = "bus"
filter = "gate.verdict.emitted"
graph = "plans/gate-failure-replan.toml"
concurrency = "queue"

[triggers.filter]
where = "payload.hard_pass == false"
```

### 3.5 ChainEvent

Trigger on on-chain events (ERC-8004 identity updates, contract events, token transfers). Phase 2+ — requires a chain indexer connection.

```rust
pub struct ChainEventTrigger {
    /// Chain ID (e.g., 1 for Ethereum mainnet, 8453 for Base).
    pub chain_id: u64,

    /// Contract address to watch.
    pub contract: String,

    /// Event signature to match.
    pub event_signature: String,

    /// ABI for decoding event data.
    pub abi: Option<Value>,

    /// Finality requirement.
    pub finality: FinalityRequirement,
}

pub enum FinalityRequirement {
    /// Wait for finality oracle to confirm.
    Final,
    /// Accept quasi-finalized (high confidence but not proven).
    QuasiFinalized,
    /// Accept immediately (may be reorged).
    Reversible,
}
```

```toml
[[triggers]]
name = "identity-updated"
kind = "chain_event"
chain_id = 8453
contract = "0x..."
event_signature = "IdentityUpdated(address,bytes32)"
finality = "quasi_finalized"
graph = "plans/update-agent-identity.toml"
```

### 3.6 Manual

Triggers that fire only when explicitly invoked via CLI, API, or TUI. Used for on-demand operations.

```rust
pub struct ManualTrigger {
    /// Description shown in CLI/TUI.
    pub description: String,

    /// Input schema for the manual payload.
    pub input_schema: Option<TypeSchema>,

    /// Whether to prompt for confirmation before firing.
    pub confirm: bool,
}
```

```toml
[[triggers]]
name = "manual-deploy"
kind = "manual"
description = "Deploy the current build to staging"
confirm = true
graph = "plans/deploy-staging.toml"
```

### 3.7 SignalPattern

Trigger when a pattern of Signals appears in Store. Unlike Bus triggers (which match individual Pulses), SignalPattern triggers match aggregate conditions over stored Signals.

**Push mechanism**: the SignalPatternTrigger subscribes to the `store.signal.graduated` Bus topic (emitted whenever a Pulse graduates to Signal). On each graduation event, it evaluates the `StoreQuerySpec` against Store. This avoids polling -- the trigger only checks when new Signals actually arrive.

```rust
pub struct SignalPatternTrigger {
    /// Query specification mapping to Store protocol's `query()` and
    /// `query_similar()` methods. See StoreQuerySpec below.
    pub query: StoreQuerySpec,

    /// Minimum number of matching Signals required to fire.
    pub min_matches: usize,

    /// Time window for the pattern (matches must occur within this window).
    pub window: Duration,
}

/// Maps to the Store protocol's query operations.
/// Evaluated against Store on each graduation event.
pub struct StoreQuerySpec {
    /// Signal kind filter. Maps to Store `query()` kind parameter.
    pub kind: Option<Kind>,

    /// Minimum Score threshold (any of the 5 axes).
    /// Maps to Store `query()` score filter.
    pub min_score: Option<f64>,

    /// HDC similarity query. When set, uses Store `query_similar()`
    /// with this vector and `min_similarity` threshold.
    pub hdc_similarity: Option<HdcSimilaritySpec>,

    /// Only match Signals created within this window from now.
    pub time_window: Option<Duration>,

    /// Tag filter. Matches Signals with any of these tags.
    pub tags: Option<Vec<String>>,

    /// Payload field predicates (evaluated as Expr against Signal payload).
    pub field_predicates: Option<Vec<Expr>>,
}

pub struct HdcSimilaritySpec {
    /// Reference HDC vector to compare against.
    pub reference_vector: HdcVector,
    /// Minimum cosine similarity threshold (0.0..=1.0).
    pub min_similarity: f64,
}
```

```toml
[[triggers]]
name = "failure-cluster"
kind = "signal_pattern"
graph = "plans/investigate-failures.toml"

[triggers.query]
kind = "Finding"
min_score = 0.7
tags = ["severity:high"]
min_matches = 3
window_seconds = 300

# Optional: HDC similarity matching
# [triggers.query.hdc_similarity]
# reference = "path/to/reference-vector.bin"
# min_similarity = 0.8
```

---

## 4. Concurrency Policies

When a trigger fires while a previous Flow from the same trigger is still running, the concurrency policy determines behavior.

```rust
pub enum ConcurrencyPolicy {
    /// Queue the new firing. Execute after the current Flow completes.
    /// Maximum queue depth is configurable (default: 10).
    Queue { max_depth: Option<usize> },

    /// Skip the new firing. Log the skip event.
    Skip,

    /// Cancel the currently running Flow and start a new one.
    CancelRunning,

    /// Run in parallel. Multiple Flows from the same trigger coexist.
    /// Maximum concurrent is configurable (default: unlimited).
    Parallel { max_concurrent: Option<usize> },
}
```

| Policy | Use Case |
|---|---|
| Queue | Webhook events that must all be processed (e.g., PR comments) |
| Skip | File watch debounce (only care about latest state) |
| CancelRunning | Deploy triggers (new deploy supersedes in-progress) |
| Parallel | Independent event processing (each event is self-contained) |

```toml
# Queue policy with max depth
[[triggers]]
name = "pr-comment"
kind = "webhook"
concurrency = { kind = "queue", max_depth = 50 }

# Skip policy (debounce)
[[triggers]]
name = "file-changed"
kind = "file_watch"
concurrency = "skip"

# CancelRunning (latest wins)
[[triggers]]
name = "deploy"
kind = "manual"
concurrency = "cancel_running"

# Parallel with limit
[[triggers]]
name = "job-submitted"
kind = "bus"
concurrency = { kind = "parallel", max_concurrent = 5 }
```

---

## 5. Trigger Chaining via Bus

Triggers compose through Bus: the output of one Flow publishes Pulses that trigger another Flow. This creates event-driven pipelines without explicit wiring between Graphs.

```
Flow A completes
    |
    +-> Publishes Pulse on "flow.{run_a}.completed"
    |
    +-> Bus trigger B matches "flow.*.completed"
    |   +-> Starts Flow B
    |
    +-> Bus trigger C matches "flow.*.completed"
        +-> Starts Flow C (parallel)
```

### Example: PR review pipeline via chaining

```toml
# Trigger 1: PR opened -> fetch diff
[[triggers]]
name = "pr-opened"
kind = "webhook"
path = "/github/pr"
graph = "plans/fetch-diff.toml"

[triggers.filter]
where = "payload.action == 'opened'"

# Trigger 2: diff fetched -> run review
[[triggers]]
name = "diff-ready"
kind = "bus"
filter = "flow.*.completed"
graph = "plans/code-review.toml"

[triggers.filter]
where = "payload.graph_name == 'fetch-diff'"

# Trigger 3: review completed -> post comment
[[triggers]]
name = "review-ready"
kind = "bus"
filter = "flow.*.completed"
graph = "plans/post-pr-comment.toml"

[triggers.filter]
where = "payload.graph_name == 'code-review'"
```

---

## 6. Filtering

Triggers support layered filtering to control when they actually fire.

```rust
pub struct TriggerFilter {
    /// Filter by event kind (for Bus and webhook triggers).
    pub event_kind: Option<Vec<Kind>>,

    /// Predicate expression evaluated against the event payload.
    /// Must return true for the trigger to fire.
    pub where_clause: Option<Expr>,

    /// Pattern matching against payload fields.
    pub matches: Option<BTreeMap<String, Value>>,

    /// Debounce: suppress repeated firings within this window.
    /// Only the last event in the window fires.
    pub debounce: Option<Duration>,

    /// Rate limit: maximum firings per time window.
    pub rate_limit: Option<RateLimit>,

    /// Custom filter Cell: evaluate via a user-defined Cell.
    pub custom_filter: Option<CellRef>,
}

pub struct RateLimit {
    /// Maximum number of firings.
    pub max_fires: u32,

    /// Time window for the rate limit.
    pub window: Duration,

    /// What to do when rate-limited.
    pub on_limit: RateLimitAction,
}

pub enum RateLimitAction {
    /// Drop the event silently.
    Drop,
    /// Queue the event for processing after the window.
    Queue,
    /// Log a warning and drop.
    Warn,
}
```

```toml
[[triggers]]
name = "important-signals"
kind = "bus"
filter = "gate.verdict.emitted"
graph = "plans/process-verdict.toml"

[triggers.filter]
event_kind = ["Verdict"]
where = "payload.reward > 0.8"
debounce_ms = 5000
rate_limit = { max_fires = 10, window_seconds = 60, on_limit = "warn" }
```

### Filter evaluation order

Filters are evaluated in order from cheapest to most expensive:

1. **event_kind**: O(1) check against Pulse Kind
2. **matches**: O(n) field comparison against payload
3. **where_clause**: Expr evaluation (bounded by 100ms timeout)
4. **debounce**: time check against last fire timestamp
5. **rate_limit**: counter check against window
6. **custom_filter**: Cell execution (most expensive, skipped if earlier filters reject)

---

## 7. Events as Pulses on Bus

All trigger system events are published as Pulses on Bus. This makes the trigger system observable via the same Lens and telemetry infrastructure as the rest of the runtime.

| Event | Topic | Graduates to Signal? |
|---|---|---|
| Trigger armed | `trigger.{name}.armed` | Yes |
| Trigger fired | `trigger.{name}.fired` | Yes |
| Trigger filtered (event rejected) | `trigger.{name}.filtered` | No (noise) |
| Trigger skipped (concurrency) | `trigger.{name}.skipped` | Yes |
| Trigger queued (concurrency) | `trigger.{name}.queued` | No |
| Trigger rate-limited | `trigger.{name}.rate_limited` | Yes |
| Trigger error | `trigger.{name}.error` | Yes |
| Trigger disarmed | `trigger.{name}.disarmed` | Yes |
| Flow started by trigger | `trigger.{name}.flow.started` | Yes |
| Flow completed by trigger | `trigger.{name}.flow.completed` | Yes |

### Graduation policy

Trigger events that affect system state (armed, fired, error, disarmed) graduate to Signals for audit. Routine events (filtered, queued) do not. Rate-limited events graduate because they indicate capacity issues.

---

## 8. Auth and Secrets

Trigger authentication uses the workspace secret store. Secrets are never stored in TOML — only references.

```rust
pub enum TriggerAuth {
    /// No authentication.
    None,

    /// HMAC-SHA256 signature verification (webhooks).
    HmacSha256 {
        /// Reference to secret in the workspace secret store.
        secret: SecretRef,
        /// HTTP header containing the signature.
        header: String,
    },

    /// Bearer token (API keys).
    BearerToken {
        secret: SecretRef,
    },

    /// Mutual TLS.
    MutualTls {
        cert: PathBuf,
        key: SecretRef,
    },
}

pub enum SecretRef {
    /// Read from environment variable.
    Env(String),

    /// Read from the workspace secret store.
    Store { key: String },

    /// Read from a file.
    File(PathBuf),
}
```

```toml
# Webhook with HMAC-SHA256 verification
[[triggers]]
name = "github-webhook"
kind = "webhook"
path = "/github"

[triggers.auth]
kind = "hmac_sha256"
secret = { env = "GITHUB_WEBHOOK_SECRET" }
header = "X-Hub-Signature-256"
```

---

## 9. Conductor Watchers (10 Rules)

The conductor subsystem provides 10 battle-tested detection rules for agent stalls, loops, and resource exhaustion. These are internal triggers that fire interventions against running agents, distinct from the external Trigger system above but sharing the same Bus-based event model.

Source: `crates/roko-conductor/src/watchers/`. All 10 watchers exist in the codebase.

### 9.1 Watcher Table

| # | Watcher | Trigger Condition | Action |
|---|---|---|---|
| 1 | **GhostTurn** | No output + fast turn (<5s) + not in gating | Restart agent |
| 2 | **ReviewLoop** | 3+ consecutive REVISE verdicts + gates pass | Skip remaining reviews |
| 3 | **IterationLoop** | Iteration >= 6 + cycling strategist/implementer | Force advance |
| 4 | **TestFailureBudget** | 70%+ tests pass but some fail | Force advance (good enough) |
| 5 | **SilenceTimeout** | No output for 180s | Restart agent |
| 6 | **CompileFailThreshold** | 3+ consecutive compile failures | Force advance |
| 7 | **TaskStall** | Single task blocking for 300s | Restart agent |
| 8 | **ContextPressure** | Prompt >80% of context window | Trim context |
| 9 | **PhaseTimeout** | Phase exceeds 30min wall-clock | Restart |
| 10 | **CooldownFilter** | Last intervention within 120s | Skip (debounce) |

Each watcher returns `Option<Intervention>`:

```rust
pub struct Intervention {
    pub tier: InterventionTier,
    pub watcher: String,
    pub target_role: Option<String>,
    pub message: String,
    pub action: InterventionAction,
}

pub enum InterventionTier {
    Info,
    Warning,
    Critical,
}

pub enum InterventionAction {
    Restart,
    ForceAdvance,
    SkipPhase,
    TrimContext,
    Noop,
}
```

### 9.2 Intervention policies

Two built-in policies select which intervention to apply when multiple watchers fire:

- **BanditPolicy**: Uses a multi-armed bandit (EFE-based) to select the intervention most likely to resolve the issue. Learns from past outcomes.
- **WorstSeverityPolicy**: Always applies the highest-severity intervention. Simple, deterministic, conservative.

### 9.3 CooldownFilter

The CooldownFilter is special: it is not a detection rule but a debounce mechanism. When any intervention fires, the CooldownFilter suppresses further interventions for 120 seconds. This prevents intervention storms where multiple watchers trigger cascading restarts.

### 9.4 Configurable thresholds

All watcher thresholds are configurable in `roko.toml` under `[conductor]`:

```toml
[conductor]
ghost_turn_max_secs = 5
review_loop_max_consecutive = 3
iteration_loop_max = 6
test_failure_budget_pass_rate = 0.70
silence_timeout_secs = 180
compile_fail_max_consecutive = 3
task_stall_secs = 300
context_pressure_percent = 80
phase_timeout_secs = 1800
cooldown_filter_secs = 120
```

If a key is missing, the hardcoded default from the watchers table applies. The conductor also exposes a circuit breaker (Holt forecasting) for detecting systemic failure patterns across watchers.

---

## 10. Trigger Lifecycle

```
Defined (TOML binding created)
    |
    +-> arm(bus) --> Armed --> Subscription active (Bus/Axum/notify/timer)
    |                   |
    |                   +-> Event pushed --> Filter passes --> publish TriggerFired Pulse
    |                   |                          |              on trigger:{name}:fired
    |                   |                          |
    |                   |                          |     Engine subscribes to trigger:*:fired
    |                   |                          |         |
    |                   |                          |         +-> ConcurrencyPolicy check
    |                   |                          |         |       |
    |                   |                          |         |       +-> Queue/Skip/Cancel/Parallel
    |                   |                          |         |       |
    |                   |                          |         |       +-> Start Flow
    |                   |                          |
    |                   |                          +-> Filter rejects --> log + continue
    |                   |
    |                   +-> Error --> retry (3x) --> Failed
    |                   |
    |                   +-> disarm() --> Disarmed (unsubscribe, cleanup)
    |
    +-> Disabled (never armed)
```

---

## 11. CLI Surface

The trigger system is fully manageable via CLI.

### Trigger management

| Command | What it does |
|---|---|
| `roko trigger list` | List all trigger bindings with status (armed/disarmed) |
| `roko trigger show <name>` | Show trigger details, recent firings, concurrency state |
| `roko trigger create <file>` | Create a trigger binding from TOML |
| `roko trigger enable <name>` | Enable a disabled trigger |
| `roko trigger disable <name>` | Disable an active trigger (disarm) |
| `roko trigger fire <name> [payload]` | Manually fire a trigger with optional payload |
| `roko trigger delete <name>` | Remove a trigger binding |
| `roko trigger history <name>` | Show firing history with Flow references |
| `roko trigger test <name> <payload>` | Dry-run: evaluate filters without starting a Flow |

### Examples

```bash
# List all triggers
roko trigger list

# Show details of a specific trigger
roko trigger show github-pr-opened

# Create a new trigger from TOML
roko trigger create triggers/my-trigger.toml

# Manually fire a trigger with payload
roko trigger fire manual-deploy '{"branch": "main", "env": "staging"}'

# Test filter evaluation without firing
roko trigger test on-gate-failure '{"hard_pass": false, "reward": 0.2}'

# View firing history
roko trigger history nightly-consolidation --limit 10
```

### API surface

| Method | Path | What |
|---|---|---|
| `GET` | `/triggers` | List all trigger bindings |
| `GET` | `/triggers/{name}` | Get trigger details |
| `POST` | `/triggers` | Create trigger binding |
| `PUT` | `/triggers/{name}` | Update trigger binding |
| `DELETE` | `/triggers/{name}` | Delete trigger binding |
| `POST` | `/triggers/{name}/fire` | Manually fire |
| `POST` | `/triggers/{name}/enable` | Enable trigger |
| `POST` | `/triggers/{name}/disable` | Disable trigger |
| `GET` | `/triggers/{name}/history` | Firing history |
| `POST` | `/webhooks/{path}` | Webhook endpoint (dynamic per binding) |

---

## 12. Crate Mapping

| Crate | Responsibility |
|---|---|
| `roko-core` | TriggerProtocol trait, TriggerBinding, TriggerEvent, TriggerSource, ConcurrencyPolicy, TriggerFilter, StoreQuerySpec |
| `roko-conductor` | 10 watchers, intervention system (BanditPolicy, WorstSeverityPolicy), circuit breaker, cooldown filter |
| `roko-runtime` | Trigger engine (arm/disarm, Bus subscription to `trigger:*:fired`, Flow spawning) |
| `roko-serve` | Webhook HTTP routes, trigger API endpoints |
| `roko-cli` | `roko trigger` subcommands, trigger configuration in roko.toml |
| `roko-chain` | ChainEventTrigger (EVM log topic subscription) |

---

## 13. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| TR-1 | `TriggerProtocol` trait compiles with `arm`, `disarm` (push-based, no poll) | Compile check |
| TR-2 | `TriggerBinding` persists to `.roko/triggers/` and survives restart | Integration test: create binding, restart, verify armed |
| TR-3 | Cron trigger fires at the scheduled time | Integration test with mock clock |
| TR-4 | Cron trigger does not double-fire on restart | Integration test: fire, restart within same minute, verify single fire |
| TR-5 | Webhook trigger registers HTTP route and receives events | Integration test: POST to webhook endpoint, verify trigger fires |
| TR-6 | Webhook HMAC-SHA256 verification rejects invalid signatures | Unit test |
| TR-7 | FileWatch trigger fires on file modification | Integration test: modify file, verify trigger fires |
| TR-8 | FileWatch glob filter excludes non-matching files | Unit test |
| TR-9 | Bus trigger fires on matching Pulse | Integration test: publish Pulse, verify trigger fires |
| TR-10 | Bus trigger body predicate filters correctly | Unit test: matching and non-matching payloads |
| TR-11 | ChainEvent trigger fires on matching on-chain event | Integration test (mocked chain indexer) |
| TR-12 | Manual trigger fires via CLI and API | Integration test |
| TR-13 | SignalPattern trigger fires when pattern threshold met via StoreQuerySpec | Integration test: add Signals, verify fires at min_matches |
| TR-13a | SignalPattern `StoreQuerySpec` maps to Store `query()` and `query_similar()` | Unit test: verify query mapping |
| TR-13b | SignalPattern evaluates on `store.signal.graduated` Bus topic (no polling) | Integration test: graduate Signal, verify pattern evaluation |
| TR-14 | ConcurrencyPolicy Queue: second fire queues behind first | Integration test |
| TR-15 | ConcurrencyPolicy Skip: second fire is dropped | Integration test |
| TR-16 | ConcurrencyPolicy CancelRunning: first Flow cancelled on second fire | Integration test |
| TR-17 | ConcurrencyPolicy Parallel: both Flows run concurrently | Integration test |
| TR-18 | Trigger chaining: Flow A completes -> Bus trigger B fires -> Flow B starts | Integration test (full chain) |
| TR-19 | Filter debounce: rapid fires produce single execution | Integration test: 10 events in 1s, debounce=2s, verify single fire |
| TR-20 | Filter rate_limit: max_fires respected within window | Integration test |
| TR-21 | Filter evaluation order: cheapest first, custom_filter last | Unit test with instrumented filters |
| TR-22 | All trigger events published as Pulses on correct topics | Integration test |
| TR-22a | Trigger Engine subscribes to `trigger:*:fired` and spawns Flows (push-based) | Integration test |
| TR-22b | `arm()` sets up push subscription (Bus/Axum/notify/timer), no poll loop | Integration test |
| TR-23 | Graduation policy: `trigger.*.fired` graduates, `trigger.*.filtered` does not | Integration test |
| TR-24 | SecretRef.Env reads from environment | Unit test |
| TR-25 | CLI `roko trigger list` shows all bindings with status | Integration test |
| TR-26 | CLI `roko trigger fire` starts a Flow | Integration test |
| TR-27 | CLI `roko trigger test` evaluates filters without starting Flow | Integration test |
| TR-28 | API endpoints return correct responses | Integration test per endpoint |
| TR-29 | Input mapping transforms trigger payload into Graph input Signals | Unit test |
| TR-30 | Disabled trigger is not armed on startup | Integration test |
| TR-30a | Space-scoped trigger only observes Bus topics within its Space partition | Integration test: trigger in space:alpha ignores space:beta events |
| TR-30b | Space-scoped trigger only fires Graphs visible within its Space | Integration test: trigger cannot spawn Flow for out-of-scope Graph |
| TR-30c | Flow spawned by space-scoped trigger runs with Space's capability grants | Integration test: verify capability intersection |
| TR-30d | `TriggerEvent.space_id` records originating Space | Unit test |
| TR-31 | All 10 conductor watchers implemented and registered | Unit test per watcher |
| TR-32 | CooldownFilter prevents intervention storms | Integration test with rapid triggers |
| TR-33 | Conductor watcher thresholds configurable via roko.toml | Integration test: override default, verify new threshold used |
| TR-34 | Interventions logged with tier/watcher/target for observability | Integration test |
