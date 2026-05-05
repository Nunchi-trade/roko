# Exponential Flywheel: Mechanisms for Superlinear Growth

> **Layer**: L4 Orchestration (coordination dynamics), with cross-cuts into L0-L3
>
> **Scope**: Superlinear compounding in collective intelligence, deployment learning, and ecosystem growth
>
> **Prerequisites**: `00-stigmergy-theory.md`, `03-digital-pheromones.md`, `09-stigmergy-scaling.md`, `11-collective-intelligence-metrics.md`
>
> **Terminology**: Use the authoritative naming map in [Naming Map and Glossary](../00-architecture/01-naming-and-glossary.md)
>
> **See also**: [tmp/refinements/15-exponential-scaling.md](../../tmp/refinements/15-exponential-scaling.md)
>
> **Implementation**: Specified

---

## Overview

This doc is the coordination-side story for REF15. The claim is simple: Roko should improve
**superlinearly** with accumulated usage, deployment count, and connected data. The result is
an **exponential** flywheel only when the feedback loops are wired deliberately; otherwise the
system falls back to linear growth, local optimization, or outright stagnation.

The seven loops below are the compounding mechanisms most relevant to coordination. They are
not independent; they reinforce one another. A better commons improves c-factor. Better
c-factor improves heuristics. Better heuristics improve retrieval, distillation, and plugin
adoption. That is the actual flywheel.

## The Seven Loops

| Loop | What compounds | Coordination effect |
|---|---|---|
| Demurrage-weighted retrieval | Usage calibrates attention cost and reward | The system keeps what is unique and useful instead of letting memory bloat dominate retrieval quality. |
| Heuristic calibration | More trials tighten uncertainty and improve downstream decisions | Each episode becomes a better test of the worldview and a better input to policy. |
| HDC codebook cleanup | More exemplars improve similarity cleanup and consensus hit-rate | Retrieval, clustering, and analogy get cleaner as the codebook grows, up to saturation. |
| c-factor feedback | Better cohort process improves output quality and learning quality | Cohort dynamics become a measured input to policy rather than an informal hope. |
| Playbook distillation | Episodes compress into reusable playbooks and meta-playbooks | The system learns how to learn, which lowers the cost of each later improvement. |
| Cross-deployment heuristic commons | Imported heuristics create shared calibration across deployments | Every deployment contributes to a shared rule base that benefits other deployments at near-zero marginal cost. |
| Plugin ecosystem | Each plugin increases the value of the system to users and builders | The interface becomes a platform if it stays narrow, stable, and easy to compose against. |

### 1. Demurrage-Weighted Retrieval

Naive memory grows without bound and retrieval quality degrades. Demurrage changes the slope:
usage earns reinforcement, while idle content pays a holding cost. That keeps the working set
small enough to stay relevant and large enough to preserve the rare, load-bearing pieces.

The compounding effect is operational, not mystical: better usage traces improve calibration of
holding costs, which improves retrieval quality, which improves the next round of usage.

Failure signal: warm-tier content grows without a steady state, or retrieval quality stops
improving even as trials accumulate.

### 2. Heuristic Calibration

Every episode is a trial for multiple heuristics. As trial counts rise, confidence intervals
tighten and the system learns which heuristics are reliable, narrow, or wrong. A well-calibrated
heuristic is valuable because it changes downstream decisions in proportion to its confidence.

This is one of the most important compounding loops because it turns experience into better
priors, not just more logs.

Failure signal: premature convergence. A heuristic that reaches high confidence early and then
avoids refutation will look stable while becoming less useful.

### 3. HDC Codebook Cleanup

HDC similarity makes retrieval and cleanup cheaper as the codebook gets richer. More episodes,
gate results, and heuristics create more anchors for cleanup, which raises the chance that a
noisy query lands on the right cluster.

The practical result is cleaner reuse: prompts, retrievals, and decisions become less token-
heavy because the right exemplar is easier to find.

Failure signal: codebook pollution. If the cleanup space fills with noisy, redundant, or stale
entries, similarity becomes less discriminative and the flywheel slows.

### 4. c-factor Feedback

c-factor is the cohort-process signal that tells us whether the collective is actually
functioning well. High c-factor should correlate with better turn-taking, better peer prediction,
better citation reciprocity, better delivery rate, and better HDC diversity.

The important distinction is causal discipline: **c-factor is a covariate, not the objective**.
The objective is task quality on work sampled by difficulty. c-factor is a measured property
that should move with better coordination, not replace it.

Failure signal: the system learns to optimize for easy tasks, flattering cohorts, or metric
gaming instead of hard-work quality.

### 5. Playbook Distillation

Episodes should not stay as raw episodes forever. They should condense into playbooks, then
meta-playbooks, so later work starts from a better compressed prior. This is where the system
learns to reuse its own past.

The compounding effect comes from transferability. A well-distilled playbook is cheaper to
apply than re-deriving the same lesson from scratch, and a meta-playbook is cheaper still.

