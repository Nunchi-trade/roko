# Active Inference for Compute Allocation

> Expected Free Energy (EFE) determines how much cognitive resource an agent invests on each tick — zero hyperparameters, pure information-theoretic optimization.


> **Implementation**: Specified

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md), [09-16-t0-probes.md](./09-16-t0-probes.md)
**Key sources**: `refactoring-prd/01-synapse-architecture.md` §Dual-Process Cognition, `refactoring-prd/09-innovations.md` §XIX-A, Friston 2010 (Nature Reviews Neuroscience)

---

## Abstract

The T0/T1/T2 tier gating described in [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) uses a heuristic threshold to decide how much compute to invest per tick. Active inference provides the theoretical foundation for this decision: the agent should invest compute that **minimizes expected free energy (EFE)** — balancing the pragmatic value of action with the epistemic value of information, minus the cost.

Friston's free energy principle (2010, "The free-energy principle: a unified brain theory?", Nature Reviews Neuroscience 11(2)) proposes that biological organisms minimize a quantity called "free energy" — the discrepancy between their internal model and the sensory evidence. Minimizing free energy is equivalent to maximizing model evidence, which is equivalent to minimizing surprise. An agent that minimizes free energy learns an accurate world model and acts to keep itself in preferred states.

The active inference extension (Friston et al. 2015, "Active inference and epistemic value", Cognitive Neuroscience 6(4)) goes further: the agent doesn't just minimize current free energy — it selects policies (sequences of actions) that minimize **expected** free energy into the future. This naturally balances exploitation (pragmatic value — doing things that achieve goals) with exploration (epistemic value — doing things that reduce uncertainty).

For Roko, active inference provides a principled, zero-hyperparameter answer to the question: "How much should I think about this?" The tier decision, the context budget, and the model selection all emerge from EFE minimization rather than hand-tuned thresholds.

---

## The EFE Formula

Expected Free Energy for a policy π at time step τ:

```
G(π, τ) = -E_Q[ln P(o_τ | C)]  +  E_Q[H[P(o_τ | s_τ)]]
           ─────────────────────     ─────────────────────
           pragmatic value            epistemic value
           (expected utility           (expected information
            of preferred outcomes)      gain from observations)
```

Where:
- `Q` is the agent's approximate posterior (its current beliefs)
- `o_τ` is the expected observation at time τ
- `s_τ` is the expected hidden state at time τ
- `C` is the agent's preferred outcomes (goals/objectives)
- `H` is entropy (uncertainty)

The first term (pragmatic value) captures how much the agent expects to achieve its goals under this policy. Policies that lead to preferred outcomes have low pragmatic free energy.

The second term (epistemic value) captures how much uncertainty the agent expects to resolve. Policies that expose the agent to informative observations have high epistemic value. This is what drives exploration -- the agent seeks out information that reduces its uncertainty, even when the immediate pragmatic value is low.

### The generative model Q: learning and bootstrapping

The formula requires `E_Q[ln P(o_τ|C)]` -- an expectation under the agent's approximate posterior Q. This raises the question: where does Q come from?

**Q is not a pretrained model.** It is a lightweight online approximation built from the agent's own observations. The generative model is a factorized categorical distribution over the CorticalState signal space (see [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md) for the CorticalState struct). Each signal dimension is modeled independently with a Dirichlet-categorical pair.

**Bootstrapping (cold start).** On the first tick, Q has no observations. The bootstrap strategy:

1. **Ticks 0-49 (prior phase):** Use a flat Dirichlet prior (alpha = 1.0 for all categories). This makes all outcomes equally likely, producing high epistemic value for every possible observation. The practical effect: the agent defaults to T2 for the first ~50 ticks, accumulating enough observations to form a posterior. At 10s gamma intervals, this is ~8 minutes of T2-heavy operation.

2. **Ticks 50-199 (transition phase):** Q has enough data to produce non-uniform predictions. The CascadeRouter's confidence stage handles model selection. Tier gating uses the heuristic threshold from [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) with EFE as a tiebreaker when prediction error is near the threshold.

3. **Ticks 200+ (steady state):** Q's posterior is well-calibrated. The ActiveInferenceRouter (Stage 3) can replace the heuristic threshold entirely.

