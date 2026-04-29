# Hypnagogia and Creativity

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How creative fragment generation emerges as a Pipeline Graph with anti-correlation, novelty scoring, and temperature control as composable Cells.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, HDC fingerprints, demurrage, Kind), [02-CELL](../../unified/02-CELL.md) (Store protocol, Compose protocol, Score protocol, Verify protocol, Functor pattern), [03-GRAPH](../../unified/03-GRAPH.md) (Pipeline pattern, Graph), [06-MEMORY](../../unified/06-MEMORY.md) (Store, HDC retrieval, dream consolidation, staging)

---

## 1. The Creative Search Problem

Standard agent loops converge. SENSE retrieves similar Signals, ASSESS scores by expected value, COMPOSE fits a budget, ACT picks the highest-probability action. Every step selects for familiarity. After fifty episodes, an agent's search cone narrows to a local optimum it can never escape through exploitation alone.

Hypnagogia is *structured divergence*. The biological precedent is the hypnagogic state -- the transition between wakefulness and sleep where associative constraints relax, novel combinations emerge, and an internal observer filters the result before it enters memory. Edison, Dali, and Horowitz's TDI experiments (2023, 43% creativity boost over controls, 90% cue incorporation rate) all exploit the same mechanism: loosen constraints, generate fragments, observe, keep the useful ones.

The redesign maps these four operations onto the unified primitives. What was a monolithic `HypnagogiaEngine` struct becomes a Pipeline Graph of four Cells. What was bespoke "resonance scoring" becomes a Store query with an inverted vector. What was a hardcoded `retention_floor` becomes a Verify Cell with a triple-axis threshold. The result is composable: hypnagogia becomes a Functor that can augment any Compose call, not just offline dream cycles.

---

## 2. The Four Cells

The hypnagogia pipeline is four Cells in sequence. Each Cell conforms to one of the 9 protocols, receives Signals, and emits Signals. The Pipeline pattern (see [03-GRAPH](../../unified/03-GRAPH.md)) guarantees that output of Cell N feeds input of Cell N+1, with type-checking on every edge.

```
ThalamicGate --> ExecutiveLoosener --> DaliInterrupt --> HomuncularObserver
(Store+Score)     (Compose)            (React)          (Verify)
```

### 2.1 ThalamicGate Cell (Store + Score protocols)

The thalamic gate retrieves raw material for creative recombination. In neuroscience, the thalamus gates sensory input during sleep onset, allowing normally-filtered associations through. In Roko, this is a Store query with two distinctive properties:

1. **Anti-correlated retrieval.** Instead of `query_similar(v, k)`, the gate queries `query_similar(invert(v), 2k)` -- retrieving Signals whose HDC fingerprints are *anti-correlated* with the current focus. This is the same Store operation the Memory Cell uses for normal retrieval, but with the query vector bit-flipped. No new mechanism needed.

2. **2x over-retrieval with confidence filtering.** Retrieve twice the target count, then filter to `confidence >= 0.20`. The over-retrieval ensures the anti-correlated pool is large enough to contain useful associations; the confidence floor prevents pure noise.

```rust
/// ThalamicGate: Store + Score Cell.
///
/// Retrieves anti-correlated Signals from Store, then scores them
/// by stochastic resonance (HDC distance as proxy for associative
/// surprise). The output is an over-sampled set of "raw material"
/// for the downstream Compose Cell to recombine.
struct ThalamicGateCell {
    /// How many Signals to retrieve (before filtering).
    over_retrieval_factor: usize,     // default: 2
    /// Minimum confidence to retain after retrieval.
    confidence_floor: f64,            // default: 0.20
    /// Maximum content length per retrieved Signal (truncation).
    max_content_chars: usize,         // default: 200
    /// Target number of Signals to emit.
    target_k: usize,                  // default: 12
}

impl Cell for ThalamicGateCell {
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Store, ProtocolId::Score]
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Extract the focus vector from the input (the current task/goal context).
        let focus = extract_focus_vector(&input)?;

        // Anti-correlation: invert the query vector.
        // In HDC, inversion is bitwise NOT -- every bit flipped.
        // query_similar with an inverted vector returns Signals
        // whose content is *dissimilar* to the focus.
        let anti_focus = focus.invert();

        // Over-retrieve: 2x the target count.
        let candidates = ctx.store.query_similar(
            &anti_focus,
            self.target_k * self.over_retrieval_factor,
        ).await?;

        // Filter by confidence floor, truncate content, cap at target_k.
        let gated: Vec<Signal> = candidates
            .into_iter()
            .filter(|s| s.confidence() >= self.confidence_floor)
            .map(|s| s.with_content_truncated(self.max_content_chars))
            .take(self.target_k)
            .collect();

        // Score each by "associative surprise": how far from the focus
        // vector. Higher distance = more surprising = more creative potential.
        let scored: Vec<Signal> = gated
            .into_iter()
            .map(|s| {
                let distance = 1.0 - s.hdc_similarity(&focus);
                s.with_metadata("hypnagogia.surprise", distance)
            })
            .collect();

        Ok(scored)
    }
}
```

**Why anti-correlation works.** Normal retrieval (`query_similar(v, k)`) returns the k nearest neighbors -- useful for exploitation. Anti-correlated retrieval (`query_similar(invert(v), k)`) returns Signals in the *opposite* region of the HDC space. These are semantically distant from the current task but still above the confidence floor, meaning they are validated knowledge from unrelated domains. The juxtaposition of task-irrelevant-but-valid knowledge with the current problem context is exactly what produces creative associations.

**Existing codebase mapping.** The current `thalamic_gate` method in `crates/roko-dreams/src/hypnagogia.rs` does confidence filtering (`signal.confidence >= self.gate.relevance_floor`) but uses a hash-based `resonance_score` instead of true anti-correlated HDC retrieval. The redesign replaces the hash heuristic with the Store's native `query_similar` on an inverted vector -- a single-line change at the call site, zero new infrastructure.

### 2.2 ExecutiveLoosener Cell (Compose protocol)

