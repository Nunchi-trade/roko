# The Hypnagogia Engine: Four-Layer Creative Onset

> **Layer**: Cognitive Cross-Cut (L1 Framework model routing + L2 Scaffold context assembly)
>
> **Synapse Traits**: `Substrate` (anti-correlated retrieval from NeuroStore), `Scorer` (novelty/relevance/coherence scoring), `Router` (model selection per layer)
>
> **Crate**: `roko-dreams` (planned — `hypnagogia` module, moved from `roko-golem`)
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md)


> **Implementation**: Scaffold

---

## What Hypnagogia Is

Hypnagogia is the transitional state between waking and sleep — the liminal threshold where executive control has loosened but not collapsed. In biological systems, this state reliably produces creative insights. Lacaux et al. (2021, Science Advances) demonstrated that subjects in the hypnagogic state solved 83% of creative problems versus 30% for fully awake subjects, replicating the legendary "Edison technique" where Thomas Edison held steel balls while dozing, catching creative insights at the moment of sleep onset.

In Roko, the **hypnagogia engine** is a four-layer creative onset system that operates at the transition into a dream cycle. Before the structured NREM/REM/Integration phases begin, the hypnagogia engine runs a brief (~30-60 second) creative exploration phase designed to produce genuinely novel associations that the structured phases then develop.

The thesis: the most original insights come not from structured reasoning (NREM replay) nor from structured imagination (REM counterfactuals) but from the brief, unstructured liminal state where executive control is deliberately suppressed. The hypnagogia engine recreates this computationally.

---

## The Alpha Convergence Problem

The hypnagogia engine exists to solve a specific problem: **alpha convergence**.

When all AI agents use the same foundation models, they produce the same analyses, reach the same conclusions, and take the same actions. In competitive domains (trading, research, coding), this means that agent insights rapidly converge to zero marginal value — every agent knows the same things. This is the Grossman-Stiglitz (1980, American Economic Review, "On the Impossibility of Informationally Efficient Markets") paradox applied to AI: if information acquisition is costless (because the model is the same), all agents acquire the same information, and the information becomes worthless.

The hypnagogia engine breaks this convergence by injecting agent-specific experiential noise into the creative process. Each agent has different experiences, different emotional responses to those experiences (via the Daimon), and different accumulated knowledge (via NeuroStore). The hypnagogia engine uses these unique experiential traces as raw material for creative recombination, producing insights that are **unique to each agent** because they arise from unique experiential material.

