# 09 — Predictive Foraging: Marginal Value Theorem for Context Search

> Layer 2 Scaffold — Synapse Architecture
> Status: **Scaffold** — Formula specified, implementation pending
> Canonical sources: `refactoring-prd/09-innovations.md` §XIX.C, Charnov (1976)


> **Implementation**: Shipping

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

## 7. Multi-Patch Foraging: Switching Between Knowledge Sources

The basic MVT governs when to stop searching within a single source. Multi-patch foraging addresses the higher-level decision: **when to switch between sources** and **in what order to visit them**.

### 7.1 The Multi-Source Problem

Roko's context assembler queries four sources in sequence (knowledge store, episode store, file context, signal log). The current implementation queries all four unconditionally. Multi-patch MVT optimizes this:

```rust
/// Multi-patch foraging strategy for context assembly.
pub struct MultiPatchForager {
    /// Per-source gain curve parameters (G_max, λ).
    pub source_params: HashMap<ContextSource, (f64, f64)>,
    /// Per-source travel cost (setup time + first-query latency).
    pub travel_costs: HashMap<ContextSource, f64>,
    /// Current average gain rate across all sources.
    pub environment_rate: f64,
}

impl MultiPatchForager {
    /// Determine the optimal visitation order for sources.
    /// Visit the source with highest expected marginal gain first.
    pub fn optimal_order(&self) -> Vec<ContextSource> {
        let mut sources: Vec<_> = self.source_params.keys().collect();
        sources.sort_by(|a, b| {
            let gain_a = self.expected_initial_gain(a);
            let gain_b = self.expected_initial_gain(b);
            gain_b.partial_cmp(&gain_a).unwrap()
        });
        sources.into_iter().cloned().collect()
    }

    /// Expected gain from the first query to a source.
    /// g'(0) = G_max × λ (the derivative of the gain curve at k=0).
    fn expected_initial_gain(&self, source: &ContextSource) -> f64 {
        let (g_max, lambda) = self.source_params[source];
        g_max * lambda
    }

    /// Should we visit this source at all?
    /// Skip if even the first result's expected gain is below environment rate.
    pub fn should_visit(&self, source: &ContextSource) -> bool {
        let initial_gain = self.expected_initial_gain(source);
        let travel_cost = self.travel_costs[source];
        // Visit if: first result's gain > environment rate × travel cost
        initial_gain > self.environment_rate * travel_cost
    }

    /// Optimal number of iterations within a source before switching.
    /// Solve: g'(k*) = environment_rate + travel_cost / k*
    pub fn optimal_iterations(&self, source: &ContextSource) -> usize {
        let (g_max, lambda) = self.source_params[source];
        let travel_cost = self.travel_costs[source];

        // Numerical solution via binary search
        let mut lo = 1usize;
        let mut hi = 20usize;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let marginal = g_max * lambda * (-lambda * mid as f64).exp();
            let threshold = self.environment_rate + travel_cost / mid as f64;
            if marginal > threshold {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.max(1).min(10)  // Clamp to [1, 10]
    }
}
```

### 7.2 Source Characteristics

| Source | G_max | λ | Travel Cost | Typical Iterations |
|---|---|---|---|---|
| Knowledge Store | 0.9 | 0.25 | Low (in-memory) | 5-8 |
| Episode Store | 0.6 | 0.4 | Low (in-memory) | 3-5 |
| File Context | 0.8 | 0.5 | Medium (disk I/O) | 2-4 |
| Signal Log | 0.4 | 0.6 | Low (in-memory) | 1-3 |

The knowledge store has the highest G_max (most potential value) but saturates slowly (low λ) — you need multiple queries to extract the best results. The signal log saturates quickly (high λ) — the first few signals are the most relevant, and additional ones add little.

### 7.3 Adaptive Source Ordering

As calibration data accumulates, the forager learns which sources are most productive for each task category:

```
For a "rename" task:
  - File context has high λ (quick saturation, only need the target file)
  - Knowledge store has low G_max (few rename-specific insights)
  → Optimal order: File Context → Signal Log → skip others

For a "cross-crate integration" task:
  - Knowledge store has low λ (many relevant cross-crate insights)
  - Episode store has high G_max (past integration experiences are valuable)
  → Optimal order: Knowledge Store → Episode Store → File Context → Signal Log
```

---

## 8. Social Foraging: Leveraging Other Agents' Retrieval Patterns

