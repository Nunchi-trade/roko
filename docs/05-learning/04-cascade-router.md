# Cascade Router

> **Crate:** `roko-learn` · **Module:** `cascade_router.rs`
> **Persistence:** `.roko/learn/cascade-router.json`
> **Wiring:** `LearningRuntime` → `CascadeRouter::select()` (called from orchestrate.rs)
> **Cross-references:** [03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md), [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md), [10-pareto-frontier-pruning](10-pareto-frontier-pruning.md), [08-cost-normalization](08-cost-normalization.md)


> **Implementation**: Shipping

---

## Purpose

The cascade router is Roko's central model selection system. It answers the question: "Given a task with these features (category, complexity, role, iteration, crate familiarity), which LLM model should run it?" The answer evolves as the system accumulates observations, transitioning through three stages of increasing sophistication.

The cascade design is inspired by production routing systems (RouteLLM, Ong et al. ICLR 2025; FrugalGPT, Chen et al. arXiv:2305.05176; AutoMix, NeurIPS 2024) but adapted for a self-hosted development tool where the reward signal (gate pass/fail) is deterministic and the decision space (which model to route to) is small enough for contextual bandits rather than neural routers.

---

## Three-Stage Cascade

The router transitions through three stages as observation count grows:

```
┌─────────────────────────────────────────────────────────────────┐
│  Stage 1: Static          │  < 50 observations                  │
│  Hardcoded role→model     │  No learning, safe defaults          │
│  table                    │                                      │
├───────────────────────────┼──────────────────────────────────────┤
│  Stage 2: Confidence      │  50 – 200 observations               │
│  Empirical pass rates +   │  Simple statistics, wide confidence  │
│  confidence intervals     │  intervals shrink with data          │
├───────────────────────────┼──────────────────────────────────────┤
│  Stage 3: UCB             │  > 200 observations                  │
│  Full LinUCB contextual   │  Context-dependent routing with      │
│  bandit                   │  learned feature weights              │
└─────────────────────────────────────────────────────────────────┘
```

### Why Three Stages?

A single bandit algorithm works poorly at all scales:

- **Too few observations for UCB**: LinUCB with 18 context dimensions needs ~50 observations per arm to begin producing meaningful weights. With 5 models, that's 250+ observations before the bandit is useful. During cold start, random exploration wastes money on expensive models for trivial tasks.
- **Too crude for static forever**: A hardcoded table can never adapt to crate-specific patterns, role-specific model preferences, or changes in model capabilities after a provider update.
- **Confidence stage bridges the gap**: Between 50 and 200 observations, simple pass-rate statistics with confidence intervals provide reasonable routing without the sample complexity requirements of a 18-dimensional linear model.

---

## Stage 1: Static Routing (< 50 observations)

Before the system has enough data to learn, it uses a hardcoded mapping from `ModelTier` to model slug:

```rust
fn static_route(tier: ModelTier) -> ModelSpec {
    match tier {
        ModelTier::Fast    => ModelSpec::new("claude-haiku-4-5-20251001"),
        ModelTier::Standard => ModelSpec::new("claude-sonnet-4-20250514"),
        ModelTier::Complex => ModelSpec::new("claude-opus-4-20250514"),
    }
}
```

The tier is determined by the task's complexity band and role. This mapping is deliberately conservative: it over-routes to stronger models to avoid gate failures during the cold-start period, accepting higher cost in exchange for higher pass rates while the system builds its observation base.

---

## Stage 2: Confidence Routing (50–200 observations)

Once 50 observations have accumulated, the router transitions to empirical pass-rate routing with confidence intervals.

### Per-Model Statistics

```rust
struct ModelStats {
    trials: u64,      // selections for this model
    successes: u64,   // gate passes
}

impl ModelStats {
    fn pass_rate(&self) -> f64 {
        if self.trials == 0 { 0.0 }
        else { self.successes as f64 / self.trials as f64 }
    }
}
```

### Selection Algorithm

For each candidate model:

```
score(model) = pass_rate(model) − cost_penalty(model) + affinity_bonus(model)
```

where:
- `cost_penalty` = normalized cost relative to the cheapest available model
- `affinity_bonus` = `CACHE_AFFINITY_BONUS` (0.15) if the model matches the previous task's model

Additional biases from the C-Factor and affect system:

- **Low affect confidence** (< `LOW_AFFECT_CONFIDENCE_THRESHOLD` = 0.3): bias toward stronger models.
- **High C-Factor** (> `HIGH_CFACTOR_THRESHOLD` = 0.8): bias toward cheaper models (system is performing well, can afford to save).
- **Low C-Factor** (< `LOW_CFACTOR_THRESHOLD` = 0.4): bias toward stronger models (system is struggling, need to invest in quality).

### Transition Threshold

The `CONFIDENCE_TO_UCB_THRESHOLD = 200` observation count triggers transition to stage 3. This threshold was chosen because:
- 200 observations with 5 models gives ~40 per model.
- LinUCB with 18 dimensions needs ~2× the dimension count in observations per arm for stable weights, so 36+ per arm.
- 200 provides a comfortable margin above this minimum.

---

## Stage 3: UCB Routing (> 200 observations)

At 200+ observations, the full `LinUCBRouter` contextual bandit takes over. See [03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md) for the algorithm details.

The LinUCB stage uses the 18-dimensional `RoutingContext` to make context-dependent decisions. This means the router can learn patterns like:

- "For `roko-core` crate with high familiarity, haiku is sufficient."
- "For cross-crate refactoring on retry (iteration > 0), use opus."
- "When the previous model was sonnet and it failed, escalate to opus rather than retrying sonnet."

---

## CascadeModel Output

The router returns a `CascadeModel` containing routing advice:

```rust
pub struct CascadeModel {
    /// Primary model to use.
    pub primary: ModelSpec,
    /// Fallback model if the primary fails or times out.
    pub fallback: Option<ModelSpec>,
    /// Latency SLA in milliseconds.
    pub latency_sla_ms: u64,
    /// Which cascade stage produced this recommendation.
    pub stage: CascadeStage,
}
```

The `fallback` field provides a pre-computed escalation target. If the primary model fails (gate failure, timeout, or provider error), the orchestrator can immediately retry with the fallback without re-querying the router. This avoids a round-trip through the cascade during time-critical retry scenarios.

---

## Provider Health Integration

The cascade router integrates with the `ProviderHealthRegistry` to avoid routing to unhealthy providers:

```
CascadeRouter::select(context)
    │
    ├── 1. Compute candidate scores (per stage algorithm)
    │
    ├── 2. Filter: ProviderHealthRegistry::is_available(model.provider)
    │       → Remove models whose provider circuit breaker is Open
    │
    ├── 3. Filter: Pareto frontier pruning
    │       → Remove dominated models (worse on both cost and quality)
    │
    └── 4. Select highest-scoring non-filtered model
```

See [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md) for the circuit breaker algorithm and [10-pareto-frontier-pruning](10-pareto-frontier-pruning.md) for the Pareto filter.

---

## C-Factor Integration

The cascade router uses the C-Factor (Collective Capability Factor) as a routing bias:

```rust
pub enum AgentDispatchBias {
    PreferStronger,   // C-Factor < 0.4 — system struggling
    PreferCheaper,    // C-Factor > 0.8 — system performing well
    Neutral,          // C-Factor 0.4–0.8 — no bias
}
```

The C-Factor is computed from a composite of gate pass rate, cost efficiency, speed, first-try rate, knowledge growth, and turn-taking equality across recent episodes. A high C-Factor indicates the system is performing well and can afford to route to cheaper models; a low C-Factor indicates the system needs investment in quality.

See [15-collective-calibration-31x](15-collective-calibration-31x.md) for the C-Factor computation and its theoretical basis.

---

## Pareto Frontier Pruning

Before presenting candidates to the bandit, the cascade router computes a Pareto frontier over `(pass_rate, cost_per_success)`:

```
Model A: pass_rate=0.90, cost/success=$10.00  → Pareto-optimal
Model B: pass_rate=0.70, cost/success=$12.00  → DOMINATED by A (worse on both)
Model C: pass_rate=0.80, cost/success=$9.00   → Pareto-optimal (lower cost than A)
```

Only Pareto-optimal models are presented to the bandit. This reduces the arm set and prevents the bandit from wasting exploration budget on clearly inferior models.

The Pareto frontier is recomputed every `PARETO_RECOMPUTE_INTERVAL = 50` observations to keep it current as model statistics evolve.

See [10-pareto-frontier-pruning](10-pareto-frontier-pruning.md) for the full algorithm.

---

## Persistence

The cascade router persists its full state to `.roko/learn/cascade-router.json`:

```json
{
  "observations": 347,
  "stage": "ucb",
  "model_stats": {
    "claude-haiku-4-5-20251001": { "trials": 89, "successes": 71 },
    "claude-sonnet-4-20250514": { "trials": 156, "successes": 108 },
    "claude-opus-4-20250514": { "trials": 102, "successes": 89 }
  },
  "linucb_state": {
    "arms": { ... },
    "observation_count": 347
  },
  "pareto_frontier": ["claude-haiku-4-5-20251001", "claude-opus-4-20250514"]
}
```

State is loaded on startup and saved after each routing decision update. The atomic tempfile+rename pattern ensures crash safety.

---

## Operating Frequency

The cascade router operates at **per-episode frequency** — every agent turn produces one routing update. This is the highest-frequency learning loop in the system.

See [14-stability-mechanisms](14-stability-mechanisms.md) for how frequency separation across subsystems prevents oscillation.

---

## Cascade Router with Lookahead

Current routing considers only the immediate task. Lookahead routing predicts the sequence of upcoming tasks and makes routing decisions that optimize across the sequence — choosing a slightly more expensive model now if it will enable cheaper routing for subsequent tasks via KV cache reuse.

### Sequence-Aware Routing

```rust
pub struct LookaheadRouter {
    /// Base cascade router for individual decisions.
    inner: CascadeRouter,
    /// Task dependency graph for lookahead.
    task_graph: TaskDag,
    /// Lookahead horizon (default: 3 tasks ahead).
    pub horizon: usize,
    /// Discount factor for future savings (default: 0.9).
    pub gamma: f64,
    /// KV cache reuse probability model.
    cache_model: CacheReuseModel,
}

pub struct CacheReuseModel {
    /// Per-(model, role) estimated cache hit rate when reusing same model.
    cache_hit_rates: HashMap<(String, String), f64>,
    /// Average input tokens saved per cache hit.
    avg_tokens_saved_per_hit: u64,
    /// Cost per 1M tokens for cache reads vs fresh input.
    cache_read_discount: f64,
}
```

### Lookahead Algorithm

```
fn select_with_lookahead(current_task, upcoming_tasks, horizon):
    // Get upcoming tasks from DAG (respecting dependencies)
    window = [current_task] + upcoming_tasks[..horizon]

    best_total_cost = infinity
    best_assignment = None

    for each candidate_model for current_task:
        // Compute immediate cost
        immediate_cost = estimated_cost(candidate_model, current_task)

        // Compute expected future savings from cache reuse
        future_savings = 0.0
        for i in 1..window.len():
            // If future task uses same model, cache reuse saves tokens
            p_cache = cache_model.hit_rate(candidate_model, window[i].role)
            tokens_saved = p_cache × cache_model.avg_tokens_saved_per_hit
            cost_saved = tokens_saved × cache_model.cache_read_discount / 1_000_000
            future_savings += gamma^i × cost_saved

        total_cost = immediate_cost - future_savings

        if total_cost < best_total_cost:
            best_total_cost = total_cost
            best_assignment = candidate_model

    return best_assignment
```

### When Lookahead Matters

Lookahead routing provides significant savings when:

| Condition | Savings | Mechanism |
|-----------|---------|-----------|
| Sequential tasks on same crate | 15-30% | KV cache reuse for crate context |
| Same role across consecutive tasks | 10-20% | System prompt caching |
| Plan with >10 tasks | 5-15% compound | Amortized model selection overhead |
| Mixed complexity plan | 10-25% | Route easy tasks cheap, cache for hard |

Lookahead provides minimal benefit when tasks are independent (no shared context), when the plan has very few tasks, or when all tasks require different models.

### Connection to Speculative Decoding

Lookahead routing is analogous to speculative decoding (Leviathan et al. 2023) applied at the task level rather than the token level. Where speculative decoding predicts future tokens to reduce latency, lookahead routing predicts future tasks to reduce cost. The SpecRouter framework (2025) demonstrates that treating LLM inference as an adaptive routing problem — dynamically constructing inference "paths" — can significantly reduce end-to-end cost. Roko's lookahead extends this insight from intra-request to inter-request optimization.

---

## Router Calibration

The cascade router's decisions are only as good as its internal estimates of model performance. Router calibration ensures that the router's confidence maps to actual performance — when the router estimates 80% pass probability for a model, that model should actually succeed approximately 80% of the time.

### Calibration Framework

