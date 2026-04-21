# Collective Intelligence Metrics: Measuring Coordination Effectiveness

> **Layer**: L3 Harness (monitoring and measurement), L4 Orchestration (collective-level
> analysis)
>
> **Synapse traits**: `Scorer` (scores cohort outputs), `Gate` (verifies cohort claims),
> `Policy` (adjusts coordination based on metrics)
>
> **Prerequisites**: `00-stigmergy-theory.md` (coordination fundamentals),
> `10-exponential-flywheel.md` (what the metrics should detect),
> `../00-architecture/01-naming-and-glossary.md` (canonical Bus/Engram/Pulse vocabulary)
>
> **See also**:
> `../../tmp/refinements/13-collective-intelligence-c-factor.md`

> **Implementation**: Specified

---

## Overview

This chapter operationalizes the c-factor refinement for coordination. The Bus is the
conversation floor, Pulses are the turns, and Engrams are the durable artifacts. The goal is
not to count activity for its own sake, but to measure whether a cohort is coordinating in a
way that improves outcomes over time.

The central idea is Woolley et al.'s collective intelligence factor, adapted to Roko's runtime:
measure the process, fit a cohort-level score, and let Policy intervene only when the score
and the task outcome move together. The c-factor is therefore a diagnostic covariate, not a
blind optimization target.

---

## Cohort Windows And Observation Units

A **cohort** is the smallest coordination unit this chapter measures: a set of agents working
together on the same plan, PRD, parent episode, or other shared objective.

A **cohort window** is the time-bounded slice used for measurement. The default window is the
smallest interval that contains the cohort's active work and closes on either:

1. `cohort.completed`
2. a timeout set by Policy or orchestration
3. a handoff to a new cohort window with the same task lineage

Observation happens at three levels:

1. **Pulse turn**: one authored Pulse on a cohort Topic.
2. **Artifact turn**: one Engram created, cited, or revised inside the window.
3. **Outcome label**: one completion or evaluation label that marks the window for learning.

The join key is a shared cohort identity on Bus and Substrate records. Pulses carry the
`cohort_id`, `topic`, `author`, and sequence number. Engrams carry lineage, provenance, and
the same cohort identity when they are part of the window. This makes the metrics computable
from instrumentation rather than from manual annotation.

---

## Five-Axis CohortMetrics

The c-factor is computed from five normalized axes. Each axis is derived from Bus or Substrate
instrumentation that already exists or is implied by the chapter model.

| Axis | Meaning | Primary source |
|---|---|---|
| `turn_taking_entropy` | How evenly turns are distributed across agents in the window | Bus Pulse authorship and delivery stats |
| `peer_prediction_accuracy` | How well agents predict each other's outputs | Bus `peer.prediction` / `peer.outcome` Pulses plus cohort labels |
| `citation_reciprocity` | How often citations flow both ways across validated Engrams | Substrate provenance and citation edges |
| `delivery_rate` | How much of the intended traffic reaches the intended subscribers | Bus publish, deliver, ack, retry, drop, and backpressure stats |
| `hdc_diversity` | How far apart the cohort's artifact fingerprints are in HDC space | Substrate Engram fingerprints and similarity queries |

### Turn-taking entropy

Turn-taking entropy is the normalized Shannon entropy of Pulse authorship within a window.
High entropy means no single agent monopolizes the floor. Low entropy means the cohort is
over-concentrated around one speaker or one route through Policy.

### Peer prediction accuracy

Each agent can emit a `peer.prediction` Pulse that encodes what it expects another agent to
say, ship, or conclude. The corresponding `peer.outcome` Pulse records what actually happened.
Accuracy is the fraction of predictions that land within the accepted tolerance for the
cohort's task type.

### Citation reciprocity

Substrate gives each Engram a provenance chain. Citation reciprocity measures whether the
cohort actually uses that chain as a working memory network instead of a one-way broadcast.
Reciprocity rises when Agent A cites Agent B and Agent B later cites or builds on the same
artifact in a way that survives validation.

### Delivery rate

Delivery rate is the proportion of intended Bus deliveries that actually arrive at the
subscribed targets. Drops due to backpressure, auth failure, filter mismatch, or transport
failure lower the rate. This makes delivery rate a direct measure of communication health, not
just throughput.

### HDC diversity

HDC diversity measures how spread out the cohort's Engram fingerprints are in the HDC space.
The point is not raw novelty; it is useful diversity. Cohorts with fingerprints that collapse
to the same region tend to lose perspective and converge too early.

