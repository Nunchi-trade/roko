# Behavioral States and Routing

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How behavioral state classification and affect-modulated routing emerge from Score and Route Cells rather than hardcoded threshold tables.

---

## 1. The Problem with Threshold Tables

The v1 behavioral state system is a match statement inside `BehavioralState::classify(pad, confidence)` that checks hardcoded threshold values. The tier routing system is a second match statement that maps each behavioral state to a pair of tier thresholds. Both live as standalone logic inside `DaimonState`, called imperatively from `orchestrate.rs`.

This has three problems:
1. **No learning.** The thresholds are constants. An agent that has been running for 1000 tasks uses the same thresholds as one that just started. There is no feedback from gate verdicts to threshold calibration.
2. **No hysteresis in practice.** The spec calls for entry/exit threshold asymmetry and dwell time minimums, but the implementation is a single classify call per tick with no state between calls. Hysteresis requires persistent state -- which is exactly what a Cell provides.
3. **No Bus integration.** Behavioral state changes are invisible to other Cells. Nothing publishes a Pulse when the agent transitions from Engaged to Struggling. The TUI polls `daimon.behavioral_state()` directly.

The unified redesign decomposes this into two Cells: a Score Cell that classifies behavioral state (with hysteresis as internal Cell state), and a Route Cell that adjusts EFE tier thresholds based on the current behavioral state. Both communicate via Bus, not function calls.

---

## 2. Behavioral State Classification as a Score Cell

### 2.1 The Six Behavioral States

Six cyclical, non-terminal states with asymmetric entry/exit thresholds (hysteresis prevents oscillation):

| State | Entry Condition | Exit Condition | Dwell Minimum |
|---|---|---|---|
| **Engaged** | Default; no other state qualifies | Another state's entry condition met | 10 ticks |
| **Struggling** | `C < 0.30 OR D < -0.25 OR (P < -0.30 AND A > 0.30)` | `C > 0.40 AND D > -0.15 AND (P > -0.20 OR A < 0.20)` | 10 ticks |
| **Coasting** | `P > 0.35 AND C > 0.65` | `P < 0.25 OR C < 0.55` | 10 ticks |
| **Focused** | `D > 0.30 AND P > 0.25` | `D < 0.20 OR P < 0.15` | 10 ticks |
| **Resting** | `A < -0.20` | `A > -0.10` | 10 ticks |
| **Exploring** | `D < 0.10 AND P > -0.20` | `D > 0.20 OR P < -0.30` | 10 ticks |

Where P=Pleasure, A=Arousal, D=Dominance, C=Confidence.

### 2.2 Score Cell Design

The behavioral state classifier implements the **Score** protocol. It receives a Signal carrying the current PAD context and produces a scored ranking of all six states. The highest-scoring state that passes the hysteresis filter becomes the active state.

```rust
/// Behavioral state classifier: a Score Cell with internal hysteresis state.
///
/// Input:  Signal with "affect.pad" metadata (PadContext)
/// Output: Signal with "behavioral.state" metadata (BehavioralState)
///         + "behavioral.scores" metadata (Vec<(BehavioralState, f64)>)
///
/// Subscribes to: "affect.transition" Pulses on Bus
/// Publishes:     "behavioral.transition" Pulse on Bus (when state changes)
struct BehavioralStateScorer {
    /// Current active state.
    current: BehavioralState,
    /// Ticks spent in current state (for dwell minimum enforcement).
    dwell_ticks: u64,
    /// Minimum dwell before allowing transition.
    min_dwell: u64,  // default: 10
    /// Entry thresholds per state.
    entry_thresholds: BehavioralThresholds,
    /// Exit thresholds per state (asymmetric with entry for hysteresis).
    exit_thresholds: BehavioralThresholds,
}

/// Threshold configuration for one behavioral state.
/// Entry and exit thresholds differ to create hysteresis band.
#[derive(Debug, Clone, Copy)]
struct StateThresholds {
    // Entry: must cross these to enter this state
    pub entry_pleasure: Option<ThresholdBound>,
    pub entry_arousal: Option<ThresholdBound>,
    pub entry_dominance: Option<ThresholdBound>,
    pub entry_confidence: Option<ThresholdBound>,

    // Exit: must cross these to leave this state
    pub exit_pleasure: Option<ThresholdBound>,
    pub exit_arousal: Option<ThresholdBound>,
    pub exit_dominance: Option<ThresholdBound>,
    pub exit_confidence: Option<ThresholdBound>,
}

#[derive(Debug, Clone, Copy)]
enum ThresholdBound {
    Above(f64),  // value must be above this
    Below(f64),  // value must be below this
}
```