```rust
pub struct RouterCalibration {
    /// Per-model calibration data.
    calibrations: HashMap<String, ModelCalibration>,
    /// Overall calibration score (lower is better, 0 = perfect).
    pub brier_score: f64,
    /// Recalibration interval (default: every 100 routing decisions).
    pub recalibrate_interval: u32,
}

pub struct ModelCalibration {
    /// Model slug.
    pub model: String,
    /// Predicted pass probabilities and actual outcomes.
    predictions: Vec<(f64, bool)>,
    /// Calibration bins (10 bins, 0-10%, 10-20%, ..., 90-100%).
    pub bins: [CalibrationBin; 10],
    /// Platt scaling parameters: a, b for sigmoid(a × raw_score + b).
    pub platt_a: f64,
    pub platt_b: f64,
    /// Isotonic regression mapping (non-parametric calibration).
    pub isotonic_map: Vec<(f64, f64)>,
}

pub struct CalibrationBin {
    /// Bin range (e.g., 0.7 to 0.8).
    pub lower: f64,
    pub upper: f64,
    /// Number of predictions in this bin.
    pub count: u32,
    /// Actual success rate within this bin.
    pub actual_rate: f64,
    /// Expected Calibration Error for this bin.
    pub ece_contribution: f64,
}
```

### Calibration Methods

**1. Platt Scaling (parametric)**

Fits a logistic regression on top of the router's raw confidence scores:

```
calibrated_probability = sigmoid(a × raw_score + b)
```

Parameters `a` and `b` are fit by minimizing log-loss on a held-out validation set of recent routing decisions. Platt scaling is fast (O(n) fitting), requires few samples (~50), and handles monotonic miscalibration well.

**2. Isotonic Regression (non-parametric)**

Fits a non-decreasing step function mapping raw scores to calibrated probabilities. More flexible than Platt scaling — handles non-monotonic miscalibration — but requires more data (~200 samples) and can overfit with small datasets.

**3. Temperature Scaling**

The simplest calibration: divide raw logits by a learned temperature T before applying softmax.

```
calibrated_score = raw_score / T
```

T > 1 reduces overconfidence. T < 1 reduces underconfidence. T is fit to minimize negative log-likelihood on validation data.

### Expected Calibration Error (ECE)

The primary calibration metric, computed over B bins:

```
ECE = Σ_{b=1}^{B} (n_b / N) × |accuracy_b - confidence_b|
```

where `n_b` is the number of predictions in bin b, `accuracy_b` is the actual success rate, and `confidence_b` is the average predicted probability. ECE = 0 means perfect calibration.

| ECE Range | Interpretation | Action |
|-----------|---------------|--------|
| < 0.05 | Well calibrated | No action needed |
| 0.05 - 0.10 | Slightly miscalibrated | Monitor |
| 0.10 - 0.20 | Miscalibrated | Apply Platt scaling |
| > 0.20 | Severely miscalibrated | Investigate data distribution shift |

### Auto-Recalibration

The router recalibrates automatically every `recalibrate_interval` decisions:

```
Every 100 routing decisions:
    1. Collect last 200 (predicted_probability, actual_outcome) pairs
    2. Compute ECE
    3. If ECE > 0.10:
       a. Fit Platt scaling parameters (a, b) via gradient descent
       b. Validate on held-out 20% of data
       c. If Platt-calibrated ECE < original ECE:
          → Apply Platt scaling to all future predictions
       d. Else: fit isotonic regression as fallback
    4. Log calibration metrics to .roko/learn/calibration.jsonl
```

### Connection to Mixture-of-Experts Routing

The cascade router's calibration challenge is analogous to the load-balancing problem in Mixture-of-Experts (MoE) models. In MoE architectures like Switch Transformer (Fedus et al. 2022) and GShard (Lepikhin et al. 2021), a gating network routes tokens to specialized expert sub-networks. Key parallels:

| MoE Concept | Cascade Router Equivalent |
|-------------|--------------------------|
| Gating network | Stage-2/3 scoring function |
| Expert capacity factor | Budget guardrail per model |
| Auxiliary load balance loss | Pareto frontier pruning |
| Expert choice routing (EC) | Inverse routing: model "claims" tasks it's best at |
| Top-k routing | CascadeModel with primary + fallback |

The Expert Choice (EC) routing innovation (Zhou et al. 2022) — where experts select their top-k tokens rather than tokens selecting experts — suggests an interesting inversion for Roko: instead of tasks being routed to models, models could "claim" tasks from a queue based on their learned specialization. This would naturally load-balance across models while allowing each model to self-select tasks where it excels.

