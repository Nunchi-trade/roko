# ADAS and Autocatalytic Thesis

> **PRD sources:** `refactoring-prd/09-innovations.md` §X–XI, `refactoring-prd/00-overview.md` (Autocatalytic Improvement)
> **Academic basis:** Hu et al. ICLR 2025 (ADAS); Kauffman 1993 (autocatalytic sets); Chen et al. 2023 (EvoSkills); Loreto & Tria 2014 (Pólya urn); Reed's Law; Metcalfe's Law
> **Legacy sources:** `agent-chain/09-exponential-flywheels.md`, `agent-chain/self-improvement-frameworks.md`
> **Cross-references:** [02-skill-library-voyager](02-skill-library-voyager.md), [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md), [15-collective-calibration-31x](15-collective-calibration-31x.md)

---

## Purpose

This document describes two frontier concepts that together form Roko's long-term thesis on compound self-improvement:

1. **ADAS (Automated Design of Agentic Systems)** — a meta-agent that searches the space of possible agent architectures, discovering new components and configurations that improve performance.
2. **Autocatalytic growth** — the theoretical framework for why a system with interconnected feedback loops can achieve super-linear improvement over time.

Both are speculative — ADAS is planned but not implemented, and the autocatalytic thesis is a design aspiration backed by theoretical models and early empirical evidence, not a proof. This document presents them with appropriate epistemic caveats.

---

## ADAS: Automated Design of Agentic Systems

### Background

Hu et al. (ICLR 2025) introduced ADAS: a meta-agent that searches the space of agentic system designs by generating, evaluating, and iterating on agent architectures in code.

**Key results:**
- +14% accuracy on ARC (Abstraction and Reasoning Corpus)
- +13.6 F1 improvement on reading comprehension tasks
- Discovered novel agent architectures that outperformed expert-designed baselines

### How ADAS Works

```
Meta-Agent (ADAS)
    │
    ├── 1. Define search space:
    │       - Agent roles (how many, what capabilities)
    │       - Communication patterns (sequential, parallel, hierarchical)
    │       - Tool configurations (which tools per role)
    │       - Prompt templates (structure, content, ordering)
    │       - Routing strategies (model selection rules)
    │
    ├── 2. Generate candidate architecture (code):
    │       "Create a 3-agent pipeline:
    │        Planner (opus) → Implementer (sonnet) → Reviewer (haiku)
    │        with shared memory via episodes"
    │
    ├── 3. Evaluate on benchmark tasks:
    │       Run the candidate architecture on held-out tasks
    │       Measure: pass rate, cost, latency, iterations
    │
    ├── 4. Select and iterate:
    │       Keep architectures that improve over baseline
    │       Mutate: change roles, models, communication patterns
    │       Recombine: merge best features of top architectures
    │
    └── 5. Deploy winner:
          Update the production configuration
```

### Roko's ADAS Pathway

Roko's architecture is designed to support ADAS-style meta-optimization:

| ADAS Requirement | Roko Component |
|-----------------|----------------|
| Architecture representation in code | `roko.toml` configuration + `SystemPromptBuilder` templates |
| Evaluation harness | Gate pipeline (11 gates, deterministic verification) |
| Performance metrics | C-Factor, task metrics, regression detection |
| Experiment framework | `ExperimentStore` for A/B testing configurations |
| Search strategy | Cascade router bandits (can be extended to architecture search) |

The key insight is that Roko already has all the components needed for ADAS — it just needs a meta-level agent that operates on configurations rather than on code. Where a normal agent modifies `src/*.rs`, the ADAS meta-agent modifies `roko.toml`, prompt templates, and routing rules, then evaluates the results through the same gate pipeline.

### Planned ADAS Capabilities

1. **Prompt template search** — generate variant prompt templates, evaluate via gate pass rate, converge on best performers. (Partially implemented via `ExperimentStore`.)
2. **Model routing search** — test different model allocations per role, find cost-optimal configurations. (Partially implemented via cascade router.)
3. **Gate configuration search** — adjust gate thresholds and rung order, optimize for development velocity vs. quality. (Partially implemented via adaptive thresholds.)
4. **Agent topology search** — test different numbers of agents, role assignments, and communication patterns. (Not implemented — requires multi-agent orchestration.)

---

## EvoSkills: Evolutionary Skill Optimization

Chen et al. (2023) introduced EvoSkills: an evolutionary approach to skill optimization where skills are treated as a population that undergoes selection, crossover, and mutation.

