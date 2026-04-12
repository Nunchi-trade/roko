# Adaptive Risk Management: Five-Layer Runtime Risk Control

> **Layer**: L3 Harness (runtime risk assessment), Cross-cut (Daimon motivation)
>
> **Crate**: Target: `roko-agent` (risk engine), `roko-daimon` (behavioral state modulation)
>
> **Synapse traits**: `Scorer` (rate risk), `Gate` (enforce risk limits), `Policy` (adapt guardrails)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [08-threat-model.md](08-threat-model.md)

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

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — Hard shields (Layer 1)
- [05-loop-detection.md](05-loop-detection.md) — Circuit breaker and loop defense
- [08-threat-model.md](08-threat-model.md) — Threat-to-layer mapping
- [10-mev-protection.md](10-mev-protection.md) — Layer 5 chain-domain threats
