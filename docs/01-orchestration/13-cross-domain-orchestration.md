# Cross-Domain Orchestration

> **Design source**: `refactoring-prd/02-five-layers.md` §Cross-Domain
> Orchestration, `refactoring-prd/05-agent-types.md` §7 Multi-Agent
> Orchestration
> **Implementation**: `UnifiedTaskDag`, `PlanRunner`, `CascadeRouter`

---

## Overview

Roko is designed as a domain-agnostic agent toolkit. While its primary use case
today is self-hosting (developing its own codebase), the orchestration layer
supports tasks spanning multiple domains: code implementation, chain
operations, research, and documentation.

Cross-domain orchestration means running a single DAG that includes tasks of
different types, with different gates, different agent roles, and different
success criteria — coordinated through the same executor and merge queue.

---

## The Single-DAG Principle

The `UnifiedTaskDag` does not distinguish between task types. It treats every
task as a node with:

- Dependencies (intra-plan and cross-plan)
- File conflicts (for concurrent execution safety)
- A position in the topological order

Whether a task involves writing Rust code, deploying a smart contract, or
generating a research document, the DAG schedules it identically. The
differentiation happens downstream:

- **Agent role selection** determines the system prompt and tool set
- **Model routing** via `CascadeRouter` determines the model tier
- **Gate selection** determines the quality checks

This separation keeps the DAG simple (pure scheduling) while enabling
arbitrarily complex per-task behavior in the runtime.

---

## Domain Types

The Roko architecture supports multiple task domains through the domain plugin
system (`refactoring-prd/05-agent-types.md`):

### Code tasks

| Aspect | Details |
|--------|---------|
| Agent role | Implementer |
| Execution | Claude CLI in worktree |
| Gates | CompileGate → TestGate → ClippyGate |
| Output | Modified source files, commits |
| Success | All gates pass |

Code tasks are the primary domain today. They represent source code
modifications, test additions, configuration changes, and documentation
updates within the Rust workspace.

### Chain tasks (Phase 2+)

| Aspect | Details |
|--------|---------|
| Agent role | Chain operator |
| Execution | Korai node interaction |
| Gates | Transaction verification, state proof |
| Output | On-chain state changes |
| Success | Transaction confirmed |

Chain tasks involve interactions with the Korai blockchain — deploying
contracts, registering identities via ERC-8004, staking, and governance
operations. These are not yet implemented but the orchestrator's design
accommodates them.

### Research tasks

| Aspect | Details |
|--------|---------|
| Agent role | Researcher |
| Execution | Claude CLI with research prompts |
| Gates | Citation verification, coherence check |
| Output | Research documents, PRD enhancements |
| Success | Document quality gate |

Research tasks produce knowledge artifacts — literature surveys, PRD
enhancements, topic deep-dives. The `roko research` subcommands use
this domain.

### Documentation tasks

| Aspect | Details |
|--------|---------|
| Agent role | Scribe |
| Execution | Claude CLI with doc templates |
| Gates | Format check, link verification |
| Output | Markdown files, API docs |
| Success | Format and link gates pass |

Documentation tasks update project documentation to reflect code changes.
The DocRevision phase uses this domain automatically.

---

## Cross-Domain DAG Example

Consider a plan that combines code and research tasks:

```toml
# tasks.toml
[[task]]
id = "research-stigmergy"
title = "Research stigmergy in digital systems"
domain = "research"
tier = "focused"

[[task]]
id = "implement-pheromones"
title = "Implement pheromone system in roko-orchestrator"
domain = "code"
tier = "architectural"
depends_on = ["research-stigmergy"]

[[task]]
id = "deploy-registry"
title = "Deploy ERC-8004 identity registry"
domain = "chain"
tier = "standard"

[[task]]
id = "wire-registry"
title = "Wire ERC-8004 registry into agent mesh"
domain = "code"
tier = "standard"
depends_on = ["implement-pheromones", "deploy-registry"]
```

The DAG for this plan:

```
research-stigmergy ──► implement-pheromones ──► wire-registry
                                                    ▲
deploy-registry ────────────────────────────────────┘
```

