# roko batch 3: orchestration loop, mirage dashboard, cloud deployment, learning feedback

**Branch:** `roko-batch3-wiring` -> `main`
**395 files changed** | **+39,561** | **-3,694** | **16 commits**

---

## What this PR does

This PR wires the remaining infrastructure that makes roko self-hosting. Four major systems land:

1. **Orchestration loop** -- The 4,011-line `orchestrate.rs` connects the plan-execute-gate-persist cycle end-to-end. Plans discovered on disk get executed by Claude CLI agents, validated by gate pipelines, and persisted with session snapshots. The loop supports resume, parallel execution, cost budgets, conductor signals, and a learning feedback path.

2. **Mirage-RS dashboard and API** -- A full interactive single-page dashboard with 35+ REST endpoints, 8 JSON-RPC methods, WebSocket streaming, a force-directed pheromone particle system, knowledge graph visualization, agent topology network, task lifecycle tracking, and a 20-agent simulation harness.

3. **Cloud deployment** -- `roko serve` exposes the entire CLI surface as an HTTP API (25+ endpoints), with Railway deployment backends, a cloud worker system, and Docker packaging.

4. **Learning and feedback** -- Efficiency events, cascade router persistence, prompt A/B experiments, adaptive gate thresholds, and a runtime feedback system that feeds failed gates back into plan generation.

---

## Commits

| SHA | Summary |
|-----|---------|
| `b15d55f` | mirage-rs: real fork info, task stats, cognitive traces, dashboard polish |
| `40b4e32` | fix agent registration: use string pubkey, not array |
| `8cc9b04` | add task system, 20-agent simulation, tokenomics dashboard, auto-WS |
| `21c10e2` | dashboard: fix jitter -- incremental DOM updates, canvas size caching, CSS containment |
| `633674f` | add interactive dashboard UI with modular ES modules and static file serving |
| `2567918` | mirage-rs HTTP API: validation, write endpoints, caching, heartbeat, tests, docs |
| `b0f8864` | issue #5: agent registry, HTTP/RPC/WS endpoints, dashboard data rendering, block timestamp fix |
| `4c246d3` | add roko serve cloud deployment + worker subcommand |
| `7a184e4` | wire parity sections 6-11: cross-plan deps, parallel limits, conductor signals, cost budget, learning loop, plan regeneration |
| `8779ff0` | wire learning plan 05 remaining items: efficiency events, cascade persistence, prompt experiments, adaptive thresholds |
| `4ba8c3a` | wire learning, MCP tools, and observability into orchestration loop |
| `9635bd4` | wire EpisodeLogger, ProcessSupervisor, and MCP config into orchestration loop |
| `78d1046` | add research and PRD agent flows |
| `99b0989` | commit remaining repo changes |
| `0e87748` | repair Claude wiring and finish CLI runtime integration |
| `bfd13e5` | gitignore: exclude CLAUDE.md and scripts/ until refined |
| `be35513` | roko batch 3: fix failing test, wire safety dispatch, orchestration loop, session persistence |

---

## 1. Orchestration loop

**File:** `crates/roko-cli/src/orchestrate.rs` (4,011 lines, new)

The core self-hosting runtime. Connects the CLI to `roko-orchestrator`'s pure state machine (`ParallelExecutor`), dispatching its `ExecutorAction`s to real agents, gates, and git, then feeding results back as `ExecutorEvent`s.

### PlanRunner

The top-level struct that manages a full orchestration run:

- Discovers plans via `roko_orchestrator::discover_plans()`
- Builds a `ParallelExecutor` with dependency-ordered DAG
- Dispatches up to `MAX_PARALLEL_TASKS` (4) agents concurrently
- Tracks running agent processes via `bardo_runtime::ProcessSupervisor`
- Auto-saves executor state every `AUTOSAVE_INTERVAL` (5) actions to `.roko/state/executor.json`
- Supports `--resume` from a saved snapshot

### Agent dispatch

Each task spawns an isolated agent subprocess via `AgentRunConfig`:

- **Claude CLI agents** (`ClaudeCliAgent`) -- spawns `claude` with model selection, system prompt, MCP config passthrough, bare mode, effort level, tool allowlists, fallback model, environment variables, resume session, and `--dangerously-skip-permissions` flag
- **Exec agents** (`ExecAgent`) -- spawns arbitrary commands with timeout and env vars
- Both wrapped in `run_prepared_agent()` which requires no `PlanRunner` borrow, enabling parallel dispatch

### System prompt assembly

`RoleSystemPromptSpec` drives 6-layer prompt construction via `roko-compose::SystemPromptBuilder`:

- Layer 1: Role identity (implementer, reviewer, strategist, researcher, etc.)
- Layer 2: Task context (plan metadata, task description, dependencies)
- Layer 3: Codebase context (relevant files, symbols, recent changes)
- Layer 4: Constraints (budget, time, quality gates)
- Layer 5: Learning context (past episodes, efficiency data, experiment variants)
- Layer 6: Output format (expected deliverables, gate requirements)

### Gate pipeline

Per-task validation after agent completion:

- `CompileGate` -- `cargo build` passes
- `TestGate` -- `cargo test` passes
- `ClippyGate` -- `cargo clippy` clean
- Diff gate -- changes stay within expected scope
- Adaptive thresholds -- EMA-adjusted pass/fail thresholds per rung via `roko_gate::AdaptiveThresholds`
- Gate results recorded as `GateVerdict` in episode log

### Conductor integration

`roko_conductor::Conductor` monitors execution health:

- Circuit breaker -- halts execution after repeated failures
- Stuck detection -- identifies tasks that exceed expected duration
- Cost overrun watcher -- enforces per-plan and per-task cost limits
- Returns `ConductorDecision` (proceed, pause, abort) each cycle

