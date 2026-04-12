# 15% Contrarian Retrieval

> Preventing mood-congruent echo chambers: mandatory opposite-valence retrieval based on Bower's (1981) associative network theory.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md), [01-pad-vector.md](./01-pad-vector.md)
**Key sources**: `bardo-backup/prd/03-daimon/02-emotion-memory.md`, `refactoring-prd/09-innovations.md` §III, `refactoring-prd/03-cognitive-subsystems.md` §2

---

## Abstract

Mood-congruent memory is well-established in cognitive psychology: emotional states bias which memories are retrieved. An anxious person retrieves anxious memories; a confident person retrieves confident memories. For agents, this creates a dangerous positive feedback loop: a failing agent retrieves memories of past failures, which reinforces the negative emotional state, which biases retrieval further toward failures. Left unchecked, this loop produces depressive rumination — the agent becomes trapped in a self-reinforcing cycle of pessimism that prevents recovery.

The 15% contrarian retrieval mechanism breaks this loop by forcing a minimum fraction of retrieved context to come from memories with opposite emotional valence. An anxious agent always sees at least 15% of context from successful experiences. A overconfident agent always sees at least 15% of context from failures. This is implemented as a rolling window schedule across 200 ticks, not a fixed per-query quota.

The mechanism is grounded in Bower's (1981) associative network theory and validated by Emotional RAG (2024, arXiv:2410.23041), which showed that emotion-weighted retrieval with diversity controls outperforms purely semantic retrieval across multiple LLM backends.

---

## The Problem: Mood-Congruent Feedback Loops

### Bower's Associative Network Theory

Bower (1981) proposed that emotions function as nodes in an associative memory network. When an emotion is activated, it spreads activation to connected memories, concepts, and interpretations. The key finding: mood-congruent recall boosts accuracy by 5–30% for memories encoded under matching emotional states (Faul & LaBar 2022).

For natural organisms, this is mostly adaptive — an anxious animal should recall where predators lurk, and a calm animal should recall where food is abundant. But the mechanism has a failure mode: sustained negative mood creates a retrieval bias that reinforces itself.

### The Agent Failure Mode

```
Step 1: Agent fails a gate check
        → Appraisal: P: -0.10, A: +0.04, D: -0.08
        → Emotional state shifts toward Anxious

Step 2: Agent retrieves context for next task
        → Mood-congruent retrieval: anxious memories surface
        → Context is dominated by past failure cases

Step 3: Agent interprets current situation through failure lens
        → More cautious approach, lower confidence
        → May over-hedge or fail to attempt viable strategies

Step 4: Cautious approach produces another gate failure
        → Emotional state shifts further toward Anxious
        → Return to Step 2 with stronger negative bias
```

Without intervention, this loop converges on a stable but maladaptive equilibrium: the agent consistently retrieves negative context, consistently approaches tasks with excessive caution, and consistently underperforms. This is the computational analog of learned helplessness (Seligman 1967).

The same problem occurs in the positive direction. An overconfident agent retrieves only success cases, approaches tasks with insufficient caution, and may fail on edge cases it would have caught with a more balanced context.

---

## The Mechanism

### Rolling Window Schedule

The contrarian retrieval mechanism maintains a minimum 15% contrarian rate across any rolling 200-tick window. This is adaptive, not fixed-interval:

```rust
pub struct ContrarianTracker {
    /// Ring buffer of recent retrieval events.
    /// Each entry records whether the retrieval was contrarian.
    window: VecDeque<ContrarianEvent>,
    /// Maximum window size.
    window_size: usize,  // default: 200
    /// Minimum contrarian fraction.
    min_contrarian_fraction: f64,  // default: 0.15
}

struct ContrarianEvent {
    tick: u64,
    was_contrarian: bool,
}

impl ContrarianTracker {
    /// Determine whether the next retrieval should be forced contrarian.
    pub fn should_inject(&self, current_tick: u64) -> bool {
        // Prune events outside the window
        let window_start = current_tick.saturating_sub(self.window_size as u64);
        let recent: Vec<&ContrarianEvent> = self.window.iter()
            .filter(|e| e.tick >= window_start)
            .collect();

        if recent.is_empty() {
            return true;  // No data → inject to bootstrap
        }

        let contrarian_count = recent.iter()
            .filter(|e| e.was_contrarian)
            .count();
        let contrarian_rate = contrarian_count as f64 / recent.len() as f64;

        // Force contrarian if below minimum fraction
        contrarian_rate < self.min_contrarian_fraction
    }

    /// Record that a retrieval occurred.
    pub fn record(&mut self, tick: u64, was_contrarian: bool) {
        self.window.push_back(ContrarianEvent { tick, was_contrarian });
        // Trim to prevent unbounded growth
        while self.window.len() > self.window_size * 2 {
            self.window.pop_front();
        }
    }
}
```

