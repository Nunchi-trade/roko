# Adaptive Risk Management: Five-Layer Runtime Risk Control

> **Layer**: L3 Harness (runtime risk assessment), Cross-cut (Daimon motivation)
>
> **Crate**: Target: `roko-agent` (risk engine), `roko-daimon` (behavioral state modulation)
>
> **Synapse traits**: `Scorer` (rate risk), `Gate` (enforce risk limits), `Policy` (adapt guardrails)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [08-threat-model.md](08-threat-model.md)


> **Implementation**: Specified

---

## Overview

The adaptive risk system provides five layers of runtime risk control that self-evolve within hard bounds. Every layer operates as T0 (deterministic Rust, no LLM calls, zero inference cost per tick). The LLM proposes actions; the risk engine disposes.

The five layers, ordered by increasing cost:

| Layer | Name | What It Does | Enforcement Point |
|-------|------|-------------|-------------------|
| 1 | Hard Shields | Immutable constraints (PolicyCage for chain, SafetyLayer for general) | Pre-execution gate |
| 2 | Position Sizing | Kelly-criterion-based allocation with confidence modulation | Pre-execution adjustment |
| 3 | Adaptive Guardrails | Bayesian trust expansion/contraction | Pre-execution gate + post-turn update |
| 4 | Health Observation | Anomaly detection, health scoring | Post-turn monitoring |
| 5 | Domain Threat Detection | Domain-specific threats (MEV for chain, supply chain for code) | Pre- and post-execution |

---

## Layer 1: Hard Shields

Hard shields are immutable constraints that cannot be overridden by the agent, the risk engine, or even the operator without a configuration change and restart.

### General-Purpose Agents

For general-purpose agents, hard shields are implemented via the `SafetyLayer` (see [00-defense-in-depth.md](00-defense-in-depth.md)):

- **BashPolicy**: Deny patterns for dangerous commands (rm -rf, sudo, fork bombs)
- **GitPolicy**: Protected branches (main/master), force-push blocking
- **NetworkPolicy**: HTTPS-only, private network blocking, host allowlists
- **PathPolicy**: Worktree sandboxing, escape prevention
- **RateLimiter**: 60 calls/60s sliding window per (role, tool)

### Chain-Domain Agents

For chain-domain agents, hard shields include the PolicyCage smart contract:

- **Spending limits**: Per-transaction, per-session, per-day caps
- **Asset allowlists**: Only approved tokens and protocols
- **Slippage bounds**: Maximum price impact per trade (enforced on-chain)
- **Circuit breakers**: Automatic pause at configurable drawdown thresholds (13%/7%/3%)

The PolicyCage is an on-chain smart contract — even if the agent's entire runtime is compromised, the on-chain constraints hold.

---

## Layer 2: Position Sizing (Fractional Kelly)

### Theory

Kelly's criterion (Kelly, 1956) determines the growth-optimal allocation for a repeated game with known edge and volatility: `f* = edge / variance`. Full Kelly sizing assumes infinite repetition, known edge, and tolerance for 50%+ drawdowns — none of which hold for an autonomous agent.

Carta et al. (2020) demonstrated via Monte Carlo simulation that full Kelly requires thousands of trades before converging to the theoretical growth rate, and triple Kelly leads to certain ruin. Half-Kelly captures approximately 75% of optimal growth with dramatically reduced drawdown risk.

Busseti, Ryu, and Boyd (2016) extended this with risk-constrained Kelly gambling formulated as a convex optimization problem that guarantees drawdown probability stays below a specified level.

### Confidence-Modulated Scaling

The Kelly fraction scales with operational confidence:

```
kelly_fraction = base_kelly * confidence_multiplier(operational_confidence)

At confidence 0.0: ~10% of growth-optimal (maximum caution)
At confidence 0.9: ~50% of growth-optimal (half-Kelly)
```

The sigmoid confidence multiplier: `f(c) = 0.1 + 0.4 * sigmoid(10 * (c - 0.5))`

A new agent starts at confidence ~0.25 (weakly pessimistic prior). The sigmoid maps this to a multiplier around 0.11. After hundreds of successful operations, confidence rises toward 0.8-0.9, and the multiplier approaches 0.5. The agent earns the right to larger actions through demonstrated competence, not through time alone.

### For General-Purpose Agents

Position sizing for code agents translates to scope sizing:
- How many files can this agent modify in one task?
- How large a refactoring is it trusted to perform?
- How many concurrent worktrees can it manage?

The same Kelly-inspired framework applies: start conservative (single-file changes), expand with demonstrated competence (multi-file refactors, architectural changes).

---

## Layer 3: Bayesian Adaptive Guardrails

### Operational Confidence (Beta-Binomial Model)

The core question: how much should the system trust an agent's decisions? Fixed limits are wrong in both directions — too tight and the agent cannot operate, too loose and it can cause damage before earning trust.

Berkenkamp et al. (2017) formalized safe exploration using Gaussian process models with Lyapunov stability guarantees. Their algorithm safely collects data and gradually expands the safe region. Roko adapts this principle.

```rust
/// Tracks operational confidence across multiple competence dimensions
/// using Beta-Binomial models with asymmetric learning rates.
pub struct OperationalConfidenceTracker {
    pub dimensions: HashMap<String, BetaDistribution>,
}

pub struct BetaDistribution {
    pub alpha: f64,  // Success count + prior
    pub beta: f64,   // Failure count + prior
}
```

**Weakly pessimistic priors**: `alpha=1, beta=3` starts at mean 0.25. The composite confidence uses the geometric mean of lower 95% credible intervals across all dimensions, ensuring a single poorly-calibrated dimension drags everything down.

