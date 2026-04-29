# Emergent Goals and Energy

> Depth for [28-emergent-goal-structures.md](../../docs/00-architecture/28-emergent-goal-structures.md) and [29-cognitive-energy-model.md](../../docs/00-architecture/29-cognitive-energy-model.md). Unifies emergent goals and cognitive energy into a single model where goals emerge from affect x knowledge x experience, and energy is the constraint that makes goals real.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, demurrage, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (Cell, Score protocol, Route protocol, React protocol), [04-SPECIALIZATIONS](../../unified/04-SPECIALIZATIONS.md) (Loop), [07-AGENT-RUNTIME](../../unified/07-AGENT-RUNTIME.md) (vitality, Daimon, somatic markers, EFE), [10-LEARNING-LOOPS](../../unified/10-LEARNING-LOOPS.md) (L1-L4 taxonomy)

---

## 1. The Unified Claim

Goals and energy are not separate systems. They are two sides of the same coin:

- **Goals** emerge from the intersection of what the agent wants (affect), what it knows (Memory), and what it has done (episodes). They are the "what to do next" signal.
- **Energy** determines whether the agent can pursue a goal. It is the constraint that turns desires into actions. You cannot pursue a goal without energy to spend.

The coupling is bidirectional: goals consume energy, and energy level influences which goals the agent can even consider. This creates natural rhythms of work and rest that mirror biological cognitive cycles.

```
         AFFECT (Daimon)           ENERGY
        "What do I want?"      "Can I afford it?"
             /      \                  |
            /        \                 |
           /   GOAL   \               |  Energy gates
          /  EMERGENCE \              |  which goals
         /              \             |  are feasible
  KNOWLEDGE     <->    EXPERIENCE     |
  (Memory)            (Episodes)      |
  "What do I know?"   "What have I done?"
```

---

## 2. Goal Emergence as a Cell

Goal emergence is a Cell that watches three input streams and produces candidate goals as output Signals. It runs during Theta reflections -- the medium-speed reasoning pass that reviews recent work.

```rust
/// Cell: goal emergence.
///
/// Watches three input streams:
///   1. Daimon affect state (Pulses on "daimon.state.*")
///   2. Memory queries (Store events, knowledge patterns)
///   3. Episode history (recent episodes from Learn)
///
/// Produces: candidate goals as Signals with Kind::Goal.
/// Runs at: Theta speed (per-task, 750ms to 16s).
pub struct GoalEmergenceCell {
    /// Active pattern detectors.
    detectors: Vec<Box<dyn GoalDetector>>,
    /// Minimum intrinsic motivation for a Nascent goal to survive.
    nascent_threshold: f64,    // default: 0.3
    /// Reinforcements required for Nascent -> Candidate promotion.
    reinforcement_threshold: u32,  // default: 3
    /// Maximum concurrent Active goals.
    max_active_goals: usize,   // default: 5
    /// HDC similarity threshold for merging duplicate goals.
    merge_threshold: f64,      // default: 0.85
}

/// A goal that emerged from the agent's internal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergentGoal {
    pub id: GoalId,
    pub description: String,
    pub sources: GoalSources,
    pub state: GoalState,
    pub priority: f64,
    pub intrinsic_motivation: f64,
    /// Expected free energy reduction if achieved.
    pub expected_efe_reduction: f64,
    /// Estimated energy cost to achieve.
    pub estimated_energy_cost: f64,
    pub sub_goals: Vec<GoalId>,
    pub parent: Option<GoalId>,
    pub created_at: SystemTime,
    pub last_evaluated: SystemTime,
    pub reinforcement_count: u32,
}

/// The three sources that converge to produce a goal.
pub struct GoalSources {
    pub affect: AffectSource,
    pub knowledge: KnowledgeSource,
    pub experience: ExperienceSource,
}

/// Goal lifecycle states.
///
///  Nascent -> Candidate -> Active -> { Achieved | Abandoned | Merged }
///               ^                          |
///               +--- (re-emerge) ----------+
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalState {
    Nascent,
    Candidate,
    Active,
    Achieved,
    Abandoned,
    Merged { into: GoalId },
}
```

### The Five Built-in Detectors

