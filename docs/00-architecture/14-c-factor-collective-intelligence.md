# C-Factor: Collective Intelligence

> **Abstract:** C-Factor is a two-level metric system that measures whether a group of Roko
> agents collectively outperforms the sum of its parts. Inspired by Woolley et al.'s
> discovery of a general collective intelligence factor in human groups (Science 330, 2010),
> Roko instruments both a high-level ratio (C-Factor) for reporting and a composite score
> (C-Score) for optimization. Four diagnostic signals — turn-taking equality, knowledge
> flow rate, cross-domain transfer, and emergent coordination — provide mechanistic
> visibility into why a collective succeeds or fails.

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [06-synapse-traits](./06-synapse-traits.md), [12-five-layer-taxonomy](./12-five-layer-taxonomy.md), [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md)
**Key sources**:
- `/Users/will/dev/nunchi/roko/refactoring-prd/00-overview.md` — C-Factor definition
- `/Users/will/dev/nunchi/roko/refactoring-prd/09-innovations.md` — Collective calibration (31.6×)
- `/Users/will/dev/nunchi/roko/roko/docs/00-architecture/13-cognitive-cross-cuts.md` — Neuro/Daimon/Dreams

---

## Abstract

When multiple agents collaborate on a shared task domain, a critical question arises: does
the collective outperform the sum of its individual members? This is not guaranteed — poorly
coordinated groups can perform *worse* than individuals due to communication overhead,
conflicting actions, and duplicated effort.

Woolley et al. (2010) demonstrated that human groups possess a general collective intelligence
factor (c) that is not simply the average or maximum intelligence of group members. Instead,
c is predicted by three factors: the equality of conversational turn-taking, social sensitivity
of group members, and the proportion of women in the group (as a proxy for social perceptiveness).
This finding implies that collective intelligence is a property of group *dynamics*, not just
individual capability.

Roko adapts this insight to agent collectives. C-Factor measures whether a group of agents
achieves superlinear intelligence — collective performance exceeding the sum of individual
performances. C-Score provides an actionable composite metric with individually improvable
components. Together, they give operators both a high-level health metric and a fine-grained
optimization target.

The theoretical foundation is extended by the 31.6× collective calibration heuristic, which
models how shared verified predictions accelerate individual agent learning. While the scaling
claim requires empirical validation (it is a Nunchi-derived heuristic, not a published theorem),
the underlying mechanism — aggregation of independently verified signals — is grounded in
classical statistics (estimation error scales as 1/sqrt(n) for independent observations).

---

## 1. The Research Foundation

### 1.1 Woolley et al.: Collective Intelligence Factor

Woolley et al. (2010) "Evidence for a Collective Intelligence Factor in the Performance of
Human Groups" (Science 330(6004), pp. 686-688) demonstrated that:

1. A single statistical factor (c) predicts group performance across a wide variety of tasks,
   analogous to the g factor for individual intelligence.
2. c is NOT strongly correlated with the average intelligence of group members.
3. c IS strongly correlated with:
   - **Turn-taking equality** — how evenly members contribute to discussion
   - **Social sensitivity** — how well members read each other's cues
   - **Group composition** — diversity of perspectives

The implication for agent systems: simply deploying more agents does not increase collective
intelligence. The *dynamics* of how agents share information, take turns, and respond to each
other's signals determine whether the collective is superlinear (c > sum), linear (c = sum),
or sublinear (c < sum).

### 1.2 Metcalfe's Law and Network Effects

Metcalfe's Law states that the value of a network grows proportional to the square of its
nodes: V ∝ N². For agent collectives, N agents produce O(N) insights, but total network
value grows O(N²) because each agent benefits from all others' contributions. This creates
a positive feedback loop: more agents → more knowledge → each agent performs better → more
agents attracted.

However, Metcalfe's Law assumes frictionless communication. In practice, communication overhead
scales as O(N²) as well, creating diminishing returns beyond a critical threshold. Roko
addresses this with stigmergic coordination (indirect communication via shared Engrams in the
Substrate) rather than direct agent-to-agent messaging, which keeps communication overhead
sublinear.

### 1.3 Stigmergy: Coordination Without Communication

Stigmergy (Grassé 1959; Parunak 2006) is the mechanism by which agents coordinate through
the environment rather than through direct communication. In Roko, agents coordinate through
shared Engrams in the Substrate — an agent posts an insight, and other agents discover it
through Substrate queries. No direct messaging protocol is needed.

