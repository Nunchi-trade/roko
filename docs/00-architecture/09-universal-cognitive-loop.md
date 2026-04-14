# The Universal Cognitive Loop

> **Abstract:** Every agent in Roko — coding, chain, research, custom — runs the same
> 9-step cognitive loop at its own timescale. This loop is the heartbeat of the Synapse
> Architecture: it maps the six traits to a concrete execution sequence that implements
> the perception-decision-action cycle from CoALA (Sumers et al. 2023, arXiv:2309.02427)
> with additions from active inference (Friston 2010) and the Good Regulator Theorem
> (Conant & Ashby 1970). This document specifies the loop, maps each step to its trait,
> shows the shipping implementation, and explains how the loop runs at three cognitive speeds.


> **Implementation**: Shipping

---

## 1. The 9-Step Loop

```
1. PERCEIVE      → Substrate.query()       What is happening?
2. EVALUATE      → Scorer.score()          How relevant/important is each result?
3. ATTEND        → Router.select()         What matters most right now?
4. INTEGRATE     → Composer.compose()      Build the context window under budget
5. ACT           → Agent.execute()         Call LLM, produce output
6. VERIFY        → Gate.verify()           Did it work? (external truth)
7. PERSIST       → Substrate.put()         Store output with lineage (audit DAG)
8. ADAPT         → Policy.decide()         What patterns emerged?
9. META-COGNIZE  → Daimon.assess()         Am I doing this well?
```

Each step maps to exactly one Synapse trait (or cognitive subsystem). The loop is universal —
the same nine steps execute whether the agent is writing code, managing a DeFi position,
conducting research, or consolidating knowledge during a Dreams cycle. What differs is the
trait implementations injected at each step.

---

## 2. Step-by-Step Specification

### Step 1: PERCEIVE — Substrate.query()

**What happens**: The agent queries its Substrate for relevant Engrams matching the current
task or goal.

**Trait**: `Substrate.query(q, ctx)`

**Input**: A `Query` filtering by Kind, tags, time range, and minimum weight. The Context
carries the current time (for decay) and the goal (for relevance).

**Output**: A vector of candidate Engrams.

**Mapping to CoALA**: This is the working memory retrieval step — the agent surveys what
information is available before making any decisions.

### Step 2: EVALUATE — Scorer.score()

**What happens**: Each candidate Engram is scored along multiple axes to determine its
relevance, quality, and priority.

**Trait**: `Scorer.score(signal, ctx)` applied to each candidate.

**Input**: The candidate Engrams from Step 1 and the current Context.

**Output**: Updated Score on each candidate (or a separate scoring table).

**Note**: In the current `loop_tick` implementation, scoring is implicit — the Router uses
scoring internally. In the full architecture, explicit scoring precedes routing.

### Step 3: ATTEND — Router.select()

**What happens**: The Router selects one candidate from the scored set — the Engram that
matters most right now.

**Trait**: `Router.select(candidates, ctx)`

**Input**: Scored candidates from Step 2.

**Output**: A `Selection` identifying the chosen Engram with confidence and reasoning.

**Mapping to Active Inference**: Attention selection maps to the exploration/exploitation
tradeoff in Expected Free Energy. High-epistemic-value candidates (novel, uncertain) compete
with high-pragmatic-value candidates (useful, reliable).

### Step 4: INTEGRATE — Composer.compose()

**What happens**: The selected Engram (and potentially other context) is assembled into a
complete context window under budget constraints.

**Trait**: `Composer.compose(signals, budget, scorer, ctx)`

**Input**: The selected Engram(s), a Budget (token/byte limits), a Scorer for ranking
inclusions, and the Context.

**Output**: A composed Engram — typically a complete prompt ready for LLM inference.

**Budget awareness**: This is where the fundamental constraint of LLM context windows is
managed. The Composer decides what to include and what to drop.

### Step 5: ACT — Agent.execute()

**What happens**: The composed Engram (prompt) is sent to an LLM or tool for execution.

