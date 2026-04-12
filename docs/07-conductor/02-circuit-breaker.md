# Circuit Breaker

> A plan can fail a maximum of two times. After that, it requires
> human attention. This is not configurable. This is law.

---

## The Problem It Solves

Without a circuit breaker, a fundamentally broken plan enters an
infinite retry loop:

```
Plan fails → orchestrator retries → plan fails the same way →
orchestrator retries → plan fails again → orchestrator retries → ...
```

Each retry costs tokens. Each retry burns wall-clock time that could
be spent on plans that might succeed. Each retry produces the same
failure output, adding noise to the signal stream without adding
information.

This was Issue #7 from production (circuit breaker for repeated
failures): "A plan fails, gets retried, fails the same way, gets
retried again, fails again. Infinite retry loop burning tokens."

The circuit breaker enforces a hard budget: two failures per plan.
After that, the plan is marked as requiring human intervention and
is never automatically retried.

---

## Implementation

The circuit breaker lives in `crates/roko-conductor/src/circuit_breaker.rs`.

```rust
use dashmap::DashMap;

pub const MAX_PLAN_FAILURES: u32 = 2;

pub struct CircuitBreaker {
    failures: DashMap<String, FailureRecord>,
}

struct FailureRecord {
    count: u32,
    // Additional metadata: timestamps, failure reasons, etc.
}
```

### Thread Safety

The `DashMap` provides lock-free concurrent reads and sharded writes.
This matters because the orchestrator may evaluate multiple plans in
parallel — each plan's conductor check should not block on other plans'
failure records.

`DashMap` is a concurrent hash map that shards its data across multiple
locks. Two plans with different IDs will almost always hit different
shards, enabling true parallel access. This is preferable to a
`Mutex<HashMap>` which would serialize all failure record access.

### API

```rust
impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            failures: DashMap::new(),
        }
    }

    /// Record a failure for a plan. Returns true if the plan is now tripped.
    pub fn record_failure(&self, plan_id: &str) -> bool {
        let mut entry = self.failures.entry(plan_id.to_string()).or_insert(FailureRecord { count: 0 });
        entry.count += 1;
        entry.count >= MAX_PLAN_FAILURES
    }

    /// Check if a plan has exceeded its failure budget.
    pub fn is_tripped(&self, plan_id: &str) -> bool {
        self.failures
            .get(plan_id)
            .map(|record| record.count >= MAX_PLAN_FAILURES)
            .unwrap_or(false)
    }

    /// Reset failure count for a plan (e.g., after manual intervention).
    pub fn reset(&self, plan_id: &str) {
        self.failures.remove(plan_id);
    }
}
```

---

## Three-State Model

The circuit breaker implements a classic three-state pattern, though
the implementation in roko-conductor uses a simplified two-state model
(tripped / not tripped). The full three-state model, implemented in
the provider health tracker (`roko-learn/src/provider_health.rs`),
provides additional granularity:

### State Transitions

```
Closed (Healthy)
  │
  │ consecutive failures >= threshold
  ▼
Open (Tripped)
  │
  │ cooldown period expires
  ▼
HalfOpen (Probing)
  │
  ├─ probe succeeds → Closed
  │
  └─ probe fails → Open (reset cooldown)
```

**Closed**: Normal operation. Failures are counted but requests proceed.
This is the initial state for every plan.

**Open**: All requests are blocked. The plan has exceeded its failure
budget. No automatic retry is permitted. In the conductor's simplified
model, this is the terminal state (tripped). In the provider health
model, the system waits for a cooldown period before transitioning to
HalfOpen.

**HalfOpen**: One probe request is permitted. If the probe succeeds,
the breaker returns to Closed. If the probe fails, the breaker returns
to Open with a fresh cooldown. This state exists in the provider health
tracker but not in the conductor's plan-level breaker — because plans
do not benefit from automatic probing (a plan that failed twice needs
a different approach, not another attempt at the same approach).

### Error-Type-Specific Cooldowns

The provider health tracker uses error classification to set cooldown
durations:

| Error Class | Cooldown | Rationale |
|------------|----------|-----------|
| RateLimit | 5 seconds | Transient; provider will accept again soon |
| Timeout | 10 seconds | Might indicate temporary load |
| ServerError | 30 seconds | Likely operational issue, needs more time |
| AuthFailure | 5 minutes | Likely persistent; manual fix needed |
| ContentPolicy | 5 minutes | Likely persistent |
| ContextOverflow | N/A | Not retryable; needs model switch |

This error-type-specific behavior lives in the provider health layer
(`roko-learn`), not in the conductor's plan-level breaker. The
conductor's plan-level breaker is simpler: two failures of any kind,
then trip.

---

## Integration with the Conductor

The circuit breaker is checked at the start of every `evaluate()` call:

```rust
impl Conductor {
    pub fn evaluate(&self, plan_id: &str, stream: &[Signal], ctx: &Context) -> ConductorDecision {
        // 1. Check circuit breaker FIRST
        if self.circuit_breaker.is_tripped(plan_id) {
            return ConductorDecision::Fail {
                reason: format!("plan {plan_id} tripped circuit breaker after {} failures", MAX_PLAN_FAILURES),
            };
        }

        // 2. Run watchers
        let watcher_outputs = self.check_all(stream, ctx);

        // 3. Apply intervention policy
        let decision = self.policy.evaluate(&watcher_outputs, ctx);

        // 4. Record failures
        if matches!(decision, ConductorDecision::Fail { .. }) {
            self.circuit_breaker.record_failure(plan_id);
        }

        decision
    }
}
```

The circuit breaker check happens before watcher evaluation. If a plan
is already tripped, there is no point running watchers — the decision
is predetermined. This short-circuit saves watcher evaluation time for
plans that are already done.

---

## Why Two Failures

The `MAX_PLAN_FAILURES = 2` constant is derived from production data:

**First failure**: Often caused by transient issues — API rate limit,
cold start, missing context. Retrying with a fresh agent and potentially
different context frequently succeeds.

**Second failure**: The same plan failing twice usually indicates a
structural problem — the task is beyond the agent's capability with
the given context, the acceptance criteria are contradictory, or the
codebase has changed in a way that makes the task impossible as
specified.

**Third failure (never reached)**: At this point, the probability of
success is negligible. The two previous attempts have already tried
the obvious approaches. A third attempt would likely repeat one of
the first two, producing the same failure at the cost of more tokens.

The math: if each attempt has a 30% success rate (typical for complex
plans that fail the first time), the probability of failing twice is
(0.7)² = 49%. The probability of failing three times is (0.7)³ = 34%.
But this assumes independence — in practice, the second failure is
correlated with the first (same root cause), so the conditional
probability of a third failure given two failures is much higher
than 70%. The expected cost of a third attempt almost always exceeds
its expected value.

---

## Relationship to Hard Guarantees

The circuit breaker implements two hard guarantees from the failure
prevention catalog:

### Hard Guarantee 3: Hard Iteration Cap

Each plan attempt includes up to 3 implementation iterations (implement
→ gate fail → retry). With 2 plan-level failures, the total maximum
is:

```
2 plan attempts × 3 iterations each = 6 total implementation cycles
```

After 6 cycles, the plan is permanently failed. This is the absolute
upper bound on token spend for any single plan.

### Hard Guarantee 7: Circuit Breaker

Direct implementation. The plan can fail a maximum of 2 times. After
2 failures, it is permanently marked as requiring human intervention
and never automatically retried.

```
MAX_PLAN_FAILURES (2) × MAX_ITERATION_LOOP (3) = 6 max attempts ever
```

This prevents:
- Infinite retry loops (max 2 failures, then stop)
- Token burn on doomed plans (6 attempts max, ever)
- Silent stuck plans (tripped state is surfaced prominently)

---

## Per-Plan Isolation

The circuit breaker is keyed by plan ID. This means:

- Plan A hitting its failure budget does not affect Plan B
- Resetting Plan A does not reset Plan B
- The breaker can track hundreds of plans concurrently

This per-plan isolation is critical for batch runs where 20+ plans
execute in parallel. A single broken plan should not cascade to
affect healthy plans.

---

## Manual Reset

The `reset()` method exists for operator override. When a human
examines a failed plan, determines the root cause, applies a fix
(updated context, different model, modified acceptance criteria), they
can reset the circuit breaker to allow the plan to retry.

This is deliberately a manual operation. The system does not auto-reset
breakers because the whole point of the breaker is to prevent automatic
retry of plans that need human judgment. If auto-reset were possible,
the breaker would be bypassed on every failure.

---

## Persistence

The circuit breaker state is part of the executor snapshot. When the
orchestrator checkpoints to `.roko/state/executor.json`, failure records
are included. On resume, the circuit breaker is restored from the
snapshot, preserving failure counts across restarts.

This prevents a circumvention where restarting the orchestrator would
reset all breakers, allowing previously-failed plans to retry. The
breaker survives crashes.

---

## Future: Adaptive Failure Budget

The current `MAX_PLAN_FAILURES = 2` is a constant. A future enhancement
is adaptive failure budgets based on plan complexity:

| Complexity | Failure Budget | Rationale |
|-----------|---------------|-----------|
| Trivial | 1 | If a trivial task fails once, something is fundamentally wrong |
| Simple | 2 | Standard budget |
| Standard | 2 | Standard budget |
| Complex | 3 | Complex tasks have higher variance; third attempt with different strategy may succeed |

This would require wiring the plan's complexity classification (from
the task TOML frontmatter) into the circuit breaker's failure threshold.
The infrastructure exists — the cascade router already uses complexity
classification for model selection.

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/circuit_breaker.rs` | CircuitBreaker struct, DashMap-based tracking |
| `crates/roko-conductor/src/conductor.rs` | Integration point — breaker checked in evaluate() |
| `crates/roko-learn/src/provider_health.rs` | Extended 3-state model for provider health |
| `crates/roko-core/src/agent.rs` | ConductorDecision enum consumed by orchestrator |
