# 04 — Execution Engine

> A single runtime for ALL Graphs. Manages Flow lifecycle, Hot Graph ticking, deterministic replay via the Workflow/Activity split, failure strategies, resumability, budget enforcement, and concurrency. Every lifecycle event is a Pulse on Bus. The cognitive loop is a 7-Cell Hot Graph with T0 short-circuit handling ~80% of ticks at zero cost.

**Kernel primitives used**: Signal (data between Cells), Cell (computation), Graph (composition being executed), Bus (lifecycle Pulses, inter-loop communication), Store (Activity records, snapshots, episodes), Protocol (all 9 — Score, Verify, Route, Compose, React, Observe, Store, Connect, Trigger — invoked from loop Cells).

**Subsumes**: PlanRunner, DAG executor, orchestrate.rs, ProcessSupervisor.

---

## 1. Engine Overview

The execution engine is the single runtime that interprets all Graphs. Plans, agent pipelines, learning loops, dream cycles, trigger chains, and verification suites are all Graphs executed by the same engine. There is no separate executor per specialization.

```rust
/// The execution engine. Interprets Graphs, manages Flows, enforces
/// budgets, handles failures, and publishes lifecycle Pulses.
pub struct Engine {
    /// Cell registry: all registered Cells available for execution.
    pub registry: Arc<CellRegistry>,

    /// Bus for lifecycle Pulses.
    pub bus: Arc<dyn Bus>,

    /// Store for Signal persistence.
    pub store: Arc<dyn Store>,

    /// Active Flows (both standard and Hot).
    pub flows: DashMap<RunId, FlowHandle>,

    /// Global budget tracker.
    pub budget: Arc<BudgetTracker>,

    /// Concurrency limiter.
    pub semaphore: Arc<tokio::sync::Semaphore>,

    /// Cancellation token for engine-wide shutdown.
    pub cancel: CancellationToken,
}

pub struct FlowHandle {
    pub flow: Arc<RwLock<Flow>>,
    pub join_handle: JoinHandle<Result<Vec<Signal>, CellError>>,
    pub cancel: CancellationToken,
}
```

### Engine API

```rust
impl Engine {
    /// Start a new Flow from a Graph definition.
    pub async fn start(&self, graph: Graph, input: Vec<Signal>) -> Result<RunId>;

    /// Resume a Flow from a snapshot.
    pub async fn resume(&self, snapshot: FlowSnapshot) -> Result<RunId>;

    /// Cancel a running Flow.
    pub async fn cancel(&self, run_id: &RunId, reason: &str) -> Result<()>;

    /// Pause a running Flow (Hot Flows only — standard Flows cannot pause mid-node).
    pub async fn pause(&self, run_id: &RunId, reason: &str) -> Result<()>;

    /// Get the current state of a Flow.
    pub async fn status(&self, run_id: &RunId) -> Result<FlowStatus>;

    /// List all active Flows.
    pub async fn list_active(&self) -> Vec<(RunId, FlowStatus)>;

    /// Register a Hot Graph. Starts its clock-driven execution loop.
    pub async fn register_hot(&self, graph: Graph, initial_state: Vec<Signal>) -> Result<RunId>;

    /// Deregister a Hot Graph. Flushes state and stops ticking.
    pub async fn deregister_hot(&self, run_id: &RunId) -> Result<Vec<Signal>>;

    /// Estimate cost and time for a Graph without executing it.
    pub fn estimate(&self, graph: &Graph) -> GraphEstimate;
}

pub struct FlowStatus {
    pub run_id: RunId,
    pub graph_name: String,
    pub state: FlowState,
    pub nodes_completed: usize,
    pub nodes_total: usize,
    pub cost_so_far: Cost,
    pub elapsed: Duration,
    pub current_nodes: Vec<NodeId>,
}
```

---

## 2. Flow Lifecycle

Every Graph execution follows a standard lifecycle, published as Pulses on Bus.

```
Created --> Running --> Completed
                |
                +---> Failed
                |
                +---> Cancelled
                |
                +---> Paused --> Running (resume)
```

```rust
pub enum FlowState {
    /// Flow has been created but execution has not started.
    Created,

    /// Flow is actively executing nodes.
    Running,

    /// Flow execution is paused (can be resumed).
    /// Only Hot Flows and human-in-loop Flows can pause.
    Paused { reason: String },

    /// All exit nodes completed successfully.
    Completed { outputs: Vec<Signal> },

    /// A node failed and the failure strategy could not recover.
    Failed { error: CellError, failed_node: NodeId },

    /// Flow was cancelled by external request.
    Cancelled { reason: String, cancelled_by: Option<AgentId> },
}
```

### Lifecycle Pulses

Every state transition publishes a Pulse on Bus. These Pulses are the sole source of execution observability — there is no separate monitoring channel.

| Transition | Topic | Graduates? | Body |
|---|---|---|---|
| Created | `flow.{run_id}.created` | Yes | Graph name, input count, policy |
| Running | `flow.{run_id}.started` | Yes | Start timestamp |
| Node started | `flow.{run_id}.node.{node_id}.started` | No | Node label |
| Node completed | `flow.{run_id}.node.{node_id}.completed` | Yes (batch) | Output count, cost, duration |
| Node failed | `flow.{run_id}.node.{node_id}.failed` | Yes | Error, attempt count |
| Node retrying | `flow.{run_id}.node.{node_id}.retrying` | No | Attempt number, strategy |
| Paused | `flow.{run_id}.paused` | Yes | Reason |
| Resumed | `flow.{run_id}.resumed` | Yes | Snapshot reference |
| Completed | `flow.{run_id}.completed` | Yes | Output refs, total cost, duration |
| Failed | `flow.{run_id}.failed` | Yes | Error, failed node |
| Cancelled | `flow.{run_id}.cancelled` | Yes | Reason, cancelled by |
| Budget warning | `flow.{run_id}.budget.warning` | Yes | Spent, remaining, threshold |
| Budget exhausted | `flow.{run_id}.budget.exhausted` | Yes | Spent, limit |
| Snapshot taken | `flow.{run_id}.snapshot` | No | Snapshot path |

---

## 3. Node Execution

The engine executes nodes in topological order, respecting edge conditions and the Graph's parallelism limits.

### Execution loop (per node)

```
1. Check preconditions:
   - All upstream nodes completed (or quorum met for FanIn)
   - Edge conditions evaluate to true
   - Budget sufficient for estimated cost
   - Cancellation token not triggered

2. Check execution class (Workflow/Activity split):
   - If Activity AND resuming from snapshot AND output exists:
     --> Return recorded output (no re-execution)
   - Otherwise: proceed to step 3

3. Apply input mappings:
   - Transform upstream outputs via edge Mapping expressions

4. Pre-verify (if Verify protocol configured for this scope):
   - Call verify_pre() with input Signals and action plan
   - If hard_pass = false: skip node (PreVerifyVeto error)

5. Publish prediction (predict-publish-correct):
   - Cell publishes prediction Pulse on prediction.{block_id}

6. Execute the Cell:
   - Pass input Signals + CellContext to block.execute()
   - CellContext.cortical is Some(Arc<CorticalState>) inside Agent Hot Graphs, None for standalone Flows
   - Start verify_stream() polling if configured
   - Enforce node-level timeout
   - Track cost via BudgetTracker

7. Post-verify (if Verify protocol configured):
   - Call verify_post() with input + output Signals
   - Record Verdict (reward, criteria, evidence)

8. Record output:
   - If Activity: write output to runs/<run-id>/activities/<node-id>.json
   - Publish completion Pulse
   - Snapshot state (if policy.snapshot_after_each_node)

9. Propagate:
   - Evaluate downstream edge conditions
   - Enqueue eligible downstream nodes
```

