# CLI Overview — `roko` as Primary Interface

> The `roko` binary is the primary interface to the Roko cognitive agent framework, supporting a default interactive shell, five additional invocation modes, and a first-class plugin surface for discovery, install, audit, and extension management.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Engram, Synapse traits, and the universal cognitive loop
**Key sources**: `refactoring-prd/06-interfaces.md`, `refactoring-prd/10-developer-guide.md`, `roko-cli/src/main.rs`, `roko-cli/src/lib.rs`, `bardo-backup/prd/18-interfaces/01-cli.md`, `bardo-backup/prd/25-mori/mori-interfaces.md`

---

## Abstract

The `roko` binary is a single Rust executable that serves as the canonical entry point for all Roko operations. It is built with `clap` for argument parsing and exposes a rich subcommand tree covering agent lifecycle management, plan orchestration, PRD-driven development, knowledge operations, research, deployment, system introspection, and target-state plugin lifecycle operations. Unlike systems that separate CLI, TUI, and server into different binaries, Roko unifies them: the same binary that runs `roko run "fix the bug"` also hosts `roko dashboard` (the interactive TUI), `roko serve` (the HTTP API), `roko daemon --start` (the background service), and, in the target-state design, `roko plugin install <id>` (the ecosystem entry point).

The CLI is designed around the principle of **progressive disclosure**: beginners see three commands (`roko init`, `roko run`, `roko status`), intermediates configure behavior through `roko.toml` and install ready-made plugins and domain profiles in the target-state design, and advanced users author Tier 4 or Tier 5 extensions against the target-state plugin SPI. This layered approach ensures that Roko is approachable for a developer who just wants to run an agent against their codebase, while providing full architectural control for those building domain-specific cognitive systems. See also [14-plugin-sdk.md](../18-tools/14-plugin-sdk.md), [16-plugin-loading.md](../18-tools/16-plugin-loading.md), [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), [tmp/refinements/17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md), and [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md).

The CLI sits at the **Application layer** — above L4 Orchestration. It consumes crates from every layer: `roko-core` (L0/L1), `roko-compose` (L2), `roko-gate` (L3), `roko-orchestrator` (L4), and the cognitive cross-cuts (`roko-neuro`, `roko-daimon`, `roko-dreams`). The `roko-serve` crate provides the HTTP server that `roko serve` starts.

