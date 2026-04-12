# Pattern Discovery: Trigram Mining

> **Crate:** `roko-learn` · **Modules:** `pattern_discovery.rs`, `hdc_clustering.rs`
> **Wiring:** `LearningRuntime::record_completed_run()` → `PatternMiner::ingest_episode()`
> **Cross-references:** [00-episode-logger](00-episode-logger.md), [01-playbook-system](01-playbook-system.md), [15-collective-calibration-31x](15-collective-calibration-31x.md)

---

## Purpose

Pattern discovery mines recurring structural signals from the episode stream. The core technique is trigram mining: extracting every three-action subsequence from each episode's gate verdict sequence, counting how often each trigram appears across episodes, and surfacing those that exceed a support threshold as recurring patterns. These patterns are the intermediate tier in the three-tier memory hierarchy (episodes → patterns → playbook rules).

The module also provides cross-episode consolidation using HDC clustering: grouping structurally similar episodes into clusters, then extracting meta-patterns that describe common traits of each cluster.

---

## EpisodeView Trait

Pattern mining is decoupled from the concrete `Episode` type via a trait:

```rust
pub trait EpisodeView {
    /// Ordered slice of action kind labels recorded during the episode.
    fn actions(&self) -> &[String];
    /// Whether the episode reached a successful terminal state.
    fn succeeded(&self) -> bool;
}
```

This decoupling lets the miner work with any type that exposes an ordered list of action kinds and a success flag — including the canonical `Episode`, synthetic fixtures, and downstream replayers. In practice, the `LearningRuntime` wraps each `Episode` in an `EpisodeActions` adapter that extracts gate names from `gate_verdicts`:

```rust
struct EpisodeActions {
    actions: Vec<String>,   // ["compile", "test", "lint", "diff"]
    success: bool,
}

impl EpisodeActions {
    fn from_episode(ep: &Episode) -> Self {
        Self {
            actions: ep.gate_verdicts.iter().map(|v| v.gate.clone()).collect(),
            success: ep.success,
        }
    }
}
```

---

## Trigram Mining Algorithm

### Step 1: Ingest

For each episode, the miner extracts all three-action subsequences (trigrams):

```
Episode actions: ["read", "edit", "compile", "test", "lint"]

Trigrams:
  ("read", "edit", "compile")
  ("edit", "compile", "test")
  ("compile", "test", "lint")
```

Each trigram is hashed to a stable 64-bit signature using FNV-1a. The miner maintains a `BTreeMap<u64, TrigramStats>` keyed by signature:

```rust
struct TrigramStats {
    trigram: [String; 3],
    signature: u64,
    support: u32,          // distinct episodes containing this trigram
    first_seen_ms: i64,
    last_seen_ms: i64,
}
```

A trigram's support count is the number of distinct episodes that contain it (not the total number of occurrences across all episodes). This prevents a single long episode from inflating support counts.

### Step 2: Discover

After ingesting a batch, `PatternMiner::discover()` returns all trigrams whose support clears the configured thresholds:

```rust
pub struct PatternMiner {
    min_support: u32,       // minimum distinct episodes (default: 2)
    min_confidence: f32,    // minimum support/total ratio (default: 0.5)
    // ...
}
```

Each qualifying trigram becomes a `Pattern`:

```rust
pub struct Pattern {
    /// Stable string id ("trigram:<signature>").
    pub id: String,
    /// Deterministic 64-bit content hash of the trigram.
    pub signature: u64,
    /// Human-readable rendering (e.g. "read → edit → test").
    pub description: String,
    /// Number of distinct episodes containing this trigram.
    pub support_count: u32,
    /// support_count / total_episodes, clamped to [0.0, 1.0].
    pub confidence: f32,
    /// Unix ms of the first episode containing this trigram.
    pub first_seen_ms: i64,
    /// Unix ms of the most recent episode containing it.
    pub last_seen_ms: i64,
}
```

### Step 3: Promote

Patterns with sufficient support (typically ≥5 episodes) are candidates for promotion to playbook rules. See [01-playbook-system](01-playbook-system.md) for the promotion criteria.

---

## Why Trigrams?

| N-gram size | Properties |
|-------------|------------|
| Unigrams (1) | Too generic — "compile" appears in every episode |
| Bigrams (2) | Still generic — "edit→compile" is nearly universal |
| **Trigrams (3)** | Captures meaningful action patterns — "read→edit→test" vs "edit→compile→fix" |
| 4-grams (4) | Too specific — many unique sequences, insufficient support for pattern extraction |

Trigrams strike the right balance between specificity and support. They capture enough context to distinguish successful from unsuccessful action sequences, while remaining common enough to accumulate statistically significant support counts.

---

## HDC Clustering for Cross-Episode Consolidation

Beyond trigram mining, the module provides cross-episode structural analysis using HDC (Hyperdimensional Computing) clustering.

### k-Medoids Algorithm

The `hdc_clustering` module implements Partitioning Around Medoids (PAM) over 10,240-bit `HdcVector`s:

```rust
pub struct KMedoidsConfig {
    pub k: usize,              // number of clusters (default: 3)
    pub max_iterations: usize, // convergence limit (default: 100)
}
```

The algorithm:

1. **Initialize** — greedy farthest-first seeding: pick the point closest to the global centroid as the first medoid, then iteratively add the point maximizing its minimum distance to all existing medoids.
2. **Assign** — each point goes to the nearest medoid (distance = `1 − similarity` where similarity is HDC Hamming similarity).
3. **Update** — for each cluster, the member minimizing total intra-cluster distance becomes the new medoid.
4. Repeat 2-3 until medoids stabilize or `max_iterations` is reached.

### Cross-Episode Consolidation

