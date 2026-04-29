# Stigmergic Coordination & Niche Construction

> **Theoretical basis**: `refactoring-prd/02-five-layers.md` §Stigmergy,
> `refactoring-prd/05-agent-types.md` §Niche Construction
> **Implementation**: Worktrees, merge queue, commit history, signal log,
> episode logger

---

## Overview

Roko's multi-agent orchestration is not a centralized command-and-control
system. It is a stigmergic system — agents coordinate indirectly through the
shared artifacts they produce. The codebase is the environment. Git commits
are pheromones. The merge queue is the integration point where individual
agent traces become collective knowledge.

This document explains the theoretical foundations and how they map to
Roko's concrete implementation.

---

## Stigmergy: Indirect Coordination

### Definition

Stigmergy is a mechanism of indirect coordination between agents where the
trace left in the environment by one agent stimulates the performance of a
subsequent action by another agent (Grassé 1959).

The term was coined by Pierre-Paul Grassé in 1959 to explain how termites
coordinate the construction of complex mound structures without central
planning or direct communication. Each termite deposits a pellet of mud. The
shape and pheromone of the deposit stimulates the next termite to deposit
nearby, creating self-organizing patterns.

### Digital stigmergy

In digital systems, stigmergy operates through shared computational
artifacts rather than physical structures (Parunak 2002). Roko implements
digital stigmergy through three channels:

#### 1. Git as the pheromone medium

Git commits are the digital equivalent of pheromone deposits:

| Termite behavior | Roko equivalent |
|-----------------|-----------------|
| Termite deposits mud pellet | Agent commits code to worktree branch |
| Pheromone on pellet attracts more building | Commit message, changed files, test results attract related work |
| Pheromone evaporates over time | Commit relevance decays (older commits less influential) |
| Colony structure emerges | Codebase architecture emerges |

When Agent A modifies `crates/roko-core/src/lib.rs` and commits the result,
Agent B (working on a related plan) may be dispatched to work on
`crates/roko-agent/src/pool.rs` — stimulated by the existence of Agent A's
changes. The DAG's file-conflict detection ensures they don't conflict, while
the merge queue serializes their integration.

#### 2. Signals as pheromone traces

The signal log (`.roko/signals.jsonl`) is a persistent pheromone field.
Agents produce signals (task completion, gate results, errors) that the
conductor monitors and other agents can consume:

| Signal type | Pheromone analogy |
|-------------|-------------------|
| `Task` | Construction deposit |
| `Metric` | Resource marker |
| `GateResult` | Quality indicator |
| `conductor:alert:*` | Alarm pheromone |

The conductor's `WatcherRunner` reads these signals every 30 seconds,
detecting patterns that no individual agent could see — cost trends,
failure rates, progress stalls.

#### 3. Knowledge as persistent pheromone

The knowledge store (`NeuroStore`) and skill library persist successful
patterns:

- When an agent successfully completes a task, its approach is extracted as a
  `Skill` — a reusable pattern that can be matched against future tasks
- When an agent fails, the failure pattern is recorded in the episode log
- Future agents receive this knowledge as context, biasing them toward
  successful approaches and away from known pitfalls

This is the digital equivalent of recruitment pheromones — successful paths
are reinforced, failed paths are avoided.

---

## Pheromone Types in Roko

The refactoring-prd (`02-five-layers.md`) defines a formal pheromone typology:

### By content

| Type | Meaning | Roko implementation |
|------|---------|---------------------|
| Threat | Danger signal — avoid this approach | Gate failure signals, conductor alerts |
| Opportunity | Resource availability — try this approach | Successful skill patterns, high-reward tasks |
| Wisdom | Accumulated knowledge — use this information | Knowledge store entries, playbook rules |

### By decay profile

| Profile | Half-life | Roko implementation |
|---------|-----------|---------------------|
| Alpha | Seconds–minutes | Real-time signals (context window pressure, cost) |
| Pattern | Hours–days | Episode patterns, gate threshold EMA |
| Anomaly | Days–weeks | Conductor alert history, failure patterns |
| Consensus | Weeks–months | Skills, playbooks, crate familiarity scores |

The decay profiles ensure that recent signals dominate immediate decisions
while long-term patterns inform strategic choices. This matches the Ebbinghaus
decay model used throughout Roko's knowledge management.

---

## Niche Construction

### Definition

Niche construction is the process by which organisms modify their own
selective environment (Odling-Smee, Laland & Feldman 2003). In evolutionary
biology, organisms don't just adapt to their environment — they actively
change it, creating feedback loops between agent and environment.

### Application to Roko

Roko agents construct the codebase they operate in. This creates a positive
feedback loop:

```
Agent writes code → Code structure changes → Future agents' task context changes
                                            → Future agents' available tools change
                                            → Future agents' difficulty changes
```

#### Positive niche construction

When agents improve the codebase:

- Adding well-structured modules makes future tasks easier
- Creating comprehensive tests provides safety nets for future modifications
- Writing clear documentation reduces future agent confusion
- Establishing consistent patterns makes pattern matching more effective

#### Negative niche construction

When agents degrade the codebase:

- Introducing technical debt makes future tasks harder
- Creating inconsistent naming confuses future pattern matching
- Leaving failed experiments pollutes the codebase
- Over-engineering increases cognitive load for future agents

### MVT stopping rule

