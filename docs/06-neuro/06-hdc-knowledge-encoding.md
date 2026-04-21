# HDC Knowledge Encoding

> How knowledge entries are encoded as 10,240-bit HDC vectors for similarity search, structured queries, and three-tier retrieval in Neuro.


> **Implementation**: Built

**Topic**: [Neuro — Cognitive Knowledge Layer](./INDEX.md)
**Prerequisites**: [05-hdc-operations.md](./05-hdc-operations.md) for HDC operations
**Key sources**:
- `bardo-backup/prd/shared/hdc-fingerprints.md` (three-tier search, role-filler encoding)
- `bardo-backup/prd/shared/hdc-applications.md` (episode compression, quality gates)
- `refactoring-prd/03-cognitive-subsystems.md` §1 (HDC Encoding section)
- `crates/roko-index/src/hdc.rs` (code symbol fingerprinting)
- `crates/roko-primitives/src/hdc.rs` (from_seed, text_fingerprint)
- `docs/00-architecture/02-engram-data-type.md` (first-class fingerprint field)
- `docs/00-architecture/07-substrate-trait.md` (native similarity query surface)
- `docs/00-architecture/27-temporal-knowledge-topology.md` (HDC clusters and tier progression)
- `../../tmp/refinements/11-hyperdimensional-substrate.md` (canonical refinement source)

---

## Abstract

Every Engram in Neuro carries a `fingerprint` field - a 10,240-bit Binary Spatter Code vector that encodes the record's semantic structure. The fingerprint is produced by a deterministic default encoder at insert time, so similarity search is native to the durable record rather than a side-table annotation. Queries are matched against stored fingerprints by Hamming distance, a single XOR + POPCNT operation per comparison that completes in ~13 nanoseconds.

The encoding scheme uses HDC's algebraic operations to capture the **structure** of an Engram - its kind, body, and tags in the default path, with room for kind-specific extensions - in a single fixed-size vector. Because HDC operations are compositional, the resulting vector preserves structural relationships: records about the same topic in different domains will have moderate similarity, records about the same topic in the same domain will have high similarity, and records about unrelated topics will be quasi-orthogonal (similarity ≈ 0.5).

This document covers the encoding pipeline (default encoder -> concept vectors -> role-filler bindings -> bundled fingerprint), the three-tier search strategy for large knowledge bases, the consensus/analogy uses of HDC fingerprints, and the current implementation in `roko-primitives/src/hdc.rs`.

---

## Encoding Pipeline

### Step 1: Concept Vector Generation

The first step is mapping each concept (a word, phrase, tag, or structured identifier) to a deterministic hypervector. Roko uses `HdcVector::from_seed()` for this:

```rust
// From roko-primitives/src/hdc.rs
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
    HdcVector::bundle(&[
        &kind_binding,
        &content_binding,
        &tag_binding,
    ])
}
```

### Default encoder and specialization

The default Engram encoder should remain deterministic and lightweight, while allowing kind-specific encoders to specialize the representation when needed:

```rust
pub trait HdcEncoder {
    fn encode(&self, engram: &Engram) -> HdcVector;
}

pub struct DefaultEncoder;
```

The default path should:
- hash canonical bytes for the Engram kind, body, and tags;
- bind each attribute to a stable role vector;
- bundle the resulting bindings into one fingerprint;
- let kind-specific encoders extend the base representation with extra fields when needed without changing the storage contract.

### What the fingerprint enables

The fingerprint is not only for nearest-neighbor lookup. The same vector supports three higher-order behaviors:

- **Consensus**: bundle the fingerprints of multiple candidate Engrams to check whether they converge on the same structure, even if the surface wording differs.
- **Analogy**: bind role vectors and compare the resulting fingerprints across domains to find structural matches.
- **Tier progression**: cluster similar Insight fingerprints before promotion, so the system promotes coherent neighborhoods of knowledge rather than isolated entries.

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

For small knowledge bases (<100K entries), brute-force Hamming distance scan is fast enough (~1.3 ms at 100K entries). Because every Engram already carries its fingerprint, `query_similar` is always available; the three-tier search strategy is an optimization for larger collections (collective knowledge on the Korai chain, potentially millions of entries):

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

