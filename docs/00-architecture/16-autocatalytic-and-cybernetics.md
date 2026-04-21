# Autocatalytic Improvement and Cybernetics

> **Abstract:** Roko is designed so that performance, capability, and quality improve
> superlinearly with accumulated usage, deployment count, and connected data. This is not a
> claim of literal exponential growth under every workload; it is a claim that the system
> compounds. The seven load-bearing loops below make each unit of use cheaper, faster, and
> better than the last.
>
> See also [tmp/refinements/15-exponential-scaling.md](../../tmp/refinements/15-exponential-scaling.md)
> for the source proposal, and [01-naming-and-glossary](./01-naming-and-glossary.md) for the
> canonical two-medium / two-fabric vocabulary.

> **Implementation**: Shipping

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [00-vision-and-thesis](./00-vision-and-thesis.md), [12-five-layer-taxonomy](./12-five-layer-taxonomy.md), [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md)
**Key sources**:
- `/Users/will/dev/nunchi/roko/refactoring-prd/00-overview.md` — Autocatalytic improvement section
- `/Users/will/dev/nunchi/roko/refactoring-prd/09-innovations.md` — integration map for compounding feedback
- `/Users/will/dev/nunchi/roko/roko/docs/00-architecture/13-cognitive-cross-cuts.md` — cross-cut interaction model

---

## Abstract

The architecture story here is simple: Roko gets better by using itself. The mechanism is
not one loop, but seven compounding loops tied together by two mediums, two fabrics, and the
seven-step loop.

- Two mediums: `Engram` for durable content-addressed records, `Pulse` for ephemeral
  topic-addressed transport.
- Two fabrics: `Substrate` for storage, `Bus` for transport.
- Seven-step loop: `SENSE`, `ASSESS`, `COMPOSE`, `ACT`, `VERIFY`, `PERSIST` /
  `BROADCAST`, `REACT`.

That framing matters because the system is not optimized for isolated feature wins. It is
optimized for positive feedback. Each loop below compounds the others: demurrage sharpens
retrieval, heuristics sharpen verification, HDC sharpens cleanup, c-factor sharpens routing,
playbooks sharpen compression, commons sharpen deployment bootstrap, and plugins sharpen
ecosystem growth.

The theory is cybernetic, but the product claim is practical: every unit of usage should make
the next unit cheaper, faster, and better.

---

## 1. Why compounding is the point

Most agent systems drift toward diminishing returns:

- More agents add coordination overhead.
- More memory adds retrieval noise.
- More tools add context pressure.
- More deployments create support burden without reuse.

Roko is built to invert those curves. The point is not that every part improves in isolation;
it is that the parts are coupled so the improvement feedback keeps feeding back into the next
turn, the next session, and the next deployment.

This is the meaning of the `superlinear` / `compounding` / `exponential` framing in REF15:
not a promise of infinite growth, but a claim that the architecture has enough feedback
structure to produce increasing returns over real workloads.

---

## 2. The seven compounding loops

Each loop is a distinct source of positive feedback. Each one uses a different combination of
the two fabrics, the seven-step loop, and the learning primitives from adjacent chapters.

| Loop | Core mechanism | Why it compounds | Main hooks |
|---|---|---|---|
| Demurrage-weighted retrieval | Idle memory is taxed; useful memory is reinforced | More usage tunes the holding cost and makes effective memory denser | `SENSE`, `ASSESS`, `PERSIST`, `REACT` |
| Heuristic calibration | Heuristics are tested against falsifiers and outcomes | Better calibration improves downstream decisions, which produces better future evidence | `VERIFY`, `REACT` |
| HDC codebook cleanup | HDC fingerprints snap noisy inputs to nearby stable codes | More episodes improve cleanup quality up to the large capacity of the codebook | `COMPOSE`, `PERSIST` |
| c-factor feedback | Cohort quality is measured continuously from Bus statistics | Better teams produce better outputs, which produce better calibration and routing | `ASSESS`, `VERIFY`, `REACT` |
| Playbook distillation | Episodes compress into reusable playbooks and meta-playbooks | Compression becomes cheaper and more transferable as the corpus grows | `PERSIST`, `REACT`, `Delta` |
| Cross-deployment heuristic commons | Heuristics are shared across deployments | Each deployment contributes once but benefits many times, so total value grows faster than linearly | `BROADCAST`, `REACT` |
| Plugin ecosystem | Plugins create a two-sided market for capability and users | More plugins attract users; more users justify more plugins | `ACT`, `BROADCAST`, `REACT` |

