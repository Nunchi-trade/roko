# 07 — Active Inference for Context Selection

> Layer 2 Scaffold — Synapse Architecture
> Status: **Scaffold** — Formula specified, implementation pending (E2 in 12a-cognitive-layer.md)
> Canonical sources: `refactoring-prd/09-innovations.md` §XIX.B, Friston (2022)


> **Implementation**: Shipping

---

## Abstract

Active inference provides a principled answer to "what should the scaffold include?" by decomposing context value into pragmatic value (goal-seeking) and epistemic value (information gain). An uncertain agent automatically explores novel context; a confident agent automatically exploits proven context. No separate exploration/exploitation tradeoff is needed — the balance emerges from the mathematics of expected free energy minimization. This document specifies the EFE formula, the scoring mechanism, the softmax selection policy, and the integration with the 5-stage assembly pipeline.

---

## 1. The Free Energy Principle

Karl Friston (2006, 2010, 2022) established the free energy principle: all self-organizing systems minimize variational free energy — the gap between their internal model and reality. Applied to agents: they act to bring their model of the world into alignment with observations, while simultaneously updating their model.

The key decomposition for context selection is **expected free energy (EFE)**:

```
G(section) = pragmatic_value(section) + epistemic_value(section) - ambiguity(section)
```

- **Pragmatic value:** "Will including this section help the agent succeed?" Measured by historical gate outcomes when this section was/was not included.
- **Epistemic value:** "Will including this section reduce the agent's uncertainty?" Measured by information gain — how much does this section change the agent's beliefs about the task?
- **Ambiguity:** "How unclear is this section's contribution?" Measured by variance in outcomes when this section is included.

---

## 2. The EFE Formula for Context Selection

From the canonical specification (refactoring-prd/09-innovations.md §XIX.B):

```
G(section) = pragmatic_value + epistemic_value - ambiguity

Where:
  pragmatic_value = E[task_success | section_included]
                  - E[task_success | section_excluded]

  epistemic_value = D_KL(P(state | section) || P(state))
                  = information gain from including section

  ambiguity      = Var[task_success | section_included]
```

The selection policy uses a softmax with inverse temperature γ (gamma):

```
P(include section_i) = softmax(γ × G(section_i))
                     = exp(γ × G_i) / Σ_j exp(γ × G_j)
```

With γ = 8.0 (from the canonical spec). Higher γ makes the selection more deterministic (greedy). Lower γ increases exploration.

### 2.1 Behavior Under Uncertainty

When the agent is **uncertain** about a domain (low track record, few historical observations):

- Epistemic value dominates the EFE score
- The agent prioritizes context that fills knowledge gaps — architectural overviews, module interfaces, existing patterns
- Even context that does not directly relate to the immediate task may be selected if it resolves uncertainty

When the agent is **confident** (high track record, many successful observations):

- Pragmatic value dominates
- The agent grabs the highest-proven context for immediate application — relevant file content, specific type signatures, proven patterns
- Epistemic context is deprioritized because the agent already knows the domain

No hyperparameters control this balance. It emerges from the mathematics.

### 2.2 Practical Example

Agent receives: "Implement HDC fingerprinting for knowledge entries in roko-neuro."

Agent's uncertainty assessment:
- HDC vectors: low uncertainty (implemented in bardo-primitives, 50+ successful tasks)
- roko-neuro crate: HIGH uncertainty (new crate, no prior episodes)

Active inference result:
- 60% of context budget allocated to roko-neuro architecture docs, module interfaces, existing patterns (epistemic — fill the gap)
- 40% allocated to HDC implementation patterns, fingerprinting algorithms (pragmatic — known-good)

Without active inference, the agent would grab the 50 highest-priority HDC-related sections and miss the roko-neuro-specific architecture that determines where the fingerprinting code should live.

---

## 3. Scoring Mechanism

The active inference score integrates with the existing SectionScorer:

```
score = track_record(entry) × belief_change(entry) / uncertainty
```

Where:

| Component | Source | Range |
|-----------|--------|-------|
| `track_record` | Historical gate pass rate when this knowledge type was included | [0.0, 1.0] |
| `belief_change` | Bayesian surprise: how much does this entry change the agent's posterior belief about the task? [Itti & Baldi, NeurIPS 2005] | [0.0, ∞) |
| `uncertainty` | Agent's current uncertainty about the domain (from prediction accuracy declining, or simply lack of prior episodes) | [0.1, ∞) |

### 3.1 Track Record Estimation

