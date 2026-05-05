# Roko

Roko is a Rust toolkit for building agents that build themselves. 18 crates, ~177K LOC.

**Goal**: roko develops itself — it reads PRDs, generates implementation plans, executes tasks
via Claude agents, validates with gates, and persists results. The core loop is wired. Your job
is to use it and improve it.

## Current state (2026-04-20)

The plan-execute-gate-persist loop **works end-to-end**, and so do the HTTP
control plane, per-agent sidecar, and interactive TUI:

| Component | Status | Where |
|---|---|---|
| Plan discovery + DAG executor | **Wired** | `crates/roko-cli/src/orchestrate.rs` |
| Agent dispatch (Claude CLI + ExecAgent) | **Wired** | `crates/roko-agent/src/dispatcher/mod.rs` |
| Safety layer (role auth, pre/post checks) | **Wired** | Integrated into ToolDispatcher |
| Gate pipeline (compile, test, clippy, diff) | **Wired** | Called from orchestrate.rs per-task |
| Session persistence (snapshot + resume) | **Wired** | `.roko/state/executor.json`, `--resume` |
| PRD lifecycle (idea/draft/plan) | **Wired** | `roko prd` subcommands |
| Research agent | **Wired** | `roko research` subcommands |
| Plan generation from PRD | **Wired** | `roko prd plan <slug>` → agent generates tasks.toml |
| SystemPromptBuilder (9-layer prompts) | **Wired** | `RoleSystemPromptSpec` in orchestrate.rs |
| EpisodeLogger (agent turn recording) | **Wired** | `.roko/episodes.jsonl` via orchestrate.rs |
| ProcessSupervisor (lifecycle mgmt) | **Wired** | `PlanRunner` tracks + shuts down agents |
| MCP config passthrough | **Wired** | `agent.mcp_config` in roko.toml → `--mcp-config` |
| Efficiency events (per-turn) | **Wired** | `.roko/learn/efficiency.jsonl` via orchestrate.rs |
| CascadeRouter (model routing) | **Wired** | Persists to `.roko/learn/cascade-router.json`, configurable models |
| Prompt experiments (A/B) | **Wired** | `ExperimentStore` in `.roko/learn/experiments.json` |
| Adaptive gate thresholds | **Wired** | EMA per rung in `.roko/learn/gate-thresholds.json` |
| Interactive TUI (ratatui) | **Wired** | `crates/roko-cli/src/tui/`, F1–F7 tabs, `roko dashboard` |
| HTTP control plane (~85 routes) | **Wired** | `crates/roko-serve/src/routes/`, `roko serve` on :6677 |
| Per-agent sidecar (13 routes) | **Wired** | `crates/roko-agent-server/`, real LLM dispatch (T9) + integration tests (T19) |
| Code-intelligence MCP | **Wired** | `crates/roko-mcp-code/` |
| `roko chat` CLI | **Wired** | `crates/roko-cli/src/chat.rs` |
| Gate rung oracles (4-6) | **Wired** | orchestrate.rs `enrich_rung_config` |
| C-factor full metrics | **Wired** | orchestrate.rs `CFactorSummary` |
| Enrichment in dispatch | **Wired** | orchestrate.rs `dispatch_agent_with` |
| Gate failure replan | **Wired** | orchestrate.rs `build_gate_failure_plan_revision` |
| PRD auto-plan trigger | **Wired** | roko-serve `prd_publish_subscriber` |
| HDC fingerprint per-episode | **Wired** | Episode `hdc_fingerprint` field, computed + stored |
| Playbook store queries | **Wired** | Queried at dispatch time → system prompt |
| VCG auction in composition | **Partial** | `vcg_allocate` built + exported but greedy path dominates at runtime |
| Context bidders (Neuro/Task/Research) | **Wired** | `AttentionBidder` variants in orchestrate.rs |
| Safety contracts enforcement | **Partial** | `AgentContract` wired but falls back to permissive default when YAML missing |
| TUI file watcher | **Wired** | `notify::RecommendedWatcher` in `tui/fs_watch.rs` |

