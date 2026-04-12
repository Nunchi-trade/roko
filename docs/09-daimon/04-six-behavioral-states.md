# Six Behavioral States

> How PAD octants map to discrete behavioral states that modulate compute allocation, model routing, exploration budgets, and retry policies — cyclical, never terminal.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [01-pad-vector.md](./01-pad-vector.md), [03-occ-scherer-appraisal.md](./03-occ-scherer-appraisal.md)
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §2, `roko-golem/src/daimon.rs`, `roko-daimon/src/lib.rs`

---

## Abstract

The PAD vector is a continuous three-dimensional signal. But agents need discrete decisions: which model tier to use, how aggressively to explore, whether to escalate or conserve. The six behavioral states bridge the continuous affect space and the discrete decision space. Each state is a named region of PAD space with a specific behavioral profile — a set of parameters that modulate tier routing, exploration rate, retry limits, and proactive maintenance.

The critical design constraint is **cyclicality**. There is no terminal state. An agent in the Struggling state will eventually recover through successful task outcomes (pleasure increases) or through dream depotentiation (arousal decreases). An agent in the Coasting state will eventually encounter a harder problem (pleasure decreases, arousal increases). The state machine is a loop, not a directed graph with a sink node.

This is a deliberate departure from the legacy specification, which defined five mortality-driven phases (Thriving → Stable → Conservation → Declining → Terminal) where Terminal was an absorbing state. The new behavioral states track **cognitive performance**, not **existential countdown**. An agent doesn't "approach death" — it encounters harder problems, runs low on budget, or accumulates failures. These are recoverable conditions.

---

## The Six States

### State Definitions

| State | PAD Profile | Description |
|---|---|---|
| **Engaged** | Balanced (near origin) | Normal operation — the agent is making progress at a sustainable rate |
| **Struggling** | Low P, High A | The agent is failing and under pressure — gate failures, blocked tasks, approaching deadlines |
| **Coasting** | High P, Low A | The agent is succeeding without difficulty — routine tasks, clean passes, familiar territory |
| **Exploring** | Low D | The agent is in unfamiliar territory — low confidence regardless of success/failure signals |
| **Focused** | High D, High P | The agent is succeeding in well-understood territory — exploit mode |
| **Resting** | Low A, Low D | The agent is idle or in a low-demand phase — time for offline learning and consolidation |

### PAD Thresholds for State Classification

The behavioral state is computed from the PAD vector and confidence score. The thresholds are derived from the `AffectEngine::modulate()` implementation in `roko-daimon/src/lib.rs`:

```rust
fn classify_behavioral_state(state: &AffectState) -> BehavioralState {
    let p = state.pad.pleasure;
    let a = state.pad.arousal;
    let d = state.pad.dominance;
    let c = state.confidence;

    // Struggling: clearly failing under pressure
    if c < 0.30 || d < -0.25 {
        return BehavioralState::Struggling;
    }

    // Coasting: succeeding without effort
    if p > 0.35 && c > 0.65 {
        return BehavioralState::Coasting;
    }

    // Focused: high confidence, high pleasure — exploit mode
    if d > 0.30 && p > 0.25 {
        return BehavioralState::Focused;
    }

    // Resting: low urgency, low confidence — maintenance mode
    if a < -0.20 {
        return BehavioralState::Resting;
    }

    // Exploring: low dominance but not failing
    if d < 0.10 && p > -0.20 {
        return BehavioralState::Exploring;
    }

    // Default: Engaged
    BehavioralState::Engaged
}
```

These thresholds are not arbitrary — they emerge from the appraisal rule magnitudes. A single task failure produces `P: -0.20, D: -0.15, C: -0.15`. Two consecutive failures push confidence to `0.70 → 0.40`, which is still above the Struggling threshold (0.30). Three consecutive failures push confidence below 0.30, triggering Struggling. This means the agent tolerates occasional setbacks (Engaged) but escalates after sustained failure (Struggling).

---

## Behavioral Modulation Parameters

Each behavioral state maps to a concrete set of parameters that modulate agent behavior. The `AffectBehaviorModulation` struct in `roko-golem/src/daimon.rs` defines these:

### Engaged (Default)

