# Emergent Goal Structures

> **Abstract:** Goals in Roko are not predefined hierarchies — they emerge from the interaction
> of three forces: affect (what the agent *wants*), knowledge (what the agent *knows*), and
> experience (what the agent has *done*). This document specifies a goal emergence engine that
> synthesizes goals from Daimon affect signals, Neuro knowledge patterns, and episodic learning
> data. Goals form, compete, merge, split, and die naturally — creating an autotelic agent that
> generates its own objectives rather than merely executing assigned tasks.

> **Implementation**: Specified

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [09-universal-cognitive-loop](./09-universal-cognitive-loop.md), [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md), [08-scorer-gate-router-composer-policy](./08-scorer-gate-router-composer-policy.md)
**Key sources**:
- arXiv:2505.17621 (2025) — IMAGINE: Intrinsic Motivation Guided Exploration
- arXiv:2508.18420 (2025) — LLM-Driven Intrinsic Motivation for Sparse Reward RL
- arXiv:2410.23022 (2024) — Online Intrinsic Rewards from LLM Feedback
- Colas et al. 2022, JMLR — IMGEP: Autotelic Agents with Intrinsically Motivated Goal Exploration
- Schmidhuber 2010, IEEE TNN — Formal Theory of Creativity, Fun, and Intrinsic Motivation
- Damasio 1994, "Descartes' Error" — Somatic Marker Hypothesis
- Friston 2010, Nature Reviews Neuroscience 11(2) — Free Energy Principle

---

## 1. The Problem: Static Goals in a Dynamic System

Roko currently operates with externally assigned goals: PRDs define what to build, plans
define how to build it, and tasks define the individual steps. The agent executes — it does
not *want*. This creates three limitations:

1. **No initiative**: The agent cannot identify that a test suite is degrading and
   spontaneously create a "fix flaky tests" goal.
2. **No curiosity**: The agent cannot notice an unexplored API and generate a
   "investigate this for potential utility" goal.
3. **No self-maintenance**: The agent cannot sense its own knowledge decay and generate
   a "consolidate and verify stale knowledge" goal.

Emergent goal structures solve these by allowing goals to arise naturally from the agent's
internal state, bridging the gap between reactive task execution and autonomous agency.

---

## 2. Three Sources of Goal Emergence

### 2.1 The Emergence Triangle

```
           AFFECT (Daimon)
          "What do I want?"
              /      \
             /        \
            /   GOAL   \
           /  EMERGENCE \
          /              \
KNOWLEDGE (Neuro)  ←→  EXPERIENCE (Learn)
"What do I know?"    "What have I done?"
```

Goals emerge at the intersection:

| Source Pair | Emergence Pattern | Example |
|---|---|---|
| Affect × Knowledge | Desire meets opportunity | "I'm frustrated (low pleasure) AND I know the tests are flaky → Goal: fix flaky tests" |
| Affect × Experience | Desire meets capability | "I'm curious (high arousal) AND I've successfully explored APIs before → Goal: investigate new API" |
| Knowledge × Experience | Opportunity meets capability | "I know the docs are stale AND I've written docs successfully → Goal: update documentation" |
| All three | Full convergence | "I want to improve (affect) + I know what's broken (knowledge) + I've done similar fixes (experience) → Goal: refactor auth module" |

### 2.2 Core Types