REF23 reframes the CLI as one rendering of a unified verb set shared by four surfaces: CLI, TUI, Chat, and Web. The CLI keeps the canonical command names, while the other surfaces render the same `ask`, `plan`, `do`, `watch`, `inspect`, `replay`, `learn`, `tune`, and `connect` actions over the same Bus-backed progress stream and the same session state. REF25 extends that same contract to profile selection and profile composition, so the user can install a domain profile once and carry it between surfaces without relearning setup. See [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [14-agent-onboarding-flow.md](./14-agent-onboarding-flow.md), [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md).

REF28 tightens the CLI story further: `roko` with no subcommand should feel familiar to users arriving from Claude Code, Aider, Cursor agent mode, Codex CLI, and similar tools. That means a default interactive entry, workspace detection, slash commands such as `/edit`, diff-first review with per-hunk control, visible budget state, transcripts, resumption, and a non-interactive mode that keeps the same verbs for shell pipelines and CI. Roko still differs where it matters — plan workflow, heuristics, c-factor, multi-agent orchestration, and explainable decisions — but the first hour should feel familiar rather than foreign. See [01-cli-command-reference.md](./01-cli-command-reference.md), [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md).

---

## Unified Verbs on the CLI

REF23's user-facing rule is simple: learn one verb set once, then carry it between the CLI, TUI, Chat, and Web surfaces.

| Unified verb | CLI rendering | Notes |
|---|---|---|
| `ask` | `roko ask <prompt>` or the legacy `roko run <prompt>` path | Single-turn work; `roko run` remains a compatibility-oriented execution noun until the surface names converge. |
| `plan` | `roko plan ...` | Proposal and inspection without committing to execution. |
| `do` | `roko do ...` or `roko plan run ...` | Execute a task or approved plan. |
| `watch` | `roko watch <session-or-episode>` | Render live Bus activity as a continuous progress stream. |
| `inspect` | `roko inspect <episode|engram|heuristic>` | Drill into durable records and supporting evidence. |
| `replay` | `roko replay <episode>` | Re-run a prior episode, optionally with changed inputs. |
| `learn` | `roko learn ...` | Browse heuristics, playbooks, experiments, and calibration state. |
| `tune` | `roko tune ...` or `roko config ...` | Adjust routing, thresholds, permissions, and other operator settings. |
| `connect` | `roko connect ...` or `roko plugin ...` | Add plugins, profile bundles, MCP servers, credentials, and provider links. |

The exact command tree can stay broader than the verb set. What matters is that every high-frequency workflow has a stable canonical verb and every help page teaches the adjacent verb, not an isolated silo.

---

## Familiar Workflow Contract

REF28's operator promise is "Claude Code-shaped where possible, Roko-shaped where necessary." The CLI should therefore preserve the common interaction loop most users already know:

1. Launch `roko` inside a project directory.
2. Detect the workspace and summarize what was found.
3. Open an interactive prompt immediately, without forcing mode selection up front.
4. Accept natural-language requests plus slash commands.
5. Render proposed edits as diff-first output with per-hunk control.
6. Save a transcript and let the operator resume later from the same session chain.
7. Keep every interactive action scriptable through a direct CLI equivalent.

Roko's additions sit on top of that familiar shell instead of replacing it:

| Familiar expectation | Roko rendering |
|---|---|
| Interactive prompt on bare launch | `roko` with no subcommand opens the default session shell |
| Slash command palette | `/edit`, `/run`, `/undo`, `/compact`, `/plan`, `/execute`, `/watch`, `/inspect`, `/explain`, `/heuristics`, `/learn`, `/replay`, `/tune`, `/help`, `/exit` |
| Diff-first review | Proposed edits render as hunks before apply |
| Per-hunk approval | Individual hunks can be accepted, rejected, or edited |
| Session continuity | Transcript plus episode chain plus tool-permission history |
| Budget awareness | Prompt line and structured output include per-turn and per-session budget state |
| Pipeline use | The same verbs support `stdin`, JSON output, semantic exit codes, and replay |

The important architectural rule is parity, not imitation. Familiar workflow affordances ride on top of the same `Bus`, `Substrate`, `StateHub`, heuristics, and gate pipeline that make Roko distinct.

---

## Six CLI Modes

Roko supports six distinct interaction modes through the same binary. The mode is selected by invocation pattern, not by a separate flag:

### 1. Default Interactive Mode

```bash
roko
```

Running `roko` with no subcommand is the familiar-first entry point. It should detect the workspace, surface recent session state, classify the operator's first turn, and propose the right action before doing work:

```text
$ roko
[roko 1.0 — /Users/will/myproject]
Workspace: Rust, cargo test, dirty git tree, 2 failing tests
[3 prior sessions found]
> fix the failing test

Proposed mode: /edit
Plan: inspect failure, patch code, rerun tests.
Proceed? [Y/n]
```

The first-turn classifier routes to `ask`, `/edit`, or `/plan` based on scope. It uses local heuristics plus a soft model check, but the user always sees the chosen mode and can override it. On re-entry, the prompt should offer session resumption from the existing transcript and episode chain rather than pretending every session starts blank.

### 2. One-Shot Mode

```bash
roko run "Add error handling to the auth module"
```

Executes a single prompt through the full universal cognitive loop (query → score → route → compose → act → verify → persist) and exits. This is the simplest and most common usage. The loop runs once: the prompt becomes an Engram of kind `Task`, flows through the Synapse pipeline, dispatches to an LLM backend, verifies with the gate pipeline, and writes results to disk.

One-shot mode is implemented in `roko-cli/src/run.rs` via the `run_once` function. It loads the layered configuration (`roko.toml` → env vars → CLI flags), wires the default trait implementations, and drives a single iteration.

**Exit codes:**
- `0` — success (agent output passed all gates)
- `1` — agent or gate failure (the build failed logically)
- `2` — system error (I/O, config, infrastructure)

### 3. REPL Mode

```bash
roko repl
```

Opens an interactive read-eval-print loop. Each line entered becomes a prompt that runs through the cognitive loop. State persists across prompts — the Substrate accumulates Engrams, the Daimon tracks affect across turns, and Neuro knowledge entries persist. This mode is useful for exploratory development where the operator wants to iterate on prompts and observe how the agent learns across interactions.

The REPL is implemented in `roko-cli/src/repl.rs`. It maintains a persistent `FileSubstrate` and `EpisodeLogger` across the session.

### 4. Pipe Mode

```bash
echo "Fix the typo in README.md" | roko
cat tasks.txt | roko --pipe
```

Reads prompts from stdin, one per line or as a single block. Designed for integration with Unix pipelines, CI systems, and scripting. In piped mode the CLI should suppress TUI chrome, write machine data to stdout, keep progress and diagnostics on stderr, and return semantic exit codes so shell automation can distinguish refusal, gate failure, and budget exhaustion.

Pipe mode is implemented in `roko-cli/src/pipe.rs`. The `stdin_is_tty()` function detects whether input is interactive or piped, automatically selecting the appropriate mode.

### 5. Daemon Mode

```bash
roko daemon --start --port 9090
roko daemon --stop
roko daemon --status
```

Runs Roko as a background service. On macOS, it generates and manages a `launchd` plist. On Linux, it generates a `systemd` unit file. The daemon exposes the HTTP API for remote control and monitors configured event sources (file watchers, cron schedules, webhooks) to trigger agent runs automatically.

Daemon mode is implemented in `roko-cli/src/daemon/` with platform-specific submodules (`launchd.rs` for macOS). The daemon state machine tracks: `Stopped`, `Starting`, `Running`, `Stopping`.

### 6. Serve Mode

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
| **Getting Started** | `roko`, `init`, `run`, `status`, `config wizard`, `explain` | Zero-to-agent in 60 seconds |
| **Plugins** | `plugin list`, `plugin search`, `plugin install`, `plugin enable`, `plugin disable`, `plugin audit` | Discover, install, and govern five-tier extensions |
| **Scaffolding** | `new domain`, `new gate`, `new scorer`, `new router`, `new policy`, `new substrate`, `new probe`, `new event-source`, `new template` | Generate working boilerplate |
| **Orchestration** | `plan list`, `plan show`, `plan create`, `plan generate`, `plan run` | DAG-based multi-task execution |
| **PRD Lifecycle** | `prd idea`, `prd list`, `prd draft new`, `prd draft promote`, `prd plan`, `prd status`, `prd consolidate` | Idea → draft → plan pipeline |
| **Research** | `research topic`, `research enhance-prd`, `research enhance-plan`, `research enhance-tasks`, `research analyze` | Deep research with citations |
| **Knowledge** | `neuro stats`, `neuro backup`, `neuro restore`, `episode list` | Inspect and manage NeuroStore |
| **Infrastructure** | `daemon --start/--stop`, `serve`, `mesh status`, `provider list`, `provider health` | Background services and networking |
| **Debugging** | `replay`, `inject`, `dashboard`, `repl` | Introspection and interactive control |
| **Deployment** | `deploy`, `worker` | Cloud deployment and worker mode |

The long command tree is an implementation detail. The user-facing mental model should stay the REF23 verb set above, with deeper subcommands nested under the verb that best describes user intent.

### Global Flags

Every subcommand inherits these global flags:

| Flag | Type | Description |
|---|---|---|
| `--config <PATH>` | PathBuf | Override the config file (default: `./roko.toml`) |
| `--role <ROLE>` | String | Set the agent role/persona |
| `--model <MODEL>` | String | Set the model name |
| `--budget <AMOUNT>` | String | Cap spend per turn or per session, for example `$0.50` or `$5/session` |
| `--repo <PATH>` | PathBuf | Set the working directory root |
| `--resume <ID>` | String | Resume a previous session |
| `--effort <LEVEL>` | low/medium/high/max | Reasoning effort level |
| `--json` | bool | Emit JSON output |
| `--format <FMT>` | human/json/yaml | Force a specific output renderer |
| `--log-format <FMT>` | text/json | Tracing log format |
| `--quiet` | bool | Suppress non-essential output |
| `--non-interactive` | bool | Disable prompts and require flags to answer approval checkpoints |
| `--record <PATH>` | PathBuf | Write a replayable transcript stream for later `roko replay --assert` |
| `--no-replan` | bool | Disable re-planning on gate failures |
| `--headless` | bool | Run as headless daemon |

---

## Plugin Workflow

REF17 promotes plugins from an implementation detail to a target-state operator workflow. The CLI
assumes a target-state five-tier plugin SPI:

| Tier | What operators install | Discovery root | Typical command path |
|---|---|---|---|
| 1 | Prompt and template bundles | `plugins/prompts/**` | `roko plugin install <id>` |
| 2 | Configuration profiles | `plugins/profiles/**` | `roko plugin install <id>` |
| 3 | Declarative tools and MCP manifests | `plugins/tools/**` | `roko plugin install <id>` then `roko plugin audit` |
| 4 | Native Rust trait implementations | `plugins/native/**` | `roko plugin install <id>` or project-local drop-in |
| 5 | WASM sandboxed extensions | `plugins/wasm/**` | `roko plugin install <id>` with host-enforced limits |

The operator workflow is discovery-first, not config-first. The runtime walks `plugins/**`,
loads manifests, validates permissions, and wires capabilities without requiring a manual
`roko.toml` edit for the common case. Disabling is either a marker file under the plugin
directory or `roko plugin disable <id>`.

The most important ergonomics path is Tier 3: a declarative tool manifest or MCP wrapper that
ships as data, advertises permissions up front, and enters the tool registry through the same
CLI flow as any other plugin. Tier 4 and Tier 5 are reserved for extensions that need more
power than pure manifests can provide.

### Domain Profiles

REF25 treats domain profiles as installable, composable bundles rather than loose config snippets.
The user-facing workflow is:

1. Discover or install the matching profile bundle.
2. Review the declared tools, gates, heuristics, roles, and starter templates.
3. Activate one profile or compose several profiles for a mixed-domain project.
4. Carry the selected `TypedContext` schema and `Custody` expectations into onboarding and later
   inspect/replay flows.

The CLI examples stay familiar:

```bash
roko plugin install @roko/coding-profile
roko plugin install @roko/research-profile
roko init --profile coding
```

When multiple profiles are active, the CLI should surface merge warnings for role or tool
collisions, show which gates will stack, and make it obvious which profile owns the default
template choices. The same profile picker and conflict summary should be available in TUI, Chat,
and Web so the user does not have to relearn the setup flow on another surface.

---

## Zero-to-Agent in 60 Seconds

The CLI is designed so that a developer can go from zero to a running agent in under a minute:

```bash
# In any project directory:
roko init                          # interactive first-run, provider/plugin/MCP checks
roko                               # workspace-aware interactive shell with resume prompt
```

`roko init` performs auto-detection:
- **Language** — scans for `Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`
- **Build system** — sets compile/test/lint commands automatically
- **Model** — defaults to `claude-sonnet-4-6` (configurable)
- **Gates** — enables compile + test gates matching the detected language
- **Profile** — offers the matching domain profile if one is available, or a blank starter if the
  user wants to compose a custom profile set

This auto-detection is implemented in `roko-cli/src/config.rs`. The `load_layered` function resolves configuration from multiple sources in priority order (see [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md)). Plugin discovery stays separate: installed plugins are discovered from `plugins/**`, then optional config layers override only the pieces that need site-specific tuning.

REF23 tightens the first-run bar: `roko init` should be interactive, never dead-end, always offer a skip/configure-later path, and commit partial progress as each step succeeds so a cancelled setup can resume cleanly. REF28 adds that the first useful command should usually be the bare `roko` entry, not a mode-picking subcommand: detect the workspace, show prior transcript history, surface budgets, and let the operator start with a natural-language request immediately. The onboarding details and failure-recovery prompts live in [14-agent-onboarding-flow.md](./14-agent-onboarding-flow.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), and [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md).

### Starter Templates

```bash
roko init --template coding        # optimized for code generation
roko init --template research      # optimized for research (large context)
roko init --template ops           # optimized for operations (event-driven)
roko init --template chain         # blockchain agent (DeFi tools, chain gates)
roko init --template blank         # minimal: just the types, you configure everything
```

Each template generates a tuned `roko.toml` with appropriate gate pipelines, model routing configurations, and prompt roles for the target domain. In the REF25 framing, templates are the profile-local defaults that a profile bundle can supply, override, or compose.

### What `roko init` Creates

```
.roko/
├── roko.toml              # configuration (auto-detected)
├── signals.jsonl           # Engram storage (created on first run)
├── transcripts/            # human-readable and JSONL session transcripts
│   └── latest.jsonl
├── learn/
│   ├── episodes.jsonl      # episode records
│   ├── cascade-router.json # model routing state
│   └── playbook.jsonl      # learned rules
├── sessions/
│   └── last-session.json   # resumption cursor, budget state, tool approvals
├── neuro/
│   └── knowledge.jsonl     # knowledge store
└── state/
    └── executor.json       # resumable execution state
```

The transcript is not just a chat log. It should preserve the episode chain, approval checkpoints, diff-first review choices, and budget state that let a later session resume with continuity instead of replaying blind.

---

## Review, Approval, and Resumption

The default interactive shell should be diff-first. Proposed edits render as hunks, not rewritten files, and the operator can accept all, accept a subset, open `/edit`, or ask `/explain` for the heuristic and claim citations behind the proposal.

Per-hunk control is load-bearing because it turns user feedback into structured signal:

- accepted hunks reinforce the path that led there
- rejected hunks become negative evidence for replay and later heuristic tuning
- edited hunks preserve the operator's correction inside the transcript

That same transcript powers later resumption. A resumed session should restore:

- the last approved or pending diff-first review state
- recent `Bus` progress and `StateHub` projections
- tool approval memory scoped to the session
- active budget totals and remaining allowance
- the durable episode chain and any heuristics learned during the run

---

## Event System

All six CLI modes consume the same event stream. The orchestrator emits `AgentEvent` variants through async channels:

- `WaveStart`, `WaveComplete` — plan wave lifecycle
- `AgentSpawn`, `AgentOutput`, `AgentExit` — agent lifecycle
- `GateStart`, `GatePass`, `GateFail` — verification pipeline
- `PlanPhaseChange` — plan state transitions
- `ConductorIntervention` — circuit breaker or watcher action

The TUI renders these events visually. Headless mode serializes them as JSON lines. Serve mode streams them over SSE or WebSocket. Same events, different consumers.

This unified event architecture means that what you see in the TUI is exactly what gets logged in headless mode and streamed via the API. There is no special rendering path or separate data model.

Under REF23, this stream is the source of truth for live progress across all four surfaces. `watch` is therefore not a CLI-only feature; it is the CLI rendering of the same token streaming, tool banners, gate feedback, and episode events that the TUI, Chat, and Web surfaces render differently.

---

## Design Principles

The CLI follows seven design principles from `refactoring-prd/10-developer-guide.md` plus the familiar-workflow contract from REF28:

1. **Zero to running in 60 seconds** — `roko init && roko` works with no configuration.
2. **Convention over configuration** — sensible defaults for everything. Only configure what you need to change.
3. **Progressive disclosure** — beginners see 3 commands. Experts see the full Synapse trait system. Same tool, different depths.
4. **Generators, not blank files** — `roko new` scaffolds everything with working boilerplate that compiles immediately.
5. **Errors are instructions** — every error message tells you exactly what to do next (see [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md)).
6. **Familiar muscle memory first** — slash commands, transcripts, diff-first review, and tab completion should work the way experienced agent-tool users expect.
7. **Examples are tests** — every example in the documentation compiles and runs in CI.

### Three Levels of Complexity

| Level | What You Do | What You Get |
|---|---|---|
| **Beginner** | `roko init` + `roko` | Working agent with smart defaults and an interactive session |
| **Intermediate** | Edit `roko.toml`, install Tier 1-3 plugins | Customized behavior without writing Rust |
| **Advanced** | Author Tier 4 native or Tier 5 WASM plugins | Full control over every operator and fabric integration |

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
- Established one-shot, orchestration, research, infrastructure, and debugging subcommands
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
- Default `roko` interactive shell with first-turn intent detection
- Slash commands, diff-first review, and per-hunk approval flows
- Transcript-backed resumption, visible budget line, and completion refresh
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
- See [21-user-ux-running-agents.md](./21-user-ux-running-agents.md) for the shared four-surface interaction contract
- See topic [01-orchestration](../01-orchestration/INDEX.md) for plan execution details
- See topic [02-agents](../02-agents/INDEX.md) for agent dispatch and LLM backends
- See [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md) for the canonical familiar-workflows proposal