The three-tier approach provides significant speedup for large knowledge bases while maintaining exact results (the final tier does full comparison on all surviving candidates). It does not change the similarity contract; it only narrows the candidate set before the exact Hamming pass.

### On-Chain HDC Precompile

For knowledge stored on the Korai chain, the three-tier search is implemented as a native EVM precompile:

- ~400 gas for topK=20 similarity search
- Same encoding used locally and on-chain — seamless knowledge transfer
- Bloom filter index maintained by the precompile; updated on each block

This is a custom Korai feature (not available on mainnet Ethereum) and is currently in the design phase. See topic [08-chain](../08-chain/INDEX.md) for details.

### Similarity Query Contract

The native query surface treats the fingerprint as the primary key for similarity:

```rust
pub trait HdcSimilarityStore {
    fn query_similar(
        &self,
        fingerprint: &HdcVector,
        limit: usize,
    ) -> Result<Vec<(EngramHash, f32)>>;
}
```

The returned score is similarity, not confidence. Confidence, decay, and tier still matter for ranking above the raw Hamming distance, but the vector match itself is the first-class signal.

See also [tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md), [Knowledge Query API](./10-knowledge-query-api.md), and [HDC/VSA Foundations](./04-hdc-vsa-foundations.md).

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

## Implementation Details: Automatic HDC Encoding Pipeline

### Trigger and integration

Every Engram ingested into `NeuroStore` should carry a fingerprint. The automatic encoding pipeline fills it at ingestion time, and kind-specific encoders can refine the fingerprint when the record needs domain-specific structure.

```rust
use crate::{HdcVector, ItemMemory, KnowledgeEntry, KnowledgeKind};

/// Encodes knowledge entries into HDC vectors at ingestion time.
///
/// Wired into NeuroStore::ingest() as a pre-processing step.
/// If an entry already has a fingerprint, this is a no-op.
pub struct KnowledgeHdcEncoder {
    /// Role vector registry — maps role names to their deterministic HDC vectors.
    role_registry: ItemMemory,
    /// Domain concept codebook — grows as new domains appear.
    domain_codebook: ItemMemory,
    /// Kind codebook — one entry per KnowledgeKind variant.
    kind_codebook: ItemMemory,
}

impl KnowledgeHdcEncoder {
    /// Create an encoder with the standard role and kind codebooks.
    pub fn new() -> Self {
        let mut role_registry = ItemMemory::new();
        for role in &[
            "role:kind", "role:domain", "role:topic", "role:content",
            "role:tag", "role:source", "role:risk_factor", "role:response",
            "role:pattern", "role:severity", "role:temporal", "role:confidence",
        ] {
            role_registry.insert_seeded(role);
        }

        let mut kind_codebook = ItemMemory::new();
        for kind in &[
            "insight", "heuristic", "warning", "causal_link",
            "strategy_fragment", "anti_knowledge",
        ] {
            kind_codebook.insert_seeded(kind);
        }

        Self {
            role_registry,
            domain_codebook: ItemMemory::new(),
            kind_codebook,
        }
    }

    /// Encode a knowledge entry into a 10,240-bit HDC vector.
    ///
    /// Pipeline:
    ///   1. Generate role vectors from registry
    ///   2. Map kind, tags, content to concept vectors
    ///   3. Bind each role-filler pair
    ///   4. Bundle all bindings into the final entry vector
    ///
    /// Returns the encoded vector (1,280 bytes).
    pub fn encode(&mut self, entry: &KnowledgeEntry) -> HdcVector {
        let role_kind = self.role_registry.get("role:kind")
            .copied().unwrap_or_else(|| HdcVector::from_seed(b"role:kind"));
        let role_content = self.role_registry.get("role:content")
            .copied().unwrap_or_else(|| HdcVector::from_seed(b"role:content"));
        let role_tag = self.role_registry.get("role:tag")
            .copied().unwrap_or_else(|| HdcVector::from_seed(b"role:tag"));

        // Kind binding
        let kind_str = format!("{:?}", entry.kind).to_lowercase();
        let kind_hv = HdcVector::from_seed(kind_str.as_bytes());
        let kind_binding = role_kind.bind(&kind_hv);

        // Content fingerprint binding
        let content_hv = HdcVector::from_seed(entry.content.as_bytes());
        let content_binding = role_content.bind(&content_hv);

        // Tag bundle binding
        let tag_binding = if entry.tags.is_empty() {
            HdcVector::zeros()
        } else {
            let tag_hvs: Vec<HdcVector> = entry.tags.iter()
                .map(|t| HdcVector::from_seed(t.as_bytes()))
                .collect();
            let tag_refs: Vec<&HdcVector> = tag_hvs.iter().collect();
            let tag_bundle = HdcVector::bundle(&tag_refs);
            role_tag.bind(&tag_bundle)
        };

        // Domain binding (if domain tag exists)
        let domain_binding = entry.tags.iter()
            .find(|t| t.starts_with("domain:"))
            .map(|domain_tag| {
                let domain_name = &domain_tag["domain:".len()..];
                let domain_hv = HdcVector::from_seed(domain_name.as_bytes());
                // Register domain in codebook for future lookups
                self.domain_codebook.insert(domain_name, domain_hv);
                let role_domain = self.role_registry.get("role:domain")
                    .copied().unwrap_or_else(|| HdcVector::from_seed(b"role:domain"));
                role_domain.bind(&domain_hv)
            });

        // Final bundle
        let mut components: Vec<&HdcVector> = vec![&kind_binding, &content_binding];
        if !entry.tags.is_empty() {
            components.push(&tag_binding);
        }
        let domain_binding_owned;
        if let Some(db) = domain_binding {
            domain_binding_owned = db;
            components.push(&domain_binding_owned);
        }

        HdcVector::bundle(&components)
    }
}
```

