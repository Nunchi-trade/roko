# Research-to-Runtime Bridge

> For each major subsystem, shows the academic foundation, the simplified runtime approximation, what fidelity is lost, and what a higher-fidelity implementation would look like.

**Depth for**: [28-ROADMAP.md](../../unified/28-ROADMAP.md)
**Sources**: `docs/21-references/25-research-to-runtime.md`, all reference files, existing depth docs
**Prerequisites**: [00-INDEX.md](../../unified/00-INDEX.md) (vocabulary), [07-LEARNING.md](../../unified/07-LEARNING.md) (learning loops)

---

## The Problem

Roko's architecture draws from dozens of academic fields: active inference, complementary learning systems, evolutionary dynamics, somatic markers, topological data analysis, conformal prediction, and more. Each field produces theories formulated for different assumptions than a production agent system. The gap between "what the paper says" and "what the runtime does" must be explicit and auditable.

This document maps that gap for each major subsystem: what does the theory promise, what does the runtime approximate, where does fidelity degrade, and what would it cost to close the gap.

---

## The Research-to-Runtime Pipeline

The pipeline itself is a Loop (feedback Graph that improves over time):

```
Paper --> Hypothesis Signal --> Implementation Cell --> Evaluation Verify --> Calibration Loop --> Knowledge Consolidation
  ^                                                                                                         |
  |_________________________________________________________________________________________________________|
                                            (falsification or confirmation feeds back)
```

### As a TOML Graph

```toml
[graph]
id = "research-to-runtime"
pattern = "Loop"

[[graph.cells]]
id = "ingest"
protocol = "Store"
description = "Paper enters as Signal (Kind::Paper)"

[[graph.cells]]
id = "hypothesize"
protocol = "Score"
description = "Extract testable Claim with explicit falsifier"

[[graph.cells]]
id = "implement"
protocol = "Compose"
description = "Lift Claim to runtime Heuristic in Cell or config"

[[graph.cells]]
id = "evaluate"
protocol = "Verify"
description = "Test Heuristic against episodes with distribution-free bounds"

[[graph.cells]]
id = "calibrate"
protocol = "React"
description = "Update ReplicationLedger, adjust confidence, retire or promote"

[[graph.edges]]
from = "ingest"
to = "hypothesize"

[[graph.edges]]
from = "hypothesize"
to = "implement"

[[graph.edges]]
from = "implement"
to = "evaluate"

[[graph.edges]]
from = "evaluate"
to = "calibrate"

[[graph.edges]]
from = "calibrate"
to = "ingest"
feedback = true
description = "Falsification creates new Paper or updates existing"
```

### Data Shapes

```rust
/// A paper in the system. Target-state Signal with Kind::Paper.
pub struct Paper {
    pub id: Uuid,
    pub doi: Option<String>,
    pub title: String,
    pub authors: Vec<String>,
    pub year: u16,
    pub fingerprint: HdcVector,  // 10,240-bit for similarity search
    pub claims: Vec<ClaimId>,
    pub protocol_grounded: Vec<Protocol>,  // which protocols this paper grounds
}

/// Smallest testable restatement of a paper result.
pub struct Claim {
    pub id: Uuid,
    pub paper: PaperId,
    pub hypothesis: String,        // structured, testable
    pub falsifier: Predicate,      // when this fires, claim is challenged
    pub context: Vec<Predicate>,   // conditions under which claim applies
    pub calibration: Calibration,  // running record of performance
}

/// Tracks whether a claim replicates in Roko's deployment.
pub struct ReplicationLedger {
    pub claim: ClaimId,
    pub paper_effect: f64,         // what the paper reported
    pub our_effect: f64,           // what we observe
    pub our_n: u32,                // sample size
    pub divergence_ci: (f64, f64), // confidence interval on divergence
    pub status: ReplicationStatus, // Untested | Insufficient | Replicates | Partial | Fails | ContextDependent
}
```

---

## Bridge 1: Active Inference --> EFE Approximation via 4 Signals

### Academic Foundation

**Friston (2006, 2010, 2015)**: The Free Energy Principle states that all self-organizing systems minimize variational free energy (the gap between internal model and sensory evidence). Expected Free Energy (EFE) decomposes into:

```
G(pi) = -E_Q[ln P(o|s)] - E_Q[D_KL(Q(s|o) || Q(s))]
         ^pragmatic          ^epistemic
```

Full active inference requires: a generative world model P(o, s), beliefs Q(s), policy evaluation over all candidate policies, and Bayesian model inversion.

### Runtime Approximation

Roko approximates EFE with 4 scalar signals combined linearly:

```rust
/// Simplified EFE for Route Cell tier selection
fn efe_approximation(signals: &RouteSignals) -> f64 {
    let pragmatic = signals.goal_alignment;          // task relevance [0,1]
    let epistemic = signals.prediction_error;        // surprise magnitude [0,1]
    let cost      = signals.tier_cost;               // normalized cost [0,1]
    let affect    = signals.daimon_arousal_bias;     // [-0.2, 0.2] modulation

    // Route to higher tier when:
    // - high prediction error (epistemic: need more information)
    // - high goal alignment (pragmatic: this matters)
    // - acceptable cost
    let efe = (0.4 * pragmatic + 0.4 * epistemic - 0.2 * cost) + affect;
    efe
}

// T0: efe < 0.3 (prediction is confident, no LLM needed)
// T1: 0.3 <= efe < 0.7 (moderate uncertainty, fast model)
// T2: efe >= 0.7 (high uncertainty or high stakes, full model)
```

### Fidelity Lost

| Full Theory | Runtime | What's Missing |
|-------------|---------|----------------|
| Generative world model P(o,s) | Scalar prediction_error from CalibrationTracker | No structured world model; no counterfactual queries |
| Policy evaluation over all futures | Linear combination of 4 signals | No planning horizon; no tree search over policies |
| Bayesian model inversion | Fixed weights (0.4, 0.4, 0.2) | No posterior updates on the weighting scheme |
| Continuous belief updating | Discrete per-tick evaluation | Belief is snapshot, not continuous process |
| Precision weighting (attention) | Static weights + Daimon affect bias | No learned precision; affect provides weak proxy |

### Higher-Fidelity Path

**Cost**: Requires building a discrete-state generative model (POMDP) of the agent's task domain.

```rust
/// Target-state: Active inference with explicit POMDP
struct ActiveInferenceRouter {
    model: DiscretePomdp,          // P(o|s), P(s'|s,a)
    beliefs: BeliefDistribution,   // Q(s) updated per observation
    policies: Vec<Policy>,         // candidate action sequences
    precision: PrecisionMatrix,    // learned attention weights
}

impl Route for ActiveInferenceRouter {
    fn select(&mut self, observation: &Signal) -> CellId {
        self.beliefs.update(observation, &self.model);
        let efe_per_policy = self.policies.iter()
            .map(|pi| self.compute_efe(pi, &self.beliefs))
            .collect();
        let softmax = boltzmann(efe_per_policy, self.precision);
        sample(softmax)
    }
}
```

**Reference**: Heins et al. (2022) pymdp provides JAX-accelerated discrete POMDP active inference. Shafiei et al. (2025) DR-FREE extends to distributionally robust settings.

**Graduation criterion**: When the 4-signal linear approximation produces routing decisions that disagree with a full POMDP solver > 20% of the time on a representative task set, upgrade.

---

## Bridge 2: Replicator Dynamics --> Demurrage + Retrieve-to-Reinforce

### Academic Foundation

**Taylor & Jonker (1978), Fisher (1930)**: The replicator equation describes how strategy frequencies change in a population:

```
dx_i/dt = x_i * (f_i - f_bar)
```

where x_i is the frequency of strategy i, f_i is its fitness, and f_bar is the population mean fitness. Strategies above mean fitness grow; below mean fitness shrink. Fisher's Fundamental Theorem: the rate of fitness increase equals the genetic variance in fitness.

### Runtime Approximation

Roko implements replicator dynamics through demurrage (decay) + retrieval reinforcement:

```rust
/// Demurrage as simplified replicator dynamics
fn update_knowledge_balance(entry: &mut KnowledgeEntry, tick: u64) {
    // Decay: all entries lose balance (analogous to replicator culling below-mean)
    let elapsed = tick - entry.last_updated;
    let decay = (-0.693 * elapsed as f64 / entry.half_life as f64).exp();
    entry.balance *= decay;

    // Retrieval reinforcement: used entries regain balance (above-mean fitness)
    if entry.retrieved_this_tick {
        entry.balance = (entry.balance + 0.1).min(1.0);
        entry.retrieve_count += 1;
    }

    // Pruning threshold: entries below 0.05 balance are eligible for GC
    // This is the "death" equivalent in replicator dynamics
}
```

### Fidelity Lost

| Full Theory | Runtime | What's Missing |
|-------------|---------|----------------|
| Continuous frequency dynamics | Discrete per-tick decay | No continuous ODE; step approximation |
| Population-relative fitness (f_i - f_bar) | Fixed decay rate per type | No dynamic adjustment based on peer performance |
| Mutation operator | Random HDC recombination in Dreams | Limited to offline REM phase; no continuous mutation |
| Selection pressure adapts to population state | Fixed half-life per knowledge type | No adaptive half-life based on Store population statistics |
| Frequency-dependent selection | Independent entries | No interaction effects between knowledge entries |

### Higher-Fidelity Path

```rust
/// Target-state: Population-relative demurrage
fn adaptive_demurrage(entry: &KnowledgeEntry, store: &NeuroStore) -> f64 {
    let population_mean_fitness = store.mean_retrieval_rate();
    let entry_fitness = entry.retrieval_rate();

    // Replicator: grow if above mean, shrink if below
    let relative_fitness = entry_fitness - population_mean_fitness;

    // Translate to decay modulation: positive = slower decay, negative = faster decay
    let decay_modulation = 1.0 + (relative_fitness * 0.5).clamp(-0.5, 0.5);
    entry.base_half_life as f64 * decay_modulation
}
```

**Graduation criterion**: When store population exceeds 10,000 entries and the fixed-rate demurrage demonstrably over-prunes high-value low-frequency entries (measured by retrieval regret: wanting an entry that was already pruned).

---

## Bridge 3: Turing Patterns --> Morphogenetic Strategy Vectors

### Academic Foundation

**Turing (1952), Gierer & Meinhardt (1972)**: Reaction-diffusion systems generate spatial patterns from homogeneous initial conditions via local activation and lateral inhibition:

```
du/dt = f(u,v) + D_u * nabla^2(u)   (activator)
dv/dt = g(u,v) + D_v * nabla^2(v)   (inhibitor, D_v >> D_u)
```

The key insight: short-range activation + long-range inhibition = spontaneous pattern formation (stripes, spots, spatial domains).

### Runtime Approximation

Roko approximates Turing patterns through pheromone-based agent specialization:

```rust
/// Pheromone field as simplified reaction-diffusion
struct PheromoneField {
    deposits: HashMap<AgentId, Vec<PheromoneDeposit>>,
}

struct PheromoneDeposit {
    kind: PheromoneKind,   // Threat, Opportunity, Wisdom
    strength: f64,         // decays over time (inhibition diffusion)
    position: HdcVector,   // location in strategy space
}

impl PheromoneField {
    fn deposit(&mut self, agent: AgentId, kind: PheromoneKind, position: HdcVector) {
        // Short-range activation: agents near this position see the signal
        self.deposits.entry(agent).or_default().push(PheromoneDeposit {
            kind,
            strength: 1.0,
            position,
        });
    }

    fn tick(&mut self) {
        // Long-range inhibition: all deposits decay (Ebbinghaus)
        for deposits in self.deposits.values_mut() {
            deposits.retain_mut(|d| {
                d.strength *= 0.95; // 5% decay per tick
                d.strength > 0.05   // pruning threshold
            });
        }
    }

    fn query_specialization_pressure(&self, position: &HdcVector) -> f64 {
        // If many deposits exist near this position, pressure to MOVE AWAY
        // (lateral inhibition -> agents differentiate)
        let nearby = self.deposits.values().flatten()
            .filter(|d| cosine_similarity(&d.position, position) > 0.7)
            .map(|d| d.strength)
            .sum::<f64>();
        nearby // high value = high pressure to specialize elsewhere
    }
}
```

### Fidelity Lost