**Asymmetric learning**: Failures count 1.5x. In portfolio management and software engineering alike, a single catastrophic failure can wipe out dozens of successes. The 1.5x multiplier is conservative relative to the actual loss asymmetry but aggressive enough to demote underperforming strategies within 10-15 failures.

### Guardrail Evolution

Guardrails evolve with confidence, tighten during failure streaks, and contract during high-risk periods:

```
effective_limit = hard_shield_limit × base_multiplier × context_multiplier

base_multiplier = 0.2 + 0.8 × confidence   // [20%, 100%] of hard shield
context_multiplier = f(failure_rate, task_complexity, domain_risk)
```

At low confidence (new agent): effective limits are 20% of hard shields.
At high confidence (proven agent): effective limits approach 100% of hard shields.

---

## Layer 4: Health Observation

### Anomaly Detection

The observation layer monitors agent behavior for anomalies:

- **Ghost turn detection**: Turns where the agent produces no meaningful output (empty responses, repeated failures)
- **Efficiency degradation**: Declining tokens-per-successful-outcome ratio
- **Gate pass rate tracking**: EMA of gate pass rates per rung
- **Context attribution**: Whether the agent is using provided context effectively

### Health Score

A composite health score feeds into the circuit breaker:

```
health_score = w1 × gate_pass_rate + w2 × efficiency_trend + w3 × (1 - ghost_turn_rate) + w4 × context_attribution_rate
```

Default weights: gate_pass_rate 0.4, efficiency_trend 0.2, ghost_turn_rate 0.2, context_attribution 0.2.

When health score drops below threshold (default 0.3), the conductor's circuit breaker opens.

---

## Layer 5: Domain Threat Detection

### Code Domain

- **Dangerous import detection**: Checks for imports of known-dangerous modules (os.system, subprocess, eval)
- **Secret detection in output**: ScrubPolicy catches credentials in generated code
- **Dependency confusion**: Checks proposed dependency additions against known-good registries

### Chain Domain

- **MEV detection**: Pattern matching for sandwich attacks, front-running, JIT liquidity (see [10-mev-protection.md](10-mev-protection.md))
- **Oracle manipulation detection**: Multi-oracle divergence, TWAP deviation
- **Cross-protocol contagion**: CFI and ASRI metrics (see [08-threat-model.md](08-threat-model.md))

---

## OperationalConfidenceTracker: full implementation

```rust
use std::collections::HashMap;

/// Tracks operational confidence across multiple competence dimensions
/// using Beta-Binomial models with asymmetric learning rates.
pub struct OperationalConfidenceTracker {
    /// Per-dimension Beta distributions.
    pub dimensions: HashMap<String, BetaDistribution>,
    /// Failure multiplier for asymmetric learning.
    /// Default: 1.5 (failures count 1.5x).
    pub failure_weight: f64,
}

/// Beta distribution parameters for a single dimension.
pub struct BetaDistribution {
    /// Success pseudo-count (alpha). Starts at 1.0 (weakly pessimistic).
    pub alpha: f64,
    /// Failure pseudo-count (beta). Starts at 3.0 (weakly pessimistic).
    pub beta: f64,
}

impl BetaDistribution {
    /// Weakly pessimistic prior: mean = 1/(1+3) = 0.25.
    pub fn pessimistic_prior() -> Self {
        Self { alpha: 1.0, beta: 3.0 }
    }

    /// Mean of the Beta distribution: alpha / (alpha + beta).
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Variance: alpha*beta / ((alpha+beta)^2 * (alpha+beta+1)).
    pub fn variance(&self) -> f64 {
        let sum = self.alpha + self.beta;
        (self.alpha * self.beta) / (sum * sum * (sum + 1.0))
    }

    /// Lower bound of 95% credible interval.
    /// Uses the normal approximation: mean - 1.96 * sqrt(variance).
    /// For small alpha+beta, this underestimates -- conservative, which is correct.
    pub fn lower_95(&self) -> f64 {
        (self.mean() - 1.96 * self.variance().sqrt()).max(0.0)
    }

    /// Record a success: increment alpha by 1.
    pub fn record_success(&mut self) {
        self.alpha += 1.0;
    }

    /// Record a failure: increment beta by failure_weight.
    pub fn record_failure(&mut self, weight: f64) {
        self.beta += weight;
    }
}

impl OperationalConfidenceTracker {
    pub fn new() -> Self {
        Self {
            dimensions: HashMap::new(),
            failure_weight: 1.5,
        }
    }

    /// Register a competence dimension with a pessimistic prior.
    /// Standard dimensions: "gate_pass", "tool_success", "cost_efficiency",
    /// "context_utilization", "task_completion".
    pub fn register_dimension(&mut self, name: &str) {
        self.dimensions
            .entry(name.to_string())
            .or_insert_with(BetaDistribution::pessimistic_prior);
    }

    /// Record a success in a dimension.
    pub fn record_success(&mut self, dimension: &str) {
        if let Some(dist) = self.dimensions.get_mut(dimension) {
            dist.record_success();
        }
    }

    /// Record a failure in a dimension.
    /// Failures are weighted by self.failure_weight (default 1.5x).
    pub fn record_failure(&mut self, dimension: &str) {
        let w = self.failure_weight;
        if let Some(dist) = self.dimensions.get_mut(dimension) {
            dist.record_failure(w);
        }
    }

    /// Composite confidence: geometric mean of lower 95% credible intervals.
    /// The geometric mean ensures a single poorly-calibrated dimension
    /// drags everything down.
    pub fn composite_confidence(&self) -> f64 {
        if self.dimensions.is_empty() {
            return 0.0;
        }
        let product: f64 = self
            .dimensions
            .values()
            .map(|d| d.lower_95().max(0.001)) // floor to avoid zero-product
            .product();
        product.powf(1.0 / self.dimensions.len() as f64)
    }
}
```

