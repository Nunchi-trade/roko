# 00 — The Agent Trait

> Sub-doc 00 of **02-agents** · Roko Documentation
>
> This document defines the `Agent` trait, explains why it exists as a separate
> capability outside the six Synapse traits, documents the `AgentResult` type,
> and traces the design lineage from Mori's agent connection layer.

---

## Why Agents Are Separate from the Six Synapse Traits

Roko's core architecture is built on six composable verb traits — the
**Synapse traits** — that process Engrams (currently named `Signal` in code,
rename tracked as Tier 0D):

| Trait | Verb | Signature shape |
|---|---|---|
| Substrate | store / retrieve | `fn query(&self, …) → Vec<Signal>` |
| Scorer | evaluate | `fn score(&self, signal) → f64` |
| Gate | accept / reject | `fn check(&self, signal) → Verdict` |
| Router | direct | `fn route(&self, signal) → Destination` |
| Composer | assemble | `fn compose(&self, signals) → Signal` |
| Policy | decide | `fn decide(&self, …) → Action` |

These traits share four properties: they are **synchronous**, **deterministic**
(given fixed inputs), **side-effect-free**, and they process **single Engrams**
at a time.

An **Agent** violates all four:

1. **Async execution** — Agents spawn subprocesses, call LLM APIs over HTTP,
   and wait for network responses. Every agent call is `async`.
2. **Side effects** — Agents edit files, run shell commands, write to stdout,
   and mutate the filesystem. These side effects are the whole point.
3. **Multiple signals** — A single agent run produces a stream of intermediate
   signals (tool calls, diff updates, status messages) before emitting its
   final output.
4. **Non-deterministic** — LLMs are stochastic. The same prompt can produce
   different outputs on every run.

Rather than distort one of the six Synapse traits (e.g., making `Composer`
async and side-effecting), Roko introduces `Agent` as its own capability
extension. The core stays clean; agent implementations live in `roko-agent`.

This design decision is documented in the trait's own source:

```rust
// crates/roko-agent/src/agent.rs

/// Agents don't fit any of the 6 core traits because they:
/// 1. Are **async** (subprocess, network, LLM API)
/// 2. Have **side effects** (file edits, stdout)
/// 3. Produce **multiple signals** over time (stream)
/// 4. Are **non-deterministic** (LLMs are stochastic)
///
/// Rather than distort another trait, `Agent` is its own capability.
```

Reference: The Synapse architecture is defined in the refactoring PRD
§01-synapse-architecture. The CoALA cognitive architecture (Sumers et al.,
2023, arXiv:2309.02427) provides the theoretical grounding for separating
perception/reasoning (the six traits) from action execution (agents).

---

## The Agent Trait

The trait lives at `crates/roko-agent/src/agent.rs` and has three methods:

```rust
#[async_trait]
pub trait Agent: Send + Sync {
    /// Run the agent against the input signal.
    ///
    /// The `input` is typically a `Signal<Kind::Prompt>`, but agents may
    /// accept any kind (e.g. a `Signal<Kind::Task>` for task-aware agents).
    async fn run(&self, input: &Signal, ctx: &Context) -> AgentResult;

    /// Human-readable name for logs/metrics.
    fn name(&self) -> &str;

    /// Does this agent emit a streaming trace (many signals), or a single output?
    fn supports_streaming(&self) -> bool {
        false
    }
}
```

### Design notes

- **`Send + Sync`** — Required because the orchestrator runs agents across
  `tokio` tasks. Every concrete implementation must be thread-safe.
- **`&Signal` input** — The input is borrowed, not consumed. This allows the
  orchestrator to keep the original prompt signal for logging and DAG lineage
  while the agent works with a reference.
- **`&Context` context** — The `Context` carries a timestamp and potentially
  other runtime metadata. It provides a clean injection point for contextual
  information without polluting the trait signature with extra parameters.
