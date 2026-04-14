# Git as Stigmergy: Version Control as a Coordination Medium

> **Layer**: L0 Runtime (file system events), L2 Scaffold (context assembly from repository
> state), L4 Orchestration (multi-agent worktree coordination)
>
> **Synapse traits**: `Substrate` (the repository as a persistent store), `Scorer` (evaluating
> code quality signals), `Gate` (CI/CD as verification), `Policy` (reacting to repository
> events)
>
> **Prerequisites**: `00-stigmergy-theory.md` (stigmergy fundamentals),
> `01-stigmergy-beyond-termites.md` (generalized stigmergy)


> **Implementation**: Specified

---

## The Repository as a Stigmergic Environment

A Git repository is the most natural stigmergic environment for software development agents.
It satisfies all three conditions for stigmergy identified by Grassé (1959) and formalized by
Theraulaz & Bonabeau (1999):

| Condition | Git Implementation |
|-----------|-------------------|
| **Shared environment** | The repository (working tree + object store + refs) accessible to all agents |
| **Persistent modifications** | Commits persist indefinitely; branches create named trails |
| **Stimulus-response coupling** | Diff output, test results, linting warnings, and merge conflicts trigger agent actions |

Every agent working in a Roko-managed repository reads from and writes to this shared
environment. No agent needs to communicate directly with any other agent — the repository
state itself is the coordination medium.

---

## Sematectonic Stigmergy in Git

Sematectonic stigmergy (structure-based coordination) is pervasive in Git repositories. The
code structure guides agent behavior without any explicit signaling.

### Code Structure as Signal

When an agent reads a repository, the structure it encounters is a rich source of coordination
signals:

| Structural Feature | Information Conveyed | Agent Response |
|-------------------|---------------------|----------------|
| Module with no tests | "Testing gap" | Testing agent writes tests |
| Function with `TODO` comment | "Incomplete work" | Coding agent completes implementation |
| Trait with one implementor | "Abstraction may be premature" | Refactoring agent simplifies or adds second implementor |
| Error type with many variants | "Complex error domain" | Agent handles each variant explicitly |
| `pub` function with no doc comment | "Missing documentation" | Documentation agent adds rustdoc |
| `unsafe` block without `SAFETY:` comment | "Unjustified unsafety" | Safety agent adds justification or removes unsafe |
| Module with many imports | "High coupling" | Refactoring agent reduces dependencies |
| Binary file in repository | "Non-text artifact" | Agent skips or handles specially |

Each of these structural features is a "pheromone" that attracts specific agent behaviors. The
code itself recruits the right kind of work, just as a partially built termite arch recruits
mud-pellet deposits.

### File System Layout as Trail

The directory structure of a well-organized codebase creates navigation trails for agents:

```
crates/
├── roko-core/          ← "kernel code lives here"
│   ├── src/
│   │   ├── lib.rs      ← "start reading here"
│   │   ├── signal.rs   ← "the Engram type"
│   │   └── traits.rs   ← "the 6 Synapse traits"
│   └── Cargo.toml      ← "these are the dependencies"
├── roko-agent/         ← "agent dispatch lives here"
├── roko-gate/          ← "verification lives here"
└── ...
```

An agent exploring the codebase for the first time follows these structural cues to orient
itself — the same way an ant follows the geometry of a tunnel to navigate the nest. The layout
was not designed as a communication protocol, but it functions as one: it is sematectonic
stigmergy.

### Cargo.toml as Dependency Signal

In a Rust workspace, `Cargo.toml` files encode dependency relationships that guide agent
behavior:

- A crate that depends on `roko-core` signals "this crate uses the kernel API"
- A crate with `[dev-dependencies]` on `proptest` signals "property-based testing is expected
  here"
- A crate with `#![forbid(unsafe_code)]` signals "no unsafe code allowed — find safe
  alternatives"
- Feature flags in `Cargo.toml` signal optional capabilities that may need conditional
  compilation

These signals persist across all agents and all time. They are structural modifications to the
shared environment that guide future work — pure sematectonic stigmergy.

