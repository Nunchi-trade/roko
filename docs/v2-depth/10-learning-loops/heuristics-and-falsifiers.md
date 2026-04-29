# Heuristics and Falsifiers

> Depth for [07-LEARNING.md](../../unified/07-LEARNING.md). Heuristics as testable predictions with mandatory falsifiers and calibration scores. Worldviews as co-citation clusters of heuristics. Predictive foraging cutoffs via Marginal Value Theorem. All expressed as Signal kinds with a predict-publish-correct Loop for calibration.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, Kind system, HDC fingerprints, demurrage), [02-CELL](../../unified/02-CELL.md) (Cell, Verify, Score, React, Compose protocols, predict-publish-correct), [06-MEMORY](../../unified/06-MEMORY.md) (Store, demurrage economics, Heuristics, AntiKnowledge), [07-LEARNING](../../unified/07-LEARNING.md) (L2 heuristic calibration, L3 playbook distillation), [autocatalytic-compounding.md](autocatalytic-compounding.md) (heuristic calibration Loop)

**Source docs**: [19-heuristics-worldviews-and-falsifiers.md](../../docs/05-learning/19-heuristics-worldviews-and-falsifiers.md), [16-predictive-foraging.md](../../docs/05-learning/16-predictive-foraging.md)

---

## 1. Why Playbooks Are Not Enough

Playbooks are the current learning currency: concrete procedural sequences extracted from successful episodes. They work well for direct reuse, but they have three structural limitations:

1. **Over-specific**: Playbooks bind to particular tools, file paths, and workflow orderings. A playbook for "modify `roko-core/src/config/schema.rs`" does not transfer to "modify `roko-agent/src/safety/contracts.rs`" even when the underlying pattern (add a field, update serde, add a test) is the same.

2. **No falsification surface**: A playbook's success or failure is attached to the whole sequence, not to any individual belief about how the world works. When a playbook fails, the system knows the sequence did not work, but not which assumption was wrong.

3. **No competing priors**: Playbooks do not give the Router or Composer a clean way to keep multiple competing approaches alive on purpose. The system uses the best-matching playbook and discards alternatives.

Heuristics solve these problems by turning reusable beliefs into first-class durable Signals with testable predictions and mandatory falsifiers. A playbook can still be compiled from heuristics, but now the system knows which prior it came from, how often it held up, and what evidence pushes against it.

---

## 2. The Heuristic Signal

A Heuristic is a Signal Kind (see [01-SIGNAL.md](../../unified/01-SIGNAL.md)) that captures a reusable claim, the conditions where it applies, the predicted outcome, and the calibration record from lived experience.

```rust
/// A durable belief with testable prediction and mandatory falsifier.
/// Stored as a Signal in the neuro Store with HDC fingerprint for similarity search.
pub struct Heuristic {
    pub id: Uuid,
    /// The claim in natural language (for human inspection).
    pub claim: String,
    /// Conditions that must hold for this heuristic to apply.
    pub preconditions: Vec<Predicate>,
    /// What should happen if the heuristic is correct.
    pub prediction: Predicate,
    /// HDC fingerprint for similarity-based retrieval.
    pub fingerprint: HdcVector,
    /// Calibration record: how well this claim matches reality.
    pub calibration: Calibration,
    /// Lineage: which heuristics this was derived from.
    pub lineage: Vec<HeuristicId>,
    /// Evidence: which episodes support this claim.
    pub receipts: Vec<EpisodeHash>,
}

pub struct Calibration {
    /// Total episodes where preconditions matched.
    pub trials: u32,
    /// Episodes where prediction held.
    pub confirmations: u32,
    /// Episodes where falsifier fired (prediction violated).
    pub violations: u32,
    /// Brier score tracking prediction quality.
    pub brier_score: f64,
    /// When this was last tested.
    pub last_trial_at: Timestamp,
    /// Wilson confidence interval bounds.
    pub confidence_interval: (f64, f64),
}
```

Three details are load-bearing:

- **`preconditions`** make the claim matchable against the current situation instead of being free text. The Compose Cell can scan for heuristics whose preconditions match without natural-language understanding.
- **`prediction`** says what should happen if the heuristic is correct. This creates a testable expectation.
- **`receipts`** preserve the episode lineage that justified the heuristic, making the belief auditable.

Because heuristics are Signals, they participate in the full Signal lifecycle: HDC fingerprint similarity search, demurrage-weighted retention, lineage tracking, and tier progression. An unused heuristic decays naturally through demurrage. A frequently-cited heuristic stays warm.

---

## 3. Predicates and Falsifiers

A heuristic is only useful if the system can tell when it applies (preconditions) and when reality disproves it (falsifier). The Predicate surface serves both roles:

```rust
/// Structured condition that can be evaluated at runtime.
pub enum Predicate {
    LanguageIs(Language),
    FileMatches(Glob),
    ToolAvailable(ToolId),
    GateRecentlyFailed(GateId),
    AgentRoleIs(Role),
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
    /// HDC similarity: applies when task fingerprint is close to this region.
    SimilarTo { fingerprint: HdcVector, threshold: f64 },
}
```

A **falsifier** is the concrete outcome check that can refute the heuristic's prediction. It is not a separate type -- it is the negation of the prediction Predicate evaluated against the actual outcome. The distinction matters because every heuristic has an inspectable failure surface rather than an untestable slogan.

Example heuristic with explicit falsifier:

```
Heuristic: "When modifying config schema (FileMatches 'config/schema.rs'),
            always add a serde default annotation."

Preconditions: [FileMatches("*/config/schema.rs")]
Prediction:    GateRecentlyFailed("compile") -> false
               (i.e., following this advice prevents compile failures)

Falsifier:     If preconditions match AND agent followed the advice AND
               the compile gate still failed, this heuristic is violated.
```

---

## 4. Heuristic Lifecycle

### Birth

Heuristics enter the system in three ways:

1. **Distilled from repeated episodes**: The L3 distillation Loop ([07-LEARNING.md](../../unified/07-LEARNING.md)) detects recurring patterns across successful episodes and extracts candidate heuristics.
2. **Stated by an agent**: An agent can propose a heuristic as a candidate prior during execution.
3. **Imported**: From research papers, other deployments, or user-provided domain knowledge. Imported heuristics start with a trust discount until locally validated.

Fresh heuristics start advisory (low confidence, low prompt weight) rather than dominant. They need trials and calibration before earning prompt weight.

### Test

Every episode is a potential test. Before action, the Compose Cell scans for heuristics whose `preconditions` match the current situation. After action, the Verify Cell (gate verdicts) and outcome Pulses close the loop:

```rust
/// Calibration verdict for a single heuristic-episode pair.
pub enum HeuristicVerdict {
    /// Preconditions matched, prediction held.
    Confirmed,
    /// Preconditions matched, prediction violated.
    Violated,
    /// Preconditions did not match (heuristic was irrelevant to this episode).
    Irrelevant,
    /// Outcome suggests a narrower version of the heuristic is correct.
    Refined(Heuristic),
    /// Outcome suggests a broader version of the heuristic is correct.
    Generalized(Heuristic),
    /// Strong evidence that the heuristic is wrong.
    Refuted,
}
```

`Confirmed` and `Violated` update the calibration record incrementally. `Refined` and `Generalized` create new lineage-linked heuristics instead of mutating history. `Refuted` retires the heuristic from the hot path without breaking lineage resolution.

### Adjust

Calibration is empirical and incremental:

- `trials` increments when preconditions actually matched.
- `confirmations` increments when the predicted outcome held.
- `violations` increments when the falsifier fired.
- Brier score and Wilson confidence intervals track both sharpness and reliability.

Prompt weighting follows the **confidence lower bound**, not raw win rate. This keeps young heuristics usable (they appear in prompts with a disclaimer) without letting a tiny sample masquerade as certainty.

```rust
/// Compute prompt weight from calibration.
/// Uses Wilson confidence interval lower bound.
pub fn prompt_weight(calibration: &Calibration) -> f64 {
    if calibration.trials < 3 {
        return 0.1;  // advisory only, minimal weight
    }
    let p = calibration.confirmations as f64 / calibration.trials as f64;
    let z = 1.96;  // 95% confidence
    let n = calibration.trials as f64;
    // Wilson score lower bound
    let lower = (p + z * z / (2.0 * n)
        - z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt())
        / (1.0 + z * z / n);
    lower.max(0.0)
}
```

### Retire

Retirement is not deletion. A refuted heuristic:

- Loses influence through the existing confidence and tiering machinery.
- Remains resolvable by content hash (lineage is preserved).
- Preserves the receipts that explain why it was trusted and later challenged.
- Decays naturally through demurrage if not cited.

History is preserved; attention is reallocated.

---

## 5. Worldviews as Co-Citation Clusters

A **worldview** is a cluster of heuristics that keep appearing together in successful episodes. It is not a handcrafted persona -- it is an observed structure in the heuristic citation graph.

```rust
/// A cluster of co-cited heuristics that form a coherent belief set.
/// Discovered by clustering the heuristic citation graph, not by declaration.
pub struct Worldview {
    pub id: Uuid,
    /// Core heuristics that define this worldview.
    pub core_heuristics: Vec<HeuristicId>,
    /// How often these heuristics appear together.
    pub coherence_score: f64,
    /// How well episodes using this worldview pass gates.
    pub effectiveness_score: f64,
    /// HDC fingerprint of the worldview's domain.
    pub domain_fingerprint: HdcVector,
}
```

