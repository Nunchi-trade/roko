# Dual-Process Cognition and Active Inference

> **Abstract:** Roko implements dual-process cognition inspired by Kahneman's System 1/
> System 2 (Kahneman 2011) and CLARION's dual-level architecture (Sun 2002). Three
> inference tiers -- T0 (no LLM), T1 (fast model), T2 (full model) -- are routed by
> active inference as a literal target-state Bus-driven predict/publish/correct/update loop:
> operators would publish `prediction.*` Pulses, later `outcome.*` Pulses would close the loop, and
> `prediction.error.*` Pulses drive calibration updates. This document specifies the tier
> model, the Expected Free Energy (EFE) formula that drives routing, the 16 T0 probes,
> and how uncertainty emerges from the architecture rather than being manually configured.
> See also [tmp/refinements/10-self-learning-cybernetic-loops.md](../../tmp/refinements/10-self-learning-cybernetic-loops.md)
> and [Naming Map and Glossary](01-naming-and-glossary.md).


> **Implementation**: Shipping

---

## 1. The Three Tiers

```
Low uncertainty  → T0 (direct tool call, no LLM)      ~80% of ticks (16 T0 probes)
               → T1 (fast model, shallow reasoning)  ~15% of ticks
High uncertainty → T2 (full model, deep reasoning)     ~5% of ticks
```

| Tier | What Runs | Approximate Cost | When Used |
|---|---|---|---|
| **T0** | Zero-LLM probes: config checks, threshold tests, regex matches, cache lookups | $0 | Nothing surprising detected. Agent coasts on heuristics. |
| **T1** | Fast model (e.g., Claude Haiku class). Narrow tool access. Quick analysis. | ~$0.001-0.003/call | Something surprised the gating threshold. Quick analysis needed. |
| **T2** | Full model (e.g., Claude Sonnet/Opus class). Full tool access. Multi-turn reasoning. | ~$0.01-0.25/call | Large prediction error or high stakes. Deep reasoning required. |

### 1.1 Cost Implications

The tier distribution (~80/15/5%) means that the vast majority of cognitive cycles cost
nothing. The FrugalGPT insight (Chen et al. 2023, arXiv:2305.05176) applied to cognitive
architecture: route easy work to cheap processing and reserve expensive reasoning for
genuinely difficult situations.

For a simple strategy where all conditions are numeric thresholds, the agent may never need
an LLM at all — T0 handles every tick. Zero inference cost.

### 1.2 The Escalation Path

Tier routing is not a fixed assignment — it is a **dynamic escalation**:

1. Every Gamma tick starts with T0 probes
2. If any probe reports surprise above threshold → escalate to T1
3. If T1's analysis reports high uncertainty or high stakes → escalate to T2
4. T2 produces a full analysis and action

This cascade ensures that compute is invested proportionally to difficulty.

---

## 2. Active Inference and Expected Free Energy

The routing between tiers is NOT a manual threshold — it emerges from active inference.
In Roko, that is not just a metaphor. It is a target-state Bus-driven predict/publish/correct/update
loop where operators would emit `prediction.*` Pulses, reality would answer with `outcome.*` Pulses,
and a calibration policy would turn the mismatch into update Pulses.

### 2.1 The EFE Formula

Expected Free Energy decomposes into two components:

```
G(π) = E_q[ log q(s|π) − log p(o,s|π) ]
     = − Pragmatic Value − Epistemic Value
```

Where:
- **Pragmatic value** = expected reward from acting on policy π
  (What will I gain by acting this way?)
- **Epistemic value** = expected information gain from observing under policy π
  (What will I learn by acting this way?)

### 2.2 How EFE Maps to Tier Routing

| Situation | Pragmatic Value | Epistemic Value | EFE | Tier |
|---|---|---|---|---|
| Routine tick, no surprise | High (known outcome) | Low (nothing to learn) | Low (confident) | T0 |
| Moderate surprise, known domain | Moderate | Moderate | Medium | T1 |
| Large surprise, unknown territory | Unknown | High (much to learn) | High (uncertain) | T2 |
| High stakes, low confidence | Low (risky) | High | High | T2 |

The key insight: EFE provides a **zero-hyperparameter** routing criterion. The agent does
not need manually-tuned thresholds for "when to use GPT-4 vs Haiku." Instead, the agent's
internal uncertainty -- as measured by prediction accuracy, confidence trends, and novelty
Pulses -- naturally drives the escalation decision.

In the target-state Bus implementation, this becomes observable and joinable:

1. An operator would publish a `prediction.*` Pulse before acting.
2. A later `outcome.*` Pulse would record what actually happened.
3. A calibration policy would join the two by lineage and publish `prediction.error.*`.
4. The operator would consume the update and adjust its internal state.

### 2.3 Practical Approximation

Computing exact EFE over a full generative model is intractable. Roko approximates it:

1. **Prediction accuracy** from the calibration stream: declining accuracy -> high epistemic
   value → escalate
2. **Confidence from Score**: low confidence on recent outputs → high uncertainty → escalate
3. **Novelty from Score**: high novelty in observations → high epistemic value → escalate
4. **Daimon arousal**: high arousal (the agent is "surprised") → escalate

