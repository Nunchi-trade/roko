# Active Inference State Space: Factorized Discrete POMDP

> A 90-state tractable model that makes active inference practical - not modeling the world, but modeling the agent's epistemic situation.


> **Implementation**: Specified

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md)
**Key sources**: `refactoring-prd/09-innovations.md` §XIX-A, Koudahl et al. 2024 (arXiv:2412.10425), VERSES AI Genius platform

---

## Abstract

Active inference promises principled, zero-hyperparameter compute allocation, but naive implementation is intractable. A real-world agent state space (all possible configurations of tasks, knowledge, predictions, affect) is effectively infinite. The key insight from Koudahl et al. (2024, arXiv:2412.10425) and VERSES AI's Genius platform is: **don't model the world - model the agent's epistemic situation.**

Instead of tracking the full state of the environment (impossible), track three dimensions that fully characterize what the agent needs to know to make good decisions:
1. **Where am I in the task lifecycle?** (TaskPhase)
2. **How good is my current context?** (ContextQuality)
3. **How uncertain am I?** (Uncertainty)

This factorized state space has only 6 × 5 × 3 = **90 states** - completely tractable for standard active inference POMDP matrices. The agent maintains a belief distribution over these 90 states and selects actions (tier, context strategy, model) that minimize expected free energy.

This document specifies the factorized state space, the four standard POMDP matrices (A, B, C, D from the pymdp framework), the action space, and the observation model. In the Roko implementation, the observation model is concrete: it is assembled from Bus topic joins over `prediction.*`, `outcome.*`, and `prediction.error.*` Pulses.

---

## The Factorized State Space

```
State = (TaskPhase, ContextQuality, Uncertainty)

TaskPhase ∈ {
    Understanding,      // Parsing the task, identifying requirements
    Planning,           // Generating approach, decomposing into steps
    GatheringContext,   // Retrieving relevant knowledge, code, docs
    Implementing,       // Writing code, executing actions
    Verifying,          // Running gates, checking results
    Complete            // Task finished
}  // 6 states

ContextQuality ∈ {
    None,              // No relevant context retrieved
    Insufficient,      // Some context but critical gaps
    Partial,           // Moderate coverage, some ambiguity
    Adequate,          // Good coverage for the current phase
    Comprehensive      // Full coverage, high confidence in context
}  // 5 states

Uncertainty ∈ {
    High,              // Agent is unsure about approach or state
    Medium,            // Some clarity but open questions remain
    Low                // High confidence in current understanding
}  // 3 states

Total: 6 × 5 × 3 = 90 states
```

### Why These Three Dimensions

The three dimensions capture the essential information the agent needs for compute allocation:

**TaskPhase** determines what kind of work is appropriate. During `Understanding`, the agent should invest in reading and retrieval (T1 for analysis). During `Implementing`, the agent should invest in deep reasoning (T2 for code generation). During `Verifying`, the agent runs gates (T0 for deterministic checks).

**ContextQuality** determines whether more retrieval is needed. If context is `None` or `Insufficient`, the agent should gather more before acting. If context is `Comprehensive`, the agent can proceed with confidence.

**Uncertainty** determines the tier. `High` uncertainty → T2 (full model, deep reasoning). `Low` uncertainty → T0 or T1 (heuristics or quick check). `Medium` → T1 (fast model assessment).

Together, these three dimensions provide enough information to make principled tier, model, and context decisions without tracking the full environment state.

---

## The Four POMDP Matrices

Following the pymdp framework (Heins et al. 2022, "pymdp: A Python library for active inference in discrete state spaces"), the active inference POMDP uses four matrices:

### A Matrix (Likelihood): States → Observations

The A matrix maps hidden states to observable signals. It answers: "Given that I'm in state (TaskPhase=implementing, ContextQuality=adequate, Uncertainty=medium), what observations do I expect?"

### Bus-backed observation model

The concrete observables are not raw environment state. They are Pulses on the Bus, joined by lineage and operator identity:

- `prediction.<operator>` Pulses publish the expected outcome of an operator.
- `outcome.<operator>` Pulses publish the later observation that closes the loop.
- `prediction.error.<operator>` Pulses publish the residual after joining prediction and outcome.
- Calibration and scheduling policies subscribe to those topic families and join them by `lineage_hint`, task hash, or operator name.

That means the A matrix is learned from topic-family joins, not from a monolithic world-state snapshot. A single observation row can be expressed as:

```text
observation = join(
  prediction.<operator>,
  outcome.<operator>,
  prediction.error.<operator>
)
```

Observable signals for each state combination:

```
Observations:
- compilation_result ∈ {success, warning, failure, not_applicable}
- test_pass_rate ∈ {high, medium, low, not_applicable}
- embedding_similarity ∈ {high, medium, low}  // similarity between task and retrieved context
- prediction.error.<operator> ∈ {low, medium, high}
- gate_verdict ∈ {pass, fail, not_applicable}
```

