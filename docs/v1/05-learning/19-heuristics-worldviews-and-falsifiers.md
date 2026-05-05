# Heuristics, Worldviews, and Falsifiers

> **REF14 source:** `../../tmp/refinements/14-worldview-validation.md`
> **Glossary:** [Naming and Glossary](../00-architecture/01-naming-and-glossary.md)
> **Cross-references:** [01-playbook-system](01-playbook-system.md), [16-predictive-foraging](16-predictive-foraging.md), [18-self-learning-cybernetic-loops](18-self-learning-cybernetic-loops.md), [20-research-to-runtime](20-research-to-runtime.md), [12-4-tier-distillation-pipeline](../06-neuro/12-4-tier-distillation-pipeline.md), [14-c-factor-collective-intelligence](../00-architecture/14-c-factor-collective-intelligence.md), [25-attention-as-currency](../00-architecture/25-attention-as-currency.md), `../../tmp/refinements/16-research-to-runtime.md`
>
> **Implementation status**: `HeuristicRule` exists in `roko-neuro`. The full worldview/falsifier/dissonance stack described here is **target-state**. Near-term: typed heuristic specs and contradiction tracking. Deferred: worldview clustering, dissonance algebra, and belief export/import.

---

## Purpose

Episodes tell Roko what happened. Playbooks tell it which concrete sequences have worked before. REF14 proposes a richer missing middle: `Heuristic` Engrams that capture a reusable claim, the conditions where it applies, the predicted outcome, and the calibration record showing whether lived experience keeps confirming it. The near-term version of that idea builds on the existing `HeuristicRule` in `roko-neuro` with typed specs and contradiction tracking. See `tmp/refinements/14-worldview-validation.md` for the full proposal.

REF16 extends that middle into a research pipeline. That full paper/claim/replication-ledger stack is deferred, but the provenance instinct remains useful: heuristics should be able to point back to source material when that source materially informed the rule.

This matters because playbooks alone are too concrete. They bind to particular tools, paths, and workflow orderings. Heuristics are more abstract: they say what to check, what to expect, and what would count as being wrong. That gives the learning stack a durable library of priors that can survive tool churn, compose across domains, and be inspected by the user.

The learning story therefore becomes:

1. Episodes capture raw work and outcomes.
2. Distillation extracts candidate insights and heuristics.
3. Calibration promotes, refines, or cools heuristics based on real outcomes.
4. Worldviews cluster heuristics that co-occur in successful episodes.
5. Playbooks compile the most concrete, battle-tested procedural fragments for direct reuse.

## Why Playbooks Are Not Enough

Playbooks remain useful, but they are the wrong level of abstraction for many learning tasks:

- They are over-specific to tools, layouts, and local workflow details.
- They flatten "what worked" without preserving the belief that made it plausible.
- They make contradiction handling awkward because success or failure is attached to the whole sequence, not the underlying prior.
- They do not give the Router or Composer a clean way to keep multiple competing priors alive on purpose.

Heuristics solve that by turning reusable beliefs into first-class durable records. A playbook can still be compiled later, but now the system knows which prior it came from, how often it held up, and what evidence is currently pushing against it.

## The `Heuristic` Engram

REF14 treats heuristics as a first-class durable kind rather than an implementation detail hidden inside playbooks or prompt templates:

```rust
pub struct Heuristic {
    pub id: Uuid,
    pub claim: String,
    pub preconditions: Vec<Predicate>,
    pub prediction: Predicate,
    pub fingerprint: HdcVector,
    pub calibration: Calibration,
    pub lineage: Vec<HeuristicId>,
    pub receipts: Vec<EpisodeHash>,
}

pub struct Calibration {
    pub trials: u32,
    pub confirmations: u32,
    pub violations: u32,
    pub brier_score: f64,
    pub last_trial_at: Timestamp,
    pub confidence_interval: (f64, f64),
}
```

Three details are load-bearing:

- `preconditions` make the claim matchable against the current situation instead of being free text.
- `prediction` says what should happen if the heuristic is correct.
- `receipts` preserve the episode lineage that justified the heuristic in the first place.

