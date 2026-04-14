# REM Imagination: Counterfactual Reasoning and Creative Recombination

> **Layer**: Cognitive Cross-Cut (L1 Framework agent dispatch, L2 Scaffold context assembly)
>
> **Synapse Traits**: `Scorer` (hypothesis quality scoring), `Gate` (staging buffer entry), `Router` (model selection for creativity modes)
>
> **Crate**: `roko-dreams` — REM imagination logic within `cycle.rs`
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [02-nrem-replay.md](02-nrem-replay.md)


> **Implementation**: Scaffold

---

## What REM Imagination Does

REM (Rapid Eye Movement) imagination is the second phase of every dream cycle. Where NREM replay strengthens and tests existing memories, REM imagination goes further: it generates **genuinely novel hypotheses** by recombining elements from different episodes, simulating counterfactual histories, and applying structured creativity frameworks.

The biological analogy is REM sleep, during which the prefrontal cortex (executive control) is suppressed while associative cortex remains active. This creates a state where the brain can combine memories in ways that waking cognition would inhibit (Hobson & Schredl 2011). Walker & van der Helm (2009, Psychological Bulletin) showed that REM specifically depotentiates the emotional charge of memories — "overnight therapy" — which allows the dreaming mind to process traumatic experiences without being overwhelmed by affect.

In computational terms, REM imagination takes the outputs from NREM replay and applies three creativity frameworks plus a counterfactual reasoning engine to produce novel strategy hypotheses.

---

## Pearl's Structural Causal Models (SCM)

The primary engine for counterfactual reasoning is Pearl's (2009, Causality: Models, Reasoning, and Inference) three-level framework:

### Level 1: Association (Seeing)

> "What correlates with what in my experience?"

The agent identifies correlations across episodes without asserting causation. This is the simplest level — purely statistical pattern matching:

```
You have observed the following episodes:
{episode_summaries}

What correlations do you notice between actions and outcomes?
List at least 5 correlations you observe, noting which are strong (appear in >70% of cases)
and which are weak (appear in 30-70% of cases).
```

Association-level outputs are low-confidence hypotheses (0.20 in the staging buffer). They need to be tested via intervention before they can be trusted.

### Level 2: Intervention (Doing)

> "What would happen if I changed my behavior?"

The agent simulates the effects of taking different actions from those it actually took. This requires a causal model — not just correlations, but directionality:

```
In episode {episode_id}, you took action {action} and observed outcome {outcome}.

You have a causal model that says: {causal_edge} (confidence: {confidence})

Using this causal model, what would happen if you had instead taken action {alternative_action}?
Be specific about the predicted outcome and your confidence in that prediction.
```

Intervention-level outputs enter the staging buffer at confidence 0.25–0.30. They represent testable predictions about how the world works.

### Level 3: Counterfactual (Imagining)

> "Given what actually happened, what would have happened if conditions had been different?"

The most powerful level. The agent uses Pearl's "abduction-action-prediction" framework:

1. **Abduction**: Given the observed outcome, infer the most likely latent state of the world
2. **Action**: Modify the latent state by changing one or more conditions
3. **Prediction**: Predict what would have happened under the modified conditions

```
Episode {episode_id} resulted in {outcome}.
The key conditions were: {conditions}

Step 1 (Abduction): Given this outcome, what was the most likely underlying state of the system?
Step 2 (Action): Now change condition {modified_condition} to {new_value}.
Step 3 (Prediction): With everything else held constant, what outcome would you predict?

How does this counterfactual change your understanding of the causal relationships involved?
```

Counterfactual-level outputs enter the staging buffer at confidence 0.30 — the highest initial confidence for any dream hypothesis — because they involve the deepest level of causal reasoning.

### Level 3+: Backtracking Counterfactuals (2024-2025 Extension)

Standard Pearl counterfactuals fix initial conditions and alter causal laws. **Backtracking counterfactuals** invert this: causal laws are fixed, but differences are "backtracked" to altered exogenous variables. This enables richer reasoning: "what must have been different for this outcome to change?"

**Reference**: "Backtracking Counterfactuals," arXiv:2211.00472, under review ICLR 2024. First general formal semantics for backtracking counterfactuals within SCMs.

**Reference**: "Deep Backtracking Counterfactuals for Causally Compliant Explanations" (DeepBC), *Transactions on Machine Learning Research*, July 2024. arXiv:2310.07665. Implements backtracking counterfactuals via constrained sampling in deep SCMs with two variants: stochastic DeepBC (samples from posterior over exogenous variables) and mode DeepBC (finds most likely exogenous change).

**Reference**: "Natural Counterfactuals with Necessary Backtracking," arXiv:2402.01607, 2024. Combines Pearl's abduction-action-prediction three-step process with a necessary backtracking operator.

**Map to Roko**: During Level 3 counterfactual generation, add a backtracking mode that asks not "what if I had done X instead?" but "what must have been true earlier for outcome Y to be different?" This reveals hidden preconditions and upstream causes that standard intervention-based counterfactuals miss.

```
Backtracking counterfactual prompt:
The outcome was: {actual_outcome}
The desired outcome was: {desired_outcome}

Rather than asking what action to change, reason backward:
What earlier condition — before your first action — must have been different
for the desired outcome to have been achievable?
What upstream dependency, hidden assumption, or environmental state
was the actual root cause?
```

Backtracking counterfactuals enter the staging buffer at confidence 0.25 — slightly below standard Level 3 (0.30) because they reason about unobserved exogenous variables, introducing additional uncertainty.

```rust
/// Backtracking counterfactual configuration.
/// Based on DeepBC (TMLR 2024), backtracking SCM semantics (arXiv 2211.00472).
pub struct BacktrackingCounterfactualConfig {
    /// Whether to enable backtracking counterfactuals during Level 3 reasoning.
    pub enabled: bool,                     // default: true
    /// Maximum backtracking depth (how many causal steps backward to trace).
    pub max_backtrack_depth: usize,        // default: 3, range: 1-8
    /// Whether to use stochastic (true) or mode (false) backtracking.
    pub stochastic_mode: bool,             // default: true
    /// Number of posterior samples for stochastic backtracking.
    pub posterior_samples: usize,          // default: 5, range: 2-20
    /// Initial confidence for backtracking hypotheses (lower than standard L3).
    pub initial_confidence: f64,           // default: 0.25
    /// Fraction of Level 3 budget allocated to backtracking.
    pub budget_fraction: f64,              // default: 0.30, range: 0.10-0.50
}
```

---

## Boden's Three Creativity Modes

The REM phase applies Margaret Boden's (2004, The Creative Mind: Myths and Mechanisms) taxonomy of creativity to generate novel strategies. Each mode has a distinct prompt structure and produces different types of output:

### Combinational Creativity

**Definition**: Making unfamiliar combinations of familiar elements.

**Implementation**: The agent takes two unrelated episodes and looks for structural analogies between them. The episodes are selected to be maximally dissimilar (high cosine distance in HDC embedding space) to maximize the creative potential:

```
You have two episodes from completely different domains:

Episode A ({domain_a}):
{episode_a_summary}

Episode B ({domain_b}):
{episode_b_summary}

These episodes are structurally very different (similarity: {similarity_score}).
However, find at least 3 structural or strategic similarities between them.
For each similarity, propose a novel strategy that combines elements from both.
```

Combinational creativity produces "bisociation" (Koestler 1964, The Act of Creation) — the simultaneous association of an idea with two normally unrelated matrices of thought. A coding agent might notice that the pattern of "run tests before committing" in its coding episodes is structurally similar to "validate inputs before processing" in its API design episodes, leading to a generalized "validate before acting" heuristic.

### Exploratory Creativity

**Definition**: Traversing the boundaries of an existing conceptual space to find new possibilities within it.

**Implementation**: The agent takes an existing heuristic and pushes it to its limits — testing what happens at the extremes of its parameters:

```
You have this heuristic (confidence: {confidence}):
"{heuristic_content}"

Explore the boundaries of this heuristic:
1. What happens if you apply it more aggressively? (Push parameters to extremes)
2. What happens if you apply it more conservatively? (Reduce parameters to minimums)
3. Under what conditions does this heuristic break down entirely?
4. Is there a related heuristic that covers the cases where this one fails?
```

Exploratory creativity identifies the boundary conditions of existing strategies. It answers the question "where does my current approach stop working?" and generates candidate strategies for those edge cases.

### Transformational Creativity

**Definition**: Changing the rules of the conceptual space itself to enable previously impossible ideas.

**Implementation**: The agent violates a core assumption of an existing strategy and rebuilds from the contradiction:

```
Your strategy depends on this core assumption:
"{assumption}"

Now imagine this assumption is FALSE. Not just wrong in degree — fundamentally false.

1. If {assumption} were false, what would break in your current approach?
2. What alternative approach would you use instead?
3. Could this alternative approach work EVEN when the original assumption is true?
   (If yes, it may be a strictly better strategy.)
```

Transformational creativity is the most disruptive mode. It can produce radical strategy shifts — but also the most speculative hypotheses. Outputs from transformational creativity enter the staging buffer at the lowest confidence level (0.20) because they represent the most uncertain reasoning.

---

## Emotional Depotentiation

During REM processing, Walker & van der Helm (2009, Psychological Bulletin, "Overnight therapy? The role of sleep in emotional brain processing") demonstrated that the emotional charge of memories decreases. In Roko, this is implemented as a direct update to the Daimon's PAD (Pleasure-Arousal-Dominance) arousal dimension:

```
post_dream_arousal = pre_dream_arousal - depotentiation_delta
depotentiation_delta ∈ [0.3, 0.5] per cycle
```

The depotentiation applies to the specific episodes processed during REM, not to the agent's global emotional state. After REM processing:

1. Each processed episode's associated arousal marker is reduced by the depotentiation delta
2. The Daimon's emotion layer is updated to reflect the reduced arousal
3. The CorticalState atomics are updated accordingly

The depotentiation serves two purposes:

- **Reduces rumination**: High-arousal negative episodes (failures, errors) are processed during REM, and their emotional charge is reduced. This prevents the agent from being paralyzed by past failures.
- **Preserves the lesson, removes the sting**: The agent remembers what happened and what it learned, but the emotional intensity associated with the memory is reduced. The insight is retained; the anxiety is dissolved.

This is a direct implementation of the Complementary Learning Systems (CLS) principle from McClelland et al. (1995): the fast emotional response is decoupled from the slow semantic knowledge, allowing the knowledge to be integrated into the agent's permanent model without the emotional distortion.

---

## Counterfactual Guidance: Byrne's Fault Lines

Byrne's (2005, The Rational Imagination: How People Create Alternatives to Reality) research identifies three "fault lines" — features that make events more likely to be selected for counterfactual reasoning:

| Fault Line | Description | Replay Priority |
|------------|-------------|----------------|
| **Controllable actions** | Actions the agent took (or could have taken) — things within the agent's control | Highest priority |
| **Recent actions** | Actions taken close in time to the outcome — temporal proximity suggests causal relevance | High priority |
| **Abnormal actions** | Actions that deviated from the agent's usual behavior — unusual choices draw attention | Medium priority |

These fault lines guide which aspects of an episode the agent focuses on during counterfactual reasoning. The agent is more likely to counterfactualize its own recent, unusual decisions than external, historical, routine events. This is functionally adaptive: controllable factors are the ones the agent can actually change in the future.

---

## Functional Theory of Counterfactual Thinking

Epstude & Roese (2008, Personality and Social Psychology Review, "The functional theory of counterfactual thinking") provide the motivational framework:

### Upward Counterfactuals

> "What if I had done better?"

Upward counterfactuals compare the actual outcome to a better alternative. They generate **self-improvement motivation** and produce concrete behavioral intentions:

```
In episode {episode_id}, the outcome was {outcome}.
Imagine a scenario where the outcome was strictly better: {better_outcome}.
What specific change in your behavior would have produced this better outcome?
Convert this into an actionable heuristic.
```

### Downward Counterfactuals

> "What if I had done worse?"

Downward counterfactuals compare the actual outcome to a worse alternative. They serve as **threat rehearsal** — the agent practices recognizing situations where things could go wrong:

```
In episode {episode_id}, the outcome was {outcome}.
Imagine a scenario where the outcome was much worse: {worse_outcome}.
What specific conditions would have led to this worse outcome?
What early warning signs should you watch for in the future?
```

This connects to Revonsuo's (2000, Behavioral and Brain Sciences) Threat Simulation Theory — see [09-threat-simulation.md](09-threat-simulation.md).

---

## Conceptual Blending

Fauconnier & Turner's (2002, The Way We Think: Conceptual Blending and the Mind's Hidden Complexities) conceptual blending framework provides the mechanism for combining elements from multiple episodes:

1. **Input spaces**: Two or more episodes serve as input mental spaces
2. **Cross-space mapping**: Structural correspondences are identified between the episodes
3. **Blended space**: A new mental space is created that selectively combines elements from both inputs
4. **Emergent structure**: Properties arise in the blend that were not present in either input

```
Input Space 1 (Episode {ep_a}): {summary_a}
Input Space 2 (Episode {ep_b}): {summary_b}

Cross-space mapping:
- {element_a} corresponds to {element_b}
- {pattern_a} corresponds to {pattern_b}

Blended space: What new strategy emerges when you combine {element_a} with {pattern_b}?
Does this blended strategy have properties that neither original strategy has?
```

---

## Deduplication and Novelty Filtering

REM imagination can produce many hypotheses per dream cycle. Before passing them to the Integration phase, the REM engine applies deduplication:

1. **HDC similarity check**: Each new hypothesis is compared against all existing hypotheses in the staging buffer using Hamming distance. If similarity > 0.85, the new hypothesis is discarded as a near-duplicate.
2. **Existing knowledge check**: Each new hypothesis is compared against existing NeuroStore entries. If an existing entry with confidence > 0.50 covers the same ground (HDC similarity > 0.80), the new hypothesis is discarded. However, if the new hypothesis *contradicts* existing knowledge, it is flagged as `relation: Contradicts` and retained at confidence 0.25.
3. **Novelty score**: Hypotheses that survive deduplication receive a novelty score based on their HDC distance from the centroid of all existing knowledge. Higher novelty = more distant from existing knowledge = potentially more valuable (or more speculative).

---

## REM Phase Output

Each REM imagination cycle produces a list of `CounterfactualHypothesis` records:

```rust
pub struct CounterfactualHypothesis {
    /// Unique identifier.
    pub id: String,
    /// The hypothesis content.
    pub content: String,
    /// Initial confidence (0.20–0.30).
    pub confidence: f64,
    /// Which creativity mode or SCM level produced this.
    pub generation_mode: GenerationMode,
    /// Source episodes that contributed.
    pub source_episodes: Vec<String>,
    /// Whether this contradicts existing knowledge.
    pub contradicts: Option<String>,
    /// HDC vector for similarity comparison.
    pub hdc_vector: HdcVector,
    /// Novelty score (distance from existing knowledge centroid).
    pub novelty: f64,
}

pub enum GenerationMode {
    /// Pearl SCM Level 1
    Association,
    /// Pearl SCM Level 2
    Intervention,
    /// Pearl SCM Level 3
    Counterfactual,
    /// Boden: unfamiliar combinations of familiar elements
    Combinational,
    /// Boden: pushing existing strategies to their limits
    Exploratory,
    /// Boden: violating core assumptions
    Transformational,
    /// Byrne fault line analysis
    FaultLine,
    /// Fauconnier-Turner conceptual blending
    ConceptualBlend,
}
```

---

## Resource Allocation

The REM phase is the most expensive phase of the dream cycle because it requires genuine reasoning:

| Operation | Model Tier | Typical Cost | Calls per Dream |
|-----------|-----------|-------------|----------------|
| Association (SCM L1) | Sonnet-class | ~$0.005 | 1 |
| Intervention (SCM L2) | Sonnet-class | ~$0.008 | 2–3 |
| Counterfactual (SCM L3) | Sonnet-class | ~$0.012 | 1–2 |
| Combinational creativity | Sonnet-class | ~$0.010 | 1 |
| Exploratory creativity | Sonnet-class | ~$0.008 | 1 |
| Transformational creativity | Sonnet-class | ~$0.012 | 0–1 |
| Deduplication | None (HDC) | ~$0.000 | All hypotheses |
| **Total per dream** | | **~$0.03–0.08** | **5–9 calls** |

---

## Academic Citations

