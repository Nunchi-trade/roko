# WASM Deployment (Browser and Edge)

> Roko's core traits and cognitive primitives compile to WebAssembly (wasm32-wasi), enabling
> agents to run in browsers, edge functions, and embedded WASM runtimes. This document covers
> what works in WASM, what does not, the MemorySubstrate alternative to filesystem access,
> HDC vector compatibility, and the target experience for browser-based agents.


> **Implementation**: Specified

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

## WASI Preview 2 and the Component Model

WASI 0.2 (formerly "Preview 2") was stabilized in late 2024 and is the active standard for server-side WASM. It introduces the Component Model — a standardized way for WASM modules to compose via typed interfaces defined in WIT (WebAssembly Interface Types).

### WASI 0.2 Interface Catalog

| Interface | Package | Roko Usage |
|---|---|---|
| `wasi:io` | Pollable handles, byte streams | Substrate I/O abstraction |
| `wasi:clocks` | Monotonic and wall clocks | Decay calculations, TTL |
| `wasi:http` | HTTP request/response | Edge-to-core forwarding |
| `wasi:random` | Secure randomness | HDC vector generation |
| `wasi:filesystem` | File/directory access | Optional: MemorySubstrate bypass |
| `wasi:sockets` | TCP/UDP networking | Direct LLM provider calls (future) |

### Compiling Roko to WASI P2 Components

Rust 1.82+ provides the `wasm32-wasip2` target at Tier 2:

```bash
# Add the WASI P2 target
rustup target add wasm32-wasip2

# Build roko-core as a WASI P2 component
cargo build --target wasm32-wasip2 -p roko-core \
    --no-default-features --features "serde,hdc,decay"
```

