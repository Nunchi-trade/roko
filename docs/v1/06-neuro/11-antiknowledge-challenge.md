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

## Anti-Knowledge Epidemiology

### How Bad Knowledge Spreads

Bad knowledge behaves like a pathogen in an information system. Understanding its epidemiology is essential for building effective defenses. Drawing from both epistemology (Kuhn 1962, Proctor 2008) and data poisoning research (Zou et al. 2025, Koh & Liang 2017), we model knowledge corruption as an epidemic process.

### Infection Vectors

Knowledge corruption enters the system through five vectors, ordered by danger:

```rust
/// Classification of how bad knowledge enters the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfectionVector {
    /// Distillation error: LLM extracts incorrect insight from valid episodes.
    /// Most common. The episode data is correct but the pattern extraction is wrong.
    /// Example: "async always improves throughput" extracted from 3 episodes where
    /// async happened to correlate with (but not cause) better performance.
    DistillationError,

    /// Confirmation cascade: a wrong insight gets confirmed by coincidence,
    /// promoting it to Heuristic tier where it resists correction.
    /// Most dangerous. Self-reinforcing — each false confirmation makes
    /// the entry harder to dislodge.
    ConfirmationCascade,

    /// External injection: bad knowledge imported from mesh sync, backup restore,
    /// or Korai chain marketplace.
    /// Controlled by confidence discounting (0.50×-0.85×) and quarantine pipeline.
    ExternalInjection,

    /// Concept drift: knowledge that was correct when created becomes incorrect
    /// as the environment changes (API updates, library version changes, etc.).
    /// Not a bug — it's the natural staleness that Ebbinghaus decay is designed
    /// to handle. Dangerous only when tier is Persistent (5× half-life resists decay).
    ConceptDrift,

    /// Adversarial poisoning: deliberately crafted entries designed to mislead.
    /// Rare in single-agent mode. Relevant for mesh/chain scenarios.
    /// Requires active defense (see Knowledge Immune System below).
    AdversarialPoisoning,
}
```

### Epidemiological Model: SIR for Knowledge

Borrowing from epidemiology's SIR (Susceptible-Infected-Recovered) model, we model the knowledge base as a population of entries:

```
S(t) = entries that could become corrupted (Susceptible)
I(t) = entries that are currently incorrect (Infected)
R(t) = entries protected by AntiKnowledge challenges (Recovered/immune)

dS/dt = -β × S × I / N + ν × R     (new susceptibles from entry ingestion, waning immunity)
dI/dt = β × S × I / N - γ × I       (infection from confirmation cascade, recovery from gate failures)
dR/dt = γ × I - ν × R               (recovery via AntiKnowledge creation, immunity waning)

where:
  β = confirmation cascade rate (how quickly bad entries get confirmed)
  γ = detection rate (how quickly gate failures expose bad entries)
  N = total knowledge base size
  ν = immunity waning rate (how quickly AntiKnowledge loses effectiveness)
```

**Basic reproduction number R₀**:
```
R₀ = β / γ
```

If R₀ > 1, bad knowledge spreads faster than it's detected. If R₀ < 1, the knowledge immune system contains the infection. Neuro's design targets R₀ < 0.5 through aggressive gate checking (high γ) and confirmation cascade prevention (low β).

### Confirmation Cascade: The Most Dangerous Failure Mode

A confirmation cascade occurs when a bad entry happens to be in the context when a task succeeds for unrelated reasons:

```
1. Bad entry B is created with confidence 0.4 (Transient tier)
2. Agent retrieves B alongside 20 other entries for task T₁
3. Task T₁ succeeds (for reasons unrelated to B)
4. B gets confirmation boost: 0.4 × 1.5 = 0.6, promoted to Working tier
5. Agent retrieves B for task T₂ (B now has higher retrieval priority)
6. Task T₂ succeeds (again, unrelated to B)
7. B gets boost: 0.6 × 1.5 = 0.9, approaching Consolidated promotion
8. After 3+ successful tasks, B is Consolidated with confidence 0.9
9. B is now extremely difficult to demote (requires explicit contradiction)
```

**Mitigation**: Attribution analysis — track not just whether B was in the context, but whether B was *causally relevant* to the outcome.

