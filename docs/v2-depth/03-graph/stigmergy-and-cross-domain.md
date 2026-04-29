# Stigmergy and Cross-Domain Orchestration

> Depth for [03-GRAPH.md](../../unified/03-GRAPH.md). How agents coordinate without direct communication via stigmergic Signals in shared Store, and how a single Graph spans multiple task domains.

---

## Problem

Multi-agent orchestration faces two coordination challenges:

1. **How do agents coordinate without direct communication?** Direct messaging creates coupling. Central dispatch creates bottlenecks. The alternative is stigmergy: agents coordinate through traces left in a shared environment.

2. **How does a single Graph span multiple domains?** A plan may include code tasks, research tasks, chain deployments, and documentation. Each domain has different agents, gates, and success criteria. The Graph must schedule them uniformly while allowing domain-specific behavior downstream.

---

## Stigmergy: Indirect Coordination

### The Mechanism

Stigmergy (Grasse 1959) is coordination through environmental traces. An agent modifies the shared environment; the modification stimulates action by other agents. No direct agent-to-agent communication required.

In Roko, the shared environment is the codebase + Store. The traces are:

| Trace Kind | Medium | Analogy |
|---|---|---|
| Git commits | Worktree branches | Construction deposits (termite mud pellets) |
| Signals in `.roko/signals.jsonl` | Store | Pheromone field |
| Episodes in `.roko/episodes.jsonl` | Store | Memory traces |
| Heuristics in neuro store | Store | Recruitment pheromones (successful path reinforcement) |

### Three Channels

#### 1. Git as Pheromone Medium

Git commits are the primary stigmergic trace:

| Termite Behavior | Roko Equivalent |
|---|---|
| Deposit mud pellet | Agent commits code to worktree branch |
| Pheromone attracts nearby building | Commit message + changed files attract related tasks |
| Pheromone evaporates | Commit relevance decays (older commits less influential) |
| Colony structure emerges | Codebase architecture emerges |

When Agent A modifies `crates/roko-core/src/lib.rs`, Agent B (working on a related plan) may be dispatched to `crates/roko-agent/src/pool.rs` -- stimulated by the existence of Agent A's changes. The DAG ensures no file conflicts; the merge queue serializes integration.

#### 2. Signals as Pheromone Traces

The Signal log is a persistent pheromone field:

| Signal Kind | Pheromone Analogy |
|---|---|
| `Task` | Construction deposit |
| `Metric` | Resource marker |
| `GateResult` | Quality indicator |
| `conductor:alert:*` | Alarm pheromone |

The Conductor's `WatcherRunner` reads these Signals every 30 seconds, detecting patterns invisible to individual agents: cost trends, failure rates, progress stalls.

#### 3. Knowledge as Persistent Pheromone

The neuro Store and Heuristic library persist successful patterns:

- Successful task completion -> extract approach as Heuristic (reusable pattern)
- Failed task -> record failure pattern in episodes
- Future agents receive this knowledge as context, biased toward success and away from known pitfalls

This is recruitment pheromone: successful paths are reinforced, failed paths are avoided.

### Pheromone Types

#### By Content

| Type | Meaning | Implementation |
|---|---|---|
| **Threat** | Danger -- avoid this approach | Gate failure Signals, conductor alerts |
| **Opportunity** | Resource available -- try this | Successful Heuristic patterns, high-reward tasks |
| **Wisdom** | Accumulated knowledge | Neuro store entries, Heuristic rules |

#### By Decay Profile

| Profile | Half-Life | Examples |
|---|---|---|
| **Alpha** | Seconds-minutes | Real-time Signals (context window pressure, cost) |
| **Pattern** | Hours-days | Episode patterns, gate threshold EMA |
| **Anomaly** | Days-weeks | Conductor alert history, failure patterns |
| **Consensus** | Weeks-months | Heuristics, crate familiarity scores |

Decay profiles ensure recent Signals dominate immediate decisions while long-term patterns inform strategic choices. This is demurrage (Gesell 1916) applied to coordination: use restores value, neglect erodes it.

---

## Niche Construction

Agents do not merely operate in their environment -- they construct it. This creates a feedback loop:

```
Agent writes code --> Code structure changes --> Future agents' context changes
                                                --> Future agents' tools change
                                                --> Future agents' difficulty changes
```

**Positive niche construction**: well-structured modules make future tasks easier. Comprehensive tests provide safety nets. Clear patterns enable pattern matching.

**Negative niche construction**: technical debt increases difficulty. Inconsistent naming confuses pattern matching. Failed experiments pollute the codebase.

### Marginal Value Theorem (Charnov 1976)

The MVT provides a stopping rule: an agent should stop modifying its environment when the marginal return drops below the expected return of moving to a new task.

In Roko, this manifests as the gate pipeline. Once an implementation passes all gates (compile, test, clippy, verify), further modification is unlikely to improve quality. The agent moves to the next task. The gate pipeline is the MVT boundary.

### Affordance Assessment

Before modification, agents assess affordances -- what actions the current codebase supports:

1. **Crate familiarity**: Historical success rate from `CrateFamiliarityTracker`
2. **Prior experience**: Successful patterns from the Heuristic library
3. **Known pitfalls**: Failure patterns from the episode log
4. **Code context**: Existing code structure from read files

This assessment is the agent's model of its environment, consulted before committing to action.

---

## C-Factor: Measuring Collective Intelligence

The c-factor (Woolley et al. 2010) measures whether a group outperforms the sum of its parts:

```
c-factor = collective_performance / sum(individual_performance)
```

A c-factor > 1.0 indicates genuine collective intelligence -- agents are synergistic, not merely parallelized.

### Drivers of c-factor > 1.0

1. **Complementary roles** -- Strategist, Implementer, Auditor, Scribe bring different capabilities. The combination catches errors any single role would miss.
2. **Stigmergic amplification** -- Agent A's successful pattern becomes Agent B's context. Knowledge compounds.
3. **Parallel exploration** -- Multiple agents explore different approaches simultaneously. The gate pipeline selects the successful ones.
4. **Error correction** -- The Auditor role catches implementation errors before merge. Impossible in single-agent systems.

### 31.6x Calibration Heuristic

Calibration improves as `1/sqrt(N * t)` where N = agents, t = time steps. With N=100 agents and t=10 cycles, collective calibration is `1/sqrt(1000) ~ 0.0316` -- a 31.6x improvement over individual calibration.

This is a theoretical upper bound under ideal conditions. Actual c-factor depends on task decomposability, communication overhead (merge conflicts, replanning), role diversity, and knowledge sharing efficiency.

---

## Cross-Domain Orchestration

### Single-DAG Principle

The `UnifiedTaskDag` does not distinguish between task types. Every task is a node with dependencies, file conflicts, and a topological position. Whether the task is Rust code, a smart contract deployment, or a research document, the DAG schedules it identically.

Differentiation happens downstream:

| Concern | Where Differentiation Occurs |
|---|---|
| Agent role / system prompt | RoleSystemPromptSpec in composition |
| Model tier | CascadeRouter in routing |
| Gate selection | Verify pipeline in gate dispatch |

This separation keeps the DAG simple (pure scheduling) while enabling arbitrarily complex per-task behavior.

### Domain Types

| Domain | Agent Role | Execution | Gates | Output |
|---|---|---|---|---|
| **Code** | Implementer | Claude CLI in worktree | Compile -> Test -> Clippy | Source files, commits |
| **Research** | Researcher | Claude CLI with research prompts | Format -> Citation -> Coherence | Documents, PRD enhancements |
| **Chain** (Phase 2+) | Chain operator | Korai node interaction | TypeCheck -> Simulation -> Security -> Deploy | On-chain state changes |
| **Documentation** | Scribe | Claude CLI with doc templates | Format -> LinkVerify | Markdown, API docs |

### Cross-Domain DAG Example

```toml
[[task]]
id = "research-stigmergy"
domain = "research"

[[task]]
id = "implement-pheromones"
domain = "code"
depends_on = ["research-stigmergy"]

[[task]]
id = "deploy-registry"
domain = "chain"

[[task]]
id = "wire-registry"
domain = "code"
depends_on = ["implement-pheromones", "deploy-registry"]
```

