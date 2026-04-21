# Self-Learning & Cybernetic Feedback Loops

> **REF10 source:** `../../tmp/refinements/10-self-learning-cybernetic-loops.md`
> **Glossary:** [Naming and Glossary](../00-architecture/01-naming-and-glossary.md)
> **Cross-references:** [16-predictive-foraging](16-predictive-foraging.md), [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md), [15-collective-calibration-31x](15-collective-calibration-31x.md), [17-adas-and-autocatalytic](17-adas-and-autocatalytic.md), [20-research-to-runtime](20-research-to-runtime.md), `../../tmp/refinements/16-research-to-runtime.md`
>
> **Implementation status**: Active inference exists in `roko-learn` (`active_inference.rs`, ~255 lines) as a working Bayesian tier selector. Prediction tracking exists in `prediction.rs`. The per-operator predict-publish-correct doctrine described here is **target-state**; today the Router has the richest prediction/outcome signals, while Bus/Pulse-mediated calibration across every operator remains planned.

---

## Purpose

REF10 describes a target-state extension that would turn learning in Roko into a Bus-backed feedback nervous system. The key move is simple: every operator becomes a predictor. It publishes a prediction Pulse, later receives an outcome Pulse, and then updates from the prediction error. That is broader than the current code: active inference already exists, but narrowly, as a routing component rather than a universal operator doctrine.

The refinement text centers three anchor learners, but the shipping learning subsystem is broader than that framing suggests. `roko-learn` already includes routing, prediction tracking, runtime feedback, bandits, drift detection, pattern discovery, skill accumulation, and provider-health handling. Within that larger crate, three obvious anchor learners are:

1. `CascadeRouter` learned which model tier to pick.
2. `EpisodeLogger` accumulated completed runs for replay and distillation.
3. `ExperimentStore` ran prompt A/B tests.

The refinement does not replace those learners. It makes them uniform. Once the Bus is first-class, the same predict-publish-correct pattern can be used for routing, prompt composition, gate thresholds, policy decisions, and storage-tier choices.

## The Predict-Publish-Correct Loop

For any operator `O` that transforms input `x` into output `y`, the learning loop is:

1. `O` publishes a prediction Pulse with a topic in the `prediction.*` family.
2. A downstream system publishes the actual outcome on `outcome.*` with the same lineage hint.
3. A calibration policy joins the two Pulses, computes loss, and updates operator state.
4. `O` subscribes to its calibration update and adjusts future behavior.

In prose, that is the Free Energy Principle implemented as a Bus protocol: make a prediction, compare it to the world, and minimize future error. The important detail is that the error is not hidden inside one operator; it becomes a first-class Pulse stream that other learners can subscribe to.

## Per-Operator Calibration

The Bus makes per-operator calibration cheap enough to do everywhere, not just in the router.

| Operator | Predicts | Outcome signal | Update policy |
|---|---|---|---|
| `Scorer` | Candidate quality or reward by score axis | Gate verdict plus episode reward | Online calibration per axis, with reliability curves |
| `Router` | Which action or model choice will succeed | Gate verdict | Contextual bandit updates |
| `Composer` | Whether the prompt fits budget and wins the gate | Token count plus gate verdict | Template EMA and variant selection |
| `Gate` | Whether the task will succeed post-patch | Next verdict plus regression tests | Threshold smoothing and drift correction |
| `Policy` | Whether a decision will improve a metric | Metric Pulse after the decision | Per-policy online calibration |
| `Substrate` | Whether an Engram belongs in a given tier | Query frequency, recency, and reuse | Tier-promotion and retention policy |

This is the missing middle between fixed heuristics and heavyweight model retraining. The calibration target is not just “did the task pass?” but “which operator was systematically overconfident, underconfident, or stale?”

The same calibration machinery could eventually apply to research-derived defaults. That depends on the separate research-to-runtime work landing first; today there is no `claim!`-style runtime resolver or replication ledger in the codebase.

## CalibrationPolicy

