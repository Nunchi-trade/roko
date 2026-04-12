# Score: 7-Axis Appraisal

> **Abstract:** Every Engram in Roko carries a multi-dimensional quality score. The Score
> struct provides a structured assessment across seven axes — four stable (confidence,
> novelty, utility, reputation) and three extended (precision, salience, coherence). These
> axes collapse into a single effective scalar via a multiplicative formula designed so that
> zero confidence produces zero effective score, novelty and utility act as bonuses, and
> reputation directly scales the result. This document specifies each axis, the effective
> score formula, score arithmetic, and how Scorers produce and consumers interpret scores.

---

## 1. Design Rationale

Agent systems produce and consume enormous quantities of information: task descriptions,
LLM outputs, gate verdicts, knowledge entries, tool traces. Not all information is equally
valuable. A scoring system must answer the question: "How much should I trust and attend to
this Engram?"

Simple scalar scoring (a single 0-1 confidence) loses information. A highly confident but
stale piece of knowledge should score differently from a novel but uncertain observation.
A response from a trusted model should score differently from one originating in untrusted
external data.

Roko's Score uses **orthogonal axes** that capture different quality dimensions. Every
scoring mechanism in the design corpus — confidence scores, novelty detectors, utility
accumulators, reputation trackers, fitness functions, prediction weights, catalytic scores —
collapses into one of these axes. The multi-dimensional representation preserves information
while the effective score formula collapses them into a single scalar when a total ordering
is needed.

---

## 2. The Four Stable Axes

