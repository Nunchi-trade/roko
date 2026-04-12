# Collective Intelligence Metrics: Measuring Coordination Effectiveness

> **Layer**: L3 Harness (monitoring and measurement), L4 Orchestration (collective-level
> analysis)
>
> **Synapse traits**: `Scorer` (scoring collective outputs), `Gate` (verifying collective
> claims), `Policy` (adjusting coordination based on metrics)
>
> **Prerequisites**: `00-stigmergy-theory.md` (coordination fundamentals),
> `10-exponential-flywheel.md` (what the metrics should detect)

---

## Overview

Stigmergic coordination is only valuable if it produces measurable collective intelligence —
outputs that exceed what individual agents could achieve alone. This sub-doc specifies the
metrics framework for measuring, diagnosing, and optimizing collective intelligence in Roko
Collectives.

The central metric is the **C-Factor** (Collective Intelligence Factor), derived from Woolley
et al.'s seminal research on human groups [Woolley, A.W. et al. "Evidence for a Collective
Intelligence Factor in the Performance of Human Groups." *Science*, 330(6004):686-688, 2010].
The C-Factor quantifies the degree to which a collective's performance exceeds the sum of
its members' individual performances.

---

## The C-Factor: Definition and Measurement

### Origin

Woolley et al. (2010) demonstrated that human groups exhibit a "collective intelligence" factor
(c) that is:

1. **Not reducible to the maximum individual intelligence in the group** — smart groups are
   not simply groups with a smart member
2. **Correlated with social sensitivity, conversational turn-taking equality, and proportion
   of women in the group** (for human groups)
3. **Predictive of group performance across diverse tasks** — c predicts performance on novel
   tasks the group has never attempted

### Roko's C-Factor Definition

For Roko Collectives, the C-Factor is defined as:

```
C-Factor = Collective_Output / Σ(Individual_Outputs)
```

Where:
- `Collective_Output` = the quality-weighted task completions achieved by the Collective
  working together (with pheromone coordination, morphogenetic specialization, shared
  knowledge)
- `Σ(Individual_Outputs)` = the sum of quality-weighted task completions each agent would
  achieve working independently (no coordination, no shared pheromones)

| C-Factor | Interpretation |
|----------|---------------|
| < 1.0 | **Coordination overhead exceeds benefit** — the collective is worse than the sum of its parts. Common in poorly configured Collectives or with redundant agents. |
| = 1.0 | **No collective intelligence** — agents don't benefit from coordination. Each contributes independently. |
| 1.0 – 1.5 | **Modest collective intelligence** — coordination provides moderate benefit. Typical for small Collectives (2-5 agents) or early-stage operation. |
| 1.5 – 3.0 | **Strong collective intelligence** — significant coordination benefit. Typical for well-configured Collectives with morphogenetic specialization. |
| > 3.0 | **Superlinear collective intelligence** — exceptional coordination. Indicates active flywheel mechanisms (see `10-exponential-flywheel.md`). |

### Measurement Method

C-Factor measurement requires a controlled comparison:

1. **Collective trial**: Run the Collective on a task set with full coordination (pheromone
   field, morphogenetic specialization, knowledge sharing enabled)
2. **Individual baseline**: Run each agent independently on the same task set (coordination
   disabled)
3. **Compute ratio**: C-Factor = Collective score / Sum of individual scores

For ongoing measurement (without disrupting production), Roko uses an estimation approach:

```rust
/// Estimate the C-Factor from instrumentation data.
///
/// Uses the ratio of collective task completion rate to the sum of
/// individual task start rates, adjusted for task difficulty and
/// interdependency.
pub fn estimate_c_factor(
    collective_completions: &[TaskCompletion],
    individual_starts: &[TaskStart],
    task_difficulty: &HashMap<TaskId, f64>,
) -> f64 {
    let collective_score: f64 = collective_completions.iter()
        .map(|c| c.quality_score * task_difficulty.get(&c.task_id).unwrap_or(&1.0))
        .sum();

    let individual_score: f64 = individual_starts.iter()
        .map(|s| s.estimated_solo_quality * task_difficulty.get(&s.task_id).unwrap_or(&1.0))
        .sum();

    if individual_score > 0.0 {
        collective_score / individual_score
    } else {
        1.0  // No individual data; assume neutral
    }
}
```

