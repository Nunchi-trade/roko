# Roko Integration Guide

Roko is a Rust toolkit for building agents that build themselves. This guide covers installation, configuration, the self-hosting workflow, and the HTTP control plane.

## Getting Started

### Prerequisites

- Rust toolchain 1.91+ (`rustup update stable`)
- At least one LLM provider (Claude CLI, Anthropic API, Ollama, etc.)

### Install and Build

```bash
git clone https://github.com/nunchi/roko.git
cd roko
cargo build --workspace --release
```

The binary is at `target/release/roko`. Add it to your PATH or use `cargo install --path crates/roko-cli`.

### Initialize a Project

```bash
roko init
```

This creates:
- `.roko/` directory for state, signals, episodes, and learning data
- `roko.toml` configuration file with sensible defaults

Options:
```bash
roko init --cloud         # Cloud-ready defaults for deployment
roko init --profile rust  # Language-specific presets (rust, typescript, go, python)
```

### Verify Installation

```bash
roko status
```

This prints signal counts, episode history, and gate pass/fail rates.

---

## Configuring LLM Providers

Roko supports multiple LLM backends simultaneously. Configure them in `roko.toml` under `[providers.*]` and `[models.*]`.

### Claude CLI (default)

Uses the `claude` CLI tool installed locally:

```toml
[providers.claude_cli]
kind = "claude_cli"
command = "claude"
timeout_ms = 120000
```

No API key needed -- authenticates through the Claude CLI's own session.

### Anthropic API (direct HTTP)

```toml
[providers.anthropic]
kind = "anthropic_api"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
timeout_ms = 120000

[models.claude-opus]
provider = "anthropic"
slug = "claude-opus-4-6"
context_window = 200000
supports_tools = true
tool_format = "anthropic_blocks"
cost_input_per_m = 15.0
cost_output_per_m = 75.0

[models.claude-sonnet]
provider = "anthropic"
slug = "claude-sonnet-4-6"
context_window = 200000
supports_tools = true
tool_format = "anthropic_blocks"
cost_input_per_m = 3.0
cost_output_per_m = 15.0
```

Set the environment variable:
```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Ollama (OpenAI-compatible)

```toml
[providers.ollama]
kind = "openai_compat"
base_url = "http://localhost:11434/v1"
timeout_ms = 300000

[models.llama3]
provider = "ollama"
slug = "llama3:70b"
context_window = 128000
supports_tools = true
tool_format = "openai_json"
```

### OpenAI / OpenAI-compatible

```toml
[providers.openai]
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
timeout_ms = 120000

[models.gpt4o]
provider = "openai"
slug = "gpt-4o"
context_window = 128000
supports_tools = true
tool_format = "openai_json"
cost_input_per_m = 2.5
cost_output_per_m = 10.0
```

### OpenRouter (multi-provider gateway)

```toml
[providers.openrouter]
kind = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
extra_headers = { "HTTP-Referer" = "https://your-app.com" }

[models.claude-via-openrouter]
provider = "openrouter"
slug = "anthropic/claude-sonnet-4-6"
context_window = 200000
supports_tools = true
tool_format = "openai_json"
provider_routing = { sort = "price", allow_fallbacks = true }
```

### Cursor (ACP protocol)

```toml
[providers.cursor]
kind = "cursor_acp"
timeout_ms = 120000
```

### Gemini

```toml
[providers.gemini]
kind = "openai_compat"
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key_env = "GEMINI_API_KEY"

[models.gemini-2]
provider = "gemini"
slug = "gemini-2.5-pro"
context_window = 1000000
supports_tools = true
supports_thinking = true
supports_grounding = true
tool_format = "openai_json"
```

### Perplexity

```toml
[providers.perplexity]
kind = "perplexity_api"
base_url = "https://api.perplexity.ai"
api_key_env = "PERPLEXITY_API_KEY"