**Wiring into NeuroStore**:

```rust
// In roko-neuro/src/store.rs
impl NeuroStore {
    /// Ingest a knowledge entry, automatically computing its HDC vector
    /// if not already present.
    pub fn ingest(&mut self, mut entry: KnowledgeEntry) -> Result<()> {
        if entry.fingerprint.is_none() {
            let hv = self.encoder.encode(&entry);
            entry.fingerprint = Some(HdcFingerprint {
                vector: hv,
                encoder_version: self.encoder.version(),
            });
        }
        self.store(entry)
    }
}
```

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| Role namespace prefix | `"role:"` | Fixed | Prevents collision between roles and content concepts |
| Domain tag prefix | `"domain:"` | Configurable | Tags starting with this prefix are treated as domain bindings |
| Max tags per bundle | 50 | 10 - 100 | Beyond 50 tags, the tag bundle SNR degrades (SNR < 14) |

**Error handling**: `encode()` never fails. Missing roles fall back to `HdcVector::from_seed()` with the role name. Entries with no tags produce a zero vector for the tag binding (excluded from the final bundle). Entries with no content produce a zero vector for the content binding.

### Role vector registry

The role registry is an `ItemMemory` codebook pre-populated with standard roles at encoder construction time. The registry serves two purposes:

1. **Consistency**: All encoders use the same role vectors, because `from_seed()` is deterministic
2. **Lookup**: Given an unknown vector from a decomposed entry, the registry enables identifying which role it represents via nearest-neighbor search

**Abstract role definitions** (shared across domains):

| Role | Seed | Encodes |
|---|---|---|
| `role:kind` | `b"role:kind"` | Knowledge type (insight, heuristic, warning, ...) |
| `role:content` | `b"role:content"` | Content fingerprint |
| `role:tag` | `b"role:tag"` | Tag bundle |
| `role:domain` | `b"role:domain"` | Problem domain (rust, defi, research, ...) |
| `role:topic` | `b"role:topic"` | Specific topic within domain |
| `role:risk_factor` | `b"role:risk_factor"` | What creates risk (for cross-domain transfer) |
| `role:response` | `b"role:response"` | How to respond (for cross-domain transfer) |
| `role:pattern` | `b"role:pattern"` | Observable signal or pattern |
| `role:severity` | `b"role:severity"` | Severity level |
| `role:temporal` | `b"role:temporal"` | Time dimension |
| `role:confidence` | `b"role:confidence"` | Certainty level |
| `role:source` | `b"role:source"` | Information source |

