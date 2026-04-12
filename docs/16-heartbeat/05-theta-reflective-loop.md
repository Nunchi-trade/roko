# Theta: The Reflective Loop (~75s)

> The medium cognitive frequency — periodic reflection, plan re-evaluation, and calibration checking.

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [03-three-cognitive-speeds.md](./03-three-cognitive-speeds.md), [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md)
**Key sources**: `refactoring-prd/02-five-layers.md` §Adaptive Clock, legacy `bardo-backup/prd/01-golem/18-cortical-state.md` §Three Concurrent Scales, `implementation-plans/12a-cognitive-layer.md` §I2

---

## Abstract

Theta is the breath. Every ~75 seconds (adapting between 30-120 seconds based on environmental regime), the agent pauses its reactive gamma processing and asks: *"Am I on the right track?"* Theta is not about reacting to individual observations — that is gamma's job. Theta is about reflecting on patterns across recent gamma ticks, checking prediction calibration, updating the Daimon's behavioral state, and re-evaluating the current plan.

The name comes from EEG theta oscillations (4-8 Hz), which Buzsáki (2006, "Rhythms of the Brain", Oxford University Press) associates with navigation, memory encoding, and the hippocampal "indexing" of episodic memories. In the brain, theta oscillations coordinate the binding of fast gamma events into coherent episodes. Roko's theta loop serves the same function: it integrates the fast gamma ticks into a coherent narrative of what the agent has been doing and whether it is making progress.

Theta always invokes at least T1 — it needs LLM reasoning to reflect meaningfully. Most theta cycles use T1 ($0.005-0.01), with complex situations escalating to T2 ($0.03-0.10). At the default cadence of 1 theta tick per ~75 seconds, this amounts to ~1,152 theta ticks per day, costing $6-12/day at T1 or $35-115/day if all T2. The typical mix (80% T1, 20% T2) yields $10-35/day for theta alone.

This document specifies the theta loop in detail: its five-phase internal structure, adaptive interval computation, triggers, and relationship to the Daimon affect engine.

---

## Theta Trigger Conditions

Theta fires based on two conditions, whichever comes first:

1. **Gamma count**: Every N=5 gamma cycles, theta fires. At a typical gamma interval of 10 seconds, this means theta fires every ~50 seconds.
2. **Episode completion**: When a logical unit of work completes (a task finishes, a gate verdict is rendered, a plan step succeeds or fails), theta fires immediately regardless of the gamma count.

The gamma-count trigger ensures regular reflection even during periods of continuous reactive processing. The episode-completion trigger ensures that significant events are reflected upon promptly rather than waiting for the next scheduled theta cycle.

```rust
/// Determine whether to fire a theta tick.
///
/// Theta fires when either:
/// 1. N=5 gamma ticks have elapsed since last theta, OR
/// 2. An episode has completed since last theta.
fn should_fire_theta(
    gamma_since_last_theta: u32,
    episode_completed: bool,
    theta_config: &ThetaConfig,
) -> bool {
    gamma_since_last_theta >= theta_config.gamma_count_trigger  // default: 5
        || episode_completed
}
```

---

## The Five Phases of Theta

Each theta tick runs five phases. These are not CoALA steps — they are theta-specific reflection activities that operate at a higher abstraction level than individual gamma ticks.

### Phase 1: Summarize Recent Gamma Work

Theta begins by reviewing what happened during the last N gamma ticks (since the previous theta). This phase reads the DecisionCycleRecords produced by recent gamma ticks and extracts patterns:

- **Outcome distribution**: How many ticks were T0/T1/T2? What was the success/failure ratio?
- **Anomaly patterns**: Are the same probes firing repeatedly? Is there a systematic issue?
- **Action patterns**: What actions were taken? Were they effective?
- **Cost accumulation**: How much has been spent since the last theta tick?