```rust
/// A goal that emerged from the agent's internal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergentGoal {
    pub id: GoalId,
    /// Human-readable description of what the goal aims to achieve.
    pub description: String,
    /// The emergence sources that created this goal.
    pub sources: GoalSources,
    /// Current lifecycle state.
    pub state: GoalState,
    /// Priority score (computed from source strengths).
    pub priority: f64,
    /// Intrinsic motivation score: how "interesting" is this goal?
    pub intrinsic_motivation: f64,
    /// Expected free energy reduction if this goal is achieved.
    pub expected_efe_reduction: f64,
    /// Estimated attention tokens to achieve.
    pub estimated_cost: AttentionToken,
    /// Sub-goals (if decomposed).
    pub sub_goals: Vec<GoalId>,
    /// Parent goal (if this is a sub-goal).
    pub parent: Option<GoalId>,
    /// Creation timestamp.
    pub created_at: SystemTime,
    /// Last evaluation timestamp.
    pub last_evaluated: SystemTime,
    /// Number of Theta reflections that have reinforced this goal.
    pub reinforcement_count: u32,
}

/// The three sources that contribute to goal emergence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalSources {
    /// Affect signal: what Daimon state contributed?
    pub affect: AffectSource,
    /// Knowledge signal: what knowledge pattern was detected?
    pub knowledge: KnowledgeSource,
    /// Experience signal: what episodic pattern was detected?
    pub experience: ExperienceSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectSource {
    /// Which PAD dimension(s) triggered this goal.
    pub pad_trigger: PadVector,
    /// Which behavioral state the agent was in.
    pub behavioral_state: BehavioralState,
    /// Somatic marker strength (Damasio 1994): gut feeling about this goal.
    pub somatic_marker: f64,  // -1.0 (bad feeling) to 1.0 (good feeling)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSource {
    /// Knowledge pattern that contributed (e.g., "gap detected", "contradiction found").
    pub pattern: KnowledgePattern,
    /// Specific Engrams that evidenced the pattern.
    pub evidence: Vec<ContentHash>,
    /// Confidence in the knowledge signal.
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceSource {
    /// Relevant past episodes.
    pub episodes: Vec<Uuid>,
    /// Success rate on similar past goals.
    pub historical_success_rate: f64,
    /// Average attention cost of similar past goals.
    pub historical_avg_cost: f64,
}

pub type GoalId = Uuid;
```

### 2.3 Goal Lifecycle

```rust
/// Goal states forming a lifecycle.
///
///  Nascent → Candidate → Active → { Achieved | Abandoned | Merged }
///                ↑                       |
///                └───── (re-emerge) ─────┘
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalState {
    /// Just emerged; needs reinforcement before becoming a candidate.
    Nascent,
    /// Reinforced by multiple Theta reflections; eligible for activation.
    Candidate,
    /// Currently being pursued (converted to a plan/task).
    Active,
    /// Successfully achieved.
    Achieved,
    /// Abandoned (cost too high, conditions changed, or contradicted by new knowledge).
    Abandoned,
    /// Merged into another goal (detected as duplicate or sub-goal).
    Merged { into: GoalId },
}
```

---

## 3. Goal Emergence Engine

### 3.1 Architecture

```rust
/// The goal emergence engine runs during Theta reflections.
///
/// It scans the agent's internal state for patterns that indicate
/// unmet needs, unexplored opportunities, or self-maintenance tasks.
pub struct GoalEmergenceEngine {
    /// Active pattern detectors.
    pub detectors: Vec<Box<dyn GoalDetector>>,
    /// The goal store (all goals in all states).
    pub goals: GoalStore,
    /// Minimum intrinsic motivation for a Nascent goal to survive.
    pub nascent_threshold: f64,  // default: 0.3
    /// Reinforcements required to promote Nascent → Candidate.
    pub reinforcement_threshold: u32,  // default: 3
    /// Maximum concurrent Active goals.
    pub max_active_goals: usize,  // default: 5
    /// Goal merge similarity threshold (HDC fingerprint cosine).
    pub merge_threshold: f64,  // default: 0.85
}

/// A detector that scans internal state for goal-worthy patterns.
pub trait GoalDetector: Send + Sync {
    /// Name of this detector (for logging).
    fn name(&self) -> &str;

    /// Scan the agent's state and return zero or more candidate goals.
    fn detect(
        &self,
        affect: &DaimonState,
        knowledge: &dyn Substrate,
        episodes: &[Episode],
        context: &Context,
    ) -> Vec<EmergentGoal>;
}
```

### 3.2 Built-in Detectors