### 2.3 Scoring Function

Rather than a binary classify, the Score Cell computes a continuous score for each state. This score represents how well the current PAD matches the state's archetype. The state with the highest score that also passes the entry threshold check is the candidate.

```rust
impl Cell for BehavioralStateScorer {
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::SCORE]
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let pad = PadContext::from_signals(&input)?;

        // Score each state by how well PAD matches its archetype.
        let scores = self.score_all_states(&pad);

        // Determine candidate state (highest score that passes entry threshold).
        let candidate = self.select_candidate(&pad, &scores);

        // Apply hysteresis: check if we should transition.
        let (new_state, transitioned) = self.apply_hysteresis(candidate, &pad);

        // Publish state change on Bus if transitioned.
        if transitioned {
            ctx.bus().publish(Pulse::new(
                "behavioral.transition",
                serde_json::json!({
                    "from": self.current,
                    "to": new_state,
                    "pad": pad,
                    "scores": scores,
                    "dwell_ticks": self.dwell_ticks,
                }),
            )).await?;
        }

        let mut out = input;
        out.push(Signal::metadata("behavioral.state", new_state));
        out.push(Signal::metadata("behavioral.scores", scores));
        Ok(out)
    }
}

impl BehavioralStateScorer {
    /// Score how well a PAD vector matches each behavioral state's archetype.
    ///
    /// Archetype centers (PAD + Confidence):
    ///   Engaged:    (0.0,  0.0,  0.0, 0.50)  — neutral
    ///   Struggling: (-0.4, 0.5, -0.4, 0.20)  — negative, stressed, unconfident
    ///   Coasting:   (0.5, -0.1,  0.2, 0.75)  — positive, calm, confident
    ///   Focused:    (0.3,  0.1,  0.5, 0.60)  — positive, dominant
    ///   Resting:    (0.0, -0.4,  0.0, 0.50)  — low arousal
    ///   Exploring:  (0.1,  0.2, -0.1, 0.45)  — curious, low dominance
    fn score_all_states(&self, pad: &PadContext) -> Vec<(BehavioralState, f64)> {
        let archetypes = [
            (BehavioralState::Engaged,    PadContext { pleasure: 0.0,  arousal: 0.0,  dominance: 0.0,  confidence: 0.50 }),
            (BehavioralState::Struggling, PadContext { pleasure: -0.4, arousal: 0.5,  dominance: -0.4, confidence: 0.20 }),
            (BehavioralState::Coasting,   PadContext { pleasure: 0.5,  arousal: -0.1, dominance: 0.2,  confidence: 0.75 }),
            (BehavioralState::Focused,    PadContext { pleasure: 0.3,  arousal: 0.1,  dominance: 0.5,  confidence: 0.60 }),
            (BehavioralState::Resting,    PadContext { pleasure: 0.0,  arousal: -0.4, dominance: 0.0,  confidence: 0.50 }),
            (BehavioralState::Exploring,  PadContext { pleasure: 0.1,  arousal: 0.2,  dominance: -0.1, confidence: 0.45 }),
        ];

        archetypes
            .iter()
            .map(|(state, archetype)| {
                // Inverse Euclidean distance in 4D (P, A, D, C) space.
                // Closer to archetype = higher score.
                let dist = (
                    (pad.pleasure - archetype.pleasure).powi(2)
                    + (pad.arousal - archetype.arousal).powi(2)
                    + (pad.dominance - archetype.dominance).powi(2)
                    + (pad.confidence - archetype.confidence).powi(2)
                ).sqrt();

                // Score = 1.0 / (1.0 + dist): ranges from (0, 1], closest = highest
                let score = 1.0 / (1.0 + dist);
                (*state, score)
            })
            .collect()
    }

    /// Select the best candidate that passes its entry threshold.
    fn select_candidate(
        &self,
        pad: &PadContext,
        scores: &[(BehavioralState, f64)],
    ) -> BehavioralState {
        let mut sorted = scores.to_vec();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        for (state, _score) in &sorted {
            if self.meets_entry_threshold(*state, pad) {
                return *state;
            }
        }

        // Fallback: Engaged is always valid.
        BehavioralState::Engaged
    }

    /// Apply hysteresis: transition only if dwell minimum met AND exit
    /// threshold of current state is crossed.
    fn apply_hysteresis(
        &mut self,
        candidate: BehavioralState,
        pad: &PadContext,
    ) -> (BehavioralState, bool) {
        self.dwell_ticks += 1;

        if candidate == self.current {
            return (self.current, false);
        }

        // Check dwell minimum.
        if self.dwell_ticks < self.min_dwell {
            return (self.current, false);
        }

        // Check exit threshold of current state.
        if !self.meets_exit_threshold(self.current, pad) {
            return (self.current, false);
        }

        // Transition.
        let old = self.current;
        self.current = candidate;
        self.dwell_ticks = 0;
        (candidate, true)
    }

    fn meets_entry_threshold(&self, state: BehavioralState, pad: &PadContext) -> bool {
        match state {
            BehavioralState::Engaged => true, // always valid
            BehavioralState::Struggling =>
                pad.confidence < 0.30
                || pad.dominance < -0.25
                || (pad.pleasure < -0.30 && pad.arousal > 0.30),
            BehavioralState::Coasting =>
                pad.pleasure > 0.35 && pad.confidence > 0.65,
            BehavioralState::Focused =>
                pad.dominance > 0.30 && pad.pleasure > 0.25,
            BehavioralState::Resting =>
                pad.arousal < -0.20,
            BehavioralState::Exploring =>
                pad.dominance < 0.10 && pad.pleasure > -0.20,
        }
    }

    fn meets_exit_threshold(&self, state: BehavioralState, pad: &PadContext) -> bool {
        match state {
            BehavioralState::Engaged => true, // can always exit
            BehavioralState::Struggling =>
                pad.confidence > 0.40
                && pad.dominance > -0.15
                && (pad.pleasure > -0.20 || pad.arousal < 0.20),
            BehavioralState::Coasting =>
                pad.pleasure < 0.25 || pad.confidence < 0.55,
            BehavioralState::Focused =>
                pad.dominance < 0.20 || pad.pleasure < 0.15,
            BehavioralState::Resting =>
                pad.arousal > -0.10,
            BehavioralState::Exploring =>
                pad.dominance > 0.20 || pad.pleasure < -0.30,
        }
    }
}
```

