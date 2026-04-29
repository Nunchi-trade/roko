# The Engram Data Type

> **Abstract:** The Engram is the universal datum of the Roko system. Every event, every
> piece of data, every agent output, every gate verdict, every knowledge entry, every
> prediction, every tool trace - is an Engram. Engrams are content-addressed (BLAKE3),
> scored (7-axis appraisal), decaying (four decay models), lineage-tracked (audit DAG),
> provenance-stamped (author + trust + taint), and fingerprinted with a first-class HDC
> vector. This document specifies the Engram struct in full detail, explains each field,
> describes the content-addressing and HDC fingerprinting schemes, and shows how Engrams
> flow through the Synapse Architecture.


> **Implementation**: Shipping

> **Historical note:** The shipping Rust implementation still uses the legacy `Signal`
> identifier in `roko-core`. This document uses **Engram** for the durable record and
> treats the old name as an implementation detail, not the canonical architectural term.

---

## 1. Why One Universal Type

Classical software architectures use many types: tasks, events, messages, requests, responses,
records, logs. Each type has its own schema, its own storage, its own lifecycle. Adding a new
capability means adding a new type, a new store, a new API.

Roko takes a different approach. There is exactly one data type — the **Engram** — and six
traits that operate on it. This design choice has three consequences:

1. **Universal composability**: Any Scorer can score any Engram. Any Substrate can store any
   Engram. Any Gate can verify any Engram. Components compose freely because they all speak
   the same language.

2. **Full audit trails**: Every Engram carries lineage — the ContentHashes of the parent
   Engrams it was derived from. This forms a directed acyclic graph (DAG) that can be
   traversed to explain any decision: why was this model chosen? What context was used? What
   gate verdict was rendered? Follow the lineage.

3. **Temporal dynamics**: Every Engram decays. Knowledge fades. Pheromone signals expire.
   Context becomes stale. The system's "memory" is not a static database — it is a living
   substrate where information has weight that changes over time.

The name "Engram" comes from neuroscience: a hypothetical means by which memories are stored
as biophysical changes in the brain (Semon 1904; Lashley 1950; Tonegawa et al. 2015, Science
348(6238)). In Roko, an Engram is the digital equivalent - a content-addressed unit of
cognition that persists, decays, and can be retrieved by exact address or by HDC similarity.
That similarity path matters because the Substrate can hold both a unique identity hash and a
first-class semantic fingerprint on the same record.

---

## 2. The Engram Struct

The target Engram struct (the architectural specification). The current Rust type still
uses the historical `Signal` identifier in shipping code; the fields below describe the
canonical Engram shape.

```rust
/// The universal datum of the Roko system.
///
/// See crate-level docs for the architectural role of Engram.
///
/// # Identity
///
/// An Engram's identity is its ContentHash, computed from its kind, body,
/// author, and tags. Score, decay, and fingerprint are excluded - they can
/// change without changing identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Engram {
    /// Content-addressed identity (computed from kind + body + author + tags).
    pub id: ContentHash,
    /// HDC fingerprint plus encoder metadata used for similarity and consensus.
    /// `None` only when the encoder is unavailable or explicitly disabled.
    pub fingerprint: Option<HdcFingerprint>,
    /// What kind of Engram this is.
    pub kind: Kind,
    /// The Engram's payload.
    pub body: Body,
    /// Unix milliseconds when this Engram was first emitted.
    pub created_at_ms: i64,
    /// How this Engram's weight decays over time.
    pub decay: Decay,
    /// Producer attribution and trust.
    pub provenance: Provenance,
    /// Quality score at emission time (may be recomputed by Scorers).
    pub score: Score,
    /// ContentHashes of Engrams this derived from (forms a DAG for auditing
    /// and autocatalytic metrics).
    pub lineage: Vec<ContentHash>,
    /// Arbitrary string metadata (ordered for stable hashing).
    pub tags: BTreeMap<String, String>,
}
```

The HDC fingerprint is itself a first-class value:

```rust
pub struct HdcFingerprint {
    pub vector: HdcVector,      // 10,240-bit HDC vector
    pub encoder_version: u32,   // registry version for deterministic comparison
}

```

Each field is specified in detail in the sections that follow.

