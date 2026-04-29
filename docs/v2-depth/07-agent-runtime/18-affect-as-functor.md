# Affect as Functor

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How the Daimon affect engine emerges as a Signal endofunctor that enriches every Signal passing through the Agent's cognitive pipeline, rather than a standalone emotional state machine.

---

## 1. The Problem with Standalone Affect

The v1 Daimon is a standalone `DaimonState` struct that `orchestrate.rs` queries by calling methods: `daimon.current_pad()`, `daimon.behavioral_state()`, `daimon.retrieve_somatic_markers()`. This is the exact "ad-hoc state glue" pattern that the Mori lessons warn against. The affect engine lives outside the cognitive pipeline, coupled to the orchestrator via function calls rather than flowing through Bus.

The consequences:
1. **Testing requires the full orchestrator.** You cannot test affect modulation without standing up the entire plan runner.
2. **No composition.** Affect is hard-wired. You cannot disable it, swap it, or compose it with other cross-cuts without editing orchestrate.rs.
3. **Feedback loops are open.** The Daimon reads gate verdicts but does not publish its state changes as Pulses. Nothing downstream can react to an affect transition except code that explicitly reads `daimon.behavioral_state()`.
4. **PAD is invisible to Signals.** The PAD vector exists only inside DaimonState. Signals flowing through the pipeline carry no affect metadata. A Signal that was composed during an Anxious episode looks identical to one composed during Exuberant -- the affect context is lost.

The unified redesign dissolves the Daimon into the Signal stream: PAD becomes Signal metadata, appraisal becomes a React Cell, the ALMA temporal model becomes three nested Loops, and affect modulation becomes a Functor wrapping the Compose protocol.

---

## 2. PAD as Signal Metadata

### 2.1 The Core Insight

The PAD vector (Mehrabian 1996) is not the state of a module. It is metadata on a Signal -- the affective context in which the Signal was produced. Every Signal flowing through the Agent's cognitive pipeline carries an optional PAD annotation from the Agent that processed it.

```rust
/// Affect metadata carried by Signals through the cognitive pipeline.
/// Three orthogonal f64 dimensions, each in [-1.0, 1.0].
///
/// Mehrabian (1996): Pleasure-Arousal-Dominance model of emotion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PadContext {
    /// Outcome quality: positive experiences increase, failures decrease.
    pub pleasure: f64,
    /// Cognitive load / urgency: high arousal = high demand on resources.
    pub arousal: f64,
    /// Confidence in current approach: high dominance = committed strategy.
    pub dominance: f64,
    /// Motivational confidence in [0.0, 1.0], orthogonal to PAD.
    pub confidence: f64,
}

impl PadContext {
    pub const NEUTRAL: Self = Self {
        pleasure: 0.0,
        arousal: 0.0,
        dominance: 0.0,
        confidence: 0.5,
    };

    /// Classify into one of 8 octant states (Mehrabian 1996).
    pub fn octant(&self) -> AffectOctant {
        match (
            self.pleasure >= 0.0,
            self.arousal >= 0.0,
            self.dominance >= 0.0,
        ) {
            (true, true, true) => AffectOctant::Exuberant,
            (true, true, false) => AffectOctant::Dependent,
            (true, false, true) => AffectOctant::Relaxed,
            (true, false, false) => AffectOctant::Docile,
            (false, true, true) => AffectOctant::Hostile,
            (false, true, false) => AffectOctant::Anxious,
            (false, false, true) => AffectOctant::Disdainful,
            (false, false, false) => AffectOctant::Depressed,
        }
    }

    /// Euclidean distance between two PAD states (used for emission thresholds).
    pub fn distance(&self, other: &Self) -> f64 {
        let dp = self.pleasure - other.pleasure;
        let da = self.arousal - other.arousal;
        let dd = self.dominance - other.dominance;
        (dp * dp + da * da + dd * dd).sqrt()
    }
}
```

### 2.2 Signal Annotation

Signals carry PAD as an optional extension in their metadata map. The key is `"affect.pad"`. When a Cell within the Agent's pipeline produces a Signal, the Agent's Functor stamps it with the current PAD context:

```rust
/// Stamp a Signal with affect context from the Agent's current state.
fn stamp_affect(signal: &mut Signal, pad: &PadContext) {
    signal.metadata.insert(
        "affect.pad".into(),
        serde_json::to_value(pad).expect("PadContext is always serializable"),
    );
}

/// Read affect context from a Signal, if present.
fn read_affect(signal: &Signal) -> Option<PadContext> {
    signal
        .metadata
        .get("affect.pad")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}
```

This means:
- Signals composed during Anxious episodes carry the Anxious PAD stamp.
- Signals persisted to Store retain their affect context for replay and dream consolidation.
- When two Signals from different episodes are compared, their affect contexts are visible for somatic marker formation.