### confidence_multiplier() and effective_limit()

```rust
/// Sigmoid confidence multiplier.
/// Maps confidence [0, 1] to Kelly fraction multiplier [0.1, 0.5].
///
/// At confidence 0.0: returns ~0.1 (10% of growth-optimal).
/// At confidence 0.5: returns ~0.3 (inflection point).
/// At confidence 1.0: returns ~0.5 (half-Kelly).
pub fn confidence_multiplier(confidence: f64) -> f64 {
    let sigmoid = 1.0 / (1.0 + (-10.0 * (confidence - 0.5)).exp());
    0.1 + 0.4 * sigmoid
}

/// Compute effective limit for a given action.
///
/// effective_limit = hard_shield_limit * base_multiplier * context_multiplier
///
/// base_multiplier = 0.2 + 0.8 * confidence  (range: [0.2, 1.0])
/// context_multiplier = f(failure_rate, task_complexity, domain_risk)
pub fn effective_limit(
    hard_shield_limit: f64,
    confidence: f64,
    failure_rate: f64,
    task_complexity: f64,
    domain_risk: f64,
) -> f64 {
    let base_multiplier = 0.2 + 0.8 * confidence;

    // Context multiplier: penalize recent failures, complex tasks, risky domains.
    // Each factor in [0, 1]; product gives the combined discount.
    let failure_factor = 1.0 - (failure_rate * 0.5).min(0.8);   // max 80% reduction
    let complexity_factor = 1.0 - (task_complexity * 0.3).min(0.6); // max 60% reduction
    let risk_factor = 1.0 - (domain_risk * 0.4).min(0.7);       // max 70% reduction
    let context_multiplier = failure_factor * complexity_factor * risk_factor;

    hard_shield_limit * base_multiplier * context_multiplier
}
```

### Risk-constrained Kelly optimization (Busseti et al. 2016)

The standard Kelly criterion maximizes log-growth: `f* = edge / variance`. Busseti, Ryu, and Boyd reformulate this as a convex optimization problem with an explicit drawdown constraint:

```
maximize    E[log(1 + f * X)]
subject to  P(drawdown > d) <= epsilon
            0 <= f <= f_max
```

Where `X` is the random return, `d` is the maximum acceptable drawdown, and `epsilon` is the probability bound on exceeding that drawdown.

Algorithm (pseudocode):

```
risk_constrained_kelly(edge, variance, max_drawdown, epsilon):
    # Step 1: Compute unconstrained Kelly fraction.
    f_kelly = edge / variance

    # Step 2: Compute drawdown-constrained upper bound.
    # From Busseti et al., the bound on ruin probability
    # for fractional Kelly is approximately:
    #   P(drawdown > d) ≈ exp(-2 * d * (1 - f/f_kelly) / (f^2 * variance))
    # Solving for f given P = epsilon:
    #   f_dd = solve: exp(-2 * d * (1 - f/f_kelly) / (f^2 * variance)) = epsilon

    # Step 3: Iterative bisection (convex, so bisection converges).
    f_low = 0.0
    f_high = f_kelly
    for _ in 0..50:
        f_mid = (f_low + f_high) / 2
        ruin_prob = exp(-2 * max_drawdown * (1 - f_mid / f_kelly) / (f_mid^2 * variance))
        if ruin_prob > epsilon:
            f_high = f_mid   # Too aggressive
        else:
            f_low = f_mid    # Can afford more

    # Step 4: Apply confidence modulation.
    f_constrained = f_low
    return f_constrained * confidence_multiplier(operational_confidence)
```

**Configuration parameters:**

```toml
[agent.risk.kelly]
base_kelly_fraction = 0.5     # Starting Kelly fraction before modulation. Range: 0.1..1.0.
max_drawdown = 0.13           # Maximum acceptable drawdown. Range: 0.01..0.5.
drawdown_epsilon = 0.05       # Probability bound on exceeding drawdown. Range: 0.001..0.2.
confidence_floor = 0.1        # Minimum confidence multiplier. Range: 0.05..0.3.
confidence_ceiling = 0.5      # Maximum confidence multiplier (half-Kelly). Range: 0.3..1.0.
```

For code-domain agents, Kelly sizing translates to scope limits:

| Confidence range | Scope allowed | Files per task | Refactor depth |
|-----------------|---------------|----------------|----------------|
| 0.0 - 0.3 | Single-file, single-function | 1 | Leaf functions only |
| 0.3 - 0.6 | Single-file, multi-function | 1-3 | Module-level |
| 0.6 - 0.8 | Multi-file, single crate | 3-10 | Cross-module |
| 0.8 - 1.0 | Multi-crate refactoring | 10+ | Architectural |

### Confidence evolution state machine

```
  +------------+     success    +------------+     success    +-----------+
  |  Cautious  | ------------> | Developing | ------------> |  Capable  |
  | conf<0.3   |               | 0.3<=c<0.7 |               | c>=0.7    |
  +-----+------+               +-----+------+               +-----+-----+
        ^                            |                             |
        |     failure streak         |  failure streak             |
        |     (>=5 consecutive)      |  (>=3 consecutive)         |
        +----------------------------+-----------------------------+
                                     |
                                     v
                              +------+------+
                              |  Recovery   |
                              | temp tighten|
                              +------+------+
                                     |
                                     | 10 turns without failure
                                     v
                              (return to previous state)
```

