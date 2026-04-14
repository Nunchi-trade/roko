# 02 — The 7-Rung Gate Selector

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-gate` (`crates/roko-gate/src/rung_selector.rs`)
> **Status**: Implemented (527 lines)


> **Implementation**: Shipping

---

## 1. Overview

Not every task needs every gate. A one-line variable rename doesn't need property-based
testing. A new subsystem does. The rung selector solves this: given a plan's complexity
and the gates available in this environment, it produces a sorted list of rungs to
execute.

The system has 7 rungs, numbered 0 through 6, ordered from cheapest to most expensive.
Lower-numbered rungs always run before higher-numbered ones. The selector determines
*which* subset of rungs to activate based on three inputs:

1. **Plan complexity** — how ambitious is this change?
2. **Rung capabilities** — which gates are actually available?
3. **Prior failures** — did previous attempts fail, warranting escalation?

> **Citation**: crates/roko-gate/src/rung_selector.rs — Full implementation.

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> 6-rung gate system design (Rungs 0–5 in the original; Roko extends to 7 with Rung 6
> for integration tests).

---

## 2. The 7 Rungs

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Rung {
    Compile     = 0,    // Does it compile?
    Lint        = 1,    // Does it pass linting?
    Test        = 2,    // Do tests pass?
    Symbol      = 3,    // Are required symbols present?
    GeneratedTest = 4,  // Do auto-generated tests pass?
    PropertyTest  = 5,  // Do property-based tests pass?
    Integration   = 6,  // Do integration tests pass?
}
```

### 2.1 Rung 0: Compile

The cheapest gate. Runs `cargo check --workspace` (or equivalent). Takes seconds.
Catches syntax errors, type mismatches, missing imports. Every plan runs at least this
rung.

**Gate**: `CompileGate`
**Cost**: Low (seconds)
**False positive rate**: 0% (deterministic)

### 2.2 Rung 1: Lint

Runs `cargo clippy -- -D warnings` (or language equivalent). Catches code quality
issues, common mistakes, and style violations. Still fast — typically under a minute.

**Gate**: `ClippyGate`
**Cost**: Low (seconds to one minute)
**False positive rate**: 0% (deterministic)

### 2.3 Rung 2: Test

Runs the project's existing test suite. This is the first "medium cost" gate — tests
can take minutes for large projects. Catches functional regressions.

**Gate**: `TestGate`
**Cost**: Medium (seconds to 15 minutes)
**False positive rate**: 0% for deterministic tests; flaky tests exist but are not
the gate's fault.

### 2.4 Rung 3: Symbol

Verifies that required symbols (structs, traits, functions) exist with correct kind,
visibility, and module path. Zero cost — no subprocess, just file walking and regex
matching.

**Gate**: `SymbolGate`
**Cost**: Near-zero (file I/O only)
**False positive rate**: 0% (deterministic)

Despite being "free," Symbol is ranked after Test because the information it provides
(symbol existence) is a subset of what compile + test already verify. Symbol is most
valuable when it catches issues *before* a failed compile-test cycle reveals them.

### 2.5 Rung 4: GeneratedTest

Runs tests that were automatically generated to exercise the agent's output. These are
more targeted than the project's existing tests — they specifically test the new code.

**Gate**: `GeneratedTestGate`
**Cost**: High (test generation + execution)
**False positive rate**: Moderate (generated tests may be incorrect)

### 2.6 Rung 5: PropertyTest

Runs property-based tests (QuickCheck / proptest). These assert invariants over
randomized inputs, providing stronger guarantees than example-based tests.

**Gate**: `PropertyTestGate`
**Cost**: High (randomized exploration)
**False positive rate**: Low (invariants are precise)

### 2.7 Rung 6: Integration

The most expensive gate. Runs full integration tests that may involve external services,
databases, or network connections.

**Gate**: `IntegrationGate`
**Cost**: Highest (minutes to hours)
**False positive rate**: Moderate (infrastructure flakiness)

> **Citation**: crates/roko-gate/src/rung_selector.rs — `Rung` enum definition with
> discriminants 0–6.

---

## 3. Plan Complexity

