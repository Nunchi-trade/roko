# Reuse map — what's already in roko

**Read this before designing any new infrastructure.** roko has ~95% of the
benchmark stack already built. The job is wiring + adapters, not new tools.

This file is the canonical map of what exists, where, and how to reuse it.
Other files in this directory (especially `03-cost-tokens.md`,
`04-eval-harnesses.md`, `05-realtime-visualization.md`) treat this as the
source of truth.

## TL;DR

| Need | What you'd reach for externally | What roko already has |
|---|---|---|
| Cost gateway | LiteLLM | `roko-agent` dispatcher with 8 backends, `cost_table.rs`, `costs_db.rs` |
| Token tracking | Native API `usage` + custom logger | `AgentEfficiencyEvent` in `.roko/learn/efficiency.jsonl` (20+ fields incl. cache, reasoning, TTFT) |
| Trace storage | Langfuse | `Episode` in `.roko/memory/episodes.jsonl` (full turn log + gate verdicts) |
| Run orchestration | Inspect AI Tasks/Solvers | `ParallelExecutor` in `roko-orchestrator`, `run_plan()` in `orchestrate.rs` |
| Scoring | Inspect Scorer / LLM judge | 11 gates × 7 rungs in `roko-gate`, gate verdicts persisted |
| Live dashboard | Streamlit / Grafana | ratatui TUI with 10 tabs, file watcher, JSONL tailer |
| Live event stream | OpenTelemetry → Tempo | `/ws` WebSocket on roko-serve, ring buffer + replay |
| Adaptive routing | n/a | `CascadeRouter` with 3-stage adaptive selection |
| Composite quality score | n/a | `CFactor` in `roko-learn` (8 components) |

The only things you genuinely don't have:
- Adapters that wrap **competing frameworks** (LangGraph, CrewAI, etc.)
  as roko backends so they emit the same efficiency events.