[models.sonar-pro]
provider = "perplexity"
slug = "sonar-pro"
context_window = 200000
supports_web_search = true
tool_format = "openai_json"
```

### Selecting the Default Model

```toml
[agent]
default_model = "claude-sonnet-4-6"
default_backend = "claude"
default_effort = "medium"   # low, medium, high, max
context_limit_k = 200
bare_mode = true
```

### Environment Variable Overrides

All agent settings can be overridden at runtime:

| Variable | Overrides |
|---|---|
| `ROKO_MODEL` | `agent.default_model` |
| `ROKO_BACKEND` | `agent.default_backend` |
| `ROKO_EFFORT` | `agent.default_effort` |
| `ROKO_PROVIDER` | Provider for the default model |
| `ROKO_CONTEXT_LIMIT_K` | `agent.context_limit_k` |
| `ROKO_MAX_AGENTS` | `conductor.max_agents` |
| `ROKO_BUDGET_USD` | `budget.max_plan_usd` |
| `ROKO_PARALLEL` | `conductor.parallel_enabled` |
| `ROKO_EXPRESS` | `conductor.express_mode` |
| `ROKO_SKIP_TESTS` | `gates.skip_tests` |
| `ROKO_CLIPPY` | `gates.clippy_enabled` |

---

## The Self-Hosting Workflow

Roko develops itself through a PRD-to-execution pipeline. Each step is a CLI command:

### 1. Capture an Idea

```bash
roko prd idea "Wire SystemPromptBuilder into orchestrate.rs"
```

Creates a signal in `.roko/prd/ideas/`.

### 2. Draft a PRD

```bash
roko prd draft new "system-prompt-wiring"
```

An agent refines the idea into a structured PRD with requirements, acceptance criteria, and scope.

### 3. Research (optional)

```bash
roko research enhance-prd system-prompt-wiring
```

Deep-researches the topic and enriches the PRD with citations and technical context.

### 4. Generate an Implementation Plan

```bash
roko prd plan system-prompt-wiring
```

An agent reads the PRD and generates a `tasks.toml` file with ordered, dependency-aware tasks.

### 5. Execute the Plan

```bash
roko plan run plans/
```

The orchestrator:
1. Discovers plans in the directory
2. Builds a DAG from task dependencies
3. Dispatches agents to execute tasks (with parallelism if configured)
4. Runs gates after each task (compile, test, clippy, diff)
5. Persists state to `.roko/state/executor.json`
6. Records episodes to `.roko/episodes.jsonl`

### 6. Resume After Interruption

```bash
roko plan run plans/ --resume .roko/state/executor.json
```

Picks up where it left off. Already-completed tasks are skipped.

### 7. Monitor Progress

```bash
roko dashboard        # Interactive ratatui TUI (F1-F7 tabs)
roko status           # Quick text summary
roko status --cfactor # Include C-Factor computation
```

### 8. Check PRD Coverage

```bash
roko prd status
```

Shows how many PRDs have plans, how many tasks are complete, and coverage ratios.

---

## HTTP Control Plane

Start the HTTP server:

```bash
roko serve                          # Default: 127.0.0.1:9090
roko serve --bind 0.0.0.0 --port 6677
```

The server exposes ~85 REST endpoints, SSE streaming, and WebSocket connections under `/api/`.

### Authentication

```toml
[serve.auth]
enabled = true
api_key = "your-secret-key"
```

When enabled, all `/api/` endpoints require the `Authorization: Bearer <key>` header.

### Key Endpoint Groups

| Prefix | Purpose |
|---|---|
| `/api/status` | Health, metrics, session info |
| `/api/plans` | Plan CRUD and execution |
| `/api/prds` | PRD lifecycle management |
| `/api/run` | Single-prompt agent runs |
| `/api/agents` | Agent registration and management |
| `/api/research` | Research topic/enhance endpoints |
| `/api/learning` | Efficiency, cascade router, experiments |
| `/api/config` | Config read/write/reload |
| `/api/templates` | Agent template CRUD and deploy |
| `/api/subscriptions` | Event subscription management |
| `/api/deployments` | Cloud deployment management |
| `/api/providers` | Provider health and testing |
| `/api/models` | Model registry |
| `/api/projections` | StateHub read/watch |
| `/webhooks/` | GitHub, Slack, generic ingress |
| `/ws` | WebSocket real-time events |
| `/api/events` | SSE event stream |

See `docs/API-REFERENCE.md` for the full endpoint catalog.

---

## Configuration Reference

### `[project]`

```toml
[project]
name = "my-project"          # Human-readable name
root = "."                   # Project root directory
fresh_base_branch = "main"   # Base branch for worktrees
```

### `[agent]`

```toml
[agent]
default_model = "claude-sonnet-4-6"
default_backend = "claude"
default_effort = "medium"
temperament = "balanced"      # balanced, exploratory, conservative, creative
context_limit_k = 200
bare_mode = true

# Per-role overrides
[agent.roles.implementer]
model = "claude-opus-4-6"
effort = "high"
temperament = "exploratory"
tools = ["read", "edit", "bash", "git-*"]

[agent.roles.reviewer]
model = "claude-sonnet-4-6"
effort = "medium"
tools = ["read", "grep"]
```

### `[providers.<name>]`

```toml
[providers.anthropic]
kind = "anthropic_api"          # claude_cli, anthropic_api, openai_compat, cursor_acp, perplexity_api
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
timeout_ms = 120000
ttft_timeout_ms = 15000         # Time-to-first-token timeout
connect_timeout_ms = 5000
max_concurrent = 5
```

### `[models.<name>]`

```toml
[models.claude-opus]
provider = "anthropic"
slug = "claude-opus-4-6"
context_window = 200000
max_output = 32000
supports_tools = true
supports_thinking = false
supports_vision = false
supports_web_search = false
tool_format = "anthropic_blocks"   # anthropic_blocks, openai_json
cost_input_per_m = 15.0
cost_output_per_m = 75.0
```

### `[gates]`

```toml
[gates]
clippy_enabled = true     # Run clippy/lint gate
skip_tests = false        # Skip test execution gate
max_iterations = 3        # Max gate retry attempts
```

### `[budget]`

```toml
[budget]
max_plan_usd = 25.0       # Max spend per plan
max_turn_usd = 3.0        # Max spend per single turn
prompt_token_budget = 10000
```

### `[conductor]`

```toml
[conductor]
max_agents = 4              # Max concurrent agents
max_parallel_plans = 2
parallel_enabled = false    # Enable parallel task execution
express_mode = false        # Single agent, no reviews, auto-fix
auto_advance_batch = true
auto_merge_on_complete = false
max_auto_fix_attempts = 3
```

### `[routing]`

```toml
[routing]
mode = "adaptive"           # adaptive, static, manual
fast_task_model = "claude-haiku-4-5"
standard_task_model = "claude-sonnet-4-6"
complex_task_model = "claude-opus-4-6"