```rust
AffectBehaviorModulation {
    strategy: Balanced,
    exploration_rate: 0.20,
    prefer_proven_playbooks: true,
    model_tier_escalation: 0,
    extra_retries: 0,
    trigger_dream_cycles: false,
    run_maintenance_tasks: false,
}
```

**Rationale**: The baseline state. 20% exploration rate means one in five strategy choices tries something new. Prefers proven playbooks but doesn't lock to them. No model escalation — use the tier router's default allocation. No extra retries — if a task fails at the standard retry limit, it fails.

### Struggling (Conservative / Escalating)

The Struggling state has two sub-profiles depending on the specific PAD signature:

**Low confidence (C < 0.30) or very low dominance (D < -0.25)**:

```rust
// From roko-daimon modulate():
params.strategy = DispatchStrategy::Escalating;
params.turn_limit = params.turn_limit.saturating_add(10);
params.model = promote_model(&params.model);  // haiku → sonnet → opus
```

**Low pleasure with high arousal (P < -0.30, A > 0.30)**:

```rust
// From roko-daimon modulate():
params.strategy = DispatchStrategy::Conservative;
params.turn_limit = params.turn_limit.saturating_sub(3);
params.model = demote_model(&params.model);  // opus → sonnet → haiku
```

**From roko-golem (Anxious octant → Conservative)**:

```rust
AffectBehaviorModulation {
    strategy: Conservative,
    exploration_rate: 0.05,
    prefer_proven_playbooks: true,
    model_tier_escalation: 0,
    extra_retries: 0,
    trigger_dream_cycles: false,
    run_maintenance_tasks: false,
}
```

**From roko-golem (Angry octant → Escalating)**:

```rust
AffectBehaviorModulation {
    strategy: Escalating,
    exploration_rate: 0.10,
    prefer_proven_playbooks: true,
    model_tier_escalation: 1,
    extra_retries: 2,
    trigger_dream_cycles: false,
    run_maintenance_tasks: false,
}
```

**Rationale**: When struggling, the agent either escalates (uses stronger models, more retries, more turns) or conserves (sticks to proven approaches, reduces scope). The choice depends on the specific failure mode:
- Frustrated and fighting (Angry octant: -P, +A, +D) → Escalating. The agent believes it *can* solve this but needs more resources.
- Anxious and unsure (Anxious octant: -P, +A, -D) → Conservative. The agent is failing and doesn't know why — fall back to known-good approaches.

This distinction implements a coarse version of the confidence-competence matrix: high dominance with low pleasure suggests a resource problem (escalate), while low dominance with low pleasure suggests a knowledge problem (conserve and fall back).

### Coasting (Exploratory)

```rust
// From roko-daimon modulate():
params.strategy = DispatchStrategy::Exploratory;
params.turn_limit = params.turn_limit.saturating_sub(5);
params.model = demote_model(&params.model);  // opus → sonnet → haiku
```

**From roko-golem (Confident octant → Exploratory)**:

```rust
AffectBehaviorModulation {
    strategy: Exploratory,
    exploration_rate: 0.35,
    prefer_proven_playbooks: false,
    model_tier_escalation: 0,
    extra_retries: 0,
    trigger_dream_cycles: false,
    run_maintenance_tasks: false,
}
```

