# 25. Active Inference State Space

> The factorized POMDP as a Loop Graph with predict-publish-correct. 90 states (6x5x3) make active inference tractable. EFE decomposes into pragmatic + epistemic - cost. Bootstrapping phases handle the cold-start problem.

See [02-CELL.md](../../unified/02-CELL.md) for predict-publish-correct, [03-GRAPH.md](../../unified/03-GRAPH.md) for Loop pattern, [05-AGENT.md](../../unified/05-AGENT.md) for tier routing.

---

## 1. Why Active Inference

Standard tier routing uses heuristic thresholds: prediction error < 0.2 = T0, < 0.6 = T1, >= 0.6 = T2. This works but has three problems:

1. **Hyperparameters**: The thresholds (0.2, 0.6) must be tuned per domain.
2. **No exploration**: The system never tries T2 when the threshold says T1, so it cannot discover that T2 would have been better.
3. **No goal integration**: The system does not know whether the current task *needs* deep reasoning or not.

Active inference (Friston 2010, 2015) solves all three by framing tier selection as policy selection in a Partially Observable Markov Decision Process (POMDP). The agent maintains beliefs about its own epistemic state and selects the tier that minimizes Expected Free Energy (EFE) -- a quantity that naturally balances exploration, exploitation, and cost with **zero hyperparameters**.

---

## 2. The Factorized State Space

The key insight from Koudahl et al. (2024, arXiv:2412.10425): **do not model the world -- model the agent's epistemic situation.** Three dimensions suffice:

```
State = (TaskPhase, ContextQuality, Uncertainty)

TaskPhase in {Understanding, Planning, GatheringContext, Implementing, Verifying, Complete}
ContextQuality in {None, Insufficient, Partial, Adequate, Comprehensive}
Uncertainty in {High, Medium, Low}

Total: 6 x 5 x 3 = 90 states
```

### 2.1 Why These Three Dimensions

**TaskPhase** determines what kind of work is appropriate:
- Understanding -> invest in retrieval (T1)
- Implementing -> invest in deep reasoning (T2)
- Verifying -> run deterministic gates (T0)

**ContextQuality** determines whether more retrieval is needed:
- None/Insufficient -> gather before acting
- Comprehensive -> proceed with confidence

**Uncertainty** determines the tier directly:
- High -> T2 (full model, explore the space)
- Low -> T0 or T1 (heuristics suffice)

Together, these 90 states are enough to make principled compute allocation decisions.

---

## 3. The POMDP as a Loop Graph

The active inference system is a Loop Graph with predict-publish-correct:

```toml
[graph]
id = "active_inference_loop"
kind = "loop"
feedback_edge = "correct -> belief_update"

[[cells]]
id = "belief_update"
protocol = "Score"
description = "Bayesian belief update over 90 states from observations"
inputs = ["observation", "correction"]
outputs = ["beliefs"]

[[cells]]
id = "efe_compute"
protocol = "Route"
description = "Compute EFE for each action, select argmin"
inputs = ["beliefs"]
outputs = ["selected_action"]

[[cells]]
id = "predict"
protocol = "React"
description = "Publish predicted observation as Pulse"
inputs = ["selected_action", "beliefs"]
outputs = ["prediction_pulse"]

[[cells]]
id = "observe"
protocol = "Store"
description = "Read actual outcome from Bus"
inputs = ["prediction_pulse"]
outputs = ["observation"]

[[cells]]
id = "correct"
protocol = "Verify"
description = "Compute prediction error, update A/B matrices"
inputs = ["prediction_pulse", "observation"]
outputs = ["correction"]
```

### 3.1 The Four Matrices

Following the pymdp framework (Heins et al. 2022):

**A Matrix (Likelihood)**: P(observation | state). Maps hidden states to expected Bus observations.

