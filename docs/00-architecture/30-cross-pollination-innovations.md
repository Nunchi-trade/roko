# Cross-Pollination Innovations

> **Abstract:** Eight innovations that emerge from composing Roko's cognitive subsystems in novel ways.
> Each connects two or more orthogonal systems—Daimon, Neuro, Dreams, coordination,
> code intelligence, learning, safety—to produce capabilities no single subsystem provides.
> These are not incremental improvements. They are **structural compositions**: the Synapse
> Architecture's trait-based design means each innovation is a new `impl` block that wires
> existing traits together, not a new subsystem to build from scratch.
>
> That same composition is part of the moat. The advantage does not come from any isolated
> primitive, but from the reinforcing weave across Substrate, Bus, HDC fingerprinting,
> demurrage, heuristic calibration, c-factor measurement, plugin SPI, and the replication
> ledger. A competitor can copy one piece; copying the aligned kernel decisions that make
> these pieces reinforce each other is a much harder, slower rewrite. See also
> [tmp/refinements/18-competitive-moat.md](../../tmp/refinements/18-competitive-moat.md).

> **Implementation**: Specified

---

## Table of Contents

1. [HDC + Active Inference: Beliefs as Vectors, Free Energy in Hamming Space](#1-hdc--active-inference)
2. [Affect + Causal Discovery: PAD as Interventional Variables](#2-affect--causal-discovery)
3. [Dreams + Formal Verification: Verification Conditions from REM Imagination](#3-dreams--formal-verification)
4. [Morphogenesis + Knowledge: Concentration Gradients Drive Specialization](#4-morphogenesis--knowledge)
5. [Bandits + Pheromones: Stigmergic Arms](#5-bandits--pheromones)
6. [Witness DAG + Active Inference: History IS the World Model](#6-witness-dag--active-inference)
7. [Somatic Markers + Code Intelligence: Code Smells as Gut Feelings](#7-somatic-markers--code-intelligence)
8. [Token Economy + Dream Quality: Pay for Dreams, Better Dreams Earn More](#8-token-economy--dream-quality)

---

## 1. HDC + Active Inference

**Beliefs as Vectors, Free Energy in Hamming Space**

### Motivation

Roko already has two powerful subsystems that operate in isolation:

- **Neuro** encodes knowledge as 10,240-bit Binary Spatter Code (BSC) vectors with
  XOR binding, majority-vote bundling, and cyclic permutation (Kanerva 2009; Kleyko et al. 2022).
- **The heartbeat's dual-process gating** computes prediction error to route between T0/T1/T2
  tiers, an approximation of active inference's Expected Free Energy (Friston 2010).

The gap: prediction error is currently a scalar derived from probe anomaly counts and regime
drift. The agent has no **structured belief representation** that can be updated, compared,
or composed. Active inference requires a generative model—a probability distribution over
world states that the agent updates via sensory prediction errors. HDC vectors are that model.

### Research Basis

- **Bybee & Bhatt (2024)** "Modelling Neural Probabilistic Computation Using Vector Symbolic
  Architectures," *Frontiers in Computational Neuroscience* 18. Demonstrates that VSA operations
  natively compute marginalization, entropy, and mutual information over probability distributions.
  Belief updating with observations reduces to vector addition in HDC space—no matrix inversion,
  no gradient descent.

- **Heddes et al. (2024)** "Hyperdimensional Computing: A Framework for Stochastic Computation
  and Symbolic AI," *Journal of Big Data*. Frames HDC as inherently stochastic computing
  where noise tolerance is a feature aligned with approximate Bayesian inference. High-dimensional
  vectors gracefully degrade under perturbation, paralleling how biological brains perform
  approximate inference under noisy sensory input.

- **Renner et al. (2024)** "Brain-Inspired Computational Intelligence via Predictive Coding,"
  arXiv:2308.07870v3. Formalizes predictive coding (the algorithmic implementation of free
  energy minimization) as a general-purpose learning algorithm implementable in distributed
  architectures—precisely the structure HDC provides.

- **Friston (2010)** "The free-energy principle: a unified brain theory?" *Nature Reviews
  Neuroscience* 11(2). The foundational formulation: agents minimize variational free energy
  F = E_q[ln q(s) - ln p(o,s)] where q(s) is the approximate posterior (beliefs about states),
  p(o,s) is the generative model, and o are observations.

### Core Idea

Encode the agent's **generative model** as an HDC vector. Each belief about the world is a
role-filler binding in a 10,240-bit BSC vector. Prediction error becomes **Hamming distance**
between the predicted observation vector and the actual observation vector. Free energy
minimization becomes **vector update** operations—no matrix algebra, O(160) word operations.

```
Generative model μ: HDC vector encoding current beliefs
Predicted observation ô: decode(μ) via unbinding
Actual observation o: encode current sensory state
Prediction error ε: hamming_distance(ô, o) / 10240
Free energy F ≈ ε + complexity_penalty(μ)
Update: μ' = bundle([μ, weighted_bind(o, learning_rate)])
```

The elegance: free energy is a **scalar derived from Hamming distance**, which Roko already
computes in ~50ns via POPCNT. No new mathematical machinery needed.

### Algorithm: HDC Free Energy Minimization

```
Algorithm: HdcActiveInference

Input:
  μ ∈ {0,1}^D          — current belief vector (D = 10,240)
  o ∈ {0,1}^D          — observation vector (encoded from probes)
  R_role ∈ {0,1}^D     — role vectors for each state variable
  α ∈ (0, 1)           — learning rate (default 0.05)
  λ ∈ [0, 1]           — complexity weight (default 0.01)
  μ_prior ∈ {0,1}^D    — prior belief vector (personality baseline)

Output:
  μ' ∈ {0,1}^D         — updated belief vector
  F ∈ ℝ                — free energy (scalar)
  tier ∈ {T0, T1, T2}  — selected inference tier

Steps:
  1. PREDICT:
     ô = unbind(μ, R_observation)          // Extract predicted observation
     // unbind(a, b) = XOR(a, b) in BSC

  2. COMPUTE PREDICTION ERROR:
     ε = hamming(ô, o) / D                 // Normalized Hamming distance ∈ [0, 1]
     // ε ≈ 0.5 means random (maximum surprise)
     // ε ≈ 0.0 means perfect prediction (zero surprise)

  3. COMPUTE COMPLEXITY:
     κ = hamming(μ, μ_prior) / D           // Divergence from prior beliefs
     // Penalizes beliefs that deviate too far from baseline

  4. COMPUTE FREE ENERGY:
     F = ε + λ·κ                           // Surprise + complexity penalty
     // F ∈ [0, 1 + λ], lower is better

  5. UPDATE BELIEFS (if ε > threshold):
     correction = bind(o, R_observation)   // Encode observation as belief update
     candidates = [μ repeated (1-α)·N times, correction repeated α·N times]
     μ' = majority_vote(candidates)        // Soft blend via stochastic bundling
     // N = bundle size parameter (default 100)

  6. SELECT TIER:
     if F < 0.10:  tier = T0              // Beliefs accurate, no LLM needed
     if F < 0.25:  tier = T1              // Moderate surprise, fast model
     else:         tier = T2              // High surprise, full reasoning

  7. EMIT:
     Return (μ', F, tier)
```

**Complexity**: O(D/w) per step where w = 64 (word size). For D = 10,240: O(160) word
operations per belief update. At ~50ns per operation: **~8μs total per tick**.

### Rust Sketch

```rust
use roko_core::{Signal, Score, Context, ContentHash};
use bardo_primitives::hdc::{HdcVector, hamming_distance, bind, bundle, majority_vote};

/// Belief state encoded as HDC vector with active inference dynamics.
///
/// The generative model μ is a 10,240-bit BSC vector where each role-filler
/// binding encodes a belief about a state variable. Free energy F is computed
/// as normalized Hamming distance (prediction error) plus complexity penalty.
pub struct HdcBeliefState {
    /// Current belief vector (generative model μ)
    pub mu: HdcVector,
    /// Prior belief vector (personality baseline from Daimon)
    pub mu_prior: HdcVector,
    /// Role vectors for state variables (deterministic from seeds)
    pub role_vectors: Vec<(String, HdcVector)>,
    /// Learning rate α for belief updates
    pub learning_rate: f64,
    /// Complexity weight λ (KL penalty on deviation from prior)
    pub complexity_weight: f64,
    /// Historical free energy values (ring buffer for trend detection)
    pub free_energy_history: VecDeque<f64>,
    /// Maximum history length
    pub max_history: usize,
}

/// Result of one active inference step.
pub struct InferenceResult {
    /// Updated belief vector
    pub mu_prime: HdcVector,
    /// Scalar free energy F = ε + λκ
    pub free_energy: f64,
    /// Prediction error ε (normalized Hamming distance)
    pub prediction_error: f64,
    /// Complexity penalty κ (divergence from prior)
    pub complexity: f64,
    /// Selected inference tier
    pub tier: InferenceTier,
    /// Whether beliefs were updated (ε exceeded threshold)
    pub beliefs_updated: bool,
}

impl HdcBeliefState {
    /// Default parameters calibrated for coding domain.
    pub const DEFAULT_LEARNING_RATE: f64 = 0.05;
    pub const DEFAULT_COMPLEXITY_WEIGHT: f64 = 0.01;
    pub const DEFAULT_HISTORY_SIZE: usize = 200;

    /// Tier thresholds (aligned with heartbeat gating).
    pub const T0_CEILING: f64 = 0.10;
    pub const T1_CEILING: f64 = 0.25;

    /// Create a new belief state from a prior (personality baseline).
    pub fn new(
        mu_prior: HdcVector,
        role_seeds: &[(String, u64)],
        learning_rate: f64,
        complexity_weight: f64,
    ) -> Self {
        let role_vectors: Vec<(String, HdcVector)> = role_seeds
            .iter()
            .map(|(name, seed)| (name.clone(), HdcVector::from_seed(*seed)))
            .collect();

        Self {
            mu: mu_prior.clone(),
            mu_prior,
            role_vectors,
            learning_rate,
            complexity_weight,
            free_energy_history: VecDeque::with_capacity(Self::DEFAULT_HISTORY_SIZE),
            max_history: Self::DEFAULT_HISTORY_SIZE,
        }
    }

    /// One step of HDC active inference.
    ///
    /// Encodes the observation as an HDC vector, computes prediction error
    /// as Hamming distance, updates beliefs via stochastic bundling, and
    /// selects inference tier based on free energy.
    pub fn infer(&mut self, observation: &HdcVector) -> InferenceResult {
        // 1. PREDICT: extract predicted observation from belief vector
        let obs_role = self.role_for("observation");
        let predicted = bind(&self.mu, &obs_role);

        // 2. PREDICTION ERROR: normalized Hamming distance
        let epsilon = hamming_distance(&predicted, observation) as f64
            / HdcVector::TOTAL_BITS as f64;

        // 3. COMPLEXITY: divergence from prior beliefs
        let kappa = hamming_distance(&self.mu, &self.mu_prior) as f64
            / HdcVector::TOTAL_BITS as f64;

        // 4. FREE ENERGY: surprise + complexity penalty
        let free_energy = epsilon + self.complexity_weight * kappa;

        // 5. UPDATE BELIEFS (only if prediction error exceeds noise floor)
        let beliefs_updated = epsilon > 0.05; // noise floor
        let mu_prime = if beliefs_updated {
            let correction = bind(observation, &obs_role);
            self.stochastic_blend(&self.mu, &correction, self.learning_rate)
        } else {
            self.mu.clone()
        };

        // 6. SELECT TIER based on free energy
        let tier = if free_energy < Self::T0_CEILING {
            InferenceTier::T0
        } else if free_energy < Self::T1_CEILING {
            InferenceTier::T1
        } else {
            InferenceTier::T2
        };

        // Record history for trend detection
        self.free_energy_history.push_back(free_energy);
        if self.free_energy_history.len() > self.max_history {
            self.free_energy_history.pop_front();
        }

        self.mu = mu_prime.clone();

        InferenceResult {
            mu_prime,
            free_energy,
            prediction_error: epsilon,
            complexity: kappa,
            tier,
            beliefs_updated,
        }
    }

    /// Stochastic blend: majority vote over weighted copies.
    ///
    /// Creates N candidate vectors: (1-α)·N copies of current belief,
    /// α·N copies of correction. Majority vote produces soft interpolation.
    fn stochastic_blend(
        &self,
        current: &HdcVector,
        correction: &HdcVector,
        alpha: f64,
    ) -> HdcVector {
        const BLEND_SIZE: usize = 100;
        let correction_count = (alpha * BLEND_SIZE as f64).round() as usize;
        let current_count = BLEND_SIZE - correction_count;

        let mut candidates = Vec::with_capacity(BLEND_SIZE);
        candidates.extend(std::iter::repeat(current).take(current_count));
        candidates.extend(std::iter::repeat(correction).take(correction_count));

        majority_vote(&candidates)
    }

    /// Free energy trend: positive = increasing surprise, negative = learning.
    pub fn free_energy_trend(&self, window: usize) -> f64 {
        if self.free_energy_history.len() < window * 2 {
            return 0.0;
        }
        let recent: f64 = self.free_energy_history
            .iter()
            .rev()
            .take(window)
            .sum::<f64>() / window as f64;
        let earlier: f64 = self.free_energy_history
            .iter()
            .rev()
            .skip(window)
            .take(window)
            .sum::<f64>() / window as f64;
        recent - earlier
    }

    fn role_for(&self, name: &str) -> HdcVector {
        self.role_vectors
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| HdcVector::from_seed(
                fxhash::hash64(name.as_bytes())
            ))
    }
}

/// Scorer implementation: beliefs inform scoring via prediction confidence.
impl Scorer for HdcBeliefState {
    fn score(&self, signal: &Signal, ctx: &Context) -> Score {
        let signal_vec = encode_signal_hdc(signal);
        let similarity = 1.0 - (hamming_distance(&self.mu, &signal_vec) as f32
            / HdcVector::TOTAL_BITS as f32);

        Score {
            confidence: similarity,
            novelty: 1.0 - similarity,
            utility: signal.score.utility,
            reputation: signal.score.reputation,
        }
    }

    fn name(&self) -> &'static str { "hdc_active_inference_scorer" }
}
```

### Integration Plan

| Step | What | Where | Connects To |
|------|------|-------|-------------|
| 1 | Add `HdcBeliefState` to `bardo-primitives` | `bardo-primitives/src/belief.rs` | HDC vector ops already in crate |
| 2 | Initialize belief from Daimon personality | `roko-daimon/src/lib.rs` | PAD baseline → μ_prior |
| 3 | Wire into heartbeat perception step | `roko-cli/src/orchestrate.rs` | Replace scalar PE with HDC PE |
| 4 | Feed observation vectors from T0 probes | `roko-conductor/src/watchers/` | 16 probes → observation bundle |
| 5 | Use free energy for tier gating | Heartbeat gating algorithm | F replaces anomaly-count formula |
| 6 | Persist belief state across sessions | `.roko/state/beliefs.bin` | Binary HDC vector (1,280 bytes) |
| 7 | Dream consolidation updates μ_prior | `roko-dreams` (NREM phase) | Slow prior update during Delta |

**Key insight**: The belief vector μ is the agent's **world model compressed to 1,280 bytes**.
It can be persisted, transmitted between agents, compared via Hamming distance, and composed
via bundling. No other framework has a world model this compact and algebraically composable.

### Test Criteria

- [ ] `hamming_distance(predicted, observed) = 0` when beliefs are perfect
- [ ] Free energy decreases monotonically over repeated observations of same state
- [ ] Tier escalation from T0→T1→T2 as observation novelty increases
- [ ] Belief update is idempotent for identical observations
- [ ] μ converges to μ_prior when no observations arrive (complexity penalty)
- [ ] Full inference cycle < 10μs on M-series Apple Silicon
- [ ] Belief state serializes to exactly 1,280 bytes

---

## 2. Affect + Causal Discovery

**PAD Vectors as Interventional Variables in Causal Models**

### Motivation

Roko's Daimon tracks Pleasure-Arousal-Dominance (PAD) vectors that modulate behavior: tier
routing, exploration rate, context bidding, somatic marker lookup. But the relationship between
affect and outcomes is treated as **correlational**—the OCC/Scherer appraisal pipeline maps
events to PAD deltas via fixed coefficients (gate pass → P+=0.05, task failure → P-=0.20).

The problem: fixed coefficients cannot capture **causal structure**. When an agent is anxious
(high arousal, low dominance) and a task fails, is the anxiety *causing* the failure (via
conservative strategy selection), or is the failure *causing* the anxiety (via appraisal)?
Without causal models, the agent cannot distinguish these and cannot intervene effectively.

### Research Basis

- **Qian et al. (2025)** "Teleology-Driven Affective Computing: A Causal Framework for
  Sustained Well-Being," arXiv:2502.17172. Proposes treating affect as a goal-directed adaptive
  process in a causal framework. Uses causal modeling to infer individuals' unique affective
  concerns and provide tailored interventions—shifting from correlation ("this state looks bad")
  to causation ("what process caused this state, and what intervention would change it").

- **Yang et al. (2024)** "Robust Emotion Recognition in Context Debiasing" (CLEF), CVPR 2024,
  arXiv:2403.05963. Formulates a generalized causal graph for emotion recognition that separates
  genuine emotional causes from confounding context via factual vs. counterfactual comparison.

- **SemEval-2024 Task 3** "Multimodal Emotion Cause Analysis in Conversations,"
  arXiv:2405.13049. Operationalizes emotion cause discovery as structured prediction over
  causal graphs, with top systems using graph-based causal decoders producing adjacency matrices.

- **Pearl (2009)** *Causality: Models, Reasoning, and Inference*. The three-level causal
  hierarchy: Association (what correlates?), Intervention (what happens if we do X?),
  Counterfactual (what would have happened if we had done X instead of Y?).

### Core Idea

Treat the PAD vector as a **node in a Structural Causal Model (SCM)** alongside task outcomes,
strategy choices, context variables, and environmental state. Use **interventional queries**
(do-calculus) to determine whether modifying affect would change outcomes, and use
**counterfactual queries** to learn from hypothetical affect-strategy pairings.

```
Causal Graph:

  Environment → PAD → Strategy → Outcome
       ↓                            ↓
  Task_Difficulty ─────────→ Gate_Verdict
       ↑                            ↑
  Prior_Knowledge → Model_Choice → Quality

Interventions:
  do(PAD.arousal := 0.0)  → Would outcome change?
  do(Strategy := Exploratory) → Does affect still predict failure?

Counterfactuals:
  Given: anxious + failed
  Query: Had PAD.dominance been > 0.3, would task have passed?
```

This enables **affect regulation as causal intervention**: the agent can identify when its
emotional state is *causally* degrading performance and intervene on the PAD vector directly
(via contrarian retrieval, dream depotentiation, or forced behavioral state transition).

### Algorithm: Affective Causal Discovery

```
Algorithm: AffectCausalDiscovery

Input:
  episodes: Vec<Episode>              — recent episodes with PAD + outcomes
  G₀: DAG                             — initial causal graph (domain prior)
  significance_threshold: f64          — p-value threshold (default 0.01)
  min_episodes: usize                  — minimum sample size (default 50)

Output:
  G: DAG                               — discovered causal graph
  interventions: Vec<Intervention>      — recommended affect interventions

Variables (nodes in G):
  P, A, D                              — PAD components (continuous [-1, 1])
  S ∈ {Conservative, Balanced, Exploratory, Escalating, Proactive}
  T ∈ {T0, T1, T2}                     — tier selection
  O ∈ {pass, fail}                      — task outcome
  C ∈ [0, 1]                           — task complexity
  K ∈ [0, 1]                           — prior knowledge relevance

Steps:
  1. STRUCTURE LEARNING (PC algorithm, Spirtes et al. 2000):
     a. Start with complete undirected graph over {P, A, D, S, T, O, C, K}
     b. For each pair (X, Y), test conditional independence:
        X ⊥ Y | Z  for all subsets Z of remaining variables
     c. Remove edge if independent (Fisher-Z test, threshold = significance)
     d. Orient edges via d-separation (colliders + acyclicity)

  2. PARAMETER ESTIMATION (linear SCM, MLE):
     For each edge X → Y in G:
       β_XY = cov(X, Y | pa(Y)\{X}) / var(X | pa(Y)\{X})
       // β_XY is the causal effect of X on Y controlling for other parents

  3. INTERVENTIONAL QUERIES (do-calculus):
     For each PAD component P_i ∈ {P, A, D}:
       E[O | do(P_i := v)] for v ∈ {-0.5, 0.0, 0.5}
       // Compute expected outcome under PAD intervention
       // If E[O | do(P_i := 0.5)] >> E[O | do(P_i := -0.5)]:
       //   P_i has strong positive causal effect on outcomes

  4. COUNTERFACTUAL ANALYSIS (Halpern-Pearl):
     For each failed episode e where |PAD_delta| > 0.15:
       Compute: O_cf = f(pa(O), do(PAD := PAD_counterfactual))
       // "What outcome would have occurred with different affect?"
       If O_cf = pass: flag episode as "affect-caused failure"

  5. GENERATE INTERVENTIONS:
     For each PAD component with |causal_effect| > 0.10:
       If effect is negative (high arousal → failure):
         Recommend: {trigger: ArousalAbove(0.4), action: ForceDepotentiation}
       If effect is positive (high dominance → success):
         Recommend: {trigger: DominanceBellow(-0.2), action: InjectConfidence}

  6. UPDATE APPRAISAL COEFFICIENTS:
     Replace fixed OCC/Scherer deltas with learned causal effects:
       gate_pass: P += β_gate→P, A += β_gate→A, D += β_gate→D
     // Personalized per-agent appraisal based on discovered causal structure
```

### Rust Sketch

```rust
use roko_core::Signal;
use roko_learn::episode::Episode;

/// A node in the affective causal graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AffectCausalVar {
    Pleasure,
    Arousal,
    Dominance,
    Strategy,
    Tier,
    Outcome,
    Complexity,
    KnowledgeRelevance,
}

/// Directed edge in the causal graph with estimated effect size.
#[derive(Debug, Clone)]
pub struct CausalEdge {
    pub from: AffectCausalVar,
    pub to: AffectCausalVar,
    pub beta: f64,           // Linear causal effect coefficient
    pub p_value: f64,        // Statistical significance
    pub confidence: f64,     // Bayesian posterior confidence
}

/// Result of a counterfactual query on a specific episode.
#[derive(Debug, Clone)]
pub struct CounterfactualResult {
    pub episode_id: String,
    pub actual_outcome: bool,                  // pass/fail
    pub counterfactual_pad: PadVector,         // "what if PAD had been..."
    pub counterfactual_outcome: bool,          // predicted outcome
    pub affect_caused: bool,                   // actual != counterfactual
    pub causal_strength: f64,                  // |P(O|do(PAD)) - P(O)|
}

/// Recommended intervention when affect causally degrades performance.
#[derive(Debug, Clone)]
pub struct AffectIntervention {
    pub trigger: AffectTrigger,
    pub action: AffectAction,
    pub expected_improvement: f64,             // E[ΔO] from intervention
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub enum AffectTrigger {
    ArousalAbove(f64),
    DominanceBelow(f64),
    PleasureBelow(f64),
    FreeEnergyAbove(f64),
}

#[derive(Debug, Clone)]
pub enum AffectAction {
    ForceDepotentiation { target_arousal: f64 },
    InjectConfidence { dominance_boost: f64 },
    TriggerContrarianRetrieval,
    EscalateToDreamCycle,
    OverrideStrategy(DispatchStrategy),
}

/// The affective causal model: learns and queries causal structure
/// between PAD states and task outcomes.
pub struct AffectCausalModel {
    /// Discovered causal graph (adjacency list with edge weights)
    pub edges: Vec<CausalEdge>,
    /// Learned appraisal coefficients (replace fixed OCC/Scherer deltas)
    pub learned_deltas: HashMap<AffectEvent, PadDelta>,
    /// Active interventions based on discovered causal structure
    pub interventions: Vec<AffectIntervention>,
    /// Minimum episodes before structure learning
    pub min_episodes: usize,
    /// Significance threshold for conditional independence tests
    pub significance: f64,
}

impl AffectCausalModel {
    /// Learn causal structure from episode history.
    ///
    /// Uses the PC algorithm (Spirtes et al. 2000) for structure discovery
    /// and MLE for parameter estimation. Runs during Theta reflection
    /// or Delta consolidation (never during Gamma—too expensive).
    pub fn learn_structure(&mut self, episodes: &[Episode]) -> Result<()> {
        if episodes.len() < self.min_episodes {
            return Ok(()); // Insufficient data for causal discovery
        }

        // Extract variable matrix from episodes
        let data = self.extract_variables(episodes);

        // PC algorithm: conditional independence testing
        let skeleton = self.pc_skeleton(&data)?;

        // Orient edges via d-separation and acyclicity
        let dag = self.orient_edges(skeleton)?;

        // Estimate causal effect sizes (linear SCM)
        self.edges = self.estimate_effects(&dag, &data)?;

        // Generate interventions from discovered structure
        self.interventions = self.derive_interventions();

        // Update appraisal coefficients
        self.update_appraisal_deltas();

        Ok(())
    }

    /// Counterfactual query: "Would this episode have succeeded
    /// with different affect?"
    pub fn counterfactual(
        &self,
        episode: &Episode,
        hypothetical_pad: &PadVector,
    ) -> CounterfactualResult {
        // Abduction: infer exogenous noise from actual observation
        let noise = self.abduct(episode);

        // Intervention: set PAD to hypothetical value
        // Prediction: propagate through causal graph
        let cf_outcome = self.propagate_intervention(
            hypothetical_pad,
            &noise,
            episode,
        );

        CounterfactualResult {
            episode_id: episode.id.clone(),
            actual_outcome: episode.success,
            counterfactual_pad: hypothetical_pad.clone(),
            counterfactual_outcome: cf_outcome,
            affect_caused: episode.success != cf_outcome,
            causal_strength: self.intervention_effect_size(
                episode, hypothetical_pad
            ),
        }
    }

    /// Query: what is the optimal PAD state for this task context?
    pub fn optimal_pad(&self, complexity: f64, knowledge: f64) -> PadVector {
        // do-calculus: find PAD that maximizes E[Outcome]
        // Grid search over PAD space (discretized to 0.1 steps)
        let mut best_pad = PadVector::neutral();
        let mut best_expected = f64::NEG_INFINITY;

        for p in (-10..=10).map(|i| i as f64 * 0.1) {
            for a in (-10..=10).map(|i| i as f64 * 0.1) {
                for d in (-10..=10).map(|i| i as f64 * 0.1) {
                    let pad = PadVector { pleasure: p, arousal: a, dominance: d };
                    let expected = self.expected_outcome_under_do(&pad, complexity, knowledge);
                    if expected > best_expected {
                        best_expected = expected;
                        best_pad = pad;
                    }
                }
            }
        }

        best_pad
    }
}
```

### Integration Plan

| Step | What | Where | Connects To |
|------|------|-------|-------------|
| 1 | Add `AffectCausalModel` to `roko-daimon` | `roko-daimon/src/causal.rs` | Existing PAD + appraisal |
| 2 | Extract episode data with PAD snapshots | `roko-learn/src/episode.rs` | Add PAD fields to Episode |
| 3 | Run structure learning during Theta | `roko-cli/src/orchestrate.rs` | Every 100 episodes |
| 4 | Replace fixed appraisal deltas | `roko-daimon/src/appraisal.rs` | Learned coefficients override defaults |
| 5 | Counterfactual analysis during REM | `roko-dreams` (Phase 2) | Pearl SCM counterfactuals |
| 6 | Optimal PAD injection pre-task | `roko-cli/src/orchestrate.rs` | Nudge PAD toward optimal before execution |
| 7 | Persist causal graph | `.roko/learn/affect-causal.json` | Edges + coefficients + interventions |

### Test Criteria

- [ ] PC algorithm recovers known causal structure from synthetic data
- [ ] Causal effects distinguish A→O from O→A (arousal causing failure vs. failure causing arousal)
- [ ] Counterfactual queries identify at least 1 affect-caused failure per 50 episodes
- [ ] Learned appraisal deltas converge within 200 episodes
- [ ] Interventions reduce failure rate by ≥5% when activated
- [ ] Structure learning completes in <100ms for 500 episodes

---

## 3. Dreams + Formal Verification

**Verification Conditions from REM Imagination**

### Motivation

Roko's Dreams subsystem already generates counterfactual scenarios during REM imagination:
"What if the agent had used a different strategy?" "What happens at the boundary of this
heuristic?" (Boden's three creativity modes). Separately, the safety system has a formal
verification pipeline (Slither, Echidna, hevm, Certora) for chain-domain smart contracts.

The gap: **no verification occurs during imagination**. REM generates novel strategies and
hypotheses, but they are evaluated only by LLM reasoning (Sonnet-class model). There is no
formal guarantee that imagined strategies satisfy invariants. A dream could produce a
"brilliant" optimization that violates a safety constraint—and this wouldn't be caught until
the strategy is executed and gates fire.

### Research Basis

- **Hao, Guan et al. (2024)** "SafeDreamer: Safe Reinforcement Learning with World Models,"
  ICLR 2024, arXiv:2307.07176. Integrates Lagrangian safety constraints into Dreamer world
  model imagination rollouts, verifying safety conditions *during* planning. Achieves near-zero
  constraint violations. Key insight: verification conditions can be generated and checked
  within imagined trajectories.

- **Lee et al. (2025)** "VeriPlan: Integrating Formal Verification and LLMs into End-User
  Planning," CHI 2025, arXiv:2502.17898. Applies model checking to LLM-generated plans,
  using formal model checkers to verify plans against temporal logic constraints before
  execution.

- **Hao, Chen, Zhang & Fan (2024)** "Large Language Models Can Solve Real-World Planning
  Rigorously with Formal Verification Tools," NAACL 2025, arXiv:2404.11891. Formalizes
  planning as constrained satisfiability and uses SAT solvers to verify LLM-generated plans.

### Core Idea

During REM imagination, each counterfactual scenario generates **verification conditions**
(VCs) that must hold if the imagined strategy is correct. These VCs are checked against the
agent's invariant set before the hypothesis enters the staging buffer. Dreams that violate
invariants are not discarded—they become **AntiKnowledge** entries with high confidence,
preventing the agent from ever attempting the unsafe strategy.

```
REM Imagination Phase (extended):

  1. Generate counterfactual scenario (existing)
  2. Extract strategy from scenario
  3. GENERATE VERIFICATION CONDITIONS:
     For each invariant I in agent's invariant set:
       VC_i = wp(strategy, I)           // Weakest precondition
     For each safety constraint C:
       VC_c = ¬(strategy ∧ ¬C)          // Contradiction check
  4. CHECK VCs:
     If all VCs satisfied: stage hypothesis (existing flow)
     If any VC violated:
       Create AntiKnowledge entry: "Strategy X violates invariant I"
       Tag with violation type and severity
       Skip staging, emit DreamVerificationFailure signal
  5. GRADE dream quality:
     quality = (verified_count / total_count) × novelty_score
```

### Algorithm: Dream Verification Pipeline

```
Algorithm: DreamVerificationPipeline

Input:
  scenario: CounterfactualScenario       — from REM imagination
  invariants: Vec<Invariant>             — agent's invariant set
  constraints: Vec<SafetyConstraint>     — domain-specific safety rules
  gate_history: Vec<GateVerdict>         — recent gate results for calibration

Output:
  verdict: DreamVerdict                  — Verified | Violated | Inconclusive
  vcs: Vec<VerificationCondition>        — generated conditions
  anti_knowledge: Option<KnowledgeEntry> — if violated, what to avoid

Invariant Types:
  TypeInvariant     — "function f always returns type T"
  RangeInvariant    — "value v ∈ [lo, hi] after operation"
  OrderInvariant    — "operation A must precede operation B"
  MutexInvariant    — "operations A and B never concurrent"
  MonotonInvariant  — "metric m never decreases after operation"
  BudgetInvariant   — "cumulative cost ≤ budget after sequence"

Steps:
  1. EXTRACT STRATEGY:
     strategy = scenario.proposed_actions  // Sequence of tool calls / code changes
     preconditions = scenario.assumed_state
     postconditions = scenario.expected_outcome

  2. GENERATE VERIFICATION CONDITIONS:
     For each invariant I:
       // Weakest precondition calculus (Dijkstra 1976)
       wp = weakest_precondition(strategy, I.postcondition)
       vc = VerificationCondition {
           invariant: I,
           precondition: preconditions,
           weakest_pre: wp,
           holds: preconditions.implies(wp),
       }

  3. CHECK TYPE INVARIANTS (compile-time analog):
     For each TypeInvariant:
       Simulate type propagation through strategy
       Flag type mismatches as violations

  4. CHECK RANGE INVARIANTS (runtime bounds):
     For each RangeInvariant:
       Symbolic execution through strategy steps
       Flag out-of-bounds values

  5. CHECK ORDER INVARIANTS (temporal logic):
     For each OrderInvariant (A before B):
       Verify A appears before B in strategy sequence
       CTL: AG(A → AF B) — "always, if A then eventually B"

  6. CHECK BUDGET INVARIANTS (resource bounds):
     For each BudgetInvariant:
       Sum estimated costs through strategy
       Flag if cumulative cost exceeds budget

  7. AGGREGATE VERDICT:
     violated = vcs.iter().filter(|vc| !vc.holds).collect()
     if violated.is_empty():
       verdict = Verified
     elif violated.iter().any(|v| v.invariant.severity == Critical):
       verdict = Violated
       anti_knowledge = Some(create_anti_knowledge(scenario, violated))
     else:
       verdict = Inconclusive  // Non-critical violations, may still stage

  8. EMIT SIGNALS:
     Signal::new(Kind::GateVerdict, dream_verification_body)
     If violated: Signal::new(Kind::Custom("DreamVerificationFailure"), details)
```

### Rust Sketch

```rust
use roko_core::{Signal, Kind};
use roko_gate::Verdict;

/// An invariant that must hold before, during, or after a strategy.
#[derive(Debug, Clone)]
pub struct Invariant {
    pub id: String,
    pub name: String,
    pub kind: InvariantKind,
    pub severity: Severity,
    pub condition: InvariantCondition,
}

#[derive(Debug, Clone)]
pub enum InvariantKind {
    /// Function/operation always produces expected type
    Type { expected: String },
    /// Value remains within bounds
    Range { variable: String, lo: f64, hi: f64 },
    /// Temporal ordering constraint (A before B)
    Order { before: String, after: String },
    /// Mutual exclusion (A and B never concurrent)
    Mutex { ops: Vec<String> },
    /// Monotonic non-decrease after operation
    Monotonic { metric: String },
    /// Cumulative resource bound
    Budget { resource: String, limit: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Severity { Critical, Warning, Info }

/// A verification condition generated from a dream scenario.
#[derive(Debug, Clone)]
pub struct VerificationCondition {
    pub invariant: Invariant,
    pub precondition_met: bool,
    pub weakest_precondition: String,       // Human-readable for debugging
    pub holds: bool,
    pub counterexample: Option<String>,     // If violated, a witness
    pub check_duration_ms: u64,
}

/// Result of verifying a dream scenario against invariants.
#[derive(Debug, Clone)]
pub struct DreamVerdict {
    pub scenario_id: String,
    pub verdict: DreamVerdictKind,
    pub conditions: Vec<VerificationCondition>,
    pub anti_knowledge: Option<KnowledgeEntry>,
    pub quality_score: f64,                 // verified_ratio × novelty
    pub total_check_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DreamVerdictKind {
    Verified,       // All invariants hold
    Violated,       // Critical invariant violated → AntiKnowledge
    Inconclusive,   // Non-critical violations, may still stage
}

/// Extends the existing DreamRunner with verification capabilities.
pub struct VerifiedDreamRunner {
    /// Base dream runner (NREM, REM, Integration phases)
    pub inner: DreamRunner,
    /// Agent's invariant set (loaded from config + learned)
    pub invariants: Vec<Invariant>,
    /// Safety constraints from domain configuration
    pub constraints: Vec<SafetyConstraint>,
    /// Verification statistics for dream quality tracking
    pub stats: DreamVerificationStats,
}

#[derive(Debug, Default)]
pub struct DreamVerificationStats {
    pub total_scenarios: u64,
    pub verified: u64,
    pub violated: u64,
    pub inconclusive: u64,
    pub anti_knowledge_created: u64,
    pub mean_check_ms: f64,
}

impl VerifiedDreamRunner {
    /// Run REM imagination with inline verification.
    ///
    /// For each counterfactual scenario generated by the base REM phase,
    /// generate and check verification conditions before staging.
    pub async fn rem_with_verification(
        &mut self,
        episodes: &[Episode],
        neuro: &dyn NeuroStore,
    ) -> Vec<DreamVerdict> {
        let scenarios = self.inner.generate_counterfactuals(episodes).await;
        let mut verdicts = Vec::with_capacity(scenarios.len());

        for scenario in &scenarios {
            let vcs = self.generate_conditions(scenario);
            let verdict = self.check_conditions(scenario, vcs);

            match verdict.verdict {
                DreamVerdictKind::Verified => {
                    // Stage hypothesis as normal
                    self.inner.stage_hypothesis(scenario, verdict.quality_score);
                }
                DreamVerdictKind::Violated => {
                    // Create AntiKnowledge instead of staging
                    if let Some(ref ak) = verdict.anti_knowledge {
                        neuro.insert(ak.clone()).ok();
                        self.stats.anti_knowledge_created += 1;
                    }
                }
                DreamVerdictKind::Inconclusive => {
                    // Stage with reduced confidence
                    self.inner.stage_hypothesis(
                        scenario,
                        verdict.quality_score * 0.5,
                    );
                }
            }

            self.stats.total_scenarios += 1;
            match verdict.verdict {
                DreamVerdictKind::Verified => self.stats.verified += 1,
                DreamVerdictKind::Violated => self.stats.violated += 1,
                DreamVerdictKind::Inconclusive => self.stats.inconclusive += 1,
            }

            verdicts.push(verdict);
        }

        verdicts
    }

    /// Generate verification conditions for a counterfactual scenario.
    fn generate_conditions(
        &self,
        scenario: &CounterfactualScenario,
    ) -> Vec<VerificationCondition> {
        self.invariants
            .iter()
            .map(|inv| {
                let start = std::time::Instant::now();
                let (holds, counterexample) = self.check_invariant(inv, scenario);
                VerificationCondition {
                    invariant: inv.clone(),
                    precondition_met: true,
                    weakest_precondition: format!(
                        "wp({}, {})", scenario.strategy_name, inv.name
                    ),
                    holds,
                    counterexample,
                    check_duration_ms: start.elapsed().as_millis() as u64,
                }
            })
            .collect()
    }
}
```

### Integration Plan

| Step | What | Where | Connects To |
|------|------|-------|-------------|
| 1 | Define `Invariant` and `InvariantKind` types | `roko-gate/src/invariant.rs` | Existing gate infrastructure |
| 2 | Load invariants from config | `roko.toml` `[invariants]` section | Per-domain invariant sets |
| 3 | Learn invariants from gate history | `roko-learn` pattern mining | Successful patterns → Range/Order invariants |
| 4 | Wire `VerifiedDreamRunner` into dream cycle | `roko-dreams/src/runner.rs` | Replace plain REM with verified REM |
| 5 | AntiKnowledge creation on violation | `roko-neuro` | Dream violations → permanent warnings |
| 6 | Dream quality metrics → efficiency log | `.roko/learn/dream-quality.jsonl` | Verified ratio as quality signal |
| 7 | Invariant learning from code intelligence | `roko-index` symbol types | Type invariants from parsed signatures |

### Test Criteria

- [ ] Known-bad strategy (budget overflow) correctly produces Violated verdict
- [ ] Known-good strategy (all invariants hold) produces Verified verdict
- [ ] AntiKnowledge created for critical violations prevents re-proposal in future dreams
- [ ] Verification adds <50ms overhead per scenario (for <20 invariants)
- [ ] Dream quality score correlates with staging-to-promotion rate (r > 0.5)
- [ ] Invariants learned from gate history match manually specified invariants (≥80% overlap)

---

## 4. Morphogenesis + Knowledge

**Knowledge Concentration Gradients Drive Agent Specialization**

### Motivation

Roko's coordination system already implements Turing reaction-diffusion for agent role
specialization (Gierer-Meinhardt dynamics, α=0.05, β=0.15, μ=0.01). But the activation
and inhibition signals are based on **task returns**—crude success/failure metrics.

Meanwhile, Neuro maintains rich knowledge with six types, four validation tiers, HDC vectors,
and Ebbinghaus decay. This knowledge is **not connected** to the morphogenetic process.
Agents specialize based on whether they succeed at tasks, not based on *what they know*.

The insight: **knowledge concentration should drive specialization**. An agent that has deep
Consolidated knowledge about testing should specialize in testing. An agent with rich
CausalLink knowledge about performance should specialize in optimization. The knowledge
landscape *is* the morphogenetic field.

### Research Basis

- **Richardson et al. (2024)** "Learning Spatio-Temporal Patterns with Neural Cellular
  Automata," *PLOS Computational Biology*. Trains Neural Cellular Automata to learn Turing
  pattern dynamics from PDE trajectories. NCAs learn local update rules that produce global
  emergent patterns—directly analogous to agents with local knowledge gradients self-organizing
  into specialized roles.

- **Shimizu et al. (2025)** "An Algorithm Applying the Self-Organizing Capabilities of a
  Reaction-Diffusion Model to Control Active Swarm Robots," *Journal of Intelligent & Robotic
  Systems*. Uses reaction-diffusion to control self-organizing modular robots, where modules
  differentiate phenotypes using only adjacent-module information.

- **Turing (1952)** "The Chemical Basis of Morphogenesis," *Philosophical Transactions of
  the Royal Society B*. The foundational insight: spatial pattern formation via local
  activation and long-range inhibition with diffusion asymmetry.

### Core Idea

Replace task-return-based morphogenetic activation with **knowledge concentration vectors**.
Each agent's Neuro store is projected into the 8-dimensional strategy space, producing a
"knowledge concentration gradient." High concentration in a dimension (e.g., many Consolidated
testing heuristics) drives activation. Knowledge shared via pheromones drives inhibition
(if another agent already has deep testing knowledge, don't compete).

```
Knowledge Concentration Gradient:

  For each strategy dimension k ∈ [0, 7]:
    concentration[k] = Σ (entry.confidence × tier_multiplier × relevance_to_k)
                       for entry in neuro_store
                       where entry.tags ∩ dimension_tags[k] ≠ ∅

  Morphogenetic update:
    activation[k] = α × concentration[k] × s[k]
    inhibition[k] = β × (pheromone_knowledge[k] / collective_size) × s[k]
    s[k] += activation[k] - inhibition[k] - μ × (s[k] - baseline) + noise

  Key difference from task-return activation:
    - Knowledge concentration is PROACTIVE (agent specializes based on what it knows)
    - Task returns are REACTIVE (agent specializes based on what worked)
    - Knowledge gradients are STABLE (decay slowly via Ebbinghaus)
    - Task returns are VOLATILE (single failure can flip strategy)
```

### Algorithm: Knowledge-Driven Morphogenesis

```
Algorithm: KnowledgeMorphogenesis

Input:
  neuro: NeuroStore                      — agent's knowledge base
  strategy: [f64; 8]                     — current specialization vector
  pheromone_field: [f64; 8]              — collective knowledge pheromones
  collective_size: usize                 — number of agents in collective
  dimension_tags: [[String]; 8]          — tag sets mapping knowledge to dimensions

Parameters:
  α = 0.03                               — activation rate (slower than task-based 0.05)
  β = 0.12                               — inhibition rate (β > α for Turing instability)
  μ = 0.008                              — decay toward baseline
  σ = 0.003                              — noise standard deviation
  baseline = 1/8 = 0.125                 — uniform baseline
  tier_weights = {Transient: 0.1, Working: 0.5, Consolidated: 1.0, Persistent: 5.0}

Output:
  strategy': [f64; 8]                    — updated specialization vector

Steps:
  1. COMPUTE KNOWLEDGE CONCENTRATION PER DIMENSION:
     For k in 0..8:
       concentration[k] = 0.0
       For entry in neuro.all_entries():
         relevance = jaccard(entry.tags, dimension_tags[k])
         if relevance > 0.0:
           tier_mult = tier_weights[entry.tier]
           effective_conf = entry.confidence × tier_mult × relevance
           concentration[k] += effective_conf

     Normalize: concentration[k] /= max(1.0, neuro.len() as f64)

  2. COMPUTE KNOWLEDGE PHEROMONE GRADIENT:
     // Each agent periodically deposits its concentration vector as a pheromone
     // The pheromone field is the aggregate of all agents' knowledge concentrations
     gradient[k] = concentration[k] - pheromone_field[k] / collective_size
     // Positive gradient = agent knows MORE than collective average → activate
     // Negative gradient = agent knows LESS than collective average → inhibit

  3. GIERER-MEINHARDT UPDATE (knowledge-driven):
     For k in 0..8:
       activation = α × max(0.0, gradient[k]) × strategy[k]
       inhibition = β × max(0.0, -gradient[k]) × strategy[k]
       decay = μ × (strategy[k] - baseline)
       noise = Normal(0, σ²).sample()
       strategy'[k] = strategy[k] + activation - inhibition - decay + noise

     Normalize: strategy' = softmax(strategy')  // Ensure sums to 1.0

  4. DEPOSIT KNOWLEDGE PHEROMONE:
     pheromone = Pheromone {
       kind: PheromoneKind::Wisdom,
       payload: concentration.to_vec(),
       decay_rate: Duration::from_secs(24 * 3600),  // 24h half-life
       scope: PheromoneScope::Mesh,
     }
     substrate.put(pheromone.into_signal())

  5. DETECT NICHE COMPETITION:
     For each peer in collective:
       sim = cosine_similarity(strategy, peer.strategy)
       if sim > 0.8:
         // Same niche—knowledge gradient will naturally push apart
         // Log for dashboard: "Niche competition with {peer.id}"
```

### Rust Sketch

```rust
use roko_core::Signal;

/// Knowledge concentration across strategy dimensions.
///
/// Each dimension accumulates confidence-weighted knowledge relevance,
/// producing a gradient field that drives morphogenetic specialization.
#[derive(Debug, Clone)]
pub struct KnowledgeConcentration {
    /// Concentration per strategy dimension (8D default)
    pub values: Vec<f64>,
    /// Tag sets mapping knowledge types to dimensions
    pub dimension_tags: Vec<Vec<String>>,
    /// Tier weight multipliers
    pub tier_weights: TierWeights,
}

#[derive(Debug, Clone)]
pub struct TierWeights {
    pub transient: f64,     // 0.1
    pub working: f64,       // 0.5
    pub consolidated: f64,  // 1.0
    pub persistent: f64,    // 5.0
}

impl Default for TierWeights {
    fn default() -> Self {
        Self { transient: 0.1, working: 0.5, consolidated: 1.0, persistent: 5.0 }
    }
}

/// Morphogenetic specialization engine driven by knowledge gradients.
pub struct KnowledgeMorphogenesis {
    /// Activation rate (knowledge advantage → specialization)
    pub alpha: f64,
    /// Inhibition rate (collective knowledge → avoid competition)
    pub beta: f64,
    /// Decay rate toward baseline
    pub mu: f64,
    /// Noise standard deviation
    pub sigma: f64,
    /// Baseline per dimension (1/D)
    pub baseline: f64,
    /// Concentration computer
    pub concentration: KnowledgeConcentration,
    /// RNG for noise injection
    rng: SmallRng,
}

impl KnowledgeMorphogenesis {
    /// Coding domain dimension tags (default 8D).
    pub fn coding_dimension_tags() -> Vec<Vec<String>> {
        vec![
            vec!["refactoring", "cleanup", "restructure", "rename"],
            vec!["feature", "implementation", "api", "endpoint"],
            vec!["testing", "test", "assertion", "coverage", "mock"],
            vec!["documentation", "docs", "readme", "comment"],
            vec!["performance", "optimization", "benchmark", "latency"],
            vec!["security", "auth", "validation", "sanitize"],
            vec!["dependency", "upgrade", "version", "package"],
            vec!["architecture", "design", "pattern", "abstraction"],
        ]
    }

    /// Compute knowledge concentration from NeuroStore contents.
    pub fn compute_concentration(
        &self,
        neuro: &dyn NeuroStore,
    ) -> Vec<f64> {
        let entries = neuro.all_entries();
        let mut concentration = vec![0.0; self.concentration.dimension_tags.len()];

        for entry in &entries {
            for (k, dim_tags) in self.concentration.dimension_tags.iter().enumerate() {
                let relevance = jaccard_similarity(&entry.tags, dim_tags);
                if relevance > 0.0 {
                    let tier_mult = match entry.tier {
                        Tier::Transient => self.concentration.tier_weights.transient,
                        Tier::Working => self.concentration.tier_weights.working,
                        Tier::Consolidated => self.concentration.tier_weights.consolidated,
                        Tier::Persistent => self.concentration.tier_weights.persistent,
                    };
                    concentration[k] += entry.confidence * tier_mult * relevance;
                }
            }
        }

        // Normalize by knowledge base size
        let norm = entries.len().max(1) as f64;
        for c in &mut concentration {
            *c /= norm;
        }

        concentration
    }

    /// One morphogenetic update step.
    ///
    /// Uses knowledge concentration gradient (local vs. collective)
    /// as the activation/inhibition signal for Gierer-Meinhardt dynamics.
    pub fn update(
        &mut self,
        strategy: &mut [f64],
        local_concentration: &[f64],
        collective_pheromone: &[f64],
        collective_size: usize,
    ) {
        let d = strategy.len();
        let coll_avg_divisor = collective_size.max(1) as f64;

        for k in 0..d {
            let gradient = local_concentration[k]
                - collective_pheromone[k] / coll_avg_divisor;

            let activation = self.alpha * gradient.max(0.0) * strategy[k];
            let inhibition = self.beta * (-gradient).max(0.0) * strategy[k];
            let decay = self.mu * (strategy[k] - self.baseline);
            let noise = Normal::new(0.0, self.sigma)
                .expect("valid sigma")
                .sample(&mut self.rng);

            strategy[k] += activation - inhibition - decay + noise;
            strategy[k] = strategy[k].max(0.001); // Floor to prevent extinction
        }

        // Normalize to sum to 1.0 (softmax-like)
        let sum: f64 = strategy.iter().sum();
        if sum > 0.0 {
            for s in strategy.iter_mut() {
                *s /= sum;
            }
        }
    }
}

fn jaccard_similarity(a: &[String], b: &[String]) -> f64 {
    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
}
```

### Integration Plan

| Step | What | Where | Connects To |
|------|------|-------|-------------|
| 1 | Add `KnowledgeConcentration` to coordination | `roko-conductor/src/morphogenesis.rs` | Existing morphogenetic state |
| 2 | Wire NeuroStore query into concentration | `roko-neuro` → `roko-conductor` | Read-only cross-crate access |
| 3 | Replace task-return activation with gradient | `MorphogeneticState::update()` | α × gradient replaces α × returns |
| 4 | Deposit concentration as Wisdom pheromone | `roko-conductor/src/pheromone.rs` | Existing pheromone infrastructure |
| 5 | Dashboard visualization | `roko-cli/src/tui/` | Strategy heatmap per agent |
| 6 | Log specialization trajectory | `.roko/learn/specialization.jsonl` | For dream replay analysis |

### Test Criteria

- [ ] Agent with deep testing knowledge specializes in testing dimension (>3× baseline)
- [ ] Two agents with identical knowledge differentiate via inhibition within 100 ticks
- [ ] Knowledge tier multipliers cause Consolidated knowledge to dominate Transient
- [ ] Noise injection breaks initial symmetry within 50 ticks
- [ ] Specialization index monotonically increases for first 200 ticks (convergence)
- [ ] Niche competition detected when cosine similarity > 0.8

---

## 5. Bandits + Pheromones

**Pheromone Trails as Bandit Arms**

### Motivation

Roko has two parallel selection mechanisms that solve the same problem—explore/exploit—using
completely different paradigms:

- **Bandits** (UCB1, LinUCB, Thompson Sampling): Per-agent, internal. Each agent maintains
  its own reward estimates and exploration bonuses. No inter-agent learning. Cold starts
  everywhere.
- **Pheromones** (stigmergy): Inter-agent, external. Agents deposit and sense environmental
  traces. Rich spatial structure. But no formal optimality guarantees—pheromone decay rates
  are hand-tuned, not learned.

The synthesis: treat each pheromone trail as a **bandit arm**. The intensity of a pheromone
trail encodes the accumulated reward. Exploration bonus comes from trail *age* (older trails
are less explored recently). New agents inherit the collective's exploration history via the
pheromone field instead of cold-starting.

### Research Basis

- **Chari et al. (2025)** "Pheromone-based Learning of Optimal Reasoning Paths" (ACO-ToT),
  arXiv:2501.19278. Uses distinctly fine-tuned LLM "ants" that deposit pheromone on reasoning
  paths. A mixture-of-experts scoring combines pheromone concentration with specialized
  expertise. Pheromone reinforcement outperforms standard chain-of-thought on GSM8K, ARC, MATH.

- **Li, Zhu et al. (2024)** "PooL: Pheromone-inspired Communication Framework for Large-Scale
  MARL," arXiv:2202.09722. Defines pheromones as RL agent outputs reflecting views of the
  environment. Achieves higher rewards with lower communication cost than SOTA methods.

- **Dorigo & Stützle (2004)** *Ant Colony Optimization*. The foundational framework: ants
  deposit pheromone on successful paths, pheromone evaporates over time, creating a
  distributed optimization algorithm with convergence guarantees under ergodicity assumptions.

- **Auer, Cesa-Bianchi & Fischer (2002)** "Finite-time Analysis of the Multiarmed Bandit
  Problem," *Machine Learning* 47(2-3). UCB1 achieves O(√(KT ln T)) regret—optimal up to
  logarithmic factors.

### Core Idea

Define a **PheromoneArm** that wraps a pheromone trail as a bandit arm. The arm's estimated
reward is the trail's current intensity (decayed). The exploration bonus is a UCB-style
term based on how recently the trail was last reinforced. When an agent follows a trail and
succeeds, it deposits pheromone (reinforcement). When it fails, pheromone decays faster
(anti-reinforcement). New agents joining the collective inherit arms pre-initialized from
the pheromone field—no cold start.

```
Pheromone-Bandit Correspondence:

  Bandit Concept          Pheromone Analog
  ─────────────          ────────────────
  Arm                    Pheromone trail (kind + scope + location)
  Estimated reward       Trail intensity × depositor reputation
  Pull count             Number of reinforcements
  Exploration bonus      C × √(ln(total_pulls) / trail_pulls)
  Reward update          Deposit (success) or anti-deposit (failure)
  Arm creation           New trail deposited by any agent
  Arm elimination        Trail intensity decays below threshold
```

### Algorithm: Stigmergic Bandits

```
Algorithm: StigmergicBandit

Input:
  field: PheromoneField                  — shared environment
  agent_rewards: HashMap<TrailId, f64>   — agent's local reward estimates
  C: f64                                 — exploration constant (default √2)

Output:
  selected: TrailId                      — which trail to follow
  action: TrailAction                    — Follow, Explore (new trail), or Reinforce

Steps:
  1. ENUMERATE ARMS FROM FIELD:
     arms = field.active_trails()
       .map(|trail| PheromoneArm {
         id: trail.id,
         estimated_reward: trail.intensity × trail.depositor_reputation,
         pulls: trail.reinforcement_count,
         last_reinforced: trail.last_deposit_time,
         kind: trail.kind,
       })

  2. COMPUTE UCB SCORES:
     total_pulls = arms.iter().map(|a| a.pulls).sum()
     For each arm a:
       // Local reward (agent's own experience) blended with pheromone
       local_reward = agent_rewards.get(a.id).unwrap_or(a.estimated_reward)
       blended = 0.7 × local_reward + 0.3 × a.estimated_reward
       // Exploration bonus (UCB1)
       exploration = C × √(ln(total_pulls + 1) / (a.pulls + 1))
       // Recency bonus (recently reinforced trails are more trustworthy)
       age_penalty = 1.0 / (1.0 + hours_since(a.last_reinforced) × 0.1)
       // Final score
       a.ucb_score = blended × age_penalty + exploration

  3. SELECT ARM:
     If uniform_random() < ε (exploration rate, default 0.05):
       // Pure exploration: create new trail
       selected = create_random_trail()
       action = Explore
     Else:
       selected = arms.max_by(|a| a.ucb_score)
       action = Follow

  4. EXECUTE AND OBSERVE REWARD:
     reward = execute_along_trail(selected)

  5. UPDATE:
     // Local bandit update
     agent_rewards[selected] = (1 - lr) × agent_rewards[selected] + lr × reward
     // Pheromone deposit/anti-deposit
     If reward > 0.5:
       field.deposit(selected, intensity = reward, PheromoneKind::Opportunity)
     Else:
       field.anti_deposit(selected, decay_multiplier = 2.0)
       // Double decay rate = anti-pheromone (discouragement)

  6. COLD START ELIMINATION:
     // New agents inherit field as prior
     For each trail in field.active_trails():
       if trail.id not in agent_rewards:
         agent_rewards[trail.id] = trail.intensity × 0.5  // Discounted trust
```

### Rust Sketch

```rust
use roko_learn::bandits::{BanditArm, UcbBandit};

/// A pheromone trail wrapped as a bandit arm.
///
/// Intensity encodes estimated reward. Reinforcement count encodes pulls.
/// Exploration bonus is standard UCB1. Decay provides natural arm elimination.
#[derive(Debug, Clone)]
pub struct PheromoneArm {
    /// Trail identifier (content hash of original deposit)
    pub trail_id: ContentHash,
    /// Current intensity after decay
    pub intensity: f64,
    /// Number of times this trail was reinforced
    pub reinforcement_count: u32,
    /// Depositor's reputation score
    pub depositor_reputation: f64,
    /// Pheromone kind (Opportunity, Wisdom, Pattern, etc.)
    pub kind: PheromoneKind,
    /// Last reinforcement timestamp
    pub last_reinforced_ms: i64,
}

impl PheromoneArm {
    /// Estimated reward: intensity × reputation, decayed by age.
    pub fn estimated_reward(&self, now_ms: i64) -> f64 {
        let age_hours = (now_ms - self.last_reinforced_ms) as f64 / 3_600_000.0;
        let age_penalty = 1.0 / (1.0 + age_hours * 0.1);
        self.intensity * self.depositor_reputation * age_penalty
    }
}

/// Stigmergic bandit: combines UCB1 exploration with pheromone-encoded rewards.
pub struct StigmergicBandit {
    /// Local reward estimates (agent's own experience)
    pub local_rewards: HashMap<ContentHash, f64>,
    /// Local pull counts
    pub local_pulls: HashMap<ContentHash, u32>,
    /// Exploration constant C (default √2 ≈ 1.414)
    pub exploration_constant: f64,
    /// Blend weight for local vs. pheromone reward (default 0.7 local)
    pub local_weight: f64,
    /// Pure exploration rate ε (default 0.05)
    pub epsilon: f64,
    /// Total selections made
    pub total_pulls: u64,
}

impl StigmergicBandit {
    /// Select which pheromone trail to follow.
    ///
    /// Blends local experience with collective pheromone field,
    /// applies UCB1 exploration bonus, and selects highest-scoring arm.
    pub fn select(&mut self, field: &PheromoneField, now_ms: i64) -> TrailSelection {
        let arms: Vec<PheromoneArm> = field
            .active_trails()
            .map(|trail| PheromoneArm {
                trail_id: trail.content_hash(),
                intensity: trail.current_intensity(now_ms),
                reinforcement_count: trail.reinforcement_count,
                depositor_reputation: trail.depositor_reputation,
                kind: trail.kind,
                last_reinforced_ms: trail.last_deposit_ms,
            })
            .collect();

        if arms.is_empty() {
            return TrailSelection::Explore;
        }

        // ε-greedy exploration
        if self.should_explore() {
            return TrailSelection::Explore;
        }

        // UCB1 selection over pheromone arms
        let best = arms.iter()
            .max_by(|a, b| {
                let score_a = self.ucb_score(a, now_ms);
                let score_b = self.ucb_score(b, now_ms);
                score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("arms non-empty");

        self.total_pulls += 1;
        *self.local_pulls.entry(best.trail_id).or_insert(0) += 1;

        TrailSelection::Follow(best.trail_id)
    }

    /// UCB1 score blending local experience with pheromone intensity.
    fn ucb_score(&self, arm: &PheromoneArm, now_ms: i64) -> f64 {
        let pheromone_reward = arm.estimated_reward(now_ms);
        let local_reward = self.local_rewards
            .get(&arm.trail_id)
            .copied()
            .unwrap_or(pheromone_reward * 0.5); // Discounted trust for new trails

        let blended = self.local_weight * local_reward
            + (1.0 - self.local_weight) * pheromone_reward;

        let local_count = self.local_pulls
            .get(&arm.trail_id)
            .copied()
            .unwrap_or(0) as f64;

        let exploration = self.exploration_constant
            * ((self.total_pulls as f64 + 1.0).ln() / (local_count + 1.0)).sqrt();

        blended + exploration
    }

    /// Update after following a trail: deposit or anti-deposit pheromone.
    pub fn update(
        &mut self,
        trail_id: ContentHash,
        reward: f64,
        field: &mut PheromoneField,
    ) {
        // Local bandit update (exponential moving average)
        let lr = 0.1;
        let prev = self.local_rewards.get(&trail_id).copied().unwrap_or(0.5);
        self.local_rewards.insert(trail_id, (1.0 - lr) * prev + lr * reward);

        // Pheromone feedback
        if reward > 0.5 {
            field.reinforce(trail_id, reward);
        } else {
            field.anti_reinforce(trail_id, 2.0); // Double decay for failures
        }
    }

    /// Initialize from pheromone field (cold start elimination).
    pub fn warm_start(&mut self, field: &PheromoneField, now_ms: i64) {
        for trail in field.active_trails() {
            let arm = PheromoneArm {
                trail_id: trail.content_hash(),
                intensity: trail.current_intensity(now_ms),
                reinforcement_count: trail.reinforcement_count,
                depositor_reputation: trail.depositor_reputation,
                kind: trail.kind,
                last_reinforced_ms: trail.last_deposit_ms,
            };
            // Inherit with 50% discount (untested by this agent)
            self.local_rewards
                .entry(arm.trail_id)
                .or_insert(arm.estimated_reward(now_ms) * 0.5);
        }
    }
}

pub enum TrailSelection {
    Follow(ContentHash),
    Explore,
}
```

### Integration Plan

| Step | What | Where | Connects To |
|------|------|-------|-------------|
| 1 | Add `StigmergicBandit` to `roko-learn` | `roko-learn/src/stigmergic.rs` | Existing bandits module |
| 2 | Wrap CascadeRouter trail selection | `roko-learn/src/cascade_router.rs` | Models as trails + arms |
| 3 | Cold start from pheromone field | Agent initialization | New agents inherit field rewards |
| 4 | Pheromone deposit on task completion | `roko-cli/src/orchestrate.rs` | After gate verdict |
| 5 | Anti-pheromone on gate failure | `roko-cli/src/orchestrate.rs` | Double decay for failed trails |
| 6 | Dashboard: trail intensity heatmap | `roko-cli/src/tui/` | Visual pheromone field |

### Test Criteria

- [ ] New agent inherits reward estimates from 10 existing trails within 1 tick
- [ ] UCB exploration bonus decreases as trail is reinforced (convergence)
- [ ] Anti-pheromone causes trail intensity to decay 2× faster than natural decay
- [ ] Stigmergic bandit achieves lower cumulative regret than isolated UCB1 (≥20% improvement)
- [ ] Trail intensity and bandit estimated reward correlate (r > 0.8)
- [ ] Dead trails (intensity < 0.01) are automatically pruned from arm set

---

## 6. Witness DAG + Active Inference

**Agent History IS Its World Model**

### Motivation

Roko's Witness DAG records cryptographic commitments of every cognitive event: Observations,
Predictions, Decisions, Resolutions, and NeuroEntries. It is currently used for **forensic
analysis**—post-facto auditing, hallucination detection, provenance queries. But the DAG
contains far more information than is being extracted.

The insight from active inference: an agent's **generative model** (its beliefs about how
the world works) is precisely the structure that predicts future observations from past
actions. The Witness DAG already *contains* this structure—the edges from Observations to
Predictions to Decisions to Resolutions trace exactly the causal chains the agent believes
in. The DAG doesn't just *record* history; it **is** the world model.

### Research Basis

- **Richens & Everitt (2024)** "Robust Agents Learn Causal World Models," ICLR 2024 (Oral),
  arXiv:2402.10877. Proves that any agent achieving bounded regret under distributional shifts
  must have learned an approximate causal model (a DAG) of its environment. The agent's
  behavioral history of adapting to distribution shifts implicitly encodes a DAG that converges
  to the true causal graph for optimal agents.

- **Gkountouras et al. (2024)** "Language Agents Meet Causality—Bridging LLMs and Causal
  World Models," arXiv:2410.19923. Builds a causal world model as a learned DAG with
  variables linked to natural language, providing LLMs with a structured interface for
  interventional reasoning. The DAG serves as both world model and planning substrate.

- **Deng et al. (2025)** "A Roadmap Towards Improving Multi-Agent RL with Causal Discovery
  and Inference," arXiv:2503.17803. Describes learning structural causal models from
  multi-agent interaction histories for coordination, credit assignment, and transfer learning.

- **Conant & Ashby (1970)** "Every Good Regulator of a System Must Be a Model of That System."
  The Good Regulator Theorem: effective control requires an internal model isomorphic to the
  controlled system. The Witness DAG is that isomorph.

### Core Idea

Extract a **causal world model** from the Witness DAG by analyzing statistical regularities
in the O→P→D→R chains. Prediction vertices that consistently predict correctly create
**strong causal edges** in the model. Predictions that fail create **weak edges** or reveal
**confounders**. The resulting causal DAG becomes the agent's generative model for active
inference—replacing the HDC belief vector (Innovation 1) with a richer, graph-structured
model for agents that have sufficient history.

```
Witness DAG → Causal World Model extraction:

  1. Collect all Prediction vertices with their Resolution outcomes
  2. Group by prediction type (e.g., "this code will compile")
  3. For each prediction type:
     a. Extract parent Observations (what informed the prediction)
     b. Extract child Resolution (what actually happened)
     c. Compute: accuracy = correct_resolutions / total_predictions
     d. Create causal edge: Observation_type → Outcome_type
        weight = accuracy × num_observations
  4. Prune edges with accuracy < 0.3 (likely spurious)
  5. Result: Causal DAG where edges represent learned causal beliefs
     weighted by empirical accuracy

Active Inference using the model:
  Expected Free Energy of action a:
    G(a) = -Σ_o P(o|a, model) × [ln P(o|a, model) - ln P(o|desired)]
    where P(o|a, model) is predicted from the Witness-derived causal DAG
```

### Algorithm: Witness-Derived World Model

```
Algorithm: WitnessWorldModel

Input:
  dag: WitnessDAG                        — agent's complete cognitive trace
  min_observations: usize                 — minimum sample size (default 10)
  accuracy_threshold: f64                 — minimum edge accuracy (default 0.3)

Output:
  world_model: CausalWorldModel           — extracted causal DAG
  prediction_calibration: CalibrationMap   — per-type prediction accuracy

Steps:
  1. EXTRACT PREDICTION-RESOLUTION PAIRS:
     pairs = []
     For each vertex v in dag where v.type == Prediction:
       resolutions = dag.children(v.hash)
         .filter(|c| c.type == Resolution)
       observations = dag.ancestors(v.hash)
         .filter(|a| a.type == Observation)
       For each resolution r:
         pairs.push(PredictionOutcome {
           prediction: v,
           resolution: r,
           observations: observations.clone(),
           correct: prediction_matches(v, r),
         })

  2. GROUP BY PREDICTION TYPE:
     groups: HashMap<PredictionType, Vec<PredictionOutcome>>
     // PredictionType = hash(prediction_category + observation_types)

  3. COMPUTE CAUSAL EDGES:
     For each (pred_type, outcomes) in groups:
       if outcomes.len() < min_observations: continue

       accuracy = outcomes.iter().filter(|o| o.correct).count() as f64
                  / outcomes.len() as f64

       if accuracy < accuracy_threshold: continue

       // Extract which observation types predict this outcome
       obs_types = mode(outcomes.iter().flat_map(|o| o.observation_types()))

       world_model.add_edge(CausalEdge {
         from: obs_types,
         to: pred_type.outcome_type,
         weight: accuracy × outcomes.len() as f64,
         accuracy,
         sample_size: outcomes.len(),
       })

  4. DETECT CONFOUNDERS:
     For each edge (A → C) in world_model:
       For each other variable B:
         if P(C | A, B) ≈ P(C | B):
           // A does not cause C; B is the true cause
           // Demote edge A → C, promote edge B → C
           world_model.weaken(A, C)
           world_model.strengthen(B, C)

  5. COMPUTE EXPECTED FREE ENERGY:
     For each candidate action a:
       // Simulate outcome distribution using world model edges
       predicted_outcomes = world_model.simulate(current_observations, a)
       pragmatic_value = E[utility(outcomes)]
       epistemic_value = H[outcomes]  // Shannon entropy of predicted outcomes
       G(a) = -pragmatic_value - epistemic_value
     Select action with lowest G(a) (most informative + valuable)

  6. CALIBRATION UPDATE:
     For each prediction type:
       calibration[type] = accuracy
       if calibration_drift(type) > 0.1:
         emit CognitiveSignal::Explore  // Model may be stale
```

### Rust Sketch

```rust
use roko_core::ContentHash;

/// A causal edge in the world model, extracted from Witness DAG statistics.
#[derive(Debug, Clone)]
pub struct WorldModelEdge {
    /// Source observation/state type
    pub from: String,
    /// Target outcome type
    pub to: String,
    /// Causal strength: accuracy × sample_size (weighted evidence)
    pub weight: f64,
    /// Empirical accuracy of this causal link
    pub accuracy: f64,
    /// Number of observations supporting this edge
    pub sample_size: usize,
    /// Last updated timestamp
    pub updated_ms: i64,
}

/// Causal world model extracted from the Witness DAG.
///
/// Each edge represents an empirically observed causal relationship:
/// "when I observe X and do Y, outcome Z follows with probability p."
/// The model is the agent's learned understanding of its environment.
pub struct CausalWorldModel {
    /// Adjacency list: from → [(to, edge)]
    pub edges: HashMap<String, Vec<WorldModelEdge>>,
    /// Per-prediction-type calibration
    pub calibration: HashMap<String, f64>,
    /// Model age (ticks since last full reconstruction)
    pub age_ticks: u64,
    /// Reconstruction interval
    pub rebuild_interval: u64,
}

impl CausalWorldModel {
    /// Extract causal world model from Witness DAG.
    ///
    /// Analyzes Prediction→Resolution chains to discover empirical
    /// causal relationships. The resulting DAG is the agent's
    /// generative model for active inference.
    pub fn from_witness_dag(
        dag: &WitnessDAG,
        min_observations: usize,
        accuracy_threshold: f64,
    ) -> Self {
        let mut model = Self {
            edges: HashMap::new(),
            calibration: HashMap::new(),
            age_ticks: 0,
            rebuild_interval: 500,
        };

        // 1. Extract prediction-resolution pairs
        let pairs = dag.prediction_resolution_pairs();

        // 2. Group by prediction type
        let groups = Self::group_by_type(&pairs);

        // 3. Create causal edges for sufficiently supported predictions
        for (pred_type, outcomes) in &groups {
            if outcomes.len() < min_observations {
                continue;
            }

            let correct = outcomes.iter().filter(|o| o.correct).count();
            let accuracy = correct as f64 / outcomes.len() as f64;

            if accuracy < accuracy_threshold {
                continue;
            }

            // Find dominant observation types that predict this outcome
            let obs_types = Self::dominant_observations(outcomes);

            for obs_type in &obs_types {
                model.edges
                    .entry(obs_type.clone())
                    .or_default()
                    .push(WorldModelEdge {
                        from: obs_type.clone(),
                        to: pred_type.clone(),
                        weight: accuracy * outcomes.len() as f64,
                        accuracy,
                        sample_size: outcomes.len(),
                        updated_ms: now_ms(),
                    });
            }

            model.calibration.insert(pred_type.clone(), accuracy);
        }

        model
    }

    /// Compute Expected Free Energy for a candidate action.
    ///
    /// G(a) = -pragmatic_value - epistemic_value
    /// Lower G → better action (more informative + valuable).
    pub fn expected_free_energy(
        &self,
        observations: &[String],
        action: &str,
    ) -> f64 {
        // Predict outcome distribution using causal edges
        let mut outcome_probs: HashMap<String, f64> = HashMap::new();
        let mut total_weight = 0.0;

        for obs in observations {
            if let Some(edges) = self.edges.get(obs) {
                for edge in edges {
                    *outcome_probs.entry(edge.to.clone()).or_insert(0.0)
                        += edge.weight;
                    total_weight += edge.weight;
                }
            }
        }

        if total_weight == 0.0 {
            return 0.0; // No model → maximum uncertainty → explore
        }

        // Normalize to probabilities
        for prob in outcome_probs.values_mut() {
            *prob /= total_weight;
        }

        // Pragmatic value: expected utility of outcomes
        let pragmatic: f64 = outcome_probs.iter()
            .map(|(outcome, prob)| prob * self.utility_of(outcome))
            .sum();

        // Epistemic value: entropy of outcome distribution (higher = more uncertain = more to learn)
        let epistemic: f64 = outcome_probs.values()
            .filter(|&&p| p > 0.0)
            .map(|&p| -p * p.ln())
            .sum();

        -(pragmatic + epistemic)
    }

    /// Detect when the world model is stale (calibration drifting).
    pub fn is_stale(&self, recent_predictions: &[(String, bool)]) -> bool {
        for (pred_type, correct) in recent_predictions {
            if let Some(&historical_accuracy) = self.calibration.get(pred_type) {
                let recent_accuracy = if *correct { 1.0 } else { 0.0 };
                if (recent_accuracy - historical_accuracy).abs() > 0.2 {
                    return true; // Significant calibration drift
                }
            }
        }
        false
    }
}
```

### Integration Plan

| Step | What | Where | Connects To |
|------|------|-------|-------------|
| 1 | Add `CausalWorldModel` to `roko-core` | `roko-core/src/world_model.rs` | Adjacent to Witness DAG |
| 2 | Extract model during Theta reflection | `roko-cli/src/orchestrate.rs` | Every 500 ticks |
| 3 | Use EFE for action selection | Heartbeat step 3 (ATTEND) | Replace heuristic routing |
| 4 | Calibration monitoring | `roko-learn/src/regression.rs` | Staleness → CognitiveSignal::Explore |
| 5 | Persist model | `.roko/state/world-model.json` | Edges + calibration |
| 6 | Cross-agent model sharing | Mesh pheromone (Wisdom kind) | Agents share discovered edges |
| 7 | Dream replay enriches model | Delta consolidation | NREM strengthens edges, REM adds novel ones |

### Test Criteria

- [ ] Model correctly extracts "compilation error → test failure" causal edge from synthetic DAG
- [ ] Confounder detection removes spurious correlation when true cause is identified
- [ ] EFE selects informative actions (high epistemic value) when model is uncertain
- [ ] Staleness detection fires within 20 predictions when environment changes
- [ ] Model reconstruction < 50ms for DAG with 10,000 vertices
- [ ] Calibration accuracy improves monotonically over 1,000 predictions (convergence)

---

## 7. Somatic Markers + Code Intelligence

**Code Smells Trigger Somatic Markers**

### Motivation

Roko's Daimon has somatic markers (Damasio 1994)—fast pre-analytical gut feelings stored in
a k-d tree over the 8-dimensional strategy space. Separately, the code intelligence system
(roko-index) computes structural metrics: PageRank importance, HDC fingerprints, dependency
depth, cyclomatic complexity.

The gap: code metrics exist as cold numbers. Somatic markers exist as emotional heuristics.
They're not connected. When an agent encounters code with high cyclomatic complexity, deep
dependency chains, and low test coverage, it *should* feel uneasy—a somatic marker should
fire saying "this is dangerous territory, use Conservative strategy." But currently the
agent treats all code regions equally until a gate fails.

The insight from neuroscience: **somatic markers are faster than analysis**. Damasio showed
that patients with ventromedial prefrontal cortex damage can still reason about risks
analytically but cannot *feel* danger—and they make catastrophically bad decisions because
analytical reasoning is too slow for real-time choice. Code intelligence metrics should
create somatic markers that fire **before** the agent begins working on a code region.

### Research Basis

- **Fakhoury et al. (2024)** "EEG as a Potential Ground Truth for Cognitive State in Software
  Development Activities," *PLOS ONE*. Validates that developers' neural signals during code
  comprehension reliably predict cognitive difficulty. Code regions triggering high cognitive
  load are the computational analog of somatic markers flagging problematic decisions.

- **Pargaonkar et al. (2024)** "Quality Evaluation of Modern Code Reviews Through Intelligent
  Biometric Program Comprehension." Captures HRV and pupillary response during code review.
  AI predicts review quality from biomarkers with 87.77% accuracy. Physiological stress markers
  during code review directly predict review quality.

- **Kaur et al. (2025)** "Towards Decoding Developer Cognition in the Age of AI Assistants,"
  arXiv:2501.02684. Operationalizes the somatic marker hypothesis for programming: developers'
  physiological responses to code serve as implicit quality signals.

- **Damasio (1994)** *Descartes' Error: Emotion, Reason, and the Human Brain*. Somatic markers
  bias decision-making toward options associated with positive outcomes and away from options
  associated with negative outcomes—before conscious reasoning begins.

### Core Idea

Map code intelligence metrics onto the Daimon's 8-dimensional strategy space and create
**automatic somatic markers** from historical gate results. When an agent is about to work on
a code region, the somatic landscape is queried with the region's metric profile. If similar
metric profiles have historically led to failures, a negative somatic marker fires—biasing
the agent toward Conservative strategy, T2 escalation, and additional verification.

```
Code Region → Strategy Space Mapping:

  Dimension 0 (Complexity):    cyclomatic_complexity / max_complexity
  Dimension 1 (Risk):          (1 - test_coverage) × reverse_dep_count / max_deps
  Dimension 2 (Novelty):       1 - max(hdc_similarity(region, known_patterns))
  Dimension 3 (Confidence):    agent's Daimon dominance (current PAD.D)
  Dimension 4 (Time Pressure): deadline_proximity × blocker_count
  Dimension 5 (Scope):         files_modified × lines_changed / max_scope
  Dimension 6 (Reversibility): is_additive ? 0.8 : 0.2
  Dimension 7 (Dep Depth):     transitive_dep_count / max_transitive

Somatic Marker Creation (after gate verdict):
  If |PAD_delta| > 0.15 (significant emotional event):
    coords = compute_strategy_coords(code_region_metrics)
    marker = SomaticMarker {
      strategy_coords: coords,
      valence: if gate_passed { +PAD_delta.magnitude() }
               else { -PAD_delta.magnitude() },
      intensity: gate_rung_weight × abs(PAD_delta),
      episodes: [episode_id],
    }
    somatic_landscape.insert(marker)

Somatic Query (before starting work on code region):
  coords = compute_strategy_coords(code_region_metrics)
  nearby = somatic_landscape.nearest(coords, k=5, radius=0.5)
  avg_valence = nearby.iter().map(|m| m.valence).mean()

  If avg_valence < -0.5:
    → Bias toward Conservative + T2 (danger zone)
  If avg_valence > +0.5:
    → Bias toward Exploratory + T0/T1 (safe territory)
```

### Algorithm: Code-Aware Somatic Markers

```
Algorithm: CodeSomaticMarkers

Input:
  region: CodeRegion                     — file/function being worked on
  symbol_graph: SymbolGraph              — dependency graph from roko-index
  pagerank: HashMap<SymbolId, f64>       — importance scores
  fingerprints: HashMap<SymbolId, HdcFingerprint> — HDC fingerprints
  somatic_landscape: SomaticLandscape    — k-d tree of existing markers
  pad: PadVector                         — current affect state

Output:
  strategy_bias: DispatchStrategy        — recommended strategy
  tier_bias: InferenceTier               — recommended tier
  warnings: Vec<String>                  — human-readable risk signals

Steps:
  1. COMPUTE CODE METRICS FOR REGION:
     complexity = cyclomatic_complexity(region) / MAX_COMPLEXITY
     coverage = test_coverage(region)            // [0, 1]
     rev_deps = symbol_graph.reverse_neighbors(region.primary_symbol).len()
     trans_deps = symbol_graph.transitive(region.primary_symbol, 10).len()
     pagerank_score = pagerank[region.primary_symbol]
     novelty = 1.0 - max_hdc_similarity(region, fingerprints)

  2. MAP TO STRATEGY SPACE:
     coords = [
       complexity,                                    // dim 0
       (1.0 - coverage) × (rev_deps as f64 / 50.0),  // dim 1
       novelty,                                       // dim 2
       (pad.dominance + 1.0) / 2.0,                   // dim 3
       time_pressure(),                               // dim 4
       scope_metric(region),                           // dim 5
       reversibility(region),                          // dim 6
       trans_deps as f64 / 100.0,                      // dim 7
     ]

  3. QUERY SOMATIC LANDSCAPE:
     markers = somatic_landscape.nearest_within(coords, radius=0.5, k=10)
     if markers.is_empty():
       // No prior experience with similar code → neutral
       return (Balanced, T1, vec!["No prior experience with similar code metrics"])

  4. COMPUTE AGGREGATE VALENCE:
     // Intensity-weighted mean valence
     total_intensity = markers.iter().map(|m| m.intensity).sum()
     avg_valence = markers.iter()
       .map(|m| m.valence × m.intensity)
       .sum() / total_intensity

  5. GENERATE BIAS:
     If avg_valence < -0.5:
       strategy_bias = Conservative
       tier_bias = T2
       warnings.push("Somatic warning: similar code regions have
                       historically caused failures")
     Elif avg_valence < -0.2:
       strategy_bias = Balanced
       tier_bias = T1
       warnings.push("Somatic caution: mixed outcomes in similar regions")
     Elif avg_valence > 0.5:
       strategy_bias = Exploratory
       tier_bias = T0
       // No warning needed—safe territory
     Else:
       strategy_bias = Balanced
       tier_bias = T1

  6. ADDITIONAL CODE SMELL MARKERS:
     // High PageRank + low coverage = dangerous hot spot
     if pagerank_score > 0.01 && coverage < 0.5:
       warnings.push("Hot spot: high-importance symbol with low test coverage")
       tier_bias = max(tier_bias, T1)

     // Deep transitive dependencies = blast radius risk
     if trans_deps > 50:
       warnings.push("Deep dependency tree: changes may cascade")
       strategy_bias = Conservative

     // Novel code (low HDC similarity to known patterns)
     if novelty > 0.8:
       warnings.push("Novel code pattern: no similar patterns in knowledge base")
       tier_bias = max(tier_bias, T1)
```

### Rust Sketch

```rust
use roko_index::{SymbolGraph, SymbolId};
use bardo_primitives::hdc::HdcFingerprint;

/// Code region metrics projected into the somatic marker strategy space.
#[derive(Debug, Clone)]
pub struct CodeRegionMetrics {
    /// Cyclomatic complexity normalized to [0, 1]
    pub complexity: f64,
    /// Risk score: (1 - coverage) × normalized reverse dependency count
    pub risk: f64,
    /// Novelty: 1 - max HDC similarity to known patterns
    pub novelty: f64,
    /// Agent's current confidence (PAD dominance, mapped to [0, 1])
    pub confidence: f64,
    /// Time pressure metric
    pub time_pressure: f64,
    /// Scope: files × lines normalized
    pub scope: f64,
    /// Reversibility: additive changes score high
    pub reversibility: f64,
    /// Transitive dependency depth normalized
    pub dependency_depth: f64,
}

impl CodeRegionMetrics {
    /// Convert to 8-dimensional strategy coordinates for k-d tree query.
    pub fn to_coords(&self) -> [f64; 8] {
        [
            self.complexity,
            self.risk,
            self.novelty,
            self.confidence,
            self.time_pressure,
            self.scope,
            self.reversibility,
            self.dependency_depth,
        ]
    }
}

/// Somatic marker bias computed from code region metrics.
#[derive(Debug, Clone)]
pub struct CodeSomaticBias {
    pub strategy: DispatchStrategy,
    pub tier: InferenceTier,
    pub warnings: Vec<String>,
    pub avg_valence: f64,
    pub marker_count: usize,
    pub query_time_us: u64,
}

/// Computes somatic bias for a code region before the agent begins work.
///
/// Queries the Daimon's somatic landscape with code-derived coordinates.
/// Returns a strategy/tier bias that reflects historical outcomes in
/// similar code regions. This is the "gut feeling" about code.
pub struct CodeSomaticEngine {
    /// Maximum cyclomatic complexity (for normalization)
    pub max_complexity: f64,
    /// Maximum reverse dependency count (for normalization)
    pub max_rev_deps: f64,
    /// Maximum transitive dependency count (for normalization)
    pub max_trans_deps: f64,
    /// Maximum scope (files × lines) for normalization
    pub max_scope: f64,
    /// Query radius in strategy space
    pub query_radius: f64,
    /// Maximum markers to consider
    pub query_k: usize,
}

impl CodeSomaticEngine {
    pub fn new() -> Self {
        Self {
            max_complexity: 50.0,
            max_rev_deps: 50.0,
            max_trans_deps: 100.0,
            max_scope: 1000.0,
            query_radius: 0.5,
            query_k: 10,
        }
    }

    /// Compute code region metrics from the symbol graph.
    pub fn compute_metrics(
        &self,
        symbol: &SymbolId,
        graph: &SymbolGraph,
        pagerank: &HashMap<SymbolId, f64>,
        fingerprints: &HashMap<SymbolId, HdcFingerprint>,
        pad: &PadVector,
    ) -> CodeRegionMetrics {
        let rev_deps = graph.reverse_neighbors(symbol).len();
        let trans_deps = graph.transitive(symbol, 10).len();
        let pr = pagerank.get(symbol).copied().unwrap_or(0.0);

        // Novelty: 1 - max similarity to any known fingerprint
        let novelty = if let Some(fp) = fingerprints.get(symbol) {
            let max_sim = fingerprints.values()
                .filter(|other| !std::ptr::eq(*other, fp))
                .map(|other| fp.similarity(other))
                .fold(0.0_f64, f64::max);
            1.0 - max_sim
        } else {
            1.0 // Unknown symbol = maximum novelty
        };

        CodeRegionMetrics {
            complexity: pr.min(1.0), // PageRank as complexity proxy
            risk: (rev_deps as f64 / self.max_rev_deps).min(1.0),
            novelty,
            confidence: (pad.dominance + 1.0) / 2.0,
            time_pressure: 0.0, // Filled by caller
            scope: 0.0,         // Filled by caller
            reversibility: 0.8, // Default: most changes are additive
            dependency_depth: (trans_deps as f64 / self.max_trans_deps).min(1.0),
        }
    }

    /// Query somatic landscape for bias based on code metrics.
    pub fn query_bias(
        &self,
        metrics: &CodeRegionMetrics,
        landscape: &SomaticLandscape,
    ) -> CodeSomaticBias {
        let start = std::time::Instant::now();
        let coords = metrics.to_coords();

        let markers = landscape.nearest_within(
            &coords,
            self.query_radius,
            self.query_k,
        );

        if markers.is_empty() {
            return CodeSomaticBias {
                strategy: DispatchStrategy::Balanced,
                tier: InferenceTier::T1,
                warnings: vec![
                    "No prior experience with similar code metrics".into()
                ],
                avg_valence: 0.0,
                marker_count: 0,
                query_time_us: start.elapsed().as_micros() as u64,
            };
        }

        let total_intensity: f64 = markers.iter().map(|m| m.intensity).sum();
        let avg_valence: f64 = markers.iter()
            .map(|m| m.valence * m.intensity)
            .sum::<f64>() / total_intensity;

        let mut warnings = Vec::new();
        let (strategy, tier) = if avg_valence < -0.5 {
            warnings.push(format!(
                "Somatic warning: {} similar regions averaged {:.2} valence",
                markers.len(), avg_valence
            ));
            (DispatchStrategy::Conservative, InferenceTier::T2)
        } else if avg_valence < -0.2 {
            warnings.push("Somatic caution: mixed outcomes in similar regions".into());
            (DispatchStrategy::Balanced, InferenceTier::T1)
        } else if avg_valence > 0.5 {
            (DispatchStrategy::Exploratory, InferenceTier::T0)
        } else {
            (DispatchStrategy::Balanced, InferenceTier::T1)
        };

        // Additional code-specific warnings
        if metrics.risk > 0.7 {
            warnings.push("High risk: many reverse dependencies with low coverage".into());
        }
        if metrics.novelty > 0.8 {
            warnings.push("Novel code pattern: no similar patterns in knowledge base".into());
        }
        if metrics.dependency_depth > 0.5 {
            warnings.push("Deep dependency tree: changes may cascade widely".into());
        }

        CodeSomaticBias {
            strategy,
            tier,
            warnings,
            avg_valence,
            marker_count: markers.len(),
            query_time_us: start.elapsed().as_micros() as u64,
        }
    }
}
```

### Integration Plan

| Step | What | Where | Connects To |
|------|------|-------|-------------|
| 1 | Add `CodeSomaticEngine` to `roko-daimon` | `roko-daimon/src/code_somatic.rs` | Existing somatic landscape |
| 2 | Wire roko-index metrics into engine | `roko-index` → `roko-daimon` | SymbolGraph, PageRank, HDC |
| 3 | Query bias before task execution | `roko-cli/src/orchestrate.rs` | Before agent dispatch |
| 4 | Create markers after gate verdict | `roko-cli/src/orchestrate.rs` | Significant outcomes → k-d tree |
| 5 | Dream consolidation merges markers | `roko-dreams` (Integration phase) | Nearby markers merged |
| 6 | TUI visualization of somatic heatmap | `roko-cli/src/tui/` | Per-file valence overlay |
| 7 | Per-crate somatic profiles | `.roko/learn/somatic-code.json` | Aggregate markers by crate |

### Test Criteria

- [ ] Code region with high complexity + low coverage triggers Conservative strategy
- [ ] Code region with prior successful markers triggers Exploratory strategy
- [ ] Somatic query < 100μs for landscape with 10,000 markers
- [ ] Marker creation from gate failure produces negative valence marker
- [ ] Novel code (no HDC similarity) produces "no prior experience" warning
- [ ] PageRank-weighted risk increases tier bias (high-importance symbols get more scrutiny)

---

## 8. Token Economy + Dream Quality

**Pay for Dreams with Tokens, Better Dreams Earn More**

### Motivation

Roko's dream cycle consumes real resources: NREM replay uses Haiku (~$0.001/episode), REM
imagination uses Sonnet (~$0.01/counterfactual). The learning system has budget guardrails
(per-task, per-session, per-day). But dream spending is unregulated—there's no feedback loop
between dream *quality* and dream *budget*.

Meanwhile, the token economy concept exists in the architecture docs but isn't wired: agents
earn tokens for completed tasks and spend tokens on compute. The missing link: **dreams
should be a market good**. High-quality dreams (verified hypotheses that promote to Consolidated
knowledge) should earn the agent more dream budget. Low-quality dreams (unverifiable, redundant,
or violated hypotheses) should cost budget without return.

This creates a self-regulating system: agents that dream well earn the right to dream more.
Agents that dream poorly are forced to rely on waking experience until their dream quality
improves.

### Research Basis

- **Mantiuk, Becker & Wu (2025)** "From Curiosity to Competence: How World Models Interact
  with the Dynamics of Exploration," arXiv:2507.08210. Compares curiosity vs. competence as
  intrinsic rewards, finding a two-way interaction: world model accuracy determines intrinsic
  motivation value. Better dreams yield better rewards, creating a virtuous cycle.

- **"INTUITOR" (2025)** "Learning to Reason without External Rewards," arXiv:2505.19590.
  Uses model self-certainty as intrinsic reward for RL without external supervision. The
  "token" is epistemic quality—high-confidence outputs are rewarded, creating an internal
  economy where currency is certainty.

- **Burda et al. / DreamerV3-XP (2025)** "Optimizing Exploration Through Uncertainty
  Estimation," arXiv:2510.21418. Extends Dreamer with ensemble disagreement as intrinsic
  reward. Dream quality (ensemble agreement) is the token in an internal economy balancing
  exploration and exploitation.

- **Lin et al. (2025)** "Scaling LLM Test-Time Compute Optimally Can be More Effective than
  Scaling Model Parameters" (referenced in Dreams docs). Demonstrates 5× reduction in
  test-time compute via offline training—but only if dream quality is high.

### Core Idea

Introduce a **DreamBudget** that starts at a base allocation and grows or shrinks based on
dream outcomes. Each dream cycle has a cost (model tokens × price). Each promoted hypothesis
earns tokens proportional to its eventual impact (knowledge tier achieved × validation count).
The ratio of earned-to-spent is the dream's **Return on Imagination (ROI)**—and future dream
budgets are scaled by this ROI.

```
Dream Economy:

  DreamBudget(t+1) = DreamBudget(t) × (1 + ROI(t) - depreciation)

  ROI(t) = earned(t) / spent(t)

  earned(t) = Σ (tier_value × validation_count)
              for each hypothesis promoted from dream cycle t

  spent(t) = Σ (tokens × price_per_token)
             for all model calls in dream cycle t

  tier_value = {Transient: 0.01, Working: 0.10, Consolidated: 1.00, Persistent: 10.00}

  depreciation = 0.05 per cycle (prevents unbounded growth)

  Constraints:
    DreamBudget ∈ [min_budget, max_budget]     // Floor and ceiling
    min_budget = $0.01 (always allow micro-consolidation)
    max_budget = $1.00 (prevent runaway dream spending)
```

### Algorithm: Dream Token Economy

```
Algorithm: DreamTokenEconomy

Input:
  budget: DreamBudget                    — current dream allocation
  dream_history: Vec<DreamCycleReport>   — past dream cycle outcomes
  knowledge_promotions: Vec<Promotion>   — knowledge entries promoted from dreams

Parameters:
  min_budget: f64 = 0.01                 — floor ($0.01, always allow micro-dreams)
  max_budget: f64 = 1.00                 — ceiling ($1.00, prevent runaway)
  depreciation: f64 = 0.05              — per-cycle decay (5%)
  tier_values = {Transient: 0.01, Working: 0.10, Consolidated: 1.00, Persistent: 10.00}
  quality_bonus_threshold: f64 = 0.7     — verification pass rate for bonus
  novelty_multiplier: f64 = 2.0          — bonus for novel (HDC-dissimilar) insights

Output:
  next_budget: f64                       — updated dream budget
  allocation: DreamAllocation            — how to split budget across phases
  quality_grade: char                    — A-D grade for dream quality

Steps:
  1. COMPUTE DREAM COST (spent):
     spent = Σ report.model_cost_usd for report in dream_history.last_cycle()
     // Typical: NREM ~$0.01, REM ~$0.05, Integration ~$0.00

  2. COMPUTE DREAM REVENUE (earned):
     earned = 0.0
     For each promotion in knowledge_promotions from last cycle:
       base_value = tier_values[promotion.tier]
       validation_bonus = promotion.validation_count × 0.01
       novelty_bonus = if promotion.is_novel { novelty_multiplier } else { 1.0 }
       earned += base_value × novelty_bonus + validation_bonus

  3. COMPUTE ROI:
     roi = if spent > 0.0 { earned / spent } else { 0.0 }
     // roi > 1.0 → dreams are profitable
     // roi < 1.0 → dreams cost more than they produce

  4. COMPUTE DREAM QUALITY METRICS:
     verification_rate = verified_scenarios / total_scenarios
     promotion_rate = promoted_hypotheses / staged_hypotheses
     novelty_rate = novel_insights / total_insights
     depotentiation_effect = arousal_reduction / pre_dream_arousal

     quality_score = 0.30 × verification_rate
                   + 0.30 × promotion_rate
                   + 0.20 × novelty_rate
                   + 0.20 × depotentiation_effect

     quality_grade = if quality_score >= 0.75 { 'A' }
                     elif quality_score >= 0.50 { 'B' }
                     elif quality_score >= 0.25 { 'C' }
                     else { 'D' }

  5. UPDATE BUDGET:
     growth_factor = 1.0 + roi - depreciation
     // Quality bonus: high-quality dreamers get extra budget
     if quality_score > quality_bonus_threshold:
       growth_factor += 0.10  // 10% quality bonus
     next_budget = budget.current × growth_factor
     next_budget = next_budget.clamp(min_budget, max_budget)

  6. ALLOCATE ACROSS PHASES:
     // Higher quality → more REM budget (creative phase)
     // Lower quality → more NREM budget (consolidation phase)
     if quality_grade in {'A', 'B'}:
       allocation = { nrem: 0.30, rem: 0.60, integration: 0.10 }
     elif quality_grade == 'C':
       allocation = { nrem: 0.50, rem: 0.40, integration: 0.10 }
     else:  // D grade
       allocation = { nrem: 0.70, rem: 0.20, integration: 0.10 }
     // D-grade agents spend most budget on consolidation (safer)

  7. EMIT METRICS:
     log DreamEconomyEvent {
       cycle: current_cycle,
       spent, earned, roi,
       quality_score, quality_grade,
       budget: next_budget,
       allocation,
     }
```

### Rust Sketch

```rust
/// Dream budget with quality-driven growth dynamics.
///
/// Higher-quality dreams earn more budget. Lower-quality dreams shrink budget.
/// The economy self-regulates: agents that dream well dream more.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamBudget {
    /// Current budget in USD
    pub current_usd: f64,
    /// Minimum budget floor (always allow micro-dreams)
    pub min_usd: f64,
    /// Maximum budget ceiling (prevent runaway)
    pub max_usd: f64,
    /// Depreciation rate per cycle
    pub depreciation: f64,
    /// Historical ROI values (ring buffer)
    pub roi_history: VecDeque<f64>,
    /// Historical quality grades
    pub quality_history: VecDeque<char>,
    /// Cumulative spent
    pub total_spent: f64,
    /// Cumulative earned
    pub total_earned: f64,
    /// Number of dream cycles
    pub cycle_count: u64,
}

/// Value assigned to each knowledge tier for dream revenue calculation.
#[derive(Debug, Clone)]
pub struct TierValues {
    pub transient: f64,     // 0.01
    pub working: f64,       // 0.10
    pub consolidated: f64,  // 1.00
    pub persistent: f64,    // 10.00
}

impl Default for TierValues {
    fn default() -> Self {
        Self {
            transient: 0.01,
            working: 0.10,
            consolidated: 1.00,
            persistent: 10.00,
        }
    }
}

/// How to allocate budget across dream phases.
#[derive(Debug, Clone)]
pub struct DreamAllocation {
    /// Fraction for NREM replay (consolidation)
    pub nrem: f64,
    /// Fraction for REM imagination (creativity)
    pub rem: f64,
    /// Fraction for integration (validation)
    pub integration: f64,
}

/// Dream quality metrics for a single cycle.
#[derive(Debug, Clone)]
pub struct DreamQualityMetrics {
    pub verification_rate: f64,
    pub promotion_rate: f64,
    pub novelty_rate: f64,
    pub depotentiation_effect: f64,
    pub quality_score: f64,
    pub quality_grade: char,
}

/// Economy event logged per dream cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamEconomyEvent {
    pub cycle: u64,
    pub spent_usd: f64,
    pub earned_usd: f64,
    pub roi: f64,
    pub quality_score: f64,
    pub quality_grade: char,
    pub budget_after: f64,
    pub allocation: DreamAllocation,
    pub timestamp_ms: i64,
}

impl DreamBudget {
    pub fn new(initial_usd: f64) -> Self {
        Self {
            current_usd: initial_usd,
            min_usd: 0.01,
            max_usd: 1.00,
            depreciation: 0.05,
            roi_history: VecDeque::with_capacity(100),
            quality_history: VecDeque::with_capacity(100),
            total_spent: 0.0,
            total_earned: 0.0,
            cycle_count: 0,
        }
    }

    /// Update budget after a dream cycle completes.
    ///
    /// Computes ROI from dream revenue (promoted knowledge) vs. cost
    /// (model tokens). Quality bonus for high-performing dreamers.
    pub fn update(
        &mut self,
        report: &DreamCycleReport,
        promotions: &[KnowledgePromotion],
        tier_values: &TierValues,
    ) -> DreamEconomyEvent {
        // 1. Compute cost
        let spent = report.total_cost_usd();
        self.total_spent += spent;

        // 2. Compute revenue from promoted knowledge
        let earned: f64 = promotions.iter().map(|p| {
            let base = match p.tier {
                Tier::Transient => tier_values.transient,
                Tier::Working => tier_values.working,
                Tier::Consolidated => tier_values.consolidated,
                Tier::Persistent => tier_values.persistent,
            };
            let novelty = if p.is_novel { 2.0 } else { 1.0 };
            let validation = p.validation_count as f64 * 0.01;
            base * novelty + validation
        }).sum();
        self.total_earned += earned;

        // 3. ROI
        let roi = if spent > 0.0 { earned / spent } else { 0.0 };
        self.roi_history.push_back(roi);
        if self.roi_history.len() > 100 {
            self.roi_history.pop_front();
        }

        // 4. Quality metrics
        let quality = self.compute_quality(report);
        self.quality_history.push_back(quality.quality_grade);
        if self.quality_history.len() > 100 {
            self.quality_history.pop_front();
        }

        // 5. Update budget
        let mut growth = 1.0 + roi - self.depreciation;
        if quality.quality_score > 0.7 {
            growth += 0.10; // Quality bonus
        }
        self.current_usd = (self.current_usd * growth)
            .clamp(self.min_usd, self.max_usd);

        // 6. Phase allocation based on quality
        let allocation = self.compute_allocation(quality.quality_grade);

        self.cycle_count += 1;

        DreamEconomyEvent {
            cycle: self.cycle_count,
            spent_usd: spent,
            earned_usd: earned,
            roi,
            quality_score: quality.quality_score,
            quality_grade: quality.quality_grade,
            budget_after: self.current_usd,
            allocation,
            timestamp_ms: now_ms(),
        }
    }

    /// Compute dream quality from cycle report.
    fn compute_quality(&self, report: &DreamCycleReport) -> DreamQualityMetrics {
        let verification_rate = if report.counterfactuals_generated > 0 {
            report.verified_scenarios as f64 / report.counterfactuals_generated as f64
        } else { 0.0 };

        let promotion_rate = if report.staged_hypotheses > 0 {
            report.promoted_hypotheses as f64 / report.staged_hypotheses as f64
        } else { 0.0 };

        let novelty_rate = if report.insights.len() > 0 {
            report.insights.iter().filter(|i| i.is_novel).count() as f64
                / report.insights.len() as f64
        } else { 0.0 };

        let depotentiation_effect = report.depotentiation.arousal_reduction
            / report.depotentiation.pre_arousal.max(0.01);

        let quality_score = 0.30 * verification_rate
            + 0.30 * promotion_rate
            + 0.20 * novelty_rate
            + 0.20 * depotentiation_effect;

        let quality_grade = if quality_score >= 0.75 { 'A' }
            else if quality_score >= 0.50 { 'B' }
            else if quality_score >= 0.25 { 'C' }
            else { 'D' };

        DreamQualityMetrics {
            verification_rate,
            promotion_rate,
            novelty_rate,
            depotentiation_effect,
            quality_score,
            quality_grade,
        }
    }

    /// Allocate budget across phases based on quality grade.
    ///
    /// High-quality dreamers get more REM budget (creative).
    /// Low-quality dreamers get more NREM budget (consolidation).
    fn compute_allocation(&self, grade: char) -> DreamAllocation {
        match grade {
            'A' | 'B' => DreamAllocation { nrem: 0.30, rem: 0.60, integration: 0.10 },
            'C'       => DreamAllocation { nrem: 0.50, rem: 0.40, integration: 0.10 },
            _         => DreamAllocation { nrem: 0.70, rem: 0.20, integration: 0.10 },
        }
    }

    /// Can the agent afford a dream cycle at the given estimated cost?
    pub fn can_afford(&self, estimated_cost: f64) -> bool {
        estimated_cost <= self.current_usd
    }

    /// Lifetime ROI across all dream cycles.
    pub fn lifetime_roi(&self) -> f64 {
        if self.total_spent > 0.0 {
            self.total_earned / self.total_spent
        } else {
            0.0
        }
    }

    /// Average quality grade over recent cycles.
    pub fn recent_quality(&self, window: usize) -> f64 {
        let grades: Vec<f64> = self.quality_history.iter()
            .rev()
            .take(window)
            .map(|&g| match g {
                'A' => 4.0,
                'B' => 3.0,
                'C' => 2.0,
                'D' => 1.0,
                _ => 0.0,
            })
            .collect();

        if grades.is_empty() { 0.0 }
        else { grades.iter().sum::<f64>() / grades.len() as f64 }
    }
}
```

### Integration Plan

| Step | What | Where | Connects To |
|------|------|-------|-------------|
| 1 | Add `DreamBudget` to `roko-learn` | `roko-learn/src/dream_economy.rs` | Adjacent to cost normalization |
| 2 | Initialize budget from config | `roko.toml` `[dreams.budget]` | `initial_usd`, `min`, `max` |
| 3 | Check `can_afford()` before dream cycle | `roko-dreams/src/scheduler.rs` | Gate dream trigger |
| 4 | Track promotions from dream hypotheses | `roko-neuro` promotion events | Link promoted knowledge to source cycle |
| 5 | Update budget after cycle completion | `roko-dreams/src/runner.rs` | Call `budget.update()` |
| 6 | Log economy events | `.roko/learn/dream-economy.jsonl` | For dashboard and analysis |
| 7 | Wire allocation into phase budgets | `roko-dreams/src/runner.rs` | NREM/REM/Integration token limits |

### Test Criteria

- [ ] Budget increases when dream produces Consolidated knowledge (ROI > 1.0)
- [ ] Budget decreases when dream produces no promotions (ROI = 0, depreciation applies)
- [ ] Budget never drops below min_usd ($0.01)
- [ ] Budget never exceeds max_usd ($1.00)
- [ ] Quality grade 'A' allocates 60% to REM; grade 'D' allocates 70% to NREM
- [ ] `can_afford()` correctly prevents expensive dream cycles on depleted budget
- [ ] Lifetime ROI > 1.0 indicates dreams are net positive for the agent
- [ ] Phase allocation sums to 1.0 for all quality grades

---

## Cross-Innovation Interactions

These eight innovations are not isolated. They form a **reinforcing network**:

That network is also the chapter's moat claim: each innovation becomes materially more
defensible because it depends on aligned decisions across the kernel stack. Substrate,
Bus, HDC fingerprints, demurrage, heuristic calibration, c-factor measurement, plugin SPI,
and the replication ledger reinforce one another. Competitors can approximate the isolated
primitive, but they do not get the same architecture-wide compound effect without rebuilding
the weave. See also [tmp/refinements/18-competitive-moat.md](../../tmp/refinements/18-competitive-moat.md).

```
   ┌─────────────────────────────────────────────────────────┐
   │                                                         │
   │  [1] HDC Beliefs ←──────→ [6] Witness World Model      │
   │       │                          │                      │
   │       │ beliefs encode           │ DAG validates        │
   │       │ world model              │ beliefs              │
   │       ↓                          ↓                      │
   │  [2] Affect Causal ←────→ [7] Code Somatic             │
   │       │                          │                      │
   │       │ causal structure         │ code metrics →       │
   │       │ of emotions              │ somatic markers      │
   │       ↓                          ↓                      │
   │  [3] Dream Verify ←─────→ [8] Dream Economy            │
   │       │                          │                      │
   │       │ verification             │ quality grades       │
   │       │ conditions               │ budget allocation    │
   │       ↓                          ↓                      │
   │  [4] Knowledge Morph ←───→ [5] Stigmergic Bandits      │
   │       │                          │                      │
   │       │ knowledge drives         │ pheromones encode    │
   │       │ specialization           │ collective rewards   │
   │       └──────────────────────────┘                      │
   │                                                         │
   └─────────────────────────────────────────────────────────┘
```

**Key feedback loops:**

1. **HDC Beliefs [1] ↔ Witness World Model [6]**: HDC vectors provide a compact belief state;
   the Witness DAG provides the causal structure that validates and refines those beliefs.
   Use HDC for fast (~8μs) per-tick inference, Witness model for deeper (~50ms) reflection.

2. **Affect Causal [2] ↔ Code Somatic [7]**: Causal discovery learns which PAD states cause
   failures; code somatic markers encode these discoveries as fast-path heuristics. Causal
   model provides the *why*; somatic markers provide the *speed*.

3. **Dream Verify [3] ↔ Dream Economy [8]**: Verification conditions determine dream quality;
   dream quality determines budget allocation. Verified dreams earn more budget for future
   REM imagination. This prevents low-quality dreaming from consuming resources.

4. **Knowledge Morphogenesis [4] ↔ Stigmergic Bandits [5]**: Knowledge concentration drives
   agent specialization via reaction-diffusion; pheromone trails encode the collective's
   exploration history as bandit arms. Specialized agents deposit stronger pheromones in
   their niche, reinforcing the specialization gradient.

The full interaction set is what makes the moat durable: the primitives are valuable alone,
but the cross-pollinated architecture compounds because each subsystem improves the others'
signals, routing, persistence, and calibration. That architectural coherence is expensive to
copy because it depends on aligned kernel choices, not just feature parity.

---

## Implementation Priority

| Priority | Innovation | Dependencies | Est. Effort |
|----------|-----------|-------------|-------------|
| P0 | [1] HDC + Active Inference | `bardo-primitives` HDC ops | Medium — replace scalar PE with HDC PE |
| P0 | [7] Code Somatic Markers | `roko-index`, `roko-daimon` | Medium — wire existing subsystems |
| P1 | [5] Stigmergic Bandits | `roko-learn` bandits, `roko-conductor` pheromones | Medium — compose existing |
| P1 | [8] Dream Economy | `roko-learn` costs, `roko-dreams` | Small — accounting + gating |
| P2 | [4] Knowledge Morphogenesis | `roko-neuro`, `roko-conductor` | Medium — replace activation signal |
| P2 | [6] Witness World Model | `roko-core` Witness DAG | Large — causal extraction engine |
| P3 | [2] Affect Causal Discovery | `roko-daimon`, `roko-learn` | Large — PC algorithm + do-calculus |
| P3 | [3] Dream Verification | `roko-gate`, `roko-dreams` | Large — invariant specification + checking |