Dorigo et al. (2000) demonstrated that stigmergic coordination in ant colony optimization
achieves near-optimal solutions with minimal communication overhead. Roko applies the same
principle: agents leave "pheromone trails" (knowledge Engrams with decay) that guide other
agents' behavior without requiring synchronous communication.

---

## 2. C-Factor: The Reporting Metric

### 2.1 Definition

C-Factor is a ratio metric that answers one question: **is the collective smarter than the
sum of its parts?**

```
C-Factor = Collective Performance / Sum(Individual Performances)
```

Where:
- **Collective Performance**: The group's output quality when agents have access to shared
  knowledge, coordination mechanisms, and the Agent Mesh.
- **Sum(Individual Performances)**: The sum of each agent's output quality when working in
  complete isolation (no shared knowledge, no mesh, no coordination).

### 2.2 Interpretation

| C-Factor | Interpretation |
|---|---|
| > 1.0 | **Superlinear** — the collective outperforms the sum of its parts. Synergy is present. |
| = 1.0 | **Linear** — agents are independent. No benefit or harm from collaboration. |
| < 1.0 | **Sublinear** — coordination overhead exceeds collaboration benefit. The group is *worse* than the sum of its parts. |
| >> 1.0 (e.g., 1.5+) | **Strongly superlinear** — emergent collective intelligence. The group discovers solutions no individual agent could reach. |

### 2.3 Measurement Methodology

Measuring C-Factor requires a controlled comparison:

1. **Baseline (isolated)**: Run N agents on a task suite individually. No shared Substrate,
   no mesh access, no collective calibration. Sum their individual gate-pass rates.

2. **Treatment (collective)**: Run the same N agents on the same task suite with full
   collective features enabled: shared Neuro knowledge, Daimon cross-modulation, collective
   calibration via Korai chain.

3. **Compute ratio**: C-Factor = treatment_pass_rate / baseline_sum_pass_rate.

The measurement should be repeated across multiple task suites to compute a robust average.
Single-task C-Factor can be misleading; the general factor (c) emerges from cross-task
measurement, just as Woolley et al. demonstrated.

### 2.4 Tracking Granularity

C-Factor is tracked at three levels:

| Level | Scope | Use |
|---|---|---|
| **Per-collective** | A specific group of cooperating agents | Diagnose why a particular team is underperforming |
| **Per-domain** | All agents working in one domain (coding, chain, research) | Compare collective effectiveness across domains |
| **Global** | All agents on the Korai network | Network health and scaling validation |

---

## 3. C-Score: The Optimization Metric

### 3.1 Definition

While C-Factor is a ratio for reporting, C-Score is a composite metric for optimization.
Each component can be independently measured and improved:

```
C-Score = gate_pass × 0.3
        + cost_efficiency × 0.2
        + speed × 0.15
        + first_try_rate × 0.25
        + knowledge_growth × 0.1
```

### 3.2 Component Breakdown

| Component | Weight | Definition | Range | How to Improve |
|---|---|---|---|---|
| **gate_pass** | 0.30 | Fraction of task outputs that pass the Gate pipeline | [0, 1] | Better context engineering, stronger verification, iterative repair |
| **cost_efficiency** | 0.20 | Output quality per dollar of inference cost | [0, ∞) normalized | T0 probe suppression, CascadeRouter optimization, caching |
| **speed** | 0.15 | Tasks completed per unit time | [0, ∞) normalized | Parallel execution, efficient DAG scheduling, reduced retry loops |
| **first_try_rate** | 0.25 | Fraction of tasks passing Gate on the first attempt | [0, 1] | Better predictions, stronger context, domain expertise |
| **knowledge_growth** | 0.10 | Net growth of validated knowledge (promotions − demotions) | ℝ normalized | Dream consolidation, cross-domain transfer, AntiKnowledge accumulation |

### 3.3 Why These Weights

The weights reflect Roko's design priorities:

- **first_try_rate (0.25)** and **gate_pass (0.30)** together account for 55% — correctness
  is the dominant concern. An agent that produces wrong output quickly and cheaply is worse
  than useless; it wastes verification resources and may cause harm.
