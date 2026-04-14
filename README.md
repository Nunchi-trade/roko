# Roko

Roko is a Rust toolkit for building agents that build themselves.

Point it at a codebase, describe what you want, and roko handles the rest: it composes prompts, dispatches LLM agents, verifies output with compilation and test gates, persists results as content-addressed signals, and learns from outcomes to get better over time. The core loop is observe, plan, execute, verify, learn, repeat.

18 crates. ~200K lines of Rust. 1,600+ tests.

## Quick start

```bash
cargo install --path crates/roko-cli
roko init my-project && cd my-project
roko run "add a health check endpoint to the API"
```

`roko init` detects your project type, sets up gates (cargo check for Rust, tsc for TypeScript, go build for Go), and writes a working `roko.toml`. `roko run` does everything else.

## How it works

### One-shot execution

The fastest path. One command, full pipeline.

```bash
roko run "refactor the auth module to use JWT"
```

Roko composes a prompt from your codebase context, calls the configured LLM agent, runs the output through compile/test/lint gates, and persists the result as a signal. If gates fail, it retries with escalating models.

### Full planning pipeline

For larger work that spans multiple tasks.

```bash
# 1. Capture what you want to build
roko prd idea "Add user authentication with OAuth2"

# 2. Research the topic (optional -- uses Perplexity for web-grounded citations)
roko research topic "OAuth2 best practices in Rust"

# 3. Generate a detailed PRD (agent-assisted)
roko prd draft new "oauth2-auth"

# 4. Create an implementation plan with tasks
roko prd plan oauth2-auth

# 5. Execute the plan (agents work in parallel, gates verify each task, state persists)
roko plan run plans/

# 6. Resume if interrupted
roko plan run plans/ --resume-plan

# 7. Watch progress
roko dashboard
```

Each task in the plan runs through its own agent loop with independent gate verification. Failed tasks feed back into the planner for re-decomposition.

### Implicit prompt mode

If no subcommand matches, roko treats the argument as a prompt:

```bash
roko "fix the bug in auth.rs"
```

This is equivalent to `roko run "fix the bug in auth.rs"`. The shortest path from thought to execution.

## Dashboard

`roko dashboard` launches an interactive terminal UI built on ratatui with the rosedust color theme. Seven tabs, accessible via F1-F7:

| Key | Tab | What it shows |
|-----|-----|---------------|
| F1 | Dashboard | Health gauges, plan progress, cost tracking, system metrics |
| F2 | Plans | Plan tree, task progress bars, wave overview |
| F3 | Agents | Live agent output, diffs, token burn, parallel pool status |
| F4 | Git | Branch tree, commit graph, worktree list |
| F5 | Logs | Scrollable log viewer with level filtering |
| F6 | Config | Effective config view with source annotations |
| F7 | Inspect | Signal DAG inspector, episode replay |

Additional keybindings: `q` to quit, `?` for help, `Tab`/`Shift+Tab` to cycle panels, `Enter` to drill into a task, `i` to inject a signal into a running session.

When idle, the dashboard shows recent episodes, gate results, system health, and config summary rather than blank panels.

## Multi-provider support

Roko routes work across LLM providers based on task complexity, cost, and latency. Supported backends:

| Provider | Kind | What it does |
|----------|------|-------------|
| Claude | CLI or API | Primary coding agent (Opus, Sonnet, Haiku) |
| Gemini | Native API + OpenAI-compat | 1M context, grounding, code execution, context caching |
| Perplexity | Search + deep research | Web-grounded research with citations |
| OpenRouter | Multi-model routing | Access any model through one API |
| Ollama | Local inference | Run open models locally (Llama, Gemma, Qwen) |
| Any OpenAI-compatible API | Generic adapter | GLM, Kimi, Groq, Together, and others |

Tier-based model routing assigns the cheapest viable model to each task:

```toml
[agent.tier_models]
mechanical = "gemini-2-5-flash-lite"   # imports, renames, trivial edits
focused = "gemini-2-5-flash"           # single functions, tests
integrative = "claude-sonnet-4-6"      # multi-module wiring
architectural = "claude-opus-4-6"      # API design, architecture
```

On failure, roko escalates to the next tier's model automatically.

See `examples/` for complete provider configurations:
- `roko-gemini.toml` -- Gemini-only with 8 model tiers
- `roko-multi-provider.toml` -- Claude + Gemini + Perplexity routing
- `roko-perplexity.toml` -- Research-focused with deep research

## Architecture

### One noun, six verbs

Everything in roko is a **Signal** -- a content-addressed (BLAKE3), timestamped, scored record of something that happened. Signals form a DAG through parent pointers, so you can always trace why the agent made a decision by walking backwards through lineage.