Example A matrix entries:
```
P(compilation_result=success | implementing, comprehensive, low) = 0.8
P(compilation_result=failure | implementing, insufficient, high) = 0.6
P(test_pass_rate=high | verifying, adequate, low) = 0.7
P(prediction.error.low | understanding, comprehensive, low) = 0.9
```

The A matrix is initialized from domain heuristics and updated from actual Bus observations via Bayesian learning. A low prediction-error topic family means the predicted and observed Pulses aligned; a high prediction-error family means the join exposed a mismatch worth learning from.

### B Matrix (Transitions): Actions → State Changes

The B matrix maps actions to state transitions. It answers: "If I take action X in state S, what state am I likely to end up in?"

```
Actions:
- retrieve_context     // Query Neuro, run code intelligence
- implement            // Write code, execute task steps
- run_tests            // Run gate pipeline
- reflect              // Theta-style step-back reflection
- escalate             // Switch to stronger model (T1→T2)
- suppress             // Stay at T0, no action
```

Example B matrix entries:
```
P(context=adequate | context=insufficient, retrieve_context) = 0.6
P(phase=implementing | phase=planning, implement) = 0.7
P(uncertainty=low | uncertainty=medium, run_tests) = 0.5
P(phase=complete | phase=verifying, run_tests, gate_pass=true) = 0.8
```

The B matrix encodes domain knowledge about how actions change the agent's epistemic situation. It is updated from actual transitions observed during gamma ticks and correlated against the same Bus joins that feed the A matrix.

### C Matrix (Preferences): Desired Observations

The C matrix encodes what the agent wants to observe — its goals. For Roko agents, preferences come from the task specification:

```
C (preferred observations):
- compilation_result=success:    high preference (+2.0)
- test_pass_rate=high:          high preference (+2.0)
- prediction.error.low:         moderate preference (+1.0)
- gate_verdict=pass:            highest preference (+3.0)
- compilation_result=failure:    negative preference (-2.0)
- gate_verdict=fail:            negative preference (-3.0)
```

The C matrix is the "goal specification" - it tells the active inference system what observations to seek and what to avoid. For a coding task, gate pass is the highest-preference outcome. For a research task, high embedding similarity (good context retrieval) might receive higher preference. In the self-learning framing from `tmp/refinements/10-self-learning-cybernetic-loops.md`, low prediction error is also a preference signal because it marks beliefs that match the Bus.

### D Matrix (Initial Beliefs): Prior State Distribution

The D matrix encodes the agent's initial belief about its state when starting a task:

```
D (initial belief):
- TaskPhase = Understanding:     P = 0.8 (start by understanding)
- ContextQuality = None:         P = 0.7 (no context yet)
- Uncertainty = High:            P = 0.6 (uncertain at start)
```

As the agent accumulates experience across tasks, the D matrix is updated to reflect better priors. An experienced agent might start with `Uncertainty=Medium` because it has seen similar tasks before.

---

## EFE Computation with the 90-State Space

Given the four matrices, expected free energy for each action is computed as:

```python
# Pseudocode (pymdp-style)
def compute_efe(action, current_beliefs, A, B, C):
    # Predict next state under this action
    predicted_state = B[action] @ current_beliefs

    # Predict observations from predicted state
    predicted_obs = A @ predicted_state

    # Pragmatic value: how close are predicted observations to preferences?
    pragmatic = -KL_divergence(predicted_obs, softmax(C))

    # Epistemic value: how much uncertainty is reduced?
    # H[P(o|s)] averaged over predicted states
    epistemic = 0.0
    for state_idx in range(90):
        if predicted_state[state_idx] > 0.001:
            obs_given_state = A[:, state_idx]
            epistemic += predicted_state[state_idx] * entropy(obs_given_state)

    return -(pragmatic + epistemic)  # Minimize EFE = maximize negative EFE
```

With only 90 states and 6 actions, this computation takes **microseconds**. The A matrix has ~90 × 15 entries (90 states × 5 observation channels × 3 values each). The B matrix has ~90 × 90 × 6 entries (transitions from each state to each state for each action). All matrices fit in a few kilobytes.

---

## Mapping to Tier Selection

The EFE computation maps directly to tier selection:

```rust
/// Select the optimal tier using active inference EFE.
///
/// This replaces the heuristic threshold approach with a
/// principled information-theoretic computation.
fn select_tier_efe(
    beliefs: &BeliefState,       // Distribution over 90 states
    matrices: &POMDPMatrices,    // A, B, C, D matrices
) -> InferenceTier {
    // Compute EFE for each action
    let efe_suppress = compute_efe(Action::Suppress, beliefs, matrices);
    let efe_quick = compute_efe(Action::RetrieveContext, beliefs, matrices);
    let efe_deep = compute_efe(Action::Escalate, beliefs, matrices);

    // Map actions to tiers
    let efes = [
        (InferenceTier::T0, efe_suppress),
        (InferenceTier::T1, efe_quick.min(compute_efe(Action::Implement, beliefs, matrices))),
        (InferenceTier::T2, efe_deep.min(compute_efe(Action::Reflect, beliefs, matrices))),
    ];

    // Select tier with lowest EFE (highest expected value)
    efes.iter()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(tier, _)| *tier)
        .unwrap_or(InferenceTier::T1)
}
```