**Rationale**: When things are going well, use cheaper models (demote from opus to sonnet, or sonnet to haiku), reduce turn limits (the tasks are easier — don't waste compute), and increase exploration rate to 35%. This is the compute efficiency dividend: successful streaks pay for experimentation. The agent can afford to try novel approaches because the baseline is reliable.

### Exploring

Exploring is not a single octant mapping — it's triggered by low dominance (D < 0.10) regardless of pleasure or arousal. The agent doesn't yet have specific behavioral parameters in the current implementation; it maps to the Balanced defaults. The legacy specification adds T2 routing for research tasks and T1 for breadth queries.

**Expected behavior** (from `refactoring-prd/03-cognitive-subsystems.md`):
- T2 for research — route to deep reasoning when exploring unfamiliar territory
- T1 for breadth — use fast models to scan many possibilities
- Exploration budget — allocate compute for learning, not producing

### Focused

Focused maps to high dominance with high pleasure — the agent is succeeding at something it understands well. In the current implementation, this falls through to the Coasting branch (high pleasure triggers Exploratory strategy). The intended behavior from the legacy spec:

- T0/T1 routing — exploit known patterns with cheap models
- Maximum speed — reduce overhead, skip optional checks
- Cached strategies — prefer playbook matches over novel synthesis

### Resting (Proactive)

```rust
// From roko-daimon modulate():
params.strategy = DispatchStrategy::Proactive;
params.turn_limit = params.turn_limit.saturating_add(5);
```

**From roko-golem (Bored octant → Proactive)**:

```rust
AffectBehaviorModulation {
    strategy: Proactive,
    exploration_rate: 0.25,
    prefer_proven_playbooks: true,
    model_tier_escalation: 0,
    extra_retries: 0,
    trigger_dream_cycles: true,
    run_maintenance_tasks: true,
}
```

**Rationale**: When arousal is low and dominance is low, the agent has no urgent work and no strong opinions about what to do. This is the time for background cognitive tasks: trigger dream cycles to consolidate recent episodes, run maintenance tasks (knowledge pruning, index rebuilding), and prepare for the next active phase. The 25% exploration rate is moderate — the agent is curious but not under pressure to discover.

---

## Tier Bias Table

The behavioral state modulates the tier router's prediction error threshold. This is the concrete mechanism by which affect controls compute allocation:

| State | T0 (Free Probes) | T1 (Fast Model) | T2 (Deep Reasoning) | Rationale |
|---|---|---|---|---|
| **Engaged** | Standard distribution | Standard | Standard | Baseline — let the prediction error decide |
| **Struggling** | Reduced | Reduced | **Increased** | Force deep reasoning sooner — lower the T2 trigger threshold |
| **Coasting** | **Increased** | **Increased** | Reduced | Stay cheap longer — raise the T2 trigger threshold |
| **Exploring** | Standard | **Increased** | Increased for research | Broad scanning with fast models, deep dive for unknowns |
| **Focused** | **Increased** | Standard | Reduced | Exploit known patterns — suppress unnecessary deep reasoning |
| **Resting** | Standard | Standard for dreams | N/A | Use T1 for dream consolidation; T2 not warranted for maintenance |

The tier bias works by modulating the prediction error threshold that determines cognitive tier routing (see topic [05-learning](../05-learning/INDEX.md)):

```
Standard threshold:   error < 0.2 → T0; error < 0.6 → T1; error ≥ 0.6 → T2
Struggling bias:      error < 0.1 → T0; error < 0.4 → T1; error ≥ 0.4 → T2
Coasting bias:        error < 0.3 → T0; error < 0.8 → T1; error ≥ 0.8 → T2
```

A Struggling agent routes to T2 (deep reasoning) at prediction error 0.4 instead of 0.6 — it escalates sooner because its recent experience suggests the situation is harder than the probes indicate. A Coasting agent stays on T1 until prediction error 0.8 — it trusts its current approach and only escalates when something is clearly wrong.

---

## Cyclicality: No Terminal State

The behavioral states form a cycle, not a directed path:

```
           ┌──────────────────────────────────┐
           │                                  │
    Engaged ──→ Struggling ──→ Resting        │
       ↑            │              │          │
       │            ▼              ▼          │
    Focused ←── Exploring    (Dream cycles)   │
       ↑                           │          │
       │                           ▼          │
       └──────── Coasting ←────────┘          │
                    │                          │
                    └──────────────────────────┘
```

**Common transition patterns**:

1. **Recovery from struggle**: Struggling → Resting → Exploring → Engaged. After sustained failure, arousal eventually decays (dream depotentiation or natural decay), dominance stays low → Exploring. Successful exploration raises dominance → Engaged.

2. **Performance optimization**: Engaged → Focused → Coasting. Sustained success raises pleasure and dominance → Focused. Continued success with decreasing difficulty → Coasting.

3. **Challenge encounter**: Coasting → Engaged → Struggling. A harder problem raises arousal and lowers pleasure → Engaged. Continued difficulty lowers pleasure and dominance further → Struggling.

4. **Knowledge plateau**: Focused → Exploring. The agent exhausts its known approaches (dominance drops as playbooks stop producing results) → low D triggers Exploring.

Every state can transition to every other state through intermediate PAD changes. No state is absorbing. The decay mechanism ensures that even sustained extreme states eventually moderate — a 4-hour half-life means that after 12 hours without reinforcing events, PAD values are at 12.5% of their peak (three half-lives: 0.5³ = 0.125).

---

## Dispatch Strategy Labels

Each behavioral state maps to a dispatch strategy that carries an effort label used for logging and cost tracking:

```rust
pub enum DispatchStrategy {
    Conservative,   // effort: "low"
    Balanced,       // effort: "medium"
    Exploratory,    // effort: "medium"
    Escalating,     // effort: "high"
    Proactive,      // effort: "medium"
}

impl DispatchStrategy {
    pub const fn effort_label(&self) -> &'static str {
        match self {
            Self::Conservative => "low",
            Self::Balanced => "medium",
            Self::Exploratory => "medium",
            Self::Escalating => "high",
            Self::Proactive => "medium",
        }
    }
}
```

The effort label is written to efficiency events (`.roko/learn/efficiency.jsonl`) for cost tracking. Over time, the system can analyze the correlation between effort levels and task outcomes to calibrate the behavioral state thresholds.

---

## Current Implementation Status

**Implemented in `roko-daimon/src/lib.rs`**: The `modulate()` method on `DaimonState` implements four of the six states through PAD threshold checks. Model promotion/demotion (haiku ↔ sonnet ↔ opus) is fully wired. Turn limit adjustments are wired. Strategy selection and effort labeling are wired.

**Implemented in `roko-golem/src/daimon.rs`**: The `AffectBehaviorModulation` struct and five factory methods (balanced, anxious, confident, angry, bored) implement the octant-to-behavior mapping. The `AffectOctant::behavior_modulation()` method dispatches to these factories.

**Gap**: The two implementations are parallel and not yet merged. `roko-daimon` uses PAD thresholds directly in `modulate()`, while `roko-golem` uses octant classification through `AffectOctant::behavior_modulation()`. The plan (Tier 0C) is to dissolve `roko-golem` and consolidate the affect logic into `roko-daimon`.

**Gap**: The Exploring and Focused states don't have dedicated behavioral profiles in the current code — they fall through to Balanced or Coasting.

**Gap**: Tier bias threshold modulation is specified but not wired to the CascadeRouter. The CascadeRouter has its own prediction error thresholds that don't yet read from the Daimon.

---

## Threshold Calibration Methodology

### Why 0.30 / -0.25?

The thresholds in `classify_behavioral_state()` are not hand-tuned constants. They derive from the appraisal rule magnitudes defined in `03-occ-scherer-appraisal.md`:

| Event | P delta | A delta | D delta | C delta |
|---|---|---|---|---|
| Single task failure | -0.20 | +0.10 | -0.15 | -0.15 |
| Gate check failure | -0.10 | +0.04 | -0.08 | -0.10 |
| Consecutive failures (3x) | -0.60 | +0.30 | -0.45 | -0.45 |

Starting from a neutral confidence of 0.70 (the midpoint of the working range), two consecutive task failures push confidence to `0.70 - 0.30 = 0.40`, which stays above the 0.30 Struggling threshold. Three consecutive failures push confidence to `0.70 - 0.45 = 0.25`, which crosses 0.30 and triggers Struggling. This produces the desired behavior: the agent tolerates one or two failures without state change, but three consecutive failures trigger intervention.

The dominance threshold of -0.25 follows a parallel logic. A single task failure produces D: -0.15 from neutral 0.0. That keeps the agent in Engaged. Two consecutive failures produce cumulative D: -0.30 (assuming partial decay between ticks), which crosses -0.25 and triggers Struggling through the dominance path.

**Calibration formula**:

```
threshold = neutral_value - (n_tolerated_failures * per_failure_delta)

confidence_threshold = 0.70 - (2.5 * 0.15) ≈ 0.30
dominance_threshold  = 0.00 - (2.0 * 0.15) ≈ -0.25 (accounting for partial decay)
```

The 2.5 and 2.0 multipliers represent the design choice: how many failures should the agent absorb before behavioral state change? Lowering the threshold increases tolerance; raising it makes the agent more reactive.

### Hysteresis: Entry vs. Exit Thresholds

The current implementation uses a single threshold for both entering and exiting a state. This creates oscillation at the boundary: an agent at confidence 0.29 enters Struggling, a single positive outcome pushes it to 0.35, it exits to Engaged, another failure pushes it back below 0.30, and it re-enters Struggling. This rapid cycling produces inconsistent behavior modulation.

The fix is split thresholds with a dead zone:

```rust
pub struct BehavioralStateThresholds {
    /// Confidence below this enters Struggling.
    pub struggling_entry_confidence: f64,   // default: 0.30
    /// Confidence above this exits Struggling.
    pub struggling_exit_confidence: f64,    // default: 0.40
    /// Dominance below this enters Struggling.
    pub struggling_entry_dominance: f64,    // default: -0.25
    /// Dominance above this exits Struggling.
    pub struggling_exit_dominance: f64,     // default: -0.15
    /// Pleasure above this enters Coasting.
    pub coasting_entry_pleasure: f64,       // default: 0.35
    /// Pleasure below this exits Coasting.
    pub coasting_exit_pleasure: f64,        // default: 0.25
    /// Arousal below this enters Resting.
    pub resting_entry_arousal: f64,         // default: -0.20
    /// Arousal above this exits Resting.
    pub resting_exit_arousal: f64,          // default: -0.10
}
```

The dead zone between entry and exit thresholds (e.g., 0.30 to 0.40 for confidence) prevents oscillation. An agent must improve its confidence by a meaningful margin before exiting Struggling, which requires sustained positive outcomes rather than a single lucky result.

**Pseudocode with hysteresis**:

```
fn classify_with_hysteresis(state: &AffectState, current: BehavioralState) -> BehavioralState:
    thresholds = load_thresholds()

    // Use exit thresholds if already in the state, entry thresholds otherwise
    if current == Struggling:
        if state.confidence > thresholds.struggling_exit_confidence
           AND state.pad.dominance > thresholds.struggling_exit_dominance:
            // Allow exit — re-evaluate for other states
            pass
        else:
            return Struggling  // Stay in Struggling
    else:
        if state.confidence < thresholds.struggling_entry_confidence
           OR state.pad.dominance < thresholds.struggling_entry_dominance:
            return Struggling  // Enter Struggling

    // Repeat pattern for each state...
```

### Dwell Time Minimum

Even with hysteresis, rapid PAD swings (e.g., from alternating successes and failures) can produce state changes every few ticks. A minimum dwell time prevents this:

```rust
pub struct BehavioralStateTracker {
    pub current_state: BehavioralState,
    /// Tick at which the current state was entered.
    pub entered_at: u64,
    /// Minimum ticks before a state transition is allowed.
    pub min_dwell_ticks: u64,  // default: 10
    pub thresholds: BehavioralStateThresholds,
}

impl BehavioralStateTracker {
    pub fn update(&mut self, state: &AffectState, current_tick: u64) -> BehavioralState {
        let candidate = classify_with_hysteresis(state, self.current_state, &self.thresholds);
        if candidate != self.current_state {
            let dwell = current_tick.saturating_sub(self.entered_at);
            if dwell >= self.min_dwell_ticks {
                self.current_state = candidate;
                self.entered_at = current_tick;
            }
            // Otherwise: suppress the transition, stay in current state
        }
        self.current_state
    }
}
```

The default of 10 ticks translates to roughly 10 task cycles. This prevents state flickering while keeping the system responsive to sustained changes.

### Domain-Specific Threshold Variants

Different agent domains have different failure rates and volatility profiles. A coding agent working on well-tested code fails rarely; a chain agent monitoring volatile markets experiences frequent prediction errors. Using the same thresholds for both produces suboptimal behavior.

```toml
# roko.toml — domain-specific threshold overrides
[daimon.behavioral_thresholds]
domain = "coding"

[daimon.behavioral_thresholds.struggling]
entry_confidence = 0.30
exit_confidence = 0.40
entry_dominance = -0.25
exit_dominance = -0.15

[daimon.behavioral_thresholds.coasting]
entry_pleasure = 0.35
exit_pleasure = 0.25

# Chain agents tolerate more volatility before entering Struggling
# [daimon.behavioral_thresholds]
# domain = "chain"
# struggling.entry_confidence = 0.20  # more tolerant
# struggling.entry_dominance = -0.35  # more tolerant
# coasting.entry_pleasure = 0.50      # harder to coast in volatile markets
```

Threshold overrides are loaded from `roko.toml` at startup. If no domain-specific overrides exist, the defaults above apply.

---

## Tier Bias to CascadeRouter Wiring

The tier bias table (above) specifies how behavioral states modulate prediction error thresholds. The CascadeRouter — defined in `roko-learn/src/cascade_router.rs` — owns these thresholds. The wiring path connects the Daimon's behavioral state to the CascadeRouter's threshold adjustment:

```rust
/// In the dispatch path (orchestrate.rs or dispatcher):
fn apply_tier_bias(
    router: &mut CascadeRouter,
    state: BehavioralState,
) {
    let bias = match state {
        BehavioralState::Struggling => TierBias {
            t0_threshold_delta: -0.1,  // T0 → T1 sooner
            t1_threshold_delta: -0.2,  // T1 → T2 sooner
        },
        BehavioralState::Coasting => TierBias {
            t0_threshold_delta: 0.1,   // T0 → T1 later
            t1_threshold_delta: 0.2,   // T1 → T2 later
        },
        BehavioralState::Focused => TierBias {
            t0_threshold_delta: 0.1,
            t1_threshold_delta: 0.0,
        },
        _ => TierBias::ZERO,
    };
    router.set_tier_bias(bias);
}

pub struct TierBias {
    /// Added to the T0→T1 prediction error threshold.
    /// Positive = harder to escalate (stay cheap longer).
    /// Negative = easier to escalate (use stronger models sooner).
    pub t0_threshold_delta: f64,
    /// Added to the T1→T2 prediction error threshold.
    pub t1_threshold_delta: f64,
}

impl TierBias {
    pub const ZERO: Self = Self { t0_threshold_delta: 0.0, t1_threshold_delta: 0.0 };
}
```

**Call site**: This runs in the dispatch path, after the Daimon computes the behavioral state and before the CascadeRouter selects a model. The flow is:

```
AffectEngine::tick() → BehavioralState
    → apply_tier_bias(&mut cascade_router, state)
    → CascadeRouter::select_model(prediction_error)  // uses biased thresholds
    → Dispatcher::dispatch(model, task)
```

### Effort Label to Budget Tracker

The `DispatchStrategy::effort_label()` method returns `"low"`, `"medium"`, or `"high"`. This feeds into the efficiency event pipeline:

```rust
/// Written per-task to .roko/learn/efficiency.jsonl
pub struct EfficiencyEvent {
    pub task_id: String,
    pub model_used: String,
    pub effort_label: String,     // from DispatchStrategy::effort_label()
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub wall_time_ms: u64,
    pub gate_passed: bool,
}
```

Over time, the efficiency log enables cost-outcome correlation analysis. If `"high"` effort tasks don't pass gates at a higher rate than `"medium"` effort tasks, the Struggling → Escalating path is wasteful and the thresholds should be adjusted. The adaptive gate thresholds (`.roko/learn/gate-thresholds.json`) already track pass rates per rung; extending this to track pass rates per effort label requires adding a grouping key to the EMA computation.

---

## Full State Transition Table

Every transition requires both a PAD condition (the target state's entry criteria) and satisfaction of the dwell time minimum for the current state:

| From | To | Trigger Condition | Typical Cause |
|---|---|---|---|
| **Engaged** | Struggling | C < 0.30 OR D < -0.25 | 3+ consecutive failures |
| **Engaged** | Coasting | P > 0.35 AND C > 0.65 | Sustained easy successes |
| **Engaged** | Focused | D > 0.30 AND P > 0.25 | Success in familiar territory |
| **Engaged** | Resting | A < -0.20 | No tasks in queue, idle period |
| **Engaged** | Exploring | D < 0.10 AND P > -0.20 | New crate, unfamiliar API |
| **Struggling** | Engaged | C > 0.40 AND D > -0.15 (exit thresholds) | Successful task after struggle |
| **Struggling** | Resting | A < -0.20 (arousal decay over time) | Dream depotentiation, idle period |
| **Coasting** | Engaged | P < 0.25 OR C < 0.65 (exit thresholds) | Harder problem encountered |
| **Coasting** | Struggling | C < 0.30 OR D < -0.25 | Sudden failure on "easy" task |
| **Focused** | Engaged | D < 0.30 OR P < 0.25 | Exhausted playbooks, diminishing returns |
| **Focused** | Coasting | P > 0.35 AND C > 0.65 | Continued success, difficulty drops |
| **Focused** | Struggling | C < 0.30 OR D < -0.25 | Unexpected failure in "known" territory |
| **Resting** | Engaged | A > -0.10 (exit threshold) | New task arrives, arousal increases |
| **Resting** | Exploring | D < 0.10 AND A > -0.10 | Task arrives in unfamiliar area |
| **Exploring** | Engaged | D > 0.10 OR P < -0.20 | Gained familiarity, or failed exploration |
| **Exploring** | Focused | D > 0.30 AND P > 0.25 | Exploration succeeded, built mastery |
| **Exploring** | Struggling | C < 0.30 OR D < -0.25 | Exploration failed repeatedly |

**Priority order**: When multiple states' entry criteria are met simultaneously, the classification function checks in this order: Struggling, Coasting, Focused, Resting, Exploring, Engaged (fallback). Struggling takes priority because it triggers protective measures that should not be delayed.

**Error handling**: If the PAD vector contains NaN or infinite values (from a buggy appraisal rule), `classify_behavioral_state` returns `BehavioralState::Engaged` as the safe default. The Engaged state applies no escalation and no demotion, so a corrupt PAD vector produces baseline behavior rather than runaway escalation.

```rust
fn classify_behavioral_state(state: &AffectState) -> BehavioralState {
    // Guard: NaN/Inf → safe default
    if !state.pad.pleasure.is_finite()
        || !state.pad.arousal.is_finite()
        || !state.pad.dominance.is_finite()
        || !state.confidence.is_finite()
    {
        tracing::warn!("non-finite PAD values detected, defaulting to Engaged");
        return BehavioralState::Engaged;
    }
    // ... normal classification
}
```

### Test Criteria

| Test | Condition | Expected |
|---|---|---|
| Three consecutive failures trigger Struggling | Confidence: 0.70 → 0.55 → 0.40 → 0.25 | State: Engaged → Engaged → Engaged → Struggling |
| Hysteresis prevents oscillation | Confidence bounces 0.29 → 0.35 → 0.29 | State: Struggling → Struggling → Struggling (exit requires > 0.40) |
| Dwell time suppresses flickering | State entered 3 ticks ago, new criteria met for Coasting | State: unchanged (min dwell = 10) |
| NaN PAD defaults to Engaged | `pad.pleasure = f64::NAN` | `BehavioralState::Engaged` |
| Struggling priority over Coasting | C < 0.30 AND P > 0.35 | Struggling (checked first) |
| Domain threshold override loads | `roko.toml` sets `struggling.entry_confidence = 0.20` | Agent tolerates 4 failures before Struggling |
| Tier bias propagates to CascadeRouter | State transitions to Struggling | Router's T1→T2 threshold drops by 0.2 |
| Effort label written to efficiency log | Escalating strategy dispatched | `efficiency.jsonl` entry has `effort_label: "high"` |

---

## Academic Foundations

- Mehrabian, A. (1996). "Pleasure-arousal-dominance: A general framework for describing and measuring individual differences in temperament." *Current Psychology*, 14(4), 261–292.
- Russell, J.A. & Mehrabian, A. (1977). "Evidence for a three-factor theory of emotions." *Journal of Research in Personality*, 11(3), 273–294.
- Ortony, A., Clore, G.L., & Collins, A. (1988). *The Cognitive Structure of Emotions*. Cambridge University Press.
- Gebhard, P. (2005). "ALMA — A Layered Model of Affect." In *Proceedings of the Fourth International Joint Conference on Autonomous Agents and Multiagent Systems*, pp. 29–36.
- Chen, L. et al. (2023). "FrugalGPT: How to Use Large Language Models While Reducing Cost and Improving Performance." arXiv:2305.05176.

---

## Cross-references

- See [01-pad-vector.md](./01-pad-vector.md) for PAD vector structure and octant classification
- See [02-alma-three-layer-temporal.md](./02-alma-three-layer-temporal.md) for temporal dynamics of state changes
- See [05-behavioral-state-to-tier-routing.md](./05-behavioral-state-to-tier-routing.md) for detailed tier routing bias
- See [10-integration-points.md](./10-integration-points.md) for how behavioral states connect to dispatch, routing, and VCG
- See [13-current-status-and-gaps.md](./13-current-status-and-gaps.md) for implementation gaps
