# Native Deployment (x86_64 and aarch64)

> Native deployment is the default and highest-performance target for Roko. The full 18-crate
> workspace compiles to a single binary per product, with all features enabled, optimized for
> the host architecture. This document covers build configuration, target triples, feature
> flags, performance characteristics, and cross-compilation for native platforms.


> **Implementation**: Specified

---

## Supported Architectures

Roko targets six native platform combinations:

| Target Triple | OS | Arch | Linking | Primary Use |
|---|---|---|---|---|
| `x86_64-apple-darwin` | macOS | Intel | dynamic (libc) | Developer laptops (older Macs) |
| `aarch64-apple-darwin` | macOS | Apple Silicon | dynamic (libc) | Developer laptops (M1/M2/M3/M4) |
| `x86_64-unknown-linux-gnu` | Linux | Intel | dynamic (glibc) | Servers, CI, Docker (glibc) |
| `aarch64-unknown-linux-gnu` | Linux | ARM64 | dynamic (glibc) | ARM servers (Graviton, Ampere) |
| `x86_64-unknown-linux-musl` | Linux | Intel | static (musl) | Docker slim images, scratch containers |
| `aarch64-unknown-linux-musl` | Linux | ARM64 | static (musl) | Docker slim images, ARM containers |

The musl targets produce fully static binaries — no runtime dependencies on the host system's
libc. These are used for Docker slim images (scratch or distroless base) and for the prebuilt
binaries distributed via GitHub Releases and cargo-dist.

The glibc targets are used for development builds and for Docker full images that need system
packages (tmux, ttyd, git).

---

## Build Configuration

### Workspace-Level Settings

The workspace `Cargo.toml` configures release profile optimizations:

```toml
[profile.release]
opt-level = 3
lto = "thin"        # Thin LTO: good balance of compile time and binary size
codegen-units = 1   # Single codegen unit for maximum optimization
strip = true        # Strip debug symbols from release binaries
panic = "abort"     # Smaller binaries, no unwinding overhead
```

For development builds, the default profile applies — no LTO, multiple codegen units, debug
symbols retained. This keeps iterative compile times fast.

### Target-Specific Configuration

`.cargo/config.toml` configures per-target settings:

```toml
[target.x86_64-unknown-linux-musl]
# Use musl-gcc for static linking
linker = "x86_64-linux-musl-gcc"

[target.aarch64-unknown-linux-musl]
linker = "aarch64-linux-musl-gcc"

[target.x86_64-unknown-linux-gnu]
# Default system linker

[target.aarch64-unknown-linux-gnu]
# Use cross-compilation linker when building from x86
linker = "aarch64-linux-gnu-gcc"
```

### Rust Toolchain Requirements

Roko requires Rust 1.91+ due to alloy dependencies (Ethereum primitives). The `rust-toolchain.toml`
at the workspace root pins the minimum:

```toml
[toolchain]
channel = "stable"
# Minimum: 1.91 for alloy deps
```

Running `rustup update stable` before building ensures the toolchain is current.

---

## Feature Flags

Native builds enable all features by default. The feature flag system controls which optional
subsystems are compiled:

### roko-core Features

```toml
[features]
default = ["full"]
full = ["serde", "hdc", "decay"]
serde = ["dep:serde", "dep:serde_json"]
hdc = ["dep:roko-primitives"]           # HDC vector encoding for Engrams
decay = []                               # Time-based Engram decay
```

### roko-agent Features

```toml
[features]
default = ["full"]
full = ["anthropic", "openai", "openrouter", "mcp"]
anthropic = []          # Anthropic Claude backends
openai = []             # OpenAI GPT backends
openrouter = []         # OpenRouter multi-model routing
mcp = ["dep:rmcp"]      # MCP client for tool discovery
```

### roko-cli Features

```toml
[features]
default = ["full"]
full = [
    "roko-agent/full",
    "roko-gate/full",
    "roko-compose/full",
    "roko-orchestrator/full",
    "tui",
]
tui = ["dep:ratatui", "dep:crossterm"]   # Terminal UI (interactive dashboard)
headless = []                             # No TUI, CLI-only (for CI/server)
```

### mirage-rs Features

```toml
[features]
default = ["binary"]
binary = ["dep:clap", "dep:tokio"]
# The "chain" feature adds chain witness integration for Roko agents.
# Not included by default — only needed when mirage-rs is spawned by an agent.
chain = ["dep:roko-chain"]
```

When building for native deployment, use the defaults (all features):

```bash
cargo build --release -p roko-cli
cargo build --release -p mirage-rs
```

For minimal builds (CI, headless server), disable optional features:

```bash
cargo build --release -p roko-cli --no-default-features --features headless
```

---

## Cross-Compilation

### macOS to Linux (for Docker images)

Building Linux binaries from macOS requires a cross-compilation toolchain. Two approaches:

**Using `cross` (recommended for CI):**

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-musl -p roko-cli
cross build --release --target aarch64-unknown-linux-musl -p roko-cli
```

`cross` uses Docker containers with pre-configured toolchains for each target. It handles musl
libc, system libraries, and linker configuration automatically. The overhead is a Docker pull on
first run (~500MB for the musl toolchain image).

**Using `cargo-zigbuild` (faster, lighter):**

```bash
cargo install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-musl -p roko-cli
cargo zigbuild --release --target aarch64-unknown-linux-musl -p roko-cli
```

`cargo-zigbuild` uses the Zig compiler's cross-compilation capabilities as a drop-in C/C++
cross-compiler. It is faster than `cross` because it does not require Docker and has a smaller
toolchain footprint. cargo-dist uses this approach for its CI builds.

### The justfile Targets

The workspace justfile provides convenience targets for cross-compilation:

```bash
# Build all binaries for Linux amd64 (musl, static)
just release-linux-amd64