### Known blockers

1. **Rustc version**: alloy deps need 1.91+. Run `rustup update stable` before `cargo test`.

## Critical rules

### 1. NEVER reimplement what already exists
Search before writing: `grep -rn 'FunctionName\|StructName' crates/ --include='*.rs' | grep -v target/`
This codebase has duplicate implementations from parallel development. CHECK FIRST.

### 2. WIRE, don't build
The pattern in this codebase is "built but never connected." Before building anything new,
check if existing code just needs to be called from the runtime. If your change isn't visible
via `cargo run -p roko-cli -- <subcommand>`, it's probably wrong.

### 3. Verify before marking done
Run the actual code path. "Code exists" != "feature works". Test via CLI, not just unit tests.

### 4. Log gaps when finishing work
After completing any implementation task, append unfinished items to `.roko/GAPS.md`.
Include: what's missing, why it wasn't done, and what subsystem it affects.
This file is the canonical gap tracker — check it before starting new work.

## Architecture

1 noun (Signal) + 6 verb traits (Substrate, Scorer, Gate, Router, Composer, Policy).
Universal loop: query -> score -> route -> compose -> act -> verify -> write -> react.

## Self-hosting workflow

This is how roko develops itself. Each step is a CLI command that exists today:

```bash
# 1. Capture a work item
cargo run -p roko-cli -- prd idea "Wire SystemPromptBuilder into orchestrate.rs"

# 2. Draft a PRD from the idea (agent-driven)
cargo run -p roko-cli -- prd draft new "system-prompt-wiring"

# 3. Research the topic for context
cargo run -p roko-cli -- research enhance-prd system-prompt-wiring

# 4. Generate implementation plan + tasks from the PRD
cargo run -p roko-cli -- prd plan system-prompt-wiring

# 5. Execute the plan (agents run tasks, gates validate, state persists)
cargo run -p roko-cli -- plan run plans/

# 6. Resume if interrupted
cargo run -p roko-cli -- plan run plans/ --resume .roko/state/executor.json

# 7. Watch progress
cargo run -p roko-cli -- dashboard

# 8. Check status
cargo run -p roko-cli -- status
```

## CLI commands reference

### Core workflow
| Command | What it does |
|---|---|
| `roko init` | Create `.roko/` directory and `roko.toml` |
| `roko run "<prompt>"` | Single prompt -> universal loop (compose->agent->gate->persist) |
| `roko status` | Query signals, report counts and episodes |
| `roko doctor` | Diagnose workspace bootstrap state |

### Planning & PRDs
| Command | What it does |
|---|---|
| `roko plan list/show/create` | Manage plans |
| `roko plan run <dir>` | Execute plans (the main orchestration loop) |
| `roko plan generate/regenerate` | Generate or regenerate plans from prompts/PRDs |
| `roko plan validate <dir>` | Lint tasks.toml without executing |
| `roko prd idea "<text>"` | Capture a work item idea |
| `roko prd list/status` | List PRDs, coverage report |
| `roko prd draft new/edit/promote/list` | Draft lifecycle |
| `roko prd plan <slug>` | Generate implementation plan from PRD |
| `roko prd consolidate` | Scan PRDs for gaps and duplicates |

### Agents
| Command | What it does |
|---|---|
| `roko agent create --name X --domain Y` | Create agent from manifest |
| `roko agent start --name X` | Start a long-running agent |
| `roko agent stop --name X` | Stop a running agent |
| `roko agent list` | List agents with status |
| `roko agent status --name X` | Detailed agent health |
| `roko agent serve` | Start per-agent HTTP sidecar |
| `roko agent chat --agent X` | Interactive chat REPL with an agent |

### Research
| Command | What it does |
|---|---|
| `roko research topic "<topic>"` | Deep research with citations |
| `roko research search "<query>"` | Direct web search (Perplexity) |
| `roko research enhance-prd/plan/tasks` | Enhance documents with research |
| `roko research analyze` | Analyze execution data |

