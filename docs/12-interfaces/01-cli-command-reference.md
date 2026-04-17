# CLI Command Reference

> Full command list for the `roko` binary with per-command syntax, flags, and descriptions.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md) for mode overview and design principles
**Key sources**: `refactoring-prd/06-interfaces.md` §1, `roko-cli/src/main.rs`, `bardo-backup/prd/25-mori/mori-interfaces.md`

---

## Abstract

This document is the canonical command reference for the `roko` CLI. Every subcommand, flag, and argument is listed with its type, default value, and purpose. Commands are organized by functional group. The reference reflects both the current implementation in `roko-cli/src/main.rs` and the target specification from `refactoring-prd/06-interfaces.md`. Where the spec describes commands not yet implemented, they are marked as such.

REF17 extends this surface with a dedicated plugin command family so operators can discover,
install, enable, disable, and audit extensions without manually editing `roko.toml`. The
underlying plugin SPI is described in [14-plugin-sdk.md](../18-tools/14-plugin-sdk.md) and
[16-plugin-loading.md](../18-tools/16-plugin-loading.md). See also
[01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) and
[tmp/refinements/17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md).
REF25 extends the same CLI surface with domain profile install and composition; see
[tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md).

The `roko` binary uses `clap` for parsing and supports both positional arguments and named flags. All subcommands inherit the global flags described in [00-cli-overview.md](./00-cli-overview.md).

