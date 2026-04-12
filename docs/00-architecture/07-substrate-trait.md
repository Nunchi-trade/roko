# The Substrate Trait

> **Abstract:** The Substrate trait is the persistence layer of the Synapse Architecture —
> every Engram in Roko is stored in and retrieved from a Substrate. This document provides
> the full trait specification, describes each method with its semantics, lists all current
> and planned implementations, explains the query model, and covers pruning, idempotence,
> and concurrency guarantees.


> **Implementation**: Shipping

---

## 1. Role in the Architecture

Substrate is the foundational trait — it provides the "ground" that all other traits operate
over. Without Substrate, there is nowhere to store Engrams, nowhere to query for candidates,
and no way to persist results. It maps to **System 1 (Operations)** in Beer's Viable System
Model (Beer 1972) and to the **working memory** and **episodic memory** components of CoALA
(Sumers et al. 2023, arXiv:2309.02427).

Substrate is the only async trait that appears in step 1 (PERCEIVE) and step 7 (PERSIST) of
the universal cognitive loop — it bookends the entire cycle.

---

## 2. Full Trait Specification

From `roko-core/src/traits.rs`:

```rust
#[async_trait]
pub trait Substrate: Send + Sync {
    /// Store an Engram. Returns its content hash. Idempotent on content.
    async fn put(&self, signal: Signal) -> Result<ContentHash>;

    /// Retrieve an Engram by content hash. Does not apply decay.
    async fn get(&self, id: &ContentHash) -> Result<Option<Signal>>;

    /// Query for Engrams matching the given filter. Impls may apply decay
    /// when evaluating min_weight and when ordering results.
    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Signal>>;

    /// Remove Engrams whose effective weight (score × decay) has fallen
    /// below threshold at ctx.now_ms. Returns count of pruned Engrams.
    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize>;

    /// Optional: total count of stored Engrams.
    async fn len(&self) -> Result<usize> { Ok(0) }

    /// Optional: is the substrate empty?
    async fn is_empty(&self) -> Result<bool> { Ok(self.len().await? == 0) }

    /// Human-readable name for logging/debugging.
    fn name(&self) -> &'static str { "unnamed_substrate" }
}
```

### 2.1 put() — Store

Stores an Engram and returns its ContentHash. Idempotent — storing the same Engram twice
(same ContentHash) is a no-op. This follows from content-addressed storage: identity IS
content.

### 2.2 get() — Retrieve

Retrieves a single Engram by its ContentHash. Returns `None` if the Engram is not found
(either never stored or pruned). Does NOT apply decay — the raw Engram is returned as stored.
Decay is applied at query time, not at retrieval time.

### 2.3 query() — Filter and Retrieve

The primary read path. The `Query` struct provides filters:

```rust
pub struct Query {
    pub kinds: Option<Vec<Kind>>,     // filter by Engram kind
    pub author: Option<String>,        // filter by author
    pub session: Option<String>,       // filter by session
    pub since_ms: Option<i64>,         // created after this timestamp
    pub until_ms: Option<i64>,         // created before this timestamp
    pub min_weight: Option<f32>,       // minimum effective weight (score × decay)
    pub tags: Vec<(String, String)>,   // all tags must match
    pub limit: Option<usize>,          // maximum results
}
```

All filters AND together. An empty `Query::all()` matches everything. Implementations may
apply decay when evaluating `min_weight` and when ordering results.

### 2.4 prune() — Garbage Collection

Removes Engrams whose effective weight has fallen below the threshold:

```
weight = score.effective() × decay.apply(ctx.now_ms - created_at_ms)
```

Returns the count of pruned Engrams. This is automatic memory management — the system
forgets information that has decayed below the threshold of relevance.

---

## 3. Implementations

### 3.1 MemorySubstrate

In-memory HashMap backend for testing. Fast, ephemeral, single-process.

```rust
pub struct MemorySubstrate {
    signals: RwLock<HashMap<ContentHash, Signal>>,
}
```

### 3.2 FileSubstrate (roko-fs)

JSONL file backend for default persistence. Each Engram is one line in `.roko/signals.jsonl`.
Provides append-only writes for crash safety and periodic compaction via `prune()`.

Located in `roko-fs`. This is the default Substrate for all Roko agents.

### 3.3 HdcSubstrate (Planned)

Hyperdimensional Computing substrate for semantic similarity queries. Engrams are encoded
as 10,240-bit HDC vectors using XOR bind and majority bundle (Kanerva 2009, Cognitive
Computation 1(2)). Queries use Hamming distance for O(1) similarity comparison.

### 3.4 ChainSubstrate (Planned)

On-chain Substrate on the Korai chain. Engram ContentHashes are posted on-chain for
attestation and shared state. Full Engram bodies are stored off-chain (IPFS or similar)
with on-chain pointers.

---

## 4. Concurrency

All Substrates are `Send + Sync`. Implementations must handle concurrent access internally:

- `MemorySubstrate` uses `RwLock<HashMap<...>>`
- `FileSubstrate` uses append-only writes with periodic compaction
- Future network Substrates use message passing or distributed locks

The three cognitive speeds (Gamma/Theta/Delta) run on separate async tasks and may access
the same Substrate concurrently. The `Send + Sync` bounds ensure this is safe.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Beer 1972, Brain of the Firm | VSM System 1 (Operations): the operational substrate. |
| Sumers et al. 2023 (arXiv:2309.02427) | CoALA: working memory and episodic memory components. |
| Kanerva 2009, Cognitive Computation 1(2) | HDC: hyperdimensional computing for similarity search. |

---

## Current Status and Gaps

- **Implemented**: `MemorySubstrate` in `roko-std`, `FileSubstrate` in `roko-fs` (37 tests).
- **Not implemented**: `HdcSubstrate`, `ChainSubstrate`.

---

## Cross-References

- [02-engram-data-type.md](02-engram-data-type.md) — What Substrates store
- [04-decay-variants.md](04-decay-variants.md) — How pruning uses decay
- [06-synapse-traits.md](06-synapse-traits.md) — Substrate in the trait overview
- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) — Substrate in steps 1 and 7
