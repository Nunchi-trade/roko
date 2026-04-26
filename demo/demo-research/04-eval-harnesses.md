# Evaluation harness

**Default answer: `roko-orchestrator` is the harness.** It loads task DAGs,
dispatches to backends, runs the gate pipeline, scores outcomes, and
persists everything to `.roko/`. Every piece you'd otherwise get from
Inspect AI + Langfuse is already wired. See `08-reuse-map.md` for the
inventory.

This file covers (a) the recommended approach — using `run_plan()` as the
benchmark runner — and (b) external harnesses if you want them as
*exporters* rather than primary infrastructure.

## What a harness has to do

1. **Load tasks** — read a benchmark dataset
2. **Run** — execute one task per (backend, rep) cell
3. **Score** — apply a deterministic checker to outputs
4. **Persist** — write results in a queryable shape
5. **Display** — show progress + final aggregates

roko already does all five.

## Primary: roko-orchestrator as harness

### The runner

`crates/roko-cli/src/orchestrate.rs::run_plan()` already does:

```rust
pub async fn run_plan(plan_dir: &Path, opts: RunOpts) -> Result<ExecutionSummary>
```

`ExecutionSummary` returns per-task cost, pass rate, duration. Per-turn
metadata persists to:

- `.roko/learn/efficiency.jsonl` — every turn's tokens/cost/timing
- `.roko/memory/episodes.jsonl` — full turn transcripts + gate verdicts
- `.roko/memory/task-metrics.jsonl` — per-task aggregate

The runner is **callable directly from Rust** (not just CLI). For a
benchmark, you call it once per (benchmark, backend) pair:

```rust
// illustrative shape, not implementation
for backend in ["roko", "anthropic", "claude-agent-sdk", "langgraph", "crewai"] {
    let opts = RunOpts {
        force_backend: Some(backend.into()),
        plan_id: format!("bench-{}-{}", bench_name, backend),
        ..default()
    };
    let summary = run_plan(&tasks_dir, opts).await?;
    // results already in .roko/learn/efficiency.jsonl tagged with this plan_id
}
```

### Task definition

Tasks already use a stable TOML schema (existing `tasks.toml` format
loaded by every plan):

```toml
# illustrative — based on existing task schema in roko-orchestrator
[[tasks]]
id = "swe-mini-001"
title = "django__django-11099 — fix UsernameValidator bug"
prompt = "Resolve issue #11099. See repo at base SHA 7f9e2."
deps = []
gates = ["compile", "test", "bench-acceptance"]
role = "python-engineer"
budget_usd = 0.50
budget_seconds = 600
metadata = { repo = "django/django", base_sha = "7f9e2..." }
```

For SWE-bench, write a small loader that converts each HuggingFace
instance into a row. For custom roko-bench, hand-author 20-30 rows from
your backlog.

### Scoring via gates (no LLM judge)

The gate pipeline already supports custom gates. Add a benchmark-specific
gate that runs the task's deterministic check:

```rust
// crates/roko-gate/src/gates/bench_acceptance.rs
pub struct BenchAcceptanceGate;

impl Gate for BenchAcceptanceGate {
    fn name(&self) -> &str { "bench-acceptance" }

    fn check(&self, ctx: &GateContext) -> GateVerdict {
        let task_meta = ctx.task.metadata.as_ref();
        match task_meta.bench_kind.as_str() {
            "swe-bench" => run_swe_bench_test(ctx),
            "humaneval" => run_humaneval_test(ctx),
            "roko-bench" => run_cargo_test(ctx, &task_meta.test_name),
            _ => GateVerdict::pass("no-bench"),
        }
    }
}
```

Register it in the gate registry and any task with `gates =
["bench-acceptance"]` is scored automatically. The verdict (pass/fail +
signature) lands in `Episode.gate_verdicts`.

This is strictly better than an LLM judge for code-correctness tasks — it's
deterministic, cheap, and fast.

### Multi-rep + pass^k

`run_plan()` already supports running the same plan multiple times via
the `--reps N` flag (or by calling it in a loop). Each rep gets a unique
`plan_id` so the efficiency events are distinguishable.

For τ-bench-style `pass^k`:

```rust
let mut outcomes_per_task: HashMap<TaskId, Vec<bool>> = ...;
for rep in 0..K {
    let summary = run_plan(&tasks_dir, opts.with_rep(rep)).await?;
    for ts in summary.task_summaries {
        outcomes_per_task.entry(ts.task_id).or_default().push(ts.passed);
    }
}
let pass_k = outcomes_per_task.values()
    .filter(|reps| reps.iter().all(|&r| r))
    .count() as f64 / outcomes_per_task.len() as f64;
```

### Scoring aggregation

Already done by `crates/roko-learn/src/task_metric.rs::compute_headlines()`
and `crates/roko-learn/src/aggregate.rs`. Returns trends, percentiles,
pass-rates over windows.

For per-backend comparison, query `CostsDb`:

```rust
// crates/roko-learn/src/costs_db.rs already exposes these
let summary = costs_db.query_by_plan(&format!("bench-{}-{}", bench, backend))
    .summarize();
// CostSummary { total_cost_usd, total_input_tokens, success_rate, ... }
```

### Composite quality: C-factor

`crates/roko-learn/src/cfactor.rs::CFactor` is roko's pre-built composite
score, computed from a window of episodes. It includes:

- gate_pass_rate
- cost_efficiency (inverse of cost-per-success)
- time_efficiency
- tool_utilization
- model_diversity
- cache_effectiveness
- feedback_velocity
- error_recovery_rate

For the bench, compute C-factor per backend by filtering episodes by
`backend`. Other frameworks will score lower on `gate_pass_rate` because
they don't gate (= 0 unless we add an external gate after their run —
worth doing for fairness, see below).

## Adapting external benchmarks

### SWE-bench Verified Mini

```
demo/demo-research/03-investor/
├── tasks/
│   └── swe-bench-mini.toml         # generated from HF dataset
├── load_swe_bench.py               # one-time loader: HF → tasks.toml
└── adapters/
    └── swe_bench_grader.rs         # custom gate that runs swe-bench-runner
```

The grader is a thin gate wrapper around the official SWE-bench evaluation
script (which applies the patch and runs pytest in a Docker container).
Reuses sandboxing infrastructure; emits a `GateVerdict`.

### τ²-bench retail

```
demo/demo-research/03-investor/
├── tasks/tau-retail.toml
├── load_tau_bench.py
└── adapters/
    └── tau_bench_grader.rs         # final-state DB diff vs gold
```

τ-bench is conversational; the agent talks to a simulated user. The
adapter wraps Sierra's runner and translates outcomes into roko gate
verdicts.

### HumanEval / MBPP

Trivial — the grader is "exec the function, run the test cases, return
pass/fail." 50 lines of Rust + a tiny Python sandbox.

### Custom roko-bench

Hand-authored `tasks.toml`. Each task's gate is a `cargo test -p X` or a
`grep -q PATTERN file.rs`. No adapter needed.

## Integrating competing frameworks (key architectural move)

Each competitor framework becomes a **roko backend**. roko's existing
dispatcher invokes it like any other backend; efficiency events are written
in the standard schema. This is the architectural move that lets us reuse
the entire harness.

### Backend pattern

```
crates/roko-agent/src/backends/langgraph.rs    NEW
demo/demo-research/adapters/langgraph.py        NEW
```

The Rust backend `subprocess.exec`s the Python adapter, which:

1. Reads the prompt + tool spec from stdin (JSON)
2. Runs LangGraph with the same model (via Anthropic SDK or LiteLLM)
3. Returns `{ output, tool_calls, usage: { input, output, cache_read, cache_write }, time_ms }`

The Rust backend converts that JSON into an `AgentEfficiencyEvent` and
emits it through the existing pipeline. The event hits efficiency.jsonl
with `backend = "langgraph"`. Done.

```python
# demo/demo-research/adapters/langgraph.py — illustrative
import sys, json, time
from langgraph.graph import StateGraph
from langchain_anthropic import ChatAnthropic

req = json.loads(sys.stdin.read())
model = ChatAnthropic(model=req["model"], anthropic_api_key=...)
# build graph from req["tools"]; invoke with req["prompt"]
start = time.time()
result = app.invoke({"input": req["prompt"]})
elapsed_ms = int((time.time() - start) * 1000)

print(json.dumps({
    "output": result.output,
    "tool_calls": result.tool_calls,
    "usage": {
        "input_tokens": ...,        # from ChatAnthropic callback or Anthropic response.usage
        "output_tokens": ...,
        "cache_read_tokens": ...,
        "cache_write_tokens": ...,
    },
    "time_ms": elapsed_ms,
}))
```

For frameworks that don't expose token counts directly, route their
calls through LiteLLM (point `ChatAnthropic` at `http://localhost:4000`)
and read counts from LiteLLM's response. This is the only place LiteLLM
is needed — internal to the adapters.

### Gate parity for competing frameworks

After a competitor backend produces an output, run the **same gate
pipeline** on that output. The gate doesn't care which backend produced
the patch — it just runs `cargo test`.

This lets you fairly compare `gate_pass_rate` across all backends. roko
will score higher because of replan-on-gate-failure (it gets retries),
but the metric is meaningful for everyone.

## External harnesses as exporters (optional)

If you want to publish to community standards in addition to roko's
storage, write *exporters*, not replace the primary harness.

### Inspect AI export