```rust
/// Detector: Knowledge gaps (things the agent should know but doesn't).
pub struct KnowledgeGapDetector {
    /// Minimum gap significance to trigger goal.
    pub min_significance: f64,  // default: 0.5
}

impl GoalDetector for KnowledgeGapDetector {
    fn name(&self) -> &str { "knowledge-gap" }

    fn detect(&self, affect: &DaimonState, knowledge: &dyn Substrate,
              episodes: &[Episode], ctx: &Context) -> Vec<EmergentGoal> {
        // Pattern: Recent queries returned 0 results for a topic that
        // was referenced in multiple tasks.
        // → Goal: "Research [topic] to fill knowledge gap"
        todo!("scan recent queries for systematic misses")
    }
}

/// Detector: Quality degradation (things that are getting worse).
pub struct QualityDegradationDetector {
    /// Lookback window (Theta ticks).
    pub lookback: usize,  // default: 10
    /// Minimum degradation slope to trigger goal.
    pub min_slope: f64,  // default: -0.1
}

impl GoalDetector for QualityDegradationDetector {
    fn name(&self) -> &str { "quality-degradation" }

    fn detect(&self, affect: &DaimonState, knowledge: &dyn Substrate,
              episodes: &[Episode], ctx: &Context) -> Vec<EmergentGoal> {
        // Pattern: Gate pass rate trending downward over recent episodes.
        // → Goal: "Investigate and fix declining [gate_name] pass rate"
        todo!("compute gate pass rate slope from recent episodes")
    }
}

/// Detector: Curiosity (unexplored high-potential areas).
pub struct CuriosityDetector {
    /// Minimum arousal for curiosity-driven goals.
    pub min_arousal: f64,  // default: 0.3
    /// Minimum novelty score for the knowledge item.
    pub min_novelty: f64,  // default: 0.6
}

impl GoalDetector for CuriosityDetector {
    fn name(&self) -> &str { "curiosity" }

    fn detect(&self, affect: &DaimonState, knowledge: &dyn Substrate,
              episodes: &[Episode], ctx: &Context) -> Vec<EmergentGoal> {
        // Pattern: Agent has high arousal AND there is knowledge with
        // high novelty score that hasn't been explored.
        // → Goal: "Investigate [novel topic] — high potential utility"
        todo!("find high-novelty unexplored knowledge items")
    }
}

/// Detector: Self-maintenance (knowledge decay, stale heuristics).
pub struct SelfMaintenanceDetector {
    /// Maximum age of unvalidated Consolidated knowledge (hours).
    pub max_unvalidated_hours: f64,  // default: 168 (7 days)
}

impl GoalDetector for SelfMaintenanceDetector {
    fn name(&self) -> &str { "self-maintenance" }

    fn detect(&self, affect: &DaimonState, knowledge: &dyn Substrate,
              episodes: &[Episode], ctx: &Context) -> Vec<EmergentGoal> {
        // Pattern: Consolidated-tier knowledge items that haven't been
        // validated by a gate verdict in > max_unvalidated_hours.
        // → Goal: "Re-validate stale knowledge about [topic]"
        todo!("find stale Consolidated knowledge items")
    }
}

/// Detector: Frustration recovery (repeated failures on same pattern).
pub struct FrustrationRecoveryDetector {
    /// Minimum consecutive failures to trigger.
    pub min_failures: usize,  // default: 3
}

impl GoalDetector for FrustrationRecoveryDetector {
    fn name(&self) -> &str { "frustration-recovery" }

    fn detect(&self, affect: &DaimonState, knowledge: &dyn Substrate,
              episodes: &[Episode], ctx: &Context) -> Vec<EmergentGoal> {
        // Pattern: Low pleasure (frustration) AND 3+ consecutive gate
        // failures on similar tasks.
        // → Goal: "Change approach for [task pattern] — current strategy failing"
        todo!("detect repeated failure patterns in episodes")
    }
}
```

---

## 4. Intrinsic Motivation Scoring

### 4.1 The Motivation Function

Intrinsic motivation combines three factors (Schmidhuber 2010; Colas et al. 2022):

```rust
/// Compute intrinsic motivation for a candidate goal.
///
/// IM = α × learning_progress + β × competence_match + γ × affect_alignment
///
/// - learning_progress: estimated knowledge gain (information-theoretic)
/// - competence_match: how well the agent's skills match the goal difficulty
/// - affect_alignment: how well the goal aligns with current Daimon state
pub fn intrinsic_motivation(
    goal: &EmergentGoal,
    agent_competence: f64,  // estimated from historical success rate
    learning_potential: f64, // estimated from knowledge gap size
    affect_alignment: f64,   // computed from Daimon state
) -> f64 {
    const ALPHA: f64 = 0.4;  // learning progress weight
    const BETA: f64 = 0.35;  // competence match weight
    const GAMMA: f64 = 0.25; // affect alignment weight

    // Learning progress: higher when goal is in the "zone of proximal development"
    // — not too easy (boring), not too hard (frustrating).
    let difficulty = goal.estimated_cost.value() / 1000.0;  // normalize
    let competence_gap = (difficulty - agent_competence).abs();
    let zpd_score = (-competence_gap * competence_gap / 0.5).exp();

    // Competence match: higher when agent has relevant experience.
    let competence_score = goal.sources.experience.historical_success_rate;

    // Affect alignment: higher when goal matches current emotional needs.
    let affect_score = affect_alignment.max(0.0);

    ALPHA * learning_potential * zpd_score
        + BETA * competence_score
        + GAMMA * affect_score
}
```

