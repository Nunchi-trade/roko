# 03 — Graph

> The universal composition primitive. Cells wired by typed edges into a directed acyclic (or cyclic) graph. TOML-defined, serializable, runtime-interpreted. **Hot Graphs** stay resident and re-fire per tick. The **Workflow/Activity split** separates deterministic orchestration from non-deterministic execution for replay.

**Kernel primitives used**: Signal (data on edges), Cell (computation at nodes), Graph (this document), Bus (lifecycle Pulses), Store (run storage, Activity records), Protocol (Score, Verify, Route, Compose, React, Observe — all invocable from Graph nodes).

**Subsumes**: Workflow, Pipeline, DAG, Plan, TickPipeline, AgentLoop, Chain.

**Design invariant**: A Graph IS a Cell (fractal composition). Any Graph can be embedded as a SubGraph node inside another Graph. The Engine does not distinguish between "top-level" and "nested" Graphs.

---

## 1. Graph Struct

A Graph is a composition of Cells connected by typed edges. Graphs are data — defined in TOML, loaded at runtime, validated before execution. The execution engine ([04-EXECUTION](04-EXECUTION.md)) interprets Graphs; it does not compile them.

```rust
/// A Graph is the universal composition primitive.
/// Everything that composes Cells is a Graph: plans, agent pipelines,
/// learning loops, dream cycles, trigger chains.
pub struct Graph {
    /// Stable identity. Content-addressed from (name, version, nodes, edges).
    pub id: GraphId,

    /// Human-readable name.
    pub name: String,

    /// Semantic version.
    pub version: Version,

    /// The nodes in this Graph.
    pub nodes: Vec<Node>,

    /// The edges connecting nodes.
    pub edges: Vec<Edge>,

    /// Entry node(s) — where execution begins.
    pub entry: Vec<NodeId>,

    /// Exit node(s) — where execution ends and output is collected.
    pub exits: Vec<NodeId>,

    /// Combined input schema (union of entry node inputs).
    pub input_schema: Option<TypeSchema>,

    /// Combined output schema (union of exit node outputs).
    pub output_schema: Option<TypeSchema>,

    /// Execution policy: budget, deadline, failure strategy, parallelism.
    pub policy: GraphPolicy,

    /// Metadata for authoring, discovery, and marketplace listing.
    pub metadata: GraphMetadata,
}

/// Content-addressed Graph identity.
/// Hash includes: name, version, sorted node IDs, sorted edge (from, to) pairs,
/// and policy.failure_strategy. Node params and metadata are excluded (config-tier
/// changes do not alter Graph identity).
pub struct GraphId(pub ContentHash);

pub struct GraphMetadata {
    pub author: Author,
    pub description: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### Graph as Cell (fractal composition)

Because Graph implements Cell, any Graph can appear wherever a Cell is expected. The Engine transparently creates a nested Flow with its own RunId. Input Signals are delivered to the Graph's entry nodes; output Signals are collected from exit nodes.

```rust
impl Cell for Graph {
    fn input_schema(&self) -> Option<&TypeSchema> {
        self.input_schema.as_ref()
    }

    fn output_schema(&self) -> Option<&TypeSchema> {
        self.output_schema.as_ref()
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Delegate to Engine: start a nested Flow
        let run_id = ctx.engine().start(self.clone(), input).await?;
        ctx.engine().await_completion(run_id).await
    }
}
```

---

## 2. Nodes

A Node wraps a computation source: a Cell reference, a sub-Graph, a branch, a fan-out/fan-in, a loop, a human input gate, a wait, a slot (late-bound), or a noop (pass-through).

```rust
pub struct Node {
    /// Unique within this Graph.
    pub id: NodeId,

    /// Human-readable label (shown in TUI, logs).
    pub label: String,

    /// What this node does.
    pub kind: NodeKind,

    /// Override the Graph-level failure strategy for this node.
    pub failure_strategy: Option<FailureStrategy>,

    /// Maximum retries before escalating to the Graph-level strategy.
    pub max_retries: Option<u32>,

    /// Timeout override for this node.
    pub timeout: Option<Duration>,

    /// Execution classification for replay (see S6).
    pub execution_class: ExecutionClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeId(pub String);

pub enum NodeKind {
    /// Execute a Cell by reference.
    Cell {
        block_ref: CellRef,
        /// Static parameters passed to the Cell (T1 config tier).
        params: BTreeMap<String, Value>,
    },

    /// Embed another Graph as a sub-computation.
    /// The sub-Graph runs as a nested Flow with its own RunId.
    SubGraph {
        graph_ref: GraphRef,
    },

    /// Conditional branch: evaluate an expression, route to one of
    /// several downstream edges based on the result.
    Branch {
        condition: Expr,
        arms: Vec<BranchArm>,
        default: Option<NodeId>,
    },

    /// Fork execution to multiple downstream nodes in parallel.
    /// All downstream edges from a FanOut node execute concurrently.
    FanOut {
        /// How to split input Signals among downstream nodes.
        split: SplitStrategy,
    },

    /// Join parallel branches. Waits for all (or a quorum of) upstream
    /// nodes to complete before proceeding.
    FanIn {
        /// Merge strategy for combining results.
        merge: MergeStrategy,
        /// Minimum upstream completions required (default: all).
        quorum: Option<usize>,
    },

