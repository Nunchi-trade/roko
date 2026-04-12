# HDC Knowledge Encoding

> How knowledge entries are encoded as 10,240-bit HDC vectors for similarity search, structured queries, and three-tier retrieval in Neuro.

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [05-hdc-operations.md](./05-hdc-operations.md) for HDC operations
**Key sources**:
- `bardo-backup/prd/shared/hdc-fingerprints.md` (three-tier search, role-filler encoding)
- `bardo-backup/prd/shared/hdc-applications.md` (episode compression, quality gates)
- `refactoring-prd/03-cognitive-subsystems.md` §1 (HDC Encoding section)
- `crates/roko-index/src/hdc.rs` (code symbol fingerprinting)
- `crates/bardo-primitives/src/hdc.rs` (from_seed, text_fingerprint)

---

## Abstract

Every `KnowledgeEntry` in Neuro can optionally carry an `hdc_vector` field — a 10,240-bit Binary Spatter Code vector that encodes the entry's semantic structure. This vector enables sub-millisecond similarity search without any external vector database: queries are matched against stored vectors by Hamming distance, a single XOR + POPCNT operation per comparison that completes in ~13 nanoseconds.

The encoding scheme uses HDC's algebraic operations to capture the **structure** of a knowledge entry — its type, domain, topic, tags, and content fingerprint — in a single fixed-size vector. Because HDC operations are compositional, the resulting vector preserves structural relationships: entries about the same topic in different domains will have moderate similarity, entries about the same topic in the same domain will have high similarity, and entries about unrelated topics will be quasi-orthogonal (similarity ≈ 0.5).

This document covers the encoding pipeline (text → concept vectors → role-filler bindings → bundled entry vector), the three-tier search strategy for large knowledge bases, and the current implementation in `roko-index/src/hdc.rs`.

---

## Encoding Pipeline

### Step 1: Concept Vector Generation

The first step is mapping each concept (a word, phrase, tag, or structured identifier) to a deterministic hypervector. Roko uses `HdcVector::from_seed()` for this:

```rust
// From bardo-primitives/src/hdc.rs
pub fn from_seed(seed: &[u8]) -> Self {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for &byte in seed {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3); // FNV prime
    }
    if hash == 0 { hash = 0xA5A5_A5A5_5A5A_5A5A; }
    let mut bits = [0u64; 160];
    for word in &mut bits {
        *word = splitmix64(&mut hash);
    }
    Self { bits }
}
```

**Properties**:
- **Deterministic**: `from_seed(b"rust")` always produces the same vector
- **Quasi-orthogonal**: `from_seed(b"rust")` and `from_seed(b"python")` have similarity ≈ 0.5
- **Zero external dependency**: No embedding API, no GPU, no model

### Step 2: Role Vector Assignment

Roles are named dimensions that give semantic meaning to bindings. Each role has a fixed, deterministic vector:

```rust
let role_domain   = HdcVector::from_seed(b"role:domain");
let role_topic    = HdcVector::from_seed(b"role:topic");
let role_type     = HdcVector::from_seed(b"role:type");
let role_content  = HdcVector::from_seed(b"role:content");
let role_tag      = HdcVector::from_seed(b"role:tag");
```

Role vectors are quasi-orthogonal to each other and to all concept vectors, because they use different seeds. The `role:` prefix namespace prevents collision with content concepts.

### Step 3: Role-Filler Binding

Each attribute of a knowledge entry is encoded as a role-filler binding:

```rust
// Encode entry attributes
let domain_binding   = role_domain.bind(&HdcVector::from_seed(b"rust"));
let topic_binding    = role_topic.bind(&HdcVector::from_seed(b"borrow-checker"));
let type_binding     = role_type.bind(&HdcVector::from_seed(b"insight"));
let content_binding  = role_content.bind(&text_fingerprint(&entry.content));
```

Each binding produces a vector quasi-orthogonal to both the role and the filler. The binding `domain_binding` represents "Rust in the domain role" — it is distinct from bare `hv_rust` and from `role_domain` alone.