[Inspect AI](https://inspect.aisi.org.uk/) has a JSON log format that
their viewer + ecosystem can consume. Write a small exporter:

```rust
// crates/roko-cli/src/bin/export_inspect.rs
fn main() {
    let episodes = EpisodeLogger::read_all(".roko/memory/episodes.jsonl")?;
    for ep in episodes.filter(|e| e.task_id.starts_with("bench-")) {
        write_inspect_log_entry(&ep)?;  // schema match
    }
}
```

Now `inspect view start` can browse roko's runs, and the runs are
shareable with anyone using Inspect AI's tooling. Roko remains the
source of truth.

### Langfuse export

[Langfuse](https://langfuse.com/) is a strong hosted/self-hosted trace
viewer. If you want non-engineers to browse traces in a web UI, run
Langfuse alongside and export from `.roko/learn/efficiency.jsonl` and
`.roko/memory/episodes.jsonl`:

```python
# demo/demo-research/exporters/to_langfuse.py
from langfuse import Langfuse
import json
lf = Langfuse(host=..., public_key=..., secret_key=...)
for line in open(".roko/learn/efficiency.jsonl"):
    evt = json.loads(line)
    lf.trace(name=evt["task_id"], user_id=evt["backend"], ...)
    lf.generation(... usage=evt["..."], cost=evt["cost_usd"])
```

This runs once per nightly batch. Langfuse's dashboard shows the same
slices (cost by backend, latency, etc.) as roko's TUI but in a browser.

### When to export, when not to

| Scenario | Export? |
|---|---|
| Internal demo, in-room | No |
| Recorded asciinema for README | No |
| Investor pitch with screen share | TUI projected; export optional |
| Public leaderboard submission (SWE-bench) | Yes — required by leaderboard format |
| Sharing traces with external collaborators | Yes — Langfuse is friendlier than `.jsonl` |
| Long-running nightly with alerting | Maybe — Grafana on Prometheus from LiteLLM is cheaper than Langfuse |

## Comparison: roko vs. external harnesses

What you'd otherwise have to assemble:

| Component | External stack | roko equivalent |
|---|---|---|
| Task definitions | YAML/JSON datasets | `tasks.toml` (existing schema) |
| Run executor | Inspect AI Solver | `roko-orchestrator::ParallelExecutor` |
| Sandboxing | Inspect AI Docker | roko-runtime + agent isolation |
| Scoring | Inspect AI Scorer | gate pipeline (`crates/roko-gate`) |
| Per-call traces | Langfuse SDK | `Episode` + `EpisodeLogger` |
| Cost tracking | LiteLLM proxy | `cost_table.rs` + `costs_db.rs` |
| Live UI | Streamlit / Phoenix | ratatui TUI bench tab |
| Persistence | Postgres | JSONL on disk |
| Aggregation | Custom SQL | `CostsDb::query_by_*` |
| Composite metric | n/a | `CFactor` |
| Adaptive routing | n/a | `CascadeRouter` |

Net new dependencies if we go roko-native: zero (Rust crates only).
Net new dependencies if we use external: 4-5 services (LiteLLM, Langfuse,
Postgres, Inspect AI, optionally Phoenix/Grafana).

## Proposed directory layout

```
demo/demo-research/
├── 00-08 *.md
├── tasks/
│   ├── humaneval.toml             # 164 tasks
│   ├── bfcl.toml                  # generated
│   ├── swe-bench-mini.toml        # 50 tasks
│   ├── tau-retail.toml            # 100 tasks
│   └── roko-bench.toml            # 25 tasks (hand-authored)
├── adapters/                      # competitor framework wrappers
│   ├── langgraph.py
│   ├── crewai.py
│   ├── autogen.py
│   ├── oai_agents.py
│   └── claude_agent_sdk.py
├── graders/                       # benchmark-specific gate plugins
│   ├── humaneval.rs               # → crates/roko-gate eventually
│   ├── swe_bench.rs
│   ├── tau_bench.rs
│   └── roko_bench.rs
├── exporters/                     # optional
│   ├── to_inspect.rs
│   └── to_langfuse.py
└── recipes/                       # one dir per recipe (see 06-recipes.md)
    ├── 01-five-frame/
    ├── 02-nightly/
    ├── 03-investor/
    └── 04-swe-bench-public/
```

Code that lands in `crates/` (new backends, new gates, new TUI tab) is
production-grade roko code. Code that lands in `demo/demo-research/` is
benchmark-specific scaffolding (task loaders, framework adapters,
exporters).

## Effort estimate

| Component | Lines | Time |
|---|---|---|
| 4 competitor backends (Rust) | ~400 | 1 day |
| 4 framework adapters (Python) | ~200 | 0.5 day |
| 4 benchmark graders (Rust gates) | ~600 | 2 days |
| Task loaders (HF dataset → toml) | ~300 | 1 day |
| TUI bench tab (see 05-realtime-visualization.md) | ~300 | 2 days |
| Optional: Langfuse exporter | ~100 | 0.5 day |
| Optional: Inspect AI exporter | ~150 | 0.5 day |
| **Total core** | ~1,800 | **6-7 days** |

Compare with "build a Python harness from Inspect AI + LiteLLM + Langfuse
+ custom adapters" which is more like 3-4 weeks and adds permanent
infrastructure.
