# Roko Integration Guide

Roko is a self-developing agent toolkit: you describe work in plain English, and Roko generates
an implementation plan, dispatches Claude agents to execute it, validates the result with a gate
pipeline, and learns from each run to do better next time. The entire system is configured from a
single `roko.toml` file in your workspace root.

This guide is written for someone reading it for the first time. It starts with the minimum
you need to get running and progressively introduces each subsystem, with collapsible reference
sections for every configuration option.

---

## Table of Contents

1. [Quick Start — Minimal roko.toml](#1-quick-start--minimal-rokotoml)
2. [Core Concepts](#2-core-concepts)
3. [Self-Hosting Workflow (End-to-End)](#3-self-hosting-workflow-end-to-end)
4. [Essential Config — What You Must Configure](#4-essential-config--what-you-must-configure)
   - [project](#41-project)
   - [agent](#42-agent)
   - [providers](#43-providers)
   - [models](#44-models)
5. [Operational Config — What You Should Configure](#5-operational-config--what-you-should-configure)
   - [gates](#51-gates)
   - [routing](#52-routing)
   - [learning](#53-learning)
   - [conductor](#54-conductor)
   - [budget](#55-budget)
   - [pipeline](#56-pipeline)
6. [Advanced Config — Power User Sections](#6-advanced-config--power-user-sections)
7. [Provider Configuration Recipes](#7-provider-configuration-recipes)
8. [Model Registry](#8-model-registry)
9. [MCP Server Configuration](#9-mcp-server-configuration)
10. [Agent Manifests and Safety Contracts](#10-agent-manifests-and-safety-contracts)
11. [Gate Pipeline Configuration](#11-gate-pipeline-configuration)
12. [Learning Configuration](#12-learning-configuration)
13. [Event System and Subscriptions](#13-event-system-and-subscriptions)
14. [HTTP Control Plane](#14-http-control-plane)
15. [Error Recovery](#15-error-recovery)
16. [Deployment Guide](#16-deployment-guide)
17. [Environment Variables Reference](#17-environment-variables-reference)
18. [WorkflowEngine Integration (Programmatic)](#18-workflowengine-integration-programmatic)
19. [Key File Locations](#19-key-file-locations)
20. [Known Gaps](#20-known-gaps)

---

## 1. Quick Start — Minimal roko.toml

The absolute minimum configuration to run Roko is a project name and an API key. If you have
`ANTHROPIC_API_KEY` set in your environment, Roko auto-detects the provider and you only need:

```toml
# roko.toml — minimal config

[project]
name = "my-project"

[agent]
default_model = "claude-sonnet-4-6"
```

Then initialize the workspace and run the self-hosting loop:

```bash
# Rust 1.91+ required (alloy deps)
rustup update stable

# Build the CLI
cargo build -p roko-cli --release

# Initialize .roko/ directory and write a starter roko.toml
roko init

# Capture work → draft a plan → execute it
roko prd idea "Add input validation to the API handler"
roko prd draft new "api-validation"
roko prd plan api-validation
roko plan run plans/api-validation/

# Watch progress in real time
roko dashboard
```

That is the complete self-hosting loop. The sections below explain every knob available.

---

## 2. Core Concepts

### What roko.toml controls

`roko.toml` is the single source of truth for how the entire system behaves:

- **Which AI providers to use** and how to reach them (`[providers.*]`, `[models.*]`)
- **Which model gets used for which kind of task** (`[routing]`, `[agent.roles.*]`)
- **What verification gates run after each task** (`[gates]`, `[pipeline]`)
- **How much money can be spent** (`[budget]`, `[energy]`)
- **How the system learns from experience** (`[learning]`, `[demurrage]`)
- **What HTTP API to expose** (`[serve]`, `[server]`)
- **What triggers fire automatically** (`[subscriptions]`, `[scheduler]`, `[webhooks]`)
- **How to deploy to production** (`[deploy]`, `[relay]`)

The schema lives in `crates/roko-core/src/config/schema.rs`. Every field has a serde default, so
you only need to write sections that differ from the defaults.

### Config layering

```
Environment variables         ROKO_MODEL=claude-opus-4-6 overrides everything
        |
        v
   roko.toml                  Your workspace config file
        |
        v
  RokoConfig::default()       Compiled-in defaults (always present)
```

Two secret-resolution passes run after parsing:

1. `${VAR}` interpolation — expands `${ENV_VAR}` references anywhere in the config.
2. `*_file` resolution — keys ending in `_file` in `extra_headers` read their value from
   the file at the specified path. Useful for Docker secrets.

### What .roko/ stores

All runtime state lives in `.roko/`. You never need to edit these files by hand; they exist
so the system can resume after a crash, accumulate learning across runs, and give you visibility
into what happened.

```
.roko/
  roko.toml is NOT here — it lives in your workspace root

  memory/               What agents learned
    episodes.jsonl      Per-turn records: model, cost, gate verdict, HDC fingerprint
    playbook.toml       Accumulated successful techniques, injected into future prompts
    skills/             Learned skill files

  learn/                How the system improves
    cascade-router.json Model routing bandit state (which model wins for which task type)
    experiments.json    A/B prompt experiment store
    gate-thresholds.json Adaptive gate skip thresholds (EMA per rung)
    efficiency.jsonl    Per-turn efficiency events
    knowledge-scores.json Knowledge admission scores
    section-effects.json Prompt section effectiveness scores

  state/                Crash recovery
    executor.json       Pipeline checkpoint — resume any interrupted plan run
    events.json         Event log snapshot

  prd/                  Product Requirements Documents
    ideas/              Raw one-line ideas
    drafts/             PRDs being written
    published/          Promoted PRDs (trigger plan generation)
    plans/              Generated implementation plans

  learn/                (see above)
  research/             Perplexity research artifacts
  cache/                Cargo target dir, context pack cache
  custody.jsonl         Append-only audit chain
  witness.jsonl         Witness DAG log
  VERSION               Layout format version (currently 1)
```

<details>
<summary>Full .roko/ path accessor reference</summary>

| Path | Method on RokoLayout | Purpose |
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

Source: `crates/roko-fs/src/layout.rs` (`RokoLayout` struct).

</details>

### Hot-reload vs. restart

When `roko serve` is running, most config changes take effect immediately:

| Section | Behavior |
|---|---|
| `budget`, `gates`, `routing`, `learning` | Hot-reload |
| `demurrage`, `scheduler`, `watcher` | Hot-reload |
| `subscriptions`, `conductor`, `attention`, `goals` | Hot-reload |
| `agent`, `project`, `serve`, `providers`, `models`, `server` | Requires restart |

### Architecture in one diagram

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

`PipelineStateV2` is a pure Rust struct — no async, no I/O. It serializes to JSON for
checkpoint/resume. `EffectDriver` owns the services and translates state-machine outputs into
real work.

<details>
<summary>Foundation trait table</summary>

| Trait | Implementor | Purpose |
|---|---|---|
| `ModelCaller` | `ModelCallService` (roko-agent) | LLM dispatch with routing |
| `PromptAssembler` | `PromptAssemblyService` (roko-compose) | 9-layer system prompt |
| `FeedbackSink` | `FeedbackService` (roko-learn) | Episode and feedback recording |
| `GateRunner` | `GateService` (roko-gate) | 7-rung gate pipeline |
| `EventConsumer` | `JsonlLogger`, `SseAdapter`, ACP | Runtime event observation |
| `AffectPolicy` | `DaimonPolicy` (roko-daimon) | Behavioral dispatch modulation |
| `EffectExecutor` | `EffectDriver` (roko-runtime) | Translates state-machine actions to I/O |

Source: `crates/roko-core/src/foundation.rs`.

</details>

---

## 3. Self-Hosting Workflow (End-to-End)

This is how Roko develops itself: a ten-step loop from capturing an idea to a validated,
committed implementation. Every command listed here exists in the CLI today.

```
idea → PRD → research → plan → execute → gate → learn → repeat
  │                               │
  └── roko prd idea/draft/plan    └── roko plan run
```

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

Creates `.roko/` with the standard directory layout and a starter `roko.toml` in the workspace
root. Edit `roko.toml` to configure your providers (see Section 4) before proceeding.

### Step 2: Capture a work item

```bash
roko prd idea "Wire knowledge store into CascadeRouter for model selection"
```

Creates a dated idea file in `.roko/prd/ideas/`. Ideas are lightweight — just a title and
timestamp. Think of this as a post-it note you don't want to lose.

```bash
roko prd list   # view all ideas and PRDs
```

### Step 3: Draft a PRD

```bash
roko prd draft new "knowledge-informed-routing"
```

Launches a Claude agent that reads the idea, reads the current codebase via the code-intelligence
MCP, and writes a structured PRD into `.roko/prd/drafts/knowledge-informed-routing.md`.

The PRD includes a mandatory **Repository Grounding** section listing the specific files and
types the implementation will touch. This grounding is injected as context when generating the
plan in step 5, which dramatically reduces hallucinated references to nonexistent functions.

### Step 4: Enrich with research (optional)

```bash
roko research enhance-prd knowledge-informed-routing
```

Launches a Perplexity research agent that queries for relevant prior art, papers, and API
documentation, then appends a **Research** section to the draft PRD. Optional but significantly
improves plan quality for novel subsystems.

### Step 5: Review and promote the PRD

```bash
# Inspect the draft
roko prd draft list

# Edit if needed
roko prd draft edit knowledge-informed-routing

# Promote to published
roko prd draft promote knowledge-informed-routing
```

Promoting moves the file to `.roko/prd/published/`. If `prd.auto_plan = true` in `roko.toml`,
plan generation (step 6) triggers automatically via the `prd_publish_subscriber` background task
in `roko serve`.

### Step 6: Generate an implementation plan

```bash
roko prd plan knowledge-informed-routing
```

A Claude agent reads the published PRD (including the Repository Grounding section), reads the
relevant source files, and produces a `plans/knowledge-informed-routing/tasks.toml` with a DAG
of implementation tasks.

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

This starts the main orchestration loop (`crates/roko-cli/src/orchestrate.rs`). For each task:

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

The `PipelineStateV2` checkpoint is written atomically after each phase transition. Resumption
restores the exact pipeline state so work is never duplicated.

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

The TUI uses a file watcher (`notify::RecommendedWatcher`) to pick up changes to `.roko/`
without polling. Updates appear in under 250 ms.

### Step 10: Inspect learning state

```bash
roko learn all          # full dump
roko learn router       # cascade router bandit state
roko learn experiments  # A/B prompt experiments
roko learn efficiency   # per-turn efficiency events
roko learn episodes     # episode history
```

After a few successful runs the cascade router accumulates enough observations to graduate from
the static routing table (stage 1, < 50 observations) into the confidence-based stage (stage 2,
50–200 observations) and eventually the LinUCB bandit stage (stage 3, > 200 observations). At
that point, Roko picks the right model for each task type automatically.

---

## 4. Essential Config — What You Must Configure

These are the sections you need to set correctly before anything works. Get these right first.

### 4.1 [project]

**Why this matters**: The project name appears in logs, episode records, and PRD metadata. The
`root` and `fresh_base_branch` fields tell the orchestrator where your code lives and which git
branch to use as a baseline when creating fresh worktrees for isolated task execution.

```toml
[project]
name = "my-project"          # string, default: "roko-project"
root = "."                   # relative or absolute project root, default: "."
fresh_base_branch = "main"   # git branch for fresh worktree creation, default: "main"
# default_domain = "coding"  # default task domain (optional)
```

<details>
<summary>Field details</summary>

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | string | `"roko-project"` | Project identifier used in logs and metadata |
| `root` | string | `"."` | Relative or absolute project root |
| `fresh_base_branch` | string | `"main"` | Git branch for fresh worktree creation |
| `default_domain` | string | none | Default task domain: `"coding"`, `"research"`, `"chain"`, `"docs"`, `"ops"`. Tasks without an explicit domain inherit this. |

</details>

<details>
<summary>[prd] — PRD lifecycle settings</summary>

**Why this matters**: Setting `auto_plan = true` with `roko serve` running means you never have
to manually run `roko prd plan` — promoting a PRD automatically queues the plan generation.

```toml
[prd]
auto_plan = false   # bool: auto-generate plan when a PRD is promoted, default: false
```

When `auto_plan = true`, `roko serve` listens for PRD publish events and triggers
`roko prd plan <slug>` automatically via the `prd_publish_subscriber` background task.

</details>

### 4.2 [agent]

**Why this matters**: This section defines the default model and backend used for all agents.
If you skip this entirely and have `ANTHROPIC_API_KEY` set, Roko auto-detects an
`anthropic_api` provider and uses `claude-sonnet-4-6` as the default. You only need to
customize this when you want to change the defaults or add per-role overrides (e.g., use
Opus for complex architectural tasks).

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
```

**`default_backend`** auto-detection: if `ANTHROPIC_API_KEY` is set, defaults to
`"anthropic_api"`; otherwise defaults to `"claude"` (the Claude CLI subprocess).

**`mode`** values:
- `"ephemeral"` — agent runs a task then exits (default, used by `plan run`)
- `"persistent"` — agent runs continuously until explicitly stopped (`roko agent start`)
- `"reactive"` — agent sleeps until a trigger fires (webhook, cron, event)

**`temperament`** values: `"conservative"`, `"balanced"`, `"exploratory"`. Higher exploration
rates cause the AffectPolicy to set `Bypass` cache policy on model calls to avoid stale
responses.

<details>
<summary>Per-role overrides — use different models for different job types</summary>

Per-role overrides let you pin the implementer role to Opus (for correctness) while using
Haiku for the autofix role (for speed and cost). Any missing field inherits from `[agent]`.

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

</details>

<details>
<summary>CaMeL dual-LLM isolation (SAFE-07)</summary>

The `data_llm` section enables CaMeL-style dual-LLM isolation: untrusted content (e.g., web
search results, user-provided data) is processed by a smaller isolated model before being
passed to the main agent. This prevents prompt injection from untrusted sources.

```toml
[agent.data_llm]
model = "claude-haiku-3-5"   # smaller model for untrusted content isolation
max_tokens = 4096
temperature = 0.0
strip_tool_calls = true      # Data LLM cannot produce tool calls
sanitize_input = true        # strip known injection patterns before sending
# output_schema = { ... }    # optional JSON Schema for Data LLM output validation
```

</details>

<details>
<summary>Policy manifests</summary>

```toml
[agent]
policy_manifests = [".roko/roles/manifest.toml"]
```

Policy manifests contain per-role YAML safety contracts and tool allowlists. Loaded before
each agent dispatch. See [Section 10](#10-agent-manifests-and-safety-contracts) for details.

</details>

### 4.3 [providers.*]

**Why this matters**: Providers are how Roko talks to LLMs. Without at least one provider
configured (or `ANTHROPIC_API_KEY` set for auto-detection), no agents can run. Each key under
`[providers]` is a name you invent; `[models.*]` entries then reference providers by that name.

You can have multiple providers configured simultaneously — useful for failover or mixing
providers (Anthropic for coding, Perplexity for research).

```toml
[providers.my-provider]
kind = "anthropic_api"     # ProviderKind value (see table below)
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

<details>
<summary>Supported provider kinds</summary>

| `ProviderKind` | TOML value | What it talks to |
|---|---|---|
| `AnthropicApi` | `"anthropic_api"` | Anthropic Messages API over HTTP |
| `ClaudeCli` | `"claude_cli"` | `claude` CLI subprocess (stream-JSON) |
| `OpenAiCompat` | `"openai_compat"` | Any OpenAI-compatible HTTP endpoint |
| `CursorAcp` | `"cursor_acp"` | Cursor Agent Client Protocol |
| `PerplexityApi` | `"perplexity_api"` | Perplexity Sonar HTTP API |
| `GeminiApi` | `"gemini_api"` | Google Gemini API (native) |
| `CerebrasApi` | `"cerebras_api"` | Cerebras inference API |

</details>

See [Section 7: Provider Configuration Recipes](#7-provider-configuration-recipes) for
ready-to-copy configs for each provider.

### 4.4 [models.*]

**Why this matters**: The model registry decouples logical names from provider-specific API
slugs. The `[routing]` section and role overrides refer to models by their registry key (e.g.,
`"standard"`), not by the raw API slug. This means you can swap from Anthropic to OpenAI for
the standard model by changing one line in `[models]` without touching `[routing]`.

```toml
[models.standard]
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

See [Section 8: Model Registry](#8-model-registry) for a minimum three-model setup and a
full-featured example.

---

## 5. Operational Config — What You Should Configure

Once you have providers and models wired up, these sections control how well the system performs.
You can leave them at defaults initially, but tuning them makes a large difference in quality and
cost.

### 5.1 [gates]

**Why this matters**: Gates are the verification pipeline that runs after every agent task. By
default, Roko runs `cargo build` (rung 0), `cargo clippy` (rung 1), `cargo test` (rung 2), and
a `git diff` sanity check (rung 3). If any gate fails, the orchestrator retries up to
`max_iterations` times before giving up. Gates are what prevent agents from shipping broken code.

For non-Rust projects, you can replace the default gates with custom shell commands via
`[gates.domain_gates]`.

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

See [Section 11: Gate Pipeline Configuration](#11-gate-pipeline-configuration) for the full
7-rung table and adaptive threshold details.

### 5.2 [routing]

**Why this matters**: The routing section controls how Roko picks which AI model to use for
each task. By default it uses a static mapping (fast tasks → Haiku, standard → Sonnet, complex
→ Opus), but with `mode = "auto_override"` and enough observations, it graduates to a LinUCB
contextual bandit that learns which models actually pass gates reliably for each task type.

The three model keys (`fast_task_model`, `standard_task_model`, `complex_task_model`) reference
entries in `[models.*]`.

```toml
[routing]
mode = "auto_override"            # routing mode, default: "auto_override"
algorithm = "linucb"              # "linucb" or "thompson", default: "linucb"
discount_factor = 0.99            # Thompson discount factor for non-stationarity
fast_task_model = "claude-haiku-4-5"      # model for mechanical/fast tasks
standard_task_model = "claude-sonnet-4-6" # model for standard tasks
complex_task_model = "claude-opus-4-6"    # model for architectural tasks
context_strategy = "mcp_first"    # "mcp_first", "hybrid", "inline_heavy"
```

<details>
<summary>Routing stages and reward weights</summary>

The CascadeRouter progresses through three stages automatically:

| Stage | Observations | Strategy |
|---|---|---|
| 1: Static | < 50 | Hardcoded role → model table from TOML |
| 2: Confidence | 50–200 | Empirical pass-rate + confidence interval |
| 3: UCB/LinUCB | > 200 | Full contextual bandit |

State persists to `.roko/learn/cascade-router.json`.

Reward scalarization weights (must sum to ~1.0):

```toml
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

</details>

### 5.3 [learning]

**Why this matters**: The learning section controls how Roko improves between runs. The most
impactful setting is `replan_on_gate_failure = true`, which makes the system automatically
revise the implementation plan when an agent keeps failing the same gate — instead of just
retrying the same approach.

`auto_playbook_refresh = true` means successful techniques are captured and injected into future
agent prompts automatically, compounding over time.

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

See [Section 12: Learning Configuration](#12-learning-configuration) for details on episodes,
playbooks, and the cascade router.

### 5.4 [conductor]

**Why this matters**: The conductor is the meta-orchestrator that controls parallelism, autofix
behavior, and which reviewer roles run. The most important setting here is `max_auto_fix_attempts`:
when a gate fails, Roko spawns a fast model to try to fix the problem automatically. Setting this
too low means you give up too soon; too high wastes money on hopeless autofix loops.

`parallel_enabled = true` lets Roko execute independent tasks concurrently — much faster for
plans with many parallel-safe tasks.

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
```

<details>
<summary>Reviewer role toggles</summary>

```toml
# Toggle which reviewer roles are active:
[conductor.enabled_roles]
architect = true
auditor = true
scribe = true
critic = true
```

</details>

<details>
<summary>Per-watcher threshold overrides for anomaly detection</summary>

The conductor runs 10 watchers that monitor for anomalous agent behavior. Each watcher has
configurable thresholds. The defaults are reasonable starting points.

```toml
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

</details>

### 5.5 [budget]

**Why this matters**: Budget settings are hard guardrails on spending. When a budget is
exceeded, Roko raises `GatewayError::BudgetExceeded` and halts cleanly. Without these set,
a runaway agent or a plan with an unexpectedly large number of tasks could spend much more
than intended.

```toml
[budget]
max_plan_usd = 25.0         # max dollars per plan run, default: 25.0
max_turn_usd = 3.0          # max dollars per agent turn, default: 3.0
prompt_token_budget = 10000 # token budget for prompt composition, default: 10000
```

### 5.6 [pipeline]

**Why this matters**: The pipeline section maps task complexity to agent orchestration
behavior. Mechanical tasks (rename a variable) get a single implementation agent with no
reviewers. Architectural tasks (redesign a subsystem) get a strategist agent before
implementation, then a full review suite (architect + auditor + scribe) after. This controls
how much process overhead Roko applies to each task.

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

---

## 6. Advanced Config — Power User Sections

These sections are optional and only relevant once you're past the basics. Collapse each one
to explore.

<details>
<summary>[demurrage] — Knowledge decay</summary>

**Why this matters**: Without demurrage, the playbook accumulates entries indefinitely. Old
heuristics from months ago might be actively wrong about refactored code. Demurrage applies
Gesellian exponential decay so stale knowledge naturally fades and fresh knowledge dominates.

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

</details>

<details>
<summary>[attention] — Prompt token budget allocation</summary>

**Why this matters**: The 9-layer system prompt can exceed context window limits for large
projects. The attention section controls how tokens are allocated across layers and what
utilization target to aim for. The auction (`auction_enabled = true`) lets layers compete
for tokens based on their effectiveness scores.

```toml
[attention]
max_tokens_per_layer = 4096    # max tokens per prompt layer, default: 4096
utilization_target = 0.85      # context window utilization target (0.0-1.0), default: 0.85
auction_enabled = false        # enable attention auction between layers, default: false
task_reserve_tokens = 512      # tokens reserved for task context, default: 512
```

</details>

<details>
<summary>[immune] — Anomaly detection and quarantine</summary>

```toml
[immune]
quarantine_threshold = 0.8   # anomaly score above which outputs are quarantined
max_quarantined = 50         # escalate if this many items are quarantined
auto_reject = false          # auto-reject quarantined outputs (vs. hold for review)
taint_levels = ["low", "medium", "high"]   # taint classification levels
```

</details>

<details>
<summary>[temporal] — Time horizon preferences</summary>

```toml
[temporal]
max_depth = 5                    # max planning lookahead depth, default: 5
epoch_secs = 3600                # epoch duration for temporal batching, default: 3600
enforce_allen_relations = true   # enforce Allen temporal relations between tasks
```

</details>

<details>
<summary>[goals] — Goal hierarchy</summary>

```toml
[goals]
max_active = 10              # max active goals at any level, default: 10
correctness_weight = 0.7     # correctness vs. speed weight (0.0-1.0), default: 0.7
completion_threshold = 0.95  # min completion ratio for "done", default: 0.95
prune_threshold = 0.1        # prune goals with priority below this, default: 0.1
```

</details>

<details>
<summary>[energy] — Compute budget pool</summary>

**Why this matters**: The energy pool is a higher-level budget abstraction than `[budget]`.
While `[budget]` sets hard per-turn and per-plan USD limits, `[energy]` controls a pool that
replenishes at a `metabolism_rate`, allowing bursty work within a rolling budget.

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

</details>

<details>
<summary>[tui] — Terminal UI preferences</summary>

```toml
[tui]
refresh_rate_ms = 250   # TUI refresh interval in milliseconds, default: 250
```

</details>

<details>
<summary>[serve] — HTTP API options</summary>

**Why this matters**: `roko serve` exposes ~85 REST routes plus SSE streaming on port 6677.
This section controls authentication, terminal access, and auto-orchestration behavior. The
`terminal_enabled` flag is off by default because it exposes a PTY shell — only enable it in
trusted environments.

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

</details>

<details>
<summary>[server] — HTTP server / gateway settings</summary>

```toml
[server]
bind = "127.0.0.1"   # bind address, default: "127.0.0.1"
port = 6677          # port number, default: 6677
cors_origins = []    # allowed CORS origins (empty = permissive), default: []
auth_token = ""      # optional bearer token for authentication
```

Empty `cors_origins` is permissive (allows all origins). Set it when deploying to production.

</details>

<details>
<summary>[scheduler] — Cron jobs</summary>

**Why this matters**: The scheduler emits signals on a cron schedule. Combined with
`[[subscriptions]]`, this lets you wire up recurring tasks — weekly knowledge digests,
nightly builds, hourly health checks — without external cron infrastructure.

```toml
[scheduler]
[[scheduler.cron]]
name = "weekly-digest"
expression = "0 9 * * MON"              # standard cron expression
signal_kind = "scheduler:cron:weekly-digest"  # engram kind emitted when fires
# metadata = { key = "value" }          # extra structured metadata (optional)
```

</details>

<details>
<summary>[webhooks] — Webhook ingress</summary>

```toml
[webhooks.github]
secret = "my-webhook-secret"   # shared secret for X-Hub-Signature-256 verification
```

The webhook endpoint is `POST /v1/webhooks/github`. Roko verifies the
`X-Hub-Signature-256` header automatically.

</details>

<details>
<summary>[subscriptions] — Event-triggered agents</summary>

**Why this matters**: Subscriptions are the glue between external events and agent execution.
A subscription says "when this signal arrives, run this agent template." This is how you
wire up automatic PR review, auto-formatting on file save, or agent triggers from GitHub
webhooks.

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

# Typed trigger — cron:
[subscriptions.trigger_config]
type = "cron"
schedule = "*/30 * * * *"

# OR — file watch:
[subscriptions.trigger_config]
type = "file_watch"
paths = ["src/", "crates/"]
extensions = ["rs", "toml"]
recursive = true

# OR — webhook:
[subscriptions.trigger_config]
type = "webhook"
event = "push"
```

</details>

<details>
<summary>[watcher] — File-system watcher</summary>

```toml
[watcher]
[[watcher.paths]]
directory = "src/"
include = ["*.rs", "*.toml"]   # accepts string or list of strings
exclude = ["target/"]
```

The watcher emits signals when watched paths change, which can trigger subscriptions.

</details>

<details>
<summary>[tools] — Global tool allowlist/denylist</summary>

**Why this matters**: The tools section controls which tools are available to agents globally
and per domain. Use this to prevent agents from writing files in research mode, or to ensure
certain tools are always available regardless of role.

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

</details>

<details>
<summary>[chain] — On-chain integration (Phase 2+)</summary>

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

This section is Phase 2+ — requires a blockchain backend for witness anchoring. Not needed
for self-hosting.

</details>

<details>
<summary>[relay] — Workspace auto-discovery</summary>

**Why this matters**: The relay makes your running Roko instance discoverable by the demo
app and other tools. When deployed on Railway or Fly.io, the public URL is auto-detected
from `RAILWAY_PUBLIC_DOMAIN` or `FLY_APP_NAME`.

```toml
[relay]
url = "wss://relay.nunchi.dev"       # relay WebSocket URL
workspace_name = "will-dev"          # human-readable workspace name (defaults to hostname)
public_url = "https://..."           # public URL of this roko instance
heartbeat_interval_secs = 30         # presence heartbeat interval, default: 30
```

</details>

<details>
<summary>[gemini] — Gemini-specific model settings</summary>

```toml
[gemini]
default_model = "gemini-2.5-flash"
grounding_model = "gemini-2.5-pro"
code_exec_model = "gemini-2.5-flash"
embed_model = "text-embedding-004"
use_free_tier = false
thinking_level = "medium"          # "minimal", "low", "medium", "high"
enable_context_caching = false

[[gemini.safety_settings]]
category = "HARM_CATEGORY_HATE_SPEECH"
threshold = "BLOCK_NONE"
```

</details>

<details>
<summary>[perplexity] — Perplexity search settings</summary>

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

</details>

<details>
<summary>[[agents]] — Multi-agent startup definitions</summary>

For multi-agent deployments, declare agents directly in `roko.toml`. Start all enabled agents
with `roko up`.

```toml
[[agents]]
name = "my-agent"
domain = "coding"
prompt = "You are an expert Rust systems programmer..."
model = "claude-sonnet-4-6"    # optional model override
chain_rpc = "https://..."      # optional chain RPC override
enabled = true
```

</details>

<details>
<summary>[deploy] — Cloud deployment</summary>

```toml
[deploy]
backend = "manual"                           # "railway-api", "railway-cli", "manual"
railway_api_token = "..."                    # Railway API token (optional)
project_id = "..."                           # Railway project ID (optional)
environment_id = "..."                       # Railway environment ID (optional)
worker_image = "ghcr.io/nunchi-trade/roko-worker:latest"  # Docker image for workers
default_region = "us-west1"                  # default deployment region (optional)
```

</details>

<details>
<summary>[oneirography] — Dream art generation (experimental)</summary>

```toml
[oneirography]
enabled = false
provider = "dall-e-3"   # image generation provider
variants = 3            # image variants per dream cycle
base_reserve = 0.01     # base reserve price for affect-reactive auctions
base_duration_seconds = 3600
```

</details>

<details>
<summary>Top-level version fields</summary>

```toml
config_version = 2    # layout version for migration tooling (current: 2)
schema_version = 2    # schema version for compatibility checks (current: 2)
```

If you have an old `config_version = 1` file, run `roko config migrate` to upgrade.

</details>

---

## 7. Provider Configuration Recipes

Copy-paste ready configurations for every supported provider.

<details>
<summary>Anthropic Claude CLI (recommended for local development)</summary>

Uses the `claude` CLI subprocess. Requires Claude Code CLI to be installed. This is the
easiest way to get started locally — no API key required if you're already authenticated
with the Claude CLI.

```toml
[providers.anthropic]
kind = "claude_cli"
command = "claude"
# args = ["--dangerously-skip-permissions"]  # if needed
# timeout_ms = 300000   # longer timeout for complex tasks
```

</details>

<details>
<summary>Anthropic API (direct HTTP)</summary>

Uses the Anthropic Messages API over HTTP. Set `ANTHROPIC_API_KEY` in the environment.
This is the recommended path for production and CI use.

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

If `ANTHROPIC_API_KEY` is set and no `[providers]` section exists, Roko synthesizes an
`anthropic_api` provider automatically.

</details>

<details>
<summary>OpenAI</summary>

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

</details>

<details>
<summary>Cerebras (ultra-fast inference)</summary>

Cerebras is useful for the `fast_task_model` in `[routing]` because latency is significantly
lower than API-based providers — often 10–20x faster for smaller models.

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

</details>

<details>
<summary>Google Gemini</summary>

OpenAI-compatible endpoint (recommended — simpler, supports tool calls):

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

Native Gemini API (for grounding and code execution features):

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

</details>

<details>
<summary>Local Ollama</summary>

No API key needed. Use for offline development or cost-free experimentation.

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

</details>

<details>
<summary>Perplexity (research tasks only)</summary>

Perplexity is used exclusively for the `roko research` commands. Do not route coding tasks
here — it has no tool-call support and is optimized for web-grounded search answers.

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

</details>

<details>
<summary>Custom gateway / proxy with extra headers</summary>

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

</details>

---

## 8. Model Registry

The model registry decouples logical model names from provider-specific slugs. The
CascadeRouter and role overrides refer to models by alias; changing the underlying provider
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

<details>
<summary>Full-featured model entry (all fields)</summary>

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

</details>

<details>
<summary>OpenRouter routing overrides</summary>

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

</details>

---

## 9. MCP Server Configuration

**Why this matters**: MCP (Model Context Protocol) servers extend the tools available to
agents. The built-in `roko-mcp-code` server gives agents code-intelligence tools — they can
search the codebase, read files, and extract symbols without burning context window on large
file reads. Without MCP, agents must inline all context into the prompt.

Roko passes MCP configuration to providers that support it (Claude CLI and Anthropic API
backends).

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

The code-intelligence MCP server (`roko-mcp-code`) provides these tools to agents:

- `list_files` — list files in a directory
- `read_file` — read file contents
- `search_code` — grep the codebase
- `get_symbols` — extract symbols from a source file

### Referencing the MCP config

In practice, MCP config is assembled programmatically in
`crates/roko-cli/src/orchestrate.rs` using `McpConfig` and `McpServerConfig` structs:

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

```bash
# Start it directly
cargo run -p roko-mcp-code --

# Use it via Claude CLI
claude --mcp-config mcp-config.json "analyze this codebase"
```

<details>
<summary>Third-party MCP servers</summary>

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

</details>

---

## 10. Agent Manifests and Safety Contracts

**Why this matters**: Safety contracts constrain what tools each agent role is allowed to
use. The researcher role should never be able to write files; the implementer role should
never be able to run arbitrary web requests. Without contracts, a single compromised prompt
could cause any agent to do anything. These constraints are enforced at the tool-dispatch
layer before any tool runs.

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

Safety contracts are YAML manifests referenced via `agent.policy_manifests`:

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

When a YAML manifest is missing for a role, the agent falls back to permissive defaults
(see Known Gaps).

### Tool role allowlists in roko.toml

Per-role tool lists can also be set directly in `roko.toml` without a manifest file:

```toml
[agent.roles.implementer]
tools = ["read_file", "edit_file", "bash", "git_commit", "write_file"]

[agent.roles.reviewer]
tools = ["read_file", "bash"]

[agent.roles.researcher]
tools = ["web_search", "web_fetch", "read_file"]
```

---

## 11. Gate Pipeline Configuration

**Why this matters**: The gate pipeline is Roko's quality control system. After every agent
task completes, the gates verify the result before committing. A failing compile gate means
the agent produced code that doesn't build. A failing test gate means tests broke. Gates
make Roko's output trustworthy even when individual agent turns are imperfect.

Rung 0 (compile) is never skipped regardless of adaptive threshold state. All other rungs can
be skipped by adaptive thresholds when they've passed consistently.

### 7-rung gate pipeline

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

Custom gates wrap any shell command. This is how you adapt the gate pipeline for non-Rust
projects or add security checks:

```toml
[gates.domain_gates]
# Domain "security" runs cargo audit instead of clippy:
security = ["shell:cargo audit --deny warnings"]

# Domain "research" skips compile/test entirely:
research = ["shell:true"]
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

`AdaptiveThresholds` tracks per-rung pass streaks with an EMA. When a rung has passed
consistently, it is temporarily skipped until a failure is observed. This saves time on
projects where certain gates almost always pass. State persists to
`.roko/learn/gate-thresholds.json`.

Tune with:

```bash
roko learn tune gates
```

Override per-role floor (prevents skipping rungs unless historical pass rate meets threshold):

```toml
[agent.roles.implementer.thresholds]
gate_pass_rate_floor = 0.65   # never skip rungs unless pass rate exceeds this
```

### Gate failure replanning

When gate failures exhaust the autofix budget and the iteration limit,
`learning.replan_on_gate_failure` triggers a plan revision — the system generates a new
implementation plan with the gate failure context injected as additional requirements:

```toml
[learning]
replan_on_gate_failure = true
replan_max_per_plan = 2      # max plan revisions per plan
replan_gate_attempts = 3     # consecutive failures before revision triggers
```

The replan emits `RokoEvent::PlanRevision` on the global event bus, which triggers a new
planning agent pass.

---

## 12. Learning Configuration

All learning state lives in `.roko/learn/`. Everything here is automatic — you don't need
to manage it manually.

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

HDC (Hyperdimensional Computing) fingerprints identify semantically similar episodes for
replay and retrieval. The `FeedbackService` computes them from the combined task + output
content.

```bash
roko learn episodes --limit 20
```

### Playbook extraction

`PlaybookStore` accumulates techniques from successful runs and stores them in
`.roko/memory/playbook.toml`. Playbooks are queried at dispatch time and injected into
layer 6 of the 9-layer system prompt. After dozens of successful runs, the playbook
becomes a highly relevant catalog of what works for your specific codebase.

```bash
roko learn all   # view current playbook state
```

### CascadeRouter bandit state

After each workflow completion, `FeedbackService` calls
`CascadeRouter::observe_outcome(role, model, success, cost, latency)`. The router updates
its LinUCB arm weights and gradually learns which model performs best for each task type.

```bash
roko learn router   # inspect bandit state per role
```

State file: `.roko/learn/cascade-router.json`

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

Each layer has an effectiveness score (`.roko/learn/section-effects.json`). Sections with
consistently low scores are trimmed first when the prompt approaches the token budget.

<details>
<summary>Other learning subsystems</summary>

**Prompt experiment tracking (A/B)**

`ModelExperimentStore` assigns variants to runs and records outcomes for statistically
grounded comparisons between prompt strategies.

```bash
roko config experiments    # list active experiments
roko learn tune routing    # tune routing weights
```

State file: `.roko/learn/experiments.json`

**Knowledge admission (A-MAC)**

Knowledge entries earn scores based on how often they correlate with successful outcomes.
Entries below a minimum score threshold are excluded from future prompts.

State file: `.roko/learn/knowledge-scores.json`

**Efficiency events**

Per-turn efficiency events (tokens/second, cost/token, time-to-first-token) are logged
to `.roko/learn/efficiency.jsonl`.

```bash
roko learn efficiency   # view efficiency events
```

**Manual tuning commands**

```bash
roko learn tune gates     # adjust adaptive gate thresholds
roko learn tune routing   # adjust routing reward weights
roko learn tune budget    # adjust budget parameters
```

</details>

---

## 13. Event System and Subscriptions

**Why this matters**: The event bus is how you observe what Roko is doing in real time and
how you wire up external triggers. The SSE stream at `/v1/events` is what the TUI, demo app,
and any external dashboard consume. Subscriptions let you make Roko react to GitHub webhooks,
file changes, cron schedules, or any custom signal.

### Event bus

The global `EventBus<RuntimeEvent>` is the backbone for all async communication between the
orchestrator, HTTP server, TUI, and external consumers.

Every `RuntimeEvent` carries:
- `run_id` — stable identifier for the workflow run
- `seq` — monotonically increasing sequence number
- `ts` — RFC 3339 timestamp
- `schema_version` — for forward compatibility
- `source` — emitting component
- `payload` — typed event data

Event kinds:
- Lifecycle: `WorkflowStarted`, `PhaseTransition`, `WorkflowCompleted`
- Agent: `AgentSpawned`, `AgentOutput`, `AgentCompleted`, `AgentFailed`
- Gates: `GateStarted`, `GatePassed`, `GateFailed`
- Feedback: `FeedbackRecorded`
- Persistence: `StateCheckpointed`

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

If `prd.auto_plan = true` and `roko serve` is running, a built-in subscriber fires on
every PRD publish event — no manual subscription entry needed:

```toml
[prd]
auto_plan = true

[serve]
auto_orchestrate = true
```

---

## 14. HTTP Control Plane

Start the server:

```bash
roko serve
# Listening on http://127.0.0.1:6677
```

The server exposes approximately 85 REST routes plus SSE streaming. All routes under `/v1/`
can be protected with `serve.auth`.

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

Single shared key:

```toml
[serve.auth]
enabled = true
api_key = "my-secret-key"
```

Named keys with scopes:

```toml
[[serve.auth.api_keys]]
name = "ci-pipeline"
key_hash = "abc123..."   # SHA-256 hex of the plaintext key
scope = "agent:write"    # "admin", "agent:write", "read"
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

Each running agent can expose its own HTTP sidecar on a dynamically assigned port:

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

## 15. Error Recovery

### Resuming after interruption

The executor checkpoint is written atomically after every phase transition. If a plan run
is interrupted (Ctrl-C, process crash, network failure), resume with:

```bash
roko plan run plans/my-plan/ \
  --resume .roko/state/executor.json
```

The checkpoint restores the exact pipeline state — phase, iteration count, accumulated review
findings — so no work is duplicated.

### Manual checkpoint inspection

```bash
# View checkpoint state
cat .roko/state/executor.json | jq

# View event log
cat .roko/state/events.json | jq
```

### Gate failure recovery flow

When gates fail the orchestrator tries three recovery strategies in order:

1. **Autofix** — spawn a fast model (`conductor.auto_fix_model`) with the gate failure output
   injected as context. Attempts up to `conductor.max_auto_fix_attempts` times.

2. **Replan** — when autofix budget is exhausted and `learning.replan_on_gate_failure = true`,
   a new planning agent pass runs with the gate failure context. Happens at most
   `learning.replan_max_per_plan` times per plan.

3. **Halt** — if replan budget is also exhausted, the task is marked failed and the plan is
   halted. Inspect with `roko plan show <plan-id>`.

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

To skip already-completed tasks, use `roko plan show` to inspect which task IDs completed,
then mark tasks complete manually with `roko plan edit`.

---

## 16. Deployment Guide

### Railway deployment

```bash
roko deploy railway
```

This reads `[serve.deploy]` from `roko.toml`, creates or updates a Railway service using
the worker image, sets the required environment variables, and configures the start command
as `roko serve`.

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

Enable relay registration so the demo app auto-discovers your deployed instance:

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

Builds and tags a Docker image from the Dockerfile in the workspace root. The image uses
a multi-stage build, copies only the `roko-cli` binary, exposes port 6677, and defaults
to `roko serve` as the entrypoint.

```bash
docker run -p 6677:6677 \
  -e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" \
  -e PERPLEXITY_API_KEY="$PERPLEXITY_API_KEY" \
  -v "$(pwd):/workspace" \
  roko-worker:latest serve
```

### System daemon (launchd / systemd)

Install Roko as a system service that starts on boot:

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

## 17. Environment Variables Reference

### Direct config overrides

These environment variables override values in `roko.toml` at startup. They take precedence
over everything in the file:

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

If `ANTHROPIC_API_KEY` is present and no `[providers]` section is configured, Roko
auto-synthesizes an `anthropic_api` provider.

If `ANTHROPIC_API_KEY` is not present but the `claude` CLI is available, Roko
auto-synthesizes a `claude_cli` provider.

### Deployment detection

Read automatically when deploying to cloud platforms:

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

In `[providers.*.extra_headers]`, keys ending in `_file` have their values treated as file
paths. The file content is read and used as the header value:

```toml
[providers.my-provider.extra_headers]
Authorization_file = "/run/secrets/api-token"
# Becomes: Authorization: <file contents>
```

---

## 18. WorkflowEngine Integration (Programmatic)

This section shows how to wire up a `WorkflowEngine` programmatically, which is what the CLI,
HTTP server, and ACP adapter all do internally. Use this if you're building a custom integration
or want to embed Roko in another application.

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

---

## 19. Key File Locations

| What | Path |
|---|---|
| Workspace config | `<project>/roko.toml` |
| Data directory | `<project>/.roko/` |
| Layout version | `<project>/.roko/VERSION` |
| Executor checkpoint | `<project>/.roko/state/executor.json` |
| Episode log | `<project>/.roko/memory/episodes.jsonl` |
| Playbook | `<project>/.roko/memory/playbook.toml` |
| Cascade router state | `<project>/.roko/learn/cascade-router.json` |
| Gate thresholds | `<project>/.roko/learn/gate-thresholds.json` |
| Prompt experiments | `<project>/.roko/learn/experiments.json` |
| Knowledge scores | `<project>/.roko/learn/knowledge-scores.json` |
| Section effectiveness | `<project>/.roko/learn/section-effects.json` |
| Efficiency events | `<project>/.roko/learn/efficiency.jsonl` |
| Custody audit chain | `<project>/.roko/custody.jsonl` |
| Witness DAG log | `<project>/.roko/witness.jsonl` |
| Gap tracker | `<project>/.roko/GAPS.md` |
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

## 20. Known Gaps

The following items are built but not fully wired at runtime. The canonical gap tracker is
`.roko/GAPS.md`.

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
