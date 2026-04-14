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

## Implementation Details: Rolling Window State Machine

### State Machine for `should_inject()`

The `ContrarianTracker` operates as a two-state machine:

```
                contrarian_rate >= 0.15
    ┌──────────────────────────────────────────────┐
    │                                              │
    ▼                                              │
 [PASSIVE] ──── contrarian_rate < 0.15 ────► [INJECTING]
    ▲                                              │
    │                                              │
    └──── contrarian_rate >= 0.15 after inject ────┘
```

- **PASSIVE**: Standard retrieval. The contrarian rate across the rolling window is at or above 15%. No forced injection needed.
- **INJECTING**: The contrarian rate has fallen below 15%. Every retrieval call checks the tracker and forces contrarian injection until the rate recovers.

The state is implicit — there is no explicit state enum. The `should_inject()` method computes the current rate and returns `true` when it falls below the threshold. This means the tracker is stateless between calls: it derives its behavior purely from the ring buffer contents.

```rust
impl ContrarianTracker {
    pub fn should_inject(&self, current_tick: u64) -> bool {
        let window_start = current_tick.saturating_sub(self.window_size as u64);

        // Count events within the window
        let (total, contrarian_count) = self.window.iter()
            .filter(|e| e.tick >= window_start)
            .fold((0usize, 0usize), |(t, c), e| {
                (t + 1, if e.was_contrarian { c + 1 } else { c })
            });

        // Bootstrap: force injection when insufficient data
        if total < 10 {
            return total % 7 == 0;  // inject roughly 1 in 7 during bootstrap
        }

        let rate = contrarian_count as f64 / total as f64;
        rate < self.min_contrarian_fraction
    }
}
```

**Bootstrap behavior**: When the window contains fewer than 10 events (agent startup or after a long idle period), the tracker injects roughly 1 in 7 retrievals (14.3%, close to the 15% target). This prevents the early window from being 100% congruent, which would create a hole the tracker has to fill later.

### Marking Retrievals as Contrarian

After each retrieval, the caller records the result:

```rust
// In the retrieval pipeline (roko-compose or orchestrate.rs):
let needs_contrarian = tracker.should_inject(current_tick);

let results = if needs_contrarian {
    let contrarian_results = retrieve_contrarian(store, query, current_pad, limit);
    tracker.record(current_tick, true);
    contrarian_results
} else {
    let standard_results = retrieve_standard(store, query, current_pad, limit);
    // Check if standard results happen to include opposite-valence entries
    let has_natural_contrarian = standard_results.iter()
        .any(|r| is_opposite_valence(r.valence, current_pad.pleasure));
    tracker.record(current_tick, has_natural_contrarian);
    standard_results
};
```

**Natural contrarian detection**: When the agent's mood is near neutral, standard retrieval often returns a mix of positive and negative entries. These "natural" contrarian entries count toward the 15% quota, so the tracker doesn't force unnecessary injections.

### Ring Buffer Persistence

The `ContrarianTracker`'s ring buffer is part of the agent's runtime state, managed by the `DaimonState`:

```rust
pub struct DaimonState {
    pub pad: PadVector,
    pub confidence: f64,
    pub behavioral_state: BehavioralState,
    pub contrarian_tracker: ContrarianTracker,
    // ...
}
```

**Persistence**: The tracker serializes with the rest of `DaimonState` to the executor snapshot (`.roko/state/executor.json`). On `--resume`, the tracker's ring buffer is restored, so the contrarian rate computation resumes from where it left off.

```rust
impl Serialize for ContrarianTracker {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Serialize the window as a Vec<(u64, bool)> for compactness
        let events: Vec<(u64, bool)> = self.window.iter()
            .map(|e| (e.tick, e.was_contrarian))
            .collect();
        events.serialize(serializer)
    }
}
```

**Crash recovery**: If the agent crashes without a snapshot, the tracker starts empty. The bootstrap logic (inject 1 in 7) handles this gracefully — the agent won't experience a contrarian drought during recovery.

