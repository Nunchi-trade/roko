# Methodology — making comparisons defensible

The single biggest risk for this demo is a methodologically sloppy run that
gets fact-checked publicly. This file is the gauntlet to run any
comparison through *before* publishing.

Berkeley's "How We Broke Top AI Agent Benchmarks" piece is the canonical
warning: published leaderboard scores are routinely 10-30% inflated by
benign mistakes. The same mistakes are easy to make in our own demo.

## Five hard rules

1. **Pin the model.** Every framework hits the same `model:version` string.
   No "the LangGraph version uses gpt-4o because it's the default."
2. **Pin the toolkit.** Same tool surface (bash, read, write, patch) across
   frameworks. No "AutoGen has its own web search, we left it on."
3. **Pin the budget.** Same wall-clock cap, same max-tool-turns,
   same max-tokens-per-call. A framework that retries 50 times "wins" by
   spending more money, not by being smarter.
4. **Run multiple reps.** N≥3 per task. Report mean + sigma. A single run
   is anecdote.
5. **Pre-register the scorer.** Define the success check *before* the run.
   No "well, that one was kind of correct" after the fact.

## What "fair" means

A fair comparison answers: "with the same model and the same tools,
which orchestration produces the most successful outputs at the lowest
cost?"

Things that make a comparison **unfair**:

| Mistake | Effect |
|---|---|
| Different models per framework | You're benchmarking models. |
| Different tool sets | You're benchmarking tools. |
| Different system prompts | You're benchmarking prompt engineering. |
| Different retry budgets | The looser one wins by spending more. |
| Per-framework prompt optimization | Selection bias toward whoever you know best. |
| Counting cached tokens differently | Hides 5-10x cost differences. |
| Cherry-picked task subset | The textbook leaderboard fraud. |

## Prompt parity

Every framework needs the *same task description* converted to its own
input format. Concretely, define a canonical task:

```yaml
task_id: roko-bench-001
prompt: |
  Add a `--dry-run` flag to `roko plan run`.
  Acceptance: tests in `crates/roko-cli/tests/dry_run_smoke.rs` pass.
context_files:
  - crates/roko-cli/src/orchestrate.rs
  - crates/roko-cli/src/main.rs
budget:
  max_seconds: 600
  max_tool_turns: 30
  max_tokens_per_call: 4096
```

Each solver translates this into its native form:

- **Anthropic direct**: a single `messages.create` with the prompt + tool
  definitions for bash/read/write
- **LangGraph**: same prompt as the entry node's input
- **CrewAI**: the prompt becomes the Task description
- **roko**: the prompt is the entire `roko run` argument

Forbidden: hand-tuning a system prompt for one framework but not others.
If you tune one, tune all. (See "the per-framework optimization budget"
below.)

## The per-framework optimization budget

Reality is that frameworks have different defaults that work better or
worse out of the box. To be fair without being dishonest, allocate equal
optimization effort:

> Each framework gets up to 4 hours of a senior engineer's time to tune
> prompts, tool configs, and retry strategies. Document what was changed.

This is what's meant by "skill-ceiling parity." It surfaces in the
methodology section of the final report:

> "We allocated 4 hours per framework for prompt and tool tuning. The
> diffs from default config are linked in [appendix]."

If that's not feasible, just run defaults for everyone and label the
report as "default-config comparison." Either is honest. Mixing — tuning
roko hard and running the others as defaults — is not.

## Statistical rigor

For a published number, you want at least:

- **N ≥ 3 reps per task per framework** (5 if the task is short).
- **Bootstrap CI on pass-rate.** `scipy.stats.bootstrap` works. Report
  pass-rate as `0.85 ± 0.04 (95% CI)` not `0.85`.