**Mapping**: This step is not a Synapse trait per se — it is the agent dispatch layer
(`roko-agent`), which handles LLM backend selection, API calls, streaming, and tool
execution.

**Output**: An Engram containing the agent's output (code, analysis, plan, etc.).

### Step 6: VERIFY — Gate.verify()

**What happens**: The agent's output is verified against external reality.

**Trait**: `Gate.verify(signal, ctx)`

**Input**: The output Engram from Step 5.

**Output**: A `Verdict` — passed/failed with evidence (reason, score, test counts,
error digest).

**Why this matters**: Verification is not optional. Every agent output passes through the
gate pipeline before being accepted. This is step 6 of 9, not an afterthought.

### Step 7: PERSIST — Substrate.put()

**What happens**: If the gate passed, the output Engram is stored in the Substrate with its
lineage (parent Engrams it derived from).

**Trait**: `Substrate.put(signal)`

**Input**: The verified output Engram.

**Output**: The Engram's ContentHash (for future reference).

**Lineage**: The stored Engram's lineage field links it to the input Engrams, the prompt,
the gate verdict — creating the audit DAG.

### Step 8: ADAPT — Policy.decide()

**What happens**: Policies examine the recent Engram stream (including the just-stored
output) and emit reactive Engrams — episode logs, efficiency metrics, circuit breaker
triggers, pheromone signals.

**Trait**: `Policy.decide(stream, ctx)`

**Input**: The recent Engram stream (including the output from this tick).

**Output**: Zero or more reactive Engrams, each stored in the Substrate.

### Step 9: META-COGNIZE — Daimon.assess()

**What happens**: The Daimon (see [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md))
assesses the agent's overall cognitive state: confidence, arousal, dominance. This assessment
influences the next tick's tier routing (T0/T1/T2) and the operating frequency
(Gamma/Theta/Delta).

**Mapping to Good Regulator Theorem**: This step implements the self-model requirement —
the agent must contain a model of itself to regulate itself effectively (Conant & Ashby 1970,
International Journal of Systems Science 1(2)).

---

## 3. The Shipping Implementation

The `loop_tick` function in `roko-core/src/loop_tick.rs` implements a streamlined version
of the 9-step loop:

```rust
pub async fn loop_tick(
    substrate: &dyn Substrate,
    scorer: &dyn Scorer,
    router: &dyn Router,
    composer: &dyn Composer,
    gate: &dyn Gate,
    policy: &dyn Policy,
    query: &Query,
    budget: &Budget,
    ctx: &Context,
) -> Result<TickOutcome> {
    // 1. PERCEIVE: Query the substrate for candidates.
    let candidates = substrate.query(query, ctx).await?;
    let candidates_examined = candidates.len();

    if candidates.is_empty() {
        return Ok(TickOutcome {
            candidates_examined: 0,
            composed: None,
            verdict: None,
            emitted: Vec::new(),
            written: Vec::new(),
        });
    }

    // 3. ATTEND: Router selects one candidate.
    let Some(selection) = router.select(&candidates, ctx) else {
        return Ok(TickOutcome {
            candidates_examined,
            composed: None, verdict: None,
            emitted: Vec::new(), written: Vec::new(),
        });
    };

    // 4. INTEGRATE: Composer builds a new Engram from the selection.
    let Some(chosen) = candidates.iter()
        .find(|s| s.id == selection.chosen).cloned() else {
        return Ok(TickOutcome {
            candidates_examined,
            composed: None, verdict: None,
            emitted: Vec::new(), written: Vec::new(),
        });
    };
    let composed = composer.compose(&[chosen], budget, scorer, ctx)?;

    // 6. VERIFY: Gate verifies the composition.
    let verdict = gate.verify(&composed, ctx).await;

    // 7-8. PERSIST + ADAPT: If passed, store and run policy.
    let mut written = Vec::new();
    let mut emitted = Vec::new();
    if verdict.passed {
        let id = substrate.put(composed.clone()).await?;
        written.push(id);

        let reactions = policy.decide(
            std::slice::from_ref(&composed), ctx
        );
        for r in reactions {
            let id = substrate.put(r.clone()).await?;
            written.push(id);
            emitted.push(r);
        }
    }

    Ok(TickOutcome {
        candidates_examined,
        composed: Some(composed),
        verdict: Some(verdict),
        emitted,
        written,
    })
}
```