### Parallel execution

Nodes with satisfied preconditions and no mutual ordering constraints execute concurrently, up to `policy.max_parallelism`. FanOut nodes explicitly fork execution. The engine uses a Tokio task-per-node model with a shared semaphore for concurrency limiting.

```rust
pub struct ConcurrencyController {
    /// Maximum concurrent node executions.
    pub semaphore: Arc<tokio::sync::Semaphore>,

    /// Currently executing nodes.
    pub active: DashMap<NodeId, JoinHandle<Result<Vec<Signal>, CellError>>>,
}
```

---

## 4. The Cognitive Loop as a 7-Cell Graph

The 7-step cognitive loop (SENSE, ASSESS, COMPOSE, ACT, VERIFY, PERSIST/BROADCAST, REACT) is not a metaphor or "conceptual mapping." Each step is a Cell with typed I/O, the transitions between steps are edges with conditions, and the Engine that executes task plans is the same Engine that fires the loop. One runtime. One execution model.

If the loop were special-cased, every improvement to the execution engine (retry strategies, snapshot resumability, budget enforcement, failure decomposition) would need separate implementation for the cognitive pipeline. By making the loop a Graph, those capabilities are inherited for free.

Each step is a Cell with typed I/O, capabilities, and protocol conformance. The table below summarizes; full implementations follow for the two load-bearing Cells (ASSESS and ACT).

| Cell | Input | Output | Protocols | Exec Class | Key Operation |
|---|---|---|---|---|---|
| **SENSE** | CorticalSnapshot | SensedMaterial (signals, pulses, external) | Observe | Workflow | Store query + Bus drain + external I/O poll |
| **ASSESS** | SensedMaterial | Assessment (selected, route, tier) | Score, Route | Workflow | Score 5 axes + Daimon bias + Route decision |
| **COMPOSE** | Assessment + CorticalSnapshot | ComposedPrompt (sections, token_count, cost_estimate) | Compose | Workflow | VCG auction (8+ bidders) for token budget |
| **ACT** | ComposedPrompt + RouteDecision | ActionResult (response, pulses, cost) | -- | **Activity** | LLM dispatch / tool execution |
| **VERIFY** | ActionResult | VerifyResult (verdict, evidence) | Verify | **Activity** | Gate pipeline; halt early on hard failure |
| **PERSIST/BROADCAST** | ActionResult + VerifyResult | PersistResult (stored_refs, published_topics) | Store | **Activity** | Write to Store + publish Pulses on Bus |
| **REACT** | PersistResult + VerifyResult + CorticalSnapshot | ReactOutput (signals, pulses) | React | Workflow | Episode consolidation, circuit-breaking, routing feedback |

### ASSESS Cell (the routing decision point)

```rust
struct AssessCell {
    scorer: Arc<dyn ScoreProtocol>,
    router: Arc<dyn RouteProtocol>,
    daimon_bias: Option<Arc<dyn DaimonBias>>,
}

impl Cell for AssessCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let material = SensedMaterial::from_signals(&input)?;

        // Score all candidates across 5 axes, with optional Daimon bias
        let mut scored: Vec<(Signal, f32)> = Vec::new();
        for sig in material.all_signals() {
            let score = self.scorer.score(&sig, ctx)?;
            let biased = match &self.daimon_bias {
                Some(d) => d.bias_score(score, ctx.cortical()),
                None => score,
            };
            scored.push((sig, biased.effective()));
        }

        let decision = self.router.route(&scored, ctx)?;
        Ok(Assessment {
            selected: decision.selected,
            route: decision,
            tier: decision.tier,
        }.into_signals())
    }
}
```

### ACT Cell (the only external provider call)

```rust
struct ActCell {
    dispatcher: Arc<dyn AgentDispatcher>,
}

impl Cell for ActCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let prompt = ComposedPrompt::from_signals(&input)?;
        let route = RouteDecision::from_signals(&input)?;

        let result = self.dispatcher.dispatch(&prompt, &route, ctx.cancel()).await?;

        // Publish live Pulses for stream observers
        for pulse in &result.pulses {
            ctx.bus().publish(pulse.clone()).await?;
        }
        Ok(result.into_signals())
    }
}
```

**ACT is the load-bearing distinction in the Workflow/Activity split.** On replay, the recorded output is returned without re-executing the LLM call. All other Workflow Cells re-execute deterministically.

---

## 5. T0 Short-Circuit

Most ticks (~80%) short-circuit at ASSESS. When the ASSESS Cell determines that all 16 T0 probes report "no change" and EFE selects T0 (zero-cost reflex), the remaining Cells do not execute.

```rust
/// The three cognitive tiers. Canonical definition; re-exported by 05-AGENT.
/// EFE thresholds determine tier selection (see select_tier in 05-AGENT S9).
pub enum CognitiveTier {
    /// T0: Pure Rust pattern matching. No LLM call. ~80% of ticks. EFE cost = $0.
    T0Reflex,
    /// T1: Lightweight model (Haiku-class). EFE cost threshold < 0.01 USD.
    T1Fast,
    /// T2: Full model (Opus-class). EFE cost threshold >= 0.01 USD.
    T2Capable,
}
```

```rust
// Inside AssessCell::execute, the T0 short-circuit:
if decision.tier == CognitiveTier::T0Reflex && decision.selected.is_empty() {
    // Nothing interesting. Skip ACT/VERIFY/PERSIST/REACT.
    return Ok(Assessment::noop().into_signals());
}
```

This short-circuit is expressed as conditional edges in the Graph (see [03-GRAPH](03-GRAPH.md) S8). When the T0 short-circuit fires, REACT still runs (to update counters, check deadlines, emit heartbeats) but ACT/VERIFY/PERSIST are skipped entirely. Cost: $0.

---

## 6. Nested Loops: Gamma, Theta, Delta

The Agent has three concurrent timescales, each running the same 7-step loop at different speeds as independent Hot Graphs sharing the same CorticalState (see [03-GRAPH](03-GRAPH.md) S8 for the TOML definitions and nesting hierarchy).

| Timescale | Period | Purpose | Typical T0 rate |
|---|---|---|---|
| Gamma | 1-5s | Fast perception, reflex processing | ~95% |
| Theta | 5-60s | Working memory, attention updates, replanning | ~70% |
| Delta | 120s+ | Consolidation, dream cycles, long-term learning | ~50% |

A Theta tick can itself spawn a sub-Graph (a plan for re-planning). That sub-Graph is a standard Flow managed by the same Engine. The Theta loop's ACT Cell submits the sub-Graph via `Engine::start()` and awaits its completion. This is Graph nesting: a Hot Graph's Cell spawns a standard Graph as a child.