### CohortMetrics shape

```rust
pub struct CohortMetrics {
    pub turn_taking_entropy: f64,      // [0, 1]
    pub peer_prediction_accuracy: f64, // [0, 1]
    pub citation_reciprocity: f64,     // [0, 1]
    pub delivery_rate: f64,            // [0, 1]
    pub hdc_diversity: f64,            // [0, 1]
}
```

---

## Computing The c-factor

The c-factor is a weighted score over the five axes. In the simplest form, it is a linear
model with a learned bias.

```rust
pub struct CohortWeights {
    pub turn_taking_entropy: f64,
    pub peer_prediction_accuracy: f64,
    pub citation_reciprocity: f64,
    pub delivery_rate: f64,
    pub hdc_diversity: f64,
    pub bias: f64,
}

pub fn c_factor(m: &CohortMetrics, w: &CohortWeights) -> f64 {
    w.turn_taking_entropy * m.turn_taking_entropy
        + w.peer_prediction_accuracy * m.peer_prediction_accuracy
        + w.citation_reciprocity * m.citation_reciprocity
        + w.delivery_rate * m.delivery_rate
        + w.hdc_diversity * m.hdc_diversity
        + w.bias
}
```

The weights are learned from cohort outcomes, not hardcoded as universal constants. Different
task families can legitimately induce different weight shapes.

### CohortWeightsLearner

The learner subscribes to Bus topics that label completed windows. The main input is
`cohort.completed`, which should carry the measured metrics plus the observed outcome score.
The learner updates weights online and republishes the new coefficients for observability.

```rust
pub struct CohortObservation {
    pub metrics: CohortMetrics,
    pub outcome_score: f64,
}

pub struct CohortWeightsLearner<B: Bus> {
    pub bus: std::sync::Arc<B>,
    pub weights: parking_lot::RwLock<CohortWeights>,
    pub learning_rate: f64,
}

impl<B: Bus> CohortWeightsLearner<B> {
    pub async fn run(self: std::sync::Arc<Self>) {
        let filter = TopicFilter::Exact(Topic::new("cohort.completed"));
        let mut rx = self.bus.subscribe(filter).await.unwrap();

        while let Some(pulse) = rx.recv().await {
            let Some(obs) = parse_cohort_observation(&pulse) else { continue };
            let prediction = c_factor(&obs.metrics, &self.weights.read());
            let error = obs.outcome_score - prediction;

            let mut w = self.weights.write();
            w.turn_taking_entropy += self.learning_rate * error * obs.metrics.turn_taking_entropy;
            w.peer_prediction_accuracy += self.learning_rate * error * obs.metrics.peer_prediction_accuracy;
            w.citation_reciprocity += self.learning_rate * error * obs.metrics.citation_reciprocity;
            w.delivery_rate += self.learning_rate * error * obs.metrics.delivery_rate;
            w.hdc_diversity += self.learning_rate * error * obs.metrics.hdc_diversity;
            w.bias += self.learning_rate * error;

            let _ = self.bus.publish(emit_weights_update(&*w)).await;
        }
    }
}
```

---

## Instrumentation Sources

The key point is that the metrics are derivable from existing Bus and Substrate signals.

| Metric | Bus instrumentation | Substrate instrumentation | Notes |
|---|---|---|---|
| `turn_taking_entropy` | Pulse sender counts, per-topic turn order, delivery confirmations | cohort metadata only | Counts who actually held the floor in the window |
| `peer_prediction_accuracy` | `peer.prediction` and `peer.outcome` Pulses | matching cohort labels and outcome Engrams | Measures social calibration, not raw optimism |
| `citation_reciprocity` | citation Pulses or citation tags on cohort messages | Engram provenance, lineage, and citation edges | Requires a stable artifact graph |
| `delivery_rate` | publish, deliver, ack, retry, drop, backpressure | none beyond transport metadata | Tracks whether the Bus is healthy enough to carry the cohort |
| `hdc_diversity` | fingerprint summaries on published artifacts | HDC fingerprint similarity over cohort Engrams | Uses Substrate similarity retrieval and pairwise distance |

The join logic is simple: Bus tells us what was attempted and what was seen; Substrate tells
us what persisted and how the artifacts relate. Together they are sufficient to compute the
five axes without special-case data collection.

---

## Policy, WisdomGate, And Interventions

Policy should not optimize the c-factor directly. It should treat c-factor as a measurement
that becomes actionable only when the downstream outcome is also worsening.