---

## Marker-Based Stigmergy in Git

Beyond the implicit signals of code structure, Git provides explicit marker mechanisms that
function as digital pheromones.

### Commit Messages as Trail Pheromones

Each commit message is a deliberate deposit of information into the shared environment:

```
feat(roko-agent): Wire CascadeRouter into dispatch pipeline

Connect the LinUCB-based model router to the agent dispatch path.
Models are selected per-request based on task complexity and
historical performance. Fallback to config default on cold start.

Refs: Lee et al. 2026 (arXiv:2603.28052) for multi-armed bandit
model routing.
```

This commit message functions as a trail pheromone:

1. **Type prefix** (`feat`) → signals what kind of change this is
2. **Scope** (`roko-agent`) → signals which subsystem was modified
3. **Description** → encodes the agent's intent and reasoning
4. **References** → links to the academic basis for the decision

An agent examining the commit history can follow these trails to understand how the codebase
arrived at its current state, which is essential for making decisions about future changes.

### Branch Names as Scoped Pheromones

Branch names create named trails through the repository's history:

| Branch Pattern | Signal | Scope |
|---------------|--------|-------|
| `feat/wire-cascade-router` | "New feature in development" | Feature scope |
| `fix/gate-threshold-overflow` | "Bug being fixed" | Bug scope |
| `refactor/agent-dispatch-cleanup` | "Structural improvement" | Refactoring scope |
| `release/v0.3.0` | "Preparing for release" | Release scope |
| `agent/task-42-implement-scorer` | "Agent working on specific task" | Task scope |

Branches are persistent markers that other agents can discover and reason about. When Roko's
orchestrator assigns tasks to agents, each agent works in its own worktree (Git worktree
feature), creating a branch that signals its active work to other agents.

### CI/CD Status as Environmental Feedback

Continuous integration results function as environmental feedback signals — modifications to
the shared environment that encode information about code quality:

| CI Signal | Pheromone Equivalent | Intensity |
|----------|---------------------|-----------|
| All tests pass (green) | `Opportunity` — "this code is safe to build on" | High |
| Clippy warnings | `Pattern` — "code quality issue detected" | Medium |
| Test failure | `Threat` — "regression detected" | High |
| Coverage decrease | `Anomaly` — "testing gap widened" | Medium |
| Build failure | `Threat` — "broken build" | Very High |

In Roko's gate pipeline (`roko-gate` crate), these CI results are explicitly converted into
scored Engrams that enter the stigmergic loop. A `Threat` Engram from a failing test triggers
remediation behavior in subsequent agents.

### Git Blame as Historical Pheromone Map

`git blame` provides a historical pheromone map: it shows which agent (or human) last modified
each line of code, and when. This is analogous to a pheromone concentration map where:

- Recently modified lines have "fresh" pheromone (recent activity signal)
- Lines untouched for months have "decayed" pheromone (stable code signal)
- Lines modified by many different agents have "mixed" pheromone (contested code signal)

An agent can use blame data to identify:

- **Hot spots**: Files or functions modified frequently (high pheromone concentration) —
  likely areas of active development or recurring bugs
- **Cold spots**: Files untouched for months — likely stable, well-understood code
- **Contested spots**: Lines modified by many different agents — possibly confusing or
  poorly abstracted code that attracts repeated changes

---

## Multi-Agent Worktree Model

Roko's multi-agent coordination uses Git worktrees as isolated stigmergic environments. Each
agent operates in its own worktree, which provides:

1. **Isolation**: Agents do not interfere with each other's uncommitted changes
2. **Parallel work**: Multiple agents can work on different branches simultaneously
3. **Merge as coordination**: When agents complete their work, merging their branches into
   the target branch is the coordination event

### Worktree as Private Pheromone Field

Each agent's worktree is a `Local(SubstrateId)` scope pheromone field. Changes the agent makes
are visible only within its worktree until the agent commits and pushes. This creates a natural
privacy boundary: work-in-progress modifications are "local pheromones" that do not affect
other agents.

