# Dream Evolution: The Fourth Phase

> **Layer**: Cognitive Cross-Cut (L2 Scaffold knowledge recombination)
>
> **Synapse Traits**: `Scorer` (memetic fitness scoring), `Policy` (strategy evolution policy)
>
> **Crate**: `roko-dreams` (planned — not yet implemented)
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [04-consolidation-and-staging.md](04-consolidation-and-staging.md)


> **Implementation**: Scaffold

---

## The EVOLUTION Phase

Beyond the three core phases (NREM Replay, REM Imagination, Integration), the dream system includes a fourth phase: **EVOLUTION**. This phase operates on promoted knowledge entries — those that have already passed through the staging buffer and been validated by waking experience — and applies evolutionary selection pressures to generate higher-order strategies.

EVOLUTION is not triggered every dream cycle. It fires when the agent has accumulated a sufficient body of promoted knowledge (configurable threshold, default: 20 promoted entries since the last EVOLUTION cycle). The phase is computationally expensive and produces high-level strategic recombinations that reshape the agent's approach.

---

## Three Operations

### 1. Memetic Selection

Heuristics and strategies in NeuroStore compete for survival. EVOLUTION evaluates each promoted entry against the agent's recent performance:

- **High-fitness heuristics** (their presence correlated with successful episodes) receive a confidence boost and have their half-life extended by 1.5×.
- **Low-fitness heuristics** (their presence correlated with failed episodes, or they were never referenced in any episode) receive a confidence penalty and begin accelerated decay.
- **Neutral heuristics** (no correlation with success or failure) are left unchanged.

The fitness function:

```
fitness(heuristic) = success_rate_when_referenced / success_rate_when_not_referenced
```

A fitness > 1.0 means the heuristic is correlated with success. A fitness < 1.0 means the heuristic is correlated with failure. A fitness ≈ 1.0 means the heuristic has no effect.

This implements a simplified version of the memetic evolution described by Dawkins (1976, The Selfish Gene): ideas (memes) compete for replication within the agent's cognitive architecture. Successful memes survive; unsuccessful ones die.

### 2. Strategy Evolution via Imagined Returns

EVOLUTION takes pairs of high-fitness heuristics and combines them to produce candidate super-strategies:

```
Heuristic A (fitness 1.8): "{heuristic_a}"
Heuristic B (fitness 2.1): "{heuristic_b}"

These heuristics are both high-fitness. They both correlate with success.
What happens if you combine them into a single compound strategy?
Under what conditions would the compound be better than either alone?
Under what conditions would they conflict?
```

The compound strategies enter the staging buffer at confidence 0.30 (the maximum for dream-generated hypotheses). They represent the agent's best current thinking — validated heuristics combined in novel ways.

### 3. Knowledge Recombination

EVOLUTION applies Wright's (1932, Proceedings of the Sixth International Congress of Genetics) shifting balance theory: knowledge entries are randomly paired and recombined using HDC vector operations to explore the neighborhood of existing strategies.

The recombination uses HDC permutation — a cyclic bit-shift on the 10,240-bit BSC vector that represents the knowledge entry's content:

```rust
let recombined = HdcVector::bundle(&[
    &entry_a.hdc_vector,
    &entry_b.hdc_vector.permute(shift_amount),
]);
```

The permuted bundle creates a vector that is related to both parent entries but distinct from either. The nearest neighbors of this recombined vector in NeuroStore identify potentially relevant knowledge entries that the agent has not yet connected. These connections are surfaced as candidate insights.

This implements the "dream seed" concept: HDC vectors serve as seeds for knowledge exploration. Each permutation explores a slightly different region of the agent's knowledge space, analogous to genetic mutation in biological evolution.

---

## The Dream-Prediction Feedback Loop

EVOLUTION closes a critical feedback loop: dreams generate predictions, waking experience validates them, and the validation results feed back into future dreams.

```
DREAM → Generate hypothesis H with predicted outcome P
WAKE → Observe actual outcome O
DREAM → Compare P vs O
  If P ≈ O: boost confidence in H, reinforce the heuristics that generated H
  If P ≠ O: reduce confidence in H, weaken the heuristics that generated H
DREAM → Next EVOLUTION considers updated fitness scores
```

