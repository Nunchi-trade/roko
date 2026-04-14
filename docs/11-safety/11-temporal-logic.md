# Temporal Logic Verification

> **Layer**: L3 Harness (runtime monitoring), L4 Orchestration (pre-execution verification)
>
> **Crate**: Target: `roko-gate` (temporal gates), `roko-conductor` (monitoring)
>
> **Synapse traits**: `Gate` (verify temporal properties), `Policy` (emit temporal violation Engrams)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [09-adaptive-risk.md](09-adaptive-risk.md)


> **Implementation**: Specified

---

## Overview

Temporal logic verification adds a time-aware dimension to safety checking. Standard safety checks verify individual actions ("is this bash command safe?"). Temporal logic verifies sequences of actions over time ("has this agent been escalating permissions over the last 10 minutes?" or "did this agent call git push without first calling the compile gate?").

Two temporal logics are used:

1. **LTL (Linear Temporal Logic)**: Monitors runtime behavior. Properties are checked against the stream of events as they occur. "Always" (G), "Eventually" (F), "Until" (U), "Next" (X).

2. **CTL (Computation Tree Logic)**: Verifies pre-execution plans. Properties are checked against the branching tree of possible execution paths. "For all paths" (A), "There exists a path" (E), combined with temporal operators.

---

## LTL Runtime Monitoring

### Buchi Automata

LTL formulas are compiled into Buchi automata — finite state machines that accept or reject infinite streams of events. The monitor tracks the automaton's state as events arrive and raises an alarm when the automaton enters a rejecting state (a state from which no accepting path exists).

### Safety Properties (Always/Never)

Safety properties specify what must always be true or must never happen:

```
G(tool_call(bash) → previous(safety_check))
    "Every bash tool call was preceded by a safety check"

G(git_push → previous(compile_gate_passed ∧ test_gate_passed))
    "Every git push was preceded by passing compile and test gates"

G(¬(write_file ∧ outside_worktree))
    "No write_file operation targets a path outside the worktree"

G(rate_limit_exceeded → ¬tool_call)
    "When rate limit is exceeded, no tool calls are dispatched"
```

### Liveness Properties (Eventually)

Liveness properties specify what must eventually happen:

```
G(task_started → F(task_completed ∨ task_failed))
    "Every started task eventually completes or fails"

G(gate_failed → F(retry ∨ skip ∨ abort))
    "Every gate failure is eventually followed by a retry, skip, or abort"

G(circuit_breaker_open → F(circuit_breaker_closed ∨ abort))
    "An open circuit breaker eventually closes or the session aborts"
```

### Fairness Properties

Fairness properties prevent starvation:

```
GF(idle_agent → scheduled)
    "An idle agent is infinitely often scheduled for work"

GF(queued_task → dispatched)
    "A queued task is infinitely often considered for dispatch"
```

---

## CTL Pre-Execution Verification

CTL verifies properties of execution plans before they run:

```
AG(EF(safe_state))
    "From any state in the plan, there exists a path to a safe state"
    (ensures no dead-end that traps the agent in an unsafe configuration)

AG(plan_node → AF(gate_check))
    "For all paths from any plan node, a gate check eventually occurs"
    (ensures no plan path bypasses verification)

¬EF(AG(¬terminated))
    "There is no path that leads to a state from which termination is
     impossible" (ensures the plan always terminates)
```

---

## BuchiAutomaton: full implementation

A Buchi automaton accepts or rejects infinite streams of events. Each LTL formula compiles to one automaton.