    /// Feedback loop: re-execute a sub-graph until a condition is met
    /// or a maximum iteration count is reached.
    Loop {
        body: GraphRef,
        condition: Expr,
        max_iterations: u32,
    },

    /// Pause execution until a human provides input.
    /// Publishes a Pulse on `human.input.requested` and waits.
    HumanInput {
        prompt: String,
        input_schema: Option<TypeSchema>,
        timeout: Option<Duration>,
    },

    /// Pause execution for a duration or until a Bus event.
    Wait {
        until: WaitCondition,
    },

    /// Late-bound slot. Filled at runtime by Rack macros or
    /// dynamic Graph construction. Execution fails if unfilled.
    Slot {
        name: String,
        expected_schema: Option<TypeSchema>,
        expected_protocols: Vec<ProtocolId>,
    },

    /// Pass-through. Used for Graph structure (merge points, labels).
    Noop,
}

pub struct BranchArm {
    pub label: String,
    pub pattern: Expr,
    pub target: NodeId,
}

pub enum WaitCondition {
    Duration(Duration),
    BusEvent { filter: TopicFilter },
    Both { duration: Duration, filter: TopicFilter },
}
```

### 2.1 Split Strategies

FanOut nodes distribute input Signals to downstream nodes using one of four strategies.

```rust
pub enum SplitStrategy {
    /// Send all input Signals to every downstream node (broadcast).
    Broadcast,

    /// Send one Signal to each downstream node (round-robin).
    RoundRobin,

    /// Partition Signals by an expression evaluating to a node ID.
    Partition { key: Expr },

    /// Duplicate input, adding an index field.
    Indexed,
}
```

### 2.2 Merge Strategies

FanIn nodes combine outputs from upstream parallel branches.

```rust
pub enum MergeStrategy {
    /// Concatenate all output Signals.
    Concat,

    /// Take the first successful output (race).
    First,

    /// Take the output with the highest Score.relevance.
    BestScore { dimension: ScoreDimension },

    /// Apply a custom Compose Cell to merge outputs.
    Compose { block_ref: CellRef },

    /// Collect into a single array Signal.
    Collect,

    /// Voting: majority result wins (for redundant verification).
    Majority,
}

pub enum ScoreDimension {
    Relevance,
    Quality,
    Confidence,
    Novelty,
    Utility,
}
```

### 2.3 Relationship to Universal Patterns

The four Universal Patterns from the vocabulary manifest as Graph topologies:

| Pattern | Graph topology |
|---|---|
| **Pipeline** | Linear chain of edges: A -> B -> C -> D |
| **Loop** | Feedback edge from a later node to an earlier node (or `Loop` NodeKind) |
| **Functor** | Cross-cut node wired to every node in a subgraph via FanOut/FanIn (e.g., Daimon bias applied to every ASSESS Cell). No dedicated `Functor` NodeKind exists; the pattern is expressed as a SubGraph containing FanOut -> [interceptor Cells] -> FanIn wrapping the target Cell. The interceptor Cells transform Signals before/after the target without changing the Graph's topology. |
| **Space** | A SubGraph with its own Bus+Store partition, isolated from the parent |

---

## 3. Edges

Edges connect nodes with optional conditions and data mappings. Every edge carries Signals — the data flowing through the Graph is always typed Signal values.

```rust
pub struct Edge {
    /// Source node.
    pub from: NodeId,

    /// Target node.
    pub to: NodeId,

    /// Optional condition — edge is traversed only if the expression
    /// evaluates to true. If `None`, the edge is always traversed.
    pub condition: Option<Expr>,

    /// Data mapping: transform output Signals from `from` before
    /// delivering to `to`. If `None`, Signals pass through unchanged.
    pub mapping: Option<Mapping>,

    /// Edge label for debugging and TUI display.
    pub label: Option<String>,

    /// Priority for tie-breaking when multiple edges are eligible.
    /// Lower number = higher priority.
    pub priority: u32,

    /// Edge kind: standard (data flow) or feedback (Hot Graph loop).
    pub kind: EdgeKind,
}

pub enum EdgeKind {
    /// Standard data-flow edge.
    Standard,
    /// Feedback edge: marks this as a Hot Graph loop boundary.
    /// Output is retained in memory and delivered on the next tick.
    Feedback,
}

/// Data mapping between nodes.
pub struct Mapping {
    /// Field-level mappings from source output to target input.
    pub fields: Vec<FieldMapping>,
}

pub struct FieldMapping {
    /// JSONPath-like expression selecting from source output.
    pub from: String,
    /// JSONPath-like expression for target input location.
    pub to: String,
    /// Optional transformation expression.
    pub transform: Option<Expr>,
}
```

### 3.1 Edge Validation via TypeSchema

At Graph load time, the runtime validates that every edge connects compatible TypeSchemas. The source node's output schema must be assignable to the target node's input schema (structural subtyping: every field required by the target must be present in the source with a compatible type).

```
validate_edge(edge):
    source_schema = graph.node(edge.from).output_schema()
    target_schema = graph.node(edge.to).input_schema()