**Initialization**: All role vectors are deterministic — `HdcVector::from_seed(b"role:kind")` always produces the same vector. The registry is built once at encoder creation and never mutates. Domain and concept codebooks grow incrementally during ingestion.

### Structured query API

Because XOR-bind distributes over majority-vote bundle, you can query specific attributes of bundled entry vectors by unbinding the role:

```rust
impl NeuroStore {
    /// Query entries by a specific role-filler combination.
    ///
    /// Example: "find all entries about Rust" →
    ///   query_by_role("role:domain", "rust")
    pub fn query_by_role(
        &self,
        role_name: &str,
        filler_name: &str,
    ) -> Vec<(usize, f32)> {
        let role_hv = HdcVector::from_seed(role_name.as_bytes());
        let filler_hv = HdcVector::from_seed(filler_name.as_bytes());
        let query = role_hv.bind(&filler_hv);

        self.entries.iter().enumerate()
            .filter_map(|(idx, entry)| {
                let entry_hv = entry.fingerprint.as_ref()?.vector;
                let sim = query.similarity(&entry_hv);
                if sim > 0.526 {
                    Some((idx, sim))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Unbind a role from an entry to recover the filler.
    ///
    /// Given an entry's HDC vector and a role name, returns the filler
    /// signal that can be looked up against a concept codebook.
    ///
    /// Example: "what is the domain of this entry?" →
    ///   unbind_role(entry_hv, "role:domain")
    ///   then nearest-neighbor lookup against domain codebook
    pub fn unbind_role(
        &self,
        entry_hv: &HdcVector,
        role_name: &str,
    ) -> HdcVector {
        let role_hv = HdcVector::from_seed(role_name.as_bytes());
        entry_hv.bind(&role_hv)  // XOR is self-inverse: bind(bind(role, filler), role) ≈ filler
    }
}
```

**Query syntax**: Callers specify role and filler as string names. The API generates HDC vectors internally. Multi-attribute queries bundle multiple role-filler bindings:

```rust
/// Find entries that match ALL specified role-filler pairs.
pub fn query_multi(
    &self,
    role_fillers: &[(&str, &str)],
) -> Vec<(usize, f32)> {
    let bindings: Vec<HdcVector> = role_fillers.iter()
        .map(|(role, filler)| {
            let role_hv = HdcVector::from_seed(role.as_bytes());
            let filler_hv = HdcVector::from_seed(filler.as_bytes());
            role_hv.bind(&filler_hv)
        })
        .collect();
    let refs: Vec<&HdcVector> = bindings.iter().collect();
    let query = HdcVector::bundle(&refs);

    // ... same scan-and-filter as query_by_role
}
```

**Threshold**: 0.526 for cross-domain queries (Bonferroni-corrected for 100K entries). Within-domain queries can use 0.52 (single-pair threshold). See [09-false-positive-math.md](./09-false-positive-math.md).

### Episode compression via bundling

Bundle multiple entries from the same episode into a single summary vector for fast pre-filtering.

```rust
/// Compress a set of knowledge entries into a single summary vector.
///
/// The summary is similar to each constituent entry, enabling fast
/// pre-filtering: check the summary first, then scan individual entries
/// only if the summary matches.
pub fn compress_episode(entries: &[&KnowledgeEntry]) -> Option<HdcVector> {
    let hvs: Vec<HdcVector> = entries.iter()
        .filter_map(|e| {
            e.fingerprint.as_ref()
                .map(|fp| fp.vector)
        })
        .collect();

    if hvs.is_empty() {
        return None;
    }

    let refs: Vec<&HdcVector> = hvs.iter().collect();
    Some(HdcVector::bundle(&refs))
}
```

**Which features are bundled**: The full entry vectors (already encoding kind, content, tags, domain). No additional feature extraction — the entry encoding already captures the relevant structure.

