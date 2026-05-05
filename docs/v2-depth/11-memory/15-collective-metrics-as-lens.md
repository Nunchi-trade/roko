# Collective Metrics as Lens

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How collective intelligence is measured via a Lens Graph reading Bus and Store, the five-axis c-factor (Woolley et al. 2010), WisdomGate verification, groupthink countermeasures, the seven compounding flywheel Loops, and scaling analysis (stigmergy O(N*M) vs direct O(N^2)).

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal/Pulse duality, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (Score, Verify, Observe, React protocols), [03-GRAPH](../../unified/03-GRAPH.md) (Lens specialization, Loop pattern), [07-LEARNING](../../unified/07-LEARNING.md) (c-factor as covariate, calibration), [15-TELEMETRY](../../unified/15-TELEMETRY.md) (Lens system, StateHub projections), [11-stigmergy-as-bus](11-stigmergy-as-bus.md) (Bus-native stigmergy), [12-pheromone-mechanics-and-interference](12-pheromone-mechanics-and-interference.md) (Alpha paradox, kind system)

---

## 1. The Measurement Problem

Collective intelligence is not the sum of individual intelligences. A group of individually brilliant agents can perform worse than a group of mediocre agents if the brilliant ones talk past each other, duplicate work, or converge prematurely on a wrong answer. Conversely, a well-coordinated group of average agents can outperform individuals through complementary perspectives, error detection, and knowledge combination.

The problem is: **how do you measure whether a group of agents is coordinating well?** Activity metrics (message count, task completion rate) can be gamed. Outcome metrics (final quality) arrive too late to correct course. What is needed is a process metric that correlates with outcome quality and is computable from existing Bus and Store instrumentation.

The answer is the **c-factor** (Woolley et al. 2010) -- a collective intelligence factor analogous to Spearman's *g* for individual intelligence. The c-factor is derived from how agents coordinate, not what they produce. It is a **covariate, not an objective** -- optimizing it directly can be gamed. It gates decisions and flags process degradation but is never the optimization target itself.

---

## 2. The c-Factor Lens Graph

The c-factor measurement system is expressed as a **Lens Graph** (see [02-CELL.md](../../unified/02-CELL.md) for the Observe protocol and [03-GRAPH.md](../../unified/03-GRAPH.md) for the Lens specialization). The Lens reads Bus and Store without mutation:

```toml
# c-Factor Lens Graph.
# Read-only observation of Bus statistics and Store artifacts.
# Emits CohortMetrics Signal at window close.

[graph]
id = "c-factor-lens"
pattern = "lens"

[[cells]]
id = "turn_taking"
protocol = "observe"
description = "Compute Shannon entropy of Pulse authorship within cohort window"

[[cells]]
id = "peer_prediction"
protocol = "observe"
description = "Match peer.prediction Pulses against peer.outcome Pulses"

[[cells]]
id = "citation_reciprocity"
protocol = "observe"
description = "Analyze Signal lineage graph for bidirectional citation"

[[cells]]
id = "delivery_rate"
protocol = "observe"
description = "Compute proportion of Bus deliveries that reached intended subscribers"

[[cells]]
id = "hdc_diversity"
protocol = "observe"
description = "Compute pairwise HDC fingerprint distances across cohort Signals"

[[cells]]
id = "aggregate"
protocol = "score"
description = "Combine five axes into CohortMetrics Signal using learned weights"

[[cells]]
id = "wisdom_gate"
protocol = "verify"
description = "Check whether cohort process quality is sufficient to trust consensus"

[[edges]]
from = ["turn_taking", "peer_prediction", "citation_reciprocity", "delivery_rate", "hdc_diversity"]
to = "aggregate"

[[edges]]
from = "aggregate"
to = "wisdom_gate"
```

---

## 3. Five-Axis CohortMetrics

### 3.1 The Five Axes

| Axis | Meaning | Source | What It Catches |
|------|---------|--------|-----------------|
| `turn_taking_entropy` | How evenly turns are distributed across agents | Bus Pulse authorship counts | One agent monopolizing the floor |
| `peer_prediction_accuracy` | How well agents predict each other's outputs | Bus `peer.prediction` / `peer.outcome` Pulse matching | Poor social calibration, agents ignoring each other |
| `citation_reciprocity` | How often citations flow both ways between agents | Signal lineage graph in Store | One-way broadcast, no building on each other's work |
| `delivery_rate` | How much intended traffic reaches subscribers | Bus publish/deliver/ack/drop statistics | Communication infrastructure problems |
| `hdc_diversity` | How far apart the cohort's artifact fingerprints are in HDC space | Signal HDC fingerprints in Store, pairwise distance | Premature convergence, echo chamber |