Each detector scans the agent's internal state for a specific pattern that suggests an unmet need or unexplored opportunity.

```rust
/// Knowledge Gap Detector
///
/// Pattern: Recent queries returned 0 results for a topic
///          referenced in multiple tasks.
/// Goal:    "Research [topic] to fill knowledge gap"
pub struct KnowledgeGapDetector { min_significance: f64 }

/// Quality Degradation Detector
///
/// Pattern: Verify pass rate trending downward over recent episodes.
/// Goal:    "Investigate and fix declining [gate_name] pass rate"
pub struct QualityDegradationDetector { lookback: usize, min_slope: f64 }

/// Curiosity Detector
///
/// Pattern: Agent has high arousal (Daimon) AND there is knowledge
///          with high novelty score that hasn't been explored.
/// Goal:    "Investigate [novel topic] -- high potential utility"
pub struct CuriosityDetector { min_arousal: f64, min_novelty: f64 }

/// Self-Maintenance Detector
///
/// Pattern: Consolidated-tier knowledge items that haven't been
///          validated by a Verify verdict in > N hours.
/// Goal:    "Re-validate stale knowledge about [topic]"
pub struct SelfMaintenanceDetector { max_unvalidated_hours: f64 }

/// Frustration Recovery Detector
///
/// Pattern: Low pleasure (Daimon) AND 3+ consecutive Verify failures
///          on similar tasks.
/// Goal:    "Change approach for [task pattern] -- current strategy failing"
pub struct FrustrationRecoveryDetector { min_failures: usize }
```

---

## 3. Intrinsic Motivation as a Score Protocol Cell

Intrinsic motivation scores candidate goals by how "interesting" they are. The score combines three factors (Schmidhuber 2010, Colas et al. 2022):

1. **Learning progress**: estimated knowledge gain (information-theoretic).
2. **Competence match**: how well the agent's skills match the goal difficulty (zone of proximal development).
3. **Affect alignment**: how well the goal aligns with current Daimon state.

```rust
/// Score Cell: intrinsic motivation.
///
/// IM = alpha * learning_progress * zpd_score
///    + beta  * competence_score
///    + gamma * affect_alignment
///
/// The zone of proximal development (Vygotsky 1978, operationalized by
/// Colas et al. 2022) peaks when the goal is at the boundary of the
/// agent's competence: challenging enough to learn from, achievable
/// enough to avoid frustration.
pub struct IntrinsicMotivationCell {
    alpha: f64,  // learning progress weight (default: 0.4)
    beta: f64,   // competence match weight (default: 0.35)
    gamma: f64,  // affect alignment weight (default: 0.25)
}

impl Cell for IntrinsicMotivationCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn name(&self) -> &str { "intrinsic-motivation" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let goal: EmergentGoal = extract_goal(&input[0])?;
        let energy: CognitiveEnergy = extract_energy(ctx)?;

        // Learning progress: how much will uncertainty decrease?
        let learning_potential = goal.sources.knowledge.confidence;

        // Zone of proximal development: peak motivation at the boundary
        // of competence. Not too easy (boring), not too hard (frustrating).
        let difficulty = goal.estimated_energy_cost / energy.max_energy;
        let competence = goal.sources.experience.historical_success_rate;
        let competence_gap = (difficulty - competence).abs();
        let zpd_score = (-competence_gap.powi(2) / 0.5).exp();

        // Competence match: historical success on similar goals
        let competence_score = goal.sources.experience.historical_success_rate;

        // Affect alignment: does this goal match what the agent wants?
        let affect_score = goal.sources.affect.somatic_marker.max(0.0);

        let im = self.alpha * learning_potential * zpd_score
            + self.beta * competence_score
            + self.gamma * affect_score;

        Ok(vec![Signal::new(
            Kind::Score,
            IntrinsicMotivationScore {
                goal_id: goal.id,
                score: im,
                zpd_score,
                learning_potential,
                competence_score,
                affect_score,
            },
        )])
    }
}
```

### Zone of Proximal Development

```
Motivation(goal)
      |
  1.0 |         /\
      |        /  \
  0.5 |       /    \
      |      /      \
  0.0 |-----/--------\------
      +-----------------------  Difficulty
            |    |    |
          Easy   ZPD   Hard
        (boring)     (frustrating)
```