---

## 7. Hot Graph Execution

Hot Graphs ([03-GRAPH](03-GRAPH.md) S8) stay resident in memory and re-fire on each tick of their bound clock. The engine manages Hot Graphs as long-lived tasks.

### Tick loop

```
1. Wait for next clock tick (or external trigger)

2. Prepare entry node inputs:
   - Previous tick's exit outputs (retained in memory)
   - New external inputs since last tick
   - Updated CorticalState (if Agent-owned)

3. Execute the Graph (same node execution loop as standard Flows)

4. Retain exit node outputs for next tick

5. Optionally checkpoint state to disk (periodic, not every tick)

6. Publish tick Pulse on agent:{id}.tick.completed
```

### State retention

Between ticks, the Hot Flow retains:
- All node outputs from the previous tick
- Graph-level variables
- Accumulated cost
- CorticalState updates (via atomic writes)

### Teardown

When the owning Agent enters the Terminal lifecycle state, the Hot Flow:
1. Completes the current tick (if in progress)
2. Flushes retained state to Store
3. Publishes a final `flow.{run_id}.terminated` Pulse
4. Releases all resources

---

## 8. Workflow/Activity Split for Deterministic Replay

The Workflow/Activity split ([03-GRAPH](03-GRAPH.md) S6) enables resumability from snapshots without re-executing non-deterministic operations.

### Replay procedure

When `Engine::resume(snapshot)` is called:

```
1. Load FlowSnapshot from disk

2. For each node in topological order:
   a. If node.state == Completed:
      - If Workflow: skip (output already in snapshot)
      - If Activity: load output from runs/<run-id>/activities/<node-id>.json
      - Mark as completed in the resumed Flow

   b. If node.state == Running:
      - If Workflow: re-execute (deterministic, same result)
      - If Activity: re-execute (was interrupted, output not recorded)

   c. If node.state == Pending:
      - Execute normally (standard execution loop)

3. Resume normal execution from first non-completed node
```

### Activity recording

Every Activity node's output is written to persistent storage immediately after execution:

```rust
pub struct ActivityRecord {
    pub node_id: NodeId,
    pub run_id: RunId,
    pub output: Vec<Signal>,
    pub cost: Cost,
    pub duration: Duration,
    pub completed_at: DateTime<Utc>,
    /// SHA-256 of the serialized output, for integrity verification.
    pub output_hash: ContentHash,
}
```

Recording is synchronous (flushed before proceeding to downstream nodes). This ensures that a crash between Activity completion and downstream execution does not lose the Activity output.

### Replay for the cognitive loop specifically

In a cognitive loop tick, the deterministic steps (SENSE with fixed Store state, ASSESS, COMPOSE, REACT) are Workflow Cells. The non-deterministic steps (ACT with LLM calls, VERIFY with external gate processes, PERSIST/BROADCAST with side effects) are Activity Cells.

```
Snapshot taken after VERIFY completed in tick 47:

Tick 47 replay:
  SENSE:             Workflow -> re-execute (same result)
  ASSESS:            Workflow -> re-execute (same result)
  COMPOSE:           Workflow -> re-execute (same result)
  ACT:               Activity -> load recorded output from activity log
  VERIFY:            Activity -> load recorded output from activity log
  PERSIST/BROADCAST: Activity -> re-execute (state may have been partially persisted)
  REACT:             Workflow -> re-execute (same result)
```

### Crash recovery

If the process crashes mid-tick:
1. The Engine loads the latest snapshot.
2. Activity Cells with recorded outputs skip re-execution.
3. Activity Cells without recorded outputs re-execute (the LLM call is retried).
4. Workflow Cells always re-execute (deterministic, so the result matches).
5. The tick resumes from the first non-completed node.

The worst case is a duplicate LLM call (if ACT completed but the Activity record was not flushed before crash). This is acceptable: the cost is one extra API call, and the system prefers availability over exactly-once semantics.

### CLI integration

```bash
# Start a plan
roko plan run plans/my-plan/

# Resume from the latest snapshot
roko plan run plans/my-plan/ --resume .roko/state/executor.json

# Resume a specific snapshot
roko plan run plans/my-plan/ --resume .roko/runs/<run-id>/state.json
```

---

## 9. Error Taxonomy: The Resilience Algebra

Errors are not a list. They form an algebra with four kinds, two operations (retry and escalate), and composition rules that determine how failures propagate through Graph execution.

### 9.1 Four Error Kinds

```rust
/// Error classification for the resilience algebra.
/// Each kind has algebraic retry and escalation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Network timeout, rate limit, temporary API error.
    /// Retry with exponential backoff. Escalate after N retries.
    Transient,

    /// Compile error, invalid config, schema mismatch.
    /// Same input = same failure. Never retry blindly.
    /// Escalate to replan or decompose.
    Deterministic,

    /// Disk full, memory pressure, too many open files.
    /// Retry after resource is freed. Escalate after timeout.
    Resource,

    /// Data corruption, missing critical files, auth revocation.
    /// Never retry. Escalate immediately.
    Catastrophic,
}
```

### 9.2 Operations on Error Kinds

The two operations are `retry` (attempt the same Cell again) and `escalate` (propagate to the containing Graph or human).

```
retry(Transient)      = Transient        -- retry is meaningful
retry(Deterministic)  = Deterministic    -- retry is pointless (same input, same failure)
retry(Resource)       = Resource         -- retry after resource freed
retry(Catastrophic)   = Catastrophic     -- retry is forbidden

escalate(Transient)   = after N retries  -- bounded patience
escalate(Deterministic) = immediately    -- no retry will help
escalate(Resource)    = after timeout    -- wait for resource, then give up
escalate(Catastrophic) = immediately     -- halt and preserve state
```

### 9.3 Supremum Composition

When a Graph has multiple nodes and multiple failures, the composite error kind is the supremum (worst case) under this partial order:

```
Catastrophic > Deterministic > Resource > Transient

sup(Transient, Resource)       = Resource
sup(Deterministic, Transient)  = Deterministic
sup(Catastrophic, anything)    = Catastrophic
```

This means: if any node in a parallel fan-out fails with a Catastrophic error, the entire fan-out fails as Catastrophic regardless of other nodes' status.

```rust
impl ErrorKind {
    /// Combine two error kinds. Returns the more severe.
    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Catastrophic, _) | (_, Self::Catastrophic) => Self::Catastrophic,
            (Self::Deterministic, _) | (_, Self::Deterministic) => Self::Deterministic,
            (Self::Resource, _) | (_, Self::Resource) => Self::Resource,
            _ => Self::Transient,
        }
    }

    /// Can this error kind be retried?
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Transient | Self::Resource)
    }

    /// Should this error kind escalate immediately?
    pub fn immediate_escalation(&self) -> bool {
        matches!(self, Self::Deterministic | Self::Catastrophic)
    }
}
```

### 9.4 Retry Policy as a Monoid

The retry policy composes monoidally over sequential Cell execution. Each Cell can declare its own retry policy, and the Graph-level policy provides the identity element.