All axes are normalized to [0, 1].

```rust
/// Five-axis collective intelligence measurement.
/// All values normalized to [0, 1].
struct CohortMetrics {
    turn_taking_entropy: f64,
    peer_prediction_accuracy: f64,
    citation_reciprocity: f64,
    delivery_rate: f64,
    hdc_diversity: f64,
}
```

### 3.2 Turn-Taking Entropy

Turn-taking entropy is the normalized Shannon entropy of Pulse authorship within a cohort window. High entropy means no single agent monopolizes the floor.

```rust
/// Compute turn-taking entropy from Pulse authorship counts.
///
/// Input: map from AgentId to number of Pulses published in window.
/// Output: normalized Shannon entropy in [0, 1].
///
/// H = -SUM p_i * ln(p_i), normalized by ln(N).
/// N = number of agents in the cohort.
fn turn_taking_entropy(authorship: &HashMap<AgentId, usize>) -> f64 {
    let total: f64 = authorship.values().map(|&c| c as f64).sum();
    if total == 0.0 { return 0.0; }

    let n = authorship.len() as f64;
    if n <= 1.0 { return 1.0; } // Single agent: trivially "equal"

    let h: f64 = authorship.values()
        .map(|&count| {
            let p = count as f64 / total;
            if p > 0.0 { -p * p.ln() } else { 0.0 }
        })
        .sum();

    h / n.ln() // Normalize to [0, 1]
}
```

**Interpretation**: Low entropy (< 0.3) means one or two agents are doing all the talking. This is not inherently bad (a leader-follower coordination mode is legitimate), but it warrants attention if the cohort is supposed to be collaborative.

### 3.3 Peer Prediction Accuracy

Each agent can emit a `peer.prediction` Pulse encoding what it expects another agent to say, ship, or conclude. The corresponding `peer.outcome` Pulse records what actually happened. Accuracy is the fraction of predictions landing within tolerance.

This axis measures **social calibration** -- whether agents understand each other well enough to predict behavior. High accuracy indicates tight coordination; low accuracy indicates agents operating independently without mutual understanding.

### 3.4 Citation Reciprocity

Reciprocity measures whether the cohort uses the Signal lineage graph as a working memory network rather than a one-way broadcast. Reciprocity rises when Agent A cites Agent B *and* Agent B later cites or builds on Agent A's work in a way that survives verification.

```rust
/// Compute citation reciprocity from the Signal lineage graph.
///
/// For each directed citation edge A->B, check if a reverse edge B->A
/// exists within the cohort window. Reciprocity = reciprocal pairs / total edges.
fn citation_reciprocity(
    citations: &[(AgentId, AgentId)],  // (citer, cited)
) -> f64 {
    let edge_set: HashSet<(&AgentId, &AgentId)> =
        citations.iter().map(|(a, b)| (a, b)).collect();

    let reciprocal = citations.iter()
        .filter(|(a, b)| edge_set.contains(&(b, a)))
        .count();

    if citations.is_empty() { return 0.0; }
    reciprocal as f64 / citations.len() as f64
}
```

### 3.5 Delivery Rate

The proportion of intended Bus deliveries that actually arrive at subscribed targets. Drops due to backpressure, auth failure, filter mismatch, or transport failure lower the rate. This measures communication health, not just throughput.

### 3.6 HDC Diversity

HDC diversity measures how spread out the cohort's Signal fingerprints are in HDC space (see [02-hdc-algebra-and-retrieval.md](02-hdc-algebra-and-retrieval.md) for HDC encoding). The point is useful diversity, not raw novelty.

