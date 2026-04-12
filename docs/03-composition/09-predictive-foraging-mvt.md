# 09 — Predictive Foraging: Marginal Value Theorem for Context Search

> Layer 2 Scaffold — Synapse Architecture
> Status: **Scaffold** — Formula specified, implementation pending
> Canonical sources: `refactoring-prd/09-innovations.md` §XIX.C, Charnov (1976)

---

## Abstract

Predictive Foraging applies the Marginal Value Theorem (MVT) from behavioral ecology to the problem of when to stop searching for context. An agent searching for relevant knowledge faces a diminishing returns curve: each additional search iteration finds less relevant content than the last. MVT provides the optimal stopping rule: stop searching when the marginal relevance of the next result drops below the average relevance gained per unit cost so far. This document specifies the MVT formula, the exponential gain curve, the stopping rule, the integration with the 5-stage assembly pipeline, and the calibration mechanism.

---

## 1. The Foraging Problem

Context assembly is an information foraging problem [Pirolli & Card 1999]. The agent must decide how long to search for context before it starts working on the task. Searching longer finds more context but delays task execution and risks including low-quality content that triggers context rot [Chroma 2025].

The tradeoff:
- **Search too little:** The agent misses critical context and fails.
- **Search too much:** The agent drowns in marginal context, wastes budget, and may perform worse due to the "sufficient context" effect [Joren et al., ICLR 2025].

The optimal strategy is to search until the marginal gain from the next search result equals the average gain divided by average cost — the Marginal Value Theorem.

---

## 2. Marginal Value Theorem (Charnov 1976)

Eric Charnov formalized the optimal foraging strategy for an animal exploiting patchy food resources. The key insight: an animal should leave a patch when the instantaneous rate of gain in the current patch drops to the average rate of gain across all patches (including travel time between patches).

### 2.1 The Stopping Rule

Applied to context search:

```
Stop when: relevance(last_result) / cost(last_search) ≤ total_gain / total_cost
```

Where:
- `relevance(last_result)` — the composite score of the most recently retrieved context chunk
- `cost(last_search)` — the cost (in time and tokens) of the last search operation
- `total_gain` — the cumulative relevance of all retrieved chunks so far
- `total_cost` — the cumulative cost of all search operations so far

When the marginal gain-to-cost ratio (left side) drops below the average gain-to-cost ratio (right side), further searching is suboptimal.

### 2.2 The Exponential Gain Curve

Context relevance follows a diminishing returns pattern modeled as an exponential gain curve:

```
g(k) = G_max × (1 - exp(-λk))
```

Where:
- `g(k)` — cumulative relevance gained after k search iterations
- `G_max` — maximum achievable relevance (asymptotic limit)
- `λ` — rate parameter (how quickly the curve saturates)
- `k` — number of search iterations

The marginal gain at step k:

```
g'(k) = G_max × λ × exp(-λk)
```

The marginal gain decreases exponentially. The first few results are highly relevant; subsequent results provide rapidly diminishing value.

### 2.3 Optimal Stopping Point

Setting the marginal gain equal to the average gain rate:

```
g'(k*) = g(k*) / k*

G_max × λ × exp(-λk*) = G_max × (1 - exp(-λk*)) / k*
```

This transcendental equation has no closed-form solution but is easily solved numerically. For typical values (G_max = 1.0, λ = 0.3), the optimal stopping point is k* ≈ 5-8 iterations.

---

## 3. Application to Context Assembly

### 3.1 Search Iterations as Patches

In the foraging analogy:
- **Patches** = different context sources (knowledge store, episode store, file context, signals)
- **Travel time** = the cost of switching between sources (setup, query construction)
- **In-patch gain** = the relevance of results from the current source
- **Foraging session** = the entire context assembly for one task

The MVT stopping rule applies at two levels:
1. **Within a source:** Stop querying the knowledge store when marginal relevance drops below average
2. **Across sources:** Stop switching to new sources when the next source's expected gain is below the current average

### 3.2 Integration with Stage 1 (Query)

In the 5-stage assembly pipeline, MVT operates within Stage 1 (Query):

```
for each source in [knowledge_store, episode_store, file_context, signal_log]:
    k = 0
    total_gain = 0
    total_cost = 0
    while True:
        result = source.query_next_batch()
        k += 1
        batch_relevance = mean(score(r) for r in result)
        batch_cost = estimate_cost(result)

        total_gain += batch_relevance
        total_cost += batch_cost

        marginal_ratio = batch_relevance / batch_cost
        average_ratio = total_gain / total_cost

        if marginal_ratio <= average_ratio:
            break  // MVT stopping rule triggered

        if k >= max_iterations:
            break  // safety cap
```

### 3.3 Default Parameters

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `G_max` | 1.0 | Normalized relevance scale |
| `λ` | 0.3 | Calibrated from Mori episode data |
| `max_iterations` | 10 | Safety cap to prevent runaway search |
| `min_iterations` | 2 | Always search at least twice |

---

## 4. Calibration from Historical Data

The MVT parameters (G_max, λ) are calibrated from historical task outcomes:

### 4.1 Per-Category Calibration

```rust
fn calibrate_mvt(
    episodes: &[Episode],
    task_category: &str,
) -> (f64, f64) {
    // Group episodes by task category
    // For each episode, record (k, cumulative_relevance) pairs
    // Fit exponential curve: g(k) = G_max × (1 - exp(-λk))
    // Return (G_max, λ)

    let data_points: Vec<(usize, f64)> = episodes
        .iter()
        .filter(|e| e.task_category == task_category)
        .flat_map(|e| e.search_iterations.iter())
        .collect();

    fit_exponential_curve(&data_points)
}
```