### Connection to Roko

The skill library (see [02-skill-library-voyager](02-skill-library-voyager.md)) accumulates skills from successful episodes. EvoSkills extends this with evolutionary operators:

1. **Selection** — skills with high success rates are selected for reproduction.
2. **Crossover** — combine steps from two successful skills for related tasks.
3. **Mutation** — vary skill parameters (tool choices, step ordering) to explore alternatives.
4. **Fitness evaluation** — gate pass rate serves as the fitness function.

This creates a population of skills that evolves toward higher fitness, complementing the Voyager-style monotonic accumulation with active optimization of existing skills.

**Status:** Not implemented. The current skill library only accumulates and tracks, it does not evolve skills. EvoSkills is a Tier 3 innovation in the priority roadmap.

---

## Autocatalytic Thesis

### Definition

An autocatalytic set (Kauffman 1993) is a collection of entities where each entity's production is catalyzed by other entities in the set. Once the set reaches a critical diversity threshold, it becomes self-sustaining: the creation of new entities accelerates the creation of further entities, producing exponential growth.

### Application to Roko

Roko's learning subsystems form an autocatalytic set:

```
Skills catalyze → better prompts
Better prompts catalyze → higher pass rates
Higher pass rates catalyze → more successful episodes
More episodes catalyze → better pattern extraction
Better patterns catalyze → better playbook rules
Better rules catalyze → fewer failures
Fewer failures catalyze → lower costs
Lower costs catalyze → more experiments
More experiments catalyze → better skills
    ↑                              │
    └──────────────────────────────┘
         (autocatalytic cycle)
```

Each element in the cycle enables the next. The cycle is autocatalytic because it is self-reinforcing: once started, it accelerates without external input.

### Compound Improvement Math

The PRD models compound improvement as:

```
compound_success = pass_rate_routing × pass_rate_prompts × pass_rate_skills × pass_rate_rules
```

If each component has an independent 90% pass rate:

```
compound = 0.9 × 0.9 × 0.9 × 0.9 = 0.656
```

This means the system succeeds 65.6% of the time when all four components must succeed. The key insight is that **small improvements in any component multiply through the chain**:

| Improvement | New compound | Absolute gain |
|------------|-------------|---------------|
| Routing 90% → 95% | 0.95 × 0.9³ = 0.692 | +3.6% |
| All 90% → 92% | 0.92⁴ = 0.716 | +6.0% |
| All 90% → 95% | 0.95⁴ = 0.815 | +15.9% |

The multiplicative structure means that a small uniform improvement (90% → 95%) produces a larger compound improvement (65.6% → 81.5%) than any single large improvement.

### Caveats

1. **Independence assumption**: The components are not independent. Better routing may make prompt optimization less impactful (because the model is already well-chosen). The multiplicative model overestimates compound improvement when components are correlated.