- **cost_efficiency (0.20)** ensures that correctness is not achieved through brute force
  (e.g., always using T2). The T0/T1/T2 tier system and FrugalGPT-inspired cascading
  directly optimize this component.
- **speed (0.15)** prevents gold-plating — spending excessive time on tasks that could be
  completed faster at acceptable quality.
- **knowledge_growth (0.10)** ensures the system is self-improving. An agent that solves
  tasks but never learns is a tool, not a cognitive system.

### 3.4 Normalization

Components with unbounded ranges (cost_efficiency, speed, knowledge_growth) are normalized
relative to a baseline established during the first measurement period. This makes C-Score
values comparable across different domains and time periods:

```
normalized_component = actual_value / baseline_value
```

Where `baseline_value` is the median value observed during the initial calibration window
(first 7 days of collective operation).

---

## 4. Four Diagnostic Signals

Beyond the aggregate metrics, four diagnostic signals provide mechanistic insight into
collective dynamics. These correspond to Woolley et al.'s empirical predictors of collective
intelligence, adapted for agent systems.

### 4.1 Turn-Taking Equality

**Woolley analogue**: Equality of conversational turn-taking — the strongest predictor of
collective intelligence in human groups.

**Agent analogue**: How evenly cognitive load and output contribution are distributed across
agents in a collective.

```
Measurement:
  contribution(agent_i) = count of Engrams produced by agent_i in period T
  equality = 1.0 - gini_coefficient(contributions)
```

A Gini coefficient of 0 means perfect equality (all agents contribute equally); 1 means
perfect inequality (one agent does everything). The diagnostic signal is `1 - gini`, so
higher is better.

**Why it matters**: When one agent dominates output, the collective degenerates to
single-agent performance. The dominant agent becomes a bottleneck, and the collective loses
the diversity of approaches that drives superlinear performance. If turn-taking equality
drops below 0.5, the system should investigate whether some agents are idle, blocked, or
producing output that is consistently rejected by Gates.

### 4.2 Knowledge Flow Rate

**Woolley analogue**: Social sensitivity — how well members perceive and respond to each
other's signals.

**Agent analogue**: How effectively knowledge transfers between agents. Measured by how
often one agent's knowledge (stored in Neuro) is successfully used by another agent.

```
Measurement:
  transfers = count of Engrams where:
    - created by agent_A (provenance.author = A)
    - retrieved and used by agent_B (lineage includes A's engram)
    - agent_B's output passes Gate verification
  flow_rate = transfers / total_knowledge_entries
```

High knowledge flow rate indicates that agents are effectively learning from each other.
Low flow rate suggests knowledge silos — agents are producing knowledge that others cannot
find or use. This may indicate poor HDC encoding, mismatched Kind taxonomies, or
insufficient Substrate query coverage.

### 4.3 Cross-Domain Transfer

**Woolley analogue**: Task diversity — collective intelligence is a general factor that
predicts performance across dissimilar tasks.

**Agent analogue**: Whether insights generated in one domain improve performance in another
domain. This is the empirical test of Cross-Domain Insight Resonance (see
[17-design-principles-and-frontier-summary](./17-design-principles-and-frontier-summary.md)).

```
Measurement:
  insight = Engram created in domain_X
  usage = Same Engram retrieved (via HDC similarity) and used in domain_Y
  success = Gate.verify() passes for the domain_Y task
  transfer_rate = successful cross-domain usages / total cross-domain retrievals
```

This metric validates whether HDC encoding genuinely enables structural analogy detection
across domains. The theoretical false positive rate at threshold 0.526 is < 1% against a
100K vocabulary (Bonferroni-corrected; see refactoring-prd/09-innovations.md Section XIII),
but empirical validation is needed.

### 4.4 Emergent Coordination

**Woolley analogue**: Emergent group structure — effective groups spontaneously develop
useful roles and coordination patterns.

**Agent analogue**: Whether agents spontaneously develop useful specializations without
explicit role assignment.

```
Measurement:
  specialization(agent_i) = entropy of agent_i's task categories
    Low entropy = high specialization (agent focuses on few task types)
    High entropy = generalist (agent handles all task types equally)
  coordination = correlation between agent specialization and task outcome quality
```

If agents that spontaneously specialize perform better than generalists on their specialty
tasks, emergent coordination is present. This is a signal that the collective is
self-organizing — a hallmark of healthy stigmergic systems (Bonabeau et al. 1999).