The `CrossEpisodeConsolidator` groups episodes by their HDC fingerprints, then extracts meta-patterns from each cluster:

```
Episodes with HDC fingerprints
    │
    ▼
k-medoids clustering (k=3, HDC similarity)
    │
    ▼
For each cluster:
    ├── Identify common trigrams across cluster members
    ├── Compute cluster-level pass rate
    ├── Extract distinguishing features (files, roles, categories)
    └── Produce CrossEpisodeConsolidationReport
```

The consolidation report identifies structural groupings in the episode stream that may not be visible from individual trigram analysis. For example, a cluster of episodes that all involve cross-crate modifications and share a high failure rate suggests a systemic issue with cross-crate tasks, even if no single trigram captures this pattern.

---

## Operating Frequency

Pattern discovery runs at **every 20 episodes** — the slowest learning loop in the system. This frequency separation prevents oscillation: rapid pattern updates could cause playbook rules to be promoted and demoted on noisy short-term data, while infrequent updates ensure that patterns reflect stable, recurring phenomena.

```
Learning Loop Frequencies:
    ├── Cascade router:       every episode          (highest)
    ├── Gate thresholds:      every 5 episodes
    ├── Pattern discovery:    every 20 episodes       (lowest)
    └── Cross-episode:        on-demand or periodic
```

See [14-stability-mechanisms](14-stability-mechanisms.md) for the full frequency separation design.

---

## Integration with LearningRuntime

The `LearningRuntime` invokes pattern mining as step 8 of the `record_completed_run()` pipeline:

```
CompletedRunInput
    │
    ├── 1-7. Episode, costs, playbook, skills, metrics, experiments
    │
    ├── 8. PatternMiner::ingest_episode(EpisodeActions::from_episode(ep))
    │       → Updates trigram counters
    │       → If episode_count % 20 == 0: auto-discover
    │
    └── 9-10. CascadeRouter, CFactor
```

The auto-discover trigger runs `PatternMiner::discover()` every 20 episodes and feeds the results to the `CrossEpisodeConsolidator` for cluster-level analysis.

---

## Performance

| Operation | Complexity | Typical Time |
|-----------|-----------|--------------|
| Ingest one episode | O(n) where n = action count | < 1μs |
| Discover patterns | O(m) where m = unique trigrams | < 100μs for 1000 trigrams |
| HDC fingerprint comparison | O(1) — bit-parallel Hamming | ~50ns |
| k-medoids clustering | O(k × n × max_iter) | < 10ms for 200 episodes |

The entire pattern discovery pipeline adds negligible overhead to the per-episode processing path. The most expensive operation (k-medoids clustering) runs at the lowest frequency (every 20 episodes) and only when cross-episode consolidation is triggered.

---

## Practical Example

### Episode Stream

```
Episode 1: actions = ["read", "edit", "compile", "test"]     success=true
Episode 2: actions = ["read", "edit", "compile", "fix", "compile", "test"]  success=true
Episode 3: actions = ["edit", "compile", "test"]              success=true
Episode 4: actions = ["read", "edit", "compile", "test"]      success=true
Episode 5: actions = ["edit", "compile", "lint", "fix", "compile", "test"]  success=false
```

### Trigram Extraction

```
Episode 1: (read,edit,compile) (edit,compile,test)
Episode 2: (read,edit,compile) (edit,compile,fix) (compile,fix,compile) (fix,compile,test)
Episode 3: (edit,compile,test)
Episode 4: (read,edit,compile) (edit,compile,test)
Episode 5: (edit,compile,lint) (compile,lint,fix) (lint,fix,compile) (fix,compile,test)
```

### Support Counts

```
(read,edit,compile):    support=3  confidence=3/5=0.60  → PATTERN (above thresholds)
(edit,compile,test):    support=3  confidence=3/5=0.60  → PATTERN
(fix,compile,test):     support=2  confidence=2/5=0.40  → below confidence threshold
(edit,compile,fix):     support=1  confidence=1/5=0.20  → below support threshold
```

### Discovered Patterns

```
Pattern "trigram:0xA1B2C3": read → edit → compile
    support: 3 episodes, confidence: 0.60
    First seen: episode 1, Last seen: episode 4

Pattern "trigram:0xD4E5F6": edit → compile → test
    support: 3 episodes, confidence: 0.60
    First seen: episode 1, Last seen: episode 4
```

These two patterns capture the dominant successful action sequence: read the code, edit it, compile, test. This pattern can be promoted to a playbook rule that instructs agents to follow this read→edit→compile→test workflow.

---

## HDC Distance Metric

The HDC clustering module uses `1 − similarity` as the distance metric, where similarity is computed via Hamming distance on 10,240-bit vectors:

```
similarity(a, b) = 1 − (hamming_distance(a, b) / 10240)
distance(a, b) = hamming_distance(a, b) / 10240
```

Values:
- Identical vectors: distance = 0, similarity = 1.0
- Orthogonal vectors: distance ≈ 0.5, similarity ≈ 0.5
- Maximally different: distance = 1.0, similarity = 0.0

The 10,240-bit dimension provides high expressiveness: two vectors created from different seeds have expected similarity ≈ 0.5 (random baseline), while vectors created from similar content cluster well above 0.7.

---

## Relationship to Other Documents

- **[00-episode-logger](00-episode-logger.md)** — Episodes are the raw data stream that the miner consumes.
- **[01-playbook-system](01-playbook-system.md)** — Patterns with sufficient support are promoted to playbook rules.
- **[02-skill-library-voyager](02-skill-library-voyager.md)** — Patterns identify recurring sequences; skills capture the full procedures associated with successful sequences.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Frequency separation ensures pattern discovery runs at the appropriate cadence.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — Pattern counts contribute to the knowledge_growth component of the C-Factor.