Scheduling: Wave 0 runs `research-stigmergy` + `deploy-registry` in parallel (no conflicts). Wave 1 runs `implement-pheromones`. Wave 2 runs `wire-registry`. Each task uses domain-specific agents and gates, but all flow through the same executor, merge queue, and recovery system.

### Gate Differentiation

Different domains require different Verify protocol implementations:

```
Code:     CompileGate -> TestGate -> ClippyGate -> (optional TaskVerify)
Research: FormatGate -> CitationGate -> CoherenceGate -> FactCheckGate
Chain:    TypeCheckGate -> SimulationGate -> SecurityGate -> DeploymentGate
```

The gate pipeline is trait-based (`Gate` trait from `roko-core`), allowing new gate types without modifying the orchestrator.

### Cross-Domain Conflict Resolution

**Prevention (preferred)**: The DAG's file-conflict inference serializes tasks touching the same files, regardless of domain.

**Semantic merge (when prevention fails)**: For structured files, textual merge often fails where semantic merge would succeed:

| File Type | Strategy |
|---|---|
| `*.toml` | TOML-aware key-value merge |
| `Cargo.lock` | Re-resolve dependencies (not textual merge) |
| `*.json` | Deep object/array merge |
| Other | Standard git textual merge |

Fallback when semantic merge fails: `PriorityWins`, `LastWriterWins`, `AgentResolve`, or `ManualResolve`.

### Cross-Domain Artifacts

Domains can have implicit dependencies where one domain produces an artifact another needs:

```rust
/// A cross-domain artifact produced by one task, consumed by another.
struct DomainArtifact {
    id: String,
    producer_domain: TaskDomain,
    producer_task: String,
    /// Filled after production.
    value: Option<serde_json::Value>,
    consumers: Vec<ArtifactConsumer>,
}
```

Artifacts are injected into consumers via environment variable, file path, or system prompt context. The executor waits for the producer to complete before dispatching consumers (late-binding dependency).

---

## Task Routing and Model Selection

The `CascadeRouter` selects models based on a multi-dimensional context vector:

```rust
struct RoutingContext {
    task_category: TaskCategory,     // Implementation, Research, Chain
    complexity: TaskComplexityBand,  // Fast, Standard, Complex
    iteration: u32,                  // retry count
    role: AgentRole,                 // domain-derived
    crate_familiarity: f64,          // historical success rate
    has_prior_failure: bool,         // failure escalation
    affect_confidence: f64,          // Daimon confidence
    previous_model: Option<String>,  // for escalation
}
```

The dual-process cascade:

```
T0 (no LLM) --> T1 (fast model) --> T2 (deep model)
```

- **T0**: 16 zero-LLM probes check if a task needs no model (file moves, template application). ~80% tier suppression on suitable tasks.
- **T1**: Cost-effective models for standard tasks.
- **T2**: Capability-maximizing models for complex tasks, architectural decisions, and tasks that failed on T1.

Domain affects routing: research tasks may benefit from stronger reasoning models; simple code tasks use faster models.

---

## Plan Repair

When gate feedback reveals a fundamentally flawed plan (not just a fixable error), the orchestrator invokes plan repair: a structured modification of the plan.

### Strategies

| Strategy | Based On | When |
|---|---|---|
| **Patch** | LPG-adapt (Gerevini 2004) | Modify only failing tasks + neighbors. Fast but may miss structural issues. |
| **Replan** | Full regeneration | Regenerate remaining plan from current state. Thorough but discards pending work. |
| **Hierarchical** | HTN (Erol 1994) | Decompose failing tasks into finer-grained subtasks. |
| **Adaptive** | Meta-reasoning | Choose strategy based on failure type. |

### Meta-Reasoning: Repair vs Replan

```
repair_cost = estimated_agent_calls * avg_cost_per_call
replan_cost = strategist_cost + new_plan_tasks * avg_cost_per_call
completed_work_value = completed_tasks * avg_task_value

if repair_cost < replan_cost - completed_work_value:
    repair (preserves completed work)
else:
    replan (fresh start cheaper)
```