The executor schedules this as:

- **Wave 0**: `research-stigmergy` + `deploy-registry` (no dependencies, no
  file conflicts — can run in parallel)
- **Wave 1**: `implement-pheromones` (depends on research completion)
- **Wave 2**: `wire-registry` (depends on both implementation and deployment)

Each task uses domain-specific agents, gates, and success criteria, but they
all flow through the same executor, merge queue, and crash recovery system.

---

## Task Routing and Model Selection

The `CascadeRouter` in `roko-learn` selects models based on a multi-dimensional
context vector that accounts for domain:

```rust
pub struct RoutingContext {
    pub task_category: TaskCategory,     // Implementation, Research, Chain, etc.
    pub complexity: TaskComplexityBand,  // Fast, Standard, Complex
    pub iteration: u32,                  // retry count
    pub role: AgentRole,                 // domain-derived role
    pub crate_familiarity: f64,          // historical success rate
    pub has_prior_failure: bool,         // failure escalation
    pub affect_confidence: f64,          // Daimon confidence
    pub previous_model: Option<String>,  // for escalation
    pub plan_context_tokens: Option<usize>, // context size
}
```

The routing context feeds into a LinUCB bandit that balances exploration
(trying new model/role combinations) with exploitation (using proven
combinations). The dual-process cascade
(`refactoring-prd/02-five-layers.md`) governs escalation:

```
T0 (no LLM) → T1 (fast model) → T2 (deep model)
```

- **T0 probes**: 16 zero-LLM probes check if a task can be resolved without
  model invocation (e.g., simple file moves, template application). This
  achieves ~80% tier suppression on suitable tasks.
- **T1 fast**: Cost-effective models (Claude Haiku, Claude Sonnet) for
  standard tasks.
- **T2 deep**: Capability-maximizing models (Claude Opus) for complex tasks,
  architectural decisions, and tasks that failed on T1.

Model selection considers domain: research tasks may benefit from models with
stronger reasoning capabilities, while simple code tasks can use faster models.

---

## Gate Differentiation by Domain

Different domains require different quality gates:

### Code gates (current)

```
Rung 0: CompileGate   → cargo build --workspace
Rung 1: TestGate      → cargo test --workspace
Rung 2: ClippyGate    → cargo clippy --workspace --no-deps -- -D warnings
Rung 3: (optional)    → Task-level verify commands from tasks.toml
```

### Research gates (future)

```
Rung 0: FormatGate    → Markdown format validation
Rung 1: CitationGate  → Verify all citations are properly formatted
Rung 2: CoherenceGate → Check document structure and flow
Rung 3: FactCheckGate → Cross-reference claims with cited sources
```

### Chain gates (future)

```
Rung 0: TypeCheckGate    → Solidity/Vyper type checking
Rung 1: SimulationGate   → Mirage (in-process EVM) simulation
Rung 2: SecurityGate     → Automated audit (reentrancy, overflow, etc.)
Rung 3: DeploymentGate   → Testnet deployment and verification
```

The gate pipeline (`roko-gate`) is designed as a trait-based system where new
gate types can be added without modifying the orchestrator. Each gate
implements the `Gate` trait from `roko-core`:

```rust
pub trait Gate: Send + Sync {
    fn check(&self, signal: &Signal, ctx: &Context) -> Verdict;
}
```

---

## Multi-Plan Cross-Domain Coordination

When multiple plans span different domains, the `UnifiedTaskDag` and
`MergeQueue` coordinate across all of them:

### File-conflict detection across domains

Code tasks and chain tasks may both modify configuration files (e.g.,
`roko.toml`, `Cargo.toml`). The DAG's file-overlap inference prevents
concurrent modification regardless of domain.

### Dependency chains across domains

A chain task can depend on a code task (e.g., deploy a contract after the
contract code is compiled), and a code task can depend on a chain task (e.g.,
generate bindings after a contract is deployed). The DAG handles these
cross-domain dependencies identically to same-domain dependencies.