```rust
/// Factorized generative model over CorticalState signals.
///
/// Each signal dimension maintains independent Dirichlet-categorical
/// parameters updated online via Bayesian updating.
pub struct GenerativeModel {
    /// Per-signal Dirichlet concentration parameters.
    /// Key: signal name. Value: vector of alpha_i for each category.
    dimensions: HashMap<SignalId, DirichletCategorical>,

    /// Total observations seen.
    observation_count: u64,
}

/// Single dimension of the factorized model.
pub struct DirichletCategorical {
    /// Concentration parameters (alpha). Updated on each observation.
    pub alphas: Vec<f64>,
    /// Number of categories for this signal.
    pub num_categories: usize,
}

impl DirichletCategorical {
    /// Create with flat prior.
    pub fn new_uniform(num_categories: usize) -> Self {
        Self {
            alphas: vec![1.0; num_categories],
            num_categories,
        }
    }

    /// Bayesian update: observe category k.
    pub fn observe(&mut self, k: usize) {
        if k < self.num_categories {
            self.alphas[k] += 1.0;
        }
    }

    /// Expected probability of category k under the posterior.
    pub fn expected_prob(&self, k: usize) -> f64 {
        let total: f64 = self.alphas.iter().sum();
        self.alphas[k] / total
    }

    /// Entropy of the predictive distribution.
    pub fn entropy(&self) -> f64 {
        let total: f64 = self.alphas.iter().sum();
        -(0..self.num_categories)
            .map(|k| {
                let p = self.alphas[k] / total;
                if p > 0.0 { p * p.ln() } else { 0.0 }
            })
            .sum::<f64>()
    }
}
```

### Preferred outcomes C: source and discounting

The preferred outcomes vector C defines what the agent considers "good." It enters the pragmatic value term as `E_Q[ln P(o_τ|C)]` -- the log probability of observing preferred outcomes under the current policy.

**Source of C.** Preferred outcomes are derived from two sources, combined additively:

1. **Task specification.** Each task in the plan DAG has success criteria (e.g., "compile passes", "test coverage > 80%", "diff under 500 lines"). These are translated into preferred CorticalState signal values. A task requiring compilation success maps to `{accuracy: high, resource_health: stable}`.

2. **PAD vector baseline.** The agent's personality baseline (see CorticalState initialization in [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md)) defines a preferred affect state. The agent prefers to return to its baseline PAD, creating a homeostatic drive.

```rust
/// Preferred outcomes for EFE computation.
pub struct PreferredOutcomes {
    /// Per-signal preferred category distributions.
    /// Values are log-probabilities: ln P(o|C).
    pub preferences: HashMap<SignalId, Vec<f64>>,
}

impl PreferredOutcomes {
    /// Build from task spec + PAD baseline.
    pub fn from_task_and_baseline(
        task: &TaskSpec,
        pad_baseline: &PadVector,
    ) -> Self {
        let mut prefs = HashMap::new();

        // Task-derived preferences
        for criterion in &task.success_criteria {
            let (signal, preferred_category) = criterion.to_signal_preference();
            let mut log_probs = vec![-10.0; signal.num_categories()]; // low default
            log_probs[preferred_category] = 0.0; // ln(1.0) = 0.0
            prefs.insert(signal, log_probs);
        }

        // PAD baseline preferences (homeostatic)
        prefs.insert(SignalId::Pleasure,
            pad_to_category_log_probs(pad_baseline.pleasure));
        prefs.insert(SignalId::Arousal,
            pad_to_category_log_probs(pad_baseline.arousal));
        prefs.insert(SignalId::Dominance,
            pad_to_category_log_probs(pad_baseline.dominance));

        Self { preferences: prefs }
    }
}
```

**Temporal discounting.** Future outcomes are discounted geometrically: `gamma^t` where `gamma = 0.95` and `t` is the number of ticks into the future. This is standard exponential discounting, not a flat sum.

Why geometric over flat? Flat sums treat an outcome 100 ticks away as equally valuable as one 1 tick away. For an agent operating on 10-second gamma intervals, an outcome 100 ticks away is ~17 minutes in the future. The environment will have changed. Geometric discounting with gamma = 0.95 means an outcome 20 ticks from now is worth `0.95^20 = 0.36` of its face value, and 100 ticks out is worth `0.95^100 = 0.006`. This matches the planning horizon: the agent should care about the next few minutes, not the next hour.

| Ticks ahead | Seconds | Discount (gamma=0.95) |
|---|---|---|
| 1 | 10s | 0.950 |
| 5 | 50s | 0.774 |
| 10 | 100s | 0.599 |
| 20 | 200s | 0.358 |
| 50 | 500s | 0.077 |
| 100 | 1000s | 0.006 |