Six traits define what you can do with signals:

| Trait | Job |
|-------|-----|
| `Substrate` | Store and query signals (memory, disk, chain) |
| `Scorer` | Rate signal relevance (recency, novelty, priority) |
| `Gate` | Verify output against ground truth (compile, test, lint) |
| `Router` | Pick among options (top-K, Thompson bandit, cascade) |
| `Composer` | Pack signals into token-budgeted prompts |
| `Policy` | React to patterns over time (episodes, retries, escalation) |

### Universal loop

Every agent runs the same loop:

```
query -> score -> route -> compose -> act -> verify -> write -> react
```

Stop at any step and you still have something useful. A prompt composer without an agent is a retrieval pipeline. An agent without gates is a raw LLM wrapper. The pieces are independent.

### Crate map

| Crate | What it does |
|-------|-------------|
| `roko-core` | Signal type, six trait definitions, config schema, tool system, errors |
| `roko-agent` | LLM backends (Claude, Gemini, Perplexity, Ollama, OpenAI-compat), tool loop, MCP, safety |
| `roko-orchestrator` | Plan DAG, parallel executor, merge queue, worktree manager, safety policy |
| `roko-gate` | 14 gate types, 7-rung pipeline, adaptive thresholds, artifact store |
| `roko-compose` | Prompt assembly, 9 role templates, U-shape placement, token budgeting |
| `roko-conductor` | 10 watchers, circuit breaker, intervention policy |
| `roko-learn` | Episodes, playbooks, bandits, model routing, prompt experiments, efficiency tracking |
| `roko-neuro` | Durable knowledge store, distillation, tier progression, garbage collection |
| `roko-dreams` | Offline dream cycle: batch episodes, cluster, distill knowledge, promote playbooks |
| `roko-serve` | HTTP API server with REST routes, SSE, WebSocket, webhook ingestion |
| `roko-cli` | CLI binary, interactive TUI dashboard, all subcommands |
| `roko-fs` | Append-only JSONL substrate with compaction and GC |
| `roko-std` | Default trait impls (memory substrate, simple routers, no-op scorers) |
| `roko-plugin` | Plugin SDK (event sources, feedback collectors) |
| `bardo-runtime` | Process supervisor, typed event bus, cancellation |
| `bardo-primitives` | 10,240-bit hyperdimensional vectors, Hamming similarity |
| `roko-index` | Code parser, symbol graph, PageRank, HDC fingerprints |
| `roko-lang-*` | Language support for Rust, TypeScript, Go |

## Gate pipeline

Every agent output passes through a gate pipeline before it is accepted. Gates run sequentially and short-circuit on the first failure by default.

### Rungs

The pipeline uses a 7-rung system. Which rungs execute depends on task complexity -- trivial tasks skip expensive checks, complex tasks run all of them.

| Rung | Gate | What it checks |
|------|------|---------------|
| 0 | Compile | `cargo check`, `tsc`, `go build` -- does it build? |
| 1 | Lint | `cargo clippy`, `eslint` -- does it pass linting? |
| 2 | Test | `cargo test` -- do existing tests pass? |
| 3 | Symbol | Symbol manifest check -- did the change break any public API? |
| 4 | GeneratedTest | Agent-generated behavioral tests |
| 5 | PropertyTest | Property-based tests (proptest/quickcheck) |
| 6 | Integration | Full integration scenario |

Additional specialized gates: `DiffGate` (patch analysis), `LlmJudge` (subjective quality), `FactCheck` (search-backed verification), `CodeExec` (sandboxed execution).

### Adaptive thresholds

Gate thresholds adjust over time using exponential moving averages. If a gate consistently passes, its threshold tightens. If it consistently fails, the threshold relaxes. Thresholds persist to `.roko/learn/gate-thresholds.json`.

## Learning and self-improvement

Roko tracks its own performance and gets better with use.

### Cascade router

Three-stage model selection: static tier mapping, learned bandit weights, and provider health. The router picks the cheapest model that can handle the task, based on historical success rates.

```bash
roko model route claude-sonnet-4-6 --explain --complexity focused
```

### Prompt experiments

A/B test different prompt strategies. The experiment store tracks success rates per variant and promotes winners automatically.

```bash
roko experiment list
roko experiment show <id>
```

### Efficiency tracking

Every agent turn records tokens in/out, latency, cost, and gate pass/fail. These events feed the cascade router, the dashboard, and the dream cycle.

### Knowledge distillation (neuro)

Completed episodes are distilled into durable knowledge entries: facts, insights, heuristics, procedures, constraints, and anti-knowledge. Knowledge decays over time with configurable half-lives (365 days for facts, 30 days for insights, 90 days for heuristics).

```bash
roko neuro query "authentication patterns"
roko neuro stats
```