Three practical uses:

1. **Route Cell** picks the worldview whose `domain_fingerprint` best matches the incoming task.
2. **Compose Cell** injects the worldview's core heuristics as a coherent prior set rather than a random pile of tips.
3. **React Cell** keeps multiple worldviews active so the system does not collapse into monoculture.

This connects to the C-factor work in [c-factor-as-lens.md](c-factor-as-lens.md): diversity is not noise to eliminate. The main worldview handles the common case. A challenger worldview keeps the calibration loop honest. Niche worldviews stay in cold storage (demurrage-cooled) until their domain matches again.

**Status**: Target-state. `HeuristicRule` exists in `roko-neuro`, but worldview clustering, coherence scoring, and domain fingerprinting are not yet implemented.

---

## 6. Dissonance and Active Learning

When two active heuristics predict incompatible outcomes for the same situation, the system has a **dissonance** -- a point of internal inconsistency. Dissonances are high-information work:

```rust
/// Two heuristics disagree about the same situation.
pub struct Dissonance {
    pub heuristics: [HeuristicId; 2],
    pub predictions: [Predicate; 2],
    pub situation: SituationHash,
}
```

Dissonance matters because:

1. It identifies where the current worldview is internally inconsistent.
2. It creates a natural active-learning queue: resolve dissonances by running the decisive test.
3. It lets later episodes update both competing heuristics against the same ground truth.

The scheduling implication: if the system can cheaply gather reality on a dissonant case, it should prefer doing that over another low-information repetition. This is the **Marginal Value Theorem** (MacArthur & Pianka 1966) applied to information foraging: stop searching for more context when the marginal information gain drops below the cost of the search.

---

## 7. Predictive Foraging: When to Stop Searching

The Marginal Value Theorem from optimal foraging theory provides the cutoff for context search. An agent foraging for information (heuristics, skills, similar episodes) faces a diminishing returns curve: the first retrieval is highly informative, each subsequent retrieval less so.

The optimal cutoff: stop searching when the marginal value of the next retrieval equals the opportunity cost of the search time.

```rust
/// Decide whether to continue searching for more context.
/// Based on Marginal Value Theorem: stop when marginal gain < search cost.
pub fn should_continue_foraging(
    retrievals_so_far: usize,
    marginal_information_gain: f64,  // estimated from HDC similarity decay
    search_cost_per_retrieval: f64,  // in tokens (opportunity cost)
    task_token_budget: usize,
) -> bool {
    // Information gain decays roughly as 1/sqrt(n) for independent retrievals
    let expected_gain = marginal_information_gain / ((retrievals_so_far + 1) as f64).sqrt();
    let remaining_budget = task_token_budget.saturating_sub(
        retrievals_so_far * search_cost_per_retrieval as usize
    );

    expected_gain > search_cost_per_retrieval && remaining_budget > 0
}
```

In practice, the Compose Cell retrieves heuristics and context greedily but stops when:
- HDC similarity of the next candidate drops below a threshold (diminishing relevance).
- Token budget for the context section is exhausted.
- The retrieval count exceeds a configurable maximum (default: 5 heuristics, 3 skills).