### When EFE Beats Heuristic Threshold

The EFE approach outperforms the heuristic threshold in several scenarios:

1. **Exploration**: When the agent is in `Understanding` phase with `None` context, the heuristic might suppress (low prediction error) while EFE correctly identifies high epistemic value in retrieval.

2. **Late-stage verification**: When the agent is in `Verifying` phase with `Comprehensive` context, the heuristic might escalate (some test failures = high prediction error) while EFE correctly identifies that T0 (run more tests) has higher expected value than T2 (re-analyze with LLM).

3. **Budget optimization**: EFE naturally accounts for cost via the cost term, eliminating the need for separate budget-aware throttling logic.

---

## Learning the Matrices

The POMDP matrices are not static - they are learned from the agent's experience:

### A Matrix Learning

After each gamma tick, update the likelihood matrix:
```
A[observation, state] += learning_rate × (observed_obs == predicted_obs)
```

This makes the A matrix more accurate over time - the agent learns what Bus observations to expect in each state.

### B Matrix Learning

After each state transition, update the transition matrix:
```
B[next_state, current_state, action] += learning_rate × (transition_actually_occurred)
```

This makes the B matrix more accurate - the agent learns the actual effects of its actions.

### C Matrix Learning

The C matrix updates slowly based on task outcomes:
- Tasks where gate_pass=true had high C[gate_pass]: increase C[gate_pass] preference
- Tasks where retrieval led to better outcomes: increase C[high_similarity] preference
- Tasks with lower prediction.error.* over time can raise preference for the operator-state combinations that generated them

### D Matrix Learning

The D matrix updates across tasks:
- If the agent frequently starts tasks in `Understanding, None, Medium` (rather than `Understanding, None, High`), update D to reflect this.

All learning is Bayesian: count-based updates with Dirichlet priors, yielding well-calibrated posterior distributions.

---

## Comparison to Alternative Approaches

| Approach | Hyperparameters | Exploration/Exploitation | Cost Awareness | State Representation |
|---|---|---|---|---|
| Fixed threshold | 2 (T1 threshold, T2 threshold) | None (pure exploitation) | Separate throttling logic | None (scalar prediction error) |
| Epsilon-greedy | 1 (epsilon) | Random exploration | Not integrated | None |
| UCB1 | 1 (exploration coefficient) | Bonus for under-explored | Not integrated | Arm-level counts |
| Thompson sampling | Prior distribution | Posterior sampling | Not integrated | Beta distribution per arm |
| **Active inference EFE** | **0** | **Emergent from EFE** | **Integrated via cost term** | **90-state POMDP** |

Active inference's zero-hyperparameter property is its distinguishing feature. All other approaches require tuning the exploration/exploitation tradeoff. EFE provides a principled, information-theoretic answer that adapts automatically, especially when the observation stream is built from Bus joins over prediction, outcome, and prediction-error topic families.

---

## Academic Foundations

- **Friston 2010** — "The free-energy principle: a unified brain theory?" (Nature Reviews Neuroscience 11(2)). Free energy minimization as a unified brain theory.
- **Friston et al. 2015** — "Active inference and epistemic value" (Cognitive Neuroscience 6(4)). Expected free energy for policy selection.
- **Koudahl et al. 2024** — "Factorized discrete POMDP for active inference" (arXiv:2412.10425). Factorized state spaces making active inference tractable.
- **Heins et al. 2022** — "pymdp: A Python library for active inference in discrete state spaces" (JOSS). Reference implementation of active inference POMDP.
- **VERSES AI** — Genius platform. Industrial deployment of active inference for AI agent cognition.
- **Parr & Friston 2017** — "Working memory, attention, and salience in active inference" (Scientific Reports 7). Attention allocation via precision weighting.

---

## Current Status and Gaps

**What exists:**
- `InferenceTier` and heuristic threshold approach (Stage 1 of implementation path).
- `CascadeRouter` with UCB1 exploration (approximates EFE exploration bonus).
- `LinUCBRouter` in `roko-learn` provides contextual bandit foundations.

**What is missing:**
- Factorized 90-state POMDP definition.
- A/B/C/D matrix initialization from domain heuristics.
- EFE computation function (microsecond-scale).
- Bayesian matrix learning from gamma tick observations.
- `ActiveInferenceRouter` integrating EFE into tier and model selection.
- Integration with `PredictiveScorer` for EFE-based context scoring.

---

## Cross-References

- See [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md) for the EFE theory
- See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for the heuristic threshold (Stage 1)
- See [09-16-t0-probes.md](./09-16-t0-probes.md) for probe signals that map to observations
- See topic [05-learning](../05-learning/INDEX.md) for CascadeRouter and bandit algorithms
- See `tmp/refinements/10-self-learning-cybernetic-loops.md` for the predict-publish-correct loop, per-operator calibration, and Bus-backed observation model
