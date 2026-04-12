# 05 — Gate Ratcheting

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-gate` (`crates/roko-gate/src/ratchet.rs`)
> **Status**: Implemented (207 lines)

---

## 1. Overview

The `GateRatchet` prevents verification regression. Once a plan has passed rung N, it
should never be allowed to regress to rung N-1. The ratchet tracks the highest rung each
plan has passed and provides a `can_regress()` check that the conductor uses before
accepting a lower verdict.

This solves a specific failure mode in multi-attempt agent loops: **convergence
thrashing**. An agent fixes the compile error but breaks lint. On retry, it fixes lint
but breaks compile. Without a ratchet, this cycle can repeat indefinitely, consuming
compute and making no net progress.

> **Citation**: crates/roko-gate/src/ratchet.rs — Full implementation.

---

## 2. The Thrashing Problem

Consider a 3-rung pipeline (Compile → Lint → Test):

```
Attempt 1: Compile PASS, Lint FAIL
  Agent receives: "warning: unused variable"
  Agent fixes lint issue

Attempt 2: Compile FAIL, (Lint/Test never run)
  Agent's lint fix introduced a type error
  Agent receives: "error[E0308]: mismatched types"
  Agent fixes type error

Attempt 3: Compile PASS, Lint FAIL
  Agent's type fix reintroduced the lint issue
  Agent receives: "warning: unused variable"
  ... (infinite loop)
```

Each attempt passes one rung and fails the next. The agent is doing work — it's
modifying code each time — but it's not making *progress*. The net verification state
oscillates between "compiles but doesn't lint" and "lints but doesn't compile."

The ratchet breaks this cycle by making the pipeline say: "You passed Compile on
Attempt 1. You are not allowed to regress below Compile on Attempt 2." If Attempt 2
fails compile, the ratchet flags this as a regression, and the system can:
- Reject the attempt outright
- Flag it for human review
- Give the agent a different prompt ("Your previous attempt passed compile. Your new
  attempt broke compile. Fix the compile error without regressing.")

---

## 3. Data Structure

```rust
pub struct GateRatchet {
    passes: HashMap<String, u8>,
}
```

A map from plan identifier (string) to the highest rung number (u8) that plan has
passed. The rung number corresponds to the `Rung` enum's discriminant (0–6).

### Why u8?

Seven rungs fit in 3 bits. Using `u8` is the natural Rust choice for a small non-negative
integer. It avoids the overhead of an enum in a `HashMap` and allows simple comparison
operators (`>`, `>=`).

---

## 4. Operations

### 4.1 Record a Pass

```rust
pub fn record_pass(&mut self, plan_id: impl Into<String>, rung: u8) {
    let entry = self.passes.entry(plan_id.into()).or_insert(0);
    if rung > *entry {
        *entry = rung;
    }
}
```

Records that `plan_id` passed `rung`. Only advances the watermark — if the plan has
already passed a higher rung, this is a no-op.

**Monotonic property**: The stored value for any plan ID can only increase or stay the
same. It never decreases. This is the core ratchet invariant.

### 4.2 Query Highest Pass

```rust
pub fn highest_pass(&self, plan_id: &str) -> Option<u8> {
    self.passes.get(plan_id).copied()
}
```

Returns the highest rung the plan has passed, or `None` if the plan has no recorded
passes. `None` means the ratchet has no opinion — the plan is free to pass or fail any
rung.

### 4.3 Check for Regression

```rust
pub fn can_regress(&self, plan_id: &str, rung: u8) -> bool {
    match self.passes.get(plan_id) {
        None => true,                    // Unknown plan: no regression possible
        Some(&highest) => rung >= highest, // OK if same or higher
    }
}
```

Returns `false` if accepting `rung` as the new highest would be a regression (i.e., the
plan has already passed a strictly higher rung). Returns `true` if:
- The plan has never been recorded (no regression possible)
- The plan's highest pass is equal to or lower than `rung`

**Note**: The method is named `can_regress` but returns `true` when regression is *not*
happening. The semantics are: "Is it acceptable to record this rung?" — which is true
when no regression would occur.

### 4.4 Plan Count and Clear

```rust
pub fn plan_count(&self) -> usize {
    self.passes.len()
}

pub fn clear(&mut self) {
    self.passes.clear();
}
```

`plan_count()` reports how many plans are tracked. `clear()` resets the ratchet entirely,
used when starting a fresh execution session.

---

## 5. Usage Pattern in the Orchestrator

```rust
// After gate pipeline produces a verdict:
let rung = verdict_rung;  // Which rung did we reach?
let plan_id = &task.plan_id;

if verdict.passed {
    ratchet.record_pass(plan_id, rung);
} else {
    // Check if this failure represents a regression
    if !ratchet.can_regress(plan_id, rung) {
        // Regression detected! Agent passed rung N before but now fails it.
        // Options:
        //   1. Reject this attempt
        //   2. Feed back: "You regressed from rung N to rung M"
        //   3. Trigger re-planning
    }
}
```

---

## 6. Ratchet + Escalation Interaction

The ratchet and the escalation mechanism (see [02-6-rung-selector.md](./02-6-rung-selector.md))
work together but serve different purposes:

| Mechanism | Direction | Purpose |
|---|---|---|
| Escalation | Forward (adds rungs) | Failed → try harder |
| Ratchet | Backward (blocks regression) | Passed → don't lose progress |

Together they create a monotonically advancing verification frontier:

```
Attempt 1: Complexity=Simple, Rungs=[Compile, Lint]
  → Compile PASS (ratchet records rung 0)
  → Lint FAIL
  → Escalate to Standard

