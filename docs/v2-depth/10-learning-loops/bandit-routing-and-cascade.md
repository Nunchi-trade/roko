# Bandit Routing and Cascade

> Depth for [07-LEARNING.md](../../unified/07-LEARNING.md). The cascade router as a Loop of Route Cells using multi-armed bandit algorithms, the three-stage cascade from static to contextual routing, pattern discovery via trigram analysis, and cost-spectrum routing -- all expressed as Loop Graphs with predict-publish-correct feedback.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, demurrage), [02-CELL](../../unified/02-CELL.md) (Route protocol, Score protocol, Verify protocol, EFE routing), [04-EXECUTION](../../unified/04-EXECUTION.md) (Loop specialization, convergence), [07-LEARNING](../../unified/07-LEARNING.md) (L2 Strategy Routing, predict-publish-correct)

**Source docs**: `docs/05-learning/03-bandits-ucb-thompson-linucb.md`, `docs/05-learning/04-cascade-router.md`, `docs/05-learning/05-pattern-discovery-trigram.md`

---

## 1. The Cascade Router as a Route Cell Loop

The cascade router answers: "Given a task with these features, which model should run it?" This is a **Route protocol Cell** (see [02-CELL.md](../../unified/02-CELL.md)) operating inside an L2 Loop -- selecting among pre-approved alternatives at theta timescale (per-task, 750ms to 16s).

The Loop's feedback edge: each routing decision produces an episode with a gate verdict, which feeds back as a reward observation to the router, which updates its belief about model quality per context, which changes the next routing decision.

```toml
[graph]
name = "l2-cascade-routing-loop"
loop = true
min_interval = "750ms"

[[nodes]]
id = "context-builder"
cell = "roko:routing-context"
protocol = "Score"

[[nodes]]
id = "cascade-router"
cell = "roko:cascade-router"
protocol = "Route"

[[nodes]]
id = "agent-dispatch"
cell = "roko:agent-dispatcher"
protocol = "Connect"

[[nodes]]
id = "gate-pipeline"
cell = "roko:gate-pipeline"
protocol = "Verify"

[[nodes]]
id = "observation-recorder"
cell = "roko:routing-observer"
protocol = "React"

[[edges]]
from = "context-builder"
to = "cascade-router"

[[edges]]
from = "cascade-router"
to = "agent-dispatch"

[[edges]]
from = "agent-dispatch"
to = "gate-pipeline"

[[edges]]
from = "gate-pipeline"
to = "observation-recorder"

# Feedback edge: observation updates the router's belief
[[edges]]
from = "observation-recorder"
to = "cascade-router"
condition = "always"
```

---

## 2. Three-Stage Cascade

The router transitions through three stages of increasing sophistication as observation count grows. This is not three different routers -- it is one Route Cell with an internal state machine.

```
Stage 1: Static           < 50 observations
    Role -> model table. No learning. Safe defaults.

Stage 2: Confidence       50-200 observations
    Empirical pass rates + confidence intervals.
    Simple statistics, wide intervals that shrink with data.

Stage 3: UCB              > 200 observations
    Full LinUCB contextual bandit.
    18-dimensional context, learned feature weights.
```

### Why Three Stages

A single bandit algorithm works poorly at all scales:

- **Too few observations for UCB**: LinUCB with 18 context dimensions needs ~50 observations per arm. With 5 models, that is 250+ observations before the bandit is useful. Random exploration wastes money.
- **Too crude for static forever**: a hardcoded table cannot adapt to crate-specific patterns or post-update model changes.
- **Confidence bridges the gap**: between 50 and 200 observations, simple pass-rate statistics with confidence intervals route reasonably without the sample complexity of a linear model.

### Stage 1: Static Route Cell

```rust
/// Route Cell: static mapping from ModelTier to model slug.
/// Used during cold start (< 50 observations).
///
/// Deliberately conservative: over-routes to stronger models
/// to avoid gate failures while building observation base.
fn static_route(tier: ModelTier) -> ModelSpec {
    match tier {
        ModelTier::Fast    => ModelSpec::new("claude-haiku-4-5-20251001"),
        ModelTier::Standard => ModelSpec::new("claude-sonnet-4-20250514"),
        ModelTier::Complex => ModelSpec::new("claude-opus-4-20250514"),
    }
}
```

### Stage 2: Confidence Route Cell

For each candidate model, score = pass_rate - cost_penalty + affinity_bonus.