REF23 adds a unified user verb set across CLI, TUI, Chat, and Web. The CLI remains the canonical naming surface for those verbs, even when some command families keep compatibility aliases or broader subcommand trees. See [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [14-agent-onboarding-flow.md](./14-agent-onboarding-flow.md), [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md).

REF28 adds the familiar-workflow contract on top of that verb set: bare `roko` should open the default interactive shell, slash commands should exist wherever the semantics match, edits should be reviewed as diff-first hunks, transcripts should preserve approvals and replay state, and every interactive action should have a direct non-interactive CLI equivalent for pipes and CI. See [00-cli-overview.md](./00-cli-overview.md), [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md).

---

## Canonical User Verbs

These verbs are the user-facing contract that should survive across surfaces and future command-tree cleanup:

| Verb | CLI shape | Status |
|---|---|---|
| `ask` | `roko ask <prompt>` | Target-state canonical single-turn noun; `roko run` remains a compatibility path. |
| `plan` | `roko plan ...` | Current command family. |
| `do` | `roko do ...` or `roko plan run ...` | Target-state execution noun over task or plan work. |
| `watch` | `roko watch <session|episode>` | Target-state live-progress surface over the shared event stream. |
| `inspect` | `roko inspect <episode|engram|heuristic>` | Target-state drill-down verb for durable artifacts. |
| `replay` | `roko replay <episode>` | Current debugging verb; promoted to first-class user verb. |
| `learn` | `roko learn ...` | Target-state surface for heuristics, playbooks, and experiments. |
| `tune` | `roko tune ...` or `roko config ...` | Target-state configuration noun; `config` remains the detailed subtree. |
| `connect` | `roko connect ...` or `roko plugin ...` | Target-state add/integrate noun for providers, MCP servers, profile bundles, and plugins. |

---

## Getting Started

### `roko`

Open the default interactive shell.

```
roko [--resume SESSION] [--budget LIMIT] [--format human|json]
```

When invoked without a subcommand, `roko` should:

- detect the workspace and summarize language, test runner, and VCS state
- offer transcript-based resumption if prior sessions exist
- classify the first message as `ask`, `/edit`, or `/plan`
- show the classification before running so the user can override it
- keep the same budget and approval contract as the non-interactive verbs

This is the target-state primary entry point for familiar-first usage. The rest of the command tree remains available for explicit and scriptable control.

### `roko init`

Interactively scaffold a new Roko project by creating `.roko/`, a default `roko.toml`, and the first session-safe runtime state. The same flow can activate or install a domain profile before the first task runs.

```
roko init [PATH] [--cloud] [--template T] [--profile P]
```

| Argument/Flag | Type | Default | Description |
|---|---|---|---|
| `PATH` | PathBuf | Current directory | Directory to initialize |
| `--cloud` | bool | false | Generate cloud-ready defaults for deployment |
| `--template` | String | (auto-detect) | Template: `coding`, `research`, `ops`, `chain`, `blank` |
| `--profile` | String | (auto-detect) | Domain profile to install or activate: `coding`, `research`, `blockchain`, `data`, `ops`, `writing`, or `blank` |

Auto-detects language, build system, gates, and matching profile options from project files (`Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`). Creates the `.roko/` directory tree with `roko.toml`, signal storage, learning state, and knowledge store.

REF23 raises the bar for `roko init`: provider checks, MCP autodiscovery, heuristic-starter import, and secret collection should all degrade gracefully with explicit `skip` or `configure later` exits. REF25 adds profile install and composition to that same contract. Partial success is valid; setup state should be persisted incrementally so the next `roko init` can resume instead of restarting.

### `roko ask`

Single-turn query on the unified verb set.

```
roko ask <PROMPT> [--stream] [--save] [--context PATH] [--budget LIMIT] [--format human|json]
```

| Argument/Flag | Type | Default | Description |
|---|---|---|---|
| `PROMPT` | String | (required) | The user prompt text |
| `--stream` | bool | true in TTY | Stream tokens and tool banners as they arrive |
| `--save` | bool | false | Persist the turn as an episode and retain session continuity |
| `--context <PATH>` | PathBuf | none | Add file or directory context for this turn |
| `--budget <LIMIT>` | String | config default | Spend ceiling for this turn or inherited session |
| `--format` | enum | `human` in TTY | Force `human` or `json` output regardless of TTY detection |

`roko ask` is the canonical REF23 spelling for "run one thing now." Existing `roko run` flows can map here until the command surface is consolidated. In piped mode, `roko ask --format json` is the stable machine-facing form.

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

**REF23 note:** keep `roko run` as a compatibility-friendly execution noun, but teach `roko ask` in help, onboarding, and related-command output. Under REF28, `roko` with no subcommand is the preferred interactive shell while `roko run` remains the explicit one-shot path.

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

Status output should also surface the currently active session budget and the location of the latest transcript when either exists, so operators can understand both cost posture and resumability at a glance.

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

### `roko plugin`

Discover, install, and govern five-tier plugins.

```
roko plugin <SUBCOMMAND>
```

| Subcommand | Syntax | Description |
|---|---|---|
| `list` | `roko plugin list` | List installed plugins discovered from `plugins/**` with tier, state, and health |
| `search <QUERY>` | `roko plugin search kubernetes` | Search a registry or GitHub-backed source for installable plugins |
| `install <ID>` | `roko plugin install cargo.udeps` | Install a plugin into `./plugins` without editing `roko.toml` |
| `uninstall <ID>` | `roko plugin uninstall cargo.udeps` | Remove an installed plugin |
| `enable <ID>` | `roko plugin enable cargo.udeps` | Re-enable a previously disabled plugin |
| `disable <ID>` | `roko plugin disable cargo.udeps` | Disable a plugin via loader state rather than deleting files |
| `info <ID>` | `roko plugin info cargo.udeps` | Show manifest, permissions, capabilities, tier, and health status |
| `audit` | `roko plugin audit` | Review requested permissions, sandbox class, and risky access patterns |

The common path is discovery-first: install a manifest-backed plugin into `plugins/**`, let the
runtime validate it, then use `roko plugin audit` before enabling it in production. Tier 1 and
Tier 2 plugins are pure data; Tier 3 plugins declare tool or MCP behavior; Tier 4 plugins bind
native Rust implementations; Tier 5 plugins run inside the WASM host.

### `roko watch`

Attach to a running session or episode and stream live progress.

```
roko watch <SESSION_OR_EPISODE> [--format human|json|yaml] [--resume-from SEQ]
```

| Argument/Flag | Type | Default | Description |
|---|---|---|---|
| `SESSION_OR_EPISODE` | String | (required) | Session name, session ID, or episode ID |
| `--format` | enum | `human` | Human-first stream or structured output |
| `--resume-from` | u64 | latest | Resume from a Bus sequence number or cursor |

`watch` renders the same shared progress stream used by TUI, Chat, and Web: token streaming, tool call banners, gate feedback, and episode/heuristic events.

### `roko inspect`

Inspect a durable record or session artifact.

```
roko inspect <episode|engram|heuristic|session> [ID]
```

Use this verb for drill-down workflows that would otherwise be scattered across `episode`, `neuro`, and diagnostics subtrees.

### `roko tune`

Adjust configuration, thresholds, routing, and permissions.

```
roko tune <SUBCOMMAND>
```

`tune` is the user-facing REF23 verb. The detailed implementation surface can remain under `roko config` while the verbs converge.

### `roko connect`

Add a plugin, profile bundle, MCP server, provider, or credential source.

```
roko connect <SUBCOMMAND>
```

`connect` is the unified integration verb. In current docs, it maps most directly to `roko plugin`, provider setup, and MCP-related configuration.

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

## Slash Commands and Familiar Controls

The interactive shell, TUI chat pane, and browser chat surface should all accept slash commands where the semantics are identical. Every slash command has a direct CLI equivalent so workflows stay scriptable:

| Slash command | Meaning | Direct CLI equivalent |
|---|---|---|
| `/edit <file>` | Ask the agent to focus on one file and propose an edit | `roko ask --context <file> ...` or bare `roko` routed into edit mode |
| `/run <cmd>` | Execute a shell command through the approval model | `roko do --cmd "<cmd>"` or the current command-runner equivalent |
| `/undo` | Revert the last applied action | `roko revert last` or `roko undo last` |
| `/compact` | Compact the active conversation context | `roko ask --compact ...` once exposed as a direct flag |
| `/plan` | Convert the current thread into a plan | `roko plan create` |
| `/execute` | Execute the current plan | `roko plan run <PLAN>` |
| `/watch` | Attach to the live progress stream | `roko watch <SESSION_OR_EPISODE>` |
| `/inspect <id>` | Drill into an Engram, heuristic, episode, or session | `roko inspect <kind> <id>` |
| `/explain` | Show why the agent made a choice | `roko explain <topic>` and `roko inspect` on the latest episode |
| `/heuristics` | Browse active heuristics | `roko learn heuristics` or `roko inspect heuristic <id>` |
| `/learn` | Promote the exchange into a heuristic or playbook candidate | `roko learn import-session` or equivalent learning verb |
| `/replay <episode>` | Re-run recorded work | `roko replay <episode>` |
| `/tune <key>` | Adjust configuration or thresholds | `roko tune <SUBCOMMAND>` or `roko config set <KEY> <VALUE>` |
| `/help` | Show help | `roko --help` or `roko explain <topic>` |
| `/exit` | Leave the interactive shell | terminate the current interactive session |

Slash command support is an affordance, not a second action model. The verbs remain the same whether they are typed with a leading `/` or passed directly as CLI commands.

### Diff-First and Per-Hunk Review

Interactive edit proposals should render as diff-first output:

```text
Proposed 3 hunks:
  [1/3] src/core.rs: add lowercase normalization
  [2/3] src/core.rs: add empty-check
  [3/3] tests/core.rs: add regression test

Apply: [a]ll, [1,2] subset, [n]one, [e]dit, [x] explain >
```

That review state belongs in the transcript so later replay and heuristic learning can tell which hunks were accepted, rejected, or edited by the operator.

### Completion

Shell completion should enumerate both the command tree and dynamic workspace objects:

```
roko completion <bash|zsh|fish>
```

Target-state completion coverage includes:

- subcommand names after `roko <TAB>`
- plan IDs after `roko plan show <TAB>`
- Engram hashes and episode IDs after `roko inspect <TAB>`
- role names after `roko ask --role <TAB>`
- config keys after `roko config set <TAB>`

Completion generation should be backed by `clap` and refreshed by `roko init` when workspace-local identifiers change.

---

## Scaffolding

### `roko new`

Generate working boilerplate for Synapse trait implementations and domain components. Every scaffold compiles immediately with working tests.

```
roko new <TYPE> <NAME>
```

| Type | What it scaffolds |
|---|---|
| `domain <name>` | Complete domain profile bundle (tools, gates, probes, templates, heuristics) |
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

The scaffolding story now aligns to the plugin SPI as well: prompt bundles and profile bundles
cover the low-power tiers, while native crates and WASM modules cover the high-power tiers.
That keeps the authoring path consistent with `roko plugin install` and `roko plugin audit`.

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
| `run <DIR>` | `roko plan run plans/ [--resume FILE] [--non-interactive] [--fail-on-gate-violation]` | Execute plans via DAG executor |
| `validate <DIR>` | `roko plan validate plans/` | Parse plans, print DAG and parallelism stats |

The `plan run` command is the main orchestration loop. It discovers TOML task files in the specified directory, builds a dependency DAG, groups tasks into parallelizable waves, and dispatches agents for each task. The `--resume` flag allows restarting from a saved executor snapshot at `.roko/state/executor.json`. In CI or shell automation, `--non-interactive` disables prompts, `--fail-on-gate-violation` turns harness failures into process failure, and explicit approval flags replace interactive checkpoints.

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
| `show <ID>` | Detailed episode view including transcript, approvals, and diff review decisions |

### `roko import`

Import prior transcripts or logs from another tool into Roko session state.

```
roko import --from <claude-code|aider|cursor> <PATH>
```

Importers should normalize prior conversation history into Roko transcripts and episodes, attach starting demurrage to imported durable records, and preserve enough metadata that replay and inspect stay meaningful after migration.

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
roko replay <HASH|SESSION_FILE> [--workdir PATH] [--modify TEXT] [--assert]
```

`roko replay` serves two related workflows:

- lineage replay for a durable Engram or episode
- transcript replay for a recorded session stream

`--modify` reapplies the prior flow with one changed operator instruction. `--assert` turns replay into a regression check suitable for CI by failing when the recorded expectations and the new run diverge beyond the configured tolerance.

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

In non-interactive and piped usage, the output contract should be:

- data on stdout
- progress, approvals, and diagnostics on stderr
- no TUI chrome unless `--interactive` is forced
- semantic exit codes: `0` success, `1` refusal, `2` gate failure, `3` budget exhausted, `4` config or invocation error

That lets the same verbs power shell pipelines, CI checks, and replay assertions without inventing a second automation surface.

---

## Academic Foundations

- Karpathy (2025) on context engineering — informs the progressive disclosure and `explain` system design

---

## Current Status and Gaps

Much of the current command tree is implemented, especially the established orchestration, research, infrastructure, and debugging families. REF28 extends this reference beyond the currently shipping surface, so treat the following as target-state unless noted otherwise:

- bare `roko` as the default interactive shell with intent detection
- slash command parity and diff-first per-hunk review
- transcript importers (`roko import --from ...`)
- completion refresh and some dynamic completion sources
- transcript replay assertions and some non-interactive approval flags

`roko explain`, `roko config wizard`, and `roko new` remain explicitly scaffold-stage. The plugin command family is part of the target CLI surface for the extension architecture and depends on the five-tier SPI described in topic 18.

---

## Cross-References

- See [00-cli-overview.md](./00-cli-overview.md) for mode architecture and design principles
- See [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md) for `/explain`, error recovery, and teaching-style diagnostics
- See [21-user-ux-running-agents.md](./21-user-ux-running-agents.md) for the four-surface interaction contract
- See [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for the REST/WebSocket API
- See topic [01-orchestration](../01-orchestration/INDEX.md) for plan execution internals
- See topic [18-tools](../18-tools/INDEX.md) for MCP server integration
- See [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md) for the canonical CLI parity proposal