Goals are most motivating when they sit at the competence boundary. This prevents the system from either always choosing easy goals (no learning) or always choosing hard goals (constant frustration).

---

## 4. Energy as the Feasibility Constraint

Energy determines what the agent can actually do. It is a **type-state on the Agent** -- the energy zone constrains which operations are available at any given moment.

### The Energy Pool

```rust
/// Cognitive energy pool for a single agent.
///
/// Energy differs from attention tokens:
///   - Attention tokens are spent and gone (like money).
///   - Energy depletes AND recovers (like biological stamina).
///   - Attention tokens buy specific operations; energy gates capability level.
pub struct CognitiveEnergy {
    pub current: f64,
    pub max_energy: f64,
    pub base_recovery_rate: f64,
    pub depletion_rate: f64,
    pub fatigue: f64,
    pub session_peak: f64,
    pub session_spent: f64,
}
```

### Five Energy Zones as Type-State

```rust
/// Energy zones constrain the agent's operating mode.
///
/// This is a type-state: the zone determines which operations are
/// available. Higher zones unlock more capabilities. Lower zones
/// force conservation and delegation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EnergyZone {
    /// 80-100%: Full capability. T2 allowed. Creative exploration.
    /// Max active goals: 5.
    Peak,
    /// 50-80%: Normal operation. All tiers allowed. Standard behavior.
    /// Max active goals: 3.
    Normal,
    /// 25-50%: Conservation mode. Prefer T0/T1. Avoid novel tasks.
    /// Max active goals: 2.
    Conserving,
    /// 10-25%: Low power. T0 only. Complete current task, defer new ones.
    /// Max active goals: 1.
    LowPower,
    /// 0-10%: Critical. Shutdown non-essential. Trigger Delta cycle.
    /// Max active goals: 0.
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

    pub fn max_inference_tier(&self) -> InferenceTier {
        match self {
            Self::Peak | Self::Normal => InferenceTier::T2,
            Self::Conserving => InferenceTier::T1,
            Self::LowPower | Self::Critical => InferenceTier::T0,
        }
    }

    pub fn max_active_goals(&self) -> usize {
        match self {
            Self::Peak => 5,
            Self::Normal => 3,
            Self::Conserving => 2,
            Self::LowPower => 1,
            Self::Critical => 0,
        }
    }

    pub fn should_trigger_delta(&self) -> bool {
        matches!(self, Self::Critical)
    }
}
```

### Per-Operation Energy Costs

Each cognitive operation has a base energy cost. Tired agents spend MORE energy on the same task (fatigue penalty).

```rust
/// Energy cost model.
///
/// Actual cost = base_cost * (1.0 + fatigue_penalty * (1.0 - energy_fraction))
///
/// At full energy: cost = base_cost * 1.0
/// At 50% energy:  cost = base_cost * 1.25
/// At 25% energy:  cost = base_cost * 1.375
/// At 10% energy:  cost = base_cost * 1.45
pub struct EnergyCostModel {
    pub t0_probe: f64,            // 0.1
    pub t1_inference: f64,        // 1.0
    pub t2_inference: f64,        // 5.0
    pub context_per_kb: f64,      // 0.05
    pub gate_eval: f64,           // 0.3
    pub theta_reflection: f64,    // 3.0
    pub delta_consolidation: f64, // 15.0
    pub goal_evaluation: f64,     // 0.5
    pub fatigue_penalty: f64,     // 0.5
}

impl EnergyCostModel {
    pub fn actual_cost(&self, base: f64, energy: &CognitiveEnergy) -> f64 {
        let fraction = energy.current / energy.max_energy.max(f64::EPSILON);
        let multiplier = 1.0 + self.fatigue_penalty * (1.0 - fraction);
        base * multiplier
    }
}
```

### Recovery Modes

Energy recovery mirrors the three cognitive speeds:

| Recovery mode | Rate (energy/sec) | When |
|---|---|---|
| Gamma | 0.05 | Active fast work ("catching your breath") |
| Theta | 0.3 | Reflection pass ("short break") |
| Delta | 2.0 | Consolidation cycle ("deep rest") |
| Idle | 0.5 | No active work |