These four axes are implemented and shipping in the current codebase (`roko-core/src/score.rs`):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// [0..1] — how confident are we this Engram is correct/valid?
    pub confidence: f32,
    /// [0..1] — how novel is this Engram compared to prior Engrams?
    pub novelty: f32,
    /// [0..∞) — how useful has this Engram proven historically?
    pub utility: f32,
    /// [0..∞) — reputation of the Engram's author at emission time.
    pub reputation: f32,
}
```

### 2.1 Confidence — [0, 1]

**What it measures**: How sure are we that this Engram is correct, valid, or truthful?

**Range**: [0, 1]. Clamped at construction.

**Examples**:
- A Gate verdict with `passed = true` → confidence near 1.0
- An LLM output that hasn't been verified → confidence 0.5 (neutral)
- A prediction that has been partially falsified → confidence drops toward 0.0

**Critical property**: Zero confidence produces zero effective score regardless of other
axes. This ensures that information known to be incorrect is never prioritized. The formula
`effective = confidence × ...` enforces this structurally.

**Where it comes from**: Gate verdicts, prediction tracking (CalibrationTracker), human
ratings, source verification.

### 2.2 Novelty — [0, 1]

**What it measures**: How new or surprising is this Engram compared to what the system
already knows?

**Range**: [0, 1]. Clamped at construction.

**Examples**:
- A completely new insight not present in any existing knowledge → novelty near 1.0
- A routine heartbeat tick → novelty near 0.0
- A piece of information that updates an existing knowledge entry → novelty ~0.5

**Role in scoring**: Novelty acts as a multiplicative bonus via `(1 + novelty)`. An Engram
with novelty 0.0 has an effective score multiplier of 1.0 from this axis; an Engram with
novelty 1.0 has a multiplier of 2.0. This ensures that novel information is prioritized
without penalizing routine information.

**Connection to Active Inference**: Novelty maps to the epistemic value component of Expected
Free Energy (Friston 2010). High-novelty Engrams carry high expected information gain, making
them priority targets for attention allocation.

### 2.3 Utility — [0, ∞)

**What it measures**: How pragmatically useful has this Engram proven to be? Utility
accumulates over time as the Engram is referenced, used in compositions, or leads to
successful outcomes.

**Range**: [0, ∞). Unbounded above; clamped to non-negative at construction.

**Examples**:
- A playbook rule that has been applied 50 times with positive outcomes → high utility
- A fresh Engram that has never been used → utility 0.0
- A knowledge entry that has been referenced in 10 successful task completions → utility
  growing proportionally

**Role in scoring**: Like novelty, utility acts as a multiplicative bonus via `(1 + utility)`.
An Engram with utility 0.0 has a multiplier of 1.0; with utility 5.0, it has a multiplier
of 6.0. Utility accumulates, giving frequently-useful Engrams exponentially increasing
priority.

**Connection to Active Inference**: Utility maps to the pragmatic value component of Expected
Free Energy. High-utility Engrams have demonstrated pragmatic value through outcomes.

### 2.4 Reputation — [0, ∞)

**What it measures**: How trustworthy is the Engram's producer at the time the Engram was
created?

**Range**: [0, ∞). Unbounded above; clamped to non-negative at construction.

**Examples**:
- An Engram from a Gate (ground truth) → reputation 1.0
- An Engram from an internal agent → reputation 0.75 (Provenance default for agents)
- An Engram from an untrusted external source → reputation 0.1
- An Engram from a model or agent with accumulated positive track record → reputation
  growing above 1.0

**Role in scoring**: Reputation directly scales the effective score. An Engram with
reputation 0.0 has zero effective score regardless of other axes — untrusted sources are
structurally excluded. An Engram with reputation 2.0 gets double the priority of one with
reputation 1.0.

**Connection to Provenance**: Reputation is initialized from the Engram's Provenance record
but can be updated as the author's track record evolves. The Score's reputation field is a
snapshot at emission time; the Provenance's trust field is the current trust level.

---

## 3. The Three Extended Axes (Specified, Not Yet Implemented)

Three additional axes are specified for the full 7-axis appraisal. These are not yet present
in the current Score struct but are planned for implementation.

### 3.1 Precision — [0, 1]

**What it measures**: How specific and well-defined is this Engram's content? Precision
captures the difference between a vague statement ("something is probably wrong") and a
specific one ("compilation fails at line 42 with error E0599").

**Role**: Used for weighting predictions and knowledge entries. High-precision Engrams are
more actionable and receive higher weight in composition decisions.

**Connection to Active Inference**: Precision weighting is central to active inference
(Friston 2010) — prediction errors are weighted by their precision to determine how much
they should update the model.

### 3.2 Salience — [0, 1]

**What it measures**: How relevant is this Engram to the current context? Salience is
context-dependent — the same Engram may be highly salient in one context and irrelevant in
another.

**Role**: Used by the VCG Attention Auction (see [17-design-principles-and-frontier-summary.md](17-design-principles-and-frontier-summary.md))
for truthful context budget allocation. High-salience Engrams bid higher for inclusion in
the context window.

**Connection to Active Inference**: Salience maps to the relevance component of attentional
selection — which signals deserve computational resources given the current goal.

### 3.3 Coherence — [0, 1]

**What it measures**: How consistent is this Engram with the system's existing knowledge base?
An Engram that contradicts well-established knowledge has low coherence; one that fits
seamlessly has high coherence.

**Role**: Used for knowledge integration decisions. Low-coherence Engrams may signal either
an error (contradiction with ground truth) or a genuine surprise (new information that updates
the model). The distinction is made by checking against Gate verdicts.

---

## 4. The Effective Score Formula

All seven axes (four stable, three extended) collapse into a single scalar via:

```
effective = confidence × (1 + novelty) × (1 + utility) × reputation
```

The current implementation (`roko-core/src/score.rs`):

```rust
impl Score {
    /// Scalar effective score combining all four axes.
    ///
    /// The formula `confidence × (1 + novelty) × (1 + utility) × reputation`
    /// was chosen so that:
    /// - zero confidence → zero effective score (false positives are worthless)
    /// - novelty and utility act as multipliers (additive bonuses to 1.0)
    /// - reputation directly scales the result
    pub fn effective(&self) -> f32 {
        self.confidence * (1.0 + self.novelty) * (1.0 + self.utility) * self.reputation
    }
}
```

### 4.1 Formula Properties

| Property | Guarantee | Why It Matters |
|---|---|---|
| `confidence = 0 → effective = 0` | Zero confidence kills the score | Invalid information is never prioritized |
| `reputation = 0 → effective = 0` | Zero reputation kills the score | Untrusted sources are structurally excluded |
| `novelty = 0 → multiplier = 1.0` | No penalty for routine information | Routine is normal, not bad |
| `novelty = 1 → multiplier = 2.0` | Novel information gets 2× priority | Surprise drives attention |
| `utility = 0 → multiplier = 1.0` | New Engrams start at baseline | No penalty for lack of history |
| `utility = n → multiplier = (1+n)` | Utility accumulates multiplicatively | Frequently-useful Engrams dominate |

### 4.2 Example Calculations

```
// A fresh, neutral Engram (builder defaults)
Score::NEUTRAL  // confidence=0.5, novelty=0, utility=0, reputation=1
→ 0.5 × 1.0 × 1.0 × 1.0 = 0.5

// A verified, novel insight from a trusted source
Score { confidence: 0.95, novelty: 0.8, utility: 0, reputation: 1.2 }
→ 0.95 × 1.8 × 1.0 × 1.2 = 2.052

// A highly-utilized playbook rule
Score { confidence: 0.9, novelty: 0, utility: 5.0, reputation: 1.0 }
→ 0.9 × 1.0 × 6.0 × 1.0 = 5.4

// An untrusted external observation
Score { confidence: 0.8, novelty: 1.0, utility: 0, reputation: 0.1 }
→ 0.8 × 2.0 × 1.0 × 0.1 = 0.16
```

---

## 5. Score Constants

Two predefined Score constants in the implementation:

```rust
impl Score {
    /// A zero score (all axes = 0). Equivalent to "no evidence".
    pub const ZERO: Self = Self {
        confidence: 0.0,
        novelty: 0.0,
        utility: 0.0,
        reputation: 0.0,
    };