The executive loosener is a Compose Cell that recombines the thalamic material using elevated LLM temperature. In the biological metaphor, prefrontal executive control relaxes during sleep onset, allowing normally-suppressed associations to form.

In unified terms: this is the same Compose protocol used by the 9-layer system prompt builder, but with different Rack parameters. The "loosening" is just temperature, top_p, and token budget knobs set to favor divergent generation.

```rust
/// ExecutiveLoosener: Compose Cell with elevated temperature.
///
/// Takes anti-correlated Signals from ThalamicGate and asks an LLM
/// to produce short associative fragments at high temperature.
/// This is a standard Compose call with non-standard Rack parameters.
struct ExecutiveLoosenerCell {
    /// LLM temperature for fragment generation.
    temperature: f64,                // default: 1.3
    /// Nucleus sampling threshold.
    top_p: f64,                      // default: 0.95
    /// Minimum probability floor (filters degenerate tokens).
    min_p: f64,                      // default: 0.02
    /// Token budget per fragment.
    max_tokens_per_fragment: usize,  // default: 50..100
    /// How many fragments to generate per session.
    fragment_count: usize,           // default: 5
}

impl ComposeProtocol for ExecutiveLoosenerCell {
    async fn compose(
        &self,
        bids: Vec<ComposeBid>,
        budget: &ComposeBudget,
        ctx: &ComposeContext,
    ) -> Result<ComposeResult> {
        // Build a prompt from the anti-correlated material.
        // Each bid carries a Signal from ThalamicGate.
        let material: Vec<&str> = bids.iter()
            .map(|b| b.section.content.as_str())
            .collect();

        let system = format!(
            "You are generating creative associations. \
             Given these knowledge fragments from distant domains, \
             produce {} short (50-100 token) associative leaps. \
             Each should connect two or more fragments in a surprising way. \
             Do not explain. Just associate.",
            self.fragment_count,
        );

        // The Rack parameters are the key differentiator.
        // Standard Compose uses T=0.3-0.7. Hypnagogia uses T=1.3.
        let rack = ComposeRack {
            temperature: self.temperature,
            top_p: self.top_p,
            min_p: self.min_p,
            max_tokens: self.max_tokens_per_fragment * self.fragment_count,
            ..ComposeRack::default()
        };

        let fragments = ctx.llm.generate(&system, &material, &rack).await?;

        // Each fragment becomes a Signal with Kind::DreamFragment.
        let signals: Vec<Signal> = fragments
            .into_iter()
            .map(|text| Signal::new(Kind::DreamFragment, text)
                .with_metadata("hypnagogia.temperature", self.temperature)
                .with_metadata("hypnagogia.stage", "loosened")
                .with_balance(0.5)) // half initial balance: must earn survival
            .collect();

        Ok(ComposeResult {
            output: signals,
            token_cost: rack.max_tokens as u32,
            ..Default::default()
        })
    }
}
```

**Temperature as Rack parameter.** The temperature curve from the source material (sigmoid alpha-to-theta transition with 0.1 oscillation amplitude) becomes a Rack parameter that the outer Loop tunes. Early in a hypnagogia session, temperature starts low (alpha state, T~0.7); as the session progresses, it rises through a sigmoid to T~1.3 (theta state). The Loop Cell (section 5) manages this progression.

```rust
/// Sigmoid temperature curve with micro-oscillations.
///
/// Models the alpha-to-theta EEG transition during sleep onset.
/// progress = 0.0 is wide-awake alpha; progress = 1.0 is deep theta.
fn temperature_at(progress: f64, config: &TemperatureCurve) -> f64 {
    let logistic = 1.0 / (1.0
        + (-(progress - config.transition_midpoint) * config.steepness).exp());
    let base = config.alpha_temp
        + (config.theta_temp - config.alpha_temp) * logistic;
    if config.oscillations {
        base + config.oscillation_amplitude
            * (progress * std::f64::consts::TAU * 3.0).sin()
    } else {
        base
    }
}

struct TemperatureCurve {
    alpha_temp: f64,              // 0.7 (waking)
    theta_temp: f64,              // 1.3 (sleep onset)
    transition_midpoint: f64,     // 0.5
    steepness: f64,               // 10.0
    oscillations: bool,           // true
    oscillation_amplitude: f64,   // 0.1
}
```

### 2.3 DaliInterrupt Cell (React protocol)

The Dali interrupt breaks fixation. Dali held a key over a metal plate while dozing; when he fell asleep, the key dropped, the clang woke him, and he captured the last pre-sleep image. The computational analog: truncate fragment generation mid-sequence, producing incomplete thoughts that the observer must complete by bridging domains.

This is a React Cell. It subscribes to the fragment stream from the ExecutiveLoosener, interrupts fragments at random positions, and emits the interrupted prefixes as Signals. The React protocol is the right fit: React Cells respond to observed Signals with new Signals, which is exactly what interruption does.

```rust
/// DaliInterrupt: React Cell that fragments associations.
///
/// Operates in two modes:
/// - Sequential: interrupt every Nth fragment at a random position.
/// - Parallel: interrupt all fragments simultaneously, keep prefixes.
///
/// The interrupted fragments are deliberately incomplete -- they
/// force the HomuncularObserver (and eventually the waking agent)
/// to bridge the gap, which is where creative insight often emerges.
struct DaliInterruptCell {
    /// Fraction of fragments to interrupt.
    interrupt_rate: f64,          // default: 0.6
    /// Minimum prefix length to retain (tokens).
    min_prefix_tokens: usize,    // default: 10
    /// Maximum prefix length (tokens).
    max_prefix_tokens: usize,    // default: 40
    /// Operating mode.
    mode: InterruptMode,         // default: Parallel
}

#[derive(Clone, Copy)]
enum InterruptMode {
    /// Interrupt every Nth fragment sequentially.
    Sequential { stride: usize },
    /// Interrupt all fragments in parallel, random positions.
    Parallel,
}

impl ReactProtocol for DaliInterruptCell {
    async fn react(
        &self,
        observed: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>> {
        let mut output = Vec::new();
        let mut rng = ctx.deterministic_rng("dali-interrupt");

        for (i, signal) in observed.iter().enumerate() {
            let should_interrupt = match self.mode {
                InterruptMode::Sequential { stride } => i % stride == 0,
                InterruptMode::Parallel => rng.gen_f64() < self.interrupt_rate,
            };

            if should_interrupt {
                // Truncate the fragment at a random position.
                let content = signal.content_str();
                let cut_point = rng.gen_range(
                    self.min_prefix_tokens..self.max_prefix_tokens.min(content.len()),
                );
                let prefix = &content[..char_boundary(content, cut_point)];

                output.push(
                    Signal::new(Kind::DreamFragment, format!("{prefix}..."))
                        .with_metadata("hypnagogia.interrupted", true)
                        .with_metadata("hypnagogia.cut_point", cut_point)
                        .with_provenance(signal.hash())
                );
            } else {
                // Pass through uninterrupted.
                output.push(signal.clone());
            }
        }

        Ok(output)
    }
}
```