---

## 3. ContentHash — Identity Through Content

An Engram's identity is its `ContentHash`: a 32-byte BLAKE3 digest computed from the
Engram's canonical encoding. Two Engrams with identical content have identical hashes.

```rust
/// A 32-byte content-addressed identifier (BLAKE3 digest).
///
/// Two Engrams with identical canonical encoding share the same ContentHash.
/// The hash is computed over the Engram's body and its identity fields, but
/// not its score, decay, or fingerprint - those can change without changing identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    /// Compute a content hash from arbitrary bytes.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Hex-encoded representation (64 chars).
    #[must_use]
    pub fn to_hex(&self) -> String {
        use std::fmt::Write;
        let mut s = String::with_capacity(64);
        for byte in self.0 {
            let _ = write!(s, "{byte:02x}");
        }
        s
    }

    /// Short form for logs/display (first 8 hex chars).
    #[must_use]
    pub fn short(&self) -> String {
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }

    /// Parse a hex string into a ContentHash. Returns None for malformed input.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
            let hi = hex_digit(chunk[0])?;
            let lo = hex_digit(chunk[1])?;
            bytes[i] = (hi << 4) | lo;
        }
        Some(Self(bytes))
    }
}
```

### 3.1 Why BLAKE3

BLAKE3 was chosen over SHA-256 for three reasons:

1. **Speed**: BLAKE3 is ~5× faster than SHA-256 on modern hardware due to its tree-based
   structure and SIMD optimizations.
2. **Streaming**: BLAKE3 supports incremental hashing, which matters when Engrams contain
   large payloads (file contents, compiled artifacts).
3. **Keyed mode**: BLAKE3 supports keyed hashing for MAC computation, useful for attestation.

### 3.2 What Is Hashed (Identity Fields)

The content hash covers:

- `kind` — the semantic type (via `kind.as_str().as_bytes()`)
- `body` — the payload (via `body.canonical_bytes()`)
- `provenance.author` — who produced this Engram
- `provenance.tainted` — whether the Engram contains untrusted data
- `lineage` — the parent ContentHashes
- `tags` — all key-value pairs in sorted order (BTreeMap guarantees sort order)

### 3.3 What Is NOT Hashed (Mutable Fields)

The content hash **excludes**:

- `score` — Scores can be recomputed by different Scorers in different contexts without
  changing what the Engram fundamentally is.
- `decay` — Decay can be adjusted (e.g., promoted from HalfLife to None for persistent
  knowledge) without changing identity.
- `created_at_ms` — Creation time is metadata, not content. Two Engrams with identical
  content produced at different times should deduplicate.
- `fingerprint` — The HDC vector is derived from semantic structure and encoder version,
  not part of identity. If the encoder changes, the fingerprint changes, not the hash.

This design means that `Substrate.put()` is **idempotent**: re-putting the same Engram
produces the same ContentHash and is a no-op in the Substrate.

### 3.4 Hash Computation (Shipping Code)

The exact hash computation from the current codebase
(`roko-core/src/signal.rs:content_hash()`):

```rust
pub fn content_hash(&self) -> ContentHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(self.kind.as_str().as_bytes());
    hasher.update(b"|");
    hasher.update(&self.body.canonical_bytes());
    hasher.update(b"|");
    hasher.update(self.provenance.author.as_bytes());
    hasher.update(b"|");
    hasher.update(&[u8::from(self.provenance.tainted)]);
    hasher.update(b"|");
    for h in &self.lineage {
        hasher.update(&h.0);
    }
    hasher.update(b"|");
    for (k, v) in &self.tags {
        hasher.update(k.as_bytes());
        hasher.update(b"=");
        hasher.update(v.as_bytes());
        hasher.update(b";");
    }
    ContentHash(*hasher.finalize().as_bytes())
}
```

Fields are separated by `|` delimiters. Tags use `key=value;` format with semicolons. The
BTreeMap guarantees lexicographic key order, making the hash deterministic regardless of
insertion order.

### 3.5 HDC fingerprint