### 2.4 Hysteresis as Cell Internal State

The critical design choice: hysteresis state (`current`, `dwell_ticks`) lives inside the Cell, not in a separate `DaimonState` struct. This makes the Cell self-contained. It snapshots with `Cell::snapshot()` and resumes with `Cell::restore()`. No external state coordination required.

The asymmetric entry/exit thresholds create a deadband. An agent that enters Struggling at C<0.30 must reach C>0.40 to exit. This prevents rapid oscillation between states when PAD hovers near a boundary.

The 10-tick dwell minimum adds temporal smoothing: even if a threshold is crossed, the agent must remain in the new qualifying region for 10 consecutive evaluations before transitioning. This filters out single-tick spikes.

---

## 3. Tier Routing Modulation as Route Cell Parameter Adjustment

### 3.1 From Table to Route Cell

The v1 tier routing table maps each behavioral state to a pair of thresholds (T0 ceiling, T1 ceiling). These thresholds determine when the EFE cascade escalates from T0 to T1 or from T1 to T2.

In the unified model, this is not a lookup table. It is a **parameter adjustment on the Route Cell's EFE computation**. The Route Cell already computes EFE for each tier candidate. The behavioral state modifies the cost term in the EFE formula:

```
EFE(candidate) = -pragmatic_value - epistemic_value + cost_term
```

The cost term is where behavioral state exerts influence:

```rust
/// Behavioral state modulation of the Route Cell's EFE cost term.
///
/// Each state adjusts three parameters:
/// - cost_multiplier: scales the base cost of each tier
/// - epistemic_bonus: adjusts exploration incentive
/// - escalation_bias: shifts the threshold for tier escalation
struct RoutingModulation {
    /// Multiplicative scaling on tier cost.
    /// < 1.0: tier appears cheaper (earlier escalation).
    /// > 1.0: tier appears more expensive (later escalation).
    pub cost_multiplier: f64,
    /// Additive bonus to epistemic value term.
    /// > 0.0: encourages exploration (try unfamiliar tiers).
    /// < 0.0: discourages exploration (stick to known tiers).
    pub epistemic_bonus: f64,
    /// Number of additional retries before escalating.
    pub retry_budget: u8,
}

impl RoutingModulation {
    /// Derive modulation parameters from behavioral state and PAD octant.
    fn from_behavioral_state(state: BehavioralState, pad: &PadContext) -> Self {
        let octant = pad.octant();

        match state {
            BehavioralState::Engaged => Self {
                cost_multiplier: 1.0,   // baseline
                epistemic_bonus: 0.0,
                retry_budget: 1,
            },

            BehavioralState::Struggling => {
                match octant {
                    // Struggling + Anxious: conservative, no retries, demote model
                    AffectOctant::Anxious | AffectOctant::Depressed => Self {
                        cost_multiplier: 0.6,   // T2 appears cheap -> escalate sooner
                        epistemic_bonus: -0.1,   // avoid exploration
                        retry_budget: 0,
                    },
                    // Struggling + Hostile: escalating, +2 retries, promote model
                    AffectOctant::Hostile | AffectOctant::Disdainful => Self {
                        cost_multiplier: 0.4,   // strong escalation pressure
                        epistemic_bonus: 0.0,
                        retry_budget: 3,         // more attempts before giving up
                    },
                    _ => Self {
                        cost_multiplier: 0.7,
                        epistemic_bonus: -0.05,
                        retry_budget: 1,
                    },
                }
            }

            BehavioralState::Coasting => Self {
                cost_multiplier: 1.5,    // T2 appears expensive -> stay on T0/T1
                epistemic_bonus: 0.15,   // 35% exploration rate
                retry_budget: 1,
            },

            BehavioralState::Focused => Self {
                cost_multiplier: 1.2,    // slight cost pressure -> prefer T1
                epistemic_bonus: 0.0,
                retry_budget: 2,
            },

            BehavioralState::Resting => Self {
                cost_multiplier: 2.0,    // strongly discourage expensive tiers
                epistemic_bonus: 0.0,
                retry_budget: 0,
            },

            BehavioralState::Exploring => Self {
                cost_multiplier: 0.8,    // lower barrier to trying expensive tiers
                epistemic_bonus: 0.20,   // strong exploration bonus
                retry_budget: 2,
            },
        }
    }
}
```

### 3.2 Route Cell Integration

The Route Cell reads behavioral state from the Signal's metadata (stamped by the Daimon Functor during ASSESS pre-enrichment). It does not call `daimon.behavioral_state()`. It reads `Signal::metadata["behavioral.state"]`.

```rust
/// Route Cell with affect-modulated EFE computation.
struct AffectAwareRouter {
    /// Base EFE parameters (pragmatic weights, cost table, etc.)
    base_params: EfeParams,
}

impl Cell for AffectAwareRouter {
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::ROUTE]
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // 1. Read behavioral state from Signal metadata (set by Daimon Functor).
        let behavioral = BehavioralState::from_signals(&input)
            .unwrap_or(BehavioralState::Engaged);

        // 2. Read PAD context.
        let pad = read_affect_from_signals(&input)
            .unwrap_or(PadContext::NEUTRAL);

        // 3. Compute routing modulation.
        let modulation = RoutingModulation::from_behavioral_state(behavioral, &pad);

        // 4. Compute EFE for each tier candidate with modulated cost.
        let candidates = [CognitiveTier::T0, CognitiveTier::T1, CognitiveTier::T2];
        let mut efe_scores: Vec<(CognitiveTier, f64)> = candidates
            .iter()
            .map(|tier| {
                let pragmatic = self.base_params.pragmatic_value(*tier, &input);
                let epistemic = self.base_params.epistemic_value(*tier, &input)
                    + modulation.epistemic_bonus;
                let cost = self.base_params.cost(*tier)
                    * modulation.cost_multiplier;

                let efe = -pragmatic - epistemic + cost;
                (*tier, efe)
            })
            .collect();

        // 5. Select lowest EFE (most negative = best).
        efe_scores.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let selected = efe_scores[0].0;

        // 6. Record routing decision for learning.
        let mut out = input;
        out.push(Signal::metadata("route.selected_tier", selected));
        out.push(Signal::metadata("route.efe_scores", &efe_scores));
        out.push(Signal::metadata("route.modulation", modulation));
        out.push(Signal::metadata("route.behavioral_state", behavioral));

        Ok(out)
    }
}
```

### 3.3 Cost Multiplier Effect on Tier Selection