### Epistemic value estimation without future observations

The EFE epistemic term `E_Q[H[P(o_τ|s_τ)]]` requires estimating how much uncertainty a future observation will resolve. The problem: the agent does not have the future observation yet.

**Bootstrap from past accuracy.** The agent estimates expected information gain from historical data: for each tier, how much did prediction accuracy improve after invoking that tier?

```rust
/// Estimate epistemic value of a tier from historical accuracy changes.
pub struct EpistemicEstimator {
    /// Per-tier: rolling window of (accuracy_before, accuracy_after) pairs.
    history: HashMap<InferenceTier, VecDeque<(f32, f32)>>,
    /// Window size.
    window: usize,  // default: 50
}

impl EpistemicEstimator {
    /// Average accuracy improvement for this tier over the last `window` uses.
    pub fn expected_info_gain(&self, tier: InferenceTier) -> f32 {
        let entries = match self.history.get(&tier) {
            Some(h) if !h.is_empty() => h,
            _ => return self.default_info_gain(tier),
        };

        let total_improvement: f32 = entries.iter()
            .map(|(before, after)| (after - before).max(0.0))
            .sum();

        total_improvement / entries.len() as f32
    }

    /// Fallback for tiers with no history yet.
    fn default_info_gain(&self, tier: InferenceTier) -> f32 {
        match tier {
            InferenceTier::T0 => 0.0,    // no LLM, no new info
            InferenceTier::T1 => 0.05,   // fast model, modest info gain
            InferenceTier::T2 => 0.15,   // full model, substantial info gain
        }
    }
}
```

The default values (0.0 / 0.05 / 0.15) reflect the structural expectation: T0 gains no new information, T1 gains some, T2 gains the most. These defaults are replaced by empirical estimates within 50 ticks of each tier being used.

### Applied to Tier Selection

For Roko's tier selection, the "policy" is the tier choice (T0, T1, T2) and the EFE formula becomes:

```
EFE(tier) = pragmatic_value(tier) + epistemic_value(tier) - cost(tier)

where:
  pragmatic_value(T0) = value of applying existing playbook rules (low if no matching rule)
  pragmatic_value(T1) = value of quick assessment + possible action (medium)
  pragmatic_value(T2) = value of deep analysis + comprehensive action (high)

  epistemic_value(T0) = 0 (no new information gained — only existing model used)
  epistemic_value(T1) = moderate (fast LLM provides some uncertainty reduction)
  epistemic_value(T2) = high (full LLM with complete context maximally reduces uncertainty)

  cost(T0) = 0
  cost(T1) = $0.001-0.003 + 200-500ms latency
  cost(T2) = $0.01-0.25 + 1-5s latency
```

The optimal tier is the one that maximizes `pragmatic_value + epistemic_value - cost`. When uncertainty is low (probes report nothing unusual), T0's zero cost dominates — there's no epistemic value to gain and no pragmatic value in redundant analysis. When uncertainty is high, T2's high epistemic value justifies its cost.

### Zero Hyperparameters

The elegance of the EFE formulation is that it requires **zero hyperparameters** for the exploration/exploitation tradeoff. Traditional approaches (epsilon-greedy, UCB, Thompson sampling) all require tuning:
- Epsilon-greedy: what ε?
- UCB: what exploration bonus coefficient?
- Thompson sampling: what prior distribution?

EFE naturally balances exploration and exploitation through the two terms of the formula. When epistemic value is high (uncertainty is large), the agent explores. When pragmatic value dominates (goals are clear, uncertainty is low), the agent exploits. The balance shifts automatically as the agent learns.

This is Friston's key insight: exploration and exploitation are not opposing objectives that require a tradeoff parameter — they are **two aspects of the same objective** (minimizing expected free energy).

---

## EFE for Context Selection

Active inference also applies to **context selection** — choosing which Engrams to include in the LLM's context window. Each potential context entry has an expected free energy:

```
EFE(entry) = -E[utility_gain | entry_included] + E[H_reduction | entry_included] - token_cost(entry)
```

Entries with high expected utility gain (they help the agent achieve its goal) and high expected uncertainty reduction (they resolve an open question) are prioritized. Entries that would consume many tokens without reducing uncertainty or improving outcomes are deprioritized.