```rust
/// Phase 1: Summarize recent gamma work.
///
/// Reads the last N DecisionCycleRecords and extracts
/// patterns for theta-level reflection.
fn summarize_gamma_history(
    records: &[DecisionCycleRecord],
) -> GammaSummary {
    let tier_counts = records.iter()
        .fold([0u32; 3], |mut acc, r| {
            acc[r.tier as usize] += 1;
            acc
        });

    let success_rate = records.iter()
        .filter(|r| r.outcome.as_ref().map_or(false, |o| o.passed))
        .count() as f32 / records.len().max(1) as f32;

    let total_cost: f64 = records.iter().map(|r| r.total_cost).sum();

    let recurring_anomalies = find_recurring_anomalies(records);

    GammaSummary {
        tick_count: records.len() as u32,
        tier_distribution: tier_counts,
        success_rate,
        total_cost,
        recurring_anomalies,
        action_count: records.iter()
            .map(|r| r.actions.len())
            .sum(),
    }
}
```

The summary is assembled into a structured Engram that becomes part of the theta context. This allows the LLM in Phase 4 (re-evaluate plan) to reason about aggregate patterns rather than individual ticks.

### Phase 2: Update Daimon State

Theta computes the **aggregate affect** from recent outcomes. While gamma updates affect per-tick (via the Daimon's `update_pad_from_resolution()` call at the META-COGNIZE step), theta performs a broader assessment of the agent's emotional trajectory.

The Daimon uses the ALMA (Adaptive Layered Model of Affect) three-layer architecture:

| Layer | Timescale | What It Represents | Decay Rate |
|---|---|---|---|
| **Emotion** | Seconds | Immediate reaction to individual events | Fast (α = 0.20) |
| **Mood** | Hours | Sustained state from accumulated outcomes | Slow (α = 0.02, 4h half-life) |
| **Personality** | Permanent | Baseline disposition from configuration | None |

Theta's affect update operates at the **mood** layer. It reads all emotion-layer updates from recent gamma ticks and computes the mood drift:

```rust
/// Phase 2: Update Daimon affect at the mood timescale.
///
/// The ALMA model (Gebhard 2005) separates affect into three layers:
/// Emotion (fast), Mood (medium), Personality (fixed). Theta operates
/// on the Mood layer, integrating emotion-layer impulses from gamma.
fn update_daimon_mood(
    daimon: &mut DaimonState,
    gamma_summary: &GammaSummary,
) {
    // Mood-level pleasure: sustained success → positive mood
    let mood_pleasure_delta = (gamma_summary.success_rate - 0.5) * 0.05;

    // Mood-level arousal: high T2 rate → sustained high arousal
    let t2_ratio = gamma_summary.tier_distribution[2] as f32
        / gamma_summary.tick_count.max(1) as f32;
    let mood_arousal_delta = (t2_ratio - 0.05) * 0.1;  // baseline: 5% T2

    // Mood-level dominance: improving over time → high dominance
    let mood_dominance_delta = if gamma_summary.success_rate > 0.7 {
        0.02  // succeeding → feel in control
    } else if gamma_summary.success_rate < 0.3 {
        -0.03  // failing → feel out of control (asymmetric, 1.5× negativity)
    } else {
        0.0
    };

    // EMA update on mood layer (slow: α = 0.02)
    daimon.mood.pleasure = ema(daimon.mood.pleasure, mood_pleasure_delta, 0.02);
    daimon.mood.arousal = ema(daimon.mood.arousal, mood_arousal_delta, 0.02);
    daimon.mood.dominance = ema(daimon.mood.dominance, mood_dominance_delta, 0.02);
}
```

The six behavioral states derived from the PAD vector are:

| State | PAD Region | Behavioral Effect |
|---|---|---|
| **Engaged** | P > 0.2, A > 0.1, D > 0.1 | Standard operation. Balanced exploration/exploitation. |
| **Struggling** | P < -0.2, A > 0.3 | More caution. Prefer validated strategies. Lower risk tolerance. |
| **Coasting** | P > 0.1, A < -0.1, D > 0.2 | May miss opportunities. Theta should flag complacency. |
| **Exploring** | P ~ 0, A > 0.2, D < 0 | Actively searching. Higher T2 rate acceptable. |
| **Focused** | P > 0, A > 0.3, D > 0.3 | Deep work. Minimize distractions. Extend gamma intervals. |
| **Resting** | A < -0.2 | Pre-delta state. Ready for dream consolidation. |

Theta writes the updated behavioral state to the CorticalState for other subsystems to read.

### Phase 3: Check Predictions

Theta performs a calibration check — how accurate were the agent's predictions over the last theta cycle?

Every gamma tick may produce predictions (registered as `PredictionClaim` Engrams before execution, resolved as `PredictionOutcome` Engrams after verification). Theta aggregates these:

```rust
/// Phase 3: Calibration check.
///
/// Aggregates prediction residuals from recent gamma ticks.
/// Computes aggregate accuracy and trend direction.
///
/// This implements Predictive Foraging (see topic 05-learning):
/// every knowledge retrieval is a falsifiable prediction.
/// Residuals feed the CalibrationTracker (~50ns corrections).
fn check_predictions(
    recent_predictions: &[PredictionOutcome],
    calibration: &mut CalibrationTracker,
) -> CalibrationReport {
    let total = recent_predictions.len();
    let correct = recent_predictions.iter()
        .filter(|p| p.correct)
        .count();
    let accuracy = correct as f32 / total.max(1) as f32;

    // Update per-(model, category) bias corrections
    for pred in recent_predictions {
        calibration.update(
            &pred.model_id,
            &pred.category,
            pred.residual,
        );
    }

    // Compute trend: improving, stable, or declining
    let trend = calibration.compute_trend();

    CalibrationReport {
        predictions_checked: total,
        accuracy,
        trend,
        largest_miss: recent_predictions.iter()
            .max_by(|a, b| a.residual.abs().partial_cmp(&b.residual.abs()).unwrap())
            .cloned(),
    }
}
```

The CalibrationTracker aggregates residuals per (model, task_category) pair and computes an `adjusted_prediction = raw_prediction - mean_bias(model, category)`. This arithmetic correction costs ~50 nanoseconds — no LLM needed. Over time, the agent self-corrects its prediction biases.

### Phase 4: Re-Evaluate Plan

This is the core of theta: the "step back and think about the plan" moment. The agent assembles a theta-specific context including:

- The gamma summary (Phase 1)
- The updated affect state (Phase 2)
- The calibration report (Phase 3)
- The current plan and progress
- Recent outcomes (successes and failures)

The LLM (at least T1, possibly T2 for complex situations) then reasons about whether the current plan is still the best approach:

**Questions theta asks:**
- Is the current plan still valid given what I've observed?
- Am I making progress or spinning my wheels?
- Should I re-prioritize tasks?
- Are there patterns in my failures that suggest a different approach?
- Am I spending too much (high T2 rate) or too little (missing important signals)?

```rust
/// Phase 4: Re-evaluate the current plan.
///
/// This is where theta does its main work: assembling a reflection
/// context and asking the LLM whether the current approach is sound.
async fn reevaluate_plan(
    gamma_summary: &GammaSummary,
    calibration: &CalibrationReport,
    daimon_state: &DaimonState,
    current_plan: &Plan,
    composer: &dyn Composer,
    agent: &dyn Agent,
) -> Result<ThetaReflection> {
    // Assemble theta-specific context
    let context = composer.compose(
        &ThetaContextRequest {
            gamma_summary: gamma_summary.clone(),
            calibration: calibration.clone(),
            affect_state: daimon_state.effective_pad(),
            behavioral_state: daimon_state.behavioral_state(),
            plan_progress: current_plan.progress_summary(),
            recent_failures: current_plan.recent_failures(5),
        },
        &ContextBudget::theta(),  // ~8,000 tokens for T1, ~16,000 for T2
    )?;

    // Determine tier for this theta tick
    let tier = if calibration.trend == Trend::Declining
        || daimon_state.behavioral_state() == BehavioralState::Struggling
        || gamma_summary.success_rate < 0.3
    {
        InferenceTier::T2  // Complex situation → full model
    } else {
        InferenceTier::T1  // Normal reflection → fast model
    };

    // Ask the LLM to reflect
    let reflection = agent.execute(
        &context,
        tier,
        &ThetaReflectionPrompt,
    ).await?;

    Ok(reflection)
}
```

The output is a `ThetaReflection` Engram containing:
- Whether the plan should continue, be modified, or be abandoned
- Any suggested re-prioritizations
- Identified patterns in recent failures
- Recommended adjustments to gamma behavior (e.g., "increase T2 rate for the next cycle")

### Phase 5: Trigger Interventions

If theta detects problematic patterns, it triggers interventions:

- **Stuck detection**: If the agent has retried the same task >3 times without progress, theta suggests escalation (stronger model, different approach, or human review).
- **Cost anomaly**: If the T2 rate exceeds 20% (double the expected 5%), theta tightens the threshold.
- **Calibration collapse**: If prediction accuracy drops below 40%, theta triggers a "Struggling" state transition and requests more conservative behavior.
- **Complacency detection**: If the agent is in "Coasting" state with declining accuracy, theta flags it — coasting feels good but may mask degrading performance.

```rust
/// Phase 5: Check for conditions requiring intervention.
fn check_interventions(
    gamma_summary: &GammaSummary,
    calibration: &CalibrationReport,
    daimon_state: &DaimonState,
    stuck_counter: &HashMap<TaskId, u32>,
) -> Vec<Intervention> {
    let mut interventions = Vec::new();

    // Stuck detection: >3 retries on same task
    for (task_id, retries) in stuck_counter {
        if *retries > 3 {
            interventions.push(Intervention::escalation(
                *task_id,
                format!("Task retried {} times without progress", retries),
            ));
        }
    }

    // Cost anomaly: T2 rate > 20%
    let t2_rate = gamma_summary.tier_distribution[2] as f32
        / gamma_summary.tick_count.max(1) as f32;
    if t2_rate > 0.20 {
        interventions.push(Intervention::cost_alert(
            format!("T2 rate {:.0}% exceeds 20% threshold", t2_rate * 100.0),
        ));
    }

    // Calibration collapse: accuracy < 40%
    if calibration.accuracy < 0.40 {
        interventions.push(Intervention::state_transition(
            BehavioralState::Struggling,
            "Prediction accuracy below 40%".to_string(),
        ));
    }

    // Complacency: Coasting + declining accuracy
    if daimon_state.behavioral_state() == BehavioralState::Coasting
        && calibration.trend == Trend::Declining
    {
        interventions.push(Intervention::complacency_alert(
            "Accuracy declining while in Coasting state".to_string(),
        ));
    }

    interventions
}
```

---

## Adaptive Theta Interval

Theta's interval adapts based on domain regime multipliers:

| Domain Regime | Theta Interval | Multiplier | Rationale |
|---|---|---|---|
| Calm / stable | ~120s | 1.6× | Little changes; less frequent reflection needed |
| Normal | ~75s | 1.0× (baseline) | Standard reflection cadence |
| Volatile / troubled | ~30s | 0.4× | Rapid changes require frequent re-evaluation |
| Crisis | ~15s | 0.2× | Near-continuous reflection during critical events |

```rust
/// Compute the theta interval based on the current regime.
///
/// During volatile periods, theta accelerates to provide more
/// frequent reflection. During calm periods, theta stretches
/// to reduce cost.
fn compute_theta_interval(regime: Regime, base: Duration) -> Duration {
    let multiplier = match regime {
        Regime::Calm => 1.6,
        Regime::Normal => 1.0,
        Regime::Volatile => 0.4,
        Regime::Crisis => 0.2,
    };
    Duration::from_secs_f64(base.as_secs_f64() * multiplier)
        .max(Duration::from_secs(15))
        .min(Duration::from_secs(120))
}
```

The base interval (75 seconds) is configurable in `roko.toml`. The regime multiplier adjusts it within bounds. The runtime tracks daily cost and can increase the multiplier when approaching budget limits, effectively trading reflection frequency for cost savings.

---

## Theta's Role in the Hierarchy

Theta sits between gamma (fast, reactive) and delta (slow, consolidation). Its role is to **bridge individual ticks and long-term learning**:

- **Upward flow (gamma → theta)**: Theta summarizes gamma ticks into patterns. The gamma summary becomes an episode-level Engram that delta will eventually process during dream consolidation.
- **Downward flow (theta → gamma)**: Theta adjustments change gamma behavior. A theta reflection that recommends "tighten the T2 threshold" immediately affects subsequent gamma ticks. A theta state transition from "Engaged" to "Struggling" modulates gamma's scoring weights.

```
Gamma ticks:  [T0] [T0] [T1] [T0] [T0]
                    ↓↓↓↓↓ aggregate
Theta:            [REFLECT]
                    ↓ adjustments
Gamma ticks:  [T0] [T0] [T0] [T0] [T1]  ← threshold adjusted
```

Theta is also responsible for incrementing the **sleep pressure** counter. Each theta tick that observes unprocessed episodes (episodes that have not yet been replayed in a dream cycle) increases sleep pressure. When sleep pressure exceeds the threshold (~50 theta cycles), delta fires.

---

## Academic Foundations

- **Buzsáki 2006** — "Rhythms of the Brain" (Oxford University Press). Theta oscillations (4-8 Hz): navigation, memory encoding, hippocampal indexing.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Hierarchical prediction at multiple temporal grains.
- **Clark 2013** — "Whatever Next?" (Behavioral and Brain Sciences 36(3)). Predictive processing at multiple timescales.
- **Gebhard 2005** — "ALMA: A Layered Model of Affect" (AAMAS 2005). Three-layer affect model: emotion, mood, personality.
- **Barrett 2017** — "How Emotions Are Made" (Houghton Mifflin). Constructed emotion from prediction residuals.
- **Mattar & Daw 2018** — "Prioritized memory access" (Nature Neuroscience 21). Sleep pressure and replay priority.
- **Scherer 2001** — "Appraisal considered as a process of multilevel sequential checking" (Oxford University Press). Multi-axis appraisal theory underlying Score computation.

---

## Current Status and Gaps

**What exists:**
- Episode logging per task in `.roko/episodes.jsonl` provides raw material for theta summarization.
- Efficiency events per turn in `.roko/learn/efficiency.jsonl` capture cost data.
- `CascadeRouter` persistence to `.roko/learn/cascade-router.json` captures model routing state.
- Adaptive gate thresholds (EMA per rung) in `.roko/learn/gate-thresholds.json`.

**What is missing (Implementation Plan 12a §I2):**
- **I2**: Theta loop with periodic "step back and think" reflection.
- Theta-specific context assembly (gamma summary + calibration + affect).
- ALMA three-layer affect model in `roko-daimon`.
- Behavioral state machine (Engaged / Struggling / Coasting / Exploring / Focused / Resting).
- Sleep pressure accumulation toward delta threshold.
- Stuck detection and intervention system.
- CalibrationTracker with per-(model, category) bias correction.
- Adaptive theta interval based on regime multipliers.

---

## Cross-References

- See [03-three-cognitive-speeds.md](./03-three-cognitive-speeds.md) for the three-speed hierarchy
- See [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md) for the fast loop theta summarizes
- See [06-delta-consolidation-loop.md](./06-delta-consolidation-loop.md) for the slow loop theta feeds
- See [07-adaptive-clock.md](./07-adaptive-clock.md) for the clock managing all three timescales
- See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for the tier gating theta adjusts
- See topic [09-daimon](../09-daimon/INDEX.md) for the ALMA affect model and PAD vectors
- See topic [05-learning](../05-learning/INDEX.md) for calibration tracking and episode logging