| From | To | Trigger |
|------|----|---------|
| Cautious | Developing | composite_confidence crosses 0.3 upward |
| Developing | Capable | composite_confidence crosses 0.7 upward |
| Capable | Recovery | 3+ consecutive failures in any dimension |
| Developing | Recovery | 5+ consecutive failures in any dimension |
| Recovery | previous state | 10 consecutive turns without failure |
| Any | Cautious | composite_confidence drops below 0.3 |

---

## Integration with Daimon

The Daimon (motivation/affect engine) modulates risk tolerance based on the agent's behavioral state:

| Daimon State | Risk Tolerance Modifier | Rationale |
|-------------|------------------------|-----------|
| Engaged | 1.0× (normal) | Agent performing well, standard limits |
| Struggling | 0.6× (tightened) | Reduce scope to help agent recover |
| Coasting | 0.8× (slightly tightened) | Agent may be cutting corners |
| Exploring | 1.2× (loosened) | Exploration needs room for failure |
| Focused | 1.0× (normal) | Concentrated work, standard limits |
| Resting | 0.3× (heavily tightened) | Minimal operations during rest |

The PAD (Pleasure-Arousal-Dominance) vector from the Daimon modulates the effective confidence used for guardrail evolution:

```
effective_confidence = base_confidence × daimon_modifier(pad_vector)
```

### Daimon state to risk_tolerance_modifier mapping

```rust
/// Compute risk tolerance modifier from Daimon behavioral state.
/// Returns a multiplier applied to effective_confidence.
pub fn risk_tolerance_modifier(state: &DaimonState) -> f64 {
    match state {
        DaimonState::Engaged => 1.0,     // Normal operation.
        DaimonState::Struggling => 0.6,  // Tighten limits to help recovery.
        DaimonState::Coasting => 0.8,    // Slight tightening; may be cutting corners.
        DaimonState::Exploring => 1.2,   // Exploration needs room for failure.
        DaimonState::Focused => 1.0,     // Concentrated work, standard limits.
        DaimonState::Resting => 0.3,     // Minimal operations during rest.
    }
}

/// Full effective confidence computation including Daimon modulation.
pub fn daimon_adjusted_confidence(
    tracker: &OperationalConfidenceTracker,
    daimon_state: &DaimonState,
) -> f64 {
    let base = tracker.composite_confidence();
    let modifier = risk_tolerance_modifier(daimon_state);
    (base * modifier).clamp(0.0, 1.0)
}
```

### roko.toml kelly_fraction configuration

```toml
[agent.risk]
# Base Kelly fraction before confidence modulation.
# Half-Kelly (0.5) captures ~75% of optimal growth with much lower drawdown.
# Range: 0.1..1.0. Default: 0.5.
kelly_fraction = 0.5

# Hard shield limits (immutable per-domain caps).
max_files_per_task = 20          # Code domain. Range: 1..100.
max_concurrent_worktrees = 3     # Code domain. Range: 1..10.
max_transaction_value_eth = 1.0  # Chain domain. Range: 0.001..100.0.
max_daily_spend_usd = 100.0     # Cost domain. Range: 1.0..10000.0.

# Adaptive guardrail parameters.
confidence_floor = 0.1           # Minimum multiplier (never below 10% of hard shield).
failure_weight = 1.5             # Failures count 1.5x in Beta update. Range: 1.0..3.0.
recovery_window_turns = 10       # Turns without failure to exit Recovery state.
```

### Integration wiring path

```
orchestrate.rs: PlanRunner::run_task()
  |
  +--> OperationalConfidenceTracker::composite_confidence()
  |      reads from: .roko/learn/gate-thresholds.json (EMA per rung)
  |
  +--> daimon_adjusted_confidence()
  |      reads from: Daimon PAD vector (roko-daimon)
  |
  +--> confidence_multiplier() --> kelly_fraction
  |
  +--> effective_limit()
  |      applies: hard_shield * base_mult * context_mult
  |
  +--> SafetyLayer::check_pre_execution()
         enforces: effective limits per action
```

### Test criteria

- `BetaDistribution::pessimistic_prior()` starts at mean 0.25 (alpha=1, beta=3)
- `record_failure()` increments beta by `failure_weight`, not by 1.0
- `composite_confidence()` uses geometric mean: one dimension at 0.01 drags composite below 0.1
- `confidence_multiplier(0.0)` returns approximately 0.1; `confidence_multiplier(1.0)` returns approximately 0.5
- `effective_limit()` at confidence 0.0 returns 20% of hard shield
- `effective_limit()` at confidence 1.0 with zero failure/complexity/risk returns 100% of hard shield
- `risk_tolerance_modifier` for Resting state (0.3) combined with low confidence (0.2) produces a heavily constrained limit
- Kelly fraction stays within `[confidence_floor, confidence_ceiling]` for all inputs
- Recovery state exits after `recovery_window_turns` consecutive successes

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Kelly (1956) | Growth-optimal allocation criterion |
| Carta et al. (2020) | Monte Carlo analysis of Kelly variants |
| Busseti, Ryu, Boyd (2016) | Risk-constrained Kelly gambling |
| MacLean, Ziemba, Blazenko (1992) | Growth versus security in Kelly betting |
| Berkenkamp et al. (2017) | Safe exploration with Gaussian process models |
| Milionis, Moallemi, Roughgarden, Zhang (2022) | LVR — Loss-Versus-Rebalancing for LP sizing |
| Loesch et al. (2021) | Empirical LP losses across 17 Uniswap v3 pools |
| NASA UAS Risk Assessment | Risk budgets for autonomous aircraft — safety margins per maneuver |
| FAA AI Safety Assurance Roadmap (2025) | Phased AI integration with safety budget tiers |
| CSA Securing Agentic Control Plane (2026) | Kill switches and blast-radius limits as first-class primitives |
| Agent-SafetyBench (arXiv:2412.14470, 2024) | 349 environments, 2000 tests — no agent scored >60% safety compliance |