    if edge.mapping is Some:
        // Mapping transforms source -> target: validate the mapping output
        mapped_schema = apply_mapping_schema(source_schema, edge.mapping)
        assert mapped_schema <: target_schema
    else:
        // Direct pass-through: source must be subtype of target
        assert source_schema <: target_schema
```

---

## 4. Expr Language

Edges and branches use a small expression language for conditions and transformations. The language is **total** (no infinite loops), **deterministic** (same inputs -> same outputs), and **bounded** (100ms timeout enforced by the runtime).

```rust
/// A small, total, deterministic expression language.
/// Evaluated by the runtime — not compiled. 100ms timeout.
pub enum Expr {
    // -- Literals ---------------------------------
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),

    // -- References -------------------------------
    /// Reference a field in the current Signal payload.
    /// Uses dot-notation: "payload.status", "score.quality".
    Field(String),

    /// Reference a node output by node ID.
    NodeOutput { node: NodeId, field: String },

    /// Reference a Graph-level variable.
    Var(String),

    // -- Comparison -------------------------------
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    Le(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    Ge(Box<Expr>, Box<Expr>),

    // -- Logic ------------------------------------
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),

    // -- Arithmetic -------------------------------
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),

    // -- Collection -------------------------------
    Contains(Box<Expr>, Box<Expr>),
    Len(Box<Expr>),
    Index(Box<Expr>, Box<Expr>),

    // -- String -----------------------------------
    Matches(Box<Expr>, String),  // regex match
    StartsWith(Box<Expr>, String),
    EndsWith(Box<Expr>, String),

    // -- Conditional ------------------------------
    If {
        cond: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },

    // -- Type -------------------------------------
    KindIs(Kind),
    HasField(String),
}
```

**Field reference namespace**: `Expr::Field` strings resolve in three namespaces:
- `payload.*` -- JSONPath into the Signal's `payload` field (e.g., `payload.status`, `payload.results[0].passed`).
- `meta.*` -- Signal struct-level fields (e.g., `meta.kind`, `meta.score.quality`, `meta.created_at`).
- Bare names (no prefix) -- shorthand for `payload.*` (e.g., `status` is equivalent to `payload.status`).

**Design decisions**:
- **No loops**: The language is total by construction. Iteration is handled by `Loop` nodes.
- **No side effects**: Expressions cannot publish, store, or mutate. They are pure functions of their input.
- **100ms timeout**: Even deeply nested expressions are bounded. The runtime aborts evaluation and returns an error if the timeout fires.
- **Deterministic**: Given the same input Signals and variables, the same expression always produces the same result. This is essential for the Workflow/Activity split (S6).

---

## 5. TOML Authoring

Graphs are authored in TOML and loaded by the runtime. The TOML format is the primary authoring surface for plans, workflows, and agent pipelines.

```toml
# plans/code-review.toml — A code review Graph

[graph]
name = "code-review"
version = "0.1.0"
description = "Review a code change: lint, test, score, compose verdict."

[graph.policy]
max_budget_usd = 0.50
deadline_seconds = 300
failure_strategy = "retry_with_escalation"
max_parallelism = 4

# -- Nodes ---------------------------------------------------

[[nodes]]
id = "fetch-diff"
label = "Fetch diff from PR"
kind = "block"
block = "builtin://github-diff-fetcher"

[[nodes]]
id = "lint"
label = "Run clippy"
kind = "block"
block = "builtin://clippy-gate"

[[nodes]]
id = "test"
label = "Run tests"
kind = "block"
block = "builtin://test-gate"

[[nodes]]
id = "score"
label = "AI code quality score"
kind = "block"
block = "code-reviewer"
[nodes.params]
model = "claude-sonnet-4-20250514"

[[nodes]]
id = "fan-in"
label = "Merge verification results"
kind = "fan_in"
merge = "collect"

[[nodes]]
id = "compose-verdict"
label = "Compose final verdict"
kind = "block"
block = "builtin://verdict-composer"

[[nodes]]
id = "branch-pass"
label = "Pass or fail?"
kind = "branch"
default = "report-fail"

[[nodes.arms]]
label = "all-pass"
pattern = "all(payload.results, |r| r.passed)"
target = "report-pass"

[[nodes]]
id = "report-pass"
label = "Report pass"
kind = "block"
block = "builtin://pr-commenter"
[nodes.params]
template = "pass"

[[nodes]]
id = "report-fail"
label = "Report failure"
kind = "block"
block = "builtin://pr-commenter"
[nodes.params]
template = "fail"

# -- Edges ---------------------------------------------------

[[edges]]
from = "fetch-diff"
to = "lint"

[[edges]]
from = "fetch-diff"
to = "test"

[[edges]]
from = "fetch-diff"
to = "score"

[[edges]]
from = "lint"
to = "fan-in"

[[edges]]
from = "test"
to = "fan-in"

[[edges]]
from = "score"
to = "fan-in"

[[edges]]
from = "fan-in"
to = "compose-verdict"

[[edges]]
from = "compose-verdict"
to = "branch-pass"