```rust
struct RetryPolicy {
    base_ms: u64,
    max_delay_ms: u64,
    max_retries: u32,
    jitter_ms: u64,
}

impl RetryPolicy {
    /// Identity: the Graph-level default.
    const DEFAULT: Self = Self {
        base_ms: 500,
        max_delay_ms: 30_000,
        max_retries: 3,
        jitter_ms: 200,
    };

    /// Combine: node policy overrides graph policy for non-default fields.
    fn combine(&self, node_override: &RetryPolicy) -> Self {
        Self {
            base_ms: if node_override.base_ms != 0 { node_override.base_ms } else { self.base_ms },
            max_delay_ms: node_override.max_delay_ms.max(self.max_delay_ms),
            max_retries: node_override.max_retries.min(self.max_retries),
            jitter_ms: node_override.jitter_ms,
        }
    }

    /// Compute delay for attempt N.
    fn delay_for(&self, attempt: u32) -> Duration {
        let exp = self.base_ms.saturating_mul(2u64.saturating_pow(attempt));
        let jitter = thread_rng().gen_range(0..=self.jitter_ms);
        let total = exp.saturating_add(jitter).min(self.max_delay_ms);
        Duration::from_millis(total)
    }
}
```

The delay sequence for defaults: 500ms, 1000ms, 2000ms (+ jitter), then escalate.

---

## 10. Failure Strategies

When a node fails, the engine applies the failure strategy from the node's override or the Graph-level policy. Strategies are ordered from simplest to most complex. Full strategy definitions in [03-GRAPH](03-GRAPH.md) S7.

Strategy definitions (Fail, Retry, RetryWithEscalation, Decompose, Skip, Compensate, Replan, HumanResolve) are in [03-GRAPH](03-GRAPH.md) S7. The engine's execution behavior for each:

| Strategy | Engine behavior |
|---|---|
| **Fail** | Terminate Flow immediately. Error propagates to FlowState. |
| **Retry** | Re-execute node up to N times with backoff. Each retry publishes a `retrying` Pulse. |
| **RetryWithEscalation** | Retry, using Route protocol to select more capable Cell per attempt (e.g., Haiku -> Sonnet -> Opus). |
| **Decompose** | LLM generates decomposition; decomposed sub-tasks form a new sub-Graph replacing the failed node. |
| **Skip** | Continue execution. Downstream nodes receive empty input from skipped node. |
| **Compensate** | Execute compensation Graph with failed node's partial output + error as input. |
| **Replan** | Generate new plan from failure context (original task, error, prior attempts). Uses `roko prd plan` pipeline. |
| **HumanResolve** | Publish `human.resolution.requested` Pulse. Pause Flow. Timeout escalates to next strategy. |

```rust
pub struct RetryState {
    pub attempts: u32,
    pub max_retries: u32,
    pub backoff: BackoffStrategy,
    pub errors: Vec<CellError>,
}

pub struct ReplanState {
    pub original_task: Signal,
    pub failure_context: Value,
    pub prior_attempts: Vec<(Vec<Signal>, CellError)>,
    pub replan_count: u32,
    pub max_replans: u32,
}
```

---

## 11. Circuit Breaker as a React Cell

The circuit breaker is not a standalone utility — it is a Cell implementing the React protocol. It observes failure Pulses on the Bus and emits state-transition Pulses that other Cells (especially the Route protocol) consume.

### State Machine

```
         +-------------------------------------+
         |                                     |
    record_success()                      record_failure()
         |                                     |
         v                                     v
    +---------+    failure_count >= N    +----------+
    | Closed  | ----------------------> |  Open    |
    | (allow) |                         | (reject) |
    +---------+                         +----------+
         ^                                     |
         |    success                          |  reset_timeout elapsed
         |                                     v
         |                              +------------+
         +----------------------------- | HalfOpen   |
                                        | (probe 1)  |
                                        +------------+
                                               |
                                          failure
                                               |
                                               v
                                        +----------+
                                        |  Open    |
                                        +----------+
```

### The Circuit Breaker Cell

```rust
struct CircuitBreakerCell {
    /// Per-provider circuit state.
    circuits: DashMap<ProviderId, CircuitState>,
    threshold: u32,          // failures before open (default: 5)
    reset_timeout: Duration, // time before half-open (default: 300s)
}

#[derive(Debug, Clone)]
struct CircuitState {
    status: CircuitStatus,
    failure_count: u32,
    last_failure: Option<Instant>,
    consecutive_successes_in_half_open: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitStatus { Closed, Open, HalfOpen }

impl Cell for CircuitBreakerCell {
    fn protocols(&self) -> &[ProtocolId] { &[REACT_PROTOCOL] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let events = parse_health_events(&input);
        let mut transitions = Vec::new();

        for event in events {
            let provider = &event.provider_id;
            let mut state = self.circuits
                .entry(provider.clone())
                .or_insert_with(CircuitState::closed);

            let old = state.status;
            match event.outcome {
                Outcome::Success => state.record_success(),
                Outcome::Failure(ErrorKind::Catastrophic) => state.force_open(),
                Outcome::Failure(kind) if kind.retryable() => {
                    state.record_failure(self.threshold);
                }
                _ => {}
            }
            if old != state.status {
                transitions.push(CircuitTransition {
                    provider: provider.clone(), from: old, to: state.status,
                });
            }

            // Time-based half-open transition
            if state.status == CircuitStatus::Open {
                if state.last_failure.map_or(false, |t| t.elapsed() >= self.reset_timeout) {
                    state.status = CircuitStatus::HalfOpen;
                    transitions.push(CircuitTransition {
                        provider: provider.clone(),
                        from: CircuitStatus::Open, to: CircuitStatus::HalfOpen,
                    });
                }
            }
        }

        // Emit transition Pulses for the Route protocol to consume
        Ok(transitions.iter()
            .map(|t| Signal::pulse(
                Topic::new(format!("circuit.{}.{}", t.provider, t.to.as_str())),
                serde_json::to_value(t).unwrap(),
            ))
            .collect())
    }
}
```

State transition rules:
- **record_success**: HalfOpen -> Closed (reset counts); Closed -> reset failure count; Open -> ignored.
- **record_failure**: Closed -> Open when `failure_count >= threshold`; HalfOpen -> Open immediately (any failure reopens).
- **force_open**: Immediate Open, used for Catastrophic errors (no threshold).
```

### How Route Consumes Circuit State

The Route protocol subscribes to `circuit.*.opened` and `circuit.*.closed` Pulses. When a provider's circuit opens, Route removes it from the candidate set. When it transitions to HalfOpen, Route allows one probe request.

```rust
// Inside RouteProtocol implementation
fn filter_by_circuit_state(
    &self,
    candidates: &[RouteCandidate],
    circuits: &DashMap<ProviderId, CircuitState>,
) -> Vec<RouteCandidate> {
    candidates.iter()
        .filter(|c| {
            match circuits.get(&c.provider_id) {
                Some(state) => match state.status {
                    CircuitStatus::Closed => true,
                    CircuitStatus::HalfOpen => true,  // allow one probe
                    CircuitStatus::Open => false,      // reject
                },
                None => true,  // no circuit state = assume healthy
            }
        })
        .cloned()
        .collect()
}
```

---

## 12. Graceful Degradation

The degradation ladder is a state machine with Verify-gated transitions. Each level restricts system behavior, and the transition between levels is guarded by explicit conditions with hysteresis.

### Six Degradation Levels

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DegradationLevel {
    /// Full operation. All features active.
    Normal      = 0,

    /// Budget pressure reached warn_threshold.
    /// Route to cheaper models. Disable experiment exploration.
    BudgetWarn  = 1,

    /// Budget reached block_threshold.
    /// Block new tasks. Complete running tasks. Save state.
    BudgetBlock = 2,

    /// One or more provider circuits are open.
    /// Route to alternative providers. Fall back to local models.
    ProviderDegraded = 3,

    /// All providers are degraded or unreachable.
    /// Queue tasks. Retry periodically. Notify user.
    AllProvidersDegraded = 4,

    /// Disk pressure or I/O failure.
    /// Reduce logging. Prune aggressively. Warn user.
    DiskPressure = 5,

    /// Unrecoverable state.
    /// Save state. Print diagnostic. Exit with non-zero code.
    Unrecoverable = 6,
}
```

