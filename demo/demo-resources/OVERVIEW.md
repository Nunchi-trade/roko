# Demo Resources

Scripts and automations for validating and demoing roko-serve, agent
matchmaking, PRDs, research, benchmark telemetry, the dashboard integration,
and the self-hosting loop.

## Quick start

```bash
# Check prerequisites and build
demo/demo-resources/bin/roko-demo doctor
demo/demo-resources/bin/roko-demo build

# Prove the dashboard integration in a disposable workspace
demo/demo-resources/bin/roko-demo verify-local

# Or run reusable checks against an existing serve
bash demo/demo-resources/run-all.sh
```

See `AUTOMATION.md` for command details, environment variables, and smoke-test
coverage.

## Automation scripts

| Script | What | Time | Exit |
|--------|------|------|------|
| `bin/roko-demo` | Main reusable runner for build, serve, seed, smoke, and wrapped workflows | varies | 0/1 |
| `run-all.sh` | Non-interactive reusable checks against an existing serve | ~10s | 0/1 |
| `smoke-test.sh` | Compatibility entrypoint for `bin/roko-smoke.sh` | ~10s | 0/1 |
| `bin/roko-up.sh` | Start roko-serve, wait for health, seed through `bin/roko-demo` | ~10s | 0/1 |
| `bin/roko-down.sh` | Stop roko-serve started by `bin/roko-up.sh` | ~2s | 0/1 |
| `bin/roko-smoke.sh` | Dashboard API smoke plus basic CLI checks | ~10s | 0/1 |
| `bin/roko-demo.sh` | Compatibility wrapper for old workflow names | varies | 0/1 |
| `agent-matchmaking/e2e-test.sh` | 40 automated checks (registration, match, lifecycle, edge cases) | ~15s | 0/1 |
| `agent-matchmaking/seed-agents.sh` | Register 5 demo agents (rustsmith, ethdev, fullstack, researcher, auditor) | ~2s | 0/1 |
| `benchmark-flow/demo-benchmark.sh` | SWE-bench proxy controls plus C-factor/learning telemetry proof | ~10s | 0/1 |

Reusable scripts accept an optional base URL argument, defaulting to
`http://127.0.0.1:6677` or `ROKO_SERVE_URL`.

## Workflow directories

| Dir | What | Mode |
|-----|------|------|
| `agent-matchmaking/` | Agent registration, skill matching, job lifecycle, e2e tests | API + CLI |
| `agent-setup/` | Agent creation, tool config, fleet registration | CLI + API |
| `agent-workflows/` | Agent sidecar start/stop, multi-agent, chat REPL | CLI (live processes) |
| `prd-workflow/` | Idea capture, PRD listing, status, plan generation | CLI + API |
| `research-workflow/` | Research dispatch, artifact listing, PRD enhancement | CLI + API |
| `full-self-hosting/` | End-to-end: capture → jobs → match → observe | CLI + API |
| `benchmark-flow/` | Native SWE-bench proxy scoring, prediction export, episodes, efficiency, C-factor | CLI |
| `dashboard-quickstart/` | Setup guide for nunchi-dashboard + roko-serve | Docs only |
| `bin/` | Shared helpers and reusable command wrappers | CLI + API |

## Interactive demo scripts

These have `pause()` calls for live walkthroughs (press Enter between steps):

| Script | What it shows |
|--------|---------------|
| `agent-matchmaking/demo-match.sh` | 6 matchmaking queries with formatted output |
| `agent-matchmaking/demo-lifecycle.sh` | Job state machine: match → create → assign → start → submit → evaluate |
| `prd-workflow/demo-prd-cli.sh` | Ideas, PRD list, status, job creation via CLI |
| `prd-workflow/demo-prd-api.sh` | Same flow via HTTP API |
| `research-workflow/demo-research.sh` | Research dispatch + ideas + jobs |
| `full-self-hosting/demo-full-loop.sh` | All 4 acts: capture, jobs, match, system state |
| `agent-setup/setup-fleet.sh` | Create 3 agents + register for matchmaking |
| `benchmark-flow/demo-benchmark.sh` | Gold/empty/command benchmark controls and C-factor proof |