| Full Theory | Runtime | What's Missing |
|-------------|---------|----------------|
| Continuous PDE over 2D space | Discrete HDC similarity queries | No continuous spatial domain; discrete cosine similarity |
| Diffusion constants (D_u, D_v) as parameters | Fixed decay rate (0.95) | No tunable diffusion ratio |
| Bifurcation analysis (pattern type depends on parameters) | Emergent only | No theoretical prediction of what patterns will form |
| Multi-scale patterns (spots within stripes) | Single-scale pheromone field | No hierarchical pattern formation |

### Higher-Fidelity Path

Use the PDE framework from "Stigmergy: From Mathematical Modelling to Control" (2024, _Proc. Royal Soc. A_) which treats the agent swarm as a fluid with trace density:

```rust
/// Target-state: Continuum-level Turing patterns
struct ContinuumPheromoneField {
    // Discretized PDE over HDC space
    activator: ScalarField,   // local reinforcement
    inhibitor: ScalarField,   // global suppression
    d_activator: f64,         // short-range diffusion
    d_inhibitor: f64,         // long-range diffusion (d_inhibitor >> d_activator)
}

impl ContinuumPheromoneField {
    fn step(&mut self, dt: f64) {
        // Laplacian diffusion + reaction terms
        let lap_u = self.activator.laplacian();
        let lap_v = self.inhibitor.laplacian();
        self.activator += dt * (reaction_u(&self.activator, &self.inhibitor) + self.d_activator * lap_u);
        self.inhibitor += dt * (reaction_v(&self.activator, &self.inhibitor) + self.d_inhibitor * lap_v);
    }
}
```

**Graduation criterion**: When group size exceeds 10 agents and naive pheromone field produces homogeneous (non-differentiated) behavior, upgrade to continuum model.

---

## Bridge 4: Somatic Markers --> PAD x Decision HDC Binding

### Academic Foundation

**Damasio (1994), Bechara et al. (2000)**: The Somatic Marker Hypothesis: patients without emotion make consistently worse decisions under uncertainty. Somatic markers are body-state associations that bias choice before deliberation. The Iowa Gambling Task demonstrates pre-cognitive anticipatory signals (skin conductance responses precede conscious awareness).

### Runtime Approximation

Roko implements somatic markers as a k-d tree over 8-dimensional strategy space with PAD-tagged outcomes:

```rust
/// Somatic landscape: fast heuristic feelings about strategy regions
struct SomaticLandscape {
    tree: KdTree<8, SomaticMarker>,  // 8-dim strategy space
}

struct SomaticMarker {
    position: [f64; 8],    // strategy vector (model, temperature, context_size, etc.)
    outcome_pad: Pad,      // PAD affect from last outcome at this position
    confidence: f64,       // how many times this region has been visited
}

impl SomaticLandscape {
    /// "Gut feeling" about a strategy: query nearest markers, return weighted PAD
    fn feeling(&self, strategy: &[f64; 8]) -> Pad {
        let neighbors = self.tree.nearest(strategy, 5);
        let weighted = neighbors.iter()
            .map(|(dist, marker)| {
                let weight = marker.confidence * (1.0 / (1.0 + dist));
                (marker.outcome_pad * weight, weight)
            })
            .fold((Pad::neutral(), 0.0), |(acc_pad, acc_w), (p, w)| {
                (acc_pad + p, acc_w + w)
            });
        weighted.0 / weighted.1
    }

    /// After outcome, deposit marker (Prospect Theory: losses weighted 2.25x)
    fn record(&mut self, strategy: &[f64; 8], outcome: f64) {
        let pad = if outcome >= 0.0 {
            Pad { pleasure: outcome.min(1.0), arousal: 0.3, dominance: 0.5 }
        } else {
            // Kahneman-Tversky lambda = 2.25: losses loom larger
            Pad { pleasure: (outcome * 2.25).max(-1.0), arousal: 0.7, dominance: -0.3 }
        };
        self.tree.add(*strategy, SomaticMarker {
            position: *strategy,
            outcome_pad: pad,
            confidence: 1.0,
        });
    }
}
```

### Fidelity Lost