### 2.3 Affect Provenance on Pulses

Ephemeral Pulses on Bus carry a lighter annotation: just the octant and confidence, not the full PAD vector. This keeps Pulse overhead minimal while still allowing Bus subscribers to react to affect regime:

```rust
/// Lightweight affect annotation for Pulses.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PulseAffect {
    pub octant: AffectOctant,
    pub confidence: f64,
}
```

---

## 3. Appraisal as a React Cell

### 3.1 From Pipeline to Cell

The OCC-Scherer 8-step appraisal pipeline is currently implemented as a sequence of method calls inside `DaimonState::appraise()`. In the unified model, each step becomes a Cell in a Pipeline, and the Pipeline itself is registered as a React-protocol Cell that subscribes to verdict and task-outcome Pulses on Bus.

```rust
/// The appraisal Pipeline: 8 Cells in sequence.
/// Registered as a React Cell on Bus topics:
///   - "verify.verdict.*"   (gate pass/fail)
///   - "task.outcome.*"     (task success/failure)
///   - "agent.blocked.*"    (blocked notifications)
///   - "agent.pressure.*"   (time pressure events)
///
/// Input:  Pulse carrying event data
/// Output: Signal carrying PAD delta + optional somatic marker update
struct AppraisalPipeline {
    classify: ClassifyCell,
    ground: GroundCell,
    scale: ScaleCell,
    compute: ComputeCell,
    decay: DecayCell,
    apply: ApplyCell,
    persist: PersistCell,
    emit: EmitCell,
}
```

### 3.2 The Eight Cells

Each Cell takes a Signal bundle and produces a Signal bundle. The Pipeline composes them sequentially.

**Cell 1 -- CLASSIFY**: Determine event type from Pulse topic.

```rust
/// CLASSIFY: Map incoming Pulse to appraisal event type.
struct ClassifyCell;

impl Cell for ClassifyCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::REACT] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let pulse = Pulse::from_signals(&input)?;

        let event_type = match pulse.topic.as_str() {
            t if t.starts_with("verify.verdict.pass") => AppraisalEvent::GatePass,
            t if t.starts_with("verify.verdict.fail") => AppraisalEvent::GateFail,
            t if t.starts_with("task.outcome.success") => AppraisalEvent::TaskSuccess,
            t if t.starts_with("task.outcome.fail") => AppraisalEvent::TaskFail,
            t if t.starts_with("agent.blocked") => AppraisalEvent::Blocked,
            t if t.starts_with("agent.pressure") => AppraisalEvent::TimePressure,
            _ => return Ok(vec![]), // no appraisal for unrecognized events
        };

        Ok(vec![Signal::metadata("appraisal.event_type", event_type)])
    }
}
```

**Cell 2 -- GROUND**: Extract contextual factors (gate rung, task metadata, blocked count).

```rust
/// GROUND: Extract context needed for delta scaling.
struct GroundCell;

impl Cell for GroundCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let event_type = AppraisalEvent::from_signals(&input)?;
        let pulse = Pulse::from_signals(&input)?;

        let grounding = match event_type {
            AppraisalEvent::GatePass | AppraisalEvent::GateFail => {
                let rung = pulse.get::<u8>("gate.rung").unwrap_or(1);
                AppraisalGrounding {
                    rung_scale: 1.0 + (rung.min(3) as f64) * 0.15,
                    blocked_count: 0,
                    time_proximity: 0.0,
                }
            }
            AppraisalEvent::Blocked => {
                let n = pulse.get::<u32>("blocked.count").unwrap_or(1);
                AppraisalGrounding {
                    rung_scale: 1.0,
                    blocked_count: n,
                    time_proximity: 0.0,
                }
            }
            AppraisalEvent::TimePressure => {
                let prox = pulse.get::<f64>("pressure.proximity").unwrap_or(0.5);
                AppraisalGrounding {
                    rung_scale: 1.0,
                    blocked_count: 0,
                    time_proximity: prox,
                }
            }
            _ => AppraisalGrounding::default(),
        };

        let mut out = input;
        out.push(Signal::metadata("appraisal.grounding", grounding));
        Ok(out)
    }
}
```

**Cell 3 -- SCALE**: Compute raw PAD deltas from event type + grounding.