The cost multiplier shifts the EFE balance between tiers. When Struggling with Anxious affect, the cost multiplier drops to 0.6. This makes T2 (the expensive, thorough tier) appear cheaper relative to T0/T1, causing earlier escalation to the most capable model. The logic: when struggling, bring the big guns sooner rather than burning retries on weak models.

| State + Octant | Cost Mult | Effect | Approx Relative Cost |
|---|---|---|---|
| Engaged (baseline) | 1.0x | Normal EFE balance | 1.0x |
| Struggling/Anxious | 0.6x | T2 escalation at ~60% normal threshold | ~3.5x total spend |
| Struggling/Hostile | 0.4x | Aggressive T2 escalation + 3 retries | ~5x total spend |
| Coasting | 1.5x | T0/T1 preferred; explore via epistemic bonus | ~0.4x |
| Focused | 1.2x | T1 preferred; moderate spend | ~0.6x |
| Resting | 2.0x | T0 strongly preferred; minimal spend | ~0.2x |
| Exploring | 0.8x | Slightly lower barrier + exploration | ~1.2x |

---

## 4. VCG Bidding Modulation as a Functor on Compose

### 4.1 How Affect Adjusts Bids

The VCG attention auction in Compose assigns token budget to context items (knowledge entries, task context, research results, system prompt sections). Each bidder submits a value for its context contribution. Affect modulates these bids:

```rust
/// Affect modulation of VCG bid values.
///
/// Two adjustments:
/// 1. urgency_weight: 1.0 + arousal * 0.5
///    High arousal makes all bids appear more urgent.
///    Effect: reduces total context (fewer items win, but winners get more budget).
///
/// 2. affect_weight: 1.0 + 0.3 * |pleasure - 0.5|
///    Extreme affect (positive or negative) amplifies affect-related bids.
///    Effect: somatic markers and emotional context get more weight.
struct VcgAffectModulator {
    pad: PadContext,
    behavioral: BehavioralState,
}

impl VcgAffectModulator {
    fn modulate_bid(&self, bid: &mut ContextBid) {
        let urgency = 1.0 + self.pad.arousal * 0.5;
        let affect_amplification = 1.0 + 0.3 * (self.pad.pleasure - 0.5).abs();

        match bid.source {
            BidSource::Somatic | BidSource::AffectContext => {
                // Affect-related bids get both adjustments.
                bid.value *= urgency * affect_amplification;
            }
            BidSource::Knowledge | BidSource::Research => {
                // Knowledge bids get urgency adjustment only.
                bid.value *= urgency;
            }
            BidSource::TaskContext => {
                // Task context always gets urgency adjustment.
                bid.value *= urgency;
                // In Struggling state, task context bid is boosted
                // (focus on the immediate problem).
                if self.behavioral == BehavioralState::Struggling {
                    bid.value *= 1.3;
                }
            }
            BidSource::SystemPrompt => {
                // System prompt bids are not modulated (always included).
            }
        }
    }
}
```

### 4.2 Functor Integration

This modulation is part of the Daimon Functor's pre-enrichment on the Compose step (see [18-affect-as-functor.md](18-affect-as-functor.md) section 5.3). The Functor injects affect metadata into the Signal bundle before Compose runs. The Compose Cell reads `"daimon.vcg_modulation"` from Signal metadata and applies it during the auction:

```rust
// Inside AffectFunctor::pre_enrich for Compose step:
let modulator = VcgAffectModulator {
    pad,
    behavioral: BehavioralState::classify(pad),
};
enriched.push(Signal::metadata("daimon.vcg_modulation", modulator));
```

The Compose Cell then applies the modulation to each bid before running the VCG mechanism. This is not a special case in the Compose Cell -- it is a standard metadata read. The Compose Cell does not know about the Daimon. It reads modulation parameters from Signal metadata, regardless of their source.

---

## 5. The Four Integration Points as Bus Subscriptions

### 5.1 From Function Calls to Bus Topics

The v1 design has four integration points implemented as direct function calls from orchestrate.rs:
1. State -> self-model + TUI
2. State -> CascadeRouter tier thresholds
3. PAD -> VCG auction bidding
4. Strategy coords -> somatic landscape query

In the unified model, each integration point is a **Bus subscription**. No Cell calls another Cell's methods. They communicate through Pulses.

### 5.2 Integration Point 1: State -> Self-Model + TUI

The BehavioralStateScorer publishes `"behavioral.transition"` Pulses on Bus. Two subscribers react:

```rust
// Subscriber 1: CorticalState updater (Observe Cell)
// Topic: "behavioral.transition"
struct CorticalBehavioralObserver {
    cortical: Arc<CorticalState>,
}

impl Cell for CorticalBehavioralObserver {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::OBSERVE] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let transition = BehavioralTransition::from_signals(&input)?;

        // Atomic write to CorticalState.
        self.cortical.behavioral_state.store(transition.to);
        self.cortical.behavioral_scores.store(transition.scores);

        Ok(vec![]) // Observe Cells consume, do not produce.
    }
}

// Subscriber 2: TUI bridge (Observe Cell)
// Topic: "behavioral.transition"
struct TuiBehavioralObserver {
    tui_sender: mpsc::Sender<TuiEvent>,
}

impl Cell for TuiBehavioralObserver {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::OBSERVE] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let transition = BehavioralTransition::from_signals(&input)?;

        // Send to TUI render loop.
        self.tui_sender.send(TuiEvent::BehavioralStateChange {
            from: transition.from,
            to: transition.to,
            pad: transition.pad,
        }).await.ok(); // Non-blocking; TUI lag is tolerable.

        Ok(vec![])
    }
}
```

### 5.3 Integration Point 2: State -> Route Cell Thresholds

The Route Cell reads behavioral state from Signal metadata (set by the Daimon Functor). No direct subscription needed -- the Functor handles injection.

But the Route Cell does subscribe to `"behavioral.transition"` to pre-compute modulation parameters for the next tick:

```rust
// Topic: "behavioral.transition"
// Pre-computes routing modulation so the Route Cell's hot path is cheap.
struct RoutingPrecomputeObserver {
    modulation_cache: Arc<AtomicModulation>,
}

impl Cell for RoutingPrecomputeObserver {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::OBSERVE] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let transition = BehavioralTransition::from_signals(&input)?;
        let modulation = RoutingModulation::from_behavioral_state(
            transition.to,
            &transition.pad,
        );

        // Atomic write: Route Cell reads this on next tick without locking.
        self.modulation_cache.store(modulation);
        Ok(vec![])
    }
}
```

### 5.4 Integration Point 3: PAD -> VCG Auction Bidding

The Daimon Functor handles this as Compose pre-enrichment (section 4 above). No separate Bus subscription. The Functor reads CorticalState atomics and injects VCG modulation metadata into the Signal bundle before Compose runs.

### 5.5 Integration Point 4: Strategy Coords -> Somatic Landscape

The Daimon Functor handles this as Compose pre-enrichment as well (see [18-affect-as-functor.md](18-affect-as-functor.md) section 5.1). The strategy coordinates are computed from the task context and passed through Signal metadata. The Functor queries the somatic landscape and injects the result.

Additionally, the somatic landscape subscribes to task outcome Pulses to record new markers:

```rust
// Topic: "task.outcome.*"
// Records outcomes into the somatic landscape for future retrieval.
struct SomaticRecordingObserver {
    somatic: Arc<RwLock<SomaticMarkerCell>>,
}

impl Cell for SomaticRecordingObserver {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::REACT] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let outcome = TaskOutcome::from_signals(&input)?;
        let pad = read_affect_from_signals(&input)
            .unwrap_or(PadContext::NEUTRAL);
        let strategy_coords = StrategyCoordinates::from_signals(&input)
            .unwrap_or(StrategyCoordinates::neutral());

        // Record the outcome into the somatic landscape.
        let mut somatic = self.somatic.write().await;
        somatic.record_outcome(
            strategy_coords,
            pad.pleasure,       // valence from outcome affect
            pad.arousal.abs(),  // intensity from arousal magnitude
            outcome.episode_hash,
            outcome.timestamp,
        ).await?;

        Ok(input) // Pass through for downstream React Cells.
    }
}
```

### 5.6 Bus Topology Summary

```
               Verdict/Outcome Pulses
                       │
                       ▼
              ┌─────────────────┐
              │ Appraisal       │   React Cell
              │ Pipeline        │   (8-step, section 3 of doc 18)
              └────────┬────────┘
                       │ "affect.transition" Pulse
                       ▼
              ┌─────────────────┐
              │ Behavioral      │   Score Cell
              │ State Scorer    │   (this doc, section 2)
              └────────┬────────┘
                       │ "behavioral.transition" Pulse
                       ▼
         ┌─────────────┼─────────────┐
         │             │             │
         ▼             ▼             ▼
┌──────────────┐ ┌──────────┐ ┌──────────────┐
│ Cortical     │ │ TUI      │ │ Routing      │
│ Observer     │ │ Observer │ │ Precompute   │
│ (Observe)    │ │ (Observe)│ │ (Observe)    │
└──────────────┘ └──────────┘ └──────────────┘

         Task Outcome Pulses
                │
                ▼
       ┌─────────────────┐
       │ Somatic         │   React Cell
       │ Recording       │   (section 5.5)
       └─────────────────┘
```

