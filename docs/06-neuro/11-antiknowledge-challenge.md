# AntiKnowledge and the Challenge Mechanism

> AntiKnowledge — validated negative knowledge about what is wrong — serves as the epistemic immune system of Neuro, with a confidence floor of 0.3, 0.5× demurrage, and a challenge mechanism that links refutations to the entries they contradict.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [01-six-knowledge-types.md](./01-six-knowledge-types.md)
**Key sources**:
- `bardo-backup/prd/04-memory/01b-grimoire-memetic.md` (AntiKnowledge definition, memetic evolution)
- `refactoring-prd/03-cognitive-subsystems.md` §1 (AntiKnowledge in six-type table)
- `crates/roko-neuro/src/lib.rs` (refuted_insight_id, refutation_evidence, refutation_warning())

---

## Abstract

Most knowledge systems focus on what is true. Neuro also tracks what is **false** — specifically, things that seem true but are not. This is AntiKnowledge: validated negative knowledge that prevents the agent from repeating known mistakes. "Moving to async doesn't always improve throughput." "Higher APY doesn't mean higher risk-adjusted returns." "More tests don't always mean better quality."

AntiKnowledge is the sixth knowledge type in Neuro's taxonomy. It has special decay behavior: a confidence floor of 0.3 (it never fully decays) and 0.5× demurrage rate (it decays at half speed). These properties reflect the insight that knowing what is wrong is **permanently valuable** — an agent that forgets its AntiKnowledge will inevitably re-discover and re-try failed approaches.

The challenge mechanism links each AntiKnowledge entry to the specific Insight or Heuristic it refutes, creating a bidirectional relationship: when the original entry is retrieved, the refutation warning is surfaced alongside it. This ensures that agents see both the claim and the counterevidence before acting.

---

## The Challenge Mechanism

### Creating an AntiKnowledge Entry

When an existing knowledge entry is contradicted by strong evidence — typically a gate failure or a direct observation that the claimed pattern does not hold — an AntiKnowledge entry is created:

```rust
let anti = KnowledgeEntry {
    id: format!("anti_{}", uuid::Uuid::new_v4()),
    kind: KnowledgeKind::AntiKnowledge,
    content: "Moving to async doesn't always improve throughput — \
              CPU-bound workloads actually degrade due to task scheduling overhead".to_string(),
    confidence: 0.6,
    refuted_insight_id: Some("ke_original_async_insight".to_string()),
    refutation_evidence: Some(
        "Benchmark showed 15% throughput regression when converting \
         CPU-bound computation from sync to async in roko-gate".to_string()
    ),
    source_episodes: vec!["ep_2026_04_10_gate_benchmark".to_string()],
    tags: vec!["rust".to_string(), "async".to_string(), "performance".to_string()],
    half_life_days: f64::INFINITY,  // never decays (confidence floor 0.3)
    ..Default::default()
};
```

### The Refutation Warning

When the refuted entry is retrieved, the AntiKnowledge entry's `refutation_warning()` method generates a warning string:

```rust
// From roko-neuro/src/lib.rs
impl KnowledgeEntry {
    pub fn refutation_warning(&self) -> Option<String> {
        if self.kind != KnowledgeKind::AntiKnowledge {
            return None;
        }
        let refuted_id = self.refuted_insight_id.as_deref()?.trim();
        if refuted_id.is_empty() {
            return None;
        }
        let evidence = self
            .refutation_evidence
            .as_deref()
            .unwrap_or(self.content.as_str())
            .trim()
            .trim_end_matches(|ch| matches!(ch, '.' | '!' | '?'));
        if evidence.is_empty() {
            return None;
        }
        Some(format!(
            "Previous insight {refuted_id} was wrong because {evidence}."
        ))
    }
}
```

This produces warnings like:

> "Previous insight ke_original_async_insight was wrong because Benchmark showed 15% throughput regression when converting CPU-bound computation from sync to async in roko-gate."

### Retrieval Integration

When Neuro assembles context for an agent, the retrieval pipeline includes a cross-check:

1. Retrieve relevant knowledge entries by topic
2. For each retrieved entry, check if any AntiKnowledge entry refutes it
3. If a refutation exists, attach the warning to the retrieved entry's context
4. The agent sees both the original claim and the counterevidence

This bidirectional linking ensures that contested knowledge is always presented with its challenge. The agent can then make an informed decision about whether to rely on the original claim, heed the warning, or investigate further.

---

## Memetic Evolution Context

### Dawkinsian Replicator Model