**Why interruption works.** Complete fragments are self-contained; the observer can evaluate them but has little to add. Interrupted fragments have a gap that must be bridged. When the HomuncularObserver (or the waking agent encountering the fragment later) fills the gap, it must import structure from its current context -- creating a cross-domain connection that would not have existed in the original complete fragment. This is the mechanism behind Horowitz's TDI results: the interrupted sleep-onset state forces association between the cue topic and whatever the dreamer's mind produces to complete the fragment.

### 2.4 HomuncularObserver Cell (Verify protocol)

The homuncular observer is the quality gate. It scores each fragment on three axes (novelty, relevance, coherence), applies a triple threshold, and computes the geometric mean as the composite score. This is a Verify Cell: Signals in, Verdict out, total function, deterministic on identical inputs.

```rust
/// HomuncularObserver: Verify Cell with triple-axis threshold.
///
/// Scores each fragment on novelty, relevance, and coherence.
/// A fragment passes only if ALL THREE scores exceed 0.5.
/// The composite score is the geometric mean: (n * r * c)^(1/3).
///
/// This is the same Verify protocol used by CompileGate and TestGate,
/// but with an LLM-based scoring function instead of a subprocess.
struct HomuncularObserverCell {
    /// Temperature for the observer LLM (low: focused evaluation).
    observer_temperature: f64,    // default: 0.4
    /// Minimum score on each axis for retention.
    threshold: f64,               // default: 0.5
    /// Maximum fragments to retain per session.
    max_retained: usize,          // default: 6
}

impl VerifyProtocol for HomuncularObserverCell {
    async fn verify(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Verdict>> {
        let focus = ctx.get::<HdcVector>("hypnagogia.focus_vector")?;
        let mut verdicts = Vec::new();

        for fragment in &input {
            let scores = self.score_fragment(fragment, &focus, ctx).await?;

            let passed = scores.novelty > self.threshold
                && scores.relevance > self.threshold
                && scores.coherence > self.threshold;

            let composite = (scores.novelty * scores.relevance * scores.coherence)
                .powf(1.0 / 3.0);

            verdicts.push(Verdict {
                passed,
                gate_name: "homuncular-observer".into(),
                details: format!(
                    "novelty={:.2} relevance={:.2} coherence={:.2} composite={:.2}",
                    scores.novelty, scores.relevance, scores.coherence, composite,
                ),
                scores: ObserverScores {
                    novelty: scores.novelty,
                    relevance: scores.relevance,
                    coherence: scores.coherence,
                    composite,
                },
                duration_ms: scores.duration_ms,
            });
        }

        Ok(verdicts)
    }
}

struct ObserverScores {
    novelty: f64,      // 0.0-1.0: how unlike anything in the archive
    relevance: f64,    // 0.0-1.0: how connected to the current goal
    coherence: f64,    // 0.0-1.0: how internally self-consistent
    composite: f64,    // geometric mean of the three
}

impl HomuncularObserverCell {
    /// Score a fragment using a low-temperature LLM call.
    async fn score_fragment(
        &self,
        fragment: &Signal,
        focus: &HdcVector,
        ctx: &CellContext,
    ) -> Result<ScoredFragment> {
        let start = std::time::Instant::now();

        // Relevance: HDC similarity between fragment and focus vector.
        // This is pure computation -- no LLM call needed.
        let relevance = if let Some(frag_hdc) = fragment.hdc_vector() {
            frag_hdc.similarity(focus) as f64
        } else {
            0.5 // neutral if no HDC vector available
        };

        // Novelty: distance from the k-nearest neighbors in the archive.
        // Uses the novelty archive (see section 3).
        let novelty = ctx.get::<NoveltyArchive>("hypnagogia.novelty_archive")?
            .novelty_score(fragment);

        // Coherence: LLM judge at low temperature.
        let coherence = self.llm_coherence_score(fragment, ctx).await?;

        Ok(ScoredFragment {
            novelty, relevance, coherence,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn llm_coherence_score(
        &self,
        fragment: &Signal,
        ctx: &CellContext,
    ) -> Result<f64> {
        let rack = ComposeRack {
            temperature: self.observer_temperature,
            max_tokens: 10,
            ..Default::default()
        };

        let response = ctx.llm.generate(
            "Rate the internal coherence of this fragment 0.0-1.0. \
             Respond with only a number.",
            &[fragment.content_str()],
            &rack,
        ).await?;

        parse_score(&response).unwrap_or(0.5)
    }
}
```

**Geometric mean, not arithmetic mean.** The composite score `(n * r * c)^(1/3)` penalizes imbalance more harshly than the arithmetic mean. A fragment with novelty=0.9, relevance=0.9, coherence=0.1 gets composite 0.43 (fail), while the arithmetic mean would be 0.63 (pass). This is intentional: a highly novel, highly relevant but incoherent fragment is useless. All three axes must be above threshold independently, and the geometric mean ensures the composite tracks the weakest axis.

---

## 3. Novelty Search as Score Cell