**Batch size**: Bundle up to 100 entries per episode summary. Beyond 100, SNR drops below 10.1 and retrieval accuracy degrades. For episodes with more than 100 entries, split into sub-episode batches and bundle each batch separately.

**Quality gate before bundling**:

```rust
/// Check whether entries are diverse enough to produce a useful bundle.
fn check_bundle_diversity(entries: &[HdcVector]) -> BundleDiversity {
    if entries.len() < 2 {
        return BundleDiversity::TooFew;
    }
    let mut total_sim = 0.0f32;
    let mut count = 0u32;
    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            total_sim += entries[i].similarity(&entries[j]);
            count += 1;
        }
    }
    let mean_sim = total_sim / count as f32;
    match mean_sim {
        s if s > 0.52 => BundleDiversity::TooSimilar,  // entries are redundant
        s if s < 0.45 => BundleDiversity::Good,         // reject: entries are too similar
        _ => BundleDiversity::Borderline,
    }
}

enum BundleDiversity {
    TooFew,
    TooSimilar,   // mean pairwise similarity > 0.52: bundling adds no information
    Borderline,   // 0.45..=0.52: bundle may have low retrieval quality
    Good,         // < 0.45: entries are diverse enough
}
```

Note: the diversity check uses the *inverse* logic of what you might expect. High pairwise similarity (> 0.52) means entries are near-duplicates — the bundle collapses to a single concept. Low pairwise similarity (< 0.45, well below the 0.5 random baseline) means entries are diverse and the bundle captures a rich set of concepts.

**Test criteria**:
- `encode()` on two entries with different kinds produces vectors with similarity < 0.55
- `encode()` on two entries with the same kind and different content produces vectors with similarity in 0.52-0.58 (shared kind structure)
- `query_by_role("role:kind", "insight")` returns entries with kind=Insight and not others
- `unbind_role(entry_hv, "role:kind")` produces a vector nearest to the entry's actual kind in the kind codebook
- `compress_episode()` on 10 entries produces a vector similar (> 0.52) to each constituent
- `compress_episode()` on empty input returns `None`
- Round-trip: `encode()` then `to_bytes()` then `from_bytes()` then `similarity()` with original returns 1.0

---

## Knowledge Ontology: Formal Schema

### The Six Knowledge Types as Formal Ontology

Each knowledge type maps to a formal ontological category with defined slots, constraints, and relationships:

```rust
/// Formal ontology schema for Neuro's six knowledge types.
///
/// Each type defines required and optional slots (role-filler pairs)
/// that structure the HDC encoding. Slots drive both storage and retrieval.
pub struct KnowledgeOntology {
    pub type_schemas: HashMap<KnowledgeKind, TypeSchema>,
}

pub struct TypeSchema {
    /// Required slots: must be present for valid encoding.
    pub required_slots: Vec<OntologySlot>,
    /// Optional slots: improve encoding quality when present.
    pub optional_slots: Vec<OntologySlot>,
    /// Relationships to other types.
    pub type_relationships: Vec<TypeRelation>,
}

pub struct OntologySlot {
    /// Role name (e.g., "role:domain", "role:pattern").
    pub role: String,
    /// Expected filler type.
    pub filler_type: FillerType,
    /// Human-readable description.
    pub description: String,
}

pub enum FillerType {
    /// Free text, encoded via text_fingerprint.
    Text,
    /// Enum from a fixed codebook (e.g., KnowledgeKind variants).
    Enum(Vec<String>),
    /// Reference to another entry (entry ID).
    Reference,
    /// Numeric value, discretized into bins.
    Numeric { min: f64, max: f64, bins: usize },
}

pub enum TypeRelation {
    /// This type can be promoted to the target type.
    PromotesTo(KnowledgeKind),
    /// This type can refute the target type.
    Refutes(KnowledgeKind),
    /// This type is composed from multiple instances of the source type.
    ComposedFrom(KnowledgeKind),
}
```

### Schema Definitions per Type