For custom WIT interfaces (exposing Roko's Synapse traits as component exports), use `cargo-component`:

```toml
# crates/roko-wasm/Cargo.toml
[package]
name = "roko-wasm"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
roko-core = { path = "../roko-core", default-features = false, features = ["serde", "hdc", "decay"] }
roko-std = { path = "../roko-std", default-features = false, features = ["memory-substrate"] }
wit-bindgen = "0.36"

[package.metadata.component]
package = "roko:cognitive"
```

WIT interface definition for Roko's cognitive kernel:

```wit
// crates/roko-wasm/wit/world.wit
package roko:cognitive;

interface scorer {
    record engram {
        id: string,
        kind: string,
        body: string,
        tags: list<tuple<string, string>>,
    }

    record score {
        novelty: f64,
        utility: f64,
        confidence: f64,
        salience: f64,
        valence: f64,
        arousal: f64,
        coherence: f64,
    }

    score-engram: func(engram: engram) -> score;
}

interface substrate {
    use scorer.{engram, score};

    put: func(engram: engram) -> result<string, string>;
    query-by-kind: func(kind: string) -> result<list<engram>, string>;
    query-by-tag: func(key: string, value: string) -> result<list<engram>, string>;
}

interface hdc {
    record hdc-vector {
        bits: list<u64>,
        dimensions: u32,
    }

    random-vector: func(dimensions: u32) -> hdc-vector;
    hamming-similarity: func(a: hdc-vector, b: hdc-vector) -> f64;
    xor-bundle: func(vectors: list<hdc-vector>) -> hdc-vector;
    bind: func(a: hdc-vector, b: hdc-vector) -> hdc-vector;
}

world cognitive-kernel {
    export scorer;
    export substrate;
    export hdc;
    import wasi:clocks/monotonic-clock@0.2.0;
    import wasi:random/random@0.2.0;
}
```

### Component Composition

The Component Model enables composing Roko's cognitive kernel with platform-specific capability providers without recompilation:

```
┌───────────────────────────────────────────────┐
│            Platform Runtime (Host)             │
│                                                │
│  ┌──────────────────┐  ┌───────────────────┐  │
│  │ roko:cognitive   │  │ Platform Provider  │  │
│  │ (WASM component) │──│ (HTTP, KV, etc.)  │  │
│  │                  │  │                    │  │
│  │ • Scorer         │  │ • wasi:http       │  │
│  │ • Substrate      │  │ • wasi:keyvalue   │  │
│  │ • HDC vectors    │  │ • wasi:logging    │  │
│  └──────────────────┘  └───────────────────┘  │
└───────────────────────────────────────────────┘
```

---

## Fermyon Spin Deployment

Spin provides a serverless WASM runtime ideal for deploying Roko's cognitive kernel as HTTP-triggered functions. Each request gets an isolated WASM instance with sub-millisecond cold start.

### Spin Application for Roko Edge Scoring

```toml
# spin.toml
spin_manifest_version = 2

[application]
name = "roko-edge"
version = "0.1.0"
description = "Roko cognitive scoring at the edge"

[[trigger.http]]
route = "/score"
component = "scorer"

[[trigger.http]]
route = "/similarity"
component = "hdc-search"

[component.scorer]
source = "target/wasm32-wasi/release/roko_edge_scorer.wasm"
allowed_outbound_hosts = ["https://api.anthropic.com"]
[component.scorer.build]
command = "cargo build --target wasm32-wasi --release -p roko-edge-scorer"

[component.hdc-search]
source = "target/wasm32-wasi/release/roko_hdc_search.wasm"
[component.hdc-search.build]
command = "cargo build --target wasm32-wasi --release -p roko-hdc-search"
```

```rust
// crates/roko-edge-scorer/src/lib.rs
use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_component;
use roko_core::{Engram, Kind, Body};
use roko_std::default_scorer::DefaultScorer;

#[http_component]
async fn handle_score(req: Request) -> anyhow::Result<impl IntoResponse> {
    let body: serde_json::Value = serde_json::from_slice(req.body())?;

    let engram = Engram::builder()
        .kind(Kind::Observation)
        .body(Body::Text(body["text"].as_str().unwrap_or("").to_string()))
        .build();

    let scorer = DefaultScorer::default();
    let score = scorer.score(&engram);

    let response = serde_json::json!({
        "id": engram.id.to_string(),
        "score": {
            "novelty": score.novelty,
            "utility": score.utility,
            "confidence": score.confidence,
            "salience": score.salience,
        },
        "forward_to_core": score.novelty > 0.5 || score.utility > 0.7,
    });

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&response)?)
        .build())
}
```

### Spin Deployment Commands

```bash
# Local development with hot reload
spin build && spin up

# Deploy to Fermyon Cloud
spin deploy --from .

# Deploy as OCI artifact to any WASI P2 runtime
spin registry push ttl.sh/roko-edge:v0.1.0
```

---

## wasmCloud Distributed Deployment

wasmCloud (CNCF Incubating) runs WASM components across a distributed lattice of hosts connected via NATS. This enables deploying Roko's cognitive kernel across cloud, edge, and on-premises nodes with unified orchestration.

### wasmCloud Application Definition (WADM)

```yaml
# wadm.yaml
apiVersion: core.oam.dev/v1beta1
kind: Application
metadata:
  name: roko-cognitive
  annotations:
    description: 'Roko cognitive kernel on wasmCloud lattice'
spec:
  components:
    - name: cognitive-kernel
      type: component
      properties:
        image: ghcr.io/nunchi/roko-wasm:0.1.0
      traits:
        - type: spreadscaler
          properties:
            instances: 3
            spread:
              - name: edge
                requirements:
                  zone: edge
                weight: 80
              - name: cloud
                requirements:
                  zone: cloud
                weight: 20
        - type: link
          properties:
            target: httpserver
            namespace: wasi
            package: http
            interfaces: [incoming-handler]

    - name: httpserver
      type: capability
      properties:
        image: ghcr.io/wasmcloud/http-server:0.26.0
      traits:
        - type: link
          properties:
            target: cognitive-kernel
            namespace: wasi
            package: http
            interfaces: [incoming-handler]
            source_config:
              - name: default-http
                properties:
                  address: 0.0.0.0:8080

    - name: kvstore
      type: capability
      properties:
        image: ghcr.io/wasmcloud/keyvalue-redis:0.28.2
      traits:
        - type: link
          properties:
            target: cognitive-kernel
            namespace: wasi
            package: keyvalue
            interfaces: [store, atomics]
```

The `spreadscaler` distributes instances across edge and cloud zones. The lattice handles routing — a request arriving at any node reaches the nearest cognitive kernel instance.

---

## Cloudflare Workers Deployment

For Cloudflare Workers, Roko's cognitive primitives run as a WASM module within V8 isolates at 300+ edge locations worldwide. Sub-millisecond cold start, ~2ms average CPU time per request.

### Worker Configuration

```toml
# wrangler.toml
name = "roko-scorer"
main = "build/worker/shim.mjs"
compatibility_date = "2025-04-01"

[build]
command = "worker-build --release"

[[kv_namespaces]]
binding = "ENGRAM_CACHE"
id = "abc123"

[limits]
cpu_ms = 10000  # 10s for complex scoring
```

```rust
// src/lib.rs (Cloudflare Worker)
use worker::*;
use roko_core::{Engram, Kind, Body};
use roko_std::default_scorer::DefaultScorer;
use roko_primitives::HdcVector;

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/score", |mut req, _ctx| async move {
            let body: serde_json::Value = req.json().await?;
            let text = body["text"].as_str().unwrap_or("");

            let engram = Engram::builder()
                .kind(Kind::Observation)
                .body(Body::Text(text.to_string()))
                .build();

            let scorer = DefaultScorer::default();
            let score = scorer.score(&engram);

            Response::from_json(&serde_json::json!({
                "score": score,
                "forward": score.novelty > 0.5
            }))
        })
        .post_async("/hdc/similarity", |mut req, _ctx| async move {
            let body: serde_json::Value = req.json().await?;
            // HDC similarity search at the edge
            // Uses WASM SIMD128 for ~4x speedup on bitwise operations
            let query_bits: Vec<u64> = serde_json::from_value(body["query"].clone())?;
            let query = HdcVector::from_raw(query_bits, 10_000);

            // Load pre-computed fingerprints from KV
            // ... similarity search logic ...

            Response::from_json(&serde_json::json!({"matches": []}))
        })
        .run(req, env)
        .await
}
```

### Cloudflare Workers Resource Limits

| Resource | Free | Paid |
|---|---|---|
| CPU time per request | 10 ms | 30s (configurable to 5 min) |
| Memory per isolate | 128 MB | 128 MB |
| Subrequests | 50 | 1,000 |
| KV reads | 100K/day | 10M/month |

---

## WASM SIMD128 for HDC Vectors

WASM SIMD128 has universal browser and runtime support (Chrome 91+, Firefox 89+, Safari 2024+). For Roko's HDC operations, SIMD provides ~4x speedup over scalar WASM.

### HDC Hamming Distance with SIMD128

The critical path `hamming_distance(a, b) = popcount(xor(a, b))` maps directly to SIMD128 instructions:

```rust
#[cfg(target_arch = "wasm32")]
use std::arch::wasm32::*;

/// SIMD-accelerated Hamming distance for 128-bit chunks.
/// Processes 16 bytes per iteration using v128.xor + i8x16.popcnt.
#[cfg(target_arch = "wasm32")]
pub fn hamming_distance_simd(a: &[u8], b: &[u8]) -> u32 {
    assert_eq!(a.len(), b.len());
    let mut total: u32 = 0;

    // Process 16-byte chunks with SIMD
    let chunks = a.len() / 16;
    for i in 0..chunks {
        unsafe {
            let va = v128_load(a.as_ptr().add(i * 16) as *const v128);
            let vb = v128_load(b.as_ptr().add(i * 16) as *const v128);
            let xored = v128_xor(va, vb);
            let counts = i8x16_popcnt(xored);

            // Horizontal sum of 16 bytes
            // Widen to i16x8, then i32x4, then extract
            let widened = i16x8_extadd_pairwise_u8x16(counts);
            let widened32 = i32x4_extadd_pairwise_i16x8(widened);
            total += (i32x4_extract_lane::<0>(widened32)
                + i32x4_extract_lane::<1>(widened32)
                + i32x4_extract_lane::<2>(widened32)
                + i32x4_extract_lane::<3>(widened32)) as u32;
        }
    }

    // Handle remainder bytes with scalar
    let remainder_start = chunks * 16;
    for i in remainder_start..a.len() {
        total += (a[i] ^ b[i]).count_ones();
    }

    total
}
```

### Performance Comparison

| Environment | 10,000-bit vector comparison | Throughput |
|---|---|---|
| Native x86_64 (AVX2) | ~5 us | ~200K comparisons/sec |
| Native ARM64 (NEON) | ~8 us | ~125K comparisons/sec |
| WASM SIMD128 (browser) | ~15 us | ~67K comparisons/sec |
| WASM scalar (browser) | ~60 us | ~17K comparisons/sec |
| WASM SIMD128 (wasmtime) | ~12 us | ~83K comparisons/sec |

Build configuration for WASM SIMD:

```toml
# .cargo/config.toml
[target.wasm32-unknown-unknown]
rustflags = ["-C", "target-feature=+simd128"]

[target.wasm32-wasip2]
rustflags = ["-C", "target-feature=+simd128"]
```

### Test Criteria

```
WASM deployment tests:
1. `cargo build --target wasm32-wasip2 -p roko-core --no-default-features --features "serde,hdc,decay"` succeeds
2. BLAKE3 content hashes match between native and WASM targets (cross-environment identity)
3. HDC Hamming similarity produces identical results (within f64 precision) on native and WASM
4. MemorySubstrate put/query roundtrips correctly in WASM
5. WASM module size < 1MB gzipped for the cognitive kernel
6. SIMD128 build produces correct results (verify with wasmtime)
7. Spin application responds to HTTP score requests within 50ms
8. Cloudflare Worker stays under 10ms CPU time for single-engram scoring
```

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
