# Rust SDK Developer UX

> Four ergonomic entry points for Rust developers building on Roko: one-liner, builder, trait impl, and runtime impl. The goal is not just API shape, but time to first working agent in under 60 seconds for anyone with `cargo` installed.

> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Engram, Pulse, Substrate, and Bus; [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) for shared vocabulary
**Key source**: [tmp/refinements/22-developer-ux-rust.md](../../tmp/refinements/22-developer-ux-rust.md)

---

## Abstract

This chapter defines the Rust-facing developer experience for Roko. The SDK should feel familiar to developers who already know `tokio`, `reqwest`, and `sqlx`: a narrow one-liner for demos, a fluent builder for most applications, stable trait contracts for custom components, and a runtime boundary for alternate execution environments. The chapter also sets expectations for typed errors, runnable examples, docs discipline, testing helpers, tracing, and release compatibility.

The guiding metric is simple: a Rust developer should be able to say "hello, agent" in under 60 seconds, then grow from there without rewriting their first successful code path.

---

## 1. The four entry points

Roko should offer four distinct surfaces, each with different ergonomics and different levels of control.

| Surface | Audience | Promise |
|---|---|---|
| One-liner | Demos, scripts, first look | A sensible default agent with minimal ceremony |
| Builder | Application authors, agent authors | Typed configuration and early validation |
| Trait impl | Component authors | Stable contracts for custom `Substrate`, `Bus`, `Scorer`, and `Router` implementations |
| Runtime impl | Runtime authors | Direct access to the kernel for browser, edge, or distributed execution |

### 1.1 One-liner

```rust
use roko::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let out = roko::run("Summarize README.md").await?;
    println!("{out}");
    Ok(())
}
```

The one-liner should work out of the box with sensible defaults: local first when available, cloud fallback when needed, `.roko/` for local state, and no plugins unless explicitly enabled.

### 1.2 Builder

```rust
use roko::{Agent, Model, Role};

#[tokio::main]
async fn main() -> Result<()> {
    let agent = Agent::builder()
        .role(Role::Researcher)
        .model(Model::claude_opus())
        .tool(tools::web_search())
        .tool(tools::fs_read("."))
        .memory_dir("./.roko")
        .build()
        .await?;

    let response = agent.send("What's new in Rust 1.91?").await?;
    println!("{response}");
    Ok(())
}
```

The builder is the default daily-driver API. It should validate configuration at build time, return typed errors, support `dry_run()` for prompt inspection, and allow cloning templates for worker agents.

### 1.3 Trait impl

```rust
use async_trait::async_trait;
use roko_core::{Bus, Engram, EngramHash, Result, Substrate};

pub struct MySubstrate;

#[async_trait]
impl Substrate for MySubstrate {
    async fn put(&self, e: Engram) -> Result<EngramHash> {
        todo!()
    }

    async fn get(&self, h: &EngramHash) -> Result<Option<Engram>> {
        todo!()
    }

    async fn query(&self, predicate: Predicate) -> Result<Vec<Engram>> {
        todo!()
    }
}
```

Trait implementors should be able to stay inside `roko-core` and implement the contract without runtime-tier leakage. The public trait surface should be narrow, stable, and self-contained enough that downstream crates can swap in their own storage, transport, routing, or scoring logic.

### 1.4 Runtime impl

```rust
use roko_runtime::{Runtime, Supervisor};

pub struct BrowserRuntime {
    supervisor: BrowserSupervisor,
    bus: InMemoryBus,
}

impl Runtime for BrowserRuntime {
    type Supervisor = BrowserSupervisor;
    type Bus = InMemoryBus;

    fn supervisor(&self) -> &BrowserSupervisor {
        &self.supervisor
    }

    fn bus(&self) -> &InMemoryBus {
        &self.bus
    }
}
```

Runtime implementors own the execution boundary. This is where alternate platforms live: browser, edge, embedded, or distributed hosts that need a different supervisor, bus, or I/O model while preserving the same agent semantics.

---

## 2. Error vocabulary