Because heuristics are Engrams, they can share the rest of the durable-memory stack: HDC fingerprint similarity where available, provenance, lineage, and tiered retention. The demurrage-balance model described elsewhere remains deferred.

## Predicates and Falsifiers

A heuristic is only useful if the system can tell when it applies and when reality disproves it. REF14's `Predicate` surface gives Roko both:

```rust
pub enum Predicate {
    LanguageIs(Language),
    FileMatches(Glob),
    ToolAvailable(ToolId),
    GateRecentlyFailed(GateId),
    AgentRoleIs(Role),
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
    SimilarTo { fingerprint: HdcVector, threshold: f64 },
}
```

In the learning layer, a **falsifier** is the concrete outcome check that can refute a heuristic's prediction. Sometimes that is a direct contradiction (`prediction` failed after the preconditions matched). Sometimes it is a targeted check emitted as a Bus-visible outcome Pulse such as a gate verdict, regression result, or metric delta that should have moved but did not. The important part is that every heuristic has an inspectable failure surface rather than an untestable slogan.

That gives the system a consistent contract:

- Composer retrieves heuristics whose preconditions match the current situation.
- ACT and VERIFY emit the outcome Pulses that reality provides.
- A calibrator decides whether those outcomes confirmed, violated, or refined the claim.
- The falsifier record is durable and queryable after the fact.

This is the REF14 synergy with REF10 and the two-fabric model: heuristics are durable Engrams, while the runtime can eventually deliver the reality-check signals that confirm, contradict, or refine them.

## Heuristic Lifecycle

### Birth

Heuristics enter the system in three ways:

1. Distilled from repeated episodes and insights.
2. Stated explicitly by an agent as a candidate prior.
3. Imported from research or from another deployment with an attached trust factor.

Fresh heuristics should start advisory rather than dominant. They need receipts, trials, and calibration before they earn prompt weight.

When a heuristic is imported from research, its receipts should at minimum include the source paper or note that informed it. The fuller replication-ledger story is deferred.

### Test

Every episode is a potential test. Before action, Composer scans for heuristics whose `preconditions` match. After action, Policy and Gate outputs close the loop:

```rust
pub trait Calibrator {
    fn score(&self, heuristic: &Heuristic, episode: &Episode) -> Verdict;
}

pub enum Verdict {
    Confirmed,
    Violated,
    Irrelevant,
    Refined(Heuristic),
    Generalized(Heuristic),
    Refuted,
}
```

`Confirmed` and `Violated` update the calibration record. `Refined` and `Generalized` create new lineage-linked heuristics instead of mutating history in place. `Refuted` retires the heuristic from the hot path without breaking lineage resolution.

### Adjust

Calibration should be empirical and incremental:

- `trials` increments when preconditions actually matched.
- `confirmations` increments when the predicted outcome held.
- `violations` increments when the falsifier surface fired.
- Brier score and Wilson confidence intervals track both sharpness and reliability.

Paper-derived heuristics can eventually use the same calibration path. For now, the practical goal is simpler provenance plus local confirmation and contradiction tracking rather than a full replication ledger.

Prompt weighting should follow the confidence lower bound, not raw win rate. That keeps young heuristics usable without letting a tiny sample masquerade as certainty.

### Retire

Retirement is not deletion. A refuted heuristic should lose influence through the existing confidence and tiering machinery, remain resolvable by content hash, and preserve the receipts that explain why it was trusted and later challenged. History is preserved; attention is reallocated.

## Worldviews As Co-Citation Clusters

In the target-state design, a worldview is a cluster of heuristics that keep appearing together in successful episodes. It is not a handcrafted persona. It is an observed structure in the heuristic citation graph.

```rust
pub struct Worldview {
    pub id: Uuid,
    pub core_heuristics: Vec<HeuristicId>,
    pub coherence_score: f64,
    pub effectiveness_score: f64,
    pub domain_fingerprint: HdcVector,
}
```

REF14 adds three practical uses for worldview clustering once the underlying data exists:

- Router can pick the worldview whose `domain_fingerprint` best matches the incoming task.
- Composer can inject the worldview's core heuristics as a coherent prior set rather than a random pile of tips.
- Policy can keep multiple worldviews active so the system does not collapse into one monoculture.

This is the link to REF13's c-factor work. Diversity is not treated as noise to eliminate; it is a measured capability. The main worldview handles the common case, a challenger worldview keeps the calibration loop honest, and niche worldviews stay in cold storage until the domain matches again.

## Dissonance and Active Learning

REF14 makes contradictions visible instead of smoothing them away. Near-term, that means contradiction tracking around heuristic calibration. In the fuller target-state design, when two active heuristics predict incompatible outcomes for the same situation, the system would emit a dissonance record:

```rust
pub struct Dissonance {
    pub heuristics: [HeuristicId; 2],
    pub predictions: [Predicate; 2],
    pub situation: SituationHash,
}
```

Dissonance matters because it is high-information work:

1. It identifies where the current worldview is internally inconsistent.
2. It creates a natural active-learning queue for decisive tests.
3. It lets later episodes update both competing heuristics against the same ground truth.

The scheduling implication is deliberate: if the system can cheaply gather reality on a dissonant case, it should often prefer doing that over another low-information repetition.

## Inspectability and Sharing

Heuristics should be externally inspectable in a way playbooks alone are not. A user should be able to ask:

- Which heuristics are highly calibrated?
- Which ones are recent, unproven hypotheses?
- Which worldviews dominate a given domain?
- Which falsifiers have been firing most often?

That implies a first-class query surface such as `roko heuristic list`, `show`, `stats`, `similar`, `export`, and `import`. Imported heuristics should retain their receipts and calibration metadata but enter with a configurable trust discount until local evidence revalidates them.

If REF16 lands, the same logic can be applied to research-derived runtime defaults: a `claim!`-style resolver could map a config key to a claim ID, then materialize the parameter only if the claim's replication ledger and local calibration are still inside tolerance. If the claim degrades, the resolver should fall back to a safe default rather than silently preserving stale provenance.

This export/import flow is a longer-range idea. It also composes with REF16's deferred replication-ledger framing, but that paper-economy layer is not current architecture.

## Interaction With Playbooks, Neuro, and Profiles

REF14 does not remove playbooks. It narrows their role:

- Heuristics are a promising durable belief layer to strengthen on top of today's `HeuristicRule` machinery.
- Playbooks are compiled procedural projections of heuristics and strategy fragments.
- Neuro stores heuristics and related knowledge as durable Engrams; broader clustering and demurrage-balance semantics remain target-state.
- Domain profiles seed an initial heuristic library, but calibration remains per heuristic rather than per profile.

That separation gives the docs a cleaner architecture story. Learning owns episode feedback, calibration, worldview competition, and externalization of beliefs. Neuro owns durable storage, similarity, and tier movement. Playbooks remain the human-readable, highly concrete output surface rather than the only memory object worth preserving.

## Relationship To Other Docs

- [01-playbook-system](01-playbook-system.md) now reads playbooks as compiled downstream artifacts rather than the only validated knowledge tier.
- [16-predictive-foraging](16-predictive-foraging.md) covers Brier scores and prediction quality at task level; heuristics reuse the same calibration logic at belief level.
- [18-self-learning-cybernetic-loops](18-self-learning-cybernetic-loops.md) explains how the Bus carries the outcome Pulses and calibration topics that falsify or reinforce heuristics.
- [20-research-to-runtime](20-research-to-runtime.md) sketches the target-state paper → claim → heuristic → trial → calibration pipeline and the deferred replication-ledger format.
- [12-4-tier-distillation-pipeline](../06-neuro/12-4-tier-distillation-pipeline.md) describes how Neuro distills, stores, and cools the durable heuristic library.
- [14-c-factor-collective-intelligence](../00-architecture/14-c-factor-collective-intelligence.md) provides the cohort-level reason to keep challenger and niche worldviews active.
- See also `tmp/refinements/14-worldview-validation.md` for the full proposal.