### Learning and feedback

Wired into the execution loop:

- **EpisodeLogger** -- records agent turns + gate results to `.roko/episodes.jsonl` as `Episode` entries with `Usage` and `GateVerdict`
- **Efficiency events** -- per-turn `AgentEfficiencyEvent` written to `.roko/learn/efficiency.jsonl`
- **CascadeRouter** -- persists model routing decisions to `.roko/learn/cascade-router.json` for replay
- **Prompt experiments** -- `ExperimentStore` runs A/B tests across prompt variants, persisted to `.roko/learn/experiments.json`
- **Adaptive gate thresholds** -- EMA per rung saved to `.roko/learn/gate-thresholds.json`
- **Cost tracking** -- `CostRecord` entries logged per task
- **Runtime feedback** -- `LearningRuntime` collects `CompletedRunInput` data and produces `LearningUpdate`s

### Context attribution

`ContextAttributionTracker` monitors which context tiers and source types agents actually reference:

- Loads historical data from `.roko/context-attribution.jsonl`
- Tracks per-(tier, source_type) reference rates
- Demotes context sources with <10% reference rate
- Records new attribution events after each agent run

### Cross-plan dependencies and parallel limits

- Tasks can declare dependencies on tasks in other plans
- Configurable `MAX_PARALLEL_TASKS` limits concurrent agent count
- DAG respects cross-plan edges during scheduling

### Plan regeneration

When tasks fail repeatedly, the learning loop feeds failure data back into plan generation for automatic re-planning.

### Report types

- `PlanRunReport` -- per-plan results (plan_id, succeeded, agent_calls, gate_results)
- `OrchestrationReport` -- aggregate across all plans (total_agent_calls, total_gate_runs, all_succeeded)

---

## 2. PRD system

**Files:** `crates/roko-cli/src/prd.rs` (~440 lines), `crates/roko-cli/src/prd_prompt.rs` (~225 lines)

Full PRD lifecycle management. PRDs live in `.roko/prd/` with this layout:

```
.roko/prd/
  ideas.md              # quick captures
  drafts/               # work-in-progress PRDs
    <slug>.md
  published/            # finalized PRDs
    <slug>.md
```

### CLI subcommands

| Command | What it does |
|---------|-------------|
| `roko prd idea "<text>"` | Append a timestamped idea to `.roko/prd/ideas.md` |
| `roko prd list` | List all PRDs (drafts + published) with status, title, creation date |
| `roko prd status` | Coverage report: plans generated per PRD, tasks per plan, completion ratio |
| `roko prd draft new "<title>"` | Create a new PRD draft with agent-driven refinement |
| `roko prd draft promote` | Promote a draft to published status (moves file, updates frontmatter) |
| `roko prd plan <slug>` | Generate implementation plan + `tasks.toml` from a published PRD |
| `roko prd consolidate` | Merge duplicate/overlapping PRDs into a single document |

### PRD frontmatter

`PrdMeta` struct parsed from markdown frontmatter:

- `id` -- stable identifier (e.g. `prd-golem-memory`)
- `title` -- human-readable title
- `status` -- lifecycle status (`draft` or `published`)

### Prompt generation

`prd_prompt.rs` builds the system prompt for PRD-related agent interactions, including context about existing PRDs, plan coverage, and the expected output format.

---

## 3. Research system

**File:** `crates/roko-cli/src/research.rs` (~286 lines)

Agent-driven research with academic rigor. Artifacts stored in `.roko/research/` as markdown files.

### CLI subcommands

| Command | What it does |
|---------|-------------|
| `roko research topic "<topic>"` | Deep research with citations (searches arXiv, ACL, NeurIPS, etc.) |
| `roko research enhance-prd <slug>` | Enhance PRD with research findings and supporting citations |
| `roko research enhance-plan <plan>` | Optimize plan with latest techniques from literature |
| `roko research enhance-tasks <plan>` | Split/optimize tasks based on research into decomposition |
| `roko research analyze` | Analyze execution data for self-learning insights |

### Research agent prompt

`RESEARCH_SYSTEM_PROMPT` enforces:

- Real citations with full author, title, venue, year in [AUTHOR-YEAR] format
- Practical relevance: every finding connects to a concrete recommendation
- Recency bias: prefer 2023-2026 papers
- Contrarian findings: actively seek papers that challenge the current approach
- Structured output: Finding / Source / Relevance / Recommendation / Confidence

### Sources checked

arXiv (cs.SE, cs.AI, cs.CL, cs.MA), ACL, EMNLP, NeurIPS, ICML, ICLR, ISSTA, ICSE, FSE, Anthropic/OpenAI/DeepMind research blogs, SWE-bench, HumanEval/MBPP benchmarks, and recent agent framework papers.

---

## 4. Cloud deployment (`roko serve`)

**Directory:** `crates/roko-cli/src/serve/` (~2,300+ lines across 21 files)

Complete HTTP API server and cloud deployment system.

### Server architecture

- `mod.rs` -- `run_server()` entry point: loads `roko.toml`, builds `AppState`, binds TCP listener with graceful shutdown
- `state.rs` -- `AppState` shared across all handlers (config, workdir, event bus)
- `events.rs` -- Server-sent event bus for real-time updates
- `error.rs` -- Typed API error responses
- `templates.rs` -- Template management utilities
- Respects `PORT` env var for Railway/cloud platform compatibility

### HTTP API endpoints

All routes nested under `/api`:

| Method | Path | Module | Description |
|--------|------|--------|-------------|
| GET | `/api/status` | `routes::status` | Server health, uptime, plan/PRD/agent counts |
| GET | `/api/plans` | `routes::plans` | List discovered plans with metadata |
| GET | `/api/plans/:id` | `routes::plans` | Plan details including task list |
| POST | `/api/plans/:id/run` | `routes::plans` | Execute a plan (kicks off orchestration loop) |
| POST | `/api/plans/:id/resume` | `routes::plans` | Resume a paused/failed plan from snapshot |
| DELETE | `/api/plans/:id` | `routes::plans` | Delete a plan |
| GET | `/api/prds` | `routes::prds` | List all PRDs |
| GET | `/api/prds/:slug` | `routes::prds` | Get PRD content and metadata |
| POST | `/api/prds` | `routes::prds` | Create new PRD |
| POST | `/api/prds/:slug/plan` | `routes::prds` | Generate implementation plan from PRD |
| POST | `/api/prds/:slug/promote` | `routes::prds` | Promote draft PRD to published |
| GET | `/api/agents` | `routes::agents` | List running/registered agents |
| GET | `/api/agents/:id` | `routes::agents` | Get agent details and stats |
| POST | `/api/agents/:id/stop` | `routes::agents` | Stop a running agent |
| GET | `/api/research` | `routes::research` | List research artifacts |
| POST | `/api/research` | `routes::research` | Start a research task |
| GET | `/api/learning` | `routes::learning` | Get learning metrics (episodes, efficiency, experiments) |
| GET | `/api/config` | `routes::config` | Get roko.toml configuration |
| PUT | `/api/config` | `routes::config` | Update configuration |
| POST | `/api/run` | `routes::run` | Single prompt -> universal loop (compose->agent->gate->persist) |
| GET | `/api/deployments` | `routes::deployments` | List cloud deployments |
| POST | `/api/deployments` | `routes::deployments` | Deploy to Railway/cloud |
| DELETE | `/api/deployments/:id` | `routes::deployments` | Tear down deployment |
| GET | `/api/templates` | `routes::templates` | List plan/PRD templates |
| POST | `/api/templates` | `routes::templates` | Create a template |
| WS | `/api/ws` | `routes::ws` | WebSocket event stream |

### Middleware

- CORS -- configurable allowed origins, permissive by default
- `tower_http::TraceLayer` -- request tracing
- Route grouping via `build_router()` that merges all submodule routers

### Deployment backends

**Railway CLI** (`deploy/railway_cli.rs`):
- `railway up` -- deploy to Railway
- `railway link` -- link to existing project
- Environment variable management

**Railway GraphQL API** (`deploy/railway_api.rs`):
- Create project and service via GraphQL
- Deploy service from Docker image
- Retrieve deployment logs

**Manual** (`deploy/manual.rs`):
- Generates Dockerfile + deployment instructions
- Produces `docker-compose.yml` for self-hosting

### Worker system

- `worker/mod.rs` + `worker/handler.rs` -- cloud worker that polls the server for pending tasks and executes them
- `roko worker` subcommand -- runs the polling loop
- `docker/worker.Dockerfile` -- container image for cloud workers
- `railway.toml` -- Railway platform configuration

---

## 5. Mirage-RS dashboard and API

**Directory:** `apps/mirage-rs/` (~8,000+ lines new/modified)

### HTTP REST API

All endpoints served under `/api` via `axum::Router`. Every list endpoint returns a `PaginatedResponse` envelope:

```json
{
  "items": [...],
  "total": 142,
  "offset": 0,
  "limit": 100,
  "has_more": true
}
```

#### Health and stats

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/health` | Server health: status, uptime_secs, chain toggles (hdc/knowledge/stigmergy), counts (insights/pheromones/agents/tasks) |
| GET | `/api/stats` | Combined dashboard stats: insight state breakdown (active/confirmed/challenged/decaying), pheromone kind breakdown (threat/opportunity/wisdom + total_intensity), task state breakdown (open/assigned/in_progress/completed/failed/cancelled + stake/reward totals), chain toggles |

#### Pheromone field

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/pheromones` | List active pheromones. Filters: `kind` (threat/opportunity/wisdom), `min_intensity`. Sort: `intensity` (default), `deposited_at`, `confirmations`. Pagination: `offset`, `limit` (max 1000). Each item includes `decay_projection` with `in_1h`, `in_4h`, `in_24h` intensities. |
| POST | `/api/pheromones` | Deposit a new pheromone. Body: `kind`, `content`, `intensity`, `half_life_seconds`. Returns the created pheromone with ID. |
| GET | `/api/pheromones/summary` | Aggregate stats per kind: count, total intensity, avg intensity, max intensity, avg half-life. |
| POST | `/api/pheromones/query` | Top-K by HDC similarity x intensity. Body: `query` (text), `k` (max 100), optional `kind` filter. Uses `ProjectionCache` (LRU, default 1024 entries) to avoid recomputing HDC projections. |
| GET | `/api/pheromones/heatmap` | Time-bucketed deposit activity. Params: `bucket_width` (min 60s), `buckets` (max 500). Returns array of `{start, end, count, kinds: {threat, opportunity, wisdom}}`. |
| GET | `/api/pheromones/{id}/projection` | Decay projection for a single pheromone at future timestamps. |