# Entry and exit are inferred from topology if not specified.
# Entry: nodes with no incoming edges. Exit: nodes with no outgoing edges.
```

### TOML Node Kinds

| TOML `kind` | Rust `NodeKind` | Required fields |
|---|---|---|
| `"block"` | `Cell` | `block` (CellRef) |
| `"sub_graph"` | `SubGraph` | `graph` (GraphRef) |
| `"branch"` | `Branch` | `condition` or `arms[]` |
| `"fan_out"` | `FanOut` | `split` (optional, default: broadcast) |
| `"fan_in"` | `FanIn` | `merge` (optional, default: concat) |
| `"loop"` | `Loop` | `body` (GraphRef), `condition`, `max_iterations` |
| `"human_input"` | `HumanInput` | `prompt` |
| `"wait"` | `Wait` | `until` (duration or event filter) |
| `"slot"` | `Slot` | `name` |
| `"noop"` | `Noop` | none |

---

## 6. Workflow/Activity Split

Inspired by Temporal (https://temporal.io), the runtime classifies each node as either a **Workflow** node or an **Activity** node. This classification determines replay semantics and is the foundation of snapshot/resume.

```rust
/// Execution classification. Determines replay behavior.
pub enum ExecutionClass {
    /// Deterministic orchestration. Pure Rust, no side effects,
    /// no LLM calls. Replaying Workflow nodes re-executes the code
    /// and produces identical results.
    ///
    /// Examples: Branch, FanOut, FanIn, Noop, Expr evaluation,
    /// data mapping, merge strategy application.
    Workflow,

    /// Non-deterministic execution. LLM calls, shell commands,
    /// network requests, human input. Replaying Activity nodes
    /// returns the recorded output from the first execution.
    ///
    /// Examples: LLM-backed Cells, shell gates, API connectors,
    /// human input, any Cell with side effects.
    Activity,
}
```

### Classification rules

| Node Kind | Default Class | Override? |
|---|---|---|
| `Cell` (LLM call) | Activity | No |
| `Cell` (pure Rust, no I/O) | Workflow | No |
| `Cell` (shell execution) | Activity | No |
| `SubGraph` | Inherits from children | No |
| `Branch` | Workflow | No |
| `FanOut` | Workflow | No |
| `FanIn` | Workflow | No |
| `Loop` (condition eval) | Workflow | No |
| `HumanInput` | Activity | No |
| `Wait` | Activity | No |
| `Slot` | Depends on what fills it | No |
| `Noop` | Workflow | No |

### Replay semantics

When a Flow is resumed from a FlowSnapshot ([04-EXECUTION](04-EXECUTION.md)):

1. **Workflow nodes** are re-executed. Their code is deterministic, so the same inputs produce the same control-flow decisions. This reconstructs the execution DAG.
2. **Activity nodes** are NOT re-executed. Instead, the recorded output from the first execution is loaded from the run storage. This avoids re-running LLM calls, re-executing shell commands, or re-requesting human input.

This split makes resumability safe and predictable. The execution engine records Activity outputs in the run storage directory (`runs/<run-id>/activities/`).

---

## 7. Graph Policy

Every Graph carries a policy governing its execution. The policy is the control surface for budget, deadline, failure handling, concurrency, and Hot Graph behavior.

```rust
pub struct GraphPolicy {
    /// Maximum budget (USD-equivalent) for the entire Graph execution.
    /// Includes all Cell costs, LLM calls, and sub-Graph costs.
    pub max_budget: Option<Cost>,

    /// Maximum wall-clock time for the entire Graph execution.
    pub deadline: Option<Duration>,

    /// Default failure strategy for nodes that don't override it.
    pub failure_strategy: FailureStrategy,

    /// Maximum number of nodes executing concurrently.
    /// Limits resource consumption for fan-out-heavy Graphs.
    pub max_parallelism: Option<usize>,

    /// Whether to snapshot state after each node completion
    /// (enables resumability at the cost of I/O).
    pub snapshot_after_each_node: bool,

    /// Whether this Graph is a Hot Graph (stays resident between firings).
    pub hot: bool,

    /// Clock binding for Hot Graphs (see S8).
    pub clock: Option<ClockBinding>,

    /// Demurrage: whether intermediate Signals produced by this Graph
    /// are subject to demurrage or exempt during execution.
    pub exempt_intermediate_demurrage: bool,
}
```

### Failure strategies

```rust
/// What to do when a node fails.
pub enum FailureStrategy {
    /// Fail the entire Graph immediately.
    Fail,

    /// Retry the failed node up to N times with backoff.
    Retry {
        max_retries: u32,
        backoff: BackoffStrategy,
    },

    /// Retry with a different (more capable) Cell on each retry.
    /// The Route protocol selects alternatives.
    RetryWithEscalation {
        max_retries: u32,
    },

    /// Decompose the failed task into smaller sub-tasks and retry each.
    /// Uses an LLM to generate the decomposition.
    Decompose {
        max_depth: u32,
    },

    /// Skip the failed node and continue execution.
    /// Downstream nodes receive empty input from the skipped node.
    Skip,

    /// Execute a compensation Graph to undo partial work.
    Compensate {
        compensation_graph: GraphRef,
    },

    /// Generate a new plan from the failure context and execute it.
    /// Uses the same plan-generation pipeline as `roko prd plan`.
    Replan {
        max_replans: u32,
    },

    /// Pause and wait for human resolution.
    HumanResolve {
        prompt: String,
        timeout: Option<Duration>,
    },
}