Novelty search (Lehman & Stanley 2011) maintains an archive of previously-seen behaviors and scores new candidates by their distance from the archive's k-nearest neighbors. In Roko, the novelty archive is a Score Cell with an HDC-backed k-NN store.

```rust
/// Novelty archive: a Score Cell that maintains a bounded archive
/// of HDC fingerprints and scores new fragments by k-NN distance.
///
/// This is the same pattern as novelty in evolutionary search:
/// score(f) = mean distance to k nearest archive members.
struct NoveltyArchive {
    /// HDC fingerprints of archived fragments.
    archive: Vec<HdcVector>,
    /// Number of neighbors for k-NN scoring.
    k_neighbors: usize,           // default: 15
    /// Minimum novelty for archive inclusion.
    novelty_threshold: f64,       // default: 0.30
    /// Maximum archive size (FIFO eviction).
    max_archive: usize,           // default: 1000
}

impl ScoreProtocol for NoveltyArchive {
    async fn score(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<ScoredSignal>> {
        input.into_iter().map(|signal| {
            let novelty = self.novelty_score(&signal);
            let serendipity = self.serendipity_score(&signal, ctx)?;

            Ok(ScoredSignal {
                signal,
                score: serendipity,
                components: vec![
                    ("novelty".into(), novelty),
                    ("serendipity".into(), serendipity),
                ],
            })
        }).collect()
    }
}

impl NoveltyArchive {
    /// Novelty score: mean distance to k nearest archive members.
    ///
    /// If the archive has fewer than k entries, uses all entries.
    /// Empty archive returns 1.0 (maximally novel).
    fn novelty_score(&self, signal: &Signal) -> f64 {
        if self.archive.is_empty() {
            return 1.0;
        }

        let query = signal.hdc_vector_or_compute();

        // Compute distances to all archive members.
        let mut distances: Vec<f64> = self.archive.iter()
            .map(|archived| 1.0 - query.similarity(archived) as f64)
            .collect();

        distances.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Mean of k nearest distances.
        let k = self.k_neighbors.min(distances.len());
        distances[..k].iter().sum::<f64>() / k as f64
    }

    /// Serendipity = novelty(f) * relevance(f, problem).
    ///
    /// High novelty alone is noise. High relevance alone is exploitation.
    /// The product selects for Signals that are both surprising AND useful.
    fn serendipity_score(
        &self,
        signal: &Signal,
        ctx: &CellContext,
    ) -> Result<f64> {
        let novelty = self.novelty_score(signal);
        let focus = ctx.get::<HdcVector>("hypnagogia.focus_vector")?;
        let relevance = signal.hdc_vector_or_compute()
            .similarity(&focus) as f64;

        Ok(novelty * relevance)
    }

    /// Conditionally add a fragment to the archive.
    ///
    /// Only archived if novelty exceeds threshold.
    /// FIFO eviction when archive is full.
    fn maybe_archive(&mut self, signal: &Signal) {
        let novelty = self.novelty_score(signal);
        if novelty < self.novelty_threshold {
            return;
        }

        let vector = signal.hdc_vector_or_compute();

        if self.archive.len() >= self.max_archive {
            self.archive.remove(0); // FIFO eviction
        }

        self.archive.push(vector);
    }
}
```

**Spreading activation as Score Cell variant.** The source material describes spreading activation: `activation(j,t) = SUM[activation(i,0) * decay_rate^path_length * e^(-lambda*t)]`. This is another Score Cell operating on the knowledge graph instead of the novelty archive. It computes the activation of a Signal by summing the decayed influence of all Signals that cite or are cited by it, weighted by path length and time. Both the novelty archive and spreading activation conform to the same Score protocol -- they differ only in what data structure they maintain and what distance metric they use.

```rust
/// Spreading activation over the knowledge citation graph.
///
/// activation(target) = SUM over sources:
///   source.activation * decay_rate^path_length * exp(-lambda * time_delta)
///
/// This is a Score Cell: Signals in, scored Signals out.
struct SpreadingActivationScorer {
    decay_rate: f64,       // default: 0.85 per hop
    lambda: f64,           // default: 0.01 per hour
    max_hops: usize,       // default: 3
}

impl ScoreProtocol for SpreadingActivationScorer {
    async fn score(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<ScoredSignal>> {
        let graph = ctx.store.citation_graph().await?;

        input.into_iter().map(|signal| {
            let activation = graph.spreading_activation(
                &signal.hash(),
                self.decay_rate,
                self.lambda,
                self.max_hops,
            );

            Ok(ScoredSignal {
                signal,
                score: activation,
                components: vec![("activation".into(), activation)],
            })
        }).collect()
    }
}
```

---

## 4. Targeted Dream Incubation as Trigger Cell

Targeted Dream Incubation (TDI, Horowitz et al. 2023) injects goal context at the pipeline's input, biasing the hypnagogic session toward a specific problem. In Roko, TDI is a Trigger Cell: it fires on a condition and injects a Signal into the Pipeline.