The HDC fingerprint is the semantic access vector for an Engram. It is 10,240 bits by
default, computed deterministically from `kind` and `body` by a registered encoder, and
stored with encoder-version metadata so nodes can compare fingerprints safely across
deployments. The field is optional so the record can still land when the encoder is
unavailable or disabled, but the normal path is for `Substrate.put()` to populate it.

The encoder registry is plural: the system ships with a default encoder, but Kind-specific
or body-specific encoders can override it when a domain needs a different binding strategy.
That plurality is deliberate. A playbook, a gate verdict, and a JSON task payload do not
need the same feature extraction path, but they all need to produce a stable fingerprint
for the same encoder version.

The canonical population point is `Substrate.put()`. When an Engram is persisted, the
Substrate resolves the appropriate encoder, computes the fingerprint if the caller did not
already stage one, and writes the finalized HDC metadata alongside the durable record.

The default encoder is simple and deterministic: it hashes each word or structured token
into HDC space, permutes by position, and bundles the result. More specialized encoders
can bind tags, structured fields, or domain-specific markers, but they must preserve
determinism for a given `encoder_version`.

---

## 4. Kind — Semantic Type

The `Kind` enum tells consumers how to interpret an Engram's body. Kinds are grouped by
architectural concern and are `#[non_exhaustive]` with a `Custom(String)` escape hatch for
extensions.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    // ─── Agent runtime ───────────────────────────────
    ProcessSpawn,         // A process was spawned
    ProcessExit,          // A process exited
    AgentMessage,         // Message chunk from an agent's stream
    AgentOutput,          // Raw stdout/stderr from an agent
    TokenUsage,           // Token usage report from an LLM call
    ApprovalRequested,    // Agent requested approval for destructive op

    // ─── Verification ────────────────────────────────
    GateVerdict,          // Gate passed or failed a check
    TestResult,           // Test suite run result
    CompileDiagnostic,    // Compile error or warning

    // ─── Tasks & plans ───────────────────────────────
    Task,                 // Task description (input to agent)
    Plan,                 // Plan (collection of tasks with deps)
    PlanPhase,            // Plan transitioned phases

    // ─── Context assembly ────────────────────────────
    PromptSection,        // Single section within an assembled prompt
    ContextPack,          // Curated bundle of context for an agent
    Prompt,               // Fully-assembled prompt ready for LLM

    // ─── Routing & learning ──────────────────────────
    RouterChoice,         // Router decision (e.g., "use Claude")
    RouterFeedback,       // Feedback about a prior router choice

    // ─── Memory ──────────────────────────────────────
    Episode,              // Logged episode of an agent run
    PlaybookRule,         // Playbook rule extracted from patterns
    Skill,                // Learned reusable procedure

    // ─── Observability ───────────────────────────────
    Metric,               // Scalar measurement
    ExperimentResult,     // A/B test outcome
    ToolInvocation,       // Tool invocation record
    ToolHealthDegraded,   // Tool health below threshold

    // ─── Chain participation ─────────────────────────
    Insight,              // Shared knowledge
    Pheromone,            // Stigmergic signal (threat/opportunity/wisdom)
    Bounty,               // Bounty available for claiming
    Transaction,          // On-chain transaction
    Service,              // Service offering (OaaS marketplace)
    Prediction,           // Prediction claim (predictive foraging)

    // ─── Extension ───────────────────────────────────
    Custom(String),       // Extension kind (reverse-DNS: "com.example.my_kind")
}
```

### 4.1 Extensibility

The `Kind` enum is `#[non_exhaustive]`, meaning new variants can be added without breaking
downstream implementations. For domain-specific kinds that don't belong in the core enum,
use `Kind::Custom("com.example.widget".into())` with reverse-DNS prefixes to avoid
collisions.

This design allows the system to grow without modifying `roko-core`. A chain domain plugin
can define `Kind::Custom("chain.pheromone.threat")` for domain-specific pheromone types
without touching the kernel.

### 4.2 Kind as Dispatch Key

Kinds serve as the switchyard for dispatch throughout the system:

- A Gate might only verify Engrams of kind `GateVerdict` or `TestResult`
- A Composer might only combine `PromptSection` Engrams into a `Prompt`
- A Policy might watch for `ToolHealthDegraded` Engrams and emit circuit-breaker responses
- A Router might select among `RouterChoice` candidates based on historical feedback
- A Substrate may select a Kind-specific HDC encoder before populating `fingerprint`

