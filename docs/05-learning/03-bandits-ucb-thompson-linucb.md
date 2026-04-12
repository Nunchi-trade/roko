# Bandits: UCB1, Thompson Sampling, LinUCB

> **Crate:** `roko-learn` · **Modules:** `bandits.rs`, `model_router.rs`
> **Persistence:** `.roko/learn/cascade-router.json` (LinUCB state), per-bandit JSON files
> **Academic basis:** Auer, Cesa-Bianchi & Fischer 2002 (UCB1); Li et al. 2010 (LinUCB); Garivier & Kaufmann 2016 (Track-and-Stop); Thompson 1933
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [11-thompson-sampling-drift](11-thompson-sampling-drift.md), [10-pareto-frontier-pruning](10-pareto-frontier-pruning.md)

---

## Purpose

Roko uses multi-armed bandit algorithms for every repeated decision in the system: which model to route a task to, which prompt section to include, which tool format to use, which backend to prefer. Bandits provide a principled framework for balancing exploration (trying less-tested options) against exploitation (using the best-known option), with formal regret bounds that guarantee convergence to optimal choices.

The `roko-learn` crate provides three bandit implementations, each suited to a different decision structure:

| Bandit | Algorithm | Use Case | Key Property |
|--------|-----------|----------|--------------|
| `UcbBandit` | UCB1 (Auer et al. 2002) | Context-free repeated decisions | O(√(T ln T)) cumulative regret |
| `LinUCBRouter` | LinUCB (Li et al. 2010) | Context-dependent model routing | Handles 18-dim context vectors |
| `TrackAndStopBandit` | Track-and-Stop (Garivier & Kaufmann 2016) | Best-arm identification | Stops when confident, not after fixed trials |
| `BanditBank` | Collection of UCB1 instances | Keyed decision spaces | One bandit per context key |

---

## UCB1: Upper Confidence Bound

The `UcbBandit` implements the classic UCB1 algorithm for context-free multi-armed bandits.

### UCB1 Formula

For each arm `a` with `pulls_a` observations:

```
ucb(a) = mean_a + C · √(ln(total_pulls) / pulls_a)
```

where:
- `mean_a` = cumulative reward / pulls_a
- `C` = exploration constant (default: √2)
- `total_pulls` = sum of all arm pulls

Arms with `pulls_a == 0` receive infinite UCB and are always chosen before any pulled arm. Tiebreaking is deterministic: first by insertion order.

### Reward Scaling

UCB1 regret bounds assume rewards in `[0, 1]`. Callers must normalize:

| Outcome | Reward |
|---------|--------|
| Gate pass | 1.0 |
| Gate fail | 0.0 |
| Mixed (partial success) | `1.0 − (cost / max_cost)` |

### Schema

```rust
pub struct BanditArm {
    /// Human-readable name (e.g. "claude", "codex").
    pub name: String,
    /// Number of times this arm has been pulled.
    pub pulls: u64,
    /// Cumulative reward received across all pulls.
    pub total_reward: f64,
}

pub struct UcbBandit {
    arms: RwLock<Vec<BanditArm>>,
    total_pulls: AtomicU64,
    /// UCB exploration constant (default: √2).
    exploration_c: f64,
    /// Persistence path (optional).
    persist_path: Option<PathBuf>,
}
```

### Thread Safety

`UcbBandit` uses `parking_lot::RwLock` for arm stats and `AtomicU64` for the pull counter. `select()` acquires only a shared read lock while `update()` acquires an exclusive write lock. This means concurrent `select()` calls never block each other — only an in-progress `update()` causes contention.

### Use Cases

- **Backend selection**: which LLM provider to route a request to.
- **Retry strategy**: immediate retry vs. escalate model vs. re-plan.
- **Context-size buckets**: how much context to include in the prompt.
- **Prompt experiment variant selection**: which variant of a prompt section to use.

---

## BanditBank: Keyed Collections

The `BanditBank` manages a collection of independent `UcbBandit` instances keyed by context string. This is used when the same decision must be made in multiple distinct contexts, each with its own reward distribution.

```
BanditBank {
    "implementer:rust:standard" → UcbBandit { arms: [claude, codex, gemini] }
    "reviewer:rust:complex"     → UcbBandit { arms: [claude, codex, gemini] }
    "planner:python:fast"       → UcbBandit { arms: [claude, codex, gemini] }
}
```

Bandits are created lazily: when a `select(key, ...)` call arrives for a key that doesn't exist, a new `UcbBandit` is initialized with all available arms and zero observations. This ensures that new context keys start with full exploration before converging.

### Persistence

The entire bank is serialized to a single JSON file. Each bandit's arm stats are included, so the system resumes with full history on restart.

---

## LinUCB: Contextual Bandit Router

The `LinUCBRouter` implements the LinUCB algorithm (Li et al. 2010) for context-dependent model selection. Unlike UCB1, which treats each arm independently, LinUCB models the expected reward as a linear function of a context vector, allowing the router to generalize across similar contexts.

### LinUCB Formula

For each arm `a` with context vector `x`:

```
score(a) = θ_a^T · x + α · √(x^T · A_a^{-1} · x)
```

where:
- `θ_a = A_a^{-1} · b_a` (ridge regression estimate)
- `A_a` = d×d matrix (initialized to identity)
- `b_a` = d×1 vector (initialized to zero)
- `α` = exploration parameter (decays from 1.0 to 0.05)

### Context Vector (18 dimensions)

The `RoutingContext` encodes task features into a fixed-length vector:

| Dimension(s) | Feature | Encoding |
|--------------|---------|----------|
| 0-7 | Task category | One-hot (8 `TaskCategory` variants) |
| 8 | Complexity band | Scalar: 0.0 (Fast) / 0.5 (Standard) / 1.0 (Complex) |
| 9 | Iteration | Normalized: `iteration / 10`, capped at 1.0 |
| 10-13 | Agent role | 4-dim float vector (hashed from role string) |
| 14 | Crate familiarity | `success_count / total_count`, clamped to [0.0, 1.0] |
| 15 | Has prior failure | Binary: 0.0 or 1.0 |
| 16 | Bias term | Always 1.0 |
| 17 | Cache affinity | 1.0 when candidate matches previous model, else 0.0 |

Total dimension: `CONTEXT_DIM = 18`.

### Alpha Decay

The exploration parameter `α` decays exponentially from 1.0 to 0.05 over 200 observations:

```
α = 0.05 + 0.95 · exp(−observations / 60)
```

At cold start (0 observations), `α = 1.0` — maximum exploration. After 200 observations, `α ≈ 0.084` — mostly exploitation with minimal exploration. The decay constant `τ = 60` was chosen so that `exp(−200/60) ≈ 0.036`, giving effective convergence by 200 observations.

### Cold Start

When observation count is below `COLD_START_THRESHOLD = 50`, the router falls back to a static mapping from `ModelTier` to a default model slug. This prevents the LinUCB from making poorly-informed decisions with insufficient data.

### Cache Affinity

The context vector includes a cache affinity dimension (dimension 17) that is 1.0 when the candidate model matches the model used for the previous task in the same plan. This encodes the observation that consecutive tasks in a plan often share similar context, and reusing the same model allows the provider's KV cache to serve prefix tokens at reduced cost.

The `CACHE_AFFINITY_BONUS = 0.15` in the cascade router provides an additional static bonus for cache-consistent routing during the confidence stage, before the LinUCB has learned the relationship from data.

---

## Track-and-Stop: Best-Arm Identification

The `TrackAndStopBandit` implements the Track-and-Stop algorithm (Garivier & Kaufmann 2016) for best-arm identification with anytime-valid stopping. Unlike UCB1 which minimizes cumulative regret, Track-and-Stop minimizes the number of samples needed to identify the best arm with probability ≥ 1 − δ.

### Algorithm

```
Phase 1: Round-robin
    Pull each arm at least once.

Phase 2: D-tracking
    Compute target allocation proportions from gap estimates.
    Pull the arm most under-sampled relative to its target.
    Forced exploration: no arm falls below √t − K/2 pulls.

Phase 3: Stopping
    When GLR statistic > β(t, δ), declare winner.
    Stop exploring permanently for this key.
```

### GLR Stopping Criterion

The Generalized Likelihood Ratio statistic is:

```
GLR(t) = t · KL(μ̂_1, μ̂_2)
```

where `μ̂_1` and `μ̂_2` are the empirical means of the top-2 arms. When `GLR(t) > β(t, δ)` where `β(t, δ) = ln((ln(t) + 1) / δ)`, the best arm is declared with confidence ≥ 1 − δ.

### Use Case: Tool Format Selection

The `TrackAndStopBandit` implements the `FormatBandit` trait for adaptive tool-format selection. For each `(model, role, tool_count, complexity)` key, the bandit identifies the best tool format (JSON, XML, native function calling) with high confidence, then stops exploring permanently for that key.

```rust
pub trait FormatBandit: Send + Sync {
    fn select_format(&self, key: &BanditKey) -> ToolFormat;
    fn update_format(&self, key: &BanditKey, format: ToolFormat, outcome: &ToolOutcome);
}
```

### Why Track-and-Stop Instead of UCB1?

UCB1 never stops exploring — it always allocates some trials to suboptimal arms. For decisions where:
1. The optimal choice is fixed (the best tool format for a given model doesn't change over time).
2. Exploration has a cost (suboptimal tool formats waste tokens and cause parse errors).
3. We need high confidence in the answer, not just low regret.

Track-and-Stop is the right algorithm: it explores only as much as needed, then commits permanently.

---

## Reward Scaling Across Bandits

All three bandit implementations assume rewards in `[0, 1]`:

| Signal | Reward Value |
|--------|-------------|
| Gate pass (first attempt) | 1.0 |
| Gate pass (after retry) | 0.7 |
| Gate fail (recoverable) | 0.2 |
| Gate fail (unrecoverable) | 0.0 |
| Cost efficiency | `1.0 − (cost / max_cost)` |

For the cascade router, rewards are typically binary (1.0 for gate pass, 0.0 for fail) with a cost adjustment that penalizes expensive successes. See [04-cascade-router](04-cascade-router.md) for the full reward computation.

Track-and-Stop also assumes sub-Gaussian rewards with parameter σ = 0.5. The GLR stopping criterion uses this assumption for threshold calibration.

---

## Persistence

| Component | Format | Path |
|-----------|--------|------|
| `UcbBandit` | JSON (arm stats) | Per-bandit file |
| `BanditBank` | JSON (all bandits) | Single file |
| `LinUCBRouter` | JSON (A matrices, b vectors, obs count) | `.roko/learn/cascade-router.json` |
| `TrackAndStopBandit` | JSON (per-key state) | Per-instance file |

All persistence uses the atomic tempfile+rename pattern for crash safety.

---

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — The cascade router uses LinUCB as its stage-3 routing algorithm.
- **[10-pareto-frontier-pruning](10-pareto-frontier-pruning.md)** — Pareto pruning restricts the arm set presented to the bandit.
- **[11-thompson-sampling-drift](11-thompson-sampling-drift.md)** — Thompson Sampling with discount factor is an alternative to UCB1 for non-stationary environments.
- **[08-cost-normalization](08-cost-normalization.md)** — Cost normalization affects the reward signal fed to bandits.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Hysteresis prevents bandits from oscillating between near-equal arms.
