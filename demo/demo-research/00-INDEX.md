# demo-research

Plans, options, and reference material for **head-to-head demos** that compare
roko against other agent frameworks (vanilla Anthropic SDK, OpenAI SDK,
LangChain/LangGraph, CrewAI, AutoGen, OpenAI Agents SDK) on:

1. **Correctness** — does the agent solve the task?
2. **Cost** — tokens in/out/cached, USD per task, USD per success.
3. **Latency** — wall time per task, time-to-first-token, throughput.
4. **Reliability** — pass^k across repeated runs, gate-failure rate.
5. **Trace quality** — what we can observe vs. what's a black box.

Nothing in this directory is implementation code. It is a research catalog
plus design notes for what the demo *could* look like. When we agree on a
recipe from `06-recipes.md`, implementation lives next to the recipe under
its own subdirectory (e.g. `roko-bench-mini/`).

## Why this matters for roko

The roko pitch is: "agent toolkit with gates, planning, persistence, and
learning, that produces equivalent or better outputs at lower per-task cost
because failures are caught early." That claim is unfalsifiable in a slide
deck. A demo grid with apples-to-apples task pass-rates and dollar costs
makes it a number.

The demo also doubles as **internal regression coverage** — every time we
change the orchestrator, prompt builder, gate pipeline, or cascade router,
re-running the grid tells us whether we got better, worse, or just different.

## Reuse-first principle

**Read `08-reuse-map.md` first.** roko already has ~95% of the benchmark
stack: efficiency event tracking, episode logging, plan executor, gate
pipeline as scorer, ratatui dashboard with file watcher, WebSocket event
stream. Every doc here defaults to extending those rather than introducing
new external services (LiteLLM, Inspect AI, Langfuse, Streamlit, Grafana).

External tools are noted where they remain useful — typically as exporters
for community-standard formats, or inside Python adapters that wrap
competing frameworks. They are never the primary infrastructure.

The only net-new code we'd write:

- **4 competitor backends** (LangGraph / CrewAI / AutoGen / OpenAI Agents),
  ~150 lines each = small Rust + small Python adapter
- **1 TUI tab** (F11 Bench) — ~300 lines of ratatui using widgets already
  in the codebase
- **Benchmark-specific gates** (HumanEval / SWE-bench / τ-bench / roko-bench
  graders) — wired into the existing `roko-gate` registry
- **Task definitions** (`tasks.toml` files in the existing schema)

Total: ~1,800 lines, ~1 week of effort. Compare with "build a Python
benchmark harness on Inspect AI + LiteLLM + Langfuse + Streamlit" which
is 3-4 weeks and adds permanent infrastructure roko doesn't otherwise
need.

## Decision flow

```
                          What is the demo for?
                                  │
           ┌──────────────────────┼──────────────────────┐
           ▼                      ▼                      ▼
   "Public-facing       "Internal regression"    "Investor / partner"
   leaderboard hit"     ──────────────────────   ─────────────────
        │                      │                      │
        ▼                      ▼                      ▼
  Use SWE-bench         Build "roko-bench"      Live TUI side-by-side
  Verified Mini /        from your own           (see 05) with cost
  Lite (50 / 300         PRDs (10-30 tasks),      meter and Pareto
  tasks). Show           run nightly. Smaller    chart at the end.
  pass@1 vs.             tasks, faster, more
  baseline frameworks.   honest.
        │                      │                      │
        └──────────────────────┴──────────────────────┘
                                  │
                                  ▼
                Pin a single model (e.g. Sonnet 4.6).
                Run all comparisons via roko's plan executor:
                each competing framework is wrapped as a
                roko backend (~150 LOC each). Cost, tokens,
                latency, gate verdicts persist automatically
                to .roko/learn/efficiency.jsonl and
                .roko/memory/episodes.jsonl. Render via a new
                F11 Bench tab in the existing ratatui TUI.
```

## File map

| File | Purpose |
|---|---|
| `00-INDEX.md` | This file. Decision flow, file map, glossary. |
| `01-benchmarks.md` | Catalog of standardized benchmarks (SWE-bench, τ-bench, BFCL, GAIA, Terminal-Bench, HumanEval) plus custom-bench guidance. |
| `02-frameworks.md` | Frameworks to compare against — wrapped as roko backends. |
| `03-cost-tokens.md` | Reuse `AgentEfficiencyEvent` + `costs_db.rs`; LiteLLM only inside adapter shims. |
| `04-eval-harnesses.md` | Reuse `roko-orchestrator` as harness; gate pipeline as scorer. Inspect AI / Langfuse as optional exporters. |
| `05-realtime-visualization.md` | Reuse ratatui TUI — extend with F11 Bench tab. Web/Streamlit demoted to optional. |
| `06-recipes.md` | End-to-end concrete demos: "5-frame side-by-side", "nightly roko-bench", "investor demo", "pareto plot run". |
| `07-methodology.md` | How to make comparisons defensible: pinning models, controlling variance, fair task selection, contamination. |
| `08-reuse-map.md` | **Read first.** Canonical map of what roko already provides. Drives every other doc. |

## Glossary

- **Solver** — In Inspect AI, the function that runs an agent against a task.
  We will write one Solver per framework we compare.
- **Task** — A scored prompt + grader. `01-benchmarks.md` lists candidate
  task sets.
- **Gate** — A roko-only concept: a check (compile, test, clippy, diff)
  that runs after an agent edit. Other frameworks don't have these, which
  is part of the point of the comparison.
- **Pass@1** — Did the agent solve it in one attempt?
- **Pass^k** — Did the agent solve it in *all* of `k` repeated attempts?
  Borrowed from τ-bench. Distinguishes lucky from reliable.
- **USD per success** — Total spend divided by successful tasks. Punishes
  expensive failures, which is what we want to highlight.
- **Trace** — The full LLM-call log for a task: messages, tool calls, token
  counts, latency. Langfuse / Phoenix / Inspect logs are all traces.

## Status

Nothing implemented yet. This directory is plan + research. When we move
from plan to action, mirror the structure of `tmp/demo-resources/`:

- one subdirectory per recipe
- a `bin/` with shared shell helpers if applicable
- top-level shell entrypoints (`run-bench.sh` etc.)
- short README per subdirectory
