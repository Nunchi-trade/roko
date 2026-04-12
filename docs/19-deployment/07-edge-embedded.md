# Edge and Embedded Deployment

> Roko's core cognitive primitives can be compiled to a minimal ~500KB binary for deployment on
> resource-constrained devices, edge nodes, IoT gateways, and embedded systems. This document
> covers the minimal feature set, the binary size budget, the no_std considerations, use cases
> for edge deployment, and the relationship to the WASM deployment target.


> **Implementation**: Specified

---

## Edge Deployment Philosophy

Edge deployment targets environments where resources are severely constrained: limited memory
(< 64MB), limited storage (< 10MB), limited or intermittent network connectivity, and limited
CPU. Examples include IoT gateways, network edge nodes, embedded Linux devices, and Cloudflare
Workers.

The design principle: **the core cognitive primitives (Engram, Score, Scorer, Router, Composer)
are pure computation with no I/O dependencies**. They can run anywhere that has a Rust allocator.
The I/O-dependent components (LLM backends, filesystem persistence, process supervision) are
optional and excluded from edge builds.

This maps directly to the Synapse Architecture's trait system — each trait is independently
implementable. An edge deployment implements `Scorer` and `Router` locally while delegating
`Substrate` (persistence) and `Gate` (verification) to a remote core node.

---

## Minimal Feature Set

Edge builds disable all optional features and compile only the core cognitive kernel:

```bash
cargo build --release \
    -p roko-core \
    --no-default-features \
    --features "serde,hdc,decay" \
    --target aarch64-unknown-linux-musl
```

### What Is Included

| Component | Size Contribution | Purpose |
|---|---|---|
| `Engram` struct + `Score` | ~15KB | Core data type, 7-axis appraisal |
| `ContentHash` (BLAKE3) | ~30KB | Content addressing |
| Synapse traits (6 trait definitions) | ~5KB | Trait interfaces only |
| `DefaultScorer` | ~10KB | Score computation |
| `DefaultRouter` | ~10KB | Selection logic |
| `DefaultComposer` | ~15KB | Context assembly under budget |
| `HdcVector` (HDC primitives) | ~20KB | Hamming similarity, XOR bundling |
| `Decay` (time-based decay) | ~5KB | HalfLife, TTL, Ebbinghaus |
| serde + serde_json | ~200KB | Serialization for IPC |
| Allocator + runtime | ~50KB | Minimal Rust runtime |
| **Total** | **~360KB** | Core cognitive kernel |

### What Is Excluded

| Component | Why Excluded | Alternative for Edge |
|---|---|---|
| Tokio async runtime | ~2MB, unnecessary for synchronous edge processing | Use `smol` or bare `poll` |
| LLM provider backends (reqwest + TLS) | ~5MB, requires network | Proxy through core node |
| ratatui TUI | ~500KB, no display on edge | Not applicable |
| Tree-sitter (code parsing) | ~3MB, C FFI dependency | Use pre-computed indexes |
| alloy (Ethereum primitives) | ~10MB, chain-specific | Not needed on edge |
| `FileSubstrate` (JSONL persistence) | Requires writable filesystem | Use `MemorySubstrate` |
| `ProcessSupervisor` | Requires process spawning | Not applicable |

---

## Binary Size Budget

The target for edge deployment is a ~500KB stripped binary (or WASM module). This budget
constrains which components can be included:

```
Budget: 500KB

Core cognitive kernel:     ~360KB (72% of budget)
IPC / networking stub:      ~50KB (10% of budget)
Application logic:          ~90KB (18% of budget — user's edge agent)
───────────────────────────────────
Total:                     ~500KB
```

### Size Optimization

To achieve the 500KB target, apply aggressive size optimization:

```toml
# Cargo.toml [profile.release] overrides for edge target
[profile.edge]
inherits = "release"
opt-level = "z"        # Optimize for size (not speed)
lto = "fat"            # Full LTO for maximum dead code elimination
codegen-units = 1      # Single codegen unit
strip = true           # Strip all symbols
panic = "abort"        # No unwinding tables
```

Build with the edge profile:

```bash
cargo build --profile edge -p roko-core --no-default-features --features "serde,hdc,decay"
```

Additional size reduction strategies:

- Replace `serde_json` with `miniserde` or `nanoserde` (~50KB → ~10KB)
- Use `blake3` with the `no_std` feature (removes threading support, saves ~15KB)
- Replace `BTreeMap` in Engram tags with a fixed-size array for known tag keys
- Use `#[cfg(feature = "edge")]` to gate out any convenience methods that inflate binary size

---

## Use Cases

### 1. Edge Scoring and Pre-Filtering

An edge node receives a stream of events (sensor data, log entries, API responses) and uses
Roko's `Scorer` to determine which events are novel enough to forward to the core agent:

```rust
use roko_core::{Engram, Score, Kind, Body};
use roko_std::default_scorer::DefaultScorer;

fn should_forward(event: &str) -> bool {
    let engram = Engram::builder()
        .kind(Kind::Observation)
        .body(Body::Text(event.to_string()))
        .build();

    let scorer = DefaultScorer::default();
    let score = scorer.score(&engram);

    // Forward only high-novelty or high-utility events
    score.novelty > 0.5 || score.utility > 0.7
}
```