pub enum BackoffStrategy {
    /// Fixed delay between retries.
    Fixed(Duration),
    /// Exponential backoff with jitter.
    Exponential { base: Duration, max: Duration },
    /// Linear increase.
    Linear { step: Duration, max: Duration },
}
```

---

## 8. Hot Graphs

A **Hot Graph** is a Graph that stays resident in memory between firings. Instead of creating a new Flow for each invocation, a Hot Graph retains its state and re-fires on each tick of a bound clock. This is how Agent pipelines work: the Agent's cognitive loop is a Hot Graph that fires every tick.

### Clock bindings

```rust
/// Clock binding for Hot Graphs.
pub enum ClockBinding {
    /// Agent gamma clock (~1-5s). Fast perception/reflex processing.
    AgentGamma,

    /// Agent theta clock (~5-60s). Working memory, attention updates.
    AgentTheta,

    /// Agent delta clock (~120s+). Planning, consolidation.
    AgentDelta,

    /// Adaptive clock: period varies by regime.
    /// Calm = longer intervals, Crisis = shorter intervals.
    Adaptive {
        timescale: AdaptiveTimescale,
    },

    /// Custom clock with explicit period.
    Custom {
        period: Duration,
        name: String,
    },
}

pub enum AdaptiveTimescale {
    /// Gamma: 1-5s (fast perception)
    Gamma,
    /// Theta: 5-60s (reflective)
    Theta,
    /// Delta: 120s+ (consolidation)
    Delta,
}
```

### Hot Graph execution model

1. **Initialization**: The Hot Graph is loaded and its entry nodes are prepared. State slots are initialized.
2. **Tick**: On each clock tick, the entry nodes receive the current state (updated since the last tick) and execute. Execution propagates through the Graph.
3. **State retention**: Between ticks, node outputs are retained in memory. The next tick's entry nodes receive the previous tick's exit node outputs (plus any new external input).
4. **Teardown**: When the owning Agent enters the Terminal lifecycle state, the Hot Graph flushes state to Store and shuts down.

### The feedback edge

Hot Graphs are distinguished from standard Graphs by the presence of a **feedback edge** — an edge from a later node back to an earlier node with `kind = "feedback"`. This edge is not traversed within a single tick. Instead, the output carried by the feedback edge is retained in memory and delivered to the target node on the next tick.

```toml
# Feedback edge: react output feeds next tick's sense
[[graph.edges]]
from = "react"
to = "sense"
kind = "feedback"    # Marks this as a Hot Graph feedback loop
```

### Agent's cognitive loop as a Hot Graph

The Agent's core processing pipeline is defined as a Hot Graph bound to an adaptive clock. This is the 7-step cognitive loop (Sense, Assess, Compose, Act, Verify, Persist/Broadcast, React) expressed as a concrete Graph.

```toml
[graph]
name = "cognitive-loop"
version = "1.0.0"
hot = true
clock = { kind = "adaptive", timescale = "theta" }

[graph.policy]
max_parallelism = 1                  # Sequential pipeline (no parallelism within a tick)
failure_strategy = "retry_with_escalation"
snapshot_interval_secs = 300         # Checkpoint every 5 minutes
budget_scope = "agent"               # Draws from Agent budget

# -- Nodes --

[[graph.nodes]]
id = "sense"
cell = "roko.cognitive.sense"
execution_class = "workflow"

[[graph.nodes]]
id = "assess"
cell = "roko.cognitive.assess"
execution_class = "workflow"

[[graph.nodes]]
id = "compose"
cell = "roko.cognitive.compose"
execution_class = "workflow"

[[graph.nodes]]
id = "act"
cell = "roko.cognitive.act"
execution_class = "activity"         # LLM call: non-deterministic

[[graph.nodes]]
id = "verify"
cell = "roko.cognitive.verify"
execution_class = "activity"         # External gate processes

[[graph.nodes]]
id = "persist_broadcast"
cell = "roko.cognitive.persist_broadcast"
execution_class = "activity"         # Side effects: store + bus

[[graph.nodes]]
id = "react"
cell = "roko.cognitive.react"
execution_class = "workflow"

# -- Edges (sequential pipeline) --

[[graph.edges]]
from = "sense"
to = "assess"

[[graph.edges]]
from = "assess"
to = "compose"

[[graph.edges]]
from = "compose"
to = "act"

[[graph.edges]]
from = "act"
to = "verify"

[[graph.edges]]
from = "verify"
to = "persist_broadcast"

[[graph.edges]]
from = "persist_broadcast"
to = "react"

# -- Feedback edge (react output feeds next tick's sense) --

[[graph.edges]]
from = "react"
to = "sense"
kind = "feedback"
```

### T0 short-circuit

Most ticks (~80%) short-circuit at ASSESS. When the ASSESS Cell determines that all T0 probes report "no change" and EFE selects T0 (zero-cost reflex), the remaining Cells do not execute. This is expressed as conditional edges:

```toml
# Replace the standard assess->compose edge with conditional edges:

[[graph.edges]]
from = "assess"
to = "compose"
condition = "assessment.tier != T0 || assessment.has_selected"