---

## Safety Budgets: Risk Allocation Framework

A safety budget is a hard limit on how much uncertainty, autonomy, and irreversibility an agent is permitted to consume before requiring human review or triggering a halt. Inspired by NASA UAS risk budgets and FAA AI Safety Assurance Roadmap (2025).

### Budget Dimensions

Define five orthogonal budget dimensions:

```rust
/// Safety budget: hard limits on agent risk consumption per session.
/// Each dimension tracks a different axis of risk.
///
/// Inspired by NASA UAS risk budgets and CSA agentic safety
/// recommendations (2026).
pub struct SafetyBudget {
    /// Irreversibility budget: sum of irreversibility scores.
    /// Read operations score 0.0; permanent deletions score 1.0.
    /// Range for limit: 0.1..100.0. Default: 10.0.
    pub irreversibility_limit: f64,

    /// Blast radius budget: maximum files modified per session.
    /// Prevents a single agent run from touching too much code.
    /// Range: 1..1000. Default: 50.
    pub blast_radius_file_limit: usize,

    /// Footprint budget: maximum external interactions.
    /// Counts tool calls, API requests, process spawns.
    /// Range: 10..10000. Default: 500.
    pub footprint_limit: usize,

    /// Uncertainty budget: tokens for low-confidence decisions.
    /// Each decision with confidence < uncertainty_threshold consumes one token.
    /// When exhausted, agent must escalate to human.
    /// Range: 1..100. Default: 10.
    pub uncertainty_tokens: usize,

    /// Cost budget: maximum spend (USD) on inference + tools.
    /// Range: 0.01..10000.0. Default: 50.0.
    pub cost_limit_usd: f64,
}

/// Tracks budget consumption during a session.
pub struct SafetyBudgetTracker {
    pub budget: SafetyBudget,
    pub usage: SafetyBudgetUsage,
    /// Threshold below which a decision is "uncertain".
    pub uncertainty_threshold: f64,
}

pub struct SafetyBudgetUsage {
    pub irreversibility_consumed: f64,
    pub files_touched: HashSet<String>,
    pub footprint_count: usize,
    pub uncertainty_tokens_used: usize,
    pub cost_consumed_usd: f64,
}

impl SafetyBudgetTracker {
    /// Check whether a proposed action is within budget.
    /// Returns the limiting dimension if budget would be exceeded.
    pub fn check(&self, action: &ProposedAction) -> BudgetCheckResult {
        let usage = &self.usage;
        let budget = &self.budget;

        if usage.irreversibility_consumed + action.irreversibility_score
            > budget.irreversibility_limit
        {
            return BudgetCheckResult::Exceeded(BudgetDimension::Irreversibility);
        }

        let new_file_count = {
            let mut files = usage.files_touched.clone();
            files.extend(action.files_modified.iter().cloned());
            files.len()
        };
        if new_file_count > budget.blast_radius_file_limit {
            return BudgetCheckResult::Exceeded(BudgetDimension::BlastRadius);
        }

        if usage.footprint_count + action.tool_calls > budget.footprint_limit {
            return BudgetCheckResult::Exceeded(BudgetDimension::Footprint);
        }

        if action.confidence < self.uncertainty_threshold
            && usage.uncertainty_tokens_used >= budget.uncertainty_tokens
        {
            return BudgetCheckResult::Exceeded(BudgetDimension::Uncertainty);
        }

        if usage.cost_consumed_usd + action.estimated_cost > budget.cost_limit_usd {
            return BudgetCheckResult::Exceeded(BudgetDimension::Cost);
        }

        BudgetCheckResult::WithinBudget
    }

    /// Record an action's consumption against the budget.
    pub fn consume(&mut self, action: &CompletedAction) {
        self.usage.irreversibility_consumed += action.irreversibility_score;
        self.usage.files_touched.extend(action.files_modified.iter().cloned());
        self.usage.footprint_count += action.tool_calls;
        if action.confidence < self.uncertainty_threshold {
            self.usage.uncertainty_tokens_used += 1;
        }
        self.usage.cost_consumed_usd += action.actual_cost;
    }
}

pub enum BudgetCheckResult {
    WithinBudget,
    Exceeded(BudgetDimension),
}

pub enum BudgetDimension {
    Irreversibility,
    BlastRadius,
    Footprint,
    Uncertainty,
    Cost,
}
```

### Irreversibility Scoring

Actions are scored on a 0.0-1.0 irreversibility scale:

| Action | Score | Rationale |
|--------|-------|-----------|
| read_file, glob, grep | 0.0 | Pure observation, no side effects |
| write_file (new file) | 0.2 | Reversible: delete the file |
| edit_file | 0.3 | Reversible via git: but requires manual intervention |
| bash (read-only: ls, cat) | 0.0 | No side effects |
| bash (build: cargo build) | 0.1 | Artifacts can be cleaned |
| bash (destructive: rm) | 0.8 | Hard to reverse without backup |
| git commit | 0.3 | Reversible via revert |
| git push | 0.6 | Affects shared state, requires force-push to undo |
| Network API call (GET) | 0.0 | Read-only |
| Network API call (POST) | 0.5 | May have irreversible side effects |

