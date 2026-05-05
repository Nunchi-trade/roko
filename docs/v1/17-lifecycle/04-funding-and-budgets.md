# Funding and Budgets

> **Layer**: L1 Framework (capabilities, model routing) + L3 Harness (monitoring, interventions)
>
> **Prerequisites**: `docs/17-lifecycle/03-configuration-and-operator-model.md` (operator controls), `docs/00-architecture/INDEX.md` (5-layer taxonomy)
>
> **Synapse traits**: Composer (budget-aware context assembly under token/cost constraints), Router (tier selection driven by budget availability), Policy (budget monitoring and intervention emission)


> **Implementation**: Specified

---

## Overview

Every agent consumes resources: inference tokens, compute time, tool invocations, and (for chain-domain agents) on-chain gas. Roko's budget system provides multi-level guardrails that prevent runaway costs while preserving agent autonomy. Budget exhaustion triggers graceful degradation — not agent death.

This document specifies the budget model, cost tracking mechanisms, multi-level guardrails, and the graceful degradation cascade that activates when budgets are constrained.

---

## Budget Model

### Resource Types

| Resource | Unit | Tracked by | Typical cost |
|----------|------|-----------|-------------|
| **Inference tokens** | Tokens consumed per LLM call | `roko-agent` dispatcher | Varies by model ($0.25-$15/M input tokens) |
| **Compute time** | Wall-clock seconds of VM runtime | Managed infrastructure | $0.025-$0.20/hr by tier |
| **Tool invocations** | Per-call tool usage | `roko-std` tool dispatcher | Usually free (local); x402-gated (remote) |
| **On-chain gas** | Gas units × gas price | `roko-chain` wallet manager | Varies by network ($0.001-$5.00/tx) |
| **Mesh operations** | Per-query and per-sync operations | `roko-mesh` relay client | x402-gated for public Mesh |

### Budget Configuration

```toml
[budget]
# Per-day inference spending limit (USD equivalent)
max_daily_inference_usd = 10.0

# Total lifetime budget (optional hard cap)
# max_total_usd = 1000.0

# Per-turn token limit (prevents single runaway turn)
max_tokens_per_turn = 8192

# Per-hour compute budget (hosted only)
# max_hourly_compute_usd = 0.20

# Warning thresholds (fraction of daily budget consumed)
warning_at = 0.7               # Warn at 70% of daily budget
critical_at = 0.9              # Critical alert at 90%

# Degradation mode when budget is constrained
degradation = "cascade"        # "cascade" | "pause" | "notify-only"
```

---

## Cost Tracking

Cost tracking happens at three levels:

### Per-Turn Tracking

Every LLM call records:

```rust
/// Per-turn cost record. Written to `.roko/learn/efficiency.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCostRecord {
    /// Turn identifier.
    pub turn_id: String,
    /// Model used for this turn.
    pub model: String,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Cache read tokens (if applicable).
    pub cache_read_tokens: u64,
    /// Estimated cost in USD.
    pub estimated_cost_usd: f64,
    /// Cognitive speed tier: Gamma, Theta, or Delta.
    pub cognitive_tier: CognitiveTier,
    /// Whether this turn was suppressed by T0 probes (zero LLM cost).
    pub t0_suppressed: bool,
    /// Timestamp.
    pub timestamp: u64,
}
```

### Per-Day Aggregation

Daily cost summaries are computed from per-turn records:

```rust
/// Daily cost aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyCostSummary {
    /// Date (YYYY-MM-DD).
    pub date: String,
    /// Total inference cost.
    pub inference_cost_usd: f64,
    /// Total compute cost (hosted only).
    pub compute_cost_usd: f64,
    /// Total gas cost (chain domain only).
    pub gas_cost_usd: f64,
    /// Total turns executed.
    pub total_turns: u64,
    /// Turns suppressed by T0 probes (zero LLM cost).
    pub t0_suppressed_turns: u64,
    /// T0 suppression rate (fraction of turns that avoided LLM calls).
    pub t0_suppression_rate: f64,
    /// Cost per turn (mean).
    pub cost_per_turn_usd: f64,
    /// Model distribution: fraction of turns per model.
    pub model_distribution: HashMap<String, f64>,
}
```

### Lifetime Tracking

Cumulative cost tracking across the agent's entire lifetime:

```rust
/// Lifetime cost tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifetimeCosts {
    /// Total inference cost since creation.
    pub total_inference_usd: f64,
    /// Total compute cost since creation.
    pub total_compute_usd: f64,
    /// Total gas cost since creation.
    pub total_gas_usd: f64,
    /// Total all-in cost.
    pub total_cost_usd: f64,
    /// Days active.
    pub days_active: u32,
    /// Average daily cost.
    pub average_daily_cost_usd: f64,
    /// Projected monthly cost at current rate.
    pub projected_monthly_cost_usd: f64,
}
```

---

## Multi-Level Guardrails

Budget enforcement operates at four levels, from most granular to most aggregate:

### Level 1: Per-Turn Token Limit

The `max_tokens_per_turn` setting (default: 8192) prevents any single LLM call from consuming excessive tokens. This is enforced at the `roko-agent` dispatcher level before the call is made.

If a turn would exceed the token limit, the Composer (L2 Scaffold) compresses the context to fit within budget. The VCG Attention Auction mechanism (see `docs/02-scaffold/`) allocates context budget across competing information sources via truthful bidding, ensuring the most valuable context is included.

### Level 2: Per-Hour Inference Rate

A sliding-window rate limiter prevents burst spending. Default: no more than 20% of daily budget consumed in any single hour. This prevents a misbehaving cognitive loop from exhausting the entire daily budget in minutes.

### Level 3: Daily Budget Ceiling

The `max_daily_inference_usd` setting (default: $10.00) is a hard daily cap. When reached, the degradation cascade activates (see below).

### Level 4: Lifetime Budget Cap

The optional `max_total_usd` setting (no default) provides an absolute spending limit. When reached, the agent enters permanent pause mode until the operator increases the cap or deletes the agent.

---

## Graceful Degradation Cascade

When budget constraints are hit, the agent does not die. It degrades gracefully through a cascade of cost-reduction measures:

### Stage 1: Model Downgrade (at 70% daily budget)

The Router switches to cheaper models:

1. Delta (consolidation) calls downgrade from `claude-opus-4-6` to `claude-sonnet-4-6`
2. Theta (reflective) calls downgrade from `claude-sonnet-4-6` to `claude-haiku-4-5`
3. Gamma (reactive) calls remain on `claude-haiku-4-5` (already cheapest)

**Cost reduction**: ~60-80% per inference call.

### Stage 2: T0 Probe Emphasis (at 80% daily budget)

The T0 probe system (16 zero-LLM probes, see `docs/02-scaffold/`) is activated at maximum sensitivity. T0 probes can handle ~80% of routine decisions without any LLM call:

- Cache probe: answers from cached Neuro entries
- Pattern probe: matches against known Engram patterns
- Threshold probe: evaluates numeric conditions without inference
- Template probe: fills templated responses without generation

**Cost reduction**: ~80% of turns suppressed (zero inference cost).

### Stage 3: Reduced Tick Frequency (at 90% daily budget)

The adaptive clock (L0 Runtime) increases tick intervals:

- Gamma interval: 15s → 60s
- Theta interval: 75s → 300s
- Delta interval: 6h → 24h

The agent operates at 25% of normal frequency. It still processes events, but less often.

**Cost reduction**: ~75% fewer inference calls per hour.

### Stage 4: Monitoring Only (at 95% daily budget)

The agent switches to monitoring-only mode:

- No actions taken (no tool invocations, no Mesh writes)
- Continues observing and logging
- Neuro continues receiving new Engrams (from monitoring, not inference)
- Daimon transitions to Resting state

**Cost reduction**: ~95% (only minimal inference for critical alerts).

### Stage 5: Budget Pause (at 100% daily budget)

The cognitive loop pauses entirely. The agent process remains alive but idle:

- Health server continues responding
- Mesh connection maintained (for incoming messages)
- No inference calls
- Resumes automatically when the daily budget window resets (midnight UTC)

The operator is notified at each stage transition. At no stage is the agent deleted — budget exhaustion is a resource constraint, not a lifecycle event.

---

## Funding Sources (Chain Domain)

For chain-domain agents, four funding sources are available:

### 1. Direct USDC Transfer

The simplest path. The operator sends USDC directly to the agent's wallet (in Delegation mode, this is the operator's own wallet with a delegation grant).

### 2. x402 Micropayments

The x402 protocol (EIP-3009 signed USDC transfers) enables pay-per-use compute and inference. Each x402 payment extends the agent's compute budget by the corresponding amount. The Coinbase/Linux Foundation x402 protocol provides the payment rail.

### 3. Metabolic Self-Funding Loop

An agent that earns revenue (e.g., from trading, LP management, or providing services) can fund its own continued operation. The self-funding loop:

```
Agent earns revenue (trading profits, LP fees, service fees)
  |
  v
Revenue deposited to agent wallet
  |
  v
Agent allocates portion to compute/inference budget
  |
  v