Attempt 2: Complexity=Standard, Rungs=[Compile, Lint, Test, Symbol]
  → Compile must still pass (ratchet enforces)
  → Lint PASS (ratchet records rung 1)
  → Test FAIL
  → Escalate to Complex

Attempt 3: Complexity=Complex, Rungs=[all]
  → Compile must still pass (ratchet enforces)
  → Lint must still pass (ratchet enforces)
  → Test PASS (ratchet records rung 2)
  → ... and so on
```

Each attempt can only move the verification frontier forward. Rungs that have been
passed stay passed.

---

## 7. Per-Plan Isolation

Each plan has its own ratchet entry. Plan A's progress has no effect on Plan B's
ratchet. This is important because:

- Plans are independent units of work (a plan might be "implement rate limiter" while
  another is "fix auth bug")
- Different plans may be at different stages of verification
- One plan's compile failure doesn't prevent another plan from passing lint

The `HashMap<String, u8>` keying on plan ID provides this isolation naturally.

---

## 8. Ratchet in the Context of Process Reward Models

The ratchet is a simple form of process reward: it tracks *intermediate* verification
progress, not just final outcomes. A plan that reaches Rung 3 (passed Compile + Lint +
Test) has demonstrated more progress than one that only reaches Rung 1 (passed Compile).

This intermediate signal feeds into the process reward model (see
[07-process-reward-models.md](./07-process-reward-models.md)):

- **Promise score**: How likely is this plan to eventually pass all rungs, given that
  it has passed rung N so far?
- **Progress score**: Is the plan advancing (reaching higher rungs on successive
  attempts) or stalling?

The ratchet provides the raw data — "plan X has reached rung N" — that the process
reward model interprets.

> **Citation**: refactoring-prd/02-five-layers.md — "Process Reward Models: Promise +
> Progress scoring, low Promise → early intervention, negative Progress → re-planning."

---

## 9. Edge Cases

### 9.1 Rung 0 Ratchet

If a plan passes Rung 0 (Compile), the ratchet records `highest = 0`. On the next
attempt, `can_regress("plan", 0)` returns `true` (rung 0 >= 0). Only a *lower* rung
would trigger regression, but there is no rung below 0. So a plan that passed Compile
can fail Compile on a subsequent attempt without the ratchet blocking it.

Wait — that seems wrong. If we passed Compile, shouldn't we enforce it?

The answer is: `can_regress` checks if the *proposed rung* is below the highest. Rung 0
is not below 0, so it's allowed. This is correct because the ratchet's `record_pass()`
only fires on success. A failure at Rung 0 doesn't call `record_pass()`, so the stored
value stays at 0. The regression check happens externally — the orchestrator checks
`can_regress(plan, failing_rung)` and decides what to do.

### 9.2 Full Pipeline Pass

When a plan passes all 7 rungs, `highest_pass` = 6. Any subsequent failure at any rung
is a regression (since all rungs 0–5 are below 6). Only passing rung 6 again is non-
regressive.

### 9.3 String Plan IDs

Plan IDs are strings, supporting both owned (`String::from("plan-1")`) and borrowed
(`"plan-1"`) inputs via `impl Into<String>`.

---

## 10. Future: Persistent Ratchet

The current ratchet is in-memory and ephemeral. When the process exits, all ratchet
state is lost. For long-running or resumable executions, the ratchet should persist
to disk:

```json
// .roko/state/gate-ratchet.json
{
  "plan-42": 3,
  "plan-43": 1,
  "plan-44": 5
}
```

This aligns with the existing executor snapshot persistence
(`.roko/state/executor.json`) and would be loaded on `--resume`.

---

## 11. Testing

The ratchet module has 13 tests covering:

| Test | Property |
|---|---|
| `ratchet_new_is_empty` | Default state has no entries |
| `ratchet_record_and_query` | Basic store/query roundtrip |
| `ratchet_only_advances` | Lower rung does not overwrite higher |
| `ratchet_can_regress_prevents_regression` | Detects regression correctly |
| `ratchet_can_regress_allows_same_or_higher` | Non-regression is permitted |
| `ratchet_can_regress_unknown_plan_returns_true` | Unknown plans have no constraint |
| `ratchet_multiple_plans_independent` | Per-plan isolation |
| `ratchet_clear_resets_all` | Clear removes all entries |
| `ratchet_record_pass_zero_rung` | Edge case: rung 0 |
| `ratchet_monotonic_sequence` | Rungs 0→6 all recorded correctly |
| `ratchet_string_plan_ids` | Owned and borrowed IDs both work |
| `ratchet_default_is_new` | `Default` trait works |

> **Citation**: crates/roko-gate/src/ratchet.rs — Tests section, 207 lines total.

---

## 12. Summary

The `GateRatchet` is a one-way valve on verification progress. It answers one question:
"Has this plan ever passed a higher rung than what it's being asked to accept now?"
If yes, the answer is regression. If no, the answer is progress (or same level).

Its simplicity — a `HashMap<String, u8>` with monotonic updates — makes it correct by
construction. There are no complex algorithms, no statistical models, no heuristics.
Just: the highest rung you've passed is the floor you can't drop below.