```rust
/// Attribution analysis for knowledge confirmation.
///
/// Instead of blindly boosting all retrieved entries when a task succeeds,
/// estimate each entry's causal contribution to the outcome.
pub struct AttributionAnalysis {
    /// Influence score: estimated causal contribution of each retrieved entry.
    /// Range: 0.0 (no influence) to 1.0 (primary driver).
    pub influence_scores: HashMap<String, f64>,
    /// Method used for attribution.
    pub method: AttributionMethod,
}

#[derive(Debug, Clone, Copy)]
pub enum AttributionMethod {
    /// All retrieved entries get equal credit (current naive approach).
    EqualCredit,
    /// Entries matching the task domain/tags get more credit.
    TagRelevance,
    /// Influence function estimation: how would the outcome change
    /// if this entry were removed from context?
    /// Based on Koh & Liang (2017) influence functions.
    InfluenceEstimation,
    /// Data Shapley: each entry's marginal contribution to the outcome.
    /// Most accurate but computationally expensive (O(2^N) exact, O(N log N) approximate).
    /// Based on Ghorbani & Zou (2019).
    DataShapley,
}

/// Compute influence-weighted confirmation boost.
///
/// Instead of CONFIRMATION_BOOST = 1.5 for all entries,
/// apply boost proportional to estimated causal influence.
pub fn influence_weighted_boost(
    entry: &KnowledgeEntry,
    influence: f64,
    base_boost: f64, // CONFIRMATION_BOOST = 1.5
) -> f64 {
    let weighted_boost = 1.0 + (base_boost - 1.0) * influence;
    // influence = 0.0 → boost = 1.0 (no change)
    // influence = 0.5 → boost = 1.25
    // influence = 1.0 → boost = 1.5 (full boost)
    (entry.confidence * weighted_boost).min(1.0)
}
```

**References**: Koh, P.W. & Liang, P. (2017). "Understanding Black-box Predictions via Influence Functions." *ICML 2017*. Ghorbani, A. & Zou, J. (2019). "Data Shapley: Equitable Valuation of Data for Machine Learning." *ICML 2019*.

---

## Knowledge Immune System

### Proactive Defense Against Knowledge Corruption

The knowledge immune system operates at three levels, analogous to biological immunity:

### Level 1: Innate Immunity (Always Active)

Fast, non-specific defenses that run on every knowledge operation:

```rust
/// Innate immune checks applied to every entry during ingestion.
pub struct InnateImmunity {
    /// Bloom filter of known-bad entry fingerprints (HDC LSH).
    /// Entries matching this filter are flagged for quarantine.
    pub bad_entry_bloom: BloomFilter,
    /// Maximum confidence for new external entries. Default: 0.7.
    /// Prevents external entries from immediately reaching high tiers.
    pub max_external_confidence: f64,
    /// Minimum source diversity for tier promotion.
    /// Entry must be confirmed by episodes from N distinct sources. Default: 2.
    pub min_source_diversity: usize,
    /// Anomaly detector: flags entries whose HDC vector is statistically
    /// unusual compared to the existing knowledge base.
    pub anomaly_threshold: f64, // Default: 3.0 (standard deviations)
}

impl InnateImmunity {
    /// Screen an incoming entry. Returns quarantine disposition.
    pub fn screen(&self, entry: &KnowledgeEntry, store: &NeuroStore) -> ScreenResult {
        // 1. Check against known-bad bloom filter
        if let Some(hv) = entry.hdc_vector.as_ref()
            .and_then(|b| HdcVector::from_bytes(b)) {
            if self.bad_entry_bloom.might_contain(&hv) {
                return ScreenResult::Quarantine("Matches known-bad signature".into());
            }
        }

        // 2. Cap confidence for external sources
        if entry.source.is_some() && entry.confidence > self.max_external_confidence {
            return ScreenResult::ReduceConfidence(self.max_external_confidence);
        }

        // 3. Anomaly detection: is this entry statistically unusual?
        if let Some(hv) = entry.hdc_vector.as_ref()
            .and_then(|b| HdcVector::from_bytes(b)) {
            let mean_sim = store.mean_similarity_to_existing(&hv);
            if mean_sim < 0.48 { // well below random 0.5 = anti-correlated
                return ScreenResult::Quarantine(
                    format!("Anomalous HDC vector (mean sim {mean_sim:.3})")
                );
            }
        }

        ScreenResult::Admit
    }
}

pub enum ScreenResult {
    Admit,
    ReduceConfidence(f64),
    Quarantine(String),
}
```

### Level 2: Adaptive Immunity (Learned Defenses)