### Merge serialization across domains

The merge queue serializes all plan merges regardless of domain. A research
document and a code implementation can merge in parallel (different files),
but two code plans touching the same crate are serialized.

---

## Agent Pool and HEFT Scheduling

The refactoring-prd (`05-agent-types.md`, §7) describes HEFT-like scheduling
for multi-agent dispatch:

> The agent pool (per collective) uses HEFT-like scheduling:
> estimate finish time per task considering (a) agent capability, (b) task
> complexity, (c) current load.

In the current implementation, the `PlanRunner` approximates HEFT scheduling
through:

1. **Capability estimation**: Model routing considers task complexity and domain
2. **Complexity estimation**: Task tier (mechanical/fast/focused/architectural/complex)
3. **Load estimation**: `max_concurrent_tasks` limits bound total parallelism

The HEFT algorithm (Topcuoglu et al. 2002) is a list scheduling heuristic
for heterogeneous computing environments. It computes the Earliest Finish
Time (EFT) for each task on each available processor, then assigns tasks
to the processor that minimizes EFT. In Roko, "processors" are model/role
combinations, and "tasks" are plan tasks with complexity estimates.

Full HEFT implementation is a future enhancement. The current approach uses
simpler heuristics (priority ordering, arousal-based modulation) that achieve
similar effects for the current scale of operations.

---

## Spore / Sparrow Job Market (Future)

The refactoring-prd describes two job market protocols for cross-domain
task distribution:

### Spore

`BountySpec` — a standardized task description that can be published to the
agent mesh for discovery and bidding.

### Sparrow

Power-of-two-choices dispatch (Ousterhout 2013) — instead of assigning a task
to the globally optimal agent, sample two random agents and assign to the less
loaded one. This achieves near-optimal load balancing with O(1) scheduling
overhead.

These protocols enable cross-collective task distribution, where tasks from
one collective can be fulfilled by agents from another collective, creating
a marketplace for computational work.

---

## Vickrey Reputation-Adjusted Auction (Future)

For cross-domain tasks where multiple agents could fulfill the work, a
Vickrey (second-price) auction with reputation adjustment determines the
winning bid:

```
s_i = p_i × (1 + (1 - R_i))
```

Where:
- `p_i` = agent's bid price
- `R_i` = agent's reputation score (0 to 1)
- `s_i` = adjusted score

Payment = `s_second / (1 + (1 - R_winner))`

This mechanism incentivizes truthful bidding (Vickrey property) while
favoring reputable agents (reputation adjustment). Low-reputation agents
must bid lower to compete, while high-reputation agents can charge a premium.

---

---

## Choreography vs Orchestration Patterns

The Roko executor is a **centralized orchestrator** — the `ParallelExecutor`
state machine controls the sequence of operations. However, some cross-domain
workflows may benefit from choreographic elements where domains react to events
autonomously.

### Pattern Comparison

| Aspect | Orchestration (current) | Choreography (future) |
|--------|------------------------|----------------------|
| **Control** | Central coordinator (`PlanRunner`) | Each domain reacts to events |
| **Coupling** | Domains coupled to executor | Domains coupled only to events |
| **Observability** | Full visibility in one place | Distributed traces needed |
| **Error handling** | Centralized retry/compensation | Per-domain saga compensations |
| **Scalability** | Bottleneck at coordinator | Scales with domains |
| **Complexity** | Simple flow, complex coordinator | Simple coordinators, complex flow |

### Saga Pattern for Cross-Domain Transactions

When a cross-domain plan involves irreversible steps (e.g., deploy a contract
AND wire it into code), failures require **compensation** rather than rollback:

```rust
/// A saga step with forward action and compensating action.
pub struct SagaStep {
    /// The forward transaction.
    pub action: TaskDef,
    /// The compensating transaction (semantic undo).
    /// None if the step is inherently reversible (e.g., code change in
    /// a worktree — just git reset).
    pub compensation: Option<TaskDef>,
    /// Status tracking.
    pub status: SagaStepStatus,
}

pub enum SagaStepStatus {
    Pending,
    Succeeded,
    CompensationNeeded,
    Compensated,
    CompensationFailed,
}

/// Saga Execution Coordinator — manages the forward/compensate flow.
pub struct SagaCoordinator {
    pub saga_id: String,
    pub state: SagaState,
    pub steps: Vec<SagaStep>,
    pub current_step: usize,
    /// Durable event log for saga recovery.
    pub log: Vec<SagaEvent>,
}

pub enum SagaState {
    Running,
    Compensating,
    Completed,
    Failed,
}

/// Saga events for durable logging (enables recovery after crash
/// during compensation).
pub enum SagaEvent {
    BeginSaga,
    BeginStep(usize),
    EndStep(usize),
    BeginCompensation(usize),
    EndCompensation(usize),
    EndSaga,
}
```

The saga coordinator integrates with the existing `EventLog` — saga events are
recorded alongside orchestration events in the hash chain, enabling recovery
of in-progress compensations after a crash.

### Hybrid Approach: Orchestrated Choreography

Roko can combine both patterns using Temporal's approach: the executor
orchestrates the high-level plan flow, while individual domains use
event-driven choreography for intra-domain coordination:

```
Executor (orchestration)
  ├── Code domain (orchestrated: specific task order)
  ├── Chain domain (choreography: react to on-chain events)
  └── Research domain (choreography: react to citation discoveries)
```

---

## Domain-Specific Plan Templates

Plan templates are reusable workflow fragments that encode domain-specific
best practices. They compose into complete cross-domain plans.

### Template System

```rust
/// A reusable plan template for a specific domain.
pub struct PlanTemplate {
    /// Unique template identifier.
    pub id: String,
    /// Domain this template applies to.
    pub domain: TaskDomain,
    /// Semantic version for backwards compatibility.
    pub version: semver::Version,
    /// Template parameters (filled in at instantiation).
    pub parameters: Vec<TemplateParameter>,
    /// Task definitions with parameter placeholders.
    pub tasks: Vec<TemplateTask>,
    /// Gate configuration for this domain.
    pub gates: Vec<GateConfig>,
    /// Dependencies on other templates (composable).
    pub requires: Vec<TemplateDependency>,
}

pub struct TemplateParameter {
    pub name: String,
    pub param_type: ParameterType,
    pub default: Option<String>,
    pub required: bool,
    pub description: String,
}

pub enum ParameterType {
    String,
    Path,
    CrateName,
    ContractAddress,
    Url,
}

/// A task within a template, with parameter placeholders.
pub struct TemplateTask {
    pub id_pattern: String,      // e.g., "impl-{{crate_name}}"
    pub title_pattern: String,   // e.g., "Implement {{feature}} in {{crate_name}}"
    pub domain: TaskDomain,
    pub tier: String,
    pub depends_on: Vec<String>, // can reference other template tasks
    pub files_pattern: Vec<String>, // e.g., "crates/{{crate_name}}/src/**"
}

impl PlanTemplate {
    /// Instantiate a template with concrete parameter values.
    /// Returns a list of concrete TaskDef entries.
    pub fn instantiate(
        &self,
        params: &HashMap<String, String>,
    ) -> Result<Vec<TaskDef>, TemplateError> { /* ... */ }

    /// Compose two templates: merge their tasks, resolve cross-template
    /// dependencies, and validate the combined DAG.
    pub fn compose(
        &self,
        other: &PlanTemplate,
        binding: &CompositionBinding,
    ) -> Result<PlanTemplate, TemplateError> { /* ... */ }
}
```

### Built-in Templates

| Template | Domain | Tasks | Description |
|----------|--------|-------|-------------|
| `rust-feature` | Code | 5 | Add feature: implement, test, document, gate, review |
| `rust-refactor` | Code | 4 | Refactor: analyze, implement, verify, review |
| `research-topic` | Research | 3 | Research: survey, synthesize, cite-check |
| `chain-deploy` | Chain | 4 | Deploy: compile, simulate, deploy-testnet, verify |
| `full-feature` | Cross-domain | 8+ | Research → implement → test → deploy → document |