This produces a principled alternative to top-k retrieval or manual priority ordering. The `PredictiveScorer` (target implementation in `roko-core`) computes this EFE approximation for each candidate Engram:

```rust
/// Scores Engrams using an active inference EFE approximation.
///
/// Each Engram is evaluated for:
/// 1. Pragmatic value: how useful is this for achieving current goals?
/// 2. Epistemic value: how much uncertainty does this resolve?
/// 3. Token cost: how many tokens does this consume?
///
/// The score is EFE = pragmatic + epistemic - cost_penalty.
pub struct PredictiveScorer {
    /// How much to weight pragmatic value relative to epistemic
    /// Note: this is NOT a tradeoff parameter — both are positive
    /// contributions. This weights domain-specific pragmatic metrics.
    pragmatic_weight: f32,
}

impl Scorer for PredictiveScorer {
    fn score(&self, engram: &Engram, ctx: &Context) -> Score {
        let pragmatic = self.compute_pragmatic_value(engram, ctx);
        let epistemic = self.compute_epistemic_value(engram, ctx);
        let cost_penalty = engram.estimated_tokens() as f32 / 1000.0 * 0.01;

        let effective = pragmatic * self.pragmatic_weight
            + epistemic
            - cost_penalty;

        Score {
            confidence: engram.score.confidence,
            novelty: epistemic,      // epistemic value IS novelty
            utility: pragmatic,      // pragmatic value IS utility
            reputation: engram.score.reputation,
            precision: engram.score.precision,
            salience: effective.max(0.0),  // effective EFE as salience
            coherence: engram.score.coherence,
        }
    }

    fn name(&self) -> &'static str { "predictive_scorer" }
}
```

### PredictiveScorer: pragmatic weight and token cost

The `PredictiveScorer` shown above has two parameters that need specification.

**`pragmatic_weight` default and tuning.** Default: `1.0`. This is not a tradeoff knob between exploitation and exploration -- both pragmatic and epistemic terms are positive contributions to the EFE score. The weight exists because pragmatic value is domain-specific and measured on an arbitrary scale, while epistemic value is measured in nats (natural units of information). The weight normalizes pragmatic value relative to epistemic value.

Per-agent tuning: agents with well-defined task specifications (e.g., "compile this code") should use a higher pragmatic weight (1.5-2.0) because their pragmatic value estimates are reliable. Agents with open-ended tasks (e.g., "research this topic") should use a lower pragmatic weight (0.5-0.8) because pragmatic value is harder to estimate and epistemic exploration is more valuable.

| Agent role | pragmatic_weight | Rationale |
|---|---|---|
| Compilation/test runner | 2.0 | Clear success criteria, reliable utility estimates |
| Code generator | 1.5 | Task has defined output, moderate uncertainty |
| General purpose (default) | 1.0 | Balanced |
| Research/exploration | 0.7 | High uncertainty, exploration-heavy |
| Brainstorming/creative | 0.5 | Epistemic value dominates |

Configured in `roko.toml`:
```toml
[agent.scoring]
pragmatic_weight = 1.0  # default, override per-agent in [agents.<name>.scoring]
```

**Token cost penalty derivation.** The formula `engram.estimated_tokens() / 1000.0 * 0.01` means each 1,000 tokens costs 0.01 salience points. This is calibrated to the cost model:

- A T1 call with 4,000 tokens costs ~$0.002. At 0.01 per 1,000 tokens, the cost penalty for filling the entire T1 budget is 0.04 salience points.
- A T2 call with 32,000 tokens costs ~$0.10. The cost penalty for filling the entire T2 budget is 0.32 salience points.
- For a typical Engram (500 tokens), the cost penalty is 0.005 -- negligible compared to the pragmatic/epistemic terms (typically 0.1-0.5 each). This is intentional: the penalty is a tiebreaker between equally useful entries of different sizes, not a primary ranking factor.

If inference costs change (cheaper models, caching improvements), adjust the coefficient. The formula becomes `tokens / 1000.0 * cost_per_1k_tokens / normalization_constant`. The normalization constant should keep the penalty in the range [0.001, 0.1] for a typical entry.

### The full EFEEstimate struct