Additional biases from system state:
- **Low affect confidence** (< 0.3): bias toward stronger models.
- **High C-Factor** (> 0.8): bias toward cheaper models (system performing well, can save).
- **Low C-Factor** (< 0.4): bias toward stronger models (system struggling, invest in quality).

The C-Factor integration connects the cascade router to the collective intelligence Lens described in [c-factor-as-lens.md](c-factor-as-lens.md).

### Stage 3: LinUCB Route Cell

The full contextual bandit. For each arm `a` with context vector `x`:

```
score(a) = theta_a^T * x + alpha * sqrt(x^T * A_a^{-1} * x)
```

Where:
- `theta_a = A_a^{-1} * b_a` (ridge regression estimate)
- `alpha` decays from 1.0 to 0.05 over 200 observations: `alpha = 0.05 + 0.95 * exp(-observations / 60)`

### 18-Dimensional Context Vector

```rust
/// The RoutingContext encodes task features for the LinUCB Route Cell.
///
/// See [02-CELL.md](../../unified/02-CELL.md) for the Route protocol's
/// EFE routing: epistemic + pragmatic - cost.
pub struct RoutingContext {
    // dims 0-7:  task category one-hot (8 variants)
    // dim 8:     complexity band (0.0/0.5/1.0)
    // dim 9:     iteration normalized (iteration/10, capped 1.0)
    // dims 10-13: role hash (4-dim float)
    // dim 14:    crate familiarity (success_count/total_count)
    // dim 15:    has_prior_failure (0.0 or 1.0)
    // dim 16:    bias term (always 1.0)
    // dim 17:    cache affinity (1.0 if same model as prev task)
    pub features: [f64; 18],
}
```

Cache affinity (dim 17) encodes that consecutive tasks in a plan share context. Reusing the same model allows KV cache to serve prefix tokens at reduced cost. The `CACHE_AFFINITY_BONUS = 0.15` provides a static bonus during stage 2 before the LinUCB learns this from data.

---

## 3. Three Bandit Algorithms

The `roko-learn` crate provides three bandit implementations, each suited to a different decision structure:

### UCB1 (Context-Free)

For each arm `a`: `ucb(a) = mean_a + C * sqrt(ln(total_pulls) / pulls_a)`

- Exploration constant C = sqrt(2) by default.
- O(sqrt(T ln T)) cumulative regret.
- Used for: backend selection, retry strategy, prompt variant selection.

```rust
/// UCB1 bandit. Each arm is a choice (model, format, variant).
/// Select acquires read lock; update acquires write lock.
/// Concurrent selects never block each other.
pub struct UcbBandit {
    arms: RwLock<Vec<BanditArm>>,
    total_pulls: AtomicU64,
    exploration_c: f64,
}
```

### LinUCB (Contextual)

Models expected reward as a linear function of context vector. Generalizes across similar contexts. Used in cascade stage 3.

### Track-and-Stop (Best-Arm Identification)

Identifies the best arm with probability >= 1-delta, then stops exploring permanently. Used for tool format selection where the optimal choice is fixed per (model, role, tool_count, complexity) key.

```
Phase 1: Round-robin (pull each arm once)
Phase 2: D-tracking (allocation proportions from gap estimates)
Phase 3: Stopping (GLR statistic > threshold -> declare winner)
```

GLR stopping: `GLR(t) = t * KL(mu_hat_1, mu_hat_2)` where mu_hat_1, mu_hat_2 are empirical means of top-2 arms. Stop when `GLR(t) > ln((ln(t) + 1) / delta)`.

### BanditBank (Keyed Collections)

A collection of independent UCB1 instances, one per context key. Created lazily: new keys start with full exploration.

```
BanditBank {
    "implementer:rust:standard" -> UcbBandit { arms: [claude, codex, gemini] }
    "reviewer:rust:complex"     -> UcbBandit { arms: [claude, codex, gemini] }
}
```

---

## 4. Reward Scaling

All bandits assume rewards in [0, 1]. The predict-publish-correct pattern: the router predicts the best model (published as a Pulse on `prediction.router`), the gate verdict is the outcome (Pulse on `outcome.router`), and the reward update is the correction.

| Outcome | Reward |
|---|---|
| Gate pass (first attempt) | 1.0 |
| Gate pass (after retry) | 0.7 |
| Gate fail (recoverable) | 0.2 |
| Gate fail (unrecoverable) | 0.0 |
| Cost efficiency bonus | 1.0 - (cost / max_cost) |