---

## 5. Collective Calibration: The 31.6× Heuristic

### 5.1 The Model

> **Important caveat**: This is a Nunchi-derived scaling heuristic, not a published theorem.
> The sqrt(N) scaling is inspired by the Central Limit Theorem (estimation error scales as
> 1/sqrt(n) for independent observations) but the specific accuracy formula is an informal
> approximation. Empirical validation on actual agent populations is needed.

Solo agent learning: `accuracy(t) = 1 - 1/sqrt(t)` where t = number of verified predictions.

Collective learning (N agents on Korai): `accuracy(t) = 1 - 1/sqrt(N × t)`

At N = 1,000 agents: `sqrt(1000) ≈ 31.6×` faster calibration (theoretical upper bound under
the independence assumption).

### 5.2 Assumptions and Limitations

| Assumption | Reality | Impact on Estimate |
|---|---|---|
| Agent predictions are independent | Agents using the same LLM will have correlated predictions | Overestimates speedup by unknown factor |
| Observations are equally informative | Some task domains yield more useful signal than others | May underestimate in high-signal domains |
| No distribution shift across agents | Agents operate in different sub-domains | Requires domain normalization |
| Communication is instantaneous | Korai blocks take ~400ms | Negligible latency at daily aggregation |

### 5.3 The Mechanism

The collective calibration mechanism works through the CalibrationTracker:

```
1. Agent makes a falsifiable prediction (PredictionClaim Engram)
2. Agent executes the task
3. External verifier (compiler, test suite, blockchain) determines actual outcome
4. Residual = predicted - actual
5. Residual is aggregated per (model, task_category) pair
6. Adjusted prediction = raw_prediction - mean_bias(model, category)
```

On the Korai chain, all agents can read the collective's calibration level. New agents start
at the collective's calibration, not at zero. This is the mechanism behind faster learning.

### 5.4 Projected Impact

| Agents | Predictions/Day | Verified Corrections/Day | Task Success | Cost/Task |
|---|---|---|---|---|
| Solo | 20 | 16 | 15% | $4.70 |
| 100 | 10,000 | 8,000 | 52% | $1.80 |
| 1,000 | 100,000 | 80,000 | 79% | $0.65 |
| 10,000 | 1,000,000 | 800,000 | 86% | $0.40 |

At 10,000 agents: 800,000 externally-verified learning signals per day versus approximately
1,000 evaluations/month for traditional ML systems.

### 5.5 The Network Flywheel

```
More agents → more insights posted to Korai
    → richer knowledge base → each agent performs better
    → more agents attracted → even richer knowledge
    → superlinear scaling (Metcalfe's Law for intelligence)
```

N agents produce O(N) insights, but total network value grows O(N²) because each agent
benefits from all others' contributions. C-Factor tracks whether this flywheel is actually
spinning (C-Factor > 1.0) or stalled (C-Factor ≤ 1.0).

---

## 6. C-Factor in the Synapse Architecture

### 6.1 Where C-Factor Is Computed

C-Factor computation spans multiple architectural layers:

| Layer | Role in C-Factor |
|---|---|
| L0 Runtime | Collects raw event counts (Engrams created, queries answered, time spent) |
| L3 Harness | Gate verdicts provide the pass/fail ground truth for gate_pass and first_try_rate |
| L4 Orchestration | Aggregates per-agent metrics into collective metrics |
| Neuro (cross-cut) | Tracks knowledge flow between agents, tier promotions/demotions |
| Daimon (cross-cut) | PAD vector evolution contributes to behavioral state tracking |
| Korai chain | On-chain verified predictions provide the calibration dataset |

### 6.2 Engram Kinds for C-Factor

C-Factor metrics are themselves Engrams, enabling the same query/score/route/compose
pipeline to operate on meta-level performance data:

| Kind | Purpose |
|---|---|
| `Kind::Metric` | Raw metric values (gate_pass_rate, cost_per_task, etc.) |
| `Kind::Episode` | Agent turn records that contribute to performance measurement |
| `Kind::Verdict` | Gate verdicts that feed into gate_pass and first_try_rate |
| `Kind::PredictionClaim` | Falsifiable predictions that feed into calibration |
| `Kind::Custom("c_factor")` | Computed C-Factor and C-Score values |