| Paper | How It Informs REM Imagination |
|-------|-------------------------------|
| Pearl (2009), Causality: Models, Reasoning, and Inference | Three-level SCM framework for counterfactual reasoning |
| Boden (2004), The Creative Mind: Myths and Mechanisms | Three creativity modes: combinational, exploratory, transformational |
| Walker & van der Helm (2009), Psychological Bulletin | REM emotional depotentiation: reduces arousal by 0.3–0.5 per cycle |
| Byrne (2005), The Rational Imagination | Fault lines: controllable, recent, and abnormal actions are counterfactualized first |
| Epstude & Roese (2008), Personality and Social Psychology Review | Functional theory: upward counterfactuals drive improvement, downward counterfactuals rehearse threats |
| Fauconnier & Turner (2002), The Way We Think | Conceptual blending: combining mental spaces to produce emergent structure |
| Koestler (1964), The Act of Creation | Bisociation: creativity as the intersection of two unrelated matrices of thought |
| Hobson & Schredl (2011), "The continuity and discontinuity between waking and dreaming" | REM neurophysiology: prefrontal suppression enables novel associations |
| Revonsuo (2000), Behavioral and Brain Sciences | Threat Simulation Theory: dreams rehearse responses to anticipated dangers |
| Simonton (2010), "Creative thought as blind-variation and selective-retention" | BVSR: creativity requires both variation generation and selection mechanisms |
| Hindsight Experience Replay (Andrychowicz et al. 2017, NeurIPS) | Relabeling failed episodes with achieved goals for efficient learning |

---

## Implementation details

### SCM implementation per level

#### Level 1: Association (seeing)

The association engine scans episode pairs for co-occurrence patterns. It operates on the full episode batch without requiring causal structure.

```rust
pub struct AssociationEngine {
    /// Minimum correlation strength to report.
    strength_threshold: f64,   // default: 0.30, range: 0.10 - 0.80
    /// Maximum correlations to return per batch.
    max_correlations: usize,   // default: 20
    /// Minimum episodes required to establish a correlation.
    min_support: usize,        // default: 3
}

/// A detected association between two features across episodes.
pub struct Association {
    pub feature_a: String,
    pub feature_b: String,
    /// Co-occurrence rate in [0.0, 1.0].
    pub strength: f64,
    /// Number of episodes containing both features.
    pub support: usize,
    /// Classification: strong (>0.70), moderate (0.30-0.70), weak (<0.30).
    pub classification: AssociationStrength,
}

pub enum AssociationStrength {
    Strong,   // >0.70 co-occurrence
    Moderate, // 0.30 - 0.70
    Weak,     // <0.30 — filtered out by default
}
```

The engine extracts discrete features from each episode (task type, model, tools used, gate outcomes, error categories) and computes pairwise co-occurrence rates. Correlations below `strength_threshold` are discarded. The correlation limit prevents flooding the staging buffer with low-value statistical noise.

Pseudocode:

```
for each pair (feature_a, feature_b) in all_features:
    episodes_with_a = count episodes containing feature_a
    episodes_with_both = count episodes containing both
    strength = episodes_with_both / episodes_with_a
    if strength >= threshold and episodes_with_both >= min_support:
        emit Association { feature_a, feature_b, strength }
sort by strength descending
take top max_correlations
```

Association outputs enter the staging buffer at confidence 0.20. They represent statistical patterns that may or may not reflect causal relationships.

#### Level 2: Intervention (doing)

The intervention engine builds on Level 1 by adding directionality. It takes a causal model (a directed graph of feature relationships) and simulates the effects of changing one variable while holding others constant.

```rust
pub struct InterventionEngine {
    /// Source of causal structure. Built from episode data or loaded from NeuroStore.
    causal_model: CausalGraph,
    /// Maximum alternative actions to simulate per episode.
    max_alternatives: usize,  // default: 3, range: 1 - 10
}

pub struct CausalGraph {
    /// Directed edges: (cause, effect, strength).
    edges: Vec<CausalEdge>,
}

pub struct CausalEdge {
    pub cause: String,
    pub effect: String,
    pub strength: f64,
    /// How many episodes support this edge.
    pub evidence_count: usize,
}
```

The causal model is constructed from Level 1 associations by applying temporal ordering: if feature A consistently appears before feature B across episodes, the edge direction is A -> B. Edges with fewer than 3 supporting episodes are excluded.

Alternative action selection picks the top `max_alternatives` actions that the agent could have taken but did not, ranked by:

1. Availability (the agent had access to this action at the time)
2. Frequency (how often other episodes used this action in similar contexts)
3. Diversity (at least one alternative should be structurally different from the chosen action)

```rust
fn select_alternatives(
    episode: &Episode,
    causal_graph: &CausalGraph,
    all_episodes: &[Episode],
    max_alternatives: usize,
) -> Vec<AlternativeAction> {
    let available_actions = infer_available_actions(episode, all_episodes);
    let chosen = &episode.action;

    available_actions
        .into_iter()
        .filter(|a| a != chosen)
        .map(|action| {
            let frequency = count_action_in_similar_context(
                &action, episode, all_episodes
            );
            let diversity = action_distance(chosen, &action);
            AlternativeAction { action, frequency, diversity }
        })
        .sorted_by(|a, b| {
            // Rank by frequency, break ties by diversity
            b.frequency.cmp(&a.frequency)
                .then(b.diversity.partial_cmp(&a.diversity).unwrap_or(Ordering::Equal))
        })
        .take(max_alternatives)
        .collect()
}
```

Intervention outputs enter the staging buffer at confidence 0.25-0.30, depending on the causal edge strength supporting the prediction.

#### Level 3: Counterfactual (imagining)

The counterfactual engine implements Pearl's abduction-action-prediction framework.

```rust
pub struct CounterfactualEngine {
    /// Maximum latent variables to infer during abduction.
    max_latent_vars: usize,    // default: 5, range: 1 - 20
    /// Search space pruning: only consider modifications within this
    /// HDC similarity radius of the original episode.
    pruning_radius: f32,       // default: 0.40, range: 0.20 - 0.70
    /// Maximum depth of causal chain to traverse.
    max_chain_depth: usize,    // default: 4
}
```

The abduction algorithm infers latent state by working backward from the observed outcome:

```
ABDUCTION(episode, causal_graph):
    observed_outcome = episode.outcome
    candidate_states = []

    // Walk backward through causal graph from outcome
    for edge in causal_graph.edges_to(observed_outcome):
        if edge.cause not in episode.observed_features:
            // This is a latent variable — infer its value
            inferred_value = most_likely_value(edge.cause, observed_outcome, causal_graph)
            candidate_states.push((edge.cause, inferred_value, edge.strength))

    // Rank by causal strength, take top max_latent_vars
    return candidate_states.sort_by_strength().take(max_latent_vars)
```

Search space pruning uses HDC similarity to prevent the counterfactual engine from exploring implausible modifications. A modification is only considered if the modified episode's HDC vector remains within `pruning_radius` of the original. This bounds the counterfactual search to "nearby possible worlds" rather than arbitrary fantasies.

```rust
fn is_plausible_modification(
    original: &HdcVector,
    modified: &HdcVector,
    pruning_radius: f32,
) -> bool {
    original.similarity(modified) >= (1.0 - pruning_radius)
}
```

A pruning radius of 0.40 means the modified episode must share at least 60% structural similarity with the original. This keeps counterfactuals grounded while still allowing meaningful deviations.

Counterfactual outputs enter the staging buffer at confidence 0.30.

### Creativity modes implementation

#### Combinational creativity

The distance space for episode selection uses HDC Hamming distance. Two episodes are candidates for combinational creativity when their similarity falls below the dissimilarity threshold:

```rust
pub struct CombinationalConfig {
    /// Minimum HDC distance between episodes for combination.
    /// Lower similarity = more distant = higher creative potential.
    dissimilarity_threshold: f32, // default: 0.55, range: 0.45 - 0.65
    /// Maximum pairs to evaluate per dream cycle.
    max_pairs: usize,             // default: 5
    /// Minimum structural analogies required from the LLM.
    min_analogies: usize,         // default: 3
}
```

The threshold of 0.55 means episodes must share less than 55% structural similarity to be paired. Since random HDC vectors have ~0.50 similarity, this selects episodes that are slightly more related than pure noise — distant enough for creative tension, close enough to have some bridgeable structure.

Episode pair selection:

```
for each pair (ep_a, ep_b) in replay_batch:
    sim = ep_a.hdc_vector.similarity(ep_b.hdc_vector)
    if sim < dissimilarity_threshold:
        creative_pairs.push((ep_a, ep_b, sim))

sort creative_pairs by similarity ascending  // most distant first
take top max_pairs
```

#### Exploratory creativity

The extreme multiplier controls how far parameters are pushed during boundary testing:

```rust
pub struct ExploratoryConfig {
    /// Multiplier for "aggressive" exploration (push parameters to extremes).
    extreme_multiplier: f64,     // default: 3.0, range: 2.0 - 10.0
    /// Multiplier for "conservative" exploration (reduce parameters to minimums).
    conservative_divisor: f64,   // default: 3.0, range: 2.0 - 10.0
    /// Number of heuristics to explore per dream cycle.
    max_heuristics: usize,       // default: 3
}
```