# Build all binaries for Linux arm64 (musl, static)
just release-linux-arm64

# Build for the current host platform
just release
```

These targets compile all publishable binaries (`roko-cli`, `roko-serve`, `mirage-rs`) in a
single invocation.

---

## Binary Size

Native release binaries with all features enabled and strip + LTO:

| Binary | Approximate Size | Notes |
|---|---|---|
| `roko-cli` | ~25-35 MB | Full CLI with TUI, all gates, all backends |
| `roko-serve` | ~20-30 MB | HTTP API server, no TUI |
| `mirage-rs` | ~15-20 MB | EVM simulator, alloy primitives |

These sizes are typical for Rust binaries that include Tokio, Axum, ratatui, and the alloy
EVM stack. The musl-linked static binaries are slightly larger than glibc-linked ones due to
static inclusion of libc.

Strategies for reducing binary size (if needed):

- `opt-level = "z"` instead of `3` — optimize for size over speed (~20% smaller, ~5% slower)
- `panic = "abort"` (already configured) — removes unwinding tables
- `strip = true` (already configured) — removes debug symbols
- Feature-gating: disable unused LLM backends or gate types for specific deployments

---

## Performance Characteristics

Native deployment provides optimal performance because:

1. **No abstraction overhead**: The binary runs directly on the host OS with no VM, container,
   or WASM interpreter overhead.
2. **Full SIMD**: HDC vector operations (Hamming distance, XOR bundling) use native SIMD
   instructions (AVX2 on x86_64, NEON on aarch64) via auto-vectorization.
3. **Full async runtime**: Tokio's multi-threaded runtime uses all available cores. The adaptive
   clock in `roko-runtime` calibrates tick intervals based on system load.
4. **Direct filesystem access**: The `FileSubstrate` in `roko-fs` uses direct filesystem calls
   for JSONL persistence, memory-mapped files for the search index, and atomic file operations
   for crash-safe state.
5. **Full networking**: All LLM provider backends (Anthropic, OpenAI, OpenRouter), MCP client
   connections, WebSocket relay, and HTTP serve endpoints are available.

### Memory Usage

Typical memory footprint for native deployments:

| Scenario | RSS (approx.) |
|---|---|
| roko-cli idle (after init) | ~50 MB |
| roko-cli running 4 agents | ~200-400 MB (depends on context sizes) |
| roko-cli running 8 agents | ~400-800 MB |
| mirage-rs with 50K dirty slots | ~150-300 MB |
| roko-serve idle | ~40 MB |

Memory scales primarily with the number of concurrent agents (each holds context in memory) and
the size of the code index (tree-sitter ASTs + HDC fingerprints + symbol graph).

---

## Installation from Source

### Prerequisites

- Rust 1.91+ (install via `rustup`: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Git (for cloning and for `roko-orchestrator` worktree operations)
- A C compiler (for tree-sitter and SQLite FFI bindings — `cc` crate auto-detects)

### Build and Install

```bash
# Clone the workspace
git clone https://github.com/nunchi/roko.git
cd roko

# Build all workspace crates
cargo build --workspace --release

# Install specific binaries to ~/.cargo/bin
cargo install --path crates/roko-cli
cargo install --path apps/mirage-rs

# Or build and run without installing
cargo run -p roko-cli -- status
cargo run -p roko-cli -- plan list
```

### Verify the Build

```bash
# Run the full test suite
cargo test --workspace

# Run clippy for lint checks
cargo clippy --workspace --no-deps -- -D warnings

# Check that the CLI works
cargo run -p roko-cli -- --version
cargo run -p roko-cli -- doctor
```

The `roko doctor` subcommand verifies the installation environment: config files, API keys,
gateway connectivity, index health, and git availability. See `13-current-status-and-port-allocation.md`
for the current implementation status of the doctor subcommand.

---

## Development Builds

For iterative development, use debug builds (faster compilation, slower runtime):

```bash
# Fast compile, debug symbols, no optimization
cargo build -p roko-cli

# Run with debug logging
RUST_LOG=debug cargo run -p roko-cli -- status

# Run specific tests
cargo test -p roko-core -- engram
cargo test -p roko-gate -- compile_gate
```

Development builds skip LTO and use multiple codegen units, reducing compile time from ~3-5
minutes (release) to ~30-60 seconds (debug) for incremental rebuilds.

### Incremental Compilation

Rust's incremental compilation is enabled by default for debug builds. After an initial full
build (~2-5 minutes for the workspace), subsequent builds that change a single crate typically
complete in 10-30 seconds. The `sccache` or `mold` linker can further reduce link times:

```bash
# Using mold linker (Linux) for faster linking
RUSTFLAGS="-C link-arg=-fuse-ld=mold" cargo build -p roko-cli

# Using sccache for shared compilation cache
RUSTC_WRAPPER=sccache cargo build -p roko-cli
```
