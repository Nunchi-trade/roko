# HDC Pattern Encoding and Metabolism

> Depth for [06-hyperdimensional-ta.md](../../docs/20-technical-analysis/06-hyperdimensional-ta.md), [08-adaptive-signal-metabolism.md](../../docs/20-technical-analysis/08-adaptive-signal-metabolism.md), [10-predictive-geometry-and-resonant-patterns.md](../../docs/20-technical-analysis/10-predictive-geometry-and-resonant-patterns.md). Reframes HDC pattern matching as Store-native pattern recognition with evolutionary dynamics -- patterns encoded as Signals compete for attention via the replicator equation, strengthened by Hebbian learning, pruned by demurrage.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, HDC fingerprint 10,240-bit, demurrage, Kind system), [02-CELL](../../unified/02-CELL.md) (Cell, Store protocol, Score protocol, query_similar), [03-GRAPH](../../unified/03-GRAPH.md) (Loop pattern, Hot Graph), [06-MEMORY](../../unified/06-MEMORY.md) (Memory specialization, dreams, consolidation, HDC algebra)

---

## 1. The Core Claim

HDC pattern matching is not a separate "technical analysis" subsystem. It is the same infrastructure that powers the knowledge Store, code intelligence, and immune system -- **Signals are already HDC-fingerprinted** (10,240-bit vectors, per spec doc 01). The "technical analysis" aspect is simply: using `Store.query_similar()` to find Signals whose patterns resemble the current observation, then using the evolutionary dynamics of demurrage and predict-publish-correct to ensure only predictively useful patterns survive.

The key equations:

| Mechanism | What it does | Unified primitive |
|---|---|---|
| `BIND(role, filler)` = XOR | Associate a concept with a value | Signal's HDC fingerprint construction |
| `BUNDLE([a, b, c])` = majority | Merge multiple observations | Signal composition (lineage bundling) |
| `PERMUTE(a, i)` = rotate by i | Encode temporal position | Temporal Signal ordering |
| `query_similar(hv, threshold)` | Find matching patterns | Store protocol's query_similar() |
| Replicator equation | Fitness-proportionate selection | Demurrage + retrieval economics |
| Oja's rule | Hebbian confidence update | Predict-publish-correct feedback |

The "niche construction" thesis: patterns are organisms that modify their environment (the Store) to favor their own reproduction. A pattern that successfully predicts outcomes gets retrieved more (increasing its demurrage balance), which keeps it alive longer, which means it gets retrieved more. This is exactly the demurrage economics of spec doc 01 -- signals that earn their keep survive; those that don't fade.

---

## 2. Patterns as Signals in Store

Every pattern in the system is a Signal with `Kind::Pattern`. The HDC vector IS the Signal's fingerprint. Pattern matching IS `Store.query_similar()`. There is no separate pattern store -- patterns live in the same Store as every other Signal, subject to the same demurrage, the same lineage tracking, the same access control.

```rust
/// A pattern is a Signal with Kind::Pattern.
///
/// The HDC fingerprint (10,240-bit) encodes:
///   - Role-filler pairs (BIND): what kind of observation + specific value
///   - Temporal sequence (PERMUTE): ordering of observations
///   - Composite state (BUNDLE): multiple concurrent observations
///
/// The pattern lives in Store, subject to demurrage.
/// Retrieval (via query_similar) reinforces the pattern's balance.
/// Non-retrieval lets it decay. Only predictively useful patterns survive.
///
/// Location: `crates/roko-primitives/src/hdc.rs`, `crates/roko-neuro/src/`
pub fn encode_pattern_as_signal(
    observations: &[Observation],
    domain: &OracleDomain,
    outcome: Option<&PredictionOutcome>,
) -> Signal {
    // Step 1: Encode each observation as role-filler binding
    let bindings: Vec<HdcVector> = observations.iter()
        .map(|obs| obs.role_vector.xor(&obs.filler_vector))
        .collect();

    // Step 2: Encode temporal ordering via permutation
    let temporal: Vec<HdcVector> = bindings.iter()
        .enumerate()
        .map(|(i, v)| v.permute(i as u32))
        .collect();

    // Step 3: Bundle into a single composite pattern
    let pattern_hv = HdcVector::bundle(&temporal);

    // Step 4: Store as Signal with Kind::Pattern
    Signal::builder()
        .kind(Kind::Pattern)
        .hdc_fingerprint(pattern_hv)
        .domain(domain.clone())
        .body(Body::PatternMetadata {
            observation_count: observations.len(),
            temporal_span: observations.last().map(|o| o.timestamp)
                .zip(observations.first().map(|o| o.timestamp))
                .map(|(end, start)| end - start),
            outcome: outcome.cloned(),
        })
        .score(Score {
            confidence: outcome.map(|o| o.accuracy.accuracy).unwrap_or(0.5),
            novelty: 0.5,  // starts at baseline
            utility: 0.0,  // accumulates from successful predictions
            reputation: 0.0,
            coherence: 1.0, // patterns are self-consistent by construction
        })
        .build()
}
```