```rust
/// Compute irreversibility score for a tool call.
pub fn irreversibility_score(tool: &str, args: &serde_json::Value) -> f64 {
    match tool {
        "read_file" | "glob" | "grep" => 0.0,
        "write_file" => 0.2,
        "edit_file" => 0.3,
        "bash" => score_bash_irreversibility(args),
        "git_commit" => 0.3,
        "git_push" => 0.6,
        "web_fetch" => 0.0,
        "web_search" => 0.0,
        _ => 0.5, // Unknown tools get moderate score
    }
}

fn score_bash_irreversibility(args: &serde_json::Value) -> f64 {
    let command = args.get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if command.starts_with("ls") || command.starts_with("cat")
        || command.starts_with("echo") || command.starts_with("grep") {
        0.0
    } else if command.contains("rm ") || command.contains("rmdir") {
        0.8
    } else if command.starts_with("cargo build") || command.starts_with("cargo test") {
        0.1
    } else if command.starts_with("cargo publish") {
        0.9
    } else {
        0.4 // Unknown bash commands get moderate score
    }
}
```

### Budget Tiers

Different task types receive different budgets:

| Task Type | Irreversibility | Files | Footprint | Uncertainty | Cost |
|-----------|----------------|-------|-----------|-------------|------|
| Read-only audit | 0.0 | 0 | 100 | 5 | $5 |
| Single-file fix | 3.0 | 3 | 200 | 5 | $10 |
| Multi-file implementation | 15.0 | 20 | 500 | 10 | $50 |
| Architectural refactor | 50.0 | 100 | 2000 | 20 | $200 |
| Full plan execution | 100.0 | 500 | 5000 | 50 | $500 |

### Integration with Adaptive Risk

Safety budgets compose with the existing five-layer adaptive risk system:

```
effective_budget = base_budget * confidence_multiplier * daimon_modifier

Where:
  base_budget = task-type default from the tier table
  confidence_multiplier = from OperationalConfidenceTracker (Layer 3)
  daimon_modifier = from Daimon behavioral state (0.3 for Resting, 1.2 for Exploring)
```

A new agent (low confidence, 0.2 multiplier) working on a multi-file task gets:
- Irreversibility: 15.0 x 0.2 = 3.0 (equivalent to a single-file fix)
- Files: 20 x 0.2 = 4
- Footprint: 500 x 0.2 = 100

### Budget Exhaustion Response

When any budget dimension is exhausted:

```rust
/// Response to budget exhaustion.
pub fn on_budget_exhausted(
    dimension: BudgetDimension,
    usage: &SafetyBudgetUsage,
    budget: &SafetyBudget,
) -> CognitiveSignal {
    match dimension {
        BudgetDimension::Irreversibility => CognitiveSignal::Pause,
        BudgetDimension::BlastRadius => CognitiveSignal::Pause,
        BudgetDimension::Footprint => CognitiveSignal::Cooldown,
        BudgetDimension::Uncertainty => CognitiveSignal::Escalate,
        BudgetDimension::Cost => CognitiveSignal::Shutdown,
    }
}
```

### Configuration

```toml
[agent.risk.budget]
# Enable safety budgets. Default: true.
enabled = true
# Default task type (determines base budget tier).
default_task_type = "multi_file"   # "read_only" | "single_file" | "multi_file" | "refactor" | "plan_execution"
# Uncertainty threshold: decisions below this confidence consume uncertainty tokens.
uncertainty_threshold = 0.5    # Range: 0.1..0.9.
# Whether budget scales with confidence. Default: true.
confidence_scaling = true

[agent.risk.budget.overrides]
# Per-dimension overrides (override the tier defaults).
# irreversibility_limit = 20.0
# blast_radius_file_limit = 30
# footprint_limit = 1000
# uncertainty_tokens = 15
# cost_limit_usd = 100.0
```

### Test Criteria

- SafetyBudgetTracker::check() returns WithinBudget for a read-only action with full budget
- SafetyBudgetTracker::check() returns Exceeded(Irreversibility) when irreversibility_consumed + action > limit
- SafetyBudgetTracker::check() returns Exceeded(Uncertainty) when tokens exhausted and confidence < threshold
- SafetyBudgetTracker::consume() correctly increments all usage counters
- irreversibility_score("read_file", _) returns 0.0
- irreversibility_score("bash", rm_command) returns 0.8
- Budget tier "read_only" has 0.0 irreversibility limit (no writes allowed)
- confidence_scaling correctly reduces budget for low-confidence agents
- on_budget_exhausted returns Escalate for Uncertainty (to involve human)
- Budget composition: effective_budget = base x confidence x daimon correctly computes for all states

---

## Hierarchical Budget Delegation

When a parent agent (the orchestrator) delegates work to child agents (task executors), safety budgets must be subdivided with formal conservation guarantees. Without conservation enforcement, child budgets can silently exceed the parent's allocation -- a subtle failure mode where each child looks safe in isolation but the aggregate blows through plan-level limits.

This design draws on Ye and Tan's Agent Contracts framework (arXiv:2601.08815, January 2025; COINE 2026), which demonstrated 90% token reduction and zero conservation violations across multi-agent hierarchies.

### Conservation law

For a parent agent P with budget B_P delegating to child agents C_1, ..., C_n:

```
For every resource dimension d:
    SUM_i(budget(C_i, d)) <= budget(P, d)

Equivalently:
    budget(P, d) - SUM_i(budget(C_i, d)) >= 0    (surplus is held in reserve)
```