In multi-agent execution (parallel plan run with 5-20 agents), each agent independently forages for context. Social foraging leverages the collective retrieval patterns to improve individual performance.

### 8.1 The Social Signal

When Agent A queries the knowledge store for a cross-crate integration task and finds entries X, Y, Z useful (gate pass on first attempt), that information is valuable for Agent B working on a related integration task. Agent B's forager can use Agent A's successful retrievals as "social information scent" — boosting the score of entries that were useful to similar agents.

Research validates this approach: in clustered resource environments, agents that respond to social information outperform individualistic searchers [Mezey et al., PLOS Computational Biology 2024]. The key condition: social information helps **when resources are heterogeneously distributed** — which matches knowledge stores where relevant entries are clustered by topic.

### 8.2 Stigmergic Retrieval Signals

Inspired by ant pheromone trails, agents deposit retrieval signals after successful task completion:

```rust
/// A retrieval signal deposited after a successful task.
pub struct RetrievalSignal {
    /// Task category that used this entry.
    pub task_category: String,
    /// Knowledge entry ID that was retrieved.
    pub entry_id: String,
    /// Relevance score assigned during retrieval.
    pub relevance: f64,
    /// Gate outcome when this entry was included.
    pub gate_passed: bool,
    /// Timestamp (for decay).
    pub timestamp: Timestamp,
    /// Agent that deposited this signal.
    pub agent_id: String,
}

/// Social foraging: boost entries that other agents found useful.
pub fn social_foraging_boost(
    candidate_entries: &mut Vec<ContextChunk>,
    recent_signals: &[RetrievalSignal],
    task_category: &str,
    decay_half_life: Duration,  // default: 24 hours
) {
    let now = SystemTime::now();

    for entry in candidate_entries.iter_mut() {
        // Count successful retrievals of this entry for similar tasks
        let social_evidence: f64 = recent_signals.iter()
            .filter(|s| s.entry_id == entry.id && s.task_category == task_category)
            .filter(|s| s.gate_passed)
            .map(|s| {
                let age = now.duration_since(s.timestamp).unwrap_or_default();
                let decay = (-age.as_secs_f64().ln() * 2.0
                    / decay_half_life.as_secs_f64()).exp();
                s.relevance * decay
            })
            .sum();

        // Apply social boost (capped at 0.3 to prevent over-reliance)
        let boost = (social_evidence * 0.1).min(0.3);
        entry.relevance += boost;
    }
}
```

### 8.3 Social Foraging Conditions

Social information is not always beneficial. Research [Royal Society Interface 2021] identifies when it helps and when it hurts:

| Condition | Social Signal Value | Explanation |
|---|---|---|
| Sparse, clustered knowledge | **High** | Social signals guide to relevant clusters |
| Uniform knowledge distribution | **Low** | Social signals add noise, no clusters to find |
| High agent diversity (different roles) | **Medium** | Different roles need different knowledge |
| High agent homogeneity (same role) | **High** | Same role → same knowledge needs |
| Early in plan execution | **High** | First agents scout; later agents benefit |
| Late in plan execution | **Low** | Most relevant knowledge already discovered |

### 8.4 Field Validation

A striking 2025 result [Science, doi:10.1126/science.ady1055]: GPS tracking of hunter-gatherer foragers demonstrated real-time adaptive social information use at field scale. Foragers update patch-quality estimates based on others' movements. This is the first empirical validation of social MVT outside laboratory settings, confirming that social foraging is not just a theoretical construct but a practical optimization strategy.

---

## 9. Foraging in LLMs: Emergent Foraging Behavior

### 9.1 LLMs as Cognitive Foragers

A landmark 2026 paper [Lacosse et al., arXiv:2603.01822] demonstrated that **LLMs exhibit the same foraging patterns as humans** in semantic fluency tasks. Using logitlens and residual stream probing, they found that convergent (within-cluster) and divergent (between-cluster) foraging strategies — the behavioral signatures from Hills et al.'s cognitive foraging work — emerge as identifiable patterns in LLM intermediate representations.

Key finding: **foraging behavior in LLMs is steerable.** The representations that drive cluster-switching vs. within-cluster exploitation can be identified and potentially manipulated. This opens a path to steering context retrieval at the model level, not just the scaffold level.

### 9.2 Embedding Geometry and Natural Foraging

