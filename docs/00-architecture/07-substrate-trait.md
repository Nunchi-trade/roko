# The Substrate Trait

> **Abstract:** The Substrate trait is the storage fabric and kernel primitive of Roko's
> runtime. It persists, retrieves, queries, and prunes durable Engrams at L0, and it also
> exposes native HDC similarity queries over `HdcVector` fingerprints. It is one half of
> the two-fabric kernel story; the other half is the Bus transport fabric in
> [07b-bus-transport-fabric.md](07b-bus-transport-fabric.md). See
> `tmp/refinements/11-hyperdimensional-substrate.md` for the load-bearing proposal and
> [01-naming-and-glossary.md](01-naming-and-glossary.md) for the authoritative naming map.

> **Implementation**: Shipping

---

## 1. Role in the Architecture

Substrate is the durable storage fabric at L0. It provides the ground for Engrams:
content-addressed persistence, query-by-filter, native HDC similarity search, and pruning
by effective weight. Every subsystem that needs durable state depends on it.

Bus is the sibling fabric, not a replacement. Substrate stores Engrams; Bus moves Pulses.
Together they are the complete kernel interface for Roko's runtime. The two fabrics are
separate because the system needs two different semantics:

- Substrate favors durability, idempotence, and retrieval of records.
- Bus favors fan-out, topic routing, and bounded replay of ephemeral Pulses.

### 1.1 Two-Fabric Summary

| Fabric | Medium | Core operations | Retention model | Typical backends |
|---|---|---|---|---|
| Substrate | Engram | `put`, `get`, `query`, `query_similar`, `prune` | Long-lived storage with content identity, HDC fingerprints, and decay-aware pruning | Memory, File, HDC, Chain |
| Bus | Pulse | `publish`, `subscribe`, `replay_since`, `current_seq` | Bounded transport ring with topic routing and replay retention | BroadcastBus, MemoryBus, MultiBus, NATS/Kafka/Redpanda, ChainBus |

Substrate remains the trait for long-lived records and storage backends. The Bus chapter
describes the transport side of the same kernel story.

---

## 2. Trait Surface

From `roko-core/src/traits.rs`:

```rust
#[async_trait]
pub trait Substrate: Send + Sync {
    /// Store an Engram. Returns its content hash. Idempotent on content.
    async fn put(&self, engram: Engram) -> Result<ContentHash>;

    /// Retrieve an Engram by content hash. Does not apply decay.
    async fn get(&self, id: &ContentHash) -> Result<Option<Engram>>;

    /// Query for Engrams matching the given filter.
    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Engram>>;

    /// Query by HDC similarity against a fingerprint, returning ranked matches.
    async fn query_similar(
        &self,
        fp: &HdcVector,
        radius: f32,
        limit: usize,
        ctx: &Context,
    ) -> Result<Vec<(ContentHash, f32)>>;

    /// Remove Engrams whose effective weight has fallen below threshold.
    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize>;

    /// Optional: total count of stored Engrams.
    async fn len(&self) -> Result<usize> { Ok(0) }

    /// Optional: is the substrate empty?
    async fn is_empty(&self) -> Result<bool> { Ok(self.len().await? == 0) }

    /// Human-readable name for logging/debugging.
    fn name(&self) -> &'static str { "unnamed_substrate" }
}
```

The shape is intentionally narrow. Substrate owns persistent state and retrieval by
filter or similarity; it does not absorb transport concerns, topic routing, or replay
windows. Those belong to the Bus fabric, which is the kernel's transport primitive.

### 2.1 `put()` - Store

Stores an Engram and returns its `ContentHash`. The operation is idempotent: storing the
same Engram twice is a no-op because identity is content. When a fingerprint is not
already present, the Substrate populates the Engram's optional HDC fingerprint metadata at
insert time using the canonical encoder for that record shape, so HDC similarity becomes a
first-class property of the stored record rather than a side-table concern.

### 2.2 `get()` - Retrieve

Retrieves a single Engram by its `ContentHash`. Returns `None` if the Engram is not found
or has been pruned. `get()` returns the raw stored record, not a decay-adjusted view.

### 2.3 `query()` - Filter and Retrieve

The primary read path. Queries combine all filters:

```rust
pub struct Query {
    pub kinds: Option<Vec<Kind>>,
    pub author: Option<String>,
    pub session: Option<String>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub min_weight: Option<f32>,
    pub tags: Vec<(String, String)>,
    pub limit: Option<usize>,
}
```

Implementations may apply decay when evaluating `min_weight` and when ordering results.

### 2.4 `prune()` - Garbage Collection

Removes Engrams whose effective weight has fallen below the threshold:

```text
weight = score.effective() × decay.apply(ctx.now_ms - created_at_ms)
```

Pruning is an explicit storage concern. It is not a transport concern and does not affect
Bus replay semantics.

### 2.5 HDC similarity query and Bus bridge

`tmp/refinements/11-hyperdimensional-substrate.md` makes this capability normative. HDC
similarity is a native Substrate read primitive, not an external vector-store add-on:
every stored Engram carries a 10,240-bit `HdcVector` fingerprint, and `query_similar`
returns the nearest matches within a radius and limit.