Agent continues operating
```

**Funding formula**: `F = (daily_cost × duration) × safety_margin`

Where:
- `daily_cost` = inference + compute + gas per day
- `duration` = desired runway in days
- `safety_margin` = 1.5x (default)

An agent burning $0.40/day (context-engineered, LLM-last, high T0 suppression) can sustain itself indefinitely on modest revenue. An agent burning $85/day (naive, every-turn Opus calls) requires substantial revenue. The mortality research on metabolic efficiency (Jonas 1966, "needful freedom") applies here — an agent's economic sustainability depends on its metabolic efficiency, which is determined by how well it uses T0 probes, model routing, and context compression.

### 4. Permissionless Extensions

On the Korai chain, anyone can extend any agent's compute budget by sending a KORAI payment. No authentication required. Payer type is tracked for attribution (`owner`, `self`, `external_user`). This enables community-funded agents — agents whose continued existence is supported by the value they provide to others.

---

## Cost Efficiency Metrics

The efficiency tracking system (wired into `.roko/learn/efficiency.jsonl`) captures per-turn efficiency data that the learning subsystem uses to improve cost performance over time:

| Metric | What it measures | Target |
|--------|-----------------|--------|
| **T0 suppression rate** | Fraction of turns handled without LLM | >80% |
| **Cost per gate pass** | Average cost of a turn that passes verification | <$0.01 |
| **Model distribution** | Fraction of calls per model tier | >60% Haiku, <5% Opus |
| **Token efficiency** | Useful output tokens / total tokens | >0.4 |
| **Cache hit rate** | Fraction of tokens served from cache | >0.3 |

These metrics feed into the CascadeRouter (see `roko-learn` crate), which adjusts model selection based on historical cost/quality tradeoffs. Over time, the agent learns which decisions require expensive models and which can be handled cheaply — a form of metabolic optimization that the mortality research attributed to death pressure but which is actually driven by budget constraints and learning.

---

## KORAI Token Demurrage (Chain Domain)

For chain-domain agents operating on the Korai chain, the KORAI token has a planned 1% annual demurrage rate. This means held KORAI tokens would lose 1% of their value per year, implemented as:

```
balance_effective = balance_raw × (1 - 0.01)^(years_since_last_update)
```

The demurrage mirrors Engram half-life at the token level — just as knowledge decays without reinforcement (Ebbinghaus 1885), tokens decay without use. This creates an incentive to circulate KORAI rather than hoard it, following Gesell's Freigeld principle (Gesell 1916) applied to agent economies.

See `docs/17-lifecycle/11-knowledge-demurrage.md` for the full demurrage specification.

---

## Gesell Demurrage Rate Calibration

The 1% annual KORAI demurrage rate is a design choice informed by historical implementations and economic theory. This section specifies the calibration framework that determines the optimal rate.

### Historical Calibration Data

| Currency | Period | Annual Rate | Observed Velocity Multiplier | Outcome |
|---|---|---|---|---|
| Wörgl stamp scrip | 1932-33 | 12% | ~14× vs Austrian schilling | Unemployment fell; Austrian central bank halted it |
| US Great Depression scrip | 1932-33 | ~52% (1%/week) | >50× exchanges per note | Extremely high velocity; impractical for planning |
| Chiemgauer (Bavaria) | 2003-present | 6% (reduced from 8%) | 3-5× vs Euro | Stable regional currency; 55%+ adoption among German regional currencies |
| WIR Bank (Switzerland) | 1934-present | 0% (abandoned 1952) | 2-3× | High velocity sustained via trust network even without demurrage |
| Freicoin (crypto) | 2012-present | 4.9% | Limited data | Proof of concept; low adoption |
| Circles UBI (Gnosis) | 2020-present | 7% (continuous) | Limited data | Active pilot; personal token issuance |

### Calibration Framework

The optimal demurrage rate balances three constraints:

```rust
/// Demurrage rate calibration framework.
/// Determines the optimal annual rate based on economic objectives.
///
/// Crate: `roko-core`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemurrageCalibration {
    /// Target velocity increase vs. zero-demurrage baseline.
    /// Typical: 3-5× (Chiemgauer empirical range).
    /// Range: 1.0-50.0.
    pub target_velocity_multiplier: f64,

    /// Hoarding sensitivity coefficient.
    /// Empirically derived: 1% rate increase → ~0.3-0.5% velocity increase.
    /// Chiemgauer data: ~0.4. Wörgl data: ~0.3.
    /// Range: 0.1-1.0.
    pub hoarding_sensitivity: f64,

    /// Minimum rate for psychological visibility.
    /// Rates below ~2%/yr are invisible to participants.
    /// Range: 0.01-0.05.
    pub minimum_effective_rate: f64,

    /// Maximum rate before transactional friction dominates.
    /// Rates above ~15%/yr create urgency that undermines planning.
    /// Range: 0.10-0.20.
    pub maximum_practical_rate: f64,

    /// Grace period before demurrage begins (Chiemgauer model: 90 days).
    /// Allows new participants to accumulate before decay starts.
    /// Range: 0-365 days.
    pub grace_period_days: u32,

    /// Floor: minimum fraction of face value at any time.
    /// Prevents asymptotic decay to zero from creating dust tokens.
    /// Default: 0.01 (1%). Range: 0.001-0.10.
    pub floor_fraction: f64,
}

