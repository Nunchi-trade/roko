# Benchmarks

Catalog of task sets we can use to score roko vs. competing frameworks. Pick
based on what you want the demo to *say*, not on what scores best.

## Quick selector

| If the demo claim is... | Use |
|---|---|
| "We solve real GitHub issues" | SWE-bench Verified Mini (50) → SWE-bench Lite (300) → SWE-bench Verified (500) |
| "We use tools correctly" | Berkeley Function Calling Leaderboard v4, τ²-bench |
| "We are reliable, not lucky" | τ²-bench with `pass^k=4`, custom bench with N=5 reps |
| "We do real terminal work" | Terminal-Bench 2.0 (89 hard tasks) |
| "We solve general assistant tasks" | GAIA (validation split) |
| "We are cheap on simple coding" | HumanEval / MBPP (saturated but fast) |
| "We solve **roko's actual workload**" | Custom roko-bench from PRDs (recommended) |

## SWE-bench family

The flagship coding-agent benchmark. Real GitHub issues from 12 popular
Python repos; agent must produce a patch that passes the project's own tests.

| Variant | Tasks | When to use |
|---|---|---|
| SWE-bench Verified Mini (Princeton HAL) | 50 | Fast iteration; ~$3-15 per full run |
| SWE-bench Lite | 300 | Standard cheap benchmark |
| SWE-bench Verified | 500 | Human-validated, the headline number |
| SWE-bench (full) | 2,294 | Don't bother — Lite covers it |
| SWE-bench Pro | ~1,500 | Newer, harder, longer-horizon |

**Setup**

```bash
pip install swebench
# or via Inspect AI's pre-built eval (see 04-eval-harnesses.md)
inspect eval inspect_evals/swe_bench --model anthropic/claude-sonnet-4-6 --limit 50
```