```rust
/// A matrix: 90 states x N observation channels.
///
/// Learned from Bus topic joins:
///   prediction.<operator> + outcome.<operator> -> prediction.error.<operator>
///
/// Each row gives the probability of observing a specific signal pattern
/// given that the agent is in a particular epistemic state.
pub struct AMatrix {
    // Dimensions: [observation_value][state_idx]
    pub data: Array2<f64>,  // shape: (obs_channels * obs_values, 90)
}
```

Observable channels:
- `compilation_result` in {success, warning, failure, n/a}
- `test_pass_rate` in {high, medium, low, n/a}
- `embedding_similarity` in {high, medium, low}
- `prediction_error_magnitude` in {low, medium, high}
- `gate_verdict` in {pass, fail, n/a}

**B Matrix (Transitions)**: P(next_state | current_state, action). How actions change the agent's epistemic situation.

```rust
/// B matrix: 90 x 90 x 6 (transitions per action).
pub struct BMatrix {
    // Dimensions: [next_state][current_state][action]
    pub data: Array3<f64>,  // shape: (90, 90, 6)
}
```

Actions:
- `suppress` (T0: do nothing)
- `retrieve_context` (query Store for relevant Signals)
- `implement` (write code, execute)
- `run_tests` (run gate pipeline)
- `reflect` (theta-style step back)
- `escalate` (switch to T2)

**C Matrix (Preferences)**: Desired observations (goals).

```rust
/// C matrix: preference over observations.
/// Higher values = more desired outcomes.
pub struct CMatrix {
    pub data: Array1<f64>,  // shape: (obs_channels * obs_values,)
}

// Example preferences:
// gate_verdict=pass:       +3.0
// compilation=success:     +2.0
// test_pass_rate=high:     +2.0
// prediction_error=low:    +1.0
// gate_verdict=fail:       -3.0
// compilation=failure:     -2.0
```

**D Matrix (Initial Beliefs)**: Prior belief distribution at task start.

```rust
/// D matrix: initial belief over 90 states.
/// Updated across tasks as the agent learns typical starting conditions.
pub struct DMatrix {
    pub data: Array1<f64>,  // shape: (90,), sums to 1.0
}

// Default initialization:
// TaskPhase=Understanding:  0.8
// ContextQuality=None:      0.7
// Uncertainty=High:         0.6
```

---

## 4. EFE Computation

Expected Free Energy for action `a` given current beliefs `b`:

```
G(a) = -pragmatic(a) - epistemic(a) + cost(a)

pragmatic(a) = -D_KL(predicted_obs(a) || preferred_obs)
epistemic(a) = sum_s [ b(s) * H[P(o|s)] ]
cost(a) = dollar_cost(a) * cost_sensitivity
```

Where:
- **Pragmatic value**: How close are predicted observations to preferences? (goal-directed)
- **Epistemic value**: How much uncertainty would this action reduce? (curiosity-driven)
- **Cost**: Resource expenditure of this action.

The system selects `argmin G(a)` -- the action with lowest free energy.

```rust
/// Compute EFE for a single action given current beliefs.
///
/// Microsecond-scale computation over 90 states.
/// No LLM needed -- pure linear algebra.
pub fn compute_efe(
    action: usize,
    beliefs: &Array1<f64>,    // shape: (90,)
    a_matrix: &AMatrix,
    b_matrix: &BMatrix,
    c_matrix: &CMatrix,
    cost: f64,
) -> f64 {
    // Predict next state under this action
    let b_action = b_matrix.data.slice(s![.., .., action]);
    let predicted_state = b_action.dot(beliefs); // shape: (90,)

    // Predict observations from predicted state
    let predicted_obs = a_matrix.data.dot(&predicted_state); // shape: (obs,)
    let predicted_obs_norm = normalize(&predicted_obs);

    // Pragmatic value: KL from predicted obs to preferred obs
    let preferred = softmax(&c_matrix.data);
    let pragmatic = -kl_divergence(&predicted_obs_norm, &preferred);

    // Epistemic value: expected entropy of observations
    let mut epistemic = 0.0;
    for s in 0..90 {
        if predicted_state[s] > 0.001 {
            let obs_given_s = a_matrix.data.column(s);
            epistemic += predicted_state[s] * entropy(&obs_given_s);
        }
    }

    -(pragmatic + epistemic) + cost
}
```

