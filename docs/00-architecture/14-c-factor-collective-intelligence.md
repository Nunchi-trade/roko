# C-Factor: Collective Intelligence

> **Abstract:** This chapter documents a broader target-state c-factor doctrine for Roko collectives. Shipping code already computes c-factor-like summaries and uses `CFactorPolicy` as a routing signal, but the Bus/Substrate-wide measurement and intervention story described below is still more ambitious than the current implementation. See also [01-naming-and-glossary](./01-naming-and-glossary.md) and [tmp/refinements/13-collective-intelligence-c-factor.md](../../tmp/refinements/13-collective-intelligence-c-factor.md).
>
> **Implementation status**: `CFactorPolicy` exists in `roko-core` and is wired into the routing stack as a live signal. The broader c-factor doctrine described here (continuous Woolley-style measurement, Bus/Substrate statistics, conditional Policy intervention) is **target-state**. Current recommendation: treat c-factor as an observability metric first, a control input second.

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [06-synapse-traits](./06-synapse-traits.md), [12-five-layer-taxonomy](./12-five-layer-taxonomy.md), [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md), [01-naming-and-glossary](./01-naming-and-glossary.md)
**Key sources**:
- `tmp/refinements/13-collective-intelligence-c-factor.md` — canonical refinement source for this chapter
- `docs/00-architecture/01-naming-and-glossary.md` — authoritative naming map for `Engram`, `Pulse`, `Bus`, `Topic`, `Datum`, and `TopicFilter`
- `/Users/will/dev/nunchi/roko/refactoring-prd/00-overview.md` — original collective-intelligence framing
- `/Users/will/dev/nunchi/roko/roko/docs/00-architecture/13-cognitive-cross-cuts.md` — Neuro, Daimon, and Dreams injection points

---

## Abstract

Woolley et al. (2010) showed that group performance across varied tasks loads onto a single collective factor, c. The important result is not that larger groups are automatically better. The important result is that process matters: how turns are shared, how well members predict one another, how often they cite one another correctly, how open the channel is, and how diverse the working set is.

Roko has partial observability into some of those process variables today, and the target-state architecture would make the rest explicit through Bus and Substrate artifacts. This chapter defines c-factor as a cohort-level diagnostic, then shows how the Policy layer could eventually react to it without turning it into a brittle single-objective reward.

The key design choice is deliberate: c-factor is a covariate and a diagnostic, not a direct objective. Near-term, treat it as an observability metric, or in public-facing language, a coordination-health signal. Once the signal matures, Policy can optionally use it as an intervention input when low c-factor coincides with degraded task results. That keeps the system from gaming the metric by routing easy work, suppressing dissent, or narrowing task scope.

---

## 1. The Research Foundation

### 1.1 Woolley et al. and the c-factor result

Woolley et al. found that a single collective factor predicts group performance across many tasks. The result matters because it points away from mean IQ and toward process variables that can actually be instrumented:

1. Turn-taking equality
2. Social perceptiveness
3. Coordination quality
4. Diversity of perspective
5. Shared channel discipline

Roko can measure those variables directly from Bus and Substrate traffic. That makes the runtime a live testbed for c-factor, not just a metaphorical analogy.

### 1.2 Why the Bus and Substrate are enough

The Bus records who spoke, when, to whom, and whether the turn was delivered. The Substrate records what became durable, what was cited, what was reused, and what survived verification. In practice that is enough to compute the observable parts of collective intelligence:

- turn-taking from Pulse authorship and delivery timing
- social perceptiveness from peer-prediction error
- trust calibration from citation reciprocity and later gate survival
- channel openness from delivery confirmation and subscription reach
- cognitive diversity from HDC distance across cohort artifacts

No separate telemetry plane is required. The measurement falls out of the architecture if the Bus and Substrate are already authoritative.

### 1.3 The cohort unit

Define a cohort as a set of agents working on a shared plan, task family, PRD, or parent episode during a bounded window. Cohorts are the unit of measurement because c-factor is about group process, not isolated agent skill.

