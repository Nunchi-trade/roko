# Cognitive Energy Model

> **Abstract:** Roko agents have explicit energy — a finite resource that depletes during
> cognitive work and replenishes during rest (Delta consolidation cycles). Energy mediates
> between attention tokens (what work costs) and affect (how the agent feels about working).
> When energy is high, the agent is capable, creative, and willing to take risks. When energy
> is low, the agent conserves, simplifies, and delegates. This document specifies the energy
> pool, depletion functions, recovery mechanisms, affect-energy coupling, and the integration
> that makes energy a first-class architectural concern.

> **Implementation**: Specified

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [10-three-cognitive-speeds](./10-three-cognitive-speeds.md), [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md), [25-attention-as-currency](./25-attention-as-currency.md)
**Key sources**:
- Gladstone et al. 2024, arXiv:2406.08862 — EBWM: Cognitively Inspired Energy-Based World Models
- arXiv:2509.20241 (2025) — Energy Use of AI Inference: Efficiency Pathways and Test-Time Compute
- arXiv:2503.17783 (2025) — Energy-Aware LLMs: Sustainable AI for Downstream Applications
- arXiv:2602.12236 (2026) — Energy-Aware Spike Budgeting for Continual Learning
- Kahneman 1973, "Attention and Effort" — Effort as scarce cognitive resource
- Hockey 2011, "A Motivational Control Theory of Cognitive Fatigue" — Compensatory control model
- Baumeister et al. 1998 — Ego depletion: limited self-regulatory resource model

---

## 1. The Problem: Unbounded Cognitive Work

Without an energy model, Roko agents have no concept of fatigue or diminishing returns. An
agent will run T2 inference on every tick, assemble maximum-size context windows, and gate-check
every output at full rigor — until the attention budget is exhausted in a single burst.

Real cognitive systems don't work this way. Humans (Kahneman 1973), biological neural networks,
and even power-constrained hardware (arXiv:2602.12236) exhibit *energy dynamics*: periods of
high performance followed by necessary recovery. The energy model introduces this dynamic to
Roko, creating natural rhythms of work and rest that:

1. **Prevent burnout**: Sustained high-intensity work degrades output quality
2. **Enable recovery**: Rest periods consolidate learning (Dreams) and improve future work
3. **Create adaptivity**: Energy level modulates risk tolerance, creativity, and delegation
4. **Align incentives**: The agent naturally paces itself instead of requiring external rate limiting

---

## 2. The Energy Pool

### 2.1 Core Type

```rust
/// Cognitive energy pool for a single agent.
///
/// Energy is a dimensionless scalar in [0.0, max_energy].
/// 1.0 energy ≈ the cost of one Gamma tick at T1 inference.
///
/// Energy differs from attention tokens:
///   - AT are spent and gone (like money)
///   - Energy depletes AND recovers (like biological stamina)
///   - AT buy specific operations; energy modulates capability level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveEnergy {
    /// Current energy level.
    pub current: f64,
    /// Maximum energy capacity (grows with experience).
    pub max_energy: f64,
    /// Base recovery rate per second (during idle).
    pub base_recovery_rate: f64,
    /// Current depletion rate (how fast energy is being consumed).
    pub depletion_rate: f64,
    /// Fatigue accumulator (long-term depletion that slow-recovers).
    pub fatigue: f64,
    /// Peak energy achieved this session.
    pub session_peak: f64,
    /// Total energy spent this session.
    pub session_spent: f64,
}

impl Default for CognitiveEnergy {
    fn default() -> Self {
        Self {
            current: 100.0,
            max_energy: 100.0,
            base_recovery_rate: 0.5,  // 0.5 energy/second during idle
            depletion_rate: 0.0,
            fatigue: 0.0,
            session_peak: 100.0,
            session_spent: 0.0,
        }
    }
}
```

### 2.2 Energy vs. Attention Tokens vs. Affect

Three distinct resource systems, each with different dynamics:

| Dimension | Attention Tokens | Cognitive Energy | Affect (Daimon) |
|---|---|---|---|
| **Nature** | Currency (spent) | Stamina (depletes + recovers) | Emotional state (continuous) |
| **Replenishment** | Per session / Delta cycle | Continuous (faster during rest) | Driven by outcomes |
| **Depletion cause** | Inference, context, gates | Sustained effort, difficult tasks | Failures, frustration |
| **Effect when low** | Cannot afford expensive ops | Reduced capability, forced conservation | Risk aversion, caution |
| **Recovery mechanism** | Budget allocation | Delta sleep cycles + idle recovery | Successes, novelty |
| **Time constant** | Session-scale (hours) | Minutes to hours | Ticks to sessions |

---

## 3. Depletion Functions

### 3.1 Per-Operation Energy Costs

Each cognitive operation has an energy cost that depends on the operation type and the
current energy level (tired agents spend more energy on the same task):

```rust
/// Energy cost per cognitive operation.
pub struct EnergyCostModel {
    /// Base costs per operation type (at full energy).
    pub base_costs: EnergyCosts,
    /// Fatigue penalty: how much extra energy is consumed when tired.
    /// Cost multiplier = 1.0 + fatigue_penalty × (1.0 - energy_fraction)
    pub fatigue_penalty: f64,  // default: 0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyCosts {
    /// T0 probe (habitual, almost free).
    pub t0_probe: f64,        // default: 0.1
    /// T1 fast inference.
    pub t1_inference: f64,    // default: 1.0
    /// T2 full inference.
    pub t2_inference: f64,    // default: 5.0
    /// Context assembly (per KB).
    pub context_per_kb: f64,  // default: 0.05
    /// Gate evaluation (per gate).
    pub gate_eval: f64,       // default: 0.3
    /// Theta reflection.
    pub theta_reflection: f64, // default: 3.0
    /// Delta consolidation (high cost, high recovery).
    pub delta_consolidation: f64, // default: 15.0
    /// Goal evaluation (per candidate).
    pub goal_evaluation: f64, // default: 0.5
}

impl Default for EnergyCosts {
    fn default() -> Self {
        Self {
            t0_probe: 0.1,
            t1_inference: 1.0,
            t2_inference: 5.0,
            context_per_kb: 0.05,
            gate_eval: 0.3,
            theta_reflection: 3.0,
            delta_consolidation: 15.0,
            goal_evaluation: 0.5,
        }
    }
}

impl EnergyCostModel {
    /// Compute actual energy cost given current energy level.
    ///
    /// Tired agents spend more energy (fatigue penalty).
    pub fn actual_cost(&self, base_cost: f64, energy: &CognitiveEnergy) -> f64 {
        let energy_fraction = energy.current / energy.max_energy.max(f64::EPSILON);
        let fatigue_multiplier = 1.0 + self.fatigue_penalty * (1.0 - energy_fraction);
        base_cost * fatigue_multiplier
    }

    /// Can the agent afford this operation at current energy?
    pub fn can_afford(&self, base_cost: f64, energy: &CognitiveEnergy) -> bool {
        let cost = self.actual_cost(base_cost, energy);
        energy.current >= cost
    }
}
```

### 3.2 Cognitive Difficulty Scaling

More difficult tasks consume disproportionately more energy. Difficulty is estimated from
task characteristics:

```rust
/// Estimate cognitive difficulty of a task.
///
/// Returns a multiplier in [1.0, 5.0] applied to base energy cost.
pub fn cognitive_difficulty(task: &TaskContext) -> f64 {
    let mut difficulty = 1.0;

    // Novelty: unfamiliar tasks are harder.
    difficulty += task.novelty_score * 1.5;  // 0.0 to 1.5

    // Complexity: more files, more dependencies = harder.
    let complexity = (task.files_involved as f64).log2().max(0.0) / 5.0;
    difficulty += complexity.min(1.0);

    // Ambiguity: unclear requirements are harder.
    difficulty += task.ambiguity_score * 1.0;  // 0.0 to 1.0

    difficulty.clamp(1.0, 5.0)
}
```

---

## 4. Recovery Mechanisms

### 4.1 Three Recovery Modes

Energy recovery mirrors Roko's three cognitive speeds:

```rust
/// Energy recovery model.
pub struct EnergyRecovery {
    /// Recovery rate during active Gamma work (minimal — "catching your breath").
    pub gamma_recovery_rate: f64,    // default: 0.05/sec
    /// Recovery rate during Theta reflection ("short break").
    pub theta_recovery_rate: f64,    // default: 0.3/sec
    /// Recovery rate during Delta consolidation ("deep rest").
    pub delta_recovery_rate: f64,    // default: 2.0/sec
    /// Recovery rate during idle (no active work).
    pub idle_recovery_rate: f64,     // default: 0.5/sec
    /// Fatigue recovery coefficient (fatigue recovers much slower).
    pub fatigue_recovery_rate: f64,  // default: 0.01/sec
}

impl EnergyRecovery {
    /// Update energy after a time interval at a given recovery mode.
    pub fn recover(
        &self,
        energy: &mut CognitiveEnergy,
        mode: RecoveryMode,
        elapsed_secs: f64,
    ) {
        let rate = match mode {
            RecoveryMode::Gamma => self.gamma_recovery_rate,
            RecoveryMode::Theta => self.theta_recovery_rate,
            RecoveryMode::Delta => self.delta_recovery_rate,
            RecoveryMode::Idle => self.idle_recovery_rate,
        };

        // Energy recovers up to (max_energy - fatigue)
        let effective_max = energy.max_energy - energy.fatigue;
        let recovery = rate * elapsed_secs;
        energy.current = (energy.current + recovery).min(effective_max);

        // Fatigue recovers slowly
        let fatigue_recovery = self.fatigue_recovery_rate * elapsed_secs;
        energy.fatigue = (energy.fatigue - fatigue_recovery).max(0.0);
    }
}

pub enum RecoveryMode {
    Gamma,
    Theta,
    Delta,
    Idle,
}
```

### 4.2 Sleep as Delta Recovery

Delta consolidation is Roko's "sleep" — it consumes energy initially (the consolidation
process itself costs energy) but produces a net positive recovery:

```
Energy during Delta cycle:

100│─────╲                    ╱─────
   │      ╲  consolidation  ╱
   │       ╲  cost (-15)   ╱ recovery (+40)
 60│        ╲             ╱
   │         ╲           ╱
 45│          ╲─────────╱
   │     active    rest/sleep    refreshed
   └────────────────────────────────────────
          Time during Delta cycle
```

```rust
/// Net energy change from a Delta consolidation cycle.
pub struct DeltaEnergyOutcome {
    /// Energy spent on consolidation itself.
    pub consolidation_cost: f64,
    /// Energy recovered during the rest phase.
    pub recovery_gained: f64,
    /// Fatigue reduced.
    pub fatigue_reduced: f64,
    /// Net energy change.
    pub net_change: f64,
    /// Max energy bonus (from experience/learning, permanent).
    pub capacity_growth: f64,
}

impl DeltaEnergyOutcome {
    pub fn compute(
        energy: &CognitiveEnergy,
        delta_duration_secs: f64,
        consolidation_quality: f64,  // 0.0 to 1.0, from Dreams
    ) -> Self {
        let consolidation_cost = 15.0;
        let recovery_gained = 2.0 * (delta_duration_secs - 7.5).max(0.0);
        let fatigue_reduced = energy.fatigue * consolidation_quality * 0.5;
        let net_change = recovery_gained - consolidation_cost;

        // Successful consolidation slightly increases max capacity
        // (the agent gets "fitter" over time).
        let capacity_growth = consolidation_quality * 0.1;

        Self {
            consolidation_cost,
            recovery_gained,
            fatigue_reduced,
            net_change,
            capacity_growth,
        }
    }
}
```

---

## 5. Energy-Affect Coupling

### 5.1 Bidirectional Influence

Energy and affect (Daimon) form a bidirectional coupling: energy level influences affect,
and affect influences energy dynamics.