---

## Composite C-Score

The C-Factor gives a single ratio. For deeper diagnostics, Roko computes a composite C-Score
that decomposes collective intelligence into four diagnostic signals:

### Signal 1: Turn-Taking Equality

In Woolley et al.'s human group studies, the most predictive feature of collective intelligence
was **equality of conversational turn-taking** — groups where one person dominated performed
worse than groups where contributions were more equally distributed.

For Roko Collectives, turn-taking equality measures how evenly pheromone deposits are
distributed across agents:

```rust
/// Measure turn-taking equality in pheromone deposits.
///
/// Returns a value in [0, 1]:
/// - 1.0 = perfectly equal (all agents deposit equally)
/// - 0.0 = maximally unequal (one agent deposits everything)
///
/// Uses normalized Shannon entropy of deposit counts.
pub fn turn_taking_equality(
    deposits_per_agent: &HashMap<AgentId, u64>,
) -> f64 {
    let total: f64 = deposits_per_agent.values().sum::<u64>() as f64;
    if total == 0.0 { return 1.0; }

    let h: f64 = deposits_per_agent.values()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let p = count as f64 / total;
            -p * p.ln()
        })
        .sum();

    let n = deposits_per_agent.len() as f64;
    let h_max = n.ln();
    if h_max == 0.0 { return 1.0; }

    h / h_max
}
```

**Target**: > 0.7. Below this threshold, the Collective has a "loudest agent" problem — one
agent's pheromones dominate the field, reducing the diversity benefit.

### Signal 2: Knowledge Flow Rate

How quickly does knowledge propagate through the Collective? Measured as the average time
between a pheromone deposit and its first confirmation by a different agent:

```rust
/// Measure knowledge flow rate in the Collective.
///
/// Returns the average time (in ticks) between pheromone deposit
/// and first confirmation by a different agent. Lower is better.
pub fn knowledge_flow_rate(
    deposits: &[PheromoneDeposit],
    confirmations: &[PheromoneConfirmation],
) -> f64 {
    let mut flow_times = Vec::new();

    for deposit in deposits {
        if let Some(first_confirm) = confirmations.iter()
            .filter(|c| c.pheromone_id == deposit.id && c.confirmer != deposit.source)
            .min_by_key(|c| c.tick)
        {
            flow_times.push((first_confirm.tick - deposit.tick) as f64);
        }
    }

    if flow_times.is_empty() { return f64::INFINITY; }
    flow_times.iter().sum::<f64>() / flow_times.len() as f64
}
```

**Target**: < 100 ticks (~25 minutes at 4 ticks/minute). Slower flow rates indicate
communication bottlenecks or insufficient agent overlap.

### Signal 3: Cross-Domain Transfer

How often does a pheromone deposited by one agent type influence an agent of a different type?
This measures the cross-pollination of ideas across specialization boundaries.

```rust
/// Measure cross-domain transfer rate.
///
/// Returns the fraction of pheromone sensing events where the
/// sensing agent has a different primary domain than the depositing
/// agent. Higher indicates more cross-domain fertilization.
pub fn cross_domain_transfer(
    sensing_events: &[PheromoneSensingEvent],
) -> f64 {
    if sensing_events.is_empty() { return 0.0; }

    let cross_domain = sensing_events.iter()
        .filter(|e| e.sensor_domain != e.depositor_domain)
        .count();

    cross_domain as f64 / sensing_events.len() as f64
}
```

**Target**: > 0.2. Below this threshold, agents are operating in silos — each domain's
pheromones only influence agents in the same domain, missing the cross-domain resonance
mechanism (see `10-exponential-flywheel.md`, Mechanism 7).

### Signal 4: Emergent Coordination

How often do agents spontaneously coordinate without explicit task assignment? Measured by
the frequency of "coordination chains" — sequences where Agent A's pheromone deposit triggers
Agent B's action, which triggers Agent C's action, without any central orchestrator directing
the chain.

```rust
/// Measure emergent coordination rate.
///
/// Returns the fraction of task completions that were triggered by
/// pheromone sensing (stigmergic coordination) rather than direct
/// task assignment (orchestrator coordination).
pub fn emergent_coordination_rate(
    task_completions: &[TaskCompletion],
) -> f64 {
    if task_completions.is_empty() { return 0.0; }

    let stigmergic = task_completions.iter()
        .filter(|c| c.trigger == TaskTrigger::PheromoneSensed)
        .count();

    stigmergic as f64 / task_completions.len() as f64
}
```