```rust
/// SCALE: Compute raw PAD deltas per appraisal rules.
/// Prospect theory: failures hit 2x harder than successes reward
/// (Kahneman & Tversky 1979).
struct ScaleCell;

impl Cell for ScaleCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let event = AppraisalEvent::from_signals(&input)?;
        let ground = AppraisalGrounding::from_signals(&input)?;
        let rs = ground.rung_scale;

        let (dp, da, dd, dc) = match event {
            // Gate pass: relief + confidence boost
            AppraisalEvent::GatePass => (
                0.05 * rs,   // P: positive
                -0.01 * rs,  // A: slight calm
                0.03 * rs,   // D: confidence
                0.03 * rs,   // C: motivational
            ),
            // Gate fail: prospect theory 2x asymmetry
            AppraisalEvent::GateFail => (
                -0.10 * rs,  // P: negative (2x gate pass)
                0.04 * rs,   // A: stress increase
                -0.08 * rs,  // D: confidence loss
                -0.08 * rs,  // C: motivational loss
            ),
            // Task success: strong positive
            AppraisalEvent::TaskSuccess => (0.10, 0.0, 0.10, 0.08),
            // Task fail: prospect theory 2x asymmetry
            AppraisalEvent::TaskFail => (-0.20, 0.0, -0.15, -0.15),
            // Blocked: arousal up, dominance down per blocker
            AppraisalEvent::Blocked => {
                let n = ground.blocked_count as f64;
                (0.0, n * 0.05, -n * 0.08, 0.0)
            }
            // Time pressure: arousal spike proportional to proximity
            AppraisalEvent::TimePressure => {
                let prox = ground.time_proximity;
                (0.0, prox * 0.40, 0.0, 0.0)
            }
        };

        let delta = PadDelta { pleasure: dp, arousal: da, dominance: dd, confidence: dc };
        let mut out = input;
        out.push(Signal::metadata("appraisal.delta", delta));
        Ok(out)
    }
}
```

**Cell 4 -- COMPUTE**: Apply delta to current ALMA emotion layer via EMA.

```rust
/// COMPUTE: Apply PAD delta to the emotion layer.
/// The emotion layer is the fastest ALMA timescale.
struct ComputeCell {
    alma: Arc<RwLock<AlmaState>>,
}

impl Cell for ComputeCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let delta = PadDelta::from_signals(&input)?;

        let mut alma = self.alma.write().await;
        let before = alma.emotion_layer().clone();

        // EMA update: emotion = (1-tau) * emotion + tau * (emotion + delta)
        let stimulus = PadContext {
            pleasure: (alma.emotion.pleasure + delta.pleasure).clamp(-1.0, 1.0),
            arousal: (alma.emotion.arousal + delta.arousal).clamp(-1.0, 1.0),
            dominance: (alma.emotion.dominance + delta.dominance).clamp(-1.0, 1.0),
            confidence: (alma.confidence + delta.confidence).clamp(0.0, 1.0),
        };
        alma.update_emotion(&stimulus);

        let after = alma.emotion_layer().clone();

        let mut out = input;
        out.push(Signal::metadata("appraisal.pad_before", before));
        out.push(Signal::metadata("appraisal.pad_after", after));
        Ok(out)
    }
}
```

**Cell 5 -- DECAY**: Apply temporal decay based on elapsed time since last appraisal.

```rust
/// DECAY: Apply half-life decay to PAD vector.
/// half_life = 4 hours; factor = 0.5^(elapsed / half_life)
struct DecayCell {
    half_life_hours: f64,  // default: 4.0
    alma: Arc<RwLock<AlmaState>>,
}

impl Cell for DecayCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let now = ctx.clock().now();

        let mut alma = self.alma.write().await;
        let elapsed_hours = alma.elapsed_since_last_decay(now);

        if elapsed_hours > 0.0 {
            let factor = 0.5_f64.powf(elapsed_hours / self.half_life_hours);
            alma.decay_toward_baseline(factor);
            alma.mark_decayed(now);
        }

        Ok(input)
    }
}
```

**Cell 6 -- APPLY**: Compute effective PAD from all three ALMA layers and write to CorticalState.

```rust
/// APPLY: Blend ALMA layers into effective PAD and write to CorticalState.
/// Effective = 0.5 * emotion + 0.3 * mood + 0.2 * personality
struct ApplyCell {
    alma: Arc<RwLock<AlmaState>>,
    cortical: Arc<CorticalState>,
}

impl Cell for ApplyCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let alma = self.alma.read().await;
        let effective = alma.effective_pad();

        // Atomic write to CorticalState: single-writer guarantee
        self.cortical.affect.pleasure.store(effective.pleasure);
        self.cortical.affect.arousal.store(effective.arousal);
        self.cortical.affect.dominance.store(effective.dominance);

        let mut out = input;
        out.push(Signal::metadata("appraisal.effective_pad", effective));
        Ok(out)
    }
}
```

**Cell 7 -- PERSIST**: Write the updated ALMA state to Store.

```rust
/// PERSIST: Write ALMA state to durable Store for resume.
struct PersistCell {
    alma: Arc<RwLock<AlmaState>>,
    store: Arc<dyn Store>,
}

impl Cell for PersistCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let alma = self.alma.read().await;
        let snapshot = alma.snapshot();

        self.store
            .write("daimon.alma", &snapshot)
            .await
            .map_err(CellError::store)?;

        Ok(input)
    }
}
```

