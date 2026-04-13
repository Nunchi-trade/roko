# Run anywhere

> Agents that run everywhere -- CLI, browser, edge, cloud, chain -- sharing intelligence
> across all deployment targets. The more instances run, the smarter all instances become.
>
> **Updated**: 2026-04-13 · **Audience**: Technical decision-makers, contributors, partners

---

## 1. The thesis

Roko is a cognitive architecture for self-developing agents. Today it runs as a CLI tool on
a developer's laptop. That is a starting point, not an endpoint.

The goal: compile the same Rust core to every target that matters -- native binaries, WASM
in browsers, WASM on edge workers, long-running cloud daemons, and eventually on-chain smart
contract execution. Every instance contributes to a shared intelligence layer. An agent that
learns an optimization in CI teaches that pattern to the agent running in your IDE, which
teaches it to the agent embedded in your documentation site.

This is not about porting software to new platforms. It is about creating a network effect
around learned knowledge. A single Roko agent is useful. A thousand Roko agents sharing
routing weights, skill libraries, and gate thresholds through conflict-free replicated data
structures are something qualitatively different.

**What exists today**: A working CLI agent with plan-execute-gate-learn loops, 18 crates,
~177K LOC, and a pure-Rust core that has zero platform dependencies in its cognitive layer.

**What this document describes**: The path from that single-target CLI to a universal agent
runtime where the compilation target is a deployment detail and the intelligence compounds
across all targets simultaneously.

---

## 2. Isomorphic architecture

### The layer split

Roko's architecture already separates pure computation from platform I/O. The key insight:
the cognitive primitives -- scoring, routing, prompt assembly, learning algorithms,
content-addressed hashing -- are pure functions over typed data. They do not touch the
filesystem, the network, or the operating system. They just compute.

```
+-----------------------------------------------------------+
|                    roko-core (pure Rust)                   |
|  Engram, Score, ToolDef, ChatRequest, ProviderConfig      |
|  NO I/O, NO async, NO platform deps                       |
|  Compiles to: native + wasm32-unknown-unknown + wasip2    |
+-----------------------------+-----------------------------+
                              |
          +-------------------+-------------------+
          |                   |                   |
+---------+---------+ +-------+---------+ +-------+---------+
|   roko-native     | |   roko-wasm     | |   roko-edge     |
|   (CLI / server)  | |   (browser)     | |   (WASI / CDN)  |
|                   | |                 | |                 |
|  tokio            | |  spawn_local    | |  tokio_wasi     |
|  std::fs          | |  IndexedDB      | |  WASI FS        |
|  reqwest(tls)     | |  reqwest(fetch) | |  reqwest(wasi)  |
|  subprocess spawn | |  HTTP API only  | |  HTTP API only  |
|  full gate suite  | |  remote gates   | |  remote gates   |
+---------+---------+ +-------+---------+ +-------+---------+
```

The bridge between the pure core and any platform implementation is four traits:

```rust
#[async_trait(?Send)]  // ?Send for WASM single-threaded compatibility
pub trait StateStore {
    async fn read(&self, key: &str) -> Option<Vec<u8>>;
    async fn write(&self, key: &str, data: &[u8]) -> Result<()>;
    async fn append(&self, key: &str, line: &[u8]) -> Result<()>;
    async fn list_keys(&self, prefix: &str) -> Vec<String>;
}

#[async_trait(?Send)]
pub trait LlmClient {
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse>;
    async fn stream_completion(&self, request: &ChatRequest)
        -> Result<Pin<Box<dyn Stream<Item = StreamChunk>>>>;
}

#[async_trait(?Send)]
pub trait GateRunner {
    async fn run_gate(&self, gate: &str, workdir: &str) -> Result<GateResult>;
}

#[async_trait(?Send)]
pub trait EventSink {
    fn publish(&self, event: AgentEvent);
}
```

The `?Send` bound is the critical detail. WASM futures are `!Send` (single-threaded). Using
`?Send` makes these traits compatible with both native (`Send` futures using tokio) and WASM
(`!Send` futures using `spawn_local`). No conditional compilation needed at the trait
definition layer.

### What this buys you

Write a scoring function once. It works in a browser, on an edge worker, in CI, and on the
CLI. Write a routing algorithm once. Same. The platform adapter handles the I/O; the
cognitive logic stays identical. Content-addressed hashing (BLAKE3) produces the same hash
on every target, so an Engram created in a browser has the same identity as one created in
the CLI -- they can be merged, synced, and deduplicated across environments.

