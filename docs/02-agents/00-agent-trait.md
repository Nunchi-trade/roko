# 00 — The Agent Trait

> Sub-doc 00 of **02-agents** · Roko Documentation
>
> This document defines the `Agent` trait, explains why it exists as a separate
> capability outside the six Synapse traits, documents the `AgentResult` type,
> and traces the design lineage from Mori's agent connection layer.


> **Implementation**: Shipping

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

## Agent Composition

Can you compose two agents into a new one — e.g., merge a coder + reviewer?
Yes. Research identifies two fundamentally different approaches: **compilation**
(merge into a single agent with combined skills) and **coordination** (keep
agents separate but wire them together).

### Compilation: Multi-Agent → Single-Agent

Compiling a multi-agent team into a single agent with a skill library reduces
token consumption by 53.7% on average and latency by 50% (arXiv:2601.04748,
2025). The largest savings come from eliminating redundant context repetition
across agent calls.

**Critical limitation:** Skill selection accuracy degrades non-linearly as
libraries grow. There is a **phase transition around 50–100 skills** where
semantic confusability causes selection failures. SkillReducer
(arXiv:2603.29919) achieves 86% pass rate across 600 skills via delta-debugging
routing compression and taxonomy-driven progressive disclosure.

```rust
/// A CompositeAgent merges multiple agent capabilities into one.
/// The agent owns a skill library and a skill selector that picks
/// relevant skills per-task from the library.
pub struct CompositeAgent {
    /// Base agent implementation (the LLM backend).
    inner: Box<dyn Agent>,
    /// Compiled skill library — each skill is a (name, schema, prompt_fragment).
    skills: Vec<AgentSkill>,
    /// Selector that picks top-K skills per task, avoiding the phase transition.
    selector: SkillSelector,
    /// Maximum skills to inject per prompt (default: 25, max safe: ~50).
    max_skills_per_prompt: usize,
}

pub struct AgentSkill {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub prompt_fragment: String,
    /// Source agent role this skill was extracted from.
    pub source_role: AgentRole,
}

pub struct SkillSelector {
    /// HDC embeddings for semantic similarity (uses roko-index).
    embeddings: Vec<(String, Vec<f32>)>,
    /// Tool transition graph for predicting likely next skills.
    transition_graph: HashMap<String, Vec<(String, f32)>>,
}

impl SkillSelector {
    /// Select top-K skills for a given task, combining semantic similarity
    /// with transition probability from recently-used skills.
    pub fn select(&self, task: &Signal, recent: &[String], k: usize) -> Vec<&AgentSkill> {
        // 1. Compute semantic similarity between task and all skills
        // 2. Boost skills that are likely next steps (transition graph)
        // 3. Return top-K, capped at max_skills_per_prompt
        todo!()
    }
}
```

### Coordination: Agent Pipelines and Meshes

Five main multi-agent coordination patterns have emerged (2025 consensus):

1. **Orchestrator-Worker** — Central coordinator fans out to agents. Roko's
   current model via `PlanRunner`.
2. **Pipeline** — Sequential stage processing: proposer → coder → reviewer → gater.
3. **Hierarchical** — Tree-structured delegation (maps to Erlang supervision trees).
4. **Swarm** — Decentralized emergent coordination (OpenAI Swarm SDK concept).
5. **Mesh** — Direct peer-to-peer communication between agents.

```rust
/// Agent composition operators — algebraic composition of agents.
/// Two agents compose if the output type of one matches the input
/// type of the other (Signal-typed boundaries).
pub enum AgentComposition {
    /// Sequential: A then B. B receives A's output signal.
    Pipeline(Vec<Box<dyn Agent>>),
    /// Parallel: A and B run concurrently, outputs merged.
    Parallel {
        agents: Vec<Box<dyn Agent>>,
        merge: MergeStrategy,
    },
    /// Conditional: route to A or B based on signal properties.
    Conditional {
        router: Box<dyn Fn(&Signal) -> usize>,
        branches: Vec<Box<dyn Agent>>,
    },
    /// Mixture-of-Agents: layer N takes all outputs from layer N-1.
    /// Wang et al. (2024), "Mixture-of-Agents Enhances LLM Capabilities."
    MixtureOfAgents {
        layers: Vec<Vec<Box<dyn Agent>>>,
        aggregator: Box<dyn Agent>,
    },
}

pub enum MergeStrategy {
    /// Concatenate all outputs.
    Concatenate,
    /// Use a dedicated aggregator agent to synthesize.
    Aggregate(Box<dyn Agent>),
    /// Vote: majority answer wins (for classification tasks).
    MajorityVote,
    /// Best-of-N: run N agents, pick highest-confidence output.
    BestOfN { n: usize },
}
```