```rust
/// Compute HDC diversity as mean pairwise Hamming distance, normalized.
///
/// Low diversity (< 0.3) indicates the cohort is producing highly
/// similar artifacts -- potential premature convergence.
fn hdc_diversity(fingerprints: &[HdcVector]) -> f64 {
    if fingerprints.len() < 2 { return 0.0; }

    let mut total_distance = 0.0;
    let mut count = 0;
    for i in 0..fingerprints.len() {
        for j in (i+1)..fingerprints.len() {
            total_distance += fingerprints[i].hamming_distance(&fingerprints[j]);
            count += 1;
        }
    }

    // Normalize: max Hamming distance for 10,240-bit vectors is 10,240.
    // Expected distance for random vectors is ~5,120 (half the bits differ).
    // Normalize so random = 1.0.
    total_distance / (count as f64 * 5120.0)
}
```

---

## 4. Computing the c-Factor

The c-factor is a weighted score over the five axes. Weights are **learned from cohort outcomes**, not hardcoded:

```rust
/// Learned weights for c-factor computation.
/// Fitted online via predict-publish-correct on cohort outcomes.
struct CohortWeights {
    turn_taking_entropy: f64,
    peer_prediction_accuracy: f64,
    citation_reciprocity: f64,
    delivery_rate: f64,
    hdc_diversity: f64,
    bias: f64,
}

/// Compute c-factor from metrics and weights.
fn c_factor(m: &CohortMetrics, w: &CohortWeights) -> f64 {
    w.turn_taking_entropy * m.turn_taking_entropy
        + w.peer_prediction_accuracy * m.peer_prediction_accuracy
        + w.citation_reciprocity * m.citation_reciprocity
        + w.delivery_rate * m.delivery_rate
        + w.hdc_diversity * m.hdc_diversity
        + w.bias
}
```

### 4.1 Online Weight Learning

The weight learner subscribes to `cohort.completed` Pulses on the Bus. Each observation carries the measured metrics plus the observed outcome score. The learner updates weights via simple gradient descent:

```rust
/// Online weight learner for c-factor.
/// Subscribes to cohort.completed Pulses and adjusts weights
/// via predict-publish-correct (see [02-CELL.md]).
fn update_weights(
    weights: &mut CohortWeights,
    observation: &CohortObservation,
    learning_rate: f64,
) {
    let prediction = c_factor(&observation.metrics, weights);
    let error = observation.outcome_score - prediction;

    weights.turn_taking_entropy += learning_rate * error * observation.metrics.turn_taking_entropy;
    weights.peer_prediction_accuracy += learning_rate * error * observation.metrics.peer_prediction_accuracy;
    weights.citation_reciprocity += learning_rate * error * observation.metrics.citation_reciprocity;
    weights.delivery_rate += learning_rate * error * observation.metrics.delivery_rate;
    weights.hdc_diversity += learning_rate * error * observation.metrics.hdc_diversity;
    weights.bias += learning_rate * error;
}
```

Different task families can legitimately induce different weight shapes. A research cohort might weight HDC diversity highly; a code review cohort might weight peer prediction accuracy.

### 4.2 c-Factor as Covariate, Not Objective

**This is the most important design decision in the entire coordination measurement system.**

The c-factor is a **covariate** -- a measured property that should move with better coordination. It is NOT the optimization target. The operational rule:

1. Observe c-factor and outcome together.
2. If c-factor drops AND outcomes drop: intervene on process variables.
3. If c-factor drops BUT outcomes stay stable: log the change, do not perturb.
4. NEVER reward agents for increasing c-factor directly.

Why? Because optimizing c-factor directly can be gamed: agents can pad turn counts, manufacture reciprocal citations, or generate diverse-but-useless artifacts. The objective is always task quality on work sampled by difficulty.

---

## 5. WisdomGate

Before a consensus Signal is finalized, the WisdomGate checks whether the inputs were broad enough to deserve aggregation. It is a **Verify Cell** (see [02-CELL.md](../../unified/02-CELL.md) for the Verify protocol):

