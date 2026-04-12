# 09 — Evaluation Lifecycle

> **Layer**: L3 Harness — Verification
> **Crates**: `roko-gate`, `roko-learn`, `roko-conductor`
> **Status**: Partial (fast loops wired, slow loops designed)

---

## 1. Overview

Evaluation in Roko is not a single step — it is a lifecycle that spans five speed tiers,
from sub-second machine checks to multi-day retrospective analysis. The gate pipeline is
the fastest tier. The evaluation lifecycle describes how gate verdicts compound, combine
with other signals, and drive progressive improvement across time.

The lifecycle has 14 feedback loops organized across 5 speed tiers. Each loop composes
with the others — the output of fast loops feeds into slower loops, and the insights
from slow loops adjust the parameters of fast loops.

> **Citation**: bardo-backup/prd/16-testing/09-evaluation-map.md — "14 feedback loops
> across 5 speed tiers, complete composition diagram."

---

## 2. The Five Speed Tiers

| Tier | Speed | Loops | What Runs Here |
|---|---|---|---|
| Machine speed | Sub-second to seconds | 5 | Confidence calibration, context attribution, cost-effectiveness, tool selection, adversarial awareness |
| Cognitive speed | Seconds to minutes | 3 | Gate pipeline, error diagnosis, retry logic |
| Consolidation speed | Minutes to hours | 3 | Skill extraction, pattern discovery, model calibration |
| Retrospective speed | Hours to days | 2 | Shadow testing, reasoning quality review |
| Meta speed | Days to weeks | 1 | Meta-learning evaluation |

### 2.1 Machine Speed (5 Loops)

These run within or immediately after a single agent turn:

**Loop 1: Confidence Calibration**
The agent (or router) predicts success probability before the gate runs. After the gate
runs, the prediction is compared to the outcome. Residuals accumulate, enabling
calibration correction.

Metric: Expected Calibration Error (ECE) — the average gap between predicted and actual
pass rates across confidence bins.

**Loop 2: Context Attribution**
Which parts of the prompt contributed to gate success? The section effectiveness tracker
correlates prompt sections with outcomes.

**Loop 3: Cost-Effectiveness**
Did the agent's token spend produce proportionate verification results? A 50,000-token
turn that fails all gates is less cost-effective than a 10,000-token turn that passes
all gates.

**Loop 4: Tool Selection**
Are the agent's tool call patterns efficient? Redundant file reads, unnecessary edits,
and tool calls that don't advance the task are identified.

**Loop 5: Adversarial Awareness**
Does the agent detect adversarial inputs (prompt injections, malicious test fixtures)?
This loop monitors the agent's defensive behavior.

> **Citation**: bardo-backup/prd/16-testing/07-fast-feedback-loops.md — "5 machine-speed
> evaluation loops: confidence calibration, context attribution, cost-effectiveness, tool
> selection, adversarial awareness."

### 2.2 Cognitive Speed (3 Loops)

These run during a single task execution (across multiple turns):

**Loop 6: Gate Pipeline**
The rung selector → gate pipeline → verdict cycle. This is the core verification loop,
documented in [03-gate-pipeline.md](./03-gate-pipeline.md).

**Loop 7: Error Diagnosis**
Gate error output is parsed into structured feedback (see
[08-agent-feedback-from-gates.md](./08-agent-feedback-from-gates.md)) and enriched
with cheap-model diagnosis.

**Loop 8: Retry Logic**
The orchestrator decides whether to retry, escalate, or re-plan based on the verdict
and process reward signals.

### 2.3 Consolidation Speed (3 Loops)

These run after a batch of tasks (e.g., after a full plan execution):

**Loop 9: Skill Extraction**
Successful episodes are analyzed to extract reusable tool-use patterns (see
[11-evoskills.md](./11-evoskills.md)).

**Loop 10: Pattern Discovery**
Cross-task analysis identifies recurring success/failure patterns. E.g., "tasks that
modify auth modules fail 3x more often than average."

**Loop 11: Model Calibration**
Aggregate per-model performance data to calibrate the router's bandit arms. Thompson
Sampling parameters are updated based on gate outcomes.

### 2.4 Retrospective Speed (2 Loops)

These run on a longer cadence (nightly, weekly):

**Loop 12: Shadow Testing**
Run the same tasks with different models/prompts in shadow mode and compare outcomes.
This discovers whether the current routing is optimal.

**Loop 13: Reasoning Quality Review**
Evaluate agent reasoning quality across a batch of completed tasks. Three signals:
alignment (did the agent follow the plan?), consistency (did reasoning stay coherent
across turns?), and annotations (did the agent leave useful comments?).

> **Citation**: bardo-backup/prd/16-testing/08-slow-feedback-loops.md — "3 slow loops:
> shadow strategy testing, reasoning quality review, meta-learning evaluation."

### 2.5 Meta Speed (1 Loop)

**Loop 14: Meta-Learning Evaluation**
Evaluate the evaluation system itself: are the 13 other loops improving outcomes over
time? This is the system's self-assessment, tracking whether its learning is net
positive.

---

## 3. Composition Diagram

The 14 loops compose hierarchically — faster loops feed data to slower loops:

```
Machine Speed                 Cognitive Speed
┌─────────────┐              ┌─────────────┐
│ Confidence  │──residuals──→│ Gate         │
│ Calibration │              │ Pipeline     │
└─────────────┘              └──────┬───────┘
┌─────────────┐                     │
│ Context     │──lift────────┐      │ verdicts
│ Attribution │              │      ↓
└─────────────┘              │ ┌────────────┐
┌─────────────┐              │ │ Retry      │
│ Cost-       │──efficiency──│ │ Logic      │
│ Effectiveness│             │ └────────────┘
└─────────────┘              │
┌─────────────┐              │  Consolidation Speed
│ Tool        │──patterns────│  ┌──────────────┐
│ Selection   │              ├─→│ Skill         │
└─────────────┘              │  │ Extraction    │
┌─────────────┐              │  └───────────────┘
│ Adversarial │──alerts──────┤  ┌──────────────┐
│ Awareness   │              ├─→│ Pattern       │
└─────────────┘              │  │ Discovery     │
                             │  └───────────────┘
                             │  ┌──────────────┐
                             └─→│ Model         │
                                │ Calibration   │
                                └───────┬───────┘
                                        │
                Retrospective Speed     │
                ┌──────────────┐        │
                │ Shadow       │←───────┘
                │ Testing      │
                └──────────────┘
                ┌──────────────┐
                │ Reasoning    │
                │ Quality      │
                └──────┬───────┘
                       │
           Meta Speed  │
           ┌───────────┴──┐
           │ Meta-Learning │
           │ Evaluation    │
           └──────────────┘
```

---

## 4. The Karpathy Property

Every loop in the evaluation lifecycle satisfies the Karpathy Property: **if the
evaluation metric improves, the system's end-to-end performance improves**. This is a
design constraint, not an observation. It means:

- No metric that is uncorrelated with actual task success
- No metric that can be gamed without improving outcomes
- No metric that improves at the expense of another metric

For gate-based loops, the Karpathy Property is straightforward: if the compile gate pass
rate improves, the system is producing better code. For slower loops (reasoning quality,
shadow testing), the property requires careful metric design.

> **Citation**: bardo-backup/prd/16-testing/07-fast-feedback-loops.md — "Karpathy
> autoresearch loop" and Karpathy Property across all loops.

> **Citation**: bardo-backup/prd/16-testing/09-evaluation-map.md — "Karpathy property
> across all loops."

---

## 5. The Four-Phase Lifecycle

Beyond the speed tiers, evaluation goes through four phases:

### Phase 1: Trace Inspection

Examine individual agent turns: what did the agent do, what tools did it call, what
was the result? This is the raw data layer.

**Data sources**: Episode log (`.roko/episodes.jsonl`), efficiency events
(`.roko/learn/efficiency.jsonl`).

### Phase 2: Backtesting

Replay past executions with different parameters: would a different model have
succeeded? Would a different prompt have helped? Would more/fewer retries have been
optimal?

**Data sources**: Artifact store (replay exact inputs), gate threshold history.

### Phase 3: Paper Trading

Run new configurations in shadow mode alongside production. Compare outcomes without
affecting real task execution.

**Data sources**: Shadow execution results vs. production results.

### Phase 4: Canary Deployment

Gradually roll out proven improvements to a fraction of tasks, monitoring for regressions
before full deployment.

**Data sources**: A/B experiment results (`.roko/learn/experiments.json`).

> **Citation**: bardo-backup/prd/16-testing/05-evaluation-lifecycle.md — "4-phase
> evaluation lifecycle: Trace Inspection → Backtesting → Paper Trading → Canary."

---

## 6. The Gauntlet

The Gauntlet is the benchmark suite that validates the evaluation lifecycle itself:

| Speed | Duration | Scope |
|---|---|---|
| Smoke | 5 minutes | Core gate pipeline on known test cases |
| Nightly | 2–4 hours | Full rung ladder on real project tasks |
| Full | 24–48 hours | All 14 loops, cross-model comparison |

The Gauntlet provides confidence that changes to the evaluation system don't regress
evaluation quality. It is the "gate for the gates."

> **Citation**: bardo-backup/prd/16-testing/01-gauntlet.md — Gauntlet benchmark suite,
> 3 speeds.

---

## 7. Gate Verdicts as the Foundation

Every loop in the evaluation lifecycle is ultimately grounded in gate verdicts. Even the
slowest loop (meta-learning) depends on the aggregate of gate outcomes to measure
whether the system is improving.

This is why the gate architecture is so important:
- Verdicts are the atomic unit of verification truth
- All 14 loops consume or aggregate verdicts
- The quality of the evaluation lifecycle is bounded by the quality of the gates

Improving gate fidelity (adding rungs, reducing false negatives, increasing coverage)
has multiplicative effects across all 14 loops. This is another manifestation of the
GVU framework's insight: invest in verification quality.

> **Citation**: Song et al. (ICLR 2025) — "Self-improvement succeeds when the verifier
> is strong, not when the generator is strong."

---

## 8. Currently Wired Components

| Component | Status | Where |
|---|---|---|
| Gate pipeline (Loop 6) | Wired | `orchestrate.rs` per-task |
| Error feedback (Loop 7) | Wired | `feedback.rs` → agent retry |
| Adaptive thresholds | Wired | `adaptive_threshold.rs` → persist |
| Efficiency events (Loops 1–5) | Wired | `.roko/learn/efficiency.jsonl` |
| Episode logging | Wired | `.roko/episodes.jsonl` |
| Model routing (Loop 11) | Wired | `cascade-router.json` |
| A/B experiments | Wired | `experiments.json` |
| Skill library (Loop 9) | Scaffold | `roko-learn/src/skill_library.rs` |
| Shadow testing (Loop 12) | Design | Implementation plan 2J |
| Meta-learning (Loop 14) | Design | Implementation plan Phase 7–8 |

---

## 9. Summary

The evaluation lifecycle is not just "run gates and check if they pass." It is a
multi-timescale system that:
1. Runs 14 feedback loops across 5 speed tiers
2. Compounds fast signals into slow insights
3. Uses slow insights to tune fast parameters
4. Validates itself through the Gauntlet

The gate pipeline is the heartbeat at the center. Everything else — calibration, skill
extraction, shadow testing, meta-learning — depends on the gate verdicts being
accurate, fast, and comprehensive.