```rust
fn track_record(section_type: &str, task_category: &str) -> f64 {
    // Query episode history:
    // - How often was this section type included for this task category?
    // - When included, what was the gate pass rate?
    // - When excluded, what was the gate pass rate?
    // Return: conditional probability of success given inclusion
    let pass_when_included = episodes
        .filter(|e| e.included_sections.contains(section_type))
        .filter(|e| e.task_category == task_category)
        .mean(|e| e.gate_passed as f64);

    let pass_when_excluded = episodes
        .filter(|e| !e.included_sections.contains(section_type))
        .filter(|e| e.task_category == task_category)
        .mean(|e| e.gate_passed as f64);

    pass_when_included - pass_when_excluded
    // Positive = this section helps. Negative = this section hurts.
}
```

### 3.2 Belief Change (Bayesian Surprise)

Bayesian surprise [Itti & Baldi, NeurIPS 2005] measures how much observing a piece of information changes the agent's beliefs:

```
belief_change = D_KL(posterior || prior)
              = Σ_x posterior(x) × log(posterior(x) / prior(x))
```

In practice, for context selection, belief change is approximated by the novelty of the section content relative to what the agent already knows:

```rust
fn belief_change(section: &ContextChunk, agent_knowledge: &[ContextChunk]) -> f64 {
    // HDC fingerprint comparison
    let section_fp = text_fingerprint(&section.content);
    let max_similarity = agent_knowledge.iter()
        .map(|k| hamming_similarity(&section_fp, &text_fingerprint(&k.content)))
        .max_f64()
        .unwrap_or(0.0);

    // High belief change = low similarity to existing knowledge
    1.0 - max_similarity
}
```

Sections that are highly similar to what the agent already has in its prompt provide low belief change (redundant). Sections that are dissimilar provide high belief change (novel).

### 3.3 Uncertainty Estimation

```rust
fn uncertainty(task_category: &str, domain: &str) -> f64 {
    // Base uncertainty from episode count
    let episode_count = episodes
        .filter(|e| e.task_category == task_category && e.domain == domain)
        .count();

    // More episodes → lower uncertainty
    let base = 1.0 / (1.0 + (episode_count as f64 / 10.0));

    // Adjust by recent prediction accuracy
    let recent_accuracy = recent_predictions
        .filter(|p| p.domain == domain)
        .mean(|p| (p.actual - p.predicted).abs());

    base + recent_accuracy.unwrap_or(0.5)
}
```

---

## 4. Comparison with Static Priority

The current implementation (SectionScorer in `roko-compose/src/scorer.rs`) uses static priority-based scoring:

```rust
// Current: SectionScorer
confidence = priority_to_score(section.priority) // 0.2 - 1.0
novelty = recency_decay(section.created_at)       // 1h fresh, 24h stale
utility = inverse_content_size(section.content)    // shorter = higher utility
reputation = trust_level(section.source)            // source trust
```

Active inference scoring replaces the hand-tuned weights with learned ones:

| Aspect | Static Priority | Active Inference |
|--------|----------------|-----------------|
| Scoring basis | Hand-tuned priority levels | Historical outcomes + information theory |
| Adaptation | None (fixed priorities) | Adapts per task type and domain |
| Exploration | None (always includes high-priority) | Automatically explores under uncertainty |
| Exploitation | Always (greedy on priority) | Automatically exploits under confidence |
| Cold start | Works immediately | Requires ~10 episodes to calibrate |
| Interpretability | High (priority is explicit) | Medium (EFE components are inspectable) |

Active inference is strictly superior after calibration (>10 episodes per task category), but requires a cold-start fallback. The design: use static priorities for the first 10 episodes per category, then switch to active inference scoring. This is the bandit's warm-up period.

---

## 5. Integration with 5-Stage Pipeline

Active inference scoring plugs into Stage 2 (Scoring) of the 5-stage assembly pipeline:

```
Stage 1: Query → Candidate retrieval (HDC search + keyword)
Stage 2: Scoring → Active inference EFE scoring ← here
Stage 3: Diversity → Deduplicate near-identical candidates
Stage 4: Budget → Fit scored candidates to token budget
Stage 5: Format → U-shaped placement
```

The active inference scorer replaces the static composite score:

```
// Old (static):
score = hdc_similarity × 0.4 + weight_decay × 0.3 + pf_utility × 0.2 + freshness × 0.1

// New (active inference):
score = track_record × belief_change / uncertainty
```

The `pf_utility` component from the old formula (Predictive Foraging utility) is subsumed by `track_record` in the active inference model. Both measure "did including this content improve outcomes?" but active inference provides a principled framework rather than ad hoc weighting.

---

## 6. Connection to Golem's VCG Attention Auction

Active inference for context selection and the VCG attention auction (see [10-vcg-attention-auction.md](10-vcg-attention-auction.md)) are solving the same problem: optimal allocation of the scarce context window. The difference is the mechanism:

- **Active inference:** Single agent, centralized scoring, softmax selection. Used by Roko's scaffold for prompt assembly.
- **VCG auction:** Multiple bidding subsystems, decentralized mechanism design, second-price payments. Designed for multi-agent collectives sharing a knowledge chain.

Both converge on the same allocation under certain conditions (VCG auctions implement efficient allocation under incentive compatibility constraints, while active inference implements efficient allocation under free energy minimization). The research path: demonstrate that VCG and active inference produce equivalent allocations on the same input, then use whichever is computationally cheaper for the context.

---

## 7. Affect Modulation

The active inference scorer is modulated by the Daimon's PAD (Pleasure-Arousal-Dominance) state:

| PAD Dimension | Effect on EFE |
|--------------|--------------|
| High arousal (≥ 0.35) | Increase pragmatic_value weight → favor proven, action-oriented context |
| Low arousal (≤ -0.35) | Increase epistemic_value weight → favor novel, exploratory context |
| Low pleasure (≤ -0.35) | Increase weight on anti-knowledge and failure history |
| High pleasure | No special modulation |
| Low dominance | Favor explanatory context (agent seeks understanding) |
| High dominance | Favor directive context (agent acts autonomously) |

This modulation is the bridge between the Daimon affect system (see [12-affect-modulated-retrieval.md](12-affect-modulated-retrieval.md)) and context selection. An anxious agent (low pleasure, high arousal) automatically receives more cautionary context. A confident, exploratory agent (high pleasure, low arousal) automatically receives more novel context.

---

## 8. Academic Foundations

**Friston (2006, 2010, 2022), The Free Energy Principle.** All self-organizing systems minimize variational free energy. Active inference extends this to agents: they act to minimize expected free energy, which naturally balances goal-seeking (pragmatic value) and information-seeking (epistemic value). The exploration/exploitation tradeoff emerges from the mathematics without separate mechanisms.

**Friston et al. (2015), Active Inference and Epistemic Value.** Formal derivation of the EFE decomposition: G = pragmatic_value + epistemic_value. Applied to planning and decision-making under uncertainty.

**Itti & Baldi (2005), Bayesian Surprise.** NeurIPS paper defining surprise as the KL divergence between posterior and prior beliefs. Used here as the epistemic_value component: how much does including a section change the agent's beliefs?

**Mehrabian (1996), PAD Model.** Three-dimensional emotional space (Pleasure-Arousal-Dominance) used for affect modulation of the EFE scorer.

**Sumers et al. (2023), CoALA.** Cognitive Architectures for Language Agents. Provides the framework for mapping active inference to agent context selection. CoALA's "working memory assembly" phase is where active inference operates.

---

## 9. Implementation Plan

From 12a-cognitive-layer.md (E2):

| # | Item | Status | Notes |
|---|------|--------|-------|
| E2a | EFE scoring function (pragmatic + epistemic - ambiguity) | **Pending** | Core formula |
| E2b | Track record estimation from episode history | **Pending** | Query past outcomes |
| E2c | Belief change via HDC fingerprint similarity | **Pending** | Reuse existing HDC |
| E2d | Uncertainty estimation from episode count + prediction accuracy | **Pending** | Cold-start fallback |
| E2e | Softmax selection with γ=8.0 | **Pending** | Selection policy |
| E2f | PAD modulation of EFE weights | **Pending** | Requires Daimon (F1) |

---

## 10. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| EFE formula specified | **Specified** |
| SectionScorer (static fallback) | **Implemented** (6 tests) |
| Active inference scorer | **Not yet** |
| Track record from episodes | **Not yet** (episodes exist, query not built) |
| Belief change via HDC | **Not yet** (HDC exists, belief change not built) |
| Softmax selection | **Not yet** |
| PAD modulation | **Not yet** (PAD struct exists in context_assembler.rs) |
| Cold-start fallback | **Designed** (use static scorer for first 10 episodes) |

---

## Cross-References

- [00-composer-trait.md](00-composer-trait.md) — Scorer parameter in Composer trait
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Stage 2 where scoring occurs
- [09-predictive-foraging-mvt.md](09-predictive-foraging-mvt.md) — MVT stopping rule for context search
- [10-vcg-attention-auction.md](10-vcg-attention-auction.md) — Alternative allocation mechanism
- [12-affect-modulated-retrieval.md](12-affect-modulated-retrieval.md) — PAD integration
- `crates/roko-compose/src/scorer.rs` — Current static scorer
- `crates/roko-compose/src/context_assembler.rs` — PadState struct and scoring hook
- `refactoring-prd/09-innovations.md` §XIX.B — Canonical EFE specification