This is enforced at delegation time. If a subdivision request would violate conservation, it is rejected before any child begins execution.

### BudgetDelegator

```rust
/// Hierarchical budget delegation with conservation enforcement.
/// Ensures child budgets never exceed parent budget in any dimension.
pub struct BudgetDelegator {
    /// The parent's total budget.
    parent_budget: SafetyBudget,
    /// Already-delegated amounts per child.
    delegations: HashMap<String, SafetyBudget>,
    /// Remaining budget (parent - sum of delegations).
    remaining: SafetyBudget,
}

impl BudgetDelegator {
    pub fn new(parent_budget: SafetyBudget) -> Self {
        Self {
            remaining: parent_budget.clone(),
            parent_budget,
            delegations: HashMap::new(),
        }
    }

    /// Delegate a budget to a child agent.
    /// Returns Err if delegation would violate conservation in any dimension.
    pub fn delegate(
        &mut self,
        child_id: &str,
        requested: SafetyBudget,
    ) -> Result<SafetyBudget, BudgetDelegationError> {
        // Check conservation for each dimension
        if requested.irreversibility_limit > self.remaining.irreversibility_limit {
            return Err(BudgetDelegationError::ExceedsRemaining {
                dimension: BudgetDimension::Irreversibility,
                requested: requested.irreversibility_limit,
                remaining: self.remaining.irreversibility_limit,
            });
        }
        if requested.blast_radius_file_limit > self.remaining.blast_radius_file_limit {
            return Err(BudgetDelegationError::ExceedsRemaining {
                dimension: BudgetDimension::BlastRadius,
                requested: requested.blast_radius_file_limit as f64,
                remaining: self.remaining.blast_radius_file_limit as f64,
            });
        }
        if requested.footprint_limit > self.remaining.footprint_limit {
            return Err(BudgetDelegationError::ExceedsRemaining {
                dimension: BudgetDimension::Footprint,
                requested: requested.footprint_limit as f64,
                remaining: self.remaining.footprint_limit as f64,
            });
        }
        if requested.uncertainty_tokens > self.remaining.uncertainty_tokens {
            return Err(BudgetDelegationError::ExceedsRemaining {
                dimension: BudgetDimension::Uncertainty,
                requested: requested.uncertainty_tokens as f64,
                remaining: self.remaining.uncertainty_tokens as f64,
            });
        }
        if requested.cost_limit_usd > self.remaining.cost_limit_usd {
            return Err(BudgetDelegationError::ExceedsRemaining {
                dimension: BudgetDimension::Cost,
                requested: requested.cost_limit_usd,
                remaining: self.remaining.cost_limit_usd,
            });
        }

        // Deduct from remaining
        self.remaining.irreversibility_limit -= requested.irreversibility_limit;
        self.remaining.blast_radius_file_limit -= requested.blast_radius_file_limit;
        self.remaining.footprint_limit -= requested.footprint_limit;
        self.remaining.uncertainty_tokens -= requested.uncertainty_tokens;
        self.remaining.cost_limit_usd -= requested.cost_limit_usd;

        self.delegations.insert(child_id.to_string(), requested.clone());
        Ok(requested)
    }

    /// Reclaim budget from a completed child agent.
    /// Unused budget returns to the parent's remaining pool.
    pub fn reclaim(
        &mut self,
        child_id: &str,
        actual_usage: &SafetyBudgetUsage,
    ) -> Result<(), BudgetDelegationError> {
        let delegated = self.delegations.remove(child_id)
            .ok_or(BudgetDelegationError::UnknownChild)?;

        // Return unused portion to remaining
        let unused_irreversibility = delegated.irreversibility_limit
            - actual_usage.irreversibility_consumed;
        let unused_files = delegated.blast_radius_file_limit
            - actual_usage.files_touched.len();
        let unused_footprint = delegated.footprint_limit
            - actual_usage.footprint_count;
        let unused_uncertainty = delegated.uncertainty_tokens
            - actual_usage.uncertainty_tokens_used;
        let unused_cost = delegated.cost_limit_usd
            - actual_usage.cost_consumed_usd;

        self.remaining.irreversibility_limit += unused_irreversibility.max(0.0);
        self.remaining.blast_radius_file_limit += unused_files;
        self.remaining.footprint_limit += unused_footprint;
        self.remaining.uncertainty_tokens += unused_uncertainty;
        self.remaining.cost_limit_usd += unused_cost.max(0.0);

        Ok(())
    }

    /// Verify the conservation invariant: parent - delegated - remaining = zero.
    /// Non-zero indicates a bug. Checked as a debug assertion.
    pub fn conservation_check(&self) -> bool {
        let total_delegated_irrev: f64 = self.delegations.values()
            .map(|b| b.irreversibility_limit)
            .sum();
        let expected = self.parent_budget.irreversibility_limit
            - total_delegated_irrev
            - self.remaining.irreversibility_limit;
        expected.abs() < 1e-10 // floating point tolerance
    }
}

pub enum BudgetDelegationError {
    ExceedsRemaining {
        dimension: BudgetDimension,
        requested: f64,
        remaining: f64,
    },
    UnknownChild,
    ConservationViolation,
}
```

### Automatic budget allocation strategies

Three strategies for dividing a parent budget among child tasks:

| Strategy | Rule | When to use |
|----------|------|-------------|
| Equal | Each task gets `parent / n` | Uniform tasks of similar scope |
| Proportional | Budget scales with estimated complexity | Heterogeneous task sizes |
| Risk-weighted | Higher cascade risk receives a smaller share | Plan contains high-risk tasks that should be tightly constrained |