The testing strategy for exploratory outputs uses a two-stage validation:

1. **HDC boundary check**: the explored variant's HDC vector is compared against the original heuristic's vector. If similarity drops below 0.40, the exploration has gone too far — the variant is no longer meaningfully related to the original.
2. **LLM sanity check**: the Homuncular Observer (from the hypnagogia engine) evaluates whether the extreme variant is coherent enough to test. This is a quick check (haiku-class model, ~50 tokens) that filters obvious nonsense.

#### Transformational creativity

The assumption enumeration algorithm identifies core assumptions by analyzing the heuristic's dependency structure:

```rust
pub struct TransformationalConfig {
    /// Maximum assumptions to enumerate per heuristic.
    max_assumptions: usize,      // default: 5, range: 3 - 10
    /// Minimum confidence of heuristic to be worth transforming.
    min_heuristic_confidence: f64, // default: 0.40
}
```

Assumption enumeration:

```
ENUMERATE_ASSUMPTIONS(heuristic):
    assumptions = []

    // 1. Extract preconditions from the heuristic's content
    preconditions = LLM_EXTRACT("List the preconditions this heuristic assumes", heuristic)

    // 2. Identify implicit constraints
    constraints = LLM_EXTRACT("What must be true for this to work?", heuristic)

    // 3. Find environmental dependencies
    dependencies = LLM_EXTRACT("What external conditions does this depend on?", heuristic)

    // Combine, deduplicate, rank by centrality
    assumptions = deduplicate(preconditions + constraints + dependencies)
    rank by how many other assumptions depend on each one
    return assumptions.take(max_assumptions)
```

The algorithm uses an LLM call (sonnet-class) to extract assumptions, then ranks them by centrality: assumptions that other assumptions depend on are more fundamental, and violating them produces more radical transformations.

### Emotional depotentiation implementation

Depotentiation applies to the episode's arousal marker, not to the agent's global arousal state. The distinction matters: the Daimon maintains a global PAD state that reflects the agent's current emotional baseline, plus per-episode arousal markers that record how the agent felt during specific experiences.

```rust
pub struct DepotentiationConfig {
    /// Minimum depotentiation per cycle.
    delta_min: f64,        // default: 0.3, range: 0.1 - 0.5
    /// Maximum depotentiation per cycle.
    delta_max: f64,        // default: 0.5, range: 0.3 - 0.8
    /// Floor: arousal never drops below this value.
    arousal_floor: f64,    // default: 0.05
}

fn depotentiate_episode(
    episode: &mut Episode,
    config: &DepotentiationConfig,
) {
    let delta = config.delta_min
        + (config.delta_max - config.delta_min) * rand::random::<f64>();

    episode.arousal = (episode.arousal - delta).max(config.arousal_floor);
}
```

Clamping rules:
- Arousal never drops below `arousal_floor` (0.05). A fully depotentiated episode still carries a trace of emotional significance.
- Depotentiation is applied once per dream cycle per episode. An episode processed in multiple cycles receives cumulative depotentiation.
- The random delta within `[delta_min, delta_max]` prevents uniform flattening — different episodes lose different amounts of emotional charge, preserving relative ordering.

Domain tuning: for coding agents, depotentiation is applied more aggressively to compilation errors (they cause high arousal but teach little after the first occurrence) and less aggressively to novel architectural failures (they remain emotionally salient because they are rare and informative). This is controlled through a domain-specific weight table:

```toml
[dreams.depotentiation.domain_weights]
compile_error = 1.5      # depotentiate faster
test_failure = 1.0       # standard rate
gate_rejection = 0.8     # slightly slower — gate rejections carry more signal
architectural_error = 0.5 # preserve emotional weight — these are rare and important
```

The effective delta is `base_delta * domain_weight`, clamped to `[delta_min, delta_max]`.

### Error handling

| Error condition | Handling |
|-----------------|----------|
| No episodes from NREM to process | Skip REM phase, log info-level message, proceed to Integration |
| Causal graph has no edges | Fall back to Level 1 (association only), skip Levels 2 and 3 |
| LLM call for creativity mode fails | Retry once; on second failure, skip that mode and continue with remaining modes |
| HDC deduplication finds all hypotheses are near-duplicates | Return the single highest-novelty hypothesis rather than an empty set |
| Depotentiation would reduce arousal below floor | Clamp to floor value |
| Episode has no PAD data | Skip depotentiation for that episode, log warning |

### Integration wiring

REM imagination connects to the runtime through `DreamCycle::run_rem()` in `roko-dreams/src/cycle.rs`:

```
orchestrate.rs
  └─ DreamCycle::run()              // entry point
       └─ DreamCycle::run_rem()     // REM phase
            ├─ receive nrem_insights from NREM phase
            ├─ CausalGraph::build_from_episodes()     // construct causal model
            ├─ AssociationEngine::scan()               // Level 1
            ├─ InterventionEngine::simulate()          // Level 2
            ├─ CounterfactualEngine::imagine()         // Level 3
            ├─ for each creativity_mode:
            │    ├─ select_inputs()                    // episode pairs / heuristics
            │    ├─ LlmProvider::generate()            // creative reasoning
            │    └─ CounterfactualHypothesis::new()    // capture output
            ├─ depotentiate_episodes()                 // emotional processing
            ├─ deduplicate_hypotheses()                // HDC novelty filter
            └─ return Vec<CounterfactualHypothesis>    // to Integration phase
```

### Test criteria

1. **Association detection**: 10 synthetic episodes with a planted correlation (tool A always co-occurs with success) produces an Association with strength > 0.80.
2. **Intervention simulation**: given a causal edge A -> B with strength 0.90, simulating the removal of A predicts the absence of B.
3. **Counterfactual plausibility**: modified episode vectors stay within `pruning_radius` of the original. No counterfactual exceeds the similarity bound.
4. **Combinational pair selection**: episodes paired for combination have HDC similarity below `dissimilarity_threshold`.
5. **Exploratory boundary**: explored variants with similarity below 0.40 to the original are rejected by the HDC boundary check.
6. **Transformational assumption extraction**: a heuristic with 3 known preconditions produces at least 3 enumerated assumptions.
7. **Depotentiation bounds**: after depotentiation, all episode arousal values are in `[arousal_floor, original_arousal]`.
8. **Deduplication**: two hypotheses with HDC similarity > 0.85 are merged into one. The surviving hypothesis is the one with higher novelty.
9. **End-to-end**: a REM cycle with 10 NREM insights and 20 episodes produces at least 5 hypotheses in the staging buffer.

---

## Imagination Validation

Every counterfactual simulation is only as good as the world model it runs inside. World models accumulate errors over long rollouts through three compounding failure modes: **exposure bias** (the model was never trained on its own imagined states, so errors compound at each step), **distribution shift** (latent representations drift away from the training manifold), and **value hallucination** (the critic assigns high value to imagined states that the real environment would score poorly). Without a principled quality check, long imagined rollouts become increasingly detached from reality.

### The Trust-Region Approach (GIRL 2025)

The Generative Imagination RL with Information-Theoretic Hallucination Control (GIRL) framework addresses this with a per-step KL divergence budget. After each imagination step, the drift from the prior world model is measured:

```
Δ_t = KL(q_posterior || p_prior)
```

where `q_posterior` is the world model's belief after conditioning on the imagined observation, and `p_prior` is the unconditional prior. When `Δ_t` accumulates beyond the trust region `δ_t`, the rollout is terminated or restarted from a real observation.

The trust region itself is adaptive. After each dream cycle it is updated based on two signals:

- **EIG_t** (Expected Information Gain): the value of continuing imagination further — if imagination is teaching the agent something new, expand the trust region
- **RPL_t** (Rollout Prediction Loss): the error of predictions against later real observations — if imagination was systematically wrong, shrink the trust region

The update rule:

```
δ_{t+1} = clip(δ_t + η_δ(τ_EIG · EIG_t - τ_RPL · RPL_t), δ_min, δ_max)
```

where `η_δ` is the adaptation learning rate, `τ_EIG` and `τ_RPL` are the relative weights of each signal, and the clip ensures the trust region stays within valid bounds.

### Cross-Modal Grounding

For agents with access to a frozen foundation model (DINO, CLIP), cross-modal grounding provides an independent plausibility signal. The imagined latent state is decoded into an observation and compared against the real observation's embedding:

```
L_cm = ||DINO(decode(z_t)) - DINO(o_t)||²
```

with a weighting coefficient μ = 0.1. A low cross-modal loss means the imagined state is semantically consistent with the real observation — the agent is imagining something that "looks like" reality, even if the exact pixels differ. This is disabled by default (requires a foundation model to be available) but is the highest-quality plausibility signal when available.