| Type | Required Slots | Optional Slots | Promotes To | Refuted By |
|---|---|---|---|---|
| **Insight** | content, domain, kind | tags, source, confidence_level | Heuristic | AntiKnowledge |
| **Heuristic** | content, domain, kind, pattern | tags, source, contexts | Playbook (D3) | AntiKnowledge |
| **Warning** | content, severity, domain, kind | affected_area, mitigation, tags | - | AntiKnowledge |
| **CausalLink** | cause, effect, domain, kind | strength, conditions, tags | - | AntiKnowledge |
| **StrategyFragment** | steps, domain, kind, problem_class | preconditions, tags | Playbook (D3) | AntiKnowledge |
| **AntiKnowledge** | content, refuted_insight_id, evidence | tags, domain | - | (immune to refutation) |

### CausalLink Encoding Schema

CausalLinks require special encoding to capture directionality:

```rust
/// CausalLink-specific HDC encoding.
///
/// Uses permutation to distinguish cause from effect:
///   causal_hv = BIND(PERM(role:cause, 1), hv_cause)
///             ⊕ BIND(PERM(role:effect, 2), hv_effect)
///             ⊕ BIND(role:domain, hv_domain)
///             ⊕ BIND(role:strength, hv_strength_bin)
///
/// The asymmetric permutation shifts (1 for cause, 2 for effect) ensure
/// that CAUSE→EFFECT has a different vector than EFFECT→CAUSE.
pub fn encode_causal_link(
    cause: &str,
    effect: &str,
    domain: &str,
    strength: f64, // 0.0 - 1.0
) -> HdcVector {
    let role_cause = HdcVector::from_seed(b"role:cause");
    let role_effect = HdcVector::from_seed(b"role:effect");
    let role_domain = HdcVector::from_seed(b"role:domain");
    let role_strength = HdcVector::from_seed(b"role:strength");

    let hv_cause = HdcVector::from_seed(cause.as_bytes());
    let hv_effect = HdcVector::from_seed(effect.as_bytes());
    let hv_domain = HdcVector::from_seed(domain.as_bytes());

    // Discretize strength into 5 bins
    let strength_bin = format!("strength:{}", (strength * 5.0).round() as u8);
    let hv_strength = HdcVector::from_seed(strength_bin.as_bytes());

    // Asymmetric permutation for directionality
    let cause_binding = role_cause.permute(1).bind(&hv_cause);
    let effect_binding = role_effect.permute(2).bind(&hv_effect);
    let domain_binding = role_domain.bind(&hv_domain);
    let strength_binding = role_strength.bind(&hv_strength);

    HdcVector::bundle(&[
        &cause_binding,
        &effect_binding,
        &domain_binding,
        &strength_binding,
    ])
}
```

---

## Knowledge Provenance Chain

### Full Lineage Tracking for Derived Knowledge

Every knowledge entry has a provenance chain that tracks its entire derivation history:

```rust
/// Full provenance chain for a knowledge entry.
///
/// Tracks how knowledge was created, from which sources,
/// through which transformations, and with what confidence at each step.
pub struct ProvenanceChain {
    /// Unique ID of this entry.
    pub entry_id: String,
    /// How this entry was created.
    pub origin: ProvenanceOrigin,
    /// Chain of transformations applied to this entry.
    pub transformations: Vec<ProvenanceTransformation>,
    /// Source entries that contributed to this entry.
    pub sources: Vec<ProvenanceSource>,
    /// Content hash at creation (for tampering detection).
    pub original_hash: ContentHash,
    /// Current content hash (should match original unless modified).
    pub current_hash: ContentHash,
}

#[derive(Debug, Clone)]
pub enum ProvenanceOrigin {
    /// Distilled from episodes by LLM (D1 stage).
    Distilled {
        episode_ids: Vec<String>,
        distiller_model: String,
        extraction_confidence: f64,
    },
    /// Promoted from lower-type entries (D2 stage: Insights → Heuristic).
    Promoted {
        source_entry_ids: Vec<String>,
        promotion_criteria: String,
    },
    /// Imported from external source.
    Imported {
        source_agent_id: Option<String>,
        source_channel: String, // "self", "mesh", "korai", "restore", "lethe"
        original_confidence: f64,
        discount_applied: f64,
    },
    /// Created by user directly.
    UserCreated {
        user_id: String,
    },
    /// Generated by Dreams consolidation.
    DreamsSynthesized {
        replay_session_id: String,
        synthesis_method: String,
    },
}

pub struct ProvenanceTransformation {
    /// What happened.
    pub operation: String, // "tier_promotion", "confidence_boost", "content_edit", etc.
    /// When it happened.
    pub timestamp: DateTime<Utc>,
    /// Who/what triggered it.
    pub trigger: String, // "gate_pass", "dreams_cycle", "user_command", etc.
    /// State before transformation.
    pub before: String,
    /// State after transformation.
    pub after: String,
}

pub struct ProvenanceSource {
    /// Source entry ID.
    pub entry_id: String,
    /// How this source contributed.
    pub contribution: String, // "primary", "supporting", "contradicting"
    /// Similarity to current entry (if HDC comparison was done).
    pub similarity: Option<f32>,
}
```