```rust
use std::collections::{HashMap, HashSet};

/// A state in the Buchi automaton.
pub type StateId = u32;

/// An atomic proposition (e.g., "bash_call", "safety_check", "gate_passed").
pub type Proposition = String;

/// A set of propositions that are true at a given time step.
pub type EventLabel = HashSet<Proposition>;

/// A compiled Buchi automaton for monitoring a single LTL formula.
pub struct BuchiAutomaton {
    /// Human-readable name of the formula being monitored.
    formula_name: String,
    /// The original LTL formula string (for diagnostics).
    formula_source: String,
    /// Number of states in the automaton.
    num_states: u32,
    /// Initial state.
    initial_state: StateId,
    /// Accepting states (the automaton must visit these infinitely often).
    accepting_states: HashSet<StateId>,
    /// Transition function: (current_state, event_label) -> set of next states.
    /// Nondeterministic: a single event can lead to multiple states.
    transitions: HashMap<(StateId, EventLabel), HashSet<StateId>>,
    /// Propositions this automaton cares about.
    alphabet: HashSet<Proposition>,
}

impl BuchiAutomaton {
    pub fn formula_name(&self) -> &str {
        &self.formula_name
    }

    /// Compute the set of next states given a current state and event.
    /// For nondeterministic automata, returns all reachable states.
    pub fn transition(
        &self,
        current: &AutomatonState,
        event: &EventLabel,
    ) -> AutomatonState {
        let mut next_states = HashSet::new();

        // Filter event to only propositions this automaton uses.
        let relevant: EventLabel = event
            .iter()
            .filter(|p| self.alphabet.contains(*p))
            .cloned()
            .collect();

        for &state in &current.active_states {
            if let Some(targets) = self.transitions.get(&(state, relevant.clone())) {
                next_states.extend(targets);
            }
        }

        // If no transitions matched, the automaton is stuck (rejecting).
        let in_accepting = next_states
            .iter()
            .any(|s| self.accepting_states.contains(s));

        AutomatonState {
            active_states: next_states,
            visited_accepting: current.visited_accepting || in_accepting,
            steps_since_accepting: if in_accepting {
                0
            } else {
                current.steps_since_accepting + 1
            },
        }
    }

    /// Check whether the automaton is in a rejecting configuration.
    ///
    /// A state is rejecting when:
    /// - The active state set is empty (no valid transitions exist), OR
    /// - The automaton has not visited an accepting state in
    ///   `liveness_bound` steps (bounded liveness check).
    pub fn is_rejecting(&self, state: &AutomatonState) -> bool {
        // Dead: no active states remain.
        if state.active_states.is_empty() {
            return true;
        }

        // Bounded liveness: if we haven't seen an accepting state
        // in too long, treat as a violation. This converts liveness
        // properties to safety properties with a finite bound.
        // Default bound: 1000 steps (~2.5 hours at gamma speed).
        let liveness_bound = 1000;
        state.steps_since_accepting > liveness_bound
    }
}

/// Runtime state of a monitored automaton.
#[derive(Debug, Clone)]
pub struct AutomatonState {
    /// Current active states (nondeterministic: may be multiple).
    pub active_states: HashSet<StateId>,
    /// Whether an accepting state has been visited at least once.
    pub visited_accepting: bool,
    /// Number of steps since the last visit to an accepting state.
    pub steps_since_accepting: u64,
}

impl AutomatonState {
    /// Create an initial state from the automaton's initial state.
    pub fn initial(automaton: &BuchiAutomaton) -> Self {
        let mut active = HashSet::new();
        active.insert(automaton.initial_state);
        Self {
            active_states: active,
            visited_accepting: automaton
                .accepting_states
                .contains(&automaton.initial_state),
            steps_since_accepting: 0,
        }
    }
}
```

### TemporalMonitor: full struct

```rust
/// Runtime temporal monitor. Runs all compiled LTL automata
/// against the stream of Engrams at each gamma tick.
pub struct TemporalMonitor {
    /// Compiled LTL formulas as Buchi automata.
    automata: Vec<BuchiAutomaton>,
    /// Current runtime state for each automaton.
    states: Vec<AutomatonState>,
    /// Event classifier: maps Engram kinds to propositions.
    classifier: EventClassifier,
    /// Liveness bound: max steps without visiting an accepting state.
    /// Default: 1000 (~2.5 hours at gamma speed).
    liveness_bound: u64,
    /// Total events processed (for diagnostics).
    events_processed: u64,
    /// Total violations detected.
    violations_detected: u64,
}

impl TemporalMonitor {
    pub fn new(liveness_bound: u64) -> Self {
        Self {
            automata: Vec::new(),
            states: Vec::new(),
            classifier: EventClassifier::default(),
            liveness_bound,
            events_processed: 0,
            violations_detected: 0,
        }
    }

    /// Add a compiled LTL formula to the monitor.
    pub fn add_formula(&mut self, automaton: BuchiAutomaton) {
        let state = AutomatonState::initial(&automaton);
        self.states.push(state);
        self.automata.push(automaton);
    }

    /// Process a batch of Engrams (one gamma tick's worth).
    /// Returns violation Engrams for any formulas that entered
    /// a rejecting state.
    pub fn process(&mut self, engrams: &[Signal]) -> Vec<Signal> {
        let mut violations = Vec::new();

        for engram in engrams {
            self.events_processed += 1;
            let event = self.classifier.classify(engram);

            for (i, automaton) in self.automata.iter().enumerate() {
                let new_state = automaton.transition(&self.states[i], &event);
                if automaton.is_rejecting(&new_state) {
                    self.violations_detected += 1;
                    violations.push(create_violation_engram(
                        automaton.formula_name(),
                        engram,
                    ));
                }
                self.states[i] = new_state;
            }
        }

        violations
    }

    /// Reset all automata to their initial states.
    /// Used after a Pause/Resume cycle.
    pub fn reset(&mut self) {
        for (i, automaton) in self.automata.iter().enumerate() {
            self.states[i] = AutomatonState::initial(automaton);
        }
    }
}

/// Maps Engram kinds and metadata to atomic propositions.
struct EventClassifier {
    /// Mapping from (Engram kind, optional tool name) to proposition set.
    rules: Vec<ClassificationRule>,
}

struct ClassificationRule {
    kind_match: Option<String>,    // Engram kind to match.
    tool_match: Option<String>,    // Tool name to match (for tool call Engrams).
    propositions: Vec<Proposition>, // Propositions to emit on match.
}
```

### LTL formula parser and compiler

The parser converts LTL formula strings into `BuchiAutomaton` instances using a two-stage pipeline:

```
"G(bash_call -> previous(safety_check))"
    |
    v
  [Parser] --> LtlAst
    |
    v
  [Compiler] --> BuchiAutomaton (via Gerth et al. 1995 algorithm)
```

**LTL AST:**

```rust
/// Abstract syntax tree for LTL formulas.
pub enum LtlAst {
    /// Atomic proposition: "bash_call", "gate_passed", etc.
    Atom(Proposition),
    /// Negation: not phi.
    Not(Box<LtlAst>),
    /// Conjunction: phi and psi.
    And(Box<LtlAst>, Box<LtlAst>),
    /// Disjunction: phi or psi.
    Or(Box<LtlAst>, Box<LtlAst>),
    /// Implication: phi -> psi (sugar for !phi || psi).
    Implies(Box<LtlAst>, Box<LtlAst>),
    /// Globally: G(phi) -- phi holds at every future step.
    Globally(Box<LtlAst>),
    /// Eventually: F(phi) -- phi holds at some future step.
    Eventually(Box<LtlAst>),
    /// Next: X(phi) -- phi holds at the next step.
    Next(Box<LtlAst>),
    /// Until: phi U psi -- phi holds until psi becomes true.
    Until(Box<LtlAst>, Box<LtlAst>),
    /// Previous: P(phi) -- phi held at the previous step (past-time LTL).
    Previous(Box<LtlAst>),
}
```

**Compiler algorithm** (Gerth, Peled, Vardi, Wolper 1995):

```
ltl_to_buchi(formula: LtlAst) -> BuchiAutomaton:
    # Step 1: Negate the formula (Buchi automaton accepts violations).
    negated = negate(formula)

    # Step 2: Convert to negation normal form (push negation inward).
    nnf = to_nnf(negated)

    # Step 3: Build generalized Buchi automaton (GBA) using the
    #         tableau construction. Each node in the tableau becomes
    #         a state. Transitions are generated by expanding temporal
    #         operators.
    gba = expand_tableau(nnf)

    # Step 4: Degenralize GBA to standard Buchi automaton.
    #         Uses the standard product construction with acceptance
    #         counter: k acceptance sets become k copies of the state
    #         space, cycling through acceptance sets.
    ba = degeneralize(gba)

    # Step 5: Minimize (optional, improves runtime monitoring speed).
    #         Remove unreachable states and merge bisimilar states.
    minimized = minimize(ba)

    return minimized
```

### CTL pre-execution verification

CTL model checking verifies properties of execution plans (DAGs of tasks) before they run. The plan's task graph is the model; CTL formulas specify required properties.

**Algorithm** (Clarke, Emerson, Sistla 1986):

```
ctl_check(plan: TaskDAG, formula: CtlFormula) -> bool:
    # The plan is a Kripke structure where:
    #   - States = task nodes + a "done" terminal state
    #   - Transitions = task dependencies (edges in the DAG)
    #   - Atomic propositions = task properties (has_gate_check, is_safe, etc.)

    match formula:
        AG(phi):
            # For all paths, globally phi.
            # Compute the set of states satisfying phi, then check
            # that all reachable states from the initial state are in that set.
            sat_phi = ctl_sat(plan, phi)
            reachable = bfs_reachable(plan, plan.initial_state)
            return reachable.is_subset(&sat_phi)

        EF(phi):
            # There exists a path where eventually phi.
            # Backward fixed-point: start from states satisfying phi,
            # iteratively add predecessors until fixed point.
            sat_phi = ctl_sat(plan, phi)
            reachable_back = backward_reachable(plan, sat_phi)
            return plan.initial_state in reachable_back

        AF(phi):
            # For all paths, eventually phi.
            # Backward fixed-point with universal quantification.
            sat_phi = ctl_sat(plan, phi)
            result = sat_phi.clone()
            loop:
                prev = result.clone()
                for state in plan.states:
                    if all successors of state are in result:
                        result.insert(state)
                if result == prev:
                    break
            return plan.initial_state in result

        AG(EF(safe_state)):
            # From every reachable state, there exists a path to safety.
            # Combined: check AG and EF composition.
            sat_safe = ctl_sat(plan, Atom("safe_state"))
            recoverable = backward_reachable(plan, sat_safe)
            reachable = bfs_reachable(plan, plan.initial_state)
            return reachable.is_subset(&recoverable)
```

**Standard plan verification properties:**

| Property | CTL formula | What it checks |
|----------|-------------|---------------|
| No dead ends | `AG(EF(done))` | Every plan state can reach completion |
| Gate coverage | `AG(task_node -> AF(gate_check))` | Every task is eventually followed by a gate check |
| Termination | `not EF(AG(not done))` | No infinite non-terminating path exists |
| Recoverability | `AG(EF(safe_state))` | Every state can reach a safe configuration |

### Integration with Gate pipeline

The `TemporalMonitor` integrates as a Policy within the existing Gate/conductor architecture:

```
Gate pipeline (per-task):
  CompileGate -> TestGate -> ClippyGate -> DiffGate
                                              |
                                              v
Conductor (continuous):                   TemporalMonitor
  circuit breaker <-- health score <-- violation Engrams
       |
       v
  Cooldown / Pause / Shutdown signals
```

