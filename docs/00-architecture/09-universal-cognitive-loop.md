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

## Academic Foundations

| Citation | Contribution |
|---|---|
| Sumers et al. 2023 (arXiv:2309.02427) | CoALA: perception-decision-action cycle for language agents. Direct blueprint for the loop. |
| Friston 2010, Nature Reviews Neuroscience 11(2) | Active inference: perception and action minimize prediction error. Motivates steps 1, 3, 6. |
| Conant & Ashby 1970, Intl. J. Systems Science 1(2) | Good Regulator Theorem: agent must model itself. Motivates step 9 (META-COGNIZE). |
| Boyd 1976 | OODA loop (Observe-Orient-Decide-Act). The military decision cycle that Roko extends with verification and meta-cognition. |
| Anderson 1983, The Architecture of Cognition | ACT-R: production system with perceive-decide-act cycle. Classical cognitive architecture reference. |

---

## Current Status and Gaps

- **Implemented**: `loop_tick` in `roko-core` with all six trait parameters. TickOutcome
  with `passed()` and `did_work()` helpers.
- **Wired**: The orchestrator (`roko-cli/src/orchestrate.rs`) calls the loop as part of
  plan execution.
- **Gap**: Steps 2, 5, 9 are handled outside `loop_tick`. A future refactor may inline
  them for a complete 9-step function.

---

## Cross-References

- [06-synapse-traits.md](06-synapse-traits.md) — The six traits used in the loop
- [10-three-cognitive-speeds.md](10-three-cognitive-speeds.md) — The loop at Gamma/Theta/Delta
- [11-dual-process-and-active-inference.md](11-dual-process-and-active-inference.md) — EFE drives tier routing
- [12-five-layer-taxonomy.md](12-five-layer-taxonomy.md) — Which layer hosts each step