---

## Cost-Spectrum Routing

Recent research on cost-aware routing (CSCR, 2025) demonstrates that the router should consider a continuous spectrum of cost-quality tradeoffs rather than discrete tiers. The Cost-Spectrum Contrastive Router achieves up to 25% improvement in accuracy-cost tradeoff by adaptively selecting cost bands.

### Continuous Cost-Quality Frontier

```rust
pub struct CostSpectrumRouter {
    /// Contrastive encoder mapping (task_context, model_descriptor) → similarity.
    encoder: ContrastiveEncoder,
    /// Per-model cost descriptors (lightweight feature vectors).
    model_descriptors: HashMap<String, ModelDescriptor>,
    /// Adaptive cost band for current system state.
    pub cost_band: CostBand,
    /// Cost band adaptation parameters.
    pub band_adaptation: BandAdaptation,
}

pub struct CostBand {
    /// Lower bound of acceptable cost per task (USD).
    pub min_cost: f64,
    /// Upper bound of acceptable cost per task (USD).
    pub max_cost: f64,
    /// Current operating point within the band.
    pub target_cost: f64,
}

pub struct BandAdaptation {
    /// Widen band when pass rate is high (can afford cheaper experiments).
    pub widen_threshold: f64,    // default: 0.85 pass rate
    /// Narrow band when budget pressure is high.
    pub narrow_threshold: f64,   // default: 0.80 budget utilization
    /// Band width change per adaptation step.
    pub step_size: f64,          // default: 0.05 (5% of current band width)
}

pub struct ModelDescriptor {
    /// Lightweight feature vector encoding model characteristics.
    /// [quality_score, cost_per_m_tokens, avg_latency_ms, context_window_size]
    pub features: [f64; 4],
    /// Provider identifier.
    pub provider: String,
    /// Whether this model supports extended thinking/reasoning.
    pub supports_reasoning: bool,
}
```

### Selection with Cost Bands

```
fn select_cost_spectrum(task_context, models, cost_band):
    // Filter models to cost band
    candidates = models.filter(|m| cost_band.min_cost <= m.cost <= cost_band.max_cost)

    // Score each candidate by contrastive similarity to task
    for candidate in candidates:
        score = encoder.similarity(task_context, candidate.descriptor)

    // Select cheapest model above quality threshold
    quality_threshold = 0.7  // minimum acceptable similarity
    qualified = candidates.filter(|c| c.score >= quality_threshold)
    return qualified.min_by(|c| c.cost)

    // Fallback: if no qualified model in band, expand band
    if qualified.is_empty():
        cost_band.max_cost *= 1.5
        return select_cost_spectrum(task_context, models, expanded_band)
```

### Adaptive Band Management

The cost band adapts to system performance:

```
After each routing decision:
    if recent_pass_rate(last 20) > widen_threshold:
        // System performing well → can try cheaper models
        cost_band.min_cost -= step_size × cost_band.target_cost
    if budget_utilization > narrow_threshold:
        // Budget pressure → restrict to cheaper models
        cost_band.max_cost -= step_size × cost_band.target_cost
    if recent_pass_rate(last 20) < 0.50:
        // System struggling → widen band to allow expensive models
        cost_band.max_cost += 2 × step_size × cost_band.target_cost
```

This creates a self-regulating cost control mechanism that complements the budget guardrails in [08-cost-normalization](08-cost-normalization.md) — the guardrails provide hard limits while cost-spectrum routing provides soft optimization within those limits.

---

## Relationship to Other Documents

- **[03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md)** — The LinUCB algorithm used in stage 3.
- **[08-cost-normalization](08-cost-normalization.md)** — Cost signals that feed into the cost penalty during confidence-stage routing.
- **[09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)** — Provider health filtering before candidate scoring.
- **[10-pareto-frontier-pruning](10-pareto-frontier-pruning.md)** — Pareto pruning that reduces the candidate set.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Loop 1 (Health→Routing) and Loop 6 (Cost→Routing) feed into cascade router decisions.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Hysteresis prevents oscillation between near-equal models.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — C-Factor provides routing bias based on system performance.

See also: [12-self-improvement-frameworks](12-self-improvement-frameworks.md) for the academic routing research (RouteLLM, FrugalGPT, AutoMix) that inspired the cascade design.