- **Paired comparisons.** Each framework runs the *same* task list. Use
  paired tests (Wilcoxon, McNemar's) not unpaired t-tests.
- **Sample size justification.** With N=20 tasks and 2 frameworks at
  ~80% pass-rate, you can detect a 15-point difference with 80% power.
  Smaller differences need more tasks.

For an internal regression test it's lower-stakes — N=3 with no CI is
fine, you're tracking trends not making absolute claims.

## Contamination

Public benchmarks risk training-data contamination. If the model has seen
the answer, the test is invalid.

- **SWE-bench Verified** is human-screened. Lower contamination risk.
- **HumanEval** is heavily contaminated at this point. Use as smoke test
  only.
- **τ-bench** is mostly synthetic. Lower risk.
- **Custom roko-bench from your own backlog** is uncontaminated by
  construction.

Mitigations for the contaminated benchmarks:
- Report numbers on the harder/newer subsets (Verified, Pro, Mini).
- Avoid making contamination-sensitive claims (like "model X memorized
  fewer answers than model Y").
- Note known contamination in the report.

## Cost accounting honesty

Common ways to under- or over-state cost:

- **Forgetting cache reads.** Anthropic prompt caching cuts cost 5-10x.
  Reporting "no-cache" cost makes roko look 5x more expensive than reality.
  Always report both modes or pick one and label clearly.
- **Counting only LLM cost.** roko also runs `cargo build`, gates, etc.
  These cost CPU/wall time but not API dollars. Be explicit:
  "API cost only" or "API + compute cost."
- **Excluding failed runs.** If a framework crashes mid-task and you don't
  count its tokens, it looks artificially cheap. Count *all* tokens.
- **Excluding retries.** Same.
- **Counting input vs output cost the same.** Output is 5x more expensive.
  Some frameworks generate way more output (chain-of-thought everywhere).
  Show input/output split.

## Time accounting honesty

- **Wall time vs CPU time.** roko parallelizes; LangGraph doesn't. Wall
  time is what users care about; CPU time is what infrastructure costs.
  Show both.
- **Cold start.** First task includes Docker pull, cargo build, etc.
  Either warm up before measurement or clearly mark cold/warm.
- **Tool latency vs LLM latency.** A 30-second `cargo test` isn't the
  framework's fault. Break out tool time vs LLM time.

## Failure modes to label clearly

Categories of failure to bucket separately:

| Bucket | What | Fair to count? |
|---|---|---|
| Solved correctly | Pass | Yes |
| Solved incorrectly | Confident wrong answer | Yes — counts as fail |
| Refused / abstained | "I cannot do this" | Bucket separately |
| Hit budget | Ran out of turns/tokens | Yes — counts as fail |
| Crashed | Framework error | Bucket separately |
| Timed out | Wall clock exceeded | Yes — counts as fail |

A 95% pass-rate where 30% of "passes" are actually framework crashes
filtered out is fraud. Show the full breakdown.

## Pre-registration

Before any "published" run:

1. Pick the model. Write it down.
2. Pick the task list. Write it down. Hash the file.
3. Pick the success criteria. Write them down.
4. Pick the budget caps. Write them down.
5. Run the comparison.
6. Report the numbers.

If you change anything between 1-4 and 5, restart from 1.

This sounds bureaucratic. It is. It also means nobody can credibly accuse
you of moving goalposts. The pre-registration document goes in
`demo/demo-research/runs/<date>/PROTOCOL.md` and is read-only after the
run starts.

## Things to disclose

In the final report, disclose:

- Models used (incl. version pins)
- Frameworks used (incl. version pins)
- Toolkit used (which tools, which implementations)
- Budgets (turns, tokens, wall time)
- Cache policy (on/off, anthropic prompt cache config)
- Number of reps per task
- How failures were bucketed
- How costs were computed (price table source, when fetched)
- Total compute / dollars spent on the eval itself
- Hardware (laptop, EC2 instance type, etc.)
- Date the eval was run (model behavior drifts)

A reader who can reproduce your results from this list is the bar.

## Audit checklist before publishing

```
[ ] Same model pinned across all frameworks
[ ] Same toolkit across all frameworks
[ ] Same budget caps across all frameworks
[ ] Same task list across all frameworks
[ ] N ≥ 3 reps per task
[ ] Cache mode declared
[ ] Failures bucketed (incl. crashes, timeouts, refusals)
[ ] Bootstrapped CI on pass-rate
[ ] All cost (incl. failed runs, retries) included
[ ] Methodology section in report
[ ] Reproducibility appendix (commit SHAs, prices, prompts)
[ ] No cherry-picked task subsets
[ ] No re-running until the result was favorable
[ ] Plot axes labeled, units shown
[ ] Pareto chart includes baseline frameworks even if they look bad
```

## What roko gives you for free on the methodology side

Several of the rules above are tedious to enforce by hand but enforced
by construction when using roko's existing infrastructure (`08-reuse-map.md`):

| Rule | How roko enforces it automatically |
|---|---|
| Pin the model | `tasks.toml` declares the model; `force_backend` overrides only routing, not model |
| Pin the toolkit | All backends dispatch through the same `ToolDispatcher` with the same registered tools |
| Pin the budget | `budget_usd` and `budget_seconds` are first-class fields on each task |
| Same task list | Every backend runs against the same `tasks.toml`; the only thing that varies is `force_backend` |
| Multiple reps | `run_plan(reps=N)` with deterministic plan IDs makes pass^k computation trivial |
| Bucket failures | `Episode.outcome` distinguishes `pass`, `fail`, `timeout`, `error`, `refused`, `crashed` |
| All cost included | `AgentEfficiencyEvent` is emitted on every turn, including failed and aborted runs |
| Reproducibility appendix | `.roko/state/executor.json` snapshots the entire run state; commit it |
| Cache mode declared | `cost_usd` and `cost_usd_without_cache` are both stored; report whichever is honest for the demo |

Things you still have to do by hand:

- Pre-register a `PROTOCOL.md` before each public run
- Decide allocation of optimization effort per backend
- Write the methodology section of the final report
- Choose which benchmarks to include / exclude (and disclose this)
- Manually verify the deterministic graders (gate definitions) are
  truly deterministic

## Things that are *not* unfair, just real

Some asymmetries are inherent to roko's pitch and are fine to highlight:

- **Gates.** Other frameworks don't have them. roko's pass^k advantage
  is *because* of gates. That's the point. Show it.
- **Plan persistence.** roko can resume after crash. This isn't a "bug"
  in other frameworks; it's a feature roko has.
- **Multi-task DAGs.** roko runs many tasks in parallel; others typically
  don't. If your benchmark has multi-task plans, this is a legitimate
  win.
- **Rust execution.** Marginal latency win, but real. Document it.

The line: it's fine to win because of architecture; it's not fine to win
because of unequal scoring rules.

## When to *not* publish a number

- Sample size < 20 tasks
- Single rep per task
- Variance > 15% across reps
- Framework crashed > 10% of the time and you suppressed those
- The result depends on which day you ran it (model drift)
- You couldn't get a baseline framework to run at all (likely your bug)
- The result feels "too good"

If any of these are true, the number is for internal use only. Use the
nightly-bench harness (Recipe 2) to build confidence first.

## References

- Berkeley: "How We Broke Top AI Agent Benchmarks" — failure mode catalog
- HAL leaderboards (Princeton) — mini benchmarks designed to be cheap and
  not gameable
- Anthropic's τ-bench reporting — model report card style
- Galileo's CLEAR framework — Cost/Latency/Efficacy/Assurance/Reliability
  as the canonical 5-axis enterprise eval