```rust
/// TDI: Trigger Cell that injects goal context into the
/// hypnagogia Pipeline.
///
/// Fires when a problem meets incubation criteria (repeated failure,
/// high prediction error, manual request). Injects the problem's
/// HDC fingerprint as the focus vector, biasing ThalamicGate retrieval
/// toward anti-correlates of the stuck problem.
struct TargetedDreamIncubationCell {
    /// Reinforcement interval: re-inject the cue every N fragments.
    reinforcement_interval: usize,    // default: 3
    /// Target semantic distance from the cue.
    semantic_distance_target: f64,    // default: 0.50
    /// Cue source strategy.
    cue_source: CueSource,
}

#[derive(Clone)]
enum CueSource {
    /// Use the task with the most consecutive gate failures.
    RepeatedFailure { min_failures: usize },
    /// Use the Signal with highest unresolved prediction error.
    HighestPredictionError,
    /// Use an explicit topic from the operator.
    Manual { topic: String },
    /// Use the current active task.
    CurrentTask,
}

impl TriggerProtocol for TargetedDreamIncubationCell {
    fn condition(&self) -> TriggerCondition {
        TriggerCondition::Or(vec![
            // Fire when an agent has been idle for 5+ minutes
            TriggerCondition::IdleTimeout(Duration::from_secs(300)),
            // Fire when unprocessed episodes exceed threshold
            TriggerCondition::SignalCount {
                kind: Kind::Episode,
                tier: Tier::Transient,
                min_count: 50,
            },
            // Fire on manual Bus signal
            TriggerCondition::BusTopic("dream.incubate".into()),
        ])
    }

    async fn fire(
        &self,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>> {
        // Resolve the cue: which problem to incubate.
        let cue = match &self.cue_source {
            CueSource::RepeatedFailure { min_failures } => {
                ctx.store.query_most_failed_task(*min_failures).await?
            }
            CueSource::HighestPredictionError => {
                ctx.store.query_highest_pe_signal().await?
            }
            CueSource::Manual { topic } => {
                Signal::new(Kind::DreamCue, topic.clone())
            }
            CueSource::CurrentTask => {
                ctx.get::<Signal>("agent.current_task")?
            }
        };

        // Compute the focus vector from the cue.
        let focus = cue.hdc_vector_or_compute();

        // Emit the focus vector as a Signal that ThalamicGate will consume.
        Ok(vec![
            Signal::new(Kind::DreamCue, cue.content_str().to_string())
                .with_hdc_vector(focus)
                .with_metadata("tdi.cue_source", format!("{:?}", self.cue_source))
                .with_metadata("tdi.reinforcement_interval",
                    self.reinforcement_interval)
                .with_metadata("tdi.semantic_distance_target",
                    self.semantic_distance_target),
        ])
    }
}
```

**Reinforcement within the session.** TDI does not just set the initial focus. Every `reinforcement_interval` fragments (default: 3), the Pipeline re-injects the cue Signal, preventing the session from drifting too far from the problem. The `semantic_distance_target` (default: 0.50) controls how far the fragments are allowed to wander: too close (< 0.30) produces exploitation, too far (> 0.70) produces noise. The Loop adjusts this target based on observed serendipity yield (section 5).

---

## 5. The Pipeline Graph and Outer Loop

### 5.1 Pipeline Graph

The four Cells compose into a Pipeline Graph -- an acyclic sequence that the Engine executes once per hypnagogia session.

```toml
# hypnagogia-pipeline.toml
[graph]
name = "hypnagogia-pipeline"
pattern = "Pipeline"
timescale = "delta"  # runs during idle periods, not on the main loop

[[cells]]
name = "tdi-trigger"
type = "trigger.tdi"
protocols = ["Trigger"]

[[cells]]
name = "thalamic-gate"
type = "hypnagogia.thalamic-gate"
protocols = ["Store", "Score"]

[[cells]]
name = "executive-loosener"
type = "hypnagogia.executive-loosener"
protocols = ["Compose"]

[[cells]]
name = "dali-interrupt"
type = "hypnagogia.dali-interrupt"
protocols = ["React"]

[[cells]]
name = "homuncular-observer"
type = "hypnagogia.homuncular-observer"
protocols = ["Verify"]

[[cells]]
name = "staging"
type = "hypnagogia.staging"
protocols = ["Store"]

[[edges]]
from = "tdi-trigger"
to = "thalamic-gate"

[[edges]]
from = "thalamic-gate"
to = "executive-loosener"

[[edges]]
from = "executive-loosener"
to = "dali-interrupt"

[[edges]]
from = "dali-interrupt"
to = "homuncular-observer"

[[edges]]
from = "homuncular-observer"
to = "staging"
condition = "verdict.passed"
```

### 5.2 The Outer Loop

The Pipeline runs inside a Loop -- a Graph that feeds output back to input on the delta timescale. The Loop runs the Pipeline repeatedly during an idle period, adjusting Rack parameters (temperature, interrupt rate, semantic distance target) based on the serendipity yield of previous iterations.

```rust
/// Outer Loop for the hypnagogia Pipeline.
///
/// Each iteration:
/// 1. Advance the temperature curve (alpha -> theta transition).
/// 2. Run the Pipeline (ThalamicGate -> ... -> HomuncularObserver).
/// 3. Compute serendipity yield (retained fragments / total fragments).
/// 4. Adjust Rack parameters based on yield.
/// 5. Check budget. Stop if budget exceeded or yield collapses.
struct HypnagogiaLoop {
    pipeline: Graph,
    temperature_curve: TemperatureCurve,
    novelty_archive: NoveltyArchive,
    budget: HypnagogiaBudget,
    max_iterations: usize,          // default: 8
    min_yield: f64,                  // default: 0.10 (stop if < 10% retained)
}

struct HypnagogiaBudget {
    max_cost_usd: f64,              // default: 0.05
    target_cost_usd: f64,           // default: 0.011
    spent_usd: f64,
}

impl HypnagogiaLoop {
    async fn run(&mut self, ctx: &CellContext) -> Result<Vec<Signal>> {
        let mut all_retained = Vec::new();
        let mut iteration = 0;

        while iteration < self.max_iterations
            && self.budget.spent_usd < self.budget.max_cost_usd
        {
            let progress = iteration as f64 / self.max_iterations as f64;

            // 1. Advance temperature.
            let temp = temperature_at(progress, &self.temperature_curve);
            ctx.set_rack("compose.temperature", temp);

            // 2. Run the Pipeline.
            let output = self.pipeline.execute(vec![], ctx).await?;

            // 3. Compute yield.
            let retained: Vec<Signal> = output.iter()
                .filter(|s| s.metadata("verdict.passed") == Some("true"))
                .cloned()
                .collect();

            let yield_rate = if output.is_empty() {
                0.0
            } else {
                retained.len() as f64 / output.len() as f64
            };

            // 4. Archive retained fragments for novelty tracking.
            for signal in &retained {
                self.novelty_archive.maybe_archive(signal);
            }

            // 5. Adjust parameters based on yield.
            if yield_rate < 0.15 {
                // Too little retained: increase relevance,
                // decrease interrupt rate.
                ctx.adjust_rack("tdi.semantic_distance_target", -0.05);
                ctx.adjust_rack("dali.interrupt_rate", -0.1);
            } else if yield_rate > 0.50 {
                // Too much retained: search is not divergent enough.
                // Increase temperature and interrupt rate.
                ctx.adjust_rack("tdi.semantic_distance_target", 0.05);
                ctx.adjust_rack("dali.interrupt_rate", 0.1);
            }

            // 6. Update budget.
            self.budget.spent_usd += estimate_cost(&output);

            // 7. Check termination.
            if yield_rate < self.min_yield && iteration > 1 {
                break; // yield collapsed, stop early
            }

            all_retained.extend(retained);
            iteration += 1;
        }

        Ok(all_retained)
    }
}
```