| Full Theory | Runtime | What's Missing |
|-------------|---------|----------------|
| Body-state (SCR, heart rate, cortisol) | PAD vector (3 floats) | No true embodiment; PAD is abstract |
| Pre-cognitive (before deliberation) | Queried during Route Cell | No temporal priority over deliberation |
| Amygdala-OFC circuit | k-d tree lookup | No neural circuit dynamics |
| Continuous bodily influence | Discrete per-tick query | No persistent background affect |
| Social somatic markers (Bechara 2005) | Individual only | No cross-agent marker propagation |

### Higher-Fidelity Path

Bind PAD vectors into HDC space for cross-domain transfer:

```rust
/// Target-state: HDC-bound somatic markers for analogical transfer
struct HdcSomaticLandscape {
    markers: Vec<(HdcVector, Pad)>,  // strategy fingerprint + affect
}

impl HdcSomaticLandscape {
    /// Cross-domain transfer: similar strategies in HDC space share feelings
    fn feeling_by_analogy(&self, strategy_hdc: &HdcVector) -> Pad {
        self.markers.iter()
            .map(|(fingerprint, pad)| {
                let sim = cosine_similarity(fingerprint, strategy_hdc);
                (*pad * sim, sim)
            })
            .filter(|(_, sim)| *sim > 0.3)
            .fold((Pad::neutral(), 0.0), |(acc, w_acc), (p, w)| (acc + p, w_acc + w))
            .0 / self.markers.len().max(1) as f64
    }
}
```

**Graduation criterion**: When somatic markers from one domain (e.g., coding tasks) should inform another domain (e.g., research tasks), HDC binding enables analogical transfer.

---

## Bridge 5: Conformal Prediction --> CalibrationTracker Distribution-Free Sets

### Academic Foundation

**Vovk, Gammerman & Shafer (2005)**: Conformal prediction provides distribution-free prediction intervals with guaranteed coverage. Given a significance level alpha, conformal prediction produces prediction sets that contain the true value with probability >= 1 - alpha, regardless of the underlying distribution. No distributional assumptions required.

### Runtime Approximation

Roko's CalibrationTracker uses a simplified version: empirical pass-rate tracking with EMA smoothing:

```rust
/// Simplified CalibrationTracker: empirical pass rates, not full conformal sets
struct CalibrationTracker {
    pass_counts: HashMap<String, (u64, u64)>,  // (passes, total) per category
    ema_rates: HashMap<String, f64>,            // EMA-smoothed pass rates
    alpha: f64,                                 // EMA smoothing (0.1)
}

impl CalibrationTracker {
    fn observe(&mut self, category: &str, passed: bool) {
        let (passes, total) = self.pass_counts.entry(category.to_string())
            .or_insert((0, 0));
        *total += 1;
        if passed { *passes += 1; }

        let rate = *passes as f64 / *total as f64;
        let ema = self.ema_rates.entry(category.to_string()).or_insert(0.5);
        *ema = self.alpha * rate + (1.0 - self.alpha) * *ema;
    }

    fn predicted_pass_rate(&self, category: &str) -> f64 {
        self.ema_rates.get(category).copied().unwrap_or(0.5)
    }

    /// Simplified confidence: Wilson score interval
    fn confidence_interval(&self, category: &str) -> (f64, f64) {
        let (passes, total) = self.pass_counts.get(category)
            .copied().unwrap_or((0, 1));
        wilson_score_interval(passes, total, 0.95)
    }
}
```

### Fidelity Lost

| Full Theory | Runtime | What's Missing |
|-------------|---------|----------------|
| Distribution-free coverage guarantee | Empirical EMA (assumes stationarity) | No formal guarantee; EMA biases toward recent |
| Nonconformity scores per example | Binary pass/fail per category | No per-example calibration; coarse categories |
| Prediction SETS (not intervals) | Point estimate + Wilson interval | No set-valued predictions |
| Online conformal (constant coverage) | EMA with fixed alpha | Coverage not guaranteed after distribution shift |
| Exchangeability assumption only | Implicit stationarity assumption | Breaks under non-stationary task distributions |

### Higher-Fidelity Path