[[graph.edges]]
from = "assess"
to = "react"
condition = "assessment.tier == T0 && !assessment.has_selected"
label = "T0 short-circuit"
```

When the T0 short-circuit fires, REACT still runs (to update counters, check deadlines, emit heartbeats) but ACT/VERIFY/PERSIST are skipped entirely. Cost: $0.

### Composing timescales: nested Hot Graphs

The Agent has three concurrent timescales (gamma, theta, delta), each running the same 7-step loop at different speeds. These are three Hot Graphs sharing the same CorticalState:

```toml
# Gamma loop: fast perception (~5-15s)
[graph]
name = "cognitive-loop-gamma"
hot = true
clock = { kind = "adaptive", timescale = "gamma" }

# Theta loop: reflective (~75s)
[graph]
name = "cognitive-loop-theta"
hot = true
clock = { kind = "adaptive", timescale = "theta" }

# Delta loop: consolidation (~hours)
[graph]
name = "cognitive-loop-delta"
hot = true
clock = { kind = "adaptive", timescale = "delta" }
```

Each loop is an independent Hot Flow managed by the Engine. They share the same CorticalState (atomic reads), Store (serialized writes), Bus (Pulses from one loop visible to others), and budget (VitalityTracker with atomic accounting). They do NOT share node outputs, clock timing, or active Cell instances.

Inter-loop communication happens through Bus Pulses and CorticalState:

```rust
// Gamma loop reads theta's replan signal via Bus
fn sense_bus_drain(bus: &dyn Bus, last_seq: u64) -> Vec<Pulse> {
    let pulses = bus.drain_since(last_seq);
    pulses.into_iter()
        .filter(|p| p.topic.matches("agent:*.theta.replanned")
                  || p.topic.matches("agent:*.delta.consolidated"))
        .collect()
}
```

### Nesting hierarchy

```
Agent
  |
  +-- Gamma Hot Graph (fires every ~10s)
  |     |
  |     +-- [T0 short-circuit most ticks]
  |     +-- [T1/T2 ACT Cell: spawns task sub-Graphs]
  |
  +-- Theta Hot Graph (fires every ~75s)
  |     |
  |     +-- ACT Cell: spawns replan sub-Graph
  |           |
  |           +-- Standard Flow: analyze_progress -> replan -> validate
  |
  +-- Delta Hot Graph (fires every ~hours)
        |
        +-- ACT Cell: spawns dream consolidation sub-Graph
              |
              +-- Standard Flow: nrem_replay -> rem_imagination -> integration
```

---

## 9. Graph Validation

Before execution, the runtime validates a Graph. Validation catches structural errors at load time rather than runtime.

```rust
pub struct ValidationReport {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

pub enum ValidationError {
    /// A node references a Cell that is not registered.
    UnknownCell { node: NodeId, block_ref: CellRef },

    /// An edge connects incompatible TypeSchemas.
    TypeMismatch { edge: (NodeId, NodeId), output: TypeSchema, input: TypeSchema },

    /// The Graph has no entry nodes (no nodes without incoming edges).
    NoEntry,

    /// The Graph has no exit nodes (no nodes without outgoing edges,
    /// excluding loops).
    NoExit,

    /// A Slot node is unfilled.
    UnfilledSlot { node: NodeId, slot_name: String },

    /// A cycle exists outside of a Loop node.
    UnexpectedCycle { nodes: Vec<NodeId> },

    /// A required capability is missing from the effective capability set.
    MissingCapability { node: NodeId, capability: Capability },

    /// Budget is statically insufficient for estimated costs.
    InsufficientBudget { estimated: Cost, budgeted: Cost },

    /// FanIn quorum exceeds number of upstream nodes.
    InvalidQuorum { node: NodeId, quorum: usize, upstream: usize },
}

pub enum ValidationWarning {
    /// A node has no downstream edges (dead end, but not an exit node).
    DeadEnd { node: NodeId },

    /// Estimated cost is close to budget limit (>80%).
    NearBudget { estimated: Cost, budgeted: Cost },

    /// A node has no timeout and no Graph-level deadline.
    Unbounded { node: NodeId },

    /// An Activity node has no snapshot policy.
    ActivityWithoutSnapshot { node: NodeId },
}
```

### Validation procedure

```
validate(graph):
    1. Structural checks:
       - Entry nodes exist (infer from topology if not declared)
       - Exit nodes exist (infer from topology if not declared)
       - All node IDs are unique
       - All edge endpoints reference existing nodes

    2. Type checks:
       - For each edge: validate TypeSchema compatibility (S3.1)
       - For each Mapping: validate that field references resolve

    3. Reference checks:
       - For each Cell node: verify CellRef resolves in the registry
       - For each SubGraph node: verify GraphRef resolves
       - For each Slot node: check if filled

    4. Cycle detection:
       - Detect cycles via topological sort
       - Cycles inside Loop nodes are expected; all others are errors
       - Feedback edges (Hot Graph) are excluded from cycle detection

    5. Budget estimation:
       - Sum estimated costs along the critical path
       - Compare against max_budget; warn if >80%, error if exceeds

    6. Capability checks:
       - For each Cell node: verify its required capabilities are
         available in the owning Space's capability set