```rust
/// Complete EFE estimate for a tier or context entry.
///
/// Captures all terms of the Expected Free Energy computation
/// for debugging, logging, and threshold adaptation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EFEEstimate {
    /// What this estimate is for: a tier, a model, or a context entry.
    pub target: EFETarget,

    /// Pragmatic value: expected utility of preferred outcomes.
    /// Higher = more goal-aligned actions expected.
    pub pragmatic_value: f64,

    /// Epistemic value: expected information gain (uncertainty reduction).
    /// Higher = more uncertainty resolved.
    pub epistemic_value: f64,

    /// Cost: inference cost + latency cost, normalized.
    pub cost: f64,

    /// Token cost component (subset of cost, for context entries).
    pub token_cost: f64,

    /// Net EFE: pragmatic + epistemic - cost.
    /// The tier/entry with the highest net EFE is selected.
    pub net_efe: f64,

    /// Temporal discount applied (gamma^t for future outcomes).
    pub discount: f64,

    /// Number of past observations informing this estimate.
    pub observation_count: u64,

    /// Confidence in this estimate (0.0 = no data, 1.0 = converged).
    /// Computed as min(observation_count / 200, 1.0).
    pub confidence: f64,
}

/// What the EFE estimate targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EFETarget {
    /// Tier selection (T0/T1/T2).
    Tier(InferenceTier),
    /// Model selection within a tier.
    Model(String),
    /// Context entry (Engram) inclusion.
    ContextEntry { engram_id: String },
}

impl EFEEstimate {
    /// Compute net EFE from components.
    pub fn compute(
        pragmatic_value: f64,
        epistemic_value: f64,
        cost: f64,
        discount: f64,
    ) -> f64 {
        discount * (pragmatic_value + epistemic_value) - cost
    }
}
```

---

## Connection to the CascadeRouter

The `CascadeRouter` in `roko-learn/src/cascade_router.rs` implements a three-stage cascade (Static → Confidence → UCB1) for model selection within a tier. Active inference provides the theoretical grounding for the UCB1 stage:

- **Static stage** (< 50 observations): Uses a fixed routing table because there isn't enough data for principled exploration. This is the "prior" phase.
- **Confidence stage** (50-200 observations): Routes by confidence score, which approximates pragmatic value. Exploration happens via a fixed probability (epsilon-like).
- **UCB1 stage** (> 200 observations): Contextual bandit with LinUCB. The UCB exploration bonus approximates epistemic value — models with fewer observations get higher exploration bonuses because there's more information to gain from trying them.

The target `ActiveInferenceRouter` (not yet implemented) replaces UCB1's heuristic exploration bonus with the full EFE computation, making the exploration/exploitation tradeoff principled rather than approximated.

### LinUCB feature vector specification

The UCB1 stage uses LinUCB (Li et al. 2010, "A Contextual-Bandit Approach to Personalized News Article Recommendation") for contextual model selection. The feature vector phi(context) encodes the current tick state as a fixed-dimension input to the linear bandit:

```rust
/// Feature vector for LinUCB contextual bandit.
///
/// 12-dimensional: encodes the tick state that determines
/// which model performs best.
pub struct BanditFeatures {
    pub features: [f64; 12],
}

impl BanditFeatures {
    /// Extract features from current tick state.
    pub fn from_tick_state(state: &TickState) -> Self {
        Self {
            features: [
                // Prediction error components (4 dims)
                state.prediction_error as f64,
                state.anomaly_count as f64 / 16.0,   // normalized
                state.drift as f64,
                if state.regime_changed { 1.0 } else { 0.0 },

                // Affect state (3 dims, from CorticalState PAD)
                state.pad.pleasure,
                state.pad.arousal,
                state.pad.dominance,

                // Resource state (2 dims)
                state.budget_usage_pct as f64,
                state.resource_health as f64,

                // Task complexity proxy (2 dims)
                (state.task_token_estimate as f64).ln().max(0.0) / 12.0,
                state.task_retry_count as f64 / 10.0,

                // Bias term (1 dim)
                1.0,
            ],
        }
    }
}
```

**Confidence ellipsoid radius (alpha).** LinUCB selects the model (arm) with the highest `theta^T * phi + alpha * sqrt(phi^T * A_inv * phi)`. The `alpha` parameter controls exploration:

- Default: `alpha = 0.5`. This produces moderate exploration -- about 15-20% of selections go to non-greedy arms when the bandit has 200+ observations.
- Range: [0.1, 2.0]. Below 0.1, the bandit barely explores and can get stuck on a suboptimal model. Above 2.0, exploration dominates and convergence is slow.
- The target ActiveInferenceRouter replaces this fixed alpha with the epistemic value term from EFE, which adapts automatically.