---

## 5. Body — Typed Payload

The `Body` enum carries the Engram's actual content. It is tagged so consumers can determine
the payload format at runtime before decoding.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format", content = "data", rename_all = "snake_case")]
pub enum Body {
    /// Empty — the Engram is purely a marker (kind and tags carry meaning).
    Empty,
    /// UTF-8 text (logs, prompts, messages).
    Text(String),
    /// Structured JSON value.
    Json(serde_json::Value),
    /// Raw bytes (binary artifacts, compressed data).
    Bytes(Vec<u8>),
}
```

### 5.1 Body Variants

| Variant | When Used | Size Method |
|---|---|---|
| `Empty` | Marker Engrams where the Kind and tags carry all meaning (e.g., `ProcessSpawn`) | `byte_size() → 0` |
| `Text(String)` | Logs, prompts, messages, natural-language content | `byte_size() → string.len()` |
| `Json(Value)` | Structured data: tool call parameters, gate results, configuration | `byte_size() → json.to_string().len()` |
| `Bytes(Vec<u8>)` | Binary artifacts, compressed data, serialized HDC vectors | `byte_size() → bytes.len()` |

### 5.2 Canonical Encoding

For content hashing, the Body produces a canonical byte representation via
`body.canonical_bytes()`. This uses JSON serialization to ensure stability across serde
versions:

```rust
pub fn canonical_bytes(&self) -> Vec<u8> {
    serde_json::to_vec(self).unwrap_or_default()
}
```

### 5.3 Typed Decoding

Bodies support typed decoding via `as_json<T>()`, `as_text()`, and `as_bytes()`. Each
returns a `Result` that errors if the body variant doesn't match:

```rust
// Decode a JSON body into a typed value
let body = Body::from_json(&my_struct)?;
let decoded: MyStruct = body.as_json()?;

// Get text content
let text_body = Body::text("hello");
let text: &str = text_body.as_text()?;
```

---

## 6. Lineage — The Audit DAG

The `lineage` field is a vector of `ContentHash`es identifying the parent Engrams from which
this Engram was derived. This forms a directed acyclic graph (DAG) that enables:

- **Causal replay**: Trace any decision back to its inputs by following lineage chains.
- **Impact analysis**: Find all Engrams that depend on a given input.
- **Autocatalytic metrics**: Measure how many downstream Engrams an input catalyzed.
- **Forensic audit**: Reconstruct the complete chain of reasoning for any output.

### 6.1 Lineage Construction

When a Gate verifies an Engram and produces a Verdict, the Verdict Engram's lineage includes
the input Engram's ContentHash. When a Composer combines multiple Engrams, the composed
output's lineage includes all input ContentHashes. The `derive()` method on Engram
automates this:

```rust
impl Engram {
    /// Emit a derived Engram — new kind/body, but tracks this Engram as lineage.
    pub fn derive(&self, kind: Kind, body: Body) -> EngramBuilder {
        EngramBuilder::new(kind)
            .body(body)
            .lineage([self.id])
            .provenance(Provenance::agent("derived"))
    }
}
```

### 6.2 DAG Traversal

The lineage DAG can be traversed by querying the Substrate for each parent ContentHash:

```rust
// Trace an Engram's full lineage
async fn trace_lineage(
    substrate: &dyn Substrate,
    engram: &Engram,
) -> Vec<Engram> {
    let mut ancestors = Vec::new();
    let mut queue: VecDeque<ContentHash> = engram.lineage.iter().copied().collect();
    let mut seen = HashSet::new();

    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue; // already visited
        }
        if let Ok(Some(parent)) = substrate.get(&id).await {
            queue.extend(parent.lineage.iter().copied());
            ancestors.push(parent);
        }
    }
    ancestors
}
```

---

## 7. The Builder Pattern

Engrams are constructed using the builder pattern, which provides sensible defaults:

```rust
pub struct EngramBuilder {
    kind: Kind,
    body: Body,
    created_at_ms: Option<i64>,
    decay: Decay,
    provenance: Provenance,
    score: Score,
    lineage: Vec<ContentHash>,
    tags: BTreeMap<String, String>,
    fingerprint: Option<HdcFingerprint>,
}
```

### 7.1 Defaults

| Field | Default | Rationale |
|---|---|---|
| `body` | `Body::Empty` | Marker Engrams are common |
| `created_at_ms` | Current wall-clock time | Most Engrams are created "now" |
| `decay` | `Decay::None` | Conservative — explicit opt-in to decay |
| `provenance` | `Provenance::default()` (trusted, author="roko") | Internal Engrams are trusted |
| `score` | `Score::NEUTRAL` (confidence=0.5, novelty=0, utility=0, reputation=1) | Neutral until scored |
| `lineage` | Empty vec | No parents unless specified |
| `tags` | Empty BTreeMap | No metadata unless specified |
| `fingerprint` | `None` | Finalized by `Substrate.put()` using the registered HDC encoder |

### 7.2 Usage Examples

```rust
use roko_core::{Engram, Kind, Body, Decay, Provenance, Score};