### Transition Guards with Hysteresis

The DegradationLens (an Observe-protocol Cell) monitors health conditions, evaluates them from most severe to least, and publishes transition Pulses. Transitions use asymmetric hysteresis:

- **Degradation** (getting worse): require 2 consecutive observations before transitioning.
- **Recovery** (getting better): require 5 consecutive observations before transitioning.

This asymmetry (Schmitt trigger analogy) prevents oscillation near thresholds.

Evaluation order (most severe first): unrecoverable error -> disk utilization > 0.95 -> all providers down -> any provider down -> budget > block_threshold -> budget > warn_threshold -> Normal.

### Per-Level Behavioral Restrictions

Each level restricts what the Engine does. Enforced at the Engine level, not by individual Cells.

| Level | Restrictions |
|---|---|
| Normal | Full operation |
| BudgetWarn | Route to cheaper models (max tier = T1). Disable experiment exploration. |
| BudgetBlock | Block new Flows. Complete running Flows. Snapshot all state proactively. |
| ProviderDegraded | Route protocol handles via circuit breaker. Emit user notification. |
| AllProvidersDegraded | Queue incoming tasks. Block new Flows. Start periodic probe timer. |
| DiskPressure | Reduce log verbosity to error-only. Aggressive GC on Store. Prune below demurrage threshold. |
| Unrecoverable | Snapshot all Flows. Flush episodes. Exit with diagnostic (non-zero code). |

---

## 13. Error Propagation Through Graphs

Errors propagate through the Graph's node hierarchy. Each layer catches errors from the layer below and decides: retry, escalate, or absorb.

### Propagation Hierarchy

```
Cell error
  |
  v
Node failure strategy (retry, decompose, skip, etc.)
  |
  v
Graph failure strategy (fail, compensate, replan)
  |
  v
Flow failure state
  |
  v
Engine: log, snapshot, notify
  |
  v
CLI/API: show to user
```

### Absorbable Subsystems

Some errors are absorbed — logged but not propagated upward. These are non-critical subsystems where failure should not halt the cognitive loop.

```rust
/// Errors from these subsystems are absorbed.
/// The system continues without the failed subsystem.
const ABSORBABLE_SUBSYSTEMS: &[&str] = &[
    "episode_logger",       // Learning is optional
    "metric_emitter",       // Observability is not critical
    "dashboard_renderer",   // Display errors are cosmetic
    "playbook_extractor",   // Skills improve future tasks only
    "experiment_tracker",   // A/B testing is best-effort
];
```

### Immediate Escalation Rules

Some errors escalate immediately regardless of retry policy.

```rust
fn should_escalate_immediately(error: &CellError) -> bool {
    match error.kind() {
        // Auth failure: cannot be fixed by retry
        ErrorKind::Catastrophic if error.is_auth() => true,

        // Budget exceeded: policy decision, not technical failure
        ErrorKind::Resource if error.is_budget() => true,

        // State corruption: risk of data loss requires human
        ErrorKind::Catastrophic if error.is_corruption() => true,

        // Config parse error: cannot start without valid config
        ErrorKind::Deterministic if error.is_config() => true,

        _ => false,
    }
}
```

### Rate Limit Handling

Rate limits (HTTP 429) are a special case of Transient errors that deserve their own retry logic.

```rust
fn handle_rate_limit(
    response: &HttpResponse,
    policy: &RetryPolicy,
    attempt: u32,
) -> Duration {
    // Prefer provider's Retry-After header
    if let Some(retry_after) = response.header("Retry-After") {
        if let Ok(secs) = retry_after.parse::<u64>() {
            return Duration::from_secs(secs.min(120));
        }
    }

    // Fall back to exponential backoff
    policy.delay_for(attempt)
}
```

When rate limits persist across multiple requests, the circuit breaker opens. The Route protocol then routes to alternative providers. Cascade: 429 -> retry -> circuit open -> route elsewhere.

---

## 14. Numerical Stability

Not every float decision matters equally. The ones that matter are on the hot path or accumulate over time.

### 14.1 The f32 vs f64 Decision Table

| Domain | Type | Why |
|---|---|---|
| Score axes (5 dimensions) | `f32` | Range [-1.0, 1.0]. 7 significant digits is sufficient for ranking. Stored per-Signal at high volume. |
| PAD vector (3 dimensions) | `f32` | Range [-1.0, 1.0]. Psychometric resolution does not need f64. |
| demurrage balance | `f32` | Range [0.0, 1.0]. The balance is a ratio, not a dollar amount. |
| HDC vectors | `u64` bitfield | Binary. No floating-point at all. 10,240 bits = 160 `u64`s. |
| Cost tracking (USD) | `f64` | Accumulates across the entire session. f32 loses precision past $16,777 (2^24). |
| EMA thresholds | `f64` | Small alpha (0.05) compounds rounding. After 1000 updates, f32 drift is measurable. |
| Bandit arm parameters | `f64` | UCB1 and Thompson sampling convergence depends on parameter precision. |
| Timestamps | `i64` | Millisecond Unix. No floating-point. |
| Token counts | `usize` | Integer. Saturating arithmetic. |
| Metric counters | `u64` | Monotonic. Never round through float. |

### 14.2 The Hot-Path Budget Table

These are the time budgets for the inner loop. Exceeding them delays the cognitive loop tick.

| Operation | Budget | Notes |
|---|---|---|
| `Decay::apply()` | < 10ns | Single `powf` or `exp`. Inline candidate. |
| `Score::effective()` | < 50ns | Weighted sum of 5 `f32`s (relevance, quality, confidence, novelty, utility per 01-SIGNAL). |
| HDC Hamming distance | < 1us | 160 `popcnt` on `u64` XOR result. |
| CorticalState read | < 1us | Single atomic load per field. |
| Metric counter increment | < 250ns | No heap allocation. |
| Histogram observation | < 750ns | Fixed bucket family. |
| Trace span start/finish | < 10us | Attribute copy. Exporter excluded. |
| Structured log enqueue | < 50us | JSON serialization may spill to background. |
| Prompt assembly | < 5ms | Token counting dominates. |
| Cascade router select | < 100us | Candidate scoring + bandit. |
| Episode log write | < 1ms | JSONL append. |
| Flow snapshot write | < 10ms | JSON serialize + atomic rename. |

