# WASM Deployment (Browser and Edge)

> Roko's core traits and cognitive primitives compile to WebAssembly (wasm32-wasi), enabling
> agents to run in browsers, edge functions, and embedded WASM runtimes. This document covers
> what works in WASM, what does not, the MemorySubstrate alternative to filesystem access,
> HDC vector compatibility, and the target experience for browser-based agents.

---

## WASM Target Overview

The `wasm32-wasi` target compiles Roko's core cognitive primitives — the Engram type, the six
Synapse traits, scoring, routing, and composition — into WebAssembly modules that can run in any
WASM runtime (browser via wasm-bindgen, Cloudflare Workers, Fastly Compute, Deno Deploy,
wasmtime, wasmer).

This is not a full agent deployment — the complete agent loop with LLM backends, filesystem
persistence, and process supervision requires native capabilities. WASM deployment targets a
specific subset: lightweight cognitive processing, context scoring, knowledge retrieval, and
HDC-based similarity search in environments where native binaries cannot run.

### What Works in WASM

| Component | WASM Support | Notes |
|---|---|---|
| `Engram` struct | Full | All fields, serialization, content hashing (BLAKE3 compiles to WASM) |
| `Score` (7-axis) | Full | Pure computation, no I/O |
| `Scorer` trait | Full | Score computation is pure math |
| `Router` trait | Full | Selection logic is pure computation |
| `Composer` trait | Full | Context assembly under budget is pure computation |
| `Policy` trait | Full | Observation and emission logic is pure computation |
| `Gate` trait | Partial | Verification logic works, but gates that shell out (compile gate, test gate) do not |
| `Substrate` trait | Partial | `MemorySubstrate` (in-memory) works; `FileSubstrate` (JSONL on disk) does not |
| HDC vectors | Full | `HdcVector`, Hamming distance, XOR bundling — pure bit operations |
| Decay calculations | Full | `HalfLife`, `Ttl`, `Ebbinghaus` — pure math on timestamps |
| Lineage DAG | Full | `Vec<ContentHash>` — no I/O required |
| Content addressing | Full | BLAKE3 hashing compiles to WASM natively |

### What Does Not Work in WASM

| Component | Reason | Alternative |
|---|---|---|
| `FileSubstrate` (JSONL persistence) | Requires filesystem access | Use `MemorySubstrate` |
| LLM provider backends (Anthropic, OpenAI) | Requires HTTP client with TLS | Use `fetch()` via wasm-bindgen or proxy through host |
| MCP client | Requires stdio or TCP sockets | Not available in WASM |
| `ProcessSupervisor` | Requires process spawning | Not applicable in WASM |
| `roko-orchestrator` (DAG executor) | Requires filesystem, git, process spawning | Run orchestration natively, deploy cognitive kernels to WASM |
| `roko-gate` compile/test gates | Requires shell execution | Use custom gates with WASM-compatible verification |
| Tokio multi-threaded runtime | WASM is single-threaded | Use `tokio::runtime::Builder::new_current_thread()` or wasm-bindgen-futures |
| Tree-sitter parsing | C FFI dependency | Not available in WASM (use pre-computed indexes) |

---

## MemorySubstrate: In-Memory Persistence for WASM

The `MemorySubstrate` is the WASM-compatible implementation of the `Substrate` trait. It stores
Engrams in a `BTreeMap<ContentHash, Engram>` in memory, with indexed lookups by kind, tags, and
time range.

```rust
use roko_core::substrate::Substrate;
use roko_std::memory_substrate::MemorySubstrate;

// Create an in-memory substrate (no filesystem required)
let substrate = MemorySubstrate::new();

// Store an Engram
substrate.put(engram).await?;

// Query by kind
let results = substrate.query(Query::by_kind(Kind::Observation)).await?;

// Query by tag
let results = substrate.query(
    Query::by_tag("domain", "coding")
).await?;
```

The `MemorySubstrate` provides the same `Substrate` trait interface as `FileSubstrate`. Code
written against the trait works identically in both native and WASM environments. The only
difference is durability: `MemorySubstrate` loses all state when the WASM module is unloaded.

For persistence in WASM environments, the host can serialize the substrate's contents via the
`Substrate::export()` method (serializes all Engrams to a JSON array) and restore them via
`Substrate::import()`. In a browser context, this maps to `localStorage` or `IndexedDB`.

