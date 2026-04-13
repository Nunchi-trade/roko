# CLI reference

Complete reference for the `roko` command-line interface. Commands are grouped by category. Every command supports these global flags:

```
--config <path>     Override the config file (default: ./roko.toml)
--role <name>       Set the agent role / persona
--model <name>      Set the model name
--repo <path>       Set the repository / working directory root
--resume <id>       Resume a previous session by ID
--effort <level>    Reasoning effort: low, medium, high, max
--json              Emit JSON output instead of human-readable text
--log-format <fmt>  Tracing format: text (default), json
--quiet             Suppress non-essential output
--no-replan         Disable re-planning; gate failures become terminal
--headless          Run as a headless daemon
```

Environment variables:
- `ROKO_LOG` -- tracing filter (default: `info`)
- `ROKO_LOG_RAW=1` -- disable secret redaction in logs

---

## Core commands

### roko init

Create a `.roko/` directory and default `roko.toml` in the target path.

```
roko init [path] [--cloud]
```

| Flag | Description |
|------|-------------|
| `path` | Directory to initialize (default: current directory) |
| `--cloud` | Generate cloud-ready defaults for deployment |

Detects the project type (Rust, TypeScript, Go), sets appropriate gates, and picks a default model. Creates `.roko/` with subdirectories for signals, episodes, state, PRDs, and research.

**Examples:**

```bash
roko init                     # initialize current directory
roko init ~/projects/my-api   # initialize a specific path
roko init --cloud             # cloud-ready defaults (worker mode)
```

---

### roko run

Execute a prompt through the full universal loop: compose, agent, gate, persist.