    /// A neutral score (confidence=0.5, others=0). Default for unscored Engrams.
    pub const NEUTRAL: Self = Self {
        confidence: 0.5,
        novelty: 0.0,
        utility: 0.0,
        reputation: 1.0,
    };
}
```

`Score::NEUTRAL` is the default assigned by the EngramBuilder. It represents "we have no
information about this Engram's quality" — moderate confidence, no novelty signal, no
utility history, trusted author. The effective value is 0.5.

`Score::ZERO` represents "no evidence" — zero across all axes. Effective value is 0.0.

---

## 6. Score Arithmetic

Scores support element-wise arithmetic for composition:

### 6.1 Element-Wise Multiplication (Scaling)

```rust
impl Mul for Score {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self {
            confidence: (self.confidence * other.confidence).clamp(0.0, 1.0),
            novelty: (self.novelty * other.novelty).clamp(0.0, 1.0),
            utility: self.utility * other.utility,
            reputation: self.reputation * other.reputation,
        }
    }
}
```

Used when applying a per-axis modifier to a base score. For example, a RecencyScorer might
produce a modifier score where confidence is 1.0 (no change) and reputation is 0.5 (halved
for older Engrams). Multiplying this modifier with the base score scales reputation without
affecting confidence.

### 6.2 Element-Wise Addition (Aggregation)

```rust
impl Add for Score {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            confidence: (self.confidence + other.confidence).clamp(0.0, 1.0),
            novelty: (self.novelty + other.novelty).clamp(0.0, 1.0),
            utility: self.utility + other.utility,
            reputation: self.reputation + other.reputation,
        }
    }
}
```

Used when aggregating evidence from multiple Scorers. Confidence and novelty are clamped to
1.0 (they cannot exceed certainty). Utility and reputation accumulate without bound.

---

## 7. Score × Decay = Weight

An Engram's effective weight at a given time combines Score and Decay (see
[04-decay-variants.md](04-decay-variants.md)):

```
weight(t) = score.effective() × decay.apply(age_ms)
```

This is the primary ordering criterion for Substrate queries. The `weight_at()` method on
Engram computes this:

```rust
pub fn weight_at(&self, now_ms: i64) -> f32 {
    let age = now_ms - self.created_at_ms;
    self.score.effective() * self.decay.apply(age)
}
```

A highly-scored Engram with aggressive decay will eventually fall below the weight threshold
and be excluded from queries or pruned from the Substrate. This is how the system implements
"forgetting" — not by deleting information, but by letting its weight decay below the
threshold of relevance.

---

## 8. How Scorers Produce Scores

Scorers are implementations of the `Scorer` trait (see
[06-synapse-traits.md](06-synapse-traits.md)). Each Scorer is a pure function of
`(Engram, Context) → Score`:

```rust
pub trait Scorer: Send + Sync {
    fn score(&self, signal: &Signal, ctx: &Context) -> Score;
    fn name(&self) -> &'static str;
}
```

Multiple Scorers compose via `CompositeScorer` using the arithmetic operations above. A
typical scoring pipeline:

1. **RelevanceScorer**: Scores how well the Engram matches the current goal (via Context).
   Sets confidence based on semantic similarity.
2. **RecencyScorer**: Scores how recent the Engram is. Reduces confidence for stale data.
3. **ReputationScorer**: Scores based on the author's track record. Sets reputation.
4. **CatalyticScorer**: Scores based on how many downstream Engrams this one has catalyzed.
   Sets utility.

The pipeline aggregates scores via addition, then the effective score formula collapses the
result to a scalar for routing and composition decisions.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Friston 2010, Nature Reviews Neuroscience 11(2) | Precision weighting in active inference — prediction errors weighted by their precision. Foundation for the precision axis. |
| Scherer 2001, Applied AI 15 | Appraisal theory of emotion: multi-dimensional evaluation of stimuli. Theoretical basis for multi-axis scoring. |
| Damasio 1994, Descartes' Error | Somatic markers: emotional signals bias decision-making. Score as a computational somatic marker. |
| Kahneman & Tversky 1979, Econometrica 47(2) | Prospect theory: non-linear weighting of probabilities. Informs the multiplicative (not additive) combination formula. |

---

## Current Status and Gaps

- **Implemented**: 4-axis Score struct with NEUTRAL/ZERO constants, effective() formula,
  Mul/Add arithmetic, clamping, `from_confidence()` helper. All tested in `roko-core`.
- **Missing**: Extended axes (precision, salience, coherence) — specified but not yet in
  the Score struct.
- **Missing**: Extended effective formula incorporating the three new axes.

---

## Cross-References

- [02-engram-data-type.md](02-engram-data-type.md) — Score as a field on the Engram
- [04-decay-variants.md](04-decay-variants.md) — How Score and Decay combine into weight
- [06-synapse-traits.md](06-synapse-traits.md) — The Scorer trait
- [11-dual-process-and-active-inference.md](11-dual-process-and-active-inference.md) — EFE and precision weighting