Additional signals: 3+ consecutive failed repairs -> always replan. Completion ratio > 0.7 -> prefer repair (most work done). Structural failure (missing crate, wrong architecture) -> replan.

### Plan Abstraction Levels

| Level | Scope | Example |
|---|---|---|
| **Strategic** | Project-wide goals (PRDs) | "Achieve full self-hosting" |
| **Tactical** | Feature-level plans (tasks.toml) | "Wire SystemPromptBuilder" |
| **Operational** | Individual tasks (agent dispatches) | "Edit orchestrate.rs line 340" |

Failure escalation flows upward: operational failure -> tactical repair -> strategic replan.

---

## Saga Pattern for Irreversible Steps

When a cross-domain plan includes irreversible steps (e.g., deploy a contract AND wire it into code), failures require compensation rather than rollback:

```rust
struct SagaStep {
    action: TaskDef,
    /// Semantic undo. None if inherently reversible (e.g., git reset).
    compensation: Option<TaskDef>,
    status: SagaStepStatus,
}

enum SagaStepStatus {
    Pending,
    Succeeded,
    CompensationNeeded,
    Compensated,
    CompensationFailed,
}
```

Saga events are recorded in the hash-chained event log, enabling recovery of in-progress compensations after a crash. This follows Garcia-Molina & Salem (1987).

---

## What This Enables

1. **Decentralized coordination** -- Agents coordinate through Store artifacts (commits, Signals, Heuristics) without direct communication. This scales with agent count without communication overhead.
2. **Domain-agnostic scheduling** -- A single DAG handles code, research, chain, and documentation tasks with uniform scheduling and domain-specific execution.
3. **Self-repairing plans** -- Gate failures trigger automated plan repair, from local patches to full replanning, guided by meta-reasoning about cost vs benefit.
4. **Knowledge compounding** -- Successful patterns persist as Heuristics, biasing future agents toward proven approaches. Failed patterns persist as episodes, steering away from known pitfalls.

---

## Feedback Loops

1. **Stigmergic reinforcement**: Agent succeeds -> Heuristic extracted -> future agents use Heuristic -> higher success rate -> more Heuristic extraction. A positive feedback loop bounded by demurrage (unused Heuristics decay).

2. **Niche construction feedback**: Agent improves codebase -> future tasks easier -> agents succeed faster -> more improvements. Bounded by the MVT stopping rule (gate pipeline).

3. **Cross-domain artifact flow**: Chain task produces contract address -> code task wires it -> research task documents it. Each domain's output feeds the next domain's context. The DAG encodes these dependencies explicitly.

4. **Plan repair escalation**: Patch fails -> hierarchical decomposition -> replan -> strategic re-evaluation. Each level consumes more resources but has a wider solution space.

---

## Open Questions

1. **Stigmergic Signal types**: The current implementation uses untyped JSON payloads in the Signal log. A typed pheromone schema (threat/opportunity/wisdom with decay profiles) would enable richer coordination patterns. The `NeuroStore` supports typed knowledge entries, but the Signal log does not.

2. **C-factor computation**: The `FleetCFactor` struct exists in orchestration reporting, but the individual vs collective performance comparison is not well-defined. What counts as "individual performance"? A single agent running all tasks sequentially? The same agent with the same model? The definition affects whether c-factor > 1.0 is meaningful.

3. **Cross-domain gates not implemented**: Research gates (citation verification, coherence), chain gates (simulation, security audit), and documentation gates (link verification) are designed but not built. Only code gates (compile, test, clippy) are implemented.

4. **Saga coordinator not implemented**: The saga pattern for irreversible cross-domain steps (deploy + wire) is designed with full compensation support but not built. Current cross-domain plans rely on the DAG's dependency ordering without compensation for partial failures.

5. **Plan templates not implemented**: The template system for reusable domain-specific workflows is designed (including parameterization and cross-template composition) but not built. Plan generation currently relies on agent-driven task generation from PRDs.

6. **HEFT scheduling**: Full Heterogeneous Earliest Finish Time scheduling for multi-model dispatch is not implemented. The current approach uses simpler heuristics (priority ordering, arousal-based modulation) that approximate HEFT for the current scale.