This is the **predictive foraging** mechanism from `agent-chain/10-predictive-foraging.md`: agents make falsifiable predictions, and incorrect predictions decay the knowledge that produced them. EVOLUTION's memetic selection operates on the fitness scores that this feedback loop produces.

---

## Implementation Status

The EVOLUTION phase is **not yet implemented** in `roko-dreams`. The implementation plan (§G8: "Novel strategy generation") lists it as a future item. The design is stable and the HDC primitives required (permutation, bundling, similarity) are all available in `roko-primitives` and `roko-learn`.

---

## Academic Citations

| Paper | How It Informs EVOLUTION |
|-------|-------------------------|
| Dawkins (1976), The Selfish Gene | Memetic evolution: ideas compete for survival within cognitive architectures |
| Wright (1932), "The roles of mutation, inbreeding, crossbreeding, and selection in evolution" | Shifting balance theory: exploration of adaptive landscapes via recombination |
| Kanerva (2009), Cognitive Computation 1(2), "Hyperdimensional Computing" | HDC permutation operations for knowledge recombination |
| Simonton (2010), "Creative thought as blind-variation and selective-retention" | BVSR theory applied to strategy evolution |

---

## Memetic Fitness: Rigorous Evaluation Framework

The existing fitness function (`fitness(heuristic) = success_rate_when_referenced / success_rate_when_not_referenced`) is a useful first approximation but has several statistical weaknesses. This section formalizes the fitness evaluation with proper statistical testing and causal inference.

### Bayesian Memetic Fitness

Instead of a simple ratio, use a Bayesian approach that accounts for sample size and prior uncertainty:

```
P(fitness > 1.0 | data) = ∫_{f>1.0} P(f | data) df
```

Where `P(f | data)` is the posterior distribution over fitness values given observed success rates.

```rust
/// Bayesian memetic fitness evaluator for the EVOLUTION phase.
/// Replaces naive ratio with proper uncertainty quantification.
pub struct BayesianMemeticFitness {
    /// Prior belief about baseline fitness (centered on 1.0 = neutral).
    pub prior_mean: f64,                   // default: 1.0
    /// Prior uncertainty (standard deviation).
    pub prior_std: f64,                    // default: 0.5, range: 0.1-2.0
    /// Minimum observations before evaluating fitness.
    pub min_observations: usize,           // default: 5, range: 3-20
    /// Confidence threshold: P(fitness > 1.0 | data) must exceed this
    /// to classify a heuristic as "high-fitness."
    pub confidence_threshold: f64,         // default: 0.75, range: 0.60-0.95
    /// Whether to account for confounding variables (other active heuristics).
    pub control_for_confounders: bool,    // default: true
    /// Maximum number of confounding heuristics to consider.
    pub max_confounders: usize,           // default: 5
}

/// Fitness evaluation result with uncertainty quantification.
pub struct FitnessEvaluation {
    pub heuristic_id: String,
    /// Point estimate of fitness (ratio of success rates).
    pub fitness_point_estimate: f64,
    /// Bayesian posterior probability that fitness > 1.0.
    pub prob_beneficial: f64,
    /// 90% credible interval for fitness.
    pub credible_interval_90: (f64, f64),
    /// Number of episodes where the heuristic was referenced.
    pub n_referenced: usize,
    /// Number of episodes where it was not referenced.
    pub n_unreferenced: usize,
    /// Classification based on confidence_threshold.
    pub classification: FitnessClassification,
}

pub enum FitnessClassification {
    /// P(fitness > 1.0) > confidence_threshold — keep and boost.
    Beneficial,
    /// P(fitness < 1.0) > confidence_threshold — harmful, accelerate decay.
    Harmful,
    /// Neither threshold met — insufficient evidence.
    Uncertain,
}
```

### Pseudocode