### Merge Conflicts as Coordination Signals

When two agents' branches conflict during merge, the conflict markers are a particularly strong
stigmergic signal:

```
<<<<<<< HEAD
fn process_engram(engram: &Engram) -> Score {
    self.scorer.score(engram)
=======
fn process_engram(engram: &Engram) -> Result<Score> {
    self.scorer.score(engram).map_err(|e| ScoringError::from(e))
>>>>>>> agent-42-add-error-handling
```

This conflict encodes information: two agents made different design decisions about error
handling in the same function. The conflict must be resolved, and the resolution itself becomes
a new structural signal that guides future agents.

In Roko's orchestration pipeline (`roko-orchestrator`), merge conflicts trigger a specific
workflow:

1. The orchestrator detects the conflict during the merge queue step
2. A resolution agent is dispatched with context about both changes
3. The resolution agent reads both branches, understands the intent of each change, and
   produces a merged version
4. The merged version is committed, creating a new structural signal that reflects the
   collective decision

### The Base + Overlay Pattern

For multi-agent code indexing, Roko uses a base + overlay pattern (from `c05-multi-agent.md`):

- **Base index**: Read-only snapshot of the repository at the branch point. Shared by all
  agents working on the same plan.
- **Overlay index**: Per-agent additions from uncommitted changes in the agent's worktree.
  Invisible to other agents.

This pattern is stigmergic: the base index is the shared environment (all agents read from it),
and each agent's overlay is a local modification that becomes visible to others only when
committed (deposited into the shared environment).

### Declared Contracts

Agents working on related tasks can declare contracts — explicit promises about what their
branch will provide when merged:

```toml
# In the agent's task configuration
[contracts]
provides = ["trait ScorerV2", "fn score_with_context"]
requires = ["trait Substrate", "struct Engram"]
```

These contracts function as explicit pheromone deposits: they signal to other agents what new
affordances will become available when the branch merges. An agent waiting for `trait ScorerV2`
can proceed with its own work, confident that the dependency will be satisfied. This is
marker-based stigmergy — an explicit signal deposited in the coordination environment.

---

## Pheromone Traces in the Codebase

Roko coding agents are instructed to "leave PATTERN traces in the codebase" (from the agent
types specification in the refactoring PRD). These traces are explicit stigmergic markers
deposited during development:

### Types of Codebase Pheromone Traces

| Trace Type | Mechanism | Example | Half-Life |
|-----------|-----------|---------|-----------|
| **Test coverage** | Tests written by one agent guide other agents' confidence | `#[test] fn scorer_handles_empty_input()` | Permanent (sematectonic) |
| **Documentation** | Doc comments signal intent and usage | `/// Scores an Engram using LinUCB contextual bandit` | Permanent (sematectonic) |
| **Type signatures** | Types constrain and guide usage | `fn score(&self, engram: &Engram) -> Result<Score>` | Permanent (sematectonic) |
| **Error types** | Error variants document failure modes | `enum ScoringError { InvalidInput, ModelNotReady }` | Permanent (sematectonic) |
| **Feature flags** | Flags signal optional capabilities | `#[cfg(feature = "mesh-sync")]` | Permanent (sematectonic) |
| **Commit messages** | Trail markers in the version history | `fix(gate): Handle NaN scores in threshold comparison` | Permanent (marker) |
| **Pheromone Engrams** | Explicit typed signals in NeuroStore | `PheromoneKind::Pattern` with intensity and decay | Configurable (marker) |

### The Coding Agent's Stigmergic Behavior

When a Roko coding agent works on a task, it naturally operates as a stigmergic agent:

1. **Sense**: Read the repository state (code structure, tests, docs, CI status) to understand
   the current environment
2. **Act**: Modify code, write tests, add documentation — all modifications to the shared
   environment