---

## HDC Vector Compatibility

Hyperdimensional Computing vectors (`HdcVector` from `roko-primitives`) are fully WASM-compatible.
The operations are pure bitwise math:

```rust
use roko_primitives::HdcVector;

// Create vectors (WASM-compatible)
let v1 = HdcVector::random(10_000);
let v2 = HdcVector::random(10_000);

// Hamming distance (pure XOR + popcount)
let similarity = v1.hamming_similarity(&v2);

// XOR bundling (associative memory)
let bundled = HdcVector::bundle(&[v1, v2]);

// Binding (structural association)
let bound = v1.bind(&v2);
```

The Hamming distance computation uses `u64::count_ones()` which compiles to efficient WASM
instructions. While WASM does not have native SIMD for popcount (unlike x86_64 AVX2 or ARM
NEON), the scalar implementation is still fast enough for real-time use: ~50μs for a 10,000-bit
vector comparison.

For environments where HDC performance is critical, WASM SIMD (the `simd128` proposal, now
widely supported in browsers) can be enabled via:

```toml
[target.wasm32-unknown-unknown]
rustflags = ["-C", "target-feature=+simd128"]
```

This enables 128-bit SIMD operations in WASM, providing approximately 4× speedup for HDC
vector operations compared to scalar WASM.

---

## Browser Deployment via wasm-bindgen

For browser-based deployment, use `wasm-bindgen` to expose Roko's cognitive primitives to
JavaScript:

```rust
use wasm_bindgen::prelude::*;
use roko_core::{Engram, Score, Kind, Body};
use roko_std::memory_substrate::MemorySubstrate;
use roko_std::default_scorer::DefaultScorer;

#[wasm_bindgen]
pub struct WasmAgent {
    substrate: MemorySubstrate,
    scorer: DefaultScorer,
}

#[wasm_bindgen]
impl WasmAgent {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            substrate: MemorySubstrate::new(),
            scorer: DefaultScorer::default(),
        }
    }

    /// Store an observation and return its content hash
    #[wasm_bindgen]
    pub async fn observe(&mut self, text: &str) -> Result<String, JsValue> {
        let engram = Engram::builder()
            .kind(Kind::Observation)
            .body(Body::Text(text.to_string()))
            .build();

        let hash = engram.id.to_string();
        self.substrate.put(engram).await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(hash)
    }

    /// Score and rank stored Engrams by relevance to a query
    #[wasm_bindgen]
    pub async fn query_ranked(&self, query: &str) -> Result<JsValue, JsValue> {
        let engrams = self.substrate.query(Query::all()).await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        let mut scored: Vec<_> = engrams.iter()
            .map(|e| (e, self.scorer.score(e)))
            .collect();

        scored.sort_by(|a, b| b.1.utility.partial_cmp(&a.1.utility).unwrap());

        serde_wasm_bindgen::to_value(&scored)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
}
```

### Build and Bundle

```bash
# Install wasm-pack
cargo install wasm-pack

# Build the WASM package
wasm-pack build --target web crates/roko-wasm

# Output: pkg/roko_wasm.js, pkg/roko_wasm_bg.wasm
```

### Usage in JavaScript

```javascript
import init, { WasmAgent } from './pkg/roko_wasm.js';

await init();

const agent = new WasmAgent();

// Store observations
await agent.observe("The API endpoint returns 404 for /users");
await agent.observe("The routing table is missing the /users route");

// Query and rank
const results = await agent.query_ranked("API error");
console.log(results); // Ranked by utility score
```

### WASM Module Size

The core cognitive primitives (Engram, Substrate, Scorer, Router, Composer) compile to a WASM
module of approximately 500KB-1MB gzipped. This is small enough for browser deployment without
code splitting.

The size breakdown:

| Component | Approximate WASM Size |
|---|---|
| Engram + Score + ContentHash | ~50KB |
| BLAKE3 (content hashing) | ~30KB |
| MemorySubstrate | ~40KB |
| DefaultScorer + DefaultRouter | ~30KB |
| DefaultComposer | ~40KB |
| HDC vectors | ~20KB |
| serde + serde_json | ~200KB |
| wasm-bindgen glue | ~50KB |
| **Total (gzipped)** | **~500KB** |