---

## 3. WASM compilation

### What compiles today with zero changes

| Component | Crate | WASM status |
|---|---|---|
| Config parsing | `serde`, `serde_json`, `toml` | Works |
| Model routing | `cascade_router.rs` (pure math) | Works |
| Learning algorithms | LinUCB, Thompson Sampling, UCB1 | Works |
| Cost normalization | `cost_table.rs` | Works |
| Prompt assembly | `system_prompt_builder.rs` | Works |
| Episode and signal types | All roko-core types | Works |
| Content addressing | BLAKE3 | Works (native WASM support) |
| HDC vectors | `HdcVector`, Hamming distance, XOR bundling | Works |
| Decay calculations | HalfLife, TTL, Ebbinghaus | Works |
| MemorySubstrate | In-memory BTreeMap store | Works (used in all tests) |

### What needs platform abstraction

| Component | Native implementation | Browser implementation | Edge (WASI) implementation |
|---|---|---|---|
| Async runtime | tokio | wasm-bindgen-futures `spawn_local` | tokio_wasi (WasmEdge) |
| HTTP client | reqwest (native TLS) | reqwest (Fetch API) | reqwest (WASI sockets) |
| Persistence | std::fs (JSONL) | IndexedDB / OPFS | WASI filesystem |
| Sync primitives | parking_lot::Mutex | RefCell (single-threaded) | parking_lot |
| Random | rand | rand (js feature) | rand (wasi feature) |
| Time | chrono | js_sys::Date | WASI clocks |

### Hard blockers that require architectural change

| Component | Why it fails in WASM | Solution |
|---|---|---|
| Agent dispatch (subprocess) | `std::process::Command` does not exist | Replace CLI spawn with HTTP API calls to LLM providers |
| Gate pipeline (cargo/clippy) | Cannot run compilers in a browser | Delegate to a server-side gate runner via A2A |
| MCP stdio servers | Cannot spawn subprocesses | Use HTTP/SSE MCP transport |
| File watcher | No inotify/FSEvents | Use polling or IndexedDB change events |
| Tree-sitter parsing | C FFI dependency | Use pre-computed indexes |

### Binary size budget

| Configuration | Uncompressed | Gzipped |
|---|---|---|
| Full agent (all features) | ~3-5 MB | ~800 KB - 1.5 MB |
| Core only (routing + learning) | ~500 KB - 1 MB | ~150-300 KB |
| Minimal (heuristic-only, no LLM) | ~100-200 KB | ~40-80 KB |

Build optimization for WASM:

```toml
[profile.wasm]
inherits = "release"
opt-level = "z"
lto = "fat"
codegen-units = 1
strip = true
panic = "abort"
```

Then run `wasm-opt -Oz` on the output. The 500KB gzipped target for the cognitive kernel
is achievable -- serde_json accounts for ~200KB, and everything else is lean computation.

### wasm-bindgen strategy

Browser-facing WASM uses `wasm-bindgen` to expose Roko's cognitive primitives to JavaScript.
The binding layer wraps the `WasmAgent` struct, exposing observe/query/route methods as
async functions that map to JavaScript Promises. The build pipeline:
`cargo` -> `wasm-pack` -> `pkg/roko_wasm.js` + `pkg/roko_wasm_bg.wasm`.

For WASI targets (edge workers, wasmtime), the WASM Component Model provides typed WIT
interfaces (`scoring.wit`, `routing.wit`, `gating.wit`, `learning.wit`) that can be composed
at the binary level using `wasm-tools compose`. Each component is independently versioned,
tested, and replaceable -- swap the routing algorithm by plugging a different component, no
rebuild required.

---

## 4. Deployment targets

### CLI (primary) -- shipping today

The current deployment. Native binary compiled with full feature set. Runs the complete
plan-execute-gate-learn loop with subprocess agent dispatch, all 11 gates, full filesystem
persistence. This is the reference implementation and the development target for all new
features.

- **Status**: Shipping. Self-hosting loop works end-to-end.
- **Binary**: ~15 MB (release, stripped)
- **Platforms**: macOS (x86_64 + aarch64), Linux (glibc + musl)

### Browser (WASM) -- specified, not built

A "smart thin client" that runs all intelligence locally while delegating I/O to the network.
Routing decisions, prompt assembly, cost tracking, anomaly detection, and learning updates
execute in the browser. LLM API calls, gate execution, and file syncs go over the network.

