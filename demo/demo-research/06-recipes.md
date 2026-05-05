# Demo recipes

Four concrete combinations of benchmark + frameworks + harness + display.
Each is a self-contained "what we'd actually build." Listed cheapest-first.

**All recipes assume the reuse pattern from `08-reuse-map.md`:**
roko-orchestrator is the harness, ratatui TUI is the live display,
competing frameworks are wrapped as roko backends, scoring is via the
gate pipeline. Recipes only list **net-new code** beyond that shared
foundation.

```
recipe                       audience           wall time   USD/run   wow
──────────────────────────────────────────────────────────────────────────
1. five-frame side-by-side   live demo          5 min       $1-3      high
2. nightly roko-bench        internal CI        45 min      $5-15     low
3. investor-grade pareto     pitch deck         3 hours     $50-100   high
4. swe-bench-mini-public     public leaderboard 6 hours     $80-150   med
```

Each recipe maps to a subdirectory you'd create when implementing.

---

## Recipe 1: Five-frame side-by-side

**Goal.** A 5-minute live demo. One representative task. Each framework in
its own pane. Token rain visible. Cost meter ticks up. roko is visibly the
fastest *and* cheapest.

### Task

A single, well-scoped, ~30-second-per-framework task. Examples:

- "Add a `--dry-run` flag to `roko plan run`"
- "Fix the typo in `crates/roko-cli/src/runner/mod.rs:42`"
- "Add a unit test for `cascade_router::route_for_task`"

Pre-validated: roko has solved this before. Frozen at a known SHA.

### Frameworks

1. roko (`cargo run -p roko-cli -- run "<task>"`)
2. Anthropic SDK direct (manual loop, ~50 lines)
3. Claude Agent SDK (`claude_agent_sdk.query`)
4. OpenAI Agents SDK
5. LangGraph

### What's reused (the heavy lifting)

- **Harness**: `roko-orchestrator::run_plan()` — already invokable from Rust
- **Cost/token tracking**: `AgentEfficiencyEvent` → `.roko/learn/efficiency.jsonl`
- **Gate scoring**: existing 7-rung gate pipeline + new `bench-acceptance` gate
- **Live display**: TUI bench tab (built once, used by all four recipes)
- **WebSocket events**: roko-serve `/ws` already streams `AgentOutput`,
  `TaskStarted`, `GateResult` — used by tmux pane tailers

### Setup (net-new only)

```
demo/demo-research/recipes/01-five-frame/
├── README.md
├── tasks.toml         # the one task, frozen (uses existing schema)
├── run.sh             # tmux split + roko bench run
└── demo.tape          # vhs script for reproducible recording
```

The Rust backend impls + Python adapters live under `crates/roko-agent/src/backends/`
and `demo/demo-research/adapters/` — built once, shared across all recipes.

### Display

The live demo is just the TUI's F11 Bench tab projected. For maximum
visual impact, add a tmux split that also tails the per-backend agent
output streams:

- left pane (60% width): `roko dashboard` with F11 Bench tab active
- right pane (40% width): split vertically, one row per backend, each
  tailing `.roko/streams/<plan-id>/<backend>.log` (existing per-backend
  log files written by roko-serve event handlers)

No `dashboard.py` needed. No streamlit. The data plane is the file
system; the display plane is the TUI.

### Recording

vhs script that drives the tmux session deterministically. Output is a
canonical `demo.gif` for the README.

### Cost & time

- Wall: 5 min total
- USD: $0.50-$3 across all 5 frameworks
- Setup time: ~1 day (most of it is the tmux + dashboard polish)

### Success criteria

- All 5 frameworks complete in < 2 min each
- roko is at most 1.5x slower than the fastest baseline
- roko is the cheapest *or* the only one to complete the task correctly
- Audience can read the dashboard from across a room

### Risk

A single task is anecdote, not data. Pair with Recipe 3's static report so
the live demo links to "see the full N=100 run here."

---

## Recipe 2: Nightly roko-bench

**Goal.** Internal regression test. Every night, run our custom 20-task
bench against roko (pinned and HEAD) and 2 baselines (anthropic-direct,
LangGraph). Alert on regressions.

### Tasks

20-30 tasks from `demo/demo-research/roko-bench/tasks.toml`. Drawn from
real PRDs and `MORI-PARITY-CHECKLIST.md`. See `01-benchmarks.md`.

### Frameworks

3 frameworks, low-cost:

1. roko @ HEAD
2. roko @ last green commit (regression baseline)
3. anthropic-direct (external sanity baseline)

(LangGraph optional — adds ~$5/night.)

### What's reused

- **Harness, cost tracking, scoring**: same as recipe 1
- **Plan persistence**: `.roko/state/executor.json` — already crash-recoverable
- **Storage**: `.roko/learn/efficiency.jsonl` is the canonical dataset

### Setup (net-new only)