The selector's primary input is `PlanComplexity`, which classifies the scope of the
planned change:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PlanComplexity {
    Trivial,    // Rename, typo fix, config change
    Simple,     // Single function change, small bug fix
    Standard,   // Multi-file feature, medium scope
    Complex,    // New subsystem, architectural change
}
```

### 3.1 Complexity → Rung Mapping

Each complexity level implies a baseline set of rungs:

| Complexity | Baseline Rungs | Rationale |
|---|---|---|
| `Trivial` | Compile only (0) | No functional changes; compile is sufficient |
| `Simple` | Compile + Lint (0–1) | Small changes; lint catches quality issues |
| `Standard` | Compile + Lint + Test + Symbol (0–3) | Feature work needs test coverage |
| `Complex` | All available (0–6) | Architectural changes need full verification |

### 3.2 Escalation on Failure

When a plan fails a gate, the complexity escalates:

```rust
impl PlanComplexity {
    pub fn escalate(&self) -> Self {
        match self {
            Self::Trivial => Self::Simple,
            Self::Simple => Self::Standard,
            Self::Standard => Self::Complex,
            Self::Complex => Self::Complex,  // Already maximal
        }
    }
}
```

Escalation adds more rungs on retry. A `Trivial` plan that fails compile gets escalated
to `Simple` (adding lint). A `Simple` plan that fails lint gets escalated to `Standard`
(adding test + symbol). This captures the heuristic: if the easy checks catch a problem,
the change is more complex than initially classified, and deeper verification is
warranted.

> **Citation**: crates/roko-gate/src/rung_selector.rs — `PlanComplexity::escalate()`
> method.

---

## 4. Rung Capabilities

Not every environment has every gate available. Some projects don't have a linter
configured. Some don't have property tests. The `RungCaps` struct records which rungs
can actually run:

```rust
pub struct RungCaps {
    pub compile: bool,
    pub lint: bool,
    pub test: bool,
    pub symbol: bool,
    pub generated_test: bool,
    pub property_test: bool,
    pub integration: bool,
}
```

The selector intersects the complexity-implied rungs with the available capabilities.
If the complexity says "run through Rung 3" but `symbol` is `false`, the symbol rung is
skipped.

---

## 5. The `select_rungs()` Function

The public API combines all three inputs:

```rust
pub fn select_rungs(
    complexity: PlanComplexity,
    caps: &RungCaps,
    prior_failures: u32,
) -> Vec<Rung>
```

The algorithm:

1. **Determine effective complexity**: If `prior_failures > 0`, escalate complexity
   by `prior_failures` levels (capped at `Complex`).
2. **Map complexity to maximum rung**: Trivial → 0, Simple → 1, Standard → 3,
   Complex → 6.
3. **Collect all rungs ≤ maximum** that are available in `caps`.
4. **Sort ascending** (cheapest first).
5. **Return the rung list**.

### Example

```
Input:
  complexity = Simple
  caps = { compile: true, lint: true, test: true, symbol: false, ... }
  prior_failures = 1

Step 1: Escalate Simple by 1 → Standard
Step 2: Standard → max rung 3
Step 3: Available rungs ≤ 3 = [Compile(0), Lint(1), Test(2)]
         (Symbol(3) excluded because caps.symbol = false)
Step 4: Already sorted
Result: [Compile, Lint, Test]
```

---

## 6. Relationship to the Gate Pipeline

The rung selector produces a `Vec<Rung>`. The orchestrator maps each `Rung` to a
concrete gate implementation and feeds them into a `GatePipeline`:

```
RungSelector::select_rungs(complexity, caps, failures)
    → Vec<Rung>
    → map each Rung to Box<dyn Gate>
    → GatePipeline::new(gates).with_short_circuit(true)
    → pipeline.verify(signal, ctx)
    → aggregated Verdict
