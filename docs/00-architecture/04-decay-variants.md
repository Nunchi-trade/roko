# Decay Variants

> **Abstract:** This chapter now treats demurrage as the primary memory model for durable Engrams. Every stored record carries a balance, pays holding cost over time, and regains credit when it is used, cited, or reinforced. That replaces the old decay-first framing for durable memory. TTL, LRU, and half-life style controls still matter for bounded-lived artifacts, but they are secondary mechanisms, not the core model for long-lived knowledge. See also
> [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md)
> and [01-naming-and-glossary.md](01-naming-and-glossary.md).

> **Implementation**: Shipping

---

## 1. Why Demurrage

Durable memory should not behave like a cache with a fixed expiry. If the system keeps everything forever, retrieval quality drops and stale knowledge crowds out current relevance. If the system uses only TTL or LRU, it throws away useful rare knowledge because the wall clock ran out or the cache was full.

Demurrage gives the system a different incentive structure:

- Useful Engrams stay warm because they are read, cited, and reinforced.
- Unused Engrams lose balance gradually instead of waiting for a hard prune.
- Knowledge that stops paying for its keep moves toward cold storage.
- The retrieval surface favors currently useful knowledge over historically cached knowledge.

That is the right model for durable memory. Half-life, TTL, and LRU are still useful, but only as constrained policies for transient artifacts or fixed-validity records.

---

## 2. The Demurrage State

The durable record carries explicit attention-economy state:

```rust
pub struct Engram {
    // ... existing fields ...

    /// Attention-credit held by this Engram.
    /// Starts at 1.0 and is reduced by demurrage unless reinforced.
    pub balance: f64,

    /// Monotonic total of holding cost paid over time.
    pub demurrage_paid: f64,

    /// Last time the Engram was charged or reinforced.
    pub last_touched_at: Timestamp,
}
```

The storage side exposes demurrage directly:

```rust
pub trait Demurrage {
    /// Charge holding cost since `last_touched_at`. Returns the new balance.
    fn charge(&mut self, engram: &mut Engram, now: Timestamp) -> f64;

    /// Reinforce an Engram when it is used, cited, or validated.
    fn reinforce(&mut self, engram: &mut Engram, kind: ReinforceKind);

    /// Compute the effective retrieval weight.
    fn effective_weight(&self, engram: &Engram) -> f64;
}

pub enum ReinforceKind {
    Cited,
    Retrieved,
    Gated,
    Surprised,
    AgentQuoted,
}
```

The rate law is intentionally simple:

```text
balance(t+Δt) = balance(t) - flat_tax * Δt - exp_tax * balance(t) * Δt
```

That gives the system a floor-aware, compounding holding cost. Reinforcement pushes balance back up, but only when the Engram earns it through actual use.

---

## 3. Effective Weight

Scorer and retrieval should read `effective_weight`, not raw decay. The old framing was:

```text
weight = score.effective() × decay.apply(age)
```

The demurrage framing is:

```text
weight = score.effective() × demurrage.effective_weight(engram)
```

That changes the shape of memory selection in three ways:

1. Age alone no longer decides relevance.
2. Reinforced knowledge can outrank older but unused knowledge.
3. Retrieval learns from use, not just passage of time.

The `ReinforceKind` value matters because different kinds of use carry different meaning:

- `Cited` means the Engram participates in lineage.
- `Retrieved` means the Engram solved a query.
- `Gated` means it survived verification.
- `Surprised` means it was informationally novel.
- `AgentQuoted` means another agent turned it into output.

This is the right place to make the attention economy visible to the Scorer, the Router, and the retrieval path.

---

## 4. Novelty Weighting

Demurrage is strongest when it is coupled to HDC similarity. Novel Engrams should earn a larger reinforcement bonus than common ones, because rare but useful knowledge is what we most want to keep.

Use the HDC fingerprint from [11-hyperdimensional-substrate.md](11-hyperdimensional-substrate.md) and see also
[tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md).

The reinforcement bonus becomes:

```text
reinforcement = bonus(kind) × novelty(engram)
novelty = 1 - max(similarity(top-K HDC neighbors))
```

That means:

- Citing a common Engram gives a small bump.
- Citing a rare Engram gives a larger bump.
- Surprising retrievals are rewarded more than routine confirmations.

This is the anti-hoarding mechanism. High-balance memory has to keep paying for its place by remaining uniquely useful.

---