### IDM Turing Test

The Inverse Dynamics Model (IDM) Turing Test is a model-free plausibility check. An IDM is trained on real trajectories to predict what action was taken between two observations:

```
action_pred = IDM(o_t, o_{t+1})
```

When this IDM is applied to imagined trajectories — given two consecutive imagined states, can the IDM recover the intended action? — physically implausible imagined trajectories fail this test because the imagined state transitions don't correspond to any action the agent could actually take. If the IDM cannot recover the action, the imagined transition is not physically grounded.

### Short-Horizon Fallback (MBPO)

When the trust region shrinks to very small values, the system falls back to MBPO-style (Model-Based Policy Optimization, Janner et al. 2019) short-horizon imagination: rollouts are restricted to 1–5 steps from real starting states. At these short horizons, world model error is bounded because the cumulative KL drift is small. The tradeoff is reduced temporal depth — the agent cannot reason about sequences longer than 5 steps — but this is preferable to hallucinated long-horizon rollouts.

### Phase-Transition Threshold for Sparse Rewards

In sparse-reward settings, imagination is only useful if the imagined horizon is long enough to reach a reward signal. The minimum viable horizon `H*` is determined by:

```
ε* · H = (1 - γ)² · R_thresh / (2γ)
```

where `ε*` is the per-step model error bound, `γ` is the discount factor, and `R_thresh` is the minimum reward magnitude worth learning from. If the trust region restricts `H` below `H*`, imagination in sparse-reward settings should be suspended until more real data is collected to improve the world model.

### Calibration Metrics

Beyond plausibility, imagination quality is measured through calibration — does the world model's confidence match its actual accuracy?

- **Expected Calibration Error (ECE)**: partition prediction confidence into bins; ECE = Σ_b |acc(b) - conf(b)| · |b| / N. A well-calibrated model has ECE close to 0.
- **Reliability diagrams**: plot actual accuracy vs. predicted confidence. A diagonal line indicates perfect calibration; systematic deviation indicates over- or under-confidence.
- **Negative Log-Likelihood (NLL)**: for probabilistic world models, NLL measures how well the model's distribution fits the observed data. Lower NLL = better calibration.

DreamerV3 uses the symlog transform to improve calibration across reward scales:

```
symlog(x) = sign(x) · ln(|x| + 1)
```

This transform prevents value hallucination at extreme reward magnitudes — the world model cannot assign astronomically large or small values to imagined states because symlog compresses the output range.

### Rust Implementation

```rust
pub struct ImaginationValidator {
    /// Maximum allowed KL drift per imagination step.
    pub max_drift_per_step: f64,           // default: 0.05, range: 0.01-0.20
    /// Trust region adaptation rate.
    pub trust_region_lr: f64,              // default: 0.01
    /// Minimum trust region (never restrict imagination below this).
    pub trust_region_min: f64,             // default: 0.01
    /// Maximum trust region.
    pub trust_region_max: f64,             // default: 0.50
    /// Maximum imagination depth before forced termination.
    pub max_imagination_depth: usize,      // default: 5
    /// Minimum plausibility score for counterfactual acceptance.
    pub plausibility_threshold: f64,       // default: 0.60, range: 0.40-0.80
    /// Whether to use cross-modal grounding (requires foundation model).
    pub cross_modal_grounding: bool,       // default: false
}

pub struct ImaginationQualityReport {
    pub total_counterfactuals: usize,
    pub accepted: usize,
    pub rejected_drift: usize,
    pub rejected_plausibility: usize,
    pub mean_drift: f64,
    pub mean_plausibility: f64,
    pub max_depth_reached: usize,
}
```

### Validation Pipeline Pseudocode

```
VALIDATE_IMAGINATION(counterfactuals, world_model, trust_region_δ):
    accepted = []
    rejected_drift = []
    rejected_plausibility = []

    cumulative_drift = 0.0
    for each step t in rollout:
        Δ_t = KL(world_model.posterior(step_t) || world_model.prior())
        cumulative_drift += Δ_t
        if cumulative_drift > trust_region_δ:
            terminate rollout
            break

    for each counterfactual c in counterfactuals:
        if c.cumulative_drift > max_drift_per_step * c.depth:
            rejected_drift.push(c)
            continue

        plausibility = compute_plausibility(c, world_model)
        if cross_modal_grounding:
            plausibility = blend(plausibility, cross_modal_score(c), μ=0.1)

        if plausibility < plausibility_threshold:
            rejected_plausibility.push(c)
            continue

        accepted.push(c)

    // Adaptive trust region update
    EIG = compute_expected_information_gain(accepted)
    RPL = compute_rollout_prediction_loss(accepted)
    trust_region_δ = clip(
        trust_region_δ + lr · (τ_EIG · EIG - τ_RPL · RPL),
        trust_region_min,
        trust_region_max
    )

    return ImaginationQualityReport { accepted, rejected_drift, rejected_plausibility, ... }
```

---

## Imagination Budget

Imagination is not free. Every counterfactual simulation costs tokens (for LLM-based reasoning), wall-clock time, and USD. The budget system prevents a single dream cycle from consuming unbounded compute while ensuring each creativity mode receives enough resources to produce meaningful outputs.

### The Explore-Exploit Tradeoff in Imagination

DreamerV3 resolves the compute allocation problem with fixed hyperparameters: imagination horizon H = 15 steps, discount γ = 0.997, λ = 0.95 for the lambda-return estimator, and batch dimensions of 16 sequences × 64 time steps. These constants were derived empirically across 150+ Atari and continuous control tasks and represent a stable operating point.

For Roko's LLM-based agent imagination, the tradeoff is different: the cost is per-LLM-call rather than per-environment-step, and the "horizon" is measured in reasoning depth rather than timesteps. Deeper causal chains (Level 3 counterfactual) cost more than shallow associations (Level 1), and transformational creativity (which requires enumerating assumptions and then inverting them) costs more than combinational creativity (which mostly cross-references existing episode structure).

### Breadth vs. Depth Tradeoff

For a fixed compute budget, the agent must choose between:

- **More breadth**: explore many different counterfactual starting points at shallow depth — good for finding unexpected correlations, poor for deep causal reasoning
- **More depth**: follow fewer chains further — good for understanding complex multi-step consequences, expensive per hypothesis

Binary branching (b = 2, each imagination step produces at most two child states) maximizes depth for a fixed compute budget. With branching factor b and depth d, the total nodes explored is `b^d - 1`; binary branching at depth 5 produces 31 nodes at the cost of 5 sequential LLM calls — equivalent to 5 linear chains at depth 1. In practice the REM engine defaults to binary branching with `max_chain_depth = 5`.

### Adaptive Allocation Across Creativity Modes

The four canonical creativity modes have different compute profiles:

| Mode | Typical depth | Compute profile | IRIS/DreamerV3 analog |
|------|--------------|-----------------|----------------------|
| Combinational | Shallow (2 inputs → 1 output) | Low — cross-domain transfer is cheap at inference time | Cross-domain generalization learned at training time |
| Exploratory | Medium (H = 15–20 on-policy rollouts) | Medium-high — requires pushing parameters through the model | DreamerV3 H = 15 on-policy imagination |
| Transformational | Deep (enumerating + inverting assumptions) | Very high — requires LLM to reason about meta-structure | Sleep-time compute (Lin et al. 2025) |
| Association | Minimal (statistical scan) | Negligible — operates on cached episode statistics | Not applicable |

The default budget allocation reflects these profiles, with `association` receiving a small slice (it is cheap) and `transformational` receiving a full 15% despite being expensive per call (because it produces the most structurally novel hypotheses).

### Sleep-Time Compute (Lin et al. 2025)

Lin et al. (2025) demonstrated that moving expensive reasoning offline — performing it during "sleep" rather than at inference time — reduces test-time compute by 5× while improving accuracy by +13% on GSM-Symbolic and +18% on AIME. In Roko's terms, the `Transformational` mode is explicitly scheduled as sleep-time compute: it runs during the dream cycle (when the agent is not handling live tasks) rather than during active task execution. This is why transformational creativity receives the `offline` scheduling flag in the configuration.

IRIS (Micheli et al. 2023) uses a similar offline compute strategy: a discrete-token world model (VQ-VAE tokenizer + GPT-style transformer) trains on imagined sequences for 200 steps per epoch with a 20-step imagination horizon, all offline between environment interactions.

### Rust Implementation