3. **Deposit**: Commit changes with descriptive messages; optionally deposit explicit
   pheromone Engrams (e.g., `PheromoneKind::Pattern` noting "this module has been heavily
   refactored, downstream consumers should verify compatibility")
4. **Signal**: Push the branch, triggering CI (which produces environmental feedback signals)

The next agent to work in the same area of the codebase will encounter all of these traces
and be guided by them — without any direct communication between the two agents.

---

## Git as Roko's Primary Substrate

In the Synapse Architecture, Git functions as a specialized `Substrate` implementation. While
`roko-fs` provides the `FileSubstrate` for Engram persistence (JSONL files), Git provides a
higher-level Substrate for code artifacts:

| Substrate Operation | Git Implementation |
|--------------------|-------------------|
| `store(engram)` | `git add` + `git commit` (deposit a code modification) |
| `query(filter)` | `git log`, `git diff`, `git blame` (sense the environment) |
| `get(hash)` | `git show <hash>` (retrieve a specific modification) |
| `gc()` | `git gc` (compact the object store, prune unreachable objects) |

The content-addressing property of Git (every object is identified by its SHA-1/SHA-256 hash)
aligns with Engram's content-addressing property (`hash: [u8; 32]`). Both systems provide
tamper-evident, immutable records of modifications to the shared environment.

---

## Stigmergic Workflow Example

A concrete example of stigmergic coordination in a Roko-managed repository:

```
Timeline:
─────────────────────────────────────────────────────────────────

T=0   Agent A reads failing test in CI (SENSES Threat pheromone)
T=1   Agent A investigates → finds bug in scorer.rs
T=2   Agent A creates branch `fix/scorer-nan-handling`
T=3   Agent A commits fix with message:
      "fix(gate): Handle NaN scores in threshold comparison

       Scores from models that haven't warmed up can be NaN.
       Now clamps to 0.0 before comparison. Adds test case."
      (DEPOSITS marker-based pheromone: commit message)
T=4   Agent A pushes branch → CI runs → all tests pass
      (ENVIRONMENTAL FEEDBACK: Threat pheromone removed,
       Opportunity pheromone deposited)

T=5   Agent B reads repository for its task (model routing)
T=6   Agent B encounters Agent A's fix in recent commits
      (SENSES marker: "NaN handling added to scorer")
T=7   Agent B realizes its model routing code should also
      handle NaN scores from cold-start models
T=8   Agent B adds NaN handling to cascade router
      (STIGMERGIC RESPONSE: A's trace guided B's work)

T=9   Agent C reviews merged code for its documentation task
T=10  Agent C sees both NaN-handling implementations
      (SENSES sematectonic signal: pattern of NaN handling)
T=11  Agent C documents the NaN-handling convention in the
      contributing guide
      (DEPOSITS sematectonic pheromone: documentation)
```

No agent communicated directly with any other agent. The repository was the sole coordination
medium. Agent A's fix guided Agent B's implementation, which guided Agent C's documentation.
Complex coordinated behavior emerged from simple local interactions with the shared environment.

---

## References

- [Bolici et al. 2009] Scalability in OSS via stigmergy, *AMCIS Proceedings*
- [Dourish, P. "The Parrot's Tale." *Proceedings of ECSCW*, 2001] — Awareness in collaborative
  software development
- [Elliott, M. 2006] Stigmergic Collaboration, University of Melbourne
- [Fowler, M. 1999] *Refactoring: Improving the Design of Existing Code*, Addison-Wesley
- [Gibson, J.J. 1979] *The Ecological Approach to Visual Perception*, Lawrence Erlbaum
- [Grassé 1959] Termite mound stigmergy, *Insectes Sociaux*
- [Odling-Smee, Laland & Feldman 2003] *Niche Construction*, Princeton University Press
- [Pirolli & Card 1999] Information Foraging, *Psychological Review*
- [Theraulaz & Bonabeau 1999] History of Stigmergy, *Artificial Life*

---

## Cross-References

- `00-stigmergy-theory.md` — Core stigmergy definitions and theory
- `03-digital-pheromones.md` — The explicit pheromone system layered on top of Git
- `06-agent-mesh-sync.md` — How pheromone signals propagate beyond the local repository
- `07-morphogenetic-specialization.md` — How agents specialize into different roles