**Cell 8 -- EMIT**: Publish a PAD transition Pulse on Bus if the Euclidean delta exceeds the emission threshold (0.15).

```rust
/// EMIT: Publish affect transition Pulse if delta exceeds threshold.
/// Emission threshold: PAD Euclidean distance > 0.15.
struct EmitCell {
    emission_threshold: f64,  // default: 0.15
}

impl Cell for EmitCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let before = PadContext::from_signal_key(&input, "appraisal.pad_before")?;
        let after = PadContext::from_signal_key(&input, "appraisal.effective_pad")?;

        if before.distance(&after) > self.emission_threshold {
            ctx.bus().publish(Pulse::new(
                "affect.transition",
                serde_json::json!({
                    "before": before,
                    "after": after,
                    "octant_before": before.octant(),
                    "octant_after": after.octant(),
                }),
            )).await?;
        }

        Ok(input)
    }
}
```

### 3.3 Bus Wiring

The AppraisalPipeline subscribes to verdict and outcome Pulses on Bus. It does not import the gate module or the task executor. It reacts to Pulses:

```
Bus topics subscribed:
  "verify.verdict.*"    -> AppraisalPipeline
  "task.outcome.*"      -> AppraisalPipeline
  "agent.blocked.*"     -> AppraisalPipeline
  "agent.pressure.*"    -> AppraisalPipeline

Bus topics published:
  "affect.transition"   <- EmitCell (when delta > 0.15)
```

This closes the feedback loop. Gate verdicts produce Pulses. The appraisal pipeline consumes those Pulses and produces affect transition Pulses. Downstream Cells (behavioral state classification, somatic marker formation) subscribe to affect transitions.

---

## 4. Three Nested Loops: ALMA Temporal Model

### 4.1 Three Timescales, Three Loops

The ALMA 3-layer temporal model (Gebhard 2005) maps to three nested Loops, each running at a different timescale. The key insight: these are not three update functions called from one loop. They are three independent Hot Graphs, each with its own clock, each reading from and writing to the shared AlmaState.

| Layer | Timescale | Clock | Loop Pattern | What It Does |
|---|---|---|---|---|
| **Emotion** (gamma-speed) | Seconds | Fires on every appraisal event | Inner Loop | EMA update with tau=0.1; immediate reaction |
| **Mood** (theta-speed) | Minutes to hours | Fires every `mood_interval` ticks (default 10) | Middle Loop | EMA of emotion layer with tau=0.5; smooths volatility |
| **Personality** (delta-speed) | Lifetime | Fires every `temperament_interval` ticks (default 100) | Outer Loop | EMA of mood layer with tau=0.9; stable baseline |

```rust
/// The three ALMA layers as nested Loop patterns.
///
/// Each layer is a Hot Graph with a feedback edge: the output of one
/// tick feeds the input of the next. The layers share AlmaState via
/// Arc<RwLock<_>>.
struct AlmaLoopSystem {
    /// Emotion: fires on every appraisal Pulse.
    emotion_loop: HotGraph,
    /// Mood: fires every mood_interval ticks.
    mood_loop: HotGraph,
    /// Personality: fires every temperament_interval ticks.
    personality_loop: HotGraph,
    /// Shared state written by all three loops.
    state: Arc<RwLock<AlmaState>>,
}

/// Shared state for all three ALMA layers.
struct AlmaState {
    /// Fast reactive layer.
    pub emotion: PadContext,
    /// Slow-moving mood.
    pub mood: PadContext,
    /// Near-static personality baseline.
    pub personality: PadContext,
    /// Motivational confidence (orthogonal to PAD).
    pub confidence: f64,
    /// Last decay timestamp.
    pub last_decay: DateTime<Utc>,
    /// Total ticks processed.
    pub tick_count: u64,
}

impl AlmaState {
    /// Effective PAD = weighted blend of all three layers.
    /// Weights: 0.5 emotion + 0.3 mood + 0.2 personality.
    pub fn effective_pad(&self) -> PadContext {
        PadContext {
            pleasure: 0.5 * self.emotion.pleasure
                + 0.3 * self.mood.pleasure
                + 0.2 * self.personality.pleasure,
            arousal: 0.5 * self.emotion.arousal
                + 0.3 * self.mood.arousal
                + 0.2 * self.personality.arousal,
            dominance: 0.5 * self.emotion.dominance
                + 0.3 * self.mood.dominance
                + 0.2 * self.personality.dominance,
            confidence: self.confidence,
        }
    }

    /// EMA update for the emotion layer.
    /// emotion = (1 - tau) * emotion + tau * stimulus
    pub fn update_emotion(&mut self, stimulus: &PadContext) {
        const TAU: f64 = 0.1;
        self.emotion = ema_blend(&self.emotion, stimulus, TAU);
    }

    /// EMA update for the mood layer from the current emotion.
    /// mood = (1 - tau) * mood + tau * emotion
    pub fn update_mood(&mut self) {
        const TAU: f64 = 0.5;
        self.mood = ema_blend(&self.mood, &self.emotion, TAU);
    }

    /// EMA update for the personality layer from the current mood.
    /// personality = (1 - tau) * personality + tau * mood
    pub fn update_personality(&mut self) {
        const TAU: f64 = 0.9;
        self.personality = ema_blend(&self.personality, &self.mood, TAU);
    }

    /// Apply temporal decay: move all layers toward neutral by factor.
    pub fn decay_toward_baseline(&mut self, factor: f64) {
        self.emotion = decay_pad(&self.emotion, factor);
        self.mood = decay_pad(&self.mood, factor);
        // Personality does not decay (it IS the baseline).
        self.confidence = 0.5 + (self.confidence - 0.5) * factor;
    }
}

fn ema_blend(current: &PadContext, target: &PadContext, tau: f64) -> PadContext {
    let retain = 1.0 - tau;
    PadContext {
        pleasure: (retain * current.pleasure + tau * target.pleasure).clamp(-1.0, 1.0),
        arousal: (retain * current.arousal + tau * target.arousal).clamp(-1.0, 1.0),
        dominance: (retain * current.dominance + tau * target.dominance).clamp(-1.0, 1.0),
        confidence: current.confidence, // confidence updated separately
    }
}

fn decay_pad(pad: &PadContext, factor: f64) -> PadContext {
    PadContext {
        pleasure: pad.pleasure * factor,
        arousal: pad.arousal * factor,
        dominance: pad.dominance * factor,
        confidence: pad.confidence,
    }
}
```