**Target**: > 0.3. A Collective where all work is explicitly assigned (orchestrator-driven)
is not leveraging stigmergic coordination. The target indicates that at least 30% of work
emerges from pheromone sensing rather than top-down assignment.

### Composite C-Score

The four signals combine into a composite score:

```
C-Score = w₁ × turn_taking + w₂ × (1 / knowledge_flow) + w₃ × cross_domain + w₄ × emergent
```

Default weights: w₁ = 0.25, w₂ = 0.25, w₃ = 0.25, w₄ = 0.25 (equal weighting).

The C-Score provides actionable diagnostics:

| C-Score | Diagnosis | Action |
|---------|-----------|--------|
| High turn-taking, low flow | Agents deposit but don't read each other's signals | Check pheromone sensing thresholds |
| Low turn-taking, high flow | One agent dominates; others follow | Increase morphogenetic inhibition (beta) |
| High flow, low cross-domain | Fast propagation but siloed | Lower domain filtering thresholds |
| High cross-domain, low emergent | Agents sense cross-domain signals but don't act | Check Policy implementations |

---

## Evidence Framework: Three Levels

The proving-collective-intelligence research document specifies three levels of evidence
for demonstrating that collective intelligence causally improves outcomes:

### Level 1: Scaling Experiments

**Method**: Vary Collective size (N = 1, 2, 5, 10, 20) and measure task completion quality
on a standardized benchmark.

**Expected result**: Quality scales superlinearly (β > 1.0) up to a ceiling determined by
task complexity.

**Control**: Each agent runs the same tasks solo (no coordination).

### Level 2: Ablation Studies

**Method**: Selectively disable coordination mechanisms and measure impact:

| Ablation | Expected Impact |
|----------|----------------|
| Disable pheromone field | C-Factor drops to ~1.0 (no coordination benefit) |
| Disable morphogenetic specialization | C-Factor drops (redundant work increases) |
| Disable confirmation mechanism | Signal quality degrades (noise accumulates) |
| Disable cross-scope promotion | Knowledge stays local; collective doesn't benefit |

### Level 3: Causal Inference

**Method**: Use Structural Causal Models (SCMs) to establish that coordination mechanisms
*cause* improved outcomes, controlling for confounds (e.g., more agents simply means more
compute, regardless of coordination).