```rust
/// Strategy for automatically allocating budget to child tasks.
pub enum BudgetAllocationStrategy {
    /// Equal shares: each task gets parent_budget / num_tasks.
    Equal,
    /// Proportional to estimated task complexity.
    ProportionalToComplexity,
    /// Risk-weighted: high-cascade-risk tasks get smaller budgets.
    RiskWeighted,
}

impl BudgetAllocationStrategy {
    pub fn allocate(
        &self,
        parent_budget: &SafetyBudget,
        tasks: &[TaskInfo],
        cascade_risks: &HashMap<TaskId, f64>,
    ) -> Vec<(TaskId, SafetyBudget)> {
        match self {
            Self::Equal => {
                let n = tasks.len();
                let share = parent_budget.divide_equally(n);
                tasks.iter().map(|t| (t.id.clone(), share.clone())).collect()
            }
            Self::ProportionalToComplexity => {
                let total_complexity: f64 = tasks.iter()
                    .map(|t| t.estimated_complexity)
                    .sum();
                tasks.iter().map(|t| {
                    let fraction = t.estimated_complexity / total_complexity;
                    (t.id.clone(), parent_budget.scale(fraction))
                }).collect()
            }
            Self::RiskWeighted => {
                // Higher cascade risk -> smaller budget (inversely proportional).
                let total_inv_risk: f64 = tasks.iter()
                    .map(|t| 1.0 / (cascade_risks.get(&t.id).unwrap_or(&0.1) + 0.01))
                    .sum();
                tasks.iter().map(|t| {
                    let risk = cascade_risks.get(&t.id).unwrap_or(&0.1);
                    let inv_risk = 1.0 / (risk + 0.01);
                    let fraction = inv_risk / total_inv_risk;
                    (t.id.clone(), parent_budget.scale(fraction))
                }).collect()
            }
        }
    }
}

impl SafetyBudget {
    /// Divide budget equally among n recipients.
    pub fn divide_equally(&self, n: usize) -> SafetyBudget {
        SafetyBudget {
            irreversibility_limit: self.irreversibility_limit / n as f64,
            blast_radius_file_limit: self.blast_radius_file_limit / n,
            footprint_limit: self.footprint_limit / n,
            uncertainty_tokens: self.uncertainty_tokens / n,
            cost_limit_usd: self.cost_limit_usd / n as f64,
        }
    }

    /// Scale budget by a fraction (0.0..1.0).
    pub fn scale(&self, fraction: f64) -> SafetyBudget {
        SafetyBudget {
            irreversibility_limit: self.irreversibility_limit * fraction,
            blast_radius_file_limit: (self.blast_radius_file_limit as f64 * fraction).ceil() as usize,
            footprint_limit: (self.footprint_limit as f64 * fraction).ceil() as usize,
            uncertainty_tokens: (self.uncertainty_tokens as f64 * fraction).ceil() as usize,
            cost_limit_usd: self.cost_limit_usd * fraction,
        }
    }
}
```

### Integration wiring

```
orchestrate.rs: PlanRunner::run_plan()
  |
  +--> BudgetDelegator::new(plan_budget)
  |
  +--> For each task in topological order:
  |      |
  |      +--> BudgetAllocationStrategy::allocate()
  |      |      compute task budget from remaining plan budget
  |      |
  |      +--> BudgetDelegator::delegate(task_id, task_budget)
  |      |      conservation check, deduct from remaining
  |      |
  |      +--> SafetyBudgetTracker::new(task_budget)
  |      |      per-task budget enforcement during execution
  |      |
  |      +--> [execute task with budget tracker]
  |      |
  |      +--> BudgetDelegator::reclaim(task_id, actual_usage)
  |             return unused budget to pool
  |
  +--> BudgetDelegator::conservation_check()
         verify: parent = delegated + remaining + reclaimed
```

The key property: tasks that finish under-budget release their surplus back to the pool. Late-stage tasks that need more headroom can draw from what earlier tasks left behind, without ever exceeding the plan-level budget.

### Configuration

```toml
[agent.risk.budget.delegation]
# Budget allocation strategy. Options: "equal", "proportional", "risk_weighted".
allocation_strategy = "risk_weighted"
# Reserve fraction: hold this fraction of parent budget as buffer.
# Range: 0.0..0.5. Default: 0.1.
reserve_fraction = 0.1
# Enable automatic budget reclamation from completed tasks.
auto_reclaim = true
# Enable conservation assertion checks (recommended for debug builds).
conservation_assertions = true
```

### Test criteria

- `BudgetDelegator::delegate()` rejects requests exceeding remaining budget
- `BudgetDelegator::reclaim()` correctly returns unused budget to remaining pool
- `BudgetDelegator::conservation_check()` returns true after any sequence of delegate/reclaim operations
- Equal allocation: N tasks each get 1/N of parent budget
- Proportional allocation: task with 2x complexity gets 2x budget
- Risk-weighted allocation: task with 2x cascade risk gets a smaller budget
- Reserve fraction: allocation respects `reserve_fraction` (only allocates `1 - reserve_fraction`)
- Zero conservation violations across 100 random delegation/reclamation sequences

---

## Cross-References

- [00-defense-in-depth.md](00-defense-in-depth.md) — Hard shields (Layer 1)
- [05-loop-detection.md](05-loop-detection.md) — Circuit breaker and loop defense
- [08-threat-model.md](08-threat-model.md) — Threat-to-layer mapping
- [10-mev-protection.md](10-mev-protection.md) — Layer 5 chain-domain threats