#### Knowledge graph

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/knowledge/entries` | List insight entries. Filters: `kind` (insight/heuristic/warning/causal_link/strategy_fragment/anti_knowledge), `state` (created/active/confirmed/decaying/challenged/pruned/stale), `min_weight`. Sort: `weight` (default), `created_at`, `confirmations`. Each entry includes: id, kind, weight, initial_weight, state, confirmations, challenges, created_at, content, author, enabled_by deps, half_life_seconds, effective_half_life_seconds, stake_wei (string for precision). |
| POST | `/api/knowledge/entries` | Post a new insight. Body: `content`, `author`, `kind`, `stake_wei`, optional `enabled_by` (dependency IDs). |
| POST | `/api/knowledge/entries/{id}/confirm` | Confirm an insight. Body: `confirmer` (agent ID). Updates confirmer's `confirmations_given` stat. |
| POST | `/api/knowledge/entries/{id}/challenge` | Challenge an insight. Body: `challenger` (agent ID). Updates challenger's `challenges_given` stat. |
| POST | `/api/knowledge/decay` | Trigger manual decay sweep across all entries. |
| GET | `/api/knowledge/edges` | Dependency edges (from `enabled_by`) + HDC similarity edges between entries. Returns `{dependency_edges, similarity_edges}` for force-directed graph layout. |
| GET | `/api/knowledge/search` | Semantic search over knowledge store. Params: `q` (query text), `k` (max 100). Uses HDC projection + cosine similarity for top-k ranking. |
| GET | `/api/knowledge/kinds` | Enumerate all knowledge kinds and pheromone kinds with descriptions. |

#### Agent registry

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/agents` | List all registered agents with summary stats: id, role, registered_at, last_heartbeat_block, last_heartbeat_ts, stats (confirmations_given, challenges_given, warnings_posted, insights_posted, tasks_completed, tasks_failed, delta_cycles, total_cost_usd, total_tokens). |
| POST | `/api/agents` | Register a new agent. Body: `id`, `pubkey` (string), `role`. |
| GET | `/api/agents/{id}/trace` | Cognitive loop history (paginated). Params: `limit` (default 10), `offset`. Each trace: cycle, phase (retrieve/reason/act/verify), reads, reasoning, action, action_id, timestamp. |
| POST | `/api/agents/{id}/trace` | Record a cognitive trace entry. Body: `cycle`, `phase`, `reads`, `reasoning`, `action`, `action_id`. |
| GET | `/api/agents/{id}/heartbeat` | Liveness status: alive (bool), last_block, last_timestamp, blocks_since, timeout_blocks (200). |
| POST | `/api/agents/{id}/heartbeat` | Send heartbeat. Body: optional `total_tokens`, `total_cost_usd`. Updates agent's last heartbeat and token/cost stats. |
| GET | `/api/agents/{id}/stats` | Aggregated stats for an agent: all fields from `AgentStats`. |

#### Agent topology

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/agents/topology` | Agent interaction graph derived from knowledge store. Returns `{nodes, edges, timestamp}`. Nodes: id, address, insights_posted, confirmations_given, challenges_given, total_weight. Edges: from, to, weight, type ("confirmed" or "challenged"). |

#### Task tracking

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/tasks` | List tasks with filters. Params: `state` (open/assigned/in_progress/completed/failed/cancelled), `kind` (research/validate/analyze/monitor/report/...), `assignee` (agent ID), `limit` (default 20, max 200), `offset`. Each task: id, title, description, kind, priority (low/medium/high/critical), state, creator, assignee, created_at, assigned_at, started_at, completed_at, stake_wei, reward_wei, result_insight_id, tags, attempts, max_attempts. |
| POST | `/api/tasks` | Create a new task. Body: `title`, `description`, `kind`, `priority`, `creator`, `tags`, `stake_wei`, `max_attempts`. |
| GET | `/api/tasks/stats` | Aggregate counts: open, assigned, in_progress, completed, failed, cancelled, total_stake_wei, total_reward_wei. |
| GET | `/api/tasks/{id}` | Get a single task by ID. |
| POST | `/api/tasks/{id}/assign` | Assign task to agent. Body: `assignee`. Transitions Open -> Assigned. |
| POST | `/api/tasks/{id}/start` | Mark task in-progress. Transitions Assigned -> InProgress. |
| POST | `/api/tasks/{id}/complete` | Complete task. Body: optional `result_insight_id`. Awards reward, increments agent's `tasks_completed`. Transitions InProgress -> Completed. |
| POST | `/api/tasks/{id}/fail` | Fail task. Increments agent's `tasks_failed` and task's `attempts`. Auto-cancels if `attempts >= max_attempts`. Transitions InProgress -> Failed (or Cancelled). |
| POST | `/api/tasks/{id}/cancel` | Cancel task. Transitions any non-terminal state -> Cancelled. |

#### WebSocket streaming

| Method | Path | Description |
|--------|------|-------------|
| WS | `/api/ws` | Live event stream. Params: `pheromones` (bool, default true), `insights` (bool, default true), `agents` (bool, default false), `agent_id` (optional filter). Wire format: `{"channel": "pheromone"|"insight"|"agent", "data": {...}}`. Server pings every 30s, closes if no pong within 90s. |

### JSON-RPC methods (chain_*)

New RPC methods added to the existing `chain_*` namespace:

| Method | Params | Returns |
|--------|--------|---------|
| `chain_registerAgent` | `{id, pubkey, role}` | `{id, registered_at}` |
| `chain_agentHeartbeat` | `{agent_id, block?, total_tokens?, total_cost_usd?}` | `{alive, last_block}` |
| `chain_agentTrace` | `{agent_id, cycle, phase, reads, reasoning, action, action_id}` | `{recorded: true}` |
| `chain_agentStats` | `{agent_id}` or `{agent_id, delta: {...}}` | `AgentStats` |
| `chain_listAgents` | none | `[AgentEntry]` |
| `chain_createTask` | `{title, description, kind, priority, creator, tags?, stake_wei?}` | `TaskEntry` |
| `chain_assignTask` | `{task_id, assignee}` | `TaskEntry` |
| `chain_completeTask` | `{task_id, result_insight_id?}` | `TaskEntry` |
| `chain_failTask` | `{task_id}` | `TaskEntry` |