```rust
/// Target-state: Full split conformal prediction
struct ConformalCalibrationTracker {
    calibration_set: VecDeque<(f64, bool)>,  // (nonconformity_score, label)
    window_size: usize,                       // sliding window for online conformal
    alpha: f64,                               // significance level (e.g., 0.05)
}

impl ConformalCalibrationTracker {
    /// Compute prediction set: all labels whose nonconformity score
    /// would not be in the top alpha-fraction of calibration scores
    fn prediction_set(&self, new_score: f64) -> PredictionSet {
        let n = self.calibration_set.len();
        let quantile_idx = ((n + 1) as f64 * (1.0 - self.alpha)).ceil() as usize;
        let mut scores: Vec<f64> = self.calibration_set.iter()
            .map(|(s, _)| *s)
            .collect();
        scores.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let threshold = scores.get(quantile_idx).copied().unwrap_or(f64::INFINITY);

        PredictionSet {
            threshold,
            contains_pass: new_score <= threshold,
            coverage_guarantee: 1.0 - self.alpha,
        }
    }
}
```

**Graduation criterion**: When CalibrationTracker predictions are used for safety-critical routing decisions (e.g., deciding whether a task needs human review), upgrade to full conformal prediction for formal coverage guarantees.

---

## Bridge Summary Table

| Subsystem | Paper Theory | Runtime Approximation | Fidelity Gap | Graduation Trigger |
|-----------|-------------|----------------------|--------------|-------------------|
| Route | Active inference POMDP | 4-signal linear EFE | No world model, no planning horizon | >20% routing disagreement with POMDP |
| Store | Replicator dynamics | Fixed-rate demurrage + retrieval boost | No population-relative fitness | >10K entries + retrieval regret |
| Coordination | Turing reaction-diffusion | Discrete pheromone deposits + decay | No continuous PDE, no bifurcation analysis | >10 agents + homogeneous behavior |
| Agent/Daimon | Somatic marker hypothesis | PAD k-d tree with Prospect Theory loss weighting | No embodiment, no temporal priority | Cross-domain transfer needed |
| Verify/Score | Conformal prediction | Empirical EMA + Wilson interval | No distribution-free guarantee | Safety-critical routing decisions |

---

## What This Enables

1. **Auditable approximation**: Every simplification is documented with its theoretical cost.
2. **Graduation paths**: Clear criteria for when to invest in higher-fidelity implementations.
3. **Research-informed engineering**: Developers know which paper to read when extending a subsystem.
4. **Replication ledger input**: Each bridge defines what "replicates" means for that subsystem.

## Feedback Loops

- **EFE approximation** calibrates against routing outcomes (did the tier produce a good result?)
- **Demurrage rates** calibrate against retrieval regret (did we prune something we later needed?)
- **Pheromone field** calibrates against collective intelligence (does differentiation increase C-Factor?)
- **Somatic markers** calibrate against decision quality (do "gut feelings" correlate with outcomes?)
- **CalibrationTracker** calibrates against empirical coverage (does the interval contain truth at the claimed rate?)

## Open Questions

1. Is the 4-signal EFE approximation sufficient for 95% of routing decisions? What is the empirical disagreement rate with a full POMDP solver?
2. Should the replicator equation be run at Delta frequency (offline) or Gamma frequency (online)?
3. Can Turing pattern formation be detected empirically in agent collective behavior logs?
4. How many somatic markers are needed before the landscape provides useful "gut feelings" (hypothesis: ~50 per strategy dimension)?
5. What is the minimum calibration set size for conformal prediction to provide meaningful guarantees in Roko's task distribution?

## Implementation Tasks

| Task | Path | Priority |
|------|------|----------|
| Add `ReplicationLedger` struct to roko-neuro | `crates/roko-neuro/src/replication.rs` | P1 |
| Wire ReplicationLedger to Gate outcomes | `crates/roko-cli/src/orchestrate.rs` | P2 |
| Add `Paper` Signal Kind | `crates/roko-core/src/kind.rs` | P2 |
| Implement population-relative demurrage (adaptive half-life) | `crates/roko-neuro/src/demurrage.rs` | P2 |
| Add SomaticLandscape HDC binding | `crates/roko-daimon/src/somatic.rs` | P2 |
| Implement ConformalCalibrationTracker | `crates/roko-learn/src/calibration.rs` | P2 |
| Add graduation-trigger metrics to `roko learn all` | `crates/roko-cli/src/learn.rs` | P1 |
| Create starter-kit Claims for 12 foundational papers | `.roko/research/claims/` | P1 |