## 5. Cold Tier, Freeze, Thaw

When balance reaches the floor, the Engram should not disappear from the system model. It should move to cold storage.

```rust
pub trait ColdSubstrate: Substrate {
    fn freeze(&self, hash: EngramHash) -> Result<()>;
    fn thaw(&self, hash: EngramHash) -> Result<Engram>;
}
```

The flow is:

1. `charge()` reduces balance over time.
2. Reinforcement raises balance while the Engram remains useful.
3. When balance reaches `min_balance`, the Engram is frozen.
4. Retrieval can thaw it on demand and reset balance to a starter value.
5. Thawing should emit a Bus Pulse so observers can update caches and policy state.

This is not hard deletion. It is a tier shift from hot memory to cold memory with lineage intact.

---

## 6. Legacy Decay Shapes

This chapter still keeps the older shapes because they are useful for bounded-lived artifacts and transient memory, but not as the primary durable-memory model.

| Shape | Best Use | Why It Exists |
|---|---|---|
| `None` | Identity, schema, fixed policy records | Never expires unless explicitly replaced. |
| `HalfLife` | Short-lived traces and bounded signals | Useful when a smooth time-only fade is enough. |
| `Ttl` | Strict validity windows | Appropriate when a record is either valid or invalid. |
| `Ebbinghaus` | Transient recall curves | Useful as a shaping function for older knowledge research, not as the main durable-memory policy. |

Use these when the artifact itself is naturally ephemeral. Do not use them to describe durable Engrams when the system should be learning from use, citation, and surprise.

Examples of appropriate secondary use:

- Session-scoped traces that should vanish after a fixed window.
- Tool output whose validity is strictly time-bound.
- Short-lived coordination signals that should fade even if nobody touches them.

---

## 7. Configuration Surface

```toml
[demurrage]
flat_tax_per_day      = 0.01
exp_decay_per_day     = 0.005
min_balance           = 0.0

cited_bonus           = 0.05
retrieved_bonus       = 0.02
gated_bonus           = 0.03
surprised_bonus       = 0.15
agent_quoted_bonus    = 0.08
```

These rates are not static doctrine. They can be tuned from retrieval quality, citation frequency, and thaw rate. The important point is that the system learns its own forgetting pressure instead of treating memory as a binary keep-or-drop store.

---

## 8. Observability

Demurrage needs direct instrumentation, otherwise the model is invisible and untunable.

- Balance histogram by tier.
- Thaw rate for cold-to-warm transitions.
- Reinforcement-by-kind breakdown over time.
- Attention leaderboard for the most highly retained Engrams.

Those views answer the operational questions the old decay-first model could not answer well:

- Is memory hoarding?
- Is it forgetting too fast?
- What kind of use is keeping knowledge alive?
- Which Engrams are earning their balance?

---

## 9. Why This Is Better Than Cache Thinking

LRU, TTL, and half-life all make sense in the right place, but they are not a durable-memory theory. They do not distinguish between useful and merely recent knowledge. Demurrage does.

The key difference is that holding cost is tied to actual use:

- Reinforced knowledge stays accessible.
- Unused knowledge fades.
- Novel knowledge gets credit for being rare and useful.
- Frozen knowledge can be thawed without breaking lineage.

That is an attention economy, not a cache.

---

## 10. Academic Foundations

| Citation | Contribution |
|---|---|
| Gesell, 1916 | Demurrage as a carrying cost on idle money. |
| Ebbinghaus, 1885 | Forgetting curve and retention decay. |
| Averell & Heathcote, 2011 | Exponential traces with power-law aggregates. |
| Murre & Dros, 2015 | Sleep-linked consolidation discontinuity. |
| FSRS / spaced-repetition work | Reinforcement updates strength from retrieval history. |

---

## 11. Cross-References

- [01-naming-and-glossary.md](01-naming-and-glossary.md) — canonical vocabulary for Engram, Bus, Topic, and Neuro.
- [11-hyperdimensional-substrate.md](11-hyperdimensional-substrate.md) — HDC fingerprint and similarity-driven novelty.
- [18-decay-tier-matrix.md](18-decay-tier-matrix.md) — tier progression and cold-storage rules.
- [20-configuration-schema.md](20-configuration-schema.md) — demurrage rate keys and tuning surface.
- [25-attention-as-currency.md](25-attention-as-currency.md) — attention economy framing.
- [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md) — source refinement.