### 6.3 C-Factor as Policy Input

Policy traits can observe C-Factor trends and emit intervention Engrams:

```
If C-Factor is declining:
  Policy emits: Kind::Intervention("investigate_collective_degradation")

If knowledge_flow_rate drops below threshold:
  Policy emits: Kind::Intervention("improve_hdc_encoding")

If turn_taking_equality drops below 0.5:
  Policy emits: Kind::Intervention("rebalance_task_distribution")
```

This closes the loop — C-Factor is not just a measurement but an input to the agent's
own self-improvement system.

---

## 7. Comparison with Existing Collective Metrics

### 7.1 Why Not Just Use Task Success Rate?

Task success rate (gate_pass) is a necessary but insufficient metric for collective
performance. A collective could achieve high gate_pass through:

- Brute force (T2 for everything) — but this is unsustainable and expensive
- Cherry-picking (only attempting easy tasks) — but this avoids valuable hard problems
- Excessive retries (passing on attempt 5) — but this wastes resources

C-Score captures all of these failure modes through its multi-component design.

### 7.2 Why Not Just Use Cost?

Cost efficiency alone encourages agents to skip verification, use cheaper models
indiscriminately, and avoid difficult tasks. By weighting correctness (0.55) above cost
(0.20), C-Score ensures that cost optimization happens *within* a correctness floor.

### 7.3 Relationship to Viable System Model

Beer's Viable System Model (1972) includes System 4 (intelligence — looking outward and
forward) and System 5 (policy — identity and meta-management). C-Factor serves both:

- As **System 4**: C-Factor diagnostics reveal external threats to collective health
  (declining knowledge flow, emerging knowledge silos, coordination breakdown)
- As **System 5**: C-Factor defines the identity of a well-functioning collective
  (superlinear, balanced, self-improving)

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Woolley et al. 2010, Science 330(6004) | Discovery of general collective intelligence factor (c) in human groups |
| Metcalfe 2013, Computer 46(12) | Network value scaling (V ∝ N²), applied to agent knowledge networks |
| Grassé 1959 | Stigmergy: coordination through environmental modification |
| Parunak 2006 | Stigmergy in digital environments, applied to multi-agent systems |
| Dorigo et al. 2000 | Ant colony optimization: stigmergic coordination achieves near-optimal solutions |
| Bonabeau et al. 1999, Swarm Intelligence | Self-organization in insect societies, emergent role specialization |
| Beer 1972, Brain of the Firm | Viable System Model: recursive organizational intelligence |
| Chen et al. 2023, arXiv:2305.05176 | FrugalGPT: cascade cost optimization for LLM inference |
| Central Limit Theorem | Estimation error scales as 1/sqrt(n) for independent observations — basis for collective calibration scaling |

---

## Current Status and Gaps

- **C-Factor definition**: Specified in refactoring-prd. Two-level metric (ratio + composite)
  fully defined with component weights.
- **Four diagnostic signals**: Specified with measurement formulas. Not yet instrumented in
  the codebase.
- **Collective calibration (31.6×)**: Heuristic model defined. CalibrationTracker specified
  in `roko-learn`. Requires Korai chain for cross-agent aggregation (not yet built).
- **Implementation gap**: C-Factor computation requires aggregation across multiple agents.
  This depends on the Agent Mesh (successor to the legacy Styx relay) and the Korai chain
  for on-chain verified predictions. Both are Tier 4+ features.
- **Empirical validation**: The 31.6× speedup, the turn-taking equality diagnostic, and the
  cross-domain transfer rate are all untested hypotheses. Validation requires running 100+
  agents on the Daeji testnet and measuring actual collective performance versus solo
  baselines.

---

## Cross-References

- See [09-universal-cognitive-loop](./09-universal-cognitive-loop.md) for the loop that generates the performance data C-Factor tracks
- See [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) for the Neuro/Daimon/Dreams subsystems that C-Factor diagnoses
- See [17-design-principles-and-frontier-summary](./17-design-principles-and-frontier-summary.md) for the 31.6× collective calibration as a Blue Ocean innovation
- See topic [08-chain](../08-chain/INDEX.md) for Korai chain mechanics underlying collective calibration
- See topic [13-coordination](../13-coordination/INDEX.md) for Agent Mesh and multi-agent coordination