```
demo/demo-research/recipes/02-nightly/
├── README.md
├── tasks.toml                  # 25 hand-authored from PRDs
├── ci/
│   ├── github-actions.yml      # nightly cron, runs `roko bench run`
│   └── slack-alert.py          # if pass-rate drops vs baseline
└── reports/
    └── <date>/
        ├── runs.csv            # exported via `roko bench export`
        ├── efficiency.jsonl    # archived snapshot
        └── report.html         # generated from runs.csv
```

A single CLI command (`roko bench run --tasks tasks.toml --reps 3`) drives
the whole night. `roko bench export` flattens to CSV. A small Python
script renders to HTML.

### Display

- **Live (during the run)**: TUI bench tab connected over `/ws` to the
  remote nightly runner; `roko dashboard --remote ci-host:6677`
- **Static**: HTML report committed to `reports/<date>/report.html`
- **Alerts**: Slack ping if `pass_rate(HEAD) < pass_rate(baseline) - 0.05`
  or `usd_per_pass(HEAD) > usd_per_pass(baseline) * 1.2`
- **Trends**: F10 Learning tab already shows efficiency trends across
  nights (sparkline of cost-per-success over 30 days). No Grafana needed
  unless you want browser-based access for non-engineers.

### Cost & time

- Wall: 30-90 min
- USD: $5-15/night
- Setup time: ~3-5 days
- Ongoing: ~$200/month

### Success criteria

- Catches roko regressions within 24h
- False positive rate < 1/week
- The dashboard makes "is HEAD better than last week" answerable in 5s

### Risk

Flaky tasks or noisy LLMs make alerts noisy. Mitigation: every night runs
N=3 reps per task; alert on `mean - sigma`, not single runs.

---

## Recipe 3: Investor-grade Pareto

**Goal.** A defensible, public-facing comparison. The number that goes on
the slide. Not live — pre-recorded with a polished static report.

### Tasks

Mix for external validity + domain relevance:

- 50 SWE-bench Verified Mini (external validity)
- 25 roko-bench (domain relevance)
- 25 τ²-retail (reliability with `pass^k=4`)

= 100 tasks per framework × 5 frameworks × 4 reps for `pass^k` = 2000
runs. (For `pass^k`, only τ-bench tasks need 4 reps; SWE-bench is
deterministic enough that 1 rep is fine.)

### Frameworks

The full grid:

1. roko
2. Anthropic SDK direct
3. Claude Agent SDK
4. OpenAI Agents SDK
5. LangGraph
6. CrewAI

(Skip AutoGen if budget is tight — its 5x cost overhead is already
documented in published benchmarks.)

### What's reused

- **Harness, cost tracking, scoring, live display**: same as recipes 1-2
- **Multi-rep / pass^k**: `run_plan(reps=4)` already supported

### Setup (net-new only)

```
demo/demo-research/recipes/03-investor/
├── README.md
├── PROTOCOL.md                  # pre-registration (see 07-methodology.md)
├── tasks/
│   ├── swe-bench-mini.toml      # generated from HF dataset
│   ├── roko-bench.toml          # hand-authored from PRDs
│   └── tau-retail.toml          # generated from sierra-research/tau-bench
├── analysis/
│   ├── render_charts.py         # plotly: pareto, heatmap, cost, pass-k
│   └── render_report.py         # jinja2: HTML report
└── outputs/
    └── <run-id>/
        ├── efficiency.jsonl     # archived from .roko/learn/
        ├── episodes.jsonl       # archived from .roko/memory/
        ├── runs.csv             # via `roko bench export`
        ├── charts/*.html
        └── report.html
```

Backends + adapters + bench tab + graders live in the shared repo
locations (`crates/roko-agent/src/backends/`, `crates/roko-gate/src/`,
`crates/roko-cli/src/tui/views/`). Already built by recipe 2; reused
here.

### Display

- **Live (during the run)**: TUI projected, F11 Bench tab. Recorded
  with OBS for the screen-share half of the talk.
- **Static**: `report.html` is the artifact. Contents:

1. Headline table: pass@1, USD/task, USD/pass, wall time per backend.
2. Pareto chart: cost vs pass-rate.
3. Per-benchmark breakdown.
4. Per-task heatmap.
5. `pass^k` chart for τ-bench (the reliability story).
6. C-factor breakdown showing component scores per backend.
7. Methodology section (model pin, scoring criteria, gate config).
8. Caveats (sample size, contamination notes).

`render_charts.py` reads the exported CSV; `render_report.py` is a
small jinja2 template that embeds the plotly HTML.

### Cost & time

- Wall: 4-8 hours (parallelizable; budget 1 day end-to-end)
- USD: $50-150
- Setup time: ~2-3 weeks the first time, then re-runnable
- Re-run cost: same dollars, ~1 day wall

### Success criteria

- Headline table shows roko wins on USD/pass
- Pareto chart shows roko on the frontier (top-left of the cluster)
- Every claim is reproducible from `runs.csv` + the runner script
- A skeptical engineer reading the report can find the methodology
  details and not flag anything as misleading