### Contrarian Retrieval Implementation

When the tracker determines that contrarian injection is needed, the retrieval system inverts the pleasure dimension of the current PAD vector and uses it to find opposite-valence context:

```rust
fn retrieve_contrarian(
    store: &dyn KnowledgeStore,
    query_embedding: &[f32],
    current_pad: &PadVector,
    limit: usize,
) -> Vec<ScoredEntry> {
    // Invert pleasure and dominance, keep arousal
    // Arousal tracks salience — contrarian entries should still
    // be relevant (high arousal) even if emotionally opposite
    let inverted_pad = PadVector {
        pleasure: -current_pad.pleasure,
        arousal: current_pad.arousal,
        dominance: -current_pad.dominance,
    };

    // Query with inverted PAD for emotional congruence scoring
    store.query_with_affect(query_embedding, &inverted_pad, limit)
}
```

**Why invert pleasure and dominance but not arousal**: Arousal tracks salience — how important or urgent something is. Contrarian entries should still be relevant to the current level of urgency. An anxious agent retrieving contrarian context should get *important* positive memories (high arousal, high pleasure), not trivial ones (low arousal, high pleasure). The arousal dimension ensures the contrarian entries match the current level of engagement.

### Blending Contrarian with Congruent Context

The retrieval pipeline blends contrarian and congruent entries in the final context:

```
Phase 1: Candidate generation (3× overfetch via HNSW/ANN search)
Phase 2: Four-factor re-ranking (recency × importance × relevance × emotional congruence)
Phase 3: Contrarian injection (if tracker says inject)
          → Add opposite-valence entries to the candidate set
          → Re-rank the combined set
Phase 4: Testing effect (mark retrieved entries as accessed)
Phase 5: Return top-k results
```

The contrarian entries compete with congruent entries in the re-ranking phase. They're scored with the inverted PAD vector, which gives them high emotional congruence scores against the inverted mood. But they still need semantic relevance — an irrelevant contrarian entry won't survive re-ranking even with an emotional congruence boost.

---

## The 15% Target

### Why 15%?

The 15% minimum contrarian fraction is calibrated to be:

1. **Large enough to break feedback loops**: A 5% contrarian rate would be absorbed by the 85% majority context — the contrarian signal would be too weak to shift the agent's interpretation. At 15%, the contrarian context is a significant minority that creates productive tension in the reasoning.

2. **Small enough to preserve mood-congruent benefits**: Mood-congruent retrieval is adaptive in the common case. An agent facing a familiar type of failure *should* retrieve past failure cases — they contain relevant warnings and solutions. Reducing the congruent fraction below 85% would sacrifice this benefit.

3. **Empirically grounded**: Bower's (1981) findings show 5–30% accuracy boost from mood-congruent retrieval. The 15% contrarian rate preserves the core benefit (85% congruent) while creating a floor on diversity.

### Why a Rolling Window, Not Per-Query?

A per-query contrarian quota (e.g., "always return 15% opposite-valence entries in every retrieval") would be wasteful when the agent's mood is neutral. At neutral mood, there's no feedback loop to break — the retrieval is already balanced. The rolling window approach only forces contrarian injection when the cumulative contrarian rate drops below 15%, which happens primarily during sustained emotional states.

During a Struggling phase (30-50 ticks of sustained negative mood), the tracker will fire 5-8 contrarian injections. During an Engaged phase (neutral mood), the tracker rarely fires because the baseline retrieval already includes a mix of positive and negative context.

---

## Interaction with Somatic Landscape