### 4.2 Inter-Loop Data Flow

The three loops communicate through the shared `AlmaState`, not through direct function calls:

```
   Appraisal Pulse ──► Emotion Loop (gamma)
                              │
                              │ writes emotion layer to AlmaState
                              ▼
                         Mood Loop (theta) reads emotion, writes mood
                              │
                              │ writes mood layer to AlmaState
                              ▼
                     Personality Loop (delta) reads mood, writes personality
```

Each loop reads the layer above it and writes its own layer. The `effective_pad()` blending function produces the unified PAD that CorticalState and downstream Cells see.

### 4.3 Loop Graphs in TOML

```toml
[graph]
name = "alma-emotion-loop"
version = "1.0.0"
hot = true

[[graph.nodes]]
id = "receive_delta"
cell = "roko.daimon.receive_appraisal_delta"
execution_class = "activity"

[[graph.nodes]]
id = "ema_update"
cell = "roko.daimon.emotion_ema_update"
execution_class = "activity"

[[graph.nodes]]
id = "write_cortical"
cell = "roko.daimon.write_cortical_pad"
execution_class = "activity"

[[graph.edges]]
from = "receive_delta"
to = "ema_update"

[[graph.edges]]
from = "ema_update"
to = "write_cortical"

# Feedback edge: output feeds next tick's input
[[graph.edges]]
from = "write_cortical"
to = "receive_delta"
feedback = true
```

The mood and personality loops follow the same pattern but fire on their respective cadences (every N emotion ticks).

---

## 5. Affect as a Functor

### 5.1 The Functorial Structure

Affect is a Functor in the precise sense defined in [26-CROSS-CUTS.md](../../unified/26-CROSS-CUTS.md): an endofunctor F_daimon: **Sig** -> **Sig** that wraps Cells with pre/post enrichment without changing the Graph's topology.

The primary injection point is the **Compose** protocol. F_daimon wraps Compose calls, enriching the context Signal with PAD information before the LLM receives it. The LLM sees affect-modulated context without any changes to the Compose Cell itself.

