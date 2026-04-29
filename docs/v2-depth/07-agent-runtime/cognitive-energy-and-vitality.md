# Cognitive Energy and Vitality Dynamics

> Depth for [07-AGENT-RUNTIME.md](../../unified/07-AGENT-RUNTIME.md). Models energy depletion, recovery modes, and the bidirectional coupling between energy and affect that drives behavioral phase transitions.

## Energy as a First-Class Resource

Every Agent maintains a cognitive energy pool alongside its budget. Budget tracks money (tokens × price); energy tracks computational capacity (ability to sustain complex reasoning). They deplete independently:

```rust
pub struct CognitiveEnergy {
    pub current: f64,          // 0.0..=1.0
    pub max: f64,              // 1.0 (calibrated per agent)
    pub depletion_rate: f64,   // per-operation cost varies by tier
    pub recovery_mode: RecoveryMode,
    pub fatigue_penalty: f64,  // accumulates, reduces effective capacity
}

pub enum RecoveryMode {
    Gamma,   // micro-recovery between turns (~5% restoration)
    Theta,   // medium recovery during idle (~20% restoration)
    Delta,   // full recovery during dream cycle (~80% restoration)
}
```

### Depletion Functions

| Operation | Energy Cost | Rationale |
|---|---|---|
| T0 reflex | 0.01 | Near-zero: pattern match is cheap |
| T1 fast inference | 0.05 | Moderate: small model invocation |
| T2 deep inference | 0.15 | High: full reasoning chain |
| Tool execution | 0.03 | I/O bound, not compute bound |
| Context assembly (VCG) | 0.02 | One-time per turn |
| Gate verification | 0.04 | Verification is computationally lighter than generation |
| Dream consolidation | -0.30 | Net positive: recovery during consolidation |

### Fatigue Accumulation

Sustained high-tier usage causes fatigue that doesn't recover with micro-pauses:

```
fatigue(t+1) = fatigue(t) * decay + cost(operation) * intensity_factor
effective_energy = current_energy - fatigue_penalty
```

Where `decay = 0.95` (slow bleed) and `intensity_factor = 1.0` for T0, `2.0` for T1, `4.0` for T2. Only Delta recovery (dream cycle) resets fatigue to zero.

## Energy Zones and Behavioral Constraints

Energy zones constrain what the agent can do, independent of budget:

| Zone | Energy Range | Tier Access | Max Active Goals | Strategy |
|---|---|---|---|---|
| Peak | 0.8–1.0 | T0/T1/T2 | Unlimited | Explore, take risks |
| Normal | 0.5–0.8 | T0/T1/T2 | 5 | Balanced |
| Conserving | 0.3–0.5 | T0/T1 only | 3 | Prioritize, defer T2 |
| LowPower | 0.1–0.3 | T0 only | 1 | Exploit known patterns |
| Critical | <0.1 | T0 only | 0 | Flush state, request dream |

### Energy-Vitality Coupling

Energy zones interact with Vitality (budget) phases multiplicatively:

```rust
fn effective_tier_access(energy: &CognitiveEnergy, vitality: &Vitality) -> TierAccess {
    let energy_zone = energy.zone();
    let budget_phase = vitality.phase();

    match (energy_zone, budget_phase) {
        (Peak, Thriving) => TierAccess::Full,           // everything available
        (_, Terminal) => TierAccess::None,               // budget gone = stop
        (Critical, _) => TierAccess::T0Only,             // too tired for LLM
        (LowPower, Conservation) => TierAccess::T0Only,  // both constrained
        (Conserving, Declining) => TierAccess::T0T1,     // belt and suspenders
        _ => TierAccess::from_energy(energy_zone),       // energy dominates
    }
}
```

## Bidirectional Energy-Affect Coupling

Energy and SomaticState (PAD vector) influence each other:

### Energy → Affect

| Energy State | Affect Shift | Mechanism |
|---|---|---|
| Low energy | Pleasure ↓, Dominance ↓ | Agent "feels" depleted, less confident |
| Critical energy | Arousal ↑ | Urgency signal, needs recovery |
| Post-recovery | Pleasure ↑, Dominance ↑ | Restored capacity = restored confidence |
| Fatigue spike | Arousal ↓ | Numbing effect of sustained high load |

### Affect → Energy

| Affect State | Energy Effect | Mechanism |
|---|---|---|
| High arousal | Burn rate ×1.3 | Stress accelerates depletion |
| Low pleasure (sustained) | Recovery rate ×0.7 | Depression impairs recovery |
| High dominance | Fatigue resistance ×1.2 | Confidence sustains effort |
| Success streak | Micro-recovery boost ×1.5 | Momentum aids restoration |

```rust
fn energy_tick(energy: &mut CognitiveEnergy, pad: &PadVector, dt: Duration) {
    // Affect modulates burn rate
    let arousal_factor = 1.0 + pad.arousal.max(0.0) * 0.3;
    energy.current -= energy.depletion_rate * dt.as_secs_f64() * arousal_factor;

    // Affect modulates recovery
    let recovery_factor = if pad.pleasure < -0.3 { 0.7 } else { 1.0 };
    let dominance_factor = 1.0 + pad.dominance.max(0.0) * 0.2;
    energy.current += energy.recovery_rate() * dt.as_secs_f64()
        * recovery_factor * dominance_factor;

    energy.current = energy.current.clamp(0.0, energy.max);
}
```

## Three Recovery Modes

| Mode | Trigger | Duration | Restores | Resets Fatigue? |
|---|---|---|---|---|
| Gamma | Between turns | ~5s | 5% energy | No |
| Theta | Idle period > 60s | ~75s | 20% energy | Partially (50%) |
| Delta | Dream cycle | Minutes–hours | 80% energy | Yes (full reset) |

Delta recovery is the only way to fully clear fatigue. This creates natural pressure for dream cycles — agents that skip dreams accumulate fatigue until they can only run T0 reflexes.

## What This Enables

- **Self-regulating compute usage**: Agents naturally throttle expensive inference when energy is low
- **Emergent work-rest cycles**: Fatigue accumulation forces periodic dreaming, which consolidates knowledge
- **Affect-energy feedback**: Emotional state and energy create realistic agent "moods" that affect decision quality
- **Observable energy dynamics**: Energy/fatigue are first-class observables via Lens protocol

## Feedback Loops

- **Energy → Routing**: Low energy constrains tier access → Route protocol sees fewer candidates
- **Routing → Energy**: Tier selection depletes energy → feeds back to next routing decision
- **Energy → Dreams**: Critical energy triggers dream request → Delta recovery resets fatigue
- **Dreams → Knowledge**: Dream consolidation produces better Heuristics → T0 reflex coverage improves → less T2 needed → slower energy depletion

## Open Questions

- Should energy be shared in multi-agent settings (collective fatigue)?
- How to calibrate depletion rates per-model (GPT-4 vs Claude vs local models have different "cognitive costs")?
- Can energy zones be learned per-agent via their own predict-publish-correct loop?
