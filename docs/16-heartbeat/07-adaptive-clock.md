# Adaptive Clock

> The runtime component that manages all three cognitive speeds — gamma, theta, and delta — adjusting each frequency based on environmental regime and resource constraints.


> **Implementation**: Specified

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [03-three-cognitive-speeds.md](./03-three-cognitive-speeds.md)
**Key sources**: `refactoring-prd/02-five-layers.md` §Adaptive Clock, legacy source `bardo-backup/prd/01-golem/18-cortical-state.md` §Adaptive Clock, `implementation-plans/12a-cognitive-layer.md` §I4

---

## Abstract

The adaptive clock is the runtime policy in `roko-runtime` (L0 Runtime layer) that publishes the three heartbeat tick Pulses on the Bus. It does not orchestrate cognition directly. Instead, a `HeartbeatPolicy` emits `heartbeat.gamma.tick`, `heartbeat.theta.tick`, and `heartbeat.delta.tick` at cadence, and the speed-specific consumers subscribe by topic. That makes the clock a Bus producer, not a special control-flow mechanism.

It still dynamically adjusts each frequency based on environmental regime (calm vs. volatile), resource constraints (approaching budget ceiling), and agent behavioral state (focused vs. resting). The fixed ~60-second heartbeat from simpler agent architectures serves all purposes at one rate. This is wrong. A code compilation check resolves in seconds. A deployment pipeline takes minutes. A research synthesis takes hours. Biology solves this with oscillatory hierarchies (Buzsáki 2006, "Rhythms of the Brain", Oxford University Press): fast gamma oscillations (30-100 Hz) ride on top of slower theta oscillations (4-8 Hz), which ride on top of delta oscillations (0.5-4 Hz). The adaptive clock implements this hierarchy for agent cognition.

The adaptive clock draws from Friston's (2010) free energy principle: the agent should sample its environment more frequently when prediction error is high (the environment is surprising) and less frequently when prediction error is low (the environment is predictable). Clark (2013, "Whatever Next?", Behavioral and Brain Sciences 36(3)) extends this into the "predictive brain" framework: biological cognition is nested prediction loops at multiple timescales, with each timescale adapting independently.

This document specifies the adaptive clock: its configuration, the regime detection system, the three-frequency scheduler, budget-aware throttling, and the Bus topic delivery model for inter-loop communication.

See also `tmp/refinements/09-phase-2-implications.md` and [Naming and Glossary](../00-architecture/01-naming-and-glossary.md) for the two-fabric framing and the Bus / Pulse / Topic vocabulary.

---

## Clock Configuration

The adaptive clock is configured in `roko.toml`:

```toml
[clock]
# Gamma: reactive perception (5-15s)
gamma_min_interval_secs = 5
gamma_max_interval_secs = 15
gamma_base_interval_secs = 10

# Theta: reflective planning (30-120s)
theta_min_interval_secs = 15
theta_max_interval_secs = 120
theta_base_interval_secs = 75
theta_gamma_count = 5  # fire theta every N gamma ticks

# Delta: consolidation
delta_episode_threshold = 50  # fire after N unprocessed episodes
delta_idle_timeout_secs = 300  # fire after 5 min idle
delta_scheduled_utc = "02:00"  # optional nightly schedule

# Budget-aware throttling
daily_budget_usd = 50.0
throttle_at_percent = 80  # start throttling at 80% budget
hard_stop_at_percent = 95  # stop T2 calls at 95% budget
```

All intervals have minimum and maximum bounds. The adaptive logic operates within these bounds — it can never make gamma faster than 5 seconds or slower than 15 seconds, regardless of regime. Those bounds define the cadence of `HeartbeatPolicy`; they do not change the Bus topic shape.

---

## Regime Detection

The adaptive clock adjusts emission cadence based on the detected environmental regime. Regime detection is performed by the gamma loop's T0 probes (see [09-16-t0-probes.md](./09-16-t0-probes.md)) and is domain-specific:

| Regime | Chain Domain | Coding Domain | Research Domain | Universal |
|---|---|---|---|---|
| **Calm** | Low volatility, stable positions, low gas | All tests passing, no build errors, stable deps | No new sources, low citation churn | Prediction error < 0.1 |
| **Normal** | Moderate market movement | Some test flakiness, normal development | Moderate new source discovery | Prediction error 0.1-0.3 |
| **Volatile** | High price swings, gas spikes, liquidation risk | Build failures, test regressions, security alerts | Major new findings, contradictory sources | Prediction error 0.3-0.6 |
| **Crisis** | Flash crash, protocol exploit, mass liquidation | Critical production outage, data breach | Fundamental paradigm challenge | Prediction error > 0.6 |

Regime is stored in the CorticalState (`regime: AtomicU8`) and is read by the `HeartbeatPolicy` to adjust tick emission for all three speeds.

---

## Frequency Adjustment Rules

Each frequency has its own adjustment function. All three adapt independently but share the same regime input.

### Gamma Adaptation

Gamma adapts based on the number of probe anomalies:

```rust
/// More anomalies → faster gamma (more frequent perception).
/// Fewer anomalies → slower gamma (less frequent sampling).
///
/// This implements Friston's (2010) active sampling: sample the
/// environment more when prediction error is high.
fn compute_gamma_interval(
    violations: &[Violation],
    config: &ClockConfig,
) -> Duration {
    let base = Duration::from_secs(config.gamma_max_interval_secs);
    let adjusted = base.mul_f64(1.0 / (1.0 + violations.len() as f64 * 0.3));
    adjusted
        .max(Duration::from_secs(config.gamma_min_interval_secs))
        .min(Duration::from_secs(config.gamma_max_interval_secs))
}
```

| Anomaly Count | Interval | Ticks/Hour |
|---|---|---|
| 0 | 15.0s | 240 |
| 1 | 11.5s | 313 |
| 2 | 9.4s | 383 |
| 3 | 7.9s | 456 |
| 5 | 6.0s | 600 |
| 7+ | 5.0s (floor) | 720 |

### Theta Adaptation

Theta adapts based on regime multipliers:

```rust
/// Theta interval adjusts with regime-based multipliers.
///
/// Volatile periods → more frequent reflection.
/// Calm periods → less frequent reflection.
fn compute_theta_interval(
    regime: Regime,
    config: &ClockConfig,
) -> Duration {
    let multiplier = match regime {
        Regime::Calm => 1.6,
        Regime::Normal => 1.0,
        Regime::Volatile => 0.4,
        Regime::Crisis => 0.2,
    };
    let base = Duration::from_secs(config.theta_base_interval_secs);
    Duration::from_secs_f64(base.as_secs_f64() * multiplier)
        .max(Duration::from_secs(config.theta_min_interval_secs))
        .min(Duration::from_secs(config.theta_max_interval_secs))
}
```

| Regime | Multiplier | Theta Interval | Ticks/Hour |
|---|---|---|---|
| Calm | 1.6× | 120s | 30 |
| Normal | 1.0× | 75s | 48 |
| Volatile | 0.4× | 30s | 120 |
| Crisis | 0.2× | 15s | 240 |

### Delta Timing

Delta does not have an adaptive interval in the same sense. It fires based on triggers (idle detection, episode count, schedule) rather than a periodic timer. However, the episode threshold can be adjusted:

- During volatile periods, the episode threshold drops to 30 (consolidate more frequently because episodes are more informative).
- During calm periods, the threshold rises to 80 (less urgency to consolidate routine observations).

---

## The Three-Frequency Scheduler

The adaptive clock runs as the top-level tick producer for three concurrent topic streams. It owns the `CancellationToken` hierarchy for its own lifecycle, but it does not own cognition. Its job is to emit the tick Pulses and let the consumers run:

```rust
/// The adaptive clock: emits gamma, theta, and delta tick Pulses.
///
/// Lives at L0 Runtime. Dependencies flow strictly downward — the policy
/// has no knowledge of domain-specific cognition.
pub struct AdaptiveClock {
    config: ClockConfig,
    gamma_interval: AtomicU64,   // stored as milliseconds
    theta_interval: AtomicU64,
    regime: AtomicU8,
    cancel: CancellationToken,
    budget_tracker: Arc<BudgetTracker>,
}

impl AdaptiveClock {
    /// Start the three-frequency tick producer.
    ///
    /// All three topics are emitted independently. Gamma has priority
    /// in cadence selection, but the policy never calls into a consumer
    /// directly.
    pub async fn run(
        &self,
        state: Arc<RwLock<AgentState>>,
    ) -> Result<()> {
        let _ = state;
        tokio::select! {
            _ = self.emit_gamma_ticks() => {},
            _ = self.emit_theta_ticks() => {},
            _ = self.emit_delta_ticks() => {},
            _ = self.cancel.cancelled() => Ok(()),
        }
    }
}
```

### Priority and Collision Handling

When gamma and theta are both due, the policy emits the gamma tick first and lets the theta consumer catch up on its own queue. The producer never invokes the consumer directly.

When gamma work arrives during delta, the policy keeps publishing the gamma topic and the delta consumer yields to the higher-priority queue. The delta consumer resumes when its topic stream has capacity again.

---

## Budget-Aware Throttling

The adaptive clock tracks daily spending and adjusts tick emission when approaching budget limits:

```rust
/// Budget-aware frequency throttling.
///
/// When daily spending approaches the configured budget, the policy
/// progressively reduces expensive operations:
/// - At 80% budget: extend theta intervals by 2×
/// - At 90% budget: extend theta intervals by 4×, limit T2 to crisis only
/// - At 95% budget: stop all T2 calls, theta at maximum interval
///
/// This ensures the agent never exceeds its daily budget while
/// maintaining minimum perception (gamma T0 probes) at all times.
fn apply_budget_throttle(
    interval: Duration,
    budget_pct: f64,
    config: &ClockConfig,
) -> Duration {
    if budget_pct >= config.hard_stop_at_percent as f64 / 100.0 {
        // 95%+: maximum intervals, T0 only
        Duration::from_secs(config.theta_max_interval_secs)
    } else if budget_pct >= 0.90 {
        // 90-95%: 4× slowdown, T2 restricted to crisis
        interval.mul_f64(4.0)
            .min(Duration::from_secs(config.theta_max_interval_secs))
    } else if budget_pct >= config.throttle_at_percent as f64 / 100.0 {
        // 80-90%: 2× slowdown
        interval.mul_f64(2.0)
            .min(Duration::from_secs(config.theta_max_interval_secs))
    } else {
        interval // Below threshold: no throttling
    }
}
```

The key insight: **gamma T0 probes always run** regardless of budget. They cost $0.00 (pure computation). Even at 100% budget utilization, the policy can still emit `heartbeat.gamma.tick` Pulses, so the agent maintains perception — it can see what's happening, it just can't deliberate about it. This means the agent never goes blind, even when it runs out of budget.

When T2 is restricted, the agent falls back to T1 (cheaper model) for situations that would normally trigger T2. If T1 is also restricted (extreme budget pressure), the agent operates on T0 only — playbook rules and deterministic heuristics handle everything.

---

## Cognitive Signals

The three speeds communicate by Bus topic subscription rather than by direct loop-to-loop control. The `HeartbeatPolicy` emits tick Pulses, and each consumer watches the topic it cares about:

```rust
let gamma_topic = "heartbeat.gamma.tick";
let theta_topic = "heartbeat.theta.tick";
let delta_topic = "heartbeat.delta.tick";
```

The policy can still emit a higher-priority gamma tick early when needed, but that is a topic emission decision, not an inline control path.