```rust
/// Daimon endofunctor: enriches Compose calls with affect context.
///
/// Pre-enrichment:
///   - Read current PAD from CorticalState
///   - Retrieve somatic markers for current strategy coordinates
///   - Inject affect metadata into the context Signal bundle
///   - Adjust VCG bid weights based on arousal and pleasure
///
/// Post-enrichment:
///   - Stamp output Signals with the PAD context in which they were produced
///   - Update somatic landscape with outcome signal
struct AffectFunctor {
    alma: Arc<RwLock<AlmaState>>,
    somatic: Arc<RwLock<SomaticLandscape>>,
    cortical: Arc<CorticalState>,
}

impl CrossCutFunctor for AffectFunctor {
    fn name(&self) -> &str { "daimon" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // 1. Read current effective PAD from CorticalState (atomic, <1us)
        let pad = PadContext {
            pleasure: self.cortical.affect.pleasure.load(),
            arousal: self.cortical.affect.arousal.load(),
            dominance: self.cortical.affect.dominance.load(),
            confidence: self.alma.read().await.confidence,
        };

        // 2. Inject PAD metadata into every input Signal
        let mut enriched: Vec<Signal> = input
            .into_iter()
            .map(|mut s| {
                stamp_affect(&mut s, &pad);
                s
            })
            .collect();

        // 3. Retrieve somatic markers for current strategy region
        let strategy_coords = ctx.get::<StrategyCoordinates>("task.strategy_coords")
            .unwrap_or(StrategyCoordinates::neutral());

        let somatic = self.somatic.read().await;
        let somatic_signal = somatic.query(strategy_coords, 5);

        if somatic_signal.is_actionable() {
            enriched.push(Signal::metadata("daimon.somatic", SomaticAnnotation {
                valence: somatic_signal.valence,
                intensity: somatic_signal.intensity,
                confidence_multiplier: somatic_confidence_bias(
                    somatic_signal.valence,
                    somatic_signal.intensity,
                ),
            }));
        }

        // 4. 15% mandatory contrarian retrieval (Bower 1981)
        let contrarian = somatic.query_contrarian(strategy_coords, 1);
        if !contrarian.source_episodes.is_empty() {
            enriched.push(Signal::metadata("daimon.contrarian", SomaticAnnotation {
                valence: contrarian.valence,
                intensity: contrarian.intensity,
                confidence_multiplier: somatic_confidence_bias(
                    contrarian.valence,
                    contrarian.intensity,
                ),
            }));
        }

        // 5. Inject behavioral state for downstream Cells
        let behavioral = BehavioralState::classify(pad);
        enriched.push(Signal::metadata("daimon.behavioral_state", behavioral));

        Ok(enriched)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let pad = PadContext {
            pleasure: self.cortical.affect.pleasure.load(),
            arousal: self.cortical.affect.arousal.load(),
            dominance: self.cortical.affect.dominance.load(),
            confidence: self.alma.read().await.confidence,
        };

        // Stamp all output Signals with the PAD context in which they were produced
        Ok(output
            .into_iter()
            .map(|mut s| {
                stamp_affect(&mut s, &pad);
                s
            })
            .collect())
    }
}
```

### 5.2 Where the Functor Injects

| Cognitive Step | Pre-enrichment | Post-enrichment |
|---|---|---|
| **SENSE** | None (affect does not filter perception) | Stamp with PAD |
| **ASSESS** | Inject PAD + behavioral state for tier selection bias | Override tier if safety condition (high anxiety + low dominance -> force T2) |
| **COMPOSE** | Inject PAD, somatic markers, contrarian retrieval, behavioral state; adjust VCG bid weights | Stamp composed context with PAD provenance |
| **ACT** | Check behavioral state for action gating (Anxious -> defer high-risk) | Stamp action result with PAD; compute prospect value |
| **VERIFY** | None (affect does not alter verification criteria) | None |
| **REACT** | None | Feed verdict outcome into appraisal pipeline via Bus Pulse |

### 5.3 Compose Enrichment in Detail

The Compose step is where affect has its deepest influence. The Functor modifies three aspects of prompt assembly:

```rust
/// How affect modifies context assembly during Compose.
struct AffectComposeModulation {
    /// VCG bid weight adjustments based on arousal and pleasure.
    ///
    /// urgency_weight = 1.0 + arousal * 0.5
    ///     High arousal increases urgency of all bids.
    ///
    /// affect_weight = 1.0 + 0.3 * |pleasure - 0.5|
    ///     Extreme pleasure (positive or negative) amplifies affect-related bids.
    pub urgency_weight: f64,
    pub affect_weight: f64,

    /// Token budget scaling by behavioral state.
    /// Coasting: allow 110% budget (exploratory surplus).
    /// Struggling: restrict to 80% (focus on essentials).
    /// Focused: baseline 100%.
    pub budget_scale: f64,

    /// Somatic marker confidence multiplier applied to knowledge entry bids.
    /// Positive somatic region: knowledge from this area bid 1.0-1.3x.
    /// Negative somatic region: knowledge from this area bid 0.7-1.0x.
    pub somatic_bid_multiplier: f64,
}

impl AffectComposeModulation {
    fn from_pad(pad: &PadContext, behavioral: BehavioralState) -> Self {
        let urgency_weight = 1.0 + pad.arousal * 0.5;
        let affect_weight = 1.0 + 0.3 * (pad.pleasure - 0.5).abs();

        let budget_scale = match behavioral {
            BehavioralState::Coasting => 1.10,
            BehavioralState::Struggling => 0.80,
            BehavioralState::Exploring => 1.05,
            BehavioralState::Resting => 0.90,
            _ => 1.00,
        };

        Self {
            urgency_weight,
            affect_weight,
            budget_scale,
            somatic_bid_multiplier: 1.0, // set separately from somatic query
        }
    }
}
```

### 5.4 Functor Composition Order