Delta consolidation is "sleep": it costs energy initially (the consolidation process) but produces net positive recovery. Energy during Delta follows a dip-and-recover curve.

---

## 5. Goal Selection via Expected Free Energy (Route Protocol)

Goal selection is a **Route protocol Cell** that ranks candidate goals by expected free energy (EFE) reduction (Friston 2010). EFE captures both epistemic value (learning) and pragmatic value (task completion), discounted by energy cost.

```rust
/// Route Cell: goal selection via EFE.
///
/// Ranks candidate goals by expected free energy reduction.
/// The selected goal becomes the agent's next objective.
///
/// EFE(goal) = epistemic_value + pragmatic_value - energy_cost_penalty
///
/// Epistemic: how much will uncertainty decrease?
/// Pragmatic: how likely is success * priority?
/// Cost: energy cost normalized by current energy.
pub struct GoalSelectionRouteCell;

impl Cell for GoalSelectionRouteCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }
    fn name(&self) -> &str { "goal-selection-efe" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let candidates: Vec<EmergentGoal> = extract_candidate_goals(&input)?;
        let energy: CognitiveEnergy = extract_energy(ctx)?;
        let zone = EnergyZone::from_energy(&energy);

        // Filter by energy zone: cannot activate more goals than zone allows
        let current_active = ctx.store().count_active_goals().await?;
        if current_active >= zone.max_active_goals() {
            return Ok(vec![]); // no activation slot available
        }

        // Filter by energy feasibility: can we afford the goal?
        let feasible: Vec<&EmergentGoal> = candidates.iter()
            .filter(|g| g.estimated_energy_cost <= energy.current * 0.8)
            .collect();

        if feasible.is_empty() {
            return Ok(vec![]); // nothing affordable
        }

        // Rank by EFE
        let mut ranked: Vec<(&EmergentGoal, f64)> = feasible.iter()
            .map(|g| {
                let epistemic = g.sources.knowledge.confidence;
                let pragmatic = g.sources.experience.historical_success_rate * g.priority;
                let cost_penalty = (g.estimated_energy_cost / energy.max_energy).min(1.0);
                let efe = epistemic + pragmatic - cost_penalty;
                (*g, efe)
            })
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Activate the highest-EFE goal
        if let Some((goal, efe)) = ranked.first() {
            Ok(vec![Signal::new(
                Kind::GoalActivation,
                GoalActivation {
                    goal_id: goal.id,
                    efe_score: *efe,
                    energy_at_activation: energy.current,
                    zone_at_activation: zone,
                },
            )])
        } else {
            Ok(vec![])
        }
    }
}
```

---

## 6. Goal Conflict Arbitration (Route Protocol)

What happens when goals conflict? Two active goals that require mutually exclusive actions (e.g., "refactor the auth module" and "keep the auth module stable for release") need arbitration.