### Role-Filler Composition (BIND)

The BIND operation associates a concept ("price", "build_time", "source_reliability") with a specific value. It uses XOR because XOR is its own inverse -- given the composite and the role, you can recover the filler:

```rust
/// BIND: associate role with filler via XOR.
///
/// Properties:
///   - Self-inverse: BIND(BIND(role, filler), role) = filler
///   - Dissimilar to components: sim(BIND(a,b), a) ≈ 0.5 (random)
///   - Associative: BIND(a, BIND(b, c)) = BIND(BIND(a, b), c)
///   - Cost: ~2ns (XOR 160 u64 words on AVX-512)
///
/// Example:
///   role = codebook.price_role          // "this is a price observation"
///   filler = codebook.price_codebook.encode(3245.50)  // "the price is 3245.50"
///   binding = role.xor(&filler)         // "price = 3245.50"
pub fn bind(role: &HdcVector, filler: &HdcVector) -> HdcVector {
    role.xor(filler)
}
```

### Temporal Encoding (PERMUTE)

The PERMUTE operation encodes sequence position. A single bit rotation distinguishes "observation at time 0" from "observation at time 1". This creates shift-sensitive representations where order matters:

```rust
/// PERMUTE: encode temporal position via bit rotation.
///
/// Properties:
///   - Position-sensitive: PERM(a, 0) ≠ PERM(a, 1) (dissimilar)
///   - Reversible: PERM(PERM(a, k), -k) = a
///   - Cheap: ~1ns (bitwise rotate of 160 u64 words)
///
/// A temporal pattern "RSI rose from 30 to 70 over 3 ticks":
///   pattern = BUNDLE(PERM(rsi_30, 0), PERM(rsi_50, 1), PERM(rsi_70, 2))
///
/// This is DIFFERENT from BUNDLE(rsi_30, rsi_50, rsi_70) which loses ordering.
pub fn permute(vector: &HdcVector, position: u32) -> HdcVector {
    vector.rotate_left(position as usize)
}
```

### Quantized Value Encoding (Thermometer Construction)

Continuous values are encoded into HDC vectors using thermometer construction -- adjacent levels have high similarity, distant levels have low similarity. This preserves ordinal relationships:

```rust
/// Thermometer construction for numeric encoding.
///
/// Level vectors are generated by progressive bit flipping:
///   level_0 = random base vector
///   level_k = level_{k-1} with (dim / 2*n_levels) bits flipped
///
/// Result: sim(level_k, level_{k+1}) ≈ 1 - 1/(2*n_levels)
///         sim(level_0, level_{n-1}) ≈ 0.5 (nearly random)
///
/// For n_levels=64, adjacent levels have similarity ~0.992.
/// This means encode(3.0) is much more similar to encode(4.0)
/// than to encode(100.0) -- ordinal relationships are preserved.
///
/// Location: `crates/roko-primitives/src/hdc/codebook.rs`
pub struct QuantizedCodebook {
    levels: Vec<HdcVector>,  // n_levels vectors, each 10,240 bits
    min: f64,
    max: f64,
    n_levels: usize,  // default: 64
}

impl QuantizedCodebook {
    pub fn encode(&self, value: f64) -> HdcVector {
        let normalized = ((value - self.min) / (self.max - self.min))
            .clamp(0.0, 1.0);
        let level_f = normalized * (self.n_levels - 1) as f64;
        let lower = level_f.floor() as usize;
        let upper = (lower + 1).min(self.n_levels - 1);
        let weight = level_f - lower as f64;

        // Weighted interpolation between adjacent level vectors
        self.levels[lower].weighted_bundle(&self.levels[upper], 1.0 - weight, weight)
    }
}
```