Defenses that improve over time based on past corruption events:

```rust
/// Adaptive immunity: learned patterns of knowledge corruption.
///
/// Built from past AntiKnowledge entries and gate failures.
/// Updated during Dreams consolidation cycle.
pub struct AdaptiveImmunity {
    /// Corruption pattern prototypes: HDC vectors of entries that were
    /// identified as harmful. Bundled by category.
    pub corruption_prototypes: HashMap<InfectionVector, HdcVector>,
    /// Per-domain vulnerability scores: how susceptible is each domain
    /// to specific types of corruption?
    pub domain_vulnerabilities: HashMap<String, Vec<(InfectionVector, f64)>>,
    /// Historical false positive rate per detection method.
    pub false_positive_rates: HashMap<AttributionMethod, f64>,
}

impl AdaptiveImmunity {
    /// Check an entry against learned corruption patterns.
    /// Returns a threat score (0.0 = safe, 1.0 = high threat).
    pub fn threat_score(&self, entry: &KnowledgeEntry) -> f64 {
        let entry_hv = match entry.hdc_vector.as_ref()
            .and_then(|b| HdcVector::from_bytes(b)) {
            Some(hv) => hv,
            None => return 0.0,
        };

        self.corruption_prototypes.values()
            .map(|proto| entry_hv.similarity(proto) as f64)
            .filter(|sim| *sim > 0.526) // above noise floor
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0)
            .clamp(0.0, 1.0)
    }

    /// Update corruption prototypes from new AntiKnowledge entries.
    /// Called during Dreams consolidation.
    pub fn learn_from_antiknowledge(
        &mut self,
        anti_entries: &[KnowledgeEntry],
    ) {
        for entry in anti_entries {
            if let Some(hv) = entry.hdc_vector.as_ref()
                .and_then(|b| HdcVector::from_bytes(b)) {
                let vector = InfectionVector::DistillationError; // classify from metadata
                self.corruption_prototypes.entry(vector)
                    .and_modify(|proto| {
                        // Bundle new anti-entry into existing prototype
                        *proto = HdcVector::bundle(&[proto, &hv]);
                    })
                    .or_insert(hv);
            }
        }
    }
}
```

### Level 3: Active Immunity (Periodic Health Checks)

Proactive audits that run during Dreams consolidation:

```rust
/// Active immunity: periodic knowledge base health audit.
///
/// Runs during Dreams consolidation. Produces a health report
/// with actionable recommendations.
pub struct KnowledgeHealthAudit {
    /// Configuration for the audit.
    pub config: AuditConfig,
}

pub struct AuditConfig {
    /// Maximum acceptable parasite ratio (high-fitness, negative-quality entries).
    /// Default: 0.05 (5% of knowledge base).
    pub max_parasite_ratio: f64,
    /// Minimum Price equation selection component for healthy knowledge base.
    /// Default: 0.0 (non-negative = healthy).
    pub min_selection_component: f64,
    /// Maximum acceptable confirmation cascade probability.
    /// Default: 0.10 (10% of entries should have multi-source confirmation).
    pub max_single_source_ratio: f64,
}

pub struct AuditReport {
    /// Overall health score: 0.0 (critical) to 1.0 (excellent).
    pub health_score: f64,
    /// Number of suspected parasites detected.
    pub parasites_detected: usize,
    /// Price equation diagnostics.
    pub price_selection: f64,
    pub price_transmission: f64,
    /// Entries at risk of confirmation cascade (single-source, high confidence).
    pub cascade_risk_entries: Vec<String>,
    /// Recommended actions.
    pub recommendations: Vec<String>,
}

impl KnowledgeHealthAudit {
    /// Run a full health audit on the knowledge base.
    pub fn audit(&self, store: &NeuroStore, outcomes: &[Outcome]) -> AuditReport {
        // 1. Detect parasites
        let parasites = detect_parasites(
            &store.all_entries(),
            &store.entry_stats(),
            outcomes,
        );

        // 2. Price equation diagnostics
        let price_data: Vec<(f64, f64, f64)> = store.all_entries().iter()
            .filter_map(|e| {
                let stats = store.entry_stats().get(&e.id)?;
                Some((
                    fitness(e, stats),
                    stats.retrievals_per_day,
                    stats.delta_fitness,
                ))
            })
            .collect();
        let (selection, transmission) = price_equation_diagnostics(&price_data);

        // 3. Confirmation cascade risk
        let cascade_risks: Vec<String> = store.all_entries().iter()
            .filter(|e| {
                e.confidence > 0.7
                    && e.source_episodes.len() <= 1
                    && e.kind != KnowledgeKind::AntiKnowledge
            })
            .map(|e| e.id.clone())
            .collect();

        // 4. Compute health score
        let parasite_ratio = parasites.len() as f64 / store.all_entries().len().max(1) as f64;
        let cascade_ratio = cascade_risks.len() as f64 / store.all_entries().len().max(1) as f64;
        let health_score = (1.0 - parasite_ratio * 5.0)  // parasites heavily penalize health
            .min(1.0 - cascade_ratio * 2.0)               // cascade risk moderately penalizes
            .min(if selection < 0.0 { 0.3 } else { 1.0 }) // negative selection is critical
            .max(0.0);

        // 5. Generate recommendations
        let mut recommendations = Vec::new();
        if parasites.len() > 0 {
            recommendations.push(format!(
                "Found {} suspected parasites. Run anti-knowledge generation for: {:?}",
                parasites.len(), &parasites[..parasites.len().min(3)]
            ));
        }
        if selection < self.config.min_selection_component {
            recommendations.push(
                "Price equation selection is negative — bad entries are being preferentially selected. Review high-fitness entries.".into()
            );
        }
        if cascade_ratio > self.config.max_single_source_ratio {
            recommendations.push(format!(
                "{} entries ({:.0}%) have high confidence but single-source confirmation. Risk of confirmation cascade.",
                cascade_risks.len(), cascade_ratio * 100.0
            ));
        }

        AuditReport {
            health_score,
            parasites_detected: parasites.len(),
            price_selection: selection,
            price_transmission: transmission,
            cascade_risk_entries: cascade_risks,
            recommendations,
        }
    }
}
```