### Risk

This is the demo someone will *try* to break. Methodology has to be
airtight — see `07-methodology.md` for the gauntlet to run before
publishing.

---

## Recipe 4: SWE-bench-mini public leaderboard run

**Goal.** Submit a roko score to the SWE-bench leaderboard. Get a
permanent, externally validated number.

### Tasks

SWE-bench Verified Mini (50) → SWE-bench Verified (500) once Mini works.

### Framework

Just roko. (You don't run baselines for this — the SWE-bench leaderboard
already has them.)

### What's reused

- **Harness, cost tracking, gates, sandboxing**: same as recipes 1-3
- **The roko backend itself** is what's being benchmarked (no comparators)

### Setup (net-new only)

```
demo/demo-research/recipes/04-swe-bench-public/
├── README.md
├── tasks/
│   └── swe-bench-verified.toml      # full 500-task generated from HF
├── graders/
│   └── swe_bench_grader.rs           # wraps swe-bench-runner; emits GateVerdict
├── exporters/
│   └── to_swebench_submission.rs     # episodes.jsonl → predictions.json
├── docker/                           # per-repo container images
├── prompts/                          # any prompt-engineering for SWE-bench
└── runs/
    └── <date>/
        ├── efficiency.jsonl
        ├── episodes.jsonl
        ├── predictions.json          # the SWE-bench submission format
        └── eval-results.json         # local re-run for sanity
```

Submission: SWE-bench accepts a JSON file with predicted patches per
instance. The exporter walks `.roko/memory/episodes.jsonl`, finds the
final patch per task, and emits predictions.json.

### Display

A leaderboard score is the display. Optional: tracking sheet of submissions
over time as roko improves.

### Cost & time

- Wall: 4-10 hours
- USD: $40-200 depending on retry behavior
- Setup time: ~1 week
- Submission cycle: weeks (community validation)

### Success criteria

- roko appears on the leaderboard
- Score > 50% on Verified Mini (table stakes for 2026)
- Aspirational: top-5 placement

### Risk

A bad showing is *public*. Don't submit until you've matched or beaten
"vanilla Anthropic SDK with the same model" in your own internal run.

---

## Sequencing recommendation

Order to attempt these in:

1. **Recipe 2** (nightly) — gives you the harness and the dataset.
   Lowest risk, immediately useful for internal use. ~1 week.
2. **Recipe 1** (live demo) — reuses the harness. Adds polish.
   Day or two on top.
3. **Recipe 3** (investor) — once you trust the harness, scale up.
   Expensive only in setup time.
4. **Recipe 4** (public) — when you've internally beat the
   anthropic-direct baseline.

Don't try to ship Recipe 3 first. Build the rails (Recipe 2) and you'll
discover the gotchas in `07-methodology.md` before they bite you publicly.

---

## What's shared across recipes

The shared infrastructure mostly lives **inside `crates/`** because the
backends, gates, and TUI tab are first-class roko features. The
`demo/demo-research/` tree only holds benchmark-specific data and
recipe glue.

```
crates/                                 # all reused (already exists)
├── roko-agent/src/backends/
│   ├── claude_cli.rs        existing
│   ├── claude_api.rs        existing
│   ├── ...
│   ├── langgraph.rs         NEW (~150 LOC)
│   ├── crewai.rs            NEW
│   ├── autogen.rs           NEW
│   ├── oai_agents.rs        NEW
│   └── claude_agent_sdk.rs  NEW
├── roko-cli/src/tui/views/
│   └── bench_view.rs        NEW (~300 LOC)
├── roko-gate/src/gates/
│   ├── bench_acceptance.rs  NEW (~100 LOC)
│   ├── humaneval.rs         NEW
│   ├── swe_bench.rs         NEW
│   └── tau_bench.rs         NEW
└── roko-cli/src/bin/
    └── bench_export.rs      NEW (~200 LOC)

demo/demo-research/                    # benchmark data + glue
├── 00-08 *.md                          # this research
├── adapters/                           # Python framework wrappers
│   ├── langgraph.py
│   ├── crewai.py
│   ├── autogen.py
│   ├── oai_agents.py
│   └── claude_agent_sdk.py
├── tasks/                              # benchmark task sets
│   ├── humaneval.toml
│   ├── bfcl.toml
│   ├── swe-bench-mini.toml
│   ├── tau-retail.toml
│   └── roko-bench.toml
├── exporters/                          # optional external integrations
│   ├── to_inspect.rs
│   ├── to_langfuse.py
│   └── to_swebench_submission.rs
└── recipes/                            # one dir per recipe
    ├── 01-five-frame/
    ├── 02-nightly/
    ├── 03-investor/
    └── 04-swe-bench-public/
```

The `crates/` additions are production roko features (a benchmarking
toolkit roko ships with). The `demo/demo-research/` content is
benchmark-specific data and demo glue.