### 4.2 Zone of Proximal Development

Inspired by Vygotsky (1978) and operationalized by Colas et al. (2022, IMGEP), goals are
most motivating when they are at the boundary of the agent's competence — challenging enough
to learn from, but achievable enough to avoid frustration:

```
Motivation(goal)
      │
  1.0 │         ╱╲
      │        ╱  ╲
  0.5 │       ╱    ╲
      │      ╱      ╲
  0.0 │─────╱────────╲──────
      └───────────────────── Difficulty
            │    │    │
          Easy   ZPD   Hard
        (boring)     (frustrating)
```

---

## 5. Goal Competition and Selection

### 5.1 Expected Free Energy (EFE) Ranking

Goals are ranked by expected free energy reduction (Friston 2010). EFE captures both
*epistemic value* (how much the agent will learn) and *pragmatic value* (how much the
agent will accomplish):

```rust
/// Expected Free Energy for goal selection.
///
/// EFE(g) = epistemic_value(g) + pragmatic_value(g)
///
/// Lower EFE = better goal (minimizing free energy).
/// We negate for ranking: higher rank = lower EFE.
pub fn expected_free_energy(
    goal: &EmergentGoal,
    current_knowledge_entropy: f64,
    predicted_post_knowledge_entropy: f64,
    task_completion_probability: f64,
) -> f64 {
    // Epistemic: how much will uncertainty decrease?
    let epistemic = current_knowledge_entropy - predicted_post_knowledge_entropy;

    // Pragmatic: how likely is the goal to succeed × its value?
    let pragmatic = task_completion_probability * goal.priority;

    // Cost-adjusted: discount by estimated attention cost.
    let cost_penalty = (goal.estimated_cost.value() / 10_000.0).min(1.0);

    epistemic + pragmatic - cost_penalty
}
```

### 5.2 Goal Selection Algorithm

```
ALGORITHM: GoalSelection(engine, budget)

1. Run all detectors → collect new Nascent goals
2. For each new Nascent goal:
   a. Compute intrinsic_motivation
   b. If IM < nascent_threshold: discard (not interesting enough)
   c. Check for duplicates via HDC similarity against existing goals
   d. If duplicate (similarity > merge_threshold): merge into existing
   e. Else: add to goal store as Nascent

3. For each existing Nascent goal:
   a. Re-evaluate intrinsic_motivation (conditions may have changed)
   b. If IM dropped below nascent_threshold: remove (interest faded)
   c. If reinforced by this Theta tick: increment reinforcement_count
   d. If reinforcement_count >= threshold: promote to Candidate

4. For each Candidate goal:
   a. Compute EFE
   b. Check if activation slot available (< max_active_goals)
   c. If yes: activate highest-EFE Candidate
   d. Convert to plan/task and submit to orchestrator

5. For each Active goal:
   a. Check progress (gate verdicts, episodes)
   b. If completed: mark Achieved, record in immune memory
   c. If stalled (no progress in 5+ Theta ticks): evaluate Abandon
   d. If conditions invalidated (knowledge changed): Abandon

6. Return updated goal store + newly activated goals
```

---

## 6. Goal Decomposition

When an activated goal is too large for a single task, the engine decomposes it into sub-goals:

```rust
/// Decompose a complex goal into sub-goals.
///
/// Strategy:
/// 1. Check if historical episodes contain a similar goal that was decomposed.
/// 2. If yes: use the historical decomposition as a template.
/// 3. If no: use LLM to generate decomposition (T2 inference).
pub struct GoalDecomposer {
    /// Maximum sub-goals per parent.
    pub max_sub_goals: usize,  // default: 7 (Miller's number)
    /// Minimum goal size that triggers decomposition (estimated AT).
    pub decomposition_threshold: f64,  // default: 5000.0 AT
}

impl GoalDecomposer {
    pub fn should_decompose(&self, goal: &EmergentGoal) -> bool {
        goal.estimated_cost.value() > self.decomposition_threshold
    }

    pub fn decompose(
        &self,
        goal: &EmergentGoal,
        knowledge: &dyn Substrate,
        episodes: &[Episode],
    ) -> Vec<EmergentGoal> {
        // Sub-goals inherit parent's sources but with refined scope
        // Each sub-goal gets a fraction of the parent's estimated cost
        // Dependencies between sub-goals form a DAG (like plan tasks)
        todo!("historical template matching or LLM decomposition")
    }
}
```