---

## 3. Cross-Domain Resonance

The deepest value of HDC encoding: structural analogies across domains are discovered automatically via `query_similar()`. When a coding pattern "high test failure rate in auth module" and a chain pattern "high volatility in ETH/USDC pool" are encoded using the same algebra, both encode the abstract structure `BIND(high_uncertainty, critical_subsystem)`. Their HDC vectors will have Hamming similarity above the resonance threshold (0.526 = 5 sigma above chance for 10,240-bit vectors).

```rust
/// Cross-domain resonance detection.
///
/// This is not a special mechanism -- it falls out naturally from
/// Store.query_similar() when the domain filter is removed.
///
/// The resonance threshold (0.526) derives from:
///   - Random 10,240-bit vectors have expected similarity 0.500
///   - Standard deviation: sqrt(0.25 / 10240) ≈ 0.00494
///   - 5-sigma threshold: 0.500 + 5 * 0.00494 ≈ 0.525
///   - We use 0.526 (slightly above) for p < 1e-7 significance
///
/// When similarity exceeds this threshold between Signals from
/// different domains, the system has discovered a structural
/// analogy that was not programmed -- it emerged from the algebra.
///
/// Location: `crates/roko-primitives/src/hdc/resonance.rs`
pub struct ResonanceDetector {
    threshold: f64,  // default: 0.526
}

impl ResonanceDetector {
    /// Detect cross-domain resonance for a new Signal.
    ///
    /// Queries Store with domain filter EXCLUDED to find
    /// structurally similar patterns from other domains.
    pub async fn detect(
        &self,
        signal: &Signal,
        store: &dyn StoreProtocol,
    ) -> Vec<ResonanceHit> {
        let results = store.query_similar(
            &signal.hdc_fingerprint(),
            QueryOptions {
                threshold: self.threshold,
                exclude_domain: Some(signal.domain()),  // cross-domain only
                max_results: 10,
            },
        ).await;

        results.into_iter()
            .map(|(sim, matched_signal)| ResonanceHit {
                source: signal.id(),
                target: matched_signal.id(),
                source_domain: signal.domain(),
                target_domain: matched_signal.domain(),
                similarity: sim,
            })
            .collect()
    }
}
```

This is the "niche construction" thesis realized: patterns from one domain (chain trading) can inform predictions in another domain (software engineering) when they share structural similarity. The knowledge transfer costs ~13ns per comparison -- pure bitwise operations on the Signal's existing HDC fingerprint.

---

## 4. Evolutionary Dynamics: The Replicator Equation

Patterns compete for attention (and thus survival) via the replicator equation. This is not metaphorical -- it is a direct implementation of evolutionary dynamics where the fitness function is predictive accuracy:

```rust
/// Replicator dynamics for pattern fitness.
///
/// The replicator equation (Taylor & Jonker 1978):
///   dw_i/dt = w_i * (f_i - f_bar)
///
/// where:
///   w_i = weight (attention allocation) for pattern i
///   f_i = fitness of pattern i (= recent predictive accuracy)
///   f_bar = average fitness across all patterns
///
/// Patterns with above-average accuracy grow.
/// Patterns with below-average accuracy shrink.
/// The population self-organizes into an optimal ensemble.
///
/// In unified terms: this IS demurrage economics.
/// A pattern's "balance" is its weight.
/// "Retrieval" (successful prediction) increases balance.
/// "Holding cost" (demurrage) decreases balance over time.
/// The replicator equation IS the rate of change.
///
/// Location: `crates/roko-primitives/src/hdc/metabolism.rs`
pub fn replicator_update(
    patterns: &mut [PatternSignal],
    dt: f64,
) {
    // Compute average fitness
    let total_weight: f64 = patterns.iter().map(|p| p.weight).sum();
    let avg_fitness: f64 = patterns.iter()
        .map(|p| p.weight * p.fitness())
        .sum::<f64>() / total_weight;

    // Update weights via replicator equation
    for pattern in patterns.iter_mut() {
        let dw = pattern.weight * (pattern.fitness() - avg_fitness) * dt;
        pattern.weight = (pattern.weight + dw).max(0.0);
    }

    // Normalize to maintain total attention budget
    let new_total: f64 = patterns.iter().map(|p| p.weight).sum();
    if new_total > 0.0 {
        for pattern in patterns.iter_mut() {
            pattern.weight /= new_total;
        }
    }
}
```

### Connection to Demurrage

The replicator equation IS the mechanism behind demurrage for pattern Signals. The mapping:

| Demurrage concept | Replicator concept | Formula |
|---|---|---|
| Balance | Weight (w_i) | Attention allocation |
| Holding cost | Death rate | Proportional to (f_bar - f_i) when f_i < f_bar |
| Retrieval reinforcement | Reproduction | Proportional to (f_i - f_bar) when f_i > f_bar |
| Minimum balance (pruning) | Extinction threshold | Pattern removed when w_i < epsilon |
| Fitness | Predictive accuracy | EMA of recent predictions vs outcomes |

This unification means we do not need a separate "signal metabolism" subsystem. Demurrage already implements the replicator equation. Pattern competition is just what demurrage LOOKS LIKE when applied to `Kind::Pattern` Signals in Store.

---

## 5. Hebbian Learning via Oja's Rule

Pattern confidence is updated via Oja's rule -- a stable variant of Hebbian learning that prevents runaway weight growth. In unified terms, this is the predict-publish-correct feedback for pattern Signals:

```rust
/// Oja's rule for pattern confidence (Hebbian learning).
///
/// Standard Hebb: Δw = η * x * y (unstable, weights explode)
/// Oja's variant: Δw = η * y * (x - y * w) (stable, bounded)
///
/// In pattern terms:
///   w = pattern's confidence score
///   x = pattern's prediction value
///   y = actual outcome value
///   η = learning rate (0.01 to 0.05)
///
/// The (- y * w) term creates a self-normalizing property:
/// weights converge to the first principal component of the
/// input-output correlation matrix. This IS learning which
/// patterns are most predictive.
///
/// Location: `crates/roko-learn/src/learning/hebbian.rs`
pub fn oja_update(
    pattern: &mut Signal,
    prediction: f64,
    outcome: f64,
    learning_rate: f64,
) {
    let w = pattern.score().confidence;
    let delta = learning_rate * outcome * (prediction - outcome * w);
    let new_confidence = (w + delta).clamp(0.0, 1.0);

    pattern.update_score(|score| {
        score.confidence = new_confidence;
    });
}
```

---

## 6. Fisher's Fundamental Theorem as a Loop

Fisher's fundamental theorem of natural selection states: "The rate of increase in fitness of any organism at any time is equal to its genetic variance in fitness at that time." In pattern terms:

> The rate at which the pattern ensemble improves = the variance in predictive accuracy across patterns.

This is a **Loop** -- the system improves faster when it has diverse patterns with varying fitness, because the replicator equation amplifies the fittest while eliminating the least fit. Homogeneity (all patterns similar) halts improvement.

