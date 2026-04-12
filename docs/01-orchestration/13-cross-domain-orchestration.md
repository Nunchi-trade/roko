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
