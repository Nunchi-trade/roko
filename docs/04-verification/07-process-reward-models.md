# 07 — Process Reward Models

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-learn` (planned integration), `roko-gate` (data source)
> **Status**: Design, informed by active gate infrastructure


> **Implementation**: Shipping

---

## 1. Overview

Process reward models (PRMs) score intermediate reasoning steps rather than only the
final output. In the context of agent-driven development, this means evaluating the
agent's *process* — each tool call, each file edit, each reasoning turn — not just
whether the final code passes gates.

Standard verification is binary: the code either compiles or it doesn't, tests either
pass or they don't. Process rewards add granularity: *how much progress did the agent
make toward a working solution?* This matters because:

- An agent that gets 90% of the way to a working solution before failing is more
  promising than one that makes no progress
- Intermediate progress signals enable early intervention (abandon a failing approach
  before it consumes the full retry budget)
- Process rewards provide 10x richer training signal than final pass/fail outcomes

> **Citation**: Lightman et al. "Let's Verify Step by Step" (2023) — PRM800K dataset,
> process reward models for mathematical reasoning.

> **Citation**: refactoring-prd/02-five-layers.md — "Process Reward Models: Promise +
> Progress scoring, low Promise → early intervention, negative Progress → re-planning."

---

## 2. The Two Dimensions: Promise and Progress

Roko's process reward model tracks two orthogonal signals per agent execution:

### 2.1 Promise

**Promise** estimates how likely the current execution is to eventually succeed, given
the work done so far. It answers: "Is this approach heading somewhere good?"

Indicators of high Promise:
- The agent is making compile-passing edits (Rung 0 passes)
- The agent's tool calls are targeting the right files
- The edit pattern matches successful historical executions for similar tasks

Indicators of low Promise:
- The agent is making the same edit repeatedly (loop detection)
- The agent's tool calls are targeting unrelated files
- The compile error count is increasing, not decreasing

**Intervention**: Low Promise → early termination of the current attempt, before the
full retry budget is consumed. Better to start fresh with a different prompt or model
than to continue down a failing path.

### 2.2 Progress

**Progress** measures whether the agent is advancing toward the goal or stalling. It
answers: "Is this attempt doing better than the previous one?"

Indicators of positive Progress:
- Higher rung reached than the previous attempt (ratchet advancing)
- Fewer compile errors than the previous attempt
- More tests passing than the previous attempt

Indicators of negative Progress:
- Same or lower rung than the previous attempt
- Same or more compile errors
- Same or fewer tests passing

**Intervention**: Negative Progress across multiple attempts → re-planning. The current
plan may be fundamentally flawed. Rather than retrying with the same approach, generate
a new plan and start over.

---

## 3. Data Sources for Process Rewards

The gate infrastructure provides rich data for computing process rewards:

### 3.1 Per-Turn Tool Call Metadata

Each agent turn produces a `ToolCallMeta`:
```rust
pub struct ToolCallMeta {
    pub tool_name: String,
    pub duration_ms: u64,
    pub result_tokens: u64,
    pub succeeded: bool,
    pub advanced_task: bool,      // Did this call advance the task?
    pub was_redundant: bool,      // Was this call unnecessary?
    pub error_category: Option<String>,
}
```

The `advanced_task` flag can be computed post-hoc: if a tool call's output was referenced
in the final solution (the agent used the information), it advanced the task. If not,
it was wasted work.

> **Citation**: tmp/implementation-plans/modelrouting/12-advanced-patterns.md §K
> (Task 2J.16) — "Per-step rewards provide 10x richer signal than final pass/fail."

### 3.2 Gate Verdicts with Rung Information

Each gate pipeline execution produces verdicts with:
- Which rung was reached (highest passing rung)
- Test counts (passed/failed/ignored) — a proxy for "how close to passing"
- Error digest — what went wrong, machine-parseable

### 3.3 Diff Analysis

The `DiffGate`'s analysis provides:
- How many substantive lines were added
- Whether the additions are vacuous (todo!/unimplemented!)
- Whether the code is getting more or less complete

### 3.4 Ratchet State

The `GateRatchet` tracks the highest rung passed per plan. Changes in the ratchet state
across attempts reveal whether the agent is making progress:
- Ratchet advances from rung 1 to rung 2: positive Progress
- Ratchet stays at rung 1 after 3 attempts: stalling
- Ratchet would regress from rung 2 to rung 1: negative Progress (blocked by ratchet)

---

## 4. The Promise Score Function

Promise is computed as a weighted combination of signals:

```
Promise(attempt) = w₁ × rung_fraction
                 + w₂ × test_pass_rate
                 + w₃ × error_trend
                 + w₄ × tool_efficiency
