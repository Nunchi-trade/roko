# 22. Heartbeat as Hot Graph

> The adaptive clock as three concurrent Hot Flow Graphs emitting tick Pulses on the Bus. Regime detection governs frequency modulation. Budget-aware throttling ensures graceful degradation under cost pressure.

See [03-GRAPH.md](../../unified/03-GRAPH.md) for Hot Graph semantics, [04-EXECUTION.md](../../unified/04-EXECUTION.md) for Engine interpretation, [05-AGENT.md](../../unified/05-AGENT.md) for Agent runtime.

---

## 1. The Clock is a Graph, Not a Special Scheduler

Most agent frameworks treat the execution clock as infrastructure -- a `tokio::interval` or cron job outside the agent's cognitive model. In Roko, the clock **is** a Graph. Specifically, it is three concurrent Hot Flow Graphs -- gamma, theta, delta -- each containing Cells that implement the universal protocols. This means:

1. The clock is observable via Lens Cells like any other subsystem.
2. The clock is tunable via the same predict-publish-correct Loop that governs all Cells.
3. The clock composes with other Graphs via standard Bus Pulses -- no special wiring.
4. The clock can be snapshot/resumed alongside the rest of the Agent's state.

The HeartbeatPolicy is a React Cell that governs emission timing. It reads CorticalState (regime, budget, behavioral state) and publishes tick Pulses at adaptive intervals. The three speed-specific consumers (gamma, theta, delta) subscribe to their respective topics and run the universal loop on each tick.

---

## 2. Three Hot Flow Graphs

### 2.1 Architecture

```
Agent Space
  |
  +-- heartbeat_policy: React Cell (publishes ticks)
  |     reads: CorticalState, BudgetTracker
  |     publishes:
  |       - heartbeat.gamma.tick
  |       - heartbeat.theta.tick
  |       - heartbeat.delta.tick
  |
  +-- gamma_flow: Hot Flow Graph
  |     subscribes: heartbeat.gamma.tick
  |     contains: 7-Cell Pipeline (SENSE->ASSESS->COMPOSE->ACT->VERIFY->PERSIST->REACT)
  |     period: 5-15s adaptive
  |     cost: ~80% $0.00 (T0), ~15% $0.001 (T1), ~5% $0.10 (T2)
  |
  +-- theta_flow: Hot Flow Graph
  |     subscribes: heartbeat.theta.tick
  |     contains: 5-Cell Pipeline (Summarize->Daimon->Calibrate->Reflect->Intervene)
  |     period: 30-120s adaptive (regime-modulated)
  |     cost: T1-T2 per tick
  |
  +-- delta_flow: Hot Flow Graph
        subscribes: heartbeat.delta.tick
        contains: 3-Cell Pipeline (NREM->REM->Integrate)
        period: triggered (idle>5min, episodes>=50, scheduled)
        cost: $0.07-0.33 per cycle
```

### 2.2 Graph TOML: Gamma Flow

```toml
[graph]
id = "gamma_flow"
kind = "hot_flow"
resident = true
subscribe_topic = "heartbeat.gamma.tick"

[[cells]]
id = "sense"
protocol = "Store"
description = "Run T0 probes, read Store, drain Bus topics"
inputs = ["tick_pulse"]
outputs = ["observation"]

[[cells]]
id = "assess"
protocol = "Score"
description = "Score candidates, compute prediction error, select tier"
inputs = ["observation"]
outputs = ["tier_decision"]

[[cells]]
id = "compose"
protocol = "Compose"
description = "Assemble context under budget (VCG auction)"
inputs = ["tier_decision"]
outputs = ["context_signal"]
condition = "tier_decision.tier != T0"

[[cells]]
id = "act"
protocol = "Connect"
description = "Invoke LLM or tool via InferenceHandle"
inputs = ["context_signal"]
outputs = ["action_result"]
condition = "tier_decision.tier != T0"

[[cells]]
id = "verify"
protocol = "Verify"
description = "Gate pipeline: compile, test, clippy, diff"
inputs = ["action_result"]
outputs = ["verdict"]

[[cells]]
id = "persist"
protocol = "Store"
description = "Store output Signal with lineage, publish Pulse"
inputs = ["verdict"]
outputs = ["persisted_hash"]

[[cells]]
id = "react"
protocol = "React"
description = "Episode logging, router feedback, Daimon update"
inputs = ["verdict", "persisted_hash"]
outputs = ["decision_cycle_record"]
```

### 2.3 Graph TOML: Theta Flow