### Epistemological Foundations

The knowledge immune system draws from three epistemological traditions:

**Popper's Falsificationism** (1934): Knowledge grows by identifying what is wrong, not by confirming what is right. AntiKnowledge entries implement this directly — they are falsification records that prevent regression to disproven beliefs.

**Kuhn's Paradigm Shifts** (1962): Knowledge is not cumulative; it undergoes revolutionary changes when anomalies accumulate. The Price equation diagnostics detect this: when the selection component goes negative (bad entries being selected), the knowledge base is in crisis and needs restructuring.

**Proctor's Agnotology** (2008): Ignorance is not just absence of knowledge but can be actively produced. Adversarial poisoning in the mesh/chain context implements culturally induced ignorance. The immune system's bloom filter and anomaly detection are defenses against manufactured ignorance.

**References**:
- Popper, K.R. (1934/1959). *The Logic of Scientific Discovery*. Hutchinson.
- Kuhn, T.S. (1962). *The Structure of Scientific Revolutions*. University of Chicago Press.
- Proctor, R.N. & Schiebinger, L. (2008). *Agnotology: The Making and Unmaking of Ignorance*. Stanford.
- Zou, W. et al. (2025). "PoisonedRAG: Knowledge Corruption Attacks to RAG." *USENIX Security 2025*.

**Test criteria**:
- InnateImmunity screens entries matching known-bad bloom filter → Quarantine
- InnateImmunity caps external entry confidence at max_external_confidence
- AdaptiveImmunity threat_score > 0.526 for entries similar to corruption prototypes
- KnowledgeHealthAudit detects parasites (high fitness, negative quality)
- KnowledgeHealthAudit flags single-source high-confidence entries as cascade risks
- Price equation (0.0, 0.0) for empty input
- Health score decreases when parasites detected

---

## Current Status and Gaps

**Implemented**: `KnowledgeKind::AntiKnowledge` variant. `refuted_insight_id` and `refutation_evidence` fields on `KnowledgeEntry`. `refutation_warning()` method. Confidence floor enforcement at `0.3` during decay and GC.

**Missing**: Half-speed demurrage (designed above; Korai precompile not implemented). Reactive checking (designed above). Epistemic parasite detection (designed above; needs outcome tracking). Price equation diagnostics (designed above). Automatic AntiKnowledge generation from gate failures (designed above; needs wiring into gate pipeline).

---

## Cross-References

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for all six knowledge types
- See [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for decay mechanics
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for how AntiKnowledge interacts with distillation
- See topic [11-safety](../11-safety/INDEX.md) for knowledge ingestion safety (quarantine pipeline)