**Budget discipline.** The target budget is ~$0.011/session, with a hard cap at $0.05. At Haiku pricing (~$0.25/M input, $1.25/M output), this allows approximately 8,000 input tokens and 4,000 output tokens -- enough for 5-8 fragments of 50-100 tokens each with observer scoring. The Loop enforces this by tracking `spent_usd` and terminating early when the cap is reached.

---

## 6. Hypnagogia as Functor

The 10x insight: hypnagogia should not be limited to offline dream cycles. Any Compose call could benefit from creative divergence. The mechanism is a **Functor** -- an endofunctor on the Signal category that wraps a Compose Cell with the hypnagogia Pipeline.

```rust
/// HypnagogiaFunctor: wraps any Compose Cell to add creative divergence.
///
/// F(compose)(bids, budget, ctx) =
///   1. Run compose(bids, budget, ctx) normally -> primary_output.
///   2. Run a single-iteration hypnagogia Pipeline on the residual
///      budget -> creative_fragments.
///   3. Merge: primary_output + creative_fragments (capped at budget).
///
/// The result is a Compose Cell that produces both focused output
/// (exploitation) and divergent fragments (exploration).
struct HypnagogiaFunctor {
    /// Fraction of budget to allocate to creative search.
    creative_budget_fraction: f64,    // default: 0.10
    /// Minimum budget in USD before creative search is attempted.
    min_budget_for_creativity: f64,   // default: 0.005
    /// The hypnagogia Pipeline (lightweight: 1 iteration, no loop).
    pipeline: Graph,
}

impl CrossCutFunctor for HypnagogiaFunctor {
    fn name(&self) -> &str { "hypnagogia.compose" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // No pre-enrichment -- creative search happens after composition.
        Ok(input)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let remaining_budget = ctx.budget_remaining;
        let creative_budget = remaining_budget.usd()
            * self.creative_budget_fraction;

        if creative_budget < self.min_budget_for_creativity {
            return Ok(output); // not enough budget for creative search
        }

        // Run a single-iteration hypnagogia Pipeline.
        let creative_ctx = ctx.with_budget(Cost::from_usd(creative_budget));
        let fragments = self.pipeline.execute(vec![], &creative_ctx).await?;

        // Tag creative fragments so downstream Cells can distinguish them.
        let tagged: Vec<Signal> = fragments.into_iter()
            .map(|s| s.with_metadata("source", "hypnagogia-functor")
                      .with_metadata("creative", true))
            .collect();

        // Merge: primary output first, creative fragments appended.
        let mut merged = output;
        merged.extend(tagged);
        Ok(merged)
    }
}
```

**Composability.** The Functor can wrap *any* Compose Cell. Apply it to the 9-layer system prompt builder, and every agent prompt gets a small creative appendix with anti-correlated associations. Apply it to the research context assembler, and research queries get serendipitous cross-domain Signals. The agent does not know the creative fragments came from hypnagogia -- they are just additional Signals in the context with lower initial balance (must earn survival through downstream gate passes).

---

## 7. Shared Novelty Archive via Bus

The novelty archive (section 3) is agent-local by default. But creative search benefits from *collective* novelty -- what is novel to agent A may be familiar to agent B, and vice versa. Sharing the archive via Bus enables fleet-wide creative coordination.

```rust
/// Bus-backed novelty archive that syncs across agents.
///
/// Each agent publishes retained fragment fingerprints as Pulses.
/// Other agents subscribe and merge them into their local archive.
/// The result: novelty is measured against the entire fleet's
/// creative output, not just one agent's history.
struct SharedNoveltyArchive {
    local: NoveltyArchive,
    bus_topic: String,               // "hypnagogia.novelty-archive"
}

impl SharedNoveltyArchive {
    /// Publish a retained fragment's fingerprint to the Bus.
    async fn publish_retained(
        &self,
        signal: &Signal,
        bus: &dyn Bus,
    ) -> Result<()> {
        let pulse = Pulse::new(
            &self.bus_topic,
            signal.hdc_vector_or_compute().to_bytes(),
        ).with_ttl(Duration::from_secs(86400)); // 24h TTL

        bus.publish(pulse).await
    }

    /// Merge incoming Pulses from other agents into the local archive.
    async fn merge_remote(&mut self, bus: &dyn Bus) -> Result<usize> {
        let pulses = bus.drain(&self.bus_topic).await?;
        let mut merged = 0;

        for pulse in pulses {
            if let Ok(vector) = HdcVector::from_bytes_slice(pulse.payload()) {
                if self.local.archive.len() < self.local.max_archive {
                    self.local.archive.push(vector);
                    merged += 1;
                }
            }
        }

        Ok(merged)
    }

    /// Novelty score measured against the combined local + remote archive.
    fn novelty_score(&self, signal: &Signal) -> f64 {
        self.local.novelty_score(signal)
    }
}
```

**Fleet-level creative diversity.** When agent A discovers a novel association, it publishes the fingerprint. Agent B's novelty score for similar associations drops, steering B toward different creative territory. Over time, the fleet explores a wider region of the creative search space than any individual agent could. The Bus's TTL (24h) ensures old creative output decays, preventing the archive from ossifying.

---