// Simple task Engram
let task = Engram::builder(Kind::Task)
    .body(Body::text("implement login"))
    .tag("priority", "high")
    .build();

// Pheromone with decay
let pheromone = Engram::builder(Kind::Pheromone)
    .body(Body::text("high gas prices detected"))
    .decay(Decay::HalfLife { half_life_ms: 14_400_000 }) // 4 hours
    .provenance(Provenance::agent("chain_monitor"))
    .tag("type", "opportunity")
    .build();

// Gate verdict derived from a task Engram
let verdict = task.derive(Kind::GateVerdict, Body::text("compilation passed"))
    .score(Score::new(1.0, 0.0, 1.0, 1.0))
    .build();
// verdict.lineage == [task.id]

// Episode with JSON body
let episode_data = serde_json::json!({
    "action": "file_edit",
    "path": "src/main.rs",
    "outcome": "success",
    "tokens_used": 1500
});
let episode = Engram::builder(Kind::Episode)
    .body(Body::Json(episode_data))
    .decay(Decay::HalfLife { half_life_ms: 604_800_000 }) // 7 days
    .tag("run", "42")
    .build();
```

### 7.3 Finalization

The `build()` method computes the content hash and freezes the durable fields:

```rust
pub fn build(self) -> Engram {
    let created_at_ms = self.created_at_ms.unwrap_or_else(current_time_ms);
    let mut engram = Engram {
        id: ContentHash([0; 32]), // placeholder
        kind: self.kind,
        body: self.body,
        created_at_ms,
        decay: self.decay,
        provenance: self.provenance,
        score: self.score,
        lineage: self.lineage,
        tags: self.tags,
        fingerprint: self.fingerprint,
    };
    engram.id = engram.content_hash();
    engram
}
```

The HDC fingerprint is finalized by `Substrate.put()` rather than by the builder. That
keeps encoder selection in the storage fabric, where the registry version is known and
semantic clustering can stay consistent across nodes.

---

## 8. Effective Weight

An Engram's effective weight at a given time combines its score and its decay:

```rust
/// The effective weight of this Engram at the given current time.
/// Combines score × decay.
pub fn weight_at(&self, now_ms: i64) -> f32 {
    let age = now_ms - self.created_at_ms;
    self.score.effective() * self.decay.apply(age)
}
```

This is the primary ordering criterion for Substrate queries with `min_weight` filters.
An Engram that was highly scored at creation but has decayed significantly may fall below
the weight threshold and be excluded from query results — or pruned entirely by
`Substrate.prune()`.

Fingerprint-based lookup is a separate path: `query_similar()` uses the HDC vector and does
not depend on the scalar weight. The two signals are complementary.

---

## 9. Serde and Persistence

Engrams are fully serializable via serde. The default persistence format is JSONL
(JSON Lines) in the `FileSubstrate` (`roko-fs`), where each line is one Engram:

```json
{"id":"a1b2c3d4...","fingerprint":{"vector":"<hdc-bytes>","encoder_version":3},"kind":"task","body":{"format":"text","data":"implement login"},"created_at_ms":1712345678000,"decay":{"kind":"none"},"provenance":{"author":"roko","trust":1.0,"tainted":false,"session":null},"score":{"confidence":0.5,"novelty":0.0,"utility":0.0,"reputation":1.0},"lineage":[],"tags":{"priority":"high"}}
```

ContentHashes serialize as hex strings (64 characters). Byte bodies serialize as base64.
HDC fingerprints serialize as a vector plus encoder-version metadata. This ensures JSONL
files are valid UTF-8 throughout while preserving the ability to detect encoder mismatch.

---

## 10. Engram Properties Summary

| Property | Value | Implication |
|---|---|---|
| **Content-addressed** | BLAKE3(kind + body + author + tags) | Deduplication, integrity, addressable storage |
| **HDC fingerprinted** | 10,240-bit vector + encoder version | Similarity, consensus, analogy, semantic clustering |
| **Scored** | 7-axis (4 stable + 3 extended) | Multi-dimensional quality assessment |
| **Decaying** | 4 models (None, HalfLife, Ttl, Ebbinghaus) | Temporal dynamics, automatic memory management |
| **Lineage-tracked** | Vec&lt;ContentHash&gt; | Audit DAG, causal replay, forensic AI |
| **Provenance-stamped** | author + trust + tainted + session | Taint analysis, audit trails, reputation |
| **Typed payload** | Body enum (Empty, Text, Json, Bytes) | Runtime type checking, canonical encoding |
| **Extensible kind** | `#[non_exhaustive]` + Custom(String) | New capabilities without core changes |
| **Serializable** | serde Serialize + Deserialize | JSONL persistence, network transport |