**Total non-LLM overhead per tick**: < 20ms. This leaves the LLM call as the dominant cost, which is the correct budget distribution.

### 14.3 Serialization Precision

When Signals are serialized to JSONL, floating-point values need stable precision to avoid bloating storage with insignificant digits.

```rust
/// Round f32 to N decimal places before serialization.
/// Apply at serialization boundaries, not at every computation step.
fn round_f32(v: f32, decimals: u32) -> f32 {
    let factor = 10_f32.powi(decimals as i32);
    (v * factor).round() / factor
}
```

| Domain | Decimal places | Example | Storage impact |
|---|---|---|---|
| Score axis | 4 | 0.8500 | "0.85" vs "0.8499999..." |
| demurrage ratio | 6 | 0.002500 | "0.0025" vs "0.002499999..." |
| Cost (USD) | 4 | 12.3456 | Consistent with pricing granularity |
| EMA threshold | 6 | 0.654321 | Preserves convergence signal |
| Calibration gauge | 4 | 0.8125 | Display-friendly |

### 14.4 NaN/Inf Defense

The defensive pattern applies at computation boundaries. Clamping at every intermediate step masks bugs; clamping at output boundaries catches them.

```rust
/// Apply at the output boundary of any Cell that produces f32/f64.
/// Log anomalies for debugging. Clamp to the valid range.
trait NumericallyStable {
    fn stabilize(self, name: &str, min: Self, max: Self, default: Self) -> Self;
}

// Implemented for both f32 and f64:
// if self.is_nan() || self.is_infinite() { warn + return default }
// else { self.clamp(min, max) }
```

### 14.5 Specific NaN/Inf Sources and Mitigations

| Source | How it arises | Mitigation |
|---|---|---|
| `0.0 / 0.0` | Division by zero in score normalization | Check denominator before division |
| `exp(710.0_f64)` | Overflow in demurrage exponent | Clamp exponent input to 700.0 |
| `powf(0.5, 0.0 / 0.0)` | NaN propagation from `half_life_ms = 0` | Guard: `if half_life_ms == 0 { return 0.0; }` |
| `(-1.0_f32).sqrt()` | Negative sqrt | Never occurs: all inputs are non-negative by construction |
| `Inf - Inf` | Indeterminate from unbounded accumulation | Avoid unbounded accumulation; use saturating arithmetic |

### 14.6 EMA Precision Under Long Runs

EMA with small alpha accumulates rounding error in f32:

```
EMA formula: new = alpha * sample + (1.0 - alpha) * old

After N updates with alpha = 0.05:
  f32 drift from true value: ~1e-4 at N=1000, ~1e-3 at N=10000
  f64 drift from true value: ~1e-12 at N=1000, ~1e-11 at N=10000
```

For adaptive gate thresholds that accumulate over thousands of gate evaluations, f64 is mandatory. The error compounds because each update reads the previous (already-rounded) value.

The concrete risk: an f32 EMA for gate thresholds could drift by 0.001 after 10,000 updates, which is enough to flip borderline pass/fail decisions. For this reason, all EMA computations use f64 and only round to f32 at serialization time.

---

## 15. Budget Enforcement

The engine enforces budgets at three levels: per-node, per-Flow, and per-Agent (global). Budget tracking is demurrage-aware: the cost of holding Signals in context (via demurrage) is included in the budget.

```rust
pub struct BudgetTracker {
    /// Per-Flow budget ledgers.
    pub flow_budgets: DashMap<RunId, BudgetLedger>,

    /// Per-Agent budget ledgers.
    pub agent_budgets: DashMap<AgentId, BudgetLedger>,
}

pub struct BudgetLedger {
    pub initial: Cost,
    pub spent: Cost,
    pub reserved: Cost,
    pub warning_threshold: f64,

    /// Per-Cell cost breakdown.
    pub block_costs: BTreeMap<CellRef, Cost>,

    /// Per-node cost breakdown.
    pub node_costs: BTreeMap<NodeId, Cost>,

    /// Demurrage costs (cost of holding Signals in context).
    pub demurrage_costs: Cost,
}

impl BudgetLedger {
    pub fn remaining(&self) -> Cost {
        Cost(self.initial.0.saturating_sub(self.spent.0 + self.reserved.0))
    }

    pub fn utilization(&self) -> f64 {
        self.spent.0 as f64 / self.initial.0 as f64
    }

    pub fn should_warn(&self) -> bool {
        self.utilization() >= self.warning_threshold
    }
}
```

### Budget enforcement behavior

| Condition | Action |
|---|---|
| Estimated cost > remaining budget | Cell not started; `BudgetExhausted` error |
| Actual cost exceeds estimate by >2x | Warning Pulse published |
| Budget utilization > warning threshold (default 80%) | `budget.warning` Pulse published |
| Budget exhausted mid-Flow | Flow fails with `BudgetExhausted` |
| Agent vitality < 0.05 | Agent enters Terminal phase; all Flows wind down |

---

## 16. Cancellation Propagation

Cancellation propagates from parent to child, from Flow to nodes, and from Agent to all owned Flows.

```rust
/// Cancellation propagation chain:
///
/// Engine shutdown
///   +-> All Flows cancelled
///       +-> All running nodes cancelled
///           +-> Cell.execute() checks cancel token
///               +-> Sub-Graphs cancelled recursively
///
/// Agent.terminate()
///   +-> All Agent-owned Flows cancelled
///       +-> Hot Flow teardown
///
/// Flow.cancel(reason)
///   +-> All running nodes receive CancellationToken signal
///       +-> Cells check token between steps
```

Cancellation is cooperative: Cells check the `CancellationToken` between steps. Long-running Cells (LLM calls) also register abort handlers with the underlying HTTP client.

```rust
impl CellContext {
    /// Check if cancellation has been requested.
    /// Cells SHOULD call this between significant operations.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}
```

---

## 17. Human-in-the-Loop

`HumanInput` nodes pause execution and wait for human input. The engine publishes a request Pulse and blocks the node until input is received or the timeout expires.

```rust
pub struct HumanInputRequest {
    pub run_id: RunId,
    pub node_id: NodeId,
    pub prompt: String,
    pub input_schema: Option<TypeSchema>,
    pub timeout: Option<Duration>,
    pub requested_at: DateTime<Utc>,
}

pub struct HumanInputResponse {
    pub run_id: RunId,
    pub node_id: NodeId,
    pub input: Vec<Signal>,
    pub responded_at: DateTime<Utc>,
    pub responder: Author,
}
```

Human input can arrive via:
- **TUI**: Interactive prompt in the dashboard
- **CLI**: `roko flow respond <run-id> <node-id> <payload>`
- **API**: `POST /flows/{run_id}/nodes/{node_id}/input`
- **Slack**: via the Slack MCP connector

---

## 18. Concurrency and Parallelism

The engine uses Tokio for async execution with explicit concurrency controls.

### Parallelism levels