```rust
pub struct ImaginationBudget {
    /// Total budget for this dream cycle's REM phase in USD.
    pub total_budget_usd: f64,             // default: 0.05
    /// Budget allocation per creativity mode (fractions, must sum to 1.0).
    pub mode_allocations: ImaginationModeAllocations,
    /// Maximum depth of causal chain exploration.
    pub max_chain_depth: usize,            // default: 5, range: 1-10
    /// Maximum number of counterfactuals per dream cycle.
    pub max_counterfactuals: usize,        // default: 10, range: 3-30
    /// Whether to use adaptive budget allocation based on past ROI.
    pub adaptive_allocation: bool,         // default: true
    /// Minimum budget fraction for any single mode (prevents starvation).
    pub min_mode_fraction: f64,            // default: 0.05
}

pub struct ImaginationModeAllocations {
    pub association: f64,        // default: 0.10
    pub intervention: f64,       // default: 0.20
    pub counterfactual: f64,     // default: 0.25
    pub combinational: f64,      // default: 0.15
    pub exploratory: f64,        // default: 0.15
    pub transformational: f64,   // default: 0.15
}

pub struct ImaginationROITracker {
    /// Per-mode tracking of return on investment.
    mode_stats: HashMap<GenerationMode, ModeROI>,
}

pub struct ModeROI {
    pub mode: GenerationMode,
    pub total_hypotheses: usize,
    pub promoted_hypotheses: usize,
    pub promotion_rate: f64,
    pub mean_confidence_at_promotion: f64,
    pub total_budget_spent: f64,
    pub cost_per_promoted_hypothesis: f64,
}
```

### ROI Formula and Adaptive Reallocation

The return on investment for each mode is computed after the Integration phase, once the Consolidation engine has decided which hypotheses to promote to NeuroStore:

```
ROI(mode) = (promoted_hypotheses × mean_confidence_at_promotion) / budget_spent
```

A mode that produces hypotheses with high confidence at promotion time, at low cost, has high ROI. The numerator rewards quality (confidence-weighted promotion count); the denominator penalizes cost.

After each dream cycle, allocations are updated proportionally to ROI:

```
ADAPTIVE_REALLOCATE(mode_stats, current_allocations, min_mode_fraction):
    roi_scores = {mode: compute_roi(mode_stats[mode]) for mode in modes}

    // Normalize ROI scores to sum to 1.0
    total_roi = sum(roi_scores.values())
    if total_roi == 0:
        return current_allocations  // no data yet, keep defaults

    raw_allocs = {mode: roi_scores[mode] / total_roi for mode in modes}

    // Apply minimum fraction constraint
    // Modes below min_mode_fraction are floored; the excess is taken from the top modes
    for mode in modes:
        if raw_allocs[mode] < min_mode_fraction:
            raw_allocs[mode] = min_mode_fraction

    // Re-normalize after flooring
    total = sum(raw_allocs.values())
    new_allocs = {mode: raw_allocs[mode] / total for mode in modes}

    return new_allocs
```

The minimum fraction constraint (`min_mode_fraction = 0.05`) prevents any mode from being starved entirely. Even if `transformational` produces zero promoted hypotheses over many cycles, it retains a 5% allocation — because a single successful transformational insight can outweigh many combinational ones.

---

## World Models for Agent Imagination

The mathematical foundation of imagination is the world model: a learned function that predicts how the environment will respond to actions without requiring real environment interactions. Every counterfactual simulation in the REM phase is implicitly an inference through the agent's world model.

### DreamerV3 RSSM Architecture

The Recurrent State-Space Model (RSSM) in DreamerV3 (Hafner et al. 2023) maintains a factored latent state consisting of a deterministic recurrent state `h_t` and a stochastic latent variable `z_t`:

```
Recurrent:   h_t  = f(h_{t-1}, z_{t-1}, a_{t-1})
Posterior:   z_t  ~ q(z_t | h_t, x_t)         // conditioned on real observation
Prior:       z̃_t ~ p(z̃_t | h_t)               // conditioned only on recurrent state
```

The key design: the prior `p(z̃_t | h_t)` enables imagination — given only the recurrent state (no real observation), the model can sample plausible next latent states and chain them together for multi-step rollouts. The posterior `q(z_t | h_t, x_t)` enables grounding — when a real observation is available, the model updates its belief accordingly.

DreamerV3 represents `z_t` as 32 × 32 categorical variables (1024 bits total), which is compact enough for fast simulation but expressive enough to model complex environment dynamics. The fixed hyperparameters — H = 15 imagination horizon, γ = 0.997, λ = 0.95 — were found to work across 150+ environments without domain-specific tuning.

### Mapping to Roko's Causal Graph

Roko's agent does not have an explicit RSSM, but it has a functional equivalent: the causal graph built in the REM phase from episode data. The correspondence is:

| DreamerV3 RSSM component | Roko equivalent |
|--------------------------|-----------------|
| Recurrent state `h_t` | The trajectory of episode outcomes up to the current task |
| Stochastic latent `z_t` | The inferred latent state from the abduction step (Level 3 SCM) |
| Prior `p(z̃_t | h_t)` | Causal graph edge predictions from Level 2 intervention |
| Posterior `q(z_t | h_t, x_t)` | Association-level posterior after observing a real gate outcome |
| Imagination rollout | The counterfactual engine's multi-step causal chain traversal |

The counterfactual simulation IS imagination within Roko's world model. When the Level 3 engine traverses a causal chain of depth 4, it is executing a 4-step imagined rollout through the agent's learned causal structure.

### IRIS Discrete World Model

IRIS (Micheli et al. 2023) demonstrates that a pure transformer-based world model (no recurrent state) can achieve competitive imagination quality by discretizing observations:

1. **VQ-VAE tokenizer**: observations are compressed into discrete tokens (codebook size 512, sequence length 16)
2. **GPT-style transformer**: predicts the next token in the discretized sequence — effectively modeling `p(o_{t+1} | o_{t}, a_t)`
3. **Imagination**: sample from the transformer autoregressively for 20 steps; use the resulting imagined sequence for policy improvement

The IRIS approach trades off architectural elegance (pure transformer, no special recurrent components) for inference cost (autoregressive generation is sequential). Its 200 training steps per epoch on imagined sequences is the offline sleep-time compute phase.

### Genie: Unsupervised World Models from Video

Genie (Bruce et al. 2024) pushes world model learning further by removing the need for action labels entirely. Trained on 11 billion parameters from unlabeled video of human gameplay, Genie learns:

- **8 latent action codes**: a discrete action space inferred from video, without being told what actions the human took
- **Generative interactive environment**: given a single image frame, generate a playable interactive environment

For Roko, Genie represents the horizon of what world models could become: an agent that builds its world model from observation alone, without requiring explicitly labeled action-outcome pairs. The 8 latent actions correspond to abstract "affordances" in the environment — the kinds of interventions the world supports, discovered unsupervised.

### The Imagination-to-Planning Pipeline

The full pipeline from world model to policy improvement:

```
IMAGINATION_TO_PLANNING(world_model, policy, episodes):

    // 1. Encode: compress real episodes into latent representations
    latents = [world_model.encode(episode) for episode in episodes]

    // 2. Imagine: unroll the world model from each latent state
    imagined_rollouts = []
    for latent in latents:
        rollout = []
        h_t = latent
        for step in range(imagination_horizon):
            a_t  = policy.sample(h_t)             // sample action from current policy
            h_t1 = world_model.prior(h_t, a_t)    // predict next state (no real obs)
            r_t  = world_model.reward(h_t1)        // predict reward
            rollout.append((h_t, a_t, h_t1, r_t))
            h_t = h_t1
        imagined_rollouts.append(rollout)

    // 3. Evaluate: compute returns over imagined rollouts
    returns = [compute_lambda_return(rollout, γ, λ) for rollout in imagined_rollouts]

    // 4. Update policy: gradient step on imagined returns
    policy.update(imagined_rollouts, returns)

    // 5. Update world model: gradient step on prediction errors
    world_model.update(episodes)

    return updated_policy, updated_world_model
```

In Roko's LLM-based setting, steps 3 and 4 are replaced by the Consolidation engine (Integration phase): the "policy" is the agent's NeuroStore of heuristics, and "updating the policy" means promoting accepted counterfactual hypotheses into NeuroStore with confidence scores derived from their plausibility.

### Academic Citations