```rust
pub trait Substrate: Send + Sync {
    async fn put(&self, engram: Engram) -> Result<ContentHash>;

    /// Native similarity search over durable records.
    async fn query_similar(
        &self,
        fp: &HdcVector,
        radius: f32,
        limit: usize,
        ctx: &Context,
    ) -> Result<Vec<(ContentHash, f32)>>;
}
```

For the current file-backed and in-memory stores, a brute-force scan is viable because
the comparison is a tight Hamming/popcount operation over a fixed-size vector, and the
stored corpus is already local. That makes the trait simple and predictable: the backend
chooses how to implement the scan, but the contract stays the same. HDC similarity is the
native way to ask the Substrate for "what is close to this record?" without introducing a
separate embedding service.

The Bus bridge is still separate and still useful:

```rust
async fn put_and_broadcast<S: Substrate, B: Bus>(
    substrate: &S,
    bus: &B,
    engram: Engram,
) -> Result<ContentHash> {
    let hash = substrate.put(engram.clone()).await?;

    // Best-effort bridge: storage succeeds first, then a Pulse is emitted for live
    // subscribers. The Pulse carries the stored Engram's identity as lineage.
    let pulse = engram.to_pulse(
        Topic::new("substrate.engram.stored"),
        0,
        PulseSource {
            component: "substrate:file".into(),
            agent_id: None,
        },
    );
    let _ = bus.publish(pulse).await;

    Ok(hash)
}
```

That bridge is the important architectural point: Substrate remains the durable store and
similarity surface, while Bus carries the notification stream that other operators can
observe without creating a layer violation. Bus-based consensus uses the transport fabric
for proposals, votes, and outcomes, while the Substrate supplies the HDC similarity needed
to compare claims, cluster agreeing records, and decide whether a consensus Pulse should
be emitted.

---

## 3. Implementations

### 3.1 MemorySubstrate

In-memory `HashMap` backend for testing. Fast, ephemeral, single-process.

```rust
pub struct MemorySubstrate {
    engrams: RwLock<HashMap<ContentHash, Engram>>,
}
```

### 3.2 FileSubstrate (roko-fs)

JSONL file backend for default persistence. It uses append-only writes for crash safety
and periodic compaction via `prune()`. It also computes or preserves each Engram's
fingerprint at `put()` time so `query_similar()` can be answered directly against stored
records.

Located in `roko-fs`. This is the default Substrate for all Roko agents.

### 3.3 HdcSubstrate (Planned)

Hyperdimensional Computing substrate for semantic similarity queries. Engrams are encoded
as 10,240-bit HDC vectors using XOR bind and majority bundle. Queries use Hamming
distance for fixed-cost similarity comparison, and the planned implementation can serve
as the specialized backend for large HDC-heavy corpora while still honoring the same
`query_similar()` contract.

### 3.4 ChainSubstrate (Planned)

On-chain Substrate on the Korai chain. Engram `ContentHash` values are posted on-chain for
attestation and shared state. Full Engram bodies are stored off-chain with on-chain
pointers.

---

## 4. Concurrency

All Substrates are `Send + Sync`. Implementations must handle concurrent access internally:

- `MemorySubstrate` uses `RwLock<HashMap<...>>`
- `FileSubstrate` uses append-only writes with periodic compaction
- Future network Substrates use message passing or distributed locks

Multiple cognitive speeds can access the same Substrate concurrently. The `Send + Sync`
bounds ensure that callers can share a substrate handle across tasks without adding
external locking.

---

## 5. Architectural Summary

The two-fabric kernel story is:

- `Substrate` persists durable Engrams and answers both filter queries and HDC similarity
  queries over stored records.
- `Bus` transports ephemeral Pulses and carries consensus and coordination traffic
  without turning storage into transport.

This chapter defines the storage side. The Bus chapter defines the transport side, including
topics, topic filters, replay, and bounded ring semantics.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Beer 1972, *Brain of the Firm* | VSM System 1 (Operations): the operational storage fabric. |
| Sumers et al. 2023 (arXiv:2309.02427) | CoALA: working memory and episodic memory components. |
| Kanerva 2009, *Cognitive Computation* 1(2) | HDC: hyperdimensional computing for similarity search. |

---

## Current Status and Gaps

- **Implemented**: `MemorySubstrate` in `roko-std`, `FileSubstrate` in `roko-fs`.
- **Not implemented**: `HdcSubstrate`, `ChainSubstrate`.

---

## Cross-References

- [01-naming-and-glossary.md](01-naming-and-glossary.md) - Canonical names for Substrate, Bus, Engram, and Pulse
- [02-engram-data-type.md](02-engram-data-type.md) - What Substrates store
- [04-decay-variants.md](04-decay-variants.md) - How pruning uses decay
- [06-synapse-traits.md](06-synapse-traits.md) - Substrate in the trait overview
- [07b-bus-transport-fabric.md](07b-bus-transport-fabric.md) - The Bus sibling fabric
- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) - Substrate in the loop
- `see tmp/refinements/03-bus-as-first-class.md`
- `see tmp/refinements/11-hyperdimensional-substrate.md`