2. **Diminishing returns**: Each component has a ceiling (can't exceed 100%). As components approach their ceilings, further improvement becomes harder, and the compound effect plateaus.

3. **Stability constraint**: Compound improvement only occurs when the system is stable (see [14-stability-mechanisms](14-stability-mechanisms.md)). Oscillation between components can produce compound degradation instead of compound improvement.

4. **Minimum viable diversity**: The autocatalytic cycle requires all components to function. A missing component (e.g., no skill library) breaks the cycle. This is why Tier 1M (the eight missing feedback loops) is prioritized: closing the loops enables the autocatalytic cycle to function.

---

## Network Effects

The autocatalytic thesis invokes two network scaling laws:

### Metcalfe's Law

The value of a network is proportional to N² (the number of possible connections between N nodes). In Roko's context, N is the number of learning subsystems. With 8 feedback loops connecting 10+ subsystems, the potential interaction space is O(N²) ≈ 100 interactions, each potentially creating an improvement pathway.

### Reed's Law

The value of a network is proportional to 2^N (the number of possible subsets). This applies when groups of subsystems can form emergent coalitions: the cascade router + provider health + cost normalization form a "routing coalition" that is more than the sum of its parts.

### Loreto & Tria Pólya Urn Model (2014)

The Pólya urn model for innovation predicts that the rate of discovery accelerates as the knowledge base grows: each new discovery opens adjacent possibilities that increase the probability of further discoveries. Applied to Roko: each new skill, pattern, or routing rule opens new optimization pathways that weren't previously visible.

---

## Flywheel Mechanisms

Ten mechanisms for compounding growth, adapted from the legacy architecture:

| # | Mechanism | Source | How it compounds |
|---|-----------|--------|-----------------|
| 1 | Skill accumulation | Voyager (Wang et al. 2023) | More skills → cheaper future tasks |
| 2 | Pattern extraction | Trigram mining | More patterns → fewer repeated mistakes |
| 3 | Playbook rules | Reflexion/ExpeL | More rules → higher first-attempt pass rate |
| 4 | Model routing | RouteLLM/FrugalGPT | Better routing → lower cost per task |
| 5 | Cache optimization | KV cache affinity | More reuse → lower marginal cost |
| 6 | Prompt optimization | DSPy/experiments | Better prompts → fewer iterations |
| 7 | Calibration | Predictive foraging | Better predictions → better decisions |
| 8 | Crate familiarity | LinUCB context | More experience → better model selection per crate |
| 9 | Cross-project transfer | HDC fingerprints | Skills from project A accelerate project B |
| 10 | Meta-optimization | ADAS (Hu et al. 2025) | Better architecture → better everything |

Each mechanism independently produces linear improvement. When they interact through feedback loops, the compound effect can be super-linear — but only if stability mechanisms prevent oscillation and the autocatalytic cycle is complete.

---

## Empirical Validation

The autocatalytic thesis is testable. The C-Factor (see [15-collective-calibration-31x](15-collective-calibration-31x.md)) should show:

1. **Initial plateau** (first 50 episodes): Learning subsystems bootstrapping, C-Factor near 0.5.
2. **Acceleration** (50-200 episodes): Feedback loops engaging, C-Factor rising.
3. **Super-linear growth** (200-500 episodes): Autocatalytic cycle active, C-Factor rising faster than linear.
4. **Saturation** (500+ episodes): Components approaching ceilings, growth rate decreasing.

If the C-Factor shows a linear or sub-linear trend instead of super-linear, the autocatalytic thesis is falsified for the current implementation. This falsifiability is essential: the thesis is a scientific hypothesis, not a marketing claim.

---

## Relationship to Other Documents

- **[02-skill-library-voyager](02-skill-library-voyager.md)** — Monotonic skill accumulation is mechanism #1.
- **[04-cascade-router](04-cascade-router.md)** — Model routing optimization is mechanism #4.
- **[05-pattern-discovery-trigram](05-pattern-discovery-trigram.md)** — Pattern extraction is mechanism #2.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — The eight loops are the connections that enable the autocatalytic cycle.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Stability is a prerequisite for compound improvement.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — C-Factor measures whether compound improvement is occurring.
- **[16-predictive-foraging](16-predictive-foraging.md)** — Calibration is mechanism #7.
- **[12-self-improvement-frameworks](12-self-improvement-frameworks.md)** — Academic foundations for all mechanisms.

---

## Appendix: Critical Diversity Threshold

Kauffman's autocatalytic set theory predicts a critical diversity threshold: below a certain number of interacting components, the autocatalytic cycle cannot sustain itself. Above the threshold, the cycle becomes self-sustaining and accelerates.

For Roko, the critical components are:

| Component | Status | Role in Cycle |
|-----------|--------|--------------|
| Episode logger | Wired | Data substrate |
| Pattern miner | Wired | Knowledge extraction |
| Playbook rules | Wired | Knowledge validation |
| Skill library | Wired | Capability accumulation |
| Cascade router | Wired | Resource optimization |
| Provider health | Wired | Reliability |
| Cost normalization | Wired | Budget management |
| Regression detection | Wired | Quality assurance |
| Prompt experiments | Wired | Prompt optimization |
| C-Factor | Wired | System measurement |

All 10 components are wired. The remaining question is whether the eight inter-component feedback loops are sufficiently connected to sustain the autocatalytic cycle. Currently, 1 of 8 loops is fully wired, 3 are partially wired, and 4 are data-collection-only. The thesis predicts that closing the remaining loops will produce a phase transition in the C-Factor trend — from linear improvement to super-linear growth.

### Falsification Criteria

The autocatalytic thesis is falsified if:
1. C-Factor shows no upward trend after 500 episodes with all 8 loops wired.
2. Individual component improvements do not compound (each improvement is additive rather than multiplicative).
3. Closing additional feedback loops does not produce measurable C-Factor acceleration.

These criteria provide concrete conditions under which the thesis should be abandoned in favor of simpler linear improvement models.