```rust
/// Fisher's fundamental theorem as a Loop observable.
///
/// Rate of improvement = Var(fitness) across all pattern Signals.
///
/// This creates a direct incentive for pattern diversity:
///   - If all patterns have similar fitness -> Var ≈ 0 -> no improvement
///   - If patterns vary widely in fitness -> high Var -> rapid improvement
///
/// The Dream cycle's REM phase generates novel patterns (mutations)
/// that increase variance, while NREM consolidation removes noise.
/// Together they maximize improvement rate.
///
/// Observable via Lens: watch this metric to know if the pattern
/// ensemble is stagnating (low variance) or actively improving (high variance).
pub fn fisher_improvement_rate(patterns: &[PatternSignal]) -> f64 {
    let fitnesses: Vec<f64> = patterns.iter().map(|p| p.fitness()).collect();
    let mean = fitnesses.iter().sum::<f64>() / fitnesses.len() as f64;
    fitnesses.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / fitnesses.len() as f64
}
```

---

## 7. Red Queen Pressure via Demurrage

The Red Queen hypothesis (Van Valen 1973): "It takes all the running you can do, to keep in the same place." In pattern terms: the environment changes, so patterns must continuously improve their predictions just to maintain their current fitness. A pattern that was accurate yesterday may be useless today if the market regime or codebase structure changed.

Demurrage implements Red Queen pressure automatically:

```
Pattern balance decays at rate = base_decay_rate * (1 - recent_retrieval_rate)

If a pattern is NOT being retrieved (not useful for current queries):
  -> balance decays toward zero
  -> eventually pruned from Store

If a pattern IS being retrieved but predictions are wrong:
  -> retrieval gives temporary boost
  -> but predict-publish-correct reduces confidence
  -> lower confidence = lower VCG bid weight = less attention
  -> replicator dynamics reduce weight
  -> eventually overtaken by better-predicting patterns

The only stable strategy: continuously predict correctly.
```

This means the pattern Store is self-maintaining. No manual curation needed. Outdated patterns fade. Novel patterns compete. The ensemble tracks the environment automatically.

---

## 8. Dream Consolidation as Pattern Evolution

The Dream cycle (Delta frequency, offline consolidation) operates directly on pattern Signals in Store. It is the evolutionary mechanism that creates novelty:

```rust
/// Dream consolidation for pattern Signals.
///
/// NREM replay: High-value patterns are replayed against the current
/// causal model. Agreement strengthens reliability. Disagreement weakens.
///
/// REM recombination: Novel patterns are generated by combining
/// existing patterns with random perturbation (mutation).
///
/// Pruning: Patterns with low reliability AND sufficient observations
/// are removed from Store.
///
/// This maps to biological sleep functions:
///   NREM = memory consolidation (Rasch & Born 2013)
///   REM = creative recombination (Lacaux et al. 2021)
///   Pruning = synaptic homeostasis (Tononi & Cirelli 2006)
///
/// Location: `crates/roko-dreams/src/consolidation.rs`
pub async fn dream_consolidation(store: &mut dyn StoreProtocol) {
    // NREM: replay and validate high-value patterns
    let high_value = store.query(Query::by_kind(Kind::Pattern)
        .order_by(ScoreAxis::Utility, Descending)
        .limit(100)
    ).await;

    for pattern in high_value {
        let still_valid = validate_against_causal_model(&pattern).await;
        if still_valid {
            store.reinforce(&pattern.id(), 0.05).await; // +5% reliability
        } else {
            store.decay(&pattern.id(), 0.10).await;     // -10% reliability
        }
    }

    // REM: generate novel recombinations
    let candidates: Vec<Signal> = store.query(Query::by_kind(Kind::Pattern)
        .min_score(ScoreAxis::Confidence, 0.5)
        .sample(20)
    ).await;

    for pair in candidates.chunks(2) {
        if pair.len() == 2 {
            let a = &pair[0].hdc_fingerprint();
            let b = &pair[1].hdc_fingerprint();
            // Recombine + mutate: XOR + rotation = novel pattern
            let novel = a.xor(b).permute(1);
            let novel_signal = Signal::builder()
                .kind(Kind::Pattern)
                .hdc_fingerprint(novel)
                .score(Score { confidence: 0.2, ..Default::default() })
                .lineage(vec![pair[0].id(), pair[1].id()])
                .build();
            store.put(novel_signal).await;
        }
    }

    // Prune: remove unreliable patterns with sufficient observation history
    let unreliable = store.query(Query::by_kind(Kind::Pattern)
        .max_score(ScoreAxis::Confidence, 0.3)
        .min_evaluation_count(10)
    ).await;

    for pattern in unreliable {
        store.prune(&pattern.id()).await;
    }
}
```