```rust
/// Bidirectional coupling between energy and affect.
pub struct EnergyAffectCoupling {
    // Energy → Affect direction
    /// Low energy reduces pleasure (tiredness feels bad).
    pub energy_to_pleasure: f64,       // default: 0.3
    /// Low energy reduces dominance (tiredness feels helpless).
    pub energy_to_dominance: f64,      // default: 0.2
    /// Very low energy increases arousal (stress response).
    pub critical_energy_arousal: f64,  // default: 0.4

    // Affect → Energy direction
    /// High pleasure reduces energy cost (work feels easier when happy).
    pub pleasure_cost_discount: f64,   // default: 0.15
    /// High arousal increases energy consumption (excitement burns energy).
    pub arousal_cost_premium: f64,     // default: 0.1
    /// High dominance improves recovery rate (feeling in control helps rest).
    pub dominance_recovery_bonus: f64, // default: 0.2
}

impl EnergyAffectCoupling {
    /// Compute PAD delta from current energy state.
    pub fn energy_to_pad(&self, energy: &CognitiveEnergy) -> PadVector {
        let fraction = energy.current / energy.max_energy.max(f64::EPSILON);

        // Low energy reduces pleasure and dominance
        let pleasure_delta = if fraction < 0.3 {
            -self.energy_to_pleasure * (1.0 - fraction / 0.3)
        } else {
            0.0
        };

        let dominance_delta = if fraction < 0.4 {
            -self.energy_to_dominance * (1.0 - fraction / 0.4)
        } else {
            0.0
        };

        // Critical energy triggers stress arousal
        let arousal_delta = if fraction < 0.15 {
            self.critical_energy_arousal * (1.0 - fraction / 0.15)
        } else {
            0.0
        };

        PadVector {
            pleasure: pleasure_delta,
            arousal: arousal_delta,
            dominance: dominance_delta,
        }
    }

    /// Modulate energy cost based on current affect.
    pub fn affect_cost_modifier(&self, pad: &PadVector) -> f64 {
        let pleasure_mod = -self.pleasure_cost_discount * pad.pleasure.clamp(-1.0, 1.0);
        let arousal_mod = self.arousal_cost_premium * pad.arousal.clamp(0.0, 1.0);
        1.0 + pleasure_mod + arousal_mod
    }

    /// Modulate recovery rate based on current affect.
    pub fn affect_recovery_modifier(&self, pad: &PadVector) -> f64 {
        let dominance_mod = self.dominance_recovery_bonus * pad.dominance.clamp(0.0, 1.0);
        1.0 + dominance_mod
    }
}
```

### 5.2 Energy Zones and Behavioral States

Energy level maps to behavioral zones that constrain the agent's operating mode:

```rust
/// Energy zones that determine the agent's operating capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnergyZone {
    /// 80-100%: Full capability. T2 allowed. Creative exploration enabled.
    Peak,
    /// 50-80%: Normal operation. All tiers allowed. Standard behavior.
    Normal,
    /// 25-50%: Conservation mode. Prefer T0/T1. Avoid novel tasks.
    Conserving,
    /// 10-25%: Low power. T0 only. Complete current task, defer new ones.
    LowPower,
    /// 0-10%: Critical. Shutdown non-essential operations. Trigger Delta cycle.
    Critical,
}

impl EnergyZone {
    pub fn from_energy(energy: &CognitiveEnergy) -> Self {
        let fraction = energy.current / energy.max_energy.max(f64::EPSILON);
        if fraction >= 0.80 { Self::Peak }
        else if fraction >= 0.50 { Self::Normal }
        else if fraction >= 0.25 { Self::Conserving }
        else if fraction >= 0.10 { Self::LowPower }
        else { Self::Critical }
    }

    /// Maximum inference tier allowed in this zone.
    pub fn max_tier(&self) -> InferenceTier {
        match self {
            Self::Peak | Self::Normal => InferenceTier::T2,
            Self::Conserving => InferenceTier::T1,
            Self::LowPower | Self::Critical => InferenceTier::T0,
        }
    }

    /// Should the agent trigger a Delta consolidation cycle?
    pub fn should_trigger_delta(&self) -> bool {
        matches!(self, Self::Critical)
    }

    /// Maximum concurrent active goals in this zone.
    pub fn max_active_goals(&self) -> usize {
        match self {
            Self::Peak => 5,
            Self::Normal => 3,
            Self::Conserving => 2,
            Self::LowPower => 1,
            Self::Critical => 0,
        }
    }
}
```

---

## 6. Capacity Growth (Fitness)

Over time, successful work and effective consolidation increase the agent's energy capacity:

```rust
/// Long-term energy capacity model.
///
/// max_energy grows logarithmically with successful task completions,
/// representing the agent becoming "fitter" at its work.
pub struct EnergyCapacityModel {
    /// Base max energy (starting value).
    pub base_capacity: f64,      // default: 100.0
    /// Growth coefficient per successful Delta cycle.
    pub growth_per_delta: f64,   // default: 0.1
    /// Growth coefficient per successful task completion.
    pub growth_per_task: f64,    // default: 0.02
    /// Absolute maximum capacity (ceiling).
    pub capacity_ceiling: f64,   // default: 200.0
    /// Decay rate on capacity if unused (use-it-or-lose-it).
    pub disuse_decay: f64,       // default: 0.001 per hour
}

impl EnergyCapacityModel {
    pub fn update_capacity(
        &self,
        energy: &mut CognitiveEnergy,
        successful_tasks: u32,
        successful_deltas: u32,
        hours_since_last_work: f64,
    ) {
        // Growth from work
        let task_growth = self.growth_per_task * successful_tasks as f64;
        let delta_growth = self.growth_per_delta * successful_deltas as f64;

        // Decay from disuse
        let disuse_loss = self.disuse_decay * hours_since_last_work;

        let new_max = (energy.max_energy + task_growth + delta_growth - disuse_loss)
            .clamp(self.base_capacity, self.capacity_ceiling);

        energy.max_energy = new_max;
    }
}
```

---

## 7. Integration with Attention Economy

Energy and attention tokens are complementary constraints:

```rust
/// Energy-gated attention spending.
///
/// Even if the agent has enough AT for T2, if energy is too low,
/// the energy gate prevents the expensive operation.
pub struct EnergyAttentionGate {
    pub energy: CognitiveEnergy,
    pub cost_model: EnergyCostModel,
    pub coupling: EnergyAffectCoupling,
}

impl EnergyAttentionGate {
    /// Can the agent afford this operation in both AT and energy?
    pub fn can_afford(
        &self,
        at_cost: AttentionToken,
        energy_base_cost: f64,
        at_budget: &AttentionBudget,
        pad: &PadVector,
    ) -> bool {
        // Check AT budget
        let at_ok = at_budget.session_remaining.value() >= at_cost.value();

        // Check energy budget (with affect modulation)
        let affect_mod = self.coupling.affect_cost_modifier(pad);
        let actual_energy_cost = self.cost_model.actual_cost(energy_base_cost, &self.energy)
            * affect_mod;
        let energy_ok = self.energy.current >= actual_energy_cost;

        // Check energy zone allows this tier
        let zone = EnergyZone::from_energy(&self.energy);
        let zone_ok = match at_cost.value() as u64 {
            c if c >= AttentionToken::T2_COST.value() as u64 =>
                zone.max_tier() >= InferenceTier::T2,
            c if c >= AttentionToken::T1_COST.value() as u64 =>
                zone.max_tier() >= InferenceTier::T1,
            _ => true,  // T0 always allowed
        };

        at_ok && energy_ok && zone_ok
    }
}
```

---

## 8. Telemetry

```rust
/// Per-tick energy telemetry, logged to .roko/learn/energy.jsonl.
#[derive(Serialize, Deserialize)]
pub struct EnergyTelemetry {
    pub tick_id: u64,
    pub speed: CognitiveSpeed,
    pub energy_before: f64,
    pub energy_after: f64,
    pub energy_spent: f64,
    pub energy_recovered: f64,
    pub fatigue_level: f64,
    pub zone: String,  // "Peak", "Normal", "Conserving", "LowPower", "Critical"
    pub max_energy: f64,
    pub affect_cost_modifier: f64,
    pub cognitive_difficulty: f64,
    pub delta_triggered: bool,
}
```

---

## 9. Configuration