```

Where:
- `rung_fraction` = highest_rung_passed / total_rungs (0.0 to 1.0)
- `test_pass_rate` = tests_passed / total_tests (0.0 to 1.0, or 0.5 if no test gate)
- `error_trend` = 1.0 if errors decreasing, 0.5 if stable, 0.0 if increasing
- `tool_efficiency` = useful_tool_calls / total_tool_calls

Default weights: w₁=0.4, w₂=0.3, w₃=0.2, w₄=0.1.

### Promise Thresholds

| Promise | Interpretation | Action |
|---|---|---|
| > 0.8 | High — likely to succeed | Continue, possibly reduce retries |
| 0.4–0.8 | Moderate — uncertain | Continue with standard retries |
| 0.2–0.4 | Low — probably failing | Consider early termination |
| < 0.2 | Very low — almost certainly failing | Terminate, try different approach |

---

## 5. The Progress Score Function

Progress compares the current attempt to the previous one:

```
Progress(attempt_n) = Δrung + Δtest_rate + Δerror_count
```

Where:
- `Δrung` = (current_rung - previous_rung) / total_rungs
- `Δtest_rate` = current_pass_rate - previous_pass_rate
- `Δerror_count` = (previous_errors - current_errors) / max(previous_errors, 1)
  (positive = improvement)

### Progress Thresholds

| Progress | Interpretation | Action |
|---|---|---|
| > 0.1 | Advancing | Continue, approach is working |
| -0.1 to 0.1 | Stalling | Escalate complexity, adjust prompt |
| < -0.1 | Regressing | Stop retrying, re-plan |

---

## 6. Integration with the Feedback Loop

Process rewards create a fast feedback loop within the agent execution cycle:

```
Agent turn N
    ↓
Gate pipeline
    ↓
Compute Promise(N) and Progress(N)
    ↓
Decision:
  Promise > 0.4 and Progress > -0.1 → continue to turn N+1
  Promise < 0.2 → early termination
  Progress < -0.1 for 3 turns → re-plan
```

This is a *within-attempt* optimization. It happens faster than the retry loop
(which operates across attempts) and much faster than the escalation mechanism
(which operates across failed attempts).

The three feedback timescales:
1. **Process reward** (per-turn): Promise/Progress → continue/terminate
2. **Retry loop** (per-attempt): Gate verdict → retry with adjusted prompt
3. **Escalation** (across attempts): Repeated failure → add rungs, re-plan

> **Citation**: bardo-backup/prd/16-testing/07-fast-feedback-loops.md — Machine-speed
> evaluation loops: confidence calibration, context attribution, cost-effectiveness.

---

## 7. Academic Foundations

### 7.1 Lightman et al. (2023) — PRM800K

Showed that process supervision (scoring each step) outperforms outcome supervision
(scoring only the final answer) for mathematical reasoning. Best-of-N selection with
process rewards outperformed majority voting by 8%.

### 7.2 AgentPRM (arXiv:2502.10325)

Extended process rewards to agent tool-use settings. Per-step rewards provide 10x richer
signal than final pass/fail. The key insight: not all tool calls contribute equally to
the outcome. Scoring each call identifies which calls are productive vs. wasteful.

### 7.3 Self-Refine (Madaan et al. 2023)

Showed that LLMs can improve their own outputs through iterative refinement with
feedback. Process rewards formalize the feedback signal that drives refinement: rather
than generic "try again," the agent gets "your Promise score dropped because error count
increased — focus on reducing errors."

### 7.4 Reflexion (Shinn et al. 2023)

Introduced the concept of verbal reinforcement for agents: converting numeric feedback
into natural language that the agent can learn from. Process rewards can be converted
to reflexion-style feedback: "Your last 3 attempts reached Rung 1 but failed at Rung 2.
Test failures are all in the auth module. Focus on auth module tests."

> **Citation**: bardo-backup/tmp/mori-refactor/06-harness.md — Academic foundations
> section: "Process Reward Models (Lightman et al. 2023 PRM800K), Self-Refine (Madaan
> et al. 2023), Reflexion (Shinn et al. 2023)."

---

## 8. Promise + Progress as Cybernetic Signals

In the Synapse Architecture's cybernetic feedback model, gate verdicts flow back to
Scorer, Router, Composer, and the agent. Process rewards add two new feedback channels:

| Signal | From | To | Effect |
|---|---|---|---|
| Promise | Gate + ToolCallMeta | Conductor | Early termination on low Promise |
| Progress | Gate (across attempts) | Router | Model re-selection on stalling |
| Progress | Gate (across attempts) | Composer | Prompt adjustment on stalling |
| Promise × Progress | Combined | Policy | Re-plan on persistent low signals |

This creates a multi-timescale control system:
- **Fast**: Promise per-turn → terminate unproductive attempts
- **Medium**: Progress per-attempt → adjust routing and prompts
- **Slow**: Trend across plans → update model preferences and prompt templates

> **Citation**: refactoring-prd/01-synapse-architecture.md — Cybernetic feedback loops
> from Gate to Scorer, Router, Composer.

---

## 9. Relationship to Predictive Foraging

Process rewards connect to the predictive foraging system (from the agent-chain
architecture). Before a task starts, the router predicts success probability. After the
task runs, the actual outcome produces a residual (prediction - reality). Process
rewards refine these predictions:

- If Promise is high but the task fails → the predictor was overconfident
- If Promise is low but the task succeeds → the predictor was underconfident
- Residuals from process rewards calibrate the predictor faster than binary outcomes

> **Citation**: tmp/implementation-plans/modelrouting/12-advanced-patterns.md §B
> (Tasks 2J.03–2J.04) — Predictive Foraging: prediction → residual → bias correction.

---

## 10. Summary

Process reward models transform binary gate verdicts into a continuous signal of agent
quality. By tracking Promise (is this attempt heading somewhere good?) and Progress (is
the agent improving across attempts?), the system can make finer-grained decisions about
when to continue, when to terminate early, and when to re-plan entirely.

The data is already there — gate verdicts, tool call metadata, ratchet state, diff
analysis. Process rewards are a *lens* on this data, not a new data collection system.
They turn "passed/failed" into "making progress / stalling / regressing" and enable
interventions that save compute and improve outcomes.