---

## What This Enables

1. **Store-native pattern recognition**: No separate pattern subsystem. Patterns are Signals. Pattern matching is `query_similar()`. Pattern competition is demurrage. The existing Store infrastructure handles everything.

2. **Cross-domain transfer at ~13ns**: Structural analogies between domains (chain, coding, research) are discovered automatically by querying without domain filter. No explicit cross-domain translation needed.

3. **Self-organizing ensembles**: The replicator equation + demurrage creates a pattern ecosystem that tracks environmental changes without manual curation. Fisher's theorem guarantees improvement as long as diversity is maintained.

4. **Predictive memory**: Patterns are not static records -- they are living predictions that must earn their retention through predictive utility. This makes the knowledge store inherently forward-looking.

5. **Biologically-inspired creativity**: Dream consolidation generates novel pattern combinations (REM) and validates existing ones (NREM), mirroring the creativity and memory consolidation functions of biological sleep.

---

## Feedback Loops

| Loop | Mechanism | Timescale |
|---|---|---|
| **Oja's rule** | Prediction accuracy -> confidence update | Per-prediction (Gamma, ~5-15s) |
| **Replicator dynamics** | Average fitness vs individual -> weight redistribution | Per-tick (continuous) |
| **Demurrage decay** | Time without retrieval -> balance reduction | Per-tick (continuous) |
| **Retrieval reinforcement** | Successful query_similar hit -> balance boost | Per-retrieval |
| **Dream consolidation** | NREM validation + REM recombination + pruning | Delta frequency (offline, hours) |
| **Fisher improvement** | Fitness variance -> rate of ensemble improvement | Observable (Lens) |

---

## Open Questions

1. **Optimal population size**: How many patterns should the Store maintain per domain? Too few = insufficient coverage. Too many = diluted attention and slow query_similar. Current heuristic: 100K per domain (100K * 13ns = 1.3ms for full scan). Is adaptive sizing better?

2. **Recombination quality**: REM recombination generates random XOR + permute combinations. Most are noise. Should the Dream cycle use a more guided recombination strategy (e.g., combining patterns that are moderately similar but from different temporal contexts)?

3. **Multi-timescale patterns**: Current encoding is flat (all observations bundled equally). Should there be hierarchical patterns -- patterns of patterns -- that capture multi-timescale structure? The Graph-of-Graphs composability suggests this should work.

4. **Codebook sharing vs isolation**: Deterministic codebooks (from domain seed) ensure all agents share a vector space. But this means codebook evolution requires coordinated updates. Should there be a mechanism for codebook versioning and migration?

---

## Implementation Tasks

- [ ] Verify that Signal's existing HDC fingerprint field (`crates/roko-primitives/src/hdc.rs`) supports the full role-filler-permute-bundle algebra
- [ ] Add `Kind::Pattern` variant to Signal's Kind enum if not already present
- [ ] Wire `Store.query_similar()` in `crates/roko-neuro/src/` to support cross-domain queries (domain filter = None)
- [ ] Implement `QuantizedCodebook` with thermometer construction in `crates/roko-primitives/src/hdc/codebook.rs`
- [ ] Add domain-specific codebooks (chain, coding, research) with deterministic seed generation
- [ ] Implement replicator dynamics update as part of demurrage tick in `crates/roko-fs/src/gc.rs`
- [ ] Add Oja's rule confidence update to the predict-publish-correct feedback path in `crates/roko-learn/`
- [ ] Wire Dream consolidation (`crates/roko-dreams/`) to operate on `Kind::Pattern` Signals specifically
- [ ] Add Fisher improvement rate as a Lens observable in `crates/roko-cli/src/tui/`
- [ ] Implement ResonanceDetector with configurable threshold in `crates/roko-primitives/src/hdc/resonance.rs`