Tasks come from HuggingFace: [`princeton-nlp/SWE-bench_Lite`](https://huggingface.co/datasets/princeton-nlp/SWE-bench_Lite).
Each task instance includes the repo URL, base commit, problem statement, and
the gold patch + test patch used for grading.

**Grading.** Apply the agent's diff, run the project's own pytest suite. A
task passes iff the originally-failing tests now pass and the
originally-passing tests still pass. This is the only honest metric;
LLM-as-judge is unsuitable for code correctness.

**Sandboxing.** Each task needs a clean container with the repo's
dependencies installed. Inspect AI ships Docker integration; SWE-bench's own
runner uses Modal/Docker. Plan for ~2-15 GB of disk per repo cached.

**Cost expectations** (Sonnet 4.6, ~April 2026 prices):

| Variant | Approx cost per run | Wall time |
|---|---|---|
| Verified Mini (50) | $3-$15 | 30-90 min |
| Lite (300) | $20-$80 | 2-6 hours |
| Verified (500) | $40-$200 | 4-10 hours |

Costs scale with how aggressively the agent retries. Bare SDK calls land at
the low end; CrewAI/AutoGen at the high end (5-6x more tokens).

**Why it fits roko.** Closest possible match to the self-hosting story.
Patch-or-don't is binary, no judge needed. Gates have something to do.

**Caveats.** Public repos = risk of training-data contamination, especially
on older issues. Use SWE-bench Verified specifically (human-screened).
Public leaderboard scores are inflated; only trust a number you ran yourself.

## τ-bench / τ²-bench (Sierra)

Tool-Agent-User benchmark. Agent has tool access (e.g. retail or airline
APIs) and must serve a simulated user, following written policies.

- Tasks: ~120 per domain (retail, airline)
- Scoring: did the final database state match the gold state?
- Distinguishing metric: **pass^k** — probability of solving across `k`
  repeated runs. With k=4 most agents drop 20-40% from pass@1.

```bash
git clone https://github.com/sierra-research/tau-bench
cd tau-bench && pip install -e .
python run.py --agent-strategy tool-calling --env retail --model claude-sonnet-4-6
```

**Why it fits roko.** Tests reliability, not lucky one-shots. roko's gates +
plan revision are exactly the mechanism that should improve pass^k. If we
can show roko's pass^4 ≥ baseline pass@1, the demo writes itself.

**Caveats.** Setup is heavier than SWE-bench (you need to mock/run domain
APIs). Tasks are conversational and Anthropic-friendly — Sonnet does well by
default.

## Berkeley Function Calling Leaderboard (BFCL v4)

2,000+ tool-call cases. Tests serial, parallel, multi-step, and agentic
function calling. AST-based grading (no LLM judge).

```bash
pip install bfcl-eval
bfcl evaluate --model claude-sonnet-4-6 --test-category all
```

Categories include simple, parallel, parallel-multiple, executable, REST,
relevance, multi-turn. v4 added stateful agentic eval.

**Why it fits.** Cheapest broad agentic signal. Runs in 30 minutes on a
laptop. Good filter — if a framework can't pass BFCL, no point running
SWE-bench.

**Caveats.** Synthetic-feeling. Doesn't test long-horizon planning.

## GAIA

301 real-world assistant questions, each requires multi-step browsing,
file analysis, and reasoning. Human-validated.

```bash
# via Inspect AI
inspect eval inspect_evals/gaia --model anthropic/claude-sonnet-4-6
```

**Why it fits.** Prestigious leaderboard, Anthropic models lead it. Lets
roko piggy-back on Anthropic's strength.

**Caveats.** Web-browsing-heavy, not coding-shaped. roko's strengths
(orchestration, gates) don't help much here.

## Terminal-Bench 2.0

89 hard tasks in real Linux terminals: install, build, debug, configure
servers, write scripts. Each task has a unique sandbox + comprehensive tests.

```bash
git clone https://github.com/laude-institute/terminal-bench
cd terminal-bench && pip install -e .
tb run --agent terminus --model claude-sonnet-4-6
```

**Why it fits.** Closest to "what an SRE / dev ops engineer would do". roko
runs real shell commands, has gates, and can spin up sandboxes — fits well.

**Caveats.** Each task needs Docker. Disk-heavy. Slow.

## HumanEval / MBPP

Classic. Function-completion from a docstring. 164 / ~1,000 tasks.

```bash
# DeepEval
pip install deepeval
deepeval benchmark humaneval --model claude-sonnet-4-6
```

**Why it fits.** Cheap baseline. Frontier models all hit 90%+ so it's not a
ranking signal anymore — but it's a fast sanity check that your harness
works at all. Useful as a smoke test before committing to a full SWE-bench
run.

**Caveats.** Saturated. Don't lead with this number.

## BigCodeBench

1,140 tasks testing realistic library integration (numpy, pandas, etc).
Less saturated than HumanEval.

```bash
pip install bigcodebench
bigcodebench evaluate --model claude-sonnet-4-6 --subset hard
```

Useful middle ground if SWE-bench is too heavy and HumanEval is too easy.

## AgentBench (THUDM, ICLR'24)

8 environments: OS interaction, DB, knowledge graph, card games, lateral
thinking, web shopping, web browsing, household.

```bash
git clone https://github.com/THUDM/AgentBench
cd AgentBench && pip install -e .
```

**Why it fits.** Breadth. Single number that touches many capabilities.

**Caveats.** Older (ICLR'24), some environments are showing their age.
Setup is heavy (multiple Docker containers).

## WebArena / VisualWebArena

Realistic web automation. Self-hosted versions of Reddit, GitLab, etc.

**Why it fits.** Less than well — roko isn't a browsing agent. Skip unless
demo specifically calls for it.

## Custom roko-bench (recommended for *your* demo)

The single most predictive benchmark is one drawn from the work roko actually
does. Concretely:

1. Pick 20-30 issues / PRDs from `bardo-backup/prd/`,
   `tmp/implementation-plans/`, `tmp/ux-followup/`, or `MORI-PARITY-CHECKLIST.md`.
2. For each, capture:
   - **input**: the PRD slug + a frozen snapshot of the relevant code at a
     known git SHA.
   - **success criterion**: a deterministic check (e.g. "after the agent
     runs, `cargo test -p roko-foo` passes" or "function `bar` exists
     in `crates/baz/src/qux.rs`").
3. Bucket by difficulty: trivial (rename, single-file edit), medium
   (cross-crate change), hard (new feature, multi-step plan).
4. Pin task definitions in `demo/demo-research/roko-bench/tasks.toml`
   with frozen SHAs.

**Format suggestion** (illustrative, not implementation):

```toml
[[task]]
id = "roko-bench-001"
slug = "wire-cold-archival-cron"
difficulty = "medium"
prd = "bardo-backup/prd/cold-archival.md"
base_sha = "0a00d130"
success_check = "cargo test -p roko-fs cold_archival_runs_on_schedule"
budget_usd = 0.50
budget_seconds = 600

[[task]]
id = "roko-bench-002"
slug = "force-backend-override-learning"
difficulty = "hard"
# ...
```

**Why this is the strongest claim.** Public benchmarks are easily gamed and
saturated. A custom bench from your own backlog is:

- Uncontaminated (these issues didn't exist in pretraining)
- Domain-relevant (success here means roko helps roko)
- Honest cost numbers (real workload, real prompt sizes)
- Reusable for nightly regression testing

**Caveats.** No leaderboard credibility. Use *alongside* SWE-bench Verified
Mini for external validity, not instead of.

## Reference: 50+ benchmark compendium

[`philschmid/ai-agent-benchmark-compendium`](https://github.com/philschmid/ai-agent-benchmark-compendium)
catalogs everything. Worth a skim before committing.

Categories: function calling/tool use, general assistant/reasoning,
coding/software engineering, computer interaction.

## Recommended starter set for roko

For a first pass demo:

1. **HumanEval** as a smoke test (10 min, near-free, confirms harness works)
2. **BFCL v4** for cheap broad signal on tool use (30-60 min)
3. **SWE-bench Verified Mini** (50 tasks) for the headline coding number
4. **Custom roko-bench** (20 tasks) for the differentiated story
5. **τ²-bench retail with `pass^k=4`** to show reliability

Total: ~$50-150 per full sweep across 5 frameworks. ~6-12 hours wall time.
Easily fits in a nightly CI job.