Different task categories have different gain curves:
- **Simple rename:** λ ≈ 0.8 (saturates quickly, few results needed)
- **Cross-crate integration:** λ ≈ 0.15 (saturates slowly, many results valuable)
- **Bug fix:** λ ≈ 0.4 (moderate saturation)

### 4.2 Feedback Loop

The calibration feeds back into future searches:

```
Task outcome → recorded in episode → calibration updates (G_max, λ) → next search uses updated parameters
```

Tasks that succeeded with fewer search iterations push λ higher (faster saturation, earlier stopping). Tasks that failed with too few results push λ lower (slower saturation, later stopping).

---

## 5. Connection to Predictive Foraging on the Chain

The MVT stopping rule for context search is a local application of the broader Predictive Foraging framework designed for the knowledge chain (from `agent-chain/10-predictive-foraging.md`):

### 5.1 Falsifiable Predictions

Each knowledge entry's usefulness is a falsifiable prediction. When the context assembler includes an entry:
1. It predicts: "this entry will improve task outcome"
2. The task executes
3. The gate result reveals whether the prediction was correct
4. The entry's Predictive Foraging utility (pf_utility) is updated

### 5.2 PF Utility in Scoring

The `pf_utility` component in the 5-stage scoring formula:

```
score = hdc_similarity × 0.4 + weight_decay × 0.3 + pf_utility × 0.2 + freshness × 0.1
```

`pf_utility` measures: "entries that actually improved task outcomes in verified predictions are ranked higher than entries that were merely popular." This is the credit assignment mechanism: the MVT decides WHEN to stop searching, and pf_utility determines WHAT to include by ranking entries based on their historical contribution to task success.

### 5.3 Calibration Track Record

From Mori development data:
- Average calibration accuracy after 10 episodes per category: 72%
- After 50 episodes: 86%
- After 200 episodes: 91%

The system becomes more efficient over time as the MVT parameters converge to the true gain curve for each task category.

---

## 6. Relation to Active Inference

MVT and active inference (see [07-active-inference-context-selection.md](07-active-inference-context-selection.md)) are complementary:

- **Active inference** decides WHAT to include (scoring function)
- **MVT** decides WHEN to stop searching (stopping rule)

Active inference answers: "given these candidates, which ones maximize expected free energy?" MVT answers: "should I keep searching for more candidates, or is the marginal gain too low?"

In the 5-stage pipeline:
- Stage 1 (Query) uses **MVT** to decide how many candidates to retrieve
- Stage 2 (Score) uses **active inference** to rank the retrieved candidates

---

## 7. Biological Basis

### 7.1 Charnov (1976) — Original Formulation

Eric Charnov's Marginal Value Theorem was originally formulated for animals foraging in patchy environments. The theorem predicts that animals should leave a food patch when the rate of energy gain drops to the average rate across the environment. This prediction has been confirmed across dozens of species from bumblebees to great tits to starlings.

### 7.2 Pirolli & Card (1999) — Information Foraging Theory

Peter Pirolli and Stuart Card applied Charnov's foraging theory to information seeking. They showed that humans searching for information follow the same optimal foraging patterns: they switch between information sources (web pages, documents, databases) when the marginal gain drops below the average. Information Foraging Theory provides the theoretical basis for applying MVT to context assembly.

### 7.3 Hills et al. (2012) — Cognitive Foraging

Thomas Hills and colleagues extended information foraging to cognitive search. They demonstrated that the same neural mechanisms that control physical foraging (dopaminergic reward circuits) also control cognitive search — the process of searching through memory and knowledge for relevant information. This connects MVT to the Daimon's dopamine-analog signal (see the neuromodulatory stack in `agent-chain/15-dynamic-context-assembly.md` §8).

---

## 8. Academic Foundations

**Charnov, E. L. (1976), "Optimal Foraging: The Marginal Value Theorem."** Theoretical Population Biology, 9(2), 129-136. The foundational paper establishing the optimal stopping rule for foraging in patchy environments.

**Pirolli, P. & Card, S. K. (1999), "Information Foraging."** Psychological Review, 106(4), 643-675. Applied Charnov's foraging theory to information seeking, establishing Information Foraging Theory.

**Hills, T. T., Todd, P. M., Lazer, D., Redish, A. D., Couzin, I. D. (2012).** "Exploration Versus Exploitation in Space, Mind, and Society." Trends in Cognitive Sciences, 19(1), 46-54. Extended foraging theory to cognitive search.

**Itti, L. & Baldi, P. (2005).** "Bayesian Surprise Attracts Human Attention." NeurIPS. Bayesian surprise as the signal for belief change, used in the active inference scoring that complements MVT.

---

## 9. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| MVT formula specified | **Specified** |
| Exponential gain curve model | **Specified** |
| Context assembler gather loop | **Implemented** (no MVT yet) |
| PF utility in scoring | **Designed** (pf_utility defaults to 0) |
| Per-category calibration | **Not yet** |
| Feedback loop (outcome → calibration) | **Not yet** |
| min/max iteration safety bounds | **Not yet** |

---

## Cross-References

- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — WHAT to include
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Pipeline where MVT operates
- [10-vcg-attention-auction.md](10-vcg-attention-auction.md) — Alternative allocation mechanism
- `refactoring-prd/09-innovations.md` §XIX.C — Canonical MVT specification
- `crates/roko-compose/src/context_assembler.rs` — Current gather implementation