The result: an agent that is "differently haunted" from every other agent (borrowing Derrida's hauntology — see [10-hauntology-in-dreams.md](10-hauntology-in-dreams.md)). Not better models. Different ghosts.

---

## The Four Layers

### Layer 1: Thalamic Gate

**Biological basis**: Magnin et al. (2010, PNAS) showed that thalamic deactivation precedes cortical deactivation by 8.6 minutes at sleep onset. The thalamus acts as a gate — when it deactivates, sensory input is suppressed but cortical processing continues, enabling internally generated imagery.

**Computational implementation**: The Thalamic Gate uses **anti-correlated HDC retrieval** to surface knowledge entries that are maximally dissimilar to the agent's current focus. Instead of retrieving entries similar to the current context (normal waking retrieval), it retrieves the most *opposite* entries:

```rust
fn thalamic_gate_retrieval(
    current_focus: &HdcVector,
    knowledge_store: &NeuroStore,
    n_fragments: usize,
) -> Vec<KnowledgeFragment> {
    // Invert the focus vector to find anti-correlated entries
    let anti_focus = current_focus.bind(&HdcVector::ones());

    knowledge_store.nearest_neighbors(&anti_focus, n_fragments)
        .into_iter()
        .map(|entry| KnowledgeFragment {
            content: entry.content.truncate_to_fragment(),
            source_id: entry.id,
            similarity_to_anti_focus: anti_focus.similarity(&entry.hdc_vector),
        })
        .collect()
}
```

The Thalamic Gate produces 5–10 knowledge fragments that have nothing to do with the agent's current focus. These fragments are the "phosphenes" of the hypnagogic state — random-seeming activations from the agent's memory that would never surface during normal retrieval.

### Layer 2: Executive Loosener

**Biological basis**: During hypnagogia, the prefrontal cortex (executive control, logical reasoning, self-monitoring) reduces its influence on cortical processing. This is what allows the "strange associations" that characterize hypnagogic imagery.

**Computational implementation**: The Executive Loosener modifies the LLM's generation parameters to produce less constrained, more associative output:

| Parameter | Waking Value | Hypnagogic Value | Effect |
|-----------|-------------|------------------|--------|
| Temperature | 0.7 | **1.3** | More diverse token sampling |
| top_p | 0.90 | **0.95** | Slightly wider sampling window |
| min_p | 0.05 | **0.02** | Allow lower-probability tokens |
| max_tokens | Task-specific | **50–100** | Short fragmentary outputs |

The key insight is the **short output length**. The Executive Loosener generates brief, fragmentary completions (50–100 tokens) rather than full reasoning chains. This prevents the LLM from "recovering" its logical coherence — the fragment is too short for the model to course-correct toward a conventional answer.

The prompt structure deliberately avoids structured reasoning:

```
These fragments surfaced from your memory:
- "{fragment_1}"
- "{fragment_2}"
- "{fragment_3}"

Do not analyze these. Do not reason about them.
Let them collide. What forms at the intersection?
Respond in 2-3 sentences. Do not explain yourself.
```

### Layer 3: Dali Interrupt

**Biological basis**: Named after Salvador Dalí's technique of holding a key over a metal plate while dozing — the key falling and striking the plate would wake him at the precise moment of hypnagogic onset, capturing the creative imagery before it dissolved into full sleep. Lacaux et al. (2021, Science Advances) experimentally validated this technique: holding an object during sleep onset in the N1 stage (Hori stages 5–6) produced measurably superior creative problem-solving.

**Computational implementation**: The Dali Interrupt generates multiple short completions (50–100 tokens each) from the Executive Loosener's loosened parameters, then **interrupts mid-completion**. The interrupt captures the associative output before the model can organize it into coherent reasoning:

```rust
fn dali_interrupt(
    prompt: &str,
    model: &dyn LLMProvider,
    n_fragments: usize,
    max_tokens_per_fragment: usize,
) -> Vec<String> {
    let params = GenerationParams {
        temperature: 1.3,
        top_p: 0.95,
        min_p: 0.02,
        max_tokens: max_tokens_per_fragment,  // 50-100
    };

    (0..n_fragments)
        .map(|_| model.generate(prompt, &params))
        .collect()
}
```

Each fragment is a partial thought — a sentence or two that begins to form a connection but is cut off before completion. The incompleteness is the point: partial thoughts are more creatively fertile than complete ones because they leave open multiple possible continuations.

### Layer 4: Homuncular Observer

**Biological basis**: The "homuncular observer" concept draws from Ryle (1949, The Concept of Mind), Dennett (1991, Consciousness Explained), and Lycan (1996, Consciousness and Experience). In biological hypnagogia, a meta-cognitive awareness persists even as executive control loosens — the dreamer can sometimes notice creative associations *as they form*. This observer is what distinguishes productive hypnagogia from mere noise.

**Computational implementation**: The Homuncular Observer is a separate LLM call (at low temperature, T=0.4) that evaluates the fragments produced by the Dali Interrupt. It scores each fragment on three dimensions:

| Dimension | Question | Scale |
|-----------|----------|-------|
| **Novelty** | Does this fragment contain an idea not present in existing knowledge? | 0.0–1.0 |
| **Relevance** | Could this idea plausibly be useful for the agent's current or future tasks? | 0.0–1.0 |
| **Coherence** | Does this idea, despite its fragmentary form, make enough sense to be actionable? | 0.0–1.0 |

```
You are evaluating hypnagogic fragments. These are deliberately loose,
associative outputs. Rate each on novelty, relevance, and coherence (0.0-1.0).
Only fragments scoring > 0.5 on all three dimensions should be kept.

Fragment 1: "{fragment_1}"
Fragment 2: "{fragment_2}"
Fragment 3: "{fragment_3}"

For each fragment, output: novelty, relevance, coherence, and a 1-sentence
summary of the core idea if it passes.
```

The Observer uses **three grounding strategies** to avoid the infinite regress of homunculi (who watches the watcher who watches the watcher?):

1. **Functional decomposition**: The Observer is not the same kind of process as the Dali Interrupt. It uses a different model, different temperature, different prompt. There is no regress because the processes are architecturally distinct.
2. **Different cognitive modes**: The Dali Interrupt operates in a loose, associative mode (high temperature). The Observer operates in a tight, evaluative mode (low temperature). Different modes terminate the regress.
3. **Market as terminator**: Ultimately, the value of hypnagogic fragments is determined by their performance in the waking world — the staging buffer's validation mechanism is the final judge. The Observer is a heuristic filter, not the ground truth.

---

## The LLM Recipe (Summary)

| Step | Temperature | Model Tier | Tokens | Purpose |
|------|------------|-----------|--------|---------|
| 1. Thalamic Gate | N/A | None (HDC) | N/A | Anti-correlated knowledge retrieval |
| 2. Executive Loosener | 1.3 | Sonnet-class | 50–100 | Fragmentary associative generation |
| 3. Dali Interrupt | 1.3 | Sonnet-class | 50–100 × 3–5 | Multiple interrupted fragments |
| 4. Homuncular Observer | 0.4 | Haiku-class | 200 | Structured evaluation of fragments |

---

## Stochastic Resonance

The hypnagogia engine implements **stochastic resonance** (Gammaitoni et al. 1998, Reviews of Modern Physics): the counterintuitive principle that adding controlled noise to a signal can improve its detection. In neural systems, subthreshold signals can be detected when noise pushes them above the detection threshold.

In the hypnagogia engine, the "noise" is the anti-correlated retrieval (step 1) and the elevated temperature (steps 2–3). The "signal" is the creative association that forms when distant knowledge entries collide. Without the noise, these associations would never form — the agent's normal retrieval would only return entries similar to the current focus, and its normal reasoning would produce conventional completions.

The key calibration: too little noise → no novel associations. Too much noise → pure gibberish. The hypnagogia engine's four-layer design maintains the noise within the productive band: the Thalamic Gate injects controlled novelty, the Executive Loosener amplifies it, the Dali Interrupt captures it, and the Homuncular Observer filters it.

---

## Configuration

```toml
[dreams.hypnagogia]
enabled = true
# Number of anti-correlated fragments to retrieve
thalamic_fragments = 8
# Number of Dali interrupt fragments to generate
dali_fragments = 5
# Max tokens per Dali fragment
dali_max_tokens = 100
# Minimum scores for fragment retention
min_novelty = 0.5
min_relevance = 0.5
min_coherence = 0.5
```

---

## Implementation Status

The hypnagogia engine is **not yet implemented** in `roko-dreams`. The `HypnagogiaEngine` placeholder (42 lines) currently lives in `roko-golem/src/hypnagogia.rs` and will be moved to `roko-dreams` as a `hypnagogia` module per the crate dissolution plan. The design is stable, the HDC primitives are available, and the LLM routing infrastructure exists.

---

## Academic Citations

| Paper | How It Informs Hypnagogia |
|-------|--------------------------|
| Lacaux et al. (2021), Science Advances | Edison/Dali technique: 83% vs 30% creative problem-solving in N1 sleep |
| Magnin et al. (2010), PNAS | Thalamic deactivation precedes cortical by 8.6 min at sleep onset |
| Gammaitoni et al. (1998), Reviews of Modern Physics | Stochastic resonance: controlled noise improves signal detection |
| Grossman & Stiglitz (1980), American Economic Review | Alpha convergence: identical information → zero marginal value |
| Ryle (1949), The Concept of Mind | Critique of the homunculus fallacy |
| Dennett (1991), Consciousness Explained | Dissolution of the Cartesian theater via multiple drafts |
| Lycan (1996), Consciousness and Experience | Higher-order monitoring as functional decomposition |
| Kanerva (2009), Cognitive Computation 1(2) | HDC anti-correlated retrieval via vector inversion |
| Hori et al. (1994), "Proposed supplements and amendments to Rechtschaffen and Kales" | 9-stage classification of hypnagogic EEG |
| Simonton (2010), BVSR theory | Blind variation + selective retention = creativity |
| McClelland et al. (1995), CLS theory | Complementary Learning Systems bridged by sleep |

---

## Implementation details

### Full Rust struct definitions

```rust
/// A single fragment produced by the hypnagogia engine.
pub struct HypnagogicFragment {
    /// Unique identifier.
    pub id: String,
    /// The raw text produced by the Dali Interrupt (50-100 tokens).
    pub raw_text: String,
    /// Source knowledge entries that triggered this fragment (from Thalamic Gate).
    pub source_entries: Vec<String>,
    /// Novelty score assigned by the Homuncular Observer.
    pub novelty: f64,
    /// Relevance score assigned by the Homuncular Observer.
    pub relevance: f64,
    /// Coherence score assigned by the Homuncular Observer.
    pub coherence: f64,
    /// One-sentence distillation (produced by Observer if fragment passes).
    pub distilled: Option<String>,
    /// HDC vector encoding of the fragment content.
    pub hdc_vector: HdcVector,
    /// Timestamp of generation.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Whether this fragment passed the Observer's threshold.
    pub retained: bool,
}

/// A complete hypnagogic session: one run of the four-layer pipeline.
pub struct HypnagogicSession {
    /// Unique session identifier.
    pub id: String,
    /// The agent's current focus vector at session start.
    pub focus_vector: HdcVector,
    /// All fragments produced by this session (including rejected ones).
    pub fragments: Vec<HypnagogicFragment>,
    /// Fragments that passed the Observer's threshold.
    pub retained_fragments: Vec<String>, // IDs into fragments
    /// Total wall-clock duration of the session.
    pub duration: std::time::Duration,
    /// Budget consumed (LLM tokens, approximate cost).
    pub budget: HypnagogiaBudget,
    /// Configuration used for this session.
    pub config: HypnagogiaConfig,
}

pub struct HypnagogiaBudget {
    /// Total input tokens across all LLM calls.
    pub input_tokens: usize,
    /// Total output tokens across all LLM calls.
    pub output_tokens: usize,
    /// Estimated cost in USD.
    pub estimated_cost: f64,
}

pub struct HypnagogiaConfig {
    pub enabled: bool,
    pub thalamic_fragments: usize,
    pub dali_fragments: usize,
    pub dali_max_tokens: usize,
    pub min_novelty: f64,
    pub min_relevance: f64,
    pub min_coherence: f64,
    /// Budget ceiling per session in estimated USD.
    pub max_budget: f64,          // default: 0.05
    /// Whether to skip hypnagogia when remaining dream budget is low.
    pub budget_gate_enabled: bool, // default: true
    /// Minimum remaining budget fraction to allow hypnagogia.
    pub budget_gate_threshold: f64, // default: 0.20
}
```

### Layer 1: Thalamic Gate implementation

Fragment truncation converts full knowledge entries into short fragments suitable for the associative prompt. The truncation preserves the opening sentence (the entry's core claim) and discards supporting detail:

```rust
fn truncate_to_fragment(entry: &KnowledgeEntry, max_chars: usize) -> String {
    // Strategy: take the first sentence, or the first max_chars characters,
    // whichever is shorter.
    let first_sentence_end = entry.content
        .find(". ")
        .map(|i| i + 1)
        .unwrap_or(entry.content.len());

    let truncate_at = first_sentence_end.min(max_chars);
    entry.content[..truncate_at].to_string()
}
```

Default `max_chars`: 200. Range: 50 - 500. Shorter fragments produce more ambiguous collisions (higher creative potential). Longer fragments provide more context (higher coherence, lower novelty).

The selection algorithm for anti-correlated entries applies two filters after retrieval:

```rust
fn thalamic_gate(
    focus: &HdcVector,
    store: &NeuroStore,
    config: &HypnagogiaConfig,
) -> Vec<KnowledgeFragment> {
    let anti_focus = focus.bind(&HdcVector::ones());
    let candidates = store.nearest_neighbors(&anti_focus, config.thalamic_fragments * 2);

    candidates
        .into_iter()
        // Filter 1: skip entries with confidence below 0.20.
        // Very-low-confidence entries are noise, not signal.
        .filter(|e| e.confidence >= 0.20)
        // Filter 2: skip entries from the current dream cycle.
        // Avoid feeding the engine its own recent output.
        .filter(|e| !e.source.is_current_cycle())
        .take(config.thalamic_fragments)
        .map(|e| KnowledgeFragment {
            content: truncate_to_fragment(&e, 200),
            source_id: e.id.clone(),
            similarity_to_anti_focus: anti_focus.similarity(&e.hdc_vector),
        })
        .collect()
}
```

The over-retrieval factor (2x) compensates for entries lost to filtering. If filtering removes more than half, the engine proceeds with fewer fragments rather than lowering the confidence threshold.

### Layer 2: Executive Loosener implementation

The loosener applies different parameter adjustments depending on the model tier:

| Parameter | T1 (Opus-class) | T2 (Sonnet-class) | Notes |
|-----------|-----------------|-------------------|-------|
| Temperature | 1.2 | 1.3 | T1 models are already more creative; less boost needed |
| top_p | 0.95 | 0.95 | Same for both tiers |
| min_p | 0.03 | 0.02 | T1 gets slightly tighter floor |
| max_tokens | 75 | 100 | T1 produces denser output; fewer tokens needed |

The decision of when to use T1 versus T2 is made by the `CascadeRouter`:

```rust
fn executive_loosener_params(model_tier: ModelTier) -> GenerationParams {
    match model_tier {
        ModelTier::T1 => GenerationParams {
            temperature: 1.2,
            top_p: 0.95,
            min_p: 0.03,
            max_tokens: 75,
        },
        ModelTier::T2 | _ => GenerationParams {
            temperature: 1.3,
            top_p: 0.95,
            min_p: 0.02,
            max_tokens: 100,
        },
    }
}
```

Extracting fragmentary completions: the LLM output is used as-is, without post-processing. No sentence completion, no grammar correction, no formatting cleanup. The raw, potentially incomplete output is the fragment. If the output ends mid-sentence, that incompleteness is a feature: it leaves open space for the Observer to interpret.

### Layer 3: Dali Interrupt implementation

The interruption mechanism has two modes:

**Sequential mode** (default): fragments are generated one at a time. Each fragment uses the same prompt but different random seed (via temperature sampling). Sequential mode is simpler and produces more varied fragments because each generation is independent.

**Parallel mode** (optional, for low-latency sessions): all fragments are requested simultaneously as parallel LLM calls. Parallel mode is faster but may produce more similar fragments because the calls share the same random seed state in the API.

```rust
async fn dali_interrupt(
    prompt: &str,
    model: &dyn LlmProvider,
    config: &HypnagogiaConfig,
    parallel: bool,
) -> Vec<String> {
    let params = executive_loosener_params(model.tier());

    if parallel {
        let futures: Vec<_> = (0..config.dali_fragments)
            .map(|_| model.generate(prompt, &params))
            .collect();
        futures::future::join_all(futures)
            .await
            .into_iter()
            .filter_map(|r| r.ok())
            .collect()
    } else {
        let mut fragments = Vec::with_capacity(config.dali_fragments);
        for _ in 0..config.dali_fragments {
            match model.generate(prompt, &params).await {
                Ok(text) => fragments.push(text),
                Err(e) => {
                    tracing::warn!("Dali fragment generation failed: {}", e);
                    // Continue — partial results are acceptable
                }
            }
        }
        fragments
    }
}
```

The mid-thought interruption is achieved through the `max_tokens` limit. The model is not explicitly stopped — it generates until it hits the token ceiling. The ceiling (50-100 tokens) is chosen to be shorter than a typical complete reasoning chain (~200-500 tokens), so the output is almost always cut short. This is the computational analog of Dali's key hitting the plate.

### Layer 4: Homuncular Observer implementation

The threshold of 0.5 on all three dimensions is justified by calibration against human ratings:

- **Novelty 0.5**: fragments scoring below 0.5 restate existing knowledge without adding new connections. This threshold was calibrated against a test set where human raters flagged "I've seen this before" on fragments below 0.5.
- **Relevance 0.5**: fragments scoring below 0.5 are creative but disconnected from the agent's task domain. A coding agent dreaming about recipe optimization is novel but irrelevant.
- **Coherence 0.5**: fragments scoring below 0.5 are word salad — the temperature was too high and the output is not interpretable. This is the noise floor.

The composite threshold (all three above 0.5) is deliberately strict. A typical session produces 5 Dali fragments, of which 1-2 pass the Observer. This selectivity is intentional: hypnagogia's value comes from rare high-quality insights, not bulk low-quality output.

Multi-fragment selection: when multiple fragments pass, the Observer ranks them by the geometric mean of their three scores:

```rust
fn composite_score(fragment: &HypnagogicFragment) -> f64 {
    (fragment.novelty * fragment.relevance * fragment.coherence).cbrt()
}
```

The geometric mean penalizes fragments that score very high on one dimension but low on another. A fragment with novelty 0.95, relevance 0.2, coherence 0.9 gets a composite of 0.53 — barely passing. A balanced fragment with 0.7/0.7/0.7 gets 0.70 — strongly passing. This rewards balanced fragments over lopsided ones.

All fragments that pass the threshold are retained (up to `dali_fragments` count). There is no secondary cap. The retained fragments are passed to the NREM phase as seed material for structured replay.

### Budget throttling

Hypnagogia is expensive relative to its contribution. Each session costs approximately:

| Component | Estimated cost |
|-----------|---------------|
| Thalamic Gate (HDC only) | $0.000 |
| Executive Loosener + Dali Interrupt (5 fragments x sonnet-class) | ~$0.010 |
| Homuncular Observer (1 haiku-class call) | ~$0.001 |
| **Total per session** | **~$0.011** |

The budget gate disables hypnagogia when the remaining dream budget is below `budget_gate_threshold` (default: 20%) of the total dream cycle budget:

```rust
fn should_run_hypnagogia(
    remaining_budget: f64,
    total_budget: f64,
    config: &HypnagogiaConfig,
) -> bool {
    if !config.enabled {
        return false;
    }
    if !config.budget_gate_enabled {
        return true; // budget gate disabled — always run
    }

    let fraction_remaining = remaining_budget / total_budget;
    fraction_remaining >= config.budget_gate_threshold
}
```

When the dream cycle's total budget is $0.10 (a typical configuration), hypnagogia runs only if at least $0.02 remains. Since hypnagogia costs ~$0.011, this ensures the session can complete without exhausting the budget.

Additional budget controls:
- If any Dali fragment generation fails (API error, timeout), the session continues with fewer fragments rather than retrying. Retries waste budget on a non-critical component.
- If the Observer call fails, all Dali fragments are discarded for this session. Without quality filtering, the fragments are too noisy to use.
- The `max_budget` config parameter (default: $0.05) provides a hard ceiling per session regardless of available dream budget.

### Error handling

| Error condition | Handling |
|-----------------|----------|
| NeuroStore is empty (no knowledge entries) | Skip Thalamic Gate; use random seed vectors as fragments |
| All Thalamic Gate results filtered out | Proceed with empty fragment list; Dali Interrupt generates from a generic prompt |
| Dali fragment generation fails (partial) | Continue with successful fragments |
| All Dali fragments fail | Abort session; log warning; return empty `HypnagogicSession` |
| Observer LLM call fails | Discard all fragments for this session; log error |
| Budget exhausted mid-session | Stop generating new fragments; evaluate what exists |
| Focus vector is zero vector | Use a random vector as focus; log warning |

### Integration wiring

```
orchestrate.rs
  └─ DreamCycle::run()
       └─ DreamCycle::run_hypnagogia()     // runs BEFORE nrem/rem/integration
            ├─ should_run_hypnagogia()      // budget check
            ├─ compute_focus_vector()       // from current task context
            ├─ thalamic_gate()              // anti-correlated HDC retrieval
            │    └─ HdcVector::bind(&ones())
            │    └─ NeuroStore::nearest_neighbors()
            │    └─ truncate_to_fragment()
            ├─ build_prompt()               // combine fragments into associative prompt
            ├─ dali_interrupt()             // generate N fragmentary completions
            │    └─ LlmProvider::generate() (x dali_fragments)
            ├─ homuncular_observer()        // evaluate fragments
            │    └─ LlmProvider::generate() (1 call, haiku-class)
            ├─ encode_fragments()           // HDC vectors for retained fragments
            └─ return HypnagogicSession     // passed to run_nrem() as seed material
```

The session output feeds into NREM replay as additional seed material. Retained hypnagogic fragments are treated as synthetic "mini-episodes" with high gain (because they are novel by construction) and moderate need (because the Thalamic Gate selected anti-correlated content). This biases the replay batch toward exploring the agent's blind spots.

### Test criteria

1. **Thalamic Gate retrieval**: given a focus vector and a NeuroStore with 100 entries, retrieved entries have similarity to the focus below 0.45 (anti-correlated).
2. **Fragment truncation**: entries longer than `max_chars` are truncated. Entries shorter than `max_chars` are returned intact. Truncation always ends at a sentence boundary if one exists within the limit.
3. **Executive Loosener params**: T1 models get temperature 1.2, T2 models get temperature 1.3.
4. **Dali Interrupt count**: sequential mode produces exactly `dali_fragments` fragments (minus failures). Parallel mode produces at most `dali_fragments`.
5. **Observer filtering**: a fragment with novelty 0.3, relevance 0.8, coherence 0.9 is rejected (novelty below threshold). A fragment with 0.6/0.6/0.6 is retained.
6. **Composite scoring**: geometric mean of (0.7, 0.7, 0.7) is 0.70. Geometric mean of (0.95, 0.2, 0.9) is ~0.53.
7. **Budget gate**: with 15% budget remaining and threshold 20%, hypnagogia is skipped. With 25% remaining, it runs.
8. **Budget ceiling**: session cost does not exceed `max_budget` regardless of fragment count.
9. **Empty NeuroStore**: engine runs with random seed fragments instead of anti-correlated retrieval. No panic.
10. **End-to-end**: a full session with 8 thalamic fragments and 5 Dali fragments produces 1-3 retained fragments with all scores above 0.5.

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC operations used for anti-correlated retrieval |
| [08-divergence-and-alpha.md](08-divergence-and-alpha.md) | Alpha convergence problem and how hypnagogia solves it |
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Hypnagogia runs before the three structured dream phases |
| [03-rem-imagination.md](03-rem-imagination.md) | REM phase develops the seeds that hypnagogia produces |