```rust
/// Route Cell: goal conflict arbitration.
///
/// When two active goals conflict, this Cell decides which to
/// pursue and which to defer or abandon.
///
/// Conflict detection: two goals conflict when their estimated
/// file sets overlap AND their success conditions are incompatible.
///
/// Resolution strategy:
///   1. Compare EFE scores. Higher EFE wins.
///   2. If EFE scores are within 10%: compare energy cost.
///      Cheaper goal wins (conserve energy).
///   3. If both criteria are tied: defer to the older goal
///      (more reinforcement = more validated).
pub struct GoalConflictArbitrationCell;

impl Cell for GoalConflictArbitrationCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }
    fn name(&self) -> &str { "goal-conflict-arbitration" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let active_goals: Vec<EmergentGoal> = extract_active_goals(&input)?;
        let mut outputs = Vec::new();

        // Detect conflicts: pairwise check for overlapping file sets
        // with incompatible success conditions
        for i in 0..active_goals.len() {
            for j in (i + 1)..active_goals.len() {
                let (goal_a, goal_b) = (&active_goals[i], &active_goals[j]);

                if goals_conflict(goal_a, goal_b) {
                    let (winner, loser) = arbitrate(goal_a, goal_b);

                    outputs.push(Signal::new(
                        Kind::GoalArbitration,
                        GoalArbitrationResult {
                            winner_id: winner.id,
                            loser_id: loser.id,
                            loser_action: if loser.reinforcement_count < 2 {
                                GoalAction::Abandon
                            } else {
                                GoalAction::Defer
                            },
                            reason: format!(
                                "Conflict on overlapping scope; {} has higher EFE",
                                winner.description
                            ),
                        },
                    ));
                }
            }
        }

        Ok(outputs)
    }
}

fn arbitrate<'a>(a: &'a EmergentGoal, b: &'a EmergentGoal) -> (&'a EmergentGoal, &'a EmergentGoal) {
    let efe_diff = (a.expected_efe_reduction - b.expected_efe_reduction).abs();
    if efe_diff > 0.1 * a.expected_efe_reduction.max(b.expected_efe_reduction) {
        // Clear EFE winner
        if a.expected_efe_reduction > b.expected_efe_reduction { (a, b) } else { (b, a) }
    } else if (a.estimated_energy_cost - b.estimated_energy_cost).abs() > f64::EPSILON {
        // EFE tied; cheaper wins
        if a.estimated_energy_cost < b.estimated_energy_cost { (a, b) } else { (b, a) }
    } else {
        // Everything tied; older goal wins (more reinforcement)
        if a.reinforcement_count >= b.reinforcement_count { (a, b) } else { (b, a) }
    }
}
```

---

## 7. Energy-Affect Coupling as a Loop

Energy and affect (Daimon) form a bidirectional coupling that creates a natural work-rest rhythm. This is a **Loop** (see [04-SPECIALIZATIONS.md](../../unified/04-SPECIALIZATIONS.md)) where energy changes affect, and affect changes energy dynamics.

```rust
/// Loop: energy-affect coupling.
///
/// Bidirectional:
///   Energy -> Affect: low energy reduces pleasure and dominance,
///                     critical energy increases arousal (stress).
///   Affect -> Energy: high pleasure reduces energy cost (work feels easier),
///                     high arousal increases energy consumption (excitement burns energy),
///                     high dominance improves recovery rate.
pub struct EnergyAffectLoop {
    // Energy -> Affect
    energy_to_pleasure: f64,        // 0.3
    energy_to_dominance: f64,       // 0.2
    critical_energy_arousal: f64,   // 0.4

    // Affect -> Energy
    pleasure_cost_discount: f64,    // 0.15
    arousal_cost_premium: f64,      // 0.1
    dominance_recovery_bonus: f64,  // 0.2
}

impl EnergyAffectLoop {
    /// Energy -> Affect: compute PAD delta from energy state.
    pub fn energy_to_pad(&self, energy: &CognitiveEnergy) -> PadDelta {
        let fraction = energy.current / energy.max_energy.max(f64::EPSILON);

        let pleasure = if fraction < 0.3 {
            -self.energy_to_pleasure * (1.0 - fraction / 0.3)
        } else { 0.0 };

        let dominance = if fraction < 0.4 {
            -self.energy_to_dominance * (1.0 - fraction / 0.4)
        } else { 0.0 };

        let arousal = if fraction < 0.15 {
            self.critical_energy_arousal * (1.0 - fraction / 0.15)
        } else { 0.0 };

        PadDelta { pleasure, arousal, dominance }
    }

    /// Affect -> Energy: modulate energy cost.
    pub fn affect_cost_modifier(&self, pad: &PadVector) -> f64 {
        let pleasure_mod = -self.pleasure_cost_discount * pad.pleasure.clamp(-1.0, 1.0);
        let arousal_mod = self.arousal_cost_premium * pad.arousal.clamp(0.0, 1.0);
        1.0 + pleasure_mod + arousal_mod
    }

    /// Affect -> Energy: modulate recovery rate.
    pub fn affect_recovery_modifier(&self, pad: &PadVector) -> f64 {
        let dominance_mod = self.dominance_recovery_bonus * pad.dominance.clamp(0.0, 1.0);
        1.0 + dominance_mod
    }
}
```