That matters for two reasons:

- it keeps the metric local enough to drive policy
- it keeps the metric comparable across domains by normalizing to a cohort window

---

## 2. C-Factor: The Reporting Metric

### 2.1 Definition

c-factor is the reported scalar for a cohort over a window. It is computed from the cohort's measured process variables and normalized outcome signals.

```
c_factor(cohort, window) = f(CohortMetrics, CohortWeights)
```

In the target-state design, the learned scalar is a weighted combination of the five process variables plus bias. The weights would be fitted from observed cohort outcomes instead of being hand-tuned.

### 2.2 The five process variables

| Variable | Agent analog | Measured from |
|---|---|---|
| Turn-taking equality | How evenly the cohort shares turns | Pulse authorship entropy and sender share on the Bus |
| Social perceptiveness | How well members predict each other's outputs | `peer.prediction` vs `peer.outcome` residuals |
| Trust calibration | How often citations are useful and verified | citation reciprocity and downstream gate survival in the Substrate |
| Channel openness | How much intended traffic is actually delivered | Bus delivery confirmation and subscriber reach |
| Cognitive diversity | How different the cohort's working set is | HDC distance across cohort Engrams |

These are the same five signals described in the refinement source, expressed in runtime terms that the Bus and Substrate can already observe.

### 2.3 The metric is continuous

In the target-state design, c-factor is not a one-off benchmark. It is tracked continuously on a rolling cadence:

1. collect Bus and Substrate events for the cohort window
2. derive process variables from those events
3. fit or refresh the learned scalar
4. publish the current c-factor value
5. compare it with task outcomes and Policy interventions

That continuous loop matters because group process changes faster than coarse reporting cycles. A cohort can drift into echo-chamber behavior in minutes, not quarters.

### 2.4 CohortMetrics

```rust
pub struct CohortMetrics {
    pub turn_taking_entropy: f64,      // normalized by cohort size
    pub peer_prediction_accuracy: f64, // 0..1
    pub citation_reciprocity: f64,     // 0..1
    pub delivery_rate: f64,            // 0..1
    pub hdc_diversity: f64,            // 0..1
}
```

The metrics are intentionally simple. They are meant to be computed from existing runtime evidence, not inferred from hidden latent state.

---

## 3. C-Score: The Optimization Metric

### 3.1 Learned scalar

c-score is the target-state learned mapping from `CohortMetrics` to a scalar that predicts cohort outcome quality. In practice, the docs use c-factor and c-score closely together because both refer to the same measurement surface: one is the published metric, the other is the fitted model behind it.

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

### 3.2 CohortWeightsLearner

The weights are not static. They are fitted online from cohort outcomes.

```rust
pub struct CohortWeightsLearner<B: Bus> {
    pub bus: Arc<B>,
    pub weights: parking_lot::RwLock<CohortWeights>,
    pub learning_rate: f64,
}
```

The learner subscribes to cohort-completion Pulses, joins them with Substrate outcomes, and updates weights by gradient step or another online learner. The important point is architectural: the scalar is learned from evidence, not declared by fiat.

### 3.3 Why a learned scalar is useful

The learned score gives Policy a compact signal. It is easier to alert on, compare across cohorts, and trend over time than a raw vector of process variables. But the raw vector still matters, because it explains why the score moved.

That is the operational split:

- `CohortMetrics` explain
- `CohortWeights` adapt
- c-factor reports
- c-score predicts

Near-term, that means dashboards, alerts, and operator review should lead. Automatic policy
actuation belongs behind explicit evidence that the signal is stable enough to govern runtime
behavior.

---

## 4. Four Diagnostic Signals

The original research story had a small number of strong process predictors. In Roko those become five process variables, but the control logic still groups them into four diagnostic families: turn-taking, perspective-taking, trust, and diversity.

### 4.1 Turn-taking equality

If one sender dominates the cohort floor, the group is not really collective. The Bus makes this visible immediately through sender-share concentration and authorship entropy.

Symptoms of low turn-taking equality:

- one agent dominates Pulses
- other agents contribute only after repeated prompting
- cohort outcome quality tracks a single voice rather than a shared process

### 4.2 Social perceptiveness

Social perceptiveness is the ability to predict what another agent will say or do next. In runtime terms, it becomes prediction accuracy on `peer.prediction` Pulses compared with later `peer.outcome` Pulses.

This is the clearest place for heuristic calibration. Heuristic models of teammates are first-class knowledge objects, and the system should learn from their misses instead of hiding them.

### 4.3 Trust calibration

Trust is not a social vibe; it is a measurable citation relation. If an agent cites an Engram that later fails verification, that citation should reduce trust on the relevant topic. If the citation survives and helps a later task, trust should increase.

That gives the system a structural way to distinguish:

- useful reuse
- speculative reuse
- cargo-cult reuse

### 4.4 Channel openness

Channel openness measures whether intended subscribers actually receive the Pulses they need. Delivery rate matters because a cohort with excellent local reasoning can still fail if the Bus drops traffic, backpressure dominates, or auth rules prevent delivery.

This is where deployment observability matters. The metric should show up alongside normal service health, not in a separate research notebook.

### 4.5 Cognitive diversity

Cognitive diversity is measured as distance across cohort HDC clouds. If every agent's working set converges to the same region, the cohort is over-coupled and likely brittle.

HDC diversity is the clearest structural hedge against monoculture because it measures whether the cohort is drawing from genuinely different semantic neighborhoods.

---

## 5. Collective Calibration: The 31.6× Heuristic

### 5.1 What survives from the heuristic

The headline scaling story is still useful: more independently verified signals should accelerate learning. But the chapter no longer depends on a single speedup claim. The live system uses continuous c-factor measurement and online weight fitting instead.

### 5.2 Policy levers

Policy should not optimize c-factor blindly. It should use c-factor as a covariate and only intervene when low c-factor coincides with poor outcomes. The main levers are:

1. Turn-taking temperature
2. Peer-prediction calibration
3. Trust update rates
4. Delivery and retry policy
5. Diversity pressure

The point is to adjust process, not to sandbag the metric.

### 5.3 WisdomGate

Before a consensus artifact is finalized, it should pass a WisdomGate. The gate encodes the aggregation conditions that make group consensus meaningful rather than merely loud.

```rust
pub struct WisdomGate {
    pub min_hdc_diversity: f64,
    pub max_lineage_overlap: f64,
    pub max_sender_share: f64,
    pub aggregator: Box<dyn Aggregator>,
}
```

The four classical conditions map cleanly:

- diversity of opinion -> HDC diversity
- independence -> low lineage overlap
- decentralization -> low sender concentration
- aggregation -> a chosen aggregation method

### 5.4 Aggregation methods

Aggregation should be an explicit operator, not an implicit average.

1. Bundle
2. Bind
3. Weighted bundle
4. Cleanup to codebook

Bundle is useful when the cohort is already well aligned. Bind is useful when provenance matters. Weighted bundle is useful when the cohort has reliable trust priors. Cleanup to codebook is useful when the output must land in existing vocabulary.

### 5.5 Anti-groupthink primitives

Optimizing for c-factor can overshoot into groupthink if the system is careless. Three structural countermeasures keep the collective honest:

- Devil's-advocate Pulse: emit an explicit opposing view on consensus topics
- Outsider injection: route some work to an agent with zero lineage overlap
- Minority report preservation: retain dissenting artifacts longer, with softer demurrage on the Substrate

Those are not rhetorical devices. They are runtime policies that keep the cohort from collapsing into self-confirmation.

### 5.6 Cross-synergies

The strongest synergies are with the primitives that already shape runtime learning:

- HDC enables diversity-aware routing and meaningful similarity search
- Demurrage keeps dominant voices from hoarding attention
- Heuristics and falsifiers turn peer-models into measurable prediction systems
- Deployment observability surfaces `roko.c_factor` next to latency, errors, and cost

---

## 6. C-Factor in the Synapse Architecture