The operational rule is:

1. Observe c-factor and outcome together.
2. If c-factor drops and outcomes also drop, intervene on process variables.
3. If c-factor drops but outcomes stay stable, log the change and do not perturb the cohort.

### WisdomGate inputs

Before a consensus Engram is finalized, the WisdomGate checks whether the inputs are broad
enough to deserve aggregation. It consumes the same signals used by c-factor plus a few
structural thresholds.

```rust
pub struct WisdomGate {
    pub min_turn_taking_entropy: f64,
    pub min_peer_prediction_accuracy: f64,
    pub min_citation_reciprocity: f64,
    pub min_hdc_diversity: f64,
    pub max_lineage_overlap: f64,
    pub max_sender_share: f64,
}
```

### Conditional interventions

| Condition | Policy response |
|---|---|
| Low turn-taking entropy | Soften top-1 routing, throttle dominant senders, or widen the speaker set |
| Low peer prediction accuracy | Route agents into calibration pairs and emit more `peer.prediction` / `peer.outcome` Pulses |
| Low citation reciprocity | Require stronger citation trails before consensus Engrams are accepted |
| Low delivery rate | Inspect Bus backpressure, auth failures, and filter mismatches before changing cohort behavior |
| Low HDC diversity | Diversify prompts, tools, or agent selection to prevent premature convergence |

The mechanism is conditional and local. Policy nudges the process only when the process is
already underperforming.

---

## Groupthink Countermeasures

Optimizing for collective intelligence can still produce groupthink if the cohort converges
too quickly or too uniformly. The countermeasures below keep the cohort honest.

1. **Devil's-advocate Pulse**: Policy spawns a deliberately opposing Pulse on key decisions
   when diversity or reciprocity falls below threshold.
2. **Outsider injection**: Route the task through an agent with low lineage overlap so the
   cohort sees an outside view before consensus is locked.
3. **Minority report preservation**: Keep dissenting Engrams alive longer than the majority
   trail so alternative hypotheses are not starved out.
4. **WisdomGate refusal**: If the inputs are too narrow, refuse to finalize the consensus
   Engram and ask the cohort to widen its evidence base.

These are structural controls, not social theater. They prevent a high-agreement but low-
information cohort from looking healthy on the surface.

---

## Surfacing And Rollout

The c-factor should be visible everywhere operators already look.

### Dashboard tile

`roko dashboard` should show a cohort intelligence tile with the headline c-factor, the five
axes, and the current weakest link. A compact tile is enough for live operations; the detailed
breakdown belongs behind drill-down.

### API

The HTTP API should expose cohort metrics as a first-class read path, for example:

- `GET /api/cohorts/{cohort_id}/metrics`
- `GET /api/cohorts/{cohort_id}/c-factor`
- `GET /api/cohorts/{cohort_id}/weights`

### Prometheus

The same measurements should be exported as gauges and counters, including `roko.c_factor`
and per-axis metrics such as `roko.cohort_turn_taking_entropy`,
`roko.cohort_peer_prediction_accuracy`, and `roko.cohort_delivery_rate`.

### Phased rollout

1. **Metrics-only**: compute CohortMetrics from Bus and Substrate data, log the values, and
   do not change behavior.
2. **Dashboard and alerts**: surface the tile, API, and Prometheus series.
3. **Passive optimization**: fit CohortWeights online, but keep Policy read-only.
4. **Active optimization**: allow Policy to apply the conditional interventions above.

That sequence keeps the first value proposition low-risk while still moving toward closed-loop
coordination control.

---

## References

- Woolley, A. W. et al. 2010, *Science*, "Evidence for a Collective Intelligence Factor in
  the Performance of Human Groups"
- `../00-architecture/11-dual-process-and-active-inference.md` — online learning loop for weights and outcomes
- `../00-architecture/02-engram-data-type.md` — HDC fingerprints on durable artifacts
- `../00-architecture/25-attention-as-currency.md` — related attention-economy mechanisms

---

## Cross-References

- [01-naming-and-glossary](../00-architecture/01-naming-and-glossary.md) — glossary for Bus, Pulse, Engram, Topic, and related terms
- [13-collective-intelligence-c-factor](../../tmp/refinements/13-collective-intelligence-c-factor.md) — full refinement source for this chapter
- `00-stigmergy-theory.md` — coordination model feeding the cohort metrics
- `10-exponential-flywheel.md` — downstream process gains this chapter helps measure