---

## 11. Content-Addressing: Comparison with IPFS CIDs and Git Objects

Roko's `ContentHash` is a fixed 32-byte BLAKE3 digest. Two other major content-addressed
systems provide instructive comparisons.

### 11.1 Git Object Model

Git stores four object types — blobs, trees, commits, and tags — each identified by the
SHA-1/SHA-256 hash of its content. The commit graph is a Merkle-DAG where parent pointers
are content hashes. Roko's `lineage: Vec<ContentHash>` is the direct analog of Git's parent
commit pointers — both form audit DAGs traversable by hash (Merkle 1989).

### 11.2 IPFS CIDs and Self-Describing Hashes

IPFS Content Identifiers (CIDs) are **self-describing**: a CIDv1 encodes the hash function,
digest length, and content codec via the Multiformats standard:

```
CIDv1 = <multibase-prefix><version><multicodec><multihash>
multihash = <hash-function-code><digest-length><digest-bytes>
```

Roko's `ContentHash` is compact (32 bytes) but not self-describing. A 2-byte multihash
prefix (`0x1e 0x20` = "blake3-256, 32 bytes") would make hashes future-proof and
IPLD-compatible at trivial cost (34 bytes instead of 32).

### 11.3 IPLD Data Model Layer

InterPlanetary Linked Data (IPLD) adds a universal data model on top of content addressing
with **DAG-CBOR** (RFC 8949 with deterministic encoding) as the canonical codec. The key
design insight: separate the data model from the codec from the identifier from the transport.
This maximizes composability — a codec change does not invalidate existing CIDs.

---

## 12. Binary Serialization: Format Comparison

| Format | Deterministic | Self-Describing | Zero-Copy | Schema Evo | Best For |
|---|---|---|---|---|---|
| **serde_json** (current) | Yes | Yes | No | Forward | Hash stability, debugging |
| **DAG-CBOR** | Yes (RFC 8949 §4.2) | Yes | No | Forward | IPLD-compatible canonical encoding |
| **postcard** | Yes (documented) | No | No | None | Minimal binary, `no_std` |
| **rkyv** | No (arch-dependent) | No | Yes | None | Bulk HDC vector access |
| **bincode** | No | No | No | None | Fast IPC, not for hashing |

If compactness becomes important, **DAG-CBOR** would reduce canonical bytes by 30-60% while
maintaining determinism and IPLD compatibility. Keep **rkyv** for zero-copy HDC access only.

---

## 13. Schema Evolution for Content-Addressed Types

### 13.1 The Two-Partition Design

| Partition | Fields | Hashed? | Evolution |
|---|---|---|---|
| **Identity** | kind, body, author, tainted, lineage, tags | Yes | Adding creates new identity (correct) |
| **Mutable** | score, decay, created_at_ms, attestation, fingerprint | No | Evolves freely |