[routing.weights]
quality = 0.5
cost = 0.3
latency = 0.2
```

### `[learning]`

```toml
[learning]
auto_playbook_refresh = true
knowledge_warnings = true
learning_min_occurrences = 3
replan_on_gate_failure = true
replan_max_per_plan = 2
replan_gate_attempts = 3
```

### `[prd]`

```toml
[prd]
auto_plan = false   # Auto-generate plan when PRD is promoted
```

### `[serve]`

```toml
[serve]
auto_orchestrate = false

[serve.auth]
enabled = false
api_key = ""

[serve.deploy]
provider = "manual"
environment = []
```

### `[webhooks]`

```toml
[webhooks.github]
secret = "your-webhook-secret"
```

---

## Custom Gates and Templates

### Custom Gate Pipeline

Gates validate agent output after each task. The default pipeline runs: compile, test, clippy, diff-review. Configure via `[gates]`:

```toml
[gates]
clippy_enabled = true
skip_tests = false
max_iterations = 3
```

Adaptive thresholds automatically adjust pass rates based on historical performance (stored in `.roko/learn/gate-thresholds.json`).

### Agent Templates

Templates define reusable agent configurations for the HTTP API:

```bash
# Create via API
curl -X POST http://localhost:9090/api/templates \
  -H "Content-Type: application/json" \
  -d '{
    "name": "code-reviewer",
    "description": "Reviews PRs for correctness and style",
    "model": "claude-sonnet-4-6",
    "role": "reviewer",
    "system_prompt": "You are a thorough code reviewer...",
    "output_format": "markdown"
  }'

# Deploy a template (spawn an agent from it)
curl -X POST http://localhost:9090/api/templates/code-reviewer/deploy
```

### Scaffold Custom Components

```bash
roko new gate my-security-gate
roko new scorer my-quality-scorer
roko new router my-custom-router
roko new template my-agent-template
```

---

## Knowledge and Dreams

### Knowledge Store (Neuro)

Roko maintains a durable knowledge store that persists across sessions:

```bash
roko neuro search "error handling patterns"
roko neuro list
```

Knowledge entries have confidence scores that decay over time (demurrage). This prevents stale knowledge from dominating decisions.

```toml
[demurrage]
rate_per_hour = 0.001
min_balance = 0.1
freeze_threshold = 0.05
gc_interval_secs = 3600
```

### Learning Subsystem

The learning system tracks:
- **Efficiency events** -- per-turn cost and token usage (`.roko/learn/efficiency.jsonl`)
- **Cascade router** -- model selection routing data (`.roko/learn/cascade-router.json`)
- **Prompt experiments** -- A/B test results (`.roko/learn/experiments.json`)
- **Gate thresholds** -- adaptive pass rates (`.roko/learn/gate-thresholds.json`)

```bash
roko learn all          # Show all learning subsystem state
roko learn router       # Show cascade router decisions
roko learn efficiency   # Show agent efficiency data
roko learn experiments  # Show active experiments
```

### Dreams (Offline Consolidation)

The dream system runs offline processing cycles:

```bash
roko dream run          # Trigger a dream cycle
roko dream report       # Show the latest dream report
roko dreams list        # List dream archive entries
```

---

## Troubleshooting

### "alloy deps need 1.91+"

```bash
rustup update stable
rustc --version  # Should be >= 1.91
```

### Agent spawns fail

1. Check the provider is configured: `roko provider list`
2. Test the provider: via the HTTP API `POST /api/providers/{id}/test`
3. Check API key is set: ensure the env var from `api_key_env` is exported
4. Check timeout: increase `timeout_ms` if the provider is slow

### Gates always fail

```bash
roko tune gates --dry-run   # Show current thresholds
roko learn all              # Check gate pass rates
```

Try:
- `ROKO_SKIP_TESTS=1 roko plan run plans/` to skip tests temporarily
- `ROKO_CLIPPY=0` to disable clippy gate
- Increase `gates.max_iterations` for more retries

### Plan execution stalls

```bash
roko status                              # Check current state
roko plan run plans/ --resume .roko/state/executor.json  # Resume
```

### Config issues

```bash
roko config show    # Display resolved config
roko config path    # Print config file location
roko explain gates  # Explain how gates work (depth 1-3)
```

### Server won't start

- Check port availability: `lsof -i :9090`
- Try a different port: `roko serve --port 8080`
- Check config: ensure `[serve]` section is valid

### MCP configuration

Pass MCP servers to agents via config:

```toml
[agent]
mcp_config = "/path/to/mcp-servers.json"
```

Or auto-discovery finds MCP configs in the workspace.