---

## 7. Somatic Marker Integration

Damasio's (1994) somatic marker hypothesis: emotions mark decisions with "gut feelings" that
speed up evaluation. In Roko, somatic markers are HDC fingerprints of past goal outcomes
that provide rapid pre-evaluation:

```rust
/// Somatic marker: a fast gut-feeling evaluation of a goal.
///
/// Stored as HDC fingerprints in the Daimon. When a new goal is proposed,
/// its fingerprint is compared against the somatic marker library.
/// If a similar goal had a strongly positive or negative outcome in the
/// past, the marker provides an instant evaluation without full analysis.
pub struct SomaticMarkerLibrary {
    /// (fingerprint, valence) pairs. Valence in [-1.0, 1.0].
    pub markers: Vec<(Vec<u8>, f64)>,
    /// HDC dimension (matches bardo-primitives).
    pub hdc_dim: usize,  // default: 10240
    /// Similarity threshold for marker activation.
    pub activation_threshold: f64,  // default: 0.75
}

impl SomaticMarkerLibrary {
    /// Evaluate a goal against somatic markers.
    /// Returns Some(valence) if a matching marker exists, None otherwise.
    pub fn evaluate(&self, goal_fingerprint: &[u8]) -> Option<f64> {
        self.markers.iter()
            .filter_map(|(fp, valence)| {
                let similarity = hdc_cosine_similarity(fp, goal_fingerprint, self.hdc_dim);
                if similarity > self.activation_threshold {
                    Some(*valence * similarity)
                } else {
                    None
                }
            })
            .max_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap())
    }

    /// Record a new marker from a completed goal.
    pub fn record(&mut self, goal_fingerprint: Vec<u8>, outcome_valence: f64) {
        self.markers.push((goal_fingerprint, outcome_valence));
    }
}
```

---

## 8. Configuration

```toml
[goals]
# Enable emergent goal generation.
enabled = true

[goals.emergence]
# Minimum intrinsic motivation for a Nascent goal to survive.
nascent_threshold = 0.3
# Theta reflections required to promote Nascent → Candidate.
reinforcement_threshold = 3
# Maximum concurrent Active goals.
max_active_goals = 5
# HDC similarity threshold for merging duplicate goals.
merge_threshold = 0.85

[goals.motivation]
# Weight for learning progress in IM score.
learning_weight = 0.4
# Weight for competence match in IM score.
competence_weight = 0.35
# Weight for affect alignment in IM score.
affect_weight = 0.25

[goals.decomposition]
# Maximum sub-goals per parent goal.
max_sub_goals = 7
# Minimum estimated AT cost to trigger decomposition.
decomposition_threshold = 5000.0

[goals.somatic]
# Similarity threshold for somatic marker activation.
activation_threshold = 0.75
# Maximum stored somatic markers.
max_markers = 1000

[goals.detectors]
# Enable specific detectors.
knowledge_gap = true
quality_degradation = true
curiosity = true
self_maintenance = true
frustration_recovery = true

[goals.detectors.quality_degradation]
lookback = 10
min_slope = -0.1

[goals.detectors.self_maintenance]
max_unvalidated_hours = 168
```

---

## 9. Integration Wiring

### 9.1 Into the Universal Cognitive Loop

| Loop Step | Goal Emergence Integration |
|---|---|
| 1. PERCEIVE | Active goals influence query (goal-relevant Engrams prioritized) |
| 2. EVALUATE | Score boosted for goal-aligned Engrams |
| 3. ATTEND | Active goals get dedicated VCG auction slots |
| 4. INTEGRATE | Active goal description injected into system prompt |
| 5. ACT | Agent works toward active goal |
| 6. VERIFY | Gate verdicts feed goal progress tracking |
| 7. PERSIST | Goal state changes persisted as Engrams |
| 8. ADAPT | Policy runs GoalSelection during Theta reflections |
| 9. META-COGNIZE | Goal completion/failure feeds Daimon and somatic markers |

### 9.2 Into Existing Crates

| Crate | Integration Point | Change |
|---|---|---|
| `roko-core` | `Context` struct | Add `active_goals: Vec<GoalId>` |
| `roko-daimon` | `DaimonState` | Add `SomaticMarkerLibrary`; goal affect feedback |
| `roko-neuro` | `NeuroStore` queries | Goal-awareness filter (prioritize goal-relevant) |
| `roko-learn` | `EpisodeLogger` | Log goal lifecycle events |
| `roko-orchestrator` | `PlanRunner` | Accept emergent goals as plan sources |
| `roko-compose` | `SystemPromptBuilder` | Include active goal description |
| `roko-cli` | `roko status` | Display active/candidate goals |

