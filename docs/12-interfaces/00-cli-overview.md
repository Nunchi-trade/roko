# CLI Overview — `roko` as Primary Interface

> The `roko` binary is the primary interface to the Roko cognitive agent framework, supporting five interaction modes from one-shot commands to long-lived daemon services.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Engram, Synapse traits, and the universal cognitive loop
**Key sources**: `refactoring-prd/06-interfaces.md`, `refactoring-prd/10-developer-guide.md`, `roko-cli/src/main.rs`, `roko-cli/src/lib.rs`, `bardo-backup/prd/18-interfaces/01-cli.md`, `bardo-backup/prd/25-mori/mori-interfaces.md`

---

## Abstract

The `roko` binary is a single Rust executable that serves as the canonical entry point for all Roko operations. It is built with `clap` for argument parsing and exposes a rich subcommand tree covering agent lifecycle management, plan orchestration, PRD-driven development, knowledge operations, research, deployment, and system introspection. Unlike systems that separate CLI, TUI, and server into different binaries, Roko unifies them: the same binary that runs `roko run "fix the bug"` also hosts `roko dashboard` (the interactive TUI), `roko serve` (the HTTP API), and `roko daemon --start` (the background service).

The CLI is designed around the principle of **progressive disclosure**: beginners see three commands (`roko init`, `roko run`, `roko status`), intermediates configure behavior through `roko.toml`, and advanced users compose custom Synapse trait implementations in Rust. This layered approach ensures that Roko is approachable for a developer who just wants to run an agent against their codebase, while providing full architectural control for those building domain-specific cognitive systems.

The CLI sits at the **Application layer** — above L4 Orchestration. It consumes crates from every layer: `roko-core` (L0/L1), `roko-compose` (L2), `roko-gate` (L3), `roko-orchestrator` (L4), and the cognitive cross-cuts (`roko-neuro`, `roko-daimon`, `roko-dreams`). The `roko-serve` crate provides the HTTP server that `roko serve` starts.

---

## Five CLI Modes

Roko supports five distinct interaction modes through the same binary. The mode is selected by invocation pattern, not by a separate flag:

### 1. One-Shot Mode

```bash
roko run "Add error handling to the auth module"
```

Executes a single prompt through the full universal cognitive loop (query → score → route → compose → act → verify → persist) and exits. This is the simplest and most common usage. The loop runs once: the prompt becomes an Engram of kind `Task`, flows through the Synapse pipeline, dispatches to an LLM backend, verifies with the gate pipeline, and writes results to disk.

One-shot mode is implemented in `roko-cli/src/run.rs` via the `run_once` function. It loads the layered configuration (`roko.toml` → env vars → CLI flags), wires the default trait implementations, and drives a single iteration.

**Exit codes:**
- `0` — success (agent output passed all gates)
- `1` — agent or gate failure (the build failed logically)
- `2` — system error (I/O, config, infrastructure)

### 2. REPL Mode

```bash
roko repl
```

Opens an interactive read-eval-print loop. Each line entered becomes a prompt that runs through the cognitive loop. State persists across prompts — the Substrate accumulates Engrams, the Daimon tracks affect across turns, and Neuro knowledge entries persist. This mode is useful for exploratory development where the operator wants to iterate on prompts and observe how the agent learns across interactions.

The REPL is implemented in `roko-cli/src/repl.rs`. It maintains a persistent `FileSubstrate` and `EpisodeLogger` across the session.

### 3. Pipe Mode

```bash
echo "Fix the typo in README.md" | roko
cat tasks.txt | roko --pipe
```

Reads prompts from stdin, one per line or as a single block. Designed for integration with Unix pipelines, CI systems, and scripting. Output goes to stdout in human-readable format by default, or structured JSON with the `--json` flag.

Pipe mode is implemented in `roko-cli/src/pipe.rs`. The `stdin_is_tty()` function detects whether input is interactive or piped, automatically selecting the appropriate mode.

### 4. Daemon Mode

```bash
roko daemon --start --port 9090
roko daemon --stop
roko daemon --status
```

Runs Roko as a background service. On macOS, it generates and manages a `launchd` plist. On Linux, it generates a `systemd` unit file. The daemon exposes the HTTP API for remote control and monitors configured event sources (file watchers, cron schedules, webhooks) to trigger agent runs automatically.

Daemon mode is implemented in `roko-cli/src/daemon/` with platform-specific submodules (`launchd.rs` for macOS). The daemon state machine tracks: `Stopped`, `Starting`, `Running`, `Stopping`.

### 5. Serve Mode

```bash
roko serve --port 8080
roko serve --bind 0.0.0.0 --port 8080
```