**SwarmBench**: Ruan et al. (2025) proposed SwarmBench as a standardized benchmark for
evaluating multi-agent systems [Ruan, Y. et al. "SwarmBench: Evaluating Multi-Agent
Collaboration." 2025]. Roko can adapt this benchmark to measure collective intelligence
specifically.

---

## Information-Theoretic Metrics

### Normalized Entropy of the Pheromone Field

The normalized entropy of the pheromone field measures information diversity:

```rust
/// Compute the normalized entropy of the pheromone field.
///
/// Returns a value in [0, 1]:
/// - 0.0 = all pheromones are the same kind (no diversity)
/// - 1.0 = pheromones are uniformly distributed across kinds (maximum diversity)
///
/// Based on the Chronos framework for information-theoretic analysis
/// of multi-agent systems.
pub fn pheromone_field_entropy(
    kind_counts: &HashMap<PheromoneKind, u64>,
) -> f64 {
    let total: f64 = kind_counts.values().sum::<u64>() as f64;
    if total == 0.0 { return 0.0; }

    let h: f64 = kind_counts.values()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let p = count as f64 / total;
            -p * p.ln()
        })
        .sum();

    let h_max = (kind_counts.len() as f64).ln();
    if h_max == 0.0 { return 0.0; }

    h / h_max
}
```

**Target**: 0.4–0.8. Too low indicates the Collective is focused on only one type of signal
(e.g., all Threats, no Opportunities). Too high indicates no prioritization (signals spread
evenly across all kinds regardless of actual conditions).

### Mutual Information Between Agents

The mutual information between two agents' pheromone deposit patterns measures how much one
agent's behavior predicts the other's:

```
I(A; B) = Σ p(a, b) × log(p(a, b) / (p(a) × p(b)))
```

High mutual information between all pairs indicates lockstep behavior (agents are copying each
other). Low mutual information indicates independence (agents are not responding to each
other's signals). The optimal operating point is moderate mutual information — agents are
influenced by but not determined by each other's signals.

---

## Population-Level A/B Testing

For rigorous evaluation of coordination mechanisms, Roko supports population-level A/B testing:

### Design

1. **Treatment group**: Collectives with the coordination mechanism enabled
2. **Control group**: Collectives with the mechanism disabled (or with a baseline version)
3. **Randomization unit**: The Collective (not the individual agent), to account for
   within-Collective correlations

### Statistical Framework

Use clustered standard errors to account for within-Collective correlation [Evan Miller,
"Statistical Significance for A/B Tests with Clustered Standard Errors"; Anthropic, 2024]:

```
Standard Error = sqrt(Σ (cluster_residual)² / (K × (K-1)))
```

Where K = number of Collectives (clusters). This prevents false positives from treating
correlated agent outcomes as independent.

### Minimum Detectable Effect

For a typical deployment (10 Collectives per group, 5 agents per Collective):
- 80% power to detect a 15% improvement in C-Factor
- Required duration: ~500 ticks per Collective (~2 hours)

---

## Dashboard Integration

The collective intelligence metrics are displayed in Roko's text-mode dashboard
(`roko dashboard`):

```
Collective Intelligence Dashboard
═══════════════════════════════════════════════════════════
C-Factor:  2.34  [▓▓▓▓▓▓▓▓▓░░░] (target: > 1.5)
C-Score:   0.71  [▓▓▓▓▓▓▓░░░░░] (composite)

Diagnostics:
  Turn-taking equality:   0.82  [▓▓▓▓▓▓▓▓░░] ✓
  Knowledge flow rate:    67 ticks [▓▓▓▓▓▓░░░░] ✓
  Cross-domain transfer:  0.28  [▓▓▓░░░░░░░] ✓
  Emergent coordination:  0.35  [▓▓▓░░░░░░░] ✓

Pheromone Field:
  Active pheromones: 142
  Field entropy: 0.63 [▓▓▓▓▓▓░░░░]
  Confirmation rate: 0.41

Morphogenetic:
  Avg specialization: 0.67
  Niche competition: 1.2 agents/niche
  Role conflicts: 0
═══════════════════════════════════════════════════════════
```

---

## Adaptive Optimization

The metrics feed back into the coordination system to optimize collective intelligence:

| Metric | Below Target | Adjustment |
|--------|-------------|-----------|
| C-Factor < 1.0 | Coordination overhead too high | Reduce pheromone deposit frequency; simplify pheromone kinds |
| Turn-taking < 0.7 | Dominant agent | Increase morphogenetic inhibition; reduce dominant agent's deposit rate |
| Knowledge flow > 100 ticks | Slow propagation | Increase immediate push threshold; check transport health |
| Cross-domain < 0.2 | Siloed agents | Lower domain filter thresholds; add cross-domain scoring bonus |
| Emergent coordination < 0.3 | Over-orchestrated | Reduce explicit task assignment; increase pheromone visibility |

These adjustments are implemented by the `Policy` trait at L4 Orchestration, which observes
the metrics stream and emits configuration-adjustment Engrams.

---

## References

- [Anthropic 2024] Clustered standard errors for A/B tests
- [Evan Miller] Statistical Significance with Clustered Standard Errors
- [Ruan et al. 2025] SwarmBench: Evaluating Multi-Agent Collaboration
- [Shannon 1948] Mathematical Theory of Communication, *Bell System Technical Journal*
- [Surowiecki 2004] *The Wisdom of Crowds*, Doubleday
- [Woolley et al. 2010] Collective Intelligence Factor, *Science* 330(6004):686-688

---

## Related Sub-Docs

- `00-stigmergy-theory.md` — The coordination mechanism being measured
- `07-morphogenetic-specialization.md` — Specialization metrics
- `09-stigmergy-scaling.md` — Scaling properties of coordination
- `10-exponential-flywheel.md` — The mechanisms that produce superlinear C-Factor
- `12-current-status-and-gaps.md` — Implementation status of metrics