```

This separation of concerns means the selector knows nothing about gate implementations
and the pipeline knows nothing about complexity. Each can evolve independently.

> **Citation**: crates/roko-gate/src/gate_pipeline.rs — GatePipeline accepts
> `Vec<Box<dyn Gate>>` without knowing which rungs they represent.

---

## 7. The Verification-First Architecture

The rung system embodies the verification-first architecture from the Mori reference
system. The core insight is:

**Cheap verification gates that run first prevent expensive retries.**

If the code doesn't compile (Rung 0), there is no point running tests (Rung 2). If
the code has lint violations (Rung 1), many of those violations will also cause test
failures. Running gates in ascending cost order with short-circuit behavior means the
system spends the minimum possible time and compute on verification before either
passing or identifying the cheapest-to-fix failure.

The cost savings compound:
- A compile failure caught in 3 seconds saves a 15-minute test run.
- A lint failure caught in 10 seconds saves the same 15-minute test run.
- A symbol check caught in 50ms saves a 3-second compile + 10-second lint + 15-minute
  test run (though symbol is ranked after test in the default ordering).

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> "6-rung gate system: separate test generation from implementation, tests must fail
> before implementation, immutable verification artifacts."

---

## 8. Escalation in Practice

The escalation mechanism creates a feedback loop between gate outcomes and verification
intensity:

```
Attempt 1: Trivial complexity → Compile only
  → Compile fails
  → Escalate to Simple (prior_failures = 1)

Attempt 2: Simple complexity → Compile + Lint
  → Both pass
  → Record pass at Rung 1
  → Ratchet prevents regression below Rung 1

Attempt 3 (if task retry): Simple complexity → Compile + Lint
  → Lint fails
  → Escalate to Standard (prior_failures = 2)

Attempt 4: Standard complexity → Compile + Lint + Test + Symbol
  → All pass
  → Record pass at Rung 3
  → Task verified
```

This means the system starts cheap and escalates only when warranted. A plan that
passes on the first attempt at Trivial complexity uses only the compile gate —
sub-second verification. A plan that fails repeatedly gets the full battery.

---

## 9. Rung Ordering vs. Rung Cost

The rung numbers (0–6) encode a rough cost ordering, but the actual cost depends on the
project:

| Rung | Typical Cost | Notes |
|---|---|---|
| 0 (Compile) | 1–10 seconds | Incremental builds are faster |
| 1 (Lint) | 2–60 seconds | Clippy can be slow on large codebases |
| 2 (Test) | 5 seconds – 15 minutes | Depends on test count |
| 3 (Symbol) | 10–100 ms | Pure file I/O, no subprocess |
| 4 (GeneratedTest) | 30 seconds – 5 minutes | Depends on generation + execution |
| 5 (PropertyTest) | 10 seconds – 10 minutes | Depends on iterations |
| 6 (Integration) | 1 minute – 1 hour | Depends on infrastructure |

The rung numbers represent a logical ordering (compile before test before integration),
not a strict cost ordering. Symbol (Rung 3) is actually cheaper than Compile (Rung 0),
but logically sits after Test because its value is in catching issues that compile + test
miss.

> **Citation**: refactoring-prd/02-five-layers.md — Layer 3 (Harness): "scoring,
> pattern detection, interventions, process rewards, adaptive gating."

---

## 10. Configuration and Extension

### 10.1 Adding a New Rung

To add a Rung 7 (e.g., "SecurityAudit"):
1. Add `SecurityAudit = 7` to the `Rung` enum.
2. Add `security_audit: bool` to `RungCaps`.
3. Update `select_rungs()` to include Rung 7 for `Complex` plans.
4. Implement a `SecurityAuditGate` in a new module.
5. Map `Rung::SecurityAudit` to `SecurityAuditGate` in the orchestrator.

### 10.2 Configuring per-Project Capabilities

The `RungCaps` struct is constructed by the orchestrator based on project detection:
- If `Cargo.toml` exists → `compile: true`, `lint: true`, `test: true`
- If `roko.toml` specifies `[gates.symbol]` → `symbol: true`
- If property test fixtures exist → `property_test: true`

This means the selector adapts to the project automatically — no configuration needed
for basic verification.

---

## 11. Relationship to Adaptive Thresholds

The adaptive threshold system (see [06-adaptive-thresholds.md](./06-adaptive-thresholds.md))
tracks per-rung pass rates. If a rung consistently passes (20+ consecutive passes), the
threshold system may recommend skipping it (advisory only). This provides a
runtime-adaptive complement to the static complexity-based selection.

The two systems compose: the rung selector determines the *baseline* set of rungs, and
the adaptive thresholds *refine* it based on historical performance.

> **Citation**: crates/roko-gate/src/adaptive_threshold.rs — `should_skip_rung()`
> advisory based on consecutive pass streak.