| Level | Control | Default |
|---|---|---|
| Engine-wide | `Engine.semaphore` | 32 concurrent nodes |
| Per-Flow | `GraphPolicy.max_parallelism` | 8 concurrent nodes |
| Per-Agent | `Agent.slots.max_slots` | 4 concurrent slots |
| Per-FanOut | Number of downstream edges | Unbounded (limited by Flow) |

### Ordering guarantees

- **Within a node**: sequential execution (Cells are not internally parallelized by the engine).
- **Within a Flow**: topological order respected. Parallel nodes are truly concurrent.
- **Across Flows**: no ordering guarantees. Flows are independent.
- **Hot Graph ticks**: sequential (tick N completes before tick N+1 starts).

---

## 19. Cascade Router Integration

The engine integrates with the Route protocol for model selection at every LLM-backed Cell execution. The cascade router uses EFE (Expected Free Energy, Friston 2006), not LinUCB, to balance exploration and exploitation.

### Progressive cascade

```
T0: Pattern match (pure Rust, no LLM, ~80% of ticks, $0 cost)
  |
  +- Match found --> execute reflex --> verify --> done
  |
  +- No match --> escalate to T1

T1: Fast/cheap model (Haiku-class, <$0.001 per call)
  |
  +- EFE score below threshold --> execute --> verify --> done
  |
  +- EFE score above threshold --> escalate to T2

T2: Capable model (Sonnet/Opus-class, $0.01-0.10 per call)
  |
  +- Execute --> verify --> done
```

The EFE threshold adapts based on:
- **Regime**: Crisis mode forces T0/T1 only
- **Vitality**: Low vitality reduces exploration
- **Task complexity**: Simple tasks stay at T0/T1
- **Historical performance**: Cells that consistently fail at lower tiers are directly routed to higher tiers

### Route context construction

```rust
pub fn build_route_context(
    agent: &Agent<Active>,
    task: &Signal,
) -> RouteContext {
    RouteContext {
        regime: agent.cortical_state.regime(),
        vitality: agent.vitality.ratio(),
        complexity: estimate_complexity(task),
        urgency: compute_urgency(task, &agent.vitality),
        budget_remaining: agent.vitality.remaining_budget,
        context_signals: vec![], // filled by Compose
    }
}
```

---

## 20. Episode Logging

Every completed Flow produces an Episode Signal persisted to `.roko/episodes.jsonl`. Episodes feed the learning loops.

```rust
pub struct Episode {
    pub run_id: RunId,
    pub graph_name: String,
    pub agent_id: Option<AgentId>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,

    /// Per-node summaries.
    pub node_summaries: Vec<NodeSummary>,

    /// Final Verdict (if Verify was invoked).
    pub verdict: Option<Verdict>,

    /// Total cost.
    pub total_cost: Cost,

    /// Vitality at start and end.
    pub vitality_start: Option<f64>,
    pub vitality_end: Option<f64>,

    /// Somatic state at end.
    pub somatic: Option<PadVector>,

    /// HDC fingerprint of this episode (for similarity search).
    pub hdc_fingerprint: HdcVector,

    /// Model(s) used.
    pub models: Vec<String>,

    /// Calibration data: predictions vs outcomes.
    pub calibration: Vec<CalibrationUpdate>,
}

pub struct NodeSummary {
    pub node_id: NodeId,
    pub block_ref: CellRef,
    pub execution_class: ExecutionClass,
    pub cost: Cost,
    pub duration: Duration,
    pub verdict: Option<Verdict>,
}
```

---

## 21. Cost and Time Estimation

Before executing a Graph, the engine provides cost and time estimates by summing Cell-level estimates along the critical path.

```rust
pub struct GraphEstimate {
    /// Minimum cost estimate (best case: all nodes at lower bound).
    pub min_cost: Cost,

    /// Maximum cost estimate (worst case: all retries exhausted at upper bound).
    pub max_cost: Cost,

    /// Critical path duration (longest sequential chain of node durations).
    pub critical_path_duration: DurationEstimate,

    /// Confidence in the estimate (0.0..=1.0).
    /// 1.0 = all Cells provided estimates. Decreases by 1/N for each
    /// Cell that returned None from estimated_cost().
    pub confidence: f64,

    /// Nodes that could not be costed (returned None from estimated_cost).
    /// These are the nodes degrading confidence.
    pub uncostable_nodes: Vec<NodeId>,

    /// Per-node estimates.
    pub node_estimates: BTreeMap<NodeId, NodeEstimate>,
}

pub struct CostEstimate {
    pub lower: Cost,
    pub expected: Cost,
    pub upper: Cost,
}

pub struct DurationEstimate {
    pub lower: Duration,
    pub expected: Duration,
    pub upper: Duration,
}

pub struct NodeEstimate {
    pub cost: Option<Cost>,
    pub duration: Option<Duration>,
    /// Whether this node is on the critical path.
    pub critical: bool,
}
```

---

## 22. Observability

### Built-in Engine Lenses

The engine publishes all lifecycle events as Pulses on Bus. Lenses (Observe-protocol Cells) subscribe and produce observation Signals for surfaces.

| Lens | Subscribes to | Produces |
|---|---|---|
| `FlowProgressLens` | `flow.*.node.*.completed` | Progress percentages per Flow |
| `CostLens` | `cost.charged`, `flow.*.budget.*` | Real-time cost telemetry |
| `FailureLens` | `flow.*.node.*.failed` | Failure analysis with suggested strategies |
| `LatencyLens` | `flow.*.node.*.completed` | Per-node and per-Flow latency distributions |
| `ConcurrencyLens` | `flow.*.started`, `flow.*.completed` | Active Flows and parallelism utilization |

### Resilience Observability

The resilience system observes itself via Bus Pulses forming a feedback Loop:

```
Circuit breaker emits:     circuit.{provider}.opened / closed / half_open
Degradation lens emits:    system.degradation.{level}
Route protocol consumes:   circuit.{provider}.*
Engine consumes:           system.degradation.*
```

| Metric | Type | What it measures |
|---|---|---|
| `circuit_state` | Gauge per provider | Current circuit status (0=closed, 1=half-open, 2=open) |
| `circuit_transitions_total` | Counter per provider | Total state transitions |
| `retry_attempts_total` | Counter per error kind | Retries by error classification |
| `degradation_level` | Gauge | Current degradation level (0-6) |
| `error_kind_total` | Counter per kind | Errors by classification |
| `time_in_degradation_seconds` | Counter per level | Cumulative time at each level |

---

## 24. Byzantine Cell Defenses

Threat model: a Byzantine Cell replaces COMPOSE and (a) injects malicious prompt instructions, (b) inflates token counts, (c) returns stale context, or (d) delays to starve downstream Cells.

| Defense | Mechanism |
|---|---|
| **Verify as consensus gate** | BFT quorum: N gates vote, `pass_count >= quorum` required. Even if COMPOSE/ACT produce corrupted output, VERIFY catches it. |
| **Budget as kill switch** | BudgetTracker rejects Cell execution when estimated cost > remaining. Post-execution tracking catches overruns. |
| **Timeout as liveness guarantee** | Node-level timeout enforced by Engine. Infinite-delay Cells killed; failure strategy takes over. |
| **CaMeL IFC tagging** | Capability provenance on Cells. No `llm_call` capability = cannot invoke LLM. Enforced by Space's capability intersection, not Cell self-report. |