This mirrors Protocol Buffers' field number stability rule (Kleppmann 2017): identity fields
are frozen; mutable fields can change without breaking existing hashes.
The fingerprint belongs in the mutable partition because it is derived semantic metadata,
not identity. Its versioning is explicit so encoder migrations stay observable.

### 13.2 CRDT Compatibility

The lineage DAG is structurally compatible with Merkle-CRDTs (Sanjuán et al. 2020,
arXiv:2004.00107). If two agents independently derive Engrams from the same parent, the
union of their lineage graphs is well-defined by content addressing — deduplication is
automatic. The Score's `utility` and `reputation` axes could be modeled as grow-only CRDT
counters for distributed accumulation.

---

## 14. Information-Theoretic Properties

### 14.1 Complexity Estimation via Compression

Kolmogorov complexity K(x) — the length of the shortest program outputting x — is
uncomputable (Kolmogorov 1965; Chaitin 1966) but upper-bounded by any compressor's output:

```
complexity_ratio = len(compress(body)) / len(body)
```

High ratio (near 1.0) → incompressible → likely novel. Low ratio → highly compressible →
likely redundant. This provides a **substrate-free novelty signal** — no query needed.

### 14.2 Bayesian Surprise as Novelty

Itti & Baldi (2005) define Bayesian surprise as `S(data) = KL[P(M|data) || P(M)]`. Engrams
with high Bayesian surprise deserve high novelty scores. Schmidhuber (2010) formalizes this
as "compression progress" — the intrinsic reward from learning to compress data better.

### 14.3 MDL for the Coherence Axis

Grünwald's MDL principle (2007) provides a rigorous definition for the planned `coherence`
axis: `coherence = 1.0 - (L(D|M) / L(D|null_model))`, where the model M is the existing
corpus of same-Kind Engrams.

```rust
/// ComplexityScorer: computes local complexity as a Kolmogorov proxy.
pub struct ComplexityScorer;

impl Scorer for ComplexityScorer {
    fn score(&self, engram: &Engram, _ctx: &Context) -> Score {
        let raw = engram.body.canonical_bytes();
        if raw.is_empty() { return Score::NEUTRAL; }
        let compressed = zstd::encode_all(&raw[..], 1).unwrap_or_default();
        let ratio = compressed.len() as f32 / raw.len() as f32;
        Score { confidence: 0.5, novelty: ratio.clamp(0.0, 1.0), utility: 0.0, reputation: 1.0 }
    }
    fn name(&self) -> &'static str { "complexity_scorer" }
}
```

---

## 15. Engram Algebra: VSA Operations

Vector Symbolic Architectures (VSAs) define algebraic operations on high-dimensional vectors
that preserve compositional structure. Extending these to Engrams creates a proper algebra.

### 15.1 The Three Operations

| Operation | HDC Implementation | Engram Meaning |
|---|---|---|
| **Bind** (⊗) | XOR of HDC vectors | Associate two Engrams (key-value pair) |
| **Bundle** (⊕) | Majority vote | Create cluster centroid / composite |
| **Permute** (ρ) | Cyclic bit shift | Encode temporal ordering |

### 15.2 Record Encoding via HDC

An Engram's Score can be encoded as a single HDC vector for nanosecond-speed approximate
nearest-neighbor queries over score profiles:

```
score_hdc = BUNDLE([
    BIND(ROLE_CONFIDENCE, QUANTIZE(0.9)),
    BIND(ROLE_NOVELTY, QUANTIZE(0.3)),
    BIND(ROLE_UTILITY, QUANTIZE(5.0)),
    BIND(ROLE_REPUTATION, QUANTIZE(1.2)),
])
```

### 15.3 Algebraic Properties

| Property | Bind (XOR) | Bundle (Majority) |
|---|---|---|
| Commutative | Yes | Yes |
| Associative | Yes | Approximately |
| Identity | Zero vector | None |
| Inverse | Self-inverse (a ⊗ a = 0) | None (lossy) |

Bind forms an **abelian group**; bundle forms a **commutative semigroup**. Together they
provide the algebraic structure for compositional knowledge representation (Plate 2003;
Gayler 2004; Kleyko et al. 2022).