The Marginal Value Theorem (Charnov 1976) provides a stopping rule for niche
construction: an agent should stop modifying its environment when the marginal
return of further modification drops below the expected return of moving to a
new task.

In Roko, this manifests as the gate pipeline: once an implementation passes
all gates (compile, test, clippy, verification), further modification is
unlikely to improve quality significantly. The agent moves to the next task.

### Affordance assessment

Before modifying the codebase, agents assess affordances — what actions the
current codebase state supports. The system prompt builder includes:

1. **Crate familiarity**: How well the system knows this crate (success rate
   from `CrateFamiliarityTracker`)
2. **Prior experience**: Successful patterns from the skill library
3. **Known pitfalls**: Failure patterns from the episode log
4. **Code context**: Existing code structure from read files

This affordance assessment is analogous to how organisms assess their
environment before deciding to modify it.

---

## C-Factor: Measuring Collective Intelligence

The C-Factor (Woolley et al. 2010) measures whether a group performs better
than the sum of its individual members:

```
C-Factor = Collective performance / Sum(Individual performance)
```

Roko computes this as `FleetCFactor` in the orchestration report:

```rust
pub struct FleetCFactor {
    pub cfactor: f64,
    pub individual_sum: f64,
    pub collective_score: f64,
}
```

A C-Factor > 1.0 means the multi-agent system outperforms individual agents
working separately. This is the hallmark of genuine collective intelligence —
the agents are not just parallelized, they are synergistic.

### What drives C-Factor > 1.0

1. **Complementary roles**: Strategist, Implementer, Auditor, and Scribe
   bring different capabilities. The combination catches errors that any
   single role would miss.

2. **Stigmergic amplification**: Agent A's successful pattern becomes
   Agent B's context. Knowledge compounds across agents.

3. **Parallel exploration**: Multiple agents explore different approaches
   simultaneously. The gate pipeline selects the successful ones.

4. **Error correction**: The Auditor role catches implementation errors
   before they merge. This correction is impossible in a single-agent system.

### The 31.6× calibration heuristic

The refactoring-prd (`09-innovations.md`) describes the 31.6× collective
calibration heuristic:

> Calibration improves as 1/sqrt(N×t) where N = number of agents and t = time
> steps. With N=100 agents and t=10 cycles, collective calibration is
> 1/sqrt(1000) ≈ 0.0316 — a 31.6× improvement over individual calibration.

This is a theoretical upper bound under ideal conditions. Actual C-Factor
depends on:

- Task decomposability (how independently tasks can be solved)
- Communication overhead (merge conflicts, re-planning costs)
- Role diversity (how different agent capabilities are)
- Knowledge sharing efficiency (how well learned patterns transfer)

---

## Ant Colony Optimization Parallels

Roko's multi-agent orchestration shares structural similarities with Ant
Colony Optimization (Dorigo & Gambardella 1997):

| ACO concept | Roko equivalent |
|-------------|-----------------|
| Ant colony | Agent collective |
| Pheromone trail | Commit history, signal log, skill library |
| Pheromone evaporation | Signal decay, Ebbinghaus forgetting, Engram half-life |
| Trail reinforcement | Skill extraction from successful tasks |
| Solution construction | Code changes accumulated across tasks |
| Colony convergence | Codebase convergence toward passing all gates |

The key difference is that ACO operates on a fixed graph (e.g., TSP), while
Roko's agents operate on a dynamic, high-dimensional space (the codebase).
The "graph" changes with every commit, and the "optimal solution" is defined
by the gate pipeline rather than a fixed objective function.

---

## Hauntology: Traces of Past Agents

The concept of hauntology (Derrida 1993) — the idea that the present is
always haunted by traces of the past — grounds Roko's approach to agent
memory. Every codebase carries traces of the agents that modified it:

- Commit messages document what was changed and why
- Code patterns reflect the approaches that succeeded
- Test suites encode the invariants that were established
- The episode log records the decisions that were made

Future agents work in an environment shaped by these traces. They don't start
from scratch — they build on (and are constrained by) the accumulated decisions
of all prior agents. This is niche construction in action: the present
environment is constructed by past agents and constrains future agents.

---

## References

- Grassé, P.-P. (1959). La reconstruction du nid et les coordinations
  interindividuelles chez Bellicositermes natalensis et Cubitermes sp.
  *Insectes Sociaux*, 6(1), 41–80. (Original stigmergy paper)
- Parunak, H. V. D. (2002). Digital pheromones for coordination of unmanned
  vehicles. *AAMAS 2002*. (Digital stigmergy)
- Dorigo, M. & Gambardella, L. M. (1997). Ant colony system: A cooperative
  learning approach to the traveling salesman problem. *IEEE Trans.
  Evolutionary Computation*, 1(1), 53–66.
- Odling-Smee, F. J., Laland, K. N. & Feldman, M. W. (2003). *Niche
  Construction: The Neglected Process in Evolution*. Princeton University
  Press.
- Woolley, A. W. et al. (2010). Evidence for a collective intelligence factor
  in the performance of human groups. *Science*, 330(6004), 686–688.
- Charnov, E. L. (1976). Optimal foraging, the marginal value theorem.
  *Theoretical Population Biology*, 9(2), 129–136.
- Derrida, J. (1993). *Specters of Marx: The State of the Debt, the Work of
  Mourning and the New International*. Routledge.
- Tomasello, M. (2014). *A Natural History of Human Thinking*. Harvard
  University Press. (Shared intentionality and collective cognition)