### Infrastructure middleware

- **Request ID** -- `x-request-id` header injected via `AtomicU64` counter (`req-1`, `req-2`, ...), echoed on response
- **Cache-Control** -- `public, max-age=N` on read-only endpoints (2s for data, `no-cache` for static files)
- **Concurrency limit** -- `tower::limit::ConcurrencyLimitLayer::new(200)`
- **Tracing** -- `tower_http::TraceLayer` on all routes
- **Validation constants** -- `MAX_LIMIT=1000`, `MAX_K=100`, `MIN_BUCKET_WIDTH=60s`, `MAX_HEATMAP_BUCKETS=500`
- **HDC projection cache** -- thread-safe bounded LRU (`ProjectionCache`) backed by `lru::LruCache<String, HdcVector>` with configurable capacity (default 1024)

### Data models

**AgentEntry:**

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` | Unique agent identifier |
| `address` | `Vec<u8>` | On-chain address bytes |
| `role` | `String` | Agent role (researcher, coder, watcher, etc.) |
| `registered_at` | `u64` | Registration timestamp (Unix seconds) |
| `last_heartbeat_block` | `u64` | Block number of last heartbeat |
| `last_heartbeat_ts` | `u64` | Timestamp of last heartbeat |
| `stats` | `AgentStats` | Accumulated statistics |

**AgentStats:**

| Field | Type | Description |
|-------|------|-------------|
| `confirmations_given` | `u64` | Insight confirmations issued |
| `challenges_given` | `u64` | Insight challenges issued |
| `warnings_posted` | `u64` | Warnings posted |
| `insights_posted` | `u64` | Insights posted |
| `tasks_completed` | `u64` | Tasks completed successfully |
| `tasks_failed` | `u64` | Tasks that failed |
| `delta_cycles` | `u64` | Cognitive cycles completed |
| `total_cost_usd` | `f64` | Total cost in USD |
| `total_tokens` | `u64` | Total tokens consumed |

**AgentTrace:**

| Field | Type | Description |
|-------|------|-------------|
| `cycle` | `u64` | Cognitive cycle number |
| `phase` | `CognitivePhase` | retrieve, reason, act, or verify |
| `reads` | `Vec<String>` | Resources read during this phase |
| `reasoning` | `String` | Reasoning text |
| `action` | `String` | Action taken |
| `action_id` | `String` | Unique action identifier |
| `timestamp` | `u64` | Unix timestamp in seconds |

**AgentEvent** (WebSocket, tagged union):
- `Trace { agent_id, trace }`
- `Heartbeat { agent_id, block, timestamp }`
- `Stats { agent_id, delta }`
- `Registered { agent_id, role }`

**TaskEntry:**

| Field | Type | Description |
|-------|------|-------------|
| `id` | `u64` | Auto-incrementing task ID |
| `title` | `String` | Short human-readable title |
| `description` | `String` | Detailed work description |
| `kind` | `String` | research, validate, analyze, monitor, report, etc. |
| `priority` | `TaskPriority` | low, medium, high, critical |
| `state` | `TaskState` | open, assigned, in_progress, completed, failed, cancelled |
| `creator` | `String` | Agent ID that created the task |
| `assignee` | `Option<String>` | Agent ID assigned to the task |
| `created_at` | `u64` | Creation timestamp |
| `assigned_at` | `Option<u64>` | Assignment timestamp |
| `started_at` | `Option<u64>` | Work start timestamp |
| `completed_at` | `Option<u64>` | Terminal state timestamp |
| `stake_wei` | `u128` | Stake deposited for this task |
| `reward_wei` | `u128` | Reward paid on completion |
| `result_insight_id` | `Option<String>` | ID of produced insight |
| `tags` | `Vec<String>` | Topic tags for matching |
| `attempts` | `u32` | Times this task was attempted |
| `max_attempts` | `u32` | Auto-cancel threshold |

**TaskEvent** (streaming, tagged union):
- `Created { id, title, kind, creator }`
- `Assigned { id, assignee }`
- `Started { id }`
- `Completed { id }`
- `Failed { id }`
- `Cancelled { id }`

**Task state machine:**

```
Open -> Assigned -> InProgress -> Completed
                               -> Failed (retryable if attempts < max_attempts)
                               -> Cancelled