The browser agent gets smarter over time through locally persisted state in IndexedDB/OPFS.
It syncs periodically with a server for collective learning. Between syncs, it operates
independently.

What runs locally:
- CascadeRouter (model selection)
- PromptAssembler (context construction)
- TokenCounter (tiktoken-wasm)
- CostTable (spend tracking)
- AnomalyDetector (drift detection)
- EpisodeLogger (IndexedDB)
- SkillLibrary (IndexedDB)

What goes to the network:
- LLM provider API calls (via browser Fetch API)
- Gate execution (delegated to server via A2A)
- CRDT sync (merge with other instances)

Precedent: Bolt.new runs a full Node.js development environment in the browser via
WebContainers. Mozilla's WASM Agents Blueprint runs OpenAI's agent SDK in WASM via Pyodide.
Notion uses WASM SQLite + OPFS for browser persistence. The technical path is proven.

- **Status**: Specified. Feature flags and MemorySubstrate exist. `roko-wasm` crate not yet
  created.
- **Target size**: ~500 KB gzipped

### Edge / embedded (WASM or no_std native subset) -- specified, not built

A ~500 KB binary or WASM module for resource-constrained environments. Runs the cognitive
kernel (scoring, routing, HDC similarity) but delegates everything else to a core node.
Targets Cloudflare Workers (300+ edge locations, sub-ms cold start), Fermyon Spin, wasmCloud
lattice, IoT gateways, and embedded Linux.

The use case: a two-tier architecture where 80% of requests are handled at the edge without
an LLM call (T0 zero-LLM classification), and 20% are forwarded to the full agent. This
maps directly to Roko's dual-process cognition model.

Real numbers from Cloudflare Workers: ~2ms average CPU time per request, 128 MB memory per
isolate, sub-millisecond cold start.

- **Status**: Specified. Core compiles with `--no-default-features`. Binary size not validated.
- **Target size**: ~500 KB

### Cloud daemon (roko-serve) -- specified, not built

A long-lived HTTP service for remote orchestration. REST API for projects, plans, runs, PRDs,
and artifacts. SSE event streaming for live progress. WebSocket for bidirectional control.
API key authentication with read/write/admin scopes.

This is the server that browser WASM agents delegate to for gate execution, and that edge
agents forward complex requests to. Deployed on Fly.io with auto-stop/auto-start, persistent
volumes, and private networking.

- **Status**: Specified. API design documented. No code.
- **Deployment**: Fly.io (primary), Docker (self-hosted)

### On-chain (Korai) -- built, blocked

A dedicated EVM for agent coordination: soulbound identity passports, 7-domain reputation
with EMA, Spore/Sparrow job marketplace, HDC precompile at ~400 gas, KORAI/DAEJI tokens
with 1% annual demurrage, and ISFR clearing. 52 tests pass. Blocked by chain deployment
decision.

The on-chain target is not about running full agents on-chain. It is about three things:
identity (ERC-8004 agent registries), reputation (cross-validated by on-chain evidence), and
coordination (job matching, payment clearing, knowledge marketplace settlement).

- **Status**: 52 tests. Blocked by chain deployment. Deferred to Phase 2.

---

## 5. Distributed learning: shared intelligence via Merkle-CRDTs

### The exponential insight

Every Roko instance -- browser, CLI, edge, CI -- produces learning data. Routing observations
(which model succeeded on which task type). Gate threshold updates (pass rates per rung).
Skill discoveries (successful tool-use patterns). Cost data (actual spend per model per
provider).

If N instances share this data, each instance learns N times faster. With 1,000 users, a new
user's agent starts with the collective experience of all 1,000. This is the network effect
that separates "a tool" from "a platform."

### Merkle-CRDTs: the sync mechanism

Merkle-CRDTs combine two proven technologies:

**CRDTs** (Conflict-free Replicated Data Types) guarantee eventual consistency without
coordination. Any replica processes writes independently. Merges are commutative, associative,
and idempotent. Two nodes that start from the same state and apply the same set of operations
in any order converge to the same result.

**Merkle DAGs** enable efficient pair-wise reconciliation. Instead of syncing full state,
nodes exchange tree hashes to identify divergences. Only the divergent subtrees are
transferred.

### How Roko's learning state maps to CRDTs