### 3.1 TickOutcome

```rust
pub struct TickOutcome {
    pub candidates_examined: usize,
    pub composed: Option<Signal>,
    pub verdict: Option<Verdict>,
    pub emitted: Vec<Signal>,
    pub written: Vec<ContentHash>,
}

impl TickOutcome {
    pub fn passed(&self) -> bool {
        self.verdict.as_ref().is_some_and(|v| v.passed)
    }

    pub const fn did_work(&self) -> bool {
        self.candidates_examined > 0
    }
}
```

### 3.2 What the Current Implementation Omits

Steps 2 (EVALUATE), 5 (ACT), and 9 (META-COGNIZE) are not in `loop_tick` because:

- **EVALUATE**: Scoring is implicit in the Router's selection logic
- **ACT**: Agent execution happens at a higher level (the orchestrator calls `loop_tick`
  as part of a larger workflow that includes LLM dispatch)
- **META-COGNIZE**: Daimon assessment runs on a separate timer (Theta frequency)

The `loop_tick` function is the kernel — the minimal composable unit. The full 9-step loop
is assembled by the orchestrator layer.

---

## 4. The Loop at Three Speeds

The same loop structure runs at three cognitive speeds concurrently (see
[10-three-cognitive-speeds.md](10-three-cognitive-speeds.md)):

| Speed | Period | What the Loop Does |
|---|---|---|
| **Gamma** | ~5-15s | One reactive tick: perceive, route, compose, act, verify, persist |
| **Theta** | ~75s | Reflective tick: summarize recent work, update Daimon state, check predictions |
| **Delta** | Hours | Consolidation: Dreams replay, knowledge synthesis, tier promotion |

All three run the same nine steps but with different Query filters, different Scorers
(recency-weighted for Gamma, pattern-oriented for Theta, synthesis-oriented for Delta), and
different Composers (tight budget for Gamma, broader for Theta, comprehensive for Delta).

---

## 5. Loop Universality

The claim that the loop is universal means: every operation in Roko can be expressed as a
specific configuration of `loop_tick` arguments. Training the scaffold optimizer, picking a
model, running a gate, assembling a prompt, claiming a bounty — all are loop_tick invocations
with different Substrates, Scorers, Gates, Routers, Composers, and Policies.

This universality is what makes the architecture composable — if you can express your
operation as "query some Engrams, score them, select one, compose a result, verify it,
persist it, and react," then Roko can run it.

---

## 6. Mapping to Established Cognitive Architectures

The 9-step loop can be compared against three established cognitive architectures to identify
structural correspondences and unique additions:

### 6.1 LIDA (Learning Intelligent Distribution Agent)

LIDA implements Global Workspace Theory (Baars 1988) with a three-phase cognitive cycle
(~260-390ms per cycle in biological timing):

| LIDA Phase | Duration | Roko Mapping |
|---|---|---|
| **Understanding** (stimuli → situational model) | ~200ms | Steps 1-2: PERCEIVE + EVALUATE |
| **Consciousness** (attention codelets compete for broadcast) | ~60ms | Steps 3-4: ATTEND + INTEGRATE |
| **Action + Learning** (broadcast, select, execute, learn) | ~130ms | Steps 5-9: ACT through META-COGNIZE |

**Key insight from LIDA**: Attention should be **competitive, not deterministic**. LIDA's
attention codelets form coalitions that compete for the "consciousness spotlight." In Roko,
this maps to multiple Scorer implementations competing to influence Router selection. The
current implementation uses a single Scorer; a codelet-inspired design would have multiple
independent Scorers running concurrently, with the Router arbitrating between their
assessments.

**Reference**: Franklin, S. et al. (2016). "LIDA: A Systems-level Architecture for Cognition,
Emotion, and Learning." IEEE Trans. Autonomous Mental Development 6(1).

