# Developer Experience and Onboarding

> Depth for [20-SURFACES.md](../../unified/20-SURFACES.md). Covers the SDK and onboarding as Graph templates and Trigger Cells, Rust SDK entry points at four abstraction levels, `roko init` as a Trigger Graph, IDE integration via ACP-first strategy, domain profiles as config Signals, and the three onboarding paths.

---

## 1. SDK Entry Points as Graph Templates

The Roko SDK offers four distinct surfaces, each corresponding to a different level of abstraction. In unified vocabulary, each entry point is a pre-built **Graph template** (see [03-GRAPH.md](../../unified/03-GRAPH.md)) at a different depth in the Cell hierarchy:

| Entry Point | Audience | Graph Depth | Abstraction |
|---|---|---|---|
| **One-liner** | Demos, scripts | Full cognitive loop Graph with all defaults | Maximum |
| **Builder** | Application authors | Cognitive loop with typed configuration | High |
| **Trait impl** | Component authors | Single Cell implementation | Medium |
| **Runtime impl** | Platform authors | Custom execution environment | Minimum |

### One-Liner

```rust
use roko::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let out = roko::run("Summarize README.md").await?;
    println!("{out}");
    Ok(())
}
```

This fires the full cognitive loop Graph with sensible defaults: local Store, in-memory Bus, auto-detected provider, `.roko/` for state. The one-liner is a Rack (see [03-GRAPH.md](../../unified/03-GRAPH.md)) with all Macros set to defaults.

### Builder

```rust
let agent = Agent::builder()
    .role(Role::Researcher)
    .model(Model::claude_opus())
    .tool(tools::web_search())
    .tool(tools::fs_read("."))
    .memory_dir("./.roko")
    .build()
    .await?;

let response = agent.send("What's new in Rust 1.91?").await?;
```

The builder is a Rack where each method call sets a Macro (knob). `.build()` validates the configuration and compiles the Rack into a concrete Graph. `.send()` fires the Graph once.

### Trait Impl

```rust
#[async_trait]
impl Store for MyStore {
    async fn put(&self, signal: Signal) -> Result<SignalHash> { ... }
    async fn get(&self, hash: &SignalHash) -> Result<Option<Signal>> { ... }
    async fn query(&self, predicate: Predicate) -> Result<Vec<Signal>> { ... }
}
```

Trait implementors provide a single Cell that plugs into the standard Graph. The Cell implements one of the 9 protocols (Store, Score, Verify, Route, Compose, React, Observe, Connect, Trigger) and is swappable at the Slot level.

### Runtime Impl

```rust
impl Runtime for BrowserRuntime {
    type Bus = InMemoryBus;
    type Store = IndexedDbStore;

    fn bus(&self) -> &InMemoryBus { &self.bus }
    fn store(&self) -> &IndexedDbStore { &self.store }
}
```

Runtime implementors own the execution boundary. This is where alternate platforms live: browser (WASM), edge, embedded, or distributed hosts that need different Bus/Store implementations while preserving the same agent semantics.

---

## 2. `roko init` as a Trigger Graph

The `roko init` command is a **Trigger Graph** -- a Graph fired by workspace detection that produces a configured `.roko/` directory. The Graph has six stages, each a Cell:

```
WorkspaceDetect → DomainAutoDetect → ProviderProbe → ProfileSelect → ConfigGenerate → Validate
```

### Stage 1: Workspace Detection

Scan the current directory for project markers:

| Detection Signal | What It Finds | Confidence |
|---|---|---|
| `Cargo.toml` | Rust project | 0.95 |
| `package.json` + TypeScript | Node/TS project | 0.90 |
| `go.mod` | Go project | 0.90 |
| `pyproject.toml` | Python project | 0.85 |
| Empty directory | No project context | 0.10 |

### Stage 2: Domain Auto-Detection

The detected workspace informs domain profile selection:

| Domain | Default Gates | Default Tools | System Prompt Context |
|---|---|---|---|
| Coding (Rust) | fmt, compile, clippy, test | read, write, edit, bash, cargo | Rust ecosystem knowledge |
| Coding (TS) | lint, typecheck, test | read, write, edit, bash, npm | TypeScript ecosystem |
| Research | citation, factuality | read, search, web, pdf | Academic/research context |
| Blockchain | compile, simulate, risk | chain RPC, exchange API | DeFi/contract knowledge |

### Stage 3: Provider Probe