The Loop creates emergent behavior:
- **Virtuous cycle**: agent succeeds at a task -> pleasure rises -> energy cost decreases -> agent can do more -> more successes.
- **Protective cycle**: agent fails repeatedly -> pleasure drops -> energy cost increases -> agent slows down -> forced into Theta/Delta -> consolidation and recovery -> return with better strategy.
- **Stress response**: energy drops to critical -> arousal spikes -> agent recognizes the situation -> triggers Delta consolidation -> deep recovery.

---

## 8. Somatic Markers: Gut Feelings about Goals

Damasio's (1994) somatic marker hypothesis: emotions mark decisions with "gut feelings" that speed up evaluation. In unified terms, somatic markers are HDC fingerprints of past goal outcomes stored in Daimon state.

```rust
/// Somatic marker library: fast gut-feeling evaluation of goals.
///
/// When a new goal is proposed, its HDC fingerprint is compared
/// against the library. If a similar goal had a strongly positive
/// or negative outcome in the past, the marker provides instant
/// evaluation without full analysis.
///
/// This is the agent's "I have a bad feeling about this" or
/// "This feels like something I'm good at."
pub struct SomaticMarkerLibrary {
    markers: Vec<(HdcVector, f64)>,  // (fingerprint, valence)
    activation_threshold: f64,        // 0.75
}

impl SomaticMarkerLibrary {
    /// Evaluate a goal against somatic markers.
    /// Returns Some(valence) if a match is found, None otherwise.
    /// Valence in [-1.0, 1.0]: negative = bad feeling, positive = good feeling.
    pub fn evaluate(&self, goal_fingerprint: &HdcVector) -> Option<f64> {
        self.markers.iter()
            .filter_map(|(fp, valence)| {
                let similarity = hdc_cosine_similarity(fp, goal_fingerprint);
                if similarity > self.activation_threshold {
                    Some(valence * similarity)
                } else {
                    None
                }
            })
            .max_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap())
    }

    /// Record a new marker from a completed goal.
    pub fn record(&mut self, goal_fingerprint: HdcVector, outcome_valence: f64) {
        self.markers.push((goal_fingerprint, outcome_valence));
    }
}
```

Somatic markers feed into the `affect_alignment` component of intrinsic motivation (section 3). A goal with a strong negative somatic marker gets penalized before any deliberate analysis. A goal with a strong positive marker gets boosted. This is sub-millisecond evaluation via HDC similarity -- orders of magnitude faster than running an LLM.

---

## 9. Capacity Growth: The Agent Gets Fitter

Over time, successful work and effective consolidation increase the agent's energy capacity. The agent becomes "fitter" -- it can do more work before needing rest.

```rust
/// Long-term capacity growth model.
///
/// max_energy grows logarithmically with successful task completions.
/// This represents the agent becoming more efficient at its work.
/// Disuse decay prevents unbounded growth -- use it or lose it.
pub struct EnergyCapacityModel {
    base_capacity: f64,       // 100.0
    growth_per_delta: f64,    // 0.1
    growth_per_task: f64,     // 0.02
    capacity_ceiling: f64,    // 200.0
    disuse_decay: f64,        // 0.001 per hour
}

impl EnergyCapacityModel {
    pub fn update(
        &self,
        energy: &mut CognitiveEnergy,
        successful_tasks: u32,
        successful_deltas: u32,
        hours_idle: f64,
    ) {
        let growth = self.growth_per_task * successful_tasks as f64
            + self.growth_per_delta * successful_deltas as f64;
        let decay = self.disuse_decay * hours_idle;

        energy.max_energy = (energy.max_energy + growth - decay)
            .clamp(self.base_capacity, self.capacity_ceiling);
    }
}
```

---

## 10. The Goal Selection Algorithm

The full algorithm, showing how goals and energy interact at every step.