Every arrow is a Bus subscription. No Cell imports another Cell. No function calls cross module boundaries. The wiring is declared in the Agent's Graph TOML, not hard-coded in orchestrate.rs.

---

## 6. Learnable Thresholds

### 6.1 The Learning Gap

The threshold values (e.g., Struggling entry at C<0.30) are initial estimates. They should improve as the agent accumulates experience. The feedback signal: gate pass rates per behavioral state.

If the agent enters Struggling and subsequently achieves a high gate pass rate (because the escalated model tier was correct), the entry threshold for Struggling should lower (enter sooner). If Struggling leads to gate failures despite escalation, the threshold should tighten (enter less readily, because escalation is not helping).

```rust
/// Learnable threshold adjustment based on gate outcomes per state.
struct ThresholdLearner {
    /// EMA of gate pass rate while in each state.
    pass_rate_per_state: HashMap<BehavioralState, EmaTracker>,
    /// Learning rate for threshold adjustment.
    learning_rate: f64,  // default: 0.01
}

impl ThresholdLearner {
    /// Update pass rate tracker after a gate verdict.
    fn observe_verdict(&mut self, state: BehavioralState, passed: bool) {
        let tracker = self.pass_rate_per_state
            .entry(state)
            .or_insert_with(|| EmaTracker::new(0.05)); // alpha=0.05

        tracker.update(if passed { 1.0 } else { 0.0 });
    }

    /// Suggest threshold adjustments based on accumulated evidence.
    ///
    /// If pass rate in Struggling > 0.7: widen entry threshold (enter sooner).
    /// If pass rate in Struggling < 0.3: tighten entry threshold (enter later).
    fn suggest_adjustments(&self) -> Vec<(BehavioralState, ThresholdAdjustment)> {
        let mut adjustments = Vec::new();

        for (state, tracker) in &self.pass_rate_per_state {
            let pass_rate = tracker.current();
            let target = 0.50; // neutral: state should lead to 50% pass rate

            let delta = (pass_rate - target) * self.learning_rate;

            if delta.abs() > 0.001 {
                adjustments.push((*state, ThresholdAdjustment {
                    // Positive delta: state is helpful, lower entry threshold
                    // Negative delta: state is not helpful, raise entry threshold
                    confidence_shift: -delta, // invert: lower threshold = easier entry
                    direction: if delta > 0.0 {
                        AdjustDirection::WidenEntry
                    } else {
                        AdjustDirection::TightenEntry
                    },
                }));
            }
        }

        adjustments
    }
}
```

### 6.2 Threshold Persistence

Learned threshold adjustments persist to Store as Signals with key `"behavioral.thresholds.{agent_id}"`. On agent resume, the BehavioralStateScorer loads these adjustments and applies them to its entry/exit thresholds.

This means: an agent that has been running for weeks has personalized behavioral state thresholds based on its own history. A fresh agent uses the defaults from section 2.1.

---

## 7. What This Enables

- **Behavioral state classification is a Cell.** It has internal state (hysteresis), it snapshots and resumes, it publishes transitions on Bus. No special treatment in the orchestrator.

- **Tier routing responds to affect without special-casing.** The Route Cell reads modulation parameters from Signal metadata. It does not import the Daimon module. Removing affect modulation means removing the Functor injection; the Route Cell's code does not change.

- **VCG bidding reacts to emotional context.** High arousal increases bid urgency (fewer context items, more focused). Struggling state boosts task context bids (focus on the immediate problem). These effects are parameters, not hardcoded branches.

- **All four integration points flow through Bus.** CorticalState, TUI, Route Cell, and somatic landscape all receive state updates via Pulse subscriptions. Adding a fifth integration point means adding a fifth subscriber -- no changes to existing Cells.

- **Thresholds can learn.** Gate pass rates per behavioral state provide a feedback signal for threshold adjustment. The system can discover that its initial Struggling threshold of C<0.30 should be C<0.35 for a particular agent's work domain.

