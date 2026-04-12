# CLI Command Reference

> Full command list for the `roko` binary with per-command syntax, flags, and descriptions.

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md) for mode overview and design principles
**Key sources**: `refactoring-prd/06-interfaces.md` §1, `roko-cli/src/main.rs`, `bardo-backup/prd/25-mori/mori-interfaces.md`

---

## Abstract

This document is the canonical command reference for the `roko` CLI. Every subcommand, flag, and argument is listed with its type, default value, and purpose. Commands are organized by functional group. The reference reflects both the current implementation in `roko-cli/src/main.rs` and the target specification from `refactoring-prd/06-interfaces.md`. Where the spec describes commands not yet implemented, they are marked as such.

The `roko` binary uses `clap` for parsing and supports both positional arguments and named flags. All subcommands inherit the global flags described in [00-cli-overview.md](./00-cli-overview.md).

---

## Getting Started

### `roko init`

Scaffold a new Roko project by creating `.roko/` and a default `roko.toml`.

```
roko init [PATH] [--cloud] [--template T]
```

| Argument/Flag | Type | Default | Description |
|---|---|---|---|
| `PATH` | PathBuf | Current directory | Directory to initialize |
| `--cloud` | bool | false | Generate cloud-ready defaults for deployment |
| `--template` | String | (auto-detect) | Template: `coding`, `research`, `ops`, `chain`, `blank` |

Auto-detects language, build system, and gates from project files (`Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`). Creates the `.roko/` directory tree with `roko.toml`, signal storage, learning state, and knowledge store.

### `roko run`

Execute a single prompt through the universal cognitive loop.

```
roko run <PROMPT> [--workdir PATH]
```

| Argument/Flag | Type | Default | Description |
|---|---|---|---|
| `PROMPT` | String | (required) | The user prompt text |
| `--workdir` | PathBuf | Current directory | Override working directory |

Runs: query → score → route → compose → act → verify → persist. Returns exit code 0 on success, 1 on agent/gate failure, 2 on system error. Respects `--json` for structured output and `--effort` for reasoning depth.

### `roko status`

Print Engram counts, most recent episode, gate pass/fail rates, and optionally compute C-Factor.

```
roko status [--workdir PATH] [--cfactor]
```

| Flag | Type | Default | Description |
|---|---|---|---|
| `--workdir` | PathBuf | Current directory | Directory containing `.roko/` |
| `--cfactor` | bool | false | Compute and persist latest C-Factor snapshot |

Reads from `.roko/signals.jsonl`, `.roko/learn/episodes.jsonl`, and `.roko/learn/efficiency.jsonl`. With `--json`, outputs structured JSON suitable for dashboards and CI.

### `roko config`

Manage global and project configuration.

```
roko config <SUBCOMMAND>
```

| Subcommand | Description |
|---|---|
| `config wizard` | Interactive config setup — walks through model selection, gate configuration, and provider setup |
| `config show` | Display resolved configuration (all layers merged) |
| `config path` | Print the path to the active config file |
| `config edit [--target global\|project]` | Open config in `$EDITOR` |
| `config set <KEY> <VALUE>` | Set a single config value |
| `config set-secret <KEY> <VALUE>` | Set a secret value (stored encrypted) |
| `config init` | Create a default config file |

### `roko explain`

Learn about Roko concepts. Progressive disclosure — starts simple, adds detail on request.

```
roko explain <TOPIC>
```

| Topic | Description |
|---|---|
| `gates` | How the verification pipeline works |
| `routing` | How model routing and the cascade work |
| `cognitive` | The universal cognitive loop and three speeds |
| `neuro` | Knowledge management and tier progression |
| `daimon` | The affect engine and behavioral states |
| `dreams` | Offline consolidation and hypnagogia |
| `engram` | The Engram data type and Synapse traits |
| `cfactor` | Collective intelligence metrics |

**Status**: Not yet implemented. Planned for Tier 4 (interfaces).

---

## Scaffolding

### `roko new`

Generate working boilerplate for Synapse trait implementations and domain components. Every scaffold compiles immediately with working tests.

```
roko new <TYPE> <NAME>
```

| Type | What it scaffolds |
|---|---|
| `domain <name>` | Complete domain plugin (tools, gates, probes, templates) |
| `gate <name>` | Custom Gate implementation with test harness |
| `scorer <name>` | Custom Scorer with composite integration |
| `router <name>` | Custom Router with feedback method |
| `policy <name>` | Custom Policy with stream processing |
| `substrate <name>` | Custom Substrate (persistence backend) |
| `probe <name>` | T0 Probe (zero-LLM deterministic check) |
| `event-source <name>` | EventSource plugin (webhook, cron, file watch) |
| `template <name>` | Agent template with system prompt and config |