The 15% contrarian mechanism also applies to somatic marker queries (see [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)). When querying the k-d tree for nearest neighbors, 15% of the returned markers are selected from the opposite-valence region:

```rust
fn query_contrarian(
    &self,
    strategy_coords: &[f64; 8],
    congruent_valence: f64,
    k: usize,
) -> ContrarianResult {
    // Find markers with opposite valence
    let all_neighbors = self.tree.nearest(strategy_coords, k * 5, &squared_euclidean);
    let contrarian: Vec<_> = all_neighbors.iter()
        .filter(|(_, marker)| marker.valence.signum() != congruent_valence.signum())
        .take(k)
        .collect();

    ContrarianResult {
        valence: contrarian.iter()
            .map(|(dist, m)| m.valence / (1.0 + dist))
            .sum::<f64>() / contrarian.len().max(1) as f64,
        count: contrarian.len(),
    }
}
```

This means the somatic landscape always presents a mixed signal: "This region generally feels positive, BUT there are cases where similar strategies failed." This prevents the somatic landscape from becoming a pure confirmation mechanism.

---

## Complementary Loop-Breaking Mechanisms

The 15% contrarian retrieval is one of three mechanisms that prevent emotional feedback loops:

| Mechanism | Level | How It Works |
|---|---|---|
| **15% contrarian retrieval** | Memory access | Forces opposite-valence context into retrieval results |
| **REM depotentiation** | Memory storage | Reduces arousal of emotionally charged memories during dreams |
| **PAD decay** | Affect state | Exponential decay (4h half-life) pulls mood toward neutral baseline |

These operate at different levels and timescales:

- **Contrarian retrieval** prevents the loop from forming in real-time (tick-level)
- **REM depotentiation** reduces the loop's fuel by calming emotionally charged memories (dream-cycle level, hours)
- **PAD decay** pulls the overall mood toward neutral, reducing the bias strength (hours to days)

All three are necessary. Without contrarian retrieval, the loop forms immediately. Without depotentiation, the loop's fuel accumulates across dreams. Without decay, the baseline mood drifts to an extreme.

---

## Mind Wandering as Spontaneous Contrarian Injection

The legacy specification (`02-emotion-memory.md`) includes a mind wandering mechanism: approximately every 200 ticks, the system retrieves a random high-arousal episode and re-appraises it in the current context. This serves as a spontaneous contrarian injection — the randomly retrieved episode is unlikely to match the current emotional state, providing a diversifying signal.

Mind wandering is a default mode network (DMN) analog. The DMN activates during periods of low cognitive demand and produces spontaneous retrievals that are emotionally charged but not task-relevant. For agents, this prevents emotional degeneration during idle periods and injects variability into the emotional landscape.

**Cost**: Zero. The retrieval is a local database query, and the re-appraisal uses the same deterministic appraisal rules as live events. No LLM call is required.

---

## Academic Foundations

- Bower, G.H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148.
- Blaney, P.H. (1986). "Affect and Memory: A Review." *Psychological Bulletin*, 99(2), 229–246.
- Faul, L. & LaBar, K.S. (2022). "Mood-Congruent Memory Revisited." *Psychological Review*.
- Emotional RAG. (2024). "Emotional RAG: Enhancing Role-Playing Agents through Emotional Retrieval." arXiv:2410.23041.
- Seligman, M.E.P. (1967). "Failure to escape traumatic shock." *Journal of Experimental Psychology*, 74(1), 1–9.
- Nietzsche, F. (1887). *On the Genealogy of Morals*. Second Essay. ("There exists a degree of rumination which is harmful and ultimately fatal to the living thing.")
- Walker, M.P. & van der Helm, E. (2009). "Overnight therapy? The role of sleep in emotional brain processing." *Psychological Bulletin*, 135(5), 731–748.

---

## Cross-references

- See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for somatic landscape query protocol
- See [09-mood-congruent-memory.md](./09-mood-congruent-memory.md) for full four-factor retrieval model
- See [02-alma-three-layer-temporal.md](./02-alma-three-layer-temporal.md) for PAD decay as loop-breaking mechanism
- See topic [03-dreams](../10-dreams/INDEX.md) for REM depotentiation