The original Grimoire (now Neuro) memetic evolution design (`bardo-backup/prd/04-memory/01b-grimoire-memetic.md`) models knowledge entries as memetic replicators with a fitness function:

```
W(E) = f × r × L
```

Where:
- **f** (fidelity): How accurately the entry is preserved when retrieved and re-applied
- **r** (fecundity): How frequently the entry is retrieved and used
- **L** (longevity): How long the entry persists (a function of tier and type half-life)

High-fitness entries are retrieved often, applied accurately, and persist a long time. This creates a selection pressure analogous to natural selection: useful knowledge replicates (gets confirmed, promoted) while useless knowledge dies off (decays, gets GC'd).

### Epistemic Parasites

A dangerous failure mode is the **epistemic parasite** — a knowledge entry with high fitness (frequently retrieved, persistently maintained) but **negative actual decision quality**. The entry appears useful by the metrics but actually harms the agent's performance.

Example: "Always use the most expensive model for safety-critical code" has high fitness (frequently retrieved for safety tasks, never explicitly contradicted because the expensive model always works) but negative value (wastes budget when cheaper models would suffice).

AntiKnowledge entries serve as the **immune system** against epistemic parasites. When an entry is identified as having high fitness but negative decision quality, an AntiKnowledge entry is created to challenge it. The confidence floor of 0.3 ensures that the immune response persists even after the parasite's fitness score would normally suppress challengers.

### Price Equation Diagnostics

The Price equation (Price 1970) decomposes the change in mean fitness of a population into selection and transmission components:

```
Δ(mean_fitness) = Cov(fitness, frequency) + E(Δfitness)
```

Applied to Neuro's knowledge base:
- **Selection component**: Knowledge entries that lead to positive outcomes are retrieved more (increasing fitness), creating a positive covariance between fitness and frequency
- **Transmission component**: Knowledge entries change during re-encoding (distillation may modify content), introducing transmission effects

The Price equation can be used as a **diagnostic** for knowledge base health:
- If the selection component is positive and the transmission component is near zero → the knowledge base is improving through natural selection
- If the transmission component is strongly negative → distillation is degrading knowledge quality
- If the selection component is near zero → the knowledge base has reached equilibrium (or the selection pressure is too weak)

---

## Special Decay Properties

### Confidence Floor: 0.3

AntiKnowledge entries have a confidence floor of 0.3. During decay:

```
new_confidence = max(0.3, confidence × decay_factor)
```

This ensures that AntiKnowledge entries always remain retrievable, even after extended periods of inactivity. The floor of 0.3 was chosen because:
- It is above the typical retrieval noise floor (0.1–0.2), ensuring the entry appears in query results
- It is below the confidence of most active knowledge entries (0.5–1.0), so AntiKnowledge does not dominate retrieval
- It matches the initial confidence of Dream-generated hypotheses (0.20–0.30), creating a natural equilibrium

### Half-Speed Demurrage: 0.5×

On the Korai chain, knowledge entries pay demurrage (1% annual confidence decay). AntiKnowledge entries pay 0.5× demurrage (0.5% annual decay), ensuring they persist on-chain as a public good. This slower decay reflects the social value of negative knowledge — it protects the entire collective from known false beliefs.

### Exemption from Garbage Collection

AntiKnowledge entries with confidence ≥ 0.3 are exempt from the standard GC threshold of 0.05. Because their confidence never drops below 0.3 (the floor), they are never garbage-collected. This is a deliberate design choice: the cost of maintaining AntiKnowledge entries (minimal storage) is far less than the cost of an agent re-discovering a known failed approach.

---

## Reactive Checking

Beyond proactive retrieval (surfacing AntiKnowledge when related entries are queried), Neuro supports **reactive checking** — automatically comparing new candidate entries against existing AntiKnowledge:

```
For each new candidate entry C:
    For each AntiKnowledge entry A:
        if HDC_similarity(C.hdc_vector, A.hdc_vector) > THRESHOLD:
            Flag C as potentially refuted
            Attach A's refutation_evidence to C
            Require additional confirmation before promoting C
```

This prevents the knowledge base from re-admitting entries that have been previously challenged. If a new Insight looks like a known-false belief (high HDC similarity to an AntiKnowledge entry), it must undergo additional scrutiny before being accepted.

---

## Academic Foundations

- Dawkins, R. (1976). *The Selfish Gene*. Oxford University Press. (Memetic replicator model)
- Price, G. R. (1970). "Selection and covariance." *Nature*, 227, 520–521. (Price equation)
- Popper, K. (1963). *Conjectures and Refutations: The Growth of Scientific Knowledge*. Routledge. (Falsificationism — knowledge grows by identifying what is wrong)
- Kahneman, D., & Tversky, A. (1979). "Prospect Theory." *Econometrica*, 47(2). (Loss aversion — negative evidence is weighted more heavily)

---

## Implementation Details: Confidence Floor Enforcement

### Where the floor is enforced

The confidence floor of 0.3 must be enforced in two places: the decay function and the garbage collector.

**In the decay function** (`roko-neuro/src/decay.rs` or equivalent):

```rust
/// Apply Ebbinghaus-style exponential decay to a knowledge entry's confidence.
///
/// Standard decay formula:
///   new_confidence = confidence * e^(-t / half_life)
///
/// For AntiKnowledge entries, a floor of 0.3 is enforced:
///   new_confidence = max(ANTI_KNOWLEDGE_FLOOR, confidence * e^(-t / half_life))
const ANTI_KNOWLEDGE_FLOOR: f64 = 0.3;

pub fn apply_decay(entry: &mut KnowledgeEntry, elapsed_days: f64) {
    let decay_factor = (-elapsed_days / entry.half_life_days).exp();
    let raw_confidence = entry.confidence * decay_factor;

    entry.confidence = if entry.kind == KnowledgeKind::AntiKnowledge {
        raw_confidence.max(ANTI_KNOWLEDGE_FLOOR)
    } else {
        raw_confidence
    };
}
```

**In the garbage collector** (`roko-neuro/src/gc.rs` or `roko-fs`):

```rust
const GC_THRESHOLD: f64 = 0.05;
const ANTI_KNOWLEDGE_FLOOR: f64 = 0.3;

/// Determine whether a knowledge entry should be garbage-collected.
///
/// Standard entries: GC if confidence < GC_THRESHOLD (0.05)
/// AntiKnowledge: never GC'd, because confidence never drops below 0.3
pub fn should_gc(entry: &KnowledgeEntry) -> bool {
    if entry.kind == KnowledgeKind::AntiKnowledge {
        return false; // Exempt: floor of 0.3 > GC threshold of 0.05
    }
    entry.confidence < GC_THRESHOLD
}
```

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `ANTI_KNOWLEDGE_FLOOR` | 0.3 | 0.1 - 0.5 | Below 0.2 risks falling below retrieval noise floor. Above 0.5 competes with active knowledge |
| `GC_THRESHOLD` | 0.05 | 0.01 - 0.10 | Standard entries below this are collected |

**Error handling**: `apply_decay` cannot fail. If `half_life_days` is `f64::INFINITY` (the default for AntiKnowledge), the decay factor is 1.0 and confidence is unchanged. If `half_life_days` is zero or negative (programming error), the decay factor is 0.0 and confidence drops to the floor immediately.

### Half-speed demurrage for on-chain AntiKnowledge

On the Korai chain, all knowledge entries pay demurrage — a continuous confidence tax that prevents the chain from filling with stale knowledge. AntiKnowledge pays half the standard rate.

**Formula**:

```
Standard demurrage:
  confidence(t) = confidence(0) * (1 - demurrage_rate)^t

AntiKnowledge demurrage:
  confidence(t) = max(FLOOR, confidence(0) * (1 - demurrage_rate * 0.5)^t)

Where:
  demurrage_rate = 0.01 per year (1% annual)
  FLOOR = 0.3
  t = time in years since last refresh
```

```rust
/// Compute demurrage for an on-chain knowledge entry.
///
/// Called during chain state transitions (block processing).
pub fn compute_demurrage(
    confidence: f64,
    kind: KnowledgeKind,
    years_elapsed: f64,
    annual_rate: f64, // 0.01 = 1%
) -> f64 {
    let effective_rate = if kind == KnowledgeKind::AntiKnowledge {
        annual_rate * 0.5
    } else {
        annual_rate
    };

    let decayed = confidence * (1.0 - effective_rate).powf(years_elapsed);

    if kind == KnowledgeKind::AntiKnowledge {
        decayed.max(ANTI_KNOWLEDGE_FLOOR)
    } else {
        decayed
    }
}
```

**Example**: An AntiKnowledge entry with initial confidence 0.8 after 10 years at half rate: `0.8 * 0.995^10 = 0.761`. After 100 years: `0.8 * 0.995^100 = 0.485` (above floor). At full rate it would be `0.8 * 0.99^100 = 0.265` (below floor, clamped to 0.3).

### Reactive checking against new knowledge candidates

When a new knowledge candidate is about to be promoted, the reactive checker compares it against existing AntiKnowledge entries.

```rust
/// Result of reactive AntiKnowledge checking.
pub struct ReactiveCheckResult {
    pub candidate_id: String,
    pub contradictions: Vec<Contradiction>,
    pub blocked: bool,
}

pub struct Contradiction {
    pub anti_entry_id: String,
    pub similarity: f32,
    pub evidence: String,
}

impl NeuroStore {
    /// Check a candidate entry against all AntiKnowledge entries.
    ///
    /// Trigger: called from ingest() before the entry is stored.
    /// If contradictions are found, the entry is flagged and requires
    /// additional confirmation before promotion.
    pub fn reactive_anti_check(
        &self,
        candidate: &KnowledgeEntry,
    ) -> ReactiveCheckResult {
        let candidate_hv = match candidate.hdc_vector.as_ref()
            .and_then(|b| HdcVector::from_bytes(b)) {
            Some(hv) => hv,
            None => return ReactiveCheckResult {
                candidate_id: candidate.id.clone(),
                contradictions: vec![],
                blocked: false,
            },
        };

        let mut contradictions = Vec::new();

        for anti_entry in self.entries_by_kind(KnowledgeKind::AntiKnowledge) {
            let anti_hv = match anti_entry.hdc_vector.as_ref()
                .and_then(|b| HdcVector::from_bytes(b)) {
                Some(hv) => hv,
                None => continue,
            };

            let sim = candidate_hv.similarity(&anti_hv);
            if sim > 0.526 {
                contradictions.push(Contradiction {
                    anti_entry_id: anti_entry.id.clone(),
                    similarity: sim,
                    evidence: anti_entry.refutation_evidence
                        .clone()
                        .unwrap_or_else(|| anti_entry.content.clone()),
                });
            }
        }

        let blocked = !contradictions.is_empty();

        ReactiveCheckResult {
            candidate_id: candidate.id.clone(),
            contradictions,
            blocked,
        }
    }
}
```

**Algorithm**: Compute Hamming similarity between the candidate's HDC vector and every AntiKnowledge entry. Flag any pair above 0.526. Blocked candidates are stored with `contested: true` and require additional confirmation before promotion.

### Epistemic parasite detection

An epistemic parasite has high fitness (frequently retrieved, persistent) but negative decision quality (outcomes are worse when it is used).

```rust
/// Compute fitness: W(E) = fidelity * fecundity * longevity
pub fn fitness(entry: &KnowledgeEntry, stats: &EntryStats) -> f64 {
    stats.preservation_rate * stats.retrievals_per_day * stats.age_days
}

/// Compute decision quality impact.
///
/// quality = mean(outcome_when_used) - mean(outcome_when_not_used)
/// Positive = helps. Negative = harms.
pub fn decision_quality(entry: &KnowledgeEntry, outcomes: &[Outcome]) -> f64 {
    let used: Vec<f64> = outcomes.iter()
        .filter(|o| o.entries_used.contains(&entry.id))
        .map(|o| o.score)
        .collect();
    let not_used: Vec<f64> = outcomes.iter()
        .filter(|o| !o.entries_used.contains(&entry.id))
        .map(|o| o.score)
        .collect();

    if used.is_empty() || not_used.is_empty() {
        return 0.0;
    }

    let mean_used = used.iter().sum::<f64>() / used.len() as f64;
    let mean_not_used = not_used.iter().sum::<f64>() / not_used.len() as f64;
    mean_used - mean_not_used
}

/// Detect parasites: high fitness but negative decision quality.
pub fn detect_parasites(
    entries: &[KnowledgeEntry],
    stats: &HashMap<String, EntryStats>,
    outcomes: &[Outcome],
) -> Vec<String> {
    entries.iter()
        .filter_map(|entry| {
            let s = stats.get(&entry.id)?;
            let fit = fitness(entry, s);
            let quality = decision_quality(entry, outcomes);
            if fit > 0.0 && quality < -0.1 {
                Some(entry.id.clone())
            } else {
                None
            }
        })
        .collect()
}
```

**Price equation diagnostics**:

```rust
/// Compute Price equation: delta_mean_fitness = Cov(fitness, frequency) + E(delta_fitness)
///
/// Returns (selection_component, transmission_component).
///   selection > 0 → healthy knowledge base improving through natural selection
///   transmission < 0 → distillation is degrading quality
///   selection < 0 → bad entries are being preferentially selected
pub fn price_equation_diagnostics(
    entries: &[(f64, f64, f64)], // (fitness, frequency, delta_fitness)
) -> (f64, f64) {
    let n = entries.len() as f64;
    if n == 0.0 {
        return (0.0, 0.0);
    }

    let mean_fitness: f64 = entries.iter().map(|(f, _, _)| f).sum::<f64>() / n;
    let mean_freq: f64 = entries.iter().map(|(_, r, _)| r).sum::<f64>() / n;
    let mean_delta: f64 = entries.iter().map(|(_, _, d)| d).sum::<f64>() / n;

    let e_product: f64 = entries.iter().map(|(f, r, _)| f * r).sum::<f64>() / n;
    let selection = e_product - mean_fitness * mean_freq;
    let transmission = mean_delta;

    (selection, transmission)
}
```

Diagnostics run during the Dreams consolidation cycle. Results are logged to `.roko/learn/price-diagnostics.jsonl`. An alert fires if `selection < -0.1` for three consecutive Dreams cycles.

### Automatic AntiKnowledge generation from gate failures

When a gate check fails and the failure can be attributed to specific retrieved knowledge entries, the system generates an AntiKnowledge candidate.

**Trigger condition**: Gate failure where (a) the agent retrieved specific knowledge entries, (b) the approach failed a gate, and (c) the failure relates to the retrieved knowledge.

```rust
/// Generate AntiKnowledge from a gate failure.
pub fn generate_anti_from_gate_failure(
    failed_gate: &GateResult,
    retrieved_entries: &[KnowledgeEntry],
    task_context: &str,
) -> Option<KnowledgeEntry> {
    if retrieved_entries.is_empty() {
        return None;
    }

    let primary_entry = retrieved_entries.iter()
        .max_by(|a, b| a.confidence.partial_cmp(&b.confidence)
            .unwrap_or(std::cmp::Ordering::Equal))?;

    let content = format!(
        "Applying '{}' led to gate failure: {}. Context: {}",
        primary_entry.content.chars().take(100).collect::<String>(),
        failed_gate.failure_reason,
        task_context,
    );

    Some(KnowledgeEntry {
        id: format!("anti_{}", uuid::Uuid::new_v4()),
        kind: KnowledgeKind::AntiKnowledge,
        content,
        confidence: 0.5,
        refuted_insight_id: Some(primary_entry.id.clone()),
        refutation_evidence: Some(failed_gate.failure_reason.clone()),
        source_episodes: vec![failed_gate.episode_id.clone()],
        tags: primary_entry.tags.clone(),
        half_life_days: f64::INFINITY,
        ..Default::default()
    })
}
```

**Creation pipeline**: Agent retrieves knowledge, produces output, gate fails, orchestrator calls `generate_anti_from_gate_failure()`, candidate enters `NeuroStore::ingest()` (subject to reactive checking). Initial confidence is 0.5 (single failure). Repeated failures of the same pattern increase confidence: 1 failure = 0.5, 2 = 0.6, 3+ = 0.7.

**Test criteria**:
- `apply_decay` on AntiKnowledge after 1000 days: confidence >= 0.3
- `should_gc` returns false for all AntiKnowledge entries
- `reactive_anti_check` flags candidates with similarity > 0.526 to existing AntiKnowledge
- `detect_parasites` identifies entries with fitness > 0 and quality < -0.1
- `price_equation_diagnostics` returns (0.0, 0.0) for empty input
- `generate_anti_from_gate_failure` returns None when no entries were retrieved
- `generate_anti_from_gate_failure` sets `refuted_insight_id` to the primary entry's ID

---

## Current Status and Gaps

**Implemented**: `KnowledgeKind::AntiKnowledge` variant. `refuted_insight_id` and `refutation_evidence` fields on `KnowledgeEntry`. `refutation_warning()` method.

**Missing**: Confidence floor enforcement (designed above). Half-speed demurrage (designed above; Korai precompile not implemented). Reactive checking (designed above). Epistemic parasite detection (designed above; needs outcome tracking). Price equation diagnostics (designed above). Automatic AntiKnowledge generation from gate failures (designed above; needs wiring into gate pipeline).

---

## Cross-references

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for all six knowledge types
- See [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for decay mechanics
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for how AntiKnowledge interacts with distillation
- See topic [11-safety](../11-safety/INDEX.md) for knowledge ingestion safety (quarantine pipeline)