    7. Quorum checks:
       - For each FanIn node: verify quorum <= upstream count
```

---

## 10. Run Storage Layout

Each Graph execution (Flow) stores its artifacts in a structured directory under `.roko/runs/`. This layout supports resumability, audit, and post-mortem analysis.

```
.roko/runs/<run-id>/
+-- graph.toml              # Frozen copy of the Graph definition
+-- state.json              # Current execution state (for resumability)
+-- activities/             # Recorded Activity outputs (for replay)
|   +-- <node-id>.json      # Output Signals from each Activity node
|   +-- ...
+-- events.jsonl            # Graduated Pulse snapshots (append-only)
+-- artifacts/              # Files produced by Cell execution
|   +-- <node-id>/
|   |   +-- output.json     # Structured output
|   |   +-- files/          # Generated files
|   +-- ...
+-- snapshots/              # Per-node state snapshots (if enabled)
|   +-- <node-id>.snap.json
|   +-- ...
+-- cost.json               # Cost accounting: per-node, per-Cell, total
+-- verdict.json            # Final verdict (if the Graph includes Verify)
```

### FlowSnapshot format

The FlowSnapshot is the serializable execution state that enables resumability. Written after each node (if configured), periodically, or on graceful shutdown.

```rust
/// Serializable execution state for resumability.
pub struct FlowSnapshot {
    pub run_id: RunId,
    pub graph_id: GraphId,
    pub graph_version: Version,
    pub created_at: DateTime<Utc>,
    pub snapshot_at: DateTime<Utc>,

    /// Per-node execution status.
    pub node_states: BTreeMap<NodeId, NodeState>,

    /// Graph-level variables (set by Expr evaluations).
    pub variables: BTreeMap<String, Value>,

    /// Total cost so far.
    pub cost_so_far: Cost,

    /// Total wall-clock time so far.
    pub elapsed: Duration,

    /// Budget remaining at snapshot time.
    pub budget_remaining: Cost,

    /// References to Activity records for replay.
    pub activity_records: Vec<ContentHash>,
}

pub enum NodeState {
    Pending,
    Running { started_at: DateTime<Utc> },
    Completed {
        output: Vec<Signal>,
        duration: Duration,
        cost: Cost,
    },
    Failed {
        error: String,
        attempts: u32,
    },
    Skipped { reason: String },
    Cancelled,
}
```

### Hot Graph snapshot extension

Hot Graphs extend the base FlowSnapshot with tick-specific state:

```rust
struct CognitiveLoopSnapshot {
    // Standard FlowSnapshot fields
    run_id: RunId,
    graph_id: GraphId,
    snapshot_at: DateTime<Utc>,

    // Hot Graph-specific: state retained between ticks
    tick_count: u64,
    last_react_output: Vec<Signal>,
    cortical_snapshot: CorticalSnapshot,

    // Per-node states for the current tick
    node_states: BTreeMap<NodeId, NodeState>,

    // Activity records for replay
    activity_records: Vec<ContentHash>,

    // Budget at snapshot time
    budget_remaining: Cost,
    demurrage_accumulated: Cost,
}
```

---

## 11. GraphRef and Graph Composition

Graphs can be referenced and embedded inside other Graphs via `SubGraph` nodes. This enables hierarchical composition — the fractal property.

```rust
/// Reference to a Graph for embedding or invocation.
pub enum GraphRef {
    /// A Graph defined in the same workspace.
    Local { path: PathBuf },

    /// A Graph from the marketplace.
    Published { id: GraphId, version: Version },

    /// A built-in Graph shipped with Roko.
    Builtin { name: String },