Each scaffold generates:
- A Rust file implementing the relevant Synapse trait
- A test file with basic passing tests
- A `Cargo.toml` entry if creating a new crate
- A README explaining the generated code

See [02-roko-new-scaffolders.md](./02-roko-new-scaffolders.md) for detailed examples.

**Status**: Not yet implemented. Planned for Tier 4 (interfaces).

---

## Orchestration

### `roko plan`

Manage execution plans — the DAG-based task structures that drive multi-step agent work.

```
roko plan <SUBCOMMAND>
```

| Subcommand | Syntax | Description |
|---|---|---|
| `list` | `roko plan list` | List all discovered plans |
| `show <ID>` | `roko plan show 01` | Show plan details (tasks, DAG, status) |
| `create` | `roko plan create` | Create a new plan interactively |
| `generate <PRD>` | `roko plan generate system-prompt-wiring` | Generate plan from a PRD |
| `run <DIR>` | `roko plan run plans/ [--resume FILE]` | Execute plans via DAG executor |
| `validate <DIR>` | `roko plan validate plans/` | Parse plans, print DAG and parallelism stats |

The `plan run` command is the main orchestration loop. It discovers TOML task files in the specified directory, builds a dependency DAG, groups tasks into parallelizable waves, and dispatches agents for each task. The `--resume` flag allows restarting from a saved executor snapshot at `.roko/state/executor.json`.

Plan selection supports ranges and individual specs: `01`, `03-07`, `01,03,08`.

### `roko orchestrate`

Alias for `roko plan run`.

```
roko orchestrate <PLAN>
```

---

## PRD Lifecycle

### `roko prd`

Manage product requirements documents — the idea-to-implementation pipeline.

```
roko prd <SUBCOMMAND>
```

| Subcommand | Syntax | Description |
|---|---|---|
| `idea <TEXT>` | `roko prd idea "Wire SystemPromptBuilder"` | Capture a quick work item idea |
| `list` | `roko prd list` | List all PRDs with status |
| `status` | `roko prd status` | Coverage report (plans/tasks/done ratio) |
| `draft new <TITLE>` | `roko prd draft new "system-prompt-wiring"` | Create PRD draft (agent-assisted) |
| `draft promote <SLUG>` | `roko prd draft promote system-prompt-wiring` | Promote draft to published |
| `plan <SLUG>` | `roko prd plan system-prompt-wiring` | Generate implementation plan from PRD |
| `consolidate` | `roko prd consolidate` | Consolidate PRDs |

PRDs flow through a lifecycle: `idea` → `draft` → `published` → `plan`. Each transition can be agent-assisted. The `prd plan` command invokes an agent to read the PRD and generate a `tasks.toml` file with dependencies, priorities, and estimated effort.

---

## Research

### `roko research`

Agent-driven deep research with citations and source tracking.

```
roko research <SUBCOMMAND>
```

| Subcommand | Syntax | Description |
|---|---|---|
| `topic <TOPIC>` | `roko research topic "attention mechanisms"` | Deep research with citations |
| `enhance-prd <SLUG>` | `roko research enhance-prd system-prompt-wiring` | Enhance PRD with research findings |
| `enhance-plan <PLAN>` | `roko research enhance-plan plans/01` | Optimize plan with research |
| `enhance-tasks <PLAN>` | `roko research enhance-tasks plans/01` | Split/optimize tasks |
| `analyze` | `roko research analyze` | Analyze execution data for patterns |

Research artifacts are stored in `.roko/research/` and can be referenced by subsequent plan generation and task execution.

---

## Knowledge and Learning

### `roko neuro`

Query and manage the NeuroStore — the persistent knowledge system.

```
roko neuro <SUBCOMMAND>
```

| Subcommand | Syntax | Description |
|---|---|---|
| `stats` | `roko neuro stats` | Knowledge store statistics (tier distribution, HDC stats) |
| `backup <AGENT>` | `roko neuro backup rust-implementer` | Export agent knowledge to file |
| `restore <FILE>` | `roko neuro restore backup.jsonl` | Import knowledge (selective) |
| `search <QUERY>` | `roko neuro search "error handling patterns"` | Search by HDC similarity |
| `gc` | `roko neuro gc` | Garbage collect expired entries |

### `roko episode`

Inspect recorded episodes — the structured records of agent turns and outcomes.