```
BAYESIAN-MEMETIC-FITNESS(heuristic, episodes, config):
  // Partition episodes by heuristic reference
  referenced = [e for e in episodes if heuristic.id in e.active_heuristics]
  unreferenced = [e for e in episodes if heuristic.id NOT in e.active_heuristics]

  IF |referenced| < config.min_observations:
    RETURN FitnessEvaluation { classification: Uncertain }

  // Compute success rates
  success_ref = count(e.succeeded for e in referenced) / |referenced|
  success_unref = count(e.succeeded for e in unreferenced) / |unreferenced|

  // Bayesian posterior via Beta-Binomial conjugate model
  // For referenced: Beta(α_ref + successes, β_ref + failures)
  // For unreferenced: Beta(α_unref + successes, β_unref + failures)
  // Fitness = ratio of two Beta-distributed variables → approximated

  // Monte Carlo estimation of P(fitness > 1.0)
  samples = 10000
  count_above_one = 0
  fitness_samples = []
  FOR i in 1..samples:
    s_ref = sample_beta(successes_ref + 1, failures_ref + 1)
    s_unref = sample_beta(successes_unref + 1, failures_unref + 1)
    f = s_ref / s_unref.max(0.001)
    fitness_samples.push(f)
    IF f > 1.0: count_above_one += 1

  prob_beneficial = count_above_one / samples
  credible_interval = (percentile(fitness_samples, 5), percentile(fitness_samples, 95))

  classification = IF prob_beneficial > config.confidence_threshold: Beneficial
                   ELSE IF (1.0 - prob_beneficial) > config.confidence_threshold: Harmful
                   ELSE: Uncertain

  RETURN FitnessEvaluation { ... }
```

### Test Criteria

```
1. Bayesian convergence: with 100+ observations and clear success rate difference,
   prob_beneficial converges to >0.99 for beneficial heuristics.
2. Insufficient evidence: with fewer than min_observations, classification is always Uncertain.
3. Credible interval coverage: the 90% CI contains the true fitness ratio in ≥85% of
   simulated test cases (accounting for Monte Carlo variance).
4. Confounder control: when control_for_confounders=true and a confounding heuristic
   explains the success difference, prob_beneficial is reduced.
5. Symmetric: a heuristic with identical success rates when referenced and unreferenced
   has prob_beneficial ≈ 0.50.
```

---

## Cross-Pollination: Evolutionary Knowledge Recombination

Cross-pollination extends the existing knowledge recombination (Section "Three Operations", Operation 3) with a tournament selection approach. Rather than randomly pairing knowledge entries, tournament selection applies evolutionary pressure by sampling candidates and selecting based on fitness, producing higher-quality recombinations.

```rust
/// Tournament selection for knowledge recombination during EVOLUTION.
pub struct TournamentRecombination {
    /// Tournament size: number of candidates sampled per selection.
    pub tournament_size: usize,           // default: 4, range: 2-8
    /// Elitism: fraction of top-fitness heuristics guaranteed to survive.
    pub elitism_fraction: f64,            // default: 0.10, range: 0.0-0.30
    /// Crossover rate: probability that two selected parents recombine.
    pub crossover_rate: f64,              // default: 0.70, range: 0.30-0.90
    /// Mutation rate: probability of random perturbation post-crossover.
    pub mutation_rate: f64,               // default: 0.15, range: 0.05-0.30
    /// Maximum population size (total active heuristics).
    pub max_population: usize,            // default: 200, range: 50-1000
}
```

---

## Quality-Diversity Search: MAP-Elites for Strategy Evolution

The EVOLUTION phase's knowledge recombination can be significantly enhanced by quality-diversity (QD) algorithms, which maintain a diverse archive of high-quality solutions indexed by behavioral characteristics. Rather than converging on a single "best" strategy, QD search produces a repertoire of diverse strategies, each optimal for a different niche.

### MAP-Elites Architecture

**Reference**: Mouret & Clune, "Illuminating search spaces by mapping elites," arXiv:1504.04909, 2015.

**Reference**: DCRL-MAP-Elites (ACM TELO 2024, GECCO 2023 Best Paper Award) — descriptor-conditioned actors serve as generative models for diverse solutions, combining PGA-MAP-Elites and DCG-MAP-Elites (RL actor-critic within MAP-Elites).

**Reference**: Santos, Julia, do Nascimento, "Diverse Prompts: Illuminating the Prompt Space of LLMs with MAP-Elites," IEEE CEC 2025, arXiv:2504.14367. MAP-Elites for generating structurally diverse, high-performing prompts across seven BigBench Lite tasks.

**Reference**: Samvelyan et al., "Rainbow Teaming: Open-Ended Generation of Diverse Adversarial Prompts," NeurIPS 2024, arXiv:2402.16822. First large-scale MAP-Elites for systematic LLM vulnerability mapping, achieving >90% attack success across Llama 2/3 with prompts diverse in both attack style and risk category.