```rust
/// TemporalMonitor as a Gate implementation.
/// Wraps the monitor and checks accumulated temporal properties
/// after each task completes.
pub struct TemporalGate {
    monitor: Arc<parking_lot::Mutex<TemporalMonitor>>,
}

#[async_trait]
impl Gate for TemporalGate {
    async fn verify(&self, engram: &Signal) -> Result<Verdict> {
        let mut monitor = self.monitor.lock();
        let violations = monitor.process(&[engram.clone()]);

        if violations.is_empty() {
            Ok(Verdict::Pass {
                confidence: 1.0,
                message: "All temporal properties hold".into(),
            })
        } else {
            Ok(Verdict::Fail {
                confidence: 1.0,
                message: format!(
                    "{} temporal violations: {}",
                    violations.len(),
                    violations
                        .iter()
                        .map(|v| v.body_text().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                violations,
            })
        }
    }
}
```

### Configuration parameters

```toml
[safety.temporal]
liveness_bound = 1000            # Max steps without accepting state. Range: 100..100000.
tick_interval_secs = 10          # Gamma tick interval. Range: 1..60.
max_formulas = 50                # Maximum compiled automata. Range: 1..200.
violation_priority = "high"      # Priority of violation Engrams: "low", "medium", "high".
```

### Test criteria

- `BuchiAutomaton::transition()` with an empty event label produces a valid state transition
- `BuchiAutomaton::is_rejecting()` returns true when active_states is empty
- `BuchiAutomaton::is_rejecting()` returns true when steps_since_accepting exceeds liveness_bound
- `TemporalMonitor::process()` returns a violation Engram when a safety property is violated
- The LTL formula `G(a -> X(b))` compiled to a Buchi automaton rejects the trace `[{a}, {}, {a}, {b}]` (a without b at next step)
- CTL `AG(EF(done))` returns false for a plan DAG with a dead-end node
- `TemporalGate` produces `Verdict::Fail` when any monitored formula is violated
- `TemporalMonitor::reset()` returns all automata to their initial states

---

## DeFi Temporal Patterns (Chain Domain)

The legacy specification catalogs 40 DeFi temporal patterns. Representative examples:

### Transaction Ordering

```
G(approve → X(transfer))
    "An approval is always followed by a transfer in the next step"
    (prevents orphaned approvals that leave unlimited allowances)

G(large_swap → ¬X(same_pool_swap))
    "A large swap is not immediately followed by another swap on the same pool"
    (prevents self-sandwiching or detectable patterns)
```

### Flash Loan Detection

```
G(borrow → F≤1block(repay))
    "Every borrow is repaid within the same block"
    (characterizes flash loans — legitimate pattern but requires monitoring)
```

### Liquidation Patterns

```
G(health_factor_below_1 → F≤5blocks(liquidation ∨ collateral_top_up))
    "A below-threshold health factor leads to liquidation or top-up within 5 blocks"
```

---

## Category-Theoretic Composition

Temporal monitors compose using category-theoretic operations:

- **Product**: Monitor A ∧ Monitor B — both must accept
- **Coproduct**: Monitor A ∨ Monitor B — at least one must accept
- **Sequential composition**: Monitor A ; Monitor B — B starts when A accepts

This enables building complex temporal safety properties from simple, well-tested components. The gate pipeline in `roko-gate` already uses sequential composition: CompileGate → TestGate → ClippyGate.

---

## Integration with Roko Safety Architecture

### Temporal Monitor as a Policy

In Roko's Synapse Architecture, the temporal monitor implements the `Policy` trait. It observes the stream of Engrams (tool calls, gate verdicts, state changes) and emits new Engrams when temporal properties are violated:

```rust
pub struct TemporalMonitor {
    /// Compiled LTL formulas as Buchi automata.
    automata: Vec<BuchiAutomaton>,
    /// Current state for each automaton.
    states: Vec<AutomatonState>,
}

impl Policy for TemporalMonitor {
    fn decide(&self, engrams: &[Signal]) -> Vec<Signal> {
        let mut violations = Vec::new();
        for engram in engrams {
            for (automaton, state) in self.automata.iter().zip(self.states.iter_mut()) {
                let event = classify_event(engram);
                let new_state = automaton.transition(state, &event);
                if automaton.is_rejecting(&new_state) {
                    violations.push(create_violation_engram(
                        automaton.formula_name(),
                        engram,
                    ));
                }
                *state = new_state;
            }
        }
        violations
    }
}
```

Violation Engrams carry `Kind::SafetyViolation` and high-priority scores, ensuring they are routed to the conductor for intervention. The temporal monitor runs at every gamma tick (~5-15 seconds), checking accumulated events against all active formulas.

### Existing Temporal Properties in Production

Several temporal safety properties are already enforced in the Roko codebase, though not expressed as formal LTL formulas:

| Property | Expression | Enforced By |
|---|---|---|
| "Every bash command is checked before execution" | `G(bash_call → previous(safety_check))` | SafetyLayer pre-execution hook |
| "Gate failures lead to retry or skip" | `G(gate_failed → F(retry ∨ skip ∨ abort))` | Conductor circuit breaker |
| "Every started task eventually terminates" | `G(task_started → F(task_completed ∨ task_failed))` | ProcessSupervisor timeout + ghost turn detection |
| "Rate limits are enforced" | `G(rate_exceeded → ¬tool_call)` | RateLimiter deny response |