    /// An inline Graph defined within the parent Graph's TOML.
    Inline(Box<Graph>),
}
```

```toml
# Embedding a sub-graph
[[nodes]]
id = "verification-suite"
label = "Run the full verification suite"
kind = "sub_graph"
graph = "plans/verify-suite.toml"
```

### Merge queue for parallel branches

When a Graph uses FanOut to spawn parallel worktree-isolated branches (e.g., multiple agents writing code simultaneously), the merge queue ensures that branches are integrated in a consistent order.

The merge queue is itself a Cell (implementing the Compose protocol) that:

1. Collects outputs from all parallel branches at a FanIn node.
2. Orders them by priority (edge priority) or completion order.
3. Attempts to merge each branch sequentially (analogous to a git merge queue).
4. If a merge conflict is detected, routes to a resolution strategy (retry, decompose, or human resolve).

```rust
pub struct MergeQueueCell {
    /// Strategy for ordering branches.
    pub ordering: MergeOrdering,
    /// Maximum concurrent merge attempts.
    pub max_concurrent: usize,
}

pub enum MergeOrdering {
    /// Merge in edge priority order (lowest priority number first).
    Priority,
    /// Merge in completion order (first finished = first merged).
    CompletionOrder,
    /// Merge by estimated conflict risk (lowest risk first).
    ConflictRisk,
}
```

---

## 12. Worktree Isolation

For plan execution involving code modifications, the Graph runtime supports worktree isolation per task or per parallel branch. Each FanOut branch can execute in its own git worktree, preventing concurrent agents from interfering with each other's changes.

```rust
pub struct WorktreePolicy {
    /// Whether to create isolated worktrees for parallel branches.
    pub isolate_branches: bool,
    /// Base directory for worktrees.
    pub worktree_dir: PathBuf,
    /// Cleanup strategy after branch completion.
    pub cleanup: WorktreeCleanup,
}

pub enum WorktreeCleanup {
    /// Remove worktree after successful merge.
    OnSuccess,
    /// Keep worktree for inspection (user must clean up).
    Keep,
    /// Remove worktree after a timeout, regardless of outcome.
    Timeout(Duration),
}
```

The worktree lifecycle for a parallel plan execution:

```
1. FanOut node creates worktrees for each branch
2. Each branch's Cell executes in its isolated worktree
3. FanIn node triggers merge queue
4. Merge queue integrates branches one at a time
5. Cleanup per WorktreePolicy
```

---

## 13. Graph as Data, Not Code

Graphs are serializable data structures, not code. This enables:

1. **TOML authoring** by domain experts (no Rust required).
2. **Agent-generated plans**: `roko prd plan <slug>` produces a Graph (as `tasks.toml`).
3. **Marketplace sharing**: Graphs are portable artifacts with declared schemas.
4. **Versioning**: Graphs are content-addressed. A frozen copy is stored with each run.
5. **Validation**: The runtime validates before executing. Type mismatches, missing Cells, and budget insufficiency are caught at load time.
6. **Visualization**: The TUI renders Graphs as node diagrams. The Generative Canvas ([20-SURFACES](20-SURFACES.md)) enables visual editing.
7. **Evolution**: L4 evolution can propose Graph mutations (add/remove nodes, change edges, adjust policies). Human review required for safety-critical Graphs.

---

## 14. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| 1 | `Graph` struct compiles with `nodes`, `edges`, `entry`, `exits`, `policy` | Compile check |
| 2 | All 10 `NodeKind` variants compile | Compile check |
| 3 | `Edge` struct compiles with `condition: Option<Expr>` and `mapping: Option<Mapping>` | Compile check |
| 4 | `Expr` language is total: no construct can loop | Review: no recursion-enabling constructors |
| 5 | `Expr` evaluation respects 100ms timeout | Unit test with deeply nested expression |
| 6 | `Expr` is deterministic: same inputs -> same outputs | Unit test with random seed comparison |
| 7 | TOML Graph loads and validates successfully | Integration test with `code-review.toml` example |
| 8 | TypeSchema mismatch on edge detected at validation time | Unit test: connect `Kind::Diff` output to `Kind::Json` input |
| 9 | Unfilled Slot detected at validation time | Unit test |
| 10 | Unexpected cycle (outside Loop node) detected at validation time | Unit test |
| 11 | SubGraph nodes create nested Flows with separate RunIds | Integration test |
| 12 | Hot Graph retains state between ticks | Integration test: two ticks, second tick sees first tick's output |
| 13 | `ClockBinding` variants bind to correct periods | Unit test |
| 14 | Agent's 7-step cognitive loop loads as a valid Hot Graph | Integration test |
| 15 | Workflow/Activity classification is correct for all NodeKinds | Unit test per NodeKind |
| 16 | Activity replay returns recorded output, not re-execution | Integration test: resume Flow, verify no LLM call |
| 17 | Workflow replay re-executes and produces same control flow | Integration test: resume Flow, verify same branch taken |
| 18 | `FlowSnapshot` serializes and deserializes round-trip | Unit test |
| 19 | Run storage layout created on Flow start | Integration test: check directory structure |
| 20 | `MergeStrategy::Majority` correctly selects majority result | Unit test with 3 inputs (2 agree, 1 disagrees) |
| 21 | `FailureStrategy::RetryWithEscalation` uses Route to select alternative | Integration test |
| 22 | `FailureStrategy::Replan` generates new plan from failure context | Integration test |
| 23 | Graph validation produces both errors and warnings | Unit test with problematic Graph |
| 24 | `GraphRef::Inline` allows inline sub-graphs in TOML | Integration test |
| 25 | Budget enforcement: execution halts when budget exceeded | Integration test |
| 26 | Parallelism limit respected in FanOut execution | Integration test with `max_parallelism = 2` and 4 branches |
| 27 | T0 short-circuit skips ACT/VERIFY/PERSIST when ASSESS returns noop | Integration test: verify only SENSE, ASSESS, REACT execute |
| 28 | Feedback edge retained across ticks (not traversed within tick) | Integration test: two ticks, verify feedback edge delivery |
| 29 | Merge queue integrates parallel branches in priority order | Integration test with 3 branches |
| 30 | Graph implements Cell (fractal composition) | Integration test: embed Graph as SubGraph in another Graph |

---

## 15. Citations

| Claim | Source |
|---|---|
| Workflow/Activity split for deterministic replay | Temporal.io execution model (https://temporal.io) |
| VCG auction for context assembly in Compose Cell | Vickrey (1961), Clarke (1971), Groves (1973) |
| EFE (Expected Free Energy) for tier selection in Route protocol | Friston (2006) |
| Content-addressed Graph identity (ContentHash) | Merkle (1979); CAS used in git, IPFS |
| Total expression language (no loops, bounded evaluation) | Coquand & Huet (1988); Total functional programming |
| BFT consensus gates for Byzantine Cell defense | Lamport, Shostak, Pease (1982) |
| Adaptive clock binding (gamma/theta/delta timescales) | Neuroscience: gamma (30-100Hz), theta (4-8Hz), delta (0.5-4Hz) brain oscillation bands, adapted to agent timescales |