## 8. Staging: The Confidence Ladder

Dream-generated fragments must not enter durable memory directly. The staging buffer is a Store Cell that implements a four-stage confidence ladder:

| Stage | Confidence Floor | Advancement Condition |
|---|---|---|
| **Raw** | 0.20 | Just extracted by HomuncularObserver |
| **Replayed** | 0.30 | Survived one subsequent dream cycle without contradiction |
| **Validated** | 0.50 | Non-redundant (HDC similarity < 0.90 vs. existing Store) and non-contradicted |
| **Promoted** | 0.70 | Written to Memory Store at Transient tier |

Raw entries that do not advance past Raw within 7 days are garbage collected. This prevents dream hallucinations from accumulating in memory.

```rust
/// Staging Cell: a Store-protocol Cell that gates dream output
/// through a confidence ladder before it enters durable Memory.
///
/// The staging buffer lives at `.roko/dreams/staging.json`.
/// Each dream cycle runs advance_replayed(), advance_validated(),
/// then promote_validated(). GC runs on every cycle.
struct StagingCell {
    buffer: StagingBuffer,
    gc_horizon_days: i64,             // default: 7
    redundancy_threshold: f32,        // default: 0.90
}

impl StoreProtocol for StagingCell {
    async fn put(&self, signals: Vec<Signal>, ctx: &CellContext) -> Result<()> {
        for signal in signals {
            self.buffer.add_candidate(signal, ctx.run_id_str());
        }
        Ok(())
    }

    async fn query(&self, query: &Query, _ctx: &CellContext) -> Result<Vec<Signal>> {
        // Query the staging buffer by stage, confidence, or HDC similarity.
        self.buffer.query(query)
    }

    async fn prune(&self, _ctx: &CellContext) -> Result<usize> {
        self.buffer.gc();
        self.buffer.remove_promoted();
        Ok(self.buffer.pruned_count())
    }
}
```

---

## 9. Semantic Elaboration

Fragments that pass the observer are short (50-100 tokens) and often cryptic. Before staging, an optional elaboration step expands retained fragments into actionable hypotheses using a mid-tier model at moderate temperature.

```rust
/// Elaboration: a Compose Cell that expands terse fragments
/// into actionable hypotheses.
///
/// Input: retained fragments (50-100 tokens, high novelty+relevance+coherence).
/// Output: elaborated hypotheses (100-200 tokens, with action suggestions).
///
/// Uses Sonnet-tier model at T=0.5 -- more focused than the loosener,
/// but still allowing some creative latitude.
struct ElaborationCell {
    model_tier: ModelTier,            // default: Sonnet
    temperature: f64,                 // default: 0.5
    max_tokens: usize,               // default: 200
    max_elaborations: usize,         // default: 4
}

impl ComposeProtocol for ElaborationCell {
    async fn compose(
        &self,
        bids: Vec<ComposeBid>,
        budget: &ComposeBudget,
        ctx: &ComposeContext,
    ) -> Result<ComposeResult> {
        let mut elaborated = Vec::new();

        // Only elaborate the top N fragments by composite score.
        let top_fragments: Vec<_> = bids.iter()
            .sorted_by(|a, b| b.value.partial_cmp(&a.value).unwrap())
            .take(self.max_elaborations)
            .collect();

        for bid in top_fragments {
            let rack = ComposeRack {
                temperature: self.temperature,
                max_tokens: self.max_tokens,
                model_tier: self.model_tier,
                ..Default::default()
            };

            let response = ctx.llm.generate(
                "Expand this creative fragment into an actionable hypothesis. \
                 State what it suggests, why it might work, and one concrete \
                 next step to test it. 100-200 tokens.",
                &[bid.section.content.as_str()],
                &rack,
            ).await?;

            elaborated.push(
                Signal::new(Kind::DreamHypothesis, response)
                    .with_provenance(bid.signal_hash())
                    .with_metadata("hypnagogia.stage", "elaborated")
                    .with_confidence(bid.value * 0.8)
                    .with_balance(0.5)
            );
        }

        Ok(ComposeResult {
            output: elaborated,
            ..Default::default()
        })
    }
}
```

---

## 10. Complete Pipeline with Budget

The full hypnagogia Pipeline, including elaboration and staging:

```
TDI Trigger
    |
    v
ThalamicGate  (Store+Score: anti-correlated retrieval, ~0 cost)
    |
    v
ExecutiveLoosener  (Compose: T=1.3, 50-100 tok * 5 fragments, ~$0.003)
    |
    v
DaliInterrupt  (React: truncation, ~0 cost)
    |
    v
HomuncularObserver  (Verify: T=0.4, 10 tok * 5 scores, ~$0.001)
    |
    v
Elaboration  (Compose: T=0.5, Sonnet, 200 tok * 4 fragments, ~$0.006)
    |
    v
Staging  (Store: confidence ladder, ~0 cost)

Total per session: ~$0.010-0.011
Hard cap: $0.05
```

**Budget breakdown per iteration.** ThalamicGate and DaliInterrupt are pure computation (HDC operations, string truncation). The three LLM calls are ExecutiveLoosener (~$0.003 at Haiku), HomuncularObserver (~$0.001 at Haiku for coherence scoring), and Elaboration (~$0.006 at Sonnet for 4 fragments at 200 tokens each). Total: ~$0.010 per iteration, well within the $0.011 target and far below the $0.05 hard cap.

---

## 11. Codebase Mapping