### Knowledge (neuro + dreams + custody + archive)
| Command | What it does |
|---|---|
| `roko knowledge query "<topic>"` | Search durable knowledge store |
| `roko knowledge stats/gc` | Store statistics, garbage collection |
| `roko knowledge backup/restore` | Backup with genomic bottleneck, restore with decay |
| `roko knowledge sync <peer>` | Mesh knowledge sync |
| `roko knowledge dream run/report/schedule` | Dream consolidation cycle |
| `roko knowledge dream journal/archive` | Dream journal and archive entries |
| `roko knowledge custody list/show/verify` | Custody audit chain |
| `roko knowledge archive` | Cold storage archival |

### Learning & feedback
| Command | What it does |
|---|---|
| `roko learn all/router/experiments/efficiency/episodes` | Inspect learning state |
| `roko learn tune gates/routing/budget` | Tune adaptive thresholds |

### Jobs
| Command | What it does |
|---|---|
| `roko job list/create/show/execute/cancel` | Manage marketplace jobs |

### Configuration
| Command | What it does |
|---|---|
| `roko config init/show/path/edit/set` | Core config management |
| `roko config validate/migrate` | Schema validation, legacy migration |
| `roko config set-secret/check-secrets` | Secret management |
| `roko config providers list/health/test` | LLM provider inspection |
| `roko config models list/route` | Model inspection and routing |
| `roko config subscriptions list/add/remove` | Event subscriptions |
| `roko config events` | Configured event sources |
| `roko config experiments` | Model A/B experiments |
| `roko config plugins list/install/remove/audit` | Plugin management |
| `roko config secrets set/get/list/rotate` | Profile-aware secrets |

### Server & deployment
| Command | What it does |
|---|---|
| `roko serve` | Start HTTP control plane (~85 routes on :6677) |
| `roko daemon start/stop/status/logs/install` | Daemon lifecycle |
| `roko deploy railway/fly/docker` | Cloud deployment |
| `roko worker` | Run as deployed worker |

### Utilities
| Command | What it does |
|---|---|
| `roko dashboard` | Interactive ratatui TUI (F1–F7 tabs) |
| `roko replay <hash>` | Walk signal DAG by hash |
| `roko inject <session> <payload>` | Signal injection |
| `roko index build/search/stats` | Code intelligence index |
| `roko new <type> <name>` | Scaffold boilerplate |
| `roko explain <topic>` | Concept explainer (3 depth levels) |
| `roko completions <shell>` | Shell completion scripts |

## Key crates