Check available LLM providers:

```
[1/3] Checking Anthropic... ANTHROPIC_API_KEY found, testing... OK
[2/3] Checking OpenAI... OPENAI_API_KEY not found. Skip? [Y/n]
[3/3] Checking Ollama... localhost:11434 not responding. Skip? [Y/n]

Using: Anthropic (claude-sonnet-4-6)
```

Every failure is recoverable in place: paste key, open docs, skip, or configure later. No single check blocks first success.

### Stage 4: Profile Selection

Domain profiles are installable, composable bundles. The Rack pattern applies: each profile is a Rack where Macros are domain-specific knobs (tools, gates, heuristics) and Slots are extension points (custom gates, custom tools).

```bash
roko plugin install @roko/coding-profile
roko init --profile coding
```

Multiple profiles can be composed for mixed-domain projects. Collisions (two profiles define the same gate) are surfaced with resolution options.

### Stage 5: Config Generation

Produce `roko.toml` with resolved values. The config is a Signal merge pipeline (see [02-cli-and-command-graph.md](./02-cli-and-command-graph.md)):

```toml
[agent]
model = "claude-sonnet-4-6"

[gates]
pipeline = ["fmt", "compile", "clippy", "test"]

[routing]
type = "cascade"
```

### Stage 6: Validate

Run the first-task validation checklist:

| Step | What It Proves |
|---|---|
| Store put/get | Persistence is connected |
| Bus publish/subscribe | Transport is connected |
| Compose prompt | Context engineering works |
| LLM dispatch | Model routing is connected |
| Gate pipeline | Verification works |

**Critical rule**: Partial success is valid. Setup state persists incrementally. A cancelled `roko init` can resume from the last completed stage.

---

## 3. Three Onboarding Paths

| Path | Time | Steps | For Whom |
|---|---|---|---|
| **Minimal** | 30s | `roko init` + `roko ask "prompt"` | Developer who wants to try immediately |
| **Standard** | 2m | `roko init --profile coding` + inspect + plan + do | Developer configuring for a project |
| **Full** | 5m | Explicit config for every option | DevOps setting up team deployment |

The minimal path works because `roko init` auto-detects everything and writes defaults. The standard path adds profile selection and review. The full path adds explicit model routing, gate configuration, budget limits, and parallelism settings.

---

## 4. IDE Integration: ACP-First, MCP as Universal Adapter

Roko's IDE strategy uses two protocols that compose:

- **MCP (Model Context Protocol)**: Roko appears as a tool provider. Stateless request-response. Works in any MCP-capable editor today.
- **ACP (Agent Communication Protocol)**: Roko appears as a full agent. Stateful sessions, bidirectional streaming, permission requests. Works in Zed, JetBrains, Neovim, Emacs, and (via extension) VS Code.

```
IDE
 |
 +-- ACP --> roko (agent)      # Full cognitive loop, streaming, gates
 |             |
 |             +-- MCP --> code search server
 |             +-- MCP --> documentation server
 |
 +-- MCP --> roko (tool provider)   # Simple tool calls for Copilot
```

### ACP Session Lifecycle

```
IDE                                   roko acp
  |--- session/new ----------------->|  cwd, MCP configs
  |<-- session ID -------------------|
  |
  |--- session/prompt -------------->|  user text + context
  |<-- session/update (notification) |  AgentThoughtChunk (reasoning)
  |<-- session/update (notification) |  ToolCallResult (edit applied)
  |<-- session/update (notification) |  _roko.dev/gate/result
  |<-- session/update (notification) |  _roko.dev/plan/status
  |<-- PromptResponse ---------------|  stopReason: done
```

ACP sessions map to plan runs. ACP tool calls map to file edits. ACP permission requests map to gate approvals. ACP `session/load` maps to `--resume` from executor snapshots.

### Why Not Fork VS Code

The maintenance cost of a VS Code fork is unsustainable at any scale below Cursor's (~300 engineers, $2B ARR). Roko's differentiators (plan execution, gates, learning, self-improvement) live in the Rust backend. The IDE is a display layer. ACP + a thin extension costs 1-2 orders of magnitude less than maintaining a fork.

**Source**: `crates/roko-agent/src/provider/cursor_acp.rs` (existing ACP adapter).

---

## 5. Macros and Testing Ergonomics

### Attribute Macros