```toml
[energy]
# Enable cognitive energy model.
enabled = true

[energy.pool]
# Starting and base max energy.
base_capacity = 100.0
# Absolute ceiling on max energy.
capacity_ceiling = 200.0
# Base recovery rate during idle (energy/second).
idle_recovery_rate = 0.5

[energy.costs]
t0_probe = 0.1
t1_inference = 1.0
t2_inference = 5.0
context_per_kb = 0.05
gate_eval = 0.3
theta_reflection = 3.0
delta_consolidation = 15.0
goal_evaluation = 0.5

[energy.fatigue]
# Extra energy cost when tired (multiplier).
fatigue_penalty = 0.5
# Fatigue recovery rate (per second, during any mode).
fatigue_recovery_rate = 0.01

[energy.recovery]
# Recovery rates by mode (energy/second).
gamma_recovery = 0.05
theta_recovery = 0.3
delta_recovery = 2.0

[energy.zones]
# Zone boundaries (as fraction of max_energy).
peak_threshold = 0.80
normal_threshold = 0.50
conserving_threshold = 0.25
low_power_threshold = 0.10

[energy.coupling]
# Energy → Affect coefficients.
energy_to_pleasure = 0.3
energy_to_dominance = 0.2
critical_energy_arousal = 0.4
# Affect → Energy coefficients.
pleasure_cost_discount = 0.15
arousal_cost_premium = 0.1
dominance_recovery_bonus = 0.2

[energy.capacity]
# Capacity growth rates.
growth_per_delta = 0.1
growth_per_task = 0.02
# Disuse decay (per hour without work).
disuse_decay = 0.001
```

---

## 10. Integration Wiring

### 10.1 Into the Universal Cognitive Loop

| Loop Step | Energy Integration |
|---|---|
| 1. PERCEIVE | No direct energy cost (query is free) |
| 2. EVALUATE | Scoring costs energy (goal_evaluation cost per candidate) |
| 3. ATTEND | Auction slot count modulated by energy zone |
| 4. INTEGRATE | Context assembly costs energy (context_per_kb) |
| 5. ACT | Inference costs energy (t0/t1/t2); zone gates max tier |
| 6. VERIFY | Gate evaluation costs energy (gate_eval per gate) |
| 7. PERSIST | No energy cost (persistence is free) |
| 8. ADAPT | Theta reflection costs energy (theta_reflection) |
| 9. META-COGNIZE | Energy state → Daimon PAD delta; zone determines next tick behavior |

### 10.2 Into Existing Crates

| Crate | Integration Point | Change |
|---|---|---|
| `roko-core` | `Context` struct | Add `energy: CognitiveEnergy` field |
| `roko-learn` | `CascadeRouter` | Energy zone constrains tier selection |
| `roko-daimon` | `DaimonState` | Receive energy PAD delta per tick |
| `roko-orchestrator` | `loop_tick()` | Deduct energy per operation; trigger Delta if Critical |
| `roko-dreams` | Delta cycle | Produce `DeltaEnergyOutcome`; update capacity |
| `roko-conductor` | Circuit breaker | Trigger on sustained LowPower (5+ ticks) |
| `roko-compose` | `Budget` | Energy-aware context size limits |
| `roko-cli` | `roko status` | Display energy level, zone, fatigue |

---

## 11. Test Criteria

| Test | What It Validates | Type |
|---|---|---|
| `test_energy_depletion_t2` | T2 inference depletes 5.0 base energy | Unit |
| `test_fatigue_penalty_increases_cost` | At 20% energy, T2 costs 7.0 (5.0 × 1.4) | Unit |
| `test_recovery_idle_rate` | Idle recovery at 0.5/sec over 10s = +5.0 energy | Unit |
| `test_recovery_delta_net_positive` | Delta cycle costs -15 but recovers net positive | Unit |
| `test_zone_peak_allows_t2` | At 90% energy, T2 is allowed | Unit |
| `test_zone_conserving_blocks_t2` | At 40% energy, T2 is blocked | Unit |
| `test_zone_critical_triggers_delta` | At 5% energy, Delta cycle auto-triggered | Unit |
| `test_energy_affect_coupling_low_energy` | Low energy reduces pleasure and dominance | Unit |
| `test_energy_affect_coupling_critical_arousal` | Critical energy increases arousal | Unit |
| `test_pleasure_reduces_cost` | High pleasure → 15% cost discount | Unit |
| `test_arousal_increases_cost` | High arousal → 10% cost premium | Unit |
| `test_capacity_growth_from_tasks` | Successful tasks increase max_energy | Unit |
| `test_capacity_disuse_decay` | Long idle period decreases max_energy | Unit |
| `test_capacity_ceiling_enforced` | max_energy cannot exceed 200.0 | Unit |
| `test_energy_attention_gate_dual_check` | Both AT and energy must be sufficient | Integration |
| `test_telemetry_logged_per_tick` | EnergyTelemetry written every tick | Integration |
| `test_max_goals_by_zone` | Peak=5, Normal=3, Conserving=2, LowPower=1, Critical=0 | Unit |