Any non-terminal -> Cancelled
```

### Dashboard frontend

**File:** `apps/mirage-rs/static/index.html` + `static/js/` + `static/style.css`

Single-page dashboard built with vanilla JS (no framework), ES modules, and Canvas 2D rendering.

#### Module architecture

| File | Responsibility |
|------|---------------|
| `js/main.js` | Init, `connect()`, `requestAnimationFrame` loop, event wiring, interval setup for all pollers |
| `js/state.js` | Single shared mutable state object imported by all modules: blocks, insights (Map), pheromones (particles), topology, heatmap, agent registry, sparkline series, RPC counters, poller handles |
| `js/api.js` | `rpc()` for JSON-RPC, `api()` for REST GET, `apiPost()` for REST POST, request logging, toast notifications, render callback registration |
| `js/polling.js` | All REST polling functions: `pollBlock`, `pollChain`, `pollEntries`, `pollEdges`, `pollKinds`, `pollPheroSummary`, `pollHeatmap`, `pollTopology`, `pollAgentRegistry`, `pollLeaderboard`, `pollTasks` |
| `js/pheromones.js` | Force-directed particle system: spatial grid for O(n) neighbor queries, shaped particles (diamond=threat, circle=opportunity, hexagon=wisdom), hover/click tooltips, entrance animations, death fade, decay projection overlay, kind filter pills, FPS counter |
| `js/graph.js` | Force-directed insight knowledge graph: HDC proximity edges, kind-colored nodes, click-to-detail sidebar, search highlighting, dynamic node addition |
| `js/topology.js` | Force-directed agent network graph: confirm/challenge edges, role-colored nodes, weight-scaled edges |
| `js/charts.js` | Sparkline renderer, knowledge growth timeline, heatmap visualization, block stream renderer, metric cards |
| `js/ws.js` | WebSocket live event stream: auto-reconnect with backoff, pheromone + insight event handlers, connection status chip |
| `style.css` | 680-line dark theme: glassmorphism panels, gradient accents, canvas animations, responsive grid, chip/badge system |

#### Dashboard sections (top to bottom)

1. **Header** -- RPC URL input, reconnect button, connection status chip, fork info chip (block number + upstream URL), agent count chip, WS toggle, reset button
2. **Hero stats** -- 6 cards with sparklines: chain tip, gas base fee (gwei), saturation (%), insights on-chain, live pheromones, registered agents. Each card shows current value, delta indicator, and a canvas sparkline
3. **Knowledge accumulation timeline** -- 60s rolling growth chart
4. **Block stream** -- Recent blocks with number, hash, gas, tx count, saturation bar
5. **Pheromone field** -- Canvas particle system: force-directed bubbles shaped by kind (diamond/circle/hexagon), colored by kind (red=threat, green=opportunity, gold=wisdom), sized by intensity, with spatial grid for collision detection. Hover shows tooltip with content, intensity, decay projection. Click pins the tooltip. Kind filter pills toggle visibility.
6. **Agent activity log** -- Real-time log of agent actions (posts, confirmations, challenges)
7. **Agent registry** -- Table of registered agents: ID, role, heartbeat status, stats. Expandable rows show cognitive traces (Retrieve -> Reason -> Act -> Verify per cycle)
8. **Pheromone summary** -- Per-kind aggregate cards: count, total intensity, avg intensity
9. **Task lifecycle** -- Task state distribution (open/assigned/in_progress/completed/failed), recent task list, stake/reward totals
10. **Tokenomics** -- Stake/reward economics, total value locked, completion rates
11. **Pheromone heatmap** -- Time-bucketed activity chart with kind breakdown
12. **Insight knowledge graph** -- Canvas force-directed graph: nodes colored by kind, sized by weight, connected by dependency + similarity edges. Click a node for detail sidebar. Search box highlights matching nodes.
13. **Agent topology** -- Canvas force-directed network: nodes per agent, edges per confirm/challenge interaction, weight-scaled
14. **Agent leaderboard** -- Ranked by total activity (insights + confirmations + challenges + tasks)
15. **Performance metrics** -- RPC call rate, cache hit rate, search latency
16. **Knowledge kinds reference** -- All registered kinds with descriptions
17. **Manual controls** -- Forms to post insight, deposit pheromone, run semantic search, register agent
18. **API trace log** -- Rolling log of all API requests with timing

#### Pheromone particle system details

- `P_COLORS`: threat (red #f87171), opportunity (green #4ade80), wisdom (gold #fbbf24)
- `P_HALFLIFE`: threat 60s, opportunity 90s, wisdom 180s
- Spatial grid (`GRID_CELL=60px`) for O(n) neighbor lookup via `gridBuild()` / `gridNeighbors()`
- Particles have: kind, content, intensity, position (x/y), anchor (anchorX/Y), velocity (vx/vy), age, deposited timestamp, halfLife, pulse animation, chainId, decayProjection
- Force simulation: repulsion between nearby particles, gentle attraction to anchor, velocity damping
- Entrance animation: scale from 0 to 1 with overshoot
- Death animation: fade out when intensity drops below threshold
- Filter pills: toggle threat/opportunity/wisdom visibility
- FPS tracking for performance monitoring

### Agent simulation

**File:** `apps/mirage-rs/examples/agent_simulation.rs`

20 concurrent agent personas across 6 roles, each running as a tokio task:

| Role | Count | Behavior | Pace |
|------|-------|----------|------|
| **Watcher** | 6 | Post insights about DeFi protocols, deposit pheromones, create tasks | 12-25s |
| **Security** | 4 | Post exploit alerts, rug pull detection, phishing warnings | 22-30s |
| **Strategy** | 4 | Yield farming analysis, momentum signals, correlation patterns | 20-28s |
| **Validator** | 4 | Confirm/challenge entries, claim and complete/fail tasks | 15-20s |
| **Synthesizer** | 2 | Cross-reference insights, post meta-analyses | 30-35s |
| **Infra** | 2 | Health monitoring, decay sweeps, task creation | 25-30s |

**Watcher agents** (6 unique DeFi focuses):
- `roko-alpha-amm` -- AMM pools: Uniswap, Curve, Balancer liquidity analysis
- `roko-beta-lending` -- Lending protocols: Aave, Compound, Morpho utilization monitoring
- `roko-gamma-mev` -- MEV: sandwiches, frontrunning, Flashbots block analysis
- `roko-delta-bridge` -- Cross-chain bridges and L2 monitoring
- `roko-epsilon-governance` -- DAO governance and voting pattern analysis
- `roko-zeta-derivatives` -- Perps, options, structured products tracking

**Security agents** (4):
- `roko-sec-exploit` -- Smart contract exploit detection
- `roko-sec-rugpull` -- Rug pull pattern matching
- `roko-sec-phishing` -- Phishing and social engineering alerts
- `roko-sec-audit` -- Code audit findings

**Cognitive traces:** Each agent cycle produces a 4-phase trace (Retrieve -> Reason -> Act -> Verify) with reads, reasoning text, action description, and unique action ID.

**Task templates:** 15 templates spanning: analyze pool rebalancing, monitor liquidation thresholds, research yield strategies, validate MEV claims, report bridge anomalies.

**Usage:**
```bash
cargo run -p mirage-rs --features chain,roko --example agent_simulation -- \
    --rpc-url http://127.0.0.1:8545