```rust
#[tool(
    description = "Read a file from disk",
    role_allow = ["researcher", "implementer"],
)]
async fn read_file(path: String) -> Result<String> {
    tokio::fs::read_to_string(&path).await.map_err(Into::into)
}
```

### Testing Helpers

```rust
use roko::testing::{AssertingBus, MockAgent};

#[tokio::test]
async fn my_flow_hits_gate() {
    let agent = MockAgent::builder()
        .expect_tool("read_file", "fake contents")
        .build();
    let bus = AssertingBus::expect("gate.passed.unit");

    agent.run(bus, "read README.md").await.unwrap();
    bus.assert_expectations();
}
```

The testing surface includes `MockAgent`, `AssertingBus`, `RecordingStore`, and clock helpers for advancing demurrage-sensitive flows.

---

## 6. Error Vocabulary

Public errors are typed, actionable, and `#[non_exhaustive]`:

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RokoError {
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    #[error("bus error: {0}")]
    Bus(#[from] BusError),

    #[error("tool {tool} failed: {reason}")]
    Tool { tool: String, reason: String },

    #[error("gate {gate} rejected: {reason}")]
    Gate { gate: String, reason: String },

    #[error("agent timed out after {0:?}")]
    Timeout(Duration),

    #[error("configuration invalid: {0}")]
    Config(String),
}
```

Rules: no `anyhow::Error` at the public boundary. Every variant tells the user what failed and what to change. `#[non_exhaustive]` enables safe API evolution.

---

## What This Enables

- **60-second first agent**: `roko init && roko ask "prompt"` works with zero manual configuration.
- **Four abstraction levels**: From one-liner to runtime implementation, each level adds control without rewriting previous code.
- **Cross-IDE support**: One ACP implementation works in Zed, JetBrains, Neovim, and Emacs. MCP provides fallback for any MCP-capable editor.
- **Resumable onboarding**: Every setup stage persists progress. Cancelled init resumes from the last completed step.
- **Profile composition**: Mixed-domain projects compose multiple profiles with collision detection and resolution.

---

## Feedback Loops

- **Onboarding -> first episode -> learning**: The first task execution produces an episode, which seeds the cascade router's model selection, which improves subsequent tasks.
- **IDE gate results -> agent retry**: Gate failures streamed via ACP `_roko.dev/gate/result` are visible in the IDE. The agent retries with error context. The operator can watch the retry cycle in real time.
- **Profile usage -> profile improvement**: Which domain profiles are most installed? Which tools from a profile are most used? Aggregate usage data (with consent) can improve default profiles.

---

## Open Questions

1. **SDK release cadence**: Should the SDK follow a fixed release train (six weeks), or release on demand? Fixed trains provide predictability; on-demand is faster.
2. **One-liner scope**: How much should `roko::run()` do by default? Auto-detect provider, auto-create `.roko/`, auto-enable gates? Or should it be minimal with explicit opt-in?
3. **ACP extension evolution**: Roko's `_roko.dev/` extensions are vendor-specific. Should they be proposed for ACP standardization once proven, or remain vendor-specific permanently?
4. **Plugin SPI timeline**: The five-tier plugin system (prompt bundles, config profiles, declarative tools, native Rust, WASM) is specified but not implemented. What is the right order of implementation?

---

## Implementation Tasks

| Task | Where | What |
|---|---|---|
| Build interactive `roko init` with resume | `crates/roko-cli/src/config.rs` | Stage-by-stage init with incremental state persistence |
| Implement provider probing with graceful degradation | `crates/roko-cli/src/config.rs` | Check Anthropic, OpenAI, Ollama with skip/retry/defer |
| Build `roko acp` subcommand | `crates/roko-cli/src/acp.rs` | ACP server over stdio, session lifecycle, streaming |
| Build MCP server mode | `crates/roko-serve/src/mcp_server.rs` | `roko serve --mcp` exposing CLI commands as MCP tools |
| Define `_roko.dev/` ACP extensions | `crates/roko-cli/src/acp.rs` | Plan progress, gate results, learning feedback, episodes |
| Add `roko::run()` one-liner API | `crates/roko-core/src/lib.rs` or facade crate | Single function with sensible defaults |
| Add `Agent::builder()` API | Same | Fluent builder with typed configuration and validation |
| Implement testing helpers | `crates/roko-core/src/testing/` | MockAgent, AssertingBus, RecordingStore |
| Implement domain profile install and composition | `crates/roko-cli/src/` | `roko plugin install @roko/coding-profile` with collision detection |
