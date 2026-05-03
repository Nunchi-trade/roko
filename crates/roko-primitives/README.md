# roko-primitives

Pure compute primitives with zero internal workspace dependencies. Two things live here: a 10,240-bit hyperdimensional computing vector and a three-tier inference router. If you need either without pulling in the full platform stack, this is the crate.

## Install

```toml
[dependencies]
roko-primitives = { git = "https://github.com/nunchi/roko", path = "crates/roko-primitives" }

# With zero-copy serialization for memory-mapped indexes
roko-primitives = { git = "https://github.com/nunchi/roko", path = "crates/roko-primitives", features = ["rkyv"] }
```

External deps: `serde`, `uuid`, optionally `rkyv`. Nothing else. `#![deny(unsafe_code)]`.

## HdcVector

A 10,240-bit binary sparse distributed representation stored as `[u64; 160]`. 1,280 bytes, `Copy`, stack-allocated, no heap.

### Why HDC over float embeddings

Similarity comparison runs at ~50 ns (XOR + popcount) vs 10-50 ms for typical float embedding cosine similarity. That's a 200,000x speedup on encoding and 20-100x on search. The tradeoff is lower fidelity per dimension, which HDC compensates with width (10,240 bits).

For code intelligence, HDC fingerprints catch structural similarity between functions and types without calling an embedding API. A renamed function with the same parameter types and return shape will match. No network round-trip, no model download, no GPU.

### Construction

```rust
use roko_primitives::HdcVector;

let random = HdcVector::random();           // seeded from UUID v4
let zero   = HdcVector::zeros();            // all bits clear
let stable = HdcVector::from_seed(b"swap"); // deterministic from byte slice
```

`from_seed` uses FNV-1a followed by splitmix64. Fast, deterministic, no crypto overhead.

### Core operations

```rust
// Bind: XOR. Involution — bind(bind(a, b), b) == a.
// Use to associate two concepts.
let bound = a.bind(&b);
assert_eq!(bound.bind(&b).similarity(&a), 1.0);

// Bundle: majority vote across vectors, bit by bit. Ties go to 0.
// Use to form a superposition of multiple concepts.
let bundled = HdcVector::bundle(&[&a, &b, &c]);

// Similarity: normalized Hamming distance in [0.0, 1.0].
// Random vectors score ~0.5. Identical vectors score 1.0.
let sim = a.similarity(&b);

// Permute: cyclic bit rotation. Encode position in a sequence.
let rotated = a.permute(3);
```

### Serialization

```rust
let bytes: [u8; 1280] = vec.to_bytes();    // little-endian
let recovered = HdcVector::from_bytes(&bytes);
assert_eq!(vec, recovered);
```

### How fingerprinting works

`mori-index` builds HDC fingerprints from code symbols using this crate:

1. **Role vector** — deterministic seed per `SymbolKind` (function, struct, trait, etc.)
2. **Name vector** — overlapping 3-char trigrams bundled together
3. **Parameter vector** — type names extracted from the signature, each seeded and bundled
4. **Final** — bundle name + params, bind with role

The result captures structural identity. Two functions with the same shape but different names score high similarity. A function and a struct with the same name score low.

### Performance characteristics

| Operation | Time | Notes |
|-----------|------|-------|
| `from_seed` | ~10 ns | FNV-1a + splitmix64, no crypto |
| `bind` (XOR) | ~5 ns | 160 u64 XORs |
| `bundle` | ~50 ns | Bit-by-bit majority vote |
| `similarity` | ~50 ns | XOR + POPCNT across 160 words |
| `to_bytes` / `from_bytes` | ~10 ns | memcpy-level |

All operations are branchless on the hot path. `count_ones()` compiles to hardware POPCNT on x86. The 1,280-byte vector fits in L1 cache.

### Zero-copy with rkyv

Enable the `rkyv` feature to skip deserialization when reading vectors from memory-mapped buffers (e.g., a LanceDB column or an mmap'd snapshot file):

```rust
// Compare directly against an archived vector without deserializing
let sim = live_vec.similarity_archived(&archived_vec);
```

Assumes little-endian platform. The archived layout is bit-identical to in-memory, so comparison is just pointer arithmetic + POPCNT.

## InferenceTier

Three-tier model routing for cost control:

```rust
use roko_primitives::{InferenceTier, TierRouter};

// T0: suppress — no LLM call at all
// T1: analyze  — Haiku-class (cheap, fast)
// T2: deliberate — Opus or Sonnet based on vitality

let tier = InferenceTier::T2;
let model = TierRouter::select_model(tier, 0.8);
assert_eq!(model, Some("claude-opus-4-6"));
```

### Routing table

| Tier | Vitality | Result |
|------|----------|--------|
| T0 | any | `None` (no call) |
| T1 | any | `"claude-haiku-4-5"` |
| T2 | >= 0.3 | `"claude-opus-4-6"` |
| T2 | < 0.3 | `"claude-sonnet-4-6"` |

`TierRouter` is a zero-sized unit struct. `select_model` is a pure function with no state and no allocations. All model selection in the workspace flows through this one function.

The vitality threshold at 0.3 is exact. At exactly 0.3, you get Opus.

### Conversions

```rust
let tier = InferenceTier::try_from(2u8)?; // Ok(T2)
let val: u8 = tier.into();                // 2
let bad = InferenceTier::try_from(5u8);   // Err(TierError(5))
```

## Use cases

- **Code search** — `mori-index` fingerprints every function, struct, and trait with HDC vectors for instant structural similarity search across a codebase
- **Semantic caching** — `bardo-gateway` uses SimHash (a simplified HDC variant) to match semantically similar LLM prompts without calling an embedding model
- **Pattern learning** — the cybernetic learning system fingerprints task episodes with HDC, clustering similar outcomes to extract reusable patterns
- **Model routing** — every LLM request in the system routes through `TierRouter` to pick the cheapest model that can handle the task

## Architecture

```
src/
├── lib.rs   # re-exports HdcVector, InferenceTier, TierError, TierRouter
├── hdc.rs   # HdcVector: 10,240-bit vector with bind/bundle/similarity
└── tier.rs  # InferenceTier, TierRouter, TierError
```

## License

MIT/Apache-2.0