### 6.2 ACT-R (Adaptive Control of Thought—Rational)

ACT-R (Anderson 1983, 2007) uses subsymbolic activation-based memory retrieval with a ~50ms
production cycle:

| ACT-R Module | Roko Mapping |
|---|---|
| **Declarative memory** (activation-based retrieval) | Substrate.query() with decay-weighted scores |
| **Procedural memory** (production rules) | Policy.decide() — pattern-action rules |
| **Goal buffer** (current task focus) | Context.goal — drives Query filtering |
| **Conflict resolution** (utility-based selection) | Router.select() — bandit-based selection |
| **Base-level learning** (activation from recency/frequency) | Decay::Ebbinghaus with knowledge tiers |

ACT-R's base-level learning equation `B_i = ln(Σ t_j^{-d})` is structurally similar to
Roko's Ebbinghaus decay, both producing logarithmic forgetting curves. The key difference:
ACT-R models human cognitive latency; Roko models LLM-era timescales (seconds, not milliseconds).

### 6.3 Soar (State, Operator And Result)

Soar (Laird 2012) uses an impasse-driven architecture where failure to make a decision
triggers deeper reasoning:

| Soar Concept | Roko Mapping |
|---|---|
| **Working memory** | Substrate (current session signals) |
| **Operator proposal + selection** | Router.select() |
| **Operator application** | Agent.execute() (Step 5) |
| **Impasse → subgoal** | T0 → T1 → T2 escalation (prediction error triggers deeper reasoning) |
| **Chunking** (compile experience into rules) | Dreams NREM replay + Neuro tier promotion |

Soar's impasse mechanism is the closest analog to Roko's dual-process tier escalation:
when the system cannot make a decision at the current level, it drops into a deeper processing
mode. In Soar this creates a subgoal; in Roko it escalates from T0 to T1 or T2.

**Reference**: Laird, J. E. (2012). "The Soar Cognitive Architecture." MIT Press.

### 6.4 What Roko Adds

Operations unique to Roko that are absent from LIDA, ACT-R, and Soar:

| Roko Feature | Absent From | Why It Matters |
|---|---|---|
| **Step 6: VERIFY** (Gate) | All three | Classical architectures trust their own outputs; Roko verifies against external reality |
| **Step 7: PERSIST** (content-addressed, lineage-tracked) | LIDA, Soar | Forensic auditability; every decision is traceable |
| **Decay as first-class** | ACT-R has activation decay; LIDA, Soar don't | All information in Roko decays; prevents stale data poisoning |
| **Budget-constrained composition** | All three | LLM context windows impose hard token limits; classical architectures don't face this constraint |

---

## 7. The Gate as Prediction-Error Detector

Active inference (Friston 2010) frames verification differently from traditional pass/fail
testing. In the active inference view, the Gate is a **prediction-error detector**: it
measures the divergence between what the agent predicted (the composed output) and external
reality (compilation results, test outcomes, simulation results).

### 7.1 Low Surprise → Strengthen Model

When a Gate passes with high confidence (verdict.score > 0.9), the prediction error is low.
This should:
- Strengthen the Router's confidence in the selected path (positive feedback via Outcome)
- Promote related knowledge in NeuroStore (low surprise validates existing knowledge)
- Reduce Daimon's arousal (everything is working as expected)

### 7.2 High Surprise → Update Model

When a Gate fails (verdict.passed = false), the prediction error is high. This should:
- Trigger Router learning via negative Outcome feedback
- Generate a new Engram with Kind::Insight capturing what went wrong
- Increase Daimon's arousal (something unexpected happened; pay more attention)
- During Dreams consolidation, prioritize this episode for NREM replay (Mattar & Daw 2018)

### 7.3 Current vs. Active-Inference Implementation

| Behavior | Current | Active Inference Ideal |
|---|---|---|
| Gate passes | Persist output, run Policy | Persist output, run Policy, **strengthen generative model** |
| Gate fails | Discard output | Discard output, **create learning signal**, update world model |
| Gate marginal (0.4-0.6 score) | Binary pass/fail | **Epistemic exploration**: retry with modified approach to reduce uncertainty |