Templates are stored in `.roko/templates/` and versioned. The `roko prd plan`
command can select appropriate templates based on PRD content analysis.

---

## Cross-Domain Conflict Resolution

When two domains modify the same artifact (e.g., both code and chain tasks
update `roko.toml`), conflicts must be detected and resolved.

### Conflict Prevention (Preferred)

Prevention is cheaper than resolution for agent systems. The existing
`UnifiedTaskDag` file-conflict inference already serializes tasks that touch
the same files, regardless of domain. This prevents most conflicts.

### Semantic Merge (When Prevention Fails)

For artifacts with domain-specific structure (TOML, Cargo.lock, Solidity ABIs),
textual merge often fails where semantic merge would succeed:

```rust
/// Domain-specific merge strategies.
pub enum MergeStrategy {
    /// Standard git textual merge (default).
    Textual,
    /// TOML-aware merge: merge at the key-value level rather than
    /// line level. Handles concurrent additions to different sections.
    TomlSemantic,
    /// Cargo.lock merge: re-resolve dependencies rather than
    /// merging the lock file textually.
    CargoLockResolve,
    /// JSON merge: deep merge at the object/array level.
    JsonDeep,
    /// Domain-specific custom merge function.
    Custom(Box<dyn Fn(&str, &str, &str) -> Result<String, MergeError>>),
}

/// Resolution strategies when semantic merge fails.
pub enum ConflictResolution {
    /// Favor the higher-priority plan's version.
    PriorityWins,
    /// Favor the more recent change (LWW).
    LastWriterWins,
    /// Delegate to an agent to manually resolve.
    AgentResolve { role: AgentRole },
    /// Fail and require operator intervention.
    ManualResolve,
}

/// Configuration per file pattern.
pub struct MergeConfig {
    /// Glob pattern matching files (e.g., "*.toml", "Cargo.lock").
    pub pattern: String,
    /// Merge strategy for matching files.
    pub strategy: MergeStrategy,
    /// Fallback resolution when strategy fails.
    pub fallback: ConflictResolution,
}

/// Default merge configurations.
pub fn default_merge_configs() -> Vec<MergeConfig> {
    vec![
        MergeConfig {
            pattern: "Cargo.lock".into(),
            strategy: MergeStrategy::CargoLockResolve,
            fallback: ConflictResolution::AgentResolve {
                role: AgentRole::AutoFixer,
            },
        },
        MergeConfig {
            pattern: "*.toml".into(),
            strategy: MergeStrategy::TomlSemantic,
            fallback: ConflictResolution::PriorityWins,
        },
        MergeConfig {
            pattern: "*.json".into(),
            strategy: MergeStrategy::JsonDeep,
            fallback: ConflictResolution::LastWriterWins,
        },
    ]
}
```

### Cross-Domain Dependency Protocols

When domains have implicit dependencies (e.g., code depends on a deployed
contract address, but the address is only known after deployment):

```rust
/// A cross-domain artifact that one domain produces and another consumes.
pub struct DomainArtifact {
    /// Unique artifact identifier.
    pub id: String,
    /// The domain that produces this artifact.
    pub producer_domain: TaskDomain,
    /// The task that produces it.
    pub producer_task: String,
    /// The value (filled in after production).
    pub value: Option<serde_json::Value>,
    /// Consumers waiting for this artifact.
    pub consumers: Vec<ArtifactConsumer>,
}

pub struct ArtifactConsumer {
    pub domain: TaskDomain,
    pub task_id: String,
    /// How the artifact is injected into the consumer's context.
    pub injection: ArtifactInjection,
}

pub enum ArtifactInjection {
    /// Set as an environment variable.
    EnvVar(String),
    /// Write to a file path.
    FilePath(PathBuf),
    /// Include in the agent's system prompt.
    PromptContext,
}
```

This enables late-binding dependencies: a code task can declare it needs a
`contract_address` artifact from a chain task, and the executor will wait
for the chain task to produce it before dispatching the code task.

---