| Data | CRDT type | Merge strategy |
|---|---|---|
| Routing observations | G-Counter per (model, category) | Sum observations across instances |
| Gate thresholds (EMA) | LWW-Register with Lamport timestamps | Latest writer wins |
| Skill library | G-Set (add-only) | Union of all discovered skills |
| Experiment results | G-Set of (variant_id, outcome) | Union of all observations |
| Cost records | G-Set | Union, deduplicate by record_id |
| Playbook rules | OR-Set (add + remove) | Merge with tombstones |
| Provider health | LWW-Map | Latest observation per provider |

### The sync protocol

```
Instance A                              Instance B
    |                                       |
    |--- Merkle root hash ----------------->|
    |<-- Merkle root hash ------------------|
    |                                       |
    |  (Roots differ: need sync)            |
    |                                       |
    |--- Divergent subtree hashes --------->|
    |<-- Missing entries from B ------------|
    |--- Missing entries from A ----------->|
    |                                       |
    | (Both merge. Roots now match.)        |
```

Network failures are fine. The CRDTs converge regardless of sync order or frequency. An
instance that goes offline for a week catches up with a single Merkle-CRDT exchange when it
reconnects.

### Privacy-preserving mode

For sensitive codebases, sync only aggregate statistics:

- Routing: "(model X, task_type Y) had 82% pass rate across 203 observations" -- no code
- Skills: "Read-before-Edit pattern succeeds 94% of the time" -- no file paths
- Costs: "$0.19 average for GLM-5.1 on implementation tasks" -- no prompts

Share the learned patterns, not the learning data. This is differential privacy at the
application level.

---

## 6. Protocol stack: MCP + A2A + ACP

Three protocols, three layers. Each solves a different coordination problem.

```
Layer 3: User interface    ACP (Agent Client Protocol)
         Agent <-> IDE     roko <-> VS Code / Zed / JetBrains

Layer 2: Agent coordination  A2A (Agent-to-Agent Protocol)
         Agent <-> Agent     roko edge <-> roko cloud <-> third-party agents

Layer 1: Tool access       MCP (Model Context Protocol)
         Agent <-> Tools   roko <-> code search / databases / APIs
```

### MCP: tool access (partially implemented)

Roko already consumes MCP tools via `agent.mcp_config` in `roko.toml` with auto-discovery
fallback. The next step: Roko as an MCP provider.

`roko serve --mcp` exposes Roko's capabilities as MCP tools. Other agents and editors can
use Roko for plan execution, research, code review, and status queries -- treating Roko as
a tool rather than an application. This turns every Roko instance into a capability node in
the MCP ecosystem.

### A2A: agent-to-agent coordination (not built)

Google's Agent-to-Agent protocol enables agents to discover each other, negotiate
capabilities, and delegate tasks. Roko publishes an Agent Card:

```json
{
  "name": "roko",
  "url": "https://roko.local:9090/a2a",
  "version": "0.1.0",
  "capabilities": {
    "streaming": true,
    "pushNotifications": true,
    "stateTransitionHistory": true
  },
  "skills": [
    {
      "id": "plan-execute",
      "name": "Execute coding plan with gate verification"
    },
    {
      "id": "code-review",
      "name": "Multi-reviewer code review"
    },
    {
      "id": "research",
      "name": "Deep research with citations"
    }
  ]
}
```

A2A is how the browser agent delegates compilation gates to the server agent. It is how
multiple Roko instances discover each other for CRDT sync. And it is how third-party agents
(LangGraph, CrewAI) delegate tasks to Roko without knowing anything about Roko's internals.

### ACP: IDE integration (not built)

`roko acp` starts Roko as an ACP agent over stdio, streaming plan progress, gate results,
and learning metrics into the IDE. This is the interface that turns Roko from a CLI tool into
an IDE-integrated development partner. The agent receives prompts and approval decisions from
the editor and reports structured progress back.

### How the protocols compose

A typical flow: the IDE sends a task to Roko via ACP (layer 3). Roko uses MCP to access code
search and documentation tools (layer 1). When it needs compilation gate results from a
remote server, it delegates via A2A (layer 2). The browser WASM agent discovers the server
agent through an A2A Agent Card, delegates gate execution, and receives results -- all
through standard protocols that any agent framework can implement.

---

## 7. Portable agent state: brain dump / restore

### The format

A Roko "brain dump" packages all learned state into a portable, importable artifact:

```
roko-brain-v1/
  manifest.json              # version, source instance, timestamp, capabilities
  routing/
    cascade-router.json      # confidence stats + LinUCB arm state
    thompson-arms.json       # Thompson Sampling beta distributions
    static-table.json        # cold-start routing defaults
  learning/
    gate-thresholds.json     # EMA pass rates per rung
    efficiency-summary.json  # aggregated efficiency metrics
    cost-table.json          # observed costs per model
    latency-stats.json       # observed latency per provider
  skills/
    skill-library.json       # discovered skills with confidence scores
    playbook-rules.json      # validated playbook entries
  heuristics/
    patterns.json            # mined patterns from episodes
    section-effects.json     # prompt section effectiveness data
  experiments/
    prompt-experiments.json  # A/B test results
    model-experiments.json   # model comparison results
  config/
    providers.json           # provider configurations (API keys stripped)
    models.json              # model profiles
```

### Import merges, not overwrites

Import uses the same CRDT semantics as the distributed sync layer. Importing a brain into an
existing instance merges the two knowledge bases -- routing observations sum, skills union,
thresholds take the latest. No data loss on either side.

```bash
# export learned state
roko brain export --output roko-brain-v1.tar.gz

# import into a new instance (merges, does not overwrite)
roko brain import roko-brain-v1.tar.gz

# share aggregates with the collective (privacy-preserving)
roko brain share --mode aggregate --endpoint https://collective.example.com/sync
```

### What this enables

**Onboarding**: A new team member imports the team's collective brain. Their agent starts
with calibrated routing weights, proven skills, and accurate thresholds instead of cold-start
defaults.

**Cross-project transfer**: Skills learned on project A ("always run tests after editing
lib.rs") transfer to project B. Routing weights learned on one codebase inform model selection
on another.

**Recovery**: If you lose your `.roko/` directory, restore from a brain dump. The learning
that took weeks to accumulate is back in seconds.

---

## 8. Agent marketplace and skills economy

### The state of the art

Agensi (agensi.io) sells agent skills as one-time purchases that work across Claude Code,
Codex CLI, Cursor, and 20+ tools. Anthropic's Agent Skills specification (adopted by
OpenAI, Microsoft, Google, Cursor, GitHub, Figma) standardizes skills as SKILL.md files
with YAML frontmatter. Over 1,600 security-vetted skills are indexed.

### What Roko adds

Roko can trade not just skills (static instructions) but learned state (empirically validated
knowledge):

| Product type | What it contains | Example |
|---|---|---|
| Skill pack | SKILL.md files + playbook rules | "Rust error handling best practices" |
| Brain | Full learned state (routing + skills + thresholds) | "Senior Rust developer brain" |
| Routing profile | Trained CascadeRouter weights | "Cost-optimized GLM+Kimi router" |
| Gate config | Custom gate pipeline + thresholds | "High-security gate pipeline" |
| Template pack | Plan + task templates for workflows | "Microservice migration kit" |

The difference from existing skill marketplaces: a Roko brain that says "GLM-5.1 passes 82%
on implementation tasks at $0.19/task" is backed by empirical data, not opinion. The routing
weights are statistically derived from thousands of observations. The gate thresholds are
EMA-smoothed from actual pass/fail data. This is not "someone wrote a prompt"; this is
"someone's agent learned something and you can import that learning."

### Verification and trust

Brains include provenance metadata: how many observations back each routing weight, what
gate pass rates the thresholds are derived from, and the confidence intervals on each
statistic. A buyer can verify that a "senior Rust developer brain" actually has 10,000+
routing observations across implementation, review, and research tasks -- not 50 observations
relabeled.

---

## 9. Self-modifying architecture

### The research

Three recent papers demonstrate agents improving their own scaffold:

- **HyperAgents** (Meta, ICLR 2026): Merges the task agent and meta-agent into a single
  self-modifiable codebase. 3x improvement on coding benchmarks through self-modification.
- **Darwin Godel Machine** (Sakana AI): Darwinian evolution + Godelian self-improvement.
  SWE-bench from 20% to 50%.
- **A-Evolve** (open source): Five-stage evolutionary loop (Solve -> Observe -> Evolve ->
  Gate -> Reload). Git-tagged mutations with automatic rollback on regression.

### How Roko maps to this

Roko already has four of the five stages:

| Stage | Roko implementation | Status |
|---|---|---|
| Solve | Agent executes tasks via plan runner | Shipping |
| Observe | EpisodeLogger + EfficiencyEvents capture data | Shipping |
| Evolve | roko-neuro distiller extracts heuristics | Built, not wired |
| Gate | 11-gate pipeline validates changes | Shipping |
| Reload | `--resume` reloads state; playbook rules inject into prompts | Shipping |

The missing piece is closing the loop: using gate failure data to automatically generate
improved heuristics, test them as experiments, and promote winners to the playbook. The
building blocks exist. The wiring does not.

### The safe path to self-modification

```
1. Agent runs task -> gate fails
2. Neuro distiller analyzes failure -> extracts heuristic
3. Heuristic is tested on next similar task -> tracked as experiment
4. If heuristic improves pass rate by >5% with p<0.05 -> promote to playbook
5. Playbook rule is injected into all future prompts
6. Agent behavior has changed WITHOUT modifying code
```

This is self-modification at the prompt level, not the code level. It is safe because the
gate pipeline validates every modification. A bad heuristic fails gates and is never promoted.
A good heuristic passes gates and becomes part of the agent's permanent knowledge.

Code-level self-modification (the HyperAgents pattern) is possible in principle -- Roko
already reads its own PRDs and generates implementation plans. An agent that writes a PR to
improve its own scoring function, validates it through the gate pipeline, and merges it is
within reach. But it is a Phase 5+ capability that requires the trust and verification
infrastructure to be rock-solid first.

---

## 10. Current reality vs vision

### What works today (shipping)

- CLI agent with full plan-execute-gate-learn loop
- 5 LLM backends (Claude CLI, Anthropic API, OpenAI-compat, Cursor ACP, Ollama)
- 3-stage CascadeRouter (Static -> Confidence -> UCB) for model selection
- 11-gate, 7-rung verification pipeline with adaptive thresholds
- 7-layer SystemPromptBuilder with 12 role templates
- EpisodeLogger, EfficiencyEvents, pattern mining, cost tracking
- MCP tool consumption with auto-discovery
- Session persistence and resume
- Self-hosting loop: `prd idea` -> `prd draft` -> `research` -> `prd plan` -> `plan run`

### What is real but not yet wired

- **Neuro knowledge store**: 6 knowledge types x 4 validation tiers, HDC vectors. Built,
  52+ tests. Not connected to runtime context injection.
- **Daimon affect engine**: PAD vectors modulating model tier and exploration. Built. Not
  connected to routing decisions.
- **Conductor regulator**: 10 watchers, circuit breakers, anomaly detection. Built. Not
  called from the orchestrator.
- **Code intelligence**: Tree-sitter parsing, symbol graph, HDC fingerprints. 30 tests.
  No MCP server for agents to consume.
- **Korai chain**: ERC-8004 registries, reputation, marketplace. 52 tests. Blocked by chain
  deployment.

### Near-term (months 1-4)

| Priority | What | Why it matters |
|---|---|---|
| 1 | Interactive TUI (ratatui) | Primary operator interface for monitoring agents |
| 2 | Automatic plan generation | Removes manual step from self-hosting loop |
| 3 | Failure feedback loop | Closes learn-from-failure cycle |
| 4 | Wire Neuro, Daimon, Conductor | Cognitive subsystems provide memory, affect, regulation |
| 5 | Platform abstraction traits | StateStore, LlmClient, GateRunner, EventSink |
| 6 | `roko-core` compiles to WASM | Verify: `cargo build --target wasm32-unknown-unknown` |

After priorities 1-3, Roko can develop itself end-to-end without human intervention beyond
initial PRD creation. After 5-6, the multi-target compilation story is validated.

### Medium-term (months 5-8)

| Priority | What | Why it matters |
|---|---|---|
| 7 | Browser prototype (roko-wasm) | CascadeRouter + PromptAssembler in a browser |
| 8 | ACP over stdio | IDE integration (VS Code, Zed, JetBrains) |
| 9 | A2A Agent Card | Agent discovery and task delegation |
| 10 | roko-serve HTTP API | Remote orchestration for browser and edge agents |
| 11 | Brain export/import | Portable agent state |
| 12 | CRDT types for learning state | Foundation for distributed sync |

### Long-term (months 9-14)

| Priority | What | Why it matters |
|---|---|---|
| 13 | Merkle-CRDT sync protocol | Distributed learning across instances |
| 14 | WASM Component Model (WIT) | Modular agent components, swap algorithms without rebuild |
| 15 | Edge deployment (Cloudflare Workers, Spin) | Scoring and routing at the network edge |
| 16 | Agent marketplace | Trade brains, skills, routing profiles |
| 17 | Collective learning (privacy-preserving) | Network effect: N instances, N times the learning |
| 18 | Prompt-level self-modification | Gate-validated heuristic evolution |