- **`AgentResult` return** — Not `Result<T, E>` — agents always return an
  `AgentResult` that wraps success/failure as a boolean flag, because even
  "failed" agent runs produce useful diagnostic output that the orchestrator
  needs for logging and retry decisions.

---

## AgentResult

The result of running an agent once. Defined at `crates/roko-agent/src/agent.rs`:

```rust
#[derive(Clone, Debug)]
pub struct AgentResult {
    /// The primary output signal (Kind::AgentOutput with the agent's response).
    pub output: Signal,

    /// Intermediate signals emitted during the run (stream messages, tool calls,
    /// diff updates, errors). Ordered chronologically.
    pub trace: Vec<Signal>,

    /// Token usage + cost.
    pub usage: Usage,

    /// Whether the agent ran successfully
    /// (non-zero exit / connection errors = false).
    pub success: bool,
}
```

### Constructors and builder methods

```rust
impl AgentResult {
    /// Construct a successful result with just an output signal.
    pub const fn ok(output: Signal) -> Self;

    /// Construct a failed result with an output signal describing the failure.
    pub const fn fail(output: Signal) -> Self;

    /// Attach trace signals.
    pub fn with_trace(mut self, trace: Vec<Signal>) -> Self;

    /// Attach usage metrics.
    pub const fn with_usage(mut self, usage: Usage) -> Self;

    /// All signals produced by this run (trace + output), chronological order.
    pub fn all_signals(&self) -> Vec<Signal>;
}
```

The `all_signals()` method returns `trace` followed by `output` — the
chronological order matters for episode logging, where each signal becomes a
row in `.roko/episodes.jsonl`.

### Usage tracking

The `Usage` struct (from `crates/roko-agent/src/usage.rs`) captures:

- `input_tokens` — Tokens sent to the model
- `output_tokens` — Tokens received from the model
- `cache_read_tokens` — Tokens served from cache (Anthropic prompt caching)
- `cache_write_tokens` — Tokens written to cache
- `cost_usd` — Estimated dollar cost
- `duration_ms` — Wall-clock time for the run
- `model` — Which model was used (for cost attribution)

This feeds into the efficiency tracking pipeline: each `AgentResult` is logged
by the `EpisodeLogger` and the efficiency events feed into
`.roko/learn/efficiency.jsonl` for the learning subsystem.

---

## Concrete Implementations

Roko ships six agent implementations, each targeting a different backend:

| Implementation | Module | Backend | Protocol |
|---|---|---|---|
| `ClaudeCliAgent` | `claude_cli_agent.rs` | `claude` CLI | Stream-JSON subprocess |
| `ClaudeAgent` | `claude_agent.rs` | Anthropic Messages API | HTTP JSON |
| `OpenAiAgent` | `openai_agent.rs` | OpenAI Chat Completions | HTTP JSON |
| `OllamaAgent` | `ollama_agent.rs` | Ollama `/api/chat` | HTTP JSON |
| `ExecAgent` | `exec.rs` | Any CLI binary | stdin/stdout subprocess |
| `MockAgent` | `mock.rs` | In-memory | Deterministic test double |

Additionally, the `CursorAgent` (`cursor_agent.rs`) targets the Cursor Agent
Client Protocol (ACP) over JSON-RPC.

Each implementation encapsulates the full lifecycle of a single agent run:
spawning the process or opening the HTTP connection, sending the prompt,
collecting intermediate outputs, parsing the final result, and computing usage
metrics.

### ExecAgent — the legacy fallback

`ExecAgent` is the original agent implementation from Roko's early development.
It spawns any CLI binary, pipes the prompt to stdin, and captures stdout:

```rust
pub struct ExecAgent {
    command: String,
    args: Vec<String>,
    name: String,
}
```

It remains in the codebase as a **legacy fallback** for situations where no
model-specific agent is available. The orchestrator (`orchestrate.rs`) currently
still uses `ExecAgent` for non-Claude backends as part of the `run_prepared_agent`
flow at line 451. Migration to the provider-based `create_agent_for_model` factory
is tracked as a Tier 1 integration priority.