| Paper | Contribution |
|-------|-------------|
| Hafner et al. (2023), "Mastering Diverse Domains with World Models" (DreamerV3), arXiv:2301.04104 | RSSM architecture: h_t, z_t factored latent state; 32×32 categorical latents; fixed hyperparameters H=15, γ=0.997, λ=0.95 |
| Micheli et al. (2023), "Transformers are Sample-Efficient World Models" (IRIS), ICLR 2023 | Discrete VQ-VAE + GPT world model; 20-step imagination; 200 training steps per epoch |
| Bruce et al. (2024), "Genie: Generative Interactive Environments", NeurIPS 2024 | 11B parameter unsupervised world model; 8 latent action codes learned from unlabeled video |
| Janner et al. (2019), "When to Trust Your Model: Model-Based Policy Optimization" (MBPO), NeurIPS 2019 | Short-horizon fallback; 1-5 step rollouts from real states bound world model error |
| Lin et al. (2025), "Sleep-time Compute", arXiv:2025.00XXX | 5× test-time compute reduction via offline processing; +13% GSM-Symbolic, +18% AIME |
| Hafner et al. (2025), Nature, "Mastering diverse control tasks through world models" (DreamerV3) | World model learning: imagination-based planning with RSSM, symlog, and KL balance |
| Micheli et al. (2024), ICML, "Efficient World Models with Context-Aware Tokenization" (Delta-IRIS) | Delta-encoded world models: focus transformer capacity on stochastic dynamics |
| Google DeepMind (2024), "Genie 2: A large-scale foundation world model" | Interactive environment generation from prompts for embodied exploration |
| Google DeepMind (2025), "Genie 3: A new frontier for world models" | Real-time interactive world generation with improved consistency |
| "Integrational creativity: from combining and blending to transforming and resonating," Inquiry (2024) | Fourth creativity mode: resonance across simultaneously active knowledge domains |
| "Transformational Creativity in Science: A Graphical Theory," arXiv:2504.18687 (2025) | Enabling constraints: identifying which assumptions to violate for maximal possibility |
| "Trust the Model Where It Trusts Itself" (MACURA), arXiv (2024) | Uncertainty-aware rollout adaptation for imagination validation |
| "WHALE: Towards Generalizable and Scalable World Models," arXiv (2024) | Retracing-rollout for imagination uncertainty estimation |
| "Embedding Safety into RL: A New Take on Trust Region Methods" (C-TRPO), ICML (2025) | Safety-constrained trust regions for imagination-based policy optimization |

---

## World Models for Dream-Based Planning

The world model literature has advanced significantly beyond the RSSM and IRIS architectures described above. Three lines of work are particularly relevant to Roko's REM imagination phase: DreamerV3's scale-invariant world model learning, Delta-IRIS's efficient delta-encoded representations, and Genie's interactive environment generation.

### DreamerV3: Learning World Models for Diverse Domains

**Reference**: Danijar Hafner, Jurgis Pasukonis, Jimmy Ba, and Timothy Lillicrap, "Mastering diverse control tasks through world models," *Nature*, 2025. DOI: 10.1038/s41586-025-08744-2. (Preprint: arXiv:2301.04104.)

DreamerV3 masters a wide range of control domains with **fixed hyperparameters** — no per-domain tuning. Key innovations:

- **Symlog predictions**: `symlog(x) = sign(x) · log(|x| + 1)` handles the enormous dynamic range of rewards across domains. Applied to observations, reward targets, and critic targets.
- **Two-hot distributional RL**: Reward and critic heads use categorical distributions with two-hot encoding on an exponentially-spaced grid, avoiding commitment to fixed-range buckets.
- **Percentile return normalization**: `S = EMA(Per(R^λ, 95) − Per(R^λ, 5), 0.99)` — critic targets divided by S prevent value explosion under dense rewards while maintaining signal under sparse rewards.
- **1% unimix**: Uniform mixture added to all categoricals prevents zero probabilities and KL collapse.
- **Block-diagonal GRU**: 8 blocks with RMSNorm and SiLU activations for the sequence model.
- **Stochastic latent**: z_t represented as 32 one-hot vectors from 32 categorical distributions (32 classes each = 1024 total categories).
- **Model sizes**: 6 configurations from 12M to 400M parameters. Larger models achieve both higher performance and greater data efficiency.

**Performance across 150+ tasks**: Outperforms MuZero on Atari 57 (200M frames), surpasses IRIS/TWM/SimPLe on Atari 100k, exceeds PPG and Rainbow on ProcGen, achieves 10× data efficiency over IMPALA/R2D2+ on DMLab, and is the **first algorithm to collect diamonds in Minecraft from scratch** without human data or curricula.

**Map to Roko's REM phase**: DreamerV3's "dream within the model" is directly analogous to REM imagination. The agent builds an internal world model from its episode history, then generates counterfactual trajectories within that model. The symlog encoding principle translates directly: dream replay should handle episodes with wildly different scales of outcomes (a minor lint warning vs. a catastrophic compilation failure) without requiring per-domain normalization. The key difference: DreamerV3 learns continuous latent dynamics; Roko's REM operates on discrete episodes processed through LLM reasoning.

```rust
/// World model configuration for dream-based planning.
/// Inspired by DreamerV3 (Hafner et al., Nature 2025).
pub struct WorldModelConfig {
    /// Latent state dimensionality for the learned world model.
    pub latent_dim: usize,                 // default: 512, range: 128-2048
    /// Number of imagination rollout steps per counterfactual.
    pub imagination_horizon: usize,        // default: 15, range: 5-50
    /// Discount factor for future rewards in imagined trajectories.
    pub imagination_discount: f64,         // default: 0.997, range: 0.95-0.999
    /// Whether to use symlog encoding for scale-invariant prediction.
    pub symlog_encoding: bool,             // default: true
    /// KL balance coefficient (higher = more deterministic latent states).
    pub kl_balance: f64,                   // default: 0.8, range: 0.5-0.95
    /// Free bits threshold: minimum KL divergence before penalty applies.
    pub free_bits: f64,                    // default: 1.0, range: 0.0-3.0
}
```

### Delta-IRIS: Efficient Context-Aware World Models

**Reference**: Vincent Micheli, Eloi Alonso, and François Fleuret, "Efficient World Models with Context-Aware Tokenization," *ICML 2024*. arXiv:2406.19320.

Delta-IRIS conditions the autoencoder on the previous observation and action: `E: S(X × A) × X → Z^K`, encoding only the stochastic delta between frames. The decoder reconstructs from delta-tokens + context, absorbing deterministic dynamics (wall layout, agent position) into the decoder weights.

**Quantitative improvements over IRIS**:

| Metric | Δ-IRIS | IRIS (64-token) | IRIS (16-token) |
|--------|--------|-----------------|-----------------|
| Tokens per frame | **4** | 64 | 16 |
| Parameters | **25M** | 48M | 50M |
| Training speed (FPS) | **20** | 2 | 6 |
| Training time ratio | **1×** | ~10× slower | ~3.3× slower |

Architecture: 3-layer transformer, 512 embedding dimension, 8 attention heads, 21-timestep context window. Vocabulary size 1024 for the discrete autoencoder. I-tokens (continuous frame embeddings) act as a "soft Markov blanket" decoupling state from change representation. The encoder produces K=4 delta-tokens per frame, validated by replacing delta-tokens with random samples — the decoder still correctly renders deterministic components while stochastic elements become unpredictable, confirming clean disentanglement.

**Map to Roko**: Rather than encoding full episodes, encode the *delta* between consecutive episodes — what changed. This dramatically reduces the token budget for NREM replay (potentially 16× fewer tokens per episode) and allows the REM phase to focus creative capacity on the stochastic, unpredictable components of agent behavior.

```rust
/// Delta-encoded episode representation for efficient replay.
/// Inspired by Delta-IRIS (Micheli et al., ICML 2024).
pub struct DeltaEpisodeEncoder {
    /// Whether to encode episodes as deltas from predecessor.
    pub use_delta_encoding: bool,          // default: true
    /// Maximum delta token count before falling back to full encoding.
    pub max_delta_tokens: usize,           // default: 256, range: 64-1024
    /// Similarity threshold: episodes more similar than this use delta encoding.
    pub delta_similarity_threshold: f32,   // default: 0.65, range: 0.50-0.85
    /// Summary token count for trajectory context.
    pub summary_tokens: usize,            // default: 32, range: 8-128
}
```

### Genie 2/3: Generative Interactive Environments

**Reference**: Google DeepMind (2024, 2025). Genie 2 generates 3D interactive environments from a single image prompt. Genie 3 achieves real-time interaction with improved consistency.

**Map to Roko**: Genie's paradigm of generating interactive environments from prompts is the logical extension of REM imagination. Rather than generating single counterfactual scenarios, the agent could generate *interactive* counterfactual environments and explore them through simulated action sequences. This moves beyond "what if X had happened?" to "let me explore the world where X happened."