Public errors should be typed, actionable, and consistent.

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RokoError {
    #[error("substrate error: {0}")]
    Substrate(#[from] SubstrateError),

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

Rules:

- Do not expose `anyhow::Error` at the public API boundary.
- Every variant should tell the user what failed and what to change.
- `#[non_exhaustive]` belongs on public enums so the API can grow safely.
- Debug builds may carry backtraces or extra context; release builds should stay lean.

---

## 3. Docs and examples discipline

The SDK needs a documentation stack that matches the API stack.

- The top-level README should answer "what is this?" in under a minute.
- Tutorial-style docs should guide first-time users through a single working path.
- Cookbook-style docs should be searchable by task, not by crate boundary.
- Reference docs should explain each crate's role in the larger architecture, not just mirror rustdoc.
- Every public item should have rustdoc, including examples and error notes where relevant.

The examples directory should be treated as a first-class product surface. A canonical `examples/` tree should include runnable cargo projects such as:

```text
examples/
├── 01-hello-agent/
├── 02-builder-basics/
├── 03-custom-tool/
├── 04-custom-gate/
├── 05-swap-substrate/
├── 06-multi-agent/
├── 07-streaming/
├── 08-plugin-manifest/
├── 09-research-workflow/
└── 10-web-ui/
```

Each example should have a README, compile independently, and link back to the relevant docs. CI should run `cargo test --examples` so the examples remain executable.

---

## 4. `cargo roko`

Rust developers should get a cargo-native workflow, not a separate mental model.

```bash
cargo roko new my-agent
cargo roko play
cargo roko replay <episode>
cargo roko bench
cargo roko explain <hash>
cargo roko heuristics
```

The goal is parity with familiar cargo tooling: scaffold, iterate, replay, benchmark, inspect, and browse beliefs without leaving the crate context.

---

## 5. Macros

Macros are allowed when they reduce ceremony without hiding semantics.

### `#[tool]`

```rust
#[tool(
    description = "Read a file from disk",
    role_allow = ["researcher", "implementer"],
    files = "{path}",
)]
async fn read_file(path: String) -> Result<String> {
    tokio::fs::read_to_string(&path).await.map_err(Into::into)
}
```

### `#[gate]`

```rust
#[gate(rung = Rung::Unit)]
async fn my_gate(ctx: &GateCtx) -> GateResult {
    todo!()
}
```

### `claim!`

The `claim!` macro should resolve a claim at build time and produce a tracked parameter at runtime.

### Prompt template DSL

```rust
let prompt = prompt! {
    role: Role::Researcher,
    context: episode.recent(5),
    situation: current_task,
    heuristics: hdc_match(0.7),
};
```

The prompt DSL should stay declarative so the Composer can compile it into the final prompt assembly without ad hoc string manipulation.

---

## 6. Testing ergonomics

The SDK should ship with testing helpers that make agent behavior observable without requiring a live model.

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

The minimum helper set should include `MockAgent`, `AssertingBus`, `RecordingSubstrate`, and a clock helper for advancing demurrage-sensitive flows. A fuller harness can expose seeded heuristics, replayable traces, and Criterion adapters so testing and benchmarking use the same setup as production-like runs.

---

## 7. Tracing and debug UX

Developer UX should include observability by default.

- Emit spans at operator boundaries so custom `Scorer`, `Gate`, `Router`, and `Composer` implementations nest cleanly under the kernel trace.
- Provide `roko::log::init()` as the zero-config path for local tracing setup.
- Offer `roko::dev::tap_bus()` for inspecting Pulse traffic while debugging custom flows.
- Keep `#[roko::instrument]` as sugar over `tracing::instrument` when the added correlation fields are useful.
- Enable extra invariants in debug builds so back-pressure, round-trips, and state transitions fail fast during development.

This layer should make local debugging feel like production tracing with less ceremony, not like a separate toolchain.

---

## 8. Release compatibility

Developer trust depends on predictable evolution.

- Treat any public trait signature change in the core SDK crates as a major version bump.
- Use a six-week release train for minor releases.
- Keep the minimum supported Rust version stable within a minor line and raise it only in minor releases.
- Require changelog updates for API-affecting changes.
- Keep deprecations around for at least two minor versions before removal.

The intent is boring on purpose: Rust developers should be able to adopt the SDK without fear that their first successful integration will churn immediately.

---

## 9. Related docs

- [00-cli-overview.md](./00-cli-overview.md) for the interface hierarchy that sits above the SDK
- [01-cli-command-reference.md](./01-cli-command-reference.md) for the command surface that complements `cargo roko`
- [02-roko-new-scaffolders.md](./02-roko-new-scaffolders.md) for generated implementations of core traits
- [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md) for error-as-teacher UX
- [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for runtime-facing transport and control-plane behavior
- [14-agent-onboarding-flow.md](./14-agent-onboarding-flow.md) for first-run product flow
- [15-generative-interfaces-a2ui.md](./15-generative-interfaces-a2ui.md) for generated interface payloads
- [../02-agents/12-extensibility.md](../02-agents/12-extensibility.md) for how the same four layers map onto custom-agent authoring and extension points

The source proposal that motivated this chapter is [tmp/refinements/22-developer-ux-rust.md](../../tmp/refinements/22-developer-ux-rust.md).
