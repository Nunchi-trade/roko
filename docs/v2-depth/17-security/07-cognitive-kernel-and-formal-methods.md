# Cognitive Kernel and Formal Methods

> Depth for [16-SECURITY.md](../../unified/16-SECURITY.md). Expresses cognitive kernel safety primitives and formal verification as Space isolation, Bus channels, and scheduling. Cognitive Namespaces map to Space specialization. Cognitive Signals map to Pulses with priority. Cognitive Scheduling maps to EDF on the Graph executor. Formal verification and MEV protection map to domain-specific Verify Cells.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, Bus), [02-CELL](../../unified/02-CELL.md) (Cell protocols, Verify), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Pipeline), [05-AGENT](../../unified/05-AGENT.md) (Agent runtime, Space), [16-SECURITY](../../unified/16-SECURITY.md) (sandboxing, autonomy levels, recursive safety)

---

## 1. The Cognitive Kernel Analogy

Roko implements OS-level primitives for agents, adapted from what Linux provides for processes. The insight: just as a Unix kernel mediates all hardware access through system calls, Roko mediates all cognitive actions through a controlled interface. The four primitives map directly to unified concepts:

| Linux Primitive | Purpose | Roko Equivalent | Unified Primitive |
|---|---|---|---|
| Namespaces (PID, net, mount) | Process isolation | Cognitive Namespaces | **Space** specialization |
| Signals (SIGTERM, SIGKILL) | Process control | Cognitive Signals | **Pulse** with priority |
| Scheduler (CFS) | CPU time allocation | Cognitive Scheduling | EDF on **Graph** executor |
| System calls (syscall table) | Hardware access control | Engram Syscalls | **Verify** protocol Pipeline |
| Capabilities (CAP_NET_RAW) | Fine-grained permissions | Capability tokens | Cell capability intersection |
| cgroups (resource limits) | Resource containment | Budget limits | Verify + React Cells |
| seccomp (syscall filtering) | Syscall allowlist | Tool permissions | Pipeline of Verify Cells |
| SELinux/AppArmor (MAC) | Mandatory access control | SafetyLayer | Composite Verify Cell |

No new concepts needed. Every kernel primitive is a composition of existing unified primitives.

---

## 2. Cognitive Namespaces as Space Specialization

A Cognitive Namespace is a **Space** (see [05-AGENT.md](../../unified/05-AGENT.md)) that owns a Bus partition and a Store partition. Members share these resources under access control. The Space provides knowledge isolation -- an agent's private knowledge cannot leak to other agents without an explicit channel.

### Space Definition

```toml
# Space: coder-namespace
# An isolated knowledge domain for a coder agent.

[space]
id = "coder-namespace"
description = "Private knowledge space for coder agent"

[space.store]
partition = "coder"
capacity = 100_000           # max Signals in this partition
demurrage_rate = 0.01        # standard decay

[space.bus]
partition = "coder"
topics = [
  "coder.tool.*",
  "coder.action.*",
  "coder.health.*",
]

[space.acl]
readers = ["coder", "reviewer"]
writers = ["coder"]
allow_anonymous_read = false
```

### Cross-Space Channels

Knowledge flows between Spaces only through declared channels. Each channel is itself described by a simple structure:

```rust
/// A channel between two Spaces with explicit transfer rules.
/// Knowledge only flows between Spaces through these channels.
pub struct SpaceChannel {
    /// Source Space.
    pub from: SpaceId,
    /// Destination Space.
    pub to: SpaceId,
    /// Which Signal Kinds can flow through.
    pub allowed_kinds: Vec<Kind>,
    /// Whether transfers are logged to the audit trail.
    pub audit_transfers: bool,
    /// Maximum transfer rate (Signals per second).
    pub rate_limit: Option<f64>,
    /// Whether taint is preserved on transfer.
    pub preserve_taint: bool,
}
```

### Safety Properties

**Isolation guarantee**: An agent's private Signals are invisible to other agents unless an explicit channel exists. This prevents:

- **Knowledge poisoning**: A compromised agent cannot corrupt another agent's Store partition.
- **Information leakage**: Proprietary strategies stay within their Space.
- **Cross-contamination**: Experimental knowledge (dream outputs, hypothesis fragments) cannot accidentally pollute production knowledge.

**Explicit flow**: Every cross-Space transfer is auditable. The channel logs source, destination, Signal hash, and timestamp. This composes with the custody chain (see [04-audit-witness-and-forensics.md](04-audit-witness-and-forensics.md)) -- cross-Space transfers are custody events.