MAP-Elites maintains a grid indexed by behavioral descriptors. Each cell contains the highest-performing solution for that behavioral niche. New solutions are generated by mutation/crossover, then placed in their corresponding cell — replacing the current occupant only if higher quality.

### Map to Roko's EVOLUTION Phase

Instead of random pairing for knowledge recombination, MAP-Elites provides structured exploration of the strategy space. Behavioral descriptors for agent strategies might include: task domain (coding/research/chain), complexity level (simple/compound), time horizon (reactive/deliberative), and confidence level (speculative/established).

```rust
/// MAP-Elites archive for quality-diversity strategy evolution.
/// Based on Mouret & Clune (2015), DCRL-MAP-Elites (ACM TELO 2024),
/// Rainbow Teaming (NeurIPS 2024).
pub struct MapElitesArchive {
    /// Behavioral descriptor dimensions for the archive grid.
    pub descriptor_dimensions: Vec<DescriptorDimension>,
    /// Number of bins per dimension.
    pub bins_per_dimension: usize,         // default: 10, range: 5-25
    /// Maximum archive size (total cells = bins^dimensions).
    pub max_archive_size: usize,           // default: 1000, range: 100-10000
    /// Mutation rate for generating new candidate strategies.
    pub mutation_rate: f64,                // default: 0.20, range: 0.05-0.50
    /// Whether to use HDC-based behavioral descriptors.
    pub hdc_descriptors: bool,             // default: true
    /// Minimum quality threshold to enter the archive.
    pub min_quality_threshold: f64,        // default: 0.30, range: 0.10-0.60
}

/// A behavioral descriptor dimension for MAP-Elites indexing.
pub struct DescriptorDimension {
    pub name: String,
    pub min_value: f64,
    pub max_value: f64,
}

/// A cell in the MAP-Elites archive.
pub struct ArchiveCell {
    pub strategy: EvolutionaryStrategy,
    pub quality: f64,
    pub descriptors: Vec<f64>,
    pub update_count: usize,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}
```

### MAP-Elites EVOLUTION Algorithm

```
MAP-ELITES-EVOLUTION(archive, promoted_knowledge, config):
  // Phase 1: Initialize archive from existing promoted heuristics
  IF archive.is_empty():
    FOR heuristic in promoted_knowledge:
      descriptors = compute_behavioral_descriptors(heuristic)
      cell = discretize(descriptors, config.bins_per_dimension)
      archive.insert_if_better(cell, heuristic, heuristic.fitness)

  // Phase 2: Generate new candidates via mutation/crossover
  n_candidates = config.max_archive_size / 10
  FOR i in 1..n_candidates:
    parent_a = archive.random_occupied_cell().strategy
    parent_b = archive.random_occupied_cell().strategy
    child_vector = HdcVector::bundle(&[&parent_a.hdc_vector, &parent_b.hdc_vector])
    child_vector = child_vector.mutate(config.mutation_rate)
    child_fitness = evaluate_fitness(child, recent_episodes)
    descriptors = compute_behavioral_descriptors(child)
    cell = discretize(descriptors, config.bins_per_dimension)
    IF child_fitness > config.min_quality_threshold:
      archive.insert_if_better(cell, child, child_fitness)

  // Phase 3: Report archive statistics
  RETURN ArchiveReport {
    filled_cells: archive.occupied_count(),
    coverage: archive.occupied_count() / archive.total_cells(),
    qd_score: archive.sum_quality(),  // QD-score: sum of all cell qualities
  }
```

### Test Criteria for MAP-Elites

```
1. Archive insertion: higher-quality candidates replace lower-quality occupants; not vice versa.
2. Quality threshold: candidates below min_quality_threshold are never inserted.
3. Coverage growth: after 100 cycles with diverse heuristics, ≥30% of cells are occupied.
4. QD-score monotonicity: QD-score never decreases across EVOLUTION cycles.
5. Mutation diversity: child vectors have HDC similarity to parents in [0.4, 0.8].
6. Descriptor bounds: all descriptors fall within configured [min_value, max_value].
```

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Promoted entries that EVOLUTION operates on |
| [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC operations used for knowledge recombination |
| [../03-neuro/INDEX.md](../06-neuro/INDEX.md) | NeuroStore where evolved strategies are persisted |