### 6.1 Where c-factor is computed

c-factor sits at the intersection of L0 runtime telemetry and L4 orchestration policy. The raw inputs are collected from:

- Bus delivery and subscription behavior
- Substrate lineage, retrieval, and gate outcomes
- cohort membership and task alignment
- HDC distance across durable artifacts

The runtime then folds those signals into a continuous cohort score.

### 6.2 Data flow

1. Bus Pulses record turns, predictions, outcomes, and cohort completion
2. Substrate Engrams record durable artifacts, citations, and lineage
3. the learner computes `CohortMetrics`
4. `CohortWeightsLearner` updates the scalar model
5. Policy reads the current c-factor and acts on it conditionally

### 6.3 Surfacing

c-factor should appear in three places:

- TUI and dashboard tiles for operators
- HTTP API for external control and automation
- metrics export, including `roko.c_factor`, for observability stacks

That makes the metric useful both as a research signal and as a production health indicator.

### 6.4 c-factor as a diagnostic

The chapter's central constraint is worth repeating: c-factor is a diagnostic covariate, not the direct target.

Good use:

- c-factor falls and task outcomes fall, so Policy intervenes
- c-factor stays flat while outcomes improve, so the cohort is probably learning more efficiently

Bad use:

- c-factor falls but Policy suppresses hard work to make the number look better
- c-factor rises because the cohort only accepts easy tasks

The metric should help the system see process quality, not hide it.

---

## 7. Comparison with Existing Collective Metrics

Many systems can count completions or average ratings. That is not enough. c-factor is load-bearing because it combines process observability with durable evidence.

The differentiators are:

1. content-addressed artifacts in the Substrate
2. delivery-confirmed turns on the Bus
3. HDC-based semantic diversity
4. online learning over cohort outcomes

That composition is what makes the metric operational rather than decorative.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Woolley et al. 2010, Science 330(6004) | Collective intelligence factor in human groups |
| Surowiecki 2004 | Diversity, independence, decentralization, aggregation |
| Metcalfe 2013, Computer 46(12) | Network value scaling intuition |
| Grassé 1959 | Stigmergy through environmental modification |
| Parunak 2006 | Digital stigmergy in multi-agent systems |
| Dorigo et al. 2000 | Emergent coordination under local rules |
| Bonabeau et al. 1999 | Self-organization and specialization |
| Beer 1972 | Recursive organizational intelligence |
| Active-inference literature | Online calibration from prediction error |

---

## Current Status and Gaps

### 8.1 Implementation phases

1. Metrics-only
2. Dashboard tile and alerts
3. Passive optimization
4. Conditional Policy actuation
5. Cross-cohort scaling

### 8.2 Practical gaps

- cohort extraction needs consistent task-family labeling
- Bus delivery confirmation needs stable instrumentation
- HDC distances need to be cheap enough for frequent recomputation
- policy actuation should remain conditional on outcomes, not c-factor alone

### 8.3 What is already implied

The architecture already contains the pieces needed for c-factor:

- the Bus for turns and delivery
- the Substrate for durable lineage
- the Policy layer for intervention
- the observability surface for operator feedback

The remaining work is mostly in wiring those pieces into one continuous metric loop.

---

## Cross-References

- See [01-naming-and-glossary](./01-naming-and-glossary.md) for the canonical vocabulary used in this chapter
- See [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) for the Neuro, Daimon, and Dreams injection points
- See [02-engram-data-type](./02-engram-data-type.md) for HDC fingerprinting on durable artifacts
- See [25-attention-as-currency](./25-attention-as-currency.md) for the attention-economy lever used to curb dominance
- See [11-dual-process-and-active-inference](./11-dual-process-and-active-inference.md) for online calibration from prediction error
- See [../13-coordination/INDEX.md](../13-coordination/INDEX.md) for coordination and collective-metric chapters
- See [../../tmp/refinements/13-collective-intelligence-c-factor.md](../../tmp/refinements/13-collective-intelligence-c-factor.md) for the full refinement proposal