```rust
impl Engram {
    /// Bind: create an association between two Engrams (XOR in HDC space).
    pub fn bind(&self, other: &Engram) -> HdcVector {
        self.fingerprint
            .as_ref()
            .expect("fingerprint populated by Substrate.put()")
            .vector
            .xor(&other.fingerprint.as_ref().expect("fingerprint populated by Substrate.put()").vector)
    }

    /// Bundle: create a composite similar to ALL inputs (majority vote).
    pub fn bundle(engrams: &[Engram]) -> HdcVector {
        HdcVector::majority(
            &engrams
                .iter()
                .map(|e| e.fingerprint.as_ref().expect("fingerprint populated by Substrate.put()").vector)
                .collect::<Vec<_>>()
        )
    }

    /// Permute: encode this Engram at a sequence position.
    pub fn at_position(&self, position: usize) -> HdcVector {
        self.fingerprint
            .as_ref()
            .expect("fingerprint populated by Substrate.put()")
            .vector
            .rotate(position)
    }
}
```

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Semon 1904, Die Mneme | Coined "engram" for memory traces. |
| Tonegawa et al. 2015, Science 348(6238) | Identified engram cells in the brain — physical substrates of memory. |
| BLAKE3 (O'Connor et al. 2020) | Cryptographic hash function. 5× faster than SHA-256, streaming, SIMD-optimized. |
| Merkle 1989, Crypto '89, LNCS 435 | Content-addressed storage via hash trees. Foundation for the lineage DAG. |
| Kolmogorov 1965, Problems of Information Transmission | Algorithmic complexity — shortest program that outputs a string. |
| Grünwald 2007, MIT Press | MDL: computable proxy for Kolmogorov complexity. Foundation for coherence scoring. |
| Itti & Baldi 2005, NIPS 18 | Bayesian surprise: KL divergence as novelty measure. |
| Schmidhuber 2010, IEEE Trans. AMD 2(3) | Compression progress as intrinsic motivation / novelty reward. |
| Shapiro et al. 2011, SSS, LNCS 6976 | CRDTs: eventual consistency via join-semilattice merge. |
| Sanjuán et al. 2020, arXiv:2004.00107 | Merkle-CRDTs: content-addressed DAGs with CRDTs. |
| Plate 2003, CSLI Publications | Holographic Reduced Representations: algebraic VSA operations. |
| Gayler 2004, arXiv:cs/0412059 | MAP architecture: Multiply-Add-Permute VSA algebra. |
| Kleyko et al. 2022, ACM Computing Surveys 55(6) | Comprehensive survey of Hyperdimensional Computing. |
| Kleppmann 2017, O'Reilly | Schema evolution across serialization formats. |

---

## Current Status and Gaps

- **Shipping code**: The current Rust implementation still carries the legacy `Signal`
  identifier and builder name in `roko-core`, while the architectural spec here uses
  `Engram` and `EngramBuilder` for the durable record. `ContentHash`, `Kind`, `Body`,
  `Score`, `Decay`, and `Provenance` are all implemented and tested in `roko-core`
  (376 tests passing).
- **Fingerprint spec**: The HDC fingerprint is specified here as a first-class field and
  is expected to be populated by `Substrate.put()` using the registered encoder version.
- **Extended score axes**: Precision, salience, and coherence are specified but not yet in
  the Score struct.
- **Attestation field**: The `attestation: Option<Attestation>` field is specified in the
  architecture but not yet present in the current shipping Engram shape. See
  [05-provenance-and-attestation.md](05-provenance-and-attestation.md).

---

## Cross-References

- [03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md) — Full Score specification
- [04-decay-variants.md](04-decay-variants.md) — Decay enum details
- [05-provenance-and-attestation.md](05-provenance-and-attestation.md) — Provenance and Attestation
- [06-synapse-traits.md](06-synapse-traits.md) — The traits that operate on Engrams
- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) — How Engrams flow through the loop
- [01-naming-and-glossary.md](01-naming-and-glossary.md) — Canonical vocabulary and terminology
- [tmp/refinements/11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md) — Source refinement for the HDC fingerprint field
