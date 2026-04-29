# Roko CLI Reference

Complete reference for the `roko` command-line interface. All flags marked `[global]`
are available on every subcommand.

---

## Global Flags

These flags apply to every command and must be placed before the subcommand.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--config <path>` | path | `./roko.toml` | Override the config file location |
| `--role <string>` | string | from config | Set the agent role / persona |
| `--model <string>` | string | from config | Override the model name for this invocation |
| `--repo <path>` | path | cwd | Set the repository / working directory root |
| `--resume <id>` | string | — | Resume a previous session by ID |
| `--effort low\|medium\|high\|max` | enum | from config | Reasoning effort level |
| `--json` | flag | false | Emit JSON output instead of human-readable text |
| `--log-format text\|json` | enum | `text` | Tracing log format |
| `--quiet` | flag | false | Suppress non-essential output |
| `--no-replan` | flag | false | Disable re-planning; gate failures become terminal |
| `--headless` | flag | false | Run as a headless daemon (background service) |
| `--color auto\|always\|never` | enum | `auto` | Control ANSI color output |
| `--timing` | flag | false | Print elapsed time after command execution |
| `--no-serve` | flag | false | Do not start the HTTP control plane in the background |

### Color resolution (auto mode)

In `auto` mode, color is determined in this order (highest priority first):

1. `NO_COLOR` set and non-empty → off
2. `CLICOLOR_FORCE` set and not `"0"` → on
3. `CLICOLOR=0` → off
4. stdout is a TTY → on
5. otherwise → off

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Agent or gate failure (logical error in the build) |
| `2` | System error (I/O, config, infrastructure) |

### Key environment variables

| Variable | Effect |
|---|---|
| `RUST_LOG` / `ROKO_LOG` | Tracing log directive (e.g. `roko=debug`) |
| `ROKO_TIMING=1` | Enable timing output (same as `--timing`) |
| `ROKO_LOG_RAW=1` | Disable secret redaction in logs |
| `ROKO_API_KEY` | API key for roko-serve authentication |
| `ANTHROPIC_API_KEY` | Claude API key (for direct API agents) |
| `GITHUB_TOKEN` | GitHub personal access token (for MCP GitHub server) |
| `NO_COLOR` | Disable ANSI colors |

---

## Default invocation

Running `roko` without a subcommand has three modes:

```
# Interactive chat (stdin is a TTY, no --headless):
roko

# One-shot execution (prompt as positional argument):
roko "fix the login bug"

# Piped input:
echo "explain this code" | roko
```

The interactive chat opens an inline ratatui UI with streaming responses and
slash command support (`/help`, `/model`, `/plan`, `/prd`, `/research`, etc.).

---

## 1. Core Workflow

### `roko init`

Create `.roko/` and a default `roko.toml` in the specified directory.

```
roko init [path] [--cloud] [--profile <name>] [--demo]
```

| Flag | Description |
|---|---|
| `path` | Directory to initialize (default: current directory) |
| `--cloud` | Generate cloud-ready defaults (`bind = "0.0.0.0"`) |
| `--profile <name>` | Project profile: `rust`, `typescript`, `go`, `python`, `general` |
| `--demo` | Seed realistic demo data after initialization |

Creates:
- `.roko/` directory tree (state, prd, research, learn, episodes, etc.)
- `roko.toml` with detected project profile and provider config
- Auto-detects the `claude` CLI on PATH and writes a `[providers.claude_cli]` block

The template includes a `[models.claude-sonnet-4-6]` entry and gate suggestions
appropriate for the detected project language.

```
# Examples:
roko init                         # Initialize in the current directory
roko init /path/to/project        # Initialize in a specific directory
roko init --cloud                 # Initialize with cloud-ready defaults
roko init --profile rust          # Initialize with Rust project profile
roko init --demo                  # Initialize and seed demo data
```

**Related:** `roko doctor`, `roko config init`

---

### `roko run`

Seed a prompt and execute it through the universal loop:
compose → agent → gate → persist.

```
roko run <prompt> [--workdir <path>] [--serve] [--share] [--engine v2|legacy]
```

| Flag | Description |
|---|---|
| `prompt` | The user prompt text (required) |
| `--workdir <path>` | Override the working directory |
| `--serve` | Start the HTTP control plane alongside the run |
| `--share` | Generate a shareable URL for this run (implies `--serve`) |
| `--engine v2\|legacy` | Execution engine (default: `v2`) |

**Engine variants:**

- `v2` — WorkflowEngine: event-driven pipeline, composable stages, SSE observability (default)
- `legacy` — run_once() path: PlanRunner + orchestrate.rs (the original implementation)

The run produces an episode in `.roko/episodes.jsonl`, records efficiency events
in `.roko/learn/efficiency.jsonl`, and updates the cascade router state.

```
# Examples:
roko run "Fix the login bug"
roko run "Add tests for auth module" --role architect
roko run "Refactor db layer" --engine legacy --model claude-opus-4-6
roko run "Ship feature X" --share
```

**Related:** `roko plan run`, `roko status`

---

### `roko status`

Print signal counts, most recent episode, and gate pass/fail rates.

```
roko status [--workdir <path>] [--cfactor] [--surfaces]
```

| Flag | Description |
|---|---|
| `--workdir <path>` | Directory containing `.roko/` |
| `--cfactor` | Compute and persist the latest C-Factor metrics snapshot |
| `--surfaces` | Print the CLI/TUI/backend surface inventory instead of session status |

Outputs: engram count, episode count, last episode timestamp, gate pass/fail
breakdown, C-Factor trend (if `--cfactor`).

```
# Examples:
roko status
roko status --json
roko status --cfactor
```

**Related:** `roko doctor`, `roko learn all`

---

### `roko doctor`

Diagnose the workspace bootstrap state: checks for required directories,
config files, secret references, and server connectivity.

```
roko doctor [--workdir <path>] [--serve-url <url>]
```

| Flag | Description |
|---|---|
| `--workdir <path>` | Directory containing `roko.toml` and `.roko/` |
| `--serve-url <url>` | roko-serve base URL or health endpoint to probe |

```
# Examples:
roko doctor
roko doctor --serve-url http://localhost:6677/health
```

**Related:** `roko init`, `roko config validate`

---

### `roko resume`

Resume a plan execution from its last checkpoint.

```
roko resume [run_id] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `run_id` | Run or plan ID to resume (defaults to most recent snapshot) |
| `--workdir <path>` | Working directory |

Equivalent to `roko plan run plans/ --resume-plan` but operates on the most
recent executor snapshot automatically.

```
# Examples:
roko resume
roko resume run_4823
```

---

## 2. Planning

### `roko plan list`

List all plans in the workspace with their progress.

```
roko plan list [--workdir <path>]
```

Output columns: `ID`, `TITLE`, `PROGRESS` (done/total), `STATUS` (pending, in-progress, done).

With `--json`: array of objects with `id`, `title`, `task_count`, `tasks_done`,
`tasks_failed`, `completed`, `has_run_state`.