Formalizing these as LTL and compiling to Buchi automata (Tier 3) would make them composable, testable, and extensible.

---

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| Gate pipeline (sequential composition) | Built | `roko-gate/` (11 gates, 6-rung pipeline) |
| Adaptive thresholds (temporal feedback) | Built | `.roko/learn/gate-thresholds.json` |
| Conductor circuit breaker (safety property) | Built | `roko-conductor/` |
| Ghost turn detection (liveness property) | Built | `orchestrate.rs` conductor integration |
| Full LTL Buchi automata monitor | Design only | Target: Tier 3 |
| CTL plan verification | Design only | Target: Tier 3 |
| DeFi temporal pattern library | Design only | Target: Tier 3 (chain domain) |

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Pnueli (1977) | Temporal logic of programs — foundational LTL |
| Clarke, Emerson, Sistla (1986) | Automatic verification via CTL model checking |
| Buchi (1962) | Buchi automata for infinite word recognition |
| Vardi & Wolper (1986) | Automata-theoretic approach to LTL verification |
| Bartocci et al. (2018) | Runtime verification — survey and perspectives |
| Havelund & Rosu (2001) | Monitoring Java programs with temporal logic |
| Havelund & Rosu (2004, FMSD) | Past-time LTL monitoring with O(\|formula\|) space |
| Maler & Nickovic (2004) | Signal temporal logic (STL) for real-valued signals |
| AgentSpec (Wang et al., 2025/2026, ICSE '26, arXiv:2503.18666) | Customizable runtime enforcement DSL for LLM agents |
| AgentGuard (Koohestani et al., 2025, arXiv:2509.23864) | Dynamic probabilistic assurance via MDP model checking |

---

## Extended Temporal Pattern Library

Beyond the existing DeFi patterns, this section defines a comprehensive pattern library for general-purpose coding agents. These are LTL formulas that can be compiled to Buchi automata and loaded into the TemporalMonitor.

### Code agent safety patterns

```
% P1: No write without prior read (understand before modifying)
G(write_file(F) -> P(read_file(F)))
    "Every file write was preceded by reading the same file"

% P2: No git push without passing gates
G(git_push -> P(compile_gate_passed /\ test_gate_passed /\ clippy_gate_passed))
    "Every push was preceded by passing all three gates"

% P3: No concurrent writes to same file
G(write_file(F) -> -X(write_file(F) /\ different_agent))
    "Two agents never write the same file in consecutive steps"

% P4: Monotonic progress -- task state only moves forward
G(task_state(T, completed) -> G(task_state(T, completed)))
    "A completed task stays completed (no regression)"

% P5: Budget enforcement
G(cost_budget_exceeded -> F<=3(pause \/ shutdown))
    "Budget exceeded leads to pause or shutdown within 3 steps"

% P6: Escalation chain -- failures trigger review
G(gate_failed(T) /\ gate_failed(T) /\ gate_failed(T) -> F(human_review(T)))
    "Three consecutive gate failures trigger human review"

% P7: Sandbox containment
G(-(file_access /\ outside_worktree))
    "No file access targets paths outside the worktree -- ever"

% P8: Secret non-disclosure
G(scrub_policy_match -> -X(content_in_llm_context(matched_content)))
    "Scrubbed content never appears in subsequent LLM context"
```

### Multi-agent coordination patterns

```
% P9: No conflicting concurrent modifications
G(agent_a_modifies(F) /\ agent_b_modifies(F) -> same_worktree(a, b) = false)
    "Concurrent modifications to the same file only happen in separate worktrees"

% P10: Task ordering respects DAG dependencies
G(task_started(T2) -> P(task_completed(dep(T2))))
    "A task starts only after all its dependencies have completed"

% P11: Merge conflict resolution
G(merge_conflict(B) -> F<=10(conflict_resolved(B) \/ merge_aborted(B)))
    "Every merge conflict is resolved or aborted within 10 steps"
```

### Pattern library registry

```rust
/// A temporal pattern in the library.
pub struct TemporalPattern {
    /// Unique identifier for this pattern.
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// The LTL formula source string.
    pub formula: String,
    /// Category: safety, liveness, fairness, coordination.
    pub category: PatternCategory,
    /// Whether this pattern is enabled by default.
    pub enabled_by_default: bool,
    /// Domain: general, code, chain, research.
    pub domain: PatternDomain,
    /// Severity of a violation.
    pub violation_severity: ViolationSeverity,
}

pub enum PatternCategory {
    Safety,       // G(-bad) -- something bad never happens
    Liveness,     // G(started -> F(completed)) -- something good eventually happens
    Fairness,     // GF(condition) -- something happens infinitely often
    Coordination, // Multi-agent ordering constraints
}

pub enum PatternDomain { General, Code, Chain, Research }
pub enum ViolationSeverity { Info, Warning, Error, Critical }

/// The pattern library: a registry of all temporal patterns.
pub struct PatternLibrary {
    patterns: Vec<TemporalPattern>,
}

impl PatternLibrary {
    /// Load the default pattern library for a given domain.
    pub fn default_for_domain(domain: PatternDomain) -> Self {
        let mut lib = Self { patterns: Vec::new() };
        // Always include general safety patterns
        lib.add_general_patterns();
        match domain {
            PatternDomain::Code => lib.add_code_patterns(),
            PatternDomain::Chain => lib.add_chain_patterns(),
            _ => {}
        }
        lib
    }

    /// Compile all enabled patterns into Buchi automata.
    pub fn compile_enabled(&self) -> Vec<BuchiAutomaton> {
        self.patterns
            .iter()
            .filter(|p| p.enabled_by_default)
            .map(|p| ltl_to_buchi(&parse_ltl(&p.formula)))
            .collect()
    }
}
```

### Past-time LTL (ptLTL) extensions

Standard LTL looks forward in time (G, F, X, U). Past-time LTL adds backward-looking operators that are essential for safety patterns like "every write was preceded by a read."

```rust
/// Past-time LTL operators.
/// These extend the LtlAst with backward-looking operators.
pub enum PtLtlOp {
    /// Previously: the formula held at some past step.
    Previously(Box<LtlAst>),
    /// Once: the formula held at some past step (past-time F).
    Once(Box<LtlAst>),
    /// Historically: the formula held at every past step (past-time G).
    Historically(Box<LtlAst>),
    /// Since: phi held since psi was true (past-time U).
    Since(Box<LtlAst>, Box<LtlAst>),
}
```

Past-time LTL formulas can be monitored in O(|formula|) space per step (Havelund & Rosu, 2004), making them efficient for real-time monitoring.

### Bounded temporal operators

Many safety patterns use bounded temporal operators (F<=k, G<=k) for practical monitoring:

```rust
/// Bounded temporal operators: F<=k and G<=k.
/// These convert liveness properties to bounded-liveness safety properties.
pub enum BoundedOp {
    /// Eventually within k steps.
    EventuallyBounded { formula: Box<LtlAst>, bound: u64 },
    /// Globally for the next k steps.
    GloballyBounded { formula: Box<LtlAst>, bound: u64 },
}
```

### Configuration

```toml
[safety.temporal.patterns]
# Domain for pattern library selection.
domain = "code"    # "general" | "code" | "chain" | "research"
# Additional custom pattern files (LTL formulas, one per line).
custom_pattern_files = [".roko/temporal-patterns.ltl"]
# Enable past-time LTL operators.
enable_ptltl = true
# Maximum bound for bounded temporal operators.
max_bounded_horizon = 100    # Range: 10..10000.
# Pattern violation response.
violation_response = "alert"  # "log" | "alert" | "pause" | "abort"
```

### Test criteria

- Pattern P1 (write-after-read) rejects trace [{write_file("a.rs")}, ...] without prior read_file("a.rs")
- Pattern P2 (push-after-gates) rejects trace with git_push but no prior gate passes
- Pattern P7 (sandbox containment) rejects any trace containing file_access + outside_worktree
- PatternLibrary::default_for_domain(Code) includes all code patterns
- PatternLibrary::compile_enabled() produces valid Buchi automata for each pattern
- Past-time Once(phi) is satisfied iff phi held at some earlier step
- Bounded F<=5(phi) is violated if phi does not hold within 5 steps
- Pattern violation severity is correctly propagated to the TemporalMonitor's violation Engrams

---

## Temporal Attack Pattern Detection

Beyond verifying safety properties, the TemporalMonitor must detect *adversarial temporal patterns* -- multi-step attack sequences that are benign individually but malicious in aggregate. This section draws on three recent works: Temporal Attack Pattern Detection in Multi-Agent Workflows (arXiv:2601.00848, January 2025), SentinelAgent graph-based anomaly detection (arXiv:2505.24201, May 2025), and AgentSpec runtime enforcement DSL (Wang et al., ICSE '26).

### Three-tier anomaly detection

Following SentinelAgent's architecture, temporal anomalies are detected at three levels:

```rust
/// Three-tier temporal anomaly detection.
/// Node-level: individual action anomalies.
/// Edge-level: anomalous transitions between actions.
/// Path-level: anomalous multi-step execution paths.
pub struct TemporalAnomalyDetector {
    /// Node-level detectors: flag individual anomalous actions.
    pub node_detectors: Vec<Box<dyn NodeAnomalyDetector>>,
    /// Edge-level detectors: flag anomalous transitions.
    pub edge_detectors: Vec<Box<dyn EdgeAnomalyDetector>>,
    /// Path-level detectors: flag anomalous execution paths.
    pub path_detectors: Vec<Box<dyn PathAnomalyDetector>>,
    /// Sliding window of recent agent actions.
    pub window: SlidingActionWindow,
    /// Anomaly score threshold for alerting.
    /// Range: 0.0..1.0. Default: 0.7.
    pub alert_threshold: f64,
}

/// Sliding window over recent agent actions for temporal analysis.
pub struct SlidingActionWindow {
    /// Actions in chronological order.
    actions: VecDeque<TimestampedAction>,
    /// Maximum window size (actions). Range: 10..10000. Default: 200.
    max_size: usize,
    /// Maximum window duration. Default: 30 minutes.
    max_duration: Duration,
}

pub struct TimestampedAction {
    pub timestamp: Instant,
    pub agent_id: String,
    pub tool_name: String,
    pub tool_args_hash: [u8; 32],
    pub result_status: ActionStatus,
    pub permission_level: u8,
    pub files_accessed: Vec<String>,
    pub taint_label: Option<SecurityLabel>,
}

pub enum ActionStatus { Success, Failure, Timeout, Blocked }
```

### Node-level: individual action anomalies

```rust
pub trait NodeAnomalyDetector: Send + Sync {
    fn score(&self, action: &TimestampedAction, context: &WindowContext) -> f64;
    fn name(&self) -> &str;
}

/// Detect actions with unusually high permission levels for the current phase.
pub struct PermissionAnomalyDetector {
    /// Expected permission level per task phase.
    pub phase_permission_map: HashMap<String, u8>,
}

/// Detect tool calls at unusual times (e.g., network calls during compile phase).
pub struct PhaseToolAnomalyDetector {
    /// Expected tools per task phase.
    pub phase_tool_map: HashMap<String, HashSet<String>>,
}

/// Detect actions targeting unusual files (outside normal working set).
pub struct FileAccessAnomalyDetector {
    /// Baseline file access patterns (learned from first N actions).
    pub baseline_files: HashSet<String>,
    /// Anomaly score for accessing non-baseline files.
    pub novel_file_score: f64, // Default: 0.3
}
```

### Edge-level: anomalous transitions

```rust
pub trait EdgeAnomalyDetector: Send + Sync {
    fn score(
        &self,
        from: &TimestampedAction,
        to: &TimestampedAction,
        context: &WindowContext,
    ) -> f64;
    fn name(&self) -> &str;
}

/// Detect rapid tool switching (may indicate confused or adversarial agent).
pub struct RapidSwitchDetector {
    /// Minimum expected interval between different tools (ms).
    /// Range: 0..10000. Default: 100.
    pub min_interval_ms: u64,
}

/// Detect read-then-exfiltrate patterns.
pub struct ReadExfiltrateDetector {
    /// Sensitive file patterns (glob).
    pub sensitive_patterns: Vec<String>,
    /// Network tools that could exfiltrate.
    pub network_tools: HashSet<String>,
}

/// Detect permission level increases across consecutive actions.
pub struct PermissionEscalationDetector {
    /// Number of consecutive escalation steps to flag.
    /// Range: 2..10. Default: 3.
    pub escalation_threshold: usize,
}
```

### Path-level: multi-step attack sequences

```rust
pub trait PathAnomalyDetector: Send + Sync {
    fn score(&self, window: &SlidingActionWindow) -> f64;
    fn name(&self) -> &str;
}

/// Detect the "boiling frog" pattern: small incremental changes
/// that individually pass gates but cumulatively are harmful.
pub struct BoilingFrogDetector {
    /// Cumulative diff threshold (lines changed across window).
    /// Range: 100..100000. Default: 5000.
    pub cumulative_diff_threshold: usize,
    /// Per-action diff that would individually pass gates.
    pub per_action_max_diff: usize,
}

/// Detect reconnaissance chains: sequences of read-only operations
/// that together map the system's attack surface.
pub struct ReconnaissanceDetector {
    /// Sequence patterns that indicate reconnaissance.
    pub recon_patterns: Vec<Vec<String>>,
    // Default patterns:
    // ["glob", "read_file", "grep"] -- mapping the codebase
    // ["read_file(.env)", "read_file(.git/config)", "read_file(Cargo.toml)"] -- collecting secrets/config
}

/// Detect delayed exfiltration: data staged early, exfiltrated later.
/// Combines taint tracking with temporal analysis.
pub struct DelayedExfiltrationDetector {
    /// Maximum delay between taint acquisition and exfiltration (seconds).
    /// Range: 1..86400. Default: 3600.
    pub max_delay_secs: u64,
    /// Taint labels that indicate sensitive data.
    pub sensitive_labels: Vec<TaintLabel>,
}

/// Detect slow privilege escalation: gradual widening of permissions
/// over many tasks that individually appear benign.
pub struct SlowEscalationDetector {
    /// Window size (tasks) for measuring permission drift.
    /// Range: 5..100. Default: 20.
    pub window_size: usize,
    /// Maximum permission set growth rate (new permissions per task).
    /// Range: 0.0..1.0. Default: 0.1.
    pub max_growth_rate: f64,
}
```

### Composite anomaly scoring

```rust
impl TemporalAnomalyDetector {
    /// Process a new action and compute composite anomaly score.
    /// Returns anomalies that exceed the alert threshold.
    pub fn process_action(
        &mut self,
        action: TimestampedAction,
    ) -> Vec<TemporalAnomaly> {
        // Add to sliding window
        self.window.push(action.clone());
        let context = self.window.context();
        let mut anomalies = Vec::new();

        // Node-level scoring
        for detector in &self.node_detectors {
            let score = detector.score(&action, &context);
            if score > self.alert_threshold {
                anomalies.push(TemporalAnomaly {
                    level: AnomalyLevel::Node,
                    detector_name: detector.name().to_string(),
                    score,
                    description: format!(
                        "Node anomaly: {} scored {:.2} for action {}",
                        detector.name(), score, action.tool_name
                    ),
                    timestamp: action.timestamp,
                });
            }
        }

        // Edge-level scoring (compare with previous action)
        if let Some(prev) = self.window.previous() {
            for detector in &self.edge_detectors {
                let score = detector.score(prev, &action, &context);
                if score > self.alert_threshold {
                    anomalies.push(TemporalAnomaly {
                        level: AnomalyLevel::Edge,
                        detector_name: detector.name().to_string(),
                        score,
                        description: format!(
                            "Edge anomaly: {} scored {:.2} for {} -> {}",
                            detector.name(), score, prev.tool_name, action.tool_name
                        ),
                        timestamp: action.timestamp,
                    });
                }
            }
        }

        // Path-level scoring (analyze full window)
        for detector in &self.path_detectors {
            let score = detector.score(&self.window);
            if score > self.alert_threshold {
                anomalies.push(TemporalAnomaly {
                    level: AnomalyLevel::Path,
                    detector_name: detector.name().to_string(),
                    score,
                    description: format!(
                        "Path anomaly: {} scored {:.2} over window",
                        detector.name(), score
                    ),
                    timestamp: action.timestamp,
                });
            }
        }

        anomalies
    }
}

pub struct TemporalAnomaly {
    pub level: AnomalyLevel,
    pub detector_name: String,
    pub score: f64,
    pub description: String,
    pub timestamp: Instant,
}

pub enum AnomalyLevel { Node, Edge, Path }
```

### Integration with TemporalMonitor

The `TemporalAnomalyDetector` operates alongside the `TemporalMonitor` (LTL property verification):

```
Agent events stream
    |
    +--> TemporalMonitor (property verification)
    |       Checks: "does this event sequence satisfy LTL properties?"
    |       Output: violation Engrams for property breaches
    |
    +--> TemporalAnomalyDetector (pattern detection)
    |       Checks: "does this event sequence match known attack patterns?"
    |       Output: anomaly Engrams with confidence scores
    |
    +--> Conductor
            Aggregates: violations + anomalies -> health score -> circuit breaker
```

### Configuration

```toml
[safety.temporal.anomaly]
# Enable temporal anomaly detection.
enabled = true
# Alert threshold for anomaly scores. Range: 0.0..1.0. Default: 0.7.
alert_threshold = 0.7
# Sliding window size (actions). Range: 10..10000. Default: 200.
window_size = 200
# Window duration (seconds). Range: 60..86400. Default: 1800.
window_duration_secs = 1800
# Enable specific detectors.
enable_boiling_frog = true
enable_reconnaissance = true
enable_delayed_exfiltration = true
enable_slow_escalation = true
enable_permission_escalation = true
enable_read_exfiltrate = true
# Boiling frog cumulative diff threshold.
boiling_frog_diff_threshold = 5000
# Slow escalation window (tasks).
slow_escalation_window = 20
# Slow escalation max growth rate.
slow_escalation_max_growth_rate = 0.1
```

### Test criteria

- `SlidingActionWindow` evicts actions beyond `max_size` and `max_duration`
- `BoilingFrogDetector` flags when cumulative diff exceeds threshold despite each individual diff being small
- `ReconnaissanceDetector` flags `["glob", "read_file", "grep"]` sequence targeting config files
- `PermissionEscalationDetector` flags 3+ consecutive permission increases
- `DelayedExfiltrationDetector` flags `read_file(.env) ... web_fetch(external)` with delay < max_delay_secs
- `SlowEscalationDetector` flags permission set growth > max_growth_rate per task over window
- Composite scoring correctly combines node + edge + path scores
- Anomalies exceeding `alert_threshold` produce Engrams routed to conductor
- Integration with TemporalMonitor: both property violations and anomalies feed conductor health score

### Academic references for this section

| Paper | Contribution |
|---|---|
| Temporal Attack Patterns (arXiv:2601.00848, 2025) | Fine-tuned detection of temporal attacks in multi-agent workflows |
| SentinelAgent (arXiv:2505.24201, 2025) | Three-tier graph-based anomaly detection for multi-agent systems |
| AgentSpec (Wang et al., ICSE '26, arXiv:2503.18666) | Runtime enforcement DSL for LLM agent constraints |
| AgentGuard (Koohestani et al., 2025, arXiv:2509.23864) | Dynamic probabilistic assurance via MDP model checking |

---

## Cross-References

- [09-adaptive-risk.md](09-adaptive-risk.md) — Layer 4 observation feeds temporal monitors
- [12-witness-dag.md](12-witness-dag.md) — Temporal traces stored in content-addressed DAG
- [13-formal-verification.md](13-formal-verification.md) — Static verification pipeline