```rust
/// WisdomGate: Verify Cell that blocks consensus formation
/// when the collective process was inadequate.
///
/// Prevents narrow, low-information cohorts from producing
/// Consensus Signals that look authoritative.
struct WisdomGate {
    /// Minimum turn-taking entropy. Default: 0.4.
    min_turn_taking_entropy: f64,
    /// Minimum peer prediction accuracy. Default: 0.3.
    min_peer_prediction_accuracy: f64,
    /// Minimum citation reciprocity. Default: 0.2.
    min_citation_reciprocity: f64,
    /// Minimum HDC diversity. Default: 0.3.
    min_hdc_diversity: f64,
    /// Maximum lineage overlap (prevents echo-chamber consensus). Default: 0.7.
    max_lineage_overlap: f64,
    /// Maximum share of Pulses from any single sender. Default: 0.5.
    max_sender_share: f64,
}

impl WisdomGate {
    fn verify(&self, metrics: &CohortMetrics, cohort_stats: &CohortStats) -> Verdict {
        if metrics.turn_taking_entropy < self.min_turn_taking_entropy {
            return Verdict::Reject("Turn distribution too uneven");
        }
        if metrics.hdc_diversity < self.min_hdc_diversity {
            return Verdict::Reject("Artifact diversity too low -- potential groupthink");
        }
        if cohort_stats.max_sender_share > self.max_sender_share {
            return Verdict::Reject("Single agent dominates -- consensus not collective");
        }
        if cohort_stats.lineage_overlap > self.max_lineage_overlap {
            return Verdict::Reject("Lineage too homogeneous -- broaden evidence base");
        }
        Verdict::Accept
    }
}
```

When WisdomGate rejects, the consensus Signal is not formed. The cohort is asked to widen its evidence base before trying again.

---

## 6. Groupthink Countermeasures

Optimizing for collective intelligence can still produce groupthink if the cohort converges too quickly or too uniformly. Four structural countermeasures keep the cohort honest:

### 6.1 Alpha Pheromone Paradoxical Decay

The Alpha kind (see [12-pheromone-mechanics-and-interference.md](12-pheromone-mechanics-and-interference.md)) already has paradoxical confirmation -- more confirmation means faster decay. This structural property means that widely agreed-upon "edges" naturally fade, preventing the group from locking onto a consensus prematurely.

### 6.2 Contrarian Retrieval

When the Compose protocol (see [02-CELL.md](../../unified/02-CELL.md)) assembles context for an agent, it includes **15% opposing Signals** -- Signals that contradict the current consensus direction. These are retrieved from Store by finding Signals with high HDC distance from the current working set:

```rust
/// Contrarian retrieval: inject opposing viewpoints into context.
///
/// Retrieve Signals from Store that are maximally distant in HDC space
/// from the cohort's current working set. Include 15% contrarian Signals
/// in the assembled context.
fn contrarian_retrieval(
    working_set: &[Signal],
    store: &Store,
    contrarian_fraction: f64,  // default: 0.15
) -> Vec<Signal> {
    let centroid = hdc_centroid(working_set);
    let contrarians = store.query_dissimilar(&centroid, /* limit */ 10);

    let total_slots = working_set.len();
    let contrarian_slots = (total_slots as f64 * contrarian_fraction).ceil() as usize;

    contrarians.into_iter().take(contrarian_slots).collect()
}
```

### 6.3 Outsider Injection

Route the task through an agent with low lineage overlap so the cohort sees an outside perspective before consensus locks. This is a Route Cell decision: when the c-factor Lens detects low HDC diversity, the router selects an agent that has NOT been part of the cohort's discussion.

### 6.4 Minority Report Preservation

Keep dissenting Signals alive longer than majority Signals. Signals that contradict the consensus receive a demurrage discount -- they decay slower than confirming Signals. This ensures alternative hypotheses are not starved out by the majority's reinforcement advantage.

---

## 7. The Seven Compounding Flywheel Loops

Seven feedback Loops compound coordination improvement. Each is a Loop Graph (see [03-GRAPH.md](../../unified/03-GRAPH.md)):

| Loop | What Compounds | Failure Signal |
|------|---------------|----------------|
| **Demurrage-weighted retrieval** | Usage calibrates attention cost and reward. The system keeps what is unique and useful. | Warm-tier content grows without steady state. |
| **Heuristic calibration** | More trials tighten uncertainty and improve downstream decisions. | Premature convergence -- high-confidence heuristics stop facing challenge. |
| **HDC codebook cleanup** | More exemplars improve similarity search and consensus hit-rate. | Codebook pollution -- noisy entries degrade discriminability. |
| **c-factor feedback** | Better process improves output quality and learning quality. | System learns to prefer easy tasks or flatter cohorts. |
| **Playbook distillation** | Episodes compress into reusable playbooks and meta-playbooks. | Overcompression -- playbooks lose context that made them valid. |
| **Cross-deployment heuristic commons** | Imported heuristics create shared calibration across deployments. | Stale imports -- shared heuristic no longer fits local deployment. |
| **Plugin ecosystem** | Each plugin increases value; each user increases incentive to build plugins. | Integration friction -- plugin surface leaks complexity. |