```
roko plan list
roko plan list --json
```

---

### `roko plan show`

Show full details of a specific plan.

```
roko plan show <plan_id> [--workdir <path>]
```

Displays plan ID, base directory, title, file paths, task count, and frontmatter
fields (`depends_on`, `parallel_with`, `priority`, `tags`, `milestone`).

```
roko plan show wire-system-prompt
roko plan show wire-system-prompt --json
```

---

### `roko plan create`

Create a new empty plan directory with `plan.md` and `tasks.toml` scaffolding.

```
roko plan create <plan_id> --title <title> [--description <text>] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `plan_id` | Plan identifier (used as directory name) |
| `--title <text>` | Human-readable plan title (required) |
| `--description <text>` | Plan description |
| `--workdir <path>` | Working directory |

Creates `plans/<plan_id>/plan.md` and `plans/<plan_id>/tasks.toml`.

```
roko plan create wire-gates --title "Wire gate pipeline into orchestrator"
```

---

### `roko plan validate`

Lint every `tasks.toml` under a plans directory without executing anything.

```
roko plan validate [dir] [--strict] [--json]
```

| Flag | Description |
|---|---|
| `dir` | Plans root directory (default: `plans/`) |
| `--strict` | Fail on warnings, not only errors |
| `--json` | Output machine-readable JSON report |

**This validation also runs automatically before every `roko plan run`.** A plan
that fails validation will not execute.

Checks include: TOML syntax, required fields, dependency cycles, unknown role
references, model name validity, gate rung ordering, and more.

Exit code is `0` for clean, `1` for errors (or warnings with `--strict`).

```
# Examples:
roko plan validate
roko plan validate plans/my-plan --strict
roko plan validate --json | jq '.totals'
```

---

### `roko plan run`

Execute a plan directory through the orchestration loop. Runs mandatory
validation before execution in both normal and dry-run modes.

```
roko plan run <plans_dir> [--workdir <path>] [--resume-plan [path]] [--approval]
              [--max-retries <n>] [--dry-run] [--fresh]
```

| Flag | Description |
|---|---|
| `plans_dir` | Path to the plans directory (required) |
| `--workdir <path>` | Working directory (repo root) |
| `--resume-plan [path]` | Resume from `.roko/state/executor.json` (or explicit path) |
| `--approval` | Launch the interactive approval TUI while the plan runs |
| `--max-retries <n>` | Maximum retry attempts per task (overrides per-task config) |
| `--dry-run` | Parse and display the plan without executing |
| `--fresh` | Archive old run state and start from scratch |

**`--fresh` behavior:** Moves the existing `executor.json` to a timestamped backup
(`.roko/state/executor.json.bak.<timestamp>`) and starts execution from the
beginning. The backup is never deleted — it is preserved for inspection.

**`--resume-plan` behavior:** If a path is specified, it is copied to the standard
location (`.roko/state/executor.json`) before execution. If omitted, the flag
uses the default snapshot location. The runner auto-resumes from `executor.json`
when it exists unless `--fresh` is set.

If the working directory has no `.git` repo, one is automatically initialized
so that agent tooling (which requires git) works correctly.

```
# Examples:
roko plan run plans/
roko plan run plans/my-plan --approval
roko plan run plans/ --dry-run
roko plan run plans/ --fresh
roko plan run plans/ --resume-plan
roko plan run plans/ --resume-plan .roko/state/executor.json
roko plan run plans/ --max-retries 3
```

**Related:** `roko plan validate`, `roko resume`

---

### `roko plan generate`

Generate implementation plans from a prompt, file, or PRD using an agent.

```
roko plan generate <source...> [--from-file <path>]
```

| Flag | Description |
|---|---|
| `source` | Free-text prompt, or file path (PRD, requirements doc) |
| `--from-file <path>` | Treat source as a file path to read instead of inline text |

```
roko plan generate "Wire the SystemPromptBuilder into orchestrate.rs"
roko plan generate --from-file .roko/prd/published/my-feature.md
```

---

### `roko plan regenerate`

Regenerate an existing plan from its source PRD or plan extract.

```
roko plan regenerate <plan_dir> [--dry-run]
```

| Flag | Description |
|---|---|
| `plan_dir` | Path to the plan directory containing `tasks.toml` |
| `--dry-run` | Preview changes without overwriting |

```
roko plan regenerate plans/my-plan
roko plan regenerate plans/my-plan --dry-run
```

---

## 3. PRDs

### `roko prd idea`

Capture a quick work item idea, saved to `.roko/prd/ideas/`.

```
roko prd idea <text...>
```

```
roko prd idea "Wire SystemPromptBuilder into orchestrate.rs"
roko prd idea Add adaptive gate threshold tuning to the learning loop
```

---

### `roko prd list`

List all PRDs: published, drafts, and ideas.

```
roko prd list
```

---

### `roko prd status`

Show a coverage report across all PRDs and plans: which PRDs have plans,
which are in-progress, which are fully executed.

```
roko prd status
```

---

### `roko prd draft new`

Create a new draft PRD with agent assistance. The agent reads the codebase
and produces a PRD including a mandatory `## Repository Grounding` section.

```
roko prd draft new <title...>
```