`CalibrationPolicy` is the chapter-level name for the Bus consumer that closes the loop. It subscribes to the `prediction.*` and `outcome.*` families, matches records by lineage, and maintains per-operator state:

- trial counts
- error accumulators
- EMA of recent error
- axis-specific calibration curves where the operator has multiple sub-scores

When the policy closes a prediction/outcome pair, it publishes a calibration update on a topic such as `calibration.scorer.updated` or `calibration.router.updated`. The operator then consumes that update the same way it consumes any other Bus-delivered fact.

The concrete implementation details can vary, but the structure should not:

- prediction Pulses are lightweight and ephemeral
- outcome Pulses are ground truth from the downstream step
- calibration updates are separate Pulses, not hidden side effects
- the policy itself is just another Bus subscriber

That same policy could eventually ingest research-derived outcomes for paper-backed claims. The replication-ledger portion of that design is deferred for now.

That separation matters because it keeps learning composable. Operators do not need to know who is measuring them. They only need to publish predictions and react to calibration updates.

## `prediction.error.*` As A First-Class Signal

`prediction.error.*` is the shared language for uncertainty, drift, and surprise. It is useful at three levels:

1. Local error tells an operator how far off a specific prediction was.
2. Aggregated error tells the system which operator or topic is drifting.
3. Elevated error tells the planner or Dreams loop where to spend attention next.

This is why the refinement treats prediction error as a first-class signal. A spike in `prediction.error.high` is not just a debugging artifact. It is a routing input for learning itself. High-error regions can be replayed, consolidated, or prioritized for retraining.

The practical effect is that curiosity becomes observable. The system learns where its own models are weakest and spends effort there first.

## Existing Learners Reading Off The Bus

The Bus does not invent new learners; it rewires the existing ones so they subscribe to facts instead of being called directly.

### `CascadeRouter`

`CascadeRouter` becomes a subscriber to `router.selection.made` and `router.selection.outcome`. It updates its bandit state from those Pulses and publishes `router.weights.updated` when its internal calibration changes. That decouples routing logic from the routing caller.

### `EpisodeLogger`

`EpisodeLogger` subscribes to `agent.turn.completed` and `gate.verdict.emitted`, then correlates them into episodes. The orchestrator no longer needs to know the logger exists. The logger just reads the Bus and persists the records that matter.

### `ExperimentStore`

`ExperimentStore` subscribes to `composer.invocation.started`, assigns a prompt variant, and publishes `composer.variant.assigned`. Gate verdicts later close the loop. That turns prompt experimentation into a continuous Bus-driven optimization process instead of an ad hoc side channel.

The architectural payoff is that each learner becomes replaceable and composable. Adding a new learner means writing one subscriber and one publisher, not threading another callback through the whole runtime.

## Why This Matters

This chapter is the learning-layer expression of the broader two-fabric design:

- the Bus carries prediction and outcome Pulses
- the Substrate retains durable records when lineage matters
- active inference is already real for routing, while broader operator coverage remains target-state
- every operator can eventually be calibrated independently once the transport and outcome surfaces exist
- existing learners can cooperate without tight coupling

The result is a self-modeling system. Prediction error is no longer an incidental byproduct; it is the signal that keeps the system honest.

## Relationship To Other Docs

- [16-predictive-foraging](16-predictive-foraging.md) covers task-level prediction and calibration; this doc generalizes the same logic to operators.
- [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md) catalogs the current wiring gaps that REF10 turns into Bus-backed learner loops.
- [15-collective-calibration-31x](15-collective-calibration-31x.md) applies calibration at the collective level.
- [17-adas-and-autocatalytic](17-adas-and-autocatalytic.md) frames the compound effect of the loops as autocatalytic growth.
- [20-research-to-runtime](20-research-to-runtime.md) sketches the target-state paper → claim → heuristic → trial → calibration pipeline and the deferred replication-ledger layer.
- See also `tmp/refinements/10-self-learning-cybernetic-loops.md` for the full proposal.