### Space Hierarchy for Groups

In a Group (see [10-GROUPS.md](../../unified/10-GROUPS.md)), Spaces nest:

```
Global Space (public)
  +-- Group Space (shared among group members)
       +-- Agent Space (private per agent)
       +-- Agent Space (private per agent)
```

Knowledge flows: private -> shared (controlled by channel policy) -> public (controlled by posting policy). Each transition is a channel crossing with audit.

---

## 3. Cognitive Signals as Pulses with Priority

Cognitive Signals are typed interrupts that alter agent behavior without killing the process. In unified terms, they are **Pulses** published on priority-ordered Bus topics.

### Signal Types as Pulses

```rust
/// Cognitive signals as Pulses on the Bus.
/// Each signal is a Pulse published to a priority-ordered topic.
/// The agent's React Cell subscribes to these topics.
pub enum CognitiveSignalKind {
    /// Suspend reasoning, serialize state to disk.
    Pause,
    /// Resume from serialized state.
    Resume,
    /// Change current task priority.
    Reprioritize { task_id: String },
    /// Add context mid-reasoning.
    InjectContext { signal_hash: ContentHash },
    /// Switch to stronger model immediately.
    Escalate,
    /// Reduce arousal, slow down.
    Cooldown,
    /// Switch to exploratory mode.
    Explore,
    /// Graceful termination.
    Shutdown,
}

/// Priority ordering. Lower number = higher priority.
impl CognitiveSignalKind {
    pub fn priority(&self) -> u8 {
        match self {
            Self::Shutdown => 1,
            Self::Pause => 2,
            Self::Escalate => 3,
            Self::Cooldown => 4,
            Self::Reprioritize { .. } => 5,
            Self::InjectContext { .. } => 6,
            Self::Explore => 7,
            Self::Resume => 8,
        }
    }

    pub fn topic(&self) -> &str {
        match self {
            Self::Shutdown => "agent.signal.shutdown",
            Self::Pause => "agent.signal.pause",
            Self::Escalate => "agent.signal.escalate",
            Self::Cooldown => "agent.signal.cooldown",
            Self::Reprioritize { .. } => "agent.signal.reprioritize",
            Self::InjectContext { .. } => "agent.signal.inject",
            Self::Explore => "agent.signal.explore",
            Self::Resume => "agent.signal.resume",
        }
    }
}
```

### Signal Delivery as Bus Subscription

The agent's cognitive runtime has a React Cell that subscribes to signal topics. Higher-priority signals preempt lower-priority ones:

```rust
/// React Cell: processes incoming cognitive signals.
/// Subscribes to all agent.signal.* Bus topics.
/// Higher-priority signals preempt lower-priority ones.
pub struct CognitiveSignalHandler {
    /// Priority queue of pending signals.
    queue: BinaryHeap<PrioritizedPulse>,
    /// Timeout before escalation (Pause -> Shutdown, etc.).
    escalation_timeouts: HashMap<u8, Duration>,
}

impl Cell for CognitiveSignalHandler {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "cognitive-signal-handler" }

    async fn execute(
        &self,
        input: Vec<Signal>,    // Pulses from agent.signal.* topics
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();

        for signal in &input {
            let kind = extract_signal_kind(signal)?;

            // Check for timeout escalation on pending signals
            self.escalate_timed_out(&mut outputs);

            match kind {
                CognitiveSignalKind::Shutdown => {
                    // Complete current work unit, persist state, exit
                    outputs.push(Signal::new(Kind::Event, AgentEvent::ShuttingDown));
                    ctx.agent().serialize_state().await?;
                    ctx.agent().shutdown().await?;
                }
                CognitiveSignalKind::Pause => {
                    // Serialize state and suspend
                    ctx.agent().serialize_state().await?;
                    ctx.agent().suspend().await?;
                    outputs.push(Signal::new(Kind::Event, AgentEvent::Paused));
                }
                CognitiveSignalKind::Escalate => {
                    // Force T2 model routing
                    ctx.agent().set_routing_override(RoutingTier::T2).await?;
                    outputs.push(Signal::new(Kind::Event, AgentEvent::Escalated));
                }
                CognitiveSignalKind::Cooldown => {
                    // Modulate affect toward caution
                    ctx.agent().modulate_affect(AffectDelta {
                        arousal: -0.3,
                        ..Default::default()
                    }).await?;
                    outputs.push(Signal::new(Kind::Event, AgentEvent::CooledDown));
                }
                CognitiveSignalKind::InjectContext { signal_hash } => {
                    // Add Signal to active context without interrupting
                    let injected = ctx.store().get(&signal_hash).await?;
                    ctx.agent().inject_context(injected).await?;
                }
                _ => { /* handle remaining signal types */ }
            }
        }

        Ok(outputs)
    }
}
```