**Residual risk**: a Byzantine ACT Cell that produces plausible but subtly wrong output passing all Verify gates. Defense: calibration over time — REACT logs episodes, learning tracks prediction accuracy, persistent errors trigger regime escalation to T2 or human review.

---

## 25. Feedback Loops

| Loop | What it observes | What it adjusts |
|---|---|---|
| **EFE adaptation** | Prediction error from ASSESS across ticks | T0/T1/T2 escalation thresholds in the Route protocol |
| **Section effects** | Gate pass/fail correlated with COMPOSE sections | Beta-distribution posteriors for VCG bidder valuations |
| **Regime transitions** | Prediction error trend over 3+ ticks | Adaptive clock multipliers (Calm/Normal/Volatile/Crisis) |
| **Vitality phases** | Budget remaining / initial | COMPOSE token budget, Verify rigor, exploration rate |
| **Reflex promotion** | T2 patterns that succeed 5+ times at >90% | T0 reflex store gains new rules; future ticks short-circuit |
| **Circuit breaker** | Provider failure/success outcomes | Provider availability for the Route protocol |
| **Degradation lens** | Budget utilization, provider health, disk state | System-wide behavioral restrictions |
| **Retry policy adaptation** | Success rate after N retries | Whether to increase/decrease max_retries |
| **Error classification refinement** | Errors initially classified as Transient that never recover | Reclassify as Deterministic after N failed retries |

Each of these Loops is itself a Graph — a feedback edge from REACT back to ASSESS with a learning Cell in between. The Loops compose because they operate on the same Signal types through the same Bus.

---

## 26. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| 1 | Engine starts a Flow from a Graph and returns a RunId | Integration test |
| 2 | Flow lifecycle transitions publish correct Pulses on Bus | Integration test: subscribe, start Flow, verify Pulses |
| 3 | All lifecycle Pulses match documented topics and graduation policies | Integration test |
| 4 | Node execution follows topological order | Unit test: 3-node chain, verify execution order |
| 5 | Parallel nodes execute concurrently up to `max_parallelism` | Integration test: FanOut with 4 branches, max_parallelism=2 |
| 6 | Hot Graph re-fires on clock tick | Integration test: register Hot Graph, advance clock, verify two executions |
| 7 | Hot Graph retains state between ticks | Integration test: first tick sets variable, second tick reads it |
| 8 | Hot Graph teardown flushes state to Store | Integration test: deregister, verify Store contains state |
| 9 | Activity replay returns recorded output (no re-execution) | Integration test: execute Flow, resume from snapshot, verify no LLM call on Activity node |
| 10 | Workflow replay re-executes deterministically | Integration test: Branch node produces same result on replay |
| 11 | Failure strategy Fail terminates Flow immediately | Unit test |
| 12 | Failure strategy Retry retries N times with backoff | Integration test with mock Cell that fails then succeeds |
| 13 | Failure strategy RetryWithEscalation uses Route for alternatives | Integration test: verify different Cell used on retry |
| 14 | Failure strategy Decompose creates sub-Graph | Integration test |
| 15 | Failure strategy Replan generates new plan | Integration test |
| 16 | Failure strategy HumanResolve publishes request and waits | Integration test with mock human input |
| 17 | Snapshot serializes and deserializes round-trip | Unit test |
| 18 | Resume from snapshot skips completed nodes | Integration test: 3-node chain, snapshot after node 2, resume executes only node 3 |
| 19 | Cancellation propagates from Flow to running nodes | Integration test: cancel Flow, verify node receives cancellation |
| 20 | Budget enforcement halts Flow when exhausted | Integration test: set tight budget, verify BudgetExhausted error |
| 21 | Budget warning published at 80% utilization | Integration test |
| 22 | Demurrage costs tracked in BudgetLedger | Unit test |
| 23 | Human-in-loop: HumanInput node pauses and resumes on input | Integration test |
| 24 | Episode logged after Flow completion | Integration test: complete Flow, verify episode in `.roko/episodes.jsonl` |
| 25 | Episode includes HDC fingerprint | Unit test |
| 26 | Cascade router uses EFE: T0 -> T1 -> T2 escalation | Integration test with mocked models |
| 27 | Cascade router respects regime (Crisis -> T0/T1 only) | Unit test |
| 28 | Lens receives lifecycle Pulses and produces observation Signals | Integration test |
| 29 | Cost estimate computed before execution | Unit test: estimate on Graph with known Cell costs |
| 30 | Time estimate identifies critical path | Unit test: parallel Graph, verify critical path is longest branch |
| 31 | Engine-wide concurrency limited by semaphore | Integration test: start 64 Flows with semaphore=32, verify max 32 concurrent |
| 32 | ErrorKind.combine() returns supremum | Unit test: all 10 pairwise combinations |
| 33 | RetryPolicy.delay_for() produces exponential backoff with jitter | Unit test: verify 500ms, 1000ms, 2000ms sequence |
| 34 | Circuit breaker transitions Closed->Open after threshold failures | Unit test |
| 35 | Circuit breaker transitions Open->HalfOpen after reset_timeout | Unit test |
| 36 | Circuit breaker transitions HalfOpen->Closed after one success | Unit test |
| 37 | DegradationLens transitions require hysteresis (2 degrade, 5 recover) | Unit test |
| 38 | Absorbable subsystem errors do not halt the cognitive loop | Integration test: inject episode_logger error, verify loop continues |
| 39 | NumericallyStable trait catches NaN and returns default | Unit test |
| 40 | EMA uses f64, rounds to f32 only at serialization | Code review: verify all EMA computations are f64 |
| 41 | T0 short-circuit handles ~80% of ticks at $0 cost | Integration test with mock ASSESS returning T0 noop |
| 42 | Byzantine Verify Cell with quorum rejects minority corruption | Unit test: 3 gates, 2 pass, 1 fail -> pass |

---

## 27. Citations

| Claim | Source |
|---|---|
| Workflow/Activity split for deterministic replay | Temporal.io execution model (https://temporal.io) |
| EFE (Expected Free Energy) for cascade routing | Friston (2006), "A free energy principle for the brain" |
| Circuit breaker pattern | Nygard (2007), "Release It!" |
| Exponential backoff with jitter | AWS Architecture Blog, "Exponential Backoff And Jitter" |
| BFT consensus gates | Lamport, Shostak, Pease (1982), "The Byzantine Generals Problem" |
| VCG auction for context assembly | Vickrey (1961), Clarke (1971), Groves (1973) |
| Gamma/theta/delta oscillation bands | Buzsaki (2006), "Rhythms of the Brain" |
| Graceful degradation with hysteresis | Standard control theory; Schmitt trigger analogy |
| CaMeL IFC (Information Flow Control) for capability tagging | Decat et al. (2012), capability-based security models |
| EMA rounding analysis (f32 drift after 10K updates) | IEEE 754-2008 single-precision analysis |
| HDC Hamming distance via popcnt | Kanerva (2009), "Hyperdimensional Computing" |
| Retry policy as monoid | Algebraic composition of configuration; cf. Scala Cats library monoid instances |