```
roko run "<prompt>" [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `prompt` | The user prompt text (required) |
| `--workdir` | Override the working directory (default: cwd) |

Assembles a token-budgeted prompt from project context, dispatches to the configured LLM agent, runs the output through the gate pipeline, and persists all signals. On gate failure, retries with model escalation up to `agent.escalation.max_retries`.

**Examples:**

```bash
roko run "add a health check endpoint to the API"
roko run "refactor auth to use JWT" --workdir ~/projects/api
roko run "fix the failing test in user_service.rs" --model claude-opus-4-6
```

---

### roko status

Print signal counts, most recent episode, and gate pass/fail summary.

```
roko status [--workdir <path>] [--cfactor]
```

| Flag | Description |
|------|-------------|
| `--workdir` | Directory containing `.roko/` (default: cwd) |
| `--cfactor` | Compute and persist the latest C-Factor snapshot |

**Examples:**

```bash
roko status
roko status --cfactor     # include confidence factor computation
roko status --json        # machine-readable output
```

---

### roko replay

Walk the lineage DAG rooted at a signal hash and print it.

```
roko replay <hash> [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `hash` | Signal hash (64 hex chars) to walk |
| `--workdir` | Directory containing `.roko/` (default: cwd) |

**Example:**

```bash
roko replay a3f8c1...
```

---

### roko dashboard

Launch the interactive terminal UI or render a text-mode dashboard.

```
roko dashboard [--page <slug>] [--list-pages] [--text] [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `--page` | Render a specific dashboard page slug |
| `--list-pages` | List all available page slugs |
| `--text` | Force text-mode output instead of interactive TUI |
| `--workdir` | Override the working directory |

The interactive TUI uses F1-F7 for tab navigation (Dashboard, Plans, Agents, Git, Logs, Config, Inspect). Press `q` to quit, `?` for help.

**Examples:**

```bash
roko dashboard                    # interactive TUI
roko dashboard --text             # text-mode fallback
roko dashboard --page efficiency  # render one page
roko dashboard --list-pages       # show available pages
```

---

## Plan commands

### roko plan list

List all plans in the workspace.

```
roko plan list [--workdir <path>]
```

---

### roko plan show

Show details of a specific plan.

```
roko plan show <plan_id> [--workdir <path>]
```

---

### roko plan create

Create a new plan.

```
roko plan create <plan_id> --title "<title>" [--description "<desc>"] [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `plan_id` | Plan identifier |
| `--title` | Plan title (required) |
| `--description` | Plan description (default: empty) |

**Example:**

```bash
roko plan create oauth2-impl --title "Implement OAuth2 authentication"
```

---

### roko plan validate

Validate a plan directory for modern `tasks.toml` fields.

```
roko plan validate <plan_dir>
```

Checks that the plan's `tasks.toml` uses the current schema with required fields (complexity, tier, role, test hints).

---

### roko plan run

Execute a plan directory through the orchestration loop.

```
roko plan run <plans_dir> [--workdir <path>] [--resume-plan [<path>]]
```

| Flag | Description |
|------|-------------|
| `plans_dir` | Path to the plans directory |
| `--workdir` | Working directory / repo root (default: cwd) |
| `--resume-plan` | Resume from state file (default: `.roko/state/executor.json`) |

This is the main orchestration command. It discovers plans, builds a task DAG, dispatches agents in parallel where possible, runs gates on each task's output, and persists state for resumption.

**Examples:**

```bash
roko plan run plans/
roko plan run plans/ --resume-plan
roko plan run plans/ --resume-plan .roko/state/executor.json
```

---

### roko plan generate

Generate implementation plans from a prompt, file, or PRD.

```
roko plan generate <source...> [--from-file <path>]
```

| Flag | Description |
|------|-------------|
| `source` | Free-text prompt or inline source |
| `--from-file` | Treat source as a file path to read |

**Examples:**

```bash
roko plan generate "Build a REST API with auth and rate limiting"
roko plan generate --from-file requirements.md
```

---

### roko plan regenerate

Regenerate an existing plan from its source PRD or plan extract.

```
roko plan regenerate <plan_dir> [--dry-run]
```

| Flag | Description |
|------|-------------|
| `plan_dir` | Path to the plan directory (containing tasks.toml) |
| `--dry-run` | Preview changes without overwriting |

---

## PRD commands

### roko prd idea

Capture a quick idea as a work item.

```
roko prd idea "<text>"
```

Appends the idea to `.roko/prd/ideas/`. Ideas are raw inputs that get refined into full PRDs via `roko prd draft`.

**Example:**

```bash
roko prd idea "Add WebSocket support for real-time updates"
```

---

### roko prd list

List all PRDs (published, drafts, and ideas).

```
roko prd list
```

---

### roko prd status

Show coverage report across PRDs and plans (plans/tasks/done ratio).

```
roko prd status
```

---

### roko prd draft new

Create a new draft PRD with agent assistance.

```
roko prd draft new "<title>"
```

The agent generates a structured PRD from the title, including problem statement, proposed solution, success criteria, and implementation notes.

**Example:**

```bash
roko prd draft new "OAuth2 authentication with Google and GitHub"
```

---

### roko prd draft edit

Refine an existing draft PRD.

```
roko prd draft edit <slug>
```

---

### roko prd draft promote

Promote a draft to published status.

```
roko prd draft promote <slug> [--auto-execute]
```

| Flag | Description |
|------|-------------|
| `--auto-execute` | Execute the generated plan immediately after promotion |

---

### roko prd draft list

List all draft PRDs.

```
roko prd draft list
```

---

### roko prd plan

Generate implementation plans from a published PRD.

```
roko prd plan <slug> [--dry-run]
```

| Flag | Description |
|------|-------------|
| `slug` | PRD slug (filename without .md) |
| `--dry-run` | Preview generation without writing tasks.toml files |

**Example:**

```bash
roko prd plan oauth2-auth
roko prd plan oauth2-auth --dry-run
```

---

### roko prd consolidate

Scan all PRDs for duplicates, gaps, and inconsistencies.

```
roko prd consolidate
```

---

## Research commands

### roko research topic

Deep-dive research on a topic. Produces `.roko/research/<slug>.md` with citations.

```
roko research topic "<topic>" [--deep]
```

| Flag | Description |
|------|-------------|
| `--deep` | Use Perplexity deep research (async, takes 1-10 minutes) |

**Examples:**

```bash
roko research topic "OAuth2 best practices in Rust"
roko research topic "Zero-knowledge proof systems" --deep
```

---

### roko research enhance-prd

Enhance a PRD with academic citations, diagrams, and research-backed improvements.

```
roko research enhance-prd <slug>
```

---

### roko research enhance-plan

Optimize an implementation plan with research-backed task decomposition.

```
roko research enhance-plan <plan>
```

---

### roko research enhance-tasks

Optimize tasks for efficiency, parallelism, and cheapest viable model.

```
roko research enhance-tasks <plan>
```

---

### roko research analyze

Analyze execution episodes for self-learning insights and bandit weight recommendations.

```
roko research analyze
```

---

### roko research list

List all research artifacts.

```
roko research list
```

---

### roko research search

Direct web search using Perplexity's search API. Returns raw results without synthesis.

```
roko research search "<query>" [--domains <list>] [--recency <filter>]
```

| Flag | Description |
|------|-------------|
| `--domains` | Restrict results to these domains (comma-separated) |
| `--recency` | Recency filter: `day`, `week`, `month`, `year` |

**Examples:**

```bash
roko research search "Rust async patterns 2026"
roko research search "axum middleware" --domains docs.rs,github.com
roko research search "LLM agent architectures" --recency month
```

---

## Knowledge commands (neuro)

### roko neuro query

Query the durable knowledge store for a topic.

```
roko neuro query "<topic>" [--workdir <path>]
```

Returns matching knowledge entries (facts, insights, heuristics, procedures, constraints, anti-knowledge) ranked by relevance and recency.

**Example:**

```bash
roko neuro query "authentication patterns"
```

---

### roko neuro stats

Show aggregate statistics for the durable knowledge store.

```
roko neuro stats [--workdir <path>]
```

Reports entry counts by kind, average confidence scores, and storage size.

---

### roko neuro gc

Run garbage collection on the durable knowledge store.

```
roko neuro gc [--workdir <path>]
```

Removes entries below the minimum confidence threshold (default: 0.1).

---

## Provider commands

### roko provider list

List configured providers and their current connection status.

```
roko provider list [--workdir <path>]
```

Shows each provider's kind, endpoint, and whether it is reachable.

---

### roko provider health

Show persisted provider circuit-breaker health and latency.

```
roko provider health [--workdir <path>]
```

Displays circuit breaker state (closed, half-open, open), rolling latency EMAs, and recent error counts for each provider.

---

### roko provider test

Send a minimal request to verify provider connectivity.

```
roko provider test <provider> [--workdir <path>]
```

**Example:**

```bash
roko provider test gemini
roko provider test claude_cli
```

---

## Model commands

### roko model list

List configured models and their capabilities.

```
roko model list [--workdir <path>]
```

Shows each model's provider, context window, output limit, tool support, and cost per million tokens.

---

### roko model route

Show the current routing decision for a model.

```
roko model route <model> [--explain] [--complexity <tier>] [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `model` | Model key or slug to explain |
| `--explain` | Show the full routing trace |
| `--complexity` | Complexity tier: `mechanical`, `focused`, `integrative`, `architectural` |

**Example:**

```bash
roko model route claude-sonnet-4-6 --explain --complexity focused
```

---

## Experiment commands

### roko experiment model create

Create a new model A/B experiment.

```
roko experiment model create --id <id> --role <role> --variant <spec>... [--min-trials <n>]
```

| Flag | Description |
|------|-------------|
| `--id` | Experiment identifier |
| `--role` | Scope the experiment to a specific agent role |
| `--variant` | Variant in `id:slug:provider` form (at least 2 required) |
| `--min-trials` | Minimum trials per variant before concluding (default: 20) |

**Example:**

```bash
roko experiment model create \
  --id sonnet-vs-gemini \
  --role implementer \
  --variant sonnet:claude-sonnet-4-6:claude_cli \
  --variant flash:gemini-2-5-flash:gemini \
  --min-trials 30
```

---

### roko experiment model show

Show a model experiment's results.

```
roko experiment model show <id>
```

---

### roko experiment model list

List all model experiments.

```
roko experiment model list
```

---

## Config commands

### roko config init

Interactive wizard that detects installed LLM CLIs and writes a global config.

```
roko config init [options]
```

| Flag | Description |
|------|-------------|
| `--yes` | Skip confirmation prompts |
| `--agent <cmd>` | Pre-select agent command |
| `--model <name>` | Pre-set model name |
| `--budget <n>` | Pre-set token budget |
| `--role <text>` | Pre-set role string |
| `--enable-gates` | Enable default compile + clippy gates |
| `--path <path>` | Write to this path instead of global default |
| `--non-interactive` | Skip all prompts; fail if any answer is missing |

**Examples:**

```bash
roko config init                          # interactive wizard
roko config init --agent claude --yes     # non-interactive with Claude
roko config init --agent ollama --model llama3.2 --yes
```

---

### roko config show

Print the effective merged config with per-field source tags.

```
roko config show [--workdir <path>]
```

Shows which values come from global config, project config, or environment variables.

---

### roko config path

Print the resolved global and project config paths.

```
roko config path [--workdir <path>]
```

---

### roko config edit

Open `$EDITOR` on the chosen config file.

```
roko config edit [--global | --project] [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `--global` | Open the global config file |
| `--project` | Open or create the project `roko.toml` |

---

### roko config set

Set a dotted key in the chosen config layer.

```
roko config set <key> <value> [--global | --project] [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `key` | Dotted key path (e.g., `agent.model`) |
| `value` | Value to write |
| `--global` | Write to global config (default) |
| `--project` | Write to project config |

**Examples:**

```bash
roko config set agent.command ollama --global
roko config set agent.model claude-opus-4-6 --project
roko config set budget.max_plan_usd 25.0 --project
```

---

### roko config set-secret

Store a secret in `~/.roko/.env`.

```
roko config set-secret <name> <value>
```

**Example:**

```bash
roko config set-secret GEMINI_API_KEY sk-abc123...
```

---

### roko config check-secrets

Check `${VAR}` references in config and validate referenced secrets exist.

```
roko config check-secrets [--workdir <path>]
```

---

### roko config validate

Validate `roko.toml` syntax, schema, and semantic references.

```
roko config validate [--workdir <path>]
```

Checks for valid TOML syntax, correct schema version, valid provider/model references, and reachable endpoints.

---

### roko config migrate

Migrate a legacy project `roko.toml` into explicit provider/model tables.

```
roko config migrate [--workdir <path>] [--dry-run]
```

| Flag | Description |
|------|-------------|
| `--dry-run` | Print the proposed migration without writing changes |

Converts legacy `[agent] command = "claude"` format to the new `[providers.*]` and `[models.*]` table format with `schema_version = 2`.

---

## Deploy commands

### roko deploy railway

Deploy the current workspace to Railway via the public GraphQL API.

```
roko deploy railway [--workdir <path>]
```

Requires `RAILWAY_TOKEN` environment variable.

---

### roko deploy fly

Generate `fly.toml` and deploy the current workspace with Fly.io.

```
roko deploy fly [--workdir <path>]
```

Requires the `fly` CLI to be installed.

---

### roko deploy docker

Build the local Docker image and tag it for the configured registry.

```
roko deploy docker [--workdir <path>] [--registry <namespace>]
```

| Flag | Description |
|------|-------------|
| `--registry` | Registry namespace to tag the image under |

---

## Daemon commands

### roko daemon start

Start the background daemon.

```
roko daemon start [--foreground] [--port <n>]
```

| Flag | Description |
|------|-------------|
| `--foreground` | Run in foreground instead of daemonizing |
| `--port` | Port for the daemon's HTTP API (default: 9090) |

The daemon watches for file changes, processes cron-scheduled tasks, ingests webhooks, and dispatches agents from event subscriptions.

---

### roko daemon stop

Gracefully stop the running daemon.

```
roko daemon stop
```

---

### roko daemon status

Check whether the daemon is running.

```
roko daemon status
```

---

### roko daemon logs

View daemon logs.

```
roko daemon logs [-f] [-n <lines>]
```

| Flag | Description |
|------|-------------|
| `-f`, `--follow` | Follow log output (like `tail -f`) |
| `-n`, `--lines` | Number of lines to show (default: 50) |

---

### roko daemon reload

Reload configuration without restarting. Re-scans subscriptions and templates.

```
roko daemon reload
```

---

### roko daemon restart

Stop and restart the daemon.

```
roko daemon restart [--port <n>]
```

---

### roko daemon install

Install as a macOS launchd service for automatic startup.

```
roko daemon install
```

Generates a launchd plist and loads it.

---

### roko daemon uninstall

Remove the launchd plist and unload the service.

```
roko daemon uninstall
```

---

## Dream commands

### roko dream run

Run a dream consolidation cycle immediately.

```
roko dream run [--workdir <path>]
```

Batches completed episodes, clusters them by task shape, distills durable knowledge, and promotes reliable success patterns into playbooks.

---

### roko dream report

Show the latest dream report without running a new cycle.

```
roko dream report [--workdir <path>]
```

---

### roko dream schedule

Show when the next dream cycle should fire.

```
roko dream schedule [--workdir <path>]
```

---

## Server commands

### roko serve

Start the HTTP API server.

```
roko serve [--bind <addr>] [--port <n>] [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `--bind` | Address to bind to (default: 127.0.0.1) |
| `--port` | Port number (default: 9090) |
| `--workdir` | Working directory (default: cwd) |

Exposes REST endpoints for plans, PRDs, agents, config, learning data, templates, webhooks, research, deployments, provider health, and subscriptions. Includes SSE and WebSocket for real-time streaming.

---

### roko worker

Run as a deployed cloud worker.

```
roko worker [--port <n>]
```

| Flag | Description |
|------|-------------|
| `--port` | Port to listen on (default: 8080, overridden by `PORT` env) |

Reads a template from environment variables and serves tasks. Designed for Railway, Fly.io, and container platforms.

---

## Event and subscription commands

### roko subscription list

List all event subscriptions.

```
roko subscription list
```

---

### roko subscription add

Create a new event subscription.

```
roko subscription add --template <name> --trigger <glob>
```

| Flag | Description |
|------|-------------|
| `--template` | Agent template name to invoke |
| `--trigger` | Signal trigger glob to match |

**Example:**

```bash
roko subscription add --template code-review --trigger "signal:gate:fail:*"
```

---

### roko subscription remove

Delete a subscription.

```
roko subscription remove <id>
```

---

### roko subscription enable

Enable a disabled subscription.

```
roko subscription enable <id>
```

---

### roko subscription disable

Disable a subscription without deleting it.

```
roko subscription disable <id>
```

---

### roko event-sources list

List configured cron schedules and file watchers.

```
roko event-sources list [--workdir <path>]
```

Shows all `[[scheduler.cron]]` entries and `[[watcher.paths]]` entries from the config.

---

## Other commands

### roko inject

Inject a signal into a running session.

```
roko inject <session> <payload> [--kind <type>] [--workdir <path>]
```

| Flag | Description |
|------|-------------|
| `session` | Target session ID |
| `payload` | Payload text |
| `--kind` | Signal kind: `directive`, `abort`, `context` (default: directive) |

**Example:**

```bash
roko inject session-123 "stop working on auth, switch to logging" --kind directive
roko inject session-123 "stop" --kind abort
```

---

## Implicit modes

When no subcommand is given, roko selects a mode based on context:

| Condition | Mode | Behavior |
|-----------|------|----------|
| Positional `<prompt>` argument | One-shot | Same as `roko run "<prompt>"` |
| `--headless` flag | Daemon | Run as headless background service |
| stdin is not a TTY | Pipe | Read prompt from stdin, execute, exit |
| stdin is a TTY, no prompt | REPL | Interactive prompt loop |

**Examples:**

```bash
roko "fix the bug in auth.rs"                  # one-shot
echo "add logging" | roko                      # pipe mode
roko --headless                                # daemon mode
roko                                           # REPL mode
```

---

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Agent or gate failure (logical error) |
| 2 | System error (I/O, config, infrastructure) |

---

## Configuration files

| File | Purpose |
|------|---------|
| `~/.config/roko/config.toml` | Global config (agent defaults, model preferences) |
| `./roko.toml` | Project config (gates, prompts, providers, models) |
| `~/.roko/.env` | Secrets (`GEMINI_API_KEY`, `PERPLEXITY_API_KEY`, etc.) |
| `.roko/state/executor.json` | Executor state snapshot for `--resume-plan` |
| `.roko/signals.jsonl` | Append-only signal log |
| `.roko/episodes.jsonl` | Episode log (agent turn records) |
| `.roko/learn/` | Learning data (cascade router, experiments, thresholds, efficiency) |
| `.roko/prd/` | PRD storage (ideas, drafts, published) |
| `.roko/research/` | Research artifacts |