### MockAgent — the test double

`MockAgent` returns a predetermined response for any input. It is used
extensively in unit tests throughout `roko-agent` and `roko-orchestrator`:

```rust
let mock = MockAgent::new("test-agent", "predetermined response");
let result = mock.run(&prompt, &Context::now()).await;
assert!(result.success);
```

---

## How the Orchestrator Calls Agents

The primary agent call site is `crates/roko-cli/src/orchestrate.rs`, in the
`run_prepared_agent` function (line 451). Here is the dispatch flow:

```
orchestrate.rs::run_prepared_agent(cfg: AgentRunConfig)
    ├── if cfg.command == "claude"
    │   └── ClaudeCliAgent::new(...)
    │       ├── .with_timeout_ms(cfg.timeout_ms)
    │       ├── .with_bare_mode(cfg.bare_mode)
    │       ├── .with_effort(cfg.effort)
    │       ├── .with_system_prompt(cfg.system_prompt)
    │       ├── .with_tools(cfg.allowed_tools_csv)
    │       ├── .with_mcp_config(mcp_path)
    │       ├── .with_fallback_model(fallback)
    │       └── agent.run(&prompt_signal, &ctx)
    └── else
        └── ExecAgent::new(...)
            └── agent.run(&prompt_signal, &ctx)
```

The `AgentRunConfig` struct at line 431 collects all the parameters needed to
run a single agent subprocess in isolation — command, model, timeout, system
prompt, tools, MCP config, environment variables, and extra CLI arguments. This
struct is constructed from `PlanRunner` state and passed to the async function
so that no borrows of the runner are held during parallel execution.

### The provider-based alternative

The newer code path, `create_agent_for_model` (in `crates/roko-agent/src/provider/mod.rs`),
resolves the model from config and creates an agent through the provider adapter
layer. This is documented in detail in sub-doc 02 (Provider Registry) and
sub-doc 03 (Provider Adapters). The eventual goal is for all agent creation in
`orchestrate.rs` to go through `create_agent_for_model`, eliminating the
manual dispatch in `run_prepared_agent`.

---

## Relationship to the Universal Cognitive Loop

In Roko's universal loop — query → score → route → compose → act → verify →
write → react — the Agent occupies the **act** step. The loop is:

1. **Query** — `Substrate.query()` retrieves relevant signals
2. **Score** — `Scorer.score()` evaluates relevance
3. **Route** — `Router.route()` selects the model/backend
4. **Compose** — `Composer.compose()` assembles the prompt
5. **Act** — `Agent.run()` executes the prompt
6. **Verify** — `Gate.check()` validates the output
7. **Write** — `Substrate.write()` persists the result
8. **React** — `Policy.decide()` determines next action

The Agent is the bridge between the pure, composable Synapse world and the
impure, side-effecting real world. This separation is deliberate: it keeps
the six Synapse traits testable and deterministic while allowing agents to
do whatever is needed to complete their task.

Reference: The universal loop is derived from the CoALA 9-step cognitive
cycle (Sumers et al., 2023, arXiv:2309.02427), adapted for Roko's
trait-based composition model. See refactoring PRD §01-synapse-architecture
for the full mapping.

---

## Citations

1. Sumers, T. R. et al. (2023). "Cognitive Architectures for Language Agents."
   arXiv:2309.02427. — Theoretical basis for separating perception/reasoning
   from action execution.
2. Refactoring PRD §01-synapse-architecture — Engram struct and 6 Synapse trait
   definitions.
3. Refactoring PRD §05-agent-types — Agent role compositions and extensibility.
4. `crates/roko-agent/src/agent.rs` — Agent trait and AgentResult source.
5. `crates/roko-cli/src/orchestrate.rs:451` — Primary agent call site.