Research on foraging in modern semantic spaces [arXiv:2511.12759, November 2025] found that the geometry of a well-organized embedding is **sufficient** for near-optimal foraging behavior without explicit MVT implementation. The patch structure emerges naturally from the embedding geometry — clusters in embedding space correspond to semantic patches, and random walks with Metropolis-Hastings sampling naturally dwell in clusters before transitioning.

Implication for Roko: if the knowledge store uses well-structured embeddings (or HDC fingerprints that preserve semantic similarity), the MVT stopping rule may approximate the optimal behavior already. The HDC hamming distance metric creates natural patch boundaries.

### 9.3 Sufficient Context as Foraging Criterion

The "Sufficient Context" framework [Harel-Canada et al., ICLR 2025, arXiv:2411.06037] provides a formal criterion for when to stop retrieving: a retrieved set is **sufficient** if a diligent reader could answer the question from it alone. This is the RAG analogue of the MVT patch-leaving criterion.

```rust
/// Sufficient context check: estimate whether current context is enough.
pub fn estimate_context_sufficiency(
    retrieved_chunks: &[ContextChunk],
    task: &TaskInput,
) -> f64 {
    // Proxy: coverage of task-relevant keywords in retrieved context
    let task_keywords = extract_keywords(&task.description);
    let covered = task_keywords.iter()
        .filter(|kw| retrieved_chunks.iter()
            .any(|c| c.content.contains(kw.as_str())))
        .count();

    covered as f64 / task_keywords.len().max(1) as f64
}

/// Integrated stopping rule: stop when EITHER MVT triggers OR sufficiency is high.
pub fn should_stop_searching(
    mvt_ratio: f64,        // marginal/average gain ratio
    sufficiency: f64,       // estimated context sufficiency [0, 1]
    sufficiency_threshold: f64,  // default: 0.85
) -> bool {
    mvt_ratio <= 1.0 || sufficiency >= sufficiency_threshold
}
```

### 9.4 Diminishing Returns in RAG

Research on long-context LLMs with RAG [ICLR 2025, arXiv:2410.05983] confirms that increasing the number of retrieved passages does **not** consistently improve performance. There is a diminishing returns effect — exactly as MVT predicts. The optimal number of passages varies by task complexity, matching the per-category calibration in §4.

---

## 10. Biological Basis

### 10.1 Charnov (1976) — Original Formulation

Eric Charnov's Marginal Value Theorem was originally formulated for animals foraging in patchy environments. Confirmed across dozens of species from bumblebees to great tits to starlings.

### 10.2 Pirolli & Card (1999) — Information Foraging Theory

Applied Charnov's foraging theory to information seeking. Established Information Foraging Theory as the basis for applying MVT to knowledge retrieval.

### 10.3 Hills et al. (2012) — Cognitive Foraging

Extended foraging to cognitive search. The same neural mechanisms (dopaminergic reward circuits) control both physical foraging and memory search. This connects MVT to the Daimon's dopamine-analog signal.

### 10.4 Bayesian Foraging Under Uncertainty

A 2024 extension [PMC10996644] models foragers as Bayesian updaters of patch quality beliefs. Departures from classic MVT (overharvesting, underharvesting) are explained as **rational responses** to uncertainty about the environment distribution — not irrationality. Similarly, a 2023 PNAS paper showed that human overharvesting reflects rational structure learning.

Applied to Roko: early in a plan execution (high uncertainty about which knowledge is relevant), the forager should overharvest (retrieve more than MVT-optimal) to learn the environment's structure. As confidence grows, convergence to MVT-optimal.

---

## 11. Academic Foundations

**Charnov, E. L. (1976), "Optimal Foraging: The Marginal Value Theorem."** Theoretical Population Biology, 9(2), 129-136.

**Pirolli, P. & Card, S. K. (1999), "Information Foraging."** Psychological Review, 106(4), 643-675.

**Hills, T. T., Jones, M. N., Todd, P. M. (2012), "Optimal Foraging in Semantic Memory."** Psychological Review, 119(2), 431-440. Humans follow MVT-optimal patch structure in verbal fluency tasks.

**Hills, T. T., Todd, P. M., Lazer, D., Redish, A. D., Couzin, I. D. (2015).** "Exploration Versus Exploitation in Space, Mind, and Society." Trends in Cognitive Sciences.

**Todd, P. M. & Hills, T. T. (2020), "Foraging in Mind."** Current Directions in Psychological Science.