```rust
/// LinUCB arm for a single model.
pub struct LinUCBArm {
    /// A matrix: d x d, where d = feature dimension (12).
    pub a_matrix: Vec<Vec<f64>>,
    /// b vector: d x 1, accumulated reward-weighted features.
    pub b_vector: Vec<f64>,
    /// Feature dimension.
    pub d: usize,
}

impl LinUCBArm {
    pub fn new(d: usize) -> Self {
        // A starts as identity matrix
        let mut a = vec![vec![0.0; d]; d];
        for i in 0..d {
            a[i][i] = 1.0;
        }
        Self {
            a_matrix: a,
            b_vector: vec![0.0; d],
            d,
        }
    }

    /// Compute the UCB score for this arm given features.
    pub fn score(&self, features: &[f64], alpha: f64) -> f64 {
        let a_inv = invert_matrix(&self.a_matrix);
        let theta: Vec<f64> = mat_vec_mul(&a_inv, &self.b_vector);
        let expected = dot(&theta, features);
        let uncertainty = (quad_form(features, &a_inv)).sqrt();
        expected + alpha * uncertainty
    }

    /// Update after observing reward for this arm.
    pub fn update(&mut self, features: &[f64], reward: f64) {
        // A += features * features^T
        for i in 0..self.d {
            for j in 0..self.d {
                self.a_matrix[i][j] += features[i] * features[j];
            }
        }
        // b += reward * features
        for i in 0..self.d {
            self.b_vector[i] += reward * features[i];
        }
    }
}
```

### Configuration parameters

| Parameter | Default | Range | Where |
|---|---|---|---|
| `pragmatic_weight` | 1.0 | [0.3, 3.0] | `roko.toml` `[agent.scoring]` |
| `token_cost_coeff` | 0.01 | [0.001, 0.1] | `roko.toml` `[agent.scoring]` |
| `temporal_discount` | 0.95 | [0.80, 0.99] | `roko.toml` `[heartbeat.active_inference]` |
| `linucb_alpha` | 0.5 | [0.1, 2.0] | `roko.toml` `[heartbeat.cascade]` |
| `bootstrap_ticks` | 50 | [20, 100] | `roko.toml` `[heartbeat.active_inference]` |
| `transition_ticks` | 200 | [100, 500] | `roko.toml` `[heartbeat.active_inference]` |
| `epistemic_window` | 50 | [20, 100] | `roko.toml` `[heartbeat.active_inference]` |
| `dirichlet_prior` | 1.0 | [0.1, 10.0] | `roko.toml` `[heartbeat.active_inference]` |
| `feature_dim` | 12 | fixed | Code constant |

### Error handling

| Failure mode | Behavior |
|---|---|
| Generative model Q has no data (cold start) | Use flat Dirichlet prior, default to T2 |
| LinUCB A matrix is singular (numerical instability) | Add epsilon (1e-6) to diagonal before inversion. Log warning. |
| EFE net value is NaN (bad inputs) | Fall back to heuristic threshold from doc 08. Log error. |
| Preferred outcomes C not specified (no task spec) | Use PAD baseline only. Pragmatic value defaults to 0.0. |
| Feature extraction fails (missing CorticalState) | Use zero vector with bias = 1.0. Log warning. |

### Integration wiring

1. `PredictiveScorer` is registered as the default `Scorer` implementation in `roko-core`.
2. The `Composer` calls `PredictiveScorer::score()` when ranking Engrams for context assembly.
3. `CascadeRouter` (Stage 3) uses `LinUCBArm::score()` for model selection within a tier.
4. `EFEEstimate` records are persisted to `.roko/learn/efe-estimates.jsonl` for post-hoc analysis.
5. The `GenerativeModel` persists to `.roko/learn/generative-model.json` and is loaded on resume.

### Test criteria

| Test | Assertion |
|---|---|
| Flat Dirichlet prior produces uniform predictions | All `expected_prob(k)` equal for k in 0..num_categories |
| Observing category k increases its probability | `expected_prob(k)` after observe > before |
| EFE(T0) > EFE(T1) when prediction error = 0.0 | T0 wins because cost = 0 and epistemic gain = 0 everywhere |
| EFE(T2) > EFE(T1) when prediction error = 0.80 | T2 wins because epistemic gain justifies cost |
| Temporal discount at t=0 is 1.0 | `0.95^0 = 1.0` |
| Temporal discount at t=20 is ~0.358 | `0.95^20 = 0.3585` |
| LinUCB A matrix starts as identity | Diagonal = 1.0, off-diagonal = 0.0 |
| LinUCB update increases confidence | Confidence ellipsoid shrinks after update |
| Token cost penalty for 500 tokens = 0.005 | `500 / 1000.0 * 0.01` |
| EFEEstimate round-trips through serde | Serialize + deserialize preserves all fields |