### 4.1 Mapping EFE to Tier Selection

```rust
/// Select tier using active inference.
/// Replaces heuristic threshold with principled information-theoretic routing.
pub fn select_tier(beliefs: &Beliefs, matrices: &Matrices) -> InferenceTier {
    let efe_suppress = compute_efe(Action::Suppress, beliefs, matrices, 0.000);
    let efe_retrieve = compute_efe(Action::Retrieve, beliefs, matrices, 0.001);
    let efe_implement = compute_efe(Action::Implement, beliefs, matrices, 0.001);
    let efe_escalate = compute_efe(Action::Escalate, beliefs, matrices, 0.100);

    let efe_t0 = efe_suppress;
    let efe_t1 = efe_retrieve.min(efe_implement);
    let efe_t2 = efe_escalate;

    if efe_t0 <= efe_t1 && efe_t0 <= efe_t2 {
        InferenceTier::T0
    } else if efe_t1 <= efe_t2 {
        InferenceTier::T1
    } else {
        InferenceTier::T2
    }
}
```

---

## 5. Learning the Matrices (Predict-Publish-Correct)

All four matrices learn from experience via Bayesian count updates:

### 5.1 A Matrix Learning

After each gamma tick, the agent observes actual outputs and updates the likelihood:

```rust
/// Update A matrix after observing actual observation in known state.
fn update_a_matrix(a: &mut AMatrix, state_idx: usize, obs_idx: usize, lr: f64) {
    // Dirichlet update: increment count for observed (obs, state) pair
    a.data[[obs_idx, state_idx]] += lr;
    // Renormalize column
    let col_sum: f64 = a.data.column(state_idx).sum();
    a.data.column_mut(state_idx).mapv_inplace(|x| x / col_sum);
}
```

### 5.2 B Matrix Learning

After each state transition:

```rust
/// Update B matrix after observing state transition under action.
fn update_b_matrix(
    b: &mut BMatrix,
    from_state: usize,
    to_state: usize,
    action: usize,
    lr: f64,
) {
    b.data[[to_state, from_state, action]] += lr;
    // Renormalize column for this (from_state, action) pair
    let col_sum: f64 = b.data.slice(s![.., from_state, action]).sum();
    b.data.slice_mut(s![.., from_state, action])
        .mapv_inplace(|x| x / col_sum);
}
```

### 5.3 Bootstrapping Phases

The system transitions through three phases as it accumulates experience:

| Phase | Observations | Behavior | Rationale |
|---|---|---|---|
| **Cold start** (0-49) | Flat prior (uniform D) | Heuristic threshold fallback | Matrices uninformative |
| **Transition** (50-199) | Learning rates high (0.1) | Blend EFE + heuristic (50/50) | Matrices forming but noisy |
| **Steady state** (200+) | Learning rates low (0.01) | Pure EFE routing | Matrices calibrated |

```rust
/// Determine bootstrapping phase.
pub fn bootstrap_phase(total_observations: u64) -> BootstrapPhase {
    match total_observations {
        0..=49 => BootstrapPhase::ColdStart,
        50..=199 => BootstrapPhase::Transition,
        _ => BootstrapPhase::SteadyState,
    }
}
```

During ColdStart, the system uses the existing heuristic threshold (prediction_error < 0.2 / 0.6). During Transition, it blends: `tier = if rand() < 0.5 { efe_tier } else { heuristic_tier }`. During SteadyState, EFE dominates entirely.

---

## 6. Composition with Other Systems

### 6.1 Tier Routing (from EFE)

The EFE output selects among three policies (T0/T1/T2). This replaces the scalar threshold comparison in the dual-process router:

```
Before: prediction_error < threshold -> tier
After:  argmin(EFE(T0), EFE(T1), EFE(T2)) -> tier
```

### 6.2 Cascade Router (model within tier)

Once EFE selects the tier, the CascadeRouter (LinUCB bandit) selects the specific model within that tier. The two systems compose cleanly:

```
EFE -> tier (T0/T1/T2)
CascadeRouter -> model within tier (e.g., T1 -> Haiku vs Gemini-Flash)
```

### 6.3 Cognitive Energy Model

The cognitive energy model (see [cognitive-energy-and-vitality.md](cognitive-energy-and-vitality.md)) gates policy availability. If energy is below the T2 threshold, the EFE system cannot select T2 regardless of its computed advantage:

```rust
/// Gate EFE selection by available energy.
fn energy_gated_tier(efe_tier: InferenceTier, energy: f64) -> InferenceTier {
    match efe_tier {
        InferenceTier::T2 if energy < 0.3 => InferenceTier::T1,
        InferenceTier::T1 if energy < 0.1 => InferenceTier::T0,
        tier => tier,
    }
}
```

---

## 7. Bus-Backed Observation Model

The concrete observables are not raw environment state. They are Pulses on the Bus, joined by lineage:

```
observation = join(
    prediction.<operator>,   // what the Cell predicted
    outcome.<operator>,      // what actually happened
    prediction.error.<operator>  // the residual
)
```

This means the A matrix is learned from topic-family joins. A low prediction-error Pulse means the predicted and observed aligned. A high one means the join exposed a mismatch. The observation model is grounded in the same Bus fabric that carries all other Pulses.

---

## What This Enables

- **Zero-hyperparameter routing**: No thresholds to tune. EFE adapts automatically.
- **Principled exploration**: Epistemic value drives the system to try T2 when uncertain (curiosity), not just when prediction error is high.
- **Goal-aware routing**: The C matrix encodes what the agent wants. Different tasks produce different routing because the pragmatic value changes.
- **Microsecond computation**: 90 states, 6 actions -- the EFE computation takes microseconds. No overhead vs the heuristic threshold.
- **Continuous improvement**: The matrices learn from every tick, making routing more accurate over time.

## Feedback Loops

1. **beliefs -> EFE -> action -> observation -> belief update** (the core active inference loop, expressed as predict-publish-correct on the Bus).
2. **A matrix -> predicted obs -> actual obs -> A matrix update** (observation model learning).
3. **B matrix -> predicted transition -> actual transition -> B matrix update** (transition model learning).
4. **Bootstrap phase -> routing blend -> outcomes -> observation count -> next bootstrap phase** (meta-learning the learning rate).

## Open Questions

1. Should the 90-state POMDP be extended for multi-task contexts (parallel tasks increase state space)?
2. How should the C matrix adapt to different task types (coding vs research have different "success" observations)?
3. Should there be a separate POMDP instance per domain profile, or one shared instance with domain-conditioned matrices?
4. What is the interaction between EFE and the affect-modulated threshold (do they compose or conflict)?

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Define factorized state space types | `crates/roko-learn/src/active_inference/state.rs` | Not started |
| Implement A/B/C/D matrices with Bayesian updates | `crates/roko-learn/src/active_inference/matrices.rs` | Not started |
| Implement EFE computation | `crates/roko-learn/src/active_inference/efe.rs` | Not started |
| Implement bootstrap phase logic | `crates/roko-learn/src/active_inference/bootstrap.rs` | Not started |
| Wire into CascadeRouter as alternative route policy | `crates/roko-learn/src/cascade_router.rs` | Not started |
| Bus observation model (topic joins) | `crates/roko-runtime/src/observation.rs` | Not started |
| Persist matrices to `.roko/learn/active-inference.json` | `crates/roko-learn/src/active_inference/persist.rs` | Not started |
| Integration test: cold-start -> transition -> steady state | `crates/roko-learn/tests/active_inference.rs` | Not started |