### Step 4: Tag Encoding

Tags are bundled together (since an entry can have multiple tags) and then bound to the tag role:

```rust
let tag_vectors: Vec<HdcVector> = entry.tags.iter()
    .map(|tag| HdcVector::from_seed(tag.as_bytes()))
    .collect();
let tag_refs: Vec<&HdcVector> = tag_vectors.iter().collect();
let tag_bundle = HdcVector::bundle(&tag_refs);
let tag_binding = role_tag.bind(&tag_bundle);
```

### Step 5: Final Bundle

All role-filler bindings are bundled into the entry's HDC vector:

```rust
let entry_hv = HdcVector::bundle(&[
    &domain_binding,
    &topic_binding,
    &type_binding,
    &content_binding,
    &tag_binding,
]);
```

This produces a single 1,280-byte vector that encodes the entry's complete structure. The resulting vector is similar to each constituent binding — a query for "Rust domain" will match entries that have Rust in the domain role.

### Complete Example

Encoding a complete knowledge entry:

```rust
fn encode_knowledge_entry(entry: &KnowledgeEntry) -> HdcVector {
    // Role vectors (deterministic, shared across all entries)
    let role_kind    = HdcVector::from_seed(b"role:kind");
    let role_content = HdcVector::from_seed(b"role:content");
    let role_tag     = HdcVector::from_seed(b"role:tag");

    // Kind binding
    let kind_str = format!("{:?}", entry.kind).to_lowercase();
    let kind_binding = role_kind.bind(&HdcVector::from_seed(kind_str.as_bytes()));

    // Content fingerprint
    let content_binding = role_content.bind(&text_fingerprint(&entry.content));

    // Tag bundle
    let tag_vectors: Vec<HdcVector> = entry.tags.iter()
        .map(|t| HdcVector::from_seed(t.as_bytes()))
        .collect();
    let tag_refs: Vec<&HdcVector> = tag_vectors.iter().collect();
    let tag_bundle = if tag_refs.is_empty() {
        HdcVector::zeros()
    } else {
        HdcVector::bundle(&tag_refs)
    };
    let tag_binding = role_tag.bind(&tag_bundle);

    // Final bundle
    HdcVector::bundle(&[&kind_binding, &content_binding, &tag_binding])
}
```

---

## Code Symbol Fingerprinting (roko-index)

The `roko-index/src/hdc.rs` module provides a specialized HDC encoding for code symbols (functions, structs, traits, modules, etc.). This is a concrete example of domain-specific HDC encoding built on the same primitives.

### Role Vectors per SymbolKind

```rust
// From roko-index/src/hdc.rs (paraphrased)
fn role_vector_for_kind(kind: SymbolKind) -> HdcVector {
    let seed = match kind {
        SymbolKind::Function  => b"symbol:function" as &[u8],
        SymbolKind::Struct    => b"symbol:struct",
        SymbolKind::Trait     => b"symbol:trait",
        SymbolKind::Enum      => b"symbol:enum",
        SymbolKind::Module    => b"symbol:module",
        SymbolKind::Constant  => b"symbol:constant",
        SymbolKind::TypeAlias => b"symbol:type_alias",
        // ... other symbol kinds
    };
    HdcVector::from_seed(seed)
}
```

### Trigram-Based Name Encoding

Symbol names are encoded using character trigrams (3-character sliding windows), which captures sub-word structure:

```rust
fn encode_name(name: &str) -> HdcVector {
    let padded = format!("__{name}__"); // padding for edge trigrams
    let trigrams: Vec<HdcVector> = padded
        .as_bytes()
        .windows(3)
        .enumerate()
        .map(|(pos, trigram)| {
            let trigram_hv = HdcVector::from_seed(trigram);
            trigram_hv.permute(pos) // position-encode each trigram
        })
        .collect();
    let refs: Vec<&HdcVector> = trigrams.iter().collect();
    HdcVector::bundle(&refs)
}
```

This encoding means that names sharing substrings have moderate similarity: `parse_config` and `parse_input` will have higher similarity than `parse_config` and `render_output`, because they share the `par`, `ars`, `rse` trigrams.