---

## Relationship to Sims' Rational Inattention

Sims (2003, "Implications of rational inattention", Journal of Monetary Economics 50(3)) provides a complementary perspective: agents have finite information processing capacity and should allocate it optimally. The key result is that agents should pay more attention to channels (information sources) with higher signal-to-noise ratio relative to their processing cost.

For Roko, Sims' framework explains why T0 probes are valuable: they have extremely high signal-to-noise ratio (deterministic checks with no stochasticity) at zero processing cost. An agent should maximally exploit these cheap, reliable signals before investing in expensive, stochastic LLM analysis.

The rational inattention framework also explains the VCG Attention Auction (see [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md)): context sections with higher signal-to-noise ratio (measured by expected utility) should receive more token budget. The auction mechanism discovers the optimal allocation through truthful bidding.

---

## Implementation Path

The active inference compute allocation is implemented in stages:

### Stage 1: Heuristic Threshold (Current)

The current implementation uses prediction error vs. adaptive threshold — a heuristic approximation of EFE. This is what [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) describes.

### Stage 2: PredictiveScorer (Near-term)

Add the `PredictiveScorer` to `roko-core` for EFE-based context selection. This doesn't change the tier gating — it improves context assembly within T1/T2.

### Stage 3: ActiveInferenceRouter (Target)

Replace the UCB1 stage of `CascadeRouter` with a full EFE computation for both tier selection and model routing. This requires:
- Maintaining a probabilistic world model (approximate posterior Q)
- Computing expected observations under each policy
- Estimating uncertainty reduction for each tier/model choice
- Tracking preferred outcomes (C matrix) from task specifications

See [11-active-inference-state-space.md](./11-active-inference-state-space.md) for the factorized discrete POMDP that makes Stage 3 tractable.

---

## Academic Foundations

- **Friston 2010** — "The free-energy principle: a unified brain theory?" (Nature Reviews Neuroscience 11(2)). The foundational paper on free energy minimization in biological cognition.
- **Friston et al. 2015** — "Active inference and epistemic value" (Cognitive Neuroscience 6(4)). Extension to expected free energy for action selection.
- **Parr & Friston 2017** — "Working memory, attention, and salience in active inference" (Scientific Reports 7). Attention as precision weighting in active inference.
- **Sims 2003** — "Implications of rational inattention" (Journal of Monetary Economics 50(3)). Optimal allocation of finite information processing capacity.
- **Kahneman 2011** — "Thinking, Fast and Slow" (Farrar, Straus and Giroux). System 1/System 2 as the cognitive basis.
- **Chen et al. 2023** — FrugalGPT (arXiv:2305.05176). Cascade routing as a practical implementation of compute-optimal allocation.
- **Koudahl et al. 2024** — (arXiv:2412.10425). Factorized discrete POMDP for tractable active inference.
- **VERSES AI** — Genius platform. Industrial deployment of active inference for agent cognition.

---

## Current Status and Gaps

**What exists:**
- `InferenceTier` and `TierRouter` in `bardo-primitives/src/tier.rs` — the heuristic threshold approach.
- `CascadeRouter` with UCB1 stage in `roko-learn/src/cascade_router.rs` — approximates exploration/exploitation.
- Prediction error concept in legacy heartbeat specification.

**What is missing:**
- `PredictiveScorer` implementing EFE approximation for context scoring.
- `ActiveInferenceRouter` replacing UCB1 with full EFE computation.
- Probabilistic world model (approximate posterior Q) for tier selection.
- Preferred outcome specification (C matrix) from task goals.
- Integration of Sims' rational inattention for attention budget allocation.

---

## Cross-References

- See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for the heuristic tier gating this formalizes
- See [09-16-t0-probes.md](./09-16-t0-probes.md) for the probes that generate prediction error signals
- See [11-active-inference-state-space.md](./11-active-inference-state-space.md) for the factorized POMDP state space
- See [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md) for VCG-based context budget allocation
- See topic [05-learning](../05-learning/INDEX.md) for the CascadeRouter and bandit algorithms