### Provenance Verification

```rust
impl ProvenanceChain {
    /// Verify the integrity of the provenance chain.
    ///
    /// Checks:
    ///   1. Content hash matches (no tampering)
    ///   2. All source entries exist in the store
    ///   3. Transformation timestamps are monotonically increasing
    ///   4. Confidence discount chain is consistent
    pub fn verify(&self, store: &NeuroStore) -> ProvenanceVerification {
        let hash_valid = self.original_hash == self.current_hash;
        let sources_valid = self.sources.iter()
            .all(|s| store.entry_exists(&s.entry_id));
        let timestamps_valid = self.transformations.windows(2)
            .all(|w| w[0].timestamp <= w[1].timestamp);

        ProvenanceVerification {
            hash_valid,
            sources_valid,
            timestamps_valid,
            overall_valid: hash_valid && sources_valid && timestamps_valid,
        }
    }
}
```

**References**: Pearl, J. (2000). *Causality*. Cambridge. (Causal structure encoding basis.)

**Test criteria**:
- CausalLink encoding: "A causes B" and "B causes A" produce different vectors (similarity < 0.55)
- CausalLink encoding: same cause-effect pair in different domains shares partial similarity (0.52-0.57)
- Provenance verification: detects when content hash mismatches
- Provenance verification: detects when source entries are missing
- Ontology schema: all six types have at least 2 required slots

---

## Current Status and Gaps

**Implemented**:
- `HdcVector::from_seed()` for deterministic concept encoding
- `text_fingerprint()` for content fingerprinting
- `fingerprint_symbol()` in `roko-index` for code symbol encoding
- Trigram-based name encoding in `roko-index`
- Role vectors per `SymbolKind` in `roko-index`
- `fingerprint: Option<HdcFingerprint>` field on `Engram`
- Basic similarity comparison in `KnowledgeStore` (HDC `MemoryIndex` feature-gated)

**Missing**:
- `KnowledgeHdcEncoder` (designed above; automatic encoding at ingestion)
- Role vector registry (designed above; `ItemMemory` with standard roles)
- Structured query API (designed above; `query_by_role`, `unbind_role`, `query_multi`)
- Episode compression via bundling (designed above; `compress_episode`)
- Three-tier search integration (see [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md))
- `ItemMemory` codebook (see [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md))
- On-chain HDC precompile

---

## Cross-References

- See [04-hdc-vsa-foundations.md](./04-hdc-vsa-foundations.md) for mathematical foundations
- See [05-hdc-operations.md](./05-hdc-operations.md) for operation details
- See [08-cross-domain-hdc-transfer.md](./08-cross-domain-hdc-transfer.md) for cross-domain structural analogy
- See [09-false-positive-math.md](./09-false-positive-math.md) for similarity threshold selection
- See [10-knowledge-query-api.md](./10-knowledge-query-api.md) for the NeuroStore query API
- See topic [15-code-intelligence](../15-code-intelligence/INDEX.md) for the `roko-index` HDC encoding