## Benchmark flow

The benchmark flow does not require `roko serve`; it only needs the `roko`
binary, Python 3, and git:

```bash
bash demo/demo-resources/benchmark-flow/demo-benchmark.sh
bash demo/demo-resources/bin/roko-demo run bench
```

It runs a positive control, a negative control, and a command-adapter control
against the built-in SWE-bench proxy dataset. It writes aggregate score rows,
prediction exports, learning episodes, efficiency events, and C-factor
snapshots to a reusable workspace. See
`benchmark-flow/README.md` for the dataset format and real-agent command
adapter contract.

## Ollama configuration

Ollama models are configured in `roko.toml` at the project root:

```toml
[providers.ollama]
kind = "openai_compat"
base_url = "http://localhost:11434/v1"
timeout_ms = 180000

[models.llama32]
provider = "ollama"
slug = "llama3.2:latest"
context_window = 8192
supports_tools = true
tool_format = "openai_json"

[models.gemma4]
provider = "ollama"
slug = "gemma4:26b-a4b-it-q8_0"
context_window = 8192
supports_tools = true
tool_format = "openai_json"

[agent.tier_models]
fast = "ollama/llama3.2:latest"
standard = "ollama/gemma4:26b-a4b-it-q8_0"
```

After editing, restart `roko serve` (providers/models are not hot-reloadable).

Verify: `curl -s http://localhost:6677/api/providers | python3 -m json.tool`

## Key HTTP endpoints

### Agents
| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/agents/register` | Register agent with skills/tier/reputation |
| GET | `/api/managed-agents` | List all agents (dashboard fleet view) |
| GET | `/api/agents/{id}` | Agent detail |
| POST | `/api/agents/{id}/message` | Send message via serve proxy |

### Jobs & Matchmaking
| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/jobs/match` | Find candidate agents for a job |
| POST | `/api/jobs` | Create job with committed candidates |
| POST | `/api/jobs/{id}/assign` | Assign to agent |
| POST | `/api/jobs/{id}/start` | Start work |
| POST | `/api/jobs/{id}/submit` | Submit result + artifacts |
| POST | `/api/jobs/{id}/evaluate` | Accept or reject |
| GET | `/api/jobs` | List all jobs |
| GET | `/api/jobs/stats` | Job statistics |

### PRDs & Plans
| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/prds/ideas` | Capture idea |
| GET | `/api/prds` | List PRDs |
| GET | `/api/prds/status` | Coverage report |
| GET | `/api/plans` | List plans |

### Research
| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/research/topic` | Dispatch research |
| GET | `/api/research` | List artifacts |

### System
| Method | Path | Purpose |
|--------|------|---------|
| GET | `/api/health` | Server health |
| GET | `/api/providers` | Provider registry |
| GET | `/api/models` | Model registry |
| GET | `/api/config` | Full runtime config |

## Prerequisites

- Rust 1.91+ (`rustup update stable`)
- `roko` built (`cargo build -p roko-cli`)
- `roko serve` running on port 6677
- Python 3 (for JSON formatting in scripts)
- `curl` only for older one-off scripts; reusable `bin/` smoke and seed paths
  use Python HTTP helpers instead

## Known gaps

1. **Agent creation is CLI-only** — dashboard can list/message agents but not create them
2. **Config editing is CLI-only** — dashboard shows roko.toml read-only (PUT /api/config exists but no UI)
3. **MCP config** lives in roko.toml `[agent]` section as `mcp_config` path, not a dedicated section
4. **Tool profiles** are configured in `[tools.profiles.<domain>]` in roko.toml

## Troubleshooting

| Issue | Fix |
|-------|-----|
| `Connection refused` | Start `roko serve` |
| No agents in match results | Run `demo/demo-resources/bin/roko-demo seed-agents` |
| Ollama provider not showing | Ensure `roko.toml` is at project root (not just `.roko/`), restart serve |
| Provider test 404 | Set `base_url` to `http://localhost:11434/v1` (needs `/v1` suffix) |
| Config changes not applied | Providers/models require restart; only budget/gates/prompt are hot-reloadable |