Starts the HTTP API server directly (without daemonizing). This is the mode used for development, cloud deployment, and integration with external systems. The server exposes REST endpoints, WebSocket streaming, and SSE event feeds. See [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for the full API specification.

Serve mode is implemented via `roko-serve` crate, invoked from `roko-cli/src/main.rs`. The `ServerBuilder` pattern allows configuration of auth, CORS, and event sources before binding.

---

## Top-Level Command Groups

The `roko` CLI organizes its subcommands into logical groups. The full command reference is in [01-cli-command-reference.md](./01-cli-command-reference.md). Here is the high-level structure:

| Group | Commands | Purpose |
|---|---|---|
| **Getting Started** | `init`, `run`, `status`, `config wizard`, `explain` | Zero-to-agent in 60 seconds |
| **Scaffolding** | `new domain`, `new gate`, `new scorer`, `new router`, `new policy`, `new substrate`, `new probe`, `new event-source`, `new template` | Generate working boilerplate |
| **Orchestration** | `plan list`, `plan show`, `plan create`, `plan generate`, `plan run` | DAG-based multi-task execution |
| **PRD Lifecycle** | `prd idea`, `prd list`, `prd draft new`, `prd draft promote`, `prd plan`, `prd status`, `prd consolidate` | Idea → draft → plan pipeline |
| **Research** | `research topic`, `research enhance-prd`, `research enhance-plan`, `research enhance-tasks`, `research analyze` | Deep research with citations |
| **Knowledge** | `neuro stats`, `neuro backup`, `neuro restore`, `episode list` | Inspect and manage NeuroStore |
| **Infrastructure** | `daemon --start/--stop`, `serve`, `mesh status`, `provider list`, `provider health` | Background services and networking |
| **Debugging** | `replay`, `inject`, `dashboard`, `repl` | Introspection and interactive control |
| **Deployment** | `deploy`, `worker` | Cloud deployment and worker mode |

### Global Flags

Every subcommand inherits these global flags:

| Flag | Type | Description |
|---|---|---|
| `--config <PATH>` | PathBuf | Override the config file (default: `./roko.toml`) |
| `--role <ROLE>` | String | Set the agent role/persona |
| `--model <MODEL>` | String | Set the model name |
| `--repo <PATH>` | PathBuf | Set the working directory root |
| `--resume <ID>` | String | Resume a previous session |
| `--effort <LEVEL>` | low/medium/high/max | Reasoning effort level |
| `--json` | bool | Emit JSON output |
| `--log-format <FMT>` | text/json | Tracing log format |
| `--quiet` | bool | Suppress non-essential output |
| `--no-replan` | bool | Disable re-planning on gate failures |
| `--headless` | bool | Run as headless daemon |

---

## Zero-to-Agent in 60 Seconds

The CLI is designed so that a developer can go from zero to a running agent in under a minute:

```bash
# In any project directory:
roko init                          # creates .roko/ + roko.toml with detected defaults
roko run "Add error handling to the auth module"   # runs agent with smart defaults
```

`roko init` performs auto-detection:
- **Language** — scans for `Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`
- **Build system** — sets compile/test/lint commands automatically
- **Model** — defaults to `claude-sonnet-4-6` (configurable)
- **Gates** — enables compile + test gates matching the detected language

This auto-detection is implemented in `roko-cli/src/config.rs`. The `load_layered` function resolves configuration from multiple sources in priority order (see [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md)).

### Starter Templates

```bash
roko init --template coding        # optimized for code generation
roko init --template research      # optimized for research (large context)
roko init --template ops           # optimized for operations (event-driven)
roko init --template chain         # blockchain agent (DeFi tools, chain gates)
roko init --template blank         # minimal: just the types, you configure everything
```

Each template generates a tuned `roko.toml` with appropriate gate pipelines, model routing configurations, and prompt roles for the target domain.

### What `roko init` Creates

```
.roko/
├── roko.toml              # configuration (auto-detected)
├── signals.jsonl           # Engram storage (created on first run)
├── learn/
│   ├── episodes.jsonl      # episode records
│   ├── cascade-router.json # model routing state
│   └── playbook.jsonl      # learned rules
├── neuro/
│   └── knowledge.jsonl     # knowledge store
└── state/
    └── executor.json       # resumable execution state
```

---

## Event System

All five CLI modes consume the same event stream. The orchestrator emits `AgentEvent` variants through async channels:

- `WaveStart`, `WaveComplete` — plan wave lifecycle
- `AgentSpawn`, `AgentOutput`, `AgentExit` — agent lifecycle
- `GateStart`, `GatePass`, `GateFail` — verification pipeline
- `PlanPhaseChange` — plan state transitions
- `ConductorIntervention` — circuit breaker or watcher action

The TUI renders these events visually. Headless mode serializes them as JSON lines. Serve mode streams them over SSE or WebSocket. Same events, different consumers.

This unified event architecture means that what you see in the TUI is exactly what gets logged in headless mode and streamed via the API. There is no special rendering path or separate data model.

---

## Design Principles

The CLI follows six design principles from `refactoring-prd/10-developer-guide.md`:

1. **Zero to running in 60 seconds** — `roko init && roko run "fix the bug"` works with no configuration.
2. **Convention over configuration** — sensible defaults for everything. Only configure what you need to change.
3. **Progressive disclosure** — beginners see 3 commands. Experts see the full Synapse trait system. Same tool, different depths.
4. **Generators, not blank files** — `roko new` scaffolds everything with working boilerplate that compiles immediately.
5. **Errors are instructions** — every error message tells you exactly what to do next (see [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md)).
6. **Examples are tests** — every example in the documentation compiles and runs in CI.

### Three Levels of Complexity

| Level | What You Do | What You Get |
|---|---|---|
| **Beginner** | `roko init` + `roko run "prompt"` | Working agent with smart defaults |
| **Intermediate** | Edit `roko.toml` to configure gates, routing, models | Customized agent behavior without code |
| **Advanced** | Implement Synapse traits in Rust, create domain plugins | Full control over every cognitive function |

Most users never need Level 3. The defaults handle 80% of use cases.

---

## Implementation Details

### Binary Architecture

The `roko` binary is defined in `roko-cli/src/main.rs`. It uses `clap::Parser` for argument parsing with a top-level `Cli` struct and a `Command` enum for subcommands. The binary links against `roko-cli` (the library crate) which re-exports key types from:

- `roko-core` — Engram types, Synapse traits, configuration schema
- `roko-agent` — LLM backends, tool dispatch, MCP client
- `roko-compose` — prompt assembly, context engineering
- `roko-gate` — verification pipeline
- `roko-orchestrator` — plan DAG execution
- `roko-conductor` — reactive watchers, circuit breakers
- `roko-learn` — episodes, playbooks, cascade router, C-Factor
- `roko-neuro` — knowledge store
- `roko-daimon` — affect/motivation engine
- `roko-dreams` — offline learning
- `roko-fs` — JSONL substrate persistence
- `roko-serve` — HTTP server

### Module Structure

```
roko-cli/src/
├── main.rs           # Entry point, arg parsing, dispatch
├── lib.rs            # Library surface, re-exports
├── run.rs            # run_once: single-shot cognitive loop
├── orchestrate.rs    # PlanRunner: DAG-based multi-task execution
├── config.rs         # roko.toml loading, layered resolution
├── config_cmd.rs     # config wizard, show, path, edit, set
├── repl.rs           # Interactive REPL
├── pipe.rs           # Stdin pipe mode
├── oneshot.rs        # One-shot mode helpers
├── plan.rs           # Plan parsing and display
├── plan_generate.rs  # Plan generation from PRDs
├── prd.rs            # PRD lifecycle commands
├── prd_prompt.rs     # PRD prompt engineering
├── research.rs       # Research subcommands
├── episode.rs        # Episode listing and inspection
├── inject.rs         # Signal injection
├── status.rs         # Status reporting
├── daemon/           # Background service management
│   └── launchd.rs    # macOS launchd plist generation
├── secrets.rs        # Secret management
├── index.rs          # Code index commands
├── clean.rs          # Cleanup utilities
├── agent_exec.rs     # Agent execution helpers
├── task_parser.rs    # TOML task parsing
├── event_sources.rs  # Event source inspection
├── subscriptions.rs  # Subscription management
├── serve_runtime.rs  # CLI runtime bridge for roko-serve
├── worker/           # Cloud worker mode
│   ├── mod.rs
│   ├── handler.rs
│   └── cloud.rs
└── tui/              # Terminal UI (see 08-tui-main-layout.md)
    ├── mod.rs
    ├── app.rs
    ├── theme.rs
    ├── color.rs
    └── ...
```

---

## Current Status and Gaps

**Built and working (38 tests):**
- All subcommands listed in the command reference
- One-shot, pipe, REPL modes
- Plan orchestration with DAG execution
- PRD lifecycle (idea → draft → plan)
- Research commands
- Status, replay, inject
- Dashboard (text-only mode)
- Serve mode with HTTP API
- Daemon mode (macOS launchd)
- Worker mode for cloud deployment

**Scaffold / in progress:**
- Interactive TUI dashboard (ratatui framework exists, text-only rendering)
- `roko new` scaffolders (not yet implemented — see [02-roko-new-scaffolders.md](./02-roko-new-scaffolders.md))
- `roko explain` command (progressive help system planned)
- Config wizard (interactive config setup planned)

**Not started:**
- `roko mesh status` (requires Agent Mesh — see Tier 5 in `refactoring-prd/07-implementation-priorities.md`)
- Spectre visualization in CLI (optional inline rendering)

---

## Cross-References

- See [01-cli-command-reference.md](./01-cli-command-reference.md) for the full command list with syntax
- See [02-roko-new-scaffolders.md](./02-roko-new-scaffolders.md) for the `roko new` scaffolding system
- See [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md) for the progressive help system
- See [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md) for `roko.toml` resolution
- See [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for the HTTP API exposed by `roko serve`
- See topic [01-orchestration](../01-orchestration/INDEX.md) for plan execution details
- See topic [02-agents](../02-agents/INDEX.md) for agent dispatch and LLM backends