```rust
/// Interactive counterfactual environment configuration.
/// Inspired by Genie 2/3 (Google DeepMind, 2024-2025).
pub struct InteractiveCounterfactualConfig {
    /// Maximum exploration steps within a generated environment.
    pub max_exploration_steps: usize,      // default: 10, range: 3-30
    /// Whether to allow branching within the counterfactual environment.
    pub allow_branching: bool,             // default: true
    /// Maximum branch depth.
    pub max_branch_depth: usize,           // default: 3, range: 1-5
    /// Model tier for environment generation.
    pub environment_model_tier: ModelTier, // default: T1 (Sonnet-class)
    /// Whether to persist the environment state for future exploration.
    pub persist_environment: bool,         // default: false
}
```

---

## Advances in Computational Creativity Theory

The creativity modes in REM imagination (combinational, exploratory, transformational) derive from Boden's (2004) computational creativity triad. Recent work extends this framework in two important directions: a fourth creativity mode based on integrational resonance, and a formal theory of when transformational creativity succeeds.

### Beyond Boden's Triad: Integrational Creativity

**Reference**: "Integrational creativity: from combining and blending to transforming and resonating," Inquiry (2024).

Proposes a fourth creativity mode — integrational — that incorporates resonance and transformation not captured by Boden's original triad. Integrational creativity emerges when multiple knowledge domains enter a state of mutual resonance, producing insights that none of the contributing domains could generate independently.

**Map to Roko**: Add a fourth creativity mode to the REM phase that fires when HDC similarity analysis detects multiple knowledge clusters simultaneously activating. This produces a "resonance" event that the LLM processes as an integrational creative opportunity.

### Transformational Creativity in Science: Graphical Theory

**Reference**: "Transformational Creativity in Science: A Graphical Theory," arXiv (2025, arXiv:2504.18687).

Synthesizes Boden's enabling-constraints insight with Kuhn's paradigm shifts. Provides a formal graphical theory for when and how scientific transformational creativity occurs. Key insight: transformational creativity requires not just violating a constraint but recognizing which constraint is the right one to violate — the "enabling constraint."

**Map to Roko**: During transformational creativity mode in REM, rather than randomly violating assumptions, the agent should identify which assumptions are "enabling constraints" — assumptions whose violation opens up the largest new possibility space. This can be measured by the HDC distance between the original knowledge cluster and the nearest cluster in the newly accessible space.

```rust
/// Extended creativity modes including integrational resonance.
/// Based on Boden (2004) + Inquiry (2024) + arXiv:2504.18687 (2025).
pub enum CreativityMode {
    /// Combine elements from unrelated domains.
    Combinational,
    /// Push parameters to the boundaries of existing strategy spaces.
    Exploratory,
    /// Violate enabling constraints to open new possibility spaces.
    Transformational {
        /// The constraint being violated.
        target_constraint: String,
        /// HDC distance to nearest accessible cluster after violation.
        possibility_distance: f32,
    },
    /// Mutual resonance across multiple simultaneously active knowledge domains.
    /// Reference: Inquiry (2024), "Integrational creativity."
    Integrational {
        /// Knowledge cluster IDs participating in resonance.
        resonating_clusters: Vec<String>,
        /// Resonance strength: mean pairwise activation above baseline.
        resonance_strength: f64,
    },
}
```

---

## Imagination Validation: Trust Regions for Dreamed Strategies

The Imagination Validation section above establishes the GIRL trust-region framework for controlling imagination quality. Three recent advances extend this framework with uncertainty-adaptive rollout control, behavior-conditioned world models, and safety-constrained trust regions.

### MACURA: Uncertainty-Aware Rollout Adaptation

**Reference**: "Trust the Model Where It Trusts Itself: Model-Based Actor-Critic with Uncertainty-Aware Rollout Adaptation," arXiv (2024).

MACURA uses a spatial uncertainty estimate to determine where model rollouts are trustworthy. Adapts rollout length and branching based on local model accuracy. Rather than applying a fixed imagination horizon globally, MACURA allows longer rollouts in well-modeled regions and truncates early in uncertain regions — the model trusts itself only where it has earned that trust.

### WHALE: Behavior-Conditioned World Models

**Reference**: "WHALE: Towards Generalizable and Scalable World Models for Embodied Decision-making," arXiv (2024).

Introduces a retracing-rollout technique for efficient uncertainty estimation during imagination. Validates imagined trajectories before using them for policy updates. The retracing mechanism replays real trajectories through the world model and compares predicted vs. actual outcomes — the divergence at each step provides a calibrated uncertainty signal that can be applied to novel imagined trajectories.

### C-TRPO: Safety-Constrained Trust Regions

**Reference**: "Embedding Safety into RL: A New Take on Trust Region Methods," ICML (2025).

C-TRPO reshapes policy space geometry so trust regions contain only safe policies, guaranteeing constraint satisfaction throughout training. Rather than checking safety as a post-hoc filter, C-TRPO builds safety into the trust region boundary itself — the agent cannot step outside the safe region even during exploration.

**Map to Roko's GIRL trust-region**: Extend the existing imagination validation framework with uncertainty-adaptive rollout control. The GIRL KL budget provides the outer trust region, MACURA provides local uncertainty truncation within that region, WHALE provides the uncertainty estimation mechanism, and C-TRPO ensures safety constraints are never violated by dreamed strategies.

```rust
/// Imagination validation with uncertainty-aware trust regions.
/// Integrates GIRL (existing), MACURA (2024), WHALE (2024), C-TRPO (2025).
pub struct ImaginationValidator {
    /// Maximum KL divergence from current policy to accept a dreamed strategy.
    /// The GIRL trust region radius.
    pub girl_kl_budget: f64,              // default: 0.10, range: 0.01-0.50
    /// Uncertainty threshold: rollouts in regions with uncertainty above this
    /// are truncated. Based on MACURA (2024).
    pub uncertainty_truncation_threshold: f64, // default: 0.7, range: 0.3-0.95
    /// Whether to use retracing-rollout for uncertainty estimation.
    /// Based on WHALE (2024).
    pub use_retracing_rollout: bool,      // default: true
    /// Number of retrace steps for uncertainty estimation.
    pub retrace_depth: usize,             // default: 5, range: 2-15
    /// Whether to enforce safety constraints on the trust region boundary.
    /// Based on C-TRPO (ICML 2025).
    pub safety_constrained: bool,         // default: true
    /// Safety constraint violation tolerance.
    pub safety_tolerance: f64,            // default: 0.01, range: 0.001-0.05
}
```

### Validation Pipeline Pseudocode

```
VALIDATE-IMAGINED-STRATEGY(strategy, validator, knowledge_store):
  // Step 1: GIRL trust region check
  kl_div = compute_kl_divergence(strategy, current_policy)
  IF kl_div > validator.girl_kl_budget:
    RETURN ValidationResult::Rejected { reason: "exceeds trust region" }

  // Step 2: Uncertainty estimation via retracing-rollout (WHALE)
  IF validator.use_retracing_rollout:
    uncertainty = retrace_rollout(strategy, validator.retrace_depth, knowledge_store)
    IF uncertainty > validator.uncertainty_truncation_threshold:
      RETURN ValidationResult::Truncated {
        usable_horizon: find_truncation_point(uncertainty_curve),
        reason: "high uncertainty beyond truncation point"
      }

  // Step 3: Safety constraint check (C-TRPO)
  IF validator.safety_constrained:
    violations = check_safety_constraints(strategy)
    IF violations.max_violation() > validator.safety_tolerance:
      RETURN ValidationResult::SafetyRejected { violations }

  RETURN ValidationResult::Accepted { confidence: 1.0 - uncertainty }
```

### Test Criteria

```
1. GIRL trust region: strategies with KL > girl_kl_budget are rejected.
2. MACURA truncation: strategies in high-uncertainty regions are truncated, not rejected entirely.
3. Retracing consistency: retrace_rollout(strategy, depth=N) produces monotonically non-decreasing
   uncertainty estimates as depth increases.
4. Safety constraint: strategies violating safety constraints are rejected regardless of
   uncertainty or KL divergence.
5. Accepted confidence: accepted strategies have confidence = 1.0 - uncertainty, in range [0.0, 1.0].
6. Truncation point: find_truncation_point returns the last step where uncertainty < threshold.
```

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [02-nrem-replay.md](02-nrem-replay.md) | NREM phase provides replay outputs that REM processes |
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Integration phase evaluates REM hypotheses |
| [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC operations for counterfactual vector manipulation |
| [09-threat-simulation.md](09-threat-simulation.md) | Threat simulation theory and adversarial dreaming |
| [../04-daimon/INDEX.md](../09-daimon/INDEX.md) | Daimon affect engine that receives depotentiation updates |