| Crate | Path | What | Status |
|---|---|---|---|
| roko-core | `crates/roko-core/` | Signal + 6 traits, types, config, tools, errors | Kernel, stable |
| roko-agent | `crates/roko-agent/` | 5+ LLM backends (Claude CLI, Claude API, Codex, Cursor, OpenAI-compat, Ollama, Gemini, Perplexity), pools, MCP, tool loop, safety | Dispatch wired, MCP passed |
| roko-agent-server | `crates/roko-agent-server/` | Per-agent HTTP sidecar: `/message` (real LLM dispatch), `/stream` WS, `/predictions`, `/research`, `/tasks` | Wired |
| roko-serve | `crates/roko-serve/` | HTTP control plane: ~85 REST routes + SSE + WebSocket on :6677 | Wired |
| roko-orchestrator | `crates/roko-orchestrator/` | Plan DAG, parallel executor, merge queue, safety | Wired via orchestrate.rs |
| roko-gate | `crates/roko-gate/` | 11 gates, 7-rung pipeline, adaptive thresholds | Wired, called per-task |
| roko-compose | `crates/roko-compose/` | Prompt assembly, 9 templates, enrichment | Wired via RoleSystemPromptSpec |
| roko-conductor | `crates/roko-conductor/` | 10 watchers, circuit breaker, diagnosis | Used by executor internals |
| roko-learn | `crates/roko-learn/` | Episodes, playbooks, bandits, model routing, experiments, efficiency | Fully wired |
| roko-cli | `crates/roko-cli/` | CLI binary: all subcommands, ratatui TUI | Main entry point |
| roko-fs | `crates/roko-fs/` | FileSubstrate (JSONL), GC, layout | Stable |
| roko-std | `crates/roko-std/` | Defaults, 19 builtin tools, mock dispatcher | Stable |
| roko-runtime | `crates/roko-runtime/` | ProcessSupervisor, event bus, cancellation | Wired into PlanRunner |
| roko-primitives | `crates/roko-primitives/` | HDC vectors, tier routing | Fully wired (tier routing + HDC fingerprint-per-episode) |
| roko-neuro | `crates/roko-neuro/` | Durable knowledge store, distillation, tier progression | Wired |
| roko-mcp-code | `crates/roko-mcp-code/` | Code-intelligence MCP server | New in PR #13 |
| roko-mcp-github / slack / scripts / stdio | `crates/roko-mcp-*/` | Additional MCP integrations | Partial; see `tmp/ux-followup/05-partially-wired-subsystems.md` |
| roko-index | `crates/roko-index/` | Parser + graph + HDC indexing | Built |
| roko-lang-rust / typescript / go | `crates/roko-lang-*/` | Language support | Built |
| roko-dreams | `crates/roko-dreams/` | Offline consolidation (hypnagogia, imagination, cycle) | Partial (used from orchestrate.rs but no runtime trigger/cron) |
| roko-daimon | `crates/roko-daimon/` | Affect engine, somatic markers, dispatch modulation | Wired (DaimonState loaded + used per-task in orchestrate.rs) |
| roko-chain | `crates/roko-chain/` | Chain witness primitives | Phase 2+ |

## Absolute paths

| What | Path |
|---|---|
| **Workspace root** | `/Users/will/dev/nunchi/roko/roko/` |
| **All crates** | `/Users/will/dev/nunchi/roko/roko/crates/` |
| **CLI source** | `/Users/will/dev/nunchi/roko/roko/crates/roko-cli/src/` |
| **Orchestrator** | `/Users/will/dev/nunchi/roko/roko/crates/roko-cli/src/orchestrate.rs` |
| **Agent dispatcher** | `/Users/will/dev/nunchi/roko/roko/crates/roko-agent/src/dispatcher/mod.rs` |
| **Safety layer** | `/Users/will/dev/nunchi/roko/roko/crates/roko-agent/src/safety/` |
| **System prompt builder** | `/Users/will/dev/nunchi/roko/roko/crates/roko-compose/src/system_prompt_builder.rs` |
| **Role templates** | `/Users/will/dev/nunchi/roko/roko/crates/roko-compose/src/templates/` |
| **Master task list** | `/Users/will/dev/nunchi/roko/roko/tmp/MASTER-TASKS.md` |
| **Roko data dir** | `/Users/will/dev/nunchi/roko/roko/.roko/` |
| **Executor snapshots** | `/Users/will/dev/nunchi/roko/roko/.roko/state/` |
| **PRD storage** | `/Users/will/dev/nunchi/roko/roko/.roko/prd/` |
| **Research artifacts** | `/Users/will/dev/nunchi/roko/roko/.roko/research/` |
| **Signal log** | `/Users/will/dev/nunchi/roko/roko/.roko/signals.jsonl` |
| **Episode log** | `/Users/will/dev/nunchi/roko/roko/.roko/episodes.jsonl` |

## Reference material (read-only, do not modify)