| Unified Concept | Existing Code | Gap |
|---|---|---|
| ThalamicGate Cell | `roko-dreams/src/hypnagogia.rs` `thalamic_gate()` | Uses hash-based resonance, not HDC anti-correlation |
| ExecutiveLoosener Cell | `roko-dreams/src/hypnagogia.rs` `executive_loosen()` | No LLM call; uses tag-based association |
| DaliInterrupt Cell | `roko-dreams/src/hypnagogia.rs` `dali_interrupt()` | Stride-based, no random truncation |
| HomuncularObserver Cell | `roko-dreams/src/hypnagogia.rs` `homuncular_observer()` | Single score axis, not triple (novelty/relevance/coherence) |
| NoveltyArchive | Not implemented | Phase 2 `NoveltyFilter` struct exists as stub only |
| TDI Trigger | `roko-dreams/src/phase2/hypnagogia.rs` `TargetedDreamIncubation` struct | Stub only, not wired |
| Temperature curve | `roko-dreams/src/phase2/hypnagogia.rs` `HypnagogicTemperatureCurve` | Implemented, not connected to Pipeline |
| Staging buffer | `roko-dreams/src/staging.rs` `StagingBuffer` | Fully implemented, needs Pipeline integration |
| Elaboration | Not implemented | No elaboration step in current pipeline |
| Shared archive via Bus | Not implemented | Requires Bus integration |
| Functor wrapper | Not implemented | Cross-cut functor pattern exists in spec |

---

## What This Enables

1. **Composable creativity.** Hypnagogia is not a monolithic engine but a Pipeline Graph. Each Cell can be tested, replaced, or reconfigured independently. Swap the ExecutiveLoosener for a different Compose Cell and the Pipeline still works.

2. **Waking creativity via Functor.** The HypnagogiaFunctor wraps any Compose Cell to add creative divergence during waking operation. An agent solving a hard problem does not need to wait for a dream cycle -- it can run a single-iteration hypnagogia Pipeline inline, using 10% of its remaining budget.

3. **Fleet-level creative coordination.** The shared novelty archive via Bus ensures that agents explore different creative territory. Agent A's discoveries reduce novelty scores for similar associations in agent B's archive, steering the fleet toward collective coverage of the creative space.

4. **Budget-controlled exploration.** Every creative operation has a cost. The Loop enforces a hard budget cap, the staging buffer filters hallucinations, and the confidence ladder ensures only validated fragments enter durable memory. Creativity is not free; it is an investment with measurable ROI (serendipity yield).

5. **Self-tuning divergence.** The Loop adjusts temperature, interrupt rate, and semantic distance target based on observed serendipity yield. If a session produces too few retained fragments, it tightens the search cone. If too many, it loosens. This is the same adaptive threshold pattern used by the gate rung pipeline, applied to creative search.

---

## Feedback Loops

1. **Serendipity yield -> Rack parameters.** The Loop monitors retained/total ratio and adjusts temperature, interrupt rate, and TDI semantic distance target. This is a fast feedback loop (per-iteration, within a single session).

2. **Staging ladder -> Memory quality.** Fragments that pass the confidence ladder (Raw -> Replayed -> Validated -> Promoted) enter Memory at Transient tier. If they survive demurrage and get cited/retrieved, they progress to higher tiers. If they don't, they decay. This is a slow feedback loop (days to weeks).

3. **Gate outcomes -> Novelty archive.** When a waking agent uses a hypnagogia-sourced Signal in a context pack that passes a gate, the Signal's balance is reinforced and the corresponding novelty archive entry is tagged as "validated." Validated entries shift the archive's coverage map, affecting future novelty scores. This closes the loop between offline creativity and online verification.

4. **Fleet archive -> Divergence metrics.** The shared novelty archive feeds into the divergence metrics system (`crates/roko-dreams/src/phase2/divergence.rs`). If fleet-wide knowledge JSD drops below the target band (< 0.20), hypnagogia sessions are triggered more aggressively to restore creative diversity. If JSD rises above the band (> 0.60), sessions are throttled.

5. **TDI cue selection -> Problem resolution.** When TDI incubates a specific problem (repeated failure, high prediction error), and the resulting fragments lead to a successful gate pass on that problem, the TDI system marks the problem as resolved and selects a new cue. This closes the loop between creative search and task completion.

---

## Open Questions

1. **Optimal k for novelty search.** The default k=15 for k-NN novelty scoring comes from Lehman & Stanley (2011) for neuroevolution. Is this the right scale for HDC-based knowledge novelty? The 10,240-bit vectors have different distance characteristics than continuous-valued behavior descriptors. Empirical calibration needed against the staging ladder's promotion rate.

2. **Anti-correlation retrieval depth.** Inverting the HDC query vector retrieves Signals from the opposite hemisphere of the HDC space. But the "opposite" of a code-compilation focus vector might be natural-language poetry Signals that are genuinely useless, not creatively useful. Should anti-correlation be bounded -- e.g., retrieve from the 40th-60th percentile of similarity rather than the bottom? The `confidence_floor` (0.20) provides some protection, but the optimal retrieval band is unknown.

3. **Interrupt position distribution.** The current design truncates at a uniform random position. Biological hypnagogia interrupts at sleep-onset transitions, which are not uniformly distributed -- they cluster around specific EEG patterns. Should the interrupt position be biased toward semantically meaningful boundaries (sentence breaks, clause boundaries) to produce more bridge-able fragments?

4. **Cross-agent elaboration.** When agent A produces a fragment and agent B encounters it via the shared archive, should B elaborate the fragment in its own context? This would produce domain-specific elaborations of domain-general fragments -- potentially powerful but expensive. The budget implications need analysis.

5. **Functor activation threshold.** The HypnagogiaFunctor currently activates on any Compose call with sufficient budget. Should it activate only when the waking agent is "stuck" (high prediction error, consecutive gate failures)? Always-on creativity adds cost without guaranteed benefit. The tradeoff between exploration budget and task completion speed needs empirical data.

6. **Geometric mean vs. other aggregations for the triple threshold.** The geometric mean penalizes imbalance harshly. An alternative is the harmonic mean (even harsher) or a weighted geometric mean where relevance counts double (reflecting the practical priority of "useful to the current problem" over "novel in the abstract"). Which aggregation produces the best staging-to-promotion ratio?

7. **Temperature curve biological fidelity.** The sigmoid alpha-to-theta transition with micro-oscillations is inspired by EEG data but not calibrated against it. Does the oscillation amplitude (0.1) and frequency (3 cycles per session) produce measurably different fragment quality than a simple linear ramp? The added complexity may not be worth it if the effect is marginal.