impl DemurrageCalibration {
    /// Compute recommended annual demurrage rate.
    ///
    /// rate = clamp(
    ///   ln(target_velocity) * hoarding_sensitivity,
    ///   minimum_effective_rate,
    ///   maximum_practical_rate
    /// )
    ///
    /// For KORAI defaults (target 3×, sensitivity 0.4):
    ///   ln(3.0) * 0.4 = 1.099 * 0.4 = 0.044 = 4.4%/yr
    ///
    /// The current 1% rate is conservative — deliberately below the
    /// Chiemgauer-derived optimum to reduce friction during adoption.
    pub fn recommended_rate(&self) -> f64 {
        let raw_rate = self.target_velocity_multiplier.ln()
            * self.hoarding_sensitivity;
        raw_rate.clamp(self.minimum_effective_rate, self.maximum_practical_rate)
    }

    /// Compute effective balance after elapsed time.
    pub fn effective_balance(&self, face_value: f64, elapsed_years: f64) -> f64 {
        let grace_years = self.grace_period_days as f64 / 365.25;
        if elapsed_years <= grace_years {
            return face_value;
        }
        let taxable_years = elapsed_years - grace_years;
        let rate = self.recommended_rate();
        let decayed = face_value * (-rate * taxable_years).exp();
        decayed.max(face_value * self.floor_fraction)
    }
}

impl Default for DemurrageCalibration {
    fn default() -> Self {
        Self {
            target_velocity_multiplier: 3.0,
            hoarding_sensitivity: 0.4,
            minimum_effective_rate: 0.02,
            maximum_practical_rate: 0.15,
            grace_period_days: 90,
            floor_fraction: 0.01,
        }
    }
}
```

### Fisher Equation Connection (MV = PT)

Gesell's argument, formalized through Fisher's equation of exchange:

- **M** (money supply) = fixed by protocol (KORAI total supply)
- **V** (velocity) = increased by demurrage (forced circulation)
- **P** (price level) = stabilized because M is fixed and V is predictable
- **T** (transactions) = the economic activity we want to maximize

Demurrage cannot change M or T directly. It increases V, which for fixed M means each KORAI does more economic work. The agent economy benefits because the same token supply supports more agent-to-agent transactions, more compute payments, and more Mesh service fees.

### Why 1% For Now

KORAI's current 1% rate is deliberately conservative — below the 4-7% range suggested by historical data. Rationale:

1. **Adoption friction**: Higher rates discourage early adopters who are price-sensitive
2. **Simplicity**: 1% is easy to reason about and communicate
3. **Adjustability**: On-chain governance can increase the rate as the ecosystem matures
4. **Complement to knowledge demurrage**: Engram-level Ebbinghaus decay already provides strong circulation incentives; token demurrage is a secondary pressure

The calibration framework above provides the analytical tools for future rate adjustment.

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_recommended_rate_is_conservative() {
        let cal = DemurrageCalibration::default();
        let rate = cal.recommended_rate();
        // ln(3.0) * 0.4 ≈ 0.044
        assert!(rate > 0.04 && rate < 0.05);
    }

    #[test]
    fn grace_period_prevents_early_decay() {
        let cal = DemurrageCalibration::default();
        let balance = cal.effective_balance(1000.0, 0.1); // ~36 days
        assert_eq!(balance, 1000.0, "Within 90-day grace period");
    }

    #[test]
    fn floor_prevents_zero_balance() {
        let cal = DemurrageCalibration::default();
        let balance = cal.effective_balance(1000.0, 1000.0); // 1000 years
        assert!(balance >= 1000.0 * cal.floor_fraction);
    }

    #[test]
    fn high_velocity_target_increases_rate() {
        let cal = DemurrageCalibration {
            target_velocity_multiplier: 10.0,
            ..Default::default()
        };
        let rate = cal.recommended_rate();
        // ln(10.0) * 0.4 ≈ 0.092 = 9.2%/yr
        assert!(rate > 0.09);
    }
}
```

---

## Cross-References

- `docs/17-lifecycle/11-knowledge-demurrage.md` — Token-level knowledge decay
- `docs/02-scaffold/INDEX.md` — VCG Attention Auction, context compression
- `docs/08-chain/INDEX.md` — KORAI/DAEJI tokens, on-chain economics
- `docs/17-lifecycle/03-configuration-and-operator-model.md` — Budget configuration