---

## Edge Function Deployment

For edge deployment (Cloudflare Workers, Fastly Compute, Deno Deploy), Roko's cognitive
primitives run as WASM modules within the edge runtime. The pattern:

```rust
// Edge worker: score incoming data against stored knowledge
use roko_core::{Engram, Score, Kind};
use roko_std::memory_substrate::MemorySubstrate;
use roko_std::default_scorer::DefaultScorer;

pub async fn handle_request(request: Request) -> Response {
    // Load pre-computed knowledge from KV store or R2
    let substrate = load_knowledge_from_kv().await;

    // Create an Engram from the incoming request
    let observation = Engram::builder()
        .kind(Kind::Observation)
        .body(Body::Text(request.text().await?))
        .build();

    // Score against existing knowledge
    let scorer = DefaultScorer::default();
    let score = scorer.score(&observation);

    // Route based on score
    if score.novelty > 0.7 {
        // High novelty — forward to full agent for processing
        forward_to_agent(observation).await
    } else {
        // Low novelty — respond from cached knowledge
        respond_from_cache(&substrate, &observation).await
    }
}
```

This pattern enables a two-tier architecture:

1. **Edge tier (WASM)**: Fast scoring, routing, and cache lookup at the network edge (~5ms
   latency). Handles 80% of requests without calling an LLM.
2. **Core tier (native)**: Full agent processing with LLM backends, filesystem persistence,
   and orchestration for the 20% of requests that need deep reasoning.

This maps to Roko's dual-process cognition model: the edge tier acts as T0 (zero-LLM probe)
processing, while the core tier handles T1/T2 (model-assisted) processing.

---

## Feature Flags for WASM Builds

When building for WASM, disable features that require native capabilities:

```bash
# Build core primitives for WASM (no filesystem, no networking)
cargo build --target wasm32-wasi -p roko-core --no-default-features --features serde,hdc,decay

# Build std with MemorySubstrate only (no FileSubstrate)
cargo build --target wasm32-wasi -p roko-std --no-default-features --features memory-substrate
```

The feature flags ensure that WASM builds do not pull in `tokio` (full), `reqwest`, `rusqlite`,
or other dependencies that require native platform capabilities.

### Conditional Compilation

Crates that support both native and WASM use conditional compilation:

```rust
// In roko-std/src/lib.rs
#[cfg(not(target_arch = "wasm32"))]
pub mod file_substrate;

#[cfg(target_arch = "wasm32")]
pub mod memory_substrate;

// Always available
pub mod default_scorer;
pub mod default_router;
pub mod default_composer;
```

This ensures that `cargo build --target wasm32-wasi` compiles without errors, excluding
native-only modules at compile time.

---

## Current Status and Limitations

As of the current implementation:

- **WASM feature flags exist** in `roko-core` and `roko-std` but have not been validated with
  end-to-end WASM builds. The conditional compilation gates are in place but the wasm-bindgen
  wrapper crate (`roko-wasm`) has not been created yet.
- **MemorySubstrate is implemented** and used in tests throughout the workspace. It is the
  default substrate for unit testing and is WASM-ready.
- **HDC vectors compile to WASM** — the `roko-primitives` crate has no platform-specific
  dependencies.
- **BLAKE3 compiles to WASM** — the `blake3` crate supports `wasm32` targets natively.

The WASM deployment target is at Tier 3H priority (see `13-current-status-and-port-allocation.md`),
meaning it is planned but not yet in the critical path. The primary deployment targets are native
(Tier 0) and Docker (Tier 1).

### Known Issues for WASM Validation

When WASM support is validated, the following items need verification:

1. `serde_json` serialization of `Score` (7-axis) roundtrips correctly in WASM
2. `BTreeMap<ContentHash, Engram>` operations in `MemorySubstrate` perform acceptably for
   collections up to 100K Engrams
3. BLAKE3 content hashing produces identical hashes in native and WASM (critical for content
   addressing across environments)
4. HDC vector similarity thresholds (e.g., 0.526 for cross-domain insight resonance) produce
   the same results in WASM and native
5. `wasm-bindgen` correctly exposes async methods (`Substrate.put()`, `Substrate.query()`)
   as JavaScript Promises