The affect Functor composes with other cross-cut Functors in a fixed order:

```
F_total = F_safety . F_daimon . F_memory . F_dreams
```

Safety is outermost (hard constraint). Daimon runs after Memory so that somatic markers can bias knowledge entries that Memory already retrieved. Dreams is innermost because dream-generated hypotheses should be subject to both affect gating and safety filtering.

---

## 6. Somatic Markers as Store Signals

### 6.1 From k-d Tree to Store

The existing `SomaticLandscape` uses an in-memory k-d tree. In the unified model, somatic markers ARE Signals in Store, indexed by their 8D strategy coordinates as HDC vectors. The k-d tree is a runtime cache rebuilt from Store on startup.

```rust
/// A somatic marker is a Signal with specific metadata.
///
/// metadata keys:
///   "somatic.strategy_coords": StrategyCoordinates (8D)
///   "somatic.valence": f64 in [-1.0, 1.0]
///   "somatic.intensity": f64 in [0.0, 1.0]
///   "somatic.source_episodes": Vec<ContentHash>
///   "somatic.updated_at": DateTime<Utc>
///
/// The Signal's content_hash uniquely identifies the marker.
/// Demurrage applies: unused markers fade over time.
/// Dream consolidation can depotentiate high-intensity negative markers.

struct SomaticMarkerCell {
    /// Runtime k-d tree cache (rebuilt from Store on init).
    tree: Arc<RwLock<KdTree<f64, 8>>>,
    /// Store reference for durable marker persistence.
    store: Arc<dyn Store>,
}

impl SomaticMarkerCell {
    /// Record a new outcome into the somatic landscape.
    ///
    /// If a marker exists within merge distance (squared Euclidean < 0.25)
    /// and has the same valence family, reinforce it. Otherwise create new.
    async fn record_outcome(
        &self,
        coords: StrategyCoordinates,
        valence: f64,
        intensity: f64,
        episode_hash: ContentHash,
        now: DateTime<Utc>,
    ) -> Result<(), CellError> {
        let tree = self.tree.read().await;
        let nearest = tree.nearest_one(&coords.as_array());

        if nearest.distance < 0.25 {
            // Reinforce existing marker (EMA update).
            // Load from Store, merge, write back.
            let existing = self.store.read_signal(&nearest.item).await?;
            let merged = merge_somatic(existing, valence, intensity, episode_hash, now);
            self.store.write_signal(&merged).await?;
        } else {
            // Create new marker Signal.
            let marker = Signal::new_somatic(coords, valence, intensity, episode_hash, now);
            self.store.write_signal(&marker).await?;
        }

        // Rebuild k-d tree cache.
        drop(tree);
        self.rebuild_cache().await?;
        Ok(())
    }

    /// Query the landscape: k nearest markers with contrarian blending.
    async fn query(
        &self,
        coords: StrategyCoordinates,
        k: usize,
    ) -> SomaticSignal {
        let tree = self.tree.read().await;
        let neighbors = tree.nearest_n(&coords.as_array(), k);

        // Aggregate congruent neighbors.
        let dominant_sign = dominant_valence_sign(&neighbors);
        let congruent = aggregate_by_sign(&neighbors, dominant_sign);

        // 15% contrarian blending (Bower 1981).
        let contrarian_count = ((k as f64) * 0.15).ceil() as usize;
        let contrarian = aggregate_contrarian(&neighbors, dominant_sign, contrarian_count);

        blend_congruent_and_contrarian(congruent, contrarian)
    }
}
```

### 6.2 Somatic Markers and Demurrage

Because somatic markers are Signals in Store, they participate in the demurrage system ([01-SIGNAL.md](../../unified/01-SIGNAL.md)). A somatic marker that is never queried (never reinforced) loses balance and eventually falls below the retention threshold. This is the mechanism by which old emotional memories fade -- not through a separate decay system, but through the same demurrage that governs all Signals.

When a somatic marker is queried during strategy assessment, its balance is refreshed (the query counts as "use"). Markers that are frequently relevant to current work stay alive; markers from abandoned domains fade naturally.

---

## 7. What This Enables

- **Affect is composable.** The Functor can be enabled, disabled, or swapped without modifying the cognitive pipeline Graph. An Agent without Daimon is the identity Functor at the affect injection point.

- **Affect is testable in isolation.** Test the appraisal Pipeline by feeding it synthetic verdict Pulses and checking the PAD delta output. No orchestrator required.

- **Every Signal carries its emotional provenance.** When dream consolidation replays an episode, it can see not just what happened but how the Agent felt. Somatic markers form from this provenance, not from a side-channel state query.

- **Feedback loops are closed via Bus.** Gate failure -> verdict Pulse -> appraisal Pipeline -> PAD delta -> CorticalState update -> behavioral state change -> Compose modulation on next tick. Every link is a Pulse or Signal, observable and logged.