### Safety Properties

**Non-destructive**: No cognitive signal causes abrupt termination with state loss. Even Shutdown allows the agent to finish its current work unit and serialize state. This prevents mid-action inconsistency (half-written files, unwound positions).

**Human oversight (EU AI Act Article 14)**: Cognitive signals provide the mechanism for human oversight:
- **Pause** for anomalous behavior.
- **InjectContext** for new safety constraints without restart.
- **Escalate** for deeper reasoning when the agent is cutting corners.
- **Cooldown** to reduce risk-taking.
- **Shutdown** for graceful termination with full state preservation.

**Escalation on timeout**: If a signal is not acknowledged within a configurable timeout, it escalates: Cooldown -> Pause -> Shutdown. An agent that ignores a Pause signal will be shut down.

---

## 4. Cognitive Scheduling as EDF on the Graph Executor

Cognitive Scheduling allocates reasoning resources based on priority, deadline, and expected value. In unified terms, it is **Earliest Deadline First (EDF) scheduling** on the Graph executor (see [04-EXECUTION.md](../../unified/04-EXECUTION.md)).

### Resource Allocation

The scheduler allocates three resources:
- **LLM inference budget** (tokens/dollar)
- **Context window space** (tokens)
- **Wall-clock time** (seconds)

```rust
/// Cognitive scheduling: EDF on Graph executor.
/// Tasks are ordered by deadline, with priority tie-breaking.
pub struct CognitiveScheduler {
    /// Tasks ordered by deadline.
    ready_queue: BinaryHeap<ScheduledTask>,
    /// Budget tracking per agent.
    budgets: HashMap<AgentId, Budget>,
}

pub struct ScheduledTask {
    pub task_id: String,
    pub agent_id: AgentId,
    pub deadline: Option<DateTime<Utc>>,
    pub priority: f64,          // cognitive_priority = urgency * value / cost
    pub estimated_cost: f64,    // estimated LLM tokens
}

pub struct Budget {
    pub token_budget: f64,
    pub token_spent: f64,
    pub cost_budget_usd: f64,
    pub cost_spent_usd: f64,
    pub time_budget: Duration,
    pub time_spent: Duration,
}

impl CognitiveScheduler {
    /// Select the next task to execute.
    /// EDF: tasks with deadlines get priority. Ties broken by priority score.
    pub fn next_task(&mut self) -> Option<ScheduledTask> {
        self.ready_queue.pop()
    }

    /// Check budget limits and emit appropriate signals.
    pub fn check_budget(&self, agent_id: &AgentId) -> BudgetDecision {
        let budget = match self.budgets.get(agent_id) {
            Some(b) => b,
            None => return BudgetDecision::Continue,
        };

        let token_ratio = budget.token_spent / budget.token_budget;
        let cost_ratio = budget.cost_spent_usd / budget.cost_budget_usd;
        let time_ratio = budget.time_spent.as_secs_f64()
            / budget.time_budget.as_secs_f64();

        let max_ratio = token_ratio.max(cost_ratio).max(time_ratio);

        if max_ratio >= 1.0 {
            BudgetDecision::Shutdown  // hard limit reached
        } else if max_ratio >= 0.9 {
            BudgetDecision::Cooldown  // approaching limit
        } else if max_ratio >= 0.8 {
            BudgetDecision::DowngradeModel  // T2 -> T1 -> T0
        } else {
            BudgetDecision::Continue
        }
    }
}

pub enum BudgetDecision {
    Continue,
    DowngradeModel,
    Cooldown,
    Shutdown,
}
```

### Safety Properties

**Starvation prevention**: The scheduler implements fairness. No task monopolizes resources indefinitely. Tasks that have been waiting get priority boosts. This prevents the scenario where a high-priority task starves all routine work.

**Deadline enforcement**: Safety-critical tasks (e.g., position unwinding before liquidation) always receive deadline priority via EDF.