This connects to the Section->Scaffold Loop (#3 in [missing-loops-and-calibration.md](missing-loops-and-calibration.md)): sections that consistently exhaust their budget without improving pass rates get their budget reduced.

---

## 8. AntiKnowledge as a Signal Kind

[06-MEMORY.md](../../unified/06-MEMORY.md) defines `AntiKnowledge` as a Signal Kind that repels future Signals in the same HDC region. In the heuristic framework, AntiKnowledge is the product of refuted heuristics:

When a heuristic is refuted (strong evidence of incorrectness), it can be converted to an AntiKnowledge Signal. This Signal does not just decay passively -- it actively discourages future heuristics in the same semantic region from gaining confidence.

```rust
/// Convert a refuted heuristic to AntiKnowledge.
/// The AntiKnowledge Signal carries the original heuristic's fingerprint
/// and repels future retrievals in the same HDC neighborhood.
pub fn refute_to_antiknowledge(heuristic: &Heuristic) -> Signal {
    Signal::new(
        Kind::AntiKnowledge,
        AntiKnowledgePayload {
            original_claim: heuristic.claim.clone(),
            fingerprint: heuristic.fingerprint.clone(),
            refutation_evidence: heuristic.calibration.violations,
            refutation_episodes: heuristic.receipts.clone(),
        },
    )
}
```

This prevents the distillation Loop from re-extracting the same wrong heuristic from the same episodes. The system learns not just what works, but what has been proven wrong.

---

## 9. Heuristic Inspection Surface

Heuristics should be externally inspectable. A user should be able to ask:

| Query | What it returns |
|---|---|
| `roko heuristic list` | All active heuristics with calibration scores |
| `roko heuristic show <id>` | Full heuristic with receipts and lineage |
| `roko heuristic stats` | Calibration summary: most/least reliable, most tested |
| `roko heuristic similar <query>` | HDC similarity search across heuristics |
| `roko heuristic export` | Export heuristics for sharing across deployments |
| `roko heuristic import <file>` | Import with configurable trust discount |

Imported heuristics retain their receipts and calibration metadata but enter with a configurable trust discount (default: 0.5x confidence) until local evidence revalidates them.

**Status**: These CLI commands do not exist. `HeuristicRule` in `roko-neuro` provides the storage layer. The query surface is target-state.

---

## 10. The Learning Story

With heuristics in place, the full learning pipeline becomes:

```
1. Episodes capture raw work and outcomes.
    -> Signal kind: Episode (durable, in Store)

2. Distillation extracts candidate insights and heuristics.
    -> L3 Loop: batch consolidation every 20 episodes

3. Calibration promotes, refines, or cools heuristics based on real outcomes.
    -> L2 Loop: predict-publish-correct per episode

4. Worldviews cluster heuristics that co-occur in successful episodes.
    -> L3 Loop: cluster analysis every 50 episodes (target-state)

5. Playbooks compile the most concrete, battle-tested procedural fragments.
    -> Downstream of heuristics: compiled, not primary
```

Playbooks are not removed. Their role narrows to compiled procedural projections. Heuristics are the durable belief layer; playbooks are the concrete execution layer. This gives the system a library of priors that survives tool churn, composes across domains, and is inspectable by the user.

---

## 11. Mori-Diffs Reality

Per `tmp/mori-diffs/04-LEARNING.md`:

- **`HeuristicRule`** exists in `roko-neuro` as a basic struct. It lacks the full `Predicate` surface, `Calibration` record, and `lineage` tracking described here.
- **Worldview clustering** is not implemented. No code for co-citation analysis or domain fingerprinting.
- **Dissonance detection** is not implemented. No contradiction tracking between heuristics.
- **AntiKnowledge** is defined as a Signal Kind but not yet produced by heuristic refutation.
- **Predictive foraging** has a `CalibrationTracker` in `roko-learn/src/prediction.rs` that handles basic prediction/outcome tracking. The full MVT foraging cutoff is not implemented.
- **Heuristic CLI commands** do not exist. The inspection surface is target-state.

The near-term implementation path: typed heuristic specs with preconditions and contradiction tracking, layered on top of the existing `HeuristicRule` in `roko-neuro`. The full worldview, dissonance, and export/import story is deferred.

---

## What This Enables

1. **Durable learning**: Heuristics survive tool churn and compose across domains because they capture beliefs, not procedures.
2. **Structural falsification**: Every heuristic has an inspectable failure surface. The system knows not just what works, but what has been proven wrong (AntiKnowledge).
3. **Active learning**: Dissonance detection creates a natural queue of high-information experiments. The system preferentially tests uncertain beliefs.
4. **Inspectable beliefs**: Users can query, export, and import heuristics. The learning system is not a black box.
5. **Efficient context assembly**: MVT foraging cutoffs prevent over-retrieval, keeping prompts lean and relevant.

## Feedback Loops

- **L2 (Heuristic Calibration)**: Every episode tests matching heuristics. Calibration updates flow back through predict-publish-correct.
- **Heuristic -> Compose**: Matched heuristics are injected into agent prompts. Better heuristics produce better outcomes, producing better evidence for further calibration.
- **AntiKnowledge -> Distillation**: Refuted heuristics prevent re-extraction of the same wrong insight. The system's error surface shrinks monotonically.
- **Dissonance -> Scheduling**: High-dissonance areas attract learning attention. The system becomes curious about its own inconsistencies.

## Open Questions

1. **Predicate expressiveness**: Is the current Predicate enum sufficient for real-world heuristics? File globs and gate names cover common cases, but domain-specific conditions (e.g., "when the PR has more than 5 files") may need extension.
2. **Calibration sample size**: How many trials does a heuristic need before its calibration is trustworthy? Wilson intervals handle small samples gracefully, but the prompt weight function treats trials < 3 as purely advisory. Is 3 the right threshold?
3. **Worldview stability**: Once worldviews form, how resistant should they be to change? Aggressive re-clustering on every batch risks instability. Infrequent clustering risks staleness. The update frequency should probably follow the same stability budget framework as other Loops.
4. **Cross-deployment heuristic transfer**: Imported heuristics enter with a trust discount. What is the right discount factor? Too low and the import is useless; too high and a wrong import corrupts local calibration.
5. **MVT calibration**: The foraging cutoff depends on estimating "marginal information gain" of the next retrieval. This estimate is itself a prediction that could be miscalibrated. Should the foraging cutoff have its own calibration loop?