- **ALMA temporal smoothing is emergent from Loop nesting.** The three timescales are not three update calls in one function. They are three independent Hot Graphs with independent failure isolation, budget accounting, and snapshot/resume.

- **Somatic markers participate in demurrage.** Old emotional memories fade through the same economic mechanism as all Signals. No separate garbage collection for affect.

---

## 8. Feedback Loops

| Loop | Observes | Adjusts | Timescale |
|---|---|---|---|
| **Appraisal -> Emotion** | Verdict Pulses, task outcome Pulses | Emotion layer PAD via EMA (tau=0.1) | Per-event (seconds) |
| **Emotion -> Mood** | Emotion layer state | Mood layer PAD via EMA (tau=0.5) | Every mood_interval ticks (minutes) |
| **Mood -> Personality** | Mood layer state | Personality layer PAD via EMA (tau=0.9) | Every temperament_interval ticks (hours) |
| **Effective PAD -> Compose** | CorticalState affect atomics | VCG bid weights, token budget scale, somatic bid multiplier | Per-Compose invocation |
| **Effective PAD -> Route** | CorticalState affect atomics | EFE tier selection bias (see [19-behavioral-states-and-routing.md](19-behavioral-states-and-routing.md)) | Per-Route invocation |
| **Somatic -> Compose** | k-d tree query at current strategy coords | Knowledge entry bid confidence multiplier (0.7-1.3x) | Per-Compose invocation |
| **Dream depotentiation -> Somatic** | High-PE episodes during delta consolidation | Somatic marker intensity reduction (Walker & van der Helm 2009) | Delta speed (hours) |
| **Compose outcome -> Somatic** | Gate verdicts for composed actions | Somatic marker reinforcement (pass) or weakening (fail) at strategy coords | Per-task completion |

---

## 9. Open Questions

1. **ALMA tau values as learnable parameters.** The current tau values (0.1, 0.5, 0.9) are fixed. Should they be learnable per-agent? An agent that works on volatile problems might benefit from faster mood decay. An agent on stable infrastructure might benefit from slower emotion reactivity. The feedback signal would be: "did the tau configuration correlate with better gate pass rates?"

2. **PAD metadata size budget.** Stamping every Signal with PAD context adds ~100 bytes of JSON metadata per Signal. For high-throughput pipelines (1000+ Signals per tick), this overhead may matter. Should PAD stamping be configurable per-pipeline, or should we use a compact binary encoding?

3. **Multi-agent affect contagion.** When agents share Signals via mesh sync, the PAD stamps travel with the Signals. Agent B receiving a Signal from Agent A's Anxious episode may be "infected" by the anxiety. Is this contagion desirable? The source material describes emotional contagion (roko-daimon contagion module). The Bus-based redesign makes contagion trivial to implement -- subscribe to foreign affect.transition Pulses -- but the question of whether to enable it by default remains open.

4. **Prospect theory lambda as a learnable parameter.** The 2x asymmetry (failures hit 2x harder than successes reward) matches Kahneman-Tversky lambda=2.2. But real coding agents may need different asymmetry ratios. An agent that is too loss-averse may refuse to attempt difficult tasks. Should lambda be part of the personality layer, learnable via the same feedback loops?

5. **Functor short-circuiting.** When the PAD state is near-neutral (all dimensions within 0.05 of zero), the Functor's pre/post enrichment adds no meaningful information. Should the Functor skip enrichment in this case to reduce overhead? The risk: if the Functor skips, Signals produced during neutral affect carry no PAD stamp, creating a gap in provenance.

---

## Cross-References

- [05-AGENT.md](../../unified/05-AGENT.md) SS1-4 -- Agent as Space + Extensions + Memory + clock + vitality; CorticalState atomic PAD
- [26-CROSS-CUTS.md](../../unified/26-CROSS-CUTS.md) SS2-4 -- CrossCutFunctor trait, F_daimon injection points, composition order
- [01-SIGNAL.md](../../unified/01-SIGNAL.md) SS4-5 -- Signal metadata, demurrage, HDC fingerprints
- [02-CELL.md](../../unified/02-CELL.md) SS2 -- React protocol, Pipeline pattern
- [03-GRAPH.md](../../unified/03-GRAPH.md) SS4 -- Hot Graph, feedback edges, Loop pattern
- [19-behavioral-states-and-routing.md](19-behavioral-states-and-routing.md) -- Behavioral state classification and affect-modulated routing
- [cross-cut-functors.md](cross-cut-functors.md) -- VCG arbitration between Memory, Daimon, and Dreams
- [cognitive-energy-and-vitality.md](cognitive-energy-and-vitality.md) -- Bidirectional energy-affect coupling
- [dual-process-and-efe-routing.md](dual-process-and-efe-routing.md) -- EFE routing formula with affect terms