These topic families combine in the CascadeRouter to produce a tier decision without explicit EFE
computation. In the target-state design, the Bus does the bookkeeping; the tier router consumes the calibrated result.

### 2.4 Per-operator calibration

The same loop applies to every operator, not just the Scorer. Each one can publish its own
prediction topic family, receive a matching outcome family, and consume
`prediction.error.*` updates through a calibration policy.

| Operator | Prediction topic | Outcome topic | Update policy |
|---|---|---|---|
| Scorer | `prediction.scorer.*` | `outcome.scorer.*` | Calibration curves per axis, then `scorer.weights.updated` |
| Router | `prediction.router.*` | `outcome.router.*` | Contextual bandit / route weights update |
| Composer | `prediction.composer.*` | `outcome.composer.*` | Template EMA and template bandit |
| Gate | `prediction.gate.*` | `outcome.gate.*` | Threshold EMA from verdict outcomes |
| Policy | `prediction.policy.*` | `outcome.policy.*` | Per-policy calibration from metric Pulses |
| Substrate | `prediction.substrate.*` | `outcome.substrate.*` | Tier promotion and retrieval calibration |

---

## 3. The 16 T0 Probes

T0 probes are zero-LLM diagnostic checks that run at Gamma frequency (~5-15s). They
determine whether the environment has changed enough to warrant LLM inference.

The 16 probes cover the complete diagnostic surface:

| # | Probe | What It Checks | If Triggered |
|---|---|---|---|
| 1 | `config_changed` | Has `roko.toml` or strategy file changed? | Escalate: configuration shift |
| 2 | `gate_failed_recently` | Did a gate fail in the last N ticks? | Escalate: verification regression |
| 3 | `file_modified` | Have watched files been modified externally? | Escalate: environment changed |
| 4 | `test_count_delta` | Did the test count change (new tests or removed)? | Escalate: test suite changed |
| 5 | `compile_error_new` | Are there new compilation errors? | Escalate: code broken |
| 6 | `budget_threshold` | Is the remaining budget below threshold? | Escalate: resource pressure |
| 7 | `confidence_dropping` | Is confidence trending downward? | Escalate: something is wrong |
| 8 | `prediction_violation` | Did a prediction fail to match reality? | Escalate: model is wrong |
| 9 | `tool_health_degraded` | Is a tool's response time or error rate degraded? | Escalate: tool problem |
| 10 | `pheromone_detected` | Has a new pheromone been deposited? | Escalate: collective signal |
| 11 | `task_deadline_near` | Is a task deadline approaching? | Escalate: urgency |
| 12 | `idle_timeout` | Has the agent been idle beyond threshold? | Escalate to Delta: consolidate |
| 13 | `knowledge_stale` | Is key knowledge past its freshness window? | Escalate: knowledge refresh |
| 14 | `dependency_changed` | Has an upstream dependency task completed? | Escalate: new work available |
| 15 | `metric_anomaly` | Is any tracked metric outside 2σ bounds? | Escalate: anomaly detected |
| 16 | `heartbeat_timeout` | Has the expected heartbeat interval elapsed? | Emit heartbeat, check status |

If all 16 probes return "no change," the tick completes at T0 cost ($0). This is how
~80% of ticks are suppressed — the FrugalGPT-inspired zero-cost majority.

---

## 4. Dual-Process Theory Mapping

The three-tier model maps to Kahneman's dual-process theory (Kahneman 2011, Thinking, Fast
and Slow) extended with CLARION's sub-conceptual level (Sun 2002):

| Kahneman | CLARION | Roko | Characteristics |
|---|---|---|---|
| — | Sub-conceptual | **T0** | Below conscious reasoning. Pattern matching, threshold checks, cached responses. |
| System 1 | Bottom-up | **T1** | Fast, automatic, heuristic. Quick analysis with limited tool access. |
| System 2 | Top-down | **T2** | Slow, deliberate, analytical. Full reasoning with complete tool access. |

### 4.1 CLARION Dual-Level

Ron Sun's CLARION architecture (Sun 2002, Duality of the Mind, Erlbaum) proposes that
cognition operates on two levels simultaneously: an explicit (rule-based) level and an
implicit (subsymbolic) level. Roko's T0 probes correspond to the implicit level — fast,
subsymbolic pattern matching that doesn't require explicit reasoning.

### 4.2 System 1 / System 2 in Practice

T1 (System 1) handles situations where a quick, heuristic response suffices: "this looks
like a formatting issue, apply the standard fix." T2 (System 2) handles situations requiring
deliberate analysis: "this test failure suggests a design flaw in the approach, reconsider
the architecture."

The escalation from T1 to T2 is driven by T1's own confidence assessment: if T1 produces
a response with low confidence, T2 is invoked for deeper analysis.

---

## 5. Classical Cognitive Architecture References

Roko's dual-process design draws from a rich tradition of cognitive architectures:

| Architecture | Key Insight | Roko Adoption |
|---|---|---|
| **ACT-R** (Anderson 1983) | Production systems with declarative/procedural memory | T0 probes as productions, knowledge as declarative memory |
| **SOAR** (Laird et al. 1987) | Universal subgoaling with impasses | T0 → T1 escalation as impasse detection |
| **CLARION** (Sun 2002) | Dual-level (explicit + implicit) processing | T0 as implicit, T1/T2 as explicit |
| **Global Workspace Theory** (Baars 1988) | Limited-capacity workspace for conscious processing | Context window as global workspace, Composer manages access |
| **CoALA** (Sumers et al. 2023) | Cognitive architecture for language agents | Direct structural blueprint for the cognitive loop |

LLM-era architectures that inform specific mechanisms:

| System | Key Insight | Roko Adoption |
|---|---|---|
| **ReAct** (Yao et al. 2022) | Interleaved reasoning and acting | The loop_tick alternates reasoning (compose) and acting (execute) |
| **Reflexion** (Shinn et al. 2023) | Self-reflection on failures | Theta ticks analyze failed gate verdicts |
| **Tree of Thoughts** (Yao et al. 2023) | Branching search over reasoning paths | Router.select() with multiple candidates |
| **ExpeL** (Zhao et al. 2023) | Learning from experience | Episode → Playbook extraction in roko-learn |
| **Voyager** (Wang et al. 2023) | Skill library accumulation | EvoSkills in roko-learn |
| **LATS** (Zhou et al. 2023) | Language Agent Tree Search | Search over Engram candidates via Router |

---

## 6. Temperament Profiling

The mapping between cognitive architecture and behavior is not fixed — it varies by agent
"temperament." The Daimon's PAD (Pleasure-Arousal-Dominance) vector modulates the T0→T1→T2
escalation thresholds, and recent `prediction.error.*` Pulses feed that state:

| Temperament | Effect on Routing |
|---|---|
| **High confidence + low arousal** (calm, competent) | Higher threshold for T1 escalation — coasts on T0 longer |
| **Low confidence + high arousal** (anxious, uncertain) | Lower threshold — escalates to T2 more readily |
| **High dominance** (assertive) | More willing to act on T1 analysis without T2 confirmation |
| **Low dominance** (cautious) | Requires T2 confirmation for any significant action |

This creates agents with distinct "personalities" without any explicit personality
programming — the behavior emerges from the interaction between the Daimon's state, the
prediction-error stream, and the routing logic.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Kahneman 2011, Thinking, Fast and Slow | Dual-process theory: System 1 (fast, automatic) / System 2 (slow, deliberate). |
| Sun 2002, Duality of the Mind, Erlbaum | CLARION: dual-level cognitive architecture (explicit + implicit). |
| Friston 2010, Nature Reviews Neuroscience 11(2) | Free Energy Principle: prediction error drives attention and action; Roko implements that loop through Bus topics and calibration updates. |
| Chen et al. 2023 (arXiv:2305.05176) | FrugalGPT: cascade routing for cost-efficient LLM use. |
| Sumers et al. 2023 (arXiv:2309.02427) | CoALA: cognitive architecture framework for language agents. |
| Anderson 1983, The Architecture of Cognition | ACT-R: production system cognitive architecture. |
| Laird et al. 1987, Artificial Intelligence 33(1) | SOAR: universal subgoaling architecture. |
| Baars 1988, A Cognitive Theory of Consciousness | Global Workspace Theory: limited-capacity conscious workspace. |
| Yao et al. 2022 (arXiv:2210.03629) | ReAct: interleaved reasoning and acting for language agents. |
| Shinn et al. 2023 (arXiv:2303.11366) | Reflexion: self-reflection for autonomous agents. |
| Zhao et al. 2023 (arXiv:2308.10144) | ExpeL: learning from autonomous agent experience. |
| Wang et al. 2023 (arXiv:2305.16291) | Voyager: open-ended embodied agent with skill library. |

---

## Current Status and Gaps

- **Implemented**: `InferenceTier` enum (T0/T1/T2) in the legacy crate path `bardo-primitives`. Operating
  frequency → inference tier mapping. CascadeRouter with confidence-based cascade.
- **Wired**: Tier routing in the orchestrator via CascadeRouter.
- **Target-state**: `prediction.*`, `outcome.*`, and `prediction.error.*` Pulses form the calibration surface for active inference.
- **Gap**: The 16 T0 probes are specified but not all implemented as a unified probe system.
- **Gap**: Exact EFE is still approximated via confidence/novelty/arousal, but the prediction/outcome loop is the implementation path.

---

## Cross-References

- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) — The loop where tier routing happens
- [10-three-cognitive-speeds.md](10-three-cognitive-speeds.md) — Gamma/Theta/Delta frequencies
- [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md) — Daimon PAD drives escalation
- [01-naming-and-glossary.md](01-naming-and-glossary.md) — Canonical two-medium / two-fabric vocabulary
- [tmp/refinements/10-self-learning-cybernetic-loops.md](../../tmp/refinements/10-self-learning-cybernetic-loops.md) — Predict/publish/correct/update loop and per-operator calibration
- [17-design-principles-and-frontier-summary.md](17-design-principles-and-frontier-summary.md) — T0 probes as innovation #1