This reduces bandwidth between edge and core by ~80%, matching the 16 T0 Probes pattern from
the dual-process cognition model — most events can be classified without invoking an LLM.

### 2. Local Knowledge Cache

An edge node maintains a `MemorySubstrate` with recently relevant Engrams, serving as a local
cache that avoids round-trips to the core agent for repeated queries:

```rust
use roko_std::memory_substrate::MemorySubstrate;

let mut cache = MemorySubstrate::new();

// Store high-utility Engrams locally
if score.utility > 0.8 {
    cache.put(engram).await?;
}

// Check local cache before forwarding to core
if let Some(cached) = cache.query(Query::similar(&query_engram)).await? {
    return Ok(cached);
}
// Cache miss — forward to core agent
```

### 3. HDC-Based Similarity Search at the Edge

Edge nodes can perform HDC vector similarity search locally, enabling fast approximate matching
without network access:

```rust
use roko_primitives::HdcVector;

// Pre-computed HDC fingerprints for known patterns
let known_patterns: Vec<(String, HdcVector)> = load_from_flash();

// Encode incoming data as HDC vector
let query = HdcVector::encode(incoming_data);

// Find closest match
let best = known_patterns.iter()
    .map(|(label, vec)| (label, query.hamming_similarity(vec)))
    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
```

HDC vector comparison is ~50ns per 10,000-bit vector on ARM Cortex-A72 — fast enough for
real-time classification at the edge.

### 4. Offline Agent with Periodic Sync

An edge agent operates autonomously when disconnected, accumulating Engrams in a
`MemorySubstrate`. When connectivity is restored, it syncs with the core agent:

```rust
// Offline: accumulate observations
loop {
    let observation = read_sensor();
    let engram = process(observation);
    local_substrate.put(engram).await?;

    if is_connected() {
        // Online: sync accumulated Engrams to core
        let pending = local_substrate.export().await?;
        send_to_core(pending).await?;

        // Receive updated knowledge from core
        let updates = receive_from_core().await?;
        for engram in updates {
            local_substrate.put(engram).await?;
        }
    }

    sleep(tick_interval);
}
```

---

## Edge-Core Communication Protocol

Edge nodes communicate with core agents using a lightweight JSON-RPC protocol over any
available transport (HTTP, MQTT, serial, BLE):

```json
// Edge → Core: forward high-novelty Engram
{
    "jsonrpc": "2.0",
    "method": "substrate.put",
    "params": {
        "engram": {
            "id": "blake3:a1b2c3...",
            "kind": "Observation",
            "body": {"text": "sensor reading: 42.5"},
            "score": {"novelty": 0.8, "utility": 0.6, "confidence": 0.9},
            "tags": {"source": "edge-node-7", "domain": "sensor"}
        }
    },
    "id": 1
}

// Core → Edge: knowledge update
{
    "jsonrpc": "2.0",
    "method": "substrate.sync",
    "params": {
        "engrams": [
            {"id": "blake3:d4e5f6...", "kind": "Heuristic", "body": {"text": "..."}}
        ],
        "since_ms": 1712345678000
    },
    "id": 2
}
```

---

## Relationship to WASM Deployment

Edge deployment and WASM deployment share the same core: the cognitive kernel compiled without
I/O dependencies. The difference is the compilation target:

| Aspect | Edge (native) | WASM |
|---|---|---|
| Target triple | `aarch64-unknown-linux-musl` | `wasm32-wasi` |
| Binary format | ELF static binary | `.wasm` module |
| Runtime | Linux kernel | WASM runtime (wasmtime, browser) |
| Performance | Full native speed, SIMD | ~2-5× slower, limited SIMD |
| Binary size | ~500KB | ~500KB (gzipped) |
| Filesystem | Possible (musl libc) | Not available |
| Networking | Possible (TCP/UDP) | Via host imports only |

For ARM edge devices running Linux, native edge deployment is preferred (better performance,
full system access). For browser-based or sandboxed environments, WASM deployment is necessary.
The shared feature flag system (`--no-default-features --features "serde,hdc,decay"`) ensures
both targets compile the same cognitive kernel.

---

## Current Status

Edge deployment is at **Tier 3H** priority (P3 — future). The core cognitive primitives already
compile with `--no-default-features` (this is tested in CI), and the `MemorySubstrate` is used
extensively in unit tests. The ~500KB binary size target has not been validated with a dedicated
edge build profile.

What needs to be done for edge readiness:

1. Create a `[profile.edge]` in workspace `Cargo.toml` with size-optimized settings
2. Validate that `roko-core` compiles standalone without pulling in Tokio or other heavy deps
3. Measure actual binary size with the edge profile
4. Create an example edge agent (`examples/edge-agent/`) demonstrating the scoring + forwarding
   pattern
5. Define the edge-core sync protocol and implement it in a lightweight transport crate