### Mixture of Agents (MoA)

Wang et al. (2024, arXiv:2406.04692, ICLR 2025) showed that a layered MoA
architecture — where each layer's agents take all outputs from the previous
layer as auxiliary information — achieves 65.1% on AlpacaEval 2.0 using only
open-source LLMs (vs. 57.5% for GPT-4 Omni). Two roles: **Proposers**
(generate diverse candidates) and **Aggregators** (merge and refine).

---

## Agent Introspection

Can agents inspect their own state, capabilities, and history? Research
distinguishes **engineering introspection** (practical self-inspection) from
**emergent introspection** (the model's internal self-awareness).

### Engineering Introspection

Practical agent self-inspection manifests as five capabilities:

| Capability | Description | Roko support |
|---|---|---|
| **State inspection** | Query own memory, tool history, current task context | EpisodeLogger |
| **Capability assessment** | Report available tools and handleable task types | `ToolRegistry::all()` |
| **Confidence estimation** | Estimate uncertainty about a response or plan | CascadeRouter signals |
| **History review** | Review past actions and learn from mistakes | `.roko/episodes.jsonl` |
| **Failure detection** | Detect loops, low-quality output, resource budget breaches | `roko-conductor` watchers |

```rust
/// AgentIntrospection provides self-inspection capabilities.
/// Injected into agent context so agents can reason about themselves.
pub struct AgentIntrospection {
    /// This agent's role and capabilities.
    pub identity: AgentIdentity,
    /// Recent episode history (last N turns) for self-reflection.
    pub recent_episodes: Vec<EpisodeSummary>,
    /// Current resource consumption (tokens, cost, time).
    pub resource_usage: ResourceUsage,
    /// Confidence estimate from the CascadeRouter for this task.
    pub confidence: f64,
    /// Available tools and their permission status.
    pub available_tools: Vec<ToolSummary>,
}

pub struct AgentIdentity {
    pub role: AgentRole,
    pub model_tier: ModelTier,
    pub temperament: Temperament,
    pub capabilities: Vec<String>,
}

pub struct EpisodeSummary {
    pub task_id: String,
    pub outcome: TaskOutcome,
    pub tools_used: Vec<String>,
    pub tokens_consumed: u64,
    pub gate_results: Vec<(String, bool)>,
    /// Verbal self-reflection (Reflexion pattern, Shinn et al. 2023).
    pub reflection: Option<String>,
}

pub struct ResourceUsage {
    pub tokens_used: u64,
    pub tokens_remaining: u64,
    pub cost_usd: f64,
    pub budget_remaining_usd: f64,
    pub elapsed_ms: u64,
    pub timeout_ms: u64,
}
```

### Metacognitive Monitoring

Agentic metacognition (arXiv:2509.19783, 2025) adds a secondary
"metacognitive" layer that monitors the primary agent for failure signals:
excessive latency, repetitive actions, error patterns. Success rates improved
from 75.78% (baseline) to 83.56% with metacognitive monitoring — a 7.78pp
improvement. This maps directly to Roko's `roko-conductor` watcher/circuit-
breaker pattern.

```rust
/// MetacognitiveMonitor watches an agent for failure signals and
/// can trigger intervention (human handoff, model escalation, task abort).
pub struct MetacognitiveMonitor {
    /// Thresholds for triggering intervention.
    pub config: MetacognitiveConfig,
    /// Rolling window of agent actions for pattern detection.
    action_window: VecDeque<AgentAction>,
}

pub struct MetacognitiveConfig {
    /// Maximum consecutive tool calls without progress (default: 5).
    pub max_stalled_turns: usize,
    /// Maximum time without meaningful output (default: 120s).
    pub max_idle_ms: u64,
    /// Repetition threshold: same tool called N times with similar args (default: 3).
    pub repetition_threshold: usize,
    /// Confidence floor: escalate if confidence drops below this (default: 0.3).
    pub confidence_floor: f64,
}

impl MetacognitiveMonitor {
    /// Check if the agent is exhibiting failure patterns.
    /// Returns an intervention recommendation if so.
    pub fn check(&self, action: &AgentAction) -> Option<Intervention> {
        // 1. Detect stalling (no new tool calls, no output)
        // 2. Detect repetition (same tool, similar args)
        // 3. Detect confidence collapse
        // 4. Detect resource budget exhaustion
        todo!()
    }
}

pub enum Intervention {
    /// Escalate to a higher-tier model.
    EscalateModel(ModelTier),
    /// Request human review before continuing.
    HumanHandoff(String),
    /// Abort the task with a failure reason.
    Abort(String),
    /// Inject a self-reflection prompt to help the agent course-correct.
    InjectReflection(String),
}
```

### Emergent Introspection in LLMs

Anthropic's Transformer Circuits Team (2025) studied emergent introspective
awareness in LLMs using concept injection (activation steering). Key findings:

- Claude Opus 4.1 and Opus 4 performed best, suggesting introspective
  capabilities emerge alongside other model improvements.
- Even the best models achieve only ~20% accuracy on true introspection tasks.
- The simplest explanation is **multiple narrow circuits** that each handle
  specific introspective tasks, not one general-purpose introspection system.

For engineering purposes, this means we should design for **explicit
self-inspection** (giving agents access to their own state via structured
data) rather than relying on the model's emergent self-awareness.

---

## Actor Model Foundations

The Agent trait's design is rooted in the **actor model** (Hewitt et al., 1973),
where an actor is an autonomous process that receives messages, does computation,
sends messages, and creates new actors — with no shared state.

| Actor model concept | Roko equivalent |
|---|---|
| Actor | `Box<dyn Agent>` |
| Message | `Signal` (Engram) |
| Behavior | `AgentRole` + system prompt |
| Supervision tree | `PlanRunner` + `ProcessSupervisor` |
| Let-it-crash | Gate pipeline: fail → retry with fallback model |
| Behavior switching | Agent metamorphosis (role change mid-task) |

### Erlang/OTP Supervision Trees

Erlang's supervision trees provide a hierarchical arrangement of workers
(processes that do computation) and supervisors (processes that monitor
workers). If a worker crashes, the supervisor restarts it. Restart strategies:

- **`one_for_one`** — Restart only the failing child. (Roko: retry single task
  with fallback model.)
- **`one_for_all`** — Restart all children if one fails. (Roko: re-run entire
  plan if critical task fails.)
- **`rest_for_one`** — Restart the failing process and all processes started
  after it. (Roko: re-run downstream DAG tasks when an upstream dependency
  fails.)

```rust
/// Supervision strategy for agent failure recovery.
/// Maps Erlang/OTP restart strategies to Roko's plan execution.
#[derive(Clone, Debug)]
pub enum SupervisionStrategy {
    /// Restart only the failed agent task with a fallback model.
    OneForOne {
        max_restarts: u32,     // Default: 3
        within_ms: u64,        // Default: 300_000 (5 min)
        fallback_tier: Option<ModelTier>,
    },
    /// Re-run all tasks in the plan group if one fails.
    OneForAll {
        max_restarts: u32,     // Default: 1
    },
    /// Re-run the failed task and all downstream dependents in the DAG.
    RestForOne {
        max_restarts: u32,     // Default: 2
    },
}
```

### Capability-Based Security for Agents

Research shows that **capability-based** (OCaps) agent models are strictly more
expressive and more secure than role-based models for dynamic agent tasks:

| Property | RBAC (Roko current) | OCaps (proposed) |
|---|---|---|
| Permission granularity | Coarse (role-level) | Fine (per-object, per-operation) |
| Delegation | Requires admin | Holder can delegate directly |
| Attenuation | Need new restricted role | Native (wrap with restrictions) |
| Dynamic adaptation | Rigid (role reassignment) | Fluid (grant/revoke per-task) |

Tenuo (tenuo.dev, 2025) implements OCaps for AI agents as cryptographic
**warrants** — unforgeable, attenuating capability tokens with ~27μs offline
verification. Each delegation hop can only reduce authority, never expand it.

```rust
/// A capability warrant — an unforgeable, attenuating token of authority.
/// Based on Tenuo's cryptographic warrant model (tenuo.dev, 2025).
pub struct AgentWarrant {
    /// What this warrant authorizes (tool name, path pattern, etc.).
    pub capability: Capability,
    /// Constraints that narrow the capability (path prefix, TTL, etc.).
    pub constraints: Vec<WarrantConstraint>,
    /// Cryptographic chain: each delegation hop is signed.
    pub chain: Vec<DelegationHop>,
    /// Expiration timestamp.
    pub expires_at: SystemTime,
}

pub enum Capability {
    /// Can invoke a specific tool.
    Tool(String),
    /// Can read files matching a glob pattern.
    ReadPath(String),
    /// Can write files matching a glob pattern.
    WritePath(String),
    /// Can execute commands matching a pattern.
    Exec(String),
    /// Can access a network destination.
    Network(String),
}

pub enum WarrantConstraint {
    /// Path must be under this prefix.
    Subpath(PathBuf),
    /// Time-to-live in milliseconds.
    Ttl(u64),
    /// Maximum invocations allowed.
    MaxInvocations(u32),
    /// CEL expression for custom constraints.
    Cel(String),
}
```

---

## Agent Metamorphosis

Can an agent change its role mid-task? MorphAgent (arXiv:2410.15048, 2024)
demonstrates that agents can autonomously adapt their "profile" — a vectorized
representation of expertise and responsibility — via Observe-Think-Act cycles.

```rust
/// Agent metamorphosis — dynamic role switching during task execution.
/// An agent starts with one role but can morph based on task demands.
pub struct MorphableAgent {
    inner: Box<dyn Agent>,
    current_role: AgentRole,
    /// Role profile vector — updated via Observe-Think-Act cycles.
    profile: RoleProfile,
    /// Allowed role transitions (not all morphs are safe).
    allowed_transitions: HashSet<(AgentRole, AgentRole)>,
}

pub struct RoleProfile {
    /// Role Clarity Score — how well-defined the current role is (0.0–1.0).
    pub clarity: f64,
    /// Role Differentiation Score — how distinct from other agents (0.0–1.0).
    pub differentiation: f64,
    /// Task-Role Alignment Score — how well role fits current task (0.0–1.0).
    pub alignment: f64,
}

impl MorphableAgent {
    /// Evaluate whether a role morph is warranted based on task signals.
    pub fn should_morph(&self, task: &Signal) -> Option<AgentRole> {
        // 1. Compute task-role alignment for current role
        // 2. Compute alignment for each allowed transition target
        // 3. If a target role has significantly higher alignment, recommend morph
        // 4. Check allowed_transitions to ensure the morph is permitted
        todo!()
    }

    /// Execute a role morph: swap system prompt, tool permissions, model tier.
    pub fn morph(&mut self, new_role: AgentRole, config: &RokoConfig) {
        self.current_role = new_role;
        // Update system prompt via SystemPromptBuilder
        // Update tool permissions via ToolDispatcher
        // Optionally swap model tier via CascadeRouter
    }
}
```

Safety constraint: morphing should only **expand** capabilities through a
capability warrant chain (OCaps), never bypass the supervision hierarchy.

---

## Citations

1. Sumers, T. R. et al. (2023). "Cognitive Architectures for Language Agents."
   arXiv:2309.02427. — Theoretical basis for separating perception/reasoning
   from action execution.
2. Hewitt, C., Bishop, P., & Steiger, R. (1973). "A Universal Modular ACTOR
   Formalism for Artificial Intelligence." IJCAI. — Actor model foundation.
3. Wang, J. et al. (2024). "Mixture-of-Agents Enhances Large Language Model
   Capabilities." arXiv:2406.04692, ICLR 2025. — MoA layered composition.
4. Anthropic Transformer Circuits Team (2025). "Emergent Introspective
   Awareness in Large Language Models." — ~20% accuracy, narrow circuits.
5. arXiv:2509.19783 (2025). "Agentic Metacognition: Self-Aware Agent for
   Failure Prediction and Human Handoff." — +7.78pp from metacognitive monitoring.
6. arXiv:2410.15048 (2024). "MorphAgent: Self-Evolving Profiles and
   Decentralized Collaboration." — Dynamic role switching.
7. arXiv:2601.04748 (2025). "When Single-Agent with Skills Replace Multi-Agent
   Systems." — 53.7% token reduction, phase transition at 50–100 skills.
8. Murray, T. "Analysing Object-Capability Security." Oxford. — OCaps model.
9. Tenuo (2025). tenuo.dev — Cryptographic capability warrants for AI agents.
10. Shinn, N. et al. (2023). "Reflexion: Language Agents with Verbal
    Reinforcement Learning." NeurIPS 2023. — Self-reflection pattern.
11. Refactoring PRD §01-synapse-architecture — Engram struct and 6 Synapse trait
    definitions.
12. Refactoring PRD §05-agent-types — Agent role compositions and extensibility.
13. `crates/roko-agent/src/agent.rs` — Agent trait and AgentResult source.
14. `crates/roko-cli/src/orchestrate.rs:451` — Primary agent call site.