```toml
[graph]
id = "theta_flow"
kind = "hot_flow"
resident = true
subscribe_topic = "heartbeat.theta.tick"

[[cells]]
id = "summarize"
protocol = "Score"
description = "Aggregate last N gamma DecisionCycleRecords into GammaSummary"
inputs = ["tick_pulse"]
outputs = ["gamma_summary"]

[[cells]]
id = "daimon_update"
protocol = "React"
description = "Update ALMA mood layer from aggregate outcomes"
inputs = ["gamma_summary"]
outputs = ["behavioral_state"]

[[cells]]
id = "calibrate"
protocol = "Verify"
description = "Check prediction accuracy, compute calibration report"
inputs = ["gamma_summary"]
outputs = ["calibration_report"]

[[cells]]
id = "reflect"
protocol = "Compose"
description = "Assemble theta context, ask LLM: am I on track?"
inputs = ["gamma_summary", "behavioral_state", "calibration_report"]
outputs = ["theta_reflection"]

[[cells]]
id = "intervene"
protocol = "React"
description = "Stuck detection, cost alerts, state transitions"
inputs = ["theta_reflection", "calibration_report"]
outputs = ["interventions"]
```

### 2.4 Graph TOML: Delta Flow

```toml
[graph]
id = "delta_flow"
kind = "hot_flow"
resident = true
subscribe_topic = "heartbeat.delta.tick"
interruptible = true
checkpoint_on_interrupt = true

[[cells]]
id = "nrem_replay"
protocol = "Score"
description = "Mattar-Daw utility-prioritized episode replay"
inputs = ["tick_pulse"]
outputs = ["nrem_output"]
model_tier = "T1"

[[cells]]
id = "rem_imagination"
protocol = "Compose"
description = "Boden creativity modes + Pearl counterfactuals"
inputs = ["nrem_output"]
outputs = ["rem_output"]
model_tier = "T1-T2"

[[cells]]
id = "integrate"
protocol = "Store"
description = "Staging buffer (0.20 confidence), tier promotion, playbook compilation"
inputs = ["nrem_output", "rem_output"]
outputs = ["consolidation_record"]
model_tier = "T0"
```

---

## 3. The HeartbeatPolicy React Cell

The HeartbeatPolicy is the single Cell responsible for deciding *when* to emit each tick Pulse. It does not execute cognition -- it only publishes timing signals.

```rust
/// HeartbeatPolicy: a React Cell that emits tick Pulses at adaptive intervals.
///
/// Lives at L0 Runtime. Reads CorticalState (regime, anomaly_count) and
/// BudgetTracker (daily_spend_fraction). Publishes three topic families.
///
/// Crate: `crates/roko-runtime/src/heartbeat.rs`
pub struct HeartbeatPolicy {
    config: ClockConfig,
    regime: Regime,
    budget_fraction: f64,
    gamma_violations: u32,
    gamma_count_since_theta: u32,
    episodes_since_delta: u32,
}

impl React for HeartbeatPolicy {
    fn topics(&self) -> Vec<&str> {
        vec![
            "cortical_state.regime_changed",
            "cortical_state.anomaly_detected",
            "budget.threshold_crossed",
            "episode.completed",
        ]
    }

    fn react(&mut self, pulse: &Pulse, bus: &Bus) {
        match pulse.topic.as_str() {
            "cortical_state.regime_changed" => {
                self.regime = pulse.payload::<Regime>();
            }
            "cortical_state.anomaly_detected" => {
                self.gamma_violations = pulse.payload::<u32>();
            }
            "budget.threshold_crossed" => {
                self.budget_fraction = pulse.payload::<f64>();
            }
            "episode.completed" => {
                self.episodes_since_delta += 1;
                // Theta fires on episode completion
                bus.publish(Pulse::new("heartbeat.theta.tick"));
                self.gamma_count_since_theta = 0;
            }
            _ => {}
        }
    }
}
```

### 3.1 Gamma Interval Formula

```
gamma_interval = base_max / (1 + violations * 0.3)
                 clamped to [gamma_min, gamma_max]

Default: 15s / (1 + N*0.3), floor 5s
```

| Violations | Interval | Ticks/Hour |
|---|---|---|
| 0 | 15.0s | 240 |
| 1 | 11.5s | 313 |
| 3 | 7.9s | 456 |
| 7+ | 5.0s | 720 |

### 3.2 Theta Interval Formula

```
theta_interval = base * regime_multiplier
                 clamped to [theta_min, theta_max]

Multipliers: Calm=1.6, Normal=1.0, Volatile=0.4, Crisis=0.2
```

| Regime | Multiplier | Interval | Ticks/Hour |
|---|---|---|---|
| Calm | 1.6x | 120s | 30 |
| Normal | 1.0x | 75s | 48 |
| Volatile | 0.4x | 30s | 120 |
| Crisis | 0.2x | 15s | 240 |

### 3.3 Delta Trigger Conditions

Delta does not tick periodically. It fires when:
1. Idle > 5 minutes (no active tasks)
2. episodes_since_delta >= 50
3. Scheduled time (configurable cron)
4. Explicit CLI command (`roko knowledge dream run`)

Delta is interruptible: if gamma work arrives, the delta flow checkpoints its state to Store and yields.

---

## 4. Regime Detection as a Score Cell

Regime detection is a Score Cell that reads probe anomaly counts and emits a regime classification on the Bus:

```rust
/// Regime detection: Score Cell that classifies environment volatility.
///
/// Reads the aggregate prediction error from T0 probes and maps it to
/// one of four regimes. Publishes regime changes as Pulses.
///
/// Implements predict-publish-correct: publishes predicted regime,
/// observes actual probe outcomes, corrects thresholds via EMA.
pub struct RegimeDetector {
    thresholds: [f32; 3], // calm/normal, normal/volatile, volatile/crisis
    ema_alpha: f32,
}

impl Score for RegimeDetector {
    fn score(&self, signal: &Signal) -> f64 {
        let error = signal.field::<f32>("aggregate_prediction_error");
        let regime = if error < self.thresholds[0] {
            Regime::Calm
        } else if error < self.thresholds[1] {
            Regime::Normal
        } else if error < self.thresholds[2] {
            Regime::Volatile
        } else {
            Regime::Crisis
        };
        regime as u8 as f64
    }
}
```

Default thresholds: `[0.1, 0.3, 0.6]`. Adjusted via EMA from actual probe distributions.

---

## 5. Budget-Aware Throttling

The HeartbeatPolicy integrates budget awareness as a Verify Cell that gates expensive tick emissions:

| Budget Fraction | Effect on Clock | Effect on Tiers |
|---|---|---|
| < 80% | Normal intervals | All tiers available |
| 80-90% | Theta interval 2x, gamma unchanged | T2 restricted to Crisis regime only |
| 90-95% | Theta interval 4x, gamma at max | T2 disabled, T1 restricted |
| >= 95% | Theta at max interval | T0 only -- probes continue, no LLM |

The invariant: **gamma T0 probes always run.** They cost $0.00. Even at 100% budget, the Agent maintains perception -- it can see what is happening, it just cannot deliberate about it.

```rust
/// Budget throttle: applied to HeartbeatPolicy emission decisions.
fn throttled_interval(base: Duration, budget_pct: f64) -> Duration {
    let multiplier = if budget_pct >= 0.95 {
        f64::MAX // effectively infinite -- topic not emitted
    } else if budget_pct >= 0.90 {
        4.0
    } else if budget_pct >= 0.80 {
        2.0
    } else {
        1.0
    };
    base.mul_f64(multiplier).min(Duration::from_secs(120))
}
```

---

## 6. TOML Configuration

```toml
[clock]
gamma_min_interval_secs = 5
gamma_max_interval_secs = 15
gamma_base_interval_secs = 10
theta_min_interval_secs = 15
theta_max_interval_secs = 120
theta_base_interval_secs = 75
theta_gamma_count = 5
delta_episode_threshold = 50
delta_idle_timeout_secs = 300
delta_scheduled_utc = "02:00"

[clock.budget]
daily_budget_usd = 50.0
throttle_at_percent = 80
hard_stop_at_percent = 95
```

---

## What This Enables

- **Autonomous perception at zero cost**: The 80% T0 suppression rate makes always-on cognition viable ($2-50/day vs $100-500/day without gating).
- **Adaptive sampling**: The Agent samples its environment more frequently when surprised (Friston 2010) and less when calm.
- **Independent failure domains**: Gamma crash does not kill theta; delta interruption does not corrupt gamma state.
- **Budget-constrained graceful degradation**: Running out of money dims the Agent progressively -- it never goes blind.
- **Composability**: Any subsystem can subscribe to tick topics. The orchestrator, conductor, dreams, and any future consumer all ride the same Bus emissions.

## Feedback Loops

1. **Probe anomaly count -> gamma interval** (Loop): More anomalies -> faster gamma -> more probes -> detect whether anomalies persist or resolve.
2. **Regime -> theta interval** (Loop): Volatile regime -> faster theta -> more reflection -> potentially downgrade regime if situation stabilizes.
3. **Budget spend -> throttle** (Loop): High spend -> slower ticks -> less spend -> budget recovers -> restore normal intervals.
4. **Predict-publish-correct on HeartbeatPolicy**: Publishes predicted next regime, Bus provides actual; policy adjusts thresholds via EMA.

## Open Questions

1. Should the HeartbeatPolicy be a single Cell or three independent Cells (one per frequency)?
2. What is the correct floor for gamma under Crisis+BudgetExhausted (currently: T0 probes at 15s max)?
3. Should the delta flow support partial resumption (resume from NREM if interrupted during REM)?
4. How should wakeup conditions (user intervention, safety alert) interact with budget throttling -- override or respect?

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Define `HeartbeatPolicy` struct + React impl | `crates/roko-runtime/src/heartbeat.rs` | Not started |
| Define `RegimeDetector` Score Cell | `crates/roko-runtime/src/regime.rs` | Not started |
| Wire gamma_flow Graph TOML | `crates/roko-cli/src/orchestrate.rs` | Partial (loop exists, not graph-structured) |
| Wire theta_flow Graph TOML | `crates/roko-cli/src/orchestrate.rs` | Not started |
| Wire delta_flow Graph TOML | `crates/roko-dreams/src/lib.rs` | Partial (scaffold exists) |
| Budget throttle integration | `crates/roko-runtime/src/heartbeat.rs` | Not started |
| Clock config in roko.toml | `crates/roko-core/src/config.rs` | Partial (heartbeat section exists) |
| CorticalState shared struct | `crates/roko-core/src/cortical.rs` | Not started |