- **Hysteresis prevents oscillation.** Asymmetric entry/exit thresholds plus 10-tick dwell minimum ensure stable behavioral state. No rapid flickering between states when PAD hovers near a boundary.

---

## 8. Feedback Loops

| Loop | Observes | Adjusts | Timescale |
|---|---|---|---|
| **PAD -> BehavioralState** | Effective PAD from CorticalState (via affect.transition Pulse) | Behavioral state classification (via Score Cell) | Per-affect-transition |
| **BehavioralState -> Routing** | behavioral.transition Pulse | Route Cell cost multiplier, epistemic bonus, retry budget | Per-routing decision (next tick) |
| **BehavioralState -> VCG** | Signal metadata from Daimon Functor | VCG bid urgency and affect weights | Per-Compose invocation |
| **Gate verdict -> Threshold learning** | verify.verdict Pulses correlated with current behavioral state | Entry/exit threshold adjustments for BehavioralStateScorer | Aggregated over sessions |
| **Task outcome -> Somatic recording** | task.outcome Pulses with PAD provenance | Somatic landscape markers at strategy coordinates | Per-task completion |
| **Somatic -> Routing** | Somatic signal from landscape query (via Functor) | Confidence multiplier on routing decisions (boost/reduce based on past experience) | Per-routing decision |
| **Behavioral state -> Compose budget** | Signal metadata from Daimon Functor | Token budget scaling (Struggling: 80%, Coasting: 110%) | Per-Compose invocation |

---

## 9. Open Questions

1. **State-dependent exploration rate.** The Coasting state sets `epistemic_bonus: 0.15` (encouraging exploration). Should this bonus be learnable? If Coasting exploration leads to gate failures (the agent was too adventurous), the bonus should decrease. If it leads to novel successful strategies, the bonus should increase. The feedback mechanism is similar to threshold learning (section 6), but applied to the exploration parameter.

2. **Multi-agent behavioral state coordination.** When agents work in a fleet, their behavioral states are independent. An Exploring agent might generate speculative work that a Struggling agent must validate. Should behavioral state transitions propagate between agents? The Bus-based design makes this trivial (subscribe to foreign `behavioral.transition` Pulses), but the policy question is whether coordination improves or hinders fleet performance.

3. **Behavioral state duration distributions.** The current model treats all dwell intervals equally. In practice, certain state durations correlate with better outcomes: short Struggling episodes (quick recovery) are healthy; long Struggling episodes (stuck) indicate a systemic problem. Should the BehavioralStateScorer track duration distributions and raise alerts (via Bus Pulse) when a state persists beyond its typical range?

4. **Vitality-behavioral state interaction.** The Agent doc ([05-AGENT.md](../../unified/05-AGENT.md)) defines 5 vitality phases (Thriving, Stable, Conservation, Declining, Terminal) that also modulate behavior. How do vitality phases and behavioral states compose? Currently they are independent. Should Conservation phase force-override Coasting to prevent waste? Should Terminal phase force-override all states to a shutdown behavior? The answer likely involves a priority hierarchy: vitality phase sets hard constraints, behavioral state sets soft preferences within those constraints.

5. **Scoring function weights.** The archetype-distance scoring (section 2.3) uses equal weights for all four dimensions (P, A, D, C). Should the weights differ per state? Struggling is primarily about low confidence (C), so the confidence dimension might deserve higher weight. Resting is primarily about low arousal (A). Weighted scoring would make state classification more sensitive to the dimension that matters most for each state.

---

## Cross-References

- [05-AGENT.md](../../unified/05-AGENT.md) SS3-4 -- Vitality phases, CorticalState atomic PAD and behavioral state
- [18-affect-as-functor.md](18-affect-as-functor.md) -- PAD as Signal metadata, appraisal Pipeline, ALMA loops, Functor injection
- [26-CROSS-CUTS.md](../../unified/26-CROSS-CUTS.md) SS4 -- F_daimon injection points on ASSESS and ACT
- [dual-process-and-efe-routing.md](dual-process-and-efe-routing.md) -- EFE formula, T0/T1/T2 tiers, affect-modulated routing
- [cognitive-energy-and-vitality.md](cognitive-energy-and-vitality.md) -- Energy zones and behavioral constraints
- [temperament-and-dual-process.md](temperament-and-dual-process.md) -- CascadeRouter, LinUCB bandits, Thompson sampling
- [cross-cut-functors.md](cross-cut-functors.md) -- VCG arbitration, natural transformations between cross-cuts