```

### Integration tests

**File:** `apps/mirage-rs/tests/http_api.rs` (1,061 lines)

Full coverage of all REST endpoints:

- Pheromone endpoints -- list with filters, deposit, summary, HDC query, heatmap, projection
- Knowledge endpoints -- list with filters, post insight, confirm, challenge, decay, edges, search, kinds
- Agent endpoints -- list, register, trace (get/post), heartbeat (get/post), stats
- Task endpoints -- list with filters, create, get, assign, start, complete, fail, cancel, stats
- Topology -- agent interaction graph
- Stats -- combined dashboard stats
- Health -- uptime, chain status, counts
- Pagination -- offset/limit, has_more flag
- Sorting -- ascending/descending on multiple fields
- Error handling -- 404 (not found), 409 (state conflict), 400 (validation)
- Input validation -- clamped limits, invalid kind strings, missing fields
- Cache-Control headers -- verified on read-only endpoints

---

## 6. Fork state improvements

- `ForkState` tracks `fork_block` (upstream head block number at fork time) and `fork_url` (upstream RPC URL)
- `mine_block()` advances timestamp via `now_secs()` instead of being frozen at init time
- `MirageStatus` includes `forkBlock` and `forkUrl` in `mirage_status` JSON-RPC responses
- `UpstreamRpc::http_url()` accessor added for fork URL display
- Cache-Control middleware on `/dashboard` static files: `no-cache, must-revalidate`

---

## 7. Agent system

**Directory:** `crates/roko-agent/`

### ClaudeCliAgent (new, 701 lines)

Spawns Claude Code CLI processes with full configuration:

- Model selection with fallback
- 6-layer system prompt injection
- MCP config passthrough (`--mcp-config` flag from `agent.mcp_config` in `roko.toml`)
- Bare mode (no interactive UI)
- Effort level control
- Tool allowlists (CSV)
- Settings JSON generation via `build_settings_json()`
- `--dangerously-skip-permissions` flag for CI/automation
- Session resume support
- Extra CLI args passthrough
- Environment variable injection
- Timeout enforcement

### Safety integration

Safety layer integrated into `ToolDispatcher`:
- Role-based authorization -- agents can only use tools permitted for their role
- Pre-execution checks -- validate tool arguments before execution
- Post-execution checks -- validate tool results after execution
- 342 lines of new safety integration tests

### MCP config passthrough

When `agent.mcp_config` is set in `roko.toml`, the path is forwarded to all spawned Claude CLI agents via `--mcp-config`. Auto-discovery fallback checks standard MCP config locations.

---

## 8. Learning and feedback

**Directory:** `crates/roko-learn/` (~1,800+ lines new)

### PromptExperiment

A/B testing framework for prompt variants:
- Define experiment variants with different prompt sections
- Track success/failure rates per variant
- Bayesian analysis for variant selection
- Persisted to `.roko/learn/experiments.json`

### RuntimeFeedback (937 lines)

`LearningRuntime` collects execution data and produces actionable updates:
- `CompletedRunInput` -- captures plan ID, task results, gate verdicts, agent metrics, cost data
- `LearningUpdate` -- recommendations for prompt changes, model routing, gate thresholds
- Integrates efficiency events, cost records, and episode data

### CostsLog

Per-task cost logging:
- Model, tokens, estimated cost per agent invocation
- Aggregated per plan and per session

### AdaptiveThreshold

EMA-based gate threshold adjustment:
- Tracks pass/fail rate per gate rung
- Adjusts thresholds based on recent performance
- Prevents threshold drift from noisy results
- Persisted to `.roko/learn/gate-thresholds.json`

### CascadeRouter persistence

Model routing decisions saved to `.roko/learn/cascade-router.json` for replay and analysis. Configurable model tiers.

### Integration tests (227 lines)

Coverage for experiment store, runtime feedback collection, threshold adjustment, and cost logging.

---

## 9. Compose system

**Directory:** `crates/roko-compose/` (~2,500+ lines new)

### ContextProvider (1,122 lines)

Assembles context for agent prompts:
- Gathers relevant source files based on task description
- Resolves code symbols referenced in the task
- Includes recent change history (git diff/log)
- Ranks context by relevance and fits within token budget
- Supports `Placement` (system/user) and `SectionPriority` (required/preferred/optional)

### SymbolResolver (616 lines)

Code symbol resolution for prompt context:
- Resolves function, struct, trait, and module references
- Extracts signatures and doc comments
- Supports cross-crate resolution

### TaskBrief (365 lines)

Generates structured task briefs for agents:
- Task description and acceptance criteria
- Dependency context (what prior tasks produced)
- File scope (which files the agent should modify)
- Gate expectations (what validation will run)

### RolePrompts (364 lines)

Role-specific prompt templates for the 6-layer system prompt builder:
- Implementer -- code generation focused
- Reviewer -- code review and quality
- Strategist -- architecture and planning
- Researcher -- literature search and analysis
- Debugger -- error diagnosis and fix
- Templates integrated via `RoleSystemPromptSpec`

### PromptComposer

Assembles `PromptSection`s into final prompts:
- Sections carry `Placement` (system vs user) and `SectionPriority`
- Budget-aware: trims optional sections when token limit approached
- Produces `PlanArtifacts` and `TaskContext` for executor consumption

---

## 10. Orchestrator

**Directory:** `crates/roko-orchestrator/` (~500+ lines modified)

- **Cross-plan dependency support** -- tasks can declare `depends_on` tasks in other plans; executor resolves cross-plan edges during DAG construction
- **Worktree improvements** (115 lines) -- `WorktreeConfig` and `WorktreeManager` for isolated git worktrees per plan execution
- **Lifecycle integration tests** (177 lines) -- end-to-end executor lifecycle: plan -> dispatch -> gate -> persist -> advance
- **Post-merge improvements** (145 lines) -- `PostMergeRunner` handles git operations after successful plan completion
- **DAG improvements** -- better parallel scheduling with dependency-aware task ordering
- **Event log** -- `EventLog`, `EventLogSnapshot`, `EventKind` for durable execution history

---

## 11. Gate system

**Directory:** `crates/roko-gate/` (~400+ lines new)

### AdaptiveThreshold (217 lines)

Adaptive gate threshold system:
- Tracks pass rates per gate rung using exponential moving average
- Adjusts thresholds up when pass rate is high (tighten quality), down when low (avoid blocking)
- Configurable alpha (learning rate) and min/max bounds
- Integrates with `roko-learn` persistence layer

### Gate improvements

- Symbol gate -- validates that expected symbols exist in modified files
- Verify chain gate -- validates chain of custody for modifications
- Integration gate -- validates cross-crate compatibility after changes

---

## 12. Additional changes

### roko-golem (new crate, phase 2+ scaffolding)

Modules for future chain witness and autonomous agent capabilities:
- `chain_witness` -- on-chain attestation of agent actions
- `daimon` -- autonomous agent daemon
- `dreams` -- long-term planning and goal setting
- `grimoire` -- knowledge base and memory
- `hypnagogia` -- sleep/wake cycle management

### roko-fs observability

- `observability.rs` (162 lines) -- `FsObservabilitySinks` for file system operation metrics
- Tool metrics sink (244 lines) -- tracks tool execution latency, success/failure rates

### CLI additions

- `tui/` -- text-mode dashboard with pages (efficiency view, operations view)
- `task_parser.rs` (611 lines) -- TOML task parser for `tasks.toml` files in plan directories
- `index.rs` (446 lines) -- codebase indexer for context assembly

### Plans

- P06: Process management plan with `tasks.toml`
- P07: Autofix retry plan with `tasks.toml`
- W01: Wire system prompts plan with `tasks.toml`

### Configuration

- `roko.toml` (70 lines) -- project configuration: server bind/port, agent settings (model, MCP config, timeout), gate thresholds, learning toggles

### Docker

- `docker/worker.Dockerfile` -- worker container for cloud execution
- Roko Dockerfile updates for `roko serve` deployment

---

## Fixes

- **Agent registration** -- use string pubkey instead of byte array in JSON-RPC registration
- **Dashboard zeros** -- fork block, task stats, and agent stats were showing zeros because `ForkState` did not track the fork block and `mine_block()` used a frozen timestamp
- **Test failure** -- `agent_http_endpoints_via_full_server` had a response envelope key mismatch (`data` vs `items`)
- **Stale static files** -- added `no-cache, must-revalidate` Cache-Control for static dashboard files
- **Pheromone jitter** -- incremental DOM updates instead of full re-renders, canvas size caching, CSS containment (`contain: layout style paint`)
- **Knowledge graph too small** -- increased canvas dimensions, improved force simulation parameters
- **Agent topology clustering** -- increased repulsion force and link distance for readability

---

## Closes / addresses

- Issue #5: agent registry with HTTP/RPC/WS endpoints

---

## Test coverage

| Suite | Count | What |
|-------|-------|------|
| Library unit tests | 287 | Core crate tests across all 18 crates |
| Integration tests | 13 | Executor lifecycle, safety, learning, orchestrator |
| HTTP API tests | 37 | Full REST endpoint coverage (mirage-rs) |
| Roko bridge tests | 6 | Chain substrate, HDC substrate, simulation gate |
| End-to-end tests | 4 | CLI -> orchestration -> gate -> persist |
| **Total** | **347** | |

---

## How to test

```bash
# Build everything
cd /Users/will/dev/nunchi/roko/roko
rustup update stable  # need 1.91+
cargo build --workspace

# Run all tests
cargo test --workspace

# Run clippy
cargo clippy --workspace --no-deps -- -D warnings

# Start mirage-rs dashboard
cargo run -p mirage-rs --features chain,roko

# Run 20-agent simulation against the dashboard
cargo run -p mirage-rs --features chain,roko --example agent_simulation -- \
    --rpc-url http://127.0.0.1:8545

# Start roko serve API
cargo run -p roko-cli -- serve

# Execute the self-hosting loop
cargo run -p roko-cli -- prd idea "test idea"
cargo run -p roko-cli -- prd list
cargo run -p roko-cli -- plan run plans/
```

---

## What comes after this

With this PR merged, the self-hosting loop is wired end-to-end. Remaining work:

1. **Interactive TUI** -- wire ratatui into the text-mode dashboard scaffold
2. **Automatic plan generation** -- trigger `prd plan` when a PRD is published
3. **Feedback loop closure** -- failed task gates feed back into plan generator for automatic re-planning (partially wired in this PR, needs end-to-end testing)

After those three items, roko can fully self-host: read its own PRDs, generate plans, execute them with agents, validate results with gates, learn from failures, and iterate without human intervention.