These Loops are not independent. Better commons improves c-factor. Better c-factor improves heuristics. Better heuristics improve retrieval. That is the flywheel.

### 7.1 Measurement

The flywheel is only real if measured on persistent workloads. Stateless benchmarks hide compounding because they reset Store between trials.

**North-star metric**: Mean time to first successful PR on a new codebase. This depends on all seven Loops.

**Anti-metrics** (should NOT grow without quality gains):
- Warm-tier episode count should reach steady state
- Heuristics with fewer than 3 confirmations should shrink over time
- Mean lineage depth per response should not increase unless quality improves

---

## 8. Scaling Analysis

### 8.1 Stigmergy O(N * M) vs Direct O(N^2)

| Agent Count (N) | Direct Comm O(N^2) | Stigmergy O(N * M), M=20 | Stigmergy Advantage |
|----------------|--------------------|--------------------------|---------------------|
| 5 | 10 | 100 | 0.1x (direct wins at small N) |
| 20 | 190 | 400 | 0.5x |
| **~50** | **1,225** | **1,000** | **Crossover point** |
| 100 | 4,950 | 2,000 | 2.5x |
| 500 | 124,750 | 10,000 | 12.5x |
| 1,000 | 499,500 | 20,000 | 25x |
| 10,000 | 49,995,000 | 200,000 | 250x |

The crossover at ~N=50 is where stigmergic coordination becomes more efficient than point-to-point communication. Below this threshold, direct communication may be simpler, but stigmergy's advantages (robustness, asynchrony, minimal agent complexity) still apply.

### 8.2 Why the Crossover Matters

M is bounded and small: 7 built-in kinds * 3 scope levels = 21 channels, plus custom kinds. Since M is fixed, coordination cost grows **linearly** with agent count. Each agent performs:
- **Deposit**: Publish one or more Pulses to Bus -> O(1) per agent
- **Sense**: Query active Pulses matching a filter -> O(M) per agent

Total coordination cost per cycle: O(N * M) = O(N) for fixed M.

### 8.3 Reed's Law Implications

Reed's Law states that the value of a group-forming network grows as O(2^N) (Reed 2001). For Roko Groups:
- Each subset of agents can potentially form a productive sub-group
- The number of productive subsets grows exponentially with N
- But coordination cost grows only linearly O(N * M)

Therefore, the **value-to-cost ratio grows exponentially** with Group size -- precisely the property needed for the compounding flywheel.

### 8.4 Practical Limits

| Factor | Practical Limit | Bottleneck |
|--------|----------------|-----------|
| Agents per Group | ~10,000 | Gossip fan-out latency |
| Pheromone field size | ~1M active Pulses | In-memory storage |
| Kind diversity | ~100 custom kinds | Configuration complexity |
| Morphogenetic convergence | ~50 agents | Convergence time > 12 hours |
| WebSocket relay connections | ~50,000 | Single relay server capacity |

---

## 9. Conditional Interventions

When the c-factor Lens detects process degradation AND outcomes are also declining, it can trigger conditional interventions via React Cells:

| Condition | Intervention |
|-----------|-------------|
| Low turn-taking entropy | Soften top-1 routing, throttle dominant senders, widen speaker set |
| Low peer prediction accuracy | Route agents into calibration pairs, emit more prediction/outcome Pulses |
| Low citation reciprocity | Require stronger citation trails before consensus Signals are accepted |
| Low delivery rate | Inspect Bus backpressure, auth failures, filter mismatches |
| Low HDC diversity | Diversify prompts, tools, or agent selection to prevent premature convergence |

The interventions are **conditional and local**. The system nudges the process only when the process is already underperforming on both c-factor and outcomes.

---

## What This Enables