### Dream cycle

Offline consolidation that runs between work sessions. The dream engine batches completed episodes, clusters them by task shape, distills knowledge, and promotes reliable success patterns into playbooks.

```bash
roko dream run
roko dream report
roko dream schedule
```

## Deployment

### Local server

```bash
roko serve --bind 0.0.0.0 --port 9090
```

Starts an HTTP API server with REST endpoints for plans, PRDs, agents, config, learning data, and provider health. Includes SSE and WebSocket for real-time streaming.

### Background daemon

```bash
roko daemon start --port 9090    # start in background
roko daemon status               # check if running
roko daemon logs -f              # tail logs
roko daemon stop                 # graceful shutdown
roko daemon install              # install as macOS launchd service
```

The daemon watches for file changes, processes cron-scheduled tasks, ingests webhooks, and dispatches agents from event subscriptions.

### Cloud worker

```bash
roko worker --port 8080
```

Reads a template from environment variables and serves tasks. Designed for Railway, Fly.io, and container platforms.

### Cloud deployment

```bash
roko deploy railway    # deploy via Railway GraphQL API
roko deploy fly        # generate fly.toml and deploy
roko deploy docker     # build and tag Docker image
```

## Configuration

Roko uses layered TOML configuration: global (`~/.config/roko/config.toml`) merged with project (`./roko.toml`), with environment variables as overrides.

### Minimal config

```toml
[agent]
command = "claude"
model = "claude-sonnet-4-6"

[[gate]]
kind = "compile"

[[gate]]
kind = "test"

[budget]
max_plan_usd = 10.0
max_task_usd = 1.0
```

### Full project config

```toml
[agent]
command = "claude"
args = ["--print", "--output-format", "stream-json"]
model = "claude-sonnet-4-6"
effort = "high"
bare_mode = true
fallback_model = "claude-haiku-4-5"
timeout_ms = 300000

[agent.tier_models]
mechanical = "claude-haiku-4-5"
focused = "claude-sonnet-4-6"
integrative = "claude-sonnet-4-6"
architectural = "claude-opus-4-6"

[agent.escalation]
max_retries = 3
escalate_model = true

[prompt]
token_budget = 50000
role = "You are a Roko agent working on the project."

[budget]
max_plan_usd = 10.0
max_task_usd = 1.0
warn_at_percent = 80

[[gate]]
kind = "compile"

[[gate]]
kind = "test"
```

### Config management

```bash
roko config init                            # interactive wizard
roko config show                            # effective merged config
roko config set agent.model claude-opus-4-6 # set a value
roko config validate                        # check syntax and references
roko config migrate                         # upgrade legacy format
```

## CLI quick reference

| Command | What it does |
|---------|-------------|
| `roko init [path]` | Create `.roko/` directory and `roko.toml` |
| `roko run "<prompt>"` | Execute prompt through the full loop |
| `roko plan run <dir>` | Execute a plan directory (the main orchestration loop) |
| `roko prd idea "<text>"` | Capture a work item |
| `roko prd draft new "<title>"` | Generate a PRD (agent-assisted) |
| `roko prd plan <slug>` | Generate implementation plan from PRD |
| `roko research topic "<topic>"` | Deep research with citations |
| `roko status` | Signal counts, recent episodes, gate results |
| `roko dashboard` | Interactive terminal dashboard |
| `roko neuro query "<topic>"` | Search durable knowledge |
| `roko dream run` | Run offline knowledge consolidation |
| `roko config init` | Interactive setup wizard |
| `roko serve` | Start HTTP API server |
| `roko daemon start` | Start background daemon |
| `roko deploy railway` | Deploy to Railway |

Full reference with all 85+ commands, flags, and examples: [docs/CLI-REFERENCE.md](docs/CLI-REFERENCE.md)

## Building and testing

```bash
rustup update stable          # 1.91+ required for alloy deps
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --no-deps -- -D warnings
```

### Running a single crate

```bash
cargo test -p roko-core
cargo test -p roko-agent
cargo test -p roko-gate
```

## Contributing

Contributions are welcome. A few ground rules:

1. **Search before writing.** This codebase has 18 crates and 200K lines. The thing you want to build might already exist. Run `grep -rn 'StructName' crates/ --include='*.rs'` first.
2. **Wire, don't build.** The most common pattern in this repo is "built but never connected." Before adding new code, check if existing code needs to be called from the runtime.
3. **Verify before marking done.** Run the actual CLI code path. Passing unit tests does not mean the feature works end-to-end.
4. **All tests must pass.** `cargo test --workspace` and `cargo clippy --workspace --no-deps -- -D warnings` must both be clean.

## License

MIT OR Apache-2.0 (dual-licensed).