---

## Event-Driven Wakeup

In addition to periodic cadence, the policy can emit an early gamma tick when urgency changes:

```rust
/// Early gamma tick conditions.
///
/// These override the normal gamma interval, triggering an
/// immediate gamma tick emission regardless of the timer.
pub enum WakeupCondition {
    /// External intervention from user (steer/followUp)
    UserIntervention,
    /// Internal intervention from safety system
    SafetyAlert,
    /// Pheromone field threat signal from mesh peers
    PheromoneAlert { intensity: f32 },
    /// Budget alert (approaching hard stop)
    BudgetAlert,
    /// Scheduled event (e.g., deployment window opens)
    ScheduledEvent(EventId),
}
```

When a wakeup condition fires, the policy skips the remaining sleep time and immediately emits a new `heartbeat.gamma.tick` Pulse. This ensures that urgent conditions are processed within milliseconds rather than waiting up to 15 seconds for the next scheduled tick.

---

## Mapping to Existing Code

The adaptive clock lives at L0 Runtime in `roko-runtime`.

| Component | Current Status | Target |
|---|---|---|
| Process lifecycle | Runtime process supervisor module | Extend with three-loop management |
| Bus transport | Runtime Bus transport module | Wire topic subscriptions and tick delivery |
| Cancellation | Runtime cancellation module | Use for loop shutdown |
| Metrics | Runtime metrics module | Track per-loop timing and cost |
| CorticalState | Not yet implemented | New struct in `roko-core` or `roko-runtime` |
| Regime detection | Implicit in probe logic | Formalize in adaptive clock |

The existing runtime support crate provides the infrastructure (process supervision, Bus transport, cancellation tokens) but does not implement the three-topic architecture. The adaptive clock would be a new component that uses these existing primitives.

---

## Academic Foundations

- **Buzsáki 2006** — "Rhythms of the Brain" (Oxford University Press). Oscillatory hierarchies: gamma rides on theta rides on delta.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Adaptive sampling based on prediction error.
- **Clark 2013** — "Whatever Next?" (Behavioral and Brain Sciences 36(3)). Nested prediction loops at multiple timescales.
- **Sims 2003** — "Implications of rational inattention" (Journal of Monetary Economics 50(3)). Cost of attention determines optimal sampling rate.
- **Koudahl et al. 2024** — (arXiv:2412.10425). Factorized discrete POMDP for tractable active inference state spaces.

---

## Current Status and Gaps

**What exists:**
- The runtime support crate provides process supervision, Bus transport, cancellation tokens, and metrics.
- The orchestration loop in `roko-cli/src/orchestrate.rs` provides a single-frequency gamma-like loop.
- The current tier enum and tier router in the primitives layer support T0/T1/T2 selection.

**What is missing (Implementation Plan 12a §I4):**
- **I4**: Frequency scheduler deciding which topic to emit based on context.
- `HeartbeatPolicy` struct managing three concurrent tick streams.
- Bus topic emission and subscription wiring.
- Regime-based interval adjustment.
- Budget-aware throttling.
- Early gamma emission for urgent conditions.
- Delta yielding and dream state serialization.
- CorticalState shared perception surface.

---

## Cross-References

- See [03-three-cognitive-speeds.md](./03-three-cognitive-speeds.md) for the three-speed model this clock manages
- See [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md) for the gamma loop
- See [05-theta-reflective-loop.md](./05-theta-reflective-loop.md) for the theta loop
- See [06-delta-consolidation-loop.md](./06-delta-consolidation-loop.md) for the delta loop
- See [09-16-t0-probes.md](./09-16-t0-probes.md) for regime detection via probes
- See `tmp/refinements/09-phase-2-implications.md` for the Phase 2+ Bus / Substrate framing
- See [Naming and Glossary](../00-architecture/01-naming-and-glossary.md) for the canonical Bus, Pulse, and Topic vocabulary
