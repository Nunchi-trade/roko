# Cascade Router

> **Crate:** `roko-learn` · **Module:** `cascade_router.rs`
> **Persistence:** `.roko/learn/cascade-router.json`
> **Wiring:** `LearningRuntime` → `CascadeRouter::select()` (called from orchestrate.rs)
> **Cross-references:** [03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md), [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md), [10-pareto-frontier-pruning](10-pareto-frontier-pruning.md), [08-cost-normalization](08-cost-normalization.md)

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

## Relationship to Other Documents

- **[03-bandits-ucb-thompson-linucb](03-bandits-ucb-thompson-linucb.md)** — The LinUCB algorithm used in stage 3.
- **[08-cost-normalization](08-cost-normalization.md)** — Cost signals that feed into the cost penalty during confidence-stage routing.
- **[09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)** — Provider health filtering before candidate scoring.
- **[10-pareto-frontier-pruning](10-pareto-frontier-pruning.md)** — Pareto pruning that reduces the candidate set.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Loop 1 (Health→Routing) and Loop 6 (Cost→Routing) feed into cascade router decisions.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Hysteresis prevents oscillation between near-equal models.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — C-Factor provides routing bias based on system performance.

See also: [12-self-improvement-frameworks](12-self-improvement-frameworks.md) for the academic routing research (RouteLLM, FrugalGPT, AutoMix) that inspired the cascade design.
