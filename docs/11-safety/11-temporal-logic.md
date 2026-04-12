# Temporal Logic Verification

> **Layer**: L3 Harness (runtime monitoring), L4 Orchestration (pre-execution verification)
>
> **Crate**: Target: `roko-gate` (temporal gates), `roko-conductor` (monitoring)
>
> **Synapse traits**: `Gate` (verify temporal properties), `Policy` (emit temporal violation Engrams)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [09-adaptive-risk.md](09-adaptive-risk.md)

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

---

## Related Topics

- [09-adaptive-risk.md](09-adaptive-risk.md) — Layer 4 observation feeds temporal monitors
- [12-witness-dag.md](12-witness-dag.md) — Temporal traces stored in content-addressed DAG
- [13-formal-verification.md](13-formal-verification.md) — Static verification pipeline