PRDs missing the Repository Grounding section emit a warning. The section is
validated against workspace crate members — false-negative claims ("no existing
crates") when the workspace has crates are flagged.

```
roko prd draft new "Adaptive gate threshold tuning"
roko prd draft new Wire SystemPromptBuilder into orchestrate.rs
```

---

### `roko prd draft edit`

Refine an existing draft with agent assistance.

```
roko prd draft edit <slug>
```

| Flag | Description |
|---|---|
| `slug` | Draft filename without `.md` extension |

```
roko prd draft edit adaptive-gate-thresholds
```

---

### `roko prd draft promote`

Promote a draft PRD to published status.

```
roko prd draft promote <slug> [--auto-execute]
```

| Flag | Description |
|---|---|
| `slug` | Draft filename without `.md` extension |
| `--auto-execute` | Execute the generated plan immediately after promotion |

Moves the draft from `.roko/prd/drafts/` to `.roko/prd/published/`.
If `prd.auto_plan = true` is set in `roko.toml`, publishing triggers automatic
plan generation via the `prd_publish_subscriber`.

```
roko prd draft promote adaptive-gate-thresholds
roko prd draft promote my-feature --auto-execute
```

---

### `roko prd draft list`

List all draft PRDs.

```
roko prd draft list
```

---

### `roko prd plan`

Generate implementation tasks from a published PRD using an agent.

```
roko prd plan <slug> [--dry-run]
```

| Flag | Description |
|---|---|
| `slug` | PRD filename without `.md` extension |
| `--dry-run` | Preview generation without writing `tasks.toml` files |

Writes `plans/<slug>/tasks.toml`. The agent reads the PRD and the current
codebase state to produce a grounded task list.

```
roko prd plan adaptive-gate-thresholds
roko prd plan my-feature --dry-run
```

---

### `roko prd consolidate`

Scan all PRDs for duplicates, gaps, and inconsistencies.

```
roko prd consolidate
```

---

## 4. Agents

### `roko agent create`

Create a new agent from a manifest.

```
roko agent create --name <name> [--domain <domain>] [--template <name>] [--prompt <text>]
                  [--skills <list>] [--tier <tier>] [--reputation <n>]
                  [--max-concurrent-jobs <n>] [--serve-url <url>] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `--name <name>` | Human-readable agent name (required) |
| `--domain <domain>` | Agent domain: `coding`, `research`, `chain`, `general` (default: `general`) |
| `--template <name>` | Strategy template (e.g. `fast-coding`, `deep-research`) |
| `--prompt <text>` | Natural-language description of what the agent should do |
| `--skills <list>` | Comma-separated skill tags (e.g. `rust,p2p,networking`) |
| `--tier <tier>` | Agent tier: `Unverified`, `Verified`, `Trusted`, `Expert`, `Pioneer` |
| `--reputation <n>` | Reputation score 0–100 (default: `0`) |
| `--max-concurrent-jobs <n>` | Maximum concurrent jobs (default: `0` = unlimited) |
| `--serve-url <url>` | Auto-register with roko-serve after creation |
| `--workdir <path>` | Working directory |

Generates `.roko/agents/<name>/manifest.toml` after validating all manifest
fields. Domain presets wire appropriate capabilities.

```
roko agent create --name coder --domain coding --skills "rust,wasm"
roko agent create --name researcher --domain research --prompt "Deep research on protocol design"
```

---

### `roko agent delete`

Delete an agent and clean up its state with an ordered 8-step shutdown.

```
roko agent delete --name <name> [--force] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `--name <name>` | Agent name to delete (required) |
| `--force` | Skip ordered shutdown and remove immediately |
| `--workdir <path>` | Working directory |

The ordered shutdown: stop processing → flush pending → backup knowledge →
deregister from mesh → release resources → archive signals → clean state →
emit deletion marker.

```
roko agent delete --name coder
roko agent delete --name coder --force
```

---

### `roko agent list`

List all agents with their status.

```
roko agent list [--workdir <path>]
```

---

### `roko agent start`

Start a previously created agent (launches the per-agent HTTP sidecar).

```
roko agent start --name <name> [--bind <addr>] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `--name <name>` | Agent name (required) |
| `--bind <addr>` | Socket address to bind (default: `127.0.0.1:0` for auto-port) |
| `--workdir <path>` | Working directory |

```
roko agent start --name coder
roko agent start --name coder --bind 127.0.0.1:7788
```

---

### `roko agent stop`

Stop a running agent.

```
roko agent stop --name <name> [--force] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `--name <name>` | Agent name (required) |
| `--force` | Send SIGKILL instead of SIGTERM |
| `--workdir <path>` | Working directory |

---

### `roko agent status`

Show detailed status for one agent.

```
roko agent status --name <name> [--workdir <path>]
```

---

### `roko agent serve`

Start a per-agent HTTP runtime (the agent sidecar server). This is the low-level
command; `roko up` or `roko agent start` are normally used instead.

```
roko agent serve --agent-id <id> [--bind <addr>] [--relay-url <url>]
                 [--chain-rpc-url <url>] [--identity-registry <addr>]
                 [--passport-id <id>] [--wallet-key <key>]
                 [--serve-url <url>]
```

| Flag | Default | Description |
|---|---|---|
| `--agent-id <id>` | required | Unique agent identifier advertised by the runtime |
| `--bind <addr>` | `127.0.0.1:0` | Socket address to bind |
| `--relay-url <url>` | — | Relay bridge URL (reserved for future use) |
| `--chain-rpc-url <url>` | — | Chain JSON-RPC URL (reserved for future use) |
| `--identity-registry <addr>` | — | ERC-8004 identity registry contract address |
| `--passport-id <id>` | — | ERC-8004 passport ID for `updateAgentCardUri` |
| `--wallet-key <key>` | — | Wallet private key (reserved for future signing) |
| `--serve-url <url>` | `http://localhost:6677` | Control plane URL for heartbeat reporting |

The sidecar exposes 13 routes including `/message` (real LLM dispatch),
`/stream` (WebSocket), `/predictions`, `/research`, and `/tasks`.

---

### `roko agent chat`

Open an interactive chat REPL with a specific agent via roko-serve.

```
roko agent chat [--agent <id>] [--serve-url <url>]
```

| Flag | Default | Description |
|---|---|---|
| `--agent <id>` | `nunchi-intelligence` | Agent ID to chat with |
| `--serve-url <url>` | `http://localhost:6677` | roko-serve base URL |

```
roko agent chat --agent coder
roko agent chat --agent researcher --serve-url http://my-server.com
```

---

## 5. Research

### `roko research topic`

Deep-dive research on a topic. Produces `.roko/research/<slug>.md` with citations.

```
roko research topic <topic...> [--deep]
```

| Flag | Description |
|---|---|
| `topic` | The research topic (joined with spaces if multiple words) |
| `--deep` | Use Perplexity deep research (async polling, 1–10 min) |

Without `--deep`, uses the configured agent (Claude CLI by default) for synthesis.
With `--deep`, uses `sonar-deep-research` model via Perplexity API and polls
every 15 seconds until complete.

```
roko research topic "cascade routing for LLM agents"
roko research topic adaptive gate thresholds --deep
```

---

### `roko research search`

Direct web search using Perplexity's search API. Returns raw results without synthesis.

```
roko research search <query...> [--domains <list>] [--recency <period>]
```

| Flag | Description |
|---|---|
| `query` | Search query text |
| `--domains <list>` | Restrict to these domains (comma-separated, e.g. `docs.rs,github.com`) |
| `--recency <period>` | Recency filter: `day`, `week`, `month`, `year` |

```
roko research search "Rust async runtime" --domains docs.rs,github.com
roko research search "Claude API changes" --recency week
```

---

### `roko research enhance-prd`

Enhance a PRD with academic citations, diagrams, and research-backed improvements.

```
roko research enhance-prd <slug>
```

Reads `.roko/prd/published/<slug>.md`, enriches it with research context from the
neuro store, and writes the improved version back.

```
roko research enhance-prd adaptive-gate-thresholds
```

---

### `roko research enhance-plan`

Optimize an implementation plan with research-backed task decomposition techniques.

```
roko research enhance-plan <plan>
```

| Argument | Description |
|---|---|
| `plan` | Plan directory name under `.roko/plans/` |

---

### `roko research enhance-tasks`

Optimize tasks for efficiency, parallelism, and cheapest viable model assignment.

```
roko research enhance-tasks <plan>
```

---

### `roko research analyze`

Analyze execution episodes for self-learning insights and bandit weight recommendations.

```
roko research analyze
```

Reads `.roko/episodes.jsonl` and `.roko/learn/efficiency.jsonl`, produces
recommendations for router weights, prompt experiments, and gate thresholds.

---

### `roko research list`

List all research artifacts in `.roko/research/`.

```
roko research list
```

---

## 6. Knowledge

### `roko knowledge query`

Query the durable knowledge (neuro) store for a topic.

```
roko knowledge query <topic...> [--workdir <path>]
```

Performs a semantic search over `.roko/learn/neuro.jsonl`. Returns matching engrams
sorted by confidence score.

```
roko knowledge query "gate threshold adaptation"
roko knowledge query cascade router model selection
```

---

### `roko knowledge stats`

Show aggregate statistics for the durable knowledge store.

```
roko knowledge stats [--workdir <path>]
```

Reports: total engrams, confidence distribution, age distribution, type breakdown.

---

### `roko knowledge gc`

Run garbage collection on the durable knowledge store.

```
roko knowledge gc [--workdir <path>]
```

Removes engrams below the minimum confidence threshold
(`DEFAULT_GC_MIN_CONFIDENCE`). Reports how many entries were removed.

---

### `roko knowledge backup`

Backup the knowledge store to a directory with optional genomic bottleneck.

```
roko knowledge backup <destination> [--workdir <path>] [--force] [--top-n <n>]
```

| Flag | Description |
|---|---|
| `destination` | Directory to write backup files into (required) |
| `--workdir <path>` | Directory containing `.roko/` |
| `--force` | Overwrite existing backup files in the destination |
| `--top-n <n>` | Genomic bottleneck: export only the top N entries by confidence |

The `--top-n` flag implements a "genetic bottleneck" — only the highest-confidence
knowledge survives, simulating evolutionary pressure.

```
roko knowledge backup ~/backups/roko-knowledge
roko knowledge backup ~/backups/roko-knowledge --top-n 500
```

---

### `roko knowledge restore`

Restore the knowledge store from a backup with confidence decay.

```
roko knowledge restore <source> [--workdir <path>] [--force] [--types <list>]
                       [--min-confidence <f>] [--generation <n>]
```

| Flag | Default | Description |
|---|---|---|
| `source` | required | Directory created by `roko knowledge backup` |
| `--workdir <path>` | — | Directory containing `.roko/` |
| `--force` | false | Overwrite existing local neuro store files |
| `--types <list>` | — | Filter by knowledge types (comma-separated) |
| `--min-confidence <f>` | — | Only restore entries with confidence >= threshold (0.0–1.0) |
| `--generation <n>` | `1` | Hop count for confidence decay |

Each generation hop applies a confidence decay factor to restored entries,
preventing stale knowledge from polluting the active store at full confidence.

```
roko knowledge restore ~/backups/roko-knowledge
roko knowledge restore ~/backups/roko-knowledge --min-confidence 0.6 --generation 2
```

---

### `roko knowledge sync`

Sync knowledge with a peer agent via the Mesh protocol.

```
roko knowledge sync <peer> [--workdir <path>] [--direction <dir>] [--max-send <n>]
```

| Flag | Default | Description |
|---|---|---|
| `peer` | required | Peer agent identifier to sync with |
| `--workdir <path>` | — | Working directory |
| `--direction <dir>` | `both` | Direction: `send`, `receive`, or `both` |
| `--max-send <n>` | `100` | Maximum engrams to send in this sync cycle |

---

### `roko knowledge dream run`

Run a dream consolidation cycle immediately.

```
roko knowledge dream run [--workdir <path>]
```

Dream consolidation processes recent episodes and engrams, extracts patterns,
and distills durable knowledge into the neuro store. This is the offline
learning cycle (the "hypnagogia" phase).

---

### `roko knowledge dream report`

Show the latest dream report without running a new cycle.

```
roko knowledge dream report [--workdir <path>]
```

---

### `roko knowledge dream schedule`

Show when the next dream cycle is scheduled to fire.

```
roko knowledge dream schedule [--workdir <path>]
```

---

### `roko knowledge dream journal`

Display recent dream journal entries.

```
roko knowledge dream journal [--limit <n>] [--workdir <path>]
```

| Flag | Default | Description |
|---|---|---|
| `--limit <n>` | `10` | Number of recent entries to display |
| `--workdir <path>` | — | Working directory |

---

### `roko knowledge dream archive`

Display recent dream archive entries.

```
roko knowledge dream archive [--limit <n>] [--workdir <path>]
```

---

### `roko knowledge custody list`

List recent custody audit records.

```
roko knowledge custody list [--limit <n>] [--workdir <path>]
```

---

### `roko knowledge custody show`

Show full details of a custody record by index.

```
roko knowledge custody show <index> [--workdir <path>]
```

---

### `roko knowledge custody verify`

Verify integrity of the custody chain.

```
roko knowledge custody verify [--workdir <path>]
```

Checks the hash chain is unbroken. Reports first broken link if found.

---

### `roko knowledge archive`

Move old engrams to cold storage (compressed monthly archives).

```
roko knowledge archive [--older-than <duration>] [--batch-size <n>]
                       [--workdir <path>] [--dry-run]
```

| Flag | Default | Description |
|---|---|---|
| `--older-than <duration>` | `30d` | Archive engrams older than this (e.g. `30d`, `7d`) |
| `--batch-size <n>` | `500` | Maximum engrams to archive per batch |
| `--workdir <path>` | — | Working directory |
| `--dry-run` | false | Print what would be archived without doing it |

```
roko knowledge archive --older-than 90d
roko knowledge archive --dry-run
```

---

## 7. Learning

### `roko learn all`

Show all learning state: cascade router, experiments, efficiency, episodes.

```
roko learn all [--workdir <path>]
```

---

### `roko learn route`

Show cascade router state (model selection weights, bandit arms, recent decisions).

```
roko learn route [--workdir <path>]
```

Reads `.roko/learn/cascade-router.json`.

---

### `roko learn experiments`

Show prompt A/B experiment state (arm assignments, success rates, EMA estimates).

```
roko learn experiments [--workdir <path>]
```

Reads `.roko/learn/experiments.json`.

---

### `roko learn efficiency`

Show per-role efficiency metrics (cost per token, latency, success rate).

```
roko learn efficiency [--workdir <path>]
```

Reads `.roko/learn/efficiency.jsonl`.

---

### `roko learn episodes`

Show episode summary (recent agent turns, gate outcomes, C-Factor trend).

```
roko learn episodes [--workdir <path>]
```

Reads `.roko/episodes.jsonl`.

---

### `roko learn tune`

Display and optionally adjust adaptive thresholds and model routing parameters.

```
roko learn tune [subsystem] [--dry-run] [--workdir <path>]
```

| Argument | Default | Description |
|---|---|---|
| `subsystem` | `gates` | Subsystem to tune: `gates`, `routing`, or `budget` |
| `--dry-run` | false | Display current values without modifying |
| `--workdir <path>` | — | Working directory |

**Subsystems:**

- `gates` — EMA-based adaptive gate thresholds (`.roko/learn/gate-thresholds.json`)
- `routing` — Cascade router state and bandit weights
- `budget` — Efficiency log entry count and budget headroom

```
roko learn tune gates
roko learn tune routing --dry-run
roko learn tune budget
```

---

## 8. Configuration

### `roko config init`

Interactive wizard: detects installed LLM CLIs, writes global config. Also available
as `roko config wizard`.

```
roko config init [--yes] [--agent <cmd>] [--model <name>] [--budget <n>]
                 [--role <text>] [--enable-gates] [--path <path>]
                 [--non-interactive]
```

| Flag | Description |
|---|---|
| `--yes` | Skip all confirmation prompts |
| `--agent <cmd>` | Pre-select agent command (skip picker) |
| `--model <name>` | Pre-set model name (ollama-only convenience) |
| `--budget <n>` | Pre-set token budget |
| `--role <text>` | Pre-set role string |
| `--enable-gates` | Enable default compile + clippy gates |
| `--path <path>` | Write to this path instead of the resolved global path |
| `--non-interactive` | Skip all prompts, fail if any required answer is missing |

In `--non-interactive` mode, `--agent` is required. Defaults: budget = 8000,
role = "You are a Roko agent.", gates disabled.

```
roko config init
roko config init --yes --agent claude --enable-gates
roko config init --non-interactive --agent claude
```

---

### `roko config show`

Print the effective merged config with per-field source tags (global, project, env).

```
roko config show [--workdir <path>]
```

---

### `roko config path`

Print the resolved global and project config file paths.

```
roko config path [--workdir <path>]
```

---

### `roko config edit`

Open `$EDITOR` on the chosen config file.

```
roko config edit [--global | --project] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `--global` | Open the global config file |
| `--project` | Open (or create) the project `roko.toml` |

If neither flag is given, auto-selects based on whether a project config exists.

```
roko config edit --global
roko config edit --project
```

---

### `roko config set`

Set a dotted key in the chosen config layer.

```
roko config set <key> <value> [--project | --global] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `key` | Dotted key path (e.g. `agent.command`, `executor.task_timeout_secs`) |
| `value` | Value to write |
| `--project` | Write to project config |
| `--global` | Write to global config (default) |

```
roko config set agent.command ollama
roko config set executor.task_timeout_secs 300 --project
```

---

### `roko config validate`

Validate `roko.toml` syntax, schema, and semantic references (provider names,
model slugs, secret references).

```
roko config validate [--workdir <path>]
```

---

### `roko config migrate`

Migrate a legacy project `roko.toml` from the v1 `[agent]` command format into
explicit `[providers.*]` and `[models.*]` tables.

```
roko config migrate [--workdir <path>] [--dry-run] [--yes]
```

| Flag | Description |
|---|---|
| `--workdir <path>` | Working directory |
| `--dry-run` | Print the proposed migration without writing changes |
| `--yes` / `-y` | Skip the confirmation prompt and apply immediately |

```
roko config migrate --dry-run
roko config migrate --yes
```

---

### `roko config set-secret`

Store a secret in `~/.roko/.env` as `NAME=VALUE`.

```
roko config set-secret <name> <value>
```

Secrets stored here are loaded automatically at startup and are redacted
from log output.

```
roko config set-secret ANTHROPIC_API_KEY sk-ant-...
roko config set-secret PERPLEXITY_API_KEY pplx-...
```

---

### `roko config check-secrets`

Check `${VAR}` references in the active config and validate that referenced
secrets are present.

```
roko config check-secrets [--workdir <path>]
```

---

### `roko config providers list`

List configured providers and their current connection status.

```
roko config providers list [--workdir <path>]
```

---

### `roko config providers health`

Show persisted provider circuit-breaker health and latency statistics.

```
roko config providers health [--workdir <path>]
```

Reads the circuit-breaker state from `.roko/learn/` and reports: circuit state
(closed/open/half-open), error rate, p50/p95 latency.

---

### `roko config providers test`

Send a minimal request to verify provider connectivity.

```
roko config providers test [<provider>] [--all] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `provider` | Provider name from `[providers.*]` |
| `--all` | Test every configured provider and print a summary table |

```
roko config providers test claude_cli
roko config providers test --all
```

---

### `roko config models list`

List configured models and their capabilities.

```
roko config models list [--workdir <path>]
```

---

### `roko config models route`

Show the current routing decision for a model key and optionally explain why it won.

```
roko config models route <model> [--explain] [--complexity <tier>] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `model` | Model key or slug to explain |
| `--explain` | Show the full routing trace instead of only the final decision |
| `--complexity <tier>` | Complexity tier: `mechanical`, `focused`, `integrative`, `architectural` |

```
roko config models route claude-sonnet-4-6 --explain
roko config models route claude-opus-4-6 --complexity architectural
```

---

### `roko config subscriptions list`

List all event subscriptions.

```
roko config subscriptions list
```

---

### `roko config subscriptions add`

Create a new event subscription.

```
roko config subscriptions add --template <name> --trigger <glob>
```

| Flag | Description |
|---|---|
| `--template <name>` | Agent template name to invoke when triggered |
| `--trigger <glob>` | Engram trigger glob to match |

---

### `roko config subscriptions remove`

Delete a subscription by ID.

```
roko config subscriptions remove <id>
```

---

### `roko config subscriptions enable`

Enable a previously disabled subscription.

```
roko config subscriptions enable <id>
```

---

### `roko config subscriptions disable`

Disable a subscription without deleting it.

```
roko config subscriptions disable <id>
```

---

### `roko config events`

Inspect configured event sources (cron jobs, file watchers).

```
roko config events [--workdir <path>]
```

---

### `roko config experiments`

Manage model A/B experiments (subcommands inherited from the experiment module).

```
roko config experiments <subcommand>
```

See `roko config experiments --help` for available subcommands.

---

### `roko config plugins list`

List available and installed plugins.

```
roko config plugins list [--workdir <path>]
```

---

### `roko config plugins install`

Install a plugin from a local path or registry.

```
roko config plugins install <source> [--workdir <path>]
```

| Argument | Description |
|---|---|
| `source` | Path to the plugin manifest (`plugin.toml`) or directory |

---

### `roko config plugins remove`

Remove an installed plugin by name.

```
roko config plugins remove <name> [--workdir <path>]
```

---

### `roko config plugins audit`

Audit installed plugins and report capabilities and permissions.

```
roko config plugins audit [--workdir <path>]
```

---

### `roko config secrets set`

Store a named secret in the profile-aware secrets store.

```
roko config secrets set <name> <value>
```

---

### `roko config secrets get`

Retrieve a named secret.

```
roko config secrets get <name>
```

---

### `roko config secrets list`

List all stored secret names (values are not shown).

```
roko config secrets list
```

---

### `roko config secrets rotate`

Rotate a named secret (prompts for the new value).

```
roko config secrets rotate <name>
```

---

## 9. Jobs

### `roko job list`

List all marketplace jobs.

```
roko job list [--workdir <path>] [--status <status>]
```

| Flag | Description |
|---|---|
| `--workdir <path>` | Working directory |
| `--status <status>` | Filter by status: `open`, `assigned`, `in_progress`, `completed`, `failed`, `cancelled` |

Output format: status icon + job type + status + ID (first 8 chars) + title.

```
roko job list
roko job list --status in_progress
roko job list --json
```

---

### `roko job create`

Create a new marketplace job.

```
roko job create <title> [--type <type>] [--description <text>] [--priority <level>]
                [--auto-execute] [--plan-id <id>] [--workdir <path>]
```

| Flag | Default | Description |
|---|---|---|
| `title` | required | Job title |
| `--type <type>` | `research` | Job type: `research`, `coding_task`, `chain_monitor`, `chain_analysis` |
| `--description <text>` | `""` | Job description |
| `--priority <level>` | `medium` | Priority: `low`, `medium`, `high`, `critical` |
| `--auto-execute` | false | Auto-execute when a runner picks it up |
| `--plan-id <id>` | — | Associated plan ID |
| `--workdir <path>` | — | Working directory |

Persists the job as `.roko/jobs/<uuid>.json`.

```
roko job create "Research cascade routing improvements" --type research
roko job create "Fix auth module" --type coding_task --priority high --auto-execute
```

---

### `roko job match`

Match a proposed job against registered agents via roko-serve.

```
roko job match <title> [--serve-url <url>] [--description <text>] [--language <lang>]
               [--min-tier <tier>] [--reward <amount>] [--skills <list>]
               [--workdir <path>]
```

| Flag | Default | Description |
|---|---|---|
| `title` | required | Job title |
| `--serve-url <url>` | `http://localhost:6677` | roko-serve base URL |
| `--description <text>` | `""` | Job description |
| `--language <lang>` | — | Primary implementation language (treated as required skill) |
| `--min-tier <tier>` | — | Minimum agent tier: `Unverified`, `Verified`, `Trusted`, `Expert`, `Pioneer` |
| `--reward <amount>` | `""` | Reward string (e.g. `"2500 KORAI"`) |
| `--skills <list>` | — | Required skills (comma-separated) |
| `--workdir <path>` | — | Working directory (for auth config) |

---

### `roko job show`

Show full details for a specific job.

```
roko job show <id> [--workdir <path>]
```

---

### `roko job execute`

Execute a job locally or via roko-serve.

```
roko job execute <id> [--serve-url <url>] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `id` | Job ID (required) |
| `--serve-url <url>` | If set, POST to `/api/jobs/<id>/execute` on roko-serve |

---

### `roko job cancel`

Cancel a job.

```
roko job cancel <id> [--workdir <path>]
```

---

## 10. Server and Deployment

### `roko up`

Start roko-serve plus all configured `[[agents]]` from `roko.toml` in one command.

```
roko up [--workdir <path>]
```

Reads `roko.toml`, starts roko-serve in the background, then creates and starts
each enabled `[[agents]]` entry. Prints a status line for each.

If no agents are configured, only the server starts. Add `[[agents]]` blocks to
`roko.toml` to launch agents automatically.

```
roko up
roko up --workdir /path/to/project
```

**Related:** `roko serve`, `roko agent create`, `roko agent start`

---

### `roko serve`

Start the HTTP API control plane.

```
roko serve [--bind <addr>] [--port <n>] [--workdir <path>] [--tui] [--enable-terminal]
```

| Flag | Default | Description |
|---|---|---|
| `--bind <addr>` | `127.0.0.1` | IP address to bind |
| `--port <n>` | `6677` | Port number |
| `--workdir <path>` | cwd | Working directory |
| `--tui` | false | Run the interactive TUI dashboard embedded in the server process |
| `--enable-terminal` | false | Expose the PTY terminal routes |

The control plane exposes approximately 85 REST routes for dashboards and external
callers. In `--tui` mode, all tracing output is routed to `.roko/serve-tui.log`
to prevent it from corrupting the ratatui screen.

```
roko serve
roko serve --port 8080 --bind 0.0.0.0
roko serve --tui
```

**Related:** `roko up`, `roko daemon start`

---

### `roko acp`

Start the ACP (Agent Client Protocol) server for editor integration.
ACP uses JSON-RPC over stdio; stdout is the protocol channel.

```
roko acp [--workdir <path>] [--profile <name>] [--config <path>] [--log-file <path>]
```

| Flag | Default | Description |
|---|---|---|
| `--workdir <path>` | `.` | Working directory |
| `--profile <name>` | `default` | Configuration profile |
| `--config <path>` | — | Path to `roko.toml` config file |
| `--log-file <path>` | `.roko/acp.log` | Log file path (stdout is the protocol channel) |

ACP mode bypasses the normal tracing subscriber initialization to keep stdout
clean for JSON-RPC traffic. All log output goes to the log file.

---

### `roko daemon start`

Start the daemon.

```
roko daemon start [--foreground] [--port <n>]
```

| Flag | Default | Description |
|---|---|---|
| `--foreground` | false | Run in the foreground instead of daemonizing |
| `--port <n>` | `6677` | Port number |

---

### `roko daemon stop`

Stop the running daemon.

```
roko daemon stop
```

---

### `roko daemon status`

Show daemon status.

```
roko daemon status
```

---

### `roko daemon logs`

Show daemon logs.

```
roko daemon logs [-f] [-n <lines>]
```

| Flag | Default | Description |
|---|---|---|
| `-f` / `--follow` | false | Follow log output (tail -f style) |
| `-n <lines>` | `50` | Number of lines to show |

---

### `roko daemon reload`

Reload daemon configuration without restart (SIGHUP equivalent — re-scans
subscriptions and templates).

```
roko daemon reload
```

---

### `roko daemon restart`

Restart the daemon.

```
roko daemon restart [--port <n>]
```

---

### `roko daemon install`

Install the daemon as a system service.

```
roko daemon install
```

On macOS, generates and installs a launchd plist. On Linux, generates a
systemd unit file.

---

### `roko daemon uninstall`

Remove the installed system service.

```
roko daemon uninstall
```

---

### `roko deploy railway`

Deploy the current workspace to Railway via the public GraphQL API.
Creates a Railway project with roko-serve as the control plane.

```
roko deploy railway [--workdir <path>] [--with-mirage] [--workers <list>]
```

| Flag | Description |
|---|---|
| `--workdir <path>` | Repository root |
| `--with-mirage` | Also deploy the mirage chain relay service |
| `--workers <list>` | Deploy worker services for these template names (comma-separated) |

Requires `RAILWAY_TOKEN` in environment or `~/.roko/.env`.

---

### `roko deploy fly`

Generate `fly.toml` and deploy with Fly.io.

```
roko deploy fly [--workdir <path>]
```

---

### `roko deploy docker`

Build the local Docker image and tag it for the configured registry.

```
roko deploy docker [--workdir <path>] [--registry <namespace>]
```

---

### `roko worker`

Run as a deployed worker. Reads the template from environment, serves tasks.

```
roko worker [--port <n>]
```

| Flag | Default | Description |
|---|---|---|
| `--port <n>` | `8080` | Port to listen on (overridden by `PORT` env variable) |

---

## 11. Utilities

### `roko dashboard`

Launch the interactive ratatui TUI dashboard.

```
roko dashboard [--page <slug>] [--list-pages] [--text] [--workdir <path>]
               [--high-contrast] [--reduced-motion]
```

| Flag | Description |
|---|---|
| `--page <slug>` | Jump directly to a specific dashboard page |
| `--list-pages` | List all available page slugs and exit |
| `--text` | Force text-mode output instead of the interactive TUI |
| `--workdir <path>` | Working directory |
| `--high-contrast` | High-contrast color scheme (WCAG 2.1 AA) |
| `--reduced-motion` | Disable animations for reduced-motion accessibility |

When stdout is a TTY and `--text` is not set, launches the interactive ratatui TUI
with a 60fps event loop. Tabs: F1–F7 for different dashboard views.

When stdout is not a TTY or `--text` is set, renders a text summary.

```
roko dashboard
roko dashboard --page health
roko dashboard --list-pages
roko dashboard --text --json
```

---

### `roko replay`

Walk the lineage DAG rooted at a signal hash and print it.

```
roko replay <hash> [--workdir <path>] [--forensic] [--as-of <step>] [--format tree|json]
```

| Flag | Default | Description |
|---|---|---|
| `hash` | required | Engram hash (64 hex chars) to walk |
| `--workdir <path>` | — | Directory containing `.roko/` |
| `--forensic` | false | Show forensic detail: timestamps, full hashes, metadata |
| `--as-of <step>` | — | Filter replay to events from this step forward |
| `--format <fmt>` | `tree` | Output format: `tree` or `json` |

```
roko replay a3f5c2d1...
roko replay a3f5c2d1... --forensic --format json
```

---

### `roko inject`

Inject a signal into a running session.

```
roko inject <session> <payload> [--kind <type>] [--workdir <path>]
```

| Flag | Default | Description |
|---|---|---|
| `session` | required | Target session ID |
| `payload` | required | Payload text |
| `--kind <type>` | `directive` | Kind of signal: `directive`, `abort`, `context` |
| `--workdir <path>` | — | Working directory (to locate the daemon socket) |

```
roko inject session-abc "stop current task" --kind abort
roko inject session-abc "here is additional context: ..." --kind context
```

---

### `roko index build`

Build a code index for the workspace.

```
roko index build [--path <dir>]
```

| Flag | Description |
|---|---|
| `--path <dir>` | Directory to index (default: cwd / `--repo`) |

---

### `roko index rebuild`

Drop existing index data and rebuild from source files.

```
roko index rebuild [--path <dir>]
```

---

### `roko index search`

Search the code index.

```
roko index search <query> [--kind <symbol_kind>] [--strategy <name>] [--limit <n>]
                  [--path <dir>]
```

| Flag | Default | Description |
|---|---|---|
| `query` | required | Search query text |
| `--kind <type>` | — | Symbol kind: `function`, `struct`, `enum`, `trait`, `const`, `type`, `module`, `impl` |
| `--strategy <name>` | `keyword` | Search strategy: `keyword`, `structural`, `hybrid` |
| `--limit <n>` | `20` | Maximum number of results |
| `--path <dir>` | — | Directory to index |

```
roko index search "run_agent"
roko index search "AgentDispatcher" --kind struct --strategy structural
```

---

### `roko index stats`

Show code index statistics.

```
roko index stats [--path <dir>]
```

---

### `roko new`

Generate boilerplate for a Roko trait or domain profile.

```
roko new <type> <name> [--output <dir>]
```

| Argument | Description |
|---|---|
| `type` | Scaffold type: `gate`, `scorer`, `router`, `policy`, `substrate`, `composer`, `domain`, `template`, `event-source` |
| `name` | Name for the generated component (e.g. `my-custom-gate`) |
| `--output <dir>` | Output directory (default: current directory) |

```
roko new gate my-approval-gate
roko new scorer latency-scorer --output crates/roko-custom/src/
roko new domain trading
```

---

### `roko explain`

Explain a Roko concept with progressive disclosure (3 depth levels).

```
roko explain <topic> [--depth <n>]
```

| Argument | Default | Description |
|---|---|---|
| `topic` | required | Topic to explain: `gates`, `routing`, `cognitive`, `neuro`, `daimon`, `dreams`, `engram`, `cfactor`, etc. |
| `--depth <n>` | `1` | Disclosure depth: `1` = summary, `2` = how it works, `3` = internals |

```
roko explain topics              # List all available topics
roko explain gates
roko explain routing --depth 3
roko explain cfactor --depth 2
```

---

### `roko completions`

Generate shell completion scripts.

```
roko completions <shell>
```

| Argument | Description |
|---|---|
| `shell` | Shell to generate for: `bash`, `zsh`, `fish` |

```
# Install completions:
roko completions zsh > ~/.zsh/completions/_roko
roko completions bash > ~/.bash_completion.d/roko
roko completions fish > ~/.config/fish/completions/roko.fish
```

---

## 12. Authentication

### `roko login`

Authenticate with a roko-serve instance.

```
roko login [url] [--api-key] [--check] [--dashboard-url <url>]
```

| Flag | Default | Description |
|---|---|---|
| `url` | `http://localhost:6677` | URL of the roko-serve instance |
| `--api-key` | false | Login with an API key instead of browser auth |
| `--check` | false | Non-interactive: validate stored credential only (requires `--api-key`) |
| `--dashboard-url <url>` | `http://localhost:5173` | Dashboard URL for browser auth |

Environment variable: `NUNCHI_DASHBOARD_URL` overrides `--dashboard-url`.

```
roko login                              # Login via browser (Privy)
roko login --api-key                    # Login with an API key (prompts)
roko login --api-key --check            # Validate stored API key credential
roko login https://my-server.com        # Login to a remote server
```

Credentials are stored in `~/.roko/credentials.json`.

---

### `roko logout`

Remove stored credentials.

```
roko logout
```

---

### `roko whoami`

Show current authentication status.

```
roko whoami
```

---

## 13. Benchmarks

### `roko bench demo`

Run a comparative benchmark: naive vs roko-optimized with simulated or real dispatch.

```
roko bench demo [--real] [--workdir <path>]
```

| Flag | Description |
|---|---|
| `--real` | Use real LLM dispatch instead of simulated results |
| `--workdir <path>` | Working directory |

```
roko bench demo
roko bench demo --real
```

---

### `roko bench swe`

Run a native SWE-bench-style proxy batch evaluation.

```
roko bench swe [--dataset <path>] [--batch-size <n>] [--offset <n>]
               [--agent-mode <mode>] [--predictions <path>] [--agent-command <cmd>]
               [--report <path>] [--export-predictions <path>]
               [--no-learning] [--keep-workdirs] [--workdir <path>]
```

| Flag | Default | Description |
|---|---|---|
| `--dataset <path>` | — | Local JSONL dataset (defaults to built-in 2-task smoke dataset) |
| `--batch-size <n>` | `2` | Number of instances to run |
| `--offset <n>` | `0` | Offset into the dataset |
| `--agent-mode <mode>` | `gold` | Agent adapter: `gold`, `prediction-file`, `command` |
| `--predictions <path>` | — | Predictions JSONL path (for `--agent-mode prediction-file`) |
| `--agent-command <cmd>` | — | Command for `--agent-mode command` (receives instance JSON on stdin, prints unified diff) |
| `--report <path>` | — | Scores JSONL output path |
| `--export-predictions <path>` | — | Write SWE-bench-style predictions JSONL |
| `--no-learning` | false | Disable learning episode, efficiency, and C-factor writes |
| `--keep-workdirs` | false | Keep per-instance benchmark workdirs for debugging |
| `--workdir <path>` | — | Working directory |

Note: This is fast proxy scoring, not official SWE-bench Docker scoring.

```
roko bench swe --batch-size 2 --agent-mode gold
roko bench swe --dataset ./swe-smoke.jsonl --predictions ./predictions.jsonl --agent-mode prediction-file
roko bench swe --agent-mode command --agent-command './my-agent.sh'
```

---

## 14. Vision Loop

### `roko vision-loop`

Iterative vision-guided UI refinement loop. Screenshots a URL, scores the result
with a vision model, edits the target file, and repeats until the target score
is reached or the iteration limit is hit.

```
roko vision-loop <target_file> --goal <text> --url <url>
                 [--max-iter <n>] [--target-score <f>] [--consecutive-target <n>]
                 [--regression-threshold <f>] [--model <key>]
                 [--viewport-width <n>] [--viewport-height <n>] [--wait-ms <n>]
```

| Flag | Default | Description |
|---|---|---|
| `target_file` | required | Source file to iterate on (e.g. `src/pages/Home.tsx`) |
| `--goal <text>` | required | What the UI should look/feel like |
| `--url <url>` | required | URL to screenshot (e.g. `http://localhost:5173`) |
| `--max-iter <n>` | `10` | Maximum iterations |
| `--target-score <f>` | `9.0` | Score threshold (1–10) for early stopping |
| `--consecutive-target <n>` | `2` | Consecutive target hits before stopping |
| `--regression-threshold <f>` | `3.0` | Score drop from peak that triggers rollback |
| `--model <key>` | auto-detect | Vision model key from `roko.toml` |
| `--viewport-width <n>` | `1280` | Viewport width in pixels |
| `--viewport-height <n>` | `720` | Viewport height in pixels |
| `--wait-ms <n>` | `2000` | Milliseconds to wait after writing (HMR settle time) |

```
roko vision-loop src/pages/Home.tsx --goal "Clean minimalist design" --url http://localhost:5173
```

---

## 15. Demo

### `roko demo setup`

Build the release binary and prepare the workspace for demos.

```
roko demo setup [--workdir <path>]
```

---

### `roko demo warm`

Pre-warm the LLM response cache with demo prompts.

```
roko demo warm [--workdir <path>]
```

---

## 16. Miscellaneous

### `roko layer-check`

Check workspace layer dependency rules (ensures crates do not violate the
layered architecture — e.g. core crates must not depend on higher-level crates).

```
roko layer-check
```

---

## Appendix A: Environment Variables Reference

| Variable | Description |
|---|---|
| `RUST_LOG` | Tracing log directive (e.g. `roko=debug,roko_agent=trace`) |
| `ROKO_LOG` | Alias for `RUST_LOG` |
| `ROKO_TIMING=1` | Enable elapsed-time output (same as `--timing`) |
| `ROKO_LOG_RAW=1` | Disable secret redaction in log output |
| `ROKO_API_KEY` | API key for roko-serve authentication |
| `ANTHROPIC_API_KEY` | Claude API key (for direct API agents) |
| `GITHUB_TOKEN` | GitHub personal access token (MCP GitHub server) |
| `GITHUB_WEBHOOK_SECRET` | GitHub webhook secret for deploy registration |
| `SLACK_BOT_TOKEN` | Slack bot token (MCP Slack server) |
| `SLACK_SIGNING_SECRET` | Slack webhook signing secret |
| `PORT` | Port override for `roko worker` |
| `NUNCHI_DASHBOARD_URL` | Dashboard URL for browser auth (overrides `--dashboard-url`) |
| `NO_COLOR` | Disable ANSI colors (https://no-color.org/) |
| `CLICOLOR` | Set to `0` to disable colors |
| `CLICOLOR_FORCE` | Set to non-`"0"` to force colors |

---

## Appendix B: Data Directory Layout

After `roko init`, the `.roko/` directory has this structure:

```
.roko/
  roko.toml              # Project config (at project root, not inside .roko/)
  engrams.jsonl          # Signal/engram log
  episodes.jsonl         # Agent turn recording
  serve-tui.log          # TUI mode tracing output

  state/
    executor.json        # Plan executor snapshot (for --resume-plan)

  prd/
    ideas/               # Captured ideas (roko prd idea)
    drafts/              # Draft PRDs
    published/           # Published PRDs

  plans/                 # Auto-generated plans (roko prd plan)

  research/              # Research artifacts (roko research topic)

  learn/
    cascade-router.json  # CascadeRouter model selection state
    experiments.json     # Prompt A/B experiment state
    efficiency.jsonl     # Per-turn efficiency events
    gate-thresholds.json # Adaptive gate threshold EMA state
    neuro.jsonl          # Durable knowledge store
    cfactor.json         # C-Factor snapshot

  agents/
    <name>/
      manifest.toml      # Agent manifest

  jobs/
    <uuid>.json          # Marketplace job records

  subscriptions/         # Event subscription configs

  templates/             # Agent template registry

  task-outputs/          # Per-task agent output captures

  acp.log                # ACP server log
```

---

## Appendix C: Self-Hosting Workflow

The canonical workflow for roko developing itself:

```bash
# 1. Capture a work item
roko prd idea "Wire SystemPromptBuilder into orchestrate.rs"

# 2. Draft a PRD (agent-driven, produces ## Repository Grounding section)
roko prd draft new "system-prompt-wiring"

# 3. Research for context
roko research enhance-prd system-prompt-wiring

# 4. Generate implementation plan
roko prd plan system-prompt-wiring

# 5. Validate the plan (also runs automatically before roko plan run)
roko plan validate plans/

# 6. Execute the plan
roko plan run plans/

# 7. Resume if interrupted
roko plan run plans/ --resume-plan

# 8. Monitor progress
roko dashboard

# 9. Check status
roko status

# 10. Inspect what was learned
roko learn all
```
