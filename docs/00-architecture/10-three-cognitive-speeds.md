# Three Cognitive Speeds

> **Abstract:** Roko agents operate at three timescales concurrently: Gamma (reactive,
> ~5-15s), Theta (reflective, ~75s), and Delta (consolidation, hours). These three speeds
> are inspired by neural oscillation bands in neuroscience (Buzsáki 2006) and map to the
> dual-process cognition model. The adaptive clock in `roko-runtime` manages all three on
> separate async tasks, with the Daimon's affect state modulating the cadence. This document
> specifies each speed, the adaptive clock scheduler, and the operating frequency selection
> logic.

---

## 1. The Three Speeds

| Speed | Period | Name | What Happens | Inference Tier |
|---|---|---|---|---|
| **Gamma** | ~5-15s | Reactive | One complete loop tick. Tool calls, LLM inference, verification. | T0 or T1 |
| **Theta** | ~75s | Reflection | Summarize recent work. Update Daimon state. Check predictions. Re-plan if needed. | T1 or T2 |
| **Delta** | Hours | Consolidation | Dreams: replay, synthesis, pruning. Knowledge tier promotion. | T2 |

All three run concurrently on separate async tasks, managed by the adaptive clock.

### 1.1 Gamma — Reactive

Gamma is the agent's heartbeat. Every ~5-15 seconds, one complete cognitive loop tick
executes: perceive the environment, select relevant information, compose a prompt, act
(call an LLM or tool), verify the output, and persist the result.

Most Gamma ticks are **T0 (no LLM call)**: the 16 T0 probes (see
[17-design-principles-and-frontier-summary.md](17-design-principles-and-frontier-summary.md))
check for changes in the environment. If nothing surprising is detected, the tick completes
without invoking any model — the agent "coasts" on its existing heuristics.

When a T0 probe detects a prediction error above threshold, the tick escalates to T1 (fast
model) or T2 (full model) depending on the magnitude of surprise.

### 1.2 Theta — Reflection

Every ~75 seconds (adjustable), the agent pauses reactive work to reflect:

- **Summarize**: What has happened since the last Theta tick?
- **Update Daimon**: Recalculate PAD vector (Pleasure-Arousal-Dominance) from recent outcomes
- **Check predictions**: Which predictions have resolved? Update calibration.
- **Re-plan**: Is the current approach working? Should the agent switch strategies?
- **Knowledge update**: Promote useful Working-tier knowledge to Consolidated.

Theta ticks typically invoke T1 (fast model) for summarization or T2 (full model) for
re-planning.

### 1.3 Delta — Consolidation

During extended idle periods or on a scheduled basis (every few hours), the agent enters
Delta mode for deep consolidation:

- **Dreams NREM replay**: Replay recent episodes, weighted by prediction error magnitude
  (Mattar & Daw 2018, Nature Neuroscience 21)
- **Dreams REM imagination**: Generate novel hypotheses via HDC recombination
  (Boden 2004, The Creative Mind)
- **Knowledge promotion**: Promote Consolidated-tier knowledge to Persistent
- **Pruning**: Remove Engrams that have decayed below threshold
- **Synthesis**: Extract cross-episode patterns into new playbook rules

Delta ticks use T2 (full model) for deep reasoning and synthesis.

---

## 2. The Operating Frequency Enum

From `roko-core/src/operating_frequency.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingFrequency {
    /// Reactive mode: perceive, retrieve, act.
    Gamma,
    /// Strategic mode: re-plan, update goals, evaluate progress.
    Theta,
    /// Consolidation mode: replay, distill, meta-cognate.
    Delta,
}
```

### 2.1 Mapping to Inference Tiers

```rust
impl OperatingFrequency {
    pub const fn inference_tier(self) -> InferenceTier {
        match self {
            Self::Gamma => InferenceTier::T0,
            Self::Theta => InferenceTier::T1,
            Self::Delta => InferenceTier::T2,
        }
    }
}
```

This mapping provides the default tier for each frequency. Gamma defaults to T0 (no LLM),
but can escalate. Theta defaults to T1 (fast model). Delta defaults to T2 (full model).

### 2.2 Turn Limits

```rust
impl OperatingFrequency {
    pub const fn turn_limit(self) -> u32 {
        match self {
            Self::Gamma => 0,   // Gamma does not dispatch an agent
            Self::Theta => 20,  // Theta gets a 20-turn budget
            Self::Delta => 50,  // Delta gets a 50-turn budget
        }
    }
}
```

---

## 3. Frequency Selection Logic

The operating frequency for a task is selected based on two inputs: the task's
characteristics and the agent's current affect state (Daimon).

```rust
impl OperatingFrequency {
    pub fn select(task: &Task, affect: &impl OperatingFrequencyAffect) -> Self {
        if is_reactive_task(task) {
            return Self::Gamma;
        }
        if is_reflective_task(task) {
            return Self::Delta;
        }
        if affect_suggests_reflection(affect) && task.is_substantial() {
            return Self::Delta;
        }
        Self::Theta
    }
}
```

### 3.1 Reactive Tasks → Gamma

Quick fixes, gate re-checks, permission checks, and filter evaluations are routed to Gamma.
These are identified by task tags (`quick_fix`) or text matching against the task title:

```rust
fn is_reactive_task(task: &Task) -> bool {
    task_tag_matches(task, "quick_fix")
        || task_text_matches(task, &[
            "quick fix", "gate re-check", "permission check",
            "subscription filter", "filter evaluation",
        ])
}
```

### 3.2 Reflective Tasks → Delta

Dream cycles, plan regeneration, retrospectives, and meta-cognition tasks route to Delta:

```rust
fn is_reflective_task(task: &Task) -> bool {
    task_text_matches(task, &[
        "dream", "plan regeneration", "retrospective",
        "meta-cognition", "consolidation",
    ])
}
```

### 3.3 Affect-Driven Escalation

When the Daimon reports low confidence, high arousal, and low dominance (indicating the agent
is struggling), substantial tasks are promoted from Theta to Delta — giving the agent more
time and deeper reasoning to step back and reconsider:

```rust
fn affect_suggests_reflection(affect: &impl OperatingFrequencyAffect) -> bool {
    affect.confidence() < 0.3
        && (affect.arousal() > 0.25 || affect.dominance() < -0.1)
}
```

---

## 4. The Adaptive Clock Scheduler

The `OperatingFrequencyScheduler` manages the cadence of Theta and Delta ticks:

```rust
pub struct OperatingFrequencyScheduler {
    theta_interval: Duration,  // default: 3 minutes
    delta_interval: Duration,  // default: 30 minutes
}
```

### 4.1 Selection Logic

```rust
impl OperatingFrequencyScheduler {
    pub fn select(&self, context: &OperatingFrequencyScheduleContext) -> OperatingFrequency {
        // Idle systems consolidate
        if context.is_idle() {
            return OperatingFrequency::Delta;
        }
        // Long time since last theta → consolidate
        if context.time_since_last_theta >= self.delta_interval {
            return OperatingFrequency::Delta;
        }
        // Theta due (possibly shortened by anxiety/stalling)
        let theta_due = self.theta_interval_for(context);
        if context.time_since_last_theta >= theta_due {
            OperatingFrequency::Theta
        } else {
            OperatingFrequency::Gamma
        }
    }
}
```

### 4.2 Adaptive Cadence

The Theta interval shortens when the agent is struggling:

- **Stalling** (completion rate ≤ 0.25): Theta interval × 0.5 — reflect sooner
- **Anxious** (low confidence + high arousal + low dominance): Theta interval × 0.66

This ensures that struggling agents reflect more frequently, while productive agents
maintain their Gamma cadence without interruption.

### 4.3 Schedule Context

```rust
pub struct OperatingFrequencyScheduleContext {
    pub time_since_last_theta: Duration,
    pub active_tasks: usize,
    pub completion_rate: f64,   // [0.0, 1.0]
    pub confidence: f64,        // [0.0, 1.0]
    pub arousal: f64,           // [-1.0, 1.0]
    pub dominance: f64,         // [-1.0, 1.0]
}
```

The confidence, arousal, and dominance fields come from the Daimon's PAD vector, creating
a feedback loop: the agent's emotional state modulates how often it reflects.

---

## 5. Neuroscience Inspiration

The three speeds are named after neural oscillation bands (Buzsáki 2006, Rhythms of the
Brain, Oxford University Press):

| Brain Rhythm | Frequency | Function | Roko Mapping |
|---|---|---|---|
| **Gamma** (30-100 Hz) | Fast | Sensory processing, attention binding | Reactive: perceive and act |
| **Theta** (4-8 Hz) | Medium | Working memory, navigation, planning | Reflective: re-plan and evaluate |
| **Delta** (0.5-4 Hz) | Slow | Deep sleep, memory consolidation | Consolidation: Dreams and synthesis |

The mapping is conceptual, not literal — Roko does not operate at these frequencies. The
names capture the functional role: Gamma for fast reaction, Theta for deliberate planning,
Delta for deep consolidation.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Buzsáki 2006, Rhythms of the Brain, OUP | Neural oscillation bands: Gamma/Theta/Delta as functional timescales. Naming inspiration. |
| Mattar & Daw 2018, Nature Neuroscience 21 | Prioritized memory replay during consolidation (NREM). Foundation for Delta Dreams. |
| Baddeley 2000, Trends in Cognitive Sciences 4(11) | Working memory model: central executive manages attention allocation. Maps to Theta reflection. |
| Friston 2010, Nature Reviews Neuroscience 11(2) | Active inference: prediction error drives attentional selection. Gamma ticks as prediction-error checking. |

---

## Current Status and Gaps

- **Implemented**: `OperatingFrequency` enum, `OperatingFrequencyScheduler`,
  `OperatingFrequencyScheduleContext`, affect-driven frequency selection. All tested in
  `roko-core` (12+ tests).
- **Wired**: Operating frequency selection integrated into orchestrate.rs via task dispatch.
- **Gap**: The three frequencies do not yet run as truly concurrent async tasks. Currently,
  the orchestrator drives frequency selection per-task rather than running three independent
  loops.

---

## Cross-References

- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) — The loop that runs at each speed
- [11-dual-process-and-active-inference.md](11-dual-process-and-active-inference.md) — T0/T1/T2 tier routing
- [13-cognitive-cross-cuts.md](13-cognitive-cross-cuts.md) — Daimon PAD vector drives cadence