### Aspirational (Phase 2+)

- Korai chain deployment and ERC-8004 registries
- On-chain reputation and knowledge marketplace settlement
- Code-level self-modification (HyperAgents pattern)
- Full dream cycle (NREM replay, REM imagination, hypnagogia)
- Morphogenetic specialization in multi-agent collectives
- Stigmergic coordination via digital pheromones

---

## Guiding principles

**1. The scaffold IS the product.** Every improvement to Roko improves the system that builds
Roko. Prioritize features that compound.

**2. Intelligence over presence.** Running on a new platform is worthless if the agent is
dumb there. Each new target must participate in the learning loop.

**3. Graceful degradation, not feature gates.** An edge agent with no LLM access should fall
back to cached heuristics, not error out. A browser agent with no server should still route
and score locally.

**4. Honesty about maturity.** Label shipping features as shipping. Label specifications as
specifications. Do not present a roadmap item as a capability.

**5. Network effects require open protocols.** MCP, A2A, and ACP are open standards. Roko's
brain dump format will be documented. The marketplace runs on portable artifacts, not lock-in.

**6. Privacy is non-negotiable.** The distributed learning layer syncs aggregate statistics,
not source code, prompts, or proprietary data. Users control what they share.

---

## Architecture summary

```
+=====================================================================+
|                        Deployment targets                           |
|                                                                     |
|   CLI        Browser      Edge         Cloud        Chain           |
|   (native)   (WASM)       (WASM/native)(native)     (EVM)          |
|   shipping   specified    specified    specified    built/blocked   |
+======+============+===========+============+==========+=============+
       |            |           |            |          |
+======+============+===========+============+==========+=============+
|                        Protocol stack                               |
|                                                                     |
|   ACP (IDE)    A2A (agents)    MCP (tools)                         |
|   not built    not built       partial                              |
+=====================================================================+
|                                                                     |
|                     Distributed learning                            |
|                                                                     |
|   Merkle-CRDTs     Brain export/import     Privacy-preserving sync  |
|   not built         not built               not built               |
+=====================================================================+
|                                                                     |
|                     Cognitive core (pure Rust)                      |
|                                                                     |
|   Engram   Score   Router   Composer   Gate   Scorer   Policy       |
|   HDC vectors   Decay models   Learning algorithms   Heuristics    |
|                                                                     |
|   shipping (compiles to native + WASM from same source)             |
+=====================================================================+
```

---

## Key risks

**WASM compilation has not been validated end-to-end.** The cognitive kernel compiles with
`--no-default-features`, and individual components (BLAKE3, serde, HDC vectors) are known to
work in WASM. But a full `cargo build --target wasm32-unknown-unknown -p roko-core` has not
been run and verified. Unknown dependency issues may surface.

**The platform abstraction traits do not exist yet.** The `StateStore`, `LlmClient`,
`GateRunner`, and `EventSink` traits shown in this document are designs, not code. Extracting
them from the current implementation requires touching the hot path in orchestrate.rs and the
agent dispatcher.

**CRDT implementation is substantial engineering.** Merkle-CRDT sync involves implementing
multiple CRDT types, a Merkle tree, a reconciliation protocol, and conflict resolution for
each data structure. The sync-from-scratch path is months of work. Using an existing CRDT
library (like `yrs` or a custom crate) would reduce that significantly.

**The marketplace requires critical mass.** A skills economy is worthless with 10 users. The
marketplace is viable only after Roko has a significant user base producing brains worth
trading. Sequence matters: ship the self-hosting loop, build the user base, then launch the
marketplace.

**Self-modification needs trust infrastructure.** Prompt-level self-modification is safe
because gate validation catches bad heuristics. Code-level self-modification is dangerous
without formal verification of the gate pipeline itself. Do not rush this.

---

*This document describes where Roko is going. For where it is today, see
[STATUS.md](STATUS.md). For the architectural foundations, see
[EXECUTIVE-SUMMARY.md](EXECUTIVE-SUMMARY.md). For the full PRD corpus, see
[INDEX.md](INDEX.md).*

*2026-04-13 -- Roko v0.1 -- 18 crates -- 1,568 tests*