| What | Path | Notes |
|---|---|---|
| Mori (original orchestrator) | `/Users/will/dev/uniswap/bardo/apps/mori/` | 108K LOC, the reference for what roko replaces |
| Mori agent connection | `/Users/will/dev/uniswap/bardo/apps/mori/src/agent/connection.rs` | Lines 2444-2620 = reference agent spawn |
| Original 36 crates | `/Users/will/dev/uniswap/bardo/crates/` | 137K LOC |
| Mori plans | `/Users/will/dev/uniswap/bardo/.mori/plans/` | 171 plans with TOML tasks |
| PRD documents | `/Users/will/dev/nunchi/roko/bardo-backup/prd/` | 359 files, 26 sections |
| Roko progress docs | `/Users/will/dev/nunchi/roko/bardo-backup/tmp/roko-progress/` | 140+ files, parity checklist (stale paths) |
| Mori parity checklist | `/Users/will/dev/nunchi/roko/bardo-backup/tmp/roko-progress/MORI-PARITY-CHECKLIST.md` | 1,253 items, ~33% done |
| Mistakes learned | `/Users/will/dev/nunchi/roko/bardo-backup/tmp/roko-progress/MISTAKES-LEARNED.md` | 30+ catalogued mistakes |
| Component specs | `/Users/will/dev/nunchi/roko/bardo-backup/tmp/roko-progress/COMPONENTS/` | 140+ per-component specs |
| Mori agent docs | `/Users/will/dev/nunchi/roko/bardo-backup/tmp/mori-agents/` | Backend arch, tool system |
| Research docs | `/Users/will/dev/nunchi/roko/bardo-backup/tmp/mori-refactor/` | Layer theory, design patterns |
| Agent chain docs | `/Users/will/dev/nunchi/roko/bardo-backup/tmp/agent-chain/` | Phase 2+ chain architecture |

## Building

```bash
cd /Users/will/dev/nunchi/roko/roko
rustup update stable          # Need 1.91+ for alloy deps
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --no-deps -- -D warnings
```

### Pre-commit checks (MANDATORY before any commit)

**Always run these before committing. CI will reject code that fails any of these.**

```bash
cargo +nightly fmt --all                              # Format (nightly, matches CI)
cargo clippy --workspace --no-deps -- -D warnings     # Lint (must pass clean)
cargo test --workspace                                # Tests (must pass)
```

Do NOT push without running all three. The CI uses the latest stable rustc which may have
stricter lints than your local toolchain.

## What to work on

Priority order for reaching full self-hosting:

1. ~~**Fix rustc**~~ → Done. Requires 1.91+ (`rustup default stable`).
2. ~~**Wire SystemPromptBuilder**~~ → Done. `RoleSystemPromptSpec` uses 9-layer builder + templates.
3. ~~**Wire EpisodeLogger**~~ → Done. Agent turns + gate results → `.roko/episodes.jsonl`.
4. ~~**Wire ProcessSupervisor**~~ → Done. `PlanRunner` tracks agents via `roko-runtime`.
5. ~~**Wire MCP**~~ → Done. `agent.mcp_config` in `roko.toml` + auto-discovery fallback.
6. ~~**Learning & feedback**~~ → Done. Efficiency events, cascade router persistence, prompt experiments, adaptive gate thresholds.
7. ~~**Interactive TUI**~~ → Done. ratatui wired; T1–T19 parity batches merged via PR #13.
8. ~~**Per-agent sidecar**~~ → Done. `roko-agent-server` real-dispatch path (T9) + integration tests (T19).
9. ~~**HTTP control plane**~~ → Done. `roko-serve` exposes ~85 routes for dashboards / external callers.
10. ~~**Automatic plan generation**~~ → Done. `prd.auto_plan` config triggers `prd plan` on publish via `spawn_prd_publish_subscriber`.
11. ~~**Feedback loop**~~ → Done. `learning_config.replan_on_gate_failure` triggers `build_gate_failure_plan_revision`.
12. ~~**Follow-up catalog**~~ → Done. Most items verified/closed; see `tmp/ux-followup/00-INDEX.md`.

Roko can now fully self-host: read PRDs, generate plans, execute them, validate results,
learn from failures, and iterate. Remaining work:

13. **Knowledge-informed agent routing** → neuro store not yet consulted for model selection in CascadeRouter.
14. **Cold substrate archival** → built but not instantiated at runtime (no cron/trigger).
15. **UX34: force_backend override learning** → cascade router doesn't learn from manual overrides.
16. **Chain runtime integration** → Phase 2+ (needs blockchain backend for witness anchoring).