```
ALGORITHM: GoalSelection(engine, energy, daimon)

1. RUN all detectors -> collect new Nascent goals
   Energy cost: goal_evaluation * num_detectors

2. FOR EACH new Nascent goal:
   a. Compute intrinsic_motivation (IntrinsicMotivationCell)
   b. IF IM < nascent_threshold: discard
   c. Check for duplicates via HDC similarity
   d. IF duplicate (similarity > merge_threshold): merge into existing
   e. ELSE: add to goal store as Nascent

3. FOR EACH existing Nascent goal:
   a. Re-evaluate intrinsic_motivation
   b. IF IM dropped below threshold: remove (interest faded)
   c. IF reinforced this Theta tick: increment reinforcement_count
   d. IF reinforcement_count >= threshold: promote to Candidate

4. FOR EACH Candidate goal:
   a. Compute EFE (GoalSelectionRouteCell)
   b. CHECK energy zone: is an activation slot available?
      - zone.max_active_goals() > current_active_count?
   c. CHECK energy feasibility: can we afford this goal?
      - goal.estimated_energy_cost <= energy.current * 0.8?
   d. IF yes to both: activate highest-EFE Candidate
   e. Convert to plan/task and submit to orchestrator

5. FOR EACH Active goal:
   a. Check progress (Verify verdicts, episodes)
   b. IF completed: mark Achieved
      - Record somatic marker (positive valence)
      - Update energy capacity (growth_per_task)
   c. IF stalled (no progress in 5+ Theta ticks): evaluate Abandon
      - Record somatic marker (negative valence)
   d. IF conditions invalidated (knowledge changed): Abandon

6. CHECK for goal conflicts (GoalConflictArbitrationCell)
   a. Pairwise conflict detection on active goals
   b. Arbitrate via EFE -> energy cost -> reinforcement

7. UPDATE energy:
   a. Deduct energy for this Theta tick (theta_reflection cost)
   b. Apply affect cost modifier (pleasure/arousal)
   c. Check zone transition (did we cross a boundary?)
   d. IF zone == Critical: trigger Delta consolidation

8. RETURN updated goals, energy, daimon state
```

---

## What This Enables

1. **Autonomous initiative**: the agent generates its own goals from internal state, bridging the gap between reactive task execution and autonomous agency.
2. **Natural work-rest rhythms**: energy zones create automatic pacing. The agent slows down before it degrades, rests during Delta, and returns refreshed.
3. **Energy-gated feasibility**: goals that are too expensive for the current energy level are filtered before any expensive analysis.
4. **Conflict resolution**: when goals compete, the Route protocol arbitrates by EFE, energy cost, and reinforcement history.
5. **Learning from outcomes**: somatic markers provide sub-millisecond gut-feeling evaluation of new goals based on HDC similarity to past outcomes.
6. **Capacity growth**: the agent becomes more capable over time through successful work and effective consolidation.

## Feedback Loops

- **L1**: energy cost parameters adjust via EMA based on actual depletion vs predicted depletion.
- **L2**: goal selection EFE model updates beliefs about which goal types succeed in which energy zones.
- **L3**: Delta consolidation compresses goal outcomes into somatic markers and updates the capacity model.
- **L4**: structural proposals to adjust detector thresholds, motivation weights, or zone boundaries based on sustained patterns.
- **Energy-affect Loop**: the bidirectional coupling between energy and Daimon state creates self-regulating work-rest cycles.

## Open Questions

1. **Multi-agent energy**: when multiple agents share a workspace, should energy be individually managed or collectively pooled? Hockey (2011) suggests individual management with collective monitoring. But goal conflicts across agents need a shared arbitration mechanism.
2. **Energy lending**: can a high-energy agent "lend" energy to a depleted agent? This would require an energy transfer protocol and raises questions about exploitation.
3. **Goal inheritance across sessions**: should goals persist across sessions? Nascent goals are cheap to regenerate. Active goals have context that is expensive to rebuild. The answer probably depends on how much session state is preserved.
4. **Circadian rhythms**: should the base recovery rate vary with time of day to match the user's work patterns? A system that matches the human operator's rhythm would be more useful.
5. **Energy as intrinsic reward**: should energy recovery itself be an intrinsic reward in the goal emergence engine? "I'm tired -> Goal: rest" as emergent behavior. The current design triggers Delta at Critical zone, but earlier, voluntary rest might be more efficient.
6. **Goal quality over time**: as the agent develops, do its emergent goals improve? Can we measure goal quality independent of goal achievement? A possible metric: the ratio of achieved to abandoned goals over time.