---

## 12. Theoretical Foundations

### 12.1 Resource Theory of Attention (Kahneman 1973)

Kahneman modeled attention as drawing from a limited "effort supply." The supply replenishes
over time but is consumed by effortful processing. The cognitive energy model extends this from
attention specifically to general cognitive capacity — the energy pool is Kahneman's "effort
supply" made explicit and computable.

### 12.2 Compensatory Control Model (Hockey 2011)

Hockey's model of cognitive fatigue distinguishes between:
- **Performance protection**: Maintaining output quality by spending more effort (our fatigue penalty)
- **Strategy adjustment**: Switching to less effortful strategies (our energy zone degradation)
- **Goal disengagement**: Abandoning current goals (our Critical zone → abandon non-essential goals)

Roko implements all three levels: the fatigue penalty models performance protection, zone
transitions model strategy adjustment, and the goal count limits model goal disengagement.

### 12.3 Energy-Based World Models (Gladstone et al. 2024)

EBWM demonstrated that energy scalars can effectively model state plausibility and guide
compute allocation. Their key insight: adaptive computation (allocating more time to harder
predictions) naturally emerges from an energy-based framework. In Roko, this maps to the
cognitive difficulty scaling — harder tasks consume more energy, creating natural pressure
to develop better heuristics (lower-energy strategies) for recurring patterns.

### 12.4 Ego Depletion (Baumeister et al. 1998)

While the ego depletion model is debated in psychology, its computational analog is well-
established: sustained high-intensity computation degrades output quality (e.g., neural
network training under compute constraints). Roko's energy model captures this:

```
Output quality ∝ base_quality × energy_fraction^0.3

At full energy: quality = base × 1.0
At 50% energy: quality = base × 0.81
At 25% energy: quality = base × 0.66
At 10% energy: quality = base × 0.50
```

The sublinear relationship (exponent 0.3) means quality degrades gracefully, not catastrophically.

---

## 13. Open Questions

1. **Multi-agent energy**: When agents share a pool, should energy be individually managed or
   collectively pooled? Hockey (2011) suggests individual management with collective monitoring.
2. **Energy lending**: Can a high-energy agent "lend" energy to a depleted agent in a multi-agent
   system? This would require an energy transfer protocol.
3. **Circadian rhythms**: Should the base recovery rate vary with time of day to match the
   user's work patterns?
4. **Energy as reward signal**: Should energy recovery itself be an intrinsic reward in the
   goal emergence engine? ("I'm tired → Goal: rest" as emergent behavior.)

---

## Cross-References

- [10-three-cognitive-speeds](./10-three-cognitive-speeds.md) — Gamma/Theta/Delta map to energy depletion/recovery modes
- [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) — Daimon affect coupling with energy
- [25-attention-as-currency](./25-attention-as-currency.md) — AT and energy as dual constraints
- [26-cognitive-immune-system](./26-cognitive-immune-system.md) — Immune responses consume energy
- [28-emergent-goal-structures](./28-emergent-goal-structures.md) — Energy zones limit active goal count
- [27-temporal-knowledge-topology](./27-temporal-knowledge-topology.md) — Temporal patterns of energy usage
- [Topic 05: Learning](../05-learning/INDEX.md) — Energy telemetry feeds efficiency tracking
- [Topic 07: Conductor](../07-conductor/INDEX.md) — Circuit breaker on sustained low energy
- [Topic 09: Daimon](../09-daimon/INDEX.md) — Bidirectional energy-affect coupling
- [Topic 10: Dreams](../10-dreams/INDEX.md) — Delta consolidation as primary recovery mechanism