## Plan Repair: Self-Modifying Plans

When gate feedback reveals that a plan is fundamentally flawed (not just a
fixable compilation error), the orchestrator can invoke **plan repair** — a
structured modification of the plan based on automated planning techniques.

### Plan Repair Engine

Drawing on AI planning research (STRIPS, PDDL, LPG-adapt) and HTN
(Hierarchical Task Network) decomposition:

```rust
/// The plan repair engine modifies a failing plan based on gate feedback.
pub struct PlanRepairEngine {
    /// Maximum repair attempts before declaring failure.
    /// Default: 3. Range: 1..=5.
    pub max_repairs: u32,
    /// Repair strategy selection.
    pub strategy: RepairStrategy,
}

pub enum RepairStrategy {
    /// Patch: modify only the failing tasks and their immediate neighbors.
    /// Fastest, but may miss structural issues.
    /// Inspired by LPG-adapt (Gerevini et al., 2004).
    Patch,
    /// Replan: regenerate the entire remaining plan from the current state.
    /// Most thorough, but discards work on pending tasks.
    Replan,
    /// Hierarchical: decompose failing tasks into subtasks at a finer grain.
    /// Inspired by HTN planning (Erol et al., 1994).
    Hierarchical,
    /// Adaptive: choose strategy based on failure type.
    Adaptive,
}

/// A repair action produced by the repair engine.
pub enum RepairAction {
    /// Replace a failing task with a revised version.
    ReviseTask { task_id: String, new_def: TaskDef },
    /// Decompose a task into subtasks.
    DecomposeTask { task_id: String, subtasks: Vec<TaskDef> },
    /// Add a prerequisite task (missing dependency discovered).
    AddPrerequisite { before: String, new_task: TaskDef },
    /// Remove an infeasible task and adjust dependencies.
    RemoveInfeasible { task_id: String },
    /// Escalate: the plan needs fundamental restructuring.
    /// Triggers `roko prd plan <slug>` to regenerate from the PRD.
    Escalate { reason: String },
}

impl PlanRepairEngine {
    /// Analyze gate failures and produce repair actions.
    ///
    /// Algorithm:
    /// 1. Classify failure type:
    ///    - Compilation error → Patch (fix the specific code)
    ///    - Test failure → Patch or Hierarchical (may need more steps)
    ///    - Multiple related failures → Hierarchical (structural issue)
    ///    - 3+ consecutive failures → Escalate (fundamental problem)
    ///
    /// 2. Generate repair actions based on strategy:
    ///    - Patch: use AutoFixer agent to propose task revision.
    ///    - Hierarchical: use Strategist agent to decompose.
    ///    - Replan: invoke `roko prd plan` with current-state context.
    ///    - Adaptive: classify failure, pick best strategy.
    ///
    /// 3. Apply repairs via DagMutation operations.
    /// 4. Re-validate the modified DAG.
    pub fn repair(
        &self,
        plan_id: &str,
        failures: &[GateResult],
        dag: &mut UnifiedTaskDag,
    ) -> Result<Vec<RepairAction>, RepairError> { /* ... */ }
}
```

### Plan Abstraction Levels

Plans operate at three abstraction levels, inspired by military/business
planning hierarchies and ABSTRIPS (Sacerdoti, 1974):

| Level | Scope | Granularity | Example |
|-------|-------|-------------|---------|
| **Strategic** | Project-wide goals | PRDs, milestones | "Achieve full self-hosting" |
| **Tactical** | Feature-level plans | Plans with task lists | "Wire SystemPromptBuilder" |
| **Operational** | Individual tasks | Agent dispatches | "Edit orchestrate.rs line 340" |