---

## 10. Test Criteria

| Test | What It Validates | Type |
|---|---|---|
| `test_knowledge_gap_detector_fires` | Systematic query misses trigger goal | Unit |
| `test_quality_degradation_detector` | Declining gate pass rate triggers goal | Unit |
| `test_curiosity_requires_arousal` | Low-arousal agent doesn't generate curiosity goals | Unit |
| `test_nascent_below_threshold_pruned` | IM < 0.3 goals are removed | Unit |
| `test_reinforcement_promotes_to_candidate` | 3 Theta reinforcements → Candidate | Unit |
| `test_duplicate_goals_merged` | HDC similarity > 0.85 → merge | Unit |
| `test_efe_ranks_correctly` | Higher EFE → higher rank | Unit |
| `test_max_active_goals_enforced` | Cannot activate more than 5 goals | Unit |
| `test_goal_decomposition_respects_max` | Decomposition creates ≤ 7 sub-goals | Unit |
| `test_somatic_marker_speeds_evaluation` | Matching marker returns instant valence | Unit |
| `test_somatic_marker_records_outcome` | Completed goal creates marker | Unit |
| `test_abandoned_goal_negative_marker` | Abandoned goal records negative valence | Unit |
| `test_active_goal_in_system_prompt` | Active goal description appears in composed prompt | Integration |
| `test_goal_lifecycle_full_cycle` | Nascent → Candidate → Active → Achieved | Integration |
| `test_frustration_detector_fires` | 3+ failures on same pattern triggers goal | Unit |

---

## 11. Theoretical Foundations

### 11.1 Autotelic Agents (Colas et al. 2022)

IMGEP (Intrinsically Motivated Goal Exploration Processes) establishes that agents which
generate their own goals explore more efficiently than externally directed agents. The key
insight: **learning progress** (reduction in prediction error over time) is a better
intrinsic reward than novelty alone, because novelty leads to random exploration while
learning progress leads to *progressive mastery*.

Roko's goal emergence engine operationalizes IMGEP: the detectors identify *where learning
is possible* (knowledge gaps, quality degradation), and the IM score ranks goals by
learning potential.

### 11.2 Free Energy Principle (Friston 2010)

Under active inference, agents minimize expected free energy — a quantity that combines
uncertainty reduction (epistemic) and goal satisfaction (pragmatic). Goal selection in Roko
uses EFE as the ranking function, ensuring that the agent naturally balances exploration
(learning) and exploitation (task completion).

### 11.3 Somatic Marker Hypothesis (Damasio 1994)

Damasio showed that emotional markers from past experiences speed up decision-making by
pre-filtering options before deliberate analysis. In Roko, the somatic marker library
provides sub-millisecond goal evaluation via HDC fingerprint matching — the agent's "gut
feeling" about whether a goal is worth pursuing, before any explicit analysis.

### 11.4 IMAGINE (arXiv:2505.17621, 2025)

IMAGINE demonstrated that trajectory-aware intrinsic motivation (not just state novelty)
significantly improves exploration in RL agents. Roko adapts this: goal emergence considers
the *trajectory* of recent episodes (quality trends, failure sequences), not just the
current state.

---

## Cross-References

- [09-universal-cognitive-loop](./09-universal-cognitive-loop.md) — The loop that goal emergence plugs into
- [10-three-cognitive-speeds](./10-three-cognitive-speeds.md) — Theta reflections run goal detection
- [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) — Daimon + Neuro + Dreams as goal sources
- [25-attention-as-currency](./25-attention-as-currency.md) — Goals consume AT budget; EFE includes cost
- [26-cognitive-immune-system](./26-cognitive-immune-system.md) — CIS prevents corrupted knowledge from generating false goals
- [27-temporal-knowledge-topology](./27-temporal-knowledge-topology.md) — Temporal patterns trigger goal emergence
- [29-cognitive-energy-model](./29-cognitive-energy-model.md) — Energy budget constrains active goal count
- [Topic 01: Orchestration](../01-orchestration/INDEX.md) — Orchestrator that executes activated goals
- [Topic 05: Learning](../05-learning/INDEX.md) — Episodes that feed experience source
- [Topic 09: Daimon](../09-daimon/INDEX.md) — Affect source for goal emergence