---

## 5. Pattern Discovery via Trigram Analysis

Pattern discovery is a **Score protocol Cell** that mines recurring structural signals from the episode stream. It feeds the Episode-to-Heuristic Loop described in [episodes-and-playbooks.md](episodes-and-playbooks.md).

### Trigram Mining

Extract every three-action subsequence from each episode's gate verdict sequence:

```
Episode actions: ["read", "edit", "compile", "test", "lint"]

Trigrams:
  ("read", "edit", "compile")
  ("edit", "compile", "test")
  ("compile", "test", "lint")
```

Support count = number of distinct episodes containing the trigram (not total occurrences). This prevents inflation from long episodes.

### Why Trigrams

| N-gram | Properties |
|---|---|
| Unigrams | Too generic ("compile" appears everywhere) |
| Bigrams | Still generic ("edit->compile" is universal) |
| **Trigrams** | Captures meaningful patterns ("read->edit->test" vs "edit->compile->fix") |
| 4-grams | Too specific, insufficient support |

### Cross-Episode Consolidation via HDC Clustering

Beyond trigrams, k-medoids clustering over 10,240-bit HDC vectors groups structurally similar episodes:

```rust
/// Score Cell: cluster episodes by HDC similarity.
/// Identifies structural groupings invisible to individual trigram analysis.
///
/// Algorithm: Partitioning Around Medoids (PAM)
///   1. Greedy farthest-first seeding
///   2. Assign each point to nearest medoid (1 - HDC similarity)
///   3. Update medoids to minimize intra-cluster distance
///   4. Repeat until convergence or max_iterations
pub struct CrossEpisodeConsolidator {
    pub k: usize,              // default: 3
    pub max_iterations: usize, // default: 100
}
```

Clustering discovers groupings like: "42 episodes involving cross-crate config modifications, 62% pass rate, suggest playbook rule for checking serde derives."

### Operating Frequency

Pattern discovery runs every 20 episodes -- the slowest learning loop. This frequency separation prevents oscillation from noisy short-term data:

```
Cascade router:       every episode         (highest frequency)
Gate thresholds:      every 5 episodes
Pattern discovery:    every 20 episodes     (lowest frequency)
```

---

## 6. Pareto Frontier Pruning

Before presenting candidates to the bandit, the Route Cell computes a Pareto frontier over (pass_rate, cost_per_success). Dominated models are pruned.

```rust
/// A model is Pareto-optimal if no other model has both
/// higher pass_rate AND lower cost_per_success.
///
/// Recomputed every 50 observations.
pub fn compute_pareto_frontier(
    stats: &HashMap<String, ModelObservation>,
) -> Vec<String> {
    let mut frontier = Vec::new();
    for (slug_a, obs_a) in stats {
        let dominated = stats.iter().any(|(slug_b, obs_b)| {
            slug_b != slug_a
                && obs_b.pass_rate >= obs_a.pass_rate
                && obs_b.cost_per_success <= obs_a.cost_per_success
                && (obs_b.pass_rate > obs_a.pass_rate
                    || obs_b.cost_per_success < obs_a.cost_per_success)
        });
        if !dominated {
            frontier.push(slug_a.clone());
        }
    }
    frontier
}
```

This reduces exploration waste: the bandit does not spend trials on clearly inferior models. Typical steady state: 2-3 Pareto-optimal models representing genuine cost-quality tradeoffs.

---

## 7. CascadeModel Output

```rust
/// The Route Cell's output Signal.
pub struct CascadeModel {
    /// Primary model to dispatch.
    pub primary: ModelSpec,
    /// Pre-computed fallback for retry without re-routing.
    pub fallback: Option<ModelSpec>,
    /// Latency SLA in milliseconds.
    pub latency_sla_ms: u64,
    /// Which cascade stage produced this.
    pub stage: CascadeStage,
}
```

The fallback field provides immediate escalation on failure without re-querying the Route Cell. This is the Route protocol's "select among candidates" made concrete.

---

## 8. Cybernetic Loop: Route -> Execute -> Observe -> Update -> Route

The cascade router participates in multiple overlapping Loops:

```
L2 inner loop (per-task):
    Route Cell selects model
    -> Agent executes with selected model
    -> Gate pipeline produces verdict
    -> Observation recorder feeds reward to Route Cell
    -> Route Cell updates beliefs
    -> FEEDBACK: next selection is better informed

L1 inner loop (per-tick):
    Gate threshold EMA adjusts what counts as "pass"
    -> Affects reward signal to Route Cell
    -> FEEDBACK: routing and gating co-adapt

Cross-loop coupling to L3 (per-session):
    Dream consolidation reviews routing decisions
    -> Identifies persistent model-task mismatches
    -> FEEDBACK: structural routing adjustments
```

The L2 loop operates at the highest frequency in the learning system (every episode). This is where the system spends most of its learning budget.

---

## 9. Router Calibration

The Route Cell's predictions must be calibrated: when it estimates 80% pass probability, the model should succeed ~80% of the time.

### Expected Calibration Error (ECE)

```
ECE = sum_{b=1}^{B} (n_b / N) * |accuracy_b - confidence_b|
```

Where B = 10 bins. ECE = 0 is perfect calibration.

| ECE Range | Action |
|---|---|
| < 0.05 | No action |
| 0.05-0.10 | Monitor |
| 0.10-0.20 | Apply Platt scaling |
| > 0.20 | Investigate distribution shift |

Auto-recalibration runs every 100 routing decisions, fitting Platt scaling parameters `sigmoid(a * raw_score + b)` via gradient descent on recent (predicted_probability, actual_outcome) pairs.

---

## 10. Lookahead Routing

Current routing considers only the immediate task. Lookahead predicts upcoming tasks from the DAG and optimizes across the sequence for KV cache reuse.

```rust
/// Route Cell extension: lookahead across task sequences.
///
/// Chooses a model that minimizes total cost across a horizon
/// of upcoming tasks, accounting for KV cache reuse savings.
pub struct LookaheadRouter {
    inner: CascadeRouter,
    task_graph: TaskDag,
    pub horizon: usize,      // default: 3 tasks ahead
    pub gamma: f64,           // discount factor, default: 0.9
    cache_model: CacheReuseModel,
}
```

Savings when applicable: 15-30% for sequential tasks on same crate, 10-20% for same role across consecutive tasks.

---

## What This Enables

1. **Adaptive model routing**: the system learns which model works best for each task context, transitioning from safe defaults to learned preferences.
2. **Cost-quality optimization**: Pareto pruning eliminates dominated models; the bandit resolves genuine tradeoffs.
3. **Pattern-informed routing**: trigram patterns and HDC clusters identify task families that can inform routing context.
4. **Sequence-aware cost reduction**: lookahead routing exploits KV cache reuse across task sequences.
5. **Calibrated confidence**: auto-recalibration ensures routing scores map to actual success probabilities.

## Feedback Loops

- **L2 predict-publish-correct**: router predicts best model, gate verdict is outcome, reward update is correction. Operates every episode.
- **Stage transitions**: observation count triggers automatic upgrade from static -> confidence -> UCB. No manual intervention.
- **Pareto recomputation**: every 50 observations, the frontier is recalculated, potentially adding or removing models from consideration.
- **C-Factor coupling**: high c-factor biases toward cheaper models (system performing well), low c-factor biases toward stronger models. See [c-factor-as-lens.md](c-factor-as-lens.md).
- **Cost-routing feedback**: budget pressure forces cheaper model selection, which reduces cost, which relaxes pressure. Cybernetic loop 6 from the source docs.

## Open Questions

1. **LinUCB vs NeuralUCB transition**: at what observation count should the system consider transitioning to a neural contextual bandit? The source docs suggest 500+ observations with nonlinear residuals as the threshold. Is this practical for self-hosted development?
2. **Lookahead horizon**: 3 tasks ahead is the default. Should this adapt based on plan structure (longer horizon for plans with many sequential same-crate tasks)?
3. **Alpha decay curve**: the exploration parameter decays from 1.0 to 0.05 over 200 observations with tau=60. Is this too aggressive for deployments that see frequent provider updates (model quality shifts)?
4. **Cross-deployment routing transfer**: can LinUCB weights transfer across deployments, or are they deployment-specific? The context vector includes crate familiarity, which is inherently local.
5. **Relationship to autocatalytic-compounding.md**: the cascade router is Loop 4 (c-factor feedback) in the compounding graph. If the router learns suboptimally (e.g., trapped in a local minimum), does this break the autocatalytic cycle?