**Snapshot size**: At window_size=200, the tracker stores at most 400 events (2x window for the trim buffer). Each event is 9 bytes (u64 tick + bool). Total: ~3.6 KB — negligible in the snapshot.

---

## Inverted PAD Retrieval: NeuroStore Query API

### PAD-Based Queries

The contrarian retrieval path needs to query the NeuroStore (the knowledge layer's storage backend) with an inverted PAD vector. This requires the NeuroStore to support affect-weighted queries:

```rust
/// Trait that the NeuroStore must implement for affect-weighted retrieval.
pub trait AffectWeightedQuery {
    /// Query entries that are semantically relevant AND emotionally congruent
    /// with the provided PAD vector.
    ///
    /// The scoring formula is:
    ///   score = alpha * semantic_similarity + (1 - alpha) * emotional_congruence
    ///
    /// where emotional_congruence = 1.0 - euclidean_distance(entry.pad, query_pad) / max_distance
    fn query_with_affect(
        &self,
        query_embedding: &[f32],
        pad: &PadVector,
        limit: usize,
    ) -> Vec<ScoredEntry>;
}

pub struct ScoredEntry {
    pub content_hash: ContentHash,
    pub semantic_score: f64,
    pub emotional_score: f64,
    pub combined_score: f64,
    pub valence: f64,  // the entry's pleasure dimension
}
```

**Blending factor `alpha`**: Default 0.7 (semantic similarity dominates). During contrarian injection, alpha drops to 0.5 to give emotional congruence (with the inverted PAD) more weight. This ensures the contrarian entries are emotionally opposite but still semantically relevant to the task.

### Retrieval Algorithm

```
fn retrieve_contrarian(store, query_embedding, current_pad, limit):
    1. Invert the PAD vector:
       inverted = PadVector {
           pleasure: -current_pad.pleasure,
           arousal:  current_pad.arousal,      // keep arousal
           dominance: -current_pad.dominance,
       }

    2. Query NeuroStore with inverted PAD, alpha=0.5:
       candidates = store.query_with_affect(query_embedding, inverted, limit * 3)

    3. Filter: discard candidates with |valence - current_pad.pleasure| < 0.10
       (these are too close to congruent to serve as contrarian)

    4. Re-rank remaining candidates by combined_score

    5. Return top `limit` entries
```

### Cost Model: Deterministic Filter, No Extra LLM Call

Contrarian retrieval does **not** invoke an additional LLM call. It is a deterministic operation against the local NeuroStore:

| Operation | Type | Latency |
|---|---|---|
| PAD inversion | Arithmetic (3 multiplications) | < 1 us |
| NeuroStore query | HNSW ANN search + PAD distance | 1-5 ms |
| Filtering + re-ranking | In-memory sort | < 0.1 ms |

Total overhead per contrarian injection: 1-5 ms. Over a 200-tick window with ~30 injections (15%), the cumulative overhead is 30-150 ms — negligible compared to LLM inference time (seconds per turn).

---

## Edge Cases

### Insufficient Opposite-Valence Entries

A young agent or an agent in a domain where outcomes are consistently positive (or consistently negative) may have few opposite-valence entries in the NeuroStore.

```rust
fn retrieve_contrarian_with_fallback(
    store: &dyn AffectWeightedQuery,
    query_embedding: &[f32],
    current_pad: &PadVector,
    limit: usize,
) -> Vec<ScoredEntry> {
    let contrarian = retrieve_contrarian(store, query_embedding, current_pad, limit);

    if contrarian.len() >= limit / 2 {
        return contrarian;  // Sufficient contrarian entries
    }

    // Fallback: retrieve high-arousal entries regardless of valence.
    // These are at least "salient" — they carry emotional weight
    // even if they aren't opposite-valence.
    let salient = store.query_by_arousal_threshold(
        query_embedding,
        0.5,  // minimum arousal
        limit - contrarian.len(),
    );

    let mut combined = contrarian;
    combined.extend(salient);
    combined.truncate(limit);
    combined
}
```

The fallback retrieves high-arousal (salient) entries regardless of valence direction. These aren't true contrarian entries, but they inject diversity because high-arousal entries tend to come from unusual or significant events — not the bland middle of the distribution.

### Forced Injection Timing

The `should_inject()` check happens at the start of each retrieval call. If the agent performs multiple retrievals within a single task (e.g., retrieval for context + retrieval for playbook matching), each retrieval independently checks the tracker. This means contrarian injection can happen on any retrieval call, not just the primary context retrieval.

**Ordering guarantee**: The tracker's ring buffer is append-only within a tick. Multiple retrievals within the same tick each see the same window state (the `record()` from earlier retrievals in the same tick is visible to later ones). This prevents a burst of retrievals from all triggering contrarian injection simultaneously.

### Window Wraparound After Long Idle Periods

If the agent is idle for longer than `window_size` ticks, the entire window expires. On the next retrieval, `should_inject()` finds an empty window and triggers bootstrap behavior (inject 1 in 7). This is correct — after a long idle period, the agent has no recent emotional context, so a fresh mix of congruent and contrarian retrieval is appropriate.

### Configuration Parameters

```rust
pub struct ContrarianConfig {
    /// Rolling window size in ticks.
    /// Larger windows smooth out short-term fluctuations.
    /// Range: [50, 1000]. Default: 200.
    pub window_size: usize,

    /// Minimum contrarian fraction within the window.
    /// Range: [0.05, 0.30]. Default: 0.15.
    pub min_contrarian_fraction: f64,

    /// Blending weight for contrarian vs. congruent in somatic queries.
    /// Range: [0.05, 0.30]. Default: 0.15.
    pub somatic_blend_weight: f64,

    /// Minimum valence difference to qualify as contrarian.
    /// Range: [0.05, 0.50]. Default: 0.10.
    pub min_valence_delta: f64,

    /// Alpha override for contrarian NeuroStore queries.
    /// Range: [0.3, 0.7]. Default: 0.5.
    pub contrarian_alpha: f64,
}

impl Default for ContrarianConfig {
    fn default() -> Self {
        Self {
            window_size: 200,
            min_contrarian_fraction: 0.15,
            somatic_blend_weight: 0.15,
            min_valence_delta: 0.10,
            contrarian_alpha: 0.5,
        }
    }
}
```

```toml
# roko.toml
[daimon.contrarian]
window_size = 200
min_contrarian_fraction = 0.15
somatic_blend_weight = 0.15
min_valence_delta = 0.10
contrarian_alpha = 0.5
```

### Test Criteria

| Test | Condition | Expected |
|---|---|---|
| Injection triggers at 14% rate | 200 ticks, 28 contrarian, next is non-contrarian | `should_inject()` returns true (28/200 = 14%) |
| No injection at 16% rate | 200 ticks, 32 contrarian | `should_inject()` returns false |
| Bootstrap injects ~1 in 7 | First 10 ticks | Approximately 1-2 injections |
| Natural contrarian counts toward quota | Standard retrieval includes opposite-valence entry | Recorded as contrarian=true |
| Tracker survives snapshot round-trip | Serialize, deserialize, check rate | Identical rate computation |
| Empty window after idle | All events older than window_size | Bootstrap behavior activates |
| Fallback to high-arousal entries | No opposite-valence entries in NeuroStore | Returns salient entries instead |
| Multiple retrievals per tick | 3 retrievals in tick 100 | Each sees prior records from same tick |
| Contrarian query is local-only | Full contrarian retrieval path | No LLM call, latency < 10 ms |

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

## Cross-References

- See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for somatic landscape query protocol
- See [09-mood-congruent-memory.md](./09-mood-congruent-memory.md) for full four-factor retrieval model
- See [02-alma-three-layer-temporal.md](./02-alma-three-layer-temporal.md) for PAD decay as loop-breaking mechanism
- See topic [03-dreams](../10-dreams/INDEX.md) for REM depotentiation