### 2.1 Demurrage-weighted retrieval

Demurrage gives memory a cost for sitting idle. That changes retrieval from "store everything"
to "store what continues to earn its keep." The result is a self-trimming substrate: useful
Engrams retain balance, weak ones fade toward cold tier, and the retrieval surface stays
indexed toward what has actually been used.

The compounding effect is subtle but real. More usage produces more reinforcement evidence,
which improves the demurrage curve, which makes the next retrieval pass more selective, which
improves the quality of the next episode. The KPI to watch is the balance histogram and the
median tokens per task.

### 2.2 Heuristic calibration

Heuristics only compound if they can be falsified. A heuristic that never sees a counterexample
does not get better; it just gets older. The Bus turns prediction and outcome into a continuous
calibration stream, so confidence intervals shrink as evidence accumulates.

The positive feedback is direct:

1. A heuristic predicts.
2. A Pulse or gate verdict contradicts or confirms it.
3. The calibration policy updates confidence.
4. The next decision is better.
5. The better decision produces better evidence.

That loop depends on `Heuristic`, `Falsifier`, `Pulse`, and `Bus` together. It is one of the
main reasons the architecture can improve without needing a full manual rewrite of the policy
stack after every new domain.

### 2.3 HDC codebook cleanup

HDC fingerprints turn similarity into a cheap cleanup operation. Every new episode, gate
result, and heuristic adds to the codebook, which makes future retrieval more likely to land
on a stable semantic neighborhood instead of a noisy miss.

This loop compounds because the codebook is not just bigger; it is better organized by use.
The more real interactions the system sees, the more likely a future query will collapse to
the right Engram cluster on the first pass. The KPI here is the percentage of Composer prompts
that hit HDC-clean cache on the first attempt.

### 2.4 c-factor feedback

c-factor measures how well a cohort cooperates, predicts, and routes under real load. High
c-factor teams produce higher-quality output, which in turn produces better learning evidence
for routing, demurrage tuning, and heuristic calibration.

This is a three-loop reinforcement:

- c-factor rises.
- Output quality rises.
- Learning quality rises.
- c-factor rises again.

The important constraint is that c-factor is a covariate, not the objective. It is a measured
property that helps explain system quality, not a target to game. The KPI here is c-factor on
randomly sampled cohorts; the failure mode is reward hacking through cherry-picked easy work.

### 2.5 Playbook distillation

Episodes are compressed into playbooks, and playbooks can be compressed into meta-playbooks.
That is learning about learning. Once the corpus is large enough, the cost per distilled unit
drops while the transfer value per unit rises.

The compounding mechanism is that the system no longer relearns the same structure from
scratch. It learns the reusable shape of the work. That makes later distillation cheaper,
which produces more playbooks, which increases reuse again. The KPI is retroactive
improvements per week from the Delta consolidation cycle.

### 2.6 Cross-deployment heuristic commons

Once heuristics can be imported across deployments, each deployment contributes to a shared
commons. The economics are simple: the marginal cost of sharing is low, but the marginal value
to other deployments is high. That is how the system becomes more valuable as the install base
grows.

This is the loop behind the product claim that the first task on a fresh deployment should get
faster as the commons grows. The KPI is first-task-after-install to success minutes. The
anti-metric is a commons that grows but does not reduce time-to-first-success.

### 2.7 Plugin ecosystem

Plugins make capability portable. Each new plugin increases the value of Roko to users who need
that capability, and each new user increases the value of building a plugin. That two-sided
market is the strongest externally visible compounding loop in the system.

The quality of the plugin interface determines how much of that network effect survives contact
with real users. Good SPI design preserves the compounding; bad SPI design leaks complexity back
to the user and caps the return. The KPI is unique plugin count and unique plugin users.

---

## 3. How the loops fit the seven-step loop

The seven compounding loops ride on the seven-step loop, with `PERSIST` and `BROADCAST`
treated as co-equal branches inside the same phase.

```text
1. SENSE      - Substrate.query | Bus.subscribe | external I/O
2. ASSESS     - Scorer + Router choose what to do next
3. COMPOSE    - Composer assembles a prompt Engram under budget
4. ACT        - LLM | tool | chain execution emits Pulses and final Engrams
5. VERIFY     - Gate pipeline and stream-gates emit verdicts
6. PERSIST    - Substrate.put for Engrams
   BROADCAST  - Bus.publish for Pulses, in parallel
7. REACT      - Policy updates, new Pulses, new Engrams, new calibration
```

