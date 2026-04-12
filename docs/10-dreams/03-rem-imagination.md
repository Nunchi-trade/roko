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

## Cross-References

| Document | Relevance |
|----------|-----------|
| [02-nrem-replay.md](02-nrem-replay.md) | NREM phase provides replay outputs that REM processes |
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Integration phase evaluates REM hypotheses |
| [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC operations for counterfactual vector manipulation |
| [09-threat-simulation.md](09-threat-simulation.md) | Threat simulation theory and adversarial dreaming |
| [../04-daimon/INDEX.md](../09-daimon/INDEX.md) | Daimon affect engine that receives depotentiation updates |