```rust
/// A hierarchical plan with multiple abstraction levels.
pub struct HierarchicalPlan {
    /// Strategic goal (from PRD).
    pub goal: String,
    /// Tactical plans (decomposition of goal).
    pub plans: Vec<PlanInfo>,
    /// Refinement mapping: strategic → tactical → operational.
    pub refinements: HashMap<String, Vec<String>>,
}

impl HierarchicalPlan {
    /// Refine a strategic goal into tactical plans.
    /// Uses the Strategist agent to decompose.
    pub fn refine_strategic(&self, goal: &str) -> Vec<PlanInfo> { /* ... */ }

    /// Refine a tactical plan into operational tasks.
    /// Uses the TasksFile format with dependency resolution.
    pub fn refine_tactical(&self, plan_id: &str) -> Vec<TaskDef> { /* ... */ }

    /// When a tactical plan fails repair, escalate to strategic level:
    /// re-evaluate whether the goal decomposition is correct.
    pub fn escalate_to_strategic(
        &self,
        plan_id: &str,
        reason: &str,
    ) -> StrategicReplanAction { /* ... */ }
}
```

### Meta-Reasoning: When to Repair vs Replan

Drawing on continual planning literature (desJardins et al., 1999) and the
PRS (Procedural Reasoning System):

```rust
/// Decision function: should we repair the current plan or replan from scratch?
///
/// Heuristic:
///   repair_cost = estimated_agent_calls × avg_cost_per_call
///   replan_cost = strategist_cost + new_plan_tasks × avg_cost_per_call
///   completed_work_value = completed_tasks × avg_task_value
///
///   if repair_cost < replan_cost - completed_work_value:
///       → repair (preserves completed work)
///   else:
///       → replan (fresh start cheaper than patching)
///
/// Additional signals:
/// - If 3+ consecutive repairs failed → always replan
/// - If completion_ratio > 0.7 → prefer repair (most work done)
/// - If failure is structural (missing crate, wrong architecture) → replan
pub fn should_repair_or_replan(
    plan_state: &PlanState,
    failures: &[GateResult],
    efficiency_history: &[AgentEfficiencyEvent],
) -> PlanRecoveryDecision { /* ... */ }

pub enum PlanRecoveryDecision {
    Repair(RepairStrategy),
    Replan,
    Abort { reason: String },
}
```

---

## References

- Topcuoglu, H., Hariri, S. & Wu, M.-Y. (2002). Performance-effective and
  low-complexity task scheduling for heterogeneous computing. *IEEE Trans.
  Parallel and Distributed Systems*, 13(3), 260–274. (HEFT algorithm)
- Ousterhout, J. (2013). Sparrow: Distributed, low latency scheduling. *SOSP
  2013*. (Power-of-two-choices dispatch)
- Vickrey, W. (1961). Counterspeculation, auctions, and competitive sealed
  tenders. *Journal of Finance*, 16(1), 8–37. (Second-price auction theory)
- Hu, S. et al. (2025). Automated design of agentic systems. *ICLR 2025*.
  (ADAS — meta-agent architecture search, relevant to automatic task
  decomposition and role assignment)
- Lee, J. et al. (2026). FrugalGPT: How to use large language models while
  reducing cost and improving performance. *arXiv:2603.28052*. (Cost-efficient
  model routing, underpins the CascadeRouter)
- Garcia-Molina, H. & Salem, K. (1987). Sagas. *ACM SIGMOD 1987*. (Saga
  pattern for long-lived transactions with compensation.)
- Gerevini, A. et al. (2004). Planning through stochastic local search and
  temporal action graphs in LPG. *JAIR*, 20, 239–290. (LPG-adapt plan repair.)
- Sacerdoti, E. D. (1974). Planning in a hierarchy of abstraction spaces.
  *Artificial Intelligence*, 5(2), 115–135. (ABSTRIPS — abstraction
  hierarchies in automated planning.)
- Erol, K., Hendler, J. & Nau, D. S. (1994). HTN planning: Complexity and
  expressivity. *AAAI 1994*. (Hierarchical Task Network decomposition.)
- Fox, M. et al. (2006). Plan stability: Replanning versus plan repair.
  *ICAPS 2006*. (When repair beats replanning.)
- desJardins, M. E. et al. (1999). A survey of research in distributed,
  continual planning. *AI Magazine*, 20(4), 13–22. (Interleaving planning
  and execution, meta-reasoning about when to replan.)