### Symbol Fingerprint

The final symbol fingerprint bundles the kind role vector with the name encoding:

```rust
pub fn fingerprint_symbol(kind: SymbolKind, name: &str) -> HdcVector {
    let kind_hv = role_vector_for_kind(kind);
    let name_hv = encode_name(name);
    HdcVector::bundle(&[&kind_hv, &name_hv])
}
```

---

## Three-Tier Search Strategy

For small knowledge bases (<100K entries), brute-force Hamming distance scan is fast enough (~1.3 ms at 100K entries). For larger collections (collective knowledge on the Korai chain, potentially millions of entries), a three-tier search strategy is used:

### Tier 1: Bloom Filter (Fast Reject)

A Bloom filter with an LSH (Locality-Sensitive Hashing) scheme provides fast rejection of clearly dissimilar entries. Each vector is hashed into multiple Bloom filter buckets using random hyperplanes as hash functions. The Bloom filter is much smaller than the full vector index and fits in L1/L2 cache.

**Expected rejection rate**: 90–95% of entries are eliminated in this tier, reducing the candidate set to ~5–10% of the total.

**Cost**: ~100 ns per query (hash computation + Bloom filter lookup).

### Tier 2: Approximate Search (Coarse)

Surviving candidates from Tier 1 are compared using a **reduced-precision Hamming distance** — comparing only a subset of the 160 u64 words (e.g., the first 32 words = 2,048 bits). This provides a coarse similarity estimate that further prunes the candidate set.

**Expected reduction**: 80–90% of Tier 1 survivors are eliminated.

**Cost**: ~3 ns per comparison (32 XOR + POPCNT vs. 160 for full comparison).

### Tier 3: Exact Top-K (Full Comparison)

The final candidates undergo full 10,240-bit Hamming distance comparison. The top K most similar entries are returned.

**Expected candidate set size**: 10–100 entries (from an initial pool of 100K+).

**Cost**: ~13 ns per comparison × 10–100 candidates = 130 ns – 1.3 µs.

### Overall Search Performance

| Knowledge Base Size | Brute Force | Three-Tier |
|---|---|---|
| 1,000 | 13 µs | 13 µs (no benefit) |
| 10,000 | 130 µs | ~50 µs |
| 100,000 | 1.3 ms | ~200 µs |
| 1,000,000 | 13 ms | ~500 µs |

The three-tier approach provides significant speedup for large knowledge bases while maintaining exact results (the final tier does full comparison on all surviving candidates).

### On-Chain HDC Precompile

For knowledge stored on the Korai chain, the three-tier search is implemented as a native EVM precompile:

- ~400 gas for topK=20 similarity search
- Same encoding used locally and on-chain — seamless knowledge transfer
- Bloom filter index maintained by the precompile; updated on each block

This is a custom Korai feature (not available on mainnet Ethereum) and is currently in the design phase. See topic [08-chain](../08-chain/INDEX.md) for details.

---

## Structured Queries

Because bind distributes over bundle, HDC enables **structured queries** — querying for specific attributes of a knowledge entry without decoding it:

### "What entries are about Rust?"

```rust
let query = HdcVector::from_seed(b"role:domain")
    .bind(&HdcVector::from_seed(b"rust"));
// Compare `query` against all stored entry vectors via similarity
// Entries with "rust" in the domain role will have high similarity
```

### "What entries are Insights about async?"

```rust
let kind_query = HdcVector::from_seed(b"role:kind")
    .bind(&HdcVector::from_seed(b"insight"));
let topic_query = HdcVector::from_seed(b"role:topic")
    .bind(&HdcVector::from_seed(b"async"));
let combined_query = HdcVector::bundle(&[&kind_query, &topic_query]);
// Entries that are Insights AND about async will have highest similarity
```

### Unbinding for Decomposition

Given an entry vector, you can extract specific attributes by unbinding the role:

```rust
// What is the domain of this entry?
let domain_signal = entry_hv.bind(&HdcVector::from_seed(b"role:domain"));
// Compare domain_signal against domain codebook (hv_rust, hv_python, hv_defi, ...)
// The closest match is the entry's domain
```

This unbinding is approximate (because the entry vector is a bundle of multiple bindings), but with 5–10 role-filler pairs and D = 10,240, the SNR is high enough (SNR ≈ 32–45) for reliable retrieval.

---

## Episode Compression via Bundling

Neuro can compress multiple related knowledge entries into a single "summary vector" using bundling. This is used for:

1. **Episode summaries**: Bundle all knowledge entries extracted from a single episode into a summary vector. The summary can be queried for any of the constituent entries.

2. **Topic summaries**: Bundle all entries with a given tag into a topic summary. This provides a compact representation of "everything the agent knows about X."

3. **Temporal summaries**: Bundle entries from a time window (e.g., "last 7 days") into a temporal snapshot.

The quality of a bundle is bounded by the SNR formula: `SNR = √(D/K)`. For K = 50 entries bundled at D = 10,240, SNR = 14.3 — sufficient for detecting whether a query is related to the bundle's content, though individual entries may not be retrievable with high precision. For precise retrieval, the bundle serves as a fast pre-filter: check similarity against the bundle first, then scan individual entries only if the bundle matches.

### Quality Gates for Episode Bundles

Before bundling, a quality gate checks that the constituent entries are sufficiently diverse:

```
diversity_score = mean pairwise Hamming distance of entries
if diversity_score < 0.45:
    reject — entries are too similar, bundling adds no information
if diversity_score > 0.52:
    accept — entries are diverse enough for a useful bundle
else:
    warn — borderline diversity, bundle may have low retrieval quality
```

---

## Academic Foundations

- Kanerva, P. (2009). "Hyperdimensional Computing." *Cognitive Computation*, 1(2), 139–159. (Foundational BSC encoding)
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). (Capacity bounds for bundled encoding)
- Frady, E. P., Kleyko, D., & Sommer, F. T. (2020). "A Theory of Sequence Indexing and Working Memory in Recurrent Neural Networks." *Neural Computation*, 32(12). (Resonator networks for unbinding)
- Thomas, A., Dasgupta, S., & Bhatt, T. (2021). "A Theoretical Perspective on Hyperdimensional Computing." *JAIR*, 72. (Trigram encoding, capacity scaling)
- Neubert, P., Schubert, S., & Protzel, P. (2019). "An Introduction to Hyperdimensional Computing for Robotics." *KI*, 33, 319–330. (Place cell analogy, practical encoding schemes)

---

## Current Status and Gaps

**Implemented**:
- `HdcVector::from_seed()` for deterministic concept encoding
- `text_fingerprint()` for content fingerprinting
- `fingerprint_symbol()` in `roko-index` for code symbol encoding
- Trigram-based name encoding in `roko-index`
- Role vectors per `SymbolKind` in `roko-index`
- `hdc_vector: Option<Vec<u8>>` field on `KnowledgeEntry`
- Basic similarity comparison in `KnowledgeStore` (HDC `MemoryIndex` feature-gated)

**Missing**:
- Automatic HDC encoding during knowledge ingestion (currently the `hdc_vector` field is optional and often empty)
- Role vector registry for knowledge entry encoding (domain, topic, type, content roles)
- Three-tier search (Bloom filter → approximate → exact)
- Structured query support (unbinding roles from bundled entry vectors)
- Episode compression via bundling
- `ItemMemory` codebook for named concept lookup
- On-chain HDC precompile

---

## Cross-references

- See [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md) for mathematical foundations
- See [05-hdc-operations.md](./05-hdc-operations.md) for operation details
- See [08-cross-domain-hdc-transfer.md](./08-cross-domain-hdc-transfer.md) for cross-domain structural analogy
- See [09-false-positive-math.md](./09-false-positive-math.md) for similarity threshold selection
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for the NeuroStore query API
- See topic [15-code-intelligence](../15-code-intelligence/INDEX.md) for the `roko-index` HDC encoding