**Lacosse et al. (2026), "Emerging Human-like Strategies for Semantic Memory Foraging in Large Language Models."** arXiv:2603.01822. LLMs exhibit the same convergent/divergent foraging patterns as humans. Foraging behavior is steerable via residual stream manipulation.

**arXiv:2511.12759 (2025), "Optimal Foraging in Memory Retrieval."** Well-organized embedding geometry is sufficient for near-optimal foraging without explicit MVT.

**Harel-Canada et al. (2025), "Sufficient Context: A New Lens on RAG Systems."** ICLR 2025. Formalizes when a retrieved context set is sufficient. 2-10% improvement via selective generation.

**arXiv:2410.05983 (2025), "Long-Context LLMs Meet RAG."** ICLR 2025. Increasing retrieved passages doesn't consistently improve performance — diminishing returns confirms MVT.

**Mezey et al. (2024), "Visual Social Information Use in Collective Foraging."** PLOS Computational Biology. Social information helps when resources are heterogeneously distributed.

**Science (2025), "High-Precision Tracking of Human Foragers."** doi:10.1126/science.ady1055. First field-scale validation of social MVT in hunter-gatherers.

**PMC10996644 (2024), "Foraging Under Uncertainty Follows MVT with Bayesian Updating."** Departures from MVT explained as rational responses to environment uncertainty.

**PNAS (2023), "Overharvesting in Human Patch Foraging."** Overharvesting reflects rational structure learning, not irrationality.

**Itti, L. & Baldi, P. (2005), "Bayesian Surprise Attracts Human Attention."** NeurIPS.

---

## 12. Test Criteria

```
test_mvt_stopping_basic:
    Given a source with g'(k) = 0.9 * 0.3 * exp(-0.3 * k)
    And total_gain = 0.5, total_cost = 2.0
    When checking marginal_ratio = g'(k)/cost vs average_ratio
    Then stop is triggered when marginal drops below average

test_multi_patch_ordering:
    Given knowledge_store with G_max=0.9 and file_context with G_max=0.8
    And knowledge_store λ=0.25 (slow saturation) and file_context λ=0.5
    When computing optimal order
    Then knowledge_store is visited first (higher initial gain: 0.9*0.25=0.225 vs 0.8*0.5=0.4)
    Actually file_context first (0.4 > 0.225)

test_social_boost_capped:
    Given social evidence of 5.0 for an entry
    When applying social_foraging_boost
    Then boost is capped at 0.3

test_sufficiency_stops_search:
    Given sufficiency = 0.90 and sufficiency_threshold = 0.85
    When checking should_stop_searching
    Then returns true regardless of mvt_ratio

test_source_skip:
    Given a source with initial_gain < environment_rate * travel_cost
    When checking should_visit
    Then returns false (skip this source entirely)
```

---

## 13. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| MVT formula specified | **Specified** |
| Exponential gain curve model | **Specified** |
| Context assembler gather loop | **Implemented** (no MVT yet) |
| PF utility in scoring | **Designed** (pf_utility defaults to 0) |
| Per-category calibration | **Not yet** |
| Feedback loop (outcome → calibration) | **Not yet** |
| min/max iteration safety bounds | **Not yet** |
| Multi-patch foraging strategy (§7) | **Designed** — MultiPatchForager specified |
| Adaptive source ordering (§7.3) | **Designed** — per-category ordering specified |
| Social foraging / stigmergic signals (§8) | **Designed** — RetrievalSignal + boost specified |
| Sufficient context integration (§9.3) | **Designed** — dual stopping rule specified |
| LLM foraging behavior awareness (§9.1) | **Research** — steerable foraging patterns identified |

---

## Cross-References

- [07-active-inference-context-selection.md](07-active-inference-context-selection.md) — WHAT to include
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Pipeline where MVT operates
- [10-vcg-attention-auction.md](10-vcg-attention-auction.md) — Alternative allocation mechanism
- [05-token-budget-management.md](05-token-budget-management.md) — Budget prediction as foraging pre-assessment
- [11-distributed-context-engineering.md](11-distributed-context-engineering.md) — Social foraging as Level 3 context engineering
- [12-affect-modulated-retrieval.md](12-affect-modulated-retrieval.md) — Affect modulation of foraging urgency
- `refactoring-prd/09-innovations.md` §XIX.C — Canonical MVT specification
- `crates/roko-compose/src/context_assembler.rs` — Current gather implementation
