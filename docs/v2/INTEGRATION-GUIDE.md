# Roko Integration Guide

A complete reference for configuring, integrating, and operating Roko. Covers
every section of `roko.toml`, the `.roko/` directory layout, provider recipes,
MCP configuration, the self-hosting workflow, error recovery, event
subscriptions, learning configuration, and deployment.

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [Self-Hosting Workflow (End-to-End)](#2-self-hosting-workflow-end-to-end)
3. [roko.toml Schema Reference](#3-rokotoml-schema-reference)
4. [.roko/ Directory Layout](#4-roko-directory-layout)
5. [Provider Configuration](#5-provider-configuration)
6. [Model Registry](#6-model-registry)
7. [MCP Server Configuration](#7-mcp-server-configuration)
8. [Agent Manifests and Safety Contracts](#8-agent-manifests-and-safety-contracts)
9. [Gate Pipeline Configuration](#9-gate-pipeline-configuration)
10. [Learning Configuration](#10-learning-configuration)
11. [Event System and Subscriptions](#11-event-system-and-subscriptions)
12. [HTTP Control Plane](#12-http-control-plane)
13. [Error Recovery](#13-error-recovery)
14. [Deployment Configuration](#14-deployment-configuration)
15. [Environment Variables Reference](#15-environment-variables-reference)
16. [WorkflowEngine Integration (Programmatic)](#16-workflowengine-integration-programmatic)
17. [Key File Locations](#17-key-file-locations)
18. [Known Gaps](#18-known-gaps)

---

## 1. Architecture Overview

### The Layer Stack

Roko is structured as a strict service hierarchy. Every entry point — CLI,
HTTP server, and ACP — shares the same runtime core.

```
Entry points
  ┌──────────┐  ┌──────────┐  ┌──────────┐
  │  CLI     │  │  Serve   │  │  ACP     │
  │  roko-cli│  │ :6677    │  │  adapter │
  └────┬─────┘  └────┬─────┘  └────┬─────┘
       │              │              │
       └──────────────▼──────────────┘
              WorkflowEngine
         (roko-runtime/workflow_engine.rs)
                    │
         ┌──────────▼──────────┐
         │  PipelineStateV2    │  ← pure state machine, no I/O
         │  EffectDriver       │  ← executes side-effects
         └──────────┬──────────┘
                    │  calls
       ┌────────────┼────────────────────┐
       ▼            ▼                    ▼
 ModelCaller  PromptAssembler      GateRunner   FeedbackSink
 (roko-agent) (roko-compose)      (roko-gate)   (roko-learn)
```

Foundation traits live in `crates/roko-core/src/foundation.rs`:

| Trait | Implementor | Purpose |
|---|---|---|
| `ModelCaller` | `ModelCallService` (roko-agent) | LLM dispatch with routing |
| `PromptAssembler` | `PromptAssemblyService` (roko-compose) | 9-layer system prompt |
| `FeedbackSink` | `FeedbackService` (roko-learn) | Episode and feedback recording |
| `GateRunner` | `GateService` (roko-gate) | 7-rung gate pipeline |
| `EventConsumer` | `JsonlLogger`, `SseAdapter`, ACP | Runtime event observation |
| `AffectPolicy` | `DaimonPolicy` (roko-daimon) | Behavioral dispatch modulation |
| `EffectExecutor` | `EffectDriver` (roko-runtime) | Translates state-machine actions to I/O |

### State Machine + Effect Driver Pattern

The core design keeps decisions separate from side-effects:

```
PipelineStateV2::step(PipelineInput) -> PipelineOutput
                                              │
                              EffectDriver executes the output
                                              │
                              returns PipelineInput back to loop
```

`PipelineStateV2` is a pure Rust struct with no `async` methods and no I/O.
It serializes to JSON for checkpoint/resume without any special handling.
`EffectDriver` owns the `Arc<dyn ModelCaller>`, `Arc<dyn GateRunner>`, etc.
and translates `PipelineOutput` variants into real work.

### Provider Dispatch

The provider layer (`crates/roko-agent/src/`) supports seven protocol families:

| `ProviderKind` | TOML value | What it talks to |
|---|---|---|
| `AnthropicApi` | `"anthropic_api"` | Anthropic Messages API over HTTP |
| `ClaudeCli` | `"claude_cli"` | `claude` CLI subprocess (stream-JSON) |
| `OpenAiCompat` | `"openai_compat"` | Any OpenAI-compatible HTTP endpoint |
| `CursorAcp` | `"cursor_acp"` | Cursor Agent Client Protocol |
| `PerplexityApi` | `"perplexity_api"` | Perplexity Sonar HTTP API |
| `GeminiApi` | `"gemini_api"` | Google Gemini API (native) |
| `CerebrasApi` | `"cerebras_api"` | Cerebras inference API |

### Event-Driven Observability

Every significant action emits a `RuntimeEvent` to a process-global broadcast
bus (`EventBus<RuntimeEvent>`). Consumers register at startup and receive
non-blocking fire-and-forget notifications.

```
WorkflowEngine / EffectDriver
     │
     │  emit_runtime_event(RuntimeEvent::AgentSpawned { ... })
     ▼
EventBus<RuntimeEvent>
     ├── JsonlLogger   → .roko/state/events.json
     ├── SseAdapter    → HTTP GET /v1/events stream
     └── AcpAdapter    → per-run ACP protocol messages
```

Event kinds:

- Lifecycle: `WorkflowStarted`, `PhaseTransition`, `WorkflowCompleted`
- Agent: `AgentSpawned`, `AgentOutput`, `AgentCompleted`, `AgentFailed`
- Gates: `GateStarted`, `GatePassed`, `GateFailed`
- Feedback: `FeedbackRecorded`
- Persistence: `StateCheckpointed`

Every event carries a `run_id` so consumers can correlate across concurrent
runs.

---

## 2. Self-Hosting Workflow (End-to-End)

This section walks through the complete loop that roko uses to develop itself.
Every command exists in the CLI today.

### Prerequisites

```bash
# Rust 1.91+ is required (alloy deps)
rustup update stable
rustup default stable

# Build the binary
cd /path/to/roko
cargo build -p roko-cli --release

# Or use cargo run for development
alias roko='cargo run -p roko-cli --'
```

### Step 1: Initialize the workspace

```bash
roko init
```

Creates `.roko/` with the standard directory layout and a starter `roko.toml`
in the workspace root. Edit `roko.toml` to configure providers before
proceeding (see [Section 3: roko.toml Schema Reference](#3-rokotoml-schema-reference)).

### Step 2: Capture a work item

```bash
roko prd idea "Wire knowledge store into CascadeRouter for model selection"
```

Creates a dated idea file in `.roko/prd/ideas/`. Ideas are lightweight —
just a title and timestamp.

```bash
roko prd list   # view all ideas and PRDs
```

### Step 3: Draft a PRD

```bash
roko prd draft new "knowledge-informed-routing"
```

Launches a Claude agent that reads the idea, reads the current codebase via
the code-intelligence MCP, and writes a structured PRD into
`.roko/prd/drafts/knowledge-informed-routing.md`.

The PRD includes a mandatory **Repository Grounding** section that lists the
specific files and types the implementation will touch. This grounding is
injected as context when generating the plan in step 5.

### Step 4: Enrich with research (optional)

```bash
roko research enhance-prd knowledge-informed-routing
```

Launches a Perplexity research agent that queries for relevant prior art,
papers, and API documentation, then appends a **Research** section to the
draft PRD. Optional but significantly improves plan quality for novel
subsystems.

### Step 5: Review and promote the PRD

```bash
# Inspect the draft
roko prd draft list

# Edit if needed
roko prd draft edit knowledge-informed-routing

# Promote to published
roko prd draft promote knowledge-informed-routing
```

Promoting moves the file to `.roko/prd/published/`. If `prd.auto_plan = true`
in `roko.toml`, plan generation (step 6) triggers automatically via the
`prd_publish_subscriber` background task in `roko-serve`.

### Step 6: Generate an implementation plan

```bash
roko prd plan knowledge-informed-routing
```

A Claude agent reads the published PRD (including the Repository Grounding
section), reads the relevant source files, and produces a
`plans/knowledge-informed-routing/tasks.toml` with a DAG of implementation
tasks.

Each task in the TOML has:

- `id` — stable identifier for resumption
- `title` — human-readable description
- `depends_on` — list of task ids that must complete first
- `prompt` — the implementation instruction given to the agent

Validate the plan without executing:

```bash
roko plan validate plans/knowledge-informed-routing/
```

### Step 7: Execute the plan

```bash
roko plan run plans/knowledge-informed-routing/
```

This starts the main orchestration loop (`crates/roko-cli/src/orchestrate.rs`).
For each task:

1. Build the 9-layer system prompt via `PromptAssemblyService`
2. Route to a model via `CascadeRouter`
3. Dispatch the agent via `ModelCallService`
4. Run the gate pipeline via `GateService`
5. On gate failure: attempt autofix or replan
6. On success: record feedback, update learning state, checkpoint
7. Commit changes and advance to the next task

Progress is visible in real time on the TUI.

### Step 8: Resume if interrupted

```bash
roko plan run plans/knowledge-informed-routing/ \
  --resume .roko/state/executor.json
```

The `PipelineStateV2` checkpoint is written atomically after each phase
transition. Resumption restores the exact pipeline state so work is never
duplicated.

### Step 9: Watch progress

```bash
roko dashboard
```

Opens the interactive ratatui TUI. F1–F7 cycle through tabs:

| Tab | Content |
|---|---|
| F1 | Active tasks and phase indicators |
| F2 | Agent output stream |
| F3 | Gate results per task |
| F4 | Episode history |
| F5 | Learning state (router, experiments) |
| F6 | Cost and token usage |
| F7 | System health |

The TUI uses a file watcher (`notify::RecommendedWatcher`) to pick up changes
to `.roko/` without polling. Updates appear in under 250 ms.

### Step 10: Inspect learning state

```bash
roko learn all          # full dump
roko learn router       # cascade router bandit state
roko learn experiments  # A/B prompt experiments
roko learn efficiency   # per-turn efficiency events
roko learn episodes     # episode history
```

After a few successful runs the cascade router accumulates enough observations
to graduate from the static routing table (stage 1, < 50 observations) into
the confidence-based stage (stage 2, 50–200 observations) and eventually the
LinUCB bandit stage (stage 3, > 200 observations).

---

## 3. roko.toml Schema Reference

The schema is defined in `crates/roko-core/src/config/schema.rs` (top-level
`RokoConfig` struct). All fields carry serde defaults; you only need to write
sections that differ from the defaults. The file is loaded from
`<workdir>/roko.toml` and falls back to `RokoConfig::default()` if missing.

Two secret-resolution passes run automatically after parsing:

1. `${VAR}` interpolation — expands `${ENV_VAR}` references in provider
   config strings.
2. `*_file` resolution — reads secrets from file paths in `extra_headers`
   whose keys end with `_file`.

### Top-level version fields

```toml
config_version = 2    # layout version for migration tooling (current: 2)
schema_version = 2    # schema version for compatibility checks (current: 2)
```

If you have an old `config_version = 1` file, run `roko config migrate` to
upgrade.

### Hot-reload vs. restart

Most sections support hot-reload when `roko serve` is running:

| Section | Requires restart? |
|---|---|
| `budget`, `gates`, `routing`, `learning` | Hot-reload |
| `demurrage`, `scheduler`, `watcher` | Hot-reload |
| `subscriptions`, `conductor`, `attention`, `goals` | Hot-reload |
| `agent`, `project`, `serve`, `providers`, `models`, `server` | Restart required |

### [project]

Project-level metadata.

```toml
[project]
name = "my-project"          # string, default: "roko-project"
root = "."                   # relative or absolute project root, default: "."
fresh_base_branch = "main"   # git branch for fresh worktree creation, default: "main"
# default_domain = "coding"  # default task domain (optional)
```

**`default_domain`** accepts: `"coding"`, `"research"`, `"chain"`, `"docs"`,
or `"ops"`. When set, tasks without an explicit domain inherit this.

### [prd]

PRD lifecycle settings.

```toml
[prd]
auto_plan = false   # bool: auto-generate plan when a PRD is promoted, default: false
```

When `auto_plan = true`, `roko serve` listens for PRD publish events and
triggers `roko prd plan <slug>` automatically.

### [agent]

Agent / model configuration, including per-role overrides.

```toml
[agent]
default_model = "claude-sonnet-4-6"   # default LLM slug, default: "claude-sonnet-4-6"
default_backend = "anthropic_api"     # provider kind, default: auto-detected from env
default_effort = "medium"             # reasoning effort: "low"/"medium"/"high"/"max"
temperament = "balanced"              # default agent temperament
context_limit_k = 200                 # context window in thousands of tokens
bare_mode = true                      # pass --bare to skip built-in system prompt
fallback_model = "claude-haiku-4-5"  # retry model on spawn failure (optional)
mode = "ephemeral"                    # agent lifecycle: "ephemeral"/"persistent"/"reactive"
domain = "coding"                     # domain profile name (optional)

# legacy CLI path (used when [providers] is empty):
# command = "claude"
# args = ["--dangerously-skip-permissions"]
# timeout_ms = 120000
# env = [["ANTHROPIC_API_KEY", "${ANTHROPIC_API_KEY}"]]

# CaMeL dual-LLM isolation (SAFE-07):
[agent.data_llm]
model = "claude-haiku-3-5"   # smaller model for untrusted content isolation
max_tokens = 4096
temperature = 0.0
strip_tool_calls = true      # Data LLM cannot produce tool calls
sanitize_input = true        # strip known injection patterns before sending
# output_schema = { ... }    # optional JSON Schema for Data LLM output validation
```

**`default_backend`** auto-detection: if `ANTHROPIC_API_KEY` is set in the
environment, defaults to `"anthropic_api"`; otherwise defaults to `"claude"`
(the Claude CLI subprocess path).

**`mode`** values:
- `"ephemeral"` — agent runs a task then exits (default, used by `plan run`)
- `"persistent"` — agent runs continuously until explicitly stopped (`roko agent start`)
- `"reactive"` — agent sleeps until a trigger fires (webhook, cron, event)

**`temperament`** values: `"conservative"`, `"balanced"`, `"exploratory"`.
Higher exploration rates cause the AffectPolicy to set `Bypass` cache policy
on model calls to avoid stale responses.

#### Per-role overrides

```toml
[agent.roles.implementer]
role = "implementer"           # explicit role label (defaults to section name)
model = "claude-opus-4-6"      # override model for this role
backend = "anthropic_api"      # override backend
effort = "high"                # override reasoning effort
temperament = "exploratory"    # override temperament
context_limit_k = 200          # override context window
tools = ["read", "edit", "bash", "git-*"]   # role-local tool whitelist

[agent.roles.implementer.budget]
max_tokens_per_turn = 12000
max_cost_usd_cents_per_turn = 500   # 500 cents = $5.00

[agent.roles.implementer.thresholds]
gate_pass_rate_floor = 0.65    # minimum acceptable gate pass rate for this role

[agent.roles.implementer.routing_overrides]
force_backend = "claude"       # pin to a specific provider family
force_tier = "focused"         # pin to a complexity tier
```

Every field in `RoleOverride` is optional. Missing fields inherit the
`[agent]` defaults.

#### Policy manifests

```toml
[agent]
policy_manifests = [".roko/roles/manifest.toml"]
```

Policy manifests contain per-role YAML safety contracts and tool allowlists.
Loaded before each agent dispatch.

### [providers.*]

Provider registry. Each key is a provider name; the value describes how to
reach the provider.

```toml
[providers.my-provider]
kind = "anthropic_api"     # ProviderKind value (see table above)
base_url = "https://..."   # HTTP base URL (for HTTP providers)
api_key_env = "MY_API_KEY" # env var holding the API key
command = "claude"         # CLI command (for ClaudeCli)
args = ["--flag"]          # extra CLI args
timeout_ms = 120000        # hard request/subprocess timeout (default: 120000)
ttft_timeout_ms = 15000    # time-to-first-token timeout (default: 15000)
connect_timeout_ms = 5000  # TCP connect timeout (default: 5000)
max_concurrent = 4         # max concurrent requests (optional)

# Extra HTTP headers (e.g. for custom auth or routing):
[providers.my-provider.extra_headers]
"X-Custom-Header" = "value"
# Secret from file (key ending in _file reads file contents):
"Authorization_file" = "/run/secrets/my-token"
```

See [Section 5: Provider Configuration](#5-provider-configuration) for
ready-to-use provider recipes.

### [models.*]

Model registry. Each key is a logical model name; the value binds it to a
provider and specifies capabilities.

```toml
[models.sonnet]
provider = "anthropic"         # key into [providers.*]
slug = "claude-sonnet-4-6"     # model ID sent on the wire
context_window = 200000        # context window in tokens (default: 128000)
max_output = 8192              # max output tokens (optional)
supports_tools = true          # default: true
supports_thinking = false      # reasoning/thinking mode support
supports_vision = false        # image input support
supports_web_search = false    # provider-side web search
supports_mcp_tools = false     # MCP tool support
supports_caching = false       # provider-side context caching
tool_format = "openai_json"    # "openai_json" or "anthropic_blocks"
cost_input_per_m = 3.0         # $ per million input tokens (optional)
cost_output_per_m = 15.0       # $ per million output tokens (optional)
cost_cache_read_per_m = 0.3    # $ per million cache-read tokens (optional)
cost_cache_write_per_m = 3.75  # $ per million cache-write tokens (optional)
```

See [Section 6: Model Registry](#6-model-registry) for examples.

### [gates]

Verification gate settings.

```toml
[gates]
clippy_enabled = true    # run clippy/lint gate, default: true
skip_tests = false       # skip test gate entirely, default: false
max_iterations = 3       # max gate retry iterations before giving up, default: 3

# Per-domain gate overrides (keys are domain labels):
[gates.domain_gates]
research = ["shell:true"]    # research tasks skip compile/test
docs = ["shell:true"]        # docs tasks skip compile/test
```

See [Section 9: Gate Pipeline Configuration](#9-gate-pipeline-configuration)
for the full rung table.

### [routing]

Model routing configuration for the CascadeRouter.

```toml
[routing]
mode = "auto_override"            # routing mode, default: "auto_override"
algorithm = "linucb"              # "linucb" or "thompson", default: "linucb"
discount_factor = 0.99            # Thompson discount factor for non-stationarity
fast_task_model = "claude-haiku-4-5"      # model for mechanical/fast tasks
standard_task_model = "claude-sonnet-4-6" # model for standard tasks
complex_task_model = "claude-opus-4-6"    # model for architectural tasks
context_strategy = "mcp_first"    # "mcp_first", "hybrid", "inline_heavy"

# Reward scalarization weights (must sum to ~1.0):
[routing.weights]
quality = 0.5    # default: 0.5
cost = 0.3       # default: 0.3
latency = 0.2    # default: 0.2

# Per-tier weight overrides (optional):
[routing.weights.mechanical]
quality = 0.3
cost = 0.5
latency = 0.2

[routing.weights.architectural]
quality = 0.7
cost = 0.2
latency = 0.1
```

The CascadeRouter progresses through three stages automatically:

| Stage | Observations | Strategy |
|---|---|---|
| 1: Static | < 50 | Hardcoded role → model table from TOML |
| 2: Confidence | 50–200 | Empirical pass-rate + confidence interval |
| 3: UCB/LinUCB | > 200 | Full contextual bandit |

State persists to `.roko/learn/cascade-router.json`.

### [pipeline]

Complexity-to-pipeline mapping. Each band configures whether a strategist
agent runs before implementation and whether reviewer agents run after.

```toml
[pipeline.mechanical]
strategist = false
reviewers = false
reviewer_mode = "quick"   # "quick" or "full"
max_iterations = 1

[pipeline.focused]
strategist = false
reviewers = false
reviewer_mode = "quick"
max_iterations = 2

[pipeline.integrative]
strategist = true
reviewers = true
reviewer_mode = "quick"
max_iterations = 2

[pipeline.architectural]
strategist = true
reviewers = true
reviewer_mode = "full"    # full: architect + auditor + scribe reviewers
max_iterations = 3
```

**`reviewer_mode`** values:
- `"quick"` — single quick-pass reviewer
- `"full"` — full review suite (architect, auditor, scribe)

### [budget]

Spend and token budget settings. Violations result in
`GatewayError::BudgetExceeded` which halts the workflow cleanly.

```toml
[budget]
max_plan_usd = 25.0         # max dollars per plan run, default: 25.0
max_turn_usd = 3.0          # max dollars per agent turn, default: 3.0
prompt_token_budget = 10000 # token budget for prompt composition, default: 10000
```

### [conductor]

Conductor (meta-orchestrator) settings controlling parallelism, auto-fix
behavior, and role toggles.

```toml
[conductor]
max_agents = 8                  # max concurrent agents, default: 8
max_parallel_plans = 1          # max concurrently running plans, default: 1
parallel_enabled = false        # enable parallel task execution, default: false
express_mode = false            # skip strategist/reviewer phases, default: false
auto_advance_batch = true       # auto-advance to next batch when ready, default: true
auto_merge_on_complete = false  # auto-merge when plan completes, default: false
pre_plan = false                # run pre-planning phase, default: false
max_auto_fix_attempts = 3       # gate failure autofix attempts, default: 3
auto_fix_model = "claude-haiku-4-5"  # model for autofix turns
conductor_model = "claude-sonnet-4-6"  # model for conductor reasoning (optional)
warm_implementers_per_plan = 1  # keep N implementers warm, default: 1

# Toggle which reviewer roles are active:
[conductor.enabled_roles]
architect = true
auditor = true
scribe = true
critic = true

# Per-watcher threshold overrides for anomaly detection:
[conductor.watchers.compile_fail_repeat]
max_repeats = 3     # halt after N consecutive compile failures

[conductor.watchers.context_window_pressure]
warn_threshold = 0.75      # warn at 75% context utilization
critical_threshold = 0.90  # escalate at 90% context utilization

[conductor.watchers.cost_overrun]
warn_usd = 1.0      # warn above $1.00 per turn
critical_usd = 5.0  # escalate above $5.00 per turn

[conductor.watchers.ghost_turn]
min_output_tokens = 1   # turns below this are "ghost" (empty) turns
max_consecutive = 3     # halt after N consecutive ghost turns

[conductor.watchers.iteration_loop]
max_iterations = 3      # halt after N iterations without progress

[conductor.watchers.review_loop]
max_rejections = 3      # halt after N reviewer rejections

[conductor.watchers.spec_drift]
max_ratio = 0.25        # flag if > 25% of output diverges from spec

[conductor.watchers.stuck_pattern]
max_identical_actions = 4   # flag stuck if agent repeats same action N times

[conductor.watchers.test_failure_budget]
min_failure_increase = 1    # flag if test failures increase by N or more

[conductor.watchers.time_overrun]
alert_ratio = 0.80          # alert when 80% of time budget is consumed
```

### [learning]

Learning subsystem configuration.

```toml
[learning]
auto_playbook_refresh = true       # refresh playbook after successful tasks, default: true
knowledge_file_intel = true        # inject file difficulty profiles, default: true
knowledge_warnings = true          # inject knowledge warnings, default: true
knowledge_wave_context = true      # cross-task wave context propagation, default: true
knowledge_error_patterns = true    # error signature pattern matching, default: true
learning_min_occurrences = 2       # min occurrences before promoting rules, default: 2
file_intel_max_entries = 15        # max file-intel entries per task, default: 15
warning_max_entries = 5            # max warning entries per task, default: 5
replan_on_gate_failure = true      # trigger plan revision on repeated gate failure
replan_max_per_plan = 2            # max gate-failure replans per plan, default: 2
replan_gate_attempts = 3           # consecutive failures before replan, default: 3
use_lookahead_router = false       # enable lookahead cost-saving tier downgrades
lookahead_threshold = 0.7          # success probability floor for downgrade, default: 0.7
```

See [Section 10: Learning Configuration](#10-learning-configuration) for
details on episodes, playbooks, and the cascade router.

### [demurrage]

Knowledge demurrage — Gesellian decay applied to playbook entries and
knowledge store items so stale heuristics naturally fade.

```toml
[demurrage]
rate_per_hour = 0.01          # exponential decay rate per hour, default: 0.01
min_balance = 0.1             # entries below this are deprioritized, default: 0.1
freeze_threshold = 0.05       # balance at which entries freeze into cold storage
thaw_balance = 0.6            # starting balance when a frozen entry is resurrected
max_balance = 5.0             # maximum balance from reinforcement
death_threshold = 0.01        # recency factor below which entries are considered dead
freeze_before_delete = true   # preserve for resurrection rather than deleting
gc_interval_secs = 0          # run GC every N seconds (0 = manual only)

# Per-kind rate multipliers (e.g. warnings decay faster):
[demurrage.kind_rate_multipliers]
warning = 2.0
insight = 0.5
```

### [attention]

Attention token budget allocation for prompt layers.

```toml
[attention]
max_tokens_per_layer = 4096    # max tokens per prompt layer, default: 4096
utilization_target = 0.85      # context window utilization target (0.0-1.0), default: 0.85
auction_enabled = false        # enable attention auction between layers, default: false
task_reserve_tokens = 512      # tokens reserved for task context, default: 512
```

### [immune]

Anomaly detection and quarantine settings.

```toml
[immune]
quarantine_threshold = 0.8   # anomaly score above which outputs are quarantined
max_quarantined = 50         # escalate if this many items are quarantined
auto_reject = false          # auto-reject quarantined outputs (vs. hold for review)
taint_levels = ["low", "medium", "high"]   # taint classification levels
```

### [temporal]

Time horizon preferences for the task planner.

```toml
[temporal]
max_depth = 5                    # max planning lookahead depth, default: 5
epoch_secs = 3600                # epoch duration for temporal batching, default: 3600
enforce_allen_relations = true   # enforce Allen temporal relations between tasks
```

### [goals]

Goal hierarchy configuration.

```toml
[goals]
max_active = 10              # max active goals at any level, default: 10
correctness_weight = 0.7     # correctness vs. speed weight (0.0-1.0), default: 0.7
completion_threshold = 0.95  # min completion ratio for "done", default: 0.95
prune_threshold = 0.1        # prune goals with priority below this, default: 0.1
```

### [energy]

Compute budget pool with per-tier caps.

```toml
[energy]
pool_usd = 50.0            # total compute budget pool in USD, default: 50.0
per_task_cap_usd = 0.0     # per-task cost cap (0.0 = no cap), default: 0.0
metabolism_rate = 0.1      # fraction of budget replenished per hour, default: 0.1

# Per-tier cost multipliers:
[energy.tier_caps]
cheap = 0.5
standard = 1.0
premium = 3.0
```

### [tui]

Terminal UI preferences.

```toml
[tui]
refresh_rate_ms = 250   # TUI refresh interval in milliseconds, default: 250
```

### [serve]

HTTP API serving options.

```toml
[serve]
port = 6677               # port override (falls back to [server].port), optional
share_ttl_days = 7        # shared transcript retention period in days, default: 7
terminal_enabled = false  # expose PTY terminal routes (shell access!), default: false
auto_orchestrate = true   # orchestrate follow-up work on publish events, default: true

[serve.auth]
enabled = false           # require X-Api-Key header on /api/* routes, default: false
api_key = ""              # shared API key (legacy single-key mode)
# Named keys with scopes:
# [[serve.auth.api_keys]]
# name = "github-actions"
# key_hash = "<sha256-hex>"     # SHA-256 of the plaintext key
# scope = "agent:write"         # "admin", "agent:write", "read"
# created_at = "2026-04-29T00:00:00Z"
# expires_at = "2027-04-29T00:00:00Z"  # optional

[serve.deploy]
provider = "railway"                  # deployment provider, default: "railway"
environment = [                       # required env vars for deployment
  "GITHUB_TOKEN",
  "GITHUB_WEBHOOK_SECRET",
  "SLACK_BOT_TOKEN",
  "SLACK_SIGNING_SECRET",
]

# Post-deploy webhook registrations:
[[serve.deploy.webhooks]]
provider = "github"
owner = "my-org"
repo = "my-repo"
```

### [server]

HTTP server / gateway settings.

```toml
[server]
bind = "127.0.0.1"   # bind address, default: "127.0.0.1"
port = 6677          # port number, default: 6677
cors_origins = []    # allowed CORS origins (empty = permissive), default: []
auth_token = ""      # optional bearer token for authentication
```

### [deploy]

Cloud deployment configuration.

```toml
[deploy]
backend = "manual"                           # "railway-api", "railway-cli", "manual"
railway_api_token = "..."                    # Railway API token (optional)
project_id = "..."                           # Railway project ID (optional)
environment_id = "..."                       # Railway environment ID (optional)
worker_image = "ghcr.io/nunchi-trade/roko-worker:latest"  # Docker image for workers
default_region = "us-west1"                  # default deployment region (optional)
```

### [scheduler]

Cron scheduler configuration.

```toml
[scheduler]
[[scheduler.cron]]
name = "weekly-digest"
expression = "0 9 * * MON"              # standard cron expression
signal_kind = "scheduler:cron:weekly-digest"  # engram kind emitted when fires
# metadata = { key = "value" }          # extra structured metadata (optional)
```

### [webhooks]

Webhook ingress configuration.

```toml
[webhooks.github]
secret = "my-webhook-secret"   # shared secret for X-Hub-Signature-256 verification
```

### [subscriptions]

Event subscriptions — each subscription fires when a matching signal arrives.

```toml
[[subscriptions]]
template = "prd-publisher"       # agent template name
trigger = "prd.published"        # engram kind glob to match
concurrency_limit = 1            # max concurrent dispatches, default: 1
cooldown_secs = 0                # min interval between dispatches, default: 0
debounce_ms = 0                  # debounce window in milliseconds, default: 0
enabled = true                   # whether the subscription is active

# Filter (all fields accept globs, or a list of globs):
[subscriptions.filter]
repo = ["my-org/my-repo"]        # match repo names
branch = ["main", "release/*"]  # match branch/ref names
path = ["src/**/*.rs"]           # match changed file paths
label = ["bug", "enhancement"]   # match issue/PR labels
author = ["bot-*"]               # match author logins

# Typed trigger (overrides plain trigger for firing mechanism):
[subscriptions.trigger_config]
type = "cron"
schedule = "*/30 * * * *"

# OR:
[subscriptions.trigger_config]
type = "file_watch"
paths = ["src/", "crates/"]
extensions = ["rs", "toml"]
recursive = true

# OR:
[subscriptions.trigger_config]
type = "webhook"
event = "push"
```

### [watcher]

File-system watcher configuration. The watcher emits signals when watched
paths change.

```toml
[watcher]
[[watcher.paths]]
directory = "src/"
include = ["*.rs", "*.toml"]   # accepts string or list of strings
exclude = ["target/"]
```

### [tools]

Global tool allowlist/denylist and domain profiles.

```toml
[tools]
allow = ["bash", "web_fetch"]     # always available regardless of role
deny = ["write_file"]             # never available regardless of role

[tools.profiles.coding]
extra_tools = ["bash", "edit_file", "write_file"]
excluded_tools = []

[tools.profiles.research]
extra_tools = ["web_search", "web_fetch"]
excluded_tools = ["write_file", "edit_file"]
```

The effective tool set for a domain is:
`(base_tools + extra_tools + global_allow) - excluded_tools - global_deny`

### [chain]

On-chain integration settings (Phase 2+).

```toml
[chain]
rpc_url = "https://mirage-devnet.up.railway.app"
chain_id = 1
wallet_key = "0x..."                  # hex-encoded private key
identity_registry = "0x..."           # ERC-8004 IdentityRegistry address
reputation_registry = "0x..."         # ERC-8004 ReputationRegistry address
validation_registry = "0x..."         # ERC-8004 ValidationRegistry address
agent_registry = "0x..."              # AgentRegistry address
bounty_market = "0x..."               # BountyMarket address
deployer = "0x..."                    # deployer/funder address
```

### [relay]

Relay registration for workspace auto-discovery.

```toml
[relay]
url = "wss://relay.nunchi.dev"       # relay WebSocket URL
workspace_name = "will-dev"          # human-readable workspace name (defaults to hostname)
public_url = "https://..."           # public URL of this roko instance
heartbeat_interval_secs = 30         # presence heartbeat interval, default: 30
```

### [gemini]

Gemini-specific model settings.

```toml
[gemini]
default_model = "gemini-2.5-flash"
grounding_model = "gemini-2.5-pro"
code_exec_model = "gemini-2.5-flash"
embed_model = "text-embedding-004"
use_free_tier = false
thinking_level = "medium"          # "minimal", "low", "medium", "high"
enable_context_caching = false

# Per-category safety settings:
[[gemini.safety_settings]]
category = "HARM_CATEGORY_HATE_SPEECH"
threshold = "BLOCK_NONE"
```

### [perplexity]

Perplexity-specific search settings.

```toml
[perplexity]
default_search_model = "sonar-pro"
default_research_model = "sonar-deep-research"
default_reasoning_model = "sonar-reasoning-pro"
default_embed_model = "r-rerank-english-v3"
search_recency_filter = "year"     # "hour"/"day"/"week"/"month"/"year"
academic_mode = false
search_domain_filter = []          # restrict to specific domains
return_images = false
return_related_questions = true
```

### [[agents]]

Multi-agent startup definitions for `roko up`.

```toml
[[agents]]
name = "my-agent"
domain = "coding"
prompt = "You are an expert Rust systems programmer..."
model = "claude-sonnet-4-6"    # optional model override
chain_rpc = "https://..."      # optional chain RPC override
enabled = true
```

### [oneirography]

Dream art generation pipeline (experimental).

```toml
[oneirography]
enabled = false
provider = "dall-e-3"   # image generation provider
variants = 3            # image variants per dream cycle
base_reserve = 0.01     # base reserve price for affect-reactive auctions
base_duration_seconds = 3600
```

---

## 4. .roko/ Directory Layout

The `.roko/` directory stores all runtime state. Its structure is defined in
`crates/roko-fs/src/layout.rs` (`RokoLayout` struct). The version is tracked
in `.roko/VERSION` (currently `1`).

```
.roko/
  VERSION               # layout format version (integer)
  engrams.jsonl         # main engram/signal log (append-only)
  signals.jsonl         # legacy name for engrams.jsonl (pre-rename)
  custody.jsonl         # append-only custody audit chain
  witness.jsonl         # append-only witness DAG log

  runtime/              # process lifecycle
    roko.pid            # PID of the running roko process
    roko.lock           # advisory lock file

  memory/               # learned knowledge
    episodes.jsonl      # per-turn episode records
    playbook.toml       # active learned playbook (techniques)
    skills/             # learned skill files

  plans/                # plan-level enrichment artifacts
    {plan_id}/          # one directory per plan

  runs/                 # per-run data
    {run_id}/           # one directory per run
      metrics.jsonl     # metrics log for this run
      traces/           # trace files for this run

  state/                # orchestrator state
    executor.json       # executor checkpoint for crash recovery
    events.json         # event log snapshot
    sessions/           # per-session state
      {session_id}/     # one directory per session

  config/               # local config overrides
    config.toml         # local config file

  cache/                # cached artifacts
    cargo-target/       # shared cargo target directory
    context-pack-cache/ # cached context packs

  learn/                # learning subsystem data
    efficiency.jsonl    # per-turn efficiency events
    cascade-router.json # model routing bandit state
    experiments.json    # prompt experiment store (A/B tests)
    gate-thresholds.json  # adaptive gate threshold state
    knowledge-scores.json # knowledge entry admission scores
    section-effects.json  # prompt section effectiveness scores

  repos/                # per-repo layout (for multi-repo setups)
    {repo_name}/        # mirrors the root layout for a specific repo

  prd/                  # PRD storage (created by roko init)
    ideas/              # raw ideas
    drafts/             # PRDs in progress
    published/          # promoted PRDs
    plans/              # generated implementation plans

  research/             # research artifacts
    {topic}/            # research output per topic
```

Key path accessors from `RokoLayout`:

| Path | Method | Purpose |
|---|---|---|
| `.roko/` | `root()` | Data directory root |
| `.roko/memory/episodes.jsonl` | `episodes_path()` | Episode log |
| `.roko/memory/playbook.toml` | `playbook_path()` | Active playbook |
| `.roko/state/executor.json` | `executor_snapshot()` | Executor checkpoint |
| `.roko/learn/cascade-router.json` | `cascade_router_path()` | Router state |
| `.roko/learn/experiments.json` | `experiments_path()` | Experiment store |
| `.roko/learn/efficiency.jsonl` | `efficiency_path()` | Efficiency events |
| `.roko/custody.jsonl` | `custody_log()` | Custody audit chain |
| `.roko/witness.jsonl` | `witness_log()` | Witness DAG |
| `.roko/runtime/roko.pid` | `pid_file()` | PID file |

---

## 5. Provider Configuration

### Anthropic Claude CLI (recommended for local development)

Uses the `claude` CLI subprocess. Requires Claude Code CLI to be installed.

```toml
[providers.anthropic]
kind = "claude_cli"
command = "claude"
# args = ["--dangerously-skip-permissions"]  # if needed
# timeout_ms = 300000   # longer timeout for complex tasks
```

### Anthropic API (direct HTTP)

Uses the Anthropic Messages API over HTTP. Set `ANTHROPIC_API_KEY` in the
environment.

```toml
[providers.anthropic]
kind = "anthropic_api"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
timeout_ms = 120000
ttft_timeout_ms = 15000
connect_timeout_ms = 5000
max_concurrent = 4
```

If `ANTHROPIC_API_KEY` is set and no `[providers]` section exists, roko
synthesizes an `anthropic_api` provider automatically.

### OpenAI

```toml
[providers.openai]
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[models.gpt41]
provider = "openai"
slug = "gpt-4.1"
context_window = 1047576
supports_tools = true
cost_input_per_m = 2.0
cost_output_per_m = 8.0

[models.gpt41-mini]
provider = "openai"
slug = "gpt-4.1-mini"
context_window = 1047576
supports_tools = true
cost_input_per_m = 0.4
cost_output_per_m = 1.6
```

### Cerebras (ultra-fast inference)

```toml
[providers.cerebras]
kind = "cerebras_api"
base_url = "https://api.cerebras.ai/v1"
api_key_env = "CEREBRAS_API_KEY"

[models.llama-fast]
provider = "cerebras"
slug = "llama-3.3-70b"
context_window = 128000
supports_tools = true
cost_input_per_m = 0.59
cost_output_per_m = 0.99
```

Cerebras is useful for the `fast_task_model` in `[routing]` because latency
is significantly lower than API-based providers.

### Google Gemini

Using the OpenAI-compatible endpoint (recommended):

```toml
[providers.gemini]
kind = "openai_compat"
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key_env = "GEMINI_API_KEY"

[models.gemini-flash]
provider = "gemini"
slug = "gemini-2.5-flash"
context_window = 1000000
supports_tools = true
supports_thinking = true
```

Using the native Gemini API (for grounding and code execution):

```toml
[providers.gemini-native]
kind = "gemini_api"
api_key_env = "GEMINI_API_KEY"

[gemini]
default_model = "gemini-2.5-flash"
grounding_model = "gemini-2.5-pro"
thinking_level = "medium"
enable_context_caching = true
```

### Local Ollama

```toml
[providers.ollama]
kind = "openai_compat"
base_url = "http://localhost:11434/v1"
# api_key_env is not needed for Ollama

[models.llama-local]
provider = "ollama"
slug = "llama3.1"
context_window = 128000
supports_tools = false
```

### Perplexity (research tasks only)

```toml
[providers.perplexity]
kind = "perplexity_api"
api_key_env = "PERPLEXITY_API_KEY"

[models.sonar]
provider = "perplexity"
slug = "sonar-pro"
supports_search = true
supports_citations = true

[models.deep-research]
provider = "perplexity"
slug = "sonar-deep-research"
supports_async = true
supports_search = true

[perplexity]
default_search_model = "sonar-pro"
default_research_model = "sonar-deep-research"
search_recency_filter = "month"
return_related_questions = true
```

### Using extra headers (custom auth)

```toml
[providers.my-gateway]
kind = "openai_compat"
base_url = "https://my-gateway.internal/v1"
# Read secret from environment variable reference:
[providers.my-gateway.extra_headers]
"Authorization" = "Bearer ${MY_GATEWAY_TOKEN}"

# Or read from a file (key must end in _file):
[providers.my-gateway.extra_headers]
"Authorization_file" = "/run/secrets/gateway-token"
```

---

## 6. Model Registry

The model registry decouples logical model names from provider-specific slugs.
The CascadeRouter refers to models by alias; changing the underlying provider
slug requires no change to routing configuration.

### Minimum model registry

```toml
[models.fast]
provider = "anthropic"
slug = "claude-haiku-4-5"

[models.standard]
provider = "anthropic"
slug = "claude-sonnet-4-6"

[models.powerful]
provider = "anthropic"
slug = "claude-opus-4-6"
```

Then in `[routing]`:

```toml
[routing]
fast_task_model = "fast"
standard_task_model = "standard"
complex_task_model = "powerful"
```

### Full-featured model entry

```toml
[models.sonnet]
provider = "anthropic"
slug = "claude-sonnet-4-6"
context_window = 200000
max_output = 8192
supports_tools = true
supports_thinking = false
supports_vision = true
supports_caching = true
supports_mcp_tools = true
tool_format = "anthropic_blocks"   # or "openai_json"
cost_input_per_m = 3.0
cost_output_per_m = 15.0
cost_input_per_m_high = 3.0        # high-context pricing tier
cost_output_per_m_high = 15.0
cost_cache_read_per_m = 0.3
cost_cache_write_per_m = 3.75
tokenizer_ratio = 1.0              # ratio vs. o200k_base tokenizer
max_tools = 64                     # tools before behavior degrades
```

### OpenRouter routing overrides

```toml
[models.sonnet-openrouter]
provider = "openrouter"
slug = "anthropic/claude-sonnet-4-6"

[models.sonnet-openrouter.provider_routing]
sort = "throughput"          # "price", "throughput", "latency"
order = ["Anthropic", "AWS"]
allow_fallbacks = true
max_price = 0.05             # max cost per token
```

---

## 7. MCP Server Configuration

MCP (Model Context Protocol) servers extend the tools available to agents.
Roko passes MCP configuration to providers that support it (the Claude CLI
and Anthropic API backends).

### Creating an MCP config file

The MCP config follows the Claude Desktop format:

```json
{
  "mcpServers": {
    "code-intel": {
      "command": "cargo",
      "args": ["run", "-p", "roko-mcp-code", "--"],
      "env": {}
    },
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_TOKEN": "${GITHUB_TOKEN}"
      }
    },
    "filesystem": {
      "command": "npx",
      "args": [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "/path/to/allowed/dir"
      ]
    }
  }
}
```

The code-intelligence MCP server (`roko-mcp-code`) provides these tools to
agents:

- `list_files` — list files in a directory
- `read_file` — read file contents
- `search_code` — grep the codebase
- `get_symbols` — extract symbols from a source file

### Referencing the MCP config

Pass the config file path to `roko plan run` or set it in the dispatcher
config in `orchestrate.rs`. For the legacy `[agent]` section:

```toml
[agent]
command = "claude"
# MCP is passed via --mcp-config flag in the agent dispatcher
```

In practice, MCP config is assembled programmatically in
`crates/roko-cli/src/orchestrate.rs` using `McpConfig` and
`McpServerConfig` structs from `roko-agent`:

```rust
let mcp_config = McpConfig {
    servers: vec![
        McpServerConfig {
            name: "code-intel".into(),
            command: "cargo".into(),
            args: vec!["run".into(), "-p".into(), "roko-mcp-code".into(), "--".into()],
            env: HashMap::new(),
        },
    ],
};
```

### roko-mcp-code (built-in code intelligence)

The built-in MCP server:

```bash
# Start it directly
cargo run -p roko-mcp-code --

# Use it via Claude CLI
claude --mcp-config mcp-config.json "analyze this codebase"
```

### Third-party MCP servers

Any MCP server can be added. Common ones:

```json
{
  "mcpServers": {
    "slack": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-slack"],
      "env": {
        "SLACK_BOT_TOKEN": "${SLACK_BOT_TOKEN}",
        "SLACK_TEAM_ID": "${SLACK_TEAM_ID}"
      }
    },
    "postgres": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-postgres", "${DATABASE_URL}"]
    }
  }
}
```

---

## 8. Agent Manifests and Safety Contracts

### Agent definitions in roko.toml

For multi-agent deployments, declare agents directly in `roko.toml`:

```toml
[[agents]]
name = "code-reviewer"
domain = "coding"
prompt = "You are a senior Rust code reviewer focused on correctness and safety."
model = "claude-sonnet-4-6"
enabled = true

[[agents]]
name = "research-agent"
domain = "research"
prompt = "You are a research specialist. Use web search to gather information."
model = "sonar-pro"
enabled = true
```

Start all enabled agents with:

```bash
roko up
```

### Per-role safety contracts

Safety contracts are YAML manifests referenced via `agent.policy_manifests`.
They constrain what actions a role is allowed to take:

```toml
[agent]
policy_manifests = [".roko/roles/manifest.toml"]
```

A manifest file (`.roko/roles/manifest.toml`):

```toml
[[roles]]
name = "implementer"
allowed_tools = ["read_file", "edit_file", "bash", "git_commit"]
denied_tools = ["web_fetch", "spawn_agent"]
max_file_writes_per_turn = 20
require_gate_before_commit = true

[[roles]]
name = "researcher"
allowed_tools = ["web_search", "web_fetch", "read_file"]
denied_tools = ["edit_file", "write_file", "bash"]
max_turns = 10
```

When a YAML manifest is missing for a role, the agent falls back to
permissive defaults.

### Tool role allowlists

In addition to manifests, per-role tool lists can be set directly in
`roko.toml`:

```toml
[agent.roles.implementer]
tools = ["read_file", "edit_file", "bash", "git_commit", "write_file"]

[agent.roles.reviewer]
tools = ["read_file", "bash"]

[agent.roles.researcher]
tools = ["web_search", "web_fetch", "read_file"]
```

---

## 9. Gate Pipeline Configuration

### 7-rung gate pipeline

Gates run in rung order, stopping at the first failure. Rung 0 (compile) is
never skipped regardless of adaptive threshold state.

| Rung | Name | Gate | What it checks |
|---|---|---|---|
| 0 | compile | `CompileGate` | `cargo build --workspace` (or equivalent) |
| 1 | clippy | `ClippyGate` | `cargo clippy --workspace --no-deps -- -D warnings` |
| 2 | test | `TestGate` | `cargo test --workspace` |
| 3 | diff | `git:diff` | `git diff --stat` (sanity check) |
| 4 | fmt | `FormatCheckGate` | `cargo fmt --check` |
| 5 | shell | `ShellGate` | Custom shell command |
| 6 | judge | `StubJudgeGate` | LLM judge (stub — not wired to live model) |

### Enabling and disabling gates

```toml
[gates]
clippy_enabled = true   # disable to skip rung 1
skip_tests = false      # true to skip rung 2
max_iterations = 3      # max retry iterations on gate failure
```

### Custom shell gates

Custom gates wrap any shell command:

```toml
[gates.domain_gates]
# Domain "security" runs cargo audit instead of clippy:
security = ["shell:cargo audit --deny warnings"]
```

In `WorkflowRunConfig` (programmatic API):

```rust
use roko_core::foundation::ShellGateCommand;

WorkflowRunConfig {
    shell_gates: vec![
        ShellGateCommand {
            program: "cargo".to_string(),
            args: vec!["audit".to_string(), "--deny".to_string(), "warnings".to_string()],
            timeout_ms: 120_000,
        },
    ],
    // ...
}
```

### Adaptive thresholds

`AdaptiveThresholds` tracks per-rung pass streaks with an EMA. When a rung
has passed consistently, it is skipped until a failure is observed. State
persists to `.roko/learn/gate-thresholds.json`.

Tune with:

```bash
roko learn tune gates
```

Override per-role floor:

```toml
[agent.roles.implementer.thresholds]
gate_pass_rate_floor = 0.65   # never skip rungs unless pass rate exceeds this
```

### Gate failure replanning

When gate failures exhaust the autofix budget and the iteration limit,
`learning.replan_on_gate_failure` triggers a plan revision:

```toml
[learning]
replan_on_gate_failure = true
replan_max_per_plan = 2      # max plan revisions per plan
replan_gate_attempts = 3     # consecutive failures before revision triggers
```

The replan emits `RokoEvent::PlanRevision` on the global event bus, which
triggers a new planning agent pass with the gate failure context injected.

---

## 10. Learning Configuration

All learning state lives in `.roko/learn/`.

### Episode logging

Every agent turn produces an `Episode` record in `.roko/memory/episodes.jsonl`:

```jsonl
{
  "id": "ep-20260429-001",
  "role": "implementer",
  "model": "claude-sonnet-4-6",
  "prompt_hash": "a1b2c3",
  "gate_verdict": "pass",
  "tokens": 4200,
  "cost_usd": 0.0126,
  "duration_ms": 8400,
  "hdc_fingerprint": "deadbeef..."
}
```

HDC (Hyperdimensional Computing) fingerprints identify semantically similar
episodes for replay and retrieval. The `FeedbackService` computes them from
the combined task + output content.

```bash
roko learn episodes --limit 20
```

### Playbook extraction

`PlaybookStore` accumulates techniques from successful runs and stores them
in `.roko/memory/playbook.toml`. Playbooks are queried at dispatch time and
injected into layer 6 of the system prompt.

```bash
roko learn all   # view current playbook state
```

### CascadeRouter bandit state

After each workflow completion, `FeedbackService` calls
`CascadeRouter::observe_outcome(role, model, success, cost, latency)`. The
router updates its LinUCB arm weights.

```bash
roko learn router   # inspect bandit state per role
```

State file: `.roko/learn/cascade-router.json`

### Prompt experiment tracking (A/B)

`ModelExperimentStore` assigns variants to runs and records outcomes for
statistically grounded comparisons between prompt strategies.

```bash
roko config experiments    # list active experiments
roko learn tune routing    # tune routing weights
```

State file: `.roko/learn/experiments.json`

### Knowledge admission (A-MAC)

Knowledge entries earn scores based on how often they correlate with
successful outcomes. Entries below a minimum score threshold are excluded
from future prompts.

State file: `.roko/learn/knowledge-scores.json`

### Prompt section effectiveness

After each run, `FeedbackService` correlates prompt section IDs with run
outcomes. Sections with consistently low scores are trimmed first when the
prompt approaches the token budget.

State file: `.roko/learn/section-effects.json`

### Efficiency events

Per-turn efficiency events are logged to `.roko/learn/efficiency.jsonl`:

```bash
roko learn efficiency   # view efficiency events
```

### Manual tuning commands

```bash
roko learn tune gates     # adjust adaptive gate thresholds
roko learn tune routing   # adjust routing reward weights
roko learn tune budget    # adjust budget parameters
```

---

## 11. Event System and Subscriptions

### Event bus

The global `EventBus<RuntimeEvent>` is the backbone for all async
communication between the orchestrator, HTTP server, TUI, and external
consumers.

Every `RuntimeEvent` carries:
- `run_id` — stable identifier for the workflow run
- `seq` — monotonically increasing sequence number
- `ts` — RFC 3339 timestamp
- `schema_version` — for forward compatibility
- `source` — emitting component
- `payload` — typed event data

### SSE streaming

Connect to the event stream:

```bash
curl -N http://127.0.0.1:6677/v1/events
```

Sample `RuntimeEventEnvelope`:

```json
{
  "run_id": "run-abc123",
  "seq": 42,
  "ts": "2026-04-29T03:00:00Z",
  "schema_version": 1,
  "source": "workflow_engine",
  "payload": {
    "kind": "gate_passed",
    "data": {
      "run_id": "run-abc123",
      "gate_name": "compile",
      "duration_ms": 1840
    }
  }
}
```

### Cron subscriptions

Fire on a schedule:

```toml
[[subscriptions]]
template = "weekly-summarizer"
trigger = "scheduler:cron:weekly-digest"
enabled = true

[subscriptions.trigger_config]
type = "cron"
schedule = "0 9 * * MON"

[[scheduler.cron]]
name = "weekly-digest"
expression = "0 9 * * MON"
signal_kind = "scheduler:cron:weekly-digest"
```

### File-watch subscriptions

Fire when watched files change:

```toml
[[subscriptions]]
template = "auto-format"
trigger = "fs.changed"
debounce_ms = 500
enabled = true

[subscriptions.trigger_config]
type = "file_watch"
paths = ["src/"]
extensions = ["rs"]
recursive = true
```

### Webhook subscriptions

Fire on incoming GitHub events:

```toml
[webhooks.github]
secret = "${GITHUB_WEBHOOK_SECRET}"

[[subscriptions]]
template = "pr-reviewer"
trigger = "github.pull_request.opened"
concurrency_limit = 2
cooldown_secs = 60
enabled = true

[subscriptions.filter]
repo = ["my-org/my-repo"]
branch = ["main", "release/*"]
author = ["!dependabot[bot]"]   # glob negation

[subscriptions.trigger_config]
type = "webhook"
event = "pull_request.opened"
```

The webhook endpoint is `POST /v1/webhooks/github`. Roko verifies the
`X-Hub-Signature-256` header using `webhooks.github.secret`.

### PRD auto-plan subscription

If `prd.auto_plan = true` and `roko serve` is running, a built-in subscriber
fires on every PRD publish event:

```toml
[prd]
auto_plan = true

[serve]
auto_orchestrate = true
```

---

## 12. HTTP Control Plane

Start the server:

```bash
roko serve
# Listening on http://127.0.0.1:6677
```

The server exposes approximately 85 REST routes plus SSE streaming.

### Key endpoint groups

| Prefix | What |
|---|---|
| `GET /v1/status` | System health, agent fleet status |
| `GET /v1/plans` | Plan list and status |
| `POST /v1/plans` | Create and run a plan |
| `GET /v1/agents` | Agent list |
| `POST /v1/agents/:id/message` | Send message to a running agent |
| `GET /v1/events` | SSE stream of `RuntimeEvent` |
| `GET /v1/learn/*` | Learning state inspection |
| `GET /v1/prd/*` | PRD management |
| `GET /v1/gates/*` | Gate pipeline status |
| `GET /v1/bench` | Benchmark endpoint |
| `GET /v1/dream` | Dream consolidation status |
| `POST /v1/webhooks/github` | GitHub webhook ingress |

### Authentication

```toml
[serve.auth]
enabled = true
api_key = "my-secret-key"   # single shared key
```

With a named key:

```toml
[[serve.auth.api_keys]]
name = "ci-pipeline"
key_hash = "abc123..."   # SHA-256 hex
scope = "agent:write"
created_at = "2026-04-29T00:00:00Z"
```

Clients pass the key in the `X-Api-Key` header:

```bash
curl -H "X-Api-Key: my-secret-key" http://127.0.0.1:6677/v1/status
```

### CORS configuration

```toml
[server]
cors_origins = ["http://localhost:3000", "https://app.example.com"]
```

Empty `cors_origins` is permissive (allows all origins).

### Per-agent sidecar

Each running agent exposes its own HTTP sidecar on a dynamically assigned
port:

```bash
roko agent serve --name my-agent --port 7001
```

The sidecar (`roko-agent-server`) exposes 13 routes including:

| Endpoint | What |
|---|---|
| `POST /message` | Send message, get real LLM response |
| `GET /stream` | WebSocket streaming |
| `GET /predictions` | Agent belief state |
| `POST /research` | Trigger agent research |
| `GET /tasks` | Agent task queue |

---

## 13. Error Recovery

### Resuming after interruption

The executor checkpoint is written atomically after every phase transition.
Resume any interrupted plan with:

```bash
roko plan run plans/my-plan/ \
  --resume .roko/state/executor.json
```

The checkpoint restores the exact pipeline state — phase, iteration count,
accumulated review findings — so no work is duplicated.

### Manual checkpoint inspection

```bash
# View checkpoint state
cat .roko/state/executor.json | jq

# View event log
cat .roko/state/events.json | jq
```

### Gate failure recovery flow

When gates fail the orchestrator tries three recovery strategies in order:

1. **Autofix** — spawn a fast model (`conductor.auto_fix_model`) with the
   gate failure output injected as context. Attempts up to
   `conductor.max_auto_fix_attempts` times.

2. **Replan** — when autofix budget is exhausted and
   `learning.replan_on_gate_failure = true`, a new planning agent pass runs
   with the gate failure context. Happens at most
   `learning.replan_max_per_plan` times per plan.

3. **Halt** — if replan budget is also exhausted, the task is marked failed
   and the plan is halted. Inspect with `roko plan show <plan-id>`.

```bash
roko plan show my-plan    # view task status and failure details
roko plan regenerate      # regenerate the plan and retry
```

### Rollback

To undo changes from a failed task:

```bash
git diff --stat   # see what changed
git checkout -- . # discard uncommitted changes
```

Or roll back a committed task:

```bash
git log --oneline -10
git revert <commit-hash>
```

Roko does not have a built-in rollback command — use standard git operations.

### Recovery from corrupt state

If the executor checkpoint is corrupt:

```bash
# Remove the corrupt checkpoint
rm .roko/state/executor.json

# Restart the plan from the beginning
roko plan run plans/my-plan/
```

To skip already-completed tasks:

```bash
# The executor tracks completed task IDs; if the checkpoint is gone,
# mark tasks complete manually with roko plan show and roko plan edit.
roko plan show my-plan
```

---

## 14. Deployment Configuration

### Railway deployment

```bash
roko deploy railway
```

This reads `[serve.deploy]` from `roko.toml`, creates or updates a Railway
service using the worker image, sets the required environment variables, and
configures the start command as `roko serve`.

```toml
[deploy]
backend = "railway-api"
railway_api_token = "${RAILWAY_API_TOKEN}"
project_id = "abc-123"
environment_id = "def-456"
worker_image = "ghcr.io/nunchi-trade/roko-worker:latest"

[serve.deploy]
provider = "railway"
environment = [
  "ANTHROPIC_API_KEY",
  "OPENAI_API_KEY",
  "PERPLEXITY_API_KEY",
  "GITHUB_TOKEN",
  "GITHUB_WEBHOOK_SECRET",
]
```

Enable relay registration so the demo app auto-discovers your deployed
instance:

```toml
[relay]
url = "wss://relay.nunchi.dev"
workspace_name = "my-workspace"
# public_url is auto-detected from RAILWAY_PUBLIC_DOMAIN
```

### Fly.io deployment

```bash
roko deploy fly
```

```toml
[deploy]
backend = "manual"
worker_image = "ghcr.io/nunchi-trade/roko-worker:latest"
default_region = "sjc"   # Fly region
```

### Docker deployment

```bash
roko deploy docker
```

Builds and tags a Docker image from the Dockerfile in the workspace root.
The image uses a multi-stage build, copies only the `roko-cli` binary, exposes
port 6677, and defaults to `roko serve` as the entrypoint.

```bash
docker run -p 6677:6677 \
  -e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" \
  -e PERPLEXITY_API_KEY="$PERPLEXITY_API_KEY" \
  -v "$(pwd):/workspace" \
  roko-worker:latest serve
```

### System daemon (launchd / systemd)

Install roko as a system service that starts on boot:

```bash
roko daemon install   # installs launchd (macOS) or systemd (Linux) service
roko daemon start
roko daemon stop
roko daemon status
roko daemon logs
```

The daemon runs `roko serve` on startup and restarts it on failure.

### Worker mode (multi-node)

For multi-node deployments, `roko worker` runs as a stateless task worker:

```bash
# Control node
roko serve    # exposes :6677

# Worker nodes
roko worker --control http://control-node:6677
```

Worker node config:

```toml
[deploy]
backend = "manual"
worker_image = "ghcr.io/nunchi-trade/roko-worker:latest"
```

---

## 15. Environment Variables Reference

### Direct config overrides

These environment variables override values in `roko.toml` at startup:

| Variable | Config equivalent | Notes |
|---|---|---|
| `ROKO_MODEL` | `agent.default_model` | Default model slug |
| `ROKO_BACKEND` | `agent.default_backend` | Default backend |
| `ROKO_EFFORT` | `agent.default_effort` | Reasoning effort |
| `ROKO_CONTEXT_LIMIT_K` | `agent.context_limit_k` | Context window (k tokens) |
| `ROKO_MAX_AGENTS` | `conductor.max_agents` | Max concurrent agents |
| `ROKO_BUDGET_USD` | `budget.max_plan_usd` | Plan budget in USD |
| `ROKO_PARALLEL` | `conductor.parallel_enabled` | Enable parallel execution |
| `ROKO_EXPRESS` | `conductor.express_mode` | Enable express mode |
| `ROKO_SKIP_TESTS` | `gates.skip_tests` | Skip test gate |
| `ROKO_CLIPPY` | `gates.clippy_enabled` | Enable clippy gate |
| `ROKO_PROVIDER` | `models.<default>.provider` | Override provider for default model |
| `ROKO_MODEL_SLUG` | `models.<default>.slug` | Override API slug for default model |

### API key environment variables

Referenced by name in `[providers.*].api_key_env`:

| Variable | Provider |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic API and Claude CLI |
| `OPENAI_API_KEY` | OpenAI |
| `PERPLEXITY_API_KEY` | Perplexity Sonar |
| `GEMINI_API_KEY` | Google Gemini |
| `CEREBRAS_API_KEY` | Cerebras |
| `OPENROUTER_API_KEY` | OpenRouter |

If `ANTHROPIC_API_KEY` is present and no `[providers]` section is configured,
roko auto-synthesizes an `anthropic_api` provider.

If `ANTHROPIC_API_KEY` is not present but the `claude` CLI is available,
roko auto-synthesizes a `claude_cli` provider.

### Deployment detection

These are read automatically when deploying to cloud platforms:

| Variable | Used for |
|---|---|
| `RAILWAY_PUBLIC_DOMAIN` | Auto-detect `relay.public_url` on Railway |
| `FLY_APP_NAME` | Auto-detect `relay.public_url` on Fly.io |
| `PORT` | Override `server.port` on cloud platforms |

### String interpolation in config

In `roko.toml`, use `${VAR}` to reference any environment variable:

```toml
[providers.my-provider]
base_url = "https://${MY_DOMAIN}/v1"
api_key_env = "MY_API_KEY"
```

### Secret files

In `[providers.*.extra_headers]`, keys ending in `_file` have their values
treated as file paths. The file content is read and used as the header value:

```toml
[providers.my-provider.extra_headers]
Authorization_file = "/run/secrets/api-token"
# Becomes: Authorization: <file contents>
```

---

## 16. WorkflowEngine Integration (Programmatic)

This section shows how to wire up a `WorkflowEngine` programmatically, which
is what the CLI, HTTP server, and ACP adapter all do.

### Instantiate the services

```rust
use std::sync::Arc;
use std::path::PathBuf;

use roko_agent::model_call_service::ModelCallService;
use roko_compose::prompt_assembly_service::PromptAssemblyService;
use roko_learn::feedback_service::FeedbackService;
use roko_gate::gate_service::GateService;
use roko_runtime::effect_driver::EffectServices;
use roko_runtime::workflow_engine::{WorkflowEngine, WorkflowRunConfig};
use roko_runtime::pipeline_state::WorkflowConfig;

// 1. ModelCallService — wraps provider dispatch, routing, cost tracking
let model_caller = Arc::new(
    ModelCallService::new("claude-sonnet-4-6".to_string())
        .with_config(roko_config.clone())           // provider routing table
        .with_feedback_sink(Arc::clone(&feedback))  // record model calls
        .with_model_router(router_fn)               // CascadeRouter closure
);

// 2. PromptAssemblyService — 9-layer system prompt builder
let prompt_assembler = Arc::new(
    PromptAssemblyService::new()
        .with_domain_context("Rust systems programming".to_string())
        .with_episodes_path(PathBuf::from(".roko/memory/episodes.jsonl"))
        .with_playbook_store(Arc::clone(&playbook_store))
        .with_knowledge_store(Arc::clone(&neuro_store))
        .with_token_budget(8192)
);

// 3. FeedbackService — records events to .roko/learn/
let feedback = Arc::new(
    FeedbackService::new(PathBuf::from(".roko/learn"))
        .with_episode_logger(episode_logger)
        .with_cascade_router(Arc::clone(&cascade_router))
);

// 4. GateService — runs the gate pipeline
let gate_runner = Arc::new(
    GateService::new()
        .with_adaptive_thresholds(adaptive_thresholds)
);
```

### Build EffectServices and WorkflowEngine

```rust
let services = EffectServices {
    default_model: "claude-sonnet-4-6".to_string(),
    model_caller: Arc::clone(&model_caller),
    prompt_assembler: Arc::clone(&prompt_assembler),
    feedback_sink: Arc::clone(&feedback),
    gate_runner: Arc::clone(&gate_runner),
    affect_policy: None,  // or Some(Arc::new(Mutex::new(daimon_policy)))
};

let mut engine = WorkflowEngine::new(services);
```

### Attach event consumers

```rust
use roko_runtime::jsonl_logger::JsonlLogger;

let logger = Arc::new(JsonlLogger::from_roko_dir(&PathBuf::from(".roko")));
engine.add_consumer(logger);
```

### Run a workflow

```rust
use roko_core::foundation::ShellGateCommand;

let config = WorkflowRunConfig {
    prompt: "Add error handling to the database connection module".to_string(),
    workdir: PathBuf::from("/path/to/workspace"),
    workflow: WorkflowConfig::standard(),   // or express() / full()
    enabled_gates: vec![
        "compile".to_string(),
        "clippy".to_string(),
        "test".to_string(),
    ],
    shell_gates: vec![
        ShellGateCommand {
            program: "cargo".to_string(),
            args: vec!["audit".to_string()],
            timeout_ms: 60_000,
        },
    ],
    commit_prefix: Some("fix:".to_string()),
};

let report = engine.run(config).await?;
println!("run_id: {}", report.run_id);
println!("success: {}", report.success);
println!("model: {}", report.model);
println!("tokens: {}", report.token_usage);
println!("cost: ${:.4}", report.cost.unwrap_or(0.0));
println!("duration: {:.1}s", report.duration_secs);
// report.gates  — Vec<GateOutcome>
// report.events — Vec<RuntimeEventEnvelope>
```

### Cooperative cancellation

```rust
use roko_runtime::cancel::CancelToken;

let token = CancelToken::new();
let token_clone = token.clone();

tokio::spawn(async move {
    tokio::time::sleep(Duration::from_secs(30)).await;
    token_clone.cancel();
});

let report = engine.run_with_cancel(config, token).await?;
// report.outcome will be WorkflowOutcome::Cancelled
```

### Checkpoint and resume

```rust
// Checkpointing is automatic inside EffectDriver::save_checkpoint.
// Manual checkpoint:
let json = pipeline_state.checkpoint()?;
std::fs::write(".roko/state/executor.json", &json)?;

// Restore:
let restored = PipelineStateV2::from_checkpoint(&json)?;
let output = restored.step(PipelineInput::Start);
```

### 9-layer system prompt

`PromptAssemblyService` builds system prompts in nine ordered layers:

| Layer | Content | Source |
|---|---|---|
| 0 | Role identity | `role_prompts::role_identity_for(role)` |
| 1 | Task specification | `PromptSpec.task` |
| 2 | Project conventions | Detected from `workdir` |
| 3 | Domain context | Static text, knowledge store entries |
| 4 | Episode history | Recent successful episodes |
| 5 | Tool instructions | Tool usage guidance |
| 6 | Playbook techniques | Relevant playbook entries |
| 7 | Gate feedback | Prior gate failure output |
| 8 | Anti-patterns | Extracted failure anti-patterns |

Each layer has an effectiveness score. Sections with consistently low scores
are trimmed first when the prompt approaches the token budget.

---

## 17. Key File Locations

| What | Absolute path |
|---|---|
| Workspace config | `/path/to/project/roko.toml` |
| Data directory | `/path/to/project/.roko/` |
| Layout version | `/path/to/project/.roko/VERSION` |
| Executor checkpoint | `/path/to/project/.roko/state/executor.json` |
| Episode log | `/path/to/project/.roko/memory/episodes.jsonl` |
| Playbook | `/path/to/project/.roko/memory/playbook.toml` |
| Cascade router state | `/path/to/project/.roko/learn/cascade-router.json` |
| Gate thresholds | `/path/to/project/.roko/learn/gate-thresholds.json` |
| Prompt experiments | `/path/to/project/.roko/learn/experiments.json` |
| Knowledge scores | `/path/to/project/.roko/learn/knowledge-scores.json` |
| Section effectiveness | `/path/to/project/.roko/learn/section-effects.json` |
| Efficiency events | `/path/to/project/.roko/learn/efficiency.jsonl` |
| Custody audit chain | `/path/to/project/.roko/custody.jsonl` |
| Witness DAG log | `/path/to/project/.roko/witness.jsonl` |
| Gap tracker | `/path/to/project/.roko/GAPS.md` |
| Config schema | `crates/roko-core/src/config/schema.rs` |
| Agent config | `crates/roko-core/src/config/agent.rs` |
| Provider config | `crates/roko-core/src/config/provider.rs` |
| Learning config | `crates/roko-core/src/config/learning.rs` |
| Routing config | `crates/roko-core/src/config/routing.rs` |
| Gates config | `crates/roko-core/src/config/gates.rs` |
| Serve config | `crates/roko-core/src/config/serve.rs` |
| Budget config | `crates/roko-core/src/config/budget.rs` |
| Subscriptions config | `crates/roko-core/src/config/subscriptions.rs` |
| Directory layout | `crates/roko-fs/src/layout.rs` |
| Foundation traits | `crates/roko-core/src/foundation.rs` |
| RuntimeEvent types | `crates/roko-core/src/runtime_event.rs` |
| Orchestrator | `crates/roko-cli/src/orchestrate.rs` |
| HTTP routes | `crates/roko-serve/src/routes/` |
| TUI | `crates/roko-cli/src/tui/` |
| CascadeRouter | `crates/roko-learn/src/cascade_router.rs` |

---

## 18. Known Gaps

The following items are built but not fully wired at runtime. Track current
gaps in `.roko/GAPS.md`.

| Gap | Status |
|---|---|
| Knowledge-informed model routing | Neuro store not yet consulted in `CascadeRouter` — it currently uses only bandit observations, not knowledge entry scores |
| Cold substrate archival | Built in `roko-dreams` but no cron trigger at runtime — `roko knowledge archive` must be run manually |
| `force_backend` override learning | `CascadeRouter` does not learn from manual overrides set via `routing_overrides.force_backend` |
| Chain runtime integration | Phase 2+ — requires blockchain backend for witness anchoring |
| VCG auction in composition | `vcg_allocate` built and exported but greedy path dominates at runtime |
| Safety contracts enforcement | `AgentContract` falls back to permissive defaults when YAML manifest is missing |
| LLM judge gate | Rung 6 is a stub — not wired to a live model |
| Dream cron trigger | `roko-dreams` offline consolidation has no automatic cron/runtime trigger |