1. **Measurable coordination quality**: The c-factor provides a quantitative answer to "is this group coordinating well?" that is computable from existing instrumentation without special-case data collection.
2. **Goodhart-resistant design**: By treating c-factor as a covariate rather than an objective, the system avoids the trap of optimizing for the metric at the expense of actual quality.
3. **Quality-gated consensus**: WisdomGate prevents low-quality collective processes from producing authoritative Signals. Only well-coordinated cohorts can form Consensus.
4. **Structural anti-groupthink**: Contrarian retrieval, outsider injection, Alpha paradox, and minority report preservation are built into the architecture, not bolted on as social rules.
5. **Exponential value scaling**: The O(N * M) coordination cost with O(2^N) potential group value creates a sustainable advantage as Group size grows.

## Feedback Loops

1. **Weight calibration Loop**: c-factor prediction error -> weight update -> better prediction -> better intervention targeting. This is the core predict-publish-correct Loop on the CohortWeights.
2. **WisdomGate calibration Loop**: Gate rejects low-quality consensus -> cohort widens evidence base -> higher-quality consensus -> Gate thresholds validated. If the Gate is too strict (rejects valid consensus), the thresholds relax over time.
3. **Groupthink detection Loop**: HDC diversity drops -> contrarian retrieval increases -> diversity recovers -> system returns to normal retrieval. The 15% contrarian fraction is the equilibrium maintenance mechanism.
4. **Flywheel acceleration Loop**: Better c-factor -> better heuristics -> better retrieval -> better episodes -> better playbooks -> better c-factor. Each Loop in the flywheel reinforces the others.

## Open Questions

1. **Should c-factor weights be shared across deployments?** If different task families require different weights, sharing may hurt. But bootstrapping new deployments would benefit from pre-trained weights.
2. **How should peer prediction be implemented in practice?** Agents need to explicitly emit prediction Pulses, which adds overhead. Is there a way to infer prediction accuracy from implicit behavior?
3. **Is 15% the right contrarian retrieval fraction?** Too low and groupthink persists. Too high and the cohort wastes attention on irrelevant opposing views. The optimal fraction likely depends on the task type.
4. **How should the c-factor interact with the cascade router?** If c-factor indicates poor coordination, should the router switch to a different coordination mode (e.g., from stigmergic to leader-follower)?
5. **What is the minimum cohort size for meaningful c-factor?** With 2 agents, most axes (especially turn-taking entropy) are trivially constrained. The c-factor likely needs N >= 3 to be informative.

## Implementation Tasks

1. **Implement `CohortMetrics` computation**: `crates/roko-learn/src/c_factor.rs` -- five-axis metric computation from Bus statistics and Store lineage queries.
2. **Implement `CohortWeightsLearner`**: `crates/roko-learn/src/c_factor.rs` -- online gradient descent on `cohort.completed` Pulses, persist weights to `.roko/learn/cohort-weights.json`.
3. **Implement `WisdomGate` Verify Cell**: `crates/roko-gate/src/wisdom.rs` -- threshold checks on CohortMetrics, gate consensus Signal formation.
4. **Add contrarian retrieval to Compose**: `crates/roko-compose/src/system_prompt_builder.rs` -- HDC-distance-based retrieval of opposing Signals, 15% contrarian fraction.
5. **Wire c-factor Lens into TUI**: `crates/roko-cli/src/tui/` -- cohort intelligence tile with headline c-factor, five axes, weakest link indicator.
6. **Add c-factor to HTTP API**: `crates/roko-serve/src/routes/` -- `GET /api/cohorts/{id}/metrics`, `GET /api/cohorts/{id}/c-factor`, `GET /api/cohorts/{id}/weights`.
7. **Implement conditional interventions as React Cells**: `crates/roko-learn/src/c_factor_interventions.rs` -- per-axis intervention logic, gated by outcome co-decline.
8. **Add minority report demurrage discount**: `crates/roko-core/src/demurrage.rs` -- Signals contradicting consensus receive slower decay rate (configurable discount factor).

---

## References

- Woolley, A.W. et al. 2010, "Evidence for a Collective Intelligence Factor in the Performance of Human Groups", *Science*
- Surowiecki, J. 2004, *The Wisdom of Crowds*, Doubleday
- Reed, D.P. 2001, "The Law of the Pack", *Harvard Business Review*
- Bonabeau, Dorigo & Theraulaz 1999, *Swarm Intelligence*, Oxford University Press
- Goodhart, C.A.E. 1984, "Problems of Monetary Management: The U.K. Experience", in *Monetary Theory and Practice*