- A **bench tab** in the TUI that aggregates per-framework comparisons
  (the data is all there; the view isn't).
- A **task set** for benchmarks (SWE-bench instance loader, custom
  roko-bench tasks.toml).

That's the actual scope. Nothing else needs to be built.

## 1. TUI dashboard — extend, don't replace

**Path:** `crates/roko-cli/src/tui/`

Already implemented:

| Tab | Key | Shows |
|---|---|---|
| Dashboard | F1 | Overview, health, plan progress, cost |
| Plans | F2 | Plan tree, task progress, wave overview |
| Agents | F3 | Agent output, diffs, token burn, parallel pool |
| Git | F4 | Branch tree, commit graph, worktrees |
| Logs | F5 | Scrollable log viewer with filtering |
| Config | F6 | Config editor / effective config view |
| Inspect | F7 | Engram DAG, episode replay |
| Marketplace | F8 | Job browser |
| Atelier | F9 | PRD workshop, plan progress |
| Learning | F10 | Cascade router, model routing, efficiency |

**Widgets in active use:** `Table`, `Sparkline`, `BarChart`, `Gauge`,
`Chart`, `Paragraph`. All the chart shapes the demo needs.

**Data pipeline (already wired):**

- `tui/fs_watch.rs` — `notify::RecommendedWatcher` over `.roko/`
- `tui/jsonl_tailer.rs` — JSONL tail with cursor for streaming updates
- `tui/dashboard.rs` — composes data loading; uses `FileStamp` (mtime+size)
  for cheap change detection
- `tui/subscriptions.rs` — file-watcher subscription patterns

**What to add for benchmarking:**

A new tab `F11 Bench` (or fold into F10 Learning) following the same shape
as existing views:

```
crates/roko-cli/src/tui/views/bench_view.rs   # NEW
crates/roko-cli/src/tui/tabs.rs               # add Tab::Bench enum variant
crates/roko-cli/src/tui/mod.rs                # register dispatch
```

The view reads the same `efficiency.jsonl` and `episodes.jsonl` that other
views already read, just groups by `backend` instead of by `agent_id`. So
**every metric you'd want to display is already in the file** — you only
need to add the rendering.

**Concrete display ideas, all using widgets already in roko:**

- `BarChart` — USD cost per backend (existing widget)
- `Sparkline` — pass-rate over time per backend (existing widget)
- `Table` — per-task heatmap with cell colors (existing widget)
- `Chart` — Pareto scatter (cost vs pass-rate) (existing widget)

No new dependencies. Strict subset of what F1 Dashboard already does.

## 2. Cost & token tracking — already authoritative

**Path:** `crates/roko-learn/src/efficiency.rs`

The `AgentEfficiencyEvent` struct is *more* detailed than what LiteLLM
emits. Fields you already have per turn:

```rust
// from crates/roko-learn/src/efficiency.rs
pub struct AgentEfficiencyEvent {
    // Identity
    pub agent_id: String,
    pub role: String,
    pub backend: String,        // "claude-cli" / "claude-api" / "openai" / ...
    pub model: String,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,

    // Tokens (richer than most providers)
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,

    // Cost (with cache discount built in)
    pub cost_usd: f64,
    pub cost_usd_without_cache: f64,

    // Prompt composition
    pub prompt_sections: Vec<...>,
    pub total_prompt_tokens: u64,
    pub system_prompt_tokens: u64,

    // Tool utilization
    pub tools_available: Vec<String>,
    pub tools_used: Vec<String>,
    pub tool_calls: Vec<...>,

    // Timing
    pub wall_time_ms: u64,
    pub time_to_first_token_ms: u64,
    pub was_warm_start: bool,

    // Outcome
    pub iteration: u32,
    pub gate_passed: bool,
    pub outcome: String,
    pub gate_errors: Vec<...>,
    pub model_used: String,
    pub frequency: f64,
    pub strategy_attempted: String,
    pub timestamp: i64,
}
```

Helper methods on the struct: `cache_hit_rate()`, `tool_utilization()`,
`cache_savings_usd()`, `total_tokens()`.

**Persisted to:** `.roko/learn/efficiency.jsonl` — append-only, parse-tolerant.

**Cost computation:**
- `crates/roko-learn/src/cost_table.rs` — model price tables
  (input/output/cache_read/cache_write per model)
- `crates/roko-learn/src/costs_db.rs` — in-memory `CostsDb` with
  `query_by_model()`, `query_by_role()`, `query_by_plan()`,
  `query_by_complexity()`, `query_by_time_range()`, `summarize()`

**For the demo:** queries you'd write against LiteLLM's spend log run
unmodified against `CostsDb`. Pass-rate, cost-per-success, latency
percentiles — already first-class queries.

## 3. Episode log — already a trace

**Path:** `crates/roko-learn/src/episode_logger.rs`

```rust
pub struct Episode {
    pub kind: String,
    pub id: String,
    pub timestamp: String,
    pub agent_id: String,
    pub task_id: String,
    pub input_signal_hash: String,
    pub output_signal_hash: String,
    pub episode_id: String,
    pub agent_template: String,
    pub model: String,
    pub backend: String,
    pub outcome: String,
    pub wall_ms: u64,
    pub iteration: u32,
    pub gate_verdicts: Vec<GateVerdict>,
    pub usage: Usage,             // tokens + cost (with/without cache)
    pub extra: HashMap<String, Value>,  // 16KB cap, extensible
}
```

`EpisodeLogger::read_all(path)` parses the file tolerantly. This is your
trace store — exactly what Langfuse would give you, without running another
service.

`gate_verdicts: Vec<GateVerdict>` is already the "score" column for any
benchmark — `gate_passed=true` for the final verdict on the final rung is
the boolean correctness signal.

## 4. Plan executor — already a harness

**Path:** `crates/roko-orchestrator/src/executor/mod.rs` + `crates/roko-cli/src/orchestrate.rs`

`ParallelExecutor` is a pure state machine:

```rust
loop {
    let actions = executor.tick();   // pure: returns Vec<ExecutorAction>
    for action in actions {
        let event = perform(action).await;  // does I/O
        executor.apply_event(event);        // pure: advances state
    }
    if executor.is_done() { break }
}
```

`crates/roko-cli/src/orchestrate.rs::run_plan()` is the wired version that
loads tasks.toml and runs the loop. **It's callable directly from Rust** —
not just from CLI. Returns:

```rust
pub struct ExecutionSummary {
    pub succeeded: bool,
    pub plan_name: String,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
    pub task_summaries: Vec<TaskSummary>,  // per-task: iterations, gate_pass_rate, outcome, cost
}
```

This is a ready-made benchmark harness. You write your tasks as tasks.toml,
call `run_plan(plan_dir)`, and get back per-task cost + gate pass rate.

**Plan/task format** (already supported):

```toml
# tasks.toml
[[tasks]]
id = "swe-001"
title = "Fix issue #1234"
prompt = "..."
deps = []
gates = ["compile", "test"]   # which gates must pass
role = "rust-engineer"
budget_usd = 0.50
budget_seconds = 600
```

The DAG (`crates/roko-orchestrator/src/dag.rs`) handles parallel
execution, conditional edges, merge gates. Already wired.

## 5. Gate pipeline — already a scorer

**Path:** `crates/roko-gate/src/`

11 gate types × 7-rung pipeline. Each gate emits a `GateVerdict { gate,
passed, signature }`. Verdicts persist in `Episode.gate_verdicts`.

**For benchmarks:** custom scorers don't need a new harness. Add a gate
type if needed:

```rust
// illustrative; gate trait already exists
impl Gate for BenchAcceptanceGate {
    fn check(&self, ctx: &GateContext) -> GateVerdict {
        // run cargo test or pytest, return verdict
    }
}
```

Wire it into the gate registry. Now any task can declare `gates =
["bench-acceptance"]` and the executor scores it automatically.

## 6. HTTP control plane — already a remote runner

**Path:** `crates/roko-serve/`

Routes relevant for benchmarking:

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/run` | Spawn a run with a prompt; returns run_id |
| GET | `/api/run/{id}/status` | Poll for completion + summary |
| GET | `/api/learning/efficiency` | Aggregated efficiency metrics |
| GET | `/api/learning/cfactor` | Composite quality score |
| GET | `/api/learning/cascade-router` | Routing decisions |
| GET | `/ws` | WebSocket: live event stream with replay |

The WebSocket subscription protocol supports filter patterns
(`projection:gate_pipeline`, `topic:agent.*`) and back-pressure modes
(`at_most_once`, `coalesce`, `resume_required`).

**For a benchmark runner:** POST tasks to `/api/run`, subscribe via `/ws`,
let the existing TUI (or any web client) display events live.

## 7. Multi-backend dispatch — competitors plug in here

**Path:** `crates/roko-agent/src/dispatcher/`

Backends supported today:

1. Claude CLI (subprocess + MCP)
2. Claude API (Anthropic SDK direct)
3. Codex (legacy OpenAI)
4. Cursor (embedded editor)
5. OpenAI-compatible (generic OAI/Together/etc.)
6. Ollama (local)
7. Gemini (Google)
8. Perplexity (research)

Backend selection happens via `CascadeRouter` (3-stage adaptive). The
dispatcher exposes `dispatch_batch()` callable from Rust.

**For competitor framework comparison:** add new backends `langgraph`,
`crewai`, `autogen`, `oai-agents` that each subprocess into a small
Python adapter. The adapter:

1. Receives the prompt + tool spec via stdin/JSON
2. Runs the framework with the same model (use Anthropic SDK direct
   inside the adapter, or LiteLLM if you want unified billing)
3. Returns a `ToolResult` with cost/usage parsed from the framework's
   API responses

```
crates/roko-agent/src/backends/
  ├── claude_cli.rs      (existing)
  ├── claude_api.rs      (existing)
  ├── ...
  ├── langgraph.rs       NEW — subprocess to demo/demo-research/adapters/langgraph.py
  ├── crewai.rs          NEW
  └── oai_agents.rs      NEW
```

Each new backend is ~100 lines of Rust + ~50 lines of Python adapter.

**Result:** roko's existing pipeline tracks **the competitor's** runs in
the **same** efficiency.jsonl + episodes.jsonl. No separate trace store, no
separate cost ledger, no separate dashboard. The TUI bench tab just groups
by `backend`.

This is the architectural move that turns the demo from "build a new
benchmarking system" into "wrap competitors as backends and read the
existing dashboard."

## 8. Storage — already canonical

```
.roko/
├── state/
│   ├── executor.json          # crash-recoverable executor snapshot
│   └── circuit-breaker.json
├── memory/
│   ├── episodes.jsonl         # turn-by-turn full traces
│   └── task-metrics.jsonl     # per-task aggregate
├── learn/
│   ├── efficiency.jsonl       # per-turn cost/tokens/timing
│   ├── cascade-router.json    # adaptive routing state
│   ├── experiments.json       # prompt A/B
│   ├── gate-thresholds.json   # adaptive gate EMAs
│   ├── provider-health.json   # circuit breaker
│   ├── latency-stats.json     # per-model percentiles
│   └── skills.json            # skill library
├── neuro/                     # durable knowledge
├── prd/                       # PRD lifecycle
├── plans/{id}/tasks.toml      # plan DAGs
├── research/                  # research artifacts
└── signals.jsonl              # signal log
```

All append-only JSONL or single-file JSON. All already read by the TUI
file watcher. The benchmark dataset lives in this tree by default.

## 9. Existing eval-shaped code

| Path | What |
|---|---|
| `crates/roko-learn/src/verdict_scorer.rs` (19K) | Scores gate verdicts; failure-pattern clustering |
| `crates/roko-learn/src/task_metric.rs` (21K) | Aggregates task metrics; `compute_headlines()` |
| `crates/roko-learn/src/cfactor.rs` (62K) | Composite C-factor with 8 subscores |
| `crates/roko-learn/src/aggregate.rs` | Trend computation across episodes |
| `crates/roko-cli/tests/` | Integration tests that run full plans |
| `crates/roko-orchestrator/tests/lifecycle.rs` | End-to-end plan execution test |
| `demo/demo-resources/benchmark-flow/` | Existing demo benchmark docs |

Nobody calls these "the eval framework" but collectively they are one.

## 10. The new code we'd write

Concretely, the deltas to ship a working benchmark demo:

**A. New backends** (~500 lines total, 8 files)
```
crates/roko-agent/src/backends/langgraph.rs
crates/roko-agent/src/backends/crewai.rs
crates/roko-agent/src/backends/oai_agents.rs
crates/roko-agent/src/backends/claude_agent_sdk.rs
demo/demo-research/adapters/langgraph.py
demo/demo-research/adapters/crewai.py
demo/demo-research/adapters/oai_agents.py
demo/demo-research/adapters/claude_agent_sdk.py
```

**B. New TUI tab** (~300 lines, 1 file + 2 small edits)
```
crates/roko-cli/src/tui/views/bench_view.rs    NEW
crates/roko-cli/src/tui/tabs.rs                EDIT (1 enum variant)
crates/roko-cli/src/tui/mod.rs                 EDIT (1 dispatch arm)
```

**C. Task sets** (just data files)
```
demo/demo-research/tasks/roko-bench.toml
demo/demo-research/tasks/swe-bench-mini.toml   (loader from HF dataset)
```

**D. (Optional) HTTP route for remote benchmark UI** (~100 lines)
```
crates/roko-serve/src/routes/bench.rs
```

That's the entire scope. Everything else is reuse.

## How other docs in this directory should treat this

- `01-benchmarks.md` — task-set definitions, untouched (still need them).
- `02-frameworks.md` — competitor frameworks become **backends**.
- `03-cost-tokens.md` — primary record is `efficiency.jsonl`; LiteLLM only
  for adapter-internal use if needed.
- `04-eval-harnesses.md` — primary harness is `roko-orchestrator`;
  external (Inspect AI, Langfuse) optional for export.
- `05-realtime-visualization.md` — primary display is the new TUI tab;
  external (Streamlit, Grafana) optional.
- `06-recipes.md` — each recipe specifies "what's reused" vs "what's new"
  and the new is small.
- `07-methodology.md` — audit trail is automatic via episodes + gate
  verdicts; methodology rules unchanged.