**Priority inversion prevention**: When a high-priority task depends on a low-priority task's result, the scheduler temporarily elevates the low-priority task (priority inheritance protocol; Sha et al. 1990).

**Cost containment**: Budget limits at three levels (soft -> model downgrade, medium -> cooldown, hard -> shutdown) prevent cost runaway.

---

## 5. Formal Verification as Domain-Specific Verify Cells

Formal verification tools (Heimdall-rs, Slither, Echidna, hevm, Certora) are domain-specific Verify Cells for chain safety. The pattern generalizes: any domain with formal methods can express its verification as Verify Cells in a Pipeline.

### The Five-Stage Verification Pipeline

```toml
# Graph: formal-verification-pipeline
# Five stages ordered by increasing cost and depth.
# Early stages filter before expensive stages run.

[graph]
id = "formal-verification-pipeline"
pattern = "Pipeline"
description = "Chain-domain formal verification"

[[graph.cells]]
id = "heimdall-decompile"
protocol = "Verify"
description = "Stage 1: decompile bytecode, extract ABI (fast, ~1s)"

[[graph.cells]]
id = "slither-static"
protocol = "Verify"
description = "Stage 2: static analysis for known vulnerability patterns (~5s)"

[[graph.cells]]
id = "echidna-fuzz"
protocol = "Verify"
description = "Stage 3: property-based fuzzing with coverage guidance (~60s)"

[[graph.cells]]
id = "hevm-symbolic"
protocol = "Verify"
description = "Stage 4: symbolic execution for assertion violations (~300s)"

[[graph.cells]]
id = "certora-formal"
protocol = "Verify"
description = "Stage 5: full formal verification of safety properties (~600s)"

[[graph.edges]]
from = "heimdall-decompile.out"
to = "slither-static.in"

[[graph.edges]]
from = "slither-static.out"
to = "echidna-fuzz.in"

[[graph.edges]]
from = "echidna-fuzz.out"
to = "hevm-symbolic.in"

[[graph.edges]]
from = "hevm-symbolic.out"
to = "certora-formal.in"
```

### Pattern Generalization

The five-stage pattern (fast filter -> medium analysis -> deep proof) generalizes to any domain:

| Domain | Stage 1 (fast) | Stage 2 (medium) | Stage 3 (deep) |
|---|---|---|---|
| **Chain** | Heimdall decompile | Slither static analysis | Certora formal proof |
| **Code** | Clippy lint | `cargo test` | Property testing / Kani |
| **Data** | Schema validation | Statistical checks | Formal property verification |
| **Config** | TOML parse | Constraint checking | Model checking |

Each domain adds its own Verify Cells to the Pipeline. The Pipeline pattern is the same; only the Cells differ.

### MEV Protection as Pre-Flight Verify Cell

MEV (Maximal Extractable Value) protection is a pre-flight Verify Cell that simulates transactions before submission:

```rust
/// MEV protection: Verify Cell that simulates a transaction
/// before submission to detect sandwich, front-run, and
/// back-run vulnerabilities.
pub struct MevProtectionCell {
    /// Maximum acceptable price impact in basis points.
    max_impact_bps: u32,
    /// Simulation backend (e.g., local fork, Flashbots simulator).
    simulator: Box<dyn TransactionSimulator>,
}

impl VerifyCell for MevProtectionCell {
    fn name(&self) -> &str { "mev-protection" }

    async fn verify(
        &self,
        action: &Signal,
        _ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let tx_params = extract_transaction(action)?;

        // Simulate against local fork
        let simulation = self.simulator.simulate(&tx_params).await?;

        // Check for sandwich vulnerability
        if simulation.sandwich_risk_bps > self.max_impact_bps {
            return Ok(Verdict::reject(format!(
                "sandwich risk {}bps exceeds threshold {}bps",
                simulation.sandwich_risk_bps, self.max_impact_bps,
            )));
        }

        // Check overall price impact
        if simulation.total_impact_bps > self.max_impact_bps {
            return Ok(Verdict::reject(format!(
                "total impact {}bps exceeds threshold {}bps",
                simulation.total_impact_bps, self.max_impact_bps,
            )));
        }

        Ok(Verdict::pass(
            simulation.confidence,
            Evidence::MevSimulation {
                impact_bps: simulation.total_impact_bps,
                sandwich_risk: simulation.sandwich_risk_bps,
            },
        ))
    }
}
```

---

## 6. Universal Enforcement Path

The four cognitive kernel primitives compose into the defense-in-depth model:

```
Engram Syscalls (outermost): Verify Pipeline wraps every action
  |
  Cognitive Namespaces: Space isolation + Store partitions
    |
    Cognitive Scheduling: EDF + budget limits on Graph executor
      |
      Cognitive Signals: Pulse-based human intervention
```

The existing `SafetyLayer` in `crates/roko-agent/src/safety/mod.rs` is a composite Verify Cell that chains BashPolicy, GitPolicy, NetworkPolicy, PathPolicy, ScrubPolicy, and RateLimiter. The cognitive kernel vision extends this to cover all agent actions, not just tool invocations.

The critical gap (see [02-defense-in-depth-as-pipeline.md](02-defense-in-depth-as-pipeline.md) SS6): the SafetyLayer is wired into routed/provider-backed execution paths but not universally from all subprocess branches. Closing this gap makes the Engram Syscall pattern effective.

---

## What This Enables

1. **OS-grade isolation**: Cognitive Namespaces provide knowledge isolation equivalent to Linux namespaces. A compromised agent cannot access another agent's knowledge.
2. **Non-destructive human control**: Cognitive Signals provide EU AI Act-compliant human oversight without state loss. Every intervention is graceful.
3. **Fair resource allocation**: EDF scheduling prevents starvation and priority inversion. Safety-critical tasks always execute on time.
4. **Domain-extensible verification**: The formal verification Pipeline pattern accommodates any domain's proof tools as Verify Cells.
5. **Single enforcement point**: All actions pass through the Verify Pipeline. No bypass path exists (once the integration gap is closed).

## Feedback Loops

- **L1**: Namespace access patterns inform channel policy. If Agent A never reads Agent B's shared knowledge, the channel can be narrowed.
- **L2**: Signal delivery metrics (time from send to acknowledge) feed health observation. Slow acknowledgment indicates agent overload.
- **L3**: Scheduling efficiency (ratio of deadline-met to deadline-missed) feeds the cascade router. Tasks consistently missing deadlines on a model/agent get rerouted.
- **Memory**: Formal verification results are stored as Signals. A contract that passed Certora verification at time T1 does not need re-verification at T2 unless the contract changed.

## Open Questions

1. **Namespace granularity**: Should every agent have its own namespace, or can agents in the same Group share a namespace? Sharing reduces channel overhead but weakens isolation.

2. **Signal priority conflicts**: What happens when two operators send conflicting signals (one sends Escalate, another sends Cooldown)? The current design uses strict priority ordering, which means Escalate (priority 3) would process before Cooldown (priority 4). But the operator intent is conflicting.

3. **EDF overhead**: EDF scheduling is optimal for deadline-driven workloads but adds overhead for workloads without deadlines. Should scheduling be adaptive -- EDF when deadlines exist, FIFO otherwise?

4. **Formal verification latency**: Certora can take 10+ minutes per verification. For an agent making rapid decisions, this is too slow. Should formal verification run asynchronously (verify while executing under temporary approval, revoke if verification fails)?

5. **Cross-domain verification**: Can the Pipeline pattern compose verification across domains? E.g., a transaction that requires both chain formal verification (Certora) and code safety (clippy + tests) before execution.

## Implementation Tasks

| Task | File | What |
|---|---|---|
| Implement Space partitions for Store | `crates/roko-fs/src/` | Add partition support to FileSubstrate (namespace-scoped queries) |
| Implement SpaceChannel with rate limiting | `crates/roko-core/src/` | Add cross-Space channel with Kind filtering and audit logging |
| Implement CognitiveSignalHandler React Cell | `crates/roko-runtime/src/` | Express signal delivery as Bus subscription with priority queue |
| Wire signal timeouts and escalation | `crates/roko-runtime/src/` | Implement Cooldown -> Pause -> Shutdown escalation chain |
| Implement EDF scheduler | `crates/roko-runtime/src/` | Add EDF scheduling to Graph executor with budget tracking |
| Add formal verification Pipeline (chain) | `crates/roko-chain/src/` | Wire Heimdall/Slither/Echidna/hevm/Certora as Verify Cells |
| Add MEV protection Verify Cell | `crates/roko-chain/src/` | Implement pre-flight transaction simulation |
| Integration test: namespace isolation | `crates/roko-core/tests/` | Verify Agent A cannot read Agent B's Store partition without channel |
| Integration test: signal escalation | `crates/roko-runtime/tests/` | Send Pause, wait timeout, verify Shutdown escalation |