Failure signal: overcompression. If a playbook loses the context that made it valid, reuse will
look efficient while silently degrading decision quality.

### 6. Cross-Deployment Heuristic Commons

When heuristics can be shared across deployments, each deployment contributes to a commons that
other deployments can reuse. The local cost of importing a useful heuristic is small; the system
value is large because one good heuristic can help many deployments.

This is the coordination version of a network effect. It only works if the commons stays
curated, versioned, and revalidated in context.

Failure signal: stale imports. A shared heuristic that no longer matches the local deployment
becomes a drag instead of an asset.

### 7. Plugin Ecosystem

Plugins create the classic two-sided flywheel. Each plugin increases user value for a capability
that was previously missing, and each new user increases the incentive to build more plugins.

The flywheel depends on the interface. If the plugin surface is stable, typed, and narrow, the
ecosystem compounds. If the interface leaks complexity, the network effect weakens and support
costs rise faster than adoption.

Failure signal: integration friction. When every plugin needs bespoke scaffolding, the platform
effect collapses into one-off integrations.

## Phase 2 Amplifiers

The current seven loops already compound. Phase 2 adds amplifiers that make the coordination
story broader and more durable.

- **Dream-consolidation compression**: offline consolidation re-reads prior episodes during idle
  time and re-distills them with current priors. Old episodes can produce new heuristics when
  viewed through a better model.
- **Agent-chain specialization**: roles specialize through chains of interaction, which turns a
  generalist fleet into a more structured functional ecosystem.
- **Witness-signed heuristic commons**: when shared heuristics carry stronger provenance, trust
  compounds across deployments instead of resetting at each boundary.

These amplifiers depend on the same two-fabric foundation described elsewhere in the docs tree
and on the current loop set staying healthy.

## Measurement

The flywheel is only real if it is measured on persistent workloads. Stateless benchmarks hide
compounding because they reset the substrate between trials.

### North-Star Metric

**Mean time to first successful PR on a new codebase**.

That metric depends on all seven loops: better priors, better retrieval, better coordination,
better compression, better tooling, and better self-awareness. It should drop steeply early on
and keep dropping at a decreasing-but-positive rate.

### Secondary KPIs

| KPI | Loop | Expected curve |
|---|---|---|
| Median tokens per task by difficulty bucket | Demurrage + HDC + playbook distillation | Monotonic decrease |
| % of Composer prompts hitting an HDC-clean cache | HDC codebook cleanup | Asymptote near 1 |
| Mean calibration CI width per heuristic | Heuristic calibration | Decrease with trial count |
| % of heuristics sourced from commons | Cross-deployment heuristic commons | Increase, then stabilize |
| c-factor on randomly sampled cohorts | c-factor feedback | Stable or rising |
| Dream-cycle retroactive improvements per week | Dream-consolidation compression | Grows with corpus |
| Plugin count and unique users | Plugin ecosystem | Plugin count linear, unique users superlinear once the flywheel turns |
| First-task-after-install success minutes | Heuristic commons bootstrap | Decreases as commons grows |

### Anti-Metrics

- Warm-tier episode count should reach a steady state instead of growing unbounded.
- Heuristics with fewer than three confirmations should shrink over time.
- Mean lineage depth per response should not increase unless quality improves with it.

If any of those rise without corresponding quality gains, the flywheel is being simulated
instead of earned.

## Failure Modes

- **Echo chambers**: positive feedback reinforces wrong beliefs. Counter by injecting outsiders,
  explicit falsifiers, and sampled disagreement.
- **Reward hacking**: optimizing c-factor directly can make the system prefer easy work. Keep the
  objective on task quality sampled by difficulty.
- **Premature convergence**: high-confidence heuristics stop facing meaningful challenge. Use
  importance sampling and deliberate boundary tests.
- **Substrate bloat**: retention without demurrage overwhelms the working set. Tune holding costs
  so the cold tier actually gets used.
- **Commons drift**: imported heuristics lose fit across deployments. Require local
  revalidation before adoption.
- **Benchmark illusion**: synthetic tests that reset state will miss compounding entirely. Measure
  across sessions with preserved substrate.
- **Plugin drag**: a large plugin surface with unstable contracts flattens the network effect.
  Keep the SPI narrow and predictable.

## Operating Rule

The flywheel is not a promise that growth will be exponential by default. It is a design goal
that becomes true only when each loop is instrumented, curbed against its failure modes, and
run on real persistent workloads. If a line flatlines, the feedback loop is broken somewhere.

## Cross-References

- [Naming Map and Glossary](../00-architecture/01-naming-and-glossary.md) - authoritative vocabulary for `Engram`, `Pulse`, `Bus`, `Topic`, `Datum`, and the current heuristic language
- [Collective Intelligence Metrics](./11-collective-intelligence-metrics.md) - c-factor instrumentation and cohort metrics
- [tmp/refinements/15-exponential-scaling.md](../../tmp/refinements/15-exponential-scaling.md) - canonical REF15 proposal