```
roko episode <SUBCOMMAND>
```

| Subcommand | Description |
|---|---|
| `list` | Recent episodes with outcomes, models, and costs |
| `show <ID>` | Detailed episode view |

---

## Infrastructure

### `roko daemon`

Manage the background service.

```
roko daemon <SUBCOMMAND>
```

| Subcommand | Flags | Description |
|---|---|---|
| `start` | `--foreground`, `--port 9090` | Start the daemon (launchd on macOS) |
| `stop` | | Stop the running daemon |
| `status` | | Check daemon state |
| `logs` | | Stream daemon logs |

### `roko serve`

Start the HTTP API server directly (without daemonizing).

```
roko serve [--bind ADDR] [--port PORT] [--workdir PATH]
```

| Flag | Type | Default | Description |
|---|---|---|---|
| `--bind` | String | `127.0.0.1` | Address to bind to |
| `--port` | u16 | `9090` | Port number |
| `--workdir` | PathBuf | Current directory | Working directory |

See [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for the full API specification.

### `roko mesh status`

Display Agent Mesh connectivity — connected peers, active pheromones, sync state.

```
roko mesh status
```

**Status**: Not yet implemented. Requires Agent Mesh (Tier 5).

### `roko provider`

Manage LLM provider configuration.

```
roko provider <SUBCOMMAND>
```

| Subcommand | Description |
|---|---|
| `list` | List configured LLM providers |
| `health` | Provider health and circuit breaker state |
| `test <ID>` | Test connectivity to a provider |

---

## Debugging

### `roko replay`

Walk the Engram lineage DAG rooted at a content hash.

```
roko replay <HASH> [--workdir PATH]
```

Traverses the `lineage` field of the specified Engram, printing the full ancestry chain. Useful for understanding how a particular output was derived.

### `roko inject`

Inject an Engram into a running session for testing or steering.

```
roko inject <SESSION> <PAYLOAD> [--kind directive|abort|context] [--workdir PATH]
```

| Argument | Type | Default | Description |
|---|---|---|---|
| `SESSION` | String | (required) | Target session ID |
| `PAYLOAD` | String | (required) | Payload text |
| `--kind` | String | `directive` | Kind of Engram to inject |

### `roko dashboard`

Launch the TUI dashboard with text fallback.

```
roko dashboard [--page SLUG] [--list-pages] [--text] [--workdir PATH]
```

| Flag | Description |
|---|---|
| `--page SLUG` | Render a specific dashboard page |
| `--list-pages` | List available page slugs |
| `--text` | Force text-mode (no interactive TUI) |

See [08-tui-main-layout.md](./08-tui-main-layout.md) for the TUI specification.

### `roko repl`

Open the interactive REPL.

```
roko repl
```

### `roko dream`

Manage dream replay and scheduling.

```
roko dream <SUBCOMMAND>
```

| Subcommand | Description |
|---|---|
| `replay` | Run dream replay manually |
| `report` | Show dream consolidation report |
| `schedule` | Configure dream scheduling |

---

## Deployment

### `roko deploy`

Manage cloud deployment targets.

```
roko deploy <SUBCOMMAND>
```

Deployment is handled through the `roko-serve` deployment backend abstraction, supporting Railway and manual deployment modes.

### `roko worker`

Run as a deployed worker — reads template from environment, serves tasks.

```
roko worker [--port 8080]
```

The worker mode is designed for cloud deployment where Roko instances receive work assignments from a central coordinator.

---

## Output Modes

All commands support two output modes:

| Mode | Flag | Format |
|---|---|---|
| Human-readable | (default) | Formatted text with colors and tables |
| Structured | `--json` | JSON lines suitable for `jq`, dashboards, CI |

The `--quiet` flag suppresses non-essential output (progress indicators, banners) while preserving result output.

---

## Academic Foundations

- Karpathy (2025) on context engineering — informs the progressive disclosure and `explain` system design

---

## Current Status and Gaps

All subcommands in the "Orchestration", "PRD Lifecycle", "Research", "Knowledge", "Infrastructure", and "Debugging" groups are implemented and functional. The "Getting Started" group is mostly implemented except for `roko explain` and `roko config wizard`. The "Scaffolding" group (`roko new`) is not yet implemented.

---

## Cross-references

- See [00-cli-overview.md](./00-cli-overview.md) for mode architecture and design principles
- See [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for the REST/WebSocket API
- See topic [01-orchestration](../01-orchestration/INDEX.md) for plan execution internals
- See topic [18-tools](../18-tools/INDEX.md) for MCP server integration
