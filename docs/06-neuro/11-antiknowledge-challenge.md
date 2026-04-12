# AntiKnowledge and the Challenge Mechanism

> AntiKnowledge — validated negative knowledge about what is wrong — serves as the epistemic immune system of Neuro, with a confidence floor of 0.3, 0.5× demurrage, and a challenge mechanism that links refutations to the entries they contradict.

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

## Current Status and Gaps

**Implemented**: `KnowledgeKind::AntiKnowledge` variant. `refuted_insight_id` and `refutation_evidence` fields on `KnowledgeEntry`. `refutation_warning()` method.

**Missing**: Confidence floor enforcement (0.3 minimum during decay/GC). Half-speed demurrage for on-chain AntiKnowledge. Reactive checking against new candidates. Epistemic parasite detection (fitness vs. decision quality correlation). Price equation diagnostics. Automatic AntiKnowledge generation from gate failures.

---

## Cross-references

- See [01-six-knowledge-types.md](./01-six-knowledge-types.md) for all six knowledge types
- See [07-ebbinghaus-decay-with-tier.md](./07-ebbinghaus-decay-with-tier.md) for decay mechanics
- See [12-4-tier-distillation-pipeline.md](./12-4-tier-distillation-pipeline.md) for how AntiKnowledge interacts with distillation
- See topic [11-safety](../11-safety/INDEX.md) for knowledge ingestion safety (quarantine pipeline)