The current implementation handles the binary case (pass/fail). The active inference extension
would add gradient-based learning from gate confidence scores, enabling the system to learn
not just *whether* an approach works but *how well* it works and *why*.

**Reference**: VERSES AI (2025). "Genius: Renormalizing Generative Models for active inference."
[verses.ai/active-inference-research](https://www.verses.ai/active-inference-research)

---

## 8. The OODA Loop: Boyd's Full Theory

The common "Observe-Orient-Decide-Act" simplification distorts Boyd's actual framework
(Osinga 2007, *Science, Strategy and War*, Routledge). Key corrections:

### 8.1 Orient Is the Dominant Phase

Orient is not one of four equal steps — it is the cognitive engine. Boyd defined orientation
as an "interactive process of many-sided implicit cross-referencing projections, empathies,
correlations, and rejections" shaped by five inputs: genetic heritage, cultural traditions,
previous experiences, new information, and analysis/synthesis.

In Roko, Step 9 (META-COGNIZE via `Daimon.assess()`) is the Orient analog. The Daimon's
PAD vector is the compressed representation of orientation. T0/T1/T2 tier routing is Boyd's
"implicit guidance and control" — orientation directly shapes what gets observed next (Query
filtering) and whether action is explicit or implicit.

### 8.2 Implicit Guidance and Control

Boyd's diagram shows Orient feeding directly into Action, **bypassing** Decision entirely.
Experienced practitioners act without explicit deciding. This maps to T0 probes — the agent
acts on cached heuristics without invoking an LLM (no explicit "decision" step).

### 8.3 Tempo as Strategic Advantage

Boyd's core insight: completing the cognitive loop faster than your environment changes
produces confusion and paralysis in adversaries. For agents, the limiting factor is Orient
(world-model update rate), not Act (execution speed). An agent with faster action but slower
orientation will be dominated by one that acts slower but orients faster.

---

## 9. Kolb Experiential Learning Cycle

Kolb (1984, *Experiential Learning*, Prentice Hall) defines learning as "the process whereby
knowledge is created through the transformation of experience." The four stages map directly
to Roko's three cognitive speeds:

| Kolb Stage | Definition | Roko Mapping | Speed |
|---|---|---|---|
| **Concrete Experience** | Doing or having an experience | Agent.execute() + Gate.verify() | Gamma |
| **Reflective Observation** | Reviewing the experience | Daimon.assess() + episode analysis | Theta |
| **Abstract Conceptualization** | Drawing conclusions, building models | Dreams NREM replay → Neuro promotion | Delta |
| **Active Experimentation** | Testing new theories | Updated Router + Policy for next tick | Gamma |

Kayes (2002, Academy of Management Learning & Education 1(2)) critiques Kolb on three
grounds relevant to agent design: (1) pure self-reflection reinforces biases without external
feedback — this is why Roko requires external Gate verdicts, not self-evaluation; (2) tacit
knowledge is not captured by explicit reflection — this maps to somatic markers in the Daimon;
(3) the model privileges individual cognition over social learning — this maps to the need for
collective calibration via C-Factor.

---

## 10. Predictive Processing Reframing

Clark (2013, BBS 36(3)) and Friston (2010) reframe the cognitive loop fundamentally:

### 10.1 Perception as Active Inference

Classical frameworks (including OODA) treat perception as passive reception. Predictive
processing says perception is **active inference**: the brain continuously generates
top-down predictions and only propagates the **prediction error** (bottom-up) when
predictions fail. What reaches higher cognition is not raw data but surprise.

In Roko, this reframes Step 1 (PERCEIVE): `Substrate.query()` should not return all matching
Engrams equally — it should return Engrams weighted by their surprise relative to the
agent's current model. High-surprise Engrams deserve more attention (higher Score).

### 10.2 Action as Error Suppression

Action is not a response to perception — it is the suppression of prediction error. The
agent predicts a desired state and acts to make that prediction true. This explains why
Step 5 (ACT) and Step 6 (VERIFY) are symmetric operations, not an afterthought:

```
ACT:    "I predict the code will compile after this edit"
VERIFY: "Did it compile?" → prediction error signal
```

### 10.3 Attention as Precision-Weighting

Clark argues that attention modulates the **precision** (inverse variance) of prediction
errors. High attention = high precision weight on that channel's errors. This is entirely
different from OODA-style spotlight attention and maps directly to Roko's planned `precision`
score axis — Engrams with high precision deserve more attentional weight.

---

## 11. Formal Verification of Loop Properties

The cognitive loop is a **reactive system** (Manna & Pnueli 1992): it maintains ongoing
interaction with an environment rather than computing a final result. Temporal logic provides
the specification language for its correctness properties.

### 11.1 Safety Properties (□ P — "always P")

| Property | LTL Formula | Description |
|---|---|---|
| Budget safety | `□ (cost ≤ budget)` | Token/dollar budget is never exceeded |
| No unverified output | `□ (output → verified)` | Every persisted Engram has passed a Gate |
| No invalid state | `□ ¬bad_state` | Loop never enters an irreversible error state |

### 11.2 Liveness Properties (◇ P — "eventually P")

| Property | LTL Formula | Description |
|---|---|---|
| Termination | `◇ done` | Every accepted task eventually completes or is abandoned |
| Progress | `□ (queued → ◇ processed)` | No task remains queued forever |
| Response | `□ (submitted → ◇ verdict)` | Every Gate submission gets a verdict or timeout |
| Recovery | `□ (failure → ◇ recovered)` | After failure, loop eventually reaches recovered state |

### 11.3 Fairness Properties

| Property | Type | Description |
|---|---|---|
| Non-starvation | Weak fairness | If a task is continuously available, it is eventually selected |
| Gate fairness | Strong fairness | If the LLM API is infinitely often available, responses arrive |

### 11.4 Model Checking Applicability

CTL model checking is PTIME; LTL is PSPACE-complete. For practical verification of the
cognitive loop, CTL is computationally preferable. TLA+ (Lamport 1994) provides the most
practical specification language for distributed/concurrent loop properties.

Rice's Theorem (1953) sets the ceiling: some loop properties (semantic correctness of outputs)
are undecidable and require external judgment (LLM-as-judge or human review). The Gate pipeline
handles this by escalating undecidable properties to semantic gates.

---

## 12. Loop Instrumentation via OpenTelemetry

The OpenTelemetry GenAI Semantic Conventions (2024-present) provide the instrumentation
standard for cognitive loops.

### 12.1 Per-Step Span Instrumentation

| Loop Step | Span Name | Key Attributes | Key Metrics |
|---|---|---|---|
| PERCEIVE | `substrate.query` | query_kind, filter_tags | latency_ms, candidates_returned |
| EVALUATE | `scorer.score` | scorer_type, engram_count | latency_ms, mean_score |
| ATTEND | `router.select` | confidence, candidates_examined | latency_ms, top_score_delta |
| INTEGRATE | `composer.compose` | budget_tokens, actual_tokens | latency_ms, budget_utilization |
| ACT | `agent.execute` | model, input_tokens, output_tokens | TTFT, cost_usd |
| VERIFY | `gate.verify` | rungs, verdict_passed, verdict_score | latency_per_rung, pass_rate |
| PERSIST | `substrate.put` | content_hash, lineage_depth | latency_ms, bytes_written |
| ADAPT | `policy.decide` | policies_evaluated, emissions_count | latency_ms, emissions_per_tick |
| META-COGNIZE | `daimon.assess` | pad_p, pad_a, pad_d, tier | latency_ms, tier_escalation_rate |

### 12.2 Root Span Metrics

```
loop_tick.duration      — total wall time for one cognitive cycle
loop_tick.tier          — T0/T1/T2 (attribute for cost-per-tier histograms)
loop_tick.passed        — boolean: did the tick produce a verified output
loop_tick.cost_usd      — total dollar cost of the tick
loop_tick.candidates    — top-of-funnel count
loop_tick.gate_score    — continuous quality signal [0, 1]
```

### 12.3 Lessons from Classical Architectures

From ACT-R (Anderson 1983): instrument **state transitions**, not just operations. The most
informative metric is the change between phases, not the duration of each phase.

From LIDA (Franklin et al. 2016): measure **attention competition**. The key observable is
which coalition won, by what margin, at what time — not just the final selection.

From Soar (Laird 2012): separate **architectural metadata** (activation weights, utility
scores) from **agent data** (task content, plans). Observability should expose metadata
without perturbing it.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Sumers et al. 2023 (arXiv:2309.02427) | CoALA: perception-decision-action cycle. Direct blueprint for the loop. |
| Friston 2010, Nature Reviews Neuroscience 11(2) | Active inference: perception and action minimize prediction error. |
| Clark 2013, BBS 36(3) | Predictive processing: perception as active inference, attention as precision-weighting. |
| Conant & Ashby 1970, IJSS 1(2) | Good Regulator Theorem: agent must model itself. Motivates META-COGNIZE. |
| Boyd 1976/1987 | OODA loop. Orient as dominant phase; implicit guidance; tempo as strategy. |
| Osinga 2007, Routledge | Full Boyd theory: destruction/creation, entropy, epistemological foundations. |
| Kolb 1984, Prentice Hall | Experiential Learning Cycle: CE → RO → AC → AE. |
| Kayes 2002, AMLE 1(2) | Critique of Kolb: self-reflection reinforces bias without external feedback. |
| Pnueli 1977, 18th IEEE FOCS | Temporal Logic of Programs: LTL for safety and liveness properties. |
| Manna & Pnueli 1992, Springer | Temporal logic of reactive and concurrent systems. |
| Lamport 1994, ACM TOPLAS 16(3) | TLA+: Temporal Logic of Actions for system specification. |
| OpenTelemetry GenAI SIG 2024 | Semantic Conventions for Generative AI instrumentation. |
| Anderson 1983, Harvard University Press | ACT-R: production system cognitive architecture. |
| Franklin et al. 2016, IEEE Trans. AMD 6(1) | LIDA: Global Workspace Theory three-phase cognitive cycle. |
| Laird 2012, MIT Press | Soar: impasse-driven architecture with chunking. |
| Laird 2022 (arXiv:2201.09305) | ACT-R and Soar comparison. |
| Parr et al. 2022, MIT Press | Active Inference textbook: EFE decomposition and formalism. |
| Dehaene et al. 2021 (arXiv:2502.21142) | GW-Dreamer: Global Workspace Theory + world-model RL. |
| Millidge et al. 2021, Neural Computation 33(2) | EFE correction: naively extending FE discourages exploration. |

---

## Current Status and Gaps

- **Implemented**: `loop_tick` in `roko-core` with all six trait parameters. TickOutcome
  with `passed()` and `did_work()` helpers.
- **Wired**: The orchestrator (`roko-cli/src/orchestrate.rs`) calls the loop as part of
  plan execution.
- **Gap**: Steps 2, 5, 9 are handled outside `loop_tick`. A future refactor may inline
  them for a complete 9-step function.
- **Gap**: Gate feedback is binary (pass/fail); active inference would use gradient confidence
  scores for continuous model updating.
- **Opportunity**: LIDA-style competitive attention — multiple Scorers running concurrently
  with Router arbitrating coalitions.

---

## Cross-References

- [06-synapse-traits.md](06-synapse-traits.md) — The six traits used in the loop
- [10-three-cognitive-speeds.md](10-three-cognitive-speeds.md) — The loop at Gamma/Theta/Delta
- [11-dual-process-and-active-inference.md](11-dual-process-and-active-inference.md) — EFE drives tier routing
- [12-five-layer-taxonomy.md](12-five-layer-taxonomy.md) — Which layer hosts each step
- [23-architectural-analysis-improvements.md](23-architectural-analysis-improvements.md) — Full architectural analysis