The loop compounds because each step improves the next one:

- `SENSE` gets better when Substrate queries hit HDC-clean memory and Bus subscriptions expose
  more relevant Pulses.
- `ASSESS` gets better when c-factor and demurrage calibrations sharpen the scoring surface.
- `COMPOSE` gets better when playbooks and HDC fingerprints compress the prompt space.
- `ACT` gets better when the plugin ecosystem and domain profiles make the action space richer.
- `VERIFY` gets better when heuristics and falsifiers are tested continuously rather than only
  after failures.
- `PERSIST` gets better when demurrage trims dead weight and preserves useful lineage.
- `BROADCAST` gets better when shared heuristics and Pulse streams make the commons richer.
- `REACT` gets better when the system learns which loops are actually paying back.

That is the two-fabric framing in operational form: Substrate stores durable improvement,
Bus transports ephemeral coordination, and the operators turn one into the other.

---

## 4. Cybernetic foundations

The theory still matters, but it now serves the compounding story rather than overshadowing it.

### 4.1 Ashby and requisite variety

Ashby's Law still applies: only variety can absorb variety. Roko's answer is not to invent a
new abstraction for every situation. It is to keep the kernel vocabulary stable while allowing
the implementations, topics, heuristics, and plugins to expand.

### 4.2 Conant-Ashby and self-models

The Good Regulator Theorem still justifies self-modeling. That is why the compounding story
includes c-factor, heuristic calibration, and retrospection rather than just raw throughput.
The system needs a model of its own learning dynamics to regulate them well.

### 4.3 Beer and recursive viability

Beer remains useful as an organizing analogy: the layers, controls, and recursive feedback
structures explain why the same loop can run at Gamma, Theta, and Delta without changing the
core architecture.

### 4.4 Bus-centered active inference

The Bus is the feedback nervous system. Prediction Pulses, outcome Pulses, prediction-error
Pulses, and calibration Pulses form the operational bridge between cybernetics and the seven
compounding loops. In practice, that means the system can learn from mismatch, not just from
success.

---

## 5. Measuring compounding

The architecture only earns the superlinear claim if we can see the curves. The most important
measurements are the ones that show whether each loop is actually closing.

### 5.1 KPI panel

| KPI | Loop it measures | Expected curve |
|---|---|---|
| Mean time to first successful PR on a new codebase | All seven loops | Steep initial drop, then continued decline |
| Median tokens per task, by difficulty bucket | Demurrage, HDC, playbooks | Monotonic decrease |
| % of Composer prompts hitting HDC-clean cache | HDC cleanup | Asymptote toward 1 |
| Mean calibration CI width per heuristic | Heuristic calibration | Decrease with trials |
| % of heuristics sourced from commons | Cross-deployment commons | Increase, then stabilize |
| c-factor on randomly sampled cohorts | c-factor feedback | Stable or rising |
| Dream-cycle retroactive improvements per week | Playbook distillation | Growth with corpus size |
| Unique plugin count | Plugin ecosystem | Linear in time, but value superlinear |
| Unique plugin users | Plugin ecosystem | Superlinear once the flywheel turns |
| First-task-after-install to success minutes | Cross-deployment commons | Decreases as commons grows |

### 5.2 Anti-metrics

Three numbers should stay flat or shrink as usage grows:

- Warm-tier episode count should stabilize, not grow without bound, if demurrage is working.
- Heuristic count with fewer than three confirmations should not grow indefinitely, or the
  calibrator is not probing enough.
- Mean lineage depth per response should not drift upward unless the extra lineage is actually
  improving answer quality.

If any of those blow out, the system is accumulating complexity without compounding value.

### 5.3 Failure modes

| Failure mode | What it looks like | Countermeasure |
|---|---|---|
| Echo chamber | The same beliefs keep winning because nothing challenges them | Outsider injection, challenger worldviews, explicit falsifiers |
| Reward hacking | c-factor rises because the policy routes only easy work | c-factor stays a covariate, not the objective; sample by difficulty |
| Premature convergence | A heuristic stops being tested because it already looks confident | Importance sampling and deliberate boundary tests |
| Substrate bloat | Warm storage grows without restraint | Demurrage tuning, cold tier promotion, balance histograms |

### 5.4 Kill switches

The operator needs reversibility when compounding goes wrong. The canonical emergency actions
are:

- `roko attention reset`
- `roko heuristic retire --confidence-below 0.3`
- `roko substrate freeze --older-than 90d`
- `roko commons opt-out`
- `roko experiments pause`

These are guardrails, not goals. They exist so the compounding loops can be tuned safely.

---

## 6. Why this is autocatalytic

Autocatalysis is the right metaphor because the outputs of one part become the inputs that
improve another part, which in turn improves the first part. The system does not need a
central planner to make that happen. It needs:

- a durable store that preserves useful structure,
- an ephemeral bus that moves coordination feedback quickly,
- calibration loops that can learn from outcomes,
- compression loops that turn episodes into reusable shape,
- and a measurement story that shows whether the positive feedback is real.

That is the load-bearing distinction between a feature set and a compounding system.

---

## 7. Theoretical limits

The compounding story still lives inside physical and computational limits.

### 7.1 Bottlenecks

- LLM throughput, not abstract compute, is the practical ceiling for many turns.
- Context windows cap how much can be composed at once.
- Cost is the first-order constraint on how often the system can use deep reasoning.

### 7.2 Undecidability

Not every property can be decided automatically. Some verification still requires escalation
to deeper reasoning or human review. That is why the system measures failure modes instead of
pretending they can all be eliminated.

### 7.3 No free lunch

No single optimization strategy dominates all problems. The compounding architecture is
therefore plural: multiple loops, multiple feedback channels, multiple surfaces, one shared vocabulary.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Kauffman 1993, "The Origins of Order" | Autocatalytic sets and self-sustaining reaction networks |
| Ashby 1956, "An Introduction to Cybernetics" | Law of Requisite Variety |
| Conant & Ashby 1970 | Good Regulator Theorem |
| Beer 1972, "Brain of the Firm" | Viable System Model and recursive viability |
| Friston 2010 | Free Energy Principle and active inference |
| Grassé 1959 | Stigmergy |
| Dorigo et al. 2000 | Ant colony optimization as stigmergic coordination |
| Wolpert & Macready 1997 | No Free Lunch Theorem |
| Rice 1953 | Limits of general semantic decidability |

---

## 8. Connection to SOFAI and modern dual-process systems

The compounding loops are compatible with dual-process and metacognitive systems, but Roko
extends the familiar fast/slow split with a third consolidation speed.

| SOFAI / dual-process idea | Roko mapping | Why it matters for compounding |
|---|---|---|
| Fast reasoning | Gamma | Enables quick feedback and cheap corrections |
| Slow reasoning | Theta | Enables deliberate calibration and verification |
| Offline consolidation | Delta | Turns episodes into reusable playbooks and commons |

The Delta layer is what turns a good turn into a reusable future advantage. That is why the
superlinear claim depends on persistence across sessions, not just within-session benchmark
scores.

---

## Current Status and Gaps

- The seven compounding loops are specified and aligned with the two-fabric vocabulary.
- The superlinear claim is architectural, not yet a completed benchmark result across every
  workload class.
- The headline KPI should be mean time to first successful PR on a new codebase, measured over
  real sessions with persistent state.
- The anti-metrics matter as much as the KPIs; if they drift upward, the system is optimizing
  complexity instead of compounding value.
- The phase-2 commons and plugin effects depend on later Bus consumers and deployment surfaces,
  so this chapter should be read as the theory and measurement frame for those chapters.

---

## Cross-References

- See [00-vision-and-thesis](./00-vision-and-thesis.md) for the thesis that scaffolding determines performance
- See [01-naming-and-glossary](./01-naming-and-glossary.md) for the canonical two-medium / two-fabric vocabulary
- See [11-dual-process-and-active-inference](./11-dual-process-and-active-inference.md) for active inference and prediction-error loops
- See [12-five-layer-taxonomy](./12-five-layer-taxonomy.md) for the layered architecture that keeps the feedback loops ordered
- See [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) for the Neuro, Daimon, and Dreams cross-cuts
- See [14-c-factor-collective-intelligence](./14-c-factor-collective-intelligence.md) for the c-factor metric
- See [10-self-learning-cybernetic-loops](../../tmp/refinements/10-self-learning-cybernetic-loops.md) for the Bus-centered self-learning loop, prediction-error Pulses, and calibration policy
- See [tmp/refinements/15-exponential-scaling.md](../../tmp/refinements/15-exponential-scaling.md) for the full exponential-scaling refinement
- See topic [13-coordination](../13-coordination/INDEX.md) for stigmergic multi-agent coordination
